//! online: the online played bout, QUIC through the relay.
//!
//! Purpose: the internet leg of the two-machine proof, run on the real ON1
//! and ON2 parts. Both clients join by invite code, the hole-punch machine
//! picks direct or relay inside [`PUNCH_TIMEOUT_TICKS`], and the bout of
//! [`crate::played`] then runs over a [`Wire`] whose lanes are QUIC datagrams
//! (inputs, state windows) and QUIC streams (join, catch-up). On the relay
//! path every frame crosses [`RelayServer`] as a [`RelayMsg::Forward`], two
//! hops each way. Owner: net lane. Units: ticks at 60 Hz, bytes.
//! Determinism: [`SimQuicFabric`] is seeded and tick-ordered; the same seed
//! gives the same loss, the same order and the same [`BoutReport::digest`].
//! Errors: [`BoutError`]. Only the real-NAT hole punch needs a second
//! network; everything else here runs on one machine.

use std::collections::{BTreeMap, VecDeque};
use std::net::SocketAddr;

use crate::link::{wire_len, Frame, LinkReport, LinkStats, Peer, Wire, DELAY_BUCKETS};
use crate::lobby::LOBBY_MAGIC;
use crate::played::{run_bout, BoutConfig, BoutError, BoutReport, BOUT_BUILD, SEATS, SEAT_COUNT};
use crate::quic::{ControlKind, InboundFrame, PeerId, SimQuicFabric, SimTransport, Transport};
use crate::receipt::{self, Obj};
use crate::relay::{
    decide_join, HolePunch, InviteCode, InviteTicket, NatKind, RelayConfig, RelayMsg, RelayServer,
    RelayStats, Signal, PUNCH_TIMEOUT_TICKS,
};

/// One-way QUIC hop latency in ticks (50 ms): the relay path is two hops.
pub const HOP_LATENCY_TICKS: u32 = 3;
/// Frame stamp: lane 1 + sender 1 + send tick 4 + flow sequence 4.
const STAMP_LEN: usize = 10;
const LANE_DATAGRAM: u8 = 0;
const LANE_STREAM: u8 = 1;
/// Endpoint index of the relay in the transport table.
const RELAY: usize = 3;
/// Relay client ids per endpoint index (server, seat 0, seat 1).
const RELAY_IDS: [u64; 3] = [1, 2, 3];

/// Which road the game bytes take.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Path {
    /// Peer to peer: the punch won.
    Direct,
    /// Through the self-hosted relay: the punch timed out or a NAT is symmetric.
    Relay,
}

/// The hole-punch verdict for both seats.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct PunchOutcome {
    /// NAT shape behind each seat.
    pub nats: [NatKind; SEAT_COUNT],
    /// Ticks the slowest seat spent before its machine decided.
    pub decided_after: u32,
    /// The road the bout takes: the relay when either seat needs it.
    pub path: Path,
}

/// Run each seat's punch machine to its decision. Direct checks succeed
/// unless the NAT is symmetric; the machine falls back after
/// [`PUNCH_TIMEOUT_TICKS`], so a decision always lands inside that bound.
#[must_use]
pub fn resolve_path(nats: [NatKind; SEAT_COUNT]) -> PunchOutcome {
    let mut decided_after = 0;
    let mut path = Path::Direct;
    for nat in nats {
        let mut punch = HolePunch::new(nat);
        punch.on_gathered();
        let mut ticks = 0;
        while !punch.is_direct() && !punch.needs_relay() {
            punch.tick(true);
            ticks += 1;
        }
        decided_after = decided_after.max(ticks);
        if punch.needs_relay() {
            path = Path::Relay;
        }
    }
    PunchOutcome {
        nats,
        decided_after,
        path,
    }
}

fn loopback(port: u16) -> SocketAddr {
    SocketAddr::from(([127, 0, 0, 1], port))
}

fn slot(peer: Peer) -> usize {
    match peer {
        Peer::Server => 0,
        Peer::Client(seat) => 1 + usize::from(seat),
    }
}

fn peer_at(index: usize) -> Peer {
    match index {
        0 => Peer::Server,
        n => Peer::Client(u8::try_from(n - 1).unwrap_or(u8::MAX)),
    }
}

/// One directed flow: sequence counters plus its measured stats.
struct Flow {
    next: u32,
    top: Option<u32>,
    stats: LinkStats,
}

/// A scripted outage on one seat's links.
#[derive(Clone, Copy)]
struct Outage {
    seat: u8,
    from: u32,
    to: u32,
}

/// The online [`Wire`]: QUIC lanes over [`SimQuicFabric`], optionally through
/// the relay. Every frame carries a stamp so the receiver measures the true
/// end-to-end transit and order.
pub struct OnlineWire {
    fabric: SimQuicFabric,
    ids: [PeerId; 4],
    transports: Vec<SimTransport>,
    relay: Option<RelayServer>,
    now: u32,
    pumped: Option<u32>,
    inbox: [VecDeque<(Peer, Frame)>; 3],
    flows: BTreeMap<(Peer, Peer), Flow>,
    outage: Option<Outage>,
    held: Vec<(Peer, Peer, u8, Vec<u8>)>,
}

impl OnlineWire {
    /// Open the link: server, two seats and the relay on one seeded fabric
    /// with `hop_latency` ticks per hop. On the relay path all three
    /// endpoints register under the relay token first.
    ///
    /// # Errors
    ///
    /// [`BoutError::Relay`] when the relay refuses a registration.
    pub fn open(path: Path, hop_latency: u32, seed: u64) -> Result<Self, BoutError> {
        let fabric = SimQuicFabric::new(hop_latency, seed);
        let addrs = [
            loopback(21_001),
            loopback(21_002),
            loopback(21_003),
            loopback(21_004),
        ];
        let transports: Vec<SimTransport> =
            addrs.iter().map(|addr| fabric.endpoint(*addr)).collect();
        let ids = addrs.map(PeerId);
        let mut wire = Self {
            fabric,
            ids,
            transports,
            relay: None,
            now: 0,
            pumped: None,
            inbox: [VecDeque::new(), VecDeque::new(), VecDeque::new()],
            flows: BTreeMap::new(),
            outage: None,
            held: Vec::new(),
        };
        if path == Path::Relay {
            wire.register_with_relay(hop_latency)?;
        }
        Ok(wire)
    }

    /// Current wire tick: the clock the outage window reads.
    #[must_use]
    pub fn now(&self) -> u32 {
        self.now
    }

    /// Script an outage: the seat's datagrams are lost and its streams wait
    /// for the link to come back.
    pub fn set_outage(&mut self, seat: u8, from: u32, to: u32) {
        self.outage = Some(Outage { seat, from, to });
    }

    /// Relay counters, when the relay carries the game.
    #[must_use]
    pub fn relay_stats(&self) -> Option<RelayStats> {
        self.relay.as_ref().map(RelayServer::stats)
    }

    /// Peers registered at the relay.
    #[must_use]
    pub fn relay_peers(&self) -> usize {
        self.relay.as_ref().map_or(0, RelayServer::peer_count)
    }

    /// Fabric datagrams lost so far, relay hops included.
    #[must_use]
    pub fn fabric_dropped(&self) -> u64 {
        self.fabric.dropped()
    }

    fn register_with_relay(&mut self, hop_latency: u32) -> Result<(), BoutError> {
        let config = RelayConfig::default();
        let mut relay = RelayServer::new(config);
        for (index, id) in RELAY_IDS.iter().enumerate() {
            let register = RelayMsg::Register {
                client_id: *id,
                token: config.token,
            }
            .encode()
            .map_err(BoutError::Relay)?;
            self.transports[index]
                .send_reliable(self.ids[RELAY], ControlKind::Lobby, &register)
                .map_err(BoutError::Net)?;
        }
        for _ in 0..=hop_latency {
            self.fabric.tick();
            self.now += 1;
        }
        for inbound in self.transports[RELAY].recv() {
            let (InboundFrame::Datagram(bytes) | InboundFrame::Stream { bytes, .. }) =
                inbound.frame;
            if let Ok(RelayMsg::Register { client_id, token }) = RelayMsg::decode(&bytes) {
                relay.register(client_id, token).map_err(BoutError::Relay)?;
                let ack = RelayMsg::Registered { client_id }
                    .encode()
                    .map_err(BoutError::Relay)?;
                self.transports[RELAY]
                    .send_reliable(inbound.from, ControlKind::Lobby, &ack)
                    .map_err(BoutError::Net)?;
            }
        }
        for _ in 0..=hop_latency {
            self.fabric.tick();
            self.now += 1;
        }
        let mut acked = 0;
        for transport in self.transports.iter_mut().take(3) {
            for inbound in transport.recv() {
                let (InboundFrame::Datagram(bytes) | InboundFrame::Stream { bytes, .. }) =
                    inbound.frame;
                acked += usize::from(matches!(
                    RelayMsg::decode(&bytes),
                    Ok(RelayMsg::Registered { .. })
                ));
            }
        }
        if acked != RELAY_IDS.len() || relay.peer_count() != RELAY_IDS.len() {
            return Err(BoutError::Relay(crate::relay::RelayError::UnknownPeer(0)));
        }
        self.relay = Some(relay);
        Ok(())
    }

    fn muted(&self, from: Peer, to: Peer) -> bool {
        self.outage.is_some_and(|o| {
            let involved = from == Peer::Client(o.seat) || to == Peer::Client(o.seat);
            involved && self.now >= o.from && self.now < o.to
        })
    }

    fn send(&mut self, from: Peer, to: Peer, lane: u8, bytes: &[u8]) {
        let flow = self.flows.entry((from, to)).or_insert(Flow {
            next: 0,
            top: None,
            stats: LinkStats::zero(),
        });
        if lane == LANE_STREAM {
            flow.stats.streams += 1;
        } else {
            flow.stats.sent += 1;
        }
        flow.stats.bytes += wire_len(bytes);
        if self.muted(from, to) {
            if lane == LANE_STREAM {
                self.held.push((from, to, lane, bytes.to_vec()));
            } else if let Some(flow) = self.flows.get_mut(&(from, to)) {
                flow.stats.outage += 1;
            }
            return;
        }
        self.transmit(from, to, lane, bytes);
    }

    /// Stamp one frame and hand it to the transport: direct to the peer, or
    /// to the relay wrapped as a forward.
    fn transmit(&mut self, from: Peer, to: Peer, lane: u8, bytes: &[u8]) {
        let Some(flow) = self.flows.get_mut(&(from, to)) else {
            return;
        };
        let seq = flow.next;
        flow.next += 1;
        let mut payload = Vec::with_capacity(STAMP_LEN + bytes.len());
        payload.push(lane);
        payload.push(u8::try_from(slot(from)).unwrap_or(u8::MAX));
        payload.extend_from_slice(&self.now.to_le_bytes());
        payload.extend_from_slice(&seq.to_le_bytes());
        payload.extend_from_slice(bytes);
        let (dest, body) = if self.relay.is_some() {
            let forward = RelayMsg::Forward {
                from: RELAY_IDS[slot(from)],
                to: RELAY_IDS[slot(to)],
                payload,
            };
            match forward.encode() {
                Ok(body) => (self.ids[RELAY], body),
                Err(_) => return,
            }
        } else {
            (self.ids[slot(to)], payload)
        };
        let kind = lane_kind(bytes);
        let transport = &mut self.transports[slot(from)];
        let _ = if lane == LANE_STREAM {
            transport.send_reliable(dest, kind, &body)
        } else {
            transport.send_unreliable(dest, &body)
        };
    }

    /// Relay hub: take every forward that reached the relay this tick, queue
    /// it, and send it on to its destination over the same lane.
    fn pump_relay(&mut self) {
        if self.pumped == Some(self.now) {
            return;
        }
        self.pumped = Some(self.now);
        let Self {
            relay,
            transports,
            ids,
            ..
        } = self;
        let Some(relay) = relay.as_mut() else {
            return;
        };
        for inbound in transports[RELAY].recv() {
            let (InboundFrame::Datagram(bytes) | InboundFrame::Stream { bytes, .. }) =
                inbound.frame;
            let Ok(RelayMsg::Forward { from, to, payload }) = RelayMsg::decode(&bytes) else {
                continue;
            };
            let Some(dest) = RELAY_IDS.iter().position(|id| *id == to) else {
                continue;
            };
            let lane = payload.first().copied().unwrap_or(LANE_DATAGRAM);
            let kind = lane_kind(payload.get(STAMP_LEN..).unwrap_or(&[]));
            if relay.forward(from, to, &payload).is_err() {
                continue;
            }
            for queued in relay.drain(to) {
                let Ok(onward) = (RelayMsg::Forward {
                    from,
                    to,
                    payload: queued,
                })
                .encode() else {
                    continue;
                };
                let _ = if lane == LANE_STREAM {
                    transports[RELAY].send_reliable(ids[dest], kind, &onward)
                } else {
                    transports[RELAY].send_unreliable(ids[dest], &onward)
                };
            }
        }
    }

    /// Pull every frame due at `at` off its transport into the inbox,
    /// unwrapping the relay forward and measuring transit and order.
    fn collect(&mut self, at: Peer) {
        let index = slot(at);
        let relayed = self.relay.is_some();
        for inbound in self.transports[index].recv() {
            let (InboundFrame::Datagram(body) | InboundFrame::Stream { bytes: body, .. }) =
                inbound.frame;
            let payload = if relayed {
                match RelayMsg::decode(&body) {
                    Ok(RelayMsg::Forward { payload, .. }) => payload,
                    _ => continue,
                }
            } else {
                body
            };
            if payload.len() < STAMP_LEN {
                continue;
            }
            let lane = payload[0];
            let from = peer_at(usize::from(payload[1]));
            let sent_at = u32::from_le_bytes([payload[2], payload[3], payload[4], payload[5]]);
            let seq = u32::from_le_bytes([payload[6], payload[7], payload[8], payload[9]]);
            let data = payload[STAMP_LEN..].to_vec();
            let transit = usize::try_from(self.now.saturating_sub(sent_at))
                .unwrap_or(DELAY_BUCKETS - 1)
                .min(DELAY_BUCKETS - 1);
            let Some(flow) = self.flows.get_mut(&(from, at)) else {
                continue;
            };
            flow.stats.delay[transit] += 1;
            let frame = if lane == LANE_STREAM {
                Frame::Stream(data)
            } else {
                flow.stats.delivered += 1;
                if flow.top.is_some_and(|top| seq < top) {
                    flow.stats.reordered += 1;
                }
                flow.top = Some(flow.top.map_or(seq, |top| top.max(seq)));
                Frame::Datagram(data)
            };
            self.inbox[index].push_back((from, frame));
        }
    }
}

/// Stream class of a payload: lobby bytes ride the lobby stream, everything
/// else (catch-up state) the snapshot stream.
fn lane_kind(bytes: &[u8]) -> ControlKind {
    if bytes.starts_with(&LOBBY_MAGIC.to_le_bytes()) {
        ControlKind::Lobby
    } else {
        ControlKind::Snapshot
    }
}

impl Wire for OnlineWire {
    fn datagram(&mut self, from: Peer, to: Peer, bytes: &[u8]) {
        self.send(from, to, LANE_DATAGRAM, bytes);
    }

    fn stream(&mut self, from: Peer, to: Peer, bytes: &[u8]) {
        self.send(from, to, LANE_STREAM, bytes);
    }

    fn recv(&mut self, at: Peer) -> Vec<(Peer, Frame)> {
        self.pump_relay();
        self.collect(at);
        self.inbox[slot(at)].drain(..).collect()
    }

    fn tick(&mut self) {
        self.fabric.tick();
        self.now += 1;
        if self.outage.is_some_and(|o| self.now >= o.to) {
            for (from, to, lane, bytes) in std::mem::take(&mut self.held) {
                self.transmit(from, to, lane, &bytes);
            }
        }
    }

    fn report(&self) -> Vec<LinkReport> {
        self.flows
            .iter()
            .map(|((from, to), flow)| {
                let mut stats = flow.stats;
                stats.dropped = stats.sent.saturating_sub(stats.delivered);
                LinkReport {
                    from: *from,
                    to: *to,
                    stats,
                }
            })
            .collect()
    }
}

/// Both seats join by invite code over the wire: the ticket is minted from
/// the seed, each seat presents code plus build, the host decides. Returns
/// which seats were accepted.
fn invite_handshake<W: Wire>(wire: &mut W, seed: u64, hop: u32) -> Result<[bool; 2], BoutError> {
    let mut rng = seed;
    let ticket = InviteTicket::issue(
        InviteCode::mint(&mut rng),
        BOUT_BUILD.version,
        BOUT_BUILD.rules,
        0,
    )
    .map_err(BoutError::Relay)?;
    for seat in 0..SEATS {
        let request = Signal::Request {
            code: ticket.code.as_bytes(),
            version: BOUT_BUILD.version,
            rules: BOUT_BUILD.rules,
            token: u64::from(seat) + 1,
        };
        wire.stream(
            Peer::Client(seat),
            Peer::Server,
            &request.encode().map_err(BoutError::Relay)?,
        );
    }
    let mut accepted = [false; 2];
    for _ in 0..=hop * 5 {
        for (from, frame) in wire.recv(Peer::Server) {
            let (Peer::Client(seat), Frame::Stream(bytes)) = (from, frame) else {
                continue;
            };
            if let Ok(Signal::Request {
                code,
                version,
                rules,
                token,
            }) = Signal::decode(&bytes)
            {
                let reply = decide_join(&ticket, code, version, rules, token, seat, 10);
                wire.stream(
                    Peer::Server,
                    from,
                    &reply.encode().map_err(BoutError::Relay)?,
                );
            }
        }
        for seat in 0..SEATS {
            for (_, frame) in wire.recv(Peer::Client(seat)) {
                if let Frame::Stream(bytes) = frame {
                    if let Ok(Signal::Accept { token, .. }) = Signal::decode(&bytes) {
                        accepted[usize::from(seat)] |= token == u64::from(seat) + 1;
                    }
                }
            }
        }
        wire.tick();
    }
    Ok(accepted)
}

/// An online bout and everything that led to it.
#[derive(Clone, PartialEq, Debug)]
pub struct OnlineReport {
    /// The hole-punch verdict and the road taken.
    pub punch: PunchOutcome,
    /// Seats the invite handshake accepted.
    pub accepted: [bool; 2],
    /// Peers registered at the relay (zero on the direct path).
    pub relay_peers: usize,
    /// Relay counters (`None` on the direct path).
    pub relay: Option<RelayStats>,
    /// The played bout.
    pub bout: BoutReport,
}

impl OnlineReport {
    /// The ON30 verdict: the bout converged in full, both seats joined by
    /// invite, the road was picked inside the punch bound, and on the relay
    /// path the relay really carried the game.
    #[must_use]
    pub fn passed(&self) -> bool {
        let via_relay = match self.punch.path {
            Path::Direct => self.relay.is_none(),
            Path::Relay => self.relay.is_some_and(|stats| stats.forwarded > 0),
        };
        self.bout.converged()
            && self.accepted == [true, true]
            && self.punch.decided_after <= PUNCH_TIMEOUT_TICKS
            && via_relay
    }
}

/// Play the online bout: invite handshake, punch decision, then the bout
/// over QUIC (through the relay when the punch falls back).
///
/// # Errors
///
/// [`BoutError`] when a part refuses; a bout that runs but does not converge
/// is a report whose [`OnlineReport::passed`] is false.
pub fn run_online(
    seed: u64,
    nats: [NatKind; SEAT_COUNT],
    hop_latency: u32,
) -> Result<OnlineReport, BoutError> {
    let punch = resolve_path(nats);
    let mut wire = OnlineWire::open(punch.path, hop_latency, seed)?;
    let accepted = invite_handshake(&mut wire, seed, hop_latency)?;
    let hops = if punch.path == Path::Relay { 2 } else { 1 };
    let cfg = BoutConfig::canonical(seed, hop_latency * hops);
    let base = wire.now();
    wire.set_outage(cfg.drop.seat, base + cfg.drop.at, base + cfg.drop.back);
    let bout = run_bout(&cfg, &mut wire)?;
    Ok(OnlineReport {
        punch,
        accepted,
        relay_peers: wire.relay_peers(),
        relay: wire.relay_stats(),
        bout,
    })
}

/// The ON30 evidence: file name and body of every receipt, plus the verdict.
#[derive(Clone, PartialEq, Debug)]
pub struct On30Receipts {
    /// Receipt files in write order: (name, JSON body).
    pub files: Vec<(&'static str, String)>,
    /// Both roads passed and both repeated their digest.
    pub passed: bool,
    /// One display line per road, for the operator log.
    pub lines: Vec<String>,
}

/// Play the relay road and the direct road, each twice, and render the
/// ON30 receipts. Files are returned, never written: the caller owns IO.
///
/// # Errors
///
/// [`BoutError`] when a part refuses.
pub fn on30_receipts(seed: u64) -> Result<On30Receipts, BoutError> {
    let run = |nats| run_online(seed, nats, HOP_LATENCY_TICKS);
    let relayed = run([NatKind::Cone, NatKind::Symmetric])?;
    let relayed_twin = run([NatKind::Cone, NatKind::Symmetric])?;
    let direct = run([NatKind::Open, NatKind::Cone])?;
    let direct_twin = run([NatKind::Open, NatKind::Cone])?;
    let wrap = |relay: String, direct: String| {
        format!(
            "{{
  \"relay\": {},
  \"direct\": {}
}}
",
            relay.trim(),
            direct.trim()
        )
    };
    let files = vec![
        (
            "online-bout.json",
            wrap(
                online_json("relay", &relayed, Some(&relayed_twin)),
                online_json("direct", &direct, Some(&direct_twin)),
            ),
        ),
        (
            "checkpoints.json",
            wrap(
                receipt::checkpoints_json("relay", &relayed.bout),
                receipt::checkpoints_json("direct", &direct.bout),
            ),
        ),
        (
            "drop-rejoin.json",
            wrap(
                receipt::drop_json("relay", &relayed.bout),
                receipt::drop_json("direct", &direct.bout),
            ),
        ),
        (
            "distributions.json",
            receipt::distributions_json(&[("relay", &relayed.bout), ("direct", &direct.bout)]),
        ),
    ];
    let passed = relayed.passed()
        && direct.passed()
        && relayed.bout.digest() == relayed_twin.bout.digest()
        && direct.bout.digest() == direct_twin.bout.digest();
    Ok(On30Receipts {
        files,
        passed,
        lines: vec![
            format!("RELAY {}", relayed.bout),
            format!("DIRECT {}", direct.bout),
        ],
    })
}

/// The online receipt: path decision, handshake, relay counters and the bout.
fn online_json(label: &str, report: &OnlineReport, twin: Option<&OnlineReport>) -> String {
    let mut o = Obj::new();
    o.text("path", label)
        .num("punch_decided_after_ticks", report.punch.decided_after)
        .num("punch_bound_ticks", PUNCH_TIMEOUT_TICKS)
        .flag(
            "punch_inside_bound",
            report.punch.decided_after <= PUNCH_TIMEOUT_TICKS,
        )
        .flag("invites_accepted", report.accepted == [true, true])
        .num("relay_peers", report.relay_peers)
        .num("relay_forwarded", report.relay.map_or(0, |s| s.forwarded))
        .num(
            "relay_bytes_forwarded",
            report.relay.map_or(0, |s| s.bytes_forwarded),
        )
        .num(
            "relay_dropped_full",
            report.relay.map_or(0, |s| s.dropped_full),
        )
        .flag("passed", report.passed());
    if let Some(twin) = twin {
        o.flag(
            "hash_x2_identical",
            twin.bout.digest() == report.bout.digest(),
        );
    }
    o.raw(
        "bout",
        receipt::bout_json(label, &report.bout, twin.map(|t| &t.bout)),
    );
    o.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn online_punch_decides_inside_the_bound() {
        let open = resolve_path([NatKind::Open, NatKind::Cone]);
        assert_eq!(open.path, Path::Direct);
        assert!(open.decided_after <= 2, "direct wins on the first check");
        let hard = resolve_path([NatKind::Cone, NatKind::Symmetric]);
        assert_eq!(hard.path, Path::Relay);
        assert_eq!(hard.decided_after, PUNCH_TIMEOUT_TICKS);
        assert!(hard.decided_after <= 300);
    }

    #[test]
    fn online_relay_bout_converges_through_drop_and_rejoin() {
        let report = run_online(2040, [NatKind::Cone, NatKind::Symmetric], HOP_LATENCY_TICKS)
            .expect("online bout runs");
        assert!(report.passed(), "{}", report.bout);
        assert_eq!(report.relay_peers, 3);
        let relay = report.relay.expect("relay carried the game");
        assert!(relay.forwarded > 1000, "{relay:?}");
        assert_eq!(relay.dropped_full, 0);
        let twin = run_online(2040, [NatKind::Cone, NatKind::Symmetric], HOP_LATENCY_TICKS)
            .expect("second run");
        assert_eq!(twin.bout.digest(), report.bout.digest(), "hash x2");
        assert_eq!(twin.relay, report.relay);
        // Two hops each way: a relayed frame takes twice the hop latency.
        let (up, _) = report.bout.totals();
        assert!(up.delay_pct(50) >= 2 * HOP_LATENCY_TICKS);
    }

    #[test]
    fn online_direct_bout_converges_and_skips_the_relay() {
        let report = run_online(2040, [NatKind::Open, NatKind::Cone], HOP_LATENCY_TICKS)
            .expect("online bout runs");
        assert!(report.passed(), "{}", report.bout);
        assert_eq!(report.relay, None);
        let (up, _) = report.bout.totals();
        assert!(up.delay_pct(50) >= HOP_LATENCY_TICKS && up.delay_pct(50) < 2 * HOP_LATENCY_TICKS);
    }
}
