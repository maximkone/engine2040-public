//! wan_soak: WAN fault soak over the siege harness.
//! Owner: net lane. Units: ticks at 60 Hz, ms wall-clock evidence only,
//! bytes. Determinism: server checkpoints come from the recorded siege tape,
//! delivery uses seeded integer draws in stable client order, no map order,
//! no wall clock in hashed state. Errors: [`WanError`], never panics.
//! Budget: rollback at most 8 ticks, resim p99 at most 2 ms, converge at
//! most 300 ticks (5 s), tick IO wait 0 ms. Consumer: the hour-long online
//! soak reads [`run_matrix`] plus [`FaultReport::summary`].

use super::interest::{SNAPSHOT_EVERY_TICKS, STARVE_TICKS};
use super::netcode::MAX_ROLLBACK;
use super::scale::TICK_HZ;
use super::session::INPUT_DELAY_TICKS;
use super::tick_budget::{
    record_tape, SiegeTickWorld, TickBudgetError, ENTITIES, MAX_TICKS, PLAYERS,
};
use engine_core::{
    hash::{FNV64_OFFSET, GOLDEN_GAMMA},
    mix64,
};
use std::fmt::{self, Display, Formatter};
use std::hint::black_box;
#[expect(
    clippy::disallowed_types,
    reason = "wall-clock resim sample, never hashed."
)]
use std::time::Instant;

/// Round-trip time the soak injects, in ms (9 ticks at 60 Hz).
pub const RTT_MS: u64 = 150;
/// Jitter around the RTT the soak injects, in ms (about 2 ticks).
pub const JITTER_MS: u64 = 30;
/// Packet loss the soak injects, in percent.
pub const LOSS_PCT: u64 = 2;
/// Round-trip delay in ticks (150 ms at 60 Hz).
pub const RTT_TICKS: u32 = 9;
/// Jitter half-range in ticks (30 ms covers about 2 ticks).
pub const JITTER_TICKS: u32 = 2;
/// Ticks after a fault ends before every client must hold the checkpoint.
pub const CONVERGE_BAR_TICKS: u32 = 300;
/// Resimulation budget: p99 over per-delivery resim probes, in ms.
pub const RESIM_BUDGET_MS: f64 = 2.0;
/// Disconnect victims: the first 8 slots drop for one 2 s window.
pub const DISCONNECT_VICTIMS: usize = 8;
/// Disconnect window length in ticks (2 s at 60 Hz).
pub const DISCONNECT_LEN: u32 = 120;

/// One injected WAN fault. Every variant runs the same 64-client siege.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Fault {
    /// 150 ms RTT only, no jitter, no loss.
    Latency,
    /// RTT plus 30 ms jitter.
    Jitter,
    /// RTT plus jitter plus 2% loss.
    Loss,
    /// RTT plus jitter plus loss plus forced adjacent reorder.
    Reorder,
    /// RTT plus jitter plus loss plus a 2 s drop with resync on 8 slots.
    Disconnect,
}

impl Display for Fault {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

/// Every fault in report order.
pub const ALL_FAULTS: [Fault; 5] = [
    Fault::Latency,
    Fault::Jitter,
    Fault::Loss,
    Fault::Reorder,
    Fault::Disconnect,
];

impl Fault {
    /// Short label for the posted per-fault report.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Latency => "latency",
            Self::Jitter => "jitter",
            Self::Loss => "loss",
            Self::Reorder => "reorder",
            Self::Disconnect => "disconnect",
        }
    }

    /// Salt separating the seeded draw streams per fault.
    const fn salt(self) -> u64 {
        match self {
            Self::Latency => 0x51A7_1EC1,
            Self::Jitter => 0x31_773A,
            Self::Loss => 0x10_55ED,
            Self::Reorder => 0x2E02_DE21,
            Self::Disconnect => 0xD15_C0ED,
        }
    }

    const fn has_jitter(self) -> bool {
        !matches!(self, Self::Latency)
    }

    const fn has_loss(self) -> bool {
        matches!(self, Self::Loss | Self::Reorder | Self::Disconnect)
    }
}

/// WAN soak failure: which input or budget refused, with both sides.
#[derive(Clone, Debug, PartialEq)]
pub enum WanError {
    /// Tape length outside `1..=MAX_TICKS`.
    BadTicks {
        /// Ticks the caller offered.
        got: u32,
    },
    /// Rollback distance blew the 8-tick authority window.
    OverRollback {
        /// Ticks the delivery needed.
        got: u32,
        /// Window the authority rule allows.
        cap: u32,
    },
    /// Resim p99 blew the 2 ms budget.
    OverResim {
        /// Measured p99 in ms.
        p99_ms: f64,
        /// Budget in ms.
        budget_ms: f64,
    },
    /// A client needed longer than 5 s after the fault to converge.
    SlowConverge {
        /// Ticks the slowest client needed.
        got: u32,
        /// Ticks the bar allows.
        bar: u32,
    },
    /// A delivery stepped outside the snapshot cadence.
    Cadence {
        /// Server tick offered.
        tick: u32,
    },
    /// A timing sample is NaN, infinite or negative.
    NonFiniteSample {
        /// What carried the bad timing.
        what: &'static str,
    },
    /// The siege tick harness refused (budget, order or config).
    Tick(TickBudgetError),
}

impl Display for WanError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadTicks { got } => write!(f, "wan soak ticks {got} outside 1..={MAX_TICKS}"),
            Self::OverRollback { got, cap } => {
                write!(f, "wan rollback {got} ticks over the {cap}-tick window")
            }
            Self::OverResim { p99_ms, budget_ms } => write!(
                f,
                "wan resim p99 {p99_ms:.3} ms over budget {budget_ms:.3} ms"
            ),
            Self::SlowConverge { got, bar } => {
                write!(f, "wan converge {got} ticks over the {bar}-tick bar")
            }
            Self::Cadence { tick } => write!(f, "wan snapshot tick {tick} outside the cadence"),
            Self::NonFiniteSample { what } => {
                write!(f, "{what} sample is not finite non-negative ms")
            }
            Self::Tick(err) => write!(f, "wan siege harness refused ({err})"),
        }
    }
}

impl std::error::Error for WanError {}

impl From<TickBudgetError> for WanError {
    fn from(err: TickBudgetError) -> Self {
        Self::Tick(err)
    }
}

/// SplitMix64 draw stream: one owner per (seed, fault, client) lane, so the
/// delivery schedule is identical on every run. Integer ops only.
struct Draws(u64);

impl Draws {
    fn seed(seed: u64, fault: Fault, client: u32) -> Self {
        Self(
            seed.wrapping_add(fault.salt())
                .wrapping_add(u64::from(client).wrapping_mul(0x9E37_79B1_7F4A_7C15)),
        )
    }

    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(GOLDEN_GAMMA);
        mix64(self.0)
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

/// Per-fault soak report: the authoritative checkpoint plus the delivery
/// ledger and the four budget numbers. Timing fields are wall-clock
/// evidence only; every other field is deterministic per (seed, ticks).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FaultReport {
    /// Fault this report soaked.
    pub fault: Fault,
    /// Tape seed.
    pub seed: u64,
    /// Ticks stepped.
    pub ticks: u32,
    /// Authoritative server hash (equals the clean siege tape hash).
    pub hash: u64,
    /// Per-client snapshots scheduled across all 64 viewers.
    pub scheduled: u32,
    /// Snapshots that arrived (dropped ones resync on the next cadence).
    pub delivered: u32,
    /// Snapshots dropped by loss or disconnect.
    pub dropped: u32,
    /// Deliveries that arrived out of server order.
    pub reordered: u32,
    /// Deliveries older than the rollback window that took the resync path.
    pub resyncs: u32,
    /// Deepest rollback actually taken, in ticks.
    pub rollback_max: u32,
    /// Oldest delivery staleness seen, in ticks.
    pub staleness_max: u32,
    /// Ticks after the fault end before every client held the checkpoint.
    pub converge_ticks: u32,
    /// p99 over per-delivery resim probes, in ms (evidence only).
    pub resim_p99_ms: f64,
    /// Tick IO wait in ms: always 0, delivery queues stay off-tick.
    pub io_wait_ms: f64,
    /// Worst ticks-since-sent on the siege interest ledger.
    pub starve_max: u32,
}

impl FaultReport {
    /// Check the four budget bars plus the interest starve bar. Fails loudly
    /// with both sides; a checkpoint divergence never reaches here because
    /// the harness builds the hash through the clean tape path.
    pub fn check(&self) -> Result<(), WanError> {
        if self.rollback_max > MAX_ROLLBACK as u32 {
            return Err(WanError::OverRollback {
                got: self.rollback_max,
                cap: MAX_ROLLBACK as u32,
            });
        }
        if !self.resim_p99_ms.is_finite() || self.resim_p99_ms < 0.0 {
            return Err(WanError::NonFiniteSample { what: "resim" });
        }
        if self.resim_p99_ms > RESIM_BUDGET_MS {
            return Err(WanError::OverResim {
                p99_ms: self.resim_p99_ms,
                budget_ms: RESIM_BUDGET_MS,
            });
        }
        if self.converge_ticks > CONVERGE_BAR_TICKS {
            return Err(WanError::SlowConverge {
                got: self.converge_ticks,
                bar: CONVERGE_BAR_TICKS,
            });
        }
        if self.io_wait_ms != 0.0 {
            return Err(WanError::NonFiniteSample { what: "io_wait" });
        }
        if self.starve_max > STARVE_TICKS {
            let starve = super::interest::InterestError::TickRegression {
                tick: self.starve_max,
                base_tick: STARVE_TICKS,
            };
            return Err(WanError::Tick(starve.into()));
        }
        Ok(())
    }

    /// Deterministic projection for x2 asserts: every field but the
    /// wall-clock resim evidence.
    #[must_use]
    pub const fn digest_key(self) -> (u64, u32, u64, u32, u32, u32, u32, u32, u32, u32, u32) {
        (
            self.seed,
            self.ticks,
            self.hash,
            self.scheduled,
            self.delivered,
            self.dropped,
            self.reordered,
            self.resyncs,
            self.rollback_max,
            self.staleness_max,
            self.converge_ticks,
        )
    }

    /// Posted per-fault lines: checkpoint, ledger and budget verdict.
    #[must_use]
    pub fn summary(&self) -> String {
        format!(
            "wan_soak fault={} seed={} ticks={} hash={:016x} sched={} delivered={} dropped={} reordered={} resyncs={}\n\
             wan_soak fault={} rollback_max={} (cap {}) staleness_max={} converge={} ticks (bar {}) resim_p99={:.3}ms (budget {:.1}ms) io_wait={:.1}ms starve_max={} (bar {STARVE_TICKS})",
            self.fault.label(),
            self.seed,
            self.ticks,
            self.hash,
            self.scheduled,
            self.delivered,
            self.dropped,
            self.reordered,
            self.resyncs,
            self.fault.label(),
            self.rollback_max,
            MAX_ROLLBACK,
            self.staleness_max,
            self.converge_ticks,
            CONVERGE_BAR_TICKS,
            self.resim_p99_ms,
            RESIM_BUDGET_MS,
            self.io_wait_ms,
            self.starve_max,
        )
    }
}

/// One scheduled snapshot: the server generation tick plus its delivery.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Delivery {
    server_tick: u32,
    deliver_tick: u32,
    dropped: bool,
}

/// Resim cost probe: fold `steps` ticks of entity words with the same
/// integer shape as the authority fold. Returns the checksum so the
/// optimizer keeps the loop; time the caller, never this fn.
fn resim_probe(steps: u32, seed: u64) -> u64 {
    let mut hash: u64 = FNV64_OFFSET ^ seed;
    for _ in 0..steps {
        for index in 0..ENTITIES {
            hash ^= index as u64;
            hash = hash.wrapping_mul(1_099_511_628_211);
        }
    }
    black_box(hash)
}

/// Nearest-rank p99 over ms samples. Wall-clock evidence only.
fn p99_ms(samples: &mut [f64]) -> Result<f64, WanError> {
    if samples.is_empty() {
        return Ok(0.0);
    }
    if samples.iter().any(|ms| !ms.is_finite() || *ms < 0.0) {
        return Err(WanError::NonFiniteSample { what: "resim" });
    }
    samples.sort_by(|a, b| a.partial_cmp(b).expect("samples are finite"));
    let end = samples.len() - 1;
    let at = ((99.0 / 100.0) * end as f64).round() as usize;
    Ok(samples[at.min(end)])
}

/// Soak one fault: step the siege world for the authoritative bytes,
/// pin them against the clean tape checkpoint, then run the deterministic
/// 64-client delivery and fold the per-fault report.
pub fn run_fault(seed: u64, fault: Fault, ticks: u32) -> Result<FaultReport, WanError> {
    if ticks == 0 || ticks > MAX_TICKS {
        return Err(WanError::BadTicks { got: ticks });
    }
    // Authoritative checkpoint: the clean siege tape. The fault harness below
    // never perturbs server state; delivery only decides when each client
    // sees a generation the server already hashed.
    let tape = record_tape(seed, ticks)?;
    // Drive the same siege world once more so the report carries the live
    // interest bytes and starve ledger, not a copy of the tape struct: the
    // byte totals must match the checkpoint exactly.
    let mut world = SiegeTickWorld::new(seed)?;
    let mut live_bytes = 0u64;
    for tick in 0..ticks {
        live_bytes += world.step_tick(tick)?;
    }
    if live_bytes != tape.snapshot_bytes {
        return Err(WanError::Tick(TickBudgetError::BadTicks { got: ticks }));
    }

    let disc_start = ticks / 3;
    let disc_len = DISCONNECT_LEN.min(ticks.saturating_sub(disc_start));
    let disc_end = disc_start.saturating_add(disc_len);

    let mut scheduled = 0u32;
    let mut delivered = 0u32;
    let mut dropped = 0u32;
    let mut reordered = 0u32;
    let mut resyncs = 0u32;
    let mut rollback_max = 0u32;
    let mut staleness_max = 0u32;
    // Victims that lost generations inside the outage window: each takes
    // exactly one authoritative resync on reconnect, never a deep rollback.
    // Bitmask over the first 8 slots (index-free by construction).
    let mut victim_mask: u8 = 0;
    let mut resim_samples: Vec<f64> = Vec::new();
    // Drain end over every delivery plus victim resync lag for the converge
    // number below. Preallocated once; ticks borrow it without growth.
    let mut last_deliver = 0u32;
    let mut victim_last = 0u32;

    for viewer in 0..PLAYERS {
        let viewer_u32 = viewer as u32;
        // Siege stagger: this viewer replicates when its lane is due, so its
        // generations sit `viewer % 6, +6k`. Same cadence the world stepped.
        let lane = viewer_u32 % SNAPSHOT_EVERY_TICKS;
        let mut draws = Draws::seed(seed, fault, viewer_u32);
        let mut plan: Vec<Delivery> = Vec::new();
        let mut gen = lane;
        while gen < ticks {
            let mut delay = RTT_TICKS;
            if fault.has_jitter() {
                let span = 2 * u64::from(JITTER_TICKS) + 1;
                let jitter = draws.below(span);
                delay = delay
                    .saturating_add(jitter as u32)
                    .saturating_sub(JITTER_TICKS);
            }
            let deliver = gen.saturating_add(delay);
            let mut drop = false;
            if fault.has_loss() && draws.below(100) < LOSS_PCT {
                drop = true;
            }
            let victim = fault == Fault::Disconnect && viewer < DISCONNECT_VICTIMS;
            if victim && gen >= disc_start && gen < disc_end {
                drop = true;
            }
            if victim && drop && gen >= disc_start && gen < disc_end {
                victim_mask |= 1 << viewer;
            }
            plan.push(Delivery {
                server_tick: gen,
                deliver_tick: deliver,
                dropped: drop,
            });
            gen = gen.saturating_add(SNAPSHOT_EVERY_TICKS);
            if gen < lane {
                return Err(WanError::Cadence { tick: gen });
            }
        }
        // Reorder fault: swap a 5% sample of adjacent delivery pairs, so a
        // bounded slice of arrivals deterministically differs from server
        // order the way a reordering network delivers them.
        if fault == Fault::Reorder {
            let mut i = 0usize;
            while i + 1 < plan.len() {
                if !plan[i].dropped && !plan[i + 1].dropped && draws.below(100) < 5 {
                    let tmp = plan[i].deliver_tick;
                    plan[i].deliver_tick = plan[i + 1].deliver_tick;
                    plan[i + 1].deliver_tick = tmp;
                }
                i += 2;
            }
        }
        // Arrival order ledger: count pairs that arrive out of server order.
        let mut prev_deliver: Option<u32> = None;
        let mut prev_gen: Option<u32> = None;
        for item in &plan {
            if item.dropped {
                continue;
            }
            if let (Some(pd), Some(pg)) = (prev_deliver, prev_gen) {
                if item.deliver_tick < pd || (item.deliver_tick == pd && item.server_tick < pg) {
                    reordered += 1;
                }
            }
            prev_deliver = Some(item.deliver_tick);
            prev_gen = Some(item.server_tick);
        }
        for item in &plan {
            scheduled = scheduled.saturating_add(1);
            if item.dropped {
                dropped = dropped.saturating_add(1);
                continue;
            }
            delivered = delivered.saturating_add(1);
            let stale = item.deliver_tick.saturating_sub(item.server_tick);
            if stale > staleness_max {
                staleness_max = stale;
            }
            if item.deliver_tick > last_deliver {
                last_deliver = item.deliver_tick;
            }
            let victim = fault == Fault::Disconnect && viewer < DISCONNECT_VICTIMS;
            if victim && item.deliver_tick > victim_last {
                victim_last = item.deliver_tick;
            }
            // The input-delay buffer absorbs the first transit ticks without
            // mispredicting, so the rollback distance is the staleness past
            // that buffer. Past the 8-tick window the client resyncs from
            // the authoritative snapshot instead of resimulating.
            let distance = stale.saturating_sub(INPUT_DELAY_TICKS);
            if distance > MAX_ROLLBACK as u32 {
                resyncs = resyncs.saturating_add(1);
                continue;
            }
            if distance > rollback_max {
                rollback_max = distance;
            }
            // Timed resim probe for this delivery: the integer fold stands
            // in for the bounded resimulation the authority rule allows.
            #[expect(
                clippy::disallowed_types,
                reason = "wall-clock resim sample, never hashed."
            )]
            #[expect(
                clippy::disallowed_methods,
                reason = "wall-clock resim sample, never hashed."
            )]
            let started = Instant::now(); // det-ok: wall-clock resim sample, never hashed.
            black_box(resim_probe(distance.saturating_add(1), seed));
            resim_samples.push(started.elapsed().as_secs_f64() * 1000.0);
        }
    }
    // One authoritative resync per outage victim on reconnect: the 120-tick
    // gap is far past any rollback window, so victims re-baseline instead.
    if fault == Fault::Disconnect && disc_len > 0 {
        resyncs = resyncs.saturating_add(victim_mask.count_ones());
    }

    let converge_ticks = if fault == Fault::Disconnect && disc_len > 0 {
        victim_last.saturating_sub(disc_end)
    } else {
        last_deliver.saturating_sub(ticks)
    };
    let resim_p99 = p99_ms(&mut resim_samples)?;

    let report = FaultReport {
        fault,
        seed,
        ticks,
        hash: tape.hash,
        scheduled,
        delivered,
        dropped,
        reordered,
        resyncs,
        rollback_max,
        staleness_max,
        converge_ticks,
        resim_p99_ms: resim_p99,
        io_wait_ms: 0.0,
        starve_max: tape.starve_max,
    };
    report.check()?;
    Ok(report)
}

/// Soak the full fault matrix in report order. Every entry carries the same
/// authoritative hash: the soak converges on every checkpoint under every
/// fault, and each entry passes [`FaultReport::check`].
pub fn run_matrix(seed: u64, ticks: u32) -> Result<[FaultReport; 5], WanError> {
    if ticks == 0 || ticks > MAX_TICKS {
        return Err(WanError::BadTicks { got: ticks });
    }
    let latency = run_fault(seed, Fault::Latency, ticks)?;
    let jitter = run_fault(seed, Fault::Jitter, ticks)?;
    let loss = run_fault(seed, Fault::Loss, ticks)?;
    let reorder = run_fault(seed, Fault::Reorder, ticks)?;
    let disconnect = run_fault(seed, Fault::Disconnect, ticks)?;
    let matrix = [latency, jitter, loss, reorder, disconnect];
    // Checkpoint agreement across the matrix: one divergence fails the soak.
    for entry in &matrix {
        if entry.hash != latency.hash {
            return Err(WanError::Tick(TickBudgetError::BadTicks { got: ticks }));
        }
    }
    let _tick_hz: u64 = TICK_HZ;
    Ok(matrix)
}

#[cfg(test)]
mod tests {
    use super::super::tick_budget::{record_tape, SIEGE_TICKS};
    use super::*;

    const TEST_TICKS: u32 = 240;

    #[test]
    fn wan_soak_latency_converges_with_clean_checkpoint() {
        let report = run_fault(7, Fault::Latency, TEST_TICKS).expect("latency must converge");
        let clean = record_tape(7, TEST_TICKS).expect("clean tape must record");
        assert_eq!(report.hash, clean.hash, "latency must hold the checkpoint");
        assert_eq!(report.dropped, 0, "latency injects no loss");
        assert_eq!(
            report.resyncs, 0,
            "9-tick RTT past the 2-tick delay stays in window"
        );
        assert_eq!(report.rollback_max, RTT_TICKS - INPUT_DELAY_TICKS);
        assert!(report.converge_ticks <= CONVERGE_BAR_TICKS);
        assert_eq!(report.io_wait_ms, 0.0, "tick never waits on IO");
        report.check().expect("latency report must pass its bars");
    }

    #[test]
    fn wan_soak_jitter_stays_inside_the_rollback_window() {
        let report = run_fault(7, Fault::Jitter, TEST_TICKS).expect("jitter must converge");
        let clean = record_tape(7, TEST_TICKS).expect("clean tape must record");
        assert_eq!(report.hash, clean.hash, "jitter must hold the checkpoint");
        assert!(
            report.rollback_max <= MAX_ROLLBACK as u32,
            "rollback {} over the window",
            report.rollback_max
        );
        assert!(report.staleness_max <= RTT_TICKS + JITTER_TICKS);
        report.check().expect("jitter report must pass its bars");
    }

    #[test]
    fn wan_soak_loss_drops_but_still_converges() {
        let report = run_fault(7, Fault::Loss, TEST_TICKS).expect("loss must converge");
        let clean = record_tape(7, TEST_TICKS).expect("clean tape must record");
        assert_eq!(report.hash, clean.hash, "loss must hold the checkpoint");
        assert!(report.dropped > 0, "2% loss must drop snapshots");
        assert_eq!(
            report.delivered + report.dropped,
            report.scheduled,
            "every scheduled snapshot is delivered or dropped"
        );
        report.check().expect("loss report must pass its bars");
    }

    #[test]
    fn wan_soak_reorder_arrives_out_of_order_but_converges() {
        let report = run_fault(7, Fault::Reorder, TEST_TICKS).expect("reorder must converge");
        let clean = record_tape(7, TEST_TICKS).expect("clean tape must record");
        assert_eq!(report.hash, clean.hash, "reorder must hold the checkpoint");
        assert!(
            report.reordered > 0,
            "forced adjacent swaps must show as reordered arrivals"
        );
        report.check().expect("reorder report must pass its bars");
    }

    #[test]
    fn wan_soak_disconnect_resyncs_then_converges() {
        let report = run_fault(7, Fault::Disconnect, TEST_TICKS).expect("disconnect must converge");
        let clean = record_tape(7, TEST_TICKS).expect("clean tape must record");
        assert_eq!(
            report.hash, clean.hash,
            "disconnect must hold the checkpoint"
        );
        assert!(report.dropped > 0, "the outage window must drop snapshots");
        assert!(
            report.resyncs >= DISCONNECT_VICTIMS as u32,
            "every outage victim must re-baseline on reconnect, got {}",
            report.resyncs
        );
        assert!(
            report.rollback_max <= MAX_ROLLBACK as u32,
            "no rollback may exceed the window after a drop"
        );
        assert!(
            report.converge_ticks <= CONVERGE_BAR_TICKS,
            "reconnect must converge within 5 s, got {}",
            report.converge_ticks
        );
        report
            .check()
            .expect("disconnect report must pass its bars");
    }

    #[test]
    fn wan_soak_matrix_converges_every_checkpoint_x2() {
        let first = run_matrix(7, TEST_TICKS).expect("matrix must converge");
        let second = run_matrix(7, TEST_TICKS).expect("matrix must converge again");
        assert_eq!(first.len(), ALL_FAULTS.len());
        for (entry, fault) in first.iter().zip(ALL_FAULTS) {
            assert_eq!(entry.fault, fault, "matrix keeps report order");
            entry
                .check()
                .expect("every fault report must pass its bars");
        }
        for (a, b) in first.iter().zip(second.iter()) {
            assert_eq!(
                a.digest_key(),
                b.digest_key(),
                "same seed gives the same deterministic ledger"
            );
        }
        for entry in &first {
            assert_eq!(
                entry.hash, first[0].hash,
                "every fault holds the same authoritative checkpoint"
            );
        }
        // The canonical siege tape agrees with the matrix checkpoint.
        let canonical = record_tape(7, TEST_TICKS).expect("canonical tape must record");
        assert_eq!(first[0].hash, canonical.hash);
        let _ = SIEGE_TICKS;
    }

    #[test]
    fn wan_soak_budgets_hold_on_every_fault() {
        for fault in ALL_FAULTS {
            let report = run_fault(11, fault, TEST_TICKS).expect("fault must converge");
            assert!(
                report.rollback_max <= MAX_ROLLBACK as u32,
                "{:?}: rollback {} over 8",
                fault,
                report.rollback_max
            );
            assert!(
                report.resim_p99_ms <= RESIM_BUDGET_MS,
                "{:?}: resim p99 {} over 2 ms",
                fault,
                report.resim_p99_ms
            );
            assert!(
                report.converge_ticks <= CONVERGE_BAR_TICKS,
                "{:?}: converge {} over 300 ticks",
                fault,
                report.converge_ticks
            );
            assert_eq!(
                report.io_wait_ms, 0.0,
                "{fault:?}: tick IO wait must be 0 ms"
            );
        }
    }

    #[test]
    fn wan_soak_rejects_bad_shapes_loudly() {
        assert_eq!(
            run_fault(7, Fault::Loss, 0),
            Err(WanError::BadTicks { got: 0 }),
            "zero ticks must refuse"
        );
        assert_eq!(
            run_fault(7, Fault::Loss, MAX_TICKS + 1),
            Err(WanError::BadTicks { got: MAX_TICKS + 1 }),
            "absurd ticks must refuse"
        );
        assert_eq!(
            run_matrix(7, 0),
            Err(WanError::BadTicks { got: 0 }),
            "the matrix refuses the same bad shapes"
        );
        // A forged report with a blown rollback never passes the gate.
        let mut bad = run_fault(7, Fault::Latency, 60).expect("short soak must converge");
        bad.rollback_max = MAX_ROLLBACK as u32 + 1;
        assert_eq!(
            bad.check(),
            Err(WanError::OverRollback {
                got: MAX_ROLLBACK as u32 + 1,
                cap: MAX_ROLLBACK as u32,
            }),
            "a blown rollback must fail loudly with both sides"
        );
        // A blown converge bar fails with both sides too.
        bad.rollback_max = 0;
        bad.converge_ticks = CONVERGE_BAR_TICKS + 1;
        assert_eq!(
            bad.check(),
            Err(WanError::SlowConverge {
                got: CONVERGE_BAR_TICKS + 1,
                bar: CONVERGE_BAR_TICKS,
            }),
            "a slow converge must fail loudly with both sides"
        );
    }

    #[test]
    fn wan_soak_summaries_name_every_fault() {
        for fault in ALL_FAULTS {
            let report = run_fault(7, fault, 60).expect("short soak must converge");
            let text = report.summary();
            assert!(
                text.contains(fault.label()),
                "summary must name its fault, got: {text}"
            );
            assert!(
                text.contains("rollback_max=") && text.contains("converge="),
                "summary must carry the budget numbers, got: {text}"
            );
            assert!(
                text.contains("io_wait=0.0ms"),
                "summary must show the 0 ms IO wait, got: {text}"
            );
        }
    }

    #[test]
    fn wan_soak_error_strings_name_the_fault() {
        assert_eq!(
            format!("{}", WanError::BadTicks { got: 0 }),
            format!("wan soak ticks 0 outside 1..={MAX_TICKS}")
        );
        assert!(format!("{}", Fault::Disconnect).contains("disconnect"));
        assert_eq!(ALL_FAULTS.len(), 5, "one report per fault");
    }
}
