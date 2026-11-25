//! Orion desktop app: window, event loop and frame pacing.

// A released Windows build is a windowed app and should not open a console beside it.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod basemap;
mod camera;
mod conflict_layers;
mod energy_layers;
mod feature_layer;
mod feed;
mod globe;
mod hazard_layers;
mod layer;
mod lines;
mod objects;
mod orbit_layer;
mod picking;
mod picture_layer;
mod places_layers;
mod reference;
mod renderer;
mod road_layers;
mod sidebar;
mod stars;
mod streetmap;
mod tracked;
mod traffic_layer;
mod ui;
mod unrest_layers;

use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::Context as _;
use glam::{DVec3, Mat4, Vec2, Vec3};
use orion_core::clock::SimulationClock;
use orion_core::sun::{subsolar_point, sun_direction};
use orion_core::time::JulianDate;
use orion_data::{Cache, CelesTrak};
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalPosition};
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::window::{Window, WindowId};

use camera::{CameraRig, ViewFrame};
use layer::{FeatureRef, Layer, LayerView, Layers};
use lines::Segment;
use orbit_layer::OrbitLayer;
use picking::PickView;
use renderer::{Renderer, Scene};

/// Trackpads report scrolling in pixels; this many count as one zoom step.
const PIXELS_PER_SCROLL_STEP: f32 = 50.0;
/// How close, in logical points, the cursor must be to a dot to hover or select it.
const PICK_REACH_POINTS: f32 = 8.0;
/// How close, in logical points, the cursor must be to a route to click it.
const ROUTE_REACH_POINTS: f32 = 6.0;
/// How close, in logical points, the cursor must be to the middle of a Cluster to click it.
const CLUSTER_REACH_POINTS: f32 = 14.0;
/// The most Clusters whose counts are written out at once.
const MAX_CLUSTER_COUNTS: usize = 400;
/// Clicking a Cluster brings the camera to this fraction of its height above the ground.
const CLUSTER_ZOOM: f32 = 0.35;
/// The largest file that will be opened as an Imported Layer.
const MAX_IMPORT_BYTES: u64 = 300 * 1024 * 1024;
/// A press and release less than this many logical points apart is a click, not a drag.
const CLICK_SLOP_POINTS: f64 = 4.0;

/// The Feature the user has picked out.
#[derive(Clone, Copy, Debug)]
struct Selection {
    feature: FeatureRef,
    /// Identifies the Feature across refreshes of its Layer, when its index can change.
    key: u64,
}

fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn,orion=info"))
        .init();

    let options = parse_options(std::env::args().skip(1))?;
    if let Some(path) = options.snapshot.clone() {
        let wait = options.wait_seconds.unwrap_or(SNAPSHOT_WAIT_SECONDS);
        return snapshot(options, &path, Duration::from_secs_f64(wait));
    }
    let event_loop = EventLoop::new().context("creating the event loop")?;
    let mut app = App {
        options,
        ..App::default()
    };
    event_loop
        .run_app(&mut app)
        .context("running the event loop")?;

    match app.startup_error {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

/// What the command line asked for.
#[derive(Clone, Debug, Default, PartialEq)]
struct Options {
    /// Catalogue number of a Tracked Object to select at launch.
    select: Option<u32>,
    /// Show the Ground Track of the selected object from launch.
    ground_track: bool,
    /// Text to put in the search box at launch.
    find: Option<String>,
    /// Select the first thing the search finds, once something is found.
    select_found: bool,
    /// Start in Star-Fixed View.
    star_fixed: bool,
    /// List the Layers that are still on trial.
    experimental: bool,
    /// Start in Follow Mode, which needs `--select` to have something to follow.
    follow: bool,
    /// Start looking down on this place.
    at: Option<Place>,
    /// Layers to switch on at launch, each named by the start of its name.
    layers: Vec<String>,
    /// Files of places to open as Imported Layers at launch.
    imports: Vec<std::path::PathBuf>,
    /// Draw to this picture file, with no window, and exit.
    snapshot: Option<std::path::PathBuf>,
    /// How long to run before taking the snapshot, in seconds.
    wait_seconds: Option<f64>,
    /// The size of the snapshot in pixels, if not the usual one.
    size: Option<[u32; 2]>,
    /// A file to keep settings in, instead of the usual place.
    settings: Option<std::path::PathBuf>,
}

/// Size of a snapshot in pixels, and its pixels per logical point: the same as the
/// app's default window on a high-density display.
const SNAPSHOT_SIZE: [u32; 2] = [2560, 1600];
const SNAPSHOT_SCALE: f64 = 2.0;
/// How long a snapshot runs before the picture is taken, unless `--wait` says otherwise.
const SNAPSHOT_WAIT_SECONDS: f64 = 8.0;

/// Where to start looking: a latitude and longitude in degrees, north and east
/// positive, and optionally a height above the surface in kilometres.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Place {
    latitude_deg: f64,
    longitude_deg: f64,
    altitude_km: Option<f64>,
}

fn parse_place(text: &str) -> Option<Place> {
    let mut numbers = text.split(',').map(|part| part.trim().parse::<f64>());
    let latitude_deg = numbers.next()?.ok()?;
    let longitude_deg = numbers.next()?.ok()?;
    let altitude_km = match numbers.next() {
        Some(altitude) => Some(altitude.ok()?),
        None => None,
    };
    let valid = numbers.next().is_none()
        && (-90.0..=90.0).contains(&latitude_deg)
        && (-180.0..=180.0).contains(&longitude_deg)
        && altitude_km.is_none_or(|altitude| altitude > 0.0);
    valid.then_some(Place {
        latitude_deg,
        longitude_deg,
        altitude_km,
    })
}

/// The names the street map's settings are kept under between runs.
const STREET_CHOICE: &str = "base map.street map";
const STREET_ADDRESS: &str = "street map.address";

const USAGE: &str = "usage: orion [--select <catalogue number>] [--ground-track] [--follow] \
     [--star-fixed] [--find <name or number> [--select-found]] [--at <latitude>,<longitude>[,<km above>]] \
     [--layers <name>[,<name>...]] [--import <GeoJSON, KML or CSV file>] \
     [--settings <file to keep settings in>] [--experimental] \
     [--snapshot <picture file> [--wait <seconds>] [--size <width>x<height>]]";

fn parse_options(mut arguments: impl Iterator<Item = String>) -> anyhow::Result<Options> {
    let mut options = Options::default();
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--select" => {
                let number = arguments
                    .next()
                    .with_context(|| format!("--select needs a catalogue number\n{USAGE}"))?;
                options.select =
                    Some(number.parse().with_context(|| {
                        format!("{number:?} is not a catalogue number\n{USAGE}")
                    })?);
            }
            "--ground-track" => options.ground_track = true,
            "--select-found" => options.select_found = true,
            "--star-fixed" => options.star_fixed = true,
            "--experimental" => options.experimental = true,
            "--follow" => options.follow = true,
            "--at" => {
                let place = arguments
                    .next()
                    .with_context(|| format!("--at needs a place\n{USAGE}"))?;
                options.at = Some(parse_place(&place).with_context(|| {
                    format!(
                        "{place:?} is not a latitude,longitude or latitude,longitude,km\n{USAGE}"
                    )
                })?);
            }
            "--snapshot" => {
                options.snapshot = Some(
                    arguments
                        .next()
                        .with_context(|| format!("--snapshot needs a file name\n{USAGE}"))?
                        .into(),
                );
            }
            "--size" => {
                let size = arguments
                    .next()
                    .with_context(|| format!("--size needs a width and height\n{USAGE}"))?;
                let read = size.split_once('x').and_then(|(width, height)| {
                    Some([width.trim().parse().ok()?, height.trim().parse().ok()?])
                });
                options.size = Some(
                    read.filter(|size: &[u32; 2]| {
                        size.iter().all(|side| (200..=8192).contains(side))
                    })
                    .with_context(|| {
                        format!("{size:?} is not a size such as 2560x1600\n{USAGE}")
                    })?,
                );
            }
            "--wait" => {
                let seconds = arguments
                    .next()
                    .with_context(|| format!("--wait needs a number of seconds\n{USAGE}"))?;
                options.wait_seconds = Some(
                    seconds
                        .parse()
                        .ok()
                        .filter(|seconds: &f64| (0.0..=600.0).contains(seconds))
                        .with_context(|| {
                            format!("{seconds:?} is not a number of seconds\n{USAGE}")
                        })?,
                );
            }
            "--layers" => {
                let names = arguments
                    .next()
                    .with_context(|| format!("--layers needs one or more Layer names\n{USAGE}"))?;
                options.layers.extend(
                    names
                        .split(',')
                        .map(|name| name.trim().to_lowercase())
                        .filter(|name| !name.is_empty()),
                );
            }
            "--settings" => {
                options.settings = Some(
                    arguments
                        .next()
                        .with_context(|| format!("--settings needs a file name\n{USAGE}"))?
                        .into(),
                );
            }
            "--import" => {
                options.imports.push(
                    arguments
                        .next()
                        .with_context(|| format!("--import needs a file name\n{USAGE}"))?
                        .into(),
                );
            }
            "--find" => {
                options.find = Some(
                    arguments
                        .next()
                        .with_context(|| format!("--find needs some text\n{USAGE}"))?,
                );
            }
            other => anyhow::bail!("unknown option {other:?}\n{USAGE}"),
        }
    }
    Ok(options)
}

#[derive(Default)]
struct App {
    options: Options,
    running: Option<Running>,
    startup_error: Option<anyhow::Error>,
}

/// Everything that exists only once the window is open.
struct Running {
    window: Arc<Window>,
    session: Session,
}

/// Everything Orion shows and how the user has arranged it. It draws through whatever
/// the renderer is aimed at, so it runs the same with a window or without one.
struct Session {
    renderer: Renderer,
    rig: CameraRig,
    /// How far, in radians, Earth has turned against the stars at the moment on show.
    sidereal_angle: f32,
    layers: Layers,
    clock: SimulationClock,
    clock_panel: ui::ClockPanel,
    selected: Option<Selection>,
    /// The Feature under the cursor.
    hovered: Option<FeatureRef>,
    sidebar: sidebar::Sidebar,
    /// True until the first search result has been selected, when launched to do so.
    /// Results can take a while to appear, because Layers fetch in the background.
    select_found: bool,
    /// How many files have been opened as Imported Layers, for giving each a colour.
    imported: usize,
    /// Where what is switched on is kept between runs; `None` when it is not kept.
    settings_file: Option<orion_data::settings::SettingsFile>,
    /// What was last written there.
    saved: orion_data::settings::Settings,
    /// Frames drawn so far.
    frames: u64,
    /// What the user has chosen to see of the Base Map.
    base_map: basemap::BaseMapChoices,
    /// The pattern street tiles are fetched from.
    street_address: String,
    place_names: reference::PlaceNames,
    /// Line segments for the current frame, kept to reuse the allocation.
    segments: Vec<Segment>,
    /// The marks of the Clusters on show, as of the last frame.
    cluster_dots: Vec<objects::Instance>,
    /// True while the left button is held after being pressed over the scene.
    dragging: bool,
    /// Where the left button went down over the scene, to tell a click from a drag.
    pressed_at: Option<PhysicalPosition<f64>>,
    cursor: Option<PhysicalPosition<f64>>,
    last_frame: Instant,
    frame_ms: f32,
}

impl Running {
    fn new(event_loop: &ActiveEventLoop, options: Options) -> anyhow::Result<Self> {
        let attributes = Window::default_attributes()
            .with_title("Orion")
            .with_window_icon(window_icon())
            .with_inner_size(LogicalSize::new(1280.0, 800.0));
        let window = Arc::new(
            event_loop
                .create_window(attributes)
                .context("creating the window")?,
        );
        let cache = default_cache();
        let renderer = pollster::block_on(Renderer::new(
            window.clone(),
            event_loop.owned_display_handle(),
            cache.as_ref().map(Cache::tiles),
        ))?;
        let settings_file = match &options.settings {
            Some(path) => Some(orion_data::settings::SettingsFile::at(path)),
            None => orion_data::settings::SettingsFile::in_default_location(),
        };
        Ok(Self {
            window,
            session: Session::new(renderer, cache, options, settings_file)?,
        })
    }

    fn redraw(&mut self) {
        self.session.frame();
        // Positions change continuously, so draw every frame; vsync paces the loop.
        self.window.request_redraw();
    }
}

/// The cache folder, with a warning if the system has nowhere to put one.
fn default_cache() -> Option<Cache> {
    let cache = Cache::in_default_location();
    if cache.is_none() {
        log::warn!("no cache folder is available; nothing fetched will be kept");
    }
    cache
}

/// Draws Orion to a picture file with no window, after running for `wait` so that
/// data arriving over several frames, such as Base Map tiles, has time to load.
fn snapshot(options: Options, path: &std::path::Path, wait: Duration) -> anyhow::Result<()> {
    let cache = default_cache();
    let size = options.size.unwrap_or(SNAPSHOT_SIZE);
    let renderer = pollster::block_on(Renderer::offscreen(
        size[0],
        size[1],
        SNAPSHOT_SCALE,
        cache.as_ref().map(Cache::tiles),
    ))?;
    // A snapshot starts as at first launch, so the same command draws the same
    // picture, unless it is told which settings to use.
    let settings_file = options
        .settings
        .as_ref()
        .map(orion_data::settings::SettingsFile::at);
    let mut session = Session::new(renderer, cache, options, settings_file)?;
    let until = Instant::now() + wait;
    let (mut frames, mut drawing) = (0_u32, Duration::ZERO);
    loop {
        let started = Instant::now();
        session.frame();
        frames += 1;
        drawing += started.elapsed();
        if Instant::now() >= until {
            break;
        }
        std::thread::sleep(Duration::from_millis(16));
    }
    log::info!(
        "drew {frames} frames, {:.1} ms each on average",
        drawing.as_secs_f64() * 1000.0 / f64::from(frames)
    );
    session.keep_settings();
    session
        .renderer
        .read_picture()?
        .save(path)
        .with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

impl Session {
    /// `settings_file` is where what is switched on is kept between runs. Without
    /// one, everything starts as at first launch and nothing is kept.
    fn new(
        mut renderer: Renderer,
        cache: Option<Cache>,
        options: Options,
        settings_file: Option<orion_data::settings::SettingsFile>,
    ) -> anyhow::Result<Self> {
        let layer_cache = cache.clone();
        let mut orbit = OrbitLayer::start(Arc::new(CelesTrak::new()), cache, SystemTime::now())?;
        orbit.set_show_ground_track(options.ground_track);
        let search = options.find.unwrap_or_default();
        orbit.set_search(&search);
        let mut layers = Layers::new(orbit);
        let fetcher: Arc<dyn orion_data::listing::Fetcher> =
            Arc::new(orion_data::listing::Http::new());
        let specs = places_layers::ALL
            .into_iter()
            .chain(energy_layers::ALL)
            .chain(road_layers::ALL)
            .chain(hazard_layers::ALL)
            .chain(conflict_layers::ALL);
        for spec in specs {
            layers.add(Box::new(feature_layer::FeatureLayer::new(
                spec,
                Arc::clone(&fetcher),
                layer_cache.clone(),
            )));
        }
        // Layers on trial are listed only when asked for.
        if options.experimental {
            layers.add(Box::new(feature_layer::FeatureLayer::new(
                &unrest_layers::UNREST_REPORTS,
                Arc::clone(&fetcher),
                layer_cache.clone(),
            )));
        }
        for spec in [&traffic_layer::FLIGHTS, &traffic_layer::SHIPS] {
            layers.add(Box::new(traffic_layer::TrafficLayer::new(
                spec,
                Arc::clone(&fetcher),
            )));
        }
        // Most pictures are found by asking their Source which it holds.
        let picture_source = orion_data::tiles::TileSource::new();
        let picture_cache = layer_cache.as_ref().map(Cache::tiles);
        let find: picture_layer::FindPicture = Arc::new(move |tiles, step, moment| {
            orion_data::tiles::newest_picture(
                &picture_source,
                picture_cache.as_ref(),
                tiles,
                step.written(),
                moment,
            )
        });
        for spec in picture_layer::ALL {
            layers.add(Box::new(picture_layer::PictureLayer::new(
                spec,
                Arc::clone(&find),
            )));
        }
        // Put back what was switched on last time, then what this launch asks for.
        let remembered = settings_file
            .as_ref()
            .map(orion_data::settings::SettingsFile::load)
            .unwrap_or_default();
        for index in 0..layers.len() {
            if let Some(layer) = layers.get_mut(index)
                && let Some(on) = remembered.get(&format!("layer.{}", layer.name()))
            {
                layer.set_on(on);
            }
        }
        let mut base_map = basemap::BaseMapChoices::default();
        for (name, switch) in base_map.switches() {
            if let Some(on) = remembered.get(name) {
                *switch = on;
            }
        }
        if let Some(choice) = remembered
            .get_text(STREET_CHOICE)
            .and_then(streetmap::StreetChoice::from_setting)
        {
            base_map.streets = choice;
        }
        // Where street tiles come from is a setting, because the host may change.
        let street_address = remembered
            .get_text(STREET_ADDRESS)
            .filter(|address| orion_data::streets::StreetTiles::is_address(address))
            .unwrap_or(orion_data::streets::DEFAULT_ADDRESS)
            .to_owned();
        renderer.set_street_tiles(
            orion_data::streets::StreetTiles::new(street_address.clone()),
            layer_cache
                .as_ref()
                .map(|cache| cache.street_tiles(&street_address)),
        );
        for wanted in &options.layers {
            let found = (0..layers.len()).find(|index| {
                layers
                    .get(*index)
                    .is_some_and(|layer| layer.name().to_lowercase().starts_with(wanted))
            });
            match found.and_then(|index| layers.get_mut(index)) {
                Some(layer) => layer.set_on(true),
                None => log::warn!("no Layer has a name starting with {wanted:?}"),
            }
        }

        let selected = options.select.and_then(|catalogue_number| {
            let key = u64::from(catalogue_number);
            let index = layers.orbit.index_of(key);
            if index.is_none() {
                log::warn!("no Tracked Object has catalogue number {catalogue_number}");
            }
            Some(Selection {
                feature: FeatureRef {
                    layer: 0,
                    index: index?,
                },
                key,
            })
        });

        let sidereal_angle = julian_date_now().greenwich_sidereal_angle() as f32;
        let mut rig = CameraRig::default();
        if options.star_fixed {
            rig.set_frame(ViewFrame::StarFixed, sidereal_angle);
        }
        rig.set_following(options.follow && selected.is_some());
        if let Some(place) = options.at {
            let (sin_lat, cos_lat) = place.latitude_deg.to_radians().sin_cos();
            let (sin_lon, cos_lon) = place.longitude_deg.to_radians().sin_cos();
            let direction = glam::DVec3::new(cos_lat * cos_lon, cos_lat * sin_lon, sin_lat);
            rig.face(direction.as_vec3(), sidereal_angle);
            if let Some(altitude_km) = place.altitude_km {
                rig.set_altitude((altitude_km / orion_core::EARTH_EQUATORIAL_RADIUS_KM) as f32);
            }
        }

        let imports = options.imports.clone();
        let mut session = Self {
            renderer,
            rig,
            sidereal_angle,
            layers,
            clock: SimulationClock::new(julian_date_now()),
            clock_panel: ui::ClockPanel::default(),
            selected,
            hovered: None,
            select_found: options.select_found && !search.trim().is_empty(),
            sidebar: sidebar::Sidebar {
                search,
                ..sidebar::Sidebar::default()
            },
            imported: 0,
            settings_file,
            // What the file holds, so that anything this launch changed is written.
            saved: remembered,
            frames: 0,
            base_map,
            street_address,
            place_names: reference::PlaceNames::bundled(),
            segments: Vec::new(),
            cluster_dots: Vec::new(),
            dragging: false,
            pressed_at: None,
            cursor: None,
            last_frame: Instant::now(),
            frame_ms: 0.0,
        };
        for path in imports {
            session.import(&path);
        }
        // The sidebar narrows every Layer to the search each frame; do it once now so
        // that what is found is right before the first frame is drawn.
        let search = session.sidebar.search.clone();
        for index in 0..session.layers.len() {
            if let Some(layer) = session.layers.get_mut(index) {
                layer.set_search(&search);
            }
        }
        Ok(session)
    }

    /// What is switched on now, as it is kept between runs. Imported Layers are left
    /// out: remembering them would mean remembering where the user's files are.
    fn settings(&self) -> orion_data::settings::Settings {
        let mut settings = orion_data::settings::Settings::default();
        for index in 0..self.layers.len() {
            if let Some(layer) = self.layers.get(index).filter(|layer| !layer.is_imported()) {
                settings.set(&format!("layer.{}", layer.name()), layer.is_on());
            }
        }
        let mut base_map = self.base_map;
        for (name, switch) in base_map.switches() {
            settings.set(name, *switch);
        }
        settings.set_text(STREET_CHOICE, self.base_map.streets.setting());
        settings.set_text(STREET_ADDRESS, &self.street_address);
        settings
    }

    /// Writes what is switched on to the settings file, if it has changed.
    fn keep_settings(&mut self) {
        let Some(file) = &self.settings_file else {
            return;
        };
        let settings = self.settings();
        if settings == self.saved {
            return;
        }
        if let Err(error) = file.save(&settings) {
            log::warn!(
                "could not keep settings in {}: {error}",
                file.path().display()
            );
        }
        // Remembered either way, so a folder that cannot be written is not retried
        // every second.
        self.saved = settings;
    }

    /// Opens a file of places as an Imported Layer, and says in the sidebar how it went.
    fn import(&mut self, path: &std::path::Path) {
        let name = path.file_name().map_or_else(
            || path.display().to_string(),
            |name| name.to_string_lossy().into_owned(),
        );
        let opened = std::fs::metadata(path)
            .map_err(|error| error.to_string())
            .and_then(|metadata| {
                if metadata.len() > MAX_IMPORT_BYTES {
                    return Err("it is larger than 300 MB".to_owned());
                }
                std::fs::read(path).map_err(|error| error.to_string())
            })
            .and_then(|bytes| {
                orion_data::import::parse_import(&name, &String::from_utf8_lossy(&bytes))
                    .map_err(|error| error.0)
            });
        self.sidebar.import_result = Some(match opened {
            Ok(features) => {
                let count = features.len();
                let first = features.first().map(|feature| {
                    reference::on_globe([feature.longitude_deg, feature.latitude_deg])
                });
                self.layers
                    .add(Box::new(feature_layer::FeatureLayer::imported(
                        &name,
                        features,
                        self.imported,
                    )));
                self.imported += 1;
                self.sidebar.import_path.clear();
                // Turn to the first place, so something of the file is in view.
                if let Some(first) = first {
                    self.rig.face(first, self.sidereal_angle);
                }
                (format!("Opened {name}: {count} places."), false)
            }
            Err(reason) => {
                log::warn!("opening {}: {reason}", path.display());
                (format!("Could not open {name}: {reason}."), true)
            }
        });
    }

    /// Takes an Imported Layer off the globe.
    fn remove_layer(&mut self, layer: usize) {
        if !self
            .layers
            .get(layer)
            .is_some_and(|layer| layer.is_imported())
        {
            return;
        }
        self.layers.remove(layer);
        self.renderer.layer_removed(layer);
        self.hovered = None;
        // A selection on a later Layer follows it up; one on the Layer removed goes.
        self.selected =
            self.selected
                .and_then(|selection| match selection.feature.layer.cmp(&layer) {
                    std::cmp::Ordering::Less => Some(selection),
                    std::cmp::Ordering::Equal => None,
                    std::cmp::Ordering::Greater => Some(Selection {
                        feature: FeatureRef {
                            layer: selection.feature.layer - 1,
                            ..selection.feature
                        },
                        ..selection
                    }),
                });
    }

    /// Advances everything by one frame and draws it.
    fn frame(&mut self) {
        let now = Instant::now();
        let elapsed_ms = now.duration_since(self.last_frame).as_secs_f32() * 1000.0;
        self.last_frame = now;
        // Exponential smoothing so the readout is legible.
        self.frame_ms += (elapsed_ms - self.frame_ms) * 0.05;

        // Everything in the scene is shown for the Simulation Clock's moment, `now`.
        let real_now = julian_date_now();
        let now = self.clock.advance(real_now, elapsed_ms as f64 / 1000.0);
        let previous_sidereal_angle = self.sidereal_angle;
        self.sidereal_angle = now.greenwich_sidereal_angle() as f32;
        if self.layers.orbit.poll() {
            // Indexes belong to the old set.
            self.hovered = None;
        }
        // Any Layer may have taken in fresh data since the last frame, which moves its
        // Features about. Find the selected one again by its key; if it is no longer
        // listed, nothing is selected.
        self.selected = self.selected.and_then(|selection| {
            let layer = self.layers.get(selection.feature.layer)?;
            if layer.stable_key(selection.feature.index) == Some(selection.key) {
                return Some(selection);
            }
            self.hovered = None;
            let index = layer.index_of(selection.key)?;
            Some(Selection {
                feature: FeatureRef {
                    index,
                    ..selection.feature
                },
                ..selection
            })
        });

        // Hovering uses the dots from the previous frame, a few milliseconds old.
        // The dots are where they were at the previous frame's moment, so they are picked
        // with the camera as it stood then.
        let pick_view = self.pick_view_at(previous_sidereal_angle);
        self.hovered = match self.cursor {
            Some(cursor) if !self.dragging && !self.renderer.pointer_over_interface() => {
                self.pick(&pick_view, cursor)
            }
            _ => None,
        };

        if self.select_found
            && let Some(feature) = self.first_found()
        {
            self.select_found = false;
            self.select_and_face(feature);
        }

        let selected = self.selected.map(|selection| selection.feature);
        // How much of the globe a point of the screen spans decides which Layers gather
        // into Clusters, and the place below the camera what a Layer of traffic fetches.
        let below = self.rig.scene_camera(self.sidereal_angle).position();
        let altitude = below.length() - 1.0;
        let radians_per_pixel = altitude.max(0.0) * 2.0 * (camera::FOV_Y / 2.0).tan()
            / self.renderer.size().height.max(1) as f32;
        let radians_per_point = radians_per_pixel * self.renderer.scale_factor() as f32;
        let latitude = (below.z / below.length()).clamp(-1.0, 1.0).asin();
        self.layers.set_view(&LayerView {
            degrees_per_point: f64::from(radians_per_point.to_degrees()),
            latitude_deg: f64::from(latitude.to_degrees()),
            longitude_deg: f64::from(below.y.atan2(below.x).to_degrees()),
        });
        self.layers.update(now, real_now, selected, self.hovered);

        // Follow Mode needs something to follow, and moves the camera before it is used.
        if selected.is_none() {
            self.rig.set_following(false);
        }
        let followed = selected
            .and_then(|feature| self.layers.dot(feature))
            .filter(|dot| !dot.is_hidden())
            .map(|dot| dot.position());
        self.rig.follow(followed, self.sidereal_angle);
        let pick_view = self.pick_view();

        self.segments.clear();
        self.layers.lines(selected, now, &mut self.segments);

        // Name the selected and hovered Features next to their dots.
        let scale_factor = self.renderer.scale_factor() as f32;
        let mut labels = Vec::new();
        for feature in [
            selected,
            self.hovered.filter(|hovered| Some(*hovered) != selected),
        ]
        .into_iter()
        .flatten()
        {
            let (Some(name), Some(dot)) =
                (self.layers.feature_name(feature), self.layers.dot(feature))
            else {
                continue;
            };
            if let Some(pixels) = pick_view.project(dot.position()) {
                let points = pixels / scale_factor;
                // A dot behind the sidebar is covered by it, and its name should be too.
                if points.x > sidebar::WIDTH {
                    labels.push((egui::pos2(points.x, points.y), name.to_owned()));
                }
            }
        }

        // A mark for each Cluster, larger the more it stands for, and its count
        // written on it where it is in view.
        self.cluster_dots.clear();
        let mut cluster_counts = Vec::new();
        for mark in self.layers.clusters() {
            let [r, g, b] = mark.color;
            let radius_points = 5.5 + 2.0 * (mark.count as f32).log10();
            self.cluster_dots.push(
                objects::Instance::new(mark.position, [r, g, b, 235], objects::Marks::default())
                    .scaled(radius_points / objects::DOT_RADIUS_POINTS),
            );
            if cluster_counts.len() < MAX_CLUSTER_COUNTS
                && let Some(pixels) = pick_view.project(mark.position)
            {
                let points = pixels / scale_factor;
                if points.x > sidebar::WIDTH {
                    cluster_counts.push((
                        egui::pos2(points.x, points.y),
                        orion_core::cluster::format_count(mark.count),
                    ));
                }
            }
        }

        // The street map has names of its own.
        let streets_showing = self.renderer.street_map_showing();
        let place_labels = if self.base_map.names && !streets_showing {
            self.place_names
                .labels(&pick_view, scale_factor, sidebar::WIDTH)
        } else {
            Vec::new()
        };

        // Credit that a Source asks to stay on screen while its data is.
        let mut credits: Vec<&str> = Vec::new();
        if self.base_map.imagery {
            credits.push("Imagery: NASA");
        }
        if streets_showing {
            credits.push(orion_data::streets::CREDIT);
        }
        for index in 0..self.layers.len() {
            let credit = self
                .layers
                .get(index)
                .filter(|layer| layer.is_on())
                .and_then(|layer| layer.attribution());
            if let Some(credit) = credit
                && !credits.contains(&credit)
            {
                credits.push(credit);
            }
        }
        let credits = credits.join("  ·  ");

        let diagnostics = ui::Diagnostics {
            frame_ms: self.frame_ms,
            subsolar: subsolar_point(now),
            tracked: self.layers.orbit.tracked().summary(now),
        };
        let card = selected.and_then(|feature| self.layers.card(feature, now));
        let mut toggles = selected
            .and_then(|feature| self.layers.get(feature.layer))
            .map(|layer| layer.toggles())
            .unwrap_or_default();
        let mut selection_actions = ui::SelectionActions::default();
        let mut view_choices = ui::ViewChoices {
            frame: self.rig.frame(),
            following: self.rig.is_following(),
            base_map: self.base_map,
        };
        let can_follow = selected.is_some();
        let mut clock_action = None;
        let mut picked = None;
        let clock = self.clock;

        // The dots are lent to the scene for the frame, which leaves the Layers free
        // for the sidebar to change while it is drawn.
        let dots = self.layers.take_dots();
        let routes = self.layers.routes();
        let pictures = self.layers.pictures();
        let scene = Scene {
            view_projection: pick_view.view_projection,
            camera_position: pick_view.camera_position,
            sun_direction: DVec3::from_array(sun_direction(now)).as_vec3(),
            objects: &dots,
            clusters: &self.cluster_dots,
            routes: &routes,
            pictures: &pictures,
            lines: &self.segments,
            sidereal_angle: self.sidereal_angle,
            base_map: self.base_map,
        };
        let layers = &mut self.layers;
        let sidebar = &mut self.sidebar;
        let clock_panel = &mut self.clock_panel;
        self.renderer.render(&scene, |ui, gpu| {
            picked =
                sidebar::draw(ui, layers, sidebar, now, real_now, SystemTime::now()).or(picked);
            let ctx = ui.ctx().clone();
            // Place names first, so a Feature's name is drawn over them.
            ui::draw_place_names(&ctx, &place_labels);
            ui::draw_cluster_counts(&ctx, &cluster_counts);
            ui::draw_labels(&ctx, &labels);
            ui::draw_credits(&ctx, &credits);
            ui::draw_view(&ctx, &mut view_choices, can_follow);
            ui::draw_about(&ctx);
            ui::draw(&ctx, gpu, &diagnostics);
            clock_action = ui::draw_clock(&ctx, &clock, real_now, clock_panel).or(clock_action);
            if let Some(card) = &card {
                selection_actions = ui::draw_selected(&ctx, card, &mut toggles);
            }
        });
        self.layers.restore_dots(dots);

        if let Some(layer) = selected.and_then(|feature| self.layers.get_mut(feature.layer)) {
            layer.set_toggles(&toggles);
        }
        if let Some(feature) = picked {
            self.select_and_face(feature);
        }
        if let Some(layer) = self.sidebar.remove_requested.take() {
            self.remove_layer(layer);
        }
        self.frames += 1;
        // About once a second is soon enough to notice a switch being flipped.
        if self.frames.is_multiple_of(60) {
            self.keep_settings();
        }
        if let Some(path) = self.sidebar.open_requested.take() {
            self.import(std::path::Path::new(&path));
        }
        // A Feature on a Layer that has been switched off is no longer on show.
        let selection_is_on_show = self.selected.is_none_or(|selection| {
            self.layers
                .get(selection.feature.layer)
                .is_some_and(|layer| layer.is_on())
        });
        if !selection_is_on_show {
            self.selected = None;
        }
        match clock_action {
            Some(ui::ClockAction::SetPaused(paused)) => self.clock.set_paused(paused),
            Some(ui::ClockAction::Faster) => self.clock.faster(),
            Some(ui::ClockAction::Slower) => self.clock.slower(),
            Some(ui::ClockAction::JumpBy(days)) => self.clock.jump_by(days, real_now),
            Some(ui::ClockAction::JumpTo(moment)) => self.clock.jump_to(moment, real_now),
            Some(ui::ClockAction::ReturnToLive) => self.clock.return_to_live(real_now),
            None => {}
        }
        self.base_map = view_choices.base_map;
        self.rig.set_frame(view_choices.frame, self.sidereal_angle);
        self.rig.set_following(view_choices.following && can_follow);
        if selection_actions.clear {
            self.selected = None;
        }
    }

    /// The camera and window facts that relate the scene to the cursor, for this frame.
    fn pick_view(&self) -> PickView {
        self.pick_view_at(self.sidereal_angle)
    }

    /// The same, for the camera as it stands when Earth has turned by `sidereal_angle`.
    fn pick_view_at(&self, sidereal_angle: f32) -> PickView {
        let size = self.renderer.size();
        let camera = self.rig.scene_camera(sidereal_angle);
        // Slide the picture right so the globe sits in the middle of the space beside
        // the sidebar, not the middle of the window. Clip space is 2 units wide.
        let width = size.width.max(1) as f32;
        let shift = sidebar::WIDTH * self.renderer.scale_factor() as f32 / width;
        let centre_beside_sidebar = Mat4::from_translation(Vec3::new(shift, 0.0, 0.0));
        PickView {
            view_projection: centre_beside_sidebar
                * camera.view_projection(width / size.height.max(1) as f32),
            camera_position: camera.position(),
            viewport: Vec2::new(size.width as f32, size.height as f32),
        }
    }

    /// The Feature whose dot is under the cursor, if any.
    fn pick(&self, view: &PickView, cursor: PhysicalPosition<f64>) -> Option<FeatureRef> {
        let dot = view.pick(
            self.layers.dots(),
            Vec2::new(cursor.x as f32, cursor.y as f32),
            PICK_REACH_POINTS * self.renderer.scale_factor() as f32,
        )?;
        self.layers.feature_at(dot)
    }

    /// Selects a Feature, remembering its key so the selection survives a refresh.
    fn select(&mut self, feature: FeatureRef) {
        let key = self
            .layers
            .get(feature.layer)
            .and_then(|layer| layer.stable_key(feature.index));
        self.selected = key.map(|key| Selection { feature, key });
    }

    /// Selects a Feature and turns the globe to bring it into view.
    /// The first search result on the last Layer that has one, once every Layer that is
    /// on has its data. Later Layers are the smaller, more particular ones, so a match
    /// there is the likelier one meant.
    fn first_found(&self) -> Option<FeatureRef> {
        let layers = (0..self.layers.len()).filter_map(|index| self.layers.get(index));
        if layers.clone().any(|layer| layer.is_loading()) {
            return None;
        }
        (0..self.layers.len()).rev().find_map(|layer| {
            let found = self
                .layers
                .get(layer)
                .filter(|layer| layer.is_on())?
                .found(1);
            let (index, _) = found.first()?;
            Some(FeatureRef {
                layer,
                index: *index,
            })
        })
    }

    fn select_and_face(&mut self, feature: FeatureRef) {
        self.select(feature);
        if let Some(position) = self.layers.position(feature) {
            self.rig.face(position, self.sidereal_angle);
        }
    }

    /// Selects the Feature under the cursor, or clears the selection if there is none.
    fn click(&mut self, cursor: PhysicalPosition<f64>) {
        let view = self.pick_view();
        if let Some(feature) = self.pick(&view, cursor) {
            self.select(feature);
            return;
        }
        let scale_factor = self.renderer.scale_factor() as f32;
        let cursor = Vec2::new(cursor.x as f32, cursor.y as f32);
        if let Some(feature) =
            self.layers
                .pick_route(&view, cursor, ROUTE_REACH_POINTS * scale_factor)
        {
            self.select(feature);
            return;
        }
        // A click on a Cluster moves in on it, far enough to start breaking it up.
        let on_cluster = view.pick(
            &self.cluster_dots,
            cursor,
            CLUSTER_REACH_POINTS * scale_factor,
        );
        match on_cluster.and_then(|index| self.cluster_dots.get(index)) {
            Some(mark) => {
                let altitude = view.camera_position.length() - 1.0;
                self.rig.face(mark.position(), self.sidereal_angle);
                self.rig.set_altitude(altitude * CLUSTER_ZOOM);
            }
            None => self.selected = None,
        }
    }

    /// Applies a mouse or trackpad event to the camera and the selection.
    fn on_scene_input(&mut self, event: &WindowEvent, used_by_interface: bool) {
        match event {
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Left,
                ..
            } => {
                // A drag or click may only start over the scene.
                self.dragging = !used_by_interface;
                self.pressed_at = self.cursor.filter(|_| !used_by_interface);
            }
            WindowEvent::MouseInput {
                state: ElementState::Released,
                button: MouseButton::Left,
                ..
            } => {
                // A drag always ends on release, wherever the cursor is by then.
                self.dragging = false;
                if let (Some(pressed_at), Some(cursor)) = (self.pressed_at.take(), self.cursor) {
                    let moved = (cursor.x - pressed_at.x).hypot(cursor.y - pressed_at.y);
                    if moved < CLICK_SLOP_POINTS * self.renderer.scale_factor() {
                        self.click(cursor);
                    }
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                if let (true, Some(previous)) = (self.dragging, self.cursor) {
                    self.rig.drag(
                        (position.x - previous.x) as f32,
                        (position.y - previous.y) as f32,
                        self.renderer.size().height as f32,
                    );
                }
                self.cursor = Some(*position);
            }
            WindowEvent::CursorLeft { .. } => self.cursor = None,
            WindowEvent::MouseWheel { delta, .. } if !used_by_interface => {
                let steps = match delta {
                    MouseScrollDelta::LineDelta(_, lines) => *lines,
                    MouseScrollDelta::PixelDelta(pixels) => {
                        pixels.y as f32 / PIXELS_PER_SCROLL_STEP
                    }
                };
                self.rig.zoom(steps);
            }
            _ => {}
        }
    }
}

/// The icon shown in the title bar and taskbar on Windows and Linux. macOS takes its
/// icon from the app bundle instead and ignores this.
fn window_icon() -> Option<winit::window::Icon> {
    let image = image::load_from_memory(include_bytes!("../assets/icon-256.png"))
        .inspect_err(|error| log::warn!("could not read the window icon: {error}"))
        .ok()?
        .into_rgba8();
    let (width, height) = image.dimensions();
    winit::window::Icon::from_rgba(image.into_raw(), width, height)
        .inspect_err(|error| log::warn!("could not use the window icon: {error}"))
        .ok()
}

fn julian_date_now() -> JulianDate {
    let since_epoch = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("the system clock is set before 1970");
    JulianDate::from_unix_seconds(since_epoch.as_secs_f64())
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.running.is_some() {
            return;
        }
        match Running::new(event_loop, self.options.clone()) {
            Ok(running) => {
                running.window.request_redraw();
                self.running = Some(running);
            }
            Err(error) => {
                self.startup_error = Some(error);
                event_loop.exit();
            }
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        let Some(running) = &mut self.running else {
            return;
        };

        let session = &mut running.session;
        let used_by_interface = session.renderer.on_window_event(&event);
        session.on_scene_input(&event, used_by_interface);

        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            // A file dropped on the window is opened as an Imported Layer.
            WindowEvent::DroppedFile(path) => running.session.import(&path),
            WindowEvent::Resized(size) => running.session.renderer.resize(size.width, size.height),
            WindowEvent::RedrawRequested => running.redraw(),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(arguments: &[&str]) -> anyhow::Result<Options> {
        parse_options(arguments.iter().map(|argument| argument.to_string()))
    }

    #[test]
    fn options_default_to_nothing_selected() {
        assert_eq!(parse(&[]).unwrap(), Options::default());
    }

    #[test]
    fn options_can_select_an_object_and_show_its_ground_track() {
        assert_eq!(
            parse(&["--ground-track", "--select", "25544"]).unwrap(),
            Options {
                select: Some(25_544),
                ground_track: true,
                ..Options::default()
            }
        );
        assert_eq!(
            parse(&["--layers", "Earthquakes, fire", "--layers", "u.s."])
                .unwrap()
                .layers,
            ["earthquakes", "fire", "u.s."]
        );
        let picture = parse(&["--snapshot", "out.png", "--wait", "2.5"]).unwrap();
        assert_eq!(
            picture.snapshot.as_deref(),
            Some(std::path::Path::new("out.png"))
        );
        assert_eq!(picture.wait_seconds, Some(2.5));
        assert_eq!(
            parse(&["--size", "1920x1080"]).unwrap().size,
            Some([1920, 1080])
        );
        assert!(parse(&["--size", "huge"]).is_err());
        assert!(parse(&["--size", "10x10"]).is_err());
        assert_eq!(
            parse(&["--import", "a.kml", "--import", "b.csv"])
                .unwrap()
                .imports
                .len(),
            2
        );
        assert_eq!(
            parse(&["--at", "37.77,-122.42,60"]).unwrap().at,
            Some(Place {
                latitude_deg: 37.77,
                longitude_deg: -122.42,
                altitude_km: Some(60.0),
            })
        );
        assert_eq!(
            parse(&["--at", "-33.9, 151.2"])
                .unwrap()
                .at
                .unwrap()
                .altitude_km,
            None
        );
        let views = parse(&["--star-fixed", "--follow"]).unwrap();
        assert!(views.star_fixed && views.follow);
        assert!(!views.experimental);
        assert!(parse(&["--experimental"]).unwrap().experimental);
        assert_eq!(
            parse(&["--find", "noaa 2"]).unwrap().find.as_deref(),
            Some("noaa 2")
        );
    }

    #[test]
    fn bad_options_are_explained() {
        for arguments in [
            &["--select"][..],
            &["--select", "station"],
            &["--find"],
            &["--snapshot"],
            &["--wait"],
            &["--wait", "soon"],
            &["--wait", "-1"],
            &["--at"],
            &["--at", "north pole"],
            &["--at", "95,0"],
            &["--at", "10,20,-5"],
            &["--at", "10,20,30,40"],
            &["--wat"],
        ] {
            let error = parse(arguments).unwrap_err();
            assert!(format!("{error:#}").contains("usage: orion"), "{error:#}");
        }
    }
}
