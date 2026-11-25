//! Headless orbital maths for Orion: propagation, time and coordinate conversion.
//!
//! This crate has no graphics or windowing dependencies, so everything in it can be
//! tested against published reference values and reused outside the desktop app.
//! Domain terms follow `GLOSSARY.md` at the repository root.

/// Equatorial radius of Earth in kilometres (WGS 84).
pub const EARTH_EQUATORIAL_RADIUS_KM: f64 = 6378.137;
