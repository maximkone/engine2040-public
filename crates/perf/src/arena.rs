//! Per-frame bump arena: presized transient scratch for render, anim and audio.
//! Consumer: the steady-state allocation proofs (`tests/alloc.rs`) plus the
//! render cull workload (`workloads::RenderCull`), which stages arena-backed
//! keys into the real draw list. Units: bytes for capacity and usage, frames
//! for runs. Determinism: the arena holds no hash state; given the same seed
//! and frame index [`FrameTransients`] produces byte-identical scratch,
//! folded with `engine_core::Fnv64`. Threading: `&mut` scoped to one frame
//! on one thread; the arena is not shared across threads mid-frame. Errors:
//! typed [`FrameError`], never an implicit grow. Budget: construction and
//! [`FrameArena::reserve_bytes`] allocate once at setup; every steady-state
//! frame allocates zero heap bytes, proven by [`FrameTransients::steady`]
//! through the crate's counting allocator.
//!
//! The arena stages opaque bytes only and contains no `unsafe`: generic
//! byte-to-value retyping has no safe std-only form, so typed compute (key
//! sorts, word folds) lives in caller stack scratch over the same seeded
//! streams, and the arena carries the staged bytes the frame folds.

use crate::alloc::{self, AllocSnapshot};
use engine_core::{Fnv64, Rng};
use std::fmt::{self, Display, Formatter};
use std::mem::{align_of, size_of};

/// Cull sort keys scratched per frame (render stand-in: packed depth+index).
pub const CULL_KEYS: usize = 1024;
/// Pose words scratched per frame (anim stand-in: packed fixed-point pairs).
pub const POSE_WORDS: usize = 512;
/// Mix words scratched per frame (audio stand-in: packed sample pairs).
pub const MIX_WORDS: usize = 256;
/// Presized scratch for one [`FrameTransients`] frame: every word at 4-byte
/// alignment plus headroom for alignment padding between the three pushes.
pub const FRAME_BYTES: usize = (CULL_KEYS + POSE_WORDS + MIX_WORDS) * 4 + 64;

/// Frame-scratch failure: what the arena refused, with byte counts carried.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameError {
    /// The bump cursor plus padding plus the request overran capacity. The
    /// arena never grows implicitly: presize at setup or call
    /// [`FrameArena::reserve_bytes`] between frames, never mid-frame.
    Overflow {
        /// Requested payload bytes (without padding).
        requested_bytes: usize,
        /// Bytes left from the cursor to capacity.
        available_bytes: usize,
    },
    /// The requested alignment is zero or not a power of two.
    BadAlign {
        /// Offending alignment in bytes.
        align_bytes: usize,
    },
}

impl Display for FrameError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Overflow {
                requested_bytes,
                available_bytes,
            } => write!(
                f,
                "frame: scratch overflow (requested {requested_bytes} bytes, {available_bytes} available): presize the arena at setup"
            ),
            Self::BadAlign { align_bytes } => {
                write!(f, "frame: bad alignment ({align_bytes} bytes): must be a nonzero power of two")
            }
        }
    }
}

impl std::error::Error for FrameError {}

/// Presized bump arena for one frame's transient scratch.
///
/// The buffer is allocated once at setup ([`FrameArena::with_capacity_bytes`]
/// or [`FrameArena::reserve_bytes`]) and fully initialized there;
/// [`FrameArena::reset`] rewinds the cursor without touching the allocator,
/// so steady-state frames allocate nothing. Staging copies caller bytes into
/// the buffer and hands back arena-borrowed slices: nothing outlives the
/// frame's `reset`. The arena holds opaque bytes and contains no `unsafe`.
///
/// # Example
///
/// ```rust
/// use engine_perf::arena::FrameArena;
///
/// let mut arena = FrameArena::with_capacity_bytes(64);
/// assert!(arena.push_bytes(&[1u8, 2, 3]).is_ok());
/// assert_eq!(arena.used_bytes(), 3);
/// arena.reset();
/// assert_eq!(arena.used_bytes(), 0);
/// ```
#[derive(Debug, Default)]
pub struct FrameArena {
    /// Owned scratch buffer, sized and zeroed once at setup and reused every
    /// frame. Length is the usable capacity: every byte is initialized, so
    /// pushes borrow it with safe slicing only.
    buf: Vec<u8>,
    /// Bump cursor in bytes from the buffer start.
    cursor: usize,
}

impl FrameArena {
    /// Empty arena: every push overflows until [`FrameArena::reserve_bytes`]
    /// sizes it. Allocates nothing.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            buf: Vec::new(),
            cursor: 0,
        }
    }

    /// Arena presized for `capacity_bytes` of scratch. Allocates and zeroes
    /// once, at setup; steady-state frames on top of this never allocate.
    #[must_use]
    pub fn with_capacity_bytes(capacity_bytes: usize) -> Self {
        Self {
            buf: vec![0u8; capacity_bytes],
            cursor: 0,
        }
    }

    /// Grow capacity by `additional_bytes` (setup only: warmup, level load,
    /// never mid-frame). May allocate; steady-state frames must not call it.
    /// New bytes are zeroed, keeping the length-is-capacity invariant.
    pub fn reserve_bytes(&mut self, additional_bytes: usize) {
        engine_core::arena::grow_zeroed(&mut self.buf, additional_bytes);
    }

    /// Rewind the cursor for the next frame. Touches no allocator and keeps
    /// every byte of capacity; old contents stay readable until overwritten.
    pub fn reset(&mut self) {
        self.cursor = 0;
    }

    /// Scratch used since the last [`FrameArena::reset`], in bytes, padding
    /// included.
    #[must_use]
    pub const fn used_bytes(&self) -> usize {
        self.cursor
    }

    /// Total scratch capacity in bytes (the initialized length).
    #[must_use]
    pub fn capacity_bytes(&self) -> usize {
        self.buf.len()
    }

    /// Bytes left from the cursor to capacity.
    #[must_use]
    pub fn remaining_bytes(&self) -> usize {
        self.buf.len().saturating_sub(self.cursor)
    }

    /// Bump out `size_bytes` zeroed bytes at `align_bytes` alignment.
    /// Returns the frame-scoped scratch; overflow is a typed error, never an
    /// implicit grow. The slice borrows the initialized buffer, so no raw
    /// pointer work is needed; zeroing keeps contents deterministic.
    ///
    /// # Errors
    ///
    /// [`FrameError::BadAlign`] on a zero or non-power-of-two alignment,
    /// [`FrameError::Overflow`] when the padded request overruns capacity.
    pub fn alloc_raw(
        &mut self,
        size_bytes: usize,
        align_bytes: usize,
    ) -> Result<&mut [u8], FrameError> {
        if !align_bytes.is_power_of_two() {
            return Err(FrameError::BadAlign { align_bytes });
        }
        if size_bytes == 0 {
            return Ok(&mut []);
        }
        let capacity = self.buf.len();
        // `align_offset` measures from the live base pointer, so the padded
        // start is truly aligned even though `Vec<u8>` only guarantees 1.
        let padding = self
            .buf
            .as_ptr()
            .wrapping_add(self.cursor)
            .align_offset(align_bytes);
        if padding == usize::MAX {
            return Err(FrameError::BadAlign { align_bytes });
        }
        let start = self.cursor.saturating_add(padding);
        let end = start.saturating_add(size_bytes);
        if end > capacity {
            return Err(FrameError::Overflow {
                requested_bytes: size_bytes,
                available_bytes: capacity.saturating_sub(self.cursor),
            });
        }
        self.cursor = end;
        let bytes = &mut self.buf[start..end];
        bytes.fill(0);
        Ok(bytes)
    }

    /// Stage `bytes` into scratch and hand back the arena copy. An empty
    /// input returns an empty slice without moving the cursor.
    ///
    /// # Errors
    ///
    /// [`FrameError::Overflow`] when the request overruns capacity.
    pub fn push_bytes(&mut self, bytes: &[u8]) -> Result<&mut [u8], FrameError> {
        if bytes.is_empty() {
            return Ok(&mut []);
        }
        let dest = self.alloc_raw(bytes.len(), 1)?;
        dest.copy_from_slice(bytes);
        Ok(dest)
    }

    /// Stage `values` as little-endian words at word alignment and hand back
    /// the staged bytes. An empty input returns an empty slice without moving
    /// the cursor. The single word stager for the frame workloads, so the
    /// render cull and the transient folds read back exactly what the arena
    /// carries. Fully safe: words encode to bytes, never retype.
    ///
    /// # Errors
    ///
    /// [`FrameError::Overflow`] when the request overruns capacity.
    pub fn push_words(&mut self, values: &[u32]) -> Result<&mut [u8], FrameError> {
        if values.is_empty() {
            return Ok(&mut []);
        }
        let dest = self.alloc_raw(
            values.len().saturating_mul(size_of::<u32>()),
            align_of::<u32>(),
        )?;
        for (slot, value) in dest.chunks_exact_mut(size_of::<u32>()).zip(values.iter()) {
            slot.copy_from_slice(&value.to_le_bytes());
        }
        Ok(dest)
    }
}

/// Fold staged words back in little-endian order. Reads exactly what
/// [`FrameArena::push_words`] wrote, so the fold proves the frame's bytes
/// round-trip through the arena.
fn fold_staged_u32(fold: &mut Fnv64, staged: &[u8]) {
    for word in staged.chunks_exact(size_of::<u32>()) {
        fold.write_u32(u32::from_le_bytes([word[0], word[1], word[2], word[3]]));
    }
}

/// One frame of transient scratch across the three presentation owners:
/// render cull keys, anim pose words and audio mix words. Shapes are fixed
/// so the arena presize fits by construction; contents derive from
/// `seed`/`frame` streams, so runs repeat exactly.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameTransients {
    /// Stream seed for the scratch contents.
    pub seed: u64,
    /// Frames per [`FrameTransients::steady`] run.
    pub frames: u32,
}

impl FrameTransients {
    /// Workload name for reports and budget errors.
    pub const NAME: &'static str = "frame_transients";

    /// Canonical workload: seed 7, 120 frames of three-owner scratch.
    pub const fn canonical() -> Self {
        Self {
            seed: 7,
            frames: 120,
        }
    }

    /// Same shape with a different seed (regression proofs).
    pub const fn with_seed(self, seed: u64) -> Self {
        Self { seed, ..self }
    }

    /// Run one frame on `arena`: reset, stream the three transient shapes,
    /// sort the cull keys in stack scratch (the render-style transient
    /// compute), stage every shape into the arena and fold the staged bytes
    /// back into a frame hash. The arena must already fit [`FRAME_BYTES`];
    /// setup owns the presize, frames never grow.
    ///
    /// # Errors
    ///
    /// [`FrameError::Overflow`] when the arena was not presized.
    pub fn run_frame(&self, arena: &mut FrameArena, frame: u32) -> Result<u64, FrameError> {
        arena.reset();
        let mut stream = Rng::new(self.seed).fork(u64::from(frame));
        let mut keys = [0u32; CULL_KEYS];
        for key in keys.iter_mut() {
            *key = stream.next_u32();
        }
        let mut pose = [0u32; POSE_WORDS];
        for word in pose.iter_mut() {
            *word = stream.next_u32();
        }
        let mut mix = [0u32; MIX_WORDS];
        for word in mix.iter_mut() {
            *word = stream.next_u32();
        }
        keys.sort_unstable();
        let mut fold = Fnv64::new();
        fold.write_u32(frame);
        // Each stage hands back the arena bytes, which the fold reads back:
        // the frame hash proves the scratch round-trips through the arena.
        // Borrows end at each fold, so the arena re-borrows one stage at a
        // time.
        let staged = arena.push_words(&keys)?;
        fold_staged_u32(&mut fold, staged);
        let staged = arena.push_words(&pose)?;
        fold_staged_u32(&mut fold, staged);
        let staged = arena.push_words(&mix)?;
        fold_staged_u32(&mut fold, staged);
        Ok(fold.finish())
    }

    /// Steady-state proof: a warmed, presized arena runs every frame inside
    /// the counting window and must allocate nothing. Returns the folded
    /// frame hashes plus the window's allocation totals (both zero on pass).
    ///
    /// # Errors
    ///
    /// [`FrameError::Overflow`] when [`FRAME_BYTES`] stops fitting the
    /// shapes (a shape change without a presize bump: fix the constant).
    pub fn steady(&self) -> Result<(u64, AllocSnapshot), FrameError> {
        let mut arena = FrameArena::with_capacity_bytes(FRAME_BYTES);
        for frame in 0..self.frames {
            self.run_frame(&mut arena, frame)?;
        }
        let (outcome, observed) = alloc::measure(|| {
            let mut fold = Fnv64::new();
            let mut refusal: Option<FrameError> = None;
            for frame in 0..self.frames {
                match self.run_frame(&mut arena, frame) {
                    Ok(hash) => fold.write_u64(hash),
                    Err(error) => {
                        refusal = Some(error);
                        break;
                    }
                }
            }
            match refusal {
                Some(error) => Err(error),
                None => Ok(fold.finish()),
            }
        });
        let hash = outcome?;
        Ok((hash, observed))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn staged_bytes_round_trip() {
        let mut arena = FrameArena::with_capacity_bytes(64);
        let staged = arena.push_bytes(&[10u8, 20, 30]).expect("fits");
        assert_eq!(staged, &[10, 20, 30]);
        let flag = arena.push_bytes(&[7u8, 8]).expect("room left");
        assert_eq!(flag, &[7, 8]);
        assert_eq!(arena.used_bytes(), 3 + 2);
        assert_eq!(arena.capacity_bytes(), 64);
        assert_eq!(arena.remaining_bytes(), 64 - 5);
    }

    #[test]
    fn staged_words_round_trip_little_endian() {
        let mut arena = FrameArena::with_capacity_bytes(64);
        let staged = arena.push_words(&[0x0102_0304, 20]).expect("two words fit");
        assert_eq!(staged, &[0x04, 0x03, 0x02, 0x01, 20, 0, 0, 0]);
        let mut fold = Fnv64::new();
        fold_staged_u32(&mut fold, staged);
        let mut direct = Fnv64::new();
        direct.write_u32(0x0102_0304);
        direct.write_u32(20);
        assert_eq!(fold.finish(), direct.finish(), "fold reads staged words");
    }

    #[test]
    fn reset_reuses_the_same_buffer() {
        let mut arena = FrameArena::with_capacity_bytes(64);
        let first = {
            let scratch = arena.push_bytes(&[1u8, 2]).expect("fits");
            assert_eq!(scratch, &[1, 2]);
            scratch.as_ptr()
        };
        arena.reset();
        assert_eq!(arena.used_bytes(), 0);
        let second = {
            let scratch = arena.push_bytes(&[3u8, 4]).expect("fits again");
            assert_eq!(scratch, &[3, 4]);
            scratch.as_ptr()
        };
        assert_eq!(first, second, "reset rewinds to the same buffer start");
    }

    #[test]
    fn overflow_names_bytes_and_keeps_the_cursor() {
        let mut arena = FrameArena::with_capacity_bytes(8);
        let error = arena
            .push_bytes(&[0u8; 16])
            .expect_err("16 bytes cannot fit 8");
        assert_eq!(
            error,
            FrameError::Overflow {
                requested_bytes: 16,
                available_bytes: 8,
            }
        );
        assert!(error.to_string().contains("16"), "{error}");
        assert_eq!(arena.used_bytes(), 0, "refused push moves no cursor");
        assert!(
            arena.push_bytes(&[0u8; 8]).is_ok(),
            "fitting push still works"
        );
    }

    #[test]
    fn bad_align_and_padding_refuse_loudly() {
        let mut arena = FrameArena::with_capacity_bytes(64);
        assert_eq!(
            arena.alloc_raw(4, 3).expect_err("3 is no alignment"),
            FrameError::BadAlign { align_bytes: 3 }
        );
        assert_eq!(
            arena.alloc_raw(4, 0).expect_err("0 is no alignment"),
            FrameError::BadAlign { align_bytes: 0 }
        );
        arena.alloc_raw(1, 1).expect("one byte fits");
        let word = arena.alloc_raw(4, 4).expect("word fits after padding");
        assert_eq!(word.len(), 4);
        assert_eq!(arena.used_bytes(), 1 + 3 + 4, "padding counts as used");
        for error in [
            FrameError::Overflow {
                requested_bytes: 1,
                available_bytes: 0,
            },
            FrameError::BadAlign { align_bytes: 3 },
        ] {
            assert!(!error.to_string().is_empty(), "{error:?} explains itself");
            let _: &dyn std::error::Error = &error;
        }
    }

    #[test]
    fn empty_arena_fits_only_empty_pushes() {
        let mut arena = FrameArena::new();
        assert!(arena.push_bytes(&[]).is_ok(), "empty push needs no bytes");
        assert_eq!(arena.used_bytes(), 0);
        assert!(
            arena.push_bytes(&[1u8]).is_err(),
            "any byte overflows empty"
        );
        arena.reserve_bytes(FRAME_BYTES);
        assert!(
            FrameTransients::canonical()
                .run_frame(&mut arena, 0)
                .is_ok(),
            "explicit setup growth sizes the arena"
        );
    }

    #[test]
    fn steady_frames_allocate_nothing_and_repeat() {
        let workload = FrameTransients::canonical();
        let (first, snap_first) = workload.steady().expect("presized scratch fits");
        let (second, snap_second) = workload.steady().expect("repeat run fits");
        assert_eq!(first, second, "steady frames hash identically");
        for (label, snap) in [("first", snap_first), ("second", snap_second)] {
            assert_eq!(snap.bytes, 0, "{label} run allocates no bytes");
            assert_eq!(snap.count, 0, "{label} run makes no allocations");
        }
    }

    #[test]
    fn seeded_frames_move_the_hash() {
        let workload = FrameTransients::canonical();
        let moved = workload.with_seed(1_000_007);
        let (base, _) = workload.steady().expect("canonical fits");
        let (other, _) = moved.steady().expect("seeded fits");
        assert_ne!(base, other, "seed moves the folded frame hashes");
    }

    #[test]
    fn injected_heap_alloc_trips_the_counter() {
        // Negative control: the zeros above are honest because the same
        // window observes a real allocation.
        let ((), observed) = alloc::measure(|| {
            std::hint::black_box(vec![0u8; 64]);
        });
        assert_eq!(observed.count, 1, "counter observes one alloc");
        assert_eq!(observed.bytes, 64, "counter observes 64 bytes");
    }
}
