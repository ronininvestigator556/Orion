// Lines of a fixed width on screen, such as an Orbit Path, hidden where Earth is in the way.
// Lengths are in Earth radii in the Earth-fixed frame, so Earth is a unit sphere at the origin.

struct Uniforms {
    view_projection: mat4x4<f32>,
    camera_position: vec3<f32>,
    // Half the width of a line, in pixels.
    half_width: f32,
    // Size of the drawing area, in pixels.
    viewport: vec2<f32>,
}

@group(0) @binding(0) var<uniform> uniforms: Uniforms;

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) world_position: vec3<f32>,
    @location(1) color: vec4<f32>,
    // Position across the line, from -1 at one edge to 1 at the other.
    @location(2) across: f32,
}

// Points closer to the camera's plane than this are cut off, because the divide by w
// that turns clip space into screen space breaks down at and behind the camera.
const NEAR: f32 = 0.001;

// Points no farther than this from Earth's centre count as on the ground.
const GROUND_RADIUS: f32 = 1.0005;

// True if the straight line from the camera to `point` passes through Earth first.
fn hidden_by_earth(camera: vec3<f32>, point: vec3<f32>) -> bool {
    let radius_squared = dot(point, point);
    if radius_squared < GROUND_RADIUS * GROUND_RADIUS {
        // On the ground the line of sight ends on the sphere itself, where rounding
        // would decide the test below. Such a point is hidden only over the horizon.
        return dot(point, camera) < sqrt(radius_squared);
    }
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
    @location(0) start: vec3<f32>,
    @location(1) end: vec3<f32>,
    @location(2) color: vec4<f32>,
) -> VertexOutput {
    // Each segment is a quad drawn as a triangle strip: two corners at each end.
    let along = f32(vertex_index & 1u);
    let across = f32(vertex_index >> 1u) * 2.0 - 1.0;

    var out: VertexOutput;
    out.color = color;
    out.across = across;

    var world_start = start;
    var world_end = end;
    var clip_start = uniforms.view_projection * vec4<f32>(start, 1.0);
    var clip_end = uniforms.view_projection * vec4<f32>(end, 1.0);

    if clip_start.w < NEAR && clip_end.w < NEAR {
        // Wholly behind the camera: collapse to a point outside the visible volume.
        out.clip_position = vec4<f32>(2.0, 2.0, 2.0, 1.0);
        out.world_position = start;
        return out;
    }
    // Partly behind the camera: shorten the segment to the part in front.
    if clip_start.w < NEAR {
        let t = (NEAR - clip_start.w) / (clip_end.w - clip_start.w);
        clip_start = mix(clip_start, clip_end, t);
        world_start = mix(start, end, t);
    } else if clip_end.w < NEAR {
        let t = (NEAR - clip_end.w) / (clip_start.w - clip_end.w);
        clip_end = mix(clip_end, clip_start, t);
        world_end = mix(end, start, t);
    }

    // Work out the segment's direction on screen, in pixels, to widen it at right angles.
    let screen_start = clip_start.xy / clip_start.w * uniforms.viewport * 0.5;
    let screen_end = clip_end.xy / clip_end.w * uniforms.viewport * 0.5;
    var direction = screen_end - screen_start;
    let length_px = length(direction);
    if length_px < 0.0001 {
        direction = vec2<f32>(1.0, 0.0);
    } else {
        direction = direction / length_px;
    }
    let sideways = vec2<f32>(-direction.y, direction.x) * across * uniforms.half_width;

    let clip = mix(clip_start, clip_end, along);
    out.clip_position = vec4<f32>(clip.xy + sideways * 2.0 / uniforms.viewport * clip.w, clip.zw);
    out.world_position = mix(world_start, world_end, along);
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    // Tested per pixel, so a line passing behind Earth is cut exactly at the limb.
    if hidden_by_earth(uniforms.camera_position, in.world_position) {
        discard;
    }
    let coverage = 1.0 - smoothstep(0.5, 1.0, abs(in.across));
    return vec4<f32>(in.color.rgb, in.color.a * coverage);
}
