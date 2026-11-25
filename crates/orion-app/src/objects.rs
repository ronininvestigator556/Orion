//! Tracked Objects drawn as fixed-size dots.

use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};
use orion_core::catalogue::ObjectKind;

/// Radius of a dot in logical points; multiplied by the display's scale factor for pixels.
const DOT_RADIUS_POINTS: f32 = 1.5;

/// One dot. The order of instances is the caller's, so an instance's index can identify
/// the Tracked Object it stands for.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct Instance {
    /// Position in Earth radii in the Earth-fixed frame.
    position: [f32; 3],
    /// The kind in the low byte, with the `Marks` as bit flags above it.
    kind_and_marks: u32,
}

/// Ways a dot can be made to stand out, or fade.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Marks {
    pub selected: bool,
    pub hovered: bool,
    /// The Element Set is too old for the Position to be trusted.
    pub stale: bool,
}

// These must match the constants in the shader.
const SELECTED: u32 = 1 << 8;
const HOVERED: u32 = 1 << 9;
const STALE: u32 = 1 << 10;

impl Instance {
    /// An instance that draws nothing. It sits at Earth's centre, where the shader's
    /// "is Earth in the way" test always hides it.
    pub const HIDDEN: Self = Self {
        position: [0.0; 3],
        kind_and_marks: u32::MAX,
    };

    pub fn new(position: Vec3, kind: ObjectKind, marks: Marks) -> Self {
        // Must match the order of the colour table in the shader.
        let kind = match kind {
            ObjectKind::Satellite => 0,
            ObjectKind::RocketBody => 1,
            ObjectKind::Debris => 2,
        };
        let flag = |set: bool, bit: u32| if set { bit } else { 0 };
        Self {
            position: position.to_array(),
            kind_and_marks: kind
                | flag(marks.selected, SELECTED)
                | flag(marks.hovered, HOVERED)
                | flag(marks.stale, STALE),
        }
    }

    /// Position in Earth radii in the Earth-fixed frame.
    pub fn position(&self) -> Vec3 {
        Vec3::from_array(self.position)
    }

    pub fn is_hidden(&self) -> bool {
        *self == Self::HIDDEN
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Uniforms {
    view_projection: [[f32; 4]; 4],
    camera_position: [f32; 3],
    dot_radius: f32,
    viewport: [f32; 2],
    _padding: [f32; 2],
}

/// The camera and screen facts the dots need each frame.
pub struct View {
    pub view_projection: Mat4,
    pub camera_position: Vec3,
    /// Size of the drawing area in pixels.
    pub viewport: [u32; 2],
    /// Pixels per logical point.
    pub scale_factor: f32,
}

pub struct Objects {
    pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    uniform_buffer: wgpu::Buffer,
    /// Holds `capacity` instances; replaced by a larger one when more are needed.
    instance_buffer: Option<wgpu::Buffer>,
    capacity: usize,
    count: u32,
}

impl Objects {
    /// `target_format` must be an sRGB format, because the shader outputs linear colour.
    pub fn new(device: &wgpu::Device, target_format: wgpu::TextureFormat) -> Self {
        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("object uniforms"),
            size: size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("objects"),
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
            label: Some("objects"),
            layout: &bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });

        let shader = device.create_shader_module(wgpu::include_wgsl!("objects.wgsl"));
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("objects"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("objects"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                // One entry per dot, not per corner: the shader builds the four corners itself.
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: size_of::<Instance>() as u64,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Uint32],
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

        Self {
            pipeline,
            bind_group,
            uniform_buffer,
            instance_buffer: None,
            capacity: 0,
            count: 0,
        }
    }

    /// Sets the view and the dots for the next draw.
    pub fn update(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        view: &View,
        instances: &[Instance],
    ) {
        let uniforms = Uniforms {
            view_projection: view.view_projection.to_cols_array_2d(),
            camera_position: view.camera_position.to_array(),
            dot_radius: DOT_RADIUS_POINTS * view.scale_factor,
            viewport: view.viewport.map(|pixels| pixels as f32),
            _padding: [0.0; 2],
        };
        queue.write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&uniforms));

        if instances.len() > self.capacity {
            self.capacity = instances.len();
            self.instance_buffer = Some(device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("object instances"),
                size: (self.capacity * size_of::<Instance>()) as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }));
        }
        if let Some(buffer) = &self.instance_buffer {
            queue.write_buffer(buffer, 0, bytemuck::cast_slice(instances));
        }
        self.count = instances.len() as u32;
    }

    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        let Some(buffer) = &self.instance_buffer else {
            return;
        };
        if self.count == 0 {
            return;
        }
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        let used_bytes = self.count as u64 * size_of::<Instance>() as u64;
        pass.set_vertex_buffer(0, buffer.slice(..used_bytes));
        pass.draw(0..4, 0..self.count);
    }
}
