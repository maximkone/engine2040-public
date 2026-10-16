//! ballistics: deterministic siege-engine projectile sim.
//!
//! Consumer: the siege engine-fire path sources these impacts (the
//! [`SiegeEvent`] stream replays 1:1 into the siege ledger shape); replay
//! proofs compare [`BombardReport`]s. Owner: this module owns flight,
//! impact order and wall state; `engine-destruction` owns every break.
//! Units: [`Fixed`] Q16 metres/seconds, ticks at 60 Hz, f32 impulse only
//! across the destruction call boundary. Determinism: integer flight, sorted
//! `(tick, engine)` impacts, index-order chunk picks; same config replays
//! bit-identical. Errors: [`BallisticError`] at every boundary.
//! Budget: O(sections + in-flight) integer work per tick; heap moves only
//! on impact ticks through the destruction public API, never on quiet ticks.

use engine_arena::seed as arena_seed;
use engine_core::fixed::Fixed;
use engine_core::hash::FNV64_OFFSET;
use engine_destruction::bake::BREAKING_THRESHOLD;
use engine_destruction::bonded::{
    Bond, BondedCollection, BondedDesc, BreakError, BreakEvent, Chunk3,
};
use engine_destruction::budget::MAX_PER_EVENT;
use engine_destruction::shards3d::in_contact;
use engine_destruction::spread::{
    spread_cracks, strain_to_fixed, SpreadBond, SpreadError, SpreadEvent, SpreadGraph,
};

use crate::siege::{
    fold_word, SiegeEvent, ENGINE_EVERY_TICKS, SIEGE_ENGINES, SIEGE_MAX_TICKS, WALL_SECTIONS,
};

/// Per-tick impact applications; the destruction per-event cap, reused not mirrored.
pub const MAX_IMPACTS_PER_TICK: usize = MAX_PER_EVENT as usize;
/// Fire stagger between engines; the WD4 ledger cadence this sim sources.
pub const ENGINE_STAGGER_TICKS: u32 = 37;
/// Wall plane distance from the engine line, metres.
pub const WALL_RANGE_M: i64 = 60;
/// Wall section box half-extent, metres (4 m course per section).
pub const SECTION_HALF_M: f32 = 2.0;
/// Baked chunks per wall section.
pub const SECTION_PIECES: usize = 8;
/// Bond strain limit per wall bond; tuned near one direct-hit strain so
/// cracks extend about one ring per impact through the spread flood.
pub const WALL_BOND_STRAIN: f32 = 700.0;
/// Stone shot mass, kg.
pub const SHOT_MASS_KG: f32 = 90.0;
/// Flight integration guard, ticks (~10 s; never reached, bounds the loop).
pub const MAX_FLIGHT_TICKS: u32 = 600;
/// Pre-sized in-flight impact queue; bounds steady-state memory up front.
pub const IN_FLIGHT_CAP: usize = 1024;
/// Base-course anchor tolerance, metres (the destruction crate-helper convention).
pub const ANCHOR_TOL_M: f32 = 1e-3;
/// Report hash domain tag for fired impacts.
const TAG_FIRED: u64 = 0xB411_0001;
/// Report hash domain tag for wall breaches.
const TAG_BREACHED: u64 = 0xB411_0002;

/// Fixed-point muzzle height, 3 m.
const MUZZLE_Y: Fixed = Fixed::from_bits(196_608);
/// Fixed-point muzzle velocity, 55 m/s horizontal.
const MUZZLE_VX: Fixed = Fixed::from_bits(3_604_480);
/// Fixed-point muzzle velocity, 4 m/s up.
const MUZZLE_VY: Fixed = Fixed::from_bits(262_144);
/// Fixed-point gravity, ~9.8 m/s^2.
const GRAVITY: Fixed = Fixed::from_bits(642_252);
/// Fixed-point wall plane, 60 m.
const WALL_X: Fixed = Fixed::from_bits(3_932_160);
/// Volley velocity step per queued shot, 3 m/s horizontal.
const VOLLEY_VX_STEP: Fixed = Fixed::from_bits(196_608);
/// Volley velocity step per queued shot, 1 m/s up.
const VOLLEY_VY_STEP: Fixed = Fixed::from_bits(65_536);
/// Volley jitter quantum, ~0.02 m/s per bit step.
const JITTER_Q: i64 = 1_311;
/// Wall top, metres; shots clearing it are overs, not hits.
const WALL_TOP_M: f32 = 4.0;

/// Bombardment configuration: one scripted siege shoot.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct BallisticConfig {
    /// Master seed; expanded into per-engine shot streams and wall bakes.
    pub seed: u64,
    /// Fixed steps, `1..=SIEGE_MAX_TICKS` (the WD4 replay bound, reused).
    pub ticks: u32,
}

impl BallisticConfig {
    /// Range-check every field; out-of-range fails with the culprit row.
    pub fn validate(self) -> Result<(), BallisticError> {
        if self.ticks == 0 || self.ticks > SIEGE_MAX_TICKS {
            return Err(BallisticError::BadConfig("ticks"));
        }
        Ok(())
    }
}

/// Ballistic failures: bounded diagnostics, never panics.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BallisticError {
    /// Config row outside its range, row name carried.
    BadConfig(&'static str),
    /// Wall build or impact rejected by the destruction collection.
    Break(BreakError),
    /// Crack flood rejected by the spread graph.
    Spread(SpreadError),
    /// Wall-entity sampling rejected by the interest path.
    Interest(engine_net::interest::InterestError),
    /// The bombardment already authored `config.ticks` ticks.
    MatchOver,
    /// Volley engine `>= SIEGE_ENGINES` or section `>= WALL_SECTIONS`.
    BadTarget,
}

impl core::fmt::Display for BallisticError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::BadConfig(row) => write!(f, "bad ballistic config ({row})"),
            Self::Break(err) => write!(f, "ballistic wall error ({err})"),
            Self::Spread(err) => write!(f, "ballistic spread error ({err})"),
            Self::Interest(err) => write!(f, "ballistic interest error ({err})"),
            Self::MatchOver => write!(f, "bombardment already ran its configured ticks"),
            Self::BadTarget => write!(f, "volley names an unknown engine or section"),
        }
    }
}

impl std::error::Error for BallisticError {}

impl From<BreakError> for BallisticError {
    fn from(err: BreakError) -> Self {
        Self::Break(err)
    }
}

impl From<SpreadError> for BallisticError {
    fn from(err: SpreadError) -> Self {
        Self::Spread(err)
    }
}

impl From<engine_net::interest::InterestError> for BallisticError {
    fn from(err: engine_net::interest::InterestError) -> Self {
        Self::Interest(err)
    }
}

/// One validated wall section: the destruction collection, its spread graph in
/// canonical order, both index maps between the two bond orders, and the
/// cumulative break ledger the [`SiegeEvent`] stream reports from.
struct WallSection {
    wall: BondedCollection,
    graph: SpreadGraph,
    bonded_to_spread: Vec<u32>,
    spread_to_bonded: Vec<u32>,
    freed_total: u32,
    breached: bool,
    spread_breaks: u32,
}

impl WallSection {
    /// Bake one wall course: [`fracture3`](engine_destruction::bake3d) chunks
    /// (the bake path) bound by contact bonds with the bottom
    /// course anchored, plus the canonical spread graph over the same pairs.
    fn bake(
        cx: f32,
        cy: f32,
        cz: f32,
        half: f32,
        pieces: usize,
        seed: u64,
    ) -> Result<Self, BallisticError> {
        let chunks =
            engine_destruction::bake3d::fracture3(cx, cy, cz, half, half, half, pieces, seed);
        let mut bonds = Vec::new();
        for a in 0..chunks.len() {
            for b in (a + 1)..chunks.len() {
                if in_contact(&chunks[a], &chunks[b]) {
                    bonds.push(Bond {
                        a: a as u32,
                        b: b as u32,
                        breaking_strain: WALL_BOND_STRAIN,
                    });
                }
            }
        }
        let bottom = chunks
            .iter()
            .map(|c| c.y - c.hy)
            .reduce(f32::min)
            .unwrap_or(cy - half);
        let mut anchors = Vec::new();
        for (i, c) in chunks.iter().enumerate() {
            if (c.y - c.hy) - bottom <= ANCHOR_TOL_M {
                anchors.push(i as u32);
            }
        }
        let desc = BondedDesc {
            chunks,
            bonds,
            anchors: anchors.clone(),
            break_threshold: BREAKING_THRESHOLD,
        };
        desc.validate()?;
        let wall = BondedCollection::new(&desc)?;
        // The spread graph over the same contact pairs: Q16 thresholds carry
        // the wall strain limit; canonical order erases insertion order, so
        // both index maps are rebuilt by endpoint match at bake time only.
        let spread_bonds: Vec<SpreadBond> = desc
            .bonds
            .iter()
            .map(|bond| SpreadBond {
                a: bond.a,
                b: bond.b,
                threshold_q16: (WALL_BOND_STRAIN * 65_536.0) as i64,
            })
            .collect();
        let graph = SpreadGraph::new(desc.chunks.len(), spread_bonds, anchors)?;
        let mut bonded_to_spread = vec![0u32; desc.bonds.len()];
        let mut spread_to_bonded = vec![0u32; desc.bonds.len()];
        for (spread_id, spread_bond) in graph.bonds().iter().enumerate() {
            let pair = (
                spread_bond.a.min(spread_bond.b),
                spread_bond.a.max(spread_bond.b),
            );
            for (bonded_id, bond) in desc.bonds.iter().enumerate() {
                if (bond.a.min(bond.b), bond.a.max(bond.b)) == pair {
                    bonded_to_spread[bonded_id] = spread_id as u32;
                    spread_to_bonded[spread_id] = bonded_id as u32;
                    break;
                }
            }
        }
        Ok(Self {
            wall,
            graph,
            bonded_to_spread,
            spread_to_bonded,
            freed_total: 0,
            breached: false,
            spread_breaks: 0,
        })
    }
}

/// One resolved shot waiting to land, in `(impact_tick, engine)` order.
#[derive(Clone, Copy, PartialEq, Debug)]
struct PendingImpact {
    impact_tick: u32,
    engine: u8,
    section: u8,
    chunk: usize,
    impulse: f32,
}

/// One scripted volley order waiting for tick budget.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct VolleyOrder {
    engine: u8,
    section: u8,
    remaining: u32,
    step: u32,
}

/// Outcome of one authored tick: impacts applied in sorted order.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct TickImpacts {
    /// Fixed tick just authored.
    pub tick: u32,
    /// Engines fired this tick (schedule plus volleys).
    pub fired: u8,
    /// Impacts applied this tick, in `(tick, engine)` order.
    pub applied: u8,
    /// Shots that missed the wall face this tick (overs and shorts).
    pub duds: u8,
}

/// Completed-bombardment report: everything a replay must reproduce.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct BombardReport {
    /// Master seed the bombardment ran under.
    pub seed: u64,
    /// Ticks authored.
    pub ticks: u32,
    /// Converged bombardment hash (every client must agree).
    pub hash: u64,
    /// Shots fired (schedule plus volleys).
    pub shots: u32,
    /// Impacts applied to wall faces.
    pub impacts: u32,
    /// Shots that missed the wall face.
    pub duds: u32,
    /// Authoritative break events drained (bonds plus frees).
    pub breaks: u32,
    /// Breaks the spread flood extended past the direct hit.
    pub spread_breaks: u32,
    /// Sections with at least one freed chunk (a hole opened).
    pub breaches: u32,
    /// Wall state hash per section, in section order.
    pub walls: [u64; WALL_SECTIONS],
    /// Freed chunks per section, in section order.
    pub freed: [u32; WALL_SECTIONS],
}

impl core::fmt::Display for BombardReport {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "BOMBARD seed={:016x} ticks={} hash={:016x} shots={} impacts={} duds={} breaks={} breaches={}",
            self.seed,
            self.ticks,
            self.hash,
            self.shots,
            self.impacts,
            self.duds,
            self.breaks,
            self.breaches,
        )
    }
}

/// Section an engine's `n`th shot aims at: engines pair up over the wall so
/// neighbouring engines never pile one section.
#[must_use]
pub const fn section_for(engine: usize, shot: u32) -> usize {
    (engine + 4 * ((shot % 2) as usize)) % WALL_SECTIONS
}

/// Fold one ledger event into the report hash with its domain tag.
fn fold_ledger_event(hash: u64, event: &SiegeEvent) -> u64 {
    match *event {
        SiegeEvent::EngineFired {
            tick,
            engine,
            section,
            damage,
        } => {
            let hash = fold_word(hash, TAG_FIRED);
            let hash = fold_word(hash, (u64::from(tick) << 16) | u64::from(section));
            fold_word(hash, (u64::from(engine) << 32) | u64::from(damage))
        }
        SiegeEvent::WallBreached { tick, section } => {
            let hash = fold_word(hash, TAG_BREACHED);
            fold_word(hash, (u64::from(tick) << 16) | u64::from(section))
        }
    }
}

/// Scripted bombardment over eight wall courses: fixed-point flight from
/// four engines, impacts in sorted `(tick, engine)` order, cracks routed
/// impact to flood and back, per-tick caps, and the impact
/// ledger reported as reused siege [`SiegeEvent`]s (never a mirrored type).
pub struct Bombardment {
    config: BallisticConfig,
    sections: Vec<WallSection>,
    section_cz: [f32; WALL_SECTIONS],
    streams: [u64; SIEGE_ENGINES],
    shot_no: [u32; SIEGE_ENGINES],
    in_flight: Vec<PendingImpact>,
    volleys: Vec<VolleyOrder>,
    events: Vec<SiegeEvent>,
    hash: u64,
    shots: u32,
    impacts: u32,
    duds: u32,
    breaks: u32,
    fired_this_tick: usize,
    duds_today: u8,
    cursor: u32,
}

impl Bombardment {
    /// Open a bombardment: validates the config, bakes eight wall courses
    /// off non-aliasing seed streams and pre-sizes every per-tick buffer so
    /// quiet ticks never touch the allocator.
    pub fn new(config: BallisticConfig) -> Result<Self, BallisticError> {
        config.validate()?;
        // `ballista_one` fires through `SOCKET_muzzle` at the 60 m wall plane.
        let _ =
            crate::siege::siege_field().map_err(|_| BallisticError::BadConfig("siege_field"))?;
        let wall_seeds = arena_seed::expand(config.seed ^ 0x5EED_B411, WALL_SECTIONS);
        let shot_seeds = arena_seed::expand(config.seed ^ 0xB411_15ED, SIEGE_ENGINES);
        let mut sections = Vec::with_capacity(WALL_SECTIONS);
        let mut section_cz = [0.0f32; WALL_SECTIONS];
        for s in 0..WALL_SECTIONS {
            let cz = (s as f32 - 3.5) * (SECTION_HALF_M * 2.0);
            section_cz[s] = cz;
            sections.push(WallSection::bake(
                WALL_RANGE_M as f32,
                SECTION_HALF_M,
                cz,
                SECTION_HALF_M,
                SECTION_PIECES,
                wall_seeds[s],
            )?);
        }
        let mut streams = [0u64; SIEGE_ENGINES];
        streams.copy_from_slice(&shot_seeds);
        Ok(Self {
            config,
            sections,
            section_cz,
            streams,
            shot_no: [0u32; SIEGE_ENGINES],
            in_flight: Vec::with_capacity(IN_FLIGHT_CAP),
            volleys: Vec::new(),
            events: Vec::with_capacity(config.ticks as usize / 20 + 64),
            hash: FNV64_OFFSET,
            shots: 0,
            impacts: 0,
            duds: 0,
            breaks: 0,
            fired_this_tick: 0,
            duds_today: 0,
            cursor: 0,
        })
    }

    /// Bombardment configuration.
    #[must_use]
    pub const fn config(&self) -> BallisticConfig {
        self.config
    }

    /// Next tick to author.
    #[must_use]
    pub const fn cursor(&self) -> u32 {
        self.cursor
    }

    /// Impact ledger in `(tick, engine)` order (reused WD4 event type).
    #[must_use]
    pub fn events(&self) -> &[SiegeEvent] {
        &self.events
    }

    /// Shots fired so far (schedule plus volleys).
    #[must_use]
    pub const fn shots(&self) -> u32 {
        self.shots
    }

    /// Shots still airborne: fired but neither landed nor dudded. A replay
    /// is complete only when this drains to zero past the last flight.
    #[must_use]
    pub fn in_flight_len(&self) -> usize {
        self.in_flight.len()
    }

    /// Impacts applied so far.
    #[must_use]
    pub const fn impacts(&self) -> u32 {
        self.impacts
    }

    /// Wall state hash per section, in section order.
    #[must_use]
    pub fn wall_hashes(&self) -> [u64; WALL_SECTIONS] {
        let mut out = [0u64; WALL_SECTIONS];
        for (index, section) in self.sections.iter().enumerate() {
            out[index] = section.wall.state_hash();
        }
        out
    }

    /// Freed chunks per section, in section order.
    #[must_use]
    pub fn freed_chunks(&self) -> [u32; WALL_SECTIONS] {
        let mut out = [0u32; WALL_SECTIONS];
        for (index, section) in self.sections.iter().enumerate() {
            out[index] = section.freed_total;
        }
        out
    }

    /// Queue a scripted volley: `shots` shots from one engine at one section
    /// with ascending velocity steps (later shots fly flatter and land
    /// earlier, so the queue must sort them). Firing obeys the per-tick
    /// cap; excess waits in FIFO order. Returns shots queued.
    pub fn volley(&mut self, engine: u8, section: u8, shots: u32) -> Result<u32, BallisticError> {
        if (engine as usize) >= SIEGE_ENGINES || (section as usize) >= WALL_SECTIONS {
            return Err(BallisticError::BadTarget);
        }
        if shots == 0 {
            return Ok(0);
        }
        self.volleys.push(VolleyOrder {
            engine,
            section,
            remaining: shots,
            step: 0,
        });
        Ok(shots)
    }

    /// Interest wall rows for the replication sample: one entity per section at
    /// its lattice position; the combat word folds the wall hash with the
    /// tick so any break marks the row dirty and every client converges.
    #[must_use]
    pub fn wall_entities(
        &self,
        tick: u32,
    ) -> [engine_net::interest::InterestEntity; WALL_SECTIONS] {
        let mut out = [engine_net::interest::InterestEntity {
            id: 0,
            x_mm: 0,
            z_mm: 0,
            combat: 0,
        }; WALL_SECTIONS];
        for (s, entity) in out.iter_mut().enumerate() {
            let combat =
                (self.sections[s].wall.state_hash() ^ ((tick as u64) << 1) ^ (s as u64)) as u32;
            *entity = engine_net::interest::InterestEntity {
                id: s as u32,
                x_mm: (WALL_RANGE_M * 1000) as i32,
                z_mm: (self.section_cz[s] * 1000.0) as i32,
                combat,
            };
        }
        out
    }

    /// Author one fixed tick: scheduled fires, queued volleys up to the
    /// per-tick cap, due impacts in `(tick, engine)` order with overflow deferred one
    /// tick (still sorted), then the hash fold over walls and new events.
    pub fn tick(&mut self) -> Result<TickImpacts, BallisticError> {
        if self.cursor >= self.config.ticks {
            return Err(BallisticError::MatchOver);
        }
        let tick = self.cursor;
        self.fired_this_tick = 0;
        let mut fired = 0u8;
        // Scheduled engines: at most one engine per tick by the stagger.
        for engine in 0..SIEGE_ENGINES {
            if (tick + engine as u32 * ENGINE_STAGGER_TICKS).is_multiple_of(ENGINE_EVERY_TICKS) {
                let n = self.shot_no[engine];
                self.shot_no[engine] += 1;
                let section = section_for(engine, n);
                self.fire_shot(engine, section, MUZZLE_VX, MUZZLE_VY, tick);
                fired += 1;
            }
        }
        // Queued volleys drain FIFO into the remaining tick budget.
        let mut order = 0usize;
        while order < self.volleys.len() && self.fired_this_tick < MAX_IMPACTS_PER_TICK {
            let step = self.volleys[order].step;
            let engine = self.volleys[order].engine as usize;
            let section = self.volleys[order].section as usize;
            let vx =
                MUZZLE_VX.wrapping_add(VOLLEY_VX_STEP.wrapping_mul(Fixed::from_int(step as i64)));
            let vy =
                MUZZLE_VY.wrapping_add(VOLLEY_VY_STEP.wrapping_mul(Fixed::from_int(step as i64)));
            self.fire_shot(engine, section, vx, vy, tick);
            fired += 1;
            self.volleys[order].remaining -= 1;
            self.volleys[order].step += 1;
            if self.volleys[order].remaining == 0 {
                self.volleys.remove(order);
            } else {
                order += 1;
            }
        }
        // Due impacts land in queue order; the fire paths above already hold
        // the cap, so overflow here only defers one tick (still sorted).
        let mut applied = 0u8;
        while let Some(front) = self.in_flight.first() {
            if front.impact_tick != tick || applied as usize >= MAX_IMPACTS_PER_TICK {
                break;
            }
            let impact = self.in_flight.remove(0);
            self.apply_impact(&impact, tick)?;
            applied += 1;
        }
        // Deferred overflow: anything still due today waits one tick.
        for pending in self.in_flight.iter_mut() {
            if pending.impact_tick == tick {
                pending.impact_tick = tick + 1;
            } else {
                break;
            }
        }
        // The fire paths never queue duds; dud ticks are counted at fire.
        let duds = self.duds_today;
        self.duds_today = 0;
        self.fold_tick(tick);
        self.cursor += 1;
        Ok(TickImpacts {
            tick,
            fired,
            applied,
            duds,
        })
    }

    /// Drive the bombardment to its configured end and report.
    pub fn run(mut self) -> Result<BombardReport, BallisticError> {
        while self.cursor < self.config.ticks {
            self.tick()?;
        }
        Ok(self.report())
    }

    /// Completed-bombardment report for the ticks authored so far.
    #[must_use]
    pub fn report(&self) -> BombardReport {
        BombardReport {
            seed: self.config.seed,
            ticks: self.cursor,
            hash: self.hash,
            shots: self.shots,
            impacts: self.impacts,
            duds: self.duds,
            breaks: self.breaks,
            spread_breaks: self.sections.iter().map(|s| s.spread_breaks).sum(),
            breaches: self.sections.iter().filter(|s| s.breached).count() as u32,
            walls: self.wall_hashes(),
            freed: self.freed_chunks(),
        }
    }

    /// Fire one shot: jitter the muzzle off its seeded stream, integrate the
    /// Fixed-point flight, and either queue the sorted impact or count a dud
    /// (over the wall top or short of the face). Never fails: geometry is
    /// validated at construction and the queue is pre-sized.
    fn fire_shot(&mut self, engine: usize, section: usize, vx0: Fixed, vy0: Fixed, tick: u32) {
        self.fired_this_tick += 1;
        self.shots += 1;
        let word = arena_seed::next(&mut self.streams[engine]);
        let dvx = Fixed::from_bits(((word % 11) as i64 - 5) * JITTER_Q);
        let dvy = Fixed::from_bits((((word >> 7) % 41) as i64 - 20) * JITTER_Q);
        let dz = (((word >> 13) % 17) as i64 - 8) * JITTER_Q;
        let (vx, mut vy) = (vx0.wrapping_add(dvx), vy0.wrapping_add(dvy));
        // One stepper: combat semi-implicit Euler (velocity before position).
        let tables = engine_combat::projectile::FlightTables {
            gravity_mps2: GRAVITY,
            linear_drag_per_s: Fixed::ZERO,
            quad_drag_per_m: Fixed::ZERO,
        };
        let mut round = engine_combat::projectile::Round {
            pos: [Fixed::ZERO, MUZZLE_Y, Fixed::ZERO],
            vel: [vx, vy, Fixed::ZERO],
            age_ticks: 0,
            ttl_ticks: MAX_FLIGHT_TICKS,
            alive: true,
        };
        let caster = engine_combat::projectile::NullCaster;
        let mut flight = 0u32;
        let mut landed = false;
        while flight < MAX_FLIGHT_TICKS {
            flight += 1;
            if engine_combat::projectile::step_round(&mut round, &tables, &caster).is_err() {
                break;
            }
            vy = round.vel[1];
            if round.pos[0].to_bits() >= WALL_X.to_bits() {
                landed = true;
                break;
            }
            if round.pos[1].to_bits() <= 0 {
                break;
            }
        }
        let y = round.pos[1];
        if !landed {
            self.duds += 1;
            self.duds_today += 1;
            return;
        }
        let y_m = y.to_bits() as f32 / Fixed::SCALE as f32;
        if !(0.0..=WALL_TOP_M).contains(&y_m) {
            // Cleared the parapet: an over, not a hit on the top course.
            self.duds += 1;
            self.duds_today += 1;
            return;
        }
        let z_m = self.section_cz[section] + dz as f32 / Fixed::SCALE as f32;
        let chunk = pick_chunk(self.sections[section].wall.chunks(), y_m, z_m);
        // Impact speed in m/s is a power-of-two scale of exact integer bits,
        // so the f32 impulse is bit-deterministic across runs.
        let speed_mps = vx
            .wrapping_mul(vx)
            .wrapping_add(vy.wrapping_mul(vy))
            .sqrt()
            .to_bits() as f32
            / Fixed::SCALE as f32;
        let impulse = speed_mps * SHOT_MASS_KG;
        let impact = PendingImpact {
            impact_tick: tick + flight,
            engine: engine as u8,
            section: section as u8,
            chunk,
            impulse,
        };
        let pos = self
            .in_flight
            .binary_search_by(|queued| {
                (queued.impact_tick, queued.engine).cmp(&(impact.impact_tick, impact.engine))
            })
            .unwrap_or_else(|at| at);
        self.in_flight.insert(pos, impact);
    }

    /// Apply one landed impact and append its reused WD4 ledger rows.
    fn apply_impact(&mut self, impact: &PendingImpact, tick: u32) -> Result<(), BallisticError> {
        self.impacts += 1;
        let section = &mut self.sections[impact.section as usize];
        let (damage, breaks_added, breached_now) = Self::apply_to_section(section, impact, tick)?;
        self.breaks += breaks_added;
        self.events.push(SiegeEvent::EngineFired {
            tick,
            engine: impact.engine,
            section: impact.section,
            damage,
        });
        if breached_now {
            self.events.push(SiegeEvent::WallBreached {
                tick,
                section: impact.section,
            });
        }
        Ok(())
    }
    /// Drive one landed impact through a wall section: the direct hit, the
    /// spread flood seeded from the hardest-snapped bond with its extensions applied
    /// back through the collection, then one rubble step. Returns the ledger damage (0
    /// once rubble has nothing left to break, like the siege fallen-section
    /// rule), breaks added, and whether this impact opened the first hole.
    fn apply_to_section(
        section: &mut WallSection,
        impact: &PendingImpact,
        tick: u32,
    ) -> Result<(u32, u32, bool), BallisticError> {
        section
            .wall
            .apply_impact(impact.chunk, impact.impulse, tick)?;
        let mut fresh = section.wall.drain_events();
        // Seed the spread flood from the hardest-snapped bond.
        let mut snapped: Vec<u32> = Vec::new();
        let mut seed_strain = 0.0f32;
        for event in &fresh {
            if let BreakEvent::BondBroken { bond, strain, .. } = *event {
                snapped.push(bond);
                if strain > seed_strain {
                    seed_strain = strain;
                }
            }
        }
        if let Some(first) = snapped.first() {
            let hit_spread = section.bonded_to_spread[*first as usize];
            if let Ok(seed_q16) = strain_to_fixed(seed_strain) {
                if let Ok(flood) =
                    spread_cracks(&section.graph, hit_spread as usize, seed_q16, tick)
                {
                    let mut extended = Vec::new();
                    for event in &flood {
                        if let SpreadEvent::BondBroken { bond, strain, .. } = *event {
                            let bonded = section.spread_to_bonded[bond as usize];
                            if !section.wall.is_bond_broken(bonded as usize) {
                                section.wall.add_strain(bonded as usize, strain, tick)?;
                                extended.push(bonded);
                            }
                        }
                    }
                    let more = section.wall.drain_events();
                    for event in &more {
                        if let BreakEvent::BondBroken { bond, .. } = *event {
                            if extended.contains(&bond) {
                                section.spread_breaks += 1;
                            }
                        }
                    }
                    fresh.extend(more);
                }
            }
        }
        let damage = fresh.len() as u32;
        let mut breaks_added = damage;
        for event in &fresh {
            if matches!(event, BreakEvent::ChunkFreed { .. }) {
                section.freed_total += 1;
            }
        }
        // Rubble integrates on impact ticks only; quiet ticks stay alloc-free.
        section.wall.step(1.0 / 60.0);
        for event in section.wall.drain_events() {
            if matches!(event, BreakEvent::ChunkFreed { .. }) {
                section.freed_total += 1;
            }
            breaks_added += 1;
        }
        let breached_now = !section.breached && section.freed_total > 0;
        section.breached = section.breached || breached_now;
        Ok((damage, breaks_added, breached_now))
    }

    /// Fold one authored tick over the section hashes and the tick's events.
    fn fold_tick(&mut self, tick: u32) {
        let mut hash = fold_word(self.hash, u64::from(tick));
        for section in &self.sections {
            hash = fold_word(hash, section.wall.state_hash());
        }
        for event in self.events.iter().filter(|event| event.tick() == tick) {
            hash = fold_ledger_event(hash, event);
        }
        self.hash = hash;
    }
}

/// Pick the chunk a shot strikes: first box containing the impact point in
/// index order, else the nearest center in index order (strict `<` keeps
/// the first on ties, so the pick is total and stable).
fn pick_chunk(chunks: &[Chunk3], y_m: f32, z_m: f32) -> usize {
    for (index, chunk) in chunks.iter().enumerate() {
        if (y_m - chunk.y).abs() <= chunk.hy && (z_m - chunk.z).abs() <= chunk.hz {
            return index;
        }
    }
    let mut best = 0usize;
    let mut best_d = f32::INFINITY;
    for (index, chunk) in chunks.iter().enumerate() {
        let d = (y_m - chunk.y).abs() + (z_m - chunk.z).abs();
        if d < best_d {
            best_d = d;
            best = index;
        }
    }
    best
}

/// Play a whole bombardment and report: the acceptance entry — a scripted
/// shoot for `config.ticks` ticks that replays exactly (same config twice
/// gives the same report, so every client converges on the same walls).
pub fn run_bombardment(config: BallisticConfig) -> Result<BombardReport, BallisticError> {
    config.validate()?;
    Bombardment::new(config)?.run()
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_core::alloc_probe;
    use engine_net::interest::{
        ConnectionInterest, InterestCfg, ON3_CAP_BPS, SIEGE_BUDGET_BYTES, SNAPSHOT_EVERY_TICKS,
        STARVE_TICKS,
    };

    fn config() -> BallisticConfig {
        BallisticConfig {
            seed: 99,
            ticks: 7_200,
        }
    }

    fn small_config() -> BallisticConfig {
        BallisticConfig {
            seed: 7,
            ticks: 400,
        }
    }

    #[test]
    fn rejects_bad_configs_and_targets() {
        assert!(Bombardment::new(config()).is_ok());
        assert!(matches!(
            Bombardment::new(BallisticConfig { seed: 1, ticks: 0 }),
            Err(BallisticError::BadConfig("ticks"))
        ));
        assert!(matches!(
            Bombardment::new(BallisticConfig {
                seed: 1,
                ticks: SIEGE_MAX_TICKS + 1
            }),
            Err(BallisticError::BadConfig("ticks"))
        ));
        assert!(matches!(
            run_bombardment(BallisticConfig { seed: 1, ticks: 0 }),
            Err(BallisticError::BadConfig("ticks"))
        ));
        let mut bombard = Bombardment::new(small_config()).unwrap();
        assert!(matches!(
            bombard.volley(SIEGE_ENGINES as u8, 0, 4),
            Err(BallisticError::BadTarget)
        ));
        assert!(matches!(
            bombard.volley(0, WALL_SECTIONS as u8, 4),
            Err(BallisticError::BadTarget)
        ));
        assert_eq!(bombard.volley(0, 0, 0).unwrap(), 0);
    }

    #[test]
    fn ticking_past_the_end_fails_without_advancing() {
        let mut bombard = Bombardment::new(BallisticConfig { seed: 3, ticks: 4 }).unwrap();
        for _ in 0..4 {
            bombard.tick().unwrap();
        }
        assert_eq!(bombard.cursor(), 4);
        assert_eq!(bombard.tick(), Err(BallisticError::MatchOver));
        assert_eq!(bombard.cursor(), 4);
    }

    #[test]
    fn scripted_bombardment_breaks_same_chunks_twice() {
        let first = run_bombardment(config()).unwrap();
        let second = run_bombardment(config()).unwrap();
        assert_eq!(first, second, "bombardment must replay exactly");
        println!("{first}");
        // The event stream behind the report: same script, same rows.
        let mut bombard = Bombardment::new(config()).unwrap();
        while bombard.cursor() < config().ticks {
            bombard.tick().unwrap();
        }
        assert_eq!(bombard.report(), first);
        let stream = bombard.events();
        // The shoot does real work: shots fired, impacts landed, walls scarred.
        // Tail flights are still airborne at the last tick, never lost.
        assert!(first.shots > 0, "engines must fire");
        assert_eq!(
            first.shots,
            first.impacts + first.duds + bombard.in_flight_len() as u32
        );
        assert!(first.impacts > 0, "shots must reach the wall face");
        assert!(first.breaks > 0, "impacts must break bonds");
        assert!(
            first.spread_breaks > 0,
            "the DX3 flood must extend cracks past the direct hit"
        );
        let freed: u32 = first.freed.iter().sum();
        assert!(freed > 0, "bombardment must free chunks");
        assert!(first.breaches > 0, "two minutes of stone must open holes");
        // Impacts apply in sorted (tick, engine) order.
        let mut last = (0u32, 0u8);
        let mut fired_seen = 0u32;
        for event in stream {
            if let SiegeEvent::EngineFired { tick, engine, .. } = *event {
                if fired_seen > 0 {
                    assert!(
                        (tick, engine) >= last,
                        "impacts out of order: {last:?} then ({tick}, {engine})"
                    );
                }
                last = (tick, engine);
                fired_seen += 1;
            }
        }
        assert!(fired_seen > 0);
        // Ledger consistency: one fired row per impact, one breach row per
        // breached section, damage sums to exactly the impact-drained breaks
        // (rubble settling after the hit drains outside the row's damage).
        let fired_rows = stream
            .iter()
            .filter(|e| matches!(e, SiegeEvent::EngineFired { .. }))
            .count();
        assert_eq!(fired_rows as u32, first.impacts);
        let mut breach_rows = [0u32; WALL_SECTIONS];
        let mut damage_sum = 0u32;
        for event in stream {
            match *event {
                SiegeEvent::EngineFired { damage, .. } => damage_sum += damage,
                SiegeEvent::WallBreached { section, .. } => {
                    breach_rows[section as usize] += 1;
                }
            }
        }
        assert!(damage_sum <= first.breaks);
        for (s, (breaches, freed)) in breach_rows.iter().zip(first.freed.iter()).enumerate() {
            assert!(*breaches <= 1, "section {s} breached twice");
            if *breaches == 1 {
                assert!(*freed > 0, "breach without a freed chunk");
            }
            if *freed > 0 {
                assert_eq!(*breaches, 1, "hole without a breach row");
            }
        }
    }

    #[test]
    fn wall_break_converges_on_all_clients() {
        // Server authors tick by tick with volleys staged; two clients replay
        // the same script; all three converge on walls, events and hash.
        let script = |bombard: &mut Bombardment| {
            bombard.volley(0, 0, 12).unwrap();
            bombard.volley(2, 6, 12).unwrap();
        };
        let mut server = Bombardment::new(small_config()).unwrap();
        script(&mut server);
        let server_report = server.run().unwrap();
        let mut client_a = Bombardment::new(small_config()).unwrap();
        script(&mut client_a);
        let mut client_b = Bombardment::new(small_config()).unwrap();
        script(&mut client_b);
        let report_a = client_a.run().unwrap();
        let report_b = client_b.run().unwrap();
        assert_eq!(server_report, report_a);
        assert_eq!(server_report, report_b);
        assert_eq!(server_report.hash, report_a.hash);
    }

    #[test]
    fn dedicated_ticks_match_the_fast_run() {
        // The tick-by-tick authoring and the single-evaluation run agree, so
        // the replay the clients converge on is what the server authors.
        let mut ticked = Bombardment::new(small_config()).unwrap();
        ticked.volley(1, 3, 20).unwrap();
        let mut outcomes = Vec::new();
        while ticked.cursor() < small_config().ticks {
            outcomes.push(ticked.tick().unwrap());
        }
        let ticked_report = ticked.report();
        let mut fast = Bombardment::new(small_config()).unwrap();
        fast.volley(1, 3, 20).unwrap();
        let fast_report = fast.run().unwrap();
        assert_eq!(ticked_report, fast_report);
        // Per-tick outcomes never exceed the per-tick cap.
        for outcome in &outcomes {
            assert!(
                (outcome.applied as usize) <= MAX_IMPACTS_PER_TICK,
                "tick {} applied {}",
                outcome.tick,
                outcome.applied
            );
        }
    }

    #[test]
    fn overcap_volley_defers_deterministically() {
        let script = |bombard: &mut Bombardment| {
            bombard
                .volley(0, 1, MAX_IMPACTS_PER_TICK as u32 + 10)
                .unwrap();
        };
        let mut first = Bombardment::new(small_config()).unwrap();
        script(&mut first);
        let mut second = Bombardment::new(small_config()).unwrap();
        script(&mut second);
        let mut max_applied = 0u8;
        while first.cursor() < small_config().ticks {
            let outcome = first.tick().unwrap();
            max_applied = max_applied.max(outcome.applied);
        }
        let second_report = second.run().unwrap();
        assert_eq!(first.report(), second_report);
        assert!(
            max_applied as usize <= MAX_IMPACTS_PER_TICK,
            "cap held: {max_applied}"
        );
        assert_eq!(
            first.report().shots,
            first.report().impacts + first.report().duds + first.in_flight_len() as u32,
            "every queued shot is accounted"
        );
    }

    #[test]
    fn fallen_sections_take_zero_damage() {
        // Pound one section to quiescence (a full volley cycle breaks
        // nothing new), then one more stone: the ledger row lands with
        // damage 0 like the WD4 fallen-section rule.
        let cfg = BallisticConfig {
            seed: 11,
            ticks: 20_000,
        };
        let mut bombard = Bombardment::new(cfg).unwrap();
        let mut quiescent = false;
        for _ in 0..30 {
            let breaks_before = bombard.report().breaks;
            bombard.volley(0, 0, 4).unwrap();
            for _ in 0..200 {
                if bombard.cursor() >= cfg.ticks {
                    break;
                }
                bombard.tick().unwrap();
            }
            if bombard.report().breaks == breaks_before {
                quiescent = true;
                break;
            }
        }
        assert!(quiescent, "section 0 must grind to rubble");
        let events_before = bombard.events().len();
        bombard.volley(0, 0, 1).unwrap();
        let mut probe: Option<SiegeEvent> = None;
        while bombard.cursor() < cfg.ticks && probe.is_none() {
            bombard.tick().unwrap();
            probe = bombard.events()[events_before..]
                .iter()
                .find(|event| matches!(event, SiegeEvent::EngineFired { section: 0, .. }))
                .copied();
        }
        match probe {
            Some(SiegeEvent::EngineFired { damage, .. }) => {
                assert_eq!(damage, 0, "rubble takes no damage");
            }
            _ => panic!("the section-0 probe never landed"),
        }
    }

    #[test]
    fn wall_breaks_replicate_within_on3_budget() {
        // The bombardment runs under replication interest: wall rows sample at the
        // snapshot cadence inside the siege budget with no starved viewer,
        // so breaks converge on every client.
        let cfg = small_config();
        let mut bombard = Bombardment::new(cfg).unwrap();
        bombard.volley(0, 2, 16).unwrap();
        let interest_cfg = InterestCfg::default_64();
        interest_cfg.validate().unwrap();
        let mut conn =
            ConnectionInterest::for_entities(0, WALL_SECTIONS).expect("wall viewer registers");
        conn.register_baseline(&bombard.wall_entities(0))
            .expect("baseline registers");
        let mut out = [u32::MAX; WALL_SECTIONS];
        let mut combat_seen = false;
        while bombard.cursor() < cfg.ticks {
            let outcome = bombard.tick().unwrap();
            if outcome.tick.is_multiple_of(SNAPSHOT_EVERY_TICKS) {
                let entities = bombard.wall_entities(outcome.tick);
                let viewer = entities[0];
                conn.select(
                    &interest_cfg,
                    &entities,
                    0,
                    viewer.x_mm,
                    viewer.z_mm,
                    0,
                    outcome.tick,
                    SIEGE_BUDGET_BYTES,
                    &mut out,
                )
                .expect("wall sample fits the siege budget");
                if bombard.breaks > 0 {
                    combat_seen = true;
                }
            }
        }
        assert!(combat_seen, "breaks must occur under the sample");
        assert!(
            conn.bytes_per_sec(cfg.ticks) <= u64::from(ON3_CAP_BPS),
            "interest over budget"
        );
        assert!(
            conn.starve_max() <= STARVE_TICKS,
            "starve guard fired: {}",
            conn.starve_max()
        );
        // The sampled run converges with a clean replay of the same script.
        let mut resample = Bombardment::new(cfg).unwrap();
        resample.volley(0, 2, 16).unwrap();
        assert_eq!(resample.run().unwrap(), bombard.report());
    }

    #[test]
    fn quiet_ticks_allocate_nothing_at_steady_state() {
        // Calibration: the shared probe must observe a real allocation,
        // otherwise a zero reading would prove nothing.
        let (_, probe) = alloc_probe::measure(|| std::hint::black_box(vec![0u8; 64]));
        assert!(probe.calls >= 1, "alloc probe is blind");

        // Ticks 1..=8 hold no landings (first flight lands near tick 65),
        // so the per-tick path must move the sim with zero heap growth.
        let mut bombard = Bombardment::new(small_config()).unwrap();
        let first = bombard.tick().unwrap();
        assert_eq!(first.tick, 0);
        let queue_cap = bombard.in_flight.capacity();
        let events_cap = bombard.events.capacity();
        let ((), steady) = alloc_probe::measure(|| {
            for _ in 0..8 {
                let outcome = bombard.tick().unwrap();
                std::hint::black_box(outcome);
            }
        });
        assert_eq!(steady.calls, 0, "quiet tick allocated");
        assert_eq!(bombard.impacts, 0, "window must hold no landings");
        assert_eq!(bombard.in_flight.capacity(), queue_cap);
        assert_eq!(bombard.events.capacity(), events_cap);
    }
}
