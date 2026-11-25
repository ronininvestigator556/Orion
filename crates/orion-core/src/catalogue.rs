//! Tracked Objects and their Element Sets.

use std::fmt;

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
}

impl ElementSet {
    fn from_sgp4(elements: &sgp4::Elements) -> Result<Self, sgp4::ElementsError> {
        Ok(Self {
            // `epoch()` is in Julian years of 365.25 days since J2000.
            epoch: JulianDate(JulianDate::J2000.0 + elements.epoch() * 365.25),
            constants: sgp4::Constants::from_elements(elements)?,
        })
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

    /// Position in kilometres in the inertial frame. SGP4's own frame differs from the
    /// one described in the crate documentation by under 0.01 degrees, which is ignored.
    fn position_inertial(&self, at: JulianDate) -> Option<[f64; 3]> {
        let minutes = (at.0 - self.epoch.0) * 1_440.0;
        self.constants
            .propagate(sgp4::MinutesSinceEpoch(minutes))
            .ok()
            .map(|prediction| prediction.position)
            .filter(|position| position.iter().all(|component| component.is_finite()))
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
    /// The text is not a valid Element Set file.
    Malformed(csv::Error),
}

impl fmt::Display for CatalogueError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Malformed(error) => write!(f, "malformed Element Set file: {error}"),
        }
    }
}

impl std::error::Error for CatalogueError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Malformed(error) => Some(error),
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

#[cfg(test)]
mod tests {
    use super::*;

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
