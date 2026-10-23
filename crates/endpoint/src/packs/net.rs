//! `net` pack: loopback bout with injected loss, latency and burst drop, plus
//! a seeded link shaper, as typed integers.
//!
//! Two read-only verbs. `net.loopback` opens a loopback bout with injected
//! loss, latency_ticks delay and burst drop and reports convergence plus the
//! first divergent tick. `net.shape` replays the same duel over a seeded
//! shape (delay_ms, jitter_ms, loss_permille) and reports the shape it
//! applied, convergence, measured loss and a sim-only digest, so the same
//! shape twice folds one digest; the isolated proof carries clean equality. An
//! agent reads the verdict through `api.call` instead of driving sockets.
//!
//! Consumer: the MCP door (`super::mcp::McpServer`) lists the verbs after
//! `api.load` and dispatches them by exact name. Data owner for the bout is
//! `engine-net`; this module owns only the verb DTOs, validation and the
//! typed reports. Units: whole percent for `loss_pct`, whole ticks for
//! `latency_ticks`, `burst_at`, `burst_len_ticks`, `delay_ticks` and
//! `jitter_ticks`, milliseconds for `delay_ms` and `jitter_ms`, permille for
//! `loss_permille`, ticks for `first_divergence`, basis points for `loss_bp`,
//! unitless `u64` for `digest` and `seed`. Determinism: the same loss plus
//! seed plus latency plus burst twice seals the same report, and the same
//! shape twice seals the same digest; integer ops only, no clock, no float,
//! no socket. Errors: [`McpToolError`] with `bad_args` for loss outside
//! `0..=100`, latency outside `0..=32`, burst outside `1..=600`, delay_ms
//! outside `0..=2000`, jitter_ms outside `0..=1000` or loss_permille outside
//! `0..=1000`, and `bout_failed` when the bout refuses.
//!
//! DTO structs carry no field docs on purpose: every doc line inflates the
//! advertised schemas, and the discovery budget is a gate.

use engine_core::Fnv64;
use rmcp::schemars::JsonSchema;
use rmcp::serde::{Deserialize, Serialize};

use super::bad_args;
use super::{PackDef, PackError, VerbSpec};
use crate::mcp::{McpServer, McpToolError};
use crate::tool_capabilities::CapabilityCard;

/// Pack name.
pub const PACK: &str = "net";
/// Verb: loopback bout with injected loss, convergence and first tick.
pub const TOOL_NET_LOOPBACK: &str = "net.loopback";
/// Verb: seeded link shape over the same duel, convergence and digest.
pub const TOOL_NET_SHAPE: &str = "net.shape";

/// The net pack: loopback bout and seeded link shaper, loaded on demand.
pub fn pack() -> Result<PackDef, PackError> {
    let mut pack = PackDef::new(
        PACK,
        "loopback bout with injected loss, convergence, first divergent tick and a seeded link shaper",
        false,
    );
    pack.add(VerbSpec::of::<NetLoopbackTool>(
        TOOL_NET_LOOPBACK,
        PACK,
        CapabilityCard::Observe,
    )?);
    pack.add(VerbSpec::of::<NetShapeTool>(
        TOOL_NET_SHAPE,
        PACK,
        CapabilityCard::Observe,
    )?);
    Ok(pack)
}

// ---------------------------------------------------------------------------
// Transport DTOs. Field docs are omitted on purpose: every doc line inflates
// the advertised schemas, and the discovery budget is a gate.
// ---------------------------------------------------------------------------

/// `net.loopback` answer: convergence, first divergent tick, digest.
///
/// `converged` is true when every seat holds the server truth with no
/// divergence. `first_divergence` is the smallest divergent tick across
/// seats, `None` when all agree. `digest` folds the deterministic bout
/// numbers, so the same inputs seal the same digest twice.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct LoopbackReport {
    pub converged: bool,
    pub first_divergence: Option<u32>,
    pub digest: u64,
}

/// Run one loopback bout with injected loss, latency and burst drop and report its verdict.
///
/// Builds the canonical bout for the profile delay, then runs it over the
/// seeded loopback wire with the same profile both ways. Loss is whole
/// percent in `0..=100`; latency is whole ticks in `0..=32`; burst is the
/// scripted outage (`DropScript` seat 1, `at`, `back = at + len`), `None`
/// means the canonical 60 plus 120. Burst `at` and `len` are whole ticks in
/// `1..=600` with `at + len <= 600`; anything outside refuses with
/// `bad_args`. A bout that refuses maps to its typed error,
/// never a panic. Integer only, no clock, no float, no socket.
pub fn loopback_report(
    loss_pct: u32,
    seed: u64,
    latency_ticks: u32,
    burst_at: Option<u32>,
    burst_len_ticks: Option<u32>,
) -> Result<LoopbackReport, McpToolError> {
    if loss_pct > 100 {
        return Err(bad_args(format!(
            "loss_pct {loss_pct} is outside 0..=100; re-run with 0..=100"
        )));
    }
    if latency_ticks > 32 {
        return Err(bad_args(format!(
            "latency_ticks {latency_ticks} is outside 0..=32; re-run with 0..=32"
        )));
    }
    let at = burst_at.unwrap_or(60);
    let len = burst_len_ticks.unwrap_or(120);
    if !(1..=600).contains(&at) {
        return Err(bad_args(format!(
            "burst_at {at} is outside 1..=600; re-run with 1..=600"
        )));
    }
    if !(1..=600).contains(&len) {
        return Err(bad_args(format!(
            "burst_len_ticks {len} is outside 1..=600; re-run with 1..=600"
        )));
    }
    let back = at.saturating_add(len);
    if back > 600 || back <= at {
        return Err(bad_args(format!(
            "burst {at} plus {len} overruns 600; re-run with at + len <= 600"
        )));
    }
    let profile = engine_net::link::LinkProfile {
        latency: latency_ticks,
        jitter: 0,
        loss_pct,
        reorder_pct: 0,
    };
    let worst = profile.worst_delay();
    let mut cfg = engine_net::played::BoutConfig::canonical(seed, worst);
    cfg.drop = engine_net::played::DropScript { seat: 1, at, back };
    let report = engine_net::played::run_loopback(&cfg, profile, profile).map_err(|error| {
        McpToolError::new(
            "bout_failed",
            format!("loopback bout failed ({error}); re-run with loss_pct 0..=100"),
        )
    })?;
    let converged = report
        .seats
        .iter()
        .all(|seat| seat.first_divergence.is_none());
    let first_divergence = report
        .seats
        .iter()
        .filter_map(|seat| seat.first_divergence)
        .min();
    // Fold the loopback probe constant: the retired local conditioner ran
    // one zero-loss packet through `pop_due(0)`, which always delivered
    // exactly one packet, so the digest carries `+ 1` and no sim work.
    let digest = report.digest().wrapping_add(1);
    Ok(LoopbackReport {
        converged,
        first_divergence,
        digest,
    })
}

/// `net.loopback` request: loss percent plus seed plus latency and burst ticks, integers only.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct NetLoopbackArgs {
    pub loss_pct: u32,
    pub seed: u64,
    pub latency_ticks: u32,
    #[serde(default)]
    pub burst_at: Option<u32>,
    #[serde(default)]
    pub burst_len_ticks: Option<u32>,
}

/// `net.loopback` handler: the pure report under the audited dispatch seam.
fn net_loopback(
    service: &McpServer,
    args: &NetLoopbackArgs,
) -> Result<LoopbackReport, McpToolError> {
    service.dispatch(TOOL_NET_LOOPBACK, |_state| {
        loopback_report(
            args.loss_pct,
            args.seed,
            args.latency_ticks,
            args.burst_at,
            args.burst_len_ticks,
        )
    })
}

crate::e2_tool!(
    /// `net.loopback` tool: loopback bout with injected loss, latency and burst, read-only.
    NetLoopbackTool,
    name = TOOL_NET_LOOPBACK,
    description = "Open a loopback bout with injected loss, latency_ticks delay and burst drop and report convergence plus the first divergent tick as typed integers. Read-only.",
    readonly = true,
    args = NetLoopbackArgs,
    output = LoopbackReport,
    invoke = net_loopback
);

// ---------------------------------------------------------------------------
// Seeded link shaper: the same duel over delay, jitter and loss.
// ---------------------------------------------------------------------------

/// `net.shape` answer: the applied shape, convergence, measured loss, digest.
///
/// `delay_ticks` and `jitter_ticks` are the whole 60 Hz ticks the asked
/// milliseconds became; `loss_pct` is the whole percent the asked permille
/// became. `converged` is true when every seat holds the server truth and the
/// dropped seat stalled, reclaimed and was granted. `first_divergence` is the
/// smallest divergent tick, `None` when all agree. `loss_bp` is the measured
/// random loss in basis points. `digest` folds the simulated outcome only
/// (seed, tape, checkpoint chain), so the same shape twice seals the same
/// digest, and a link that delivers the same input tape seals it too.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct ShapeReport {
    pub delay_ticks: u32,
    pub jitter_ticks: u32,
    pub loss_pct: u32,
    pub converged: bool,
    pub first_divergence: Option<u32>,
    pub loss_bp: u32,
    pub digest: u64,
}

/// Milliseconds to whole 60 Hz ticks, rounded up.
fn ms_to_ticks(ms: u32) -> u32 {
    ms.saturating_mul(60).saturating_add(999) / 1_000
}

/// Run the scripted duel under a seeded link shape and report its verdict.
///
/// `delay_ms` is the base one-way delay, `jitter_ms` the extra delay drawn
/// from `0..=jitter`, `loss_permille` the datagram loss. Delay and jitter
/// become ticks at 60 Hz (rounded up), loss becomes whole percent (rounded
/// up). The server sizes its lag window for the slow link plus one tick, so
/// the third redundancy copy is still admitted, and the scripted outage ends
/// one one-way delay early, so the delayed rejoin request reaches the server
/// on the clean run's rejoin step. Integer only, seeded, no clock, no float,
/// no socket. Anything outside the ranges refuses with `bad_args`; a bout
/// that refuses maps to `bout_failed`, never a panic.
pub fn shape_report(
    delay_ms: u32,
    jitter_ms: u32,
    loss_permille: u32,
    seed: u64,
) -> Result<ShapeReport, McpToolError> {
    if delay_ms > 2_000 {
        return Err(bad_args(format!(
            "delay_ms {delay_ms} is outside 0..=2000; re-run with 0..=2000"
        )));
    }
    if jitter_ms > 1_000 {
        return Err(bad_args(format!(
            "jitter_ms {jitter_ms} is outside 0..=1000; re-run with 0..=1000"
        )));
    }
    if loss_permille > 1_000 {
        return Err(bad_args(format!(
            "loss_permille {loss_permille} is outside 0..=1000; re-run with 0..=1000"
        )));
    }
    let delay_ticks = ms_to_ticks(delay_ms);
    let jitter_ticks = ms_to_ticks(jitter_ms);
    let loss_pct = loss_permille.div_ceil(10);
    let profile = engine_net::link::LinkProfile {
        latency: delay_ticks,
        jitter: jitter_ticks,
        loss_pct,
        reorder_pct: 0,
    };
    let worst = profile.worst_delay();
    let mut cfg = engine_net::played::BoutConfig::for_profile(seed, profile);
    cfg.hold += 1;
    let back = cfg.drop.back.saturating_sub(worst);
    if back > cfg.drop.at {
        cfg.drop.back = back;
    }
    let report = engine_net::played::run_loopback(&cfg, profile, profile).map_err(|error| {
        McpToolError::new(
            "bout_failed",
            format!(
                "shaped bout failed ({error}); re-run with delay_ms 0..=2000, \
                 jitter_ms 0..=1000 and loss_permille 0..=1000"
            ),
        )
    })?;
    let first_divergence = report
        .seats
        .iter()
        .filter_map(|seat| seat.first_divergence)
        .min();
    let (up, _) = report.totals();
    // Sim-only fold: seed, tape and checkpoint chain. Wire counters stay out,
    // so a shaped link that delivers the same inputs folds the clean digest.
    let mut h = Fnv64::new();
    h.write_u64(report.config.seed);
    h.write_u32(report.config.ticks);
    h.write_u64(report.tape_hash);
    h.write_u64(u64::try_from(report.tape_len).unwrap_or(u64::MAX));
    h.write_bool(report.replay_equal);
    h.write_u64(report.server_tip.hash);
    for checkpoint in &report.checkpoints {
        h.write_u64(checkpoint.hash);
    }
    Ok(ShapeReport {
        delay_ticks,
        jitter_ticks,
        loss_pct,
        converged: report.converged(),
        first_divergence,
        loss_bp: up.loss_bp(),
        digest: h.finish(),
    })
}

/// `net.shape` request: delay and jitter in ms, loss in permille, seed.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct NetShapeArgs {
    pub delay_ms: u32,
    pub jitter_ms: u32,
    pub loss_permille: u32,
    pub seed: u64,
}

/// `net.shape` handler: the pure report under the audited dispatch seam.
fn net_shape(service: &McpServer, args: &NetShapeArgs) -> Result<ShapeReport, McpToolError> {
    service.dispatch(TOOL_NET_SHAPE, |_state| {
        shape_report(args.delay_ms, args.jitter_ms, args.loss_permille, args.seed)
    })
}

crate::e2_tool!(
    /// `net.shape` tool: seeded link shaper over the scripted duel, read-only.
    NetShapeTool,
    name = TOOL_NET_SHAPE,
    description = "Replay the scripted duel under a seeded link shape (delay_ms, jitter_ms, loss_permille) and report the applied ticks and percent, convergence, measured loss_bp and a sim-only digest. Read-only.",
    readonly = true,
    args = NetShapeArgs,
    output = ShapeReport,
    invoke = net_shape
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::insight::mcp_tools::{ApiDescribeArgs, ApiDescribeTool};
    use crate::packs::PackRegistry;
    use engine_editor::capability::Capability;
    use rmcp::handler::server::router::tool::SyncTool;

    fn server() -> McpServer {
        McpServer::new(7, Capability::unrestricted(7))
    }

    #[test]
    fn net_loopback_10pct_loss_converges_with_no_divergence() {
        let first = loopback_report(10, 2040, 0, None, None).expect("10pct loss runs");
        assert!(first.converged, "10pct loss converges: {first:?}");
        assert_eq!(first.first_divergence, None);
        let second = loopback_report(10, 2040, 0, None, None).expect("repeats");
        assert_eq!(first, second, "same inputs give the same report twice");
        assert_eq!(first.digest, second.digest);
        let verb = PackRegistry::shipped()
            .verb(TOOL_NET_LOOPBACK)
            .expect("net.loopback is a registered verb");
        assert_eq!(verb.pack(), PACK);
        assert_eq!(verb.card(), CapabilityCard::Observe);
        assert!(verb.readonly());
        let described = ApiDescribeTool::invoke(
            &server(),
            ApiDescribeArgs {
                path: TOOL_NET_LOOPBACK.to_owned(),
            },
        )
        .expect("api.describe lists net.loopback");
        assert_eq!(described.path, TOOL_NET_LOOPBACK);
        assert!(described.docs.contains("loss"));
        assert!(described.signature.contains("net"));
    }

    #[test]
    fn net_loopback_seeded_desync_names_its_tick() {
        let first = loopback_report(100, 11, 0, None, None).expect("full loss runs");
        assert_eq!(first.first_divergence, Some(0));
        let again = loopback_report(100, 11, 0, None, None).expect("repeats");
        assert_eq!(first, again, "same seed reproduces its tick");
        let other = loopback_report(100, 12, 0, None, None).expect("other seed runs");
        assert_eq!(other.first_divergence, Some(0));
        let other_again = loopback_report(100, 12, 0, None, None).expect("other repeats");
        assert_eq!(other, other_again, "other seed reproduces its tick");
    }

    #[test]
    fn net_loopback_refuses_loss_over_100() {
        let error = loopback_report(101, 2040, 0, None, None).expect_err("loss 101 refuses");
        assert_eq!(error.code, "bad_args");
        assert!(error.message.contains("0..=100"));
        assert!(error.message.contains("re-run"));
    }

    #[test]
    fn net_loopback_latency_knob_is_typed_and_stable() {
        assert_eq!(NetLoopbackArgs::default().latency_ticks, 0);
        let args = NetLoopbackArgs {
            loss_pct: 0,
            seed: 2040,
            latency_ticks: 8,
            burst_at: None,
            burst_len_ticks: None,
        };
        let first = loopback_report(
            args.loss_pct,
            args.seed,
            args.latency_ticks,
            args.burst_at,
            args.burst_len_ticks,
        )
        .expect("latency 8 runs");
        assert!(
            first.converged,
            "0 loss plus latency 8 converges: {first:?}"
        );
        assert_eq!(first.first_divergence, None);
        let second = loopback_report(
            args.loss_pct,
            args.seed,
            args.latency_ticks,
            args.burst_at,
            args.burst_len_ticks,
        )
        .expect("repeats");
        assert_eq!(first, second, "same latency seals the same report twice");
        let error = loopback_report(0, 2040, 33, None, None).expect_err("latency 33 refuses");
        assert_eq!(error.code, "bad_args");
        assert!(error.message.contains("0..=32"));
        assert!(error.message.contains("re-run"));
    }

    #[test]
    fn net_loopback_burst_knobs_pin_the_seeded_tick() {
        // Burst drop is the scripted outage (`DropScript` seat 1, `at`,
        // `back = at + len`); `None` means the canonical 60..180.
        let canonical = loopback_report(10, 2040, 0, None, None).expect("canonical runs");
        assert!(canonical.converged, "10pct loss converges: {canonical:?}");
        assert_eq!(canonical.first_divergence, None);
        let explicit =
            loopback_report(10, 2040, 0, Some(60), Some(120)).expect("explicit canonical runs");
        assert_eq!(canonical, explicit, "None means 60 plus 120");
        let moved = loopback_report(10, 2040, 0, Some(100), Some(60)).expect("moved burst runs");
        assert_ne!(
            canonical.digest, moved.digest,
            "moved burst seals another digest"
        );
        let moved_again = loopback_report(10, 2040, 0, Some(100), Some(60)).expect("repeats");
        assert_eq!(moved, moved_again, "same burst replays its tick twice");
        for (at, len) in [
            (Some(0), Some(120)),
            (Some(601), Some(120)),
            (Some(60), Some(0)),
        ] {
            let error = loopback_report(10, 2040, 0, at, len).expect_err("bad burst refuses");
            assert_eq!(error.code, "bad_args");
            assert!(error.message.contains("re-run"));
        }
        let over = loopback_report(10, 2040, 0, Some(590), Some(20)).expect_err("overrun refuses");
        assert_eq!(over.code, "bad_args");
    }

    #[test]
    fn net_loopback_latency_plus_burst_pins_the_seeded_tick() {
        // DP1 remainder: latency and burst knobs together seal one digest.
        let lat_only = loopback_report(10, 2040, 8, None, None).expect("latency runs");
        let lat_burst =
            loopback_report(10, 2040, 8, Some(100), Some(60)).expect("latency plus burst runs");
        assert_ne!(
            lat_only.digest, lat_burst.digest,
            "burst moves the latency-sealed digest"
        );
        let again = loopback_report(10, 2040, 8, Some(100), Some(60)).expect("repeats");
        assert_eq!(lat_burst, again, "same latency plus burst replays twice");
        let desync = loopback_report(100, 11, 8, Some(100), Some(60)).expect("seeded desync runs");
        assert_eq!(desync.first_divergence, Some(0));
        let desync_again = loopback_report(100, 11, 8, Some(100), Some(60)).expect("repeats");
        assert_eq!(desync, desync_again, "seeded tick replays twice");
    }

    #[test]
    fn net_loopback_partial_loss_desync_pins_the_exact_tick() {
        // DP1 remainder: a partial loss desync names an exact, non-zero tick
        // per seed (full loss `Some(0)` stays the boundary case above). At 90
        // percent loss the seeded wire starves a seat a few ticks in, so the
        // tick is non-trivial and reproduces on the same seed twice.
        for (seed, tick) in [(11_u64, 94_u32), (12, 17)] {
            let first = loopback_report(90, seed, 0, None, None).expect("90pct loss runs");
            assert_eq!(
                first.first_divergence,
                Some(tick),
                "seed {seed} names its exact tick: {first:?}"
            );
            assert!(
                !first.converged,
                "a desync never reads converged: {first:?}"
            );
            let again = loopback_report(90, seed, 0, None, None).expect("repeats");
            assert_eq!(first, again, "seed {seed} reproduces its tick twice");
        }
    }

    #[test]
    fn net_shape_120ms_5pct_converges_and_seals_the_same_digest_twice() {
        let first = shape_report(120, 0, 50, 2040).expect("120 ms plus 5 pct runs");
        assert_eq!(first.delay_ticks, 8, "120 ms is 8 ticks at 60 Hz");
        assert_eq!(first.jitter_ticks, 0);
        assert_eq!(first.loss_pct, 5, "50 permille is 5 whole percent");
        assert!(first.converged, "the shaped duel converges: {first:?}");
        assert_eq!(first.first_divergence, None);
        assert!(
            (1..1_000).contains(&first.loss_bp),
            "the shaped link really lost packets: {} bp",
            first.loss_bp
        );
        let second = shape_report(120, 0, 50, 2040).expect("repeats");
        assert_eq!(first, second, "same shape seals the same report twice");
        // A clean link is a different wire, not a different sim: it converges
        // with no loss, and the shaped digest is stable on its own.
        let clean = shape_report(0, 0, 0, 2040).expect("the clean link runs");
        assert!(clean.converged, "the clean duel converges: {clean:?}");
        assert_eq!(clean.delay_ticks, 0);
        assert_eq!(clean.loss_bp, 0);
        let verb = PackRegistry::shipped()
            .verb(TOOL_NET_SHAPE)
            .expect("net.shape is a registered verb");
        assert_eq!(verb.pack(), PACK);
        assert_eq!(verb.card(), CapabilityCard::Observe);
        assert!(verb.readonly());
        let described = ApiDescribeTool::invoke(
            &server(),
            ApiDescribeArgs {
                path: TOOL_NET_SHAPE.to_owned(),
            },
        )
        .expect("api.describe lists net.shape");
        assert_eq!(described.path, TOOL_NET_SHAPE);
        assert!(described.docs.contains("delay"));
        assert!(described.signature.contains(TOOL_NET_SHAPE));
    }

    #[test]
    fn net_shape_zero_jitter_is_pure_delay() {
        let pure = shape_report(120, 0, 0, 2040).expect("pure delay runs");
        assert_eq!(pure.delay_ticks, 8);
        assert_eq!(pure.jitter_ticks, 0);
        assert_eq!(pure.loss_pct, 0);
        assert_eq!(pure.loss_bp, 0, "zero loss drops no live datagram");
        let jittered = shape_report(120, 20, 0, 2040).expect("jitter runs");
        assert_eq!(jittered.delay_ticks, 8);
        assert!(jittered.jitter_ticks > 0);
    }

    #[test]
    fn net_shape_refuses_out_of_range_knobs() {
        for (delay, jitter, loss, wanted) in [
            (2_001, 0, 0, "0..=2000"),
            (0, 1_001, 0, "0..=1000"),
            (0, 0, 1_001, "0..=1000"),
        ] {
            let error = shape_report(delay, jitter, loss, 2040).expect_err("out of range refuses");
            assert_eq!(error.code, "bad_args");
            assert!(error.message.contains(wanted), "{}", error.message);
            assert!(error.message.contains("re-run"));
        }
    }
}
