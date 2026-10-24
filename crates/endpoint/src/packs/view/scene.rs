//! The view pack's scene lane (rows SP62, EG35): the spatial scene report an
//! agent reads, and the render steer that lets it look again.
//!
//! `view.scene` returns one report of the live scene: every body, prop and
//! spear with its place, bounds, screen rect and cover, floor contact, the
//! hand that holds what, the bone a spear is stuck in, what hides it from the
//! camera, whether the sun reaches it and where its shadow falls, the camera
//! pose, the renderer's pass switches and the view laws, as one compact JSON
//! value plus a readable text form and a short findings list. It reads a live
//! window through the agent seat (`seat.open` first) or, with no seat, plays
//! the session's bout in a windowless `arena.exe --scene-report` and reports
//! its last frame. `view.steer` changes how the live window draws and looks
//! again: pin the camera to a named shot or a pose, switch one render pass,
//! take a capture, freeze the sim on its tick; each change but a capture is
//! undoable.
//!
//! The door owns no view and links no renderer: the window or the arena
//! subprocess builds the report (`engine_game::scene_report`) and the door
//! checks it against the contract before it passes it on (schema 1, a camera,
//! things with finite ordered rects and shares inside `0..=1`, one fighter per
//! slot) and refuses `bad_view` otherwise, never a partial success. Units:
//! meters, degrees, centimeters for contact gaps, picture share `0..=1`, fixed
//! ticks. Determinism: the door adds nothing to the host's bytes; one bout
//! reported twice gives the same bytes. Errors: [`McpToolError`] with
//! `no_live_bout`, `no_seat`, `seat_closed`, `seat_timeout`,
//! `arena_exe_missing`, `arena_view_failed`, `bad_view`, `bad_args` or the
//! window's own refusal text under `bad_args`. Field docs on the transport
//! DTOs are omitted on purpose, as in `packs::view`: every doc line inflates
//! the advertised schemas, and the discovery budget is a gate.

use std::path::Path;
use std::process::Command;

use rmcp::schemars::JsonSchema;
use rmcp::serde::{Deserialize, Serialize};
use rmcp::serde_json::{self, Map, Value};

use super::{
    arena_exe_path, bad_args, bad_view, no_live_bout, ViewBody, ViewCamera, ViewFrame,
    ARENA_EXE_ENV,
};
use crate::mcp::{McpServer, McpToolError};
use crate::play::play_session::PlaySlot;
use crate::tool_capabilities::CheckKind;

/// Verb: the live spatial scene report.
pub const TOOL_VIEW_SCENE: &str = "view.scene";

/// Verb: steer the live window's render and look again.
pub const TOOL_VIEW_STEER: &str = "view.steer";

/// Prefix `arena.exe --scene-report` prints before the JSON line, so a boot or
/// warning line can never be mistaken for the answer.
pub const SCENE_REPORT_PREFIX: &str = "arena scene-report: ";

/// Version of the report shape the door accepts.
pub const SCENE_SCHEMA: u64 = 1;

/// Steer operations, in the order the description lists them.
pub const STEER_OPS: [&str; 9] = [
    "shot", "pose", "pass", "capture", "hold", "resume", "step", "state", "undo",
];

/// Longest name a steer argument may carry, chars.
const MAX_NAME_CHARS: usize = 32;

// ---------------------------------------------------------------------------
// view.scene
// ---------------------------------------------------------------------------

/// Args for `view.scene`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct ViewSceneArgs {
    pub source: Option<String>,
    pub format: Option<String>,
    pub pixels: Option<bool>,
    pub showcase: Option<bool>,
    pub shot: Option<String>,
}

/// `view.scene` result: where the report came from, the tick it describes, the
/// findings, and the report and its text as asked.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct ViewSceneOutput {
    pub source: String,
    pub tick: u32,
    pub seed: String,
    pub check: String,
    pub findings: Vec<String>,
    pub text: Option<String>,
    pub report: Option<Value>,
}

/// The windowless host: `(seed, ticks, fighters, showcase, shot)` in, the
/// report's JSON line out.
pub type SceneBoutFn = fn(
    seed: u64,
    ticks: u32,
    fighters: u8,
    showcase: bool,
    shot: Option<&str>,
) -> Result<String, McpToolError>;

/// The live host: `pixels` in, the window's report JSON out.
pub type SceneSeatFn = fn(pixels: bool) -> Result<String, McpToolError>;

/// How the report is asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Source {
    Auto,
    Seat,
    Bout,
}

/// What the answer carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Format {
    Both,
    Json,
    Text,
}

fn source_of(args: &ViewSceneArgs) -> Result<Source, McpToolError> {
    match args.source.as_deref() {
        None | Some("auto") => Ok(Source::Auto),
        Some("seat") => Ok(Source::Seat),
        Some("bout") => Ok(Source::Bout),
        Some(other) => Err(bad_args(format!(
            "source `{other}` is not auto, seat or bout; re-run with one of them"
        ))),
    }
}

fn format_of(args: &ViewSceneArgs) -> Result<Format, McpToolError> {
    match args.format.as_deref() {
        None | Some("both") => Ok(Format::Both),
        Some("json") => Ok(Format::Json),
        Some("text") => Ok(Format::Text),
        Some(other) => Err(bad_args(format!(
            "format `{other}` is not both, json or text; re-run with one of them"
        ))),
    }
}

/// The arguments that only one source takes, checked against the source.
fn check_extras(args: &ViewSceneArgs, source: Source) -> Result<(), McpToolError> {
    if let Some(shot) = args.shot.as_deref() {
        if !matches!(shot, "hero" | "wide" | "action") {
            return Err(bad_args(format!(
                "shot `{shot}` is not hero, wide or action (the director's shots; steer the live window's camera with view.steer)"
            )));
        }
    }
    let bout_only = args.shot.is_some() || args.showcase == Some(true);
    if source == Source::Seat && bout_only {
        return Err(bad_args(
            "shot and showcase shape a windowless bout; a live seat reports the window as it is \
             (steer its camera with view.steer)"
                .to_owned(),
        ));
    }
    if source == Source::Bout && args.pixels == Some(true) {
        return Err(bad_args(
            "pixels reads the live renderer's id buffer; a windowless bout has none, use source seat"
                .to_owned(),
        ));
    }
    Ok(())
}

/// A number field that must be a finite JSON number.
fn finite_at(value: &Value, what: &str) -> Result<f64, McpToolError> {
    value
        .as_f64()
        .filter(|v| v.is_finite())
        .ok_or_else(|| bad_view(format!("{what} is not a finite number")))
}

/// A rect `[x0, y0, x1, y1]` that is finite, ordered and inside the picture.
fn check_rect(rect: &Value, what: &str) -> Result<[f64; 4], McpToolError> {
    let cells = rect
        .as_array()
        .filter(|a| a.len() == 4)
        .ok_or_else(|| bad_view(format!("{what} is not a four-number rect")))?;
    let mut out = [0.0; 4];
    for (slot, cell) in out.iter_mut().zip(cells) {
        *slot = finite_at(cell, what)?;
        if slot.abs() > 1.0 {
            return Err(bad_view(format!("{what} {rect} leaves -1..=1")));
        }
    }
    if out[0] >= out[2] || out[1] >= out[3] {
        return Err(bad_view(format!("{what} {rect} is not ordered")));
    }
    Ok(out)
}

/// A share field inside `0..=1`.
fn check_share(value: &Value, what: &str) -> Result<(), McpToolError> {
    let share = finite_at(value, what)?;
    if (0.0..=1.0).contains(&share) {
        Ok(())
    } else {
        Err(bad_view(format!("{what} {share} is not in 0..=1")))
    }
}

/// One thing against the contract: an id, a kind, a screen block that is null
/// or a finite ordered rect with a cover share, visibility shares in `0..=1`.
fn check_thing(thing: &Value) -> Result<(), McpToolError> {
    let id = thing
        .get("id")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
        .ok_or_else(|| bad_view("a thing has no id"))?;
    let kind = thing.get("kind").and_then(Value::as_str).unwrap_or("");
    if !matches!(kind, "fighter" | "prop" | "spear") {
        return Err(bad_view(format!("{id} has kind `{kind}`")));
    }
    match thing.get("screen") {
        Some(Value::Null) => {}
        Some(screen) => {
            check_rect(
                screen.get("ndc").unwrap_or(&Value::Null),
                &format!("{id} screen"),
            )?;
            check_share(
                screen.get("cover").unwrap_or(&Value::Null),
                &format!("{id} cover"),
            )?;
        }
        None => return Err(bad_view(format!("{id} has no screen field"))),
    }
    let vis = thing.get("vis").unwrap_or(&Value::Null);
    for key in ["visible", "hidden", "outside"] {
        check_share(
            vis.get(key).unwrap_or(&Value::Null),
            &format!("{id} vis.{key}"),
        )?;
    }
    Ok(())
}

/// The report against the contract, and against the bout when the door knows
/// it: schema, camera, laws, findings, text, every thing, one fighter per slot
/// (every live fighter once the bout has ticked).
fn check_report(report: &Value, bout: Option<(u32, u8)>) -> Result<(), McpToolError> {
    let top = report
        .as_object()
        .ok_or_else(|| bad_view("the report is not a JSON object"))?;
    if top.get("schema").and_then(Value::as_u64) != Some(SCENE_SCHEMA) {
        return Err(bad_view(format!("the report schema is not {SCENE_SCHEMA}")));
    }
    for key in ["camera", "light", "laws"] {
        if !top.get(key).is_some_and(Value::is_object) {
            return Err(bad_view(format!("the report has no `{key}` object")));
        }
    }
    for key in ["things", "findings", "text"] {
        if !top.get(key).is_some_and(Value::is_array) {
            return Err(bad_view(format!("the report has no `{key}` list")));
        }
    }
    for key in ["eye", "target"] {
        let v = report["camera"].get(key).and_then(Value::as_array);
        if !v.is_some_and(|v| v.len() == 3 && v.iter().all(|c| c.as_f64().is_some())) {
            return Err(bad_view(format!(
                "camera.{key} is not three finite numbers"
            )));
        }
    }
    let things = report["things"].as_array().map_or(&[][..], Vec::as_slice);
    let mut slots = Vec::new();
    for thing in things {
        check_thing(thing)?;
        if thing["kind"] == "fighter" {
            let slot = thing["slot"]
                .as_u64()
                .ok_or_else(|| bad_view("a fighter has no slot"))?;
            if slots.contains(&slot) {
                return Err(bad_view(format!("fighter slot {slot} appears twice")));
            }
            slots.push(slot);
        }
    }
    if let Some((ticks, participants)) = bout {
        if report["tick"].as_u64() != Some(u64::from(ticks)) {
            return Err(bad_view(format!(
                "the report tick {} is not the live bout's {ticks}",
                report["tick"]
            )));
        }
        if ticks > 0 {
            if let Some(missing) = (0..u64::from(participants)).find(|s| !slots.contains(s)) {
                return Err(bad_view(format!(
                    "fighter slot {missing} is missing; the host must name every live fighter once"
                )));
            }
        }
    }
    Ok(())
}

/// Parse the host's JSON text and check it.
fn parse_report(raw: &str, bout: Option<(u32, u8)>) -> Result<Value, McpToolError> {
    let report: Value = serde_json::from_str(raw)
        .map_err(|err| bad_view(format!("the report is not JSON ({err})")))?;
    check_report(&report, bout)?;
    Ok(report)
}

/// The findings list as strings.
fn findings_of(report: &Value) -> Vec<String> {
    report["findings"].as_array().map_or_else(Vec::new, |list| {
        list.iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect()
    })
}

/// The text lines joined by newlines.
fn text_of(report: &Value) -> String {
    report["text"].as_array().map_or_else(String::new, |list| {
        list.iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join("\n")
    })
}

/// Shape the checked report into the answer.
fn answer(
    source: &str,
    raw: &str,
    format: Format,
    bout: Option<(u32, u8)>,
) -> Result<ViewSceneOutput, McpToolError> {
    let mut report = parse_report(raw, bout)?;
    let tick = report["tick"]
        .as_u64()
        .and_then(|t| u32::try_from(t).ok())
        .unwrap_or(0);
    let seed = report["seed"].as_str().unwrap_or("0").to_owned();
    let findings = findings_of(&report);
    let text = text_of(&report);
    if let Some(map) = report.as_object_mut() {
        map.remove("text");
    }
    Ok(ViewSceneOutput {
        source: source.to_owned(),
        tick,
        seed,
        check: CheckKind::ReadOnly.name().to_owned(),
        findings,
        text: (format != Format::Json).then_some(text),
        report: (format != Format::Text).then_some(report),
    })
}

/// Pure scene core: one report from the live seat or the session's bout. Steps
/// nothing and writes nothing.
fn scene_core(
    slot: &PlaySlot,
    args: &ViewSceneArgs,
    seat_open: bool,
    seat: SceneSeatFn,
    bout_host: SceneBoutFn,
) -> Result<ViewSceneOutput, McpToolError> {
    let format = format_of(args)?;
    let wanted = source_of(args)?;
    let source = match wanted {
        Source::Auto if seat_open => Source::Seat,
        Source::Auto => Source::Bout,
        other => other,
    };
    check_extras(args, source)?;
    if source == Source::Seat {
        let raw = seat(args.pixels == Some(true))?;
        return answer("seat", &raw, format, None);
    }
    let bout = slot.current().ok_or_else(no_live_bout)?;
    let spec = bout.spec();
    let ticks = bout.observe(0).tick;
    let showcase = args.showcase == Some(true);
    let fighters = if showcase { 2 } else { spec.participants };
    let raw = bout_host(spec.seed, ticks, fighters, showcase, args.shot.as_deref())?;
    answer("bout", &raw, format, Some((ticks, fighters)))
}

fn view_scene(service: &McpServer, args: &ViewSceneArgs) -> Result<ViewSceneOutput, McpToolError> {
    service.dispatch(TOOL_VIEW_SCENE, |state| {
        scene_core(
            &state.play,
            args,
            crate::packs::play::seat_is_open(),
            seat_scene,
            arena_scene_report,
        )
    })
}

/// The live window's report: `scene` or `scene pixels` on the session's seat.
fn seat_scene(pixels: bool) -> Result<String, McpToolError> {
    let line = if pixels { "scene pixels" } else { "scene" };
    crate::packs::play::seat_ask(line, "scene ")
}

// ---------------------------------------------------------------------------
// The arena subprocess host: `arena.exe --scene-report` plays the same bout
// headless, prints one prefixed line and exits.
// ---------------------------------------------------------------------------

/// The report in `stdout`, when the arena printed the prefixed line.
fn report_from_stdout(stdout: &str) -> Option<String> {
    stdout.lines().find_map(|line| {
        line.strip_prefix(SCENE_REPORT_PREFIX)
            .map(|json| json.trim_end_matches('\r').to_owned())
    })
}

/// The last non-empty line of a child's stderr, for a loud refusal.
fn error_tail(stderr: &[u8]) -> String {
    let text = String::from_utf8_lossy(stderr);
    text.lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .map_or_else(|| "no stderr".to_owned(), |line| line.trim().to_owned())
}

/// `view.scene` served by a windowless `arena.exe`: spawn the shipped binary
/// with `--scene-report <seed> <ticks> <fighters>` (plus `--showcase` and
/// `--shot`), read its one prefixed line and pass it on unchanged. The path is
/// [`ARENA_EXE_ENV`] or the shipped default, relative to the door's working
/// directory. The bout folds with no scripted input, as `view.laws` does.
///
/// # Errors
///
/// `arena_exe_missing` names the path tried when no binary is there,
/// `arena_view_failed` carries the exit status plus the child's stderr tail,
/// and `bad_view` when the child printed no `arena scene-report:` line.
pub fn arena_scene_report(
    seed: u64,
    ticks: u32,
    fighters: u8,
    showcase: bool,
    shot: Option<&str>,
) -> Result<String, McpToolError> {
    arena_scene_report_from(&arena_exe_path(), seed, ticks, fighters, showcase, shot)
}

/// [`arena_scene_report`] over an explicit binary path: the test seam, and the
/// one owner of the spawn, the exit check and the line parse.
fn arena_scene_report_from(
    exe: &Path,
    seed: u64,
    ticks: u32,
    fighters: u8,
    showcase: bool,
    shot: Option<&str>,
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
    let mut command = Command::new(exe);
    command
        .arg("--scene-report")
        .arg(seed.to_string())
        .arg(ticks.to_string())
        .arg(fighters.to_string());
    if showcase {
        command.arg("--showcase");
    }
    if let Some(shot) = shot {
        command.arg("--shot").arg(shot);
    }
    let output = command.output().map_err(|err| {
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
    report_from_stdout(&String::from_utf8_lossy(&output.stdout)).ok_or_else(|| {
        bad_view(format!(
            "`{}` printed no `{SCENE_REPORT_PREFIX}` line; re-run with a built arena.exe",
            exe.display()
        ))
    })
}

// ---------------------------------------------------------------------------
// view.observe from the scene report: the shipped door's view host (SP62).
// ---------------------------------------------------------------------------

/// One finite `f32` out of a JSON number.
fn f32_of(value: &Value) -> Option<f32> {
    serde_json::from_value::<f32>(value.clone())
        .ok()
        .filter(|v| v.is_finite())
}

/// `N` finite numbers.
fn nums_of<const N: usize>(value: &Value) -> Option<[f32; N]> {
    let list = value.as_array().filter(|a| a.len() == N)?;
    let mut out = [0.0f32; N];
    for (slot, cell) in out.iter_mut().zip(list) {
        *slot = f32_of(cell)?;
    }
    Some(out)
}

/// One fighter of the report as the `view.observe` body.
fn body_of(thing: &Value) -> Result<ViewBody, McpToolError> {
    let slot = thing["slot"]
        .as_u64()
        .and_then(|s| u8::try_from(s).ok())
        .ok_or_else(|| bad_view("a fighter has no slot"))?;
    let screen = thing["screen"].get("ndc").and_then(nums_of::<4>);
    let cover = thing["screen"].get("cover").and_then(f32_of).unwrap_or(0.0);
    Ok(ViewBody {
        slot,
        act: thing["act"].as_str().unwrap_or("").to_owned(),
        clip: thing["clip"]
            .as_u64()
            .and_then(|c| u8::try_from(c).ok())
            .unwrap_or(0),
        tint: thing["tint"].as_bool().unwrap_or(false),
        root: nums_of::<3>(&thing["root"]).ok_or_else(|| bad_view("a fighter has no root"))?,
        min: nums_of::<3>(&thing["min"]),
        max: nums_of::<3>(&thing["max"]),
        screen,
        cover,
    })
}

/// The `view.observe` frame of a checked report.
fn frame_of(report: &Value, ticks: u32) -> Result<ViewFrame, McpToolError> {
    let camera = &report["camera"];
    let things = report["things"].as_array().map_or(&[][..], Vec::as_slice);
    let mut bodies = Vec::new();
    for thing in things.iter().filter(|t| t["kind"] == "fighter") {
        bodies.push(body_of(thing)?);
    }
    bodies.sort_by_key(|b| b.slot);
    Ok(ViewFrame {
        tick: ticks,
        camera: ViewCamera {
            eye: nums_of::<3>(&camera["eye"]).ok_or_else(|| bad_view("camera.eye"))?,
            target: nums_of::<3>(&camera["target"]).ok_or_else(|| bad_view("camera.target"))?,
            fov_deg: f32_of(&camera["fov_deg"]).unwrap_or(50.0),
        },
        bodies,
        laws: report["laws"]["line"].as_str().unwrap_or("").to_owned(),
    })
}

/// The view host over an explicit bout host: the test seam of
/// [`scene_view_frame`].
fn scene_view_frame_with(
    bout_host: SceneBoutFn,
    seed: u64,
    ticks: u32,
    fighters: u8,
) -> Result<ViewFrame, McpToolError> {
    let raw = bout_host(seed, ticks, fighters, false, None)?;
    let report = parse_report(&raw, Some((ticks, fighters)))?;
    frame_of(&report, ticks)
}

/// The shipped door's `view.observe` host: plays the bout in a windowless
/// `arena.exe --scene-report` and maps its fighters to the frame contract.
/// Register it once with [`super::register_view_source`] at start.
///
/// # Errors
///
/// The errors of [`arena_scene_report`], and `bad_view` when the report breaks
/// the contract.
pub fn scene_view_frame(seed: u64, ticks: u32, fighters: u8) -> Result<ViewFrame, McpToolError> {
    scene_view_frame_with(arena_scene_report, seed, ticks, fighters)
}

// ---------------------------------------------------------------------------
// view.steer
// ---------------------------------------------------------------------------

/// Args for `view.steer`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct ViewSteerArgs {
    pub op: String,
    pub shot: Option<String>,
    pub name: Option<String>,
    pub on: Option<bool>,
    pub eye: Option<[f32; 3]>,
    pub target: Option<[f32; 3]>,
    pub fov_deg: Option<f32>,
    pub tag: Option<String>,
}

/// `view.steer` result: the operation, the tick it ran on and the window's
/// answer with the fixed fields (`ok`, `op`, `tick`) lifted out.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct ViewSteerOutput {
    pub op: String,
    pub tick: u32,
    pub result: Value,
}

/// A name of letters, digits and `_` `-` `:`, at most [`MAX_NAME_CHARS`].
fn word<'a>(field: Option<&'a str>, what: &str) -> Result<&'a str, McpToolError> {
    let text = field.ok_or_else(|| bad_args(format!("{what} is required for this op")))?;
    let ok = !text.is_empty()
        && text.chars().count() <= MAX_NAME_CHARS
        && text
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | ':'));
    if ok {
        Ok(text)
    } else {
        Err(bad_args(format!(
            "{what} `{text}` must be 1..={MAX_NAME_CHARS} of letters, digits, _ - and :"
        )))
    }
}

/// Three finite numbers as seat line words.
fn point_words(point: Option<[f32; 3]>, what: &str) -> Result<String, McpToolError> {
    let p = point.ok_or_else(|| bad_args(format!("{what} is required for pose")))?;
    if !p.iter().all(|v| v.is_finite()) {
        return Err(bad_args(format!("{what} is not finite")));
    }
    Ok(format!("{} {} {}", p[0], p[1], p[2]))
}

/// The seat line for one steer request. The window parses it again and is the
/// authority on ranges; the door refuses only what could break the line.
fn steer_line(args: &ViewSteerArgs) -> Result<String, McpToolError> {
    let op = args.op.as_str();
    let line = match op {
        "shot" => format!("steer shot {}", word(args.shot.as_deref(), "shot")?),
        "pose" => {
            let fov = args
                .fov_deg
                .filter(|f| f.is_finite())
                .map_or_else(String::new, |f| format!(" {f}"));
            format!(
                "steer pose {} {}{fov}",
                point_words(args.eye, "eye")?,
                point_words(args.target, "target")?
            )
        }
        "pass" => {
            let on = args
                .on
                .ok_or_else(|| bad_args("on is required for pass (true or false)".to_owned()))?;
            format!(
                "steer pass {} {}",
                word(args.name.as_deref(), "name")?,
                if on { "on" } else { "off" }
            )
        }
        "capture" => match args.tag.as_deref() {
            Some(tag) => format!("steer capture {}", word(Some(tag), "tag")?),
            None => "steer capture".to_owned(),
        },
        "hold" | "resume" | "step" | "state" | "undo" => format!("steer {op}"),
        other => {
            return Err(bad_args(format!(
                "op `{other}` is not one of {}; re-run with one of them",
                STEER_OPS.join(", ")
            )))
        }
    };
    Ok(line)
}

/// Map the window's answer to the output, or its refusal to a typed error.
fn steer_answer(op: &str, raw: &str) -> Result<ViewSteerOutput, McpToolError> {
    let reply: Value = serde_json::from_str(raw)
        .map_err(|err| bad_view(format!("the steer answer is not JSON ({err})")))?;
    let mut fields: Map<String, Value> = reply
        .as_object()
        .cloned()
        .ok_or_else(|| bad_view("the steer answer is not a JSON object"))?;
    if fields.get("ok") != Some(&Value::Bool(true)) {
        let code = fields
            .get("code")
            .and_then(Value::as_str)
            .unwrap_or("refused");
        let why = fields
            .get("why")
            .and_then(Value::as_str)
            .unwrap_or("no reason given");
        return Err(bad_args(format!(
            "the window refused `{op}` ({code}): {why}"
        )));
    }
    let tick = fields
        .get("tick")
        .and_then(Value::as_u64)
        .and_then(|t| u32::try_from(t).ok())
        .unwrap_or(0);
    for key in ["ok", "op", "tick"] {
        fields.remove(key);
    }
    Ok(ViewSteerOutput {
        op: op.to_owned(),
        tick,
        result: Value::Object(fields),
    })
}

/// Pure steer core over an explicit ask: build the line, send it, map the answer.
fn steer_core(
    args: &ViewSteerArgs,
    ask: fn(&str, &str) -> Result<String, McpToolError>,
) -> Result<ViewSteerOutput, McpToolError> {
    let line = steer_line(args)?;
    let raw = ask(&line, "steer ")?;
    steer_answer(&args.op, &raw)
}

fn view_steer(service: &McpServer, args: &ViewSteerArgs) -> Result<ViewSteerOutput, McpToolError> {
    service.dispatch(TOOL_VIEW_STEER, |_state| {
        steer_core(args, crate::packs::play::seat_ask)
    })
}

crate::e2_tool!(
    /// `view.scene` tool: the live spatial scene report. Read-only.
    ViewSceneTool,
    name = TOOL_VIEW_SCENE,
    description = "Spatial scene report of the live view as JSON plus text plus findings: every fighter, prop and spear with world place, bounds, screen rect and cover, floor contact, what each hand holds, the bone a spear is stuck in, what hides it from the camera, sun lit/shaded and shadow-map facts, the camera, render passes and the view laws. source auto: the live window if seat.open ran, else the session bout in a windowless arena (empty tape; showcase, shot hero|wide|action shape it). pixels (seat): add id-buffer pixel counts. format both|json|text. Writes nothing.",
    readonly = true,
    args = ViewSceneArgs,
    output = ViewSceneOutput,
    invoke = view_scene
);

crate::e2_tool!(
    /// `view.steer` tool: steer the live window's render and look again. Writes.
    ViewSteerTool,
    name = TOOL_VIEW_STEER,
    description = "Steer the live window (seat.open first), then re-read with view.scene. op shot: shot auto|hero|wide|action|top|side|low|front|sun|close:N. pose: eye, target, fov_deg. pass: name sun_shadow|contact_shadow|fog|bloom|auto_exposure|msaa|taa|debug_lines plus on. capture: tag; writes target/scene-captures/<tag>-t<tick>-<n>.png and returns path, fnv, per-pass ms. hold|resume|step: freeze, run, or run one tick. state, undo: shot, pass and hold undo newest first. Presentation only; no sim byte moves.",
    readonly = false,
    args = ViewSteerArgs,
    output = ViewSteerOutput,
    invoke = view_steer
);

#[cfg(test)]
mod tests;
