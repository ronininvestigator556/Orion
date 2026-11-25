// One tile of the streamed Base Map: a patch of the unit sphere with its own picture.
// Tiles are drawn over the bundled picture of Earth, coarse levels first.

struct Frame {
    view_projection: mat4x4<f32>,
    // Unit vector from Earth's centre to the Sun, in the Earth-fixed frame.
    sun_direction: vec3<f32>,
    // 0 seen from far away, rising to 1 close to the ground.
    closeness: f32,
    // For a picture laid over the Base Map: 1, its opacity, and 1 if black is empty.
    // All zero for the Base Map itself.
    overlay: vec4<f32>,
}

struct Tile {
    // The west and north edges, and the width and height that lie inside the world,
    // all in radians.
    west: f32,
    north: f32,
    width: f32,
    height: f32,
    // The fraction of the picture that lies inside the world, across and down.
    filled: vec2<f32>,
}

@group(0) @binding(0) var<uniform> frame: Frame;
@group(0) @binding(1) var tile_sampler: sampler;
@group(1) @binding(0) var<uniform> tile: Tile;
@group(1) @binding(1) var tile_texture: texture_2d<f32>;

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) normal: vec3<f32>,
    @location(1) uv: vec2<f32>,
}

@vertex
fn vs_main(@location(0) grid: vec2<f32>) -> VertexOutput {
    // `grid` runs from 0 to 1 east across the tile and south down it.
    let longitude = tile.west + grid.x * tile.width;
    let latitude = tile.north - grid.y * tile.height;
    let position = vec3<f32>(
        cos(latitude) * cos(longitude),
        cos(latitude) * sin(longitude),
        sin(latitude),
    );

    var out: VertexOutput;
    out.clip_position = frame.view_projection * vec4<f32>(position, 1.0);
    out.normal = position;
    out.uv = grid * tile.filled;
    return out;
}

// Sine of the Sun's elevation at which twilight ends: 6 degrees either side of the horizon.
const TWILIGHT: f32 = 0.1;

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let texel = textureSample(tile_texture, tile_sampler, in.uv);
    if frame.overlay.x > 0.5 {
        // A picture over the Base Map is shown as it is, by day and by night, through
        // its own transparency. Some pictures leave what they did not see black.
        var seen = texel.a;
        if frame.overlay.z > 0.5 {
            seen *= smoothstep(0.004, 0.03, max(texel.r, max(texel.g, texel.b)));
        }
        return vec4<f32>(texel.rgb, seen * frame.overlay.y);
    }
    let day = texel.rgb;
    let sun_elevation = dot(normalize(in.normal), frame.sun_direction);
    let sunlit = pow(max(sun_elevation, 0.0), 0.6);
    let daylight = smoothstep(-TWILIGHT, TWILIGHT, sun_elevation);

    // From far away the night side is left to the picture underneath, which shows city
    // lights. Close to the ground that picture is a blur, so the tile is shown there
    // too, dimmed, and the map stays readable at night.
    let brightness = max(sunlit, 0.3 * frame.closeness);
    let opacity = max(daylight, frame.closeness);
    return vec4<f32>(day * brightness, opacity);
}
