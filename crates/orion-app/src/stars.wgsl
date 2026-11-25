// The star field: one soft dot per star, infinitely far away.
// Star directions are in the inertial frame; the scene is drawn in the Earth-fixed frame.

struct Uniforms {
    view_projection: mat4x4<f32>,
    // Size of the drawing area, in pixels.
    viewport: vec2<f32>,
    // Pixels per logical point.
    scale_factor: f32,
    // How far, in radians, the Earth-fixed frame is turned east of the inertial frame.
    sidereal_angle: f32,
}

@group(0) @binding(0) var<uniform> uniforms: Uniforms;

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    // Position within the dot's square, from -1 to 1 on each axis.
    @location(0) offset: vec2<f32>,
    @location(1) color: vec4<f32>,
}

@vertex
fn vs_main(
    @builtin(vertex_index) vertex_index: u32,
    @location(0) direction: vec3<f32>,
    // Radius of the dot in logical points.
    @location(1) radius: f32,
    @location(2) color: vec4<f32>,
) -> VertexOutput {
    var out: VertexOutput;
    out.offset = vec2<f32>(f32(vertex_index & 1u), f32(vertex_index >> 1u)) * 2.0 - 1.0;
    out.color = color;

    // Seen from the turning Earth, the sky is turned west by the sidereal angle.
    let s = sin(uniforms.sidereal_angle);
    let c = cos(uniforms.sidereal_angle);
    let in_scene = vec3<f32>(
        direction.x * c + direction.y * s,
        -direction.x * s + direction.y * c,
        direction.z,
    );

    // A direction, not a place: with w = 0 the camera's position drops out and only
    // the way it is facing matters, which is what "infinitely far away" means.
    let clip = uniforms.view_projection * vec4<f32>(in_scene, 0.0);
    let radius_px = radius * uniforms.scale_factor;
    out.clip_position = vec4<f32>(
        clip.xy + out.offset * (2.0 * radius_px / uniforms.viewport) * clip.w,
        // Halfway into the depth range, where nothing clips it. Stars behind the
        // camera have a negative w and are discarded by the GPU.
        0.5 * clip.w,
        clip.w,
    );
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    // Brightest in the middle, fading to nothing at the edge.
    let coverage = 1.0 - smoothstep(0.25, 1.0, length(in.offset));
    return vec4<f32>(in.color.rgb, in.color.a * coverage);
}
