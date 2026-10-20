//! Prefabs: a selection subtree saved as data and instantiated as new nodes.
//!
//! The consumer is [`Shell`](crate::shell::Shell): `save_prefab` captures the
//! selected node's subtree, `place_prefab` builds one command batch that
//! inserts a fresh copy under any parent. Data owner is [`Prefab`]: local
//! poses, typed components, material bindings and children in hierarchy
//! order. Units follow the document (meters/radians). Capture and instantiate
//! are pure; the shell owns the command service, logging and undo. Errors are
//! typed [`PrefabError`]; a refused capture changes nothing.
//!
//! Determinism: capture keeps insertion order; instantiate allocates fresh
//! [`ObjectId`]s, so two placements never share nodes. A placement is one
//! batch, so one undo removes the whole copy.

use std::collections::BTreeMap;

use engine_scene::component::{Component, ComponentKind};
use engine_scene::ids::ObjectId;
use engine_scene::transform::Transform;

use crate::commands::{Document, MaterialName, Op};

/// One node in a prefab: local pose, typed components, material and children.
#[derive(Clone, Debug, PartialEq)]
pub struct PrefabNode {
    /// Pose relative to the parent, in meters/radians.
    pub local: Transform,
    /// Every typed component the node carried, in kind order.
    pub components: BTreeMap<ComponentKind, Component>,
    /// Material asset binding, if any.
    pub material: Option<MaterialName>,
    /// Child nodes in hierarchy order.
    pub children: Vec<PrefabNode>,
}

/// A named subtree captured from the document.
#[derive(Clone, Debug, PartialEq)]
pub struct Prefab {
    /// Unique name in the shell's prefab list.
    pub name: String,
    /// Captured root with its descendants.
    pub root: PrefabNode,
}

/// Ops plus the fresh root id of one instantiation.
#[derive(Clone, Debug, PartialEq)]
pub struct Instantiation {
    /// Insert / component / material ops, parents before children.
    pub ops: Vec<Op>,
    /// Fresh id of the copied root (what the caller may select).
    pub root: ObjectId,
}

/// What refused a prefab operation. Nothing is captured or staged on error.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PrefabError {
    /// The prefab name was empty or only whitespace.
    EmptyName,
    /// The selected root is not in the document.
    MissingNode(ObjectId),
    /// Capture was asked with no selection.
    NoSelection,
    /// The placement offset held NaN or infinity.
    NonFinite,
}

impl std::fmt::Display for PrefabError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyName => f.write_str("prefab name is empty"),
            Self::MissingNode(id) => write!(f, "prefab root {id} is not in the document"),
            Self::NoSelection => f.write_str("select a node before saving a prefab"),
            Self::NonFinite => f.write_str("prefab placement offset is not finite"),
        }
    }
}

impl std::error::Error for PrefabError {}

impl Prefab {
    /// Capture `root` and every descendant from `doc` under `name`.
    pub fn capture(
        doc: &Document,
        root: ObjectId,
        name: impl Into<String>,
    ) -> Result<Self, PrefabError> {
        let name = name.into();
        if name.trim().is_empty() {
            return Err(PrefabError::EmptyName);
        }
        if !doc.hierarchy.contains(root) {
            return Err(PrefabError::MissingNode(root));
        }
        Ok(Self {
            name,
            root: capture_node(doc, root)?,
        })
    }

    /// Build the ops that insert a fresh copy under `parent` (`NIL` for a
    /// root), with the root's local position offset by `offset` meters.
    pub fn instantiate(
        &self,
        parent: ObjectId,
        offset: [f32; 3],
    ) -> Result<Instantiation, PrefabError> {
        if !offset.iter().all(|c| c.is_finite()) {
            return Err(PrefabError::NonFinite);
        }
        let mut ops = Vec::new();
        let root = push_node(&self.root, parent, Some(offset), &mut ops);
        Ok(Instantiation { ops, root })
    }

    /// Number of nodes in the prefab (root included).
    pub fn node_count(&self) -> usize {
        1 + self
            .root
            .children
            .iter()
            .map(PrefabNode::node_count)
            .sum::<usize>()
    }
}

impl PrefabNode {
    /// Number of nodes in this subtree (self included).
    pub fn node_count(&self) -> usize {
        1 + self.children.iter().map(Self::node_count).sum::<usize>()
    }
}

fn capture_node(doc: &Document, id: ObjectId) -> Result<PrefabNode, PrefabError> {
    let local = doc
        .hierarchy
        .local(id)
        .ok_or(PrefabError::MissingNode(id))?;
    let key = id.to_string();
    let components = doc.components.get(&key).cloned().unwrap_or_default();
    let material = doc.materials.get(&key).copied();
    let mut children = Vec::new();
    for child in doc.hierarchy.children(id) {
        children.push(capture_node(doc, child)?);
    }
    Ok(PrefabNode {
        local,
        components,
        material,
        children,
    })
}

fn push_node(
    node: &PrefabNode,
    parent: ObjectId,
    offset: Option<[f32; 3]>,
    ops: &mut Vec<Op>,
) -> ObjectId {
    let id = ObjectId::new();
    let mut local = node.local;
    if let Some(offset) = offset {
        local.position = [
            local.position[0] + offset[0],
            local.position[1] + offset[1],
            local.position[2] + offset[2],
        ];
    }
    ops.push(Op::Insert { id, parent, local });
    for component in node.components.values() {
        ops.push(Op::SetComponent {
            id,
            component: *component,
        });
    }
    if let Some(material) = node.material {
        ops.push(Op::SetMaterial {
            id,
            material: Some(material),
        });
    }
    for child in &node.children {
        push_node(child, id, None, ops);
    }
    id
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capture_refuses_empty_name_and_missing_node() {
        let doc = Document::default();
        let missing = ObjectId::new();
        assert_eq!(
            Prefab::capture(&doc, missing, "tower"),
            Err(PrefabError::MissingNode(missing))
        );

        let mut doc = Document::default();
        let id = ObjectId::new();
        doc.hierarchy
            .insert(id, ObjectId::NIL, Transform::identity())
            .unwrap();
        assert_eq!(
            Prefab::capture(&doc, id, "   "),
            Err(PrefabError::EmptyName)
        );
    }

    #[test]
    fn instantiate_builds_parent_first_ops_with_fresh_ids() {
        let mut doc = Document::default();
        let root = ObjectId::new();
        let child = ObjectId::new();
        doc.hierarchy
            .insert(root, ObjectId::NIL, Transform::identity())
            .unwrap();
        doc.hierarchy
            .insert(
                child,
                root,
                Transform::new([0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0]),
            )
            .unwrap();
        let prefab = Prefab::capture(&doc, root, "tower").unwrap();
        assert_eq!(prefab.node_count(), 2);
        let made = prefab.instantiate(ObjectId::NIL, [10.0, 0.0, 0.0]).unwrap();
        assert_eq!(made.ops.len(), 2);
        let Op::Insert {
            id: root_id,
            parent,
            local,
        } = &made.ops[0]
        else {
            panic!("root insert first");
        };
        assert_eq!(*parent, ObjectId::NIL);
        assert_eq!(local.position, [10.0, 0.0, 0.0]);
        assert_eq!(*root_id, made.root);
        let Op::Insert {
            id: child_id,
            parent: child_parent,
            ..
        } = &made.ops[1]
        else {
            panic!("child insert second");
        };
        assert_eq!(*child_parent, made.root);
        assert_ne!(*child_id, *root_id);
        assert!(!doc.hierarchy.contains(*root_id));
    }
}
