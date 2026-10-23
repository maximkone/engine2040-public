//! WG4: `prefab.save` and `prefab.place` — a live subtree becomes a prefab, and a prefab
//! becomes a journaled batch of inserts, so every placement undoes like any other edit.
//!
//! A prefab node keeps its authored pose relative to its parent and a key built from the
//! path under the saved root (`root`, `root/0`, `root/0/1`). `prefab.place` inserts the
//! nodes parents-first through the editor's own `commands::apply`, minting one fresh id per
//! node, so the batch lands in the journal and `tx.undo` takes it back with no prefab code.
//!
//! Overrides are per placement, keyed by prefab key: a root's pose moves by the placement
//! point, and any node's override replaces its local position exactly, so two placements of
//! one prefab differ where the caller says and only there.
//!
//! The store is this process's memory: a restart forgets every prefab (no file, no shell).
//! Errors name the fix in one word: `bad_root` (ask `scene.query` for an id), `unknown_prefab`
//! (run `prefab.save` first) and `prefab_failed` (re-run once the base revision is current).

use std::collections::BTreeMap;

use engine_editor::commands::{self, Batch, Document, Op};
use engine_scene::ids::{content_digest, ObjectId};
use engine_scene::prefab::{Prefab, PrefabNode};
use engine_scene::transform::Hierarchy;
use rmcp::schemars::JsonSchema;
use rmcp::serde::{Deserialize, Serialize};

use crate::mcp::{McpServer, McpToolError};
use crate::packs::bad_args;

/// Verb: save a live subtree as a prefab.
pub const TOOL_PREFAB_SAVE: &str = "prefab.save";
/// Verb: instantiate a saved prefab at a point, with per-instance overrides.
pub const TOOL_PREFAB_PLACE: &str = "prefab.place";
/// Most prefabs one process holds; a save past it is refused, never dropped.
pub const MAX_PREFABS: usize = 64;
/// Most nodes one prefab may hold.
pub const MAX_PREFAB_NODES: usize = 256;
/// Longest prefab name in chars.
pub const MAX_PREFAB_NAME_CHARS: usize = 64;

/// The process's prefabs by name. No file, no shell: a restart forgets them.
static PREFABS: std::sync::Mutex<BTreeMap<String, Prefab>> = std::sync::Mutex::new(BTreeMap::new());

/// Args for `prefab.save`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct PrefabSaveArgs {
    /// Root object id from `scene.query`.
    pub root: String,
    /// Name: letters, digits, `-_.`, at most 64 chars.
    pub name: String,
}

/// `prefab.save` result.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct PrefabSaveOutput {
    /// Stored name.
    pub name: String,
    /// Nodes captured, root first.
    pub nodes: u64,
    /// Digest of the captured poses.
    pub prefab_digest: String,
}

/// One per-instance override.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct PrefabOverride {
    /// Prefab key: `root`, `root/0`, `root/0/1`.
    pub key: String,
    /// Local position in meters.
    pub position: [f32; 3],
}

/// Args for `prefab.place`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct PrefabPlaceArgs {
    /// Prefab name from `prefab.save`.
    pub name: String,
    /// World position of the instance root, meters, y up.
    pub at: [f32; 3],
    /// Per-key position overrides.
    #[serde(default)]
    pub overrides: Vec<PrefabOverride>,
}

/// One placed node.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct PrefabPlacedNode {
    /// Prefab key.
    pub key: String,
    /// New object id.
    pub id: String,
}

/// `prefab.place` result.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct PrefabPlaceOutput {
    /// Revision after the batch landed.
    pub revision: u64,
    /// Content digest after the placement.
    pub digest: String,
    /// Inserted nodes, parents first.
    pub nodes: Vec<PrefabPlacedNode>,
}

/// Digest of a scene: the editor's revision-free content digest, as hex, so an edit
/// and its undo report one value for one scene.
fn scene_digest(doc: &Document) -> String {
    commands::document_digest(doc)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Digest of the captured nodes: keys, parents and pose bits, in capture order.
fn prefab_digest(prefab: &Prefab) -> String {
    let mut text = String::new();
    for node in &prefab.nodes {
        text.push_str(&node.key);
        text.push('|');
        text.push_str(node.parent.as_deref().unwrap_or(""));
        for value in node
            .transform
            .position
            .iter()
            .chain(node.transform.rotation.iter())
            .chain(node.transform.scale.iter())
        {
            text.push_str(&format!("|{:08x}", value.to_bits()));
        }
        text.push('\n');
    }
    content_digest(text.as_bytes())
}

/// True when `name` is a legal prefab name: 1..=64 chars of letters, digits and `-_.`.
fn legal_name(name: &str) -> bool {
    !name.is_empty()
        && name.chars().count() <= MAX_PREFAB_NAME_CHARS
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

/// Capture `root` and its descendants, parents first, keyed by path under the root.
fn capture(
    tree: &Hierarchy,
    root: ObjectId,
    key: &str,
    parent: Option<&str>,
    out: &mut Vec<PrefabNode>,
) -> Result<(), McpToolError> {
    if out.len() >= MAX_PREFAB_NODES {
        return Err(bad_args(format!(
            "the subtree holds more than {MAX_PREFAB_NODES} nodes; save a smaller subtree and place the pieces"
        )));
    }
    let transform = tree.local(root).ok_or_else(|| {
        McpToolError::new(
            "bad_root",
            format!(
                "prefab.save lost node `{key}` between the walk and the read; re-run with a current root"
            ),
        )
    })?;
    out.push(PrefabNode {
        key: key.to_owned(),
        parent: parent.map(str::to_owned),
        transform,
        components: Vec::new(),
    });
    for (index, child) in tree.children(root).into_iter().enumerate() {
        let child_key = format!("{key}/{index}");
        capture(tree, child, &child_key, Some(key), out)?;
    }
    Ok(())
}

/// `prefab.save` handler: snapshot the subtree under `root` into the process store.
fn prefab_save(
    service: &McpServer,
    args: &PrefabSaveArgs,
) -> Result<PrefabSaveOutput, McpToolError> {
    if !legal_name(&args.name) {
        return Err(bad_args(format!(
            "`{}` is not a prefab name; use 1..={MAX_PREFAB_NAME_CHARS} chars of letters, digits and - _ .",
            args.name
        )));
    }
    let root = ObjectId::parse(&args.root).map_err(|_| {
        bad_args(format!(
            "`{}` is not an object id; ask scene.query for the ids it lists",
            args.root
        ))
    })?;
    service.dispatch(TOOL_PREFAB_SAVE, |state| {
        if !state.doc.hierarchy.contains(root) {
            return Err(McpToolError::new(
                "bad_root",
                format!(
                    "`{}` is not a node in the live scene; ask scene.query for the ids it lists",
                    args.root
                ),
            ));
        }
        let mut nodes = Vec::new();
        capture(&state.doc.hierarchy, root, "root", None, &mut nodes)?;
        let prefab = Prefab {
            schema_version: engine_scene::prefab::PREFAB_VERSION,
            nodes,
        };
        let prefab_digest = prefab_digest(&prefab);
        let nodes = u64::try_from(prefab.nodes.len()).unwrap_or(u64::MAX);
        {
            let mut store = PREFABS.lock().map_err(|_| {
                McpToolError::new(
                    "prefab_store",
                    "the prefab store is poisoned; restart the door and save again",
                )
            })?;
            if !store.contains_key(&args.name) && store.len() >= MAX_PREFABS {
                return Err(bad_args(format!(
                    "the door already holds {MAX_PREFABS} prefabs; restart the door or reuse a name"
                )));
            }
            store.insert(args.name.clone(), prefab);
        }
        Ok(PrefabSaveOutput {
            name: args.name.clone(),
            nodes,
            prefab_digest,
        })
    })
}

/// The ops and ids one placement inserts, parents first.
fn place_ops(
    prefab: &Prefab,
    args: &PrefabPlaceArgs,
) -> Result<(Vec<Op>, Vec<PrefabPlacedNode>), McpToolError> {
    let mut overrides: BTreeMap<&str, [f32; 3]> = BTreeMap::new();
    for row in &args.overrides {
        if !prefab.nodes.iter().any(|node| node.key == row.key) {
            return Err(bad_args(format!(
                "`{}` is not a key in prefab `{}`; every node is named `root`, `root/0`, `root/0/1`",
                row.key, args.name
            )));
        }
        overrides.insert(row.key.as_str(), row.position);
    }
    if args.at.iter().any(|value| !value.is_finite())
        || overrides.values().flatten().any(|value| !value.is_finite())
    {
        return Err(bad_args(
            "`at` and every override position must be finite meters, y up".to_owned(),
        ));
    }
    let mut ids: BTreeMap<&str, ObjectId> = BTreeMap::new();
    let mut ops = Vec::with_capacity(prefab.nodes.len());
    let mut placed = Vec::with_capacity(prefab.nodes.len());
    // Repeated passes place parents before children whatever order the prefab lists them
    // in; a key whose parent never appears is refused, never left dangling.
    let mut pending: Vec<&PrefabNode> = prefab.nodes.iter().collect();
    while !pending.is_empty() {
        let mut progress = false;
        let mut rest = Vec::new();
        for node in pending {
            let parent = match node.parent.as_deref() {
                None => ObjectId::NIL,
                Some(key) => match ids.get(key) {
                    Some(id) => *id,
                    None => {
                        rest.push(node);
                        continue;
                    }
                },
            };
            let mut local = node.transform;
            if node.parent.is_none() {
                local.position = [
                    node.transform.position[0] + args.at[0],
                    node.transform.position[1] + args.at[1],
                    node.transform.position[2] + args.at[2],
                ];
            }
            if let Some(position) = overrides.get(node.key.as_str()) {
                local.position = *position;
            }
            let id = ObjectId::new();
            ids.insert(node.key.as_str(), id);
            placed.push(PrefabPlacedNode {
                key: node.key.clone(),
                id: id.as_uuid().to_string(),
            });
            ops.push(Op::Insert { id, parent, local });
            progress = true;
        }
        if !progress {
            let key = rest.first().map_or("root", |node| node.key.as_str());
            return Err(bad_args(format!(
                "prefab `{}` lists `{key}` under a parent it never declares; re-run prefab.save",
                args.name
            )));
        }
        pending = rest;
    }
    Ok((ops, placed))
}

/// `prefab.place` handler: instantiate the prefab as one journaled batch.
fn prefab_place(
    service: &McpServer,
    args: &PrefabPlaceArgs,
) -> Result<PrefabPlaceOutput, McpToolError> {
    service.dispatch(TOOL_PREFAB_PLACE, |state| {
        let prefab = {
            let store = PREFABS.lock().map_err(|_| {
                McpToolError::new(
                    "prefab_store",
                    "the prefab store is poisoned; restart the door and save again",
                )
            })?;
            store.get(&args.name).cloned().ok_or_else(|| {
                McpToolError::new(
                    "unknown_prefab",
                    format!(
                        "no prefab named `{}` is stored; run prefab.save first",
                        args.name
                    ),
                )
            })?
        };
        let (ops, nodes) = place_ops(&prefab, args)?;
        let batch = Batch {
            ops,
            base_revision: state.doc.revision,
            idempotency_key: 0x5052_4546_0000_0000 | state.doc.revision,
            provenance: None,
        };
        commands::apply(&mut state.doc, &mut state.journal, &batch).map_err(|error| {
            McpToolError::new(
                "prefab_failed",
                format!(
                    "prefab.place could not land on revision {}: {error}; re-run after tx.preview names a current base",
                    batch.base_revision
                ),
            )
        })?;
        Ok(PrefabPlaceOutput {
            revision: state.doc.revision,
            digest: scene_digest(&state.doc),
            nodes,
        })
    })
}

crate::e2_tool!(
    /// `prefab.save` tool: capture a live subtree as a named prefab.
    PrefabSaveTool,
    name = TOOL_PREFAB_SAVE,
    description = "Save the subtree under an object id as a named prefab. Returns its node count and digest.",
    readonly = false,
    args = PrefabSaveArgs,
    output = PrefabSaveOutput,
    invoke = prefab_save
);

crate::e2_tool!(
    /// `prefab.place` tool: instantiate a saved prefab as one journaled batch.
    PrefabPlaceTool,
    name = TOOL_PREFAB_PLACE,
    description = "Place a saved prefab as one undoable batch, with per-key position overrides. Returns revision, digest and the new ids.",
    readonly = false,
    args = PrefabPlaceArgs,
    output = PrefabPlaceOutput,
    invoke = prefab_place
);

#[cfg(test)]
mod tests {
    use super::*;
    use engine_editor::capability::Capability;
    use engine_editor::commands::Journal;
    use engine_scene::transform::Transform;

    use crate::mcp::ServerState;

    /// Root plus one child, as one applied batch; returns both ids.
    fn seeded() -> (Document, Journal, ObjectId, ObjectId) {
        let (mut doc, mut journal) = (Document::new(), Journal::new());
        let root = ObjectId::new();
        let batch = Batch {
            ops: vec![Op::Insert {
                id: root,
                parent: ObjectId::NIL,
                local: Transform::new([1.0, 0.0, 0.0], [0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0]),
            }],
            base_revision: 0,
            idempotency_key: 1,
            provenance: None,
        };
        commands::apply(&mut doc, &mut journal, &batch).expect("seed root applies");
        let child = ObjectId::new();
        let batch = Batch {
            ops: vec![Op::Insert {
                id: child,
                parent: root,
                local: Transform::new([0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0]),
            }],
            base_revision: doc.revision,
            idempotency_key: 2,
            provenance: None,
        };
        commands::apply(&mut doc, &mut journal, &batch).expect("seed child applies");
        (doc, journal, root, child)
    }

    fn door(doc: Document, journal: Journal) -> McpServer {
        McpServer::with_state(ServerState::with_document(
            doc,
            journal,
            7,
            Capability::unrestricted(7),
        ))
    }

    fn digest(door: &McpServer) -> String {
        let state = door.lock().expect("test lock holds");
        scene_digest(&state.doc)
    }

    #[test]
    fn save_place_twice_move_one_override_undo_both_keeps_the_digest() {
        let (doc, journal, root, _child) = seeded();
        let service = door(doc, journal);
        let before = digest(&service);
        let base_revision = service.lock().expect("test lock holds").doc.revision;
        let saved = prefab_save(
            &service,
            &PrefabSaveArgs {
                root: root.as_uuid().to_string(),
                name: "duo".to_owned(),
            },
        )
        .expect("the subtree saves");
        assert_eq!(saved.nodes, 2, "root plus its one child");

        let first = prefab_place(
            &service,
            &PrefabPlaceArgs {
                name: "duo".to_owned(),
                at: [10.0, 0.0, 0.0],
                overrides: Vec::new(),
            },
        )
        .expect("first placement lands");
        assert_eq!(first.nodes.len(), 2);
        assert_eq!(first.revision, base_revision + 1, "one batch per placement");
        let after_first = first.digest.clone();
        assert_ne!(after_first, before, "the placement moved the scene");

        // The second instance moves one child: same prefab, its own override.
        let child_key = "root/0".to_owned();
        let second = prefab_place(
            &service,
            &PrefabPlaceArgs {
                name: "duo".to_owned(),
                at: [20.0, 0.0, 0.0],
                overrides: vec![PrefabOverride {
                    key: child_key.clone(),
                    position: [0.0, 5.0, 0.0],
                }],
            },
        )
        .expect("second placement lands");
        assert_eq!(second.revision, base_revision + 2);
        assert_ne!(second.digest, after_first);
        {
            let state = service.lock().expect("test lock holds");
            let moved = second
                .nodes
                .iter()
                .find(|row| row.key == child_key)
                .expect("the override names a placed node");
            let id = ObjectId::parse(&moved.id).expect("the id parses");
            let local = state.doc.hierarchy.local(id).expect("the node exists");
            assert_eq!(local.position, [0.0, 5.0, 0.0], "the override holds");
            let first_child = first
                .nodes
                .iter()
                .find(|row| row.key == child_key)
                .expect("the first instance has the child");
            assert_ne!(first_child.id, moved.id, "two instances, two ids");
        }
        assert_eq!(
            digest(&service),
            second.digest,
            "the reported digest is the live scene"
        );

        // Move the second instance's child again through a plain edit, the way a person
        // would: one instance's override changes, the other's does not.
        {
            let mut state = service.lock().expect("test lock holds");
            let ServerState { doc, journal, .. } = &mut *state;
            let row = second
                .nodes
                .iter()
                .find(|row| row.key == child_key)
                .expect("the second instance has the child");
            let id = ObjectId::parse(&row.id).expect("the id parses");
            let batch = Batch {
                ops: vec![Op::SetLocal {
                    id,
                    local: Transform::new([9.0, 0.0, 0.0], [0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0]),
                }],
                base_revision: doc.revision,
                idempotency_key: 3,
                provenance: None,
            };
            commands::apply(doc, journal, &batch).expect("the move lands");
        }
        assert_ne!(
            digest(&service),
            second.digest,
            "the move changed the scene"
        );

        // Undo the move and both placements: the scene returns to where it stood.
        {
            let mut state = service.lock().expect("test lock holds");
            let ServerState { doc, journal, .. } = &mut *state;
            commands::undo(doc, journal).expect("the move undoes");
        }
        assert_eq!(digest(&service), second.digest, "the move is gone");
        {
            let mut state = service.lock().expect("test lock holds");
            let ServerState { doc, journal, .. } = &mut *state;
            commands::undo(doc, journal).expect("the second placement undoes");
        }
        assert_eq!(
            digest(&service),
            after_first,
            "undo takes the override back"
        );
        {
            let mut state = service.lock().expect("test lock holds");
            let ServerState { doc, journal, .. } = &mut *state;
            commands::undo(doc, journal).expect("the first placement undoes");
        }
        assert_eq!(digest(&service), before, "both placements are gone");
    }

    #[test]
    fn a_bad_root_an_unknown_prefab_and_a_bad_key_are_named() {
        let (doc, journal, root, _child) = seeded();
        let service = door(doc, journal);
        let error = prefab_save(
            &service,
            &PrefabSaveArgs {
                root: "not-an-id".to_owned(),
                name: "duo".to_owned(),
            },
        )
        .expect_err("an id has to parse");
        assert_eq!(error.code(), "bad_args");
        assert!(error.to_string().contains("scene.query"));

        let error = prefab_save(
            &service,
            &PrefabSaveArgs {
                root: ObjectId::new().as_uuid().to_string(),
                name: "duo".to_owned(),
            },
        )
        .expect_err("a missing node is refused");
        assert_eq!(error.code(), "bad_root");

        let error = prefab_save(
            &service,
            &PrefabSaveArgs {
                root: root.as_uuid().to_string(),
                name: "no spaces".to_owned(),
            },
        )
        .expect_err("a name rule holds");
        assert_eq!(error.code(), "bad_args");

        let error = prefab_place(
            &service,
            &PrefabPlaceArgs {
                name: "missing".to_owned(),
                at: [0.0; 3],
                overrides: Vec::new(),
            },
        )
        .expect_err("nothing saved under that name");
        assert_eq!(error.code(), "unknown_prefab");
        assert!(error.to_string().contains("prefab.save"));

        prefab_save(
            &service,
            &PrefabSaveArgs {
                root: root.as_uuid().to_string(),
                name: "duo".to_owned(),
            },
        )
        .expect("the subtree saves");
        let error = prefab_place(
            &service,
            &PrefabPlaceArgs {
                name: "duo".to_owned(),
                at: [0.0; 3],
                overrides: vec![PrefabOverride {
                    key: "root/9".to_owned(),
                    position: [0.0; 3],
                }],
            },
        )
        .expect_err("the key has to exist");
        assert_eq!(error.code(), "bad_args");
        assert!(error.to_string().contains("root/9"));
    }

    #[test]
    fn both_verbs_sit_in_the_scene_pack_with_a_commit_card() {
        let registry = crate::packs::PackRegistry::shipped();
        for name in [TOOL_PREFAB_SAVE, TOOL_PREFAB_PLACE] {
            let spec = registry.verb(name).expect("verb is registered");
            assert_eq!(spec.pack(), "scene");
            assert_eq!(
                spec.card(),
                crate::tool_capabilities::CapabilityCard::Commit
            );
        }
    }
}
