//! The star field behind the globe.

use anyhow::Context as _;
use bytemuck::{Pod, Zeroable};
use glam::Mat4;

/// The naked-eye stars of the Yale Bright Star Catalogue (5th revised edition, public
/// domain), trimmed to position, brightness and colour: right ascension and declination
/// in degrees for the year 2000, visual magnitude, and B-V colour index.
const CATALOGUE: &str = include_str!("../assets/stars.csv");

/// One star.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
struct Star {
    /// Unit vector towards the star in the inertial frame.
    direction: [f32; 3],
    /// Radius of its dot in logical points.
    radius: f32,
    /// Linear colour with alpha.
    color: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Uniforms {
    view_projection: [[f32; 4]; 4],
    viewport: [f32; 2],
    scale_factor: f32,
    sidereal_angle: f32,
}

/// Reads stars from CSV text with the columns `ra_deg,dec_deg,magnitude,b_v`.
fn parse_stars(text: &str) -> anyhow::Result<Vec<Star>> {
    text.lines()
        .skip(1)
        .enumerate()
        .map(|(row, line)| {
            let mut fields = line.split(',');
            let mut number = || -> Option<f32> { fields.next()?.parse().ok() };
            let (ra, dec, magnitude) = (number(), number(), number());
            let b_v = number();
            let (Some(ra), Some(dec), Some(magnitude)) = (ra, dec, magnitude) else {
                anyhow::bail!("star {} is not `ra,dec,magnitude,b_v`: {line:?}", row + 1);
            };
            Ok(star(ra, dec, magnitude, b_v))
        })
        .collect()
}

fn star(ra_deg: f32, dec_deg: f32, magnitude: f32, b_v: Option<f32>) -> Star {
    let (sin_ra, cos_ra) = ra_deg.to_radians().sin_cos();
    let (sin_dec, cos_dec) = dec_deg.to_radians().sin_cos();
    let [r, g, b] = color(b_v);
    Star {
        // Right ascension is measured east from the March equinox, the inertial +X axis.
        direction: [cos_dec * cos_ra, cos_dec * sin_ra, sin_dec],
        // Lower magnitudes are brighter. The faintest stars get the smallest dot that
        // still shows, and fade instead of shrinking further.
        radius: (2.6 - 0.33 * magnitude).clamp(0.6, 3.2),
        color: [r, g, b, (1.1 - 0.13 * magnitude).clamp(0.2, 1.0)],
    }
}

/// An approximate colour for a B-V colour index, from blue-white through white to
/// orange-red. Stars with no index are taken to be white.
fn color(b_v: Option<f32>) -> [f32; 3] {
    const STOPS: [(f32, [f32; 3]); 5] = [
        (-0.4, [0.55, 0.68, 1.0]),
        (0.0, [0.8, 0.87, 1.0]),
        (0.6, [1.0, 0.97, 0.9]),
        (1.2, [1.0, 0.78, 0.5]),
        (2.0, [1.0, 0.5, 0.25]),
    ];
    let b_v = b_v
        .unwrap_or(0.6)
        .clamp(STOPS[0].0, STOPS[STOPS.len() - 1].0);
    for pair in STOPS.windows(2) {
        let ((low, from), (high, to)) = (pair[0], pair[1]);
        if b_v <= high {
            let t = (b_v - low) / (high - low);
            return [0, 1, 2].map(|i| from[i] + (to[i] - from[i]) * t);
        }
    }
    STOPS[STOPS.len() - 1].1
}

pub struct Stars {
    pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    uniform_buffer: wgpu::Buffer,
    star_buffer: wgpu::Buffer,
    count: u32,
}

impl Stars {
    /// `target_format` must be an sRGB format, because the shader outputs linear colour.
    pub fn new(device: &wgpu::Device, target_format: wgpu::TextureFormat) -> anyhow::Result<Self> {
        use wgpu::util::DeviceExt as _;

        let stars = parse_stars(CATALOGUE).context("reading the bundled star catalogue")?;
        let star_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("stars"),
            contents: bytemuck::cast_slice(&stars),
            usage: wgpu::BufferUsages::VERTEX,
        });

        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("star uniforms"),
            size: size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("stars"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("stars"),
            layout: &bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });

        let shader = device.create_shader_module(wgpu::include_wgsl!("stars.wgsl"));
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("stars"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("stars"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                // One entry per star: the shader builds the dot's four corners itself.
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: size_of::<Star>() as u64,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &wgpu::vertex_attr_array![
                        0 => Float32x3,
                        1 => Float32,
                        2 => Float32x4
                    ],
                })],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleStrip,
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

        Ok(Self {
            pipeline,
            bind_group,
            uniform_buffer,
            star_buffer,
            count: stars.len() as u32,
        })
    }

    /// Sets the camera and how far Earth has turned for the next draw.
    pub fn update(
        &self,
        queue: &wgpu::Queue,
        view_projection: Mat4,
        viewport: [u32; 2],
        scale_factor: f32,
        sidereal_angle: f32,
    ) {
        let uniforms = Uniforms {
            view_projection: view_projection.to_cols_array_2d(),
            viewport: viewport.map(|pixels| pixels as f32),
            scale_factor,
            sidereal_angle,
        };
        queue.write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&uniforms));
    }

    /// Draw before the globe, which then covers the stars behind it.
    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.set_vertex_buffer(0, self.star_buffer.slice(..));
        pass.draw(0..4, 0..self.count);
    }
}

#[cfg(test)]
mod tests {
    use glam::Vec3;

    use super::*;

    #[test]
    fn the_bundled_catalogue_holds_the_naked_eye_stars() {
        let stars = parse_stars(CATALOGUE).expect("a readable catalogue");
        assert_eq!(stars.len(), 9_096);
        for star in &stars {
            assert!((Vec3::from_array(star.direction).length() - 1.0).abs() < 1e-5);
            assert!((0.6..=3.2).contains(&star.radius));
        }
    }

    #[test]
    fn stars_sit_where_their_coordinates_say() {
        // Sirius, the brightest star, heads the catalogue: 6h45m east of the equinox
        // and 16.7 degrees south of the celestial equator.
        let sirius = parse_stars(CATALOGUE).unwrap()[0];
        let direction = Vec3::from_array(sirius.direction);
        assert!((direction.z.asin().to_degrees() + 16.716).abs() < 0.01);
        assert!((direction.y.atan2(direction.x).to_degrees() - 101.287).abs() < 0.01);

        // Polaris is within a degree of the celestial north pole, the inertial +Z axis.
        let polaris = star(37.95, 89.264, 2.02, Some(0.6));
        assert!(polaris.direction[2] > 0.9999);
    }

    #[test]
    fn brighter_stars_are_drawn_larger_and_stronger() {
        let bright = star(0.0, 0.0, -1.46, None);
        let faint = star(0.0, 0.0, 6.5, None);
        assert!(bright.radius > faint.radius);
        assert!(bright.color[3] > faint.color[3]);
        assert_eq!(faint.radius, 0.6);
    }

    #[test]
    fn colour_runs_from_blue_to_red_with_the_colour_index() {
        let [blue_r, _, blue_b] = color(Some(-0.3));
        let [red_r, _, red_b] = color(Some(1.85));
        assert!(blue_b > blue_r);
        assert!(red_r > red_b);
        // Out-of-range and missing values are still colours.
        assert_eq!(color(Some(-5.0)), [0.55, 0.68, 1.0]);
        assert_eq!(color(Some(9.0)), [1.0, 0.5, 0.25]);
        assert_eq!(color(None), [1.0, 0.97, 0.9]);
    }

    #[test]
    fn a_damaged_catalogue_is_an_error() {
        assert!(parse_stars("ra_deg,dec_deg,magnitude,b_v\n1.0,2.0\n").is_err());
        assert!(parse_stars("ra_deg,dec_deg,magnitude,b_v\n1.0,2.0,3.0,\n").is_ok());
    }
}
