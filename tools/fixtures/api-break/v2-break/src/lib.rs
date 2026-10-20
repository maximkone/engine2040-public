//! Seeded breaking change for the api-break gate: three edits a caller sees.
//!
//! 1. `add` is gone (a removed public function).
//! 2. `Counter::step` takes an argument (a changed signature).
//! 3. `Fault::Range` names its field `max`, not `limit` (a changed variant).

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

    /// Steps the counter by `by`.
    pub fn step(&mut self, by: u32) {
        self.value = self.value.wrapping_add(by);
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
        max: u32,
    },
}
