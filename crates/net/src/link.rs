//! link: seeded fault-injection wire for played bouts.
//!
//! Purpose: the loopback network of the two-machine proofs. One [`FaultLink`](crate::link::FaultLink)
//! is one direction between two peers; a [`Wire`](crate::link::Wire) joins a server and two
//! clients. Datagrams ride the lossy lane (loss, jitter, reorder, scripted
//! outage); streams ride the reliable lane (ordered, never lost, held across
//! an outage), the QUIC channel split. Owner: net lane.
//! Units: ticks at 60 Hz, bytes, whole percent. Determinism: SplitMix64 draws
//! per link in stable send order, integer ops only, no clock, no socket.
//! Errors: none, bad profiles clamp. Measured: every link counts sent,
//! dropped, delivered and reordered frames plus a one-way delay histogram,
//! the loss and latency distributions the proofs post.

use engine_core::{hash::GOLDEN_GAMMA, mix64};

/// Delay histogram buckets: one per tick, the last holds everything slower.
pub const DELAY_BUCKETS: usize = 33;

/// Bytes of one frame as the counters hold them.
pub(crate) fn wire_len(bytes: &[u8]) -> u64 {
    u64::try_from(bytes.len()).unwrap_or(u64::MAX)
}

/// Extra delay a reordered datagram takes, in ticks: enough to land behind
/// its next two successors at the 60 Hz send cadence.
const REORDER_HOLD_TICKS: u32 = 3;

/// One direction's network conditions. Percentages clamp to `0..=100`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct LinkProfile {
    /// Base one-way delay in ticks.
    pub latency: u32,
    /// Extra one-way delay drawn uniformly from `0..=jitter` ticks.
    pub jitter: u32,
    /// Datagram loss in percent.
    pub loss_pct: u32,
    /// Datagrams held back `REORDER_HOLD_TICKS` extra ticks, in percent.
    pub reorder_pct: u32,
}

impl LinkProfile {
    /// Slowest one-way transit this profile can draw, in ticks.
    #[must_use]
    pub const fn worst_delay(&self) -> u32 {
        self.latency
            + self.jitter
            + if self.reorder_pct > 0 {
                REORDER_HOLD_TICKS
            } else {
                0
            }
    }

    /// Home network: 1 to 2 ticks one way (17 to 33 ms), 1% loss.
    pub const LAN: Self = Self {
        latency: 1,
        jitter: 1,
        loss_pct: 1,
        reorder_pct: 0,
    };
    /// Internet path: 4 to 6 ticks one way (67 to 100 ms), 5% loss, 5% reorder.
    pub const WAN: Self = Self {
        latency: 4,
        jitter: 2,
        loss_pct: 5,
        reorder_pct: 5,
    };
}

/// Which side of the match a frame belongs to.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Peer {
    /// The dedicated server.
    Server,
    /// A human client by seat index.
    Client(u8),
}

/// One received frame: lossy datagram bytes or one reliable message.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Frame {
    /// Unreliable datagram (inputs, state windows).
    Datagram(Vec<u8>),
    /// Reliable ordered message (join, grant, catch-up chunks).
    Stream(Vec<u8>),
}

/// Measured traffic of one link.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct LinkStats {
    /// Datagrams offered to the link.
    pub sent: u32,
    /// Datagrams lost to the loss draw or an outage.
    pub dropped: u32,
    /// Of those, the datagrams lost to a scripted outage.
    pub outage: u32,
    /// Datagrams handed to the receiver.
    pub delivered: u32,
    /// Datagrams that arrived behind a later-sent datagram.
    pub reordered: u32,
    /// Reliable messages offered (all delivered).
    pub streams: u32,
    /// Bytes offered across both lanes.
    pub bytes: u64,
    /// One-way transit of every delivered frame, in ticks, one bucket per tick.
    pub delay: [u32; DELAY_BUCKETS],
}

impl LinkStats {
    const EMPTY: Self = Self {
        sent: 0,
        dropped: 0,
        outage: 0,
        delivered: 0,
        reordered: 0,
        streams: 0,
        bytes: 0,
        delay: [0; DELAY_BUCKETS],
    };

    /// Measured random datagram loss in basis points (1/100 percent) over
    /// the datagrams offered to a live link: a scripted outage is counted
    /// apart in `outage`, not folded into the loss rate.
    #[must_use]
    pub fn loss_bp(&self) -> u32 {
        let live = u64::from(self.sent.saturating_sub(self.outage));
        if live == 0 {
            return 0;
        }
        let lost = u64::from(self.dropped.saturating_sub(self.outage));
        u32::try_from(lost * 10_000 / live).unwrap_or(10_000)
    }

    /// Frames recorded in the delay histogram.
    #[must_use]
    pub fn frames(&self) -> u32 {
        self.delay.iter().sum()
    }

    /// Nearest-rank delay percentile in ticks (`pct` in `1..=100`); zero on
    /// an empty histogram. The last bucket reads as its lower edge.
    #[must_use]
    pub fn delay_pct(&self, pct: u32) -> u32 {
        let total = u64::from(self.frames());
        if total == 0 {
            return 0;
        }
        let rank = (total * u64::from(pct.clamp(1, 100))).div_ceil(100).max(1);
        let mut seen = 0u64;
        for (ticks, count) in self.delay.iter().enumerate() {
            seen += u64::from(*count);
            if seen >= rank {
                return u32::try_from(ticks).unwrap_or(u32::MAX);
            }
        }
        u32::try_from(DELAY_BUCKETS - 1).unwrap_or(u32::MAX)
    }

    /// Slowest delivered frame in ticks.
    #[must_use]
    pub fn delay_max(&self) -> u32 {
        self.delay
            .iter()
            .rposition(|count| *count > 0)
            .map_or(0, |ticks| u32::try_from(ticks).unwrap_or(u32::MAX))
    }

    /// Fold another link's counters into this one (the per-class totals).
    pub fn absorb(&mut self, other: &Self) {
        self.sent += other.sent;
        self.dropped += other.dropped;
        self.outage += other.outage;
        self.delivered += other.delivered;
        self.reordered += other.reordered;
        self.streams += other.streams;
        self.bytes += other.bytes;
        for (mine, theirs) in self.delay.iter_mut().zip(other.delay.iter()) {
            *mine += *theirs;
        }
    }

    /// Empty counters, the identity of [`Self::absorb`].
    #[must_use]
    pub const fn zero() -> Self {
        Self::EMPTY
    }
}

/// One frame in flight.
struct Flight {
    deliver_at: u32,
    sent_at: u32,
    seq: u64,
    stream: bool,
    bytes: Vec<u8>,
}

/// One direction of the injected network.
pub struct FaultLink {
    profile: LinkProfile,
    rng: u64,
    now: u32,
    seq: u64,
    top_seq: u64,
    stream_tail: u32,
    outage: Option<(u32, u32)>,
    flights: Vec<Flight>,
    stats: LinkStats,
}

impl FaultLink {
    /// Open a link under `profile` with its own draw stream from `seed`.
    #[must_use]
    pub fn new(profile: LinkProfile, seed: u64) -> Self {
        Self {
            profile,
            rng: seed,
            now: 0,
            seq: 0,
            top_seq: 0,
            stream_tail: 0,
            outage: None,
            flights: Vec::new(),
            stats: LinkStats::EMPTY,
        }
    }

    /// Script an outage: every datagram sent in `from..to` is lost and
    /// reliable messages wait for tick `to`.
    pub fn set_outage(&mut self, from: u32, to: u32) {
        self.outage = Some((from, to));
    }

    /// Measured counters so far.
    #[must_use]
    pub fn stats(&self) -> &LinkStats {
        &self.stats
    }

    fn draw(&mut self) -> u64 {
        self.rng = self.rng.wrapping_add(GOLDEN_GAMMA);
        mix64(self.rng)
    }

    fn in_outage(&self, tick: u32) -> Option<u32> {
        self.outage
            .filter(|(from, to)| tick >= *from && tick < *to)
            .map(|(_, to)| to)
    }

    fn base_delay(&mut self) -> u32 {
        let spread = u64::from(self.profile.jitter) + 1;
        let jitter = u32::try_from(self.draw() % spread).unwrap_or(0);
        self.profile.latency + jitter
    }

    /// Offer one datagram to the lossy lane.
    pub fn send_datagram(&mut self, bytes: &[u8]) {
        self.stats.sent += 1;
        self.stats.bytes += wire_len(bytes);
        let lost = self.draw() % 100 < u64::from(self.profile.loss_pct.min(100));
        let held = self.draw() % 100 < u64::from(self.profile.reorder_pct.min(100));
        let delay = self.base_delay() + if held { REORDER_HOLD_TICKS } else { 0 };
        let dark = self.in_outage(self.now).is_some();
        if lost || dark {
            self.stats.dropped += 1;
            self.stats.outage += u32::from(dark);
            return;
        }
        self.seq += 1;
        self.flights.push(Flight {
            deliver_at: self.now + delay,
            sent_at: self.now,
            seq: self.seq,
            stream: false,
            bytes: bytes.to_vec(),
        });
    }

    /// Offer one reliable message: ordered, never lost, delivered after any
    /// outage that covers its transit.
    pub fn send_stream(&mut self, bytes: &[u8]) {
        self.stats.streams += 1;
        self.stats.bytes += wire_len(bytes);
        let mut at = self.now + self.base_delay();
        if let Some(end) = self.in_outage(self.now).or_else(|| self.in_outage(at)) {
            at = at.max(end + self.profile.latency);
        }
        at = at.max(self.stream_tail);
        self.stream_tail = at;
        self.seq += 1;
        self.flights.push(Flight {
            deliver_at: at,
            sent_at: self.now,
            seq: self.seq,
            stream: true,
            bytes: bytes.to_vec(),
        });
    }

    /// Every frame due at the current tick, oldest delivery first.
    pub fn due(&mut self) -> Vec<Frame> {
        let now = self.now;
        let (mut due, rest): (Vec<Flight>, Vec<Flight>) = std::mem::take(&mut self.flights)
            .into_iter()
            .partition(|flight| flight.deliver_at <= now);
        self.flights = rest;
        due.sort_by_key(|flight| (flight.deliver_at, flight.seq));
        let mut out = Vec::with_capacity(due.len());
        for flight in due {
            let transit = usize::try_from(flight.deliver_at - flight.sent_at)
                .unwrap_or(DELAY_BUCKETS - 1)
                .min(DELAY_BUCKETS - 1);
            self.stats.delay[transit] += 1;
            if flight.stream {
                out.push(Frame::Stream(flight.bytes));
                continue;
            }
            self.stats.delivered += 1;
            if flight.seq < self.top_seq {
                self.stats.reordered += 1;
            }
            self.top_seq = self.top_seq.max(flight.seq);
            out.push(Frame::Datagram(flight.bytes));
        }
        out
    }

    /// Advance the link one tick.
    pub fn tick(&mut self) {
        self.now += 1;
    }
}

/// Measured traffic of one directed link.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct LinkReport {
    /// Sender of the link.
    pub from: Peer,
    /// Receiver of the link.
    pub to: Peer,
    /// Counters and delay histogram.
    pub stats: LinkStats,
}

/// The network a played bout runs over: datagrams lose, streams hold.
pub trait Wire {
    /// Queue one lossy datagram.
    fn datagram(&mut self, from: Peer, to: Peer, bytes: &[u8]);
    /// Queue one reliable ordered message.
    fn stream(&mut self, from: Peer, to: Peer, bytes: &[u8]);
    /// Drain every frame due for `at`, sender-tagged, oldest first.
    fn recv(&mut self, at: Peer) -> Vec<(Peer, Frame)>;
    /// Advance the whole wire one tick.
    fn tick(&mut self);
    /// Measured traffic of every directed link, in a fixed order.
    fn report(&self) -> Vec<LinkReport>;
}

/// Server plus two client seats over four [`FaultLink`]s.
pub struct LoopbackWire {
    up: Vec<FaultLink>,
    down: Vec<FaultLink>,
}

impl LoopbackWire {
    /// Open `seats` client seats; every link draws from its own stream
    /// forked off `seed` so seats never share a sequence.
    #[must_use]
    pub fn new(seats: u8, up: LinkProfile, down: LinkProfile, seed: u64) -> Self {
        let fork = |lane: u64| mix64(seed ^ lane.wrapping_mul(GOLDEN_GAMMA));
        let mut wire = Self {
            up: Vec::new(),
            down: Vec::new(),
        };
        for seat in 0..seats {
            let lane = u64::from(seat) * 2 + 1;
            wire.up.push(FaultLink::new(up, fork(lane)));
            wire.down.push(FaultLink::new(down, fork(lane + 1)));
        }
        wire
    }

    /// Script an outage on both directions of one seat.
    pub fn set_outage(&mut self, seat: u8, from: u32, to: u32) {
        if let Some(link) = self.up.get_mut(usize::from(seat)) {
            link.set_outage(from, to);
        }
        if let Some(link) = self.down.get_mut(usize::from(seat)) {
            link.set_outage(from, to);
        }
    }

    /// The directed link from `from` to `to`; `None` when that pair is not a
    /// lane. A bout split across machines drives each link on its sender.
    pub fn link_mut(&mut self, from: Peer, to: Peer) -> Option<&mut FaultLink> {
        match (from, to) {
            (Peer::Client(seat), Peer::Server) => self.up.get_mut(usize::from(seat)),
            (Peer::Server, Peer::Client(seat)) => self.down.get_mut(usize::from(seat)),
            _ => None,
        }
    }
}

impl Wire for LoopbackWire {
    fn datagram(&mut self, from: Peer, to: Peer, bytes: &[u8]) {
        match (from, to) {
            (Peer::Client(seat), Peer::Server) => {
                if let Some(link) = self.up.get_mut(usize::from(seat)) {
                    link.send_datagram(bytes);
                }
            }
            (Peer::Server, Peer::Client(seat)) => {
                if let Some(link) = self.down.get_mut(usize::from(seat)) {
                    link.send_datagram(bytes);
                }
            }
            _ => {}
        }
    }

    fn stream(&mut self, from: Peer, to: Peer, bytes: &[u8]) {
        match (from, to) {
            (Peer::Client(seat), Peer::Server) => {
                if let Some(link) = self.up.get_mut(usize::from(seat)) {
                    link.send_stream(bytes);
                }
            }
            (Peer::Server, Peer::Client(seat)) => {
                if let Some(link) = self.down.get_mut(usize::from(seat)) {
                    link.send_stream(bytes);
                }
            }
            _ => {}
        }
    }

    fn recv(&mut self, at: Peer) -> Vec<(Peer, Frame)> {
        let mut out = Vec::new();
        match at {
            Peer::Server => {
                for (seat, link) in self.up.iter_mut().enumerate() {
                    let from = Peer::Client(u8::try_from(seat).unwrap_or(u8::MAX));
                    out.extend(link.due().into_iter().map(|frame| (from, frame)));
                }
            }
            Peer::Client(seat) => {
                if let Some(link) = self.down.get_mut(usize::from(seat)) {
                    out.extend(link.due().into_iter().map(|frame| (Peer::Server, frame)));
                }
            }
        }
        out
    }

    fn tick(&mut self) {
        for link in self.up.iter_mut().chain(self.down.iter_mut()) {
            link.tick();
        }
    }

    fn report(&self) -> Vec<LinkReport> {
        let mut out = Vec::new();
        for (seat, link) in self.up.iter().enumerate() {
            let seat = u8::try_from(seat).unwrap_or(u8::MAX);
            out.push(LinkReport {
                from: Peer::Client(seat),
                to: Peer::Server,
                stats: *link.stats(),
            });
        }
        for (seat, link) in self.down.iter().enumerate() {
            let seat = u8::try_from(seat).unwrap_or(u8::MAX);
            out.push(LinkReport {
                from: Peer::Server,
                to: Peer::Client(seat),
                stats: *link.stats(),
            });
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Push `count` datagrams through a link and drain it to the end.
    fn run(profile: LinkProfile, seed: u64, count: u32) -> (FaultLink, Vec<u32>) {
        let mut link = FaultLink::new(profile, seed);
        let mut seen = Vec::new();
        for step in 0..count + 64 {
            if step < count {
                link.send_datagram(&step.to_le_bytes());
            }
            for frame in link.due() {
                if let Frame::Datagram(bytes) = frame {
                    seen.push(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]));
                }
            }
            link.tick();
        }
        (link, seen)
    }

    #[test]
    fn link_clean_profile_delivers_everything_in_order_after_latency() {
        let clean = LinkProfile {
            latency: 3,
            jitter: 0,
            loss_pct: 0,
            reorder_pct: 0,
        };
        let (link, seen) = run(clean, 5, 100);
        assert_eq!(seen, (0..100).collect::<Vec<_>>());
        let stats = link.stats();
        assert_eq!((stats.sent, stats.dropped, stats.reordered), (100, 0, 0));
        assert_eq!(
            (stats.delay_pct(50), stats.delay_pct(99), stats.delay_max()),
            (3, 3, 3)
        );
        assert_eq!(stats.frames(), 100);
    }

    #[test]
    fn link_loss_jitter_reorder_land_inside_their_profile() {
        let (link, seen) = run(LinkProfile::WAN, 0xFA17, 4000);
        let stats = link.stats();
        // 5% loss measured over 4000 draws stays within 3 to 7 percent.
        assert!(
            (300..=700).contains(&stats.loss_bp()),
            "loss {} bp",
            stats.loss_bp()
        );
        assert!(stats.reordered > 0, "5% reorder must show");
        assert_eq!(stats.delivered, u32::try_from(seen.len()).expect("count"));
        assert_eq!(stats.delivered + stats.dropped, stats.sent);
        // One-way delay is latency 4 plus jitter 0..=2 plus a 3-tick hold on
        // the reordered share, never anything else.
        let min = (0..DELAY_BUCKETS)
            .find(|t| stats.delay[*t] > 0)
            .expect("min");
        assert_eq!(min, 4);
        assert!(stats.delay_max() <= 4 + 2 + REORDER_HOLD_TICKS);
        assert!((4..=6).contains(&stats.delay_pct(50)));
    }

    #[test]
    fn link_outage_drops_datagrams_and_holds_streams() {
        let mut link = FaultLink::new(LinkProfile::LAN, 9);
        link.set_outage(10, 40);
        let mut datagrams = 0u32;
        let mut stream_at = None;
        for step in 0..80u32 {
            if step == 20 {
                link.send_stream(b"join");
            }
            link.send_datagram(&step.to_le_bytes());
            for frame in link.due() {
                match frame {
                    Frame::Datagram(_) => datagrams += 1,
                    Frame::Stream(bytes) => {
                        assert_eq!(bytes, b"join");
                        stream_at = Some(step);
                    }
                }
            }
            link.tick();
        }
        let stats = link.stats();
        assert!(
            stats.dropped >= 30,
            "the 30-tick outage loses its datagrams"
        );
        assert_eq!(
            stats.outage, 30,
            "all and only the dark ticks count as outage"
        );
        assert!(
            stats.loss_bp() < 1_000,
            "the outage is not folded into loss"
        );
        assert!(datagrams < 80 - 30 + 1);
        let at = stream_at.expect("the stream survives the outage");
        assert!(at >= 40, "held until the outage ends, delivered at {at}");
    }

    #[test]
    fn link_streams_stay_ordered_under_jitter() {
        let mut link = FaultLink::new(LinkProfile::WAN, 77);
        let mut order = Vec::new();
        for step in 0..120u32 {
            if step < 60 {
                link.send_stream(&step.to_le_bytes());
            }
            for frame in link.due() {
                if let Frame::Stream(bytes) = frame {
                    order.push(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]));
                }
            }
            link.tick();
        }
        assert_eq!(order, (0..60).collect::<Vec<_>>());
        assert_eq!(link.stats().dropped, 0, "streams never lose");
    }

    #[test]
    fn link_same_seed_same_conditions_other_seed_differs() {
        let (a, sa) = run(LinkProfile::WAN, 1, 500);
        let (b, sb) = run(LinkProfile::WAN, 1, 500);
        let (c, sc) = run(LinkProfile::WAN, 2, 500);
        assert_eq!(sa, sb);
        assert_eq!(a.stats(), b.stats());
        assert_ne!(sa, sc, "another seed must draw another network");
        assert_ne!(a.stats(), c.stats());
    }

    #[test]
    fn link_wire_routes_seats_apart() {
        let clean = LinkProfile {
            latency: 1,
            jitter: 0,
            loss_pct: 0,
            reorder_pct: 0,
        };
        let mut wire = LoopbackWire::new(2, clean, clean, 3);
        wire.datagram(Peer::Client(0), Peer::Server, b"zero");
        wire.datagram(Peer::Client(1), Peer::Server, b"one");
        wire.stream(Peer::Server, Peer::Client(1), b"grant");
        // Client to client is not a lane: nothing queues.
        wire.datagram(Peer::Client(0), Peer::Client(1), b"stray");
        for _ in 0..4 {
            wire.tick();
        }
        assert_eq!(
            wire.recv(Peer::Server),
            vec![
                (Peer::Client(0), Frame::Datagram(b"zero".to_vec())),
                (Peer::Client(1), Frame::Datagram(b"one".to_vec())),
            ]
        );
        assert_eq!(
            wire.recv(Peer::Client(1)),
            vec![(Peer::Server, Frame::Stream(b"grant".to_vec()))]
        );
        assert!(wire.recv(Peer::Client(0)).is_empty());
        assert_eq!(wire.report().len(), 4);
    }
}
