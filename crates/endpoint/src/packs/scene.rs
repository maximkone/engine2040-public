//! `scene` pack: layout authoring beyond the core transaction tools.
//!
//! Loaded by default: the door's own domain. Holds the brief, spatial
//! findings, intent placement, brief-to-level build, spatial questions and
//! the digest watch. Component verbs (typed `set_component` /
//! `remove_component` ops) ride `tx.preview` / `tx.apply` in the core.
//!
//! It also holds the person's half of a mark: `note.pin.add`, `.list` and
//! `.clear` keep notes at world points in memory (monotonic ids, a 256-pin
//! ring, no file, no shell), so a sentence can point at one stone.

use super::{bad_args, PackDef, PackError, VerbSpec};
use crate::build_level::{BuildLevelTool, TOOL_BUILD_LEVEL};
use crate::discover::{WatchTool, TOOL_SCENE_WATCH};
#[cfg(feature = "rules")]
use crate::game_play::{GamePlayTool, TOOL_GAME_PLAY};
use crate::scene_next::{SceneNextTool, TOOL_SCENE_NEXT};
use crate::scene_pick::{ScenePickTool, TOOL_SCENE_PICK};
use crate::scene_query::{SceneQueryTool, TOOL_SCENE_QUERY};
use crate::scene_tools::{
    SceneBriefTool, ScenePlaceTool, SceneSpatialTool, TOOL_SCENE_BRIEF, TOOL_SCENE_PLACE,
    TOOL_SCENE_SPATIAL,
};
use crate::tool_capabilities::CapabilityCard;
use crate::world_query::{WorldQueryTool, TOOL_WORLD_QUERY};
use engine_scene::component::ComponentKind;
use rmcp::schemars::JsonSchema;
use rmcp::serde::{Deserialize, Serialize};

mod prefab;
use prefab::{PrefabPlaceTool, PrefabSaveTool, TOOL_PREFAB_PLACE, TOOL_PREFAB_SAVE};
mod save;
use save::{SceneLoadTool, SceneSaveTool, TOOL_SCENE_LOAD, TOOL_SCENE_SAVE};

/// Pack name.
pub const PACK: &str = "scene";

/// One component schema as the index lists it: the JSON shape and the
/// legal ranges, so `api.describe component:<kind>` answers without a source
/// read. Authored through `set_component` ops in `tx.preview` / `tx.apply`.
pub fn component_doc(kind: ComponentKind) -> (&'static str, &'static str) {
    match kind {
        ComponentKind::Mesh => (
            r#"{"asset": name, "cast_shadows": bool=true}"#,
            "Mesh asset to draw. Names are 1..=64 bytes of 0-9 A-Z a-z - _ . /.",
        ),
        ComponentKind::Collider => (
            r#"{"shape": "box"|"sphere"|"capsule", "half": [x,y,z], "sensor": bool=false}"#,
            "Collision shape. half is meters: box half extents, sphere radius in x, capsule radius in x and half height in y; each in 0.0001..=100000. A sensor reports overlaps and never blocks.",
        ),
        ComponentKind::Material => (
            r#"{"asset": name, "tint": [r,g,b]=[1,1,1]}"#,
            "Material reference plus a linear RGB tint, each channel 0..=1.",
        ),
        ComponentKind::Light => (
            r#"{"kind": "point"|"spot"|"directional", "color": [r,g,b], "intensity": n, "range": n}"#,
            "Light source. color channels 0..=1, intensity 0..=1000000, range meters 0..=100000 (ignored by directional lights).",
        ),
        ComponentKind::Spawner => (
            r#"{"archetype": name, "count": n, "period_ticks": n}"#,
            "Entity spawner. count 1..=65535 entities per burst, period_ticks at least 1 (integer ticks, never seconds).",
        ),
        ComponentKind::Destructible => (
            r#"{"material": name, "health": n, "chunks": n}"#,
            "Destructible body. health is an integer of at least 1; chunks is the fracture budget 1..=4096; the material names the physical table row.",
        ),
        ComponentKind::Anatomy => (
            r#"{"template": name, "limbs": n}"#,
            "Body plan: anatomy template name and limb count 1..=32.",
        ),
        ComponentKind::Rig => (
            r#"{"skeleton": name, "clips": name}"#,
            "Skeleton asset plus clip set asset.",
        ),
    }
}

/// The scene pack: twelve verbs, loaded by default.
pub fn pack() -> Result<PackDef, PackError> {
    let mut pack = PackDef::new(
        PACK,
        "layout authoring: brief, spatial findings, intent placement, brief-to-level build, \
         play the built game, spatial questions, the digest watch, the next-call planner, \
         pointing at a capture and the BRP component filter",
        true,
    );
    pack.add(VerbSpec::of::<SceneBriefTool>(
        TOOL_SCENE_BRIEF,
        PACK,
        CapabilityCard::Observe,
    )?);
    pack.add(VerbSpec::of::<SceneSpatialTool>(
        TOOL_SCENE_SPATIAL,
        PACK,
        CapabilityCard::Observe,
    )?);
    pack.add(VerbSpec::of::<ScenePlaceTool>(
        TOOL_SCENE_PLACE,
        PACK,
        CapabilityCard::Commit,
    )?);
    pack.add(VerbSpec::of::<BuildLevelTool>(
        TOOL_BUILD_LEVEL,
        PACK,
        CapabilityCard::Commit,
    )?);
    #[cfg(feature = "rules")]
    pack.add(VerbSpec::of::<GamePlayTool>(
        TOOL_GAME_PLAY,
        PACK,
        CapabilityCard::Observe,
    )?);
    pack.add(VerbSpec::of::<SceneQueryTool>(
        TOOL_SCENE_QUERY,
        PACK,
        CapabilityCard::Observe,
    )?);
    pack.add(VerbSpec::of::<WatchTool>(
        TOOL_SCENE_WATCH,
        PACK,
        CapabilityCard::Observe,
    )?);
    pack.add(VerbSpec::of::<SceneNextTool>(
        TOOL_SCENE_NEXT,
        PACK,
        CapabilityCard::Observe,
    )?);
    pack.add(VerbSpec::of::<ScenePickTool>(
        TOOL_SCENE_PICK,
        PACK,
        CapabilityCard::Observe,
    )?);
    pack.add(VerbSpec::of::<NotePinAddTool>(
        TOOL_NOTE_PIN_ADD,
        PACK,
        CapabilityCard::Commit,
    )?);
    pack.add(VerbSpec::of::<NotePinListTool>(
        TOOL_NOTE_PIN_LIST,
        PACK,
        CapabilityCard::Observe,
    )?);
    pack.add(VerbSpec::of::<NotePinClearTool>(
        TOOL_NOTE_PIN_CLEAR,
        PACK,
        CapabilityCard::Revert,
    )?);
    pack.add(VerbSpec::of::<WorldQueryTool>(
        TOOL_WORLD_QUERY,
        PACK,
        CapabilityCard::Observe,
    )?);
    pack.add(VerbSpec::of::<SceneSaveTool>(
        TOOL_SCENE_SAVE,
        PACK,
        CapabilityCard::Commit,
    )?);
    pack.add(VerbSpec::of::<SceneLoadTool>(
        TOOL_SCENE_LOAD,
        PACK,
        CapabilityCard::Commit,
    )?);
    pack.add(VerbSpec::of::<PrefabSaveTool>(
        TOOL_PREFAB_SAVE,
        PACK,
        CapabilityCard::Commit,
    )?);
    pack.add(VerbSpec::of::<PrefabPlaceTool>(
        TOOL_PREFAB_PLACE,
        PACK,
        CapabilityCard::Commit,
    )?);
    Ok(pack)
}

// ---------------------------------------------------------------------------
// note.pin.*: the person's half of a mark. World points with a note, held in
// memory, no file, no shell.
// ---------------------------------------------------------------------------

/// Verb: add an in-memory pin at a world point.
pub const TOOL_NOTE_PIN_ADD: &str = "note.pin.add";

/// Verb: list the live pins, oldest first.
pub const TOOL_NOTE_PIN_LIST: &str = "note.pin.list";

/// Verb: drop every live pin.
pub const TOOL_NOTE_PIN_CLEAR: &str = "note.pin.clear";

/// Most pins one process holds; the oldest is dropped past it.
pub const MAX_PINS: usize = 256;

/// Longest pin text in chars.
pub const MAX_PIN_TEXT_CHARS: usize = 280;

/// Longest pin author in chars.
pub const MAX_PIN_AUTHOR_CHARS: usize = 32;

/// One note pinned at a world point.
#[derive(Debug, Clone, PartialEq)]
struct Pin {
    id: u64,
    pos: [f32; 3],
    text: String,
    author: String,
}

/// The process's pins: monotonic ids, a bounded ring, a dropped count.
struct PinStore {
    next_id: u64,
    pins: std::collections::VecDeque<Pin>,
    dropped: u64,
}

impl PinStore {
    /// An empty store whose first id is 1.
    const fn new() -> Self {
        Self {
            next_id: 1,
            pins: std::collections::VecDeque::new(),
            dropped: 0,
        }
    }
}

/// The process's one pin store. No file, no shell: a restart forgets it.
static PINS: std::sync::Mutex<PinStore> = std::sync::Mutex::new(PinStore::new());

/// One pin as the door reports it.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct PinRow {
    /// Monotonic id, 1 upward.
    pub id: u64,
    /// World position in meters, y up.
    pub pos: [f32; 3],
    /// The note, at most 280 chars.
    pub text: String,
    /// Who wrote it, at most 32 chars.
    pub author: String,
}

/// Args for `note.pin.add`: one world point and its note.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct NotePinAddArgs {
    /// World position in meters, y up.
    pub pos: [f32; 3],
    /// The note, 1..=280 chars.
    pub text: String,
    /// Who wrote it, 1..=32 chars.
    pub author: String,
}

/// `note.pin.add` result: the new pin's id and the live count.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct NotePinAddOutput {
    /// Id the pin was given.
    pub id: u64,
    /// Pins live after the add.
    pub pins: u64,
    /// Pins the ring has dropped since the process started.
    pub dropped: u64,
}

/// Args for `note.pin.list`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
#[expect(
    clippy::empty_structs_with_brackets,
    reason = "serde reads the wire shape `{}`; a unit struct reads null"
)]
pub struct NotePinListArgs {}

/// `note.pin.list` result: every live pin, oldest first.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct NotePinListOutput {
    /// Live pins, oldest first.
    pub pins: Vec<PinRow>,
    /// Pins the ring has dropped since the process started.
    pub dropped: u64,
}

/// Args for `note.pin.clear`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
#[expect(
    clippy::empty_structs_with_brackets,
    reason = "serde reads the wire shape `{}`; a unit struct reads null"
)]
pub struct NotePinClearArgs {}

/// `note.pin.clear` result: how many pins went.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct NotePinClearOutput {
    /// Pins removed by this call.
    pub cleared: u64,
}

/// A pin text or author outside its bound: the message names the legal range
/// and one recovery.
fn bad_pin(field: &str, chars: usize, max: usize) -> crate::mcp::McpToolError {
    bad_args(format!(
        "{field} is {chars} chars and must be 1..={max}; re-run with 1..={max} chars"
    ))
}

/// The store's guard. A poisoned lock still holds the pins: the door reports
/// them rather than refusing, so one earlier panic cannot hide the notes.
fn pins() -> std::sync::MutexGuard<'static, PinStore> {
    PINS.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Add one pin: validate, clip through the core text owner, ring out the
/// oldest, report the dropped count.
fn run_pin_add(args: &NotePinAddArgs) -> Result<NotePinAddOutput, crate::mcp::McpToolError> {
    if !args.pos.iter().all(|v| v.is_finite()) {
        return Err(bad_args(format!(
            "pos {:?} is not finite; re-run with meters on each axis",
            args.pos
        )));
    }
    let text_chars = args.text.chars().count();
    if text_chars == 0 || text_chars > MAX_PIN_TEXT_CHARS {
        return Err(bad_pin("text", text_chars, MAX_PIN_TEXT_CHARS));
    }
    let author_chars = args.author.chars().count();
    if author_chars == 0 || author_chars > MAX_PIN_AUTHOR_CHARS {
        return Err(bad_pin("author", author_chars, MAX_PIN_AUTHOR_CHARS));
    }
    let mut store = pins();
    let id = store.next_id;
    store.next_id = store.next_id.saturating_add(1);
    let text = engine_core::text::clip_bytes(&args.text, MAX_PIN_TEXT_CHARS * 4).to_owned();
    let author = engine_core::text::clip_bytes(&args.author, MAX_PIN_AUTHOR_CHARS * 4).to_owned();
    store.pins.push_back(Pin {
        id,
        pos: args.pos,
        text,
        author,
    });
    while store.pins.len() > MAX_PINS {
        store.pins.pop_front();
        store.dropped = store.dropped.saturating_add(1);
    }
    Ok(NotePinAddOutput {
        id,
        pins: u64::try_from(store.pins.len()).unwrap_or(u64::MAX),
        dropped: store.dropped,
    })
}

/// Every live pin, oldest first.
fn run_pin_list() -> NotePinListOutput {
    let store = pins();
    NotePinListOutput {
        pins: store
            .pins
            .iter()
            .map(|pin| PinRow {
                id: pin.id,
                pos: pin.pos,
                text: pin.text.clone(),
                author: pin.author.clone(),
            })
            .collect(),
        dropped: store.dropped,
    }
}

/// Drop every live pin.
fn run_pin_clear() -> NotePinClearOutput {
    let mut store = pins();
    let cleared = u64::try_from(store.pins.len()).unwrap_or(u64::MAX);
    store.pins.clear();
    NotePinClearOutput { cleared }
}

fn note_pin_add(
    service: &crate::mcp::McpServer,
    args: &NotePinAddArgs,
) -> Result<NotePinAddOutput, crate::mcp::McpToolError> {
    service.dispatch(TOOL_NOTE_PIN_ADD, |_state| run_pin_add(args))
}

fn note_pin_list(
    service: &crate::mcp::McpServer,
    _args: &NotePinListArgs,
) -> Result<NotePinListOutput, crate::mcp::McpToolError> {
    service.dispatch(TOOL_NOTE_PIN_LIST, |_state| Ok(run_pin_list()))
}

fn note_pin_clear(
    service: &crate::mcp::McpServer,
    _args: &NotePinClearArgs,
) -> Result<NotePinClearOutput, crate::mcp::McpToolError> {
    service.dispatch(TOOL_NOTE_PIN_CLEAR, |_state| Ok(run_pin_clear()))
}

crate::e2_tool!(
    /// `note.pin.add` tool: pin a note at a world point. In memory only.
    NotePinAddTool,
    name = TOOL_NOTE_PIN_ADD,
    description = "Pin a note at a world point: pos in meters (y up), text 1..=280 chars, author 1..=32 chars. Ids are monotonic, the ring holds 256 pins and drops the oldest, and the answer reports the dropped count. In memory only: no file, no shell, forgotten on restart.",
    readonly = false,
    args = NotePinAddArgs,
    output = NotePinAddOutput,
    invoke = note_pin_add
);

crate::e2_tool!(
    /// `note.pin.list` tool: every live pin, oldest first.
    NotePinListTool,
    name = TOOL_NOTE_PIN_LIST,
    description = "List the live pins oldest first: id, world pos, text, author, plus the dropped count. Read-only.",
    readonly = true,
    args = NotePinListArgs,
    output = NotePinListOutput,
    invoke = note_pin_list
);

crate::e2_tool!(
    /// `note.pin.clear` tool: drop every live pin.
    NotePinClearTool,
    name = TOOL_NOTE_PIN_CLEAR,
    description = "Drop every live pin in this process and report how many went. No file, no shell.",
    readonly = false,
    args = NotePinClearArgs,
    output = NotePinClearOutput,
    invoke = note_pin_clear
);

#[cfg(test)]
mod tests {
    use super::*;
    use engine_scene::component::ALL_KINDS;

    /// Every kind documents a non-empty shape plus prose; the index answers
    /// `api.describe component:<kind>` from this table alone, so a blank or
    /// missing row would serve an empty answer.
    #[test]
    fn component_doc_covers_every_kind() {
        for kind in ALL_KINDS {
            let (shape, docs) = component_doc(kind);
            assert!(!shape.is_empty(), "{kind:?} shape is empty");
            assert!(!docs.is_empty(), "{kind:?} docs are empty");
        }
        let (mesh_shape, _) = component_doc(ComponentKind::Mesh);
        assert!(
            mesh_shape.contains("asset"),
            "mesh shape names the asset: {mesh_shape}"
        );
    }

    fn pin_args(text: &str, author: &str) -> NotePinAddArgs {
        NotePinAddArgs {
            pos: [1.0, 2.0, 3.0],
            text: text.to_owned(),
            author: author.to_owned(),
        }
    }

    /// The one test that touches the process pin store: ids climb, the ring
    /// keeps its cap and reports how many it dropped, clear empties it, and a
    /// text or author outside its bound refuses.
    #[test]
    fn note_pins_keep_monotonic_ids_a_bounded_ring_and_clear() {
        run_pin_clear();
        let first = run_pin_add(&pin_args("watch this stone", "maxim")).expect("first pin");
        let second = run_pin_add(&pin_args("and this one", "maxim")).expect("second pin");
        assert_eq!(first.id + 1, second.id);
        let listed = run_pin_list();
        assert_eq!(listed.pins.len(), 2);
        assert_eq!(listed.pins[0].text, "watch this stone");
        assert_eq!(listed.pins[1].pos, [1.0, 2.0, 3.0]);
        // Fill past the cap: the oldest goes and the dropped count says so.
        for n in 0..MAX_PINS {
            run_pin_add(&pin_args(&format!("pin {n}"), "bot")).expect("pin");
        }
        let full = run_pin_list();
        assert_eq!(full.pins.len(), MAX_PINS);
        assert_eq!(full.dropped, 2);
        assert!(full.pins.windows(2).all(|pair| pair[0].id < pair[1].id));
        assert_eq!(
            run_pin_clear().cleared,
            u64::try_from(MAX_PINS).unwrap_or(u64::MAX)
        );
        assert!(run_pin_list().pins.is_empty());

        let long_text = "x".repeat(MAX_PIN_TEXT_CHARS + 1);
        let error = run_pin_add(&pin_args(&long_text, "maxim")).expect_err("long text");
        assert_eq!(error.code, "bad_args");
        assert!(error.message.contains("1..=280"), "{}", error.message);
        assert!(error.message.contains("re-run"), "{}", error.message);
        let long_author = "y".repeat(MAX_PIN_AUTHOR_CHARS + 1);
        assert_eq!(
            run_pin_add(&pin_args("short", &long_author))
                .expect_err("long author")
                .code,
            "bad_args"
        );
        assert_eq!(
            run_pin_add(&pin_args("", "maxim")).expect_err("empty").code,
            "bad_args"
        );
        let mut not_finite = pin_args("pos", "maxim");
        not_finite.pos[1] = f32::NAN;
        assert_eq!(run_pin_add(&not_finite).expect_err("nan").code, "bad_args");
    }
}
