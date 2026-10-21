//! bolt: the manuballista (cheiroballistra), the ballistarius's one-man
//! torsion bolt-shooter, integer authority.
//!
//! The engine is small: two twisted springs of sinew or hair sit in an
//! iron frame, a wind key turns them, and a rail carries one short iron
//! bolt. The shooter winds both springs, loads a bolt of about 18 cm and
//! looses it flat and fast at about 31 m/s. The shot spends the tension,
//! so every next bolt needs the crank again. The case is counted: the
//! bolts run out and the engine goes quiet.
//!
//! The bolt flies flat. It sinks a fixed amount per tick instead of
//! falling in a parabola, so at 30 m it has lost less height than a
//! thrown pilum has at the same range and still strikes at chest height.
//! A bolt that meets a raised scutum sticks in it or pierces it, by the
//! table; a bolt that meets an unshielded man goes through him along the
//! body path. A bolt never strikes the shooter's own side, so the
//! ballistarius can cover a work party from the rampart.
//!
//! The start numbers sit in the kit table `content/rome/kit.toml` under
//! `kit.manuballista`, with the refs of the owner's reconstruction
//! sources: a bolt of about 7 in (18 cm) shot at over 70 mph (31 m/s).
//! The tick costs are our own start values and are rechecked when the
//! codex holds Schalles 2010.
//!
//! Plan: `docs/plan/rome.md` section 4.9 (the manuballista and its
//! jobs) and section 5 (the numbers).
//!
//! Consumer: `Sim::step_with`, only for a match whose bolt channel is
//! armed (`Sim::set_bolt`) and only under the rules v2 rows: the engine
//! is a rules v2 weapon, so rules v1 never arms it and the golden and
//! the arena digest never move.
//! Data owner: [`BoltState`], owned by the `Sim`.
//! Units: integer millimetres on y-up axes, velocities in mm per tick
//! (60 Hz), directions as Q16 unit vectors.
//! Determinism: `+ - * /`, `isqrt` and the Q16 `swept_cast`; slots in
//! order, no float ever enters the hash.
//! Errors: none returned; a cast the physics refuses counts as a miss.
//! Budget: no heap per tick beyond the buffers reserved once.

use engine_combat::fight::{FightState, Tables};
use engine_core::hash::Fnv64;
use engine_input::fighter::{ATTACK, BLOCK};
use engine_input::FighterInput;
use engine_physics::query::{CastCapsule, SweptHit};

use crate::match_wire::pos_mm;
use crate::spear;
use crate::world::MatchWorld;
use crate::{facing_q16, ArenaRules, FighterState, FIGHT_RULES_V2, MAX_SLOTS};

/// The slot that carries the manuballista: the ballistarius on the
/// rampart. He is the only one with the engine, so the rest of the line
/// is the work party the bolt flies over.
pub const BALLISTARIUS: usize = 0;

/// Muzzle speed of a manuballista bolt, mm per tick (31 m/s at 60 Hz).
const BOLT_SPEED: i32 = 517;
/// Vertical sink of the bolt, mm per tick: the bolt flies flat, so it
/// loses a fixed height per tick instead of falling in a parabola.
const BOLT_SINK: i32 = 8;
/// Spawn distance ahead of the shooter, mm.
const BOLT_AHEAD: i32 = 300;
/// Spawn height over the floor, mm: the rail sits at the shoulder.
const BOLT_HEIGHT: i32 = 1400;
/// Springs in the frame: one turn of the key winds one of them.
const SPRING_TURNS: u8 = 2;
/// Ticks one turn of the wind key takes.
const WIND_TICKS_PER_TURN: u16 = 30;
/// Ticks the loader takes to put one bolt on the rail.
const LOAD_TICKS: u16 = 20;
/// Bolts the ballistarius carries in his case.
const BOLT_CARRY: u8 = 12;
/// Range within which a bolt pierces a scutum, mm; beyond it the bolt
/// sticks in the plank sandwich and the man behind it is unhurt.
const PIERCE_RANGE: i32 = 20_000;
/// Damage a bolt through the body deals, permille.
const BODY_DAMAGE: u16 = 250;
/// Length of the wound path through a body, mm: the bolt goes in one
/// side and out the other.
const BODY_PATH_MM: i32 = 600;
/// Longest flight of a bolt, ticks (about 60 m at 517 mm per tick).
const BOLT_MAX_TICKS: u16 = 120;
/// The scutum stands this far in front of its bearer, mm.
const SHIELD_AHEAD: i32 = 350;
/// Lowest edge of the scutum, mm.
const SHIELD_BOTTOM: i32 = 400;
/// Highest edge of the scutum, mm.
const SHIELD_TOP: i32 = 1500;
/// Half-thickness of the scutum as a swept capsule, mm.
const SHIELD_RADIUS: i32 = 250;
/// Event vec capacity reserved once.
const EVENT_CAP: usize = 8;
/// Fold tag byte.
const FOLD_TAG: u8 = 0x42;

/// What the manuballista is doing.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Phase {
    /// The springs are relaxed and no bolt is on the rail.
    Idle,
    /// The crank is turning: `turn` springs are wound and the key has
    /// run `tick` ticks into the next turn.
    Winding {
        /// Springs wound so far.
        turn: u8,
        /// Ticks into the current turn.
        tick: u16,
    },
    /// Both springs hold; the loader is putting a bolt on the rail.
    Loading {
        /// Ticks of loading so far.
        tick: u16,
    },
    /// A bolt is on the rail and the springs hold it.
    Ready,
}

/// What a bolt does to a scutum, by the table.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ShieldOutcome {
    /// The bolt lodged in the scutum: the man behind it is unhurt.
    Stuck,
    /// The bolt went through the scutum and through the man behind it.
    Pierced,
}

/// Why a bolt flight ended without a hit.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MissWhy {
    /// The bolt dropped to the floor.
    Ground,
    /// The bolt met the arena wall, a pillar or an obstacle.
    Geometry,
    /// The bolt used up its flight.
    Range,
}

/// One thing the manuballista did this tick, for presentation to react to.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BoltEvent {
    /// The key wound one spring.
    Wound {
        /// The ballistarius slot.
        shooter: u8,
        /// The spring that is wound now, counted from one.
        turn: u8,
    },
    /// A bolt sat on the rail.
    Loaded {
        /// The ballistarius slot.
        shooter: u8,
    },
    /// The bolt left the rail.
    Shot {
        /// The ballistarius slot.
        shooter: u8,
        /// Spawn point, mm.
        at_mm: [i32; 3],
        /// Initial velocity, mm per tick.
        vel_mm: [i32; 3],
    },
    /// The bolt met a scutum: it stuck or it pierced, by the table.
    ShieldHit {
        /// The ballistarius slot.
        shooter: u8,
        /// The struck fighter.
        victim: u8,
        /// Contact point, mm.
        at_mm: [i32; 3],
        /// What the table says the bolt did.
        outcome: ShieldOutcome,
    },
    /// The bolt went through a man: the wound runs the body path.
    BodyHit {
        /// The ballistarius slot.
        shooter: u8,
        /// The struck fighter.
        victim: u8,
        /// Where the bolt struck, mm.
        at_mm: [i32; 3],
        /// Where the bolt came out, mm.
        out_mm: [i32; 3],
    },
    /// The bolt flew out and met nothing.
    Missed {
        /// The ballistarius slot.
        shooter: u8,
        /// Where the bolt ended, mm.
        at_mm: [i32; 3],
        /// Why the flight ended.
        why: MissWhy,
    },
}

/// A bolt in the air.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Flight {
    /// Bolt tip, mm.
    pub pos_mm: [i32; 3],
    /// Velocity, mm per tick.
    pub vel_mm: [i32; 3],
    /// Ticks in the air.
    pub age: u16,
    /// The ballistarius slot.
    pub shooter: u8,
    /// Where the bolt left the rail, `[x, z]` mm.
    pub origin_mm: [i32; 2],
}

/// The ballistarius and his engine.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct BoltSlot {
    /// Bolts left in the case.
    pub bolts: u8,
    /// What the engine is doing.
    pub phase: Phase,
    /// The bolt in the air, one at a time.
    pub flight: Option<Flight>,
    /// The attack button was down on the last tick: the trigger edge.
    pub trigger_held: bool,
}

impl BoltSlot {
    /// A ballistarius with a full case and a relaxed engine.
    const fn new() -> Self {
        Self {
            bolts: BOLT_CARRY,
            phase: Phase::Idle,
            flight: None,
            trigger_held: false,
        }
    }
}

/// Everything the bolt step touches in the sim, borrowed for one tick.
pub(crate) struct Env<'a> {
    /// This tick's merged inputs, one per fighter.
    pub inputs: &'a [FighterInput],
    /// Fighter poses.
    pub states: &'a [FighterState],
    /// Combat state (damage only).
    pub fight: &'a mut FightState,
    /// Fight rules (teams, version).
    pub rules: &'a ArenaRules,
    /// Fight rules tables.
    pub tables: &'a Tables,
    /// Static obstacle world.
    pub world: &'a MatchWorld,
    /// Arena half-extent, metres.
    pub half_extent_m: f32,
}

/// All bolt state of one match.
pub struct BoltState {
    slot: BoltSlot,
    fighters: usize,
    events: Vec<BoltEvent>,
    hits: Vec<SweptHit>,
    caps: Vec<CastCapsule>,
}

impl BoltState {
    /// Fresh state for `fighters` slots: the ballistarius holds a full
    /// case of bolts and his engine is relaxed.
    pub(crate) fn new(fighters: usize) -> Self {
        Self {
            slot: BoltSlot::new(),
            fighters,
            events: Vec::with_capacity(EVENT_CAP),
            hits: Vec::with_capacity(2 * MAX_SLOTS),
            caps: Vec::with_capacity(2 * MAX_SLOTS),
        }
    }

    /// The ballistarius and his engine.
    pub fn slot(&self) -> &BoltSlot {
        &self.slot
    }

    /// Events of the last stepped tick, in emission order.
    pub fn events(&self) -> &[BoltEvent] {
        &self.events
    }

    /// Fold the bolt state into a checkpoint hash.
    pub(crate) fn fold(&self, hash: u64) -> u64 {
        let mut digest = Fnv64::with_state(hash);
        digest.write_u8(FOLD_TAG);
        digest.write_u8(self.slot.bolts);
        digest.write_bool(self.slot.trigger_held);
        match self.slot.phase {
            Phase::Idle => digest.write_u8(0),
            Phase::Winding { turn, tick } => {
                digest.write_u8(1);
                digest.write_u8(turn);
                digest.write_u16(tick);
            }
            Phase::Loading { tick } => {
                digest.write_u8(2);
                digest.write_u16(tick);
            }
            Phase::Ready => digest.write_u8(3),
        }
        match self.slot.flight {
            None => digest.write_u8(0),
            Some(f) => {
                digest.write_u8(1);
                for lane in f.pos_mm.into_iter().chain(f.vel_mm).chain(f.origin_mm) {
                    digest.write_i32(lane);
                }
                digest.write_u16(f.age);
                digest.write_u8(f.shooter);
            }
        }
        digest.finish()
    }

    /// One tick: the crank, the loader, the trigger and the bolt in the
    /// air. The rules v1 rows never reach here: the engine is a rules v2
    /// weapon, so a v1 match leaves every tick and every hash untouched.
    pub(crate) fn step(&mut self, env: &mut Env<'_>) {
        self.events.clear();
        if env.rules.version != FIGHT_RULES_V2 {
            return;
        }
        self.act(env);
        self.fly(env);
    }

    /// The trigger: the attack press winds the engine and, once the bolt
    /// is on the rail, looses it.
    fn act(&mut self, env: &Env<'_>) {
        let buttons = env.inputs[BALLISTARIUS].buttons.0;
        let pressed = buttons & ATTACK != 0;
        let edge = pressed && !self.slot.trigger_held;
        self.slot.trigger_held = pressed;
        match self.slot.phase {
            Phase::Idle => {
                if edge && self.slot.bolts > 0 {
                    self.slot.phase = Phase::Winding { turn: 0, tick: 0 };
                }
            }
            Phase::Winding { turn, tick } => self.wind(turn, tick),
            Phase::Loading { tick } => self.load(tick),
            Phase::Ready => {
                // One bolt in the air: the trigger waits for the last one.
                if edge && self.slot.bolts > 0 && self.slot.flight.is_none() {
                    self.shoot(env);
                }
            }
        }
    }

    /// The crank: one turn of the wind key takes `WIND_TICKS_PER_TURN`
    /// ticks, and both springs must be wound before the loader can work.
    fn wind(&mut self, turn: u8, tick: u16) {
        let tick = tick.saturating_add(1);
        if tick < WIND_TICKS_PER_TURN {
            self.slot.phase = Phase::Winding { turn, tick };
            return;
        }
        let turn = turn.saturating_add(1);
        self.events.push(BoltEvent::Wound {
            shooter: shooter_id(),
            turn,
        });
        self.slot.phase = if turn >= SPRING_TURNS {
            Phase::Loading { tick: 0 }
        } else {
            Phase::Winding { turn, tick: 0 }
        };
    }

    /// The loader: the bolt goes on the rail while the springs hold.
    fn load(&mut self, tick: u16) {
        let tick = tick.saturating_add(1);
        if tick < LOAD_TICKS {
            self.slot.phase = Phase::Loading { tick };
            return;
        }
        self.events.push(BoltEvent::Loaded {
            shooter: shooter_id(),
        });
        self.slot.phase = Phase::Ready;
    }

    /// Loose the bolt: it leaves the rail flat and fast, the case loses
    /// one bolt and the springs spend their tension.
    fn shoot(&mut self, env: &Env<'_>) {
        let (sin, cos) = facing_q16(env.inputs[BALLISTARIUS].facing);
        let dir = [sin, cos];
        let body = pos_mm(env.states[BALLISTARIUS].pos);
        let (at_mm, muzzle) = spear::throw_start(body, dir, BOLT_AHEAD, BOLT_HEIGHT, BOLT_SPEED);
        let vel_mm = [muzzle[0], -BOLT_SINK, muzzle[2]];
        self.slot.bolts -= 1;
        self.slot.phase = Phase::Idle;
        self.slot.flight = Some(Flight {
            pos_mm: at_mm,
            vel_mm,
            age: 0,
            shooter: shooter_id(),
            origin_mm: [at_mm[0], at_mm[2]],
        });
        self.events.push(BoltEvent::Shot {
            shooter: shooter_id(),
            at_mm,
            vel_mm,
        });
    }

    /// Advance a flying bolt one tick and resolve what it meets.
    fn fly(&mut self, env: &mut Env<'_>) {
        let Some(mut f) = self.slot.flight else {
            return;
        };
        f.age += 1;
        let from = f.pos_mm;
        let full = [
            from[0] + f.vel_mm[0],
            from[1] + f.vel_mm[1],
            from[2] + f.vel_mm[2],
        ];
        let (to, geometry) =
            spear::march_segment(env.world, env.rules, env.half_extent_m, from, full);
        if let Some((victim, shield, at)) = self.cast(&*env, from, to) {
            self.strike(env, f, victim, shield, at);
        } else if geometry {
            self.miss(f, to, MissWhy::Geometry);
        } else if to[1] <= 0 {
            self.miss(f, to, MissWhy::Ground);
        } else if f.age >= BOLT_MAX_TICKS {
            self.miss(f, to, MissWhy::Range);
        } else {
            f.pos_mm = to;
            self.slot.flight = Some(f);
        }
    }

    /// The nearest fighter the segment `from..to` meets: the scutum
    /// first, the body behind it. Legal: living, not the shooter, not a
    /// comrade of his side, not in dodge i-frames.
    fn cast(
        &mut self,
        env: &Env<'_>,
        from: [i32; 3],
        to: [i32; 3],
    ) -> Option<(usize, bool, [i32; 3])> {
        self.caps.clear();
        for (victim, state) in env.states.iter().enumerate().take(self.fighters) {
            let combat = &env.fight.fighters[victim];
            if victim == BALLISTARIUS
                || combat.hp_permille == 0
                || env.rules.teams[victim] == env.rules.teams[BALLISTARIUS]
                || combat.invulnerable_with(env.tables)
            {
                continue;
            }
            let body = pos_mm(state.pos);
            // The pair of ids: the body is even, the scutum odd, so one
            // number says which of the two the bolt met.
            let pair = victim.saturating_mul(2);
            self.caps.push(spear::hurt_capsule(id_of(pair), body));
            // The scutum stands in the way while its bearer guards: that
            // is the arena's shield cone, the same rule a block needs.
            if env.inputs[victim].buttons.0 & BLOCK != 0 {
                let (sin, cos) = facing_q16(env.inputs[victim].facing);
                self.caps.push(scutum_capsule(
                    id_of(pair.saturating_add(1)),
                    body,
                    [sin, cos],
                ));
            }
        }
        let (id, at) = spear::cast_contact(&mut self.hits, from, to, &self.caps)?;
        let victim = usize::try_from(id / 2).ok()?;
        Some((victim, id % 2 == 1, at))
    }

    /// The bolt met fighter `victim`: a raised scutum takes it, by the
    /// table; a man takes it through the body path.
    fn strike(&mut self, env: &mut Env<'_>, f: Flight, victim: usize, shield: bool, at: [i32; 3]) {
        self.slot.flight = None;
        if !shield {
            self.wound(env, f, victim, at);
            return;
        }
        let outcome = if travelled_mm(f, at) <= PIERCE_RANGE {
            ShieldOutcome::Pierced
        } else {
            ShieldOutcome::Stuck
        };
        self.events.push(BoltEvent::ShieldHit {
            shooter: f.shooter,
            victim: victim_id(victim),
            at_mm: at,
            outcome,
        });
        if outcome == ShieldOutcome::Pierced {
            // The bolt went through the scutum and through the man behind it.
            self.wound(env, f, victim, at);
        }
    }

    /// The bolt goes through a man: the wound runs the body path and the
    /// bolt is spent in him.
    fn wound(&mut self, env: &mut Env<'_>, f: Flight, victim: usize, at: [i32; 3]) {
        spear::hurt_fighter(env.fight, victim, BODY_DAMAGE);
        let along = |lane: i32| {
            i32::try_from(i64::from(lane) * i64::from(BODY_PATH_MM) / i64::from(BOLT_SPEED))
                .unwrap_or(0)
        };
        let out_mm = [
            at[0] + along(f.vel_mm[0]),
            at[1],
            at[2] + along(f.vel_mm[2]),
        ];
        self.events.push(BoltEvent::BodyHit {
            shooter: f.shooter,
            victim: victim_id(victim),
            at_mm: at,
            out_mm,
        });
    }

    /// The bolt flew out and met nothing.
    fn miss(&mut self, f: Flight, at: [i32; 3], why: MissWhy) {
        self.slot.flight = None;
        self.events.push(BoltEvent::Missed {
            shooter: f.shooter,
            at_mm: at,
            why,
        });
    }
}

/// The ballistarius slot as the wire id.
fn shooter_id() -> u8 {
    u8::try_from(BALLISTARIUS).unwrap_or(u8::MAX)
}

/// Slot index as the wire id (slots are below 16, so the fallback never fires).
fn victim_id(slot: usize) -> u8 {
    u8::try_from(slot).unwrap_or(u8::MAX)
}

/// Capsule id of a slot pair: the body is even, the scutum odd.
fn id_of(pair: usize) -> u32 {
    u32::try_from(pair).unwrap_or(u32::MAX)
}

/// The scutum of one fighter: a vertical plate standing in front of the
/// bearer, the plate a bolt meets before the man.
fn scutum_capsule(id: u32, body_mm: [i32; 3], dir: [i32; 2]) -> CastCapsule {
    // The plate stands ahead of the bearer, along the facing he shows.
    let step =
        |lane: i32| i32::try_from(i64::from(lane) * i64::from(SHIELD_AHEAD) / 65_536).unwrap_or(0);
    let x = body_mm[0].saturating_add(step(dir[0]));
    let z = body_mm[2].saturating_add(step(dir[1]));
    CastCapsule {
        id,
        a: [spear::fx(x), spear::fx(SHIELD_BOTTOM), spear::fx(z)],
        b: [spear::fx(x), spear::fx(SHIELD_TOP), spear::fx(z)],
        radius: spear::fx(SHIELD_RADIUS),
    }
}

/// The distance the bolt flew when it met `at`, mm: the horizontal reach
/// from the rail to the contact point.
fn travelled_mm(f: Flight, at: [i32; 3]) -> i32 {
    let dx = i64::from(at[0]) - i64::from(f.origin_mm[0]);
    let dz = i64::from(at[2]) - i64::from(f.origin_mm[1]);
    i32::try_from(spear::isqrt(dx * dx + dz * dz)).unwrap_or(i32::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ArenaConfig, Sim};
    use engine_combat::tables::TABLES_VERSION_V2;
    use engine_input::{Buttons, FighterId, FighterTape};

    /// The drop of a thrown pilum at 30 m, mm: the pilum flies 600 mm per
    /// tick with a fall of 1 mm per tick squared, so it reaches 30 m in
    /// 50 ticks and has dropped 50 * 51 / 2 = 1275 mm (the pilum rows in
    /// `crates/arena/src/pilum.rs`).
    const PILUM_DROP_AT_30_M: i32 = 1_275;

    /// One tick of a match whose rules v2 rows arm the engine.
    fn config(fighters: u8, half_extent_m: f32) -> ArenaConfig {
        ArenaConfig {
            seed: 7,
            ticks: 4_000,
            fighters,
            half_extent_m,
            table_version: TABLES_VERSION_V2,
        }
    }

    /// A bolt match on open sand: the ballistarius on his own side, the
    /// rest of the line on the other.
    fn sim_in(fighters: u8, half_extent_m: f32) -> Sim {
        sim_in_own(1, fighters, half_extent_m)
    }

    /// A bolt match with `own` men on the ballistarius' side: he and his
    /// work party, the rest of the line on the other side.
    fn sim_in_own(own: usize, fighters: u8, half_extent_m: f32) -> Sim {
        let mut sim = Sim::build_world(
            &config(fighters, half_extent_m),
            &FighterTape::default(),
            false,
            false,
            MatchWorld::empty(),
        )
        .expect("sim builds");
        assert!(sim.set_rules(team_rules(own, usize::from(fighters))));
        assert!(sim.set_bolt());
        sim
    }

    /// A match under the rules v1 rows: the engine never arms there.
    fn v1_sim() -> Sim {
        let mut sim = Sim::build_world(
            &ArenaConfig {
                seed: 7,
                ticks: 600,
                fighters: 2,
                half_extent_m: 40.0,
                table_version: engine_combat::tables::TABLES_VERSION_V1,
            },
            &FighterTape::default(),
            false,
            false,
            MatchWorld::empty(),
        )
        .expect("sim builds");
        place(&mut sim, BALLISTARIUS, [0.0, -20.0]);
        place(&mut sim, 1, [0.0, 0.0]);
        sim
    }

    /// Two sides: the ballistarius and his work party on one, the enemy
    /// line on the other.
    fn team_rules(own: usize, enemies: usize) -> ArenaRules {
        let mut ids = [0u8; MAX_SLOTS];
        for (slot, id) in ids.iter_mut().enumerate() {
            *id = if slot < own {
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
                    fighter: FighterId(victim_id(slot)),
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

    /// Run `n` ticks on the same script, collecting every bolt event.
    fn idle_events(sim: &mut Sim, n: u32, script: &[(u8, i16)]) -> Vec<BoltEvent> {
        let mut seen = Vec::new();
        for _ in 0..n {
            tick(sim, script);
            seen.extend(events(sim));
        }
        seen
    }

    /// The bolt events of the last stepped tick.
    fn events(sim: &Sim) -> Vec<BoltEvent> {
        sim.bolt.as_ref().expect("bolt").events().to_vec()
    }

    /// The ballistarius and his engine.
    fn slot(sim: &Sim) -> BoltSlot {
        *sim.bolt.as_ref().expect("bolt").slot()
    }

    /// The script of the ballistarius at rest: nobody presses a button, the
    /// ballistarius faces the enemy and the enemy faces him.
    fn still_script(fighters: usize) -> Vec<(u8, i16)> {
        (0..fighters)
            .map(|slot| {
                if slot == BALLISTARIUS {
                    (0, 0)
                } else {
                    (0, 18_000)
                }
            })
            .collect()
    }

    /// The script of the ballistarius pressing the trigger, the enemy line
    /// holding its ground.
    fn trigger_script(fighters: usize) -> Vec<(u8, i16)> {
        (0..fighters)
            .map(|slot| {
                if slot == BALLISTARIUS {
                    (ATTACK, 0)
                } else {
                    (0, 18_000)
                }
            })
            .collect()
    }

    #[test]
    fn manuballista_winds_loads_and_shoots_on_the_table_ticks() {
        let mut sim = sim_in(2, 40.0);
        place(&mut sim, BALLISTARIUS, [0.0, -20.0]);
        place(&mut sim, 1, [0.0, 0.0]);
        let still = still_script(2);
        // The trigger starts the crank.
        tick(&mut sim, &trigger_script(2));
        assert_eq!(
            slot(&sim).phase,
            Phase::Winding { turn: 0, tick: 0 },
            "the crank turns"
        );
        // One turn of the key is not done before the table says so.
        let seen = idle_events(&mut sim, u32::from(WIND_TICKS_PER_TURN) - 1, &still);
        assert!(
            !seen.iter().any(|e| matches!(e, BoltEvent::Wound { .. })),
            "the first spring is not wound yet: {seen:?}"
        );
        assert_eq!(
            slot(&sim).phase,
            Phase::Winding {
                turn: 0,
                tick: WIND_TICKS_PER_TURN - 1
            },
            "the crank is one tick short of the first spring"
        );
        // The first spring is wound after one turn of the key.
        let seen = idle_events(&mut sim, 1, &still);
        assert!(
            seen.iter()
                .any(|e| matches!(e, BoltEvent::Wound { turn: 1, .. })),
            "one spring is wound: {seen:?}"
        );
        // The second spring, then the loader, then the rail.
        let seen = idle_events(&mut sim, u32::from(WIND_TICKS_PER_TURN), &still);
        assert!(
            seen.iter()
                .any(|e| matches!(e, BoltEvent::Wound { turn: 2, .. })),
            "both springs are wound: {seen:?}"
        );
        let seen = idle_events(&mut sim, u32::from(LOAD_TICKS), &still);
        assert!(
            seen.iter().any(|e| matches!(e, BoltEvent::Loaded { .. })),
            "the bolt is on the rail: {seen:?}"
        );
        assert_eq!(slot(&sim).phase, Phase::Ready);
        assert_eq!(
            slot(&sim).bolts,
            BOLT_CARRY,
            "the bolt is still in the case"
        );
        // The trigger looses the bolt.
        tick(&mut sim, &trigger_script(2));
        let seen = events(&sim);
        assert!(
            seen.iter().any(|e| matches!(e, BoltEvent::Shot { .. })),
            "the bolt left the rail: {seen:?}"
        );
        assert_eq!(slot(&sim).bolts, BOLT_CARRY - 1, "the case lost one bolt");
        assert_eq!(
            slot(&sim).phase,
            Phase::Idle,
            "the shot spent the tension: the crank is needed again"
        );
    }

    #[test]
    fn manuballista_bolt_goes_through_a_man_along_the_body_path() {
        let mut sim = sim_in(2, 40.0);
        place(&mut sim, BALLISTARIUS, [0.0, -10.0]);
        place(&mut sim, 1, [0.0, 0.0]);
        let still = still_script(2);
        tick(&mut sim, &trigger_script(2));
        idle_events(
            &mut sim,
            u32::from(WIND_TICKS_PER_TURN) * 2 + u32::from(LOAD_TICKS),
            &still,
        );
        tick(&mut sim, &trigger_script(2));
        let seen = idle_events(&mut sim, 60, &still);
        let hit = seen
            .iter()
            .find_map(|e| match e {
                BoltEvent::BodyHit { at_mm, out_mm, .. } => Some((*at_mm, *out_mm)),
                _ => None,
            })
            .expect("the bolt goes through the man");
        let path = spear::isqrt(spear::sq(hit.1[0] - hit.0[0]) + spear::sq(hit.1[2] - hit.0[2]));
        assert_eq!(
            path,
            i64::from(BODY_PATH_MM),
            "the wound runs the body path"
        );
        assert_eq!(
            sim.fight.fighters[1].hp_permille,
            1_000 - BODY_DAMAGE,
            "the man is wounded through the body"
        );
    }

    #[test]
    fn manuballista_bolt_pierces_a_scutum_close_and_sticks_far() {
        // Close: the bolt keeps its speed and goes through the scutum.
        let mut near = sim_in(2, 60.0);
        place(&mut near, BALLISTARIUS, [0.0, -15.0]);
        place(&mut near, 1, [0.0, 0.0]);
        let guard = [(ATTACK, 0), (BLOCK, 18_000)];
        let still = still_script(2);
        tick(&mut near, &trigger_script(2));
        idle_events(
            &mut near,
            u32::from(WIND_TICKS_PER_TURN) * 2 + u32::from(LOAD_TICKS),
            &still,
        );
        tick(&mut near, &trigger_script(2));
        let seen = idle_events(&mut near, 60, &guard);
        let pierced = seen
            .iter()
            .find_map(|e| match e {
                BoltEvent::ShieldHit { outcome, .. } => Some(*outcome),
                _ => None,
            })
            .expect("the bolt meets the scutum");
        assert_eq!(pierced, ShieldOutcome::Pierced, "close range pierces");
        assert_eq!(
            near.fight.fighters[1].hp_permille,
            1_000 - BODY_DAMAGE,
            "the bolt went through the scutum into the man"
        );
        // Far: the bolt has spent itself and lodges in the plank sandwich.
        let mut far = sim_in(2, 60.0);
        place(&mut far, BALLISTARIUS, [0.0, -25.0]);
        place(&mut far, 1, [0.0, 0.0]);
        tick(&mut far, &trigger_script(2));
        idle_events(
            &mut far,
            u32::from(WIND_TICKS_PER_TURN) * 2 + u32::from(LOAD_TICKS),
            &still,
        );
        tick(&mut far, &trigger_script(2));
        let seen = idle_events(&mut far, 80, &guard);
        let stuck = seen
            .iter()
            .find_map(|e| match e {
                BoltEvent::ShieldHit { outcome, .. } => Some(*outcome),
                _ => None,
            })
            .expect("the bolt meets the scutum");
        assert_eq!(stuck, ShieldOutcome::Stuck, "far range sticks");
        assert_eq!(
            far.fight.fighters[1].hp_permille, 1_000,
            "the man is unhurt"
        );
    }

    #[test]
    fn manuballista_bolt_flies_over_its_own_work_party() {
        let mut sim = sim_in_own(2, 3, 60.0);
        place(&mut sim, BALLISTARIUS, [0.0, -30.0]);
        place(&mut sim, 1, [0.0, -15.0]);
        place(&mut sim, 2, [0.0, 0.0]);
        let still = still_script(3);
        tick(&mut sim, &trigger_script(3));
        idle_events(
            &mut sim,
            u32::from(WIND_TICKS_PER_TURN) * 2 + u32::from(LOAD_TICKS),
            &still,
        );
        tick(&mut sim, &trigger_script(3));
        let seen = idle_events(&mut sim, 90, &still);
        let struck = seen
            .iter()
            .find_map(|e| match e {
                BoltEvent::BodyHit { victim, .. } => Some(*victim),
                _ => None,
            })
            .expect("the bolt reaches the enemy");
        assert_eq!(struck, 2, "the bolt flew over the work party");
        assert_eq!(
            sim.fight.fighters[1].hp_permille, 1_000,
            "the work party is unhurt"
        );
    }

    #[test]
    fn manuballista_bolt_drops_less_than_a_thrown_pilum_at_thirty_meters() {
        let mut sim = sim_in(2, 60.0);
        place(&mut sim, BALLISTARIUS, [0.0, -30.0]);
        place(&mut sim, 1, [40.0, 40.0]);
        let still = still_script(2);
        tick(&mut sim, &trigger_script(2));
        idle_events(
            &mut sim,
            u32::from(WIND_TICKS_PER_TURN) * 2 + u32::from(LOAD_TICKS),
            &still,
        );
        tick(&mut sim, &trigger_script(2));
        let mut drop = None;
        for _ in 0..120 {
            tick(&mut sim, &still);
            let Some(flight) = slot(&sim).flight else {
                break;
            };
            let travelled = i32::from(flight.age) * BOLT_SPEED;
            if travelled >= 30_000 {
                drop = Some(BOLT_HEIGHT - flight.pos_mm[1]);
                break;
            }
        }
        let drop = drop.expect("the bolt reaches 30 m");
        assert!(drop > 0, "the bolt still drops: {drop}");
        assert!(
            drop < PILUM_DROP_AT_30_M,
            "at 30 m the bolt has dropped {drop} mm, the pilum {PILUM_DROP_AT_30_M} mm"
        );
    }

    #[test]
    fn manuballista_counts_its_bolts_and_quiet_when_the_case_is_empty() {
        let mut sim = sim_in(2, 60.0);
        place(&mut sim, BALLISTARIUS, [0.0, -20.0]);
        place(&mut sim, 1, [40.0, 40.0]);
        let still = still_script(2);
        let mut shots = 0;
        for _ in 0..BOLT_CARRY {
            // The engine answers a fresh press: let the trigger go, then
            // press it again to start the crank.
            tick(&mut sim, &still);
            tick(&mut sim, &trigger_script(2));
            idle_events(
                &mut sim,
                u32::from(WIND_TICKS_PER_TURN) * 2 + u32::from(LOAD_TICKS),
                &still,
            );
            // One bolt in the air: the trigger waits for the last one to
            // land before the rail takes another.
            let mut waited = 0;
            while slot(&sim).flight.is_some() && waited < 200 {
                tick(&mut sim, &still);
                waited += 1;
            }
            tick(&mut sim, &trigger_script(2));
            let seen = events(&sim);
            assert!(
                seen.iter().any(|e| matches!(e, BoltEvent::Shot { .. })),
                "the bolt left the rail: {seen:?}"
            );
            shots += 1;
        }
        assert_eq!(shots, usize::from(BOLT_CARRY));
        assert_eq!(slot(&sim).bolts, 0, "the case is empty");
        // The trigger does nothing now: no winding, no bolt.
        tick(&mut sim, &still);
        tick(&mut sim, &trigger_script(2));
        assert_eq!(
            slot(&sim).phase,
            Phase::Idle,
            "an empty case does not turn the crank"
        );
        let seen = idle_events(&mut sim, 40, &still);
        assert!(
            !seen
                .iter()
                .any(|e| matches!(e, BoltEvent::Wound { .. } | BoltEvent::Shot { .. })),
            "an empty case is quiet: {seen:?}"
        );
        assert_eq!(slot(&sim).bolts, 0);
    }

    #[test]
    fn manuballista_replay_twice_gives_the_same_hashes() {
        fn run() -> Vec<u64> {
            let mut sim = sim_in(3, 60.0);
            place(&mut sim, BALLISTARIUS, [0.0, -20.0]);
            place(&mut sim, 1, [0.0, 0.0]);
            place(&mut sim, 2, [3.0, 0.0]);
            let still = still_script(3);
            let guard = [(0, 0), (BLOCK, 18_000), (BLOCK, 18_000)];
            tick(&mut sim, &trigger_script(3));
            idle_events(
                &mut sim,
                u32::from(WIND_TICKS_PER_TURN) * 2 + u32::from(LOAD_TICKS),
                &still,
            );
            tick(&mut sim, &trigger_script(3));
            idle_events(&mut sim, 60, &guard);
            sim.checkpoints.iter().map(|c| c.hash).collect()
        }
        let first = run();
        let second = run();
        assert_eq!(first, second, "the same match hashes the same twice");
        assert!(first.len() >= 60);
    }

    #[test]
    fn manuballista_is_a_rules_v2_weapon() {
        // Rules v1: the engine never arms, so the match is untouched.
        let mut armed = v1_sim();
        assert!(!armed.set_bolt(), "rules v1 never arms the engine");
        assert!(armed.bolt.is_none(), "no bolt state without the channel");
        let mut plain = v1_sim();
        assert!(plain.bolt.is_none(), "the channel is off by default");
        let still = still_script(2);
        idle(&mut armed, 60, &still);
        idle(&mut plain, 60, &still);
        let armed_hash: Vec<u64> = armed.checkpoints.iter().map(|c| c.hash).collect();
        let plain_hash: Vec<u64> = plain.checkpoints.iter().map(|c| c.hash).collect();
        assert_eq!(
            armed_hash, plain_hash,
            "the v1 rows never see the engine: the match is byte for byte the same"
        );
        // The channel is off under the rules v2 rows as well.
        let off = Sim::build_world(
            &config(2, 40.0),
            &FighterTape::default(),
            false,
            false,
            MatchWorld::empty(),
        )
        .expect("sim builds");
        assert!(off.bolt.is_none(), "the channel is off by default");
    }

    #[test]
    fn manuballista_start_numbers_carry_their_refs_in_the_kit_table() {
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../content/rome/kit.toml");
        let text = std::fs::read_to_string(&path).expect("the kit table");
        let item = text
            .split("[kit.manuballista]")
            .nth(1)
            .expect("the manuballista item")
            .split("\n[")
            .next()
            .expect("the item body");
        assert!(item.contains("certainty"), "the item says how sure we are");
        assert!(item.contains("ref ="), "the item names its source");
        // The start numbers sit in the data file with their refs.
        assert!(item.contains("18 cm"), "the bolt length is named: {item}");
        assert!(item.contains("31 m/s"), "the muzzle speed is named: {item}");
        assert!(item.contains("Schalles"), "the paper to recheck is named");
    }
}
