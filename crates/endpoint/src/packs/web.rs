//! `web` pack: build the browser shell, serve it, look at it, drive it (AIS2).
//!
//! The loop an AI runs: `build.web`, `serve.web`, `browser.capture`, read the receipt,
//! fix, again. Each verb shells out to the node tool that already does the work
//! (`tools/web-build.mjs`, `tools/web-browse.mjs`) through the one process seam the
//! endpoint has, [`crate::secret_launch::launch_one_child`] (one child, cleared
//! environment plus an allowlist, stdout and stderr captured). [`WebRunner`] is the
//! only thing the verbs call, so a test swaps in a fake runner and never starts a
//! process.
//!
//! Refused by default. The project allows the web verbs, not the AI: the host calls
//! [`McpServer::allow_web`] with a [`WebScope`] (project root, work folder, Chrome
//! path). Until then every verb answers the typed code `unscoped` and runs nothing.
//! Nothing a verb takes can name a program, a command line or a path outside the
//! work folder: args are small typed values with ranges.
//!
//! Undo. A build lands in its own folder `<work>/build-<n>` (nothing is overwritten);
//! `serve.web` points the next captures at one build; `browser.input` queues events
//! that replay from page load in every later capture. Each of these three takes
//! `undo: true`, which reverts the newest change of that verb (the build pointer, the
//! serve pointer, the last queued batch) and refuses with `nothing_to_undo` when the
//! newest change belongs to another verb. `browser.capture` changes no door state; it
//! writes a fresh `<work>/capture-<n>` folder.
//!
//! "Serve" is a pointer, not a daemon: the door call model is one synchronous call, and
//! `tools/web-browse.mjs` starts its own loopback-only static server (127.0.0.1, port
//! chosen by the OS) for every capture, so no port stays open between calls.
//!
//! Caveats, stated plainly. A call blocks until its child ends (a build takes minutes;
//! the browse tool bounds its own waits). The receipt line carries `tier=` and
//! `validation=` from the page's ready line when the page prints them, and `na` when it
//! does not yet (the web quality tier is WEB8). Page text (console lines) is data from
//! an untrusted page and is length-capped here.
//!
//! DTO structs carry no doc comments on purpose: every doc line inflates the discovery
//! schemas the door budgets.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use super::{bad_args as bad, PackDef, PackError, VerbSpec};
use crate::mcp::{McpServer, McpToolError};
use crate::secret_launch::{launch_one_child, LaunchSpec};
use crate::tool_capabilities::CapabilityCard;
use rmcp::schemars::JsonSchema;
use rmcp::serde::{Deserialize, Serialize};
use rmcp::serde_json::{self, Value};

/// Pack name.
pub const PACK: &str = "web";
/// Verb: build the browser shell into a fresh folder.
pub const TOOL_BUILD_WEB: &str = "build.web";
/// Verb: point the next captures at one build.
pub const TOOL_SERVE_WEB: &str = "serve.web";
/// Verb: headless Chrome loads the served build; PNG, console lines, receipt line.
pub const TOOL_BROWSER_CAPTURE: &str = "browser.capture";
/// Verb: queue scripted keyboard and mouse events for the next captures.
pub const TOOL_BROWSER_INPUT: &str = "browser.input";
/// Typed refusal code of a call made before the project allowed the web verbs.
pub const UNSCOPED: &str = "unscoped";

/// Most builds one session keeps (each is a folder under the work folder).
pub const MAX_BUILDS: usize = 32;
/// Most captures one session makes.
pub const MAX_CAPTURES: u32 = 256;
/// Most events one `browser.input` call may add.
pub const MAX_EVENTS_PER_CALL: usize = 64;
/// Most events queued for replay at once.
pub const MAX_QUEUED_EVENTS: usize = 256;
/// Longest key name (`ArrowLeft`, `Space`, `a`).
pub const MAX_KEY_CHARS: usize = 16;
/// Largest x in the 1280 by 720 page.
pub const MAX_X: i32 = 1279;
/// Largest y in the 1280 by 720 page.
pub const MAX_Y: i32 = 719;
/// Longest wait event in ms.
pub const MAX_WAIT_MS: u32 = 2000;
/// Longest build commit label.
pub const MAX_COMMIT_CHARS: usize = 40;
/// Most console lines a capture returns.
pub const MAX_CONSOLE_LINES: usize = 30;
/// Longest console line returned, chars.
pub const MAX_LINE_CHARS: usize = 300;
/// Most undo records kept.
pub const MAX_UNDO: usize = 256;
/// Highest flag-set index of `tools/web-browse.mjs`.
pub const MAX_FLAG_SET: u32 = 6;
/// Largest PNG the capture reads back, bytes.
pub const MAX_PNG_BYTES: u64 = 16 * 1024 * 1024;

/// Files a build must hold before it can be served.
const SERVE_FILES: [&str; 4] = ["index.html", "engine.js", "engine_bg.wasm", "build.json"];
/// Event kinds `tools/web-browse.mjs` replays.
const EVENT_KINDS: [&str; 6] = ["key", "key_down", "key_up", "mouse_move", "click", "wait"];
/// Named keys beside single letters and digits.
const NAMED_KEYS: [&str; 11] = [
    "ArrowLeft",
    "ArrowUp",
    "ArrowRight",
    "ArrowDown",
    "Space",
    "Enter",
    "Escape",
    "Tab",
    "Shift",
    "Control",
    "Alt",
];
/// Parent variables a launched node tool may see (everything else is cleared).
const INHERIT: [&str; 16] = [
    "PATH",
    "HOME",
    "USERPROFILE",
    "APPDATA",
    "LOCALAPPDATA",
    "TEMP",
    "TMP",
    "TMPDIR",
    "CARGO_HOME",
    "RUSTUP_HOME",
    "RUSTUP_TOOLCHAIN",
    "CARGO_TARGET_DIR",
    "ENGINE_WEB_COMMIT",
    "PROGRAMFILES",
    "PROGRAMDATA",
    "USERNAME",
];

// ---------------------------------------------------------------------------
// The process seam.
// ---------------------------------------------------------------------------

/// What one finished child said.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunOutput {
    /// Exit code (`-1` when the child had none).
    pub code: i32,
    /// Captured stdout, lossy UTF-8.
    pub stdout: String,
    /// Captured stderr, lossy UTF-8.
    pub stderr: String,
}

/// A child that could not be started (no such program, bad argument).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunFailure(pub String);

/// Runs one program with args and waits. The shipped runner is [`LaunchRunner`]; tests
/// pass a fake. An `Err` is a spawn failure (no such program).
pub trait WebRunner: Send + Sync {
    /// Run `program` with `args` to completion.
    fn run(&self, program: &str, args: &[String]) -> Result<RunOutput, RunFailure>;
}

/// The shipped runner: [`launch_one_child`] with a cleared environment and the
/// [`INHERIT`] allowlist.
#[derive(Debug, Default)]
pub struct LaunchRunner {
    launches: AtomicU64,
}

impl WebRunner for LaunchRunner {
    fn run(&self, program: &str, args: &[String]) -> Result<RunOutput, RunFailure> {
        let fail = |error: crate::secret_launch::LaunchError| RunFailure(error.to_string());
        let mut spec = LaunchSpec::new(program).map_err(fail)?;
        for arg in args {
            spec.arg(arg).map_err(fail)?;
        }
        for name in INHERIT {
            spec.inherit(name).map_err(fail)?;
        }
        let launch = self.launches.fetch_add(1, Ordering::Relaxed);
        let outcome = launch_one_child(&spec, launch).map_err(fail)?;
        Ok(RunOutput {
            code: outcome.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&outcome.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&outcome.stderr).into_owned(),
        })
    }
}

// ---------------------------------------------------------------------------
// Scope and session slot.
// ---------------------------------------------------------------------------

/// What the project allows: where the node tools live, where builds and captures go,
/// which Chrome to drive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebScope {
    root: PathBuf,
    work: PathBuf,
    chrome: Option<String>,
}

impl WebScope {
    /// Scope over a project root; builds and captures land in `<root>/target/door-web`.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        let work = root.join("target").join("door-web");
        Self {
            root,
            work,
            chrome: None,
        }
    }

    /// Put builds and captures in another folder.
    #[must_use]
    pub fn with_work(mut self, work: impl Into<PathBuf>) -> Self {
        self.work = work.into();
        self
    }

    /// Name the Chrome program or path `browser.capture` drives.
    #[must_use]
    pub fn with_chrome(mut self, chrome: impl Into<String>) -> Self {
        self.chrome = Some(chrome.into());
        self
    }

    fn tool(&self, name: &str) -> Result<String, McpToolError> {
        let path = self.root.join("tools").join(name);
        if !path.is_file() {
            return Err(McpToolError::new(
                "tool_missing",
                format!(
                    "{} does not exist; the web scope root must be the repo root holding tools/{name}; \
                     re-run after the host allows the right root",
                    path.display()
                ),
            ));
        }
        path_text(&path)
    }
}

#[derive(Debug, Clone)]
struct BuildRec {
    id: u32,
    dir: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Undo {
    Build { prev_latest: Option<usize> },
    Serve { prev: Option<usize> },
    Input { prev_len: usize },
}

impl Undo {
    fn verb(&self) -> &'static str {
        match self {
            Self::Build { .. } => TOOL_BUILD_WEB,
            Self::Serve { .. } => TOOL_SERVE_WEB,
            Self::Input { .. } => TOOL_BROWSER_INPUT,
        }
    }
}

/// Per-session web state: scope, runner, builds, serve pointer, queued input, undo.
pub struct WebSlot {
    scope: Option<WebScope>,
    runner: Arc<dyn WebRunner>,
    builds: Vec<BuildRec>,
    latest: Option<usize>,
    serving: Option<usize>,
    inputs: Vec<InputEventDto>,
    undo: Vec<Undo>,
    captures: u32,
}

impl Default for WebSlot {
    fn default() -> Self {
        Self {
            scope: None,
            runner: Arc::new(LaunchRunner::default()),
            builds: Vec::new(),
            latest: None,
            serving: None,
            inputs: Vec::new(),
            undo: Vec::new(),
            captures: 0,
        }
    }
}

impl McpServer {
    /// Host call: allow the web verbs for this session under `scope`. No verb can do this.
    pub fn allow_web(&self, scope: WebScope) -> Result<(), McpToolError> {
        self.lock()?.web.scope = Some(scope);
        Ok(())
    }

    /// Host call: take the web scope away again (the verbs refuse `unscoped` once more).
    pub fn disallow_web(&self) -> Result<(), McpToolError> {
        self.lock()?.web.scope = None;
        Ok(())
    }

    /// Host call: replace the process runner (tests pass a fake; the default is [`LaunchRunner`]).
    pub fn set_web_runner(&self, runner: Arc<dyn WebRunner>) -> Result<(), McpToolError> {
        self.lock()?.web.runner = runner;
        Ok(())
    }
}

fn unscoped(verb: &str) -> McpToolError {
    McpToolError::new(
        UNSCOPED,
        format!(
            "`{verb}` is refused: the project has not allowed the web verbs for this session; \
             nothing ran. The host allows them with McpServer::allow_web(WebScope); re-run after \
             that grant, not before"
        ),
    )
}

fn path_text(path: &Path) -> Result<String, McpToolError> {
    path.to_str().map(str::to_owned).ok_or_else(|| {
        McpToolError::new(
            "bad_scope",
            format!(
                "path {} is not UTF-8; re-run with a scope root that is",
                path.display()
            ),
        )
    })
}

/// Scope plus runner of this session, or the `unscoped` refusal.
fn scoped(
    state: &crate::mcp::ServerState,
    verb: &str,
) -> Result<(WebScope, Arc<dyn WebRunner>), McpToolError> {
    match &state.web.scope {
        Some(scope) => Ok((scope.clone(), Arc::clone(&state.web.runner))),
        None => Err(unscoped(verb)),
    }
}

/// First `<work>/<prefix>-<n>` folder that does not exist yet, counting from `from`.
fn free_dir(work: &Path, prefix: &str, from: u32) -> (u32, PathBuf) {
    let mut n = from.max(1);
    loop {
        let dir = work.join(format!("{prefix}-{n}"));
        if !dir.exists() {
            return (n, dir);
        }
        n += 1;
    }
}

fn run_tool(
    runner: &Arc<dyn WebRunner>,
    verb: &str,
    args: &[String],
) -> Result<RunOutput, McpToolError> {
    runner.run("node", args).map_err(|why| {
        McpToolError::new(
            "spawn_failed",
            format!(
                "`{verb}` could not start node ({}); install Node 24 on the PATH and re-run",
                why.0
            ),
        )
    })
}

fn tail(text: &str, lines: usize, chars: usize) -> String {
    let all: Vec<&str> = text.trim_end().lines().collect();
    let from = all.len().saturating_sub(lines);
    let joined = all[from..].join("\n");
    clip(&joined, chars)
}

/// At most `max` chars, control characters (other than newline) dropped.
fn clip(text: &str, max: usize) -> String {
    text.chars()
        .filter(|c| !c.is_control() || *c == '\n')
        .take(max)
        .collect()
}

fn read_json(path: &Path) -> Option<Value> {
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

// ---------------------------------------------------------------------------
// build.web
// ---------------------------------------------------------------------------

/// Args of `build.web`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde", default)]
#[schemars(crate = "rmcp::schemars")]
pub struct BuildWebArgs {
    pub commit: Option<String>,
    pub optimize: Option<bool>,
    pub undo: Option<bool>,
}

/// Result of `build.web`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct BuildWebOutput {
    pub build: u32,
    pub dir: String,
    pub commit: String,
    pub total_bytes: u64,
    pub wasm_bytes: u64,
    pub files: Vec<String>,
    pub undone: bool,
}

fn valid_commit(commit: &str) -> bool {
    !commit.is_empty()
        && commit.len() <= MAX_COMMIT_CHARS
        && commit
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-')
}

fn describe_build(rec: &BuildRec, undone: bool) -> Result<BuildWebOutput, McpToolError> {
    let info = read_json(&rec.dir.join("build.json")).ok_or_else(|| {
        McpToolError::new(
            "build_incomplete",
            format!(
                "{} has no readable build.json; re-run build.web",
                rec.dir.display()
            ),
        )
    })?;
    let files = info.get("files").and_then(Value::as_object);
    let wasm_bytes = files
        .and_then(|f| f.get("engine_bg.wasm"))
        .and_then(|f| f.get("bytes"))
        .and_then(Value::as_u64)
        .ok_or_else(|| {
            McpToolError::new(
                "build_incomplete",
                format!(
                    "{}/build.json lists no engine_bg.wasm; re-run build.web",
                    rec.dir.display()
                ),
            )
        })?;
    let mut names: Vec<String> = files
        .map(|f| f.keys().take(64).cloned().collect())
        .unwrap_or_default();
    names.sort();
    Ok(BuildWebOutput {
        build: rec.id,
        dir: rec.dir.to_string_lossy().into_owned(),
        commit: info
            .get("commit")
            .and_then(Value::as_str)
            .map(|c| clip(c, MAX_COMMIT_CHARS))
            .unwrap_or_default(),
        total_bytes: info.get("totalBytes").and_then(Value::as_u64).unwrap_or(0),
        wasm_bytes,
        files: names,
        undone,
    })
}

fn nothing_to_undo(verb: &str, newest: Option<&Undo>) -> McpToolError {
    let why = match newest {
        None => "no web change has been made in this session".to_owned(),
        Some(other) => format!(
            "the newest change was made by `{}`; undo that verb first",
            other.verb()
        ),
    };
    McpToolError::new(
        "nothing_to_undo",
        format!("`{verb}` has nothing to undo: {why}; re-run without undo or undo the newest verb"),
    )
}

fn push_undo(state: &mut crate::mcp::ServerState, record: Undo) {
    if state.web.undo.len() >= MAX_UNDO {
        state.web.undo.remove(0);
    }
    state.web.undo.push(record);
}

fn build_web(service: &McpServer, args: &BuildWebArgs) -> Result<BuildWebOutput, McpToolError> {
    service.dispatch(TOOL_BUILD_WEB, |state| {
        let (scope, runner) = scoped(state, TOOL_BUILD_WEB)?;
        if args.undo.unwrap_or(false) {
            if args.commit.is_some() || args.optimize.is_some() {
                return Err(bad(
                    "undo takes no other args; re-run with only {undo: true}".to_owned(),
                ));
            }
            return match state.web.undo.last().cloned() {
                Some(Undo::Build { prev_latest }) => {
                    let shown = state
                        .web
                        .latest
                        .and_then(|i| state.web.builds.get(i).cloned())
                        .ok_or_else(|| nothing_to_undo(TOOL_BUILD_WEB, None))?;
                    // The report may not read the folder (it is never deleted, but may have been moved).
                    let out = describe_build(&shown, true).unwrap_or_else(|_| BuildWebOutput {
                        build: shown.id,
                        dir: shown.dir.to_string_lossy().into_owned(),
                        commit: String::new(),
                        total_bytes: 0,
                        wasm_bytes: 0,
                        files: Vec::new(),
                        undone: true,
                    });
                    state.web.latest = prev_latest;
                    state.web.undo.pop();
                    Ok(out)
                }
                other => Err(nothing_to_undo(TOOL_BUILD_WEB, other.as_ref())),
            };
        }
        if let Some(commit) = &args.commit {
            if !valid_commit(commit) {
                return Err(bad(format!(
                    "commit `{}` must be 1..={MAX_COMMIT_CHARS} chars of a-z A-Z 0-9 . _ -; re-run with a label like a short hash",
                    clip(commit, 40)
                )));
            }
        }
        if state.web.builds.len() >= MAX_BUILDS {
            return Err(McpToolError::new(
                "over_budget",
                format!("this session holds {MAX_BUILDS} builds already; start a new session to build more"),
            ));
        }
        let script = scope.tool("web-build.mjs")?;
        let next = state.web.builds.last().map_or(1, |b| b.id + 1);
        let (id, dir) = free_dir(&scope.work, "build", next);
        let mut argv = vec![script, "--out".to_owned(), path_text(&dir)?];
        if let Some(commit) = &args.commit {
            argv.push("--commit".to_owned());
            argv.push(commit.clone());
        }
        if args.optimize == Some(false) {
            argv.push("--no-opt".to_owned());
        }
        let out = run_tool(&runner, TOOL_BUILD_WEB, &argv)?;
        if out.code != 0 {
            return Err(McpToolError::new(
                "build_failed",
                format!(
                    "web-build exited {}; last lines: {}; fix the cause and re-run build.web",
                    out.code,
                    tail(&format!("{}\n{}", out.stdout, out.stderr), 12, 1500)
                ),
            ));
        }
        let rec = BuildRec { id, dir };
        let output = describe_build(&rec, false)?;
        let prev_latest = state.web.latest;
        state.web.builds.push(rec);
        state.web.latest = Some(state.web.builds.len() - 1);
        push_undo(state, Undo::Build { prev_latest });
        Ok(output)
    })
}

crate::e2_tool!(
    /// `build.web` tool: build the browser shell into a fresh folder.
    BuildWebTool,
    name = TOOL_BUILD_WEB,
    description = "Build the browser shell (tools/web-build.mjs) into a fresh build folder; optional commit label, optimize false skips wasm-opt. Returns build id, sizes, files. undo true points back at the previous build. Refused (unscoped) until the project allows web.",
    readonly = false,
    args = BuildWebArgs,
    output = BuildWebOutput,
    invoke = build_web
);

// ---------------------------------------------------------------------------
// serve.web
// ---------------------------------------------------------------------------

/// Args of `serve.web`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde", default)]
#[schemars(crate = "rmcp::schemars")]
pub struct ServeWebArgs {
    pub build: Option<u32>,
    pub undo: Option<bool>,
}

/// Result of `serve.web`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct ServeWebOutput {
    pub build: Option<u32>,
    pub dir: String,
    pub files: u32,
    pub bytes: u64,
    pub loopback_only: bool,
    pub undone: bool,
}

fn serve_web(service: &McpServer, args: &ServeWebArgs) -> Result<ServeWebOutput, McpToolError> {
    service.dispatch(TOOL_SERVE_WEB, |state| {
        scoped(state, TOOL_SERVE_WEB)?;
        if args.undo.unwrap_or(false) {
            if args.build.is_some() {
                return Err(bad(
                    "undo takes no other args; re-run with only {undo: true}".to_owned(),
                ));
            }
            return match state.web.undo.last().cloned() {
                Some(Undo::Serve { prev }) => {
                    state.web.serving = prev;
                    state.web.undo.pop();
                    let rec = prev.and_then(|i| state.web.builds.get(i));
                    Ok(ServeWebOutput {
                        build: rec.map(|b| b.id),
                        dir: rec.map_or(String::new(), |b| b.dir.to_string_lossy().into_owned()),
                        files: 0,
                        bytes: 0,
                        loopback_only: true,
                        undone: true,
                    })
                }
                other => Err(nothing_to_undo(TOOL_SERVE_WEB, other.as_ref())),
            };
        }
        let index = match args.build {
            Some(id) => state
                .web
                .builds
                .iter()
                .position(|b| b.id == id)
                .ok_or_else(|| {
                    let ids: Vec<String> =
                        state.web.builds.iter().map(|b| b.id.to_string()).collect();
                    McpToolError::new(
                        "unknown_build",
                        format!(
                            "build {id} does not exist; this session has builds [{}]; re-run with one of them or none for the newest",
                            ids.join(", ")
                        ),
                    )
                })?,
            None => state.web.latest.ok_or_else(|| {
                McpToolError::new(
                    "no_build",
                    "nothing is built in this session; run build.web first, then re-run serve.web",
                )
            })?,
        };
        let rec = state.web.builds[index].clone();
        let mut bytes = 0u64;
        for name in SERVE_FILES {
            let meta = std::fs::metadata(rec.dir.join(name)).map_err(|_| {
                McpToolError::new(
                    "incomplete_build",
                    format!(
                        "build {} lacks {name}; run build.web again and serve that build",
                        rec.id
                    ),
                )
            })?;
            bytes += meta.len();
        }
        let prev = state.web.serving;
        state.web.serving = Some(index);
        push_undo(state, Undo::Serve { prev });
        Ok(ServeWebOutput {
            build: Some(rec.id),
            dir: rec.dir.to_string_lossy().into_owned(),
            files: u32::try_from(SERVE_FILES.len()).unwrap_or(0),
            bytes,
            loopback_only: true,
            undone: false,
        })
    })
}

crate::e2_tool!(
    /// `serve.web` tool: point the next captures at one build.
    ServeWebTool,
    name = TOOL_SERVE_WEB,
    description = "Pick the build (default newest) that browser.capture loads and check it is complete. Captures serve it on 127.0.0.1 only, one server per capture. undo true restores the previous pick. Refused (unscoped) until the project allows web.",
    readonly = false,
    args = ServeWebArgs,
    output = ServeWebOutput,
    invoke = serve_web
);

// ---------------------------------------------------------------------------
// browser.input
// ---------------------------------------------------------------------------

/// One scripted input event.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde", default)]
#[schemars(crate = "rmcp::schemars")]
pub struct InputEventDto {
    pub kind: String,
    pub key: Option<String>,
    pub x: Option<i32>,
    pub y: Option<i32>,
    pub ms: Option<u32>,
}

/// Args of `browser.input`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde", default)]
#[schemars(crate = "rmcp::schemars")]
pub struct BrowserInputArgs {
    pub events: Vec<InputEventDto>,
    pub undo: Option<bool>,
}

/// Result of `browser.input`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct BrowserInputOutput {
    pub added: u32,
    pub queued: u32,
    pub undone: bool,
}

fn check_event(index: usize, event: &InputEventDto) -> Result<(), McpToolError> {
    let at = |why: String| {
        bad(format!(
            "event {index}: {why}; re-run with the legal values"
        ))
    };
    if !EVENT_KINDS.contains(&event.kind.as_str()) {
        return Err(at(format!(
            "kind `{}` is not one of [{}]",
            clip(&event.kind, 24),
            EVENT_KINDS.join(", ")
        )));
    }
    match event.kind.as_str() {
        "key" | "key_down" | "key_up" => {
            let key = event.key.as_deref().unwrap_or("");
            let single = key.chars().count() == 1 && key.chars().all(|c| c.is_ascii_alphanumeric());
            if !(single || NAMED_KEYS.contains(&key)) || key.len() > MAX_KEY_CHARS {
                return Err(at(format!(
                    "key `{}` must be one letter or digit or one of [{}]",
                    clip(key, 20),
                    NAMED_KEYS.join(", ")
                )));
            }
        }
        "mouse_move" | "click" => {
            let (x, y) = (event.x.unwrap_or(-1), event.y.unwrap_or(-1));
            if !(0..=MAX_X).contains(&x) || !(0..=MAX_Y).contains(&y) {
                return Err(at(format!(
                    "x {x} and y {y} must be inside 0..={MAX_X} and 0..={MAX_Y}"
                )));
            }
        }
        _ => {
            let ms = event.ms.unwrap_or(u32::MAX);
            if ms > MAX_WAIT_MS {
                return Err(at(format!("ms must be inside 0..={MAX_WAIT_MS}")));
            }
        }
    }
    Ok(())
}

fn browser_input(
    service: &McpServer,
    args: &BrowserInputArgs,
) -> Result<BrowserInputOutput, McpToolError> {
    service.dispatch(TOOL_BROWSER_INPUT, |state| {
        scoped(state, TOOL_BROWSER_INPUT)?;
        let queued_now =
            |s: &crate::mcp::ServerState| u32::try_from(s.web.inputs.len()).unwrap_or(u32::MAX);
        if args.undo.unwrap_or(false) {
            if !args.events.is_empty() {
                return Err(bad(
                    "undo takes no events; re-run with only {undo: true}".to_owned()
                ));
            }
            return match state.web.undo.last().cloned() {
                Some(Undo::Input { prev_len }) => {
                    let removed = state.web.inputs.len().saturating_sub(prev_len);
                    state.web.inputs.truncate(prev_len);
                    state.web.undo.pop();
                    Ok(BrowserInputOutput {
                        added: u32::try_from(removed).unwrap_or(u32::MAX),
                        queued: queued_now(state),
                        undone: true,
                    })
                }
                other => Err(nothing_to_undo(TOOL_BROWSER_INPUT, other.as_ref())),
            };
        }
        if args.events.is_empty() || args.events.len() > MAX_EVENTS_PER_CALL {
            return Err(bad(format!(
                "events holds {} entries; send 1..={MAX_EVENTS_PER_CALL} per call and re-run",
                args.events.len()
            )));
        }
        if state.web.inputs.len() + args.events.len() > MAX_QUEUED_EVENTS {
            return Err(McpToolError::new(
                "over_budget",
                format!(
                    "the queue would pass {MAX_QUEUED_EVENTS} events; undo some batches and re-run"
                ),
            ));
        }
        for (index, event) in args.events.iter().enumerate() {
            check_event(index, event)?;
        }
        let prev_len = state.web.inputs.len();
        state.web.inputs.extend(args.events.iter().cloned());
        push_undo(state, Undo::Input { prev_len });
        Ok(BrowserInputOutput {
            added: u32::try_from(args.events.len()).unwrap_or(u32::MAX),
            queued: queued_now(state),
            undone: false,
        })
    })
}

crate::e2_tool!(
    /// `browser.input` tool: queue scripted input for the next captures.
    BrowserInputTool,
    name = TOOL_BROWSER_INPUT,
    description = "Queue 1..=64 input events (key, key_down, key_up, mouse_move, click at x 0..=1279 y 0..=719, wait ms 0..=2000). Every later capture replays the queue from page load. undo true drops the newest batch. Refused (unscoped) until the project allows web.",
    readonly = false,
    args = BrowserInputArgs,
    output = BrowserInputOutput,
    invoke = browser_input
);

// ---------------------------------------------------------------------------
// browser.capture
// ---------------------------------------------------------------------------

/// Args of `browser.capture`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde", default)]
#[schemars(crate = "rmcp::schemars")]
pub struct BrowserCaptureArgs {
    pub flag_set: Option<u32>,
}

/// Result of `browser.capture`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct BrowserCaptureOutput {
    pub capture: u32,
    pub dir: String,
    pub png_path: String,
    pub png_bytes: u64,
    pub png_hash: String,
    pub width: u32,
    pub height: u32,
    pub non_blank: bool,
    pub flag_set: String,
    pub receipt: String,
    pub ready_line: String,
    pub console: Vec<String>,
    pub exceptions: Vec<String>,
    pub input_events: u32,
    pub input_png: Option<String>,
}

/// `(width, height)` from the IHDR chunk of a PNG, or `None` when it is not one.
fn png_size(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() < 24 || bytes[..8] != [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a] {
        return None;
    }
    let w = u32::from_be_bytes(bytes[16..20].try_into().ok()?);
    let h = u32::from_be_bytes(bytes[20..24].try_into().ok()?);
    Some((w, h))
}

/// The same test `tools/web-browse.mjs` calls drawn: not black, not one flat colour.
fn stats_drawn(stats: &Value) -> bool {
    let num = |k: &str| stats.get(k).and_then(Value::as_f64).unwrap_or(0.0);
    num("meanLuma") >= 20.0 && num("distinct") >= 4.0 && num("dominantShare") < 0.98
}

/// One receipt line: the facts of the page's ready line plus the budget numbers.
fn receipt_line(ready: &Value, receipt: &Value) -> String {
    let text = |v: Option<&Value>| {
        v.and_then(Value::as_str)
            .map_or_else(|| "na".to_owned(), |s| clip(s, 40))
    };
    let number = |v: Option<&Value>| {
        v.and_then(Value::as_u64)
            .map_or_else(|| "na".to_owned(), |n| n.to_string())
    };
    let adapter = ready.get("adapter");
    format!(
        "commit={} backend={} adapter={}/{} tier={} validation={} mem={} ttff_ms={}",
        text(ready.get("commit")),
        text(ready.get("backend")),
        text(adapter.and_then(|a| a.get("vendor"))),
        text(adapter.and_then(|a| a.get("architecture"))),
        text(ready.get("tier")),
        number(
            ready
                .get("validation")
                .or_else(|| ready.get("validationErrors"))
        ),
        number(receipt.get("wasmMemoryBytes")),
        number(receipt.get("ttffMs")),
    )
}

fn strings(value: Option<&Value>, max: usize) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .take(max)
                .map(|line| clip(line, MAX_LINE_CHARS))
                .collect()
        })
        .unwrap_or_default()
}

/// Read what `tools/web-browse.mjs` left in `dir` into the capture answer.
fn read_capture(
    dir: &Path,
    attempts: Option<&Value>,
    capture: u32,
    input_events: u32,
) -> Result<BrowserCaptureOutput, McpToolError> {
    let png_file = dir.join("ready.png");
    let size = std::fs::metadata(&png_file).map(|m| m.len()).unwrap_or(0);
    if size == 0 || size > MAX_PNG_BYTES {
        let reasons: Vec<String> = attempts
            .and_then(|a| a.get("attempts"))
            .and_then(Value::as_array)
            .map(|list| {
                list.iter()
                    .take(4)
                    .map(|a| {
                        let set = a.get("set").and_then(Value::as_str).unwrap_or("?");
                        let why = a
                            .get("verdict")
                            .and_then(|v| v.get("why"))
                            .and_then(Value::as_str)
                            .unwrap_or("?");
                        clip(&format!("{set}: {why}"), 200)
                    })
                    .collect()
            })
            .unwrap_or_default();
        return Err(McpToolError::new(
            "no_adapter",
            format!(
                "no flag set gave a drawn canvas, so there is no PNG ({}); try another flag_set on a machine with WebGPU, or read {}",
                reasons.join(" | "),
                dir.join("attempts.json").display()
            ),
        ));
    }
    let png = std::fs::read(&png_file).map_err(|error| {
        McpToolError::new(
            "io_failed",
            format!("cannot read {}: {error}", png_file.display()),
        )
    })?;
    let (width, height) = png_size(&png).ok_or_else(|| {
        McpToolError::new(
            "bad_png",
            format!(
                "{} is not a PNG; re-run browser.capture",
                png_file.display()
            ),
        )
    })?;
    let winner = attempts
        .and_then(|a| a.get("attempts"))
        .and_then(Value::as_array)
        .and_then(|list| {
            list.iter().find(|a| {
                a.get("verdict")
                    .and_then(|v| v.get("ok"))
                    .and_then(Value::as_bool)
                    == Some(true)
            })
        });
    let non_blank = winner.and_then(|w| w.get("stats")).is_some_and(stats_drawn);
    let ready_text = std::fs::read_to_string(dir.join("report.json")).unwrap_or_default();
    let ready: Value = serde_json::from_str(&ready_text).unwrap_or(Value::Null);
    let receipt = read_json(&dir.join("receipt.json")).unwrap_or(Value::Null);
    let input_png = dir.join("input.png");
    Ok(BrowserCaptureOutput {
        capture,
        dir: dir.to_string_lossy().into_owned(),
        png_path: png_file.to_string_lossy().into_owned(),
        png_bytes: size,
        png_hash: format!("{:016x}", engine_core::hash_bytes(&png)),
        width,
        height,
        non_blank,
        flag_set: winner
            .and_then(|w| w.get("set"))
            .and_then(Value::as_str)
            .map(|s| clip(s, 40))
            .unwrap_or_default(),
        receipt: receipt_line(&ready, &receipt),
        ready_line: clip(ready_text.trim(), 4000),
        console: strings(winner.and_then(|w| w.get("console")), MAX_CONSOLE_LINES),
        exceptions: strings(winner.and_then(|w| w.get("exceptions")), MAX_CONSOLE_LINES),
        input_events,
        input_png: input_png
            .is_file()
            .then(|| input_png.to_string_lossy().into_owned()),
    })
}

fn browser_capture(
    service: &McpServer,
    args: &BrowserCaptureArgs,
) -> Result<BrowserCaptureOutput, McpToolError> {
    service.dispatch(TOOL_BROWSER_CAPTURE, |state| {
        let (scope, runner) = scoped(state, TOOL_BROWSER_CAPTURE)?;
        if let Some(set) = args.flag_set {
            if set > MAX_FLAG_SET {
                return Err(bad(format!(
                    "flag_set {set} is outside 0..={MAX_FLAG_SET}; re-run in range or leave it out to try every set"
                )));
            }
        }
        let serving = state.web.serving.and_then(|i| state.web.builds.get(i)).cloned().ok_or_else(|| {
            McpToolError::new(
                "not_served",
                "no build is served; run build.web, then serve.web, then re-run browser.capture",
            )
        })?;
        let chrome = scope.chrome.clone().ok_or_else(|| {
            McpToolError::new(
                "no_chrome",
                "the web scope names no Chrome; the host sets WebScope::with_chrome(path) and you re-run",
            )
        })?;
        if state.web.captures >= MAX_CAPTURES {
            return Err(McpToolError::new(
                "over_budget",
                format!("this session made {MAX_CAPTURES} captures; start a new session for more"),
            ));
        }
        let script = scope.tool("web-browse.mjs")?;
        let (_, dir) = free_dir(&scope.work, "capture", state.web.captures + 1);
        std::fs::create_dir_all(&dir).map_err(|error| {
            McpToolError::new(
                "io_failed",
                format!("cannot create {}: {error}; re-run with a writable work folder", dir.display()),
            )
        })?;
        let mut argv = vec![
            script,
            "--dir".to_owned(),
            path_text(&serving.dir)?,
            "--chrome".to_owned(),
            chrome,
            "--out".to_owned(),
            path_text(&dir)?,
        ];
        if let Some(set) = args.flag_set {
            argv.push("--only".to_owned());
            argv.push(set.to_string());
        }
        let input_events = u32::try_from(state.web.inputs.len()).unwrap_or(u32::MAX);
        if !state.web.inputs.is_empty() {
            let file = dir.join("input.json");
            let body = serde_json::to_string(&state.web.inputs).map_err(|error| {
                McpToolError::new("io_failed", format!("cannot encode the input queue: {error}"))
            })?;
            std::fs::write(&file, body).map_err(|error| {
                McpToolError::new("io_failed", format!("cannot write {}: {error}", file.display()))
            })?;
            argv.push("--input".to_owned());
            argv.push(path_text(&file)?);
        }
        let out = run_tool(&runner, TOOL_BROWSER_CAPTURE, &argv)?;
        state.web.captures += 1;
        let attempts = read_json(&dir.join("attempts.json"));
        if out.code != 0 {
            return Err(McpToolError::new(
                "browse_failed",
                format!(
                    "web-browse exited {}; last lines: {}; fix the page and re-run browser.capture",
                    out.code,
                    tail(&format!("{}\n{}", out.stdout, out.stderr), 12, 1500)
                ),
            ));
        }
        read_capture(&dir, attempts.as_ref(), state.web.captures, input_events)
    })
}

crate::e2_tool!(
    /// `browser.capture` tool: headless Chrome loads the served build.
    BrowserCaptureTool,
    name = TOOL_BROWSER_CAPTURE,
    description = "Load the served build in headless Chrome (127.0.0.1 only), replay queued input, return the canvas PNG path, hash and size, non_blank, console lines and a receipt line (commit, backend, adapter, tier, validation, mem, ttff_ms). Needs build.web then serve.web. Refused (unscoped) until the project allows web.",
    readonly = false,
    args = BrowserCaptureArgs,
    output = BrowserCaptureOutput,
    invoke = browser_capture
);

/// The web pack: four verbs, loaded on demand.
pub fn pack() -> Result<PackDef, PackError> {
    let mut pack = PackDef::new(
        PACK,
        "browser loop: build the shell into a fresh folder, pick a build to serve, queue scripted \
         input, capture the canvas PNG with console lines and a receipt line in headless Chrome; \
         refused (unscoped) until the project allows web; build, serve and input undo",
        false,
    );
    pack.add(VerbSpec::of::<BuildWebTool>(
        TOOL_BUILD_WEB,
        PACK,
        CapabilityCard::Commit,
    )?);
    pack.add(VerbSpec::of::<ServeWebTool>(
        TOOL_SERVE_WEB,
        PACK,
        CapabilityCard::Commit,
    )?);
    pack.add(VerbSpec::of::<BrowserCaptureTool>(
        TOOL_BROWSER_CAPTURE,
        PACK,
        CapabilityCard::Commit,
    )?);
    pack.add(VerbSpec::of::<BrowserInputTool>(
        TOOL_BROWSER_INPUT,
        PACK,
        CapabilityCard::Commit,
    )?);
    Ok(pack)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::packs::PackRegistry;
    use engine_editor::capability::Capability;
    use rmcp::handler::server::router::tool::SyncTool;
    use std::sync::Mutex;

    /// A PNG header with the given size, then filler: enough for `png_size` and a hash.
    fn fake_png(w: u32, h: u32) -> Vec<u8> {
        let mut png = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 13];
        png.extend_from_slice(b"IHDR");
        png.extend_from_slice(&w.to_be_bytes());
        png.extend_from_slice(&h.to_be_bytes());
        png.extend_from_slice(&[8, 6, 0, 0, 0, 0, 0, 0, 0]);
        png.extend_from_slice(b"IDAT-fixture-pixels");
        png
    }

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Browse {
        Drawn,
        NoAdapter,
        Crash,
    }

    /// Stands in for node: writes what `web-build.mjs` and `web-browse.mjs` write.
    struct Fake {
        calls: Mutex<Vec<Vec<String>>>,
        input_seen: Mutex<Option<String>>,
        build_code: i32,
        browse: Browse,
    }

    impl Fake {
        fn new(build_code: i32, browse: Browse) -> Arc<Self> {
            Arc::new(Self {
                calls: Mutex::new(Vec::new()),
                input_seen: Mutex::new(None),
                build_code,
                browse,
            })
        }
        fn arg(args: &[String], flag: &str) -> Option<String> {
            args.iter()
                .position(|a| a == flag)
                .and_then(|i| args.get(i + 1))
                .cloned()
        }
        fn count(&self, script: &str) -> usize {
            self.calls
                .lock()
                .expect("calls lock")
                .iter()
                .filter(|c| c[0].ends_with(script))
                .count()
        }
    }

    impl WebRunner for Fake {
        fn run(&self, program: &str, args: &[String]) -> Result<RunOutput, RunFailure> {
            assert_eq!(program, "node", "only node is ever started");
            self.calls.lock().expect("calls lock").push(args.to_vec());
            let ok = |stdout: &str| RunOutput {
                code: 0,
                stdout: stdout.to_owned(),
                stderr: String::new(),
            };
            if args[0].ends_with("web-build.mjs") {
                if self.build_code != 0 {
                    return Ok(RunOutput {
                        code: self.build_code,
                        stdout: String::new(),
                        stderr: "web-build: cargo build failed (exit 101)\nerror[E0432]: unresolved import"
                            .to_owned(),
                    });
                }
                let out = PathBuf::from(Self::arg(args, "--out").expect("--out"));
                std::fs::create_dir_all(&out).expect("build dir");
                for name in ["index.html", "engine.js", "engine_bg.wasm"] {
                    std::fs::write(out.join(name), b"fixture").expect("write");
                }
                let commit = Self::arg(args, "--commit").unwrap_or_else(|| "abc123def456".into());
                let info = serde_json::json!({
                    "schema": 1, "commit": commit, "totalBytes": 288_616,
                    "files": {
                        "engine.js": {"bytes": 69_514}, "engine_bg.wasm": {"bytes": 215_789},
                        "index.html": {"bytes": 3_313}
                    }
                });
                std::fs::write(out.join("build.json"), info.to_string()).expect("write");
                return Ok(ok("[web-build] done"));
            }
            assert!(
                args[0].ends_with("web-browse.mjs"),
                "unexpected script {}",
                args[0]
            );
            let out = PathBuf::from(Self::arg(args, "--out").expect("--out"));
            if let Some(file) = Self::arg(args, "--input") {
                *self.input_seen.lock().expect("input lock") =
                    Some(std::fs::read_to_string(file).expect("input file"));
            }
            match self.browse {
                Browse::Crash => Ok(RunOutput {
                    code: 2,
                    stdout: String::new(),
                    stderr: "web-browse: could not start Chrome".to_owned(),
                }),
                Browse::NoAdapter => {
                    let attempts = serde_json::json!({"attempts": [
                        {"set": "bare", "verdict": {"ok": false, "code": "no-adapter", "why": "no suitable graphics adapter"}}
                    ]});
                    std::fs::write(out.join("attempts.json"), attempts.to_string()).expect("write");
                    Ok(ok("NO ADAPTER: every flag set was tried"))
                }
                Browse::Drawn => {
                    std::fs::write(out.join("ready.png"), fake_png(1280, 720)).expect("png");
                    let ready = serde_json::json!({
                        "ok": true, "commit": "abc123def456", "backend": "BrowserWebGpu",
                        "adapter": {"vendor": "google", "architecture": "swiftshader"},
                        "tier": "web", "validation": 0, "wasmMemoryBytes": 1_179_648
                    });
                    std::fs::write(out.join("report.json"), format!("{ready}\n")).expect("write");
                    std::fs::write(
                        out.join("receipt.json"),
                        r#"{"schema":1,"wasmMemoryBytes":1179648,"ttffMs":900}"#,
                    )
                    .expect("write");
                    let attempts = serde_json::json!({"attempts": [
                        {"set": "bare", "verdict": {"ok": false}, "console": [], "exceptions": []},
                        {"set": "swiftshader-angle", "verdict": {"ok": true},
                         "stats": {"distinct": 81, "dominantShare": 0.77, "meanLuma": 131.8},
                         "console": ["log: golden d57b5c8ae3096614 OK", "warning: x\u{7}y"],
                         "exceptions": []}
                    ]});
                    std::fs::write(out.join("attempts.json"), attempts.to_string()).expect("write");
                    if args.iter().any(|a| a == "--input") {
                        std::fs::write(out.join("input.png"), fake_png(1280, 720)).expect("png");
                    }
                    Ok(ok("READY LINE ..."))
                }
            }
        }
    }

    struct Rig {
        server: McpServer,
        fake: Arc<Fake>,
        root: PathBuf,
    }

    impl Drop for Rig {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    fn rig(name: &str, build_code: i32, browse: Browse, allowed: bool) -> Rig {
        let root = std::env::temp_dir().join(format!("e2040-web-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("tools")).expect("tools dir");
        for script in ["web-build.mjs", "web-browse.mjs"] {
            std::fs::write(root.join("tools").join(script), "// fixture").expect("script");
        }
        let server = McpServer::new(7, Capability::unrestricted(7));
        let fake = Fake::new(build_code, browse);
        let runner: Arc<dyn WebRunner> = Arc::<Fake>::clone(&fake);
        server.set_web_runner(runner).expect("runner");
        if allowed {
            server
                .allow_web(WebScope::new(&root).with_chrome("chrome-fixture"))
                .expect("allow");
        }
        Rig { server, fake, root }
    }

    fn build(rig: &Rig, args: BuildWebArgs) -> Result<BuildWebOutput, McpToolError> {
        BuildWebTool::invoke(&rig.server, args)
    }
    fn serve(rig: &Rig, args: ServeWebArgs) -> Result<ServeWebOutput, McpToolError> {
        ServeWebTool::invoke(&rig.server, args)
    }
    fn capture(rig: &Rig) -> Result<BrowserCaptureOutput, McpToolError> {
        BrowserCaptureTool::invoke(&rig.server, BrowserCaptureArgs::default())
    }
    fn input(rig: &Rig, events: Vec<InputEventDto>) -> Result<BrowserInputOutput, McpToolError> {
        BrowserInputTool::invoke(&rig.server, BrowserInputArgs { events, undo: None })
    }
    fn undo_input(rig: &Rig) -> Result<BrowserInputOutput, McpToolError> {
        BrowserInputTool::invoke(
            &rig.server,
            BrowserInputArgs {
                events: Vec::new(),
                undo: Some(true),
            },
        )
    }
    fn click(x: i32, y: i32) -> InputEventDto {
        InputEventDto {
            kind: "click".into(),
            x: Some(x),
            y: Some(y),
            ..Default::default()
        }
    }
    fn key(name: &str) -> InputEventDto {
        InputEventDto {
            kind: "key".into(),
            key: Some(name.into()),
            ..Default::default()
        }
    }

    #[test]
    fn the_pack_registers_four_commit_verbs_that_write() {
        let registry = PackRegistry::shipped();
        assert!(registry.errors().is_empty(), "{:?}", registry.errors());
        let pack = registry.pack(PACK).expect("web pack is registered");
        assert!(!pack.default_loaded(), "web loads on demand");
        assert_eq!(
            pack.verb_names(),
            vec![
                TOOL_BUILD_WEB,
                TOOL_SERVE_WEB,
                TOOL_BROWSER_CAPTURE,
                TOOL_BROWSER_INPUT
            ]
        );
        for verb in pack.verbs() {
            assert_eq!(verb.card(), CapabilityCard::Commit, "{}", verb.name());
            assert!(!verb.readonly(), "{} writes", verb.name());
            let model = verb.model();
            assert!(model.output_schema.is_some(), "{}", verb.name());
            assert!(
                verb.description().contains("unscoped"),
                "{} says it is refused by default",
                verb.name()
            );
        }
    }

    #[test]
    fn every_verb_is_refused_unscoped_by_default_and_nothing_runs() {
        let rig = rig("unscoped", 0, Browse::Drawn, false);
        let refusals = [
            build(&rig, BuildWebArgs::default()).expect_err("build refused"),
            serve(&rig, ServeWebArgs::default()).expect_err("serve refused"),
            capture(&rig).expect_err("capture refused"),
            input(&rig, vec![click(1, 1)]).expect_err("input refused"),
            undo_input(&rig).expect_err("undo refused too"),
        ];
        for refusal in &refusals {
            assert_eq!(refusal.code, UNSCOPED, "{refusal}");
            assert!(refusal.message.contains("allow_web"), "{refusal}");
            assert!(refusal.message.contains("re-run"), "{refusal}");
        }
        assert!(
            rig.fake.calls.lock().expect("calls").is_empty(),
            "no process ran"
        );
        // The same through api.call: the door routes by name and the gate holds.
        let via_door = crate::packs::ApiCallTool::invoke(
            &rig.server,
            crate::packs::ApiCallArgs {
                verb: TOOL_BUILD_WEB.to_owned(),
                args: None,
            },
        )
        .expect_err("api.call refuses");
        assert_eq!(via_door.code, UNSCOPED);
        // Granting the scope opens the verbs; taking it away closes them again.
        rig.server
            .allow_web(WebScope::new(&rig.root))
            .expect("allow");
        build(&rig, BuildWebArgs::default()).expect("scoped build runs");
        rig.server.disallow_web().expect("disallow");
        assert_eq!(
            build(&rig, BuildWebArgs::default())
                .expect_err("refused again")
                .code,
            UNSCOPED
        );
        // Every refusal and the one run sealed an audit record.
        let log = rig.server.audit_log().expect("audit");
        assert!(log.iter().any(|r| r.tool == TOOL_BUILD_WEB));
    }

    #[test]
    fn build_serve_capture_returns_a_non_blank_png_and_a_receipt_line() {
        let rig = rig("loop", 0, Browse::Drawn, true);
        let built = build(
            &rig,
            BuildWebArgs {
                commit: Some("abc123def456".into()),
                ..Default::default()
            },
        )
        .expect("build");
        assert_eq!(built.build, 1);
        assert_eq!(built.wasm_bytes, 215_789);
        assert_eq!(built.total_bytes, 288_616);
        assert_eq!(built.commit, "abc123def456");
        assert!(built.files.contains(&"engine_bg.wasm".to_owned()));
        assert!(built.dir.ends_with("build-1"));
        let served = serve(&rig, ServeWebArgs::default()).expect("serve");
        assert_eq!(served.build, Some(1));
        assert!(served.loopback_only);
        assert_eq!(served.files, 4);
        let shot = capture(&rig).expect("capture");
        assert!(shot.non_blank, "the canvas is drawn");
        assert!(shot.png_bytes > 0);
        assert_eq!((shot.width, shot.height), (1280, 720));
        assert_eq!(shot.png_hash.len(), 16);
        assert!(shot.png_path.ends_with("ready.png"));
        assert!(shot.receipt.contains("tier=web"), "{}", shot.receipt);
        assert!(shot.receipt.contains("validation=0"), "{}", shot.receipt);
        assert!(
            shot.receipt.contains("mem=1179648 ttff_ms=900"),
            "{}",
            shot.receipt
        );
        assert_eq!(shot.flag_set, "swiftshader-angle");
        assert_eq!(shot.console.len(), 2);
        assert!(
            !shot.console[1].contains('\u{7}'),
            "page text loses control characters: {:?}",
            shot.console
        );
        assert_eq!(shot.input_events, 0);
        assert_eq!(shot.input_png, None);
        // The process line is exactly the tool's own flags: Chrome, the served dir, a fresh out dir.
        let calls = rig.fake.calls.lock().expect("calls").clone();
        let browse = calls.last().expect("browse call");
        assert_eq!(
            Fake::arg(browse, "--chrome").as_deref(),
            Some("chrome-fixture")
        );
        assert_eq!(
            Fake::arg(browse, "--dir").as_deref(),
            Some(built.dir.as_str())
        );
        assert!(Fake::arg(browse, "--out")
            .expect("out")
            .ends_with("capture-1"));
        assert!(!browse.iter().any(|a| a == "--input" || a == "--only"));
        // A second capture lands in a fresh folder.
        let again = capture(&rig).expect("second capture");
        assert_eq!(again.capture, 2);
        assert_ne!(again.dir, shot.dir);
    }

    #[test]
    fn capture_before_build_or_serve_is_refused_with_the_next_step() {
        let rig = rig("order", 0, Browse::Drawn, true);
        assert_eq!(
            serve(&rig, ServeWebArgs::default())
                .expect_err("no build")
                .code,
            "no_build"
        );
        assert_eq!(
            capture(&rig).expect_err("nothing served").code,
            "not_served"
        );
        build(&rig, BuildWebArgs::default()).expect("build");
        let refused = capture(&rig).expect_err("built but not served");
        assert_eq!(refused.code, "not_served");
        assert!(refused.message.contains("serve.web"), "{refused}");
        assert_eq!(
            serve(
                &rig,
                ServeWebArgs {
                    build: Some(9),
                    undo: None
                }
            )
            .expect_err("unknown build")
            .code,
            "unknown_build"
        );
        assert_eq!(rig.fake.count("web-browse.mjs"), 0, "no browser started");
    }

    #[test]
    fn undo_walks_back_build_serve_and_input_in_order() {
        let rig = rig("undo", 0, Browse::Drawn, true);
        let first = build(&rig, BuildWebArgs::default()).expect("build 1");
        let second = build(&rig, BuildWebArgs::default()).expect("build 2");
        assert_eq!((first.build, second.build), (1, 2));
        serve(&rig, ServeWebArgs::default()).expect("serve newest");
        input(&rig, vec![click(5, 6)]).expect("queue");
        // The newest change is the input: build undo must wait its turn.
        let wrong = BuildWebTool::invoke(
            &rig.server,
            BuildWebArgs {
                undo: Some(true),
                ..Default::default()
            },
        )
        .expect_err("not the newest");
        assert_eq!(wrong.code, "nothing_to_undo");
        assert!(wrong.message.contains(TOOL_BROWSER_INPUT), "{wrong}");
        let dropped = undo_input(&rig).expect("undo input");
        assert!(dropped.undone);
        assert_eq!((dropped.added, dropped.queued), (1, 0));
        let unserved = ServeWebTool::invoke(
            &rig.server,
            ServeWebArgs {
                build: None,
                undo: Some(true),
            },
        )
        .expect("undo serve");
        assert!(unserved.undone);
        assert_eq!(unserved.build, None);
        assert_eq!(capture(&rig).expect_err("serve undone").code, "not_served");
        let back = BuildWebTool::invoke(
            &rig.server,
            BuildWebArgs {
                undo: Some(true),
                ..Default::default()
            },
        )
        .expect("undo build 2");
        assert!(back.undone);
        assert_eq!(back.build, 2, "the undone build is the one reported");
        // Nothing was deleted: the folder is still there; the pointer moved back to build 1.
        assert!(Path::new(&second.dir).is_dir());
        assert_eq!(
            serve(&rig, ServeWebArgs::default()).expect("serve").build,
            Some(1)
        );
        // Undo with extra args, or with an empty history, refuses.
        assert_eq!(
            BuildWebTool::invoke(
                &rig.server,
                BuildWebArgs {
                    undo: Some(true),
                    commit: Some("x".into()),
                    optimize: None
                }
            )
            .expect_err("mixed")
            .code,
            "bad_args"
        );
        let fresh = rig_fresh_undo();
        assert_eq!(
            undo_input(&fresh).expect_err("empty history").code,
            "nothing_to_undo"
        );
    }

    fn rig_fresh_undo() -> Rig {
        rig("undo-empty", 0, Browse::Drawn, true)
    }

    #[test]
    fn queued_input_replays_in_the_capture_and_is_validated() {
        let rig = rig("input", 0, Browse::Drawn, true);
        build(&rig, BuildWebArgs::default()).expect("build");
        serve(&rig, ServeWebArgs::default()).expect("serve");
        let queued = input(
            &rig,
            vec![
                key("a"),
                key("ArrowLeft"),
                click(640, 360),
                InputEventDto {
                    kind: "wait".into(),
                    ms: Some(50),
                    ..Default::default()
                },
            ],
        )
        .expect("queue");
        assert_eq!((queued.added, queued.queued), (4, 4));
        let shot = capture(&rig).expect("capture with input");
        assert_eq!(shot.input_events, 4);
        assert!(shot
            .input_png
            .as_deref()
            .is_some_and(|p| p.ends_with("input.png")));
        let seen = rig
            .fake
            .input_seen
            .lock()
            .expect("input")
            .clone()
            .expect("input file");
        let events: Vec<InputEventDto> = serde_json::from_str(&seen).expect("input json");
        assert_eq!(events.len(), 4);
        assert_eq!(events[2], click(640, 360));
        let bad_cases = [
            vec![],
            vec![InputEventDto {
                kind: "tap".into(),
                ..Default::default()
            }],
            vec![key("NoSuchKey")],
            vec![key("ab")],
            vec![click(1280, 0)],
            vec![click(0, 720)],
            vec![click(-1, 5)],
            vec![InputEventDto {
                kind: "wait".into(),
                ms: Some(2001),
                ..Default::default()
            }],
            vec![InputEventDto {
                kind: "key".into(),
                key: None,
                ..Default::default()
            }],
            vec![click(1, 1); MAX_EVENTS_PER_CALL + 1],
        ];
        for events in bad_cases {
            let refusal = input(&rig, events.clone()).expect_err("refused");
            assert_eq!(refusal.code, "bad_args", "{events:?}");
        }
        assert_eq!(
            rig.server.lock().expect("lock").web.inputs.len(),
            4,
            "refusals queue nothing"
        );
        // The queue is capped.
        for _ in 0..3 {
            input(&rig, vec![click(1, 1); MAX_EVENTS_PER_CALL]).expect("fills");
        }
        let over = input(&rig, vec![click(1, 1); MAX_EVENTS_PER_CALL]).expect_err("over cap");
        assert_eq!(over.code, "over_budget");
    }

    #[test]
    fn failures_are_typed_and_change_nothing() {
        let broken = rig("buildfail", 1, Browse::Drawn, true);
        let refusal = build(&broken, BuildWebArgs::default()).expect_err("build fails");
        assert_eq!(refusal.code, "build_failed");
        assert!(refusal.message.contains("E0432"), "{refusal}");
        assert!(broken.server.lock().expect("lock").web.builds.is_empty());
        assert_eq!(
            undo_input(&broken).expect_err("no history").code,
            "nothing_to_undo"
        );

        let blind = rig("noadapter", 0, Browse::NoAdapter, true);
        build(&blind, BuildWebArgs::default()).expect("build");
        serve(&blind, ServeWebArgs::default()).expect("serve");
        let none = capture(&blind).expect_err("no adapter");
        assert_eq!(none.code, "no_adapter");
        assert!(
            none.message.contains("no suitable graphics adapter"),
            "{none}"
        );

        let crashed = rig("crash", 0, Browse::Crash, true);
        build(&crashed, BuildWebArgs::default()).expect("build");
        serve(&crashed, ServeWebArgs::default()).expect("serve");
        assert_eq!(
            capture(&crashed).expect_err("chrome missing").code,
            "browse_failed"
        );

        let bad_label = rig("label", 0, Browse::Drawn, true);
        for commit in ["../x", "a b", "", "x;rm", &"a".repeat(41)] {
            let refusal = build(
                &bad_label,
                BuildWebArgs {
                    commit: Some(commit.to_owned()),
                    ..Default::default()
                },
            )
            .expect_err("label refused");
            assert_eq!(refusal.code, "bad_args", "{commit}");
        }
        assert_eq!(bad_label.fake.count("web-build.mjs"), 0);
        let out_of_range = BrowserCaptureTool::invoke(
            &bad_label.server,
            BrowserCaptureArgs {
                flag_set: Some(MAX_FLAG_SET + 1),
            },
        )
        .expect_err("flag set range");
        assert_eq!(out_of_range.code, "bad_args");
    }

    #[test]
    fn a_missing_tool_or_chrome_is_named() {
        let rig = rig("missing", 0, Browse::Drawn, false);
        std::fs::remove_file(rig.root.join("tools").join("web-build.mjs")).expect("remove");
        rig.server
            .allow_web(WebScope::new(&rig.root))
            .expect("allow without chrome");
        let refusal = build(&rig, BuildWebArgs::default()).expect_err("script missing");
        assert_eq!(refusal.code, "tool_missing");
        assert!(refusal.message.contains("web-build.mjs"), "{refusal}");
        std::fs::write(rig.root.join("tools").join("web-build.mjs"), "//").expect("restore");
        build(&rig, BuildWebArgs::default()).expect("build");
        serve(&rig, ServeWebArgs::default()).expect("serve");
        assert_eq!(
            capture(&rig).expect_err("no chrome in scope").code,
            "no_chrome"
        );
    }

    #[test]
    fn flag_set_is_passed_through_to_the_browse_tool() {
        let rig = rig("flagset", 0, Browse::Drawn, true);
        build(&rig, BuildWebArgs::default()).expect("build");
        serve(&rig, ServeWebArgs::default()).expect("serve");
        BrowserCaptureTool::invoke(&rig.server, BrowserCaptureArgs { flag_set: Some(6) })
            .expect("capture");
        let calls = rig.fake.calls.lock().expect("calls").clone();
        assert_eq!(
            Fake::arg(calls.last().expect("call"), "--only").as_deref(),
            Some("6")
        );
    }

    #[test]
    fn png_size_and_drawn_stats_read_what_the_tool_reports() {
        assert_eq!(png_size(&fake_png(1280, 720)), Some((1280, 720)));
        assert_eq!(png_size(b"not a png at all, sorry...."), None);
        assert_eq!(png_size(&[]), None);
        let drawn = serde_json::json!({"distinct": 81, "dominantShare": 0.77, "meanLuma": 131.8});
        assert!(stats_drawn(&drawn));
        for flat in [
            serde_json::json!({"distinct": 1, "dominantShare": 1.0, "meanLuma": 0.0}),
            serde_json::json!({"distinct": 81, "dominantShare": 0.99, "meanLuma": 100.0}),
            serde_json::json!({"distinct": 81, "dominantShare": 0.5, "meanLuma": 5.0}),
            Value::Null,
        ] {
            assert!(!stats_drawn(&flat), "{flat}");
        }
    }

    #[test]
    fn receipt_line_says_na_for_what_the_page_does_not_print_yet() {
        let ready = serde_json::json!({"commit": "c1", "backend": "BrowserWebGpu",
            "adapter": {"vendor": "nvidia", "architecture": "ampere"}});
        let line = receipt_line(&ready, &Value::Null);
        assert_eq!(
            line,
            "commit=c1 backend=BrowserWebGpu adapter=nvidia/ampere tier=na validation=na mem=na ttff_ms=na"
        );
        let line = receipt_line(&Value::Null, &Value::Null);
        assert!(line.contains("tier=na validation=na"), "{line}");
    }

    #[test]
    fn the_launch_runner_starts_one_real_child_with_a_cleared_environment() {
        // Uses the real process seam on `node --version`; skipped where node is not installed.
        let runner = LaunchRunner::default();
        match runner.run("node", &["--version".to_owned()]) {
            Ok(out) => {
                assert_eq!(out.code, 0, "{out:?}");
                assert!(out.stdout.trim().starts_with('v'), "{out:?}");
            }
            Err(why) => assert!(why.0.contains("spawn"), "{why:?}"),
        }
        let missing = runner
            .run("e2040-no-such-program-xyz", &[])
            .expect_err("no such program");
        assert!(missing.0.contains("spawn failed"), "{missing:?}");
    }
}
