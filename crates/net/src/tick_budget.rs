//! tick_budget: siege server tick budget.
//! Owner: net lane. Units: ticks at 60 Hz, ms wall-clock (presentation
//! only, never hashed), bytes. Determinism: AI wishes plus the harness
//! lattice are integer ops in stable index order, no float/trig/clock in
//! hashed state. Errors: [`TickBudgetError`], never panics. Budget: server
//! tick p99 at or under 4 ms at 64 players plus 500 AI with siege interest
//! filtering on. Consumer: the online soak reuses
//! [`record_tape`] plus [`BudgetReport::summary`]; the dedicated-server
//! operator reads the posted report. Crate edge: reuses the interest
//! [`InterestCfg`] plus [`ConnectionInterest`], never mirrors them, and the
//! harness lattice below is deliberately distinct from the siege `SiegeBot`
//! walk in `interest.rs` (one owner per bot math).

use super::interest::{
    ConnectionInterest, InterestCfg, InterestEntity, InterestError, SIEGE_BUDGET_BYTES,
    SNAPSHOT_EVERY_TICKS,
};
use super::scale::TICK_HZ;
use engine_core::digest::MerkleDigest;
use engine_core::hash::FNV64_OFFSET;
use std::fmt::{self, Display, Formatter};
use std::hint::black_box;
#[expect(
    clippy::disallowed_types,
    reason = "wall-clock perf sample, never hashed."
)]
use std::time::Instant;

/// Human players in the siege (equals the interest proof's connection count
/// by test).
pub const PLAYERS: usize = 64;
/// AI soldiers in the siege (equals the interest proof's legion count by
/// test).
pub const SOLDIERS: usize = 500;
/// Every authored entity: fighters plus soldiers.
pub const ENTITIES: usize = PLAYERS + SOLDIERS;
/// Slot count from which a hash mismatch is located by Merkle descent (RC9):
/// below it the full hash and a linear scan are cheaper; the siege is far above.
pub const LOCATE_MIN_SLOTS: usize = 256;
const _: () = assert!(ENTITIES >= LOCATE_MIN_SLOTS);
/// Canonical siege tape seed (matches the interest soak seed).
pub const SIEGE_SEED: u64 = 7;
/// Canonical short tape in ticks: 10 s at 60 Hz (the posted report's tape).
pub const SIEGE_TICKS: u32 = 600;
/// Acceptance tape in ticks: 10 minutes at 60 Hz (the p99 tape).
pub const SIEGE_TICKS_10MIN: u32 = 36_000;
/// Longest tape one call steps (10 minutes); bounds gate time.
pub const MAX_TICKS: u32 = 36_000;
/// Server tick bar: p99 at or under 4 ms with interest filtering on.
pub const BUDGET_P99_MS: f64 = 4.0;

/// Per-system breakdown name: AI wish authoring.
pub const SYSTEM_AI: &str = "ai";
/// Per-system breakdown name: wish application plus harness drift.
pub const SYSTEM_SIMULATE: &str = "simulate";
/// Per-system breakdown name: interest selects over all viewers.
pub const SYSTEM_INTEREST: &str = "interest";
/// Per-system breakdown name: snapshot checksum fold.
pub const SYSTEM_ENCODE: &str = "encode";
/// Every measured system, in tick order.
pub const SYSTEMS: &[&str] = &[SYSTEM_AI, SYSTEM_SIMULATE, SYSTEM_INTEREST, SYSTEM_ENCODE];

/// Siege budget failure: which input or budget refused, with both sides.
#[derive(Clone, Debug, PartialEq)]
pub enum TickBudgetError {
    /// Tape length outside `1..=MAX_TICKS`.
    BadTicks {
        /// Ticks the caller offered.
        got: u32,
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
    /// Tick p99 blew the 4 ms bar.
    OverBudget {
        /// Measured p99 in ms.
        p99_ms: f64,
        /// Budget in ms.
        budget_ms: f64,
    },
    /// Interest selection refused (budget, order or config); owned by the
    /// interest module.
    Interest(InterestError),
}

impl Display for TickBudgetError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadTicks { got } => write!(f, "siege ticks {got} outside 1..={MAX_TICKS}"),
            Self::OutTooSmall { need, have } => {
                write!(f, "selection fits {have} ids but {need} must fit")
            }
            Self::EmptySamples { what } => write!(f, "{what} captured no samples"),
            Self::NonFiniteSample { what } => {
                write!(f, "{what} sample is not finite non-negative ms")
            }
            Self::OverBudget { p99_ms, budget_ms } => write!(
                f,
                "server tick p99 budget blown: p99 {p99_ms:.3} ms over budget {budget_ms:.3} ms"
            ),
            Self::Interest(err) => write!(f, "siege interest refused ({err})"),
        }
    }
}

impl std::error::Error for TickBudgetError {}

impl From<InterestError> for TickBudgetError {
    fn from(err: InterestError) -> Self {
        Self::Interest(err)
    }
}

/// One AI wish: packed movement intent plus scripted combat bits for one
/// entity on one tick. A pure function of `(seed, tick, index)`, so the
/// same tape authors byte-identical inputs on every run.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct AiInput {
    /// Forward wish in `[-100, 100]`.
    pub wish_x: i16,
    /// Strafe wish in `[-100, 100]`.
    pub wish_z: i16,
    /// Scripted combat bits (intent only, hashed through the AI fold).
    pub buttons: u8,
}

/// Author one AI wish as a pure function of `(seed, tick, index)`.
/// Integer ops only; fighters weave every tick while soldiers hold
/// formation between shifts, so culled refreshes stay cheap under the
/// interest filter. Constants are harness-local (Knuth-family avalanche),
/// deliberately distinct from the interest `SiegeBot` walk (one owner per
/// math).
#[must_use]
pub fn author_ai(seed: u64, tick: u32, index: usize) -> AiInput {
    if index >= PLAYERS
        && !u64::from(tick)
            .wrapping_add(index as u64)
            .is_multiple_of(90)
    {
        return AiInput {
            wish_x: 0,
            wish_z: 0,
            buttons: 0,
        };
    }
    let mut mix = seed
        .wrapping_add(u64::from(tick).wrapping_mul(0x85EB_CA6B))
        .wrapping_add((index as u64).wrapping_mul(0xC2B2_AE35));
    mix ^= mix >> 29;
    mix = mix.wrapping_mul(0x1656_67B1_9E37_79F9);
    mix ^= mix >> 32;
    AiInput {
        wish_x: ((mix >> 7) % 201) as i16 - 100,
        wish_z: ((mix >> 23) % 201) as i16 - 100,
        buttons: (mix & 0x7) as u8,
    }
}

/// Byte form of one AI wish: the byte-identical unit the x2 test compares.
#[must_use]
pub fn author_ai_bytes(seed: u64, tick: u32, index: usize) -> [u8; 4] {
    let wish = author_ai(seed, tick, index);
    [
        (wish.wish_x as i8).cast_unsigned(),
        (wish.wish_z as i8).cast_unsigned(),
        wish.buttons,
        (index & 0xFF) as u8,
    ]
}

/// Harness seed position: fighters hold a duel ring, soldiers a march grid.
/// Harness-local strides, deliberately distinct from the interest bot spread.
const fn seed_pos(index: usize, seed: u64) -> (i32, i32) {
    let lane = (seed ^ 0x9E37_79B9_7F4A_7C15).wrapping_add(index as u64 * 0x9E37_79B1);
    let extent: i64 = if index < PLAYERS {
        1_600_000
    } else {
        2_000_000
    };
    let half = extent / 2;
    let x = (lane % extent as u64) as i64 - half;
    let z = ((lane >> 21) % extent as u64) as i64 - half;
    (x as i32, z as i32)
}

/// A component of one siege entity, in leaf order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SiegeComponent {
    /// East-west position in millimetres.
    X,
    /// North-south position in millimetres.
    Z,
    /// Combat hit points.
    Hp,
}

/// Where two siege worlds first differ: the lowest entity and its component.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DesyncSite {
    /// Entity index (fighters first, then soldiers).
    pub entity: usize,
    /// Component that differs on that entity.
    pub component: SiegeComponent,
}

/// Server-authoritative siege world: 64 fighters plus 500 AI soldiers with
/// one interest connection per viewer. All state is flat `Vec`s allocated once
/// in [`SiegeTickWorld::new`]; a tick borrows them mutably and allocates
/// nothing (proven by the capacity-stability test).
pub struct SiegeTickWorld {
    x_mm: Vec<i32>,
    z_mm: Vec<i32>,
    hp: Vec<u16>,
    wish_x: Vec<i16>,
    wish_z: Vec<i16>,
    conns: Vec<ConnectionInterest>,
    entities: Vec<InterestEntity>,
    out: Vec<u32>,
    seed: u64,
}

impl SiegeTickWorld {
    /// Build the siege from its seed. Allocates once; ticks never allocate.
    /// Registers every viewer on the tick-zero baseline, like the interest soak.
    pub fn new(seed: u64) -> Result<Self, TickBudgetError> {
        let mut x_mm = Vec::with_capacity(ENTITIES);
        let mut z_mm = Vec::with_capacity(ENTITIES);
        for index in 0..ENTITIES {
            let (x, z) = seed_pos(index, seed);
            x_mm.push(x);
            z_mm.push(z);
        }
        let mut entities = Vec::with_capacity(ENTITIES);
        for index in 0..ENTITIES {
            entities.push(InterestEntity {
                id: index as u32,
                x_mm: x_mm[index],
                z_mm: z_mm[index],
                combat: 1_000,
            });
        }
        let mut conns = Vec::with_capacity(PLAYERS);
        for viewer in 0..PLAYERS {
            let mut conn = ConnectionInterest::for_entities(viewer as u32, ENTITIES)?;
            conn.register_baseline(&entities)?;
            conns.push(conn);
        }
        Ok(Self {
            x_mm,
            z_mm,
            hp: vec![1_000; ENTITIES],
            wish_x: vec![0; ENTITIES],
            wish_z: vec![0; ENTITIES],
            conns,
            entities,
            out: vec![0; ENTITIES],
            seed,
        })
    }

    /// Entities in the siege (always 564).
    #[must_use]
    pub fn len(&self) -> usize {
        self.x_mm.len()
    }

    /// True when the world holds no entities (never: built with 564).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.x_mm.is_empty()
    }

    /// Buffer capacity tuple for the no-growth test below.
    #[must_use]
    pub fn capacities(&self) -> (usize, usize, usize, usize) {
        (
            self.x_mm.capacity(),
            self.wish_x.capacity(),
            self.entities.capacity(),
            self.out.capacity(),
        )
    }

    /// AI system: one authored wish per entity. Writes the caller-owned
    /// wish buffers only; allocates nothing.
    pub fn step_ai(&mut self, tick: u32) {
        for index in 0..ENTITIES {
            let wish = author_ai(self.seed, tick, index);
            self.wish_x[index] = wish.wish_x;
            self.wish_z[index] = wish.wish_z;
        }
    }

    /// Simulation system: apply wishes, harness drift and scripted
    /// attrition. Integer saturating ops only; hp steps flow to the interest
    /// baseline as the combat word, so combat-dirty needs no side channel.
    /// Reads the wish buffers the AI wrote; allocates nothing.
    pub fn step_simulate(&mut self, tick: u32) {
        for index in 0..ENTITIES {
            self.x_mm[index] = self.x_mm[index].saturating_add(i32::from(self.wish_x[index]) * 10);
            self.z_mm[index] = self.z_mm[index].saturating_add(i32::from(self.wish_z[index]) * 10);
            // Harness drift: fighters duel-step every tick, soldiers shift
            // with their AI cadence. Knuth-family mix, harness-local.
            let mix = u64::from(tick)
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add((index as u64).wrapping_mul(0x9E37_79B1))
                .wrapping_add(self.seed >> 1);
            if index < PLAYERS {
                let dx = (mix % 541) as i32 - 270;
                let dz = ((mix >> 11) % 541) as i32 - 270;
                self.x_mm[index] = self.x_mm[index].saturating_add(dx);
                self.z_mm[index] = self.z_mm[index].saturating_add(dz);
            } else if u64::from(tick)
                .wrapping_add(index as u64)
                .is_multiple_of(90)
            {
                let dx = (mix % 361) as i32 - 180;
                let dz = ((mix >> 10) % 361) as i32 - 180;
                self.x_mm[index] = self.x_mm[index].saturating_add(dx);
                self.z_mm[index] = self.z_mm[index].saturating_add(dz);
            }
            // Attrition: rare hp steps (cadence and damage harness-local:
            // 139 ticks, 3 hp) flow to the interest baseline as the combat word.
            if u64::from(tick)
                .wrapping_add(index as u64)
                .is_multiple_of(139)
            {
                self.hp[index] = self.hp[index].saturating_sub(3);
            }
        }
    }

    /// Interest system: run the siege tuning at the 10 Hz snapshot
    /// cadence, staggered across ticks: tick `t` selects the viewers with
    /// `viewer % 6 == t % 6`, so every viewer still replicates every sixth
    /// tick (10 Hz) but no tick pays all 64 selects at once. Returns total
    /// snapshot bytes across the viewers selected this tick (`0` would mean
    /// a tick selected nobody, which never happens with 64 viewers over a
    /// cadence of 6). `yaw 0` matches the interest soak's viewer convention.
    /// Allocates nothing.
    pub fn snapshot_all(&mut self, tick: u32) -> Result<u64, TickBudgetError> {
        for index in 0..ENTITIES {
            self.entities[index] = InterestEntity {
                id: index as u32,
                x_mm: self.x_mm[index],
                z_mm: self.z_mm[index],
                combat: u32::from(self.hp[index]),
            };
        }
        let cfg = InterestCfg::default_siege();
        let mut total = 0u64;
        let lane = tick % SNAPSHOT_EVERY_TICKS;
        let (conns, entities, out) = (&mut self.conns, &self.entities, &mut self.out);
        if out.len() < ENTITIES {
            return Err(TickBudgetError::OutTooSmall {
                need: ENTITIES,
                have: out.len(),
            });
        }
        for viewer in 0..PLAYERS {
            if (viewer as u32) % SNAPSHOT_EVERY_TICKS != lane {
                continue;
            }
            let conn = &mut conns[viewer];
            let report = conn.select(
                &cfg,
                entities,
                viewer as u32,
                entities[viewer].x_mm,
                entities[viewer].z_mm,
                0,
                tick,
                SIEGE_BUDGET_BYTES,
                out,
            )?;
            total += report.bytes as u64;
        }
        Ok(total)
    }

    /// Encode system: fold the snapshot byte total into a checksum so the
    /// optimizer cannot elide the interest walk in release. Integer fold.
    #[must_use]
    pub fn fold_snapshot(&self, bytes: u64, tick: u32) -> u64 {
        let mut hash: u64 = 1_464_717_047_381_303_168;
        hash ^= bytes;
        hash = hash.wrapping_mul(1_099_511_628_211);
        hash ^= u64::from(tick);
        hash = hash.wrapping_mul(1_099_511_628_211);
        hash ^= self.seed;
        hash.wrapping_mul(1_099_511_628_211)
    }

    /// One full dedicated tick in server order: AI, simulate, then the due
    /// interest lane plus encode. Returns the snapshot bytes selected this
    /// tick. Allocates nothing.
    pub fn step_tick(&mut self, tick: u32) -> Result<u64, TickBudgetError> {
        self.step_ai(tick);
        self.step_simulate(tick);
        let bytes = self.snapshot_all(tick)?;
        black_box(self.fold_snapshot(bytes, tick));
        Ok(bytes)
    }

    /// Full state hash after `tick`: the cheap check two peers compare first.
    #[must_use]
    pub fn state_hash(&self, tick: u32) -> u64 {
        self.fold_state(tick, FNV64_OFFSET)
    }

    /// First entity and component where `peer` differs from this world, found
    /// by Merkle descent over one leaf per component (RC9), `None` when equal.
    /// Builds both trees, O(entities): call it only after [`Self::state_hash`]
    /// disagreed, never per tick. Diagnostics only, nothing here feeds a hash.
    #[must_use]
    pub fn first_divergence(&self, peer: &Self) -> Option<DesyncSite> {
        let leaf = self.digest().first_divergence(&peer.digest())?;
        let component = match leaf % 3 {
            0 => SiegeComponent::X,
            1 => SiegeComponent::Z,
            _ => SiegeComponent::Hp,
        };
        Some(DesyncSite {
            entity: leaf / 3,
            component,
        })
    }

    /// Merkle tree with leaves `3 * entity + {x, z, hp}`.
    fn digest(&self) -> MerkleDigest {
        let mut leaves = Vec::with_capacity(ENTITIES * 3);
        for index in 0..ENTITIES {
            leaves.push(self.x_mm[index] as u64);
            leaves.push(self.z_mm[index] as u64);
            leaves.push(u64::from(self.hp[index]));
        }
        MerkleDigest::from_leaves(&leaves)
    }

    /// Test seam: move one entity one millimetre, as a desynced peer would.
    #[cfg(test)]
    pub(crate) fn nudge_x(&mut self, entity: usize) {
        self.x_mm[entity] += 1;
    }

    /// Canonical hash over the simulation state after `tick`: FNV-1a over
    /// every entity's words plus the tick. Integer ops only.
    fn fold_state(&self, tick: u32, mut hash: u64) -> u64 {
        for index in 0..ENTITIES {
            hash ^= self.x_mm[index] as u64;
            hash = hash.wrapping_mul(1_099_511_628_211);
            hash ^= self.z_mm[index] as u64;
            hash = hash.wrapping_mul(1_099_511_628_211);
            hash ^= u64::from(self.hp[index]);
            hash = hash.wrapping_mul(1_099_511_628_211);
        }
        hash ^= u64::from(tick);
        hash.wrapping_mul(1_099_511_628_211)
    }

    /// Worst ticks-since-sent over every viewer (the starve ledger, owned
    /// by the interest connections).
    fn starve_max(&self) -> u32 {
        let mut worst = 0u32;
        for conn in &self.conns {
            if conn.starve_max() > worst {
                worst = conn.starve_max();
            }
        }
        worst
    }
}

/// Deterministic digest of one recorded siege tape: seed, length, entity
/// count, the canonical simulation hash and the AI-input hash. Same seed
/// plus same ticks give the same digest on every run; wall-clock time
/// never enters.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TickDigest {
    /// Tape seed.
    pub seed: u64,
    /// Ticks stepped.
    pub ticks: u32,
    /// Entities per tick (always 564).
    pub entities: usize,
    /// Canonical simulation hash.
    pub hash: u64,
    /// Canonical AI-input hash (byte-identical x2 proves the AI claim).
    pub ai_hash: u64,
    /// Snapshot bytes across all viewers (interest-filtered total).
    pub snapshot_bytes: u64,
    /// Worst ticks-since-sent over every viewer (must stay within the bar).
    pub starve_max: u32,
}

/// Fold one AI byte word (integer ops only, stable across runs).
const fn fold_ai_byte(hash: u64, word: u64) -> u64 {
    (hash ^ word).wrapping_mul(1_099_511_628_211)
}

/// Run the recorded siege tape: step `ticks` ticks from `seed` with the
/// full AI plus simulate plus siege-interest path and hash the simulation
/// state and the AI bytes. Same inputs give the same digest on every run.
pub fn record_tape(seed: u64, ticks: u32) -> Result<TickDigest, TickBudgetError> {
    record_tape_world(seed, ticks).map(|(digest, _)| digest)
}

/// [`record_tape`] that also hands back the world at the tape's last tick, so
/// a peer's run can be compared against it.
pub fn record_tape_world(
    seed: u64,
    ticks: u32,
) -> Result<(TickDigest, SiegeTickWorld), TickBudgetError> {
    if ticks == 0 || ticks > MAX_TICKS {
        return Err(TickBudgetError::BadTicks { got: ticks });
    }
    let mut world = SiegeTickWorld::new(seed)?;
    let mut hash: u64 = 1_464_717_047_381_303_168 ^ seed;
    let mut ai_hash: u64 = FNV64_OFFSET ^ seed;
    let mut snapshot_bytes = 0u64;
    for tick in 0..ticks {
        let bytes = world.step_tick(tick)?;
        snapshot_bytes += bytes;
        hash = world.fold_state(tick, hash);
        for index in 0..ENTITIES {
            let word = author_ai_bytes(seed, tick, index);
            let packed = u64::from(u32::from_le_bytes(word));
            ai_hash = fold_ai_byte(ai_hash, packed);
        }
    }
    let digest = TickDigest {
        seed,
        ticks,
        entities: ENTITIES,
        hash,
        ai_hash,
        snapshot_bytes,
        starve_max: world.starve_max(),
    };
    Ok((digest, world))
}

/// Canonical tape: seed 7, 600 ticks (the posted budget report's tape).
pub fn canonical_digest() -> Result<TickDigest, TickBudgetError> {
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
    pub fn of(what: &'static str, samples: &[f64]) -> Result<Self, TickBudgetError> {
        if samples.is_empty() {
            return Err(TickBudgetError::EmptySamples { what });
        }
        if samples.iter().any(|ms| !ms.is_finite() || *ms < 0.0) {
            return Err(TickBudgetError::NonFiniteSample { what });
        }
        let mut sorted = samples.to_vec();
        sorted.sort_by(|a, b| a.partial_cmp(b).expect("samples are finite"));
        let at = |p: f64| {
            let end = sorted.len() - 1;
            let index = ((p / 100.0) * end as f64).round() as usize;
            sorted[index.min(end)]
        };
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
    /// System name from [`SYSTEMS`].
    pub system: &'static str,
    /// Quantiles over per-tick ms.
    pub quantiles: TickQuantiles,
}

/// Budget report for the recorded siege tape: the deterministic digest
/// plus wall-clock tick quantiles and the per-system breakdown. Numbers
/// are this machine's wall clock (evidence only, never hashed).
#[derive(Clone, Debug, PartialEq)]
pub struct BudgetReport {
    /// Deterministic digest of the tape.
    pub digest: TickDigest,
    /// Quantiles over full-tick ms.
    pub tick: TickQuantiles,
    /// One row per system, in [`SYSTEMS`] order.
    pub systems: Vec<SystemStat>,
    /// True while tick p99 sits at or under the 4 ms bar.
    pub within_budget: bool,
}

impl BudgetReport {
    /// Check tick p99 against the 4 ms bar. Fails loudly with both sides.
    pub fn check(&self) -> Result<(), TickBudgetError> {
        check_p99(self.tick.p99_ms)
    }

    /// Human-readable lines for the posted budget report: tape identity,
    /// tick quantiles, one row per system, the siege tuning and the budget
    /// verdict. Numbers vary per machine; names and units are stable.
    #[must_use]
    pub fn summary(&self) -> String {
        let mut out = format!(
            "siege_tick seed={} ticks={} entities={} hash={:016x} ai={:016x} snap_bytes={} starve_max={}\n",
            self.digest.seed,
            self.digest.ticks,
            self.digest.entities,
            self.digest.hash,
            self.digest.ai_hash,
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
        let cfg = InterestCfg::default_siege();
        out.push_str(&format!(
            "interest near={}m@{}Hz far={}m@{}Hz cap={}B/s snap_every={} stagger=1/{} budget={}B tick_hz={}\n",
            cfg.near_m,
            cfg.near_hz,
            cfg.far_m,
            cfg.far_hz,
            cfg.cap_bps,
            SNAPSHOT_EVERY_TICKS,
            SNAPSHOT_EVERY_TICKS,
            SIEGE_BUDGET_BYTES,
            TICK_HZ,
        ));
        out
    }
}

/// Check one tick p99 against the 4 ms bar. Wall-clock gates compare here;
/// the default suite feeds synthetic numbers, the `--perf` proof feeds the
/// measured p99.
pub fn check_p99(p99_ms: f64) -> Result<(), TickBudgetError> {
    if !p99_ms.is_finite() || p99_ms < 0.0 {
        return Err(TickBudgetError::NonFiniteSample { what: "tick" });
    }
    if p99_ms > BUDGET_P99_MS {
        return Err(TickBudgetError::OverBudget {
            p99_ms,
            budget_ms: BUDGET_P99_MS,
        });
    }
    Ok(())
}

/// Capture wall-clock timings for the recorded tape in a single pass: one
/// full-tick sample plus one sample per system per tick, over `ticks`
/// ticks from `seed`, while folding the same deterministic digest
/// [`record_tape`] returns (asserted equal in tests). Simulation bytes are
/// folded through `black_box` so the optimizer cannot elide the work in
/// release. Returns the budget report.
#[expect(
    clippy::disallowed_types,
    reason = "wall-clock perf sample, never hashed."
)]
#[expect(
    clippy::disallowed_methods,
    reason = "wall-clock perf sample, never hashed."
)]
pub fn capture(seed: u64, ticks: u32) -> Result<BudgetReport, TickBudgetError> {
    if ticks == 0 || ticks > MAX_TICKS {
        return Err(TickBudgetError::BadTicks { got: ticks });
    }
    let mut world = SiegeTickWorld::new(seed)?;
    let mut tick_ms = Vec::with_capacity(ticks as usize);
    let mut ai_ms = Vec::with_capacity(ticks as usize);
    let mut sim_ms = Vec::with_capacity(ticks as usize);
    let mut interest_ms = Vec::with_capacity(ticks as usize);
    let mut encode_ms = Vec::with_capacity(ticks as usize);
    let mut hash: u64 = 1_464_717_047_381_303_168 ^ seed;
    let mut ai_hash: u64 = FNV64_OFFSET ^ seed;
    let mut snapshot_bytes = 0u64;
    for tick in 0..ticks {
        let full = Instant::now(); // det-ok: wall-clock perf sample, never hashed.
        let started = Instant::now(); // det-ok: wall-clock perf sample, never hashed.
        world.step_ai(tick);
        ai_ms.push(started.elapsed().as_secs_f64() * 1000.0);
        let started = Instant::now(); // det-ok: wall-clock perf sample, never hashed.
        world.step_simulate(tick);
        sim_ms.push(started.elapsed().as_secs_f64() * 1000.0);
        let started = Instant::now(); // det-ok: wall-clock perf sample, never hashed.
        let bytes = world.snapshot_all(tick)?;
        interest_ms.push(started.elapsed().as_secs_f64() * 1000.0);
        let started = Instant::now(); // det-ok: wall-clock perf sample, never hashed.
        black_box(world.fold_snapshot(bytes, tick));
        encode_ms.push(started.elapsed().as_secs_f64() * 1000.0);
        tick_ms.push(full.elapsed().as_secs_f64() * 1000.0);
        snapshot_bytes += bytes;
        hash = world.fold_state(tick, hash);
        for index in 0..ENTITIES {
            let word = author_ai_bytes(seed, tick, index);
            let packed = u64::from(u32::from_le_bytes(word));
            ai_hash = fold_ai_byte(ai_hash, packed);
        }
    }
    black_box(hash);
    let digest = TickDigest {
        seed,
        ticks,
        entities: ENTITIES,
        hash,
        ai_hash,
        snapshot_bytes,
        starve_max: world.starve_max(),
    };
    let tick = TickQuantiles::of("tick", &tick_ms)?;
    let systems = vec![
        SystemStat {
            system: SYSTEM_AI,
            quantiles: TickQuantiles::of(SYSTEM_AI, &ai_ms)?,
        },
        SystemStat {
            system: SYSTEM_SIMULATE,
            quantiles: TickQuantiles::of(SYSTEM_SIMULATE, &sim_ms)?,
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
    Ok(BudgetReport {
        digest,
        tick,
        systems,
        within_budget,
    })
}

#[cfg(test)]
mod tests {
    use super::super::interest::{
        MAX_CONNECTIONS, MAX_INTEREST_ENTITIES, ON9_CAP_BPS, STARVE_TICKS,
    };
    use super::*;

    #[test]
    fn first_divergence_names_the_lowest_entity_and_its_component() {
        let a = SiegeTickWorld::new(3).expect("world builds");
        let mut b = SiegeTickWorld::new(3).expect("world builds");
        assert_eq!(a.first_divergence(&b), None);
        assert_eq!(a.state_hash(0), b.state_hash(0));
        b.hp[ENTITIES - 1] -= 1;
        b.z_mm[41] += 7;
        b.x_mm[ENTITIES - 2] -= 1;
        assert_ne!(a.state_hash(0), b.state_hash(0));
        let at = |entity, component| Some(DesyncSite { entity, component });
        assert_eq!(a.first_divergence(&b), at(41, SiegeComponent::Z));
        assert_eq!(b.first_divergence(&a), at(41, SiegeComponent::Z));
        b.z_mm[41] -= 7;
        assert_eq!(a.first_divergence(&b), at(ENTITIES - 2, SiegeComponent::X));
        b.x_mm[ENTITIES - 2] += 1;
        assert_eq!(a.first_divergence(&b), at(ENTITIES - 1, SiegeComponent::Hp));
    }

    #[test]
    fn on9_contract_pins_without_mirroring() {
        // The siege tape reuses the interest tuning and shape; any drift in the
        // shared contract fails here, never silently.
        let cfg = InterestCfg::default_siege();
        cfg.validate().expect("ON9 tuning is valid");
        assert_eq!(
            (cfg.near_m, cfg.far_m, cfg.near_hz, cfg.far_hz),
            (32, 100, 10, 2)
        );
        assert_eq!(cfg.cap_bps, ON9_CAP_BPS);
        assert_eq!(cfg.every_near(), 6);
        assert_eq!(SNAPSHOT_EVERY_TICKS, 6);
        assert_eq!(SIEGE_BUDGET_BYTES, 1_200);
        assert_eq!(PLAYERS, MAX_CONNECTIONS);
        assert_eq!(ENTITIES, PLAYERS + SOLDIERS);
        // Anchored on the live world below (not a bare constant assert):
        // the tape shape must fit the interest selection bound.
        let world = SiegeTickWorld::new(SIEGE_SEED).expect("world builds");
        assert_eq!(world.len(), 564);
        assert!(world.len() <= MAX_INTEREST_ENTITIES);
        assert_eq!(SIEGE_TICKS_10MIN, 10 * 60 * TICK_HZ as u32);
        assert_eq!(MAX_TICKS, SIEGE_TICKS_10MIN);
    }

    #[test]
    fn ai_inputs_byte_identical_x2() {
        // Every AI byte over 120 ticks must reproduce exactly; a second
        // seed must diverge so the equality below proves something.
        let (mut first, mut second) = (Vec::new(), Vec::new());
        for tick in 0..120u32 {
            for index in 0..ENTITIES {
                first.extend_from_slice(&author_ai_bytes(SIEGE_SEED, tick, index));
                second.extend_from_slice(&author_ai_bytes(SIEGE_SEED, tick, index));
            }
        }
        assert_eq!(first, second);
        assert!(!first.is_empty());
        assert!(
            first.iter().any(|b| *b != 0),
            "AI must author nonzero bytes"
        );
        let mut other = Vec::new();
        for tick in 0..120u32 {
            for index in 0..ENTITIES {
                other.extend_from_slice(&author_ai_bytes(SIEGE_SEED + 1, tick, index));
            }
        }
        assert_ne!(first, other, "different seeds must diverge");
        // Soldiers hold formation between shifts: most soldier wishes rest.
        let resting = (PLAYERS..ENTITIES)
            .filter(|index| author_ai(SIEGE_SEED, 7, *index).wish_x == 0)
            .count();
        assert!(resting > 0, "soldiers must rest between shifts");
    }

    #[test]
    fn tape_replays_identically_x2() {
        let first = record_tape(SIEGE_SEED, 240).expect("tape records");
        let second = record_tape(SIEGE_SEED, 240).expect("tape records");
        assert_eq!(first, second, "same tape must replay identically");
        assert_eq!((first.entities, first.ticks), (ENTITIES, 240));
        assert_ne!(first.hash, 0, "tape must actually simulate");
        assert_ne!(first.ai_hash, 0, "tape must actually author AI");
        assert!(first.snapshot_bytes > 0, "snapshots must carry bytes");
        assert!(first.starve_max <= STARVE_TICKS);
    }

    #[test]
    fn canonical_short_tape_matches_its_digest() {
        let tape = record_tape(SIEGE_SEED, SIEGE_TICKS).expect("canonical tape");
        assert_eq!(tape, canonical_digest().expect("canonical digest"));
        assert!(tape.starve_max <= STARVE_TICKS);
    }

    #[test]
    fn different_seeds_diverge() {
        let a = record_tape(7, 240).expect("tape");
        let b = record_tape(8, 240).expect("tape");
        assert_ne!(a.hash, b.hash, "different seeds must diverge");
        assert_ne!(a.ai_hash, b.ai_hash, "AI streams must diverge");
    }

    #[test]
    fn tape_actually_moves_and_decides() {
        // A tape that never moves bodies or never writes wishes would still
        // hash; pin both paths so a gutted step fails this test.
        let mut world = SiegeTickWorld::new(SIEGE_SEED).expect("world builds");
        let before_x = world.x_mm.clone();
        for tick in 0..120 {
            world.step_tick(tick).expect("tick steps");
        }
        assert_ne!(world.x_mm, before_x, "bodies must move");
        assert!(
            world.wish_x.iter().any(|w| *w != 0) || world.wish_z.iter().any(|w| *w != 0),
            "AI must write nonzero wishes"
        );
        assert!(
            world.hp.iter().any(|hp| *hp < 1_000),
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
        let mut world = SiegeTickWorld::new(11).expect("world builds");
        for index in 0..ENTITIES {
            world.x_mm[index] = 0;
            world.z_mm[index] = 0;
            world.hp[index] = 1_000;
        }
        world.step_simulate(11);
        world.x_mm[100] = 1_000_000;
        world.z_mm[100] = 0;
        // hp 999 against the 1000 baseline is the combat-dirty word the interest
        // baseline comparison owns; no side channel needed.
        world.hp[100] = 999;
        // Tick 12 selects the `viewer % 6 == 0` lane with the far LOD closed
        // for the siege tuning (far every 30 ticks: 12 is not a multiple
        // of 30). Every selecting viewer must still take combat-dirty 100.
        let bytes = world.snapshot_all(12).expect("snapshot");
        assert!(bytes > 0, "snapshot must carry bytes");
        assert!(
            world.out.contains(&100),
            "combat-dirty entity must send beyond the far ring"
        );
    }

    #[test]
    fn stagger_holds_10hz_per_viewer() {
        // The stagger spreads work without thinning replication: over 600
        // ticks every viewer selects exactly 100 snapshots (10 Hz), so a
        // collapsed lane fails here, not in the timing proof.
        let mut world = SiegeTickWorld::new(SIEGE_SEED).expect("world builds");
        for tick in 0..SIEGE_TICKS {
            world.step_tick(tick).expect("tick steps");
        }
        for (viewer, conn) in world.conns.iter().enumerate() {
            assert_eq!(
                conn.snapshots(),
                (SIEGE_TICKS / SNAPSHOT_EVERY_TICKS) as u64,
                "viewer {viewer} lost its 10 Hz lane"
            );
        }
    }

    #[test]
    fn steady_tick_holds_capacity_across_ticks() {
        // Steady-state allocation proxy (the net-crate convention, like the
        // interest `select_holds_capacity_across_ticks` gate): internal buffers
        // never regrow across 240 ticks and repeat decisions bit-identically.
        let mut world = SiegeTickWorld::new(SIEGE_SEED).expect("world builds");
        for tick in 0..60 {
            world.step_tick(tick).expect("warmup steps");
        }
        let before = world.capacities();
        for tick in 60..300 {
            world.step_tick(tick).expect("tick steps");
            assert_eq!(world.capacities(), before, "tick {tick} regrew a buffer");
        }
        let mut fresh = SiegeTickWorld::new(SIEGE_SEED).expect("world builds");
        for tick in 0..300 {
            fresh.step_tick(tick).expect("tick steps");
        }
        assert_eq!(world.x_mm, fresh.x_mm);
        assert_eq!(world.hp, fresh.hp);
    }

    #[test]
    fn capture_digest_matches_record_tape() {
        // The single-pass capture folds the same digest the two-pass tape
        // returns, so timing never leaks into hashed state.
        let taped = record_tape(SIEGE_SEED, 120).expect("tape");
        let captured = capture(SIEGE_SEED, 120).expect("capture");
        assert_eq!(captured.digest, taped);
        assert_eq!(captured.systems.len(), SYSTEMS.len());
        assert_eq!(
            captured
                .systems
                .iter()
                .map(|row| row.system)
                .collect::<Vec<_>>(),
            SYSTEMS,
            "breakdown names every system in order"
        );
        assert!(!captured.summary().is_empty());
    }

    #[test]
    fn rejects_bad_shapes_loudly() {
        assert_eq!(
            record_tape(SIEGE_SEED, 0),
            Err(TickBudgetError::BadTicks { got: 0 })
        );
        assert_eq!(
            record_tape(SIEGE_SEED, MAX_TICKS + 1),
            Err(TickBudgetError::BadTicks { got: MAX_TICKS + 1 })
        );
        assert_eq!(
            capture(SIEGE_SEED, 0),
            Err(TickBudgetError::BadTicks { got: 0 })
        );
        assert_eq!(
            TickQuantiles::of("tick", &[]),
            Err(TickBudgetError::EmptySamples { what: "tick" })
        );
        assert!(TickQuantiles::of("tick", &[1.0, f64::NAN]).is_err());
        assert!(TickQuantiles::of("tick", &[1.0, -0.5]).is_err());
        assert!(TickQuantiles::of("tick", &[1.0, f64::INFINITY]).is_err());
        assert!(check_p99(f64::NAN).is_err());
        let mut world = SiegeTickWorld::new(SIEGE_SEED).expect("world builds");
        world.out.truncate(8);
        assert_eq!(
            world.snapshot_all(6),
            Err(TickBudgetError::OutTooSmall {
                need: ENTITIES,
                have: 8
            })
        );
    }

    #[test]
    fn budget_gate_trips_with_both_sides() {
        check_p99(BUDGET_P99_MS).expect("exact budget passes");
        check_p99(0.0).expect("idle passes");
        let blown = check_p99(BUDGET_P99_MS + 0.5).expect_err("over budget must trip");
        assert_eq!(
            blown,
            TickBudgetError::OverBudget {
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
        let cases: &[TickBudgetError] = &[
            TickBudgetError::BadTicks { got: 0 },
            TickBudgetError::OverBudget {
                p99_ms: 5.0,
                budget_ms: 4.0,
            },
            TickBudgetError::OutTooSmall { need: 9, have: 3 },
            TickBudgetError::EmptySamples { what: "tick" },
            TickBudgetError::NonFiniteSample { what: "tick" },
            TickBudgetError::Interest(InterestError::BadConfig),
        ];
        for error in cases {
            let text = error.to_string();
            assert!(!text.is_empty(), "{error:?} explains itself");
            let _: &dyn std::error::Error = error;
        }
        assert!(TickBudgetError::BadTicks { got: 0 }
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
    fn world_shape_is_always_564() {
        let world = SiegeTickWorld::new(SIEGE_SEED).expect("world builds");
        assert_eq!(world.len(), ENTITIES);
        assert!(!world.is_empty());
    }

    #[test]
    #[ignore = "timing: siege p99 proof, run with --release -- --ignored --test-threads=1"]
    fn siege_tick_p99_within_budget() {
        let report = capture(SIEGE_SEED, SIEGE_TICKS).expect("capture runs");
        println!("{}", report.summary());
        report.check().expect("siege tick p99 within 4 ms");
        assert!(
            report.digest.starve_max <= STARVE_TICKS,
            "no entity starved over 500 ms"
        );
    }

    #[test]
    #[ignore = "timing: 10-minute siege p99 proof, run with --release -- --ignored --test-threads=1"]
    fn siege_tick_p99_10min_within_budget() {
        let report = capture(SIEGE_SEED, SIEGE_TICKS_10MIN).expect("capture runs");
        println!("{}", report.summary());
        let second = record_tape(SIEGE_SEED, SIEGE_TICKS_10MIN).expect("tape records");
        assert_eq!(report.digest, second, "10-minute siege must replay exactly");
        report
            .check()
            .expect("10-minute siege tick p99 within 4 ms");
    }
}
