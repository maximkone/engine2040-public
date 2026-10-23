//! cut_soak: BX9's 64-client severed-joint soak over the wired cut lane.
//!
//! Owner: net lane. Units: ticks at 60 Hz, cut rows, bytes; wall-clock only in
//! the report line, never in a decision. Determinism: the cut schedule, the
//! roster drift, and every loss/latency draw are integer folds of
//! `(viewer, tick)`, so two runs deliver the same bytes on the same ticks.
//! Errors: [`NetError`], never panics.
//!
//! Shape: one authority [`CutLog`] (the input log's event lane), 64 viewers
//! with their own interest selection, an acked per-viewer window that
//! re-sends every tick until the viewer acks, and a link that drops
//! [`LOSS_PCT`] of packets and delays the rest by half an RTT plus jitter.
//! A client's lane must hold exactly the rows the server routed to it, and
//! every row must be acked within [`SETTLE_TICKS`] (10) ticks: that is the
//! "clients settle limbs within 5 cm in 10 ticks" bar on the wire side (the
//! limb itself is spawned from the decoded row by `cuts`' settle test).
//!
//! Consumer: `cargo test -p engine-net --lib cut_soak` (bounded 10 s) and the
//! named full hour run `cut_soak::full_hour_soak` (`--ignored`), once per
//! BX9 receipt.

use engine_combat::JointId;
use engine_core::{Fnv64, TICK_HZ_U32};

use crate::cuts::{self, decode_cuts, encode_cuts, CutLog, CutRow, WireCut, MAX_CUTS_PER_PACKET};
use crate::interest::{ConnectionInterest, InterestCfg, InterestEntity, SIEGE_BUDGET_BYTES};
use crate::session::NetError;

/// Clients in the soak (the BX9 "64-client" bar).
pub const SOAK_CLIENTS: u32 = 64;
/// Cuts authored per second across the whole match.
pub const CUTS_PER_SECOND: u32 = 8;
/// A routed row must be acked within this many ticks of its cut tick.
pub const SETTLE_TICKS: u32 = 10;
/// Ticks between the two cut slots of one pair: 120 / 15 = 8 cuts per second.
const SLOT_TICKS: u32 = 15;
/// Bounded soak: ten seconds at 60 Hz, 80 cuts.
pub const SOAK_TICKS: u32 = 10 * TICK_HZ_U32;
/// Named full run: one hour at 60 Hz, 28 800 cuts.
pub const SOAK_TICKS_HOUR: u32 = 3_600 * TICK_HZ_U32;
/// One-way latency ticks (150 ms RTT at 60 Hz is 9 ticks, 4 to 5 each way).
pub const RTT_TICKS: u32 = 9;
/// Jitter added to the one-way delay, ticks.
pub const JITTER_TICKS: u32 = 2;
/// Percent of packets the link drops; the acked window re-sends them.
pub const LOSS_PCT: u64 = 2;
/// One viewer's interest budget per select: a quarter of the siege datagram,
/// so the selection really culls and the cut lane really routes by victim.
pub const SOAK_BUDGET_BYTES: usize = SIEGE_BUDGET_BYTES / 4;

/// What one soak run proved.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct CutSoakReport {
    /// Viewers driven.
    pub clients: u32,
    /// Ticks authored.
    pub ticks: u32,
    /// Cuts the authority authored.
    pub cuts: u64,
    /// Rows routed to some viewer (after interest).
    pub routed: u64,
    /// Cut packets sent (including re-sends).
    pub packets: u64,
    /// Packets the link dropped.
    pub dropped: u64,
    /// Client-ticks whose ack lag passed [`SETTLE_TICKS`].
    pub late: u64,
    /// Clients whose lane differs from the rows routed to them.
    pub desyncs: u64,
    /// Worst ack lag seen, in ticks.
    pub worst_lag: u32,
    /// Authority cut lane digest.
    pub digest: u64,
}

impl CutSoakReport {
    /// One operator line.
    #[must_use]
    pub fn summary(&self) -> String {
        format!(
            "cut-soak clients={} ticks={} cuts={} routed={} packets={} dropped={} late={} desyncs={} worst_lag={} digest={:016x}",
            self.clients,
            self.ticks,
            self.cuts,
            self.routed,
            self.packets,
            self.dropped,
            self.late,
            self.desyncs,
            self.worst_lag,
            self.digest
        )
    }
}

/// One client's cut-lane mirror: interest, the rows the server routed here,
/// the rows this client applied, and its ack watermark.
struct Mirror {
    viewer: u32,
    interest: ConnectionInterest,
    selected: Vec<u32>,
    routed: CutLog,
    log: CutLog,
    acked: u32,
}

impl Mirror {
    fn new(viewer: u32, clients: usize) -> Result<Self, NetError> {
        Ok(Self {
            viewer,
            interest: ConnectionInterest::for_entities(viewer, clients)
                .map_err(|_| NetError::BadPayload)?,
            selected: vec![0u32; clients],
            routed: CutLog::new(),
            log: CutLog::new(),
            acked: 0,
        })
    }

    /// Every tick below `acked` is settled: all of its routed rows applied.
    fn advance_ack(&mut self, tick: u32) {
        while self.acked <= tick {
            let next = self.acked;
            let expected = self.routed.window(next, next).len();
            if expected == 0 || self.log.window(next, next).len() == expected {
                self.acked = next.saturating_add(1);
            } else {
                return;
            }
        }
    }
}

/// One cut packet on the link.
struct InFlight {
    due: u32,
    viewer: usize,
    bytes: Vec<u8>,
}

/// Run the soak for `ticks` authored ticks.
pub fn run(ticks: u32) -> Result<CutSoakReport, NetError> {
    if ticks == 0 {
        return Err(NetError::BadPayload);
    }
    let clients = usize::try_from(SOAK_CLIENTS).map_err(|_| NetError::BadPayload)?;
    let cfg = InterestCfg::default_64();
    let joints = severable_joints();
    let mut roster: Vec<InterestEntity> = (0..SOAK_CLIENTS).map(|at| lattice(at, 0)).collect();
    let mut mirrors = (0..SOAK_CLIENTS)
        .map(|viewer| Mirror::new(viewer, clients))
        .collect::<Result<Vec<_>, _>>()?;
    let mut server = CutLog::new();
    let mut in_flight: Vec<InFlight> = Vec::new();
    let mut report = CutSoakReport {
        clients: SOAK_CLIENTS,
        ticks,
        cuts: 0,
        routed: 0,
        packets: 0,
        dropped: 0,
        late: 0,
        desyncs: 0,
        worst_lag: 0,
        digest: 0,
    };

    for tick in 0..ticks {
        for (at, entity) in roster.iter_mut().enumerate() {
            let id = u32::try_from(at).map_err(|_| NetError::BadPayload)?;
            *entity = lattice(id, tick);
        }
        let authored = scheduled_cuts(tick, &joints);
        report.cuts = report
            .cuts
            .saturating_add(u64::try_from(authored.len()).unwrap_or(u64::MAX));
        server.record(tick, &authored);
        route_tick(&mut mirrors, &roster, &cfg, &server, tick, &mut report)?;
        send_pending(&mut mirrors, &mut in_flight, tick, &mut report)?;
        deliver(&mut in_flight, tick, &mut mirrors)?;
        for mirror in &mut mirrors {
            mirror.advance_ack(tick);
            let lag = tick.saturating_add(1).saturating_sub(mirror.acked);
            report.worst_lag = report.worst_lag.max(lag);
            if lag > SETTLE_TICKS {
                report.late = report.late.saturating_add(1);
            }
        }
    }

    // Drain: re-send what is still unacked and deliver until the window is
    // clean, bounded so a pathological loss pattern ends in a receipt instead
    // of a hang.
    let last = ticks.saturating_sub(1);
    let drain_bound = RTT_TICKS
        .saturating_add(JITTER_TICKS)
        .saturating_mul(4)
        .saturating_add(SETTLE_TICKS);
    let mut drain = 1;
    while drain <= drain_bound {
        let at = ticks.saturating_add(drain).saturating_sub(1);
        let clean = mirrors.iter().all(|mirror| mirror.acked > last) && in_flight.is_empty();
        if clean {
            break;
        }
        send_pending(&mut mirrors, &mut in_flight, at, &mut report)?;
        deliver(&mut in_flight, at, &mut mirrors)?;
        for mirror in &mut mirrors {
            mirror.advance_ack(last);
        }
        drain = drain.saturating_add(1);
    }

    for mirror in &mirrors {
        if !mirror.log.matches(&mirror.routed) {
            report.desyncs = report.desyncs.saturating_add(1);
        }
    }
    report.digest = server.digest();
    Ok(report)
}

/// Route this tick's rows to every viewer whose selection holds the victim,
/// then queue the viewer's window on the link.
fn route_tick(
    mirrors: &mut [Mirror],
    roster: &[InterestEntity],
    cfg: &InterestCfg,
    server: &CutLog,
    tick: u32,
    report: &mut CutSoakReport,
) -> Result<(), NetError> {
    for (at, mirror) in mirrors.iter_mut().enumerate() {
        let Some(entity) = roster.get(at) else {
            continue;
        };
        let chosen = mirror
            .interest
            .select(
                cfg,
                roster,
                mirror.viewer,
                entity.x_mm,
                entity.z_mm,
                yaw_ddeg(mirror.viewer),
                tick,
                SOAK_BUDGET_BYTES,
                &mut mirror.selected,
            )
            .map_err(|_| NetError::BadPayload)?;
        let visible = cuts::route(
            &mirror.selected[..chosen.selected],
            server.window(tick, tick),
        );
        if visible.is_empty() {
            continue;
        }
        let added = mirror.routed.record_rows(&visible);
        report.routed = report
            .routed
            .saturating_add(u64::try_from(added).unwrap_or(u64::MAX));
    }
    Ok(())
}

/// Send each viewer's oldest unacked rows; the link drops a seeded share and
/// delays the rest.
fn send_pending(
    mirrors: &mut [Mirror],
    in_flight: &mut Vec<InFlight>,
    tick: u32,
    report: &mut CutSoakReport,
) -> Result<(), NetError> {
    for (at, mirror) in mirrors.iter_mut().enumerate() {
        let Some(tip) = mirror.routed.tip() else {
            continue;
        };
        if tip < mirror.acked {
            continue;
        }
        let window = mirror.routed.window(mirror.acked, tip);
        if window.is_empty() {
            continue;
        }
        let take = window.len().min(MAX_CUTS_PER_PACKET);
        let rows: Vec<CutRow> = window[..take].to_vec();
        let bytes = encode_cuts(&rows)?;
        report.packets = report.packets.saturating_add(1);
        let viewer = u32::try_from(at).unwrap_or(0);
        if draw(viewer, tick, 1) % 100 < LOSS_PCT {
            report.dropped = report.dropped.saturating_add(1);
            continue;
        }
        let half = RTT_TICKS / 2;
        let jitter =
            u32::try_from(draw(viewer, tick, 2) % u64::from(JITTER_TICKS + 1)).unwrap_or(0);
        in_flight.push(InFlight {
            due: tick.saturating_add(half).saturating_add(jitter),
            viewer: at,
            bytes,
        });
    }
    Ok(())
}

/// Apply every packet due at `tick`.
fn deliver(
    in_flight: &mut Vec<InFlight>,
    tick: u32,
    mirrors: &mut [Mirror],
) -> Result<(), NetError> {
    let mut at = 0;
    while at < in_flight.len() {
        if in_flight[at].due > tick {
            at = at.saturating_add(1);
            continue;
        }
        let packet = in_flight.swap_remove(at);
        let (_, rows) = decode_cuts(&packet.bytes)?;
        if let Some(mirror) = mirrors.get_mut(packet.viewer) {
            mirror.log.record_rows(&rows);
        }
    }
    Ok(())
}

/// The tick's cut slots: two cuts per [`SLOT_TICKS`] ticks, 8 per second.
fn scheduled_cuts(tick: u32, joints: &[u8]) -> Vec<WireCut> {
    let slot = tick % SLOT_TICKS;
    let pair = (tick / SLOT_TICKS).saturating_mul(2);
    let index = if slot == 0 {
        pair
    } else if slot == SLOT_TICKS / 2 {
        pair.saturating_add(1)
    } else {
        return Vec::new();
    };
    let victim = u8::try_from(index % SOAK_CLIENTS).unwrap_or(0);
    let attacker = (victim + 1) % u8::try_from(SOAK_CLIENTS).unwrap_or(u8::MAX);
    let joint = joints
        .get(usize::try_from(index).unwrap_or(0) % joints.len().max(1))
        .copied()
        .unwrap_or(0);
    vec![WireCut {
        swing: tick.wrapping_mul(4).saturating_add(index % 4),
        attacker,
        victim,
        joint,
        flags: 0,
        energy_q16: 600_000,
        protection_q16: 589_824,
    }]
}

/// The anatomy's severable joints, ascending wire value.
fn severable_joints() -> Vec<u8> {
    JointId::all()
        .iter()
        .copied()
        .filter(|joint| joint.is_severable())
        .map(|joint| joint.as_u8())
        .collect()
}

/// One roster slot on an 8x8 lattice, drifting four metres on a slow sawtooth
/// so relative distances change and the interest selection really moves.
fn lattice(id: u32, tick: u32) -> InterestEntity {
    let row = i32::try_from((id / 8) % 8).unwrap_or(0);
    let col = i32::try_from(id % 8).unwrap_or(0);
    let phase = i32::try_from(tick.wrapping_add(id.wrapping_mul(7)) % 400).unwrap_or(0);
    InterestEntity {
        id,
        x_mm: col
            .saturating_mul(8_000)
            .saturating_add(phase.saturating_mul(100))
            .saturating_sub(20_000),
        z_mm: row.saturating_mul(8_000),
        combat: 0,
    }
}

/// Facing from the slot id: eight octants around the ring.
fn yaw_ddeg(id: u32) -> i16 {
    i16::try_from((id % 8).saturating_mul(450)).unwrap_or(0)
}

/// Seeded integer draw: the same `(viewer, tick, lane)` always gives the same
/// value, so the link's losses and delays replay exactly.
fn draw(viewer: u32, tick: u32, lane: u64) -> u64 {
    let mut fold = Fnv64::new();
    fold.write(b"e2cutsoak1");
    fold.write_u32(viewer);
    fold.write_u32(tick);
    fold.write_u64(lane);
    fold.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_soak_holds_64_clients_at_8_cuts_per_second() {
        let report = run(SOAK_TICKS).expect("soak runs");
        eprintln!("{}", report.summary());
        let cuts = u64::from(CUTS_PER_SECOND) * 10;
        assert_eq!(report.cuts, cuts, "8 cuts/s for 10 s");
        assert_eq!(report.desyncs, 0, "{}", report.summary());
        assert_eq!(report.late, 0, "{}", report.summary());
        assert!(report.worst_lag <= SETTLE_TICKS, "{}", report.summary());
        assert!(report.dropped > 0, "the link must drop some packets");
        assert!(report.routed > 0, "interest keeps some rows");
        assert!(
            report.routed < report.cuts * u64::from(SOAK_CLIENTS),
            "interest culls"
        );
    }

    #[test]
    fn soak_repeats_x2() {
        assert_eq!(run(SOAK_TICKS), run(SOAK_TICKS));
    }

    #[test]
    #[ignore = "full one-hour 64-client cut soak: run once on a runner (BX9)"]
    fn full_hour_soak() {
        let report = run(SOAK_TICKS_HOUR).expect("hour soak runs");
        eprintln!("{}", report.summary());
        assert_eq!(report.cuts, u64::from(CUTS_PER_SECOND) * 3_600);
        assert_eq!(report.desyncs, 0, "{}", report.summary());
        assert_eq!(report.late, 0, "{}", report.summary());
        assert!(report.worst_lag <= SETTLE_TICKS, "{}", report.summary());
    }
}
