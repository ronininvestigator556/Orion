//! The camera that looks at the globe.

// wgpu's clip space matches DirectX: Y up, depth from 0 to 1.
use glam::camera::rh::proj::directx::perspective;
use glam::camera::rh::view;
use glam::{Mat4, Vec3};

/// Vertical field of view.
const FOV_Y: f32 = std::f32::consts::FRAC_PI_4;
/// Closest approach, in Earth radii from the centre (about 640 km above the surface).
const MIN_DISTANCE: f32 = 1.1;
/// Farthest retreat, in Earth radii from the centre (well beyond the geostationary ring at 6.6).
const MAX_DISTANCE: f32 = 30.0;
/// Stops short of the poles, where "up" would become undefined.
const MAX_LATITUDE: f32 = 89.0 * std::f32::consts::PI / 180.0;
/// Fraction of the altitude removed by one scroll step.
const ZOOM_PER_STEP: f32 = 0.1;

/// A camera in the Earth-fixed frame that orbits Earth's centre and always looks at it.
///
/// Lengths are in Earth radii, so the globe is a unit sphere at the origin.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OrbitCamera {
    /// Longitude of the point directly below the camera, in radians east.
    longitude: f32,
    /// Latitude of the point directly below the camera, in radians north.
    latitude: f32,
    /// Distance from Earth's centre, in Earth radii.
    distance: f32,
}

impl Default for OrbitCamera {
    fn default() -> Self {
        Self {
            longitude: 0.0,
            latitude: 20_f32.to_radians(),
            distance: 3.2,
        }
    }
}

impl OrbitCamera {
    /// Turns the globe as if the surface under the cursor had been grabbed and moved
    /// by `dx`, `dy` pixels (right and down positive) in a viewport `viewport_height` pixels tall.
    pub fn drag(&mut self, dx: f32, dy: f32, viewport_height: f32) {
        // One pixel spans this angle at the camera; at the surface directly below, that
        // is this many radians of arc on the unit sphere.
        let radians_per_pixel = (self.distance - 1.0) * FOV_Y / viewport_height.max(1.0);
        self.longitude -= dx * radians_per_pixel;
        self.latitude = (self.latitude + dy * radians_per_pixel).clamp(-MAX_LATITUDE, MAX_LATITUDE);
    }

    /// Turns the camera to look straight down on the point of the globe beneath
    /// `position`, keeping its distance.
    pub fn face(&mut self, position: Vec3) {
        let Some(direction) = position.try_normalize() else {
            return;
        };
        self.longitude = direction.y.atan2(direction.x);
        self.latitude = direction.z.asin().clamp(-MAX_LATITUDE, MAX_LATITUDE);
    }

    /// Moves towards the globe for positive `steps` and away for negative ones.
    /// Each step changes the altitude by a fixed fraction, so zooming feels even at any height.
    pub fn zoom(&mut self, steps: f32) {
        let altitude = (self.distance - 1.0) * (1.0 - ZOOM_PER_STEP).powf(steps);
        self.distance = (1.0 + altitude).clamp(MIN_DISTANCE, MAX_DISTANCE);
    }

    pub fn position(&self) -> Vec3 {
        let (sin_lat, cos_lat) = self.latitude.sin_cos();
        let (sin_lon, cos_lon) = self.longitude.sin_cos();
        Vec3::new(cos_lat * cos_lon, cos_lat * sin_lon, sin_lat) * self.distance
    }

    /// Transform from the Earth-fixed frame to clip space for a viewport of the given aspect ratio.
    pub fn view_projection(&self, aspect: f32) -> Mat4 {
        let view = view::look_at_mat4(self.position(), Vec3::ZERO, Vec3::Z);
        // Nothing is depth-tested yet, so the planes only need to contain the scene.
        let projection = perspective(FOV_Y, aspect, 0.01, 2.0 * MAX_DISTANCE);
        projection * view
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn camera(longitude_deg: f32, latitude_deg: f32, distance: f32) -> OrbitCamera {
        OrbitCamera {
            longitude: longitude_deg.to_radians(),
            latitude: latitude_deg.to_radians(),
            distance,
        }
    }

    #[test]
    fn position_follows_the_earth_fixed_axes() {
        assert!(
            camera(0.0, 0.0, 2.0)
                .position()
                .abs_diff_eq(Vec3::X * 2.0, 1e-6)
        );
        assert!(
            camera(90.0, 0.0, 2.0)
                .position()
                .abs_diff_eq(Vec3::Y * 2.0, 1e-6)
        );
        assert!(camera(0.0, 89.0, 2.0).position().z > 1.99);
    }

    #[test]
    fn the_point_below_the_camera_lands_in_the_centre_of_the_view() {
        let camera = camera(30.0, -40.0, 3.0);
        let below = camera.position().normalize();
        let clip = camera.view_projection(1.5) * below.extend(1.0);
        assert!((clip.x / clip.w).abs() < 1e-5);
        assert!((clip.y / clip.w).abs() < 1e-5);
    }

    #[test]
    fn dragging_right_brings_western_longitudes_into_view() {
        let mut camera = camera(0.0, 0.0, 3.0);
        camera.drag(50.0, 0.0, 800.0);
        assert!(camera.longitude < 0.0);
    }

    #[test]
    fn dragging_cannot_pass_the_poles() {
        let mut camera = camera(0.0, 80.0, 3.0);
        camera.drag(0.0, 100_000.0, 800.0);
        assert_eq!(camera.latitude, MAX_LATITUDE);
    }

    #[test]
    fn facing_a_position_puts_it_in_the_centre_of_the_view() {
        let mut camera = camera(10.0, 10.0, 3.0);
        let target = Vec3::new(-0.4, 0.7, -0.5) * 1.07;
        camera.face(target);

        let clip = camera.view_projection(1.5) * target.extend(1.0);
        assert!((clip.x / clip.w).abs() < 1e-5 && (clip.y / clip.w).abs() < 1e-5);
        assert_eq!(camera.distance, 3.0);

        // Earth's centre is beneath nowhere, so it changes nothing.
        let before = camera;
        camera.face(Vec3::ZERO);
        assert_eq!(camera, before);
    }

    #[test]
    fn zoom_stays_within_its_limits() {
        let mut camera = camera(0.0, 0.0, 3.0);
        camera.zoom(1_000.0);
        assert_eq!(camera.distance, MIN_DISTANCE);
        camera.zoom(-1_000.0);
        assert_eq!(camera.distance, MAX_DISTANCE);
    }
}
