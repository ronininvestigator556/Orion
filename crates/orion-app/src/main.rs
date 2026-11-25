//! Orion desktop app: window, event loop and frame pacing.

mod renderer;
mod ui;

use std::sync::Arc;
use std::time::Instant;

use anyhow::Context as _;
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::window::{Window, WindowId};

use renderer::Renderer;

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

        Ok(Self {
            window,
            renderer,
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

        let frame_ms = self.frame_ms;
        self.renderer.render(&self.window, |ctx, gpu| {
            ui::draw(ctx, gpu, frame_ms);
        });

        // Positions change continuously, so draw every frame; vsync paces the loop.
        self.window.request_redraw();
    }
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

        running.renderer.on_window_event(&running.window, &event);

        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => running.renderer.resize(size.width, size.height),
            WindowEvent::RedrawRequested => running.redraw(),
            _ => {}
        }
    }
}
