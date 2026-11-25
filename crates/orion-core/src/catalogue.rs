//! Tracked Objects and their Element Sets.

use std::fmt;

use crate::EARTH_EQUATORIAL_RADIUS_KM;
use crate::frames::inertial_to_earth_fixed;
use crate::time::JulianDate;

/// What a Tracked Object is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ObjectKind {
    Satellite,
    RocketBody,
    Debris,
}

/// The published description of one Tracked Object's orbit, ready for Propagation.
#[derive(Clone, Debug)]
pub struct ElementSet {
    epoch: JulianDate,
    constants: sgp4::Constants,
    mean_motion_rev_per_day: f64,
    eccentricity: f64,
    inclination_deg: f64,
}

/// Where a Tracked Object is and how fast it is moving at one moment.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct State {
    /// Position in kilometres in the Earth-fixed frame.
    pub position_km: [f64; 3],
    /// Speed in kilometres per second relative to the stars, not to the ground.
    pub speed_km_s: f64,
}

/// Earth's gravitational parameter in km^3/s^2 (WGS 84).
const EARTH_GM: f64 = 398_600.441_8;

impl ElementSet {
    fn from_sgp4(elements: &sgp4::Elements) -> Result<Self, sgp4::ElementsError> {
        Ok(Self {
            // `epoch()` is in Julian years of 365.25 days since J2000.
            epoch: JulianDate(JulianDate::J2000.0 + elements.epoch() * 365.25),
            constants: sgp4::Constants::from_elements(elements)?,
            mean_motion_rev_per_day: elements.mean_motion,
            eccentricity: elements.eccentricity,
            inclination_deg: elements.inclination,
        })
    }

    /// Time to complete one revolution, in minutes.
    pub fn period_minutes(&self) -> f64 {
        1_440.0 / self.mean_motion_rev_per_day
    }

    /// How far the orbit departs from a circle: 0 is circular, values near 1 are very elongated.
    pub fn eccentricity(&self) -> f64 {
        self.eccentricity
    }

    /// Tilt of the orbit relative to the equator, in degrees. Above 90 the object
    /// travels against Earth's rotation.
    pub fn inclination_deg(&self) -> f64 {
        self.inclination_deg
    }

    /// Half the long axis of the orbit, in kilometres, from Kepler's third law.
    fn semi_major_axis_km(&self) -> f64 {
        let radians_per_second = self.mean_motion_rev_per_day * std::f64::consts::TAU / 86_400.0;
        (EARTH_GM / (radians_per_second * radians_per_second)).cbrt()
    }

    /// Approximate height above the equatorial radius at the orbit's highest point.
    pub fn apogee_altitude_km(&self) -> f64 {
        self.semi_major_axis_km() * (1.0 + self.eccentricity) - EARTH_EQUATORIAL_RADIUS_KM
    }

    /// Approximate height above the equatorial radius at the orbit's lowest point.
    pub fn perigee_altitude_km(&self) -> f64 {
        self.semi_major_axis_km() * (1.0 - self.eccentricity) - EARTH_EQUATORIAL_RADIUS_KM
    }

    /// The moment this Element Set describes.
    pub fn epoch(&self) -> JulianDate {
        self.epoch
    }

    /// Element Age in days at the given moment. Negative if the moment is before the Epoch.
    pub fn age_days(&self, at: JulianDate) -> f64 {
        at.0 - self.epoch.0
    }

    /// Position in kilometres in the Earth-fixed frame at the given moment, or `None`
    /// if the orbit model cannot produce one, as happens for an object that has decayed.
    pub fn position(&self, at: JulianDate) -> Option<[f64; 3]> {
        self.position_inertial(at)
            .map(|position| inertial_to_earth_fixed(position, at))
    }

    /// Position and speed at the given moment, or `None` if the orbit model cannot produce them.
    pub fn state(&self, at: JulianDate) -> Option<State> {
        let prediction = self.predict(at)?;
        let [vx, vy, vz] = prediction.velocity;
        Some(State {
            position_km: inertial_to_earth_fixed(prediction.position, at),
            speed_km_s: (vx * vx + vy * vy + vz * vz).sqrt(),
        })
    }

    /// The Orbit Path: `samples` points along one revolution starting at `at`, in
    /// kilometres in the Earth-fixed frame as it stands at `at`.
    ///
    /// The orbit is a closed curve against the stars, so every point is turned by the
    /// same angle. The result is the ring the object is travelling along right now;
    /// Earth turns beneath it. Points the model cannot produce are left out.
    pub fn orbit_path(&self, at: JulianDate, samples: usize) -> Vec<[f64; 3]> {
        self.sample_revolution(at, samples)
            .filter_map(|moment| self.position_inertial(moment))
            .map(|position| inertial_to_earth_fixed(position, at))
            .collect()
    }

    /// The path over the ground for one revolution starting at `at`: `samples`
    /// positions in kilometres in the Earth-fixed frame, each at its own moment.
    /// The Ground Track is the line on the surface beneath them.
    pub fn ground_path(&self, at: JulianDate, samples: usize) -> Vec<[f64; 3]> {
        self.sample_revolution(at, samples)
            .filter_map(|moment| self.position(moment))
            .collect()
    }

    /// `samples` evenly spaced moments covering one revolution from `at`, ends included.
    fn sample_revolution(
        &self,
        at: JulianDate,
        samples: usize,
    ) -> impl Iterator<Item = JulianDate> {
        let period_days = self.period_minutes() / 1_440.0;
        let steps = samples.saturating_sub(1).max(1) as f64;
        (0..samples).map(move |sample| JulianDate(at.0 + period_days * sample as f64 / steps))
    }

    /// Position in kilometres in the inertial frame. SGP4's own frame differs from the
    /// one described in the crate documentation by under 0.01 degrees, which is ignored.
    fn position_inertial(&self, at: JulianDate) -> Option<[f64; 3]> {
        self.predict(at).map(|prediction| prediction.position)
    }

    fn predict(&self, at: JulianDate) -> Option<sgp4::Prediction> {
        let minutes = (at.0 - self.epoch.0) * 1_440.0;
        self.constants
            .propagate(sgp4::MinutesSinceEpoch(minutes))
            .ok()
            .filter(|prediction| {
                prediction
                    .position
                    .iter()
                    .chain(&prediction.velocity)
                    .all(|component| component.is_finite())
            })
    }
}

/// Anything in orbit that appears in the Catalogue.
#[derive(Clone, Debug)]
pub struct TrackedObject {
    pub catalogue_number: u32,
    pub name: String,
    /// International designator: launch year, launch number and piece, such as `1998-067A`.
    pub designator: String,
    pub kind: ObjectKind,
    pub element_set: ElementSet,
}

/// The outcome of reading a file of Element Sets.
#[derive(Debug)]
pub struct ParsedElementSets {
    pub objects: Vec<TrackedObject>,
    /// Rows that were well-formed but describe an orbit the model cannot represent.
    pub rejected: usize,
}

#[derive(Debug)]
pub enum CatalogueError {
    /// The text is not a valid CSV file of the expected kind.
    Malformed(csv::Error),
    /// The file has no column with this name.
    MissingColumn(&'static str),
}

impl fmt::Display for CatalogueError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Malformed(error) => write!(f, "malformed file: {error}"),
            Self::MissingColumn(name) => write!(f, "the file has no {name} column"),
        }
    }
}

impl std::error::Error for CatalogueError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Malformed(error) => Some(error),
            Self::MissingColumn(_) => None,
        }
    }
}

/// Reads Element Sets in CelesTrak's CSV form of the OMM standard. The file does not
/// say what each object is, so every row is given `kind`.
pub fn parse_element_sets_csv(
    text: &str,
    kind: ObjectKind,
) -> Result<ParsedElementSets, CatalogueError> {
    let mut objects = Vec::new();
    let mut rejected = 0;

    let mut reader = csv::Reader::from_reader(text.as_bytes());
    for row in reader.deserialize::<sgp4::Elements>() {
        let elements = row.map_err(CatalogueError::Malformed)?;
        let (Ok(catalogue_number), Ok(element_set)) = (
            u32::try_from(elements.norad_id),
            ElementSet::from_sgp4(&elements),
        ) else {
            rejected += 1;
            continue;
        };
        objects.push(TrackedObject {
            catalogue_number,
            name: elements.object_name.unwrap_or_default(),
            designator: elements.international_designator.unwrap_or_default(),
            kind,
            element_set,
        });
    }

    Ok(ParsedElementSets { objects, rejected })
}

/// Reads several Element Set files, one per kind, into a single list.
///
/// An object can appear in more than one file, because the files come from separate
/// queries. The first occurrence wins, so list the most trusted kind first.
pub fn parse_element_set_files(
    files: &[(ObjectKind, &str)],
) -> Result<ParsedElementSets, CatalogueError> {
    let mut seen = std::collections::HashSet::new();
    let mut combined = ParsedElementSets {
        objects: Vec::new(),
        rejected: 0,
    };
    for &(kind, text) in files {
        let parsed = parse_element_sets_csv(text, kind)?;
        combined.rejected += parsed.rejected;
        combined.objects.extend(
            parsed
                .objects
                .into_iter()
                .filter(|object| seen.insert(object.catalogue_number)),
        );
    }
    Ok(combined)
}

/// The Catalogue's lasting facts about one Tracked Object, as the codes it uses.
/// See [`crate::codes`] for what the codes stand for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CatalogueRecord {
    /// Code for the owning country or organisation, such as `US`.
    pub owner: String,
    /// Launch date as `YYYY-MM-DD`.
    pub launch_date: String,
    /// Code for the launch site, such as `AFETR`.
    pub launch_site: String,
}

/// Catalogue Records by catalogue number.
pub type CatalogueRecords = std::collections::HashMap<u32, CatalogueRecord>;

/// Reads Catalogue Records from CelesTrak's satellite catalogue in CSV form.
///
/// Columns are found by name, so the full file and a copy trimmed to the needed columns
/// both work. Objects that have decayed are left out, since they are never on show.
pub fn parse_catalogue_records_csv(text: &str) -> Result<CatalogueRecords, CatalogueError> {
    let mut reader = csv::Reader::from_reader(text.as_bytes());
    let headers = reader.headers().map_err(CatalogueError::Malformed)?.clone();
    let column = |name: &'static str| {
        headers
            .iter()
            .position(|header| header == name)
            .ok_or(CatalogueError::MissingColumn(name))
    };
    let number = column("NORAD_CAT_ID")?;
    let owner = column("OWNER")?;
    let launch_date = column("LAUNCH_DATE")?;
    let launch_site = column("LAUNCH_SITE")?;
    let decay_date = column("DECAY_DATE").ok();

    let mut records = CatalogueRecords::new();
    for row in reader.records() {
        let row = row.map_err(CatalogueError::Malformed)?;
        let field = |index: usize| row.get(index).unwrap_or_default();
        if decay_date.is_some_and(|index| !field(index).is_empty()) {
            continue;
        }
        let Ok(catalogue_number) = field(number).parse() else {
            continue;
        };
        records.insert(
            catalogue_number,
            CatalogueRecord {
                owner: field(owner).to_owned(),
                launch_date: field(launch_date).to_owned(),
                launch_site: field(launch_site).to_owned(),
            },
        );
    }
    Ok(records)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Real rows from CelesTrak's satellite catalogue. Sputnik 1 has decayed.
    const CATALOGUE: &str = "\
OBJECT_NAME,OBJECT_ID,NORAD_CAT_ID,OBJECT_TYPE,OPS_STATUS_CODE,OWNER,LAUNCH_DATE,LAUNCH_SITE,DECAY_DATE,PERIOD,INCLINATION,APOGEE,PERIGEE,RCS,DATA_STATUS_CODE,ORBIT_CENTER,ORBIT_TYPE
SPUTNIK 1,1957-001B,2,PAY,D,CIS,1957-10-04,TYMSC,1958-01-03,96.10,65.00,1080,64,,,EA,IMP
ISS (ZARYA),1998-067A,25544,PAY,+,ISS,1998-11-20,TYMSC,,92.93,51.63,423,413,399.0524,,EA,ORB
";

    #[test]
    fn catalogue_records_are_read_for_objects_still_in_orbit() {
        let records = parse_catalogue_records_csv(CATALOGUE).expect("a valid file");
        assert_eq!(records.len(), 1);
        assert_eq!(
            records[&25_544],
            CatalogueRecord {
                owner: "ISS".to_owned(),
                launch_date: "1998-11-20".to_owned(),
                launch_site: "TYMSC".to_owned(),
            }
        );
    }

    #[test]
    fn a_trimmed_catalogue_file_reads_the_same() {
        let trimmed = "NORAD_CAT_ID,OWNER,LAUNCH_DATE,LAUNCH_SITE\n25544,ISS,1998-11-20,TYMSC\n";
        assert_eq!(
            parse_catalogue_records_csv(trimmed).unwrap(),
            parse_catalogue_records_csv(CATALOGUE).unwrap()
        );
    }

    #[test]
    fn a_file_without_the_needed_columns_is_an_error() {
        let result = parse_catalogue_records_csv("NORAD_CAT_ID,OWNER\n25544,ISS\n");
        assert!(
            matches!(result, Err(CatalogueError::MissingColumn("LAUNCH_DATE"))),
            "{result:?}"
        );
        // An Element Set file is not a catalogue file.
        let element_sets = format!("{HEADER}\n{CALSPHERE}\n");
        assert!(parse_catalogue_records_csv(&element_sets).is_err());
    }

    const HEADER: &str = "OBJECT_NAME,OBJECT_ID,EPOCH,MEAN_MOTION,ECCENTRICITY,INCLINATION,RA_OF_ASC_NODE,ARG_OF_PERICENTER,MEAN_ANOMALY,EPHEMERIS_TYPE,CLASSIFICATION_TYPE,NORAD_CAT_ID,ELEMENT_SET_NO,REV_AT_EPOCH,BSTAR,MEAN_MOTION_DOT,MEAN_MOTION_DDOT";
    // Real rows from CelesTrak, 24 November 2026. The second has a six-digit catalogue number.
    const CALSPHERE: &str = "CALSPHERE 1,1964-063C,2026-11-24T19:13:23.305152,13.76776729,.00275414,90.2245,75.5043,163.9822,294.5252,0,U,900,999,9352,.4264642E-3,.43E-5,0";
    const SARAMAGO: &str = "SARAMAGO,2026-067CY,2026-11-24T04:03:35.615808,15.22537328,.00081744,97.4801,285.3319,152.2349,207.9325,0,U,100000,999,3602,.25513667E-3,.5852E-4,0";

    fn parse(rows: &[&str]) -> ParsedElementSets {
        let text = format!("{HEADER}\n{}\n", rows.join("\n"));
        parse_element_sets_csv(&text, ObjectKind::Satellite).expect("a valid file")
    }

    fn distance_from_centre(position: [f64; 3]) -> f64 {
        position.iter().map(|c| c * c).sum::<f64>().sqrt()
    }

    #[test]
    fn rows_become_tracked_objects() {
        let parsed = parse(&[CALSPHERE, SARAMAGO]);
        assert_eq!(parsed.rejected, 0);
        assert_eq!(parsed.objects.len(), 2);

        let calsphere = &parsed.objects[0];
        assert_eq!(calsphere.catalogue_number, 900);
        assert_eq!(calsphere.name, "CALSPHERE 1");
        assert_eq!(calsphere.designator, "1964-063C");
        assert_eq!(calsphere.kind, ObjectKind::Satellite);

        assert_eq!(parsed.objects[1].catalogue_number, 100_000);
    }

    #[test]
    fn epoch_is_read_as_utc() {
        let parsed = parse(&[CALSPHERE]);
        let expected = JulianDate::from_utc(2026, 11, 24, 19, 13, 23.305_152);
        let epoch = parsed.objects[0].element_set.epoch();
        // Within a millisecond.
        assert!(
            (epoch.0 - expected.0).abs() < 1e-8,
            "{epoch:?} vs {expected:?}"
        );
        assert!(
            (parsed.objects[0]
                .element_set
                .age_days(JulianDate(expected.0 + 2.5))
                - 2.5)
                .abs()
                < 1e-8
        );
    }

    #[test]
    fn orbit_radius_agrees_with_keplers_third_law() {
        // 13.76776729 revolutions a day gives a semi-major axis of about 7,352 km, and
        // the orbit is nearly circular, so the object stays within a few tens of km of that.
        let parsed = parse(&[CALSPHERE]);
        let element_set = &parsed.objects[0].element_set;
        for hours in [0.0, 0.4, 0.9, 13.0] {
            let at = JulianDate(element_set.epoch().0 + hours / 24.0);
            let radius = distance_from_centre(element_set.position(at).expect("a position"));
            assert!(
                (radius - 7_352.0).abs() < 40.0,
                "{radius} km after {hours} h"
            );
        }
    }

    #[test]
    fn orbit_figures_match_a_geostationary_orbit() {
        // One revolution per sidereal day in a circle is the geostationary orbit:
        // a period of 1,436 minutes at 35,786 km.
        let row = CALSPHERE
            .replace(",13.76776729,", ",1.00273791,")
            .replace(",.00275414,", ",0.0,");
        let parsed = parse(&[&row]);
        let element_set = &parsed.objects[0].element_set;

        assert!((element_set.period_minutes() - 1_436.07).abs() < 0.01);
        assert!((element_set.apogee_altitude_km() - 35_786.0).abs() < 5.0);
        assert!((element_set.perigee_altitude_km() - 35_786.0).abs() < 5.0);
        assert_eq!(element_set.eccentricity(), 0.0);
        assert_eq!(element_set.inclination_deg(), 90.2245);
    }

    #[test]
    fn speed_matches_a_circular_orbit_at_that_height() {
        // A circular orbit of radius 7,353.5 km is travelled at sqrt(GM / r) = 7.36 km/s.
        let parsed = parse(&[CALSPHERE]);
        let element_set = &parsed.objects[0].element_set;
        let state = element_set.state(element_set.epoch()).expect("a state");
        assert!((state.speed_km_s - 7.36).abs() < 0.05, "{state:?}");
        assert_eq!(
            Some(state.position_km),
            element_set.position(element_set.epoch())
        );
    }

    #[test]
    fn orbit_path_is_a_closed_ring_through_the_current_position() {
        let parsed = parse(&[CALSPHERE]);
        let element_set = &parsed.objects[0].element_set;
        let at = JulianDate(element_set.epoch().0 + 0.3);

        let path = element_set.orbit_path(at, 181);
        assert_eq!(path.len(), 181);
        assert_eq!(Some(path[0]), element_set.position(at));

        // After one revolution the object is back where it started, give or take the
        // slow drift of the orbit itself.
        let gap = distance_from_centre([
            path[180][0] - path[0][0],
            path[180][1] - path[0][1],
            path[180][2] - path[0][2],
        ]);
        assert!(gap < 60.0, "{gap} km between the ends");
        for point in &path {
            let altitude = distance_from_centre(*point) - EARTH_EQUATORIAL_RADIUS_KM;
            assert!((930.0..1_020.0).contains(&altitude), "{altitude} km");
        }
    }

    #[test]
    fn ground_path_slips_west_as_earth_turns() {
        // This orbit passes over the poles and takes 104.6 minutes. In that time Earth
        // turns 26.2 degrees east, so the next pass crosses the same latitude that far west.
        let parsed = parse(&[CALSPHERE]);
        let element_set = &parsed.objects[0].element_set;
        let at = JulianDate(element_set.epoch().0 + 0.3);

        let path = element_set.ground_path(at, 181);
        assert_eq!(Some(path[0]), element_set.position(at));

        let longitude = |point: [f64; 3]| point[1].atan2(point[0]).to_degrees();
        let slip = (longitude(path[180]) - longitude(path[0]) + 540.0).rem_euclid(360.0) - 180.0;
        assert!((slip + 26.2).abs() < 0.5, "slipped {slip} degrees");
    }

    #[test]
    fn impossible_orbits_are_rejected_not_fatal() {
        // An eccentricity of 1 or more is not a closed orbit.
        let escaping = CALSPHERE.replace(",.00275414,", ",1.5,");
        let parsed = parse(&[&escaping, SARAMAGO]);
        assert_eq!(parsed.rejected, 1);
        assert_eq!(parsed.objects.len(), 1);
        assert_eq!(parsed.objects[0].name, "SARAMAGO");
    }

    #[test]
    fn an_object_in_two_files_keeps_its_first_kind() {
        let satellites = format!("{HEADER}\n{CALSPHERE}\n");
        let debris = format!("{HEADER}\n{CALSPHERE}\n{SARAMAGO}\n");
        let parsed = parse_element_set_files(&[
            (ObjectKind::Satellite, &satellites),
            (ObjectKind::Debris, &debris),
        ])
        .expect("valid files");

        let kinds: Vec<_> = parsed
            .objects
            .iter()
            .map(|object| (object.catalogue_number, object.kind))
            .collect();
        assert_eq!(
            kinds,
            [(900, ObjectKind::Satellite), (100_000, ObjectKind::Debris)]
        );
    }

    #[test]
    fn malformed_files_are_errors() {
        let text = format!("{HEADER}\nCALSPHERE 1,1964-063C,not a date,13.7\n");
        assert!(parse_element_sets_csv(&text, ObjectKind::Satellite).is_err());
    }

    #[test]
    fn propagation_matches_the_sgp4_verification_case() {
        // Catalogue number 5 from the verification set published with Vallado et al.,
        // "Revisiting Spacetrack Report #3". Reference positions are in SGP4's own frame.
        // The reference run used the older WGS 72 constants, so allow a small difference.
        let elements = sgp4::Elements::from_tle(
            None,
            b"1 00005U 58002B   00179.78495062  .00000023  00000-0  28098-4 0  4753",
            b"2 00005  34.2682 348.7242 1859667 331.7664  19.3264 10.82419157413667",
        )
        .expect("a valid element set");
        let element_set = ElementSet::from_sgp4(&elements).expect("a representable orbit");

        let cases = [
            (0.0, [7_022.465_292_66, -1_400.082_967_55, 0.039_951_55]),
            (
                360.0,
                [-7_154.031_202_02, -3_783.176_825_04, -3_536.194_122_94],
            ),
        ];
        for (minutes, expected) in cases {
            let at = JulianDate(element_set.epoch().0 + minutes / 1_440.0);
            let actual = element_set.position_inertial(at).expect("a position");
            let error = distance_from_centre([
                actual[0] - expected[0],
                actual[1] - expected[1],
                actual[2] - expected[2],
            ]);
            assert!(error < 1.0, "{error} km off after {minutes} minutes");
        }
    }
}
