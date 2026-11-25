//! Unrest Reports: protests, protests with violence and strikes as the news reported
//! them, sifted from GDELT's index of the news by a job in the repository and
//! published as one small file (ADR 0004). See `tools/make_unrest.py` for what is
//! kept and why.

use std::time::Duration;

use orion_core::feature::Feature;
use orion_core::time::JulianDate;

use crate::listing::{Listing, ParseError};

/// The file of Unrest Reports, rebuilt every hour from the GDELT Project's index of
/// the world's news.
pub const UNREST_REPORTS: Listing = Listing {
    source: "the GDELT Project's index of the news",
    url: concat!(
        env!("CARGO_PKG_REPOSITORY"),
        "/releases/download/extracts/unrest.tsv"
    ),
    accept: None,
    cache_name: "unrest.tsv",
    parse: parse_unrest_reports,
    refresh_every: Duration::from_secs(30 * 60),
};

/// Classes of Unrest Report. The first three are for reports that at least two news
/// outlets carried; whatever only one carried is in the last, whatever its kind.
pub mod unrest_class {
    pub const PROTEST: u8 = 0;
    pub const VIOLENT: u8 = 1;
    pub const STRIKE: u8 = 2;
    pub const ONE_OUTLET: u8 = 3;
}

const HEADING: &str = "#orion-unrest";
const VERSION: &str = "1";
/// Shown with every report, because it is the thing most worth knowing about one.
const CAUTION: &str = "This is a report, not an observation. It was put together by machine \
     from news articles, is placed no more exactly than the town, and is sometimes wrong. It \
     does not say where a crowd is now, or what is planned.";

/// Reads the file the job publishes: a heading naming the format, its version, when
/// it was built and how many reports follow, then a line for each town and day.
pub fn parse_unrest_reports(text: &str) -> Result<Vec<Feature>, ParseError> {
    let mut lines = text.lines();
    let heading: Vec<&str> = lines.next().unwrap_or_default().split('\t').collect();
    if heading.len() != 4 || heading[0] != HEADING {
        return Err(ParseError(
            "this is not a file of Unrest Reports".to_owned(),
        ));
    }
    if heading[1] != VERSION {
        return Err(ParseError(format!(
            "this file of Unrest Reports is version {}, and this version of Orion reads version {VERSION}",
            heading[1]
        )));
    }
    Ok(lines.filter_map(report).collect())
}

/// One line: identifier, day, latitude, longitude, kind, place, how many outlets
/// carried it, and the addresses of up to five of their articles.
fn report(line: &str) -> Option<Feature> {
    let fields: Vec<&str> = line.split('\t').collect();
    let [id, day, latitude, longitude, kind, place, outlets, articles] = fields[..] else {
        return None;
    };
    let (label, class) = match kind {
        "protest" => ("Protest", unrest_class::PROTEST),
        "violent" => ("Protest with violence", unrest_class::VIOLENT),
        "strike" => ("Strike", unrest_class::STRIKE),
        _ => return None,
    };
    let outlets: u32 = outlets.parse().ok()?;
    let town = place.split(',').next().unwrap_or(place).trim();

    let mut report = Feature::at(
        id,
        format!("{label} reported in {town}"),
        latitude.parse().ok()?,
        longitude.parse().ok()?,
    );
    // The day's first moment, so a report of today is never ahead of the clock.
    report.time = Some(JulianDate::parse_utc(&format!("{day} 00:00"))?);
    report.size = outlets as f32;
    report.class = if outlets >= 2 {
        class
    } else {
        unrest_class::ONE_OUTLET
    };
    report.row("Kind", label);
    report.row("Place", place);
    report.row("Day", day);
    report.row(
        "Carried by",
        match outlets {
            1 => "1 news outlet".to_owned(),
            several => format!("{several} news outlets"),
        },
    );
    for (number, article) in articles
        .split(' ')
        .filter(|url| !url.is_empty())
        .enumerate()
    {
        report.row(&format!("Article {}", number + 1), article);
    }
    report.details.push(CAUTION.to_owned());
    report.is_on_earth().then_some(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILE: &str = "#orion-unrest\t1\t2026-11-25T19:05:00Z\t3\n\
        2026-11-25/-2707018\t2026-11-25\t6.4531\t3.3958\tprotest\tLagos, Lagos, Nigeria\t3\t\
        https://punchng.com/nama-pensioners-protest-today/ https://b.example/pensioners-protest-in-lagos\n\
        2026-11-25/-2305566\t2026-11-25\t-11.6667\t27.4833\tviolent\tLubumbashi, Katanga, Democratic Republic Of The Congo\t2\t\
        https://a.example/police-fire-tear-gas-at-bishops/\n\
        2026-11-24/-873530\t2026-11-24\t-33.4500\t-70.6667\tstrike\tSantiago, Chile\t1\t\
        https://a.example/teachers-national-strike\n";

    #[test]
    fn the_file_is_fetched_from_the_projects_own_release_every_half_hour() {
        assert!(UNREST_REPORTS.url.starts_with("https://github.com/"));
        assert!(
            UNREST_REPORTS
                .url
                .ends_with("/releases/download/extracts/unrest.tsv")
        );
        assert_eq!(UNREST_REPORTS.refresh_every, Duration::from_secs(1800));
    }

    #[test]
    fn a_report_says_what_where_when_and_who_carried_it() {
        let reports = parse_unrest_reports(FILE).unwrap();
        assert_eq!(reports.len(), 3);

        let lagos = &reports[0];
        assert_eq!(lagos.name, "Protest reported in Lagos");
        assert_eq!((lagos.latitude_deg, lagos.longitude_deg), (6.4531, 3.3958));
        assert_eq!(lagos.class, unrest_class::PROTEST);
        assert_eq!(lagos.size, 3.0);
        // The first moment of its day.
        assert_eq!(
            lagos.time,
            Some(JulianDate::from_utc(2026, 11, 25, 0, 0, 0.0))
        );
        let rows: Vec<(&str, &str)> = lagos
            .rows
            .iter()
            .map(|row| (row.label.as_str(), row.value.as_str()))
            .collect();
        assert_eq!(
            rows,
            [
                ("Kind", "Protest"),
                ("Place", "Lagos, Lagos, Nigeria"),
                ("Day", "2026-11-25"),
                ("Carried by", "3 news outlets"),
                (
                    "Article 1",
                    "https://punchng.com/nama-pensioners-protest-today/"
                ),
                ("Article 2", "https://b.example/pensioners-protest-in-lagos"),
            ]
        );
        // Every report carries the warning about what it is.
        assert!(lagos.details[0].starts_with("This is a report, not an observation."));

        assert_eq!(
            reports[1].name,
            "Protest with violence reported in Lubumbashi"
        );
        assert_eq!(reports[1].class, unrest_class::VIOLENT);
    }

    #[test]
    fn what_only_one_outlet_carried_is_kept_apart_whatever_its_kind() {
        let reports = parse_unrest_reports(FILE).unwrap();
        let santiago = &reports[2];
        assert_eq!(santiago.name, "Strike reported in Santiago");
        assert_eq!(santiago.class, unrest_class::ONE_OUTLET);
        assert_eq!(santiago.rows[3].value, "1 news outlet");
    }

    #[test]
    fn the_same_report_keeps_its_key_from_one_hour_to_the_next() {
        let first = parse_unrest_reports(FILE).unwrap();
        let again = parse_unrest_reports(FILE).unwrap();
        assert_eq!(first[0].key, again[0].key);
        assert_ne!(first[0].key, first[1].key);
    }

    #[test]
    fn files_that_are_not_unrest_reports_this_version_reads_are_errors() {
        assert!(parse_unrest_reports("").is_err());
        assert!(parse_unrest_reports("<html>Not Found</html>").is_err());
        assert!(parse_unrest_reports("#orion-extract\t1\tmines\t2026-11-25\t0\n").is_err());
        let newer = parse_unrest_reports("#orion-unrest\t2\t2026-11-25T19:05:00Z\t0\n");
        assert!(newer.unwrap_err().0.contains("version 2"));
        // An empty file is not an error: a quiet week is possible.
        let quiet = parse_unrest_reports("#orion-unrest\t1\t2026-11-25T19:05:00Z\t0\n");
        assert_eq!(quiet.unwrap(), Vec::new());
    }

    #[test]
    fn lines_that_cannot_be_read_are_passed_over() {
        let text = "#orion-unrest\t1\t2026-11-25T19:05:00Z\t4\n\
            a\t2026-11-25\t6.4\t3.3\tparade\tLagos\t2\thttps://a.example/x\n\
            b\t2026-11-25\tnorth\t3.3\tprotest\tLagos\t2\thttps://a.example/x\n\
            c\t2026-11-25\t96.4\t3.3\tprotest\tLagos\t2\thttps://a.example/x\n\
            d\tsoon\t6.4\t3.3\tprotest\tLagos\t2\thttps://a.example/x\n\
            too\tfew\n";
        assert_eq!(parse_unrest_reports(text).unwrap(), Vec::new());
    }
}
