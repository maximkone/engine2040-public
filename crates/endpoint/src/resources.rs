//! D16: the four static door resources plus the live scene view and the
//! lesson store, served over `resources/list` and `resources/read`.
//!
//! Consumer: an MCP client that wants the crate map, the door reference, the
//! error catalog, the `game.toml` schema, the live scene or the lessons a
//! build left behind. Data owner stays the crate it names: the crate map is
//! the engine's own crate list, the door reference is the tool router plus the
//! pack registry, the error catalog is the typed refusal list, the `game.toml`
//! schema is the project manifest contract, the scene view is the session
//! document, and the lesson rows are the JSONL store
//! `tools/memory/memory.mjs` writes, read and ordered through
//! [`crate::memory_store`] (BG14). This module owns only the URIs, the root
//! pointers and the text each one carries.
//!
//! Why resources and not tools: a read-only view does not need a tool
//! definition in every `tools/list`. Serving it here keeps the advertised tool
//! list (and its 66,500-char budget) where it is, and lets a client fetch the
//! view once, on demand, with `resources/read`. Four resources, zero tool
//! budget.
//!
//! Shape: one `Resource` per family, one root pointer each. `resources/list`
//! names the roots; a client that learned a child URI from a root can read it
//! directly, and the roots stay four short lines instead of a fan-out. A
//! child URI (`<root>/<id>`) reads the one row its id names, so a template a
//! client learned from `resources/templates/list` is never a dead end.
//!
//! Units: chars for the text, counts for the rows. Determinism: the same
//! build always produces the same bytes; no clock, no network, no float. The
//! door reference carries a pin line (tool count and the FNV-64 of the rows
//! below it) computed at render time, so a reader can see the surface moved
//! without a version number to trust. The lesson view is the one body that
//! reads a file: the same store always renders the same rows, in stamp order,
//! and a store that is not there renders no row.
//! Errors: [`ResourceError`], one named variant per refusal, every message
//! naming the legal URIs.

use engine_core::hash::hash_hex;
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
/// URI of the live scene view: the session document's revision, object count
/// and content digest. Unlike the four static families this body is read from
/// the session, so it is served beside them, not through [`ROOT_URIS`].
pub const SCENE_URI: &str = "e2040://scene";
/// Root URI of the lesson store: the rows `tools/memory/memory.mjs` keeps, in
/// stamp order. `?path=`, `?kind=` and `?hash=` filter it, the same three the
/// tool's `query` takes (BG14).
pub const MEMORY_URI: &str = "e2040://memory";
/// The filter keys one lesson read takes, in contract order.
pub const MEMORY_FILTER_KEYS: [&str; 3] = ["kind", "path", "hash"];

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

/// URI root of the receipts a verb publishes by reference: `<root>/<id>`.
/// A receipt row is one full body a verb answered with a short summary of, so
/// a client fetches the whole thing with `resources/read` instead of asking
/// again with narrower arguments (OPT58).
pub const RECEIPT_URI_ROOT: &str = "e2040://receipt";

/// Most receipt rows one session keeps. The oldest drops first, so a bounded
/// table of answers never grows without limit; a dropped row reads as absent
/// and the verb answers with the summary alone.
pub const RECEIPT_ROWS: usize = 8;

/// One bounded table of the full bodies a verb answered with a summary.
///
/// The row id is the FNV-64 of the body, so one body always publishes the same
/// URI and a read is byte-identical every time. No clock and no address enters
/// the id, so the same call publishes the same row on every run.
#[derive(Debug, Clone, Default)]
pub struct ReceiptStore {
    rows: Vec<(String, String)>,
}

impl ReceiptStore {
    /// Empty table.
    #[must_use]
    pub fn new() -> Self {
        Self { rows: Vec::new() }
    }

    /// Publish one full body and return the URI that reads it back.
    pub fn publish(&mut self, body: &str) -> String {
        let uri = format!("{RECEIPT_URI_ROOT}/{}", hash_hex(body.as_bytes()));
        if let Some((_, held)) = self.rows.iter_mut().find(|(known, _)| *known == uri) {
            held.clear();
            held.push_str(body);
            return uri;
        }
        if self.rows.len() >= RECEIPT_ROWS {
            self.rows.remove(0);
        }
        self.rows.push((uri.clone(), body.to_owned()));
        uri
    }

    /// The body published under `uri`, if this session still holds it.
    #[must_use]
    pub fn read(&self, uri: &str) -> Option<&str> {
        self.rows
            .iter()
            .find(|(known, _)| known == uri)
            .map(|(_, body)| body.as_str())
    }

    /// Rows held.
    #[must_use]
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    /// True when no receipt is held.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }
}

/// True when the URI names a receipt row, not one of the static families.
#[must_use]
pub fn is_receipt_uri(uri: &str) -> bool {
    uri.strip_prefix(RECEIPT_URI_ROOT)
        .is_some_and(|rest| rest.len() > 1 && rest.starts_with('/'))
}

/// The wire contents of one receipt body.
#[must_use]
pub fn receipt_contents(uri: &str, text: String) -> ResourceContents {
    ResourceContents::text(text, uri).with_mime_type(RESOURCE_MIME)
}

impl crate::mcp::McpServer {
    /// Read one receipt body by URI: the fetch side of the `resource_uri` a
    /// verb prints beside its summary. `None` means this session never
    /// published it, or its row already dropped from the bounded table.
    #[must_use]
    pub fn receipt_text(&self, uri: &str) -> Option<String> {
        if !is_receipt_uri(uri) {
            return None;
        }
        let state = self.lock_read().ok()?;
        state.receipt_text(uri).map(str::to_owned)
    }
}

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

/// The resources the door lists: the four static families in contract order,
/// then the lesson view. The live scene view rides beside them, pushed by the
/// door itself because its body comes from the session.
#[must_use]
pub fn resource_list() -> Vec<Resource> {
    let mut listed: Vec<Resource> = ROOT_URIS
        .iter()
        .map(|uri| {
            let mut resource = Resource::new(*uri, resource_name(uri));
            resource.description = Some(resource_description(uri).to_owned());
            resource.mime_type = Some(RESOURCE_MIME.to_owned());
            resource
        })
        .collect();
    listed.push(memory_resource());
    listed
}

/// The resource templates: one per static family, so a client can discover a
/// child URI shape before it has read the root, plus the lesson view's query
/// shape.
#[must_use]
pub fn resource_templates() -> Vec<ResourceTemplate> {
    let mut listed: Vec<ResourceTemplate> = ROOT_URIS
        .iter()
        .map(|uri| {
            let mut template = ResourceTemplate::new(format!("{uri}/{{id}}"), resource_name(uri));
            template.description = Some(resource_description(uri).to_owned());
            template.mime_type = Some(RESOURCE_MIME.to_owned());
            template
        })
        .collect();
    listed.push(memory_template());
    listed
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

/// The live scene view, listed beside the four static families. It has no
/// child URIs and no template: its body is the session document itself.
#[must_use]
pub fn scene_resource() -> Resource {
    let mut resource = Resource::new(SCENE_URI, "scene");
    resource.description = Some(
        "The live scene: its revision, object count and the content digest a `scene.save` \
         would print; a subscribed client gets one update when the digest moves."
            .to_owned(),
    );
    resource.mime_type = Some(RESOURCE_MIME.to_owned());
    resource
}

/// Render the live scene view.
///
/// The same revision, object count and digest always render the same bytes, so
/// a reader can compare two views and the push channel can tell when the
/// document moved without a clock.
#[must_use]
pub fn scene_text(revision: u64, objects: usize, digest: &str) -> String {
    format!("# scene\n\nrevision: {revision}\nobjects: {objects}\ndigest: {digest}\n")
}

/// The wire contents of the live scene view.
#[must_use]
pub fn scene_contents(text: String) -> ResourceContents {
    ResourceContents::text(text, SCENE_URI).with_mime_type(RESOURCE_MIME)
}

/// The lesson view, listed beside the four static families. Its body is one
/// JSONL file the memory tool already writes, so the door adds no store of its
/// own; a client filters it with the query keys in [`MEMORY_FILTER_KEYS`].
#[must_use]
pub fn memory_resource() -> Resource {
    let mut resource = Resource::new(MEMORY_URI, "memory");
    resource.description = Some(
        "The lessons a build left behind: one row per decision, failed fix or tuned \
         value, each carrying the tape hash it was learned under, in UTC-stamp order. \
         `?kind=`, `?path=` and `?hash=` filter it."
            .to_owned(),
    );
    resource.mime_type = Some(RESOURCE_MIME.to_owned());
    resource
}

/// The lesson view's template, so a client can learn the query shape before it
/// reads a row.
#[must_use]
pub fn memory_template() -> ResourceTemplate {
    let mut template = ResourceTemplate::new(format!("{MEMORY_URI}{{?kind,path,hash}}"), "memory");
    template.description = Some(
        "One lesson store, three filters: `kind` (exact), `path` (a substring of the \
         source path the lesson is about) and `hash` (the 16 hex digits of the tape it \
         was learned under)."
            .to_owned(),
    );
    template.mime_type = Some(RESOURCE_MIME.to_owned());
    template
}

/// True when the URI names a lesson read: the root itself or the root with a
/// `?` query. A `/` child is not a lesson URI; the lesson view has no children.
#[must_use]
pub fn is_memory_uri(uri: &str) -> bool {
    uri.strip_prefix(MEMORY_URI)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with('?'))
}

/// The filter one lesson URI asks for.
///
/// `e2040://memory` alone asks for every row. A query takes the three keys of
/// [`MEMORY_FILTER_KEYS`], each at most once, none empty.
///
/// # Errors
/// [`ResourceError::UnknownUri`] for a URI that is not a lesson URI, and
/// [`ResourceError::Unavailable`] for a key the view does not take, a key given
/// twice, an empty value, a kind that is not a lesson kind or a hash that is
/// not 16 hex digits.
pub fn memory_filter(uri: &str) -> Result<crate::memory_store::Filter, ResourceError> {
    let refusal = |reason: String| ResourceError::Unavailable {
        uri: uri.to_owned(),
        reason,
    };
    let rest = uri
        .strip_prefix(MEMORY_URI)
        .ok_or_else(|| ResourceError::UnknownUri {
            uri: uri.to_owned(),
        })?;
    let query = match rest.as_bytes().first() {
        None => return Ok(crate::memory_store::Filter::default()),
        Some(b'?') => &rest[1..],
        Some(_) => {
            return Err(ResourceError::UnknownUri {
                uri: uri.to_owned(),
            })
        }
    };
    let mut filter = crate::memory_store::Filter::default();
    let mut given: Vec<String> = Vec::new();
    for pair in query.split('&').filter(|pair| !pair.is_empty()) {
        let (key, value) = pair.split_once('=').ok_or_else(|| {
            refusal(format!(
                "`{pair}` carries no `=`; the keys are kind, path, hash"
            ))
        })?;
        if !MEMORY_FILTER_KEYS.contains(&key) {
            return Err(refusal(format!(
                "`{key}` is not a lesson filter; the keys are {}",
                MEMORY_FILTER_KEYS.join(", ")
            )));
        }
        if given.contains(&key.to_owned()) {
            return Err(refusal(format!("`{key}` is given twice")));
        }
        given.push(key.to_owned());
        if value.is_empty() {
            return Err(refusal(format!("`{key}` has no value")));
        }
        match key {
            "kind" => {
                if !crate::memory_store::is_kind(value) {
                    return Err(refusal(format!(
                        "`{value}` is not a lesson kind; legal kinds are {}",
                        crate::memory_store::KINDS.join(", ")
                    )));
                }
                filter.kind = Some(value.to_owned());
            }
            "path" => filter.path = Some(value.replace('\\', "/")),
            "hash" => {
                filter.hash = Some(
                    crate::memory_store::normalize_hash(value)
                        .map_err(|error| refusal(error.to_string()))?,
                );
            }
            _ => {}
        }
    }
    Ok(filter)
}

/// Render the lesson view: the rows the filter keeps, in the order they were
/// given (stamp order from [`crate::memory_store::stamp_order`]), one canonical
/// JSON line each — the same bytes `memory.mjs query --json` prints.
#[must_use]
pub fn memory_body(
    rows: &[crate::memory_store::Record],
    filter: &crate::memory_store::Filter,
) -> String {
    let kept = filter.select(rows);
    let mut out = String::from("# memory\n\n");
    out.push_str(&format!(
        "rows: {} of {} | filter: {}\n\n",
        kept.len(),
        rows.len(),
        filter.describe()
    ));
    for row in &kept {
        out.push_str(&row.line());
        out.push('\n');
    }
    if kept.is_empty() {
        out.push_str("(no lesson row matches)\n");
    }
    out
}

/// Read one resource body as text.
///
/// The `tool_rows` are the door's own advertised rows: name, pack and
/// capability card, one per verb. The door reference pins the rows it
/// rendered, so a reader can tell two surfaces apart by the fingerprint.
pub fn read_resource_text(
    uri: &str,
    tool_rows: &[(String, String, String)],
) -> Result<String, ResourceError> {
    let body = if is_memory_uri(uri) {
        // BG14: the lesson rows the memory tool keeps, filtered by the query
        // the URI carries. A store that is not there renders no row.
        let filter = memory_filter(uri)?;
        let rows = crate::memory_store::read_ordered(&crate::memory_store::store_path()).map_err(
            |error| ResourceError::Unavailable {
                uri: uri.to_owned(),
                reason: error.to_string(),
            },
        )?;
        memory_body(&rows, &filter)
    } else if let Some((root, id)) = split_child(uri) {
        child_text(root, id, tool_rows).ok_or_else(|| ResourceError::UnknownUri {
            uri: uri.to_owned(),
        })?
    } else {
        match uri {
            CRATE_MAP_URI => crate_map_text(),
            DOOR_REFERENCE_URI => door_reference_text(tool_rows),
            ERROR_CATALOG_URI => error_catalog_text(),
            GAME_TOML_SCHEMA_URI => game_toml_schema_text(),
            other => {
                return Err(ResourceError::UnknownUri {
                    uri: other.to_owned(),
                })
            }
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

/// Split `<root>/<id>` into the root it belongs to and the child id.
///
/// Returns `None` for a root URI itself and for any URI no root prefixes.
fn split_child(uri: &str) -> Option<(&'static str, &str)> {
    ROOT_URIS.iter().find_map(|root| {
        uri.strip_prefix(*root)
            .and_then(|rest| rest.strip_prefix('/'))
            .filter(|id| !id.is_empty())
            .map(|id| (*root, id))
    })
}

/// One child row of one family, under the same header its root carries.
///
/// `None` means the id names no row in that family; the caller turns that
/// into the same `UnknownUri` a bad root gets.
fn child_text(root: &str, id: &str, tool_rows: &[(String, String, String)]) -> Option<String> {
    match root {
        CRATE_MAP_URI => {
            let (crate_dir, job) = CRATE_MAP.iter().find(|(name, _)| *name == id)?;
            Some(format!(
                "# crate map: {crate_dir}\n\ncrate | what it owns\n{crate_dir} | {job}\n"
            ))
        }
        DOOR_REFERENCE_URI => {
            let (name, pack, card) = tool_rows.iter().find(|(name, _, _)| name == id)?;
            Some(format!(
                "# door reference: {name}\n\ntool | pack | card\n{name} | {pack} | {card}\n"
            ))
        }
        ERROR_CATALOG_URI => {
            let (code, meaning, recovery) =
                ERROR_CATALOG.iter().find(|(code, _, _)| *code == id)?;
            Some(format!(
                "# error catalog: {code}\n\ncode | meaning | recovery\n{code} | {meaning} | {recovery}\n"
            ))
        }
        GAME_TOML_SCHEMA_URI => {
            let (key, kind, legal, meaning) =
                GAME_TOML_SCHEMA.iter().find(|(key, _, _, _)| *key == id)?;
            Some(format!(
                "# game.toml schema: {key}\n\nkey | type | legal values | meaning\n{key} | {kind} | {legal} | {meaning}\n"
            ))
        }
        _ => None,
    }
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

/// The door reference: one line per advertised tool, with its pack and card,
/// under a pin line that names the tool count and the schema fingerprint.
fn door_reference_text(tool_rows: &[(String, String, String)]) -> String {
    let body = door_reference_body(tool_rows);
    let pin = door_reference_pin(tool_rows.len(), &body);
    format!("# door reference\n\n{pin}\n\n{body}")
}

/// The table body of the door reference: every row, sorted by tool name.
fn door_reference_body(tool_rows: &[(String, String, String)]) -> String {
    let mut out = String::from("tool | pack | card\n");
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

/// The pin line: the tool count and the fingerprint of the body it names.
///
/// The fingerprint is FNV-1a 64 over the rendered body's exact bytes, so one
/// changed byte in any row moves it. It is computed here, never written down:
/// a constant would say nothing about the surface a reader holds. There is no
/// clock, address or hash map in the input, so the same rows always pin the
/// same way.
fn door_reference_pin(tools: usize, body: &str) -> String {
    format!(
        "pin: {tools} tools, schema {} (fnv64 of the rows below)",
        hash_hex(body.as_bytes())
    )
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
    fn four_static_roots_and_the_lesson_view_are_served_and_listed() {
        let list = resource_list();
        assert_eq!(
            list.len(),
            ROOT_URIS.len() + 1,
            "the door serves the four static families plus the lesson view"
        );
        let uris: Vec<&str> = list.iter().map(|r| r.uri.as_str()).collect();
        assert_eq!(uris[..ROOT_URIS.len()], ROOT_URIS.to_vec());
        assert_eq!(uris[ROOT_URIS.len()], MEMORY_URI, "then the lesson view");
        for resource in &list {
            assert!(!resource.name.is_empty(), "every resource is named");
            assert!(
                resource.description.is_some(),
                "every resource explains itself"
            );
            assert_eq!(resource.mime_type.as_deref(), Some(RESOURCE_MIME));
        }
        assert_eq!(
            resource_templates().len(),
            ROOT_URIS.len() + 1,
            "one template per static family, plus the lesson query shape"
        );
    }

    #[test]
    fn door_resource_serves_the_roots_at_no_tool_budget() {
        // These resources cost no tool-list budget: no root URI is a tool
        // name, so `tools/list` does not grow by a char for them.
        let list = resource_list();
        assert_eq!(
            list.len(),
            ROOT_URIS.len() + 1,
            "the door serves the four static families plus the lesson view"
        );
        let mut tool_names: Vec<String> = crate::mcp::tool_names()
            .into_iter()
            .map(|name| (*name).to_owned())
            .collect();
        for pack in crate::packs::PackRegistry::shipped().packs() {
            for verb in pack.verb_names() {
                tool_names.push((*verb).to_owned());
            }
        }
        for uri in ROOT_URIS.into_iter().chain([MEMORY_URI]) {
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
        println!(
            "door_resource: {} resources, 0 tool-budget chars PASS",
            list.len()
        );
    }

    #[test]
    fn scene_view_carries_revision_count_and_digest() {
        let resource = scene_resource();
        assert_eq!(resource.uri, SCENE_URI);
        assert_eq!(resource.name, "scene");
        assert_eq!(
            resource.mime_type.as_deref(),
            Some(RESOURCE_MIME),
            "the scene view is plain text"
        );
        assert!(
            resource
                .description
                .as_deref()
                .is_some_and(|text| !text.is_empty()),
            "the scene view explains what it carries"
        );
        let first = scene_text(3, 7, "abc123");
        assert_eq!(
            first, "# scene\n\nrevision: 3\nobjects: 7\ndigest: abc123\n",
            "the view names the revision, the object count and the digest"
        );
        assert_eq!(
            first,
            scene_text(3, 7, "abc123"),
            "no clock, no per-call state"
        );
        assert_ne!(
            first,
            scene_text(4, 7, "abc123"),
            "a moved revision moves the view"
        );
        assert_ne!(
            first,
            scene_text(3, 8, "abc123"),
            "a moved count moves the view"
        );
        assert_ne!(
            first,
            scene_text(3, 7, "def456"),
            "a moved digest moves the view"
        );
        let contents = scene_contents(first.clone());
        match &contents {
            ResourceContents::TextResourceContents { uri, text, .. } => {
                assert_eq!(uri, SCENE_URI, "the contents name the scene URI");
                assert_eq!(text, &first, "the contents carry the rendered view");
            }
            other => panic!("the scene view is text: {other:?}"),
        }
    }

    #[test]
    fn scene_uri_is_not_one_of_the_static_families() {
        assert!(
            !ROOT_URIS.contains(&SCENE_URI),
            "the live view is served beside the four static families"
        );
        assert!(
            read_resource_text(SCENE_URI, &[]).is_err(),
            "the live view needs a document; it is read through the door, not this table"
        );
    }

    /// The lesson view's rows, as the body prints them: the JSON lines only.
    fn body_rows(body: &str) -> Vec<&str> {
        body.lines().filter(|line| line.starts_with('{')).collect()
    }

    fn lesson(kind: &str, path: &str, utc: &str, text: &str) -> crate::memory_store::Record {
        crate::memory_store::build(kind, "d57b5c8ae3096614", path, utc, text, Some("49f3615e4"))
            .expect("a well-shaped lesson")
    }

    #[test]
    fn the_lesson_view_is_listed_and_its_query_shape_is_advertised() {
        let list = resource_list();
        let listed: Vec<&str> = list.iter().map(|r| r.uri.as_str()).collect();
        assert_eq!(
            listed,
            vec![
                CRATE_MAP_URI,
                DOOR_REFERENCE_URI,
                ERROR_CATALOG_URI,
                GAME_TOML_SCHEMA_URI,
                MEMORY_URI,
            ],
            "the four families, then the lesson view"
        );
        let templates: Vec<String> = resource_templates()
            .iter()
            .map(|template| template.uri_template.clone())
            .collect();
        assert!(
            templates.contains(&format!("{MEMORY_URI}{{?kind,path,hash}}")),
            "the lesson query shape is advertised: {templates:?}"
        );
    }

    #[test]
    fn the_lesson_resource_answers_rows_in_stamp_order() {
        // The shipped store: every row reads back, in UTC-stamp order, and the
        // body renders each row as the line the tool would print.
        let text = read_resource_text(MEMORY_URI, &[]).expect("the lesson view reads");
        let printed = body_rows(&text);
        assert!(printed.len() >= 4, "{text}");
        assert!(text.starts_with("# memory\n\n"), "{text}");
        let rows = crate::memory_store::parse(&printed.join("\n"), "view")
            .expect("every printed line is a row");
        assert_eq!(
            rows,
            crate::memory_store::stamp_order(&rows),
            "the view answers in stamp order"
        );
        assert_eq!(
            text,
            read_resource_text(MEMORY_URI, &[]).expect("a second read"),
            "no clock, no per-call state"
        );
        // A filter keeps only the rows it names, and says how many it dropped.
        let anim = read_resource_text(&format!("{MEMORY_URI}?path=crates/anim"), &[])
            .expect("a filtered read");
        for line in body_rows(&anim) {
            assert!(line.contains("crates/anim"), "{line}");
        }
        assert!(anim.contains("| filter: path=crates/anim"), "{anim}");
        assert!(
            body_rows(&anim).len() < printed.len(),
            "the filter kept fewer rows than the root"
        );
        let none = read_resource_text(&format!("{MEMORY_URI}?path=crates/nope"), &[])
            .expect("an empty answer is still an answer");
        assert!(body_rows(&none).is_empty(), "{none}");
        assert!(none.contains("(no lesson row matches)"), "{none}");
    }

    #[test]
    fn the_lesson_query_refuses_the_shape_it_did_not_ask_for() {
        for uri in [
            "e2040://memory?kind=rumour",
            "e2040://memory?hash=not-a-hash",
            "e2040://memory?size=3",
            "e2040://memory?path=",
            "e2040://memory?path=a&path=b",
        ] {
            let error = memory_filter(uri).expect_err("the query refuses");
            assert!(
                matches!(error, ResourceError::Unavailable { .. }),
                "{uri} refuses as unavailable: {error}"
            );
        }
        for uri in ["e2040://memories", "e2040://memory/row", "e2040://scene"] {
            assert!(!is_memory_uri(uri), "{uri} is not a lesson read");
            let error = read_resource_text(uri, &[]).expect_err("unknown family");
            assert!(matches!(error, ResourceError::UnknownUri { .. }), "{uri}");
        }
        // The keys normalize the way the tool normalizes them.
        let filter = memory_filter("e2040://memory?hash=0xD57B5C8AE3096614&kind=decision")
            .expect("an 0x upper-case hash is the same query");
        assert_eq!(filter.hash.as_deref(), Some("d57b5c8ae3096614"));
        assert_eq!(filter.kind.as_deref(), Some("decision"));
        assert_eq!(
            memory_filter(MEMORY_URI).expect("the root asks for all rows"),
            crate::memory_store::Filter::default()
        );
    }

    #[test]
    fn the_lesson_body_prints_the_canonical_line_of_each_row_it_keeps() {
        let early = lesson(
            "decision",
            "crates/anim/src/a.rs",
            "2026-10-23T09:00:00Z",
            "one",
        );
        let late = lesson(
            "failed-fix",
            "crates/anim/src/b.rs",
            "2026-10-24T09:00:00Z",
            "two",
        );
        let rows = crate::memory_store::stamp_order(&[late.clone(), early.clone()]);
        let body = memory_body(&rows, &crate::memory_store::Filter::default());
        assert_eq!(
            body,
            format!(
                "# memory\n\nrows: 2 of 2 | filter: all rows\n\n{}\n{}\n",
                early.line(),
                late.line()
            ),
            "stamp order, one canonical line each"
        );
        let kind = crate::memory_store::Filter {
            kind: Some("failed-fix".to_owned()),
            ..crate::memory_store::Filter::default()
        };
        assert_eq!(
            memory_body(&rows, &kind),
            format!(
                "# memory\n\nrows: 1 of 2 | filter: kind=failed-fix\n\n{}\n",
                late.line()
            )
        );
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
    fn resource_template_reads_one_child() {
        let rows = vec![(
            "scene.read".to_owned(),
            "core".to_owned(),
            "observe".to_owned(),
        )];
        let cases = [
            ("e2040://door-reference/scene.read", "scene.read"),
            ("e2040://error-catalog/bad_args", "bad_args"),
            ("e2040://game-toml-schema/name", "name"),
            ("e2040://crate-map/core", "core"),
        ];
        for (uri, id) in cases {
            let text = read_resource_text(uri, &rows).expect("the child reads");
            let body: Vec<&str> = text
                .lines()
                .filter(|line| !line.is_empty() && !line.starts_with('#'))
                .collect();
            assert_eq!(body.len(), 2, "one header and one row at {uri}: {text}");
            assert!(
                body[1].starts_with(&format!("{id} |")),
                "the child carries its own row at {uri}: {text}"
            );
        }
        for uri in [
            "e2040://door-reference/no.such",
            "e2040://error-catalog/no_such",
            "e2040://game-toml-schema/nope",
            "e2040://crate-map/nope",
        ] {
            let error = read_resource_text(uri, &rows).expect_err("an unknown child refuses");
            assert!(
                matches!(error, ResourceError::UnknownUri { .. }),
                "{uri} refuses as unknown_uri"
            );
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

    /// The fingerprint a rendered reference pins: the 16-char hex after
    /// `schema ` on the pin line.
    fn pinned_schema(text: &str) -> &str {
        let marker = "schema ";
        let start = text.find(marker).expect("the pin names a schema") + marker.len();
        let rest = &text[start..];
        &rest[..rest.find(' ').expect("the fingerprint ends at a space")]
    }

    /// Two rows the pin tests share: a core verb and a pack verb.
    fn two_rows() -> Vec<(String, String, String)> {
        vec![
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
        ]
    }

    #[test]
    fn door_reference_pins_the_tool_count_and_the_schema() {
        let rows = two_rows();
        let text = read_resource_text(DOOR_REFERENCE_URI, &rows).expect("reference reads");
        let pin = text
            .lines()
            .find(|line| line.starts_with("pin: "))
            .unwrap_or_else(|| panic!("the reference carries a pin line: {text}"));
        assert!(pin.contains("2 tools"), "the pin counts the rows: {pin}");
        let schema = pinned_schema(&text);
        assert_eq!(schema.len(), 16, "a 16-char hex fingerprint: {pin}");
        assert!(
            schema.chars().all(|c| c.is_ascii_hexdigit()),
            "the fingerprint is hex: {pin}"
        );
        let body = door_reference_body(&rows);
        assert_eq!(
            schema,
            hash_hex(body.as_bytes()),
            "the pin is the fingerprint of the body it names: {pin}"
        );
        let empty = read_resource_text(DOOR_REFERENCE_URI, &[]).expect("empty reference reads");
        assert!(
            empty.contains("pin: 0 tools"),
            "an empty surface pins its zero count: {empty}"
        );
    }

    #[test]
    fn one_changed_schema_byte_moves_the_fingerprint() {
        let rows = two_rows();
        let first = read_resource_text(DOOR_REFERENCE_URI, &rows).expect("reference reads");
        let mut edited = rows.clone();
        edited[0].2 = "obserwe".to_owned();
        let second = read_resource_text(DOOR_REFERENCE_URI, &edited).expect("edited reads");
        assert_ne!(first, second, "a changed row moves the rendered text");
        assert_ne!(
            pinned_schema(&first),
            pinned_schema(&second),
            "one changed byte moves the pinned fingerprint"
        );
        assert_eq!(
            pinned_schema(&second),
            hash_hex(door_reference_body(&edited).as_bytes()),
            "the edited pin covers the edited body"
        );
    }

    #[test]
    fn the_same_schema_pins_the_same_text_twice() {
        // No clock, no address, no hash map order: the same rows render the
        // same bytes on every call and in every process.
        let rows = two_rows();
        let first = read_resource_text(DOOR_REFERENCE_URI, &rows).expect("reference reads");
        let second = read_resource_text(DOOR_REFERENCE_URI, &rows).expect("reference reads");
        assert_eq!(first, second, "the same schema pins the same text");
        let pin = first
            .lines()
            .find(|line| line.starts_with("pin: "))
            .expect("a pin line");
        assert_eq!(
            pin,
            door_reference_pin(rows.len(), &door_reference_body(&rows)),
            "the pin reads no clock and holds no per-call state"
        );
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
