//! Draws Orion's app icon and writes it in each platform's format.
//!
//!     cargo run --package orion-app --example make_icon
//!
//! The icon is drawn by the code below, so this file is its source. It shows Earth
//! with an orbit ring and a satellite, under the three stars of Orion's Belt.
//! Rerun it after changing the drawing; the files it writes are committed.

use std::io::Cursor;
use std::path::{Path, PathBuf};

use glam::{Vec2, Vec3};
use image::{ImageFormat, RgbaImage};

/// Samples taken along each side of a pixel, for smooth edges.
const SUPERSAMPLE: u32 = 4;

fn main() -> anyhow::Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let png = |size: u32| encode_png(&render(size));

    write(&root, "packaging/icon/orion-1024.png", &png(1024)?)?;
    write(&root, "packaging/linux/orion.png", &png(256)?)?;
    // The running app decodes this one for its window icon.
    write(&root, "crates/orion-app/assets/icon-256.png", &png(256)?)?;

    // Windows: an .ico is a list of the same picture at several sizes.
    let ico_sizes = [16, 24, 32, 48, 64, 128, 256];
    let ico_images = ico_sizes
        .iter()
        .map(|&size| Ok((size, png(size)?)))
        .collect::<anyhow::Result<Vec<_>>>()?;
    write(&root, "packaging/windows/orion.ico", &ico(&ico_images))?;

    // macOS: an .icns is a list of pictures tagged by a four-letter type, one for
    // each size the system asks for. The "@2x" types are the high-density variants.
    let icns_types: [(&[u8; 4], u32); 10] = [
        (b"icp4", 16),
        (b"icp5", 32),
        (b"ic07", 128),
        (b"ic08", 256),
        (b"ic09", 512),
        (b"ic10", 1024),
        (b"ic11", 32),
        (b"ic12", 64),
        (b"ic13", 256),
        (b"ic14", 512),
    ];
    let icns_images = icns_types
        .iter()
        .map(|&(kind, size)| Ok((*kind, png(size)?)))
        .collect::<anyhow::Result<Vec<_>>>()?;
    write(&root, "packaging/macos/Orion.icns", &icns(&icns_images))?;
    Ok(())
}

fn write(root: &Path, relative: &str, bytes: &[u8]) -> anyhow::Result<()> {
    let path: PathBuf = root.join(relative);
    std::fs::write(&path, bytes)?;
    println!("wrote {relative} ({} bytes)", bytes.len());
    Ok(())
}

fn encode_png(image: &RgbaImage) -> anyhow::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    image.write_to(&mut Cursor::new(&mut bytes), ImageFormat::Png)?;
    Ok(bytes)
}

/// Packs PNG pictures into a Windows icon file.
fn ico(images: &[(u32, Vec<u8>)]) -> Vec<u8> {
    let mut file = Vec::new();
    // Header: reserved, type 1 (icon), number of pictures.
    file.extend_from_slice(&[0, 0, 1, 0]);
    file.extend_from_slice(&(images.len() as u16).to_le_bytes());

    // Then a directory entry per picture, saying where its data starts.
    let mut offset = 6 + 16 * images.len() as u32;
    for (size, png) in images {
        // A size of 256 is written as 0.
        let side = if *size >= 256 { 0 } else { *size as u8 };
        file.extend_from_slice(&[side, side, 0, 0]);
        file.extend_from_slice(&1u16.to_le_bytes()); // colour planes
        file.extend_from_slice(&32u16.to_le_bytes()); // bits per pixel
        file.extend_from_slice(&(png.len() as u32).to_le_bytes());
        file.extend_from_slice(&offset.to_le_bytes());
        offset += png.len() as u32;
    }
    for (_, png) in images {
        file.extend_from_slice(png);
    }
    file
}

/// Packs PNG pictures into a macOS icon file.
fn icns(images: &[([u8; 4], Vec<u8>)]) -> Vec<u8> {
    let body_len: usize = images.iter().map(|(_, png)| 8 + png.len()).sum();
    let mut file = Vec::new();
    file.extend_from_slice(b"icns");
    file.extend_from_slice(&((8 + body_len) as u32).to_be_bytes());
    for (kind, png) in images {
        file.extend_from_slice(kind);
        // Each entry's length counts its own eight-byte header.
        file.extend_from_slice(&((8 + png.len()) as u32).to_be_bytes());
        file.extend_from_slice(png);
    }
    file
}

/// Draws the icon at `size` by `size` pixels.
fn render(size: u32) -> RgbaImage {
    RgbaImage::from_fn(size, size, |x, y| {
        // Average several samples per pixel. Colour is weighted by coverage so the
        // transparent corners do not darken the edge.
        let mut colour = Vec3::ZERO;
        let mut coverage = 0.0;
        for sy in 0..SUPERSAMPLE {
            for sx in 0..SUPERSAMPLE {
                let offset = Vec2::new(sx as f32 + 0.5, sy as f32 + 0.5) / SUPERSAMPLE as f32;
                let (sample, alpha) = shade((Vec2::new(x as f32, y as f32) + offset) / size as f32);
                colour += sample * alpha;
                coverage += alpha;
            }
        }
        let samples = (SUPERSAMPLE * SUPERSAMPLE) as f32;
        let colour = if coverage > 0.0 {
            colour / coverage
        } else {
            Vec3::ZERO
        };
        let byte = |value: f32| (value.clamp(0.0, 1.0) * 255.0).round() as u8;
        image::Rgba([
            byte(colour.x),
            byte(colour.y),
            byte(colour.z),
            byte(coverage / samples),
        ])
    })
}

/// Lays `colour` over what is already there, at the given opacity.
fn over(base: &mut Vec3, colour: Vec3, opacity: f32) {
    *base = base.lerp(colour, opacity.clamp(0.0, 1.0));
}

fn rgb(hex: u32) -> Vec3 {
    Vec3::new(
        ((hex >> 16) & 0xff) as f32,
        ((hex >> 8) & 0xff) as f32,
        (hex & 0xff) as f32,
    ) / 255.0
}

fn smoothstep(low: f32, high: f32, value: f32) -> f32 {
    let t = ((value - low) / (high - low)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

// The picture is described on a unit square, with y running down the page.
const GLOBE_CENTRE: Vec2 = Vec2::new(0.5, 0.565);
const GLOBE_RADIUS: f32 = 0.235;
/// Half-width and half-height of the orbit ring before it is tilted.
const RING: Vec2 = Vec2::new(0.37, 0.125);
/// The ring's tilt, lifting its right-hand end.
const RING_TILT: f32 = -0.38;
const RING_HALF_WIDTH: f32 = 0.0115;
/// Where the satellite sits on the ring, as an angle around it.
const SATELLITE_ANGLE: f32 = 0.7;

/// The colour and opacity of the icon at point `p` of the unit square.
fn shade(p: Vec2) -> (Vec3, f32) {
    // The tile: a rounded square with a margin, as macOS icons have.
    let from_centre = (p - 0.5).abs() - Vec2::splat(0.402 - 0.18);
    let outside = from_centre.max(Vec2::ZERO).length() + from_centre.max_element().min(0.0) - 0.18;
    if outside > 0.0 {
        return (Vec3::ZERO, 0.0);
    }

    // Night sky, slightly lighter towards the top.
    let mut colour = rgb(0x05_07_14).lerp(rgb(0x18_1d_4d), smoothstep(1.0, 0.0, p.y));

    // A scattering of faint stars, and the three bright ones of Orion's Belt.
    let faint = [
        (0.70, 0.19, 0.006),
        (0.82, 0.27, 0.004),
        (0.60, 0.14, 0.004),
        (0.17, 0.48, 0.005),
        (0.86, 0.72, 0.005),
        (0.24, 0.84, 0.004),
        (0.72, 0.85, 0.006),
        (0.44, 0.17, 0.003),
    ];
    for (x, y, radius) in faint {
        let distance = p.distance(Vec2::new(x, y));
        over(
            &mut colour,
            rgb(0xc8_d6_ff),
            0.8 * smoothstep(radius, radius * 0.3, distance),
        );
    }
    for star in [
        Vec2::new(0.236, 0.32),
        Vec2::new(0.312, 0.272),
        Vec2::new(0.388, 0.226),
    ] {
        let distance = p.distance(star);
        over(
            &mut colour,
            rgb(0x7f_a8_ff),
            0.35 * smoothstep(0.05, 0.0, distance),
        );
        over(
            &mut colour,
            rgb(0xf2_f6_ff),
            smoothstep(0.017, 0.011, distance),
        );
    }

    // The ring in its own untilted frame. The half above its long axis passes behind Earth.
    let (sin, cos) = RING_TILT.sin_cos();
    let rel = p - GLOBE_CENTRE;
    let on_ring = Vec2::new(rel.x * cos + rel.y * sin, -rel.x * sin + rel.y * cos);
    let scaled = on_ring / RING;
    // How far the point is from the ellipse, estimated from how fast "scaled.length()" changes.
    let slope = (on_ring / (RING * RING)).length() / scaled.length().max(1e-6);
    let ring_distance = ((scaled.length() - 1.0) / slope.max(1e-6)).abs();
    let ring = smoothstep(RING_HALF_WIDTH, RING_HALF_WIDTH * 0.6, ring_distance);
    let ring_colour = rgb(0xa9_d8_ff);
    let behind = on_ring.y < 0.0;
    if behind {
        over(&mut colour, ring_colour, 0.6 * ring);
    }

    // Earth, lit from the upper left, with a thin glow of atmosphere around it.
    let from_globe = rel.length() / GLOBE_RADIUS;
    let light = Vec3::new(-0.55, 0.6, 0.58).normalize();
    let glow = smoothstep(1.16, 1.0, from_globe) * smoothstep(0.9, 1.0, from_globe);
    over(&mut colour, rgb(0x5f_b0_ff), 0.5 * glow);
    if from_globe < 1.0 {
        // The surface normal: x to the right, y up, z towards the viewer.
        let flat = rel / GLOBE_RADIUS;
        let normal = Vec3::new(flat.x, -flat.y, (1.0 - flat.length_squared()).sqrt());
        let sunlight = smoothstep(-0.15, 0.75, normal.dot(light));

        // Loosely continent-shaped patches, from a few overlapping waves.
        let longitude = normal.x.atan2(normal.z) + 0.6;
        let latitude = normal.y.asin();
        let waves = (2.4 * longitude + 0.9 * latitude).sin() * (2.1 * latitude + 0.5).cos()
            + 0.55 * (4.3 * longitude - 2.7 * latitude + 1.1).sin()
            + 0.25 * (7.0 * longitude + 5.0 * latitude).sin();
        let land = smoothstep(0.46, 0.58, waves);

        let sea = rgb(0x06_12_30).lerp(rgb(0x2c_7d_e6), sunlight);
        let ground = rgb(0x07_1a_1c).lerp(rgb(0x5c_bf_72), sunlight);
        let mut surface = sea.lerp(ground, land);
        // A soft highlight where the light strikes most directly.
        over(
            &mut surface,
            rgb(0xd8_ec_ff),
            0.22 * smoothstep(0.86, 1.0, normal.dot(light)),
        );
        // Blend the very edge so the limb is not jagged against the glow.
        over(&mut colour, surface, smoothstep(1.0, 0.985, from_globe));
    }

    // The near half of the ring crosses in front of Earth.
    if !behind {
        over(&mut colour, ring_colour, ring);
    }

    // The satellite: a bright dot on the near side of the ring, with a glow.
    let (angle_sin, angle_cos) = SATELLITE_ANGLE.sin_cos();
    let local = RING * Vec2::new(angle_cos, angle_sin);
    let satellite =
        GLOBE_CENTRE + Vec2::new(local.x * cos - local.y * sin, local.x * sin + local.y * cos);
    let distance = p.distance(satellite);
    over(
        &mut colour,
        rgb(0x8f_d0_ff),
        0.5 * smoothstep(0.075, 0.02, distance),
    );
    over(
        &mut colour,
        rgb(0xff_ff_ff),
        smoothstep(0.034, 0.027, distance),
    );

    (colour, 1.0)
}
