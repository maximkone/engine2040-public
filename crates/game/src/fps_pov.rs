//! fps_pov: the legionary's first person, played well (row POV2 and the look audit of
//! 2026-10-24, defect 2): the eye at the real eye point of the head joint riding a halflife
//! spring, the aim ray that never rides it, a spawn that looks into the arena, a centre reticle
//! and the two view laws that say whether the picture is playable.
//! Owner: engine-game. Units: meters, radians, seconds; plain arrays, no GPU.
//! Determinism: presentation only, never hashed; the same tape gives the same bytes twice.
use super::{legionary_eye, RenderEye};
use crate::camera::camera_rig::{damper_step, halflife_to_damping};
use crate::camera::CameraError;
use engine_core::vec3::cross3;
use std::f32::consts::PI;

/// Halflife of the helmet mount, seconds (`docs/plan/touch-and-pov.md` section 4, table).
pub const HELMET_HALFLIFE_S: f32 = 0.06;
/// Halflife of the chest mount, seconds: the chest is the looser mount.
pub const CHEST_HALFLIFE_S: f32 = 0.10;
/// Near plane of the first-person camera, meters. The wearer's head is hidden, so the plane no
/// longer has to stand past it: a wall a hand away is a wall, not a hole.
pub const EYE_NEAR_M: f32 = 0.08;
/// A mount that jumps farther than this in one frame is a respawn or a rope projection, not a
/// bob: the spring snaps instead of gliding across the pit.
const SNAP_M: f32 = 1.0;
/// The spawn eye needs at least this much open floor ahead of it, meters, or it turns to the pit
/// centre.
pub const WALL_CLEAR_M: f32 = 6.0;
/// Own geometry covering more of the picture than this share fails the frame.
pub const COVER_MAX: f32 = 0.30;
/// Own geometry on the screen centre for more than this many frames in a row fails the run
/// (0.6 s at 60 Hz: a swing crosses the reticle in a blink, a shield held up does not).
pub const CENTRE_FRAMES_MAX: u32 = 36;
/// Cover grid across the picture; one cell is `1 / (32 * 18)` of it.
const GRID_W: usize = 32;
/// Cover grid down the picture.
const GRID_H: usize = 18;

/// Where the camera is mounted on the wearer.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Mount {
    /// On the helmet, at the eye point of the head joint (the default view).
    #[default]
    Helmet,
    /// On the chest, a body camera on the torso joint.
    Chest,
}

impl Mount {
    /// Halflife of the spring this mount rides, seconds.
    #[must_use]
    pub const fn halflife_s(self) -> f32 {
        match self {
            Self::Helmet => HELMET_HALFLIFE_S,
            Self::Chest => CHEST_HALFLIFE_S,
        }
    }
}

/// The wearer's body at one pose, in world meters: where the root stands, the two mount points
/// and the posed vertices the view laws read. Built by the fighter look from the skinned pose,
/// so the camera and the drawn body are one pose.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct PovBody {
    /// The drawn root (feet), lift included.
    pub root_m: [f32; 3],
    /// The eye point of the head joint: the head's own eye, rigidly on the posed head.
    pub head_eye_m: [f32; 3],
    /// The chest camera point: on the torso joint, in front of the breastbone.
    pub chest_cam_m: [f32; 3],
    /// Every posed vertex of the wearer's drawn parts, part after part.
    pub positions: Vec<[f32; 3]>,
}

impl PovBody {
    /// The rigid mount point of `mount`, world meters.
    #[must_use]
    pub fn mount_m(&self, mount: Mount) -> [f32; 3] {
        match mount {
            Mount::Helmet => self.head_eye_m,
            Mount::Chest => self.chest_cam_m,
        }
    }
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn length(a: [f32; 3]) -> f32 {
    dot(a, a).sqrt()
}

fn normalized(a: [f32; 3]) -> Option<[f32; 3]> {
    let len = length(a);
    (len.is_finite() && len > 1e-9).then(|| [a[0] / len, a[1] / len, a[2] / len])
}

fn finite3(a: [f32; 3]) -> bool {
    a.iter().all(|lane| lane.is_finite())
}

/// The camera mount's spring: the mount point, relative to the root, low-passed through the
/// fight camera's own halflife damper (`camera_rig::damper_step`, no new spring type). The
/// root's own travel never enters it, so a run does not drag the picture behind the body.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct MountSpring {
    damping: f32,
    held: Option<[f32; 3]>,
}

impl MountSpring {
    /// A spring at `mount`'s halflife, empty until its first frame, which snaps.
    /// Errors: [`CameraError::BadParam`] for a bad halflife (never for the shipped two).
    pub fn new(mount: Mount) -> Result<Self, CameraError> {
        Ok(Self {
            damping: halflife_to_damping(mount.halflife_s())?,
            held: None,
        })
    }

    /// Fold one frame: the mount point relative to the root, `dt_s` seconds after the last frame.
    /// Returns the smoothed offset. A stalled clock (`dt_s` not positive) holds the last value;
    /// a jump past [`SNAP_M`] snaps.
    /// Errors: [`CameraError::BadParam`] naming `POV2 mount` for a non-finite offset.
    pub fn step(&mut self, rel_m: [f32; 3], dt_s: f32) -> Result<[f32; 3], CameraError> {
        if !finite3(rel_m) {
            return Err(CameraError::BadParam("POV2 mount"));
        }
        let Some(held) = self.held else {
            self.held = Some(rel_m);
            return Ok(rel_m);
        };
        if !dt_s.is_finite() || dt_s <= 0.0 {
            return Ok(held);
        }
        if length(sub(rel_m, held)) > SNAP_M {
            self.held = Some(rel_m);
            return Ok(rel_m);
        }
        let mut next = held;
        for (lane, value) in next.iter_mut().enumerate() {
            *value = damper_step(held[lane], rel_m[lane], self.damping, dt_s)?;
        }
        self.held = Some(next);
        Ok(next)
    }

    /// Forget the held offset: the next frame snaps (a toggle back from the third person).
    pub fn reset(&mut self) {
        self.held = None;
    }
}

/// One first-person frame: the picture's eye, the aim ray's eye and how far the picture sits off
/// the rigid mount.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct PovEye {
    /// The eye the picture is taken from: the sprung mount point plus the eye-space trauma kick.
    pub view: RenderEye,
    /// The aim ray's eye (FV1: feet plus eye height, the facing yaw, no kick): the swing, the
    /// throw and the lock-on read this one, so the view shakes and the aim does not.
    pub aim: RenderEye,
    /// How far the sprung picture sits off the rigid mount point, meters: the shake the mount
    /// adds. Zero for a still head; grows with the mount's halflife.
    pub lag_m: f32,
}

/// The first-person rig: the mount and the spring it rides. One per window.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct PovRig {
    mount: Mount,
    spring: MountSpring,
}

impl PovRig {
    /// A rig on `mount`. Errors: none for the shipped mounts.
    pub fn new(mount: Mount) -> Result<Self, CameraError> {
        Ok(Self {
            mount,
            spring: MountSpring::new(mount)?,
        })
    }

    /// The mount this rig rides.
    #[must_use]
    pub const fn mount(&self) -> Mount {
        self.mount
    }

    /// Forget the spring: the next frame snaps to the mount.
    pub fn reset(&mut self) {
        self.spring.reset();
    }

    /// One frame of the first person. `feet_m` and `yaw_rad` are the sim's (FV1); `body` is the
    /// wearer's posed body when the look has one, else the picture takes the FV1 eye. The
    /// picture's eye is `root + spring(mount - root)`: the head's real eye point, smoothed.
    /// Units: meters, radians, `trauma_permille` permille, `dt_s` seconds since the last frame.
    /// Errors: [`CameraError::BadParam`] for a non-finite position, facing or mount.
    /// Determinism: the same sequence of calls gives the same bytes twice; zero heap.
    pub fn eye(
        &mut self,
        feet_m: [f32; 3],
        yaw_rad: f32,
        body: Option<&PovBody>,
        trauma_permille: u32,
        tick: u32,
        dt_s: f32,
    ) -> Result<PovEye, CameraError> {
        let aim = legionary_eye(feet_m, yaw_rad, 0, tick)?;
        let shaken = legionary_eye(feet_m, yaw_rad, trauma_permille, tick)?;
        let kick_mm = [
            shaken.eye_mm[0] - aim.eye_mm[0],
            shaken.eye_mm[1] - aim.eye_mm[1],
            shaken.eye_mm[2] - aim.eye_mm[2],
        ];
        let mut view = RenderEye {
            roll_rad: shaken.roll_rad,
            ..aim
        };
        let mut lag_m = 0.0;
        match body {
            Some(body) => {
                let rigid = body.mount_m(self.mount);
                let rel = sub(rigid, body.root_m);
                let held = self.spring.step(rel, dt_s)?;
                for lane in 0..3 {
                    view.eye_mm[lane] = (body.root_m[lane] + held[lane]) * 1000.0 + kick_mm[lane];
                }
                lag_m = length(sub(rel, held));
            }
            None => {
                self.spring.reset();
                for lane in 0..3 {
                    view.eye_mm[lane] = shaken.eye_mm[lane];
                }
            }
        }
        Ok(PovEye { view, aim, lag_m })
    }
}

/// The aim ray of an eye: its origin in meters and the unit direction it looks along. Right-handed,
/// `+y` up, yaw `0` faces `+z` (the same frame as `camera_for_eye`).
#[must_use]
pub fn aim_ray(eye: &RenderEye) -> ([f32; 3], [f32; 3]) {
    let (sy, cy) = eye.yaw_rad.sin_cos();
    let (sp, cp) = eye.pitch_rad.sin_cos();
    (super::eye_m(eye), [sy * cp, sp, cy * cp])
}

/// Distance along the facing from `feet_m` to the pit wall, meters: the square `[-h, h]^2`.
/// Units: meters, radians (yaw `0` faces `+z`). A facing that never reaches a wall reads
/// `f32::MAX`.
#[must_use]
pub fn wall_distance_m(feet_m: [f32; 3], yaw_rad: f32, half_extent_m: f32) -> f32 {
    let dir = [yaw_rad.sin(), yaw_rad.cos()];
    let at = [feet_m[0], feet_m[2]];
    let mut best = f32::MAX;
    for axis in 0..2 {
        if dir[axis].abs() < 1e-6 {
            continue;
        }
        let wall = half_extent_m.copysign(dir[axis]);
        let along = (wall - at[axis]) / dir[axis];
        if along >= 0.0 {
            best = best.min(along);
        }
    }
    best
}

/// The yaw the first-person eye looks along. With a living rival it is the lock-on bearing the
/// sim reads for the swing (`rival_cdeg`, centidegrees from `+z` toward `+x`), so the picture
/// and the blow agree from the first frame. With nobody to face it keeps the body's yaw, turned
/// to the pit centre when that yaw would face a wall inside [`WALL_CLEAR_M`]: the spawn never
/// stares into stone.
/// Units: meters, radians, centidegrees. Pure; zero heap.
#[must_use]
pub fn spawn_yaw(
    feet_m: [f32; 3],
    body_yaw_rad: f32,
    rival_cdeg: Option<i16>,
    half_extent_m: f32,
) -> f32 {
    if let Some(cdeg) = rival_cdeg {
        return (f32::from(cdeg) / 100.0).to_radians();
    }
    let yaw = if body_yaw_rad.is_finite() {
        body_yaw_rad
    } else {
        0.0
    };
    if wall_distance_m(feet_m, yaw, half_extent_m) >= WALL_CLEAR_M {
        yaw
    } else {
        (-feet_m[0]).atan2(-feet_m[2])
    }
}

/// A pinhole camera for the view laws: eye, the three view axes, the half-angle tangents and the
/// near plane. Built from the frame's final camera, so the law reads the picture that is drawn.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct PovLens {
    eye: [f32; 3],
    right: [f32; 3],
    up: [f32; 3],
    forward: [f32; 3],
    tan_y: f32,
    tan_x: f32,
    near: f32,
}

impl PovLens {
    /// The lens of a look-at camera; `None` for a non-finite or degenerate camera.
    /// Units: meters, radians; `aspect` is width over height.
    #[must_use]
    pub fn look_at(
        eye: [f32; 3],
        target: [f32; 3],
        up_hint: [f32; 3],
        fov_y_rad: f32,
        aspect: f32,
        near: f32,
    ) -> Option<Self> {
        let sane = finite3(eye)
            && finite3(target)
            && finite3(up_hint)
            && fov_y_rad.is_finite()
            && fov_y_rad > 0.0
            && fov_y_rad < PI
            && aspect.is_finite()
            && aspect > 0.0
            && near.is_finite()
            && near > 0.0;
        if !sane {
            return None;
        }
        let forward = normalized(sub(target, eye))?;
        let right = normalized(cross3(forward, up_hint))?;
        let up = cross3(right, forward);
        let tan_y = (fov_y_rad * 0.5).tan();
        Some(Self {
            eye,
            right,
            up,
            forward,
            tan_y,
            tan_x: tan_y * aspect,
            near,
        })
    }

    /// `p` in camera space: `x` right, `y` up, `z` the distance along the view.
    fn to_camera(self, p: [f32; 3]) -> [f32; 3] {
        let d = sub(p, self.eye);
        [dot(d, self.right), dot(d, self.up), dot(d, self.forward)]
    }

    /// A camera-space point in the picture's NDC (`-1..=1`, `y` up); `z` must be past the plane.
    fn ndc(&self, c: [f32; 3]) -> [f32; 2] {
        [c[0] / (c[2] * self.tan_x), c[1] / (c[2] * self.tan_y)]
    }
}

/// What the wearer's own geometry does to one frame's picture.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct PovCover {
    /// Triangles the near plane cuts inside the picture: the shards of a camera inside a body.
    pub near_cut: u32,
    /// Distance of the own triangle on the screen centre, meters; `None` when the centre is clear.
    pub centre_m: Option<f32>,
    /// Share of the picture the own geometry covers, `0..=1`.
    pub share: f32,
}

impl PovCover {
    /// True when the frame breaks a per-frame floor: a cut near plane or a cover past
    /// [`COVER_MAX`]. The centre cover is judged over a run of frames, not one.
    #[must_use]
    pub fn broken(&self) -> bool {
        self.near_cut > 0 || self.share > COVER_MAX
    }
}

/// The accumulator for [`PovCover`]: feed it every drawn part of the wearer, then read it.
#[derive(Clone, Debug)]
pub struct CoverAcc {
    lens: PovLens,
    near_cut: u32,
    centre_m: Option<f32>,
    mask: [bool; GRID_W * GRID_H],
}

impl CoverAcc {
    /// An empty accumulator over `lens`.
    #[must_use]
    pub fn new(lens: PovLens) -> Self {
        Self {
            lens,
            near_cut: 0,
            centre_m: None,
            mask: [false; GRID_W * GRID_H],
        }
    }

    /// Fold one part: posed `positions` and the triangle `indices` into them. A triangle with a
    /// bad index or a non-finite corner is skipped.
    pub fn add_part(&mut self, positions: &[[f32; 3]], indices: &[u32]) {
        for tri in indices.chunks_exact(3) {
            let corner = |at: u32| usize::try_from(at).ok().and_then(|i| positions.get(i));
            let (Some(a), Some(b), Some(c)) = (corner(tri[0]), corner(tri[1]), corner(tri[2]))
            else {
                continue;
            };
            if !(finite3(*a) && finite3(*b) && finite3(*c)) {
                continue;
            }
            let cam = [
                self.lens.to_camera(*a),
                self.lens.to_camera(*b),
                self.lens.to_camera(*c),
            ];
            self.fold_triangle(cam);
        }
    }

    fn fold_triangle(&mut self, cam: [[f32; 3]; 3]) {
        let near = self.lens.near;
        let (poly, n) = clip_to_plane(cam, near);
        if n < 3 {
            return;
        }
        let poly = &poly[..n];
        let mut corners = [[0.0f32; 2]; 4];
        for (slot, corner) in corners.iter_mut().zip(poly) {
            *slot = self.lens.ndc(*corner);
        }
        let ndc = &corners[..n];
        // Some corner behind the plane and some ahead of it: the plane cuts this triangle.
        let cut = cam.iter().any(|c| c[2] < near);
        if cut && ndc_overlaps_picture(ndc) {
            self.near_cut = self.near_cut.saturating_add(1);
        }
        for i in 1..n - 1 {
            self.raster([ndc[0], ndc[i], ndc[i + 1]]);
            if let Some(depth) = centre_depth([poly[0], poly[i], poly[i + 1]]) {
                self.centre_m = Some(self.centre_m.map_or(depth, |near| near.min(depth)));
            }
        }
    }

    /// Mark the grid cells whose centre sits inside the projected triangle.
    fn raster(&mut self, tri: [[f32; 2]; 3]) {
        let area = edge(tri[0], tri[1], tri[2]);
        if area.abs() < 1e-12 || !area.is_finite() {
            return;
        }
        let cell_x =
            |ndc: f32| ((ndc + 1.0) * 0.5 * grid_f(GRID_W)).clamp(-1.0, grid_f(GRID_W) + 1.0);
        let cell_y =
            |ndc: f32| ((ndc + 1.0) * 0.5 * grid_f(GRID_H)).clamp(-1.0, grid_f(GRID_H) + 1.0);
        let xs = tri.map(|p| cell_x(p[0]));
        let ys = tri.map(|p| cell_y(p[1]));
        let span = |lane: &[f32; 3], cells: usize| {
            let lo = lane
                .iter()
                .copied()
                .fold(f32::MAX, f32::min)
                .floor()
                .max(0.0);
            let hi = lane.iter().copied().fold(f32::MIN, f32::max).ceil();
            (grid_index(lo, cells), grid_index(hi, cells))
        };
        let (x0, x1) = span(&xs, GRID_W);
        let (y0, y1) = span(&ys, GRID_H);
        for gy in y0..=y1.min(GRID_H - 1) {
            for gx in x0..=x1.min(GRID_W - 1) {
                let p = [
                    (grid_f(gx) + 0.5) / grid_f(GRID_W) * 2.0 - 1.0,
                    (grid_f(gy) + 0.5) / grid_f(GRID_H) * 2.0 - 1.0,
                ];
                let w = [
                    edge(tri[1], tri[2], p),
                    edge(tri[2], tri[0], p),
                    edge(tri[0], tri[1], p),
                ];
                let inside = if area > 0.0 {
                    w.iter().all(|v| *v >= 0.0)
                } else {
                    w.iter().all(|v| *v <= 0.0)
                };
                if inside {
                    self.mask[gy * GRID_W + gx] = true;
                }
            }
        }
    }

    /// The frame's cover so far.
    #[must_use]
    pub fn finish(&self) -> PovCover {
        let covered = self.mask.iter().filter(|cell| **cell).count();
        PovCover {
            near_cut: self.near_cut,
            centre_m: self.centre_m,
            share: grid_f(covered) / grid_f(GRID_W * GRID_H),
        }
    }
}

/// A small grid count as `f32` without a numeric cast (counts here stay under `u16`).
fn grid_f(n: usize) -> f32 {
    f32::from(u16::try_from(n).unwrap_or(u16::MAX))
}

/// A grid coordinate in cells as an index, clamped into `0..cells`.
fn grid_index(v: f32, cells: usize) -> usize {
    (0..cells)
        .find(|i| grid_f(*i + 1) > v)
        .unwrap_or_else(|| cells.saturating_sub(1))
}

/// Twice the signed area of `(a, b, p)`: the edge function.
fn edge(a: [f32; 2], b: [f32; 2], p: [f32; 2]) -> f32 {
    (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0])
}

/// Clip a camera-space triangle to `z >= near`: the polygon (up to four corners) and its count.
fn clip_to_plane(tri: [[f32; 3]; 3], near: f32) -> ([[f32; 3]; 4], usize) {
    let mut out = [[0.0f32; 3]; 4];
    let mut n = 0usize;
    for i in 0..3 {
        let (p, q) = (tri[i], tri[(i + 1) % 3]);
        let (p_in, q_in) = (p[2] >= near, q[2] >= near);
        if p_in {
            out[n] = p;
            n += 1;
        }
        if p_in != q_in {
            let t = (near - p[2]) / (q[2] - p[2]);
            out[n] = [p[0] + (q[0] - p[0]) * t, p[1] + (q[1] - p[1]) * t, near];
            n += 1;
        }
    }
    (out, n)
}

/// True when the polygon's NDC box touches the picture.
fn ndc_overlaps_picture(ndc: &[[f32; 2]]) -> bool {
    let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
    for p in ndc {
        for lane in 0..2 {
            lo[lane] = lo[lane].min(p[lane]);
            hi[lane] = hi[lane].max(p[lane]);
        }
    }
    hi[0] >= -1.0 && lo[0] <= 1.0 && hi[1] >= -1.0 && lo[1] <= 1.0
}

/// Depth at which the screen-centre ray (the camera's `z` axis) meets a camera-space triangle.
fn centre_depth(tri: [[f32; 3]; 3]) -> Option<f32> {
    let flat = tri.map(|c| [c[0], c[1]]);
    let origin = [0.0, 0.0];
    let area = edge(flat[0], flat[1], flat[2]);
    if area.abs() < 1e-12 || !area.is_finite() {
        return None;
    }
    let w = [
        edge(flat[1], flat[2], origin) / area,
        edge(flat[2], flat[0], origin) / area,
        edge(flat[0], flat[1], origin) / area,
    ];
    w.iter()
        .all(|v| *v >= 0.0)
        .then(|| w[0] * tri[0][2] + w[1] * tri[1][2] + w[2] * tri[2][2])
}

/// Reticle: the rects of one frame as `[x0, y0, x1, y1]` in the overlay's `0..1` space, top left
/// origin, in draw order: each element's dark outline, then its fill. Four ticks around a gap
/// and a centre dot, square in pixels at any `aspect`.
pub const RETICLE_RECTS: usize = 10;

/// Reticle tick length, in picture heights.
const TICK_LEN: f32 = 0.016;
/// Reticle gap between the centre and a tick, in picture heights.
const TICK_GAP: f32 = 0.012;
/// Reticle tick thickness, in picture heights.
const TICK_THICK: f32 = 0.004;
/// Outline width around every element, in picture heights.
const OUTLINE: f32 = 0.0015;
/// Centre dot size, in picture heights.
const DOT: f32 = 0.0035;

/// The reticle's overlay rects for a picture of `aspect` (width over height).
/// A non-finite or non-positive aspect reads as square.
#[must_use]
pub fn reticle_rects(aspect: f32) -> [[f32; 4]; RETICLE_RECTS] {
    let aspect = if aspect.is_finite() && aspect > 0.0 {
        aspect
    } else {
        1.0
    };
    // A box `[x0, x1] x [y0, y1]` in picture heights from the centre, as an overlay rect.
    let rect = |x0: f32, x1: f32, y0: f32, y1: f32, grow: f32| {
        [
            0.5 + (x0 - grow) / aspect,
            0.5 + (y0 - grow),
            0.5 + (x1 + grow) / aspect,
            0.5 + (y1 + grow),
        ]
    };
    let half = TICK_THICK * 0.5;
    let far = TICK_GAP + TICK_LEN;
    let boxes = [
        (-far, -TICK_GAP, -half, half),
        (TICK_GAP, far, -half, half),
        (-half, half, -far, -TICK_GAP),
        (-half, half, TICK_GAP, far),
        (-DOT, DOT, -DOT, DOT),
    ];
    let mut out = [[0.0f32; 4]; RETICLE_RECTS];
    for (i, (x0, x1, y0, y1)) in boxes.into_iter().enumerate() {
        out[2 * i] = rect(x0, x1, y0, y1, OUTLINE);
        out[2 * i + 1] = rect(x0, x1, y0, y1, 0.0);
    }
    out
}

/// Stage the reticle over the HUD quads already staged this frame. Returns false when the
/// overlay ring was full and some rect did not land. Window leg only.
#[cfg(feature = "app")]
pub fn stage_reticle(presenter: &mut engine_render::present::Presenter, aspect: f32) -> bool {
    let mut all = true;
    for (i, rect) in reticle_rects(aspect).iter().enumerate() {
        // The outline is a bar with no fill (its dark back); the fill is a full bar over it.
        let fraction = if i % 2 == 0 { 0.0 } else { 1.0 };
        all &= presenter.push_hp_bar(rect[0], rect[1], rect[2], rect[3], fraction);
    }
    all
}

#[cfg(test)]
#[path = "fps_pov_tests.rs"]
mod fps_pov_tests;
