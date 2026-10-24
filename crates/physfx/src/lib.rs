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
//! - Every collider this crate builds carries the group and the mask of the collision layer
//!   table (`engine_physics::layers`): the floor and the walls are the world, a crate and a
//!   thrown object are props, the bones of a fighter and the limbs that came off are the
//!   body, and the pieces of a broken wall are debris. Two bodies meet only when each one
//!   accepts the other's layer, so a ragdoll rests on a crate and debris never meets cloth.
//! - A `Blast` (integers, as the sim holds it) shoves every body inside its radius straight
//!   away from the center, weaker with distance. `spawn_thrown` flies an arrow or a thrown
//!   object along the drag-free arc that reaches the authority's hit point on the hit tick.
//!   An `Impact` (the hit itself, integers) shoves the bodies around the hit point along the
//!   projectile's heading with a real impulse, so heavy bodies move less. Blasts and impacts
//!   shove only the nearest `Caps::blast_bodies` and `Caps::impact_bodies` bodies of the tier.
//! - A spear tether is a rope: `attach_tether` ties a verlet chain between two points (the
//!   thrower and the target, meters), `move_tether` gives its ends every tick, `tether_nodes`
//!   reads the chain back and `tether_info` says slack, taut or loose and how hard it pulls (a
//!   spring on the stretch plus a damper). `carry_ragdoll` drags a ragdoll's root along the
//!   sim's own track with a soft spring-damper, so the limbs trail and swing. Rope length and
//!   count come from the tier's caps (`Caps::tethers`, `Caps::tether_nodes`).
//! - A ragdoll spawn measures itself: `PhysFx::ragdoll_receipt` has the mass of every bone, the
//!   child over parent mass ratio of every joint and the unjoined bone pairs that overlapped in
//!   the standing pose. Grandparent pairs among them have their contacts switched off so the
//!   solver does not blast them apart. A rig names its knees and elbows with `Rig::with_hinges`;
//!   those joints bend about the pelvis side axis inside a flexion range, the rest are balls.
//! - `PhysFx::hit_ragdoll` spreads a blow over the bones near it, its total unchanged, and makes
//!   the joints brace: a PD motor holds each joint at its spawn angle with a strength that falls
//!   from 1 to 0 over `HIT_FADE_S`.
//! - `PhysFx::sever` takes one bone out of a ragdoll into a `Limb` group of its own, removes the
//!   joint to its parent and leaves the chain below jointed to it, so the limb falls away while
//!   the ragdoll keeps the rest of its bones. The cut is authority; this only paints it.
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
mod body;
#[cfg(feature = "rapier")]
mod caps;
#[cfg(feature = "rapier")]
mod check;
#[cfg(feature = "rapier")]
mod error;
#[cfg(feature = "rapier")]
mod fx;
#[cfg(feature = "rapier")]
mod groups;
#[cfg(feature = "rapier")]
mod rig;
#[cfg(feature = "rapier")]
mod soft;
#[cfg(feature = "rapier")]
mod terrain;
#[cfg(feature = "rapier")]
mod tether;
#[cfg(feature = "rapier")]
mod types;
#[cfg(feature = "rapier")]
mod verbs;

#[cfg(feature = "rapier")]
pub use body::{
    load as load_body, parse as parse_body, BodyDesc, BodyError, BodyFileError, BoneRow, CutRow,
    GORE_FULL, MUSCLE_FULL,
};
#[cfg(feature = "rapier")]
pub use caps::{Caps, Tier};
/// Largest blast momentum the physical layer accepts, newton-seconds. The game's loudest blast
/// cue is this same ceiling.
#[cfg(feature = "rapier")]
pub use check::MAX_BLAST_NS;
/// The collision layer table every collider of this crate takes its group and mask from.
#[cfg(feature = "rapier")]
pub use engine_physics::layers;
#[cfg(feature = "rapier")]
pub use engine_physics::material::Material;
#[cfg(feature = "rapier")]
pub use engine_physics::ragdoll::BoneShape;
#[cfg(feature = "rapier")]
pub use error::{Error, Field};
#[cfg(feature = "rapier")]
pub use fx::{PhysFx, RagdollReceipt, Severed, TetherInfo, HIT_FADE_S};
#[cfg(feature = "rapier")]
pub use rig::{Hinge, Rig};
#[cfg(feature = "rapier")]
pub use soft::{ClothDesc, ClothError, SoftCloth, SoftReceipt, SoftWorld, MAX_CLOTHS};
#[cfg(feature = "rapier")]
pub use terrain::{
    HeightTerrain, ShapeParams, TerrainError, TerrainShape, MAX_SAMPLES, TERRAIN_FRICTION,
};
#[cfg(feature = "rapier")]
pub use tether::TetherState;
#[cfg(feature = "rapier")]
pub use types::{Batch, Blast, Flight, Impact, Key, Placed, Shard, Stats, Transform};
#[cfg(feature = "rapier")]
pub use verbs::{
    fx_blast, fx_break, fx_cloth, fx_limb, fx_ragdoll, BlastEvent, BlastFx, BreakEvent, BreakFx,
    ClothEvent, ClothFx, LimbEvent, LimbFx, RagdollEvent, RagdollFx,
};

#[cfg(all(test, feature = "rapier"))]
mod tests;
