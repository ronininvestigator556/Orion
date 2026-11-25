//! Earth: sphere mesh, day and night textures, and the pipeline that draws them.

use anyhow::Context as _;
use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};
use wgpu::util::DeviceExt as _;

/// Bands of latitude in the sphere mesh.
const RINGS: u32 = 96;
/// Slices of longitude in the sphere mesh.
const SEGMENTS: u32 = 192;

// NASA Blue Marble Next Generation (December) and Black Marble 2016, both public domain.
const DAY_IMAGE: &[u8] = include_bytes!("../assets/earth_day.jpg");
const NIGHT_IMAGE: &[u8] = include_bytes!("../assets/earth_night.jpg");

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Vertex {
    /// Point on the unit sphere in the Earth-fixed frame.
    position: [f32; 3],
    /// Texture coordinate in an equirectangular map: u runs east from longitude 180 west,
    /// v runs south from the North Pole.
    uv: [f32; 2],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Uniforms {
    view_projection: [[f32; 4]; 4],
    sun_direction: [f32; 3],
    _padding: f32,
}

pub struct Globe {
    pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    uniform_buffer: wgpu::Buffer,
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    index_count: u32,
}

impl Globe {
    /// `target_format` is the format of the view the globe is drawn into. It must be an
    /// sRGB format, because the shader outputs linear colour.
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        target_format: wgpu::TextureFormat,
    ) -> anyhow::Result<Self> {
        let day = load_texture(device, queue, "earth day", DAY_IMAGE)?;
        let night = load_texture(device, queue, "earth night", NIGHT_IMAGE)?;
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("earth"),
            // Longitude wraps around; latitude stops at the poles.
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            // Keeps the surface sharp where it is seen edge-on near the limb.
            anisotropy_clamp: 8,
            ..Default::default()
        });

        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("globe uniforms"),
            size: size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let texture_entry = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("globe"),
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
                texture_entry(1),
                texture_entry(2),
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("globe"),
            layout: &bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&day),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&night),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });

        let shader = device.create_shader_module(wgpu::include_wgsl!("globe.wgsl"));
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("globe"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("globe"),
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
                // Only the near hemisphere is ever visible, so skip triangles facing away.
                cull_mode: Some(wgpu::Face::Back),
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(target_format.into())],
            }),
            multiview_mask: None,
            cache: None,
        });

        let (vertices, indices) = sphere_mesh(RINGS, SEGMENTS);
        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("globe vertices"),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("globe indices"),
            contents: bytemuck::cast_slice(&indices),
            usage: wgpu::BufferUsages::INDEX,
        });

        Ok(Self {
            pipeline,
            bind_group,
            uniform_buffer,
            vertex_buffer,
            index_buffer,
            index_count: indices.len() as u32,
        })
    }

    /// Sets the camera transform and the Sun's direction (Earth-fixed frame) for the next draw.
    pub fn update(&self, queue: &wgpu::Queue, view_projection: Mat4, sun_direction: Vec3) {
        let uniforms = Uniforms {
            view_projection: view_projection.to_cols_array_2d(),
            sun_direction: sun_direction.to_array(),
            _padding: 0.0,
        };
        queue.write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&uniforms));
    }

    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint16);
        pass.draw_indexed(0..self.index_count, 0, 0..1);
    }
}

/// Builds a latitude/longitude sphere of radius 1 whose triangles wind counter-clockwise
/// when seen from outside.
///
/// The column at longitude 180 west is repeated at 180 east so the texture can wrap
/// without a seam, which is why there is one more column of vertices than segments.
fn sphere_mesh(rings: u32, segments: u32) -> (Vec<Vertex>, Vec<u16>) {
    let columns = segments + 1;
    assert!(
        (rings + 1) * columns <= u16::MAX as u32 + 1,
        "sphere mesh has too many vertices for 16-bit indices"
    );

    let mut vertices = Vec::with_capacity(((rings + 1) * columns) as usize);
    for ring in 0..=rings {
        let v = ring as f32 / rings as f32;
        let latitude = std::f32::consts::FRAC_PI_2 - v * std::f32::consts::PI;
        let (sin_lat, cos_lat) = latitude.sin_cos();
        for segment in 0..=segments {
            let u = segment as f32 / segments as f32;
            let longitude = (u - 0.5) * std::f32::consts::TAU;
            let (sin_lon, cos_lon) = longitude.sin_cos();
            vertices.push(Vertex {
                position: [cos_lat * cos_lon, cos_lat * sin_lon, sin_lat],
                uv: [u, v],
            });
        }
    }

    let mut indices = Vec::with_capacity((rings * segments * 6) as usize);
    for ring in 0..rings {
        for segment in 0..segments {
            // Seen from outside with north up: north-west, north-east, south-west, south-east.
            let nw = (ring * columns + segment) as u16;
            let ne = nw + 1;
            let sw = nw + columns as u16;
            let se = sw + 1;
            indices.extend_from_slice(&[nw, sw, se, nw, se, ne]);
        }
    }

    (vertices, indices)
}

/// Decodes an sRGB image and uploads it with a full chain of mipmaps, so the texture
/// stays smooth when the globe is small on screen.
fn load_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    label: &str,
    encoded: &[u8],
) -> anyhow::Result<wgpu::TextureView> {
    let mut level = image::load_from_memory(encoded)
        .with_context(|| format!("decoding the {label} image"))?
        .into_rgba8();

    let (width, height) = level.dimensions();
    let mip_level_count = width.max(height).ilog2() + 1;
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });

    for mip_level in 0..mip_level_count {
        if mip_level > 0 {
            let (w, h) = level.dimensions();
            level = image::imageops::resize(
                &level,
                (w / 2).max(1),
                (h / 2).max(1),
                image::imageops::FilterType::Triangle,
            );
        }
        let (w, h) = level.dimensions();
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &level,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4 * w),
                rows_per_image: Some(h),
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
    }

    Ok(texture.create_view(&wgpu::TextureViewDescriptor::default()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mesh_vertices_lie_on_the_unit_sphere() {
        let (vertices, _) = sphere_mesh(8, 16);
        for vertex in &vertices {
            assert!((Vec3::from_array(vertex.position).length() - 1.0).abs() < 1e-6);
        }
    }

    #[test]
    fn mesh_maps_texture_coordinates_to_geography() {
        let (vertices, _) = sphere_mesh(8, 16);
        let at = |u: f32, v: f32| {
            let vertex = vertices
                .iter()
                .find(|vertex| vertex.uv == [u, v])
                .expect("a vertex at that texture coordinate");
            Vec3::from_array(vertex.position)
        };
        // Centre of the map is latitude 0, longitude 0.
        assert!(at(0.5, 0.5).abs_diff_eq(Vec3::X, 1e-6));
        // Three quarters across is longitude 90 east.
        assert!(at(0.75, 0.5).abs_diff_eq(Vec3::Y, 1e-6));
        // Top edge is the North Pole.
        assert!(at(0.5, 0.0).abs_diff_eq(Vec3::Z, 1e-6));
    }

    #[test]
    fn mesh_triangles_face_outward() {
        let (vertices, indices) = sphere_mesh(8, 16);
        for triangle in indices.as_chunks::<3>().0 {
            let [a, b, c] =
                [0, 1, 2].map(|i| Vec3::from_array(vertices[triangle[i] as usize].position));
            let normal = (b - a).cross(c - a);
            // Triangles touching a pole collapse to a sliver with no area; skip those.
            if normal.length() > 1e-6 {
                assert!(normal.dot(a + b + c) > 0.0);
            }
        }
    }
}
