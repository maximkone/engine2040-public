//! pilum: the legionary's throwing pilum, integer authority.
//!
//! Every legionary carries two pila. On the horn the line looses them: the
//! pilum flies flat, it strikes the scutum in front of the enemy or the body
//! behind it, and a pilum that sticks in a scutum weighs it down until the
//! bearer pulls it out or drops the shield.
//!
//! The flight is the spear's (`spear.rs`): the same throw origin, the same
//! swept cast against the fighter capsules, the same walk through the arena
//! geometry. The pilum adds the scutum, the guard weight and its own reach:
//! an effective throw of 15-25 m and about 30 m at the far end, where the
//! pilum falls short of anything beyond.
//!
//! Plan: `docs/plan/rome.md` section 4.2 (the pila on the horn) and
//! section 5 (the range numbers).
//!
//! Consumer: `Sim::step_with`, only for a match whose pilum channel is
//! armed (`Sim::set_pilum`). The horn is the arena's press-tick call cue:
//! the same cue the window maps to sound, so a call tick sounds the horn
//! and the line looses.
//! Data owner: [`PilumState`], owned by the `Sim`.
//! Units: integer millimetres on y-up axes, velocities in mm per tick
//! (60 Hz), directions as Q16 unit vectors.
//! Determinism: `+ - * /`, `isqrt` and the Q16 `swept_cast`; slots in
//! order, no float ever enters the hash.
//! Errors: none returned; a cast the physics refuses counts as a miss.
//! Budget: no heap per tick beyond the buffers reserved once.

use engine_combat::fight::{FightState, Tables};
use engine_core::hash::Fnv64;
use engine_input::fighter::{ATTACK, BLOCK, DODGE};
use engine_input::FighterInput;
use engine_physics::query::{CastCapsule, SweptHit};

use crate::match_wire::pos_mm;
use crate::spear;
use crate::world::MatchWorld;
use crate::{facing_q16, ArenaRules, FighterState, MAX_SLOTS};

/// Muzzle speed of a thrown pilum, mm per tick (36 m/s).
const PILUM_SPEED: i32 = 600;
/// Vertical drop per tick squared, mm: the pilum falls as it flies.
const PILUM_GRAVITY: i32 = 1;
/// Spawn distance ahead of the thrower, mm.
const PILUM_AHEAD: i32 = 400;
/// Spawn height over the floor, mm (the shoulder).
const PILUM_HEIGHT: i32 = 1400;
/// Longest flight in ticks: fifty ticks at 600 mm per tick is the 30 m
/// reach, and past it the pilum falls short.
const PILUM_MAX_TICKS: u16 = 50;
/// Pila each legionary carries.
const PILA_CARRY: u8 = 2;
/// The scutum stands this far in front of its bearer, mm.
const SHIELD_AHEAD: i32 = 350;
/// Lowest edge of the scutum, mm.
const SHIELD_BOTTOM: i32 = 400;
/// Highest edge of the scutum, mm.
const SHIELD_TOP: i32 = 1500;
/// Half-thickness of the scutum as a swept capsule, mm.
const SHIELD_RADIUS: i32 = 250;
/// Damage a pilum in the body deals, permille.
const BODY_DAMAGE: u16 = 90;
/// Ticks the guard stays down while the pilum sits in the scutum.
const GUARD_DELAY: u16 = 24;
/// Ticks the bearer needs to pull the pilum out.
const PULL_TICKS: u16 = 30;
/// Event vec capacity reserved once.
const EVENT_CAP: usize = 4 * MAX_SLOTS;
/// Fold tag byte.
const FOLD_TAG: u8 = 0x50;

/// Why a pilum flight ended without a hit.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MissWhy {
    /// The flight used up its reach: the pilum fell short.
    Short,
    /// The pilum dropped to the floor.
    Ground,
    /// The pilum met the arena wall, a pillar or an obstacle.
    Geometry,
}

/// One thing the pilum did this tick, for presentation to react to.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PilumEvent {
    /// The horn sounded and the line loosed.
    Horn {
        /// Tick of the match the horn sounded on.
        tick: u32,
        /// Team that called the volley.
        team: u8,
    },
    /// A legionary loosed a pilum.
    Thrown {
        /// Thrower slot.
        thrower: u8,
        /// Spawn point, mm.
        at_mm: [i32; 3],
        /// Initial velocity, mm per tick.
        vel_mm: [i32; 3],
    },
    /// The pilum struck a fighter: in the scutum or in the body.
    Hit {
        /// Thrower slot.
        thrower: u8,
        /// Struck fighter.
        victim: u8,
        /// Contact point, mm.
        at_mm: [i32; 3],
        /// True when the pilum stuck in the scutum.
        shield: bool,
    },
    /// The pilum fell: it reached no target.
    Landed {
        /// Thrower slot.
        thrower: u8,
        /// Landing point, mm.
        at_mm: [i32; 3],
        /// Why the flight ended.
        why: MissWhy,
    },
    /// The bearer pulled the pilum out of the scutum.
    Pulled {
        /// Fighter slot.
        fighter: u8,
    },
    /// The bearer dropped the scutum with the pilum in it.
    Dropped {
        /// Fighter slot.
        fighter: u8,
        /// Where the scutum fell, mm.
        at_mm: [i32; 3],
    },
}

/// A pilum in the air.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Flight {
    /// Pilum tip, mm.
    pub pos_mm: [i32; 3],
    /// Velocity, mm per tick.
    pub vel_mm: [i32; 3],
    /// Ticks in the air.
    pub age: u16,
    /// Thrower slot.
    pub thrower: u8,
}

/// What one fighter's scutum carries.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Shield {
    /// The scutum is still on the arm.
    pub carried: bool,
    /// A pilum is stuck in it.
    pub stuck: bool,
    /// Ticks left of the pull that takes the pilum out.
    pub pull: u16,
    /// Ticks the guard still waits before it comes up.
    pub guard_wait: u16,
}

impl Shield {
    /// A scutum on the arm with nothing in it.
    const fn new() -> Self {
        Self {
            carried: true,
            stuck: false,
            pull: 0,
            guard_wait: 0,
        }
    }
}

/// One slot's pila, flight and scutum.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct PilumSlot {
    /// Pila still on the man.
    pub pila: u8,
    /// The pilum in the air, one at a time.
    pub flight: Option<Flight>,
    /// The scutum on this fighter's arm.
    pub shield: Shield,
    /// The attack button was down on the last tick: the pull edge.
    pub attack_held: bool,
    /// The block button was down on the last tick: the guard edge.
    pub block_held: bool,
    /// The dodge button was down on the last tick: the drop edge.
    pub dodge_held: bool,
}

impl PilumSlot {
    /// A legionary with two pila, a scutum on the arm and nothing in the air.
    const fn new() -> Self {
        Self {
            pila: PILA_CARRY,
            flight: None,
            shield: Shield::new(),
            attack_held: false,
            block_held: false,
            dodge_held: false,
        }
    }
}

/// Everything the pilum step touches in the sim, borrowed for one tick.
pub(crate) struct Env<'a> {
    /// This tick's merged inputs, one per fighter.
    pub inputs: &'a [FighterInput],
    /// Fighter poses.
    pub states: &'a [FighterState],
    /// Combat state (damage only).
    pub fight: &'a mut FightState,
    /// Fight rules (teams).
    pub rules: &'a ArenaRules,
    /// Move tables of the match.
    pub tables: &'a Tables,
    /// Static obstacle world.
    pub world: &'a MatchWorld,
    /// Arena half-extent, metres.
    pub half_extent_m: f32,
    /// The horn sounded this tick: the arena's press-tick call cue, and
    /// the team that called it.
    pub horn: Option<u8>,
    /// Tick of the match.
    pub tick: u32,
}

/// All pilum state of one match.
pub struct PilumState {
    slots: [PilumSlot; MAX_SLOTS],
    fighters: usize,
    events: Vec<PilumEvent>,
    hits: Vec<SweptHit>,
    caps: Vec<CastCapsule>,
}

impl PilumState {
    /// Fresh state for `fighters` slots, every legionary with two pila.
    pub(crate) fn new(fighters: usize) -> Self {
        Self {
            slots: [PilumSlot::new(); MAX_SLOTS],
            fighters,
            events: Vec::with_capacity(EVENT_CAP),
            hits: Vec::with_capacity(2 * MAX_SLOTS),
            caps: Vec::with_capacity(2 * MAX_SLOTS),
        }
    }

    /// Fold the pilum state into a checkpoint hash.
    pub(crate) fn fold(&self, hash: u64) -> u64 {
        let mut digest = Fnv64::with_state(hash);
        digest.write_u8(FOLD_TAG);
        for slot in &self.slots[..self.fighters] {
            digest.write_u8(slot.pila);
            digest.write_u8(u8::from(slot.shield.carried));
            digest.write_u8(u8::from(slot.shield.stuck));
            digest.write_u16(slot.shield.pull);
            digest.write_u16(slot.shield.guard_wait);
            digest.write_bool(slot.attack_held);
            digest.write_bool(slot.block_held);
            digest.write_bool(slot.dodge_held);
            match slot.flight {
                None => digest.write_u8(0),
                Some(f) => {
                    digest.write_u8(1);
                    for lane in f.pos_mm.into_iter().chain(f.vel_mm) {
                        digest.write_i32(lane);
                    }
                    digest.write_u16(f.age);
                    digest.write_u8(f.thrower);
                }
            }
        }
        digest.finish()
    }

    /// Hold the guards down while a pilum sits in a scutum: a fresh block
    /// press waits [`GUARD_DELAY`] ticks before the guard comes up. The
    /// block bit is masked before the fight step sees it, so the fighter
    /// really guards late.
    pub(crate) fn hold_guards(&mut self, row: &mut [FighterInput]) {
        for (slot, input) in row.iter_mut().enumerate().take(self.fighters) {
            let pressed = input.buttons.0 & BLOCK != 0;
            if !pressed {
                self.slots[slot].block_held = false;
                self.slots[slot].shield.guard_wait = 0;
                continue;
            }
            if !self.slots[slot].block_held && self.slots[slot].shield.stuck {
                self.slots[slot].shield.guard_wait = GUARD_DELAY;
            }
            self.slots[slot].block_held = true;
            if self.slots[slot].shield.guard_wait > 0 {
                self.slots[slot].shield.guard_wait -= 1;
                input.buttons.0 &= !BLOCK;
            }
        }
    }

    /// One tick: the horn, the flights and the scutum.
    pub(crate) fn step(&mut self, env: &mut Env<'_>) {
        self.events.clear();
        if let Some(team) = env.horn {
            self.events.push(PilumEvent::Horn {
                tick: env.tick,
                team,
            });
            self.loose(env, team);
        }
        for slot in 0..self.fighters {
            self.fly(slot, env);
        }
        for slot in 0..self.fighters {
            self.scutum(slot, env);
        }
    }

    /// The horn: every man of the calling side that still carries a pilum
    /// looses one along its facing.
    fn loose(&mut self, env: &Env<'_>, team: u8) {
        for slot in 0..self.fighters {
            if env.rules.teams[slot] != team {
                continue;
            }
            if self.slots[slot].pila == 0 || self.slots[slot].flight.is_some() {
                continue;
            }
            if env.fight.fighters[slot].hp_permille == 0 {
                continue;
            }
            self.throw(slot, env);
        }
    }

    /// Loose one pilum from `slot` along its facing.
    fn throw(&mut self, slot: usize, env: &Env<'_>) {
        let (sin, cos) = facing_q16(env.inputs[slot].facing);
        let dir = [sin, cos];
        let body = pos_mm(env.states[slot].pos);
        let (at_mm, vel_mm) = spear::throw_start(body, dir, PILUM_AHEAD, PILUM_HEIGHT, PILUM_SPEED);
        self.slots[slot].pila -= 1;
        self.slots[slot].flight = Some(Flight {
            pos_mm: at_mm,
            vel_mm,
            age: 0,
            thrower: slot_id(slot),
        });
        self.events.push(PilumEvent::Thrown {
            thrower: slot_id(slot),
            at_mm,
            vel_mm,
        });
    }

    /// Advance a flying pilum one tick and resolve what it meets.
    fn fly(&mut self, slot: usize, env: &mut Env<'_>) {
        let Some(mut f) = self.slots[slot].flight else {
            return;
        };
        f.vel_mm[1] -= PILUM_GRAVITY;
        f.age += 1;
        let from = f.pos_mm;
        let full = [
            from[0] + f.vel_mm[0],
            from[1] + f.vel_mm[1],
            from[2] + f.vel_mm[2],
        ];
        let (to, geometry) =
            spear::march_segment(env.world, env.rules, env.half_extent_m, from, full);
        if let Some((victim, shield, at)) = self.cast(env, from, to, slot) {
            self.strike(slot, f, victim, shield, at, env);
        } else if geometry {
            self.land(slot, f, to, MissWhy::Geometry);
        } else if to[1] <= 0 {
            self.land(slot, f, to, MissWhy::Ground);
        } else if f.age >= PILUM_MAX_TICKS {
            self.land(slot, f, to, MissWhy::Short);
        } else {
            f.pos_mm = to;
            self.slots[slot].flight = Some(f);
        }
    }

    /// The nearest fighter the segment `from..to` meets: the scutum
    /// first, the body behind it. Legal: living, not the thrower, not a
    /// teammate, not in dodge i-frames.
    fn cast(
        &mut self,
        env: &Env<'_>,
        from: [i32; 3],
        to: [i32; 3],
        thrower: usize,
    ) -> Option<(usize, bool, [i32; 3])> {
        self.caps.clear();
        for (victim, state) in env.states.iter().enumerate().take(self.fighters) {
            let combat = &env.fight.fighters[victim];
            if victim == thrower
                || combat.hp_permille == 0
                || env.rules.teams[victim] == env.rules.teams[thrower]
                || combat.invulnerable_with(env.tables)
            {
                continue;
            }
            let body = pos_mm(state.pos);
            self.caps.push(spear::hurt_capsule(body_id(victim), body));
            if self.slots[victim].shield.carried {
                let (sin, cos) = facing_q16(env.inputs[victim].facing);
                self.caps
                    .push(shield_capsule(shield_id(victim), body, [sin, cos]));
            }
        }
        let (id, at) = spear::cast_contact(&mut self.hits, from, to, &self.caps)?;
        let victim = usize::try_from(id / 2).ok()?;
        Some((victim, id % 2 == 1, at))
    }

    /// The pilum met fighter `victim`: in the scutum it sticks and weighs
    /// it down, in the body it wounds.
    fn strike(
        &mut self,
        slot: usize,
        f: Flight,
        victim: usize,
        shield: bool,
        at: [i32; 3],
        env: &mut Env<'_>,
    ) {
        self.slots[slot].flight = None;
        if shield {
            self.slots[victim].shield.stuck = true;
        } else {
            spear::hurt_fighter(env.fight, victim, BODY_DAMAGE);
        }
        self.events.push(PilumEvent::Hit {
            thrower: f.thrower,
            victim: slot_id(victim),
            at_mm: at,
            shield,
        });
    }

    /// The pilum fell: the flight ends and the pilum is spent.
    fn land(&mut self, slot: usize, f: Flight, at: [i32; 3], why: MissWhy) {
        self.slots[slot].flight = None;
        self.events.push(PilumEvent::Landed {
            thrower: f.thrower,
            at_mm: at,
            why,
        });
    }

    /// The bearer works the pilum out of the scutum, or drops the scutum
    /// with the pilum in it. Both free the guard.
    fn scutum(&mut self, slot: usize, env: &Env<'_>) {
        let buttons = env.inputs[slot].buttons.0;
        let attack_edge = buttons & ATTACK != 0 && !self.slots[slot].attack_held;
        let dodge_edge = buttons & DODGE != 0 && !self.slots[slot].dodge_held;
        self.slots[slot].attack_held = buttons & ATTACK != 0;
        self.slots[slot].dodge_held = buttons & DODGE != 0;
        if !self.slots[slot].shield.stuck {
            return;
        }
        if dodge_edge {
            let body = pos_mm(env.states[slot].pos);
            self.slots[slot].shield = Shield {
                carried: false,
                stuck: false,
                pull: 0,
                guard_wait: 0,
            };
            self.events.push(PilumEvent::Dropped {
                fighter: slot_id(slot),
                at_mm: [body[0], 0, body[2]],
            });
            return;
        }
        if attack_edge && self.slots[slot].shield.pull == 0 {
            self.slots[slot].shield.pull = PULL_TICKS;
        }
        if self.slots[slot].shield.pull > 0 {
            self.slots[slot].shield.pull -= 1;
            if self.slots[slot].shield.pull == 0 {
                self.slots[slot].shield.stuck = false;
                self.events.push(PilumEvent::Pulled {
                    fighter: slot_id(slot),
                });
            }
        }
    }
}

/// Slot index as the wire id (slots are below 16, so the fallback never fires).
fn slot_id(slot: usize) -> u8 {
    u8::try_from(slot).unwrap_or(u8::MAX)
}

/// Capsule id of a fighter's body: the even half of the pair.
fn body_id(slot: usize) -> u32 {
    u32::try_from(slot.saturating_mul(2)).unwrap_or(u32::MAX)
}

/// Capsule id of a fighter's scutum: the odd half of the pair.
fn shield_id(slot: usize) -> u32 {
    u32::try_from(slot.saturating_mul(2).saturating_add(1)).unwrap_or(u32::MAX)
}

/// The scutum of one fighter: a vertical plate in front of the bearer.
fn shield_capsule(id: u32, body_mm: [i32; 3], dir: [i32; 2]) -> CastCapsule {
    let ahead = |lane: i32| lane.saturating_mul(SHIELD_AHEAD) / 65_536;
    let x = body_mm[0].saturating_add(ahead(dir[0]));
    let z = body_mm[2].saturating_add(ahead(dir[1]));
    CastCapsule {
        id,
        a: [spear::fx(x), spear::fx(SHIELD_BOTTOM), spear::fx(z)],
        b: [spear::fx(x), spear::fx(SHIELD_TOP), spear::fx(z)],
        radius: spear::fx(SHIELD_RADIUS),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ArenaConfig, Sim};
    use engine_combat::tables::TABLES_VERSION_V2;
    use engine_input::{Buttons, FighterId, FighterTape};

    /// Frontage of a Roman line: a man every 1.2 m, so the scuta of one
    /// rank never touch.
    const FRONTAGE_M: f32 = 1.2;

    /// One tick of a match whose rules v2 rows raise the call cue the
    /// horn rides.
    fn config(fighters: u8, half_extent_m: f32) -> ArenaConfig {
        ArenaConfig {
            seed: 7,
            ticks: 600,
            fighters,
            half_extent_m,
            table_version: TABLES_VERSION_V2,
        }
    }

    /// A pilum match on open sand.
    fn sim_in(fighters: u8, half_extent_m: f32) -> Sim {
        let mut sim = Sim::build_world(
            &config(fighters, half_extent_m),
            &FighterTape::default(),
            false,
            false,
            MatchWorld::empty(),
        )
        .expect("sim builds");
        assert!(sim.set_pilum());
        sim
    }

    /// Two teams: the legionaries on one, the enemy line on the other, so
    /// a sword never strikes a comrade.
    fn team_rules(legion: usize, enemies: usize) -> ArenaRules {
        let mut ids = [0u8; MAX_SLOTS];
        for (slot, id) in ids.iter_mut().enumerate() {
            *id = if slot < legion {
                0
            } else if slot < enemies {
                1
            } else {
                2
            };
        }
        ArenaRules::v2_teams(ids)
    }

    /// Put one fighter on the map.
    fn place(sim: &mut Sim, slot: usize, at: [f32; 2]) {
        sim.states[slot].pos = [at[0], 0.0, at[1]];
    }

    /// One tick with scripted buttons and facings, one entry per fighter.
    fn tick(sim: &mut Sim, script: &[(u8, i16)]) {
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

    /// Run `n` ticks on the same script.
    fn idle(sim: &mut Sim, n: u32, script: &[(u8, i16)]) {
        for _ in 0..n {
            tick(sim, script);
        }
    }

    /// Run `n` ticks on the same script, collecting every pilum event.
    fn idle_events(sim: &mut Sim, n: u32, script: &[(u8, i16)]) -> Vec<PilumEvent> {
        let mut seen = Vec::new();
        for _ in 0..n {
            tick(sim, script);
            seen.extend(events(sim));
        }
        seen
    }

    /// The pilum events of the last stepped tick.
    fn events(sim: &Sim) -> Vec<PilumEvent> {
        sim.pilum.as_ref().expect("pilum").events.clone()
    }

    /// The pilum state of one slot.
    fn slot(sim: &Sim, index: usize) -> PilumSlot {
        sim.pilum.as_ref().expect("pilum").slots[index]
    }

    /// The script of the horn: the legionaries press attack, the enemy
    /// line holds its ground. The press is released on the next tick, so
    /// the call sounds once.
    fn horn_script(fighters: usize, legion: usize) -> Vec<(u8, i16)> {
        (0..fighters)
            .map(|slot| {
                if slot < legion {
                    (ATTACK, 0)
                } else {
                    (0, 18_000)
                }
            })
            .collect()
    }

    /// The script of the line at rest: nobody presses a button, the
    /// legionaries face the enemy and the enemy faces them.
    fn still_script(fighters: usize, legion: usize) -> Vec<(u8, i16)> {
        (0..fighters)
            .map(|slot| if slot < legion { (0, 0) } else { (0, 18_000) })
            .collect()
    }

    /// The x of the man at `slot` in a line of `count` men.
    fn line_x(slot: usize, count: usize) -> f32 {
        (slot as f32 - (count as f32 - 1.0) / 2.0) * FRONTAGE_M
    }

    #[test]
    fn pilum_volley_of_eight_at_twenty_meters_lands_on_a_line_of_shields() {
        const LINE: usize = 8;
        let mut sim = sim_in(16, 40.0);
        assert!(sim.set_rules(team_rules(LINE, 2 * LINE)));
        for slot in 0..LINE {
            place(&mut sim, slot, [line_x(slot, LINE), -20.0]);
            place(&mut sim, slot + LINE, [line_x(slot, LINE), 0.0]);
        }
        // The horn sounds, the line looses, the pila fly.
        tick(&mut sim, &horn_script(16, LINE));
        let still = still_script(16, LINE);
        let mut seen = events(&sim);
        seen.extend(idle_events(&mut sim, 60, &still));
        let thrown = seen
            .iter()
            .filter(|e| matches!(e, PilumEvent::Thrown { .. }))
            .count();
        assert_eq!(thrown, LINE, "one pilum per legionary: {seen:?}");
        let hits = seen
            .iter()
            .filter(|e| matches!(e, PilumEvent::Hit { shield: true, .. }))
            .count();
        assert_eq!(hits, LINE, "every pilum stuck in a scutum: {seen:?}");
        for victim in LINE..2 * LINE {
            assert!(
                slot(&sim, victim).shield.stuck,
                "the scutum of slot {victim} carries the pilum"
            );
            assert_eq!(sim.fight.fighters[victim].hp_permille, 1000, "no wound");
        }
        for man in 0..LINE {
            assert!(
                !slot(&sim, man).shield.stuck,
                "the scutum of the legionary {man} is clean"
            );
        }
        assert_eq!(slot(&sim, 0).pila, 1, "one pila left on the man");
    }

    #[test]
    fn pilum_in_a_shield_makes_the_guard_slower() {
        let mut sim = sim_in(2, 40.0);
        assert!(sim.set_rules(team_rules(1, 2)));
        place(&mut sim, 0, [0.0, -20.0]);
        place(&mut sim, 1, [0.0, 0.0]);
        // The horn: the legionary looses, the pilum sticks in the scutum.
        tick(&mut sim, &horn_script(2, 1));
        let still = still_script(2, 1);
        idle(&mut sim, 40, &still);
        assert!(slot(&sim, 1).shield.stuck, "the pilum is in the scutum");
        // The bearer now guards: the guard comes up late.
        let guard = [(0, 0), (BLOCK, 18_000)];
        tick(&mut sim, &guard);
        assert!(
            !sim.fight.fighters[1].is_blocking,
            "the pilum weighs the guard down"
        );
        let mut late = 0;
        while !sim.fight.fighters[1].is_blocking && late < 120 {
            tick(&mut sim, &guard);
            late += 1;
        }
        assert_eq!(late, u32::from(GUARD_DELAY), "the guard is late");
        assert!(sim.fight.fighters[1].is_blocking, "the guard came up");
    }

    #[test]
    fn pilum_thrown_past_thirty_meters_falls_short() {
        let mut sim = sim_in(2, 60.0);
        assert!(sim.set_rules(team_rules(1, 2)));
        place(&mut sim, 0, [0.0, -35.0]);
        place(&mut sim, 1, [0.0, 0.0]);
        tick(&mut sim, &horn_script(2, 1));
        let still = still_script(2, 1);
        let seen = idle_events(&mut sim, 80, &still);
        let landed = seen
            .iter()
            .find_map(|e| match e {
                PilumEvent::Landed { at_mm, why, .. } => Some((*at_mm, *why)),
                _ => None,
            })
            .expect("the pilum lands");
        assert_eq!(landed.1, MissWhy::Short, "the reach ran out");
        assert!(
            landed.0[2] < -4_000,
            "it fell short of the enemy line: {:?}",
            landed.0
        );
        assert!(
            !seen.iter().any(|e| matches!(e, PilumEvent::Hit { .. })),
            "nothing was hit: {seen:?}"
        );
        assert_eq!(sim.fight.fighters[1].hp_permille, 1000);
    }

    #[test]
    fn pilum_replay_twice_gives_the_same_hashes() {
        fn run() -> Vec<u64> {
            let mut sim = sim_in(4, 40.0);
            assert!(sim.set_rules(team_rules(2, 4)));
            for slot in 0..2 {
                place(&mut sim, slot, [line_x(slot, 2), -20.0]);
                place(&mut sim, slot + 2, [line_x(slot, 2), 0.0]);
            }
            tick(&mut sim, &horn_script(4, 2));
            let still = still_script(4, 2);
            idle(&mut sim, 60, &still);
            sim.checkpoints.iter().map(|c| c.hash).collect()
        }
        let first = run();
        let second = run();
        assert_eq!(first, second, "the same match hashes the same twice");
        assert!(first.len() >= 60);
    }

    #[test]
    fn pilum_off_by_default_leaves_the_match_untouched() {
        let mut off = Sim::build_world(
            &config(2, 40.0),
            &FighterTape::default(),
            false,
            false,
            MatchWorld::empty(),
        )
        .expect("sim builds");
        assert!(off.pilum.is_none(), "the channel is off by default");
        place(&mut off, 0, [0.0, -20.0]);
        place(&mut off, 1, [0.0, 0.0]);
        tick(&mut off, &horn_script(2, 1));
        let still = still_script(2, 1);
        idle(&mut off, 40, &still);
        assert_eq!(off.fight.fighters[1].hp_permille, 1000, "nothing flew");
        assert_eq!(off.states[1].pos, [0.0, 0.0, 0.0], "nobody was hit");
    }
}
