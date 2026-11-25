//! The Listings behind Layers of fixed places: airports, and the Extracts the project
//! builds from OpenStreetMap.

use std::time::Duration;

use orion_core::feature::Feature;

use crate::listing::{Listing, ParseError};

const WEEK: Duration = Duration::from_secs(7 * 24 * 60 * 60);

/// Every airport, heliport and seaplane base in the world, from OurAirports, a
/// volunteer-kept list released into the public domain and rebuilt every night.
pub const AIRPORTS: Listing = Listing {
    source: "OurAirports",
    url: "https://davidmegginson.github.io/ourairports-data/airports.csv",
    accept: None,
    cache_name: "airports.csv",
    parse: parse_airports,
    refresh_every: WEEK,
};

/// Classes of airport, by the kind OurAirports gives each.
pub mod airport_class {
    pub const LARGE: u8 = 0;
    pub const MEDIUM: u8 = 1;
    pub const SMALL: u8 = 2;
    pub const HELIPORT: u8 = 3;
    pub const SEAPLANE_BASE: u8 = 4;
    pub const OTHER: u8 = 5;
}

/// Reads OurAirports' list, largest airports first. Closed ones are left out.
pub fn parse_airports(text: &str) -> Result<Vec<Feature>, ParseError> {
    let mut reader = csv::Reader::from_reader(text.as_bytes());
    let headers = reader
        .headers()
        .map_err(|error| ParseError(error.to_string()))?
        .clone();
    let column = |name: &str| {
        headers
            .iter()
            .position(|title| title == name)
            .ok_or_else(|| ParseError(format!("no {name} column")))
    };
    let (ident, kind, name) = (column("ident")?, column("type")?, column("name")?);
    let (latitude, longitude) = (column("latitude_deg")?, column("longitude_deg")?);
    let (elevation, country) = (column("elevation_ft")?, column("iso_country")?);
    let (town, scheduled) = (column("municipality")?, column("scheduled_service")?);
    let (iata, icao) = (column("iata_code")?, column("gps_code")?);

    let mut airports: Vec<Feature> = reader
        .records()
        .filter_map(|record| {
            let record = record.ok()?;
            let field = |index: usize| record.get(index).unwrap_or("").trim();
            let (class, kind_name) = match field(kind) {
                "large_airport" => (airport_class::LARGE, "Large airport"),
                "medium_airport" => (airport_class::MEDIUM, "Medium airport"),
                "small_airport" => (airport_class::SMALL, "Small airport"),
                "heliport" => (airport_class::HELIPORT, "Heliport"),
                "seaplane_base" => (airport_class::SEAPLANE_BASE, "Seaplane base"),
                "closed" => return None,
                _ => (airport_class::OTHER, "Other landing place"),
            };
            let latitude_deg: f64 = field(latitude).parse().ok()?;
            let longitude_deg: f64 = field(longitude).parse().ok()?;
            let title = match field(iata) {
                "" => field(name).to_owned(),
                code => format!("{} ({code})", field(name)),
            };
            let mut airport = Feature::at(field(ident), title, latitude_deg, longitude_deg);
            airport.class = class;

            let mut attribute = |label: &str, value: &str| {
                if !value.is_empty() {
                    airport.attributes.push_str(label);
                    airport.attributes.push('\t');
                    airport.attributes.push_str(value);
                    airport.attributes.push('\n');
                }
            };
            attribute("Kind", kind_name);
            let codes: Vec<String> = [("IATA", field(iata)), ("ICAO", field(icao))]
                .into_iter()
                .filter(|(_, code)| !code.is_empty())
                .map(|(scheme, code)| format!("{scheme} {code}"))
                .collect();
            attribute("Codes", &codes.join(", "));
            let place: Vec<&str> = [field(town), field(country)]
                .into_iter()
                .filter(|part| !part.is_empty())
                .collect();
            attribute("Serves", &place.join(", "));
            if let Ok(feet) = field(elevation).parse::<f64>() {
                attribute(
                    "Elevation",
                    &format!("{:.0} m ({feet:.0} ft)", feet * 0.3048),
                );
            }
            if field(scheduled) == "yes" {
                attribute("Scheduled flights", "Yes");
            }
            airport.is_on_earth().then_some(airport)
        })
        .collect();
    if airports.is_empty() {
        return Err(ParseError("no airports listed".to_owned()));
    }
    airports.sort_by(|a, b| a.class.cmp(&b.class).then_with(|| a.name.cmp(&b.name)));
    Ok(airports)
}

/// Where the project's scheduled job publishes Extracts: files attached to one
/// release of the repository, replaced each time the job runs (ADR 0004).
macro_rules! extract_url {
    ($file:literal) => {
        concat!(
            env!("CARGO_PKG_REPOSITORY"),
            "/releases/download/extracts/",
            $file
        )
    };
}

const fn extract(url: &'static str, cache_name: &'static str) -> Listing {
    Listing {
        source: "OpenStreetMap",
        url,
        accept: None,
        cache_name,
        parse: parse_extract,
        refresh_every: WEEK,
    }
}

/// Cameras that read licence plates, as mapped in OpenStreetMap.
pub const CAMERAS: Listing = extract(extract_url!("cameras.tsv"), "cameras.tsv");
/// Classes of camera, by who made it.
pub mod camera_class {
    pub const FLOCK_SAFETY: u8 = 0;
    pub const OTHER_MAKER: u8 = 1;
    pub const UNKNOWN_MAKER: u8 = 2;
}

/// Data centres, as mapped in OpenStreetMap.
pub const DATA_CENTRES: Listing = extract(extract_url!("data_centres.tsv"), "data_centres.tsv");

/// Oil and gas platforms at sea, as mapped in OpenStreetMap.
pub const OFFSHORE_PLATFORMS: Listing = extract(
    extract_url!("offshore_platforms.tsv"),
    "offshore_platforms.tsv",
);

/// Military bases, airfields and naval bases, as mapped in OpenStreetMap.
pub const MILITARY_SITES: Listing =
    extract(extract_url!("military_sites.tsv"), "military_sites.tsv");
/// Classes of military site.
pub mod military_class {
    pub const BASE: u8 = 0;
    pub const AIRFIELD: u8 = 1;
    pub const NAVAL_BASE: u8 = 2;
}

/// Power plants, as mapped in OpenStreetMap. Each one's size is its capacity in
/// megawatts, where that is recorded.
pub const POWER_PLANTS: Listing = extract(extract_url!("power_plants.tsv"), "power_plants.tsv");
/// Classes of power plant, by what it runs on.
pub mod plant_class {
    pub const SOLAR: u8 = 0;
    pub const WIND: u8 = 1;
    pub const HYDRO: u8 = 2;
    pub const GAS: u8 = 3;
    pub const COAL: u8 = 4;
    pub const OIL: u8 = 5;
    pub const NUCLEAR: u8 = 6;
    pub const BIOMASS: u8 = 7;
    pub const GEOTHERMAL: u8 = 8;
    pub const OTHER: u8 = 9;
}

/// Mines and quarries whose product is recorded, as mapped in OpenStreetMap.
pub const MINES: Listing = extract(extract_url!("mines.tsv"), "mines.tsv");
/// Classes of mine, by what is dug.
pub mod mine_class {
    pub const COAL: u8 = 0;
    pub const IRON: u8 = 1;
    pub const COPPER: u8 = 2;
    pub const PRECIOUS: u8 = 3;
    pub const LITHIUM: u8 = 4;
    pub const OTHER_MINERAL: u8 = 5;
    pub const STONE: u8 = 6;
    pub const NOT_RECORDED: u8 = 7;
}

/// Refineries and other oil and gas sites on land, as mapped in OpenStreetMap.
pub const OIL_AND_GAS_SITES: Listing = extract(
    extract_url!("oil_and_gas_sites.tsv"),
    "oil_and_gas_sites.tsv",
);

/// Named dams, as mapped in OpenStreetMap.
pub const DAMS: Listing = extract(extract_url!("dams.tsv"), "dams.tsv");

/// Oil, gas and fuel pipelines, as mapped in OpenStreetMap. Each is a route.
pub const PIPELINES: Listing = extract(extract_url!("pipelines.tsv"), "pipelines.tsv");
/// Classes of pipeline, by what it carries.
pub mod pipeline_class {
    pub const GAS: u8 = 0;
    pub const OIL: u8 = 1;
    pub const FUEL: u8 = 2;
}

/// How many decimal places of a degree an Extract's lines are written to.
const EXTRACT_LINE_PRECISION: f64 = 1e4;

/// Reads a line written as an encoded polyline, the compact form web maps use: each
/// point is the step from the one before, latitude then longitude, in units of
/// `1 / precision` degrees, packed five bits to a character. Returns longitude then
/// latitude for each point; text that breaks off mid-point ends the line there.
fn decode_line(text: &str, precision: f64) -> Vec<[f64; 2]> {
    let mut points = Vec::new();
    let mut bytes = text.bytes();
    let (mut latitude, mut longitude) = (0_i64, 0_i64);
    let next_step = |bytes: &mut std::str::Bytes<'_>| -> Option<i64> {
        let (mut value, mut shift) = (0_i64, 0);
        loop {
            let chunk = i64::from(bytes.next()?.checked_sub(63)?);
            value |= (chunk & 0x1F) << shift;
            shift += 5;
            if chunk < 0x20 {
                break;
            }
            if shift > 60 {
                return None;
            }
        }
        // The lowest bit carries the sign.
        Some(if value & 1 == 1 {
            !(value >> 1)
        } else {
            value >> 1
        })
    };
    while let Some(step) = next_step(&mut bytes) {
        let Some(across) = next_step(&mut bytes) else {
            break;
        };
        latitude += step;
        longitude += across;
        points.push([longitude as f64 / precision, latitude as f64 / precision]);
    }
    points
}

/// The first field of the first line of every Extract.
const EXTRACT_MARK: &str = "#orion-extract";
/// The version of the Extract format this build reads.
const EXTRACT_VERSION: &str = "1";

/// Reads an Extract built by `tools/make_extracts.py`.
///
/// The first line names the format, its version, the layer, the day it was built and
/// how many places follow. Every other line is one place: its OpenStreetMap id,
/// latitude, longitude, class, name, details as `Label=value` pairs separated by
/// `|`, then optionally a size and, for a route, its line as encoded polylines
/// separated by spaces. Lines that cannot be read are skipped.
pub fn parse_extract(text: &str) -> Result<Vec<Feature>, ParseError> {
    let mut lines = text.lines();
    let mut heading = lines.next().unwrap_or("").split('\t');
    if heading.next() != Some(EXTRACT_MARK) {
        return Err(ParseError("not an Extract".to_owned()));
    }
    match heading.next() {
        Some(EXTRACT_VERSION) => {}
        other => {
            return Err(ParseError(format!(
                "an Extract of version {}, which this version of Orion cannot read",
                other.unwrap_or("unknown")
            )));
        }
    }
    let built = heading.nth(1).unwrap_or("").to_owned();

    let places: Vec<Feature> = lines
        .filter_map(|line| {
            let mut fields = line.split('\t');
            let id = fields.next()?;
            let latitude: f64 = fields.next()?.parse().ok()?;
            let longitude: f64 = fields.next()?.parse().ok()?;
            let class: u8 = fields.next()?.parse().ok()?;
            let name = fields.next()?;
            let details = fields.next().unwrap_or("");

            let mut place = Feature::at(id, name, latitude, longitude);
            place.class = class;
            place.size = fields
                .next()
                .and_then(|size| size.parse().ok())
                .unwrap_or(0.0);
            place.shape = fields
                .next()
                .unwrap_or("")
                .split(' ')
                .map(|line| decode_line(line, EXTRACT_LINE_PRECISION))
                .filter(|line| line.len() >= 2)
                .collect();
            for pair in details.split('|') {
                if let Some((label, value)) = pair.split_once('=') {
                    place.attributes.push_str(label);
                    place.attributes.push('\t');
                    place.attributes.push_str(value);
                    place.attributes.push('\n');
                }
            }
            // The id says which kind of OpenStreetMap object it is and its number.
            let kind = match id.as_bytes().first() {
                Some(b'n') => "node",
                Some(b'w') => "way",
                Some(b'r') => "relation",
                _ => "object",
            };
            place.attributes.push_str(&format!(
                "OpenStreetMap\t{kind} {}\nCollected\t{built}\n",
                id.get(1..).unwrap_or("")
            ));
            place.is_on_earth().then_some(place)
        })
        .collect();
    if places.is_empty() {
        return Err(ParseError("an Extract with no places in it".to_owned()));
    }
    Ok(places)
}

#[cfg(test)]
mod tests {
    use super::*;

    const AIRPORTS_CSV: &str = "\
\"id\",\"ident\",\"type\",\"name\",\"latitude_deg\",\"longitude_deg\",\"elevation_ft\",\"continent\",\"iso_country\",\"iso_region\",\"municipality\",\"scheduled_service\",\"icao_code\",\"iata_code\",\"gps_code\",\"local_code\",\"home_link\",\"wikipedia_link\",\"keywords\"
6523,\"00A\",\"heliport\",\"Total RF Heliport\",40.070985,-74.933689,11,\"NA\",\"US\",\"US-PA\",\"Bensalem\",\"no\",,,\"K00A\",\"00A\",,,
2434,\"EGLL\",\"large_airport\",\"London Heathrow Airport\",51.4706,-0.461941,83,\"EU\",\"GB\",\"GB-ENG\",\"London\",\"yes\",\"EGLL\",\"LHR\",\"EGLL\",,\"http://www.heathrowairport.com/\",,\"LON, Londres\"
1,\"XX01\",\"closed\",\"Old Field\",10,10,0,\"NA\",\"US\",\"US-TX\",\"Nowhere\",\"no\",,,,,,,
2,\"XX02\",\"small_airport\",\"Bad Place\",not-a-number,10,0,\"NA\",\"US\",\"US-TX\",\"Nowhere\",\"no\",,,,,,,
3,\"SCQP\",\"medium_airport\",\"La Araucanía Airport\",-38.9259,-72.6515,321,\"SA\",\"CL\",\"CL-AR\",\"Temuco\",\"yes\",\"SCQP\",\"ZCO\",\"SCQP\",,,,
";

    #[test]
    fn airports_are_read_largest_first_without_the_closed_ones() {
        let airports = parse_airports(AIRPORTS_CSV).unwrap();
        let names: Vec<&str> = airports
            .iter()
            .map(|airport| airport.name.as_str())
            .collect();
        assert_eq!(
            names,
            [
                "London Heathrow Airport (LHR)",
                "La Araucanía Airport (ZCO)",
                "Total RF Heliport"
            ]
        );
        let heathrow = &airports[0];
        assert_eq!(heathrow.class, airport_class::LARGE);
        assert_eq!(
            (heathrow.latitude_deg, heathrow.longitude_deg),
            (51.4706, -0.461941)
        );
        assert_eq!(
            heathrow.attributes,
            "Kind\tLarge airport\nCodes\tIATA LHR, ICAO EGLL\nServes\tLondon, GB\nElevation\t25 m (83 ft)\nScheduled flights\tYes\n"
        );
        assert_eq!(airports[2].class, airport_class::HELIPORT);
        assert!(airports[2].attributes.contains("Codes\tICAO K00A\n"));
        assert!(!airports[2].attributes.contains("Scheduled"));
    }

    #[test]
    fn a_file_that_is_not_the_airport_list_is_an_error() {
        assert!(parse_airports("<html>moved</html>").is_err());
        assert!(parse_airports("ident,type,name\nA,heliport,B\n").is_err());
        // The right columns with nothing under them is as wrong as the wrong columns.
        let heading = AIRPORTS_CSV.lines().next().unwrap();
        assert!(parse_airports(heading).is_err());
    }

    const EXTRACT: &str = "#orion-extract\t1\tcameras\t2026-11-25\t3\n\
        n123\t33.74900\t-84.38800\t0\tLicence plate camera, Anytown Police\tOperator=Anytown Police|Maker=Flock Safety|Facing=270\n\
        w456\t51.50000\t-0.12000\t2\tLicence plate camera\t\n\
        broken line\n\
        r7\t95.0\t0.0\t1\tNowhere\t\n";

    #[test]
    fn an_extract_is_read_into_places_with_their_details() {
        let places = parse_extract(EXTRACT).unwrap();
        assert_eq!(places.len(), 2);
        let camera = &places[0];
        assert_eq!(camera.name, "Licence plate camera, Anytown Police");
        assert_eq!(camera.class, camera_class::FLOCK_SAFETY);
        assert_eq!(
            (camera.latitude_deg, camera.longitude_deg),
            (33.749, -84.388)
        );
        assert_eq!(
            camera.attributes,
            "Operator\tAnytown Police\nMaker\tFlock Safety\nFacing\t270\nOpenStreetMap\tnode 123\nCollected\t2026-11-25\n"
        );
        assert_eq!(
            places[1].attributes,
            "OpenStreetMap\tway 456\nCollected\t2026-11-25\n"
        );
        assert_ne!(places[0].key, places[1].key);
    }

    #[test]
    fn encoded_lines_are_read_back_into_points() {
        // The worked example from the format's own documentation, at five places.
        let line = decode_line("_p~iF~ps|U_ulLnnqC_mqNvxq`@", 1e5);
        assert_eq!(line, [[-120.2, 38.5], [-120.95, 40.7], [-126.453, 43.252]]);
        // The same three points as tools/make_extracts.py writes them, at four.
        let line = decode_line("o}nV~sjhA_~i@vsM_zp@jnjB", EXTRACT_LINE_PRECISION);
        assert_eq!(line, [[-120.2, 38.5], [-120.95, 40.7], [-126.453, 43.252]]);
        assert!(decode_line("", 1e4).is_empty());
        // Text cut off mid-point gives the points before the break.
        assert_eq!(decode_line("o}nV~sjhA_~i@", 1e4).len(), 1);
    }

    #[test]
    fn an_extract_can_carry_sizes_and_routes() {
        let text = "#orion-extract\t1\tpipelines\t2026-11-25\t2\n\
            w1\t40.70000\t-120.95000\t0\tPipeline, natural gas\tCarries=natural gas\t\to}nV~sjhA_~i@vsM_zp@jnjB\n\
            n2\t10.00000\t20.00000\t3\tBig Plant\tRuns on=Gas\t450\t\n";
        let places = parse_extract(text).unwrap();
        assert_eq!(places[0].shape.len(), 1);
        assert_eq!(places[0].shape[0].len(), 3);
        assert_eq!(places[0].shape[0][0], [-120.2, 38.5]);
        assert_eq!(places[0].size, 0.0);
        assert!(places[1].shape.is_empty());
        assert_eq!(places[1].size, 450.0);
    }

    #[test]
    fn files_that_are_not_extracts_this_version_reads_are_errors() {
        assert!(parse_extract("Not Found").is_err());
        assert!(parse_extract("").is_err());
        let newer = EXTRACT.replacen("\t1\t", "\t2\t", 1);
        let error = parse_extract(&newer).unwrap_err();
        assert!(error.0.contains("version 2"), "{error}");
        assert!(parse_extract("#orion-extract\t1\tcameras\t2026-11-25\t0\n").is_err());
    }

    #[test]
    fn extracts_are_fetched_from_the_projects_own_release() {
        assert!(CAMERAS.url.starts_with("https://github.com/"));
        assert!(
            CAMERAS
                .url
                .ends_with("/releases/download/extracts/cameras.tsv")
        );
        assert_eq!(CAMERAS.source, "OpenStreetMap");
    }
}
