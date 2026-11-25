//! Where each country is, for Sources that name a country and give no place.
//!
//! The table is built by `tools/make_countries.py` from Natural Earth's map of the
//! world's countries, which is in the public domain: each line is a country's
//! two-letter code, the code the U.S. government uses for it, its name in English,
//! and the latitude and longitude of the point where a map would put its name.

use std::collections::HashMap;
use std::sync::OnceLock;

const TABLE: &str = include_str!("../data/countries.tsv");

/// A country's name with everything that varies from one list to another taken out.
fn plain(name: &str) -> String {
    let letters: String = name
        .to_lowercase()
        .chars()
        .map(|letter| if letter.is_alphabetic() { letter } else { ' ' })
        .collect();
    let words: Vec<&str> = letters.split_whitespace().collect();
    match words.split_first() {
        Some((&"the", rest)) if !rest.is_empty() => rest.join(" "),
        _ => words.join(" "),
    }
}

/// The latitude and longitude, in degrees, of the point a map would name the country
/// at, or `None` if the table has no country of that name.
pub fn position(name: &str) -> Option<(f64, f64)> {
    static BY_NAME: OnceLock<HashMap<String, (f64, f64)>> = OnceLock::new();
    let by_name = BY_NAME.get_or_init(|| {
        TABLE
            .lines()
            .filter(|line| !line.starts_with('#'))
            .filter_map(|line| {
                let mut fields = line.split('\t').skip(2);
                let name = fields.next()?;
                let latitude = fields.next()?.parse().ok()?;
                let longitude = fields.next()?.parse().ok()?;
                Some((plain(name), (latitude, longitude)))
            })
            .collect()
    });
    by_name.get(&plain(name)).copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_country_is_found_by_its_name_however_it_is_written() {
        assert_eq!(position("Ukraine"), Some((49.725, 32.141)));
        assert_eq!(position("  saudi arabia "), position("Saudi Arabia"));
        assert_eq!(position("the Bahamas"), position("Bahamas"));
        assert!(position("United Arab Emirates").is_some());
        assert_eq!(position("Atlantis"), None);
        assert_eq!(position(""), None);
    }

    #[test]
    fn the_table_holds_the_worlds_countries() {
        let lines = TABLE.lines().filter(|line| !line.starts_with('#')).count();
        assert!(lines > 200, "{lines}");
        for name in [
            "France", "Russia", "Iran", "Sudan", "Mali", "Japan", "Brazil",
        ] {
            assert!(position(name).is_some(), "{name}");
        }
    }
}
