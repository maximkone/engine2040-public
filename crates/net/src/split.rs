//! split: one played bout across two machines over real UDP.
//!
//! Purpose: the real-hardware legs of the two-machine proofs, over a LAN and
//! across two networks through the self-hosted relay. Machine A plays the
//! dedicated host plus seat 0, machine B plays seat 1. Each machine drives
//! the [`LoopbackWire`] links it sends on; once per bout step the two trade,
//! through a [`Courier`] slot exchange, every frame due on a link that
//! crosses between them. Owner: net lane. Units: bout steps, bytes.
//! Determinism: the fault links run in step time and both machines build
//! them from the same config, so a split bout is the same pure function of
//! config as [`crate::played::run_loopback`] and posts the same digest; real
//! loss costs wall time, never a frame.
//! Errors: [`SplitError`]; decoders refuse hostile bytes, never panic.

use std::net::SocketAddr;

use engine_arena::Checkpoint;
use engine_core::mix64;

use crate::courier::{Courier, PunchReport, Reader, Road, RunNet, SplitError};
use crate::link::{
    Frame, LinkProfile, LinkReport, LinkStats, LoopbackWire, Peer, Wire, DELAY_BUCKETS,
};
use crate::played::{
    loopback_wire, BoutConfig, BoutError, BoutPart, BoutReport, DropScript, SeatOutcome, SeatStats,
    BOUT_BUILD, ROLLBACK_BUCKETS, SEAT_COUNT,
};
use crate::relay::{gather_candidates, Candidate, CandidateKind, InviteCode};

const TAG_HELLO: u8 = 0;
const TAG_SETUP: u8 = 1;
const TAG_END: u8 = 2;
/// Salt of the relay ids derived from an invite code.
const INVITE_SALT: u64 = 0x5350_4C49_5452_454C;

/// Which machine of a split bout this is.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Side {
    /// Machine A: the dedicated host plus seat 0.
    Host,
    /// Machine B: seat 1.
    Join,
}

impl Side {
    /// Whether `peer` plays on this machine.
    #[must_use]
    pub fn plays(self, peer: Peer) -> bool {
        matches!(
            (self, peer),
            (Self::Host, Peer::Server | Peer::Client(0)) | (Self::Join, Peer::Client(1))
        )
    }

    /// The seats this machine plays.
    #[must_use]
    pub fn seats(self) -> &'static [u8] {
        match self {
            Self::Host => &[0],
            Self::Join => &[1],
        }
    }
}

const _: () = assert!(SEAT_COUNT == 2, "the split pairs exactly two seats");

/// Every lane in [`LoopbackWire`] report order: uplinks, then downlinks.
const LANES: [(Peer, Peer); 4] = [
    (Peer::Client(0), Peer::Server),
    (Peer::Client(1), Peer::Server),
    (Peer::Server, Peer::Client(0)),
    (Peer::Server, Peer::Client(1)),
];

/// One machine's side of the bout wire. Links whose sender plays here run
/// on the local [`LoopbackWire`]; frames on links whose sender plays on the
/// other machine arrive through [`Self::deliver`], one step at a time.
pub struct SplitWire {
    inner: LoopbackWire,
    side: Side,
    remote: Vec<(Peer, Peer, Frame)>,
    remote_stats: Vec<(Peer, Peer, LinkStats)>,
}

impl SplitWire {
    /// Split `inner` for `side`. Both machines pass the same
    /// [`loopback_wire`], so every link draws the same stream wherever it runs.
    #[must_use]
    pub fn new(inner: LoopbackWire, side: Side) -> Self {
        Self {
            inner,
            side,
            remote: Vec::new(),
            remote_stats: Vec::new(),
        }
    }

    /// Frames due this step on links from here to the other machine, in
    /// lane order. Call once per step, before the step plays: every link
    /// delays at least one step, so nothing sent this step is due yet.
    pub fn outbound(&mut self) -> Vec<(Peer, Peer, Frame)> {
        let mut out = Vec::new();
        for (from, to) in LANES {
            if !self.side.plays(from) || self.side.plays(to) {
                continue;
            }
            if let Some(link) = self.inner.link_mut(from, to) {
                out.extend(link.due().into_iter().map(|frame| (from, to, frame)));
            }
        }
        out
    }

    /// Take the other machine's frames due this step.
    ///
    /// # Errors
    ///
    /// [`SplitError::BadBlob`] when a frame rides a lane that does not run
    /// from the other machine to this one.
    pub fn deliver(&mut self, frames: Vec<(Peer, Peer, Frame)>) -> Result<(), SplitError> {
        let foreign = |(from, to): (Peer, Peer)| {
            !self.side.plays(from) && self.side.plays(to) && LANES.contains(&(from, to))
        };
        if !frames.iter().all(|(from, to, _)| foreign((*from, *to))) {
            return Err(SplitError::BadBlob("lane"));
        }
        self.remote = frames;
        Ok(())
    }

    /// Counters of the links this machine sends on, in lane order.
    #[must_use]
    pub fn local_stats(&self) -> Vec<(Peer, Peer, LinkStats)> {
        self.inner
            .report()
            .into_iter()
            .filter(|link| self.side.plays(link.from))
            .map(|link| (link.from, link.to, link.stats))
            .collect()
    }

    /// Counters of the links the other machine sends on, from its final blob.
    pub fn set_remote_stats(&mut self, stats: Vec<(Peer, Peer, LinkStats)>) {
        self.remote_stats = stats;
    }
}

impl Wire for SplitWire {
    fn datagram(&mut self, from: Peer, to: Peer, bytes: &[u8]) {
        if self.side.plays(from) {
            self.inner.datagram(from, to, bytes);
        }
    }

    fn stream(&mut self, from: Peer, to: Peer, bytes: &[u8]) {
        if self.side.plays(from) {
            self.inner.stream(from, to, bytes);
        }
    }

    fn recv(&mut self, at: Peer) -> Vec<(Peer, Frame)> {
        let mut out = Vec::new();
        for (from, to) in LANES {
            if to != at {
                continue;
            }
            if self.side.plays(from) {
                if let Some(link) = self.inner.link_mut(from, to) {
                    out.extend(link.due().into_iter().map(|frame| (from, frame)));
                }
                continue;
            }
            let (mine, rest) = std::mem::take(&mut self.remote)
                .into_iter()
                .partition(|(sender, receiver, _)| (*sender, *receiver) == (from, to));
            self.remote = rest;
            out.extend(
                mine.into_iter()
                    .map(|(sender, _, frame): (Peer, Peer, Frame)| (sender, frame)),
            );
        }
        out
    }

    fn tick(&mut self) {
        self.inner.tick();
    }

    fn report(&self) -> Vec<LinkReport> {
        let mut links = self.inner.report();
        for link in &mut links {
            if self.side.plays(link.from) {
                continue;
            }
            if let Some((_, _, stats)) = self
                .remote_stats
                .iter()
                .find(|(from, to, _)| (*from, *to) == (link.from, link.to))
            {
                link.stats = *stats;
            }
        }
        links
    }
}

/// Relay ids of an invite's two machines, host first: distinct, never zero.
#[must_use]
pub fn invite_ids(code: &InviteCode) -> (u64, u64) {
    let key = code
        .as_bytes()
        .iter()
        .fold(0u64, |acc, byte| (acc << 8) | u64::from(*byte));
    let host = mix64(key ^ INVITE_SALT).max(1);
    (host, host.wrapping_add(1).max(1))
}

fn peer_code(peer: Peer) -> u8 {
    match peer {
        Peer::Server => 0,
        Peer::Client(seat) => seat.saturating_add(1),
    }
}

fn read_peer(r: &mut Reader<'_>) -> Result<Peer, SplitError> {
    match r.u8()? {
        0 => Ok(Peer::Server),
        code @ 1..=2 => Ok(Peer::Client(code - 1)),
        _ => Err(SplitError::BadBlob("peer")),
    }
}

fn put_count(out: &mut Vec<u8>, count: usize) {
    out.extend_from_slice(&u32::try_from(count).unwrap_or(u32::MAX).to_le_bytes());
}

fn put_words(out: &mut Vec<u8>, words: &[u32]) {
    for word in words {
        out.extend_from_slice(&word.to_le_bytes());
    }
}

/// Step blob: every frame due on the crossing links, sender and lane tagged.
fn encode_frames(frames: &[(Peer, Peer, Frame)]) -> Vec<u8> {
    let mut out = Vec::new();
    put_count(&mut out, frames.len());
    for (from, to, frame) in frames {
        let (lane, bytes) = match frame {
            Frame::Datagram(bytes) => (0u8, bytes),
            Frame::Stream(bytes) => (1u8, bytes),
        };
        out.extend_from_slice(&[peer_code(*from), peer_code(*to), lane]);
        put_count(&mut out, bytes.len());
        out.extend_from_slice(bytes);
    }
    out
}

fn decode_frames(bytes: &[u8]) -> Result<Vec<(Peer, Peer, Frame)>, SplitError> {
    let mut r = Reader::new(bytes);
    let count = r.count()?;
    let mut out = Vec::with_capacity(count.min(256));
    for _ in 0..count {
        let (from, to, lane) = (read_peer(&mut r)?, read_peer(&mut r)?, r.u8()?);
        let len = r.count()?;
        let body = r.take(len)?.to_vec();
        let frame = match lane {
            0 => Frame::Datagram(body),
            1 => Frame::Stream(body),
            _ => return Err(SplitError::BadBlob("lane")),
        };
        out.push((from, to, frame));
    }
    r.end()?;
    Ok(out)
}

fn put_seat_stats(out: &mut Vec<u8>, s: &SeatStats) {
    put_words(out, &[s.applied, s.rejected]);
    put_words(out, &s.rollback);
    put_words(
        out,
        &[s.rollback_max, s.over_cap, s.catchups, s.resync_depth],
    );
    out.push(u8::from(s.last_catchup_step.is_some()));
    put_words(
        out,
        &[
            s.last_catchup_step.unwrap_or(0),
            s.gap_resyncs,
            s.input_delay,
        ],
    );
    out.push(u8::from(s.granted));
}

fn read_seat_stats(r: &mut Reader<'_>) -> Result<SeatStats, SplitError> {
    let [applied, rejected] = r.words()?;
    let rollback = r.words::<ROLLBACK_BUCKETS>()?;
    let [rollback_max, over_cap, catchups, resync_depth] = r.words()?;
    let caught = r.flag()?;
    let [last, gap_resyncs, input_delay] = r.words()?;
    Ok(SeatStats {
        applied,
        rejected,
        rollback,
        rollback_max,
        over_cap,
        input_delay,
        catchups,
        resync_depth,
        last_catchup_step: caught.then_some(last),
        gap_resyncs,
        granted: r.flag()?,
    })
}

fn put_link_stats(out: &mut Vec<u8>, s: &LinkStats) {
    put_words(
        out,
        &[
            s.sent,
            s.dropped,
            s.outage,
            s.delivered,
            s.reordered,
            s.streams,
        ],
    );
    out.extend_from_slice(&s.bytes.to_le_bytes());
    put_words(out, &s.delay);
}

fn read_link_stats(r: &mut Reader<'_>) -> Result<LinkStats, SplitError> {
    let [sent, dropped, outage, delivered, reordered, streams] = r.words()?;
    Ok(LinkStats {
        sent,
        dropped,
        outage,
        delivered,
        reordered,
        streams,
        bytes: r.u64()?,
        delay: r.words::<DELAY_BUCKETS>()?,
    })
}

/// Final blob of machine B: its seats' outcomes and its links' counters.
fn encode_final(outcomes: &[SeatOutcome], links: &[(Peer, Peer, LinkStats)]) -> Vec<u8> {
    let mut out = Vec::new();
    put_count(&mut out, outcomes.len());
    for seat in outcomes {
        out.push(seat.seat);
        out.extend_from_slice(&seat.token.to_le_bytes());
        out.push(seat.slot);
        out.extend_from_slice(&seat.mispredict_ticks.to_le_bytes());
        put_seat_stats(&mut out, &seat.stats);
        put_count(&mut out, seat.checkpoints.len());
        for checkpoint in &seat.checkpoints {
            out.extend_from_slice(&checkpoint.tick.to_le_bytes());
            out.extend_from_slice(&checkpoint.hash.to_le_bytes());
        }
    }
    put_count(&mut out, links.len());
    for (from, to, stats) in links {
        out.extend_from_slice(&[peer_code(*from), peer_code(*to)]);
        put_link_stats(&mut out, stats);
    }
    out
}

/// Seat outcomes and link counters of a final blob.
type FinalBlob = (Vec<SeatOutcome>, Vec<(Peer, Peer, LinkStats)>);

fn decode_final(bytes: &[u8]) -> Result<FinalBlob, SplitError> {
    let mut r = Reader::new(bytes);
    let seats = r.count()?;
    let mut outcomes = Vec::with_capacity(seats.min(SEAT_COUNT));
    for _ in 0..seats {
        let (seat, token, slot) = (r.u8()?, r.u64()?, r.u8()?);
        let mispredict_ticks = r.u64()?;
        let stats = read_seat_stats(&mut r)?;
        let count = r.count()?;
        let mut checkpoints = Vec::with_capacity(count.min(1 << 16));
        for _ in 0..count {
            checkpoints.push(Checkpoint {
                tick: r.u32()?,
                hash: r.u64()?,
            });
        }
        outcomes.push(SeatOutcome {
            seat,
            token,
            slot,
            checkpoints,
            mispredict_ticks,
            stats,
        });
    }
    let count = r.count()?;
    let mut links = Vec::with_capacity(count.min(LANES.len()));
    for _ in 0..count {
        let (from, to) = (read_peer(&mut r)?, read_peer(&mut r)?);
        links.push((from, to, read_link_stats(&mut r)?));
    }
    r.end()?;
    Ok((outcomes, links))
}

/// One bout of a split session: the config, both link profiles, the road.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct RunSpec {
    /// The bout.
    pub cfg: BoutConfig,
    /// Every client-to-server link, in step time.
    pub up: LinkProfile,
    /// Every server-to-client link, in step time.
    pub down: LinkProfile,
    /// The road the bout's bytes take between the machines.
    pub road: Road,
}

impl RunSpec {
    fn check(&self) -> Result<(), SplitError> {
        self.cfg.validate()?;
        if self.up.latency == 0 || self.down.latency == 0 {
            return Err(SplitError::Bout(BoutError::BadConfig("latency")));
        }
        Ok(())
    }
}

fn put_build(out: &mut Vec<u8>) {
    out.extend_from_slice(&BOUT_BUILD.version.to_le_bytes());
    out.extend_from_slice(&BOUT_BUILD.rules.to_le_bytes());
}

fn check_build(r: &mut Reader<'_>) -> Result<(), SplitError> {
    if (r.u16()?, r.u64()?) == (BOUT_BUILD.version, BOUT_BUILD.rules) {
        Ok(())
    } else {
        Err(SplitError::Build)
    }
}

fn hello() -> Vec<u8> {
    let mut out = vec![TAG_HELLO];
    put_build(&mut out);
    out
}

fn check_hello(bytes: &[u8]) -> Result<(), SplitError> {
    let mut r = Reader::new(bytes);
    if r.u8()? != TAG_HELLO {
        return Err(SplitError::BadBlob("hello"));
    }
    check_build(&mut r)?;
    r.end()
}

fn put_profile(out: &mut Vec<u8>, p: &LinkProfile) {
    put_words(out, &[p.latency, p.jitter, p.loss_pct, p.reorder_pct]);
}

fn read_profile(r: &mut Reader<'_>) -> Result<LinkProfile, SplitError> {
    let [latency, jitter, loss_pct, reorder_pct] = r.words()?;
    Ok(LinkProfile {
        latency,
        jitter,
        loss_pct,
        reorder_pct,
    })
}

fn encode_setup(spec: &RunSpec) -> Vec<u8> {
    let cfg = &spec.cfg;
    let mut out = vec![TAG_SETUP];
    put_build(&mut out);
    out.push(u8::from(spec.road == Road::Relay));
    out.extend_from_slice(&cfg.seed.to_le_bytes());
    out.push(cfg.fighters);
    put_words(
        &mut out,
        &[cfg.ticks, cfg.half_extent_m.to_bits(), cfg.hold],
    );
    out.push(cfg.drop.seat);
    put_words(&mut out, &[cfg.drop.at, cfg.drop.back]);
    for token in cfg.tokens {
        out.extend_from_slice(&token.to_le_bytes());
    }
    put_profile(&mut out, &spec.up);
    put_profile(&mut out, &spec.down);
    out
}

fn read_setup(r: &mut Reader<'_>) -> Result<RunSpec, SplitError> {
    check_build(r)?;
    let road = if r.flag()? { Road::Relay } else { Road::Direct };
    let (seed, fighters) = (r.u64()?, r.u8()?);
    let [ticks, extent, hold] = r.words()?;
    let seat = r.u8()?;
    let [at, back] = r.words()?;
    let tokens = [r.u64()?, r.u64()?];
    Ok(RunSpec {
        cfg: BoutConfig {
            seed,
            fighters,
            ticks,
            half_extent_m: f32::from_bits(extent),
            hold,
            drop: DropScript { seat, at, back },
            tokens,
        },
        up: read_profile(r)?,
        down: read_profile(r)?,
        road,
    })
}

/// Machine A's verdict on one bout, sent to machine B at the end.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct RunVerdict {
    /// The bout digest.
    pub digest: u64,
    /// The bout converged.
    pub converged: bool,
}

impl RunVerdict {
    /// The verdict of one report.
    #[must_use]
    pub fn of(report: &BoutReport) -> Self {
        Self {
            digest: report.digest(),
            converged: report.converged(),
        }
    }
}

fn encode_end(verdicts: &[RunVerdict]) -> Vec<u8> {
    let mut out = vec![TAG_END];
    put_count(&mut out, verdicts.len());
    for verdict in verdicts {
        out.extend_from_slice(&verdict.digest.to_le_bytes());
        out.push(u8::from(verdict.converged));
    }
    out
}

/// What machine A tells machine B at a slot boundary.
enum Control {
    Setup(RunSpec),
    End(Vec<RunVerdict>),
}

fn decode_control(bytes: &[u8]) -> Result<Control, SplitError> {
    let mut r = Reader::new(bytes);
    let control = match r.u8()? {
        TAG_SETUP => Control::Setup(read_setup(&mut r)?),
        TAG_END => {
            let count = r.count()?;
            let mut verdicts = Vec::with_capacity(count.min(64));
            for _ in 0..count {
                verdicts.push(RunVerdict {
                    digest: r.u64()?,
                    converged: r.flag()?,
                });
            }
            Control::End(verdicts)
        }
        _ => return Err(SplitError::BadBlob("control")),
    };
    r.end()?;
    Ok(control)
}

/// Play every step of one bout in lockstep with the other machine: trade
/// the frames due on the crossing links, then play this machine's parts.
fn lockstep(
    courier: &mut Courier,
    part: &mut BoutPart,
    wire: &mut SplitWire,
) -> Result<(), SplitError> {
    for step in 0..part.steps() {
        let theirs = courier.exchange(&encode_frames(&wire.outbound()))?;
        wire.deliver(decode_frames(&theirs)?)?;
        part.step(step, wire)?;
        wire.tick();
    }
    Ok(())
}

/// What machine A played: every finished bout, its wall cost, and the
/// failure that stopped the session early, if any.
#[derive(Clone, PartialEq, Debug)]
pub struct HostSession {
    /// Reports of the finished bouts, in run order.
    pub reports: Vec<BoutReport>,
    /// Wall cost of each finished bout.
    pub nets: Vec<RunNet>,
    /// The failure that ended the session before every run finished.
    pub failed: Option<SplitError>,
}

/// Play every run from machine A: send the run's setup, play the host and
/// seat 0 in lockstep with machine B, fold B's seat into the report. Stops
/// at the first failure and keeps every bout finished before it.
pub fn host_session(courier: &mut Courier, runs: &[RunSpec]) -> HostSession {
    let mut session = HostSession {
        reports: Vec::new(),
        nets: Vec::new(),
        failed: None,
    };
    for spec in runs {
        match host_run(courier, spec) {
            Ok((report, net)) => {
                session.reports.push(report);
                session.nets.push(net);
            }
            Err(err) => {
                session.failed = Some(err);
                return session;
            }
        }
    }
    let verdicts: Vec<RunVerdict> = session.reports.iter().map(RunVerdict::of).collect();
    let ended = courier
        .exchange(&encode_end(&verdicts))
        .and_then(|bytes| check_hello(&bytes))
        .and_then(|()| courier.linger());
    session.failed = ended.err();
    session
}

fn host_run(courier: &mut Courier, spec: &RunSpec) -> Result<(BoutReport, RunNet), SplitError> {
    spec.check()?;
    check_hello(&courier.exchange(&encode_setup(spec))?)?;
    courier.enter(spec.road)?;
    let mut part = BoutPart::new(&spec.cfg, true, Side::Host.seats())?;
    let mut wire = SplitWire::new(loopback_wire(&spec.cfg, spec.up, spec.down), Side::Host);
    lockstep(courier, &mut part, &mut wire)?;
    let (remote, stats) = decode_final(&courier.exchange(&[])?)?;
    let net = courier.finish_run();
    wire.set_remote_stats(stats);
    let mut seats = part.outcomes();
    seats.extend(remote);
    seats.sort_by_key(|seat| seat.seat);
    Ok((part.report(&seats, wire.report())?, net))
}

/// What machine B played: machine A's verdicts and each bout's wall cost.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct JoinSession {
    /// Machine A's verdict on every bout, in run order.
    pub verdicts: Vec<RunVerdict>,
    /// Wall cost of each bout.
    pub nets: Vec<RunNet>,
}

/// Play every run machine A sets up, as seat 1, until A ends the session.
///
/// # Errors
///
/// [`SplitError`] when the link, a blob or the bout fails.
pub fn join_session(courier: &mut Courier) -> Result<JoinSession, SplitError> {
    let mut nets = Vec::new();
    loop {
        match decode_control(&courier.exchange(&hello())?)? {
            Control::Setup(spec) => nets.push(join_run(courier, &spec)?),
            Control::End(verdicts) => {
                courier.linger()?;
                return Ok(JoinSession { verdicts, nets });
            }
        }
    }
}

fn join_run(courier: &mut Courier, spec: &RunSpec) -> Result<RunNet, SplitError> {
    spec.check()?;
    courier.enter(spec.road)?;
    let mut part = BoutPart::new(&spec.cfg, false, Side::Join.seats())?;
    let mut wire = SplitWire::new(loopback_wire(&spec.cfg, spec.up, spec.down), Side::Join);
    lockstep(courier, &mut part, &mut wire)?;
    courier.exchange(&encode_final(&part.outcomes(), &wire.local_stats()))?;
    Ok(courier.finish_run())
}

fn put_addr(out: &mut Vec<u8>, addr: Option<SocketAddr>) {
    let bytes = addr
        .map(|addr| {
            Candidate {
                kind: CandidateKind::Srflx,
                addr,
            }
            .encode()
        })
        .unwrap_or_default();
    out.push(u8::try_from(bytes.len()).unwrap_or(0));
    out.extend_from_slice(&bytes);
}

fn read_addr(r: &mut Reader<'_>) -> Result<Option<SocketAddr>, SplitError> {
    let len = usize::from(r.u8()?);
    if len == 0 {
        return Ok(None);
    }
    Ok(Some(Candidate::decode(r.take(len)?)?.addr))
}

fn encode_candidates(candidates: &[Candidate]) -> Vec<u8> {
    let mut out = Vec::new();
    put_build(&mut out);
    out.push(u8::try_from(candidates.len()).unwrap_or(u8::MAX));
    for candidate in candidates {
        let bytes = candidate.encode();
        out.push(u8::try_from(bytes.len()).unwrap_or(0));
        out.extend_from_slice(&bytes);
    }
    out
}

fn decode_candidates(bytes: &[u8]) -> Result<Vec<Candidate>, SplitError> {
    let mut r = Reader::new(bytes);
    check_build(&mut r)?;
    let count = r.u8()?;
    let mut out = Vec::with_capacity(usize::from(count));
    for _ in 0..count {
        let len = usize::from(r.u8()?);
        out.push(Candidate::decode(r.take(len)?)?);
    }
    r.end()?;
    Ok(out)
}

fn encode_punch(punch: &PunchReport) -> Vec<u8> {
    let mut out = vec![u8::from(punch.confirmed)];
    put_words(
        &mut out,
        &[
            punch.decided_after_ticks,
            punch.probes_sent,
            punch.punches_heard,
        ],
    );
    out.extend_from_slice(&punch.wall_us.to_le_bytes());
    put_addr(&mut out, punch.heard_from);
    out
}

fn decode_punch_report(bytes: &[u8]) -> Result<PunchReport, SplitError> {
    let mut r = Reader::new(bytes);
    let confirmed = r.flag()?;
    let [decided_after_ticks, probes_sent, punches_heard] = r.words()?;
    let report = PunchReport {
        confirmed,
        decided_after_ticks,
        wall_us: r.u64()?,
        probes_sent,
        punches_heard,
        heard_from: read_addr(&mut r)?,
    };
    r.end()?;
    Ok(report)
}

/// What the rendezvous through the relay found.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Rendezvous {
    /// This machine's public address per the relay's STUN echo.
    pub srflx: Option<SocketAddr>,
    /// This machine's candidates, best first.
    pub mine: Vec<Candidate>,
    /// The other machine's candidates, best first.
    pub theirs: Vec<Candidate>,
    /// This machine's punch.
    pub punch: PunchReport,
    /// The other machine's punch.
    pub their_punch: PunchReport,
    /// Direct only when both machines confirmed a two-way exchange.
    pub road: Road,
}

impl Rendezvous {
    /// Ticks the slower machine spent before its punch decided.
    #[must_use]
    pub fn decided_after_ticks(&self) -> u32 {
        self.punch
            .decided_after_ticks
            .max(self.their_punch.decided_after_ticks)
    }
}

/// Meet the other machine through the relay: register, learn the public
/// address, trade candidates, punch, then agree on the road. Both machines
/// call this with mirrored relay legs; the courier keeps the relay road.
///
/// # Errors
///
/// [`SplitError`] when the relay, the link or a blob fails.
pub fn rendezvous(courier: &mut Courier) -> Result<Rendezvous, SplitError> {
    let srflx = courier.register()?;
    let mine = gather_candidates(courier.host_candidate()?, srflx, None)?;
    let theirs = decode_candidates(&courier.exchange(&encode_candidates(&mine))?)?;
    let punch = courier.punch(&theirs)?;
    let their_punch = decode_punch_report(&courier.exchange(&encode_punch(&punch))?)?;
    let road = match punch.heard_from {
        Some(addr) if punch.confirmed && their_punch.confirmed => {
            courier.open_direct(addr);
            Road::Direct
        }
        _ => Road::Relay,
    };
    Ok(Rendezvous {
        srflx,
        mine,
        theirs,
        punch,
        their_punch,
        road,
    })
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::thread;

    use std::net::UdpSocket;

    use super::*;
    use crate::courier::{serve_relay, RelayLeg};
    use crate::played::{run_loopback, BOUT_TICKS};
    use crate::relay::{RelayConfig, RelayHub, PUNCH_TIMEOUT_TICKS};

    fn local() -> SocketAddr {
        SocketAddr::from(([127, 0, 0, 1], 0))
    }

    /// Short bout: 300 ticks, drop 100..160, both links on `profile`.
    fn short(seed: u64, profile: LinkProfile, road: Road) -> RunSpec {
        let mut cfg = BoutConfig::for_profile(seed, profile);
        cfg.ticks = BOUT_TICKS / 2;
        cfg.drop = DropScript {
            seat: 1,
            at: 100,
            back: 160,
        };
        RunSpec {
            cfg,
            up: profile,
            down: profile,
            road,
        }
    }

    fn assert_twin(reports: &[BoutReport], runs: &[RunSpec]) {
        assert_eq!(reports.len(), runs.len());
        for (report, spec) in reports.iter().zip(runs) {
            let twin = run_loopback(&spec.cfg, spec.up, spec.down).expect("loopback bout");
            assert!(report.converged(), "{report}");
            assert_eq!(*report, twin, "the split bout is the loopback bout");
            assert_eq!(report.digest(), twin.digest());
        }
    }

    #[test]
    fn split_lan_bouts_over_udp_equal_the_loopback_bouts() {
        let mut host = Courier::listen(local()).expect("listen");
        host.set_probes(16);
        let server = host.local_addr().expect("address");
        let runs = [
            short(2040, LinkProfile::LAN, Road::Direct),
            short(7, LinkProfile::WAN, Road::Direct),
        ];
        let join = thread::spawn(move || {
            let mut courier = Courier::connect(local(), server).expect("connect");
            courier.set_probes(16);
            join_session(&mut courier)
        });
        let session = host_session(&mut host, &runs);
        let joined = join.join().expect("join thread").expect("join session");
        assert_eq!(session.failed, None);
        assert_twin(&session.reports, &runs);
        let verdicts: Vec<RunVerdict> = session.reports.iter().map(RunVerdict::of).collect();
        assert_eq!(joined.verdicts, verdicts, "machine B hears every verdict");
        assert_eq!(joined.nets.len(), runs.len());
        let probe = &host.probes()[0];
        assert_eq!((probe.road, probe.sent), (Road::Direct, 16));
        assert!(!probe.rtt_us.is_empty(), "loopback probes come back");
        assert_eq!(
            session.nets[0].waits_us.len(),
            joined.nets[0].waits_us.len()
        );
    }

    #[test]
    fn split_blobs_roundtrip_and_refuse_junk() {
        let frames = vec![
            (
                Peer::Client(1),
                Peer::Server,
                Frame::Datagram(vec![1, 2, 3]),
            ),
            (Peer::Client(1), Peer::Server, Frame::Stream(vec![])),
        ];
        let bytes = encode_frames(&frames);
        assert_eq!(decode_frames(&bytes), Ok(frames));
        assert_eq!(
            decode_frames(&bytes[..bytes.len() - 1]),
            Err(SplitError::BadBlob("short"))
        );
        let mut trailing = bytes.clone();
        trailing.push(0);
        assert_eq!(
            decode_frames(&trailing),
            Err(SplitError::BadBlob("trailing"))
        );
        let mut stray = bytes;
        stray[4] = 9;
        assert_eq!(decode_frames(&stray), Err(SplitError::BadBlob("peer")));

        let spec = short(11, LinkProfile::WAN, Road::Relay);
        let setup = encode_setup(&spec);
        assert!(matches!(decode_control(&setup), Ok(Control::Setup(got)) if got == spec));
        let mut other_build = setup.clone();
        other_build[1] ^= 1;
        assert!(matches!(
            decode_control(&other_build),
            Err(SplitError::Build)
        ));
        assert_eq!(check_hello(&hello()), Ok(()));
        assert_eq!(check_hello(&setup), Err(SplitError::BadBlob("hello")));

        let report = run_loopback(&spec.cfg, spec.up, spec.down).expect("bout");
        let outcome = SeatOutcome {
            seat: 1,
            token: report.seats[1].token,
            slot: report.seats[1].slot,
            checkpoints: report.checkpoints.clone(),
            mispredict_ticks: report.seats[1].mispredict_ticks,
            stats: report.seats[1].stats,
        };
        let links: Vec<(Peer, Peer, LinkStats)> = report
            .links
            .iter()
            .map(|link| (link.from, link.to, link.stats))
            .collect();
        let blob = encode_final(std::slice::from_ref(&outcome), &links);
        assert_eq!(decode_final(&blob), Ok((vec![outcome], links)));
        assert!(decode_final(&blob[..blob.len() / 2]).is_err());

        let mut wire = SplitWire::new(loopback_wire(&spec.cfg, spec.up, spec.down), Side::Join);
        let own = vec![(Peer::Client(1), Peer::Server, Frame::Datagram(vec![1]))];
        assert_eq!(
            wire.deliver(own),
            Err(SplitError::BadBlob("lane")),
            "machine B never receives its own lane"
        );
    }

    #[test]
    fn split_online_bouts_punch_then_ride_both_roads_like_the_loopback() {
        let relay_sock = UdpSocket::bind(local()).expect("relay bind");
        let relay = relay_sock.local_addr().expect("relay address");
        let config = RelayConfig::default();
        let stop = Arc::new(AtomicBool::new(false));
        let hub_stop = Arc::clone(&stop);
        let hub = thread::spawn(move || {
            let mut hub = RelayHub::new(config);
            serve_relay(&relay_sock, &mut hub, |_| !hub_stop.load(Ordering::Relaxed))
                .expect("relay serves");
            hub.stats()
        });
        let mut state = 0x0A30_2040;
        let (host_id, join_id) = invite_ids(&InviteCode::mint(&mut state));
        let leg = move |me, peer| RelayLeg {
            addr: relay,
            token: config.token,
            me,
            peer,
        };
        let runs = [
            short(2040, LinkProfile::WAN, Road::Relay),
            short(2040, LinkProfile::WAN, Road::Direct),
        ];
        let join = thread::spawn(move || {
            let mut courier =
                Courier::via_relay(local(), leg(join_id, host_id)).expect("join bind");
            courier.set_probes(8);
            let met = rendezvous(&mut courier)?;
            Ok::<_, SplitError>((met, join_session(&mut courier)?))
        });
        let mut host = Courier::via_relay(local(), leg(host_id, join_id)).expect("host bind");
        host.set_probes(8);
        let met = rendezvous(&mut host).expect("rendezvous");
        assert_eq!(
            met.road,
            Road::Direct,
            "two sockets on one host punch through"
        );
        assert!(met.decided_after_ticks() <= PUNCH_TIMEOUT_TICKS);
        assert_eq!(
            met.srflx,
            Some(host.local_addr().expect("address")),
            "STUN echoes the source"
        );
        let session = host_session(&mut host, &runs);
        let (their_met, joined) = join.join().expect("join thread").expect("join side");
        stop.store(true, Ordering::Relaxed);
        let hub_stats = hub.join().expect("hub thread");
        assert_eq!(session.failed, None);
        assert_eq!(their_met.road, Road::Direct);
        assert_twin(&session.reports, &runs);
        assert_eq!(joined.verdicts.len(), 2);
        assert_eq!(session.nets[0].road, Road::Relay);
        assert_eq!(session.nets[1].road, Road::Direct);
        let roads: Vec<Road> = host.probes().iter().map(|probe| probe.road).collect();
        assert_eq!(
            roads,
            [Road::Relay, Road::Direct],
            "each fresh road is probed"
        );
        assert!(
            hub_stats.registered >= 2 && hub_stats.stun >= 2,
            "{hub_stats:?}"
        );
        assert_eq!(hub_stats.denied, 0);
    }
}
