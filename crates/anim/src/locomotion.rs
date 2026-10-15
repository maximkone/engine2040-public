//! locomotion: baseline gait state machine.
//!
//! Consumer: arena locomotion consumes [`LocoOutput::root`] as the authoritative
//! per-tick root intent and [`LocoOutput::contacts`] as foot-contact intent;
//! render consumes [`LocoOutput::pose`] (presentation only) through
//! [`skinning_matrices`](crate::contract::skinning_matrices). Data owner:
//! `engine-anim`; this module never imports assets or arena. Units: meters,
//! seconds, radians; fixed 60 Hz ticks ([`TICK_DT`]).
//!
//! Authority split: [`RootDelta`] + [`ContactTable`] are authoritative and
//! must stay bit-identical for identical input scripts; [`LocalPose`] is
//! presentation and must never move a physics capsule. Gait phase in
//! `[0, 1)` drives contacts: left foot planted on `[0.0, 0.6)`, right foot
//! planted on `[0.5, 1.0)` plus `[0.0, 0.1)` (double support `[0.5, 0.6)` and
//! `[0.0, 0.1)`). Airborne ticks plant nothing; idle/land ticks plant both.
//!
//! Determinism: only `+ - * /` and [`f32::sqrt`] (unit-length
//! normalization); no transcendentals, no wall clock, stable joint order.
//! Facing is a unit `[x, z]` vector turned by small-angle rotation plus
//! renormalization; positive `dyaw` rotates `+Z` toward `+X` (right-handed
//! about `+Y`). Procedural limb swing uses triangle waves of phase, and
//! leans use small-angle quaternions, for the same reason. Same input script
//! twice gives identical outputs. Invalid external input returns
//! [`LocoError`], never panics, and leaves state untouched.

use super::clip::{TICK_DT, clamp01};
use super::contract::{Contact, ContactTable, Joint, LocalPose, RootDelta, Skeleton};
use super::pose::blend_poses;

/// Idle below this wish speed (m/s).
pub const IDLE_EPS: f32 = 0.05;
/// Steady walk above idle up to this speed (m/s).
pub const WALK_MAX: f32 = 2.0;
/// Run hysteresis: run falls back to walk below this speed (m/s).
pub const WALK_RESUME: f32 = 1.8;
/// Steady run above [`WALK_MAX`] up to this speed (m/s).
pub const RUN_MAX: f32 = 6.0;
/// Strafe locomotion caps at this speed (m/s).
pub const STRAFE_MAX: f32 = 2.0;
/// Wishes above this speed (m/s) are rejected, never clamped silently.
pub const MAX_WISH_SPEED: f32 = 12.0;
/// Lateral wish (`|cross|` above this, roughly past 40 deg) holds facing.
/// Below the opposing threshold it strafes instead of turning.
pub const STRAFE_CROSS: f32 = 0.65;
/// Wish opposing facing (`dot` below this) while moving starts a pivot.
pub const PIVOT_DOT: f32 = -0.4;
/// Wish speed (m/s) below which no pivot starts.
pub const PIVOT_MIN_SPEED: f32 = 0.5;
/// Wish speed (m/s) above which lateral input counts as strafe.
pub const STRAFE_MIN_SPEED: f32 = 0.3;
/// Fixed ticks for start/stop/pivot/land transients.
pub const START_TICKS: u32 = 10;
/// Fixed ticks for start/stop/pivot/land transients.
pub const STOP_TICKS: u32 = 12;
/// Fixed ticks for start/stop/pivot/land transients.
pub const PIVOT_TICKS: u32 = 10;
/// Fixed ticks for start/stop/pivot/land transients.
pub const LAND_TICKS: u32 = 10;
/// Gait cycle durations (s) per moving state.
pub const WALK_CYCLE: f32 = 0.6;
/// Gait cycle durations (s) per moving state.
pub const RUN_CYCLE: f32 = 0.38;
/// Gait cycle durations (s) per moving state.
pub const STRAFE_CYCLE: f32 = 0.55;
/// Gait cycle durations (s) per moving state.
pub const PIVOT_CYCLE: f32 = 0.5;
/// Facing turn rates (rad per tick) per state.
pub const TURN_START: f32 = 0.08;
/// Facing turn rates (rad per tick) per state.
pub const TURN_WALK: f32 = 0.06;
/// Facing turn rates (rad per tick) per state.
pub const TURN_RUN: f32 = 0.045;
/// Facing turn rates (rad per tick) per state.
pub const TURN_STOP: f32 = 0.02;
/// Facing turn rates (rad per tick) per state.
pub const TURN_PIVOT: f32 = 0.20;
/// Facing turn rates (rad per tick) per state.
pub const TURN_AIR: f32 = 0.03;
/// Planar wish actually applied while airborne (momentum-lite steering).
pub const AIR_CONTROL: f32 = 0.35;
/// Combat-interrupt blend-in/out and total fixed ticks.
pub const HIT_IN_TICKS: u32 = 4;
/// Combat-interrupt blend-in/out and total fixed ticks.
pub const HIT_OUT_TICKS: u32 = 6;
/// Combat-interrupt blend-in/out and total fixed ticks.
pub const HIT_LIGHT_TICKS: u32 = 14;
/// Combat-interrupt blend-in/out and total fixed ticks.
pub const HIT_HEAVY_TICKS: u32 = 24;
/// Joints of [`loco_skeleton`]: root, spine, head, leg_l, leg_r, arm_l, arm_r.
pub const LOCO_JOINT_COUNT: usize = 7;
/// Leg joints of [`loco_skeleton`] (`leg_l`, `leg_r`): the stride-warp targets.
pub const LOCO_LEG_JOINTS: [usize; 2] = [3, 4];

/// Grounded/transient/airborne locomotion states.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LocoState {
    /// Standing, no root motion, both feet planted.
    Idle,
    /// Steady forward walk.
    Walk,
    /// Steady forward run.
    Run,
    /// Lateral/diagonal motion with facing held.
    Strafe,
    /// Idle-to-move acceleration ramp.
    Start,
    /// Move-to-idle deceleration ramp.
    Stop,
    /// Reversal turn with fast facing update.
    Pivot,
    /// Airborne after [`Locomotion::set_airborne`] with `true`.
    Air,
    /// Grounded again; brief planted recovery before the wish resolves.
    Land,
}

/// Combat interrupt kinds: hit reactions that freeze the gait, zero root
/// motion (hit-stop) and blend presentation to a hit pose over fixed ticks.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CombatInterrupt {
    /// Short reaction ([`HIT_LIGHT_TICKS`] ticks).
    Light,
    /// Long reaction ([`HIT_HEAVY_TICKS`] ticks).
    Heavy,
}

impl CombatInterrupt {
    /// Total fixed ticks this interrupt holds the machine.
    pub fn duration_ticks(self) -> u32 {
        match self {
            CombatInterrupt::Light => HIT_LIGHT_TICKS,
            CombatInterrupt::Heavy => HIT_HEAVY_TICKS,
        }
    }
}

/// Bounded locomotion input failures: typed, never panics.
#[derive(Clone, PartialEq, Debug)]
pub enum LocoError {
    /// Wish speed was negative; value carried.
    NegativeSpeed(f32),
    /// Wish speed was not finite.
    NonFiniteSpeed,
    /// Wish speed exceeded [`MAX_WISH_SPEED`]; value carried.
    SpeedTooHigh(f32),
    /// Wish direction had a non-finite lane.
    NonFiniteWishDir,
    /// Wish speed needed a direction but got a zero vector.
    ZeroWishDir,
}

/// Authoritative root intent plus presentation pose plus contact intent.
/// Root and contacts drive simulation; pose is render-only.
#[derive(Clone, PartialEq, Debug)]
pub struct LocoOutput {
    /// One tick of planar plus yaw intent for this tick stamp.
    pub root: RootDelta,
    /// Presentation pose on [`loco_skeleton`]; never authoritative.
    pub pose: LocalPose,
    /// Foot plant/lift samples for this tick stamp.
    pub contacts: ContactTable,
}

/// Baseline locomotion state machine. Construct with [`Locomotion::new`],
/// drive one fixed tick per [`Locomotion::update`], toggle flight with
/// [`Locomotion::set_airborne`]. `Clone` snapshots full authoritative state
/// (state, phase, facing, timers) for rollback or repeat checks.
#[derive(Clone, PartialEq, Debug)]
pub struct Locomotion {
    /// Current locomotion state (frozen underneath an active interrupt).
    pub state: LocoState,
    phase: f32,
    facing: [f32; 2],
    /// Planar speed (m/s) applied on the previous tick; transients capture
    /// it as their entry speed so Stop/Pivot decay from real momentum.
    speed: f32,
    trans_left: u32,
    trans_dur: u32,
    entry_speed: f32,
    airborne: bool,
    interrupt_left: u32,
    interrupt_dur: u32,
}

/// Fixed presentation skeleton: root, spine, head, leg_l, leg_r, arm_l,
/// arm_r in parent-first order.
pub fn loco_skeleton() -> Skeleton {
    Skeleton {
        joints: vec![
            Joint {
                name: "root".into(),
                parent: None,
            },
            Joint {
                name: "spine".into(),
                parent: Some(0),
            },
            Joint {
                name: "head".into(),
                parent: Some(1),
            },
            Joint {
                name: "leg_l".into(),
                parent: Some(0),
            },
            Joint {
                name: "leg_r".into(),
                parent: Some(0),
            },
            Joint {
                name: "arm_l".into(),
                parent: Some(1),
            },
            Joint {
                name: "arm_r".into(),
                parent: Some(1),
            },
        ],
    }
}

/// Pure contact intent for one gait phase at one tick stamp: left planted on
/// `[0.0, 0.6)`, right planted on `[0.5, 1.0)` or `[0.0, 0.1)`.
/// Non-finite phase plants both feet (safe bind).
pub fn contacts_for_phase(phase: f32, tick: u32) -> ContactTable {
    if !phase.is_finite() {
        return ContactTable {
            contacts: vec![
                Contact {
                    tick,
                    foot: 0,
                    down: true,
                },
                Contact {
                    tick,
                    foot: 1,
                    down: true,
                },
            ],
        };
    }
    let left = phase < 0.6;
    // Right stance is the complement window [0.5, 1.0) plus [0.0, 0.1);
    // inputs here are finite (guarded above), so this is exactly the
    // outside of [0.1, 0.5).
    let right = !(0.1..0.5).contains(&phase);
    ContactTable {
        contacts: vec![
            Contact {
                tick,
                foot: 0,
                down: left,
            },
            Contact {
                tick,
                foot: 1,
                down: right,
            },
        ],
    }
}

/// Pure planted contact intent (idle/land): both feet down.
pub fn contacts_planted(tick: u32) -> ContactTable {
    ContactTable {
        contacts: vec![
            Contact {
                tick,
                foot: 0,
                down: true,
            },
            Contact {
                tick,
                foot: 1,
                down: true,
            },
        ],
    }
}

/// Pure airborne contact intent: both feet up.
pub fn contacts_airborne(tick: u32) -> ContactTable {
    ContactTable {
        contacts: vec![
            Contact {
                tick,
                foot: 0,
                down: false,
            },
            Contact {
                tick,
                foot: 1,
                down: false,
            },
        ],
    }
}

/// Smoothstep on `[0, 1]` with `+ - *` only: `x*x*(3-2x)`.
fn smooth01(x: f32) -> f32 {
    let c = clamp01(x);
    c * c * (3.0 - 2.0 * c)
}

/// Triangle wave of phase in `[0, 1)`: `-1` at `0`, `+1` at `0.5`.
fn tri01(p: f32) -> f32 {
    if p < 0.5 {
        p * 4.0 - 1.0
    } else {
        3.0 - p * 4.0
    }
}

/// Unit `[x, z]` vector or `[0, 1]` on degenerate input; shared with ground feel.
pub(crate) fn normalize2(v: [f32; 2]) -> [f32; 2] {
    let len2 = v[0] * v[0] + v[1] * v[1];
    if !len2.is_finite() || len2 <= 0.0 {
        return [0.0, 1.0];
    }
    let inv = 1.0 / len2.sqrt();
    [v[0] * inv, v[1] * inv]
}

/// Small-angle pitch quaternion about local X for a lean in radians:
/// `[a/2, 0, 0, 1]` normalized. Accurate for presentation leans
/// (well under 0.3 rad) without transcendentals.
fn lean_quat(lean: f32) -> [f32; 4] {
    let q = [lean * 0.5, 0.0, 0.0, 1.0];
    let n2 = q[0] * q[0] + q[3] * q[3];
    let inv = 1.0 / n2.sqrt();
    [q[0] * inv, 0.0, 0.0, q[3] * inv]
}

/// Small-angle roll quaternion about local Z.
fn roll_quat(roll: f32) -> [f32; 4] {
    let q = [0.0, 0.0, roll * 0.5, 1.0];
    let n2 = q[2] * q[2] + q[3] * q[3];
    let inv = 1.0 / n2.sqrt();
    [0.0, 0.0, q[2] * inv, q[3] * inv]
}

const IDENTITY_Q: [f32; 4] = [0.0, 0.0, 0.0, 1.0];

/// Which emitter owns one tick's presentation. The authority core
/// ([`Locomotion::advance`]) returns this alongside the root delta so the
/// presentation half ([`Locomotion::present`]) rebuilds exactly the old
/// per-branch pose and contacts; the interrupt/land arms carry the
/// pre-decrement values the old emitters read before counting down.
#[derive(Clone, Copy, PartialEq, Debug)]
enum TickEmit {
    /// Combat interrupt: pre-decrement envelope position.
    Interrupt { elapsed: u32, dur: u32 },
    /// Airborne tuck with airborne contacts.
    Air,
    /// Landing crouch: pre-decrement root dip in meters.
    Land { dip: f32 },
    /// Grounded gait pose with phase contacts.
    Ground,
}

impl Locomotion {
    /// New machine: idle, phase `0`, facing `+Z`, grounded, no interrupt.
    pub fn new() -> Self {
        Self {
            state: LocoState::Idle,
            phase: 0.0,
            facing: [0.0, 1.0],
            speed: 0.0,
            trans_left: 0,
            trans_dur: 0,
            entry_speed: 0.0,
            airborne: false,
            interrupt_left: 0,
            interrupt_dur: 0,
        }
    }

    /// Current gait phase in `[0, 1)`.
    pub fn phase(&self) -> f32 {
        self.phase
    }

    /// Current facing as a unit `[x, z]` vector.
    pub fn facing(&self) -> [f32; 2] {
        self.facing
    }

    /// Planar speed (m/s) applied on the previous tick; presentation input
    /// for learned nets, never authoritative.
    pub fn speed(&self) -> f32 {
        self.speed
    }

    /// True after [`Locomotion::set_airborne`] with `true` until cleared.
    pub fn is_airborne(&self) -> bool {
        self.airborne
    }

    /// True while a combat interrupt holds the machine (hit-stop).
    pub fn interrupt_active(&self) -> bool {
        self.interrupt_left > 0
    }

    /// Current interrupt blend weight in `[0, 1]`: `0` at rest, rising
    /// through blend-in, `1` on hold, falling through blend-out.
    pub fn interrupt_weight(&self) -> f32 {
        if self.interrupt_left == 0 || self.interrupt_dur == 0 {
            return 0.0;
        }
        let elapsed = self.interrupt_dur - self.interrupt_left;
        envelope_weight(elapsed, self.interrupt_dur)
    }

    /// Mark the body airborne (`true`) or grounded (`false`). Leaving the
    /// ground enters [`LocoState::Air`]; touching down enters
    /// [`LocoState::Land`] for [`LAND_TICKS`] ticks, then the wish resolves.
    pub fn set_airborne(&mut self, airborne: bool) {
        if airborne == self.airborne {
            return;
        }
        self.airborne = airborne;
        if airborne {
            self.state = LocoState::Air;
            self.trans_left = 0;
            self.trans_dur = 0;
        } else if self.state == LocoState::Air {
            self.state = LocoState::Land;
            self.trans_left = LAND_TICKS;
            self.trans_dur = LAND_TICKS;
        }
    }

    /// Advance one fixed tick. `wish_speed` is planar speed (m/s),
    /// `wish_dir` the world-space `[x, z]` wish direction (ignored near
    /// zero speed), `tick` stamps [`RootDelta::tick`], and `interrupt`
    /// starts (or restarts) a hit reaction. Rejects out-of-bounds input
    /// without mutating state.
    pub fn update(
        &mut self,
        wish_speed: f32,
        wish_dir: [f32; 2],
        tick: u32,
        interrupt: Option<CombatInterrupt>,
    ) -> Result<LocoOutput, LocoError> {
        let wish = check_wish(wish_speed, wish_dir)?;
        let (root, emit) = self.advance(wish, tick, interrupt);
        let (pose, contacts) = self.present(emit, &wish, tick);
        Ok(LocoOutput {
            root,
            pose,
            contacts,
        })
    }

    /// Advance one fixed tick returning only the authoritative root intent,
    /// with zero heap allocation. The state machine (transitions, facing,
    /// phase, interrupt countdown) runs through the same [`Locomotion::advance`]
    /// core as [`Locomotion::update`], so the returned delta and the machine
    /// state are bit-identical to the full path; only the presentation pose
    /// and contact table are skipped.
    ///
    /// Consumer: the arena authority tick (`record=false`) and forks, which
    /// fold root intent into the checkpoint hash and drop pose/contacts.
    /// Presentation runs (`record=true`) keep [`Locomotion::update`].
    /// Budget: 0 bytes of heap per tick on every branch (idle, transients,
    /// air, land, interrupts); pinned by the counting-allocator test below.
    /// Determinism and errors match [`Locomotion::update`]: only `+ - * /`
    /// and `sqrt`, and rejected input leaves state untouched.
    pub fn update_root(
        &mut self,
        wish_speed: f32,
        wish_dir: [f32; 2],
        tick: u32,
        interrupt: Option<CombatInterrupt>,
    ) -> Result<RootDelta, LocoError> {
        let wish = check_wish(wish_speed, wish_dir)?;
        let (root, _) = self.advance(wish, tick, interrupt);
        Ok(root)
    }

    /// Drive a presentation [`Readable`](super::readable::Readable) overlay
    /// from this machine. `mx`/`mz` carry planar velocity (m/s) in world
    /// `[x, z]`; facing comes from the machine, so the arrow tracks motion
    /// while the sim stays untouched. Consumer: HUD and capture read the
    /// returned frame beside [`LocoOutput::pose`]; the authority path
    /// ([`Locomotion::update_root`]) never calls this, so the tick hash
    /// cannot move. Errors match [`ReadableError`](super::readable::ReadableError);
    /// rejected input leaves the overlay untouched.
    /// Allocation-free: only `Copy` scalars move here.
    pub fn drive_readable(
        &self,
        readable: &mut super::readable::Readable,
        mx: f32,
        mz: f32,
    ) -> Result<super::readable::ReadableFrame, super::readable::ReadableError> {
        readable.update(mx, mz, self.facing)
    }

    /// One authority tick shared by [`Locomotion::update`] and
    /// [`Locomotion::update_root`]: validates nothing (the caller checked
    /// the wish), runs every mutation in the old `update` order, and returns
    /// the root delta plus which emitter owns this tick's presentation.
    /// Allocation-free: only `Copy` scalars move here; pose and contact
    /// vectors are built later by [`Locomotion::present`].
    fn advance(
        &mut self,
        wish: Wish,
        tick: u32,
        interrupt: Option<CombatInterrupt>,
    ) -> (RootDelta, TickEmit) {
        if let Some(kind) = interrupt {
            self.interrupt_left = kind.duration_ticks();
            self.interrupt_dur = kind.duration_ticks();
        }
        if self.interrupt_left > 0 {
            let emit = TickEmit::Interrupt {
                elapsed: self.interrupt_dur - self.interrupt_left,
                dur: self.interrupt_dur,
            };
            let root = RootDelta {
                tick,
                dp: [0.0, 0.0],
                dyaw: 0.0,
            };
            self.interrupt_left -= 1;
            return (root, emit);
        }
        if self.airborne {
            self.state = LocoState::Air;
            let root = self.air_root(tick, &wish);
            return (root, TickEmit::Air);
        }
        if self.state == LocoState::Air {
            // Grounded without the flag ever set (fresh machine or restored
            // snapshot): recover through Land like a real touchdown.
            self.state = LocoState::Land;
            self.trans_left = LAND_TICKS;
            self.trans_dur = LAND_TICKS;
        }
        if self.state == LocoState::Land {
            let emit = TickEmit::Land {
                dip: 0.10 * (self.trans_left as f32) / (self.trans_dur.max(1) as f32),
            };
            let root = RootDelta {
                tick,
                dp: [0.0, 0.0],
                dyaw: 0.0,
            };
            self.speed = 0.0;
            self.trans_left -= 1;
            if self.trans_left == 0 {
                self.resolve_ground(&wish);
            }
            return (root, emit);
        }
        self.step_ground(&wish);
        let root = self.ground_root(tick, &wish);
        (root, TickEmit::Ground)
    }

    /// Presentation half of [`Locomotion::update`]: pose plus contacts for a
    /// tick the authority core already advanced. Pure over post-tick state
    /// (`state`, `phase`, `speed`, `facing`) plus the captured `emit`
    /// pre-decrement values, so outputs match the old per-branch emitters
    /// exactly. Allocates 3 vectors (positions, rotations, contacts).
    fn present(&self, emit: TickEmit, wish: &Wish, tick: u32) -> (LocalPose, ContactTable) {
        match emit {
            TickEmit::Interrupt { elapsed, dur } => {
                let w = envelope_weight(elapsed, dur);
                let base = self.base_pose(
                    &Wish {
                        speed: 0.0,
                        dir: self.facing,
                    },
                    0.0,
                );
                let pose = blend_poses(&base, &hit_pose(), w);
                (pose, contacts_for_phase(self.phase, tick))
            }
            TickEmit::Air => {
                let mut pose = self.base_pose(wish, self.speed);
                // Tuck: legs trail back, arms out. Pure offsets, no trig.
                pose.positions[3] = [0.11, -0.88, -0.12];
                pose.positions[4] = [-0.11, -0.88, -0.12];
                pose.positions[5] = [0.42, 0.12, 0.0];
                pose.positions[6] = [-0.42, 0.12, 0.0];
                (pose, contacts_airborne(tick))
            }
            TickEmit::Land { dip } => {
                let mut pose = self.base_pose(
                    &Wish {
                        speed: 0.0,
                        dir: self.facing,
                    },
                    0.0,
                );
                pose.positions[0][1] -= dip;
                (pose, contacts_planted(tick))
            }
            TickEmit::Ground => (
                self.base_pose(wish, self.speed),
                contacts_for_phase(self.phase, tick),
            ),
        }
    }

    /// Steady desire from a validated wish, relative to current facing.
    fn desire(&self, wish: &Wish) -> LocoState {
        if wish.speed < IDLE_EPS {
            return LocoState::Idle;
        }
        let dot = self.facing[0] * wish.dir[0] + self.facing[1] * wish.dir[1];
        let cross = self.facing[0] * wish.dir[1] - self.facing[1] * wish.dir[0];
        if wish.speed > STRAFE_MIN_SPEED && dot > PIVOT_DOT && cross.abs() > STRAFE_CROSS {
            return LocoState::Strafe;
        }
        if wish.speed <= WALK_MAX {
            return LocoState::Walk;
        }
        if self.state == LocoState::Run && wish.speed >= WALK_RESUME {
            return LocoState::Run;
        }
        if self.state == LocoState::Walk && wish.speed <= WALK_MAX {
            return LocoState::Walk;
        }
        LocoState::Run
    }

    /// Capped steady speed for a validated wish in the given state.
    fn steady_speed(state: LocoState, speed: f32) -> f32 {
        match state {
            LocoState::Walk | LocoState::Start => speed.min(WALK_MAX),
            LocoState::Run => speed.min(RUN_MAX),
            LocoState::Strafe => speed.min(STRAFE_MAX),
            _ => 0.0,
        }
    }

    fn begin_trans(&mut self, state: LocoState, dur: u32, entry: f32) {
        self.state = state;
        self.trans_left = dur;
        self.trans_dur = dur;
        self.entry_speed = entry;
    }

    /// One grounded tick: transitions first, then facing, then phase.
    fn step_ground(&mut self, wish: &Wish) {
        let dot = self.facing[0] * wish.dir[0] + self.facing[1] * wish.dir[1];
        match self.state {
            LocoState::Idle => {
                if self.desire(wish) != LocoState::Idle {
                    let target = Self::steady_speed(self.desire(wish), wish.speed);
                    self.begin_trans(LocoState::Start, START_TICKS, target);
                }
            }
            LocoState::Walk | LocoState::Run | LocoState::Strafe => {
                if wish.speed > PIVOT_MIN_SPEED && dot < PIVOT_DOT {
                    self.begin_trans(LocoState::Pivot, PIVOT_TICKS, self.speed);
                } else {
                    let want = self.desire(wish);
                    if want == LocoState::Idle {
                        self.begin_trans(LocoState::Stop, STOP_TICKS, self.speed);
                    } else if want != self.state
                        && matches!(want, LocoState::Walk | LocoState::Run | LocoState::Strafe)
                    {
                        self.state = want;
                    }
                }
            }
            LocoState::Start => {
                if self.desire(wish) == LocoState::Idle {
                    self.begin_trans(LocoState::Stop, STOP_TICKS, self.speed);
                } else if self.trans_left > 0 {
                    self.trans_left -= 1;
                    if self.trans_left == 0 {
                        self.state = self.desire(wish);
                    }
                } else {
                    self.state = self.desire(wish);
                }
            }
            LocoState::Stop => {
                if self.desire(wish) != LocoState::Idle {
                    let target = Self::steady_speed(self.desire(wish), wish.speed);
                    self.begin_trans(LocoState::Start, START_TICKS, target);
                } else if self.trans_left > 0 {
                    self.trans_left -= 1;
                    if self.trans_left == 0 {
                        self.state = LocoState::Idle;
                    }
                } else {
                    self.state = LocoState::Idle;
                }
            }
            LocoState::Pivot => {
                if self.desire(wish) == LocoState::Idle {
                    self.begin_trans(LocoState::Stop, STOP_TICKS, self.speed);
                } else if self.trans_left > 0 {
                    self.trans_left -= 1;
                    if self.trans_left == 0 {
                        self.state = self.desire(wish);
                    }
                } else {
                    self.state = self.desire(wish);
                }
            }
            LocoState::Air | LocoState::Land => {}
        }
        self.turn_facing(wish);
        self.advance_phase();
    }

    /// Resolve the post-Land state from the current wish.
    fn resolve_ground(&mut self, wish: &Wish) {
        let want = self.desire(wish);
        if want == LocoState::Idle {
            self.state = LocoState::Idle;
        } else {
            let target = Self::steady_speed(want, wish.speed);
            self.begin_trans(LocoState::Start, START_TICKS, target);
        }
    }

    /// Planar speed actually applied this tick in a grounded state.
    /// Start ramps `1/dur .. 1` over its ticks; Stop/Pivot decay from the
    /// entry speed so the entry tick already moves less than full speed.
    fn ground_speed(&self, wish: &Wish) -> f32 {
        match self.state {
            LocoState::Idle | LocoState::Land => 0.0,
            LocoState::Start => {
                let target = Self::steady_speed(self.desire(wish), wish.speed);
                let done = self.trans_dur - self.trans_left + 1;
                target * (done as f32) / (self.trans_dur.max(1) as f32)
            }
            LocoState::Stop | LocoState::Pivot => {
                let left = self.trans_left.saturating_sub(1);
                self.entry_speed * (left as f32) / (self.trans_dur.max(1) as f32)
            }
            LocoState::Walk | LocoState::Run | LocoState::Strafe => {
                Self::steady_speed(self.state, wish.speed)
            }
            LocoState::Air => 0.0,
        }
    }

    fn turn_rate(&self) -> f32 {
        match self.state {
            LocoState::Start => TURN_START,
            LocoState::Walk => TURN_WALK,
            LocoState::Run => TURN_RUN,
            LocoState::Strafe => 0.0,
            LocoState::Stop => TURN_STOP,
            LocoState::Pivot => TURN_PIVOT,
            LocoState::Air => TURN_AIR,
            LocoState::Idle | LocoState::Land => 0.0,
        }
    }

    /// Turn facing toward the wish. Proportional on `-cross` (which is
    /// `sin` of the signed angle to the wish under the `+Z`-to-`+X`
    /// convention), limited by the state rate; pivots snap at full rate.
    /// Rotation uses the small-angle `(sin, cos) = (d, 1-d*d/2)` pair plus
    /// renormalization: `+ - * /` and `sqrt` only.
    fn turn_facing(&mut self, wish: &Wish) {
        if wish.speed < IDLE_EPS {
            return;
        }
        let rate = self.turn_rate();
        if rate <= 0.0 {
            return;
        }
        let cross = self.facing[0] * wish.dir[1] - self.facing[1] * wish.dir[0];
        let dyaw = if self.state == LocoState::Pivot {
            if cross < 0.0 {
                -rate
            } else {
                rate
            }
        } else {
            (-cross).clamp(-rate, rate)
        };
        let sin = dyaw;
        let cos = 1.0 - dyaw * dyaw * 0.5;
        let f = self.facing;
        self.facing = normalize2([f[0] * cos + f[1] * sin, -f[0] * sin + f[1] * cos]);
    }

    /// Gait cycle duration (s) for the current state; `0` freezes phase.
    fn cycle(&self) -> f32 {
        match self.state {
            LocoState::Walk | LocoState::Start | LocoState::Stop => WALK_CYCLE,
            LocoState::Run => RUN_CYCLE,
            LocoState::Strafe => STRAFE_CYCLE,
            LocoState::Pivot => PIVOT_CYCLE,
            LocoState::Idle | LocoState::Air | LocoState::Land => 0.0,
        }
    }

    /// Advance gait phase by one tick of the state cycle, wrapping into
    /// `[0, 1)` by subtraction so loop-boundary ticks stay continuous.
    fn advance_phase(&mut self) {
        let cycle = self.cycle();
        if cycle <= 0.0 {
            return;
        }
        self.phase += TICK_DT / cycle;
        if self.phase >= 1.0 {
            self.phase -= 1.0;
            if self.phase >= 1.0 {
                // Long stalls cannot accumulate here (one tick per call),
                // but stay bounded under any future larger step.
                self.phase %= 1.0;
            }
        }
    }

    /// Authoritative planar intent for a grounded tick: records the applied
    /// speed, then the wish-scaled planar step plus the yaw the machine
    /// turned. Allocation-free half of the old grounded emitter; the pose
    /// half lives in [`Locomotion::present`].
    fn ground_root(&mut self, tick: u32, wish: &Wish) -> RootDelta {
        let speed = self.ground_speed(wish);
        self.speed = speed;
        RootDelta {
            tick,
            dp: [wish.dir[0] * speed * TICK_DT, wish.dir[1] * speed * TICK_DT],
            dyaw: self.steer_dyaw(wish),
        }
    }

    /// Signed yaw applied on the current tick for the current state and
    /// wish: the proportional `-cross` law limited by the state rate, full
    /// rate on pivots (exact reversal picks `+rate`). Recomputed from the
    /// post-turn facing; post-turn cross differs from pre-turn by
    /// `O(rate^2)`, so the reported value matches the applied step up to
    /// the rate clamp, which tests pin down exactly.
    fn steer_dyaw(&self, wish: &Wish) -> f32 {
        if wish.speed < IDLE_EPS {
            return 0.0;
        }
        let rate = self.turn_rate();
        if rate <= 0.0 {
            return 0.0;
        }
        // Post-turn cross differs from pre-turn by `O(rate^2)`; clamping it
        // reproduces the applied step exactly whenever the turn saturates
        // (the strafe/pivot/walk cases the course pins down) and follows the
        // same proportional law otherwise.
        let cross = self.facing[0] * wish.dir[1] - self.facing[1] * wish.dir[0];
        if self.state == LocoState::Pivot {
            if cross < 0.0 {
                -rate
            } else {
                rate
            }
        } else {
            (-cross).clamp(-rate, rate)
        }
    }

    /// Authoritative planar intent for an airborne tick (momentum-lite
    /// steering): records the applied speed, then the scaled planar step
    /// plus yaw. Allocation-free half of the old air emitter.
    fn air_root(&mut self, tick: u32, wish: &Wish) -> RootDelta {
        let speed = wish.speed.min(RUN_MAX) * AIR_CONTROL;
        self.speed = speed;
        RootDelta {
            tick,
            dp: [wish.dir[0] * speed * TICK_DT, wish.dir[1] * speed * TICK_DT],
            dyaw: self.steer_dyaw(wish),
        }
    }

    /// Presentation pose for the current state/phase/speed on
    /// [`loco_skeleton`]. Triangle-wave limb swing, speed-scaled bob and
    /// lean, strafe roll, land/pivot crouch. Positions carry the motion;
    /// rotations stay near identity (small-angle lean/roll, normalized).
    fn base_pose(&self, wish: &Wish, speed: f32) -> LocalPose {
        let amp = (speed / RUN_MAX).clamp(0.0, 1.0);
        let swing = 0.26 * amp;
        let bob = match self.state {
            LocoState::Run => 0.055 * amp,
            LocoState::Air => 0.0,
            _ => 0.035 * amp,
        };
        let s_l = tri01(self.phase) * swing;
        let mut p = self.phase + 0.5;
        if p >= 1.0 {
            p -= 1.0;
        }
        let s_r = tri01(p) * swing;
        // Double-frequency lift from the phase triangle: dip at mid-stance.
        let mut q = self.phase * 2.0;
        if q >= 1.0 {
            q -= 1.0;
        }
        if q >= 1.0 {
            q -= 1.0;
        }
        let lift = tri01(q).abs() * bob;
        let crouch = match self.state {
            LocoState::Idle => 0.02,
            LocoState::Pivot => 0.06,
            _ => 0.0,
        };
        let lean = (speed * 0.022).clamp(0.0, 0.15);
        let cross = self.facing[0] * wish.dir[1] - self.facing[1] * wish.dir[0];
        let roll = if self.state == LocoState::Strafe {
            (-cross * 0.12).clamp(-0.12, 0.12)
        } else {
            0.0
        };
        let spine_q = if roll.abs() > 0.0 {
            roll_quat(roll)
        } else {
            lean_quat(lean)
        };
        LocalPose {
            positions: vec![
                [0.0, 0.95 - lift - crouch, 0.0],
                [0.0, 0.22, lean * 0.4],
                [0.0, 0.30, lean * 0.3],
                [0.11, -0.90 + lift * 0.5, s_l],
                [-0.11, -0.90 + lift * 0.5, s_r],
                [0.32, -0.05, -s_l * 0.6],
                [-0.32, -0.05, -s_r * 0.6],
            ],
            rotations: vec![
                IDENTITY_Q, spine_q, IDENTITY_Q, IDENTITY_Q, IDENTITY_Q, IDENTITY_Q, IDENTITY_Q,
            ],
        }
    }
}

impl Default for Locomotion {
    fn default() -> Self {
        Self::new()
    }
}

/// Validated wish: finite speed in `[0, MAX_WISH_SPEED]`, finite direction,
/// unit direction (zero vector only with near-zero speed).
#[derive(Clone, Copy, PartialEq, Debug)]
struct Wish {
    speed: f32,
    dir: [f32; 2],
}

fn check_wish(speed: f32, dir: [f32; 2]) -> Result<Wish, LocoError> {
    if !speed.is_finite() {
        return Err(LocoError::NonFiniteSpeed);
    }
    if speed < 0.0 {
        return Err(LocoError::NegativeSpeed(speed));
    }
    if speed > MAX_WISH_SPEED {
        return Err(LocoError::SpeedTooHigh(speed));
    }
    if dir.iter().any(|v| !v.is_finite()) {
        return Err(LocoError::NonFiniteWishDir);
    }
    if speed < IDLE_EPS {
        return Ok(Wish {
            speed,
            dir: [0.0, 1.0],
        });
    }
    let len2 = dir[0] * dir[0] + dir[1] * dir[1];
    if len2 <= 0.0 {
        return Err(LocoError::ZeroWishDir);
    }
    let inv = 1.0 / len2.sqrt();
    Ok(Wish {
        speed,
        dir: [dir[0] * inv, dir[1] * inv],
    })
}

/// Interrupt envelope over `elapsed` of `dur` ticks: smoothstep up over
/// [`HIT_IN_TICKS`], hold at `1`, smoothstep down over [`HIT_OUT_TICKS`].
fn envelope_weight(elapsed: u32, dur: u32) -> f32 {
    if dur == 0 {
        return 0.0;
    }
    if elapsed < HIT_IN_TICKS {
        return smooth01((elapsed + 1) as f32 / HIT_IN_TICKS as f32);
    }
    if elapsed + HIT_OUT_TICKS >= dur {
        let left = dur - elapsed;
        if left == 0 {
            return 0.0;
        }
        return smooth01((left - 1) as f32 / HIT_OUT_TICKS as f32);
    }
    1.0
}

/// Hit-react presentation pose on [`loco_skeleton`]: crouch, rear lean,
/// arms raised. Blended over the base pose by the interrupt envelope.
fn hit_pose() -> LocalPose {
    LocalPose {
        positions: vec![
            [0.0, 0.80, -0.06],
            [0.0, 0.20, -0.10],
            [0.0, 0.28, -0.06],
            [0.13, -0.78, 0.06],
            [-0.13, -0.78, -0.04],
            [0.36, 0.22, 0.22],
            [-0.36, 0.22, 0.22],
        ],
        rotations: vec![
            IDENTITY_Q,
            lean_quat(-0.18),
            IDENTITY_Q,
            IDENTITY_Q,
            IDENTITY_Q,
            IDENTITY_Q,
            IDENTITY_Q,
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::super::contract::{validate_contacts, validate_pose, validate_root, RootTable};
    use super::*;
    use engine_core::alloc_probe::measure;

    fn skeleton() -> Skeleton {
        loco_skeleton()
    }

    /// Drive a script of `(speed, dir)` wishes; ticks auto-increment.
    fn drive(machine: &mut Locomotion, script: &[(f32, [f32; 2])]) -> Vec<LocoOutput> {
        let mut outs = Vec::with_capacity(script.len());
        for (i, (speed, dir)) in script.iter().enumerate() {
            outs.push(machine.update(*speed, *dir, i as u32, None).unwrap());
        }
        outs
    }

    fn script_speeds(speeds: &[f32], dir: [f32; 2]) -> Vec<(f32, [f32; 2])> {
        speeds.iter().map(|s| (*s, dir)).collect()
    }

    #[test]
    fn drive_readable_wires_overlay_without_touching_authority() {
        use super::super::readable::Readable;
        let fwd = [0.0, 1.0];
        let mut m = Locomotion::new();
        let script = script_speeds(&[0.0, 1.2, 1.2, 4.0, 0.0], fwd);
        let mut overlay = Readable::new(fwd);
        let mut direct = Readable::new(fwd);
        for (i, (speed, dir)) in script.iter().enumerate() {
            let before = m.clone();
            let out = m.update(*speed, *dir, i as u32, None).unwrap();
            let mx = speed * dir[0];
            let mz = speed * dir[1];
            let via_machine = m.drive_readable(&mut overlay, mx, mz).unwrap();
            let expect = direct.update(mx, mz, m.facing()).unwrap();
            assert_eq!(via_machine, expect, "wiring drifts at tick {i}");
            let mut fork = before.clone();
            let root_only = fork.update_root(*speed, *dir, i as u32, None).unwrap();
            assert_eq!(root_only, out.root, "authority diverges at tick {i}");
        }
        let snap = overlay.clone();
        assert!(m.drive_readable(&mut overlay, f32::NAN, 0.0).is_err());
        assert_eq!(overlay, snap, "rejected input mutates overlay");
    }

    #[test]
    fn transition_map_idle_walk_run_stop_idle() {
        let fwd = [0.0, 1.0];
        let mut m = Locomotion::new();
        // Idle dwell.
        for t in 0..5 {
            let out = m.update(0.0, fwd, t, None).unwrap();
            assert_eq!(m.state, LocoState::Idle, "t={t}");
            assert_eq!(out.root.dp, [0.0, 0.0]);
            assert_eq!(out.contacts, contacts_planted(t));
        }
        // Walk wish: first tick enters Start, holds START_TICKS, then Walk.
        let out = m.update(1.2, fwd, 5, None).unwrap();
        assert_eq!(m.state, LocoState::Start);
        assert!(out.root.dp[1] > 0.0, "start ramps from rest: {out:?}");
        for t in 6..(5 + START_TICKS) {
            m.update(1.2, fwd, t, None).unwrap();
            assert_eq!(m.state, LocoState::Start, "t={t}");
        }
        let out = m.update(1.2, fwd, 5 + START_TICKS, None).unwrap();
        assert_eq!(m.state, LocoState::Walk);
        let expect_dp = 1.2 * TICK_DT;
        assert!((out.root.dp[1] - expect_dp).abs() < 1e-6, "{out:?}");
        assert_eq!(out.root.dp[0], 0.0);
        // Run wish: direct Walk -> Run.
        let out = m.update(4.0, fwd, 100, None).unwrap();
        assert_eq!(m.state, LocoState::Run);
        assert!((out.root.dp[1] - 4.0 * TICK_DT).abs() < 1e-6, "{out:?}");
        // Drop the wish: Run -> Stop ramp, then Idle.
        let out = m.update(0.0, fwd, 101, None).unwrap();
        assert_eq!(m.state, LocoState::Stop);
        assert!(
            out.root.dp[1] > 0.0 && out.root.dp[1] < 4.0 * TICK_DT,
            "{out:?}"
        );
        for t in 102..(101 + STOP_TICKS) {
            m.update(0.0, fwd, t, None).unwrap();
            assert_eq!(m.state, LocoState::Stop, "t={t}");
        }
        let out = m.update(0.0, fwd, 101 + STOP_TICKS, None).unwrap();
        assert_eq!(m.state, LocoState::Idle);
        assert_eq!(out.root.dp, [0.0, 0.0]);
        // Presentation stays valid at every checkpoint.
        validate_pose(&skeleton(), &out.pose).unwrap();
    }

    #[test]
    fn strafe_holds_facing_and_pivot_turns() {
        let mut m = Locomotion::new();
        let fwd = [0.0, 1.0];
        // Reach Walk first.
        for t in 0..(START_TICKS + 2) {
            m.update(1.5, fwd, t, None).unwrap();
        }
        assert_eq!(m.state, LocoState::Walk);
        let facing = m.facing();
        // Hard lateral wish -> Strafe, facing held.
        let out = m.update(1.5, [1.0, 0.0], 50, None).unwrap();
        assert_eq!(m.state, LocoState::Strafe, "{out:?}");
        assert!(out.root.dp[0] > 0.0, "strafe moves along wish: {out:?}");
        for t in 51..56 {
            m.update(1.5, [1.0, 0.0], t, None).unwrap();
            assert_eq!(m.facing(), facing, "strafe holds facing t={t}");
        }
        // Reversal -> Pivot with fast yaw and decaying root.
        let out = m.update(1.5, [-facing[0], -facing[1]], 60, None).unwrap();
        assert_eq!(m.state, LocoState::Pivot, "{out:?}");
        assert_eq!(out.root.dyaw.abs(), TURN_PIVOT, "{out:?}");
        validate_pose(&skeleton(), &out.pose).unwrap();
    }

    #[test]
    fn air_land_cycle_plants_nothing_then_both() {
        let mut m = Locomotion::new();
        let fwd = [0.0, 1.0];
        for t in 0..(START_TICKS + 2) {
            m.update(1.5, fwd, t, None).unwrap();
        }
        m.set_airborne(true);
        assert!(m.is_airborne());
        let out = m.update(1.5, fwd, 50, None).unwrap();
        assert_eq!(m.state, LocoState::Air);
        assert_eq!(out.contacts, contacts_airborne(50));
        validate_pose(&skeleton(), &out.pose).unwrap();
        m.set_airborne(false);
        assert_eq!(m.state, LocoState::Land);
        // Land holds planted for LAND_TICKS ticks; the resolving tick still
        // emits a land output, then the state reflects the wish (idle here).
        for t in 51..(51 + LAND_TICKS - 1) {
            let out = m.update(0.0, fwd, t, None).unwrap();
            assert_eq!(m.state, LocoState::Land, "t={t}");
            assert_eq!(out.root.dp, [0.0, 0.0]);
            assert_eq!(out.contacts, contacts_planted(t));
        }
        let out = m.update(0.0, fwd, 51 + LAND_TICKS - 1, None).unwrap();
        assert_eq!(out.root.dp, [0.0, 0.0]);
        assert_eq!(out.contacts, contacts_planted(51 + LAND_TICKS - 1));
        assert_eq!(m.state, LocoState::Idle);
        // Wish resolves after Land: moving wish re-enters Start.
        m.update(1.5, fwd, 51 + LAND_TICKS, None).unwrap();
        assert_eq!(m.state, LocoState::Start);
    }

    #[test]
    fn phase_wraps_across_loop_boundary() {
        let mut m = Locomotion::new();
        let fwd = [0.0, 1.0];
        // Long walk: every output validates, and the gait shows both
        // single-support (one foot) and double-support (two feet) samples,
        // proving phase actually cycles rather than sticking.
        let script = script_speeds(&vec![1.5; 400], fwd);
        let outs = drive(&mut m, &script);
        let mut singles = 0u32;
        for (t, out) in outs.iter().enumerate() {
            validate_pose(&skeleton(), &out.pose).unwrap();
            validate_contacts(&out.contacts).unwrap();
            let downs = out.contacts.contacts.iter().filter(|c| c.down).count();
            assert!(downs == 1 || downs == 2, "t={t}: {out:?}");
            if downs == 1 {
                singles += 1;
            }
        }
        assert!(singles > 100, "gait cycles show single support: {singles}");
        // Phase stays in [0, 1) and wraps several times over 400 ticks.
        let mut m2 = Locomotion::new();
        let mut wrapped = 0u32;
        let mut prev = 0.0f32;
        for t in 0..400 {
            m2.update(1.5, fwd, t, None).unwrap();
            assert!(
                (0.0..1.0).contains(&m2.phase()),
                "phase bounded t={t}: {}",
                m2.phase()
            );
            if m2.phase() < prev {
                wrapped += 1;
            }
            prev = m2.phase();
        }
        assert!(wrapped >= 3, "walk loops several gait cycles: {wrapped}");
        // Phase advances monotonically inside a cycle (no jitter).
        let mut m3 = Locomotion::new();
        m3.update(1.5, fwd, 0, None).unwrap();
        let mut last = m3.phase();
        for t in 1..30 {
            m3.update(1.5, fwd, t, None).unwrap();
            let p = m3.phase();
            assert!(
                p >= last || p < 0.05,
                "monotonic modulo wrap: {last} -> {p}"
            );
            last = p;
        }
    }

    #[test]
    fn contact_table_matches_gait_phase() {
        let mut m = Locomotion::new();
        let fwd = [0.0, 1.0];
        for t in 0..300 {
            let out = m.update(1.5, fwd, t, None).unwrap();
            let expect = contacts_for_phase(m.phase(), t);
            assert_eq!(out.contacts, expect, "t={t} phase={}", m.phase());
            validate_contacts(&out.contacts).unwrap();
        }
        // Window edges: left plants [0, 0.6), right plants [0.5, 1) + [0, 0.1).
        let edge = |phase: f32, left: bool, right: bool| {
            let table = contacts_for_phase(phase, 7);
            assert_eq!(table.contacts[0].down, left, "phase={phase}");
            assert_eq!(table.contacts[1].down, right, "phase={phase}");
        };
        edge(0.0, true, true);
        edge(0.05, true, true);
        edge(0.2, true, false);
        edge(0.55, true, true);
        edge(0.7, false, true);
        edge(0.99, false, true);
    }

    #[test]
    fn combat_interrupt_blends_and_returns() {
        let mut m = Locomotion::new();
        let fwd = [0.0, 1.0];
        for t in 0..(START_TICKS + 5) {
            m.update(1.5, fwd, t, None).unwrap();
        }
        assert_eq!(m.state, LocoState::Walk);
        assert_eq!(m.interrupt_weight(), 0.0);
        // Start a light interrupt: hit-stop zeroes root, pose blends.
        let out = m
            .update(1.5, fwd, 50, Some(CombatInterrupt::Light))
            .unwrap();
        assert!(m.interrupt_active());
        assert!(m.interrupt_weight() > 0.0);
        assert_eq!(out.root.dp, [0.0, 0.0]);
        assert_eq!(out.root.dyaw, 0.0);
        validate_pose(&skeleton(), &out.pose).unwrap();
        let mid = m.update(1.5, fwd, 51, None).unwrap();
        assert!(m.interrupt_active());
        assert_ne!(mid.pose, out.pose, "envelope advances across ticks");
        // Hold reaches full blend; capture it.
        let mut held_pose: Option<LocalPose> = None;
        for t in 52..(50 + HIT_LIGHT_TICKS) {
            let o = m.update(1.5, fwd, t, None).unwrap();
            validate_pose(&skeleton(), &o.pose).unwrap();
            assert_eq!(o.root.dp, [0.0, 0.0], "hit-stop holds t={t}");
            if m.interrupt_weight() == 1.0 && held_pose.is_none() {
                held_pose = Some(o.pose.clone());
            }
        }
        let held_pose = held_pose.expect("envelope reaches full hit pose");
        // After the fixed ticks the machine returns to its state, blends
        // out, and walks on with full root intent again.
        assert!(!m.interrupt_active());
        assert_eq!(m.interrupt_weight(), 0.0);
        assert_eq!(m.state, LocoState::Walk);
        let after = m.update(1.5, fwd, 200, None).unwrap();
        assert_ne!(after.pose, held_pose, "envelope blends back out");
        assert!((after.root.dp[1] - 1.5 * TICK_DT).abs() < 1e-6, "{after:?}");
        validate_pose(&skeleton(), &after.pose).unwrap();
        // Heavy interrupts hold longer and also return.
        let out = m
            .update(1.5, fwd, 300, Some(CombatInterrupt::Heavy))
            .unwrap();
        assert!(m.interrupt_active());
        assert_eq!(out.root.dp, [0.0, 0.0]);
        for t in 301..(300 + HIT_HEAVY_TICKS) {
            m.update(1.5, fwd, t, None).unwrap();
        }
        assert!(!m.interrupt_active());
        assert_eq!(m.state, LocoState::Walk);
    }

    #[test]
    fn bounded_inputs_rejected_without_mutation() {
        let mut m = Locomotion::new();
        let fwd = [0.0, 1.0];
        for t in 0..(START_TICKS + 2) {
            m.update(1.5, fwd, t, None).unwrap();
        }
        let before = m.clone();
        assert_eq!(
            m.update(-0.5, fwd, 50, None),
            Err(LocoError::NegativeSpeed(-0.5))
        );
        assert_eq!(
            m.update(f32::NAN, fwd, 50, None),
            Err(LocoError::NonFiniteSpeed)
        );
        assert_eq!(
            m.update(f32::INFINITY, fwd, 50, None),
            Err(LocoError::NonFiniteSpeed)
        );
        assert_eq!(
            m.update(MAX_WISH_SPEED + 0.5, fwd, 50, None),
            Err(LocoError::SpeedTooHigh(MAX_WISH_SPEED + 0.5))
        );
        assert_eq!(
            m.update(1.0, [f32::NAN, 0.0], 50, None),
            Err(LocoError::NonFiniteWishDir)
        );
        assert_eq!(
            m.update(1.0, [f32::INFINITY, 0.0], 50, None),
            Err(LocoError::NonFiniteWishDir)
        );
        assert_eq!(
            m.update(1.0, [0.0, 0.0], 50, None),
            Err(LocoError::ZeroWishDir)
        );
        assert_eq!(m, before, "rejected input mutates nothing");
        // Dropping the wish enters Stop (decaying root), not a hard cut.
        let out = m.update(0.0, [0.0, 0.0], 51, None).unwrap();
        assert_eq!(m.state, LocoState::Stop);
        assert!(
            out.root.dp[1] > 0.0 && out.root.dp[1] < 1.5 * TICK_DT,
            "{out:?}"
        );
    }

    #[test]
    fn repeatable_authoritative_state() {
        let script: Vec<(f32, [f32; 2], u32, Option<CombatInterrupt>)> = vec![
            (0.0, [0.0, 1.0], 0, None),
            (0.0, [0.0, 1.0], 1, None),
            (1.2, [0.0, 1.0], 2, None),
            (1.2, [0.0, 1.0], 3, None),
            (4.0, [0.2, 0.9], 4, None),
            (4.0, [0.2, 0.9], 5, Some(CombatInterrupt::Light)),
            (4.0, [0.2, 0.9], 6, None),
            (1.5, [1.0, 0.0], 7, None),
            (1.5, [-0.2, -0.9], 8, None),
            (0.0, [0.0, 1.0], 9, None),
            (0.0, [0.0, 1.0], 10, Some(CombatInterrupt::Heavy)),
            (0.0, [0.0, 1.0], 11, None),
            (2.5, [0.5, 0.5], 12, None),
        ];
        let run = || {
            let mut m = Locomotion::new();
            let mut outs = Vec::new();
            for (speed, dir, tick, irq) in &script {
                outs.push(m.update(*speed, *dir, *tick, *irq).unwrap());
            }
            (m, outs)
        };
        let (m1, outs1) = run();
        let (m2, outs2) = run();
        assert_eq!(m1, m2);
        assert_eq!(outs1, outs2);
        // An air/land excursion on a separate interrupt-free script is also
        // repeatable, and diverges from the grounded run (input changes
        // cover trajectories).
        let div = |air: bool| {
            let mut m = Locomotion::new();
            let mut outs = Vec::new();
            for t in 0..20 {
                if air && t == 12 {
                    m.set_airborne(true);
                }
                if air && t == 15 {
                    m.set_airborne(false);
                }
                outs.push(m.update(1.5, [0.0, 1.0], t, None).unwrap());
            }
            outs
        };
        assert_eq!(div(false), div(false));
        assert_eq!(div(true), div(true));
        assert_ne!(
            div(false),
            div(true),
            "air excursion must change the trajectory"
        );
        // Every output validates: root tables order, contacts order, poses.
        for outs in [&outs1, &div(true)] {
            let table = RootTable {
                deltas: outs.iter().map(|o| o.root).collect(),
            };
            validate_root(&table).unwrap();
            for o in outs {
                validate_pose(&skeleton(), &o.pose).unwrap();
                validate_contacts(&o.contacts).unwrap();
            }
        }
    }

    #[test]
    fn dyaw_matches_facing_turn() {
        // The reported yaw equals the proportional law on the pre-turn
        // facing to first order; assert the sign convention (+Z to +X is
        // positive) and the rate limit directly.
        let mut m = Locomotion::new();
        for t in 0..(START_TICKS + 2) {
            m.update(1.5, [0.0, 1.0], t, None).unwrap();
        }
        let before = m.facing();
        // Mild diagonal wish stays Walk (no strafe) and turns at the rate
        // limit: cross ~ -0.29, so -cross saturates TURN_WALK exactly.
        let out = m.update(1.5, [0.3, 1.0], 100, None).unwrap();
        assert_eq!(m.state, LocoState::Walk);
        // Wish +X from facing +Z: cross < 0, dyaw = +rate (toward +X).
        assert!((out.root.dyaw - TURN_WALK).abs() < 1e-6, "{out:?}");
        assert!(m.facing()[0] > before[0], "facing turns toward +X");
    }

    #[test]
    fn presentation_valid_in_every_state() {
        // Sweep every state through update paths and validate the pose.
        let mut m = Locomotion::new();
        let fwd = [0.0, 1.0];
        let check = |m: &Locomotion, out: &LocoOutput| {
            validate_pose(&skeleton(), &out.pose).unwrap();
            assert_eq!(out.pose.positions.len(), LOCO_JOINT_COUNT);
            assert!(
                (0.0..1.0).contains(&m.phase()),
                "phase bounded: {}",
                m.phase()
            );
        };
        let o = m.update(0.0, fwd, 0, None).unwrap();
        check(&m, &o);
        for t in 1..(START_TICKS + 3) {
            let o = m.update(1.5, fwd, t, None).unwrap();
            check(&m, &o);
        }
        let o = m.update(4.5, fwd, 50, None).unwrap();
        check(&m, &o);
        let o = m.update(1.0, [1.0, 0.2], 51, None).unwrap();
        check(&m, &o);
        let o = m.update(1.0, [-0.1, -1.0], 52, None).unwrap();
        check(&m, &o);
        m.set_airborne(true);
        let o = m.update(2.0, fwd, 53, None).unwrap();
        check(&m, &o);
        m.set_airborne(false);
        for t in 54..(54 + LAND_TICKS) {
            let o = m.update(0.0, fwd, t, None).unwrap();
            check(&m, &o);
        }
        let o = m
            .update(0.0, fwd, 100, Some(CombatInterrupt::Light))
            .unwrap();
        check(&m, &o);
    }

    /// Authority script covering every branch the tick core can take:
    /// idle, Start ramp, Walk, Run, Strafe, Stop, Pivot, Light and Heavy
    /// interrupts (including a restart mid-hold), Air and Land. Each entry
    /// is one tick; ticks auto-increment in the driver.
    struct ScriptTick {
        speed: f32,
        dir: [f32; 2],
        irq: Option<CombatInterrupt>,
        air: Option<bool>,
    }

    fn authority_script() -> Vec<ScriptTick> {
        let fwd = [0.0, 1.0];
        let side = [1.0, 0.0];
        let back = [0.0, -1.0];
        let mut script = Vec::new();
        let mut push =
            |speed: f32, dir: [f32; 2], irq: Option<CombatInterrupt>, air: Option<bool>| {
                script.push(ScriptTick {
                    speed,
                    dir,
                    irq,
                    air,
                });
            };
        for _ in 0..3 {
            push(0.0, fwd, None, None); // idle dwell
        }
        for _ in 0..(START_TICKS + 2) {
            push(1.5, fwd, None, None); // Start ramp into Walk
        }
        push(4.0, fwd, None, None); // Walk -> Run
        for _ in 0..3 {
            push(4.0, fwd, None, None); // steady Run
        }
        push(1.5, side, None, None); // Run -> Strafe, facing held
        for _ in 0..3 {
            push(1.5, side, None, None);
        }
        push(1.5, back, None, None); // reversal -> Pivot
        for _ in 0..(PIVOT_TICKS + 1) {
            push(1.5, fwd, None, None); // pivot resolves toward Walk
        }
        push(1.5, fwd, Some(CombatInterrupt::Light), None); // hit-stop
        for _ in 0..4 {
            push(1.5, fwd, None, None); // hold mid-envelope ...
        }
        push(1.5, fwd, Some(CombatInterrupt::Heavy), None); // ... restart
        for _ in 0..HIT_HEAVY_TICKS {
            push(1.5, fwd, None, None); // hold to expiry, walk on
        }
        push(0.0, fwd, None, Some(true)); // take off
        for _ in 0..3 {
            push(2.0, fwd, None, None); // Air steering
        }
        push(0.0, fwd, None, Some(false)); // touchdown -> Land
        for _ in 0..LAND_TICKS {
            push(0.0, fwd, None, None); // planted recovery
        }
        push(0.0, fwd, None, None); // resolved Idle
        script
    }

    #[test]
    fn root_advance_matches_full_update_on_every_branch() {
        // The zero-alloc entry must be the same state machine, not a second
        // one: per tick the root delta and the whole machine equal the full
        // path, and the root-only table validates.
        let script = authority_script();
        let mut full = Locomotion::new();
        let mut roots = Locomotion::new();
        let mut full_roots = Vec::with_capacity(script.len());
        let mut root_only = Vec::with_capacity(script.len());
        for (i, entry) in script.iter().enumerate() {
            let tick = i as u32;
            if let Some(lift) = entry.air {
                full.set_airborne(lift);
                roots.set_airborne(lift);
            }
            full_roots.push(
                full.update(entry.speed, entry.dir, tick, entry.irq)
                    .unwrap()
                    .root,
            );
            root_only.push(
                roots
                    .update_root(entry.speed, entry.dir, tick, entry.irq)
                    .unwrap(),
            );
            assert_eq!(full, roots, "machine state diverges at tick {tick}");
        }
        assert_eq!(full_roots, root_only);
        // The script really visited the branches above (not an idle line).
        assert!(full_roots.iter().any(|r| r.dp != [0.0, 0.0]));
        assert!(full_roots.iter().any(|r| r.dyaw != 0.0));
        let table = RootTable { deltas: root_only };
        validate_root(&table).unwrap();
    }

    #[test]
    fn root_advance_rejects_bad_input_without_mutation() {
        // Failure parity with `update`: the same typed errors, and a
        // rejected tick mutates nothing.
        let mut m = Locomotion::new();
        let fwd = [0.0, 1.0];
        for t in 0..(START_TICKS + 2) {
            m.update_root(1.5, fwd, t, None).unwrap();
        }
        let before = m.clone();
        assert_eq!(
            m.update_root(-0.5, fwd, 50, None),
            Err(LocoError::NegativeSpeed(-0.5))
        );
        assert_eq!(
            m.update_root(f32::NAN, fwd, 50, None),
            Err(LocoError::NonFiniteSpeed)
        );
        assert_eq!(
            m.update_root(MAX_WISH_SPEED + 0.5, fwd, 50, None),
            Err(LocoError::SpeedTooHigh(MAX_WISH_SPEED + 0.5))
        );
        assert_eq!(
            m.update_root(1.0, [f32::NAN, 0.0], 50, None),
            Err(LocoError::NonFiniteWishDir)
        );
        assert_eq!(
            m.update_root(1.0, [0.0, 0.0], 50, None),
            Err(LocoError::ZeroWishDir)
        );
        assert_eq!(m, before, "rejected input mutates nothing");
    }

    #[test]
    fn alloc_probe_sees_a_real_allocation() {
        // Calibration: a blind counter must not fake the readings below.
        let ((), heap) = measure(|| {
            std::hint::black_box(vec![0u8; 64]);
        });
        assert!(heap.calls >= 1, "alloc probe is blind");
    }

    #[test]
    fn full_update_allocates_three_per_steady_tick() {
        // Calibration for the arena allocation floor: pose positions, pose
        // rotations and contacts, one heap block each. Fails if the footprint
        // moves, so the arena steady floor is recounted instead of silently
        // drifting.
        let mut m = Locomotion::new();
        let fwd = [0.0, 1.0];
        for t in 0..20 {
            m.update(1.5, fwd, t, None).unwrap();
        }
        let ((), heap) = measure(|| {
            for t in 20..80 {
                let out = m.update(1.5, fwd, t, None).unwrap();
                std::hint::black_box(out);
            }
        });
        assert_eq!(
            heap.calls,
            3 * 60,
            "locomotion share moved: recount the arena steady floor"
        );
    }

    #[test]
    fn root_advance_allocates_nothing_on_any_branch() {
        // The authority entry skips pose/contacts construction on every
        // branch (idle, transients, air, land, both interrupts), so the
        // arena authority tick can take it with zero per-tick heap cost.
        let script = authority_script();
        let mut m = Locomotion::new();
        let ((), heap) = measure(|| {
            for (i, entry) in script.iter().enumerate() {
                if let Some(lift) = entry.air {
                    m.set_airborne(lift);
                }
                let root = m
                    .update_root(entry.speed, entry.dir, i as u32, entry.irq)
                    .unwrap();
                std::hint::black_box(root);
            }
        });
        assert_eq!(heap.calls, 0, "authority tick must stay allocation-free");
    }
}
