//! jobs: bulk-loop helpers on the crate's shared [`JobPool`](crate::pool::JobPool). Bake/import/verify only, never sim hot paths. Same input = same bytes.
//!
//! Each chunk is one pool job; no call spawns a thread.

use crate::pool;

/// Chunk size for bulk loops. `0` normalizes to `1` via [`ChunkSize::value`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChunkSize(pub usize);

impl ChunkSize {
    /// Build a chunk size. `0` is accepted and means `1` (see [`value`]).
    ///
    /// [`value`]: Self::value
    pub fn new(n: usize) -> Self {
        Self(n)
    }
    /// Usable chunk length, at least `1`.
    pub fn value(self) -> usize {
        self.0.max(1)
    }
}

/// Split `len` items into index-ordered `(start, end)` ranges of at most
/// `chunk` items. Empty input gives no ranges; a chunk larger than `len`
/// gives one range. Pure and single-threaded: the parallel helpers below
/// reuse exactly these boundaries.
pub fn split_ranges(len: usize, chunk: ChunkSize) -> Vec<(usize, usize)> {
    let c = chunk.value();
    let mut out = Vec::with_capacity(len.div_ceil(c));
    let mut s = 0;
    while s < len {
        let e = (s + c).min(len);
        out.push((s, e));
        s = e;
    }
    out
}

/// Run `f(index, item)` over `items` in place, one pool job per chunk.
/// Each element is touched exactly once by its owning chunk in index order,
/// so a per-element-pure `f` gives byte-identical output to the serial loop.
/// A panic in `f` re-raises on the caller. Bulk loops only; never sim/net
/// tick paths.
pub fn par_for_each_mut<T, F>(items: &mut [T], chunk: ChunkSize, f: F)
where
    T: Send,
    F: Fn(usize, &mut T) + Sync,
{
    if items.is_empty() {
        return;
    }
    let c = chunk.value();
    let g = &f;
    pool::shared().scope(|s| {
        for (ci, part) in items.chunks_mut(c).enumerate() {
            let base = ci * c;
            s.spawn(move || {
                for (i, v) in part.iter_mut().enumerate() {
                    g(base + i, v);
                }
            });
        }
    });
}

/// Map `items` to a new `Vec` with deterministic index-ordered merge: chunk
/// `k` fills output rows `[k*c, k*c + len)`, so the result equals the serial
/// `items.iter().map(f).collect()`. Requires `U: Default` only to pre-size
/// the output without `unsafe`. One pool job per chunk; a panic in `f`
/// re-raises on the caller. Bulk loops only, never sim/net tick paths.
pub fn par_map_default<T, U, F>(items: &[T], chunk: ChunkSize, f: F) -> Vec<U>
where
    T: Sync,
    U: Default + Send,
    F: Fn(&T) -> U + Sync,
{
    let n = items.len();
    let mut out: Vec<U> = Vec::with_capacity(n);
    out.resize_with(n, U::default);
    if n == 0 {
        return out;
    }
    let c = chunk.value();
    let g = &f;
    pool::shared().scope(|s| {
        for (src, dst) in items.chunks(c).zip(out.chunks_mut(c)) {
            s.spawn(move || {
                for (a, b) in src.iter().zip(dst.iter_mut()) {
                    *b = g(a);
                }
            });
        }
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_core::hash_u64s;

    fn work(x: &u32) -> u64 {
        (*x as u64).wrapping_mul(0x9e3779b97f4a7c15).rotate_left(17)
    }

    #[test]
    fn parallel_map_matches_serial_hash() {
        let v: Vec<u32> = (0..2048).collect();
        let serial: Vec<u64> = v.iter().map(work).collect();
        let par = par_map_default(&v, ChunkSize::new(64), work);
        assert_eq!(par, serial);
        assert_eq!(hash_u64s(&par), hash_u64s(&serial));
    }

    #[test]
    fn merge_order_is_index_order() {
        assert_eq!(
            split_ranges(10, ChunkSize::new(3)),
            vec![(0, 3), (3, 6), (6, 9), (9, 10)]
        );
        let v: Vec<u32> = (0..16).collect();
        let par = par_map_default(&v, ChunkSize::new(5), |x| *x + 1);
        let serial: Vec<u32> = v.iter().map(|x| *x + 1).collect();
        assert_eq!(par, serial);
    }

    #[test]
    fn empty_and_oversize_chunk_edges() {
        assert!(split_ranges(0, ChunkSize::new(8)).is_empty());
        assert_eq!(split_ranges(3, ChunkSize::new(99)), vec![(0, 3)]);
        assert_eq!(
            split_ranges(4, ChunkSize::new(0)),
            vec![(0, 1), (1, 2), (2, 3), (3, 4)]
        );
        let e: Vec<u32> = vec![];
        assert!(par_map_default(&e, ChunkSize::new(8), |x| *x).is_empty());
    }

    #[test]
    fn different_chunks_same_hash() {
        let v: Vec<u32> = (0..512).collect();
        let a = par_map_default(&v, ChunkSize::new(1), work);
        let b = par_map_default(&v, ChunkSize::new(1024), work);
        assert_eq!(a, b);
    }

    #[test]
    fn a_panicking_chunk_reraises_on_the_caller() {
        let mut v: Vec<u32> = (0..64).collect();
        let run = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            par_for_each_mut(&mut v, ChunkSize::new(8), |i, _| {
                assert!(i != 37, "item 37");
            });
        }));
        assert!(run.is_err(), "the panic is never a silent no-op");
        let run = std::panic::catch_unwind(|| {
            par_map_default(&[1u32, 2, 3], ChunkSize::new(1), |x| {
                assert!(*x != 2, "item 2");
                *x
            })
        });
        assert!(run.is_err());
    }

    #[test]
    fn for_each_mut_matches_serial() {
        let mut par: Vec<u32> = (0..1024).collect();
        let mut serial = par.clone();
        par_for_each_mut(&mut par, ChunkSize::new(96), |i, x| *x += i as u32);
        for (i, x) in serial.iter_mut().enumerate() {
            *x += i as u32;
        }
        assert_eq!(par, serial);
    }
}
