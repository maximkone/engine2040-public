//! wan_soak: WAN fault soak over the siege harness.
//! Owner: net lane. Units: ticks at 60 Hz, ms wall-clock evidence only,
//! bytes. Determinism: server checkpoints come from the recorded siege tape,
//! delivery uses seeded integer draws in stable client order, no map order,
//! no wall clock in hashed state. Errors: [`WanError`], never panics.
//! Budget: rollback at most 8 ticks, resim p99 at most 2 ms, converge at
//! most 300 ticks (5 s), tick IO wait 0 ms. Consumer: the hour-long online
//! soak reads [`run_matrix`] plus [`FaultReport::summary`].
//!
//! Join tokens plus the server replay window: [`run_matrix_sized`] soaks
//! the same fault matrix over a 16-32 player session, [`JoinToken`] is the
//! short-lived join token of a rejoin, and [`ReplayWindow`] is the server's
//! per-lane history the per-fault reports count their refused replays
//! against. Token bytes ride the transport, so confidentiality is the
//! transport's job; nothing here hand-rolls a cipher.

use super::interest::{SNAPSHOT_EVERY_TICKS, STARVE_TICKS};
use super::netcode::MAX_ROLLBACK;
use super::relay::{answer_challenge, mint_challenge, verify_answer};
use super::scale::{MAX_DATAGRAM_BYTES, TICK_HZ};
use super::session::INPUT_DELAY_TICKS;
use super::tick_budget::{
    record_tape, record_tape_world, DesyncSite, SiegeTickWorld, TickBudgetError, ENTITIES,
    MAX_TICKS, PLAYERS,
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
/// Smallest session the sized matrix soaks: 16 players.
pub const SESSION_PLAYERS_MIN: usize = 16;
/// Largest session the sized matrix soaks: 32 players.
pub const SESSION_PLAYERS_MAX: usize = 32;
/// Join-token lifetime in ticks: 30 s at 60 Hz. Short-lived by design, and
/// tick-counted, so a token verdict never reads a wall clock.
pub const TOKEN_TTL_TICKS: u32 = 1_800;
/// Snapshot generations the server keeps per viewer lane for a rejoining
/// client: 32 generations of 6 ticks is 3.2 s of history, which covers the
/// 2 s disconnect window with room to spare.
pub const REPLAY_WINDOW_GENS: u32 = 32;
/// Datagram sequences the replay window remembers: one bit each, newest at
/// bit 0 (the rolling-window shape). A sequence behind the window or one
/// already seen refuses, so the sim never sees a generation twice.
pub const REPLAY_SEQ_BITS: u32 = 64;
/// One arrival in this many is re-sent by the network as a duplicate
/// datagram, so every per-fault report carries replays the window refuses.
/// A lattice, not a draw: the delivery schedule stays byte-identical.
pub const REPLAY_LATTICE: u32 = 16;
/// Ticks a duplicate datagram trails its original by (one jitter span).
pub const REPLAY_DELAY_TICKS: u32 = 7;
/// Wire bytes a version stamp adds to a resync request (lane plus version).
pub const STAMP_HEADER_BYTES: usize = 12;
/// Wire bytes an emergency-pool ticket adds to a pooled resync.
pub const POOL_TICKET_BYTES: usize = 4;
/// Emergency pool reserve in bytes: one generation payload, the same
/// one-datagram cap the interest encoder already holds (`MAX_DATAGRAM_BYTES`).
pub const EMERGENCY_POOL_BYTES: usize = MAX_DATAGRAM_BYTES;
/// Byte-overhead bar for the version path: ten whole percent over the AH4
/// resync bytes. Over the bar, or any missed checkpoint, kills the spike.
pub const OVERHEAD_BAR_PCT: u32 = 10;

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
    /// Session size outside `1..=PLAYERS` (no viewers means nothing to soak).
    BadPlayers {
        /// Players the caller offered.
        got: usize,
        /// Smallest soakable session.
        min: usize,
        /// Largest soakable session (the siege viewer bound).
        max: usize,
    },
    /// A rejoin presented a token the host refused.
    TokenRefused {
        /// Seat the token was bound to.
        viewer: u32,
        /// Why: expired, another session, another seat or a wrong answer.
        why: &'static str,
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
    /// The delivery ledger splits wrong: scheduled snapshots must each land
    /// as delivered or dropped, so the loss/distribution posts stay honest.
    Ledger {
        /// Snapshots scheduled across all viewers.
        scheduled: u32,
        /// Snapshots that arrived.
        delivered: u32,
        /// Snapshots dropped by loss or disconnect.
        dropped: u32,
    },
    /// A timing sample is NaN, infinite or negative.
    NonFiniteSample {
        /// What carried the bad timing.
        what: &'static str,
    },
    /// The live run's full state hash disagrees with the clean tape's at the
    /// last tick; `site` names the first divergent entity and component.
    Desync {
        /// Tick both hashes were taken at.
        tick: u32,
        /// First divergent entity and component (`None` only if the two
        /// worlds compare equal yet the hashes differ).
        site: Option<DesyncSite>,
    },
    /// The siege tick harness refused (budget, order or config).
    Tick(TickBudgetError),
    /// The version path's resync bytes went over the overhead bar.
    Overhead {
        /// Measured overhead in hundredths of a percent.
        pct_x100: u32,
        /// Bar in whole percent.
        bar_pct: u32,
        /// Bytes the version path sent for resyncs (plus its pool reserve).
        version_bytes: u64,
        /// Bytes the AH4 path sent for the same resyncs.
        ah4_bytes: u64,
    },
    /// The version path missed an authoritative checkpoint (the spike is
    /// killed by any miss, so the verdict names it).
    Miss {
        /// Checkpoints that disagreed with the clean tape.
        misses: u32,
        /// Last tick the tape hashed.
        tick: u32,
    },
    /// An emergency-pool admission did not fit the reserved bytes.
    PoolRefused {
        /// Bytes the urgent resync needed.
        need: usize,
        /// Bytes the pool holds.
        capacity: usize,
    },
}

impl Display for WanError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadTicks { got } => write!(f, "wan soak ticks {got} outside 1..={MAX_TICKS}"),
            Self::BadPlayers { got, min, max } => {
                write!(f, "wan soak players {got} outside {min}..={max}")
            }
            Self::TokenRefused { viewer, why } => {
                write!(f, "wan rejoin token for viewer {viewer} refused: {why}")
            }
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
            Self::Ledger {
                scheduled,
                delivered,
                dropped,
            } => write!(
                f,
                "wan ledger split wrong: scheduled {scheduled} != delivered {delivered} + dropped {dropped}"
            ),
            Self::NonFiniteSample { what } => {
                write!(f, "{what} sample is not finite non-negative ms")
            }
            Self::Desync { tick, site } => match site {
                Some(site) => write!(
                    f,
                    "wan desync at tick {tick}: first divergence entity {} component {:?}",
                    site.entity, site.component
                ),
                None => write!(f, "wan desync at tick {tick}: state hashes differ"),
            },
            Self::Tick(err) => write!(f, "wan siege harness refused ({err})"),
            Self::Overhead {
                pct_x100,
                bar_pct,
                version_bytes,
                ah4_bytes,
            } => write!(
                f,
                "wan version resync overhead {}.{:02}% over the {bar_pct}% bar (version {version_bytes} B vs ah4 {ah4_bytes} B)",
                pct_x100 / 100,
                pct_x100 % 100
            ),
            Self::Miss { misses, tick } => {
                write!(f, "wan version resync missed {misses} checkpoint(s) at tick {tick}")
            }
            Self::PoolRefused { need, capacity } => {
                write!(f, "wan emergency pool refused {need} B over {capacity} B")
            }
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
    /// Players in the soaked session (the sized matrix runs 16-32).
    pub players: u32,
    /// Authoritative server hash (equals the clean siege tape hash).
    pub hash: u64,
    /// Per-client snapshots scheduled across every viewer of the session.
    pub scheduled: u32,
    /// Snapshots that arrived (dropped ones resync on the next cadence).
    pub delivered: u32,
    /// Snapshots dropped by loss or disconnect.
    pub dropped: u32,
    /// Duplicated datagrams the replay window refused: they arrived, so they
    /// count under `delivered`, but the sim never sees a generation twice.
    pub replays_refused: u32,
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
    /// the harness builds the hash through the clean tape path. The ledger
    /// splits first: every scheduled snapshot is delivered or dropped, so
    /// the loss/latency distributions post from honest counts.
    pub fn check(&self) -> Result<(), WanError> {
        if u64::from(self.delivered) + u64::from(self.dropped) != u64::from(self.scheduled) {
            return Err(WanError::Ledger {
                scheduled: self.scheduled,
                delivered: self.delivered,
                dropped: self.dropped,
            });
        }
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
    /// wall-clock resim evidence. Twelve fields is the largest tuple that
    /// keeps `Debug` and `PartialEq`; the replay refusals are compared
    /// directly by the x2 test.
    #[must_use]
    pub const fn digest_key(self) -> (u64, u32, u32, u64, u32, u32, u32, u32, u32, u32, u32, u32) {
        (
            self.seed,
            self.ticks,
            self.players,
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
            "wan_soak fault={} seed={} ticks={} players={} hash={:016x} sched={} delivered={} dropped={} reordered={} resyncs={} replays_refused={}\n\
             wan_soak fault={} rollback_max={} (cap {}) staleness_max={} converge={} ticks (bar {}) resim_p99={:.3}ms (budget {:.1}ms) io_wait={:.1}ms starve_max={} (bar {STARVE_TICKS})",
            self.fault.label(),
            self.seed,
            self.ticks,
            self.players,
            self.hash,
            self.scheduled,
            self.delivered,
            self.dropped,
            self.reordered,
            self.resyncs,
            self.replays_refused,
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

/// Short-lived join token: minted by the host for one viewer of one session
/// and presented at (re)join, where the host verifies it deny-by-default.
///
/// Lifetime is tick-counted and the secret comes from the tape seed, so a
/// token verdict is identical on every run (no wall clock, no entropy source
/// in the verdict). The token bytes ride the transport session, so
/// confidentiality belongs to the transport; this type only carries the
/// short-lived credential and the challenge answer that proves it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct JoinToken {
    /// Session the token joins.
    pub session: u64,
    /// Viewer seat the token is bound to.
    pub viewer: u32,
    /// Tick the host minted the token.
    pub issued_at: u32,
    /// Lifetime in ticks; the token is dead past it.
    pub ttl_ticks: u32,
    /// Secret the challenge round proves. Never sent in clear.
    pub secret: u64,
}

impl JoinToken {
    /// Mint a token for `viewer` of `session` at `now`, live for
    /// [`TOKEN_TTL_TICKS`]. The secret is a seeded integer word, the same
    /// shape the relay ticket carries: a credential, not a cipher.
    #[must_use]
    pub fn mint(seed: u64, session: u64, viewer: u32, now: u32) -> Self {
        Self {
            session,
            viewer,
            issued_at: now,
            ttl_ticks: TOKEN_TTL_TICKS,
            secret: mix64(seed ^ session ^ u64::from(viewer) ^ u64::from(now) ^ GOLDEN_GAMMA),
        }
    }

    /// True while `now` is inside `[issued_at, issued_at + ttl]`. Tick
    /// arithmetic wraps, so a token minted after `now` reads expired, never
    /// live by accident.
    #[must_use]
    pub fn is_live(&self, now: u32) -> bool {
        self.ttl_ticks > 0 && now.wrapping_sub(self.issued_at) <= self.ttl_ticks
    }

    /// Challenge answer this token proves for `challenge` (the relay
    /// challenge round, shared with the invite handshake).
    #[must_use]
    pub fn answer(&self, challenge: u64) -> u64 {
        answer_challenge(challenge, self.secret)
    }

    /// Verify the rejoin handshake deny-by-default: same session, same seat,
    /// a live token, then a matching challenge answer. Every refusal names
    /// its reason and changes nothing.
    pub fn verify(
        &self,
        session: u64,
        viewer: u32,
        now: u32,
        challenge: u64,
        answer: u64,
    ) -> Result<(), WanError> {
        let refused = |why| WanError::TokenRefused { viewer, why };
        if self.session != session {
            return Err(refused("another session"));
        }
        if self.viewer != viewer {
            return Err(refused("another seat"));
        }
        if !self.is_live(now) {
            return Err(refused("expired"));
        }
        if !verify_answer(challenge, self.secret, answer) {
            return Err(refused("wrong answer"));
        }
        Ok(())
    }
}

/// Server replay window over one viewer lane's snapshot generations.
///
/// Two jobs, both posted in the per-fault reports: the sequence gate refuses
/// a datagram that duplicates or falls behind the rolling window, so the sim
/// never sees a generation twice; and the generations held are what a
/// rejoining client is handed, so a short absence costs no full resync.
/// Bounded by construction: one `u64` of sequences plus a generation count.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ReplayWindow {
    /// Newest datagram sequence accepted.
    newest_seq: u32,
    /// Bit `d` set means sequence `newest_seq - d` was already accepted.
    seen: u64,
    /// Generations held for a rejoining client, at most the window size.
    held: u32,
}

impl ReplayWindow {
    /// Open the window on viewer `lane`, holding every generation published
    /// up to `upto_tick` (at most [`REPLAY_WINDOW_GENS`]).
    #[must_use]
    pub fn new(lane: u32, upto_tick: u32) -> Self {
        let lane = lane % SNAPSHOT_EVERY_TICKS;
        let published = if upto_tick < lane {
            0
        } else {
            (upto_tick - lane) / SNAPSHOT_EVERY_TICKS + 1
        };
        Self {
            newest_seq: 0,
            seen: 0,
            held: published.min(REPLAY_WINDOW_GENS),
        }
    }

    /// Generations held for a rejoining client.
    #[must_use]
    pub const fn held(&self) -> u32 {
        self.held
    }

    /// Ticks of history the window can hand back.
    #[must_use]
    pub const fn coverage_ticks(&self) -> u32 {
        self.held * SNAPSHOT_EVERY_TICKS
    }

    /// Accept or refuse one datagram sequence. A sequence already seen (a
    /// replay) or one at or behind `newest_seq - REPLAY_SEQ_BITS` refuses;
    /// anything newer advances the window.
    pub fn accept(&mut self, seq: u32) -> bool {
        if seq > self.newest_seq {
            let delta = seq - self.newest_seq;
            self.seen = if delta >= REPLAY_SEQ_BITS {
                0
            } else {
                self.seen << delta
            };
            self.newest_seq = seq;
            self.seen |= 1;
            return true;
        }
        let delta = self.newest_seq - seq;
        if delta >= REPLAY_SEQ_BITS {
            return false;
        }
        let bit = 1u64 << delta;
        if self.seen & bit != 0 {
            return false;
        }
        self.seen |= bit;
        true
    }

    /// Hand a rejoining client its history after `away_ticks` away:
    /// `(generations replayed, ticks of the gap the window cannot cover)`.
    #[must_use]
    pub const fn handoff(&self, away_ticks: u32) -> (u32, u32) {
        let coverage = self.coverage_ticks();
        let covered = if away_ticks < coverage {
            away_ticks
        } else {
            coverage
        };
        (
            covered / SNAPSHOT_EVERY_TICKS,
            away_ticks.saturating_sub(coverage),
        )
    }
}

/// One seeded rejoin: a viewer drops, then rejoins with its short-lived token
/// and the server's replay window hands its history back.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct RejoinReport {
    /// Fault the drop models (the disconnect window).
    pub fault: Fault,
    /// Tape seed.
    pub seed: u64,
    /// Ticks stepped.
    pub ticks: u32,
    /// Players in the session.
    pub players: u32,
    /// Seat that dropped.
    pub viewer: u32,
    /// Tick it dropped.
    pub dropped_at: u32,
    /// Tick it rejoined.
    pub rejoined_at: u32,
    /// Ticks it was away.
    pub away_ticks: u32,
    /// Generations the window replayed to it.
    pub replayed: u32,
    /// Ticks of its gap the window could not cover.
    pub uncovered_ticks: u32,
    /// Duplicated datagrams the window refused over the tape.
    pub replays_refused: u32,
    /// Ticks from rejoin to holding the checkpoint (bar
    /// [`CONVERGE_BAR_TICKS`], 5 s).
    pub converge_ticks: u32,
    /// Authoritative checkpoint the rejoin converged on.
    pub hash: u64,
}

/// Newest snapshot generation a client holds: the lane it replicates on and
/// the generation's version (its server tick). A resync presents this stamp,
/// so the host sends only generations newer than it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct VersionStamp {
    /// Viewer lane (server tick modulo the snapshot cadence).
    pub lane: u32,
    /// Newest generation the client holds (a server tick).
    pub version: u32,
}

impl VersionStamp {
    /// Stamp a client that holds generation `version` on `lane`.
    #[must_use]
    pub const fn of(lane: u32, version: u32) -> Self {
        Self { lane, version }
    }

    /// True when the client already holds `generation`, so a resync has
    /// nothing to send for it.
    #[must_use]
    pub const fn covers(self, generation: u32) -> bool {
        generation <= self.version
    }
}

/// Bounded byte pool reserved for urgent resyncs: the newest stamped baseline
/// is copied into the reserve and served at once, so a resyncing client does
/// not wait for the next cadence. The reserve is held for the tape, so its
/// bytes are capacity the normal delta stream cannot use; an admission that
/// does not fit refuses by name instead of growing the pool.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct EmergencyPool {
    capacity: usize,
    used: usize,
    peak: usize,
    refused: u32,
}

impl EmergencyPool {
    /// Reserve `capacity` bytes for urgent resyncs.
    #[must_use]
    pub const fn new(capacity: usize) -> Self {
        Self {
            capacity,
            used: 0,
            peak: 0,
            refused: 0,
        }
    }

    /// Bytes currently admitted (back to 0 after a release).
    #[must_use]
    pub const fn used(&self) -> usize {
        self.used
    }

    /// High-water mark of admitted bytes.
    #[must_use]
    pub const fn peak(&self) -> usize {
        self.peak
    }

    /// Refused admissions (the pool never grows).
    #[must_use]
    pub const fn refused(&self) -> u32 {
        self.refused
    }

    /// Admit one urgent payload, or refuse with both sides. A zero-byte
    /// request needs no reserve.
    pub fn admit(&mut self, bytes: usize) -> Result<(), WanError> {
        if bytes == 0 {
            return Ok(());
        }
        if bytes > self.capacity.saturating_sub(self.used) {
            self.refused = self.refused.saturating_add(1);
            return Err(WanError::PoolRefused {
                need: bytes,
                capacity: self.capacity,
            });
        }
        self.used += bytes;
        if self.used > self.peak {
            self.peak = self.used;
        }
        Ok(())
    }

    /// Give one admitted payload back when its resync converges.
    pub fn release(&mut self, bytes: usize) {
        self.used = self.used.saturating_sub(bytes);
    }
}

/// One fault's version-stamped resync report: the AH4 baseline row beside the
/// version path's converge and wire bytes, the pool's use, and the numbers
/// the overhead verdict reads. Deterministic per (seed, ticks, players).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VersionResyncReport {
    /// The AH4 row this path is measured against (checkpoint and converge).
    pub ah4: FaultReport,
    /// Urgent resyncs the version path served.
    pub resyncs: u32,
    /// Resync windows the pool was armed for.
    pub pool_armed: u32,
    /// Admissions the pool refused (0 while one generation fits the reserve).
    pub pool_refused: u32,
    /// High-water mark of the pool reserve, in bytes.
    pub pool_peak_bytes: u32,
    /// Mean bytes of one interest-filtered snapshot generation.
    pub generation_bytes: u64,
    /// Ticks the version path needs (the pool serves at once, one transit).
    pub version_converge_ticks: u32,
    /// Bytes the AH4 path sent to resync: one generation per urgent resync.
    pub ah4_bytes: u64,
    /// Bytes the version path sent, pool reserve included.
    pub version_bytes: u64,
    /// Overhead over the AH4 resync bytes, in hundredths of a percent.
    pub overhead_pct_x100: u32,
    /// Checkpoints that disagreed with the clean tape.
    pub checkpoint_misses: u32,
}

impl VersionResyncReport {
    /// Gate the spike: any miss, an overhead over [`OVERHEAD_BAR_PCT`], or a
    /// converge past the 5 s bar fails with both sides. A killed spike is a
    /// result, not a pass, so the caller must read the verdict.
    pub fn check(&self) -> Result<(), WanError> {
        if self.checkpoint_misses > 0 {
            return Err(WanError::Miss {
                misses: self.checkpoint_misses,
                tick: self.ah4.ticks.saturating_sub(1),
            });
        }
        if self.overhead_pct_x100 > OVERHEAD_BAR_PCT.saturating_mul(100) {
            return Err(WanError::Overhead {
                pct_x100: self.overhead_pct_x100,
                bar_pct: OVERHEAD_BAR_PCT,
                version_bytes: self.version_bytes,
                ah4_bytes: self.ah4_bytes,
            });
        }
        if self.version_converge_ticks > CONVERGE_BAR_TICKS {
            return Err(WanError::SlowConverge {
                got: self.version_converge_ticks,
                bar: CONVERGE_BAR_TICKS,
            });
        }
        Ok(())
    }

    /// True when the spike holds every checkpoint and the overhead bar.
    #[must_use]
    pub fn accepted(&self) -> bool {
        self.check().is_ok()
    }

    /// Posted per-fault line: converge and bytes for both schemes, the pool's
    /// use, the overhead and the verdict.
    #[must_use]
    pub fn summary(&self) -> String {
        format!(
            "wan_version fault={} seed={} players={} hash={:016x} resyncs={} armed={} refused={} peak={}B gen={}B\n\
             wan_version fault={} converge={} (ah4 {}) ah4_bytes={} version_bytes={} overhead={}.{:02}% (bar {}%) misses={} verdict={}",
            self.ah4.fault.label(),
            self.ah4.seed,
            self.ah4.players,
            self.ah4.hash,
            self.resyncs,
            self.pool_armed,
            self.pool_refused,
            self.pool_peak_bytes,
            self.generation_bytes,
            self.ah4.fault.label(),
            self.version_converge_ticks,
            self.ah4.converge_ticks,
            self.ah4_bytes,
            self.version_bytes,
            self.overhead_pct_x100 / 100,
            self.overhead_pct_x100 % 100,
            OVERHEAD_BAR_PCT,
            self.checkpoint_misses,
            if self.accepted() { "accepted" } else { "KILLED" },
        )
    }
}

/// Disconnect window a tape of `ticks` models: start tick and end tick.
fn disconnect_window(ticks: u32) -> (u32, u32) {
    let start = ticks / 3;
    let len = DISCONNECT_LEN.min(ticks.saturating_sub(start));
    (start, start.saturating_add(len))
}

/// One viewer's delivery plan for one fault: every generation it should
/// receive, in server order, with its arrival tick and drop flag. Reorder
/// swaps the arrival ticks of a seeded 5% sample of adjacent pairs.
fn plan_viewer(
    seed: u64,
    fault: Fault,
    viewer: u32,
    ticks: u32,
) -> Result<Vec<Delivery>, WanError> {
    let lane = viewer % SNAPSHOT_EVERY_TICKS;
    let (disc_start, disc_end) = disconnect_window(ticks);
    let victims = u32::try_from(DISCONNECT_VICTIMS).unwrap_or(u32::MAX);
    let mut draws = Draws::seed(seed, fault, viewer);
    let mut plan: Vec<Delivery> = Vec::new();
    let mut gen = lane;
    while gen < ticks {
        let mut delay = RTT_TICKS;
        if fault.has_jitter() {
            let span = 2 * u64::from(JITTER_TICKS) + 1;
            let jitter = draws.below(span);
            let jitter = u32::try_from(jitter).unwrap_or(JITTER_TICKS);
            delay = delay.saturating_add(jitter).saturating_sub(JITTER_TICKS);
        }
        let deliver = gen.saturating_add(delay);
        let mut drop = false;
        if fault.has_loss() && draws.below(100) < LOSS_PCT {
            drop = true;
        }
        let victim = fault == Fault::Disconnect && viewer < victims;
        if victim && gen >= disc_start && gen < disc_end {
            drop = true;
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
    Ok(plan)
}

/// Ticks until the next generation of `lane` after `tick`: the wait an AH4
/// re-baseline pays before its full snapshot is sent. Bounded by the cadence.
fn cadence_wait(lane: u32, tick: u32) -> u32 {
    let lane = lane % SNAPSHOT_EVERY_TICKS;
    let since = tick.wrapping_sub(lane) % SNAPSHOT_EVERY_TICKS;
    SNAPSHOT_EVERY_TICKS - since
}

/// Generations the canonical siege tape publishes over `ticks`: the world
/// always runs all [`PLAYERS`] connections, whatever the soaked session size,
/// so this is the denominator that turns the tape's byte total into a
/// per-generation mean. One lane publishes `lane, lane + 6, ...`.
fn tape_generations(ticks: u32) -> u32 {
    (0..PLAYERS)
        .map(|viewer| u32::try_from(viewer).unwrap_or(0))
        .map(|lane| {
            let lane = lane % SNAPSHOT_EVERY_TICKS;
            if ticks <= lane {
                0
            } else {
                (ticks - lane - 1) / SNAPSHOT_EVERY_TICKS + 1
            }
        })
        .sum()
}

/// Refusal for a session size the siege harness cannot serve.
fn bad_players(players: usize) -> WanError {
    WanError::BadPlayers {
        got: players,
        min: 1,
        max: PLAYERS,
    }
}

/// Validate one soak shape: ticks inside the tape bound and a session size
/// the siege harness can serve.
fn check_session(ticks: u32, players: usize) -> Result<(), WanError> {
    if ticks == 0 || ticks > MAX_TICKS {
        return Err(WanError::BadTicks { got: ticks });
    }
    if players == 0 || players > PLAYERS {
        return Err(bad_players(players));
    }
    Ok(())
}

/// Soak one seeded rejoin over the standard disconnect window
/// ([`DISCONNECT_LEN`] ticks away).
pub fn run_rejoin(seed: u64, ticks: u32, players: usize) -> Result<RejoinReport, WanError> {
    run_rejoin_away(seed, ticks, players, DISCONNECT_LEN)
}

/// [`run_rejoin`] with `away_ticks` chosen by the caller, so a test can prove
/// a gap past the window's coverage blows the 5 s bar instead of passing.
///
/// Order: validate the shape, record the authoritative tape, pick the victim
/// seat from the seed, mint its short-lived token at the drop, then at the
/// rejoin run the token handshake, count the replays the window refused and
/// hand its held generations back. Converge is the uncovered part of the gap
/// plus one transit ([`RTT_TICKS`]); a client the window covers resumes at
/// once, so a covered rejoin converges in one transit.
pub fn run_rejoin_away(
    seed: u64,
    ticks: u32,
    players: usize,
    away_ticks: u32,
) -> Result<RejoinReport, WanError> {
    check_session(ticks, players)?;
    let seats = u32::try_from(players).map_err(|_| bad_players(players))?;
    let tape = record_tape_world(seed, ticks)?.0;
    let session = seed ^ Fault::Disconnect.salt();
    // Victim seat: a seeded pick inside the session, so the same seed always
    // drops, and rejoins, the same seat.
    let pick = mix64(session) % u64::from(seats);
    let viewer = u32::try_from(pick).map_err(|_| bad_players(players))?;
    let lane = viewer % SNAPSHOT_EVERY_TICKS;
    // The seat is away for `away_ticks` inside the tape, so its outage leaves
    // generations the window must cover.
    let dropped_at = (ticks / 3).max(lane);
    let rejoined_at = dropped_at.saturating_add(away_ticks).min(ticks);
    // Join-token handshake: mint at the drop, present at the rejoin. A wrong
    // session, seat, lifetime or answer refuses and changes nothing.
    let token = JoinToken::mint(seed, session, viewer, dropped_at);
    let challenge = mint_challenge(session, u64::from(seats));
    token.verify(
        session,
        viewer,
        rejoined_at,
        challenge,
        token.answer(challenge),
    )?;
    let window = ReplayWindow::new(lane, ticks);
    let (replayed, uncovered_ticks) = window.handoff(away_ticks);
    // Every generation is offered once; the ones the seat missed while it was
    // away are offered again, and the window refuses each duplicate, so the
    // report carries the refusals it made over the tape.
    let mut gate = window;
    let mut replays_refused = 0u32;
    let mut generation = lane;
    while generation < ticks {
        if !gate.accept(generation) {
            replays_refused = replays_refused.saturating_add(1);
        }
        if generation >= dropped_at && generation < rejoined_at && !gate.accept(generation) {
            replays_refused = replays_refused.saturating_add(1);
        }
        generation = generation.saturating_add(SNAPSHOT_EVERY_TICKS);
    }
    let converge_ticks = uncovered_ticks.saturating_add(RTT_TICKS);
    if converge_ticks > CONVERGE_BAR_TICKS {
        return Err(WanError::SlowConverge {
            got: converge_ticks,
            bar: CONVERGE_BAR_TICKS,
        });
    }
    Ok(RejoinReport {
        fault: Fault::Disconnect,
        seed,
        ticks,
        players: seats,
        viewer,
        dropped_at,
        rejoined_at,
        away_ticks,
        replayed,
        uncovered_ticks,
        replays_refused,
        converge_ticks,
        hash: tape.hash,
    })
}

/// Soak one fault: step the siege world for the authoritative bytes,
/// pin them against the clean tape checkpoint, then run the deterministic
/// per-client delivery and fold the per-fault report.
pub fn run_fault(seed: u64, fault: Fault, ticks: u32) -> Result<FaultReport, WanError> {
    run_fault_sized(seed, fault, ticks, PLAYERS)
}

/// [`run_fault`] over a session of `players` viewers; the sized matrix runs
/// 16-32, and the default is the full siege.
pub fn run_fault_sized(
    seed: u64,
    fault: Fault,
    ticks: u32,
    players: usize,
) -> Result<FaultReport, WanError> {
    check_session(ticks, players)?;
    let seats = u32::try_from(players).map_err(|_| bad_players(players))?;
    run_fault_with(seed, fault, ticks, seats, |_, _| {})
}

/// [`run_fault_sized`] with `after_tick` called on the live world after every
/// step, so a test can play a desynced peer.
fn run_fault_with(
    seed: u64,
    fault: Fault,
    ticks: u32,
    players: u32,
    mut after_tick: impl FnMut(u32, &mut SiegeTickWorld),
) -> Result<FaultReport, WanError> {
    if ticks == 0 || ticks > MAX_TICKS {
        return Err(WanError::BadTicks { got: ticks });
    }
    // Authoritative checkpoint: the clean siege tape. The fault harness below
    // never perturbs server state; delivery only decides when each client
    // sees a generation the server already hashed.
    let (tape, clean_world) = record_tape_world(seed, ticks)?;
    // Drive the same siege world once more so the report carries the live
    // interest bytes and starve ledger, not a copy of the tape struct: the
    // byte totals must match the checkpoint exactly.
    let mut world = SiegeTickWorld::new(seed)?;
    let mut live_bytes = 0u64;
    for tick in 0..ticks {
        live_bytes += world.step_tick(tick)?;
        after_tick(tick, &mut world);
    }
    if live_bytes != tape.snapshot_bytes {
        return Err(WanError::Tick(TickBudgetError::BadTicks { got: ticks }));
    }
    // Peers compare the cheap full hash; only a mismatch pays for the Merkle
    // descent that names the first divergent entity and component.
    let last = ticks - 1;
    if world.state_hash(last) != clean_world.state_hash(last) {
        return Err(WanError::Desync {
            tick: last,
            site: world.first_divergence(&clean_world),
        });
    }

    let (disc_start, disc_end) = disconnect_window(ticks);
    let disc_len = disc_end.saturating_sub(disc_start);

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
    // Duplicated datagrams the replay window refused over the whole session.
    let mut replays_refused = 0u32;
    // Outage seats as a loop-width count (the first eight, matching
    // [`DISCONNECT_VICTIMS`]); a target too narrow to hold it models none.
    let victims = u32::try_from(DISCONNECT_VICTIMS).unwrap_or(u32::MAX);
    // Drain end over every delivery plus victim resync lag for the converge
    // number below. Preallocated once; ticks borrow it without growth.
    let mut last_deliver = 0u32;
    let mut victim_last = 0u32;

    for viewer in 0..players {
        // Siege stagger: this viewer replicates when its lane is due, so its
        // generations sit `viewer % 6, +6k`. Same cadence the world stepped.
        let lane = viewer % SNAPSHOT_EVERY_TICKS;
        let plan = plan_viewer(seed, fault, viewer, ticks)?;
        // Victims that lost generations inside the outage window take one
        // authoritative resync on reconnect, never a deep rollback. Bitmask
        // over the first 8 slots (index-free by construction).
        if fault == Fault::Disconnect && viewer < victims {
            let lost = plan.iter().any(|item| {
                item.dropped && item.server_tick >= disc_start && item.server_tick < disc_end
            });
            if lost {
                victim_mask |= 1 << viewer;
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
        // Replay window: feed this viewer's arrivals in delivery order plus
        // the lattice duplicate datagrams every REPLAY_LATTICE-th generation
        // draws. A duplicate or a sequence behind the window refuses, so the
        // sim never sees one generation twice; the refusals are posted in the
        // report. This pass touches no ledger and no budget number below.
        let mut window = ReplayWindow::new(lane, ticks);
        let mut arrivals: Vec<(u32, u32)> = plan
            .iter()
            .filter(|item| !item.dropped)
            .map(|item| (item.deliver_tick, item.server_tick))
            .collect();
        for item in &plan {
            let lattice = (item.server_tick / SNAPSHOT_EVERY_TICKS).is_multiple_of(REPLAY_LATTICE);
            if item.dropped || !lattice {
                continue;
            }
            arrivals.push((
                item.deliver_tick.saturating_add(REPLAY_DELAY_TICKS),
                item.server_tick,
            ));
        }
        arrivals.sort_unstable();
        for (_, seq) in arrivals {
            if !window.accept(seq) {
                replays_refused = replays_refused.saturating_add(1);
            }
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
            let victim = fault == Fault::Disconnect && viewer < victims;
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
        players,
        hash: tape.hash,
        scheduled,
        delivered,
        dropped,
        replays_refused,
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
    run_matrix_sized(seed, ticks, PLAYERS)
}

/// [`run_matrix`] over a session of `players` viewers: the sized matrix runs
/// 16-32 ([`SESSION_PLAYERS_MIN`], [`SESSION_PLAYERS_MAX`]) and every entry
/// still holds the clean tape's checkpoint.
pub fn run_matrix_sized(
    seed: u64,
    ticks: u32,
    players: usize,
) -> Result<[FaultReport; 5], WanError> {
    check_session(ticks, players)?;
    let latency = run_fault_sized(seed, Fault::Latency, ticks, players)?;
    let jitter = run_fault_sized(seed, Fault::Jitter, ticks, players)?;
    let loss = run_fault_sized(seed, Fault::Loss, ticks, players)?;
    let reorder = run_fault_sized(seed, Fault::Reorder, ticks, players)?;
    let disconnect = run_fault_sized(seed, Fault::Disconnect, ticks, players)?;
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

/// Byte and converge ledger for one fault's version-stamped resync path: a
/// stamp that already covers the target sends nothing, a pooled resync sends
/// the newest baseline at once, and a refused admission waits for the cadence.
#[derive(Default)]
struct VersionLedger {
    resyncs: u32,
    armed: u32,
    refused: u32,
    bytes: u64,
    wait_saved: u32,
}

impl VersionLedger {
    /// Serve one urgent resync at `tick` whose newest generation is `target`
    /// and whose client holds `held` (none before its first arrival).
    fn urgent(
        &mut self,
        pool: &mut EmergencyPool,
        lane: u32,
        target: u32,
        held: Option<u32>,
        tick: u32,
        generation_bytes: u64,
    ) {
        self.resyncs = self.resyncs.saturating_add(1);
        // A client with no baseline yet is treated as holding the generation
        // one cadence back, so its gap is the pool's one baseline.
        let version = held.unwrap_or_else(|| target.saturating_sub(SNAPSHOT_EVERY_TICKS));
        if VersionStamp::of(lane, version).covers(target) {
            return;
        }
        let need = usize::try_from(generation_bytes).unwrap_or(usize::MAX);
        let stamp_bytes = u64::try_from(STAMP_HEADER_BYTES).unwrap_or(0);
        self.armed = self.armed.saturating_add(1);
        self.bytes = self
            .bytes
            .saturating_add(generation_bytes)
            .saturating_add(stamp_bytes);
        if pool.admit(need).is_ok() {
            pool.release(need);
            // The reserve serves the baseline now: no cadence wait, and the
            // ticket rides the pooled payload.
            self.bytes = self
                .bytes
                .saturating_add(u64::try_from(POOL_TICKET_BYTES).unwrap_or(0));
            let saved = cadence_wait(lane, tick);
            if saved > self.wait_saved {
                self.wait_saved = saved;
            }
        } else {
            // A refused admission re-baselines on the cadence, exactly like
            // AH4; only the stamp request is extra.
            self.refused = self.refused.saturating_add(1);
        }
    }
}

/// Walk every viewer's delivery plan for one fault and serve its urgent
/// resyncs through the version path, returning the ledger and the pool.
fn version_walk(
    seed: u64,
    fault: Fault,
    ticks: u32,
    seats: u32,
    generation_bytes: u64,
) -> Result<(VersionLedger, EmergencyPool), WanError> {
    let (disc_start, disc_end) = disconnect_window(ticks);
    let disc_len = disc_end.saturating_sub(disc_start);
    let victims = u32::try_from(DISCONNECT_VICTIMS).unwrap_or(u32::MAX);
    let mut pool = EmergencyPool::new(EMERGENCY_POOL_BYTES);
    let mut ledger = VersionLedger::default();
    for viewer in 0..seats {
        let lane = viewer % SNAPSHOT_EVERY_TICKS;
        let plan = plan_viewer(seed, fault, viewer, ticks)?;
        let mut held: Option<u32> = None;
        for item in &plan {
            if item.dropped {
                continue;
            }
            let stale = item.deliver_tick.saturating_sub(item.server_tick);
            let distance = stale.saturating_sub(INPUT_DELAY_TICKS);
            // Same trigger as the AH4 ledger: past the rollback window the
            // client cannot resimulate and needs an authoritative resync.
            if distance > u32::try_from(MAX_ROLLBACK).unwrap_or(u32::MAX) {
                ledger.urgent(
                    &mut pool,
                    lane,
                    item.server_tick,
                    held,
                    item.deliver_tick,
                    generation_bytes,
                );
            }
            held = Some(item.server_tick);
        }
        // Outage victims: one urgent resync on reconnect, matching the AH4
        // ledger's one-per-victim rule.
        if fault == Fault::Disconnect && viewer < victims && disc_len > 0 {
            let lost = plan.iter().any(|item| {
                item.dropped && item.server_tick >= disc_start && item.server_tick < disc_end
            });
            if lost {
                let last_held = plan
                    .iter()
                    .filter(|item| !item.dropped && item.server_tick < disc_start)
                    .map(|item| item.server_tick)
                    .next_back();
                // Newest generation the server already holds for this lane:
                // the baseline the pool serves at reconnect.
                let newest = if disc_end < lane {
                    lane
                } else {
                    lane + ((disc_end - lane) / SNAPSHOT_EVERY_TICKS) * SNAPSHOT_EVERY_TICKS
                };
                ledger.urgent(
                    &mut pool,
                    lane,
                    newest,
                    last_held,
                    disc_end,
                    generation_bytes,
                );
            }
        }
    }
    Ok((ledger, pool))
}

/// Soak one fault over `players` viewers through the version-stamped resync
/// path (the CC-OPI spike): each urgent resync presents the client's stamp,
/// the emergency pool serves the newest baseline at once, and the report
/// posts both schemes' converge and bytes. The AH4 run underneath stays the
/// baseline, so its checkpoint and ledger are what the spike is measured
/// against. No codec changes: a version is the generation's server tick.
pub fn run_version_fault_sized(
    seed: u64,
    fault: Fault,
    ticks: u32,
    players: usize,
) -> Result<VersionResyncReport, WanError> {
    check_session(ticks, players)?;
    let seats = u32::try_from(players).map_err(|_| bad_players(players))?;
    let ah4 = run_fault_sized(seed, fault, ticks, players)?;
    let tape = record_tape(seed, ticks)?;
    // The tape always publishes all 64 lanes, so its byte total is divided by
    // the tape's generation count, not by the soaked session's schedule.
    let generation_bytes = tape.snapshot_bytes / u64::from(tape_generations(ticks).max(1));
    let (mut ledger, pool) = version_walk(seed, fault, ticks, seats, generation_bytes)?;
    // The reserve is held for the tape, so its bytes are charged once when the
    // pool was armed at all: capacity the normal delta stream cannot use.
    if ledger.armed > 0 {
        ledger.bytes = ledger
            .bytes
            .saturating_add(u64::try_from(EMERGENCY_POOL_BYTES).unwrap_or(0));
    }
    let ah4_bytes = u64::from(ah4.resyncs).saturating_mul(generation_bytes);
    let overhead_pct_x100 = if ah4_bytes == 0 {
        0
    } else {
        let extra = ledger.bytes.saturating_sub(ah4_bytes);
        u32::try_from(extra.saturating_mul(10_000) / ah4_bytes).unwrap_or(u32::MAX)
    };
    let checkpoint_misses = u32::from(ah4.hash != tape.hash);
    let version_converge_ticks = ah4.converge_ticks.saturating_sub(ledger.wait_saved);
    let pool_peak_bytes = u32::try_from(pool.peak()).unwrap_or(u32::MAX);
    Ok(VersionResyncReport {
        ah4,
        resyncs: ledger.resyncs,
        pool_armed: ledger.armed,
        pool_refused: ledger.refused,
        pool_peak_bytes,
        generation_bytes,
        version_converge_ticks,
        ah4_bytes,
        version_bytes: ledger.bytes,
        overhead_pct_x100,
        checkpoint_misses,
    })
}

#[cfg(test)]
mod tests {
    use super::super::tick_budget::{record_tape, SiegeComponent, SIEGE_TICKS};
    use super::*;

    const TEST_TICKS: u32 = 240;
    /// Ticks per sized-matrix cell in the band sweep: the band runs 17
    /// session sizes, so each cell stays short.
    const BAND_TICKS: u32 = 120;

    #[test]
    fn wan_desync_names_the_first_divergent_entity_and_component() {
        let desynced = run_fault_with(
            7,
            Fault::Latency,
            TEST_TICKS,
            PLAYERS as u32,
            |tick, world| {
                if tick == 100 {
                    world.nudge_x(450);
                    world.nudge_x(300);
                }
            },
        );
        let site = DesyncSite {
            entity: 300,
            component: SiegeComponent::X,
        };
        assert_eq!(
            desynced.unwrap_err(),
            WanError::Desync {
                tick: TEST_TICKS - 1,
                site: Some(site),
            }
        );
        run_fault(7, Fault::Latency, TEST_TICKS).expect("without the nudge the peers agree");
    }

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
    fn wan_soak_ledger_split_fails_loudly() {
        // ON30 posts loss/latency distributions from this ledger: every
        // scheduled snapshot is delivered or dropped, never both, never lost.
        let good = run_fault(7, Fault::Loss, 60).expect("short soak must converge");
        assert_eq!(
            u64::from(good.delivered) + u64::from(good.dropped),
            u64::from(good.scheduled),
            "the harness keeps a split ledger"
        );
        good.check().expect("a split ledger must pass its bars");
        // A forged report that drops a snapshot from the ledger fails loudly.
        let mut bad = good;
        bad.delivered = bad.delivered.saturating_add(1);
        assert_eq!(
            bad.check(),
            Err(WanError::Ledger {
                scheduled: bad.scheduled,
                delivered: bad.delivered,
                dropped: bad.dropped,
            }),
            "a split ledger must fail loudly with all three sides"
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

    #[test]
    fn wan_soak_matrix_sized_16_to_32_holds_every_checkpoint() {
        // Accept: the fault matrix at 16-32 players converges on every
        // authoritative checkpoint with a per-fault report that passes its
        // bars, and the replay window refuses the duplicated datagrams.
        let clean = record_tape(7, BAND_TICKS).expect("clean tape must record");
        for players in SESSION_PLAYERS_MIN..=SESSION_PLAYERS_MAX {
            let matrix = run_matrix_sized(7, BAND_TICKS, players)
                .unwrap_or_else(|err| panic!("{players} players: {err}"));
            assert_eq!(matrix.len(), ALL_FAULTS.len(), "one report per fault");
            for (entry, fault) in matrix.iter().zip(ALL_FAULTS) {
                assert_eq!(entry.fault, fault, "matrix keeps report order");
                assert_eq!(entry.players as usize, players, "report names its session");
                assert_eq!(
                    entry.hash, clean.hash,
                    "{fault} at {players} players must hold the checkpoint"
                );
                assert!(
                    entry.scheduled > 0,
                    "{fault}: a session schedules deliveries"
                );
                assert!(
                    entry.replays_refused > 0,
                    "{fault} at {players} players: the window must refuse the lattice replays"
                );
                entry
                    .check()
                    .expect("every per-fault report must pass its bars");
                let text = entry.summary();
                assert!(text.contains(&format!("players={players}")), "{text}");
                assert!(text.contains("replays_refused="), "{text}");
            }
        }
    }

    #[test]
    fn wan_soak_matrix_sized_repeats_identically_x2() {
        // The sized matrix is seeded, so the endpoints replay bit-identically.
        for players in [SESSION_PLAYERS_MIN, SESSION_PLAYERS_MAX] {
            let first = run_matrix_sized(7, TEST_TICKS, players).expect("matrix converges");
            let second = run_matrix_sized(7, TEST_TICKS, players).expect("matrix converges again");
            for (a, b) in first.iter().zip(second.iter()) {
                assert_eq!(a.digest_key(), b.digest_key(), "same seed, same ledger");
                assert_eq!(
                    a.replays_refused, b.replays_refused,
                    "the window refuses the same replays every run"
                );
            }
            for entry in &first {
                assert_eq!(entry.players as usize, players);
                assert!(
                    entry.rollback_max as usize <= MAX_ROLLBACK,
                    "sized matrix keeps the rollback window"
                );
            }
        }
    }

    #[test]
    fn wan_rejoin_seeded_converges_in_five_seconds() {
        // Accept: a seeded rejoin converges in 5 s or less, on the tape's
        // checkpoint, with the window handing its history back.
        for players in [SESSION_PLAYERS_MIN, SESSION_PLAYERS_MAX] {
            let report = run_rejoin(7, TEST_TICKS, players).expect("seeded rejoin converges");
            let clean = record_tape(7, TEST_TICKS).expect("clean tape must record");
            assert_eq!(report.fault, Fault::Disconnect);
            assert_eq!(report.hash, clean.hash, "rejoin must hold the checkpoint");
            assert_eq!(report.players as usize, players);
            assert!(
                report.viewer < players as u32,
                "victim sits inside the session"
            );
            assert_eq!(report.away_ticks, DISCONNECT_LEN);
            assert_eq!(
                report.rejoined_at,
                report.dropped_at + DISCONNECT_LEN,
                "the seat rejoins when its window ends"
            );
            assert!(report.replayed > 0, "the window must hand history back");
            assert_eq!(report.uncovered_ticks, 0, "the window covers the 2 s gap");
            assert!(
                report.converge_ticks <= CONVERGE_BAR_TICKS,
                "rejoin converge {} over the {CONVERGE_BAR_TICKS}-tick (5 s) bar",
                report.converge_ticks
            );
            assert!(
                report.replays_refused > 0,
                "the window refuses the generations the seat already holds"
            );
            // Seeded, not sampled: the same rejoin repeats exactly.
            let again = run_rejoin(7, TEST_TICKS, players).expect("rejoin again");
            assert_eq!(report, again);
        }
    }

    #[test]
    fn wan_rejoin_past_the_window_blows_the_bar_loudly() {
        // A gap wider than the window's coverage leaves the uncovered ticks
        // to recover, so the 5 s bar refuses with both sides.
        let coverage = REPLAY_WINDOW_GENS * SNAPSHOT_EVERY_TICKS;
        let away = coverage + CONVERGE_BAR_TICKS;
        let uncovered = away - coverage;
        assert_eq!(
            run_rejoin_away(7, 600, SESSION_PLAYERS_MIN, away),
            Err(WanError::SlowConverge {
                got: uncovered + RTT_TICKS,
                bar: CONVERGE_BAR_TICKS,
            }),
            "a gap past the window must refuse loudly"
        );
        // Exactly the coverage converges in one transit.
        let exact = run_rejoin_away(7, 600, SESSION_PLAYERS_MIN, coverage).expect("covered gap");
        assert_eq!(exact.uncovered_ticks, 0);
        assert_eq!(exact.converge_ticks, RTT_TICKS);
        assert_eq!(exact.replayed, REPLAY_WINDOW_GENS);
    }

    #[test]
    fn wan_join_token_is_short_lived_and_refuses_deny_by_default() {
        let session = 7 ^ Fault::Disconnect.salt();
        let viewer = 3u32;
        let challenge = mint_challenge(session, 16);
        let token = JoinToken::mint(7, session, viewer, 0);
        assert_eq!(token.ttl_ticks, TOKEN_TTL_TICKS);
        assert!(token.is_live(TOKEN_TTL_TICKS), "inside the token lifetime");
        assert!(
            !token.is_live(TOKEN_TTL_TICKS + 1),
            "past the token lifetime"
        );
        assert!(!token.is_live(u32::MAX), "a wrapped clock reads expired");
        let answer = token.answer(challenge);
        token
            .verify(session, viewer, 10, challenge, answer)
            .expect("a live token with the right answer verifies");
        let refused = |why| WanError::TokenRefused { viewer, why };
        assert_eq!(
            token.verify(session ^ 1, viewer, 10, challenge, answer),
            Err(refused("another session"))
        );
        assert_eq!(
            token.verify(session, viewer + 1, 10, challenge, answer),
            Err(WanError::TokenRefused {
                viewer: viewer + 1,
                why: "another seat"
            }),
            "the refusal names the seat that presented the token"
        );
        assert_eq!(
            token.verify(session, viewer, TOKEN_TTL_TICKS + 1, challenge, answer),
            Err(refused("expired"))
        );
        assert_eq!(
            token.verify(session, viewer, 10, challenge, answer ^ 1),
            Err(refused("wrong answer"))
        );
        assert_eq!(
            token.verify(session, viewer, 10, challenge ^ 1, answer),
            Err(refused("wrong answer"))
        );
        assert!(format!("{}", refused("expired")).contains("expired"));
    }

    #[test]
    fn wan_replay_window_refuses_duplicates_and_stale_sequences() {
        let mut window = ReplayWindow::new(0, 600);
        assert_eq!(window.held(), REPLAY_WINDOW_GENS);
        assert_eq!(
            window.coverage_ticks(),
            REPLAY_WINDOW_GENS * SNAPSHOT_EVERY_TICKS
        );
        assert_eq!(
            window.handoff(DISCONNECT_LEN),
            (DISCONNECT_LEN / SNAPSHOT_EVERY_TICKS, 0),
            "the 2 s gap is inside the window"
        );
        assert!(window.accept(0), "the first arrival is new");
        assert!(!window.accept(0), "a duplicated datagram refuses");
        assert!(window.accept(6), "a newer generation advances the window");
        let far = 6 + REPLAY_SEQ_BITS;
        assert!(window.accept(far), "a jump past the window resets it");
        assert!(!window.accept(6), "a sequence behind the window refuses");
        assert!(!window.accept(far), "the reset forgot the old window");
        assert!(window.accept(far + 6), "newer still accepts");
        // A lane with little history holds only what was published.
        let short = ReplayWindow::new(5, 5);
        assert_eq!(short.held(), 1);
        assert_eq!(short.coverage_ticks(), SNAPSHOT_EVERY_TICKS);
        assert_eq!(
            short.handoff(600),
            (1, 600 - SNAPSHOT_EVERY_TICKS),
            "the uncovered part is what the client must recover"
        );
    }

    #[test]
    fn wan_version_resync_matrix_16_to_32_posts_converge_and_overhead() {
        // Accept: the ON4 fault matrix at 16-32 converges on every
        // checkpoint, with a per-fault converge time and byte-overhead row.
        let clean = record_tape(7, BAND_TICKS).expect("clean tape must record");
        for players in SESSION_PLAYERS_MIN..=SESSION_PLAYERS_MAX {
            let matrix = ALL_FAULTS.map(|fault| {
                run_version_fault_sized(7, fault, BAND_TICKS, players)
                    .unwrap_or_else(|err| panic!("{players} players {fault}: {err}"))
            });
            let mut resynced = 0u32;
            for (row, fault) in matrix.iter().zip(ALL_FAULTS) {
                assert_eq!(row.ah4.fault, fault, "matrix keeps report order");
                assert_eq!(
                    row.ah4.players as usize, players,
                    "report names its session"
                );
                assert_eq!(
                    row.ah4.hash, clean.hash,
                    "{fault} at {players} players must hold the checkpoint"
                );
                assert_eq!(row.checkpoint_misses, 0, "{fault}: no miss is allowed");
                assert!(
                    row.version_converge_ticks <= row.ah4.converge_ticks,
                    "{fault}: the pool cannot converge slower than the cadence, got {} vs {}",
                    row.version_converge_ticks,
                    row.ah4.converge_ticks
                );
                assert!(row.version_converge_ticks <= CONVERGE_BAR_TICKS);
                assert!(row.pool_peak_bytes as usize <= EMERGENCY_POOL_BYTES);
                assert_eq!(row.pool_refused, 0, "one generation must fit the reserve");
                // The byte row is arithmetic on the tape, not a guess: every
                // resync sends one generation plus its stamp and pool ticket,
                // and the reserve is charged once.
                let per_resync = row.generation_bytes
                    + u64::try_from(STAMP_HEADER_BYTES).expect("header fits")
                    + u64::try_from(POOL_TICKET_BYTES).expect("ticket fits");
                let reserve = if row.pool_armed > 0 {
                    u64::try_from(EMERGENCY_POOL_BYTES).expect("reserve fits")
                } else {
                    0
                };
                assert_eq!(
                    row.version_bytes,
                    u64::from(row.resyncs) * per_resync + reserve,
                    "{fault}: version bytes must match the posted ledger"
                );
                assert_eq!(row.ah4_bytes, u64::from(row.resyncs) * row.generation_bytes);
                let expected = row
                    .version_bytes
                    .saturating_sub(row.ah4_bytes)
                    .checked_mul(10_000)
                    .and_then(|extra| extra.checked_div(row.ah4_bytes))
                    .map_or(0, |pct| u32::try_from(pct).expect("percent fits"));
                assert_eq!(row.overhead_pct_x100, expected);
                if row.resyncs == 0 {
                    assert_eq!(row.version_bytes, 0, "{fault}: nothing to resync");
                    assert_eq!(row.overhead_pct_x100, 0);
                    assert_eq!(row.pool_armed, 0);
                } else {
                    resynced += 1;
                    assert!(row.pool_armed > 0, "{fault}: the pool arms on a resync");
                }
                println!("{}", row.summary());
            }
            assert!(
                resynced > 0,
                "{players} players must exercise the resync path"
            );
        }
    }

    #[test]
    fn wan_version_resync_matches_the_ah4_resync_ledger() {
        // Both schemes read the same delivery plan, so the resync count must
        // match the AH4 report cell for cell; the byte rows compare against it.
        for fault in ALL_FAULTS {
            let ah4 = run_fault_sized(7, fault, BAND_TICKS, SESSION_PLAYERS_MIN)
                .expect("AH4 baseline runs");
            let row = run_version_fault_sized(7, fault, BAND_TICKS, SESSION_PLAYERS_MIN)
                .expect("version path runs");
            assert_eq!(row.resyncs, ah4.resyncs, "{fault}: one resync ledger");
            assert_eq!(row.ah4.hash, ah4.hash);
            assert_eq!(
                row.ah4.digest_key(),
                ah4.digest_key(),
                "{fault}: the AH4 baseline row is carried whole (resim p99 is wall clock)"
            );
        }
    }

    #[test]
    fn wan_version_resync_kill_gate_names_numbers() {
        let row = run_version_fault_sized(7, Fault::Disconnect, BAND_TICKS, SESSION_PLAYERS_MIN)
            .expect("version path runs");
        assert_eq!(row.accepted(), row.check().is_ok());
        assert_eq!(row.checkpoint_misses, 0);
        // A blown overhead bar fails loudly with both sides.
        let mut killed = row;
        killed.overhead_pct_x100 = OVERHEAD_BAR_PCT * 100 + 1;
        assert_eq!(
            killed.check(),
            Err(WanError::Overhead {
                pct_x100: OVERHEAD_BAR_PCT * 100 + 1,
                bar_pct: OVERHEAD_BAR_PCT,
                version_bytes: killed.version_bytes,
                ah4_bytes: killed.ah4_bytes,
            }),
            "over the bar must kill the spike with numbers"
        );
        assert!(!killed.accepted());
        assert!(killed.summary().contains("KILLED"));
        // A single missed checkpoint kills it too, ahead of the byte gate.
        let mut missed = row;
        missed.checkpoint_misses = 1;
        assert_eq!(
            missed.check(),
            Err(WanError::Miss {
                misses: 1,
                tick: BAND_TICKS - 1,
            }),
            "any miss kills the spike with numbers"
        );
        assert!(row.check().is_ok() || matches!(row.check(), Err(WanError::Overhead { .. })));
    }

    #[test]
    fn wan_version_stamp_and_pool_are_bounded() {
        // The stamp is the generation version: it covers what the client
        // holds, so a covered resync sends nothing.
        let stamp = VersionStamp::of(3, 12);
        assert!(stamp.covers(6));
        assert!(stamp.covers(12));
        assert!(!stamp.covers(18));
        let mut pool = EmergencyPool::new(32);
        let mut ledger = VersionLedger::default();
        ledger.urgent(&mut pool, 0, 12, Some(12), 12, 24);
        assert_eq!(ledger.resyncs, 1);
        assert_eq!(ledger.armed, 0, "a covered stamp arms nothing");
        assert_eq!(ledger.bytes, 0);
        assert_eq!(pool.used(), 0);
        // The pool admits one payload, refuses the next with both sides, and
        // gives the first back on converge.
        pool.admit(24).expect("24 B fits a 32 B reserve");
        assert_eq!(pool.peak(), 24);
        assert_eq!(
            pool.admit(24),
            Err(WanError::PoolRefused {
                need: 24,
                capacity: 32,
            })
        );
        assert_eq!(pool.refused(), 1);
        assert_eq!(pool.used(), 24);
        pool.release(24);
        assert_eq!(pool.used(), 0);
        assert!(format!(
            "{}",
            WanError::PoolRefused {
                need: 24,
                capacity: 32
            }
        )
        .contains("24"));
        // A refused admission falls back to the cadence: one baseline plus
        // the stamp, no ticket, no converge saved.
        let mut tiny = EmergencyPool::new(8);
        let mut fallback = VersionLedger::default();
        fallback.urgent(&mut tiny, 0, 18, Some(12), 30, 24);
        assert_eq!(fallback.refused, 1);
        assert_eq!(
            fallback.bytes,
            24 + u64::try_from(STAMP_HEADER_BYTES).expect("header fits")
        );
        assert_eq!(fallback.wait_saved, 0, "a refusal waits for the cadence");
        // A pooled resync costs the payload, the stamp and the ticket, and
        // saves the cadence wait the AH4 re-baseline pays.
        let mut roomy = EmergencyPool::new(64);
        let mut pooled = VersionLedger::default();
        pooled.urgent(&mut roomy, 0, 18, Some(12), 30, 24);
        assert_eq!(pooled.refused, 0);
        assert_eq!(
            pooled.bytes,
            24 + u64::try_from(STAMP_HEADER_BYTES).expect("header fits")
                + u64::try_from(POOL_TICKET_BYTES).expect("ticket fits")
        );
        assert_eq!(pooled.wait_saved, cadence_wait(0, 30));
    }

    #[test]
    fn wan_soak_sized_shapes_refuse_loudly() {
        assert_eq!(
            run_matrix_sized(7, 0, 16),
            Err(WanError::BadTicks { got: 0 })
        );
        assert_eq!(
            run_fault_sized(7, Fault::Loss, 240, 0),
            Err(WanError::BadPlayers {
                got: 0,
                min: 1,
                max: PLAYERS,
            })
        );
        assert_eq!(
            run_matrix_sized(7, 240, PLAYERS + 1),
            Err(WanError::BadPlayers {
                got: PLAYERS + 1,
                min: 1,
                max: PLAYERS,
            })
        );
        assert_eq!(
            run_rejoin(7, 240, 0),
            Err(WanError::BadPlayers {
                got: 0,
                min: 1,
                max: PLAYERS,
            })
        );
        assert_eq!(run_rejoin(7, 0, 16), Err(WanError::BadTicks { got: 0 }));
        // The legacy entry points still soak the full siege session.
        let siege = run_fault(7, Fault::Latency, 60).expect("short soak converges");
        assert_eq!(siege.players as usize, PLAYERS);
        assert!(format!(
            "{}",
            WanError::BadPlayers {
                got: 3,
                min: 1,
                max: PLAYERS
            }
        )
        .contains('3'));
    }
}
