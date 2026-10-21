//! groups: the collision groups of every body, taken from the engine's layer table.
//!
//! The table in `engine_physics::layers` owns the group bits and the accept masks.
//! This crate only says which layer each of its bodies is: the floor and the walls are
//! the world, a crate and a thrown object are props, the bones of a fighter and the
//! limbs that came off are the body, and the pieces of a broken wall are debris.
//!
//! The groups are Rapier's `InteractionGroups` in `And` mode: two bodies meet only when
//! each one accepts the other's layer. That is the table's own pair rule, the same one
//! the Bullet proxy pair rule uses, so the float world and the integer world agree on
//! who may touch whom.

use engine_physics::layers as table;
use rapier3d::geometry::{Group, InteractionGroups, InteractionTestMode};

/// The groups of one table layer: the group bit the body is in and the mask it accepts.
/// A layer the table does not know has an empty mask, so such a body meets nothing.
pub(crate) fn of(layer: u32) -> InteractionGroups {
    InteractionGroups::new(
        Group::from_bits_retain(layer),
        Group::from_bits_retain(table::default_mask(layer)),
        InteractionTestMode::And,
    )
}
