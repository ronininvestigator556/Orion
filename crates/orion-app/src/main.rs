//! Orion desktop app: window, event loop and frame pacing.

mod camera;
mod feed;
mod globe;
mod lines;
mod objects;
mod picking;
mod renderer;
mod tracked;
mod ui;

use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use anyhow::Context as _;
use glam::{DVec3, Vec2};
use orion_core::sun::{subsolar_point, sun_direction};
use orion_core::time::JulianDate;
use orion_data::{Cache, CelesTrak};
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalPosition};
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::window::{Window, WindowId};

use camera::OrbitCamera;
use feed::Feed;
use lines::Segment;
use picking::PickView;
use renderer::{Renderer, Scene};
use tracked::{Highlight, Tracked};

/// Trackpads report scrolling in pixels; this many count as one zoom step.
const PIXELS_PER_SCROLL_STEP: f32 = 50.0;
/// How close, in logical points, the cursor must be to a dot to hover or select it.
const PICK_REACH_POINTS: f32 = 8.0;
/// A press and release less than this many logical points apart is a click, not a drag.
const CLICK_SLOP_POINTS: f64 = 4.0;
/// Linear colours with alpha.
const ORBIT_PATH_COLOR: [f32; 4] = [1.0, 1.0, 1.0, 0.9];
const GROUND_TRACK_COLOR: [f32; 4] = [1.0, 0.75, 0.15, 0.9];

/// The Tracked Object the user has picked out.
#[derive(Clone, Copy, Debug)]
struct Selection {
    /// Identifies the object across refreshes, when its index can change.
    catalogue_number: u32,
    index: usize,
}

fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn,orion=info"))
        .init();

    let options = parse_options(std::env::args().skip(1))?;
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
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Options {
    /// Catalogue number of a Tracked Object to select at launch.
    select: Option<u32>,
    /// Show the Ground Track of the selected object from launch.
    ground_track: bool,
}

const USAGE: &str = "usage: orion [--select <catalogue number>] [--ground-track]";

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
    renderer: Renderer,
    camera: OrbitCamera,
    tracked: Tracked,
    feed: Feed,
    selected: Option<Selection>,
    /// Index of the Tracked Object under the cursor.
    hovered: Option<usize>,
    show_ground_track: bool,
    /// Line segments for the current frame, kept to reuse the allocation.
    segments: Vec<Segment>,
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
            .with_inner_size(LogicalSize::new(1280.0, 800.0));
        let window = Arc::new(
            event_loop
                .create_window(attributes)
                .context("creating the window")?,
        );
        let renderer = pollster::block_on(Renderer::new(
            window.clone(),
            event_loop.owned_display_handle(),
        ))?;

        let cache = Cache::in_default_location();
        if cache.is_none() {
            log::warn!("no cache folder is available; Element Sets will not be fetched");
        }
        let (feed, objects) = Feed::start(Arc::new(CelesTrak::new()), cache, SystemTime::now())?;

        let tracked = Tracked::new(objects);
        let selected = options.select.and_then(|catalogue_number| {
            let index = tracked.index_of(catalogue_number);
            if index.is_none() {
                log::warn!("no Tracked Object has catalogue number {catalogue_number}");
            }
            Some(Selection {
                catalogue_number,
                index: index?,
            })
        });

        Ok(Self {
            window,
            renderer,
            camera: OrbitCamera::default(),
            tracked,
            feed,
            selected,
            hovered: None,
            show_ground_track: options.ground_track,
            segments: Vec::new(),
            dragging: false,
            pressed_at: None,
            cursor: None,
            last_frame: Instant::now(),
            frame_ms: 0.0,
        })
    }

    fn redraw(&mut self) {
        let now = Instant::now();
        let elapsed_ms = now.duration_since(self.last_frame).as_secs_f32() * 1000.0;
        self.last_frame = now;
        // Exponential smoothing so the readout is legible.
        self.frame_ms += (elapsed_ms - self.frame_ms) * 0.05;

        // Until the Simulation Clock exists, the scene always shows the present moment.
        let now = julian_date_now();
        if let Some(objects) = self.feed.poll() {
            self.tracked.replace(objects);
            // Indexes belong to the old set; find the selected object again by number.
            self.hovered = None;
            self.selected = self.selected.and_then(|selection| {
                let index = self.tracked.index_of(selection.catalogue_number)?;
                Some(Selection { index, ..selection })
            });
        }

        // Hovering uses the dots from the previous frame, a few milliseconds old.
        let pick_view = self.pick_view();
        self.hovered = match self.cursor {
            Some(cursor) if !self.dragging && !self.renderer.pointer_over_interface() => {
                self.pick(&pick_view, cursor)
            }
            _ => None,
        };

        let selected = self.selected.map(|selection| selection.index);
        self.tracked.propagate(
            now,
            Highlight {
                selected,
                hovered: self.hovered,
            },
        );

        self.segments.clear();
        if let Some(index) = selected {
            let orbit_path = self.tracked.orbit_path(index, now);
            lines::push_polyline(&mut self.segments, &orbit_path, ORBIT_PATH_COLOR);
            if self.show_ground_track {
                let ground_track = self.tracked.ground_track(index, now);
                lines::push_polyline(&mut self.segments, &ground_track, GROUND_TRACK_COLOR);
            }
        }

        let scene = Scene {
            view_projection: pick_view.view_projection,
            camera_position: pick_view.camera_position,
            sun_direction: DVec3::from_array(sun_direction(now)).as_vec3(),
            objects: self.tracked.instances(),
            lines: &self.segments,
        };

        // Name the selected and hovered objects next to their dots.
        let scale_factor = self.window.scale_factor() as f32;
        let mut labels = Vec::new();
        for index in [
            selected,
            self.hovered.filter(|hovered| Some(*hovered) != selected),
        ]
        .into_iter()
        .flatten()
        {
            let (Some(object), Some(instance)) = (
                self.tracked.object(index),
                self.tracked.instances().get(index),
            ) else {
                continue;
            };
            if let Some(pixels) = pick_view.project(instance.position()) {
                let points = pixels / scale_factor;
                labels.push((egui::pos2(points.x, points.y), object.name.clone()));
            }
        }

        let diagnostics = ui::Diagnostics {
            frame_ms: self.frame_ms,
            subsolar: subsolar_point(now),
            tracked: self.tracked.summary(now),
        };
        let details = selected.and_then(|index| self.tracked.details(index, now));
        let feed_status = self.feed.status(SystemTime::now());
        let mut refresh_requested = false;
        let mut show_ground_track = self.show_ground_track;
        let mut selection_actions = ui::SelectionActions::default();
        self.renderer.render(&self.window, &scene, |ctx, gpu| {
            ui::draw_labels(ctx, &labels);
            ui::draw(ctx, gpu, &diagnostics);
            refresh_requested |= ui::draw_element_sets(ctx, &feed_status);
            if let Some(details) = &details {
                selection_actions = ui::draw_selected(ctx, details, &mut show_ground_track);
            }
        });
        self.show_ground_track = show_ground_track;
        if selection_actions.clear {
            self.selected = None;
        }
        if refresh_requested {
            self.feed.request_refresh(SystemTime::now());
        }

        // Positions change continuously, so draw every frame; vsync paces the loop.
        self.window.request_redraw();
    }

    /// The camera and window facts that relate the scene to the cursor, for this frame.
    fn pick_view(&self) -> PickView {
        let size = self.window.inner_size();
        PickView {
            view_projection: self
                .camera
                .view_projection(size.width as f32 / size.height.max(1) as f32),
            camera_position: self.camera.position(),
            viewport: Vec2::new(size.width as f32, size.height as f32),
        }
    }

    /// The index of the Tracked Object whose dot is under the cursor, if any.
    fn pick(&self, view: &PickView, cursor: PhysicalPosition<f64>) -> Option<usize> {
        view.pick(
            self.tracked.instances(),
            Vec2::new(cursor.x as f32, cursor.y as f32),
            PICK_REACH_POINTS * self.window.scale_factor() as f32,
        )
    }

    /// Selects the Tracked Object under the cursor, or clears the selection if there is none.
    fn click(&mut self, cursor: PhysicalPosition<f64>) {
        self.selected = self.pick(&self.pick_view(), cursor).and_then(|index| {
            Some(Selection {
                catalogue_number: self.tracked.object(index)?.catalogue_number,
                index,
            })
        });
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
                    if moved < CLICK_SLOP_POINTS * self.window.scale_factor() {
                        self.click(cursor);
                    }
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                if let (true, Some(previous)) = (self.dragging, self.cursor) {
                    self.camera.drag(
                        (position.x - previous.x) as f32,
                        (position.y - previous.y) as f32,
                        self.window.inner_size().height as f32,
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
                self.camera.zoom(steps);
            }
            _ => {}
        }
    }
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
        match Running::new(event_loop, self.options) {
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

        let used_by_interface = running.renderer.on_window_event(&running.window, &event);
        running.on_scene_input(&event, used_by_interface);

        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => running.renderer.resize(size.width, size.height),
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
            }
        );
    }

    #[test]
    fn bad_options_are_explained() {
        for arguments in [&["--select"][..], &["--select", "station"], &["--wat"]] {
            let error = parse(arguments).unwrap_err();
            assert!(format!("{error:#}").contains("usage: orion"), "{error:#}");
        }
    }
}
