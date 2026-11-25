//! Road Incidents: accidents, closures and roadworks as each road's authority
//! publishes them. Each country publishes in its own way, so each has its own reader:
//!
//! - France: Bison Futé's list for the state's own roads, in the European DATEX II
//!   format. Most toll motorways are run by companies that publish nothing here.
//! - Finland: Fintraffic's traffic messages, as GeoJSON.
//! - United States: the work-zone lists of the states that publish one without a key,
//!   gathered into one file by a job in the repository (`tools/make_road_works.py`).
//!   No state publishes its accidents this way.

use std::time::Duration;

use orion_core::feature::Feature;
use orion_core::time::JulianDate;
use serde_json::Value;

use crate::hazards::format_moment;
use crate::listing::{Listing, ParseError};

/// Classes of Road Incident, the same in every country.
pub mod incident_class {
    pub const ACCIDENT: u8 = 0;
    pub const CLOSURE: u8 = 1;
    pub const ROADWORKS: u8 = 2;
    pub const OTHER: u8 = 3;
}

/// Classes of U.S. work zone: these are never accidents, so they have two of their own.
pub mod work_zone_class {
    pub const ROAD_CLOSED: u8 = 0;
    pub const ROADWORKS: u8 = 1;
}

/// What is happening now on the roads the French state runs. Licence Ouverte 2.0.
pub const FRANCE: Listing = Listing {
    source: "Bison Futé, the French state's road information service",
    url: "https://tipi.bison-fute.gouv.fr/bison-fute-ouvert/publicationsDIR/Evenementiel-DIR/grt/RRN/content.xml",
    accept: None,
    cache_name: "roads_france.xml",
    parse: parse_france,
    refresh_every: Duration::from_secs(10 * 60),
};

/// Finland's traffic announcements and roadworks in force now. CC BY 4.0.
pub const FINLAND: Listing = Listing {
    source: "Fintraffic",
    url: "https://tie.digitraffic.fi/api/traffic-message/v1/messages?inactiveHours=0\
          &includeAreaGeometry=false&situationType=TRAFFIC_ANNOUNCEMENT&situationType=ROAD_WORK",
    accept: None,
    cache_name: "roads_finland.geojson",
    parse: parse_finland,
    // Every roadwork comes with the whole line of its road, so the list is several
    // megabytes: often enough is not very often.
    refresh_every: Duration::from_secs(30 * 60),
};

/// Work zones under way in the U.S. states that publish theirs without a key.
pub const UNITED_STATES: Listing = Listing {
    source: "U.S. state transport departments",
    url: concat!(
        env!("CARGO_PKG_REPOSITORY"),
        "/releases/download/extracts/road_works_us.tsv"
    ),
    accept: None,
    cache_name: "road_works_us.tsv",
    parse: parse_united_states,
    refresh_every: Duration::from_secs(60 * 60),
};

type Node<'a, 'input> = roxmltree::Node<'a, 'input>;

/// The first element inside `node`, however deep, with the given name. Names are
/// matched without their prefix, which differs from publisher to publisher.
fn inside<'a, 'input>(node: Node<'a, 'input>, name: &str) -> Option<Node<'a, 'input>> {
    node.descendants()
        .find(|inner| inner.is_element() && inner.tag_name().name() == name)
}

fn text_inside<'a>(node: Node<'a, '_>, name: &str) -> Option<&'a str> {
    inside(node, name)
        .and_then(|found| found.text())
        .map(str::trim)
        .filter(|text| !text.is_empty())
}

/// What kind of record a DATEX II record is, in plain words, and its class.
fn french_kind(kind: &str) -> (String, u8) {
    use incident_class::{ACCIDENT, CLOSURE, OTHER, ROADWORKS};
    let (label, class) = match kind {
        "Accident" => ("Accident", ACCIDENT),
        "MaintenanceWorks" | "ConstructionWorks" => ("Roadworks", ROADWORKS),
        "RoadOrCarriagewayOrLaneManagement" => ("Lane or carriageway restriction", CLOSURE),
        "ReroutingManagement" => ("Diversion", CLOSURE),
        "GeneralNetworkManagement" => ("Traffic management", CLOSURE),
        "VehicleObstruction" => ("Vehicle in the way", OTHER),
        "GeneralObstruction" | "EnvironmentalObstruction" | "AnimalPresenceObstruction" => {
            ("Obstruction", OTHER)
        }
        "WeatherRelatedRoadConditions" | "NonWeatherRelatedRoadConditions" => {
            ("Road conditions", OTHER)
        }
        "AbnormalTraffic" => ("Slow or queuing traffic", OTHER),
        "SpeedManagement" => ("Speed limit", OTHER),
        other => {
            // A kind not met before is shown under its own name, with its words apart.
            let mut spaced = String::new();
            for letter in other.chars() {
                if letter.is_uppercase() && !spaced.is_empty() {
                    spaced.push(' ');
                    spaced.extend(letter.to_lowercase());
                } else {
                    spaced.push(letter);
                }
            }
            return (spaced, OTHER);
        }
    };
    (label.to_owned(), class)
}

/// A French road's number as people write it: `A0028` is the A28.
fn french_road(number: &str) -> String {
    let letters: String = number.chars().take_while(|c| c.is_alphabetic()).collect();
    let digits = number[letters.len()..].trim_start_matches('0');
    format!("{letters}{digits}")
}

/// Reads Bison Futé's list, which is in the European DATEX II format: a list of
/// situations, each with one or more records.
pub fn parse_france(text: &str) -> Result<Vec<Feature>, ParseError> {
    let document = roxmltree::Document::parse(text)
        .map_err(|error| ParseError(format!("this is not a list of road incidents: {error}")))?;
    if inside(document.root(), "d2LogicalModel").is_none() {
        return Err(ParseError(
            "this is not a list of road incidents".to_owned(),
        ));
    }
    Ok(document
        .descendants()
        .filter(|node| node.is_element() && node.tag_name().name() == "situationRecord")
        .filter_map(french_record)
        .collect())
}

fn french_record(record: Node<'_, '_>) -> Option<Feature> {
    let id = record.attribute("id")?;
    let kind = record
        .attributes()
        .find(|attribute| attribute.name() == "type")
        .map(|attribute| attribute.value().rsplit(':').next().unwrap_or_default())?;
    let place = inside(record, "pointCoordinates")?;
    let latitude: f64 = text_inside(place, "latitude")?.parse().ok()?;
    let longitude: f64 = text_inside(place, "longitude")?.parse().ok()?;

    // The names given to the place: its town, and the road it is on.
    let named = |wanted: &str| {
        record
            .descendants()
            .filter(|node| node.is_element() && node.tag_name().name() == "name")
            .find(|name| text_inside(*name, "tpegOtherPointDescriptorType") == Some(wanted))
            .and_then(|name| text_inside(name, "value"))
    };
    let road = text_inside(record, "roadNumber")
        .map(french_road)
        .or_else(|| named("linkName").map(str::to_owned));
    let town = named("townName");

    let (label, class) = french_kind(kind);
    let title = match (&road, town) {
        (Some(road), Some(town)) => format!("{label} on the {road} near {town}"),
        (Some(road), None) => format!("{label} on the {road}"),
        (None, Some(town)) => format!("{label} near {town}"),
        (None, None) => label.clone(),
    };
    let mut incident = Feature::at(id, title, latitude, longitude);
    incident.class = class;
    incident.time = text_inside(record, "overallStartTime").and_then(JulianDate::parse_rfc3339);
    incident.until = text_inside(record, "overallEndTime").and_then(JulianDate::parse_rfc3339);

    incident.row("Kind", label);
    incident.row("Road", road.unwrap_or_default());
    incident.row("Near", town.unwrap_or_default());
    if let Some(since) = incident.time {
        incident.row("Since", format_moment(since));
    }
    if let Some(until) = incident.until {
        incident.row("Until", format_moment(until));
    }
    if let (Some(closed), Some(lanes)) = (
        text_inside(record, "numberOfLanesRestricted"),
        text_inside(record, "originalNumberOfLanes"),
    ) {
        incident.row("Lanes", format!("{closed} of {lanes} restricted"));
    }
    let comments: Vec<&str> = record
        .descendants()
        .filter(|node| node.is_element() && node.tag_name().name() == "generalPublicComment")
        .filter_map(|comment| text_inside(comment, "value"))
        .collect();
    incident.row("As published", comments.join("; "));
    incident.row(
        "Published by",
        text_inside(record, "sourceIdentification").unwrap_or_default(),
    );
    incident.is_on_earth().then_some(incident)
}

/// Reads Fintraffic's traffic messages. Each is published in Finnish.
pub fn parse_finland(text: &str) -> Result<Vec<Feature>, ParseError> {
    let not_messages = || ParseError("this is not a list of traffic messages".to_owned());
    let document: Value = serde_json::from_str(text).map_err(|_| not_messages())?;
    let messages = document["features"].as_array().ok_or_else(not_messages)?;
    Ok(messages.iter().filter_map(finnish_message).collect())
}

/// A longitude and a latitude, in degrees, in the order GeoJSON gives them.
type Place = [f64; 2];
type Line = Vec<Place>;

/// The lines of a GeoJSON geometry, each a run of longitude and latitude, and the
/// place to mark it by: its own point, or where its first line begins.
fn lines_of(geometry: &Value) -> Option<(Place, Vec<Line>)> {
    let point = |value: &Value| Some([value.get(0)?.as_f64()?, value.get(1)?.as_f64()?]);
    let line =
        |value: &Value| -> Option<Vec<[f64; 2]>> { value.as_array()?.iter().map(point).collect() };
    let coordinates = &geometry["coordinates"];
    let lines: Vec<Vec<[f64; 2]>> = match geometry["type"].as_str()? {
        "Point" => return Some((point(coordinates)?, Vec::new())),
        "LineString" => vec![line(coordinates)?],
        "MultiLineString" => coordinates.as_array()?.iter().filter_map(line).collect(),
        _ => return None,
    };
    let first = *lines.first()?.first()?;
    Some((first, lines))
}

fn finnish_message(message: &Value) -> Option<Feature> {
    use incident_class::{ACCIDENT, CLOSURE, OTHER, ROADWORKS};
    let properties = &message["properties"];
    let id = properties["situationId"].as_str()?;
    let announcement = properties["announcements"].get(0)?;
    let ([longitude, latitude], lines) = lines_of(&message["geometry"])?;

    let what: Vec<&str> = announcement["features"]
        .as_array()
        .map(|features| {
            features
                .iter()
                .filter_map(|feature| feature["name"].as_str())
                .collect()
        })
        .unwrap_or_default();
    let announced = properties["trafficAnnouncementType"]
        .as_str()
        .unwrap_or_default();
    let (label, class) = if properties["situationType"].as_str() == Some("ROAD_WORK") {
        ("Roadworks", ROADWORKS)
    } else if announced.contains("ACCIDENT") {
        ("Accident", ACCIDENT)
    } else if what
        .iter()
        .any(|feature| feature.to_lowercase().contains("suljettu"))
    {
        // "Suljettu" is Finnish for closed.
        ("Closure", CLOSURE)
    } else {
        ("Traffic announcement", OTHER)
    };

    let title = announcement["title"].as_str().unwrap_or(label).trim();
    let mut incident = Feature::at(id, title.trim_end_matches('.'), latitude, longitude);
    incident.class = class;
    incident.shape = lines;
    let times = &announcement["timeAndDuration"];
    incident.time = times["startTime"]
        .as_str()
        .and_then(JulianDate::parse_rfc3339);
    incident.until = times["endTime"]
        .as_str()
        .and_then(JulianDate::parse_rfc3339);

    incident.row("Kind", label);
    let location = announcement["location"]["description"]
        .as_str()
        .unwrap_or_default();
    incident.row("Where", location.lines().next().unwrap_or_default());
    if let Some(since) = incident.time {
        incident.row("Since", format_moment(since));
    }
    if let Some(until) = incident.until {
        incident.row("Until", format_moment(until));
    }
    incident.row("What", what.join("; "));
    let comment = announcement["comment"].as_str().unwrap_or_default();
    incident.row(
        "As published",
        comment.split_whitespace().collect::<Vec<_>>().join(" "),
    );
    incident.is_on_earth().then_some(incident)
}

const US_HEADING: &str = "#orion-road-works";
const US_VERSION: &str = "1";

/// Reads the file of U.S. work zones the job publishes: a heading naming the format,
/// its version, when it was built and how many follow, then a line for each.
pub fn parse_united_states(text: &str) -> Result<Vec<Feature>, ParseError> {
    let mut lines = text.lines();
    let heading: Vec<&str> = lines.next().unwrap_or_default().split('\t').collect();
    if heading.len() != 4 || heading[0] != US_HEADING {
        return Err(ParseError(
            "this is not a file of U.S. work zones".to_owned(),
        ));
    }
    if heading[1] != US_VERSION {
        return Err(ParseError(format!(
            "this file of U.S. work zones is version {}, and this version of Orion reads version {US_VERSION}",
            heading[1]
        )));
    }
    Ok(lines.filter_map(american_work_zone).collect())
}

/// One line: identifier, latitude, longitude, kind, state, road, direction, what
/// the state says of it, when it began and ends, and who published it.
fn american_work_zone(line: &str) -> Option<Feature> {
    let fields: Vec<&str> = line.split('\t').collect();
    let [
        id,
        latitude,
        longitude,
        kind,
        state,
        road,
        direction,
        description,
        start,
        end,
        publisher,
    ] = fields[..]
    else {
        return None;
    };
    let (label, class) = match kind {
        "closure" => ("Road closed", work_zone_class::ROAD_CLOSED),
        "roadworks" => ("Roadworks", work_zone_class::ROADWORKS),
        _ => return None,
    };
    let title = if road.is_empty() {
        format!("{label} in {state}")
    } else {
        format!("{label} on {road}")
    };
    let mut zone = Feature::at(id, title, latitude.parse().ok()?, longitude.parse().ok()?);
    zone.class = class;
    zone.time = JulianDate::parse_rfc3339(start);
    zone.until = JulianDate::parse_rfc3339(end);
    zone.row("Kind", label);
    zone.row("Road", road);
    zone.row("Direction", direction);
    zone.row("State", state);
    if let Some(since) = zone.time {
        zone.row("Since", format_moment(since));
    }
    if let Some(until) = zone.until {
        zone.row("Until", format_moment(until));
    }
    zone.row("As published", description);
    zone.row("Published by", publisher);
    zone.is_on_earth().then_some(zone)
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

    /// Two records as Bison Futé publishes them, cut down to what is read: an
    /// accident on the A28, and roadworks with an end and no town.
    const FRANCE_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<soap:Envelope xmlns:soap="http://www.w3.org/2003/05/soap-envelope"><soap:Body>
<d2LogicalModel xmlns:ns2="http://datex2.eu/schema/2/2_0" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" modelBaseVersion="2">
<ns2:payloadPublication xsi:type="ns2:SituationPublication" lang="fr">
<ns2:situation id="S1" version="3">
 <ns2:situationRecord xsi:type="ns2:Accident" id="S1-1" version="3">
  <ns2:source><ns2:sourceIdentification>DIR Nord-Ouest</ns2:sourceIdentification></ns2:source>
  <ns2:validity><ns2:validityTimeSpecification>
   <ns2:overallStartTime>2026-11-25T17:15:58.676+01:00</ns2:overallStartTime>
  </ns2:validityTimeSpecification></ns2:validity>
  <ns2:impact><ns2:numberOfLanesRestricted>1</ns2:numberOfLanesRestricted><ns2:originalNumberOfLanes>2</ns2:originalNumberOfLanes></ns2:impact>
  <ns2:generalPublicComment><ns2:comment><ns2:values><ns2:value lang="fr">situé 4888 m au sud de Saint-Saëns</ns2:value></ns2:values></ns2:comment></ns2:generalPublicComment>
  <ns2:generalPublicComment><ns2:comment><ns2:values><ns2:value lang="fr">De Rouen vers Abbeville</ns2:value></ns2:values></ns2:comment></ns2:generalPublicComment>
  <ns2:groupOfLocations xsi:type="ns2:Point">
   <ns2:tpegPointLocation xsi:type="ns2:TpegSimplePoint"><ns2:point xsi:type="ns2:TpegNonJunctionPoint">
    <ns2:pointCoordinates><ns2:latitude>49.612236</ns2:latitude><ns2:longitude>1.2656432</ns2:longitude></ns2:pointCoordinates>
    <ns2:name><ns2:descriptor><ns2:values><ns2:value lang="fr">Rocquemont</ns2:value></ns2:values></ns2:descriptor><ns2:tpegOtherPointDescriptorType>townName</ns2:tpegOtherPointDescriptorType></ns2:name>
    <ns2:name><ns2:descriptor><ns2:values><ns2:value lang="fr">A28</ns2:value></ns2:values></ns2:descriptor><ns2:tpegOtherPointDescriptorType>linkName</ns2:tpegOtherPointDescriptorType></ns2:name>
   </ns2:point></ns2:tpegPointLocation>
   <ns2:pointAlongLinearElement><ns2:linearElement><ns2:roadNumber>A0028</ns2:roadNumber></ns2:linearElement></ns2:pointAlongLinearElement>
  </ns2:groupOfLocations>
  <ns2:accidentType>accident</ns2:accidentType>
 </ns2:situationRecord>
</ns2:situation>
<ns2:situation id="S2" version="1">
 <ns2:situationRecord xsi:type="ns2:MaintenanceWorks" id="S2-1" version="1">
  <ns2:validity><ns2:validityTimeSpecification>
   <ns2:overallStartTime>2026-11-24T08:00:00+01:00</ns2:overallStartTime>
   <ns2:overallEndTime>2026-11-28T17:00:00+01:00</ns2:overallEndTime>
  </ns2:validityTimeSpecification></ns2:validity>
  <ns2:groupOfLocations xsi:type="ns2:Linear"><ns2:tpegLinearLocation>
   <ns2:from><ns2:pointCoordinates><ns2:latitude>42.844486</ns2:latitude><ns2:longitude>1.601671</ns2:longitude></ns2:pointCoordinates>
    <ns2:name><ns2:descriptor><ns2:values><ns2:value lang="fr">N20</ns2:value></ns2:values></ns2:descriptor><ns2:tpegOtherPointDescriptorType>linkName</ns2:tpegOtherPointDescriptorType></ns2:name></ns2:from>
   <ns2:to><ns2:pointCoordinates><ns2:latitude>42.9</ns2:latitude><ns2:longitude>1.61</ns2:longitude></ns2:pointCoordinates></ns2:to>
  </ns2:tpegLinearLocation></ns2:groupOfLocations>
 </ns2:situationRecord>
 <ns2:situationRecord xsi:type="ns2:SpeedManagement" id="S2-2" version="1">
  <ns2:validity/>
 </ns2:situationRecord>
</ns2:situation>
</ns2:payloadPublication></d2LogicalModel></soap:Body></soap:Envelope>"#;

    #[test]
    fn a_french_accident_says_which_road_and_near_where() {
        let incidents = parse_france(FRANCE_XML).unwrap();
        // The record with no place is passed over.
        assert_eq!(incidents.len(), 2);
        let accident = &incidents[0];
        assert_eq!(accident.name, "Accident on the A28 near Rocquemont");
        assert_eq!(
            (accident.latitude_deg, accident.longitude_deg),
            (49.612236, 1.2656432)
        );
        assert_eq!(accident.class, incident_class::ACCIDENT);
        // 17:15:58 in Paris in winter is 16:15:58 UTC.
        let began = JulianDate::from_utc(2026, 11, 25, 16, 15, 58.676);
        assert!((accident.time.unwrap().0 - began.0).abs() < 1e-8);
        assert_eq!(accident.until, None);
        assert_eq!(
            rows(accident),
            [
                ("Kind", "Accident"),
                ("Road", "A28"),
                ("Near", "Rocquemont"),
                ("Since", "2026-11-25 16:16 UTC"),
                ("Lanes", "1 of 2 restricted"),
                (
                    "As published",
                    "situé 4888 m au sud de Saint-Saëns; De Rouen vers Abbeville"
                ),
                ("Published by", "DIR Nord-Ouest"),
            ]
        );
    }

    #[test]
    fn french_roadworks_are_marked_where_they_start_and_know_when_they_end() {
        let incidents = parse_france(FRANCE_XML).unwrap();
        let works = &incidents[1];
        assert_eq!(works.name, "Roadworks on the N20");
        assert_eq!(works.class, incident_class::ROADWORKS);
        assert_eq!(
            (works.latitude_deg, works.longitude_deg),
            (42.844486, 1.601671)
        );
        // 17:00 in Paris in winter is 16:00 UTC.
        let ends = JulianDate::from_utc(2026, 11, 28, 16, 0, 0.0);
        assert!((works.until.unwrap().0 - ends.0).abs() < 1e-8);
    }

    #[test]
    fn french_kinds_are_put_in_plain_words() {
        assert_eq!(
            french_kind("RoadOrCarriagewayOrLaneManagement"),
            (
                "Lane or carriageway restriction".to_owned(),
                incident_class::CLOSURE
            )
        );
        assert_eq!(
            french_kind("ConstructionWorks").1,
            incident_class::ROADWORKS
        );
        assert_eq!(french_kind("VehicleObstruction").1, incident_class::OTHER);
        // A kind not met before keeps its own name.
        assert_eq!(
            french_kind("PublicEvent"),
            ("Public event".to_owned(), incident_class::OTHER)
        );
        assert_eq!(french_road("A0028"), "A28");
        assert_eq!(french_road("N0020"), "N20");
        assert_eq!(french_road("RD906"), "RD906");
    }

    const FINLAND_JSON: &str = r#"{"type":"FeatureCollection","features":[
      {"type":"Feature","geometry":{"type":"MultiLineString","coordinates":[[[23.9,60.25],[23.95,60.26]],[[24.0,60.27],[24.1,60.28]]]},
       "properties":{"situationId":"GUID50001","situationType":"TRAFFIC_ANNOUNCEMENT","trafficAnnouncementType":"GENERAL",
        "announcements":[{"language":"FI","title":"Tie 1, Lohja. Liikennetiedote. ",
          "location":{"description":"Tie 1 välillä Helsinki - Turku, Lohja.\nTarkempi paikka: tunneli"},
          "features":[{"name":"Liikenne ohjataan kaksisuuntaisena"}],
          "comment":"Tunnelin huoltotyö. \nAja varovasti.",
          "timeAndDuration":{"startTime":"2026-11-25T16:00:00Z","endTime":"2026-11-25T22:00:00Z"}}]}},
      {"type":"Feature","geometry":{"type":"Point","coordinates":[25.0,62.0]},
       "properties":{"situationId":"GUID50002","situationType":"TRAFFIC_ANNOUNCEMENT","trafficAnnouncementType":"ACCIDENT_REPORT",
        "announcements":[{"title":"Tie 4, Jyväskylä. Onnettomuus.","location":{"description":"Tie 4"},"features":[],
          "timeAndDuration":{"startTime":"2026-11-25T18:00:00Z"}}]}},
      {"type":"Feature","geometry":{"type":"LineString","coordinates":[[26.0,63.0],[26.1,63.1]]},
       "properties":{"situationId":"GUID50003","situationType":"ROAD_WORK",
        "announcements":[{"title":"Tie 5. Tietyö.","features":[{"name":"Tie suljettu"}],"timeAndDuration":{}}]}},
      {"type":"Feature","geometry":null,"properties":{"situationId":"GUID50004","announcements":[{"title":"Nowhere"}]}}
    ]}"#;

    #[test]
    fn a_finnish_message_is_marked_where_its_line_begins_and_keeps_its_line() {
        let messages = parse_finland(FINLAND_JSON).unwrap();
        // The message with no place is passed over.
        assert_eq!(messages.len(), 3);
        let tunnel = &messages[0];
        assert_eq!(tunnel.name, "Tie 1, Lohja. Liikennetiedote");
        assert_eq!((tunnel.latitude_deg, tunnel.longitude_deg), (60.25, 23.9));
        assert_eq!(tunnel.shape.len(), 2);
        assert_eq!(tunnel.class, incident_class::OTHER);
        assert_eq!(
            tunnel.until,
            Some(JulianDate::from_utc(2026, 11, 25, 22, 0, 0.0))
        );
        assert_eq!(
            rows(tunnel),
            [
                ("Kind", "Traffic announcement"),
                ("Where", "Tie 1 välillä Helsinki - Turku, Lohja."),
                ("Since", "2026-11-25 16:00 UTC"),
                ("Until", "2026-11-25 22:00 UTC"),
                ("What", "Liikenne ohjataan kaksisuuntaisena"),
                ("As published", "Tunnelin huoltotyö. Aja varovasti."),
            ]
        );
    }

    #[test]
    fn finnish_accidents_and_roadworks_are_told_apart() {
        let messages = parse_finland(FINLAND_JSON).unwrap();
        assert_eq!(messages[1].class, incident_class::ACCIDENT);
        assert_eq!(
            (messages[1].latitude_deg, messages[1].longitude_deg),
            (62.0, 25.0)
        );
        assert!(messages[1].shape.is_empty());
        // Roadworks stay roadworks even where the road is closed for them.
        assert_eq!(messages[2].class, incident_class::ROADWORKS);
        assert_eq!(rows(&messages[2])[0], ("Kind", "Roadworks"));
    }

    const US_FILE: &str = "#orion-road-works\t1\t2026-11-25T21:00:00Z\t2\n\
        wisconsin/d09c8972\t43.0312\t-87.9816\tclosure\tWisconsin\tI-94 EB\teastbound\t\
        Ramp Full Closure on I-94 EB at OFF RAMP TO HAWLEY RD.\t2025-11-05T03:00:00+00:00\t\
        2027-11-15T11:30:00+00:00\tWisconsin DOT\n\
        iowa/77\t41.6\t-93.6\troadworks\tIowa\t\t\tBridge painting\t2026-11-01T12:00:00Z\t\tIowa DOT\n";

    #[test]
    fn an_american_work_zone_says_whether_the_road_is_closed() {
        let zones = parse_united_states(US_FILE).unwrap();
        assert_eq!(zones.len(), 2);
        let ramp = &zones[0];
        assert_eq!(ramp.name, "Road closed on I-94 EB");
        assert_eq!(ramp.class, work_zone_class::ROAD_CLOSED);
        assert_eq!((ramp.latitude_deg, ramp.longitude_deg), (43.0312, -87.9816));
        assert_eq!(
            rows(ramp),
            [
                ("Kind", "Road closed"),
                ("Road", "I-94 EB"),
                ("Direction", "eastbound"),
                ("State", "Wisconsin"),
                ("Since", "2025-11-05 03:00 UTC"),
                ("Until", "2027-11-15 11:30 UTC"),
                (
                    "As published",
                    "Ramp Full Closure on I-94 EB at OFF RAMP TO HAWLEY RD."
                ),
                ("Published by", "Wisconsin DOT"),
            ]
        );
        // With no road named it is placed by its state, and it may have no end.
        assert_eq!(zones[1].name, "Roadworks in Iowa");
        assert_eq!(zones[1].class, work_zone_class::ROADWORKS);
        assert_eq!(zones[1].until, None);
    }

    #[test]
    fn answers_that_are_not_road_incidents_are_errors() {
        assert!(parse_france("").is_err());
        assert!(parse_france("<html><body>Maintenance</body></html>").is_err());
        assert!(parse_france("{\"features\": []}").is_err());
        assert!(parse_finland("<html>").is_err());
        assert!(parse_finland("{\"message\": \"busy\"}").is_err());
        assert!(parse_united_states("").is_err());
        assert!(parse_united_states("#orion-unrest\t1\t2026-11-25T19:05:00Z\t0\n").is_err());
        let newer = parse_united_states("#orion-road-works\t2\t2026-11-25T21:00:00Z\t0\n");
        assert!(newer.unwrap_err().0.contains("version 2"));
        // A quiet moment is not an error.
        assert_eq!(parse_finland("{\"features\": []}").unwrap(), Vec::new());
        let quiet = "#orion-road-works\t1\t2026-11-25T21:00:00Z\t0\n";
        assert_eq!(parse_united_states(quiet).unwrap(), Vec::new());
    }
}
