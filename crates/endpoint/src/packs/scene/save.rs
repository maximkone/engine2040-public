//! DOOR2 scene persistence: `scene.save` seals the live scene into the project
//! folder, `scene.load` reads the newest sealed save back into the session.
//!
//! Data owner: the editor's durable generations (`gen-{n:08}.edoc`). Each file
//! holds one sealed document plus its journal, so a reload keeps the
//! idempotency map and undo history. The door adds no format of its own.
//! `scene.save` numbers each new file one past the newest on disk, so the
//! newest file is always the last save, even after an undo moved the revision
//! back. Startup already reopens that newest generation; `scene.load` is the
//! in-session reload of the same pair.
//!
//! Digest: blake3 of the scene's save bytes (the bytes a generation holds), so
//! save and load report one value for one scene.
//!
//! Errors name the fix in one word: `no_project_dir` (start with
//! `--project-dir`), `no_saved_scene` (run `scene.save` first), `save_failed`
//! and `load_failed` (re-run once the folder is readable and writable).

use std::path::{Path, PathBuf};

use engine_editor::commands::{Document, Journal};
use engine_editor::durable::{
    generation_file_name, persist_sealed, rehydrate, reopen_sealed, TMP_SUFFIX,
};
use engine_editor::save::save_to_bytes;
use engine_scene::ids::content_digest;
use rmcp::schemars::JsonSchema;
use rmcp::serde::{Deserialize, Serialize};

use crate::mcp::{McpServer, McpToolError, ServerState};

/// Tool name: seal the live scene into the project folder.
pub const TOOL_SCENE_SAVE: &str = "scene.save";
/// Tool name: reload the newest sealed scene from the project folder.
pub const TOOL_SCENE_LOAD: &str = "scene.load";

/// `scene.save` args: none.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
#[expect(
    clippy::empty_structs_with_brackets,
    reason = "serde reads the wire shape `{}`; a unit struct reads null"
)]
pub struct SceneSaveArgs {}

/// `scene.load` args: none.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
#[expect(
    clippy::empty_structs_with_brackets,
    reason = "serde reads the wire shape `{}`; a unit struct reads null"
)]
pub struct SceneLoadArgs {}

// DTO fields carry no doc comments on purpose: their text lands in the
// discovery schema the door budgets (see packs/session.rs).

/// `scene.save` output.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct SceneSaveOutput {
    pub revision: u64,
    pub generation: u64,
    pub digest: String,
    pub file: String,
}

/// `scene.load` output.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct SceneLoadOutput {
    pub revision: u64,
    pub generation: u64,
    pub digest: String,
    pub objects: u64,
    pub skipped: u64,
}

/// The project folder, checked to exist as a directory.
fn project_folder(state: &ServerState) -> Result<PathBuf, McpToolError> {
    let dir = state.project_dir.clone().ok_or_else(|| {
        McpToolError::new(
            "no_project_dir",
            "this door has no project folder; start e2040-mcp with --project-dir <dir> \
             so scene.save has a folder to seal into (re-run after restarting)",
        )
    })?;
    if !dir.is_dir() {
        return Err(McpToolError::new(
            "no_project_dir",
            format!(
                "project folder {} is not a directory; create it or pass an existing \
                 --project-dir (re-run after fixing it)",
                dir.display()
            ),
        ));
    }
    Ok(dir)
}

/// One past the newest sealed generation in `dir` (1 for an empty folder).
/// In-flight `.tmp` leftovers never count.
fn next_generation(dir: &Path) -> Result<u64, McpToolError> {
    let entries = std::fs::read_dir(dir).map_err(|error| {
        McpToolError::new(
            "save_failed",
            format!(
                "cannot read the project folder {}: {error}; re-run after fixing the folder",
                dir.display()
            ),
        )
    })?;
    let mut newest = 0u64;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.ends_with(TMP_SUFFIX) {
            continue;
        }
        let number = name
            .strip_prefix("gen-")
            .and_then(|rest| rest.strip_suffix(".edoc"))
            .and_then(|stem| stem.parse::<u64>().ok());
        if let Some(number) = number {
            newest = newest.max(number);
        }
    }
    Ok(newest.saturating_add(1))
}

/// Digest of a scene: blake3 hex of the bytes a generation would hold.
fn scene_digest(doc: &Document) -> String {
    content_digest(&save_to_bytes(doc))
}

/// `scene.save` handler: seal the live document and journal as the next
/// generation in the project folder.
fn scene_save(service: &McpServer, _args: &SceneSaveArgs) -> Result<SceneSaveOutput, McpToolError> {
    service.dispatch(TOOL_SCENE_SAVE, |state| {
        let dir = project_folder(state)?;
        let generation = next_generation(&dir)?;
        let digest = scene_digest(&state.doc);
        persist_sealed(&dir, generation, &state.doc, &state.journal).map_err(|error| {
            McpToolError::new(
                "save_failed",
                format!(
                    "scene.save could not seal generation {generation} in {}: {error}; \
                     re-run after the folder is writable",
                    dir.display()
                ),
            )
        })?;
        Ok(SceneSaveOutput {
            revision: state.doc.revision,
            generation,
            digest,
            file: generation_file_name(generation),
        })
    })
}

/// `scene.load` handler: replace the live document and journal with the
/// newest sealed generation. Takes a snapshot so a later `scene.diff` starts
/// from the loaded revision.
fn scene_load(service: &McpServer, _args: &SceneLoadArgs) -> Result<SceneLoadOutput, McpToolError> {
    service.dispatch(TOOL_SCENE_LOAD, |state| {
        let dir = project_folder(state)?;
        let opened = reopen_sealed(&dir)
            .map_err(|error| {
                McpToolError::new(
                    "load_failed",
                    format!(
                        "cannot read the project folder {}: {error}; re-run after fixing the folder",
                        dir.display()
                    ),
                )
            })?
            .ok_or_else(|| {
                McpToolError::new(
                    "no_saved_scene",
                    format!(
                        "the project folder {} holds no sealed scene; re-run after scene.save seals one",
                        dir.display()
                    ),
                )
            })?;
        let skipped = u64::try_from(opened.skipped.len()).unwrap_or(u64::MAX);
        let mut doc = Document::new();
        let mut journal = Journal::new();
        let generation = rehydrate(&mut doc, &mut journal, opened);
        let out = SceneLoadOutput {
            revision: doc.revision,
            generation,
            digest: scene_digest(&doc),
            objects: u64::try_from(doc.hierarchy.len()).unwrap_or(u64::MAX),
            skipped,
        };
        state.rehydrate(doc, journal);
        state.take_snapshot();
        Ok(out)
    })
}

crate::e2_tool!(
    /// `scene.save` tool: seal the scene into the project folder.
    SceneSaveTool,
    name = TOOL_SCENE_SAVE,
    description = "Seal the scene and its undo history into the project folder as the next generation. Takes no arguments; returns revision, generation, digest and file.",
    readonly = false,
    args = SceneSaveArgs,
    output = SceneSaveOutput,
    invoke = scene_save
);

crate::e2_tool!(
    /// `scene.load` tool: reload the newest sealed scene from the project folder.
    SceneLoadTool,
    name = TOOL_SCENE_LOAD,
    description = "Reload the newest sealed scene from the project folder, replacing the live one. Takes no arguments; returns revision, generation, digest, objects and skipped.",
    readonly = false,
    args = SceneLoadArgs,
    output = SceneLoadOutput,
    invoke = scene_load
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::startup::{build_server, StartupConfig};
    use engine_editor::capability::Capability;
    use engine_editor::commands::{self, Batch, Journal, Op};
    use engine_scene::ids::ObjectId;
    use engine_scene::transform::Transform;

    /// Fresh scratch folder per test (unique by process and name).
    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("e2040-door2-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch dir creates");
        dir
    }

    /// One insert at `base` as its own batch.
    fn insert(doc: &mut Document, journal: &mut Journal, base: u64, key: u64) {
        commands::apply(
            doc,
            journal,
            &Batch {
                ops: vec![Op::Insert {
                    id: ObjectId::new(),
                    parent: ObjectId::NIL,
                    local: Transform::new([1.0, 0.0, 0.0], [0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0]),
                }],
                base_revision: base,
                idempotency_key: key,
                provenance: None,
            },
        )
        .expect("seed batch applies");
    }

    /// A door whose project folder is `dir`, over the given document.
    fn door_in(dir: &Path, doc: Document, journal: Journal) -> McpServer {
        let mut state = ServerState::with_document(doc, journal, 7, Capability::unrestricted(7));
        state.project_dir = Some(dir.to_path_buf());
        McpServer::with_state(state)
    }

    #[test]
    fn save_restart_load_keeps_revision_and_digest() {
        let dir = scratch("roundtrip");
        let (mut doc, mut journal) = (Document::new(), Journal::new());
        insert(&mut doc, &mut journal, 0, 11);
        insert(&mut doc, &mut journal, 1, 12);
        let live = door_in(&dir, doc, journal);
        let saved = scene_save(&live, &SceneSaveArgs::default()).expect("scene.save seals");
        assert_eq!(saved.revision, 2);
        assert_eq!(saved.generation, 1);
        assert_eq!(saved.file, "gen-00000001.edoc");

        // Restart: a fresh door reads the folder at startup, then scene.load
        // reads the save again. Both must land at the saved revision.
        let config = StartupConfig {
            project_dir: dir.clone(),
            policy: None,
            web: None,
        };
        let fresh = build_server(&config).expect("restart opens the folder");
        let loaded = scene_load(&fresh, &SceneLoadArgs::default()).expect("scene.load reads");
        assert_eq!(loaded.revision, saved.revision);
        assert_eq!(loaded.digest, saved.digest);
        assert_eq!(loaded.generation, saved.generation);
        assert_eq!(loaded.objects, 2);
        assert_eq!(loaded.skipped, 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn newest_save_wins_after_an_undo_and_load_keeps_undo() {
        let dir = scratch("newest");
        let (mut doc, mut journal) = (Document::new(), Journal::new());
        insert(&mut doc, &mut journal, 0, 21);
        insert(&mut doc, &mut journal, 1, 22);
        let door = door_in(&dir, doc, journal);
        let first = scene_save(&door, &SceneSaveArgs::default()).expect("first save");
        assert_eq!(first.revision, 2);

        // Undo and save again: revision only rises (undo is a new revision), so
        // the newest file must win by generation, not by an older revision.
        {
            let mut state = door.lock().expect("test lock holds");
            let ServerState { doc, journal, .. } = &mut *state;
            commands::undo(doc, journal).expect("undo reverts the second insert");
            assert_eq!(doc.revision, 3);
        }
        let second = scene_save(&door, &SceneSaveArgs::default()).expect("second save");
        assert_eq!(second.generation, 2);
        assert_ne!(second.digest, first.digest);

        let fresh = build_server(&StartupConfig {
            project_dir: dir.clone(),
            policy: None,
            web: None,
        })
        .expect("restart opens the folder");
        let loaded = scene_load(&fresh, &SceneLoadArgs::default()).expect("scene.load reads");
        assert_eq!(loaded.generation, 2);
        assert_eq!(loaded.digest, second.digest);

        // The loaded journal still undoes: the revision steps back again.
        {
            let mut state = fresh.lock().expect("test lock holds");
            let ServerState { doc, journal, .. } = &mut *state;
            assert_eq!(doc.revision, loaded.revision);
            commands::undo(doc, journal).expect("loaded undo works");
            assert_eq!(doc.revision, loaded.revision + 1);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_and_load_refuse_with_one_named_fix() {
        let bare = McpServer::new(3, Capability::unrestricted(3));
        let error = scene_save(&bare, &SceneSaveArgs::default()).expect_err("no folder");
        assert_eq!(error.code(), "no_project_dir");
        assert!(crate::tool_capabilities::message_carries_recovery(
            &error.to_string()
        ));

        let dir = scratch("empty");
        let door = door_in(&dir, Document::new(), Journal::new());
        let error = scene_load(&door, &SceneLoadArgs::default()).expect_err("nothing saved");
        assert_eq!(error.code(), "no_saved_scene");
        assert!(error.to_string().contains("scene.save"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn both_verbs_sit_in_the_scene_pack_with_a_commit_card() {
        let registry = crate::packs::PackRegistry::shipped();
        for name in [TOOL_SCENE_SAVE, TOOL_SCENE_LOAD] {
            let spec = registry.verb(name).expect("verb is registered");
            assert_eq!(spec.pack(), "scene");
            assert_eq!(
                spec.card(),
                crate::tool_capabilities::CapabilityCard::Commit
            );
        }
    }
}
