//! islands: deterministic island builder over contact pairs. Same pairs, any order -> same bytes.
//!
//! Topology plus hashes live in [`crate::islands_build`]; sleep and the
//! parallel island step stay here. Re-exports keep the single
//! [`crate::islands`] path for existing callers.
//!
//! [`step_islands`] steps independent units (one Rapier world per fighter,
//! say) as jobs on a persistent [`JobPool`]. Units joined by a coupling
//! pair form one island and step together, members in ascending order, in
//! one job. Islands are cut into chunks by count alone, lanes take chunks
//! round-robin, and results merge by chunk index, so neither the lane count
//! nor the pool size reaches state or result order: 1, 2, 4 and 8 threads
//! give the same bytes. Units must not share mutable state (each owns its
//! world); Rapier's own `parallel` feature is not used. Errors:
//! [`ParError`], nothing stepped. Budget: a few small vectors per call
//! (units plus islands) and one boxed job per extra lane; no thread is
//! spawned per call, and the sim thread waits for every job before it
//! returns.
#![forbid(unsafe_code)]

use engine_ecs::JobPool;

pub use crate::islands_build::{build_islands, combined_hash, island_hash, roots};

/// Lane-unrolled float slices, allocation-free; the island sleep fold reads
/// its peak through [`simd_math::island_peak`].
#[path = "simd_math.rs"]
pub mod simd_math;

/// Most lanes one island step may fan out to.
pub const MAX_THREADS: usize = 64;

/// How [`step_islands`] cuts and schedules its work.
///
/// Chunk edges depend on the island count and `chunk` only, never on
/// `threads`, so every lane count visits the same chunks and merges them
/// in the same order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ParConfig {
    /// Lanes, `1..=MAX_THREADS`: lane 0 runs on the caller and every other
    /// lane is one pool job, so `1` runs everything on the caller. The
    /// pool's worker count bounds how many lanes really run at once.
    pub threads: usize,
    /// Islands per chunk, at least `1`.
    pub chunk: usize,
}

impl ParConfig {
    /// Range-checks the config.
    pub fn validate(&self) -> Result<(), ParError> {
        if self.threads == 0 || self.threads > MAX_THREADS {
            return Err(ParError::BadThreads(self.threads));
        }
        if self.chunk == 0 {
            return Err(ParError::BadChunk);
        }
        Ok(())
    }
}

/// Chunked sleeping split beside [`ParConfig`]: far static groups (every
/// member asleep) go to sleeping, near dynamics (any member awake, empty,
/// or unknown) stay awake in index order, awake cut into chunks of `chunk`
/// by count alone; `None` asleep means all awake (`partition` path).
/// Units: island indices. Determinism: index order, count-only chunk edges,
/// no clock, no map. Errors: none (unknown fails awake). Budget: two index
/// vectors sized by the island count. Caller: [`partition`].
pub fn split_sleeping(
    islands: &[Vec<usize>],
    asleep: Option<&[bool]>,
    chunk: usize,
) -> (Vec<Vec<usize>>, Vec<usize>) {
    let width = chunk.max(1);
    let mut awake: Vec<usize> = Vec::new();
    let mut sleeping: Vec<usize> = Vec::new();
    for (idx, members) in islands.iter().enumerate() {
        let mut rests = !members.is_empty();
        if let Some(flags) = asleep {
            for &m in members {
                match flags.get(m) {
                    Some(true) => {}
                    _ => {
                        rests = false;
                        break;
                    }
                }
            }
        }
        if rests && asleep.is_some() {
            sleeping.push(idx);
        } else {
            awake.push(idx);
        }
    }
    let mut chunks: Vec<Vec<usize>> = Vec::new();
    for (pos, idx) in awake.into_iter().enumerate() {
        if pos % width == 0 {
            chunks.push(Vec::new());
        }
        if let Some(back) = chunks.last_mut() {
            back.push(idx);
        }
    }
    (chunks, sleeping)
}

/// Committed sleep chunks beside [`split_sleeping`]: the Rapier sleep-commit
/// discipline in index terms (`PORT(dimforge/rapier@475caa3ff4646fd9a796b67d14116256f4962bc7:src/dynamics/island_manager/sleep.rs)`,
/// idea-only, zero donor bytes). Whole-cover sleeps in place with an epoch
/// bump; partial cover extracts each chunk into its own sleeping island, so
/// epoch-keyed caches tell a membership change from a quiet tick. Units:
/// island indices. Determinism: ascending index order throughout, count-only
/// edges, no clock, no map. Errors: none (unknown fails awake, overlap stays
/// awake). Budget: a few index vectors sized by the member count. Caller: the
/// solver sleep-commit site once it stages eligible chunks; `commit_sleeping`
/// tests pin the shape until it is wired.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SleepCommit {
    /// Bodies that stay awake in ascending order; `None` when one chunk
    /// covered the whole island, so nothing stays awake.
    pub awake: Option<Vec<usize>>,
    /// One sleeping island per committed chunk, each ascending, islands in
    /// first-member order.
    pub sleeping: Vec<Vec<usize>>,
    /// Cache epoch: `wrapping_add(1)` only when the island sleeps whole,
    /// held when chunks extract beside awake stragglers.
    pub epoch: u64,
}

/// Commit disjoint sleep-eligible `chunks` of one awake island.
///
/// Each chunk is sorted ascending with out-of-range ends dropped; empty
/// chunks are ignored; chunk members outside `awake` are unknown and stay
/// awake (never sleep what was not listed). Chunks must be disjoint subsets
/// of `awake`: a member listed twice stays awake and leaves every chunk.
/// One chunk holding every awake member sleeps the island in place (`awake`
/// is `None`, `epoch` bumps); otherwise each chunk becomes its own sleeping
/// island in first-member order, `awake` holds the ascending remainder and
/// `epoch` is unchanged.
#[must_use]
pub fn commit_sleeping_chunks(awake: &[usize], chunks: &[Vec<usize>], epoch: u64) -> SleepCommit {
    let mut members: Vec<usize> = awake.to_vec();
    members.sort_unstable();
    members.dedup();
    // Filter chunks to known members; unknown ends rejoin the awake rest.
    let mut stragglers: Vec<usize> = Vec::new();
    let mut kept: Vec<Vec<usize>> = Vec::new();
    for chunk in chunks {
        let mut sorted = chunk.clone();
        sorted.sort_unstable();
        sorted.dedup();
        let mut known = Vec::with_capacity(sorted.len());
        for body in sorted {
            if members.binary_search(&body).is_ok() {
                known.push(body);
            } else {
                stragglers.push(body);
            }
        }
        if !known.is_empty() {
            kept.push(known);
        }
    }
    // A member in two chunks stays awake: cut it from every chunk. Runs in
    // one sorted sweep, so chunk order never reaches the answer.
    let mut all: Vec<usize> = kept.iter().flatten().copied().collect();
    all.sort_unstable();
    let mut doubled: Vec<usize> = Vec::new();
    let mut at = 0;
    while at < all.len() {
        let mut end = at + 1;
        while end < all.len() && all[end] == all[at] {
            end += 1;
        }
        if end - at > 1 {
            doubled.push(all[at]);
        }
        at = end;
    }
    for island in &mut kept {
        island.retain(|body| doubled.binary_search(body).is_err());
    }
    kept.retain(|island| !island.is_empty());
    kept.sort_unstable_by_key(|island| island[0]);
    stragglers.sort_unstable();
    stragglers.dedup();
    if kept.len() == 1 && stragglers.is_empty() && kept[0] == members {
        return SleepCommit {
            awake: None,
            sleeping: kept,
            epoch: epoch.wrapping_add(1),
        };
    }
    let mut asleep: Vec<usize> = kept.iter().flatten().copied().collect();
    asleep.sort_unstable();
    let mut rest: Vec<usize> = members
        .into_iter()
        .filter(|body| asleep.binary_search(body).is_err())
        .chain(stragglers)
        .collect();
    rest.sort_unstable();
    rest.dedup();
    SleepCommit {
        awake: Some(rest),
        sleeping: kept,
        epoch,
    }
}

/// Sorted merge beside [`ParConfig`]: chunk results by chunk index, island
/// order inside, so 1/2/4/8 threads give the same bytes. Units: chunk
/// indices plus results. Determinism: `sort_unstable` by index, no clock.
/// Errors: none. Caller: [`step_islands`].
pub fn merge_sorted<R>(mut done: Vec<(usize, Vec<R>)>) -> Vec<R> {
    done.sort_unstable_by_key(|(index, _)| *index);
    done.into_iter().flat_map(|(_, results)| results).collect()
}

/// Why a parallel island step was refused. Nothing was stepped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParError {
    /// Thread count zero or past [`MAX_THREADS`]; the culprit is carried.
    BadThreads(usize),
    /// Chunk size zero.
    BadChunk,
    /// The island builder returned a unit twice or not at all: a bug in the
    /// builder, reported instead of stepping a unit twice.
    Partition,
}

impl core::fmt::Display for ParError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::BadThreads(n) => write!(f, "island threads {n} is outside 1..={MAX_THREADS}"),
            Self::BadChunk => write!(f, "island chunk size must be at least 1"),
            Self::Partition => write!(f, "island partition did not cover each unit once"),
        }
    }
}

impl std::error::Error for ParError {}

/// One island's members in ascending unit order, as exclusive borrows.
type Members<'a, T> = Vec<&'a mut T>;
/// A chunk index and the islands in it.
type Chunk<'a, T> = (usize, Vec<Members<'a, T>>);

/// Step every island of one chunk, in island order.
fn run_chunks<T, R, F>(chunks: Vec<Chunk<'_, T>>, step: &F) -> Vec<(usize, Vec<R>)>
where
    F: Fn(&mut [&mut T]) -> R,
{
    chunks
        .into_iter()
        .map(|(index, mut islands)| {
            let results = islands
                .iter_mut()
                .map(|members| step(members.as_mut_slice()))
                .collect();
            (index, results)
        })
        .collect()
}

/// Step all `units` once, island by island, in up to `cfg.threads` lanes
/// run on `pool`, a pool started once by the caller.
///
/// `coupled` lists unit pairs that interact during the step (any order,
/// repeats and out-of-range ends are ignored, as in [`build_islands`]).
/// `step` receives one island at a time: its members in ascending unit
/// order. It must touch only those members and read no shared mutable state.
/// Returns one result per island, in island order (ascending first member),
/// whatever the lane count, the pool size or which job finished first.
///
/// Determinism: islands come from [`build_islands`]; chunks of `cfg.chunk`
/// islands are fixed by the island count alone; lane `w` takes chunks
/// `w, w + lanes, ...`; results merge sorted by chunk index. A panic in
/// `step` unwinds on the caller, as the serial call would.
///
/// Errors: [`ParError`] for a bad config; units are untouched then.
pub fn step_islands<T, R, F>(
    pool: &JobPool,
    units: &mut [T],
    coupled: &[(usize, usize)],
    cfg: ParConfig,
    step: F,
) -> Result<Vec<R>, ParError>
where
    T: Send,
    R: Send,
    F: Fn(&mut [&mut T]) -> R + Sync,
{
    cfg.validate()?;
    let lanes = partition(units, coupled, cfg)?;
    Ok(merge_sorted(run_lanes(pool, lanes, &step)))
}

/// Cut `units` into islands, islands into chunks of `cfg.chunk`, and chunks
/// into one lane per worker (chunk `i` goes to lane `i % workers`). This is
/// the per-call setup: a handful of vectors sized by the unit count.
fn partition<'a, T>(
    units: &'a mut [T],
    coupled: &[(usize, usize)],
    cfg: ParConfig,
) -> Result<Vec<Vec<Chunk<'a, T>>>, ParError> {
    let islands = build_islands(units.len(), coupled);
    let (awake_chunks, _sleeping) = split_sleeping(&islands, None, cfg.chunk);
    let mut free: Vec<Option<&mut T>> = units.iter_mut().map(Some).collect();
    let mut chunks: Vec<Chunk<'a, T>> = Vec::with_capacity(awake_chunks.len());
    for (index, group) in awake_chunks.iter().enumerate() {
        let mut taken: Vec<Members<'a, T>> = Vec::with_capacity(group.len());
        for &island in group {
            let members = islands.get(island).ok_or(ParError::Partition)?;
            let mut borrowed = Vec::with_capacity(members.len());
            for &unit in members {
                let slot = free.get_mut(unit).and_then(Option::take);
                borrowed.push(slot.ok_or(ParError::Partition)?);
            }
            taken.push(borrowed);
        }
        chunks.push((index, taken));
    }
    let workers = cfg.threads.min(chunks.len()).max(1);
    let mut lanes: Vec<Vec<Chunk<'a, T>>> = (0..workers).map(|_| Vec::new()).collect();
    for chunk in chunks {
        if let Some(lane) = lanes.get_mut(chunk.0 % workers) {
            lane.push(chunk);
        }
    }
    Ok(lanes)
}

/// Queue every lane but the first as a job on `pool`, run lane 0 on the
/// caller, and wait for the scope. Each lane writes its own slot, so the
/// chunk results come back in lane order; [`merge_sorted`] sorts them by
/// chunk index. A panicking lane re-raises here after every job is done.
fn run_lanes<T, R, F>(
    pool: &JobPool,
    lanes: Vec<Vec<Chunk<'_, T>>>,
    step: &F,
) -> Vec<(usize, Vec<R>)>
where
    T: Send,
    R: Send,
    F: Fn(&mut [&mut T]) -> R + Sync,
{
    let mut slots: Vec<Vec<(usize, Vec<R>)>> = Vec::new();
    slots.resize_with(lanes.len(), Vec::new);
    pool.scope(|scope| {
        let mut pairs = lanes.into_iter().zip(slots.iter_mut());
        let mine = pairs.next();
        for (lane, slot) in pairs {
            scope.spawn(move || *slot = run_chunks(lane, step));
        }
        if let Some((lane, slot)) = mine {
            *slot = run_chunks(lane, step);
        }
    });
    slots.into_iter().flatten().collect()
}

/// Floats per body state: position 3, rotation `(x, y, z, w)` 4, linear
/// velocity 3, angular velocity 3.
pub const STATE_FLOATS: usize = 13;
/// Wire version of island snapshot bytes.
pub const SNAPSHOT_VERSION: u32 = 1;
/// Header bytes: version, body count, island count (`u32` each).
pub const SNAPSHOT_HEADER_BYTES: usize = 12;
/// Bytes per island: its member count.
pub const SNAPSHOT_ISLAND_BYTES: usize = 4;
/// Bytes per body: its index plus [`STATE_FLOATS`] floats.
pub const SNAPSHOT_BODY_BYTES: usize = 4 + 4 * STATE_FLOATS;
/// Worst-case bytes per fighter: a body alone in its own island. The net
/// encoder budgets a field of `n` fighters as
/// `SNAPSHOT_HEADER_BYTES + n * FIGHTER_SNAPSHOT_BYTES` at most.
pub const FIGHTER_SNAPSHOT_BYTES: usize = SNAPSHOT_BODY_BYTES + SNAPSHOT_ISLAND_BYTES;

/// Exact byte length of an island snapshot of `bodies` bodies in `islands`
/// islands.
#[must_use]
pub const fn snapshot_budget(bodies: usize, islands: usize) -> usize {
    SNAPSHOT_HEADER_BYTES + islands * SNAPSHOT_ISLAND_BYTES + bodies * SNAPSHOT_BODY_BYTES
}

/// Why island snapshot bytes were refused. Nothing was applied.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SnapshotError {
    /// The bytes end early or carry trailing data.
    Size,
    /// The version is not [`SNAPSHOT_VERSION`]; the culprit is carried.
    Version(u32),
    /// The snapshot holds a different body count than the world.
    Bodies {
        /// Bodies in the snapshot.
        have: u32,
        /// Bodies in the world.
        want: usize,
    },
    /// The islands are not canonical: each member list ascending, islands by
    /// first member, every body exactly once, no empty island.
    Order,
    /// A state float is NaN or infinite; the body index is carried.
    NotFinite(u32),
}

impl core::fmt::Display for SnapshotError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Size => write!(f, "island snapshot bytes are truncated or oversized"),
            Self::Version(v) => write!(f, "island snapshot version {v} is not {SNAPSHOT_VERSION}"),
            Self::Bodies { have, want } => {
                write!(f, "island snapshot holds {have} bodies, world has {want}")
            }
            Self::Order => write!(f, "island snapshot is not in canonical island order"),
            Self::NotFinite(i) => write!(f, "island snapshot body {i} holds a non-finite float"),
        }
    }
}

impl std::error::Error for SnapshotError {}

fn put_u32(out: &mut Vec<u8>, value: usize) {
    out.extend_from_slice(&u32::try_from(value).unwrap_or(u32::MAX).to_le_bytes());
}

/// Canonical bytes of a body field, in [`build_islands`] order: header, then
/// per island its member count and each member's index and state. The same
/// states and the same pairs (in any order) give the same bytes on every PC;
/// the length is exactly [`snapshot_budget`].
#[must_use]
pub fn island_snapshot(states: &[[f32; STATE_FLOATS]], coupled: &[(usize, usize)]) -> Vec<u8> {
    let islands = build_islands(states.len(), coupled);
    let mut out = Vec::with_capacity(snapshot_budget(states.len(), islands.len()));
    out.extend_from_slice(&SNAPSHOT_VERSION.to_le_bytes());
    put_u32(&mut out, states.len());
    put_u32(&mut out, islands.len());
    for members in &islands {
        put_u32(&mut out, members.len());
        for &body in members {
            put_u32(&mut out, body);
            for word in states[body] {
                out.extend_from_slice(&word.to_bits().to_le_bytes());
            }
        }
    }
    out
}

/// Bounds-checked little-endian reader over the snapshot bytes.
struct Reader<'a> {
    rest: &'a [u8],
}

impl Reader<'_> {
    fn u32(&mut self) -> Result<u32, SnapshotError> {
        let (head, rest) = self
            .rest
            .split_first_chunk::<4>()
            .ok_or(SnapshotError::Size)?;
        self.rest = rest;
        Ok(u32::from_le_bytes(*head))
    }
}

/// Parse [`island_snapshot`] bytes for a world of `bodies` bodies into
/// states in body-index order. Everything is validated before anything is
/// returned, so a caller that applies the result applies all of it or
/// nothing.
///
/// Errors: [`SnapshotError`] for truncated or trailing bytes, a wrong
/// version or body count, non-canonical island order, a repeated or
/// out-of-range body, or a non-finite float.
pub fn parse_island_snapshot(
    bytes: &[u8],
    bodies: usize,
) -> Result<Vec<[f32; STATE_FLOATS]>, SnapshotError> {
    let mut reader = Reader { rest: bytes };
    let version = reader.u32()?;
    if version != SNAPSHOT_VERSION {
        return Err(SnapshotError::Version(version));
    }
    let have = reader.u32()?;
    if usize::try_from(have).ok() != Some(bodies) {
        return Err(SnapshotError::Bodies { have, want: bodies });
    }
    let island_count = reader.u32()?;
    let mut states = vec![[0.0f32; STATE_FLOATS]; bodies];
    let mut seen = vec![false; bodies];
    let mut previous_first: Option<usize> = None;
    for _ in 0..island_count {
        let members = reader.u32()?;
        if members == 0 {
            return Err(SnapshotError::Order);
        }
        let mut previous: Option<usize> = None;
        for _ in 0..members {
            let body = reader.u32()?;
            let at = usize::try_from(body).map_err(|_| SnapshotError::Order)?;
            let ascending = previous.is_none_or(|p| at > p);
            let first_ok = previous.is_some() || previous_first.is_none_or(|p| at > p);
            if at >= bodies || seen[at] || !ascending || !first_ok {
                return Err(SnapshotError::Order);
            }
            if previous.is_none() {
                previous_first = Some(at);
            }
            previous = Some(at);
            seen[at] = true;
            for word in &mut states[at] {
                *word = f32::from_bits(reader.u32()?);
                if !word.is_finite() {
                    return Err(SnapshotError::NotFinite(body));
                }
            }
        }
    }
    if !reader.rest.is_empty() {
        return Err(SnapshotError::Size);
    }
    if seen.iter().any(|&was| !was) {
        return Err(SnapshotError::Order);
    }
    Ok(states)
}

/// Whole-island sleep hysteresis.
/// An island sleeps only when EVERY member rests (`v2 <= sleep_v2` for
/// `sleep_steps` ticks); one fast member (`v2 >= wake_v2`) wakes all.
/// Mid-band resets timers but keeps state — no flapping.
#[derive(Clone, Copy, Debug)]
pub struct SleepConfig {
    pub sleep_v2: f32,
    pub wake_v2: f32,
    pub sleep_steps: u32,
}

impl Default for SleepConfig {
    fn default() -> Self {
        Self {
            sleep_v2: 0.0025,
            wake_v2: 0.01,
            sleep_steps: 60,
        }
    }
}

/// Fold per-body motion (`v2`: squared speed, len `n`) into per-body sleep
/// state through canonical `islands`. `timers`/`asleep` must have len `n`.
pub fn update_sleep(
    n: usize,
    islands: &[Vec<usize>],
    v2: &[f32],
    timers: &mut [u32],
    asleep: &mut [bool],
    cfg: SleepConfig,
) {
    if timers.len() < n || asleep.len() < n || v2.len() < n {
        return;
    }
    for members in islands {
        // Peak includes sleepers: a kick on zeroed velocity is the wake signal.
        // The fold is the scalar `>` loop unrolled four-wide (bit-equal, no heap).
        let peak = simd_math::island_peak(v2, members, n);
        if peak >= cfg.wake_v2 {
            for &i in members {
                if i < n {
                    asleep[i] = false;
                    timers[i] = 0;
                }
            }
        } else if peak <= cfg.sleep_v2 {
            for &i in members {
                if i < n && !asleep[i] {
                    timers[i] = timers[i].saturating_add(1);
                    if timers[i] >= cfg.sleep_steps {
                        asleep[i] = true;
                    }
                }
            }
        } else {
            for &i in members {
                if i < n {
                    timers[i] = 0;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    #[test]
    fn chain_plus_singletons() {
        let is = build_islands(5, &[(0, 1), (1, 2)]);
        assert_eq!(is, vec![vec![0, 1, 2], vec![3], vec![4]]);
    }

    #[test]
    fn input_order_invariant() {
        let a = build_islands(4, &[(0, 1), (2, 3), (1, 2)]);
        let b = build_islands(4, &[(1, 2), (2, 3), (0, 1)]);
        let c = build_islands(4, &[(3, 2), (2, 1), (1, 0), (0, 1), (1, 1), (9, 9)]);
        assert_eq!(a, vec![vec![0, 1, 2, 3]]);
        assert_eq!(a, b);
        assert_eq!(a, c, "reversed ends, dupes, self/oob ignored");
    }

    #[test]
    fn empty_and_determinism_x2() {
        assert_eq!(build_islands(0, &[]), Vec::<Vec<usize>>::new());
        assert_eq!(build_islands(3, &[]), vec![vec![0], vec![1], vec![2]]);
        let p = [(0usize, 1usize), (3, 4), (1, 2)];
        assert_eq!(build_islands(6, &p), build_islands(6, &p));
        assert_eq!(
            build_islands(6, &p),
            vec![vec![0, 1, 2], vec![3, 4], vec![5]]
        );
    }

    #[test]
    fn island_count_splits_far_groups() {
        let is = build_islands(6, &[(0, 1), (1, 2), (3, 4)]);
        assert_eq!(is.len(), 3, "two contact groups + one singleton: {is:?}");
        assert_eq!(is, vec![vec![0, 1, 2], vec![3, 4], vec![5]]);
        let one = build_islands(4, &[(0, 1), (2, 3), (1, 2)]);
        assert_eq!(one.len(), 1, "bridge pair merges all: {one:?}");
    }

    #[test]
    fn combined_hash_order_independent_and_stable() {
        let bh: Vec<u64> = [11, 22, 33, 44, 55, 66].to_vec();
        let p = [(0usize, 1usize), (3, 4), (1, 2)];
        let h1 = combined_hash(6, &p, &bh);
        let rev: Vec<(usize, usize)> = p.iter().rev().map(|&(a, b)| (b, a)).collect();
        assert_eq!(
            h1,
            combined_hash(6, &rev, &bh),
            "pair order must not move hash"
        );
        assert_eq!(h1, combined_hash(6, &p, &bh), "same input, same hash x2");
        let moved: Vec<u64> = [11, 22, 34, 44, 55, 66].to_vec();
        assert_ne!(
            h1,
            combined_hash(6, &p, &moved),
            "body change must move hash"
        );
        assert_ne!(
            h1,
            combined_hash(6, &[(0, 1), (1, 2), (2, 3), (3, 4)], &bh),
            "split vs merged must not alias"
        );
    }

    #[test]
    fn sleep_hysteresis_sleeps_rest_wakes_kick() {
        let cfg = SleepConfig::default();
        let islands = build_islands(4, &[(0, 1), (2, 3)]);
        let (mut timers, mut asleep) = (vec![0u32; 4], vec![false; 4]);
        for _ in 0..cfg.sleep_steps {
            update_sleep(4, &islands, &[0.0; 4], &mut timers, &mut asleep, cfg);
        }
        assert_eq!(asleep, vec![true; 4], "full rest sleeps every island");
        // Mid-band motion resets timers but never wakes.
        asleep = vec![false; 4];
        timers = vec![7u32; 4];
        update_sleep(4, &islands, &[0.005; 4], &mut timers, &mut asleep, cfg);
        assert_eq!(asleep, vec![false; 4]);
        assert_eq!(timers, vec![0u32; 4], "mid-band clears sleep timers");
        // One fast body wakes only its own island.
        asleep = vec![true; 4];
        timers = vec![0u32; 4];
        update_sleep(
            4,
            &islands,
            &[0.0, 4.0, 0.0, 0.0],
            &mut timers,
            &mut asleep,
            cfg,
        );
        assert_eq!(asleep, vec![false, false, true, true]);
    }

    #[test]
    fn sleeping_split_sleeps_10k_static_wakes_kick_hash_identical_x4() {
        const DYNAMICS: usize = 1_000;
        const STATICS: usize = 10_000;
        const N: usize = DYNAMICS + STATICS;
        let islands = build_islands(N, &[]);
        assert_eq!(islands.len(), N, "singletons keep index order");
        let cfg = SleepConfig::default();
        let (mut timers, mut asleep) = (vec![0u32; N], vec![false; N]);
        let mut v2 = vec![0.0f32; N];
        for v in v2.iter_mut().take(DYNAMICS) {
            *v = 1.0;
        }
        for _ in 0..cfg.sleep_steps {
            update_sleep(N, &islands, &v2, &mut timers, &mut asleep, cfg);
        }
        assert!(
            asleep.iter().take(DYNAMICS).all(|&s| !s),
            "dynamics stay awake"
        );
        assert!(
            asleep.iter().skip(DYNAMICS).all(|&s| s),
            "10k far statics must sleep"
        );
        let (awake_chunks, sleeping) = split_sleeping(&islands, Some(&asleep), 64);
        let awake_total: usize = awake_chunks.iter().map(Vec::len).sum();
        assert_eq!(awake_total, DYNAMICS, "awake keeps the 1k dynamics");
        assert_eq!(sleeping.len(), STATICS, "sleeping keeps the 10k statics");
        let mut awake_flat: Vec<usize> = awake_chunks.iter().flatten().copied().collect();
        awake_flat.sort_unstable();
        assert_eq!(awake_flat.len(), DYNAMICS);
        let mut sleeping_sorted = sleeping.clone();
        sleeping_sorted.sort_unstable();
        assert!(
            sleeping_sorted.windows(2).all(|w| w[0] < w[1]),
            "merge keeps index order"
        );
        let kicks = [DYNAMICS, DYNAMICS + 4_000, N - 1];
        for &k in &kicks {
            v2[k] = cfg.wake_v2 + 1.0;
        }
        update_sleep(N, &islands, &v2, &mut timers, &mut asleep, cfg);
        let (awake_after, sleeping_after) = split_sleeping(&islands, Some(&asleep), 64);
        let flat_after: Vec<usize> = awake_after.iter().flatten().copied().collect();
        for &k in &kicks {
            assert!(!asleep[k], "kick at {k} must wake");
            assert!(flat_after.contains(&k), "wake recall holds island {k}");
        }
        assert_eq!(sleeping_after.len(), STATICS - kicks.len());
        assert_eq!(split_sleeping(&[], None, 0), (Vec::new(), Vec::new()));
        let (awake_unknown, _) = split_sleeping(&islands[..3], Some(&[]), 2);
        assert_eq!(awake_unknown.iter().flatten().count(), 3);
        let mut reference: Option<Vec<Vec<usize>>> = None;
        for threads in [1, 2, 4, 8] {
            let pool = pool(threads);
            let got = step_islands(
                &pool,
                &mut cells(N),
                &[],
                ParConfig { threads, chunk: 64 },
                member_ids,
            )
            .expect("steps");
            assert_eq!(got.len(), N, "threads {threads} keep every island");
            match reference.as_ref() {
                None => reference = Some(got),
                Some(want) => assert_eq!(&got, want, "threads {threads} match"),
            }
        }
    }

    #[test]
    fn commit_sleeping_whole_cover_sleeps_in_place_with_epoch_bump() {
        let awake: Vec<usize> = (0..11).collect();
        let full = vec![awake.clone()];
        assert_eq!(
            commit_sleeping_chunks(&awake, &full, 7),
            SleepCommit {
                awake: None,
                sleeping: vec![awake.clone()],
                epoch: 8,
            }
        );
        // Shuffled members still cover the island whole.
        let mut mixed = awake.clone();
        mixed.reverse();
        assert_eq!(
            commit_sleeping_chunks(&mixed, &[mixed.clone()], 7).awake,
            None
        );
        // The epoch wraps, never panics.
        assert_eq!(commit_sleeping_chunks(&awake, &full, u64::MAX).epoch, 0);
    }

    #[test]
    fn commit_sleeping_partial_extracts_sorted_chunks_with_epoch_held() {
        let awake: Vec<usize> = (0..11).collect();
        let chunks = vec![vec![6, 7, 8], vec![0, 1, 2], vec![4, 3, 5]];
        let got = commit_sleeping_chunks(&awake, &chunks, 7);
        assert_eq!(
            got.sleeping,
            vec![vec![0, 1, 2], vec![3, 4, 5], vec![6, 7, 8]],
            "islands sort by first member whatever the chunk order"
        );
        assert_eq!(got.awake, Some(vec![9, 10]));
        assert_eq!(got.epoch, 7, "partial cover holds the epoch");
        // No chunk eligible: everything stays awake, epoch held.
        let idle = commit_sleeping_chunks(&awake, &[], 7);
        assert_eq!(idle.awake, Some(awake));
        assert!(idle.sleeping.is_empty());
        assert_eq!(idle.epoch, 7);
    }

    #[test]
    fn commit_sleeping_overlap_unknown_and_empty_stay_awake() {
        let awake: Vec<usize> = (0..6).collect();
        let chunks = vec![vec![1, 0], vec![1, 2], vec![], vec![9]];
        let got = commit_sleeping_chunks(&awake, &chunks, 3);
        assert_eq!(got.sleeping, vec![vec![0], vec![2]]);
        assert_eq!(got.awake, Some(vec![1, 3, 4, 5, 9]));
        assert_eq!(got.epoch, 3);
        for island in &got.sleeping {
            assert!(!island.contains(&9), "never sleep what was not listed");
        }
        // Unknowns alone cannot fake whole-cover with an empty awake set.
        let stray = commit_sleeping_chunks(&[], &[vec![9]], 3);
        assert_eq!(stray.awake, Some(vec![9]));
        assert!(stray.sleeping.is_empty());
    }

    #[test]
    fn commit_sleeping_shuffled_bytes_match_x2() {
        let awake: Vec<usize> = (0..11).collect();
        let chunks = vec![vec![0, 1, 2], vec![3, 4, 5], vec![6, 7, 8]];
        let want = commit_sleeping_chunks(&awake, &chunks, 7);
        let mut mixed_awake = awake.clone();
        mixed_awake.reverse();
        for _ in 0..2 {
            let mixed = vec![vec![8, 6, 7], vec![5, 3, 4], vec![2, 0, 1]];
            assert_eq!(
                commit_sleeping_chunks(&mixed_awake, &mixed, 7),
                want,
                "shuffled awake, chunk and member order commits the same bytes"
            );
        }
    }

    #[test]
    fn simd_peak_matches_scalar_peak_bit_exact() {
        // Scalar copy of the pre-SIMD peak fold in `update_sleep`.
        fn scalar_peak(v2: &[f32], members: &[usize], n: usize) -> f32 {
            let mut peak = 0.0f32;
            for &i in members {
                if i < n {
                    if let Some(&v) = v2.get(i) {
                        if v > peak {
                            peak = v;
                        }
                    }
                }
            }
            peak
        }
        let mut rng = engine_core::Rng::new(2040).fork_named("simd-peak");
        for round in 0..200 {
            let n = usize::try_from(rng.next_range(0, 64)).unwrap_or(0);
            let span = u64::try_from(n + 2).unwrap_or(2);
            let v2: Vec<f32> = (0..n)
                .map(|_| f32::from_bits(rng.next_u32() & 0x7f7f_ffff))
                .collect();
            let m = usize::try_from(rng.next_range(0, 12)).unwrap_or(0);
            let members: Vec<usize> = (0..m)
                .map(|_| usize::try_from(rng.next_range(0, span)).unwrap_or(0))
                .collect();
            assert_eq!(
                super::simd_math::island_peak(&v2, &members, n).to_bits(),
                scalar_peak(&v2, &members, n).to_bits(),
                "round {round}: n={n} members={members:?}"
            );
        }
        // Failure inputs never panic and never exceed the floor.
        assert_eq!(
            super::simd_math::island_peak(&[], &[], 0).to_bits(),
            0.0f32.to_bits()
        );
        assert_eq!(
            super::simd_math::island_peak(&[1.0], &[0, 5, usize::MAX], 1).to_bits(),
            1.0f32.to_bits()
        );
        assert_eq!(
            super::simd_math::island_peak(&[f32::NAN, 2.0], &[0, 1], 2).to_bits(),
            2.0f32.to_bits()
        );
    }

    #[test]
    fn update_sleep_wired_peak_keeps_scalar_hysteresis() {
        // From-scratch scalar copy of `update_sleep` (peak as a plain loop).
        fn scalar_sleep(
            n: usize,
            islands: &[Vec<usize>],
            v2: &[f32],
            timers: &mut [u32],
            asleep: &mut [bool],
            cfg: SleepConfig,
        ) {
            for members in islands {
                let mut peak = 0.0f32;
                for &i in members {
                    if i < n && v2[i] > peak {
                        peak = v2[i];
                    }
                }
                if peak >= cfg.wake_v2 {
                    for &i in members {
                        if i < n {
                            asleep[i] = false;
                            timers[i] = 0;
                        }
                    }
                } else if peak <= cfg.sleep_v2 {
                    for &i in members {
                        if i < n && !asleep[i] {
                            timers[i] = timers[i].saturating_add(1);
                            if timers[i] >= cfg.sleep_steps {
                                asleep[i] = true;
                            }
                        }
                    }
                } else {
                    for &i in members {
                        if i < n {
                            timers[i] = 0;
                        }
                    }
                }
            }
        }
        let cfg = SleepConfig::default();
        let mut rng = engine_core::Rng::new(99).fork_named("simd-sleep-wire");
        for round in 0..50 {
            let n = usize::try_from(rng.next_range(1, 24)).unwrap_or(1);
            let count =
                usize::try_from(rng.next_range(0, 2 * u64::try_from(n).unwrap_or(1))).unwrap_or(0);
            let pairs: Vec<(usize, usize)> = (0..count)
                .map(|_| {
                    (
                        usize::try_from(rng.next_range(0, u64::try_from(n).unwrap_or(1)))
                            .unwrap_or(0),
                        usize::try_from(rng.next_range(0, u64::try_from(n).unwrap_or(1)))
                            .unwrap_or(0),
                    )
                })
                .collect();
            let islands = build_islands(n, &pairs);
            let v2: Vec<f32> = (0..n)
                .map(|_| f32::from_bits(rng.next_u32() & 0x3f7f_ffff) % 0.02)
                .collect();
            let (mut t_live, mut s_live) = (vec![3u32; n], vec![false; n]);
            let (mut t_ref, mut s_ref) = (vec![3u32; n], vec![false; n]);
            for _ in 0..5 {
                update_sleep(n, &islands, &v2, &mut t_live, &mut s_live, cfg);
                scalar_sleep(n, &islands, &v2, &mut t_ref, &mut s_ref, cfg);
            }
            assert_eq!(s_live, s_ref, "round {round}: sleep flags match scalar");
            assert_eq!(t_live, t_ref, "round {round}: timers match scalar");
        }
    }

    #[test]
    fn hash_matches_shared_fnv_bytes() {
        use engine_core::hash::{FNV64_OFFSET, FNV64_PRIME, GOLDEN_GAMMA};
        fn legacy_island(body_hash: &[u64], members: &[usize]) -> u64 {
            let mut h: u64 = FNV64_OFFSET;
            for &i in members {
                let w = body_hash.get(i).copied().unwrap_or(0);
                for byte in w.to_le_bytes() {
                    h ^= u64::from(byte);
                    h = h.wrapping_mul(FNV64_PRIME);
                }
            }
            h ^= u64::try_from(members.len()).unwrap_or(u64::MAX);
            h = h.wrapping_mul(FNV64_PRIME);
            h
        }
        let bh: Vec<u64> = vec![11, 22, 33, 44, 55, 66];
        // Empty, out-of-range and 300-member islands pin the count tail.
        let big: Vec<usize> = (0..300).collect();
        let big_hash: Vec<u64> = (0..300).collect();
        for (hashes, members) in [
            (bh.clone(), vec![]),
            (bh.clone(), vec![0, 2, 4]),
            (bh.clone(), vec![99]),
            (big_hash.clone(), big.clone()),
        ] {
            assert_eq!(
                island_hash(&hashes, &members),
                legacy_island(&hashes, &members)
            );
        }
        let p = [(0usize, 1usize), (3, 4), (1, 2)];
        assert_eq!(combined_hash(6, &p, &bh), {
            let islands = build_islands(6, &p);
            let mut h: u64 = 0;
            for members in &islands {
                h ^= legacy_island(&bh, members).wrapping_mul(GOLDEN_GAMMA);
            }
            h ^= u64::try_from(islands.len())
                .unwrap_or(u64::MAX)
                .wrapping_mul(FNV64_PRIME);
            h
        });
    }

    #[test]
    fn count_conversion_matches_reachable_saturates_absurd() {
        use crate::islands_build::count_as_u64;
        for (n, want) in [
            (0usize, 0u64),
            (1, 1),
            (6, 6),
            (300, 300),
            (1_000_000, 1_000_000),
        ] {
            assert_eq!(count_as_u64(n), want);
        }
        assert_eq!(count_as_u64(usize::MAX), u64::MAX);
    }

    /// A unit with order-sensitive state: every fold depends on the member
    /// before it, so a wrong member order or a wrong island split shows.
    #[derive(Clone, Debug, PartialEq, Eq)]
    struct Cell {
        id: usize,
        state: u64,
        steps: u32,
    }

    fn cells(n: usize) -> Vec<Cell> {
        (0..n)
            .map(|id| Cell {
                id,
                state: engine_core::splitmix64(u64::try_from(id).unwrap_or(0)),
                steps: 0,
            })
            .collect()
    }

    /// Uneven busy work per member, so chunks finish in a different order
    /// from the one they merge in.
    fn step_island(members: &mut [&mut Cell]) -> u64 {
        let mut carry = u64::try_from(members.len()).unwrap_or(0);
        for member in members.iter_mut() {
            for _ in 0..(member.id % 7 + 1) * 40 {
                member.state = engine_core::splitmix64(member.state ^ carry.rotate_left(17));
                carry = carry.wrapping_add(member.state);
            }
            member.steps += 1;
            carry = engine_core::splitmix64(carry ^ member.state);
        }
        carry
    }

    /// The same step with no executor: islands one after another, each on
    /// copies of its members written back.
    fn serial_step(cells: &mut [Cell], coupled: &[(usize, usize)]) -> Vec<u64> {
        build_islands(cells.len(), coupled)
            .iter()
            .map(|members| {
                let mut group: Vec<Cell> = members.iter().map(|&i| cells[i].clone()).collect();
                let result = {
                    let mut refs: Vec<&mut Cell> = group.iter_mut().collect();
                    step_island(&mut refs)
                };
                for (&i, cell) in members.iter().zip(group) {
                    cells[i] = cell;
                }
                result
            })
            .collect()
    }

    /// Chains, a pair, a triple whose ends are written backwards, and many
    /// singletons.
    fn coupling() -> Vec<(usize, usize)> {
        vec![
            (1, 0),
            (2, 1),
            (3, 2),
            (10, 11),
            (21, 20),
            (22, 21),
            (40, 70),
            (70, 41),
            (99, 5),
            (5, 99),
        ]
    }

    /// A pool whose workers plus the caller make `threads` threads, started
    /// once and reused for every tick, as a match would.
    fn pool(threads: usize) -> JobPool {
        JobPool::new(threads.saturating_sub(1)).expect("pool starts")
    }

    #[test]
    fn par_step_is_the_serial_step_under_any_threads_and_chunking() {
        let pairs = coupling();
        let mut reference = cells(120);
        let mut want = Vec::new();
        for _ in 0..12 {
            want.push(serial_step(&mut reference, &pairs));
        }
        for threads in [1, 2, 4, 8] {
            let pool = pool(threads);
            for chunk in [1, 2, 3, 7, 64, 1000] {
                let cfg = ParConfig { threads, chunk };
                let mut live = cells(120);
                for (tick, expected) in want.iter().enumerate() {
                    let got =
                        step_islands(&pool, &mut live, &pairs, cfg, step_island).expect("steps");
                    assert_eq!(
                        &got, expected,
                        "threads {threads} chunk {chunk} tick {tick}"
                    );
                }
                assert_eq!(live, reference, "threads {threads} chunk {chunk}");
                assert!(
                    live.iter().all(|c| c.steps == 12),
                    "every unit stepped once per tick"
                );
            }
        }
    }

    fn member_ids(members: &mut [&mut Cell]) -> Vec<usize> {
        members.iter().map(|c| c.id).collect()
    }

    #[test]
    fn par_step_results_come_back_in_island_order_with_ascending_members() {
        let pairs = [(5, 2), (0, 9), (9, 4), (7, 7), (30, 1)];
        let mut live = cells(10);
        let islands = build_islands(10, &pairs);
        let firsts: Vec<usize> = islands.iter().map(|m| m[0]).collect();
        assert_eq!(firsts, vec![0, 1, 2, 3, 6, 7, 8], "{islands:?}");
        assert_eq!(islands[0], vec![0, 4, 9]);
        for (threads, chunk) in [(1, 1), (3, 2), (8, 1), (8, 100)] {
            let cfg = ParConfig { threads, chunk };
            let got =
                step_islands(&pool(threads), &mut live, &pairs, cfg, member_ids).expect("steps");
            assert_eq!(got, islands, "threads {threads} chunk {chunk}");
        }
    }

    /// Yields on odd islands so workers finish out of merge order.
    fn jittered(members: &mut [&mut Cell]) -> u64 {
        if members[0].id % 2 == 1 {
            thread::yield_now();
        }
        step_island(members)
    }

    #[test]
    fn par_step_completion_order_cannot_change_the_answer() {
        let pairs = coupling();
        let pool = pool(8);
        let mut first: Option<(Vec<Cell>, Vec<u64>)> = None;
        for round in 0..30 {
            let mut live = cells(120);
            let mut last = Vec::new();
            for _ in 0..4 {
                let cfg = ParConfig {
                    threads: 8,
                    chunk: 3,
                };
                last = step_islands(&pool, &mut live, &pairs, cfg, jittered).expect("steps");
            }
            match &first {
                None => first = Some((live, last)),
                Some((state, result)) => {
                    assert_eq!(&live, state, "round {round}");
                    assert_eq!(&last, result, "round {round}");
                }
            }
        }
    }

    #[test]
    fn par_step_refuses_a_bad_config_and_steps_nothing() {
        let pool = pool(4);
        let mut live = cells(8);
        let untouched = live.clone();
        for (cfg, want) in [
            (
                ParConfig {
                    threads: 0,
                    chunk: 1,
                },
                ParError::BadThreads(0),
            ),
            (
                ParConfig {
                    threads: MAX_THREADS + 1,
                    chunk: 1,
                },
                ParError::BadThreads(MAX_THREADS + 1),
            ),
            (
                ParConfig {
                    threads: 2,
                    chunk: 0,
                },
                ParError::BadChunk,
            ),
        ] {
            assert_eq!(
                step_islands(&pool, &mut live, &[], cfg, step_island),
                Err(want)
            );
            assert_eq!(live, untouched);
            assert!(!want.to_string().is_empty());
        }
        let top = ParConfig {
            threads: MAX_THREADS,
            chunk: 1,
        };
        assert_eq!(
            step_islands(&pool, &mut live, &[], top, step_island).map(|r| r.len()),
            Ok(8)
        );
        let mut none: Vec<Cell> = Vec::new();
        let ok = ParConfig {
            threads: 4,
            chunk: 4,
        };
        assert_eq!(
            step_islands(&pool, &mut none, &[], ok, step_island),
            Ok(Vec::new())
        );
    }

    fn boom(members: &mut [&mut Cell]) -> u8 {
        let id = members[0].id;
        assert!(id != 11, "island {id} blew up");
        0
    }

    #[test]
    fn par_step_a_panicking_island_unwinds_on_the_caller() {
        let mut live = cells(16);
        for threads in [1, 4] {
            let pool = pool(threads);
            let run = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                step_islands(&pool, &mut live, &[], ParConfig { threads, chunk: 2 }, boom)
            }));
            let payload = run.expect_err("the panic reaches the caller");
            let text = payload
                .downcast_ref::<String>()
                .cloned()
                .unwrap_or_default();
            assert!(
                text.contains("island 11 blew up"),
                "threads {threads}: {text:?}"
            );
            // The pool survives and the next step runs.
            let cfg = ParConfig { threads, chunk: 2 };
            assert_eq!(
                step_islands(&pool, &mut live, &[], cfg, member_ids).map(|r| r.len()),
                Ok(16)
            );
        }
    }

    #[test]
    fn par_step_spawns_no_thread_per_step() {
        // Thread ids are never reused in a process, so a thread spawned per
        // step would show up as a new id on every tick.
        let pairs = coupling();
        let pool = pool(4);
        let seen = std::sync::Mutex::new(Vec::new());
        let mut live = cells(120);
        let cfg = ParConfig {
            threads: 4,
            chunk: 2,
        };
        for _ in 0..60 {
            step_islands(&pool, &mut live, &pairs, cfg, |members| {
                let id = thread::current().id();
                if let Ok(mut ids) = seen.lock() {
                    if !ids.contains(&id) {
                        ids.push(id);
                    }
                }
                step_island(members)
            })
            .expect("steps");
        }
        let seen = seen.into_inner().unwrap_or_default();
        assert!(
            seen.len() <= 4,
            "60 steps ran on {} threads; the pool has 3 workers plus the caller",
            seen.len()
        );
        assert!(
            seen.contains(&thread::current().id()),
            "lane 0 is the caller"
        );
    }

    /// Finite states from raw bits: exponent of 0.5..1.0, random mantissa
    /// and sign, so signed values and tiny differences both occur.
    fn field_states(n: usize, seed: u64) -> Vec<[f32; STATE_FLOATS]> {
        let mut draw = engine_core::CounterCursor::from_rng(engine_core::CounterRng::new(seed), 0);
        (0..n)
            .map(|_| {
                core::array::from_fn(|_| {
                    f32::from_bits(0x3f00_0000 | (draw.next_u32() & 0x807f_ffff))
                })
            })
            .collect()
    }

    /// Clinching fighters of a 64-fighter field, ends in both directions,
    /// a repeat, a self pair and an out-of-range end.
    fn field_pairs() -> Vec<(usize, usize)> {
        vec![
            (0, 1),
            (3, 2),
            (3, 4),
            (10, 20),
            (33, 32),
            (63, 50),
            (1, 0),
            (7, 7),
            (99, 1),
        ]
    }

    /// Bytes for arbitrary (possibly non-canonical) island lists, the shape
    /// a hostile peer could send.
    fn raw_snapshot(version: u32, bodies: usize, islands: &[Vec<usize>], word: f32) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&version.to_le_bytes());
        put_u32(&mut out, bodies);
        put_u32(&mut out, islands.len());
        for members in islands {
            put_u32(&mut out, members.len());
            for &body in members {
                put_u32(&mut out, body);
                for _ in 0..STATE_FLOATS {
                    out.extend_from_slice(&word.to_bits().to_le_bytes());
                }
            }
        }
        out
    }

    fn bits(states: &[[f32; STATE_FLOATS]]) -> Vec<u32> {
        states.iter().flatten().map(|w| w.to_bits()).collect()
    }

    #[test]
    fn snapshot_bytes_follow_build_islands_order_within_the_posted_budget() {
        let states = field_states(64, 1);
        let pairs = field_pairs();
        let bytes = island_snapshot(&states, &pairs);
        let islands = build_islands(64, &pairs);
        assert_eq!(
            islands.len(),
            64 - 6,
            "six links join 11 fighters into 5 islands"
        );
        assert_eq!(bytes.len(), snapshot_budget(64, islands.len()));
        assert!(bytes.len() <= SNAPSHOT_HEADER_BYTES + 64 * FIGHTER_SNAPSHOT_BYTES);
        // Header, then each island and member in build_islands order.
        let word = |at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().expect("4 bytes"));
        assert_eq!((word(0), word(4), word(8)), (SNAPSHOT_VERSION, 64, 58));
        let mut at = SNAPSHOT_HEADER_BYTES;
        for members in &islands {
            assert_eq!(usize::try_from(word(at)).ok(), Some(members.len()));
            at += SNAPSHOT_ISLAND_BYTES;
            for &body in members {
                assert_eq!(usize::try_from(word(at)).ok(), Some(body));
                at += SNAPSHOT_BODY_BYTES;
            }
        }
        assert_eq!(at, bytes.len());
        // The two extremes bound every field of 64: all alone, all in one.
        let alone = island_snapshot(&states, &[]);
        assert_eq!(
            alone.len(),
            SNAPSHOT_HEADER_BYTES + 64 * FIGHTER_SNAPSHOT_BYTES
        );
        let chain: Vec<(usize, usize)> = (0..63).map(|i| (i, i + 1)).collect();
        let one = island_snapshot(&states, &chain);
        assert_eq!(one.len(), snapshot_budget(64, 1));
        assert!(one.len() < alone.len());
    }

    #[test]
    fn snapshot_restore_then_snapshot_is_byte_equal_x2() {
        let states = field_states(64, 2);
        let pairs = field_pairs();
        let bytes = island_snapshot(&states, &pairs);
        for round in 0..2 {
            let back = parse_island_snapshot(&bytes, 64).expect("parses");
            assert_eq!(bits(&back), bits(&states), "round {round}");
            assert_eq!(island_snapshot(&back, &pairs), bytes, "round {round}");
        }
        // Pair order and direction never move the bytes; a state change does.
        let reversed: Vec<(usize, usize)> = pairs.iter().rev().map(|&(a, b)| (b, a)).collect();
        assert_eq!(island_snapshot(&states, &reversed), bytes);
        let mut moved = states.clone();
        moved[40][5] = f32::from_bits(moved[40][5].to_bits() ^ 1);
        assert_ne!(island_snapshot(&moved, &pairs), bytes);
        assert_ne!(
            island_snapshot(&states, &[(0, 1)]),
            bytes,
            "a split is not a merge"
        );
    }

    #[test]
    fn snapshot_every_truncation_and_trailing_byte_is_refused() {
        let bytes = island_snapshot(&field_states(64, 3), &field_pairs());
        for cut in 0..bytes.len() {
            assert_eq!(
                parse_island_snapshot(&bytes[..cut], 64),
                Err(SnapshotError::Size),
                "prefix of {cut} bytes"
            );
        }
        let mut longer = bytes.clone();
        longer.push(0);
        assert_eq!(parse_island_snapshot(&longer, 64), Err(SnapshotError::Size));
        assert!(parse_island_snapshot(&bytes, 64).is_ok());
    }

    #[test]
    fn snapshot_hostile_bytes_are_refused_by_name() {
        let ok = |islands: &[Vec<usize>], n: usize| {
            parse_island_snapshot(&raw_snapshot(SNAPSHOT_VERSION, n, islands, 0.5), n)
        };
        assert!(ok(&[vec![0], vec![1, 2]], 3).is_ok());
        assert_eq!(ok(&[vec![0, 1], vec![2]], 3).map(|s| s.len()), Ok(3));
        // Not canonical: islands out of order, members descending, a body
        // twice, out of range or missing, an empty island.
        for (what, islands, n) in [
            ("islands out of order", vec![vec![1], vec![0]], 2),
            ("members descending", vec![vec![1, 0]], 2),
            ("body twice", vec![vec![0], vec![0]], 2),
            ("body twice in one island", vec![vec![0, 0]], 1),
            ("body out of range", vec![vec![0, 5]], 2),
            ("body missing", vec![vec![0]], 2),
            ("empty island", vec![vec![0, 1], vec![]], 2),
        ] {
            assert_eq!(ok(&islands, n), Err(SnapshotError::Order), "{what}");
        }
        // Wrong version, wrong body count, lying counts, non-finite floats.
        let good = raw_snapshot(SNAPSHOT_VERSION, 2, &[vec![0, 1]], 0.5);
        assert_eq!(
            parse_island_snapshot(&raw_snapshot(7, 2, &[vec![0, 1]], 0.5), 2),
            Err(SnapshotError::Version(7))
        );
        assert_eq!(
            parse_island_snapshot(&good, 3),
            Err(SnapshotError::Bodies { have: 2, want: 3 })
        );
        let mut many = good.clone();
        many[8..12].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(parse_island_snapshot(&many, 2), Err(SnapshotError::Size));
        let mut huge = good.clone();
        huge[12..16].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(parse_island_snapshot(&huge, 2), Err(SnapshotError::Size));
        for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let bytes = raw_snapshot(SNAPSHOT_VERSION, 2, &[vec![0], vec![1]], bad);
            assert_eq!(
                parse_island_snapshot(&bytes, 2),
                Err(SnapshotError::NotFinite(0))
            );
        }
        assert!(
            parse_island_snapshot(&[], 0).is_err(),
            "no header is not an empty world"
        );
        assert_eq!(
            parse_island_snapshot(&island_snapshot(&[], &[]), 0),
            Ok(Vec::new())
        );
        assert!(SnapshotError::Order.to_string().contains("canonical"));
    }

    /// Rapier fighters: one world per fighter, the engine's real island unit.
    #[cfg(feature = "rapier")]
    mod fighters {
        use super::*;
        use crate::ragdoll::{BoneShape, Ragdoll};
        use crate::rapier::FIXED_DT;
        use engine_anim::contract::{Joint, LocalPose, Skeleton};

        /// 17 bones: pelvis, spine, chest, neck, head, two arms of three,
        /// two legs of three.
        fn humanoid() -> (Skeleton, Vec<[f32; 3]>) {
            // (parent, local offset in the parent's frame)
            let rows: [(Option<usize>, [f32; 3]); 17] = [
                (None, [0.0, 1.0, 0.0]),
                (Some(0), [0.0, 0.2, 0.0]),
                (Some(1), [0.0, 0.25, 0.0]),
                (Some(2), [0.0, 0.25, 0.0]),
                (Some(3), [0.0, 0.12, 0.0]),
                (Some(2), [-0.2, 0.2, 0.0]),
                (Some(5), [0.0, -0.3, 0.0]),
                (Some(6), [0.0, -0.28, 0.0]),
                (Some(2), [0.2, 0.2, 0.0]),
                (Some(8), [0.0, -0.3, 0.0]),
                (Some(9), [0.0, -0.28, 0.0]),
                (Some(0), [-0.1, -0.05, 0.0]),
                (Some(11), [0.0, -0.45, 0.0]),
                (Some(12), [0.0, -0.42, 0.0]),
                (Some(0), [0.1, -0.05, 0.0]),
                (Some(14), [0.0, -0.45, 0.0]),
                (Some(15), [0.0, -0.42, 0.0]),
            ];
            let joints = rows
                .iter()
                .enumerate()
                .map(|(i, (parent, _))| Joint {
                    name: format!("b{i}"),
                    parent: *parent,
                })
                .collect();
            (Skeleton { joints }, rows.iter().map(|(_, o)| *o).collect())
        }

        /// 64 fighters on an 8 x 8 grid, 3 m apart, each in its own world
        /// with a floor, limp and falling.
        fn field() -> Vec<Ragdoll> {
            let (skeleton, offsets) = humanoid();
            let shapes = vec![
                BoneShape {
                    radius: 0.06,
                    half_height: 0.1
                };
                skeleton.joints.len()
            ];
            (0..64u32)
                .map(|k| {
                    let (col, row) = (k % 8, k / 8);
                    let mut positions = offsets.clone();
                    positions[0] = [
                        3.0 * f32::from(u8::try_from(col).unwrap_or(0)),
                        1.2 + 0.05 * f32::from(u8::try_from(k % 5).unwrap_or(0)),
                        3.0 * f32::from(u8::try_from(row).unwrap_or(0)),
                    ];
                    let pose = LocalPose {
                        positions,
                        rotations: vec![[0.0, 0.0, 0.0, 1.0]; skeleton.joints.len()],
                    };
                    let mut r = Ragdoll::spawn(&skeleton, &pose, &shapes).expect("spawns");
                    r.add_static_box([30.0, 0.5, 30.0], [10.0, -0.5, 10.0])
                        .expect("floor");
                    r.blend_to_ragdoll(1.0).expect("limp");
                    r
                })
                .collect()
        }

        /// Fighters 3-4, 10-12 and 40-41 clinch: the first of each island
        /// drags the second's pelvis along before both step.
        fn clinches() -> Vec<(usize, usize)> {
            vec![(3, 4), (11, 10), (12, 11), (40, 41)]
        }

        fn step_fighters(members: &mut [&mut Ragdoll]) -> Vec<u64> {
            let mut pull: Option<[f32; 3]> = None;
            for fighter in members.iter_mut() {
                if let Some(v) = pull {
                    fighter.set_bone_linvel(0, v).expect("clinch pull");
                }
                pull = fighter.bone_linvel(0);
            }
            members
                .iter_mut()
                .map(|r| {
                    r.step(FIXED_DT).expect("fixed step");
                    r.state_hash()
                })
                .collect()
        }

        fn field_hash(field: &[Ragdoll], coupled: &[(usize, usize)]) -> u64 {
            let hashes: Vec<u64> = field.iter().map(Ragdoll::state_hash).collect();
            combined_hash(field.len(), coupled, &hashes)
        }

        fn run(threads: usize, chunk: usize, ticks: usize) -> (u64, Vec<Vec<Vec<u64>>>) {
            let mut f = field();
            let pairs = clinches();
            let pool = pool(threads);
            let mut per_tick = Vec::new();
            for _ in 0..ticks {
                let cfg = ParConfig { threads, chunk };
                per_tick
                    .push(step_islands(&pool, &mut f, &pairs, cfg, step_fighters).expect("steps"));
            }
            (field_hash(&f, &pairs), per_tick)
        }

        #[test]
        fn par_fighters_hash_the_same_under_one_two_four_and_eight_threads() {
            fn assert_send<T: Send>() {}
            assert_send::<Ragdoll>();
            let (base, base_ticks) = run(1, 8, 24);
            for threads in [2, 4, 8] {
                for chunk in [1, 8] {
                    let (hash, ticks) = run(threads, chunk, 24);
                    assert_eq!(hash, base, "threads {threads} chunk {chunk}");
                    assert_eq!(ticks, base_ticks, "threads {threads} chunk {chunk}");
                }
            }
            // The scene moved: fighters fell, and a clinch changed an island.
            let (still, _) = run(1, 8, 1);
            assert_ne!(still, base, "24 ticks differ from 1");
            let islands = build_islands(64, &clinches());
            assert_eq!(
                islands.len(),
                64 - 4,
                "four clinch links join 7 fighters into 3"
            );
        }

        #[test]
        fn par_fighters_match_stepping_each_island_by_hand() {
            let pairs = clinches();
            let mut by_hand = field();
            for _ in 0..24 {
                for members in build_islands(by_hand.len(), &pairs) {
                    for k in 1..members.len() {
                        let v = by_hand[members[k - 1]].bone_linvel(0).expect("bone 0");
                        by_hand[members[k]].set_bone_linvel(0, v).expect("pull");
                    }
                    for &i in &members {
                        by_hand[i].step(FIXED_DT).expect("fixed step");
                    }
                }
            }
            let (hash, _) = run(8, 1, 24);
            assert_eq!(field_hash(&by_hand, &pairs), hash);
        }

        #[test]
        fn par_fighters_clinch_changes_the_outcome_so_islands_matter() {
            // Without the pairs the fighters step apart and 11 is not dragged.
            let with = run(4, 2, 24).0;
            let mut apart = field();
            let pool = pool(4);
            for _ in 0..24 {
                step_islands(
                    &pool,
                    &mut apart,
                    &[],
                    ParConfig {
                        threads: 4,
                        chunk: 2,
                    },
                    step_fighters,
                )
                .expect("steps");
            }
            assert_ne!(field_hash(&apart, &clinches()), with);
        }

        /// Best of three wall times for 60 ticks of the whole field, in
        /// microseconds.
        #[expect(
            clippy::disallowed_types,
            clippy::disallowed_methods,
            reason = "timing test reads the wall clock; it never enters the tick"
        )]
        fn best_micros(threads: usize) -> u128 {
            let mut best = u128::MAX;
            let pool = pool(threads);
            for _ in 0..3 {
                let mut f = field();
                let pairs = clinches();
                let cfg = ParConfig { threads, chunk: 1 };
                let start = std::time::Instant::now();
                for _ in 0..60 {
                    step_islands(&pool, &mut f, &pairs, cfg, step_fighters).expect("steps");
                }
                best = best.min(start.elapsed().as_micros());
            }
            best
        }

        #[test]
        #[ignore = "timing: wall clock, run alone under --release (docs/plan/physics.md PX6: at least 2x at 8 threads)"]
        fn par_fighters_scale_at_least_twice_on_eight_threads() {
            let (one, eight) = (best_micros(1), best_micros(8));
            eprintln!("islands 64 fighters x 60 ticks: 1 thread {one} us, 8 threads {eight} us");
            assert!(
                one >= eight * 2,
                "1 thread {one} us vs 8 threads {eight} us"
            );
        }
    }
}
