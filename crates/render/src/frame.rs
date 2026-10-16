//! frame: reused draw list plus uniform and draw encode over one arena.
//!
//! Owner: render frame path; consumer of `super::frame_alloc` and the next
//! submit-path slice. Units: draws, bytes. Determinism: integer
//! `(depth_bucket, id)` order, little-endian bytes. Errors: typed
//! `FrameError`. Budget: 256 draws, 64 KiB arena, no heap once warm.

use super::frame_alloc::{FrameAllocError, FrameArena};
use crate::draw::MAX_DRAWS;
use std::error::Error;
use std::fmt;

/// Bytes of one encoded draw record: id, mesh, depth, lod, pad.
pub const DRAW_RECORD_BYTES: usize = 24;

/// Bytes of one encoded uniform block: frame index, size, flags.
pub const UNIFORM_BYTES: usize = 20;

/// One frame draw as plain data; ordering never follows push order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameDraw {
    /// Stable node identity; secondary sort key.
    pub id: u64,
    /// Stable mesh identity the submit path resolves.
    pub mesh: u64,
    /// Camera distance in millimeters, saturated; primary sort key.
    pub depth_bucket: u32,
    /// LOD level picked with hysteresis.
    pub lod: u8,
}

/// One frame uniform block as plain data; copied as bits, never cast.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameUniforms {
    /// Monotonic frame index for jitter and history lookup.
    pub frame_index: u64,
    /// Target width in pixels.
    pub width: u32,
    /// Target height in pixels.
    pub height: u32,
    /// Bit flags the submit path reads (bit 0: look probes on).
    pub flags: u32,
}

/// Loud frame failures; every variant names the budget it hit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameError {
    /// More draws than the shared cap; split the scene or raise it.
    OverDrawCap {
        /// Draws pushed when the refusal fired.
        count: usize,
        /// Shared per-frame draw cap.
        cap: usize,
    },
    /// Arena encode ran past backing; warm bigger or push less.
    Arena(FrameAllocError),
}

impl fmt::Display for FrameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OverDrawCap { count, cap } => {
                write!(f, "frame draw cap exceeded: {count} draws, cap {cap}")
            }
            Self::Arena(inner) => write!(f, "frame arena: {inner}"),
        }
    }
}

impl Error for FrameError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::OverDrawCap { .. } => None,
            Self::Arena(inner) => Some(inner),
        }
    }
}

/// Reused per-frame state: a warm draw buffer plus one bump arena.
///
/// Steady frames clear both in place and refill them, so the counting
/// allocator reads zero calls. The draw cap is the shared
/// `crate::draw::MAX_DRAWS`; the two never drift apart.
#[derive(Clone, Debug, Default)]
pub struct Frame {
    draws: Vec<FrameDraw>,
    arena: FrameArena,
}

impl Frame {
    /// Empty frame with no allocation; warm before the first frame.
    #[must_use]
    pub fn new() -> Self {
        Self {
            draws: Vec::new(),
            arena: FrameArena::new(),
        }
    }

    /// Frame warmed for `draws` draws and `arena_bytes` backing. Cold
    /// path, may allocate once; refuses past the shared cap loudly.
    pub fn with_capacity(draws: usize, arena_bytes: usize) -> Result<Self, FrameError> {
        if draws > MAX_DRAWS {
            return Err(FrameError::OverDrawCap {
                count: draws,
                cap: MAX_DRAWS,
            });
        }
        let mut frame = Self::new();
        frame.draws.reserve(draws);
        frame.arena.warm(arena_bytes).map_err(FrameError::Arena)?;
        Ok(frame)
    }

    /// Start a frame: the draw list clears and the arena rewinds.
    pub fn begin_frame(&mut self) {
        self.draws.clear();
        self.arena.begin_frame();
    }

    /// Visible draws in integer-stable order after `sort_draws`.
    #[must_use]
    pub fn draws(&self) -> &[FrameDraw] {
        &self.draws
    }

    /// Reserved draw slots; steady frames never move it.
    #[must_use]
    pub fn draw_capacity(&self) -> usize {
        self.draws.capacity()
    }

    /// Arena bytes used this frame.
    #[must_use]
    pub fn arena_used(&self) -> usize {
        self.arena.used()
    }

    /// Arena backing bytes behind this frame.
    #[must_use]
    pub fn arena_capacity(&self) -> usize {
        self.arena.capacity()
    }

    /// This frame's scratch bytes, in encode order.
    #[must_use]
    pub fn arena_bytes(&self) -> &[u8] {
        self.arena.bytes()
    }

    /// Digest over draws in output order; equal bytes twice prove the
    /// order is bit-stable.
    #[must_use]
    pub fn order_digest(&self) -> u64 {
        let mut hash = engine_core::Fnv64::new();
        for draw in &self.draws {
            hash.write_u64(draw.id);
            hash.write_u64(draw.mesh);
            hash.write_u32(draw.depth_bucket);
            hash.write_u8(draw.lod);
        }
        hash.finish()
    }

    /// Digest over this frame's scratch bytes.
    #[must_use]
    pub fn arena_digest(&self) -> u64 {
        self.arena.digest()
    }

    /// Push one draw; refuses past the shared cap without truncating.
    pub fn push_draw(&mut self, draw: FrameDraw) -> Result<(), FrameError> {
        if self.draws.len() >= MAX_DRAWS {
            return Err(FrameError::OverDrawCap {
                count: self.draws.len(),
                cap: MAX_DRAWS,
            });
        }
        self.draws.push(draw);
        Ok(())
    }

    /// Integer-stable sort by `(depth_bucket, id)`; in place, no heap.
    pub fn sort_draws(&mut self) {
        self.draws.sort_unstable_by_key(|d| (d.depth_bucket, d.id));
    }

    /// Encode uniforms into scratch; returns the block offset. The
    /// block starts 4-aligned; fields follow as little-endian bytes.
    pub fn encode_uniforms(&mut self, uniforms: &FrameUniforms) -> Result<usize, FrameError> {
        let off = self.arena.alloc_bytes(0, 4).map_err(FrameError::Arena)?;
        self.arena
            .push_u64_le(uniforms.frame_index)
            .map_err(FrameError::Arena)?;
        self.arena
            .push_u32_le(uniforms.width)
            .map_err(FrameError::Arena)?;
        self.arena
            .push_u32_le(uniforms.height)
            .map_err(FrameError::Arena)?;
        self.arena
            .push_u32_le(uniforms.flags)
            .map_err(FrameError::Arena)?;
        Ok(off)
    }

    /// Encode every draw as fixed records; returns offset and length.
    /// Call after `sort_draws` so bytes follow output order. Records
    /// start 8-aligned; each is id, mesh, depth, lod plus pad.
    pub fn encode_draws(&mut self) -> Result<(usize, usize), FrameError> {
        let total = self.draws.len().saturating_mul(DRAW_RECORD_BYTES);
        let off = self.arena.alloc_bytes(0, 8).map_err(FrameError::Arena)?;
        for draw in &self.draws {
            self.arena.push_u64_le(draw.id).map_err(FrameError::Arena)?;
            self.arena
                .push_u64_le(draw.mesh)
                .map_err(FrameError::Arena)?;
            self.arena
                .push_u32_le(draw.depth_bucket)
                .map_err(FrameError::Arena)?;
            self.arena
                .push_bytes(&[draw.lod, 0, 0, 0])
                .map_err(FrameError::Arena)?;
        }
        Ok((off, total))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_core::alloc_probe::measure;

    fn draw(id: u64, depth: u32) -> FrameDraw {
        FrameDraw {
            id,
            mesh: id.wrapping_add(1000),
            depth_bucket: depth,
            lod: (id % 4) as u8,
        }
    }

    fn uniforms(index: u64) -> FrameUniforms {
        FrameUniforms {
            frame_index: index,
            width: 64,
            height: 64,
            flags: 1,
        }
    }

    #[test]
    fn steady_120_frames_read_zero_allocs() {
        let mut frame = Frame::with_capacity(64, 8192).expect("warms");
        frame.begin_frame();
        for id in 0..48u64 {
            frame
                .push_draw(draw(id, 1000 - (id as u32 % 17)))
                .expect("fits");
        }
        frame.sort_draws();
        frame.encode_uniforms(&uniforms(0)).expect("fits");
        frame.encode_draws().expect("fits");
        let warmed_draw_cap = frame.draw_capacity();
        let warmed_arena_cap = frame.arena_capacity();
        let warmed_digest = frame.order_digest();
        let (_, count) = measure(|| {
            for index in 0..120u64 {
                frame.begin_frame();
                for id in 0..48u64 {
                    frame
                        .push_draw(draw(id, 1000 - (id as u32 % 17)))
                        .expect("steady fits");
                }
                frame.sort_draws();
                frame.encode_uniforms(&uniforms(index)).expect("fits");
                frame.encode_draws().expect("fits");
                assert_eq!(frame.order_digest(), warmed_digest);
            }
        });
        assert_eq!(count.calls, 0, "steady frames must not allocate");
        assert_eq!(frame.draw_capacity(), warmed_draw_cap);
        assert_eq!(frame.arena_capacity(), warmed_arena_cap);
    }

    #[test]
    fn over_cap_refuses_loudly_and_keeps_the_list() {
        let mut frame = Frame::with_capacity(MAX_DRAWS, 8192).expect("warms");
        for id in 0..MAX_DRAWS as u64 {
            frame.push_draw(draw(id, id as u32)).expect("cap fits");
        }
        let err = frame
            .push_draw(draw(9999, 1))
            .expect_err("past cap refuses");
        assert_eq!(
            err,
            FrameError::OverDrawCap {
                count: MAX_DRAWS,
                cap: MAX_DRAWS,
            }
        );
        assert_eq!(frame.draws().len(), MAX_DRAWS);
    }

    #[test]
    fn order_survives_input_permutation() {
        let build = |shift: usize| {
            let mut frame = Frame::with_capacity(64, 8192).expect("warms");
            let mut ids: Vec<u64> = (0..48u64).collect();
            let len = ids.len().max(1);
            ids.rotate_left(shift % len);
            for id in ids {
                frame
                    .push_draw(draw(id, 500 + (id as u32 % 9)))
                    .expect("fits");
            }
            frame.sort_draws();
            let order: Vec<(u32, u64)> = frame
                .draws()
                .iter()
                .map(|d| (d.depth_bucket, d.id))
                .collect();
            (order, frame.order_digest())
        };
        let (first_order, first_digest) = build(0);
        let mut sorted = first_order.clone();
        sorted.sort_unstable();
        assert_eq!(first_order, sorted, "keys must ascend");
        for shift in [7, 23, 41] {
            let (order, digest) = build(shift);
            assert_eq!(order, first_order, "order moved on shift {shift}");
            assert_eq!(digest, first_digest);
        }
    }

    #[test]
    fn tiny_arena_surfaces_a_frame_error() {
        let mut frame = Frame::with_capacity(8, 64).expect("warms");
        for id in 0..8u64 {
            frame.push_draw(draw(id, id as u32)).expect("fits");
        }
        frame.sort_draws();
        let err = frame.encode_draws().expect_err("8 records need 192 bytes");
        assert!(matches!(err, FrameError::Arena(_)));
    }

    #[test]
    fn uniform_bits_round_trip() {
        let mut frame = Frame::with_capacity(4, 256).expect("warms");
        let off = frame.encode_uniforms(&uniforms(41)).expect("fits");
        let bytes = frame.arena_bytes();
        assert_eq!(off, 0);
        assert_eq!(bytes.len(), UNIFORM_BYTES);
        let mut index_bytes = [0u8; 8];
        index_bytes.copy_from_slice(&bytes[..8]);
        assert_eq!(u64::from_le_bytes(index_bytes), 41);
        let mut width_bytes = [0u8; 4];
        width_bytes.copy_from_slice(&bytes[8..12]);
        assert_eq!(u32::from_le_bytes(width_bytes), 64);
    }

    #[test]
    fn draw_records_follow_output_order() {
        let mut frame = Frame::with_capacity(4, 512).expect("warms");
        frame.push_draw(draw(9, 30)).expect("fits");
        frame.push_draw(draw(3, 30)).expect("fits");
        frame.push_draw(draw(5, 5)).expect("fits");
        frame.sort_draws();
        let (off, len) = frame.encode_draws().expect("fits");
        assert_eq!((off, len), (0, 3 * DRAW_RECORD_BYTES));
        let bytes = frame.arena_bytes();
        assert_eq!(bytes.len(), 3 * DRAW_RECORD_BYTES);
        let mut first_id = [0u8; 8];
        first_id.copy_from_slice(&bytes[..8]);
        assert_eq!(u64::from_le_bytes(first_id), 5);
        let mut first_mesh = [0u8; 8];
        first_mesh.copy_from_slice(&bytes[8..16]);
        assert_eq!(u64::from_le_bytes(first_mesh), 1005);
    }

    #[test]
    fn error_display_names_the_fix() {
        let cap = FrameError::OverDrawCap {
            count: 300,
            cap: MAX_DRAWS,
        };
        assert!(format!("{cap}").contains("draw cap"));
        let arena = FrameError::Arena(FrameAllocError::OverBudget {
            requested: 8,
            used: 64,
            cap: 64,
        });
        assert!(format!("{arena}").contains("arena"));
    }
}
