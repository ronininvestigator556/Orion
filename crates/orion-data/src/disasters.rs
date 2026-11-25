//! Warnings of disasters, as the bodies that watch for them publish them:
//!
//! - GDACS, the Global Disaster Alert and Coordination System of the United Nations
//!   and the European Commission: earthquakes, cyclones, floods, droughts, wildfires
//!   and eruptions anywhere, each given a level by how much help it is likely to need
//!   from abroad. Published under the Commission's notice, CC BY 4.0.
//! - The two U.S. tsunami warning centres, which between them watch the Pacific, the
//!   Caribbean and North America's coasts. Public domain.
//! - The U.S. Geological Survey's alert levels for the volcanoes of the United States.
//!   Public domain.
//!
//! None of these replaces a country's own authorities, and GDACS says so itself: its
//! levels come from models and are not checked by a person before they go out.

use std::time::Duration;

use orion_core::feature::Feature;
use orion_core::time::JulianDate;
use serde_json::Value;

use crate::hazards::format_moment;
use crate::listing::{Listing, ParseError};

/// Classes of disaster alert, by GDACS's own level.
pub mod disaster_class {
    pub const RED: u8 = 0;
    pub const ORANGE: u8 = 1;
    pub const GREEN: u8 = 2;
}

/// Classes of tsunami message, gravest first.
pub mod tsunami_class {
    pub const WARNING: u8 = 0;
    pub const WATCH_OR_ADVISORY: u8 = 1;
    pub const INFORMATION: u8 = 2;
}

/// Classes of volcano, by its alert level.
pub mod volcano_class {
    pub const WARNING: u8 = 0;
    pub const WATCH: u8 = 1;
    pub const ADVISORY: u8 = 2;
    pub const NORMAL: u8 = 3;
}

pub const DISASTERS: Listing = Listing {
    source: "GDACS, the United Nations and European Commission's disaster alert system",
    url: "https://www.gdacs.org/xml/rss.xml",
    accept: None,
    cache_name: "disasters.xml",
    parse: parse_disasters,
    refresh_every: Duration::from_secs(30 * 60),
};

/// Messages from the Pacific Tsunami Warning Center in Hawaii.
pub const TSUNAMI_PACIFIC: Listing = Listing {
    source: "the U.S. Pacific Tsunami Warning Center",
    url: "https://www.tsunami.gov/events/xml/PHEBAtom.xml",
    accept: None,
    cache_name: "tsunami_pacific.xml",
    parse: parse_tsunami_messages,
    refresh_every: Duration::from_secs(5 * 60),
};

/// Messages from the National Tsunami Warning Center in Alaska.
pub const TSUNAMI_NORTH_AMERICA: Listing = Listing {
    source: "the U.S. National Tsunami Warning Center",
    url: "https://www.tsunami.gov/events/xml/PAAQAtom.xml",
    accept: None,
    cache_name: "tsunami_north_america.xml",
    parse: parse_tsunami_messages,
    refresh_every: Duration::from_secs(5 * 60),
};

pub const VOLCANOES: Listing = Listing {
    source: "the U.S. Geological Survey's Volcano Hazards Program",
    url: "https://volcanoes.usgs.gov/vsc/api/volcanoApi/geojson",
    accept: None,
    cache_name: "volcanoes.geojson",
    parse: parse_volcanoes,
    refresh_every: Duration::from_secs(30 * 60),
};

type Node<'a, 'input> = roxmltree::Node<'a, 'input>;

/// The text of the first element directly inside `node` with the given name. Names
/// are matched without their prefix.
fn child_text<'a>(node: Node<'a, '_>, name: &str) -> Option<&'a str> {
    node.children()
        .find(|child| child.is_element() && child.tag_name().name() == name)
        .and_then(|child| child.text())
        .map(str::trim)
        .filter(|text| !text.is_empty())
}

/// Reads a moment written the way news feeds write them, such as
/// `Fri, 03 Jul 2026 12:57:44 GMT`. Only GMT is read, which is all GDACS writes.
fn parse_feed_moment(text: &str) -> Option<JulianDate> {
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let after_weekday = text.split_once(',').map_or(text, |(_, rest)| rest);
    let mut parts = after_weekday.split_whitespace();
    let day: u32 = parts.next()?.parse().ok()?;
    let month = MONTHS
        .iter()
        .position(|name| Some(*name) == parts.clone().next())?
        + 1;
    parts.next();
    let year: i32 = parts.next()?.parse().ok()?;
    let time = parts.next()?;
    if !matches!(parts.next(), Some("GMT" | "UTC" | "Z")) {
        return None;
    }
    JulianDate::parse_utc(&format!("{year:04}-{month:02}-{day:02} {time}"))
}

/// Reads GDACS's list of the disasters it is following.
pub fn parse_disasters(text: &str) -> Result<Vec<Feature>, ParseError> {
    let not_alerts = |_| ParseError("this is not a list of disaster alerts".to_owned());
    let document = roxmltree::Document::parse(text).map_err(not_alerts)?;
    let is_feed = document
        .descendants()
        .any(|node| node.is_element() && node.tag_name().name() == "channel");
    if !is_feed {
        return Err(ParseError(
            "this is not a list of disaster alerts".to_owned(),
        ));
    }
    Ok(document
        .descendants()
        .filter(|node| node.is_element() && node.tag_name().name() == "item")
        .filter_map(disaster)
        .collect())
}

fn disaster(item: Node<'_, '_>) -> Option<Feature> {
    let id = child_text(item, "guid")?;
    // The place is written latitude then longitude, anywhere under the item.
    let place = item
        .descendants()
        .find(|node| node.is_element() && node.tag_name().name() == "point")
        .and_then(|point| point.text())?;
    let mut place = place.split_whitespace();
    let latitude: f64 = place.next()?.parse().ok()?;
    let longitude: f64 = place.next()?.parse().ok()?;

    let kind = match child_text(item, "eventtype")? {
        "EQ" => "Earthquake",
        "TC" => "Tropical cyclone",
        "FL" => "Flood",
        "VO" => "Volcanic eruption",
        "DR" => "Drought",
        "WF" => "Wildfire",
        "TS" => "Tsunami",
        _ => "Disaster",
    };
    let level = child_text(item, "alertlevel").unwrap_or("Green");
    let class = match level {
        "Red" => disaster_class::RED,
        "Orange" => disaster_class::ORANGE,
        _ => disaster_class::GREEN,
    };
    let country = child_text(item, "country").unwrap_or_default();
    let whereabouts = child_text(item, "eventname")
        .or((!country.is_empty()).then_some(country))
        .unwrap_or("place not named");

    let mut alert = Feature::at(id, format!("{kind}: {whereabouts}"), latitude, longitude);
    alert.class = class;
    alert.size = f32::from(2 - class);
    alert.time = child_text(item, "fromdate").and_then(parse_feed_moment);
    alert.until = child_text(item, "todate").and_then(parse_feed_moment);
    alert.row("Kind", kind);
    alert.row("Level", level);
    alert.row("Where", country);
    alert.row(
        "How severe",
        child_text(item, "severity").unwrap_or_default(),
    );
    alert.row(
        "People affected",
        child_text(item, "population").unwrap_or_default(),
    );
    if let Some(from) = alert.time {
        alert.row("From", format_moment(from));
    }
    if let Some(to) = alert.until {
        alert.row("Last report", format_moment(to));
    }
    alert.row("Report", child_text(item, "link").unwrap_or_default());
    if let Some(title) = child_text(item, "title") {
        alert.details.push(title.to_owned());
    }
    alert.is_on_earth().then_some(alert)
}

/// The words after `label` in `text`, up to whichever of `ends` comes first.
fn after<'a>(text: &'a str, label: &str, ends: &[&str]) -> Option<&'a str> {
    let rest = &text[text.find(label)? + label.len()..];
    let end = ends
        .iter()
        .filter_map(|end| rest.find(end))
        .min()
        .unwrap_or(rest.len());
    Some(rest[..end].trim()).filter(|found| !found.is_empty())
}

/// Reads a tsunami warning centre's list of its latest messages. Most say only that
/// an earthquake poses no danger.
pub fn parse_tsunami_messages(text: &str) -> Result<Vec<Feature>, ParseError> {
    let not_messages = || ParseError("this is not a list of tsunami messages".to_owned());
    let document = roxmltree::Document::parse(text).map_err(|_| not_messages())?;
    let feed = document.root_element();
    if feed.tag_name().name() != "feed" {
        return Err(not_messages());
    }
    Ok(feed
        .children()
        .filter(|node| node.is_element() && node.tag_name().name() == "entry")
        .filter_map(tsunami_message)
        .collect())
}

fn tsunami_message(entry: Node<'_, '_>) -> Option<Feature> {
    let id = child_text(entry, "id")?;
    let region = child_text(entry, "title")?;
    let latitude: f64 = child_text(entry, "lat")?.parse().ok()?;
    let longitude: f64 = child_text(entry, "long")?.parse().ok()?;

    // The message itself is a block of marked-up text: every word of it, in order.
    let summary: String = entry
        .children()
        .find(|child| child.is_element() && child.tag_name().name() == "summary")
        .map(|summary| {
            summary
                .descendants()
                .filter_map(|node| node.text().filter(|_| node.is_text()))
                .collect::<Vec<_>>()
                .join(" ")
        })
        .unwrap_or_default();
    let summary = summary.split_whitespace().collect::<Vec<_>>().join(" ");
    let category = after(
        &summary,
        "Category:",
        &["Bulletin Issue Time", "Preliminary"],
    )
    .unwrap_or("Message");
    let class = match category.to_lowercase().as_str() {
        "warning" => tsunami_class::WARNING,
        "watch" | "advisory" => tsunami_class::WATCH_OR_ADVISORY,
        _ => tsunami_class::INFORMATION,
    };

    let mut message = Feature::at(
        id,
        format!("Tsunami {}: {region}", category.to_lowercase()),
        latitude,
        longitude,
    );
    message.class = class;
    message.time = child_text(entry, "updated").and_then(JulianDate::parse_rfc3339);
    message.row("Category", category);
    message.row("Earthquake near", region);
    message.row(
        "Magnitude",
        after(
            &summary,
            "Preliminary Magnitude:",
            &["Lat/Lon", "Affected Region"],
        )
        .unwrap_or_default(),
    );
    if let Some(issued) = message.time {
        message.row("Issued", format_moment(issued));
    }
    let bulletin = entry
        .children()
        .filter(|child| child.is_element() && child.tag_name().name() == "link")
        .find(|link| link.attribute("title") == Some("Bulletin"))
        .and_then(|link| link.attribute("href"));
    message.row("Bulletin", bulletin.unwrap_or_default().trim());
    if let Some(note) = after(&summary, "Note:", &["Definition:", "View bulletin"]) {
        message
            .details
            .push(note.trim_start_matches(['*', ' ']).to_owned());
    }
    message.is_on_earth().then_some(message)
}

/// Reads the Survey's list of U.S. volcanoes with their alert levels. Volcanoes that
/// are not watched closely enough to be given a level are left out.
pub fn parse_volcanoes(text: &str) -> Result<Vec<Feature>, ParseError> {
    let not_volcanoes = || ParseError("this is not a list of volcanoes".to_owned());
    let document: Value = serde_json::from_str(text).map_err(|_| not_volcanoes())?;
    let volcanoes = document["features"].as_array().ok_or_else(not_volcanoes)?;
    Ok(volcanoes.iter().filter_map(volcano).collect())
}

fn volcano(entry: &Value) -> Option<Feature> {
    let facts = &entry["properties"];
    let name = facts["volcanoName"].as_str()?;
    let coordinates = &entry["geometry"]["coordinates"];
    let (longitude, latitude) = (coordinates.get(0)?.as_f64()?, coordinates.get(1)?.as_f64()?);
    let (level, class) = match facts["alertLevel"].as_str()? {
        "WARNING" => ("Warning", volcano_class::WARNING),
        "WATCH" => ("Watch", volcano_class::WATCH),
        "ADVISORY" => ("Advisory", volcano_class::ADVISORY),
        "NORMAL" => ("Normal", volcano_class::NORMAL),
        _ => return None,
    };
    let id = facts["vnum"].as_str().unwrap_or(name);
    let title = if class == volcano_class::NORMAL {
        name.to_owned()
    } else {
        format!("{name}: volcano {}", level.to_lowercase())
    };
    let mut volcano = Feature::at(id, title, latitude, longitude);
    volcano.class = class;
    volcano.size = f32::from(3 - class);
    volcano.row("Alert level", level);
    let colour = facts["colorCode"]
        .as_str()
        .unwrap_or_default()
        .to_lowercase();
    volcano.row("Aviation colour", colour);
    volcano.row("Region", facts["region"].as_str().unwrap_or_default());
    volcano.row("Since", facts["alertDate"].as_str().unwrap_or_default());
    volcano.row("Notice", facts["noticeUrl"].as_str().unwrap_or_default());
    if let Some(synopsis) = facts["noticeSynopsis"]
        .as_str()
        .filter(|text| !text.is_empty())
    {
        volcano.details.push(synopsis.to_owned());
    }
    volcano.is_on_earth().then_some(volcano)
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

    const GDACS: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<rss version="2.0" xmlns:georss="http://www.georss.org/georss" xmlns:gdacs="http://www.gdacs.org" xmlns:geo="http://www.w3.org/2003/01/geo/wgs84_pos#">
<channel><title>GDACS RSS information</title>
<item>
 <title>Drought is on going in Burundi, Kenya</title>
 <link>http://www.gdacs.org/report.aspx?eventtype=DR&amp;eventid=1027450</link>
 <gdacs:fromdate>Thu, 21 May 2026 00:00:00 GMT</gdacs:fromdate>
 <gdacs:todate>Mon, 23 Nov 2026 00:00:00 GMT</gdacs:todate>
 <guid isPermaLink="false">DR1027450</guid>
 <geo:Point><geo:lat>6.5</geo:lat><geo:long>39.5</geo:long></geo:Point>
 <georss:point>6.5 39.5</georss:point>
 <gdacs:eventtype>DR</gdacs:eventtype>
 <gdacs:alertlevel>Orange</gdacs:alertlevel>
 <gdacs:eventname>East Africa, Central Sahel-2026</gdacs:eventname>
 <gdacs:severity unit="km2" value="1435673">Medium impact for agricultural drought in 1435673 km2</gdacs:severity>
 <gdacs:population unit="" value="0"></gdacs:population>
 <gdacs:country>Burundi, Kenya</gdacs:country>
</item>
<item>
 <title>Green forest fire alert in Australia</title>
 <guid isPermaLink="false">WF1024001</guid>
 <georss:point>-33.1 150.2</georss:point>
 <gdacs:eventtype>WF</gdacs:eventtype>
 <gdacs:alertlevel>Green</gdacs:alertlevel>
 <gdacs:eventname></gdacs:eventname>
 <gdacs:country>Australia</gdacs:country>
</item>
<item><title>Nowhere</title><guid>X1</guid><gdacs:eventtype>FL</gdacs:eventtype></item>
</channel></rss>"#;

    #[test]
    fn a_disaster_alert_says_what_where_and_how_grave() {
        let alerts = parse_disasters(GDACS).unwrap();
        // The one with no place is passed over.
        assert_eq!(alerts.len(), 2);
        let drought = &alerts[0];
        assert_eq!(drought.name, "Drought: East Africa, Central Sahel-2026");
        assert_eq!((drought.latitude_deg, drought.longitude_deg), (6.5, 39.5));
        assert_eq!(drought.class, disaster_class::ORANGE);
        assert_eq!(
            drought.time,
            Some(JulianDate::from_utc(2026, 5, 21, 0, 0, 0.0))
        );
        assert_eq!(
            rows(drought),
            [
                ("Kind", "Drought"),
                ("Level", "Orange"),
                ("Where", "Burundi, Kenya"),
                (
                    "How severe",
                    "Medium impact for agricultural drought in 1435673 km2"
                ),
                ("From", "2026-05-21 00:00 UTC"),
                ("Last report", "2026-11-23 00:00 UTC"),
                (
                    "Report",
                    "http://www.gdacs.org/report.aspx?eventtype=DR&eventid=1027450"
                ),
            ]
        );
        assert_eq!(drought.details, ["Drought is on going in Burundi, Kenya"]);
        // One with no name of its own is named by its country.
        assert_eq!(alerts[1].name, "Wildfire: Australia");
        assert_eq!(alerts[1].class, disaster_class::GREEN);
    }

    #[test]
    fn feed_moments_are_read_in_gmt_only() {
        assert_eq!(
            parse_feed_moment("Fri, 03 Jul 2026 12:57:44 GMT"),
            Some(JulianDate::from_utc(2026, 7, 3, 12, 57, 44.0))
        );
        assert_eq!(
            parse_feed_moment("3 Jan 2027 00:00:00 GMT"),
            Some(JulianDate::from_utc(2027, 1, 3, 0, 0, 0.0))
        );
        assert_eq!(parse_feed_moment("Fri, 03 Jul 2026 12:57:44 +0200"), None);
        assert_eq!(parse_feed_moment("Fri, 03 Foo 2026 12:57:44 GMT"), None);
        assert_eq!(parse_feed_moment("yesterday"), None);
    }

    const TSUNAMI: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<feed xmlns="http://www.w3.org/2005/Atom" xmlns:geo="http://www.w3.org/2003/01/geo/wgs84_pos#">
 <id>urn:uuid:feed</id><title>Tsunami Information Statement Number 1</title>
 <updated>2026-11-24T16:43:10Z</updated>
 <entry>
  <title>40 miles SW of Palmer, Alaska</title><updated>2026-11-24T16:43:10Z</updated>
  <geo:lat>61.300</geo:lat>
  <geo:long>-150.100</geo:long>
  <summary type="xhtml"><div xmlns="http://www.w3.org/1999/xhtml">
   <strong>Category:</strong> Information<br/><strong>Bulletin Issue Time: </strong> 2026.11.24 16:43:10 UTC <br/><strong>Preliminary Magnitude: </strong>4.1(Mwp)<br/><strong>Lat/Lon: </strong>61.300 / -150.100<br/><strong>Affected Region: </strong>40 miles SW of Palmer, Alaska<br/><b>Note:</b>  * There is NO tsunami danger from this earthquake.<br/><strong>Definition: </strong>An information statement. <a href="https://www.tsunami.gov/x.txt">View bulletin</a>  </div></summary>
  <id>urn:uuid:f1542e41</id>
  <link rel="related" title="CapXML document" href="https://www.tsunami.gov/events/PAAQ/cap.xml" type="application/cap+xml" />
  <link rel="alternate" title="Bulletin" href="https://www.tsunami.gov/events/PAAQ/WEAK53.txt" type="application/xml" />
 </entry>
 <entry>
  <title>Off the coast of Chile</title><updated>2026-11-25T02:00:00Z</updated>
  <geo:lat>-30.0</geo:lat><geo:long>-72.0</geo:long>
  <summary type="xhtml"><div xmlns="http://www.w3.org/1999/xhtml"><strong>Category:</strong> Warning<br/><strong>Preliminary Magnitude: </strong>8.3(Mwp)<br/><strong>Lat/Lon: </strong>-30 / -72</div></summary>
  <id>urn:uuid:chile</id>
 </entry>
</feed>"#;

    #[test]
    fn a_tsunami_message_says_its_category_and_what_the_centre_makes_of_it() {
        let messages = parse_tsunami_messages(TSUNAMI).unwrap();
        assert_eq!(messages.len(), 2);
        let palmer = &messages[0];
        assert_eq!(
            palmer.name,
            "Tsunami information: 40 miles SW of Palmer, Alaska"
        );
        assert_eq!((palmer.latitude_deg, palmer.longitude_deg), (61.3, -150.1));
        assert_eq!(palmer.class, tsunami_class::INFORMATION);
        assert_eq!(
            palmer.time,
            Some(JulianDate::from_utc(2026, 11, 24, 16, 43, 10.0))
        );
        assert_eq!(
            rows(palmer),
            [
                ("Category", "Information"),
                ("Earthquake near", "40 miles SW of Palmer, Alaska"),
                ("Magnitude", "4.1(Mwp)"),
                ("Issued", "2026-11-24 16:43 UTC"),
                ("Bulletin", "https://www.tsunami.gov/events/PAAQ/WEAK53.txt"),
            ]
        );
        assert_eq!(
            palmer.details,
            ["There is NO tsunami danger from this earthquake."]
        );
        // A warning is told apart from a statement that there is no danger.
        assert_eq!(messages[1].class, tsunami_class::WARNING);
        assert_eq!(messages[1].name, "Tsunami warning: Off the coast of Chile");
        assert_eq!(rows(&messages[1])[2], ("Magnitude", "8.3(Mwp)"));
    }

    const VOLCANOES_JSON: &str = r#"{"type":"FeatureCollection","features":[
      {"type":"Feature","geometry":{"type":"Point","coordinates":[-176.111,52.076]},
       "properties":{"volcanoName":"Great Sitkin","vnum":"311120","region":"Aleutians",
        "alertLevel":"WATCH","colorCode":"ORANGE","alertDate":"2026-11-20 20:33:25",
        "noticeUrl":"https://volcanoes.usgs.gov/hans-public/notice/X",
        "noticeSynopsis":"Lava continues to erupt slowly in the summit crater."}},
      {"type":"Feature","geometry":{"type":"Point","coordinates":[-121.76,46.853]},
       "properties":{"volcanoName":"Mount Rainier","vnum":"321030","region":"Cascades",
        "alertLevel":"NORMAL","colorCode":"GREEN","noticeUrl":null,"noticeSynopsis":null}},
      {"type":"Feature","geometry":{"type":"Point","coordinates":[-176.5852,51.9905]},
       "properties":{"volcanoName":"Adagdak","vnum":"311800","alertLevel":"UNASSIGNED","colorCode":"UNASSIGNED"}}
    ]}"#;

    #[test]
    fn a_volcano_shows_its_alert_level_and_one_with_none_is_left_out() {
        let volcanoes = parse_volcanoes(VOLCANOES_JSON).unwrap();
        assert_eq!(volcanoes.len(), 2);
        let sitkin = &volcanoes[0];
        assert_eq!(sitkin.name, "Great Sitkin: volcano watch");
        assert_eq!(
            (sitkin.latitude_deg, sitkin.longitude_deg),
            (52.076, -176.111)
        );
        assert_eq!(sitkin.class, volcano_class::WATCH);
        assert_eq!(
            rows(sitkin),
            [
                ("Alert level", "Watch"),
                ("Aviation colour", "orange"),
                ("Region", "Aleutians"),
                ("Since", "2026-11-20 20:33:25"),
                ("Notice", "https://volcanoes.usgs.gov/hans-public/notice/X"),
            ]
        );
        assert_eq!(
            sitkin.details,
            ["Lava continues to erupt slowly in the summit crater."]
        );
        assert_eq!(volcanoes[1].name, "Mount Rainier");
        assert_eq!(volcanoes[1].class, volcano_class::NORMAL);
    }

    #[test]
    fn answers_that_are_not_these_lists_are_errors() {
        for wrong in ["", "<html><body>Busy</body></html>", "{\"features\": []}"] {
            assert!(parse_disasters(wrong).is_err(), "{wrong}");
            assert!(parse_tsunami_messages(wrong).is_err(), "{wrong}");
        }
        assert!(parse_volcanoes("<html>").is_err());
        assert!(parse_volcanoes("{\"message\": \"busy\"}").is_err());
        // A quiet moment is not an error.
        let quiet = "<rss><channel><title>GDACS</title></channel></rss>";
        assert_eq!(parse_disasters(quiet).unwrap(), Vec::new());
        let calm = "<feed xmlns=\"http://www.w3.org/2005/Atom\"><title>None</title></feed>";
        assert_eq!(parse_tsunami_messages(calm).unwrap(), Vec::new());
        assert_eq!(parse_volcanoes("{\"features\": []}").unwrap(), Vec::new());
    }
}
