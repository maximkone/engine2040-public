//! Frame slice arena: presized typed scratch for per-frame words.
//! Owner: engine-core (std-only; every crate may depend on it).
//! Units: words (`u32`) for capacity and use. Determinism: same pushes
//! give same slices; fresh slices read zeroed so replays repeat.
//! Errors: typed [`ArenaError`], never an implicit grow. Budget: setup
//! allocates once; steady-state frames allocate nothing, proven by
//! `alloc_probe` tests. Threading: `&mut` scoped to one frame on one
//! thread; slices never outlive the frame's `reset`.
//! Caller: `crates/perf/src/pgo_harness.rs` stages frame words here.

use std::fmt::{self, Display, Formatter};

/// Frame-scratch failure: what the arena refused, with word counts carried.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArenaError {
    /// The cursor plus the request overran capacity. The arena never grows
    /// implicitly: presize at setup or call [`SliceArena::reserve_words`]
    /// between frames, never mid-frame.
    Overflow {
        /// Total words needed (cursor plus the request).
        needed_words: usize,
        /// Usable capacity in words.
        capacity_words: usize,
    },
}

impl Display for ArenaError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Overflow {
                needed_words,
                capacity_words,
            } => write!(
                f,
                "slice arena overflow (needed {needed_words} words, {capacity_words} reserved): presize the arena at setup"
            ),
        }
    }
}

impl std::error::Error for ArenaError {}

/// Grow `buf` by `additional` slots and zero the new capacity. Setup-time only
/// (it may allocate); every presized frame arena grows through this one owner.
pub fn grow_zeroed<T: Default + Clone>(buf: &mut Vec<T>, additional: usize) {
    buf.reserve(additional);
    let capacity = buf.capacity();
    buf.resize(capacity, T::default());
}

/// Presized bump arena for one frame's typed scratch.
///
/// The buffer is allocated and zeroed once at setup
/// ([`SliceArena::with_capacity_words`] or [`SliceArena::reserve_words`]);
/// [`SliceArena::reset`] rewinds the cursor without touching the allocator,
/// so steady-state frames allocate nothing. Staging copies caller words into
/// the buffer and hands back arena-borrowed slices: nothing outlives the
/// frame's `reset`. The arena holds plain words and contains no `unsafe`.
///
/// # Example
///
/// ```rust
/// use engine_core::arena::SliceArena;
///
/// let mut arena = SliceArena::with_capacity_words(8);
/// assert!(arena.push_slice(&[1u32, 2, 3]).is_ok());
/// assert_eq!(arena.used_words(), 3);
/// arena.reset();
/// assert_eq!(arena.used_words(), 0);
/// ```
#[derive(Debug, Default)]
pub struct SliceArena {
    /// Owned scratch buffer, sized and zeroed once at setup and reused every
    /// frame. Length is the usable capacity: every word is initialized, so
    /// pushes borrow it with safe slicing only.
    buf: Vec<u32>,
    /// Bump cursor in words from the buffer start.
    cursor: usize,
}

impl SliceArena {
    /// Empty arena: every non-empty push overflows until
    /// [`SliceArena::reserve_words`] sizes it. Allocates nothing.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            buf: Vec::new(),
            cursor: 0,
        }
    }

    /// Arena presized for `capacity_words` of scratch. Allocates and zeroes
    /// once, at setup; steady-state frames on top of this never allocate.
    #[must_use]
    pub fn with_capacity_words(capacity_words: usize) -> Self {
        Self {
            buf: vec![0u32; capacity_words],
            cursor: 0,
        }
    }

    /// Grow capacity by `additional_words` (setup only: warmup, level load,
    /// never mid-frame). May allocate; steady-state frames never call it.
    /// New words are zeroed, keeping the length-is-capacity invariant.
    pub fn reserve_words(&mut self, additional_words: usize) {
        grow_zeroed(&mut self.buf, additional_words);
    }

    /// Rewind the cursor for the next frame. Touches no allocator and keeps
    /// every word of capacity; old contents stay readable until overwritten.
    pub fn reset(&mut self) {
        self.cursor = 0;
    }

    /// Scratch used since the last [`SliceArena::reset`], in words.
    #[must_use]
    pub const fn used_words(&self) -> usize {
        self.cursor
    }

    /// Total scratch capacity in words (the initialized length).
    #[must_use]
    pub fn capacity_words(&self) -> usize {
        self.buf.len()
    }

    /// Words left from the cursor to capacity.
    #[must_use]
    pub fn remaining_words(&self) -> usize {
        self.buf.len().saturating_sub(self.cursor)
    }

    /// Bump out `len` zeroed words for the frame. Overflow is a typed error,
    /// never an implicit grow. Zeroing keeps contents deterministic.
    ///
    /// # Errors
    ///
    /// [`ArenaError::Overflow`] when the request overruns capacity.
    pub fn alloc(&mut self, len: usize) -> Result<&mut [u32], ArenaError> {
        if len == 0 {
            return Ok(&mut []);
        }
        let end = self.cursor.saturating_add(len);
        if end > self.buf.len() {
            return Err(ArenaError::Overflow {
                needed_words: end,
                capacity_words: self.buf.len(),
            });
        }
        let start = self.cursor;
        self.cursor = end;
        let words = &mut self.buf[start..end];
        words.fill(0);
        Ok(words)
    }

    /// Stage `values` into scratch and hand back the arena copy. An empty
    /// input returns an empty slice without moving the cursor.
    ///
    /// # Errors
    ///
    /// [`ArenaError::Overflow`] when the request overruns capacity.
    pub fn push_slice(&mut self, values: &[u32]) -> Result<&mut [u32], ArenaError> {
        if values.is_empty() {
            return Ok(&mut []);
        }
        let dest = self.alloc(values.len())?;
        dest.copy_from_slice(values);
        Ok(dest)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Fnv64, Rng};

    /// One deterministic frame over caller scratch: three word shapes from
    /// the `seed`/`frame` stream, sorted cull keys, staged into the arena
    /// and folded back. Mirrors the perf harness shape at a smaller size so
    /// the primitive's steady-state cost is what the probe measures.
    fn run_frame(arena: &mut SliceArena, seed: u64, frame: u32) -> Result<u64, ArenaError> {
        arena.reset();
        let mut stream = Rng::new(seed).fork(u64::from(frame));
        let mut keys = [0u32; 64];
        for key in keys.iter_mut() {
            *key = stream.next_u32();
        }
        let mut pose = [0u32; 32];
        for word in pose.iter_mut() {
            *word = stream.next_u32();
        }
        keys.sort_unstable();
        let mut fold = Fnv64::new();
        fold.write_u32(frame);
        let staged = arena.push_slice(&keys)?;
        for word in staged.iter() {
            fold.write_u32(*word);
        }
        let staged = arena.push_slice(&pose)?;
        for word in staged.iter() {
            fold.write_u32(*word);
        }
        Ok(fold.finish())
    }

    #[test]
    fn staged_slices_round_trip() {
        let mut arena = SliceArena::with_capacity_words(16);
        let staged = arena.push_slice(&[10u32, 20, 30]).expect("fits");
        assert_eq!(staged, &[10, 20, 30]);
        let flag = arena.push_slice(&[7u32, 8]).expect("room left");
        assert_eq!(flag, &[7, 8]);
        assert_eq!(arena.used_words(), 3 + 2);
        assert_eq!(arena.capacity_words(), 16);
        assert_eq!(arena.remaining_words(), 16 - 5);
        let empty = arena.push_slice(&[]).expect("empty needs no words");
        assert!(empty.is_empty());
        assert_eq!(arena.used_words(), 5, "empty push moves no cursor");
    }

    #[test]
    fn reset_reuses_the_same_buffer() {
        let mut arena = SliceArena::with_capacity_words(16);
        let first = {
            let scratch = arena.push_slice(&[1u32, 2]).expect("fits");
            assert_eq!(scratch, &[1, 2]);
            scratch.as_ptr()
        };
        arena.reset();
        assert_eq!(arena.used_words(), 0);
        let second = {
            let scratch = arena.push_slice(&[3u32, 4]).expect("fits again");
            assert_eq!(scratch, &[3, 4]);
            scratch.as_ptr()
        };
        assert_eq!(first, second, "reset rewinds to the same buffer start");
    }

    #[test]
    fn overflow_names_words_and_keeps_the_cursor() {
        let mut arena = SliceArena::with_capacity_words(4);
        let error = arena
            .push_slice(&[0u32; 8])
            .expect_err("8 words cannot fit 4");
        assert_eq!(
            error,
            ArenaError::Overflow {
                needed_words: 8,
                capacity_words: 4,
            }
        );
        assert!(error.to_string().contains('8'), "{error}");
        assert_eq!(arena.used_words(), 0, "refused push moves no cursor");
        assert!(
            arena.push_slice(&[0u32; 4]).is_ok(),
            "fitting push still works"
        );
        let _: &dyn std::error::Error = &error;
    }

    #[test]
    fn empty_arena_fits_only_empty_pushes() {
        let mut arena = SliceArena::new();
        assert!(arena.push_slice(&[]).is_ok(), "empty push needs no words");
        assert_eq!(arena.used_words(), 0);
        assert!(
            arena.push_slice(&[1u32]).is_err(),
            "any word overflows empty"
        );
        arena.reserve_words(128);
        assert!(
            run_frame(&mut arena, 7, 0).is_ok(),
            "explicit setup growth sizes the arena"
        );
    }

    #[test]
    fn steady_frames_allocate_nothing_and_repeat() {
        let (_, probe) = crate::alloc_probe::measure(|| std::hint::black_box(vec![0u8; 64]));
        assert!(probe.calls >= 1, "alloc probe is blind");
        // Setup allocates once, outside the window: the probe watches the
        // steady-state frames only, which must allocate nothing.
        let mut arena = SliceArena::with_capacity_words(128);
        let mut run = || {
            let mut fold = Fnv64::new();
            for frame in 0..60u32 {
                let hash = run_frame(&mut arena, 7, frame).expect("presized fits");
                fold.write_u64(hash);
            }
            fold.finish()
        };
        let (first_hash, first_seen) = crate::alloc_probe::measure(&mut run);
        let (second_hash, second_seen) = crate::alloc_probe::measure(&mut run);
        assert_eq!(first_hash, second_hash, "steady frames hash identically");
        assert_eq!(first_seen.calls, 0, "steady frames make no allocations");
        assert_eq!(first_seen.bytes, 0, "steady frames allocate no bytes");
        assert_eq!(second_seen.calls, 0, "repeat run makes no allocations");
        assert_eq!(second_seen.bytes, 0, "repeat run allocates no bytes");
    }

    #[test]
    fn seeded_frames_move_the_hash() {
        let run = |seed: u64| {
            let mut arena = SliceArena::with_capacity_words(128);
            run_frame(&mut arena, seed, 3).expect("presized fits")
        };
        assert_ne!(run(7), run(1_000_007), "seed moves the frame hash");
    }

    #[test]
    fn injected_heap_alloc_trips_the_counter() {
        // Negative control: the zeros above are honest because the same
        // window observes a real allocation.
        let ((), observed) = crate::alloc_probe::measure(|| {
            std::hint::black_box(vec![0u8; 64]);
        });
        assert_eq!(observed.calls, 1, "counter observes one alloc");
        assert_eq!(observed.bytes, 64, "counter observes 64 bytes");
    }
}
