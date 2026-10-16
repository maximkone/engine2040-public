//! `replay` pack: verify a tape against its recorded hash chain.
//!
//! Two read-only verbs, `replay.verify` and `replay.bisect`, resimulate one
//! tape with [`engine_arena::run`] and name the first tick whose hash leaves
//! the recorded chain, so an agent reads the verdict through `api.call`
//! instead of driving the sim.
//!
//! Consumer: the MCP door (`super::mcp::McpServer`) lists the verbs after
//! `api.load` and dispatches them by exact name. Data owner for the bout is
//! `engine-arena`; this module owns only the verb DTOs, validation and the
//! typed report. Units: whole ticks for `ticks` and `tick`, whole fighters
//! for `participants`, meters for `half_extent_m`, `u64` hashes (decimal
//! strings on the wire). Determinism: the same seed plus tape plus chain
//! twice seals the same report; integer compares only, no clock, no float.
//! Errors: [`McpToolError`] with `bad_ticks`, `bad_fighters`, `bad_extent`,
//! `too_many_inputs`, `bad_chain`, `no_chain`, `invalid_tape` and
//! `corrupt_tape`, each naming the legal values plus one recovery.
//!
//! Slice note: this file calls `engine-arena` directly. When the endpoint
//! manifest may grow again, the core compare should delegate to the playtest
//! replay library (`crates/playtest/src/replay.rs`) with no second search.
//!
//! DTO structs carry no field docs on purpose: every doc line inflates the
//! advertised schemas, and the discovery budget is a gate.

use super::{PackDef, PackError, VerbSpec};
use crate::mcp::{McpServer, McpToolError};
use crate::play::play_session::dec_u64;
use crate::play::{build_tape, PlayInputDto, MAX_PLAY_INPUTS, MAX_PLAY_TICKS};
use crate::tool_capabilities::CapabilityCard;
use engine_arena::{ArenaConfig, BoutTape as BodySnap, MAX_PARTICIPANTS};
use rmcp::schemars::JsonSchema;
// `self` for schemars derive with `serde(with)`.
use rmcp::serde::{self, Deserialize, Serialize};

/// Pack name.
pub const PACK: &str = "replay";
/// Verb: resimulate a tape and report the first divergent tick.
pub const TOOL_REPLAY_VERIFY: &str = "replay.verify";
/// Verb: binary-search a tape for its first divergent tick.
pub const TOOL_REPLAY_BISECT: &str = "replay.bisect";

/// The replay pack: verify plus bisect, loaded on demand.
pub fn pack() -> Result<PackDef, PackError> {
    let mut pack = PackDef::new(
        PACK,
        "resimulate a tape against its recorded chain and name the first divergent tick",
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
    Ok(pack)
}

/// `Vec<u64>` hashes as decimal strings on the wire (JS and LLM clients
/// parse JSON numbers as f64 and round above 2^53); input takes strings or
/// plain JSON integers, one entry per tick in tick order.
mod dec_u64_vec {
    use rmcp::serde::de::Error;
    use rmcp::serde::{Deserialize, Deserializer, Serialize, Serializer};
    use rmcp::serde_json::Value;

    /// Write hashes as decimal strings, one entry per tick in tick order.
    pub fn serialize<S: Serializer>(values: &Vec<u64>, out: S) -> Result<S::Ok, S::Error> {
        let texts: Vec<String> = values.iter().map(|value| value.to_string()).collect();
        texts.serialize(out)
    }

    /// Read hashes back from decimal strings or plain JSON integers.
    pub fn deserialize<'de, D: Deserializer<'de>>(into: D) -> Result<Vec<u64>, D::Error> {
        let items = Vec::<Value>::deserialize(into)?;
        let mut out = Vec::with_capacity(items.len());
        for item in &items {
            let parsed = match item {
                Value::Number(number) => number.as_u64(),
                Value::String(text) => text.parse::<u64>().ok(),
                _ => None,
            };
            out.push(parsed.ok_or_else(|| {
                D::Error::custom(format!(
                    "expected a u64 as a decimal string or a non-negative JSON integer, got {item}"
                ))
            })?);
        }
        Ok(out)
    }
}

// ---------------------------------------------------------------------------
// Transport DTOs. Field docs are omitted on purpose: every doc line inflates
// the advertised schemas, and the discovery budget is a gate.
// ---------------------------------------------------------------------------

/// `replay.verify` / `replay.bisect` request: the bout, its tape and the
/// recorded per-tick chain to compare against.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct ReplayArgs {
    #[serde(default, with = "dec_u64::opt")]
    #[schemars(with = "Option<String>")]
    pub seed: Option<u64>,
    #[serde(default)]
    pub ticks: Option<u32>,
    #[serde(default, alias = "fighters")]
    pub participants: Option<u8>,
    #[serde(default)]
    pub half_extent_m: Option<f32>,
    #[serde(default)]
    pub inputs: Option<Vec<PlayInputDto>>,
    #[serde(default, with = "dec_u64_vec")]
    #[schemars(with = "Vec<String>")]
    pub chain: Vec<u64>,
}

/// `replay.verify` / `replay.bisect` answer: clean or the first divergent
/// tick with both hashes. `tick`, `expected` and `observed` are `None` when
/// clean; `ticks` is the resimulated tick count.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct ReplayReport {
    pub clean: bool,
    #[serde(default)]
    pub tick: Option<u32>,
    #[serde(default, with = "dec_u64::opt")]
    #[schemars(with = "Option<String>")]
    pub expected: Option<u64>,
    #[serde(default, with = "dec_u64::opt")]
    #[schemars(with = "Option<String>")]
    pub observed: Option<u64>,
    pub ticks: u32,
}

/// First tick where a resimulation leaves its recorded chain, with both
/// hashes. A length mismatch reads as divergence at the shared prefix end
/// (the missing side hashes as `0`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Divergence {
    tick: u32,
    expected: u64,
    observed: u64,
}

fn bad_ticks(message: String) -> McpToolError {
    McpToolError::new("bad_ticks", message)
}

fn no_chain() -> McpToolError {
    McpToolError::new(
        "no_chain",
        "chain holds 0 hashes; submit the recorded per-tick chain in tick order, then re-run",
    )
}

/// Validate one request into its arena config, tape and chain.
fn replay_inputs(args: &ReplayArgs) -> Result<(ArenaConfig, BodySnap, Vec<u64>), McpToolError> {
    let seed = args.seed.unwrap_or(0);
    let participants = args.participants.unwrap_or(2);
    if !(2..=MAX_PARTICIPANTS).contains(&participants) {
        return Err(McpToolError::new(
            "bad_fighters",
            format!(
                "participants {participants} is outside 2..={MAX_PARTICIPANTS}; re-run in range"
            ),
        ));
    }
    let half = args.half_extent_m.unwrap_or(20.0);
    if !half.is_finite() || !(5.0..=100.0).contains(&half) {
        return Err(McpToolError::new(
            "bad_extent",
            format!("half_extent_m {half} is outside 5.0..=100.0; re-run in range"),
        ));
    }
    let ticks = args.ticks.ok_or_else(|| {
        bad_ticks(format!(
            "ticks missing; submit 1..={MAX_PLAY_TICKS}; re-run with a tick count"
        ))
    })?;
    if ticks == 0 || ticks > MAX_PLAY_TICKS {
        return Err(bad_ticks(format!(
            "ticks {ticks} is outside 1..={MAX_PLAY_TICKS}; re-run in range"
        )));
    }
    let inputs = args.inputs.clone().unwrap_or_default();
    if inputs.len() > MAX_PLAY_INPUTS {
        return Err(McpToolError::new(
            "too_many_inputs",
            format!(
                "inputs holds {} entries; at most {MAX_PLAY_INPUTS}; re-run shorter",
                inputs.len()
            ),
        ));
    }
    if args.chain.is_empty() {
        return Err(no_chain());
    }
    if args.chain.len() > usize::try_from(MAX_PLAY_TICKS).unwrap_or(usize::MAX) {
        return Err(McpToolError::new(
            "bad_chain",
            format!(
                "chain holds {} hashes; at most {MAX_PLAY_TICKS}; re-run with the recorded prefix",
                args.chain.len()
            ),
        ));
    }
    let tape = build_tape(&inputs, ticks, participants)?;
    let config = ArenaConfig {
        seed,
        ticks,
        fighters: participants,
        half_extent_m: half,
        table_version: 1,
    };
    if config.validate().is_err() {
        return Err(McpToolError::new(
            "corrupt_tape",
            "arena refused the bout config; checked ranges already passed, so report this request verbatim",
        ));
    }
    Ok((config, tape, args.chain.clone()))
}

/// Resimulate one tape into its per-tick hashes.
fn resim_hashes(config: &ArenaConfig, tape: &BodySnap) -> Result<Vec<u64>, McpToolError> {
    let run = engine_arena::run(config, tape);
    if run.is_empty() {
        return Err(McpToolError::new(
            "corrupt_tape",
            "arena ran 0 ticks; checked ranges already passed, so report this tape verbatim",
        ));
    }
    Ok(run.iter().map(|point| point.hash).collect())
}

/// Hash at `index`, or `0` past the end (a length mismatch diverges at the
/// shared prefix end).
fn hash_at(hashes: &[u64], index: usize) -> u64 {
    hashes.get(index).copied().unwrap_or(0)
}

/// Linear scan for the first divergent index, or `None` when the two agree.
fn find_first(resim: &[u64], chain: &[u64]) -> Option<Divergence> {
    for (index, (left, right)) in resim.iter().zip(chain.iter()).enumerate() {
        if left != right {
            let tick = u32::try_from(index).ok()?;
            return Some(Divergence {
                tick,
                expected: *right,
                observed: *left,
            });
        }
    }
    if resim.len() == chain.len() {
        return None;
    }
    let shared = resim.len().min(chain.len());
    let tick = u32::try_from(shared).ok()?;
    Some(Divergence {
        tick,
        expected: hash_at(chain, shared),
        observed: hash_at(resim, shared),
    })
}

/// True when the first `len` resimulated hashes equal the recorded chain.
/// Callers hold `len <= min(lens)`, so the slices never overrun.
fn prefix_matches(resim: &[u64], chain: &[u64], len: usize) -> bool {
    resim[..len]
        .iter()
        .zip(chain[..len].iter())
        .all(|(left, right)| left == right)
}

/// Binary search for the first divergent index in `[0, shared)`: `shared`
/// mismatches while tick `0` vacuously matches, so the answer always exists.
fn search_first(resim: &[u64], chain: &[u64], shared: usize) -> usize {
    let mut lo = 0usize;
    let mut hi = shared;
    while lo + 1 < hi {
        let mid = lo + (hi - lo) / 2;
        if prefix_matches(resim, chain, mid) {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    lo
}

/// First divergence via binary search, or `None` when the two agree exactly.
/// Length mismatch reads as divergence at the shared prefix end.
fn bisect_first(resim: &[u64], chain: &[u64]) -> Option<Divergence> {
    let shared = resim.len().min(chain.len());
    if !prefix_matches(resim, chain, shared) {
        let index = search_first(resim, chain, shared);
        let tick = u32::try_from(index).ok()?;
        return Some(Divergence {
            tick,
            expected: hash_at(chain, index),
            observed: hash_at(resim, index),
        });
    }
    if resim.len() == chain.len() {
        return None;
    }
    let tick = u32::try_from(shared).ok()?;
    Some(Divergence {
        tick,
        expected: hash_at(chain, shared),
        observed: hash_at(resim, shared),
    })
}

fn report_of(first: Option<Divergence>, ticks: u32) -> ReplayReport {
    match first {
        None => ReplayReport {
            clean: true,
            tick: None,
            expected: None,
            observed: None,
            ticks,
        },
        Some(hit) => ReplayReport {
            clean: false,
            tick: Some(hit.tick),
            expected: Some(hit.expected),
            observed: Some(hit.observed),
            ticks,
        },
    }
}

/// Run one `replay.verify`: resimulate and scan for the first divergent tick.
pub fn verify_report(args: &ReplayArgs) -> Result<ReplayReport, McpToolError> {
    let (config, tape, chain) = replay_inputs(args)?;
    let resim = resim_hashes(&config, &tape)?;
    let ticks = u32::try_from(resim.len()).unwrap_or(u32::MAX);
    Ok(report_of(find_first(&resim, &chain), ticks))
}

/// Run one `replay.bisect`: resimulate and binary-search the divergent tick.
pub fn bisect_report(args: &ReplayArgs) -> Result<ReplayReport, McpToolError> {
    let (config, tape, chain) = replay_inputs(args)?;
    let resim = resim_hashes(&config, &tape)?;
    let ticks = u32::try_from(resim.len()).unwrap_or(u32::MAX);
    Ok(report_of(bisect_first(&resim, &chain), ticks))
}

/// `replay.verify` handler: the pure report under the audited dispatch seam.
fn replay_verify(service: &McpServer, args: &ReplayArgs) -> Result<ReplayReport, McpToolError> {
    service.dispatch(TOOL_REPLAY_VERIFY, |_| verify_report(args))
}

/// `replay.bisect` handler: the pure report under the audited dispatch seam.
fn replay_bisect(service: &McpServer, args: &ReplayArgs) -> Result<ReplayReport, McpToolError> {
    service.dispatch(TOOL_REPLAY_BISECT, |_| bisect_report(args))
}

crate::e2_tool!(
    /// `replay.verify` tool: resimulate a tape and report the first divergent tick. Read-only.
    ReplayVerifyTool,
    name = TOOL_REPLAY_VERIFY,
    description = "Resimulate a tape against its recorded per-tick chain and report clean or the first divergent tick with both hashes. Read-only.",
    readonly = true,
    args = ReplayArgs,
    output = ReplayReport,
    invoke = replay_verify
);

crate::e2_tool!(
    /// `replay.bisect` tool: binary-search a tape for its first divergent tick. Read-only.
    ReplayBisectTool,
    name = TOOL_REPLAY_BISECT,
    description = "Binary-search a resimulated tape for the first tick whose hash leaves the recorded chain. Read-only.",
    readonly = true,
    args = ReplayBisectArgs,
    output = ReplayBisectReport,
    invoke = replay_bisect_verb
);

/// `replay.bisect` request: same shape as verify (its own schema entry keeps
/// each verb self-describing in `api.describe`).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct ReplayBisectArgs {
    #[serde(default, with = "dec_u64::opt")]
    #[schemars(with = "Option<String>")]
    pub seed: Option<u64>,
    #[serde(default)]
    pub ticks: Option<u32>,
    #[serde(default, alias = "fighters")]
    pub participants: Option<u8>,
    #[serde(default)]
    pub half_extent_m: Option<f32>,
    #[serde(default)]
    pub inputs: Option<Vec<PlayInputDto>>,
    #[serde(default, with = "dec_u64_vec")]
    #[schemars(with = "Vec<String>")]
    pub chain: Vec<u64>,
}

/// `replay.bisect` answer: same shape as verify.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct ReplayBisectReport {
    pub clean: bool,
    #[serde(default)]
    pub tick: Option<u32>,
    #[serde(default, with = "dec_u64::opt")]
    #[schemars(with = "Option<String>")]
    pub expected: Option<u64>,
    #[serde(default, with = "dec_u64::opt")]
    #[schemars(with = "Option<String>")]
    pub observed: Option<u64>,
    pub ticks: u32,
}

fn replay_bisect_verb(
    service: &McpServer,
    args: &ReplayBisectArgs,
) -> Result<ReplayBisectReport, McpToolError> {
    service.dispatch(TOOL_REPLAY_BISECT, |_| {
        bisect_report(&ReplayArgs {
            seed: args.seed,
            ticks: args.ticks,
            participants: args.participants,
            half_extent_m: args.half_extent_m,
            inputs: args.inputs.clone(),
            chain: args.chain.clone(),
        })
        .map(|report| ReplayBisectReport {
            clean: report.clean,
            tick: report.tick,
            expected: report.expected,
            observed: report.observed,
            ticks: report.ticks,
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_editor::capability::Capability;
    use rmcp::handler::server::router::tool::SyncTool;

    fn server() -> McpServer {
        McpServer::new(7, Capability::unrestricted(7))
    }

    fn clean_args(ticks: u32) -> ReplayArgs {
        let config = ArenaConfig {
            seed: 2040,
            ticks,
            fighters: 2,
            half_extent_m: 20.0,
            table_version: 1,
        };
        let tape = BodySnap::default();
        let chain: Vec<u64> = engine_arena::run(&config, &tape)
            .iter()
            .map(|point| point.hash)
            .collect();
        assert!(!chain.is_empty(), "control records a chain");
        ReplayArgs {
            seed: Some(2040),
            ticks: Some(ticks),
            participants: Some(2),
            half_extent_m: Some(20.0),
            inputs: Some(Vec::new()),
            chain,
        }
    }

    #[test]
    fn replay_verify_clean_reports_clean_x2() {
        let args = clean_args(60);
        let first = verify_report(&args).expect("clean verifies");
        assert!(first.clean, "control versus its chain is clean: {first:?}");
        assert_eq!(first.tick, None);
        assert_eq!(first.ticks, 60);
        let second = verify_report(&args).expect("repeats");
        assert_eq!(first, second, "same inputs seal the same report twice");
        let tool = ReplayVerifyTool::invoke(&server(), args).expect("verb serves");
        assert!(tool.clean);
    }

    #[test]
    fn replay_verify_and_bisect_name_tick_8() {
        let mut args = clean_args(60);
        assert!(args.chain.len() > 8, "chain covers tick 8");
        args.chain[8] = args.chain[8].wrapping_add(1);
        let verify = verify_report(&args).expect("dirty verifies");
        assert!(!verify.clean);
        assert_eq!(verify.tick, Some(8), "verify names tick 8: {verify:?}");
        assert_ne!(verify.expected, verify.observed);
        let bisect = bisect_report(&args).expect("dirty bisects");
        assert_eq!(bisect, verify, "bisect agrees with verify");
        let again = bisect_report(&args).expect("repeats");
        assert_eq!(bisect, again, "same tape bisects identically twice");
        let verb = replay_bisect_verb(
            &server(),
            &ReplayBisectArgs {
                seed: args.seed,
                ticks: args.ticks,
                participants: args.participants,
                half_extent_m: args.half_extent_m,
                inputs: args.inputs.clone(),
                chain: args.chain.clone(),
            },
        )
        .expect("bisect verb serves");
        assert_eq!(verb.tick, Some(8));
    }

    #[test]
    fn replay_refuses_empty_chain_and_bad_ticks() {
        let mut args = clean_args(60);
        args.chain.clear();
        let error = verify_report(&args).expect_err("empty chain refuses");
        assert_eq!(error.code, "no_chain");
        assert!(error.message.contains("re-run"));
        let error = bisect_report(&args).expect_err("bisect refuses too");
        assert_eq!(error.code, "no_chain");
        let missing = ReplayArgs {
            ticks: None,
            ..clean_args(60)
        };
        assert_eq!(
            verify_report(&missing).expect_err("missing ticks").code,
            "bad_ticks"
        );
        let bad = ReplayArgs {
            ticks: Some(0),
            ..clean_args(60)
        };
        assert_eq!(
            verify_report(&bad).expect_err("zero ticks").code,
            "bad_ticks"
        );
        let bad = ReplayArgs {
            participants: Some(1),
            ..clean_args(60)
        };
        assert_eq!(
            verify_report(&bad).expect_err("one fighter").code,
            "bad_fighters"
        );
    }

    #[test]
    fn replay_pack_holds_two_observe_verbs() {
        let pack = pack().expect("replay pack builds");
        assert_eq!(pack.name(), PACK);
        assert_eq!(
            pack.verb_names(),
            vec![TOOL_REPLAY_VERIFY, TOOL_REPLAY_BISECT]
        );
        for verb in pack.verbs() {
            assert_eq!(verb.pack(), PACK);
            assert_eq!(verb.card(), CapabilityCard::Observe);
            assert!(verb.readonly());
        }
    }
}
