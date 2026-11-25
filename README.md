# Orion

A desktop app that shows every publicly catalogued object orbiting Earth on a 3D globe, at its computed position for any chosen moment.

**Status:** early. The app draws a textured globe with a live day/night terminator, which you can turn by dragging and zoom by scrolling, down to about 25 km above the ground. As you come closer the Base Map sharpens, from imagery streamed from NASA, and gains borders, major roads and place names. About 29,000 Tracked Objects are drawn at their current Positions. Click one to see its details, including its owner and where and when it was launched, with its Orbit Path and optionally its Ground Track. A sidebar lists the Layers under their Themes, each with a switch, and a search box across the ones that are on. Under Places there are airports, licence plate cameras, data centres and military sites. Under Energy there are power plants by what they run on, pipelines drawn as their routes, refineries and oil sites, offshore platforms, mines by what they dig, and dams. Under Traffic there are flights: aircraft in the air around the middle of the view, refreshed every few minutes and moved along their headings in between. Under Weather there are earthquakes of the past week, fire detections of the past day, tropical storms and U.S. weather alerts, and pictures laid over the Base Map: each day's clouds, the view from three weather satellites every ten minutes, rain and snow, and sea surface temperature. The pictures follow the Simulation Clock back as far as NASA keeps them. You can also open a file of your own places as an Imported Layer: drop a GeoJSON, KML or CSV file on the window, or give its path in the sidebar. It is read where it is and sent nowhere. Every Layer is fetched only once it is switched on, and the dots of those that have them can be clicked for details. From far away a Layer of many places gathers into Clusters, each showing how many places it stands for; clicking one moves in on it. The satellites Layer can be filtered by kind, Orbit Regime and Group, such as weather satellites or Starlink. The View panel switches between Earth-Fixed View and Star-Fixed View and turns on Follow Mode, and the real naked-eye stars sit behind the globe. A Simulation Clock lets you pause, speed up, reverse or jump the displayed moment within two weeks of now. Element Sets are fetched from CelesTrak when the ones on hand are more than six hours old, kept in a cache on disk, and fall back to a Snapshot bundled with the app when there is no connection.

## Download

Each [release on GitHub](https://github.com/ronininvestigator556/Orion/releases) has a download for macOS, Windows and Linux. They are not signed, so your system will warn you the first time you open one.

- **macOS** (Apple silicon and Intel): unzip, move `Orion.app` to Applications and open it. When macOS refuses, go to System Settings, then Privacy & Security, and choose "Open Anyway".
- **Windows** (64-bit): unzip and run `orion.exe`. If SmartScreen appears, choose "More info", then "Run anyway".
- **Linux** (64-bit): extract the archive and run `./orion`. It needs a desktop with Vulkan graphics drivers.

## Build and run from source

Requires a recent stable Rust toolchain (1.95 or newer) and a GPU with Metal, Vulkan or DirectX 12.

```bash
cargo run
```

To start with a Tracked Object already selected, pass its catalogue number. This selects the International Space Station and shows its Ground Track:

```bash
cargo run -- --select 25544 --ground-track
```

Add `--follow` to keep the selected object centred, and `--star-fixed` to start in Star-Fixed View.

To start with a search already typed in, use `--find`:

```bash
cargo run -- --find "noaa 2"
```

To start with other Layers switched on, list the starts of their names after `--layers`. Adding `--select-found` to `--find` selects the first thing the search finds:

```bash
cargo run -- --layers earthquakes,fire,tropical,u.s. --find "M 6" --select-found
```

To start looking down on a place, give `--at` a latitude and longitude in degrees and, optionally, a height in kilometres. This looks at Paris from 200 km up:

```bash
cargo run -- --at 48.85,2.35,200
```

To open files as Imported Layers at launch, give each to `--import`:

```bash
cargo run -- --import my-places.geojson --import sites.csv
```

To draw a picture to a file without opening a window, add `--snapshot` and, to give streamed imagery longer to arrive, `--wait` with a number of seconds. `--size` sets the picture's size in pixels, such as `--size 1920x1080`:

```bash
cargo run -- --at 48.85,2.35,200 --snapshot paris.png --wait 15
```

## Extracts

Places taken from OpenStreetMap reach the app as Extracts: one small file per Layer, attached to the release named `extracts`. OpenStreetMap's query service must not be used as an app's backend, so the app never asks it anything. The Extracts workflow does, a handful of times each Monday, by running `tools/make_extracts.py`, and replaces the files on that release. It can also be started by hand from the Actions page, which is how the files come to exist the first time. Until then those Layers report that their Source cannot be reached.

To build the Extracts on your own computer, for instance to try a new one:

```bash
python3 tools/make_extracts.py extracts
```

Copying a file from there into the `layers` folder inside Orion's cache folder makes the app use it for a week without fetching.

## Releasing

1. Set `version` under `[workspace.package]` in `Cargo.toml`, run `cargo build` so `Cargo.lock` follows, and commit.
2. Tag that commit `v` plus the version, such as `v0.1.0`, and push the tag.

The Release workflow then builds each platform's download and publishes them as a GitHub Release. It stops if the tag and the version disagree. The macOS download is made by `packaging/macos/bundle.sh`, which can also be run by hand.

Every download carries `THIRD-PARTY-LICENSES.html`, the licences of the crates compiled into Orion. It is generated by [cargo-about](https://github.com/EmbarkStudios/cargo-about) from `about.toml`, which lists the licences a dependency may be under. CI fails if a dependency arrives under any other.

## Layout

- `crates/orion-core`: headless orbital maths (propagation, time, coordinate conversion), with no graphics dependencies.
- `crates/orion-data`: fetching Element Sets, Base Map tiles and the Listings behind other Layers, and caching them on disk, with the rules for how often a Source may be asked.
- `crates/orion-app`: the desktop app, built on `wgpu`, `winit` and `egui`. The borders, roads and place names in its `assets` folder are built from Natural Earth by `cargo run --release --package orion-core --example make_reference`.
- `about.toml`, `about.hbs`: which licences dependencies may use, and the page their texts are gathered into.
- `packaging/`: what turns a compiled binary into each platform's download. The app icon is drawn by `cargo run --package orion-app --example make_icon`, which writes the icon files kept there.
- `tools/`: scripts that prepare data. `make_extracts.py` builds the Extracts from OpenStreetMap, and `make_nws_zones.py` the bundled table of U.S. forecast zones that weather alerts are placed by.
- `.github/workflows/`: checks run on every push, the release build, and the weekly Extracts.
- `GLOSSARY.md`: the project's vocabulary. Code and docs use these terms.
- `docs/adr/`: decision records.

## Credits

- Earth imagery: NASA Blue Marble Next Generation and NASA Black Marble 2016, courtesy of NASA Earth Observatory. Both are in the public domain.
- Streamed imagery: Blue Marble, the Landsat WELD composite and the weather pictures (VIIRS true colour, GOES and Himawari, IMERG precipitation, MUR sea surface temperature), provided by NASA's Global Imagery Browse Services (GIBS), part of NASA's Earth Science Data and Information System (ESDIS).
- Borders, roads and place names: made with [Natural Earth](https://www.naturalearthdata.com), free vector map data in the public domain.
- Airports: [OurAirports](https://ourairports.com), released into the public domain.
- Licence plate cameras, data centres, military sites and every Layer under Energy: © [OpenStreetMap](https://www.openstreetmap.org/copyright) contributors, under the Open Database Licence. That licence, not the code's, covers the Extracts.
- Flights: [adsb.lol](https://adsb.lol), open data from volunteers' receivers, under the Open Database Licence.
- Earthquakes: the U.S. Geological Survey's Earthquake Hazards Program.
- Fire detections: NASA's Fire Information for Resource Management System (FIRMS), part of NASA's Earth Science Data and Information System (ESDIS).
- Tropical storms: the U.S. National Hurricane Center. Weather alerts and forecast zones: the U.S. National Weather Service. Both are part of NOAA, and their data is in the public domain.
- Stars: the Yale Bright Star Catalogue, 5th revised edition (Hoffleit and Warren, 1991), which is in the public domain.
- Orbital data: Element Sets from [CelesTrak](https://celestrak.org), derived from the catalogue published by the U.S. Space Force at Space-Track.org.

## Licence

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT licence](LICENSE-MIT), at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in this work by you, as defined in the Apache-2.0 licence, shall be dual licensed as above, without any additional terms or conditions.
