// One tile of the street map: a patch of the globe with its own ready-drawn picture.
//
// Close to the ground a tile is a few hundred metres across, far finer than a
// position on the whole globe can be held to in single precision. So each tile's
// points are given from its own middle, and the matrix that takes them to the screen
// is worked out for that tile in double precision before it is handed over.

struct Tile {
    // From the tile's own middle to clip space.
    to_clip: mat4x4<f32>,
    // How much of what lies beneath the street map covers, from 0 to 1, in x.
    opacity: vec4<f32>,
}

@group(0) @binding(0) var tile_sampler: sampler;
@group(1) @binding(0) var<uniform> tile: Tile;
@group(1) @binding(1) var tile_texture: texture_2d<f32>;

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs_main(@location(0) from_middle: vec3<f32>, @location(1) uv: vec2<f32>) -> VertexOutput {
    var out: VertexOutput;
    out.clip_position = tile.to_clip * vec4<f32>(from_middle, 1.0);
    out.uv = uv;
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    // A map is read the same by day and by night, so the Sun plays no part.
    let texel = textureSample(tile_texture, tile_sampler, in.uv);
    return vec4<f32>(texel.rgb, texel.a * tile.opacity.x);
}
