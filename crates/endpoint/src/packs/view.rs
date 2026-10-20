//! `view` pack: what the door sees about itself and the run.
//!
//! Holds the observe-log digest and pages, the self-describing manifest,
//! `view.observe` and `view.laws` (the live view as numbers) and
//! `look.judge`, which scores a capture set by machine so a Look row accepts
//! on a number instead of an opened PNG.
//!
//! `view.observe` and `view.laws` read the live bout of the session's play
//! slot exactly as `sim.observe` reads it (seed, headcount and tick count):
//! no bout answers `no_live_bout` first. The door owns no view and links no
//! renderer: a host that links the SP61 view truth registers it once with
//! [`register_view_source`], and the door passes back its camera, bodies and
//! `arena view:` line unchanged. The door checks the host's answer against
//! the contract (one body per live fighter, finite rect or null, ordered
//! bounds, a camera inside its ranges) and refuses `bad_view` otherwise,
//! never a partial success. Until a host registers, both verbs refuse
//! `unsupported_view_source`, never a success shape. Units: meters, radians,
//! seconds, degrees, picture share `0..=1`, fixed ticks. Determinism: the
//! door adds nothing to the host's numbers; same seed plus tape twice gives
//! the same bytes. Errors: [`McpToolError`] with `no_live_bout`,
//! `unsupported_view_source`, `bad_view` or the host's own code.
//!
//! `view.laws` has a narrower host too: a host that can fold the bout's
//! `arena view:` line but cannot draw a frame registers [`ViewLawsFn`] with
//! [`register_view_laws_source`]. The shipped door registers
//! [`arena_view_line`], which spawns `arena.exe --view-line <seed> <ticks>
//! <fighters>` (no window, no GPU) and reads its one prefixed line, so
//! `view.laws` answers in every build. `view.observe` stays
//! `unsupported_view_source` until a host that links the renderer registers
//! a camera and every drawn body: the CPU pose alone cannot supply the
//! picture share, and the door never fakes one.
//!
//! `look.judge` follows the capture pattern: the endpoint owns no judge. A
//! host that links the judge registers it once with [`register_look_judge`];
//! until then the verb refuses as `unsupported_look_judge`, never a success
//! shape. The door checks the directory arguments before anything runs (a
//! relative path with no `..`) and checks the judge's answer against the
//! contract before it passes it on: schema 1, a 16-hex digest, nine rubric
//! items `L1`..`L9` in order with integer permille scores, no float anywhere.
//! Units: permille for scores, chars for path bounds. Determinism: the door
//! adds nothing to the judge's bytes. Errors: [`McpToolError`] with
//! `bad_args`, `unsupported_look_judge`, `bad_receipt` or the judge's own code.

use std::borrow::Cow;
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

use rmcp::handler::server::router::tool::{SyncTool, ToolBase};
use rmcp::schemars::JsonSchema;
use rmcp::serde::{Deserialize, Serialize};
use rmcp::serde_json::{self, Value};

use super::bad_args;
use super::{PackDef, PackError, VerbSpec};
use crate::discover::{DiscoverTool, TOOL_RPC_DISCOVER};
use crate::log_tools::{LogDigestTool, LogQueryTool, TOOL_LOG_DIGEST, TOOL_LOG_QUERY};
use crate::mcp::{McpServer, McpToolError};
use crate::play::play_session::PlaySlot;
use crate::tool_capabilities::{CapabilityCard, CheckKind};

/// Pack name.
pub const PACK: &str = "view";

/// Verb: score a capture set by machine.
pub const TOOL_LOOK_JUDGE: &str = "look.judge";

/// Verb: read the live view's camera, drawn bodies and laws.
pub const TOOL_VIEW_OBSERVE: &str = "view.observe";

/// Verb: read the live view's `arena view:` laws line.
pub const TOOL_VIEW_LAWS: &str = "view.laws";

/// Rubric ids a judge answer must carry, in order.
pub const LOOK_RUBRIC_IDS: [&str; 9] = ["L1", "L2", "L3", "L4", "L5", "L6", "L7", "L8", "L9"];

/// Schema version a judge answer must carry.
pub const LOOK_JUDGE_SCHEMA: u64 = 1;

/// Longest directory argument in chars.
const MAX_DIR_CHARS: usize = 512;

/// The view pack: the live view as numbers, the log digest and pages, the
/// manifest and the look judge, loaded on demand.
pub fn pack() -> Result<PackDef, PackError> {
    let mut pack = PackDef::new(
        PACK,
        "what the door sees: the live view as numbers, observe logs, the manifest, look.judge scores",
        false,
    );
    pack.add(VerbSpec::of::<ViewObserveTool>(
        TOOL_VIEW_OBSERVE,
        PACK,
        CapabilityCard::Observe,
    )?);
    pack.add(VerbSpec::of::<ViewLawsTool>(
        TOOL_VIEW_LAWS,
        PACK,
        CapabilityCard::Observe,
    )?);
    pack.add(VerbSpec::of::<LogDigestTool>(
        TOOL_LOG_DIGEST,
        PACK,
        CapabilityCard::Observe,
    )?);
    pack.add(VerbSpec::of::<LogQueryTool>(
        TOOL_LOG_QUERY,
        PACK,
        CapabilityCard::Observe,
    )?);
    pack.add(VerbSpec::of::<DiscoverTool>(
        TOOL_RPC_DISCOVER,
        PACK,
        CapabilityCard::Observe,
    )?);
    pack.add(VerbSpec::of::<LookJudgeTool>(
        TOOL_LOOK_JUDGE,
        PACK,
        CapabilityCard::Observe,
    )?);
    Ok(pack)
}

// ---------------------------------------------------------------------------
// Transport DTOs. Field docs are omitted on purpose: every doc line inflates
// the advertised schemas, and the discovery budget is a gate.
// ---------------------------------------------------------------------------

/// Judge request: a capture directory (PNG frames plus an optional
/// `manifest.json`), optionally a fighter-preview directory (default
/// `content/fighters/previews`) and a Q1 reference directory. Units are
/// relative directory paths. Errors are typed `bad_args` for a path that is
/// absolute, climbs with `..` or is empty. Budget is 512 chars a path.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct LookJudgeArgs {
    pub capture_dir: String,
    pub preview_dir: Option<String>,
    pub q1_dir: Option<String>,
}

/// Judge answer: schema, digest, verdict, mean score and the full report
/// (rubric L1..L9, per-frame scores, entity-grounded regions, sequence).
/// Units are permille. No errors once judged. Budget rides on the capture
/// set (at most 64 frames).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct LookJudgeOutput {
    pub schema: u64,
    pub digest: String,
    pub pass: bool,
    pub score: u64,
    pub judgement: Value,
}

/// A registered judge: reads the directories named in the request and
/// returns the report as JSON text, or the refusal to pass on.
pub type LookJudgeFn = fn(&LookJudgeArgs) -> Result<String, McpToolError>;

static LOOK_JUDGE: OnceLock<LookJudgeFn> = OnceLock::new();

/// Register the judge for this process. The host that links the judge calls
/// this once at start; a second call refuses, so no later code swaps the
/// judge under a running door.
///
/// # Errors
///
/// `look_judge_registered` when a judge is already registered.
pub fn register_look_judge(judge: LookJudgeFn) -> Result<(), McpToolError> {
    LOOK_JUDGE.set(judge).map_err(|_| {
        McpToolError::new(
            "look_judge_registered",
            "a look judge is already registered for this process; \
             restart the host to register another",
        )
    })
}

/// A directory argument must be a non-empty relative path of plain names.
fn check_dir(field: &str, raw: &str) -> Result<(), McpToolError> {
    let plain = !raw.is_empty()
        && raw.chars().count() <= MAX_DIR_CHARS
        && !raw.contains(['\0', '\\', ':'])
        && Path::new(raw)
            .components()
            .all(|part| matches!(part, Component::Normal(_) | Component::CurDir));
    if plain {
        return Ok(());
    }
    Err(bad_args(format!(
        "{field} `{}` is not a relative directory of plain names (1..={MAX_DIR_CHARS} chars, \
         `/` separators, no `..`, no drive, colon, backslash or leading slash); re-run \
         with a path such as target/reports/staged-fight/captures",
        raw.chars().take(80).collect::<String>()
    )))
}

fn check_args(args: &LookJudgeArgs) -> Result<(), McpToolError> {
    check_dir("capture_dir", &args.capture_dir)?;
    for (field, value) in [("preview_dir", &args.preview_dir), ("q1_dir", &args.q1_dir)] {
        if let Some(raw) = value {
            check_dir(field, raw)?;
        }
    }
    Ok(())
}

/// Unwired judge: structured unsupported, never a success shape.
fn unsupported() -> McpToolError {
    McpToolError::new(
        "unsupported_look_judge",
        "no look judge is registered: the endpoint owns no judge, so a host must call \
         register_look_judge; absent judge is unsupported, never success. Next: run the \
         judge from a host that links it, or read the frames with the capture verb",
    )
}

fn contract(why: &str) -> McpToolError {
    McpToolError::new(
        "bad_receipt",
        format!("the judge answered outside its contract ({why}); re-run after fixing the judge"),
    )
}

/// No float anywhere: every JSON number is an integer.
fn all_integers(value: &Value) -> bool {
    match value {
        Value::Number(n) => n.is_i64() || n.is_u64(),
        Value::Array(items) => items.iter().all(all_integers),
        Value::Object(map) => map.values().all(all_integers),
        _ => true,
    }
}

fn permille_of(item: &Value, key: &str) -> Option<u64> {
    item.get(key)?.as_u64().filter(|score| *score <= 1000)
}

/// Check the judge's answer against the contract and lift the headline.
fn validate(text: &str) -> Result<LookJudgeOutput, McpToolError> {
    let judgement: Value =
        serde_json::from_str(text).map_err(|_| contract("the answer is not JSON"))?;
    if !all_integers(&judgement) {
        return Err(contract("a number is not an integer"));
    }
    let schema = judgement.get("schema").and_then(Value::as_u64);
    if schema != Some(LOOK_JUDGE_SCHEMA) {
        return Err(contract("schema is not 1"));
    }
    let digest = judgement
        .get("digest")
        .and_then(Value::as_str)
        .filter(|d| d.len() == 16 && d.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')))
        .ok_or_else(|| contract("digest is not 16 lowercase hex"))?
        .to_owned();
    let pass = judgement
        .get("pass")
        .and_then(Value::as_bool)
        .ok_or_else(|| contract("pass is missing"))?;
    let score =
        permille_of(&judgement, "score").ok_or_else(|| contract("score is not 0..=1000"))?;
    let rubric = judgement.get("rubric").and_then(Value::as_array);
    let ordered = rubric.is_some_and(|items| {
        items.len() == LOOK_RUBRIC_IDS.len()
            && items.iter().zip(LOOK_RUBRIC_IDS).all(|(item, id)| {
                item.get("id").and_then(Value::as_str) == Some(id)
                    && permille_of(item, "score").is_some()
                    && item.get("pass").is_some_and(Value::is_boolean)
            })
    });
    if !ordered {
        return Err(contract(
            "rubric is not L1..L9 in order with permille scores",
        ));
    }
    if !judgement
        .get("frames")
        .and_then(Value::as_array)
        .is_some_and(|frames| !frames.is_empty())
    {
        return Err(contract("frames is empty"));
    }
    Ok(LookJudgeOutput {
        schema: LOOK_JUDGE_SCHEMA,
        digest,
        pass,
        score,
        judgement,
    })
}

/// Run one request against an optional judge: arguments first, then the
/// refusal when no judge is wired, then the contract check on its answer.
fn run_look_judge(
    args: &LookJudgeArgs,
    judge: Option<LookJudgeFn>,
) -> Result<LookJudgeOutput, McpToolError> {
    check_args(args)?;
    let judge = judge.ok_or_else(unsupported)?;
    validate(&judge(args)?)
}

/// `look.judge` tool: machine rubric scores for a capture set, read-only.
pub struct LookJudgeTool;

impl ToolBase for LookJudgeTool {
    type Parameter = LookJudgeArgs;
    type Output = LookJudgeOutput;
    type Error = McpToolError;

    fn name() -> Cow<'static, str> {
        TOOL_LOOK_JUDGE.into()
    }

    fn description() -> Option<Cow<'static, str>> {
        Some(
            "Machine look score for a capture directory: rubric L1-L9 from pixels, \
             frame checks and references, as integer permille JSON. Read-only; \
             unsupported until a judge is registered."
                .into(),
        )
    }

    fn annotations() -> Option<rmcp::model::ToolAnnotations> {
        crate::mcp::read_only_annotations()
    }
}

impl SyncTool<McpServer> for LookJudgeTool {
    fn invoke(service: &McpServer, param: Self::Parameter) -> Result<Self::Output, Self::Error> {
        service.dispatch(TOOL_LOOK_JUDGE, |_state| {
            run_look_judge(&param, LOOK_JUDGE.get().copied())
        })
    }
}

// ---------------------------------------------------------------------------
// view.observe and view.laws: the live view as numbers, from a host that
// links the SP61 view truth.
// ---------------------------------------------------------------------------

/// Camera the host drew with: eye and target in meters, vertical fov in
/// degrees.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct ViewCamera {
    pub eye: [f32; 3],
    pub target: [f32; 3],
    pub fov_deg: f32,
}

/// One drawn body as the host posed it.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct ViewBody {
    pub slot: u8,
    pub act: String,
    pub clip: u8,
    pub tint: bool,
    pub root: [f32; 3],
    pub min: Option<[f32; 3]>,
    pub max: Option<[f32; 3]>,
    pub screen: Option<[f32; 4]>,
    pub cover: f32,
}

/// One live frame from the view host: the camera, every drawn body and the
/// `arena view:` laws line so far. The door returns these bytes unchanged
/// after the contract check.
#[derive(Debug, Clone, PartialEq)]
pub struct ViewFrame {
    /// Ticks simulated when the frame was drawn (the live bout's count).
    pub tick: u32,
    /// Camera that drew the frame.
    pub camera: ViewCamera,
    /// Every drawn body, one per live fighter.
    pub bodies: Vec<ViewBody>,
    /// The `arena view:` laws line, byte for byte; `""` at tick 0.
    pub laws: String,
}

/// A registered view host: `(seed, ticks, fighters)` from the live bout in,
/// one [`ViewFrame`] out, or the refusal to pass on. The host links the SP61
/// view truth; the door never links a renderer.
pub type ViewSourceFn = fn(seed: u64, ticks: u32, fighters: u8) -> Result<ViewFrame, McpToolError>;

static VIEW_SOURCE: OnceLock<ViewSourceFn> = OnceLock::new();

/// Register the view host for this process. The host that links the SP61 view
/// truth calls this once at start; a second call refuses, so no later code
/// swaps the view under a running door.
///
/// # Errors
///
/// `view_source_registered` when a host is already registered.
pub fn register_view_source(source: ViewSourceFn) -> Result<(), McpToolError> {
    VIEW_SOURCE.set(source).map_err(|_| {
        McpToolError::new(
            "view_source_registered",
            "a view host is already registered for this process; \
             restart the host to register another",
        )
    })
}

/// A registered laws host: `(seed, ticks, fighters)` from the live bout in,
/// the `arena view:` line out, or the refusal to pass on. A host that can fold
/// the line but not draw a frame registers this one, so `view.laws` answers
/// without claiming a camera or a body it cannot supply.
pub type ViewLawsFn = fn(seed: u64, ticks: u32, fighters: u8) -> Result<String, McpToolError>;

static VIEW_LAWS: OnceLock<ViewLawsFn> = OnceLock::new();

/// Register the laws host for this process. One-shot like
/// [`register_view_source`]: a second call refuses, so no later code swaps the
/// line under a running door.
///
/// # Errors
///
/// `view_laws_registered` when a laws host is already registered.
pub fn register_view_laws_source(source: ViewLawsFn) -> Result<(), McpToolError> {
    VIEW_LAWS.set(source).map_err(|_| {
        McpToolError::new(
            "view_laws_registered",
            "a view laws host is already registered for this process; \
             restart the host to register another",
        )
    })
}

/// Environment override for the arena binary the view and seat hosts spawn.
pub const ARENA_EXE_ENV: &str = "E2040_ARENA_EXE";

/// Shipped arena binary path, relative to the door's working directory.
pub const DEFAULT_ARENA_EXE: &str = "target/release/arena.exe";

/// Prefix `arena.exe --view-line` prints before the laws line, so a boot or
/// warning line can never be mistaken for the answer.
pub const VIEW_LINE_PREFIX: &str = "arena view-line: ";

/// Longest laws line a host may return, in chars.
const MAX_VIEW_LAWS_CHARS: usize = 4096;

/// The arena binary the hosts spawn: [`ARENA_EXE_ENV`] when set, else
/// [`DEFAULT_ARENA_EXE`]. Pure over the given value; the caller checks that the
/// path is a file so a missing build fails loudly with the path it tried.
fn arena_exe_from(value: Option<std::ffi::OsString>) -> PathBuf {
    value.map_or_else(|| PathBuf::from(DEFAULT_ARENA_EXE), PathBuf::from)
}

/// [`arena_exe_from`] over the process environment.
#[must_use]
pub fn arena_exe_path() -> PathBuf {
    arena_exe_from(std::env::var_os(ARENA_EXE_ENV))
}

/// Args for `view.observe`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
#[expect(
    clippy::empty_structs_with_brackets,
    reason = "serde reads the wire shape `{}`; a unit struct reads null"
)]
pub struct ViewObserveArgs {}

/// `view.observe` result: the live view after the bout's last tick.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct ViewObserveOutput {
    pub tick: u32,
    pub seed: String,
    pub participants: u8,
    pub camera: ViewCamera,
    pub bodies: Vec<ViewBody>,
    pub laws: String,
    pub check: String,
}

/// Args for `view.laws`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
#[expect(
    clippy::empty_structs_with_brackets,
    reason = "serde reads the wire shape `{}`; a unit struct reads null"
)]
pub struct ViewLawsArgs {}

/// `view.laws` result: the live view's laws line.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct ViewLawsOutput {
    pub tick: u32,
    pub laws: String,
}

/// No bout: the refusal `sim.observe` gives, same code and same recovery.
fn no_live_bout() -> McpToolError {
    McpToolError::new(
        "no_live_bout",
        "no live bout in this session; call sim.reset or sim.step first, then re-run view.observe \
         or view.laws",
    )
}

/// Unwired host: structured unsupported, never a success shape.
fn unsupported_view_source() -> McpToolError {
    McpToolError::new(
        "unsupported_view_source",
        "no view host is registered: the door owns no view, so a host that links the SP61 view \
         truth must call register_view_source; absent host is unsupported, never success. Next: \
         run the door in a host that links the view truth",
    )
}

fn bad_view(why: impl std::fmt::Display) -> McpToolError {
    McpToolError::new(
        "bad_view",
        format!(
            "the view host answered outside its contract ({why}); re-run after fixing the host"
        ),
    )
}

/// A finite camera inside its ranges.
fn check_camera(camera: &ViewCamera) -> Result<(), McpToolError> {
    if !camera
        .eye
        .iter()
        .chain(&camera.target)
        .all(|v| v.is_finite())
    {
        return Err(bad_view("a camera vector is not finite"));
    }
    if !(camera.fov_deg.is_finite() && camera.fov_deg > 0.0 && camera.fov_deg < 180.0) {
        return Err(bad_view(format!(
            "fov_deg {} is not in (0, 180)",
            camera.fov_deg
        )));
    }
    Ok(())
}

/// One body inside the contract: an act, both bounds or neither, a finite
/// ordered rect inside the picture, a finite cover in `0..=1`.
fn check_body(body: &ViewBody) -> Result<(), McpToolError> {
    if body.act.is_empty() {
        return Err(bad_view(format!("body slot {} has no act", body.slot)));
    }
    match (body.min, body.max) {
        (Some(lo), Some(hi)) => {
            let finite = (0..3).all(|axis| lo[axis].is_finite() && hi[axis].is_finite());
            let ordered = (0..3).all(|axis| lo[axis] <= hi[axis]);
            if !finite || !ordered {
                return Err(bad_view(format!(
                    "body slot {} bounds are not finite and ordered",
                    body.slot
                )));
            }
        }
        (None, None) => {}
        _ => {
            return Err(bad_view(format!(
                "body slot {} carries one bound and not the other",
                body.slot
            )))
        }
    }
    if let Some(rect) = body.screen {
        let finite = rect.iter().all(|v| v.is_finite());
        let ordered = rect[0] < rect[2] && rect[1] < rect[3];
        let inside = rect.iter().all(|v| (-1.0..=1.0).contains(v));
        if !finite || !ordered || !inside {
            return Err(bad_view(format!(
                "body slot {} screen rect {rect:?} is not a finite ordered rect inside -1..=1",
                body.slot
            )));
        }
    }
    if !(body.cover.is_finite() && (0.0..=1.0).contains(&body.cover)) {
        return Err(bad_view(format!(
            "body slot {} cover {} is not in 0..=1",
            body.slot, body.cover
        )));
    }
    Ok(())
}

/// The host's frame against the live bout: same tick, one body per live
/// fighter in `0..participants`, a laws line once a tick was folded.
fn check_frame(frame: &ViewFrame, ticks: u32, participants: u8) -> Result<(), McpToolError> {
    if frame.tick != ticks {
        return Err(bad_view(format!(
            "frame tick {} is not the live bout's {ticks}",
            frame.tick
        )));
    }
    check_camera(&frame.camera)?;
    if ticks > 0 && frame.laws.is_empty() {
        return Err(bad_view(format!(
            "laws line is empty at tick {ticks}; the host must fold the run"
        )));
    }
    let mut seen = vec![false; usize::from(participants)];
    for body in &frame.bodies {
        let Some(here) = seen.get_mut(usize::from(body.slot)) else {
            return Err(bad_view(format!(
                "body slot {} is outside 0..{participants}",
                body.slot
            )));
        };
        if *here {
            return Err(bad_view(format!("body slot {} appears twice", body.slot)));
        }
        *here = true;
        check_body(body)?;
    }
    if let Some(slot) = seen.iter().position(|seen| !seen) {
        return Err(bad_view(format!(
            "body slot {slot} is missing; the host must name every live fighter once"
        )));
    }
    Ok(())
}

/// Read the live bout's view spec and the host's frame for it. Refusal order
/// matches `sim.observe`: no bout first, then no host, then the host's own
/// refusal, then the contract check.
fn live_frame(
    slot: &PlaySlot,
    source: Option<ViewSourceFn>,
) -> Result<(u64, u8, u32, ViewFrame), McpToolError> {
    let bout = slot.current().ok_or_else(no_live_bout)?;
    let spec = bout.spec();
    let seed = spec.seed;
    let participants = spec.participants;
    let ticks = bout.observe(0).tick;
    let source = source.ok_or_else(unsupported_view_source)?;
    let frame = source(seed, ticks, participants)?;
    check_frame(&frame, ticks, participants)?;
    Ok((seed, participants, ticks, frame))
}

/// Pure observe core: the live bout's view. Steps nothing and writes nothing.
fn observe_core(
    slot: &PlaySlot,
    source: Option<ViewSourceFn>,
) -> Result<ViewObserveOutput, McpToolError> {
    let (seed, participants, ticks, frame) = live_frame(slot, source)?;
    Ok(ViewObserveOutput {
        tick: ticks,
        seed: seed.to_string(),
        participants,
        camera: frame.camera,
        bodies: frame.bodies,
        laws: frame.laws,
        check: CheckKind::ReadOnly.name().to_owned(),
    })
}

/// Pure laws core: the live bout's `arena view:` line, unchanged. A laws host
/// answers with the line alone; without one the older view source's frame line
/// serves, and with neither the verb refuses `unsupported_view_source`.
fn laws_core(
    slot: &PlaySlot,
    source: Option<ViewSourceFn>,
    laws: Option<ViewLawsFn>,
) -> Result<ViewLawsOutput, McpToolError> {
    if let Some(laws) = laws {
        let bout = slot.current().ok_or_else(no_live_bout)?;
        let spec = bout.spec();
        let ticks = bout.observe(0).tick;
        let line = laws(spec.seed, ticks, spec.participants)?;
        check_laws_line(&line, ticks)?;
        return Ok(ViewLawsOutput {
            tick: ticks,
            laws: line,
        });
    }
    let (_, _, ticks, frame) = live_frame(slot, source)?;
    Ok(ViewLawsOutput {
        tick: ticks,
        laws: frame.laws,
    })
}

/// The host's laws line against the contract: at most
/// [`MAX_VIEW_LAWS_CHARS`], one line, the `arena view:` head once a tick was
/// folded, and empty at tick 0.
fn check_laws_line(line: &str, ticks: u32) -> Result<(), McpToolError> {
    if line.chars().count() > MAX_VIEW_LAWS_CHARS {
        return Err(bad_view(format!(
            "laws line is longer than {MAX_VIEW_LAWS_CHARS} chars"
        )));
    }
    if line.contains(['\n', '\r']) {
        return Err(bad_view("laws line carries a line break"));
    }
    if ticks == 0 {
        if !line.is_empty() {
            return Err(bad_view("laws line is not empty at tick 0"));
        }
        return Ok(());
    }
    if !line.starts_with("arena view: ") {
        return Err(bad_view(format!(
            "laws line `{}` is not an `arena view:` line",
            line.chars().take(40).collect::<String>()
        )));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// The arena subprocess host: `arena.exe --view-line` folds the same headless
// bout `FighterLook::view_run` draws, prints one prefixed line and exits, so
// `view.laws` answers in a build that links no renderer.
// ---------------------------------------------------------------------------

/// The laws line in `stdout`, when the arena printed the prefixed line.
fn view_line_from_stdout(stdout: &str) -> Option<String> {
    stdout.lines().find_map(|line| {
        line.strip_prefix(VIEW_LINE_PREFIX)
            .map(|laws| laws.trim_end_matches('\r').to_owned())
    })
}

/// The last line of a child's stderr, for a loud refusal.
fn error_tail(stderr: &[u8]) -> String {
    let text = String::from_utf8_lossy(stderr);
    let tail = text.lines().rev().find(|line| !line.trim().is_empty());
    tail.map_or_else(|| "no stderr".to_owned(), |line| line.trim().to_owned())
}

/// `view.laws` served by a windowless `arena.exe`: spawn the shipped binary
/// with `--view-line <seed> <ticks> <fighters>`, read its one prefixed line and
/// pass it on unchanged. The path is [`ARENA_EXE_ENV`] or
/// [`DEFAULT_ARENA_EXE`], relative to the door's working directory.
///
/// The line folds the bout with no scripted input, exactly as
/// `FighterLook::view_run` does, so it is the live bout's own line when that
/// bout ran the empty tape.
///
/// # Errors
///
/// `arena_exe_missing` names the path tried when no binary is there,
/// `arena_view_failed` carries the exit status plus the child's stderr tail,
/// and `bad_view` when the child printed no `arena view-line:` line.
pub fn arena_view_line(seed: u64, ticks: u32, fighters: u8) -> Result<String, McpToolError> {
    arena_view_line_from(&arena_exe_path(), seed, ticks, fighters)
}

/// [`arena_view_line`] over an explicit binary path: the test seam, and the
/// one owner of the spawn, the exit check and the line parse.
fn arena_view_line_from(
    exe: &Path,
    seed: u64,
    ticks: u32,
    fighters: u8,
) -> Result<String, McpToolError> {
    if !exe.is_file() {
        return Err(McpToolError::new(
            "arena_exe_missing",
            format!(
                "no arena binary at `{}`; build one with \
                 `cargo build -p engine-game --release --features app --bin arena` or set \
                 {ARENA_EXE_ENV} to a built arena.exe, then re-run",
                exe.display()
            ),
        ));
    }
    let output = Command::new(exe)
        .arg("--view-line")
        .arg(seed.to_string())
        .arg(ticks.to_string())
        .arg(fighters.to_string())
        .output()
        .map_err(|err| {
            McpToolError::new(
                "arena_view_failed",
                format!(
                    "`{}` did not run ({err}); re-run with a built arena.exe",
                    exe.display()
                ),
            )
        })?;
    if !output.status.success() {
        return Err(McpToolError::new(
            "arena_view_failed",
            format!(
                "`{}` exited {}: {}; re-run with a built arena.exe",
                exe.display(),
                output.status,
                error_tail(&output.stderr)
            ),
        ));
    }
    view_line_from_stdout(&String::from_utf8_lossy(&output.stdout)).ok_or_else(|| {
        McpToolError::new(
            "bad_view",
            format!(
                "`{}` printed no `{VIEW_LINE_PREFIX}` line; re-run with a built arena.exe",
                exe.display()
            ),
        )
    })
}

fn view_laws(service: &McpServer, _args: &ViewLawsArgs) -> Result<ViewLawsOutput, McpToolError> {
    service.dispatch(TOOL_VIEW_LAWS, |state| {
        laws_core(
            &state.play,
            VIEW_SOURCE.get().copied(),
            VIEW_LAWS.get().copied(),
        )
    })
}

fn view_observe(
    service: &McpServer,
    _args: &ViewObserveArgs,
) -> Result<ViewObserveOutput, McpToolError> {
    service.dispatch(TOOL_VIEW_OBSERVE, |state| {
        observe_core(&state.play, VIEW_SOURCE.get().copied())
    })
}

crate::e2_tool!(
    /// `view.observe` tool: the live view's camera, drawn bodies and laws. Read-only.
    ViewObserveTool,
    name = TOOL_VIEW_OBSERVE,
    description = "Live view as numbers: camera, every drawn body (act, clip, tint, root, bounds, screen rect, cover) and the arena view laws line, from the view host. Run sim.reset or sim.step first. Writes nothing.",
    readonly = true,
    args = ViewObserveArgs,
    output = ViewObserveOutput,
    invoke = view_observe
);

crate::e2_tool!(
    /// `view.laws` tool: the live view's `arena view:` laws line. Read-only.
    ViewLawsTool,
    name = TOOL_VIEW_LAWS,
    description = "The arena view laws line for the live bout, byte for byte from the view host: frames, ticks, pops, sinking, foot slide, ragdolls. Run sim.reset or sim.step first. The shipped door folds the bout's seed, ticks and fighters over the shipped 20 m arena with no scripted input through arena.exe; the host's line is passed on unchanged. Writes nothing.",
    readonly = true,
    args = ViewLawsArgs,
    output = ViewLawsOutput,
    invoke = view_laws
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::insight::mcp_tools::{ApiDescribeArgs, ApiDescribeTool};
    use crate::packs::PackRegistry;
    use engine_editor::capability::Capability;

    fn server() -> McpServer {
        McpServer::new(7, Capability::unrestricted(7))
    }

    fn args(dir: &str) -> LookJudgeArgs {
        LookJudgeArgs {
            capture_dir: dir.to_owned(),
            preview_dir: None,
            q1_dir: None,
        }
    }

    /// A contract-clean answer: nine items, one frame, integers only.
    fn answer(edit: impl Fn(String) -> String) -> Result<String, McpToolError> {
        let items: Vec<String> = LOOK_RUBRIC_IDS
            .iter()
            .map(|id| format!("{{\"id\":\"{id}\",\"score\":500,\"pass\":false}}"))
            .collect();
        Ok(edit(format!(
            "{{\"schema\":1,\"digest\":\"00ff00ff00ff00ff\",\"pass\":false,\"score\":500,\
             \"rubric\":[{}],\"frames\":[{{\"name\":\"frame_000000.png\"}}]}}",
            items.join(",")
        )))
    }

    fn good(_: &LookJudgeArgs) -> Result<String, McpToolError> {
        answer(|text| text)
    }

    #[test]
    fn look_judge_view_pack_lists_the_verb_and_api_describe_reads_it() {
        let verb = PackRegistry::shipped()
            .verb(TOOL_LOOK_JUDGE)
            .expect("look.judge is a registered verb");
        assert_eq!(verb.pack(), PACK);
        assert_eq!(verb.card(), CapabilityCard::Observe);
        assert!(verb.readonly());
        assert!(PackRegistry::shipped()
            .pack(PACK)
            .expect("view pack")
            .verb_names()
            .contains(&TOOL_LOOK_JUDGE));
        let described = ApiDescribeTool::invoke(
            &server(),
            ApiDescribeArgs {
                path: TOOL_LOOK_JUDGE.to_owned(),
            },
        )
        .expect("api.describe lists look.judge");
        assert_eq!(described.path, TOOL_LOOK_JUDGE);
        assert!(described.docs.contains("rubric L1-L9"));
        assert!(described.signature.contains("view"));
    }

    #[test]
    fn look_judge_refuses_by_default_until_a_judge_is_registered() {
        let error = LookJudgeTool::invoke(&server(), args("target/captures"))
            .expect_err("no judge is wired in the library tests");
        assert_eq!(error.code, "unsupported_look_judge");
        assert!(error.message.contains("never success"));
        let via_registry = PackRegistry::shipped()
            .verb(TOOL_LOOK_JUDGE)
            .expect("verb")
            .invoke(
                &server(),
                serde_json::json!({"capture_dir": "target/captures"}),
            )
            .expect_err("the registry path refuses the same way");
        assert_eq!(via_registry.code, "unsupported_look_judge");
    }

    #[test]
    fn look_judge_checks_paths_before_anything_runs() {
        let long = "a".repeat(MAX_DIR_CHARS + 1);
        for bad in [
            "",
            "..",
            "../up",
            "a/../../b",
            "/abs",
            "\\abs",
            "C:\\x",
            "C:x",
            "a\0b",
            &long,
        ] {
            let error = run_look_judge(&args(bad), Some(good)).expect_err(bad);
            assert_eq!(error.code, "bad_args", "{bad:?}");
            assert!(error.message.contains("re-run"));
        }
        for ok in [
            "target/captures",
            "./captures",
            "a",
            "content/fighters/previews",
        ] {
            assert!(run_look_judge(&args(ok), Some(good)).is_ok(), "{ok}");
        }
        let mut with_bad_q1 = args("target/captures");
        with_bad_q1.q1_dir = Some("../q1".to_owned());
        assert_eq!(
            run_look_judge(&with_bad_q1, Some(good))
                .expect_err("q1 escapes")
                .code,
            "bad_args"
        );
        let mut with_bad_previews = args("target/captures");
        with_bad_previews.preview_dir = Some("/previews".to_owned());
        assert!(run_look_judge(&with_bad_previews, None)
            .expect_err("previews")
            .message
            .contains("preview_dir"));
        // Bad args win over the unwired refusal.
        assert_eq!(
            run_look_judge(&args(".."), None).expect_err("bad").code,
            "bad_args"
        );
    }

    #[test]
    fn look_judge_wired_judge_answers_typed_json() {
        let out = run_look_judge(&args("target/captures"), Some(good)).expect("answers");
        assert_eq!((out.schema, out.pass, out.score), (1, false, 500));
        assert_eq!(out.digest, "00ff00ff00ff00ff");
        assert_eq!(out.judgement["rubric"].as_array().map(Vec::len), Some(9));
        assert_eq!(out.judgement["rubric"][8]["id"], "L9");
        let wire = serde_json::to_string(&out).expect("serializes");
        assert!(wire.contains("\"judgement\""));
    }

    #[test]
    fn look_judge_refuses_answers_outside_the_contract() {
        let bad = |edit: fn(String) -> String, why: &str| {
            let error = validate(&answer(edit).expect("fixture builds")).expect_err(why);
            assert_eq!(error.code, "bad_receipt", "{why}");
            assert!(error.message.contains(why), "{why}: {}", error.message);
        };
        bad(
            |t| t.replace("\"schema\":1", "\"schema\":2"),
            "schema is not 1",
        );
        bad(
            |t| t.replace("00ff00ff00ff00ff", "00FF00ff00ff00ff"),
            "digest is not 16 lowercase hex",
        );
        bad(
            |t| t.replace("00ff00ff00ff00ff", "00ff"),
            "digest is not 16 lowercase hex",
        );
        bad(
            |t| t.replace("\"score\":500,\"rubric\"", "\"score\":500.5,\"rubric\""),
            "a number is not an integer",
        );
        bad(
            |t| t.replace("\"score\":500,\"rubric\"", "\"score\":1001,\"rubric\""),
            "score is not 0..=1000",
        );
        bad(
            |t| t.replace("\"pass\":false,\"score\"", "\"score\""),
            "pass is missing",
        );
        bad(
            |t| t.replace("\"id\":\"L9\"", "\"id\":\"L8\""),
            "rubric is not L1..L9 in order",
        );
        bad(
            |t| {
                t.replace(
                    "{\"id\":\"L9\",\"score\":500,\"pass\":false}",
                    "{\"id\":\"L9\",\"score\":9000,\"pass\":false}",
                )
            },
            "rubric is not L1..L9 in order",
        );
        bad(
            |t| t.replace(",{\"id\":\"L9\",\"score\":500,\"pass\":false}", ""),
            "rubric is not L1..L9 in order",
        );
        bad(
            |t| t.replace("[{\"name\":\"frame_000000.png\"}]", "[]"),
            "frames is empty",
        );
        bad(|_| "not json".to_owned(), "the answer is not JSON");
        // A judge's own refusal passes through untouched.
        fn refusing(_: &LookJudgeArgs) -> Result<String, McpToolError> {
            Err(McpToolError::new(
                "no_frames",
                "capture set holds no frame; re-run",
            ))
        }
        let passed =
            run_look_judge(&args("target/empty"), Some(refusing)).expect_err("judge refuses");
        assert_eq!(
            (passed.code, passed.message.as_str()),
            ("no_frames", "capture set holds no frame; re-run")
        );
    }

    // ---- view.observe and view.laws: the live view as numbers --------------

    use crate::packs::gym::{SimResetArgs, SimResetTool, SimStepArgs, SimStepTool};
    use crate::play::PlayInputDto;
    use rmcp::handler::server::router::tool::SyncTool;

    /// Match seed: the bout these door tests step.
    const BOUT_SEED: u64 = 2;

    /// The exact `arena view:` line SP61 prints for its breach fixture
    /// (`view_truth.rs`, `breaches_count_and_the_receipt_line_is_exact`).
    /// The door must pass it byte for byte.
    const SP61_VIEW_LINE: &str = "arena view: frames=2 sim_ticks=1.0..2.0 bodies=3 offscreen_share=0.33 nonfinite=1 error_parts=1 sunk_cm_max=12.0 idle_float_cm_max=4.0 pops=0 pop_acts=none pop_cm_max=0.0 eye_in_body=0 eye_below_ground=1 closest_cm=4100 cover_max=0.02 foot_slide_cm=0.0 planted_feet=0 ragdolls=0 spear=0 tether=0";

    /// Fighter 0 closes and swings at tick 0, then swings again at tick 30.
    fn script() -> Vec<PlayInputDto> {
        vec![
            PlayInputDto {
                tick: 0,
                fighter: 0,
                mx: 100,
                mz: 0,
                facing: 0,
                buttons: 0x01,
            },
            PlayInputDto {
                tick: 30,
                fighter: 0,
                mx: 60,
                mz: 0,
                facing: 0,
                buttons: 0x01,
            },
        ]
    }

    fn step_args(seed: u64, ticks: u32) -> SimStepArgs {
        SimStepArgs {
            seed: Some(seed),
            participants: Some(2),
            half_extent_m: Some(20.0),
            ticks: Some(ticks),
            inputs: Some(
                script()
                    .into_iter()
                    .filter(|input| input.tick < ticks)
                    .collect(),
            ),
            events: Some(64),
        }
    }

    fn step(server: &McpServer, seed: u64, ticks: u32) {
        SimStepTool::invoke(server, step_args(seed, ticks)).expect("step serves");
    }

    fn fixture_camera() -> ViewCamera {
        ViewCamera {
            eye: [0.0, 5.0, 12.0],
            target: [0.0, 1.0, 0.0],
            fov_deg: 50.0,
        }
    }

    /// A contract-clean frame: slot 0 draws on screen with bounds, later
    /// slots sit behind the camera with no rect and no bounds.
    fn fixture(_seed: u64, ticks: u32, fighters: u8) -> Result<ViewFrame, McpToolError> {
        let bodies = (0..fighters)
            .map(|slot| ViewBody {
                slot,
                act: "idle".to_owned(),
                clip: 0,
                tint: slot % 2 == 1,
                root: [f32::from(slot), 0.0, 0.0],
                min: (slot == 0).then_some([-0.3, 0.0, -0.3]),
                max: (slot == 0).then_some([0.3, 1.8, 0.3]),
                screen: (slot == 0).then_some([-0.2, -0.4, 0.2, 0.4]),
                cover: if slot == 0 { 0.08 } else { 0.0 },
            })
            .collect();
        Ok(ViewFrame {
            tick: ticks,
            camera: fixture_camera(),
            bodies,
            laws: if ticks == 0 {
                String::new()
            } else {
                SP61_VIEW_LINE.to_owned()
            },
        })
    }

    fn observe_with(server: &McpServer) -> ViewObserveOutput {
        let state = server.lock().expect("test lock holds");
        observe_core(&state.play, Some(fixture)).expect("view.observe serves")
    }

    fn laws_with(server: &McpServer) -> ViewLawsOutput {
        let state = server.lock().expect("test lock holds");
        laws_core(&state.play, Some(fixture), None).expect("view.laws serves")
    }

    fn observe_json(server: &McpServer) -> String {
        rmcp::serde_json::to_string(&observe_with(server)).expect("view.observe serializes")
    }

    #[test]
    fn view_observe_names_every_live_fighter_after_120_ticks() {
        let server = server();
        step(&server, BOUT_SEED, 120);
        let seen = observe_with(&server);
        assert_eq!(seen.tick, 120, "the view is drawn at the bout's tick");
        assert_eq!(seen.seed, "2");
        assert_eq!(seen.participants, 2);
        assert_eq!(seen.check, CheckKind::ReadOnly.name());
        assert_eq!(seen.laws, SP61_VIEW_LINE, "the laws ride along unchanged");
        let slots: Vec<u8> = seen.bodies.iter().map(|body| body.slot).collect();
        assert_eq!(
            slots,
            [0, 1],
            "every live fighter is named once, in slot order"
        );
        for body in &seen.bodies {
            assert!(
                body.cover.is_finite() && (0.0..=1.0).contains(&body.cover),
                "slot {} cover {}",
                body.slot,
                body.cover
            );
            if let Some(rect) = body.screen {
                assert!(
                    rect.iter().all(|v| v.is_finite()),
                    "slot {} rect {rect:?} is finite",
                    body.slot
                );
            }
        }
        assert!(
            seen.bodies[0].screen.is_some(),
            "the body in front reads a rect"
        );
        assert!(
            seen.bodies[1].screen.is_none(),
            "a body behind the camera reads null"
        );
        assert!(seen.bodies[0].min.is_some() && seen.bodies[0].max.is_some());
        assert!(seen.bodies[1].min.is_none() && seen.bodies[1].max.is_none());
    }

    #[test]
    fn view_observe_one_tape_twice_is_byte_identical() {
        let first = {
            let session = server();
            step(&session, BOUT_SEED, 120);
            observe_json(&session)
        };
        let second = {
            let session = server();
            step(&session, BOUT_SEED, 120);
            observe_json(&session)
        };
        assert_eq!(first, second, "two sessions, one tape, one byte stream");
        // One session replaying the same tape through a reset says the same.
        let replay = server();
        step(&replay, BOUT_SEED, 120);
        let live = observe_json(&replay);
        SimResetTool::invoke(
            &replay,
            SimResetArgs {
                seed: Some(BOUT_SEED),
                participants: Some(2),
                half_extent_m: Some(20.0),
            },
        )
        .expect("reset serves");
        step(&replay, BOUT_SEED, 120);
        assert_eq!(
            live,
            observe_json(&replay),
            "reset plus the tape replays the bytes"
        );
        assert_eq!(live, first);
        // Another seed changes the bout identity, so the bytes differ.
        let other = {
            let session = server();
            step(&session, BOUT_SEED + 1, 120);
            observe_json(&session)
        };
        assert_ne!(first, other, "another seed is another view");
    }

    #[test]
    fn view_laws_pops_the_sp61_line() {
        let server = server();
        step(&server, BOUT_SEED, 120);
        let laws = laws_with(&server);
        assert_eq!(laws.laws, SP61_VIEW_LINE, "byte for byte from SP61");
        assert_eq!(laws.tick, 120);
        assert_eq!(
            laws.laws,
            observe_with(&server).laws,
            "view.observe and view.laws pop the same line"
        );
    }

    #[test]
    fn view_laws_at_tick_zero_pops_the_empty_line() {
        let server = server();
        SimResetTool::invoke(
            &server,
            SimResetArgs {
                seed: Some(BOUT_SEED),
                participants: Some(2),
                half_extent_m: Some(20.0),
            },
        )
        .expect("reset serves");
        let laws = laws_with(&server);
        assert_eq!(laws.tick, 0);
        assert_eq!(laws.laws, "", "no frame folded yet, so no line");
    }

    #[test]
    fn view_refuses_without_a_bout_then_without_a_source() {
        let server = server();
        {
            let state = server.lock().expect("test lock holds");
            for error in [
                observe_core(&state.play, Some(fixture)).expect_err("nothing stepped yet"),
                laws_core(&state.play, Some(fixture), None).expect_err("nothing stepped yet"),
            ] {
                assert_eq!(error.code, "no_live_bout");
                assert!(error.message.contains("sim.step"), "{}", error.message);
            }
        }
        step(&server, BOUT_SEED, 8);
        {
            let state = server.lock().expect("test lock holds");
            for error in [
                observe_core(&state.play, None).expect_err("no host"),
                laws_core(&state.play, None, None).expect_err("no host"),
            ] {
                assert_eq!(error.code, "unsupported_view_source");
                assert!(
                    error.message.contains("register_view_source"),
                    "{}",
                    error.message
                );
                assert!(error.message.contains("never success"), "{}", error.message);
            }
        }
    }

    #[test]
    fn view_refuses_a_host_frame_outside_the_contract() {
        let good = fixture(BOUT_SEED, 120, 2).expect("fixture builds");
        let outside = |edit: &dyn Fn(&mut ViewFrame), why: &str| {
            let mut frame = good.clone();
            edit(&mut frame);
            let error = check_frame(&frame, 120, 2).expect_err(why);
            assert_eq!(error.code, "bad_view", "{why}");
            assert!(error.message.contains(why), "{why}: {}", error.message);
        };
        outside(&|frame| frame.tick = 119, "frame tick");
        outside(&|frame| frame.camera.eye = [f32::NAN; 3], "camera");
        outside(&|frame| frame.camera.fov_deg = 0.0, "fov_deg");
        outside(&|frame| frame.laws.clear(), "laws line is empty");
        outside(
            &|frame| {
                frame.bodies.pop();
            },
            "slot 1 is missing",
        );
        outside(&|frame| frame.bodies[1].slot = 0, "slot 0 appears twice");
        outside(&|frame| frame.bodies[1].slot = 2, "outside 0..2");
        outside(&|frame| frame.bodies[0].act.clear(), "no act");
        outside(&|frame| frame.bodies[0].max = None, "one bound");
        outside(
            &|frame| frame.bodies[0].min = Some([1.0, 0.0, 0.0]),
            "not finite and ordered",
        );
        outside(
            &|frame| frame.bodies[0].screen = Some([0.5, 0.0, -0.5, 1.0]),
            "screen rect",
        );
        outside(
            &|frame| frame.bodies[0].screen = Some([f32::NAN, -0.4, 0.2, 0.4]),
            "screen rect",
        );
        outside(&|frame| frame.bodies[0].cover = 1.5, "cover");
        // A host's own refusal passes through untouched.
        fn refusing(_: u64, _: u32, _: u8) -> Result<ViewFrame, McpToolError> {
            Err(McpToolError::new(
                "view_host_busy",
                "host is drawing; re-run",
            ))
        }
        let server = server();
        step(&server, BOUT_SEED, 8);
        let state = server.lock().expect("test lock holds");
        let passed = observe_core(&state.play, Some(refusing)).expect_err("host refuses");
        assert_eq!(
            (passed.code, passed.message.as_str()),
            ("view_host_busy", "host is drawing; re-run")
        );
    }

    #[test]
    fn view_tools_register_in_the_pack_and_api_describe_reads_them() {
        let registry = PackRegistry::shipped();
        let pack = registry.pack(PACK).expect("view pack");
        for (name, needle) in [
            (TOOL_VIEW_OBSERVE, "screen rect"),
            (TOOL_VIEW_LAWS, "arena view laws line"),
        ] {
            let verb = registry.verb(name).expect("the view verb registers");
            assert_eq!(verb.pack(), PACK);
            assert_eq!(verb.card(), CapabilityCard::Observe);
            assert!(verb.readonly(), "{name} is read-only");
            assert!(
                pack.verb_names().contains(&name),
                "{name} lists in the pack"
            );
            let described = ApiDescribeTool::invoke(
                &server(),
                ApiDescribeArgs {
                    path: name.to_owned(),
                },
            )
            .expect("api.describe reads the verb");
            assert_eq!(described.path, name);
            assert!(described.docs.contains(needle), "{}", described.docs);
            assert!(
                described.signature.contains("view"),
                "{}",
                described.signature
            );
        }
    }

    // ---- the laws host: line only, no camera and no bodies ----------------

    /// A laws host built from the live bout: names the seed, ticks and fighters
    /// it was handed, so a test proves the door forwards the bout's spec.
    fn fixture_laws(seed: u64, ticks: u32, fighters: u8) -> Result<String, McpToolError> {
        Ok(format!(
            "arena view: frames={fighters} sim_ticks=0.5..{ticks}.5 seed={seed} bodies={fighters}"
        ))
    }

    #[test]
    fn view_laws_host_answers_the_bout_line_without_a_frame() {
        let server = server();
        step(&server, BOUT_SEED, 120);
        let state = server.lock().expect("test lock holds");
        let laws = laws_core(&state.play, None, Some(fixture_laws)).expect("laws host serves");
        assert_eq!(laws.tick, 120);
        assert!(laws.laws.starts_with("arena view: "), "{}", laws.laws);
        assert!(laws.laws.contains("seed=2"), "{}", laws.laws);
        assert!(laws.laws.contains("bodies=2"), "{}", laws.laws);
        assert_eq!(
            laws,
            laws_core(&state.play, None, Some(fixture_laws)).expect("same answer twice"),
            "one bout, one line"
        );
    }

    #[test]
    fn view_laws_host_refuses_a_line_outside_the_contract() {
        let long = "x".repeat(MAX_VIEW_LAWS_CHARS + 1);
        for (line, why, ticks) in [
            (
                "not a view line".to_owned(),
                "is not an `arena view:` line",
                8u32,
            ),
            (
                "arena view: a\narena view: b".to_owned(),
                "carries a line break",
                8,
            ),
            (
                "arena view: at tick zero".to_owned(),
                "not empty at tick 0",
                0,
            ),
            (long, "longer than", 8),
        ] {
            let error = check_laws_line(&line, ticks).expect_err(why);
            assert_eq!(error.code, "bad_view", "{why}");
            assert!(error.message.contains(why), "{why}: {}", error.message);
        }
        assert!(
            check_laws_line("", 0).is_ok(),
            "tick 0 serves the empty line"
        );
        assert!(check_laws_line("arena view: frames=1", 1).is_ok());
        assert_eq!(
            check_laws_line("", 1).expect_err("no line").code,
            "bad_view",
            "a folded tick must carry its line"
        );
    }

    #[test]
    fn view_laws_parse_the_arena_windowless_line() {
        let stdout =
            "arena log: target/arena.log\narena view-line: arena view: frames=3 bodies=2\n";
        assert_eq!(
            view_line_from_stdout(stdout).as_deref(),
            Some("arena view: frames=3 bodies=2")
        );
        assert_eq!(view_line_from_stdout("arena smoke: ticks=0\n"), None);
        assert_eq!(
            view_line_from_stdout("arena view-line: \r\n").as_deref(),
            Some(""),
            "the empty tick-0 line still crosses"
        );
        assert_eq!(
            arena_exe_from(Some("D:/x/arena.exe".into())),
            PathBuf::from("D:/x/arena.exe"),
            "the env override wins"
        );
        assert_eq!(arena_exe_from(None), PathBuf::from(DEFAULT_ARENA_EXE));
    }

    #[test]
    fn view_laws_host_refuses_a_missing_arena_binary_loudly() {
        // A path no build can be at: the refusal names the path it tried and
        // the env var that overrides it, and never falls back to a fake line.
        let exe = PathBuf::from("target/no-such-build/arena.exe");
        let error = arena_view_line_from(&exe, 2040, 600, 2).expect_err("no binary there");
        assert_eq!(error.code, "arena_exe_missing");
        assert!(
            error.message.contains(&exe.display().to_string()),
            "{}",
            error.message
        );
        assert!(error.message.contains(ARENA_EXE_ENV), "{}", error.message);
        assert!(error.message.contains("re-run"), "{}", error.message);
    }
}
