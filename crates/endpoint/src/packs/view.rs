//! `view` pack: what the door sees about itself and the run.
//!
//! Holds the observe-log digest and pages, the self-describing manifest,
//! `view.observe` and `view.laws` (the live view as numbers), `view.compare`
//! (two captures over the whole frame or one rectangle) and `look.judge`,
//! which scores a capture set by machine so a Look row accepts on a number
//! instead of an opened PNG.
//!
//! `view.observe` and `view.laws` read the live bout of the session's play
//! slot exactly as `sim.observe` reads it (seed, headcount and tick count):
//! no bout answers `no_live_bout` first. The door owns no view and links no
//! renderer: a host that links the SP61 view truth registers it once with
//! [`register_view_source`], and the door passes back its camera, bodies and
//! `arena view:` line unchanged. `view.observe` also numbers the bodies it can
//! point at: one [`ViewMark`] per body with a screen rect, id = the body slot,
//! so a sentence about "body 1" is an id and not a paragraph. The door checks
//! the host's answer against
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

use engine_core::hash::hex16;
use engine_core::Fnv64;
use engine_playtest::look::metric::{self, Frame, Region};
use engine_playtest::look::ssim2;
use rmcp::handler::server::router::tool::{SyncTool, ToolBase};
use rmcp::schemars::JsonSchema;
use rmcp::serde::{Deserialize, Serialize};
use rmcp::serde_json::{self, Value};

use super::bad_args;
use super::{PackDef, PackError, VerbSpec};
use crate::collapse::MAX_ADVERTISED_CHARS;
use crate::discover::{DiscoverTool, TOOL_RPC_DISCOVER};
use crate::log_tools::{LogDigestTool, LogQueryTool, TOOL_LOG_DIGEST, TOOL_LOG_QUERY};
use crate::mcp::{McpServer, McpToolError, ServerState};
use crate::play::play_session::{encode_png_rgb, PlaySlot};
use crate::tool_capabilities::{CapabilityCard, CheckKind};

/// The art lane: contact sheet, look proposal, ignored-region compare.
pub mod art;

/// Pack name.
pub const PACK: &str = "view";

/// Verb: score a capture set by machine.
pub const TOOL_LOOK_JUDGE: &str = "look.judge";

/// Verb: read the live view's camera, drawn bodies and laws.
pub const TOOL_VIEW_OBSERVE: &str = "view.observe";

/// Verb: read the live view's `arena view:` laws line.
pub const TOOL_VIEW_LAWS: &str = "view.laws";

/// Verb: compare two PNG captures with the machine metrics.
pub const TOOL_VIEW_COMPARE: &str = "view.compare";

/// Verb: the door's own health (session, revision, packs, budget, audit head).
pub const TOOL_DOOR_STATUS: &str = "door.status";

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
    pack.add(VerbSpec::of::<DoorStatusTool>(
        TOOL_DOOR_STATUS,
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
    pack.add(VerbSpec::of::<ViewCompareTool>(
        TOOL_VIEW_COMPARE,
        PACK,
        CapabilityCard::Observe,
    )?);
    pack.add(VerbSpec::of::<art::ViewSheetTool>(
        art::TOOL_VIEW_SHEET,
        PACK,
        CapabilityCard::Observe,
    )?);
    pack.add(VerbSpec::of::<art::LookProposeTool>(
        art::TOOL_LOOK_PROPOSE,
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
// view.compare: two PNGs, one machine score, one optional diff image
// ---------------------------------------------------------------------------

/// The picture area one verb scores: the whole frame or one rectangle.
///
/// The rectangle is in pixels with the origin at the top-left, the same space
/// as a capture PNG. `{"kind":"full"}` is the default and scores every pixel;
/// `{"kind":"rect","x":..,"y":..,"w":..,"h":..}` scores pixels inside it only.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(crate = "rmcp::serde", tag = "kind", rename_all = "snake_case")]
#[schemars(crate = "rmcp::schemars")]
pub enum CompareRegion {
    /// Every pixel of the frame.
    Full,
    /// Pixels inside the rectangle only.
    Rect {
        /// Left edge in pixels.
        x: u32,
        /// Top edge in pixels.
        y: u32,
        /// Width in pixels, at least 1.
        w: u32,
        /// Height in pixels, at least 1.
        h: u32,
    },
}

impl CompareRegion {
    /// Whether the region is the whole frame.
    fn is_full(self) -> bool {
        matches!(self, Self::Full)
    }

    /// The rectangle, when the region is one.
    fn rect(self) -> Option<metric::Rect> {
        match self {
            Self::Full => None,
            Self::Rect { x, y, w, h } => Some(metric::Rect { x, y, w, h }),
        }
    }
}

/// Two relative PNG paths and an optional third to write, plus the region to
/// score. Units are path chars and pixels; errors are typed `bad_args` for an
/// absolute path, `..`, a non-PNG name, an unreadable input or a rectangle
/// with no area or one outside the frame. `diff` is written under the project
/// root, never read, and need not exist; a write failure is `write_failed`.
/// Without `region` the whole frame scores, exactly as before the field
/// existed.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct ViewCompareArgs {
    pub left: String,
    pub right: String,
    pub diff: Option<String>,
    pub region: Option<CompareRegion>,
    pub ignore: Option<Vec<CompareRegion>>,
}

/// Machine score over the resolved region. All scores are fixed point; 10000
/// is identical for SSIM and 0 is identical for FLIP and deltaE.
/// `ssimulacra2_x10000` is SSIMULACRA2 v2.1 in hundredths of a donor point:
/// 10000 is identical, 0 is worst and higher is better, the same direction as
/// SSIM. It is `None` when the donor refused the pair (mismatched sizes, or a
/// side under 8x8). `region` is `full` or `rect` and `rect` is the resolved
/// `[x, y, w, h]` in pixels, `None` for a full-frame score. `width` and
/// `height` stay the frame size, so a region score still names the picture it
/// came from. `diff_png` is the relative path the diff image was written to,
/// or `None` when the request named no diff path.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct ViewCompareOutput {
    pub width: u32,
    pub height: u32,
    pub ssim_x10000: i32,
    pub flip_x10000: u32,
    pub mean_de_x100: u32,
    pub hist_tv_x10000: u32,
    pub hist_emd_x10000: u32,
    pub ssimulacra2_x10000: Option<u32>,
    pub diff_png: Option<String>,
    pub score_x10000: Option<u32>,
    pub region: String,
    pub rect: Option<[u32; 4]>,
}

/// A PNG path must be a non-empty relative path of plain names ending in
/// `.png`. The two inputs are read; a diff path is written and never read, so
/// it need not exist yet.
fn check_png(field: &str, raw: &str) -> Result<(), McpToolError> {
    check_dir(field, raw)?;
    if raw.to_ascii_lowercase().ends_with(".png") {
        return Ok(());
    }
    Err(bad_args(format!(
        "{field} `{}` is not a .png file; re-run with a path such as \
         target/reports/staged-fight/captures/frame.png",
        raw.chars().take(80).collect::<String>()
    )))
}

/// Decode one bounded PNG under `root`; the error names the side and the path.
fn read_frame(field: &str, root: &Path, rel: &str) -> Result<Frame, McpToolError> {
    let bytes = std::fs::read(root.join(rel)).map_err(|error| {
        McpToolError::new(
            "bad_args",
            format!("{field} `{rel}` could not be read: {error}"),
        )
    })?;
    Frame::decode_png(&bytes).map_err(|error| {
        McpToolError::new(
            "bad_image",
            format!("{field} `{rel}` is not a decodable PNG: {error}"),
        )
    })
}

/// Resolve the request's region against the decoded frame, with the error a
/// caller can act on. Nothing is scored before this passes.
fn resolve_region(
    arg: Option<CompareRegion>,
    frame: &Frame,
) -> Result<(CompareRegion, Region<'static>), McpToolError> {
    let region = arg.unwrap_or(CompareRegion::Full);
    let Some(rect) = region.rect() else {
        return Ok((region, Region::Full));
    };
    let inside = u64::from(rect.x) + u64::from(rect.w) <= u64::from(frame.width())
        && u64::from(rect.y) + u64::from(rect.h) <= u64::from(frame.height());
    if rect.w == 0 || rect.h == 0 || !inside {
        return Err(McpToolError::new(
            "bad_args",
            format!(
                "region {}x{} at ({},{}) scores no pixel of the {}x{} frame; legal kinds are \
                 {{\"kind\":\"full\"}} or {{\"kind\":\"rect\",\"x\":0,\"y\":0,\"w\":{},\"h\":{}}} \
                 with w and h at least 1 and the rect inside the frame: re-run with a rect \
                 inside the capture",
                rect.w,
                rect.h,
                rect.x,
                rect.y,
                frame.width(),
                frame.height(),
                frame.width(),
                frame.height()
            ),
        ));
    }
    Ok((region, Region::Rect(rect)))
}

/// One frame cut to `rect`, as a fresh frame. Used so the second metric scores
/// the same pixels as the first.
fn crop(frame: &Frame, rect: metric::Rect) -> Result<Frame, McpToolError> {
    let to_usize = |v: u32| usize::try_from(v).unwrap_or(0);
    let (w, h) = (to_usize(rect.w), to_usize(rect.h));
    let width = to_usize(frame.width());
    let (left, top) = (to_usize(rect.x), to_usize(rect.y));
    let mut rgba = Vec::with_capacity(w * h * 4);
    for y in 0..h {
        for x in 0..w {
            let at = (top + y) * width + left + x;
            rgba.extend_from_slice(&frame.rgb_at(at));
            rgba.push(255);
        }
    }
    Frame::from_rgba(rect.w, rect.h, rgba)
        .map_err(|error| McpToolError::new("bad_image", format!("region does not crop: {error}")))
}

/// Score two decoded frames over `region`.
///
/// The second metric reads `left` as the source (the pristine reference)
/// and `right` as the distorted one (the capture), so its score says how
/// far the capture moved from the reference. The donor is float, so the
/// score is published as a coarse fixed point: 10000 is identical, 0 is
/// worst. A rectangle is cropped for the second metric, which scores one
/// whole picture and has no region argument.
fn compare_frames_in(
    left: &Frame,
    right: &Frame,
    region: CompareRegion,
    scoring: Region<'_>,
) -> Result<ViewCompareOutput, McpToolError> {
    let compared = metric::compare(left, right, scoring).map_err(|error| {
        McpToolError::new("bad_image", format!("frames do not compare: {error}"))
    })?;
    let second = match region.rect() {
        Some(rect) => match (crop(left, rect), crop(right, rect)) {
            (Ok(left), Ok(right)) => ssim2::score_x10000(&left, &right).ok(),
            _ => None,
        },
        None => ssim2::score_x10000(left, right).ok(),
    };
    Ok(ViewCompareOutput {
        width: left.width(),
        height: left.height(),
        ssim_x10000: compared.ssim_x10000,
        flip_x10000: compared.flip_x10000,
        mean_de_x100: compared.mean_de_x100,
        hist_tv_x10000: compared.hist_tv_x10000,
        hist_emd_x10000: compared.hist_emd_x10000,
        ssimulacra2_x10000: second,
        diff_png: None,
        score_x10000: None,
        region: if region.is_full() { "full" } else { "rect" }.to_owned(),
        rect: region.rect().map(|rect| [rect.x, rect.y, rect.w, rect.h]),
    })
}

/// Whole-frame score: [`compare_frames_in`] over every pixel, the answer a
/// caller that names no region gets.
fn compare_frames(left: &Frame, right: &Frame) -> Result<ViewCompareOutput, McpToolError> {
    compare_frames_in(left, right, CompareRegion::Full, Region::Full)
}

/// Amplification of the per-channel gap in the diff image: a gap of 1 reads 8,
/// so a near-match still shows where the unaided eye would miss it.
const DIFF_AMPLIFY: u32 = 8;

/// One diff channel: `min(255, |a - b| * DIFF_AMPLIFY)`.
fn diff_channel(a: u8, b: u8) -> u8 {
    u8::try_from(u32::from(a.abs_diff(b)) * DIFF_AMPLIFY).unwrap_or(u8::MAX)
}

/// Write the amplified per-pixel RGB difference of two same-size frames at
/// `root/rel`. The caller scores the pair first, so a refused pair writes
/// nothing and the image always matches the size the output reports.
fn write_diff_png(root: &Path, rel: &str, left: &Frame, right: &Frame) -> Result<(), McpToolError> {
    let mut rgb = Vec::with_capacity(left.pixels() * 3);
    for at in 0..left.pixels() {
        for (a, b) in left.rgb_at(at).into_iter().zip(right.rgb_at(at)) {
            rgb.push(diff_channel(a, b));
        }
    }
    let failed = |why: std::io::Error| {
        McpToolError::new(
            "write_failed",
            format!(
                "could not write diff `{rel}` under the project root ({why}); re-run with a \
                 writable path such as target/reports/diff.png"
            ),
        )
    };
    let path = root.join(rel);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(failed)?;
    }
    std::fs::write(&path, encode_png_rgb(left.width(), left.height(), &rgb)).map_err(failed)
}

/// Score one pair over the requested region, then write the diff image when
/// the request named one.
fn run_view_compare(
    root: &Path,
    args: &ViewCompareArgs,
) -> Result<ViewCompareOutput, McpToolError> {
    check_png("left", &args.left)?;
    check_png("right", &args.right)?;
    if let Some(diff) = &args.diff {
        check_png("diff", diff)?;
    }
    let left = read_frame("left", root, &args.left)?;
    let right = read_frame("right", root, &args.right)?;
    let (region, scoring) = resolve_region(args.region, &left)?;
    let mut out = compare_frames_in(&left, &right, region, scoring)?;
    if let Some(ignore) = &args.ignore {
        out.score_x10000 = Some(art::compare_ignoring(&left, &right, args.region, ignore)?);
    }
    if let Some(diff) = &args.diff {
        write_diff_png(root, diff, &left, &right)?;
        out.diff_png = Some(diff.clone());
    }
    Ok(out)
}

/// `view.compare` tool: two PNG captures scored by machine, plus the optional
/// amplified diff image; read-only on the captures.
pub struct ViewCompareTool;

impl ToolBase for ViewCompareTool {
    type Parameter = ViewCompareArgs;
    type Output = ViewCompareOutput;
    type Error = McpToolError;

    fn name() -> Cow<'static, str> {
        TOOL_VIEW_COMPARE.into()
    }

    fn description() -> Option<Cow<'static, str>> {
        Some(
            "Compare two relative PNG captures over the whole frame: SSIM, FLIP, \
             deltaE, histogram distances and SSIMULACRA2 v2.1 as fixed-point \
             integers. An optional `diff` names a relative .png that is written \
             under the project root as the per-pixel channel gap times 8 \
             (amplified, 255 saturates) and returned as `diff_png`; with no \
             `diff` nothing is written. SSIMULACRA2 reads 10000 for identical \
             frames. Read-only on the captures."
                .into(),
        )
    }

    fn annotations() -> Option<rmcp::model::ToolAnnotations> {
        crate::mcp::read_only_annotations()
    }
}

impl SyncTool<McpServer> for ViewCompareTool {
    fn invoke(_service: &McpServer, param: Self::Parameter) -> Result<Self::Output, Self::Error> {
        run_view_compare(&super::codex::project_root(None), &param)
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

/// One numbered region over a body, the Set-of-Mark shape: an id a person and
/// a client can both name, a short label and the picture rect in `-1..=1`
/// picture share, the same space as [`ViewBody::screen`].
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct ViewMark {
    /// Stable mark id: the body's slot, so the mark, the body and a later
    /// `scene.pick` on the same fighter name one thing.
    pub id: u64,
    /// Short label, `body_<slot>`.
    pub label: String,
    /// Picture rect `[x0, y0, x1, y1]` in `-1..=1` share of the frame.
    pub rect: [f32; 4],
}

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
    pub marks: Vec<ViewMark>,
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

/// Args for `door.status`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
#[expect(
    clippy::empty_structs_with_brackets,
    reason = "serde reads the wire shape `{}`; a unit struct reads null"
)]
pub struct DoorStatusArgs {}

/// `door.status` result: the door's own health. Read-only.
///
/// `digest` covers the fields that do not move when the door is read (session,
/// revision, packs, tool count, surface and budget); the audit head and count
/// are the values this call saw before its own record, so an unchanged session
/// reports the same digest twice. `durable_generation` and `build_sha` are
/// `None` when the host holds no durable store or build stamp: unknown stays
/// unknown, never a guessed zero.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct DoorStatusOutput {
    pub session: u64,
    pub revision: u64,
    pub packs: Vec<String>,
    pub tools: u64,
    pub surface_chars: u64,
    pub budget_chars: u64,
    pub headroom_chars: u64,
    pub audit_head: Option<String>,
    pub audit_total: u64,
    pub durable_generation: Option<u64>,
    pub build_sha: Option<String>,
    pub digest: String,
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

/// The marks of one frame: one per body that has a screen rect, in body order.
/// Pure over the bodies, so two identical calls report the same ids. A body
/// behind the camera has no rect and so no mark: a mark always names pixels.
fn marks_of(bodies: &[ViewBody]) -> Vec<ViewMark> {
    bodies
        .iter()
        .filter_map(|body| {
            body.screen.map(|rect| ViewMark {
                id: u64::from(body.slot),
                label: format!("body_{}", body.slot),
                rect,
            })
        })
        .collect()
}

/// Pure observe core: the live bout's view. Steps nothing and writes nothing.
fn observe_core(
    slot: &PlaySlot,
    source: Option<ViewSourceFn>,
) -> Result<ViewObserveOutput, McpToolError> {
    let (seed, participants, ticks, frame) = live_frame(slot, source)?;
    let marks = marks_of(&frame.bodies);
    Ok(ViewObserveOutput {
        tick: ticks,
        seed: seed.to_string(),
        participants,
        camera: frame.camera,
        bodies: frame.bodies,
        laws: frame.laws,
        check: CheckKind::ReadOnly.name().to_owned(),
        marks,
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

/// The status payload. The audit fields are read inside the dispatch, before
/// the door seals this call's own record, so a reader reports the chain it saw.
fn status_core(service: &McpServer, state: &ServerState) -> DoorStatusOutput {
    let session = state.session;
    let revision = state.doc.revision;
    let packs: Vec<String> = state.loaded_packs.iter().map(|p| (*p).to_owned()).collect();
    let tools = u64::try_from(service.tool_count_with(&state.loaded_packs)).unwrap_or(u64::MAX);
    let surface_chars =
        u64::try_from(service.surface_chars_with(&state.loaded_packs)).unwrap_or(u64::MAX);
    let budget_chars = u64::try_from(MAX_ADVERTISED_CHARS).unwrap_or(u64::MAX);
    let headroom_chars = budget_chars.saturating_sub(surface_chars);
    let audit_head = state.audit_head().map(|head| crate::play::hex_bytes(&head));
    let audit_total = state.audit_total_appended();
    let durable_generation = None;
    let build_sha = option_env!("E2040_BUILD_SHA").map(str::to_owned);
    let digest = status_digest(
        session,
        revision,
        &packs,
        tools,
        surface_chars,
        budget_chars,
    );
    DoorStatusOutput {
        session,
        revision,
        packs,
        tools,
        surface_chars,
        budget_chars,
        headroom_chars,
        audit_head,
        audit_total,
        durable_generation,
        build_sha,
        digest,
    }
}

/// Domain tag so a status digest cannot collide with another FNV lane.
const STATUS_DOMAIN: u64 = 0x646f_6f72_7374_6174;

/// Digest of the stable fields: two calls on an unchanged session report the
/// same digest even though each call appends its own audit record.
fn status_digest(
    session: u64,
    revision: u64,
    packs: &[String],
    tools: u64,
    surface_chars: u64,
    budget_chars: u64,
) -> String {
    let mut lane = Fnv64::with_state(STATUS_DOMAIN);
    lane.write_u64(session);
    lane.write_u64(revision);
    lane.write_u64(tools);
    lane.write_u64(surface_chars);
    lane.write_u64(budget_chars);
    lane.write_len(packs.len());
    for pack in packs {
        lane.write_len(pack.len());
        lane.write_str(pack);
    }
    hex16(lane.finish())
}

fn door_status(
    service: &McpServer,
    _args: &DoorStatusArgs,
) -> Result<DoorStatusOutput, McpToolError> {
    service.dispatch(TOOL_DOOR_STATUS, |state| Ok(status_core(service, state)))
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

crate::e2_tool!(
    /// `door.status` tool: the door's own health. Read-only.
    DoorStatusTool,
    name = TOOL_DOOR_STATUS,
    description = "The door's own health as numbers: session, scene revision, loaded packs, advertised tools and chars, the budget ceiling and its headroom, the audit chain head and count, a build stamp. Unknown stays null. This call's own audit record lands after the answer. Read-only.",
    readonly = true,
    args = DoorStatusArgs,
    output = DoorStatusOutput,
    invoke = door_status
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::insight::mcp_tools::{ApiDescribeArgs, ApiDescribeTool};
    use crate::packs::{ApiLoadArgs, ApiLoadTool, PackRegistry};
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

    /// A 16x16 textured frame: SSIM needs at least one whole 8x8 window.
    fn frame_of(paint: impl Fn(usize) -> [u8; 4]) -> Frame {
        let mut rgba = Vec::with_capacity(16 * 16 * 4);
        for i in 0..16 * 16 {
            rgba.extend_from_slice(&paint(i));
        }
        Frame::from_rgba(16, 16, rgba).expect("16x16 frame")
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

    #[test]
    fn view_compare_scores_identical_frames_one_and_zero_flip() {
        let left = frame_of(|i| {
            [
                u8::try_from((i * 7) % 230).expect("byte"),
                u8::try_from((i * 13) % 200).expect("byte"),
                u8::try_from((i * 3) % 180).expect("byte"),
                255,
            ]
        });
        let right = left.clone();
        let compared = compare_frames(&left, &right).expect("identical frames compare");
        assert_eq!(compared.width, 16);
        assert_eq!(compared.height, 16);
        assert_eq!(compared.ssim_x10000, 10000);
        assert_eq!(compared.flip_x10000, 0);
        assert_eq!(compared.mean_de_x100, 0);
        assert_eq!(compared.hist_tv_x10000, 0);
        assert_eq!(compared.hist_emd_x10000, 0);
        // The second metric: identical frames are the identical value, the
        // same 10000 the SSIM score uses for a perfect match.
        assert_eq!(compared.ssimulacra2_x10000, Some(10000));
    }

    #[test]
    fn view_compare_scores_a_changed_frame_below_one() {
        let dark = frame_of(|i| {
            [
                u8::try_from((i * 5) % 120).expect("byte"),
                u8::try_from((i * 11) % 100).expect("byte"),
                u8::try_from((i * 3) % 80).expect("byte"),
                255,
            ]
        });
        let bright = frame_of(|i| {
            [
                u8::try_from(120 + (i * 5) % 120).expect("byte"),
                u8::try_from(100 + (i * 11) % 100).expect("byte"),
                u8::try_from(80 + (i * 3) % 80).expect("byte"),
                255,
            ]
        });
        let compared = compare_frames(&dark, &bright).expect("different frames compare");
        assert!(compared.ssim_x10000 < 10000, "{}", compared.ssim_x10000);
        assert!(compared.flip_x10000 > 0, "{}", compared.flip_x10000);
    }

    #[test]
    fn view_compare_scores_the_second_metric_below_identical() {
        let left = frame_of(|i| {
            [
                u8::try_from((i * 5) % 120).expect("byte"),
                u8::try_from((i * 11) % 100).expect("byte"),
                u8::try_from((i * 3) % 80).expect("byte"),
                255,
            ]
        });
        let right = frame_of(|i| {
            [
                u8::try_from(120 + (i * 5) % 120).expect("byte"),
                u8::try_from(100 + (i * 11) % 100).expect("byte"),
                u8::try_from(80 + (i * 3) % 80).expect("byte"),
                255,
            ]
        });
        let compared = compare_frames(&left, &right).expect("different frames compare");
        let second = compared
            .ssimulacra2_x10000
            .expect("the donor scores two same-size frames");
        assert!(second < 10000, "{second}");
        assert!(second > 0, "{second}");
    }

    #[test]
    fn view_compare_refuses_escapes_and_non_png_names() {
        for bad in [
            "../up.png",
            "/abs.png",
            "C:\\x.png",
            "a\\b.png",
            "a/../../b.png",
            "target/captures/frame.jpg",
            "",
        ] {
            assert_eq!(check_png("left", bad).expect_err(bad).code, "bad_args");
        }
        for ok in [
            "target/captures/frame.png",
            "content/reference/native/key.png",
            "./target/x.PNG",
        ] {
            assert!(check_png("left", ok).is_ok(), "{ok}");
        }
    }

    #[test]
    fn view_compare_registers_read_only_in_the_view_pack() {
        let registry = PackRegistry::shipped();
        let verb = registry
            .verb(TOOL_VIEW_COMPARE)
            .expect("view.compare registers");
        assert_eq!(verb.pack(), PACK);
        assert_eq!(verb.card(), CapabilityCard::Observe);
        assert!(verb.readonly());
        assert!(registry
            .pack(PACK)
            .expect("view pack")
            .verb_names()
            .contains(&TOOL_VIEW_COMPARE));
    }

    /// A project root of this test: both inputs and the diff image live under
    /// it, so the verb never touches the repo tree.
    fn compare_root(name: &str) -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("e2040-view-compare-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("compare root creates");
        root
    }

    /// Write an RGB8 PNG under `root`.
    fn write_png(root: &Path, name: &str, width: u32, height: u32, rgb: &[u8]) {
        std::fs::write(root.join(name), encode_png_rgb(width, height, rgb)).expect("png writes");
    }

    /// A 4x4 RGB pair with known gaps: red rises by one, green falls by two
    /// and blue rises by five per pixel, so the amplified blue saturates.
    fn known_rgb_pair() -> (Vec<u8>, Vec<u8>) {
        let mut left = Vec::with_capacity(4 * 4 * 3);
        let mut right = Vec::with_capacity(4 * 4 * 3);
        for i in 0..16u8 {
            left.extend_from_slice(&[i * 10, 60, 0]);
            right.extend_from_slice(&[i * 10 + 1, 60 - i * 2, i * 5]);
        }
        (left, right)
    }

    #[test]
    fn view_compare_writes_the_exact_amplified_diff() {
        let root = compare_root("exact");
        let (left_rgb, right_rgb) = known_rgb_pair();
        write_png(&root, "left.png", 4, 4, &left_rgb);
        write_png(&root, "right.png", 4, 4, &right_rgb);
        // The scores need one whole 8x8 SSIM window, so the 4x4 pixel oracle
        // runs on the decoded frames and the diff writer.
        let left = read_frame("left", &root, "left.png").expect("left decodes");
        let right = read_frame("right", &root, "right.png").expect("right decodes");
        write_diff_png(&root, "reports/diff.png", &left, &right).expect("diff writes");
        let bytes = std::fs::read(root.join("reports/diff.png")).expect("diff reads back");
        let diff = Frame::decode_png(&bytes).expect("diff decodes");
        assert_eq!(
            (diff.width(), diff.height()),
            (4, 4),
            "the diff keeps the pair's size"
        );
        // min(255, gap * 8): red 1 -> 8, green 2i -> 16i, blue 5i -> 40i.
        for i in 0..16usize {
            let at = u8::try_from(i).expect("pixel index");
            let blue = u8::try_from(u32::from(at) * 40).unwrap_or(u8::MAX);
            assert_eq!(
                diff.rgb_at(i),
                [8, at * 16, blue],
                "diff pixel {i} of the known 4x4 pair"
            );
        }
        assert_eq!(diff.rgb_at(0), [8, 0, 0], "a one-step red gap reads 8");
        assert_eq!(diff.rgb_at(6), [8, 96, 240], "a 30-step blue gap reads 240");
        assert_eq!(
            diff.rgb_at(7),
            [8, 112, 255],
            "a 35-step blue gap saturates"
        );
    }

    #[test]
    fn view_compare_identical_frames_write_an_all_zero_diff() {
        let root = compare_root("identical");
        let mut rgb = Vec::with_capacity(8 * 8 * 3);
        for i in 0..64u8 {
            rgb.extend_from_slice(&[i, 255 - i, 128]);
        }
        write_png(&root, "both.png", 8, 8, &rgb);
        let plain = run_view_compare(
            &root,
            &ViewCompareArgs {
                ignore: None,
                left: "both.png".to_owned(),
                right: "both.png".to_owned(),
                diff: None,
                region: None,
            },
        )
        .expect("a pair without a diff compares");
        assert_eq!(plain.diff_png, None, "no diff path, no written path");
        assert!(
            !root.join("reports").exists(),
            "no diff path writes nothing"
        );
        let out = run_view_compare(
            &root,
            &ViewCompareArgs {
                ignore: None,
                left: "both.png".to_owned(),
                right: "both.png".to_owned(),
                diff: Some("reports/zero.png".to_owned()),
                region: None,
            },
        )
        .expect("an identical pair compares");
        assert_eq!(out.diff_png.as_deref(), Some("reports/zero.png"));
        assert_eq!(out.width, 8);
        assert_eq!(out.height, 8);
        assert_eq!(out.ssim_x10000, 10000);
        assert_eq!(out.flip_x10000, 0);
        let bytes = std::fs::read(root.join("reports/zero.png")).expect("zero diff reads");
        let diff = Frame::decode_png(&bytes).expect("zero diff decodes");
        assert_eq!(
            (diff.width(), diff.height()),
            (8, 8),
            "the diff keeps the pair's size"
        );
        for at in 0..diff.pixels() {
            assert_eq!(diff.rgb_at(at), [0, 0, 0], "identical pixel {at}");
        }
    }

    #[test]
    fn view_compare_mismatched_sizes_keep_the_error_and_write_no_diff() {
        let root = compare_root("mismatch");
        let (left_rgb, _) = known_rgb_pair();
        write_png(&root, "small.png", 4, 4, &left_rgb);
        write_png(&root, "large.png", 8, 8, &[127u8; 8 * 8 * 3]);
        let error = run_view_compare(
            &root,
            &ViewCompareArgs {
                ignore: None,
                left: "small.png".to_owned(),
                right: "large.png".to_owned(),
                diff: Some("reports/none.png".to_owned()),
                region: None,
            },
        )
        .expect_err("a mismatched pair refuses");
        assert_eq!(error.code, "bad_image", "{}", error.message);
        assert!(
            error.message.contains("do not compare"),
            "{}",
            error.message
        );
        assert!(
            !root.join("reports/none.png").exists(),
            "a refused pair writes no diff"
        );
        assert!(
            !root.join("reports").exists(),
            "a refused pair creates no diff folder"
        );
    }

    #[test]
    fn view_compare_refuses_bad_diff_paths_and_writes_nothing() {
        let root = compare_root("bad-diff");
        write_png(&root, "a.png", 8, 8, &[200u8; 8 * 8 * 3]);
        let escape_name = format!("../e2040-view-compare-{}-escape.png", std::process::id());
        let escape = std::env::temp_dir().join(format!(
            "e2040-view-compare-{}-escape.png",
            std::process::id()
        ));
        for bad in [
            escape_name.as_str(),
            "/abs.png",
            "C:\\x.png",
            "a\\b.png",
            "a/../../b.png",
            "reports/frame.jpg",
            "",
        ] {
            let error = run_view_compare(
                &root,
                &ViewCompareArgs {
                    ignore: None,
                    left: "a.png".to_owned(),
                    right: "a.png".to_owned(),
                    diff: Some(bad.to_owned()),
                    region: None,
                },
            )
            .expect_err(bad);
            assert_eq!(error.code, "bad_args", "{bad}: {}", error.message);
            assert!(error.message.contains("diff"), "{bad}: {}", error.message);
        }
        assert!(!escape.exists(), "an escaping diff path writes nothing");
        assert!(
            !root.join("reports").exists(),
            "a refused diff path writes nothing"
        );
    }

    #[test]
    fn view_compare_names_the_path_when_the_diff_cannot_be_written() {
        let root = compare_root("write-fail");
        let mut rgb = Vec::with_capacity(8 * 8 * 3);
        for i in 0..64u8 {
            rgb.extend_from_slice(&[i, 60, 200]);
        }
        write_png(&root, "a.png", 8, 8, &rgb);
        std::fs::write(root.join("blocked"), b"a file, not a folder").expect("blocker writes");
        let error = run_view_compare(
            &root,
            &ViewCompareArgs {
                ignore: None,
                left: "a.png".to_owned(),
                right: "a.png".to_owned(),
                diff: Some("blocked/diff.png".to_owned()),
                region: None,
            },
        )
        .expect_err("a file blocks the diff folder");
        assert_eq!(error.code, "write_failed", "{}", error.message);
        assert!(
            error.message.contains("blocked/diff.png"),
            "the refusal names the path: {}",
            error.message
        );
        assert!(error.message.contains("re-run"), "{}", error.message);
    }

    #[test]
    fn door_status_reports_the_session_and_a_digest_that_repeats() {
        let server = server();
        let first = DoorStatusTool::invoke(&server, DoorStatusArgs {}).expect("first status");
        assert_eq!(first.session, 7);
        assert_eq!(first.revision, 0);
        assert!(
            first.packs.iter().any(|p| p == "scene"),
            "{:?}",
            first.packs
        );
        assert!(!first.packs.iter().any(|p| p == "view"));
        assert_eq!(
            first.budget_chars,
            u64::try_from(MAX_ADVERTISED_CHARS).unwrap()
        );
        assert_eq!(
            first.headroom_chars,
            first.budget_chars - first.surface_chars
        );
        assert!(first.surface_chars > 0);
        assert!(first.tools > 0);
        assert_eq!(first.audit_head, None, "the first call sees an empty chain");
        assert_eq!(first.audit_total, 0);
        assert_eq!(first.durable_generation, None, "unknown stays unknown");
        assert_eq!(first.build_sha, None, "no build stamp in a library test");
        let second = DoorStatusTool::invoke(&server, DoorStatusArgs {}).expect("second status");
        assert_eq!(
            second.digest, first.digest,
            "the stable fields digest the same twice"
        );
        assert_eq!(second.surface_chars, first.surface_chars);
        assert!(
            second.audit_head.is_some(),
            "the first call's record is visible"
        );
        assert_eq!(second.audit_total, 1);
    }

    #[test]
    fn door_status_registers_read_only_and_the_view_pack_fits_the_budget() {
        let registry = PackRegistry::shipped();
        let verb = registry
            .verb(TOOL_DOOR_STATUS)
            .expect("door.status registers");
        assert_eq!(verb.pack(), PACK);
        assert_eq!(verb.card(), CapabilityCard::Observe);
        assert!(verb.readonly());
        assert!(registry
            .pack(PACK)
            .expect("view pack")
            .verb_names()
            .contains(&TOOL_DOOR_STATUS));
        let described = ApiDescribeTool::invoke(
            &server(),
            ApiDescribeArgs {
                path: TOOL_DOOR_STATUS.to_owned(),
            },
        )
        .expect("api.describe reads door.status");
        assert_eq!(described.path, TOOL_DOOR_STATUS);
        assert!(described.docs.contains("audit"));

        let server = server();
        ApiLoadTool::invoke(
            &server,
            ApiLoadArgs {
                pack: PACK.to_owned(),
                unload: None,
            },
        )
        .expect("the view pack fits under the budget");
        let status = DoorStatusTool::invoke(&server, DoorStatusArgs {}).expect("status after load");
        assert!(status.packs.iter().any(|p| p == PACK));
        assert!(
            status.surface_chars <= status.budget_chars,
            "surface {} exceeds budget {}",
            status.surface_chars,
            status.budget_chars
        );
        assert!(status.headroom_chars > 0);
    }

    // -----------------------------------------------------------------------
    // region compare and the observe marks
    // -----------------------------------------------------------------------

    /// Sand over the whole frame, then a darker band below `split`.
    fn banded(split: usize, band: [u8; 4]) -> Frame {
        frame_of(|at| {
            if at / 16 >= split {
                band
            } else {
                [194, 160, 104, 255]
            }
        })
    }

    #[test]
    fn view_compare_rect_scores_only_the_pixels_inside_it() {
        let reference = banded(8, [194, 160, 104, 255]);
        let capture = banded(8, [60, 50, 30, 255]);
        let rect = CompareRegion::Rect {
            x: 0,
            y: 0,
            w: 16,
            h: 8,
        };
        let (region, scoring) = resolve_region(Some(rect), &reference).expect("rect resolves");
        let inside =
            compare_frames_in(&reference, &capture, region, scoring).expect("inside scores");
        assert_eq!(
            (inside.region.as_str(), inside.rect),
            ("rect", Some([0, 0, 16, 8]))
        );
        // The band starts at row 8, so the scored rows are identical.
        assert_eq!(inside.ssim_x10000, metric::SSIM_ONE);
        assert_eq!(inside.mean_de_x100, 0);
        let (region, scoring) = resolve_region(None, &reference).expect("full resolves");
        let full = compare_frames_in(&reference, &capture, region, scoring).expect("full scores");
        assert_eq!((full.region.as_str(), full.rect), ("full", None));
        assert!(full.ssim_x10000 < metric::SSIM_ONE);
        assert!(full.mean_de_x100 > 0);
    }

    #[test]
    fn view_compare_bad_rect_names_the_legal_kinds_and_a_recovery() {
        let frame = frame_of(|_| [10, 20, 30, 255]);
        for bad in [
            CompareRegion::Rect {
                x: 0,
                y: 0,
                w: 0,
                h: 8,
            },
            CompareRegion::Rect {
                x: 8,
                y: 0,
                w: 16,
                h: 8,
            },
            CompareRegion::Rect {
                x: 0,
                y: 9,
                w: 16,
                h: 8,
            },
        ] {
            let error = resolve_region(Some(bad), &frame).expect_err("bad rect");
            assert_eq!(error.code, "bad_args");
            assert!(
                error.message.contains("\"kind\":\"full\""),
                "{}",
                error.message
            );
            assert!(
                error.message.contains("\"kind\":\"rect\""),
                "{}",
                error.message
            );
            assert!(error.message.contains("re-run"), "{}", error.message);
        }
        // A legal rect reaches the frame's last pixel.
        assert!(resolve_region(
            Some(CompareRegion::Rect {
                x: 8,
                y: 8,
                w: 8,
                h: 8
            }),
            &frame
        )
        .is_ok());
    }

    #[test]
    fn view_observe_marks_are_one_per_visible_body_and_stable() {
        let body = |slot: u8, screen: Option<[f32; 4]>| ViewBody {
            slot,
            act: "idle".to_owned(),
            clip: 0,
            tint: false,
            root: [0.0, 0.0, 0.0],
            min: None,
            max: None,
            screen,
            cover: 0.5,
        };
        let bodies = vec![
            body(0, Some([-0.5, -0.5, 0.0, 0.0])),
            body(1, Some([0.0, 0.0, 0.5, 0.5])),
            body(2, None),
        ];
        let first = marks_of(&bodies);
        let second = marks_of(&bodies);
        assert_eq!(first, second);
        assert_eq!(first.len(), 2);
        assert_eq!(first[0].id, 0);
        assert_eq!(first[1].id, 1);
        assert_eq!(first[1].label, "body_1");
        assert_eq!(first[1].rect, [0.0, 0.0, 0.5, 0.5]);
    }
}
