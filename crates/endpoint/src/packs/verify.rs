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
//! Units: fixed ticks, integer hashes (hex text on the wire), hex text for
//! proof tape and bundle bytes. Determinism: the same args give the same
//! answer twice; no wall clock, no float compare, no map order. Errors:
//! [`McpToolError`] with `bad_args`, `corrupt_tape`, `corrupt_bundle`,
//! `clean_replay`, `no_chain` or `seeded_bug`, each naming the recovery; the
//! replay verbs keep the `replay` pack's own codes.
//!
//! DTO structs carry no field docs on purpose: every doc line inflates the
//! advertised schemas, and the discovery budget is a gate.

use engine_arena::Checkpoint;
use engine_core::hash::hex16;
use engine_core::{decode_seal_hex, Fnv64};
use engine_playtest::mirror::{run_mirror, MirrorReport};
use engine_playtest::mirror_kit::MirrorRow;
use engine_playtest::replay::{bisect_tape, verify_tape, ReplayError};
use engine_playtest::seeded_bug::{
    bisect_seeded, seeded_bugfile, verify_seeded_clean, SEEDED_BUG_ID, SEEDED_TICK,
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
    service.dispatch(TOOL_REPLAY_BISECT, |_| {
        replay_report(&args.as_replay(), true).map(ReplayBisectReport::from)
    })
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

/// `verify.proof` request: how many mirror tasks to report (all when omitted).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct VerifyProofArgs {
    pub tasks: Option<u32>,
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

/// `verify.proof` answer: the A7 seeded bug verdict and one digest per mirror
/// task per backend, task-major in report order.
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
    Ok(VerifyProofOutput {
        seeded_bug: SEEDED_BUG_ID.to_owned(),
        seeded_tick,
        seeded_expected_tick: SEEDED_TICK,
        seeded_clean: verify_seeded_clean(&bug),
        tasks: u32::try_from(tasks).unwrap_or(u32::MAX),
        mirror: mirror_task_digests(&report, tasks),
    })
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
    description = "Run the verifier proof through the door: bisect the A7 seeded bug tape to its seeded tick and digest every mirror task outcome, so a parity run compares this door with the playtest drive. Read-only.",
    readonly = true,
    args = VerifyProofArgs,
    output = VerifyProofOutput,
    invoke = verify_proof
);

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

    #[test]
    fn proof_report_bisects_the_seeded_bug_and_digests_the_mirror_twice() {
        let first = proof_report(&VerifyProofArgs { tasks: Some(20) }).expect("proof runs");
        assert_eq!(first.seeded_bug, SEEDED_BUG_ID);
        assert_eq!(
            first.seeded_tick, SEEDED_TICK,
            "the seeded probe bisects to its seeded tick"
        );
        assert_eq!(first.seeded_expected_tick, SEEDED_TICK);
        assert!(first.seeded_clean, "the control tape replays clean");
        assert_eq!(first.tasks, 20);
        assert_eq!(first.mirror.len(), 20 * 3, "every task on every backend");
        let second = proof_report(&VerifyProofArgs { tasks: Some(20) }).expect("repeats");
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
        let refused = proof_report(&VerifyProofArgs { tasks: Some(0) }).expect_err("zero refuses");
        assert_eq!(refused.code, "bad_args");
        let refused =
            proof_report(&VerifyProofArgs { tasks: Some(1_000) }).expect_err("too many refuses");
        assert_eq!(refused.code, "bad_args");
    }
}
