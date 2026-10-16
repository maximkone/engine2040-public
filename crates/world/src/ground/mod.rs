//! ground: staged courtyard bake beside the frozen floor.
//! Owner: `engine-world`; bake-time tables only, never the tick.
//! Units: texels in, tile and prop tables out.
//! Determinism: integer passes in base, wear, scatter order.
//! Errors: typed [`crate::BakeErr`]; parse refusals are `None`.
//! Budget: bakes complete under 100 ms; no per-tick work.
//! Caller: [`bake::bake_ground_bytes`] calls [`bake::bake_ground`].
/// Staged courtyard bake passes (base, wear, scatter).
pub mod bake;
