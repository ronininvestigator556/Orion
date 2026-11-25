//! Warnings that authorities publish to keep others clear of military activity.
//!
//! Airspace Warnings are the European Union Aviation Safety Agency's Conflict Zone
//! Information Bulletins: its notices to airlines that a country's airspace is
//! dangerous to fly through. The Agency allows them to be reproduced with
//! acknowledgement.

use std::time::Duration;

use orion_core::feature::Feature;
use orion_core::time::JulianDate;
use serde_json::Value;

use crate::countries;
use crate::listing::{Listing, ParseError};

/// The Agency's list of its bulletins, those in force and those withdrawn.
pub const AIRSPACE_WARNINGS: Listing = Listing {
    source: "the European Union Aviation Safety Agency",
    url: "https://www.easa.europa.eu/en/domains/air-operations/czibs/export-json?page&_format=json",
    accept: None,
    cache_name: "airspace_warnings.json",
    parse: parse_airspace_warnings,
    refresh_every: Duration::from_secs(6 * 60 * 60),
};

/// What every Airspace Warning has to be read with.
const MEANING: &str = "A notice to airlines that flying through this airspace is dangerous \
     because of a conflict. It speaks for the airspace named, not for the point it is marked \
     at, and says nothing about the ground beneath.";

/// Reads the Agency's list. Bulletins that have been withdrawn are left out, and one
/// that covers several countries is marked in each.
pub fn parse_airspace_warnings(text: &str) -> Result<Vec<Feature>, ParseError> {
    let not_bulletins = || ParseError("this is not a list of airspace warnings".to_owned());
    let document: Value = serde_json::from_str(text).map_err(|_| not_bulletins())?;
    let bulletins = document["conflict_zones"]
        .as_array()
        .ok_or_else(not_bulletins)?;
    Ok(bulletins.iter().flat_map(airspace_warnings).collect())
}

/// A day written `31/01/2027`, as the last moment of that day.
fn day_month_year(text: &str) -> Option<JulianDate> {
    let mut parts = text.trim().split('/');
    let (day, month, year) = (parts.next()?, parts.next()?, parts.next()?);
    JulianDate::parse_utc(&format!("{year}-{month}-{day} 23:59"))
}

fn airspace_warnings(bulletin: &Value) -> Vec<Feature> {
    let text = |name: &str| bulletin[name].as_str().unwrap_or_default().trim();
    if text("status") != "Active" || text("Nid").is_empty() {
        return Vec::new();
    }
    // Where the Agency gives a place, it is used. Often it gives none.
    let given = text("coordinates").split_once(',').and_then(|(lat, lon)| {
        Some((
            lat.trim().parse::<f64>().ok()?,
            lon.trim().parse::<f64>().ok()?,
        ))
    });
    let countries: Vec<&str> = text("country")
        .split(',')
        .map(str::trim)
        .filter(|country| !country.is_empty())
        .collect();
    let several = countries.len() > 1;

    countries
        .into_iter()
        .filter_map(|country| {
            let (latitude, longitude) = given
                .filter(|_| !several)
                .or_else(|| countries::position(country))?;
            let id = format!("{}/{country}", text("Nid"));
            let mut warning = Feature::at(
                &id,
                format!("Airspace warning: {country}"),
                latitude,
                longitude,
            );
            warning.until = day_month_year(text("valid_until_date"));
            warning.row("Covers", text("name"));
            warning.row("Country", country);
            warning.row(
                "First issued",
                text("issued_date").get(..10).unwrap_or_default(),
            );
            // The date it was last revised sits inside a scrap of markup.
            let revised = text("updated")
                .split_once("datetime=\"")
                .and_then(|(_, rest)| rest.get(..10));
            warning.row("Last revised", revised.unwrap_or_default());
            if warning.until.is_some() {
                let until = text("valid_until_date");
                warning.row(
                    "In force until",
                    format!("{until}, unless reviewed earlier"),
                );
            }
            warning.row(
                "Bulletin",
                format!("https://www.easa.europa.eu/en/node/{}", text("Nid")),
            );
            warning.details.push(MEANING.to_owned());
            warning.is_on_earth().then_some(warning)
        })
        .collect()
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

    const BULLETINS: &str = r#"{"conflict_zones":[
      {"Nid":"20599","issued_date":"2017-03-31T00:00:00+0300","valid_until_date":"30/04/2027",
       "name":"Airspace of Syria","status":"Active","country":"Syria",
       "coordinates":"33.5130695, 36.3095814",
       "updated":"<time datetime=\"2026-11-16T09:43:12+02:00\">2026-11-16T09:43:12+0200</time>\n"},
      {"Nid":"136057","issued_date":"2022-02-24T00:00:00+0200","valid_until_date":"31/01/2027",
       "name":"Airspace of Ukraine","status":"Active","country":"Ukraine","coordinates":"",
       "updated":"<time datetime=\"2026-07-24T14:07:23+03:00\">x</time>"},
      {"Nid":"143899","issued_date":"2026-07-14T00:00:00+0300","valid_until_date":"31/01/2027",
       "name":"Airspace of the Persian Gulf and Gulf of Oman","status":"Active",
       "country":"Bahrain, Kuwait, Atlantis","coordinates":"","updated":""},
      {"Nid":"20001","issued_date":"2019-01-01T00:00:00+0200","valid_until_date":"01/01/2020",
       "name":"Airspace of Egypt","status":"Withdrawn","country":"Egypt","coordinates":"30.0, 31.2"}
    ]}"#;

    #[test]
    fn a_warning_in_force_says_what_it_covers_and_until_when() {
        let warnings = parse_airspace_warnings(BULLETINS).unwrap();
        let names: Vec<&str> = warnings
            .iter()
            .map(|warning| warning.name.as_str())
            .collect();
        // The withdrawn one is left out, and a country the table lacks is passed over.
        assert_eq!(
            names,
            [
                "Airspace warning: Syria",
                "Airspace warning: Ukraine",
                "Airspace warning: Bahrain",
                "Airspace warning: Kuwait"
            ]
        );
        let syria = &warnings[0];
        assert_eq!(
            (syria.latitude_deg, syria.longitude_deg),
            (33.5130695, 36.3095814)
        );
        let last_minute = JulianDate::from_utc(2027, 4, 30, 23, 59, 0.0);
        assert!((syria.until.unwrap().0 - last_minute.0).abs() < 1e-8);
        assert_eq!(
            rows(syria),
            [
                ("Covers", "Airspace of Syria"),
                ("Country", "Syria"),
                ("First issued", "2017-03-31"),
                ("Last revised", "2026-11-16"),
                ("In force until", "30/04/2027, unless reviewed earlier"),
                ("Bulletin", "https://www.easa.europa.eu/en/node/20599"),
            ]
        );
        assert!(syria.details[0].contains("not for the point it is marked at"));
    }

    #[test]
    fn a_warning_given_no_place_is_marked_where_its_country_is() {
        let warnings = parse_airspace_warnings(BULLETINS).unwrap();
        let ukraine = &warnings[1];
        assert_eq!(
            Some((ukraine.latitude_deg, ukraine.longitude_deg)),
            countries::position("Ukraine")
        );
        // One bulletin for several countries is marked in each, each its own Feature.
        assert_eq!(
            rows(&warnings[2])[0].1,
            "Airspace of the Persian Gulf and Gulf of Oman"
        );
        assert_ne!(warnings[2].key, warnings[3].key);
        assert_eq!(
            Some((warnings[3].latitude_deg, warnings[3].longitude_deg)),
            countries::position("Kuwait")
        );
    }

    #[test]
    fn answers_that_are_not_the_list_are_errors() {
        assert!(parse_airspace_warnings("").is_err());
        assert!(parse_airspace_warnings("<html>Busy</html>").is_err());
        assert!(parse_airspace_warnings("{\"features\": []}").is_err());
        // No warnings in force is not an error.
        assert_eq!(
            parse_airspace_warnings("{\"conflict_zones\": []}").unwrap(),
            Vec::new()
        );
        assert_eq!(day_month_year("31/01/2027").map(|_| ()), Some(()));
        assert_eq!(day_month_year("soon"), None);
    }
}
