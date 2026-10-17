//! budget: debris budgets for arena-scale destruction.
//!
//! Consumer: the arena match admits bonded-collection break chunks per event,
//! then steps the ledger each fixed tick with the camera pose; the renderer
//! reads LOD flags.
//! Units: integer mm counts, ticks. Determinism: integer-only math, shards
//! stored in canonical `(island, id)` order, so shuffled admit order and
//! reruns hash identically. Errors: [`BudgetError`]. Budget: `step` scans at
//! most 2048 shards once with integer math and skips sleeping islands, so
//! the amortized cost stays under 1 ms; only `admit` allocates, bounded by
//! the per-event cap.
//!
//! Mirror: shard/event shapes repeat `bonded.rs` so this file stays std-only
//! and `rustc --test` clean; callers map bonded-collection chunks into
//! [`ShardSeed`] when they admit a break.

use engine_core::Fnv64;

/// Canonical snapshot bytes and restore of the ledger (rollback resim).
mod snapshot;

/// Largest shard count one ledger holds; bounds memory up front.
pub const MAX_BUDGET_SHARDS: usize = 2048;
/// Ceiling for any per-event cap; bounds the admit sort up front.
pub const MAX_PER_EVENT: u32 = 64;
/// Largest batch one `admit` call accepts before truncating to the cap.
pub const MAX_ADMIT_BATCH: usize = 4096;
/// Largest accepted `sleep_ticks`; about a minute at 60 Hz.
pub const MAX_SLEEP_TICKS: u32 = 3600;
/// Largest accepted LOD distance; +-10 km covers any arena; mm.
pub const MAX_LOD_DISTANCE_MM: i32 = 10_000_000;
/// Largest accepted coordinate or camera value; mm.
pub const MAX_POS_MM: i32 = 10_000_000;
/// Largest accepted per-tick velocity component; mm/tick.
pub const MAX_VEL_MM_PER_TICK: i32 = 100_000;
/// Gravity applied to awake shards each tick; mm/tick^2.
pub const GRAVITY_MM_PER_TICK2: i32 = 3;
/// Fixed-point scale of [`DebrisLedger::sleep_ratio_q16`].
pub const RATIO_SCALE: u32 = 65536;

/// Canonical 8-fighter soak budget: 16 shards per event, 1024 total, sleep
/// after 30 still ticks, LOD swap past 4 m, still means at most 1 mm/tick.
pub fn arena_default_budget() -> DebrisBudget {
    DebrisBudget {
        per_event_cap: 16,
        max_shards: 1024,
        sleep_ticks: 30,
        lod_distance_mm: 4000,
        sleep_speed_mm_per_tick: 1,
    }
}

/// Per-event and ledger-wide budget knobs. All counts except distances.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DebrisBudget {
    /// Largest shard count one event admits; 1..=`MAX_PER_EVENT`.
    pub per_event_cap: u32,
    /// Largest shard count the ledger holds; 1..=`MAX_BUDGET_SHARDS`.
    pub max_shards: u32,
    /// Still ticks before an island sleeps; 1..=`MAX_SLEEP_TICKS`.
    pub sleep_ticks: u32,
    /// Distance past which shards render low LOD; 1..=`MAX_LOD_DISTANCE_MM`.
    pub lod_distance_mm: i32,
    /// Speed at or below which a shard counts as still; 0..=`MAX_VEL_MM_PER_TICK`.
    pub sleep_speed_mm_per_tick: i32,
}

impl DebrisBudget {
    /// Range-checks every knob without building a ledger.
    pub fn validate(&self) -> Result<(), BudgetError> {
        if self.per_event_cap == 0 || self.per_event_cap > MAX_PER_EVENT {
            return Err(BudgetError::BadPerEventCap);
        }
        if self.max_shards == 0 || self.max_shards as usize > MAX_BUDGET_SHARDS {
            return Err(BudgetError::BadMaxShards);
        }
        if self.sleep_ticks == 0 || self.sleep_ticks > MAX_SLEEP_TICKS {
            return Err(BudgetError::BadSleepTicks);
        }
        if self.lod_distance_mm <= 0 || self.lod_distance_mm > MAX_LOD_DISTANCE_MM {
            return Err(BudgetError::BadLodDistance);
        }
        if self.sleep_speed_mm_per_tick < 0 || self.sleep_speed_mm_per_tick > MAX_VEL_MM_PER_TICK {
            return Err(BudgetError::BadSleepSpeed);
        }
        Ok(())
    }
}

/// One shard offered by a break event. Mirrors the bonded-collection chunk
/// identity (index becomes `id`, support island becomes `island`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShardSeed {
    /// Shard id, unique across the ledger.
    pub id: u32,
    /// Support island; the whole island sleeps or wakes together.
    pub island: u32,
    /// Spawn position; mm, each within +-`MAX_POS_MM`.
    pub pos_mm: [i32; 3],
    /// Spawn velocity; mm/tick, each within +-`MAX_VEL_MM_PER_TICK`.
    pub vel_mm_per_tick: [i32; 3],
}

/// What one `admit` call kept and dropped; counts only.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AdmitReport {
    /// Seeds stored this call.
    pub admitted: u32,
    /// Seeds cut by the per-event cap (highest ids first).
    pub dropped_cap: u32,
    /// Seeds refused because the ledger already holds `max_shards`.
    pub dropped_full: u32,
}

/// Read-only view of one stored shard; counts and flags.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShardView {
    /// Shard id.
    pub id: u32,
    /// Support island.
    pub island: u32,
    /// Current position; mm.
    pub pos_mm: [i32; 3],
    /// Current velocity; mm/tick.
    pub vel_mm_per_tick: [i32; 3],
    /// Consecutive still ticks; saturates instead of wrapping.
    pub still_ticks: u32,
    /// True once the whole island cooled down; skips integration.
    pub sleeping: bool,
    /// True past the LOD distance; set from the camera each step.
    pub low_lod: bool,
}

/// Typed validation failures; external input maps here, never to a panic.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BudgetError {
    /// Per-event cap is zero or past `MAX_PER_EVENT`.
    BadPerEventCap,
    /// Ledger cap is zero or past `MAX_BUDGET_SHARDS`.
    BadMaxShards,
    /// Sleep ticks are zero or past `MAX_SLEEP_TICKS`.
    BadSleepTicks,
    /// LOD distance is non-positive or past `MAX_LOD_DISTANCE_MM`.
    BadLodDistance,
    /// Sleep speed is negative or past `MAX_VEL_MM_PER_TICK`.
    BadSleepSpeed,
    /// Batch holds more than `MAX_ADMIT_BATCH` seeds.
    TooManySeeds,
    /// Seed at the carried batch index has a bad coordinate or velocity.
    BadSeed(usize),
    /// Kick velocity is out of range.
    BadVelocity,
    /// Shard id already stored or repeated in the batch, id carried.
    DuplicateSeed(u32),
    /// No stored shard carries the id, id carried.
    UnknownShard(u32),
    /// Snapshot bytes are truncated, versioned wrong, out of range,
    /// misordered or trailing.
    BadSnapshot,
}

impl core::fmt::Display for BudgetError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            BudgetError::BadPerEventCap => write!(f, "per-event cap is out of bounds"),
            BudgetError::BadMaxShards => write!(f, "ledger cap is out of bounds"),
            BudgetError::BadSleepTicks => write!(f, "sleep ticks are out of bounds"),
            BudgetError::BadLodDistance => write!(f, "LOD distance is out of bounds"),
            BudgetError::BadSleepSpeed => write!(f, "sleep speed is out of bounds"),
            BudgetError::TooManySeeds => write!(f, "admit batch exceeds MAX_ADMIT_BATCH"),
            BudgetError::BadSeed(i) => write!(f, "admit seed {i} is out of bounds"),
            BudgetError::BadVelocity => write!(f, "kick velocity is out of bounds"),
            BudgetError::DuplicateSeed(id) => write!(f, "shard {id} is already stored"),
            BudgetError::UnknownShard(id) => write!(f, "shard {id} is not stored"),
            BudgetError::BadSnapshot => write!(f, "debris snapshot bytes are not valid"),
        }
    }
}

impl std::error::Error for BudgetError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Shard {
    id: u32,
    island: u32,
    pos_mm: [i32; 3],
    vel_mm_per_tick: [i32; 3],
    still_ticks: u32,
    sleeping: bool,
    low_lod: bool,
}

/// Debris ledger: capped shards in canonical `(island, id)` order with
/// sleep-island tracking and camera-distance LOD. The arena admits
/// bonded-collection break chunks per event and steps once per fixed tick.
#[derive(Clone, Debug)]
pub struct DebrisLedger {
    cfg: DebrisBudget,
    shards: Vec<Shard>,
    tick: u32,
    last_admit_tick: u32,
    admitted_total: u64,
    dropped_total: u64,
    lod_swaps: u64,
}

impl DebrisLedger {
    /// Builds an empty ledger; reserves the full cap once so steps never
    /// grow the allocation.
    pub fn new(cfg: &DebrisBudget) -> Result<Self, BudgetError> {
        cfg.validate()?;
        Ok(Self {
            cfg: *cfg,
            shards: Vec::with_capacity(cfg.max_shards as usize),
            tick: 0,
            last_admit_tick: 0,
            admitted_total: 0,
            dropped_total: 0,
            lod_swaps: 0,
        })
    }

    /// Budget knobs of this ledger.
    pub fn config(&self) -> DebrisBudget {
        self.cfg
    }

    /// Stored shard count, always within `max_shards`.
    pub fn len(&self) -> usize {
        self.shards.len()
    }

    /// True when the ledger holds no shards.
    pub fn is_empty(&self) -> bool {
        self.shards.is_empty()
    }

    /// Ledger tick; counts accepted steps only.
    pub fn tick(&self) -> u32 {
        self.tick
    }

    /// Shards admitted across all calls; count.
    pub fn admitted_total(&self) -> u64 {
        self.admitted_total
    }

    /// Shards dropped across all calls (cap plus full); count.
    pub fn dropped_total(&self) -> u64 {
        self.dropped_total
    }

    /// Cumulative LOD flag flips; count.
    pub fn lod_swaps(&self) -> u64 {
        self.lod_swaps
    }

    /// Sleeping shard count.
    pub fn sleeping_count(&self) -> usize {
        self.shards.iter().filter(|s| s.sleeping).count()
    }

    /// Low-LOD shard count.
    pub fn low_lod_count(&self) -> usize {
        self.shards.iter().filter(|s| s.low_lod).count()
    }

    /// Sleeping share in Q16 (`RATIO_SCALE` == all asleep); zero when empty.
    pub fn sleep_ratio_q16(&self) -> u32 {
        if self.shards.is_empty() {
            return 0;
        }
        (self.sleeping_count() as u64 * u64::from(RATIO_SCALE) / self.shards.len() as u64) as u32
    }

    /// View of the shard at canonical index; `None` when out of range.
    pub fn shard_at(&self, index: usize) -> Option<ShardView> {
        self.shards.get(index).map(|s| ShardView {
            id: s.id,
            island: s.island,
            pos_mm: s.pos_mm,
            vel_mm_per_tick: s.vel_mm_per_tick,
            still_ticks: s.still_ticks,
            sleeping: s.sleeping,
            low_lod: s.low_lod,
        })
    }

    /// True when the stored shard sleeps; unknown ids read false.
    pub fn is_sleeping(&self, id: u32) -> bool {
        self.shards.iter().any(|s| s.id == id && s.sleeping)
    }

    /// True when the stored shard renders low LOD; unknown ids read false.
    pub fn is_low_lod(&self, id: u32) -> bool {
        self.shards.iter().any(|s| s.id == id && s.low_lod)
    }

    /// True when every shard of the island sleeps; `None` when the island
    /// holds no shards.
    pub fn island_sleeping(&self, island: u32) -> Option<bool> {
        let mut found = false;
        for s in &self.shards {
            if s.island == island {
                found = true;
                if !s.sleeping {
                    return Some(false);
                }
            }
        }
        if found {
            Some(true)
        } else {
            None
        }
    }

    /// Admits one break event: validates the whole batch first (over-large
    /// batches, bad coordinates, duplicates change nothing), then keeps the
    /// lowest ids up to the per-event cap in canonical order. Stored shards
    /// stay; newcomers past `max_shards` drop. The report carries counts.
    pub fn admit(&mut self, tick: u32, seeds: &[ShardSeed]) -> Result<AdmitReport, BudgetError> {
        if seeds.len() > MAX_ADMIT_BATCH {
            return Err(BudgetError::TooManySeeds);
        }
        for (i, seed) in seeds.iter().enumerate() {
            if !seed_in_range(seed) {
                return Err(BudgetError::BadSeed(i));
            }
        }
        // Lowest ids win, so sort batch indices by id; the input order
        // never affects who survives the cap.
        let mut order: Vec<usize> = (0..seeds.len()).collect();
        order.sort_by_key(|&i| seeds[i].id);
        for window in order.windows(2) {
            if seeds[window[0]].id == seeds[window[1]].id {
                return Err(BudgetError::DuplicateSeed(seeds[window[1]].id));
            }
        }
        for &i in &order {
            if self.shards.iter().any(|s| s.id == seeds[i].id) {
                return Err(BudgetError::DuplicateSeed(seeds[i].id));
            }
        }
        let keep = order.len().min(self.cfg.per_event_cap as usize);
        let mut report = AdmitReport {
            admitted: 0,
            dropped_cap: (order.len() - keep) as u32,
            dropped_full: 0,
        };
        for &i in order.iter().take(keep) {
            if self.shards.len() >= self.cfg.max_shards as usize {
                report.dropped_full += 1;
                continue;
            }
            let seed = &seeds[i];
            let shard = Shard {
                id: seed.id,
                island: seed.island,
                pos_mm: seed.pos_mm,
                vel_mm_per_tick: seed.vel_mm_per_tick,
                still_ticks: 0,
                sleeping: false,
                low_lod: false,
            };
            // Canonical `(island, id)` order keeps islands contiguous so
            // the sleep pass runs in one scan without auxiliary storage.
            let at = self
                .shards
                .iter()
                .position(|s| (s.island, s.id) > (shard.island, shard.id));
            match at {
                Some(at) => self.shards.insert(at, shard),
                None => self.shards.push(shard),
            }
            report.admitted += 1;
        }
        if report.admitted > 0 {
            self.last_admit_tick = tick;
        }
        self.admitted_total += u64::from(report.admitted);
        self.dropped_total += u64::from(report.dropped_cap + report.dropped_full);
        Ok(report)
    }

    /// Wakes one shard with a fresh velocity: still count clears and the
    /// island leaves sleep on the next step. Unknown ids refuse loudly.
    pub fn kick(&mut self, id: u32, vel_mm_per_tick: [i32; 3]) -> Result<(), BudgetError> {
        for v in vel_mm_per_tick {
            if !(-MAX_VEL_MM_PER_TICK..=MAX_VEL_MM_PER_TICK).contains(&v) {
                return Err(BudgetError::BadVelocity);
            }
        }
        match self.shards.iter_mut().find(|s| s.id == id) {
            Some(shard) => {
                shard.vel_mm_per_tick = vel_mm_per_tick;
                shard.still_ticks = 0;
                shard.sleeping = false;
                Ok(())
            }
            None => Err(BudgetError::UnknownShard(id)),
        }
    }

    /// Advances one tick: integrates awake shards with integer gravity and
    /// a ground clamp (contact stops the shard), cools still counters, then
    /// sleeps islands whose every shard cooled for `sleep_ticks`. LOD flags
    /// refresh from the camera every step, asleep or not. Out-of-range
    /// camera values clamp; the tick still advances.
    pub fn step(&mut self, camera_mm: [i32; 3]) {
        let camera = [
            camera_mm[0].clamp(-MAX_POS_MM, MAX_POS_MM),
            camera_mm[1].clamp(-MAX_POS_MM, MAX_POS_MM),
            camera_mm[2].clamp(-MAX_POS_MM, MAX_POS_MM),
        ];
        self.tick = self.tick.saturating_add(1);
        let sleep_limit = self.cfg.sleep_ticks;
        let still_limit_sq = i64::from(self.cfg.sleep_speed_mm_per_tick)
            * i64::from(self.cfg.sleep_speed_mm_per_tick);
        let lod_sq = i64::from(self.cfg.lod_distance_mm) * i64::from(self.cfg.lod_distance_mm);
        for shard in self.shards.iter_mut() {
            if !shard.sleeping {
                shard.pos_mm[0] = shard.pos_mm[0].saturating_add(shard.vel_mm_per_tick[0]);
                shard.pos_mm[1] = shard.pos_mm[1].saturating_add(shard.vel_mm_per_tick[1]);
                shard.pos_mm[2] = shard.pos_mm[2].saturating_add(shard.vel_mm_per_tick[2]);
                shard.vel_mm_per_tick[1] =
                    shard.vel_mm_per_tick[1].saturating_sub(GRAVITY_MM_PER_TICK2);
                if shard.pos_mm[1] <= 0 {
                    shard.pos_mm[1] = 0;
                    shard.vel_mm_per_tick = [0, 0, 0];
                }
                let speed_sq = i64::from(shard.vel_mm_per_tick[0])
                    * i64::from(shard.vel_mm_per_tick[0])
                    + i64::from(shard.vel_mm_per_tick[1]) * i64::from(shard.vel_mm_per_tick[1])
                    + i64::from(shard.vel_mm_per_tick[2]) * i64::from(shard.vel_mm_per_tick[2]);
                if speed_sq <= still_limit_sq {
                    shard.still_ticks = shard.still_ticks.saturating_add(1);
                } else {
                    shard.still_ticks = 0;
                }
            }
            let dx = i64::from(shard.pos_mm[0] - camera[0]);
            let dy = i64::from(shard.pos_mm[1] - camera[1]);
            let dz = i64::from(shard.pos_mm[2] - camera[2]);
            let low = dx * dx + dy * dy + dz * dz > lod_sq;
            if low != shard.low_lod {
                shard.low_lod = low;
                self.lod_swaps = self.lod_swaps.saturating_add(1);
            }
        }
        // Islands are contiguous in canonical order: one scan sleeps runs
        // whose every shard cooled long enough.
        let mut run_start = 0;
        while run_start < self.shards.len() {
            let island = self.shards[run_start].island;
            let mut run_end = run_start + 1;
            while run_end < self.shards.len() && self.shards[run_end].island == island {
                run_end += 1;
            }
            let mut cooled = true;
            for shard in &self.shards[run_start..run_end] {
                if shard.still_ticks < sleep_limit {
                    cooled = false;
                    break;
                }
            }
            if cooled {
                for shard in &mut self.shards[run_start..run_end] {
                    shard.sleeping = true;
                }
            }
            run_start = run_end;
        }
    }

    /// FNV-1a64 over ticks, knobs and shards in canonical order with
    /// bit-exact integers, so reruns hash identically.
    pub fn state_hash(&self) -> u64 {
        let mut h = Fnv64::new();
        h.write_u32(self.tick);
        h.write_u32(self.last_admit_tick);
        h.write_u32(self.cfg.per_event_cap);
        h.write_u32(self.cfg.max_shards);
        h.write_u32(self.cfg.sleep_ticks);
        h.write_i32(self.cfg.lod_distance_mm);
        h.write_i32(self.cfg.sleep_speed_mm_per_tick);
        h.write_len32(self.shards.len());
        for s in &self.shards {
            h.write_u32(s.id);
            h.write_u32(s.island);
            h.write_i32(s.pos_mm[0]);
            h.write_i32(s.pos_mm[1]);
            h.write_i32(s.pos_mm[2]);
            h.write_i32(s.vel_mm_per_tick[0]);
            h.write_i32(s.vel_mm_per_tick[1]);
            h.write_i32(s.vel_mm_per_tick[2]);
            h.write_u32(s.still_ticks);
            h.write_u32(u32::from(s.sleeping));
            h.write_u32(u32::from(s.low_lod));
        }
        h.write_u32((self.admitted_total & 0xffff_ffff) as u32);
        h.write_u32((self.dropped_total & 0xffff_ffff) as u32);
        h.write_u32((self.lod_swaps & 0xffff_ffff) as u32);
        h.finish()
    }
}

fn seed_in_range(seed: &ShardSeed) -> bool {
    for c in seed.pos_mm {
        if !(-MAX_POS_MM..=MAX_POS_MM).contains(&c) {
            return false;
        }
    }
    for v in seed.vel_mm_per_tick {
        if !(-MAX_VEL_MM_PER_TICK..=MAX_VEL_MM_PER_TICK).contains(&v) {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seed(id: u32, island: u32) -> ShardSeed {
        ShardSeed {
            id,
            island,
            pos_mm: [0, 5000, 0],
            vel_mm_per_tick: [0, 0, 0],
        }
    }

    fn soak_script() -> (u64, usize, u32, u64) {
        // 8 fighters break one crate each every 30 ticks over 300 ticks;
        // the camera orbits so LOD flags keep flipping deterministically.
        let cfg = arena_default_budget();
        let mut ledger = DebrisLedger::new(&cfg).expect("valid soak budget");
        let posts: [[i32; 3]; 8] = [
            [-6000, 0, -6000],
            [6000, 0, -6000],
            [-6000, 0, 6000],
            [6000, 0, 6000],
            [-6000, 0, 0],
            [6000, 0, 0],
            [0, 0, -6000],
            [0, 0, 6000],
        ];
        let mut rng: u64 = 0xD7A4_5EED_0001;
        let mut next_id: u32 = 0;
        let step_rng = |state: &mut u64| {
            *state ^= *state << 13;
            *state ^= *state >> 7;
            *state ^= *state << 17;
            *state
        };
        for tick in 0..300 {
            for (fighter, post) in posts.iter().enumerate() {
                if (tick + fighter as u32 * 7).is_multiple_of(30) {
                    let island = next_id;
                    let mut batch = Vec::new();
                    for _ in 0..20 {
                        let r = step_rng(&mut rng);
                        batch.push(ShardSeed {
                            id: next_id,
                            island,
                            pos_mm: [
                                post[0] + (r % 1000) as i32,
                                3000 + (r / 1000 % 2000) as i32,
                                post[2] + (r / 1_000_000 % 1000) as i32,
                            ],
                            vel_mm_per_tick: [
                                (r / 7 % 21) as i32 - 10,
                                20 + (r / 13 % 20) as i32,
                                (r / 29 % 21) as i32 - 10,
                            ],
                        });
                        next_id += 1;
                    }
                    let report = ledger.admit(tick, &batch).expect("soak admit valid");
                    assert!(
                        report.admitted <= cfg.per_event_cap,
                        "per-event cap held: {report:?}"
                    );
                }
            }
            // Integer orbit, no float math: radius 2000 mm, period 120 ticks.
            let angle = (tick % 120) as i64;
            let (cx, cz) = if angle < 30 {
                (angle * 2000 / 30, 0)
            } else if angle < 60 {
                (2000 - (angle - 30) * 4000 / 30, 0)
            } else if angle < 90 {
                (-2000, (angle - 60) * 2000 / 30)
            } else {
                (
                    -2000 + (angle - 90) * 4000 / 30,
                    2000 - (angle - 90) * 2000 / 30,
                )
            };
            ledger.step([cx as i32, 1500, cz as i32]);
            assert!(
                ledger.len() <= cfg.max_shards as usize,
                "ledger cap held at tick {tick}"
            );
        }
        (
            ledger.state_hash(),
            ledger.len(),
            ledger.sleep_ratio_q16(),
            ledger.lod_swaps(),
        )
    }

    #[test]
    fn state_hash_is_fnv1a_over_the_canonical_word_stream() {
        let cfg = DebrisBudget {
            lod_distance_mm: 3000,
            ..arena_default_budget()
        };
        let mut ledger = DebrisLedger::new(&cfg).expect("valid budget");
        // Negative positions and velocities cover the signed-word encoding.
        let batch = [
            ShardSeed {
                id: 2,
                island: 1,
                pos_mm: [-1500, 4200, -30],
                vel_mm_per_tick: [-7, 31, -2],
            },
            ShardSeed {
                id: 5,
                island: 1,
                pos_mm: [900, 100, 8000],
                vel_mm_per_tick: [3, -4, 5],
            },
        ];
        ledger.admit(2, &batch).expect("valid batch");
        for _ in 0..5 {
            ledger.step([0, 1500, 0]);
        }
        // Oracle: every word the hash documents, spelled as plain LE bytes
        // (signed values through their two's-complement `u32`).
        let mut words: Vec<u32> = vec![
            ledger.tick,
            ledger.last_admit_tick,
            ledger.cfg.per_event_cap,
            ledger.cfg.max_shards,
            ledger.cfg.sleep_ticks,
            ledger.cfg.lod_distance_mm as u32,
            ledger.cfg.sleep_speed_mm_per_tick as u32,
            ledger.shards.len() as u32,
        ];
        for s in &ledger.shards {
            words.extend([s.id, s.island]);
            words.extend(s.pos_mm.map(|v| v as u32));
            words.extend(s.vel_mm_per_tick.map(|v| v as u32));
            words.extend([s.still_ticks, u32::from(s.sleeping), u32::from(s.low_lod)]);
        }
        words.extend([
            (ledger.admitted_total & 0xffff_ffff) as u32,
            (ledger.dropped_total & 0xffff_ffff) as u32,
            (ledger.lod_swaps & 0xffff_ffff) as u32,
        ]);
        let bytes: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
        assert_eq!(ledger.len(), 2);
        assert_eq!(ledger.state_hash(), engine_core::hash_bytes(&bytes));
    }

    #[test]
    fn per_event_cap_keeps_lowest_ids() {
        let cfg = DebrisBudget {
            per_event_cap: 4,
            ..arena_default_budget()
        };
        let mut ledger = DebrisLedger::new(&cfg).expect("valid budget");
        // Offered high-id-first on purpose: the cap must not favor order.
        let batch: Vec<ShardSeed> = (0..10).rev().map(|id| seed(id, 99)).collect();
        let report = ledger.admit(3, &batch).expect("valid batch");
        assert_eq!(
            report,
            AdmitReport {
                admitted: 4,
                dropped_cap: 6,
                dropped_full: 0,
            }
        );
        let ids: Vec<u32> = (0..ledger.len())
            .map(|i| ledger.shard_at(i).expect("stored").id)
            .collect();
        assert_eq!(ids, vec![0, 1, 2, 3]);
    }

    #[test]
    fn ledger_cap_refuses_newcomers_and_counts_drops() {
        let cfg = DebrisBudget {
            per_event_cap: 8,
            max_shards: 6,
            ..arena_default_budget()
        };
        let mut ledger = DebrisLedger::new(&cfg).expect("valid budget");
        let first: Vec<ShardSeed> = (0..6).map(|id| seed(id, 1)).collect();
        let report = ledger.admit(1, &first).expect("fits");
        assert_eq!(report.admitted, 6);
        let second: Vec<ShardSeed> = (6..12).map(|id| seed(id, 2)).collect();
        let report = ledger.admit(2, &second).expect("overflows count as drops");
        assert_eq!(report.admitted, 0);
        assert_eq!(report.dropped_full, 6);
        assert_eq!(ledger.len(), 6);
        assert_eq!(ledger.dropped_total(), 6);
    }

    #[test]
    fn invalid_config_rejected() {
        let good = arena_default_budget();
        for cfg in [
            DebrisBudget {
                per_event_cap: 0,
                ..good
            },
            DebrisBudget {
                per_event_cap: MAX_PER_EVENT + 1,
                ..good
            },
            DebrisBudget {
                max_shards: 0,
                ..good
            },
            DebrisBudget {
                max_shards: MAX_BUDGET_SHARDS as u32 + 1,
                ..good
            },
            DebrisBudget {
                sleep_ticks: 0,
                ..good
            },
            DebrisBudget {
                lod_distance_mm: 0,
                ..good
            },
            DebrisBudget {
                lod_distance_mm: -50,
                ..good
            },
            DebrisBudget {
                sleep_speed_mm_per_tick: -1,
                ..good
            },
        ] {
            assert!(DebrisLedger::new(&cfg).is_err(), "cfg {cfg:?} must fail");
        }
    }

    #[test]
    fn bad_seeds_and_duplicates_rejected_without_mutation() {
        let mut ledger = DebrisLedger::new(&arena_default_budget()).expect("valid budget");
        let before = ledger.state_hash();
        let far = ShardSeed {
            pos_mm: [MAX_POS_MM + 1, 0, 0],
            ..seed(1, 1)
        };
        assert_eq!(
            ledger.admit(1, &[far]).unwrap_err(),
            BudgetError::BadSeed(0)
        );
        let fast = ShardSeed {
            vel_mm_per_tick: [0, MAX_VEL_MM_PER_TICK + 1, 0],
            ..seed(1, 1)
        };
        assert_eq!(
            ledger.admit(1, &[fast]).unwrap_err(),
            BudgetError::BadSeed(0)
        );
        let dup = [seed(2, 1), seed(2, 1)];
        assert_eq!(
            ledger.admit(1, &dup).unwrap_err(),
            BudgetError::DuplicateSeed(2)
        );
        ledger.admit(1, &[seed(3, 1)]).expect("first store works");
        assert_eq!(
            ledger.admit(2, &[seed(3, 2)]).unwrap_err(),
            BudgetError::DuplicateSeed(3)
        );
        assert_eq!(
            ledger.kick(77, [0, 0, 0]).unwrap_err(),
            BudgetError::UnknownShard(77)
        );
        assert_eq!(
            ledger.kick(3, [0, MAX_VEL_MM_PER_TICK + 1, 0]).unwrap_err(),
            BudgetError::BadVelocity
        );
        // Nothing stored, dropped or hashed by the refused calls: the ledger
        // matches a clean replay holding only the one accepted seed.
        let mut clean = DebrisLedger::new(&arena_default_budget()).expect("valid");
        clean.admit(1, &[seed(3, 1)]).expect("replay");
        assert_eq!(ledger.len(), clean.len());
        assert_eq!(ledger.dropped_total(), 0);
        assert_eq!(ledger.state_hash(), clean.state_hash());
        assert_ne!(ledger.state_hash(), before);
    }

    #[test]
    fn sleep_island_cools_together_and_kick_wakes() {
        let cfg = DebrisBudget {
            sleep_ticks: 5,
            sleep_speed_mm_per_tick: 0,
            ..arena_default_budget()
        };
        let mut ledger = DebrisLedger::new(&cfg).expect("valid budget");
        // Island 7 spawns grounded so it cools at once; island 8 keeps one
        // moving shard aloft, so the island must stay awake.
        let mut rest_a = seed(0, 7);
        rest_a.pos_mm = [100, 0, 0];
        let mut rest_b = seed(1, 7);
        rest_b.pos_mm = [-100, 0, 0];
        ledger.admit(0, &[rest_a, rest_b]).expect("island 7");
        let mut mover = seed(2, 8);
        mover.pos_mm = [0, 9000, 0];
        mover.vel_mm_per_tick = [60, 0, 0];
        let mut settler = seed(3, 8);
        settler.pos_mm = [0, 0, 0];
        ledger.admit(0, &[mover, settler]).expect("island 8");
        for _ in 0..6 {
            ledger.step([0, 1500, 0]);
        }
        assert_eq!(ledger.island_sleeping(7), Some(true));
        assert_eq!(ledger.island_sleeping(8), Some(false));
        assert!(ledger.is_sleeping(0) && ledger.is_sleeping(1));
        assert!(!ledger.is_sleeping(2) && !ledger.is_sleeping(3));
        assert_eq!(ledger.island_sleeping(404), None);
        // A kick wakes one shard; the island reports awake again.
        ledger.kick(0, [10, 0, 0]).expect("kick works");
        assert!(!ledger.is_sleeping(0));
        assert_eq!(ledger.island_sleeping(7), Some(false));
        assert!(ledger.sleep_ratio_q16() < RATIO_SCALE);
    }

    #[test]
    fn lod_swaps_count_camera_crossings() {
        let mut ledger = DebrisLedger::new(&arena_default_budget()).expect("valid budget");
        let mut shard = seed(0, 1);
        shard.pos_mm = [6000, 0, 0];
        ledger.admit(0, &[shard]).expect("far shard");
        assert_eq!(ledger.lod_swaps(), 0);
        // 6 m away with a 4 m LOD line: first step flips to low LOD.
        ledger.step([0, 0, 0]);
        assert!(ledger.is_low_lod(0));
        assert_eq!(ledger.lod_swaps(), 1);
        // Camera walks to the shard: flips back exactly once.
        ledger.step([6000, 0, 0]);
        assert!(!ledger.is_low_lod(0));
        assert_eq!(ledger.lod_swaps(), 2);
        // A still camera flips nothing more.
        ledger.step([6000, 0, 0]);
        assert_eq!(ledger.lod_swaps(), 2);
        assert_eq!(ledger.low_lod_count(), 0);
    }

    #[test]
    fn shuffled_admit_orders_hash_equal() {
        // Insertion shuffled 8 ways still stores the same canonical ledger.
        let batch: Vec<ShardSeed> = (0..24).map(|id| seed(id, id % 3)).collect();
        let reference = {
            let mut ledger = DebrisLedger::new(&arena_default_budget()).expect("valid");
            ledger.admit(9, &batch).expect("ordered admit");
            for _ in 0..40 {
                ledger.step([1000, 1500, 1000]);
            }
            ledger.state_hash()
        };
        let mut rng: u64 = 0x5EED_1234;
        for _ in 0..8 {
            let mut shuffled = batch.clone();
            for i in (1..shuffled.len()).rev() {
                rng ^= rng << 13;
                rng ^= rng >> 7;
                rng ^= rng << 17;
                shuffled.swap(i, (rng % (i as u64 + 1)) as usize);
            }
            // The cap keeps the same lowest ids whatever the order, so the
            // stepped ledger must match the ordered reference exactly.
            let mut ledger = DebrisLedger::new(&arena_default_budget()).expect("valid");
            ledger.admit(9, &shuffled).expect("shuffled admit");
            for _ in 0..40 {
                ledger.step([1000, 1500, 1000]);
            }
            assert_eq!(ledger.state_hash(), reference);
        }
    }

    #[test]
    fn soak_8_fighters_300_ticks_holds_caps_and_hashes_twice() {
        let (h1, n1, ratio1, swaps1) = soak_script();
        let (h2, n2, ratio2, swaps2) = soak_script();
        assert_eq!((h1, n1, ratio1, swaps1), (h2, n2, ratio2, swaps2));
        assert!(n1 <= arena_default_budget().max_shards as usize);
        assert!(n1 > 0, "the soak must admit shards");
        assert!(ratio1 > 0, "grounded shards must sleep: ratio {ratio1}");
        assert!(swaps1 > 0, "the orbiting camera must flip LOD: {swaps1}");
        println!("soak shards={n1} sleep_q16={ratio1} lod_swaps={swaps1} hash={h1:016x}");
    }

    #[test]
    fn steps_hold_capacity_without_growth() {
        // Proxy for no steady-state allocation: a full soak of steps never
        // grows the shard allocation past the one admit-time reserve.
        let mut ledger = DebrisLedger::new(&arena_default_budget()).expect("valid");
        let batch: Vec<ShardSeed> = (0..16).map(|id| seed(id, id % 4)).collect();
        ledger.admit(0, &batch).expect("prefill");
        let cap = ledger.shards.capacity();
        for t in 0..300 {
            ledger.step([t % 500, 1500, 0]);
        }
        assert_eq!(ledger.shards.capacity(), cap);
        assert_eq!(ledger.tick(), 300);
    }

    #[test]
    fn hash_moves_with_state() {
        let empty_a = DebrisLedger::new(&arena_default_budget()).expect("valid");
        let mut empty_b = DebrisLedger::new(&arena_default_budget()).expect("valid");
        assert_eq!(empty_a.state_hash(), empty_b.state_hash());
        empty_b.step([0, 1500, 0]);
        assert_ne!(empty_a.state_hash(), empty_b.state_hash());
        let other_cfg = DebrisBudget {
            lod_distance_mm: 8000,
            ..arena_default_budget()
        };
        let other = DebrisLedger::new(&other_cfg).expect("valid");
        assert_ne!(empty_a.state_hash(), other.state_hash());
    }
}
