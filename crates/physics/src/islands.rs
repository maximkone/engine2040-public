//! islands: deterministic island topology plus canonical snapshot bytes.
//!
//! Contact pairs fold into islands by union-find: the same pairs in any
//! order, with repeats, reversed ends, self pairs or out-of-range ends, give
//! the same islands. The snapshot writes a body field in island order, so
//! the same states and the same pairs give the same bytes on every PC. The
//! Rapier world reads these bytes for checkpoint and rollback
//! (`rapier::World::island_snapshot`). Std-only; no sim step lives here.
#![forbid(unsafe_code)]

/// Union-find root per body. Skips self and out-of-range pairs, then unions
/// by smaller root: no rank noise, fully deterministic.
///
/// Each component root is its minimum index, so edge order and duplicates
/// cannot change the output and no sort or dedup pass is needed.
pub fn roots(n: usize, pairs: &[(usize, usize)]) -> Vec<usize> {
    let mut p: Vec<usize> = (0..n).collect();
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
    p
}

/// Canonical islands: every body appears in exactly one island; isolated
/// bodies yield singleton islands. Members ascend inside an island and
/// islands ascend by first member, for any input order.
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
}
