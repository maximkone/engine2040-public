//! `replay` pack: verify a tape against its recorded hash chain, and bisect the
//! first tick a monotone view-receipt floor crosses.
//!
//! Two read-only verbs, `replay.verify` and `replay.bisect`. Verify mode
//! resimulates one tape with [`engine_arena::run`] and names the first tick
//! whose hash leaves the recorded chain. Metric mode bisects the first tick
//! whose prefix run crosses a view-receipt floor: a prefix maximum of
//! `sunk_cm_max`, `nonfinite`, `error_parts`, `eye_in_body` or
//! `eye_below_ground` never decreases, so "crossed" is monotone and a binary
//! search over tick prefixes is sound. A clean prefix run reports `clean`.
//! An agent reads the verdict through `api.call` instead of driving the sim.
//!
//! Consumer: the MCP door (`super::mcp::McpServer`) lists the verbs after
//! `api.load` and dispatches them by exact name. Data owner for the bout is
//! `engine-arena`; this module owns only the verb DTOs, validation and the
//! typed report. Units: whole ticks for `ticks` and `tick`, whole fighters
//! for `participants`, meters for `half_extent_m`, `u64` hashes (decimal
//! strings on the wire), receipt samples in the metric's own unit
//! (centimeters for `sunk_cm_max`, counts for the others). Determinism: the
//! same seed plus tape plus chain (or the same receipts) twice seals the same
//! report; integer compares for hashes, total-order float compares for prefix
//! maxima, no clock. Errors: [`McpToolError`] with
//! `bad_ticks`, `bad_fighters`, `bad_extent`, `too_many_inputs`, `bad_chain`,
//! `no_chain`, `invalid_tape`, `corrupt_tape`, `bad_metric`, `bad_floor`,
//! `no_receipts` and `bad_receipts`, each naming the legal values plus one
//! recovery.
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
/// Verb: binary-search a tape for its first divergent tick, or a monotone
/// receipt metric for the first tick its prefix crosses a floor.
pub const TOOL_REPLAY_BISECT: &str = "replay.bisect";

/// Monotone view-receipt metrics `replay.bisect` accepts: each is a prefix
/// maximum in the `arena view:` line, so it never decreases over a run.
pub const BISECT_METRICS: [&str; 5] = [
    "sunk_cm_max",
    "nonfinite",
    "error_parts",
    "eye_in_body",
    "eye_below_ground",
];

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
    pub fn serialize<S: Serializer>(values: &[u64], out: S) -> Result<S::Ok, S::Error> {
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

/// Refusal for a metric token outside [`BISECT_METRICS`].
fn bad_metric(metric: &str) -> McpToolError {
    McpToolError::new(
        "bad_metric",
        format!(
            "metric {metric:?} is not one of {}; re-run with one of those monotone receipt metrics",
            BISECT_METRICS.join(", ")
        ),
    )
}

/// Refusal for a floor that is missing, non-finite or negative.
fn bad_floor(why: &str) -> McpToolError {
    McpToolError::new(
        "bad_floor",
        format!("floor {why}; submit a finite floor of 0 or more, then re-run"),
    )
}

/// Refusal for metric mode without the per-tick receipt samples.
fn no_receipts() -> McpToolError {
    McpToolError::new(
        "no_receipts",
        "receipts holds 0 samples; submit one finite value per tick in tick order, then re-run",
    )
}

/// Refusal for receipt samples that are missing, mis-sized or not finite.
fn bad_receipts(message: String) -> McpToolError {
    McpToolError::new("bad_receipts", message)
}

/// Refusal for a missing recorded chain; the verify pack reuses it so both
/// owners of the verb name refuse with one message.
pub(crate) fn no_chain() -> McpToolError {
    McpToolError::new(
        "no_chain",
        "chain holds 0 hashes; submit the recorded per-tick chain in tick order, then re-run",
    )
}

/// Validate one request into its arena config, tape and chain. Crate-visible:
/// the `verify` pack's replay verbs reuse this validation, so the two owners of
/// the name keep one range and refusal contract.
pub(crate) fn replay_inputs(
    args: &ReplayArgs,
) -> Result<(ArenaConfig, BodySnap, Vec<u64>), McpToolError> {
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

/// One crossing of a monotone receipt floor: the 0-based tick and the prefix
/// value that crossed there.
#[derive(Debug, Clone, Copy, PartialEq)]
struct BisectHit {
    tick: u32,
    value: f64,
}

/// First tick whose prefix maximum of `value_at(0..=tick)` is above `floor`,
/// or `None` when the whole run stays at or under it.
///
/// Pure: no sim, no clock, no file. `value_at` yields one sample per tick in
/// tick order. A prefix maximum never decreases, so "crossed" is monotone
/// (false up to one point, true after it) and the binary search is sound. A
/// non-finite sample refuses with `bad_receipts`, because NaN would break the
/// ordering the search relies on.
fn bisect_floor<F>(
    ticks: u32,
    floor: f64,
    mut value_at: F,
) -> Result<Option<BisectHit>, McpToolError>
where
    F: FnMut(u32) -> Result<f64, McpToolError>,
{
    if ticks == 0 {
        return Err(bad_ticks(format!(
            "ticks 0 is outside 1..={MAX_PLAY_TICKS}; re-run with a tick count"
        )));
    }
    let mut prefix = Vec::with_capacity(usize::try_from(ticks).unwrap_or(usize::MAX));
    let mut running = f64::NEG_INFINITY;
    for tick in 0..ticks {
        let value = value_at(tick)?;
        if !value.is_finite() {
            return Err(bad_receipts(format!(
                "receipt at tick {tick} is not finite; submit finite values, then re-run"
            )));
        }
        running = running.max(value);
        prefix.push(running);
    }
    let crossed = |tick: u32| {
        prefix
            .get(usize::try_from(tick).unwrap_or(usize::MAX))
            .is_some_and(|value| *value > floor)
    };
    let last = ticks - 1;
    if !crossed(last) {
        return Ok(None);
    }
    let (mut lo, mut hi) = (0u32, last);
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        if crossed(mid) {
            hi = mid;
        } else {
            lo = mid + 1;
        }
    }
    Ok(Some(BisectHit {
        tick: lo,
        value: prefix[usize::try_from(lo).unwrap_or(0)],
    }))
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

/// Run one `replay.bisect` in metric mode: validate the monotone metric, its
/// floor and one finite sample per tick, then bisect the prefix maxima.
///
/// The samples are the per-tick readings of `metric` (for example a
/// `--view-brief` row's `sunk_cm_max`), in the metric's own unit. The prefix
/// maximum makes the crossing monotone, so the answer is the first bad tick.
/// The real view host is not wired here yet: a caller submits the samples.
/// `seed`, `participants` and `half_extent_m` stay metadata for that run; they
/// are not resimulated here, so a bad chain is not needed either.
pub(crate) fn bisect_metric_report(
    args: &ReplayBisectArgs,
    metric: &str,
) -> Result<ReplayBisectReport, McpToolError> {
    if !BISECT_METRICS.contains(&metric) {
        return Err(bad_metric(metric));
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
    let floor = args.floor.ok_or_else(|| bad_floor("missing"))?;
    if !floor.is_finite() || floor < 0.0 {
        return Err(bad_floor(&format!("{floor} is not finite or is below 0")));
    }
    let receipts = args.receipts.as_deref().ok_or_else(no_receipts)?;
    if receipts.is_empty() {
        return Err(no_receipts());
    }
    if receipts.len() != usize::try_from(ticks).unwrap_or(usize::MAX) {
        return Err(bad_receipts(format!(
            "receipts holds {} samples for {ticks} ticks; submit one value per tick, then re-run",
            receipts.len()
        )));
    }
    let crossing = bisect_floor(ticks, floor, |tick| {
        receipts
            .get(usize::try_from(tick).unwrap_or(usize::MAX))
            .copied()
            .ok_or_else(|| bad_receipts(format!("receipt for tick {tick} missing")))
    })?;
    Ok(ReplayBisectReport {
        clean: crossing.is_none(),
        tick: crossing.map(|hit| hit.tick),
        expected: None,
        observed: None,
        ticks,
        metric: Some(metric.to_string()),
        floor: Some(floor),
        value: crossing.map(|hit| hit.value),
    })
}

/// `replay.verify` handler: the pure report under the audited dispatch seam.
fn replay_verify(service: &McpServer, args: &ReplayArgs) -> Result<ReplayReport, McpToolError> {
    service.dispatch(TOOL_REPLAY_VERIFY, |_| verify_report(args))
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
    /// `replay.bisect` tool: binary-search a tape or a monotone receipt metric. Read-only.
    ReplayBisectTool,
    name = TOOL_REPLAY_BISECT,
    description = "Binary-search a resimulated tape for the first tick whose hash leaves the recorded chain, or (with metric, floor and receipts) the first tick whose monotone view-receipt prefix crosses the floor. Read-only.",
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
    #[serde(default)]
    pub metric: Option<String>,
    #[serde(default)]
    pub floor: Option<f64>,
    #[serde(default)]
    pub receipts: Option<Vec<f64>>,
}

/// `replay.bisect` answer: same shape as verify in chain mode; metric mode
/// names the metric, floor and crossing value instead of two hashes.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
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
    #[serde(default)]
    pub metric: Option<String>,
    #[serde(default)]
    pub floor: Option<f64>,
    #[serde(default)]
    pub value: Option<f64>,
}

impl ReplayBisectArgs {
    /// The same request seen as the verify shape: one owner for the field
    /// copy, so a new field is added once.
    pub(crate) fn as_replay(&self) -> ReplayArgs {
        ReplayArgs {
            seed: self.seed,
            ticks: self.ticks,
            participants: self.participants,
            half_extent_m: self.half_extent_m,
            inputs: self.inputs.clone(),
            chain: self.chain.clone(),
        }
    }
}

impl From<ReplayReport> for ReplayBisectReport {
    /// Same report, bisect's own schema name.
    fn from(report: ReplayReport) -> Self {
        Self {
            clean: report.clean,
            tick: report.tick,
            expected: report.expected,
            observed: report.observed,
            ticks: report.ticks,
            metric: None,
            floor: None,
            value: None,
        }
    }
}

fn replay_bisect_verb(
    service: &McpServer,
    args: &ReplayBisectArgs,
) -> Result<ReplayBisectReport, McpToolError> {
    service.dispatch(TOOL_REPLAY_BISECT, |_| {
        if let Some(metric) = args.metric.as_deref() {
            return bisect_metric_report(args, metric);
        }
        bisect_report(&args.as_replay()).map(ReplayBisectReport::from)
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
                metric: None,
                floor: None,
                receipts: None,
            },
        )
        .expect("bisect verb serves");
        assert_eq!(verb.tick, Some(8));
    }

    #[test]
    fn bisect_floor_names_a_known_crossing_twice() {
        // A pure predicate: the prefix crosses at tick 41, once and forever.
        let sample = |tick: u32| Ok(if tick < 41 { 1.0 } else { 12.0 });
        let first = bisect_floor(120, 3.0, sample).expect("searches");
        assert_eq!(
            first,
            Some(BisectHit {
                tick: 41,
                value: 12.0
            })
        );
        let second = bisect_floor(120, 3.0, sample).expect("repeats");
        assert_eq!(first, second, "same predicate, same crossing twice");
        assert_eq!(
            bisect_floor(120, 3.0, |_| Ok(1.0)).expect("calm searches"),
            None,
            "a prefix under the floor never crosses"
        );
        // A value equal to the floor is not a crossing; the next tick is.
        let edge = bisect_floor(8, 0.0, |tick| Ok(if tick == 0 { 0.0 } else { 1.0 }))
            .expect("edge searches");
        assert_eq!(edge.map(|hit| hit.tick), Some(1));
        // A NaN sample breaks the ordering, so it refuses typed.
        let error = bisect_floor(8, 1.0, |tick| Ok(if tick == 3 { f64::NAN } else { 0.0 }))
            .expect_err("NaN refuses");
        assert_eq!(error.code, "bad_receipts");
        assert_eq!(
            bisect_floor(0, 1.0, |_| Ok(0.0))
                .expect_err("zero ticks refuses")
                .code,
            "bad_ticks"
        );
    }

    #[test]
    fn replay_bisect_metric_names_the_seeded_tick_twice() {
        let receipts = |ticks: usize| {
            let mut values = vec![1.0f64; ticks];
            for value in values.iter_mut().skip(41) {
                *value = 12.0;
            }
            values
        };
        let args = ReplayBisectArgs {
            seed: Some(2040),
            ticks: Some(120),
            participants: Some(2),
            half_extent_m: Some(20.0),
            inputs: Some(Vec::new()),
            chain: Vec::new(),
            metric: Some("sunk_cm_max".to_string()),
            floor: Some(3.0),
            receipts: Some(receipts(120)),
        };
        let first = replay_bisect_verb(&server(), &args).expect("metric bisects");
        assert_eq!(first.tick, Some(41), "seeded crossing: {first:?}");
        assert!(!first.clean);
        assert_eq!(first.metric.as_deref(), Some("sunk_cm_max"));
        assert_eq!(first.value, Some(12.0));
        assert_eq!(first.expected, None, "no hash chain in metric mode");
        let second = replay_bisect_verb(&server(), &args).expect("repeats");
        assert_eq!(first, second, "same receipts name the same tick twice");
        // A run that never crosses reads clean and names no tick.
        let calm = ReplayBisectArgs {
            receipts: Some(vec![1.0; 120]),
            ..args.clone()
        };
        let clean = replay_bisect_verb(&server(), &calm).expect("calm bisects");
        assert!(clean.clean && clean.tick.is_none(), "{clean:?}");
        // Refusals stay typed and name one recovery.
        let bad = ReplayBisectArgs {
            metric: Some("pops".to_string()),
            ..calm
        };
        assert_eq!(
            replay_bisect_verb(&server(), &bad)
                .expect_err("bad metric refuses")
                .code,
            "bad_metric"
        );
        let bad = ReplayBisectArgs {
            floor: None,
            ..args.clone()
        };
        assert_eq!(
            replay_bisect_verb(&server(), &bad)
                .expect_err("missing floor refuses")
                .code,
            "bad_floor"
        );
        let bad = ReplayBisectArgs {
            receipts: None,
            ..args.clone()
        };
        assert_eq!(
            replay_bisect_verb(&server(), &bad)
                .expect_err("missing receipts refuses")
                .code,
            "no_receipts"
        );
        let bad = ReplayBisectArgs {
            receipts: Some(vec![1.0; 60]),
            ..args.clone()
        };
        assert_eq!(
            replay_bisect_verb(&server(), &bad)
                .expect_err("short receipts refuse")
                .code,
            "bad_receipts"
        );
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

// WG2: this pack publishes its own row; `packs/mod.rs` keeps no hand list.
#[cfg(not(feature = "verify"))]
#[expect(
    unsafe_code,
    reason = "linkme publishes a pack row through a link section"
)]
#[linkme::distributed_slice(super::PACK_MODULES)]
static PACK_ROW: super::PackModule = super::PackModule {
    order: 15,
    name: PACK,
    default_loaded: false,
    tools: &["replay.verify", "replay.bisect"],
    build: pack,
};
