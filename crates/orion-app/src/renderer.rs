//! GPU surface, device and the per-frame render, including the egui overlay.

use std::sync::Arc;

use anyhow::Context as _;
use winit::event::WindowEvent;
use winit::event_loop::OwnedDisplayHandle;
use winit::window::Window;

/// Background colour behind the globe, in linear RGB.
const SPACE: wgpu::Color = wgpu::Color {
    r: 0.004,
    g: 0.005,
    b: 0.012,
    a: 1.0,
};

/// Facts about the graphics adapter in use, for the diagnostics panel.
pub struct GpuInfo {
    pub adapter: String,
    pub backend: String,
}

pub struct Renderer {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    gpu_info: GpuInfo,

    egui_ctx: egui::Context,
    egui_state: egui_winit::State,
    egui_renderer: egui_wgpu::Renderer,
}

impl Renderer {
    pub async fn new(window: Arc<Window>, display: OwnedDisplayHandle) -> anyhow::Result<Self> {
        let mut instance_desc =
            wgpu::InstanceDescriptor::new_with_display_handle(Box::new(display));
        // Metal, Vulkan and DirectX 12 only; there is deliberately no OpenGL fallback (ADR 0001).
        instance_desc.backends =
            wgpu::Backends::METAL | wgpu::Backends::VULKAN | wgpu::Backends::DX12;
        let instance = wgpu::Instance::new(instance_desc);

        let surface = instance
            .create_surface(window.clone())
            .context("creating the window surface")?;
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                compatible_surface: Some(&surface),
                ..Default::default()
            })
            .await
            .context("no graphics adapter supports Metal, Vulkan or DirectX 12")?;
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("orion"),
                ..Default::default()
            })
            .await
            .context("opening the graphics device")?;

        let size = window.inner_size();
        let mut config = surface
            .get_default_config(&adapter, size.width.max(1), size.height.max(1))
            .context("the graphics adapter cannot draw to this window")?;
        // egui blends in gamma space and wants a non-sRGB target.
        config.format =
            egui_wgpu::preferred_framebuffer_format(&surface.get_capabilities(&adapter).formats)
                .context("no supported surface format")?;
        surface.configure(&device, &config);

        let info = adapter.get_info();
        let gpu_info = GpuInfo {
            adapter: info.name,
            backend: format!("{:?}", info.backend),
        };
        log::info!("rendering on {} ({})", gpu_info.adapter, gpu_info.backend);

        let egui_ctx = egui::Context::default();
        let egui_state = egui_winit::State::new(
            egui_ctx.clone(),
            egui::ViewportId::ROOT,
            &window,
            Some(window.scale_factor() as f32),
            window.theme(),
            Some(device.limits().max_texture_dimension_2d as usize),
        );
        let egui_renderer = egui_wgpu::Renderer::new(
            &device,
            config.format,
            egui_wgpu::RendererOptions::default(),
        );

        Ok(Self {
            surface,
            device,
            queue,
            config,
            gpu_info,
            egui_ctx,
            egui_state,
            egui_renderer,
        })
    }

    /// Feeds a window event to the interface.
    pub fn on_window_event(&mut self, window: &Window, event: &WindowEvent) {
        let _ = self.egui_state.on_window_event(window, event);
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        // A minimised window reports a zero size, which a surface cannot be configured with.
        if width == 0 || height == 0 {
            return;
        }
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&self.device, &self.config);
    }

    /// Draws one frame: the scene, then the interface built by `build_ui` on top.
    pub fn render(&mut self, window: &Window, mut build_ui: impl FnMut(&egui::Context, &GpuInfo)) {
        let mut reconfigure_after_present = false;
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame) => frame,
            wgpu::CurrentSurfaceTexture::Suboptimal(frame) => {
                reconfigure_after_present = true;
                frame
            }
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => return,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.surface.configure(&self.device, &self.config);
                return;
            }
            wgpu::CurrentSurfaceTexture::Validation => {
                log::error!("the surface rejected the frame request; skipping this frame");
                return;
            }
        };
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        let raw_input = self.egui_state.take_egui_input(window);
        let gpu_info = &self.gpu_info;
        let output = self
            .egui_ctx
            .run_pass(raw_input, |ctx| build_ui(ctx, gpu_info));
        self.egui_state
            .handle_platform_output(window, output.platform_output);
        let paint_jobs = self
            .egui_ctx
            .tessellate(output.shapes, output.pixels_per_point);
        let mut textures_delta = output.textures_delta;
        let screen = egui_wgpu::ScreenDescriptor {
            size_in_pixels: [self.config.width, self.config.height],
            pixels_per_point: output.pixels_per_point,
        };

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("frame"),
            });

        // egui requires every texture change to be consumed, so drain the lists.
        for (id, deltas) in textures_delta.set.drain() {
            for delta in deltas {
                self.egui_renderer
                    .update_texture(&self.device, &self.queue, id, &delta);
            }
        }
        let egui_commands = self.egui_renderer.update_buffers(
            &self.device,
            &self.queue,
            &mut encoder,
            &paint_jobs,
            &screen,
        );

        {
            // The globe and Tracked Objects will be drawn in this pass, before the interface.
            let mut pass = encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("frame"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        resolve_target: None,
                        depth_slice: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(SPACE),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    ..Default::default()
                })
                .forget_lifetime();
            self.egui_renderer.render(&mut pass, &paint_jobs, &screen);
        }

        for id in textures_delta.free.drain() {
            self.egui_renderer.free_texture(&id);
        }

        self.queue
            .submit(egui_commands.into_iter().chain([encoder.finish()]));
        self.queue.present(frame);

        if reconfigure_after_present {
            self.surface.configure(&self.device, &self.config);
        }
    }
}
