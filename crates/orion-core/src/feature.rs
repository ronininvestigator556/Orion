//! A Feature that sits at a place on the ground: an event, a building, a site.
//!
//! Layers of such Features differ in where their data comes from and what it says,
//! but once read they are all drawn, searched, filtered and described the same way.

use crate::layer::CardRow;
use crate::time::JulianDate;

/// One thing at a place on the ground, as a Source describes it.
#[derive(Clone, Debug, PartialEq)]
pub struct Feature {
    /// Identifies the Feature for as long as its Source lists it.
    pub key: u64,
    /// What it is called in search results and beside its dot.
    pub name: String,
    /// North positive.
    pub latitude_deg: f64,
    /// East positive.
    pub longitude_deg: f64,
    /// When it happened or was observed or came into force, for things that have a moment.
    pub time: Option<JulianDate>,
    /// When it stops being in force, for things that end.
    pub until: Option<JulianDate>,
    /// The number its Layer's filter and dot size work from, such as an earthquake's
    /// magnitude. Zero where the Layer has no such number.
    pub size: f32,
    /// Which of its Layer's classes it belongs to, for its colour and the filter.
    pub class: u8,
    /// What it shows about itself when selected.
    pub rows: Vec<CardRow>,
    /// Longer text shown under the rows, a paragraph at a time.
    pub details: Vec<String>,
    /// Its outline or route, when it has one: lines of longitude then latitude in degrees.
    pub shape: Vec<Vec<[f64; 2]>>,
}

impl Feature {
    /// A Feature with a name and a place and nothing else yet.
    pub fn at(id: &str, name: impl Into<String>, latitude_deg: f64, longitude_deg: f64) -> Self {
        Self {
            key: stable_key(id),
            name: name.into(),
            latitude_deg,
            longitude_deg,
            time: None,
            until: None,
            size: 0.0,
            class: 0,
            rows: Vec::new(),
            details: Vec::new(),
            shape: Vec::new(),
        }
    }

    /// Adds a row to the card, unless the value is empty.
    pub fn row(&mut self, label: &str, value: impl Into<String>) {
        let value = value.into();
        if !value.trim().is_empty() {
            self.rows.push(CardRow::new(label, value));
        }
    }

    /// True if the place is a real one. Sources do publish coordinates that are not.
    pub fn is_on_earth(&self) -> bool {
        (-90.0..=90.0).contains(&self.latitude_deg)
            && (-180.0..=180.0).contains(&self.longitude_deg)
    }
}

/// Turns a Source's own identifier into the number Orion keeps hold of a Feature by.
/// The same text always gives the same number, on every computer and in every version.
pub fn stable_key(id: &str) -> u64 {
    // FNV-1a, which is simple, fast and stable by definition.
    id.bytes().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_the_same_every_time_and_differ_between_ids() {
        // The published FNV-1a test values for the empty string and for "a".
        assert_eq!(stable_key(""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(stable_key("a"), 0xaf63_dc4c_8601_ec8c);
        assert_ne!(stable_key("us6000udes"), stable_key("us6000udet"));
    }

    #[test]
    fn empty_rows_are_left_off_the_card() {
        let mut feature = Feature::at("x", "Somewhere", 10.0, 20.0);
        feature.row("Depth", "10 km");
        feature.row("Felt by", "  ");
        assert_eq!(feature.rows, [CardRow::new("Depth", "10 km")]);
    }

    #[test]
    fn impossible_places_are_recognised() {
        assert!(Feature::at("x", "x", 90.0, -180.0).is_on_earth());
        assert!(!Feature::at("x", "x", 91.0, 0.0).is_on_earth());
        assert!(!Feature::at("x", "x", 0.0, 200.0).is_on_earth());
        assert!(!Feature::at("x", "x", f64::NAN, 0.0).is_on_earth());
    }
}
