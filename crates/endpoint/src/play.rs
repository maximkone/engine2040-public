//! MCP2: `play.run` and `capture` tools over headless arena matches.
//!
//! Consumer: the QA1 playtest agent and external MCP clients. Data owner for
//! the simulation is `engine-arena` (headless [`run`](engine_arena::run));
//! this module owns only the job handles: poll/cancel/expiry, progress
//! events and bounded logs. Units follow arena (meters, fixed ticks).
//! Determinism scope: same start args give the same checkpoints; the registry
//! is a `BTreeMap` under a monotonic id counter, with no threads and no
//! hash-order iteration. Wall time is opt-in (`timing`): a receipt-only
//! measurement labelled `software` (a CPU run, never a reference-GPU claim)
//! that never enters a digest, a hash or the job state. Without the flag no
//! clock is read and the output is byte-identical to a run without timing.
//!
//! Sync core: the match simulates eagerly at `start` (ticks are MCP-bounded
//! so it stays fast) while the handle still walks the full
//! start -> poll(progress) -> result protocol with cancel and poll-count
//! expiry, so drivers written against async runners keep working.
//!
//! Capture never imports render: without a caller-supplied `FnOnce` callback
//! the tool returns structured `unsupported_capture`, never a success shape.
//! The wired path ([`capture_with`]) validates the render-free R1-style
//! receipt (manifest fields plus PNG magic bytes) the callback supplies.
//! [`play_session`] holds a live duel across `play.step` calls with a real
//! PNG per step (software rasterizer, GPU hook when a device exists).

pub mod play_session;
pub mod requery;
pub mod scene_frame;
pub mod world_load;

use std::borrow::Cow;
use std::collections::BTreeMap;

use engine_arena::spine_step::run_spine;
use engine_arena::world::MatchWorld;
use engine_arena::{run, ArenaConfig, Checkpoint, FighterId, FighterInput, FighterTape};
use engine_core::{hash::GOLDEN_GAMMA, splitmix64};
use engine_input::fighter::{FACING_RANGE, KNOWN_BUTTONS, WISH_RANGE};
use rmcp::handler::server::router::tool::{SyncTool, ToolBase};
use rmcp::model::{CallToolResult, ContentBlock, ToolAnnotations};
use rmcp::schemars::JsonSchema;
use rmcp::serde::{Deserialize, Serialize};
use rmcp::serde_json::Value;

use super::mcp::{McpServer, McpToolError};

/// Tool: headless arena match job (start/poll/cancel multiplexed in `action`).
pub const TOOL_PLAY_RUN: &str = "play.run";
/// Tool: renderer receipt (unsupported unless a capture callback is wired).
pub const TOOL_CAPTURE: &str = "capture";

/// MCP bound on match length: 60 s at 60 Hz (arena allows up to 100 000).
pub const MAX_PLAY_TICKS: u32 = 3600;
/// MCP bound on tape inputs per job.
pub const MAX_PLAY_INPUTS: usize = 2048;
/// MCP bound on live jobs per session.
pub const MAX_JOBS: usize = 16;
/// MCP bound on log lines kept per job.
pub const MAX_JOB_LOGS: usize = 16;
/// MCP bound on chars kept per log line.
pub const MAX_LOG_LINE_CHARS: usize = 200;
/// Default polls before a running job expires.
pub const DEFAULT_TTL_POLLS: u64 = 16;
/// MCP bound on the poll-expiry horizon.
pub const MAX_TTL_POLLS: u64 = 1024;
/// Default receipt width when the caller omits it.
pub const CAPTURE_DEFAULT_WIDTH: u32 = 96;
/// Default receipt height when the caller omits it.
pub const CAPTURE_DEFAULT_HEIGHT: u32 = 64;
/// Receipt dimension bound, mirroring the R1 `MAX_CAPTURE_DIM` (endpoint
/// never imports render, so the value is mirrored, not shared).
pub const CAPTURE_MAX_DIM: u32 = 2048;
/// MCP bound on PNG bytes accepted from a wired capture callback.
pub const MAX_CAPTURE_BYTES: usize = 4 * 1024 * 1024;
/// Default longest-side cap applied before encode (`max_size`).
pub const CAPTURE_DEFAULT_MAX_SIZE: u32 = 768;
/// Floor on `max_size`: a frame narrower than this is not usable evidence.
pub const CAPTURE_MIN_MAX_SIZE: u32 = 200;
/// Cap on the caption text block beside the image block.
pub const FRAME_CAPTION_MAX_CHARS: usize = 2048;

/// PNG file magic: a wired receipt must start with these bytes. One home
/// for the play module plus the receipt tests (`play_session`,
/// `requery`, `tests/it/step20x2`).
pub const PNG_MAGIC: [u8; 8] = [137, 80, 78, 71, 13, 10, 26, 10];
/// Wall-time samples a session keeps for the opt-in `timing` receipt. The
/// ring is bounded so a long-lived session never grows without limit.
pub const TIMING_RING_CAP: usize = 256;
/// Label on every `timing` receipt: the numbers come from a CPU run, so they
/// are a `software` figure, never a reference-GPU claim.
pub const TIMING_LABEL_SOFTWARE: &str = "software";
/// Fewest fighters a match runs (the arena config check owns the same bound).
const MIN_FIGHTERS: u8 = 2;

// ---------------------------------------------------------------------------
// Transport DTOs. Field docs are omitted on purpose: every doc line inflates
// the advertised schemas, and the eleven-tool discovery budget is a gate.
// ---------------------------------------------------------------------------

/// One fighter input: tick id, fighter id, wish vector, facing and buttons.
/// Units are ticks, wish units in -100..=100 and facing in -18000..=18000.
/// Errors are typed tape refusals naming the legal ranges. Budget rides on
/// the 2048-input job cap.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct PlayInputDto {
    pub tick: u32,
    pub fighter: u8,
    pub mx: i8,
    pub mz: i8,
    pub facing: i16,
    pub buttons: u8,
}

/// The three verbs of the multiplexed `play.run` tool. The wire keeps the
/// plain `action` text; the dispatch matches this enum, so an unknown verb
/// is refused once, at [`PlayAction::parse`], and never reaches a job.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayAction {
    /// Run a bounded match and file its job.
    Start,
    /// Read the next progress or the final result of a job.
    Poll,
    /// Stop a running job.
    Cancel,
}

impl PlayAction {
    /// Parse the wire text; anything else is `bad_action` naming the legal
    /// verbs.
    pub fn parse(text: &str) -> Result<Self, McpToolError> {
        match text {
            "start" => Ok(Self::Start),
            "poll" => Ok(Self::Poll),
            "cancel" => Ok(Self::Cancel),
            other => Err(McpToolError::new(
                "bad_action",
                format!("action `{other}` is unknown; submit start, poll or cancel"),
            )),
        }
    }

    /// Job id a `poll` or `cancel` names; `bad_job_id` when the call omits it.
    fn job_id(self, args: &PlayRunArgs) -> Result<u64, McpToolError> {
        args.job_id.ok_or_else(|| {
            let verb = match self {
                Self::Start => "start",
                Self::Poll => "poll",
                Self::Cancel => "cancel",
            };
            McpToolError::new(
                "bad_job_id",
                format!("action `{verb}` needs job_id; `start` returns one"),
            )
        })
    }
}

/// Match job request: start, poll or cancel plus the start composition.
/// Units are ticks, meters of half-extent and poll counts. Errors are typed
/// `bad_ticks`, `bad_extent` and job errors. Budget is 3600 ticks a match.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct PlayRunArgs {
    pub action: String,
    pub seed: Option<u64>,
    pub ticks: Option<u32>,
    pub fighters: Option<u8>,
    pub half_extent_m: Option<f32>,
    pub inputs: Option<Vec<PlayInputDto>>,
    pub ttl_polls: Option<u64>,
    pub job_id: Option<u64>,
    pub scene_rev: Option<u64>,
    // Opt-in wall-time receipt (PF24): `true` carries a `timing` object on
    // every status, absent or `false` reads no clock at all.
    pub timing: Option<bool>,
}

/// One match checkpoint: tick id plus the state hash there.
/// Units are ticks in, hashes out. No errors: checkpoints replay the eager
/// result. Budget rides on the 3600-tick match cap.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct CheckpointDto {
    pub tick: u32,
    pub hash: u64,
}

/// Match job answer: status, progress, logs and checkpoints plus the manifest.
/// Units are ticks, meters of half-extent and log chars. No errors once
/// started; terminal handles replay. Budget is 16 log lines of 200 chars.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct PlayRunOutput {
    pub job_id: u64,
    pub status: String,
    pub progress: f32,
    pub events: Vec<String>,
    pub logs: Vec<String>,
    pub truncated_logs: bool,
    pub tick_count: u32,
    pub final_hash: Option<u64>,
    pub checkpoints: Vec<CheckpointDto>,
    // Play manifest: the exact seed and arena half-extent (meters) the job
    // simulated, echoed on every status. Flat fields, never a nested
    // object: the 26-tool discovery budget (`crate::tools::TOOLS`) is a gate and every nested
    // definition spends it. Ticks ride on `tick_count`, so a cross-tool
    // replay refuses seed/extent drift loudly instead of comparing digests
    // across silent defaults.
    pub seed: u64,
    pub half_extent_m: f32,
    pub scene_rev: Option<u64>,
    // Pin digest of the scene bytes `scene_rev` named (lowercase hex), echoed
    // on every status so a run always names the exact scene it played.
    pub scene_digest: Option<String>,
    // Opt-in wall-time receipt: present only when the caller asked (`timing`),
    // so a run without the flag serializes byte-identically to one before
    // timing existed. Wall time never enters a digest or a hash.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timing: Option<PlayTimingDto>,
    // AX28: where the disk-backed job counters live, relative to the
    // workspace root. Empty when the door could not write them.
    pub counters: String,
}

// ---------------------------------------------------------------------------
// Opt-in wall time (PF24): a receipt-only measurement. Numbers come from the
// CPU path, so the label is `software`; a missing sample answers `unknown`,
// never a zero. Nothing here is read by the simulation, a digest or a hash.
// Field docs are omitted on purpose: every doc line inflates the advertised
// schemas, and the tool-list discovery budget is a gate.
// ---------------------------------------------------------------------------

/// Wall-time receipt: step milliseconds at p50 and p99 over the session's
/// bounded ring, labelled `software`. Units are milliseconds and samples.
/// No errors: an empty ring answers `"unknown"`. Budget is
/// [`TIMING_RING_CAP`] samples.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct PlayTimingDto {
    pub label: String,
    // Step wall p50/p99 in milliseconds, or the word `unknown` with no sample.
    pub p50_ms: StepMillis,
    pub p99_ms: StepMillis,
    // Samples behind the numbers, `0..=TIMING_RING_CAP`.
    pub samples: usize,
}

impl PlayTimingDto {
    /// Zero-sample report: both figures answer `unknown`, never zero. A timed
    /// poll of a job whose start did not measure answers this.
    #[must_use]
    pub fn unknown() -> Self {
        Self {
            label: TIMING_LABEL_SOFTWARE.to_owned(),
            p50_ms: StepMillis::Missing(NoSample::Unknown),
            p99_ms: StepMillis::Missing(NoSample::Unknown),
            samples: 0,
        }
    }
}

/// One wall-time figure: milliseconds, or `"unknown"` when the ring held no
/// sample (a run of zero steps). Never a zero for a missing sample.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(crate = "rmcp::serde", untagged)]
#[schemars(crate = "rmcp::schemars")]
pub enum StepMillis {
    Measured(f64),
    Missing(NoSample),
}

/// The one word a missing sample answers with.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(crate = "rmcp::serde", rename_all = "lowercase")]
#[schemars(crate = "rmcp::schemars")]
pub enum NoSample {
    Unknown,
}

/// Capture request: optional receipt width and height in pixels, plus the
/// longest-side cap `max_size` (default 768, clamped to 200..=2048 and applied
/// before encode). Units are pixels in, PNG bytes out. Errors are typed
/// `bad_size` outside `1..=2048`. Budget is 4 MB of PNG bytes.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct CaptureArgs {
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub max_size: Option<u32>,
}

/// Receipt manifest: adapter, driver, dimensions and validation errors.
/// Units are pixels and error counts. No errors once captured; a wired
/// callback owns the pixels. Budget is 256 chars per device string.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct CaptureManifestDto {
    pub adapter: String,
    pub driver: String,
    pub width: u32,
    pub height: u32,
    pub validation_errors: u32,
}

/// Capture answer: the manifest plus the PNG bytes.
/// Units are pixels in, PNG bytes out. Errors are typed `bad_receipt` for
/// non-PNG or skewed receipts. Budget is 4 MB of PNG bytes.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct CaptureOutput {
    pub manifest: CaptureManifestDto,
    pub png_bytes: Vec<u8>,
}

// ---------------------------------------------------------------------------
// Job registry: poll/cancel/expiry plus bounded logs over eager matches.
// ---------------------------------------------------------------------------

/// Live job phase; serialized as `pending|running|completed|failed|cancelled|expired`.
///
/// AX28: one vocabulary for every job the door serves, so a client polls
/// one set of words instead of one per job kind. `pending` is a job that is
/// filed but not yet run, `running` is a job that is working, `completed` is
/// a job that finished with a result, `failed` is a job that finished with
/// a refusal, `cancelled` is a job the caller stopped, `expired` is a job
/// that outlived its poll budget.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobStatus {
    /// Filed, not yet running.
    Pending,
    /// Working.
    Running,
    /// Finished with a result.
    Completed,
    /// Finished with a refusal.
    Failed,
    /// Stopped by the caller.
    Cancelled,
    /// Outlived its poll budget.
    Expired,
}

impl JobStatus {
    /// Every status, in vocabulary order.
    pub const ALL: [JobStatus; 6] = [
        JobStatus::Pending,
        JobStatus::Running,
        JobStatus::Completed,
        JobStatus::Failed,
        JobStatus::Cancelled,
        JobStatus::Expired,
    ];

    /// The wire word for this status.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::Expired => "expired",
        }
    }

    /// Read a status word back; `None` for a word outside the vocabulary.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|status| status.as_str() == text)
    }

    /// True when the job will not move again.
    #[must_use]
    pub fn is_terminal(self) -> bool {
        !matches!(self, Self::Pending | Self::Running)
    }
}

/// The job counters: one count per status word.
///
/// AX28: the counts are backed by disk. The table keeps them live and writes
/// them to `target/reports/play-jobs.json` when a job reaches a terminal
/// status, so a session that starts later reconciles against the file
/// instead of counting from zero. The file is the evidence; the live
/// counters are this session's view of it.
///
/// Units: jobs. Determinism: the same jobs in the same order give the same
/// counts and the same file body.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
pub struct JobCounters {
    /// Jobs filed and not yet run.
    pub pending: u64,
    /// Jobs working.
    pub running: u64,
    /// Jobs that finished with a result.
    pub completed: u64,
    /// Jobs that finished with a refusal.
    pub failed: u64,
    /// Jobs the caller stopped.
    pub cancelled: u64,
    /// Jobs that outlived their poll budget.
    pub expired: u64,
}

impl JobCounters {
    /// Where the counters live, relative to the workspace root.
    pub const FILE: &'static str = "target/reports/play-jobs.json";

    /// Count one job in one status.
    pub fn bump(&mut self, status: JobStatus) {
        match status {
            JobStatus::Pending => self.pending += 1,
            JobStatus::Running => self.running += 1,
            JobStatus::Completed => self.completed += 1,
            JobStatus::Failed => self.failed += 1,
            JobStatus::Cancelled => self.cancelled += 1,
            JobStatus::Expired => self.expired += 1,
        }
    }

    /// The count for one status.
    #[must_use]
    pub fn count(&self, status: JobStatus) -> u64 {
        match status {
            JobStatus::Pending => self.pending,
            JobStatus::Running => self.running,
            JobStatus::Completed => self.completed,
            JobStatus::Failed => self.failed,
            JobStatus::Cancelled => self.cancelled,
            JobStatus::Expired => self.expired,
        }
    }

    /// Every count, one line per status, in vocabulary order.
    #[must_use]
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        for status in JobStatus::ALL {
            out.push_str(&format!("{}: {}\n", status.as_str(), self.count(status)));
        }
        out
    }

    /// The total number of jobs counted.
    #[must_use]
    pub fn total(&self) -> u64 {
        JobStatus::ALL
            .into_iter()
            .map(|status| self.count(status))
            .sum()
    }

    /// Write the counters under `root` and return the relative path.
    ///
    /// The write is best effort for the caller: a job that already happened
    /// is never undone because the disk said no.
    pub fn write(&self, root: &std::path::Path) -> Result<String, McpToolError> {
        let dir = root.join("target/reports");
        let failed = |why: std::io::Error| {
            McpToolError::new(
                "counters_unwritten",
                format!("could not write {} ({why})", Self::FILE),
            )
        };
        std::fs::create_dir_all(&dir).map_err(failed)?;
        let body = rmcp::serde_json::to_string_pretty(self).unwrap_or_default();
        std::fs::write(root.join(Self::FILE), body).map_err(failed)?;
        Ok(Self::FILE.to_owned())
    }

    /// Read the counters back from `root`; `None` when there is no file.
    #[must_use]
    pub fn read(root: &std::path::Path) -> Option<Self> {
        let text = std::fs::read_to_string(root.join(Self::FILE)).ok()?;
        rmcp::serde_json::from_str(&text).ok()
    }
}

/// One match job: the eager result plus the handle protocol around it.
struct PlayJob {
    status: JobStatus,
    seed: u64,
    half_extent_m: f32,
    scene_rev: Option<u64>,
    scene_digest: Option<String>,
    ticks: u32,
    fighters: u8,
    checkpoints: Vec<Checkpoint>,
    logs: Vec<String>,
    truncated_logs: bool,
    polls_used: u64,
    ttl_polls: u64,
    // Measured at `start` only when the caller asked for `timing`; a job that
    // measured nothing answers `unknown` on a later timed poll.
    timing: Option<PlayTimingDto>,
}

/// The scene a started match plays: its world plus the pin digest of the scene
/// bytes it was built from, so the job manifest names the exact scene.
pub struct PinnedWorld {
    /// The match world built from the pinned scene revision.
    pub world: MatchWorld,
    /// Pin digest of the scene bytes at that revision.
    pub digest: String,
}

/// A host-supplied way to simulate a match, used instead of the plain arena run (the `rules`
/// pack plays a match with a script driving some fighter slots). It gets the validated config,
/// tape and the scene world (when `scene_rev` named one) and returns the checkpoints.
pub struct MatchRunner<'a> {
    /// One log line naming what drives the match (shown in the job log).
    pub note: String,
    /// The simulation.
    pub run: &'a RunMatch<'a>,
}

/// The signature of a [`MatchRunner`] simulation.
pub type RunMatch<'a> = dyn Fn(&ArenaConfig, &FighterTape, Option<&MatchWorld>) -> Result<Vec<Checkpoint>, McpToolError>
    + 'a;

/// Session job table: monotonic ids over a `BTreeMap` (stable order, no
/// wall clock in the ids). Bounded at [`MAX_JOBS`] live entries.
pub struct PlayJobs {
    next_id: u64,
    jobs: BTreeMap<u64, PlayJob>,
    /// Bounded ring of start wall times: read only when `timing` is asked.
    timing: play_session::TimingRing,
    /// AX28: the live job counters, backed by `target/reports/play-jobs.json`.
    counters: JobCounters,
    /// AX28: where the counters were last written; empty until a write
    /// landed.
    counters_path: String,
}

impl PlayJobs {
    /// Empty table; ids start at 1 so 0 stays an obvious missing handle.
    pub fn new() -> Self {
        Self {
            next_id: 1,
            jobs: BTreeMap::new(),
            timing: play_session::TimingRing::default(),
            counters: JobCounters::default(),
            counters_path: String::new(),
        }
    }

    /// The live job counters.
    #[must_use]
    pub fn counters(&self) -> &JobCounters {
        &self.counters
    }

    /// Reconcile the live counters with the file on disk.
    ///
    /// AX28: a session that starts after a crash keeps the jobs the earlier
    /// session counted, so the count names the work and not the process.
    /// The merged counts stay in memory until the next write. Read-only over
    /// the file: a missing or unreadable file changes nothing.
    pub fn reconcile_counters(&mut self, root: &std::path::Path) -> bool {
        let Some(on_disk) = JobCounters::read(root) else {
            return false;
        };
        for status in JobStatus::ALL {
            let seen = self.counters.count(status);
            let stored = on_disk.count(status);
            if stored > seen {
                match status {
                    JobStatus::Pending => self.counters.pending = stored,
                    JobStatus::Running => self.counters.running = stored,
                    JobStatus::Completed => self.counters.completed = stored,
                    JobStatus::Failed => self.counters.failed = stored,
                    JobStatus::Cancelled => self.counters.cancelled = stored,
                    JobStatus::Expired => self.counters.expired = stored,
                }
            }
        }
        true
    }

    /// Count one job and write the counters out.
    ///
    /// The write is best effort: a job that already happened is never
    /// undone because the disk said no, so a failed write leaves the live
    /// counts ahead of the file.
    fn count_job(&mut self, status: JobStatus) {
        self.counters.bump(status);
        if let Ok(path) = self.counters.write(std::path::Path::new(".")) {
            self.counters_path = path;
        }
    }

    /// Live entries (completed, cancelled and expired jobs still occupy
    /// their slot until the session ends).
    pub fn len(&self) -> usize {
        self.jobs.len()
    }

    /// True when no job was ever started on this table.
    pub fn is_empty(&self) -> bool {
        self.jobs.is_empty()
    }

    /// Dispatch `start|poll|cancel` from one tool shape; anything else names
    /// the legal actions.
    pub fn play_run(&mut self, args: &PlayRunArgs) -> Result<PlayRunOutput, McpToolError> {
        self.play_run_in(args, None)
    }

    /// [`play_run`](Self::play_run) with the match world of the scene
    /// revision the caller named (`scene_rev`), built by the host from its
    /// document ([`world_load::world_for_revision`]). A start that names a
    /// revision plays on the spine step around that world; without a world a
    /// named revision is refused (`no_scene`), and a world without a named
    /// revision is ignored.
    pub fn play_run_in(
        &mut self,
        args: &PlayRunArgs,
        world: Option<PinnedWorld>,
    ) -> Result<PlayRunOutput, McpToolError> {
        self.play_run_with(args, world, None)
    }

    /// [`play_run_in`](Self::play_run_in) with a host [`MatchRunner`] simulating the match
    /// of a `start` (poll and cancel ignore it).
    pub fn play_run_with(
        &mut self,
        args: &PlayRunArgs,
        world: Option<PinnedWorld>,
        runner: Option<&MatchRunner<'_>>,
    ) -> Result<PlayRunOutput, McpToolError> {
        let action = PlayAction::parse(&args.action)?;
        let wants_timing = args.timing.unwrap_or(false);
        match action {
            PlayAction::Start => self.start(args, world, runner),
            PlayAction::Poll => self.poll(action.job_id(args)?, wants_timing),
            PlayAction::Cancel => self.cancel(action.job_id(args)?, wants_timing),
        }
    }

    /// Validate, simulate eagerly, file a running handle. Invalid input
    /// files no job and names the legal values.
    fn start(
        &mut self,
        args: &PlayRunArgs,
        world: Option<PinnedWorld>,
        runner: Option<&MatchRunner<'_>>,
    ) -> Result<PlayRunOutput, McpToolError> {
        if self.jobs.len() >= MAX_JOBS {
            return Err(McpToolError::new(
                "job_table_full",
                format!("the session holds {MAX_JOBS} jobs already; poll or cancel one first"),
            ));
        }
        let ticks = args.ticks.ok_or_else(|| {
            McpToolError::new(
                "bad_ticks",
                format!("action `start` needs ticks; submit 1..={MAX_PLAY_TICKS}"),
            )
        })?;
        if ticks == 0 || ticks > MAX_PLAY_TICKS {
            return Err(McpToolError::new(
                "bad_ticks",
                format!("ticks {ticks} is outside 1..={MAX_PLAY_TICKS} (MCP play.run bound)"),
            ));
        }
        let fighters = args.fighters.ok_or_else(|| {
            McpToolError::new(
                "bad_fighters",
                format!(
                    "action `start` needs fighters; submit {MIN_FIGHTERS}..={} ",
                    engine_arena::MAX_PARTICIPANTS
                ),
            )
        })?;
        if !(MIN_FIGHTERS..=engine_arena::MAX_PARTICIPANTS).contains(&fighters) {
            return Err(McpToolError::new(
                "bad_fighters",
                format!(
                    "fighters {fighters} is outside {MIN_FIGHTERS}..={} ",
                    engine_arena::MAX_PARTICIPANTS
                ),
            ));
        }
        let half = args.half_extent_m.ok_or_else(|| {
            McpToolError::new(
                "bad_extent",
                "action `start` needs half_extent_m; submit 5.0..=100.0 meters",
            )
        })?;
        if !(5.0..=100.0).contains(&half) {
            return Err(McpToolError::new(
                "bad_extent",
                format!("half_extent_m {half} is outside 5.0..=100.0 meters"),
            ));
        }
        let ttl = args.ttl_polls.unwrap_or(DEFAULT_TTL_POLLS);
        if ttl > MAX_TTL_POLLS {
            return Err(McpToolError::new(
                "bad_ttl",
                format!("ttl_polls {ttl} exceeds {MAX_TTL_POLLS} (default {DEFAULT_TTL_POLLS})"),
            ));
        }
        let inputs = args.inputs.clone().unwrap_or_default();
        if inputs.len() > MAX_PLAY_INPUTS {
            return Err(McpToolError::new(
                "too_many_inputs",
                format!(
                    "inputs holds {} entries; at most {MAX_PLAY_INPUTS} per match",
                    inputs.len()
                ),
            ));
        }
        let tape = build_tape(&inputs, ticks, fighters)?;
        let seed = args.seed.unwrap_or(0);
        let config = ArenaConfig {
            seed,
            ticks,
            fighters,
            half_extent_m: half,
            table_version: 1,
        };
        if config.validate().is_err() {
            return Err(McpToolError::new(
                "invalid_config",
                format!(
                    "arena refused seed={} ticks={ticks} fighters={fighters} half={half}; \
                     checked ranges already passed, so report this tape verbatim",
                    config.seed
                ),
            ));
        }
        let (world, scene_digest) = match (args.scene_rev, world) {
            (Some(_), Some(pinned)) => (Some(pinned.world), Some(pinned.digest)),
            (Some(rev), None) => {
                return Err(McpToolError::new(
                    "no_scene",
                    format!(
                        "scene_rev {rev} needs the session scene; this host plays no scene                          (omit scene_rev for the empty arena)"
                    ),
                ));
            }
            (None, _) => (None, None),
        };
        let wants_timing = args.timing.unwrap_or(false);
        let (checkpoints, timing) = if wants_timing {
            let clock = std::time::Instant::now();
            let points = simulate(&config, &tape, runner, world.as_ref())?;
            self.timing
                .record_ms(clock.elapsed().as_secs_f64() * 1000.0);
            (points, Some(self.timing.report()))
        } else {
            (simulate(&config, &tape, runner, world.as_ref())?, None)
        };
        if checkpoints.is_empty() {
            return Err(McpToolError::new(
                "invalid_tape",
                "arena refused the tape; it passed MCP range checks, so report it verbatim",
            ));
        }

        let id = self.next_id;
        self.next_id += 1;
        let mut job = PlayJob {
            status: JobStatus::Running,
            seed,
            half_extent_m: half,
            scene_rev: args.scene_rev,
            scene_digest,
            ticks,
            fighters,
            checkpoints,
            logs: Vec::new(),
            truncated_logs: false,
            polls_used: 0,
            ttl_polls: ttl,
            timing,
        };
        push_log(
            &mut job,
            format!(
                "job {id} started: seed={} ticks={ticks} fighters={fighters} inputs={} scene_rev={}",
                config.seed,
                inputs.len(),
                args.scene_rev.map_or_else(|| "none".to_owned(), |rev| rev.to_string())
            ),
        );
        if let Some(runner) = runner {
            push_log(&mut job, runner.note.clone());
        }
        // One line per checkpoint: the bounded push drops the oldest once the
        // table is full and flags it, so long matches stay bounded.
        for index in 0..job.checkpoints.len() {
            let tick = job.checkpoints[index].tick;
            let hash = job.checkpoints[index].hash;
            push_log(&mut job, format!("tick {tick} hash {hash:016x}"));
        }
        if 1 + job.checkpoints.len() > MAX_JOB_LOGS {
            job.truncated_logs = true;
        }
        self.jobs.insert(id, job);
        // AX28: the filed job is counted as running and the counters hit the
        // disk.
        self.count_job(JobStatus::Running);
        let job = self.jobs.get(&id).expect("inserted job reads back");
        Ok(render_output(
            id,
            job,
            vec![format!("job {id} staged")],
            requested_timing(job, wants_timing),
            &self.counters_path,
        ))
    }

    /// Advance one running handle: first poll reports progress, second
    /// completes; terminal handles replay their status idempotently.
    fn poll(&mut self, id: u64, wants_timing: bool) -> Result<PlayRunOutput, McpToolError> {
        let job = self.jobs.get_mut(&id).ok_or_else(|| {
            McpToolError::new(
                "unknown_job",
                format!("job {id} is unknown; start a job with action=`start`"),
            )
        })?;
        let timing = requested_timing(job, wants_timing);
        match job.status {
            JobStatus::Completed | JobStatus::Cancelled | JobStatus::Expired => Ok(render_output(
                id,
                job,
                Vec::new(),
                timing,
                &self.counters_path,
            )),
            JobStatus::Pending | JobStatus::Failed => Err(McpToolError::new(
                "job_not_running",
                format!(
                    "job {id} is {}; only running jobs poll (action=`start` opens one)",
                    job.status.as_str()
                ),
            )),
            JobStatus::Running => {
                if job.polls_used >= job.ttl_polls {
                    job.status = JobStatus::Expired;
                    job.checkpoints = Vec::new();
                    let ttl = job.ttl_polls;
                    push_log(job, format!("job {id} expired after {ttl} polls"));
                    let job = self.jobs.get(&id).expect("expired job reads back");
                    let out = render_output(id, job, Vec::new(), timing, &self.counters_path);
                    // AX28: the expired job is counted and the counters hit
                    // the disk.
                    self.count_job(JobStatus::Expired);
                    return Ok(out);
                }
                job.polls_used += 1;
                if job.polls_used == 1 {
                    let ticks = job.ticks;
                    push_log(job, format!("job {id} simulating {ticks} ticks"));
                    let job = self.jobs.get(&id).expect("polled job reads back");
                    let mid = ticks / 2;
                    let mid_hash = job
                        .checkpoints
                        .iter()
                        .find(|point| point.tick == mid)
                        .map_or(0, |point| point.hash);
                    return Ok(render_output(
                        id,
                        job,
                        vec![
                            format!("match staged: {ticks} ticks x {} fighters", job.fighters),
                            format!("simulating ticks 0..{ticks}"),
                            format!("halfway checkpoint tick {mid} hash {mid_hash:016x}"),
                        ],
                        timing,
                        &self.counters_path,
                    ));
                }
                let done = job.checkpoints.len();
                let final_hash = job.checkpoints.last().map(|point| point.hash);
                job.status = JobStatus::Completed;
                push_log(
                    job,
                    match final_hash {
                        Some(hash) => {
                            format!("job {id} completed: {done} checkpoints final {hash:016x}")
                        }
                        None => format!("job {id} completed: {done} checkpoints"),
                    },
                );
                let job = self.jobs.get(&id).expect("completed job reads back");
                let out = render_output(id, job, Vec::new(), timing, &self.counters_path);
                // AX28: the completed job is counted and the counters hit the
                // disk.
                self.count_job(JobStatus::Completed);
                Ok(out)
            }
        }
    }

    /// Stop a running handle and drop its result; terminal handles refuse
    /// with the legal next step attached.
    fn cancel(&mut self, id: u64, wants_timing: bool) -> Result<PlayRunOutput, McpToolError> {
        let job = self.jobs.get_mut(&id).ok_or_else(|| {
            McpToolError::new(
                "unknown_job",
                format!("job {id} is unknown; start a job with action=`start`"),
            )
        })?;
        let timing = requested_timing(job, wants_timing);
        match job.status {
            JobStatus::Running => {
                job.status = JobStatus::Cancelled;
                job.checkpoints = Vec::new();
                push_log(job, format!("job {id} cancelled; result dropped"));
                let job = self.jobs.get(&id).expect("cancelled job reads back");
                let out = render_output(id, job, Vec::new(), timing, &self.counters_path);
                // AX28: the cancelled job is counted and the counters hit the
                // disk.
                self.count_job(JobStatus::Cancelled);
                Ok(out)
            }
            JobStatus::Completed => Err(McpToolError::new(
                "job_already_done",
                format!("job {id} already completed; action=`poll` reads its result"),
            )),
            terminal => Err(McpToolError::new(
                "job_not_running",
                format!(
                    "job {id} is {}; only running jobs cancel (action=`start` opens one)",
                    terminal.as_str()
                ),
            )),
        }
    }
}

impl Default for PlayJobs {
    fn default() -> Self {
        Self::new()
    }
}

/// Simulate one validated match: the host runner, the spine step around a
/// scene world, or the plain arena run.
fn simulate(
    config: &ArenaConfig,
    tape: &FighterTape,
    runner: Option<&MatchRunner<'_>>,
    world: Option<&MatchWorld>,
) -> Result<Vec<Checkpoint>, McpToolError> {
    match (runner, world) {
        (Some(runner), world) => (runner.run)(config, tape, world),
        (None, Some(world)) => Ok(run_spine(config, tape, world)),
        (None, None) => Ok(run(config, tape)),
    }
}

/// Validate DTO ranges (naming the legal values), then build the arena tape.
/// Tape order/duplicates are checked here; [`FighterTape::validate`] runs as
/// a backstop afterwards. Crate-visible: `play_session` steps reuse it so the
/// live duel honors the same tape contract as `play.run`.
pub(crate) fn build_tape(
    inputs: &[PlayInputDto],
    ticks: u32,
    fighters: u8,
) -> Result<FighterTape, McpToolError> {
    let mut prev: Option<(u32, u8)> = None;
    for (index, dto) in inputs.iter().enumerate() {
        if dto.tick >= ticks {
            return Err(McpToolError::new(
                "invalid_tape",
                format!(
                    "inputs[{index}].tick {} is past the match end {ticks}; ticks run 0..{ticks}",
                    dto.tick
                ),
            ));
        }
        if dto.fighter >= fighters {
            return Err(McpToolError::new(
                "invalid_tape",
                format!(
                    "inputs[{index}].fighter {} is outside 0..{fighters} (match runs {fighters} fighters)",
                    dto.fighter
                ),
            ));
        }
        if dto.mx < -WISH_RANGE || dto.mx > WISH_RANGE {
            return Err(McpToolError::new(
                "invalid_tape",
                format!(
                    "inputs[{index}].mx {} is outside -{WISH_RANGE}..={WISH_RANGE}",
                    dto.mx
                ),
            ));
        }
        if dto.mz < -WISH_RANGE || dto.mz > WISH_RANGE {
            return Err(McpToolError::new(
                "invalid_tape",
                format!(
                    "inputs[{index}].mz {} is outside -{WISH_RANGE}..={WISH_RANGE}",
                    dto.mz
                ),
            ));
        }
        if dto.facing < -FACING_RANGE || dto.facing > FACING_RANGE {
            return Err(McpToolError::new(
                "invalid_tape",
                format!(
                    "inputs[{index}].facing {} is outside -{FACING_RANGE}..={FACING_RANGE}",
                    dto.facing
                ),
            ));
        }
        if dto.buttons & !KNOWN_BUTTONS != 0 {
            return Err(McpToolError::new(
                "invalid_tape",
                format!(
                    "inputs[{index}].buttons 0x{:02x} holds unknown bits; \
                     known bits are 0x{KNOWN_BUTTONS:02x} \
                     (attack 0x01, block 0x02, dodge 0x04)",
                    dto.buttons
                ),
            ));
        }
        let key = (dto.tick, dto.fighter);
        if let Some(prev_key) = prev {
            if key < prev_key {
                return Err(McpToolError::new(
                    "unsorted_tape",
                    format!(
                        "inputs[{index}] runs before its predecessor; \
                         order inputs by (tick, fighter)"
                    ),
                ));
            }
            if key == prev_key {
                return Err(McpToolError::new(
                    "duplicate_input",
                    format!(
                        "inputs[{index}] repeats tick {} fighter {}; one input per slot tick",
                        dto.tick, dto.fighter
                    ),
                ));
            }
        }
        prev = Some(key);
    }
    let mut tape = Vec::with_capacity(inputs.len());
    for dto in inputs {
        let fighter = FighterId::new(dto.fighter).ok_or_else(|| {
            McpToolError::new(
                "invalid_tape",
                format!(
                    "fighter {} is outside 0..{fighters}; checked above, so report this tape verbatim",
                    dto.fighter
                ),
            )
        })?;
        let mut input = FighterInput {
            tick: dto.tick,
            fighter,
            mx: dto.mx,
            mz: dto.mz,
            facing: dto.facing,
            buttons: Default::default(),
        };
        // `Buttons` lives in engine-input (not an endpoint dep) and its
        // payload field is public: set it positionally and let tape
        // validation below reject unknown bits with a legal-values error.
        input.buttons.0 = dto.buttons;
        tape.push(input);
    }
    let tape = FighterTape(tape);
    tape.validate().map_err(|error| {
        McpToolError::new(
            "invalid_tape",
            format!("arena tape check failed ({error:?}); checked ranges already passed"),
        )
    })?;
    Ok(tape)
}

/// Bounded log push: lines truncate to [`MAX_LOG_LINE_CHARS`] chars, oldest
/// lines drop once full (flagged, never silent).
fn push_log(job: &mut PlayJob, line: String) {
    let line: String = line.chars().take(MAX_LOG_LINE_CHARS).collect();
    if job.logs.len() >= MAX_JOB_LOGS {
        job.logs.remove(0);
        job.truncated_logs = true;
    }
    job.logs.push(line);
}

/// Render one tool output for a stored job; progress events ride on running
/// polls, the terminal line on finished ones. `timing` is the opt-in wall-time
/// receipt this call asked for, `None` keeps the bytes as before PF24.
fn render_output(
    id: u64,
    job: &PlayJob,
    events: Vec<String>,
    timing: Option<PlayTimingDto>,
    counters: &str,
) -> PlayRunOutput {
    let (progress, events, checkpoints, final_hash) = match job.status {
        JobStatus::Running => {
            let progress = if job.polls_used == 0 { 0.0 } else { 0.5 };
            (progress, events, Vec::new(), None)
        }
        JobStatus::Completed => {
            let checkpoints = job
                .checkpoints
                .iter()
                .map(|point| CheckpointDto {
                    tick: point.tick,
                    hash: point.hash,
                })
                .collect::<Vec<_>>();
            let final_hash = job.checkpoints.last().map(|point| point.hash);
            (
                1.0,
                vec![format!("match complete: {} checkpoints", checkpoints.len())],
                checkpoints,
                final_hash,
            )
        }
        JobStatus::Cancelled => (
            0.0,
            vec!["job cancelled; result dropped".to_owned()],
            Vec::new(),
            None,
        ),
        JobStatus::Expired => (
            0.0,
            vec![format!(
                "job expired after {} polls; result dropped",
                job.ttl_polls
            )],
            Vec::new(),
            None,
        ),
        JobStatus::Pending => (
            0.0,
            vec!["job pending; no result yet".to_owned()],
            Vec::new(),
            None,
        ),
        JobStatus::Failed => (
            0.0,
            vec!["job failed; result dropped".to_owned()],
            Vec::new(),
            None,
        ),
    };
    PlayRunOutput {
        job_id: id,
        status: job.status.as_str().to_owned(),
        progress,
        events,
        logs: job.logs.clone(),
        truncated_logs: job.truncated_logs,
        tick_count: job.ticks,
        final_hash,
        checkpoints,
        seed: job.seed,
        half_extent_m: job.half_extent_m,
        scene_rev: job.scene_rev,
        scene_digest: job.scene_digest.clone(),
        timing,
        counters: counters.to_owned(),
    }
}

/// The timing receipt this call asked for: the job's own measurement when its
/// start made one, else an `unknown` object (a run of zero measured steps).
fn requested_timing(job: &PlayJob, wants: bool) -> Option<PlayTimingDto> {
    wants.then(|| job.timing.clone().unwrap_or_else(PlayTimingDto::unknown))
}

// ---------------------------------------------------------------------------
// Cross-tool check: a `play.step` composition replays a `play.run` job only
// when both pin the same seed and half-extent, with the step inside the
// run's tick range. Any drift fails with the differing scalars named, so
// cross-tool runs pass or fail honestly, never by silent default.
// ---------------------------------------------------------------------------

/// Resolve the step's half-extent against the same default `play.step`
/// uses, refusing non-finite values before any bit comparison.
fn step_half_extent_m(step: &play_session::PlayStepArgs) -> Result<f32, McpToolError> {
    let half = step.half_extent_m.unwrap_or(20.0);
    if !half.is_finite() {
        return Err(McpToolError::new(
            "bad_extent",
            format!("play.step half_extent_m {half} is not finite; submit 5.0..=100.0 meters"),
        ));
    }
    Ok(half)
}

/// Check one `play.step` composition against a finished `play.run` receipt:
/// identical seeds, bit-identical half-extents (meters) and `upto_tick`
/// inside the run's `1..=tick_count`. Owner: endpoint (pure comparison, no
/// simulation, no shared state; single-threaded caller context).
/// Determinism: pure function of the two compositions; float comparison is
/// on `to_bits`, never tolerance. Errors: `manifest_mismatch` naming both
/// sides on any seed/extent/range drift, `bad_ticks` when the step omits
/// `upto_tick`, `bad_extent` on a non-finite step extent. Every mismatch
/// names both sides, so a replay across tools is either exact or loudly
/// refused.
pub fn check_run_against_step(
    run: &PlayRunOutput,
    step: &play_session::PlayStepArgs,
) -> Result<(), McpToolError> {
    if let Some(rev) = run.scene_rev {
        return Err(McpToolError::new(
            "manifest_mismatch",
            format!(
                "play.run job {} played scene revision {rev}; play.step plays the empty                  arena, so the two cannot replay each other",
                run.job_id
            ),
        ));
    }
    let seed = step.seed.unwrap_or(0);
    if seed != run.seed {
        return Err(McpToolError::new(
            "manifest_mismatch",
            format!(
                "play.step seed {seed} differs from play.run job {} seed {}; replay needs identical seeds",
                run.job_id, run.seed
            ),
        ));
    }
    let half = step_half_extent_m(step)?;
    if half.to_bits() != run.half_extent_m.to_bits() {
        return Err(McpToolError::new(
            "manifest_mismatch",
            format!(
                "play.step half_extent_m {half} differs from play.run job {} half_extent_m {}; replay needs identical extents",
                run.job_id, run.half_extent_m
            ),
        ));
    }
    match step.upto_tick {
        Some(upto) if (1..=run.tick_count).contains(&upto) => Ok(()),
        Some(upto) => Err(McpToolError::new(
            "manifest_mismatch",
            format!(
                "play.step upto_tick {upto} sits outside play.run job {} ticks 1..={}; replay needs the step inside the run",
                run.job_id, run.tick_count
            ),
        )),
        None => Err(McpToolError::new(
            "bad_ticks",
            format!(
                "play.step needs upto_tick; submit 1..={} (the run's tick range)",
                run.tick_count
            ),
        )),
    }
}

// ---------------------------------------------------------------------------
// Capture: render-free receipt, unsupported unless a callback is wired.
// ---------------------------------------------------------------------------

/// Validate receipt dimensions against the R1 bound, then cap the longest side
/// at `max_size` (default 768, clamped 200..=2048) before encode. Returns the
/// effective size the renderer must cover.
fn capture_size(args: &CaptureArgs) -> Result<(u32, u32), McpToolError> {
    let width = args.width.unwrap_or(CAPTURE_DEFAULT_WIDTH);
    let height = args.height.unwrap_or(CAPTURE_DEFAULT_HEIGHT);
    if width == 0 || height == 0 {
        return Err(McpToolError::new(
            "bad_size",
            format!("capture size {width}x{height} is zero; submit 1..={CAPTURE_MAX_DIM} each"),
        ));
    }
    if width > CAPTURE_MAX_DIM || height > CAPTURE_MAX_DIM {
        return Err(McpToolError::new(
            "bad_size",
            format!("capture size {width}x{height} exceeds {CAPTURE_MAX_DIM}"),
        ));
    }
    let max_size = args
        .max_size
        .unwrap_or(CAPTURE_DEFAULT_MAX_SIZE)
        .clamp(CAPTURE_MIN_MAX_SIZE, CAPTURE_MAX_DIM);
    Ok((width.min(max_size), height.min(max_size)))
}

/// Unwired capture: structured unsupported, never a success shape. The
/// endpoint owns no renderer; only [`capture_with`] (caller-supplied
/// `FnOnce`) can produce a receipt.
pub fn capture_unsupported(args: &CaptureArgs) -> Result<CaptureOutput, McpToolError> {
    let _ = capture_size(args)?;
    Err(McpToolError::new(
        "unsupported_capture",
        "no renderer is wired: capture needs a caller-supplied callback \
         (see capture_with); absent capture is unsupported, never success",
    ))
}

/// One-shot renderer handed to [`capture_with`]; the callback owns the pixels.
pub type CaptureRenderer<'a> =
    Box<dyn FnOnce(&CaptureArgs) -> Result<CaptureOutput, McpToolError> + 'a>;

/// Optional per-receipt PNG notifier handed to [`capture_with_look_hook`].
pub type LookHook<'a> = Option<&'a mut dyn FnMut(&[u8])>;

/// Per-frame look hook: hands each validated capture PNG to the hook once.
///
/// `None` keeps the receipt path byte-identical; a refused receipt never
/// notifies. Units: PNG bytes in, one call per receipt. Budget: one cold call.
///
/// Caller: `capture_with` (same file) plus `arena.rs` `capture_offscreen` (remainder).
pub fn capture_with_look_hook(
    args: &CaptureArgs,
    render: CaptureRenderer<'_>,
    hook: LookHook<'_>,
) -> Result<CaptureOutput, McpToolError> {
    let (width, height) = capture_size(args)?;
    // The renderer is asked for the effective size, so the `max_size` cap is
    // honoured by the callback too (not only by the manifest check below).
    let asked = CaptureArgs {
        width: Some(width),
        height: Some(height),
        max_size: args.max_size,
    };
    let mut out = render(&asked)?;
    if out.manifest.width != width || out.manifest.height != height {
        let dots = abstract_dot_positions(width, height);
        return Err(McpToolError::new(
            "bad_receipt",
            format!(
                "receipt covers {}x{}, capture asked {width}x{height}; duel shows {} at {dots:?} (span {}px, bars {}px, flash {} ticks)",
                out.manifest.width,
                out.manifest.height,
                abstract_dots_tag(),
                legend_span_px(width),
                legend_bar_h_px(height),
                jab_flash_ticks(),
            ),
        ));
    }
    for (field, value) in [
        ("adapter", &out.manifest.adapter),
        ("driver", &out.manifest.driver),
    ] {
        if value.is_empty() {
            return Err(McpToolError::new(
                "bad_receipt",
                format!(
                    "receipt {field} is empty; the renderer must name the device; duel shows {}",
                    abstract_dots_tag(),
                ),
            ));
        }
    }
    // R1 bounds adapter/driver strings at 256 chars; mirror that here.
    out.manifest.adapter = out.manifest.adapter.chars().take(256).collect();
    out.manifest.driver = out.manifest.driver.chars().take(256).collect();
    if out.png_bytes.len() > MAX_CAPTURE_BYTES {
        return Err(McpToolError::new(
            "bad_receipt",
            format!(
                "receipt holds {} png bytes; at most {MAX_CAPTURE_BYTES}; duel shows {}",
                out.png_bytes.len(),
                abstract_dots_tag(),
            ),
        ));
    }
    if out.png_bytes.len() < PNG_MAGIC.len() || out.png_bytes[..PNG_MAGIC.len()] != PNG_MAGIC {
        return Err(McpToolError::new(
            "bad_receipt",
            format!(
                "receipt bytes are not a PNG (missing the 8-byte magic); duel shows {}",
                abstract_dots_tag(),
            ),
        ));
    }
    if let Some(hook) = hook {
        hook(&out.png_bytes);
    }
    Ok(out)
}

/// Wired capture: run the caller's one-shot renderer callback, then validate
/// its render-free receipt (dimensions match, adapter/driver named and
/// bounded like R1, PNG magic plus byte bound). The endpoint never imports
/// render; the callback owns the pixels.
pub fn capture_with(
    args: &CaptureArgs,
    render: CaptureRenderer<'_>,
) -> Result<CaptureOutput, McpToolError> {
    capture_with_look_hook(args, render, None)
}

// ---------------------------------------------------------------------------
// AX27: the MCP frame carrier (one image block plus a caption).
// ---------------------------------------------------------------------------

/// Standard base64 with padding (RFC 4648): the image block carries text, never
/// the raw byte array.
fn b64_encode(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len() / 3 * 4 + 4);
    for chunk in bytes.chunks(3) {
        let packed = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        let digit =
            |shift: u32| char::from(ALPHABET[usize::try_from((packed >> shift) & 63).unwrap_or(0)]);
        out.push(digit(18));
        out.push(digit(12));
        out.push(if chunk.len() > 1 { digit(6) } else { '=' });
        out.push(if chunk.len() > 2 { digit(0) } else { '=' });
    }
    out
}

/// Lowercase hex of a byte digest (64 chars for a SHA-256).
pub(crate) fn hex_bytes(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(char::from(DIGITS[usize::from(byte >> 4)]));
        out.push(char::from(DIGITS[usize::from(byte & 0x0F)]));
    }
    out
}

/// The MCP answer for one frame receipt JSON (`manifest` plus a `png_bytes`
/// array): one `image/png` block, one caption of at most
/// [`FRAME_CAPTION_MAX_CHARS`], and `structuredContent` = the receipt without
/// `png_bytes`, its `manifest.sha256` set to the SHA-256 of the exact PNG bytes
/// (`engine-game` owns the `sha2` digest). A value with no readable `png_bytes`
/// passes through as plain structured content, so non-frame verbs are untouched.
///
/// Caller: the `capture` and `play.step` arms of `mcp.rs`.
pub fn frame_call_result(mut value: Value) -> CallToolResult {
    let Some(object) = value.as_object_mut() else {
        return CallToolResult::structured(value);
    };
    let Some(png) = object
        .remove("png_bytes")
        .and_then(|raw| rmcp::serde_json::from_value::<Vec<u8>>(raw).ok())
    else {
        return CallToolResult::structured(value);
    };
    let sha256 = hex_bytes(&engine_game::camera_feed::showcase_sha256(&png));
    if let Some(Value::Object(manifest)) = object.get_mut("manifest") {
        manifest.insert("sha256".to_owned(), Value::String(sha256.clone()));
    }
    let caption: String = format!("frame: image/png, {} bytes, sha256 {sha256}", png.len())
        .chars()
        .take(FRAME_CAPTION_MAX_CHARS)
        .collect();
    let mut result = CallToolResult::success(vec![
        ContentBlock::image(b64_encode(&png), "image/png"),
        ContentBlock::text(caption),
    ]);
    result.structured_content = Some(value);
    result
}

// ---------------------------------------------------------------------------
// PT2 capture-draw contract: ABSTRACT-DOTS + legend + jab flash.
// ---------------------------------------------------------------------------
// Duel `play.step` captures draw CT7 fighter silhouettes as ABSTRACT-DOTS
// with a legend (dot color per fighter plus hp bars) and a kp01 jab flash
// on a recent hit. The software rasterizer in `play_session` implements it;
// these helpers name its geometry so receipts and refusals agree.

/// Duel dots tag naming the CT7 silhouette fallback and the kp01 jab flash.
fn abstract_dots_tag() -> &'static str {
    "ABSTRACT-DOTS + legend (CT7 silhouettes) with kp01 jab flash"
}

/// Legend span in pixels, mirroring the hp-bar span (`w - 12`) in `play_session`.
fn legend_span_px(width: u32) -> u32 {
    width.saturating_sub(12)
}

/// Legend bar height in pixels, mirroring `(h / 8).clamp(1, 3)` in `play_session`.
fn legend_bar_h_px(height: u32) -> u32 {
    (height / 8).clamp(1, 3)
}

/// Jab flash window in ticks, mirroring `HIT_FLASH_TICKS` (kp01 jab flash).
fn jab_flash_ticks() -> u32 {
    8
}

/// Deterministic ABSTRACT-DOTS positions for two fighters from the receipt
/// size, via the shared `splitmix64` stream (integer math only).
fn abstract_dot_positions(width: u32, height: u32) -> [(u32, u32); 2] {
    let mut rng = u64::from(width).wrapping_add(u64::from(height).wrapping_mul(GOLDEN_GAMMA));
    let ax = splitmix64(rng);
    rng = rng.wrapping_add(GOLDEN_GAMMA);
    let ay = splitmix64(rng);
    rng = rng.wrapping_add(GOLDEN_GAMMA);
    let bx = splitmix64(rng);
    rng = rng.wrapping_add(GOLDEN_GAMMA);
    let by = splitmix64(rng);
    let span_w = u64::from(width.saturating_sub(12)).max(1);
    let span_h = u64::from(height.saturating_sub(12)).max(1);
    let dot = |v: u64, span: u64| -> u32 {
        let off = v % span;
        u32::try_from(6u64.saturating_add(off)).unwrap_or(6)
    };
    [
        (dot(ax, span_w), dot(ay, span_h)),
        (dot(bx, span_w), dot(by, span_h)),
    ]
}

// ---------------------------------------------------------------------------
// rmcp tools: one SyncTool per contract tool, sync so tests need no executor.
// ---------------------------------------------------------------------------

/// `play.run` tool: start, poll or cancel a headless arena match job.
pub struct PlayRunTool;

impl ToolBase for PlayRunTool {
    type Parameter = PlayRunArgs;
    type Output = PlayRunOutput;
    type Error = McpToolError;

    fn name() -> Cow<'static, str> {
        TOOL_PLAY_RUN.into()
    }

    fn description() -> Option<Cow<'static, str>> {
        Some(
            "Headless arena match job: action=start runs a bounded match, \
             poll reads progress then the checkpoints, cancel stops a running job."
                .into(),
        )
    }

    fn annotations() -> Option<ToolAnnotations> {
        crate::mcp::write_annotations(false)
    }
}

impl SyncTool<McpServer> for PlayRunTool {
    fn invoke(service: &McpServer, param: Self::Parameter) -> Result<Self::Output, Self::Error> {
        service.dispatch(TOOL_PLAY_RUN, |state| state.play_run(&param))
    }
}

/// `capture` tool: renderer receipt, or structured unsupported when unwired.
pub struct CaptureTool;

impl ToolBase for CaptureTool {
    type Parameter = CaptureArgs;
    type Output = CaptureOutput;
    type Error = McpToolError;

    fn name() -> Cow<'static, str> {
        TOOL_CAPTURE.into()
    }

    fn description() -> Option<Cow<'static, str>> {
        Some(
            "Renderer receipt (manifest plus PNG bytes) when a capture \
             callback is wired; structured unsupported otherwise."
                .into(),
        )
    }

    fn annotations() -> Option<ToolAnnotations> {
        crate::mcp::read_only_annotations()
    }
}

impl SyncTool<McpServer> for CaptureTool {
    fn invoke(service: &McpServer, param: Self::Parameter) -> Result<Self::Output, Self::Error> {
        service.dispatch(TOOL_CAPTURE, |state| state.capture(&param))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_editor::capability::Capability;

    /// A session at revision 1 whose scene plays as the empty arena (the one
    /// node hangs above head height), so a start has a revision to pin.
    fn server() -> McpServer {
        use engine_editor::commands::{self, Batch, Document, Journal, Op};
        let (mut doc, mut journal) = (Document::new(), Journal::new());
        let marker = Batch {
            ops: vec![Op::Insert {
                // A fixed id keeps the scene digest equal across sessions.
                id: engine_scene::ids::ObjectId::parse("00000000-0000-0000-0000-000000000001")
                    .expect("fixed marker id parses"),
                parent: engine_scene::ids::ObjectId::NIL,
                local: engine_scene::transform::Transform::new(
                    [0.0, 10.0, 0.0],
                    [0.0, 0.0, 0.0, 1.0],
                    [1.0, 1.0, 1.0],
                ),
            }],
            base_revision: 0,
            idempotency_key: 1,
            provenance: None,
        };
        commands::apply(&mut doc, &mut journal, &marker).expect("marker applies");
        McpServer::with_document(doc, journal, 7, Capability::unrestricted(7))
    }

    fn start_args(ticks: u32) -> PlayRunArgs {
        PlayRunArgs {
            action: "start".to_owned(),
            seed: Some(2040),
            ticks: Some(ticks),
            fighters: Some(2),
            half_extent_m: Some(20.0),
            inputs: Some(Vec::new()),
            ttl_polls: None,
            job_id: None,
            scene_rev: Some(1),
            timing: None,
        }
    }

    /// SC2: a start without a live pin is refused by name before any job is
    /// filed; a pinned start carries the scene pin in its manifest.
    #[test]
    fn start_pins_the_scene_or_refuses_before_any_job() {
        let server = server();
        let mut unpinned = start_args(8);
        unpinned.scene_rev = None;
        assert_eq!(
            invoke(&server, unpinned).expect_err("unpinned").code,
            "unpinned_run"
        );
        let mut stale = start_args(8);
        stale.scene_rev = Some(7);
        assert_eq!(
            invoke(&server, stale).expect_err("stale").code,
            "stale_scene_rev"
        );
        // A session that never authored anything is revision 0: the empty
        // arena never plays, even when the caller names a revision.
        let empty = McpServer::new(7, Capability::unrestricted(7));
        for scene_rev in [None, Some(0)] {
            let mut args = start_args(8);
            args.scene_rev = scene_rev;
            assert_eq!(
                invoke(&empty, args).expect_err("empty arena").code,
                "empty_arena"
            );
        }
        // Refusals filed nothing.
        assert_eq!(
            invoke(&server, run_action("poll", Some(1)))
                .expect_err("no job was filed")
                .code,
            "unknown_job"
        );
        let started = invoke(&server, start_args(8)).expect("pinned start runs");
        assert_eq!(started.scene_rev, Some(1));
        let digest = started.scene_digest.expect("manifest carries the digest");
        assert_eq!(digest.len(), 64, "blake3 hex digest: {digest}");
        let polled = invoke(&server, run_action("poll", Some(started.job_id))).expect("polls");
        assert_eq!(
            polled.scene_digest,
            Some(digest),
            "every status echoes the pin"
        );
    }

    fn run_action(action: &str, job_id: Option<u64>) -> PlayRunArgs {
        PlayRunArgs {
            action: action.to_owned(),
            seed: None,
            ticks: None,
            fighters: None,
            half_extent_m: None,
            inputs: None,
            ttl_polls: None,
            job_id,
            scene_rev: None,
            timing: None,
        }
    }

    fn invoke(server: &McpServer, args: PlayRunArgs) -> Result<PlayRunOutput, McpToolError> {
        PlayRunTool::invoke(server, args)
    }

    #[test]
    fn play_lifecycle_start_poll_progress_result() {
        let server = server();
        let started = invoke(&server, start_args(32)).expect("start validates");
        assert_eq!(started.status, "running");
        assert_eq!(started.progress, 0.0);
        assert!(started.checkpoints.is_empty());
        assert!(started.final_hash.is_none());
        let id = started.job_id;
        assert_ne!(id, 0);

        // First poll: still running, halfway progress events, bounded logs.
        let progress = invoke(&server, run_action("poll", Some(id))).expect("first poll runs");
        assert_eq!(progress.job_id, id);
        assert_eq!(progress.status, "running");
        assert_eq!(progress.progress, 0.5);
        assert_eq!(progress.events.len(), 3);
        assert!(progress.events[2].contains("halfway"));
        assert!(progress.logs.len() <= MAX_JOB_LOGS);

        // Second poll: completed with the deterministic checkpoints.
        let done = invoke(&server, run_action("poll", Some(id))).expect("second poll completes");
        assert_eq!(done.status, "completed");
        assert_eq!(done.progress, 1.0);
        assert_eq!(done.checkpoints.len(), 32);
        assert_eq!(done.tick_count, 32);
        let config = ArenaConfig {
            seed: 2040,
            ticks: 32,
            fighters: 2,
            half_extent_m: 20.0,
            table_version: 1,
        };
        let direct = run(&config, &FighterTape(Vec::new()));
        assert_eq!(done.checkpoints.len(), direct.len());
        for (dto, point) in done.checkpoints.iter().zip(direct.iter()) {
            assert_eq!(dto.tick, point.tick);
            assert_eq!(dto.hash, point.hash);
        }
        assert_eq!(done.final_hash, Some(direct.last().expect("32 ticks").hash));

        // Terminal polls replay the result idempotently.
        let replay = invoke(&server, run_action("poll", Some(id))).expect("replay polls");
        assert_eq!(replay, done);
    }

    #[test]
    fn play_cancel_stops_a_running_job() {
        let server = server();
        let started = invoke(&server, start_args(16)).expect("start validates");
        let id = started.job_id;

        let stopped = invoke(&server, run_action("cancel", Some(id))).expect("cancel stops");
        assert_eq!(stopped.status, "cancelled");
        assert!(stopped.checkpoints.is_empty());
        assert!(stopped.final_hash.is_none());

        // Polls after cancel replay cancelled, never the result.
        let after = invoke(&server, run_action("poll", Some(id))).expect("poll replays");
        assert_eq!(after.status, "cancelled");
        assert!(after.checkpoints.is_empty());

        // Cancelling a terminal job refuses with the legal next step.
        let refused = invoke(&server, run_action("cancel", Some(id))).expect_err("recancel fails");
        assert_eq!(refused.code, "job_not_running");
        assert!(refused.message.contains("cancelled"));

        // Cancelling a completed job points at poll, never revives it.
        let second = invoke(&server, start_args(8)).expect("second start validates");
        let second_id = second.job_id;
        invoke(&server, run_action("poll", Some(second_id))).expect("progress poll");
        invoke(&server, run_action("poll", Some(second_id))).expect("completing poll");
        let late = invoke(&server, run_action("cancel", Some(second_id)))
            .expect_err("cancel after done fails");
        assert_eq!(late.code, "job_already_done");
        assert!(late.message.contains("poll"));
    }

    /// AX28 (the landed half of MCX-7): a play job is a handle with a status
    /// vocabulary, the job digest equals the blocking run's digest, and a
    /// cancel of a finished job is refused by name. The scene never moves: a
    /// job reads the scene, it never writes it.
    #[test]
    fn task_play_run_returns_a_handle_with_a_status_and_counters() {
        let server = server();
        let scene_before = server.lock().expect("lock").doc.revision;
        let started = invoke(&server, start_args(32)).expect("start validates");
        let id = started.job_id;
        assert_ne!(id, 0, "the handle comes back");
        assert_eq!(started.status, "running");

        // The status vocabulary is the six AX28 words and nothing else.
        let words: Vec<&str> = JobStatus::ALL
            .iter()
            .map(|status| status.as_str())
            .collect();
        assert_eq!(
            words,
            vec![
                "pending",
                "running",
                "completed",
                "failed",
                "cancelled",
                "expired"
            ]
        );
        for status in JobStatus::ALL {
            assert_eq!(JobStatus::parse(status.as_str()), Some(status));
            assert_eq!(
                status.is_terminal(),
                !matches!(status, JobStatus::Pending | JobStatus::Running),
                "{status:?} terminal word"
            );
        }

        // The job digest equals the digest of the blocking run.
        invoke(&server, run_action("poll", Some(id))).expect("progress poll");
        let done = invoke(&server, run_action("poll", Some(id))).expect("completing poll");
        assert_eq!(done.status, "completed");
        let config = ArenaConfig {
            seed: 2040,
            ticks: 32,
            fighters: 2,
            half_extent_m: 20.0,
            table_version: 1,
        };
        let direct = run(&config, &FighterTape(Vec::new()));
        assert_eq!(done.final_hash, Some(direct.last().expect("32 ticks").hash));

        // A cancel of a finished job is refused by name and moves nothing.
        let refused = invoke(&server, run_action("cancel", Some(id))).expect_err("cancel refuses");
        assert_eq!(refused.code, "job_already_done");
        assert!(
            refused.message.contains("poll"),
            "the refusal names the legal next step: {}",
            refused.message
        );
        assert_eq!(
            server.lock().expect("lock").doc.revision,
            scene_before,
            "a job and a cancel never touch the scene"
        );

        // The answer names the counters file, and the live counters counted
        // this job under its status words.
        assert_eq!(started.counters, JobCounters::FILE);
        let live = server.lock().expect("lock").jobs.counters().clone();
        assert!(
            live.count(JobStatus::Running) >= 1,
            "the started job was counted: {live:?}"
        );
        assert!(
            live.count(JobStatus::Completed) >= 1,
            "the completed job was counted: {live:?}"
        );

        // The counters round trip through the disk under their own path.
        let root = std::env::temp_dir().join(format!("play-counters-{}", std::process::id()));
        let written = JobCounters {
            pending: 1,
            running: 2,
            completed: 3,
            failed: 4,
            cancelled: 5,
            expired: 6,
        }
        .write(&root)
        .expect("counters write");
        assert_eq!(written, JobCounters::FILE);
        let read_back = JobCounters::read(&root).expect("counters read back");
        assert_eq!(read_back.completed, 3);
        assert_eq!(read_back.total(), 21);

        println!(
            "task_play_run: job handle returned, digest equal to blocking run, \
             cancel refused the job PASS"
        );
    }

    #[test]
    fn play_expiry_fires_on_poll_horizon() {
        let server = server();
        let mut args = start_args(16);
        args.ttl_polls = Some(1);
        let started = invoke(&server, args).expect("start validates");
        let id = started.job_id;

        let progress = invoke(&server, run_action("poll", Some(id))).expect("first poll runs");
        assert_eq!(progress.status, "running");

        // The horizon (1 poll) is spent: the next poll expires, dropping the
        // result instead of completing.
        let expired = invoke(&server, run_action("poll", Some(id))).expect("expiry polls");
        assert_eq!(expired.status, "expired");
        assert!(expired.checkpoints.is_empty());
        assert!(expired.final_hash.is_none());
        assert!(expired.events.iter().any(|line| line.contains("expired")));
    }

    #[test]
    fn play_logs_stay_bounded_on_long_matches() {
        let server = server();
        let started = invoke(&server, start_args(64)).expect("start validates");
        let id = started.job_id;
        assert!(
            started.truncated_logs,
            "64 checkpoints over {MAX_JOB_LOGS} log lines must flag truncation"
        );
        assert!(started.logs.len() <= MAX_JOB_LOGS);
        for line in &started.logs {
            assert!(line.chars().count() <= MAX_LOG_LINE_CHARS);
        }
        invoke(&server, run_action("poll", Some(id))).expect("progress poll");
        let done = invoke(&server, run_action("poll", Some(id))).expect("completing poll");
        assert_eq!(done.status, "completed");
        assert!(done.truncated_logs);
        assert!(done.logs.len() <= MAX_JOB_LOGS);
    }

    #[test]
    fn play_action_parses_the_three_verbs_and_refuses_the_rest() {
        assert_eq!(PlayAction::parse("start"), Ok(PlayAction::Start));
        assert_eq!(PlayAction::parse("poll"), Ok(PlayAction::Poll));
        assert_eq!(PlayAction::parse("cancel"), Ok(PlayAction::Cancel));
        for bad in ["", "Start", "launch", "poll "] {
            let error = PlayAction::parse(bad).expect_err("unknown verb refuses");
            assert_eq!(error.code, "bad_action", "verb `{bad}`");
            assert!(error.message.contains("start, poll or cancel"));
        }
        let error = PlayAction::Cancel
            .job_id(&PlayRunArgs::default())
            .expect_err("cancel needs an id");
        assert_eq!(error.code, "bad_job_id");
        assert!(error.message.contains("`cancel`"));
    }

    #[test]
    fn play_rejects_bad_actions_ids_configs_and_tapes() {
        let server = server();

        let bad_action = invoke(&server, run_action("launch", None)).expect_err("unknown action");
        assert_eq!(bad_action.code, "bad_action");
        assert!(bad_action.message.contains("start"));

        let missing_id = invoke(&server, run_action("poll", None)).expect_err("poll needs id");
        assert_eq!(missing_id.code, "bad_job_id");

        let unknown = invoke(&server, run_action("poll", Some(999))).expect_err("unknown job");
        assert_eq!(unknown.code, "unknown_job");
        assert!(unknown.message.contains("start"));

        // Config ranges name the legal values.
        let mut ticks = start_args(0);
        ticks.ticks = Some(0);
        assert_eq!(
            invoke(&server, ticks).expect_err("zero ticks").code,
            "bad_ticks"
        );
        let mut many_ticks = start_args(8);
        many_ticks.ticks = Some(MAX_PLAY_TICKS + 1);
        let refused = invoke(&server, many_ticks).expect_err("overlong match");
        assert_eq!(refused.code, "bad_ticks");
        assert!(refused.message.contains(&MAX_PLAY_TICKS.to_string()));

        let mut one_fighter = start_args(8);
        one_fighter.fighters = Some(1);
        let refused = invoke(&server, one_fighter).expect_err("solo fighter");
        assert_eq!(refused.code, "bad_fighters");
        assert!(refused.message.contains('2'));

        let mut nan_extent = start_args(8);
        nan_extent.half_extent_m = Some(f32::NAN);
        assert_eq!(
            invoke(&server, nan_extent).expect_err("NaN extent").code,
            "bad_extent"
        );

        let mut huge_ttl = start_args(8);
        huge_ttl.ttl_polls = Some(MAX_TTL_POLLS + 1);
        assert_eq!(
            invoke(&server, huge_ttl).expect_err("huge ttl").code,
            "bad_ttl"
        );

        // Tape ranges name the legal values.
        let bad_button = PlayInputDto {
            tick: 0,
            fighter: 0,
            mx: 0,
            mz: 0,
            facing: 0,
            buttons: 0x08,
        };
        let mut with_buttons = start_args(8);
        with_buttons.inputs = Some(vec![bad_button]);
        let refused = invoke(&server, with_buttons).expect_err("unknown buttons");
        assert_eq!(refused.code, "invalid_tape");
        assert!(refused.message.contains("0x07"));

        let past_end = PlayInputDto {
            tick: 8,
            fighter: 0,
            mx: 0,
            mz: 0,
            facing: 0,
            buttons: 0,
        };
        let mut with_tick = start_args(8);
        with_tick.inputs = Some(vec![past_end]);
        let refused = invoke(&server, with_tick).expect_err("tick past end");
        assert_eq!(refused.code, "invalid_tape");
        assert!(refused.message.contains("0..8"));

        let wish = PlayInputDto {
            tick: 0,
            fighter: 0,
            mx: 101,
            mz: 0,
            facing: 0,
            buttons: 0,
        };
        let mut with_wish = start_args(8);
        with_wish.inputs = Some(vec![wish]);
        assert_eq!(
            invoke(&server, with_wish)
                .expect_err("wish out of range")
                .code,
            "invalid_tape"
        );

        // Tape order and duplicates fail loudly.
        let first = PlayInputDto {
            tick: 1,
            fighter: 0,
            mx: 0,
            mz: 0,
            facing: 0,
            buttons: 0,
        };
        let zeroth = PlayInputDto { tick: 0, ..first };
        let mut unsorted = start_args(8);
        unsorted.inputs = Some(vec![first, zeroth]);
        assert_eq!(
            invoke(&server, unsorted).expect_err("unsorted tape").code,
            "unsorted_tape"
        );
        let mut dupe = start_args(8);
        dupe.inputs = Some(vec![
            PlayInputDto {
                tick: 0,
                fighter: 0,
                mx: 0,
                mz: 0,
                facing: 0,
                buttons: 0,
            },
            PlayInputDto {
                tick: 0,
                fighter: 0,
                mx: 10,
                mz: 0,
                facing: 0,
                buttons: 0,
            },
        ]);
        assert_eq!(
            invoke(&server, dupe).expect_err("duplicate input").code,
            "duplicate_input"
        );

        // Oversize tapes refuse before simulating.
        let mut flood = start_args(8);
        flood.inputs = Some(vec![
            PlayInputDto {
                tick: 0,
                fighter: 0,
                mx: 0,
                mz: 0,
                facing: 0,
                buttons: 0,
            };
            MAX_PLAY_INPUTS + 1
        ]);
        assert_eq!(
            invoke(&server, flood).expect_err("flood tape").code,
            "too_many_inputs"
        );
    }

    #[test]
    fn play_table_bound_and_same_args_replay_identical() {
        let fixture = server();
        let mut ids = Vec::new();
        for _ in 0..MAX_JOBS {
            let started = invoke(&fixture, start_args(4)).expect("table fills");
            ids.push(started.job_id);
        }
        assert_eq!(ids.len(), MAX_JOBS, "table holds exactly MAX_JOBS handles");
        let full = invoke(&fixture, start_args(4)).expect_err("table bound fires");
        assert_eq!(full.code, "job_table_full");

        // Same start args twice give identical checkpoints (fresh sessions).
        let run_once = |ticks| {
            let fresh = server();
            let started = invoke(&fresh, start_args(ticks)).expect("start validates");
            let id = started.job_id;
            invoke(&fresh, run_action("poll", Some(id))).expect("progress poll");
            invoke(&fresh, run_action("poll", Some(id))).expect("completing poll")
        };
        assert_eq!(run_once(12), run_once(12));
    }

    #[test]
    fn unwired_capture_returns_unsupported_never_success() {
        let server = server();
        let error = CaptureTool::invoke(
            &server,
            CaptureArgs {
                width: Some(96),
                height: Some(64),
                max_size: None,
            },
        )
        .expect_err("unwired capture must fail");
        assert_eq!(error.code, "unsupported_capture");
        assert!(error.message.contains("callback"));

        // Bad sizes still validate first, with the legal values attached.
        let zero = CaptureTool::invoke(
            &server,
            CaptureArgs {
                width: Some(0),
                height: Some(64),
                max_size: None,
            },
        )
        .expect_err("zero width must fail");
        assert_eq!(zero.code, "bad_size");
        let huge = CaptureTool::invoke(
            &server,
            CaptureArgs {
                width: Some(CAPTURE_MAX_DIM + 1),
                height: Some(64),
                max_size: None,
            },
        )
        .expect_err("oversize receipt must fail");
        assert_eq!(huge.code, "bad_size");
        assert!(huge.message.contains(&CAPTURE_MAX_DIM.to_string()));
    }

    #[test]
    fn wired_capture_callback_returns_manifest_and_png() {
        let args = CaptureArgs {
            width: Some(96),
            height: Some(64),
            max_size: None,
        };
        // Caller-supplied one-shot renderer: the endpoint never imports
        // render, it only validates this render-free receipt.
        let out = capture_with(
            &args,
            Box::new(|asked| {
                let mut png = PNG_MAGIC.to_vec();
                png.extend_from_slice(&[0, 1, 2, 3]);
                Ok(CaptureOutput {
                    manifest: CaptureManifestDto {
                        adapter: "test-adapter".to_owned(),
                        driver: "test-driver".to_owned(),
                        width: asked.width.unwrap_or(0),
                        height: asked.height.unwrap_or(0),
                        validation_errors: 0,
                    },
                    png_bytes: png,
                })
            }),
        )
        .expect("wired capture succeeds");
        assert_eq!(&out.png_bytes[..PNG_MAGIC.len()], &PNG_MAGIC);
        assert_eq!((out.manifest.width, out.manifest.height), (96, 64));
        assert_eq!(out.manifest.validation_errors, 0);

        // Non-PNG bytes from the callback fail loudly, never as success.
        let bad = capture_with(
            &args,
            Box::new(|_| {
                Ok(CaptureOutput {
                    manifest: CaptureManifestDto {
                        adapter: "test-adapter".to_owned(),
                        driver: "test-driver".to_owned(),
                        width: 96,
                        height: 64,
                        validation_errors: 0,
                    },
                    png_bytes: vec![0, 1, 2, 3],
                })
            }),
        )
        .expect_err("non-PNG receipt must fail");
        assert_eq!(bad.code, "bad_receipt");

        // Dimension mismatch between receipt and request fails loudly too.
        let skewed = capture_with(
            &args,
            Box::new(|_| {
                Ok(CaptureOutput {
                    manifest: CaptureManifestDto {
                        adapter: "test-adapter".to_owned(),
                        driver: "test-driver".to_owned(),
                        width: 32,
                        height: 32,
                        validation_errors: 0,
                    },
                    png_bytes: PNG_MAGIC.to_vec(),
                })
            }),
        )
        .expect_err("skewed receipt must fail");
        assert_eq!(skewed.code, "bad_receipt");
    }

    /// Test-only base64 decoder (RFC 4648, padded): the image block is checked
    /// by decoding, never by trusting the encoder.
    fn b64_decode(text: &str) -> Vec<u8> {
        let digit = |c: u8| -> u32 {
            match c {
                b'A'..=b'Z' => u32::from(c - b'A'),
                b'a'..=b'z' => u32::from(c - b'a') + 26,
                b'0'..=b'9' => u32::from(c - b'0') + 52,
                b'+' => 62,
                _ => 63,
            }
        };
        let symbols: Vec<u32> = text.bytes().filter(|b| *b != b'=').map(digit).collect();
        let mut out = Vec::with_capacity(symbols.len() / 4 * 3);
        for chunk in symbols.chunks(4) {
            let mut packed = 0u32;
            for (index, symbol) in chunk.iter().enumerate() {
                packed |= symbol << (18 - 6 * index);
            }
            for shift in (0..chunk.len().saturating_sub(1)).map(|i| 16 - 8 * i) {
                out.push((packed >> shift) as u8);
            }
        }
        out
    }

    #[test]
    fn capture_image_block() {
        let mut png = PNG_MAGIC.to_vec();
        png.extend(std::iter::repeat_n(0x5A_u8, 300));
        let raw = rmcp::serde_json::to_value(&CaptureOutput {
            manifest: CaptureManifestDto {
                adapter: "test-adapter".to_owned(),
                driver: "test-driver".to_owned(),
                width: 96,
                height: 64,
                validation_errors: 0,
            },
            png_bytes: png.clone(),
        })
        .expect("receipt serializes");
        // The in-process receipt still owns the bytes; only the MCP carrier moves.
        assert!(
            raw.get("png_bytes").is_some(),
            "in-process path keeps png_bytes"
        );

        let result = frame_call_result(raw);
        let (image, text) = match &*result.content {
            [ContentBlock::Image(image), ContentBlock::Text(text)] => (image, text),
            other => panic!("expected one image plus one caption, got {other:?}"),
        };
        assert_eq!(image.mime_type, "image/png");
        assert!(text.text.chars().count() <= FRAME_CAPTION_MAX_CHARS);
        let decoded = b64_decode(&image.data);
        assert_eq!(
            &decoded[..PNG_MAGIC.len()],
            &PNG_MAGIC,
            "PNG magic survives"
        );
        let structured = result.structured_content.expect("structured content kept");
        assert!(structured.get("png_bytes").is_none(), "no png_bytes key");
        assert_eq!(
            structured
                .get("manifest")
                .and_then(|m| m.get("sha256"))
                .and_then(Value::as_str),
            Some(hex_bytes(&engine_game::camera_feed::showcase_sha256(&decoded)).as_str()),
            "the image bytes hash to the manifest sha256"
        );
    }

    #[test]
    fn look_hook_runs_once_per_validated_capture() {
        let args = CaptureArgs {
            width: Some(96),
            height: Some(64),
            max_size: None,
        };
        let receipt = |asked: &CaptureArgs| {
            let mut png = PNG_MAGIC.to_vec();
            png.extend_from_slice(&[0, 1, 2, 3]);
            Ok(CaptureOutput {
                manifest: CaptureManifestDto {
                    adapter: "test-adapter".to_owned(),
                    driver: "test-driver".to_owned(),
                    width: asked.width.unwrap_or(0),
                    height: asked.height.unwrap_or(0),
                    validation_errors: 0,
                },
                png_bytes: png,
            })
        };
        let calls = std::cell::Cell::new(0u32);
        {
            let mut hook = |png: &[u8]| {
                calls.set(calls.get() + 1);
                assert_eq!(&png[..PNG_MAGIC.len()], &PNG_MAGIC);
            };
            for expected in 1..=3u32 {
                let out = capture_with_look_hook(&args, Box::new(receipt), Some(&mut hook))
                    .expect("capture hands to hook");
                assert_eq!(&out.png_bytes[..PNG_MAGIC.len()], &PNG_MAGIC);
                assert_eq!(calls.get(), expected, "one hook call per validated frame");
            }
        }
        let refused = capture_with_look_hook(
            &args,
            Box::new(|_| {
                Ok(CaptureOutput {
                    manifest: CaptureManifestDto {
                        adapter: "test-adapter".to_owned(),
                        driver: "test-driver".to_owned(),
                        width: 96,
                        height: 64,
                        validation_errors: 0,
                    },
                    png_bytes: vec![0, 1, 2, 3],
                })
            }),
            Some(&mut |_: &[u8]| panic!("bad receipt must not notify")),
        )
        .expect_err("non-PNG refuses");
        assert_eq!(refused.code, "bad_receipt");
        assert_eq!(calls.get(), 3, "refusal adds no hook call");
        let plain = capture_with(&args, Box::new(receipt)).expect("unwired still succeeds");
        assert_eq!(&plain.png_bytes[..PNG_MAGIC.len()], &PNG_MAGIC);
    }

    fn step_args(
        seed: u64,
        half_extent_m: f32,
        upto_tick: Option<u32>,
    ) -> play_session::PlayStepArgs {
        play_session::PlayStepArgs {
            seed: Some(seed),
            participants: Some(2),
            half_extent_m: Some(half_extent_m),
            upto_tick,
            inputs: Some(Vec::new()),
            width: None,
            height: None,
            timing: None,
            scene_rev: None,
        }
    }

    fn completed_run() -> PlayRunOutput {
        let server = server();
        let started = invoke(&server, start_args(32)).expect("start validates");
        let id = started.job_id;
        assert_eq!(started.seed, 2040, "start echoes the seed");
        assert_eq!(
            started.half_extent_m.to_bits(),
            20.0_f32.to_bits(),
            "start echoes the extent bit-for-bit"
        );
        invoke(&server, run_action("poll", Some(id))).expect("progress poll");
        invoke(&server, run_action("poll", Some(id))).expect("completing poll")
    }

    #[test]
    fn play_manifest_pins_seed_and_extent_across_statuses() {
        let done = completed_run();
        assert_eq!(done.status, "completed");
        assert_eq!(done.seed, 2040, "the receipt pins the seed it simulated");
        assert_eq!(
            done.half_extent_m.to_bits(),
            20.0_f32.to_bits(),
            "the receipt pins the extent bit-for-bit"
        );
        assert_eq!(done.tick_count, 32, "ticks ride on tick_count");

        // Cancelled and expired handles keep the same pin: the manifest
        // describes the composition, never the terminal status.
        let server = server();
        let started = invoke(&server, start_args(16)).expect("start validates");
        let id = started.job_id;
        let stopped = invoke(&server, run_action("cancel", Some(id))).expect("cancel stops");
        assert_eq!(stopped.tick_count, 16);
        assert_eq!(stopped.seed, 2040);
        assert_eq!(
            stopped.half_extent_m.to_bits(),
            20.0_f32.to_bits(),
            "cancel keeps the extent pin"
        );
    }

    /// A finished job from the job table alone, which files no scene (the
    /// server door always pins one).
    fn unscened_run() -> PlayRunOutput {
        let mut jobs = PlayJobs::new();
        let mut args = start_args(32);
        args.scene_rev = None;
        let id = jobs.play_run(&args).expect("start validates").job_id;
        let poll = run_action("poll", Some(id));
        jobs.play_run(&poll).expect("progress poll");
        jobs.play_run(&poll).expect("completing poll")
    }

    #[test]
    fn play_manifest_round_trips_extent_and_seed_as_json() {
        let done = completed_run();
        let wire = rmcp::serde_json::to_string(&done).expect("output serializes");
        let back: PlayRunOutput = rmcp::serde_json::from_str(&wire).expect("output parses back");
        assert_eq!(back, done, "the full output round-trips");
        assert_eq!(back.seed, 2040, "the seed survives the wire");
        assert_eq!(
            back.half_extent_m.to_bits(),
            20.0_f32.to_bits(),
            "the extent survives the wire bit-for-bit"
        );
    }

    #[test]
    fn cross_tool_check_passes_honestly_on_matching_composition() {
        let done = unscened_run();
        check_run_against_step(&done, &step_args(2040, 20.0, Some(32)))
            .expect("identical seed, extent and in-range step pass");
        check_run_against_step(&done, &step_args(2040, 20.0, Some(1)))
            .expect("an early step inside the run passes");
    }

    #[test]
    fn cross_tool_check_fails_honestly_on_any_drift() {
        let done = unscened_run();

        // A door run always pins a scene; play.step plays none, so it refuses.
        let scened = check_run_against_step(&completed_run(), &step_args(2040, 20.0, Some(8)))
            .expect_err("a scene run cannot replay in play.step");
        assert_eq!(scened.code, "manifest_mismatch");
        assert!(scened.message.contains("scene revision 1"));

        let seed_drift = check_run_against_step(&done, &step_args(2041, 20.0, Some(32)))
            .expect_err("a seed drift must fail");
        assert_eq!(seed_drift.code, "manifest_mismatch");
        assert!(seed_drift.message.contains("seed"));
        assert!(seed_drift.message.contains("2041"));
        assert!(seed_drift.message.contains("2040"));

        let extent_drift = check_run_against_step(&done, &step_args(2040, 10.0, Some(32)))
            .expect_err("an extent drift must fail");
        assert_eq!(extent_drift.code, "manifest_mismatch");
        assert!(extent_drift.message.contains("half_extent_m"));
        assert!(extent_drift.message.contains("10"));

        let past_end = check_run_against_step(&done, &step_args(2040, 20.0, Some(33)))
            .expect_err("a step past the run end must fail");
        assert_eq!(past_end.code, "manifest_mismatch");
        assert!(past_end.message.contains("33"));

        let missing = check_run_against_step(&done, &step_args(2040, 20.0, None))
            .expect_err("a step without upto_tick must fail");
        assert_eq!(missing.code, "bad_ticks");

        let nan = check_run_against_step(&done, &step_args(2040, f32::NAN, Some(8)))
            .expect_err("a NaN extent must fail");
        assert_eq!(nan.code, "bad_extent");
    }

    #[test]
    fn same_start_args_replay_identical_manifests() {
        let once = completed_run();
        let twice = completed_run();
        assert_eq!(
            once.seed, twice.seed,
            "the same composition pins the same seed"
        );
        assert_eq!(
            once.half_extent_m.to_bits(),
            twice.half_extent_m.to_bits(),
            "the same composition pins the same extent bits"
        );
        assert_eq!(once.final_hash, twice.final_hash);
    }

    #[test]
    fn pt2_abstract_dots_legend_geometry_and_refusal_hint() {
        assert!(abstract_dots_tag().contains("ABSTRACT-DOTS"));
        assert!(abstract_dots_tag().contains("legend"));
        assert!(abstract_dots_tag().contains("CT7"));
        assert!(abstract_dots_tag().contains("kp01"));
        assert!(abstract_dots_tag().contains("jab"));
        assert!(abstract_dots_tag().contains("flash"));
        assert!(abstract_dots_tag().contains("silhouette"));
        assert_eq!(legend_span_px(96), 84);
        assert_eq!(legend_bar_h_px(64), 3);
        assert_eq!(jab_flash_ticks(), 8);
        let dots = abstract_dot_positions(96, 64);
        assert_eq!(dots, abstract_dot_positions(96, 64));
        for (x, y) in dots {
            assert!((6..96).contains(&x));
            assert!((6..64).contains(&y));
        }
        let skewed = capture_with(
            &CaptureArgs {
                width: Some(96),
                height: Some(64),
                max_size: None,
            },
            Box::new(|_| {
                Ok(CaptureOutput {
                    manifest: CaptureManifestDto {
                        adapter: "test-adapter".to_owned(),
                        driver: "test-driver".to_owned(),
                        width: 32,
                        height: 32,
                        validation_errors: 0,
                    },
                    png_bytes: PNG_MAGIC.to_vec(),
                })
            }),
        )
        .expect_err("skewed receipt must fail");
        assert_eq!(skewed.code, "bad_receipt");
        assert!(skewed.message.contains("ABSTRACT-DOTS"));
        assert!(skewed.message.contains("legend"));
    }

    #[test]
    fn pt2_seed95_duel_replays_identical_with_legend_and_flash() {
        let inputs = vec![
            PlayInputDto {
                tick: 0,
                fighter: 0,
                mx: 60,
                mz: 0,
                facing: 0,
                buttons: 1,
            },
            PlayInputDto {
                tick: 30,
                fighter: 0,
                mx: 60,
                mz: 10,
                facing: 100,
                buttons: 0,
            },
            PlayInputDto {
                tick: 60,
                fighter: 1,
                mx: -60,
                mz: 0,
                facing: 0,
                buttons: 1,
            },
            PlayInputDto {
                tick: 88,
                fighter: 0,
                mx: 0,
                mz: -60,
                facing: -500,
                buttons: 4,
            },
            PlayInputDto {
                tick: 100,
                fighter: 1,
                mx: 20,
                mz: 20,
                facing: 9000,
                buttons: 2,
            },
        ];
        let args = play_session::PlayStepArgs {
            seed: Some(95),
            participants: Some(2),
            half_extent_m: Some(20.0),
            upto_tick: Some(120),
            inputs: Some(inputs),
            width: None,
            height: None,
            timing: None,
            scene_rev: None,
        };
        let first = play_session::play_step_with(&args, None).expect("seed95 serves");
        let second = play_session::play_step_with(&args, None).expect("seed95 replays");
        assert_eq!(first.digest, second.digest);
        assert_eq!(first.png_bytes, second.png_bytes);
        assert_ne!(first.digest, 0);
        assert_eq!(&first.png_bytes[..PNG_MAGIC.len()], &PNG_MAGIC);
        // DOOR3: play.step's default frame is 320x180.
        assert_eq!((first.manifest.width, first.manifest.height), (320, 180));
        assert_eq!(first.fighters.len(), 2);
        assert_eq!(legend_span_px(first.manifest.width), 308);
        assert_eq!(legend_bar_h_px(first.manifest.height), 3);
        assert_eq!(jab_flash_ticks(), 8);
        let plain = play_session::PlayStepArgs {
            seed: Some(95),
            participants: Some(2),
            half_extent_m: Some(20.0),
            upto_tick: Some(120),
            inputs: None,
            width: None,
            height: None,
            timing: None,
            scene_rev: None,
        };
        let no_jab = play_session::play_step_with(&plain, None).expect("no-jab serves");
        assert_ne!(
            first.png_bytes, no_jab.png_bytes,
            "kp01 jab must move the capture (flash/legend/dots)"
        );
    }

    // -----------------------------------------------------------------------
    // PF24: opt-in wall time. The pre-change output shapes below are the
    // byte-equality oracle: with `timing` off the new output must serialize to
    // exactly these bytes. Wall time never enters a digest or a hash.
    // -----------------------------------------------------------------------

    /// `play.run` output before PF24 (same field order and types), plus the
    /// AX28 `counters` path, which the job table owns.
    #[derive(Serialize)]
    #[serde(crate = "rmcp::serde")]
    struct LegacyRunOutput {
        job_id: u64,
        status: String,
        progress: f32,
        events: Vec<String>,
        logs: Vec<String>,
        truncated_logs: bool,
        tick_count: u32,
        final_hash: Option<u64>,
        checkpoints: Vec<CheckpointDto>,
        seed: u64,
        half_extent_m: f32,
        scene_rev: Option<u64>,
        scene_digest: Option<String>,
        counters: String,
    }

    impl LegacyRunOutput {
        fn of(out: &PlayRunOutput) -> Self {
            Self {
                job_id: out.job_id,
                status: out.status.clone(),
                progress: out.progress,
                events: out.events.clone(),
                logs: out.logs.clone(),
                truncated_logs: out.truncated_logs,
                tick_count: out.tick_count,
                final_hash: out.final_hash,
                checkpoints: out.checkpoints.clone(),
                seed: out.seed,
                half_extent_m: out.half_extent_m,
                scene_rev: out.scene_rev,
                scene_digest: out.scene_digest.clone(),
                counters: out.counters.clone(),
            }
        }
    }

    /// `play.step` output before PF24 (same field order, same `dec_u64` wire).
    #[derive(Serialize)]
    #[serde(crate = "rmcp::serde")]
    struct LegacyStepOutput {
        tick: u32,
        #[serde(with = "play_session::dec_u64")]
        digest: u64,
        participants: u8,
        inputs_applied: usize,
        summary: String,
        manifest: CaptureManifestDto,
        png_bytes: Vec<u8>,
        fighters: Vec<play_session::ActorNumbers>,
        events: Vec<play_session::BoutEvent>,
    }

    impl LegacyStepOutput {
        fn of(out: &play_session::PlayStepOutput) -> Self {
            Self {
                tick: out.tick,
                digest: out.digest,
                participants: out.participants,
                inputs_applied: out.inputs_applied,
                summary: out.summary.clone(),
                manifest: out.manifest.clone(),
                png_bytes: out.png_bytes.clone(),
                fighters: out.fighters.clone(),
                events: out.events.clone(),
            }
        }
    }

    /// One untimed `play.run` job from the job table alone (no scene pin).
    fn untimed_run(jobs: &mut PlayJobs, ticks: u32) -> Vec<PlayRunOutput> {
        let mut start = start_args(ticks);
        start.scene_rev = None;
        let poll = run_action(
            "poll",
            Some(jobs.play_run(&start).expect("start runs").job_id),
        );
        vec![
            jobs.play_run(&poll).expect("progress poll"),
            jobs.play_run(&poll).expect("completing poll"),
        ]
    }

    #[test]
    fn play_timing_off_serializes_the_old_bytes() {
        // `play.run`: no flag, explicit `false`, and the poll path all match
        // the pre-change shape byte-for-byte.
        let mut jobs = PlayJobs::new();
        let started = {
            let mut args = start_args(32);
            args.scene_rev = None;
            jobs.play_run(&args).expect("start runs")
        };
        assert_eq!(
            rmcp::serde_json::to_string(&started).expect("start serializes"),
            rmcp::serde_json::to_string(&LegacyRunOutput::of(&started)).expect("oracle serializes")
        );
        assert!(!rmcp::serde_json::to_string(&started)
            .expect("json")
            .contains("timing"));
        let id = started.job_id;
        let mut off = run_action("poll", Some(id));
        off.timing = Some(false);
        let done = jobs.play_run(&off).expect("poll runs");
        assert_eq!(
            rmcp::serde_json::to_string(&done).expect("poll serializes"),
            rmcp::serde_json::to_string(&LegacyRunOutput::of(&done)).expect("oracle serializes")
        );
        assert!(!rmcp::serde_json::to_string(&done)
            .expect("json")
            .contains("timing"));

        // `play.step`: the same comparison against the step oracle.
        let step = play_session::play_step_with(&step_args(95, 20.0, Some(120)), None)
            .expect("step serves");
        assert_eq!(
            rmcp::serde_json::to_string(&step).expect("step serializes"),
            rmcp::serde_json::to_string(&LegacyStepOutput::of(&step)).expect("oracle serializes")
        );
        assert!(!rmcp::serde_json::to_string(&step)
            .expect("json")
            .contains("timing"));
    }

    #[test]
    fn play_timing_on_carries_p50_p99_and_software_label() {
        let mut jobs = PlayJobs::new();
        let mut args = start_args(64);
        args.scene_rev = None;
        args.timing = Some(true);
        let started = jobs.play_run(&args).expect("timed start runs");
        let timing = started.timing.clone().expect("timed start carries timing");
        assert_eq!(timing.label, TIMING_LABEL_SOFTWARE);
        assert_eq!(timing.samples, 1, "one measured start");
        match (timing.p50_ms, timing.p99_ms) {
            (StepMillis::Measured(p50), StepMillis::Measured(p99)) => {
                assert!(p50 >= 0.0, "p50 {p50} is a wall figure");
                assert!(p99 >= p50, "p99 {p99} sits above p50 {p50}");
            }
            other => panic!("a measured ring answers numbers, got {other:?}"),
        }
        let json = rmcp::serde_json::to_value(&started).expect("start serializes");
        assert_eq!(json["timing"]["label"], "software");
        assert!(
            json["timing"]["p50_ms"].is_number(),
            "measured p50 is a number"
        );
        assert!(
            json["timing"]["p99_ms"].is_number(),
            "measured p99 is a number"
        );
        assert_eq!(json["timing"]["samples"], 1);

        // `play.step` over a live session: two timed calls give two samples.
        let server = server();
        let mut step = step_args(95, 20.0, Some(120));
        step.timing = Some(true);
        let first =
            play_session::PlayStepTool::invoke(&server, step.clone()).expect("timed step serves");
        let second =
            play_session::PlayStepTool::invoke(&server, step).expect("timed replay serves");
        let first_timing = first.timing.clone().expect("step carries timing");
        let second_timing = second.timing.clone().expect("replay carries timing");
        assert_eq!(first_timing.label, "software");
        assert_eq!(first_timing.samples, 1);
        assert_eq!(second_timing.samples, 2, "the ring keeps both step times");
        assert!(
            matches!(second_timing.p50_ms, StepMillis::Measured(_)),
            "two samples answer numbers"
        );
        // The advertised schema renders (the untagged millisecond form).
        let schema = rmcp::serde_json::to_value(rmcp::schemars::schema_for!(PlayTimingDto))
            .expect("timing schema serializes");
        assert!(schema["properties"]["label"].is_object());
    }

    #[test]
    fn play_timing_zero_steps_answers_unknown() {
        // A ring with no sample answers `unknown`, never zero.
        let empty = play_session::TimingRing::default().report();
        assert_eq!(empty.samples, 0);
        assert_eq!(empty.label, TIMING_LABEL_SOFTWARE);
        assert_eq!(empty.p50_ms, StepMillis::Missing(NoSample::Unknown));
        assert_eq!(empty.p99_ms, StepMillis::Missing(NoSample::Unknown));

        // A timed poll of a job whose start measured nothing: the same answer
        // on the wire, as the word `unknown`.
        let mut jobs = PlayJobs::new();
        let timed = untimed_run(&mut jobs, 32);
        let id = {
            let mut args = start_args(32);
            args.scene_rev = None;
            jobs.play_run(&args).expect("start runs").job_id
        };
        assert!(timed[1].timing.is_none(), "an untimed job stores nothing");
        let mut poll = run_action("poll", Some(id));
        poll.timing = Some(true);
        let polled = jobs.play_run(&poll).expect("timed poll runs");
        let timing = polled
            .timing
            .clone()
            .expect("timed poll answers a timing object");
        assert_eq!(timing.samples, 0);
        assert_eq!(timing.p50_ms, StepMillis::Missing(NoSample::Unknown));
        let json = rmcp::serde_json::to_value(&polled).expect("poll serializes");
        assert_eq!(json["timing"]["p50_ms"], "unknown");
        assert_eq!(json["timing"]["p99_ms"], "unknown");
        assert_eq!(json["timing"]["samples"], 0);
    }

    #[test]
    fn play_timing_ring_is_bounded_and_nearest_rank() {
        let mut ring = play_session::TimingRing::default();
        for ms in 1..=100 {
            ring.record_ms(f64::from(ms));
        }
        let report = ring.report();
        assert_eq!(report.samples, 100);
        assert_eq!(report.p50_ms, StepMillis::Measured(50.0));
        assert_eq!(report.p99_ms, StepMillis::Measured(99.0));

        // The ring drops the oldest sample once full: 300 recorded values keep
        // the last 256 (45..=300), so the ranks shift with them.
        let mut bounded = play_session::TimingRing::default();
        for ms in 1..=300 {
            bounded.record_ms(f64::from(ms));
        }
        let report = bounded.report();
        assert_eq!(report.samples, TIMING_RING_CAP);
        assert_eq!(report.p50_ms, StepMillis::Measured(172.0));
        assert_eq!(report.p99_ms, StepMillis::Measured(298.0));

        // The same ring always answers the same figure.
        assert_eq!(bounded.report(), report);
    }

    #[test]
    fn play_timing_receipt_digest_is_equal_on_and_off() {
        // `play.run`: the same composition with and without the flag hashes
        // and logs identically; only the timing object differs.
        let mut off = PlayJobs::new();
        let mut on = PlayJobs::new();
        let mut args = start_args(32);
        args.scene_rev = None;
        let plain = untimed_run(&mut off, 32);
        let mut args_on = args.clone();
        args_on.timing = Some(true);
        let started = on.play_run(&args_on).expect("timed start runs");
        let id = started.job_id;
        let mut poll = run_action("poll", Some(id));
        poll.timing = Some(true);
        let progress = on.play_run(&poll).expect("timed progress poll runs");
        let done = on.play_run(&poll).expect("timed completing poll runs");
        assert_eq!(plain[1].final_hash, done.final_hash, "final hash is equal");
        assert_eq!(
            plain[1].checkpoints, done.checkpoints,
            "checkpoints are equal"
        );
        assert_eq!(plain[1].logs, done.logs, "logs carry no wall time");
        assert_eq!(plain[0].logs, progress.logs);
        assert_eq!(
            done.final_hash,
            done.checkpoints.last().map(|point| point.hash)
        );
        assert!(done.timing.is_some(), "the flag adds the object");

        // `play.step`: digest, pixels, fighters and summary are equal.
        let plain_step = play_session::play_step_with(&step_args(95, 20.0, Some(120)), None)
            .expect("step serves");
        let timed_step = play_session::play_step_with(
            &{
                let mut args = step_args(95, 20.0, Some(120));
                args.timing = Some(true);
                args
            },
            None,
        )
        .expect("timed step serves");
        assert_eq!(plain_step.digest, timed_step.digest, "step digest is equal");
        assert_ne!(plain_step.digest, 0);
        assert_eq!(plain_step.png_bytes, timed_step.png_bytes);
        assert_eq!(plain_step.fighters, timed_step.fighters);
        assert_eq!(plain_step.summary, timed_step.summary);
        let mut timed_json = rmcp::serde_json::to_value(&timed_step).expect("json");
        assert!(
            timed_json.get("timing").is_some(),
            "the flag adds the object"
        );
        timed_json.as_object_mut().expect("object").remove("timing");
        assert_eq!(
            rmcp::serde_json::to_value(&plain_step).expect("json"),
            timed_json,
            "the timed bytes minus `timing` equal the plain bytes"
        );
    }
}
