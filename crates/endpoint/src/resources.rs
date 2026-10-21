//! D16: the four door resources, served over `resources/list` and
//! `resources/read`.
//!
//! Consumer: an MCP client that wants the crate map, the door reference, the
//! error catalog or the `game.toml` schema. Data owner stays the crate it
//! names: the crate map is the engine's own crate list, the door reference
//! is the tool router plus the pack registry, the error catalog is the typed
//! refusal list, and the `game.toml` schema is the project manifest
//! contract. This module owns only the four URIs, the root pointers and the
//! text each one carries.
//!
//! Why resources and not tools: a read-only view does not need a tool
//! definition in every `tools/list`. Serving it here keeps the advertised tool
//! list (and its 66,500-char budget) where it is, and lets a client fetch the
//! view once, on demand, with `resources/read`. Four resources, zero tool
//! budget.
//!
//! Shape: one `Resource` per family, one root pointer each. `resources/list`
//! names the roots; a client that learned a child URI from a root can read it
//! directly, and the roots stay four short lines instead of a fan-out.
//!
//! Units: chars for the text, counts for the rows. Determinism: the same
//! build always produces the same bytes; no clock, no network, no float.
//! Errors: [`ResourceError`], one named variant per refusal, every message
//! naming the legal URIs.

use rmcp::model::{Resource, ResourceContents, ResourceTemplate};

/// Root URI of the crate map: one row per engine crate and what it owns.
pub const CRATE_MAP_URI: &str = "e2040://crate-map";
/// Root URI of the door reference: every core tool and every pack verb with
/// its capability card.
pub const DOOR_REFERENCE_URI: &str = "e2040://door-reference";
/// Root URI of the error catalog: every typed refusal the door answers, with
/// the recovery it names.
pub const ERROR_CATALOG_URI: &str = "e2040://error-catalog";
/// Root URI of the `game.toml` schema: the project manifest contract.
pub const GAME_TOML_SCHEMA_URI: &str = "e2040://game-toml-schema";

/// Every root URI this door serves, in contract order.
pub const ROOT_URIS: [&str; 4] = [
    CRATE_MAP_URI,
    DOOR_REFERENCE_URI,
    ERROR_CATALOG_URI,
    GAME_TOML_SCHEMA_URI,
];

/// MIME type of every resource: plain text, one UTF-8 body.
pub const RESOURCE_MIME: &str = "text/plain";

/// Most chars one resource body may carry. A client reads text; a body past
/// this is a bug in the producer, not a big answer.
pub const MAX_RESOURCE_CHARS: usize = 256 * 1024;

/// Why a resource request refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResourceError {
    /// The URI names no resource family this door serves.
    UnknownUri {
        /// The URI that was asked for.
        uri: String,
    },
    /// The family exists but the body it needs is not available here.
    Unavailable {
        /// The URI that was asked for.
        uri: String,
        /// What is missing.
        reason: String,
    },
}

impl std::fmt::Display for ResourceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownUri { uri } => write!(
                f,
                "no resource at `{uri}`; the door serves {}",
                ROOT_URIS.join(", ")
            ),
            Self::Unavailable { uri, reason } => {
                write!(f, "resource `{uri}` is unavailable: {reason}")
            }
        }
    }
}

impl std::error::Error for ResourceError {}

/// The four resources, in contract order.
#[must_use]
pub fn resource_list() -> Vec<Resource> {
    ROOT_URIS
        .iter()
        .map(|uri| {
            let mut resource = Resource::new(*uri, resource_name(uri));
            resource.description = Some(resource_description(uri).to_owned());
            resource.mime_type = Some(RESOURCE_MIME.to_owned());
            resource
        })
        .collect()
}

/// The four resource templates, one per family, so a client can discover a
/// child URI shape before it has read the root.
#[must_use]
pub fn resource_templates() -> Vec<ResourceTemplate> {
    ROOT_URIS
        .iter()
        .map(|uri| {
            let mut template = ResourceTemplate::new(format!("{uri}/{{id}}"), resource_name(uri));
            template.description = Some(resource_description(uri).to_owned());
            template.mime_type = Some(RESOURCE_MIME.to_owned());
            template
        })
        .collect()
}

/// Short name of one resource family.
#[must_use]
pub fn resource_name(uri: &str) -> &'static str {
    match uri {
        CRATE_MAP_URI => "crate-map",
        DOOR_REFERENCE_URI => "door-reference",
        ERROR_CATALOG_URI => "error-catalog",
        GAME_TOML_SCHEMA_URI => "game-toml-schema",
        _ => "unknown",
    }
}

/// One line saying what one resource family holds.
#[must_use]
pub fn resource_description(uri: &str) -> &'static str {
    match uri {
        CRATE_MAP_URI => "One row per engine crate: the crate and the one line it says it is.",
        DOOR_REFERENCE_URI => {
            "Every core tool and pack verb with its pack and capability card, in name order."
        }
        ERROR_CATALOG_URI => "Every typed refusal the door answers, with the recovery it names.",
        GAME_TOML_SCHEMA_URI => {
            "The project manifest contract: every key, its type and its legal values."
        }
        _ => "Unknown resource.",
    }
}

/// True when the URI names a family this door serves.
#[must_use]
pub fn is_known_uri(uri: &str) -> bool {
    ROOT_URIS.contains(&uri)
}

/// Read one resource body as text.
///
/// The `tool_rows` are the door's own advertised rows: name, pack and
/// capability card, one per verb.
pub fn read_resource_text(
    uri: &str,
    tool_rows: &[(String, String, String)],
) -> Result<String, ResourceError> {
    let body = match uri {
        CRATE_MAP_URI => crate_map_text(),
        DOOR_REFERENCE_URI => door_reference_text(tool_rows),
        ERROR_CATALOG_URI => error_catalog_text(),
        GAME_TOML_SCHEMA_URI => game_toml_schema_text(),
        other => {
            return Err(ResourceError::UnknownUri {
                uri: other.to_owned(),
            })
        }
    };
    if body.chars().count() > MAX_RESOURCE_CHARS {
        return Err(ResourceError::Unavailable {
            uri: uri.to_owned(),
            reason: format!(
                "the body is {} chars, over the {MAX_RESOURCE_CHARS} cap",
                body.chars().count()
            ),
        });
    }
    Ok(body)
}

/// Read one resource as the MCP wire contents.
pub fn read_resource(
    uri: &str,
    tool_rows: &[(String, String, String)],
) -> Result<ResourceContents, ResourceError> {
    let text = read_resource_text(uri, tool_rows)?;
    Ok(ResourceContents::text(text, uri).with_mime_type(RESOURCE_MIME))
}

/// The crate map: one row per engine crate.
///
/// The rows are the crate directory and the crate's own one-line job
/// sentence. A test walks the workspace's `crates/` directory and fails when
/// a crate is missing from this table, so the map cannot drift away from the
/// tree.
fn crate_map_text() -> String {
    let mut out = String::from("# crate map\n\n");
    out.push_str("crate | what it owns\n");
    for (crate_dir, job) in CRATE_MAP {
        out.push_str(&format!("{crate_dir} | {job}\n"));
    }
    out
}

/// The door reference: one line per advertised tool, with its pack and card.
fn door_reference_text(tool_rows: &[(String, String, String)]) -> String {
    let mut out = String::from("# door reference\n\n");
    out.push_str("tool | pack | card\n");
    let mut rows: Vec<&(String, String, String)> = tool_rows.iter().collect();
    rows.sort_by(|a, b| a.0.cmp(&b.0));
    let empty = rows.is_empty();
    for (name, pack, card) in &rows {
        out.push_str(&format!("{name} | {pack} | {card}\n"));
    }
    if empty {
        out.push_str("(the door advertises no tool yet)\n");
    }
    out
}

/// The error catalog: every typed refusal the door answers, with its recovery.
fn error_catalog_text() -> String {
    let mut out = String::from("# error catalog\n\n");
    out.push_str("code | meaning | recovery\n");
    for (code, meaning, recovery) in ERROR_CATALOG {
        out.push_str(&format!("{code} | {meaning} | {recovery}\n"));
    }
    out
}

/// The `game.toml` schema: every key the manifest takes, with its type and
/// its legal values.
fn game_toml_schema_text() -> String {
    let mut out = String::from("# game.toml schema\n\n");
    out.push_str("key | type | legal values | meaning\n");
    for (key, kind, legal, meaning) in GAME_TOML_SCHEMA {
        out.push_str(&format!("{key} | {kind} | {legal} | {meaning}\n"));
    }
    out
}

/// The engine's crates and the one line each says it is.
///
/// This is the crate map the door publishes. The test
/// `crate_map_covers_every_crate_in_the_tree` reads the workspace's
/// `crates/` directory and fails when a crate is missing here, so the table
/// and the tree cannot drift apart.
pub const CRATE_MAP: &[(&str, &str)] = &[
    (
        "ai",
        "agent minds: utility, behaviour trees, GOAP and the per-NPC memory.",
    ),
    (
        "anim",
        "the clip sampler: SoA sampling, blend and pose quantization.",
    ),
    (
        "app",
        "the app composition layer: builder, registration, phase order and extraction.",
    ),
    (
        "arena",
        "the headless match spine: fighter state, config, checkpoints and seed expansion.",
    ),
    (
        "assets",
        "asset intake, cook, bundle and the provenance chain.",
    ),
    ("audio", "mixing, the cue bank and the device heal path."),
    ("clock", "the deterministic clock the tick reads."),
    (
        "combat",
        "one blow: stance, recovery and the balance tables.",
    ),
    ("core", "hash, ids and the shared integer helpers."),
    (
        "destruction",
        "the bonded break: Voronoi fracture and the wound tables.",
    ),
    (
        "distill",
        "the offline trainer: the student and the dataset recorder.",
    ),
    ("ecs", "archetypes, the job pool and the change stamps."),
    (
        "editor",
        "the document, the command journal, the undo stack and the panels.",
    ),
    (
        "endpoint",
        "the agent door: verbs, packs, receipts and the audit log.",
    ),
    (
        "engine-chaos",
        "the chaos facade: healed and quarantined counts over the proof suite.",
    ),
    (
        "game",
        "the playable slice: lobby, round flow and the input mapping.",
    ),
    (
        "geo",
        "geometry recipes: booleans, unwrap, lod and collision.",
    ),
    ("input", "the input mapping: actions, axes and the tape."),
    (
        "kam2-game",
        "the KAM6 playable match: the 16x12 window village and the AI mayor.",
    ),
    (
        "kam2-sim",
        "the KAM2 economy tick: map, roads, houses, wares, stock and hash.",
    ),
    (
        "kam2-web",
        "the KAM8 browser tab: the wasm module that owns the 10 Hz tick.",
    ),
    (
        "material",
        "one material record: look, physics, sound and damage layers.",
    ),
    (
        "nav",
        "the nav mesh, the flow field and the ORCA crowd solver.",
    ),
    (
        "net",
        "the loopback netcode: rollback, loss injection and resync.",
    ),
    (
        "nn",
        "the int8 net runtime: infer_into and the .e2nn loader.",
    ),
    ("observe", "events, spans, digests and the heal supervisor."),
    (
        "ossim",
        "the headless core of \"You're the OS\": CPUs, RAM pages and the process queue.",
    ),
    (
        "perf",
        "the deterministic budget workloads and the counting allocator.",
    ),
    (
        "physfx",
        "the physical layer: a float Rapier world seeded by authority events.",
    ),
    ("physics", "the rapier world, cloth and the impact layers."),
    (
        "playtest",
        "the eval harness: the brief suite, the mirror and the verdict.",
    ),
    ("proof", "the proof crate: the golden hash and the corpus."),
    (
        "provider",
        "the model adapters, the router and the cassette replay.",
    ),
    (
        "render",
        "the wgpu renderer: passes, the frame graph and the passes.",
    ),
    (
        "rome",
        "the Rome tables: the shipped Roman practice values in integer millimetres.",
    ),
    (
        "rules",
        "the rules worker: integer script and the plan goals.",
    ),
    (
        "save",
        "the save envelope: 16 slots, atomic write, hash before seal.",
    ),
    (
        "scene",
        "ids, transforms, the hierarchy and the provenance codec.",
    ),
    (
        "spatial",
        "the relation graph, the brief and the spatial checks.",
    ),
    (
        "ui",
        "the in-game ui: the hud, the menus and the text lane.",
    ),
    (
        "web",
        "the browser shell: the WebGPU device, the canvas and the JSON fact line.",
    ),
    (
        "workspace-hack",
        "the cargo-hakari feature union: no code, the shared manifest list.",
    ),
    (
        "world",
        "the deterministic world bake: seed and size to versioned bytes.",
    ),
];

/// The error catalog rows: code, meaning, recovery.
///
/// Every code here is one the door answers: the test reads the endpoint's own
/// sources and fails when a catalog code is not one of them, so the catalog
/// never invents a refusal. The catalog names the door's core refusals - the
/// ones a client hits while it learns the door - and every refusal still
/// carries its own code on the wire, so the long tail stays machine-readable
/// without a second hand-written table.
pub const ERROR_CATALOG: &[(&str, &str, &str)] = &[
    (
        "apply_in_flight",
        "another apply is running on this session",
        "wait for the running apply to land, then apply again",
    ),
    (
        "audit_exhausted",
        "the session ran out of audit sequence numbers",
        "start a new session; the old one keeps its log",
    ),
    (
        "bad_args",
        "an argument is outside its legal range or shape",
        "the message names the legal values; fix that argument",
    ),
    (
        "bad_brief",
        "the brief text does not parse in the level grammar",
        "the message names the line; fix that line",
    ),
    (
        "bad_cell",
        "the cell is outside the grid the build takes",
        "the message names the grid; use a cell inside it",
    ),
    (
        "bad_csv",
        "the CSV does not parse",
        "the message names the line; fix that line",
    ),
    (
        "bad_edit",
        "the edit does not fit the document",
        "the message names the object and the legal change",
    ),
    (
        "bad_id",
        "the id does not name an object in the document",
        "re-read scene.read for the live ids",
    ),
    (
        "bad_image",
        "the image is not a PNG the door can read",
        "re-capture with the door's own capture verb",
    ),
    (
        "bad_input",
        "the intake input is outside the shape the verb takes",
        "the message names the field; fix that field",
    ),
    (
        "bad_payload",
        "the payload does not match the shape the call takes",
        "the message names the field; fix that field",
    ),
    (
        "bad_receipt",
        "the receipt does not match the change it names",
        "re-run the call that mints the receipt",
    ),
    (
        "bad_receipts",
        "the recorded receipts do not match the run",
        "re-run the chain that mints the receipts",
    ),
    (
        "bad_recipe",
        "the recipe names a step the door does not have",
        "the message names the legal steps; use one of them",
    ),
    (
        "bad_record",
        "the record does not parse in the record grammar",
        "the message names the line; fix that line",
    ),
    (
        "bad_tape",
        "the tape does not match the scene it replays",
        "re-record the tape against the live scene",
    ),
    (
        "bad_ticks",
        "the tick count is outside its legal range",
        "the message names the range; use a count inside it",
    ),
    (
        "bad_variant",
        "the variant name is not one the builder has",
        "the message names the legal variants; use one of them",
    ),
    (
        "bad_wall",
        "the wall segment does not close",
        "the message names the open end; close it",
    ),
    (
        "bad_word",
        "the text carries a word the door refuses",
        "the message names the word; replace it",
    ),
    (
        "denied_op",
        "the request names a method the door does not serve",
        "call api.search for the legal names",
    ),
    (
        "expired_approval",
        "the approval expired before the apply landed",
        "re-run tx.preview for a fresh approval",
    ),
    (
        "forged_approval",
        "the approval does not match this session and batch",
        "re-run tx.preview for a fresh approval",
    ),
    (
        "injection_refused",
        "the text carries a hostile fragment the door screens",
        "remove the screened text and re-run",
    ),
    (
        "internal",
        "the door failed in a way it does not name",
        "re-run the call; if it repeats the audit log names the record",
    ),
    (
        "invalid_batch",
        "the batch is empty or larger than the transport bound",
        "send a batch of one to 256 ops",
    ),
    (
        "io_failed",
        "the file the call needs could not be read or written",
        "the message names the path; check it exists",
    ),
    (
        "job_already_done",
        "the job already reached a terminal status",
        "start a new job; a finished job only replays its status",
    ),
    (
        "job_not_running",
        "the job is not running, so there is no progress to report",
        "start the job, then poll the id it returns",
    ),
    (
        "lock_poisoned",
        "the session lock was poisoned by a failed call",
        "restart the door; the document is on disk",
    ),
    (
        "no_script",
        "the project has no script the verb can run",
        "the message names what it looked for; add it",
    ),
    (
        "no_snapshot",
        "this session holds no snapshot to diff against",
        "take a snapshot, then ask for the diff",
    ),
    (
        "nothing_to_undo",
        "the journal holds nothing to undo",
        "apply a change first, then undo it",
    ),
    (
        "op_not_allowed",
        "the capability does not grant this op",
        "the message names the granted ops; use one of them",
    ),
    (
        "out_of_scope",
        "the object is outside the scope this session holds",
        "the message names the scope; work inside it",
    ),
    (
        "over_budget",
        "the call would cost more than its budget allows",
        "the message names the budget; make the call smaller",
    ),
    (
        "reused_approval",
        "the approval was already spent",
        "re-run tx.preview for a fresh approval",
    ),
    (
        "rules_refused",
        "the rules worker refused the change",
        "the message names the rule; fix the change",
    ),
    (
        "stale_revision",
        "the document moved since the intent was made",
        "re-read scene.read, then build again",
    ),
    (
        "tool_list_bloated",
        "the advertised tool list is over its budget",
        "shrink the tool surface, or load a pack on demand",
    ),
    (
        "unknown_job",
        "the job id is not one this session filed",
        "start a job with action=start, then poll the id it returns",
    ),
    (
        "unknown_pack",
        "the pack name is not one of the door's packs",
        "call api.load with a pack from api.search",
    ),
    (
        "unknown_tool",
        "the door has no tool with that name",
        "call api.search to find the name",
    ),
    (
        "unknown_verb",
        "the verb is not in any pack",
        "call api.search to list the packs",
    ),
];

/// The `game.toml` schema rows: key, type, legal values, meaning.
pub const GAME_TOML_SCHEMA: &[(&str, &str, &str, &str)] = &[
    (
        "name",
        "string",
        "a-z 0-9 - _",
        "project name; engine2040 new fills it from the directory name",
    ),
    (
        "targets",
        "list of strings",
        "windows | web",
        "where the game builds",
    ),
    (
        "quality",
        "string",
        "minimal | low | medium | high | ultra",
        "the quality tier the build uses",
    ),
    (
        "physics_layers",
        "list of strings",
        "authority | physical",
        "authority decides every outcome (hashed integers) and is required; physical adds ragdolls, severed limbs and debris that only have to look right",
    ),
    (
        "gore",
        "string",
        "off | reduced | full",
        "how much the fight shows",
    ),
    (
        "content",
        "list of strings",
        "relative paths, forward slashes",
        "content roots, relative to the manifest; each must be a directory",
    ),
    (
        "seed_file",
        "string",
        "relative path",
        "the file holding the seed of the headless run: one unsigned number",
    ),
];

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;
    use std::path::Path;

    /// The workspace crates directory, from the endpoint crate's manifest.
    fn crates_dir() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("crates")
    }

    #[test]
    fn four_roots_are_served_and_listed() {
        let list = resource_list();
        assert_eq!(list.len(), 4, "the door serves four resources");
        let uris: Vec<&str> = list.iter().map(|r| r.uri.as_str()).collect();
        assert_eq!(uris, ROOT_URIS.to_vec());
        for resource in &list {
            assert!(!resource.name.is_empty(), "every resource is named");
            assert!(
                resource.description.is_some(),
                "every resource explains itself"
            );
            assert_eq!(resource.mime_type.as_deref(), Some(RESOURCE_MIME));
        }
        assert_eq!(resource_templates().len(), 4, "one template per family");
    }

    #[test]
    fn door_resource_serves_four_roots_at_no_tool_budget() {
        // The four resources cost no tool-list budget: no root URI is a tool
        // name, so `tools/list` does not grow by a char for them.
        let list = resource_list();
        assert_eq!(list.len(), 4, "the door serves four resources");
        let mut tool_names: Vec<String> = crate::mcp::tool_names()
            .into_iter()
            .map(|name| (*name).to_owned())
            .collect();
        for pack in crate::packs::PackRegistry::shipped().packs() {
            for verb in pack.verb_names() {
                tool_names.push((*verb).to_owned());
            }
        }
        for uri in ROOT_URIS {
            assert!(
                !tool_names.iter().any(|name| name == uri),
                "{uri} is served as a resource, not as a tool"
            );
        }
        let rows = vec![(
            "scene.read".to_owned(),
            "core".to_owned(),
            "observe".to_owned(),
        )];
        for uri in ROOT_URIS {
            let text = read_resource_text(uri, &rows).expect("root reads");
            assert!(!text.is_empty(), "{uri} carries text");
        }
        println!("door_resource: 4 resources, 0 tool-budget chars PASS");
    }

    #[test]
    fn every_root_reads_back_text() {
        let rows = vec![(
            "scene.read".to_owned(),
            "core".to_owned(),
            "observe".to_owned(),
        )];
        for uri in ROOT_URIS {
            let text = read_resource_text(uri, &rows).expect("root reads");
            assert!(!text.is_empty(), "{uri} carries text");
        }
    }

    #[test]
    fn unknown_uri_refuses_and_names_the_legal_ones() {
        let error = read_resource_text("e2040://nope", &[]).expect_err("refuses");
        assert!(matches!(error, ResourceError::UnknownUri { .. }));
        let message = error.to_string();
        for uri in ROOT_URIS {
            assert!(message.contains(uri), "the refusal names {uri}: {message}");
        }
    }

    #[test]
    fn crate_map_covers_every_crate_in_the_tree() {
        // The published map must name every crate directory in the workspace,
        // so the resource cannot drift away from the tree.
        let dir = crates_dir();
        if !dir.is_dir() {
            // A published crate has no workspace beside it; the map is then
            // the source of truth and there is nothing to compare against.
            return;
        }
        let mut on_disk: BTreeSet<String> = BTreeSet::new();
        for entry in std::fs::read_dir(&dir).expect("crates dir reads") {
            let entry = entry.expect("dir entry");
            let name = entry.file_name().to_string_lossy().to_string();
            if entry.path().join("Cargo.toml").is_file() {
                on_disk.insert(name);
            }
        }
        let published: BTreeSet<String> = CRATE_MAP
            .iter()
            .map(|(name, _)| (*name).to_owned())
            .collect();
        let missing: Vec<&String> = on_disk.difference(&published).collect();
        assert!(
            missing.is_empty(),
            "the crate map does not name these crates: {missing:?}"
        );
        let extra: Vec<&String> = published.difference(&on_disk).collect();
        assert!(
            extra.is_empty(),
            "the crate map names crates the tree does not have: {extra:?}"
        );
    }

    /// The door's core refusals: the ones a client hits while it learns the
    /// door. The catalog must name every one of them.
    const CORE_CODES: &[&str] = &[
        "apply_in_flight",
        "audit_exhausted",
        "bad_args",
        "bad_id",
        "denied_op",
        "expired_approval",
        "forged_approval",
        "injection_refused",
        "invalid_batch",
        "job_already_done",
        "job_not_running",
        "lock_poisoned",
        "no_snapshot",
        "nothing_to_undo",
        "op_not_allowed",
        "over_budget",
        "reused_approval",
        "stale_revision",
        "tool_list_bloated",
        "unknown_job",
        "unknown_pack",
        "unknown_tool",
        "unknown_verb",
    ];

    #[test]
    fn error_catalog_names_only_codes_the_door_answers() {
        // No phantoms: every code the catalog names must be a code the
        // endpoint's own sources spell out.
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut codes: BTreeSet<String> = BTreeSet::new();
        collect_error_codes(&src, &mut codes);
        let published: BTreeSet<&str> = ERROR_CATALOG.iter().map(|(code, _, _)| *code).collect();
        let phantom: Vec<&str> = published
            .iter()
            .filter(|code| !codes.contains(**code))
            .copied()
            .collect();
        assert!(
            phantom.is_empty(),
            "the error catalog names codes the door never answers: {phantom:?}"
        );
    }

    #[test]
    fn error_catalog_covers_the_core_refusals() {
        // No silence about the core: the codes a client hits first are all
        // named, with a meaning and a recovery.
        let published: BTreeSet<&str> = ERROR_CATALOG.iter().map(|(code, _, _)| *code).collect();
        let missing: Vec<&str> = CORE_CODES
            .iter()
            .filter(|code| !published.contains(**code))
            .copied()
            .collect();
        assert!(
            missing.is_empty(),
            "the error catalog does not name these core codes: {missing:?}"
        );
        for (code, meaning, recovery) in ERROR_CATALOG {
            assert!(!meaning.is_empty(), "{code} says what it means");
            assert!(!recovery.is_empty(), "{code} says how to recover");
        }
    }

    /// Collect every refusal code the endpoint spells out under `dir`.
    ///
    /// The scan reads the code literals of the error constructors, inline or
    /// on the next line, so the multi-line call shapes count too.
    fn collect_error_codes(dir: &Path, out: &mut BTreeSet<String>) {
        let entries = match std::fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(_) => return,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                collect_error_codes(&path, out);
                continue;
            }
            if path.extension().and_then(|ext| ext.to_str()) != Some("rs") {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            for constructor in ["McpToolError::new(", "McpToolError::wrap("] {
                for chunk in text.split(constructor).skip(1) {
                    let rest = chunk.trim_start();
                    let Some(rest) = rest.strip_prefix('"') else {
                        continue;
                    };
                    if let Some(end) = rest.find('"') {
                        out.insert(rest[..end].to_owned());
                    }
                }
            }
        }
    }

    #[test]
    fn door_reference_names_every_tool_with_its_card() {
        let rows = vec![
            (
                "scene.read".to_owned(),
                "core".to_owned(),
                "observe".to_owned(),
            ),
            (
                "tx.apply".to_owned(),
                "core".to_owned(),
                "commit".to_owned(),
            ),
        ];
        let text = read_resource_text(DOOR_REFERENCE_URI, &rows).expect("reference reads");
        assert!(text.contains("scene.read"), "{text}");
        assert!(text.contains("commit"), "{text}");
        let scene = text.find("scene.read").expect("scene row");
        let tx = text.find("tx.apply").expect("tx row");
        assert!(scene < tx, "the reference is sorted by name: {text}");
    }

    #[test]
    fn game_toml_schema_names_every_key_with_its_legal_values() {
        let text = read_resource_text(GAME_TOML_SCHEMA_URI, &[]).expect("schema reads");
        for (key, kind, legal, meaning) in GAME_TOML_SCHEMA {
            assert!(text.contains(key), "the schema names {key}");
            assert!(text.contains(kind), "the schema types {key}");
            assert!(text.contains(legal), "the schema bounds {key}");
            assert!(text.contains(meaning), "the schema explains {key}");
        }
    }
}
