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
    add_people_and_camera_surface(&mut pack)?;
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

/// The person's half and the camera list: `note.pin.*`, `scene.camera.*` and
/// `scene.shot.*`, kept in one place so `pack()` reads as the layout surface.
fn add_people_and_camera_surface(pack: &mut PackDef) -> Result<(), PackError> {
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
    pack.add(VerbSpec::of::<CameraAddTool>(
        TOOL_CAMERA_ADD,
        PACK,
        CapabilityCard::Commit,
    )?);
    pack.add(VerbSpec::of::<CameraSetTool>(
        TOOL_CAMERA_SET,
        PACK,
        CapabilityCard::Commit,
    )?);
    pack.add(VerbSpec::of::<CameraListTool>(
        TOOL_CAMERA_LIST,
        PACK,
        CapabilityCard::Observe,
    )?);
    pack.add(VerbSpec::of::<CameraRemoveTool>(
        TOOL_CAMERA_REMOVE,
        PACK,
        CapabilityCard::Commit,
    )?);
    pack.add(VerbSpec::of::<CameraFrameTool>(
        TOOL_CAMERA_FRAME,
        PACK,
        CapabilityCard::Observe,
    )?);
    pack.add(VerbSpec::of::<ShotAddTool>(
        TOOL_SHOT_ADD,
        PACK,
        CapabilityCard::Commit,
    )?);
    pack.add(VerbSpec::of::<ShotListTool>(
        TOOL_SHOT_LIST,
        PACK,
        CapabilityCard::Observe,
    )?);
    pack.add(VerbSpec::of::<ShotRemoveTool>(
        TOOL_SHOT_REMOVE,
        PACK,
        CapabilityCard::Commit,
    )?);
    Ok(())
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

// ---------------------------------------------------------------------------
// scene.camera and scene.shot: the camera list and the shot list
// ---------------------------------------------------------------------------

/// Verb: add a camera node.
pub const TOOL_CAMERA_ADD: &str = "scene.camera.add";

/// Verb: change one camera node's fields.
pub const TOOL_CAMERA_SET: &str = "scene.camera.set";

/// Verb: list the camera nodes.
pub const TOOL_CAMERA_LIST: &str = "scene.camera.list";

/// Verb: drop one camera node and the shots that name it.
pub const TOOL_CAMERA_REMOVE: &str = "scene.camera.remove";

/// Verb: one camera's view and projection numbers against the scene bounds.
pub const TOOL_CAMERA_FRAME: &str = "scene.camera.frame";

/// Verb: add a shot over a camera.
pub const TOOL_SHOT_ADD: &str = "scene.shot.add";

/// Verb: list the shots by name.
pub const TOOL_SHOT_LIST: &str = "scene.shot.list";

/// Verb: drop one shot by name.
pub const TOOL_SHOT_REMOVE: &str = "scene.shot.remove";

/// Most camera nodes one process holds; the next add refuses.
pub const MAX_CAMERAS: usize = 16;

/// Most shots one process holds; the next add refuses.
pub const MAX_SHOTS: usize = 64;

/// Longest shot in ticks, one minute at 60 Hz.
pub const MAX_SHOT_TICKS: u32 = 3600;

/// Narrowest horizontal field of view, hundredths of a degree (1 degree).
pub const MIN_FOV_X100: u32 = 100;

/// Widest horizontal field of view, hundredths of a degree (179 degrees).
pub const MAX_FOV_X100: u32 = 17_900;

/// Nearest usable clip plane, in cm.
pub const MIN_NEAR_CM: u32 = 1;

/// Farthest usable clip plane, in cm (10 km).
pub const MAX_FAR_CM: u32 = 1_000_000;

/// Longest camera or shot name in chars.
pub const MAX_CAMERA_NAME_CHARS: usize = 32;

/// One camera node as the store keeps it.
#[derive(Debug, Clone, PartialEq)]
struct CameraNode {
    id: u64,
    name: String,
    eye: [f32; 3],
    target: [f32; 3],
    fov_deg_x100: u32,
    near_cm: u32,
    far_cm: u32,
}

/// One shot: a camera, a duration in ticks and a name.
#[derive(Debug, Clone, PartialEq)]
struct Shot {
    camera: u64,
    ticks: u32,
    name: String,
}

/// The process's cameras and shots: monotonic camera ids, no file, no shell.
struct CameraStore {
    next_id: u64,
    cameras: Vec<CameraNode>,
    shots: Vec<Shot>,
}

impl CameraStore {
    /// An empty store whose first camera id is 1.
    const fn new() -> Self {
        Self {
            next_id: 1,
            cameras: Vec::new(),
            shots: Vec::new(),
        }
    }
}

/// The process's one camera store. A restart forgets it.
static CAMERAS: std::sync::Mutex<CameraStore> = std::sync::Mutex::new(CameraStore::new());

/// Means the camera's store: a poisoned lock still holds the cameras.
fn cameras() -> std::sync::MutexGuard<'static, CameraStore> {
    CAMERAS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// One camera node as the door reports it.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct CameraRow {
    /// Monotonic id, 1 upward.
    pub id: u64,
    /// Lower-snake name, unique among the live cameras.
    pub name: String,
    /// Eye in meters, y up.
    pub eye: [f32; 3],
    /// Look-at point in meters, y up.
    pub target: [f32; 3],
    /// Horizontal field of view in hundredths of a degree.
    pub fov_deg_x100: u32,
    /// Near clip plane in cm.
    pub near_cm: u32,
    /// Far clip plane in cm.
    pub far_cm: u32,
}

impl CameraRow {
    /// The row for one stored node.
    fn of(node: &CameraNode) -> Self {
        Self {
            id: node.id,
            name: node.name.clone(),
            eye: node.eye,
            target: node.target,
            fov_deg_x100: node.fov_deg_x100,
            near_cm: node.near_cm,
            far_cm: node.far_cm,
        }
    }
}

/// Args for `scene.camera.add`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct CameraAddArgs {
    /// Lower-snake name, 1..=32 chars, unique.
    pub name: String,
    /// Eye in meters, y up.
    pub eye: [f32; 3],
    /// Look-at point in meters, y up; must differ from `eye`.
    pub target: [f32; 3],
    /// Horizontal field of view in hundredths of a degree, 100..=17900.
    pub fov_deg_x100: u32,
    /// Near clip plane in cm, at least 1 and under `far_cm`.
    pub near_cm: u32,
    /// Far clip plane in cm, over `near_cm` and at most 1000000.
    pub far_cm: u32,
}

/// Args for `scene.camera.set`: the id plus the fields to change.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct CameraSetArgs {
    /// Id of the camera to change.
    pub camera: u64,
    /// New lower-snake name; kept when absent.
    pub name: Option<String>,
    /// New eye in meters; kept when absent.
    pub eye: Option<[f32; 3]>,
    /// New look-at point in meters; kept when absent.
    pub target: Option<[f32; 3]>,
    /// New horizontal field of view in hundredths of a degree; kept when absent.
    pub fov_deg_x100: Option<u32>,
    /// New near clip plane in cm; kept when absent.
    pub near_cm: Option<u32>,
    /// New far clip plane in cm; kept when absent.
    pub far_cm: Option<u32>,
}

/// `scene.camera.add` and `scene.camera.set` result.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct CameraSaveOutput {
    /// The camera as stored.
    pub camera: CameraRow,
    /// Cameras live after the call.
    pub cameras: u32,
}

/// Args for `scene.camera.list`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
#[expect(
    clippy::empty_structs_with_brackets,
    reason = "serde reads the wire shape `{}`; a unit struct reads null"
)]
pub struct CameraListArgs {}

/// `scene.camera.list` result: every camera, id order.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct CameraListOutput {
    /// Live cameras, id order.
    pub cameras: Vec<CameraRow>,
    /// Live shots.
    pub shots: u32,
}

/// Args for `scene.camera.remove`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct CameraRemoveArgs {
    /// Id of the camera to drop.
    pub camera: u64,
}

/// `scene.camera.remove` result.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct CameraRemoveOutput {
    /// True when a camera was dropped.
    pub removed: bool,
    /// Shots dropped with it.
    pub shots_dropped: u32,
    /// Cameras live after the call.
    pub cameras: u32,
}

/// Args for `scene.camera.frame`: a camera and the viewport it fills.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct CameraFrameArgs {
    /// Id of the camera to read.
    pub camera: u64,
    /// Viewport width in pixels, 16..=4096.
    pub width: u32,
    /// Viewport height in pixels, 16..=4096.
    pub height: u32,
}

/// `scene.camera.frame` result: the view and projection numbers plus a
/// `view.frame`-style self-check over the scene bounds the brief exposes.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct CameraFrameOutput {
    /// Camera id.
    pub camera: u64,
    /// Camera name.
    pub name: String,
    /// Viewport width in pixels.
    pub width: u32,
    /// Viewport height in pixels.
    pub height: u32,
    /// Horizontal field of view in hundredths of a degree.
    pub fov_deg_x100: u32,
    /// Vertical field of view in hundredths of a degree.
    pub fov_y_deg_x100: u32,
    /// Width over height, in thousandths.
    pub aspect_x1000: u32,
    /// Near clip plane in cm.
    pub near_cm: u32,
    /// Far clip plane in cm.
    pub far_cm: u32,
    /// Normalized view axis.
    pub forward: [f32; 3],
    /// Normalized view right axis.
    pub right: [f32; 3],
    /// Normalized view up axis.
    pub up: [f32; 3],
    /// Scene bounds in cm as `min x, y, z, max x, y, z`; all zero when empty.
    pub bounds_cm: [i32; 6],
    /// Bounds corners in front of the camera (0..=8).
    pub corners_in_front: u8,
    /// Bounds corners inside the frustum (0..=8).
    pub corners_visible: u8,
    /// Projected bounds as `[x0, y0, x1, y1]`, when any corner is in front.
    pub box_px: Option<[u32; 4]>,
    /// The framing self-check over that box, when it projects.
    pub framing: Option<crate::packs::view::art::ViewFrameOutput>,
}

/// Args for `scene.shot.add`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct ShotAddArgs {
    /// Camera id this shot uses.
    pub camera: u64,
    /// Duration in ticks, 1..=3600.
    pub ticks: u32,
    /// Lower-snake name, unique among the live shots.
    pub name: String,
}

/// One shot as the door reports it.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct ShotRow {
    /// Camera id.
    pub camera: u64,
    /// Duration in ticks.
    pub ticks: u32,
    /// Shot name.
    pub name: String,
}

/// Args for `scene.shot.list`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
#[expect(
    clippy::empty_structs_with_brackets,
    reason = "serde reads the wire shape `{}`; a unit struct reads null"
)]
pub struct ShotListArgs {}

/// `scene.shot.list` result: every shot, name order.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct ShotListOutput {
    /// Live shots, name order.
    pub shots: Vec<ShotRow>,
}

/// Args for `scene.shot.remove`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct ShotRemoveArgs {
    /// Shot name to drop.
    pub name: String,
}

/// `scene.shot.add` and `scene.shot.remove` result.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct ShotCountOutput {
    /// True when a shot was dropped.
    pub removed: bool,
    /// Shots live after the call.
    pub shots: u32,
}

/// A name must be lower snake case: `a-z`, digits and `_`, starting with a
/// letter, 1..=32 chars.
fn check_lower_snake(field: &str, raw: &str) -> Result<String, crate::mcp::McpToolError> {
    let name = raw.trim();
    let chars = name.chars().count();
    let ok = (1..=MAX_CAMERA_NAME_CHARS).contains(&chars)
        && name.starts_with(|c: char| c.is_ascii_lowercase())
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
    if ok {
        return Ok(name.to_owned());
    }
    Err(bad_args(format!(
        "{field} `{}` is not lower_snake (1..={MAX_CAMERA_NAME_CHARS} chars, `a-z`, digits and \
         `_`, starting with a letter); re-run with a name such as camera_left",
        engine_core::text::clip_bytes(name, MAX_CAMERA_NAME_CHARS)
    )))
}

/// A camera's optics must form a usable frustum.
fn check_optics(
    fov_deg_x100: u32,
    near_cm: u32,
    far_cm: u32,
) -> Result<(), crate::mcp::McpToolError> {
    if !(MIN_FOV_X100..=MAX_FOV_X100).contains(&fov_deg_x100) {
        return Err(bad_args(format!(
            "fov_deg_x100 {fov_deg_x100} is outside {MIN_FOV_X100}..={MAX_FOV_X100} (1 to 179 \
             degrees); re-run with a field of view such as 6000"
        )));
    }
    if near_cm < MIN_NEAR_CM || far_cm > MAX_FAR_CM || near_cm >= far_cm {
        return Err(bad_args(format!(
            "near_cm {near_cm} and far_cm {far_cm} do not clip: legal pair is {MIN_NEAR_CM} <= \
             near_cm < far_cm <= {MAX_FAR_CM}; re-run with near_cm 10 and far_cm 1000000"
        )));
    }
    Ok(())
}

/// The eye and target must be finite and apart.
fn check_eye_target(eye: [f32; 3], target: [f32; 3]) -> Result<(), crate::mcp::McpToolError> {
    let finite = eye.iter().chain(target.iter()).all(|v| v.is_finite());
    let apart = eye
        .iter()
        .zip(target.iter())
        .any(|(a, b)| (a - b).abs() > f32::EPSILON);
    if finite && apart {
        return Ok(());
    }
    Err(bad_args(
        "eye and target must be finite meters and name two different points; re-run with an eye \
         over the sand and a target down the arena"
            .to_owned(),
    ))
}

/// The camera a live id names, or a refusal naming the legal ids.
fn camera_with(store: &CameraStore, id: u64) -> Result<CameraNode, crate::mcp::McpToolError> {
    store
        .cameras
        .iter()
        .find(|camera| camera.id == id)
        .cloned()
        .ok_or_else(|| {
            let ids = store
                .cameras
                .iter()
                .map(|camera| camera.id.to_string())
                .collect::<Vec<_>>()
                .join(", ");
            crate::mcp::McpToolError::new(
                "unknown_camera",
                format!(
                    "camera {id} is not live; legal ids are [{}]; re-run scene.camera.list for the \
                     live cameras",
                    if ids.is_empty() { "none".to_owned() } else { ids }
                ),
            )
        })
}

/// A name another live camera already holds.
fn name_taken(
    store: &CameraStore,
    name: &str,
    skip: Option<u64>,
) -> Result<(), crate::mcp::McpToolError> {
    let taken = store
        .cameras
        .iter()
        .any(|camera| camera.name == name && Some(camera.id) != skip);
    if !taken {
        return Ok(());
    }
    Err(bad_args(format!(
        "camera name `{name}` is taken; re-run with a unique lower_snake name"
    )))
}

/// Add one camera node.
fn run_camera_add(args: &CameraAddArgs) -> Result<CameraSaveOutput, crate::mcp::McpToolError> {
    let mut store = cameras();
    if store.cameras.len() >= MAX_CAMERAS {
        return Err(bad_args(format!(
            "the camera list holds {MAX_CAMERAS} nodes; re-run scene.camera.remove on one you no \
             longer shoot first"
        )));
    }
    let name = check_lower_snake("name", &args.name)?;
    name_taken(&store, &name, None)?;
    check_eye_target(args.eye, args.target)?;
    check_optics(args.fov_deg_x100, args.near_cm, args.far_cm)?;
    let id = store.next_id;
    store.next_id = store.next_id.saturating_add(1);
    let node = CameraNode {
        id,
        name,
        eye: args.eye,
        target: args.target,
        fov_deg_x100: args.fov_deg_x100,
        near_cm: args.near_cm,
        far_cm: args.far_cm,
    };
    store.cameras.push(node.clone());
    Ok(CameraSaveOutput {
        camera: CameraRow::of(&node),
        cameras: u32::try_from(store.cameras.len()).unwrap_or(u32::MAX),
    })
}

/// Change one camera node's fields, validating the merged result.
fn run_camera_set(args: &CameraSetArgs) -> Result<CameraSaveOutput, crate::mcp::McpToolError> {
    let mut store = cameras();
    let current = camera_with(&store, args.camera)?;
    let name = match &args.name {
        Some(raw) => {
            let name = check_lower_snake("name", raw)?;
            name_taken(&store, &name, Some(current.id))?;
            name
        }
        None => current.name.clone(),
    };
    let eye = args.eye.unwrap_or(current.eye);
    let target = args.target.unwrap_or(current.target);
    check_eye_target(eye, target)?;
    let fov_deg_x100 = args.fov_deg_x100.unwrap_or(current.fov_deg_x100);
    let near_cm = args.near_cm.unwrap_or(current.near_cm);
    let far_cm = args.far_cm.unwrap_or(current.far_cm);
    check_optics(fov_deg_x100, near_cm, far_cm)?;
    let node = CameraNode {
        id: current.id,
        name,
        eye,
        target,
        fov_deg_x100,
        near_cm,
        far_cm,
    };
    if let Some(slot) = store
        .cameras
        .iter_mut()
        .find(|camera| camera.id == current.id)
    {
        *slot = node.clone();
    }
    Ok(CameraSaveOutput {
        camera: CameraRow::of(&node),
        cameras: u32::try_from(store.cameras.len()).unwrap_or(u32::MAX),
    })
}

/// Every live camera, id order, plus the live shot count.
fn run_camera_list() -> CameraListOutput {
    let store = cameras();
    CameraListOutput {
        cameras: store.cameras.iter().map(CameraRow::of).collect(),
        shots: u32::try_from(store.shots.len()).unwrap_or(u32::MAX),
    }
}

/// Drop one camera and every shot that names it.
fn run_camera_remove(
    args: &CameraRemoveArgs,
) -> Result<CameraRemoveOutput, crate::mcp::McpToolError> {
    let mut store = cameras();
    let before = store.cameras.len();
    store.cameras.retain(|camera| camera.id != args.camera);
    let removed = store.cameras.len() < before;
    let shots_before = store.shots.len();
    store.shots.retain(|shot| shot.camera != args.camera);
    Ok(CameraRemoveOutput {
        removed,
        shots_dropped: u32::try_from(shots_before - store.shots.len()).unwrap_or(0),
        cameras: u32::try_from(store.cameras.len()).unwrap_or(u32::MAX),
    })
}

/// Add one shot over a live camera.
fn run_shot_add(args: &ShotAddArgs) -> Result<ShotCountOutput, crate::mcp::McpToolError> {
    let name = check_lower_snake("name", &args.name)?;
    if args.ticks == 0 || args.ticks > MAX_SHOT_TICKS {
        return Err(bad_args(format!(
            "ticks {} is outside 1..={MAX_SHOT_TICKS}; re-run with a shot length in ticks",
            args.ticks
        )));
    }
    let mut store = cameras();
    camera_with(&store, args.camera)?;
    if store.shots.len() >= MAX_SHOTS {
        return Err(bad_args(format!(
            "the shot list holds {MAX_SHOTS} shots; re-run scene.shot.remove on one first"
        )));
    }
    if store.shots.iter().any(|shot| shot.name == name) {
        return Err(bad_args(format!(
            "shot name `{name}` is taken; re-run with a unique lower_snake name"
        )));
    }
    store.shots.push(Shot {
        camera: args.camera,
        ticks: args.ticks,
        name,
    });
    Ok(ShotCountOutput {
        removed: false,
        shots: u32::try_from(store.shots.len()).unwrap_or(u32::MAX),
    })
}

/// Every live shot, name order.
fn run_shot_list() -> ShotListOutput {
    let store = cameras();
    let mut shots: Vec<ShotRow> = store
        .shots
        .iter()
        .map(|shot| ShotRow {
            camera: shot.camera,
            ticks: shot.ticks,
            name: shot.name.clone(),
        })
        .collect();
    shots.sort_by(|a, b| a.name.cmp(&b.name).then(a.camera.cmp(&b.camera)));
    ShotListOutput { shots }
}

/// Drop one shot by name.
fn run_shot_remove(args: &ShotRemoveArgs) -> Result<ShotCountOutput, crate::mcp::McpToolError> {
    let name = args.name.trim();
    let mut store = cameras();
    let before = store.shots.len();
    store.shots.retain(|shot| shot.name != name);
    if store.shots.len() == before {
        let names = store
            .shots
            .iter()
            .map(|shot| shot.name.clone())
            .collect::<Vec<_>>()
            .join(", ");
        return Err(bad_args(format!(
            "shot `{name}` is not live; legal names are [{}]; re-run scene.shot.list for the live \
             shots",
            if names.is_empty() {
                "none".to_owned()
            } else {
                names
            }
        )));
    }
    Ok(ShotCountOutput {
        removed: true,
        shots: u32::try_from(store.shots.len()).unwrap_or(u32::MAX),
    })
}

/// The scene bounds in cm as `min x, y, z, max x, y, z`, from the same
/// snapshot `scene.brief` reads; `None` when no node has a finite pose.
fn scene_bounds_cm(doc: &engine_editor::commands::Document) -> Option<[i32; 6]> {
    let snap = crate::scene_tools::snapshot(doc);
    let mut bounds: Option<[f32; 6]> = None;
    for actor in &snap.actors {
        let sides = [actor.x, actor.y, actor.z, actor.sx, actor.sy, actor.sz];
        if !sides.iter().all(|value| value.is_finite()) {
            continue;
        }
        let lo = [
            (actor.x - actor.sx) * 100.0,
            (actor.y - actor.sy) * 100.0,
            (actor.z - actor.sz) * 100.0,
        ];
        let hi = [
            (actor.x + actor.sx) * 100.0,
            (actor.y + actor.sy) * 100.0,
            (actor.z + actor.sz) * 100.0,
        ];
        let next = match bounds {
            None => [lo[0], lo[1], lo[2], hi[0], hi[1], hi[2]],
            Some(before) => [
                before[0].min(lo[0]),
                before[1].min(lo[1]),
                before[2].min(lo[2]),
                before[3].max(hi[0]),
                before[4].max(hi[1]),
                before[5].max(hi[2]),
            ],
        };
        bounds = Some(next);
    }
    bounds.map(|b| b.map(cm_of))
}

/// A whole number as `f32`.
fn f32_of(value: u32) -> f32 {
    engine_core::checked::f32_from_u64(u64::from(value))
}

/// A signed cm value as `f32` cm.
fn f32_cm(value: i32) -> f32 {
    engine_core::checked::f32_from_f64(f64::from(value))
}

/// One `f32` meter value as rounded cm.
fn cm_of(value: f32) -> i32 {
    let scaled = (f64::from(value) * 100.0).round();
    if scaled < 0.0 {
        let magnitude = engine_core::checked::saturating_f64_to_u64(-scaled).min(2_147_483_648);
        -i32::try_from(magnitude).unwrap_or(i32::MIN)
    } else {
        i32::try_from(engine_core::checked::saturating_f64_to_u64(scaled)).unwrap_or(i32::MAX)
    }
}

/// A finite `f32` as a rounded `u32`, clamped to `0..=max`.
fn whole(value: f32, max: u32) -> u32 {
    let clamped = f64::from(value).round().clamp(0.0, f64::from(max));
    u32::try_from(engine_core::checked::saturating_f64_to_u64(clamped)).unwrap_or(max)
}

/// The camera's view axes: forward down the eye-to-target line, right and up.
fn view_axes(
    eye: [f32; 3],
    target: [f32; 3],
) -> Result<([f32; 3], [f32; 3], [f32; 3]), crate::mcp::McpToolError> {
    let dir = [target[0] - eye[0], target[1] - eye[1], target[2] - eye[2]];
    let len = (dir[0] * dir[0] + dir[1] * dir[1] + dir[2] * dir[2]).sqrt();
    if !(len > 1e-4) {
        return Err(bad_args(
            "eye and target are the same point; re-run with a target down the arena".to_owned(),
        ));
    }
    let forward = [dir[0] / len, dir[1] / len, dir[2] / len];
    let cross = |a: [f32; 3], b: [f32; 3]| {
        [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ]
    };
    let mut right = cross(forward, [0.0, 1.0, 0.0]);
    let right_len = (right[0] * right[0] + right[1] * right[1] + right[2] * right[2]).sqrt();
    if right_len > 1e-4 {
        right = [
            right[0] / right_len,
            right[1] / right_len,
            right[2] / right_len,
        ];
    } else {
        right = [1.0, 0.0, 0.0];
    }
    let up = cross(right, forward);
    Ok((forward, right, up))
}

/// Score one camera against the scene bounds for a viewport.
fn run_camera_frame(
    doc: &engine_editor::commands::Document,
    args: &CameraFrameArgs,
) -> Result<CameraFrameOutput, crate::mcp::McpToolError> {
    if !(16..=4096).contains(&args.width) || !(16..=4096).contains(&args.height) {
        return Err(bad_args(format!(
            "viewport {}x{} is outside 16..=4096 per side; re-run with the width and height you \
             shoot at",
            args.width, args.height
        )));
    }
    let node = camera_with(&cameras(), args.camera)?;
    let (forward, right, up) = view_axes(node.eye, node.target)?;
    let bounds = scene_bounds_cm(doc).unwrap_or([0; 6]);
    let aspect = f32_of(args.width) / f32_of(args.height);
    let half_x = (f32_of(node.fov_deg_x100) / 100.0 * std::f32::consts::PI / 360.0).tan();
    let half_y = half_x / aspect;
    let dot = |a: [f32; 3], b: [f32; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    let corners = [
        [bounds[0], bounds[1], bounds[2]],
        [bounds[3], bounds[1], bounds[2]],
        [bounds[0], bounds[4], bounds[2]],
        [bounds[3], bounds[4], bounds[2]],
        [bounds[0], bounds[1], bounds[5]],
        [bounds[3], bounds[1], bounds[5]],
        [bounds[0], bounds[4], bounds[5]],
        [bounds[3], bounds[4], bounds[5]],
    ];
    let (mut in_front, mut visible) = (0u8, 0u8);
    let (mut x0, mut y0, mut x1, mut y1) = (args.width, args.height, 0u32, 0u32);
    let mut any = false;
    for corner in corners {
        let v = [
            f32_cm(corner[0]) - node.eye[0] * 100.0,
            f32_cm(corner[1]) - node.eye[1] * 100.0,
            f32_cm(corner[2]) - node.eye[2] * 100.0,
        ];
        let z = dot(v, forward);
        if z <= f32_of(node.near_cm) {
            continue;
        }
        in_front = in_front.saturating_add(1);
        let ndc_x = dot(v, right) / z / half_x;
        let ndc_y = dot(v, up) / z / half_y;
        if ndc_x.abs() <= 1.0 && ndc_y.abs() <= 1.0 {
            visible = visible.saturating_add(1);
        }
        let px = whole((ndc_x + 1.0) * 0.5 * f32_of(args.width), args.width);
        let py = whole((1.0 - ndc_y) * 0.5 * f32_of(args.height), args.height);
        any = true;
        x0 = x0.min(px);
        y0 = y0.min(py);
        x1 = x1.max(px);
        y1 = y1.max(py);
    }
    let (box_px, framing) = if any {
        let mask = crate::packs::view::art::box_mask(args.width, args.height, x0, y0, x1, y1);
        let energy = crate::packs::view::art::box_row_energy(&mask, args.width, args.height);
        let framing = crate::packs::view::art::framing_of(&mask, args.width, args.height, &energy);
        (Some([x0, y0, x1, y1]), Some(framing))
    } else {
        (None, None)
    };
    let fov_y_deg_x100 = whole(
        (half_y.atan() * 2.0 * 180.0 / std::f32::consts::PI) * 100.0,
        MAX_FOV_X100,
    );
    Ok(CameraFrameOutput {
        camera: node.id,
        name: node.name,
        width: args.width,
        height: args.height,
        fov_deg_x100: node.fov_deg_x100,
        fov_y_deg_x100,
        aspect_x1000: whole(aspect * 1000.0, u32::MAX),
        near_cm: node.near_cm,
        far_cm: node.far_cm,
        forward,
        right,
        up,
        bounds_cm: bounds,
        corners_in_front: in_front,
        corners_visible: visible,
        box_px,
        framing,
    })
}

fn camera_add(
    service: &crate::mcp::McpServer,
    args: &CameraAddArgs,
) -> Result<CameraSaveOutput, crate::mcp::McpToolError> {
    service.dispatch(TOOL_CAMERA_ADD, |_state| run_camera_add(args))
}

fn camera_set(
    service: &crate::mcp::McpServer,
    args: &CameraSetArgs,
) -> Result<CameraSaveOutput, crate::mcp::McpToolError> {
    service.dispatch(TOOL_CAMERA_SET, |_state| run_camera_set(args))
}

fn camera_list(
    service: &crate::mcp::McpServer,
    _args: &CameraListArgs,
) -> Result<CameraListOutput, crate::mcp::McpToolError> {
    service.dispatch(TOOL_CAMERA_LIST, |_state| Ok(run_camera_list()))
}

fn camera_remove(
    service: &crate::mcp::McpServer,
    args: &CameraRemoveArgs,
) -> Result<CameraRemoveOutput, crate::mcp::McpToolError> {
    service.dispatch(TOOL_CAMERA_REMOVE, |_state| run_camera_remove(args))
}

fn camera_frame(
    service: &crate::mcp::McpServer,
    args: &CameraFrameArgs,
) -> Result<CameraFrameOutput, crate::mcp::McpToolError> {
    service.dispatch(TOOL_CAMERA_FRAME, |state| {
        run_camera_frame(&state.doc, args)
    })
}

fn shot_add(
    service: &crate::mcp::McpServer,
    args: &ShotAddArgs,
) -> Result<ShotCountOutput, crate::mcp::McpToolError> {
    service.dispatch(TOOL_SHOT_ADD, |_state| run_shot_add(args))
}

fn shot_list(
    service: &crate::mcp::McpServer,
    _args: &ShotListArgs,
) -> Result<ShotListOutput, crate::mcp::McpToolError> {
    service.dispatch(TOOL_SHOT_LIST, |_state| Ok(run_shot_list()))
}

fn shot_remove(
    service: &crate::mcp::McpServer,
    args: &ShotRemoveArgs,
) -> Result<ShotCountOutput, crate::mcp::McpToolError> {
    service.dispatch(TOOL_SHOT_REMOVE, |_state| run_shot_remove(args))
}

crate::e2_tool!(
    /// `scene.camera.add` tool: add a camera node. In memory only.
    CameraAddTool,
    name = TOOL_CAMERA_ADD,
    description = "Add a camera node: lower_snake name, eye and target in meters, fov_deg_x100 100..=17900, near_cm at least 1 under far_cm at most 1000000. Up to 16 cameras, ids monotonic from 1. In memory only: no file, no shell, forgotten on restart.",
    readonly = false,
    args = CameraAddArgs,
    output = CameraSaveOutput,
    invoke = camera_add
);

crate::e2_tool!(
    /// `scene.camera.set` tool: change one camera's fields.
    CameraSetTool,
    name = TOOL_CAMERA_SET,
    description = "Change one camera by id: any of name, eye, target, fov_deg_x100, near_cm, far_cm; the merged camera keeps the same id and is validated as a whole. In memory only.",
    readonly = false,
    args = CameraSetArgs,
    output = CameraSaveOutput,
    invoke = camera_set
);

crate::e2_tool!(
    /// `scene.camera.list` tool: every camera, id order.
    CameraListTool,
    name = TOOL_CAMERA_LIST,
    description = "List the live cameras by id: name, eye, target, fov_deg_x100, near_cm, far_cm, plus the live shot count. Read-only.",
    readonly = true,
    args = CameraListArgs,
    output = CameraListOutput,
    invoke = camera_list
);

crate::e2_tool!(
    /// `scene.camera.remove` tool: drop one camera and its shots.
    CameraRemoveTool,
    name = TOOL_CAMERA_REMOVE,
    description = "Drop one camera by id and every shot that names it, reporting the camera count and the shots dropped. In memory only.",
    readonly = false,
    args = CameraRemoveArgs,
    output = CameraRemoveOutput,
    invoke = camera_remove
);

crate::e2_tool!(
    /// `scene.camera.frame` tool: view numbers and a framing self-check.
    CameraFrameTool,
    name = TOOL_CAMERA_FRAME,
    description = "Read one camera for a viewport: fov_y, aspect, near and far, the forward, right and up axes, the scene bounds in cm, the bounds corners in front and visible, the projected bounds box and a view.frame-style framing score over that box (coverage, headroom, thirds, horizon, weighted total, worst part). Read-only.",
    readonly = true,
    args = CameraFrameArgs,
    output = CameraFrameOutput,
    invoke = camera_frame
);

crate::e2_tool!(
    /// `scene.shot.add` tool: add a shot over a live camera.
    ShotAddTool,
    name = TOOL_SHOT_ADD,
    description = "Add a shot: a live camera id, ticks 1..=3600 and a unique lower_snake name. Up to 64 shots. In memory only.",
    readonly = false,
    args = ShotAddArgs,
    output = ShotCountOutput,
    invoke = shot_add
);

crate::e2_tool!(
    /// `scene.shot.list` tool: every shot, name order.
    ShotListTool,
    name = TOOL_SHOT_LIST,
    description = "List the live shots by name: camera id and ticks. Read-only.",
    readonly = true,
    args = ShotListArgs,
    output = ShotListOutput,
    invoke = shot_list
);

crate::e2_tool!(
    /// `scene.shot.remove` tool: drop one shot by name.
    ShotRemoveTool,
    name = TOOL_SHOT_REMOVE,
    description = "Drop one shot by name and report the live count; a name that is not live is refused with the legal names. In memory only.",
    readonly = false,
    args = ShotRemoveArgs,
    output = ShotCountOutput,
    invoke = shot_remove
);

#[cfg(test)]
mod tests {
    use super::*;
    use engine_scene::component::ALL_KINDS;
    use rmcp::handler::server::router::tool::SyncTool;

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

    fn camera_args(name: &str) -> CameraAddArgs {
        CameraAddArgs {
            name: name.to_owned(),
            eye: [0.0, 200.0, -800.0],
            target: [0.0, 100.0, 0.0],
            fov_deg_x100: 6000,
            near_cm: 10,
            far_cm: 100_000,
        }
    }

    #[test]
    fn cameras_add_list_set_and_remove_with_stable_ids() {
        let first = run_camera_add(&camera_args("e12_main")).expect("first camera");
        let second = run_camera_add(&camera_args("e12_wide")).expect("second camera");
        assert!(second.camera.id > first.camera.id);
        assert_eq!(first.camera.name, "e12_main");
        let listed = run_camera_list();
        assert!(listed.cameras.iter().any(|row| row.id == first.camera.id));
        assert_eq!(listed, run_camera_list());
        let changed = run_camera_set(&CameraSetArgs {
            camera: first.camera.id,
            fov_deg_x100: Some(7000),
            ..CameraSetArgs::default()
        })
        .expect("set");
        assert_eq!(changed.camera.id, first.camera.id);
        assert_eq!(changed.camera.fov_deg_x100, 7000);
        let removed = run_camera_remove(&CameraRemoveArgs {
            camera: second.camera.id,
        })
        .expect("remove");
        assert!(removed.removed);
        assert!(run_camera_list()
            .cameras
            .iter()
            .all(|row| row.id != second.camera.id));
    }

    #[test]
    fn cameras_refuse_a_duplicate_name_and_bad_optics() {
        // Another test may already hold `e12_dup`: only the second add must fail.
        let held = run_camera_add(&camera_args("e12_dup")).is_ok();
        assert!(held, "a fresh store accepts the name");
        let duplicate = run_camera_add(&camera_args("e12_dup")).expect_err("duplicate name");
        assert_eq!(duplicate.code, "bad_args");
        assert!(duplicate.message.contains("taken"), "{}", duplicate.message);
        let mut bad = camera_args("e12_bad_fov");
        bad.fov_deg_x100 = 99;
        let fov = run_camera_add(&bad).expect_err("fov");
        assert_eq!(fov.code, "bad_args");
        assert!(fov.message.contains("100..=17900"), "{}", fov.message);
        assert!(fov.message.contains("re-run"), "{}", fov.message);
        let mut bad = camera_args("e12_bad_near");
        bad.near_cm = 500_000;
        bad.far_cm = 10;
        assert_eq!(run_camera_add(&bad).expect_err("near").code, "bad_args");
        let bad_name = camera_args("Camera Upper");
        let name = run_camera_add(&bad_name).expect_err("name");
        assert_eq!(name.code, "bad_args");
        assert!(name.message.contains("lower_snake"), "{}", name.message);
    }

    #[test]
    fn shots_need_a_live_camera_and_list_by_name() {
        let camera = run_camera_add(&camera_args("e12_shot_cam")).expect("camera");
        let missing = run_shot_add(&ShotAddArgs {
            camera: camera.camera.id.saturating_add(999),
            ticks: 60,
            name: "e12_orphan".to_owned(),
        })
        .expect_err("missing camera");
        assert_eq!(missing.code, "unknown_camera");
        assert!(missing.message.contains("legal ids"), "{}", missing.message);
        assert!(
            missing.message.contains("scene.camera.list"),
            "{}",
            missing.message
        );
        let zero = run_shot_add(&ShotAddArgs {
            camera: camera.camera.id,
            ticks: 0,
            name: "e12_zero".to_owned(),
        })
        .expect_err("zero ticks");
        assert_eq!(zero.code, "bad_args");
        assert!(zero.message.contains("1..=3600"), "{}", zero.message);
        for (name, ticks) in [("e12_b_shot", 120u32), ("e12_a_shot", 60)] {
            run_shot_add(&ShotAddArgs {
                camera: camera.camera.id,
                ticks,
                name: name.to_owned(),
            })
            .expect("shot");
        }
        let listed = run_shot_list();
        let mine: Vec<&str> = listed
            .shots
            .iter()
            .map(|shot| shot.name.as_str())
            .filter(|name| name.starts_with("e12_"))
            .collect();
        assert_eq!(mine, vec!["e12_a_shot", "e12_b_shot"]);
        let removed = run_shot_remove(&ShotRemoveArgs {
            name: "e12_a_shot".to_owned(),
        })
        .expect("remove shot");
        assert!(removed.removed);
        let dropped = run_camera_remove(&CameraRemoveArgs {
            camera: camera.camera.id,
        })
        .expect("remove cam");
        assert!(dropped.shots_dropped >= 1, "{dropped:?}");
        let unknown = run_shot_remove(&ShotRemoveArgs {
            name: "e12_gone".to_owned(),
        })
        .expect_err("gone shot");
        assert!(
            unknown.message.contains("legal names"),
            "{}",
            unknown.message
        );
    }

    #[test]
    fn camera_frame_reads_the_view_and_scores_the_bounds() {
        let camera = run_camera_add(&camera_args("e12_frame")).expect("camera");
        let server =
            crate::mcp::McpServer::new(7, engine_editor::capability::Capability::unrestricted(7));
        let args = || CameraFrameArgs {
            camera: camera.camera.id,
            width: 640,
            height: 360,
        };
        let first = CameraFrameTool::invoke(&server, args()).expect("frame");
        assert_eq!(first.fov_deg_x100, 6000);
        assert_eq!(first.aspect_x1000, 1778);
        assert!(
            (3400..=3800).contains(&first.fov_y_deg_x100),
            "fov_y {}",
            first.fov_y_deg_x100
        );
        assert!(first.corners_in_front >= 1, "{first:?}");
        let framing = first.framing.as_ref().expect("bounds project");
        assert!(framing.subject_pixels > 0);
        assert!(framing.total_permille <= 1000);
        assert_eq!(
            first,
            CameraFrameTool::invoke(&server, args()).expect("twice")
        );
        let bad = CameraFrameTool::invoke(
            &server,
            CameraFrameArgs {
                camera: camera.camera.id,
                width: 8,
                height: 360,
            },
        )
        .expect_err("narrow viewport");
        assert_eq!(bad.code, "bad_args");
        assert!(bad.message.contains("16..=4096"), "{}", bad.message);
        let unknown = CameraFrameTool::invoke(
            &server,
            CameraFrameArgs {
                camera: camera.camera.id.saturating_add(999),
                width: 640,
                height: 360,
            },
        )
        .expect_err("unknown camera");
        assert_eq!(unknown.code, "unknown_camera");
        assert!(unknown.message.contains("legal ids"), "{}", unknown.message);
    }
}
