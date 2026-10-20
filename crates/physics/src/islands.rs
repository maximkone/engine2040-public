//! islands: deterministic island topology plus canonical snapshot bytes.
//!
//! Contact pairs fold into islands by union-find: the same pairs in any
//! order, with repeats, reversed ends, self pairs or out-of-range ends, give
//! the same islands. The snapshot writes a body field in island order, so
//! the same states and the same pairs give the same bytes on every PC. The
//! Rapier world reads these bytes for checkpoint and rollback
//! (`rapier::World::island_snapshot`).
//!
//! [`step_islands`] steps one unit per island, one island at a time, in up to
//! [`MAX_THREADS`] lanes on a caller-owned [`JobPool`]. Islands are cut into
//! chunks by island count and `chunk` alone, lanes take chunks round-robin,
//! and the chunk results merge by chunk index, so the lane count reaches
//! neither state nor result order: 1, 2, 4 and 8 threads give the same
//! answer. The step itself is the caller's frozen function; this module only
//! cuts, schedules and merges. Std-only.
#![forbid(unsafe_code)]

use engine_ecs::JobPool;

/// Union-find root per body. Skips self and out-of-range pairs, then unions
/// by smaller root: no rank noise, fully deterministic.
///
/// Each component root is its minimum index, so edge order and duplicates
/// cannot change the output and no sort or dedup pass is needed.
pub fn roots(n: usize, pairs: &[(usize, usize)]) -> Vec<usize> {
    let mut p = Vec::with_capacity(n);
    union_roots(&mut p, n, pairs);
    p
}

/// Fill `p` with the [`roots`] answer on a caller-owned buffer.
///
/// Same edges, same union order and the same path compression as [`roots`], so
/// the buffer holds the same roots; only the buffer is reused, which keeps a
/// per-tick rebuild allocation-free.
fn union_roots(p: &mut Vec<usize>, n: usize, pairs: &[(usize, usize)]) {
    p.clear();
    p.extend(0..n);
    for &(a, b) in pairs {
        if a == b || a >= n || b >= n {
            continue;
        }
        let mut ra = a;
        while p[ra] != ra {
            p[ra] = p[p[ra]];
            ra = p[ra];
        }
        let mut rb = b;
        while p[rb] != rb {
            p[rb] = p[p[rb]];
            rb = p[rb];
        }
        if ra != rb {
            if ra < rb {
                p[rb] = ra;
            } else {
                p[ra] = rb;
            }
        }
    }
    for i in 0..n {
        let mut x = i;
        while p[x] != x {
            p[x] = p[p[x]];
            x = p[x];
        }
        p[i] = x;
    }
}

/// Most lanes one [`step_islands`] call may fan out to.
pub const MAX_THREADS: usize = 64;

/// How [`step_islands`] cuts and schedules its work.
///
/// Chunk edges depend on the island count and `chunk` only, never on
/// `threads`, so every lane count visits the same chunks and merges them in
/// the same order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ParConfig {
    /// Lanes, `1..=MAX_THREADS`: lane 0 runs on the caller and every other
    /// lane is one pool job, so `1` steps everything on the caller. The
    /// pool's worker count bounds how many lanes really run at once.
    pub threads: usize,
    /// Islands per chunk, at least `1`.
    pub chunk: usize,
}

impl ParConfig {
    /// Range-checks the config; nothing is stepped on an error.
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

/// Chunked split beside [`step_islands`]: the island ranges a step visits,
/// `chunk` islands each. Entry `i` covers islands `start..end` in island
/// order. Units: island indices. Determinism: the island count and `chunk`
/// alone decide the edges, so 1/2/4/8 threads get the same ranges. Errors:
/// none. Budget: one `usize` pair per chunk.
#[must_use]
pub fn island_chunks(count: usize, chunk: usize) -> Vec<(usize, usize)> {
    let width = chunk.max(1);
    (0..count)
        .step_by(width)
        .map(|start| (start, (start + width).min(count)))
        .collect()
}

/// Sorted merge beside [`step_islands`]: chunk results by chunk index, island
/// order inside, so 1/2/4/8 threads give the same sequence. Units: chunk
/// indices plus results. Determinism: `sort_unstable` by index, no clock.
/// Errors: none. Caller: [`step_islands`].
#[must_use]
pub fn merge_sorted<R>(mut done: Vec<(usize, Vec<R>)>) -> Vec<R> {
    done.sort_unstable_by_key(|(index, _)| *index);
    done.into_iter().flat_map(|(_, results)| results).collect()
}

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

/// Step all `units` once, island by island, in up to `cfg.threads` lanes run
/// on `pool`, a pool the caller starts once and reuses.
///
/// `coupled` lists unit pairs that interact during the step (any order,
/// repeats and out-of-range ends are ignored, as in [`build_islands`]).
/// `step` receives one island at a time: its members in ascending unit order.
/// It must touch only those members and read no shared mutable state.
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
    let ranges = island_chunks(islands.len(), cfg.chunk);
    let mut free: Vec<Option<&mut T>> = units.iter_mut().map(Some).collect();
    let mut chunks: Vec<Chunk<'a, T>> = Vec::with_capacity(ranges.len());
    for (index, (begin, end)) in ranges.into_iter().enumerate() {
        let group = islands.get(begin..end).ok_or(ParError::Partition)?;
        let mut taken: Vec<Members<'a, T>> = Vec::with_capacity(group.len());
        for members in group {
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

/// Canonical islands: every body appears in exactly one island; isolated
/// bodies yield singleton islands. Members ascend inside an island and
/// islands ascend by first member, for any input order.
///
/// Reference owner: it allocates a root table, a sort and one vector per
/// island on every call. [`IslandsScratch::build`] gives the same islands on
/// reused buffers for a per-tick rebuild, and the island tests hold the two
/// side by side.
#[must_use]
pub fn build_islands(n: usize, pairs: &[(usize, usize)]) -> Vec<Vec<usize>> {
    let r = roots(n, pairs);
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by_key(|&i| (r[i], i));
    let mut out: Vec<Vec<usize>> = Vec::new();
    for i in order {
        match out.last_mut() {
            Some(last) if r[last[0]] == r[i] => last.push(i),
            _ => out.push(vec![i]),
        }
    }
    out.sort_by_key(|v| v[0]);
    out
}

/// Reusable island buffers beside [`build_islands`].
///
/// Same islands, same member order and the same snapshot bytes as
/// [`build_islands`], but the root table, the counting table, the flat member
/// order and one vector per island live for the life of the scratch, so a
/// per-tick rebuild allocates nothing after the first tick that saw that many
/// islands. Grouping is a stable counting pass keyed by root: roots are
/// component minimums, so ascending roots are ascending island first members
/// and the in-order fill keeps members ascending inside an island, the same
/// total order as sorting by `(root, index)`.
#[derive(Clone, Debug, Default)]
pub struct IslandsScratch {
    roots: Vec<usize>,
    /// Per-root island length before the fill, then its exclusive end cursor.
    counts: Vec<usize>,
    /// Members in canonical order, island after island.
    order: Vec<usize>,
    islands: Vec<Vec<usize>>,
}

impl IslandsScratch {
    /// Empty scratch; the first [`Self::build`] allocates, later ones reuse.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Canonical islands of `n` bodies on the reused buffers, identical to
    /// [`build_islands`] for the same `n` and `pairs`.
    ///
    /// The returned slice is valid until the next call on this scratch.
    pub fn build(&mut self, n: usize, pairs: &[(usize, usize)]) -> &[Vec<usize>] {
        let Self {
            roots,
            counts,
            order,
            islands,
        } = self;
        union_roots(roots, n, pairs);
        counts.clear();
        counts.resize(n, 0);
        for &root in roots.iter() {
            counts[root] += 1;
        }
        // Exclusive prefix sums: where each root's members start.
        let mut start = 0usize;
        for count in counts.iter_mut() {
            let len = *count;
            *count = start;
            start += len;
        }
        order.clear();
        order.resize(n, 0);
        // Bodies ascend, so each island's members land in ascending order and
        // each root's end cursor is its own exclusive end afterwards.
        for (body, &root) in roots.iter().enumerate() {
            order[counts[root]] = body;
            counts[root] += 1;
        }
        let mut used = 0usize;
        let mut begin = 0usize;
        for &end in counts.iter() {
            if end == begin {
                continue;
            }
            if used == islands.len() {
                islands.push(Vec::new());
            }
            islands[used].clear();
            islands[used].extend_from_slice(&order[begin..end]);
            used += 1;
            begin = end;
        }
        &islands[..used]
    }
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
/// Mid-band resets timers but keeps state, so nothing flaps.
#[derive(Clone, Copy, Debug)]
pub struct SleepConfig {
    /// Squared speed (m^2/s^2) at or below which a member counts as resting.
    pub sleep_v2: f32,
    /// Squared speed at or above which a member wakes its whole island.
    pub wake_v2: f32,
    /// Resting ticks in a row before an island falls asleep.
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
        let peak = members
            .iter()
            .filter(|&&i| i < n)
            .fold(0.0f32, |p, &i| p.max(v2[i]));
        for &i in members.iter().filter(|&&i| i < n) {
            if peak >= cfg.wake_v2 {
                asleep[i] = false;
                timers[i] = 0;
            } else if peak <= cfg.sleep_v2 {
                if !asleep[i] {
                    timers[i] = timers[i].saturating_add(1);
                    asleep[i] = timers[i] >= cfg.sleep_steps;
                }
            } else {
                timers[i] = 0;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    #[test]
    fn build_slice_topology_stable_under_pair_order() {
        let fwd = build_islands(4, &[(0, 1), (2, 3), (1, 2)]);
        assert_eq!(fwd, vec![vec![0, 1, 2, 3]]);
        let rev = build_islands(4, &[(2, 1), (3, 2), (1, 0), (0, 1), (1, 1), (9, 9)]);
        assert_eq!(fwd, rev, "reversed ends, dupes, self/oob ignored");
        assert_eq!(
            build_islands(5, &[(3, 4)]),
            vec![vec![0], vec![1], vec![2], vec![3, 4]]
        );
        assert!(build_islands(0, &[]).is_empty());
    }

    /// Reference: normalize ends, sort, dedup, then union by minimum.
    fn sorted_roots(n: usize, pairs: &[(usize, usize)]) -> Vec<usize> {
        let mut p: Vec<usize> = (0..n).collect();
        let mut norm: Vec<(usize, usize)> = pairs
            .iter()
            .filter(|&&(a, b)| a != b && a < n && b < n)
            .map(|&(a, b)| (a.min(b), a.max(b)))
            .collect();
        norm.sort_unstable();
        norm.dedup();
        for (a, b) in norm {
            let (mut ra, mut rb) = (a, b);
            while p[ra] != ra {
                p[ra] = p[p[ra]];
                ra = p[ra];
            }
            while p[rb] != rb {
                p[rb] = p[p[rb]];
                rb = p[rb];
            }
            if ra != rb {
                p[ra.max(rb)] = ra.min(rb);
            }
        }
        (0..n)
            .map(|i| {
                let mut x = i;
                while p[x] != x {
                    x = p[x];
                }
                x
            })
            .collect()
    }

    #[test]
    fn roots_are_component_minimums_whatever_the_edge_order() {
        assert_eq!(roots(4, &[(0, 1), (2, 3), (1, 2)]), vec![0, 0, 0, 0]);
        assert_eq!(roots(5, &[(0, 1), (1, 2)]), vec![0, 0, 0, 3, 4]);
        assert_eq!(roots(3, &[]), vec![0, 1, 2]);
        assert_eq!(roots(0, &[]), Vec::<usize>::new());
        assert_eq!(roots(2, &[(1, 1), (9, 9)]), vec![0, 1]);
        let a = roots(6, &[(0, 1), (3, 4), (1, 2)]);
        assert_eq!(a, roots(6, &[(1, 2), (3, 4), (0, 1)]));
        assert_eq!(a, roots(6, &[(4, 3), (2, 1), (1, 0)]));
        assert_eq!(a, vec![0, 0, 0, 3, 3, 5]);
    }

    #[test]
    fn roots_match_the_sorted_reference_on_seeded_pair_sets() {
        let mut rng = engine_core::Rng::new(2040).fork_named("island-roots");
        for round in 0..200 {
            let n = usize::try_from(rng.next_range(0, 40)).unwrap_or(0);
            let count = usize::try_from(rng.next_range(0, 80)).unwrap_or(0);
            // Ends may run past `n`, repeat and point at themselves.
            let span = u64::try_from(n + 3).unwrap_or(3);
            let pairs: Vec<(usize, usize)> = (0..count)
                .map(|_| {
                    let end = |rng: &mut engine_core::Rng| {
                        usize::try_from(rng.next_range(0, span)).unwrap_or(0)
                    };
                    (end(&mut rng), end(&mut rng))
                })
                .collect();
            let want = sorted_roots(n, &pairs);
            assert_eq!(
                roots(n, &pairs),
                want,
                "round {round}: n={n} pairs={pairs:?}"
            );
            let reversed: Vec<(usize, usize)> = pairs.iter().rev().map(|&(a, b)| (b, a)).collect();
            assert_eq!(roots(n, &reversed), want, "round {round}: reversed");
            assert!(
                want.iter().enumerate().all(|(i, &r)| r <= i),
                "roots are minimums"
            );
        }
        // The shared union-find writer behind `roots` must fill a reused
        // buffer with the same roots, whatever the buffer held before.
        let mut buffer = vec![usize::MAX; 9];
        for (n, pairs) in [
            (0, vec![]),
            (3, vec![]),
            (6, vec![(0, 1), (3, 4), (1, 2)]),
            (2, vec![(1, 1), (9, 9)]),
            (5, vec![(3, 4), (4, 3), (0, 1)]),
        ] {
            union_roots(&mut buffer, n, &pairs);
            assert_eq!(buffer, roots(n, &pairs), "n={n} pairs={pairs:?}");
        }
    }

    /// The reused-buffer build must give the shipped build's islands, member
    /// for member, on every shape: joins, reversed ends, repeats, self pairs,
    /// out-of-range ends, empty worlds, and a smaller world after a bigger one
    /// (so no stale member survives in a reused vector).
    #[test]
    fn scratch_build_matches_the_shipped_build_on_every_shape() {
        let mut scratch = IslandsScratch::new();
        let mut rng = engine_core::Rng::new(2040).fork_named("island-scratch");
        for round in 0..200 {
            let n = usize::try_from(rng.next_range(0, 40)).unwrap_or(0);
            let count = usize::try_from(rng.next_range(0, 80)).unwrap_or(0);
            let span = u64::try_from(n + 3).unwrap_or(3);
            let pairs: Vec<(usize, usize)> = (0..count)
                .map(|_| {
                    let end = |rng: &mut engine_core::Rng| {
                        usize::try_from(rng.next_range(0, span)).unwrap_or(0)
                    };
                    (end(&mut rng), end(&mut rng))
                })
                .collect();
            let want = build_islands(n, &pairs);
            assert_eq!(
                scratch.build(n, &pairs),
                want.as_slice(),
                "round {round}: n={n} pairs={pairs:?}"
            );
            let reversed: Vec<(usize, usize)> = pairs.iter().rev().map(|&(a, b)| (b, a)).collect();
            assert_eq!(
                scratch.build(n, &reversed),
                want.as_slice(),
                "round {round}: reversed"
            );
        }
        // Every body of the 64-fighter field, then a three-body world on the
        // same scratch, then the field again: reuse never moves a member.
        let field = build_islands(64, &field_pairs());
        assert_eq!(scratch.build(64, &field_pairs()), field.as_slice());
        assert_eq!(scratch.build(3, &[]), build_islands(3, &[]).as_slice());
        assert_eq!(
            scratch.build(64, &field_pairs()),
            field.as_slice(),
            "wide island come back after a narrow one"
        );
        assert_eq!(scratch.build(0, &[]), build_islands(0, &[]).as_slice());
        assert_eq!(
            scratch.build(64, &field_pairs()),
            field.as_slice(),
            "empty world left no island behind"
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

    /// A unit with order-sensitive state: every step depends on the member
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
    fn step_cell_island(members: &mut [&mut Cell]) -> u64 {
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
    fn serial_cell_step(cells: &mut [Cell], coupled: &[(usize, usize)]) -> Vec<u64> {
        build_islands(cells.len(), coupled)
            .iter()
            .map(|members| {
                let mut group: Vec<Cell> = members.iter().map(|&i| cells[i].clone()).collect();
                let result = {
                    let mut refs: Vec<&mut Cell> = group.iter_mut().collect();
                    step_cell_island(&mut refs)
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
    /// once and reused, as a match would.
    fn lane_pool(threads: usize) -> JobPool {
        JobPool::new(threads.saturating_sub(1)).expect("pool starts")
    }

    /// Chunk edges are a function of the island count and `chunk` only, and
    /// they cover every island once in ascending order.
    #[test]
    fn par_split_edges_depend_on_count_and_chunk_only() {
        assert_eq!(island_chunks(64, 5).len(), 13);
        assert_eq!(island_chunks(64, 5).first().copied(), Some((0, 5)));
        assert_eq!(island_chunks(64, 5).last().copied(), Some((60, 64)));
        assert!(island_chunks(0, 4).is_empty(), "no island, no chunk");
        assert_eq!(
            island_chunks(64, 0),
            island_chunks(64, 1),
            "chunk 0 reads as 1"
        );
        for chunk in [1, 2, 3, 7, 64, 1000] {
            let edges = island_chunks(64, chunk);
            let covered: Vec<usize> = edges.iter().flat_map(|&(a, b)| a..b).collect();
            assert_eq!(covered, (0..64).collect::<Vec<_>>(), "chunk {chunk}");
            // The split never sees a lane count: the same call, twice.
            assert_eq!(edges, island_chunks(64, chunk), "chunk {chunk}");
        }
        assert_eq!(
            merge_sorted(vec![(1, vec![3]), (0, vec![1, 2])]),
            vec![1, 2, 3]
        );
        assert!(merge_sorted(Vec::<(usize, Vec<u8>)>::new()).is_empty());
    }

    /// The parallel step is the serial step under any threads and chunking.
    #[test]
    fn par_step_matches_the_serial_step_under_1_2_4_8_threads() {
        let pairs = coupling();
        let mut reference = cells(120);
        let mut want = Vec::new();
        for _ in 0..12 {
            want.push(serial_cell_step(&mut reference, &pairs));
        }
        for threads in [1, 2, 4, 8] {
            let pool = lane_pool(threads);
            for chunk in [1, 2, 3, 7, 64, 1000] {
                let cfg = ParConfig { threads, chunk };
                let mut live = cells(120);
                for (tick, expected) in want.iter().enumerate() {
                    let got = step_islands(&pool, &mut live, &pairs, cfg, step_cell_island)
                        .expect("steps");
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
            let got = step_islands(&lane_pool(threads), &mut live, &pairs, cfg, member_ids)
                .expect("steps");
            assert_eq!(got, islands, "threads {threads} chunk {chunk}");
        }
    }

    /// Yields on odd islands so workers finish out of merge order.
    fn jittered(members: &mut [&mut Cell]) -> u64 {
        if members[0].id % 2 == 1 {
            thread::yield_now();
        }
        step_cell_island(members)
    }

    #[test]
    fn par_step_completion_order_cannot_change_the_answer() {
        let pairs = coupling();
        let pool = lane_pool(8);
        let mut first: Option<(Vec<Cell>, Vec<u64>)> = None;
        for round in 0..30 {
            let mut live = cells(120);
            let mut last = Vec::new();
            let cfg = ParConfig {
                threads: 8,
                chunk: 3,
            };
            for _ in 0..4 {
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
        let pool = lane_pool(4);
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
                step_islands(&pool, &mut live, &[], cfg, step_cell_island),
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
            step_islands(&pool, &mut live, &[], top, step_cell_island).map(|r| r.len()),
            Ok(8)
        );
        let mut none: Vec<Cell> = Vec::new();
        let ok = ParConfig {
            threads: 4,
            chunk: 4,
        };
        assert_eq!(
            step_islands(&pool, &mut none, &[], ok, step_cell_island),
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
            let pool = lane_pool(threads);
            let cfg = ParConfig { threads, chunk: 2 };
            let run = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                step_islands(&pool, &mut live, &[], cfg, boom)
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
        let pool = lane_pool(4);
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
                step_cell_island(members)
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

    /// The 64-fighter scene: one Rapier world per fighter, each a floor with a
    /// three-capsule rig (legs, torso, head) that settles on itself, so a step
    /// carries real contacts and the solver has real work.
    #[cfg(feature = "rapier")]
    fn fighter_worlds() -> Vec<crate::rapier::World> {
        (0..64u8)
            .map(|k| {
                let mut world = crate::rapier::World::new();
                world
                    .add_static_box([30.0, 0.5, 30.0], [0.0, -0.5, 0.0])
                    .expect("floor");
                let x = f32::from(k % 8) * 2.0 - 7.0;
                let z = f32::from(k / 8) * 2.0 - 7.0;
                let lift = 0.1 * f32::from(k % 3);
                world
                    .spawn_capsule([x, 0.9 + lift, z], 0.4, 0.9)
                    .expect("legs");
                world
                    .spawn_capsule([x, 1.8 + lift, z], 0.4, 0.6)
                    .expect("torso");
                world
                    .spawn_capsule([x, 2.6 + lift, z], 0.35, 0.3)
                    .expect("head");
                world
            })
            .collect()
    }

    /// One island of the scene: every fighter world steps its own fixed tick.
    #[cfg(feature = "rapier")]
    fn step_fighter(worlds: &mut [&mut crate::rapier::World]) {
        for world in worlds.iter_mut() {
            world
                .step(crate::rigid::FIXED_DT)
                .expect("a fixed step runs");
        }
    }

    /// One digest over the scene, in fighter index order.
    #[cfg(feature = "rapier")]
    fn scene_hash(worlds: &[crate::rapier::World]) -> u64 {
        let mut hash = engine_core::Fnv64::new();
        hash.write_len(worlds.len());
        for world in worlds {
            hash.write_u64(world.state_hash());
        }
        hash.finish()
    }

    /// The row's hash leg: the 64-fighter scene steps to the same digest at
    /// 1, 2, 4 and 8 threads, for every chunking.
    #[cfg(feature = "rapier")]
    #[test]
    fn par_64_fighter_scene_hash_is_identical_under_1_2_4_8_threads() {
        const TICKS: u32 = 120;
        let run = |threads: usize, chunk: usize| {
            let mut worlds = fighter_worlds();
            let pool = lane_pool(threads);
            let cfg = ParConfig { threads, chunk };
            for _ in 0..TICKS {
                step_islands(&pool, &mut worlds, &[], cfg, step_fighter).expect("steps");
            }
            scene_hash(&worlds)
        };
        let fresh = scene_hash(&fighter_worlds());
        let want = run(1, 8);
        assert_ne!(want, fresh, "the scene stepped");
        for threads in [2, 4, 8] {
            for chunk in [1, 2, 4, 8, 16] {
                assert_eq!(run(threads, chunk), want, "threads {threads} chunk {chunk}");
            }
        }
    }

    /// The row's wall leg: `cargo test -p engine-physics --features rapier
    /// --release --lib par_speedup_64_fighters -- --ignored --nocapture`.
    /// Best of nine runs per config; the world build sits outside the clock,
    /// so the bar measures the island step alone, and the digest keeps the
    /// work alive.
    #[cfg(feature = "rapier")]
    #[test]
    #[ignore = "timing report, run in release on the reference CPU"]
    #[expect(
        clippy::disallowed_methods,
        clippy::disallowed_types,
        reason = "wall clock in a test-only timing report, never in the tick"
    )]
    fn par_speedup_64_fighters() {
        use std::time::Instant;
        const TICKS: u32 = 60;
        println!(
            "islands: available_parallelism {:?}",
            thread::available_parallelism()
        );
        let timed = |threads: usize, chunk: usize| {
            let mut best = u64::MAX;
            let mut hash = 0;
            for _ in 0..9 {
                let mut worlds = fighter_worlds();
                let pool = lane_pool(threads);
                let cfg = ParConfig { threads, chunk };
                let start = Instant::now();
                for _ in 0..TICKS {
                    step_islands(&pool, &mut worlds, &[], cfg, step_fighter).expect("steps");
                }
                let ns = u64::try_from(start.elapsed().as_nanos()).unwrap_or(u64::MAX);
                best = best.min(ns);
                hash = scene_hash(&worlds);
            }
            (best, hash)
        };
        let (serial, want) = timed(1, 8);
        println!("islands: 1 thread {serial} ns for 64 fighters x {TICKS} ticks");
        let mut bar = 0.0f64;
        for chunk in [1, 2, 4, 8, 16] {
            let (par, hash) = timed(4, chunk);
            assert_eq!(hash, want, "the 4-lane run must agree with 1 thread");
            let ratio = serial as f64 / par.max(1) as f64;
            bar = bar.max(ratio);
            println!("islands: 4 threads chunk {chunk}: {par} ns, {ratio:.2}x of 1 thread");
        }
        assert!(
            bar >= 2.0,
            "the row bar: 64 fighters on 4 threads must be at least 2x 1 thread, got {bar:.2}x"
        );
    }
}
