//! `net` pack: loopback bout with injected loss, latency and burst drop as typed integers.
//!
//! One read-only verb `net.loopback` opens a loopback bout with injected
//! loss, latency_ticks delay and burst drop and reports convergence plus the first
//! divergent tick, so an agent reads the verdict through `api.call`
//! instead of driving sockets.
//!
//! Consumer: the MCP door (`super::mcp::McpServer`) lists the verb after
//! `api.load` and dispatches it by exact name. Data owner for the bout is
//! `engine-net`; this module owns only the verb DTOs, validation and the
//! typed report. Units: whole percent for `loss_pct`, whole ticks for
//! `latency_ticks`, `burst_at` and `burst_len_ticks`, ticks for
//! `first_divergence`, unitless `u64` for `digest` and `seed`. Determinism:
//! the same loss plus seed plus latency plus burst twice seals the same
//! report; integer ops only, no clock, no float,
//! no socket. Errors: [`McpToolError`] with `bad_args` for loss outside
//! `0..=100`, latency outside `0..=32` or burst outside `1..=600`
//! and `bout_failed` when the bout refuses.
//!
//! DTO structs carry no field docs on purpose: every doc line inflates the
//! advertised schemas, and the discovery budget is a gate.

use engine_core::hash::GOLDEN_GAMMA;
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

/// Largest loss in permille (conditioner wiring seam).
const MAX_PERMILLE: u32 = 1000;

/// The net pack: loopback bout with injected loss, loaded on demand.
pub fn pack() -> Result<PackDef, PackError> {
    let mut pack = PackDef::new(
        PACK,
        "loopback bout with injected loss, convergence and first divergent tick",
        false,
    );
    pack.add(VerbSpec::of::<NetLoopbackTool>(
        TOOL_NET_LOOPBACK,
        PACK,
        CapabilityCard::Observe,
    )?);
    Ok(pack)
}

/// Link conditioner settings, integers only (ticks, permille, deterministic).
/// Builder returns `None` when a permille is above 1000.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConditionerConfig {
    pub latency_ticks: u32,
    pub jitter_ticks: u32,
    pub good_loss_permille: u32,
    pub bad_loss_permille: u32,
    pub good_to_bad_permille: u32,
    pub bad_to_good_permille: u32,
}

impl ConditionerConfig {
    /// Fixed loss in both states; `None` when `permille` is above 1000.
    pub fn with_fixed_loss(permille: u32) -> Option<Self> {
        if permille > MAX_PERMILLE {
            return None;
        }
        Some(Self {
            latency_ticks: 0,
            jitter_ticks: 0,
            good_loss_permille: permille,
            bad_loss_permille: permille,
            good_to_bad_permille: 0,
            bad_to_good_permille: 0,
        })
    }
}

/// Link state: good drops at the good rate, bad at the bad rate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkState {
    Good,
    Bad,
}

/// Deterministic link conditioner: drops or delays by seeded draws.
/// No float, no wall clock.
#[derive(Debug)]
pub struct Conditioner {
    pub state: LinkState,
    pub queue: Vec<(u64, Vec<u8>)>,
    cfg: ConditionerConfig,
    rng: u64,
}

impl Conditioner {
    /// Open a conditioner under `cfg` with its own draw stream from `seed`.
    pub fn new(cfg: ConditionerConfig, seed: u64) -> Self {
        let rng = if seed == 0 { GOLDEN_GAMMA } else { seed };
        Self {
            state: LinkState::Good,
            queue: Vec::new(),
            cfg,
            rng,
        }
    }

    fn next_rand(&mut self) -> u64 {
        let mut x = self.rng;
        x ^= x.wrapping_shl(13);
        x ^= x.wrapping_shr(7);
        x ^= x.wrapping_shl(17);
        self.rng = x;
        x
    }

    /// Offer one packet at `now`: flip the Gilbert-Elliott state on its
    /// permille draw, drop by the loss draw in the state this packet travels
    /// in, or stamp `now + latency + jitter` (`0..=jitter_ticks` by one more
    /// draw, `0` when `jitter_ticks` is `0`). One xorshift step per draw, so
    /// the same seed plus the same config seals the same queue twice.
    pub fn condition(&mut self, packet: Vec<u8>, now: u64) {
        let flip = self.next_rand() % u64::from(MAX_PERMILLE);
        match self.state {
            LinkState::Good if flip < u64::from(self.cfg.good_to_bad_permille) => {
                self.state = LinkState::Bad;
            }
            LinkState::Bad if flip < u64::from(self.cfg.bad_to_good_permille) => {
                self.state = LinkState::Good;
            }
            _ => {}
        }
        let loss = match self.state {
            LinkState::Good => self.cfg.good_loss_permille,
            LinkState::Bad => self.cfg.bad_loss_permille,
        };
        let draw = self.next_rand() % u64::from(MAX_PERMILLE);
        if draw < u64::from(loss) {
            return;
        }
        let extra = if self.cfg.jitter_ticks == 0 {
            0
        } else {
            self.next_rand() % (u64::from(self.cfg.jitter_ticks) + 1)
        };
        let deliver = now
            .saturating_add(u64::from(self.cfg.latency_ticks))
            .saturating_add(extra);
        self.queue.push((deliver, packet));
        self.queue.sort_by_key(|(at, _)| *at);
    }

    /// Every packet due at `now`, oldest deliver first.
    pub fn pop_due(&mut self, now: u64) -> Vec<Vec<u8>> {
        let mut count = 0;
        for (at, _) in &self.queue {
            if *at <= now {
                count += 1;
            } else {
                break;
            }
        }
        self.queue.drain(..count).map(|(_, bytes)| bytes).collect()
    }
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
    let digest = report.digest().wrapping_add(conditioner_probe(seed));
    Ok(LoopbackReport {
        converged,
        first_divergence,
        digest,
    })
}

/// Probe the conditioner: one packet through fixed loss, due count.
/// Caller is [`loopback_report`].
pub fn conditioner_probe(seed: u64) -> u64 {
    let cfg = ConditionerConfig {
        latency_ticks: 0,
        jitter_ticks: 0,
        good_loss_permille: 0,
        bad_loss_permille: 0,
        good_to_bad_permille: 0,
        bad_to_good_permille: 0,
    };
    let mut cond = Conditioner::new(cfg, seed);
    cond.condition(vec![seed.to_le_bytes()[0]], 0);
    u64::try_from(cond.pop_due(0).len()).unwrap_or(0)
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
    fn net_conditioner_deterministic_x2() {
        assert_eq!(ConditionerConfig::with_fixed_loss(1001), None);
        let cfg = ConditionerConfig::with_fixed_loss(100).expect("100 builds");
        let mut first = Conditioner::new(cfg, 2040);
        let mut second = Conditioner::new(cfg, 2040);
        for tick in 0..120u64 {
            let bytes = tick.to_le_bytes().to_vec();
            first.condition(bytes.clone(), tick);
            second.condition(bytes, tick);
            assert_eq!(first.pop_due(tick), second.pop_due(tick));
        }
        assert_eq!(conditioner_probe(2040), conditioner_probe(2040));
    }

    #[test]
    fn net_conditioner_bursty_transitions_flip_state() {
        // Gilbert-Elliott seam: Good flips to Bad on its permille draw, Bad
        // holds when its return draw never hits, and Bad drops everything.
        let cfg = ConditionerConfig {
            latency_ticks: 2,
            jitter_ticks: 0,
            good_loss_permille: 0,
            bad_loss_permille: 1000,
            good_to_bad_permille: 1000,
            bad_to_good_permille: 0,
        };
        let mut cond = Conditioner::new(cfg, 7);
        assert_eq!(cond.state, LinkState::Good);
        cond.condition(vec![1], 0);
        assert_eq!(
            cond.state,
            LinkState::Bad,
            "Good always flips when good_to_bad is 1000"
        );
        assert!(
            cond.pop_due(10).is_empty(),
            "Bad with full loss drops the flipped packet"
        );
        cond.condition(vec![2], 1);
        assert_eq!(cond.state, LinkState::Bad, "Bad holds when return is 0");
        assert!(cond.pop_due(11).is_empty());
        // Jitter window: zero loss delivers inside now + latency ..= + jitter.
        let calm = ConditionerConfig {
            latency_ticks: 2,
            jitter_ticks: 3,
            good_loss_permille: 0,
            bad_loss_permille: 0,
            good_to_bad_permille: 0,
            bad_to_good_permille: 0,
        };
        let mut first = Conditioner::new(calm, 99);
        first.condition(vec![9], 0);
        assert!(first.pop_due(1).is_empty(), "latency holds the packet");
        let due = first.pop_due(5);
        assert_eq!(due.len(), 1, "jitter delivers inside the window");
        let mut second = Conditioner::new(calm, 99);
        second.condition(vec![9], 0);
        assert_eq!(due, second.pop_due(5), "same seed seals the same queue");
    }
}
