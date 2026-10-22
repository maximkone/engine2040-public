//! One scene -> spatial adapter: the only place that decides how big a node is.
//!
//! Consumers: the CAP1 staged gate ([`crate::capability`]), the findings panel
//! ([`crate::findings`]), and every endpoint read that measures a scene
//! (`scene.brief`, `scene.spatial`, `scene.place`, `scene.query`, intents, the
//! assistant and `level.build`), plus the play path's blocking boxes
//! (`play/world_load.rs::blocks`, which keeps the arena height filter).
//! Producer of data: the authoring [`Document`] (`hierarchy` plus the
//! `components` table). Output: one row per node, world pose plus full world
//! extents in meters. Units: meters, radians.
//!
//! One rule, in [`row_of`]: a node with a `Collider` component is its real
//! shape times the node's world scale ([`SizeBasis::Collider`]); a node with no
//! collider falls back to its world scale ([`SizeBasis::Default`], the unit
//! cube convention: a unit-scale node is a 1 m cube). A default-sized node is
//! never silently believed: every caller can read [`NodeRow::basis`], and the
//! reported rows say so.
//!
//! Determinism: rows visit in id-text order (byte order equals id text order:
//! hyphenated lowercase hex), so the same document always maps to the same
//! actor ids and the same bytes. Pure: no clock, no IO, no `f32` trig in the
//! tick.

use engine_scene::component::{ColliderShape, Component, ComponentKind};
use engine_scene::ids::ObjectId;
use engine_scene::transform::{Hierarchy, Transform};
use engine_spatial::Actor;

use crate::commands::{ComponentTable, Document};

/// Edge of the fallback box for a unit-scale node with no collider, in meters.
pub const DEFAULT_SIZE_M: f32 = 1.0;

/// Where a node's box size came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SizeBasis {
    /// Measured from the node's `Collider` component, times its world scale.
    Collider,
    /// No collider: the node's world scale stands in. A unit-scale node is
    /// [`DEFAULT_SIZE_M`]; nothing about this box was measured.
    Default,
}

impl SizeBasis {
    /// Short stable word for reports: `collider` or `default`.
    pub fn word(self) -> &'static str {
        match self {
            SizeBasis::Collider => "collider",
            SizeBasis::Default => "default",
        }
    }

    /// True when the box is the fallback, not a measurement.
    pub fn is_default(self) -> bool {
        self == SizeBasis::Default
    }
}

/// Whether the play path blocks fighters on a row, and why not.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Solid {
    /// Blocks fighters: a real collider, or a bare node (pose only).
    Blocks,
    /// A `sensor` collider reports overlaps without pushing.
    Sensor,
    /// Carries components but no collider: decoration, never blocks.
    Decoration,
}

/// One node mapped to a spatial row.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NodeRow {
    /// Persistent node id.
    pub id: ObjectId,
    /// World pose (position, rotation, scale), all lanes finite.
    pub world: Transform,
    /// Full world extents in meters `[x, y, z]`, always finite and positive.
    pub size: [f32; 3],
    /// Where [`Self::size`] came from.
    pub basis: SizeBasis,
    /// Whether the play path blocks on this row.
    pub solid: Solid,
}

/// The node table of one document: rows in id-text order plus the ids whose
/// world pose is missing or non-finite (they get no row; the caller posts a
/// NaN actor if its report needs one).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SceneRows {
    /// One row per node with a finite world pose, id-text order.
    pub rows: Vec<NodeRow>,
    /// Nodes without a usable world pose, id-text order.
    pub missing: Vec<ObjectId>,
}

/// Counts by size basis, index-aligned with the rows they came from.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SizeCounts {
    /// Rows sized from a collider component.
    pub collider: u64,
    /// Rows posted as the fallback box.
    pub default: u64,
}

impl SizeCounts {
    /// Count the bases of one row list.
    pub fn of(rows: &[NodeRow]) -> Self {
        let mut out = Self::default();
        for row in rows {
            match row.basis {
                SizeBasis::Collider => out.collider += 1,
                SizeBasis::Default => out.default += 1,
            }
        }
        out
    }
}

/// The row of one node, or `None` when it has no finite world pose.
pub fn row_of(tree: &Hierarchy, components: &ComponentTable, id: ObjectId) -> Option<NodeRow> {
    let world = tree.world(id).filter(|w| {
        w.position
            .iter()
            .chain(w.rotation.iter())
            .all(|v| v.is_finite())
    })?;
    let (size, basis, solid) = box_of(components, id, world.scale);
    Some(NodeRow {
        id,
        world,
        size,
        basis,
        solid,
    })
}

/// Full world extents in meters plus where they came from and whether the play
/// path blocks on them. Scale is taken by magnitude: a negative scale mirrors
/// the shape without shrinking it.
fn box_of(
    components: &ComponentTable,
    id: ObjectId,
    scale: [f32; 3],
) -> ([f32; 3], SizeBasis, Solid) {
    let slot = components.get(&id.to_string());
    let extent = [
        clean(scale[0]).abs(),
        clean(scale[1]).abs(),
        clean(scale[2]).abs(),
    ];
    let Some(Component::Collider(collider)) =
        slot.and_then(|kinds| kinds.get(&ComponentKind::Collider))
    else {
        // No collider at all is a bare node (the unit cube convention); a node
        // with other components is decoration that never blocks.
        let (size, solid) = match slot {
            Some(kinds) if !kinds.is_empty() => (extent, Solid::Decoration),
            _ => (extent, Solid::Blocks),
        };
        return (size, SizeBasis::Default, solid);
    };
    let [a, b, c] = collider.half;
    let own = match collider.shape {
        ColliderShape::Box => [2.0 * a, 2.0 * b, 2.0 * c],
        ColliderShape::Sphere => [2.0 * a, 2.0 * a, 2.0 * a],
        ColliderShape::Capsule => [2.0 * a, 2.0 * (a + b), 2.0 * a],
    };
    let size = [
        clean(own[0].abs() * extent[0]),
        clean(own[1].abs() * extent[1]),
        clean(own[2].abs() * extent[2]),
    ];
    let solid = if collider.sensor {
        Solid::Sensor
    } else {
        Solid::Blocks
    };
    (size, SizeBasis::Collider, solid)
}

/// A scale lane that is finite and positive, else 1 (a degenerate scale never
/// turns a box into NaN or a negative hole).
fn clean(value: f32) -> f32 {
    if value.is_finite() && value != 0.0 {
        value
    } else {
        DEFAULT_SIZE_M
    }
}

/// Every reachable node of a document, mapped once.
pub fn rows_of(doc: &Document) -> SceneRows {
    rows(&doc.hierarchy, &doc.components)
}

/// Every reachable node of a tree plus component table, mapped once.
pub fn rows(tree: &Hierarchy, components: &ComponentTable) -> SceneRows {
    let mut order = tree.propagation_order();
    order.sort_by_key(|id| id.to_string());
    let mut out = SceneRows::default();
    for id in order {
        match row_of(tree, components, id) {
            Some(row) => out.rows.push(row),
            None => out.missing.push(id),
        }
    }
    out
}

/// Every reachable node of a document as one spatial actor: id is the row
/// index, name is `prop_<index>` (grammar-clean), parent links stay unset
/// because the adapter reports world poses only.
pub fn actors(doc: &Document) -> (Vec<Actor>, Vec<ObjectId>) {
    actors_of(&rows_of(doc).rows)
}

/// Rows as spatial actors, id = index, name = `prop_<index>`.
pub fn actors_of(rows: &[NodeRow]) -> (Vec<Actor>, Vec<ObjectId>) {
    let mut ids = Vec::with_capacity(rows.len());
    let mut actors = Vec::with_capacity(rows.len());
    for (index, row) in rows.iter().enumerate() {
        let Ok(id) = u32::try_from(index) else {
            break;
        };
        ids.push(row.id);
        actors.push(actor(row, id, format!("prop_{id:04}")));
    }
    (actors, ids)
}

/// One row as a spatial actor with a caller-chosen actor id and name.
pub fn actor(row: &NodeRow, id: u32, name: String) -> Actor {
    Actor {
        id,
        name,
        x: row.world.position[0],
        y: row.world.position[1],
        z: row.world.position[2],
        sx: row.size[0],
        sy: row.size[1],
        sz: row.size[2],
        parent: None,
        physical: true,
        vx: 0.0,
        vz: 0.0,
        yaw: yaw_of(row.world.rotation),
    }
}

/// The block a row gives the match world: XZ footprint plus yaw axis, or
/// `None` when the play path does not block on it (sensor or decoration).
/// The arena's height filter is the caller's (`play/world_load.rs`).
pub fn block_of(row: &NodeRow) -> Option<([f32; 2], [f32; 2])> {
    if row.solid != Solid::Blocks {
        return None;
    }
    let [qx, qy, qz, qw] = row.world.rotation;
    // Yaw axis from the quaternion without trigonometry:
    // cos ~ 1 - 2(y^2 + z^2), sin ~ 2(w y + x z).
    let cos = 1.0 - 2.0 * (qy * qy + qz * qz);
    let sin = 2.0 * (qw * qy + qx * qz);
    let axis = if cos == 0.0 && sin == 0.0 {
        [1.0, 0.0]
    } else {
        [cos, sin]
    };
    Some(([row.size[0], row.size[2]], axis))
}

/// Y yaw in radians from a world rotation quaternion. The quaternion is
/// assumed unit length (the scene validates it); a zero quaternion reads 0.
pub fn yaw_of(rotation: [f32; 4]) -> f32 {
    let [x, y, z, w] = rotation;
    if !rotation.iter().all(|v| v.is_finite()) {
        return 0.0;
    }
    (2.0 * (w * y + x * z)).atan2(1.0 - 2.0 * (y * y + z * z))
}

/// One line naming a default-sized node: label plus box, so a reader can tell
/// an assumed size from a measured one.
pub fn default_row(label: &str, size: [f32; 3]) -> String {
    format!(
        "{label} {:.2}x{:.2}x{:.2} m: no collider, the box is the node scale \
         ({})",
        size[0],
        size[1],
        size[2],
        SizeBasis::Default.word()
    )
}

#[cfg(test)]
mod tests;
