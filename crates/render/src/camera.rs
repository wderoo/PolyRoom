use glam::{Mat4, Vec3};
use polyroom_mesh::Aabb;

/// An orbit camera around `target`, Z-up like a printer's build plate.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Camera {
    /// Rotation around Z in radians; -90° looks at the model's front (from −Y).
    pub yaw: f32,
    /// Elevation in radians, clamped to just under ±90°.
    pub pitch: f32,
    pub distance: f32,
    pub target: Vec3,
    /// Vertical field of view in radians.
    pub fov_y: f32,
    /// Model radius, used to bound zoom and clip planes.
    pub radius: f32,
}

impl Camera {
    pub const DEFAULT_YAW: f32 = -60.0_f32.to_radians();
    pub const DEFAULT_PITCH: f32 = 25.0_f32.to_radians();
    const PITCH_LIMIT: f32 = 89.0_f32.to_radians();

    /// A three-quarter view framing the whole bounding box.
    pub fn fit(bounds: &Aabb, aspect: f32) -> Self {
        let mut cam = Self {
            yaw: Self::DEFAULT_YAW,
            pitch: Self::DEFAULT_PITCH,
            distance: 1.0,
            target: bounds.center(),
            fov_y: 30.0_f32.to_radians(),
            radius: bounds.radius().max(1e-3),
        };
        cam.distance = cam.fit_distance(aspect);
        cam
    }

    /// Distance at which the bounding sphere fits the narrower field of view.
    pub fn fit_distance(&self, aspect: f32) -> f32 {
        let half_y = self.fov_y * 0.5;
        let half_x = (half_y.tan() * aspect.max(1e-3)).atan();
        self.radius / half_y.min(half_x).sin() * 1.02
    }

    pub fn forward(&self) -> Vec3 {
        -Vec3::new(
            self.pitch.cos() * self.yaw.cos(),
            self.pitch.cos() * self.yaw.sin(),
            self.pitch.sin(),
        )
    }

    pub fn eye(&self) -> Vec3 {
        self.target - self.forward() * self.distance
    }

    pub fn right(&self) -> Vec3 {
        self.forward().cross(Vec3::Z).normalize_or(Vec3::X)
    }

    pub fn up(&self) -> Vec3 {
        self.right().cross(self.forward())
    }

    /// Turns the camera to look at the target from the side `axis` points to (e.g. `[0, 0, 1]`
    /// is a top view). Straight up or down keeps the current heading.
    pub fn look_along(&mut self, axis: [f32; 3]) {
        let a = Vec3::from(axis).normalize_or(Vec3::ZERO);
        if a == Vec3::ZERO {
            return;
        }
        self.pitch = a.z.asin().clamp(-Self::PITCH_LIMIT, Self::PITCH_LIMIT);
        if a.x.abs() > 1e-4 || a.y.abs() > 1e-4 {
            self.yaw = a.y.atan2(a.x);
        }
    }

    /// A world direction in camera terms: `[right, up, away]` components. Negative `away`
    /// points towards the viewer.
    pub fn view_dir(&self, dir: [f32; 3]) -> [f32; 3] {
        let d = Vec3::from(dir);
        [d.dot(self.right()), d.dot(self.up()), d.dot(self.forward())]
    }

    /// Orbit by a drag of `dx`, `dy` logical pixels.
    pub fn orbit(&mut self, dx: f32, dy: f32) {
        self.yaw -= dx * 0.008;
        self.pitch = (self.pitch + dy * 0.008).clamp(-Self::PITCH_LIMIT, Self::PITCH_LIMIT);
    }

    /// Pan by a drag of `dx`, `dy` pixels in a viewport `height` pixels tall.
    pub fn pan(&mut self, dx: f32, dy: f32, height: f32) {
        let per_pixel = 2.0 * self.distance * (self.fov_y * 0.5).tan() / height.max(1.0);
        self.target += (-self.right() * dx + self.up() * dy) * per_pixel;
    }

    /// Multiplies the distance by `factor` (< 1 zooms in).
    pub fn zoom(&mut self, factor: f32) {
        if factor.is_finite() && factor > 0.0 {
            self.distance = (self.distance * factor).clamp(self.radius * 0.05, self.radius * 50.0);
        }
    }

    pub fn view_proj(&self, aspect: f32) -> Mat4 {
        let view = glam::camera::rh::view::look_at_mat4(self.eye(), self.target, Vec3::Z);
        let near = (self.distance - self.radius * 2.0).max(self.distance * 0.01);
        let far = self.distance + self.radius * 4.0;
        glam::camera::rh::proj::directx::perspective(self.fov_y, aspect.max(1e-3), near, far) * view
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cube() -> Aabb {
        Aabb {
            min: [0.0; 3],
            max: [10.0; 3],
        }
    }

    #[test]
    fn fit_keeps_all_corners_on_screen() {
        for aspect in [0.5, 1.0, 2.5] {
            let cam = Camera::fit(&cube(), aspect);
            let vp = cam.view_proj(aspect);
            for i in 0..8 {
                let c = Vec3::new(
                    if i & 1 == 0 { 0.0 } else { 10.0 },
                    if i & 2 == 0 { 0.0 } else { 10.0 },
                    if i & 4 == 0 { 0.0 } else { 10.0 },
                );
                let p = vp.project_point3(c);
                assert!(p.x.abs() <= 1.0 && p.y.abs() <= 1.0, "{aspect}: {p}");
                assert!((0.0..=1.0).contains(&p.z));
            }
        }
    }

    #[test]
    fn orbit_clamps_pitch_and_basis_stays_orthonormal() {
        let mut cam = Camera::fit(&cube(), 1.0);
        cam.orbit(123.0, 10_000.0);
        assert!(cam.pitch < std::f32::consts::FRAC_PI_2);
        assert!(cam.right().dot(cam.up()).abs() < 1e-5);
        assert!((cam.up().length() - 1.0).abs() < 1e-5);
    }

    #[test]
    fn look_along_puts_the_eye_on_that_side() {
        let mut cam = Camera::fit(&cube(), 1.0);
        for axis in [
            [1.0, 0.0, 0.0],
            [0.0, -1.0, 0.0],
            [-1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
        ] {
            cam.look_along(axis);
            let dir = (cam.eye() - cam.target).normalize();
            assert!(dir.dot(Vec3::from(axis)) > 0.999, "{axis:?}: {dir}");
            assert!(cam.view_dir(axis)[2] < -0.999); // towards the viewer
        }
        // Top and bottom keep the heading and stay just short of the pole.
        cam.look_along([0.0, -1.0, 0.0]);
        let yaw = cam.yaw;
        cam.look_along([0.0, 0.0, 1.0]);
        assert_eq!(cam.yaw, yaw);
        assert!(cam.view_dir([0.0, 0.0, 1.0])[2] < -0.99);
        cam.look_along([0.0, 0.0, -1.0]);
        assert!(cam.eye().z < cam.target.z);
        cam.look_along([0.0, 0.0, 0.0]); // no direction: unchanged
        assert!(cam.eye().z < cam.target.z);
    }

    #[test]
    fn zoom_is_bounded() {
        let mut cam = Camera::fit(&cube(), 1.0);
        cam.zoom(1e-9);
        assert!(cam.distance >= cam.radius * 0.05);
        cam.zoom(f32::NAN);
        assert!(cam.distance.is_finite());
    }
}
