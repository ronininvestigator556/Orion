// Tracked Objects: one fixed-size dot per object, hidden when Earth is in the way.
// Lengths are in Earth radii in the Earth-fixed frame, so Earth is a unit sphere at the origin.

struct Uniforms {
    view_projection: mat4x4<f32>,
    camera_position: vec3<f32>,
    // Radius of a dot, in pixels.
    dot_radius: f32,
    // Size of the drawing area, in pixels.
    viewport: vec2<f32>,
}

@group(0) @binding(0) var<uniform> uniforms: Uniforms;

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    // Position within the dot's square, from -1 to 1 on each axis.
    @location(0) offset: vec2<f32>,
    @location(1) color: vec4<f32>,
    // 1 for the selected object, which is drawn as a ring around a dot.
    @location(2) @interpolate(flat) selected: u32,
}

// Bit flags above the kind. These must match the constants in objects.rs.
const SELECTED: u32 = 256u;
const HOVERED: u32 = 512u;
const STALE: u32 = 1024u;

// True if the straight line from the camera to `point` passes through Earth first.
// The camera is always outside the sphere.
fn hidden_by_earth(camera: vec3<f32>, point: vec3<f32>) -> bool {
    // Solve |camera + t * to_point| = 1 for t; a hit with t below 1 is in front of the point.
    let to_point = point - camera;
    let a = dot(to_point, to_point);
    let half_b = dot(camera, to_point);
    let c = dot(camera, camera) - 1.0;
    let discriminant = half_b * half_b - a * c;
    if discriminant <= 0.0 || half_b >= 0.0 {
        return false;
    }
    return (-half_b - sqrt(discriminant)) / a < 1.0;
}

@vertex
fn vs_main(
    @builtin(vertex_index) vertex_index: u32,
    @location(0) position: vec3<f32>,
    @location(1) kind_and_marks: u32,
) -> VertexOutput {
    let kind = kind_and_marks & 255u;
    // Linear colours, indexed by kind: Satellite, Rocket Body, Debris.
    var colors = array<vec4<f32>, 3>(
        vec4<f32>(0.35, 0.75, 1.0, 1.0),
        vec4<f32>(1.0, 0.5, 0.12, 1.0),
        vec4<f32>(0.75, 0.2, 0.2, 0.75),
    );

    var out: VertexOutput;
    // The four corners of a triangle strip: (-1,-1), (1,-1), (-1,1), (1,1).
    out.offset = vec2<f32>(f32(vertex_index & 1u), f32(vertex_index >> 1u)) * 2.0 - 1.0;
    out.color = colors[min(kind, 2u)];
    out.selected = 0u;

    var radius = uniforms.dot_radius;
    if (kind_and_marks & STALE) != 0u {
        // The Position is unreliable, so the dot is faint.
        out.color.a *= 0.3;
    }
    if (kind_and_marks & HOVERED) != 0u {
        radius *= 1.8;
        out.color = vec4<f32>(mix(out.color.rgb, vec3<f32>(1.0), 0.5), 1.0);
    }
    if (kind_and_marks & SELECTED) != 0u {
        radius *= 4.0;
        out.color = vec4<f32>(1.0, 1.0, 1.0, 1.0);
        out.selected = 1u;
    }

    if hidden_by_earth(uniforms.camera_position, position) {
        // Collapse the dot to a point outside the visible volume so nothing is drawn.
        out.clip_position = vec4<f32>(2.0, 2.0, 2.0, 1.0);
        return out;
    }

    var clip = uniforms.view_projection * vec4<f32>(position, 1.0);
    // Clip space spans 2 units across the viewport before the divide by w, so scaling
    // by w keeps the dot the same size on screen at any distance.
    clip = vec4<f32>(
        clip.xy + out.offset * (2.0 * radius / uniforms.viewport) * clip.w,
        clip.zw,
    );
    out.clip_position = clip;
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let distance = length(in.offset);
    // Round the square off into a disc with a soft edge.
    var coverage = 1.0 - smoothstep(0.6, 1.0, distance);
    if in.selected == 1u {
        // A small dot in the middle with a ring around it.
        let dot = 1.0 - smoothstep(0.2, 0.3, distance);
        let ring = smoothstep(0.7, 0.8, distance) * (1.0 - smoothstep(0.9, 1.0, distance));
        coverage = max(dot, ring);
    }
    return vec4<f32>(in.color.rgb, in.color.a * coverage);
}
