//! The street map: ready-drawn tiles from OpenStreetMap laid over the globe when the
//! view is close (ADR 0006).
//!
//! Tiles are fetched only for what is in view, at the level that suits the height,
//! and never ahead of the view, as the host's policy asks. Until a tile arrives, the
//! nearest coarser one already on hand is drawn in its place.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant, SystemTime};

use bytemuck::{Pod, Zeroable};
use glam::{DMat4, DVec3, Mat4, Vec3};
use image::RgbaImage;
use orion_core::street_tiles;
use orion_core::tiles::TileKey;
use orion_data::streets::{MAX_LEVEL, StreetCache, StreetTiles, load_street_tile};
use wgpu::util::DeviceExt;

use crate::basemap::{grid_mesh, visible_radius};
use crate::camera::FOV_Y;

/// How finely each tile is laid on the globe: the cells along each side.
const CELLS: u16 = 16;
/// The most tiles kept ready to draw. A view takes a few dozen.
const MAX_TILES_IN_MEMORY: usize = 220;
/// The most tiles a view may ask for. A view that would need more is drawn a level
/// coarser.
const MAX_TILES_IN_VIEW: usize = 90;
/// The host asks apps not to open many connections at once.
const WORKERS: usize = 2;
const MAX_IN_FLIGHT: usize = 4;
/// How long a tile that could not be fetched is left alone before it is tried again.
const RETRY: Duration = Duration::from_secs(60);
const EARTH_RADIUS_KM: f32 = 6_371.0;
/// With the choice left to Orion, the street map starts to show below this height and
/// has taken over by the next, where the imagery has no more detail to give.
const FADE_IN_FROM_KM: f32 = 60.0;
const FULL_BELOW_KM: f32 = 35.0;

/// What the user has chosen to see close to the ground.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum StreetChoice {
    /// Imagery from far away, the street map close up.
    #[default]
    Automatic,
    /// Imagery at every height. No street tile is fetched.
    Imagery,
    /// The street map at every height.
    Streets,
}

impl StreetChoice {
    pub const ALL: [Self; 3] = [Self::Automatic, Self::Imagery, Self::Streets];

    pub fn label(self) -> &'static str {
        match self {
            Self::Automatic => "Automatic",
            Self::Imagery => "Imagery",
            Self::Streets => "Streets",
        }
    }

    /// The word the choice is kept under between runs.
    pub fn setting(self) -> &'static str {
        match self {
            Self::Automatic => "automatic",
            Self::Imagery => "imagery",
            Self::Streets => "streets",
        }
    }

    pub fn from_setting(word: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|choice| choice.setting() == word.trim())
    }

    /// How much of what lies beneath the street map covers, from 0 to 1, with the
    /// camera `distance` Earth radii from Earth's centre.
    pub fn opacity(self, distance: f32) -> f32 {
        match self {
            Self::Imagery => 0.0,
            Self::Streets => 1.0,
            Self::Automatic => {
                let altitude_km = (distance - 1.0) * EARTH_RADIUS_KM;
                let t = ((FADE_IN_FROM_KM - altitude_km) / (FADE_IN_FROM_KM - FULL_BELOW_KM))
                    .clamp(0.0, 1.0);
                t * t * (3.0 - 2.0 * t)
            }
        }
    }
}

/// What the street map needs to know about the view each frame.
pub struct StreetMapView {
    pub choice: StreetChoice,
    pub view_projection: Mat4,
    /// Camera position in Earth radii in the Earth-fixed frame.
    pub camera_position: Vec3,
    /// Size of the drawing area in pixels.
    pub viewport: [u32; 2],
    /// How many pixels make one point of the screen's own measure: 2 on a sharp
    /// display.
    pub pixels_per_point: f32,
}

/// The level whose tiles suit the view. A tile's lettering is drawn for one tile
/// pixel to a point of the screen, so that is what is aimed for: on a sharp display
/// the map is a little soft, and its street names stay large enough to read. The
/// nearest level is taken, not the next coarser, so names appear a little sooner and
/// are never drawn at more than about one and a half times their size.
fn target_level(
    distance: f32,
    viewport_height: u32,
    pixels_per_point: f32,
    latitude_deg: f64,
) -> u8 {
    let points = viewport_height.max(1) as f32 / pixels_per_point.max(0.1);
    let radians_per_point = (distance - 1.0).max(1e-7) * 2.0 * (FOV_Y / 2.0).tan() / points;
    street_tiles::level_for_resolution(
        f64::from(radians_per_point) / std::f64::consts::SQRT_2,
        latitude_deg,
        MAX_LEVEL,
    )
}

/// The tiles to draw for a view, coarsest first: each tile wanted if it is ready, and
/// otherwise the nearest coarser tile that is, so the map is never left with a hole
/// where something could be shown.
fn draw_order(wanted: &[TileKey], ready: &HashSet<TileKey>) -> Vec<TileKey> {
    let mut order: Vec<TileKey> = Vec::with_capacity(wanted.len());
    for key in wanted {
        let mut candidate = Some(*key);
        while let Some(tile) = candidate {
            if ready.contains(&tile) {
                if !order.contains(&tile) {
                    order.push(tile);
                }
                break;
            }
            candidate = tile.parent();
        }
    }
    order.sort_by_key(|tile| tile.level);
    order
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Vertex {
    from_middle: [f32; 3],
    uv: [f32; 2],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct TileUniforms {
    to_clip: [[f32; 4]; 4],
    opacity: [f32; 4],
}

enum Tile {
    /// Asked for, not yet arrived.
    Loading,
    /// Could not be fetched or read, at this moment.
    Failed(Instant),
    Ready {
        vertices: wgpu::Buffer,
        uniforms: wgpu::Buffer,
        bind_group: wgpu::BindGroup,
        /// The tile's own middle, from Earth's centre, in Earth radii.
        middle: DVec3,
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
    fn start(source: StreetTiles, cache: Option<StreetCache>) -> Self {
        let jobs = Arc::new((Mutex::new(VecDeque::new()), Condvar::new()));
        let (sender, results) = mpsc::channel();
        let source = Arc::new(source);
        for _ in 0..WORKERS {
            let jobs: Arc<(Mutex<VecDeque<TileKey>>, Condvar)> = Arc::clone(&jobs);
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
                                Some(job) => break job,
                                None => {
                                    queue =
                                        arrived.wait(queue).expect("the job queue is not poisoned");
                                }
                            }
                        }
                    };
                    let image = load_street_tile(&source, cache.as_ref(), key, SystemTime::now())
                        .map_err(|error| error.to_string())
                        .and_then(|bytes| {
                            image::load_from_memory(&bytes).map_err(|error| error.to_string())
                        })
                        .map(|image| image.into_rgba8())
                        .inspect_err(|error| log::debug!("street map tile {key:?}: {error}"))
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

    /// Drops the tiles asked for and not yet started on. Returns them.
    fn forget_waiting(&self) -> Vec<TileKey> {
        let (queue, _) = &*self.jobs;
        queue
            .lock()
            .expect("the job queue is not poisoned")
            .drain(..)
            .collect()
    }
}

pub struct StreetMap {
    pipeline: wgpu::RenderPipeline,
    sampler_bind_group: wgpu::BindGroup,
    tile_layout: wgpu::BindGroupLayout,
    indices: wgpu::Buffer,
    index_count: u32,
    /// Fetches tiles, once it is known where from.
    loader: Option<Loader>,
    tiles: HashMap<TileKey, Tile>,
    in_flight: HashSet<TileKey>,
    /// The tiles to draw this frame, coarsest first.
    draw_list: Vec<TileKey>,
    frame: u64,
    opacity: f32,
    warned: bool,
}

impl StreetMap {
    /// `target_format` must be an sRGB format, because the shader outputs linear colour.
    pub fn new(device: &wgpu::Device, target_format: wgpu::TextureFormat) -> Self {
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("street map"),
            // A tile's own edge is the edge of its picture.
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let sampler_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("street map sampler"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            }],
        });
        let sampler_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("street map sampler"),
            layout: &sampler_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Sampler(&sampler),
            }],
        });
        let tile_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("street map tile"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
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

        let shader = device.create_shader_module(wgpu::include_wgsl!("streetmap.wgsl"));
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("street map"),
            bind_group_layouts: &[Some(&sampler_layout), Some(&tile_layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("street map"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: size_of::<Vertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x2],
                })],
            },
            primitive: wgpu::PrimitiveState {
                // The far side of the globe faces away and is not drawn.
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

        let (_, grid_indices) = grid_mesh(CELLS);
        let indices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("street map grid indices"),
            contents: bytemuck::cast_slice(&grid_indices),
            usage: wgpu::BufferUsages::INDEX,
        });

        Self {
            pipeline,
            sampler_bind_group,
            tile_layout,
            indices,
            index_count: grid_indices.len() as u32,
            loader: None,
            tiles: HashMap::new(),
            in_flight: HashSet::new(),
            draw_list: Vec::new(),
            frame: 0,
            opacity: 0.0,
            warned: false,
        }
    }

    /// Says where tiles come from and where they are kept. Until this is called the
    /// street map shows nothing.
    pub fn set_source(&mut self, source: StreetTiles, cache: Option<StreetCache>) {
        self.loader = Some(Loader::start(source, cache));
        self.tiles.clear();
        self.in_flight.clear();
        self.draw_list.clear();
    }

    /// True while the street map covers most of what lies beneath it.
    pub fn is_showing(&self) -> bool {
        self.opacity > 0.5 && !self.draw_list.is_empty()
    }

    /// Works out which tiles the view needs, asks for the missing ones and takes in
    /// those that have arrived.
    pub fn update(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, view: &StreetMapView) {
        self.frame += 1;
        self.draw_list.clear();
        let distance = view.camera_position.length();
        self.opacity = view.choice.opacity(distance);
        if self.loader.is_none() {
            return;
        }
        self.receive(device, queue);
        if self.opacity <= 0.0 {
            // Nothing is fetched for a map nobody is looking at.
            let Some(loader) = &self.loader else { return };
            for key in loader.forget_waiting() {
                self.in_flight.remove(&key);
                self.tiles.remove(&key);
            }
            return;
        }

        let below = view.camera_position / distance;
        let latitude = f64::from(below.z.clamp(-1.0, 1.0).asin().to_degrees());
        let longitude = f64::from(below.y.atan2(below.x).to_degrees());
        let aspect = view.viewport[0] as f32 / view.viewport[1].max(1) as f32;
        let radius = f64::from(visible_radius(distance, aspect).to_degrees());
        let mut level = target_level(distance, view.viewport[1], view.pixels_per_point, latitude);
        let mut wanted = street_tiles::tiles_covering(latitude, longitude, radius, level);
        while wanted.len() > MAX_TILES_IN_VIEW && level > 0 {
            level -= 1;
            wanted = street_tiles::tiles_covering(latitude, longitude, radius, level);
        }

        // Ask for what is missing, nearest the middle of the view first.
        let middle = street_tiles::containing(level, latitude, longitude);
        let now = Instant::now();
        let mut missing: Vec<TileKey> = wanted
            .iter()
            .copied()
            .filter(|key| match self.tiles.get(key) {
                None => true,
                Some(Tile::Failed(at)) => now.duration_since(*at) >= RETRY,
                Some(_) => false,
            })
            .collect();
        missing.sort_by_key(|key| key.row.abs_diff(middle.row) + key.col.abs_diff(middle.col));
        if let Some(loader) = &self.loader {
            // What was asked for and is no longer in view is not worth fetching.
            for key in loader.forget_waiting() {
                if wanted.contains(&key) {
                    loader.request(key);
                } else {
                    self.in_flight.remove(&key);
                    self.tiles.remove(&key);
                }
            }
            for key in missing {
                if self.in_flight.len() >= MAX_IN_FLIGHT {
                    break;
                }
                self.in_flight.insert(key);
                self.tiles.insert(key, Tile::Loading);
                loader.request(key);
            }
        }

        let ready: HashSet<TileKey> = self
            .tiles
            .iter()
            .filter(|(_, tile)| matches!(tile, Tile::Ready { .. }))
            .map(|(key, _)| *key)
            .collect();
        self.draw_list = draw_order(&wanted, &ready);

        // Each tile's own way to the screen, worked out in double precision.
        let view_projection = view.view_projection.as_dmat4();
        for key in &self.draw_list {
            if let Some(Tile::Ready {
                uniforms,
                middle,
                last_drawn,
                ..
            }) = self.tiles.get_mut(key)
            {
                *last_drawn = self.frame;
                let to_clip = (view_projection * DMat4::from_translation(*middle)).as_mat4();
                let values = TileUniforms {
                    to_clip: to_clip.to_cols_array_2d(),
                    opacity: [self.opacity, 0.0, 0.0, 0.0],
                };
                queue.write_buffer(uniforms, 0, bytemuck::bytes_of(&values));
            }
        }
        self.forget_least_recently_drawn();
    }

    /// Takes in the tiles the loader has finished with.
    fn receive(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) {
        let Some(loader) = &self.loader else { return };
        let arrived: Vec<(TileKey, Option<RgbaImage>)> = loader.results.try_iter().collect();
        for (key, image) in arrived {
            self.in_flight.remove(&key);
            let tile = match image {
                Some(image) => self.upload(device, queue, key, &image),
                None => {
                    if !self.warned {
                        self.warned = true;
                        log::warn!("a street map tile could not be loaded; it will be tried again");
                    }
                    Tile::Failed(Instant::now())
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
    ) -> Tile {
        let (width, height) = image.dimensions();
        let texture = device.create_texture_with_data(
            queue,
            &wgpu::TextureDescriptor {
                label: Some("street map tile"),
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

        let (middle, points) = patch_from_middle(key);
        let vertices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("street map tile vertices"),
            contents: bytemuck::cast_slice(&points),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("street map tile"),
            size: size_of::<TileUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("street map tile"),
            layout: &self.tile_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniforms.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(
                        &texture.create_view(&wgpu::TextureViewDescriptor::default()),
                    ),
                },
            ],
        });
        Tile::Ready {
            vertices,
            uniforms,
            bind_group,
            middle,
            last_drawn: self.frame,
        }
    }

    /// Drops the tiles drawn longest ago once more are held than memory allows.
    fn forget_least_recently_drawn(&mut self) {
        if self.tiles.len() <= MAX_TILES_IN_MEMORY {
            return;
        }
        let mut drawn: Vec<(u64, TileKey)> = self
            .tiles
            .iter()
            .filter_map(|(key, tile)| match tile {
                Tile::Ready { last_drawn, .. } if *last_drawn != self.frame => {
                    Some((*last_drawn, *key))
                }
                _ => None,
            })
            .collect();
        drawn.sort();
        let excess = self.tiles.len() - MAX_TILES_IN_MEMORY;
        for (_, key) in drawn.into_iter().take(excess) {
            self.tiles.remove(&key);
        }
    }

    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        if self.draw_list.is_empty() || self.opacity <= 0.0 {
            return;
        }
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.sampler_bind_group, &[]);
        pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint16);
        for key in &self.draw_list {
            if let Some(Tile::Ready {
                vertices,
                bind_group,
                ..
            }) = self.tiles.get(key)
            {
                pass.set_vertex_buffer(0, vertices.slice(..));
                pass.set_bind_group(1, bind_group, &[]);
                pass.draw_indexed(0..self.index_count, 0, 0..1);
            }
        }
    }
}

/// A tile's middle on the globe, and each of its points as measured from there.
fn patch_from_middle(key: TileKey) -> (DVec3, Vec<Vertex>) {
    let points = street_tiles::patch(key, CELLS);
    let middle = DVec3::from_array(points[points.len() / 2].0);
    let vertices = points
        .into_iter()
        .map(|(position, uv)| Vertex {
            from_middle: (DVec3::from_array(position) - middle).as_vec3().to_array(),
            uv,
        })
        .collect();
    (middle, vertices)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(level: u8, col: u32, row: u32) -> TileKey {
        TileKey { level, row, col }
    }

    /// Distance from Earth's centre, in Earth radii, of a camera `km` above the ground.
    fn above(km: f32) -> f32 {
        1.0 + km / EARTH_RADIUS_KM
    }

    #[test]
    fn left_to_orion_the_street_map_takes_over_close_to_the_ground() {
        let automatic = StreetChoice::Automatic;
        assert_eq!(automatic.opacity(above(2_000.0)), 0.0);
        assert!(automatic.opacity(above(61.0)) < 1e-6);
        let part = automatic.opacity(above(45.0));
        assert!(part > 0.1 && part < 0.9, "{part}");
        assert!(automatic.opacity(above(34.0)) > 0.999);
        assert_eq!(automatic.opacity(above(2.0)), 1.0);
        // The other two choices hold whatever the height.
        for km in [20_000.0, 500.0, 2.0] {
            assert_eq!(StreetChoice::Imagery.opacity(above(km)), 0.0);
            assert_eq!(StreetChoice::Streets.opacity(above(km)), 1.0);
        }
    }

    #[test]
    fn the_choice_is_kept_between_runs_by_a_plain_word() {
        for choice in StreetChoice::ALL {
            assert_eq!(StreetChoice::from_setting(choice.setting()), Some(choice));
        }
        assert_eq!(
            StreetChoice::from_setting(" streets "),
            Some(StreetChoice::Streets)
        );
        assert_eq!(StreetChoice::from_setting("satellite"), None);
        assert_eq!(StreetChoice::default(), StreetChoice::Automatic);
    }

    #[test]
    fn the_level_deepens_as_the_camera_comes_down_to_street_names() {
        let level = |km: f32| target_level(above(km), 1_600, 2.0, 48.86);
        // From orbit the whole world fits in a handful of tiles.
        assert!(level(8_000.0) <= 4, "{}", level(8_000.0));
        let levels: Vec<u8> = [2_000.0, 200.0, 20.0, 2.0].map(level).to_vec();
        assert!(
            levels.windows(2).all(|pair| pair[0] < pair[1]),
            "{levels:?}"
        );
        // Two kilometres up is where street names are drawn.
        assert!((16..=18).contains(&level(2.0)), "{}", level(2.0));
        // A sharp display is not given finer tiles, which would shrink the lettering.
        assert_eq!(
            target_level(above(20.0), 1_600, 2.0, 0.0),
            target_level(above(20.0), 800, 1.0, 0.0)
        );
        assert_eq!(target_level(above(0.001), 1_600, 2.0, 0.0), MAX_LEVEL);
    }

    #[test]
    fn a_tile_not_yet_arrived_is_stood_in_for_by_the_nearest_coarser_one() {
        let wanted = [
            key(14, 8299, 5636),
            key(14, 8300, 5636),
            key(14, 8299, 5637),
        ];
        let parent = key(13, 4149, 2818);
        let grandparent = key(12, 2074, 1409);
        assert_eq!(wanted[0].parent(), Some(parent));

        // Nothing ready: nothing to draw.
        assert_eq!(draw_order(&wanted, &HashSet::new()), Vec::new());
        // Only a grandparent: it stands in for all three, once.
        let ready = HashSet::from([grandparent]);
        assert_eq!(draw_order(&wanted, &ready), [grandparent]);
        // One tile has arrived: it is drawn over the stand-in, coarsest first.
        let ready = HashSet::from([grandparent, wanted[1]]);
        assert_eq!(draw_order(&wanted, &ready), [grandparent, wanted[1]]);
        // All three ready: the stand-ins are not drawn at all.
        let ready = HashSet::from([grandparent, parent, wanted[0], wanted[1], wanted[2]]);
        assert_eq!(draw_order(&wanted, &ready), wanted);
    }

    #[test]
    fn a_tiles_points_are_measured_from_its_own_middle_so_they_stay_exact() {
        let (middle, vertices) = patch_from_middle(key(17, 66_396, 45_090));
        assert_eq!(vertices.len(), 17 * 17);
        assert!((middle.length() - 1.0).abs() < 1e-12);
        // A tile this deep is about 200 metres across: every point is within a
        // twenty-thousandth of an Earth radius of the middle, where single precision
        // holds a position to well under a millimetre.
        let farthest = vertices
            .iter()
            .map(|vertex| Vec3::from_array(vertex.from_middle).length())
            .fold(0.0_f32, f32::max);
        assert!(farthest > 0.0 && farthest < 5e-5, "{farthest}");
        assert_eq!(vertices[vertices.len() / 2].from_middle, [0.0; 3]);
        assert_eq!(vertices[0].uv, [0.0, 0.0]);
        assert_eq!(vertices[vertices.len() - 1].uv, [1.0, 1.0]);
    }
}
