//! HS23 Kani harnesses: 20 proofs over the four integer kernels.
//!
//! The parent modules are only compiled under Kani's `cfg(kani)`, so a
//! normal build and `cargo test` never see the `kani` crate. Run from the
//! workspace root:
//!
//! ```text
//! cargo kani -p engine-proof --lib --default-unwind 64
//! ```
//!
//! `engine-destruction` is a normal core-only dependency, so `--lib` needs no
//! test targets. Each harness names its kernel and bound in its doc comment.

#[path = "bond.rs"]
mod bond;
#[path = "cut.rs"]
mod cut;
#[path = "q16.rs"]
mod q16;
#[path = "tape.rs"]
mod tape;
