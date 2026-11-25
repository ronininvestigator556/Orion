//! Orion desktop app: window, event loop and frame pacing.

mod camera;
mod feed;
mod globe;
mod objects;
mod renderer;
mod tracked;
mod ui;

use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use anyhow::Context as _;
use glam::DVec3;
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
use renderer::{Renderer, Scene};
use tracked::Tracked;

/// Trackpads report scrolling in pixels; this many count as one zoom step.
const PIXELS_PER_SCROLL_STEP: f32 = 50.0;

fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn,orion=info"))
        .init();

    let event_loop = EventLoop::new().context("creating the event loop")?;
    let mut app = App::default();
    event_loop
        .run_app(&mut app)
        .context("running the event loop")?;

    match app.startup_error {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

#[derive(Default)]
struct App {
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
    /// True while the left button is held after being pressed over the scene.
    dragging: bool,
    cursor: Option<PhysicalPosition<f64>>,
    last_frame: Instant,
    frame_ms: f32,
}

impl Running {
    fn new(event_loop: &ActiveEventLoop) -> anyhow::Result<Self> {
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

        Ok(Self {
            window,
            renderer,
            camera: OrbitCamera::default(),
            tracked: Tracked::new(objects),
            feed,
            dragging: false,
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
        }
        self.tracked.propagate(now);

        let size = self.window.inner_size();
        let scene = Scene {
            view_projection: self
                .camera
                .view_projection(size.width as f32 / size.height.max(1) as f32),
            camera_position: self.camera.position(),
            sun_direction: DVec3::from_array(sun_direction(now)).as_vec3(),
            objects: self.tracked.instances(),
        };

        let diagnostics = ui::Diagnostics {
            frame_ms: self.frame_ms,
            subsolar: subsolar_point(now),
            tracked: self.tracked.summary(now),
        };
        let feed_status = self.feed.status(SystemTime::now());
        let mut refresh_requested = false;
        self.renderer.render(&self.window, &scene, |ctx, gpu| {
            ui::draw(ctx, gpu, &diagnostics);
            refresh_requested |= ui::draw_element_sets(ctx, &feed_status);
        });
        if refresh_requested {
            self.feed.request_refresh(SystemTime::now());
        }

        // Positions change continuously, so draw every frame; vsync paces the loop.
        self.window.request_redraw();
    }

    /// Applies a mouse or trackpad event to the camera.
    fn on_scene_input(&mut self, event: &WindowEvent, used_by_interface: bool) {
        match event {
            WindowEvent::MouseInput {
                state,
                button: MouseButton::Left,
                ..
            } => {
                // A drag may only start over the scene, but always ends on release,
                // wherever the cursor is by then.
                self.dragging = *state == ElementState::Pressed && !used_by_interface;
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
        match Running::new(event_loop) {
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
