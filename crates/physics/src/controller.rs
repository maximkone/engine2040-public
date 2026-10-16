//! controller: Rapier-backed character controller.
//!
//! Kinematic capsule on top of the Rapier [`World`](super::rapier::World)
//! adapter: the inner world mirrors statics plus the resolved capsule
//! translation, so the adapter's snapshot/restore and state hash cover the
//! same bytes. The controller
//! never calls [`World::step`](super::rapier::World::step); movement resolves
//! ONCE per [`Controller::move_and_slide`] call from the requested delta:
//! axis-separated horizontal slide, autostep, slope limit, ground snap,
//! ceiling clamp and airborne gravity.
//!
//! Consumer: arena locomotion. Units: meters, seconds, Y-up.
//! Errors: [`Error`](super::rapier::Error) from adapter validation; per-tick
//! input is sanitized (non-finite becomes zero) and never panics.
//!
//! Determinism scope (HASH-LAW tier note): fixed `dt` per call (canonical
//! `FIXED_DT` from [`super::rapier`]), spawn-order iteration, glue uses only
//! comparisons, `+ - * /`, `sqrt` (wish clamp), `round` (millisecond timers
//! and hash) and integer math. No `sin`/`cos`/`angle_between`, no `Duration`
//! or wall clock. Grade is rise-over-run compared against [`MAX_SLOPE_GRADE`].
#![forbid(unsafe_code)]

use super::rapier::{quantize, Error, Snapshot, World, GRAVITY_Y};
use engine_core::hash::FNV64_PRIME;

/// Maximum climbable grade (rise over run); 1.0 is tan(45 deg) by definition,
/// so no trigonometric call is needed in the glue.
pub const MAX_SLOPE_GRADE: f32 = 1.0;
/// Maximum autostep height, meters.
pub const STEP_HEIGHT: f32 = 0.5;
/// Maximum snap-down distance that keeps the character grounded, meters.
pub const GROUND_SNAP_DISTANCE: f32 = 0.4;
/// Horizontal wish scale while airborne (air control limit).
pub const AIR_CONTROL_SCALE: f32 = 0.3;
/// Upper bound for one tick's requested horizontal delta, meters.
pub const MAX_WISH_PER_TICK: f32 = 1.0;
/// Fastest allowed fall speed, m/s.
pub const MAX_FALL_SPEED: f32 = -20.0;
/// Contact skin kept from walls and ceilings, meters.
pub const SKIN: f32 = 0.02;
/// Coyote grace after leaving walkable ground, milliseconds (Tnua walk basis).
pub const COYOTE_MS: u32 = 150;
/// Jump-input buffer before landing or coyote, milliseconds (Tnua walk basis).
pub const BUFFER_MS: u32 = 150;
/// Downhill slide speed on over-steep ground, m/s.
pub const SLIP_SPEED_M_S: f32 = 4.0;
/// Comparison epsilon, meters.
const EPS: f32 = 1e-6;

/// Autostep lane: max rise plus minimum tread width after the step.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Autostep {
    /// Max step height, meters.
    pub max_height: f32,
    /// Minimum free tread width after the step, meters.
    pub min_width: f32,
}

/// Snap length: absolute meters or fraction of capsule total height.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SnapLength {
    /// Absolute meters.
    Absolute(f32),
    /// Fraction of capsule total height (`2 * half`).
    Relative(f32),
}

impl SnapLength {
    /// Resolve to meters (HASH-LAW ops only); bad input maps to zero.
    pub fn resolve(self, character_height: f32) -> f32 {
        let (v, m) = match self {
            Self::Absolute(v) => (v, 1.0),
            Self::Relative(f) => (f, character_height),
        };
        if v.is_finite() && v > 0.0 && m.is_finite() && m > 0.0 {
            v * m
        } else {
            0.0
        }
    }
}

/// Rapier `move_shape` pattern (PORT dimforge/rapier@475caa3:src/control/character_controller.rs).
/// Caller: `Controller::move_and_slide` (legacy frozen tuning via `MoveShape::legacy`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MoveShape {
    /// Slide along walls when true; stop dead with `wall_normal` when false.
    pub slide: bool,
    /// Autostep lane when `Some`, off when `None`.
    pub autostep: Option<Autostep>,
    /// Snap-to-ground length when `Some`, off when `None`.
    pub snap: Option<SnapLength>,
    /// Max climbable grade (rise/run, 1.0 is 45 deg by definition).
    pub max_climb_grade: f32,
    /// Min grade that slides (rise/run, 1.0 is 45 deg by definition).
    pub min_slide_grade: f32,
    /// Contact skin offset, meters.
    pub offset: f32,
}

impl Default for MoveShape {
    /// Donor defaults: slide on, autostep off, snap 0.2 relative, 45 deg gates, skin 0.01.
    fn default() -> Self {
        Self {
            slide: true,
            autostep: None,
            snap: Some(SnapLength::Relative(0.2)),
            max_climb_grade: 1.0,
            min_slide_grade: 1.0,
            offset: 0.01,
        }
    }
}

impl MoveShape {
    /// Frozen legacy tuning: slide on, 0.5 m step, 0.4 m snap, 45 deg gate, skin 0.02.
    pub fn legacy() -> Self {
        Self {
            slide: true,
            autostep: Some(Autostep {
                max_height: STEP_HEIGHT,
                min_width: 0.0,
            }),
            snap: Some(SnapLength::Absolute(GROUND_SNAP_DISTANCE)),
            max_climb_grade: MAX_SLOPE_GRADE,
            min_slide_grade: MAX_SLOPE_GRADE,
            offset: SKIN,
        }
    }
}

/// Per-tick resolution report: the requested delta against what moved.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Resolved {
    /// Requested horizontal delta for this tick (x, z), meters.
    pub requested: [f32; 2],
    /// Resolved horizontal displacement actually applied (x, z), meters.
    pub displacement: [f32; 2],
    /// Resolved vertical displacement applied (gravity, snap, step, ceiling).
    pub vertical: f32,
    /// True when the character rests on walkable ground after this tick.
    pub grounded: bool,
    /// Outward wall normal (x, z) when a horizontal axis was blocked.
    pub wall_normal: Option<[f32; 2]>,
    /// True when the character stepped up a stair, step box or ramp slice.
    pub stepped: bool,
    /// True when over-steep ground pushed the character downhill this tick.
    pub slipped: bool,
    /// True when on a slope steeper than `min_slide_grade` (donor `is_sliding_down_slope`).
    pub sliding_down_slope: bool,
}

/// Owned snapshot: the Rapier mirror plus the kinematic extras (vertical
/// velocity, grounded flag and the integer-millisecond jump timers).
#[derive(Clone, Debug, PartialEq)]
pub struct ControllerSnapshot {
    world: Snapshot,
    pos: [f32; 3],
    vel_y: f32,
    grounded: bool,
    coyote_ms: u32,
    buffer_ms: u32,
    buffered_vel: f32,
}

/// Axis-aligned static box, min/max corners in meters.
#[derive(Clone, Copy, Debug)]
struct StaticBox {
    min: [f32; 3],
    max: [f32; 3],
}

/// Static wedge ramp rising from `y0` at `x0` to `y1` at `x1`.
#[derive(Clone, Copy, Debug)]
struct Ramp {
    x0: f32,
    x1: f32,
    y0: f32,
    y1: f32,
    z0: f32,
    z1: f32,
}

/// Horizontal probe outcome at one target footprint.
enum Hit {
    /// Nothing in the way.
    Free,
    /// Steppable top height.
    Step(f32),
    /// Blocked: wall, too-tall face or over-steep slope.
    Wall,
}

/// Rapier-backed kinematic character controller (one capsule).
///
/// Walk basis (Tnua): grade-gated slopes (`rise/run <= MAX_SLOPE_GRADE`),
/// downhill slip on over-steep ground, 150 ms coyote plus 150 ms jump
/// buffer in integer milliseconds, gravity on the fixed tick. Units: meters,
/// seconds, Y-up. Per-tick cost is the existing axis/vertical passes plus one
/// ramp scan when grounded (no allocation).
pub struct Controller {
    world: World,
    body: usize,
    pos: [f32; 3],
    radius: f32,
    height: f32,
    vel_y: f32,
    grounded: bool,
    /// Remaining coyote grace, ms. Refreshed while grounded.
    coyote_ms: u32,
    /// Remaining buffered-jump life, ms. Set by [`Controller::jump`].
    buffer_ms: u32,
    /// Take-off velocity stored with the buffer, m/s.
    buffered_vel: f32,
    boxes: Vec<StaticBox>,
    ramps: Vec<Ramp>,
}

impl Controller {
    /// Spawn a Y-up capsule center at `pos`. `height` is the cylinder
    /// mid-section; total height is `height + 2 * radius`. Validation matches
    /// [`World::spawn_capsule`](super::rapier::World::spawn_capsule).
    pub fn spawn(pos: [f32; 3], radius: f32, height: f32) -> Result<Self, Error> {
        let mut world = World::new();
        let body = world.spawn_capsule(pos, radius, height)?;
        Ok(Self {
            world,
            body,
            pos,
            radius,
            height,
            vel_y: 0.0,
            grounded: false,
            // Fresh spawn grants full coyote so an immediate take-off behaves
            // like a grounded jump; the first grounded tick refreshes it.
            coyote_ms: COYOTE_MS,
            buffer_ms: 0,
            buffered_vel: 0.0,
            boxes: Vec::new(),
            ramps: Vec::new(),
        })
    }

    /// Add a static box from half-extents plus center. Mirrored into the
    /// inner Rapier world and into the kinematic resolver.
    pub fn add_static_box(&mut self, half_extents: [f32; 3], pos: [f32; 3]) -> Result<(), Error> {
        self.world.add_static_box(half_extents, pos)?;
        self.boxes.push(StaticBox {
            min: [
                pos[0] - half_extents[0],
                pos[1] - half_extents[1],
                pos[2] - half_extents[2],
            ],
            max: [
                pos[0] + half_extents[0],
                pos[1] + half_extents[1],
                pos[2] + half_extents[2],
            ],
        });
        Ok(())
    }

    /// Add a wedge ramp rising from `y0` at `x0` to `y1` at `x1`, spanning
    /// `z_center +- z_half`. Needs `x1 > x0`, finite heights and
    /// `z_half > 0`; rejected as [`Error::InvalidHalfExtents`] otherwise.
    pub fn add_ramp(
        &mut self,
        x0: f32,
        x1: f32,
        y0: f32,
        y1: f32,
        z_half: f32,
        z_center: f32,
    ) -> Result<(), Error> {
        let finite = x0.is_finite()
            && x1.is_finite()
            && y0.is_finite()
            && y1.is_finite()
            && z_half.is_finite()
            && z_center.is_finite();
        if !finite || x0 >= x1 || z_half <= 0.0 {
            return Err(Error::InvalidHalfExtents);
        }
        self.ramps.push(Ramp {
            x0,
            x1,
            y0,
            y1,
            z0: z_center - z_half,
            z1: z_center + z_half,
        });
        Ok(())
    }

    /// Queue an upward take-off velocity (jump) on the Tnua walk basis.
    /// Non-finite or downward input is ignored. When grounded or inside the
    /// coyote window the take-off applies immediately; otherwise it buffers
    /// for [`BUFFER_MS`] ms and fires on the next grounded or coyote tick.
    /// Determinism: integer-millisecond timers only, no wall clock.
    pub fn jump(&mut self, vel: f32) {
        if !(vel.is_finite() && vel > 0.0) {
            return;
        }
        if self.grounded || self.coyote_ms > 0 {
            self.vel_y = vel;
            self.grounded = false;
            self.coyote_ms = 0;
            self.buffer_ms = 0;
            self.buffered_vel = 0.0;
        } else {
            self.buffered_vel = vel;
            self.buffer_ms = BUFFER_MS;
        }
    }

    /// Capsule center, meters.
    pub fn position(&self) -> [f32; 3] {
        self.pos
    }

    /// Grounded flag from the last tick.
    pub fn grounded(&self) -> bool {
        self.grounded
    }

    /// Current vertical velocity, m/s.
    pub fn velocity_y(&self) -> f32 {
        self.vel_y
    }

    /// Remaining coyote grace, integer milliseconds.
    pub fn coyote_ms(&self) -> u32 {
        self.coyote_ms
    }

    /// Remaining buffered-jump life, integer milliseconds.
    pub fn buffer_ms(&self) -> u32 {
        self.buffer_ms
    }

    /// Resolve ONE tick from the requested horizontal delta `wish_dp`
    /// (x, z in meters for this tick). Legacy frozen tuning via
    /// [`MoveShape::legacy`]; new behavior uses [`Controller::move_and_slide_with`].
    pub fn move_and_slide(&mut self, wish_dp: [f32; 2], dt: f32) -> Resolved {
        self.move_and_slide_with(wish_dp, dt, &MoveShape::legacy())
    }

    /// Resolve ONE tick with explicit Rapier `move_shape` params. Horizontal
    /// axes resolve x-then-z; `slide=false` stops dead on a wall, `slide=true`
    /// keeps the tangential part. Autostep needs grounded-before-step plus
    /// `min_width` tread; snap resolves from [`SnapLength`]; slip reports both
    /// `slipped` and `sliding_down_slope` from `min_slide_grade`.
    pub fn move_and_slide_with(
        &mut self,
        wish_dp: [f32; 2],
        dt: f32,
        shape: &MoveShape,
    ) -> Resolved {
        let requested = sanitize2(wish_dp);
        let start = self.pos;
        let was_grounded = self.grounded;
        let mut clamped = requested;
        let len2 = clamped[0] * clamped[0] + clamped[1] * clamped[1];
        if len2 > MAX_WISH_PER_TICK * MAX_WISH_PER_TICK {
            let len = len2.sqrt();
            clamped[0] = clamped[0] / len * MAX_WISH_PER_TICK;
            clamped[1] = clamped[1] / len * MAX_WISH_PER_TICK;
        }
        if !dt.is_finite() || dt <= 0.0 {
            return Resolved {
                requested,
                displacement: [0.0, 0.0],
                vertical: 0.0,
                grounded: was_grounded,
                wall_normal: None,
                stepped: false,
                slipped: false,
                sliding_down_slope: false,
            };
        }
        let dt_ms = dt_to_ms(dt);
        let fired = self.tick_timers(was_grounded, dt_ms);
        let mut wish = clamped;
        if !was_grounded {
            wish[0] *= AIR_CONTROL_SCALE;
            wish[1] *= AIR_CONTROL_SCALE;
        }
        if !fired {
            self.vel_y += GRAVITY_Y * dt;
            if self.vel_y < MAX_FALL_SPEED {
                self.vel_y = MAX_FALL_SPEED;
            }
        }
        let dy = self.vel_y * dt;
        let half = self.height * 0.5 + self.radius;
        let (wall, stepped) = self.horizontal_pass(wish, half, shape, was_grounded);
        self.vertical_pass(dy, was_grounded, half, shape);
        let slipped = self.slip_pass(dt, half, shape);
        self.world.set_body_translation(self.body, self.pos);
        Resolved {
            requested,
            displacement: [self.pos[0] - start[0], self.pos[2] - start[2]],
            vertical: self.pos[1] - start[1],
            grounded: self.grounded,
            wall_normal: wall,
            stepped,
            slipped,
            sliding_down_slope: slipped,
        }
    }

    /// X-then-Z slide with `MoveShape` gates.
    fn horizontal_pass(
        &mut self,
        wish: [f32; 2],
        half: f32,
        shape: &MoveShape,
        was_grounded: bool,
    ) -> (Option<[f32; 2]>, bool) {
        let max_step = shape.autostep.map_or(0.0, |a| a.max_height);
        let min_width = shape.autostep.map_or(0.0, |a| a.min_width);
        let mut wall: Option<[f32; 2]> = None;
        let mut stepped = false;
        if wish[0] != 0.0 {
            let feet = self.pos[1] - half;
            let head = self.pos[1] + half;
            let nx = self.pos[0] + wish[0];
            let s = if wish[0] > 0.0 { 1.0 } else { -1.0 };
            match self.classify(nx, self.pos[2], feet, head, shape) {
                Hit::Free => self.pos[0] = nx,
                Hit::Step(top)
                    if shape.autostep.is_some()
                        && was_grounded
                        && self.tread_holds(nx, self.pos[2], top, 0, s, min_width, max_step) =>
                {
                    self.pos[0] = nx;
                    self.pos[1] = top + half;
                    stepped = true;
                }
                _ => {
                    if !shape.slide {
                        return (Some([-s, 0.0]), stepped);
                    }
                    self.pos[0] = self.slide_block(0, s, self.pos[0], nx, feet, head, shape);
                    wall = Some([-s, 0.0]);
                }
            }
        }
        if wish[1] != 0.0 {
            let feet = self.pos[1] - half;
            let head = self.pos[1] + half;
            let nz = self.pos[2] + wish[1];
            let s = if wish[1] > 0.0 { 1.0 } else { -1.0 };
            match self.classify(self.pos[0], nz, feet, head, shape) {
                Hit::Free => self.pos[2] = nz,
                Hit::Step(top)
                    if shape.autostep.is_some()
                        && was_grounded
                        && self.tread_holds(self.pos[0], nz, top, 2, s, min_width, max_step) =>
                {
                    self.pos[2] = nz;
                    self.pos[1] = top + half;
                    stepped = true;
                }
                _ => {
                    if !shape.slide {
                        let w = match wall {
                            Some([wx, _]) => [wx, -s],
                            None => [0.0, -s],
                        };
                        return (Some(w), stepped);
                    }
                    self.pos[2] = self.slide_block(2, s, self.pos[2], nz, feet, head, shape);
                    wall = Some(match wall {
                        Some([wx, _]) => [wx, -s],
                        None => [0.0, -s],
                    });
                }
            }
        }
        (wall, stepped)
    }

    /// Clamp `cur` toward `target` at the wall face plus skin.
    #[expect(
        clippy::too_many_arguments,
        reason = "axis-separated wall clamp keeps its inputs flat until a move params struct lands"
    )]
    fn slide_block(
        &self,
        axis: usize,
        sign: f32,
        cur: f32,
        target: f32,
        feet: f32,
        head: f32,
        shape: &MoveShape,
    ) -> f32 {
        if let Some(face) = self.face_ahead(axis, sign, feet, head, shape) {
            let c = face - sign * (self.radius + shape.offset);
            if sign > 0.0 {
                cur.max(c.min(target))
            } else {
                cur.min(c.max(target))
            }
        } else {
            cur
        }
    }

    /// Tread check: step top must continue `min_width` ahead.
    #[expect(
        clippy::too_many_arguments,
        reason = "tread probe keeps its inputs flat until a move params struct lands"
    )]
    fn tread_holds(
        &self,
        x: f32,
        z: f32,
        top: f32,
        axis: usize,
        sign: f32,
        min_width: f32,
        max_step: f32,
    ) -> bool {
        if !(min_width.is_finite() && min_width > EPS) {
            return true;
        }
        let (ax, az) = if axis == 0 {
            (x + sign * min_width, z)
        } else {
            (x, z + sign * min_width)
        };
        match self.ground_height(ax, az, top, max_step) {
            Some(g) => (g - top).abs() <= 0.05,
            None => false,
        }
    }

    /// Land, snap down, or clamp against the ceiling from `dy` this tick.
    fn vertical_pass(&mut self, dy: f32, was_grounded: bool, half: f32, shape: &MoveShape) {
        let feet = self.pos[1] - half;
        let head = self.pos[1] + half;
        let max_rise = shape.autostep.map_or(0.0, |a| a.max_height);
        let snap_dist = shape.snap.map_or(0.0, |s| s.resolve(2.0 * half));
        if dy <= 0.0 {
            match self.ground_height(self.pos[0], self.pos[2], feet, max_rise) {
                Some(g) if feet + dy <= g => {
                    self.pos[1] = g + half;
                    self.vel_y = 0.0;
                    self.grounded = true;
                }
                Some(g) if was_grounded && feet - g >= 0.0 && feet - g <= snap_dist => {
                    self.pos[1] = g + half;
                    self.vel_y = 0.0;
                    self.grounded = true;
                }
                _ => {
                    self.pos[1] += dy;
                    self.grounded = false;
                }
            }
        } else if let Some(c) = self.ceiling_height(self.pos[0], self.pos[2], head, shape.offset) {
            if head + dy >= c {
                // Ceiling blocks upward motion; velocity is consumed.
                self.pos[1] = c - half - shape.offset;
                self.vel_y = 0.0;
                self.grounded = false;
            } else {
                self.pos[1] += dy;
                self.grounded = false;
            }
        } else {
            self.pos[1] += dy;
            self.grounded = false;
        }
    }

    /// Slope-gated downhill slip (grade above `min_slide_grade`).
    /// Grade is rise-over-run, so no trigonometric call is needed.
    /// Returns true when slip moved the capsule (`slipped` and `sliding_down_slope`).
    fn slip_pass(&mut self, dt: f32, half: f32, shape: &MoveShape) -> bool {
        if !self.grounded {
            return false;
        }
        let feet = self.pos[1] - half;
        let snap_dist = shape.snap.map_or(0.0, |s| s.resolve(2.0 * half));
        let Some(grade) = self.steep_grade_at(
            self.pos[0],
            self.pos[2],
            feet,
            shape.min_slide_grade,
            snap_dist,
        ) else {
            return false;
        };
        let downhill = if grade > 0.0 { -1.0 } else { 1.0 };
        self.pos[0] += downhill * SLIP_SPEED_M_S * dt;
        true
    }

    /// Copy out the full future-affecting state (Rapier mirror plus extras).
    pub fn snapshot(&self) -> ControllerSnapshot {
        ControllerSnapshot {
            world: self.world.snapshot(),
            pos: self.pos,
            vel_y: self.vel_y,
            grounded: self.grounded,
            coyote_ms: self.coyote_ms,
            buffer_ms: self.buffer_ms,
            buffered_vel: self.buffered_vel,
        }
    }

    /// Restore a snapshot; resuming with the same `dt` and wish sequence is
    /// bit-identical to the recorded run.
    pub fn restore(&mut self, snapshot: ControllerSnapshot) -> Result<(), Error> {
        self.world.restore(snapshot.world)?;
        self.pos = snapshot.pos;
        self.vel_y = snapshot.vel_y;
        self.grounded = snapshot.grounded;
        self.coyote_ms = snapshot.coyote_ms;
        self.buffer_ms = snapshot.buffer_ms;
        self.buffered_vel = snapshot.buffered_vel;
        self.world.set_body_translation(self.body, self.pos);
        Ok(())
    }

    /// Canonical hash: the Rapier mirror hash folded with vertical velocity,
    /// the grounded flag and the integer-millisecond jump timers (all affect
    /// future ticks). The tail folds whole words plus one prime step each,
    /// exactly like before, with the prime owned by the shared hash module.
    pub fn state_hash(&self) -> u64 {
        let mut h = self.world.state_hash();
        for v in [
            u64::from_le_bytes(quantize(self.vel_y).to_le_bytes()),
            u64::from(self.grounded),
            u64::from(self.coyote_ms),
            u64::from(self.buffer_ms),
            u64::from_le_bytes(quantize(self.buffered_vel).to_le_bytes()),
        ] {
            h ^= v;
            h = h.wrapping_mul(FNV64_PRIME);
        }
        h
    }

    /// Tick the integer-millisecond timers. Returns true when a buffered
    /// take-off fires on this tick (velocity set, gravity skipped by caller).
    fn tick_timers(&mut self, was_grounded: bool, dt_ms: u32) -> bool {
        if was_grounded {
            self.coyote_ms = COYOTE_MS;
        } else {
            self.coyote_ms = self.coyote_ms.saturating_sub(dt_ms);
        }
        if self.buffer_ms == 0 {
            return false;
        }
        if was_grounded || self.coyote_ms > 0 {
            self.vel_y = self.buffered_vel;
            self.grounded = false;
            self.coyote_ms = 0;
            self.buffer_ms = 0;
            self.buffered_vel = 0.0;
            return true;
        }
        self.buffer_ms = self.buffer_ms.saturating_sub(dt_ms);
        false
    }

    /// Over-steep ramp grade under the footprint, if the capsule stands on
    /// one. `Some` only when the surface is within snap distance below the
    /// feet and steeper than `min_grade`.
    fn steep_grade_at(
        &self,
        x: f32,
        z: f32,
        feet: f32,
        min_grade: f32,
        snap_dist: f32,
    ) -> Option<f32> {
        for r in &self.ramps {
            let Some((h, grade)) = ramp_height_at(r, x, z) else {
                continue;
            };
            if grade <= min_grade {
                continue;
            }
            // Standing contact: surface at or just below the feet.
            if h <= feet + 0.05 && h >= feet - snap_dist - 0.05 {
                return Some(grade);
            }
        }
        None
    }

    /// Classify the capsule footprint at `(x, z)` with the given feet/head.
    fn classify(&self, x: f32, z: f32, feet: f32, head: f32, shape: &MoveShape) -> Hit {
        let max_step = shape.autostep.map_or(0.0, |a| a.max_height);
        let mut wall = false;
        let mut step_top: Option<f32> = None;
        // Wedge ramps: grade gate first, so steep faces block even in slices
        // thinner than max step; climbable slices step up gradually.
        for r in &self.ramps {
            let Some((h, grade)) = ramp_height_at(r, x, z) else {
                continue;
            };
            if h <= feet + EPS {
                continue;
            }
            if h - feet > max_step + EPS || grade > shape.max_climb_grade {
                wall = true;
            } else {
                step_top = Some(step_top.map_or(h, |t: f32| t.max(h)));
            }
        }
        for b in &self.boxes {
            if !footprint_overlaps(b, x, z, self.radius) {
                continue;
            }
            let top = b.max[1];
            let bottom = b.min[1];
            if top <= feet + EPS {
                continue; // Ground underfoot; the vertical pass owns it.
            }
            if bottom >= head - EPS {
                continue; // Overhead; the ceiling pass owns it.
            }
            if top - feet <= max_step + EPS && self.headroom_free(x, z, top, shape.offset) {
                step_top = Some(step_top.map_or(top, |t: f32| t.max(top)));
            } else {
                wall = true;
            }
        }
        if wall {
            Hit::Wall
        } else if let Some(top) = step_top {
            Hit::Step(top)
        } else {
            Hit::Free
        }
    }

    /// Nearest strict-wall face ahead on `axis` (0 = x, 2 = z) in direction
    /// `sign`, for slide-to-contact. Steppable boxes are skipped: the caller
    /// steps onto those instead of stopping.
    fn face_ahead(
        &self,
        axis: usize,
        sign: f32,
        feet: f32,
        head: f32,
        shape: &MoveShape,
    ) -> Option<f32> {
        let max_step = shape.autostep.map_or(0.0, |a| a.max_height);
        let here = if axis == 0 { self.pos[0] } else { self.pos[2] };
        let mut face: Option<f32> = None;
        for b in &self.boxes {
            let other = if axis == 0 { self.pos[2] } else { self.pos[0] };
            let (omin, omax) = if axis == 0 {
                (b.min[2], b.max[2])
            } else {
                (b.min[0], b.max[0])
            };
            if !(other + self.radius > omin && other - self.radius < omax) {
                continue;
            }
            if !(b.min[1] < head - shape.offset && b.max[1] > feet + max_step) {
                continue; // Not a strict wall (ground, step or ceiling).
            }
            let f = if axis == 0 { b.min[0] } else { b.min[2] };
            let g = if axis == 0 { b.max[0] } else { b.max[2] };
            if sign > 0.0 {
                if f >= here - self.radius {
                    face = Some(match face {
                        Some(prev) => prev.min(f),
                        None => f,
                    });
                }
            } else if g <= here + self.radius {
                face = Some(match face {
                    Some(prev) => prev.max(g),
                    None => g,
                });
            }
        }
        face
    }

    /// Highest reachable ground top under the footprint, if any.
    fn ground_height(&self, x: f32, z: f32, feet: f32, max_rise: f32) -> Option<f32> {
        let mut g: Option<f32> = None;
        let m = self.radius * 0.5;
        for b in &self.boxes {
            if !(x + m > b.min[0] && x - m < b.max[0] && z + m > b.min[2] && z - m < b.max[2]) {
                continue;
            }
            let top = b.max[1];
            if top <= feet + max_rise + EPS {
                g = Some(g.map_or(top, |v: f32| v.max(top)));
            }
        }
        for r in &self.ramps {
            if let Some((h, _)) = ramp_height_at(r, x, z) {
                if h <= feet + max_rise + EPS {
                    g = Some(g.map_or(h, |v: f32| v.max(h)));
                }
            }
        }
        g
    }

    /// Lowest ceiling bottom above the head overlapping the footprint, if any.
    fn ceiling_height(&self, x: f32, z: f32, head: f32, offset: f32) -> Option<f32> {
        let mut c: Option<f32> = None;
        let m = self.radius * 0.5;
        for b in &self.boxes {
            if !(x + m > b.min[0] && x - m < b.max[0] && z + m > b.min[2] && z - m < b.max[2]) {
                continue;
            }
            let bottom = b.min[1];
            if bottom >= head - offset && b.max[1] > head {
                c = Some(c.map_or(bottom, |v: f32| v.min(bottom)));
            }
        }
        c
    }

    /// Headroom check for stepping onto `top`: no box may cap the new head.
    fn headroom_free(&self, x: f32, z: f32, top: f32, offset: f32) -> bool {
        let new_head = top + 2.0 * (self.height * 0.5 + self.radius);
        for b in &self.boxes {
            if !footprint_overlaps(b, x, z, self.radius) {
                continue;
            }
            if b.min[1] < new_head - offset && b.max[1] > top + offset {
                return false;
            }
        }
        true
    }
}

/// Capsule footprint (XZ circle) against a box, expanded by `radius`.
fn footprint_overlaps(b: &StaticBox, x: f32, z: f32, radius: f32) -> bool {
    x + radius > b.min[0] && x - radius < b.max[0] && z + radius > b.min[2] && z - radius < b.max[2]
}

/// Ramp surface height plus grade at `(x, z)`; `None` outside the wedge.
fn ramp_height_at(r: &Ramp, x: f32, z: f32) -> Option<(f32, f32)> {
    if x < r.x0 || x > r.x1 || z < r.z0 || z > r.z1 {
        return None;
    }
    let span = r.x1 - r.x0;
    if span <= 0.0 {
        return None;
    }
    let t = (x - r.x0) / span;
    let grade = (r.y1 - r.y0) / span;
    Some((r.y0 + (r.y1 - r.y0) * t, grade))
}

/// Replace non-finite wish components with zero so bad input never moves.
fn sanitize2(v: [f32; 2]) -> [f32; 2] {
    [
        if v[0].is_finite() { v[0] } else { 0.0 },
        if v[1].is_finite() { v[1] } else { 0.0 },
    ]
}

/// Quantize a validated positive `dt` (seconds) to whole milliseconds for the
/// integer coyote/buffer timers. Canonical `FIXED_DT` maps to 17 ms; the
/// result saturates to 1..=1000 so a large fixed step cannot wrap a timer.
/// Never panics: non-finite input maps to 1, out-of-range input saturates.
/// The float-to-integer step uses only comparisons plus `From` conversions
/// (exact for 1..=1000), so no cast and no fallible conversion remain.
fn dt_to_ms(dt: f32) -> u32 {
    let ms = (dt * 1000.0).round();
    if !ms.is_finite() {
        return 1;
    }
    if ms <= 1.0 {
        return 1;
    }
    if ms >= 1000.0 {
        return 1000;
    }
    // `ms` is finite with 1.0 < ms < 1000.0; its ceiling is the timer value.
    // Binary search over the exact `u16` range keeps behavior identical to
    // the old saturating cast for every input, deterministically.
    let mut lo: u16 = 1;
    let mut hi: u16 = 1000;
    while lo < hi {
        let mid: u16 = (lo + hi) / 2;
        if ms <= f32::from(mid) {
            hi = mid;
        } else {
            lo = mid + 1;
        }
    }
    u32::from(lo)
}

#[cfg(test)]
mod tests {
    use super::super::rapier::FIXED_DT;
    use super::*;

    const RADIUS: f32 = 0.3;
    const HEIGHT: f32 = 0.8;
    const HALF: f32 = HEIGHT * 0.5 + RADIUS; // 0.7 capsule half-height.

    /// Declared slope/step/ground-snap course: floor, climbable ramp
    /// (grade 0.5), a 0.3 step ledge and an upper deck.
    fn course() -> Controller {
        let mut c = Controller::spawn([-2.0, HALF, 0.0], RADIUS, HEIGHT).expect("spawn");
        // Floor top y=0.
        c.add_static_box([10.0, 0.5, 3.0], [0.0, -0.5, 0.0])
            .expect("floor");
        // Ramp x[2,4] rising 0->1 (grade 0.5, under the 45-degree gate).
        c.add_ramp(2.0, 4.0, 0.0, 1.0, 3.0, 0.0).expect("ramp");
        // Step ledge x[4,5] top 1.3 (0.3 above the ramp exit).
        c.add_static_box([0.5, 0.65, 3.0], [4.5, 0.65, 0.0])
            .expect("step");
        // Upper deck x[5,9] top 1.3.
        c.add_static_box([2.0, 0.65, 3.0], [7.0, 0.65, 0.0])
            .expect("deck");
        c
    }

    fn walk(c: &mut Controller, wish: [f32; 2], n: usize) -> Vec<Resolved> {
        let mut out = Vec::with_capacity(n);
        for _ in 0..n {
            out.push(c.move_and_slide(wish, FIXED_DT));
        }
        out
    }

    #[test]
    fn constants_declared() {
        // Compile-time pins for the packet's declared tuning.
        const { assert!(STEP_HEIGHT == 0.5) };
        const { assert!(GROUND_SNAP_DISTANCE == 0.4) };
        // 45 degrees is grade 1 by definition; the glue stores the grade
        // literal so no trigonometric call is needed.
        const { assert!(MAX_SLOPE_GRADE == 1.0) };
        const { assert!(AIR_CONTROL_SCALE > 0.0 && AIR_CONTROL_SCALE < 1.0) };
        const { assert!(COYOTE_MS == 150) };
        const { assert!(BUFFER_MS == 150) };
    }

    #[test]
    fn course_climbs_steps_and_stays_grounded() {
        let mut c = course();
        // 64 ticks at 0.15 m lands mid-deck (deck spans x 5..9).
        let trace = walk(&mut c, [0.15, 0.0], 64);
        assert!(
            trace.iter().any(|r| r.stepped),
            "course must autostep at least once"
        );
        let last = trace.last().expect("trace");
        assert!(last.grounded, "deck walk ends grounded");
        let p = c.position();
        assert!(p[0] > 5.0, "must cross ramp+step, x={}", p[0]);
        assert!(p[0] < 9.0, "must still be on the deck, x={}", p[0]);
        assert!(
            (p[1] - (1.3 + HALF)).abs() < 0.05,
            "must stand on the deck, y={}",
            p[1]
        );
        assert!(p.iter().all(|v| v.is_finite()));
        // Same course twice is identical (local reproducibility).
        let run = || {
            let mut c = course();
            walk(&mut c, [0.15, 0.0], 64);
            (c.state_hash(), c.position())
        };
        assert_eq!(run(), run());
    }

    #[test]
    fn steep_slope_blocks_instead_of_climbing() {
        let mut c = Controller::spawn([-2.0, HALF, 0.0], RADIUS, HEIGHT).expect("spawn");
        c.add_static_box([10.0, 0.5, 3.0], [0.0, -0.5, 0.0])
            .expect("floor");
        // Grade 2.0: twice the 45-degree gate, must refuse to climb.
        c.add_ramp(2.0, 3.0, 0.0, 2.0, 3.0, 0.0).expect("steep");
        let trace = walk(&mut c, [0.15, 0.0], 60);
        let p = c.position();
        assert!(p[0] < 2.4, "steep face must block, x={}", p[0]);
        assert!(
            trace.iter().any(|r| r.wall_normal.is_some()),
            "steep face reports a wall"
        );
        assert!(
            !trace.iter().any(|r| r.stepped),
            "steep face must never autostep"
        );
    }

    #[test]
    fn wall_slide_preserves_tangential_motion() {
        let mut c = Controller::spawn([2.0, HALF, 0.0], RADIUS, HEIGHT).expect("spawn");
        c.add_static_box([10.0, 0.5, 10.0], [0.0, -0.5, 0.0])
            .expect("floor");
        // Wall face at x=4.75, tall in y and long in z.
        c.add_static_box([0.25, 2.0, 10.0], [5.0, 2.0, 0.0])
            .expect("wall");
        let trace = walk(&mut c, [0.2, 0.2], 40);
        let p = c.position();
        assert!(
            p[0] + RADIUS <= 4.75 + 0.05,
            "must not cross the wall, x={}",
            p[0]
        );
        assert!(p[2] > 4.0, "tangential motion must continue, z={}", p[2]);
        let last = trace.last().expect("trace");
        assert_eq!(last.requested, [0.2, 0.2]);
        assert!(last.displacement[0].abs() < 1e-6, "{last:?}");
        assert!(
            (last.displacement[1] - 0.2).abs() < 1e-6,
            "slide keeps tangential delta, {last:?}"
        );
        assert_eq!(last.wall_normal, Some([-1.0, 0.0]));
        assert!(last.grounded, "slide stays on the floor");
    }

    #[test]
    fn requested_vs_resolved_reported_into_wall() {
        let mut c = Controller::spawn([4.0, HALF, 0.0], RADIUS, HEIGHT).expect("spawn");
        c.add_static_box([10.0, 0.5, 10.0], [0.0, -0.5, 0.0])
            .expect("floor");
        c.add_static_box([0.25, 2.0, 10.0], [5.0, 2.0, 0.0])
            .expect("wall");
        // One tick must already be grounded from the settle below.
        c.move_and_slide([0.0, 0.0], FIXED_DT);
        let r = c.move_and_slide([0.9, 0.1], FIXED_DT);
        assert_eq!(r.requested, [0.9, 0.1]);
        assert!(
            r.displacement[0] < r.requested[0] - 0.1,
            "wall must shorten x, {r:?}"
        );
        assert!(
            (r.displacement[1] - r.requested[1]).abs() < 1e-6,
            "z passes through, {r:?}"
        );
        assert!(r.wall_normal.is_some());
    }

    #[test]
    fn ceiling_blocks_upward_motion() {
        let mut c = Controller::spawn([0.0, HALF, 0.0], RADIUS, HEIGHT).expect("spawn");
        c.add_static_box([10.0, 0.5, 10.0], [0.0, -0.5, 0.0])
            .expect("floor");
        // Ceiling slab bottom at y=2.75.
        c.add_static_box([3.0, 0.25, 3.0], [0.0, 3.0, 0.0])
            .expect("ceiling");
        c.jump(6.0);
        let mut max_y = 0.0f32;
        for _ in 0..120 {
            c.move_and_slide([0.0, 0.0], FIXED_DT);
            max_y = max_y.max(c.position()[1]);
        }
        // The head may approach the slab but never penetrate it: the clamp
        // fires when the next step would cross, so the cap is touching
        // distance (bottom minus half-height). Free apex would be 2.53 m,
        // well above the slab, so staying under the cap proves the block.
        let cap = 2.75 - HALF;
        assert!(max_y > 1.9, "jump must reach the ceiling, max_y={max_y}");
        assert!(
            max_y <= cap + 1e-4,
            "ceiling caps the rise at {cap}, max_y={max_y}"
        );
        assert!(c.grounded, "must land again after the jump");
        assert!(c.position()[1].is_finite());
    }

    #[test]
    fn air_control_limited() {
        // Grounded reference: settle, then one wish tick.
        let mut g = Controller::spawn([0.0, HALF, 0.0], RADIUS, HEIGHT).expect("spawn");
        g.add_static_box([10.0, 0.5, 10.0], [0.0, -0.5, 0.0])
            .expect("floor");
        g.move_and_slide([0.0, 0.0], FIXED_DT);
        assert!(g.grounded());
        let gr = g.move_and_slide([0.4, 0.0], FIXED_DT);
        // Airborne twin: same wish while falling.
        let mut a = Controller::spawn([0.0, 5.0, 0.0], RADIUS, HEIGHT).expect("spawn");
        a.add_static_box([10.0, 0.5, 10.0], [0.0, -0.5, 0.0])
            .expect("floor");
        assert!(!a.grounded());
        let ar = a.move_and_slide([0.4, 0.0], FIXED_DT);
        assert_eq!(ar.requested, [0.4, 0.0]);
        assert!(
            (ar.displacement[0] - 0.4 * AIR_CONTROL_SCALE).abs() < 1e-6,
            "{ar:?}"
        );
        assert!(
            ar.displacement[0] < gr.displacement[0] - 1e-6,
            "air must move less than ground, {ar:?} vs {gr:?}"
        );
    }

    #[test]
    fn snapshot_resimulate_identical_x2() {
        let mut c = course();
        walk(&mut c, [0.15, 0.0], 20);
        let snap = c.snapshot();
        walk(&mut c, [0.15, 0.0], 20);
        let (h1, p1) = (c.state_hash(), c.position());
        c.restore(snap.clone()).expect("restore");
        // Restored state matches the snapshot exactly.
        assert_eq!(c.snapshot(), snap);
        assert_eq!(c.position(), snap.pos);
        walk(&mut c, [0.15, 0.0], 20);
        let (h2, p2) = (c.state_hash(), c.position());
        assert_eq!((h1, p1), (h2, p2));
    }

    #[test]
    fn invalid_input_never_panics() {
        assert!(Controller::spawn([0.0, 1.0, 0.0], 0.0, 1.0).is_err());
        assert!(Controller::spawn([0.0, 1.0, 0.0], RADIUS, f32::NAN).is_err());
        assert!(Controller::spawn([0.0, f32::INFINITY, 0.0], RADIUS, HEIGHT).is_err());
        let mut c = Controller::spawn([0.0, HALF, 0.0], RADIUS, HEIGHT).expect("spawn");
        assert!(c.add_ramp(3.0, 3.0, 0.0, 1.0, 1.0, 0.0).is_err());
        assert!(c.add_ramp(4.0, 2.0, 0.0, 1.0, 1.0, 0.0).is_err());
        assert!(c.add_static_box([0.0, 0.5, 0.5], [0.0, 0.0, 0.0]).is_err());
        // NaN wish sanitizes to zero motion; bad dt holds state.
        let r = c.move_and_slide([f32::NAN, f32::INFINITY], FIXED_DT);
        assert_eq!(r.requested, [0.0, 0.0]);
        assert_eq!(r.displacement, [0.0, 0.0]);
        for bad in [0.0, -FIXED_DT, f32::NAN, f32::INFINITY] {
            let before = c.position();
            let r = c.move_and_slide([0.3, 0.0], bad);
            assert_eq!(r.displacement, [0.0, 0.0], "dt={bad}");
            assert_eq!(c.position(), before, "dt={bad}");
        }
        // NaN jump is ignored on a fresh controller, never stored.
        let mut j = Controller::spawn([0.0, HALF, 0.0], RADIUS, HEIGHT).expect("spawn");
        j.jump(f32::NAN);
        j.jump(-2.0);
        assert_eq!(j.velocity_y(), 0.0);
        // Restoring an own snapshot after motion returns exactly.
        let snap = c.snapshot();
        c.move_and_slide([0.3, 0.0], FIXED_DT);
        c.restore(snap.clone()).expect("restore");
        assert_eq!(c.snapshot(), snap);
    }

    #[test]
    fn hash_matches_shared_fnv_bytes() {
        use engine_core::hash::FNV64_PRIME;
        fn legacy(c: &Controller) -> u64 {
            let mut h = c.world.state_hash();
            for v in [
                u64::from_le_bytes(quantize(c.vel_y).to_le_bytes()),
                u64::from(c.grounded),
                u64::from(c.coyote_ms),
                u64::from(c.buffer_ms),
                u64::from_le_bytes(quantize(c.buffered_vel).to_le_bytes()),
            ] {
                h ^= v;
                h = h.wrapping_mul(FNV64_PRIME);
            }
            h
        }
        // Fresh, walked and airborne states cover both flag values.
        let fresh = course();
        assert_eq!(fresh.state_hash(), legacy(&fresh));
        let mut c = course();
        walk(&mut c, [0.15, 0.0], 20);
        assert_eq!(c.state_hash(), legacy(&c));
        c.jump(5.0);
        c.move_and_slide([0.0, 0.0], FIXED_DT);
        assert_eq!(c.state_hash(), legacy(&c));
    }

    #[test]
    fn grade_45_holds_46_slips_hash_x2() {
        // 45-degree gate is grade 1.0 by definition: a grade-1.0 ramp holds,
        // anything above it slips downhill. No trigonometry in the glue.
        let hold_run = || {
            let mut c = Controller::spawn([0.5, 0.5 + HALF, 0.0], RADIUS, HEIGHT).expect("spawn");
            c.add_static_box([5.0, 0.5, 3.0], [0.0, -0.5, 0.0])
                .expect("floor");
            c.add_ramp(0.0, 1.0, 0.0, 1.0, 3.0, 0.0).expect("45deg");
            let mut slipped = false;
            for _ in 0..30 {
                let r = c.move_and_slide([0.0, 0.0], FIXED_DT);
                slipped = slipped || r.slipped;
            }
            (c.state_hash(), c.position(), c.grounded(), slipped)
        };
        let (h1, p1, g1, s1) = hold_run();
        let (h2, p2, g2, s2) = hold_run();
        assert_eq!((h1, p1), (h2, p2), "hold run must hash x2");
        assert!(g1 && g2, "45-degree ramp must stay grounded");
        assert!(!s1 && !s2, "45-degree ramp must never slip");
        assert!((p1[0] - 0.5).abs() < 1e-6, "hold keeps x, {p1:?}");

        let slip_run = || {
            let mut c = Controller::spawn([0.5, 0.53 + HALF, 0.0], RADIUS, HEIGHT).expect("spawn");
            c.add_static_box([5.0, 0.5, 3.0], [0.0, -0.5, 0.0])
                .expect("floor");
            // Grade 1.06: just past the 46-degree line, must slip.
            c.add_ramp(0.0, 1.0, 0.0, 1.06, 3.0, 0.0).expect("steep");
            let mut slipped = false;
            for _ in 0..30 {
                let r = c.move_and_slide([0.0, 0.0], FIXED_DT);
                slipped = slipped || r.slipped;
            }
            (c.state_hash(), c.position(), slipped)
        };
        let (a1, q1, f1) = slip_run();
        let (a2, q2, f2) = slip_run();
        assert_eq!((a1, q1), (a2, q2), "slip run must hash x2");
        assert!(f1 && f2, "over-steep ground must report slip");
        assert!(q1[0] < 0.5 - 0.2, "must slide downhill, {q1:?}");
    }

    #[test]
    fn stairs_half_meter_steps() {
        // STEP_HEIGHT boundary: a 0.5 m face steps up, 0.6 m blocks.
        let mut c = Controller::spawn([-1.0, HALF, 0.0], RADIUS, HEIGHT).expect("spawn");
        c.add_static_box([10.0, 0.5, 3.0], [0.0, -0.5, 0.0])
            .expect("floor");
        // Ledge top exactly 0.5 above the floor.
        c.add_static_box([1.0, 0.25, 3.0], [3.0, 0.25, 0.0])
            .expect("step");
        c.move_and_slide([0.0, 0.0], FIXED_DT);
        let trace = walk(&mut c, [0.15, 0.0], 40);
        assert!(trace.iter().any(|r| r.stepped), "0.5 m face must step");
        let p = c.position();
        assert!(
            (p[1] - (0.5 + HALF)).abs() < 0.05,
            "must stand on the 0.5 m ledge, y={}",
            p[1]
        );

        let mut tall = Controller::spawn([-1.0, HALF, 0.0], RADIUS, HEIGHT).expect("spawn");
        tall.add_static_box([10.0, 0.5, 3.0], [0.0, -0.5, 0.0])
            .expect("floor");
        tall.add_static_box([1.0, 0.3, 3.0], [3.0, 0.3, 0.0])
            .expect("tall");
        tall.move_and_slide([0.0, 0.0], FIXED_DT);
        let trace = walk(&mut tall, [0.15, 0.0], 40);
        assert!(
            !trace.iter().any(|r| r.stepped),
            "0.6 m face must never step"
        );
        assert!(tall.position()[0] < 2.0, "0.6 m face must block");
    }

    #[test]
    fn snap_four_tenths_holds() {
        // Deck top 0.4 above the floor: walking off snaps down grounded.
        let mut c = Controller::spawn([-4.0, 0.4 + HALF, 0.0], RADIUS, HEIGHT).expect("spawn");
        c.add_static_box([2.0, 0.2, 3.0], [-3.0, 0.2, 0.0])
            .expect("deck");
        c.add_static_box([5.0, 0.5, 3.0], [2.5, -0.5, 0.0])
            .expect("floor");
        c.move_and_slide([0.0, 0.0], FIXED_DT);
        assert!(c.grounded(), "deck start grounds");
        let trace = walk(&mut c, [0.15, 0.0], 40);
        assert!(trace.last().expect("trace").grounded, "0.4 m drop snaps");
        let p = c.position();
        assert!(
            (p[1] - HALF).abs() < 0.05,
            "must rest on the low floor, y={}",
            p[1]
        );
        assert!(p[0] > 0.0, "must cross the seam, x={}", p[0]);
    }

    #[test]
    fn coyote_fires_after_leaving_ground() {
        // Walk off a deck, then take off inside the 150 ms coyote window.
        let mut c = Controller::spawn([-4.0, 1.0 + HALF, 0.0], RADIUS, HEIGHT).expect("spawn");
        c.add_static_box([3.0, 0.5, 3.0], [-2.0, 0.5, 0.0])
            .expect("deck");
        c.add_static_box([5.0, 0.5, 10.0], [0.0, -2.5, 0.0])
            .expect("catch");
        c.move_and_slide([0.0, 0.0], FIXED_DT);
        assert!(c.grounded());
        // Walk to the edge and one step past it.
        let mut left_ground = false;
        for _ in 0..30 {
            c.move_and_slide([0.2, 0.0], FIXED_DT);
            if !c.grounded() {
                left_ground = true;
                break;
            }
        }
        assert!(left_ground, "must walk off the deck");
        assert!(c.coyote_ms() > 0, "coyote must still be live");
        assert!(c.coyote_ms() <= COYOTE_MS);
        c.jump(5.0);
        c.move_and_slide([0.0, 0.0], FIXED_DT);
        assert!(c.velocity_y() > 0.0, "coyote take-off must rise");
        assert!(!c.grounded());
        // Same walk with no jump falls instead (oracle, not a restatement).
        let mut fall = Controller::spawn([-4.0, 1.0 + HALF, 0.0], RADIUS, HEIGHT).expect("spawn");
        fall.add_static_box([3.0, 0.5, 3.0], [-2.0, 0.5, 0.0])
            .expect("deck");
        fall.add_static_box([5.0, 0.5, 10.0], [0.0, -2.5, 0.0])
            .expect("catch");
        fall.move_and_slide([0.0, 0.0], FIXED_DT);
        for _ in 0..30 {
            fall.move_and_slide([0.2, 0.0], FIXED_DT);
            if !fall.grounded() {
                break;
            }
        }
        let y_before = fall.position()[1];
        fall.move_and_slide([0.0, 0.0], FIXED_DT);
        assert!(fall.position()[1] < y_before, "no jump must keep falling");
    }

    #[test]
    fn buffer_fires_on_landing() {
        // Press mid-fall; the buffered take-off fires after touchdown.
        let mut c = Controller::spawn([0.0, 3.0, 0.0], RADIUS, HEIGHT).expect("spawn");
        c.add_static_box([10.0, 0.5, 10.0], [0.0, -0.5, 0.0])
            .expect("floor");
        let mut pressed = false;
        let mut fired = false;
        for _ in 0..120 {
            let p = c.position();
            // Press once while close to the floor and still falling.
            if !pressed && !c.grounded() && p[1] - HALF < 0.6 && c.velocity_y() < 0.0 {
                c.jump(5.0);
                pressed = true;
            }
            c.move_and_slide([0.0, 0.0], FIXED_DT);
            if pressed && c.velocity_y() > 0.0 {
                fired = true;
                break;
            }
        }
        assert!(pressed, "must press inside the fall");
        assert!(fired, "buffered press must take off after landing");
        assert!(c.buffer_ms() == 0, "buffer clears on fire");
    }

    #[test]
    fn slide_flag_stops_vs_slides() {
        let mk = || {
            let mut c = Controller::spawn([2.0, HALF, 0.0], RADIUS, HEIGHT).expect("spawn");
            c.add_static_box([10.0, 0.5, 10.0], [0.0, -0.5, 0.0])
                .expect("floor");
            c.add_static_box([0.25, 2.0, 10.0], [5.0, 2.0, 0.0])
                .expect("wall");
            c.move_and_slide([0.0, 0.0], FIXED_DT);
            c
        };
        let mut a = mk();
        for _ in 0..40 {
            a.move_and_slide_with([0.2, 0.2], FIXED_DT, &MoveShape::legacy());
        }
        assert!(
            a.position()[2] > 4.0,
            "slide keeps tangential, {:?}",
            a.position()
        );
        let mut b = mk();
        let mut noslide = MoveShape::legacy();
        noslide.slide = false;
        for _ in 0..40 {
            b.move_and_slide_with([0.2, 0.2], FIXED_DT, &noslide);
        }
        assert!(
            b.position()[2] < 3.0,
            "no slide stops dead, {:?}",
            b.position()
        );
        let r = b.move_and_slide_with([0.2, 0.0], FIXED_DT, &noslide);
        assert!(r.displacement[0].abs() < 1e-6, "{r:?}");
        assert!(r.wall_normal.is_some(), "{r:?}");
    }

    #[test]
    fn autostep_snap_grades() {
        let mkledge = || {
            let mut c = Controller::spawn([-1.0, HALF, 0.0], RADIUS, HEIGHT).expect("spawn");
            c.add_static_box([10.0, 0.5, 3.0], [0.0, -0.5, 0.0])
                .expect("floor");
            c.add_static_box([1.0, 0.15, 3.0], [3.0, 0.15, 0.0])
                .expect("ledge");
            c.move_and_slide([0.0, 0.0], FIXED_DT);
            c
        };
        let mut off = mkledge();
        for _ in 0..40 {
            off.move_and_slide_with([0.15, 0.0], FIXED_DT, &MoveShape::default());
        }
        assert!(
            off.position()[0] < 2.0,
            "autostep off blocks, {:?}",
            off.position()
        );
        let mut on = mkledge();
        let mut stepped = false;
        for _ in 0..40 {
            stepped = on
                .move_and_slide_with([0.15, 0.0], FIXED_DT, &MoveShape::legacy())
                .stepped
                || stepped;
        }
        assert!(stepped, "legacy autostep climbs");
        let mut narrow = mkledge();
        let mut wide = MoveShape::legacy();
        wide.autostep = Some(Autostep {
            max_height: STEP_HEIGHT,
            min_width: 5.0,
        });
        for _ in 0..40 {
            narrow.move_and_slide_with([0.15, 0.0], FIXED_DT, &wide);
        }
        assert!(narrow.position()[0] < 2.0, "min_width blocks narrow tread");
        let mut c = Controller::spawn([0.5, 0.53 + HALF, 0.0], RADIUS, HEIGHT).expect("spawn");
        c.add_static_box([5.0, 0.5, 3.0], [0.0, -0.5, 0.0])
            .expect("floor");
        c.add_ramp(0.0, 1.0, 0.0, 1.06, 3.0, 0.0).expect("steep");
        let r = c.move_and_slide_with([0.0, 0.0], FIXED_DT, &MoveShape::legacy());
        assert!(r.slipped && r.sliding_down_slope, "{r:?}");
        assert!(
            SnapLength::Relative(0.2).resolve(1.4) > 0.27,
            "snap resolves"
        );
        assert!(
            SnapLength::Absolute(f32::NAN).resolve(1.4) == 0.0,
            "bad snap maps to zero"
        );
    }
}
