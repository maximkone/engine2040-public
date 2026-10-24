//! camera_frame: where the third-person fight camera puts the bodies it follows.
//!
//! Owner: game lane. Consumers: `fighter_look_frame` (the fight camera's refit
//! before the springs and the containment after them) and the `arena framing:`
//! receipt (`framing_law`). The fight camera used to aim at the floor points
//! under the fighters with a fixed look height, so a body that left the floor (a
//! rope swing, a yank, a tow, a jump) left the picture: the window showed bare sand
//! while both bodies were in the air, and a pair 8 m apart was two specks.
//! This module is the picture-space half: it projects each body's real world bounds,
//! reads how big and how inside the frame it is, and refits the camera so the bodies
//! stay inside a comfort frame at a readable size, leading their velocity.
//!
//! Units: meters, radians, NDC (`-1..=1`, y up) and shares of the frame height.
//! Determinism: presentation only, pure over its inputs, same inputs give the same
//! rig twice; nothing here reaches a tick hash. Errors: none, a rig that cannot
//! build a basis comes back unchanged. Budget: a few hundred float ops per frame
//! (six refit passes over at most [`MAX_SUBJECTS`] bodies), no heap.
//! The rig is plain arrays so the math has no render dependency; the adapters live
//! with the look.

use engine_core::vec3::cross3 as cross;

use crate::camera_arm::{arm_length, Aabb};

/// The `arena framing:` receipt tally, with the law over the bodies the camera follows.
pub mod framing;

/// Most bodies one framing holds: the hero, the threat and two bystanders.
pub const MAX_SUBJECTS: usize = 4;
/// Share of the frame height below which the biggest body is too small to read.
pub const HEIGHT_FLOOR: f32 = 0.15;
/// Share of the frame height the biggest body is brought back to from above the
/// [`SOFT`] ceiling: the top of the 15 to 45 percent band.
pub const HEIGHT_BAND_TOP: f32 = 0.45;
/// Share of the frame height past which one body fills the picture.
pub const HEIGHT_FILLS: f32 = 0.85;
/// Most of one body that may sit outside the frame before it counts as lost.
pub const MAX_CROPPED: f32 = 0.25;
/// Refit passes per frame: dolly, then pan, each time from the new picture.
const PASSES: u8 = 6;
/// Smallest on-screen change worth a dolly: a share of the current scale.
const DOLLY_DEADBAND: f32 = 0.02;
/// Narrowest the scale may shrink in one pass: a body inside the lens never asks for more.
const MIN_SCALE: f32 = 0.2;
/// Floor of the eye height, meters: a camera under this is in the sand.
const MIN_EYE_Y_M: f32 = 0.5;
/// Floor of the look height, meters.
const MIN_LOOK_Y_M: f32 = 0.2;
/// Radius of the boom sweep against obstacles, meters.
const SWEEP_RADIUS_M: f32 = 0.25;
/// Seconds of velocity the lead looks ahead.
pub const LEAD_S: f32 = 0.3;
/// Farthest the lead moves a body's bounds on one axis, meters.
pub const LEAD_MAX_M: f32 = 2.0;
/// Share of a tick's new velocity a smoothed velocity takes.
const VEL_BLEND: f32 = 0.5;
/// Fastest body the lead trusts, m/s: a teleport reads as a speed above this.
const VEL_MAX_MS: f32 = 16.0;

/// A point or vector in meters.
pub type Vec3 = [f32; 3];

fn sub(a: Vec3, b: Vec3) -> Vec3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn add(a: Vec3, b: Vec3) -> Vec3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn scale(a: Vec3, k: f32) -> Vec3 {
    [a[0] * k, a[1] * k, a[2] * k]
}

fn dot(a: Vec3, b: Vec3) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn unit(a: Vec3) -> Option<Vec3> {
    let len = dot(a, a).sqrt();
    (len.is_finite() && len > 1.0e-6).then(|| scale(a, 1.0 / len))
}

fn finite(a: Vec3) -> bool {
    a.iter().all(|v| v.is_finite())
}

/// The camera the framing moves: eye, look point and lens. World up is `+y`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rig {
    /// Eye position, meters.
    pub eye: Vec3,
    /// Look point, meters.
    pub target: Vec3,
    /// Vertical field of view, radians.
    pub fov_y: f32,
    /// Width over height.
    pub aspect: f32,
    /// Near clip, meters.
    pub near: f32,
}

/// A rectangle in NDC: `x0 < x1` left to right, `y0 < y1` bottom to top.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    /// Left edge.
    pub x0: f32,
    /// Bottom edge.
    pub y0: f32,
    /// Right edge.
    pub x1: f32,
    /// Top edge.
    pub y1: f32,
}

impl Rect {
    /// Width in NDC.
    #[must_use]
    pub fn w(self) -> f32 {
        self.x1 - self.x0
    }

    /// Height in NDC.
    #[must_use]
    pub fn h(self) -> f32 {
        self.y1 - self.y0
    }

    /// Area in NDC squared.
    #[must_use]
    pub fn area(self) -> f32 {
        self.w().max(0.0) * self.h().max(0.0)
    }

    fn union(self, other: Self) -> Self {
        Self {
            x0: self.x0.min(other.x0),
            y0: self.y0.min(other.y0),
            x1: self.x1.max(other.x1),
            y1: self.y1.max(other.y1),
        }
    }

    fn overlap(self, other: Self) -> f32 {
        let (w, h) = (
            self.x1.min(other.x1) - self.x0.max(other.x0),
            self.y1.min(other.y1) - self.y0.max(other.y0),
        );
        w.max(0.0) * h.max(0.0)
    }

    /// Whether the point sits inside the rectangle.
    #[must_use]
    pub fn holds(self, x: f32, y: f32) -> bool {
        x >= self.x0 && x <= self.x1 && y >= self.y0 && y <= self.y1
    }
}

/// The frame a body must stay inside for the law: just inside the picture.
pub const SAFE: Rect = Rect {
    x0: -0.93,
    y0: -0.93,
    x1: 0.93,
    y1: 0.93,
};

/// One body the camera keeps: its drawn world bounds, meters.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Subject {
    /// Lowest corner.
    pub min: Vec3,
    /// Highest corner.
    pub max: Vec3,
}

impl Subject {
    /// Half width of a standing body's stand-in box, meters.
    const HALF_M: f32 = 0.4;
    /// Height of a standing body's stand-in box, meters.
    const TALL_M: f32 = 1.8;

    /// A standing body's stand-in box at `root` (feet), for a look with nothing drawn yet.
    #[must_use]
    pub fn standing(root: Vec3) -> Self {
        Self {
            min: [root[0] - Self::HALF_M, root[1], root[2] - Self::HALF_M],
            max: [
                root[0] + Self::HALF_M,
                root[1] + Self::TALL_M,
                root[2] + Self::HALF_M,
            ],
        }
    }

    /// True when both corners are finite and ordered.
    #[must_use]
    pub fn valid(self) -> bool {
        finite(self.min) && finite(self.max) && (0..3).all(|axis| self.min[axis] <= self.max[axis])
    }

    /// Middle of the box.
    #[must_use]
    pub fn center(self) -> Vec3 {
        scale(add(self.min, self.max), 0.5)
    }

    /// The same box moved by `by`.
    #[must_use]
    pub fn moved(self, by: Vec3) -> Self {
        Self {
            min: add(self.min, by),
            max: add(self.max, by),
        }
    }

    /// The box holding both.
    #[must_use]
    pub fn joined(self, other: Self) -> Self {
        Self {
            min: std::array::from_fn(|axis| self.min[axis].min(other.min[axis])),
            max: std::array::from_fn(|axis| self.max[axis].max(other.max[axis])),
        }
    }

    /// The box as a camera obstacle for [`arm_length`].
    #[must_use]
    pub fn aabb(self) -> Aabb {
        Aabb::new(self.min, self.max)
    }

    /// A standing body's drawn bounds cut to a body's reach around its `root` (the
    /// feet): [`REACH_M`] sideways and [`TALL_M`] up, so a cape in the wind or a
    /// stray vertex never makes the body wider than it stands. The whole box comes
    /// back when the cut would leave nothing.
    #[must_use]
    pub fn trimmed(self, root: Vec3) -> Self {
        let cut = Self {
            min: [
                self.min[0].max(root[0] - REACH_M),
                self.min[1].max(root[1] - 0.2),
                self.min[2].max(root[2] - REACH_M),
            ],
            max: [
                self.max[0].min(root[0] + REACH_M),
                self.max[1].min(root[1] + TALL_M),
                self.max[2].min(root[2] + REACH_M),
            ],
        };
        if cut.valid() {
            cut
        } else {
            self
        }
    }
}

/// Farthest a standing body's bounds may reach from its root sideways, meters.
pub const REACH_M: f32 = 1.4;
/// Tallest a standing body's bounds may stand over its root, meters.
pub const TALL_M: f32 = 2.4;

/// Up to [`MAX_SUBJECTS`] bodies, the primary (the hero) first.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Subjects {
    items: [Subject; MAX_SUBJECTS],
    len: usize,
}

impl Default for Subjects {
    fn default() -> Self {
        Self::new()
    }
}

impl Subjects {
    /// An empty set.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            items: [Subject {
                min: [0.0; 3],
                max: [0.0; 3],
            }; MAX_SUBJECTS],
            len: 0,
        }
    }

    /// Add a body; an invalid box or a full set drops it.
    pub fn push(&mut self, subject: Subject) {
        if subject.valid() && self.len < MAX_SUBJECTS {
            self.items[self.len] = subject;
            self.len += 1;
        }
    }

    /// The bodies in order.
    #[must_use]
    pub fn as_slice(&self) -> &[Subject] {
        &self.items[..self.len]
    }

    /// How many bodies.
    #[must_use]
    pub fn len(&self) -> usize {
        self.len
    }

    /// True with no bodies.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

/// The camera's axes and lens, built once per read.
struct Basis {
    eye: Vec3,
    fwd: Vec3,
    right: Vec3,
    up: Vec3,
    tan_half: f32,
    aspect: f32,
    near: f32,
}

impl Basis {
    fn new(rig: &Rig) -> Option<Self> {
        let fwd = unit(sub(rig.target, rig.eye))?;
        let right = unit(cross(fwd, [0.0, 1.0, 0.0]))?;
        let up = cross(right, fwd);
        let tan_half = (rig.fov_y * 0.5).tan();
        (finite(rig.eye)
            && tan_half.is_finite()
            && tan_half > 0.0
            && rig.aspect.is_finite()
            && rig.aspect > 0.0
            && rig.near > 0.0)
            .then_some(Self {
                eye: rig.eye,
                fwd,
                right,
                up,
                tan_half,
                aspect: rig.aspect,
                near: rig.near,
            })
    }

    /// `(right, up, depth)` of a point in the eye's frame, meters.
    fn local(&self, point: Vec3) -> Vec3 {
        let d = sub(point, self.eye);
        [dot(d, self.right), dot(d, self.up), dot(d, self.fwd)]
    }

    /// The unclipped NDC rectangle of `subject` and whether the near plane cuts it;
    /// `None` when the whole box sits behind the near plane. A corner past the near
    /// plane is replaced by where its box edge crosses it, so a body around the eye
    /// reads as filling the picture and never as a rectangle the wrong way round.
    fn rect(&self, subject: Subject) -> Option<(Rect, bool)> {
        let corners: [Vec3; 8] = std::array::from_fn(|corner| {
            let pick = |axis: usize| {
                if corner & (1 << axis) == 0 {
                    subject.min[axis]
                } else {
                    subject.max[axis]
                }
            };
            self.local([pick(0), pick(1), pick(2)])
        });
        let mut kept: Vec<Vec3> = Vec::with_capacity(20);
        kept.extend(corners.iter().copied().filter(|c| c[2] > self.near));
        let cut = kept.len() < corners.len();
        for (a, corner) in corners.iter().enumerate() {
            for bit in [1usize, 2, 4] {
                let Some(other) = corners.get(a | bit).filter(|_| a & bit == 0) else {
                    continue;
                };
                if (corner[2] > self.near) != (other[2] > self.near) {
                    let s = (self.near - corner[2]) / (other[2] - corner[2]);
                    kept.push(std::array::from_fn(|k| {
                        corner[k] + (other[k] - corner[k]) * s
                    }));
                }
            }
        }
        if kept.is_empty() {
            return None;
        }
        let mut rect = Rect {
            x0: f32::MAX,
            y0: f32::MAX,
            x1: f32::MIN,
            y1: f32::MIN,
        };
        for [x, y, depth] in kept {
            let depth = depth.max(self.near);
            let (nx, ny) = (
                x / (depth * self.tan_half * self.aspect),
                y / (depth * self.tan_half),
            );
            rect = rect.union(Rect {
                x0: nx,
                y0: ny,
                x1: nx,
                y1: ny,
            });
        }
        Some((rect, cut))
    }
}

/// One body as the picture shows it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BodyRead {
    /// Unclipped NDC rectangle of the body's bounds.
    pub rect: Rect,
    /// Share of the frame height the body fills: its larger screen side, so a
    /// body lying down or flying flat still counts by its length.
    pub size: f32,
    /// Share of the rectangle inside the safe frame, `0..=1`.
    pub visible: f32,
    /// Distance from the eye along the view axis to the body's middle, meters.
    pub depth_m: f32,
    /// True when the near plane cuts the body.
    pub near_cut: bool,
}

/// What the picture shows of a set of bodies.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Read {
    /// One entry per subject; `None` for a body wholly behind the near plane.
    pub bodies: [Option<BodyRead>; MAX_SUBJECTS],
    /// Rectangle holding every body in front of the eye.
    pub union: Rect,
    /// Biggest `size` of any body.
    pub size_max: f32,
    /// Mean depth of the bodies in front of the eye, meters.
    pub depth_m: f32,
}

fn read_with(basis: &Basis, subjects: &[Subject], safe: Rect) -> Option<Read> {
    let mut bodies = [None; MAX_SUBJECTS];
    let mut union: Option<Rect> = None;
    let (mut size_max, mut depth_sum, mut seen) = (0.0f32, 0.0f32, 0u8);
    for (slot, subject) in bodies.iter_mut().zip(subjects) {
        let Some((rect, near_cut)) = basis.rect(*subject) else {
            continue;
        };
        let depth_m = basis.local(subject.center())[2].max(basis.near);
        let size = (rect.h() * 0.5).max(rect.w() * 0.5 * basis.aspect);
        let visible = if rect.area() > 0.0 {
            (rect.overlap(safe) / rect.area()).clamp(0.0, 1.0)
        } else if safe.holds((rect.x0 + rect.x1) * 0.5, (rect.y0 + rect.y1) * 0.5) {
            1.0
        } else {
            0.0
        };
        size_max = size_max.max(size);
        depth_sum += depth_m;
        seen += 1;
        union = Some(union.map_or(rect, |u| u.union(rect)));
        *slot = Some(BodyRead {
            rect,
            size,
            visible,
            depth_m,
            near_cut,
        });
    }
    let union = union?;
    Some(Read {
        bodies,
        union,
        size_max,
        depth_m: depth_sum / f32::from(seen),
    })
}

/// What the picture of `rig` shows of `subjects`, judged against the safe frame;
/// `None` for no bodies, a rig that cannot build, or every body behind the eye.
#[must_use]
pub fn read(rig: &Rig, subjects: &[Subject]) -> Option<Read> {
    read_with(&Basis::new(rig)?, subjects, SAFE)
}

/// How one frame went for the law.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Verdict {
    /// Every body is in front of the eye, inside the safe frame with its middle
    /// in, and cropped by no more than [`MAX_CROPPED`].
    pub in_safe: bool,
    /// Biggest body's share of the frame height.
    pub size_max: f32,
}

impl Read {
    /// The frame's verdict against `subjects` (the same set the read came from).
    #[must_use]
    pub fn verdict(&self, subjects: usize) -> Verdict {
        let in_safe = self.bodies.iter().take(subjects).all(|body| {
            body.is_some_and(|b| {
                let (cx, cy) = ((b.rect.x0 + b.rect.x1) * 0.5, (b.rect.y0 + b.rect.y1) * 0.5);
                b.visible >= 1.0 - MAX_CROPPED && SAFE.holds(cx, cy)
            })
        });
        Verdict {
            in_safe,
            size_max: self.size_max,
        }
    }
}

/// How hard a refit pulls and where it stops.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Params {
    /// The frame the bodies are brought back into.
    pub comfort: Rect,
    /// Biggest-body share under which the camera dollies in.
    pub floor: f32,
    /// Share the dolly in aims for.
    pub aim_small: f32,
    /// Biggest-body share over which the camera dollies out.
    pub ceil: f32,
    /// Share the dolly out aims for.
    pub aim_big: f32,
    /// Share of a vertical re-aim the eye takes; the look point takes the rest.
    pub eye_follow: f32,
    /// Half width of the square the eye must stay inside, meters.
    pub pit_m: f32,
    /// Nearest the eye may come to a body, meters.
    pub eye_clear_m: f32,
    /// Highest the eye may rise, meters.
    pub eye_max_y_m: f32,
    /// Widest the lens may open when the dolly cannot hold the bodies, radians.
    pub fov_max: f32,
    /// Whether a body that reads too small pulls the camera in.
    pub dolly_in: bool,
}

impl Params {
    /// The pass before the springs: bring the bodies into a comfort frame at a readable size.
    pub const SOFT: Self = Self {
        comfort: Rect {
            x0: -0.78,
            y0: -0.84,
            x1: 0.78,
            y1: 0.80,
        },
        floor: HEIGHT_FLOOR,
        aim_small: 0.2,
        ceil: 0.55,
        aim_big: HEIGHT_BAND_TOP,
        eye_follow: 0.5,
        pit_m: f32::MAX,
        eye_clear_m: 1.0,
        eye_max_y_m: 14.0,
        fov_max: 1.4,
        dolly_in: true,
    };

    /// The pass after the springs: only keep the bodies inside the safe frame.
    pub const HARD: Self = Self {
        comfort: Rect {
            x0: -0.88,
            y0: -0.90,
            x1: 0.88,
            y1: 0.88,
        },
        floor: 0.0,
        aim_small: 0.0,
        ceil: f32::MAX,
        aim_big: f32::MAX,
        eye_follow: 0.5,
        pit_m: f32::MAX,
        eye_clear_m: 1.0,
        eye_max_y_m: 14.0,
        fov_max: 0.0,
        dolly_in: false,
    };

    /// The same params with the eye held inside a square pit of half width `pit_m`.
    #[must_use]
    pub const fn in_pit(mut self, pit_m: f32) -> Self {
        self.pit_m = pit_m;
        self
    }
}

/// Scale the bodies' picture wants, `1.0` for none: below `1.0` they must shrink to
/// fit (or stop filling the frame), above `1.0` they may grow to a readable size.
fn wanted_scale(read: &Read, p: &Params) -> f32 {
    let (w, h) = (read.union.w().max(1.0e-4), read.union.h().max(1.0e-4));
    let fit = (p.comfort.w() / w).min(p.comfort.h() / h);
    let size = read.size_max.max(1.0e-4);
    if fit < 1.0 {
        fit.max(MIN_SCALE)
    } else if size > p.ceil {
        (p.aim_big / size).clamp(MIN_SCALE, 1.0)
    } else if p.dolly_in && size < p.floor {
        (p.aim_small / size).min(fit).max(1.0)
    } else {
        1.0
    }
}

/// The NDC shift that brings `lo..hi` into `bound_lo..bound_hi`: the smallest move
/// that holds it, or the move that centres it when it is wider than the bound.
fn pan_need(lo: f32, hi: f32, bound_lo: f32, bound_hi: f32) -> f32 {
    if hi - lo >= bound_hi - bound_lo {
        (bound_lo + bound_hi) * 0.5 - (lo + hi) * 0.5
    } else if lo < bound_lo {
        bound_lo - lo
    } else if hi > bound_hi {
        bound_hi - hi
    } else {
        0.0
    }
}

/// Largest travel along `dir` (a unit vector) from `from` that keeps the eye inside
/// the pit square and under the ceiling, meters; never negative.
fn room_along(from: Vec3, dir: Vec3, p: &Params) -> f32 {
    let mut room = f32::MAX;
    for axis in [0usize, 2] {
        if dir[axis].abs() > 1.0e-4 {
            let edge = if dir[axis] > 0.0 { p.pit_m } else { -p.pit_m };
            room = room.min((edge - from[axis]) / dir[axis]);
        }
    }
    if dir[1] > 1.0e-4 {
        room = room.min((p.eye_max_y_m - from[1]) / dir[1]);
    } else if dir[1] < -1.0e-4 {
        room = room.min((MIN_EYE_Y_M - from[1]) / dir[1]);
    }
    room.max(0.0)
}

/// Nearest the eye at `eye` is to any body's box, meters.
fn eye_gap(eye: Vec3, subjects: &[Subject]) -> f32 {
    subjects
        .iter()
        .map(|s| {
            let d: Vec3 = std::array::from_fn(|axis| {
                (s.min[axis] - eye[axis])
                    .max(eye[axis] - s.max[axis])
                    .max(0.0)
            });
            dot(d, d).sqrt()
        })
        .fold(f32::MAX, f32::min)
}

/// Move eye and look point together along the view axis by `forward` meters
/// (negative backs away), held by the pit and, going in, by the eye clearance.
fn dolly(rig: &mut Rig, fwd: Vec3, forward: f32, subjects: &[Subject], p: &Params) {
    let travel = if forward < 0.0 {
        -(-forward).min(room_along(rig.eye, scale(fwd, -1.0), p))
    } else {
        let mut t = forward;
        for _ in 0..4 {
            if eye_gap(add(rig.eye, scale(fwd, t)), subjects) >= p.eye_clear_m || t <= 0.0 {
                break;
            }
            t *= 0.5;
        }
        if eye_gap(add(rig.eye, scale(fwd, t)), subjects) >= p.eye_clear_m {
            t
        } else {
            0.0
        }
    };
    let by = scale(fwd, travel);
    rig.eye = add(rig.eye, by);
    rig.target = add(rig.target, by);
}

/// Turn the view so the bodies' picture moves by `(sx, sy)` NDC: the look point moves
/// the other way, and the eye takes `eye_follow` of the vertical part.
fn pan(rig: &mut Rig, basis: &Basis, depth_m: f32, shift: (f32, f32), p: &Params) {
    let move_x = -shift.0 * depth_m * basis.tan_half * basis.aspect;
    let move_y = -shift.1 * depth_m * basis.tan_half;
    rig.target = add(
        rig.target,
        add(scale(basis.right, move_x), scale(basis.up, move_y)),
    );
    rig.target[1] = rig.target[1].max(MIN_LOOK_Y_M);
    rig.eye[1] = (rig.eye[1] + move_y * p.eye_follow).clamp(MIN_EYE_Y_M, p.eye_max_y_m);
}

/// Pull the eye in along its boom until the pit holds it, keeping the look point.
fn hold_eye(rig: &mut Rig, p: &Params) {
    let boom = sub(rig.eye, rig.target);
    let mut share = 1.0f32;
    for axis in [0usize, 2] {
        if boom[axis].abs() > 1.0e-4 {
            let edge = if boom[axis] > 0.0 { p.pit_m } else { -p.pit_m };
            share = share.min((edge - rig.target[axis]) / boom[axis]);
        }
    }
    if share < 1.0 {
        rig.eye = add(rig.target, scale(boom, share.clamp(0.05, 1.0)));
    }
    rig.eye[1] = rig.eye[1].clamp(MIN_EYE_Y_M, p.eye_max_y_m);
}

/// Open the lens just enough to hold the bodies the dolly could not (soft pass only).
fn widen(rig: &mut Rig, subjects: &[Subject], p: &Params) {
    if p.fov_max <= rig.fov_y {
        return;
    }
    let Some(basis) = Basis::new(rig) else {
        return;
    };
    let Some(read) = read_with(&basis, subjects, p.comfort) else {
        return;
    };
    let fit = (p.comfort.w() / read.union.w().max(1.0e-4))
        .min(p.comfort.h() / read.union.h().max(1.0e-4));
    if fit < 1.0 {
        let tan = (basis.tan_half / fit.max(MIN_SCALE)).atan();
        rig.fov_y = (2.0 * tan).clamp(rig.fov_y, p.fov_max);
    }
}

/// The boom sweep: when an obstacle stands between the bodies and the eye, the eye
/// comes in to just short of it (never closer than the clearance to the bodies).
fn clear_obstacles(rig: &mut Rig, subjects: &[Subject], obstacles: &[Aabb], p: &Params) {
    let Some(first) = subjects.first() else {
        return;
    };
    let pivot = first.center();
    let boom = sub(rig.eye, pivot);
    let len = dot(boom, boom).sqrt();
    if obstacles.is_empty() || len < 1.0e-3 {
        return;
    }
    let arm = arm_length(pivot, boom, len, SWEEP_RADIUS_M, obstacles);
    if arm < len - 0.01 {
        let kept = arm.max(p.eye_clear_m.min(len));
        rig.eye = add(pivot, scale(boom, kept / len));
    }
}

/// `rig` refit so `subjects` sit inside `p.comfort` at a readable size: dolly to the
/// size the params want, pan the view until the bodies are inside the frame (the eye
/// takes part of a vertical move), hold the eye in the pit and out of obstacles, and
/// open the lens when nothing else holds the bodies. A rig that already holds them
/// comes back unchanged. Pure.
#[must_use]
pub fn refit(rig: &Rig, subjects: &[Subject], p: &Params, obstacles: &[Aabb]) -> Rig {
    let mut rig = *rig;
    for _ in 0..PASSES {
        let Some(basis) = Basis::new(&rig) else {
            break;
        };
        let Some(first) = read_with(&basis, subjects, p.comfort) else {
            break;
        };
        let k = wanted_scale(&first, p);
        if (k - 1.0).abs() > DOLLY_DEADBAND {
            dolly(
                &mut rig,
                basis.fwd,
                first.depth_m * (1.0 - 1.0 / k),
                subjects,
                p,
            );
        }
        let Some(basis) = Basis::new(&rig) else {
            break;
        };
        let Some(read) = read_with(&basis, subjects, p.comfort) else {
            break;
        };
        let u = read.union;
        let shift = (
            pan_need(u.x0, u.x1, p.comfort.x0, p.comfort.x1),
            pan_need(u.y0, u.y1, p.comfort.y0, p.comfort.y1),
        );
        if shift.0.abs() < 1.0e-3 && shift.1.abs() < 1.0e-3 && (k - 1.0).abs() <= DOLLY_DEADBAND {
            break;
        }
        pan(&mut rig, &basis, read.depth_m, shift, p);
        hold_eye(&mut rig, p);
    }
    clear_obstacles(&mut rig, subjects, obstacles, p);
    widen(&mut rig, subjects, p);
    rig
}

/// Smoothed body velocities for the look-ahead: one tick's step, blended over ticks.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Lead {
    slots: [Option<u8>; MAX_SUBJECTS],
    last: [Vec3; MAX_SUBJECTS],
    vel: [Vec3; MAX_SUBJECTS],
    tick: Option<u32>,
}

impl Default for Lead {
    fn default() -> Self {
        Self::new()
    }
}

impl Lead {
    /// A tracker that has seen nothing.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            slots: [None; MAX_SUBJECTS],
            last: [[0.0; 3]; MAX_SUBJECTS],
            vel: [[0.0; 3]; MAX_SUBJECTS],
            tick: None,
        }
    }

    /// Fold the bodies' middles at sim `tick` (`ids` are their slots, in subject
    /// order) and return each body's velocity in m/s. A tick seen twice holds the
    /// last velocities; a body that was not the same slot last time, a step faster
    /// than a body can run, or a tick that went backwards starts again from rest.
    pub fn step(
        &mut self,
        tick: u32,
        tick_hz: f32,
        ids: &[u8],
        centers: &[Vec3],
    ) -> [Vec3; MAX_SUBJECTS] {
        let count = ids.len().min(centers.len()).min(MAX_SUBJECTS);
        let ahead = self
            .tick
            .map(|last| tick.checked_sub(last))
            .unwrap_or(Some(0));
        match ahead {
            Some(0) if self.tick.is_some() => {}
            Some(gap) => {
                let gap = f32::from(u16::try_from(gap).unwrap_or(u16::MAX).max(1));
                let dt = gap / tick_hz.max(1.0);
                for i in 0..MAX_SUBJECTS {
                    if i >= count {
                        self.slots[i] = None;
                        self.vel[i] = [0.0; 3];
                        continue;
                    }
                    let same = self.tick.is_some() && self.slots[i] == Some(ids[i]);
                    let raw = scale(sub(centers[i], self.last[i]), 1.0 / dt);
                    self.vel[i] = if !same || !finite(raw) || dot(raw, raw).sqrt() > VEL_MAX_MS {
                        [0.0; 3]
                    } else {
                        add(self.vel[i], scale(sub(raw, self.vel[i]), VEL_BLEND))
                    };
                    self.slots[i] = Some(ids[i]);
                    self.last[i] = centers[i];
                }
                self.tick = Some(tick);
            }
            None => *self = Self::new(),
        }
        self.vel
    }

    /// The bounds `subject` is expected to sweep over the next [`LEAD_S`] at `vel`:
    /// the box now joined with the box moved ahead, never farther than [`LEAD_MAX_M`].
    #[must_use]
    pub fn ahead(subject: Subject, vel: Vec3) -> Subject {
        let by: Vec3 =
            std::array::from_fn(|axis| (vel[axis] * LEAD_S).clamp(-LEAD_MAX_M, LEAD_MAX_M));
        subject.joined(subject.moved(by))
    }
}

/// A run of frames the law watches: how many in all and the longest in a row.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Run {
    frames: u32,
    now: u32,
    longest: u32,
}

impl Run {
    fn note(&mut self, bad: bool) {
        if bad {
            self.frames = self.frames.saturating_add(1);
            self.now = self.now.saturating_add(1);
            self.longest = self.longest.max(self.now);
        } else {
            self.now = 0;
        }
    }
}

/// Longest run of frames with a body outside the safe frame that the law allows.
pub const OUT_RUN_MAX: u32 = 20;
/// Longest run of frames with every body under [`HEIGHT_FLOOR`] that the law allows.
pub const SMALL_RUN_MAX: u32 = 45;
/// Longest run of frames with one body filling the picture that the law allows.
pub const FILLS_RUN_MAX: u32 = 60;

/// The framing law over a run: the bodies stay in the safe frame, the biggest reads
/// above the floor, and none fills the picture, each with a run allowance so a cut
/// or a landing never fails it. Folded one verdict per presented frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Law {
    judged: u32,
    out: Run,
    small: Run,
    fills: Run,
    size_min: f32,
    size_max: f32,
}

impl Default for Law {
    fn default() -> Self {
        Self::new()
    }
}

impl Law {
    /// A law that has judged nothing.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            judged: 0,
            out: Run {
                frames: 0,
                now: 0,
                longest: 0,
            },
            small: Run {
                frames: 0,
                now: 0,
                longest: 0,
            },
            fills: Run {
                frames: 0,
                now: 0,
                longest: 0,
            },
            size_min: f32::MAX,
            size_max: 0.0,
        }
    }

    /// Frames judged so far.
    #[must_use]
    pub fn judged(&self) -> u32 {
        self.judged
    }

    /// Fold one frame.
    pub fn note(&mut self, verdict: &Verdict) {
        self.judged = self.judged.saturating_add(1);
        self.out.note(!verdict.in_safe);
        self.small.note(verdict.size_max < HEIGHT_FLOOR);
        self.fills.note(verdict.size_max > HEIGHT_FILLS);
        self.size_min = self.size_min.min(verdict.size_max);
        self.size_max = self.size_max.max(verdict.size_max);
    }

    /// Why the law fails, `None` while every run is inside its allowance.
    #[must_use]
    pub fn failure(&self) -> Option<String> {
        let mut why = Vec::new();
        if self.out.longest > OUT_RUN_MAX {
            why.push(format!(
                "bodies left the safe frame for {} frames in a row (allowed {OUT_RUN_MAX})",
                self.out.longest
            ));
        }
        if self.small.longest > SMALL_RUN_MAX {
            why.push(format!(
                "the biggest body read under {:.0} percent of the frame for {} frames in a row (allowed {SMALL_RUN_MAX})",
                HEIGHT_FLOOR * 100.0,
                self.small.longest
            ));
        }
        if self.fills.longest > FILLS_RUN_MAX {
            why.push(format!(
                "one body filled the picture for {} frames in a row (allowed {FILLS_RUN_MAX})",
                self.fills.longest
            ));
        }
        (!why.is_empty()).then(|| why.join("; "))
    }

    /// The receipt fields for the `arena framing:` line, led by a space; empty when
    /// nothing was judged. `body_h` is the biggest body's share of the frame height
    /// (lowest and highest over the run); each pair reads `frames/longest run`.
    #[must_use]
    pub fn fields(&self) -> String {
        if self.judged == 0 {
            return String::new();
        }
        let verdict = self
            .failure()
            .map_or_else(|| "ok".to_string(), |_| "FAIL".to_string());
        format!(
            " law={verdict} judged={} body_h={:.2}..{:.2} out={}/{} small={}/{} fills={}/{}",
            self.judged,
            self.size_min,
            self.size_max,
            self.out.frames,
            self.out.longest,
            self.small.frames,
            self.small.longest,
            self.fills.frames,
            self.fills.longest,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FOV: f32 = 1.047_197_6;
    const ASPECT: f32 = 16.0 / 9.0;

    fn rig(eye: Vec3, target: Vec3) -> Rig {
        Rig {
            eye,
            target,
            fov_y: FOV,
            aspect: ASPECT,
            near: 0.1,
        }
    }

    fn body(x: f32, y: f32, z: f32) -> Subject {
        Subject::standing([x, y, z])
    }

    fn one(s: Subject) -> Vec<Subject> {
        vec![s]
    }

    #[test]
    fn a_standing_body_reads_by_its_height_in_the_frame() {
        // 1.8 m at 6 m under a 60 degree lens: 1.8 / (2 * 6 * tan 30 degrees) = 0.26, and
        // the box's near face (5.6 m) is what the rectangle reads: 0.28.
        let r = rig([0.0, 1.0, 6.0], [0.0, 1.0, 0.0]);
        let read = read(&r, &one(body(0.0, 0.1, 0.0))).expect("in front");
        let size = read.bodies[0].expect("body").size;
        assert!((size - 0.278).abs() < 0.01, "{size}");
        assert!(read.bodies[0].expect("body").visible > 0.99);
    }

    #[test]
    fn a_stray_vertex_is_cut_to_the_bodys_reach() {
        let root = [2.0, 0.0, -1.0];
        let stray = Subject {
            min: [-30.0, -0.1, -1.4],
            max: [30.0, 9.0, -0.6],
        };
        let cut = stray.trimmed(root);
        assert_eq!(cut.min, [0.6, -0.1, -1.4]);
        assert_eq!(cut.max, [3.4, TALL_M, -0.6]);
        let own = body(2.0, 0.0, -1.0);
        assert_eq!(
            own.trimmed(root),
            own,
            "a body inside its reach is unchanged"
        );
        let nothing = Subject {
            min: [f32::MAX; 3],
            max: [f32::MIN; 3],
        };
        assert!(!nothing.trimmed(root).valid(), "no bounds stay no bounds");
    }

    #[test]
    fn a_body_behind_the_eye_reads_nothing() {
        let r = rig([0.0, 1.0, 6.0], [0.0, 1.0, 0.0]);
        assert!(read(&r, &one(body(0.0, 0.0, 20.0))).is_none());
        assert!(read(&r, &[]).is_none());
    }

    #[test]
    fn a_body_around_the_eye_fills_the_picture() {
        let r = rig([0.0, 1.0, 0.0], [0.0, 1.0, -5.0]);
        let read = read(&r, &one(body(0.0, 0.0, 0.0))).expect("cut by the near plane");
        assert!(read.bodies[0].expect("body").near_cut);
        assert!(read.size_max > HEIGHT_FILLS);
    }

    /// The bug: a body in the air, looked at from the floor, is out of the picture.
    fn swing_scene() -> (Rig, Vec<Subject>) {
        let r = rig([0.0, 1.9, 8.0], [0.0, 1.0, 0.0]);
        let air = vec![body(-1.0, 5.5, 0.0), body(1.0, 6.0, 0.0)];
        (r, air)
    }

    #[test]
    fn the_floor_camera_loses_two_bodies_in_the_air() {
        let (r, air) = swing_scene();
        let read = read(&r, &air).expect("in front");
        assert!(!read.verdict(2).in_safe, "the bodies are above the frame");
    }

    #[test]
    fn a_refit_follows_the_bodies_up() {
        let (r, air) = swing_scene();
        let fit = refit(&r, &air, &Params::SOFT, &[]);
        let read = read(&fit, &air).expect("in front");
        assert!(read.verdict(2).in_safe, "{read:?}");
        assert!(fit.target[1] > 3.0, "looks up at them: {}", fit.target[1]);
        assert!(fit.eye[1] > r.eye[1], "the eye rises with them");
        assert!(
            read.union.y1 <= Params::SOFT.comfort.y1 + 0.02,
            "{:?}",
            read.union
        );
    }

    #[test]
    fn a_pair_that_is_too_small_comes_in_to_a_readable_size() {
        let r = rig([0.0, 1.9, 30.0], [0.0, 1.0, 0.0]);
        let pair = vec![body(-1.0, 0.0, 0.0), body(1.0, 0.0, 0.0)];
        let before = read(&r, &pair).expect("in front").size_max;
        assert!(before < HEIGHT_FLOOR, "{before}");
        let fit = refit(&r, &pair, &Params::SOFT, &[]);
        let after = read(&fit, &pair).expect("in front");
        assert!(after.size_max >= HEIGHT_FLOOR, "{}", after.size_max);
        assert!(after.verdict(2).in_safe);
        assert!(after.union.x0 >= Params::SOFT.comfort.x0 - 0.02);
        assert!(after.union.x1 <= Params::SOFT.comfort.x1 + 0.02);
    }

    #[test]
    fn a_body_that_fills_the_frame_backs_off_to_the_band() {
        let r = rig([0.0, 1.5, 2.0], [0.0, 1.0, 0.0]);
        let one_body = one(body(0.0, 0.0, 0.0));
        let before = read(&r, &one_body).expect("in front").size_max;
        assert!(before > 0.7, "{before}");
        let fit = refit(&r, &one_body, &Params::SOFT.in_pit(20.0), &[]);
        let after = read(&fit, &one_body).expect("in front").size_max;
        assert!(after < 0.6 && after > 0.3, "{after}");
        assert!(fit.eye[2] > r.eye[2], "backed away");
    }

    #[test]
    fn the_pit_wall_holds_the_eye_when_the_body_fills_the_frame() {
        // The eye is already on the wall: the dolly out has no room and the eye stays in.
        let r = rig([19.0, 1.5, 0.0], [17.0, 1.0, 0.0]);
        let subject = one(body(17.8, 0.0, 0.0));
        let fit = refit(&r, &subject, &Params::SOFT.in_pit(19.0), &[]);
        assert!(fit.eye[0] <= 19.0 + 1.0e-3, "{:?}", fit.eye);
        assert!(fit.eye[0].is_finite() && fit.eye[2].abs() <= 19.0);
    }

    #[test]
    fn a_settled_shot_is_left_alone() {
        let r = rig([0.0, 1.9, 5.0], [0.0, 1.0, 0.0]);
        let subject = one(body(0.0, 0.0, 0.0));
        assert_eq!(refit(&r, &subject, &Params::SOFT, &[]), r);
    }

    #[test]
    fn the_hard_pass_only_contains() {
        // Far too small (no dolly in) but outside the frame: it pans in and does not dolly.
        let r = rig([0.0, 1.9, 30.0], [10.0, 1.0, 0.0]);
        let far = vec![body(-2.0, 0.0, 0.0), body(0.0, 0.0, 0.0)];
        let hard = refit(&r, &far, &Params::HARD, &[]);
        let read = read(&hard, &far).expect("in front");
        assert!(read.verdict(2).in_safe, "{read:?}");
        assert!(
            (hard.eye[2] - r.eye[2]).abs() < 1.0e-3,
            "no dolly: {:?}",
            hard.eye
        );
    }

    #[test]
    fn the_refit_never_leaves_the_pit() {
        for z in [-18.0, 0.0, 18.0] {
            let r = rig([18.5, 2.0, z], [0.0, 1.0, 0.0]);
            let pair = vec![body(-6.0, 4.0, 0.0), body(6.0, 0.0, 3.0)];
            let fit = refit(&r, &pair, &Params::SOFT.in_pit(19.0), &[]);
            assert!(fit.eye[0].abs() <= 19.0 + 1.0e-3 && fit.eye[2].abs() <= 19.0 + 1.0e-3);
            assert!(fit.eye[1] >= MIN_EYE_Y_M && fit.eye[1] <= 14.0);
        }
    }

    #[test]
    fn the_lens_opens_when_the_pit_stops_the_dolly() {
        // Two bodies far apart, the eye on the wall: only the lens can hold both.
        let r = rig([19.0, 1.9, 0.0], [0.0, 1.0, 0.0]);
        let wide = vec![body(5.0, 0.0, -14.0), body(5.0, 0.0, 14.0)];
        let fit = refit(&r, &wide, &Params::SOFT.in_pit(19.0), &[]);
        assert!(fit.fov_y > r.fov_y, "{} vs {}", fit.fov_y, r.fov_y);
        assert!(fit.fov_y <= Params::SOFT.fov_max + 1.0e-4);
    }

    #[test]
    fn an_obstacle_between_the_bodies_and_the_eye_pulls_the_eye_in() {
        let r = rig([0.0, 1.9, 9.0], [0.0, 1.0, 0.0]);
        let subject = one(body(0.0, 0.0, 0.0));
        let pillar = Aabb::new([-0.5, 0.0, 3.5], [0.5, 4.0, 4.5]);
        let fit = refit(&r, &subject, &Params::SOFT, &[pillar]);
        assert!(fit.eye[2] < 3.5 - SWEEP_RADIUS_M + 0.05, "{:?}", fit.eye);
        let clear = refit(&r, &subject, &Params::SOFT, &[]);
        assert!(
            clear.eye[2] > 8.0,
            "without it the eye stays out: {:?}",
            clear.eye
        );
    }

    #[test]
    fn the_lead_reads_a_rising_body_and_the_camera_goes_up_before_it_gets_there() {
        let mut lead = Lead::new();
        let mut vel = [[0.0; 3]; MAX_SUBJECTS];
        for tick in 0..10u32 {
            let rise = 0.12 * f32::from(u8::try_from(tick).unwrap_or(0));
            vel = lead.step(tick, 60.0, &[0], &[[0.0, 1.0 + rise, 0.0]]);
        }
        assert!((vel[0][1] - 7.2).abs() < 0.5, "7.2 m/s up: {:?}", vel[0]);
        let r = rig([0.0, 1.9, 8.0], [0.0, 1.0, 0.0]);
        let now = body(0.0, 3.0, 0.0);
        let plain = refit(&r, &[now], &Params::SOFT, &[]);
        let led = refit(&r, &[Lead::ahead(now, vel[0])], &Params::SOFT, &[]);
        assert!(
            led.target[1] > plain.target[1],
            "{} vs {}",
            led.target[1],
            plain.target[1]
        );
    }

    #[test]
    fn the_lead_starts_from_rest_on_a_new_body_a_teleport_and_a_rewind() {
        let mut lead = Lead::new();
        let at = |x: f32| [[x, 1.0, 0.0]];
        let _ = lead.step(1, 60.0, &[3], &at(0.0));
        assert_eq!(lead.step(2, 60.0, &[3], &at(0.1))[0][0].signum(), 1.0);
        assert_eq!(
            lead.step(3, 60.0, &[4], &at(0.2))[0],
            [0.0; 3],
            "another slot"
        );
        let _ = lead.step(4, 60.0, &[4], &at(0.3));
        assert_eq!(
            lead.step(5, 60.0, &[4], &at(40.0))[0],
            [0.0; 3],
            "a teleport"
        );
        let held = lead.step(5, 60.0, &[4], &at(41.0));
        assert_eq!(held[0], [0.0; 3], "the same tick holds");
        assert_eq!(lead.step(2, 60.0, &[4], &at(0.0))[0], [0.0; 3], "a rewind");
    }

    #[test]
    fn the_lead_is_capped() {
        let b = body(0.0, 0.0, 0.0);
        let far = Lead::ahead(b, [0.0, 50.0, 0.0]);
        assert!((far.max[1] - (b.max[1] + LEAD_MAX_M)).abs() < 1.0e-4);
        assert_eq!(far.min, b.min);
    }

    #[test]
    fn the_refit_is_deterministic() {
        let (r, air) = swing_scene();
        let a = refit(&r, &air, &Params::SOFT.in_pit(19.0), &[]);
        let b = refit(&r, &air, &Params::SOFT.in_pit(19.0), &[]);
        assert_eq!(a, b);
    }

    fn verdict(in_safe: bool, size_max: f32) -> Verdict {
        Verdict { in_safe, size_max }
    }

    #[test]
    fn the_law_passes_a_good_run_and_prints_its_numbers() {
        let mut law = Law::new();
        assert_eq!(law.fields(), "");
        for size in [0.22, 0.31, 0.45] {
            law.note(&verdict(true, size));
        }
        assert_eq!(law.failure(), None);
        assert_eq!(
            law.fields(),
            " law=ok judged=3 body_h=0.22..0.45 out=0/0 small=0/0 fills=0/0"
        );
    }

    #[test]
    fn the_law_forgives_a_short_loss_and_fails_a_long_one() {
        let mut law = Law::new();
        for _ in 0..OUT_RUN_MAX {
            law.note(&verdict(false, 0.3));
        }
        law.note(&verdict(true, 0.3));
        assert_eq!(law.failure(), None, "a run of exactly the allowance passes");
        for _ in 0..=OUT_RUN_MAX {
            law.note(&verdict(false, 0.3));
        }
        let why = law.failure().expect("a longer run fails");
        assert!(why.contains("left the safe frame"), "{why}");
        assert!(law.fields().starts_with(" law=FAIL"), "{}", law.fields());
    }

    #[test]
    fn the_law_fails_bodies_that_stay_below_the_floor_or_fill_the_picture() {
        let mut small = Law::new();
        for _ in 0..=SMALL_RUN_MAX {
            small.note(&verdict(true, 0.08));
        }
        assert!(small.failure().expect("fails").contains("under 15 percent"));
        let mut fills = Law::new();
        for _ in 0..=FILLS_RUN_MAX {
            fills.note(&verdict(true, 1.3));
        }
        assert!(fills
            .failure()
            .expect("fails")
            .contains("filled the picture"));
        let mut broken_up = Law::new();
        for n in 0..300 {
            broken_up.note(&verdict(true, if n % 40 == 0 { 0.3 } else { 0.08 }));
        }
        assert_eq!(broken_up.failure(), None, "runs under the allowance pass");
    }

    #[test]
    fn the_verdict_reads_cropping_and_the_middle() {
        let r = rig([0.0, 1.9, 6.0], [0.0, 1.0, 0.0]);
        let inside = read(&r, &one(body(0.0, 0.0, 0.0))).expect("in front");
        assert!(inside.verdict(1).in_safe);
        let edge = read(&r, &one(body(6.4, 0.0, 0.0))).expect("in front");
        assert!(!edge.verdict(1).in_safe, "a body half out is cropped");
        let missing = read(&r, &[body(0.0, 0.0, 0.0), body(0.0, 0.0, 30.0)]).expect("one in front");
        assert!(!missing.verdict(2).in_safe, "a body behind the eye is lost");
    }
}
