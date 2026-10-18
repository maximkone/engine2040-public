//! spear: the throwable spear and its grapple tether, integer authority.
//!
//! Plan: `docs/plan/spear-grapple.md` (read it for the rules and constants).
//! A fighter presses `THROW_SPEAR`; a spear flies along the thrower's facing
//! and either tethers the thrower to an enemy (both are then carried toward
//! the spear's end point with momentum, damage on arrival or on a wall slam)
//! or anchors on world geometry (the thrower zips to it, Spiderman style).
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
use engine_input::fighter::THROW_SPEAR;
use engine_input::FighterInput;
use engine_physics::query::{swept_cast, CastCapsule, SweptHit};

use crate::collide::collide_states;
use crate::match_wire::pos_mm;
use crate::world::MatchWorld;
use crate::{
    canon_zero, facing_q16, wall_limit, ArenaRules, FighterState, FIGHTER_RADIUS_M, MAX_SLOTS,
};

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
const HIT_DAMAGE: u16 = 30;
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
}

/// A tether between a thrower and an enemy or a world anchor.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Tether {
    /// Tethered fighter, `None` for a world anchor.
    pub victim: Option<u8>,
    /// Where the pull is headed, `[x, z]` mm.
    pub goal_mm: [i32; 2],
    /// Flight direction `[x, z]` as a Q16 unit vector.
    pub dir: [i32; 2],
    /// Ticks since the spear stuck.
    pub age: u16,
    /// True once the two bodies share one momentum.
    pub latched: bool,
    /// Thrower speed after the last tick, mm per tick.
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
}

/// All spear state of one match.
pub struct SpearState {
    slots: [SpearSlot; MAX_SLOTS],
    fighters: usize,
    events: Vec<SpearEvent>,
    hits: Vec<SweptHit>,
    /// Per-tick scratch: `(blocked, speed before the stop)` per slot.
    shocks: [(bool, i64); MAX_SLOTS],
}

impl SpearState {
    /// Fresh state for `fighters` slots, nobody throwing.
    pub(crate) fn new(fighters: usize) -> Self {
        Self {
            slots: [SpearSlot::default(); MAX_SLOTS],
            fighters,
            events: Vec::with_capacity(EVENT_CAP),
            hits: Vec::with_capacity(MAX_SLOTS),
            shocks: [(false, 0); MAX_SLOTS],
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
            match slot.phase {
                Phase::Idle => digest.write_u8(0),
                Phase::Flight(f) => {
                    digest.write_u8(1);
                    for lane in f.pos_mm.into_iter().chain(f.vel_mm).chain(f.origin_mm) {
                        digest.write_i32(lane);
                    }
                    digest.write_u16(f.age);
                }
                Phase::Tether(t) => {
                    digest.write_u8(2);
                    digest.write_u8(t.victim.map_or(u8::MAX, |v| v));
                    for lane in t.goal_mm.into_iter().chain(t.dir) {
                        digest.write_i32(lane);
                    }
                    digest.write_u16(t.age);
                    digest.write_bool(t.latched);
                }
            }
        }
        digest.finish()
    }

    /// One tick: presses and flights, tether physics, body motion, endings.
    pub(crate) fn step(&mut self, env: &mut Env<'_>) {
        self.events.clear();
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
        let pressed = input.buttons.0 & THROW_SPEAR != 0;
        let edge = pressed && !self.slots[slot].held;
        self.slots[slot].held = pressed;
        let cooldown = self.slots[slot].cooldown.saturating_sub(1);
        self.slots[slot].cooldown = cooldown;
        match self.slots[slot].phase {
            Phase::Idle => {
                if edge && cooldown == 0 && env.fight.fighters[slot].can_act() {
                    self.throw(slot, input, env);
                    self.fly(slot, env);
                }
            }
            Phase::Flight(_) => self.fly(slot, env),
            Phase::Tether(_) if edge => self.end(slot, EndWhy::Released, 0, env),
            Phase::Tether(_) => self.pull(slot, env),
        }
    }

    /// Launch a spear from `slot` along its facing.
    fn throw(&mut self, slot: usize, input: FighterInput, env: &Env<'_>) {
        let (sin, cos) = facing_q16(input.facing);
        let dir = [sin, cos];
        let body = pos_mm(env.states[slot].pos);
        let ahead = |lane: i32| lane.saturating_mul(SPEAR_AHEAD) / 65_536;
        let pos_mm = [
            body[0].saturating_add(ahead(dir[0])),
            SPEAR_HEIGHT,
            body[2].saturating_add(ahead(dir[1])),
        ];
        let muzzle = |lane: i32| lane.saturating_mul(SPEAR_SPEED) / 65_536;
        let vel_mm = [muzzle(dir[0]), 0, muzzle(dir[1])];
        self.slots[slot].cooldown = THROW_COOLDOWN;
        self.slots[slot].phase = Phase::Flight(Flight {
            pos_mm,
            vel_mm,
            age: 0,
            origin_mm: [body[0], body[2]],
            dir,
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
        let (to, geometry) = march(env, from, full);
        if let Some((victim, at)) = self.cast(slot, env, from, to) {
            self.strike(slot, victim, at, f, env);
        } else if geometry {
            self.anchor(slot, to, f);
        } else if to[1] <= 0 {
            self.miss(slot, MissWhy::Floor);
        } else if f.age >= MAX_FLIGHT_TICKS {
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
            caps[count] = CastCapsule {
                id: u32::from(slot_id(victim)),
                a: [fx(body[0]), fx(CAPSULE_BOTTOM), fx(body[2])],
                b: [fx(body[0]), fx(CAPSULE_TOP), fx(body[2])],
                radius: fx(CAPSULE_RADIUS),
            };
            count += 1;
        }
        let tip = [fx(from[0]), fx(from[1]), fx(from[2])];
        let end = [fx(to[0]), fx(to[1]), fx(to[2])];
        swept_cast(tip, end, fx(TIP_RADIUS), &caps[..count], &mut self.hits).ok()?;
        let hit = self.hits.first()?;
        let t = i64::from(u32::try_from(hit.toi.to_bits()).unwrap_or(0).min(65_536));
        let at = [0, 1, 2].map(|lane| {
            let span = i64::from(to[lane]) - i64::from(from[lane]);
            from[lane] + i32::try_from(span * t / 65_536).unwrap_or(0)
        });
        Some((usize::try_from(hit.id).ok()?, at))
    }

    /// The spear met fighter `victim`: a guard turns it away, anything else
    /// sticks and tethers.
    fn strike(&mut self, slot: usize, victim: usize, at: [i32; 3], f: Flight, env: &mut Env<'_>) {
        let combat = env.fight.fighters[victim];
        let thrower = pos_mm(env.states[slot].pos);
        let body = pos_mm(env.states[victim].pos);
        let (sin, cos) = facing_q16(env.inputs[victim].facing);
        let to_thrower = (
            i64::from(thrower[0]) - i64::from(body[0]),
            i64::from(thrower[2]) - i64::from(body[2]),
        );
        let facing_thrower = i64::from(sin) * to_thrower.0 + i64::from(cos) * to_thrower.1 > 0;
        if combat.is_blocking && facing_thrower {
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
            return;
        }
        hurt(env, victim, i64::from(HIT_DAMAGE));
        let travelled = isqrt(sq(at[0] - f.origin_mm[0]) + sq(at[2] - f.origin_mm[1]));
        let reach = i64::from(SPEAR_RANGE).max(travelled + i64::from(MIN_CARRY));
        let goal_mm = [0, 1].map(|lane| {
            f.origin_mm[lane] + i32::try_from(i64::from(f.dir[lane]) * reach / 65_536).unwrap_or(0)
        });
        self.slots[slot].phase = Phase::Tether(Tether {
            victim: Some(slot_id(victim)),
            goal_mm,
            dir: f.dir,
            age: 0,
            latched: false,
            speed: 0,
        });
        self.events.push(SpearEvent::Hit {
            thrower: slot_id(slot),
            victim: Some(slot_id(victim)),
            at_mm: at,
        });
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
            dir: f.dir,
            age: 0,
            latched: false,
            speed: 0,
        });
        self.events.push(SpearEvent::Hit {
            thrower: slot_id(slot),
            victim: None,
            at_mm: at,
        });
    }

    /// Tether cut checks, then the velocity update of both bodies.
    fn pull(&mut self, slot: usize, env: &mut Env<'_>) {
        let Phase::Tether(mut t) = self.slots[slot].phase else {
            return;
        };
        let victim = t.victim.map(usize::from);
        let combat = victim.map(|v| env.fight.fighters[v]);
        let down =
            env.fight.fighters[slot].hp_permille == 0 || combat.is_some_and(|c| c.hp_permille == 0);
        let why = if down {
            Some(EndWhy::Down)
        } else if t.age >= TETHER_MAX {
            Some(EndWhy::Timeout)
        } else if combat.is_some_and(|c| c.parry_open_with(env.tables)) {
            stagger(env, slot);
            Some(EndWhy::Parry)
        } else if combat.is_some_and(|c| c.is_blocking) {
            Some(EndWhy::Guard)
        } else {
            None
        };
        if let Some(why) = why {
            self.end(slot, why, 0, env);
            return;
        }
        let me = xz(env.states[slot].pos);
        let mut mine = self.slots[slot].drift_mm;
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
        } else {
            push(&mut mine, unit(me, t.goal_mm), WALL_REEL_ACCEL);
            cap(&mut mine, WALL_MAX_SPEED);
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
            self.shocks[slot] = (blocked, speed);
            if blocked {
                self.slots[slot].drift_mm = [0, 0];
            }
        }
    }

    /// Arrival and wall endings after the bodies moved.
    fn settle(&mut self, slot: usize, env: &mut Env<'_>) {
        let Phase::Tether(t) = self.slots[slot].phase else {
            return;
        };
        let (thrower_blocked, _) = self.shocks[slot];
        let Some(v) = t.victim.map(usize::from) else {
            let me = xz(env.states[slot].pos);
            if thrower_blocked {
                self.end(slot, EndWhy::Wall, 0, env);
            } else if arrived(me, t) {
                self.end(slot, EndWhy::Arrived, 0, env);
            }
            return;
        };
        let (victim_blocked, victim_speed) = self.shocks[v];
        let you = xz(env.states[v].pos);
        if victim_blocked {
            let damage = (SLAM_DAMAGE + victim_speed / 6).min(MAX_TETHER_DAMAGE);
            self.end(slot, EndWhy::Wall, damage, env);
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

/// Floating metres from integer millimetres, for the float pose only.
fn metres(mm: i32) -> f32 {
    mm as f32 / 1000.0
}

/// Q16 metres from integer millimetres.
fn fx(mm: i32) -> Fixed {
    Fixed::from_bits(i64::from(mm) * Fixed::SCALE / 1000)
}

/// Square of a millimetre lane.
fn sq(lane: i32) -> i64 {
    i64::from(lane) * i64::from(lane)
}

/// Integer square root of a non-negative value.
fn isqrt(value: i64) -> i64 {
    i64::try_from(u64::try_from(value).unwrap_or(0).isqrt()).unwrap_or(0)
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
fn march(env: &Env<'_>, from: [i32; 3], full: [i32; 3]) -> ([i32; 3], bool) {
    let span = [0, 1, 2].map(|lane| full[lane] - from[lane]);
    let len = isqrt(sq(span[0]) + sq(span[1]) + sq(span[2]));
    let steps = (len / i64::from(MARCH_STEP) + 1).clamp(1, 16);
    for k in 1..=steps {
        let point = [0, 1, 2]
            .map(|lane| from[lane] + i32::try_from(i64::from(span[lane]) * k / steps).unwrap_or(0));
        if in_geometry(env, point) {
            return (point, true);
        }
    }
    (full, false)
}

/// True when a spear tip at `p` is in the arena wall, a pillar or an obstacle.
fn in_geometry(env: &Env<'_>, p: [i32; 3]) -> bool {
    let face = pos_mm([env.half_extent_m, 0.0, 0.0])[0];
    if p[0].abs() >= face || p[2].abs() >= face {
        return true;
    }
    let in_pillar = env.rules.pillars.iter().flatten().any(|pillar| {
        (pillar.min_mm[0]..=pillar.max_mm[0]).contains(&p[0])
            && (pillar.min_mm[1]..=pillar.max_mm[1]).contains(&p[2])
    });
    in_pillar
        || (!env.world.is_empty()
            && env
                .world
                .overlaps([metres(p[0]), metres(p[2])], TIP_RADIUS_M))
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
    let hp = &mut env.fight.fighters[victim].hp_permille;
    *hp = hp.saturating_sub(u16::try_from(damage).unwrap_or(u16::MAX));
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
}
