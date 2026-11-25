//! The listings behind the Hazards Layers: earthquakes, fire detections, tropical storms
//! and U.S. weather alerts. All four are published for anyone to use, without a key.

use std::collections::HashMap;
use std::sync::OnceLock;
use std::time::Duration;

use orion_core::feature::Feature;
use orion_core::time::JulianDate;
use serde_json::Value;

use crate::listing::{Listing, ParseError};

/// Earthquakes of magnitude 2.5 and over in the past seven days, worldwide, from the
/// U.S. Geological Survey. The file is rebuilt every minute; the Survey asks that it
/// not be fetched more often than that.
pub const EARTHQUAKES: Listing = Listing {
    source: "U.S. Geological Survey",
    url: "https://earthquake.usgs.gov/earthquakes/feed/v1.0/summary/2.5_week.geojson",
    accept: None,
    cache_name: "earthquakes.geojson",
    parse: parse_earthquakes,
    refresh_every: Duration::from_secs(5 * 60),
};

/// Hot spots seen in the past 24 hours by the MODIS instruments on NASA's Terra and
/// Aqua satellites, worldwide, from NASA's Fire Information for Resource Management
/// System. The file is rebuilt a few times an hour as passes come in.
pub const FIRE_DETECTIONS: Listing = Listing {
    source: "NASA FIRMS",
    url: "https://firms.modaps.eosdis.nasa.gov/data/active_fire/modis-c6.1/csv/MODIS_C6_1_Global_24h.csv",
    accept: None,
    cache_name: "fire_detections.csv",
    parse: parse_fire_detections,
    refresh_every: Duration::from_secs(60 * 60),
};

/// Tropical storms being tracked by the U.S. National Hurricane Center, which covers
/// the Atlantic and the eastern and central Pacific. Advisories come every few hours.
pub const STORMS: Listing = Listing {
    source: "NOAA National Hurricane Center",
    url: "https://www.nhc.noaa.gov/CurrentStorms.json",
    accept: None,
    cache_name: "storms.json",
    parse: parse_storms,
    refresh_every: Duration::from_secs(30 * 60),
};

/// Weather alerts in force in the United States, from the National Weather Service.
pub const ALERTS: Listing = Listing {
    source: "U.S. National Weather Service",
    url: "https://api.weather.gov/alerts/active?status=actual&message_type=alert",
    accept: Some("application/geo+json"),
    cache_name: "alerts.geojson",
    parse: parse_alerts,
    refresh_every: Duration::from_secs(5 * 60),
};

/// Classes of earthquake, by magnitude.
pub mod quake_class {
    pub const UNDER_4_5: u8 = 0;
    pub const FROM_4_5: u8 = 1;
    pub const FROM_6: u8 = 2;
}

/// Classes of fire detection, by how sure the instrument is that it saw a fire.
pub mod fire_class {
    pub const HIGH: u8 = 0;
    pub const NOMINAL: u8 = 1;
    pub const LOW: u8 = 2;
}

/// Classes of tropical storm.
pub mod storm_class {
    pub const HURRICANE: u8 = 0;
    pub const TROPICAL_STORM: u8 = 1;
    pub const OTHER: u8 = 2;
}

/// Classes of alert, by the severity its issuer gave it.
pub mod alert_class {
    pub const EXTREME: u8 = 0;
    pub const SEVERE: u8 = 1;
    pub const MODERATE: u8 = 2;
    pub const MINOR: u8 = 3;
    pub const UNKNOWN: u8 = 4;
}

fn json(text: &str) -> Result<Value, ParseError> {
    serde_json::from_str(text).map_err(|error| ParseError(error.to_string()))
}

fn features_of(document: &Value) -> Result<&Vec<Value>, ParseError> {
    document["features"]
        .as_array()
        .ok_or_else(|| ParseError("no list of features".to_owned()))
}

/// Writes a moment as `2026-11-25 10:14 UTC`, to the nearest minute.
pub(crate) fn format_moment(moment: JulianDate) -> String {
    let time = JulianDate(moment.0 + 30.0 / 86_400.0).to_utc();
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02} UTC",
        time.year, time.month, time.day, time.hour, time.minute
    )
}

/// A number without a trailing `.0`, such as `10` or `10.5`.
fn tidy(number: f64) -> String {
    let rounded = (number * 10.0).round() / 10.0;
    if rounded.fract() == 0.0 {
        format!("{rounded:.0}")
    } else {
        format!("{rounded:.1}")
    }
}

/// Reads the U.S. Geological Survey's earthquake listing, strongest first.
pub fn parse_earthquakes(text: &str) -> Result<Vec<Feature>, ParseError> {
    let document = json(text)?;
    let mut quakes: Vec<Feature> = features_of(&document)?
        .iter()
        .filter_map(|entry| {
            let properties = &entry["properties"];
            let coordinates = entry["geometry"]["coordinates"].as_array()?;
            let (longitude, latitude) = (
                coordinates.first()?.as_f64()?,
                coordinates.get(1)?.as_f64()?,
            );
            let magnitude = properties["mag"].as_f64()?;
            let place = properties["place"].as_str().unwrap_or("Unnamed place");
            let name = match properties["title"].as_str() {
                Some(title) => title.to_owned(),
                None => format!("M {} - {place}", tidy(magnitude)),
            };
            let mut quake = Feature::at(entry["id"].as_str()?, name, latitude, longitude);
            quake.size = magnitude as f32;
            quake.class = match magnitude {
                m if m >= 6.0 => quake_class::FROM_6,
                m if m >= 4.5 => quake_class::FROM_4_5,
                _ => quake_class::UNDER_4_5,
            };
            quake.time = properties["time"]
                .as_f64()
                .map(|milliseconds| JulianDate::from_unix_seconds(milliseconds / 1000.0));

            let scale = properties["magType"].as_str().unwrap_or("");
            quake.row("Magnitude", format!("{} {scale}", tidy(magnitude)));
            quake.row("Place", place);
            if let Some(depth) = coordinates.get(2).and_then(Value::as_f64) {
                quake.row("Depth", format!("{} km", tidy(depth)));
            }
            if let Some(time) = quake.time {
                quake.row("Time", format_moment(time));
            }
            if let Some(felt) = properties["felt"].as_u64().filter(|felt| *felt > 0) {
                quake.row("Felt reports", felt.to_string());
            }
            if let Some(alert) = properties["alert"].as_str() {
                quake.row("Impact alert level", alert);
            }
            if properties["tsunami"].as_u64() == Some(1) {
                quake.row("Tsunami", "A tsunami message was issued for this event");
            }
            if properties["type"]
                .as_str()
                .is_some_and(|kind| kind != "earthquake")
            {
                quake.row("Kind of event", properties["type"].as_str().unwrap_or(""));
            }
            quake.row("Review", properties["status"].as_str().unwrap_or(""));
            quake.row("More", properties["url"].as_str().unwrap_or(""));
            quake.is_on_earth().then_some(quake)
        })
        .collect();
    quakes.sort_by(|a, b| b.size.total_cmp(&a.size));
    Ok(quakes)
}

/// Reads NASA's list of MODIS fire detections, most intense first.
pub fn parse_fire_detections(text: &str) -> Result<Vec<Feature>, ParseError> {
    let mut lines = text.lines();
    let heading: Vec<&str> = lines.next().unwrap_or("").trim().split(',').collect();
    let column = |name: &str| {
        heading
            .iter()
            .position(|title| *title == name)
            .ok_or_else(|| ParseError(format!("no {name} column")))
    };
    let (latitude, longitude) = (column("latitude")?, column("longitude")?);
    let (date, time) = (column("acq_date")?, column("acq_time")?);
    let (confidence, power) = (column("confidence")?, column("frp")?);
    let (satellite, brightness) = (column("satellite")?, column("brightness")?);
    let day_or_night = column("daynight")?;

    let mut detections: Vec<Feature> = lines
        .filter_map(|line| {
            let fields: Vec<&str> = line.trim().split(',').collect();
            let field = |index: usize| fields.get(index).copied();
            let latitude_deg: f64 = field(latitude)?.parse().ok()?;
            let longitude_deg: f64 = field(longitude)?.parse().ok()?;
            // The time of day is hours and minutes run together, such as 0011.
            let clock = format!("{:0>4}", field(time)?);
            let moment = JulianDate::parse_utc(&format!(
                "{} {}:{}",
                field(date)?,
                &clock[..2],
                &clock[2..]
            ))?;
            let confidence_percent: f64 = field(confidence)?.parse().ok()?;
            let megawatts: f64 = field(power)?.parse().unwrap_or(0.0);

            let name = format!("Fire detection at {latitude_deg:.2}, {longitude_deg:.2}");
            let mut detection = Feature::at(line.trim(), name, latitude_deg, longitude_deg);
            detection.time = Some(moment);
            detection.size = megawatts as f32;
            detection.class = match confidence_percent {
                c if c >= 80.0 => fire_class::HIGH,
                c if c >= 30.0 => fire_class::NOMINAL,
                _ => fire_class::LOW,
            };
            detection.row("Seen at", format_moment(moment));
            detection.row(
                "Seen by",
                match field(satellite)? {
                    "T" | "Terra" => "MODIS on the Terra satellite",
                    "A" | "Aqua" => "MODIS on the Aqua satellite",
                    other => other,
                },
            );
            detection.row("Confidence", format!("{} %", tidy(confidence_percent)));
            detection.row("Radiative power", format!("{} MW", tidy(megawatts)));
            if let Some(kelvin) = field(brightness).and_then(|text| text.parse::<f64>().ok()) {
                detection.row("Brightness temperature", format!("{} K", tidy(kelvin)));
            }
            detection.row(
                "Pass",
                match field(day_or_night)? {
                    "D" => "Daytime",
                    "N" => "Night-time",
                    other => other,
                },
            );
            detection.is_on_earth().then_some(detection)
        })
        .collect();
    detections.sort_by(|a, b| b.size.total_cmp(&a.size));
    Ok(detections)
}

/// Reads the National Hurricane Center's list of storms being tracked, strongest first.
pub fn parse_storms(text: &str) -> Result<Vec<Feature>, ParseError> {
    let document = json(text)?;
    let listed = document["activeStorms"]
        .as_array()
        .ok_or_else(|| ParseError("no list of storms".to_owned()))?;
    let mut storms: Vec<Feature> = listed
        .iter()
        .filter_map(|entry| {
            let text_of = |key: &str| entry[key].as_str().unwrap_or("");
            // Numbers arrive sometimes as numbers and sometimes as text.
            let number = |key: &str| {
                entry[key]
                    .as_f64()
                    .or_else(|| entry[key].as_str()?.trim().parse().ok())
            };
            let latitude = number("latitudeNumeric")?;
            let longitude = number("longitudeNumeric")?;
            let (kind, class) = match text_of("classification") {
                "HU" | "MH" => ("Hurricane", storm_class::HURRICANE),
                "TY" | "STY" => ("Typhoon", storm_class::HURRICANE),
                "TS" => ("Tropical storm", storm_class::TROPICAL_STORM),
                "STS" => ("Subtropical storm", storm_class::TROPICAL_STORM),
                "TD" => ("Tropical depression", storm_class::OTHER),
                "STD" => ("Subtropical depression", storm_class::OTHER),
                "PTC" => ("Post-tropical cyclone", storm_class::OTHER),
                "PC" => ("Potential tropical cyclone", storm_class::OTHER),
                _ => ("Tropical cyclone", storm_class::OTHER),
            };
            let name = format!("{kind} {}", text_of("name"));
            let mut storm = Feature::at(text_of("id"), name.trim(), latitude, longitude);
            storm.class = class;
            storm.row("Kind", kind);
            if let Some(knots) = number("intensity") {
                storm.size = knots as f32;
                storm.row(
                    "Sustained wind",
                    format!("{} kt ({} km/h)", tidy(knots), tidy(knots * 1.852)),
                );
            }
            if let Some(millibars) = number("pressure") {
                storm.row("Central pressure", format!("{} mb", tidy(millibars)));
            }
            if let (Some(heading), Some(knots)) = (number("movementDir"), number("movementSpeed")) {
                storm.row(
                    "Moving",
                    format!(
                        "towards {}° at {} kt ({} km/h)",
                        tidy(heading),
                        tidy(knots),
                        tidy(knots * 1.852)
                    ),
                );
            }
            storm.time = JulianDate::parse_rfc3339(text_of("lastUpdate"));
            if let Some(time) = storm.time {
                storm.row("Advisory issued", format_moment(time));
            }
            storm.is_on_earth().then_some(storm)
        })
        .collect();
    storms.sort_by(|a, b| b.size.total_cmp(&a.size));
    Ok(storms)
}

/// Where each U.S. forecast zone and county is: its code, then the latitude and
/// longitude of its middle. Built by `tools/make_nws_zones.py` from files the National
/// Weather Service publishes.
const ZONES: &str = include_str!("../data/nws_zones.tsv");

fn zones() -> &'static HashMap<&'static str, [f64; 2]> {
    static TABLE: OnceLock<HashMap<&'static str, [f64; 2]>> = OnceLock::new();
    TABLE.get_or_init(|| {
        ZONES
            .lines()
            .filter_map(|line| {
                let mut fields = line.split('\t');
                let code = fields.next()?;
                let latitude = fields.next()?.parse().ok()?;
                let longitude = fields.next()?.trim().parse().ok()?;
                Some((code, [longitude, latitude]))
            })
            .collect()
    })
}

/// The middle of some points given as longitude then latitude, found on the globe
/// itself so that points either side of the date line do not average to Greenwich.
fn middle(points: impl Iterator<Item = [f64; 2]>) -> Option<[f64; 2]> {
    let mut sum = [0.0; 3];
    let mut count = 0;
    for [longitude, latitude] in points {
        let (sin_lat, cos_lat) = latitude.to_radians().sin_cos();
        let (sin_lon, cos_lon) = longitude.to_radians().sin_cos();
        sum[0] += cos_lat * cos_lon;
        sum[1] += cos_lat * sin_lon;
        sum[2] += sin_lat;
        count += 1;
    }
    let length = (sum[0] * sum[0] + sum[1] * sum[1] + sum[2] * sum[2]).sqrt();
    (count > 0 && length > 1e-9).then(|| {
        [
            sum[1].atan2(sum[0]).to_degrees(),
            (sum[2] / length).asin().to_degrees(),
        ]
    })
}

/// The rings of a GeoJSON polygon or multi-polygon, as lines.
fn rings(geometry: &Value) -> Vec<Vec<[f64; 2]>> {
    let ring = |ring: &Value| -> Vec<[f64; 2]> {
        ring.as_array()
            .map(|points| {
                points
                    .iter()
                    .filter_map(|point| Some([point[0].as_f64()?, point[1].as_f64()?]))
                    .collect()
            })
            .unwrap_or_default()
    };
    let polygon = |polygon: &Value| -> Vec<Vec<[f64; 2]>> {
        polygon
            .as_array()
            .map(|rings| rings.iter().map(ring).collect())
            .unwrap_or_default()
    };
    let coordinates = &geometry["coordinates"];
    let rings = match geometry["type"].as_str() {
        Some("Polygon") => polygon(coordinates),
        Some("MultiPolygon") => coordinates
            .as_array()
            .map(|polygons| polygons.iter().flat_map(polygon).collect())
            .unwrap_or_default(),
        _ => Vec::new(),
    };
    rings.into_iter().filter(|ring| ring.len() >= 2).collect()
}

/// Shortens long text to about `limit` characters, ending at a word.
fn shorten(text: &str, limit: usize) -> String {
    let text = text.trim();
    if text.chars().count() <= limit {
        return text.to_owned();
    }
    let cut: String = text.chars().take(limit).collect();
    let cut = cut
        .rsplit_once(char::is_whitespace)
        .map_or(cut.as_str(), |(head, _)| head);
    format!("{cut}…")
}

/// Reads the National Weather Service's alerts in force, most severe first.
///
/// An alert that carries its own outline is placed in the middle of it. Most name
/// the forecast zones they cover instead, and are placed in the middle of those.
/// An alert that does neither, or names only zones Orion does not know, is left out.
pub fn parse_alerts(text: &str) -> Result<Vec<Feature>, ParseError> {
    let document = json(text)?;
    let mut alerts: Vec<Feature> = features_of(&document)?
        .iter()
        .filter_map(|entry| {
            let properties = &entry["properties"];
            let text_of = |key: &str| properties[key].as_str().unwrap_or("");
            let shape = rings(&entry["geometry"]);
            let place = if shape.is_empty() {
                let codes = properties["geocode"]["UGC"].as_array()?;
                middle(
                    codes
                        .iter()
                        .filter_map(|code| zones().get(code.as_str()?).copied()),
                )?
            } else {
                middle(shape.iter().flatten().copied())?
            };

            let event = match text_of("event") {
                "" => "Weather alert",
                event => event,
            };
            let area = text_of("areaDesc");
            let name = format!("{event}: {}", shorten(area, 60));
            let mut alert = Feature::at(text_of("id"), name, place[1], place[0]);
            alert.shape = shape;
            alert.class = match text_of("severity") {
                "Extreme" => alert_class::EXTREME,
                "Severe" => alert_class::SEVERE,
                "Moderate" => alert_class::MODERATE,
                "Minor" => alert_class::MINOR,
                _ => alert_class::UNKNOWN,
            };
            alert.size = f32::from(alert_class::UNKNOWN - alert.class);
            let moment = |key: &str| JulianDate::parse_rfc3339(text_of(key));
            alert.time = moment("onset").or_else(|| moment("effective"));
            alert.until = moment("ends").or_else(|| moment("expires"));

            alert.row("Alert", event);
            alert.row("Area", shorten(area, 300));
            alert.row("Severity", text_of("severity"));
            alert.row("Urgency", text_of("urgency"));
            alert.row("Certainty", text_of("certainty"));
            if let Some(from) = alert.time {
                alert.row("In force from", format_moment(from));
            }
            if let Some(until) = alert.until {
                alert.row("Until", format_moment(until));
            }
            alert.row("Issued by", text_of("senderName"));
            for (key, limit) in [
                ("headline", 300),
                ("description", 900),
                ("instruction", 500),
            ] {
                let text = shorten(text_of(key), limit);
                if !text.is_empty() {
                    alert.details.push(text);
                }
            }
            alert.is_on_earth().then_some(alert)
        })
        .collect();
    alerts.sort_by(|a, b| a.class.cmp(&b.class).then_with(|| a.name.cmp(&b.name)));
    Ok(alerts)
}

#[cfg(test)]
mod tests {
    use orion_core::layer::CardRow;

    use super::*;

    fn value<'a>(feature: &'a Feature, label: &str) -> Option<&'a str> {
        feature
            .rows
            .iter()
            .find(|row| row.label == label)
            .map(|row| row.value.as_str())
    }

    const QUAKES: &str = r#"{"type":"FeatureCollection","features":[
        {"type":"Feature","id":"us6000udes","properties":{"mag":4.2,"place":"Revilla Gigedo Islands region",
          "time":1795600868791,"url":"https://earthquake.usgs.gov/earthquakes/eventpage/us6000udes","felt":null,
          "alert":null,"status":"reviewed","tsunami":0,"magType":"mb","type":"earthquake",
          "title":"M 4.2 - Revilla Gigedo Islands region"},
         "geometry":{"type":"Point","coordinates":[-108.946,19.8382,10]}},
        {"type":"Feature","id":"us7000big1","properties":{"mag":6.8,"place":"off the coast of somewhere",
          "time":1795500000000,"url":"","felt":212,"alert":"yellow","status":"reviewed","tsunami":1,
          "magType":"mww","type":"earthquake","title":"M 6.8 - off the coast of somewhere"},
         "geometry":{"type":"Point","coordinates":[142.5,38.1,35.4]}},
        {"type":"Feature","id":"nomag","properties":{"mag":null,"place":"x","time":1,"title":"x"},
         "geometry":{"type":"Point","coordinates":[0,0,0]}}]}"#;

    #[test]
    fn earthquakes_are_read_strongest_first() {
        let quakes = parse_earthquakes(QUAKES).unwrap();
        assert_eq!(quakes.len(), 2);
        let (big, small) = (&quakes[0], &quakes[1]);
        assert_eq!(big.name, "M 6.8 - off the coast of somewhere");
        assert_eq!(big.class, quake_class::FROM_6);
        assert_eq!(value(big, "Felt reports"), Some("212"));
        assert_eq!(value(big, "Impact alert level"), Some("yellow"));
        assert!(value(big, "Tsunami").is_some());
        assert_eq!(value(big, "Depth"), Some("35.4 km"));

        assert_eq!(small.class, quake_class::UNDER_4_5);
        assert_eq!(small.size, 4.2);
        assert_eq!(
            (small.latitude_deg, small.longitude_deg),
            (19.8382, -108.946)
        );
        assert_eq!(value(small, "Magnitude"), Some("4.2 mb"));
        assert_eq!(value(small, "Depth"), Some("10 km"));
        assert_eq!(value(small, "Time"), Some("2026-11-25 10:01 UTC"));
        assert_eq!(value(small, "Felt reports"), None);
        assert_eq!(value(small, "Tsunami"), None);
        assert_ne!(big.key, small.key);
    }

    #[test]
    fn answers_that_are_not_listings_are_errors() {
        assert!(parse_earthquakes("<html>Service unavailable</html>").is_err());
        assert!(parse_earthquakes("{}").is_err());
        assert!(parse_alerts("{}").is_err());
        assert!(parse_storms("{}").is_err());
        assert!(parse_fire_detections("<html></html>").is_err());
    }

    #[test]
    fn fire_detections_are_read_most_intense_first() {
        let text = "latitude,longitude,brightness,scan,track,acq_date,acq_time,satellite,confidence,version,bright_t31,frp,daynight\n\
            2.77981,125.41086,314.22,1,1,2026-11-24,0011,T,60,6.1NRT,290.02,10.56,D\n\
            -15.5,-47.25,345.9,1.2,1.1,2026-11-24,1705,A,95,6.1NRT,301.2,180.4,N\n\
            not,a,row\n\
            10.0,10.0,300.0,1,1,2026-11-24,930,A,12,6.1NRT,290.0,3.1,D\n";
        let detections = parse_fire_detections(text).unwrap();
        assert_eq!(detections.len(), 3);
        let strongest = &detections[0];
        assert_eq!(strongest.size, 180.4);
        assert_eq!(strongest.class, fire_class::HIGH);
        assert_eq!(value(strongest, "Seen at"), Some("2026-11-24 17:05 UTC"));
        assert_eq!(
            value(strongest, "Seen by"),
            Some("MODIS on the Aqua satellite")
        );
        assert_eq!(value(strongest, "Pass"), Some("Night-time"));
        assert_eq!(detections[1].class, fire_class::NOMINAL);
        assert_eq!(value(&detections[1], "Radiative power"), Some("10.6 MW"));
        // A time of day written without its leading zero still reads as 09:30.
        assert_eq!(detections[2].class, fire_class::LOW);
        assert_eq!(
            value(&detections[2], "Seen at"),
            Some("2026-11-24 09:30 UTC")
        );
    }

    #[test]
    fn storms_are_read_strongest_first_and_no_storms_is_not_an_error() {
        assert_eq!(parse_storms(r#"{"activeStorms": []}"#).unwrap(), []);

        let text = r#"{"activeStorms":[
            {"id":"ep152026","binNumber":"EP5","name":"Olaf","classification":"TD","intensity":"30",
             "pressure":"1006","latitudeNumeric":14.2,"longitudeNumeric":-112.6,"movementDir":290,
             "movementSpeed":9,"lastUpdate":"2026-10-06T15:00:00.000Z"},
            {"id":"al142026","binNumber":"AT4","name":"Milton","classification":"HU","intensity":"140",
             "pressure":"929","latitudeNumeric":22.5,"longitudeNumeric":-88.8,"movementDir":75,
             "movementSpeed":8,"lastUpdate":"2026-10-06T15:00:00.000Z"}]}"#;
        let storms = parse_storms(text).unwrap();
        assert_eq!(storms.len(), 2);
        assert_eq!(storms[0].name, "Hurricane Milton");
        assert_eq!(storms[0].class, storm_class::HURRICANE);
        assert_eq!(
            value(&storms[0], "Sustained wind"),
            Some("140 kt (259.3 km/h)")
        );
        assert_eq!(value(&storms[0], "Central pressure"), Some("929 mb"));
        assert_eq!(
            value(&storms[0], "Moving"),
            Some("towards 75° at 8 kt (14.8 km/h)")
        );
        assert_eq!(
            value(&storms[0], "Advisory issued"),
            Some("2026-10-06 15:00 UTC")
        );
        assert_eq!(storms[1].name, "Tropical depression Olaf");
        assert_eq!(storms[1].class, storm_class::OTHER);
    }

    const ALERTS_TEXT: &str = r#"{"type":"FeatureCollection","features":[
        {"id":"a","type":"Feature","geometry":null,"properties":{"id":"urn:oid:1","areaDesc":"Gloucester; Camden",
          "geocode":{"SAME":["034015"],"UGC":["NJZ017","NJZ018"]},"effective":"2026-11-25T05:14:00-05:00",
          "onset":"2026-11-26T11:00:00-05:00","expires":"2026-11-25T18:00:00-05:00","ends":"2026-11-27T19:00:00-05:00",
          "severity":"Minor","certainty":"Likely","urgency":"Expected","event":"Coastal Flood Advisory",
          "senderName":"NWS Mount Holly NJ","headline":"Coastal Flood Advisory issued November 25",
          "description":"* WHAT...Up to one foot of inundation.","instruction":"Minor tidal flooding is expected."}},
        {"id":"b","type":"Feature","geometry":{"type":"Polygon","coordinates":[[[-97.0,35.0],[-96.0,35.0],[-96.0,36.0],[-97.0,36.0],[-97.0,35.0]]]},
          "properties":{"id":"urn:oid:2","areaDesc":"Lincoln, OK","geocode":{"UGC":["OKC081"]},
          "effective":"2026-11-25T10:00:00-06:00","expires":"2026-11-25T10:45:00-06:00","severity":"Extreme",
          "certainty":"Observed","urgency":"Immediate","event":"Tornado Warning","senderName":"NWS Norman OK",
          "headline":null,"description":"A tornado is on the ground.","instruction":null}},
        {"id":"c","type":"Feature","geometry":null,"properties":{"id":"urn:oid:3","areaDesc":"Nowhere",
          "geocode":{"UGC":["XXZ999"]},"severity":"Severe","event":"Mystery Warning"}}]}"#;

    #[test]
    fn alerts_are_placed_by_outline_or_by_zone_most_severe_first() {
        let alerts = parse_alerts(ALERTS_TEXT).unwrap();
        // The one naming only an unknown zone cannot be placed and is left out.
        assert_eq!(alerts.len(), 2);

        let tornado = &alerts[0];
        assert_eq!(tornado.name, "Tornado Warning: Lincoln, OK");
        assert_eq!(tornado.class, alert_class::EXTREME);
        assert_eq!(tornado.shape.len(), 1);
        assert!(
            (tornado.latitude_deg - 35.4).abs() < 0.2,
            "{}",
            tornado.latitude_deg
        );
        assert!(
            (tornado.longitude_deg + 96.6).abs() < 0.2,
            "{}",
            tornado.longitude_deg
        );
        assert_eq!(value(tornado, "Until"), Some("2026-11-25 16:45 UTC"));
        assert_eq!(tornado.details, ["A tornado is on the ground."]);

        let flood = &alerts[1];
        assert_eq!(flood.class, alert_class::MINOR);
        assert!(flood.shape.is_empty());
        // Southern New Jersey, from the bundled table of zones.
        assert!(
            (39.0..40.5).contains(&flood.latitude_deg),
            "{}",
            flood.latitude_deg
        );
        assert!(
            (-76.0..-74.0).contains(&flood.longitude_deg),
            "{}",
            flood.longitude_deg
        );
        // It takes effect at its onset and lasts until it ends, not until this message expires.
        assert_eq!(value(flood, "In force from"), Some("2026-11-26 16:00 UTC"));
        assert_eq!(value(flood, "Until"), Some("2026-11-28 00:00 UTC"));
        assert_eq!(
            flood.rows[0],
            CardRow::new("Alert", "Coastal Flood Advisory")
        );
        assert_eq!(flood.details.len(), 3);
    }

    #[test]
    fn the_bundled_zones_cover_land_counties_and_sea() {
        let table = zones();
        assert!(table.len() > 7_000, "{}", table.len());
        for code in ["NJZ017", "TXC201", "ANZ050", "PKZ011"] {
            assert!(table.contains_key(code), "{code}");
        }
    }

    #[test]
    fn the_middle_of_points_across_the_date_line_stays_there() {
        let [longitude, latitude] = middle([[179.0, 50.0], [-179.0, 52.0]].into_iter()).unwrap();
        assert!(longitude.abs() > 179.0, "{longitude}");
        assert!((latitude - 51.0).abs() < 0.1);
        assert_eq!(middle(std::iter::empty()), None);
    }

    #[test]
    fn long_text_is_cut_at_a_word() {
        assert_eq!(shorten("short", 10), "short");
        assert_eq!(shorten("one two three four", 9), "one two…");
    }
}
