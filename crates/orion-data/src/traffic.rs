//! Live traffic: aircraft in flight, as reported by adsb.lol, a network of
//! volunteers' receivers that publishes what it hears as open data.

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

#[cfg(test)]
mod tests {
    use super::*;

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
