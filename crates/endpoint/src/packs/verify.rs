//! `verify` pack: the playtest and proof verifiers at the door.
//!
//! Consumer: the MCP door (`super::mcp::McpServer`) lists the verbs after
//! `api.load` and dispatches them by exact name. Feature `verify` (off by
//! default) links `engine-playtest` (already a `mcp` edge) and `engine-proof`,
//! so a plain `mcp` build carries no proof crate.
//!
//! Verbs: `replay.verify` and `replay.bisect` resimulate an arena tape against
//! its recorded chain through the playtest replay bodies (RB1);
//! `bug.replay` replays an `engine-proof` bug bundle and names its first
//! divergent tick; `trace.summary` re-runs a proof tape with tracing and
//! returns the live-matching summary hash; `verify.proof` runs the verifier
//! proof itself (the A7 seeded bug plus the mirror outcome digests) so the
//! parity test compares one door answer instead of a stand-in.
//!
//! One owner per verb name: when this pack is built it takes the `replay`
//! contract slot, so `replay.verify` and `replay.bisect` are registered once,
//! here, and a default build keeps the `replay` pack as their owner. The names
//! never appear twice in one registry.
//!
//! Check rules (opt-in on `verify.proof`): a caller submits lines like
//! `hits.max <= 4`, `base:damage.final >= 0`, `events.delta < 0`,
//! `no_divergence` or `no_divergence_before 8`. `hits`, `events` and `damage`
//! are per-tick integer series (event counts, hit counts, damage in permille)
//! over two real runs: the A7 seeded control tape (base) and the same match
//! with the seeded probe input applied (candidate). `stat` is `start`, `final`,
//! `min`, `max` or `delta`, where `delta` is the candidate final minus the
//! base final. The answer carries both runs' metric rows, one pass/FAIL per
//! rule with the observed value in the reason, and the checksum transitions.
//! Honest note: every edit changes the checkpoint hashes from tick 0, so
//! `first_divergent_tick` alone is not a verdict; judge the candidate by its
//! metrics (the rendered report says so too).
//!
//! Units: fixed ticks, integer hashes (hex text on the wire), hex text for
//! proof tape and bundle bytes. Determinism: the same args give the same
//! answer twice; no wall clock, no float compare, no map order. Errors:
//! [`McpToolError`] with `bad_args`, `corrupt_tape`, `corrupt_bundle`,
//! `clean_replay`, `no_chain` or `seeded_bug`, each naming the recovery; the
//! replay verbs keep the `replay` pack's own codes.
//!
//! DTO structs carry no field docs on purpose: every doc line inflates the
//! advertised schemas, and the discovery budget is a gate.

use engine_arena::{ArenaConfig, BodyTape, Checkpoint};
use engine_combat::FightEvent;
use engine_core::hash::hex16;
use engine_core::{decode_seal_hex, Fnv64};
use engine_playtest::mirror::{run_mirror, MirrorReport};
use engine_playtest::mirror_kit::MirrorRow;
use engine_playtest::replay::{bisect_tape, verify_tape, ReplayError};
use engine_playtest::seeded_bug::{
    bisect_seeded, seeded_agent, seeded_bugfile, verify_seeded_clean, SEEDED_BUG_ID, SEEDED_TICK,
};
use engine_proof::bug::{replay as replay_bundle, BugBundle, BundleError};
use engine_proof::trace::replay_verbose;
use engine_proof::Tape;
use rmcp::schemars::JsonSchema;
use rmcp::serde::{Deserialize, Serialize};

use super::bad_args;
use super::replay::{
    no_chain, replay_inputs, ReplayArgs, ReplayBisectArgs, ReplayBisectReport, ReplayReport,
    TOOL_REPLAY_BISECT, TOOL_REPLAY_VERIFY,
};
use super::{PackDef, PackError, VerbSpec};
use crate::mcp::{McpServer, McpToolError};
use crate::tool_capabilities::CapabilityCard;

/// Pack name.
pub const PACK: &str = "verify";
/// Verb: replay an `engine-proof` bug bundle and name its first divergent tick.
pub const TOOL_BUG_REPLAY: &str = "bug.replay";
/// Verb: summarize a proof tape's traced replay.
pub const TOOL_TRACE_SUMMARY: &str = "trace.summary";
/// Verb: run the verifier proof (seeded bug plus mirror outcome digests).
pub const TOOL_VERIFY_PROOF: &str = "verify.proof";

/// The verify pack: the replay pair, the proof pair and the proof run, loaded
/// on demand.
pub fn pack() -> Result<PackDef, PackError> {
    let mut pack = PackDef::new(
        PACK,
        "the verifiers at the door: resimulate a tape and bisect its first divergence, \
         replay a proof bug bundle, summarize a traced proof tape, and run the verifier \
         proof (A7 seeded bug plus mirror outcome digests)",
        false,
    );
    pack.add(VerbSpec::of::<ReplayVerifyTool>(
        TOOL_REPLAY_VERIFY,
        PACK,
        CapabilityCard::Observe,
    )?);
    pack.add(VerbSpec::of::<ReplayBisectTool>(
        TOOL_REPLAY_BISECT,
        PACK,
        CapabilityCard::Observe,
    )?);
    pack.add(VerbSpec::of::<BugReplayTool>(
        TOOL_BUG_REPLAY,
        PACK,
        CapabilityCard::Observe,
    )?);
    pack.add(VerbSpec::of::<TraceSummaryTool>(
        TOOL_TRACE_SUMMARY,
        PACK,
        CapabilityCard::Observe,
    )?);
    pack.add(VerbSpec::of::<VerifyProofTool>(
        TOOL_VERIFY_PROOF,
        PACK,
        CapabilityCard::Observe,
    )?);
    Ok(pack)
}

// ---------------------------------------------------------------------------
// Shared helpers: hex transport, typed refusals, report shapes.
// ---------------------------------------------------------------------------

/// Lowercase hex of raw bytes; the transport for proof tape and bundle bytes.
/// Test-only: the verbs decode hex, they never produce it.
#[cfg(test)]
fn hex_bytes(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(char::from(DIGITS[usize::from(byte >> 4)]));
        out.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }
    out
}

/// Even-length hex argument to bytes, or a `bad_args` refusal naming the field.
fn decode_hex(field: &str, text: &str) -> Result<Vec<u8>, McpToolError> {
    decode_seal_hex(text).ok_or_else(|| {
        bad_args(format!(
            "{field} is not even-length hex ({} chars); submit the verbatim lowercase hex \
             of the bytes, then re-run",
            text.chars().count()
        ))
    })
}

/// Refusal for tape bytes that do not decode or do not replay.
fn corrupt_tape(detail: impl core::fmt::Display) -> McpToolError {
    McpToolError::new(
        "corrupt_tape",
        format!("the tape bytes do not replay ({detail}); re-record the tape, then re-run"),
    )
}

/// A clean replay report with its resimulated tick count.
fn clean_report(ticks: u32) -> ReplayReport {
    ReplayReport {
        clean: true,
        tick: None,
        expected: None,
        observed: None,
        ticks,
    }
}

/// A divergent replay report carrying both hashes.
fn divergent_report(tick: u32, expected: u64, observed: u64, ticks: u32) -> ReplayReport {
    ReplayReport {
        clean: false,
        tick: Some(tick),
        expected: Some(expected),
        observed: Some(observed),
        ticks,
    }
}

// ---------------------------------------------------------------------------
// replay.verify / replay.bisect: the RB1 bodies (playtest replay module).
// ---------------------------------------------------------------------------

/// Recorded chain as arena checkpoints: only the hash is compared, the index
/// is the tick.
fn chain_checkpoints(chain: &[u64]) -> Vec<Checkpoint> {
    chain
        .iter()
        .enumerate()
        .map(|(index, hash)| Checkpoint {
            tick: u32::try_from(index).unwrap_or(u32::MAX),
            hash: *hash,
        })
        .collect()
}

/// Resimulate one tape through the playtest replay bodies and report clean or
/// the first divergent tick. `bisect` picks the binary search (a clean run is
/// a clean report); verify keeps the linear scan.
fn replay_report(args: &ReplayArgs, bisect: bool) -> Result<ReplayReport, McpToolError> {
    let (config, tape, chain) = replay_inputs(args)?;
    let checkpoints = chain_checkpoints(&chain);
    let ticks = config.ticks;
    if bisect {
        return match bisect_tape(&config, &tape, &checkpoints) {
            Ok(hit) => Ok(divergent_report(
                hit.tick,
                hit.expected,
                hit.observed,
                ticks,
            )),
            Err(ReplayError::NoChain) => Ok(clean_report(ticks)),
            Err(ReplayError::CorruptTape) => Err(corrupt_tape("replay refused the tape")),
        };
    }
    match verify_tape(&config, &tape, &checkpoints) {
        Ok(None) => Ok(clean_report(ticks)),
        Ok(Some(hit)) => Ok(divergent_report(
            hit.tick,
            hit.expected,
            hit.observed,
            ticks,
        )),
        Err(ReplayError::NoChain) => Err(no_chain()),
        Err(ReplayError::CorruptTape) => Err(corrupt_tape("replay refused the tape")),
    }
}

/// `replay.verify` handler: the RB1 linear scan under the audited seam.
fn replay_verify(service: &McpServer, args: &ReplayArgs) -> Result<ReplayReport, McpToolError> {
    service.dispatch(TOOL_REPLAY_VERIFY, |_| replay_report(args, false))
}

/// `replay.bisect` handler: the RB1 binary search under the audited seam.
fn replay_bisect(
    service: &McpServer,
    args: &ReplayBisectArgs,
) -> Result<ReplayBisectReport, McpToolError> {
    service.dispatch(TOOL_REPLAY_BISECT, |_| replay_bisect_core(args))
}

/// The body behind [`replay_bisect`]: metric mode delegates to the replay
/// pack's receipt bisect, chain mode keeps the RB1 binary search. Split out so
/// the verify-feature route is testable without a server.
fn replay_bisect_core(args: &ReplayBisectArgs) -> Result<ReplayBisectReport, McpToolError> {
    if let Some(metric) = args.metric.as_deref() {
        return crate::packs::replay::bisect_metric_report(args, metric);
    }
    replay_report(&args.as_replay(), true).map(ReplayBisectReport::from)
}

crate::e2_tool!(
    /// `replay.verify` tool: resimulate a tape and report the first divergent tick. Read-only.
    ReplayVerifyTool,
    name = TOOL_REPLAY_VERIFY,
    description = "Resimulate an arena tape against its recorded per-tick chain through the playtest replay body and report clean or the first divergent tick with both hashes. Read-only.",
    readonly = true,
    args = ReplayArgs,
    output = ReplayReport,
    invoke = replay_verify
);

crate::e2_tool!(
    /// `replay.bisect` tool: binary-search a tape for its first divergent tick. Read-only.
    ReplayBisectTool,
    name = TOOL_REPLAY_BISECT,
    description = "Binary-search a resimulated arena tape for the first tick whose hash leaves the recorded chain, through the playtest replay body. Read-only.",
    readonly = true,
    args = ReplayBisectArgs,
    output = ReplayBisectReport,
    invoke = replay_bisect
);

// ---------------------------------------------------------------------------
// bug.replay: one engine-proof bundle back to its first divergent tick.
// ---------------------------------------------------------------------------

/// `bug.replay` request: one `engine-proof` bug bundle as hex bytes.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct BugReplayArgs {
    pub bundle: String,
}

/// `bug.replay` answer: the authoritative first divergent tick and both
/// checkpoints as 16-char hex. `expected` is the recorded side, `actual` the
/// re-simulated side.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct BugReplayOutput {
    pub first_divergent_tick: u32,
    pub expected_legacy: String,
    pub expected_state: String,
    pub actual_legacy: String,
    pub actual_state: String,
}

/// Replay one bundle and lift the authoritative divergence.
pub fn bug_replay_report(args: &BugReplayArgs) -> Result<BugReplayOutput, McpToolError> {
    let bytes = decode_hex("bundle", &args.bundle)?;
    let bundle = BugBundle::decode(&bytes).map_err(|error| {
        McpToolError::new(
            "corrupt_bundle",
            format!(
                "the bundle bytes do not decode ({error}); submit the bytes of a written \
                 bundle, then re-run"
            ),
        )
    })?;
    match replay_bundle(&bundle) {
        Ok(report) => Ok(BugReplayOutput {
            first_divergent_tick: report.first_divergent_tick,
            expected_legacy: hex16(report.expected.legacy),
            expected_state: hex16(report.expected.state),
            actual_legacy: hex16(report.actual.legacy),
            actual_state: hex16(report.actual.state),
        }),
        Err(BundleError::CleanReplay) => Err(McpToolError::new(
            "clean_replay",
            "the bundle tape replays clean, so there is no divergent tick to name; \
             replay the tape with trace.summary instead",
        )),
        Err(error) => Err(McpToolError::new(
            "corrupt_bundle",
            format!(
                "the bundle does not replay ({error}); re-record it from the failing build, \
                 then re-run"
            ),
        )),
    }
}

/// `bug.replay` handler: the proof body under the audited dispatch seam.
fn bug_replay(service: &McpServer, args: &BugReplayArgs) -> Result<BugReplayOutput, McpToolError> {
    service.dispatch(TOOL_BUG_REPLAY, |_| bug_replay_report(args))
}

crate::e2_tool!(
    /// `bug.replay` tool: replay a proof bug bundle to its first divergent tick. Read-only.
    BugReplayTool,
    name = TOOL_BUG_REPLAY,
    description = "Replay an engine-proof bug bundle (hex bytes) and report the authoritative first divergent tick with both checkpoint hashes. Refuses a clean tape. Read-only.",
    readonly = true,
    args = BugReplayArgs,
    output = BugReplayOutput,
    invoke = bug_replay
);

// ---------------------------------------------------------------------------
// trace.summary: proof tape through the tracing replay.
// ---------------------------------------------------------------------------

/// `trace.summary` request: one `engine-proof` input tape as hex bytes.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct TraceSummaryArgs {
    pub tape: String,
}

/// `trace.summary` answer: the traced replay's tick and event counts, its
/// live-matching summary hash (hex) and a bounded agent digest.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct TraceSummaryOutput {
    pub ticks: u32,
    pub events: u32,
    pub summary_hash: String,
    pub digest: String,
}

/// Digest budget for one trace summary answer, in bytes.
pub const TRACE_SUMMARY_DIGEST_BYTES: usize = 256;

/// Re-run one proof tape with tracing and lift the summary.
pub fn trace_summary_report(args: &TraceSummaryArgs) -> Result<TraceSummaryOutput, McpToolError> {
    let bytes = decode_hex("tape", &args.tape)?;
    let tape = Tape::decode(&bytes).map_err(corrupt_tape)?;
    let report = replay_verbose(&tape).map_err(corrupt_tape)?;
    Ok(TraceSummaryOutput {
        ticks: u32::try_from(report.checkpoints.len()).unwrap_or(u32::MAX),
        events: u32::try_from(report.events.len()).unwrap_or(u32::MAX),
        summary_hash: hex16(report.summary_hash),
        digest: report.digest(TRACE_SUMMARY_DIGEST_BYTES),
    })
}

/// `trace.summary` handler: the proof body under the audited dispatch seam.
fn trace_summary(
    service: &McpServer,
    args: &TraceSummaryArgs,
) -> Result<TraceSummaryOutput, McpToolError> {
    service.dispatch(TOOL_TRACE_SUMMARY, |_| trace_summary_report(args))
}

crate::e2_tool!(
    /// `trace.summary` tool: traced replay summary of one proof tape. Read-only.
    TraceSummaryTool,
    name = TOOL_TRACE_SUMMARY,
    description = "Re-run an engine-proof tape (hex bytes) with tracing and return tick and event counts, the live-matching summary hash and a bounded digest. Read-only.",
    readonly = true,
    args = TraceSummaryArgs,
    output = TraceSummaryOutput,
    invoke = trace_summary
);

// ---------------------------------------------------------------------------
// verify.proof: the verifier proof itself, through the door.
// ---------------------------------------------------------------------------

/// `verify.proof` request: how many mirror tasks to report (all when omitted)
/// and optional check rules over the seeded base/candidate pair.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct VerifyProofArgs {
    pub tasks: Option<u32>,
    #[serde(default)]
    pub checks: Option<Vec<String>>,
}

/// One mirror row's outcome digest: the task, the backend and the digest over
/// both submissions' verdict, checks, state hash and detail.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct MirrorTaskDigest {
    pub task: String,
    pub backend: String,
    pub digest: String,
}

/// One check's verdict: the rule as submitted, whether it passed, and the
/// reason naming the observed value so an agent can fix the run.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct VerifyCheckOutcome {
    pub rule: String,
    pub pass: bool,
    pub reason: String,
}

/// One metric's `start`, `final`, `min` and `max` over a run's per-tick series.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct VerifyMetricSummary {
    pub metric: String,
    pub start: i64,
    #[serde(rename = "final")]
    #[schemars(rename = "final")]
    pub last: i64,
    pub min: i64,
    pub max: i64,
}

/// Checksum transitions between the base and candidate runs.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct VerifyTransitions {
    /// First tick whose hash differs; `None` when the chains agree.
    pub first_tick: Option<u32>,
    /// Ticks whose hash differs (a shorter run counts each missing tick).
    pub differing_ticks: u32,
    /// Ticks the candidate ran.
    pub ticks: u32,
}

/// `verify.proof` answer: the A7 seeded bug verdict and one digest per mirror
/// task per backend, task-major in report order, plus the optional check
/// report (verdict text first, then both runs' metric rows and each verdict).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct VerifyProofOutput {
    pub seeded_bug: String,
    pub seeded_tick: u32,
    pub seeded_expected_tick: u32,
    pub seeded_clean: bool,
    pub tasks: u32,
    pub mirror: Vec<MirrorTaskDigest>,
    #[serde(default)]
    pub verdict: String,
    #[serde(default)]
    pub checks: Vec<VerifyCheckOutcome>,
    #[serde(default)]
    pub checks_passed: u32,
    #[serde(default)]
    pub checks_total: u32,
    #[serde(default)]
    pub base: Vec<VerifyMetricSummary>,
    #[serde(default)]
    pub candidate: Vec<VerifyMetricSummary>,
    #[serde(default)]
    pub transitions: VerifyTransitions,
}

/// Digest of one mirror row's two outcomes: verdict, named checks, state hash
/// and detail. Single owner: the verb and the parity test both call this, so
/// the two paths cannot measure different things.
pub fn mirror_row_digest(row: &MirrorRow) -> String {
    let mut hash = Fnv64::new();
    hash.write_str(row.task);
    hash.write_str(row.category);
    hash.write_str(row.shape);
    hash.write_str(row.backend);
    for outcome in [&row.reference, &row.weak] {
        hash.write_bool(outcome.pass);
        hash.write_u64(outcome.hash);
        hash.write_str(&outcome.detail);
        hash.write_u64(u64::try_from(outcome.checks.len()).unwrap_or(u64::MAX));
        for check in &outcome.checks {
            hash.write_str(check.name);
            hash.write_bool(check.ok);
        }
    }
    hex16(hash.finish())
}

/// Digests of the first `tasks` distinct mirror tasks, every backend, in
/// report order. The parity test calls this on the playtest report and the
/// verb calls it on the same report inside the spawned door.
pub fn mirror_task_digests(report: &MirrorReport, tasks: usize) -> Vec<MirrorTaskDigest> {
    let mut order: Vec<&'static str> = Vec::new();
    for slice in &report.slices {
        for row in &slice.rows {
            if !order.contains(&row.task) {
                order.push(row.task);
            }
        }
    }
    let chosen = &order[..order.len().min(tasks)];
    let mut out = Vec::new();
    for slice in &report.slices {
        for row in &slice.rows {
            if chosen.contains(&row.task) {
                out.push(MirrorTaskDigest {
                    task: row.task.to_owned(),
                    backend: row.backend.to_owned(),
                    digest: mirror_row_digest(row),
                });
            }
        }
    }
    out
}

/// Run the verifier proof: the A7 seeded bug through its causal bisect and
/// the mirror outcome digests. `tasks` is refused outside `1..=available`.
pub fn proof_report(args: &VerifyProofArgs) -> Result<VerifyProofOutput, McpToolError> {
    // Checks first: one bad rule refuses before the seeded drive and the mirror
    // run spend any work.
    let checks = match args.checks.as_deref().filter(|lines| !lines.is_empty()) {
        Some(lines) => Some(check_run(lines)?),
        None => None,
    };
    let bug = seeded_bugfile().map_err(|error| {
        McpToolError::new(
            "seeded_bug",
            format!(
                "the A7 seeded run filed no bug ({error}); re-run on a build with the seeded \
                 probe intact"
            ),
        )
    })?;
    let seeded_tick = bisect_seeded(&bug).map_err(|error| {
        McpToolError::new(
            "seeded_bug",
            format!(
                "the A7 filed tape did not bisect ({error}); re-run on a build with the \
                 seeded probe intact"
            ),
        )
    })?;
    let report = run_mirror();
    let mut available = Vec::new();
    for slice in &report.slices {
        for row in &slice.rows {
            if !available.contains(&row.task) {
                available.push(row.task);
            }
        }
    }
    let limit = available.len();
    let tasks = match args.tasks {
        Some(tasks) => {
            let asked = usize::try_from(tasks).unwrap_or(usize::MAX);
            if asked == 0 || asked > limit {
                return Err(bad_args(format!(
                    "tasks {tasks} is outside 1..={limit}; re-run in range or omit it for all"
                )));
            }
            asked
        }
        None => limit,
    };
    let mut output = VerifyProofOutput {
        seeded_bug: SEEDED_BUG_ID.to_owned(),
        seeded_tick,
        seeded_expected_tick: SEEDED_TICK,
        seeded_clean: verify_seeded_clean(&bug),
        tasks: u32::try_from(tasks).unwrap_or(u32::MAX),
        mirror: mirror_task_digests(&report, tasks),
        ..VerifyProofOutput::default()
    };
    if let Some(run) = checks {
        output.verdict = run.render();
        output.checks = run.outcomes();
        output.checks_passed = u32::try_from(run.passed()).unwrap_or(u32::MAX);
        output.checks_total = u32::try_from(run.rows.len()).unwrap_or(u32::MAX);
        output.base = metric_summaries(&run.base);
        output.candidate = metric_summaries(&run.candidate);
        output.transitions = VerifyTransitions {
            first_tick: run.transitions.first,
            differing_ticks: run.transitions.differing,
            ticks: u32::try_from(run.candidate.hashes.len()).unwrap_or(u32::MAX),
        };
    }
    Ok(output)
}

/// `verify.proof` handler: the verifier bodies under the audited seam.
fn verify_proof(
    service: &McpServer,
    args: &VerifyProofArgs,
) -> Result<VerifyProofOutput, McpToolError> {
    service.dispatch(TOOL_VERIFY_PROOF, |_| proof_report(args))
}

crate::e2_tool!(
    /// `verify.proof` tool: the A7 seeded bug and the mirror outcome digests. Read-only.
    VerifyProofTool,
    name = TOOL_VERIFY_PROOF,
    description = "Run the verifier proof through the door: bisect the A7 seeded bug tape to its seeded tick and digest every mirror task outcome, so a parity run compares this door with the playtest drive. With checks (`hits.max <= 4`, `base:damage.final >= 0`, `no_divergence_before 8`) it also reports both runs' metric rows and one pass/FAIL per rule. Read-only.",
    readonly = true,
    args = VerifyProofArgs,
    output = VerifyProofOutput,
    invoke = verify_proof
);

// ---------------------------------------------------------------------------
// Check rules: integer metrics over the seeded base and its candidate edit.
// ---------------------------------------------------------------------------

/// Metric a rule reads from one side's per-tick series.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CheckMetric {
    Events,
    Hits,
    Damage,
}

impl CheckMetric {
    /// Every metric, in report order.
    const ALL: [Self; 3] = [Self::Events, Self::Hits, Self::Damage];

    /// Wire token.
    fn token(self) -> &'static str {
        match self {
            Self::Events => "events",
            Self::Hits => "hits",
            Self::Damage => "damage",
        }
    }

    /// Parse a wire token; `None` when it is not a metric this pack reads.
    fn parse(token: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|metric| metric.token() == token)
    }
}

/// Statistic a rule reads from one series (`delta` spans both runs).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CheckStat {
    Start,
    Final,
    Min,
    Max,
    Delta,
}

impl CheckStat {
    /// Every statistic, in report order.
    const ALL: [Self; 5] = [Self::Start, Self::Final, Self::Min, Self::Max, Self::Delta];

    /// Wire token.
    fn token(self) -> &'static str {
        match self {
            Self::Start => "start",
            Self::Final => "final",
            Self::Min => "min",
            Self::Max => "max",
            Self::Delta => "delta",
        }
    }

    /// Parse a wire token; `None` when it is not a statistic this pack reads.
    fn parse(token: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|stat| stat.token() == token)
    }
}

/// Comparison a rule applies to the observed integer value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CheckCmp {
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
    Ne,
}

impl CheckCmp {
    /// Every comparison, in report order.
    const ALL: [Self; 6] = [Self::Lt, Self::Le, Self::Gt, Self::Ge, Self::Eq, Self::Ne];

    /// Wire token.
    fn token(self) -> &'static str {
        match self {
            Self::Lt => "<",
            Self::Le => "<=",
            Self::Gt => ">",
            Self::Ge => ">=",
            Self::Eq => "==",
            Self::Ne => "!=",
        }
    }

    /// Parse a wire token; `None` when it is not a comparison this pack reads.
    fn parse(token: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|cmp| cmp.token() == token)
    }

    /// True when `value cmp bound` holds.
    fn holds(self, value: i64, bound: i64) -> bool {
        match self {
            Self::Lt => value < bound,
            Self::Le => value <= bound,
            Self::Gt => value > bound,
            Self::Ge => value >= bound,
            Self::Eq => value == bound,
            Self::Ne => value != bound,
        }
    }
}

/// Which run a rule reads: the unedited base or the candidate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RuleSide {
    Base,
    Candidate,
}

impl RuleSide {
    /// Wire token.
    fn token(self) -> &'static str {
        match self {
            Self::Base => "base",
            Self::Candidate => "candidate",
        }
    }
}

/// One parsed rule.
#[derive(Debug, Clone, PartialEq, Eq)]
enum CheckRule {
    /// `<side><metric>.<stat> <cmp> <bound>`.
    Metric {
        side: RuleSide,
        metric: CheckMetric,
        stat: CheckStat,
        cmp: CheckCmp,
        bound: i64,
    },
    /// The candidate chain must equal the base chain on every tick.
    NoDivergence,
    /// The first divergent tick, when there is one, must not be below this.
    NoDivergenceBefore(u32),
}

/// One rule line plus its parsed form; the line is echoed in the report.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ParsedCheck {
    line: String,
    rule: CheckRule,
}

/// Refusal for a line that is not a rule at all. Names the line and the three
/// legal shapes, then one recovery.
fn bad_rule(line: &str, why: String) -> McpToolError {
    McpToolError::new(
        "bad_rule",
        format!(
            "check rule {line:?} is malformed ({why}); submit `metric.stat cmp number`, \
             `no_divergence` or `no_divergence_before tick`, then re-run"
        ),
    )
}

/// Refusal for a metric token this pack does not read.
fn bad_rule_metric(metric: &str) -> McpToolError {
    McpToolError::new(
        "bad_metric",
        format!("metric {metric:?} is not one of events, hits, damage; re-run with one of those"),
    )
}

/// Refusal for a stat token this pack does not read.
fn bad_rule_stat(stat: &str) -> McpToolError {
    McpToolError::new(
        "bad_stat",
        format!(
            "stat {stat:?} is not one of start, final, min, max, delta; re-run with one of those"
        ),
    )
}

/// Parse one rule line. Every refusal is `bad_rule`, `bad_metric` or
/// `bad_stat` and names the recovery; nothing panics on bad input.
fn parse_check(line: &str) -> Result<ParsedCheck, McpToolError> {
    let text = line.trim();
    let words: Vec<&str> = text.split_whitespace().collect();
    let rule = match words.as_slice() {
        ["no_divergence"] => CheckRule::NoDivergence,
        ["no_divergence_before", tick] => {
            let before = tick
                .parse::<u32>()
                .map_err(|_| bad_rule(text, format!("`{tick}` is not a whole tick number")))?;
            CheckRule::NoDivergenceBefore(before)
        }
        [target, cmp, bound] => parse_metric_rule(text, target, cmp, bound)?,
        _ => {
            return Err(bad_rule(
                text,
                "expected three words, `no_divergence` or `no_divergence_before tick`".to_owned(),
            ));
        }
    };
    Ok(ParsedCheck {
        line: text.to_owned(),
        rule,
    })
}

/// Parse `<metric>.<stat> <cmp> <number>` with an optional `base:` prefix.
fn parse_metric_rule(
    line: &str,
    target: &str,
    cmp_text: &str,
    bound_text: &str,
) -> Result<CheckRule, McpToolError> {
    let (side, name) = match target.strip_prefix("base:") {
        Some(name) => (RuleSide::Base, name),
        None => (RuleSide::Candidate, target),
    };
    let (metric_text, stat_text) = name.split_once('.').ok_or_else(|| {
        bad_rule(
            line,
            format!("`{name}` has no `.stat`; write `{name}.final` for example"),
        )
    })?;
    let metric = CheckMetric::parse(metric_text).ok_or_else(|| bad_rule_metric(metric_text))?;
    let stat = CheckStat::parse(stat_text).ok_or_else(|| bad_rule_stat(stat_text))?;
    if side == RuleSide::Base && stat == CheckStat::Delta {
        return Err(bad_rule(
            line,
            "delta already compares the candidate final with the base final, so it takes no \
             base: prefix"
                .to_owned(),
        ));
    }
    let cmp = CheckCmp::parse(cmp_text).ok_or_else(|| {
        bad_rule(
            line,
            format!("`{cmp_text}` is not one of <, <=, >, >=, ==, !="),
        )
    })?;
    let bound = bound_text
        .parse::<i64>()
        .map_err(|_| bad_rule(line, format!("`{bound_text}` is not an integer bound")))?;
    Ok(CheckRule::Metric {
        side,
        metric,
        stat,
        cmp,
        bound,
    })
}

/// One side's per-tick integer metrics plus its checkpoint hashes.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct RunMetrics {
    events: Vec<i64>,
    hits: Vec<i64>,
    damage: Vec<i64>,
    hashes: Vec<u64>,
}

impl RunMetrics {
    /// One metric's per-tick samples.
    fn samples(&self, metric: CheckMetric) -> &[i64] {
        match metric {
            CheckMetric::Events => &self.events,
            CheckMetric::Hits => &self.hits,
            CheckMetric::Damage => &self.damage,
        }
    }

    /// `start`, `final`, `min` or `max` over one metric. An empty run reads
    /// `0`; [`run_metrics`] never builds one, so this is a total function.
    fn stat(&self, metric: CheckMetric, stat: CheckStat) -> i64 {
        match stat {
            CheckStat::Start => self.samples(metric).first().copied().unwrap_or(0),
            CheckStat::Final => self.samples(metric).last().copied().unwrap_or(0),
            CheckStat::Min => self.samples(metric).iter().copied().min().unwrap_or(0),
            CheckStat::Max => self.samples(metric).iter().copied().max().unwrap_or(0),
            CheckStat::Delta => 0,
        }
    }
}

/// Resimulate one tape and lift its per-tick metrics and hash chain. Refuses
/// typed (`corrupt_tape`) when the arena runs nothing or the event run
/// disagrees on length, so a metric row never comes from a partial run.
fn run_metrics(config: &ArenaConfig, tape: &BodyTape) -> Result<RunMetrics, McpToolError> {
    let checkpoints = engine_arena::run(config, tape);
    let event_rows = engine_arena::run_events(config, tape);
    if checkpoints.is_empty() || checkpoints.len() != event_rows.len() {
        return Err(corrupt_tape(format!(
            "arena ran {} checkpoints and {} event rows",
            checkpoints.len(),
            event_rows.len()
        )));
    }
    let mut events = Vec::with_capacity(checkpoints.len());
    let mut hits = Vec::with_capacity(checkpoints.len());
    let mut damage = Vec::with_capacity(checkpoints.len());
    for row in &event_rows {
        let mut hit_count = 0i64;
        let mut dealt = 0i64;
        for event in &row.events {
            match event {
                FightEvent::Hit { damage: hit, .. } => {
                    hit_count = hit_count.saturating_add(1);
                    dealt = dealt.saturating_add(i64::from(*hit));
                }
                FightEvent::Blocked { chip, .. } => {
                    dealt = dealt.saturating_add(i64::from(*chip));
                }
                _ => {}
            }
        }
        events.push(i64::try_from(row.events.len()).unwrap_or(i64::MAX));
        hits.push(hit_count);
        damage.push(dealt);
    }
    Ok(RunMetrics {
        events,
        hits,
        damage,
        hashes: checkpoints.iter().map(|point| point.hash).collect(),
    })
}

/// First tick where the candidate chain leaves the base chain, and how many
/// ticks differ. A run that ended early diverges at the shared end.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct ChainTransitions {
    first: Option<u32>,
    differing: u32,
}

/// Compare two per-tick hash chains; integer compares only.
fn chain_transitions(base: &[u64], candidate: &[u64]) -> ChainTransitions {
    let mut transitions = ChainTransitions::default();
    for (index, (left, right)) in base.iter().zip(candidate.iter()).enumerate() {
        if left != right {
            transitions.differing = transitions.differing.saturating_add(1);
            if transitions.first.is_none() {
                transitions.first = u32::try_from(index).ok();
            }
        }
    }
    let shared = base.len().min(candidate.len());
    let extra = base.len().abs_diff(candidate.len());
    if extra > 0 {
        transitions.differing = transitions
            .differing
            .saturating_add(u32::try_from(extra).unwrap_or(u32::MAX));
        if transitions.first.is_none() {
            transitions.first = u32::try_from(shared).ok();
        }
    }
    transitions
}

/// One check's verdict: pass flag plus the reason an agent reads.
#[derive(Debug, Clone, PartialEq, Eq)]
struct CheckVerdict {
    pass: bool,
    reason: String,
}

/// Evaluate one rule against both runs. Every reason names the observed value
/// and the side it came from, so a FAIL is fixable without a second call.
fn evaluate_check(
    check: &ParsedCheck,
    base: &RunMetrics,
    candidate: &RunMetrics,
    transitions: ChainTransitions,
) -> CheckVerdict {
    match &check.rule {
        CheckRule::Metric {
            side,
            metric,
            stat,
            cmp,
            bound,
        } => {
            let value = match (side, stat) {
                (_, CheckStat::Delta) => candidate
                    .stat(*metric, CheckStat::Final)
                    .saturating_sub(base.stat(*metric, CheckStat::Final)),
                (RuleSide::Base, stat) => base.stat(*metric, *stat),
                (RuleSide::Candidate, stat) => candidate.stat(*metric, *stat),
            };
            let label = format!("{} {}.{}", side.token(), metric.token(), stat.token());
            let pass = cmp.holds(value, *bound);
            let reason = if pass {
                format!("{label} is {value}, satisfies {} {bound}", cmp.token())
            } else {
                format!("{label} is {value}, not {} {bound}", cmp.token())
            };
            CheckVerdict { pass, reason }
        }
        CheckRule::NoDivergence => match transitions.first {
            None => CheckVerdict {
                pass: true,
                reason: format!(
                    "candidate and base hashes agree on all {} ticks",
                    candidate.hashes.len()
                ),
            },
            Some(tick) => CheckVerdict {
                pass: false,
                reason: format!("candidate leaves the base chain at tick {tick}"),
            },
        },
        CheckRule::NoDivergenceBefore(before) => match transitions.first {
            None => CheckVerdict {
                pass: true,
                reason: format!(
                    "candidate and base agree on all {} ticks, so none is before {before}",
                    candidate.hashes.len()
                ),
            },
            Some(tick) if tick >= *before => CheckVerdict {
                pass: true,
                reason: format!("first divergence is tick {tick}, not before {before}"),
            },
            Some(tick) => CheckVerdict {
                pass: false,
                reason: format!("first divergence is tick {tick}, before {before}"),
            },
        },
    }
}

/// Everything one check run answers: both runs, their transitions and the
/// per-rule verdicts. The compact text is [`CheckRun::render`]; the JSON side
/// is [`metric_summaries`] plus [`CheckRun::outcomes`].
#[derive(Debug, Clone, PartialEq, Eq)]
struct CheckRun {
    base: RunMetrics,
    candidate: RunMetrics,
    transitions: ChainTransitions,
    rows: Vec<(ParsedCheck, CheckVerdict)>,
}

impl CheckRun {
    /// Checks that passed.
    fn passed(&self) -> usize {
        self.rows.iter().filter(|(_, verdict)| verdict.pass).count()
    }

    /// Per-rule outcomes in submitted order.
    fn outcomes(&self) -> Vec<VerifyCheckOutcome> {
        self.rows
            .iter()
            .map(|(check, verdict)| VerifyCheckOutcome {
                rule: check.line.clone(),
                pass: verdict.pass,
                reason: verdict.reason.clone(),
            })
            .collect()
    }

    /// Compact report: the bucket line, one pass/FAIL line per rule with its
    /// reason, the checksum transitions, then every metric row for both runs
    /// and the cross-run deltas. Text first because an agent reads it before
    /// it parses JSON.
    fn render(&self) -> String {
        let total = self.rows.len();
        let passed = self.passed();
        let bucket = if passed == total { "PASSED" } else { "FAILED" };
        let mut out = format!(
            "verify.proof checks: base=A7 control, candidate=A7 seeded probe, {} ticks\n",
            self.candidate.hashes.len()
        );
        out.push_str(&format!("CHECKS {bucket}: {passed}/{total} passed.\n"));
        for (check, verdict) in &self.rows {
            let mark = if verdict.pass { "pass" } else { "FAIL" };
            out.push_str(&format!(
                "  [{mark}] {}  ({})\n",
                check.line, verdict.reason
            ));
        }
        let ticks = self.candidate.hashes.len();
        match self.transitions.first {
            None => out.push_str(&format!(
                "checksums: candidate and base agree on all {ticks} ticks.\n"
            )),
            Some(tick) => out.push_str(&format!(
                "checksums: candidate leaves the base chain at tick {tick}; {} of {ticks} \
                 ticks differ. Any edit differs from tick 0, so judge by metrics, not by the \
                 first divergent tick.\n",
                self.transitions.differing
            )),
        }
        for (side, run) in [("base", &self.base), ("candidate", &self.candidate)] {
            for metric in CheckMetric::ALL {
                out.push_str(&format!(
                    "{side} {} start {} -> final {} (min {}..max {})\n",
                    metric.token(),
                    run.stat(metric, CheckStat::Start),
                    run.stat(metric, CheckStat::Final),
                    run.stat(metric, CheckStat::Min),
                    run.stat(metric, CheckStat::Max),
                ));
            }
        }
        let deltas: Vec<String> = CheckMetric::ALL
            .into_iter()
            .map(|metric| {
                let delta = self
                    .candidate
                    .stat(metric, CheckStat::Final)
                    .saturating_sub(self.base.stat(metric, CheckStat::Final));
                format!("{} {delta:+}", metric.token())
            })
            .collect();
        out.push_str(&format!(
            "delta (candidate final - base final): {}\n",
            deltas.join(", ")
        ));
        out
    }
}

/// One run's metric rows in [`CheckMetric::ALL`] order.
fn metric_summaries(run: &RunMetrics) -> Vec<VerifyMetricSummary> {
    CheckMetric::ALL
        .into_iter()
        .map(|metric| VerifyMetricSummary {
            metric: metric.token().to_owned(),
            start: run.stat(metric, CheckStat::Start),
            last: run.stat(metric, CheckStat::Final),
            min: run.stat(metric, CheckStat::Min),
            max: run.stat(metric, CheckStat::Max),
        })
        .collect()
}

/// The A7 pair: the seeded control tape (base) and the same match with the
/// seeded probe input applied (candidate). The playtest owner keeps the
/// fixture (seed, ticks, fighters, policy); this only maps its agent config
/// onto the arena config the two runs share.
fn seeded_pair() -> (ArenaConfig, BodyTape, BodyTape) {
    let agent = seeded_agent();
    let config = ArenaConfig {
        seed: agent.config.base_seed,
        ticks: agent.config.ticks,
        fighters: agent.config.fighters,
        half_extent_m: agent.config.half_extent_m,
        table_version: 1,
    };
    let base = agent.driver.control_tape(&config);
    let candidate = agent.driver.probe_tape(&config, &base);
    (config, base, candidate)
}

/// Parse every rule first (one bad rule refuses before any resimulation),
/// then measure both runs and evaluate the rules in submitted order.
fn check_run(lines: &[String]) -> Result<CheckRun, McpToolError> {
    let mut parsed = Vec::with_capacity(lines.len());
    for line in lines {
        parsed.push(parse_check(line)?);
    }
    let (config, base_tape, candidate_tape) = seeded_pair();
    let base = run_metrics(&config, &base_tape)?;
    let candidate = run_metrics(&config, &candidate_tape)?;
    let transitions = chain_transitions(&base.hashes, &candidate.hashes);
    let rows = parsed
        .into_iter()
        .map(|check| {
            let verdict = evaluate_check(&check, &base, &candidate, transitions);
            (check, verdict)
        })
        .collect();
    Ok(CheckRun {
        base,
        candidate,
        transitions,
        rows,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::packs::PackRegistry;
    use engine_editor::capability::Capability;

    fn server() -> McpServer {
        McpServer::new(7, Capability::unrestricted(7))
    }

    /// A clean 24-tick proof tape: the tracing replay accepts it.
    fn proof_tape() -> Tape {
        engine_proof::record(engine_proof::Config { seed: 7, ticks: 24 }, &[])
            .expect("the empty-input control records")
    }

    #[test]
    fn verify_pack_holds_the_five_read_only_verbs() {
        let pack = pack().expect("verify pack builds");
        assert_eq!(pack.name(), PACK);
        assert_eq!(
            pack.verb_names(),
            vec![
                TOOL_REPLAY_VERIFY,
                TOOL_REPLAY_BISECT,
                TOOL_BUG_REPLAY,
                TOOL_TRACE_SUMMARY,
                TOOL_VERIFY_PROOF
            ]
        );
        for verb in pack.verbs() {
            assert_eq!(verb.pack(), PACK);
            assert_eq!(verb.card(), CapabilityCard::Observe);
            assert!(verb.readonly(), "{} writes nothing", verb.name());
        }
        // Under this feature the verify pack takes the replay slot: each name
        // is served exactly once.
        let registry = PackRegistry::shipped();
        assert_eq!(
            registry.verb(TOOL_REPLAY_VERIFY).map(VerbSpec::pack),
            Some(PACK)
        );
        assert!(registry.verb(TOOL_BUG_REPLAY).is_some());
        assert!(registry.verb(TOOL_TRACE_SUMMARY).is_some());
        assert!(registry.verb(TOOL_VERIFY_PROOF).is_some());
    }

    #[test]
    fn trace_summary_matches_the_plain_replay_hash_x2() {
        let tape = proof_tape();
        let bytes = tape.encode().expect("control tape encodes");
        let args = TraceSummaryArgs {
            tape: hex_bytes(&bytes),
        };
        let first = trace_summary_report(&args).expect("the control summarizes");
        assert_eq!(first.ticks, 24);
        assert_eq!(first.events, 24, "one sim.tick event per tick");
        assert_eq!(
            first.summary_hash,
            hex16(engine_proof::trace::replay_summary(&tape).expect("control replays")),
            "the traced hash equals the live summary"
        );
        assert!(!first.digest.is_empty());
        let second = trace_summary_report(&args).expect("repeats");
        assert_eq!(first, second, "the same tape summarizes identically twice");
        assert_eq!(
            super::trace_summary(&server(), &args).expect("verb serves"),
            first
        );
    }

    #[test]
    fn trace_summary_refuses_bad_hex_and_a_corrupt_tape() {
        let odd = TraceSummaryArgs {
            tape: "abc".to_owned(),
        };
        assert_eq!(
            trace_summary_report(&odd).expect_err("odd hex").code,
            "bad_args"
        );
        let junk = TraceSummaryArgs {
            tape: hex_bytes(b"not a tape"),
        };
        assert_eq!(
            trace_summary_report(&junk).expect_err("no magic").code,
            "corrupt_tape"
        );
    }

    #[test]
    fn bug_replay_names_the_flipped_tick_and_refuses_a_clean_bundle() {
        let mut tape = proof_tape();
        let tick = 11usize;
        tape.checkpoints[tick].state ^= 1;
        let error = engine_proof::replay(&tape).expect_err("the flip diverges");
        let bundle = engine_proof::bug::collect(&tape, &error);
        let bytes = bundle.encode().expect("bundle encodes");
        let args = BugReplayArgs {
            bundle: hex_bytes(&bytes),
        };
        let out = bug_replay_report(&args).expect("the bundle replays to its divergence");
        assert_eq!(
            out.first_divergent_tick,
            u32::try_from(tick).unwrap_or(u32::MAX)
        );
        assert_eq!(out.expected_state.len(), 16);
        assert_ne!(out.expected_state, out.actual_state);
        assert_eq!(
            bug_replay_report(&args).expect("the bundle replays twice"),
            out,
            "the same bundle answers the same bytes twice"
        );

        // A bundle whose tape replays clean has no tick to name.
        let clean = engine_proof::record(engine_proof::Config { seed: 7, ticks: 4 }, &[])
            .expect("control records");
        let clean_error = engine_proof::Error::Invalid("fixture: no failure to collect");
        let clean_bundle = engine_proof::bug::collect(&clean, &clean_error);
        let clean_args = BugReplayArgs {
            bundle: hex_bytes(&clean_bundle.encode().expect("clean bundle encodes")),
        };
        assert_eq!(
            bug_replay_report(&clean_args)
                .expect_err("clean refuses")
                .code,
            "clean_replay"
        );
        let junk = BugReplayArgs {
            bundle: hex_bytes(b"not a bundle"),
        };
        assert_eq!(
            bug_replay_report(&junk).expect_err("junk").code,
            "corrupt_bundle"
        );
    }

    /// The RB1 replay verbs agree with the local arena scan and name the same
    /// tick: verify and bisect share one answer.
    #[test]
    fn replay_verbs_agree_with_the_arena_scan_and_name_the_twin() {
        let config = engine_arena::ArenaConfig {
            seed: 2040,
            ticks: 60,
            fighters: 2,
            half_extent_m: 20.0,
            table_version: 1,
        };
        let tape = crate::play::build_tape(&[], config.ticks, config.fighters)
            .expect("an empty tape builds");
        let chain: Vec<u64> = engine_arena::run(&config, &tape)
            .iter()
            .map(|point| point.hash)
            .collect();
        let args = ReplayArgs {
            seed: Some(config.seed),
            ticks: Some(config.ticks),
            participants: Some(config.fighters),
            half_extent_m: Some(config.half_extent_m),
            inputs: Some(Vec::new()),
            chain: chain.clone(),
        };
        let clean = replay_report(&args, false).expect("clean verifies");
        assert!(clean.clean);
        assert_eq!(clean.ticks, 60);
        assert_eq!(replay_report(&args, true).expect("clean bisects"), clean);

        let mut dirty = args;
        dirty.chain[8] = dirty.chain[8].wrapping_add(1);
        let hit = replay_report(&dirty, false).expect("dirty verifies");
        assert_eq!(hit.tick, Some(8), "{hit:?}");
        assert_eq!(replay_report(&dirty, true).expect("dirty bisects"), hit);
    }

    /// The verify feature's `replay.bisect` must route a metric request to the
    /// receipt bisect, not the tape body: same seeded crossing, same answer.
    #[test]
    fn replay_bisect_routes_metric_mode_under_the_verify_pack() {
        let mut receipts = vec![1.0f64; 120];
        for value in receipts.iter_mut().skip(41) {
            *value = 12.0;
        }
        let args = ReplayBisectArgs {
            ticks: Some(120),
            metric: Some("sunk_cm_max".to_string()),
            floor: Some(3.0),
            receipts: Some(receipts),
            ..ReplayBisectArgs::default()
        };
        let first = replay_bisect_core(&args).expect("metric mode routes");
        assert_eq!(first.tick, Some(41), "seeded crossing: {first:?}");
        assert!(!first.clean);
        assert_eq!(first.metric.as_deref(), Some("sunk_cm_max"));
        assert_eq!(first.value, Some(12.0));
        let second = replay_bisect_core(&args).expect("repeats");
        assert_eq!(first, second, "same receipts name the same tick twice");
    }

    #[test]
    fn proof_report_bisects_the_seeded_bug_and_digests_the_mirror_twice() {
        let first = proof_report(&VerifyProofArgs {
            tasks: Some(20),
            checks: None,
        })
        .expect("proof runs");
        assert_eq!(first.seeded_bug, SEEDED_BUG_ID);
        assert_eq!(
            first.seeded_tick, SEEDED_TICK,
            "the seeded probe bisects to its seeded tick"
        );
        assert_eq!(first.seeded_expected_tick, SEEDED_TICK);
        assert!(first.seeded_clean, "the control tape replays clean");
        assert_eq!(first.tasks, 20);
        assert_eq!(first.mirror.len(), 20 * 3, "every task on every backend");
        let second = proof_report(&VerifyProofArgs {
            tasks: Some(20),
            checks: None,
        })
        .expect("repeats");
        assert_eq!(first, second, "the same proof answers twice");
        // Every digest is 16 lowercase hex chars.
        for row in &first.mirror {
            assert_eq!(row.digest.len(), 16, "{row:?}");
            assert!(row
                .digest
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)));
        }
        // Out-of-range tasks refuse typed.
        let refused = proof_report(&VerifyProofArgs {
            tasks: Some(0),
            checks: None,
        })
        .expect_err("zero refuses");
        assert_eq!(refused.code, "bad_args");
        let refused = proof_report(&VerifyProofArgs {
            tasks: Some(1_000),
            checks: None,
        })
        .expect_err("too many refuses");
        assert_eq!(refused.code, "bad_args");
    }

    /// Synthetic pair: the evaluator is pure, so every stat and comparison is
    /// exercised without a resimulation.
    fn synthetic(events: &[i64], hits: &[i64], damage: &[i64], hashes: &[u64]) -> RunMetrics {
        RunMetrics {
            events: events.to_vec(),
            hits: hits.to_vec(),
            damage: damage.to_vec(),
            hashes: hashes.to_vec(),
        }
    }

    #[test]
    fn check_rules_parse_every_stat_comparison_and_side() {
        for stat in CheckStat::ALL {
            for cmp in CheckCmp::ALL {
                let line = format!("hits.{} {} 3", stat.token(), cmp.token());
                let parsed = parse_check(&line).expect("a metric rule parses");
                match &parsed.rule {
                    CheckRule::Metric {
                        side,
                        metric,
                        stat: got,
                        cmp: got_cmp,
                        bound,
                    } => {
                        assert_eq!(*side, RuleSide::Candidate, "{line}");
                        assert_eq!(*metric, CheckMetric::Hits, "{line}");
                        assert_eq!(*got, stat, "{line}");
                        assert_eq!(*got_cmp, cmp, "{line}");
                        assert_eq!(*bound, 3, "{line}");
                    }
                    other => panic!("`{line}` parsed as {other:?}"),
                }
                if stat == CheckStat::Delta {
                    // Delta spans both runs, so it refuses the base: prefix.
                    let prefixed = format!("base:hits.delta {} 0", cmp.token());
                    assert_eq!(
                        parse_check(&prefixed).expect_err("base:delta refuses").code,
                        "bad_rule",
                        "{prefixed}"
                    );
                    continue;
                }
                let prefixed = format!("base:damage.{} {} -2", stat.token(), cmp.token());
                let parsed = parse_check(&prefixed).expect("a base rule parses");
                match &parsed.rule {
                    CheckRule::Metric {
                        side,
                        metric,
                        bound,
                        ..
                    } => {
                        assert_eq!(*side, RuleSide::Base, "{prefixed}");
                        assert_eq!(*metric, CheckMetric::Damage, "{prefixed}");
                        assert_eq!(*bound, -2, "{prefixed}");
                    }
                    other => panic!("`{prefixed}` parsed as {other:?}"),
                }
            }
        }
        assert_eq!(
            parse_check("  no_divergence  ").expect("trimmed").rule,
            CheckRule::NoDivergence
        );
        assert_eq!(
            parse_check("no_divergence_before 8")
                .expect("tick rule")
                .rule,
            CheckRule::NoDivergenceBefore(8)
        );
    }

    #[test]
    fn malformed_rules_refuse_typed() {
        for (line, code) in [
            ("", "bad_rule"),
            ("hits", "bad_rule"),
            ("hits.max", "bad_rule"),
            ("hits.max < ", "bad_rule"),
            ("hits.max <> 2", "bad_rule"),
            ("hits.max < 2.5", "bad_rule"),
            ("hits.max < two", "bad_rule"),
            ("no_divergence 8", "bad_rule"),
            ("no_divergence_before nine", "bad_rule"),
            ("pops.max < 2", "bad_metric"),
            ("hits.median < 2", "bad_stat"),
        ] {
            let error = parse_check(line).expect_err(line);
            assert_eq!(error.code, code, "`{line}`: {}", error.message);
            assert!(
                error.message.contains("re-run"),
                "`{line}` names a recovery: {}",
                error.message
            );
        }
    }

    #[test]
    fn delta_is_candidate_final_minus_base_final() {
        let base = synthetic(&[0, 1, 0], &[1, 2, 3], &[10, 20, 30], &[1, 1, 1]);
        let candidate = synthetic(&[0, 1, 0], &[1, 2, 9], &[10, 20, 40], &[1, 1, 2]);
        let transitions = chain_transitions(&base.hashes, &candidate.hashes);
        assert_eq!(transitions.first, Some(2));
        assert_eq!(transitions.differing, 1);
        let check = parse_check("hits.delta == 6").expect("parses");
        let verdict = evaluate_check(&check, &base, &candidate, transitions);
        assert!(verdict.pass, "9 - 3 = 6: {}", verdict.reason);
        assert!(verdict.reason.contains("hits.delta"), "{}", verdict.reason);
        let check = parse_check("damage.delta == 10").expect("parses");
        let verdict = evaluate_check(&check, &base, &candidate, transitions);
        assert!(verdict.pass, "40 - 30 = 10: {}", verdict.reason);
        // The base prefix reads the unedited run, never the candidate.
        let base_rule = parse_check("base:hits.final == 3").expect("parses");
        assert!(
            evaluate_check(&base_rule, &base, &candidate, transitions).pass,
            "base hits final is 3"
        );
        let candidate_rule = parse_check("hits.final == 3").expect("parses");
        let verdict = evaluate_check(&candidate_rule, &base, &candidate, transitions);
        assert!(
            !verdict.pass,
            "candidate hits final is 9: {}",
            verdict.reason
        );
        assert!(verdict.reason.contains("candidate hits.final"));
    }

    #[test]
    fn a_failing_check_names_the_metric_it_refused() {
        let base = synthetic(&[1, 2, 3], &[1, 1, 1], &[0, 0, 0], &[7, 7, 7]);
        let candidate = synthetic(&[1, 2, 3], &[1, 1, 5], &[0, 0, 0], &[7, 7, 7]);
        let transitions = chain_transitions(&base.hashes, &candidate.hashes);
        assert_eq!(transitions.first, None, "same hashes agree");
        let check = parse_check("hits.max <= 2").expect("parses");
        let verdict = evaluate_check(&check, &base, &candidate, transitions);
        assert!(!verdict.pass);
        assert_eq!(
            verdict.reason, "candidate hits.max is 5, not <= 2",
            "the reason names the metric, the value and the bound"
        );
        let passing = parse_check("events.max >= 3").expect("parses");
        assert!(evaluate_check(&passing, &base, &candidate, transitions).pass);
        // The divergence rules read the checksum transitions.
        let no_divergence = parse_check("no_divergence").expect("parses");
        assert!(evaluate_check(&no_divergence, &base, &candidate, transitions).pass);
        let early = parse_check("no_divergence_before 0").expect("parses");
        assert!(evaluate_check(&early, &base, &candidate, transitions).pass);
    }

    #[test]
    fn seeded_pair_check_report_names_both_runs_and_the_checksum_tick() {
        use engine_playtest::seeded_bug::MATCH_TICKS;
        let lines = vec![
            "no_divergence".to_owned(),
            "no_divergence_before 8".to_owned(),
            "damage.min >= 0".to_owned(),
        ];
        let run = check_run(&lines).expect("the seeded pair runs");
        assert_eq!(
            run.transitions.first,
            Some(SEEDED_TICK),
            "the probe leaves the control chain at the seeded tick"
        );
        assert!(run.transitions.differing >= 1, "at least one tick differs");
        assert_eq!(run.base.hashes.len(), run.candidate.hashes.len());
        assert_eq!(
            run.base.hashes.len(),
            usize::try_from(MATCH_TICKS).unwrap_or(0)
        );
        assert!(!run.rows[0].1.pass, "the seeded edit diverges");
        assert!(
            run.rows[0].1.reason.contains("tick 8"),
            "{}",
            run.rows[0].1.reason
        );
        assert!(run.rows[1].1.pass, "{}", run.rows[1].1.reason);
        assert!(run.rows[2].1.pass, "{}", run.rows[2].1.reason);
        let text = run.render();
        for marker in [
            "CHECKS FAILED: 2/3 passed.",
            "[FAIL] no_divergence",
            "[pass] no_divergence_before 8",
            "checksums: candidate leaves the base chain at tick 8",
            "judge by metrics",
            "base events start",
            "candidate events start",
            "base hits start",
            "candidate damage start",
            "delta (candidate final - base final): events",
        ] {
            assert!(text.contains(marker), "report names {marker}: {text}");
        }
        let again = check_run(&lines).expect("repeats");
        assert_eq!(again.render(), text, "the same rules render twice");
        assert_eq!(again.outcomes(), run.outcomes());
    }

    #[test]
    fn proof_report_carries_the_check_block_only_when_asked() {
        use engine_playtest::seeded_bug::MATCH_TICKS;
        let first = proof_report(&VerifyProofArgs {
            tasks: Some(1),
            checks: Some(vec!["no_divergence_before 8".to_owned()]),
        })
        .expect("proof runs with checks");
        assert_eq!(first.checks_total, 1);
        assert_eq!(first.checks_passed, 1);
        assert!(first.checks[0].pass);
        assert!(
            first.verdict.contains("CHECKS PASSED: 1/1 passed."),
            "{}",
            first.verdict
        );
        assert_eq!(first.base.len(), CheckMetric::ALL.len());
        assert_eq!(first.candidate.len(), CheckMetric::ALL.len());
        assert_eq!(
            first
                .base
                .iter()
                .map(|row| row.metric.as_str())
                .collect::<Vec<_>>(),
            ["events", "hits", "damage"]
        );
        assert_eq!(first.transitions.first_tick, Some(SEEDED_TICK));
        assert_eq!(first.transitions.ticks, MATCH_TICKS);
        assert!(first.transitions.differing_ticks > 0);
        let second = proof_report(&VerifyProofArgs {
            tasks: Some(1),
            checks: Some(vec!["no_divergence_before 8".to_owned()]),
        })
        .expect("repeats");
        assert_eq!(first, second, "the same checks answer twice");

        // Unasked: the check block stays empty and the old fields still answer.
        let plain = proof_report(&VerifyProofArgs {
            tasks: Some(1),
            checks: None,
        })
        .expect("plain proof runs");
        assert!(plain.verdict.is_empty());
        assert!(plain.checks.is_empty());
        assert!(plain.base.is_empty() && plain.candidate.is_empty());
        assert_eq!(plain.transitions, VerifyTransitions::default());

        // A bad rule refuses before the seeded drive and the mirror run.
        let refused = proof_report(&VerifyProofArgs {
            tasks: Some(1),
            checks: Some(vec!["pops.max < 1".to_owned()]),
        })
        .expect_err("an unknown metric refuses");
        assert_eq!(refused.code, "bad_metric");
        assert!(refused.message.contains("events, hits, damage"));
    }
}

// WG2: this pack publishes its own row; `packs/mod.rs` keeps no hand list.
#[cfg(feature = "verify")]
#[expect(
    unsafe_code,
    reason = "linkme publishes a pack row through a link section"
)]
#[linkme::distributed_slice(super::PACK_MODULES)]
static PACK_ROW: super::PackModule = super::PackModule {
    order: 15,
    name: PACK,
    default_loaded: false,
    tools: &[
        "replay.verify",
        "replay.bisect",
        "bug.replay",
        "trace.summary",
        "verify.proof",
    ],
    build: pack,
};
