//! Armed Conflicts and Conflict Events, from the Uppsala Conflict Data Program's
//! record of organised violence, prepared by a job in the repository (ADR 0004). See
//! `tools/make_conflicts.py` for what is kept.
//!
//! UCDP records an incident only if at least one person died in it, publishes each
//! month about three weeks after it ends, and marks the records it has yet to check.

use std::time::Duration;

use orion_core::feature::Feature;
use orion_core::time::JulianDate;

use crate::listing::{Listing, ParseError};

const DAY: Duration = Duration::from_secs(24 * 60 * 60);

/// One mark for each conflict in each country, over the last three months on record.
pub const ARMED_CONFLICTS: Listing = Listing {
    source: "the Uppsala Conflict Data Program",
    url: concat!(
        env!("CARGO_PKG_REPOSITORY"),
        "/releases/download/extracts/armed_conflicts.tsv"
    ),
    accept: None,
    cache_name: "armed_conflicts.tsv",
    parse: parse_armed_conflicts,
    refresh_every: DAY,
};

/// Every incident of those three months that could be placed at least to a province.
pub const CONFLICT_EVENTS: Listing = Listing {
    source: "the Uppsala Conflict Data Program",
    url: concat!(
        env!("CARGO_PKG_REPOSITORY"),
        "/releases/download/extracts/conflict_events.tsv"
    ),
    accept: None,
    cache_name: "conflict_events.tsv",
    parse: parse_conflict_events,
    refresh_every: DAY,
};

/// Classes of Armed Conflict and of Conflict Event: the three kinds of organised
/// violence UCDP tells apart.
pub mod conflict_class {
    pub const INVOLVING_A_STATE: u8 = 0;
    pub const BETWEEN_ARMED_GROUPS: u8 = 1;
    pub const AGAINST_CIVILIANS: u8 = 2;
}

const VERSION: &str = "1";

/// The lines after a file's heading, and the days it covers. The heading names the
/// format, its version, when it was built, how many lines follow, and the first and
/// last day covered.
fn body<'a>(
    text: &'a str,
    format: &str,
    what: &str,
) -> Result<(std::str::Lines<'a>, String), ParseError> {
    let mut lines = text.lines();
    let heading: Vec<&str> = lines.next().unwrap_or_default().split('\t').collect();
    if heading.len() != 6 || heading[0] != format {
        return Err(ParseError(format!("this is not a file of {what}")));
    }
    if heading[1] != VERSION {
        return Err(ParseError(format!(
            "this file of {what} is version {}, and this version of Orion reads version {VERSION}",
            heading[1]
        )));
    }
    Ok((lines, format!("{} to {}", heading[4], heading[5])))
}

fn kind(word: &str) -> Option<(&'static str, u8)> {
    Some(match word {
        "state" => (
            "Fighting involving a state",
            conflict_class::INVOLVING_A_STATE,
        ),
        "groups" => (
            "Fighting between armed groups",
            conflict_class::BETWEEN_ARMED_GROUPS,
        ),
        "civilians" => ("Attack on civilians", conflict_class::AGAINST_CIVILIANS),
        _ => return None,
    })
}

/// Deaths as UCDP estimates them: its best figure, and the range it could be in.
fn deaths(best: &str, low: &str, high: &str) -> String {
    if low == high {
        best.to_owned()
    } else {
        format!("{best} (between {low} and {high})")
    }
}

/// Reads the file of Armed Conflicts the job publishes.
pub fn parse_armed_conflicts(text: &str) -> Result<Vec<Feature>, ParseError> {
    let (lines, period) = body(text, "#orion-armed-conflicts", "Armed Conflicts")?;
    Ok(lines
        .filter_map(|line| armed_conflict(line, &period))
        .collect())
}

/// One line: identifier, latitude, longitude, kind, country, name, sides, how many
/// incidents, and the best, low and high estimates of deaths.
fn armed_conflict(line: &str, period: &str) -> Option<Feature> {
    let fields: Vec<&str> = line.split('\t').collect();
    let [
        id,
        latitude,
        longitude,
        kind_word,
        country,
        name,
        sides,
        events,
        best,
        low,
        high,
    ] = fields[..]
    else {
        return None;
    };
    let (label, class) = kind(kind_word)?;
    // A conflict fought in several countries has a mark in each, so each says where.
    let title = if name.contains(country) {
        name.to_owned()
    } else {
        format!("{name} ({country})")
    };
    let mut conflict = Feature::at(id, title, latitude.parse().ok()?, longitude.parse().ok()?);
    conflict.class = class;
    conflict.size = best.parse().unwrap_or(0.0);
    conflict.row("Kind", label);
    conflict.row("Country", country);
    conflict.row("Between", sides);
    conflict.row("Period", period);
    conflict.row("Incidents recorded", events);
    conflict.row("Deaths estimated", deaths(best, low, high));
    conflict.is_on_earth().then_some(conflict)
}

/// Reads the file of Conflict Events the job publishes.
pub fn parse_conflict_events(text: &str) -> Result<Vec<Feature>, ParseError> {
    let (lines, _) = body(text, "#orion-conflict-events", "Conflict Events")?;
    Ok(lines.filter_map(conflict_event).collect())
}

/// One line: identifier, day, latitude, longitude, how exactly it is placed, kind,
/// country, place, conflict, the two sides, the best, low and high estimates of
/// deaths, and whether UCDP has checked the record.
fn conflict_event(line: &str) -> Option<Feature> {
    let fields: Vec<&str> = line.split('\t').collect();
    let [
        id,
        day,
        latitude,
        longitude,
        precision,
        kind_word,
        country,
        place,
        conflict,
        side_a,
        side_b,
        best,
        low,
        high,
        checked,
    ] = fields[..]
    else {
        return None;
    };
    let (label, class) = kind(kind_word)?;
    let whereabouts = if place.is_empty() { country } else { place };
    let mut event = Feature::at(
        id,
        format!("{label}: {whereabouts}"),
        latitude.parse().ok()?,
        longitude.parse().ok()?,
    );
    event.class = class;
    event.size = best.parse().unwrap_or(0.0);
    event.time = Some(JulianDate::parse_utc(&format!("{day} 00:00"))?);
    event.row("Kind", label);
    event.row("Day", day);
    event.row("Place", place);
    event.row("Country", country);
    event.row(
        "Placed",
        match precision {
            "1" => "at the place itself",
            "2" => "to within about 25 km",
            "3" => "only to its district",
            _ => "only to its province",
        },
    );
    event.row("Between", format!("{side_a} – {side_b}"));
    event.row("Conflict", conflict);
    event.row("Deaths estimated", deaths(best, low, high));
    if checked != "1" {
        event
            .details
            .push("The Uppsala Conflict Data Program has yet to check this record.".to_owned());
    }
    event.is_on_earth().then_some(event)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(feature: &Feature) -> Vec<(&str, &str)> {
        feature
            .rows
            .iter()
            .map(|row| (row.label.as_str(), row.value.as_str()))
            .collect()
    }

    const CONFLICTS: &str = "#orion-armed-conflicts\t1\t2026-11-25T23:00:00Z\t2\t2026-08-01\t2026-10-31\n\
        218/Ukraine\t48.5000\t37.5000\tstate\tUkraine\tRussia - Ukraine\t\
        Government of Russia (Soviet Union) – Government of Ukraine\t799\t18045\t17721\t18788\n\
        9001/Mali\t14.5000\t-4.0000\tcivilians\tMali\tIS - Civilians\tIS – Civilians\t12\t40\t40\t40\n";

    #[test]
    fn the_files_are_fetched_from_the_projects_own_release() {
        assert!(
            ARMED_CONFLICTS
                .url
                .ends_with("/releases/download/extracts/armed_conflicts.tsv")
        );
        assert!(
            CONFLICT_EVENTS
                .url
                .ends_with("/releases/download/extracts/conflict_events.tsv")
        );
    }

    #[test]
    fn an_armed_conflict_names_its_sides_and_its_toll_over_the_period() {
        let conflicts = parse_armed_conflicts(CONFLICTS).unwrap();
        assert_eq!(conflicts.len(), 2);
        let ukraine = &conflicts[0];
        assert_eq!(ukraine.name, "Russia - Ukraine");
        assert_eq!((ukraine.latitude_deg, ukraine.longitude_deg), (48.5, 37.5));
        assert_eq!(ukraine.class, conflict_class::INVOLVING_A_STATE);
        assert_eq!(ukraine.size, 18045.0);
        assert_eq!(
            rows(ukraine),
            [
                ("Kind", "Fighting involving a state"),
                ("Country", "Ukraine"),
                (
                    "Between",
                    "Government of Russia (Soviet Union) – Government of Ukraine"
                ),
                ("Period", "2026-08-01 to 2026-10-31"),
                ("Incidents recorded", "799"),
                ("Deaths estimated", "18045 (between 17721 and 18788)"),
            ]
        );
        // A conflict whose name does not say where is given its country.
        assert_eq!(conflicts[1].name, "IS - Civilians (Mali)");
        assert_eq!(conflicts[1].class, conflict_class::AGAINST_CIVILIANS);
        assert_eq!(rows(&conflicts[1])[5], ("Deaths estimated", "40"));
    }

    const EVENTS: &str = "#orion-conflict-events\t1\t2026-11-25T23:00:00Z\t2\t2026-08-01\t2026-10-31\n\
        650501\t2026-10-31\t15.88806\t45.55083\t2\tstate\tYemen (North Yemen)\tHuwayshiyan area\t\
        Yemen (North Yemen): Government\tGovernment of Yemen (North Yemen)\t\
        Forces of the Presidential Leadership Council\t2\t2\t2\t1\n\
        644900\t2026-10-12\t4.6\t-74.1\t4\tgroups\tColombia\t\tUnidentified armed groups in Colombia\t\
        an unidentified armed group\tan unidentified armed group\t3\t1\t6\t0\n";

    #[test]
    fn a_conflict_event_says_where_when_between_whom_and_how_surely() {
        let events = parse_conflict_events(EVENTS).unwrap();
        assert_eq!(events.len(), 2);
        let yemen = &events[0];
        assert_eq!(yemen.name, "Fighting involving a state: Huwayshiyan area");
        assert_eq!(
            (yemen.latitude_deg, yemen.longitude_deg),
            (15.88806, 45.55083)
        );
        assert_eq!(
            yemen.time,
            Some(JulianDate::from_utc(2026, 10, 31, 0, 0, 0.0))
        );
        assert_eq!(
            rows(yemen),
            [
                ("Kind", "Fighting involving a state"),
                ("Day", "2026-10-31"),
                ("Place", "Huwayshiyan area"),
                ("Country", "Yemen (North Yemen)"),
                ("Placed", "to within about 25 km"),
                (
                    "Between",
                    "Government of Yemen (North Yemen) – Forces of the Presidential Leadership Council"
                ),
                ("Conflict", "Yemen (North Yemen): Government"),
                ("Deaths estimated", "2"),
            ]
        );
        assert!(yemen.details.is_empty());

        // One placed only to its province, with no place named and not yet checked.
        let colombia = &events[1];
        assert_eq!(colombia.name, "Fighting between armed groups: Colombia");
        assert_eq!(colombia.class, conflict_class::BETWEEN_ARMED_GROUPS);
        assert_eq!(rows(colombia)[3], ("Placed", "only to its province"));
        assert_eq!(
            rows(colombia)[6],
            ("Deaths estimated", "3 (between 1 and 6)")
        );
        assert_eq!(
            colombia.details,
            ["The Uppsala Conflict Data Program has yet to check this record."]
        );
    }

    #[test]
    fn files_that_are_not_these_or_not_this_version_are_errors() {
        assert!(parse_armed_conflicts("").is_err());
        assert!(parse_conflict_events("<html>Not Found</html>").is_err());
        // Each reader takes only its own file.
        assert!(parse_armed_conflicts(EVENTS).is_err());
        assert!(parse_conflict_events(CONFLICTS).is_err());
        let newer = "#orion-conflict-events\t2\t2026-11-25T23:00:00Z\t0\t2026-08-01\t2026-10-31\n";
        assert!(
            parse_conflict_events(newer)
                .unwrap_err()
                .0
                .contains("version 2")
        );
        // Lines that cannot be read are passed over.
        let odd = "#orion-armed-conflicts\t1\t2026-11-25T23:00:00Z\t2\t2026-08-01\t2026-10-31\n\
                   too\tfew\n1/X\tnorth\t0\tstate\tX\tA - B\tA – B\t1\t1\t1\t1\n\
                   2/X\t0\t0\triot\tX\tA - B\tA – B\t1\t1\t1\t1\n";
        assert_eq!(parse_armed_conflicts(odd).unwrap(), Vec::new());
    }
}
