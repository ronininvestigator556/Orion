//! Travel Advisories: what the governments of the United States, the United Kingdom
//! and Canada each say about travelling to a country, joined into one small file by
//! a job in the repository (ADR 0004). See `tools/make_advisories.py`.

use std::time::Duration;

use orion_core::feature::Feature;

use crate::listing::{Listing, ParseError};

/// The file of Travel Advisories, rebuilt several times a day from what the three
/// governments publish.
pub const TRAVEL_ADVISORIES: Listing = Listing {
    source: "the governments of the United States, the United Kingdom and Canada",
    url: concat!(
        env!("CARGO_PKG_REPOSITORY"),
        "/releases/download/extracts/advisories.tsv"
    ),
    accept: None,
    cache_name: "advisories.tsv",
    parse: parse_travel_advisories,
    refresh_every: Duration::from_secs(6 * 60 * 60),
};

/// Classes of Travel Advisory: the United States' level, from 1 (the usual care) to
/// 4 (do not go), and one for countries it sets no level for.
pub mod advisory_class {
    pub const LEVEL_1: u8 = 0;
    pub const LEVEL_2: u8 = 1;
    pub const LEVEL_3: u8 = 2;
    pub const LEVEL_4: u8 = 3;
    pub const NO_US_LEVEL: u8 = 4;
}

const HEADING: &str = "#orion-advisories";
const VERSION: &str = "1";

/// Reads the file the job publishes: a heading naming the format, its version, when
/// it was built and how many countries follow, then a line for each country.
pub fn parse_travel_advisories(text: &str) -> Result<Vec<Feature>, ParseError> {
    let mut lines = text.lines();
    let heading: Vec<&str> = lines.next().unwrap_or_default().split('\t').collect();
    if heading.len() != 4 || heading[0] != HEADING {
        return Err(ParseError(
            "this is not a file of Travel Advisories".to_owned(),
        ));
    }
    if heading[1] != VERSION {
        return Err(ParseError(format!(
            "this file of Travel Advisories is version {}, and this version of Orion reads version {VERSION}",
            heading[1]
        )));
    }
    Ok(lines.filter_map(advisory).collect())
}

/// One line: the country's code, name and place, then what each government says:
/// the United States' level, advice, date, page and summary; the United Kingdom's
/// latest change, date and page; Canada's advice, date and page. A government with
/// nothing to say about a country leaves its fields empty.
fn advisory(line: &str) -> Option<Feature> {
    let fields: Vec<&str> = line.split('\t').collect();
    let [
        code,
        name,
        latitude,
        longitude,
        us_level,
        us_advice,
        us_day,
        us_page,
        us_summary,
        uk_change,
        uk_day,
        uk_page,
        canada_advice,
        canada_day,
        canada_page,
    ] = fields[..]
    else {
        return None;
    };
    let mut advisory = Feature::at(code, name, latitude.parse().ok()?, longitude.parse().ok()?);
    let level: Option<u8> = us_level
        .parse()
        .ok()
        .filter(|level| (1..=4).contains(level));
    advisory.size = f32::from(level.unwrap_or(0));
    advisory.class = level.map_or(advisory_class::NO_US_LEVEL, |level| level - 1);

    if let Some(level) = level {
        advisory.row("United States", format!("Level {level}: {us_advice}"));
    }
    advisory.row("U.S. advice from", us_day);
    advisory.row("U.S. page", us_page);
    advisory.row("United Kingdom", uk_change);
    advisory.row("UK advice changed", uk_day);
    advisory.row("UK page", uk_page);
    advisory.row("Canada", canada_advice);
    advisory.row("Canadian advice from", canada_day);
    advisory.row("Canadian page", canada_page);
    if !us_summary.is_empty() {
        advisory
            .details
            .push(format!("The United States says: {us_summary}"));
    }
    advisory.is_on_earth().then_some(advisory)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILE: &str = "#orion-advisories\t1\t2026-11-25T20:00:00Z\t2\n\
        FR\tFrance\t46.696\t2.552\t2\tExercise Increased Caution\t2026-10-20\t\
        https://travel.state.gov/destination.fra.html\t\
        Exercise increased caution in France due to terrorism and unrest.\t\
        Updated information about demonstrations.\t2026-10-22\t\
        https://www.gov.uk/foreign-travel-advice/france\t\
        Exercise a high degree of caution\t2026-11-25\thttps://travel.gc.ca/destinations/france\n\
        US\tUnited States of America\t39.538\t-97.483\t\t\t\t\t\t\
        Latest update: entry rules.\t2026-09-01\thttps://www.gov.uk/foreign-travel-advice/usa\t\
        Take normal security precautions\t2026-11-20\thttps://travel.gc.ca/destinations/united-states\n";

    fn rows(feature: &Feature) -> Vec<(&str, &str)> {
        feature
            .rows
            .iter()
            .map(|row| (row.label.as_str(), row.value.as_str()))
            .collect()
    }

    #[test]
    fn the_file_is_fetched_from_the_projects_own_release() {
        assert!(
            TRAVEL_ADVISORIES
                .url
                .ends_with("/releases/download/extracts/advisories.tsv")
        );
        assert_eq!(TRAVEL_ADVISORIES.refresh_every, Duration::from_secs(21_600));
    }

    #[test]
    fn a_country_shows_what_each_government_says_with_where_to_read_it() {
        let advisories = parse_travel_advisories(FILE).unwrap();
        assert_eq!(advisories.len(), 2);
        let france = &advisories[0];
        assert_eq!(france.name, "France");
        assert_eq!((france.latitude_deg, france.longitude_deg), (46.696, 2.552));
        assert_eq!(france.class, advisory_class::LEVEL_2);
        assert_eq!(france.size, 2.0);
        assert_eq!(
            rows(france),
            [
                ("United States", "Level 2: Exercise Increased Caution"),
                ("U.S. advice from", "2026-10-20"),
                ("U.S. page", "https://travel.state.gov/destination.fra.html"),
                (
                    "United Kingdom",
                    "Updated information about demonstrations."
                ),
                ("UK advice changed", "2026-10-22"),
                ("UK page", "https://www.gov.uk/foreign-travel-advice/france"),
                ("Canada", "Exercise a high degree of caution"),
                ("Canadian advice from", "2026-11-25"),
                ("Canadian page", "https://travel.gc.ca/destinations/france"),
            ]
        );
        assert_eq!(
            france.details,
            [
                "The United States says: Exercise increased caution in France due to terrorism and unrest."
            ]
        );
    }

    #[test]
    fn a_country_the_united_states_sets_no_level_for_shows_the_others_advice() {
        let advisories = parse_travel_advisories(FILE).unwrap();
        let home = &advisories[1];
        assert_eq!(home.class, advisory_class::NO_US_LEVEL);
        assert_eq!(home.size, 0.0);
        assert_eq!(
            rows(home)[0],
            ("United Kingdom", "Latest update: entry rules.")
        );
        assert!(home.details.is_empty());
    }

    #[test]
    fn files_that_are_not_travel_advisories_this_version_reads_are_errors() {
        assert!(parse_travel_advisories("").is_err());
        assert!(parse_travel_advisories("#orion-unrest\t1\t2026-11-25T19:05:00Z\t0\n").is_err());
        let newer = parse_travel_advisories("#orion-advisories\t2\t2026-11-25T20:00:00Z\t0\n");
        assert!(newer.unwrap_err().0.contains("version 2"));
        // Lines that cannot be read are passed over.
        let odd = "#orion-advisories\t1\t2026-11-25T20:00:00Z\t2\nFR\tFrance\tnorth\t2.5\n\
                   XX\tNowhere\t95.0\t0.0\t\t\t\t\t\t\t\t\t\t\t\n";
        assert_eq!(parse_travel_advisories(odd).unwrap(), Vec::new());
    }
}
