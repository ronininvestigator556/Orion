// Earth: a unit sphere in the Earth-fixed frame, lit by the Sun, with city lights on the night side.

struct Uniforms {
    view_projection: mat4x4<f32>,
    // Unit vector from Earth's centre to the Sun, in the Earth-fixed frame.
    sun_direction: vec3<f32>,
}

@group(0) @binding(0) var<uniform> uniforms: Uniforms;
@group(0) @binding(1) var day_texture: texture_2d<f32>;
@group(0) @binding(2) var night_texture: texture_2d<f32>;
@group(0) @binding(3) var earth_sampler: sampler;

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) normal: vec3<f32>,
    @location(1) uv: vec2<f32>,
}

@vertex
fn vs_main(@location(0) position: vec3<f32>, @location(1) uv: vec2<f32>) -> VertexOutput {
    var out: VertexOutput;
    out.clip_position = uniforms.view_projection * vec4<f32>(position, 1.0);
    // On a unit sphere centred on the origin, the outward normal is the position itself.
    out.normal = position;
    out.uv = uv;
    return out;
}

// Sine of the Sun's elevation at which twilight ends: 6 degrees either side of the horizon.
const TWILIGHT: f32 = 0.1;

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let day = textureSample(day_texture, earth_sampler, in.uv).rgb;
    let night = textureSample(night_texture, earth_sampler, in.uv).rgb;

    // Sine of the Sun's elevation above the local horizon: 1 directly overhead, 0 on the terminator.
    let sun_elevation = dot(normalize(in.normal), uniforms.sun_direction);

    // Sunlit ground dims towards the terminator. The exponent lifts the mid-tones to stand
    // in for the light the atmosphere scatters, which a bare cosine falloff leaves too dark.
    let sunlit = day * pow(max(sun_elevation, 0.0), 0.6);
    let daylight = smoothstep(-TWILIGHT, TWILIGHT, sun_elevation);

    return vec4<f32>(mix(night, sunlit, daylight), 1.0);
}
