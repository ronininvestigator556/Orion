//! Live traffic: aircraft in flight, as reported by adsb.lol, a network of volunteers'
//! receivers that publishes what it hears as open data, and ships in the northern
//! Baltic, as heard by Finland's coastal receivers and published by Fintraffic.

use orion_core::feature::stable_key;
use serde_json::Value;

use crate::listing::ParseError;

/// Who publishes the aircraft positions, as shown to the user.
pub const AIRCRAFT_SOURCE: &str = "adsb.lol";
/// The farthest from a point, in nautical miles, that the service will look.
pub const MAX_RADIUS_NM: u32 = 250;
const ENDPOINT: &str = "https://api.adsb.lol/v2/point";

/// The address listing aircraft within `radius_nm` of a place. The place is given to
/// the nearest whole degree, which is all the service needs to be told about where
/// the user is looking.
pub fn aircraft_url(latitude_deg: f64, longitude_deg: f64, radius_nm: u32) -> String {
    let (latitude, longitude) = snap(latitude_deg, longitude_deg);
    format!(
        "{ENDPOINT}/{latitude}/{longitude}/{}",
        radius_nm.min(MAX_RADIUS_NM)
    )
}

/// A place to the nearest whole degree, as the service is asked about it.
pub fn snap(latitude_deg: f64, longitude_deg: f64) -> (i32, i32) {
    let latitude = latitude_deg.round().clamp(-90.0, 90.0) as i32;
    // 180 and -180 are the same meridian: always write it one way.
    let longitude = (longitude_deg.round() as i32 + 180).rem_euclid(360) - 180;
    (latitude, longitude)
}

/// What kind of aircraft a transponder says it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AircraftKind {
    /// Airliners and other large aircraft.
    Large,
    Light,
    Helicopter,
    Other,
}

/// One aircraft as last heard.
#[derive(Clone, Debug, PartialEq)]
pub struct Aircraft {
    /// Identifies the aircraft for as long as it is heard, from its transponder's address.
    pub key: u64,
    /// The transponder's address, six hexadecimal digits.
    pub address: String,
    /// The flight's callsign; empty if it is not sending one.
    pub callsign: String,
    pub registration: String,
    /// The aircraft type as the aviation authorities abbreviate it, such as `A333`.
    pub type_code: String,
    pub kind: AircraftKind,
    pub latitude_deg: f64,
    pub longitude_deg: f64,
    pub on_ground: bool,
    /// Height above sea level in feet, by air pressure; `None` on the ground or if not sent.
    pub altitude_ft: Option<f64>,
    /// Speed over the ground in knots.
    pub speed_kt: Option<f64>,
    /// Direction of travel over the ground, in degrees clockwise from north.
    pub track_deg: Option<f64>,
    /// Rate of climb in feet per minute; negative when descending.
    pub climb_fpm: Option<f64>,
    pub squawk: String,
    /// The emergency it is declaring, if any, in the transponder's words.
    pub emergency: Option<String>,
    /// How long before the answer was made the position was heard, in seconds.
    pub position_age_s: f64,
}

/// Reads the service's list of aircraft. Those without a position are left out.
pub fn parse_aircraft(text: &str) -> Result<Vec<Aircraft>, ParseError> {
    let document: Value =
        serde_json::from_str(text).map_err(|error| ParseError(error.to_string()))?;
    let listed = document["ac"]
        .as_array()
        .ok_or_else(|| ParseError("no list of aircraft".to_owned()))?;
    Ok(listed
        .iter()
        .filter_map(|entry| {
            let text_of = |key: &str| entry[key].as_str().unwrap_or("").trim().to_owned();
            let address = text_of("hex");
            let latitude_deg = entry["lat"].as_f64()?;
            let longitude_deg = entry["lon"].as_f64()?;
            if address.is_empty()
                || !(-90.0..=90.0).contains(&latitude_deg)
                || !(-180.0..=180.0).contains(&longitude_deg)
            {
                return None;
            }
            // The height is a number of feet, or the word "ground".
            let on_ground = entry["alt_baro"].as_str() == Some("ground");
            let category = text_of("category");
            let kind = match category.as_str() {
                "A3" | "A4" | "A5" | "A6" => AircraftKind::Large,
                "A1" | "A2" => AircraftKind::Light,
                "A7" => AircraftKind::Helicopter,
                _ => AircraftKind::Other,
            };
            let emergency = Some(text_of("emergency"))
                .filter(|declared| !declared.is_empty() && declared != "none");
            Some(Aircraft {
                key: stable_key(&address),
                callsign: text_of("flight"),
                registration: text_of("r"),
                type_code: text_of("t"),
                kind,
                latitude_deg,
                longitude_deg,
                on_ground,
                altitude_ft: entry["alt_baro"].as_f64(),
                speed_kt: entry["gs"].as_f64(),
                track_deg: entry["track"]
                    .as_f64()
                    .or_else(|| entry["true_heading"].as_f64()),
                climb_fpm: entry["baro_rate"]
                    .as_f64()
                    .or_else(|| entry["geom_rate"].as_f64()),
                squawk: text_of("squawk"),
                emergency,
                position_age_s: entry["seen_pos"].as_f64().unwrap_or(0.0).max(0.0),
                address,
            })
        })
        .collect())
}

/// Who publishes the ship positions, as shown to the user.
pub const SHIP_SOURCE: &str = "Fintraffic";
/// Where each ship last reported itself. Open data under CC BY 4.0, no key needed.
pub const SHIP_POSITIONS_URL: &str = "https://meri.digitraffic.fi/api/ais/v1/locations";
/// What each ship says about itself: name, type, destination.
pub const SHIP_DETAILS_URL: &str = "https://meri.digitraffic.fi/api/ais/v1/vessels";
/// A position older than this, in seconds, says too little about where a ship is now.
const SHIP_POSITION_MAX_AGE_S: f64 = 30.0 * 60.0;

/// What kind of ship a transponder says it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ShipKind {
    Cargo,
    Tanker,
    Passenger,
    /// Tugs, pilot boats, rescue and other working craft.
    Service,
    /// Fishing boats, yachts and pleasure craft.
    FishingOrLeisure,
    Other,
}

impl ShipKind {
    /// Reads the type number ships broadcast, where the tens digit gives the kind.
    fn from_code(code: u64) -> Self {
        match code {
            70..=79 => Self::Cargo,
            80..=89 => Self::Tanker,
            60..=69 => Self::Passenger,
            31..=35 | 50..=59 => Self::Service,
            30 | 36 | 37 => Self::FishingOrLeisure,
            _ => Self::Other,
        }
    }
}

/// One ship as last heard.
#[derive(Clone, Debug, PartialEq)]
pub struct Ship {
    /// Identifies the ship for as long as it is heard, from its radio identity.
    pub key: u64,
    /// Its maritime radio identity, nine digits.
    pub mmsi: u64,
    /// Empty if the ship's details have not been heard.
    pub name: String,
    pub call_sign: String,
    pub kind: ShipKind,
    /// The number its builders registered it under, where it has one.
    pub imo: Option<u64>,
    /// Where it says it is going, as its crew typed it.
    pub destination: String,
    pub draught_m: Option<f64>,
    pub latitude_deg: f64,
    pub longitude_deg: f64,
    /// Speed over the ground in knots.
    pub speed_kt: Option<f64>,
    /// Direction of travel over the ground, in degrees clockwise from north.
    pub course_deg: Option<f64>,
    /// What it reports itself as doing, in words.
    pub status: &'static str,
    /// True if it reports itself moored, at anchor or aground.
    pub at_rest: bool,
    /// How long before the answer was made the position was heard, in seconds.
    pub position_age_s: f64,
}

/// Reads Fintraffic's list of ship positions, joined to its list of ships' details.
/// Positions more than half an hour old are left out.
pub fn parse_ships(positions: &str, details: &str) -> Result<Vec<Ship>, ParseError> {
    let read = |text: &str| -> Result<Value, ParseError> {
        serde_json::from_str(text).map_err(|error| ParseError(error.to_string()))
    };
    let positions = read(positions)?;
    let listed = positions["features"]
        .as_array()
        .ok_or_else(|| ParseError("no list of ship positions".to_owned()))?;
    let details = read(details)?;
    let described: std::collections::HashMap<u64, &Value> = details
        .as_array()
        .ok_or_else(|| ParseError("no list of ships".to_owned()))?
        .iter()
        .filter_map(|ship| Some((ship["mmsi"].as_u64()?, ship)))
        .collect();
    // The answer is as old as its newest position; each position's age is measured from that.
    let newest = listed
        .iter()
        .filter_map(|entry| entry["properties"]["timestampExternal"].as_f64())
        .fold(0.0, f64::max);

    Ok(listed
        .iter()
        .filter_map(|entry| {
            let properties = &entry["properties"];
            let mmsi = entry["mmsi"]
                .as_u64()
                .or_else(|| properties["mmsi"].as_u64())?;
            let coordinates = &entry["geometry"]["coordinates"];
            let (longitude_deg, latitude_deg) =
                (coordinates[0].as_f64()?, coordinates[1].as_f64()?);
            if !(-90.0..=90.0).contains(&latitude_deg) || !(-180.0..=180.0).contains(&longitude_deg)
            {
                return None;
            }
            let heard = properties["timestampExternal"].as_f64()?;
            let position_age_s = ((newest - heard) / 1000.0).max(0.0);
            if position_age_s > SHIP_POSITION_MAX_AGE_S {
                return None;
            }
            let (status, at_rest) = match properties["navStat"].as_u64() {
                Some(0) => ("Under way using its engine", false),
                Some(1) => ("At anchor", true),
                Some(2) => ("Not under command", false),
                Some(3) => ("Restricted in its ability to manoeuvre", false),
                Some(4) => ("Constrained by its draught", false),
                Some(5) => ("Moored", true),
                Some(6) => ("Aground", true),
                Some(7) => ("Fishing", false),
                Some(8) => ("Under way sailing", false),
                _ => ("", false),
            };
            let about = described.get(&mmsi);
            let text_of = |key: &str| {
                about
                    .and_then(|ship| ship[key].as_str())
                    .unwrap_or("")
                    .trim()
                    .to_owned()
            };
            // Speeds and courses are sent as "not available" numbers when unknown.
            let speed_kt = properties["sog"].as_f64().filter(|speed| *speed < 102.0);
            let course_deg = properties["cog"].as_f64().filter(|course| *course < 360.0);
            Some(Ship {
                key: stable_key(&mmsi.to_string()),
                mmsi,
                name: text_of("name"),
                call_sign: text_of("callSign"),
                kind: ShipKind::from_code(
                    about
                        .and_then(|ship| ship["shipType"].as_u64())
                        .unwrap_or(0),
                ),
                imo: about
                    .and_then(|ship| ship["imo"].as_u64())
                    .filter(|imo| *imo > 0),
                destination: text_of("destination"),
                // Draught is sent in tenths of a metre.
                draught_m: about
                    .and_then(|ship| ship["draught"].as_f64())
                    .filter(|draught| *draught > 0.0)
                    .map(|draught| draught / 10.0),
                latitude_deg,
                longitude_deg,
                speed_kt,
                course_deg,
                status,
                at_rest,
                position_age_s,
            })
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHIP_POSITIONS: &str = r#"{"type":"FeatureCollection","dataUpdatedTime":"2026-11-25T11:21:25Z","features":[
        {"mmsi":255802540,"type":"Feature","geometry":{"type":"Point","coordinates":[24.95,60.10]},
         "properties":{"mmsi":255802540,"sog":12.4,"cog":195.0,"navStat":0,"heading":195,"timestampExternal":1795605524000}},
        {"mmsi":230000001,"type":"Feature","geometry":{"type":"Point","coordinates":[21.2148,62.3527]},
         "properties":{"mmsi":230000001,"sog":0.0,"cog":360.0,"navStat":5,"heading":511,"timestampExternal":1795605464000}},
        {"mmsi":230000002,"type":"Feature","geometry":{"type":"Point","coordinates":[22.0,60.0]},
         "properties":{"mmsi":230000002,"sog":102.3,"cog":10.0,"navStat":15,"timestampExternal":1795605500000}},
        {"mmsi":230000003,"type":"Feature","geometry":{"type":"Point","coordinates":[23.0,60.0]},
         "properties":{"mmsi":230000003,"sog":5.0,"cog":10.0,"navStat":0,"timestampExternal":1795500000000}}]}"#;
    const SHIP_DETAILS: &str = r#"[
        {"callSign":"CQPB","destination":"NLRTM","draught":59,"imo":9350783,"mmsi":255802540,"name":"ALAND","shipType":70},
        {"callSign":"OJAA","destination":"","draught":0,"imo":0,"mmsi":230000001,"name":"HARBOUR TUG","shipType":52}]"#;

    #[test]
    fn ships_are_read_with_their_details_joined_on() {
        let ships = parse_ships(SHIP_POSITIONS, SHIP_DETAILS).unwrap();
        // The one last heard a day ago is left out.
        assert_eq!(ships.len(), 3);

        let cargo = &ships[0];
        assert_eq!(cargo.name, "ALAND");
        assert_eq!(cargo.kind, ShipKind::Cargo);
        assert_eq!(cargo.imo, Some(9_350_783));
        assert_eq!(cargo.destination, "NLRTM");
        assert_eq!(cargo.draught_m, Some(5.9));
        assert_eq!((cargo.latitude_deg, cargo.longitude_deg), (60.10, 24.95));
        assert_eq!(cargo.speed_kt, Some(12.4));
        assert_eq!(cargo.course_deg, Some(195.0));
        assert_eq!(cargo.status, "Under way using its engine");
        assert!(!cargo.at_rest);
        assert_eq!(cargo.position_age_s, 0.0);

        let tug = &ships[1];
        assert_eq!(tug.kind, ShipKind::Service);
        assert!(tug.at_rest);
        assert_eq!(tug.status, "Moored");
        // A course of 360 means "not known", as does a draught or number of nothing.
        assert_eq!(tug.course_deg, None);
        assert_eq!(tug.draught_m, None);
        assert_eq!(tug.imo, None);
        assert_eq!(tug.position_age_s, 60.0);

        // One whose details have not been heard is still shown, without a name.
        let unnamed = &ships[2];
        assert_eq!(unnamed.name, "");
        assert_eq!(unnamed.kind, ShipKind::Other);
        assert_eq!(unnamed.speed_kt, None);
        assert_ne!(unnamed.key, cargo.key);
    }

    #[test]
    fn answers_that_are_not_ship_lists_are_errors() {
        assert!(parse_ships("<html></html>", SHIP_DETAILS).is_err());
        assert!(parse_ships(SHIP_POSITIONS, "{}").is_err());
        assert!(parse_ships("{}", "[]").is_err());
        assert_eq!(parse_ships(r#"{"features":[]}"#, "[]").unwrap(), []);
    }

    #[test]
    fn ship_kinds_follow_the_broadcast_type_number() {
        assert_eq!(ShipKind::from_code(70), ShipKind::Cargo);
        assert_eq!(ShipKind::from_code(89), ShipKind::Tanker);
        assert_eq!(ShipKind::from_code(60), ShipKind::Passenger);
        assert_eq!(ShipKind::from_code(52), ShipKind::Service);
        assert_eq!(ShipKind::from_code(30), ShipKind::FishingOrLeisure);
        assert_eq!(ShipKind::from_code(0), ShipKind::Other);
    }

    const ANSWER: &str = r#"{"ac":[
        {"hex":"4caa41","type":"adsb_icao","flight":"EIN142  ","r":"EI-EIN","t":"A333","alt_baro":12825,
         "gs":307.1,"track":78.54,"baro_rate":-1024,"squawk":"3607","emergency":"none","category":"A5",
         "lat":53.131714,"lon":-6.697109,"seen_pos":0.083},
        {"hex":"406abc","flight":"GABCD   ","r":"G-ABCD","t":"C172","alt_baro":"ground","gs":4.0,
         "true_heading":120.0,"category":"A1","lat":51.47,"lon":-0.46,"seen_pos":3.5},
        {"hex":"43c001","alt_baro":1500,"gs":90,"track":10,"category":"A7","emergency":"general",
         "lat":51.0,"lon":0.1},
        {"hex":"nopos1","flight":"LOST1","alt_baro":30000},
        {"hex":"","lat":1.0,"lon":1.0}
      ],"msg":"No error","now":1795604311500,"total":5}"#;

    #[test]
    fn aircraft_are_read_with_what_they_report() {
        let aircraft = parse_aircraft(ANSWER).unwrap();
        // The one with no position and the one with no address are left out.
        assert_eq!(aircraft.len(), 3);

        let airliner = &aircraft[0];
        assert_eq!(airliner.callsign, "EIN142");
        assert_eq!(airliner.registration, "EI-EIN");
        assert_eq!(airliner.type_code, "A333");
        assert_eq!(airliner.kind, AircraftKind::Large);
        assert_eq!(airliner.altitude_ft, Some(12_825.0));
        assert_eq!(airliner.speed_kt, Some(307.1));
        assert_eq!(airliner.track_deg, Some(78.54));
        assert_eq!(airliner.climb_fpm, Some(-1024.0));
        assert!(!airliner.on_ground);
        assert_eq!(airliner.emergency, None);

        let parked = &aircraft[1];
        assert!(parked.on_ground);
        assert_eq!(parked.altitude_ft, None);
        assert_eq!(parked.kind, AircraftKind::Light);
        // With no track sent, the way it is pointing stands in.
        assert_eq!(parked.track_deg, Some(120.0));
        assert_eq!(parked.position_age_s, 3.5);

        let helicopter = &aircraft[2];
        assert_eq!(helicopter.kind, AircraftKind::Helicopter);
        assert_eq!(helicopter.callsign, "");
        assert_eq!(helicopter.emergency.as_deref(), Some("general"));
        assert_ne!(helicopter.key, airliner.key);
    }

    #[test]
    fn an_answer_without_a_list_is_an_error_and_an_empty_list_is_not() {
        assert!(parse_aircraft("<html>rate limited</html>").is_err());
        assert!(parse_aircraft(r#"{"msg":"busy"}"#).is_err());
        assert_eq!(parse_aircraft(r#"{"ac":[]}"#).unwrap(), []);
    }

    #[test]
    fn the_service_is_told_where_to_look_only_to_the_nearest_degree() {
        assert_eq!(
            aircraft_url(51.4706, -0.461941, 250),
            "https://api.adsb.lol/v2/point/51/0/250"
        );
        assert_eq!(
            aircraft_url(-33.94, 151.18, 900),
            "https://api.adsb.lol/v2/point/-34/151/250"
        );
        assert_eq!(snap(10.2, 179.6), (10, -180));
        assert_eq!(snap(10.2, -179.6), (10, -180));
        assert_eq!(snap(89.7, 12.4), (90, 12));
    }
}
