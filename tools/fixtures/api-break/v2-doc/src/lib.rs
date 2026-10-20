//! The "released" API of the api-break gate fixture, with the prose rewritten.
//!
//! Only doc text and code comments changed: every public path, signature,
//! field and variant is byte-for-byte the same shape as `v1`.

/// Adds both numbers, wrapping on overflow.
///
/// A caller sees the same `(u32, u32) -> u32` as before.
pub fn add(a: u32, b: u32) -> u32 {
    // wrapping keeps the fixture overflow-free in debug builds too
    a.wrapping_add(b)
}

/// A counter of completed steps.
///
/// The field stays public and typed `u32`.
pub struct Counter {
    /// The current count.
    pub value: u32,
}

impl Counter {
    /// Fresh counter at zero.
    ///
    /// No allocation, no clocks, nothing to configure.
    pub fn new() -> Self {
        Self { value: 0 }
    }

    /// Steps the counter by one.
    ///
    /// Wraps at the `u32` ceiling instead of panicking.
    pub fn step(&mut self) {
        self.value = self.value.wrapping_add(1);
    }

    /// Reads the count.
    pub fn get(&self) -> u32 {
        self.value
    }
}

/// Failure of a check.
pub enum Fault {
    /// Nothing to do.
    Empty,
    /// Out of range.
    Range {
        /// Upper bound.
        limit: u32,
    },
}
