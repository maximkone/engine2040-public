//! spear: the throwable spear and its grapple tether, integer authority.
//!
//! Plan: `docs/plan/spear-grapple.md` (read it for the rules and constants).
//! A fighter presses `THROW_SPEAR`; a spear flies along the thrower's facing
//! and either tethers the thrower to an enemy (both are then carried toward
//! the spear's end point with momentum, damage on arrival or on a wall slam)
//! or anchors on world geometry (the thrower zips to it, Spiderman style).
//! Pressing `THROW_SPEAR_WORLD` with it aims past fighters at the geometry, so
//! a wall behind the enemy can be the anchor; a body in the air may throw
//! again at once, so anchors chain.
//!
//! Consumer: `Sim::step_with`, only for a match built from a
//! [`MatchSpec`](crate::MatchSpec) with `spear` on; the window reads
//! [`SpearEvent`]s and [`SpearSlot`]s through `LiveMatch::spear_events` and
//! `LiveMatch::spear_view`. Data owner: [`SpearState`], owned by the `Sim`.
//! Units: integer millimetres on y-up axes, velocities in mm per tick (60 Hz),
//! directions as Q16 unit vectors. Determinism: `+ - * /`, `isqrt`, shifts and
//! comparisons on integers, plus the Q16 `swept_cast` (PX20) for the hit test;
//! slots in order, no float ever enters the hash (bodies move by an integer
//! displacement added onto the float pose, like the collision pass does).
//! Errors: none returned; a cast the physics refuses counts as a miss.
//! Budget: no heap per tick beyond the event vec reserved once.

use engine_combat::fight::{FightState, Tables};
use engine_core::fixed::Fixed;
use engine_core::hash::Fnv64;
use engine_input::fighter::{THROW_NOOSE, THROW_SPEAR, THROW_SPEAR_WORLD};
use engine_input::FighterInput;
use engine_physics::query::{swept_cast, CastCapsule, SweptHit};

use crate::collide::collide_states;
use crate::match_wire::{facing_vector, metres, pos_mm, sweep_for_attacker};
use crate::world::MatchWorld;
use crate::{
    canon_zero, facing_q16, wall_limit, ArenaRules, FighterState, FIGHTER_RADIUS_M, MAX_SLOTS,
};

mod noose;

/// Muzzle speed along the facing, mm per tick (36 m/s).
const SPEAR_SPEED: i32 = 600;
/// Vertical drop per tick squared, mm.
const SPEAR_GRAVITY: i32 = 3;
/// Spawn distance ahead of the thrower, mm.
const SPEAR_AHEAD: i32 = 400;
/// Spawn height over the floor, mm (chest).
const SPEAR_HEIGHT: i32 = 1300;
/// Longest flight, mm.
const SPEAR_RANGE: i32 = 12_000;
/// Flight ticks the range allows (`SPEAR_RANGE / SPEAR_SPEED`).
const MAX_FLIGHT_TICKS: u16 = 20;
/// Spear tip radius for the capsule sweep, mm.
const TIP_RADIUS: i32 = 80;
/// Spear tip radius for the obstacle test, metres.
const TIP_RADIUS_M: f32 = 0.05;
/// Fighter hurt capsule: axis from this height to `CAPSULE_TOP`, mm.
const CAPSULE_BOTTOM: i32 = 400;
/// Top of the fighter hurt capsule axis, mm.
const CAPSULE_TOP: i32 = 1400;
/// Fighter hurt capsule radius, mm.
const CAPSULE_RADIUS: i32 = 400;
/// Sub-step length when marching a flight segment through geometry, mm.
const MARCH_STEP: i32 = 100;
/// Ticks from a throw until the next one.
const THROW_COOLDOWN: u16 = 90;
/// Cooldown restarted when a tether ends.
const TETHER_COOLDOWN: u16 = 60;
/// Damage at the impact on an enemy, permille.
const HIT_DAMAGE: u16 = 5;
/// Distance at which the two bodies latch together, mm.
const REST: i64 = 1600;
/// Thrower acceleration toward the enemy before the latch, mm per tick squared.
const REEL_ACCEL: i64 = 20;
/// Acceleration of a body toward the goal, mm per tick squared.
const CARRY_ACCEL: i64 = 12;
/// Speed cap of an enemy tether, mm per tick (25 m/s).
const MAX_SPEED: i64 = 420;
/// Thrower acceleration toward a wall anchor.
const WALL_REEL_ACCEL: i64 = 28;
/// Speed cap of a wall tether.
const WALL_MAX_SPEED: i64 = 500;
/// Rope taken in per tick by a swinging tether, mm (row SG4): 3.6 m/s at the
/// fixed tick, so a hook reels the thrower up its arc.
const SWING_REEL: i32 = 60;
/// Damping of a swinging tether: a sixty-fourth of the speed per tick, so a
/// release still carries most of the arc.
const SWING_DAMP_DIV: i32 = 64;
/// Shortest rope a swing reels down to, mm.
const MIN_ROPE_MM: i32 = 400;
/// Goal radius that ends a pull, mm.
const ARRIVE: i64 = 700;
/// How far short of a wall anchor the thrower stops, mm.
const ANCHOR_BACKOFF: i64 = 700;
/// How far past the victim the goal lies at least, mm.
const MIN_CARRY: i32 = 2500;
/// Ticks before a tether times out.
const TETHER_MAX: u16 = 90;
/// Ticks between two [`SpearEvent::TetherPull`] cues.
const PULL_CUE_PERIOD: u16 = 6;
/// Arrival damage base, permille; speed adds `speed / 8`.
const ARRIVE_DAMAGE: i64 = 40;
/// Slam damage base, permille; speed adds `speed / 6`.
const SLAM_DAMAGE: i64 = 60;
/// Most damage one tether deals, permille.
const MAX_TETHER_DAMAGE: i64 = 200;
/// Slowest body that counts as slammed when its move comes up short.
const MIN_SLAM_SPEED: i64 = 60;
/// Steps a rope-snag march takes, [`MARCH_STEP`] each: 6.4 m, the longest
/// rope a wall anchor leaves.
const WRAP_STEPS: i64 = 64;
/// Yank snap speed, mm per tick (24 m/s), row SG9: a thrower who holds back
/// as the spear sticks reels the enemy in. The victim moves at this speed
/// from the first pulled tick; there is no ramp, which is the snap.
const YANK_SPEED: i64 = 400;
/// Gap a yank closes to, mm: inside blade reach (`BLADE_REACH_MM` 1500), so
/// the victim lands in front of the thrower's sword.
const YANK_STOP: i64 = 1200;
/// Yank ease: the victim covers at most a quarter of the gap left per tick,
/// so the snap slows into the stop instead of overshooting it.
const YANK_EASE_DIV: i64 = 4;
/// Slowest closing speed of a yank, mm per tick, so the ease finishes.
const YANK_MIN_SPEED: i64 = 30;
/// Hit-stop when a yank hook sticks, ticks: the freeze sells the catch.
const YANK_HITSTOP: u32 = 5;
/// Stun a yanked victim holds through the pull and after the stop, ticks:
/// no guard cuts the rope, and the thrower gets one free swing.
const YANK_STUN: u32 = 20;
/// Damage when a yank lands the victim, permille.
const YANK_ARRIVE_DAMAGE: i64 = 20;
/// Fold byte of a yank tether (a carry folds nothing extra).
const YANK_TAG: u8 = 0xA7;
/// Ticks from the impact tick during which a swinger's blade is still
/// moving through its arc and can cut a rope (row SG13).
const CUT_LIVE_TICKS: u32 = 4;
/// Half the rope's thickness for the cut test, mm (the blade adds its own).
const ROPE_HALF_MM: i64 = 60;
/// Event vec capacity reserved once.
const EVENT_CAP: usize = 4 * MAX_SLOTS;
/// Fold tag byte.
const FOLD_TAG: u8 = 0x5A;

/// Why a spear flight ended without a hit.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MissWhy {
    /// The flight used up its range.
    Range,
    /// The spear dropped to the floor.
    Floor,
}

/// Why a tether ended.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EndWhy {
    /// The pair reached the spear's end point.
    Arrived,
    /// A body hit a wall, obstacle or pillar.
    Wall,
    /// The tether lived its full length.
    Timeout,
    /// The thrower pressed the button again.
    Released,
    /// The victim raised a guard.
    Guard,
    /// The victim parried (the thrower staggers).
    Parry,
    /// Either side went out of the fight.
    Down,
    /// A swing's blade crossed the rope (row SG13): the victim's escape, or a
    /// third fighter's. The spear drops.
    Cut,
    /// A shield push landed on a snared man (row SG11): the noose comes off
    /// and he goes down.
    Toppled,
}

/// One thing the spear did this tick, for presentation to react to.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SpearEvent {
    /// A spear left the thrower.
    Thrown {
        /// Thrower slot.
        thrower: u8,
        /// Spawn point, mm.
        at_mm: [i32; 3],
        /// Initial velocity, mm per tick.
        vel_mm: [i32; 3],
    },
    /// The spear stuck: in a fighter, or in world geometry (`victim` is `None`).
    Hit {
        /// Thrower slot.
        thrower: u8,
        /// Struck fighter, `None` for a world anchor.
        victim: Option<u8>,
        /// Contact point, mm.
        at_mm: [i32; 3],
    },
    /// The spear flew out without a hit and no tether formed.
    Missed {
        /// Thrower slot.
        thrower: u8,
        /// Why the flight ended.
        why: MissWhy,
    },
    /// A guard stopped the spear; no tether formed.
    Deflected {
        /// Thrower slot.
        thrower: u8,
        /// Guarding fighter.
        victim: u8,
        /// True when the guard was inside its parry window.
        parried: bool,
    },
    /// The tether is pulling (every sixth tether tick).
    TetherPull {
        /// Thrower slot.
        thrower: u8,
        /// Tethered fighter, `None` for a world anchor.
        victim: Option<u8>,
        /// Thrower speed, mm per tick.
        speed: u16,
    },
    /// The tether ended.
    TetherEnd {
        /// Thrower slot.
        thrower: u8,
        /// Tethered fighter, `None` for a world anchor.
        victim: Option<u8>,
        /// Why it ended.
        why: EndWhy,
        /// Damage dealt to the victim by the end, permille.
        damage_permille: u16,
    },
}

/// A spear in the air.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Flight {
    /// Spear tip, mm.
    pub pos_mm: [i32; 3],
    /// Velocity, mm per tick.
    pub vel_mm: [i32; 3],
    /// Ticks in the air.
    pub age: u16,
    /// Thrower position at the throw, `[x, z]` mm.
    pub origin_mm: [i32; 2],
    /// Flight direction `[x, z]` as a Q16 unit vector.
    pub dir: [i32; 2],
    /// The throw was aimed at the world (`THROW_SPEAR_WORLD`): fighters are
    /// not targets, so the spear flies past them and sticks in geometry.
    pub world: bool,
    /// The flight is a noose (row SG11, `THROW_NOOSE`): a loop on a cord that
    /// snares the man it catches instead of hooking him. It never anchors.
    pub noose: bool,
}

/// A tether between a thrower and an enemy or a world anchor.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Tether {
    /// Tethered fighter, `None` for a world anchor.
    pub victim: Option<u8>,
    /// Where the pull is headed, `[x, z]` mm.
    pub goal_mm: [i32; 2],
    /// Pivot of a world tether, `[x, z]` mm: the point the spear stuck in. It
    /// equals `goal_mm` on an enemy tether, where there is no rope.
    pub anchor_mm: [i32; 2],
    /// Height of the pivot, mm, for the swing's sphere.
    pub anchor_y_mm: i32,
    /// Rope length, mm. Zero on an enemy tether and until a swing's first
    /// tethered tick; a swung thrower stays on that sphere.
    pub rope_mm: i32,
    /// Flight direction `[x, z]` as a Q16 unit vector.
    pub dir: [i32; 2],
    /// Ticks since the spear stuck.
    pub age: u16,
    /// True once the two bodies share one momentum.
    pub latched: bool,
    /// True when the hook reels the victim to a planted thrower (row SG9: the
    /// thrower held back as the spear stuck); false is the carry.
    pub yank: bool,
    /// True when the cord is a noose that snared the victim (row SG11): no
    /// carry, no pull on the thrower; the cord only drags a man who pulls away.
    pub noose: bool,
    /// Thrower speed after the last tick, mm per tick (the victim's on a
    /// yank, where the thrower stands).
    pub speed: u16,
}

/// What one slot's spear is doing.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Phase {
    /// No spear out.
    #[default]
    Idle,
    /// A spear is flying.
    Flight(Flight),
    /// A tether holds.
    Tether(Tether),
}

/// One slot's spear state: the whole per-slot view the window reads.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct SpearSlot {
    /// Flight or tether in progress.
    pub phase: Phase,
    /// Ticks until the next throw is allowed.
    pub cooldown: u16,
    /// Whether the button was down last tick (edge detection).
    pub held: bool,
    /// Body momentum, `[x, z]` mm per tick.
    pub drift_mm: [i32; 2],
    /// Point the rope turns on while it is snagged on a corner, mm; all zero
    /// when the rope runs straight to its anchor. Presentation reads it to
    /// bend the drawn rope; it never feeds back into the step.
    pub wrap_mm: [i32; 3],
}

/// Everything the spear step touches in the sim, borrowed for one tick.
pub(crate) struct Env<'a> {
    /// This tick's merged inputs, one per fighter.
    pub inputs: &'a [FighterInput],
    /// Fighter poses (the bodies move here).
    pub states: &'a mut [FighterState],
    /// Combat state (damage, stagger, guard reads).
    pub fight: &'a mut FightState,
    /// Fight rules (teams, pillars, collision).
    pub rules: &'a ArenaRules,
    /// Static obstacle world.
    pub world: &'a MatchWorld,
    /// Move tables of the match.
    pub tables: &'a Tables,
    /// Arena half-extent, metres.
    pub half_extent_m: f32,
    /// Height channel vertical speeds (row SG5), one per fighter, mm per
    /// tick. Empty while the channel is off, which keeps the wall tether on
    /// its straight zip and leaves every hash as it was.
    pub rise: &'a mut [i32],
}

/// All spear state of one match.
pub struct SpearState {
    slots: [SpearSlot; MAX_SLOTS],
    fighters: usize,
    events: Vec<SpearEvent>,
    hits: Vec<SweptHit>,
    /// Per-tick scratch: `(blocked, speed before the stop, pinned)` per slot;
    /// pinned is a blocked body that barely moved (it did not slide).
    shocks: [(bool, i64, bool); MAX_SLOTS],
    /// `THROW_SPEAR` throws a spear (on unless the state was built noose-only).
    spear_on: bool,
    /// `THROW_NOOSE` throws a noose (row SG11); off folds and runs nothing.
    noose_on: bool,
    /// Whether `THROW_NOOSE` was down last tick, per slot (the press edge).
    noose_held: [bool; MAX_SLOTS],
    /// A shield push landed on this snared man and waits for the freeze to end.
    pushed: [bool; MAX_SLOTS],
    /// `(thrower, victim)` of every man a push toppled in the last step.
    toppled: Vec<(u8, u8)>,
}

impl SpearState {
    /// Fresh state for `fighters` slots, nobody throwing.
    pub(crate) fn new(fighters: usize) -> Self {
        Self {
            slots: [SpearSlot::default(); MAX_SLOTS],
            fighters,
            events: Vec::with_capacity(EVENT_CAP),
            hits: Vec::with_capacity(MAX_SLOTS),
            shocks: [(false, 0, false); MAX_SLOTS],
            spear_on: true,
            noose_on: false,
            noose_held: [false; MAX_SLOTS],
            pushed: [false; MAX_SLOTS],
            toppled: Vec::with_capacity(MAX_SLOTS),
        }
    }

    /// Per-slot state of the last stepped tick.
    pub(crate) fn slots(&self) -> &[SpearSlot] {
        &self.slots[..self.fighters]
    }

    /// Events of the last stepped tick, in emission order.
    pub(crate) fn events(&self) -> &[SpearEvent] {
        &self.events
    }

    /// Fold the spear state into a checkpoint hash.
    pub(crate) fn fold(&self, hash: u64) -> u64 {
        let mut digest = Fnv64::with_state(hash);
        digest.write_u8(FOLD_TAG);
        for slot in self.slots() {
            digest.write_u16(slot.cooldown);
            digest.write_bool(slot.held);
            for lane in slot.drift_mm {
                digest.write_i32(lane);
            }
            for lane in slot.wrap_mm {
                digest.write_i32(lane);
            }
            match slot.phase {
                Phase::Idle => digest.write_u8(0),
                Phase::Flight(f) => {
                    digest.write_u8(1);
                    for lane in f.pos_mm.into_iter().chain(f.vel_mm).chain(f.origin_mm) {
                        digest.write_i32(lane);
                    }
                    digest.write_u16(f.age);
                    digest.write_bool(f.world);
                    if f.noose {
                        digest.write_u8(noose::TAG);
                    }
                }
                Phase::Tether(t) => {
                    digest.write_u8(2);
                    digest.write_u8(t.victim.map_or(u8::MAX, |v| v));
                    for lane in t.goal_mm.into_iter().chain(t.dir) {
                        digest.write_i32(lane);
                    }
                    digest.write_u16(t.age);
                    digest.write_bool(t.latched);
                    if t.yank {
                        digest.write_u8(YANK_TAG);
                    }
                    if t.noose {
                        digest.write_u8(noose::TAG);
                        digest.write_i32(t.rope_mm);
                    }
                }
            }
        }
        if self.noose_on {
            self.fold_noose(&mut digest);
        }
        digest.finish()
    }

    /// One tick: presses and flights, tether physics, body motion, endings.
    pub(crate) fn step(&mut self, env: &mut Env<'_>) {
        self.events.clear();
        self.toppled.clear();
        self.topple_pushed(env);
        for slot in 0..self.fighters {
            self.act(slot, env);
        }
        self.drift(env);
        for slot in 0..self.fighters {
            self.settle(slot, env);
        }
        for slot in 0..self.fighters {
            if !self.tethered(slot) {
                let drift = &mut self.slots[slot].drift_mm;
                *drift = drift.map(friction);
            }
        }
    }

    /// True while `slot` is the thrower or the victim of a live tether.
    fn tethered(&self, slot: usize) -> bool {
        self.slots[..self.fighters]
            .iter()
            .enumerate()
            .any(|(thrower, s)| match s.phase {
                Phase::Tether(t) => thrower == slot || t.victim.map(usize::from) == Some(slot),
                _ => false,
            })
    }

    /// The press edge, then the slot's flight or tether for the tick.
    fn act(&mut self, slot: usize, env: &mut Env<'_>) {
        let input = env.inputs[slot];
        let pressed = self.spear_on && input.buttons.0 & THROW_SPEAR != 0;
        let edge = pressed && !self.slots[slot].held;
        self.slots[slot].held = pressed;
        // Row SG11: the noose has its own button and its own press edge.
        let cord = self.noose_on && input.buttons.0 & THROW_NOOSE != 0;
        let cord_edge = cord && !self.noose_held[slot];
        self.noose_held[slot] = cord;
        let cooldown = self.slots[slot].cooldown.saturating_sub(1);
        self.slots[slot].cooldown = cooldown;
        match self.slots[slot].phase {
            Phase::Idle => {
                // The floor cooldown paces a duel on the ground. A body in the
                // air (the height channel carries it) may throw again at once,
                // so a swing chains into the next anchor without dead time.
                let airborne = env.rise.get(slot).is_some_and(|&rise| rise != 0)
                    || pos_mm(env.states[slot].pos)[1] > 0;
                let ready = (cooldown == 0 || airborne) && env.fight.fighters[slot].can_act();
                if edge && ready {
                    self.throw(slot, input, env);
                    self.fly(slot, env);
                } else if cord_edge && ready {
                    self.throw_noose(slot, input, env);
                    self.fly(slot, env);
                }
            }
            Phase::Flight(_) => self.fly(slot, env),
            Phase::Tether(t) if (edge && !t.noose) || (cord_edge && t.noose) => {
                self.end(slot, EndWhy::Released, 0, env);
            }
            Phase::Tether(_) => self.pull(slot, env),
        }
    }

    /// Launch a spear from `slot` along its facing.
    fn throw(&mut self, slot: usize, input: FighterInput, env: &Env<'_>) {
        let (sin, cos) = facing_q16(input.facing);
        let dir = [sin, cos];
        let body = pos_mm(env.states[slot].pos);
        let (pos_mm, vel_mm) = throw_start(body, dir, SPEAR_AHEAD, SPEAR_HEIGHT, SPEAR_SPEED);
        self.slots[slot].cooldown = THROW_COOLDOWN;
        self.slots[slot].wrap_mm = [0; 3];
        self.slots[slot].phase = Phase::Flight(Flight {
            pos_mm,
            vel_mm,
            age: 0,
            origin_mm: [body[0], body[2]],
            dir,
            world: input.buttons.0 & THROW_SPEAR_WORLD != 0,
            noose: false,
        });
        self.events.push(SpearEvent::Thrown {
            thrower: slot_id(slot),
            at_mm: pos_mm,
            vel_mm,
        });
    }

    /// Advance a flying spear one tick and resolve what it meets.
    fn fly(&mut self, slot: usize, env: &mut Env<'_>) {
        let Phase::Flight(mut f) = self.slots[slot].phase else {
            return;
        };
        f.vel_mm[1] -= SPEAR_GRAVITY;
        f.age += 1;
        let from = f.pos_mm;
        let full = [
            from[0] + f.vel_mm[0],
            from[1] + f.vel_mm[1],
            from[2] + f.vel_mm[2],
        ];
        let (to, geometry) = march_segment(env.world, env.rules, env.half_extent_m, from, full);
        // A world-aimed throw has no fighter targets: it flies past them and
        // anchors the geometry behind, which is how a duel reaches a wall.
        let struck = if f.world {
            None
        } else if f.noose {
            self.cast_loop(slot, env, from, to)
        } else {
            self.cast(slot, env, from, to)
        };
        if let Some((victim, at)) = struck {
            if f.noose {
                self.catch(slot, victim, at, f, env);
            } else {
                self.strike(slot, victim, at, f, env);
            }
        } else if geometry && f.noose {
            // A loop is not a hook: it slaps the wall and falls.
            self.miss(slot, MissWhy::Range);
        } else if geometry {
            self.anchor(slot, to, f);
        } else if to[1] <= 0 {
            self.miss(slot, MissWhy::Floor);
        } else if f.age >= noose::max_ticks(f.noose) {
            self.miss(slot, MissWhy::Range);
        } else {
            f.pos_mm = to;
            self.slots[slot].phase = Phase::Flight(f);
        }
    }

    /// End a flight with no tether.
    fn miss(&mut self, slot: usize, why: MissWhy) {
        self.slots[slot].phase = Phase::Idle;
        self.events.push(SpearEvent::Missed {
            thrower: slot_id(slot),
            why,
        });
    }

    /// The nearest legal fighter the segment `from..to` meets, with the
    /// contact point. Legal: living, not the thrower, not a teammate, not in
    /// dodge i-frames.
    fn cast(
        &mut self,
        slot: usize,
        env: &Env<'_>,
        from: [i32; 3],
        to: [i32; 3],
    ) -> Option<(usize, [i32; 3])> {
        let mut caps = [CastCapsule {
            id: 0,
            a: [Fixed::ZERO; 3],
            b: [Fixed::ZERO; 3],
            radius: Fixed::ZERO,
        }; MAX_SLOTS];
        let mut count = 0;
        for (victim, state) in env.states.iter().enumerate().take(self.fighters) {
            let combat = &env.fight.fighters[victim];
            if victim == slot
                || combat.hp_permille == 0
                || env.rules.teams[victim] == env.rules.teams[slot]
                || combat.invulnerable_with(env.tables)
            {
                continue;
            }
            let body = pos_mm(state.pos);
            caps[count] = hurt_capsule(u32::from(slot_id(victim)), body);
            count += 1;
        }
        let (id, at) = cast_contact(&mut self.hits, from, to, &caps[..count])?;
        Some((usize::try_from(id).ok()?, at))
    }

    /// The spear met fighter `victim`: a guard turns it away, anything else
    /// sticks and tethers.
    fn strike(&mut self, slot: usize, victim: usize, at: [i32; 3], f: Flight, env: &mut Env<'_>) {
        if self.guard_turns(slot, victim, env) {
            return;
        }
        hurt(env, victim, i64::from(HIT_DAMAGE));
        let yank = holds_back(env.inputs[slot], f.dir);
        if yank {
            env.fight.hitstop = env.fight.hitstop.max(YANK_HITSTOP);
            stun(env, victim, YANK_STUN);
            self.slots[slot].drift_mm = [0, 0];
            self.slots[victim].drift_mm = [0, 0];
        }
        let travelled = isqrt(sq(at[0] - f.origin_mm[0]) + sq(at[2] - f.origin_mm[1]));
        let reach = i64::from(SPEAR_RANGE).max(travelled + i64::from(MIN_CARRY));
        let goal_mm = [0, 1].map(|lane| {
            f.origin_mm[lane] + i32::try_from(i64::from(f.dir[lane]) * reach / 65_536).unwrap_or(0)
        });
        self.slots[slot].phase = Phase::Tether(Tether {
            victim: Some(slot_id(victim)),
            goal_mm,
            anchor_mm: goal_mm,
            anchor_y_mm: at[1],
            rope_mm: 0,
            dir: f.dir,
            age: 0,
            latched: false,
            yank,
            noose: false,
            speed: 0,
        });
        self.events.push(SpearEvent::Hit {
            thrower: slot_id(slot),
            victim: Some(slot_id(victim)),
            at_mm: at,
        });
    }

    /// True when `victim` holds a guard facing the thrower: the throw is turned
    /// away (a parry also staggers the thrower), the flight ends and
    /// [`SpearEvent::Deflected`] is emitted. The spear and the noose share it.
    fn guard_turns(&mut self, slot: usize, victim: usize, env: &mut Env<'_>) -> bool {
        let combat = env.fight.fighters[victim];
        let thrower = pos_mm(env.states[slot].pos);
        let body = pos_mm(env.states[victim].pos);
        let (sin, cos) = facing_q16(env.inputs[victim].facing);
        let to_thrower = (
            i64::from(thrower[0]) - i64::from(body[0]),
            i64::from(thrower[2]) - i64::from(body[2]),
        );
        let facing_thrower = i64::from(sin) * to_thrower.0 + i64::from(cos) * to_thrower.1 > 0;
        if !(combat.is_blocking && facing_thrower) {
            return false;
        }
        let parried = combat.parry_open_with(env.tables);
        if parried {
            stagger(env, slot);
        }
        self.slots[slot].phase = Phase::Idle;
        self.events.push(SpearEvent::Deflected {
            thrower: slot_id(slot),
            victim: slot_id(victim),
            parried,
        });
        true
    }

    /// The spear met geometry at `at`: the thrower zips to it.
    fn anchor(&mut self, slot: usize, at: [i32; 3], f: Flight) {
        let goal_mm = [0, 1].map(|lane| {
            let lane_at = if lane == 0 { at[0] } else { at[2] };
            lane_at - i32::try_from(i64::from(f.dir[lane]) * ANCHOR_BACKOFF / 65_536).unwrap_or(0)
        });
        self.slots[slot].phase = Phase::Tether(Tether {
            victim: None,
            goal_mm,
            anchor_mm: [at[0], at[2]],
            anchor_y_mm: at[1],
            rope_mm: 0,
            dir: f.dir,
            age: 0,
            latched: false,
            yank: false,
            noose: false,
            speed: 0,
        });
        self.events.push(SpearEvent::Hit {
            thrower: slot_id(slot),
            victim: None,
            at_mm: at,
        });
    }

    /// Why `slot`'s tether ends this tick, if it does: a fighter down, the
    /// timeout, a blade through the rope, then the victim's guard.
    fn tether_ends(&self, slot: usize, t: &Tether, env: &mut Env<'_>) -> Option<EndWhy> {
        let combat = t.victim.map(|v| env.fight.fighters[usize::from(v)]);
        let down =
            env.fight.fighters[slot].hp_permille == 0 || combat.is_some_and(|c| c.hp_permille == 0);
        if down {
            Some(EndWhy::Down)
        } else if t.age >= noose::tether_max(t.noose) {
            Some(EndWhy::Timeout)
        } else if rope_cut(self.fighters, slot, t, env) {
            Some(EndWhy::Cut)
        } else if t.yank || t.noose {
            // A yanked victim is stunned: no guard is up to cut the rope. A
            // snared man may guard all he likes; the cord stays on him.
            None
        } else if combat.is_some_and(|c| c.parry_open_with(env.tables)) {
            stagger(env, slot);
            Some(EndWhy::Parry)
        } else if combat.is_some_and(|c| c.is_blocking) {
            Some(EndWhy::Guard)
        } else {
            None
        }
    }

    /// Tether cut checks, then the velocity update of both bodies.
    fn pull(&mut self, slot: usize, env: &mut Env<'_>) {
        let Phase::Tether(mut t) = self.slots[slot].phase else {
            return;
        };
        let victim = t.victim.map(usize::from);
        if let Some(why) = self.tether_ends(slot, &t, env) {
            self.end(slot, why, 0, env);
            return;
        }
        if t.noose {
            self.snare(slot, t, env);
            return;
        }
        let me = xz(env.states[slot].pos);
        let mut mine = self.slots[slot].drift_mm;
        if let (true, Some(v)) = (t.yank, victim) {
            // Row SG9: the thrower plants, the victim snaps in at full speed
            // and eases into the stop in front of the blade.
            let you = xz(env.states[v].pos);
            let left = isqrt(sq(me[0] - you[0]) + sq(me[1] - you[1])) - YANK_STOP;
            let speed = (left / YANK_EASE_DIV)
                .clamp(YANK_MIN_SPEED, YANK_SPEED)
                .min(left.max(0));
            let toward = unit(you, me);
            let mut theirs = [0, 0];
            push(&mut theirs, toward, speed);
            stun(env, v, YANK_STUN);
            mine = [0, 0];
            self.slots[v].drift_mm = theirs;
            t.latched = false;
            t.age += 1;
            t.speed = u16::try_from(speed).unwrap_or(u16::MAX);
            if t.age % PULL_CUE_PERIOD == 1 {
                self.events.push(SpearEvent::TetherPull {
                    thrower: slot_id(slot),
                    victim: t.victim,
                    speed: t.speed,
                });
            }
            self.slots[slot].drift_mm = mine;
            self.slots[slot].phase = Phase::Tether(t);
            return;
        }
        if let Some(v) = victim {
            let you = xz(env.states[v].pos);
            let mut theirs = self.slots[v].drift_mm;
            t.latched = sq(you[0] - me[0]) + sq(you[1] - me[1]) <= REST * REST;
            if t.latched {
                let shared = [0, 1].map(|lane| (mine[lane] + theirs[lane]) / 2);
                mine = shared;
                theirs = shared;
            } else {
                push(&mut mine, unit(me, you), REEL_ACCEL);
            }
            push(&mut mine, unit(me, t.goal_mm), CARRY_ACCEL);
            push(&mut theirs, unit(you, t.goal_mm), CARRY_ACCEL);
            cap(&mut mine, MAX_SPEED);
            cap(&mut theirs, MAX_SPEED);
            self.slots[v].drift_mm = theirs;
        } else if env.rise.is_empty() {
            // No height channel: the wall tether is the winch it always was.
            push(&mut mine, unit(me, t.goal_mm), WALL_REEL_ACCEL);
            cap(&mut mine, WALL_MAX_SPEED);
        } else {
            // Row SG4: with the height channel armed the wall anchor is a
            // rope, not a winch (see `swing`). A rope that crosses a corner
            // snags on it first, so the arc turns round the corner.
            self.wrap(slot, &mut t, env);
            self.swing(slot, &mut t, &mut mine, env);
        }
        self.slots[slot].drift_mm = mine;
        t.age += 1;
        t.speed = u16::try_from(isqrt(sq(mine[0]) + sq(mine[1]))).unwrap_or(u16::MAX);
        if t.age % PULL_CUE_PERIOD == 1 {
            self.events.push(SpearEvent::TetherPull {
                thrower: slot_id(slot),
                victim: t.victim,
                speed: t.speed,
            });
        }
        self.slots[slot].phase = Phase::Tether(t);
    }

    /// Move every body with momentum by its drift through the same wall,
    /// obstacle and collision resolution walking uses, then mark the bodies
    /// whose move came up short (they stop dead).
    fn drift(&mut self, env: &mut Env<'_>) {
        let mut intended = [[0i32; 2]; MAX_SLOTS];
        let mut moved = false;
        for (slot, goal) in intended.iter_mut().enumerate().take(self.fighters) {
            let drift = self.slots[slot].drift_mm;
            let body = pos_mm(env.states[slot].pos);
            *goal = [body[0] + drift[0], body[2] + drift[1]];
            if drift != [0, 0] {
                move_body(env, slot, drift);
                moved = true;
            }
        }
        if moved && env.rules.collides() {
            collide_states(env.states, env.rules, env.half_extent_m, self.fighters);
        }
        for (slot, goal) in intended.iter().enumerate().take(self.fighters) {
            let drift = self.slots[slot].drift_mm;
            let speed = isqrt(sq(drift[0]) + sq(drift[1]));
            let body = pos_mm(env.states[slot].pos);
            let short = sq(goal[0] - body[0]) + sq(goal[1] - body[2]);
            let blocked = speed >= MIN_SLAM_SPEED && short * 4 > speed * speed;
            // Pinned: the body barely moved at all (under a quarter of its
            // speed), not merely slid along what it met.
            let start = [goal[0] - drift[0], goal[1] - drift[1]];
            let went = sq(body[0] - start[0]) + sq(body[2] - start[1]);
            let pinned = blocked && went * 16 < speed * speed;
            self.shocks[slot] = (blocked, speed, pinned);
            if blocked {
                self.slots[slot].drift_mm = [0, 0];
            }
        }
    }

    /// A wall tether with the height channel armed: the anchor is a rope. The
    /// reel takes rope in, the sphere constraint (in `settle`, after the
    /// bodies moved) turns that take-in into an arc, and a release carries
    /// the arc out. Here the drift keeps only its tangential part, because a
    /// taut rope has no radial speed.
    fn swing(&mut self, slot: usize, t: &mut Tether, mine: &mut [i32; 2], env: &mut Env<'_>) {
        let body = pos_mm(env.states[slot].pos);
        let anchor = self.pivot(slot, t);
        let off = [
            body[0].wrapping_sub(anchor[0]),
            body[1].wrapping_sub(anchor[1]),
            body[2].wrapping_sub(anchor[2]),
        ];
        let span = isqrt(sq(off[0]) + sq(off[1]) + sq(off[2]));
        if span <= 0 {
            return;
        }
        if t.rope_mm == 0 {
            t.rope_mm = i32::try_from(span).unwrap_or(i32::MAX).max(MIN_ROPE_MM);
        }
        t.rope_mm = (t.rope_mm - SWING_REEL).max(MIN_ROPE_MM);
        // The reel keeps the old pull toward the goal; the rope decides where
        // the body can go with it.
        push(mine, unit([body[0], body[2]], t.goal_mm), WALL_REEL_ACCEL);
        *mine = damp(*mine);
        cap(mine, WALL_MAX_SPEED);
        // Drop the radial part of (drift, rise): what is left is the arc.
        let unit3 = [
            i64::from(off[0]) * 65_536 / span,
            i64::from(off[1]) * 65_536 / span,
            i64::from(off[2]) * 65_536 / span,
        ];
        let rise = i64::from(env.rise[slot]);
        let radial =
            (i64::from(mine[0]) * unit3[0] + rise * unit3[1] + i64::from(mine[1]) * unit3[2])
                / 65_536;
        if radial != 0 {
            mine[0] -= i32::try_from(unit3[0] * radial / 65_536).unwrap_or(0);
            mine[1] -= i32::try_from(unit3[2] * radial / 65_536).unwrap_or(0);
            env.rise[slot] -= i32::try_from(unit3[1] * radial / 65_536).unwrap_or(0);
        }
    }

    /// Put a swung body back on its rope after the move: the take-in lifts it
    /// along the arc instead of dragging it straight at the wall. True when
    /// the arc reaches the floor, which ends the tether.
    fn project(&mut self, slot: usize, t: &Tether, env: &mut Env<'_>) -> bool {
        let body = pos_mm(env.states[slot].pos);
        let anchor = self.pivot(slot, t);
        let off = [
            body[0].wrapping_sub(anchor[0]),
            body[1].wrapping_sub(anchor[1]),
            body[2].wrapping_sub(anchor[2]),
        ];
        let span = isqrt(sq(off[0]) + sq(off[1]) + sq(off[2]));
        if span <= 0 {
            return false;
        }
        let rope = i64::from(t.rope_mm.max(MIN_ROPE_MM));
        let at = [0, 1, 2].map(|lane| {
            anchor[lane] + i32::try_from(i64::from(off[lane]) * rope / span).unwrap_or(off[lane])
        });
        let floor = at[1] <= 0;
        env.states[slot].pos = [metres(at[0]), metres(at[1].max(0)), metres(at[2])];
        if floor {
            env.rise[slot] = 0;
        }
        floor
    }

    /// The point the rope turns on: the wrap pivot while the rope is snagged
    /// on a corner, else its anchor.
    fn pivot(&self, slot: usize, t: &Tether) -> [i32; 3] {
        let wrap = self.slots[slot].wrap_mm;
        if wrap == [0, 0, 0] {
            [t.anchor_mm[0], t.anchor_y_mm, t.anchor_mm[1]]
        } else {
            wrap
        }
    }

    /// Snag the rope on the first corner between its pivot and the body: the
    /// rope then bends there and the arc turns around the corner instead of
    /// cutting through the wall. The pivot moves onto the corner and the rope
    /// takes the length it has left, so the wrap bends the swing without a
    /// jump; it never unwraps, and the tether is short anyway.
    fn wrap(&mut self, slot: usize, t: &mut Tether, env: &mut Env<'_>) {
        let body = pos_mm(env.states[slot].pos);
        let from = self.pivot(slot, t);
        let (clear, blocked) = snag_point(env.world, env.rules, env.half_extent_m, from, body);
        if !blocked {
            return;
        }
        let left = isqrt(sq(body[0] - clear[0]) + sq(body[1] - clear[1]) + sq(body[2] - clear[2]));
        self.slots[slot].wrap_mm = clear;
        t.rope_mm = i32::try_from(left).unwrap_or(i32::MAX).max(MIN_ROPE_MM);
    }

    /// Arrival and wall endings after the bodies moved.
    fn settle(&mut self, slot: usize, env: &mut Env<'_>) {
        let Phase::Tether(t) = self.slots[slot].phase else {
            return;
        };
        if t.noose {
            // A snare ends in `pull` (timeout, cut, release) or on a push.
            return;
        }
        let (thrower_blocked, _, _) = self.shocks[slot];
        let Some(v) = t.victim.map(usize::from) else {
            if t.rope_mm > 0 {
                // A swing rides the rope until the arc meets the floor; the
                // rope, not the wall, decides where the body goes.
                if self.project(slot, &t, env) {
                    self.end(slot, EndWhy::Arrived, 0, env);
                }
            } else if thrower_blocked {
                self.end(slot, EndWhy::Wall, 0, env);
            } else if arrived(xz(env.states[slot].pos), t) {
                self.end(slot, EndWhy::Arrived, 0, env);
            }
            return;
        };
        let (victim_blocked, victim_speed, victim_pinned) = self.shocks[v];
        let you = xz(env.states[v].pos);
        // A yanked body that slides along a wall or round a corner keeps
        // coming; only one pinned against it is slammed.
        let slammed = if t.yank {
            victim_pinned
        } else {
            victim_blocked
        };
        if slammed {
            let damage = (SLAM_DAMAGE + victim_speed / 6).min(MAX_TETHER_DAMAGE);
            self.end(slot, EndWhy::Wall, damage, env);
        } else if t.yank {
            // The yank lands when the gap is inside the stop; the thrower
            // stands, so a thrower short move is not a wall here.
            let me = xz(env.states[slot].pos);
            let gap = isqrt(sq(me[0] - you[0]) + sq(me[1] - you[1]));
            if gap <= YANK_STOP + YANK_MIN_SPEED {
                self.end(slot, EndWhy::Arrived, YANK_ARRIVE_DAMAGE, env);
            }
        } else if thrower_blocked {
            self.end(slot, EndWhy::Wall, 0, env);
        } else if t.latched && arrived(you, t) {
            let damage = (ARRIVE_DAMAGE + victim_speed / 8).min(MAX_TETHER_DAMAGE);
            self.end(slot, EndWhy::Arrived, damage, env);
        }
    }

    /// End `slot`'s tether: damage the victim, restart the cooldown, emit.
    fn end(&mut self, slot: usize, why: EndWhy, damage: i64, env: &mut Env<'_>) {
        let Phase::Tether(t) = self.slots[slot].phase else {
            return;
        };
        if let Some(v) = t.victim {
            hurt(env, usize::from(v), damage);
            if t.yank || t.noose {
                // A yank stops dead in front of the blade, no slide past it;
                // a freed man has no cord left to be dragged by.
                self.slots[usize::from(v)].drift_mm = [0, 0];
            }
        }
        self.slots[slot].phase = Phase::Idle;
        self.slots[slot].cooldown = TETHER_COOLDOWN;
        self.events.push(SpearEvent::TetherEnd {
            thrower: slot_id(slot),
            victim: t.victim,
            why,
            damage_permille: u16::try_from(damage).unwrap_or(u16::MAX),
        });
    }
}

/// Slot index as the wire id (slots are below 16, so the fallback never fires).
fn slot_id(slot: usize) -> u8 {
    u8::try_from(slot).unwrap_or(u8::MAX)
}

/// Q16 metres from integer millimetres.
pub(crate) fn fx(mm: i32) -> Fixed {
    Fixed::from_bits(i64::from(mm) * Fixed::SCALE / 1000)
}

/// Square of a millimetre lane.
pub(crate) fn sq(lane: i32) -> i64 {
    i64::from(lane) * i64::from(lane)
}

/// Integer square root of a non-negative value.
pub(crate) fn isqrt(value: i64) -> i64 {
    i64::try_from(u64::try_from(value).unwrap_or(0).isqrt()).unwrap_or(0)
}

/// The spawn point and the muzzle velocity of a thrown weapon: ahead of
/// the thrower along the Q16 facing, `height_mm` over the floor, at
/// `speed_mm` per tick. The spear and the pilum share it, so both leave
/// the hand at the same place.
pub(crate) fn throw_start(
    body_mm: [i32; 3],
    dir: [i32; 2],
    ahead_mm: i32,
    height_mm: i32,
    speed_mm: i32,
) -> ([i32; 3], [i32; 3]) {
    let ahead = |lane: i32| lane.saturating_mul(ahead_mm) / 65_536;
    let at = [
        body_mm[0].saturating_add(ahead(dir[0])),
        height_mm,
        body_mm[2].saturating_add(ahead(dir[1])),
    ];
    let muzzle = |lane: i32| lane.saturating_mul(speed_mm) / 65_536;
    let vel = [muzzle(dir[0]), 0, muzzle(dir[1])];
    (at, vel)
}

/// The hurt capsule of one fighter: the axis from the knees to the head
/// with the body radius around it.
pub(crate) fn hurt_capsule(id: u32, body_mm: [i32; 3]) -> CastCapsule {
    CastCapsule {
        id,
        a: [fx(body_mm[0]), fx(CAPSULE_BOTTOM), fx(body_mm[2])],
        b: [fx(body_mm[0]), fx(CAPSULE_TOP), fx(body_mm[2])],
        radius: fx(CAPSULE_RADIUS),
    }
}

/// The first capsule the segment `from..to` meets, with the contact
/// point in millimetres. `None` when the sweep meets nothing.
pub(crate) fn cast_contact(
    hits: &mut Vec<SweptHit>,
    from: [i32; 3],
    to: [i32; 3],
    caps: &[CastCapsule],
) -> Option<(u32, [i32; 3])> {
    let tip = [fx(from[0]), fx(from[1]), fx(from[2])];
    let end = [fx(to[0]), fx(to[1]), fx(to[2])];
    swept_cast(tip, end, fx(TIP_RADIUS), caps, hits).ok()?;
    let hit = hits.first()?;
    let t = i64::from(u32::try_from(hit.toi.to_bits()).unwrap_or(0).min(65_536));
    let at = [0, 1, 2].map(|lane| {
        let span = i64::from(to[lane]) - i64::from(from[lane]);
        from[lane] + i32::try_from(span * t / 65_536).unwrap_or(0)
    });
    Some((hit.id, at))
}

/// Take `damage` permille off a fighter, flooring at zero.
pub(crate) fn hurt_fighter(fight: &mut FightState, victim: usize, damage: u16) {
    let hp = &mut fight.fighters[victim].hp_permille;
    *hp = hp.saturating_sub(damage);
}

/// True when a blade in its swing crosses the rope of the tether `t` thrown by
/// `thrower` (row SG13). The rope runs from the thrower to the victim, or to
/// the world anchor; the blade is the arena's mapped sweep (`sweep_for_attacker`)
/// of every other fighter whose strike has just landed: the impact tick and
/// the few after it, while the steel is still moving through the arc. A
/// fighter stunned or down has no swing (the fight cancels it). Planar,
/// integer, slot order does not matter (the answer is a plain any).
fn rope_cut(fighters: usize, thrower: usize, t: &Tether, env: &Env<'_>) -> bool {
    let from = xz(env.states[thrower].pos);
    let to = match t.victim {
        Some(v) => xz(env.states[usize::from(v)].pos),
        None => t.anchor_mm,
    };
    (0..fighters).filter(|&s| s != thrower).any(|s| {
        let f = env.fight.fighters[s];
        let impact = engine_combat::heft::startup(f.heft, env.tables);
        let live = f.is_attacking
            && f.hp_permille > 0
            && f.react == 0
            && f.attack_age >= impact
            && f.attack_age < impact + CUT_LIVE_TICKS;
        if !live {
            return false;
        }
        let input = env.inputs[s];
        let facing = facing_vector(input.mx, input.mz, input.facing);
        let blade = sweep_for_attacker(slot_id(s), pos_mm(env.states[s].pos), facing);
        let reach = i64::from(blade.blade_half_mm) + ROPE_HALF_MM;
        segment_gap_sq(
            [blade.p0_mm[0], blade.p0_mm[2]],
            [blade.p1_mm[0], blade.p1_mm[2]],
            from,
            to,
        ) <= reach * reach
    })
}

/// Squared planar distance between the segments `a..b` and `c..d`, mm squared:
/// zero when they cross, else the nearest endpoint-to-segment gap.
fn segment_gap_sq(a: [i32; 2], b: [i32; 2], c: [i32; 2], d: [i32; 2]) -> i64 {
    let side = |o: [i32; 2], p: [i32; 2], q: [i32; 2]| {
        (i64::from(p[0]) - i64::from(o[0])) * (i64::from(q[1]) - i64::from(o[1]))
            - (i64::from(p[1]) - i64::from(o[1])) * (i64::from(q[0]) - i64::from(o[0]))
    };
    let (d1, d2, d3, d4) = (side(c, d, a), side(c, d, b), side(a, b, c), side(a, b, d));
    if d1.signum() * d2.signum() < 0 && d3.signum() * d4.signum() < 0 {
        return 0;
    }
    point_gap_sq(a, c, d)
        .min(point_gap_sq(b, c, d))
        .min(point_gap_sq(c, a, b))
        .min(point_gap_sq(d, a, b))
}

/// Squared distance from the point `p` to the segment `a..b`, mm squared.
fn point_gap_sq(p: [i32; 2], a: [i32; 2], b: [i32; 2]) -> i64 {
    let ab = [0, 1].map(|lane| i64::from(b[lane]) - i64::from(a[lane]));
    let ap = [0, 1].map(|lane| i64::from(p[lane]) - i64::from(a[lane]));
    let len_sq = ab[0] * ab[0] + ab[1] * ab[1];
    let along = if len_sq == 0 {
        0
    } else {
        (ap[0] * ab[0] + ap[1] * ab[1]).clamp(0, len_sq)
    };
    let foot = [0, 1].map(|lane| {
        if len_sq == 0 {
            0
        } else {
            ab[lane] * along / len_sq
        }
    });
    let gap = [ap[0] - foot[0], ap[1] - foot[1]];
    gap[0] * gap[0] + gap[1] * gap[1]
}

/// Planar millimetre position of a pose.
fn xz(pos: [f32; 3]) -> [i32; 2] {
    let mm = pos_mm(pos);
    [mm[0], mm[2]]
}

/// Q16 unit vector from `from` toward `to` (zero when they coincide).
fn unit(from: [i32; 2], to: [i32; 2]) -> [i64; 2] {
    let (dx, dz) = (
        i64::from(to[0]) - i64::from(from[0]),
        i64::from(to[1]) - i64::from(from[1]),
    );
    let len = isqrt(dx * dx + dz * dz);
    if len == 0 {
        [0, 0]
    } else {
        [dx * 65_536 / len, dz * 65_536 / len]
    }
}

/// Add `accel` mm per tick squared along the Q16 unit vector.
fn push(v: &mut [i32; 2], toward: [i64; 2], accel: i64) {
    for lane in 0..2 {
        v[lane] += i32::try_from(toward[lane] * accel / 65_536).unwrap_or(0);
    }
}

/// Clamp a velocity to `max` mm per tick without changing its direction.
fn cap(v: &mut [i32; 2], max: i64) {
    let speed = isqrt(sq(v[0]) + sq(v[1]));
    if speed > max {
        *v = v.map(|lane| i32::try_from(i64::from(lane) * max / speed).unwrap_or(0));
    }
}

/// One tick of swing damping: a [`SWING_DAMP_DIV`]th of each lane's speed, at
/// least one step, so a slow swing still comes to rest (a truncated quotient
/// alone stalls every lane under `SWING_DAMP_DIV` mm/tick).
fn damp(v: [i32; 2]) -> [i32; 2] {
    v.map(|lane| {
        let bite = lane / SWING_DAMP_DIV;
        lane - if bite == 0 { lane.signum() } else { bite }
    })
}

/// One tick of drift friction: a sixteenth of the speed, at least one step.
fn friction(v: i32) -> i32 {
    let bite = v / 16;
    v - if bite == 0 { v.signum() } else { bite }
}

/// True when the lead body is within [`ARRIVE`] of the goal or past it.
fn arrived(at: [i32; 2], t: Tether) -> bool {
    let (dx, dz) = (
        i64::from(t.goal_mm[0]) - i64::from(at[0]),
        i64::from(t.goal_mm[1]) - i64::from(at[1]),
    );
    dx * dx + dz * dz <= ARRIVE * ARRIVE || dx * i64::from(t.dir[0]) + dz * i64::from(t.dir[1]) <= 0
}

/// March `from..full` in [`MARCH_STEP`] steps; the first point inside
/// geometry (arena wall, rule pillar, obstacle) ends the segment there.
/// The spear and the pilum share it: a thrown weapon stops at the wall
/// the same way.
pub(crate) fn march_segment(
    world: &MatchWorld,
    rules: &ArenaRules,
    half_extent_m: f32,
    from: [i32; 3],
    full: [i32; 3],
) -> ([i32; 3], bool) {
    let span = [0, 1, 2].map(|lane| full[lane] - from[lane]);
    let len = isqrt(sq(span[0]) + sq(span[1]) + sq(span[2]));
    let steps = (len / i64::from(MARCH_STEP) + 1).clamp(1, 16);
    for k in 1..=steps {
        let point = [0, 1, 2]
            .map(|lane| from[lane] + i32::try_from(i64::from(span[lane]) * k / steps).unwrap_or(0));
        if in_geometry(world, rules, half_extent_m, point) {
            return (point, true);
        }
    }
    (full, false)
}

/// The last clear point on the line `from..to` before geometry, mm, and true
/// when geometry blocks it. The rope march looks the whole rope long, unlike
/// the weapon march, because a long rope snags far from the hand.
fn snag_point(
    world: &MatchWorld,
    rules: &ArenaRules,
    half_extent_m: f32,
    from: [i32; 3],
    to: [i32; 3],
) -> ([i32; 3], bool) {
    let span = [0, 1, 2].map(|lane| to[lane] - from[lane]);
    let len = isqrt(sq(span[0]) + sq(span[1]) + sq(span[2]));
    let steps = (len / i64::from(MARCH_STEP)).clamp(1, WRAP_STEPS);
    let mut clear = from;
    for k in 1..=steps {
        let point = [0, 1, 2]
            .map(|lane| from[lane] + i32::try_from(i64::from(span[lane]) * k / steps).unwrap_or(0));
        if in_geometry(world, rules, half_extent_m, point) {
            return (clear, true);
        }
        clear = point;
    }
    (to, false)
}

/// True when a weapon tip at `p` is in the arena wall, a pillar or an obstacle.
fn in_geometry(world: &MatchWorld, rules: &ArenaRules, half_extent_m: f32, p: [i32; 3]) -> bool {
    let face = pos_mm([half_extent_m, 0.0, 0.0])[0];
    if p[0].abs() >= face || p[2].abs() >= face {
        return true;
    }
    let in_pillar = rules.pillars.iter().flatten().any(|pillar| {
        (pillar.min_mm[0]..=pillar.max_mm[0]).contains(&p[0])
            && (pillar.min_mm[1]..=pillar.max_mm[1]).contains(&p[2])
    });
    in_pillar || (!world.is_empty() && world.overlaps([metres(p[0]), metres(p[2])], TIP_RADIUS_M))
}

/// Add an integer displacement to a pose and resolve it like walking does:
/// the square wall clamp, then the obstacle push-out.
fn move_body(env: &mut Env<'_>, slot: usize, drift: [i32; 2]) {
    let limit = wall_limit(env.half_extent_m);
    let state = &mut env.states[slot];
    let from = [state.pos[0], state.pos[2]];
    let raw = [from[0] + metres(drift[0]), from[1] + metres(drift[1])];
    let to = [raw[0].clamp(-limit, limit), raw[1].clamp(-limit, limit)];
    let placed = if env.world.is_empty() {
        to
    } else {
        env.world.resolve(from, to, FIGHTER_RADIUS_M, limit)
    };
    state.pos[0] = canon_zero(placed[0]);
    state.pos[2] = canon_zero(placed[1]);
}

/// Take `damage` permille off a fighter, flooring at zero.
fn hurt(env: &mut Env<'_>, victim: usize, damage: i64) {
    let amount = u16::try_from(damage).unwrap_or(u16::MAX);
    hurt_fighter(env.fight, victim, amount);
}

/// True when the thrower's move wish points against the flight `dir` (Q16):
/// holding back as the spear sticks asks for a yank, not a carry.
fn holds_back(input: FighterInput, dir: [i32; 2]) -> bool {
    i64::from(input.mx) * i64::from(dir[0]) + i64::from(input.mz) * i64::from(dir[1]) < 0
}

/// Hold `slot` in hit react for at least `ticks` (a yanked victim).
fn stun(env: &mut Env<'_>, slot: usize, ticks: u32) {
    let react = &mut env.fight.fighters[slot].react;
    *react = (*react).max(ticks);
}

/// Parry stagger on a thrower whose spear or tether was parried.
fn stagger(env: &mut Env<'_>, slot: usize) {
    let react = &mut env.fight.fighters[slot].react;
    *react = (*react).max(env.tables.parry_stagger_ticks);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::Obstacle;
    use crate::{ArenaConfig, Sim};
    use engine_core::laws;
    use engine_input::fighter::{BLOCK, KNOWN_BUTTONS};
    use engine_input::{Buttons, FighterId, FighterTape};

    fn config() -> ArenaConfig {
        ArenaConfig {
            seed: 7,
            ticks: 600,
            fighters: 2,
            half_extent_m: 12.0,
            table_version: 2,
        }
    }

    fn sim_in(world: MatchWorld) -> Sim {
        let mut sim = Sim::build_world(&config(), &FighterTape::default(), false, false, world)
            .expect("sim builds");
        assert!(sim.set_spear());
        sim
    }

    /// A spear match with the height channel armed (rows SG1 + SG5).
    fn sim_swing(world: MatchWorld) -> Sim {
        let mut sim = sim_in(world);
        assert!(sim.set_height());
        sim
    }

    /// One tick with scripted buttons, facing and movement wishes.
    fn tick_move(sim: &mut Sim, script: [(u8, i16, i8, i8); 2]) {
        let at = sim.tick;
        let tape = FighterTape(
            script
                .iter()
                .enumerate()
                .map(|(slot, &(buttons, facing, mx, mz))| FighterInput {
                    tick: at,
                    fighter: FighterId(slot_id(slot)),
                    mx,
                    mz,
                    facing,
                    buttons: Buttons(buttons),
                })
                .collect(),
        );
        sim.cursor = 0;
        sim.step(&tape).expect("step");
    }

    /// Run `n` moving ticks, collecting every event.
    fn idle_move(sim: &mut Sim, n: u32, script: [(u8, i16, i8, i8); 2]) -> Vec<SpearEvent> {
        let mut seen = Vec::new();
        for _ in 0..n {
            tick_move(sim, script);
            seen.extend(events(sim));
        }
        seen
    }

    fn place(sim: &mut Sim, thrower: [f32; 2], victim: [f32; 2]) {
        sim.states[0].pos = [thrower[0], 0.0, thrower[1]];
        sim.states[1].pos = [victim[0], 0.0, victim[1]];
    }

    /// One tick with scripted `(buttons, facing)` for both fighters.
    fn tick(sim: &mut Sim, script: [(u8, i16); 2]) {
        let at = sim.tick;
        let tape = FighterTape(
            script
                .iter()
                .enumerate()
                .map(|(slot, &(buttons, facing))| FighterInput {
                    tick: at,
                    fighter: FighterId(slot_id(slot)),
                    mx: 0,
                    mz: 0,
                    facing,
                    buttons: Buttons(buttons),
                })
                .collect(),
        );
        sim.cursor = 0;
        sim.step(&tape).expect("step");
    }

    fn events(sim: &Sim) -> Vec<SpearEvent> {
        sim.spear.as_ref().expect("spear").events().to_vec()
    }

    fn slot(sim: &Sim, i: usize) -> SpearSlot {
        sim.spear.as_ref().expect("spear").slots()[i]
    }

    /// Run `n` idle ticks, collecting every event.
    fn idle(sim: &mut Sim, n: u32, script: [(u8, i16); 2]) -> Vec<SpearEvent> {
        let mut seen = Vec::new();
        for _ in 0..n {
            tick(sim, script);
            seen.extend(events(sim));
        }
        seen
    }

    const THROW: u8 = THROW_SPEAR;

    #[test]
    fn spear_hits_enemy_and_drags_both_toward_the_goal() {
        let mut sim = sim_in(MatchWorld::empty());
        place(&mut sim, [0.0, -4.0], [0.0, 2.0]);
        tick(&mut sim, [(THROW, 0), (0, 0)]);
        let mut seen = events(&sim);
        seen.extend(idle(&mut sim, 80, [(0, 0), (0, 0)]));
        assert!(seen
            .iter()
            .any(|e| matches!(e, SpearEvent::Thrown { thrower: 0, .. })));
        assert!(seen.iter().any(|e| matches!(
            e,
            SpearEvent::Hit {
                thrower: 0,
                victim: Some(1),
                ..
            }
        )));
        let end = seen
            .iter()
            .find_map(|e| match e {
                SpearEvent::TetherEnd {
                    why,
                    damage_permille,
                    ..
                } => Some((*why, *damage_permille)),
                _ => None,
            })
            .expect("the tether ends");
        assert_eq!(end.0, EndWhy::Arrived);
        assert!(end.1 > 40, "momentum adds to the base damage: {end:?}");
        let (thrower, victim) = (sim.states[0].pos[2], sim.states[1].pos[2]);
        assert!(thrower > -1.0, "the thrower was carried forward: {thrower}");
        assert!(
            victim > 4.0,
            "the victim was dragged toward the goal: {victim}"
        );
        assert!(sim.fight.fighters[1].hp_permille < 1000 - 30);
        assert!(
            sim.states[1].pos[0].abs() < 1.0e-3,
            "straight along the throw"
        );
    }

    #[test]
    fn spear_hits_wall_and_zips_the_thrower() {
        let world =
            MatchWorld::try_new(vec![Obstacle::aligned([0.0, 4.0], [3.0, 0.5])]).expect("world");
        let mut sim = sim_in(world);
        place(&mut sim, [0.0, -2.0], [8.0, -8.0]);
        tick(&mut sim, [(THROW, 0), (0, 0)]);
        let mut seen = events(&sim);
        seen.extend(idle(&mut sim, 60, [(0, 0), (0, 0)]));
        assert!(seen.iter().any(|e| matches!(
            e,
            SpearEvent::Hit {
                thrower: 0,
                victim: None,
                ..
            }
        )));
        assert!(seen.iter().any(|e| matches!(
            e,
            SpearEvent::TetherEnd {
                victim: None,
                damage_permille: 0,
                ..
            }
        )));
        let z = sim.states[0].pos[2];
        assert!(z > 2.0 && z < 3.2, "zipped up to the wall face: {z}");
        assert!(!sim
            .world
            .overlaps([sim.states[0].pos[0], z], FIGHTER_RADIUS_M));
        assert_eq!(
            sim.fight.fighters[1].hp_permille, 1000,
            "nobody else is hurt"
        );
        assert_eq!(sim.states[1].pos, [8.0, 0.0, -8.0]);
    }

    #[test]
    fn a_world_aimed_throw_passes_the_enemy_and_anchors_the_wall() {
        // The aim: an enemy stands in the line, and the throw is still aimed
        // at the wall behind them, so the duel can reach a rope anchor.
        let world =
            MatchWorld::try_new(vec![Obstacle::aligned([0.0, 6.0], [3.0, 0.5])]).expect("world");
        let aimed = |buttons: u8| {
            let mut sim = sim_in(world.clone());
            place(&mut sim, [0.0, -2.0], [0.0, 1.5]);
            tick(&mut sim, [(buttons, 0), (0, 0)]);
            let mut seen = events(&sim);
            seen.extend(idle(&mut sim, 40, [(0, 0), (0, 0)]));
            (sim, seen)
        };
        // Plain throw: the enemy is in the way and takes the spear.
        let (_, plain) = aimed(THROW);
        assert!(plain.iter().any(|e| matches!(
            e,
            SpearEvent::Hit {
                thrower: 0,
                victim: Some(1),
                ..
            }
        )));
        // World aim: the spear goes past the enemy and sticks the wall.
        let (sim, over) = aimed(THROW | THROW_SPEAR_WORLD);
        assert!(over.iter().any(|e| matches!(
            e,
            SpearEvent::Hit {
                thrower: 0,
                victim: None,
                ..
            }
        )));
        assert!(
            !over.iter().any(|e| matches!(
                e,
                SpearEvent::Hit {
                    victim: Some(_),
                    ..
                }
            )),
            "a world throw never targets a fighter"
        );
        assert_eq!(
            sim.fight.fighters[1].hp_permille, 1000,
            "the enemy it flew past is unhurt"
        );
        assert!(
            sim.states[0].pos[2] > 2.0,
            "the thrower was carried past the enemy: {}",
            sim.states[0].pos[2]
        );
    }

    #[test]
    fn a_body_in_the_air_throws_again_without_waiting_for_the_cooldown() {
        // The chain: while the height channel carries a body, a second anchor
        // is one press away. On the floor the cooldown still paces the throw.
        let chain = |height: bool| {
            let mut sim = if height {
                sim_swing(MatchWorld::empty())
            } else {
                sim_in(MatchWorld::empty())
            };
            place(&mut sim, [0.0, -4.0], [0.0, 2.0]);
            // Off the floor and rising: what a rope swing leaves the body as.
            if height {
                sim.rise[0] = 500;
                sim.states[0].pos[1] = 1.2;
            }
            tick(&mut sim, [(THROW, 0), (0, 0)]);
            let mut seen = events(&sim);
            seen.extend(idle(&mut sim, 14, [(0, 0), (0, 0)]));
            assert!(matches!(slot(&sim, 0).phase, Phase::Tether(_)));
            // Release, then throw again inside the floor cooldown.
            tick(&mut sim, [(THROW, 0), (0, 0)]);
            seen.extend(events(&sim));
            if height {
                sim.rise[0] = 500;
            }
            tick(&mut sim, [(0, 0), (0, 0)]);
            tick(&mut sim, [(THROW, 0), (0, 0)]);
            seen.extend(events(&sim));
            seen.iter()
                .filter(|e| matches!(e, SpearEvent::Thrown { thrower: 0, .. }))
                .count()
        };
        assert_eq!(chain(true), 2, "the airborne body chains a second throw");
        assert_eq!(chain(false), 1, "on the floor the cooldown still holds");
    }

    #[test]
    fn a_swing_snags_the_rope_on_a_corner_and_turns_round_it() {
        // A pillar stands between the wall anchor and the arc. The rope must
        // bend on the corner instead of cutting through it, so the swing turns
        // round the pillar.
        let world = MatchWorld::try_new(vec![Obstacle::aligned([3.0, 3.0], [0.5, 0.5])]).expect("world");
        let mut sim = sim_swing(world);
        place(&mut sim, [0.0, 8.0], [-8.0, 8.0]);
        tick(&mut sim, [(THROW, 0), (0, 0)]);
        idle(&mut sim, 12, [(0, 0), (0, 0)]);
        assert!(matches!(slot(&sim, 0).phase, Phase::Tether(_)), "tethered");
        assert_eq!(
            slot(&sim, 0).wrap_mm,
            [0; 3],
            "a clear line to the anchor does not wrap"
        );
        // The body swings across the pillar: the line from the anchor (0, 12)
        // to (4, 0) runs through (3, 3), the pillar's centre.
        sim.states[0].pos = [4.0, 1.6, 0.0];
        tick(&mut sim, [(0, 0), (0, 0)]);
        let wrap = slot(&sim, 0).wrap_mm;
        assert_ne!(wrap, [0; 3], "the rope snagged the pillar");
        assert!(
            wrap[2] > 0 && wrap[2] < 6_000,
            "the corner sits between the anchor and the body: {wrap:?}"
        );
        let Phase::Tether(t) = slot(&sim, 0).phase else {
            panic!("the snag keeps the tether");
        };
        assert!(
            t.rope_mm < 5_000,
            "the rope took the length left round the corner: {}",
            t.rope_mm
        );
    }

    #[test]
    fn spear_miss_leaves_no_tether() {
        let mut sim = sim_in(MatchWorld::empty());
        place(&mut sim, [-2.0, 0.0], [-8.0, 8.0]);
        tick(&mut sim, [(THROW, 9_000), (0, 0)]);
        let mut seen = events(&sim);
        seen.extend(idle(&mut sim, 30, [(0, 9_000), (0, 0)]));
        assert!(seen.contains(&SpearEvent::Missed {
            thrower: 0,
            why: MissWhy::Range
        }));
        assert!(!seen.iter().any(|e| matches!(e, SpearEvent::Hit { .. })));
        assert_eq!(slot(&sim, 0).phase, Phase::Idle);
        assert_eq!(sim.states[0].pos, [-2.0, 0.0, 0.0]);
        assert_eq!(sim.fight.fighters[1].hp_permille, 1000);
    }

    #[test]
    fn a_second_throw_waits_for_the_spear_and_the_cooldown() {
        let mut sim = sim_in(MatchWorld::empty());
        place(&mut sim, [0.0, -4.0], [8.0, 8.0]);
        tick(&mut sim, [(THROW, 0), (0, 0)]);
        tick(&mut sim, [(0, 0), (0, 0)]);
        tick(&mut sim, [(THROW, 0), (0, 0)]);
        assert!(
            !events(&sim)
                .iter()
                .any(|e| matches!(e, SpearEvent::Thrown { .. })),
            "the spear is still out"
        );
        idle(&mut sim, 40, [(0, 0), (0, 0)]);
        tick(&mut sim, [(THROW, 0), (0, 0)]);
        assert!(
            !events(&sim)
                .iter()
                .any(|e| matches!(e, SpearEvent::Thrown { .. })),
            "cooldown {} still runs",
            slot(&sim, 0).cooldown
        );
    }

    /// Throw at slot 1 holding back until the hook sticks, then let go: the
    /// events from the throw on and the tick of the hit.
    fn yank_from(sim: &mut Sim, victim_buttons: u8) -> (Vec<SpearEvent>, Option<u32>) {
        let back = (0, 0, 0, -100);
        tick_move(sim, [(THROW, 0, 0, -100), (victim_buttons, 0, 0, 0)]);
        let mut seen = events(sim);
        let mut hit_tick = None;
        for _ in 0..90 {
            let hooked = seen.iter().any(|e| matches!(e, SpearEvent::Hit { .. }));
            if hooked && hit_tick.is_none() {
                hit_tick = Some(sim.tick);
                assert!(sim.fight.hitstop > 0, "the catch freezes the fight");
            }
            let me = if hooked { (0, 0, 0, 0) } else { back };
            tick_move(sim, [me, (victim_buttons, 0, 0, 0)]);
            seen.extend(events(sim));
        }
        (seen, hit_tick)
    }

    #[test]
    fn yank_reels_the_hooked_enemy_to_a_planted_thrower() {
        let mut sim = sim_in(MatchWorld::empty());
        place(&mut sim, [0.0, -4.0], [0.0, 4.0]);
        let mut landed = None;
        let back = (0, 0, 0, -100);
        tick_move(&mut sim, [(THROW, 0, 0, -100), (0, 0, 0, 0)]);
        let mut seen = events(&sim);
        for _ in 0..90 {
            let hooked = seen.iter().any(|e| matches!(e, SpearEvent::Hit { .. }));
            let me = if hooked { (0, 0, 0, 0) } else { back };
            tick_move(&mut sim, [me, (0, 0, 0, 0)]);
            let now = events(&sim);
            if landed.is_none() && now.iter().any(|e| matches!(e, SpearEvent::TetherEnd { .. })) {
                let z = [sim.states[0].pos[2], sim.states[1].pos[2]];
                landed = Some((sim.tick, sim.fight.fighters[1].react, z));
            }
            seen.extend(now);
        }
        let hit = seen
            .iter()
            .position(|e| matches!(e, SpearEvent::Hit { victim: Some(1), .. }))
            .expect("the hook sticks");
        let end = seen[hit..]
            .iter()
            .find_map(|e| match *e {
                SpearEvent::TetherEnd { why, damage_permille, .. } => Some((why, damage_permille)),
                _ => None,
            })
            .expect("the yank ends");
        assert_eq!(end, (EndWhy::Arrived, 20), "landed, not cut");
        let (tick, react, z) = landed.expect("a landing tick");
        assert!(tick < 55, "a snap, not a float: landed at tick {tick}");
        assert!(react > 10, "the victim is still stunned on landing: {react}");
        let gap = z[1] - z[0];
        assert!(gap > 0.9 && gap < 1.5, "in front of the blade: {gap}");
        assert!(z[1] < 0.0, "the victim came to the thrower: {z:?}");
        assert_eq!(slot(&sim, 1).drift_mm, [0, 0], "no slide past the blade");
    }

    #[test]
    fn a_held_guard_cannot_cut_a_yank() {
        // Back turned with the guard up: a carry would be cut (the test
        // above), a yank stuns the guard down and lands.
        let mut sim = sim_in(MatchWorld::empty());
        place(&mut sim, [0.0, -4.0], [0.0, 4.0]);
        idle(&mut sim, 12, [(0, 0), (BLOCK, 0)]);
        let (seen, hit) = yank_from(&mut sim, BLOCK);
        assert!(hit.is_some(), "the hook sticks in a turned back");
        let ends: Vec<EndWhy> = seen
            .iter()
            .filter_map(|e| match *e {
                SpearEvent::TetherEnd { why, .. } => Some(why),
                _ => None,
            })
            .collect();
        assert_eq!(ends.first(), Some(&EndWhy::Arrived), "{ends:?}");
    }

    #[test]
    fn a_yank_slides_round_a_corner_instead_of_dying_on_it() {
        // The window case: the spear line clears a box by 150 mm, the victim's
        // 400 mm body does not. The yank slides it past the corner and lands.
        let world = MatchWorld::try_new(vec![Obstacle::aligned([1.075, 0.5], [0.925, 0.5])])
            .expect("world");
        let mut sim = sim_in(world);
        place(&mut sim, [0.0, -4.0], [0.0, 4.0]);
        let (seen, hit) = yank_from(&mut sim, 0);
        assert!(hit.is_some(), "the spear passes the box and sticks");
        let end = seen.iter().find_map(|e| match *e {
            SpearEvent::TetherEnd { why, .. } => Some(why),
            _ => None,
        });
        assert_eq!(end, Some(EndWhy::Arrived), "slid round, not slammed");
        let z = sim.states[1].pos[2];
        assert!(z < -2.0, "the victim came past the box to the thrower: {z}");
    }

    #[test]
    fn no_back_hold_keeps_the_carry() {
        let mut sim = sim_in(MatchWorld::empty());
        place(&mut sim, [0.0, -4.0], [0.0, 2.0]);
        tick(&mut sim, [(THROW, 0), (0, 0)]);
        idle(&mut sim, 12, [(0, 0), (0, 0)]);
        let Phase::Tether(t) = slot(&sim, 0).phase else {
            panic!("tethered");
        };
        assert!(!t.yank);
        assert_eq!(sim.fight.fighters[1].react, 0, "a carry stuns nobody");
    }

    /// Three fighters, spear armed (row SG13: a third fighter's blade).
    fn sim3() -> Sim {
        let cfg = ArenaConfig {
            fighters: 3,
            ..config()
        };
        let mut sim = Sim::build_world(
            &cfg,
            &FighterTape::default(),
            false,
            false,
            MatchWorld::empty(),
        )
        .expect("sim builds");
        assert!(sim.set_spear());
        sim
    }

    /// One tick with scripted `(buttons, facing, mx, mz)` for every fighter.
    fn tick_all(sim: &mut Sim, script: &[(u8, i16, i8, i8)]) {
        let at = sim.tick;
        let tape = FighterTape(
            script
                .iter()
                .enumerate()
                .map(|(slot, &(buttons, facing, mx, mz))| FighterInput {
                    tick: at,
                    fighter: FighterId(slot_id(slot)),
                    mx,
                    mz,
                    facing,
                    buttons: Buttons(buttons),
                })
                .collect(),
        );
        sim.cursor = 0;
        sim.step(&tape).expect("step");
    }

    /// Put `slot` on the impact tick of a swing, as if it had pressed attack
    /// a full startup ago: the blade is in its arc on the next step.
    fn swing_now(sim: &mut Sim, slot: usize) {
        let f = &mut sim.fight.fighters[slot];
        f.is_attacking = true;
        f.attack_age = engine_combat::heft::startup(f.heft, &engine_combat::fight::TABLES_V2);
    }

    /// The first tether ending among `seen`.
    fn first_end(seen: &[SpearEvent]) -> Option<EndWhy> {
        seen.iter().find_map(|e| match *e {
            SpearEvent::TetherEnd { why, .. } => Some(why),
            _ => None,
        })
    }

    /// Carry run: thrower at z -4 hooks a victim at z -1, then the victim
    /// faces `face` and, when `swing`, swings at once. Returns the events
    /// from the throw on.
    fn carry_with_swing(face: i16, swing: bool) -> (Sim, Vec<SpearEvent>) {
        let mut sim = sim_in(MatchWorld::empty());
        place(&mut sim, [0.0, -4.0], [0.0, -1.0]);
        tick(&mut sim, [(THROW, 0), (0, face)]);
        let mut seen = events(&sim);
        while !seen.iter().any(|e| matches!(e, SpearEvent::Hit { .. })) {
            tick(&mut sim, [(0, 0), (0, face)]);
            seen.extend(events(&sim));
            assert!(sim.tick < 40, "the hook sticks");
        }
        if swing {
            swing_now(&mut sim, 1);
        }
        for _ in 0..80 {
            tick(&mut sim, [(0, 0), (0, face)]);
            seen.extend(events(&sim));
        }
        (sim, seen)
    }

    #[test]
    fn a_victims_swing_across_the_rope_cuts_a_carry() {
        // The victim turns on the thrower and cuts: the rope ends with Cut, no
        // arrival damage, the spear is dropped and the throw cooldown runs.
        let (sim, seen) = carry_with_swing(18_000, true);
        assert_eq!(first_end(&seen), Some(EndWhy::Cut), "{seen:?}");
        let damage = seen.iter().find_map(|e| match *e {
            SpearEvent::TetherEnd {
                damage_permille, ..
            } => Some(damage_permille),
            _ => None,
        });
        assert_eq!(damage, Some(0), "a cut deals nothing");
        assert!(matches!(slot(&sim, 0).phase, Phase::Idle), "the spear drops");
        // The same run without the swing is carried to the goal, not cut.
        let (_, control) = carry_with_swing(18_000, false);
        assert_eq!(first_end(&control), Some(EndWhy::Arrived), "{control:?}");
    }

    #[test]
    fn a_swing_that_misses_the_rope_does_not_cut_it() {
        // The victim swings away from the thrower: the blade lies ahead of the
        // body, the rope behind it.
        let (_, seen) = carry_with_swing(0, true);
        assert_eq!(first_end(&seen), Some(EndWhy::Arrived), "{seen:?}");
    }

    /// Yank run with a bystander at x `side` (z 0.5) facing `bystander_face`,
    /// swinging when `swing`. Returns how the tether ended.
    fn yank_with_bystander(side: f32, bystander_face: i16, swing: bool) -> Option<EndWhy> {
        let mut sim = sim3();
        place(&mut sim, [0.0, -4.0], [0.0, 4.0]);
        sim.states[2].pos = [side, 0.0, 0.5];
        let back = (0, 0, 0, -100);
        let still = (0, bystander_face, 0, 0);
        tick_all(&mut sim, &[(THROW, 0, 0, -100), (0, 0, 0, 0), still]);
        let mut seen = events(&sim);
        let mut armed = false;
        for _ in 0..90 {
            let hooked = seen.iter().any(|e| matches!(e, SpearEvent::Hit { .. }));
            if hooked && swing && !armed {
                swing_now(&mut sim, 2);
                armed = true;
            }
            let me = if hooked { (0, 0, 0, 0) } else { back };
            tick_all(&mut sim, &[me, (0, 0, 0, 0), still]);
            seen.extend(events(&sim));
        }
        assert!(
            seen.iter()
                .any(|e| matches!(e, SpearEvent::Hit { victim: Some(1), .. })),
            "the hook sticks"
        );
        first_end(&seen)
    }

    #[test]
    fn a_third_fighter_cuts_a_yank() {
        // Beside the line, facing it: the blade reaches the rope. The yank
        // stuns its victim, so only a bystander can cut it.
        assert_eq!(yank_with_bystander(1.5, -9_000, true), Some(EndWhy::Cut));
        assert_eq!(
            yank_with_bystander(1.5, -9_000, false),
            Some(EndWhy::Arrived),
            "no swing, no cut"
        );
        assert_eq!(
            yank_with_bystander(3.0, -9_000, true),
            Some(EndWhy::Arrived),
            "the blade falls short of the rope"
        );
        assert_eq!(
            yank_with_bystander(1.5, 9_000, true),
            Some(EndWhy::Arrived),
            "the blade points away from the rope"
        );
    }

    #[test]
    fn the_throwers_own_swing_does_not_cut_its_rope() {
        let mut sim = sim_in(MatchWorld::empty());
        place(&mut sim, [0.0, -4.0], [0.0, -1.0]);
        tick(&mut sim, [(THROW, 0), (0, 18_000)]);
        let mut seen = events(&sim);
        while !seen.iter().any(|e| matches!(e, SpearEvent::Hit { .. })) {
            tick(&mut sim, [(0, 0), (0, 0)]);
            seen.extend(events(&sim));
        }
        swing_now(&mut sim, 0);
        for _ in 0..80 {
            tick(&mut sim, [(0, 0), (0, 0)]);
            seen.extend(events(&sim));
        }
        assert_ne!(first_end(&seen), Some(EndWhy::Cut), "{seen:?}");
    }

    #[test]
    fn rope_geometry_crossing_touching_and_missing() {
        // Crossing X, a near parallel inside the blade + rope tolerance, a
        // parallel outside it, and a zero-length rope.
        let gap = |a, b, c, d| segment_gap_sq(a, b, c, d);
        assert_eq!(gap([-100, 0], [100, 0], [0, -100], [0, 100]), 0);
        assert_eq!(gap([0, 0], [1000, 0], [0, 50], [1000, 50]), 2500);
        assert_eq!(gap([0, 0], [1000, 0], [0, 500], [1000, 500]), 250_000);
        assert_eq!(gap([0, 0], [1000, 0], [500, 300], [500, 300]), 90_000);
        assert_eq!(gap([0, 0], [1000, 0], [1500, 0], [2500, 0]), 250_000);
    }

    #[test]
    fn parry_cuts_the_tether_and_staggers_the_thrower() {
        let mut sim = sim_in(MatchWorld::empty());
        place(&mut sim, [0.0, -4.0], [0.0, 2.0]);
        tick(&mut sim, [(THROW, 0), (0, 0)]);
        idle(&mut sim, 12, [(0, 0), (0, 0)]);
        assert!(matches!(slot(&sim, 0).phase, Phase::Tether(_)), "tethered");
        tick(&mut sim, [(0, 0), (BLOCK, 0)]);
        let cut = events(&sim)
            .into_iter()
            .find(|e| matches!(e, SpearEvent::TetherEnd { .. }));
        assert_eq!(
            cut,
            Some(SpearEvent::TetherEnd {
                thrower: 0,
                victim: Some(1),
                why: EndWhy::Parry,
                damage_permille: 0
            })
        );
        assert!(sim.fight.fighters[0].react > 0, "the thrower staggers");
    }

    #[test]
    fn block_cuts_the_tether_and_a_facing_guard_turns_the_spear() {
        // Guard raised long before the hit, back to the thrower: it tethers,
        // then the held guard (past the parry window) cuts it.
        let mut sim = sim_in(MatchWorld::empty());
        place(&mut sim, [0.0, -4.0], [0.0, 2.0]);
        idle(&mut sim, 12, [(0, 0), (BLOCK, 0)]);
        tick(&mut sim, [(THROW, 0), (BLOCK, 0)]);
        let seen = idle(&mut sim, 20, [(0, 0), (BLOCK, 0)]);
        assert!(seen.iter().any(|e| matches!(
            e,
            SpearEvent::TetherEnd {
                why: EndWhy::Guard,
                ..
            }
        )));
        // Same guard turned to face the thrower: the spear is deflected.
        let mut sim = sim_in(MatchWorld::empty());
        place(&mut sim, [0.0, -4.0], [0.0, 2.0]);
        idle(&mut sim, 12, [(0, 0), (BLOCK, 18_000)]);
        tick(&mut sim, [(THROW, 0), (BLOCK, 18_000)]);
        let seen = idle(&mut sim, 20, [(0, 0), (BLOCK, 18_000)]);
        assert!(seen.contains(&SpearEvent::Deflected {
            thrower: 0,
            victim: 1,
            parried: false
        }));
        assert!(!seen.iter().any(|e| matches!(e, SpearEvent::Hit { .. })));
        assert_eq!(slot(&sim, 0).phase, Phase::Idle);
    }

    #[test]
    fn release_press_ends_the_tether_and_keeps_momentum() {
        let mut sim = sim_in(MatchWorld::empty());
        place(&mut sim, [0.0, -4.0], [0.0, 2.0]);
        tick(&mut sim, [(THROW, 0), (0, 0)]);
        idle(&mut sim, 10, [(0, 0), (0, 0)]);
        assert!(matches!(slot(&sim, 0).phase, Phase::Tether(_)));
        tick(&mut sim, [(THROW, 0), (0, 0)]);
        let ended = events(&sim);
        assert!(ended.iter().any(|e| matches!(
            e,
            SpearEvent::TetherEnd {
                why: EndWhy::Released,
                damage_permille: 0,
                ..
            }
        )));
        let before = sim.states[0].pos[2];
        let carried = slot(&sim, 0).drift_mm[1];
        assert!(carried > 0, "momentum survives the release");
        tick(&mut sim, [(0, 0), (0, 0)]);
        assert!(sim.states[0].pos[2] > before, "the thrower keeps gliding");
        idle(&mut sim, 200, [(0, 0), (0, 0)]);
        assert_eq!(slot(&sim, 0).drift_mm, [0, 0], "friction stops the glide");
    }

    #[test]
    fn slam_into_an_obstacle_ends_the_tether_with_damage() {
        let world =
            MatchWorld::try_new(vec![Obstacle::aligned([0.0, 7.0], [3.0, 0.5])]).expect("world");
        let mut sim = sim_in(world);
        place(&mut sim, [0.0, -3.0], [0.0, 5.0]);
        tick(&mut sim, [(THROW, 0), (0, 0)]);
        let mut seen = events(&sim);
        seen.extend(idle(&mut sim, 60, [(0, 0), (0, 0)]));
        let end = seen
            .iter()
            .find_map(|e| match e {
                SpearEvent::TetherEnd {
                    why,
                    damage_permille,
                    ..
                } => Some((*why, *damage_permille)),
                _ => None,
            })
            .expect("the tether ends");
        assert_eq!(end.0, EndWhy::Wall, "stopped by the obstacle");
        assert!(end.1 >= 60, "slam damage: {end:?}");
        assert!(sim.states[1].pos[2] < 6.2);
    }

    #[test]
    fn spear_off_has_no_state_and_the_button_changes_nothing() {
        let mut on = sim_in(MatchWorld::empty());
        let mut off = Sim::build_world(
            &config(),
            &FighterTape::default(),
            false,
            false,
            MatchWorld::empty(),
        )
        .expect("sim builds");
        assert!(off.spear.is_none());
        place(&mut on, [0.0, -4.0], [0.0, 2.0]);
        place(&mut off, [0.0, -4.0], [0.0, 2.0]);
        for _ in 0..30 {
            tick(&mut off, [(THROW, 0), (0, 0)]);
        }
        assert_eq!(off.states[1].pos, [0.0, 0.0, 2.0], "nothing flew");
        assert_eq!(off.fight.fighters[1].hp_permille, 1000);
        assert_eq!(off.buttons[0] & THROW_SPEAR, 0, "the bit is not latched");
        assert_eq!(KNOWN_BUTTONS & THROW_SPEAR, 0);
    }

    /// SG4: with the height channel armed the wall anchor is a rope. The hook
    /// holds the thrower on a sphere around it while the run turns into an
    /// arc, the reel takes rope in, and a release carries the arc out. The
    /// same tape without the channel is the old zip, straight to the wall.
    #[test]
    fn rope_swing_holds_the_arc_and_the_release_carries_it() {
        const RUN: i8 = 100;
        let world =
            || MatchWorld::try_new(vec![Obstacle::aligned([0.0, 4.0], [3.0, 0.5])]).expect("world");
        // Throw +z into the wall while running +x along it, then 24 ticks of
        // the run: returns the events of those ticks and the final pose.
        fn throw_and_run(sim: &mut Sim) -> (Vec<SpearEvent>, [f32; 3]) {
            tick_move(sim, [(THROW, 0, RUN, 0), (0, 0, 0, 0)]);
            let mut seen = events(sim);
            seen.extend(idle_move(sim, 24, [(0, 0, RUN, 0), (0, 0, 0, 0)]));
            (seen, sim.states[0].pos)
        }

        let mut swing = sim_swing(world());
        place(&mut swing, [0.0, -3.0], [8.0, -8.0]);
        let (seen, _) = throw_and_run(&mut swing);
        assert!(
            seen.iter().any(|e| matches!(
                e,
                SpearEvent::Hit {
                    thrower: 0,
                    victim: None,
                    ..
                }
            )),
            "the spear sticks in the wall: {seen:?}"
        );
        let Phase::Tether(t) = swing.spear.as_ref().expect("armed").slots[0].phase else {
            panic!("still on the rope after 25 ticks");
        };
        let body = pos_mm(swing.states[0].pos);
        let anchor = [t.anchor_mm[0], t.anchor_y_mm, t.anchor_mm[1]];
        let span =
            isqrt(sq(body[0] - anchor[0]) + sq(body[1] - anchor[1]) + sq(body[2] - anchor[2]));
        assert!(
            (span - i64::from(t.rope_mm)).abs() < 500,
            "the rope holds the arc: span={span} rope={}",
            t.rope_mm
        );
        assert!(
            span > 3000,
            "the arc keeps the thrower off the wall: span={span}"
        );

        // The same tape on the floor-plane path is covered by
        // `spear_hits_wall_and_zips_the_thrower`, which must stay green.

        // Release: the arc's speed is kept, so the body keeps travelling.
        let before = swing.states[0].pos;
        tick_move(&mut swing, [(THROW, 0, RUN, 0), (0, 0, 0, 0)]);
        assert!(
            events(&swing).iter().any(|e| matches!(
                e,
                SpearEvent::TetherEnd {
                    why: EndWhy::Released,
                    ..
                }
            )),
            "a second press releases the rope"
        );
        idle_move(&mut swing, 10, [(0, 0, RUN, 0), (0, 0, 0, 0)]);
        let (from, to) = (pos_mm(before), pos_mm(swing.states[0].pos));
        let carried = isqrt(sq(to[0] - from[0]) + sq(to[2] - from[2]));
        assert!(
            carried > 250,
            "the release carries the arc: {carried} mm in 10 ticks"
        );
    }

    /// One of a mirrored pair of matches: the same fight with x negated, and
    /// the facing that throws along its own +x (`-9_000` on the mirror side).
    struct Twin {
        sim: Sim,
        facing: i16,
    }

    impl std::fmt::Debug for Twin {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            let poses: Vec<[f32; 3]> = self.sim.states.iter().map(|s| s.pos).collect();
            write!(
                f,
                "facing {} poses {poses:?} spear {:?}",
                self.facing,
                self.sim.spear.as_ref().map(|s| s.slots())
            )
        }
    }

    /// A spear match with both fighters placed; `sign` mirrors x.
    fn twin(
        world: MatchWorld,
        sign: f32,
        facing: i16,
        thrower: [f32; 2],
        victim: [f32; 2],
    ) -> Twin {
        let mut sim = sim_in(world);
        place(
            &mut sim,
            [sign * thrower[0], thrower[1]],
            [sign * victim[0], victim[1]],
        );
        Twin { sim, facing }
    }

    /// Throw once on tick 0, then hold still.
    fn throw_then_hold(twin: &mut Twin, tick: u32) {
        let buttons = if tick == 0 { THROW } else { 0 };
        tick_move(&mut twin.sim, [(buttons, twin.facing, 0, 0), (0, 0, 0, 0)]);
    }

    /// Run one twin for `ticks`, collecting the spear events of every tick.
    fn run_twin(twin: &mut Twin, ticks: u32) -> Vec<SpearEvent> {
        let mut seen = Vec::new();
        for tick in 0..ticks {
            throw_then_hold(twin, tick);
            seen.extend(events(&twin.sim));
        }
        seen
    }

    /// Two spear phases are x-mirrors of each other (positions and velocities
    /// negate in x only; the anchor, goal and victim match).
    fn phase_mirrored(p: Phase, q: Phase) -> bool {
        match (p, q) {
            (Phase::Idle, Phase::Idle) => true,
            (Phase::Flight(f), Phase::Flight(g)) => {
                f.pos_mm[0] == -g.pos_mm[0]
                    && f.pos_mm[1..] == g.pos_mm[1..]
                    && f.vel_mm[0] == -g.vel_mm[0]
                    && f.vel_mm[1..] == g.vel_mm[1..]
            }
            (Phase::Tether(t), Phase::Tether(u)) => {
                t.victim == u.victim
                    && t.latched == u.latched
                    && t.goal_mm[0] == -u.goal_mm[0]
                    && t.goal_mm[1] == u.goal_mm[1]
                    && t.anchor_mm[0] == -u.anchor_mm[0]
                    && t.anchor_mm[1] == u.anchor_mm[1]
            }
            _ => false,
        }
    }

    /// Every body and spear of `b` is the x-mirror of `a`'s; hp and cooldowns match.
    fn mirrored_twins(a: &Twin, b: &Twin) -> bool {
        let bodies =
            a.sim.states.iter().zip(&b.sim.states).all(|(p, q)| {
                p.pos[0] == -q.pos[0] && p.pos[1] == q.pos[1] && p.pos[2] == q.pos[2]
            });
        let hp = a
            .sim
            .fight
            .fighters
            .iter()
            .zip(&b.sim.fight.fighters)
            .all(|(p, q)| p.hp_permille == q.hp_permille);
        let spears = match (a.sim.spear.as_ref(), b.sim.spear.as_ref()) {
            (Some(p), Some(q)) => p
                .slots()
                .iter()
                .zip(q.slots())
                .all(|(s, t)| s.cooldown == t.cooldown && phase_mirrored(s.phase, t.phase)),
            _ => false,
        };
        bodies && hp && spears
    }

    #[test]
    fn mirrored_throws_land_mirrored_on_a_fighter() {
        let right = || twin(MatchWorld::empty(), 1.0, 9_000, [-3.0, 0.0], [1.5, 0.0]);
        let left = twin(MatchWorld::empty(), -1.0, -9_000, [-3.0, 0.0], [1.5, 0.0]);
        // The +x throw really lands, so the mirror check below is not vacuous.
        let seen = run_twin(&mut right(), 80);
        assert!(
            seen.iter().any(|e| matches!(
                e,
                SpearEvent::Hit {
                    thrower: 0,
                    victim: Some(1),
                    ..
                }
            )),
            "the +x throw hits: {seen:?}"
        );
        laws::mirror(right(), left, 80, throw_then_hold, mirrored_twins)
            .expect("a mirrored throw stays mirrored");
    }

    #[test]
    fn mirrored_throws_anchor_mirrored_on_a_wall() {
        let wall = |x: f32| {
            MatchWorld::try_new(vec![Obstacle::aligned([x, 0.0], [0.5, 3.0])]).expect("world")
        };
        let right = || twin(wall(4.5), 1.0, 9_000, [-2.0, 0.0], [-8.0, -8.0]);
        let left = twin(wall(-4.5), -1.0, -9_000, [-2.0, 0.0], [-8.0, -8.0]);
        let seen = run_twin(&mut right(), 60);
        assert!(
            seen.iter().any(|e| matches!(
                e,
                SpearEvent::Hit {
                    thrower: 0,
                    victim: None,
                    ..
                }
            )),
            "the +x throw sticks in the wall: {seen:?}"
        );
        laws::mirror(right(), left, 60, throw_then_hold, mirrored_twins)
            .expect("a mirrored wall anchor stays mirrored");
    }

    #[test]
    fn a_spear_never_passes_a_fighter_between_ticks() {
        // One tick of SPEAR_SPEED is 600 mm. A fighter centred anywhere on
        // that path, up to TIP_RADIUS + CAPSULE_RADIUS off it, must be caught.
        let reach = TIP_RADIUS + CAPSULE_RADIUS;
        let mut hits = Vec::new();
        for tick in 0..25 {
            let z0 = -7_200 + 600 * tick;
            for quarter in 0..=4 {
                let z = z0 + 150 * quarter;
                for lateral in [0, 300, reach - 1] {
                    let body = [lateral, 0, z];
                    let caught = cast_contact(
                        &mut hits,
                        [0, SPEAR_HEIGHT, z0],
                        [0, SPEAR_HEIGHT, z0 + 600],
                        &[hurt_capsule(7, body)],
                    );
                    assert_eq!(
                        caught.map(|(id, _)| id),
                        Some(7),
                        "tick {tick}, fighter centred at {body:?}"
                    );
                }
            }
        }
        // Just past reach the same sweep misses, so the test above can fail.
        let past = cast_contact(
            &mut hits,
            [0, SPEAR_HEIGHT, -600],
            [0, SPEAR_HEIGHT, 0],
            &[hurt_capsule(7, [reach + 20, 0, -300])],
        );
        assert_eq!(past, None);
    }

    #[test]
    fn a_drifting_body_comes_to_exact_rest_and_stays_there() {
        let speed = |v: &i32| f64::from(v.abs());
        for start in -420..=420 {
            laws::energy_never_rises(start, 2_000, |v| *v = friction(*v), speed, 0.0)
                .expect("drift never speeds up");
            let mut v = start;
            for _ in 0..500 {
                let next = friction(v);
                assert!(
                    next == 0 || next.signum() == v.signum(),
                    "friction flipped {v} to {next}"
                );
                v = next;
            }
            assert_eq!(v, 0, "drift from {start} sticks at {v}");
        }
        laws::rest_stays_rest(0, 200, |v| *v = friction(*v)).expect("rest stays rest");
    }

    #[test]
    fn a_swing_never_gains_energy_without_reel() {
        // With the reel off, the swing's damping is the only motion term left:
        // it may take speed out of the arc, never put it in.
        let energy = |v: &[i32; 2]| v.iter().map(|&l| f64::from(l) * f64::from(l)).sum::<f64>();
        for start in [[1_000, -700], [420, 420], [63, -63], [5, 0], [-420, 420]] {
            laws::energy_never_rises(start, 2_000, |v| *v = damp(*v), energy, 0.0)
                .expect("a swing never pumps");
        }
    }

    #[test]
    fn a_swing_damps_every_speed_to_rest() {
        for start in [[1_000, -700], [63, -63], [5, 0], [-420, 420]] {
            let mut v = start;
            for _ in 0..2_000 {
                v = damp(v);
            }
            assert_eq!(v, [0, 0], "swing damping from {start:?} stalls at {v:?}");
        }
    }

    #[test]
    fn cap_never_raises_speed_and_keeps_direction() {
        for max in [420_i64, 500] {
            for x in (-1_200..=1_200).step_by(37) {
                for z in (-1_200..=1_200).step_by(41) {
                    let before = [x, z];
                    let mut after = before;
                    cap(&mut after, max);
                    let (sb, sa) = (sq(before[0]) + sq(before[1]), sq(after[0]) + sq(after[1]));
                    // Truncation only shrinks a lane, so the cap never adds speed.
                    assert!(sa <= sb, "cap grew {before:?} to {after:?}");
                    // Each lane keeps its sign, so the direction is kept.
                    assert!(
                        before
                            .iter()
                            .zip(&after)
                            .all(|(b, a)| *a == 0 || a.signum() == b.signum()),
                        "cap turned {before:?} to {after:?}"
                    );
                    if isqrt(sb) <= max {
                        assert_eq!(after, before, "below the cap nothing changes");
                    } else {
                        // Each lane truncates by under one mm/tick, so the result
                        // is at most one mm/tick over the limit.
                        assert!(sa <= (max + 1) * (max + 1), "cap overshot {max}: {after:?}");
                    }
                    // The same per-lane bound caps the bend: |cross| < |b0| + |b1|.
                    let cross = i64::from(before[0]) * i64::from(after[1])
                        - i64::from(before[1]) * i64::from(after[0]);
                    assert!(
                        cross.abs() <= i64::from(before[0]).abs() + i64::from(before[1]).abs(),
                        "cap bent {before:?} to {after:?}"
                    );
                }
            }
        }
    }
}
