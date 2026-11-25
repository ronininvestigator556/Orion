# Orion

A desktop app that shows publicly available information about the world on a 3D globe, in Layers you switch on and off: what is in orbit, and places, energy, traffic and weather on and above the surface. It shows places and vehicles as their Sources publish them, and is not a tool for following individuals.

**Status:** early, and moving fast. Version 0.1.0 showed satellites only; version 0.2.0 adds everything else below.

## What it shows

The globe has a live day and night side and the real naked-eye stars behind it. Drag to turn it and scroll to zoom, down to about 1.5 km above the ground. As you come closer the Base Map sharpens, from imagery streamed from NASA, and gains borders, major roads and place names. Below about 50 km it gives way to a street map from OpenStreetMap, with every street and its name; the View panel can keep it to imagery, or show streets from any height.

A sidebar lists the Layers under their Themes, each with a switch, a filter and a line saying where its data came from. Nothing is fetched for a Layer until it is switched on, and which Layers are on is remembered between launches.

- **Orbit.** About 29,000 Tracked Objects at their computed Positions. Click one for its details, including its owner and where and when it was launched, with its Orbit Path and optionally its Ground Track. Filter by kind, Orbit Regime and Group, such as weather satellites or Starlink.
- **Places.** Airports, licence plate cameras, data centres and military sites.
- **Energy.** Power plants by what they run on, pipelines drawn as their routes, refineries and oil sites, offshore platforms, mines by what they dig, and dams.
- **Traffic.** Flights around the middle of the view, refreshed every few minutes and moved along their headings in between. Ships in the northern Baltic, the one sea whose ships are published for anyone to use without a key.
- **Weather.** Pictures laid over the Base Map: each day's clouds, the view from three weather satellites every ten minutes, rain and snow, and sea surface temperature.
- **Hazards.** Earthquakes of the past week, fire detections of the past day, tropical storms and U.S. weather alerts. And Travel Advisories: one mark for each country, coloured by the level the United States sets from 1 to 4, with the United Kingdom's and Canada's advice beside it and a link to each government's page.
- **Imported.** A file of your own places: drop a GeoJSON, KML or CSV file on the window, or give its path in the sidebar. It is read where it is and sent nowhere.

A search box looks across every Layer that is on. From far away a Layer of many places gathers into Clusters, each showing how many places it stands for; clicking one moves in on it.

A Simulation Clock pauses, speeds up, reverses or jumps the moment on show, within two weeks of now. Satellites follow it anywhere; earthquakes and the weather pictures follow it back as far as their Sources keep them; live Layers such as flights show only the present, and say so when the clock is elsewhere. The View panel switches between Earth-Fixed View and Star-Fixed View and turns on Follow Mode, which keeps a selected satellite, flight or ship centred.

Element Sets for satellites are fetched from CelesTrak when the ones on hand are more than six hours old, kept in a cache on disk, and fall back to a Snapshot bundled with the app when there is no connection.

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

The same release carries the file of Travel Advisories, which the Travel Advisories workflow rebuilds four times a day by running `tools/make_advisories.py`. Each government files its advice under its own names and codes for countries, so the job joins them once for everybody.

## On trial

Starting Orion with `--experimental` lists Layers that are still being judged and are not part of a release:

```bash
cargo run -- --experimental
```

- **Protests and unrest**, under the Unrest Theme. Unrest Reports are protests, protests with violence and strikes as the news reported them, by town, for the past three days. The Unrest Reports workflow sifts them every hour from the GDELT Project's index of the world's news by running `tools/make_unrest.py`, and publishes one small file beside the Extracts. GDELT reads the news by machine and is often wrong, so the job keeps only what an article's own address says is about a protest, and Orion shows by default only what at least two news outlets carried. A report never says where a crowd is now, or what is planned.

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
- `tools/`: scripts that prepare data. `make_extracts.py` builds the Extracts from OpenStreetMap, `make_unrest.py` sifts the news into Unrest Reports, and `make_advisories.py` joins three governments' Travel Advisories, placing countries by `countries.tsv`, which `make_countries.py` builds. `make_nws_zones.py` builds the bundled table of U.S. forecast zones that weather alerts are placed by.
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
- Ships: Fintraffic / [digitraffic.fi](https://www.digitraffic.fi/en/marine-traffic/), under the Creative Commons Attribution 4.0 licence.
- Earthquakes: the U.S. Geological Survey's Earthquake Hazards Program.
- Fire detections: NASA's Fire Information for Resource Management System (FIRMS), part of NASA's Earth Science Data and Information System (ESDIS).
- Tropical storms: the U.S. National Hurricane Center. Weather alerts and forecast zones: the U.S. National Weather Service. Both are part of NOAA, and their data is in the public domain.
- Travel Advisories: the U.S. Department of State (public domain); the UK Foreign, Commonwealth & Development Office, which contains public sector information licensed under the [Open Government Licence v3.0](https://www.nationalarchives.gov.uk/doc/open-government-licence/version/3/); and Global Affairs Canada, which contains information licensed under the [Open Government Licence - Canada](https://open.canada.ca/en/open-government-licence-canada). Countries are placed using [Natural Earth](https://www.naturalearthdata.com).
- Stars: the Yale Bright Star Catalogue, 5th revised edition (Hoffleit and Warren, 1991), which is in the public domain.
- Orbital data: Element Sets from [CelesTrak](https://celestrak.org), derived from the catalogue published by the U.S. Space Force at Space-Track.org.

## Licence

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT licence](LICENSE-MIT), at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in this work by you, as defined in the Apache-2.0 licence, shall be dual licensed as above, without any additional terms or conditions.
