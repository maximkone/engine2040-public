//! `anim` pack: animation data an AI can edit.
//!
//! The state machine verbs (`anim.graph.set`, `anim.graph.undo`,
//! `anim.graph.get`, `anim.preview`) wrap `engine_anim::state_machine`: a
//! versioned JSON machine (states, transitions with switch mode, condition,
//! fade ticks; integer ticks only) that is validated before it is kept, undoable,
//! and previewable on a scripted tape without being applied. The machine lives
//! per door session in a side table keyed by the session id, so the core door
//! state is untouched; a refused set keeps the last good graph.
//!
//! The clip verbs (`anim.clip.trim`, `anim.clip.retime`, `anim.clip.mirror`,
//! `anim.clip.undo`) wrap `engine_anim::edit` over a per-session clip seeded
//! from a fixture ramp: each verb returns the same `qclip_hash` the library
//! call gives on the same fixture, keeps the last good clip on a refusal, and
//! is undoable.
//!
//! The retarget verbs score a third-party skeleton onto the 17 bone roles of
//! `engine_anim::contract`; `anim.retarget.preview` (in [`preview`]) draws one
//! scored map as a real PNG frame under the project root.
//! Errors: [`McpToolError`] with the machine's own code (`bad_document`,
//! `invalid_machine`, `no_graph`, `bad_script`, `preview_failed`,
//! `nothing_to_undo`), the document path and one recovery step.

use std::collections::BTreeMap;
use std::sync::{Mutex, OnceLock};

use engine_anim::clip::{bake_clip, bake_track, qclip_hash, QClip, TICK_DT};
use engine_anim::clip_rot::{bake_rot_blob, rot_blob_hash, sample_rot_blob};
use engine_anim::contract::{
    bone_coverage, bone_role_for_name, AnimError, BoneMapping, BoneRole, MAX_JOINTS,
    REQUIRED_BONE_ROLES,
};
use engine_anim::edit;
use engine_anim::pose::QuatTrack;
use engine_anim::state_machine::anim_door::{AnimDoor, DoorRefusal};
use engine_anim::timeline::{Easing, Key, Timeline, TimelineDoc};
use engine_core::checked::{f32_from_f64, f32_from_u64};
use rmcp::schemars::JsonSchema;
use rmcp::serde::de::DeserializeOwned;
use rmcp::serde::{Deserialize, Serialize};
use rmcp::serde_json::Value;

use super::{PackDef, PackError, VerbSpec};
use crate::mcp::{McpServer, McpToolError};
use crate::tool_capabilities::CapabilityCard;

mod preview;

/// Pack name.
pub const PACK: &str = "anim";
/// Verb: validate a JSON state machine and make it current (undoable).
pub const TOOL_ANIM_GRAPH_SET: &str = "anim.graph.set";
/// Verb: undo the last `anim.graph.set`.
pub const TOOL_ANIM_GRAPH_UNDO: &str = "anim.graph.undo";
/// Verb: read the current state machine as canonical JSON.
pub const TOOL_ANIM_GRAPH_GET: &str = "anim.graph.get";
/// Verb: run a scripted tape on a graph without applying it.
pub const TOOL_ANIM_PREVIEW: &str = "anim.preview";
/// Verb: trim the session clip to a tick window.
pub const TOOL_ANIM_CLIP_TRIM: &str = "anim.clip.trim";
/// Verb: scale the session clip's key times by a ratio.
pub const TOOL_ANIM_CLIP_RETIME: &str = "anim.clip.retime";
/// Verb: mirror the session clip left to right.
pub const TOOL_ANIM_CLIP_MIRROR: &str = "anim.clip.mirror";
/// Verb: undo the last session clip edit.
pub const TOOL_ANIM_CLIP_UNDO: &str = "anim.clip.undo";
/// Verb: bake and sample a joint's rotation sidecar from a clip JSON.
pub const TOOL_ANIM_ROT_PREVIEW: &str = "anim.rot.preview";
/// Verb: map a third-party skeleton onto the 17 bone roles.
pub const TOOL_ANIM_RETARGET: &str = "anim.retarget";

/// Args for `anim.graph.set`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct GraphSetArgs {
    /// The machine: `{"version":1,"name":..,"start":..,"params":[{"name","default","min","max"}],
    /// "states":[{"name","clip"|"node","length_ticks","looping"}],
    /// "transitions":[{"from","to","switch":"immediate|sync|at_end","advance":"disabled|enabled|auto",
    /// "when":"always"|{"param","op","value"}|{"time_ge":n}|{"not":c}|{"all":[..]}|{"any":[..]},
    /// "fade_ticks","curve":"linear|ease_in|ease_out|ease_in_out","priority","reset"}]}`.
    /// Integers only; unknown fields are refused with their path.
    pub graph: Value,
}

/// Result of `anim.graph.set`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct GraphSetOutput {
    /// Digest (16 hex digits) before, null for the first graph.
    pub before: Option<String>,
    /// Digest (16 hex digits) after.
    pub after: String,
    /// State count.
    pub states: u32,
    /// Transition count.
    pub transitions: u32,
    /// Non-fatal findings (unreachable states, dead ends).
    pub lints: Vec<String>,
}

/// Args for the verbs that take nothing.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
#[expect(
    clippy::empty_structs_with_brackets,
    reason = "serde reads the wire shape `{}`; a unit struct reads null"
)]
pub struct NoArgs {}

/// Result of `anim.graph.undo`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct GraphUndoOutput {
    /// Digest (16 hex digits) after the undo.
    pub after: String,
    /// Edits still undoable.
    pub undo_depth: u32,
}

/// Result of `anim.graph.get`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct GraphGetOutput {
    /// The current machine, null before the first set.
    pub graph: Option<Value>,
}

/// Args for `anim.preview`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct PreviewArgs {
    /// A candidate machine to run instead of the current one (not applied).
    #[serde(default)]
    pub graph: Option<Value>,
    /// The tape: `{"ticks":N,"params":[{"at":T,"name":..,"value":..}],"travel":[{"at":T,"to":..}]}`;
    /// at most 10000 ticks; writes and travel requests apply before tick `at` runs.
    pub script: Value,
    /// Ticks to sample the session timeline at, when one is set: the pose comes from the
    /// same clip sampler the playback uses. A session without one refuses `no_timeline`.
    #[serde(default)]
    pub timeline_ticks: Option<Vec<u32>>,
}

/// One transition seen in a preview.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct PreviewEvent {
    /// Tick the transition started on.
    pub tick: u32,
    /// State left.
    pub from: String,
    /// State entered.
    pub to: String,
    /// Cross-fade length in ticks.
    pub fade_ticks: u32,
}

/// Result of `anim.preview`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct PreviewOutput {
    /// Every transition in order.
    pub events: Vec<PreviewEvent>,
    /// State at the end of the tape.
    pub final_state: String,
    /// Playback digest (16 hex digits): equal on every replay.
    pub digest: String,
    /// Timeline poses at `timeline_ticks`, one per tick; empty when none were asked.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub timeline_samples: Vec<TimelineSample>,
}

fn refuse(refusal: DoorRefusal) -> McpToolError {
    McpToolError::new(
        refusal.code,
        format!("{} Next: {}.", refusal.message, refusal.recovery),
    )
}

/// Pre-apply cost caps for one `anim.graph.set`: hostile graph sets refuse
/// before the door mutates anything. Caps sit well under the underlying
/// `engine_anim` limits (128 states, 1024 transitions, 600 fade ticks per
/// transition, 1MiB JSON) so legal fighter graphs (2-4 states) pass with
/// headroom while giant sets fail cheap without touching the door.
const MAX_GRAPH_STATES: u32 = 32;
/// Most transitions one pre-apply graph may hold.
const MAX_GRAPH_TRANSITIONS: u32 = 64;
/// Most summed `fade_ticks` over all transitions, in ticks.
const MAX_GRAPH_FADE_SUM_TICKS: u64 = 4_800;
/// Most canonical JSON bytes one pre-apply graph may hold.
const MAX_GRAPH_BYTES: usize = 16_384;

/// Cost of one candidate graph: state/transition counts, summed fade ticks,
/// and estimated bytes from the canonical JSON length. Pure: reads only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct GraphCost {
    /// States in `states`.
    states: u32,
    /// Transitions in `transitions`.
    transitions: u32,
    /// Summed `fade_ticks` over all transitions, saturating.
    fade_sum: u64,
    /// Canonical JSON length in bytes.
    bytes: usize,
}

/// Count the cost of one candidate graph. Pure: the same input seals the same
/// cost on every machine; missing lists count zero so shape errors stay with
/// the validator; integer math only, no clock.
fn graph_cost(graph: &Value) -> GraphCost {
    let states = graph
        .get("states")
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    let transitions_list = graph.get("transitions").and_then(Value::as_array);
    let transitions = transitions_list.map_or(0, Vec::len);
    let mut fade_sum: u64 = 0;
    if let Some(list) = transitions_list {
        for item in list {
            let fade = item.get("fade_ticks").and_then(Value::as_u64).unwrap_or(0);
            fade_sum = fade_sum.saturating_add(fade);
        }
    }
    GraphCost {
        states: u32::try_from(states).unwrap_or(u32::MAX),
        transitions: u32::try_from(transitions).unwrap_or(u32::MAX),
        fade_sum,
        bytes: graph.to_string().len(),
    }
}

/// Caps summary for refusal messages, so the caller sees the legal values
/// without a second round trip.
fn graph_caps_summary() -> String {
    format!(
        "caps states<={MAX_GRAPH_STATES} transitions<={MAX_GRAPH_TRANSITIONS} \
         fade_sum<={MAX_GRAPH_FADE_SUM_TICKS} bytes<={MAX_GRAPH_BYTES}"
    )
}

/// Pre-apply gate over one cost: refuse hostile graphs before `anim.graph.set`
/// touches the door. Errors: `graph_too_large` naming the violated invariant
/// plus the caps and one recovery; the door and the last-good graph are
/// untouched because the caller returns before `with_door`.
fn check_graph_cost(cost: &GraphCost) -> Result<(), McpToolError> {
    let caps = graph_caps_summary();
    let recovery = "shrink the graph to the caps and call anim.graph.set again; \
        the last good graph is kept";
    if cost.states > MAX_GRAPH_STATES {
        return Err(McpToolError::new(
            "graph_too_large",
            format!(
                "graph_too_large: states {} exceeds cap {MAX_GRAPH_STATES}; {caps}; \
                 recovery: {recovery}",
                cost.states,
            ),
        ));
    }
    if cost.transitions > MAX_GRAPH_TRANSITIONS {
        return Err(McpToolError::new(
            "graph_too_large",
            format!(
                "graph_too_large: transitions {} exceeds cap {MAX_GRAPH_TRANSITIONS}; {caps}; \
                 recovery: {recovery}",
                cost.transitions,
            ),
        ));
    }
    if cost.fade_sum > MAX_GRAPH_FADE_SUM_TICKS {
        return Err(McpToolError::new(
            "graph_too_large",
            format!(
                "graph_too_large: fade_sum {} exceeds cap {MAX_GRAPH_FADE_SUM_TICKS}; {caps}; \
                 recovery: {recovery}",
                cost.fade_sum,
            ),
        ));
    }
    if cost.bytes > MAX_GRAPH_BYTES {
        return Err(McpToolError::new(
            "graph_too_large",
            format!(
                "graph_too_large: bytes {} exceeds cap {MAX_GRAPH_BYTES}; {caps}; \
                 recovery: {recovery}",
                cost.bytes,
            ),
        ));
    }
    Ok(())
}

fn typed<T: DeserializeOwned>(tool: &str, value: Value) -> Result<T, McpToolError> {
    rmcp::serde_json::from_value(value).map_err(|error| {
        McpToolError::new(
            "bad_output",
            format!(
                "`{tool}` produced output outside its schema ({error}); re-run and report the verb"
            ),
        )
    })
}

/// Run `f` on this session's machine under the audited dispatch seam.
fn with_door<R>(
    server: &McpServer,
    tool: &str,
    f: impl FnOnce(&mut AnimDoor) -> Result<R, DoorRefusal>,
) -> Result<R, McpToolError> {
    static DOORS: OnceLock<Mutex<BTreeMap<u64, AnimDoor>>> = OnceLock::new();
    server.dispatch(tool, |state| {
        let mut doors = DOORS.get_or_init(Mutex::default).lock().map_err(|_| {
            McpToolError::new(
                "lock_poisoned",
                "anim door table is poisoned; restart the door",
            )
        })?;
        f(doors.entry(state.session()).or_default()).map_err(refuse)
    })
}

fn graph_set(server: &McpServer, args: &GraphSetArgs) -> Result<GraphSetOutput, McpToolError> {
    check_graph_cost(&graph_cost(&args.graph))?;
    let text = args.graph.to_string();
    let value = with_door(server, TOOL_ANIM_GRAPH_SET, |door| door.set(&text))?;
    typed(TOOL_ANIM_GRAPH_SET, value)
}

fn graph_undo(server: &McpServer, _args: &NoArgs) -> Result<GraphUndoOutput, McpToolError> {
    let value = with_door(server, TOOL_ANIM_GRAPH_UNDO, AnimDoor::undo)?;
    typed(TOOL_ANIM_GRAPH_UNDO, value)
}

fn graph_get(server: &McpServer, _args: &NoArgs) -> Result<GraphGetOutput, McpToolError> {
    let graph = with_door(server, TOOL_ANIM_GRAPH_GET, |door| Ok(door.get()))?;
    Ok(GraphGetOutput { graph })
}

fn preview(server: &McpServer, args: &PreviewArgs) -> Result<PreviewOutput, McpToolError> {
    let graph = args.graph.as_ref().map(Value::to_string);
    let script = args.script.to_string();
    let value = with_door(server, TOOL_ANIM_PREVIEW, |door| {
        door.preview(graph.as_deref(), &script)
    })?;
    let mut output: PreviewOutput = typed(TOOL_ANIM_PREVIEW, value)?;
    if let Some(ticks) = &args.timeline_ticks {
        if ticks.len() > MAX_TIMELINE_SAMPLES {
            return Err(McpToolError::new(
                "too_many_samples",
                format!(
                    "{} sample ticks is past the cap {MAX_TIMELINE_SAMPLES}; recovery: ask for at \
                     most {MAX_TIMELINE_SAMPLES} ticks",
                    ticks.len()
                ),
            ));
        }
        output.timeline_samples = sample_timeline(server, TOOL_ANIM_PREVIEW, ticks)?;
    }
    Ok(output)
}

crate::e2_tool!(
    /// `anim.graph.set`: validate a JSON state machine and make it current.
    GraphSetTool,
    name = TOOL_ANIM_GRAPH_SET,
    description = "Set the animation state machine from JSON (states, transitions with switch mode, condition and fade ticks, integers only). Refused documents name the field path and keep the last good graph; undoable with anim.graph.undo.",
    readonly = false,
    args = GraphSetArgs,
    output = GraphSetOutput,
    invoke = graph_set
);

crate::e2_tool!(
    /// `anim.graph.undo`: undo the last `anim.graph.set` exactly.
    GraphUndoTool,
    name = TOOL_ANIM_GRAPH_UNDO,
    description = "Undo the last anim.graph.set; the graph digest returns to its earlier value exactly.",
    readonly = false,
    args = NoArgs,
    output = GraphUndoOutput,
    invoke = graph_undo
);

crate::e2_tool!(
    /// `anim.graph.get`: the current machine as canonical JSON.
    GraphGetTool,
    name = TOOL_ANIM_GRAPH_GET,
    description = "Read the current animation state machine as canonical JSON (null before the first anim.graph.set). No side effects.",
    readonly = true,
    args = NoArgs,
    output = GraphGetOutput,
    invoke = graph_get
);

crate::e2_tool!(
    /// `anim.preview`: run a scripted tape on a graph without applying it.
    PreviewTool,
    name = TOOL_ANIM_PREVIEW,
    description = "Run a scripted tape (parameter writes and travel requests by tick, at most 10000 ticks) on the current or a candidate state machine; returns every transition, the final state and a replay digest. Nothing is applied.",
    readonly = true,
    args = PreviewArgs,
    output = PreviewOutput,
    invoke = preview
);

// ---------------------------------------------------------------------------
// anim.clip.*: the clip edit verbs
// ---------------------------------------------------------------------------

/// The session's clip and the edits still undoable.
struct ClipDoor {
    clip: QClip,
    undo: Vec<QClip>,
    rev: u32,
}

impl Default for ClipDoor {
    fn default() -> Self {
        Self {
            clip: clip_fixture(),
            undo: Vec::new(),
            rev: 0,
        }
    }
}

impl ClipDoor {
    /// Replace the clip with `next`, keeping the old one so undo can restore it.
    fn install(&mut self, next: QClip) -> ClipEditOutput {
        let before = clip_digest(&self.clip);
        let after = clip_digest(&next);
        if before != after {
            if self.undo.len() == MAX_CLIP_UNDO {
                self.undo.remove(0);
            }
            self.undo.push(self.clip.clone());
            self.rev = self.rev.saturating_add(1);
            self.clip = next;
        }
        self.output(before, after)
    }

    fn undo(&mut self) -> Option<ClipEditOutput> {
        let before = clip_digest(&self.clip);
        let previous = self.undo.pop()?;
        let after = clip_digest(&previous);
        self.clip = previous;
        self.rev = self.rev.saturating_add(1);
        Some(self.output(before, after))
    }

    fn output(&self, before: String, after: String) -> ClipEditOutput {
        ClipEditOutput {
            before,
            after,
            rev: self.rev,
            tracks: u32::try_from(self.clip.tracks.len()).unwrap_or(u32::MAX),
            undo_depth: u32::try_from(self.undo.len()).unwrap_or(u32::MAX),
        }
    }
}

/// Most clip edits one session can undo.
const MAX_CLIP_UNDO: usize = 16;

/// The clip every session starts with: one track, 0..=60 ticks, a straight ramp.
fn clip_fixture() -> QClip {
    let mut times = Vec::with_capacity(61);
    let mut values = Vec::with_capacity(61);
    let mut t = 0.0;
    for _ in 0..=60 {
        times.push(t);
        values.push([t * 0.1, t * 0.2, t * 0.3]);
        t += TICK_DT;
    }
    bake_clip(vec![bake_track(times, values)])
}

fn clip_digest(clip: &QClip) -> String {
    format!("{:016x}", qclip_hash(clip))
}

/// Run `f` on this session's clip under the audited dispatch seam.
fn with_clip_door<R>(
    server: &McpServer,
    tool: &str,
    f: impl FnOnce(&mut ClipDoor) -> Result<R, McpToolError>,
) -> Result<R, McpToolError> {
    static CLIPS: OnceLock<Mutex<BTreeMap<u64, ClipDoor>>> = OnceLock::new();
    server.dispatch(tool, |state| {
        let mut clips = CLIPS.get_or_init(Mutex::default).lock().map_err(|_| {
            McpToolError::new(
                "lock_poisoned",
                "anim clip table is poisoned; restart the door",
            )
        })?;
        f(clips.entry(state.session()).or_default())
    })
}

/// Args for `anim.clip.trim`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct ClipTrimArgs {
    /// First tick kept; keys shift to 0.
    pub start_tick: u32,
    /// Last tick kept; must be above `start_tick`.
    pub end_tick: u32,
}

/// Args for `anim.clip.retime`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct ClipRetimeArgs {
    /// Time scale numerator, from 1.
    pub num: u32,
    /// Time scale denominator, from 1; the ratio must stay inside 1/8..=8.
    pub den: u32,
}

/// Result of one clip edit or undo.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct ClipEditOutput {
    /// Digest (16 hex digits) before the edit.
    pub before: String,
    /// Digest (16 hex digits) after the edit.
    pub after: String,
    /// Edits installed on this session's clip, from 0.
    pub rev: u32,
    /// Tracks in the clip now.
    pub tracks: u32,
    /// Edits still undoable.
    pub undo_depth: u32,
}

fn clip_trim(server: &McpServer, args: &ClipTrimArgs) -> Result<ClipEditOutput, McpToolError> {
    let (start, end) = (args.start_tick, args.end_tick);
    with_clip_door(server, TOOL_ANIM_CLIP_TRIM, |door| {
        let next = edit::trim(&door.clip, start, end).ok_or_else(|| {
            McpToolError::new(
                "bad_clip",
                format!(
                    "trim window {start}..{end} is empty or reversed; pass start_tick < end_tick; the clip is kept"
                ),
            )
        })?;
        Ok(door.install(next))
    })
}

fn clip_retime(server: &McpServer, args: &ClipRetimeArgs) -> Result<ClipEditOutput, McpToolError> {
    let (num, den) = (args.num, args.den);
    with_clip_door(server, TOOL_ANIM_CLIP_RETIME, |door| {
        let next = edit::retime(&door.clip, num, den).ok_or_else(|| {
            McpToolError::new(
                "bad_clip",
                format!("retime ratio {num}/{den} is zero or outside 1/8..=8; the clip is kept"),
            )
        })?;
        Ok(door.install(next))
    })
}

fn clip_mirror(server: &McpServer, _args: &NoArgs) -> Result<ClipEditOutput, McpToolError> {
    with_clip_door(server, TOOL_ANIM_CLIP_MIRROR, |door| {
        Ok(door.install(edit::mirror(&door.clip)))
    })
}

fn clip_undo(server: &McpServer, _args: &NoArgs) -> Result<ClipEditOutput, McpToolError> {
    with_clip_door(server, TOOL_ANIM_CLIP_UNDO, |door| {
        door.undo().ok_or_else(|| {
            McpToolError::new(
                "nothing_to_undo",
                "no clip edit is undoable on this session; call anim.clip.trim, anim.clip.retime or anim.clip.mirror first",
            )
        })
    })
}

crate::e2_tool!(
    /// `anim.clip.trim`: keep one tick window of the session clip.
    ClipTrimTool,
    name = TOOL_ANIM_CLIP_TRIM,
    description = "Trim the session clip to ticks start_tick..end_tick, shifting keys to zero and sampling both ends. Returns the digest before and after. A reversed or empty window refuses bad_clip and keeps the clip; undoable.",
    readonly = false,
    args = ClipTrimArgs,
    output = ClipEditOutput,
    invoke = clip_trim
);

crate::e2_tool!(
    /// `anim.clip.retime`: scale every key time by a ratio.
    ClipRetimeTool,
    name = TOOL_ANIM_CLIP_RETIME,
    description = "Scale every key time of the session clip by num/den rounded to the nearest tick, inside the ratio 1/8..=8. Returns the digest before and after. A zero or out-of-range ratio refuses bad_clip and keeps the clip; undoable.",
    readonly = false,
    args = ClipRetimeArgs,
    output = ClipEditOutput,
    invoke = clip_retime
);

crate::e2_tool!(
    /// `anim.clip.mirror`: mirror the session clip left to right.
    ClipMirrorTool,
    name = TOOL_ANIM_CLIP_MIRROR,
    description = "Mirror the session clip left to right (x and y swap sign and track order reverses). Returns the digest before and after; undoable.",
    readonly = false,
    args = NoArgs,
    output = ClipEditOutput,
    invoke = clip_mirror
);

crate::e2_tool!(
    /// `anim.clip.undo`: undo the last session clip edit.
    ClipUndoTool,
    name = TOOL_ANIM_CLIP_UNDO,
    description = "Undo the last clip edit on this session's clip; the digest returns to its value before that edit. Refuses nothing_to_undo when no edit is undoable.",
    readonly = false,
    args = NoArgs,
    output = ClipEditOutput,
    invoke = clip_undo
);

/// Args for `anim.rot.preview`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct RotPreviewArgs {
    /// Path to a QClip JSON: `{"tracks":[{"times":[..]}, ..],"dur":1.0}`.
    pub clip_path: String,
    /// Joint whose rotation lane is sampled; out of range is refused.
    pub joint: u16,
    /// Tick span the eight samples spread over; default 60.
    pub ticks: Option<u32>,
}

/// Result of `anim.rot.preview`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct RotPreviewOutput {
    /// Sample times in seconds, 8 entries.
    pub times: Vec<f32>,
    /// Sampled joint rotations, one per time.
    pub rot: Vec<[f32; 4]>,
    /// Socket offset turned by each sampled rotation; zero for a plain joint.
    pub socket_offset: Vec<[f32; 3]>,
    /// Digest (16 hex digits) of the serialized rotation blob.
    pub digest: String,
}

/// Samples one `anim.rot.preview` call returns.
const ROT_PREVIEW_SAMPLES: u32 = 8;
/// Tick span when the caller names none.
const ROT_PREVIEW_DEFAULT_TICKS: u32 = 60;

/// The clip JSON this verb reads: a QClip's `tracks` (only `times` is used)
/// and `dur`. Unknown clip keys such as `keys`, `min` and `max` are ignored,
/// so a full QClip document reads too.
#[derive(Debug, Deserialize)]
#[serde(crate = "rmcp::serde")]
struct ClipDoc {
    tracks: Vec<ClipTrackDoc>,
    dur: f32,
}

/// One clip track in the JSON; only its sample times feed the rotation bake.
#[derive(Debug, Deserialize)]
#[serde(crate = "rmcp::serde")]
struct ClipTrackDoc {
    times: Vec<f32>,
}

/// `anim.rot.preview`: bake the rotation sidecar for a clip JSON and sample one
/// joint over 8 evenly spaced ticks. Read-only; the clip file is only read.
fn rot_preview(
    _server: &McpServer,
    args: &RotPreviewArgs,
) -> Result<RotPreviewOutput, McpToolError> {
    let text = std::fs::read_to_string(&args.clip_path).map_err(|error| {
        McpToolError::new(
            "missing_clip",
            format!(
                "{} could not be read ({error}); recovery: create it with a `tracks` array, \
                 one entry per joint carrying `times`, and a `dur` in seconds",
                args.clip_path
            ),
        )
    })?;
    let doc: ClipDoc = rmcp::serde_json::from_str(&text).map_err(|error| {
        McpToolError::new(
            "bad_clip",
            format!(
                "{} is not a QClip JSON ({error}); recovery: keep the `tracks` key as an array \
                 of objects with a `times` array, plus `dur`",
                args.clip_path
            ),
        )
    })?;
    let mut tracks = Vec::with_capacity(doc.tracks.len());
    for track in &doc.tracks {
        tracks.push(bake_track(
            track.times.clone(),
            vec![[0.0; 3]; track.times.len()],
        ));
    }
    let clip = QClip {
        tracks,
        dur: doc.dur,
    };
    let rotations: Vec<QuatTrack> = clip
        .tracks
        .iter()
        .map(|track| QuatTrack {
            times: track.times.clone(),
            values: vec![[0.0, 0.0, 0.0, 1.0]; track.times.len()],
        })
        .collect();
    let blob = bake_rot_blob(&clip, &rotations, &[]);
    if usize::from(args.joint) >= blob.lanes.len() {
        return Err(McpToolError::new(
            "bad_joint",
            format!(
                "joint {} is outside 0..{} in {}; recovery: pass a joint index below {}",
                args.joint,
                blob.lanes.len(),
                args.clip_path,
                blob.lanes.len()
            ),
        ));
    }
    let ticks = args.ticks.unwrap_or(ROT_PREVIEW_DEFAULT_TICKS).max(1);
    let mut hint = 0usize;
    let mut times = Vec::new();
    let mut rot = Vec::new();
    let mut socket_offset = Vec::new();
    for i in 0..ROT_PREVIEW_SAMPLES {
        let tick = i * ticks / ROT_PREVIEW_SAMPLES;
        let t = f32_from_u64(u64::from(tick)) * TICK_DT;
        let Some((offset, q)) = sample_rot_blob(&blob, t, usize::from(args.joint), &mut hint)
        else {
            return Err(McpToolError::new(
                "bad_joint",
                "joint left the blob while sampling; recovery: pass a joint index below the clip's track count",
            ));
        };
        times.push(t);
        rot.push(q);
        socket_offset.push(offset);
    }
    Ok(RotPreviewOutput {
        times,
        rot,
        socket_offset,
        digest: format!("{:016x}", rot_blob_hash(&blob)),
    })
}

crate::e2_tool!(
    /// `anim.rot.preview`: sample a joint's rotation sidecar over 8 ticks.
    RotPreviewTool,
    name = TOOL_ANIM_ROT_PREVIEW,
    description = "Bake the rotation sidecar for a QClip JSON at clip_path and sample one joint over 8 evenly spaced ticks; returns the times, the joint rotation, the socket offset and the blob digest. Read-only, no file writes; a missing or malformed clip names the key to fix.",
    readonly = true,
    args = RotPreviewArgs,
    output = RotPreviewOutput,
    invoke = rot_preview
);

/// Verb: validate a keyframe timeline and make it the session's playback clip.
pub const TOOL_ANIM_TIMELINE_SET: &str = "anim.timeline.set";

/// Ticks one `anim.timeline.set` call samples at most.
const MAX_TIMELINE_SAMPLES: usize = 240;

/// Args for `anim.timeline.set`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct TimelineSetArgs {
    /// The document: `{"tracks":["hips","head"],"keys":[{"track":"hips","tick":0,
    /// "value":[0,1,0],"easing":"linear"}]}`. A key naming a track the document
    /// does not declare is refused by name; a track's keys must be
    /// non-decreasing in tick. `easing` is one of linear, step, ease_in,
    /// ease_out, ease_in_out (default linear).
    pub doc: Value,
    /// Ticks to sample after the set, at most 240; default the last key tick.
    /// Samples run through the same clip sampler the playback uses.
    #[serde(default)]
    pub sample_ticks: Option<Vec<u32>>,
}

/// One sampled tick of a timeline.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct TimelineSample {
    /// Tick sampled.
    pub tick: u32,
    /// One three-value pose per declared track, in `tracks` order.
    pub values: Vec<[f32; 3]>,
}

/// Result of `anim.timeline.set`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct TimelineSetOutput {
    /// Digest (16 hex digits) of the compiled clip.
    pub digest: String,
    /// Declared track count.
    pub tracks: u32,
    /// Key count.
    pub keys: u32,
    /// Last key tick.
    pub dur_ticks: u32,
    /// Sampled ticks, in the order asked.
    pub samples: Vec<TimelineSample>,
}

/// A timeline document outside the shape this verb reads.
fn bad_timeline(detail: String) -> McpToolError {
    McpToolError::new("bad_timeline", detail)
}

/// Read the wire document into the library's plain types, naming the path of
/// the first field that is outside its shape.
fn parse_timeline(doc: &Value) -> Result<TimelineDoc, McpToolError> {
    let Some(track_list) = doc.get("tracks").and_then(Value::as_array) else {
        return Err(bad_timeline(
            "`tracks` must be an array of track names; recovery: list every track once".to_owned(),
        ));
    };
    let mut tracks = Vec::with_capacity(track_list.len());
    for (index, name) in track_list.iter().enumerate() {
        let Some(name) = name.as_str() else {
            return Err(bad_timeline(format!(
                "`tracks[{index}]` is not a string; recovery: name each track with its own string"
            )));
        };
        tracks.push(name.to_owned());
    }
    let Some(key_list) = doc.get("keys").and_then(Value::as_array) else {
        return Err(bad_timeline(
            "`keys` must be an array of keys; recovery: give each key a track, tick and value"
                .to_owned(),
        ));
    };
    let mut keys = Vec::with_capacity(key_list.len());
    for (index, key) in key_list.iter().enumerate() {
        keys.push(parse_key(index, key)?);
    }
    Ok(TimelineDoc { tracks, keys })
}

/// Read one wire key, naming the field that is outside its shape.
fn parse_key(index: usize, key: &Value) -> Result<Key, McpToolError> {
    let Some(track) = key.get("track").and_then(Value::as_str) else {
        return Err(bad_timeline(format!(
            "`keys[{index}].track` must be a track name; recovery: name a track listed in `tracks`"
        )));
    };
    let Some(tick) = key.get("tick").and_then(Value::as_u64) else {
        return Err(bad_timeline(format!(
            "`keys[{index}].tick` must be an integer tick; recovery: count ticks from 0"
        )));
    };
    let Some(tick) = u32::try_from(tick).ok() else {
        return Err(bad_timeline(format!(
            "`keys[{index}].tick` {tick} is past the 32-bit tick range; recovery: keep ticks below \
             4294967296"
        )));
    };
    let Some(values) = key.get("value").and_then(Value::as_array) else {
        return Err(bad_timeline(format!(
            "`keys[{index}].value` must be three numbers; recovery: pass a three-value pose"
        )));
    };
    if values.len() != 3 {
        return Err(bad_timeline(format!(
            "`keys[{index}].value` holds {} numbers, not 3; recovery: pass a three-value pose",
            values.len()
        )));
    }
    let mut pose = [0.0f32; 3];
    for (axis, item) in values.iter().enumerate() {
        let Some(number) = item.as_f64() else {
            return Err(bad_timeline(format!(
                "`keys[{index}].value[{axis}]` is not a number; recovery: pass a three-value pose"
            )));
        };
        pose[axis] = f32_from_f64(number);
    }
    let easing = match key.get("easing") {
        None => Easing::Linear,
        Some(word) => {
            let word_text = word.as_str().unwrap_or_default();
            Easing::from_word(word_text).ok_or_else(|| {
                McpToolError::new(
                    "bad_easing",
                    format!(
                        "`keys[{index}].easing` is `{word_text}`; recovery: use one of linear, \
                         step, ease_in, ease_out, ease_in_out"
                    ),
                )
            })?
        }
    };
    Ok(Key {
        track: track.to_owned(),
        tick,
        value: pose,
        easing,
    })
}

/// Run `f` on this session's timeline under the audited dispatch seam.
/// The per-session timeline table: one compiled timeline per door session.
static TIMELINES: OnceLock<Mutex<BTreeMap<u64, Timeline>>> = OnceLock::new();

/// The table, or the poisoned-lock refusal naming the restart.
fn timelines() -> Result<std::sync::MutexGuard<'static, BTreeMap<u64, Timeline>>, McpToolError> {
    TIMELINES.get_or_init(Mutex::default).lock().map_err(|_| {
        McpToolError::new(
            "lock_poisoned",
            "anim timeline table is poisoned; restart the door",
        )
    })
}

/// Keep `timeline` as this session's playback clip under the audited dispatch seam.
fn store_timeline(server: &McpServer, tool: &str, timeline: Timeline) -> Result<(), McpToolError> {
    server.dispatch(tool, |state| {
        timelines()?.insert(state.session(), timeline);
        Ok(())
    })
}

/// Sample this session's timeline at `ticks` under the audited dispatch seam;
/// `no_timeline` when the session has none yet.
fn sample_timeline(
    server: &McpServer,
    tool: &str,
    ticks: &[u32],
) -> Result<Vec<TimelineSample>, McpToolError> {
    server.dispatch(tool, |state| {
        let table = timelines()?;
        let Some(timeline) = table.get(&state.session()) else {
            return Err(McpToolError::new(
                "no_timeline",
                "no timeline is set for this session; recovery: call anim.timeline.set first",
            ));
        };
        Ok(ticks
            .iter()
            .map(|tick| TimelineSample {
                tick: *tick,
                values: timeline.sample(*tick),
            })
            .collect())
    })
}

/// `anim.timeline.set`: validate a keyframe document, keep it as the session's
/// playback clip, and sample the ticks asked through the clip sampler.
fn timeline_set(
    server: &McpServer,
    args: &TimelineSetArgs,
) -> Result<TimelineSetOutput, McpToolError> {
    let doc = parse_timeline(&args.doc)?;
    let timeline = Timeline::compile(&doc).map_err(|error| {
        McpToolError::new(
            error.code(),
            format!("{}: {}", error.code(), error.message()),
        )
    })?;
    let ticks = match &args.sample_ticks {
        Some(ticks) if ticks.len() > MAX_TIMELINE_SAMPLES => {
            return Err(McpToolError::new(
                "too_many_samples",
                format!(
                    "{} sample ticks is past the cap {MAX_TIMELINE_SAMPLES}; recovery: ask for at \
                     most {MAX_TIMELINE_SAMPLES} ticks",
                    ticks.len()
                ),
            ));
        }
        Some(ticks) => ticks.clone(),
        None => vec![timeline.dur_ticks()],
    };
    let digest = format!("{:016x}", timeline.digest());
    let tracks = u32::try_from(timeline.tracks().len()).unwrap_or(u32::MAX);
    let keys = u32::try_from(timeline.keys()).unwrap_or(u32::MAX);
    let dur_ticks = timeline.dur_ticks();
    let samples: Vec<TimelineSample> = ticks
        .iter()
        .map(|tick| TimelineSample {
            tick: *tick,
            values: timeline.sample(*tick),
        })
        .collect();
    store_timeline(server, TOOL_ANIM_TIMELINE_SET, timeline)?;
    Ok(TimelineSetOutput {
        digest,
        tracks,
        keys,
        dur_ticks,
        samples,
    })
}

crate::e2_tool!(
    /// `anim.timeline.set`: validate a cutscene keyframe timeline and keep it.
    TimelineSetTool,
    name = TOOL_ANIM_TIMELINE_SET,
    description = "Validate a cutscene keyframe document (declared tracks plus keys carrying a tick, a three-value pose and an easing word), keep it as the session's playback clip, and sample the ticks asked through the same clip sampler the playback uses. Refuses a key on an undeclared track by name and a key tick behind its predecessor.",
    readonly = false,
    args = TimelineSetArgs,
    output = TimelineSetOutput,
    invoke = timeline_set
);

// ---------------------------------------------------------------------------
// anim.retarget: a third-party skeleton onto our 17 bone roles
// ---------------------------------------------------------------------------

/// The only profile this door scores: the 17 required roles.
const RETARGET_PROFILE: &str = "humanoid";
/// Most source bones one call may name: the contract's own joint cap.
const MAX_RETARGET_BONES: usize = MAX_JOINTS;
/// Confidence of a name read straight from a role, in per-mille.
const EXACT_CONFIDENCE: u16 = 1000;
/// Confidence of a family alias (`mixamorig_Hips`, `upperarm_l`), in per-mille.
const ALIAS_CONFIDENCE: u16 = 900;

/// One hand-written source bone to role pair.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct BoneOverrideDto {
    /// Source bone name, as the third-party rig authors it.
    pub source: String,
    /// Role name from the 17: `root`, `pelvis`, `spine_lower`, `spine_upper`,
    /// `head`, `upper_arm_l`, `upper_arm_r`, `lower_arm_l`, `lower_arm_r`,
    /// `hand_l`, `hand_r`, `upper_leg_l`, `upper_leg_r`, `lower_leg_l`,
    /// `lower_leg_r`, `foot_l`, `foot_r`.
    pub role: String,
}

/// Args for `anim.retarget`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct RetargetArgs {
    /// Profile to score against: only `humanoid`.
    pub profile: String,
    /// Source bone names of the rig, in any order; the answer sorts them. A
    /// name no family table knows refuses `unknown_bone`, so leave a bone out
    /// when it has no role of its own, such as `clavicle_l`.
    pub bones: Vec<String>,
    /// Hand-written roles for names the alias table does not know. A manual
    /// row wins over a guess for the same source, so one override per source;
    /// overriding the same source twice refuses `duplicate_bone`.
    #[serde(default)]
    pub overrides: Vec<BoneOverrideDto>,
}

/// One source bone with the role it landed on.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct BoneMapDto {
    /// Role name from the 17.
    pub role: String,
    /// Source bone name as authored.
    pub source: String,
    /// Match confidence in per-mille: 1000 for `exact` and `manual`, 900 for
    /// an alias.
    pub confidence_permille: u16,
    /// How the row was built: `exact` (the name is the role), `alias` (a
    /// Mixamo or Unreal family name) or `manual` (an override).
    pub method: String,
}

/// Result of `anim.retarget`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct RetargetOutput {
    /// Profile the map was scored against.
    pub profile: String,
    /// One row per source bone, sorted by role then source.
    pub mappings: Vec<BoneMapDto>,
    /// Coverage of the 17 required roles in per-mille. A success is always
    /// 1000: a partial map refuses `missing_roles` and names the roles.
    pub coverage_permille: u16,
    /// Map digest (16 hex digits) over the sorted rows: equal on every replay
    /// of the same names.
    pub digest: String,
}

/// The role whose name is `name`, for an override.
fn role_by_name(name: &str) -> Option<BoneRole> {
    REQUIRED_BONE_ROLES
        .iter()
        .copied()
        .find(|role| role.name() == name)
}

/// The 17 role names, for a refusal that has to list them.
fn role_names() -> String {
    REQUIRED_BONE_ROLES
        .iter()
        .map(|role| role.name())
        .collect::<Vec<&'static str>>()
        .join(", ")
}

/// The alias-table row for one source name, refusing a bone no table knows.
fn alias_row(source: &str, profile: &str) -> Result<BoneMapping, McpToolError> {
    let role = bone_role_for_name(source).map_err(|_| {
        McpToolError::new(
            "unknown_bone",
            format!(
                "`{source}` maps to no role in profile `{profile}`; recovery: leave the bone out \
                 (a bone with no role of its own, such as `clavicle_l`, is not mapped) or submit \
                 an override for it. Roles: {}",
                role_names()
            ),
        )
    })?;
    let exact = role.name() == source;
    Ok(BoneMapping {
        source: source.to_owned(),
        role,
        confidence_permille: if exact {
            EXACT_CONFIDENCE
        } else {
            ALIAS_CONFIDENCE
        },
        method: if exact { "exact" } else { "alias" },
    })
}

/// A hand-written row: the caller named the role, so it scores full.
fn manual_row(source: &str, role: BoneRole) -> BoneMapping {
    BoneMapping {
        source: source.to_owned(),
        role,
        confidence_permille: EXACT_CONFIDENCE,
        method: "manual",
    }
}

/// Read the alias table over the named bones, then apply the hand-written
/// overrides, so a manual row always wins over a guess. An override whose
/// source is not in the bone list still adds its row.
fn mapping_rows(args: &RetargetArgs) -> Result<Vec<BoneMapping>, McpToolError> {
    let mut manual: Vec<(&str, BoneRole)> = Vec::with_capacity(args.overrides.len());
    for over in &args.overrides {
        let role = role_by_name(&over.role).ok_or_else(|| {
            McpToolError::new(
                "bad_override",
                format!(
                    "`{}` is not a role; recovery: name one of the 17 for source `{}`. Roles: {}",
                    over.role,
                    over.source,
                    role_names()
                ),
            )
        })?;
        if manual.iter().any(|(name, _)| *name == over.source.as_str()) {
            return Err(McpToolError::new(
                "duplicate_bone",
                format!(
                    "source `{}` is overridden twice; recovery: keep one override per source",
                    over.source
                ),
            ));
        }
        manual.push((over.source.as_str(), role));
    }
    let mut rows: Vec<BoneMapping> = Vec::with_capacity(args.bones.len() + manual.len());
    for source in &args.bones {
        match manual.iter().find(|(name, _)| *name == source.as_str()) {
            Some((_, role)) => rows.push(manual_row(source, *role)),
            None => rows.push(alias_row(source, &args.profile)?),
        }
    }
    for (source, role) in &manual {
        if !args.bones.iter().any(|name| name == source) {
            rows.push(manual_row(source, *role));
        }
    }
    Ok(rows)
}

/// Turn a scoring refusal into the door's own code, naming the bone.
fn coverage_refusal(error: &AnimError, rows: &[BoneMapping]) -> McpToolError {
    match error {
        AnimError::UnknownRole(index) => {
            let source = rows
                .get(*index)
                .map_or("?".to_owned(), |row| row.source.clone());
            McpToolError::new(
                "unknown_bone",
                format!("`{source}` maps to no role; recovery: leave it out or override it"),
            )
        }
        AnimError::DuplicateBone(index) => {
            let source = rows
                .get(*index)
                .map_or("?".to_owned(), |row| row.source.clone());
            McpToolError::new(
                "duplicate_bone",
                format!(
                    "source `{source}` is submitted twice; recovery: keep one row for it, or \
                     rename the override"
                ),
            )
        }
        AnimError::MissingRoles(roles) => McpToolError::new(
            "missing_roles",
            format!(
                "the map leaves {} roles empty: {}; recovery: add an override for each",
                roles.len(),
                roles
                    .iter()
                    .map(|role| role.name())
                    .collect::<Vec<&'static str>>()
                    .join(", ")
            ),
        ),
        other => McpToolError::new(
            "internal",
            format!("the role map failed with {other:?}; report this retarget"),
        ),
    }
}

fn retarget(_server: &McpServer, args: &RetargetArgs) -> Result<RetargetOutput, McpToolError> {
    if args.profile != RETARGET_PROFILE {
        return Err(McpToolError::new(
            "bad_profile",
            format!(
                "`{}` is not a profile; recovery: run with profile `{RETARGET_PROFILE}",
                args.profile
            ),
        ));
    }
    if args.bones.is_empty() && args.overrides.is_empty() {
        return Err(McpToolError::new(
            "bad_args",
            "anim.retarget needs work: submit bones, or overrides that name a role".to_owned(),
        ));
    }
    if args.bones.len() > MAX_RETARGET_BONES {
        return Err(McpToolError::new(
            "too_many_inputs",
            format!(
                "{} bones is past the cap {MAX_RETARGET_BONES}; recovery: retarget a rig with at \
                 most {MAX_RETARGET_BONES} named bones",
                args.bones.len()
            ),
        ));
    }
    let mut rows = mapping_rows(args)?;
    // Canonical order, so the same names always give the same digest.
    rows.sort_by(|left, right| {
        left.role
            .index()
            .cmp(&right.role.index())
            .then_with(|| left.source.cmp(&right.source))
    });
    let coverage_permille =
        bone_coverage(&rows).map_err(|error| coverage_refusal(&error, &rows))?;
    let mut text = String::new();
    for row in &rows {
        text.push_str(row.role.name());
        text.push('\0');
        text.push_str(&row.source);
        text.push('\0');
    }
    let digest = format!("{:016x}", engine_core::hash_bytes(text.as_bytes()));
    Ok(RetargetOutput {
        profile: args.profile.clone(),
        mappings: rows
            .iter()
            .map(|row| BoneMapDto {
                role: row.role.name().to_owned(),
                source: row.source.clone(),
                confidence_permille: row.confidence_permille,
                method: row.method.to_owned(),
            })
            .collect(),
        coverage_permille,
        digest,
    })
}

crate::e2_tool!(
    /// `anim.retarget`: map a third-party skeleton onto the 17 bone roles.
    RetargetTool,
    name = TOOL_ANIM_RETARGET,
    description = "Retarget a third-party rig: name the profile and the source bone names, and the call answers one row per bone (role, confidence in per-mille, method exact/alias/manual) over the 17 roles plus a map digest. Bones named by the Mixamo and Unreal families resolve without help; an unknown bone, a repeated source or an empty role list refuses and names what to fix. Nothing is written.",
    readonly = true,
    args = RetargetArgs,
    output = RetargetOutput,
    invoke = retarget
);

/// The anim pack: state machine, clip, timeline and retarget verbs, loaded on
/// demand. Every verb in this file, each with its capability card, in the
/// order `anim.<verb>` names them.
pub fn pack() -> Result<PackDef, PackError> {
    let mut pack = PackDef::new(
        PACK,
        "animation: the state machine as versioned data (set, undo, get, preview on a tape), \
         clip edits (trim, retime, mirror, undo) over the session clip, a cutscene keyframe \
         timeline that compiles onto the same clip sampler, a read-only rotation sidecar \
         preview per joint, a bone retarget onto the 17 roles, and that map drawn
         as one PNG frame",
        false,
    );
    pack.add(VerbSpec::of::<GraphSetTool>(
        TOOL_ANIM_GRAPH_SET,
        PACK,
        CapabilityCard::Commit,
    )?);
    pack.add(VerbSpec::of::<GraphUndoTool>(
        TOOL_ANIM_GRAPH_UNDO,
        PACK,
        CapabilityCard::Revert,
    )?);
    pack.add(VerbSpec::of::<GraphGetTool>(
        TOOL_ANIM_GRAPH_GET,
        PACK,
        CapabilityCard::Observe,
    )?);
    pack.add(VerbSpec::of::<PreviewTool>(
        TOOL_ANIM_PREVIEW,
        PACK,
        CapabilityCard::Observe,
    )?);
    pack.add(VerbSpec::of::<ClipTrimTool>(
        TOOL_ANIM_CLIP_TRIM,
        PACK,
        CapabilityCard::Commit,
    )?);
    pack.add(VerbSpec::of::<ClipRetimeTool>(
        TOOL_ANIM_CLIP_RETIME,
        PACK,
        CapabilityCard::Commit,
    )?);
    pack.add(VerbSpec::of::<ClipMirrorTool>(
        TOOL_ANIM_CLIP_MIRROR,
        PACK,
        CapabilityCard::Commit,
    )?);
    pack.add(VerbSpec::of::<ClipUndoTool>(
        TOOL_ANIM_CLIP_UNDO,
        PACK,
        CapabilityCard::Revert,
    )?);
    pack.add(VerbSpec::of::<RotPreviewTool>(
        TOOL_ANIM_ROT_PREVIEW,
        PACK,
        CapabilityCard::Observe,
    )?);
    pack.add(VerbSpec::of::<TimelineSetTool>(
        TOOL_ANIM_TIMELINE_SET,
        PACK,
        CapabilityCard::Commit,
    )?);
    pack.add(VerbSpec::of::<RetargetTool>(
        TOOL_ANIM_RETARGET,
        PACK,
        CapabilityCard::Observe,
    )?);
    pack.add(VerbSpec::of::<preview::RetargetPreviewTool>(
        preview::TOOL_ANIM_RETARGET_PREVIEW,
        PACK,
        CapabilityCard::Observe,
    )?);
    Ok(pack)
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_editor::capability::Capability;
    use rmcp::handler::server::router::tool::SyncTool;
    use rmcp::serde_json::json;

    fn server(session: u64) -> McpServer {
        McpServer::new(session, Capability::unrestricted(session))
    }

    fn machine(fade: u64) -> Value {
        json!({
            "version": 1, "name": "fighter", "start": "idle",
            "params": [{"name": "speed", "default": 0, "min": 0, "max": 10}],
            "states": [
                {"name": "idle", "clip": 0, "length_ticks": 30, "looping": true},
                {"name": "walk", "clip": 1, "length_ticks": 24, "looping": true}
            ],
            "transitions": [
                {"from": "idle", "to": "walk", "when": {"param": "speed", "op": ">=", "value": 2}, "fade_ticks": fade},
                {"from": "walk", "to": "idle", "when": {"param": "speed", "op": "<", "value": 2}, "fade_ticks": fade}
            ]
        })
    }

    #[test]
    fn pack_registers_twelve_annotated_verbs() {
        let pack = pack().expect("registers");
        assert_eq!(
            pack.verb_names(),
            vec![
                TOOL_ANIM_GRAPH_SET,
                TOOL_ANIM_GRAPH_UNDO,
                TOOL_ANIM_GRAPH_GET,
                TOOL_ANIM_PREVIEW,
                TOOL_ANIM_CLIP_TRIM,
                TOOL_ANIM_CLIP_RETIME,
                TOOL_ANIM_CLIP_MIRROR,
                TOOL_ANIM_CLIP_UNDO,
                TOOL_ANIM_ROT_PREVIEW,
                TOOL_ANIM_TIMELINE_SET,
                TOOL_ANIM_RETARGET,
                preview::TOOL_ANIM_RETARGET_PREVIEW
            ]
        );
        for verb in pack.verbs() {
            assert_ne!(verb.card(), CapabilityCard::Deny);
            assert!(
                verb.model().annotations.is_some(),
                "{} is annotated",
                verb.name()
            );
        }
        assert!(super::super::PackRegistry::shipped().errors().is_empty());
        assert!(crate::mcp::tool_names().contains(&TOOL_ANIM_PREVIEW));
    }

    #[test]
    fn set_undo_preview_through_the_door() {
        let server = server(41);
        let first = GraphSetTool::invoke(&server, GraphSetArgs { graph: machine(6) }).expect("set");
        assert_eq!(
            (first.before.clone(), first.states, first.transitions),
            (None, 2, 2)
        );
        let second =
            GraphSetTool::invoke(&server, GraphSetArgs { graph: machine(9) }).expect("set");
        assert_eq!(second.before, Some(first.after.clone()));
        let refused = GraphSetTool::invoke(
            &server,
            GraphSetArgs {
                graph: json!({"version": 1, "name": "x", "start": "a", "mood": 1}),
            },
        )
        .expect_err("unknown field");
        assert_eq!(refused.code, "bad_document");
        assert!(refused.message.contains("$.mood"), "{}", refused.message);
        let kept = GraphGetTool::invoke(&server, NoArgs {}).expect("get");
        let kept = kept.graph.expect("a graph is set");
        assert_eq!(kept["transitions"][0]["fade_ticks"], 9, "last good kept");
        let tape = json!({"ticks": 60, "params": [{"at": 2, "name": "speed", "value": 5}]});
        let a = PreviewTool::invoke(
            &server,
            PreviewArgs {
                graph: None,
                script: tape.clone(),
                timeline_ticks: None,
            },
        )
        .expect("preview");
        let b = PreviewTool::invoke(
            &server,
            PreviewArgs {
                graph: None,
                script: tape,
                timeline_ticks: None,
            },
        )
        .expect("preview");
        assert_eq!(a, b);
        assert_eq!(
            a.events.first().map(|e| (e.to.as_str(), e.fade_ticks)),
            Some(("walk", 9))
        );
        let back = GraphUndoTool::invoke(&server, NoArgs {}).expect("undo");
        assert_eq!(back.after, first.after, "undo restores the first digest");
        assert_eq!(
            GraphUndoTool::invoke(&server, NoArgs {})
                .expect_err("nothing left")
                .code,
            "nothing_to_undo"
        );
    }

    #[test]
    fn rot_preview_is_repeatable_and_names_a_missing_clip() {
        let server = server(61);
        let path =
            std::env::temp_dir().join(format!("e2040_rot_preview_{}.json", std::process::id()));
        std::fs::write(
            &path,
            r#"{"tracks":[{"times":[0.0,0.5,1.0],"keys":[[0,0,0]]},{"times":[0.0,1.0]}],"dur":1.0}"#,
        )
        .expect("clip writes");
        let args = RotPreviewArgs {
            clip_path: path.display().to_string(),
            joint: 1,
            ticks: Some(60),
        };
        let first = RotPreviewTool::invoke(&server, args.clone()).expect("first preview");
        let second = RotPreviewTool::invoke(&server, args).expect("second preview");
        assert_eq!(first, second, "same input, same samples");
        assert_eq!(first.digest, second.digest, "same input, same digest");
        assert_eq!(first.times.len(), 8);
        assert_eq!(first.rot.len(), 8);
        assert_eq!(first.socket_offset.len(), 8);

        let missing = RotPreviewTool::invoke(
            &server,
            RotPreviewArgs {
                clip_path: path.with_extension("absent.json").display().to_string(),
                joint: 0,
                ticks: None,
            },
        )
        .expect_err("missing file");
        assert_eq!(missing.code, "missing_clip");
        assert!(missing.message.contains("tracks"), "{}", missing.message);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn sessions_do_not_share_graphs_and_preview_needs_a_graph() {
        let (a, b) = (server(51), server(52));
        GraphSetTool::invoke(&a, GraphSetArgs { graph: machine(3) }).expect("set");
        assert_eq!(
            GraphGetTool::invoke(&b, NoArgs {}).expect("get").graph,
            None
        );
        let script = json!({"ticks": 5});
        let refused = PreviewTool::invoke(
            &b,
            PreviewArgs {
                graph: None,
                script: script.clone(),
                timeline_ticks: None,
            },
        )
        .expect_err("no graph");
        assert_eq!(refused.code, "no_graph");
        let candidate = PreviewTool::invoke(
            &b,
            PreviewArgs {
                graph: Some(machine(3)),
                script,
                timeline_ticks: None,
            },
        )
        .expect("a candidate runs without a set graph");
        assert_eq!(candidate.final_state, "idle");
    }

    fn giant_states(count: usize) -> Value {
        let mut states = Vec::with_capacity(count);
        for i in 0..count {
            states.push(json!({
                "name": format!("s{i}"),
                "clip": 0,
                "length_ticks": 30,
                "looping": true
            }));
        }
        json!({
            "version": 1, "name": "fighter", "start": "s0",
            "params": [{"name": "speed", "default": 0, "min": 0, "max": 10}],
            "states": states,
            "transitions": [
                {"from": "s0", "to": "s1",
                    "when": {"param": "speed", "op": ">=", "value": 2}, "fade_ticks": 5},
                {"from": "s1", "to": "s0",
                    "when": {"param": "speed", "op": "<", "value": 2}, "fade_ticks": 5}
            ]
        })
    }

    fn fade_sum_graph(count: usize, fade_each: u64) -> Value {
        let mut transitions = Vec::with_capacity(count);
        for i in 0..count {
            let (from, to) = if i % 2 == 0 {
                ("idle", "walk")
            } else {
                ("walk", "idle")
            };
            transitions.push(json!({
                "from": from, "to": to,
                "when": {"param": "speed", "op": ">=", "value": 2},
                "fade_ticks": fade_each
            }));
        }
        json!({
            "version": 1, "name": "fighter", "start": "idle",
            "params": [{"name": "speed", "default": 0, "min": 0, "max": 10}],
            "states": [
                {"name": "idle", "clip": 0, "length_ticks": 30, "looping": true},
                {"name": "walk", "clip": 1, "length_ticks": 24, "looping": true}
            ],
            "transitions": transitions
        })
    }

    fn over_bytes_graph(pad_len: usize) -> Value {
        let mut graph = machine(5);
        if let Some(obj) = graph.as_object_mut() {
            obj.insert("pad".to_string(), Value::String("x".repeat(pad_len)));
        }
        graph
    }

    fn hostile_graphs() -> Vec<Value> {
        vec![
            giant_states(40),
            giant_states(50),
            giant_states(70),
            giant_states(90),
            giant_states(110),
            giant_states(150),
            giant_states(200),
            fade_sum_graph(9, 600),
            fade_sum_graph(10, 600),
            fade_sum_graph(12, 500),
            fade_sum_graph(15, 400),
            fade_sum_graph(20, 300),
            fade_sum_graph(12, 600),
            fade_sum_graph(16, 400),
            over_bytes_graph(20_000),
            over_bytes_graph(22_000),
            over_bytes_graph(25_000),
            over_bytes_graph(30_000),
            over_bytes_graph(40_000),
            over_bytes_graph(50_000),
        ]
    }

    #[test]
    fn hostile_graph_sets_refuse_pre_apply_and_keep_last_good() {
        let hostiles = hostile_graphs();
        assert_eq!(hostiles.len(), 20);
        let legals = vec![machine(3), machine(6), machine(9)];

        let mut pure_refused = 0;
        for graph in &hostiles {
            let first = graph_cost(graph);
            let second = graph_cost(graph);
            assert_eq!(first, second, "same input seals the same cost");
            let verdict = check_graph_cost(&first);
            let replay = check_graph_cost(&second);
            assert_eq!(
                verdict.is_ok(),
                replay.is_ok(),
                "repeated runs give identical results"
            );
            let refused = verdict.expect_err("hostile graph refuses pre-apply");
            assert_eq!(refused.code, "graph_too_large");
            assert!(
                refused.message.contains("caps states<="),
                "{}",
                refused.message
            );
            assert!(refused.message.contains("recovery:"), "{}", refused.message);
            assert!(
                crate::tool_capabilities::message_carries_recovery(&refused.message),
                "{}",
                refused.message
            );
            pure_refused += 1;
        }
        assert!(
            pure_refused >= 16,
            "at least 16 of 20 hostile graphs refuse pre-apply, got {pure_refused}"
        );

        for graph in &legals {
            let cost = graph_cost(graph);
            check_graph_cost(&cost).expect("legal small graph passes the gate");
            let replay = graph_cost(graph);
            assert_eq!(cost, replay, "legal cost replays exactly");
        }

        let door = server(61);
        let first = GraphSetTool::invoke(
            &door,
            GraphSetArgs {
                graph: legals[0].clone(),
            },
        )
        .expect("legal set lands");
        let baseline = GraphGetTool::invoke(&door, NoArgs {})
            .expect("get")
            .graph
            .expect("a graph is set");
        assert_eq!(first.states, 2);

        let mut wired_refused = 0;
        for graph in &hostiles {
            let refused = GraphSetTool::invoke(
                &door,
                GraphSetArgs {
                    graph: graph.clone(),
                },
            )
            .expect_err("hostile set refuses before the door mutates");
            assert_eq!(
                refused.code, "graph_too_large",
                "pre-apply, not the validator"
            );
            wired_refused += 1;
            let kept = GraphGetTool::invoke(&door, NoArgs {})
                .expect("get")
                .graph
                .expect("a graph is set");
            assert_eq!(kept, baseline, "a refused set keeps the last good graph");
        }
        assert!(
            wired_refused >= 16,
            "at least 16 of 20 hostile sets refuse at the door, got {wired_refused}"
        );

        for graph in &legals {
            GraphSetTool::invoke(
                &door,
                GraphSetArgs {
                    graph: graph.clone(),
                },
            )
            .expect("legal small graph passes at the door");
        }
        let kept = GraphGetTool::invoke(&door, NoArgs {})
            .expect("get")
            .graph
            .expect("a graph is set");
        assert!(
            kept["states"].as_array().map_or(0, Vec::len) <= 4,
            "legal graphs stay small"
        );
    }
}

#[cfg(test)]
mod clip_tests {
    use super::preview::{preview_frame, RetargetPreviewArgs, PREVIEW_MAX_SIZE};
    use super::*;
    use engine_editor::capability::Capability;
    use rmcp::handler::server::router::tool::SyncTool;

    fn server() -> McpServer {
        McpServer::new(77, Capability::unrestricted(77))
    }

    fn trimmed() -> QClip {
        edit::trim(&clip_fixture(), 10, 40).expect("library trim")
    }

    #[test]
    fn anim_clip_edits_match_the_library_and_undo() {
        let server = server();
        let fixture = clip_fixture();

        let trim = ClipTrimTool::invoke(
            &server,
            ClipTrimArgs {
                start_tick: 10,
                end_tick: 40,
            },
        )
        .expect("trim runs");
        assert_eq!(trim.before, clip_digest(&fixture));
        assert_eq!(trim.after, clip_digest(&trimmed()));
        assert_eq!(trim.rev, 1);

        for refused in [
            ClipRetimeTool::invoke(&server, ClipRetimeArgs { num: 0, den: 1 }).map(|out| out.after),
            ClipTrimTool::invoke(
                &server,
                ClipTrimArgs {
                    start_tick: 30,
                    end_tick: 30,
                },
            )
            .map(|out| out.after),
        ] {
            let error = refused.expect_err("the library refuses this edit");
            assert_eq!(error.code, "bad_clip");
        }

        let retime = ClipRetimeTool::invoke(&server, ClipRetimeArgs { num: 2, den: 1 })
            .expect("retime runs");
        let expected = edit::retime(&trimmed(), 2, 1).expect("library retime");
        assert_eq!(retime.after, clip_digest(&expected));

        let mirror = ClipMirrorTool::invoke(&server, NoArgs {}).expect("mirror runs");
        let expected = edit::mirror(&expected);
        assert_eq!(mirror.after, clip_digest(&expected));
        assert_eq!(mirror.undo_depth, 3);

        // Undo walks back through mirror, retime and trim, then refuses.
        let back = ClipUndoTool::invoke(&server, NoArgs {}).expect("undo mirror");
        assert_eq!(
            back.after,
            clip_digest(&edit::retime(&trimmed(), 2, 1).unwrap())
        );
        let back = ClipUndoTool::invoke(&server, NoArgs {}).expect("undo retime");
        assert_eq!(back.after, clip_digest(&trimmed()));
        let back = ClipUndoTool::invoke(&server, NoArgs {}).expect("undo trim");
        assert_eq!(back.after, clip_digest(&fixture));
        let empty = ClipUndoTool::invoke(&server, NoArgs {}).expect_err("nothing left");
        assert_eq!(empty.code, "nothing_to_undo");
    }

    fn two_track_doc() -> Value {
        use rmcp::serde_json::json;
        json!({
            "tracks": ["hips", "head"],
            "keys": [
                {"track": "hips", "tick": 0, "value": [0.0, 1.0, 0.0]},
                {"track": "hips", "tick": 30, "value": [0.0, 2.0, 0.0]},
                {"track": "head", "tick": 15, "value": [10.0, 0.0, 0.0]},
                {"track": "head", "tick": 45, "value": [20.0, 0.0, 0.0]}
            ]
        })
    }

    /// F2P: a two-track timeline set through the door samples exactly as the
    /// hand-built clip does at the same ticks.
    #[test]
    fn timeline_set_plays_through_the_clip_sampler() {
        use engine_anim::clip::{sample_all, Cursor};
        use rmcp::serde_json::json;
        let server = server();
        let ticks = vec![0u32, 7, 15, 22, 30, 44];
        let out = TimelineSetTool::invoke(
            &server,
            TimelineSetArgs {
                doc: two_track_doc(),
                sample_ticks: Some(ticks.clone()),
            },
        )
        .expect("timeline sets");
        assert_eq!((out.tracks, out.keys, out.dur_ticks), (2, 4, 45));
        assert_eq!(out.samples.len(), ticks.len());
        let t = |tick: u64| f32_from_u64(tick) * TICK_DT;
        let hand = bake_clip(vec![
            bake_track(vec![t(0), t(30)], vec![[0.0, 1.0, 0.0], [0.0, 2.0, 0.0]]),
            bake_track(vec![t(15), t(45)], vec![[10.0, 0.0, 0.0], [20.0, 0.0, 0.0]]),
        ]);
        assert_eq!(out.digest, format!("{:016x}", qclip_hash(&hand)));
        let mut cursor = Cursor::new(2);
        let mut want = [[0.0f32; 3]; 2];
        for sample in &out.samples {
            sample_all(
                &hand,
                f32_from_u64(u64::from(sample.tick)) * TICK_DT,
                &mut cursor,
                &mut want,
            );
            assert_eq!(sample.values.len(), 2);
            for track in 0..2 {
                for axis in 0..3 {
                    assert!(
                        (sample.values[track][axis] - want[track][axis]).abs() <= 1e-5,
                        "tick {} track {track} axis {axis}: {} vs {}",
                        sample.tick,
                        sample.values[track][axis],
                        want[track][axis]
                    );
                }
            }
        }
        // The same timeline plays through `anim.preview`: the door samples it there too.
        let graph = json!({
            "version": 1, "name": "timeline", "start": "idle", "params": [],
            "states": [{"name": "idle", "clip": 0, "length_ticks": 30, "looping": true}],
            "transitions": []
        });
        let preview = PreviewTool::invoke(
            &server,
            PreviewArgs {
                graph: Some(graph),
                script: json!({"ticks": 30}),
                timeline_ticks: Some(ticks.clone()),
            },
        )
        .expect("preview samples the timeline");
        assert_eq!(preview.timeline_samples.len(), ticks.len());
        for (played, sampled) in preview.timeline_samples.iter().zip(out.samples.iter()) {
            assert_eq!(played, sampled, "preview and set sample the same pose");
        }
    }

    #[test]
    fn timeline_set_refuses_an_undeclared_track_by_name() {
        use rmcp::serde_json::json;
        let server = server();
        let refused = TimelineSetTool::invoke(
            &server,
            TimelineSetArgs {
                doc: json!({
                    "tracks": ["hips"],
                    "keys": [{"track": "tail", "tick": 0, "value": [0.0, 0.0, 0.0]}]
                }),
                sample_ticks: None,
            },
        )
        .expect_err("undeclared track refuses");
        assert_eq!(refused.code, "unknown_track");
        assert!(refused.message.contains("tail"), "{}", refused.message);
        let backwards = TimelineSetTool::invoke(
            &server,
            TimelineSetArgs {
                doc: json!({
                    "tracks": ["hips", "head"],
                    "keys": [
                        {"track": "hips", "tick": 40, "value": [0.0, 0.0, 0.0]},
                        {"track": "hips", "tick": 30, "value": [0.0, 0.0, 0.0]},
                        {"track": "head", "tick": 0, "value": [0.0, 0.0, 0.0]}
                    ]
                }),
                sample_ticks: None,
            },
        )
        .expect_err("backwards key refuses");
        assert_eq!(backwards.code, "non_monotonic");
        assert!(backwards.message.contains("hips"), "{}", backwards.message);
        let easing = TimelineSetTool::invoke(
            &server,
            TimelineSetArgs {
                doc: json!({
                    "tracks": ["hips"],
                    "keys": [{"track": "hips", "tick": 0, "value": [0.0, 0.0, 0.0],
                               "easing": "bounce"}]
                }),
                sample_ticks: None,
            },
        )
        .expect_err("unknown easing refuses");
        assert_eq!(easing.code, "bad_easing");
        assert!(easing.message.contains("bounce"), "{}", easing.message);
        // A session with no timeline refuses the preview sampling by name.
        let fresh = McpServer::new(78, Capability::unrestricted(78));
        let refused = PreviewTool::invoke(
            &fresh,
            PreviewArgs {
                graph: Some(json!({
                    "version": 1, "name": "timeline", "start": "idle", "params": [],
                    "states": [{"name": "idle", "clip": 0, "length_ticks": 30, "looping": true}],
                    "transitions": []
                })),
                script: json!({"ticks": 1}),
                timeline_ticks: Some(vec![0]),
            },
        )
        .expect_err("no timeline refuses");
        assert_eq!(refused.code, "no_timeline");
    }

    /// The 17 bones of an Unreal-mannequin rig, as that family names them.
    fn unreal_bones() -> Vec<String> {
        [
            "root",
            "pelvis",
            "spine_01",
            "spine_03",
            "head",
            "upperarm_l",
            "upperarm_r",
            "lowerarm_l",
            "lowerarm_r",
            "hand_l",
            "hand_r",
            "thigh_l",
            "thigh_r",
            "calf_l",
            "calf_r",
            "foot_l",
            "foot_r",
        ]
        .iter()
        .map(|name| (*name).to_owned())
        .collect()
    }

    /// The same body from a Mixamo export.
    fn mixamo_bones() -> Vec<String> {
        [
            "Root",
            "mixamorig_Hips",
            "mixamorig_Spine",
            "mixamorig_Spine1",
            "mixamorig_Head",
            "mixamorig_LeftArm",
            "mixamorig_RightArm",
            "mixamorig_LeftForeArm",
            "mixamorig_RightForeArm",
            "mixamorig_LeftHand",
            "mixamorig_RightHand",
            "mixamorig_LeftUpLeg",
            "mixamorig_RightUpLeg",
            "mixamorig_LeftLeg",
            "mixamorig_RightLeg",
            "mixamorig_LeftFoot",
            "mixamorig_RightFoot",
        ]
        .iter()
        .map(|name| (*name).to_owned())
        .collect()
    }

    fn retarget_args(bones: Vec<String>) -> RetargetArgs {
        RetargetArgs {
            profile: "humanoid".to_owned(),
            bones,
            overrides: Vec::new(),
        }
    }

    fn map_ok(args: RetargetArgs) -> RetargetOutput {
        RetargetTool::invoke(&server(), args).expect("the bone map runs")
    }

    fn map_err(args: RetargetArgs) -> McpToolError {
        RetargetTool::invoke(&server(), args).expect_err("the bone map refuses")
    }

    #[test]
    fn an_unreal_named_rig_retargets_to_full_coverage() {
        let out = map_ok(retarget_args(unreal_bones()));
        assert_eq!(out.coverage_permille, 1000, "coverage is per-mille");
        assert_eq!(out.mappings.len(), 17);
        // Sorted by role, so the first row is always the root.
        assert_eq!(out.mappings[0].role, "root");
        assert_eq!(out.mappings[5].role, "upper_arm_l");
        assert_eq!(out.mappings[13].role, "lower_leg_l");
        // A family alias scores 900; a name that is the role scores 1000.
        let thigh = out
            .mappings
            .iter()
            .find(|row| row.source == "thigh_l")
            .expect("thigh_l mapped");
        assert_eq!(
            (thigh.role.as_str(), thigh.method.as_str()),
            ("upper_leg_l", "alias")
        );
        assert_eq!(thigh.confidence_permille, 900);
        let pelvis = out
            .mappings
            .iter()
            .find(|row| row.source == "pelvis")
            .expect("pelvis mapped");
        assert_eq!(
            (pelvis.role.as_str(), pelvis.method.as_str()),
            ("pelvis", "exact")
        );
        assert_eq!(pelvis.confidence_permille, 1000);
        let replay = map_ok(retarget_args(unreal_bones()));
        assert_eq!(out.digest, replay.digest, "the map moved between runs");
        assert_eq!(out.digest.len(), 16, "16 hex digits");
    }

    #[test]
    fn a_mixamo_rig_lands_the_same_roles() {
        let unreal = map_ok(retarget_args(unreal_bones()));
        let mixamo = map_ok(retarget_args(mixamo_bones()));
        let roles = |out: &RetargetOutput| -> Vec<String> {
            out.mappings
                .iter()
                .map(|row| row.role.clone())
                .collect::<Vec<String>>()
        };
        assert_eq!(roles(&unreal), roles(&mixamo), "two families, one body");
        assert_eq!(mixamo.coverage_permille, 1000);
        assert_ne!(unreal.digest, mixamo.digest, "sources reach the digest");
    }

    #[test]
    fn a_manual_override_names_a_bone_no_table_knows() {
        let mut bones = unreal_bones();
        bones.retain(|name| name != "pelvis");
        bones.push("hips_custom".to_owned());
        let out = map_ok(RetargetArgs {
            overrides: vec![BoneOverrideDto {
                source: "hips_custom".to_owned(),
                role: "pelvis".to_owned(),
            }],
            ..retarget_args(bones)
        });
        assert_eq!(out.coverage_permille, 1000);
        let row = out
            .mappings
            .iter()
            .find(|row| row.source == "hips_custom")
            .expect("the custom bone mapped");
        assert_eq!(
            (row.role.as_str(), row.method.as_str()),
            ("pelvis", "manual")
        );
        assert_eq!(row.confidence_permille, 1000);
        // An override for a source the bone list does not name still maps.
        let extra = map_ok(RetargetArgs {
            overrides: vec![BoneOverrideDto {
                source: "hips_custom".to_owned(),
                role: "pelvis".to_owned(),
            }],
            ..retarget_args(unreal_bones())
        });
        assert_eq!(extra.coverage_permille, 1000, "one role, two sources");
        assert_eq!(extra.mappings.len(), 18, "both pelvis rows stay");
    }

    #[test]
    fn refusals_name_the_bone_the_role_or_the_empty_roles() {
        let mut unknown = unreal_bones();
        unknown.push("bone_007".to_owned());
        let refused = map_err(retarget_args(unknown));
        assert_eq!(refused.code, "unknown_bone");
        assert!(refused.message.contains("bone_007"), "{}", refused.message);
        assert!(
            refused.message.contains("foot_r"),
            "the 17 roles are listed"
        );

        let mut half = unreal_bones();
        half.retain(|name| name != "calf_l");
        let refused = map_err(retarget_args(half));
        assert_eq!(refused.code, "missing_roles");
        assert!(
            refused.message.contains("lower_leg_l"),
            "{}",
            refused.message
        );

        let refused = map_err(RetargetArgs {
            profile: "quadruped".to_owned(),
            ..retarget_args(unreal_bones())
        });
        assert_eq!(refused.code, "bad_profile");

        let refused = map_err(RetargetArgs {
            overrides: vec![BoneOverrideDto {
                source: "tail".to_owned(),
                role: "wing".to_owned(),
            }],
            ..retarget_args(unreal_bones())
        });
        assert_eq!(refused.code, "bad_override");
        assert!(refused.message.contains("tail"), "{}", refused.message);

        let refused = map_err(RetargetArgs {
            overrides: vec![
                BoneOverrideDto {
                    source: "hips_custom".to_owned(),
                    role: "pelvis".to_owned(),
                },
                BoneOverrideDto {
                    source: "hips_custom".to_owned(),
                    role: "head".to_owned(),
                },
            ],
            ..retarget_args(unreal_bones())
        });
        assert_eq!(refused.code, "duplicate_bone");
        assert!(
            refused.message.contains("hips_custom"),
            "{}",
            refused.message
        );

        let refused = map_err(retarget_args(Vec::new()));
        assert_eq!(refused.code, "bad_args");
    }

    /// A fresh directory under the system temp dir, one per call site.
    fn scratch(tag: &str) -> std::path::PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("e2040-retarget-{tag}-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("scratch dir");
        dir
    }

    fn preview_args(map: RetargetArgs, out: &str) -> RetargetPreviewArgs {
        RetargetPreviewArgs {
            map,
            dir: "frames".to_owned(),
            out: out.to_owned(),
            width: Some(128),
            height: Some(128),
        }
    }

    #[test]
    fn the_preview_is_a_real_frame_with_pixels() {
        let root = scratch("frame");
        let out = preview_frame(
            &root,
            &server(),
            &preview_args(retarget_args(unreal_bones()), "rig.png"),
        )
        .expect("the frame writes");
        assert_eq!((out.width, out.height), (128, 128), "the asked size");
        assert_eq!(out.edges, 16, "the whole humanoid tree drew");
        assert_eq!(out.coverage_permille, 1000);
        assert!(
            out.lit_pixels > 500,
            "the bones lit only {}",
            out.lit_pixels
        );
        let bytes = std::fs::read(root.join("frames").join("rig.png")).expect("the PNG is there");
        assert_eq!(
            &bytes[..8],
            &[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a][..],
            "PNG magic"
        );
        assert_eq!(
            bytes.len(),
            usize::try_from(out.bytes).expect("a 4 GiB PNG"),
            "byte count"
        );
        // The IHDR chunk holds the frame size, so the file really is the frame.
        let width = u32::from_be_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]);
        let height = u32::from_be_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]);
        assert_eq!((width, height), (128, 128), "the PNG header size");
        let replay = preview_frame(
            &root,
            &server(),
            &preview_args(retarget_args(unreal_bones()), "rig.png"),
        )
        .expect("a replay writes");
        assert_eq!(out.digest, replay.digest, "the frame moved between runs");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_manual_row_colours_the_frame() {
        let root = scratch("manual");
        let plain = preview_frame(
            &root,
            &server(),
            &preview_args(retarget_args(unreal_bones()), "plain.png"),
        )
        .expect("the plain frame writes");
        let mut bones = unreal_bones();
        bones.retain(|name| name != "pelvis");
        bones.push("hips_custom".to_owned());
        let hinted = preview_frame(
            &root,
            &server(),
            &preview_args(
                RetargetArgs {
                    overrides: vec![BoneOverrideDto {
                        source: "hips_custom".to_owned(),
                        role: "pelvis".to_owned(),
                    }],
                    ..retarget_args(bones)
                },
                "hinted.png",
            ),
        )
        .expect("the manual frame writes");
        assert_eq!(plain.edges, hinted.edges, "one body, one tree");
        assert_eq!(
            plain.lit_pixels, hinted.lit_pixels,
            "same bones, same pixels"
        );
        assert_ne!(
            plain.digest, hinted.digest,
            "the manual colour missed the frame"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_refused_map_writes_no_frame() {
        let root = scratch("refused");
        let mut bones = unreal_bones();
        bones.push("bone_007".to_owned());
        let refused = preview_frame(
            &root,
            &server(),
            &preview_args(retarget_args(bones), "none.png"),
        )
        .expect_err("an unknown bone refuses");
        assert_eq!(refused.code, "unknown_bone");
        assert!(
            !root.join("frames").join("none.png").exists(),
            "a refusal wrote a frame"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_frame_stays_under_the_root_at_a_legal_size() {
        let root = scratch("paths");
        let good = retarget_args(unreal_bones());
        let escaping = RetargetPreviewArgs {
            out: "../evil.png".to_owned(),
            ..preview_args(good.clone(), "ok.png")
        };
        let refused = preview_frame(&root, &server(), &escaping).expect_err("escaping out");
        assert_eq!(refused.code, "bad_args");
        assert!(refused.message.contains("out"), "{}", refused.message);

        let absolute = RetargetPreviewArgs {
            dir: "/etc".to_owned(),
            ..preview_args(good.clone(), "ok.png")
        };
        assert_eq!(
            preview_frame(&root, &server(), &absolute)
                .expect_err("absolute dir")
                .code,
            "bad_args"
        );

        let not_png = RetargetPreviewArgs {
            out: "ok.jpg".to_owned(),
            ..preview_args(good.clone(), "ok.png")
        };
        assert_eq!(
            preview_frame(&root, &server(), &not_png)
                .expect_err("no png suffix")
                .code,
            "bad_args"
        );

        for side in [0u32, PREVIEW_MAX_SIZE + 1] {
            let sized = RetargetPreviewArgs {
                width: Some(side),
                ..preview_args(good.clone(), "ok.png")
            };
            let refused = preview_frame(&root, &server(), &sized).expect_err("side {side}");
            assert_eq!(refused.code, "bad_size");
            assert!(refused.message.contains("width"), "{}", refused.message);
        }
        assert!(
            !root.join("frames").join("ok.png").exists(),
            "a refused call wrote a frame"
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}
