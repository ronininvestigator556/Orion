//! Finding which dot is under the cursor.

use glam::{Mat4, Vec2, Vec3};

use crate::objects::Instance;

/// The camera and screen facts needed to relate the scene to the cursor.
#[derive(Clone, Copy, Debug)]
pub struct PickView {
    pub view_projection: Mat4,
    /// Camera position in Earth radii.
    pub camera_position: Vec3,
    /// Size of the drawing area in pixels.
    pub viewport: Vec2,
}

impl PickView {
    /// Where a point in the scene appears on screen, in pixels from the top-left corner,
    /// or `None` if it cannot be seen: off screen, behind the camera or behind Earth.
    pub fn project(&self, position: Vec3) -> Option<Vec2> {
        if hidden_by_earth(self.camera_position, position) {
            return None;
        }
        let clip = self.view_projection * position.extend(1.0);
        if clip.w <= 0.0 {
            return None;
        }
        let ndc = clip.truncate() / clip.w;
        if ndc.x.abs() > 1.0 || ndc.y.abs() > 1.0 || !(0.0..=1.0).contains(&ndc.z) {
            return None;
        }
        // Clip space has +Y up; the screen has +Y down.
        Some(Vec2::new(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5) * self.viewport)
    }

    /// The index of the visible dot nearest to `cursor`, if one is within `reach` pixels.
    pub fn pick(&self, instances: &[Instance], cursor: Vec2, reach: f32) -> Option<usize> {
        instances
            .iter()
            .enumerate()
            .filter(|(_, instance)| !instance.is_hidden())
            .filter_map(|(index, instance)| {
                let distance = self.project(instance.position())?.distance(cursor);
                (distance <= reach).then_some((index, distance))
            })
            .min_by(|(_, a), (_, b)| a.total_cmp(b))
            .map(|(index, _)| index)
    }
}

/// Points no farther than this from Earth's centre count as on the ground.
/// The shaders use the same figure.
const GROUND_RADIUS: f32 = 1.0005;

/// True if the straight line from the camera to `point` passes through Earth first.
/// Earth is the unit sphere at the origin and the camera is outside it. This mirrors
/// the test the shaders use, so what can be picked is exactly what is drawn.
fn hidden_by_earth(camera: Vec3, point: Vec3) -> bool {
    let radius_squared = point.length_squared();
    if radius_squared < 0.25 {
        // Deep inside Earth, where dots that are not to be drawn are put.
        return true;
    }
    if radius_squared < GROUND_RADIUS * GROUND_RADIUS {
        // On the ground the line of sight ends on the sphere itself, where rounding
        // would decide the test below. Such a point is hidden only over the horizon.
        return point.dot(camera) < radius_squared.sqrt();
    }
    let to_point = point - camera;
    let a = to_point.length_squared();
    let half_b = camera.dot(to_point);
    let c = camera.length_squared() - 1.0;
    let discriminant = half_b * half_b - a * c;
    if discriminant <= 0.0 || half_b >= 0.0 {
        return false;
    }
    (-half_b - discriminant.sqrt()) / a < 1.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::camera::OrbitCamera;
    use crate::objects::Marks;

    /// The default camera sits above longitude 0 on the +X side of Earth.
    fn view() -> PickView {
        let camera = OrbitCamera::default();
        PickView {
            view_projection: camera.view_projection(1.6),
            camera_position: camera.position(),
            viewport: Vec2::new(1600.0, 1000.0),
        }
    }

    fn dot(position: Vec3) -> Instance {
        Instance::new(position, [255; 4], Marks::default())
    }

    #[test]
    fn a_point_between_camera_and_earth_projects_to_the_centre() {
        let view = view();
        let point = view.camera_position.normalize() * 1.1;
        let on_screen = view.project(point).expect("a visible point");
        assert!(
            on_screen.distance(Vec2::new(800.0, 500.0)) < 0.01,
            "{on_screen}"
        );
    }

    #[test]
    fn points_behind_earth_or_the_camera_cannot_be_seen() {
        let view = view();
        let towards_camera = view.camera_position.normalize();
        assert_eq!(view.project(-towards_camera * 1.1), None);
        assert_eq!(view.project(towards_camera * 10.0), None);
    }

    #[test]
    fn the_nearest_dot_within_reach_is_picked() {
        let view = view();
        let towards_camera = view.camera_position.normalize();
        let centre = towards_camera * 1.1;
        let instances = [
            dot(centre + Vec3::Z * 0.05),
            dot(centre + Vec3::Z * 0.01),
            Instance::HIDDEN,
            dot(-towards_camera * 1.1),
        ];
        let cursor = Vec2::new(800.0, 500.0);

        assert_eq!(view.pick(&instances, cursor, 20.0), Some(1));
        // Nothing is within one pixel of the cursor.
        assert_eq!(view.pick(&instances, cursor, 1.0), None);
        // The dot behind Earth is never picked, however generous the reach.
        assert_eq!(view.pick(&instances[2..], cursor, 10_000.0), None);
    }
}
