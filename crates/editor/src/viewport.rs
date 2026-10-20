//! Editor viewport: camera, ray-cast picking and gizmo view matrices.
//!
//! The consumer is the editor viewport panel (`shell_ui`, behind the `ui`
//! feature) and headless tests here. The data owner is [`Viewport`]: a pixel
//! [`ViewportSize`] plus a [`Camera`]. Units follow `engine-scene`: meters
//! for positions, radians for angles (field of view is stored in degrees for
//! readability and converted at the math boundary).
//!
//! [`Viewport::pick`] casts a ray from a pixel against [`SceneRow`] snapshots
//! and returns the nearest hit. It is pure math over snapshot data: no window,
//! no GPU, deterministic row order (ties keep the first row). Each row counts
//! as a sphere centered on its *world* position (rebuilt from the rows'
//! parent links, so nested nodes pick where they are drawn) with radius half
//! the longest world-scale axis; rows with a non-positive or non-finite
//! radius are not pickable.
//!
//! The windowed present loop (swapchain, `egui-wgpu`/`egui-winit` event pump,
//! per-frame capture) stays with the window host: this module only models the camera
//! and the pick ray, plus the view/projection matrices the gizmo session
//! needs. Nothing here opens a window or touches the GPU.

use std::fmt;

use engine_scene::ids::ObjectId;
use engine_scene::transform::Hierarchy;

use crate::shell::SceneRow;

/// Largest accepted viewport extent in pixels; bounds garbage input.
pub const MAX_VIEWPORT_PX: f32 = 16384.0;
/// Near plane in meters for the gizmo projection matrix.
pub const VIEW_NEAR_M: f64 = 0.05;
/// Far plane in meters for the gizmo projection matrix.
pub const VIEW_FAR_M: f64 = 500.0;
/// Smallest pickable sphere radius in meters; thinner nodes are skipped.
pub const MIN_PICK_RADIUS_M: f32 = 1e-6;
/// Closest the orbit camera may sit to its target, in meters.
pub const MIN_CAMERA_DISTANCE_M: f32 = 0.25;
/// Farthest the orbit camera may sit from its target, in meters.
pub const MAX_CAMERA_DISTANCE_M: f32 = 200.0;
/// Pitch limit (about 83 degrees) so the view never aligns with `up`.
pub const MAX_CAMERA_PITCH_RAD: f32 = 1.45;

/// Pixel extent of the viewport panel.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViewportSize {
    /// Width in pixels, always finite and positive.
    pub width: f32,
    /// Height in pixels, always finite and positive.
    pub height: f32,
}

/// Why [`ViewportSize::new`] refused an extent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewportSizeError {
    /// Width or height is NaN or infinite.
    NotFinite,
    /// Width or height is zero or negative.
    NotPositive,
    /// Width or height is over [`MAX_VIEWPORT_PX`].
    TooLarge,
}

impl fmt::Display for ViewportSizeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::NotFinite => "viewport size must be finite",
            Self::NotPositive => "viewport size must be positive",
            Self::TooLarge => "viewport size too large",
        })
    }
}

impl std::error::Error for ViewportSizeError {}

impl ViewportSize {
    /// Build a size. Err on non-finite, non-positive or absurd extents.
    pub fn new(width: f32, height: f32) -> Result<Self, ViewportSizeError> {
        if !width.is_finite() || !height.is_finite() {
            return Err(ViewportSizeError::NotFinite);
        }
        if width <= 0.0 || height <= 0.0 {
            return Err(ViewportSizeError::NotPositive);
        }
        if width > MAX_VIEWPORT_PX || height > MAX_VIEWPORT_PX {
            return Err(ViewportSizeError::TooLarge);
        }
        Ok(Self { width, height })
    }

    /// Width over height; always finite and positive for a valid size.
    pub fn aspect(self) -> f32 {
        self.width / self.height
    }
}

/// Pinhole viewport camera: right-handed, +Y up, meters.
///
/// The camera sits at `position` and looks at `target`; `up` fixes the roll.
/// The projection is a symmetric perspective with vertical field of view
/// `fov_y_deg`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    /// Eye point in meters (world frame).
    pub position: [f32; 3],
    /// Look-at point in meters (world frame).
    pub target: [f32; 3],
    /// World up hint; must not be parallel to the view direction.
    pub up: [f32; 3],
    /// Vertical field of view in degrees, valid in `[1, 179]`.
    pub fov_y_deg: f32,
}

impl Camera {
    /// Default framing: eye above and in front of the origin, looking at it.
    pub fn default_frame() -> Self {
        Self {
            position: [0.0, 1.5, 5.0],
            target: [0.0, 0.0, 0.0],
            up: [0.0, 1.0, 0.0],
            fov_y_deg: 60.0,
        }
    }

    /// True when every field is finite and the field of view is sane.
    /// Coincidence/parallelism is checked where the basis is built.
    pub fn is_well_formed(self) -> bool {
        self.position.iter().all(|c| c.is_finite())
            && self.target.iter().all(|c| c.is_finite())
            && self.up.iter().all(|c| c.is_finite())
            && self.fov_y_deg.is_finite()
            && (1.0..=179.0).contains(&self.fov_y_deg)
    }

    /// Orthonormal basis `(right, cam_up, forward)` plus the eye point.
    /// None when the eye matches the target or `up` is parallel to the view.
    fn basis(self) -> Option<([f32; 3], [f32; 3], [f32; 3])> {
        if !self.is_well_formed() {
            return None;
        }
        let forward = normalize(sub(self.target, self.position))?;
        let right = normalize(engine_core::vec3::cross3(forward, self.up))?;
        let cam_up = engine_core::vec3::cross3(right, forward);
        Some((right, cam_up, forward))
    }

    /// Row-major look-at view matrix (right-handed, camera looks along -Z).
    /// None when the camera basis is degenerate.
    pub fn view_matrix_rows(self) -> Option<[[f64; 4]; 4]> {
        let (right, cam_up, forward) = self.basis()?;
        let p = self.position.map(f64::from);
        let r = right.map(f64::from);
        let u = cam_up.map(f64::from);
        let f = forward.map(f64::from);
        Some([
            [r[0], r[1], r[2], -(r[0] * p[0] + r[1] * p[1] + r[2] * p[2])],
            [u[0], u[1], u[2], -(u[0] * p[0] + u[1] * p[1] + u[2] * p[2])],
            [-f[0], -f[1], -f[2], f[0] * p[0] + f[1] * p[1] + f[2] * p[2]],
            [0.0, 0.0, 0.0, 1.0],
        ])
    }

    /// Row-major symmetric perspective matrix (OpenGL-style NDC `z` in
    /// `[-1, 1]`). `aspect` is width over height. None on bad inputs.
    pub fn projection_matrix_rows(self, aspect: f64) -> Option<[[f64; 4]; 4]> {
        if !self.is_well_formed() || !aspect.is_finite() || aspect <= 0.0 {
            return None;
        }
        let half = (self.fov_y_deg as f64).to_radians() / 2.0;
        let f = 1.0 / half.tan();
        if !f.is_finite() || f <= 0.0 {
            return None;
        }
        let near = VIEW_NEAR_M;
        let far = VIEW_FAR_M;
        Some([
            [f / aspect, 0.0, 0.0, 0.0],
            [0.0, f, 0.0, 0.0],
            [
                0.0,
                0.0,
                (far + near) / (near - far),
                (2.0 * far * near) / (near - far),
            ],
            [0.0, 0.0, -1.0, 0.0],
        ])
    }

    /// Distance from the eye to the look-at target, in meters. Zero for a
    /// degenerate camera; orbit and zoom refuse in that case.
    pub fn distance_to_target(self) -> f32 {
        let offset = sub(self.position, self.target);
        dot(offset, offset).sqrt()
    }

    /// Orbit around the target by `dyaw` and `dpitch` radians. The target,
    /// up hint and field of view stay put; pitch is clamped to
    /// [`MAX_CAMERA_PITCH_RAD`] and the distance is preserved. Non-finite
    /// input or a degenerate camera returns `self`.
    pub fn orbit(self, dyaw: f32, dpitch: f32) -> Self {
        if !dyaw.is_finite() || !dpitch.is_finite() {
            return self;
        }
        let offset = sub(self.position, self.target);
        let distance = dot(offset, offset).sqrt();
        if distance.is_nan() || distance <= 1e-4 {
            return self;
        }
        let yaw = offset[0].atan2(offset[2]) + dyaw;
        let pitch = (offset[1] / distance).clamp(-1.0, 1.0).asin() + dpitch;
        let pitch = pitch.clamp(-MAX_CAMERA_PITCH_RAD, MAX_CAMERA_PITCH_RAD);
        let (sin_pitch, cos_pitch) = pitch.sin_cos();
        Self {
            position: [
                self.target[0] + distance * cos_pitch * yaw.sin(),
                self.target[1] + distance * sin_pitch,
                self.target[2] + distance * cos_pitch * yaw.cos(),
            ],
            ..self
        }
    }

    /// Pan in screen units: `right_m` along the camera right, `up_m` along
    /// the camera up, both in meters. Eye and target move together, so the
    /// view direction and distance do not change.
    pub fn pan(self, right_m: f32, up_m: f32) -> Self {
        if !right_m.is_finite() || !up_m.is_finite() {
            return self;
        }
        let Some((right, up, _)) = self.basis() else {
            return self;
        };
        let shift = [
            right[0] * right_m + up[0] * up_m,
            right[1] * right_m + up[1] * up_m,
            right[2] * right_m + up[2] * up_m,
        ];
        Self {
            position: [
                self.position[0] + shift[0],
                self.position[1] + shift[1],
                self.position[2] + shift[2],
            ],
            target: [
                self.target[0] + shift[0],
                self.target[1] + shift[1],
                self.target[2] + shift[2],
            ],
            ..self
        }
    }

    /// Dolly toward or away from the target by `factor` (> 1 moves away),
    /// clamped to [`MIN_CAMERA_DISTANCE_M`]..=[`MAX_CAMERA_DISTANCE_M`].
    /// Bad input or a degenerate camera returns `self`.
    pub fn zoom(self, factor: f32) -> Self {
        if !factor.is_finite() || factor <= 0.0 {
            return self;
        }
        let offset = sub(self.position, self.target);
        let distance = dot(offset, offset).sqrt();
        if distance.is_nan() || distance <= 1e-4 {
            return self;
        }
        let new = (distance * factor).clamp(MIN_CAMERA_DISTANCE_M, MAX_CAMERA_DISTANCE_M);
        let scale = new / distance;
        Self {
            position: [
                self.target[0] + offset[0] * scale,
                self.target[1] + offset[1] * scale,
                self.target[2] + offset[2] * scale,
            ],
            ..self
        }
    }

    /// Frame a sphere: keep the view direction, move the target to `center`
    /// and back the eye off until a sphere of `radius` fits the vertical
    /// field of view. A radius below 0.5 m is raised to 0.5 m so a single
    /// node fills a sane part of the panel, not the whole screen.
    pub fn frame(self, center: [f32; 3], radius: f32) -> Self {
        if !center.iter().all(|c| c.is_finite()) || !radius.is_finite() {
            return self;
        }
        let forward = normalize(sub(self.target, self.position)).unwrap_or([0.0, 0.0, -1.0]);
        let radius = radius.max(0.5);
        let half = self.fov_y_deg.to_radians() * 0.5;
        let distance = (radius / half.sin().max(1e-3) * 1.25)
            .clamp(MIN_CAMERA_DISTANCE_M, MAX_CAMERA_DISTANCE_M);
        Self {
            target: center,
            position: [
                center[0] - forward[0] * distance,
                center[1] - forward[1] * distance,
                center[2] - forward[2] * distance,
            ],
            ..self
        }
    }
}

impl Default for Camera {
    fn default() -> Self {
        Self::default_frame()
    }
}

/// Normalized world-space ray: an eye point plus a unit direction.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ray {
    /// Eye point in meters.
    pub origin: [f32; 3],
    /// Unit direction (world frame).
    pub dir: [f32; 3],
}

impl Ray {
    /// Distance to a sphere, if the ray hits it. Inside-origin counts as a
    /// hit at distance zero. Non-finite or non-positive radii never hit.
    pub fn hit_sphere(self, center: [f32; 3], radius: f32) -> Option<f32> {
        if !radius.is_finite() || radius <= MIN_PICK_RADIUS_M {
            return None;
        }
        if !center.iter().all(|c| c.is_finite()) {
            return None;
        }
        let oc = sub(center, self.origin);
        let tca = dot(oc, self.dir);
        let disc = tca * tca - dot(oc, oc) + radius * radius;
        if !disc.is_finite() || disc < 0.0 {
            return None;
        }
        let half = disc.sqrt();
        let enter = tca - half;
        if enter >= 0.0 {
            return Some(enter);
        }
        if tca + half >= 0.0 {
            return Some(0.0);
        }
        None
    }

    /// Point where the ray meets the y=0 ground plane ahead of the eye.
    /// None when the ray is parallel to the plane or points away from it.
    pub fn hit_ground(self) -> Option<[f32; 3]> {
        if !self.dir[1].is_finite() || self.dir[1].abs() < 1e-6 {
            return None;
        }
        let t = -self.origin[1] / self.dir[1];
        if !t.is_finite() || t <= 0.0 {
            return None;
        }
        Some([
            self.origin[0] + self.dir[0] * t,
            0.0,
            self.origin[2] + self.dir[2] * t,
        ])
    }
}

/// Viewport state: pixel size plus camera. Owned by [`Shell`](crate::shell::Shell).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Viewport {
    /// Panel extent in pixels.
    pub size: ViewportSize,
    /// View camera.
    pub camera: Camera,
}

impl Viewport {
    /// 800x600 panel with the default framing.
    pub fn default_panel() -> Self {
        Self {
            // Validated literals; any failure would be a code bug.
            size: ViewportSize::new(800.0, 600.0).expect("default viewport size is valid"),
            camera: Camera::default_frame(),
        }
    }

    /// Ray through pixel `(x, y)` (origin top-left, units pixels). None for
    /// out-of-panel pixels or a degenerate camera.
    pub fn ray_for_pixel(self, x: f32, y: f32) -> Option<Ray> {
        if !x.is_finite() || !y.is_finite() {
            return None;
        }
        if x < 0.0 || y < 0.0 || x > self.size.width || y > self.size.height {
            return None;
        }
        let (right, cam_up, forward) = self.camera.basis()?;
        let half = (self.camera.fov_y_deg as f64).to_radians() / 2.0;
        let tan_half = half.tan() as f32;
        if !tan_half.is_finite() || tan_half <= 0.0 {
            return None;
        }
        let ndc_x = (2.0 * x / self.size.width - 1.0) * tan_half * self.size.aspect();
        let ndc_y = (1.0 - 2.0 * y / self.size.height) * tan_half;
        let dir = normalize([
            right[0] * ndc_x + cam_up[0] * ndc_y + forward[0],
            right[1] * ndc_x + cam_up[1] * ndc_y + forward[1],
            right[2] * ndc_x + cam_up[2] * ndc_y + forward[2],
        ])?;
        Some(Ray {
            origin: self.camera.position,
            dir,
        })
    }

    /// Ground point under pixel `(x, y)`; None off-panel, on a degenerate
    /// camera, or when the ray never meets y=0 ahead of the eye.
    pub fn ground_at(self, x: f32, y: f32) -> Option<[f32; 3]> {
        self.ray_for_pixel(x, y)?.hit_ground()
    }

    /// Nearest row hit by the pick ray, or None on a miss. Rows keep their
    /// snapshot order and win ties, so the result is deterministic for the
    /// same rows. World positions rebuild from the rows' own parent links,
    /// so nested nodes pick where they stand.
    pub fn pick(self, x: f32, y: f32, rows: &[SceneRow]) -> Option<ObjectId> {
        let ray = self.ray_for_pixel(x, y)?;
        // Rebuild world poses from the snapshot alone so picking needs no
        // scene handle; corrupt rows simply resolve to None and are skipped.
        let mut tree = Hierarchy::new();
        for row in rows {
            let _ = tree.insert(row.id, row.parent, row.local);
        }
        let mut best: Option<(ObjectId, f32)> = None;
        for row in rows {
            let Some(world) = tree.world(row.id) else {
                continue;
            };
            let longest = world.scale[0]
                .abs()
                .max(world.scale[1].abs())
                .max(world.scale[2].abs());
            let radius = 0.5 * longest;
            let Some(hit) = ray.hit_sphere(world.position, radius) else {
                continue;
            };
            let wins = match best {
                Some((_, depth)) => hit < depth,
                None => true,
            };
            if wins {
                best = Some((row.id, hit));
            }
        }
        best.map(|(id, _)| id)
    }
}

impl Default for Viewport {
    fn default() -> Self {
        Self::default_panel()
    }
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// Unit vector, or None for zero/non-finite input.
fn normalize(v: [f32; 3]) -> Option<[f32; 3]> {
    let len_sq = dot(v, v);
    if !len_sq.is_finite() || len_sq < 1e-12 {
        return None;
    }
    let len = len_sq.sqrt();
    Some([v[0] / len, v[1] / len, v[2] / len])
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_scene::ids::ObjectId;
    use engine_scene::transform::Transform;

    fn pose_at(p: [f32; 3]) -> Transform {
        Transform::new(p, [0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0])
    }

    fn rows_for(world: &[(ObjectId, [f32; 3])]) -> Vec<SceneRow> {
        world
            .iter()
            .map(|(id, p)| SceneRow {
                id: *id,
                parent: ObjectId::NIL,
                depth: 0,
                local: pose_at(*p),
            })
            .collect()
    }

    fn center(view: Viewport) -> (f32, f32) {
        (view.size.width / 2.0, view.size.height / 2.0)
    }

    #[test]
    fn ray_meets_the_ground_only_ahead() {
        let down = Ray {
            origin: [2.0, 3.0, -1.0],
            dir: [0.0, -1.0, 0.0],
        };
        assert_eq!(down.hit_ground(), Some([2.0, 0.0, -1.0]));
        let parallel = Ray {
            origin: [0.0, 3.0, 0.0],
            dir: [1.0, 0.0, 0.0],
        };
        assert_eq!(parallel.hit_ground(), None);
        let up = Ray {
            origin: [0.0, 3.0, 0.0],
            dir: [0.0, 1.0, 0.0],
        };
        assert_eq!(up.hit_ground(), None);
    }

    #[test]
    fn pick_center_hits_node_at_target() {
        let view = Viewport::default_panel();
        let id = ObjectId::new();
        let rows = rows_for(&[(id, [0.0, 0.0, 0.0])]);
        let (x, y) = center(view);
        assert_eq!(view.pick(x, y, &rows), Some(id));
    }

    #[test]
    fn pick_empty_scene_misses() {
        let view = Viewport::default_panel();
        let (x, y) = center(view);
        assert_eq!(view.pick(x, y, &[]), None);
        // A node far off the view axis is a miss too.
        let id = ObjectId::new();
        let rows = rows_for(&[(id, [40.0, 0.0, 0.0])]);
        assert_eq!(view.pick(x, y, &rows), None);
    }

    #[test]
    fn pick_nearest_wins_and_order_breaks_ties() {
        let view = Viewport::default_panel();
        let near = ObjectId::new();
        let far = ObjectId::new();
        // Both on the view axis: the nearer sphere wins regardless of order.
        let rows = rows_for(&[(far, [0.0, 0.0, -2.0]), (near, [0.0, 0.75, 2.5])]);
        let (x, y) = center(view);
        assert_eq!(view.pick(x, y, &rows), Some(near));
        let flipped = rows_for(&[(near, [0.0, 0.75, 2.5]), (far, [0.0, 0.0, -2.0])]);
        assert_eq!(view.pick(x, y, &flipped), Some(near));
        // Same spot twice: the first row wins, deterministically.
        let twin = ObjectId::new();
        let tied = rows_for(&[(near, [0.0, 0.0, 0.0]), (twin, [0.0, 0.0, 0.0])]);
        assert_eq!(view.pick(x, y, &tied), Some(near));
    }

    #[test]
    fn pick_uses_world_pose_for_nested_rows() {
        let view = Viewport::default_panel();
        let root = ObjectId::new();
        let child = ObjectId::new();
        // Root far off-axis, child offset back onto the view axis in world.
        let rows = vec![
            SceneRow {
                id: root,
                parent: ObjectId::NIL,
                depth: 0,
                local: pose_at([40.0, 0.0, 0.0]),
            },
            SceneRow {
                id: child,
                parent: root,
                depth: 1,
                local: pose_at([-40.0, 0.0, 0.0]),
            },
        ];
        let (x, y) = center(view);
        assert_eq!(view.pick(x, y, &rows), Some(child));
    }

    #[test]
    fn pick_rejects_bad_pixels_and_cameras() {
        let view = Viewport::default_panel();
        let id = ObjectId::new();
        let rows = rows_for(&[(id, [0.0, 0.0, 0.0])]);
        for (x, y) in [
            (f32::NAN, 300.0),
            (400.0, f32::INFINITY),
            (-1.0, 300.0),
            (400.0, 601.0),
            (801.0, 300.0),
        ] {
            assert_eq!(view.pick(x, y, &rows), None, "pixel ({x}, {y})");
        }
        // Degenerate cameras never produce a ray.
        let mut bad = view;
        bad.camera.position = bad.camera.target;
        let (x, y) = center(view);
        assert_eq!(bad.pick(x, y, &rows), None);
        let mut tilted = view;
        tilted.camera.up = [0.0, 0.0, 0.0];
        assert_eq!(tilted.pick(x, y, &rows), None);
        let mut narrow = view;
        narrow.camera.fov_y_deg = 0.0;
        assert_eq!(narrow.pick(x, y, &rows), None);
    }

    #[test]
    fn node_behind_camera_is_a_miss() {
        let view = Viewport::default_panel();
        let id = ObjectId::new();
        let rows = rows_for(&[(id, [0.0, 1.5, 12.0])]);
        let (x, y) = center(view);
        assert_eq!(view.pick(x, y, &rows), None);
    }

    #[test]
    fn degenerate_scale_is_not_pickable() {
        let view = Viewport::default_panel();
        let id = ObjectId::new();
        let flat = Transform::new([0.0, 0.0, 0.0], [0.0, 0.0, 0.0, 1.0], [0.0, 0.0, 0.0]);
        let rows = vec![SceneRow {
            id,
            parent: ObjectId::NIL,
            depth: 0,
            local: flat,
        }];
        let (x, y) = center(view);
        assert_eq!(view.pick(x, y, &rows), None);
    }

    #[test]
    fn size_validation_rejects_garbage() {
        assert!(ViewportSize::new(800.0, 600.0).is_ok());
        assert_eq!(
            ViewportSize::new(0.0, 600.0),
            Err(ViewportSizeError::NotPositive)
        );
        assert_eq!(
            ViewportSize::new(800.0, -1.0),
            Err(ViewportSizeError::NotPositive)
        );
        assert_eq!(
            ViewportSize::new(f32::NAN, 600.0),
            Err(ViewportSizeError::NotFinite)
        );
        assert_eq!(
            ViewportSize::new(1e9, 600.0),
            Err(ViewportSizeError::TooLarge)
        );
    }

    /// Row-major matrix times a homogeneous point.
    fn mat_vec(m: &[[f64; 4]; 4], p: [f64; 3]) -> [f64; 4] {
        [
            m[0][0] * p[0] + m[0][1] * p[1] + m[0][2] * p[2] + m[0][3],
            m[1][0] * p[0] + m[1][1] * p[1] + m[1][2] * p[2] + m[1][3],
            m[2][0] * p[0] + m[2][1] * p[1] + m[2][2] * p[2] + m[2][3],
            m[3][0] * p[0] + m[3][1] * p[1] + m[3][2] * p[2] + m[3][3],
        ]
    }

    #[test]
    fn view_matrix_parks_the_eye_at_origin() {
        let camera = Camera::default_frame();
        let m = camera.view_matrix_rows().expect("default camera is valid");
        let p = camera.position.map(f64::from);
        let eye = mat_vec(&m, p);
        for c in eye[..3].iter() {
            assert!(c.abs() < 1e-9, "eye maps to origin, got {eye:?}");
        }
        assert!(
            (eye[3] - 1.0).abs() < 1e-9,
            "homogeneous w stays 1, got {eye:?}"
        );
        // The target lands on the -Z axis (x and y near zero up to f32
        // dust, z negative).
        let t = camera.target.map(f64::from);
        let aimed = mat_vec(&m, t);
        assert!(aimed[0].abs() < 1e-5 && aimed[1].abs() < 1e-5);
        assert!(aimed[2] < 0.0);
    }

    #[test]
    fn projection_maps_near_plane_center_to_ndc_minus_one() {
        let camera = Camera::default_frame();
        let view = Viewport::default_panel();
        let aspect = f64::from(view.size.aspect());
        let proj = camera
            .projection_matrix_rows(aspect)
            .expect("default camera is valid");
        // Near-plane center in front of the eye along the view direction.
        let fwd = {
            let f = sub(camera.target, camera.position);
            let len = dot(f, f).sqrt();
            [f[0] / len, f[1] / len, f[2] / len]
        };
        let p = [
            f64::from(camera.position[0]) + f64::from(fwd[0]) * VIEW_NEAR_M,
            f64::from(camera.position[1]) + f64::from(fwd[1]) * VIEW_NEAR_M,
            f64::from(camera.position[2]) + f64::from(fwd[2]) * VIEW_NEAR_M,
        ];
        let v = camera.view_matrix_rows().expect("default camera is valid");
        let eye = mat_vec(&v, p);
        let clip = mat_vec(&proj, [eye[0], eye[1], eye[2]]);
        let ndc_z = clip[2] / clip[3];
        assert!(
            (ndc_z + 1.0).abs() < 1e-6,
            "near plane maps to -1, got {ndc_z}"
        );
        assert!(camera.projection_matrix_rows(0.0).is_none());
        assert!(camera.projection_matrix_rows(f64::NAN).is_none());
    }

    #[test]
    fn orbit_keeps_distance_and_clamps_pitch() {
        let camera = Camera::default_frame();
        let before = camera.distance_to_target();
        let orbited = camera.orbit(0.5, 0.1);
        assert_eq!(orbited.target, camera.target);
        assert!((orbited.distance_to_target() - before).abs() < 1e-4);
        assert!((orbited.position[0] - camera.position[0]).abs() > 0.1);
        let top = camera.orbit(0.0, 10.0);
        let offset = sub(top.position, top.target);
        let pitch = (offset[1] / dot(offset, offset).sqrt()).asin();
        assert!((pitch - MAX_CAMERA_PITCH_RAD).abs() < 1e-4);
        assert!(top.is_well_formed());
        assert_eq!(camera.orbit(f32::NAN, 0.0), camera);
    }

    #[test]
    fn zoom_moves_the_eye_and_clamps_distance() {
        let camera = Camera::default_frame();
        let near = camera.zoom(0.5);
        assert!(near.distance_to_target() < camera.distance_to_target());
        assert!((near.distance_to_target() - camera.distance_to_target() * 0.5).abs() < 1e-4);
        let closest = camera.zoom(1e-6);
        assert!((closest.distance_to_target() - MIN_CAMERA_DISTANCE_M).abs() < 1e-4);
        let farthest = camera.zoom(1e6);
        assert!((farthest.distance_to_target() - MAX_CAMERA_DISTANCE_M).abs() < 1e-3);
        assert_eq!(camera.zoom(0.0), camera);
    }

    #[test]
    fn pan_moves_eye_and_target_together() {
        let camera = Camera::default_frame();
        let panned = camera.pan(1.0, 0.5);
        let eye = sub(panned.position, camera.position);
        let target = sub(panned.target, camera.target);
        for axis in 0..3 {
            assert!((eye[axis] - target[axis]).abs() < 1e-5);
        }
        assert!((panned.distance_to_target() - camera.distance_to_target()).abs() < 1e-5);
    }

    #[test]
    fn frame_centers_the_target_and_fits_radius() {
        let camera = Camera::default_frame();
        let framed = camera.frame([4.0, 2.0, -3.0], 2.0);
        assert_eq!(framed.target, [4.0, 2.0, -3.0]);
        assert!(framed.is_well_formed());
        let distance = framed.distance_to_target();
        let half = framed.fov_y_deg.to_radians() * 0.5;
        assert!(distance >= 2.0 / half.sin());
        assert!(distance < 2.0 / half.sin() * 1.5);
        let before = normalize(sub(camera.target, camera.position)).expect("valid camera");
        let after = normalize(sub(framed.target, framed.position)).expect("valid camera");
        for axis in 0..3 {
            assert!((before[axis] - after[axis]).abs() < 1e-5);
        }
    }

    #[test]
    fn camera_controls_refuse_garbage() {
        let camera = Camera::default_frame();
        assert_eq!(camera.orbit(f32::NAN, 0.0), camera);
        assert_eq!(camera.pan(f32::INFINITY, 0.0), camera);
        assert_eq!(camera.zoom(f32::NAN), camera);
        assert_eq!(camera.frame([f32::NAN, 0.0, 0.0], 1.0), camera);
        let degenerate = Camera {
            position: [1.0, 1.0, 1.0],
            target: [1.0, 1.0, 1.0],
            ..camera
        };
        assert_eq!(degenerate.orbit(0.1, 0.1), degenerate);
        assert_eq!(degenerate.zoom(0.5), degenerate);
    }
}
