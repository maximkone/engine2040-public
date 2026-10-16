//! rigid: fixed-dt semi-implicit Euler bodies + gravity + ground clamp. Same seed + steps = same hash.
#![forbid(unsafe_code)]

use engine_core::hash::{FNV64_OFFSET, FNV64_PRIME};

/// Gravity acceleration (+down as `-y`), m/s^2.
pub const GRAVITY: f32 = 9.8;
/// Fixed step, 60 Hz.
pub const FIXED_DT: f32 = 1.0 / 60.0;

/// Rigid body: position + velocity + half-extents. Ground at `y == hy`.
#[derive(Clone, Copy, Debug, Default)]
pub struct Body {
    /// Center x.
    pub x: f32,
    /// Center y.
    pub y: f32,
    /// Velocity x.
    pub vx: f32,
    /// Velocity y.
    pub vy: f32,
    /// Half-extent x.
    pub hx: f32,
    /// Half-extent y.
    pub hy: f32,
}

impl Body {
    /// Construct a body.
    pub fn new(x: f32, y: f32, vx: f32, vy: f32, hx: f32, hy: f32) -> Self {
        Self {
            x,
            y,
            vx,
            vy,
            hx,
            hy,
        }
    }
}

/// Advance one body by `dt` (semi-implicit Euler + ground clamp).
pub fn step_body(b: &mut Body, dt: f32) {
    if dt <= 0.0 || !dt.is_finite() {
        return;
    }
    let h = dt.min(0.25);
    step_with_h(b, h, GRAVITY * h);
}

/// Single owner of the per-body Euler plus ground clamp.
///
/// `h` is the clamped step and `gh` is `GRAVITY * h`; callers hoist both so
/// the 11k-body loop pays one `min` plus one multiply. Units: metres, m/s.
/// Determinism: same `+ - *` order as `step_body`, slot order, no allocation.
/// Ground-slide fast path keeps `y`/`vy` when `y == hy`, `vy == +0`, small
/// `hy` and `h >= FIXED_DT`, so only `x` moves (sequential scan keeps cache
/// misses low [geaen | 3.4.2.7 Avoiding Cache Misses]).
#[expect(clippy::inline_always, reason = "11k hot loop")]
#[inline(always)]
pub fn step_with_h(b: &mut Body, h: f32, gh: f32) {
    // Ground-slide: `y` already on the plane bit-exact, `vy` is `+0`,
    // `|hy|` below 32768 so `hy - GRAVITY*h*h` moves at least one ulp when
    // `h >= FIXED_DT`, so the clamp restores `y`/`vy` and only `x` moves.
    // Same `x += vx*h` bits as the full path, same `x = 0, vx = 0` fixup
    // when `x` goes non-finite (then `y`/`vy` already `hy`/`0`), so
    // bit-identical while the 1k sliding prefix skips two stores plus the
    // finite/ground branches. `vy == +0` first: airborne bodies fail after
    // one load; tiny `h` stays full so sub-ulp `y` keeps `vy == -gh`
    // ([geaen | 3.4.3.2 Branch Prediction]).
    if b.vy.to_bits() == 0 && h >= FIXED_DT {
        let y_bits = b.y.to_bits();
        let hy_bits = b.hy.to_bits();
        if y_bits == hy_bits && (hy_bits & 0x7FFF_FFFF) < 0x4700_0000 {
            b.x += b.vx * h;
            if (b.x.to_bits() & 0x7F80_0000) == 0x7F80_0000 {
                b.x = 0.0;
                b.vx = 0.0;
            }
            return;
        }
    }
    b.vy -= gh;
    b.x += b.vx * h;
    b.y += b.vy * h;
    // Single-branch finite check: same truth table as `!x || !y || !vy`,
    // one predictable not-taken branch for finite bodies
    // ([geaen | 3.4.3.2 Branch Prediction]). `vy` needs no separate load:
    // non-finite `vy` always makes `y` non-finite after `y += vy*h`
    // (`h > 0`), so `x`/`y` alone decide and the 1k dynamic prefix pays
    // two bit tests instead of three.
    let bad = u32::from((b.x.to_bits() & 0x7F80_0000) == 0x7F80_0000)
        | u32::from((b.y.to_bits() & 0x7F80_0000) == 0x7F80_0000);
    if bad != 0 {
        step_with_h_slow(b);
        return;
    }
    if b.y < b.hy {
        b.y = b.hy;
        if b.vy < 0.0 {
            b.vy = 0.0;
        }
    }
}

/// Cold non-finite fixup for [`step_with_h`], bit-identical to the scalar tail.
///
/// Hot finite bodies never call it, so the 11k loop keeps one finite branch
/// plus the ground clamp. Units and determinism match [`step_with_h`].
#[cold]
#[inline(never)]
fn step_with_h_slow(b: &mut Body) {
    if !b.x.is_finite() {
        b.x = 0.0;
        b.vx = 0.0;
    }
    if !b.y.is_finite() || !b.vy.is_finite() {
        b.y = b.hy;
        b.vy = 0.0;
        return;
    }
    if b.y < b.hy {
        b.y = b.hy;
        if b.vy < 0.0 {
            b.vy = 0.0;
        }
    }
}

/// Fast `step_with_h` for the [`integrate`] fast path.
///
/// Same body as [`step_with_h`] with the `h >= FIXED_DT` guard hoisted:
/// `integrate` calls this only when `h >= FIXED_DT`, so the ground-slide
/// test reduces to `vy == +0` with the same truth table and the same
/// `+ - *` order, slot order, and cold fixup. Saves one float compare plus
/// one predictable branch per stepped body in the 1k dynamic prefix
/// ([geaen | 3.4.3.2 Branch Prediction]). Tiny `h` never reaches it: the
/// `integrate` slow path keeps [`step_with_h`].
/// Units and determinism match [`step_with_h`]. Caller: [`integrate`].
#[expect(clippy::inline_always, reason = "11k hot loop")]
#[inline(always)]
fn step_with_h_fast(b: &mut Body, h: f32, gh: f32) {
    if b.vy.to_bits() == 0 {
        let y_bits = b.y.to_bits();
        let hy_bits = b.hy.to_bits();
        if y_bits == hy_bits && (hy_bits & 0x7FFF_FFFF) < 0x4700_0000 {
            b.x += b.vx * h;
            if (b.x.to_bits() & 0x7F80_0000) == 0x7F80_0000 {
                b.x = 0.0;
                b.vx = 0.0;
            }
            return;
        }
    }
    b.vy -= gh;
    b.x += b.vx * h;
    b.y += b.vy * h;
    let bad = u32::from((b.x.to_bits() & 0x7F80_0000) == 0x7F80_0000)
        | u32::from((b.y.to_bits() & 0x7F80_0000) == 0x7F80_0000);
    if bad != 0 {
        step_with_h_slow(b);
        return;
    }
    if b.y < b.hy {
        b.y = b.hy;
        if b.vy < 0.0 {
            b.vy = 0.0;
        }
    }
}

/// Settled-body predicate beside [`step_with_h`].
///
/// True only when one Euler step plus ground clamp keeps every bit: `+0`
/// velocity, `y` already on the plane bit-exact, finite `x`, `x` not
/// negative zero (so `x + 0.0` keeps bits), and `|hy|` below 32768 so
/// `hy - GRAVITY*h*h` moves at least one ulp when `h >= FIXED_DT`.
/// Single-branch integer fast path with the same truth table as float
/// compares: `y == hy` via `(y_bits ^ hy_bits)` ORed into the same `bad`
/// mask (zero iff bit-equal, same table as `u32::from(y_bits != hy_bits)`
/// with one fewer compare per settled body), so the 10k
/// settled slice pays one predictable not-taken branch per body instead of
/// two ([geaen | 3.4.3.2 Branch Prediction]). Falling bodies still fail on
/// the `y != hy` bit; both velocities `+0` via one OR of exact bits (so
/// `-0.0` fails and steps to `+0.0` via the clamp instead of skipping), `hy`
/// range via masked bits (implies `y` finite when equal, so no separate
/// check), `x` finite plus neg-zero via one load. Same slot order,
/// no allocation.
/// Units: metres, m/s. Determinism: bit compares, no allocation.
/// Caller: `integrate` 32-wide skip plus `mixed_space_step` 32-wide skip in
/// `crates/perf/src/main.rs` (harness, not edited here).
#[expect(clippy::inline_always, reason = "11k hot predicate")]
#[inline(always)]
pub fn is_settled(b: &Body) -> bool {
    // Fast fail for dynamic bodies (mixed 1k prefix carries non-zero
    // velocity): two loads plus one predictable branch, saving the y/hy/x
    // loads. Exact `+0` bits (no mask) so `-0.0` fails and steps to `+0.0`
    // like the scalar path; settled `+0` bodies fall through to the
    // branchless tail ([geaen | 3.4.3.2 Branch Prediction]).
    if (b.vx.to_bits() | b.vy.to_bits()) != 0 {
        return false;
    }
    let y_bits = b.y.to_bits();
    let hy_bits = b.hy.to_bits();
    let hm = hy_bits & 0x7FFF_FFFF;
    let x_bits = b.x.to_bits();
    let bad = (y_bits ^ hy_bits)
        | u32::from(hm >= 0x4700_0000)
        | u32::from(x_bits == 0x8000_0000)
        | u32::from((x_bits & 0x7F80_0000) == 0x7F80_0000);
    bad == 0
}

/// Chunk-settled predicate beside [`is_settled`].
///
/// Test-only 4-wide owner for the chunk predicate family: true only when all
/// four bodies keep every bit through one Euler step plus ground clamp.
/// Stepping a settled lane keeps bits, so running all four on a mixed group
/// stays bit-identical to the per-body skip.
/// Units: metres, m/s. Determinism: bit compares, no allocation.
#[cfg(test)]
#[expect(clippy::inline_always, reason = "11k hot predicate")]
#[inline(always)]
fn chunk_all_settled(a: &Body, b: &Body, c: &Body, d: &Body) -> bool {
    is_settled(a) && is_settled(b) && is_settled(c) && is_settled(d)
}

/// Eight-wide settled predicate beside [`is_settled`].
///
/// Test-only 8-wide owner (8 x `f32` fills one AVX2 lane): true only when all
/// eight keep every bit. Iterator short-circuits on the first awake body in
/// index order with no per-lane bounds checks and a sequential scan
/// ([geaen | 3.4.2.7 Avoiding Cache Misses]). Stepping a settled lane keeps
/// bits, so running all eight on a mixed group stays bit-identical
/// ([geaen | 3.4.3.2 Branch Prediction]).
/// Units: metres, m/s. Determinism: bit compares, no allocation.
#[cfg(test)]
#[expect(clippy::inline_always, reason = "11k hot predicate")]
#[inline(always)]
fn chunk_all_settled8(chunk: &[Body]) -> bool {
    chunk.iter().all(is_settled)
}

/// Slice-settled predicate beside [`is_settled`], owner for the narrow family.
///
/// Bit-identical to `slice.iter().all(is_settled)`: true only when every lane
/// keeps every bit through one Euler step plus ground clamp. Velocity
/// fast-fail short-circuits in index order with no per-lane bounds checks
/// (sequential scan keeps cache misses low [geaen | 3.4.2.7 Avoiding Cache
/// Misses]); settled lanes accumulate bit checks branchlessly into one mask
/// with a single skip branch per call (one predictable branch
/// [geaen | 3.4.3.2 Branch Prediction]). `y == hy` folds via
/// `(y_bits ^ hy_bits)` ORed into the mask (zero iff equal), same truth table
/// as `u32::from(y_bits != hy_bits)` with one fewer compare per settled body.
/// Same slot order, same float order, no allocation. Caller: [`integrate`]
/// remainder fast path plus [`chunk_all_settled32`] only.
/// Units: metres, m/s. Determinism: bit compares, no allocation.
#[expect(clippy::inline_always, reason = "11k hot predicate")]
#[inline(always)]
fn slice_all_settled(slice: &[Body]) -> bool {
    let mut acc: u32 = 0;
    for b in slice.iter() {
        if (b.vx.to_bits() | b.vy.to_bits()) != 0 {
            return false;
        }
        let y_bits = b.y.to_bits();
        let hy_bits = b.hy.to_bits();
        let hm = hy_bits & 0x7FFF_FFFF;
        let x_bits = b.x.to_bits();
        acc |= (y_bits ^ hy_bits)
            | u32::from(hm >= 0x4700_0000)
            | u32::from(x_bits == 0x8000_0000)
            | u32::from((x_bits & 0x7F80_0000) == 0x7F80_0000);
    }
    acc == 0
}

/// Thirty-two-wide settled predicate beside [`is_settled`].
///
/// Bit-identical to `chunk.iter().all(is_settled)`: true only when all
/// thirty-two keep every bit, so `integrate` pays one skip branch per
/// 32-wide group while the 10k settled slice keeps its skip. Delegates to
/// [`slice_all_settled`] (same mask, same order) so the 1k dynamic prefix
/// still fails after one `vx|vy` check with no `y`/`hy`/`x` loads and settled
/// lanes keep one predictable branch per group
/// ([geaen | 3.4.3.2 Branch Prediction]). Same slot order, same float order,
/// no allocation. The scalar tail in [`integrate`] stays the fallback for
/// remainders and tiny `h`. Caller: [`integrate`] fast path only.
/// Units: metres, m/s. Determinism: bit compares, no allocation.
#[expect(clippy::inline_always, reason = "11k hot predicate")]
#[inline(always)]
fn chunk_all_settled32(chunk: &[Body]) -> bool {
    debug_assert_eq!(chunk.len(), 32, "32-wide groups only");
    slice_all_settled(chunk)
}

/// Advance all bodies by `dt` in index order (deterministic).
///
/// Chunked thirty-two-wide path with a single-branch remainder tail beside
/// [`step_body`]: the same [`step_with_h_fast`] body (same ops as
/// [`step_with_h`] with `h >= FIXED_DT` hoisted), so outputs are bit-identical
/// while the loop pays one `min` plus one multiply total and the compiler
/// fuses each group into SIMD lanes (8 x `f32` fills one AVX2 lane, four lanes
/// per 32-group, sequential scan keeps cache misses low
/// [geaen | 3.4.2.7 Avoiding Cache Misses]). One [`chunk_all_settled32`]
/// narrowed check per group skips resting ground bodies when `h >= FIXED_DT`,
/// so the 10k settled slice of the 11k mixed fixture pays only the predicate
/// and the 1k dynamic prefix pays one velocity fast-fail (`vx|vy` two loads)
/// plus one predictable branch per 32 bodies instead of per 8
/// ([geaen | 3.4.3.2 Branch Prediction]). The remainder tail takes one
/// [`slice_all_settled`] branch for its settled lanes (stepping a settled lane
/// via the fast path keeps every bit, so the batch-fail fallback may step all
/// remainder lanes with no per-body skip); the stepping loop is an iterator
/// with no per-lane bounds checks in index order. The skip flag fans into two
/// loops so the hot path carries no per-group flag branch.
/// Units: metres, m/s, seconds. Determinism: index order, same float ops,
/// no allocation, no wall clock. Errors: bad `dt` steps nothing.
pub fn integrate(bodies: &mut [Body], dt: f32) {
    if dt <= 0.0 || !dt.is_finite() {
        return;
    }
    let h = dt.min(0.25);
    let gh = GRAVITY * h;
    if h >= FIXED_DT {
        let mut chunks = bodies.chunks_exact_mut(32);
        for chunk in &mut chunks {
            if chunk_all_settled32(chunk) {
                continue;
            }
            for b in chunk.iter_mut() {
                step_with_h_fast(b, h, gh);
            }
        }
        let rem = chunks.into_remainder();
        // Single skip branch for the settled tail (hot 24-lane remainder):
        // stepping a settled lane via the fast path keeps every bit, so the
        // batch-fail fallback steps all remainder lanes with no per-body
        // skip; the scalar loop stays the fallback for tiny `h`.
        if !slice_all_settled(rem) {
            for b in rem.iter_mut() {
                step_with_h_fast(b, h, gh);
            }
        }
        return;
    }
    for b in bodies.iter_mut() {
        step_with_h(b, h, gh);
    }
}

/// Snapshot bodies (copy out for rollback / hash).
pub fn snapshot(bodies: &[Body]) -> Vec<Body> {
    bodies.to_vec()
}

/// Restore a snapshot into `dst` (copies shared prefix).
pub fn restore(dst: &mut [Body], src: &[Body]) {
    let n = dst.len().min(src.len());
    dst[..n].copy_from_slice(&src[..n]);
}

/// Cached FNV-1a prime powers for zero-word folding beside [`state_hash`].
///
/// `n` zero bytes (`(h ^ 0) * P == h * P`) collapse to one multiply by `P^n`,
/// so a settled 8-byte zero word pays one multiply (little-endian, same bytes).
/// Same digest, no hash change. No allocation.
const FNV_P2: u64 = FNV64_PRIME.wrapping_mul(FNV64_PRIME);
/// `P^3` for the three zero bytes of `y == 0.5`.
const FNV_P3: u64 = FNV_P2.wrapping_mul(FNV64_PRIME);
/// `P^4` intermediate for `P^8`.
const FNV_P4: u64 = FNV_P2.wrapping_mul(FNV_P2);
/// `P^8` for the 8 zero bytes of `vx == vy == +0`.
const FNV_P8: u64 = FNV_P4.wrapping_mul(FNV_P4);
/// `0.5f32` bits: settled sprite `y` (`y == hy == 0.5`, little-endian
/// bytes `00 00 00 3F`), so three zeros fold to `P^3` plus the tail byte.
const HALF_BITS: u32 = 0x3F00_0000;
/// Tail byte of `0.5f32` little-endian.
const HALF_TAIL: u64 = 0x3F;

/// One `u32` little-endian word through FNV-1a beside [`state_hash`].
///
/// Same 4 bytes and order as `Fnv64::write_u32`, one multiply per byte.
/// Units: bits. Determinism: LE order, no allocation.
#[inline]
fn fold_u32(mut h: u64, word: u32) -> u64 {
    let w = u64::from(word);
    h = (h ^ (w & 0xFF)).wrapping_mul(FNV64_PRIME);
    h = (h ^ ((w >> 8) & 0xFF)).wrapping_mul(FNV64_PRIME);
    h = (h ^ ((w >> 16) & 0xFF)).wrapping_mul(FNV64_PRIME);
    h = (h ^ ((w >> 24) & 0xFF)).wrapping_mul(FNV64_PRIME);
    h
}

/// One `u64` little-endian word through FNV-1a beside [`state_hash`].
///
/// LE bytes are `lo` then `hi`, same order as two `write_u32`, so digests
/// stay bit-identical. A zero word folds to one multiply by `FNV_P8`, same
/// 8 zero bytes; a non-zero word folds as two `fold_u32` (8 multiplies).
/// Units: bits. Determinism: LE order, no allocation.
#[inline]
fn fold_u64_word(h: u64, lo: u32, hi: u32) -> u64 {
    let word = u64::from(lo) | (u64::from(hi) << 32);
    if word == 0 {
        h.wrapping_mul(FNV_P8)
    } else {
        fold_u32(fold_u32(h, lo), hi)
    }
}

/// Deterministic FNV-1a hash over body bits (bytes via the shared owner).
///
/// Same bytes and order as `write_u32` (little-endian per word), so digests
/// stay bit-identical while the 11k loop keeps one `u64` accumulator in a
/// register with no array or slice loop ([geaen | 3.4.2.7 Avoiding Cache Misses]).
/// Fold: each body is two `u64` words (`x|y` lo `x` hi `y`, `vx|vy` lo
/// `vx` hi `vy`); a zero word folds to one multiply by the cached power,
/// so the 10k settled suffix (`is_settled` implies `vx == vy == +0`,
/// checked directly to skip `hy` loads) pays 7 multiplies for `y == 0.5`
/// else 9, vs 16 dynamic, with one predictable branch per body
/// ([geaen | 3.4.3.2 Branch Prediction]). A settled-suffix digest cache
/// would need the harness split (`crates/perf/src/main.rs`) or shared
/// state, so it stays out of this one-file fold (no hash change).
/// Units: bits. Determinism: index order, no allocation.
/// Caller: `Space::state_hash` in `crates/physics/src/lib.rs`.
pub fn state_hash(bodies: &[Body]) -> u64 {
    let mut h = FNV64_OFFSET;
    for b in bodies {
        let xb = b.x.to_bits();
        let yb = b.y.to_bits();
        let vxb = b.vx.to_bits();
        let vyb = b.vy.to_bits();
        if (vxb | vyb) == 0 {
            h = fold_u32(h, xb);
            if yb == HALF_BITS {
                h = h.wrapping_mul(FNV_P3);
                h = (h ^ HALF_TAIL).wrapping_mul(FNV64_PRIME);
            } else {
                h = fold_u32(h, yb);
            }
            h = h.wrapping_mul(FNV_P8);
        } else {
            h = fold_u64_word(h, xb, yb);
            h = fold_u64_word(h, vxb, vyb);
        }
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cell_bodies(n: usize) -> Vec<Body> {
        let mut v = Vec::with_capacity(n);
        for i in 0..n {
            let k = i as f32;
            v.push(Body::new(
                (k * 0.37) % 20.0 - 10.0,
                2.0 + (k * 0.53) % 18.0,
                (k * 0.11) % 4.0 - 2.0,
                0.0,
                0.5,
                0.5,
            ));
        }
        v
    }

    #[test]
    fn hundred_bodies_six_hundred_steps_ground_holds() {
        let mut b = cell_bodies(100);
        for _ in 0..600 {
            integrate(&mut b, FIXED_DT);
        }
        for m in &b {
            assert!(m.y >= m.hy, "below ground: {m:?}");
            assert!(
                m.x.is_finite() && m.y.is_finite() && m.vx.is_finite() && m.vy.is_finite(),
                "NaN/inf: {m:?}"
            );
        }
        let h1 = state_hash(&b);
        let mut c = cell_bodies(100);
        for _ in 0..600 {
            integrate(&mut c, FIXED_DT);
        }
        assert_eq!(h1, state_hash(&c), "same seed = same hash");
    }

    #[test]
    fn snapshot_roundtrip_and_ground_clamp() {
        let mut b = vec![Body::new(0.0, 5.0, 1.0, 0.0, 0.5, 0.5)];
        step_body(&mut b[0], FIXED_DT);
        let snap = snapshot(&b);
        let mut c = vec![Body::new(9.0, 9.0, 9.0, 9.0, 0.5, 0.5)];
        restore(&mut c, &snap);
        assert_eq!((c[0].x, c[0].y), (snap[0].x, snap[0].y));
        assert_eq!(state_hash(&b), state_hash(&c));
        for _ in 0..600 {
            step_body(&mut b[0], FIXED_DT);
        }
        assert_eq!(b[0].y, b[0].hy);
        assert_eq!(b[0].vy, 0.0);
    }

    #[test]
    fn hash_matches_shared_fnv_bytes() {
        use engine_core::hash::{FNV64_OFFSET, FNV64_PRIME};
        fn legacy(bodies: &[Body]) -> u64 {
            let mut h: u64 = FNV64_OFFSET;
            for b in bodies {
                for w in [b.x.to_bits(), b.y.to_bits(), b.vx.to_bits(), b.vy.to_bits()] {
                    for byte in w.to_le_bytes() {
                        h ^= u64::from(byte);
                        h = h.wrapping_mul(FNV64_PRIME);
                    }
                }
            }
            h
        }
        let cases = [
            vec![],
            vec![Body::new(0.0, 0.5, 0.0, 0.0, 0.5, 0.5)],
            vec![
                Body::new(-0.0, 2.5, 1.25, -3.0, 0.5, 0.5),
                Body::new(f32::NAN, f32::INFINITY, 0.0, 0.0, 0.5, 0.5),
            ],
        ];
        for bodies in &cases {
            assert_eq!(state_hash(bodies), legacy(bodies));
        }
        let moved = vec![Body::new(0.001, 0.5, 0.0, 0.0, 0.5, 0.5)];
        assert_ne!(state_hash(&cases[1]), state_hash(&moved));
    }

    #[test]
    fn u64_words_match_le_bytes_and_suffix_needs_prefix() {
        use engine_core::hash::{FNV64_OFFSET, FNV64_PRIME};
        let settled = Body::new(1.25, 0.5, 0.0, 0.0, 0.5, 0.5);
        let xb = settled.x.to_bits();
        let yb = settled.y.to_bits();
        let h = FNV64_OFFSET;
        assert_eq!(
            fold_u64_word(h, xb, yb),
            fold_u32(fold_u32(h, xb), yb),
            "u64 x|y must match two u32 LE"
        );
        assert_eq!(
            fold_u64_word(h, 0, 0),
            h.wrapping_mul(FNV_P8),
            "zero u64 word is one multiply"
        );
        let packed = u64::from(xb) | (u64::from(yb) << 32);
        let single = (h ^ packed).wrapping_mul(FNV64_PRIME);
        assert_ne!(
            single,
            fold_u64_word(h, xb, yb),
            "one multiply per non-zero word would move the hash"
        );
        let prefix_a = Body::new(0.0, 5.0, 1.0, 0.0, 0.5, 0.5);
        let prefix_c = Body::new(2.0, 5.0, 1.0, 0.0, 0.5, 0.5);
        let whole_a = state_hash(&[prefix_a, settled]);
        let whole_c = state_hash(&[prefix_c, settled]);
        let suffix_alone = state_hash(&[settled]);
        assert_ne!(whole_a, whole_c, "same suffix needs its prefix hash");
        assert_ne!(
            whole_a, suffix_alone,
            "suffix alone is not the suffix segment"
        );
    }

    fn mixed_11k(seed: u64) -> Vec<Body> {
        use engine_core::Rng;
        let mut rng = Rng::new(seed);
        let mut v = Vec::with_capacity(11_000);
        for i in 0..11_000 {
            if i < 1_000 {
                let x = (rng.next_u64() % 10_000) as f32 / 100.0 - 50.0;
                let y = (rng.next_u64() % 2_000) as f32 / 100.0 + 0.5;
                v.push(Body::new(x, y, 1.0, 0.5, 0.5, 0.5));
            } else {
                let k = i as f32;
                let x = (k * 0.37) % 20.0 - 10.0;
                v.push(Body::new(x, 0.5, 0.0, 0.0, 0.5, 0.5));
            }
        }
        v
    }

    #[test]
    fn batch_11k_mixed_matches_scalar_bits() {
        for seed in [2_040_u64, 1, 7, 9_001] {
            let mut a = mixed_11k(seed);
            let mut b = a.clone();
            for body in a.iter_mut() {
                step_body(body, FIXED_DT);
            }
            integrate(&mut b, FIXED_DT);
            assert_eq!(a.len(), b.len(), "len seed {seed}");
            for (i, (x, y)) in a.iter().zip(b.iter()).enumerate() {
                assert_eq!(
                    (x.x.to_bits(), x.y.to_bits(), x.vx.to_bits(), x.vy.to_bits()),
                    (y.x.to_bits(), y.y.to_bits(), y.vx.to_bits(), y.vy.to_bits()),
                    "diverging body {i} seed {seed}: scalar {x:?} vs batch {y:?}"
                );
            }
            for (i, body) in b.iter().enumerate().skip(1_000) {
                assert_eq!(
                    (body.y.to_bits(), body.vx.to_bits(), body.vy.to_bits()),
                    (body.hy.to_bits(), 0.0f32.to_bits(), 0.0f32.to_bits()),
                    "resting static moved at {i} seed {seed}: {body:?}"
                );
            }
            assert_eq!(state_hash(&a), state_hash(&b), "hash seed {seed}");
        }
    }

    #[test]
    fn settled_skip_bit_identical_and_covers_rest() {
        let resting = Body::new(1.25, 0.5, 0.0, 0.0, 0.5, 0.5);
        assert!(is_settled(&resting), "resting ground body must skip");
        assert!(!is_settled(&Body::new(0.0, 5.0, 1.0, 0.0, 0.5, 0.5)));
        assert!(!is_settled(&Body::new(0.0, 0.5, 1.0, 0.0, 0.5, 0.5)));
        assert!(
            !is_settled(&Body::new(-0.0, 0.5, 0.0, 0.0, 0.5, 0.5)),
            "neg-zero x with +0 vx must not skip"
        );
        assert!(
            !is_settled(&Body::new(0.0, 100_000.0, 0.0, 0.0, 0.5, 100_000.0)),
            "large hy may not move one ulp"
        );
        assert!(!is_settled(&Body::new(
            f32::INFINITY,
            0.5,
            0.0,
            0.0,
            0.5,
            0.5
        )));
        let mixed = mixed_11k(2_040);
        let settled = mixed.iter().filter(|b| is_settled(b)).count();
        assert_eq!(settled, 10_000, "10k resting bodies must skip");
        for body in [
            resting,
            Body::new(-0.0, 0.5, 0.0, 0.0, 0.5, 0.5),
            Body::new(0.0, 100_000.0, 0.0, 0.0, 0.5, 100_000.0),
        ] {
            let mut a = [body];
            let mut b = [body];
            for x in a.iter_mut() {
                step_body(x, FIXED_DT);
            }
            integrate(&mut b, FIXED_DT);
            assert_eq!(
                (
                    a[0].x.to_bits(),
                    a[0].y.to_bits(),
                    a[0].vx.to_bits(),
                    a[0].vy.to_bits()
                ),
                (
                    b[0].x.to_bits(),
                    b[0].y.to_bits(),
                    b[0].vx.to_bits(),
                    b[0].vy.to_bits()
                ),
                "scalar vs skip diverge for {body:?}"
            );
        }
        let mut a = [Body::new(1.0, 0.5, 0.0, 0.0, 0.5, 0.5)];
        let mut b = a;
        for x in a.iter_mut() {
            step_body(x, 1e-7);
        }
        integrate(&mut b, 1e-7);
        assert_eq!(
            (
                a[0].x.to_bits(),
                a[0].y.to_bits(),
                a[0].vx.to_bits(),
                a[0].vy.to_bits()
            ),
            (
                b[0].x.to_bits(),
                b[0].y.to_bits(),
                b[0].vx.to_bits(),
                b[0].vy.to_bits()
            ),
            "tiny dt must match scalar without skipping"
        );
    }

    #[test]
    fn neg_zero_velocity_must_not_skip_and_matches_scalar() {
        // Failure: `-0.0` velocities must not skip. Scalar turns `-0.0`
        // into `+0.0` via the ground clamp, while a skip would keep `-0.0`
        // bits. Failure: any lane claims settled or diverges scalar vs skip.
        for (vx, vy) in [(-0.0, 0.0), (0.0, -0.0), (-0.0, -0.0)] {
            let body = Body::new(1.25, 0.5, vx, vy, 0.5, 0.5);
            assert!(
                !is_settled(&body),
                "neg-zero velocity vx {vx:?} vy {vy:?} must not skip: {body:?}"
            );
            let mut a = [body];
            let mut b = [body];
            for x in a.iter_mut() {
                step_body(x, FIXED_DT);
            }
            integrate(&mut b, FIXED_DT);
            assert_eq!(
                (
                    a[0].x.to_bits(),
                    a[0].y.to_bits(),
                    a[0].vx.to_bits(),
                    a[0].vy.to_bits()
                ),
                (
                    b[0].x.to_bits(),
                    b[0].y.to_bits(),
                    b[0].vx.to_bits(),
                    b[0].vy.to_bits()
                ),
                "neg-zero velocity vx {vx:?} vy {vy:?} diverged: scalar {a:?} vs skip {b:?}"
            );
            assert_eq!(
                b[0].vy.to_bits(),
                0.0f32.to_bits(),
                "neg-zero velocity must settle to +0: {b:?}"
            );
        }
    }

    #[test]
    fn chunk_predicate_skips_settled_runs_dynamic() {
        let settled = Body::new(1.25, 0.5, 0.0, 0.0, 0.5, 0.5);
        let awake = Body::new(0.0, 5.0, 1.0, 0.0, 0.5, 0.5);
        assert!(
            chunk_all_settled(&settled, &settled, &settled, &settled),
            "all-settled chunk must skip"
        );
        assert!(
            !chunk_all_settled(&settled, &settled, &awake, &settled),
            "one-awake chunk must run"
        );
        assert!(
            !chunk_all_settled(&awake, &awake, &awake, &awake),
            "dynamic chunk must run"
        );
        for chunk in [[settled; 4], [awake; 4], [settled, settled, awake, settled]] {
            let mut scalar = chunk;
            let mut batched = chunk;
            for body in scalar.iter_mut() {
                step_body(body, FIXED_DT);
            }
            integrate(&mut batched, FIXED_DT);
            for (i, (x, y)) in scalar.iter().zip(batched.iter()).enumerate() {
                assert_eq!(
                    (x.x.to_bits(), x.y.to_bits(), x.vx.to_bits(), x.vy.to_bits()),
                    (y.x.to_bits(), y.y.to_bits(), y.vx.to_bits(), y.vy.to_bits()),
                    "chunk diverge at lane {i}: scalar {x:?} vs batch {y:?}"
                );
            }
        }
    }

    #[test]
    fn chunk8_predicate_skips_settled_runs_dynamic() {
        let settled = Body::new(1.25, 0.5, 0.0, 0.0, 0.5, 0.5);
        let awake = Body::new(0.0, 5.0, 1.0, 0.0, 0.5, 0.5);
        assert!(chunk_all_settled8(&[settled; 8]), "all-settled 8 must skip");
        assert!(
            !chunk_all_settled8(&[
                settled, settled, awake, settled, settled, settled, settled, settled
            ]),
            "one-awake 8 must run"
        );
        assert!(
            !chunk_all_settled8(&[
                settled, settled, settled, settled, settled, settled, settled, awake
            ]),
            "tail-awake 8 must run"
        );
        assert!(!chunk_all_settled8(&[awake; 8]), "dynamic 8 must run");
        for chunk in [
            [settled; 8],
            [awake; 8],
            [
                settled, settled, awake, settled, settled, settled, settled, settled,
            ],
            [
                settled, settled, settled, settled, settled, settled, awake, settled,
            ],
        ] {
            let mut scalar = chunk;
            let mut batched = chunk;
            for body in scalar.iter_mut() {
                step_body(body, FIXED_DT);
            }
            integrate(&mut batched, FIXED_DT);
            for (i, (x, y)) in scalar.iter().zip(batched.iter()).enumerate() {
                assert_eq!(
                    (x.x.to_bits(), x.y.to_bits(), x.vx.to_bits(), x.vy.to_bits()),
                    (y.x.to_bits(), y.y.to_bits(), y.vx.to_bits(), y.vy.to_bits()),
                    "8-wide diverge at lane {i}: scalar {x:?} vs batch {y:?}"
                );
            }
        }
        // Failure: bad `dt` steps nothing through the 8-wide path.
        for bad in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            let mut scalar = [awake; 8];
            let mut batched = [awake; 8];
            for body in scalar.iter_mut() {
                step_body(body, bad);
            }
            integrate(&mut batched, bad);
            for (i, (x, y)) in scalar.iter().zip(batched.iter()).enumerate() {
                assert_eq!(
                    (x.x.to_bits(), x.y.to_bits(), x.vx.to_bits(), x.vy.to_bits()),
                    (y.x.to_bits(), y.y.to_bits(), y.vx.to_bits(), y.vy.to_bits()),
                    "bad dt {bad:?} diverged at lane {i}"
                );
            }
        }
    }

    #[test]
    fn chunk32_predicate_skips_settled_runs_dynamic() {
        let settled = Body::new(1.25, 0.5, 0.0, 0.0, 0.5, 0.5);
        let awake = Body::new(0.0, 5.0, 1.0, 0.0, 0.5, 0.5);
        assert!(
            chunk_all_settled32(&[settled; 32]),
            "all-settled 32 must skip"
        );
        for lane in [0, 7, 8, 15, 16, 23, 24, 31] {
            let mut chunk = [settled; 32];
            chunk[lane] = awake;
            assert!(!chunk_all_settled32(&chunk), "awake at {lane} must run");
        }
        assert!(!chunk_all_settled32(&[awake; 32]), "dynamic 32 must run");
        for chunk in [[settled; 32], [awake; 32]] {
            let mut scalar = chunk;
            let mut batched = chunk;
            for body in scalar.iter_mut() {
                step_body(body, FIXED_DT);
            }
            integrate(&mut batched, FIXED_DT);
            for (i, (x, y)) in scalar.iter().zip(batched.iter()).enumerate() {
                assert_eq!(
                    (x.x.to_bits(), x.y.to_bits(), x.vx.to_bits(), x.vy.to_bits()),
                    (y.x.to_bits(), y.y.to_bits(), y.vx.to_bits(), y.vy.to_bits()),
                    "32-wide diverge at lane {i}: scalar {x:?} vs batch {y:?}"
                );
            }
        }
        // Mixed 32 with one awake keeps settled lanes bit-identical.
        {
            let mut chunk = [settled; 32];
            chunk[17] = awake;
            let mut scalar = chunk;
            let mut batched = chunk;
            for body in scalar.iter_mut() {
                step_body(body, FIXED_DT);
            }
            integrate(&mut batched, FIXED_DT);
            for (i, (x, y)) in scalar.iter().zip(batched.iter()).enumerate() {
                assert_eq!(
                    (x.x.to_bits(), x.y.to_bits(), x.vx.to_bits(), x.vy.to_bits()),
                    (y.x.to_bits(), y.y.to_bits(), y.vx.to_bits(), y.vy.to_bits()),
                    "mixed 32 diverge at lane {i}"
                );
            }
        }
        // Failure: bad `dt` steps nothing through the 32-wide path.
        for bad in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            let mut scalar = [awake; 32];
            let mut batched = [awake; 32];
            for body in scalar.iter_mut() {
                step_body(body, bad);
            }
            integrate(&mut batched, bad);
            for (i, (x, y)) in scalar.iter().zip(batched.iter()).enumerate() {
                assert_eq!(
                    (x.x.to_bits(), x.y.to_bits(), x.vx.to_bits(), x.vy.to_bits()),
                    (y.x.to_bits(), y.y.to_bits(), y.vx.to_bits(), y.vy.to_bits()),
                    "bad dt {bad:?} diverged at lane {i}"
                );
            }
        }
        // Failure: 33 bodies cover the 32-wide group plus the one-body tail.
        {
            let mut scalar = [settled; 33];
            scalar[0] = awake;
            scalar[32] = awake;
            let mut batched = scalar;
            for body in scalar.iter_mut() {
                step_body(body, FIXED_DT);
            }
            integrate(&mut batched, FIXED_DT);
            for (i, (x, y)) in scalar.iter().zip(batched.iter()).enumerate() {
                assert_eq!(
                    (x.x.to_bits(), x.y.to_bits(), x.vx.to_bits(), x.vy.to_bits()),
                    (y.x.to_bits(), y.y.to_bits(), y.vx.to_bits(), y.vy.to_bits()),
                    "33-wide tail diverged at lane {i}"
                );
            }
        }
    }

    #[test]
    fn slice_remainder_matches_all_and_skips_tail() {
        // Failure: batch remainder predicate diverges from `all(is_settled)`
        // or stepping a settled lane via fast changes bits, so the
        // single-branch tail skip would move the hash.
        let settled = Body::new(1.25, 0.5, 0.0, 0.0, 0.5, 0.5);
        let awake = Body::new(0.0, 5.0, 1.0, 0.0, 0.5, 0.5);
        let empty: Vec<Body> = Vec::new();
        assert!(slice_all_settled(&empty));
        assert_eq!(slice_all_settled(&empty), empty.iter().all(is_settled));
        assert!(slice_all_settled(&[settled; 24]));
        assert!(!slice_all_settled(&[awake; 24]));
        for lane in [0, 7, 23] {
            let mut tail = [settled; 24];
            tail[lane] = awake;
            assert!(!slice_all_settled(&tail));
            assert_eq!(slice_all_settled(&tail), tail.iter().all(is_settled));
        }
        for edge in [
            Body::new(-0.0, 0.5, 0.0, 0.0, 0.5, 0.5),
            Body::new(f32::INFINITY, 0.5, 0.0, 0.0, 0.5, 0.5),
            Body::new(1.25, 0.5, -0.0, 0.0, 0.5, 0.5),
            Body::new(1.25, 0.5, 0.0, -0.0, 0.5, 0.5),
            Body::new(1.25, 5.0, 0.0, 0.0, 0.5, 0.5),
            Body::new(1.25, 0.5, 0.0, 0.0, 0.5, 100_000.0),
        ] {
            let mut tail = [settled; 24];
            tail[17 % 24] = edge;
            assert!(!slice_all_settled(&tail));
            assert_eq!(slice_all_settled(&tail), tail.iter().all(is_settled));
        }
        let h = FIXED_DT;
        let gh = GRAVITY * h;
        for base in [settled, Body::new(-3.5, 0.5, 0.0, 0.0, 0.5, 0.5)] {
            assert!(is_settled(&base), "fixture must be settled: {base:?}");
            let mut stepped = [base];
            for body in stepped.iter_mut() {
                step_with_h_fast(body, h, gh);
            }
            assert_eq!(
                (
                    stepped[0].x.to_bits(),
                    stepped[0].y.to_bits(),
                    stepped[0].vx.to_bits(),
                    stepped[0].vy.to_bits()
                ),
                (
                    base.x.to_bits(),
                    base.y.to_bits(),
                    base.vx.to_bits(),
                    base.vy.to_bits()
                ),
                "fast on settled must keep bits like a skip: {base:?}"
            );
        }
    }

    #[test]
    fn step_with_h_vy_nonfinite_matches_scalar() {
        // Coverage for the two-check finite fast path: a non-finite `vy`
        // always makes `y` non-finite after `y += vy*h` (`h > 0`), so
        // checking `x`/`y` alone keeps the same truth table as also
        // checking `vy`. Failure: any lane diverges scalar vs fast.
        let h = FIXED_DT;
        let gh = GRAVITY * h;
        for vy in [f32::INFINITY, f32::NEG_INFINITY, f32::NAN, 1.0, -2.5] {
            for y in [0.5, 5.0, f32::INFINITY, f32::NAN] {
                let base = Body::new(1.0, y, 0.5, vy, 0.5, 0.5);
                let mut a = [base];
                let mut b = [base];
                for x in a.iter_mut() {
                    step_body(x, FIXED_DT);
                }
                for x in b.iter_mut() {
                    step_with_h(x, h, gh);
                }
                assert_eq!(
                    (
                        a[0].x.to_bits(),
                        a[0].y.to_bits(),
                        a[0].vx.to_bits(),
                        a[0].vy.to_bits()
                    ),
                    (
                        b[0].x.to_bits(),
                        b[0].y.to_bits(),
                        b[0].vx.to_bits(),
                        b[0].vy.to_bits()
                    ),
                    "vy {vy:?} y {y:?} diverged: scalar {a:?} vs fast {b:?}"
                );
            }
        }
    }

    #[test]
    fn slide_ground_keeps_y_vy_and_matches_scalar() {
        // Ground-slide pin: `y == hy`, `vy == +0`, small `hy` keeps
        // `y`/`vy` through one Euler step plus clamp while only `x` moves.
        // Failure: any lane diverges scalar vs fast, or tiny `h` keeps
        // `vy == 0` (sub-ulp `y` must keep `vy == -gh` via the full path).
        let h = FIXED_DT;
        let gh = GRAVITY * h;
        for (x, vx) in [
            (1.25, 1.0),
            (-3.5, -2.0),
            (-0.0, 0.0),
            (f32::INFINITY, 1.0),
            (1.0, f32::NAN),
        ] {
            let base = Body::new(x, 0.5, vx, 0.0, 0.5, 0.5);
            let mut a = [base];
            let mut b = [base];
            for body in a.iter_mut() {
                step_body(body, FIXED_DT);
            }
            for body in b.iter_mut() {
                step_with_h(body, h, gh);
            }
            assert_eq!(
                (
                    a[0].x.to_bits(),
                    a[0].y.to_bits(),
                    a[0].vx.to_bits(),
                    a[0].vy.to_bits()
                ),
                (
                    b[0].x.to_bits(),
                    b[0].y.to_bits(),
                    b[0].vx.to_bits(),
                    b[0].vy.to_bits()
                ),
                "slide x {x:?} vx {vx:?} diverged: scalar {a:?} vs fast {b:?}"
            );
            if x.is_finite() && vx.is_finite() {
                assert_eq!(
                    (b[0].y.to_bits(), b[0].vy.to_bits()),
                    (b[0].hy.to_bits(), 0.0f32.to_bits()),
                    "slide must keep y/vy for x {x:?} vx {vx:?}: {b:?}"
                );
            }
        }
        // Tiny `h` stays on the full path: sub-ulp `y` keeps `vy == -gh`.
        {
            let tiny_h = 1e-7;
            let tiny_gh = GRAVITY * tiny_h;
            let base = Body::new(1.25, 0.5, 0.0, 0.0, 0.5, 0.5);
            let mut a = [base];
            let mut b = [base];
            for body in a.iter_mut() {
                step_body(body, tiny_h);
            }
            for body in b.iter_mut() {
                step_with_h(body, tiny_h, tiny_gh);
            }
            assert_eq!(
                (
                    a[0].x.to_bits(),
                    a[0].y.to_bits(),
                    a[0].vx.to_bits(),
                    a[0].vy.to_bits()
                ),
                (
                    b[0].x.to_bits(),
                    b[0].y.to_bits(),
                    b[0].vx.to_bits(),
                    b[0].vy.to_bits()
                ),
                "tiny slide diverged: scalar {a:?} vs fast {b:?}"
            );
            assert_eq!(
                b[0].vy.to_bits(),
                (-tiny_gh).to_bits(),
                "tiny h must keep vy == -gh, not fast-path 0: {b:?}"
            );
        }
    }

    #[test]
    fn fast_assumes_ge_fixed_matches_scalar() {
        // Failure: `step_with_h_fast` (h >= FIXED_DT only) diverges from
        // scalar `step_body` on any lane when h == FIXED_DT.
        let h = FIXED_DT;
        let gh = GRAVITY * h;
        let cases = [
            Body::new(1.25, 0.5, 0.0, 0.0, 0.5, 0.5),
            Body::new(1.25, 0.5, 1.0, 0.0, 0.5, 0.5),
            Body::new(0.0, 5.0, 1.0, 0.0, 0.5, 0.5),
            Body::new(1.0, 0.5, 0.5, -0.0, 0.5, 0.5),
            Body::new(-0.0, 0.5, 0.0, 0.0, 0.5, 0.5),
            Body::new(1.0, f32::NAN, 0.5, f32::INFINITY, 0.5, 0.5),
            Body::new(f32::INFINITY, 0.5, 1.0, 0.0, 0.5, 0.5),
        ];
        for base in cases {
            let mut a = [base];
            let mut b = [base];
            for body in a.iter_mut() {
                step_body(body, FIXED_DT);
            }
            for body in b.iter_mut() {
                step_with_h_fast(body, h, gh);
            }
            assert_eq!(
                (
                    a[0].x.to_bits(),
                    a[0].y.to_bits(),
                    a[0].vx.to_bits(),
                    a[0].vy.to_bits()
                ),
                (
                    b[0].x.to_bits(),
                    b[0].y.to_bits(),
                    b[0].vx.to_bits(),
                    b[0].vy.to_bits()
                ),
                "fast diverged for {base:?}: scalar {a:?} vs fast {b:?}"
            );
        }
        let mut a = mixed_11k(2_040);
        let mut b = a.clone();
        for body in a.iter_mut() {
            step_body(body, FIXED_DT);
        }
        integrate(&mut b, FIXED_DT);
        let mut c = mixed_11k(2_040);
        for body in c.iter_mut() {
            step_with_h_fast(body, h, gh);
        }
        for (i, ((x, y), z)) in a.iter().zip(b.iter()).zip(c.iter()).enumerate() {
            assert_eq!(
                (x.x.to_bits(), x.y.to_bits(), x.vx.to_bits(), x.vy.to_bits()),
                (y.x.to_bits(), y.y.to_bits(), y.vx.to_bits(), y.vy.to_bits()),
                "integrate diverged at {i}"
            );
            assert_eq!(
                (x.x.to_bits(), x.y.to_bits(), x.vx.to_bits(), x.vy.to_bits()),
                (z.x.to_bits(), z.y.to_bits(), z.vx.to_bits(), z.vy.to_bits()),
                "fast diverged at {i}: scalar {x:?} vs fast {z:?}"
            );
        }
    }
}
