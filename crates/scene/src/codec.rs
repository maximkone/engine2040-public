//! Versioned scene codec: [`Hierarchy`] to JSON text and back.
//!
//! A save is a [`Scene`] document: a format version plus one entry per node
//! carrying its id, parent link and local pose. [`Scene`] derives serde so a
//! consumer with `serde_json` reads and writes human-readable JSON text; the
//! types here own the version gate, the migration step and the reference
//! checks. Decoding builds a private hierarchy and returns it only on
//! success, so a bad save never leaves a partial tree behind.
//!
//! Nodes encode sorted by id, so the same hierarchy always prints the same
//! text whatever order nodes were inserted in. Stable text keeps diffs small
//! and lets a future content digest hash the save bytes directly.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use serde::{Deserialize, Serialize};

use super::ids::{ObjectId, SCHEMA_VERSION};
use super::pin::{canonical_digest, PinError, ScenePin};
use super::transform::{Hierarchy, HierarchyError, Transform};

/// Newest save version this codec writes and reads.
pub const CURRENT_VERSION: u32 = SCHEMA_VERSION;

/// Oldest save version this codec still migrates. Older saves, and saves
/// newer than [`CURRENT_VERSION`], fail with
/// [`CodecError::UnsupportedVersion`] naming the versions involved.
pub const MIN_SUPPORTED_VERSION: u32 = 1;

/// One saved node: identity, parent link and local pose.
///
/// A nil parent means a root. Any other parent must name a node in the same
/// document; dangling links fail decode with the culprit named.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct SceneNode {
    /// Identity of the authored object; never nil.
    pub id: ObjectId,
    /// Parent object, or [`ObjectId::NIL`] for a root.
    pub parent: ObjectId,
    /// Pose relative to the parent (or to the world for roots).
    pub transform: Transform,
}

/// A versioned scene save: format version plus one entry per node.
///
/// The JSON text form is `{"version":1,"nodes":[...]}`. Node order in the
/// text is by id (see [`Scene::encode`]); decode accepts any order.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Scene {
    /// Save format version; see [`CURRENT_VERSION`].
    pub version: u32,
    /// One entry per node, sorted by id.
    pub nodes: Vec<SceneNode>,
}

/// What went wrong reading a save.
///
/// Every variant names the culprit (node id, parent id or version numbers),
/// so editor and command layers can quote it in diagnostics.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CodecError {
    /// The save version is newer than this codec or older than the oldest
    /// supported migration. Carries what was found plus the accepted range.
    UnsupportedVersion {
        /// Version stamped on the save.
        got: u32,
        /// Oldest version this codec migrates ([`MIN_SUPPORTED_VERSION`]).
        min: u32,
        /// Newest version this codec reads ([`CURRENT_VERSION`]).
        current: u32,
    },
    /// A node id was nil; nil is the "no parent" slot, not a node.
    NilId,
    /// Two entries share one id.
    Duplicate {
        /// The repeated id.
        id: ObjectId,
    },
    /// A parent link names no node in the same save.
    MissingParent {
        /// The node holding the dangling link.
        node: ObjectId,
        /// The parent id no entry provides.
        parent: ObjectId,
    },
    /// Parent links form a loop, so no insertion order can satisfy them.
    Cycle {
        /// A node stuck in the loop.
        node: ObjectId,
    },
    /// A saved pose holds NaN or infinity.
    NonFinite {
        /// The node holding the bad pose.
        id: ObjectId,
    },
}

impl fmt::Display for CodecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedVersion { got, min, current } => write!(
                f,
                "unsupported scene version v{got}; this codec reads v{min}..=v{current} \
                 (save is newer than the reader or older than the oldest migration)"
            ),
            Self::NilId => f.write_str("scene node id must not be nil"),
            Self::Duplicate { id } => write!(f, "duplicate scene node id {id}"),
            Self::MissingParent { node, parent } => write!(
                f,
                "scene node {node} names unknown parent {parent}; \
                 the parent entry is missing from the save"
            ),
            Self::Cycle { node } => {
                write!(f, "scene parent links form a cycle at node {node}")
            }
            Self::NonFinite { id } => {
                write!(f, "scene node {id} holds a non-finite transform")
            }
        }
    }
}

impl std::error::Error for CodecError {}

impl Scene {
    /// Encode a hierarchy as a versioned save with nodes sorted by id.
    ///
    /// Sorting makes the printed text independent of insertion order.
    pub fn encode(tree: &Hierarchy) -> Self {
        // The public hierarchy API keeps every node reachable from a root,
        // so the propagation order lists each stored id exactly once.
        let mut nodes: Vec<SceneNode> = tree
            .propagation_order()
            .into_iter()
            .map(|id| SceneNode {
                id,
                parent: tree.parent(id).expect("propagation order lists stored ids"),
                transform: tree.local(id).expect("propagation order lists stored ids"),
            })
            .collect();
        nodes.sort_by_key(|node| node.id.as_uuid());
        Self {
            version: CURRENT_VERSION,
            nodes,
        }
    }

    /// Decode a save into a hierarchy.
    ///
    /// Accepts any supported version (see [`Scene::migrate`] to stamp an
    /// older save forward first). Entries may arrive in any order. Any
    /// failure returns an error; no partial hierarchy escapes.
    pub fn decode(&self) -> Result<Hierarchy, CodecError> {
        check_version(self.version)?;
        // Sort first so the reported culprit is deterministic for a given
        // set of entries, whatever order the text listed them in.
        let mut ordered: Vec<&SceneNode> = self.nodes.iter().collect();
        ordered.sort_by_key(|node| node.id.as_uuid());

        let mut known: BTreeMap<ObjectId, &SceneNode> = BTreeMap::new();
        for node in &ordered {
            if node.id.is_nil() {
                return Err(CodecError::NilId);
            }
            if known.contains_key(&node.id) {
                return Err(CodecError::Duplicate { id: node.id });
            }
            if !node.transform.is_finite() {
                return Err(CodecError::NonFinite { id: node.id });
            }
            known.insert(node.id, node);
        }
        for node in &ordered {
            if !node.parent.is_nil() && !known.contains_key(&node.parent) {
                return Err(CodecError::MissingParent {
                    node: node.id,
                    parent: node.parent,
                });
            }
        }

        // Insert parents before children. A pass with no progress and nodes
        // left means the remaining links loop, since every named parent is
        // known to exist from the check above.
        let mut tree = Hierarchy::new();
        let mut done: BTreeSet<ObjectId> = BTreeSet::new();
        while done.len() < ordered.len() {
            let mut progressed = false;
            for node in &ordered {
                if done.contains(&node.id) {
                    continue;
                }
                if node.parent.is_nil() || done.contains(&node.parent) {
                    tree.insert(node.id, node.parent, node.transform)
                        .map_err(|error| map_insert_error(error, node))?;
                    done.insert(node.id);
                    progressed = true;
                }
            }
            if !progressed {
                let stuck = ordered
                    .iter()
                    .find(|node| !done.contains(&node.id))
                    .expect("loop exits only with nodes left");
                return Err(CodecError::Cycle { node: stuck.id });
            }
        }
        // Valid by construction: poses checked finite, parents checked
        // known, ids fresh, so no dangling links, loops or bad poses.
        Ok(tree)
    }

    /// Migrate a supported save forward to the current version.
    ///
    /// v1 is current, so today this only stamps the version. Later versions
    /// add their field fixups here ahead of the stamp; versions outside the
    /// supported range fail without touching the document.
    pub fn migrate(&mut self) -> Result<(), CodecError> {
        check_version(self.version)?;
        self.version = CURRENT_VERSION;
        Ok(())
    }

    /// Content digest of this save's canonical bytes (see
    /// [`canonical_digest`]): the same nodes always digest alike, whatever
    /// order the text listed them in.
    pub fn digest(&self) -> String {
        canonical_digest(self.version, &self.nodes)
    }

    /// Pin this save at `revision` for a pre-start run gate.
    ///
    /// The pin carries the revision plus [`Scene::digest`] for the run
    /// manifest; revision 0 refuses with [`PinError::EmptyArena`], so an
    /// empty arena never plays silently.
    pub fn pin(&self, revision: u64) -> Result<ScenePin, PinError> {
        ScenePin::require(Some(ScenePin {
            version: super::pin::RUN_PIN_VERSION,
            schema: self.version,
            revision,
            digest: self.digest(),
        }))
    }
}

/// Reject versions newer than the reader or older than the oldest migration.
fn check_version(version: u32) -> Result<(), CodecError> {
    if (MIN_SUPPORTED_VERSION..=CURRENT_VERSION).contains(&version) {
        Ok(())
    } else {
        Err(CodecError::UnsupportedVersion {
            got: version,
            min: MIN_SUPPORTED_VERSION,
            current: CURRENT_VERSION,
        })
    }
}

/// Translate a hierarchy insert failure into the matching codec error.
///
/// Unreachable after decode's pre-checks (finite poses, known parents,
/// fresh ids, and fresh ids cannot close a loop); mapped rather than
/// panicked so corrupt saves always stay errors, never crashes.
fn map_insert_error(error: HierarchyError, node: &SceneNode) -> CodecError {
    match error {
        HierarchyError::NilId => CodecError::NilId,
        HierarchyError::Duplicate => CodecError::Duplicate { id: node.id },
        HierarchyError::UnknownParent | HierarchyError::UnknownNode => CodecError::MissingParent {
            node: node.id,
            parent: node.parent,
        },
        HierarchyError::Cycle | HierarchyError::DegenerateScale => {
            CodecError::Cycle { node: node.id }
        }
        HierarchyError::NonFinite => CodecError::NonFinite { id: node.id },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pose(position: [f32; 3]) -> Transform {
        Transform::new(position, [0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0])
    }

    #[expect(
        clippy::disallowed_methods,
        reason = "test-only fixture quaternion, never hashed"
    )]
    fn quarter_turn_y() -> [f32; 4] {
        let half = std::f32::consts::FRAC_PI_4;
        [0.0, half.sin(), 0.0, half.cos()]
    }

    /// Root plus child plus grandchild with rotation and scale in play.
    fn sample_tree() -> (Hierarchy, ObjectId, ObjectId, ObjectId) {
        let mut tree = Hierarchy::new();
        let root = ObjectId::new();
        let child = ObjectId::new();
        let grand = ObjectId::new();
        tree.insert(root, ObjectId::NIL, pose([1.0, 0.0, 0.0]))
            .unwrap();
        tree.insert(
            child,
            root,
            Transform::new([0.0, 2.0, 0.0], quarter_turn_y(), [1.0, 2.0, 1.0]),
        )
        .unwrap();
        tree.insert(grand, child, pose([0.0, 0.0, 3.0])).unwrap();
        (tree, root, child, grand)
    }

    fn world_poses(tree: &Hierarchy) -> Vec<(ObjectId, Transform)> {
        tree.world_all()
    }

    #[test]
    fn roundtrip_preserves_ids_links_and_poses() {
        let (tree, root, child, grand) = sample_tree();
        let before_world = world_poses(&tree);

        // Save: hierarchy to versioned JSON text.
        let scene = Scene::encode(&tree);
        assert_eq!(scene.version, CURRENT_VERSION);
        let text = serde_json::to_string(&scene).unwrap();
        assert!(text.contains("\"version\":1"), "text stamps the version");

        // Reopen: text back to a hierarchy.
        let reopened: Scene = serde_json::from_str(&text).unwrap();
        assert_eq!(reopened, scene);
        let back = reopened.decode().unwrap();
        assert_eq!(back.validate(), Ok(()));

        assert_eq!(back.parent(root), Some(ObjectId::NIL));
        assert_eq!(back.parent(child), Some(root));
        assert_eq!(back.parent(grand), Some(child));
        for id in [root, child, grand] {
            assert_eq!(back.local(id), tree.local(id), "local pose moved for {id}");
            assert_eq!(back.world(id), tree.world(id), "world pose moved for {id}");
        }
        assert_eq!(world_poses(&back), before_world);
        assert_eq!(back.len(), 3);
    }

    #[test]
    fn encoding_is_deterministic_across_insertion_orders() {
        let (first, _, _, _) = sample_tree();
        // Same tree encodes to the same text on repeated saves.
        let text_a = serde_json::to_string(&Scene::encode(&first)).unwrap();
        let text_b = serde_json::to_string(&Scene::encode(&first)).unwrap();
        assert_eq!(text_a, text_b);

        // Sibling order never reaches the text: nodes print sorted by id.
        let mut tree = Hierarchy::new();
        let root = ObjectId::new();
        let left = ObjectId::new();
        let right = ObjectId::new();
        tree.insert(root, ObjectId::NIL, pose([0.0, 0.0, 0.0]))
            .unwrap();
        tree.insert(left, root, pose([1.0, 0.0, 0.0])).unwrap();
        tree.insert(right, root, pose([2.0, 0.0, 0.0])).unwrap();
        let text = serde_json::to_string(&Scene::encode(&tree)).unwrap();

        let mut other = Hierarchy::new();
        other
            .insert(root, ObjectId::NIL, pose([0.0, 0.0, 0.0]))
            .unwrap();
        other.insert(right, root, pose([2.0, 0.0, 0.0])).unwrap();
        other.insert(left, root, pose([1.0, 0.0, 0.0])).unwrap();
        assert_eq!(text, serde_json::to_string(&Scene::encode(&other)).unwrap());
    }

    #[test]
    fn decode_accepts_any_node_order() {
        let (tree, root, child, grand) = sample_tree();
        let mut scene = Scene::encode(&tree);
        scene.nodes.reverse();
        let back = scene.decode().unwrap();
        assert_eq!(back.validate(), Ok(()));
        for id in [root, child, grand] {
            assert_eq!(back.world(id), tree.world(id));
        }
    }

    #[test]
    fn migration_accepts_current_and_rejects_unsupported() {
        let (tree, _, _, _) = sample_tree();
        let mut scene = Scene::encode(&tree);
        scene.migrate().unwrap();
        assert_eq!(scene.version, CURRENT_VERSION);
        scene.decode().unwrap();

        for bad in [0, CURRENT_VERSION + 1, u32::MAX] {
            let mut old = Scene::encode(&tree);
            old.version = bad;
            let migrate_error = old.migrate().unwrap_err();
            assert_eq!(
                migrate_error,
                CodecError::UnsupportedVersion {
                    got: bad,
                    min: MIN_SUPPORTED_VERSION,
                    current: CURRENT_VERSION,
                }
            );
            // The rejected document is untouched, and decode refuses it too.
            assert_eq!(old.version, bad);
            assert_eq!(old.decode().unwrap_err(), migrate_error);
            let report = migrate_error.to_string();
            assert!(
                report.contains(&bad.to_string()),
                "report names the save version"
            );
            assert!(
                report.contains(&CURRENT_VERSION.to_string()),
                "report names what the reader supports"
            );
        }
    }

    #[test]
    fn missing_reference_names_culprit_and_leaves_nothing() {
        let (tree, _, _, _) = sample_tree();
        let mut scene = Scene::encode(&tree);
        let orphan = ObjectId::new();
        let ghost = ObjectId::new();
        assert!(!tree.contains(ghost));
        scene.nodes.push(SceneNode {
            id: orphan,
            parent: ghost,
            transform: pose([9.0, 9.0, 9.0]),
        });

        let error = scene.decode().unwrap_err();
        assert_eq!(
            error,
            CodecError::MissingParent {
                node: orphan,
                parent: ghost,
            }
        );
        // Diagnostic quotes both ends of the dangling link.
        let report = error.to_string();
        assert!(report.contains(&orphan.to_string()));
        assert!(report.contains(&ghost.to_string()));

        // Never partial: the only outcome is the error, and the valid save
        // behind it still decodes to the full tree.
        scene.nodes.pop();
        let back = scene.decode().unwrap();
        assert_eq!(back.len(), tree.len());
        assert_eq!(back.validate(), Ok(()));
    }

    #[test]
    fn duplicates_cycles_and_bad_nodes_are_rejected() {
        let (tree, root, child, _) = sample_tree();

        let mut doubled = Scene::encode(&tree);
        doubled.nodes.push(SceneNode {
            id: child,
            parent: root,
            transform: pose([0.0, 0.0, 0.0]),
        });
        assert_eq!(
            doubled.decode().unwrap_err(),
            CodecError::Duplicate { id: child }
        );

        // A two-node loop: every named parent exists, yet no order works.
        let a = ObjectId::new();
        let b = ObjectId::new();
        let looping = Scene {
            version: CURRENT_VERSION,
            nodes: vec![
                SceneNode {
                    id: a,
                    parent: b,
                    transform: Transform::identity(),
                },
                SceneNode {
                    id: b,
                    parent: a,
                    transform: Transform::identity(),
                },
            ],
        };
        assert!(matches!(
            looping.decode().unwrap_err(),
            CodecError::Cycle { .. }
        ));

        // A node parented to itself loops as well.
        let selfish = Scene {
            version: CURRENT_VERSION,
            nodes: vec![SceneNode {
                id: a,
                parent: a,
                transform: Transform::identity(),
            }],
        };
        assert_eq!(selfish.decode().unwrap_err(), CodecError::Cycle { node: a });

        // Nil ids and non-finite poses fail with their culprit named.
        let nilled = Scene {
            version: CURRENT_VERSION,
            nodes: vec![SceneNode {
                id: ObjectId::NIL,
                parent: ObjectId::NIL,
                transform: Transform::identity(),
            }],
        };
        assert_eq!(nilled.decode().unwrap_err(), CodecError::NilId);

        let mut unfinite = Scene::encode(&tree);
        unfinite.nodes[0].transform =
            Transform::new([f32::NAN, 0.0, 0.0], [0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0]);
        assert!(matches!(
            unfinite.decode().unwrap_err(),
            CodecError::NonFinite { .. }
        ));
    }

    #[test]
    fn empty_scene_roundtrips() {
        let scene = Scene::encode(&Hierarchy::new());
        assert_eq!(scene.version, CURRENT_VERSION);
        assert!(scene.nodes.is_empty());
        let text = serde_json::to_string(&scene).unwrap();
        let back: Scene = serde_json::from_str(&text).unwrap();
        let tree = back.decode().unwrap();
        assert!(tree.is_empty());
    }
}
