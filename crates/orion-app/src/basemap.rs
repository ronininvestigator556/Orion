//! The streamed Base Map: imagery tiles drawn over the bundled picture of Earth, which
//! sharpen as the view moves closer.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};
use image::RgbaImage;
use orion_core::tiles::{self, TileKey};
use orion_data::tiles::{BLUE_MARBLE, LANDSAT, TileCache, TileLayer, TileSource, load_tile};
use wgpu::util::DeviceExt as _;

use crate::camera::FOV_Y;

/// The coarsest level drawn. Its tiles are 72 degrees across, so fifteen cover the world.
const MIN_LEVEL: u8 = 2;
/// The deepest level any picture is published at.
const MAX_LEVEL: u8 = LANDSAT.max_level;
/// Cells along each side of the patch of sphere a tile is drawn on.
const GRID: u16 = 16;
/// Tiles kept ready to draw. Each is about a megabyte of graphics memory.
const MAX_TILES_IN_MEMORY: usize = 160;
/// Tiles being fetched at once. Kept low out of courtesy to a free public service.
const MAX_IN_FLIGHT: usize = 6;
const WORKERS: usize = 4;
/// After this many failures in a row, fetching rests for a while: the connection or
/// the service is probably down, and asking again at once helps nobody.
const FAILURES_BEFORE_REST: u32 = 6;
const REST: Duration = Duration::from_secs(60);

/// Which picture a level's tiles come from: Blue Marble from far away, Landsat close up.
fn imagery_for(level: u8) -> &'static TileLayer {
    if level <= BLUE_MARBLE.max_level {
        &BLUE_MARBLE
    } else {
        &LANDSAT
    }
}

/// The angle at Earth's centre, in radians, between the point below the camera and the
/// edge of what the camera can see: the horizon from far away, the corner of the
/// window close to the ground. `distance` is from Earth's centre in Earth radii.
pub fn visible_radius(distance: f32, aspect: f32) -> f32 {
    let horizon = (1.0 / distance).clamp(-1.0, 1.0).acos();
    // Half the angle between opposite corners of the window, with room to spare for the
    // sidebar sliding the picture sideways.
    let corner = ((FOV_Y / 2.0).tan() * (1.0 + aspect * aspect).sqrt()).atan() * 1.3;
    let corner = corner.min(1.5);
    // Where a ray at that angle from straight down meets the sphere, if it does.
    let reach = distance * corner.sin();
    if reach >= 1.0 {
        horizon
    } else {
        (reach.asin() - corner).min(horizon)
    }
}

/// The level whose tiles suit the view: about one tile pixel per screen pixel
/// straight below the camera.
fn target_level(distance: f32, viewport_height: u32) -> u8 {
    let radians_per_pixel =
        (distance - 1.0).max(1e-6) * 2.0 * (FOV_Y / 2.0).tan() / viewport_height.max(1) as f32;
    tiles::level_for_resolution(
        f64::from(radians_per_pixel.to_degrees()),
        MIN_LEVEL,
        MAX_LEVEL,
    )
}

/// 0 seen from far away, rising to 1 within a few hundred kilometres of the ground.
fn closeness(distance: f32) -> f32 {
    let altitude = distance - 1.0;
    let t = ((0.6 - altitude) / (0.6 - 0.05)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// What the user has chosen to see of the Base Map.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BaseMapChoices {
    /// Sharper pictures streamed as the view comes closer. When off nothing is fetched.
    pub imagery: bool,
    pub borders: bool,
    pub roads: bool,
    pub names: bool,
}

impl Default for BaseMapChoices {
    fn default() -> Self {
        Self {
            imagery: true,
            borders: true,
            roads: true,
            names: true,
        }
    }
}

/// What the Base Map needs to know about the view each frame.
pub struct BaseMapView {
    /// False if the user has switched streamed pictures off: none are drawn or fetched.
    pub streaming: bool,
    pub view_projection: Mat4,
    /// Camera position in Earth radii in the Earth-fixed frame.
    pub camera_position: Vec3,
    /// Unit vector from Earth's centre to the Sun.
    pub sun_direction: Vec3,
    /// Size of the drawing area in pixels.
    pub viewport: [u32; 2],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct FrameUniforms {
    view_projection: [[f32; 4]; 4],
    sun_direction: [f32; 3],
    closeness: f32,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct TileUniforms {
    west: f32,
    north: f32,
    width: f32,
    height: f32,
    filled: [f32; 2],
    _padding: [f32; 2],
}

enum Tile {
    /// Asked for, not yet arrived.
    Loading,
    /// Could not be fetched or read. Not asked for again this run.
    Failed,
    Ready {
        bind_group: wgpu::BindGroup,
        /// The frame it was last drawn in, for deciding what to drop when memory is full.
        last_drawn: u64,
    },
}

/// Fetches and decodes tiles on background threads.
struct Loader {
    jobs: Arc<(Mutex<VecDeque<TileKey>>, Condvar)>,
    results: Receiver<(TileKey, Option<RgbaImage>)>,
}

impl Loader {
    fn start(cache: Option<TileCache>) -> Self {
        if let Some(cache) = cache.clone() {
            // Walking the cache folder can take a while, so it is done out of the way.
            std::thread::spawn(move || {
                let freed = cache.trim(TileCache::DEFAULT_LIMIT_BYTES);
                if freed > 0 {
                    log::info!("cleared {freed} bytes of old Base Map tiles");
                }
            });
        }
        let jobs = Arc::new((Mutex::new(VecDeque::<TileKey>::new()), Condvar::new()));
        let (sender, results) = mpsc::channel();
        let source = Arc::new(TileSource::new());
        for _ in 0..WORKERS {
            let jobs = Arc::clone(&jobs);
            let sender = sender.clone();
            let source = Arc::clone(&source);
            let cache = cache.clone();
            std::thread::spawn(move || {
                loop {
                    let key = {
                        let (queue, arrived) = &*jobs;
                        let mut queue = queue.lock().expect("the job queue is not poisoned");
                        loop {
                            match queue.pop_front() {
                                Some(key) => break key,
                                None => {
                                    queue =
                                        arrived.wait(queue).expect("the job queue is not poisoned");
                                }
                            }
                        }
                    };
                    let image = load_tile(&source, cache.as_ref(), imagery_for(key.level), key)
                        .map_err(|error| error.to_string())
                        .and_then(|bytes| {
                            image::load_from_memory(&bytes).map_err(|error| error.to_string())
                        })
                        .map(|image| image.into_rgba8())
                        .inspect_err(|error| log::debug!("Base Map tile {key:?}: {error}"))
                        .ok();
                    // The receiver is gone only when the app is shutting down.
                    if sender.send((key, image)).is_err() {
                        return;
                    }
                }
            });
        }
        Self { jobs, results }
    }

    fn request(&self, key: TileKey) {
        let (queue, arrived) = &*self.jobs;
        queue
            .lock()
            .expect("the job queue is not poisoned")
            .push_back(key);
        arrived.notify_one();
    }
}

pub struct BaseMap {
    pipeline: wgpu::RenderPipeline,
    frame_buffer: wgpu::Buffer,
    frame_bind_group: wgpu::BindGroup,
    tile_layout: wgpu::BindGroupLayout,
    grid_vertices: wgpu::Buffer,
    grid_indices: wgpu::Buffer,
    index_count: u32,

    loader: Loader,
    tiles: HashMap<TileKey, Tile>,
    in_flight: HashSet<TileKey>,
    /// The tiles to draw this frame, coarsest first so finer ones cover them.
    draw_list: Vec<TileKey>,
    frame: u64,
    failures_in_a_row: u32,
    resting_until: Option<Instant>,
}

impl BaseMap {
    /// `target_format` must be an sRGB format, because the shader outputs linear colour.
    /// Tiles are kept in `cache` between runs when there is one.
    pub fn new(
        device: &wgpu::Device,
        target_format: wgpu::TextureFormat,
        cache: Option<TileCache>,
    ) -> Self {
        let frame_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("base map frame"),
            size: size_of::<FrameUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("base map"),
            // A tile's own edge is the edge of its picture; never wrap round to the far side.
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let uniform_entry = |binding, visibility| wgpu::BindGroupLayoutEntry {
            binding,
            visibility,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let frame_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("base map frame"),
            entries: &[
                uniform_entry(0, wgpu::ShaderStages::VERTEX_FRAGMENT),
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let frame_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("base map frame"),
            layout: &frame_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: frame_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });
        let tile_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("base map tile"),
            entries: &[
                uniform_entry(0, wgpu::ShaderStages::VERTEX),
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });

        let shader = device.create_shader_module(wgpu::include_wgsl!("basemap.wgsl"));
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("base map"),
            bind_group_layouts: &[Some(&frame_layout), Some(&tile_layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("base map"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: size_of::<[f32; 2]>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x2],
                })],
            },
            primitive: wgpu::PrimitiveState {
                cull_mode: Some(wgpu::Face::Back),
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: target_format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });

        let (vertices, indices) = grid_mesh(GRID);
        let grid_vertices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("base map grid vertices"),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let grid_indices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("base map grid indices"),
            contents: bytemuck::cast_slice(&indices),
            usage: wgpu::BufferUsages::INDEX,
        });

        Self {
            pipeline,
            frame_buffer,
            frame_bind_group,
            tile_layout,
            grid_vertices,
            grid_indices,
            index_count: indices.len() as u32,
            loader: Loader::start(cache),
            tiles: HashMap::new(),
            in_flight: HashSet::new(),
            draw_list: Vec::new(),
            frame: 0,
            failures_in_a_row: 0,
            resting_until: None,
        }
    }

    /// Takes in tiles that have arrived, decides which to draw for this view and asks
    /// for the ones still missing.
    pub fn update(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, view: &BaseMapView) {
        self.frame += 1;
        self.receive(device, queue);
        if !view.streaming {
            self.draw_list.clear();
            return;
        }

        let distance = view.camera_position.length().max(1.000_1);
        let below = view.camera_position / distance;
        let latitude = f64::from(below.z.clamp(-1.0, 1.0).asin().to_degrees());
        let longitude = f64::from(below.y.atan2(below.x).to_degrees());
        let aspect = view.viewport[0] as f32 / view.viewport[1].max(1) as f32;
        let radius = f64::from(visible_radius(distance, aspect).to_degrees()) * 1.1;
        let target = target_level(distance, view.viewport[1]);

        self.draw_list.clear();
        let mut missing = Vec::new();
        for level in MIN_LEVEL..=target {
            // Only the coarsest level, as a backdrop, and the level that suits the view
            // are fetched. Levels between are drawn if an earlier view left them behind.
            let fetch = level == MIN_LEVEL || level == target;
            for key in tiles::tiles_covering(latitude, longitude, radius, level) {
                match self.tiles.get_mut(&key) {
                    Some(Tile::Ready { last_drawn, .. }) => {
                        *last_drawn = self.frame;
                        self.draw_list.push(key);
                    }
                    Some(Tile::Loading | Tile::Failed) => {}
                    None if fetch => missing.push(key),
                    None => {}
                }
            }
        }
        self.request(missing, latitude, longitude);
        self.forget_least_recently_drawn();

        let uniforms = FrameUniforms {
            view_projection: view.view_projection.to_cols_array_2d(),
            sun_direction: view.sun_direction.to_array(),
            closeness: closeness(distance),
        };
        queue.write_buffer(&self.frame_buffer, 0, bytemuck::bytes_of(&uniforms));
    }

    /// Turns tiles that have finished loading into textures ready to draw.
    fn receive(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) {
        while let Ok((key, image)) = self.loader.results.try_recv() {
            self.in_flight.remove(&key);
            let tile = match image {
                Some(image) => {
                    self.failures_in_a_row = 0;
                    Tile::Ready {
                        bind_group: self.upload(device, queue, key, &image),
                        last_drawn: self.frame,
                    }
                }
                None => {
                    self.failures_in_a_row += 1;
                    if self.failures_in_a_row >= FAILURES_BEFORE_REST {
                        log::warn!("Base Map tiles are failing to load; resting for a minute");
                        self.resting_until = Some(Instant::now() + REST);
                        self.failures_in_a_row = 0;
                    }
                    Tile::Failed
                }
            };
            self.tiles.insert(key, tile);
        }
    }

    fn upload(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        key: TileKey,
        image: &RgbaImage,
    ) -> wgpu::BindGroup {
        let (width, height) = image.dimensions();
        let texture = device.create_texture_with_data(
            queue,
            &wgpu::TextureDescriptor {
                label: Some("base map tile"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8UnormSrgb,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            },
            wgpu::util::TextureDataOrder::LayerMajor,
            image,
        );
        let bounds = key.bounds();
        let uniforms = TileUniforms {
            west: bounds.west.to_radians() as f32,
            north: bounds.north.to_radians() as f32,
            width: (bounds.east - bounds.west).to_radians() as f32,
            height: (bounds.north - bounds.south).to_radians() as f32,
            filled: bounds.filled.map(|fraction| fraction as f32),
            _padding: [0.0; 2],
        };
        let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("base map tile"),
            contents: bytemuck::bytes_of(&uniforms),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("base map tile"),
            layout: &self.tile_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(
                        &texture.create_view(&wgpu::TextureViewDescriptor::default()),
                    ),
                },
            ],
        })
    }

    /// Asks for missing tiles, nearest the middle of the view first, without having
    /// more than a few on the way at once.
    fn request(&mut self, mut missing: Vec<TileKey>, latitude: f64, longitude: f64) {
        if self
            .resting_until
            .is_some_and(|until| Instant::now() < until)
        {
            return;
        }
        self.resting_until = None;

        let distance_from_view = |key: &TileKey| {
            let bounds = key.bounds();
            let north_south = (bounds.north + bounds.south) / 2.0 - latitude;
            let east_west = ((bounds.west + bounds.east) / 2.0 - longitude + 540.0) % 360.0 - 180.0;
            // Coarser tiles first: they fill the view soonest.
            (key.level, (north_south.hypot(east_west) * 100.0) as i64)
        };
        missing.sort_by_key(distance_from_view);
        for key in missing {
            if self.in_flight.len() >= MAX_IN_FLIGHT {
                break;
            }
            self.in_flight.insert(key);
            self.tiles.insert(key, Tile::Loading);
            self.loader.request(key);
        }
    }

    /// Drops the tiles drawn longest ago once more are held than memory allows.
    fn forget_least_recently_drawn(&mut self) {
        let ready = |tile: &Tile| matches!(tile, Tile::Ready { .. });
        let held = self.tiles.values().filter(|tile| ready(tile)).count();
        if held <= MAX_TILES_IN_MEMORY {
            return;
        }
        let mut by_age: Vec<(u64, TileKey)> = self
            .tiles
            .iter()
            .filter_map(|(key, tile)| match tile {
                Tile::Ready { last_drawn, .. } if *last_drawn < self.frame => {
                    Some((*last_drawn, *key))
                }
                _ => None,
            })
            .collect();
        by_age.sort_unstable();
        for (_, key) in by_age.into_iter().take(held - MAX_TILES_IN_MEMORY) {
            self.tiles.remove(&key);
        }
    }

    /// Draw after the bundled picture of Earth, which shows wherever no tile has arrived.
    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        if self.draw_list.is_empty() {
            return;
        }
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.frame_bind_group, &[]);
        pass.set_vertex_buffer(0, self.grid_vertices.slice(..));
        pass.set_index_buffer(self.grid_indices.slice(..), wgpu::IndexFormat::Uint16);
        for key in &self.draw_list {
            if let Some(Tile::Ready { bind_group, .. }) = self.tiles.get(key) {
                pass.set_bind_group(1, bind_group, &[]);
                pass.draw_indexed(0..self.index_count, 0, 0..1);
            }
        }
    }
}

/// A square grid of `cells` by `cells`, as points from (0, 0) to (1, 1) with x running
/// east and y south, whose triangles wind counter-clockwise seen from outside the globe.
fn grid_mesh(cells: u16) -> (Vec<[f32; 2]>, Vec<u16>) {
    let points = cells + 1;
    let mut vertices = Vec::with_capacity(usize::from(points) * usize::from(points));
    for row in 0..points {
        for col in 0..points {
            vertices.push([
                f32::from(col) / f32::from(cells),
                f32::from(row) / f32::from(cells),
            ]);
        }
    }
    let mut indices = Vec::with_capacity(usize::from(cells) * usize::from(cells) * 6);
    for row in 0..cells {
        for col in 0..cells {
            let nw = row * points + col;
            let ne = nw + 1;
            let sw = nw + points;
            let se = sw + 1;
            indices.extend_from_slice(&[nw, sw, se, nw, se, ne]);
        }
    }
    (vertices, indices)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_far_away_the_camera_sees_to_the_horizon() {
        // From 3.2 radii the horizon is 71.8 degrees round the globe from straight below.
        let radius = visible_radius(3.2, 1.6).to_degrees();
        assert!((radius - 71.8).abs() < 0.2, "{radius}");
    }

    #[test]
    fn close_to_the_ground_the_window_limits_what_is_seen() {
        // 64 km up, the view is a patch well under a degree across, not the whole horizon.
        let radius = visible_radius(1.01, 1.6).to_degrees();
        assert!(radius > 0.2 && radius < 2.0, "{radius}");
        // And it shrinks steadily on the way down.
        assert!(visible_radius(1.005, 1.6) < visible_radius(1.01, 1.6));
        assert!(visible_radius(1.01, 1.6) < visible_radius(1.1, 1.6));
    }

    #[test]
    fn the_level_deepens_as_the_camera_comes_closer() {
        let far = target_level(3.2, 1600);
        assert_eq!(far, 3);
        let mut previous = far;
        for distance in [2.0, 1.5, 1.2, 1.05, 1.01, 1.004] {
            let level = target_level(distance, 1600);
            assert!(level >= previous, "level fell to {level} at {distance}");
            previous = level;
        }
        assert_eq!(previous, MAX_LEVEL);
        assert_eq!(target_level(30.0, 1600), MIN_LEVEL);
    }

    #[test]
    fn landsat_takes_over_where_blue_marble_runs_out() {
        assert_eq!(imagery_for(MIN_LEVEL).name, BLUE_MARBLE.name);
        assert_eq!(imagery_for(7).name, BLUE_MARBLE.name);
        assert_eq!(imagery_for(8).name, LANDSAT.name);
        assert_eq!(imagery_for(MAX_LEVEL).name, LANDSAT.name);
    }

    #[test]
    fn night_side_tiles_only_show_close_to_the_ground() {
        assert_eq!(closeness(3.2), 0.0);
        assert_eq!(closeness(1.01), 1.0);
        assert!(closeness(1.3) > 0.0 && closeness(1.3) < 1.0);
    }

    #[test]
    fn the_grid_has_two_triangles_per_cell() {
        let (vertices, indices) = grid_mesh(4);
        assert_eq!(vertices.len(), 25);
        assert_eq!(indices.len(), 4 * 4 * 6);
        assert_eq!(vertices[0], [0.0, 0.0]);
        assert_eq!(vertices[24], [1.0, 1.0]);
        assert!(
            indices
                .iter()
                .all(|index| usize::from(*index) < vertices.len())
        );
    }
}
