//! 64-player server tick budget: 64 fighters plus 500 AI soldiers.
//! Consumer: the perf report reads `ServerReport::summary`; the `--perf`
//! gate runs the ignored p99 proof. Units: millimetres, ticks at 60 Hz,
//! bytes, milliseconds (presentation only, never hashed). Determinism: the
//! hash path is integer `+ - *` with stable index order only, no map
//! iteration, no wall clock. Errors: `ServerBudgetError`, never panics.
//! Budget: dedicated tick p99 under 4 ms with interest filtering on.
//! Crate edge: snapshot cadence plus the starve bar are owned by
//! `engine-net::interest` (game/siege.rs follows the same pattern), never
//! mirrored here.

use engine_core::hash::FNV64_PRIME;
use engine_net::interest::{SNAPSHOT_EVERY_TICKS, STARVE_TICKS};
use std::fmt::{self, Display, Formatter};
use std::hint::black_box;
use std::time::Instant;

/// Start state of the server-budget folds. The recorded digests pin it, so
/// it stays distinct from the standard FNV-1a offset basis.
const FOLD_START: u64 = 1_464_717_047_381_303_168;

/// Fold whole words into `hash`: xor, then the FNV prime multiply. A
/// word-wise step, not the byte-wise `Fnv64` fold, so the digests stay put.
fn fold_words(hash: u64, words: &[u64]) -> u64 {
    words
        .iter()
        .fold(hash, |acc, word| (acc ^ word).wrapping_mul(FNV64_PRIME))
}

/// Human players in the siege (matches the net crate's 64-client soak).
pub const PLAYERS: usize = 64;
/// AI soldiers in the siege (matches the net crate's 500-soldier legion).
pub const SOLDIERS: usize = 500;
/// Every replicated entity: fighters plus soldiers.
pub const ENTITIES: usize = PLAYERS + SOLDIERS;
/// Fixed tick rate in Hz (parity with the arena and net scale modules).
pub const TICK_HZ: u64 = 60;
/// Dedicated tick bar: p99 at or under 4 ms with interest filtering on.
pub const BUDGET_P99_MS: f64 = 4.0;
/// Canonical siege tape seed: single home is `engine-net::tick_budget::SIEGE_SEED`
/// (this crate reuses it, never mirrors it; `engine-proof::online_soak` does the same).
pub use engine_net::tick_budget::SIEGE_SEED;
/// Canonical siege tape length: 600 ticks (10 s at 60 Hz).
pub const SIEGE_TICKS: u32 = 600;
/// Longest tape one call steps (10 minutes); bounds gate time.
pub const MAX_TICKS: u32 = 36_000;
/// Near ring in millimetres: full rate inside (the net crate's siege tuning).
pub const NEAR_MM: i64 = 40_000;
/// Far ring in millimetres: LOD rate inside, combat-only beyond.
pub const FAR_MM: i64 = 120_000;

/// Per-system breakdown names, in tick order.
pub const SYSTEM_SIMULATE: &str = "simulate";
/// Per-system breakdown names, in tick order.
pub const SYSTEM_AI: &str = "ai";
/// Per-system breakdown names, in tick order.
pub const SYSTEM_INTEREST: &str = "interest";
/// Per-system breakdown names, in tick order.
pub const SYSTEM_ENCODE: &str = "encode";
/// Every measured system, in report order.
pub const SYSTEMS: &[&str] = &[SYSTEM_SIMULATE, SYSTEM_AI, SYSTEM_INTEREST, SYSTEM_ENCODE];

/// Siege budget failure: which input or budget refused, with both sides.
#[derive(Clone, Debug, PartialEq)]
pub enum ServerBudgetError {
    /// Tape length outside `1..=MAX_TICKS`.
    BadTicks {
        /// Ticks the caller offered.
        got: u32,
    },
    /// Tick p99 blew the 4 ms bar.
    OverBudget {
        /// Measured p99 in ms.
        p99_ms: f64,
        /// Budget in ms.
        budget_ms: f64,
    },
    /// Caller selection buffer shorter than the entity count.
    OutTooSmall {
        /// Slots that must fit.
        need: usize,
        /// Slots the buffer fits.
        have: usize,
    },
    /// No samples to summarize.
    EmptySamples {
        /// What needed samples.
        what: &'static str,
    },
    /// A timing is NaN, infinite or negative; wall clocks never produce one.
    NonFiniteSample {
        /// What carried the bad timing.
        what: &'static str,
    },
}

impl Display for ServerBudgetError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadTicks { got } => write!(f, "siege ticks {got} outside 1..={MAX_TICKS}"),
            Self::OverBudget { p99_ms, budget_ms } => write!(
                f,
                "server tick p99 budget blown: p99 {p99_ms:.3} ms over budget {budget_ms:.3} ms"
            ),
            Self::OutTooSmall { need, have } => {
                write!(f, "selection fits {have} ids but {need} must fit")
            }
            Self::EmptySamples { what } => write!(f, "{what} captured no samples"),
            Self::NonFiniteSample { what } => {
                write!(f, "{what} sample is not finite non-negative ms")
            }
        }
    }
}

impl std::error::Error for ServerBudgetError {}

/// One siege entity: integer lattice position plus combat word. Units
/// (index below `PLAYERS`) duel-step every tick; soldiers hold formation
/// and shift about once a second, so culled refreshes stay cheap.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct SiegeEntity {
    x_mm: i32,
    z_mm: i32,
    hp: u16,
}

impl SiegeEntity {
    const fn seed(index: usize, seed: u64) -> Self {
        let spread = (index as u64)
            .wrapping_mul(2_692_537)
            .wrapping_add(seed % 1_000_000);
        let extent: u64 = if index < PLAYERS {
            3_000_000
        } else {
            2_400_000
        };
        let half = (extent / 2) as i32;
        Self {
            x_mm: (spread % extent) as i32 - half,
            z_mm: ((spread / extent) % extent) as i32 - half,
            hp: 1_000,
        }
    }
}

/// Server-authoritative siege world: 64 fighters plus 500 AI soldiers.
/// All state is flat `Vec`s allocated once in `new`; a tick borrows them
/// mutably and allocates nothing (proven by the counting-allocator test).
pub struct SiegeWorld {
    entities: Vec<SiegeEntity>,
    /// Per-entity AI wish for this tick (packed dx/dz in [-100, 100]).
    wish_x: Vec<i16>,
    wish_z: Vec<i16>,
    /// Per-entity combat-dirty flag since the last snapshot.
    dirty: Vec<bool>,
    /// Last tick each (viewer, entity) pair was sent; `u32::MAX` is never.
    /// Indexed `viewer * ENTITIES + entity`; allocated once, updated in place.
    last_sent: Vec<u32>,
    seed: u64,
}

/// Bit-preserving `i32` millimetres to `u64` hash words.
/// Sign-extends exactly like the old `as u64`, so canonical hashes
/// never move; the named conversion keeps the cast gate quiet.
fn u64_from_mm(v: i32) -> u64 {
    u64::from_ne_bytes(i64::from(v).to_ne_bytes())
}

/// Saturating `i32` wish pull to `i16`. Pulls stay in [-100, 100] by
/// construction, so every production value converts identically; an
/// out-of-range input saturates instead of wrapping like `as i16`.
fn i16_from_pull(v: i32) -> i16 {
    i16::try_from(v).unwrap_or(if v < 0 { i16::MIN } else { i16::MAX })
}

/// Saturating `u64` lattice magnitude to `i32`. Magnitudes stay under
/// 3_000_000 by construction, so every production value converts
/// identically; an out-of-range input saturates instead of wrapping.
fn i32_from_magnitude(v: u64) -> i32 {
    i32::try_from(v).unwrap_or(i32::MAX)
}

impl SiegeWorld {
    /// Build the siege from its seed. Allocates once; ticks never allocate.
    #[must_use]
    pub fn new(seed: u64) -> Self {
        let mut entities = Vec::with_capacity(ENTITIES);
        for index in 0..ENTITIES {
            entities.push(SiegeEntity::seed(index, seed));
        }
        Self {
            entities,
            wish_x: vec![0; ENTITIES],
            wish_z: vec![0; ENTITIES],
            dirty: vec![false; ENTITIES],
            last_sent: vec![u32::MAX; PLAYERS * ENTITIES],
            seed,
        }
    }

    /// Entities in the siege (always 564).
    #[must_use]
    pub fn len(&self) -> usize {
        self.entities.len()
    }

    /// True when the world holds no entities (never: built with 564).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entities.is_empty()
    }

    /// AI system: one integer-utility wish per entity. Each soldier advances
    /// toward the arena centre pulled by a seeded weave (formation
    /// discipline without float or trig); each fighter weaves toward its
    /// mirrored rival, so wishes actually move bodies. Writes the caller-
    /// owned wish buffers only; allocates nothing.
    pub fn step_ai(&mut self, tick: u32) {
        for (index, entity) in self.entities.iter().enumerate() {
            let mix = u64::from(tick)
                .wrapping_mul(2_654_435_761)
                .wrapping_add((index as u64).wrapping_mul(97_531))
                .wrapping_add(self.seed);
            // Centre pull in centimetres keeps the legion inside the bowl;
            // the weave keeps fighters duelling instead of standing still.
            let pull_x = i16_from_pull(-(entity.x_mm / 1000).clamp(-100, 100));
            let pull_z = i16_from_pull(-(entity.z_mm / 1000).clamp(-100, 100));
            let weave = i16_from_pull(i32_from_magnitude((mix >> 7) % 61)) - 30;
            let (wx, wz) = if index < PLAYERS {
                // Duel shape: weave hard, pull gently.
                (
                    weave.saturating_add(pull_x / 4),
                    (30 - weave).saturating_add(pull_z / 4),
                )
            } else {
                // Soldier: hold formation, drift with the pull (legion shape).
                let shift = if u64::from(tick)
                    .wrapping_add(index as u64)
                    .is_multiple_of(90)
                {
                    i16_from_pull(i32_from_magnitude(mix % 41)) - 20
                } else {
                    0
                };
                (pull_x / 8 + shift, pull_z / 8 - shift)
            };
            self.wish_x[index] = wx.clamp(-100, 100);
            self.wish_z[index] = wz.clamp(-100, 100);
        }
    }

    /// Simulation system: apply wishes, lattice drift and scripted
    /// attrition. Integer wrapping ops only; marks combat-dirty entities.
    /// Reads the wish buffers the AI wrote; allocates nothing.
    pub fn step_simulate(&mut self, tick: u32) {
        for (index, entity) in self.entities.iter_mut().enumerate() {
            // Wish application: 10 mm per wish unit keeps speeds bounded.
            entity.x_mm = entity
                .x_mm
                .saturating_add(i32::from(self.wish_x[index]) * 10);
            entity.z_mm = entity
                .z_mm
                .saturating_add(i32::from(self.wish_z[index]) * 10);
            // Lattice drift: fighters duel-step every tick, soldiers hold.
            let mix = u64::from(tick)
                .wrapping_mul(2_654_435_761)
                .wrapping_add((index as u64).wrapping_mul(97_531))
                .wrapping_add(self.seed);
            if index < PLAYERS {
                let dx = i32_from_magnitude(mix % 601) - 300;
                let dz = i32_from_magnitude((mix >> 10) % 601) - 300;
                entity.x_mm = entity.x_mm.saturating_add(dx);
                entity.z_mm = entity.z_mm.saturating_add(dz);
            } else if u64::from(tick)
                .wrapping_add(index as u64)
                .is_multiple_of(90)
            {
                let dx = i32_from_magnitude(mix % 401) - 200;
                let dz = i32_from_magnitude((mix >> 9) % 401) - 200;
                entity.x_mm = entity.x_mm.saturating_add(dx);
                entity.z_mm = entity.z_mm.saturating_add(dz);
            }
            // Attrition: rare hp steps mark combat-dirty entities.
            if u64::from(tick)
                .wrapping_add(index as u64)
                .is_multiple_of(131)
            {
                let next = entity.hp.saturating_sub(5);
                if next != entity.hp {
                    entity.hp = next;
                    self.dirty[index] = true;
                }
            }
        }
    }

    /// Interest system: select one viewer's snapshot at 10 Hz over the
    /// siege tuning (40 m near at full rate, 120 m far at LOD rate,
    /// combat-dirty always sent, 500 ms starve guard). Writes ascending
    /// entity ids into `out`; returns `(sent, bytes)`. Integer distance
    /// only, stable id order, updates `last_sent` in place; allocates
    /// nothing. Non-snapshot ticks send nothing and return `(0, 0)`.
    pub fn select_for(
        &mut self,
        viewer: usize,
        tick: u32,
        out: &mut [u32],
    ) -> Result<(usize, usize), ServerBudgetError> {
        if out.len() < ENTITIES {
            return Err(ServerBudgetError::OutTooSmall {
                need: ENTITIES,
                have: out.len(),
            });
        }
        if !tick.is_multiple_of(SNAPSHOT_EVERY_TICKS) {
            return Ok((0, 0));
        }
        let viewer_pos = self.entities[viewer];
        let near2 = NEAR_MM * NEAR_MM;
        let far2 = FAR_MM * FAR_MM;
        // Far LOD: far-ring entities refresh every third snapshot (~3.3 Hz).
        let far_open = (tick / SNAPSHOT_EVERY_TICKS).is_multiple_of(3);
        let mut count = 0usize;
        let mut bytes = 21usize; // Snapshot header bytes (net scale parity).
        for (index, entity) in self.entities.iter().enumerate() {
            let dx = i64::from(entity.x_mm) - i64::from(viewer_pos.x_mm);
            let dz = i64::from(entity.z_mm) - i64::from(viewer_pos.z_mm);
            let dist2 = dx * dx + dz * dz;
            let slot = viewer * ENTITIES + index;
            let since = if self.last_sent[slot] == u32::MAX {
                u32::MAX
            } else {
                tick.saturating_sub(self.last_sent[slot])
            };
            let combat = self.dirty[index];
            // Combat-dirty entities are never culled: the no-desync rule.
            let send = combat
                || dist2 <= near2
                || (dist2 <= far2 && far_open)
                || since == u32::MAX
                || since >= STARVE_TICKS;
            if send {
                out[count] = index as u32;
                count += 1;
                // Wire cost by class: near 8, far 6, combat 10, starve 2.
                bytes += if combat {
                    10
                } else if dist2 <= near2 {
                    8
                } else if dist2 <= far2 {
                    6
                } else {
                    2
                };
                self.last_sent[slot] = tick;
            }
        }
        // Snapshot consumed the dirty flags for every viewer only after the
        // last viewer selects; the caller drives all 64 viewers per snapshot
        // through `snapshot_all`, which clears flags once per snapshot.
        Ok((count, bytes))
    }

    /// Run interest plus byte accounting for all 64 viewers on snapshot
    /// ticks; clears per-snapshot dirty flags once. Returns total snapshot
    /// bytes across viewers. `scratch` is the caller buffer (`ENTITIES`
    /// ids); allocates nothing.
    pub fn snapshot_all(
        &mut self,
        tick: u32,
        scratch: &mut [u32],
    ) -> Result<u64, ServerBudgetError> {
        if !tick.is_multiple_of(SNAPSHOT_EVERY_TICKS) {
            return Ok(0);
        }
        let mut total = 0u64;
        for viewer in 0..PLAYERS {
            let (_, bytes) = self.select_for(viewer, tick, scratch)?;
            total += bytes as u64;
        }
        for flag in self.dirty.iter_mut() {
            *flag = false;
        }
        Ok(total)
    }

    /// Encode system: fold the snapshot byte total plus the per-viewer
    /// selection counts into a checksum so the optimizer cannot elide the
    /// interest walk in release. Pure integer fold; allocates nothing.
    #[must_use]
    pub fn fold_snapshot(&self, bytes: u64, tick: u32) -> u64 {
        fold_words(FOLD_START, &[bytes, u64::from(tick), self.seed])
    }

    /// One full dedicated tick in server order: AI, simulate, then (on
    /// snapshot ticks) interest plus encode. Returns the snapshot bytes
    /// (0 off-cadence). Allocates nothing.
    pub fn step_tick(&mut self, tick: u32, scratch: &mut [u32]) -> Result<u64, ServerBudgetError> {
        self.step_ai(tick);
        self.step_simulate(tick);
        let bytes = self.snapshot_all(tick, scratch)?;
        black_box(self.fold_snapshot(bytes, tick));
        Ok(bytes)
    }

    /// Canonical hash over the simulation state after `tick`: FNV-1a over
    /// every entity's words plus the tick. Integer ops only.
    fn fold_state(&self, tick: u32, hash: u64) -> u64 {
        let hash = self.entities.iter().fold(hash, |acc, entity| {
            fold_words(
                acc,
                &[
                    u64_from_mm(entity.x_mm),
                    u64_from_mm(entity.z_mm),
                    u64::from(entity.hp),
                ],
            )
        });
        fold_words(hash, &[u64::from(tick)])
    }

    /// Worst ticks-since-sent over every (viewer, entity) pair at `tick`.
    /// Pairs never sent read as sent at tick 0, so a fresh world reports
    /// `tick`; a converged soak stays within `STARVE_TICKS`.
    fn starve_max(&self, tick: u32) -> u32 {
        let mut worst = 0u32;
        for slot in &self.last_sent {
            let since = if *slot == u32::MAX {
                tick
            } else {
                tick.saturating_sub(*slot)
            };
            if since > worst {
                worst = since;
            }
        }
        worst
    }
}

/// Deterministic digest of one recorded siege tape: seed, length, entity
/// count and the canonical simulation hash. Same seed plus same ticks give
/// the same hash on every run; wall-clock time never enters.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SiegeDigest {
    /// Tape seed.
    pub seed: u64,
    /// Ticks stepped.
    pub ticks: u32,
    /// Entities per tick (always 564).
    pub entities: usize,
    /// Canonical simulation hash.
    pub hash: u64,
    /// Snapshot bytes across all viewers (interest-filtered total).
    pub snapshot_bytes: u64,
    /// Worst ticks-since-sent over every pair (must stay within the bar).
    pub starve_max: u32,
}

/// Run the recorded siege tape: step `ticks` ticks from `seed` with the
/// full AI plus simulate plus interest path and hash the simulation state.
/// Same inputs give the same digest on every run (asserted x2 in tests).
pub fn record_tape(seed: u64, ticks: u32) -> Result<SiegeDigest, ServerBudgetError> {
    if ticks == 0 || ticks > MAX_TICKS {
        return Err(ServerBudgetError::BadTicks { got: ticks });
    }
    let mut world = SiegeWorld::new(seed);
    let mut scratch = vec![0u32; ENTITIES];
    let mut hash: u64 = FOLD_START ^ seed;
    let mut snapshot_bytes = 0u64;
    for tick in 0..ticks {
        let bytes = world.step_tick(tick, &mut scratch)?;
        snapshot_bytes += bytes;
        hash = world.fold_state(tick, hash);
    }
    Ok(SiegeDigest {
        seed,
        ticks,
        entities: ENTITIES,
        hash,
        snapshot_bytes,
        starve_max: world.starve_max(ticks - 1),
    })
}

/// Canonical tape: seed 7, 600 ticks (the posted budget report's tape).
pub fn canonical_digest() -> Result<SiegeDigest, ServerBudgetError> {
    record_tape(SIEGE_SEED, SIEGE_TICKS)
}

/// Quantiles over per-tick milliseconds, in report order.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TickQuantiles {
    /// Ticks sampled.
    pub ticks: u32,
    /// Median ms.
    pub p50_ms: f64,
    /// 95th percentile ms.
    pub p95_ms: f64,
    /// 99th percentile ms.
    pub p99_ms: f64,
}

impl TickQuantiles {
    /// Summarize per-tick ms: refuses empty input and non-finite samples,
    /// then sorts and reads nearest-rank quantiles (deterministic per input).
    /// Ranks delegate to [`crate::latency::nearest_rank_sorted`], the single
    /// float owner (ceil-ranked JS gate and integer fight-cost ranks differ
    /// deliberately, see there).
    pub fn of(what: &'static str, samples: &[f64]) -> Result<Self, ServerBudgetError> {
        if samples.is_empty() {
            return Err(ServerBudgetError::EmptySamples { what });
        }
        if samples.iter().any(|ms| !ms.is_finite() || *ms < 0.0) {
            return Err(ServerBudgetError::NonFiniteSample { what });
        }
        let mut sorted = samples.to_vec();
        sorted.sort_by(|a, b| a.partial_cmp(b).expect("samples are finite"));
        let at = |p: f64| crate::latency::nearest_rank_sorted(&sorted, p);
        Ok(Self {
            ticks: samples.len() as u32,
            p50_ms: at(50.0),
            p95_ms: at(95.0),
            p99_ms: at(99.0),
        })
    }
}

/// Per-system timing over the capture.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SystemStat {
    /// System name from `SYSTEMS`.
    pub system: &'static str,
    /// Quantiles over per-tick ms.
    pub quantiles: TickQuantiles,
}

/// Budget report for the recorded siege tape: the deterministic digest
/// plus wall-clock tick quantiles and the per-system breakdown. Numbers
/// are this machine's wall clock (evidence only, never hashed).
#[derive(Clone, Debug, PartialEq)]
pub struct ServerReport {
    /// Deterministic digest of the tape.
    pub digest: SiegeDigest,
    /// Quantiles over full-tick ms.
    pub tick: TickQuantiles,
    /// One row per system, in `SYSTEMS` order.
    pub systems: Vec<SystemStat>,
    /// True while tick p99 sits at or under the 4 ms bar.
    pub within_budget: bool,
}

impl ServerReport {
    /// Check tick p99 against the 4 ms bar. Fails loudly with both sides.
    pub fn check(&self) -> Result<(), ServerBudgetError> {
        check_p99(self.tick.p99_ms)
    }

    /// Human-readable lines for the posted budget report: tape identity,
    /// tick quantiles, one row per system and the budget verdict. Numbers
    /// vary per machine; names and units are stable.
    #[must_use]
    pub fn summary(&self) -> String {
        let mut out = format!(
            "siege_tick seed={} ticks={} entities={} hash={:016x} snap_bytes={} starve_max={}\n",
            self.digest.seed,
            self.digest.ticks,
            self.digest.entities,
            self.digest.hash,
            self.digest.snapshot_bytes,
            self.digest.starve_max,
        );
        out.push_str(&format!(
            "tick p50={:.3}ms p95={:.3}ms p99={:.3}ms budget_p99={:.1}ms {}\n",
            self.tick.p50_ms,
            self.tick.p95_ms,
            self.tick.p99_ms,
            BUDGET_P99_MS,
            if self.within_budget {
                "within"
            } else {
                "BLOWN"
            },
        ));
        for row in &self.systems {
            out.push_str(&format!(
                "system {} p50={:.3}ms p95={:.3}ms p99={:.3}ms\n",
                row.system, row.quantiles.p50_ms, row.quantiles.p95_ms, row.quantiles.p99_ms,
            ));
        }
        out
    }
}

/// Check one tick p99 against the 4 ms bar. Wall-clock gates compare here;
/// the default suite feeds synthetic numbers, the `--perf` proof feeds the
/// measured p99.
pub fn check_p99(p99_ms: f64) -> Result<(), ServerBudgetError> {
    if !p99_ms.is_finite() || p99_ms < 0.0 {
        return Err(ServerBudgetError::NonFiniteSample { what: "tick" });
    }
    if p99_ms > BUDGET_P99_MS {
        return Err(ServerBudgetError::OverBudget {
            p99_ms,
            budget_ms: BUDGET_P99_MS,
        });
    }
    Ok(())
}

/// Capture wall-clock timings for the recorded tape: one full-tick sample
/// plus one sample per system per tick, over `ticks` ticks from `seed`.
/// Simulation bytes are hashed and folded through `black_box` so the
/// optimizer cannot elide the work in release. Returns the budget report.
pub fn capture(seed: u64, ticks: u32) -> Result<ServerReport, ServerBudgetError> {
    if ticks == 0 || ticks > MAX_TICKS {
        return Err(ServerBudgetError::BadTicks { got: ticks });
    }
    let digest = record_tape(seed, ticks)?;
    let mut world = SiegeWorld::new(seed);
    let mut scratch = vec![0u32; ENTITIES];
    let capacity = ticks as usize;
    let mut tick_ms = Vec::with_capacity(capacity);
    let mut sim_ms = Vec::with_capacity(capacity);
    let mut ai_ms = Vec::with_capacity(capacity);
    let mut interest_ms = Vec::with_capacity(capacity);
    let mut encode_ms = Vec::with_capacity(capacity);
    for tick in 0..ticks {
        let full = Instant::now();
        let started = Instant::now();
        world.step_ai(tick);
        ai_ms.push(started.elapsed().as_secs_f64() * 1000.0);
        let started = Instant::now();
        world.step_simulate(tick);
        sim_ms.push(started.elapsed().as_secs_f64() * 1000.0);
        let started = Instant::now();
        let bytes = world.snapshot_all(tick, &mut scratch)?;
        interest_ms.push(started.elapsed().as_secs_f64() * 1000.0);
        let started = Instant::now();
        black_box(world.fold_snapshot(bytes, tick));
        encode_ms.push(started.elapsed().as_secs_f64() * 1000.0);
        tick_ms.push(full.elapsed().as_secs_f64() * 1000.0);
    }
    black_box(world.fold_state(ticks - 1, digest.hash));
    let tick = TickQuantiles::of("tick", &tick_ms)?;
    let systems = vec![
        SystemStat {
            system: SYSTEM_SIMULATE,
            quantiles: TickQuantiles::of(SYSTEM_SIMULATE, &sim_ms)?,
        },
        SystemStat {
            system: SYSTEM_AI,
            quantiles: TickQuantiles::of(SYSTEM_AI, &ai_ms)?,
        },
        SystemStat {
            system: SYSTEM_INTEREST,
            quantiles: TickQuantiles::of(SYSTEM_INTEREST, &interest_ms)?,
        },
        SystemStat {
            system: SYSTEM_ENCODE,
            quantiles: TickQuantiles::of(SYSTEM_ENCODE, &encode_ms)?,
        },
    ];
    let within_budget = tick.p99_ms <= BUDGET_P99_MS;
    Ok(ServerReport {
        digest,
        tick,
        systems,
        within_budget,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::alloc;

    #[test]
    fn u64_from_mm_sign_extends_negatives() {
        assert_eq!(u64_from_mm(-1), u64::MAX);
        assert_eq!(u64_from_mm(0), 0);
        assert_eq!(u64_from_mm(1_000_000), 1_000_000);
        assert_ne!(u64_from_mm(-1_000), u64_from_mm(1_000));
    }

    #[test]
    fn named_conversions_saturate_instead_of_wrapping() {
        // The old `as` casts wrapped out-of-range inputs (100_000i32 as i16
        // is 34_464, not 32_767); the named conversions saturate, so every
        // production value (always in range) converts identically.
        assert_eq!(i16_from_pull(100_000), i16::MAX);
        assert_eq!(i16_from_pull(-100_000), i16::MIN);
        assert_eq!(i16_from_pull(42), 42);
        assert_eq!(i16_from_pull(-100), -100);
        assert_eq!(i32_from_magnitude(u64::from(u32::MAX) + 1), i32::MAX);
        assert_eq!(i32_from_magnitude(601), 601);
        assert_eq!(i32_from_magnitude(0), 0);
    }

    #[test]
    fn tape_replays_identically_x2() {
        let first = record_tape(SIEGE_SEED, SIEGE_TICKS).expect("tape records");
        let second = record_tape(SIEGE_SEED, SIEGE_TICKS).expect("tape records");
        assert_eq!(first, second, "same tape must replay identically");
        assert_eq!((first.entities, first.ticks), (ENTITIES, SIEGE_TICKS));
        assert_ne!(first.hash, 0, "tape must actually simulate");
        assert!(first.snapshot_bytes > 0, "snapshots must carry bytes");
        assert_eq!(first, canonical_digest().expect("canonical tape"));
    }

    #[test]
    fn different_seeds_diverge() {
        let a = record_tape(7, SIEGE_TICKS).expect("tape");
        let b = record_tape(8, SIEGE_TICKS).expect("tape");
        assert_ne!(a.hash, b.hash, "different seeds must diverge");
    }

    #[test]
    fn tape_actually_moves_and_decides() {
        // A tape that never moves bodies or never writes wishes would still
        // hash; pin both paths so a gutted step fails this test.
        let mut world = SiegeWorld::new(SIEGE_SEED);
        let before: Vec<SiegeEntity> = world.entities.clone();
        let mut scratch = vec![0u32; ENTITIES];
        for tick in 0..120 {
            world.step_tick(tick, &mut scratch).expect("tick steps");
        }
        assert_ne!(world.entities, before, "bodies must move");
        assert!(
            world.wish_x.iter().any(|w| *w != 0) || world.wish_z.iter().any(|w| *w != 0),
            "AI must write nonzero wishes"
        );
        assert!(
            world.entities.iter().any(|e| e.hp < 1_000),
            "attrition must mark combat over 120 ticks"
        );
    }

    #[test]
    fn interest_never_starves_and_sends_combat() {
        let digest = record_tape(SIEGE_SEED, SIEGE_TICKS).expect("tape");
        assert!(
            digest.starve_max <= STARVE_TICKS,
            "starve_max {} over bar {STARVE_TICKS}",
            digest.starve_max,
        );
        // Combat-dirty entities are never culled: force one far dirty body
        // and prove the far viewer still sends it on a closed LOD tick.
        let mut world = SiegeWorld::new(11);
        for entity in world.entities.iter_mut() {
            entity.x_mm = 0;
            entity.z_mm = 0;
            entity.hp = 1_000;
        }
        // Viewer 0 at origin; entity 100 flung beyond the far ring and dirty.
        world.entities[100].x_mm = 1_000_000;
        world.entities[100].z_mm = 0;
        world.dirty[100] = true;
        // Tick 12 is a snapshot tick with the far LOD closed (12/6=2, 2%3!=0).
        let mut out = vec![0u32; ENTITIES];
        let (sent, bytes) = world.select_for(0, 12, &mut out).expect("select");
        assert!(
            out[..sent].contains(&100),
            "combat-dirty entity must send beyond the far ring"
        );
        assert!(bytes > 21, "selection must account bytes");
    }

    #[test]
    fn steady_tick_allocates_nothing() {
        let mut world = SiegeWorld::new(SIEGE_SEED);
        let mut scratch = vec![0u32; ENTITIES];
        for tick in 0..30 {
            world.step_tick(tick, &mut scratch).expect("warmup steps");
        }
        let ((), observed) = alloc::measure(|| {
            for tick in 30..90 {
                let bytes = world.step_tick(tick, &mut scratch).expect("tick steps");
                black_box(bytes);
            }
        });
        assert_eq!(observed.bytes, 0, "steady tick must not allocate bytes");
        assert_eq!(observed.count, 0, "steady tick must not allocate");
    }

    #[test]
    fn snapshot_cadence_matches_the_net_owner() {
        // Single-owner rule (SL17): the cadence lives in
        // `engine-net::interest` and this harness must snapshot on exactly
        // its multiples, so a reintroduced local cadence fails here.
        let mut world = SiegeWorld::new(SIEGE_SEED);
        let mut scratch = vec![0u32; ENTITIES];
        let mut snapped = Vec::new();
        for tick in 0..(SNAPSHOT_EVERY_TICKS * 3 + 1) {
            let bytes = world
                .snapshot_all(tick, &mut scratch)
                .expect("snapshot runs");
            if bytes > 0 {
                snapped.push(tick);
            }
        }
        assert_eq!(snapped.len(), 4, "10 Hz lane over {snapped:?}");
        for (index, tick) in snapped.iter().enumerate() {
            assert_eq!(
                *tick,
                index as u32 * SNAPSHOT_EVERY_TICKS,
                "lane drift at tick {tick}"
            );
        }
    }

    #[test]
    fn rejects_bad_shapes_loudly() {
        assert_eq!(
            record_tape(SIEGE_SEED, 0),
            Err(ServerBudgetError::BadTicks { got: 0 })
        );
        assert_eq!(
            record_tape(SIEGE_SEED, MAX_TICKS + 1),
            Err(ServerBudgetError::BadTicks { got: MAX_TICKS + 1 })
        );
        assert_eq!(
            capture(SIEGE_SEED, 0),
            Err(ServerBudgetError::BadTicks { got: 0 })
        );
        let mut world = SiegeWorld::new(SIEGE_SEED);
        let mut tiny = vec![0u32; 8];
        assert_eq!(
            world.select_for(0, 6, &mut tiny),
            Err(ServerBudgetError::OutTooSmall {
                need: ENTITIES,
                have: 8
            })
        );
        assert_eq!(
            TickQuantiles::of("tick", &[]),
            Err(ServerBudgetError::EmptySamples { what: "tick" })
        );
        assert!(TickQuantiles::of("tick", &[1.0, f64::NAN]).is_err());
        assert!(TickQuantiles::of("tick", &[1.0, -0.5]).is_err());
        assert!(check_p99(f64::NAN).is_err());
    }

    #[test]
    fn budget_gate_trips_with_both_sides() {
        check_p99(BUDGET_P99_MS).expect("exact budget passes");
        check_p99(0.0).expect("idle passes");
        let blown = check_p99(BUDGET_P99_MS + 0.5).expect_err("over budget must trip");
        assert_eq!(
            blown,
            ServerBudgetError::OverBudget {
                p99_ms: BUDGET_P99_MS + 0.5,
                budget_ms: BUDGET_P99_MS,
            }
        );
        let text = blown.to_string();
        assert!(text.contains("p99"), "{text}");
        assert!(text.contains('4'), "{text}");
    }

    #[test]
    fn error_strings_name_themselves() {
        let cases: &[ServerBudgetError] = &[
            ServerBudgetError::BadTicks { got: 0 },
            ServerBudgetError::OverBudget {
                p99_ms: 5.0,
                budget_ms: 4.0,
            },
            ServerBudgetError::OutTooSmall { need: 9, have: 3 },
            ServerBudgetError::EmptySamples { what: "tick" },
            ServerBudgetError::NonFiniteSample { what: "tick" },
        ];
        for error in cases {
            let text = error.to_string();
            assert!(!text.is_empty(), "{error:?} explains itself");
            let _: &dyn std::error::Error = error;
        }
        assert!(ServerBudgetError::BadTicks { got: 0 }
            .to_string()
            .contains("ticks"));
    }

    #[test]
    fn quantiles_rank_deterministically() {
        let q = TickQuantiles::of("tick", &[4.0, 1.0, 3.0, 2.0]).expect("ranks");
        assert_eq!(q.ticks, 4);
        assert!(q.p50_ms <= q.p95_ms && q.p95_ms <= q.p99_ms);
    }

    #[test]
    #[ignore = "timing: siege p99 proof, run with --release -- --ignored --test-threads=1"]
    fn siege_tick_p99_within_budget() {
        let report = capture(SIEGE_SEED, SIEGE_TICKS).expect("capture runs");
        println!("{}", report.summary());
        assert_eq!(report.systems.len(), SYSTEMS.len());
        assert_eq!(
            report
                .systems
                .iter()
                .map(|row| row.system)
                .collect::<Vec<_>>(),
            SYSTEMS,
            "breakdown names every system in order"
        );
        report.check().expect("siege tick p99 within 4 ms");
        assert!(
            report.digest.starve_max <= STARVE_TICKS,
            "no entity starved over 500 ms"
        );
    }
}
