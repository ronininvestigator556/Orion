//! Lines of a fixed width on screen, used for the Orbit Path, the Ground Track and the
//! borders and roads over the Base Map.

use std::ops::Range;

use bytemuck::{Pod, Zeroable};
use glam::Vec3;

use crate::objects::View;

/// Usual width of a line in logical points; multiplied by the display's scale factor
/// for pixels.
const WIDTH_POINTS: f32 = 1.5;

/// One straight piece of a line.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct Segment {
    /// End points in Earth radii in the Earth-fixed frame.
    start: [f32; 3],
    end: [f32; 3],
    /// Linear colour with alpha.
    color: [f32; 4],
}

impl Segment {
    pub fn new(start: Vec3, end: Vec3, color: [f32; 4]) -> Self {
        Self {
            start: start.to_array(),
            end: end.to_array(),
            color,
        }
    }

    /// Its two end points, in Earth radii in the Earth-fixed frame.
    pub fn ends(&self) -> (Vec3, Vec3) {
        (Vec3::from_array(self.start), Vec3::from_array(self.end))
    }
}

/// Appends the segments joining `points` in order to `segments`.
pub fn push_polyline(segments: &mut Vec<Segment>, points: &[Vec3], color: [f32; 4]) {
    segments.extend(
        points
            .windows(2)
            .map(|pair| Segment::new(pair[0], pair[1], color)),
    );
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Uniforms {
    view_projection: [[f32; 4]; 4],
    camera_position: [f32; 3],
    half_width: f32,
    viewport: [f32; 2],
    _padding: [f32; 2],
}

pub struct Lines {
    pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    uniform_buffer: wgpu::Buffer,
    /// Holds `capacity` segments; replaced by a larger one when more are needed.
    segment_buffer: Option<wgpu::Buffer>,
    capacity: usize,
    count: u32,
    width_points: f32,
}

impl Lines {
    /// `target_format` must be an sRGB format, because the shader outputs linear colour.
    pub fn new(device: &wgpu::Device, target_format: wgpu::TextureFormat) -> Self {
        Self::with_width(device, target_format, WIDTH_POINTS)
    }

    /// As [`Self::new`], with lines `width_points` logical points wide.
    pub fn with_width(
        device: &wgpu::Device,
        target_format: wgpu::TextureFormat,
        width_points: f32,
    ) -> Self {
        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("line uniforms"),
            size: size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("lines"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("lines"),
            layout: &bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });

        let shader = device.create_shader_module(wgpu::include_wgsl!("lines.wgsl"));
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("lines"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("lines"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                // One entry per segment: the shader builds the quad's four corners itself.
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: size_of::<Segment>() as u64,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &wgpu::vertex_attr_array![
                        0 => Float32x3,
                        1 => Float32x3,
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

        Self {
            pipeline,
            bind_group,
            uniform_buffer,
            segment_buffer: None,
            capacity: 0,
            count: 0,
            width_points,
        }
    }

    /// Sets the view and the segments for the next draw.
    pub fn update(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        view: &View,
        segments: &[Segment],
    ) {
        self.set_view(queue, view);
        self.set_segments(device, queue, segments);
    }

    /// Sets the view for the next draw, keeping the segments as they are.
    pub fn set_view(&self, queue: &wgpu::Queue, view: &View) {
        let uniforms = Uniforms {
            view_projection: view.view_projection.to_cols_array_2d(),
            camera_position: view.camera_position.to_array(),
            half_width: 0.5 * self.width_points * view.scale_factor,
            viewport: view.viewport.map(|pixels| pixels as f32),
            _padding: [0.0; 2],
        };
        queue.write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&uniforms));
    }

    /// Replaces the segments.
    pub fn set_segments(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        segments: &[Segment],
    ) {
        if segments.len() > self.capacity {
            self.capacity = segments.len();
            self.segment_buffer = Some(device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("line segments"),
                size: (self.capacity * size_of::<Segment>()) as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }));
        }
        if let Some(buffer) = &self.segment_buffer {
            queue.write_buffer(buffer, 0, bytemuck::cast_slice(segments));
        }
        self.count = segments.len() as u32;
    }

    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        self.draw_ranges(pass, std::slice::from_ref(&(0..self.count)));
    }

    /// Draws only the segments whose indexes fall in `ranges`.
    pub fn draw_ranges(&self, pass: &mut wgpu::RenderPass<'_>, ranges: &[Range<u32>]) {
        let Some(buffer) = &self.segment_buffer else {
            return;
        };
        if self.count == 0 || ranges.is_empty() {
            return;
        }
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        let used_bytes = self.count as u64 * size_of::<Segment>() as u64;
        pass.set_vertex_buffer(0, buffer.slice(..used_bytes));
        for range in ranges {
            let range = range.start.min(self.count)..range.end.min(self.count);
            if !range.is_empty() {
                pass.draw(0..4, range);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_polyline_becomes_one_segment_per_join() {
        let mut segments = Vec::new();
        let color = [1.0, 0.5, 0.25, 1.0];
        push_polyline(&mut segments, &[Vec3::X, Vec3::Y, Vec3::Z], color);
        assert_eq!(
            segments,
            [
                Segment::new(Vec3::X, Vec3::Y, color),
                Segment::new(Vec3::Y, Vec3::Z, color),
            ]
        );

        // A single point is not a line.
        push_polyline(&mut segments, &[Vec3::X], color);
        assert_eq!(segments.len(), 2);
    }
}
