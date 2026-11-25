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

    /// The same camera turned east about Earth's axis by `angle` radians.
    fn turned(self, angle: f32) -> Self {
        Self {
            longitude: (self.longitude + angle).rem_euclid(std::f32::consts::TAU),
            ..self
        }
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

/// What the camera keeps still against.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ViewFrame {
    /// Earth stays still and Tracked Objects move over it.
    #[default]
    EarthFixed,
    /// The stars stay still and Earth rotates beneath the orbits.
    StarFixed,
}

/// The camera together with the ways it moves by itself: turning with the stars in
/// Star-Fixed View, and keeping an object centred in Follow Mode.
///
/// The scene is always drawn in the Earth-fixed frame. Methods that need to relate
/// that frame to the stars take the sidereal angle: how far, in radians, the
/// Earth-fixed frame is turned east of the inertial frame at the moment on show.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CameraRig {
    /// The camera in the frame it keeps still against. In Star-Fixed View its longitude
    /// is measured from the inertial +X axis instead of from Greenwich.
    camera: OrbitCamera,
    frame: ViewFrame,
    following: bool,
}

impl CameraRig {
    /// The camera in the Earth-fixed frame, ready to draw the scene with.
    pub fn scene_camera(&self, sidereal_angle: f32) -> OrbitCamera {
        match self.frame {
            ViewFrame::EarthFixed => self.camera,
            // A camera fixed against the stars slips west over the turning Earth.
            ViewFrame::StarFixed => self.camera.turned(-sidereal_angle),
        }
    }

    pub fn frame(&self) -> ViewFrame {
        self.frame
    }

    /// Changes what the camera keeps still against, without moving the view.
    pub fn set_frame(&mut self, frame: ViewFrame, sidereal_angle: f32) {
        if frame == self.frame {
            return;
        }
        self.camera = match frame {
            ViewFrame::StarFixed => self.camera.turned(sidereal_angle),
            ViewFrame::EarthFixed => self.camera.turned(-sidereal_angle),
        };
        self.frame = frame;
    }

    pub fn is_following(&self) -> bool {
        self.following
    }

    pub fn set_following(&mut self, following: bool) {
        self.following = following;
    }

    /// Turns the globe by hand, which ends Follow Mode: the two would fight.
    pub fn drag(&mut self, dx: f32, dy: f32, viewport_height: f32) {
        self.following = false;
        self.camera.drag(dx, dy, viewport_height);
    }

    pub fn zoom(&mut self, steps: f32) {
        self.camera.zoom(steps);
    }

    /// Turns the camera to look straight down on `position`, given in the scene's
    /// Earth-fixed frame.
    pub fn face(&mut self, position: Vec3, sidereal_angle: f32) {
        if position.try_normalize().is_none() {
            return;
        }
        self.camera.face(position);
        if self.frame == ViewFrame::StarFixed {
            self.camera = self.camera.turned(sidereal_angle);
        }
    }

    /// In Follow Mode, keeps `target` centred. Call once per frame with the followed
    /// object's position in the scene, or `None` if it has none.
    pub fn follow(&mut self, target: Option<Vec3>, sidereal_angle: f32) {
        if let (true, Some(target)) = (self.following, target) {
            self.face(target, sidereal_angle);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Where a point of the scene lands on screen, as x and y from -1 to 1.
    fn on_screen(camera: OrbitCamera, point: glam::Vec4) -> glam::Vec2 {
        let clip = camera.view_projection(1.5) * point;
        clip.truncate().truncate() / clip.w
    }

    /// A direction fixed against the stars, as it appears in the Earth-fixed scene
    /// once Earth has turned by `sidereal_angle`.
    fn star_in_scene(sidereal_angle: f32) -> glam::Vec4 {
        let star = Vec3::new(0.8, 0.5, 0.33).normalize();
        (glam::Quat::from_rotation_z(-sidereal_angle) * star).extend(0.0)
    }

    #[test]
    fn in_earth_fixed_view_the_ground_stays_put_and_the_stars_drift() {
        let rig = CameraRig::default();
        let ground = Vec3::new(1.0, 0.1, 0.3).normalize().extend(1.0);

        let (early, late) = (rig.scene_camera(0.3), rig.scene_camera(0.9));
        assert_eq!(early, late);
        assert_eq!(on_screen(early, ground), on_screen(late, ground));
        let drift =
            on_screen(early, star_in_scene(0.3)).distance(on_screen(late, star_in_scene(0.9)));
        assert!(drift > 0.1, "the star only moved {drift}");
    }

    #[test]
    fn in_star_fixed_view_the_stars_stay_put_and_the_ground_drifts() {
        let mut rig = CameraRig::default();
        rig.set_frame(ViewFrame::StarFixed, 0.3);
        let ground = Vec3::new(1.0, 0.1, 0.3).normalize().extend(1.0);

        let (early, late) = (rig.scene_camera(0.3), rig.scene_camera(0.9));
        let star_shift =
            on_screen(early, star_in_scene(0.3)).distance(on_screen(late, star_in_scene(0.9)));
        assert!(star_shift < 1e-4, "the star moved {star_shift}");
        let ground_shift = on_screen(early, ground).distance(on_screen(late, ground));
        assert!(ground_shift > 0.1, "the ground only moved {ground_shift}");
    }

    #[test]
    fn switching_views_does_not_move_the_picture() {
        let mut rig = CameraRig::default();
        let before = rig.scene_camera(1.2).position();

        rig.set_frame(ViewFrame::StarFixed, 1.2);
        assert_eq!(rig.frame(), ViewFrame::StarFixed);
        assert!(rig.scene_camera(1.2).position().abs_diff_eq(before, 1e-5));

        rig.set_frame(ViewFrame::EarthFixed, 1.2);
        assert!(rig.scene_camera(1.2).position().abs_diff_eq(before, 1e-5));
    }

    #[test]
    fn follow_mode_keeps_the_target_centred_in_either_view() {
        for frame in [ViewFrame::EarthFixed, ViewFrame::StarFixed] {
            let mut rig = CameraRig::default();
            rig.set_frame(frame, 2.0);
            let target = Vec3::new(-0.3, 0.9, 0.4).normalize() * 1.07;

            // Nothing happens until Follow Mode is on.
            let before = rig;
            rig.follow(Some(target), 2.0);
            assert_eq!(rig, before);

            rig.set_following(true);
            rig.follow(Some(target), 2.0);
            let centre = on_screen(rig.scene_camera(2.0), target.extend(1.0));
            assert!(centre.length() < 1e-4, "{frame:?}: {centre}");

            // An object with no Position leaves the camera where it was.
            let held = rig;
            rig.follow(None, 2.5);
            assert_eq!(rig, held);
        }
    }

    #[test]
    fn dragging_ends_follow_mode() {
        let mut rig = CameraRig::default();
        rig.set_following(true);
        rig.drag(10.0, 0.0, 800.0);
        assert!(!rig.is_following());
    }

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
