//! Narrowing down which Tracked Objects are on show.

use std::collections::{HashMap, HashSet};

use crate::catalogue::{CatalogueError, ElementSet, ObjectKind, TrackedObject};

/// The broad class of a Tracked Object's orbit by altitude and shape.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum OrbitRegime {
    /// Wholly below 2,000 km.
    Low,
    /// Between low orbit and geostationary altitude, such as navigation satellites.
    Medium,
    /// One revolution per day, give or take 5 percent: geostationary satellites, the
    /// inclined ones that drift north and south, and the graveyard orbits just above.
    Geostationary,
    /// Markedly elongated, with an eccentricity of 0.25 or more.
    HighlyElliptical,
}

/// Length of a sidereal day in minutes: the period of a geostationary orbit.
const SIDEREAL_DAY_MINUTES: f64 = 1_436.07;

impl OrbitRegime {
    pub const ALL: [Self; 4] = [
        Self::Low,
        Self::Medium,
        Self::Geostationary,
        Self::HighlyElliptical,
    ];

    /// Classifies an orbit. Shape is judged first, so an elongated orbit is highly
    /// elliptical whatever its altitude.
    pub fn of(element_set: &ElementSet) -> Self {
        let period = element_set.period_minutes();
        if element_set.eccentricity() >= 0.25 {
            Self::HighlyElliptical
        } else if element_set.apogee_altitude_km() < 2_000.0 {
            Self::Low
        } else if (period / SIDEREAL_DAY_MINUTES - 1.0).abs() <= 0.05 {
            Self::Geostationary
        } else {
            Self::Medium
        }
    }
}

/// A named set of Tracked Objects that share a purpose or belong to one constellation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Group {
    Weather,
    SpaceStations,
    Navigation,
    Starlink,
    OneWeb,
    Kuiper,
    Qianfan,
    Guowang,
}

impl Group {
    pub const ALL: [Self; 8] = [
        Self::Weather,
        Self::SpaceStations,
        Self::Navigation,
        Self::Starlink,
        Self::OneWeb,
        Self::Kuiper,
        Self::Qianfan,
        Self::Guowang,
    ];

    /// Groups whose members are published as a list, because nothing in an object's
    /// name says it belongs.
    pub const LISTED: [Self; 3] = [Self::Weather, Self::SpaceStations, Self::Navigation];

    /// For a constellation, the start of every member's name in the Catalogue.
    fn name_prefix(self) -> Option<&'static str> {
        match self {
            Self::Weather | Self::SpaceStations | Self::Navigation => None,
            Self::Starlink => Some("STARLINK"),
            Self::OneWeb => Some("ONEWEB"),
            Self::Kuiper => Some("KUIPER"),
            Self::Qianfan => Some("QIANFAN"),
            // The Catalogue lists the Guowang constellation under this name.
            Self::Guowang => Some("HULIANWANG"),
        }
    }

    /// True if the object belongs to this Group.
    pub fn contains(self, object: &TrackedObject, lists: &GroupLists) -> bool {
        match self.name_prefix() {
            Some(prefix) => object.name.starts_with(prefix),
            None => lists
                .members
                .get(&self)
                .is_some_and(|members| members.contains(&object.catalogue_number)),
        }
    }
}

/// The published member lists for the Groups in [`Group::LISTED`], by catalogue number.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GroupLists {
    members: HashMap<Group, HashSet<u32>>,
}

impl GroupLists {
    /// Sets the members of one Group, replacing any it had.
    pub fn set(&mut self, group: Group, members: HashSet<u32>) {
        self.members.insert(group, members);
    }

    /// How many members a listed Group has.
    pub fn len(&self, group: Group) -> usize {
        self.members.get(&group).map_or(0, HashSet::len)
    }

    /// True if no Group has any members.
    pub fn is_empty(&self) -> bool {
        self.members.values().all(HashSet::is_empty)
    }
}

/// Reads the catalogue numbers in a CSV file that has a `NORAD_CAT_ID` column, such as
/// one of CelesTrak's group files.
pub fn parse_group_members_csv(text: &str) -> Result<HashSet<u32>, CatalogueError> {
    let mut reader = csv::Reader::from_reader(text.as_bytes());
    let column = reader
        .headers()
        .map_err(CatalogueError::Malformed)?
        .iter()
        .position(|header| header == "NORAD_CAT_ID")
        .ok_or(CatalogueError::MissingColumn("NORAD_CAT_ID"))?;

    let mut members = HashSet::new();
    for row in reader.records() {
        let row = row.map_err(CatalogueError::Malformed)?;
        if let Some(Ok(number)) = row.get(column).map(str::parse) {
            members.insert(number);
        }
    }
    Ok(members)
}

/// Which Tracked Objects to show. The default shows everything.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Filter {
    pub satellites: bool,
    pub rocket_bodies: bool,
    pub debris: bool,
    pub low: bool,
    pub medium: bool,
    pub geostationary: bool,
    pub highly_elliptical: bool,
    /// Show only members of this Group; `None` shows objects whether or not they are in one.
    pub group: Option<Group>,
    /// Show only objects whose name contains this text, or whose catalogue number or
    /// designator starts with it. Letter case and surrounding spaces are ignored.
    pub text: String,
}

impl Default for Filter {
    fn default() -> Self {
        Self {
            satellites: true,
            rocket_bodies: true,
            debris: true,
            low: true,
            medium: true,
            geostationary: true,
            highly_elliptical: true,
            group: None,
            text: String::new(),
        }
    }
}

impl Filter {
    /// True if the object passes every part of the filter.
    pub fn matches(&self, object: &TrackedObject, lists: &GroupLists) -> bool {
        let kind = match object.kind {
            ObjectKind::Satellite => self.satellites,
            ObjectKind::RocketBody => self.rocket_bodies,
            ObjectKind::Debris => self.debris,
        };
        let regime = match OrbitRegime::of(&object.element_set) {
            OrbitRegime::Low => self.low,
            OrbitRegime::Medium => self.medium,
            OrbitRegime::Geostationary => self.geostationary,
            OrbitRegime::HighlyElliptical => self.highly_elliptical,
        };
        kind && regime
            && self.group.is_none_or(|group| group.contains(object, lists))
            && self.matches_text(object)
    }

    fn matches_text(&self, object: &TrackedObject) -> bool {
        let needle = self.text.trim().to_lowercase();
        needle.is_empty()
            || object.name.to_lowercase().contains(&needle)
            || object.catalogue_number.to_string().starts_with(&needle)
            || object.designator.to_lowercase().starts_with(&needle)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalogue::parse_element_sets_csv;

    const HEADER: &str = "OBJECT_NAME,OBJECT_ID,EPOCH,MEAN_MOTION,ECCENTRICITY,INCLINATION,RA_OF_ASC_NODE,ARG_OF_PERICENTER,MEAN_ANOMALY,EPHEMERIS_TYPE,CLASSIFICATION_TYPE,NORAD_CAT_ID,ELEMENT_SET_NO,REV_AT_EPOCH,BSTAR,MEAN_MOTION_DOT,MEAN_MOTION_DDOT";

    /// A Tracked Object with the given name, catalogue number, revolutions per day and eccentricity.
    fn object(
        name: &str,
        number: u32,
        kind: ObjectKind,
        mean_motion: f64,
        eccentricity: f64,
    ) -> TrackedObject {
        let text = format!(
            "{HEADER}\n{name},1998-067A,2026-11-24T19:13:23.305152,{mean_motion},{eccentricity},51.6,75.5,163.9,294.5,0,U,{number},999,9352,.4264642E-3,.43E-5,0\n"
        );
        parse_element_sets_csv(&text, kind)
            .expect("a valid row")
            .objects
            .remove(0)
    }

    fn satellite(name: &str, number: u32) -> TrackedObject {
        object(name, number, ObjectKind::Satellite, 15.5, 0.0005)
    }

    fn regime(mean_motion: f64, eccentricity: f64) -> OrbitRegime {
        OrbitRegime::of(
            &object("X", 1, ObjectKind::Satellite, mean_motion, eccentricity).element_set,
        )
    }

    #[test]
    fn orbits_are_classified_by_shape_then_altitude() {
        // The space station, 15.5 revolutions a day at about 420 km.
        assert_eq!(regime(15.5, 0.0005), OrbitRegime::Low);
        // A GPS satellite, two revolutions a day at about 20,200 km.
        assert_eq!(regime(2.005_6, 0.01), OrbitRegime::Medium);
        // Geostationary, and a graveyard orbit about 300 km above it.
        assert_eq!(regime(1.002_737_9, 0.0002), OrbitRegime::Geostationary);
        assert_eq!(regime(0.992, 0.001), OrbitRegime::Geostationary);
        // A Molniya orbit: the same period as GPS, but very elongated.
        assert_eq!(regime(2.006, 0.72), OrbitRegime::HighlyElliptical);
    }

    #[test]
    fn constellations_are_recognised_by_name_and_other_groups_by_list() {
        let lists = {
            let mut lists = GroupLists::default();
            lists.set(Group::Weather, HashSet::from([43_013]));
            lists
        };
        let starlink = satellite("STARLINK-1008", 44_714);
        let noaa = satellite("NOAA 20 (JPSS-1)", 43_013);

        assert!(Group::Starlink.contains(&starlink, &lists));
        assert!(!Group::OneWeb.contains(&starlink, &lists));
        assert!(!Group::Weather.contains(&starlink, &lists));
        assert!(Group::Weather.contains(&noaa, &lists));
        // A listed Group with no list on hand has no members.
        assert!(!Group::Navigation.contains(&noaa, &lists));
        assert_eq!(
            (lists.len(Group::Weather), lists.len(Group::Navigation)),
            (1, 0)
        );
    }

    #[test]
    fn group_files_are_read_by_their_catalogue_number_column() {
        let full = format!(
            "{HEADER}\nNOAA 20,2017-073A,2026-11-24T19:13:23.305152,14.19,.0001,98.7,75.5,163.9,294.5,0,U,43013,999,9352,.4E-3,.43E-5,0\n"
        );
        assert_eq!(
            parse_group_members_csv(&full).unwrap(),
            HashSet::from([43_013])
        );
        assert_eq!(
            parse_group_members_csv("NORAD_CAT_ID\n25544\n48274\n").unwrap(),
            HashSet::from([25_544, 48_274])
        );
        assert!(parse_group_members_csv("NAME\nISS\n").is_err());
    }

    #[test]
    fn the_default_filter_shows_everything() {
        let filter = Filter::default();
        let lists = GroupLists::default();
        assert!(filter.matches(&satellite("ISS (ZARYA)", 25_544), &lists));
        assert!(filter.matches(
            &object("FENGYUN 1C DEB", 29_733, ObjectKind::Debris, 14.1, 0.01),
            &lists
        ));
        assert!(filter.matches(
            &object("MOLNIYA 1-91", 25_485, ObjectKind::Satellite, 2.006, 0.72),
            &lists
        ));
    }

    #[test]
    fn kinds_and_regimes_can_be_switched_off() {
        let lists = GroupLists::default();
        let debris = object("FENGYUN 1C DEB", 29_733, ObjectKind::Debris, 14.1, 0.01);
        let geostationary = object("GOES 16", 41_866, ObjectKind::Satellite, 1.002_7, 0.0001);

        let no_debris = Filter {
            debris: false,
            ..Filter::default()
        };
        assert!(!no_debris.matches(&debris, &lists));
        assert!(no_debris.matches(&geostationary, &lists));

        let only_low = Filter {
            medium: false,
            geostationary: false,
            highly_elliptical: false,
            ..Filter::default()
        };
        assert!(only_low.matches(&debris, &lists));
        assert!(!only_low.matches(&geostationary, &lists));
    }

    #[test]
    fn a_group_narrows_the_filter_to_its_members() {
        let lists = GroupLists::default();
        let filter = Filter {
            group: Some(Group::Starlink),
            ..Filter::default()
        };
        assert!(filter.matches(&satellite("STARLINK-1008", 44_714), &lists));
        assert!(!filter.matches(&satellite("ISS (ZARYA)", 25_544), &lists));
    }

    #[test]
    fn text_finds_names_numbers_and_designators() {
        let lists = GroupLists::default();
        let station = satellite("ISS (ZARYA)", 25_544);
        let matches = |text: &str| {
            Filter {
                text: text.to_owned(),
                ..Filter::default()
            }
            .matches(&station, &lists)
        };

        assert!(matches("iss"));
        assert!(matches("  Zarya "));
        assert!(matches("25544"));
        assert!(matches("255"));
        assert!(matches("1998-067"));
        assert!(!matches("5544"));
        assert!(!matches("hubble"));
    }
}
