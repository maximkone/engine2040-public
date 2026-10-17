//! 3D transform hierarchy: parent links plus local and world poses.
//!
//! Coordinates follow the proposed engine convention: right-handed, +Y up,
//! meters, radians. Each node stores a parent [`ObjectId`] (nil is a root)
//! and a local [`Transform`]; the world pose composes locals root-first.
//!
//! Composition order is `M = T * R * S` (scale, then rotation, then
//! translation). Scale composes componentwise and shear is never
//! represented: exact for uniform or axis-aligned scales, an approximation
//! under a rotated, nonuniformly scaled ancestor. [`Transform::to_matrix`]
//! always reports the exact single-triple matrix.
//!
//! [`Hierarchy::reparent`] keeps the node's world pose and rejects
//! cycle-forming moves. Child listings and propagation order sort siblings
//! by [`ObjectId`], so traversal is deterministic for a given set of ids.

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};

use super::ids::ObjectId;
use engine_core::quat_mul;

/// Smallest absolute scale component that can still be inverted.
/// Below this, links keep their poses but reparenting across them fails.
pub const MIN_INVERTIBLE_SCALE: f32 = 1e-6;

/// Local (or world) pose: translation, rotation and scale.
///
/// Rotation is a unit quaternion stored as `[x, y, z, w]`. Composition
/// normalizes it, so slightly denormalized input stays usable.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Transform {
    /// Translation in meters, expressed in the parent frame.
    pub position: [f32; 3],
    /// Unit quaternion `[x, y, z, w]`, applied in the parent frame.
    pub rotation: [f32; 4],
    /// Scale factors per axis applied to children and geometry.
    pub scale: [f32; 3],
}

impl Transform {
    /// Pose with no translation, no rotation and unit scale.
    pub const IDENTITY: Self = Self {
        position: [0.0, 0.0, 0.0],
        rotation: [0.0, 0.0, 0.0, 1.0],
        scale: [1.0, 1.0, 1.0],
    };

    /// Build a pose from its parts. Rotation is normalized on use.
    pub fn new(position: [f32; 3], rotation: [f32; 4], scale: [f32; 3]) -> Self {
        Self {
            position,
            rotation,
            scale,
        }
    }

    /// Pose with no translation, no rotation and unit scale.
    pub fn identity() -> Self {
        Self::IDENTITY
    }

    /// True when every component is finite (no NaN or infinity).
    pub fn is_finite(self) -> bool {
        let all = [&self.position[..], &self.rotation[..], &self.scale[..]];
        all.iter().flat_map(|s| s.iter()).all(|c| c.is_finite())
    }

    /// Exact `T * R * S` matrix, row-major, column vectors: a local point
    /// maps to `m * p` with the translation in column 3 (`m[row][3]`).
    pub fn to_matrix(self) -> [[f32; 4]; 4] {
        let [x, y, z, w] = quat_normalized(self.rotation);
        let [sx, sy, sz] = self.scale;
        let [tx, ty, tz] = self.position;
        let r00 = 1.0 - 2.0 * (y * y + z * z);
        let r01 = 2.0 * (x * y - z * w);
        let r02 = 2.0 * (x * z + y * w);
        let r10 = 2.0 * (x * y + z * w);
        let r11 = 1.0 - 2.0 * (x * x + z * z);
        let r12 = 2.0 * (y * z - x * w);
        let r20 = 2.0 * (x * z - y * w);
        let r21 = 2.0 * (y * z + x * w);
        let r22 = 1.0 - 2.0 * (x * x + y * y);
        [
            [r00 * sx, r01 * sy, r02 * sz, tx],
            [r10 * sx, r11 * sy, r12 * sz, ty],
            [r20 * sx, r21 * sy, r22 * sz, tz],
            [0.0, 0.0, 0.0, 1.0],
        ]
    }
}

impl Default for Transform {
    fn default() -> Self {
        Self::identity()
    }
}

/// What went wrong with a hierarchy operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HierarchyError {
    /// The node id itself was nil; nil is the "no parent" slot, not a node.
    NilId,
    /// A node with this id already exists.
    Duplicate,
    /// No node has the requested id.
    UnknownNode,
    /// The requested parent id is not nil and names no known node.
    UnknownParent,
    /// The reparent would make the node its own ancestor.
    Cycle,
    /// A local pose held NaN or infinity.
    NonFinite,
    /// A scale on the path was too close to zero to invert.
    DegenerateScale,
}

impl fmt::Display for HierarchyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            Self::NilId => "node id must not be nil",
            Self::Duplicate => "node id already exists",
            Self::UnknownNode => "unknown node id",
            Self::UnknownParent => "unknown parent id",
            Self::Cycle => "reparent would create a cycle",
            Self::NonFinite => "transform holds NaN or infinity",
            Self::DegenerateScale => "scale too close to zero to invert",
        };
        f.write_str(text)
    }
}

impl std::error::Error for HierarchyError {}

/// One stored node: its parent link and its local pose.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
struct Node {
    /// Parent node; [`ObjectId::NIL`] means this node is a root.
    parent: ObjectId,
    /// Pose relative to the parent (or to the world for roots).
    local: Transform,
}

/// Parent links plus local poses for a set of [`ObjectId`] nodes.
///
/// Nodes carry no payload; links are the whole structure. Cycles cannot be
/// built through [`Hierarchy::insert`] and [`Hierarchy::reparent`]; storage
/// restored by hand should be checked with [`Hierarchy::validate`].
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Hierarchy {
    nodes: BTreeMap<ObjectId, Node>,
}

impl Hierarchy {
    /// Empty hierarchy.
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of nodes.
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// True when no nodes are stored.
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// True when a node with this id exists.
    pub fn contains(&self, id: ObjectId) -> bool {
        self.nodes.contains_key(&id)
    }

    /// Insert a node with a parent (`nil` for a root) and a local pose.
    ///
    /// A fresh id cannot close a cycle; duplicates, nil ids, unknown
    /// parents and non-finite poses fail here.
    pub fn insert(
        &mut self,
        id: ObjectId,
        parent: ObjectId,
        local: Transform,
    ) -> Result<(), HierarchyError> {
        if id.is_nil() {
            return Err(HierarchyError::NilId);
        }
        if self.nodes.contains_key(&id) {
            return Err(HierarchyError::Duplicate);
        }
        if !local.is_finite() {
            return Err(HierarchyError::NonFinite);
        }
        if !parent.is_nil() && !self.nodes.contains_key(&parent) {
            return Err(HierarchyError::UnknownParent);
        }
        self.nodes.insert(id, Node { parent, local });
        Ok(())
    }

    /// Replace a node's local pose.
    pub fn set_local(&mut self, id: ObjectId, local: Transform) -> Result<(), HierarchyError> {
        if !local.is_finite() {
            return Err(HierarchyError::NonFinite);
        }
        match self.nodes.get_mut(&id) {
            Some(node) => {
                node.local = local;
                Ok(())
            }
            None => Err(HierarchyError::UnknownNode),
        }
    }

    /// A node's local pose, if the node exists.
    pub fn local(&self, id: ObjectId) -> Option<Transform> {
        self.nodes.get(&id).map(|node| node.local)
    }

    /// A node's parent (`nil` for roots, `None` for unknown ids).
    pub fn parent(&self, id: ObjectId) -> Option<ObjectId> {
        self.nodes.get(&id).map(|node| node.parent)
    }

    /// Move a node to a new parent, recomputing its local pose so its world
    /// pose is unchanged. Nil detaches the node to a root, keeping its world
    /// pose as its new local pose.
    ///
    /// Moves onto the node's own id or a descendant fail with
    /// [`HierarchyError::Cycle`]; the same parent is a successful no-op.
    pub fn reparent(&mut self, id: ObjectId, new_parent: ObjectId) -> Result<(), HierarchyError> {
        let node = self.nodes.get(&id).ok_or(HierarchyError::UnknownNode)?;
        if new_parent == node.parent {
            return Ok(());
        }
        if new_parent == id || self.is_descendant_of(new_parent, id) {
            return Err(HierarchyError::Cycle);
        }
        if !new_parent.is_nil() && !self.nodes.contains_key(&new_parent) {
            return Err(HierarchyError::UnknownParent);
        }
        let world = self.world(id).ok_or(HierarchyError::UnknownNode)?;
        let parent_world = match self.world(new_parent) {
            Some(pose) => pose,
            None if new_parent.is_nil() => Transform::identity(),
            None => return Err(HierarchyError::UnknownParent),
        };
        let local = relative(parent_world, world).ok_or(HierarchyError::DegenerateScale)?;
        let slot = self.nodes.get_mut(&id).ok_or(HierarchyError::UnknownNode)?;
        slot.parent = new_parent;
        slot.local = local;
        Ok(())
    }

    /// Remove a node. Its children attach to the removed node's parent with
    /// local poses recomputed so every surviving world pose is unchanged.
    pub fn remove(&mut self, id: ObjectId) -> Result<(), HierarchyError> {
        let node = *self.nodes.get(&id).ok_or(HierarchyError::UnknownNode)?;
        // Precompute before mutating so a degenerate scale fails atomically.
        let mut relinked = Vec::new();
        // Every child shares one grandparent: walk its chain once, on the first child.
        let mut grandparent_world = None;
        for child in self.children(id) {
            let child_world = self.world(child).ok_or(HierarchyError::UnknownNode)?;
            let parent_pose = match grandparent_world {
                Some(pose) => pose,
                None => {
                    let pose = match self.world(node.parent) {
                        Some(pose) => pose,
                        None if node.parent.is_nil() => Transform::identity(),
                        None => return Err(HierarchyError::UnknownParent),
                    };
                    grandparent_world = Some(pose);
                    pose
                }
            };
            let local =
                relative(parent_pose, child_world).ok_or(HierarchyError::DegenerateScale)?;
            relinked.push((child, local));
        }
        for (child, local) in relinked {
            let slot = self.nodes.get_mut(&child).ok_or(HierarchyError::UnknownNode)?;
            slot.parent = node.parent;
            slot.local = local;
        }
        self.nodes.remove(&id);
        Ok(())
    }

    /// World pose of a node, if it exists and its ancestor chain is intact.
    ///
    /// Root-first composition; `None` for unknown ids and corrupt chains
    /// (dangling parents, link cycles). See [`Hierarchy::validate`].
    pub fn world(&self, id: ObjectId) -> Option<Transform> {
        let mut chain = Vec::new();
        let mut cursor = id;
        // One step per node plus the final root; a longer walk repeats one.
        for _ in 0..=self.nodes.len() {
            let node = self.nodes.get(&cursor)?;
            chain.push(node.local);
            if node.parent.is_nil() {
                break;
            }
            cursor = node.parent;
        }
        if !chain.is_empty() && !self.nodes.contains_key(&cursor) {
            return None;
        }
        let mut world = Transform::identity();
        for local in chain.iter().rev() {
            world = compose(world, *local);
        }
        Some(world)
    }

    /// Direct children of a parent, sorted by id. Pass nil for the roots.
    pub fn children(&self, parent: ObjectId) -> Vec<ObjectId> {
        let mut out: Vec<ObjectId> = self
            .nodes
            .iter()
            .filter(|(_, node)| node.parent == parent)
            .map(|(id, _)| *id)
            .collect();
        out.sort_by_key(|id: &ObjectId| id.as_uuid());
        out
    }

    /// Every node's direct children in one pass: parent id to child ids,
    /// each list sorted like [`Hierarchy::children`]. Roots sit under the nil
    /// key. Walking a whole tree through this map costs `O(n log n)`, where
    /// calling [`Hierarchy::children`] per node costs `O(n^2)`.
    pub fn child_map(&self) -> BTreeMap<ObjectId, Vec<ObjectId>> {
        let mut map: BTreeMap<ObjectId, Vec<ObjectId>> = BTreeMap::new();
        // The node map iterates in id order, so each list lands sorted.
        for (id, node) in &self.nodes {
            map.entry(node.parent).or_default().push(*id);
        }
        map
    }

    /// Nodes without a parent, sorted by id.
    pub fn roots(&self) -> Vec<ObjectId> {
        self.children(ObjectId::NIL)
    }

    /// Every node id, parents before children and siblings in id order.
    /// Use it for world-pose propagation or any parent-first pass.
    pub fn propagation_order(&self) -> Vec<ObjectId> {
        let mut order = Vec::with_capacity(self.nodes.len());
        let mut stack: Vec<ObjectId> = self.roots();
        stack.reverse();
        while let Some(id) = stack.pop() {
            order.push(id);
            let mut kids = self.children(id);
            kids.reverse();
            stack.extend(kids);
        }
        order
    }

    /// Every node's world pose in [`Hierarchy::propagation_order`].
    ///
    /// Nodes unreachable from a root (corrupt hand-built documents only)
    /// trail last in id order so no stored node is silently dropped.
    pub fn world_all(&self) -> Vec<(ObjectId, Transform)> {
        let mut out = Vec::with_capacity(self.nodes.len());
        for id in self.propagation_order() {
            if let Some(pose) = self.world(id) {
                out.push((id, pose));
            }
        }
        if out.len() < self.nodes.len() {
            let mut missing: Vec<ObjectId> = self
                .nodes
                .keys()
                .filter(|id| !out.iter().any(|(listed, _)| listed == *id))
                .copied()
                .collect();
            missing.sort_by_key(|id: &ObjectId| id.as_uuid());
            for id in missing {
                if let Some(pose) = self.world(id) {
                    out.push((id, pose));
                }
            }
        }
        out
    }

    /// Check stored links: every parent is nil or a known node, every pose
    /// is finite, and no link cycle exists. Nodes visit in id order, so the
    /// first reported culprit is deterministic for a given set of ids.
    pub fn validate(&self) -> Result<(), HierarchyError> {
        for (id, node) in &self.nodes {
            if !node.local.is_finite() {
                return Err(HierarchyError::NonFinite);
            }
            if !node.parent.is_nil() && !self.nodes.contains_key(&node.parent) {
                return Err(HierarchyError::UnknownParent);
            }
            // A cycle shows up as a walk that revisits a node.
            let mut cursor = *id;
            for _ in 0..=self.nodes.len() {
                let Some(next) = self.nodes.get(&cursor) else {
                    break;
                };
                if next.parent.is_nil() {
                    break;
                }
                if next.parent == *id && cursor != *id {
                    return Err(HierarchyError::Cycle);
                }
                cursor = next.parent;
                if cursor == *id {
                    return Err(HierarchyError::Cycle);
                }
            }
        }
        Ok(())
    }

    /// True when `maybe_descendant` is `ancestor` itself or sits below it.
    fn is_descendant_of(&self, maybe_descendant: ObjectId, ancestor: ObjectId) -> bool {
        let mut cursor = maybe_descendant;
        for _ in 0..=self.nodes.len() {
            if cursor == ancestor {
                return true;
            }
            match self.nodes.get(&cursor) {
                Some(node) if !node.parent.is_nil() => cursor = node.parent,
                _ => return false,
            }
        }
        // Over-long walk means corrupt links; refuse to build on them.
        true
    }
}

/// Compose a parent world pose with a local pose (see module docs).
fn compose(parent: Transform, local: Transform) -> Transform {
    let parent_rotation = quat_normalized(parent.rotation);
    let scaled = [
        local.position[0] * parent.scale[0],
        local.position[1] * parent.scale[1],
        local.position[2] * parent.scale[2],
    ];
    let rotated = quat_rotate(parent_rotation, scaled);
    Transform {
        position: [
            parent.position[0] + rotated[0],
            parent.position[1] + rotated[1],
            parent.position[2] + rotated[2],
        ],
        rotation: quat_normalized(quat_mul(parent_rotation, quat_normalized(local.rotation))),
        scale: [
            parent.scale[0] * local.scale[0],
            parent.scale[1] * local.scale[1],
            parent.scale[2] * local.scale[2],
        ],
    }
}

/// Invert a pose triple, or `None` when a scale axis is too small.
fn inverse(pose: Transform) -> Option<Transform> {
    let ok = pose
        .scale
        .iter()
        .all(|s| s.is_finite() && s.abs() >= MIN_INVERTIBLE_SCALE);
    if !ok {
        return None;
    }
    let scale = [
        1.0 / pose.scale[0],
        1.0 / pose.scale[1],
        1.0 / pose.scale[2],
    ];
    let rotation = quat_conjugate(quat_normalized(pose.rotation));
    let negated = [-pose.position[0], -pose.position[1], -pose.position[2]];
    let rotated = quat_rotate(rotation, negated);
    Some(Transform {
        position: [
            rotated[0] * scale[0],
            rotated[1] * scale[1],
            rotated[2] * scale[2],
        ],
        rotation,
        scale,
    })
}

/// Local pose that puts `child_world` under `parent_world`.
fn relative(parent_world: Transform, child_world: Transform) -> Option<Transform> {
    let undo_parent = inverse(parent_world)?;
    if !child_world.is_finite() {
        return None;
    }
    Some(compose(undo_parent, child_world))
}

/// Conjugate (inverse for unit quaternions).
fn quat_conjugate(q: [f32; 4]) -> [f32; 4] {
    [-q[0], -q[1], -q[2], q[3]]
}

/// Unit quaternion, or identity when the input is degenerate.
fn quat_normalized(q: [f32; 4]) -> [f32; 4] {
    let len_sq = q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3];
    if !len_sq.is_finite() || len_sq < 1e-12 {
        return [0.0, 0.0, 0.0, 1.0];
    }
    let len = len_sq.sqrt();
    [q[0] / len, q[1] / len, q[2] / len, q[3] / len]
}

/// Rotate a vector by a unit quaternion.
fn quat_rotate(q: [f32; 4], v: [f32; 3]) -> [f32; 3] {
    // t = 2 * cross(q.xyz, v); v' = v + q.w * t + cross(q.xyz, t).
    let tx = 2.0 * (q[1] * v[2] - q[2] * v[1]);
    let ty = 2.0 * (q[2] * v[0] - q[0] * v[2]);
    let tz = 2.0 * (q[0] * v[1] - q[1] * v[0]);
    [
        v[0] + q[3] * tx + (q[1] * tz - q[2] * ty),
        v[1] + q[3] * ty + (q[2] * tx - q[0] * tz),
        v[2] + q[3] * tz + (q[0] * ty - q[1] * tx),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    const EPS: f32 = 1e-4;

    fn pose(position: [f32; 3]) -> Transform {
        Transform::new(position, [0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0])
    }

    /// Pose with unit scale; rotation fixed to a quarter turn about +Y.
    fn spun(position: [f32; 3]) -> Transform {
        Transform::new(position, quarter_turn_y(), [2.0, 2.0, 2.0])
    }

    /// Pose with identity rotation and matching scale.
    fn pose_s(position: [f32; 3], scale: [f32; 3]) -> Transform {
        Transform::new(position, [0.0, 0.0, 0.0, 1.0], scale)
    }

    fn near(a: &[f32], b: &[f32]) -> bool {
        a.iter().zip(b.iter()).all(|(x, y)| (x - y).abs() <= EPS)
    }

    fn assert_pose(actual: Transform, expected: Transform) {
        assert!(
            near(&actual.position, &expected.position)
                && near(&actual.rotation, &expected.rotation)
                && near(&actual.scale, &expected.scale),
            "pose mismatch:\n  actual:   {actual:?}\n  expected: {expected:?}"
        );
    }

    /// Quarter turn about +Y, `[x, y, z, w]`.
    #[expect(
        clippy::disallowed_methods,
        reason = "test-only fixture quaternion, never hashed"
    )]
    fn quarter_turn_y() -> [f32; 4] {
        let half = std::f32::consts::FRAC_PI_4;
        [0.0, half.sin(), 0.0, half.cos()]
    }

    #[test]
    fn insert_rejects_bad_links() {
        let mut tree = Hierarchy::new();
        let root = ObjectId::new();
        assert_eq!(
            tree.insert(ObjectId::NIL, ObjectId::NIL, Transform::identity()),
            Err(HierarchyError::NilId)
        );
        tree.insert(root, ObjectId::NIL, Transform::identity())
            .unwrap();
        assert_eq!(
            tree.insert(root, ObjectId::NIL, Transform::identity()),
            Err(HierarchyError::Duplicate)
        );
        assert_eq!(
            tree.insert(ObjectId::new(), ObjectId::new(), Transform::identity()),
            Err(HierarchyError::UnknownParent)
        );
        let nan = Transform::new([f32::NAN, 0.0, 0.0], [0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0]);
        assert_eq!(
            tree.insert(ObjectId::new(), ObjectId::NIL, nan),
            Err(HierarchyError::NonFinite)
        );
        assert_eq!(tree.validate(), Ok(()));
    }

    #[test]
    fn world_pose_composes_down_the_chain() {
        let mut tree = Hierarchy::new();
        let root = ObjectId::new();
        let child = ObjectId::new();
        tree.insert(root, ObjectId::NIL, pose([1.0, 0.0, 0.0]))
            .unwrap();
        tree.insert(child, root, pose([0.0, 2.0, 0.0])).unwrap();
        assert_pose(tree.world(child).unwrap(), pose([1.0, 2.0, 0.0]));
        assert_pose(tree.world(root).unwrap(), pose([1.0, 0.0, 0.0]));
        assert_eq!(tree.world(ObjectId::new()), None);
    }

    #[test]
    fn rotation_and_scale_reach_children() {
        let mut tree = Hierarchy::new();
        let root = ObjectId::new();
        let child = ObjectId::new();
        tree.insert(root, ObjectId::NIL, spun([0.0, 0.0, 0.0]))
            .unwrap();
        tree.insert(child, root, pose([1.0, 0.0, 0.0])).unwrap();
        // +X scaled by 2 then turned a quarter about +Y lands on -Z.
        assert_pose(tree.world(child).unwrap(), spun([0.0, 0.0, -2.0]));
    }

    #[test]
    fn cycles_are_rejected_on_reparent() {
        let mut tree = Hierarchy::new();
        let a = ObjectId::new();
        let b = ObjectId::new();
        let c = ObjectId::new();
        tree.insert(a, ObjectId::NIL, pose([1.0, 0.0, 0.0]))
            .unwrap();
        tree.insert(b, a, pose([0.0, 1.0, 0.0])).unwrap();
        tree.insert(c, b, pose([0.0, 0.0, 1.0])).unwrap();

        assert_eq!(tree.reparent(a, a), Err(HierarchyError::Cycle));
        assert_eq!(tree.reparent(a, b), Err(HierarchyError::Cycle));
        assert_eq!(tree.reparent(a, c), Err(HierarchyError::Cycle));
        assert_eq!(tree.reparent(b, c), Err(HierarchyError::Cycle));
        assert_pose(tree.world(c).unwrap(), pose([1.0, 1.0, 1.0]));
        assert_eq!(tree.parent(a), Some(ObjectId::NIL));
        assert_eq!(tree.parent(b), Some(a));
        tree.reparent(c, a).unwrap();
        assert_eq!(tree.parent(c), Some(a));
        assert_pose(tree.world(c).unwrap(), pose([1.0, 1.0, 1.0]));
        assert_eq!(tree.validate(), Ok(()));
    }

    #[test]
    fn propagation_order_is_parent_first_and_deterministic() {
        let root = ObjectId::new();
        let left = ObjectId::new();
        let right = ObjectId::new();
        let grand = ObjectId::new();
        let build = |flip: bool| {
            let mut tree = Hierarchy::new();
            tree.insert(root, ObjectId::NIL, Transform::identity())
                .unwrap();
            // Siblings inserted in opposite orders must walk identically.
            let (first, second) = if flip { (left, right) } else { (right, left) };
            tree.insert(first, root, Transform::identity()).unwrap();
            tree.insert(second, root, Transform::identity()).unwrap();
            tree.insert(grand, left, Transform::identity()).unwrap();
            tree
        };
        let first = build(false);
        let second = build(true);
        assert_eq!(first.propagation_order(), second.propagation_order());

        let tree = first;
        let order = tree.propagation_order();
        assert_eq!(order.len(), 4);
        let rank = |id: ObjectId| order.iter().position(|listed| *listed == id).unwrap();
        assert!(rank(root) < rank(left));
        assert!(rank(root) < rank(right));
        assert!(rank(left) < rank(grand));
        let mut siblings = vec![left, right];
        siblings.sort_by_key(|id: &ObjectId| id.as_uuid());
        assert_eq!(tree.children(root), siblings);
        let mut seen = std::collections::BTreeSet::new();
        for id in order {
            let parent = tree.parent(id).unwrap();
            if !parent.is_nil() {
                assert!(seen.contains(&parent));
            }
            seen.insert(id);
        }
    }

    #[test]
    fn reparent_keeps_world_pose() {
        let mut tree = Hierarchy::new();
        let a = ObjectId::new();
        let b = ObjectId::new();
        let child = ObjectId::new();
        tree.insert(a, ObjectId::NIL, spun([5.0, 0.0, 0.0]))
            .unwrap();
        tree.insert(b, ObjectId::NIL, pose([-3.0, 1.0, 0.0]))
            .unwrap();
        tree.insert(child, a, pose([1.0, 1.0, 0.0])).unwrap();

        let before = tree.world(child).unwrap();
        tree.reparent(child, b).unwrap();
        assert_eq!(tree.parent(child), Some(b));
        assert_pose(tree.world(child).unwrap(), before);

        let before = tree.world(child).unwrap();
        tree.reparent(child, ObjectId::NIL).unwrap();
        assert_eq!(tree.parent(child), Some(ObjectId::NIL));
        assert_pose(tree.world(child).unwrap(), before);
        assert_pose(tree.local(child).unwrap(), before);
        assert_eq!(tree.validate(), Ok(()));
    }

    #[test]
    fn reparent_needs_invertible_scales() {
        let mut tree = Hierarchy::new();
        let flat = ObjectId::new();
        let child = ObjectId::new();
        let squashed = Transform::new([0.0, 0.0, 0.0], [0.0, 0.0, 0.0, 1.0], [0.0, 1.0, 1.0]);
        tree.insert(flat, ObjectId::NIL, squashed).unwrap();
        tree.insert(child, ObjectId::NIL, pose([1.0, 0.0, 0.0]))
            .unwrap();
        assert_eq!(
            tree.reparent(child, flat),
            Err(HierarchyError::DegenerateScale)
        );
        assert_eq!(tree.parent(child), Some(ObjectId::NIL));
    }

    #[test]
    fn child_map_matches_children_for_every_node() {
        let mut tree = Hierarchy::new();
        let mut ids = Vec::new();
        for index in 0..40 {
            let id = ObjectId::new();
            let parent = if index < 4 {
                ObjectId::NIL
            } else {
                ids[index % 4]
            };
            tree.insert(id, parent, pose([index as f32, 0.0, 0.0]))
                .unwrap();
            ids.push(id);
        }
        let map = tree.child_map();
        assert_eq!(map.get(&ObjectId::NIL), Some(&tree.roots()));
        for id in &ids {
            let listed = map.get(id).cloned().unwrap_or_default();
            assert_eq!(listed, tree.children(*id));
        }
        let listed: usize = map.values().map(Vec::len).sum();
        assert_eq!(listed, tree.len(), "every node sits under one parent");
        assert!(Hierarchy::new().child_map().is_empty());
    }

    #[test]
    fn remove_relinks_children_without_moving_them() {
        let mut tree = Hierarchy::new();
        let root = ObjectId::new();
        let mid = ObjectId::new();
        let leaf = ObjectId::new();
        tree.insert(root, ObjectId::NIL, pose([10.0, 0.0, 0.0]))
            .unwrap();
        tree.insert(mid, root, pose([0.0, 5.0, 0.0])).unwrap();
        tree.insert(leaf, mid, pose([0.0, 0.0, 2.0])).unwrap();

        let leaf_world = tree.world(leaf).unwrap();
        tree.remove(mid).unwrap();
        assert!(!tree.contains(mid));
        assert_eq!(tree.parent(leaf), Some(root));
        assert_pose(tree.world(leaf).unwrap(), leaf_world);
        assert_eq!(tree.validate(), Ok(()));
        assert_eq!(
            tree.remove(ObjectId::new()),
            Err(HierarchyError::UnknownNode)
        );
    }

    #[test]
    fn serde_roundtrip_preserves_links_and_poses() {
        let mut tree = Hierarchy::new();
        let root = ObjectId::new();
        let child = ObjectId::new();
        tree.insert(
            root,
            ObjectId::NIL,
            Transform::new([1.0, 2.0, 3.0], quarter_turn_y(), [1.0, 2.0, 1.0]),
        )
        .unwrap();
        tree.insert(child, root, pose([4.0, 5.0, 6.0])).unwrap();

        let json = serde_json::to_string(&tree).unwrap();
        let back: Hierarchy = serde_json::from_str(&json).unwrap();
        assert_eq!(back, tree);
        assert_eq!(back.parent(child), Some(root));
        assert_pose(back.world(child).unwrap(), tree.world(child).unwrap());
        assert_eq!(back.validate(), Ok(()));

        let pose_json = serde_json::to_string(&Transform::identity()).unwrap();
        let pose_back: Transform = serde_json::from_str(&pose_json).unwrap();
        assert_eq!(pose_back, Transform::identity());
    }

    #[test]
    fn matrix_matches_composition_for_axis_aligned_poses() {
        let parent = pose_s([1.0, 2.0, 3.0], [2.0, 2.0, 2.0]);
        let local = pose([1.0, 0.0, 0.0]);
        // Uniform parent scale reaches the child position and scale.
        let world = compose(parent, local);
        assert_pose(world, pose_s([3.0, 2.0, 3.0], [2.0, 2.0, 2.0]));

        let product = mat_mul(parent.to_matrix(), local.to_matrix());
        let direct = world.to_matrix();
        for row in 0..4 {
            for col in 0..4 {
                assert!(
                    (product[row][col] - direct[row][col]).abs() <= EPS,
                    "matrix mismatch at [{row}][{col}]"
                );
            }
        }
    }

    fn mat_mul(a: [[f32; 4]; 4], b: [[f32; 4]; 4]) -> [[f32; 4]; 4] {
        let mut out = [[0.0; 4]; 4];
        for row in 0..4 {
            for col in 0..4 {
                out[row][col] = a[row][0] * b[0][col]
                    + a[row][1] * b[1][col]
                    + a[row][2] * b[2][col]
                    + a[row][3] * b[3][col];
            }
        }
        out
    }
}
