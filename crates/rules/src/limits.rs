//! Sandbox limits. They are part of the replay contract: a different budget can change a
//! tick's outcome, so the limits feed the ABI id written in every tape header.

use engine_core::Fnv64;

/// Actors one world may hold (the net server's player cap).
pub const MAX_FIGHTERS: usize = 64;
/// Integer memory slots each fighter's rules own.
pub const MEM_SLOTS: usize = 8;

/// Hard limits for one rules engine.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// Operations one `on_tick` call may spend (Rhai counts expression steps).
    pub max_ops: u64,
    /// Nested script function calls.
    pub max_call_levels: usize,
    /// Expression nesting at top level.
    pub max_expr_depth: usize,
    /// Expression nesting inside functions.
    pub max_fn_expr_depth: usize,
    /// Source bytes accepted by `set` and `test`.
    pub max_source_bytes: usize,
    /// Host actions one `on_tick` call may emit.
    pub max_actions_per_call: usize,
    /// Live variables in one call.
    pub max_variables: usize,
    /// Script functions in one script.
    pub max_functions: usize,
}

impl Limits {
    /// The shipped limits: a shield bash costs about 40 operations, a budget of 2,000 leaves
    /// room for real rules and still stops a runaway loop in microseconds.
    pub const DEFAULT: Self = Self {
        max_ops: 2_000,
        max_call_levels: 8,
        max_expr_depth: 32,
        max_fn_expr_depth: 16,
        max_source_bytes: 16_384,
        max_actions_per_call: 8,
        max_variables: 32,
        max_functions: 16,
    };

    /// Fold every limit into a hasher (field order is frozen).
    pub fn write_into(&self, hasher: &mut Fnv64) {
        hasher.write_u64(self.max_ops);
        for value in [
            self.max_call_levels,
            self.max_expr_depth,
            self.max_fn_expr_depth,
            self.max_source_bytes,
            self.max_actions_per_call,
            self.max_variables,
            self.max_functions,
        ] {
            hasher.write_u64(value as u64);
        }
    }
}

impl Default for Limits {
    fn default() -> Self {
        Self::DEFAULT
    }
}
