//! digest: shared fold for canonical words beside the full rehash.
//! Owner: engine-core (std-only). Units: hashes (u64). LE words, index order, no alloc.
//! Reserved for RC9/RC22 incremental digest experiment; no live caller yet.

use crate::hash::{Fnv64, FNV64_PRIME};

/// Twin FNV-1a fold, one index-ordered pass over two streams.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TwinFold {
    a: Fnv64,
    b: Fnv64,
}

impl TwinFold {
    /// Fresh fold at the FNV offset basis for both streams.
    pub const fn new() -> Self {
        Self {
            a: Fnv64::new(),
            b: Fnv64::new(),
        }
    }
    /// Feed one stream-A word, preserving stream order.
    pub fn push_a_u32(&mut self, word: u32) {
        self.a.write_u32(word);
    }
    /// Feed one stream-B word, preserving stream order.
    pub fn push_b_u32(&mut self, word: u32) {
        self.b.write_u32(word);
    }
    /// Two-pass combine value, bit for bit.
    #[must_use]
    pub fn finish(&self) -> u64 {
        self.a.finish() ^ self.b.finish().wrapping_mul(FNV64_PRIME)
    }
}
