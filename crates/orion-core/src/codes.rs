//! Names for the codes the Catalogue uses for owners and launch sites.
//!
//! The tables are CelesTrak's, from its "sources" and "launch sites" pages as they stood
//! on 25 November 2026, with one spelling corrected and three owners added that appear
//! in the Catalogue but not in CelesTrak's table (Jordan, Kuwait and Slovakia).

use std::collections::HashMap;
use std::sync::OnceLock;

type Table = HashMap<&'static str, &'static str>;

/// Reads a table of `CODE<tab>Name` lines.
fn table(text: &'static str) -> Table {
    text.lines()
        .filter_map(|line| line.split_once('\t'))
        .collect()
}

fn owners() -> &'static Table {
    static OWNERS: OnceLock<Table> = OnceLock::new();
    OWNERS.get_or_init(|| table(include_str!("../data/owners.tsv")))
}

fn launch_sites() -> &'static Table {
    static LAUNCH_SITES: OnceLock<Table> = OnceLock::new();
    LAUNCH_SITES.get_or_init(|| table(include_str!("../data/launch_sites.tsv")))
}

/// The country or organisation an owner code stands for, such as `US` or `ESA`.
pub fn owner_name(code: &str) -> Option<&'static str> {
    owners().get(code).copied()
}

/// The place a launch site code stands for, such as `AFETR`.
pub fn launch_site_name(code: &str) -> Option<&'static str> {
    launch_sites().get(code).copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_have_names() {
        assert_eq!(owner_name("US"), Some("United States"));
        assert_eq!(owner_name("ISS"), Some("International Space Station"));
        assert_eq!(owner_name("KWT"), Some("Kuwait"));
        assert_eq!(
            launch_site_name("AFETR"),
            Some("Air Force Eastern Test Range, Florida, USA")
        );
    }

    #[test]
    fn unknown_codes_have_no_name() {
        assert_eq!(owner_name("NOPE"), None);
        assert_eq!(launch_site_name(""), None);
    }

    #[test]
    fn every_line_of_the_tables_is_read() {
        let lines = |text: &str| text.lines().count();
        assert_eq!(owners().len(), lines(include_str!("../data/owners.tsv")));
        assert_eq!(
            launch_sites().len(),
            lines(include_str!("../data/launch_sites.tsv"))
        );
        assert!(owners().len() > 130 && launch_sites().len() > 40);
    }
}
