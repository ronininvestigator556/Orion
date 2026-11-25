//! The GPU device, the place frames are drawn to, and the per-frame render, including
//! the egui interface.

use std::sync::Arc;
use std::time::Instant;

use anyhow::Context as _;
use glam::{Mat4, Vec3};
use image::RgbaImage;
use orion_data::tiles::TileCache;
use winit::dpi::PhysicalSize;
use winit::event::WindowEvent;
use winit::event_loop::OwnedDisplayHandle;
use winit::window::Window;

use crate::basemap::{BaseMap, BaseMapChoices, BaseMapView, Picture};
use crate::globe::Globe;
use crate::layer::Routes;
use crate::lines::{Lines, Segment};
use crate::objects::{Instance, Objects, View};
use crate::reference::ReferenceLines;
use crate::stars::Stars;

/// Background colour behind the globe, in linear RGB.
const SPACE: wgpu::Color = wgpu::Color {
    r: 0.004,
    g: 0.005,
    b: 0.012,
    a: 1.0,
};

/// Width of a route, such as a pipeline, in logical points.
const ROUTE_WIDTH_POINTS: f32 = 1.6;

/// Facts about the graphics adapter in use, for the diagnostics panel.
pub struct GpuInfo {
    pub adapter: String,
    pub backend: String,
}

/// What to draw this frame, in the Earth-fixed frame.
pub struct Scene<'a> {
    pub view_projection: Mat4,
    /// Camera position in Earth radii.
    pub camera_position: Vec3,
    /// Unit vector from Earth's centre to the Sun.
    pub sun_direction: Vec3,
    /// One dot per Feature on show.
    pub objects: &'a [Instance],
    /// One mark per Cluster on show.
    pub clusters: &'a [Instance],
    /// The routes of each Layer that draws them.
    pub routes: &'a [Routes],
    /// The pictures laid over the Base Map, each with its Layer's position, in the
    /// order they are to be drawn.
    pub pictures: &'a [(usize, Picture)],
    /// Orbit Paths and Ground Tracks.
    pub lines: &'a [Segment],
    /// How far, in radians, the Earth-fixed frame is turned east of the stars.
    pub sidereal_angle: f32,
    /// What to show of the Base Map.
    pub base_map: BaseMapChoices,
}

/// Where frames are drawn.
// There is only ever one, so the two kinds differing in size wastes nothing.
#[expect(clippy::large_enum_variant)]
enum Target {
    /// A window on screen, which also supplies the mouse and keyboard.
    Window {
        window: Arc<Window>,
        surface: wgpu::Surface<'static>,
        config: wgpu::SurfaceConfiguration,
        input: egui_winit::State,
    },
    /// A picture held in memory, for saving to a file. Nothing needs to be on screen,
    /// so this works when the display is asleep or locked.
    Picture {
        texture: wgpu::Texture,
        started: Instant,
    },
}

pub struct Renderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    target: Target,
    /// sRGB twin of the format frames are stored in, which is not an sRGB format because
    /// egui blends in gamma space and wants its values stored as they are. The scene is
    /// drawn through a view in this format so the GPU encodes its linear colours for
    /// display.
    scene_format: wgpu::TextureFormat,
    size: PhysicalSize<u32>,
    /// Pixels per logical point.
    scale_factor: f64,
    gpu_info: GpuInfo,

    globe: Globe,
    base_map: BaseMap,
    /// A pyramid for each Layer that has laid a picture over the Base Map, with the
    /// Layer's position. Kept until the app closes.
    overlays: Vec<(usize, BaseMap)>,
    /// Which of those to draw this frame.
    overlays_on_show: Vec<usize>,
    tile_cache: Option<TileCache>,
    reference: ReferenceLines,
    stars: Stars,
    lines: Lines,
    /// The routes of each Layer that has drawn any, with the Layer's position and
    /// the generation of routes held on the GPU. Kept until the app closes.
    routes: Vec<(usize, u64, Lines)>,
    /// Which of those to draw this frame.
    routes_on_show: Vec<usize>,
    clusters: Objects,
    objects: Objects,

    egui_ctx: egui::Context,
    egui_renderer: egui_wgpu::Renderer,
}

/// Metal, Vulkan and DirectX 12 only; there is deliberately no OpenGL fallback (ADR 0001).
const BACKENDS: wgpu::Backends = wgpu::Backends::METAL
    .union(wgpu::Backends::VULKAN)
    .union(wgpu::Backends::DX12);

impl Renderer {
    /// Draws to a window. Base Map tiles are kept in `tile_cache` between runs when
    /// there is one.
    pub async fn new(
        window: Arc<Window>,
        display: OwnedDisplayHandle,
        tile_cache: Option<TileCache>,
    ) -> anyhow::Result<Self> {
        let mut instance_desc =
            wgpu::InstanceDescriptor::new_with_display_handle(Box::new(display));
        instance_desc.backends = BACKENDS;
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
        let (device, queue) = open_device(&adapter).await?;

        let size = window.inner_size();
        let mut config = surface
            .get_default_config(&adapter, size.width.max(1), size.height.max(1))
            .context("the graphics adapter cannot draw to this window")?;
        // egui wants a non-sRGB target, while the scene is lit in linear colour and
        // wants an sRGB one. One surface serves both by allowing a second view format.
        config.format =
            egui_wgpu::preferred_framebuffer_format(&surface.get_capabilities(&adapter).formats)
                .context("no supported surface format")?;
        let scene_format = config.format.add_srgb_suffix();
        config.view_formats.push(scene_format);
        surface.configure(&device, &config);

        let egui_ctx = egui::Context::default();
        let input = egui_winit::State::new(
            egui_ctx.clone(),
            egui::ViewportId::ROOT,
            &window,
            Some(window.scale_factor() as f32),
            window.theme(),
            Some(device.limits().max_texture_dimension_2d as usize),
        );
        let format = config.format;
        let scale_factor = window.scale_factor();
        let target = Target::Window {
            window,
            surface,
            config,
            input,
        };
        Self::assemble(
            device,
            queue,
            &adapter,
            target,
            format,
            PhysicalSize::new(size.width.max(1), size.height.max(1)),
            scale_factor,
            egui_ctx,
            tile_cache,
        )
    }

    /// Draws to a picture in memory of the given size in pixels, with no window at all.
    /// Read the result with [`Self::read_picture`].
    pub async fn offscreen(
        width: u32,
        height: u32,
        scale_factor: f64,
        tile_cache: Option<TileCache>,
    ) -> anyhow::Result<Self> {
        let mut instance_desc = wgpu::InstanceDescriptor::new_without_display_handle();
        instance_desc.backends = BACKENDS;
        let instance = wgpu::Instance::new(instance_desc);
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions::default())
            .await
            .context("no graphics adapter supports Metal, Vulkan or DirectX 12")?;
        let (device, queue) = open_device(&adapter).await?;

        let format = wgpu::TextureFormat::Rgba8Unorm;
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("picture"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[format.add_srgb_suffix()],
        });
        let egui_ctx = egui::Context::default();
        egui_ctx.set_pixels_per_point(scale_factor as f32);
        let target = Target::Picture {
            texture,
            started: Instant::now(),
        };
        Self::assemble(
            device,
            queue,
            &adapter,
            target,
            format,
            PhysicalSize::new(width, height),
            scale_factor,
            egui_ctx,
            tile_cache,
        )
    }

    #[expect(clippy::too_many_arguments)]
    fn assemble(
        device: wgpu::Device,
        queue: wgpu::Queue,
        adapter: &wgpu::Adapter,
        target: Target,
        format: wgpu::TextureFormat,
        size: PhysicalSize<u32>,
        scale_factor: f64,
        egui_ctx: egui::Context,
        tile_cache: Option<TileCache>,
    ) -> anyhow::Result<Self> {
        let scene_format = format.add_srgb_suffix();
        let info = adapter.get_info();
        let gpu_info = GpuInfo {
            adapter: info.name,
            backend: format!("{:?}", info.backend),
        };
        log::info!("rendering on {} ({})", gpu_info.adapter, gpu_info.backend);

        let globe = Globe::new(&device, &queue, scene_format)?;
        let base_map = BaseMap::new(&device, scene_format, tile_cache.clone());
        let reference = ReferenceLines::new(&device, &queue, scene_format)?;
        let stars = Stars::new(&device, scene_format)?;
        let lines = Lines::new(&device, scene_format);
        let clusters = Objects::new(&device, scene_format);
        let objects = Objects::new(&device, scene_format);
        let egui_renderer =
            egui_wgpu::Renderer::new(&device, format, egui_wgpu::RendererOptions::default());

        Ok(Self {
            device,
            queue,
            target,
            scene_format,
            size,
            scale_factor,
            gpu_info,
            globe,
            base_map,
            overlays: Vec::new(),
            overlays_on_show: Vec::new(),
            tile_cache,
            reference,
            stars,
            lines,
            routes: Vec::new(),
            routes_on_show: Vec::new(),
            clusters,
            objects,
            egui_ctx,
            egui_renderer,
        })
    }

    /// Forgets what is held for the Layer at this position and re-numbers what is held
    /// for the Layers after it, which have each moved up one. Call when a Layer is removed.
    pub fn layer_removed(&mut self, removed: usize) {
        self.routes.retain(|(layer, ..)| *layer != removed);
        for (layer, ..) in &mut self.routes {
            if *layer > removed {
                *layer -= 1;
            }
        }
        self.routes_on_show.clear();
        self.overlays.retain(|(layer, _)| *layer != removed);
        for (layer, _) in &mut self.overlays {
            if *layer > removed {
                *layer -= 1;
            }
        }
        self.overlays_on_show.clear();
    }

    /// Size of the drawing area in pixels.
    pub fn size(&self) -> PhysicalSize<u32> {
        self.size
    }

    /// Pixels per logical point.
    pub fn scale_factor(&self) -> f64 {
        self.scale_factor
    }

    /// Feeds a window event to the interface. Returns true if the interface used it,
    /// in which case the scene should ignore it.
    pub fn on_window_event(&mut self, event: &WindowEvent) -> bool {
        match &mut self.target {
            Target::Window { window, input, .. } => {
                if let WindowEvent::ScaleFactorChanged { scale_factor, .. } = event {
                    self.scale_factor = *scale_factor;
                }
                input.on_window_event(window, event).consumed
            }
            Target::Picture { .. } => false,
        }
    }

    /// True if the cursor is over a panel, so the scene beneath should not react to it.
    pub fn pointer_over_interface(&self) -> bool {
        self.egui_ctx.is_pointer_over_egui()
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        // A minimised window reports a zero size, which a surface cannot be configured with.
        if width == 0 || height == 0 {
            return;
        }
        if let Target::Window {
            surface, config, ..
        } = &mut self.target
        {
            config.width = width;
            config.height = height;
            surface.configure(&self.device, config);
            self.size = PhysicalSize::new(width, height);
        }
    }

    /// Draws one frame: the scene, then the interface built by `build_ui` on top.
    pub fn render(&mut self, scene: &Scene<'_>, mut build_ui: impl FnMut(&mut egui::Ui, &GpuInfo)) {
        let mut reconfigure_after_present = false;
        let (surface_frame, interface_view, scene_view) = match &mut self.target {
            Target::Window {
                surface, config, ..
            } => {
                let frame = match surface.get_current_texture() {
                    wgpu::CurrentSurfaceTexture::Success(frame) => frame,
                    wgpu::CurrentSurfaceTexture::Suboptimal(frame) => {
                        reconfigure_after_present = true;
                        frame
                    }
                    wgpu::CurrentSurfaceTexture::Timeout
                    | wgpu::CurrentSurfaceTexture::Occluded => return,
                    wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                        surface.configure(&self.device, config);
                        return;
                    }
                    wgpu::CurrentSurfaceTexture::Validation => {
                        log::error!("the surface rejected the frame request; skipping this frame");
                        return;
                    }
                };
                let (interface_view, scene_view) = views(&frame.texture, self.scene_format);
                (Some(frame), interface_view, scene_view)
            }
            Target::Picture { texture, .. } => {
                let (interface_view, scene_view) = views(texture, self.scene_format);
                (None, interface_view, scene_view)
            }
        };

        let viewport = [self.size.width, self.size.height];
        let scale_factor = self.scale_factor as f32;
        let view = View {
            view_projection: scene.view_projection,
            camera_position: scene.camera_position,
            viewport,
            scale_factor,
        };
        self.globe
            .update(&self.queue, scene.view_projection, scene.sun_direction);
        let mut base_map_view = BaseMapView {
            streaming: scene.base_map.imagery,
            view_projection: scene.view_projection,
            camera_position: scene.camera_position,
            sun_direction: scene.sun_direction,
            viewport,
        };
        self.base_map
            .update(&self.device, &self.queue, &base_map_view);
        // A Layer's picture is streamed whether or not the Base Map's own imagery is.
        base_map_view.streaming = true;
        self.overlays_on_show.clear();
        for (layer, picture) in scene.pictures {
            let slot = match self.overlays.iter().position(|(owner, _)| owner == layer) {
                Some(slot) => {
                    self.overlays[slot].1.set_picture(picture.clone());
                    slot
                }
                None => {
                    let pyramid = BaseMap::overlay(
                        &self.device,
                        self.scene_format,
                        self.tile_cache.clone(),
                        picture.clone(),
                    );
                    self.overlays.push((*layer, pyramid));
                    self.overlays.len() - 1
                }
            };
            self.overlays[slot]
                .1
                .update(&self.device, &self.queue, &base_map_view);
            self.overlays_on_show.push(slot);
        }
        self.reference.update(&self.queue, &view, scene.base_map);
        self.stars.update(
            &self.queue,
            scene.view_projection,
            viewport,
            scale_factor,
            scene.sidereal_angle,
        );
        self.lines
            .update(&self.device, &self.queue, &view, scene.lines);
        self.routes_on_show.clear();
        for routes in scene.routes {
            let slot = match self
                .routes
                .iter()
                .position(|(layer, ..)| *layer == routes.layer)
            {
                Some(slot) => slot,
                None => {
                    let lines =
                        Lines::with_width(&self.device, self.scene_format, ROUTE_WIDTH_POINTS);
                    // A generation no Layer starts at, so the first routes are always sent.
                    self.routes.push((routes.layer, u64::MAX, lines));
                    self.routes.len() - 1
                }
            };
            let (_, generation, lines) = &mut self.routes[slot];
            if *generation != routes.generation {
                *generation = routes.generation;
                lines.set_segments(&self.device, &self.queue, &routes.segments);
            }
            lines.set_view(&self.queue, &view);
            self.routes_on_show.push(slot);
        }
        self.clusters
            .update(&self.device, &self.queue, &view, scene.clusters);
        self.objects
            .update(&self.device, &self.queue, &view, scene.objects);

        let raw_input = match &mut self.target {
            Target::Window { window, input, .. } => input.take_egui_input(window),
            // With no window there is no mouse or keyboard: only the size and the time.
            Target::Picture { started, .. } => egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(viewport[0] as f32, viewport[1] as f32) / scale_factor,
                )),
                time: Some(started.elapsed().as_secs_f64()),
                ..Default::default()
            },
        };
        let gpu_info = &self.gpu_info;
        let output = self.egui_ctx.run_ui(raw_input, |ui| build_ui(ui, gpu_info));
        if let Target::Window { window, input, .. } = &mut self.target {
            input.handle_platform_output(window, output.platform_output);
        }
        let paint_jobs = self
            .egui_ctx
            .tessellate(output.shapes, output.pixels_per_point);
        let mut textures_delta = output.textures_delta;
        let screen = egui_wgpu::ScreenDescriptor {
            size_in_pixels: viewport,
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
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("scene"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &scene_view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(SPACE),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            // Farthest first: stars, then the globe that covers the ones behind it.
            self.stars.draw(&mut pass);
            self.globe.draw(&mut pass);
            // Sharper tiles over the bundled picture, wherever they have arrived.
            self.base_map.draw(&mut pass);
            for slot in &self.overlays_on_show {
                self.overlays[*slot].1.draw(&mut pass);
            }
            // Borders and roads lie on the ground, under everything that flies.
            self.reference.draw(&mut pass);
            for slot in &self.routes_on_show {
                self.routes[*slot].2.draw(&mut pass);
            }
            // Lines go under the dots so a dot on a line stays readable.
            self.lines.draw(&mut pass);
            // Clusters lie on the ground, so everything else is drawn over them.
            self.clusters.draw(&mut pass);
            self.objects.draw(&mut pass);
        }

        {
            let mut pass = encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("interface"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &interface_view,
                        resolve_target: None,
                        depth_slice: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Load,
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
        if let Some(frame) = surface_frame {
            self.queue.present(frame);
        }
        if let (
            true,
            Target::Window {
                surface, config, ..
            },
        ) = (reconfigure_after_present, &self.target)
        {
            surface.configure(&self.device, config);
        }
    }

    /// Copies the last frame drawn off the GPU. Only a renderer made with
    /// [`Self::offscreen`] has a picture to read.
    pub fn read_picture(&self) -> anyhow::Result<RgbaImage> {
        let Target::Picture { texture, .. } = &self.target else {
            anyhow::bail!("this renderer draws to a window, not to a picture");
        };
        let (width, height) = (self.size.width, self.size.height);
        // Rows in the copy must be padded to a multiple of 256 bytes.
        let row_bytes = width * 4;
        let padded_row_bytes = row_bytes.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
            * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("picture readback"),
            size: u64::from(padded_row_bytes) * u64::from(height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("picture readback"),
            });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded_row_bytes),
                    rows_per_image: Some(height),
                },
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit([encoder.finish()]);

        buffer.map_async(wgpu::MapMode::Read, .., |_| ());
        self.device
            .poll(wgpu::PollType::wait_indefinitely())
            .context("waiting for the picture to be copied off the GPU")?;
        let data = buffer
            .get_mapped_range(..)
            .context("reading the picture copied off the GPU")?;
        let mut pixels = Vec::with_capacity((row_bytes * height) as usize);
        for row in data.chunks(padded_row_bytes as usize) {
            pixels.extend_from_slice(&row[..row_bytes as usize]);
        }
        // The stored bytes are already display-ready sRGB: the scene was encoded on
        // the way in, and the interface is drawn in gamma space.
        RgbaImage::from_raw(width, height, pixels).context("the picture has an unexpected size")
    }
}

async fn open_device(adapter: &wgpu::Adapter) -> anyhow::Result<(wgpu::Device, wgpu::Queue)> {
    adapter
        .request_device(&wgpu::DeviceDescriptor {
            label: Some("orion"),
            ..Default::default()
        })
        .await
        .context("opening the graphics device")
}

/// Two views of the same texture: one as stored, for the interface, and one through
/// the sRGB format, for the scene.
fn views(
    texture: &wgpu::Texture,
    scene_format: wgpu::TextureFormat,
) -> (wgpu::TextureView, wgpu::TextureView) {
    (
        texture.create_view(&wgpu::TextureViewDescriptor::default()),
        texture.create_view(&wgpu::TextureViewDescriptor {
            format: Some(scene_format),
            ..Default::default()
        }),
    )
}
