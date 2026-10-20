//! The "released" API of the api-break gate fixture.

/// Adds two numbers.
pub fn add(a: u32, b: u32) -> u32 {
    a.wrapping_add(b)
}

/// A counter.
pub struct Counter {
    /// Current value.
    pub value: u32,
}

impl Counter {
    /// Fresh counter at zero.
    pub fn new() -> Self {
        Self { value: 0 }
    }

    /// Steps the counter.
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
