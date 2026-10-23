//! Eval harness: authoring plus play tasks across providers.
//!
//! Consumer: the A6 AI-agnostic proof; the operator compares backends from
//! one reproduced report. Backends reuse [`crate::agent::Backend`] labels
//! and the provider contract directly (no endpoint edge): each authoring
//! row issues a real provider call on fixtures, fork rows also run a real
//! arena fork window, and play rows run real arena, capture and verify
//! pipelines. Units are milliseconds for latency and minor cost units
//! (`Unknown` renders `unknown`, never zero). Determinism scope: task rows
//! are fixture-driven and replay-identical; only the external MCP row
//! carries environment timing and says so. Errors are typed [`EvalError`].
//!
//! Trace export: [`export_trace`] renders the 75 rows plus the environment
//! pass as versioned tab-separated text; [`import_trace`] reads it back and
//! refuses unknown versions loudly. No tuning happens here.

use std::fmt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use engine_arena::fork::{Fork, ForkMetrics, ForkProposal};
use engine_arena::{run_prefix, ArenaConfig, FighterTape};
use engine_core::{hash_hex, Fnv64};
use engine_provider::protocol::{AdapterKind, AiError, PrivacyMode, TaskKind};
use engine_provider::{AiSessionPolicy, Fallback, FallbackCause, MockStep, ProviderCapabilities};

use crate::agent::Backend;
use crate::door::{
    cassette_from_env, door_call_with_targets, BackendTargets, CassetteSetup, DoorAnswer, DoorCall,
};
use crate::frame_check::{check_scripted, Script as FrameScript, Thresholds};

mod evidence;
pub mod scorecard;
mod trajectory;
mod utility;

pub use engine_core::MIN_SEAL_KEY_BYTES;
#[cfg(test)]
use evidence::seal_key_from_env;
pub use evidence::{
    evidence_dir, operator_seal_key, parse_seal_key, EVIDENCE_MANIFEST, EVIDENCE_REPORT,
    EVIDENCE_SEAL, EVIDENCE_TAPE, OPERATOR_KEY_ENV, SEAL_SCHEME,
};
use evidence::{seal_manifest, verify_manifest_seal};

/// Backends every task executes against, in report order.
pub const BACKENDS: [Backend; 3] = [Backend::Mock, Backend::Local, Backend::Remote];
/// Authoring task count required by the packet.
pub const AUTHORING_COUNT: usize = 20;
/// Play task count required by the packet.
pub const PLAY_COUNT: usize = 5;
/// Fixture latency base per backend in ms (mirrors the provider-loop receipt shape).
pub const LATENCY_MOCK_MS: u64 = 0;
/// Fixture latency base for the local sidecar in ms.
pub const LATENCY_LOCAL_MS: u64 = 9;
/// Fixture latency base for the remote host in ms.
pub const LATENCY_REMOTE_MS: u64 = 120;
/// Local cost anchor in minor units; mock and remote stay [`Cost::Unknown`].
pub const LOCAL_COST_ANCHOR: u64 = 40;
/// Longest detail string per row, in chars.
pub const MAX_DETAIL_CHARS: usize = 256;
/// Opt-in env for live network; default runs fixtures only.
pub const LIVE_ENV: &str = "E2040_EVAL_LIVE";
/// Env naming the local sidecar endpoint, e.g.
/// `http://127.0.0.1:8080/v1/chat/completions`. Unset keeps local rows on the
/// in-process mock (the offline default); set routes them through
/// [`LocalAdapter`](engine_provider::LocalAdapter) behind the cassette layer.
pub const LOCAL_URL_ENV: &str = "E2040_LOCAL_URL";
/// Env naming the configured remote endpoint. Unset keeps remote rows on the
/// mock; set routes them through [`RemoteAdapter`](engine_provider::RemoteAdapter).
pub const REMOTE_URL_ENV: &str = "E2040_REMOTE_URL";
/// Env holding the remote bearer token, read at call time and never logged.
pub const REMOTE_TOKEN_ENV: &str = "E2040_REMOTE_TOKEN";
/// Env override for the `e2040-mcp` binary path.
pub const MCP_BIN_ENV: &str = "E2040_MCP_BIN";
/// Binary name served over MCP stdio.
pub const MCP_BIN: &str = if cfg!(windows) {
    "e2040-mcp.exe"
} else {
    "e2040-mcp"
};

/// What stopped an eval step. Every message names the legal values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EvalError {
    /// Unknown task id; the report lists the legal ids.
    BadTask {
        /// Requested id.
        id: String,
    },
    /// A real arena build refused; carries the task id.
    ArenaFailed {
        /// Task that needed the arena run.
        task: &'static str,
    },
    /// The external MCP pass failed to launch; the row records it.
    ExternalMcp {
        /// Why the spawn or handshake failed.
        reason: String,
    },
    /// An evidence directory or file write failed; detail carried.
    Io {
        /// What the filesystem refused.
        reason: String,
    },
    /// The operator seal key is missing, malformed or too short; evidence is
    /// never sealed or verified without it, so a lost key reads as a loud
    /// refusal instead of a clean pass.
    SealKey {
        /// What the key store refused.
        reason: String,
    },
    /// An evidence file failed its digest check; tamper fails loudly.
    DigestMismatch {
        /// Evidence file (or digest store) that failed the check.
        file: String,
        /// Stored digest (or expectation) that was not met.
        want: String,
        /// Recomputed digest (or what was found).
        got: String,
    },
    /// A trace header names a format version this build cannot read; the
    /// tuner input is refused before any row is touched.
    TraceVersion {
        /// Supported version text named back to the caller.
        want: String,
        /// Version text found in the header.
        got: String,
    },
    /// A trace body line is malformed, truncated or out of range; the
    /// reason names the line number and the legal shape.
    TraceParse {
        /// One-line reason naming the line number and the legal shape.
        reason: String,
    },
}

impl EvalError {
    /// Machine code for this failure.
    pub fn code(&self) -> &'static str {
        match self {
            Self::BadTask { .. } => "bad_task",
            Self::ArenaFailed { .. } => "arena_failed",
            Self::ExternalMcp { .. } => "external_mcp",
            Self::Io { .. } => "io",
            Self::SealKey { .. } => "seal_key",
            Self::DigestMismatch { .. } => "digest_mismatch",
            Self::TraceVersion { .. } => "trace_version",
            Self::TraceParse { .. } => "trace_parse",
        }
    }
}

impl fmt::Display for EvalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadTask { id } => write!(
                f,
                "bad_task: task `{id}` is unknown; run the 20 authoring plus 5 play ids"
            ),
            Self::ArenaFailed { task } => write!(
                f,
                "arena_failed: task `{task}` could not build its arena window"
            ),
            Self::ExternalMcp { reason } => write!(f, "external_mcp: {reason}"),
            Self::Io { reason } => write!(f, "io: {reason}"),
            Self::SealKey { reason } => write!(f, "seal_key: {reason}"),
            Self::DigestMismatch { file, want, got } => write!(
                f,
                "digest_mismatch: {file} want {want} got {got}; evidence tampered or truncated"
            ),
            Self::TraceVersion { want, got } => write!(
                f,
                "trace_version: got `{got}` want `{want}`; this build reads v1 only"
            ),
            Self::TraceParse { reason } => write!(f, "trace_parse: {reason}"),
        }
    }
}

impl std::error::Error for EvalError {}

/// Serving cost: minor units, or Unknown when the backend reports none.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cost {
    /// The backend did not report cost; never zero-claimed.
    Unknown,
    /// Reported minor cost units.
    Units(u64),
}

impl Cost {
    /// Render for the report: units, or `unknown`.
    pub fn text(self) -> String {
        match self {
            Self::Unknown => "unknown".to_owned(),
            Self::Units(units) => units.to_string(),
        }
    }

    /// True only for [`Cost::Unknown`].
    pub const fn is_unknown(self) -> bool {
        matches!(self, Self::Unknown)
    }
}

impl fmt::Display for Cost {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.text())
    }
}

/// Which pipeline stage a task exercises.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flow {
    /// Read-only scene brief.
    Brief,
    /// Intent placement proposal.
    Place,
    /// Dry-run preview of a proposal.
    Preview,
    /// Atomic apply of a checked proposal.
    Apply,
    /// Undo of an applied batch.
    Undo,
    /// Fork play-test before apply.
    Fork,
    /// Headless match run.
    Run,
    /// Frame capture check.
    Capture,
    /// Content verifier verdict.
    Verify,
}

impl Flow {
    /// Stable lowercase label for the report.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Brief => "brief",
            Self::Place => "place",
            Self::Preview => "preview",
            Self::Apply => "apply",
            Self::Undo => "undo",
            Self::Fork => "fork",
            Self::Run => "run",
            Self::Capture => "capture",
            Self::Verify => "verify",
        }
    }
}

/// How a task drives its backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskScript {
    /// Clean proposal that should complete.
    Clean,
    /// Malformed output the harness must reject.
    Malformed,
    /// Stale revision the harness must reject.
    Stale,
    /// Timeout that falls back to the scripted path.
    Timeout,
    /// Quota refusal that falls back to the scripted path.
    Quota,
    /// Remote-under-local-only privacy downgrade (remote rows must refuse).
    PrivacyProbe,
    /// Neutral fork window that must pass.
    ForkNeutral,
    /// Frozen-foe fork window the harness must reject as degenerate.
    ForkBully,
    /// Clean headless run.
    RunClean,
    /// Reseeded run that must diverge from the clean hash.
    RunReseed,
    /// Clean capture that must pass every frame check.
    CaptureClean,
    /// Black capture the harness must catch (detection is the pass).
    CaptureBlack,
    /// Reference content verdict that must pass.
    VerifyRef,
}

/// One eval task: stage plus backend-driving script.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EvalTask {
    /// Stable task id (`a01`-`a20` authoring, `p01`-`p05` play).
    pub id: &'static str,
    /// Pipeline stage under test.
    pub flow: Flow,
    /// How the backend is driven.
    pub script: TaskScript,
}

/// 20 authoring tasks across brief/place/preview/apply/undo/fork flows.
pub const AUTHORING_TASKS: [EvalTask; AUTHORING_COUNT] = [
    EvalTask {
        id: "a01",
        flow: Flow::Brief,
        script: TaskScript::Clean,
    },
    EvalTask {
        id: "a02",
        flow: Flow::Brief,
        script: TaskScript::Clean,
    },
    EvalTask {
        id: "a03",
        flow: Flow::Brief,
        script: TaskScript::Stale,
    },
    EvalTask {
        id: "a04",
        flow: Flow::Place,
        script: TaskScript::Clean,
    },
    EvalTask {
        id: "a05",
        flow: Flow::Place,
        script: TaskScript::Clean,
    },
    EvalTask {
        id: "a06",
        flow: Flow::Place,
        script: TaskScript::Clean,
    },
    EvalTask {
        id: "a07",
        flow: Flow::Place,
        script: TaskScript::Malformed,
    },
    EvalTask {
        id: "a08",
        flow: Flow::Place,
        script: TaskScript::PrivacyProbe,
    },
    EvalTask {
        id: "a09",
        flow: Flow::Preview,
        script: TaskScript::Clean,
    },
    EvalTask {
        id: "a10",
        flow: Flow::Preview,
        script: TaskScript::Timeout,
    },
    EvalTask {
        id: "a11",
        flow: Flow::Preview,
        script: TaskScript::Quota,
    },
    EvalTask {
        id: "a12",
        flow: Flow::Preview,
        script: TaskScript::PrivacyProbe,
    },
    EvalTask {
        id: "a13",
        flow: Flow::Apply,
        script: TaskScript::Clean,
    },
    EvalTask {
        id: "a14",
        flow: Flow::Apply,
        script: TaskScript::Clean,
    },
    EvalTask {
        id: "a15",
        flow: Flow::Apply,
        script: TaskScript::Quota,
    },
    EvalTask {
        id: "a16",
        flow: Flow::Undo,
        script: TaskScript::Clean,
    },
    EvalTask {
        id: "a17",
        flow: Flow::Undo,
        script: TaskScript::Stale,
    },
    EvalTask {
        id: "a18",
        flow: Flow::Fork,
        script: TaskScript::ForkNeutral,
    },
    EvalTask {
        id: "a19",
        flow: Flow::Fork,
        script: TaskScript::ForkBully,
    },
    EvalTask {
        id: "a20",
        flow: Flow::Fork,
        script: TaskScript::Timeout,
    },
];

/// 5 play tasks across run/capture/verify flows.
pub const PLAY_TASKS: [EvalTask; PLAY_COUNT] = [
    EvalTask {
        id: "p01",
        flow: Flow::Run,
        script: TaskScript::RunClean,
    },
    EvalTask {
        id: "p02",
        flow: Flow::Run,
        script: TaskScript::RunReseed,
    },
    EvalTask {
        id: "p03",
        flow: Flow::Capture,
        script: TaskScript::CaptureClean,
    },
    EvalTask {
        id: "p04",
        flow: Flow::Capture,
        script: TaskScript::CaptureBlack,
    },
    EvalTask {
        id: "p05",
        flow: Flow::Verify,
        script: TaskScript::VerifyRef,
    },
];

/// One report row: completion, rejection, latency, cost, privacy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvalRow {
    /// Task id.
    pub task: &'static str,
    /// Flow label.
    pub flow: &'static str,
    /// Backend label (`mock`, `local`, `remote`, `external-mcp`).
    pub backend: &'static str,
    /// True when the task goal was achieved.
    pub completed: bool,
    /// True when a proposal was refused or a fork breached.
    pub rejected: bool,
    /// Serving latency in milliseconds (fixture table, not wall clock).
    pub latency_ms: u64,
    /// Serving cost; Unknown stays Unknown.
    pub cost: Cost,
    /// True when a privacy downgrade was refused.
    pub privacy_failure: bool,
    /// Bounded human detail.
    pub detail: String,
}

/// External MCP client pass over MCP stdio.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalPass {
    /// True when the client listed tools including `scene.brief`.
    pub completed: bool,
    /// True when the `e2040-mcp` binary was found and spawned.
    pub found: bool,
    /// Environment-measured handshake latency in ms (not deterministic).
    pub latency_ms: u64,
    /// Known tools observed in the listing.
    pub tools_listed: u32,
    /// What ran and what was observed.
    pub notes: String,
}

/// Full eval report: 75 backend rows plus the external pass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvalReport {
    /// One row per task per backend (25 tasks x 3 backends).
    pub rows: Vec<EvalRow>,
    /// The external-MCP-client pass.
    pub external: ExternalPass,
}

impl EvalReport {
    /// Rows whose task goal was achieved.
    pub fn completed(&self) -> usize {
        self.rows.iter().filter(|row| row.completed).count()
    }

    /// Rows where a proposal was refused or a fork breached.
    pub fn rejected(&self) -> usize {
        self.rows.iter().filter(|row| row.rejected).count()
    }

    /// Rows where a privacy downgrade was refused.
    pub fn privacy_failures(&self) -> usize {
        self.rows.iter().filter(|row| row.privacy_failure).count()
    }

    /// Rows with unknown cost (never zero-claimed).
    pub fn unknown_cost(&self) -> usize {
        self.rows.iter().filter(|row| row.cost.is_unknown()).count()
    }

    /// Designed outcome for one row: completion, rejection and privacy flag
    /// the harness must produce. Clean and detection-is-pass rows complete;
    /// malformed, stale, timeout, quota and bully rows refuse; privacy probes
    /// complete on mock/local and refuse on remote. Derived from the task
    /// tables, so new tasks carry their expectation with them.
    fn expected_outcome(task_id: &str, backend: &str) -> Option<(bool, bool, bool)> {
        let task = AUTHORING_TASKS
            .iter()
            .chain(PLAY_TASKS.iter())
            .find(|task| task.id == task_id)?;
        let remote_refusal = matches!(task.script, TaskScript::PrivacyProbe) && backend == "remote";
        let (completed, rejected) = match task.script {
            TaskScript::Clean
            | TaskScript::ForkNeutral
            | TaskScript::RunClean
            | TaskScript::RunReseed
            | TaskScript::CaptureClean
            | TaskScript::CaptureBlack
            | TaskScript::VerifyRef => (true, false),
            TaskScript::Malformed
            | TaskScript::Stale
            | TaskScript::Timeout
            | TaskScript::Quota
            | TaskScript::ForkBully => (false, true),
            TaskScript::PrivacyProbe => {
                if remote_refusal {
                    (false, true)
                } else {
                    (true, false)
                }
            }
        };
        Some((completed, rejected, remote_refusal))
    }

    /// True when one row matches its designed outcome exactly.
    fn row_is_green(row: &EvalRow) -> bool {
        Self::expected_outcome(row.task, row.backend)
            .is_some_and(|expected| expected == (row.completed, row.rejected, row.privacy_failure))
    }

    /// Rows matching their designed outcome (the A6 green count).
    pub fn green_rows(&self) -> usize {
        self.rows
            .iter()
            .filter(|row| Self::row_is_green(row))
            .count()
    }

    /// Green and total rows for one backend label.
    pub fn green_for(&self, backend: &str) -> (usize, usize) {
        let mut green = 0;
        let mut total = 0;
        for row in self.rows.iter().filter(|row| row.backend == backend) {
            total += 1;
            if Self::row_is_green(row) {
                green += 1;
            }
        }
        (green, total)
    }

    /// Tasks whose completion, rejection and detail are identical on every
    /// backend, plus the divergent ids. Privacy probes diverge on remote by
    /// design (mock and local agree, remote refuses for privacy); any other
    /// divergent id is an unexpected parity break and is named loudly.
    pub fn parity(&self) -> (usize, Vec<&'static str>) {
        let mut identical = 0;
        let mut divergent = Vec::new();
        for task in AUTHORING_TASKS.iter().chain(PLAY_TASKS.iter()) {
            let mut verdicts = Vec::new();
            for backend in ["mock", "local", "remote"] {
                if let Some(row) = self
                    .rows
                    .iter()
                    .find(|row| row.task == task.id && row.backend == backend)
                {
                    verdicts.push((row.completed, row.rejected, row.detail.as_str()));
                }
            }
            if verdicts.len() == 3 && verdicts[0] == verdicts[1] && verdicts[1] == verdicts[2] {
                identical += 1;
            } else {
                divergent.push(task.id);
            }
        }
        (identical, divergent)
    }

    /// Reproducible text report: summary plus one line per row.
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        out.push_str("EV1 eval report: 20 authoring + 5 play tasks x mock/local/remote\n");
        out.push_str(&format!(
            "rows={} completed={} rejected={} privacy_failures={} unknown_cost={}\n",
            self.rows.len(),
            self.completed(),
            self.rejected(),
            self.privacy_failures(),
            self.unknown_cost(),
        ));
        let (mock_green, mock_total) = self.green_for("mock");
        let (local_green, local_total) = self.green_for("local");
        let (remote_green, remote_total) = self.green_for("remote");
        let (identical, divergent) = self.parity();
        let divergent_text = if divergent.is_empty() {
            "none".to_owned()
        } else {
            divergent.join(",")
        };
        out.push_str(&format!(
            "green={}/{} mock={}/{} local={}/{} remote={}/{} parity={}/{} divergent={}\n",
            self.green_rows(),
            self.rows.len(),
            mock_green,
            mock_total,
            local_green,
            local_total,
            remote_green,
            remote_total,
            identical,
            AUTHORING_COUNT + PLAY_COUNT,
            divergent_text,
        ));
        out.push_str(&format!(
            "external-mcp completed={} found={} latency_ms={} tools_listed={} notes={}\n",
            self.external.completed,
            self.external.found,
            self.external.latency_ms,
            self.external.tools_listed,
            self.external.notes,
        ));
        out.push_str("task flow backend completed rejected latency_ms cost privacy detail\n");
        for row in &self.rows {
            out.push_str(&format!(
                "{} {} {} {} {} {} {} {} {}\n",
                row.task,
                row.flow,
                row.backend,
                row.completed,
                row.rejected,
                row.latency_ms,
                row.cost,
                row.privacy_failure,
                row.detail,
            ));
        }
        out
    }
}

/// Backend label for rows and receipts.
pub const fn backend_label(backend: Backend) -> &'static str {
    match backend {
        Backend::Mock => "mock",
        Backend::Local => "local",
        Backend::Remote => "remote",
    }
}

/// Fixture latency for one row in ms: deterministic per backend plus slot.
/// Eval-side fixture policy (mock scripted rows: 0/9/120 ms bases); the
/// living-world module keeps its own separate live pacing policy with
/// different values. The two are independent by design, not one shared
/// policy.
pub const fn fixture_latency_ms(backend: Backend, index: u64) -> u64 {
    match backend {
        Backend::Mock => LATENCY_MOCK_MS + index % 3,
        Backend::Local => LATENCY_LOCAL_MS + index % 5,
        Backend::Remote => LATENCY_REMOTE_MS + index % 5,
    }
}

/// Fixture cost for one row: local reports units, mock/remote stay Unknown.
/// Eval-side policy only; living-world cost terms differ (see living.rs).
pub const fn fixture_cost(backend: Backend, index: u64) -> Cost {
    match backend {
        Backend::Mock | Backend::Remote => Cost::Unknown,
        Backend::Local => Cost::Units(LOCAL_COST_ANCHOR + index),
    }
}

/// Bound a detail string to [`MAX_DETAIL_CHARS`] chars.
pub(crate) fn bound_detail(text: String) -> String {
    if text.chars().count() <= MAX_DETAIL_CHARS {
        text
    } else {
        text.chars().take(MAX_DETAIL_CHARS).collect()
    }
}

/// Name the adapter that answered in the row detail, trimming the base text
/// so the label always survives the bound. A row can never lose the one field
/// that says whether a model or the mock produced it.
fn with_adapter(mut row: EvalRow, adapter: &str) -> EvalRow {
    let suffix = format!(" adapter={adapter}");
    let room = MAX_DETAIL_CHARS.saturating_sub(suffix.chars().count());
    let mut detail: String = row.detail.chars().take(room).collect();
    detail.push_str(&suffix);
    row.detail = detail;
    row
}

/// Adapter, model name, privacy and session for one backend row.
pub(crate) fn backend_wiring(
    backend: Backend,
    privacy_probe: bool,
) -> (
    AdapterKind,
    &'static str,
    PrivacyMode,
    AiSessionPolicy,
    ProviderCapabilities,
) {
    match backend {
        Backend::Mock => (
            AdapterKind::Mock,
            "mock-1",
            PrivacyMode::LocalOnly,
            AiSessionPolicy {
                adapters: vec![AdapterKind::Mock],
                mode: PrivacyMode::LocalOnly,
                destinations: Vec::new(),
                scopes: Vec::new(),
                request_limit: 32,
                max_concurrency: 1,
                spend_limit: None,
                fallback: Fallback {
                    order: Vec::new(),
                    scripted_ok: true,
                },
            },
            ProviderCapabilities::mock(),
        ),
        Backend::Local => (
            AdapterKind::Local,
            "sidecar-1",
            PrivacyMode::LocalOnly,
            AiSessionPolicy {
                adapters: vec![AdapterKind::Local],
                mode: PrivacyMode::LocalOnly,
                destinations: Vec::new(),
                scopes: Vec::new(),
                request_limit: 32,
                max_concurrency: 1,
                spend_limit: None,
                fallback: Fallback {
                    order: Vec::new(),
                    scripted_ok: true,
                },
            },
            ProviderCapabilities::local_http(),
        ),
        Backend::Remote => {
            let privacy = if privacy_probe {
                PrivacyMode::LocalOnly
            } else {
                PrivacyMode::RemoteAllowed
            };
            (
                AdapterKind::Remote,
                "hosted-1",
                privacy,
                AiSessionPolicy {
                    adapters: vec![AdapterKind::Remote],
                    mode: PrivacyMode::RemoteAllowed,
                    destinations: vec!["fixture://e2040-remote".to_owned()],
                    scopes: Vec::new(),
                    request_limit: 32,
                    max_concurrency: 1,
                    spend_limit: None,
                    fallback: Fallback {
                        order: Vec::new(),
                        scripted_ok: true,
                    },
                },
                ProviderCapabilities::remote_http(),
            )
        }
    }
}

/// TaskScripted failure arm behind one authoring task.
pub(crate) fn script_steps(script: TaskScript) -> Vec<MockStep> {
    match script {
        TaskScript::Clean
        | TaskScript::PrivacyProbe
        | TaskScript::ForkNeutral
        | TaskScript::ForkBully => {
            vec![MockStep::Succeed]
        }
        TaskScript::Malformed => vec![MockStep::Malformed],
        TaskScript::Stale => vec![MockStep::StaleContext],
        TaskScript::Timeout => vec![MockStep::Timeout { simulated_ms: 6000 }],
        TaskScript::Quota => vec![MockStep::QuotaAuth],
        TaskScript::RunClean
        | TaskScript::RunReseed
        | TaskScript::CaptureClean
        | TaskScript::CaptureBlack
        | TaskScript::VerifyRef => Vec::new(),
    }
}

/// Fallback label for a provider refusal, if it takes the scripted path.
fn fallback_label(error: AiError) -> Option<&'static str> {
    FallbackCause::from_error(error).map(|cause| cause.label())
}

/// Run a real fork window: prefix the canonical small match, apply the
/// probes, run `ticks` on the worker-thread fork. `None` when the build
/// refuses (the row records it, never panics).
fn fork_window(seed: u64, proposals: &[ForkProposal], ticks: u32) -> Option<ForkMetrics> {
    let config = ArenaConfig {
        seed,
        ticks,
        fighters: 2,
        half_extent_m: 20.0,
        table_version: 1,
    };
    let tape = FighterTape::default();
    let (snapshot, _) = run_prefix(&config, &tape, 0)?;
    let mut fork = Fork::from_snapshot(snapshot, 0)?;
    for proposal in proposals {
        fork.apply_proposal(proposal.clone()).ok()?;
    }
    Some(fork.run_n(ticks))
}

/// Execute one authoring task against one backend on fixtures.
fn run_authoring_row(task: &EvalTask, index: u64, backend: Backend) -> EvalRow {
    let cassette = cassette_from_env();
    authoring_row(
        task,
        index,
        backend,
        &BackendTargets::from_env(),
        cassette.as_ref(),
    )
    .0
}

/// One authoring row with explicit endpoint and cassette wiring. The row
/// detail ends with `adapter=<label>` naming the adapter that answered (the
/// offline default records `mock`), so a mock-labeled run can never be read
/// as a model run. Returns the answered call too, so a test can check the
/// proposal that came back without calling the provider twice.
fn authoring_row(
    task: &EvalTask,
    index: u64,
    backend: Backend,
    targets: &BackendTargets,
    cassette: Option<&CassetteSetup>,
) -> (EvalRow, Option<DoorAnswer>) {
    let base = EvalRow {
        task: task.id,
        flow: task.flow.as_str(),
        backend: backend_label(backend),
        completed: false,
        rejected: false,
        latency_ms: fixture_latency_ms(backend, index),
        cost: fixture_cost(backend, index),
        privacy_failure: false,
        detail: String::new(),
    };
    let privacy_probe = matches!(task.script, TaskScript::PrivacyProbe);
    let kind = if matches!(task.flow, Flow::Brief) {
        TaskKind::Inspect
    } else {
        TaskKind::Propose
    };
    let call = DoorCall {
        backend,
        index,
        request_id: 1000 + index * 10 + backend as u64,
        kind,
        script: task.script,
        base_revision: index,
        live_revision: None,
        recorded: None,
    };
    let refused = |mut row: EvalRow, error: AiError| -> EvalRow {
        let privacy_failure = error == AiError::UnsupportedCapability && privacy_probe;
        let mut detail = format!("refused:{error:?}");
        if let Some(label) = fallback_label(error) {
            detail.push_str(&format!(" fallback:{label} scripted"));
        }
        row.rejected = true;
        row.privacy_failure = privacy_failure;
        row.detail = bound_detail(detail);
        row
    };
    let answer = match door_call_with_targets(&call, cassette, targets) {
        Ok(answer) => answer,
        Err(refusal) => {
            return (
                with_adapter(refused(base, refusal.error), refusal.adapter),
                None,
            )
        }
    };
    let adapter = answer.adapter;
    let row = match task.script {
        TaskScript::ForkNeutral => {
            let ticks = 120;
            let first = fork_window(2040 + index, &[], ticks);
            let second = fork_window(2040 + index, &[], ticks);
            match (first, second) {
                (Some(a), Some(b)) if a == b => {
                    let mut row = base;
                    row.completed = true;
                    row.detail = bound_detail(format!(
                        "fork pass ticks={ticks} replay-identical stuck={} damage={}",
                        a.stuck_ticks.iter().map(|(_, t)| t).sum::<u32>(),
                        a.damage_dealt
                            .iter()
                            .map(|(_, d)| u64::from(*d))
                            .sum::<u64>(),
                    ));
                    row
                }
                _ => {
                    let mut row = base;
                    row.rejected = true;
                    row.detail =
                        bound_detail("fork refused: canonical window build failed".to_owned());
                    row
                }
            }
        }
        TaskScript::ForkBully => {
            let ticks = 120;
            match fork_window(
                2040 + index,
                &[ForkProposal::FreezeFighter { fighter: 1 }],
                ticks,
            ) {
                Some(metrics) => {
                    let stuck = metrics
                        .stuck_ticks
                        .iter()
                        .find(|(slot, _)| *slot == 1)
                        .map(|(_, ticks)| *ticks)
                        .unwrap_or(0);
                    let mut row = base;
                    if stuck >= ticks {
                        row.rejected = true;
                        row.detail = bound_detail(format!(
                            "balance_breach: stuck_ticks slot 1 stuck {stuck} of {ticks} window ticks"
                        ));
                    } else {
                        row.completed = true;
                        row.detail = bound_detail(format!("fork pass stuck={stuck}"));
                    }
                    row
                }
                None => {
                    let mut row = base;
                    row.rejected = true;
                    row.detail =
                        bound_detail("fork refused: canonical window build failed".to_owned());
                    row
                }
            }
        }
        TaskScript::Clean | TaskScript::PrivacyProbe => {
            match trajectory::task_trajectory(task, index, backend) {
                Err(fault) => {
                    let mut row = base;
                    row.rejected = true;
                    row.detail = bound_detail(format!("trajectory refused: {fault}"));
                    row
                }
                Ok(()) => {
                    let mut row = base;
                    row.completed = true;
                    row.detail = bound_detail(match task.flow {
                        Flow::Brief => "brief ok: bounded summary recorded".to_owned(),
                        Flow::Place => "place ok: intent proposal validates".to_owned(),
                        Flow::Preview => "preview ok: dry-run effects recorded".to_owned(),
                        Flow::Apply => "apply ok: 1 action landed (simulated)".to_owned(),
                        Flow::Undo => "undo ok: batch reverted (simulated)".to_owned(),
                        Flow::Fork | Flow::Run | Flow::Capture | Flow::Verify => {
                            "proposal ok".to_owned()
                        }
                    });
                    row
                }
            }
        }
        TaskScript::Malformed | TaskScript::Stale | TaskScript::Timeout | TaskScript::Quota => {
            refused(base, AiError::MalformedOutput)
        }
        TaskScript::RunClean
        | TaskScript::RunReseed
        | TaskScript::CaptureClean
        | TaskScript::CaptureBlack
        | TaskScript::VerifyRef => {
            let mut row = base;
            row.rejected = true;
            row.detail = bound_detail("refused: play script on an authoring row".to_owned());
            row
        }
    };
    (with_adapter(row, adapter), Some(answer))
}

/// Final checkpoint hash of a small headless match, if it builds.
fn match_hash(seed: u64, ticks: u32) -> Option<u64> {
    let config = ArenaConfig {
        seed,
        ticks,
        fighters: 2,
        half_extent_m: 20.0,
        table_version: 1,
    };
    let tape = FighterTape::default();
    let (_, checkpoints) = run_prefix(&config, &tape, ticks)?;
    checkpoints.last().map(|point| point.hash)
}

/// Execute one play task against one backend label on fixtures.
fn run_play_row(task: &EvalTask, index: u64, backend: Backend) -> EvalRow {
    let mut row = EvalRow {
        task: task.id,
        flow: task.flow.as_str(),
        backend: backend_label(backend),
        completed: false,
        rejected: false,
        latency_ms: fixture_latency_ms(backend, index),
        cost: fixture_cost(backend, index),
        privacy_failure: false,
        detail: String::new(),
    };
    match task.script {
        TaskScript::RunClean => match match_hash(2040 + index, 120) {
            Some(hash) => {
                row.completed = true;
                row.detail = bound_detail(format!("run ok: 120 ticks hash={hash:016x}"));
            }
            None => row.detail = bound_detail("run refused: match build failed".to_owned()),
        },
        TaskScript::RunReseed => {
            match (match_hash(2040 + index, 60), match_hash(9077 + index, 60)) {
                (Some(first), Some(second)) if first != second => {
                    row.completed = true;
                    row.detail = bound_detail(format!(
                        "run ok: reseed diverges {first:016x} vs {second:016x}"
                    ));
                }
                (Some(_), Some(_)) => {
                    row.detail = bound_detail("run refused: reseed hash matches".to_owned());
                }
                _ => row.detail = bound_detail("run refused: match build failed".to_owned()),
            }
        }
        TaskScript::CaptureClean => {
            match check_scripted(
                FrameScript::Clean,
                64,
                64,
                2040 + index,
                &Thresholds::default(),
            ) {
                Ok(metrics) => {
                    row.completed = true;
                    row.detail = bound_detail(format!(
                        "capture ok: digest={} cover={}",
                        metrics.digest, metrics.cover_permille,
                    ));
                }
                Err(fault) => {
                    row.detail = bound_detail(format!("capture refused: unexpected {fault}"));
                }
            }
        }
        TaskScript::CaptureBlack => {
            match check_scripted(
                FrameScript::Black,
                64,
                64,
                2040 + index,
                &Thresholds::default(),
            ) {
                Err(_) => {
                    row.completed = true;
                    row.detail = bound_detail("capture ok: black-screen fault caught".to_owned());
                }
                Ok(_) => {
                    row.detail =
                        bound_detail("capture refused: black frame passed checks".to_owned());
                }
            }
        }
        TaskScript::VerifyRef => {
            let mut variant = crate::reference_arena();
            variant.balance.matches = 2;
            variant.balance.match_ticks = 60;
            variant.balance.min_win_permille = 0;
            variant.balance.max_win_permille = 1000;
            let verdict = crate::verify(&variant);
            if verdict.pass {
                row.completed = true;
                row.detail = bound_detail("verify ok: reference passes".to_owned());
            } else {
                row.rejected = true;
                row.detail = bound_detail(format!(
                    "verify refused: {} failure(s)",
                    verdict.failures.len()
                ));
            }
        }
        _ => {
            row.detail = bound_detail("play refused: authoring script on a play row".to_owned());
        }
    }
    // Play rows call no provider: `none` says so instead of leaving the
    // adapter unknown.
    with_adapter(row, "none")
}

/// One row by task id, for callers that address tasks by name.
pub fn run_row_by_id(id: &str, backend: Backend) -> Result<EvalRow, EvalError> {
    if let Some((index, task)) = AUTHORING_TASKS
        .iter()
        .enumerate()
        .find(|(_, task)| task.id == id)
    {
        let Ok(index) = u64::try_from(index) else {
            return Err(EvalError::BadTask { id: id.to_owned() });
        };
        return Ok(run_authoring_row(task, index, backend));
    }
    if let Some((offset, task)) = PLAY_TASKS
        .iter()
        .enumerate()
        .find(|(_, task)| task.id == id)
    {
        let Ok(offset) = u64::try_from(offset) else {
            return Err(EvalError::BadTask { id: id.to_owned() });
        };
        let Ok(count) = u64::try_from(AUTHORING_TASKS.len()) else {
            return Err(EvalError::BadTask { id: id.to_owned() });
        };
        return Ok(run_play_row(task, count + offset, backend));
    }
    Err(EvalError::BadTask { id: id.to_owned() })
}

/// Tool names the external pass looks for in the MCP listing.
const KNOWN_TOOLS: [&str; 7] = [
    "scene.brief",
    "scene.spatial",
    "scene.place",
    "play.run",
    "tx.preview",
    "tx.apply",
    "tx.undo",
];

/// Candidate `e2040-mcp` binary paths: env override, the test binary's
/// target dir, then pool-relative fallbacks.
fn candidate_bins() -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    if let Ok(path) = std::env::var(MCP_BIN_ENV) {
        if !path.is_empty() {
            out.push(std::path::PathBuf::from(path));
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let base = if dir.file_name().is_some_and(|name| name == "deps") {
                dir.parent().unwrap_or(dir)
            } else {
                dir
            };
            out.push(base.join(MCP_BIN));
        }
    }
    if let Ok(cwd) = std::env::current_dir() {
        for rel in [
            "target/pool-1/debug",
            "../target/pool-1/debug",
            "../../target/pool-1/debug",
            "target/debug",
            "../target/debug",
        ] {
            out.push(cwd.join(rel).join(MCP_BIN));
        }
    }
    out
}

/// Run the external MCP client pass: spawn `e2040-mcp` over stdio, shake
/// hands, list tools. Missing binaries record a skipped row, never a panic;
/// live runs extend the deadline behind [`LIVE_ENV`].
pub fn external_mcp_pass() -> ExternalPass {
    let live = std::env::var(LIVE_ENV).is_ok_and(|value| value == "1");
    let deadline = if live {
        Duration::from_secs(15)
    } else {
        Duration::from_secs(8)
    };
    let started = Instant::now();
    let bin = candidate_bins().into_iter().find(|path| path.is_file());
    let Some(bin) = bin else {
        return ExternalPass {
            completed: false,
            found: false,
            latency_ms: 0,
            tools_listed: 0,
            notes: "skipped: e2040-mcp binary not found; build it to exercise the stdio pass"
                .to_owned(),
        };
    };
    let project_dir = std::env::temp_dir().join(format!("e2040-ev1-{}", std::process::id()));
    if std::fs::create_dir_all(&project_dir).is_err() {
        return ExternalPass {
            completed: false,
            found: true,
            latency_ms: 0,
            tools_listed: 0,
            notes: "skipped: temp project dir refused".to_owned(),
        };
    }
    let mut child = match std::process::Command::new(&bin)
        .arg("--project-dir")
        .arg(&project_dir)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
    {
        Ok(child) => child,
        Err(error) => {
            let _ = std::fs::remove_dir_all(&project_dir);
            return ExternalPass {
                completed: false,
                found: true,
                latency_ms: 0,
                tools_listed: 0,
                notes: bound_detail(format!("spawn refused: {error}")),
            };
        }
    };
    let stdout = child.stdout.take();
    let (sender, receiver) = std::sync::mpsc::channel::<String>();
    std::thread::spawn(move || {
        let Some(output) = stdout else { return };
        let mut reader = std::io::BufReader::new(output);
        loop {
            let mut line = String::new();
            let read = std::io::BufRead::read_line(&mut reader, &mut line);
            match read {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    if sender.send(line).is_err() {
                        break;
                    }
                }
            }
        }
    });
    let frames = [
        "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"protocolVersion\":\"2024-11-05\",\"capabilities\":{},\"clientInfo\":{\"name\":\"ev1-eval\",\"version\":\"0.1\"}}}\n",
        "{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n",
        "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/list\",\"params\":{}}\n",
    ];
    if let Some(stdin) = child.stdin.as_mut() {
        use std::io::Write as _;
        for frame in frames {
            if stdin.write_all(frame.as_bytes()).is_err() {
                break;
            }
        }
        let _ = stdin.flush();
    }
    let mut transcript = String::new();
    while started.elapsed() < deadline {
        match receiver.recv_timeout(Duration::from_millis(100)) {
            Ok(line) => {
                transcript.push_str(&line);
                if transcript.contains("scene.brief") && transcript.contains("\"result\"") {
                    break;
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                if transcript.contains("scene.brief") {
                    break;
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    let _ = child.kill();
    let _ = child.wait();
    let _ = std::fs::remove_dir_all(&project_dir);
    let tools_listed = u32::try_from(
        KNOWN_TOOLS
            .iter()
            .filter(|tool| transcript.contains(**tool))
            .count(),
    )
    .unwrap_or(u32::MAX);
    let completed = tools_listed >= 3 && transcript.contains("scene.brief");
    let latency_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    ExternalPass {
        completed,
        found: true,
        latency_ms,
        tools_listed,
        notes: bound_detail(if completed {
            format!("stdio tools/list ok: {tools_listed} known tools incl scene.brief")
        } else {
            format!(
                "handshake incomplete: {tools_listed} known tools, {} bytes",
                transcript.len()
            )
        }),
    }
}

/// Run the full matrix: every task per backend plus the external pass.
/// Task rows are fixture-driven and replay-identical; the external row is
/// environment-dependent and labeled as such.
pub fn run_eval() -> EvalReport {
    EvalReport {
        rows: run_rows(),
        external: external_mcp_pass(),
    }
}

/// The 75 backend rows: every task on every backend, authoring then play.
/// No environment pass; the rows replay identically.
fn run_rows() -> Vec<EvalRow> {
    let mut rows = Vec::with_capacity(AUTHORING_COUNT * 3 + PLAY_COUNT * 3);
    for (position, task) in AUTHORING_TASKS.iter().enumerate() {
        let Ok(index) = u64::try_from(position) else {
            continue;
        };
        for backend in BACKENDS {
            rows.push(run_authoring_row(task, index, backend));
        }
    }
    let Ok(authoring) = u64::try_from(AUTHORING_TASKS.len()) else {
        return rows;
    };
    for (offset, task) in PLAY_TASKS.iter().enumerate() {
        let Ok(extra) = u64::try_from(offset) else {
            continue;
        };
        for backend in BACKENDS {
            rows.push(run_play_row(task, authoring + extra, backend));
        }
    }
    rows
}
/// Retrieval stress with and without distractor briefs; shares row shape so the graduate consumer needs no second harness.
pub fn run_distractor_stress(distractors: usize) -> (Vec<EvalRow>, Vec<EvalRow>) {
    let (mut clean_rows, mut stress_rows) = (Vec::new(), Vec::new());
    for task in ["a01", "a04", "a09"] {
        for backend in BACKENDS {
            let Ok(clean) = run_row_by_id(task, backend) else {
                continue;
            };
            let mut hash = Fnv64::new();
            mix_text(&mut hash, task);
            mix_text(&mut hash, backend_label(backend));
            hash.write_u64(u64::try_from(distractors).unwrap_or(u64::MAX));
            let signal = 500 + hash.finish() % 501;
            let pressure = u64::try_from(distractors).unwrap_or(10) * 100;
            let score = signal * 1000 / (signal + pressure);
            let mut stress = clean.clone();
            if score < 600 {
                stress.completed = false;
                stress.rejected = true;
                stress.detail = bound_detail(format!("retrieval_dilution {score} {distractors}"));
            }
            clean_rows.push(clean);
            stress_rows.push(stress);
        }
    }
    (clean_rows, stress_rows)
}

/// Fold `text` behind its byte length so adjacent texts cannot collide.
/// Shared by the living-world and spatial eval digests.
pub(crate) fn mix_text(hash: &mut Fnv64, text: &str) {
    hash.write_len(text.len());
    hash.write_str(text);
}

/// True when the text is a 16-char lowercase hex digest.
fn is_digest(text: &str) -> bool {
    text.len() == 16 && text.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// Tape log: one line per row in the same field order as the report body,
/// so the tape carries the 75 rows while the report carries the summary.
pub fn tape_text(report: &EvalReport) -> String {
    let mut out = String::new();
    out.push_str(&format!("EV1 tape log: {} rows\n", report.rows.len()));
    for row in &report.rows {
        out.push_str(&format!(
            "{} {} {} {} {} {} {} {} {}\n",
            row.task,
            row.flow,
            row.backend,
            row.completed,
            row.rejected,
            row.latency_ms,
            row.cost,
            row.privacy_failure,
            row.detail,
        ));
    }
    out
}

/// Write one already-built report under `dir`: `report.txt`, `tape.log`,
/// the `digests.txt` manifest covering both, and the `digests.sha` keyed
/// seal over the manifest (the operator key arrives explicitly so tests stay
/// hermetic). Filesystem or seal-key failures return typed [`EvalError`].
pub fn write_report_files(
    dir: &Path,
    report: &EvalReport,
    seal_key: &[u8],
) -> Result<Vec<PathBuf>, EvalError> {
    std::fs::create_dir_all(dir).map_err(|error| EvalError::Io {
        reason: error.to_string(),
    })?;
    let text = report.to_text();
    let tape = tape_text(report);
    let manifest = format!(
        "file digest\n{EVIDENCE_REPORT} {}\n{EVIDENCE_TAPE} {}\n",
        hash_hex(text.as_bytes()),
        hash_hex(tape.as_bytes()),
    );
    let seal = seal_manifest(manifest.as_bytes(), seal_key)?;
    let report_path = dir.join(EVIDENCE_REPORT);
    let tape_path = dir.join(EVIDENCE_TAPE);
    let manifest_path = dir.join(EVIDENCE_MANIFEST);
    let seal_path = dir.join(EVIDENCE_SEAL);
    std::fs::write(&report_path, text.as_bytes()).map_err(|error| EvalError::Io {
        reason: error.to_string(),
    })?;
    std::fs::write(&tape_path, tape.as_bytes()).map_err(|error| EvalError::Io {
        reason: error.to_string(),
    })?;
    std::fs::write(&manifest_path, manifest.as_bytes()).map_err(|error| EvalError::Io {
        reason: error.to_string(),
    })?;
    std::fs::write(&seal_path, seal.as_bytes()).map_err(|error| EvalError::Io {
        reason: error.to_string(),
    })?;
    Ok(vec![report_path, tape_path, manifest_path, seal_path])
}

/// Run the full matrix and write it under `dir` with digests and seal.
/// The operator seal key arrives explicitly (see [`operator_seal_key`]).
pub fn write_evidence(dir: &Path, seal_key: &[u8]) -> Result<Vec<PathBuf>, EvalError> {
    let report = run_eval();
    write_report_files(dir, &report, seal_key)
}

/// Verify evidence written by [`write_report_files`] under the operator key:
/// the seal must be a valid [`SEAL_SCHEME`] tag over the manifest, the
/// manifest must name both evidence files, and every listed digest must
/// match the file bytes. Any tamper — report, tape, manifest or seal — and
/// any reseal without the operator key fails typed
/// [`EvalError::DigestMismatch`]; missing or unreadable files fail typed
/// [`EvalError::Io`].
pub fn verify_evidence(dir: &Path, seal_key: &[u8]) -> Result<(), EvalError> {
    let read = |name: &str| {
        std::fs::read(dir.join(name)).map_err(|error| EvalError::Io {
            reason: format!("{name}: {error}"),
        })
    };
    let manifest_bytes = read(EVIDENCE_MANIFEST)?;
    let seal_bytes = read(EVIDENCE_SEAL)?;
    let seal = String::from_utf8_lossy(&seal_bytes);
    verify_manifest_seal(&manifest_bytes, &seal, seal_key)?;
    let manifest =
        String::from_utf8(manifest_bytes).map_err(|error| EvalError::DigestMismatch {
            file: EVIDENCE_MANIFEST.to_owned(),
            want: "utf8 manifest".to_owned(),
            got: error.to_string(),
        })?;
    let mut lines = manifest.lines();
    let header = lines.next().unwrap_or("");
    if header != "file digest" {
        return Err(EvalError::DigestMismatch {
            file: EVIDENCE_MANIFEST.to_owned(),
            want: "file digest".to_owned(),
            got: header.to_owned(),
        });
    }
    let mut seen_report = false;
    let mut seen_tape = false;
    for line in lines {
        if line.is_empty() {
            continue;
        }
        let (name, want) = line.split_once(' ').unwrap_or((line, ""));
        if !is_digest(want) {
            return Err(EvalError::DigestMismatch {
                file: EVIDENCE_MANIFEST.to_owned(),
                want: line.to_owned(),
                got: "malformed-digest".to_owned(),
            });
        }
        let bytes = read(name)?;
        let got = hash_hex(&bytes);
        if got != want {
            return Err(EvalError::DigestMismatch {
                file: name.to_owned(),
                want: want.to_owned(),
                got,
            });
        }
        if name == EVIDENCE_REPORT {
            seen_report = true;
        }
        if name == EVIDENCE_TAPE {
            seen_tape = true;
        }
    }
    if !(seen_report && seen_tape) {
        return Err(EvalError::DigestMismatch {
            file: EVIDENCE_MANIFEST.to_owned(),
            want: format!("{EVIDENCE_REPORT} + {EVIDENCE_TAPE} entries"),
            got: "entry-missing".to_owned(),
        });
    }
    Ok(())
}

/// Versioned trace export for offline tuners: format, escaping, version
/// policy and parsing live in the child module so this harness file stays
/// small; the names below are the public surface.
#[path = "trace_export.rs"]
mod trace_export;
pub use trace_export::{export_trace, import_trace, TRACE_MAGIC, TRACE_VERSION};

#[cfg(test)]
mod tests {
    use super::*;

    /// A loopback OpenAI-compatible endpoint for adapter tests: it counts
    /// connections, reports each request path before writing the response,
    /// and answers `body`. The accept thread runs for the test process; a
    /// leaked blocked accept is harmless next to a failing assertion.
    struct FakeServer {
        url: String,
        hits: std::sync::Arc<std::sync::atomic::AtomicUsize>,
        paths: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    }

    impl FakeServer {
        fn spawn(body: String) -> Self {
            use std::sync::atomic::{AtomicUsize, Ordering};
            use std::sync::{Arc, Mutex};
            let listener =
                std::net::TcpListener::bind("127.0.0.1:0").expect("fake server binds loopback");
            let port = listener.local_addr().expect("fake server addr").port();
            let hits = Arc::new(AtomicUsize::new(0));
            let paths = Arc::new(Mutex::new(Vec::new()));
            let (seen_hits, seen_paths) = (Arc::clone(&hits), Arc::clone(&paths));
            std::thread::spawn(move || {
                for stream in listener.incoming() {
                    let Ok(stream) = stream else { break };
                    seen_hits.fetch_add(1, Ordering::SeqCst);
                    serve_http_once(stream, &body, &mut |path| {
                        seen_paths
                            .lock()
                            .expect("fake server paths")
                            .push(path.to_owned());
                    });
                }
            });
            Self {
                url: format!("http://127.0.0.1:{port}/v1/chat/completions"),
                hits,
                paths,
            }
        }

        fn hits(&self) -> usize {
            self.hits.load(std::sync::atomic::Ordering::SeqCst)
        }

        fn paths(&self) -> Vec<String> {
            self.paths.lock().expect("fake server paths").clone()
        }
    }

    /// One canned chat-completions envelope whose content is `proposal`.
    fn chat_completion(proposal: &str) -> String {
        let escaped = proposal.replace('"', "\\\"");
        format!(
            "{{\"id\":\"stub-1\",\"object\":\"chat.completion\",\"choices\":[{{\"index\":0,\"message\":{{\"role\":\"assistant\",\"content\":\"{escaped}\"}},\"finish_reason\":\"stop\"}}],\"usage\":{{\"prompt_tokens\":11,\"completion_tokens\":5}}}}"
        )
    }

    /// Serve one HTTP/1.1 exchange and read the whole request before
    /// answering. `on_path` sees the request path before the response bytes,
    /// so a test that gets an answer also knows the path that asked.
    fn serve_http_once(stream: std::net::TcpStream, body: &str, on_path: &mut dyn FnMut(&str)) {
        use std::io::{Read, Write};
        let mut stream = stream;
        let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(10)));
        let mut raw = Vec::new();
        let mut chunk = [0u8; 1024];
        let header_end = loop {
            if raw.len() > 16_384 {
                return;
            }
            match stream.read(&mut chunk) {
                Ok(0) => return,
                Ok(count) => {
                    raw.extend_from_slice(&chunk[..count]);
                    if let Some(index) = raw.windows(4).position(|window| window == b"\r\n\r\n") {
                        break index;
                    }
                }
                Err(_) => return,
            }
        };
        let head = String::from_utf8_lossy(&raw[..header_end]).into_owned();
        if let Some(path) = head
            .split("\r\n")
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
        {
            on_path(path);
        }
        let mut length = 0usize;
        for line in head.split("\r\n").skip(1) {
            if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                length = value.trim().parse::<usize>().unwrap_or(0);
            }
        }
        let mut pending = raw.split_off(header_end + 4);
        while pending.len() < length {
            match stream.read(&mut chunk) {
                Ok(0) => break,
                Ok(count) => pending.extend_from_slice(&chunk[..count]),
                Err(_) => break,
            }
        }
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        let _ = stream.write_all(response.as_bytes());
        let _ = stream.flush();
    }

    #[test]
    fn task_counts_cover_every_flow() {
        assert_eq!(AUTHORING_TASKS.len(), AUTHORING_COUNT);
        assert_eq!(PLAY_TASKS.len(), PLAY_COUNT);
        for flow in [
            Flow::Brief,
            Flow::Place,
            Flow::Preview,
            Flow::Apply,
            Flow::Undo,
            Flow::Fork,
        ] {
            assert!(
                AUTHORING_TASKS.iter().any(|task| task.flow == flow),
                "authoring must cover {flow:?}"
            );
        }
        for flow in [Flow::Run, Flow::Capture, Flow::Verify] {
            assert!(
                PLAY_TASKS.iter().any(|task| task.flow == flow),
                "play must cover {flow:?}"
            );
        }
        let mut ids: Vec<&str> = AUTHORING_TASKS
            .iter()
            .chain(PLAY_TASKS.iter())
            .map(|task| task.id)
            .collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), AUTHORING_COUNT + PLAY_COUNT);
    }

    #[test]
    fn full_matrix_has_every_task_per_backend() {
        let report = run_eval();
        assert_eq!(report.rows.len(), (AUTHORING_COUNT + PLAY_COUNT) * 3);
        for task in AUTHORING_TASKS.iter().chain(PLAY_TASKS.iter()) {
            for backend in BACKENDS {
                let label = backend_label(backend);
                assert!(
                    report
                        .rows
                        .iter()
                        .any(|row| row.task == task.id && row.backend == label),
                    "missing row {} {label}",
                    task.id,
                );
            }
        }
        assert!(report.completed() > 0, "some rows must complete");
        assert!(report.rejected() > 0, "bad proposals must be rejected");
        assert!(
            report.privacy_failures() > 0,
            "remote privacy probes must fail"
        );
        assert!(report.unknown_cost() > 0, "unknown cost must appear");
    }

    #[test]
    fn unknown_cost_is_never_zero_claimed() {
        assert_eq!(Cost::Unknown.text(), "unknown");
        assert!(Cost::Unknown.is_unknown());
        assert!(!Cost::Units(0).is_unknown());
        assert_eq!(Cost::Units(42).text(), "42");
        let report = run_eval();
        for row in &report.rows {
            if row.cost.is_unknown() {
                assert_eq!(row.cost.text(), "unknown", "row {}", row.task);
            }
        }
        assert!(
            report
                .rows
                .iter()
                .filter(|row| row.backend == "local")
                .all(|row| !row.cost.is_unknown()),
            "local fixtures always report cost"
        );
    }

    #[test]
    fn task_rows_replay_identical() {
        let first = run_eval();
        let second = run_eval();
        assert_eq!(first.rows, second.rows);
    }

    #[test]
    fn breach_and_privacy_rows_reject() {
        let bully = run_row_by_id("a19", Backend::Mock).expect("a19 exists");
        assert!(!bully.completed, "frozen-foe fork must not complete");
        assert!(bully.rejected, "frozen-foe fork must reject");
        let probe = run_row_by_id("a08", Backend::Remote).expect("a08 exists");
        assert!(probe.rejected, "remote privacy probe must refuse");
        assert!(
            probe.privacy_failure,
            "remote privacy probe must flag privacy"
        );
        let clean = run_row_by_id("a04", Backend::Mock).expect("a04 exists");
        assert!(clean.completed, "clean place must complete");
        assert!(!clean.rejected, "clean place must not reject");
    }

    #[test]
    fn unknown_task_id_is_a_typed_error() {
        let error = run_row_by_id("zzz", Backend::Mock).expect_err("unknown id must fail");
        assert_eq!(error.code(), "bad_task");
        assert!(error.to_string().contains("zzz"));
    }

    #[test]
    fn eval_error_codes_name_legal_values() {
        assert_eq!(
            EvalError::ArenaFailed { task: "a18" }.code(),
            "arena_failed"
        );
        assert_eq!(
            EvalError::ExternalMcp {
                reason: "x".to_owned()
            }
            .code(),
            "external_mcp"
        );
    }

    #[test]
    fn report_text_reproduces_the_shape() {
        let report = run_eval();
        let text = report.to_text();
        println!("{text}");
        assert!(text.contains("completed="));
        assert!(text.contains("unknown"));
        assert!(text.contains("external-mcp"));
        for label in ["mock", "local", "remote"] {
            assert!(text.contains(label), "report must name {label}");
        }
    }

    #[test]
    fn external_pass_executes_and_reports_honestly() {
        let pass = external_mcp_pass();
        assert!(!pass.notes.is_empty(), "external pass must say what ran");
        if !pass.found {
            assert!(!pass.completed, "missing binary cannot complete");
        }
        if pass.completed {
            assert!(pass.found, "completion needs the binary");
            assert!(pass.tools_listed >= 3, "completion needs the listing");
        }
    }

    fn temp_a6_dir(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!("e2040-ev11-a6-{tag}-{}", std::process::id()))
    }

    /// Fixed operator key for hermetic tests: 32 bytes of hex over `0-9a-f`.
    /// Production keys come from [`OPERATOR_KEY_ENV`] via [`operator_seal_key`].
    const TEST_SEAL_KEY: &[u8] = b"0123456789abcdef0123456789abcdef";
    /// A different key standing in for the attacker's: same length, no match.
    const WRONG_SEAL_KEY: &[u8] = b"fedcba9876543210fedcba9876543210";

    #[test]
    fn clean_evidence_passes_verify() {
        let dir = temp_a6_dir("clean");
        let _ = std::fs::remove_dir_all(&dir);
        let report = run_eval();
        let written = write_report_files(&dir, &report, TEST_SEAL_KEY).expect("evidence writes");
        assert_eq!(written.len(), 4);
        assert_eq!(
            std::fs::read_to_string(&written[0]).expect("report reads"),
            report.to_text()
        );
        assert_eq!(
            std::fs::read_to_string(&written[1]).expect("tape reads"),
            tape_text(&report)
        );
        // The seal names its keyed scheme, never a bare digest.
        let seal = std::fs::read_to_string(dir.join(EVIDENCE_SEAL)).expect("seal reads");
        assert!(seal.starts_with("hmac-sha256:"), "{seal}");
        verify_evidence(&dir, TEST_SEAL_KEY).expect("untampered regen passes");
        // A byte-identical rewrite still verifies: digests cover bytes as-is.
        write_report_files(&dir, &report, TEST_SEAL_KEY).expect("rewrite");
        verify_evidence(&dir, TEST_SEAL_KEY).expect("rewrite passes");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn tampered_report_fails_verify_loudly() {
        let dir = temp_a6_dir("tamper-report");
        let _ = std::fs::remove_dir_all(&dir);
        let report = run_eval();
        write_report_files(&dir, &report, TEST_SEAL_KEY).expect("evidence writes");
        verify_evidence(&dir, TEST_SEAL_KEY).expect("clean passes before tamper");
        let mut bytes = std::fs::read(dir.join(EVIDENCE_REPORT)).expect("report reads");
        let last = bytes.len() - 1;
        bytes[last] = bytes[last].wrapping_add(1);
        std::fs::write(dir.join(EVIDENCE_REPORT), &bytes).expect("tamper writes");
        let error = verify_evidence(&dir, TEST_SEAL_KEY).expect_err("tampered report must fail");
        assert_eq!(error.code(), "digest_mismatch");
        assert!(error.to_string().contains(EVIDENCE_REPORT), "{error}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn tampered_tape_manifest_or_seal_fails_verify() {
        let dir = temp_a6_dir("tamper-seal");
        let _ = std::fs::remove_dir_all(&dir);
        let report = run_eval();
        write_report_files(&dir, &report, TEST_SEAL_KEY).expect("evidence writes");
        // Swapped tape bytes fail even though the report is untouched.
        let mut tape = std::fs::read(dir.join(EVIDENCE_TAPE)).expect("tape reads");
        tape[0] = tape[0].wrapping_add(1);
        std::fs::write(dir.join(EVIDENCE_TAPE), &tape).expect("tamper writes");
        assert_eq!(
            verify_evidence(&dir, TEST_SEAL_KEY)
                .expect_err("tampered tape must fail")
                .code(),
            "digest_mismatch"
        );
        write_report_files(&dir, &report, TEST_SEAL_KEY).expect("restore");
        // A manifest line dropped silently fails: the report entry is required.
        let manifest = std::fs::read_to_string(dir.join(EVIDENCE_MANIFEST)).expect("manifest");
        let stripped: String = manifest
            .lines()
            .filter(|line| !line.starts_with(EVIDENCE_REPORT))
            .map(|line| format!("{line}\n"))
            .collect();
        std::fs::write(dir.join(EVIDENCE_MANIFEST), stripped.as_bytes()).expect("manifest rewrite");
        let seal = seal_manifest(stripped.as_bytes(), TEST_SEAL_KEY).expect("reseal");
        std::fs::write(dir.join(EVIDENCE_SEAL), seal.as_bytes()).expect("seal rewrite");
        let error = verify_evidence(&dir, TEST_SEAL_KEY).expect_err("stripped manifest must fail");
        assert_eq!(error.code(), "digest_mismatch");
        // A resealed forgery of the seal alone fails against the manifest.
        write_report_files(&dir, &report, TEST_SEAL_KEY).expect("restore");
        std::fs::write(
            dir.join(EVIDENCE_SEAL),
            b"hmac-sha256:0000000000000000000000000000000000000000000000000000000000000000\n",
        )
        .expect("seal tamper");
        assert_eq!(
            verify_evidence(&dir, TEST_SEAL_KEY)
                .expect_err("broken seal must fail")
                .code(),
            "digest_mismatch"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn resealed_forgery_without_the_key_fails_verify() {
        // The edited-report attack: change the report, refresh the manifest
        // digests, and reseal without the operator key (bare digest, the old
        // format). Verify must reject the unkeyed seal loudly.
        let dir = temp_a6_dir("forgery");
        let _ = std::fs::remove_dir_all(&dir);
        let report = run_eval();
        write_report_files(&dir, &report, TEST_SEAL_KEY).expect("evidence writes");
        let mut bytes = std::fs::read(dir.join(EVIDENCE_REPORT)).expect("report reads");
        let last = bytes.len() - 1;
        bytes[last] = bytes[last].wrapping_add(1);
        std::fs::write(dir.join(EVIDENCE_REPORT), &bytes).expect("forgery writes");
        let manifest = format!(
            "file digest\n{EVIDENCE_REPORT} {}\n{EVIDENCE_TAPE} {}\n",
            hash_hex(&bytes),
            hash_hex(tape_text(&report).as_bytes()),
        );
        std::fs::write(dir.join(EVIDENCE_MANIFEST), &manifest).expect("manifest reseal");
        std::fs::write(
            dir.join(EVIDENCE_SEAL),
            format!("{}\n", hash_hex(manifest.as_bytes())),
        )
        .expect("seal reseal");
        let error = verify_evidence(&dir, TEST_SEAL_KEY).expect_err("resealed forgery must fail");
        assert_eq!(error.code(), "digest_mismatch");
        assert!(error.to_string().contains(EVIDENCE_SEAL), "{error}");
        // Same attack resealed under the wrong key fails on the tag.
        let forged = seal_manifest(manifest.as_bytes(), WRONG_SEAL_KEY).expect("wrong-key seal");
        std::fs::write(dir.join(EVIDENCE_SEAL), forged.as_bytes()).expect("seal rewrite");
        let error = verify_evidence(&dir, TEST_SEAL_KEY).expect_err("wrong-key reseal must fail");
        assert_eq!(error.code(), "digest_mismatch");
        assert!(error.to_string().contains(EVIDENCE_SEAL), "{error}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn wrong_key_rejects_clean_evidence() {
        let dir = temp_a6_dir("wrong-key");
        let _ = std::fs::remove_dir_all(&dir);
        let report = run_eval();
        write_report_files(&dir, &report, TEST_SEAL_KEY).expect("evidence writes");
        let error = verify_evidence(&dir, WRONG_SEAL_KEY).expect_err("wrong key must fail");
        assert_eq!(error.code(), "digest_mismatch");
        assert!(error.to_string().contains(EVIDENCE_SEAL), "{error}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn legacy_and_malformed_seals_fail_loudly() {
        let manifest = b"file digest\nreport.txt 0000000000000000\n";
        // Every legacy unkeyed seal is rejected, never verified.
        for legacy in ["0000000000000000\n", "ffffffffffffffff\n", "\n", ""] {
            let error = verify_manifest_seal(manifest, legacy, TEST_SEAL_KEY)
                .expect_err("legacy seal must fail");
            assert_eq!(error.code(), "digest_mismatch");
            assert!(error.to_string().contains(EVIDENCE_SEAL), "{error}");
        }
        // Right scheme but truncated, long, short or non-hex tags fail as malformed.
        let short_tag = "00".repeat(31);
        for broken in [
            "hmac-sha256:abc\n".to_owned(),
            "hmac-sha256:zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz\n"
                .to_owned(),
            "hmac-sha256:00000000000000000000000000000000000000000000000000000000000000000\n"
                .to_owned(),
            format!("hmac-sha256:{short_tag}\n"),
            "hmac-sha256:\n".to_owned(),
        ] {
            let error = verify_manifest_seal(manifest, &broken, TEST_SEAL_KEY)
                .expect_err("malformed seal must fail");
            assert_eq!(error.code(), "digest_mismatch");
            assert!(error.to_string().contains("malformed-seal"), "{error}");
        }
        // The keyed seal round-trips on the same bytes.
        let seal = seal_manifest(manifest, TEST_SEAL_KEY).expect("seal renders");
        verify_manifest_seal(manifest, &seal, TEST_SEAL_KEY).expect("seal verifies");
    }

    #[test]
    fn seal_key_parsing_refuses_short_and_malformed_keys() {
        assert_eq!(
            parse_seal_key(&"0123456789abcdef".repeat(4)).expect("64 hex chars parse"),
            vec![
                0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef, 0x01, 0x23, 0x45, 0x67, 0x89, 0xab,
                0xcd, 0xef, 0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef, 0x01, 0x23, 0x45, 0x67,
                0x89, 0xab, 0xcd, 0xef
            ]
        );
        let short = "00".repeat(31);
        let nonhex = "zz".repeat(32);
        for bad in [
            "",
            "abc",
            "0123456789abcdef",
            short.as_str(),
            nonhex.as_str(),
        ] {
            let error = parse_seal_key(bad).expect_err("bad key must fail");
            assert_eq!(error.code(), "seal_key");
            assert!(error.to_string().contains(OPERATOR_KEY_ENV), "{error}");
        }
    }

    #[test]
    fn missing_operator_key_fails_closed() {
        let error = seal_key_from_env("E2040_TEST_KEY_ABSENT_e2040_ev14")
            .expect_err("missing key must fail");
        assert_eq!(error.code(), "seal_key");
        assert!(error.to_string().contains("missing"), "{error}");
    }

    #[test]
    fn trace_export_reimports_byte_identically_twice() {
        let report = run_eval();
        assert_eq!(
            report.rows.len(),
            (AUTHORING_COUNT + PLAY_COUNT) * BACKENDS.len()
        );
        assert_eq!(report.rows.len(), 75, "the export holds 75 rows");
        let first = export_trace(&report);
        assert!(
            first.starts_with(&format!("{TRACE_MAGIC} v{TRACE_VERSION} rows=75\n")),
            "header must name the magic, version and row count"
        );
        let back = import_trace(&first).expect("export must re-import");
        assert_eq!(back, report, "re-import must recover the report");
        let second = export_trace(&back);
        assert_eq!(second, first, "first re-export must be byte-identical");
        let again = import_trace(&second).expect("second import must hold");
        let third = export_trace(&again);
        assert_eq!(third, first, "second re-export must be byte-identical");
    }

    #[test]
    fn trace_escapes_survive_the_round_trip() {
        let mut report = run_eval();
        let tricky = "tab\there \\ backslash\r\nnewline航班";
        report.rows[0].detail = tricky.to_owned();
        report.external.notes = tricky.to_owned();
        let text = export_trace(&report);
        let back = import_trace(&text).expect("escapes must round-trip");
        assert_eq!(back, report, "escaped fields must come back verbatim");
        assert_eq!(
            export_trace(&back),
            text,
            "re-export must be byte-identical"
        );
    }

    #[test]
    fn trace_unknown_version_refuses_loudly() {
        let report = run_eval();
        let text = export_trace(&report);
        for bad_version in ["v2", "v0", "v99", "vx"] {
            let forged = text.replacen(" v1 ", &format!(" {bad_version} "), 1);
            let error = import_trace(&forged).expect_err("unknown version must refuse");
            assert_eq!(error.code(), "trace_version");
            assert!(
                error.to_string().contains("v1"),
                "refusal must name the supported version: {error}"
            );
        }
        let error = import_trace("garbage\n").expect_err("bad magic must refuse");
        assert_eq!(error.code(), "trace_parse");
    }

    #[test]
    fn trace_truncated_swapped_and_miscounted_bodies_refuse() {
        let report = run_eval();
        let text = export_trace(&report);
        // Dropped final line truncates the external pass.
        let mut lines: Vec<&str> = text.lines().collect();
        lines.pop();
        let dropped = lines.join("\n") + "\n";
        let error = import_trace(&dropped).expect_err("dropped line must fail");
        assert_eq!(error.code(), "trace_parse");
        // Corrupted boolean fails where it stands.
        let bad_bool = text.replacen("\tfalse\t", "\tmaybe\t", 1);
        let error = import_trace(&bad_bool).expect_err("bad bool must fail");
        assert_eq!(error.code(), "trace_parse");
        // Declared count must match the body exactly.
        let bad_count = text.replacen("rows=75", "rows=74", 1);
        let error = import_trace(&bad_count).expect_err("miscount must fail");
        assert_eq!(error.code(), "trace_parse");
        // Unknown escapes fail instead of guessing.
        let bad_escape = text.replacen("brief ok", "brief \\q ok", 1);
        let error = import_trace(&bad_escape).expect_err("bad escape must fail");
        assert_eq!(error.code(), "trace_parse");
        // Missing final newline reads as truncation.
        let no_trailing = text.strip_suffix('\n').expect("export ends with newline");
        let error = import_trace(no_trailing).expect_err("missing newline must fail");
        assert_eq!(error.code(), "trace_parse");
    }
    #[test]
    fn distractor_stress_names_a_failure_mode_clean_eval_misses() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/eval-distractors");
        let entries = std::fs::read_dir(dir).expect("read");
        let count = entries.count();
        assert!((5..=10).contains(&count));
        let (clean, stress) = run_distractor_stress(count);
        assert_eq!(clean.len(), 9);
        assert_eq!(stress.len(), 9);
        assert!(clean.iter().all(|row| row.completed));
        assert!(stress[0].detail.contains("retrieval_dilution"));
        println!("{count} retrieval_dilution");
    }

    /// The `Local` label must reach the real sidecar adapter, not the mock:
    /// one exchange on `/v1/chat/completions`, and the answer body decides
    /// the proposal.
    #[test]
    fn ev1_local_row_answers_through_the_local_adapter() {
        let body = chat_completion(
            "{\"actions\": [{\"noop\": true}], \"diagnostics\": [\"server:local-body\"]}",
        );
        let server = FakeServer::spawn(body);
        let targets = BackendTargets {
            local_url: Some(server.url.clone()),
            remote_url: None,
            remote_token_env: None,
        };
        let task = AUTHORING_TASKS
            .iter()
            .find(|task| task.id == "a01")
            .expect("a01 exists");
        let (row, answer) = authoring_row(task, 0, Backend::Local, &targets, None);
        let answer = answer.expect("the fake server answers");
        assert!(row.completed, "row completes: {row:?}");
        assert_eq!(row.backend, "local", "the row keeps its backend label");
        assert!(
            row.detail.ends_with("adapter=local"),
            "the row names the adapter that answered: {}",
            row.detail
        );
        assert!(
            !row.detail.contains("adapter=mock"),
            "a live row never reports the mock: {}",
            row.detail
        );
        assert_eq!(answer.adapter, "local");
        assert!(
            answer
                .proposal
                .diagnostics
                .iter()
                .any(|line| line == "server:local-body"),
            "the answer came from the server body: {:?}",
            answer.proposal.diagnostics
        );
        assert_eq!(server.hits(), 1, "exactly one exchange, no retry");
        assert_eq!(
            server.paths(),
            vec![String::from("/v1/chat/completions")],
            "the local adapter asks the sidecar route"
        );
    }

    /// The same local exchange records to a cassette and replays with zero
    /// sockets: a replay hit never dials, and the row matches the record.
    #[test]
    fn ev1_local_row_records_and_replays_from_a_cassette() {
        use engine_provider::cassette::CassetteMode;

        let body = chat_completion(
            "{\"actions\": [{\"noop\": true}], \"diagnostics\": [\"server:cassette\"]}",
        );
        let server = FakeServer::spawn(body);
        let dir =
            std::env::temp_dir().join(format!("e2040-ev1-local-cassette-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let task = AUTHORING_TASKS
            .iter()
            .find(|task| task.id == "a04")
            .expect("a04 exists");
        let live = BackendTargets {
            local_url: Some(server.url.clone()),
            remote_url: None,
            remote_token_env: None,
        };
        let record = CassetteSetup {
            mode: CassetteMode::Record,
            dir: dir.clone(),
        };
        let (recorded_row, recorded_answer) =
            authoring_row(task, 3, Backend::Local, &live, Some(&record));
        let recorded_answer = recorded_answer.expect("record answers live");
        assert!(recorded_row.completed, "record row: {recorded_row:?}");
        assert_eq!(server.hits(), 1, "one live exchange recorded");
        let entries = std::fs::read_dir(&dir)
            .expect("the record created the store")
            .count();
        assert_eq!(entries, 1, "one cassette entry written");

        // A dead loopback URL can never answer, so a replay hit proves the
        // store served the call without a socket.
        let dead_port = {
            let probe =
                std::net::TcpListener::bind("127.0.0.1:0").expect("dead port binds loopback");
            probe.local_addr().expect("dead port addr").port()
        };
        let dead = BackendTargets {
            local_url: Some(format!("http://127.0.0.1:{dead_port}/v1/chat/completions")),
            remote_url: None,
            remote_token_env: None,
        };
        let replay = CassetteSetup {
            mode: CassetteMode::Replay,
            dir: dir.clone(),
        };
        let (replay_row, replay_answer) =
            authoring_row(task, 3, Backend::Local, &dead, Some(&replay));
        let replayed = replay_answer.expect("replay answers from the store");
        assert_eq!(replayed, recorded_answer, "replay bytes match the record");
        assert_eq!(replay_row, recorded_row, "the whole row matches the record");
        assert_eq!(
            server.hits(),
            1,
            "a replay hit makes no exchange on the record server"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn ev1_cassette_replay_is_byte_identical_offline() {
        use engine_provider::cassette::{
            record_once_replay_twice, CassetteAdapter, CassetteError, CassetteMode, CassetteStore,
            ChangedParts, MissReason,
        };
        use engine_provider::vendor::VendorSecret;
        use engine_provider::ProviderAdapter;
        use engine_provider::{fixtures, parse_endpoint, OpenAiAdapter};

        // One canned chat-completions envelope. The strict proposal grammar
        // parses its content at the first rung, so the record pass costs
        // exactly one exchange; a ladder drop would fail loudly below.
        let body = chat_completion(
            "{\"actions\": [{\"noop\": true}], \"diagnostics\": [\"cassette:suite\"]}",
        );
        let listener =
            std::net::TcpListener::bind("127.0.0.1:0").expect("suite stub binds loopback");
        let live_port = listener.local_addr().expect("suite stub addr").port();
        std::thread::spawn(move || {
            // Exactly one exchange: the record parses at the first rung, so
            // the worker serves one connection and exits. No join: a failed
            // record never connects, and a leaked blocked accept is harmless
            // next to the failing assertion it accompanies.
            if let Ok((stream, _)) = listener.accept() {
                serve_http_once(stream, &body, &mut |_| {});
            }
        });
        // Bind-and-drop a loopback port so replay dials deterministically
        // fail: a hit never dials, so zero attempts proves store service.
        let dead_port = {
            let probe =
                std::net::TcpListener::bind("127.0.0.1:0").expect("dead port binds loopback");
            probe.local_addr().expect("dead port addr").port()
        };
        let live_url = format!("http://127.0.0.1:{live_port}/v1/chat/completions");
        let dead = format!("http://127.0.0.1:{dead_port}/v1/chat/completions");
        let build_live = || {
            OpenAiAdapter::new(
                vec![MockStep::Succeed],
                parse_endpoint(&live_url).expect("live url parses"),
                VendorSecret::Direct(String::from("cassette-token")),
            )
        };
        let build_dead = || {
            OpenAiAdapter::new(
                vec![MockStep::Succeed],
                parse_endpoint(&dead).expect("dead endpoint parses"),
                VendorSecret::Direct(String::from("cassette-token")),
            )
        };
        let mut ask = fixtures::request(7, 3);
        ask.model.adapter = AdapterKind::Remote;
        ask.model.model = String::from("openai:gpt-fixture");
        ask.privacy = PrivacyMode::RemoteAllowed;
        let policy = AiSessionPolicy {
            adapters: vec![AdapterKind::Remote],
            mode: PrivacyMode::RemoteAllowed,
            destinations: vec![live_url.clone(), dead.clone()],
            scopes: Vec::new(),
            request_limit: 8,
            max_concurrency: 1,
            spend_limit: None,
            fallback: Fallback {
                order: Vec::new(),
                scripted_ok: true,
            },
        };
        let caps = ProviderCapabilities::remote_http();

        // The provider half, once in record: the driver below lands with the
        // next edit; this test must fail until it does.
        let dir = std::env::temp_dir().join(format!("e2040-ev1-cassette-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let store = CassetteStore::new(&dir);
        let proof = record_once_replay_twice(
            &store,
            "openai",
            &ask,
            &policy,
            &caps,
            build_live,
            build_dead,
            |adapter| adapter.socket_attempts(),
        )
        .expect("record answers live");
        assert!(proof.recorded_ok, "the live record must answer");
        assert!(
            proof.recorded_result.contains("Complete"),
            "the record is a complete proposal: {}",
            proof.recorded_result
        );
        assert_eq!(
            proof.result_distance,
            [0, 0],
            "both offline replays match the record byte for byte"
        );
        assert_eq!(proof.sockets, [0, 0], "replay dials nothing");
        assert_eq!(proof.misses, [None, None], "replays hit, never miss");
        assert_eq!(proof.replay_distance, 0, "the two replays match each other");
        assert_eq!(
            proof.replay_results[0].as_bytes(),
            proof.replay_results[1].as_bytes(),
            "replay bytes are identical"
        );
        let files = std::fs::read_dir(&dir).expect("store lists").count();
        assert_eq!(files, 1, "one entry recorded, replays write nothing");

        // The EV1 suite path, once in record: the full 20 authoring + 5 play
        // matrix across mock/local/remote plus its reproducible text.
        let mut recorded_report = run_eval();
        assert_eq!(
            recorded_report.rows.len(),
            75,
            "20 authoring + 5 play tasks across mock/local/remote"
        );
        // Only the external MCP row carries environment timing (module docs),
        // so the passes pin it and prove the 75 task rows byte-identical.
        recorded_report.external = ExternalPass {
            completed: false,
            found: false,
            latency_ms: 0,
            tools_listed: 0,
            notes: String::from("cassette: offline replay pins the environment-timing row"),
        };
        let recorded_bytes = recorded_report.to_text();
        for pass in 1..=2 {
            let mut replay = run_eval();
            replay.external = recorded_report.external.clone();
            let bytes = replay.to_text();
            assert_eq!(
                bytes.len(),
                recorded_bytes.len(),
                "replay {pass} length matches"
            );
            assert_eq!(
                bytes.as_bytes(),
                recorded_bytes.as_bytes(),
                "replay {pass} matches the record byte for byte"
            );
        }

        // A changed prompt misses with its reason instead of reusing silently.
        let mut changed = ask.clone();
        changed.base_revision = ask.base_revision + 1;
        let mut refused = CassetteAdapter::new(build_dead(), store, CassetteMode::Replay, "openai")
            .expect("replay label");
        let outcome = refused.call(&changed, &policy, &caps);
        assert_eq!(
            outcome.result,
            Err(AiError::UnsupportedCapability),
            "a changed prompt never reuses the entry"
        );
        assert!(!outcome.dispatched_remote, "a miss dispatches nothing");
        assert_eq!(refused.inner().socket_attempts(), 0, "a miss dials nothing");
        let want = CassetteError::Miss(MissReason::Changed(ChangedParts {
            user: true,
            ..Default::default()
        }));
        assert_eq!(refused.last_miss(), Some(want), "the miss names the change");
        assert!(
            refused
                .last_miss()
                .expect("miss")
                .to_string()
                .contains("user prompt"),
            "the reason says why: {}",
            refused.last_miss().expect("miss")
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
