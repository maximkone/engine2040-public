//! physfx: the physical layer, a float Rapier world seeded by authority events, never hashed.
//!
//! Two physics layers keep the game honest (docs/plan/web-and-realism.md, section 2). The
//! authority layer (integers, hashed, in arena and combat) decides who fell, what was cut and
//! which bond broke. This crate is the other one: it only has to look and feel heavy. A client
//! feeds it plain events (a pose and a velocity for a fallen fighter, a transform for a severed
//! limb, a list of shards for a broken wall) and reads transforms back. It never calls back into
//! the sim, and the sim never reads a float from it.
//!
//! Intended consumers: `crates/game` only, through RZ2 (ragdoll on knockout), RZ3 (limbs that
//! come off) and RZ4 (walls and crates that break). `engine-arena`, `engine-combat` and
//! `engine-proof` must never depend on it; the `sim_crates_never_name_physfx` test and the
//! dependency edges in `tools/workspace-check.mjs` hold that line.
//!
//! The rules of the world:
//! - Units are meters, seconds and Y-up; rotations are unit quaternions `[x, y, z, w]`;
//!   impulses are newton-seconds.
//! - One `PhysFx::step` is one tick at the fixed 60 Hz (`engine_physics::rapier::FIXED_DT`).
//!   The caller steps once per sim tick and passes its render alpha to `PhysFx::transforms`,
//!   which blends the last two ticks, so a 144 Hz window does not stutter.
//! - Hard caps come from one table (`Tier::caps`): bodies, ragdolls, contacts, statics and the
//!   solver budget. When a spawn does not fit, the OLDEST body (a whole ragdoll counts as one
//!   unit) is recycled first. Contacts above the cap shed the oldest limbs and debris. The
//!   same ladder carries the cost bar of each tier (`Tier::tick_bar_us`), and
//!   `Stats::receipt_line` is the one line that prints bodies, contacts and step cost.
//! - What a body is made of (density, grip, bounce, drag) comes from the one table in
//!   `engine_physics::material`: bones are flesh, `spawn_debris` is stone, and the `_of`
//!   spawns take any `Material`.
//! - A `Blast` (integers, as the sim holds it) shoves every body inside its radius straight
//!   away from the center, weaker with distance. `spawn_thrown` flies an arrow or a thrown
//!   object along the drag-free arc that reaches the authority's hit point on the hit tick.
//! - Bodies sleep. A sleeping body costs no solver work and reads back the same transform every
//!   frame.
//! - Bad input (NaN, infinity, out of range) is refused with a typed `Error` before anything
//!   changes: a refused spawn leaves the world exactly as it was.
//! - Determinism: the same inputs give the same transforms on one machine. Across machines they
//!   may differ by centimeters; that is allowed because nothing here is authority.
//! - The crate reads no clock of its own (a browser build has no `Instant`). Give it one with
//!   `PhysFx::with_clock` to fill `Stats::micros`.
//!
//! Everything sits behind the cargo feature `rapier`, off by default, so the headless default
//! graph stays std-only. Build and test with `--features rapier`.
#![forbid(unsafe_code)]

#[cfg(feature = "rapier")]
mod caps;
#[cfg(feature = "rapier")]
mod check;
#[cfg(feature = "rapier")]
mod error;
#[cfg(feature = "rapier")]
mod fx;
#[cfg(feature = "rapier")]
mod rig;
#[cfg(feature = "rapier")]
mod types;

#[cfg(feature = "rapier")]
pub use caps::{Caps, Tier};
#[cfg(feature = "rapier")]
pub use engine_physics::material::Material;
#[cfg(feature = "rapier")]
pub use engine_physics::ragdoll::BoneShape;
#[cfg(feature = "rapier")]
pub use error::{Error, Field};
#[cfg(feature = "rapier")]
pub use fx::PhysFx;
#[cfg(feature = "rapier")]
pub use rig::Rig;
#[cfg(feature = "rapier")]
pub use types::{Batch, Blast, Flight, Key, Placed, Shard, Stats, Transform};

#[cfg(all(test, feature = "rapier"))]
mod tests;
