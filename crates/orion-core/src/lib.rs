//! Headless orbital maths for Orion: propagation, time and coordinate conversion.
//!
//! This crate has no graphics or windowing dependencies, so everything in it can be
//! tested against published reference values and reused outside the desktop app.
//! Domain terms follow `GLOSSARY.md` at the repository root.
//!
//! # Frames
//!
//! Two right-handed frames share Earth's centre as their origin and the North Pole as +Z:
//!
//! - **Earth-fixed**: turns with the planet. +X passes through latitude 0, longitude 0,
//!   and +Y through latitude 0, longitude 90 east.
//! - **Inertial**: fixed against the stars. +X points towards the March equinox.

pub mod catalogue;
pub mod clock;
pub mod codes;
pub mod filter;
pub mod frames;
pub mod layer;
pub mod reference;
pub mod sun;
pub mod tiles;
pub mod time;

/// Equatorial radius of Earth in kilometres (WGS 84).
pub const EARTH_EQUATORIAL_RADIUS_KM: f64 = 6378.137;
