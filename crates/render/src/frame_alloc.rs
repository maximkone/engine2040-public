//! frame_alloc: one reusable byte buffer as a per-frame bump arena.
//!
//! Owner: render frame path; consumer `crate::frame` encodes uniforms
//! and draw records into it. Units: bytes, offsets. Determinism: same
//! push order gives identical bytes. Errors: typed `FrameAllocError`.
//! Budget: 64 KiB default backing, O(1) bump, no heap use once warm.

use std::error::Error;
use std::fmt;

/// Default backing bytes; holds 256 draw records plus uniforms.
pub const DEFAULT_ARENA_BYTES: usize = 64 * 1024;

/// Largest backing any frame may warm; bigger is a config bug.
pub const MAX_ARENA_BYTES: usize = 4 * 1024 * 1024;

/// Loud bump failures; every variant names the budget it hit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameAllocError {
    /// Bump ran past backing; warm bigger or push less per frame.
    OverBudget {
        /// Bytes the failed push asked for.
        requested: usize,
        /// Bytes already used this frame.
        used: usize,
        /// Backing bytes behind the arena.
        cap: usize,
    },
    /// Warm asked past the max; shrink the request.
    TooLarge {
        /// Bytes the warm asked for.
        requested: usize,
        /// Largest backing allowed.
        cap: usize,
    },
    /// Alignment is zero or not a power of two; pass 1, 4 or 8.
    BadAlign {
        /// Alignment the caller passed.
        align: usize,
    },
}

impl fmt::Display for FrameAllocError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OverBudget {
                requested,
                used,
                cap,
            } => write!(
                f,
                "frame arena over budget: {requested} bytes over {used}/{cap} used"
            ),
            Self::TooLarge { requested, cap } => {
                write!(f, "frame arena too large: {requested} over max {cap}")
            }
            Self::BadAlign { align } => {
                write!(f, "frame arena bad align: {align} is not 1, 2, 4 or 8")
            }
        }
    }
}

impl Error for FrameAllocError {}

/// Reusable per-frame bump arena over one byte buffer.
///
/// The buffer grows only on the warm path; steady frames bump inside the
/// high-water mark and never touch the heap. Call `begin_frame` once per
/// frame, then push; bytes before `used` are this frame's scratch and die
/// at the next `begin_frame`, never stored past it.
#[derive(Clone, Debug, Default)]
pub struct FrameArena {
    buf: Vec<u8>,
    used: usize,
}

impl FrameArena {
    /// Empty arena with no allocation; warm before the first frame.
    #[must_use]
    pub fn new() -> Self {
        Self {
            buf: Vec::new(),
            used: 0,
        }
    }

    /// Arena warmed to `bytes` backing; cold path, may allocate once.
    pub fn with_capacity(bytes: usize) -> Result<Self, FrameAllocError> {
        let mut arena = Self::new();
        arena.warm(bytes)?;
        Ok(arena)
    }

    /// Grow backing toward `bytes` without resetting the cursor. Cold
    /// path: steady frames never call it, so growth never hides in a
    /// frame budget. Refuses past [`MAX_ARENA_BYTES`] loudly.
    pub fn warm(&mut self, bytes: usize) -> Result<(), FrameAllocError> {
        if bytes > MAX_ARENA_BYTES {
            return Err(FrameAllocError::TooLarge {
                requested: bytes,
                cap: MAX_ARENA_BYTES,
            });
        }
        if bytes > self.buf.len() {
            self.buf.resize(bytes, 0);
        }
        Ok(())
    }

    /// Start a frame: the cursor returns to zero, backing stays warm.
    pub fn begin_frame(&mut self) {
        self.used = 0;
    }

    /// Bytes used this frame.
    #[must_use]
    pub fn used(&self) -> usize {
        self.used
    }

    /// Backing bytes behind the arena.
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.buf.capacity()
    }

    /// Most bytes ever bumped in one frame; steady frames stay under it.
    #[must_use]
    pub fn high_water(&self) -> usize {
        self.buf.len()
    }

    /// This frame's scratch bytes, in push order.
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        let end = self.used.min(self.buf.len());
        &self.buf[..end]
    }

    /// FNV-1a digest over this frame's bytes; equal pushes give it twice.
    #[must_use]
    pub fn digest(&self) -> u64 {
        engine_core::hash_bytes(self.bytes())
    }

    /// Bump `len` bytes at `align`; returns the offset. Grows the buffer
    /// only past the high-water mark; steady frames reuse in place.
    pub fn alloc_bytes(&mut self, len: usize, align: usize) -> Result<usize, FrameAllocError> {
        if align == 0 || !align.is_power_of_two() {
            return Err(FrameAllocError::BadAlign { align });
        }
        let mask = align.saturating_sub(1);
        let aligned = self.used.saturating_add(mask) & !mask;
        let end = aligned.saturating_add(len);
        let cap = self.capacity();
        if end > cap && end > self.buf.len() {
            return Err(FrameAllocError::OverBudget {
                requested: len,
                used: self.used,
                cap,
            });
        }
        if end > self.buf.len() {
            self.buf.resize(end, 0);
        }
        self.used = end;
        Ok(aligned)
    }

    /// Copy `src` into scratch; returns its offset.
    pub fn push_bytes(&mut self, src: &[u8]) -> Result<usize, FrameAllocError> {
        let off = self.alloc_bytes(src.len(), 1)?;
        let end = off.saturating_add(src.len());
        if end <= self.buf.len() {
            self.buf[off..end].copy_from_slice(src);
        }
        Ok(off)
    }

    /// Push one little-endian `u32`; returns its offset.
    pub fn push_u32_le(&mut self, value: u32) -> Result<usize, FrameAllocError> {
        self.push_bytes(&value.to_le_bytes())
    }

    /// Push one little-endian `u64`; returns its offset.
    pub fn push_u64_le(&mut self, value: u64) -> Result<usize, FrameAllocError> {
        self.push_bytes(&value.to_le_bytes())
    }

    /// Push one `f32` by its bit pattern; returns its offset.
    pub fn push_f32_le(&mut self, value: f32) -> Result<usize, FrameAllocError> {
        self.push_u32_le(value.to_bits())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_core::alloc_probe::measure;

    #[test]
    fn naive_vec_per_frame_allocates_every_frame() {
        let (_, count) = measure(|| {
            for frame in 0..120u32 {
                let mut scratch = vec![0u8; 1024];
                scratch[0] = (frame & 0xFF) as u8;
                assert_eq!(scratch.len(), 1024);
            }
        });
        assert!(
            count.calls >= 120,
            "naive per-frame vec must allocate per frame, got {}",
            count.calls
        );
    }

    #[test]
    fn steady_120_frames_read_zero_allocs() {
        let mut arena = FrameArena::with_capacity(4096).expect("warms");
        arena.begin_frame();
        for v in 0..64u64 {
            arena.push_u64_le(v).expect("warmup fits");
        }
        let warmed_used = arena.used();
        let warmed_cap = arena.capacity();
        let (_, count) = measure(|| {
            for _ in 0..120u32 {
                arena.begin_frame();
                for v in 0..64u64 {
                    arena.push_u64_le(v).expect("steady fits");
                }
                assert_eq!(arena.used(), warmed_used);
            }
        });
        assert_eq!(count.calls, 0, "steady frames must not allocate");
        assert_eq!(arena.capacity(), warmed_cap);
    }

    #[test]
    fn same_pushes_give_same_bytes_and_digest() {
        let run = || {
            let mut arena = FrameArena::with_capacity(256).expect("warms");
            arena.begin_frame();
            arena.push_u64_le(0x1D1D).expect("fits");
            arena.push_u32_le(7).expect("fits");
            arena.push_f32_le(1.5).expect("fits");
            (arena.bytes().to_vec(), arena.digest())
        };
        let (first_bytes, first_digest) = run();
        let (second_bytes, second_digest) = run();
        assert_eq!(first_bytes, second_bytes);
        assert_eq!(first_digest, second_digest);
    }

    #[test]
    fn over_budget_refuses_and_holds_the_cursor() {
        let mut arena = FrameArena::with_capacity(64).expect("warms");
        arena.begin_frame();
        arena.push_bytes(&[9u8; 64]).expect("fills backing");
        let held = arena.used();
        let err = arena.push_bytes(&[1u8; 8]).expect_err("past cap refuses");
        assert_eq!(
            err,
            FrameAllocError::OverBudget {
                requested: 8,
                used: held,
                cap: arena.capacity(),
            }
        );
        assert_eq!(arena.used(), held, "failed push leaves the cursor");
    }

    #[test]
    fn bad_align_is_refused() {
        let mut arena = FrameArena::with_capacity(64).expect("warms");
        for bad in [0usize, 3, 6] {
            assert_eq!(
                arena.alloc_bytes(4, bad),
                Err(FrameAllocError::BadAlign { align: bad })
            );
        }
        assert!(arena.alloc_bytes(4, 1).is_ok());
        assert!(arena.alloc_bytes(4, 4).is_ok());
        assert!(arena.alloc_bytes(8, 8).is_ok());
    }

    #[test]
    fn error_display_names_the_budget() {
        let over = FrameAllocError::OverBudget {
            requested: 8,
            used: 64,
            cap: 64,
        };
        assert!(format!("{over}").contains("over budget"));
        let big = FrameAllocError::TooLarge {
            requested: MAX_ARENA_BYTES + 1,
            cap: MAX_ARENA_BYTES,
        };
        assert!(format!("{big}").contains("too large"));
        let align = FrameAllocError::BadAlign { align: 3 };
        assert!(format!("{align}").contains("align"));
        assert!(format!("{align}").contains('3'));
    }
}
