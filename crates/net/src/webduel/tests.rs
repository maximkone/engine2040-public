//! The duel over in-memory transports: the host and the client are the same
//! state machines the binary and the page run, so these pin the protocol, the
//! hash the page and the native server compare, and the loss and rejoin paths.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::net::SocketAddr;
use std::rc::Rc;

use engine_core::mix64;

use super::*;
use crate::guard::{Refusal, MAX_AHEAD_TICKS};
use crate::quic::check_budget;

type Queue = Rc<RefCell<VecDeque<Inbound>>>;

/// One end of a link whose datagrams can be dropped by a seeded draw.
struct End {
    me: PeerId,
    peer: PeerId,
    inbox: Queue,
    outbox: Queue,
    /// Per mille of datagrams this end drops on send.
    loss: u64,
    draws: u64,
    dropped: u64,
    /// From this tick on, the state datagrams this end sends carry a wrong hash.
    lie_from: Option<u32>,
}

fn pair() -> (End, End) {
    let server = PeerId(SocketAddr::from(([127, 0, 0, 1], 9_000)));
    let client = PeerId(SocketAddr::from(([127, 0, 0, 1], 9_001)));
    let (to_server, to_client): (Queue, Queue) = Default::default();
    (
        End {
            me: server,
            peer: client,
            inbox: Rc::clone(&to_server),
            outbox: Rc::clone(&to_client),
            loss: 0,
            draws: 0,
            dropped: 0,
            lie_from: None,
        },
        End {
            me: client,
            peer: server,
            inbox: to_client,
            outbox: to_server,
            loss: 0,
            draws: 0,
            dropped: 0,
            lie_from: None,
        },
    )
}

impl Transport for End {
    fn send_unreliable(&mut self, to: PeerId, bytes: &[u8]) -> Result<(), NetError> {
        check_budget(bytes)?;
        assert_eq!(to, self.peer, "the link has one peer");
        self.draws += 1;
        if mix64(self.draws) % 1000 < self.loss {
            self.dropped += 1;
            return Ok(());
        }
        let bytes = self.lie(bytes);
        self.outbox.borrow_mut().push_back(Inbound {
            from: self.me,
            frame: InboundFrame::Datagram(bytes),
        });
        Ok(())
    }

    fn send_reliable(
        &mut self,
        to: PeerId,
        kind: ControlKind,
        bytes: &[u8],
    ) -> Result<(), NetError> {
        check_budget(bytes)?;
        assert_eq!(to, self.peer, "the link has one peer");
        self.outbox.borrow_mut().push_back(Inbound {
            from: self.me,
            frame: InboundFrame::Stream {
                kind,
                bytes: bytes.to_vec(),
            },
        });
        Ok(())
    }

    fn recv(&mut self) -> Vec<Inbound> {
        self.inbox.borrow_mut().drain(..).collect()
    }
}

impl End {
    /// A state datagram from the lying tick on, with its hash flipped; anything else as sent.
    fn lie(&self, bytes: &[u8]) -> Vec<u8> {
        let Some(from) = self.lie_from else {
            return bytes.to_vec();
        };
        match decode_state(bytes) {
            Ok(mut packet) if packet.checkpoint.tick >= from => {
                packet.checkpoint.hash ^= 1;
                encode_state(packet.checkpoint, &packet.inputs).expect("re-encodes")
            }
            _ => bytes.to_vec(),
        }
    }
}

/// Rounds a stalled run may take before the test fails.
const ROUNDS: u32 = 20_000;

fn host() -> DuelHost {
    DuelHost::new(duel_spec().expect("spec")).expect("host")
}

/// Play rounds until the client ends; the host never gives up on a seat.
fn play(host: &mut DuelHost, server: &mut End, client: &mut DuelClient, wire: &mut End) {
    let at = server.me;
    for _ in 0..ROUNDS {
        host.pump(&mut [&mut *server]);
        host.tick(&mut [&mut *server], false).expect("tick");
        if client.poll(wire, at) {
            return;
        }
    }
    panic!("the duel stalled at host tick {}", host.tick_now());
}

#[test]
fn webduel_ends_on_the_reference_hash_the_server_and_the_page_compare() {
    let (mut server, mut wire) = pair();
    let (mut host, mut client) = (host(), DuelClient::new(0xC0FFEE).expect("client"));
    play(&mut host, &mut server, &mut client, &mut wire);
    let done = client
        .outcome()
        .expect("ended")
        .as_ref()
        .expect("converged");
    let reference = reference_hash(0).expect("reference");
    assert_eq!(done.ticks, DUEL_TICKS);
    assert_eq!(
        done.hash, reference,
        "the page's hash is the plain server's"
    );
    assert_eq!(host.final_hash(), Some(reference), "and the host's");
    assert_eq!((done.peers, done.slot, done.rebuilt), (1, 0, false));
    assert_eq!(
        net_line("websocket", client.outcome().expect("ended")),
        format!("net: transport=websocket ticks=600 hash={reference:016x} peers=1")
    );
    assert!(host.take_events().iter().any(|event| matches!(
        event,
        HostEvent::Joined {
            slot: 0,
            rejoin: false,
            ..
        }
    )));
}

#[test]
fn webduel_hash_is_not_the_v1_open_sand_match() {
    // The spec is the window's duel: the same script on v1 open sand ends elsewhere.
    let mut v1 = Server::new(duel_spec().expect("spec").config).expect("v1 server");
    let mut hash = 0;
    for tick in 0..DUEL_TICKS {
        hash = v1.tick(&[scripted_input(tick, 0)]).expect("tick").0.hash;
    }
    assert_ne!(hash, reference_hash(0).expect("reference"));
}

#[test]
fn webduel_survives_lost_state_datagrams_and_still_ends_on_the_servers_hash() {
    let (mut server, mut wire) = pair();
    server.loss = 400;
    let (mut host, mut client) = (host(), DuelClient::new(7).expect("client"));
    play(&mut host, &mut server, &mut client, &mut wire);
    let done = client
        .outcome()
        .expect("ended")
        .as_ref()
        .expect("converged");
    assert!(
        server.dropped > 100,
        "the link really lost datagrams: {}",
        server.dropped
    );
    assert_eq!(done.hash, reference_hash(0).expect("reference"));
    assert_eq!(host.final_hash(), Some(done.hash));
}

#[test]
fn webduel_refuses_a_peer_on_another_match() {
    let (mut server, mut wire) = pair();
    let mut host = host();
    let mut other = duel_spec().expect("spec");
    other.config.seed ^= 1;
    let mut client = DuelClient::with_spec(9, other);
    play(&mut host, &mut server, &mut client, &mut wire);
    let failure = client
        .outcome()
        .expect("ended")
        .as_ref()
        .expect_err("refused");
    assert!(failure.0.contains("refused the join"), "{failure}");
    assert!(
        failure.0.contains("rules"),
        "both builds are named: {failure}"
    );
    assert!(!host.started(), "a refused peer does not start the match");
    assert!(matches!(
        host.take_events().as_slice(),
        [HostEvent::Refused {
            reason: RefuseReason::VersionMismatch,
            ..
        }]
    ));
}

#[test]
fn webduel_ai_fills_a_seat_that_leaves_and_catches_it_up_when_it_returns() {
    let (mut server, mut wire) = pair();
    let (mut host, mut client) = (host(), DuelClient::new(0xBEEF).expect("client"));
    let at = server.me;
    // Play 200 ticks, then the page goes away.
    while host.tick_now() < 200 {
        host.pump(&mut [&mut server]);
        host.tick(&mut [&mut server], false).expect("tick");
        client.poll(&mut wire, at);
    }
    drop(client);
    let link = SeatLink {
        via: 0,
        peer: wire.me,
    };
    host.depart(link, "the page closed");
    assert_eq!(host.live_seats(), 0);
    // The match goes on with the seat AI-filled.
    while host.tick_now() < 350 {
        host.tick(&mut [&mut server], false).expect("tick");
    }
    // A new page with the same token takes the seat back mid-match, on a second transport.
    let (mut server2, mut wire2) = pair();
    let mut back = DuelClient::new(0xBEEF).expect("client");
    let at2 = server2.me;
    for _ in 0..ROUNDS {
        host.pump(&mut [&mut server, &mut server2]);
        host.tick(&mut [&mut server, &mut server2], false)
            .expect("tick");
        if back.poll(&mut wire2, at2) {
            break;
        }
    }
    let done = back.outcome().expect("ended").as_ref().expect("converged");
    assert_eq!(
        Some(done.hash),
        host.final_hash(),
        "the returning page ends on the server's hash"
    );
    assert_ne!(
        done.hash,
        reference_hash(0).expect("reference"),
        "the AI played the gap, so this is not the scripted hash"
    );
    // The returning page predicted its way back from tick 0 to the tip, 150 ticks ahead, but
    // sent the server only the ticks it still needed: the guard saw no flood.
    let receipt = host.admission()[0];
    assert_eq!(receipt.count(Refusal::Rate), 0, "{receipt:?}");
    assert_eq!(receipt.count(Refusal::Future), 0, "{receipt:?}");
    let events = host.take_events();
    assert!(events
        .iter()
        .any(|e| matches!(e, HostEvent::Dropped { slot: 0, .. })));
    assert!(events.iter().any(|e| matches!(
        e,
        HostEvent::Joined {
            slot: 0,
            rejoin: true,
            ..
        }
    )));
}

#[test]
fn webduel_gives_up_on_a_silent_seat_and_drops_it_after_the_stall_limit() {
    let (mut server, mut wire) = pair();
    let (mut host, mut client) = (host(), DuelClient::new(5).expect("client"));
    let at = server.me;
    // Join, then go quiet: the client is never polled again.
    client.poll(&mut wire, at);
    host.pump(&mut [&mut server]);
    assert_eq!(host.live_seats(), 1);
    assert!(host.blocked());
    assert_eq!(
        host.tick(&mut [&mut server], false).expect("tick"),
        None,
        "it waits"
    );
    for _ in 0..STALL_LIMIT {
        host.tick(&mut [&mut server], true).expect("tick");
    }
    assert_eq!(host.live_seats(), 0, "the silent seat is dropped");
    let events = host.take_events();
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, HostEvent::Stalled { .. }))
            .count(),
        STALL_LIMIT as usize
    );
    assert!(events
        .iter()
        .any(|event| matches!(event, HostEvent::Dropped { .. })));
}

fn input_bytes(tick: u32, slot: u8) -> Vec<u8> {
    crate::session::encode_input(tick, &[scripted_input(tick, slot)]).expect("encode")
}

#[test]
fn webduel_ignores_inputs_from_strangers_far_ahead_and_for_another_slot() {
    let (mut server, mut wire) = pair();
    let mut host = host();
    let at = server.me;
    // An input before any join.
    wire.send_unreliable(at, &input_bytes(0, 0)).expect("send");
    host.pump(&mut [&mut server]);
    assert!(!host.started());
    assert!(host.inputs.is_empty());
    // Joined, then an input beyond the flood window.
    let mut client = DuelClient::new(3).expect("client");
    client.poll(&mut wire, at);
    host.pump(&mut [&mut server]);
    assert!(host.started());
    wire.send_unreliable(at, &input_bytes(MAX_AHEAD_TICKS + 5, 0))
        .expect("send");
    host.pump(&mut [&mut server]);
    assert!(host.inputs.is_empty(), "a far-ahead input is refused");
    wire.send_unreliable(at, &input_bytes(3, 0)).expect("send");
    host.pump(&mut [&mut server]);
    assert_eq!(host.inputs.len(), 1);
    // The wrong slot is refused too.
    wire.send_unreliable(at, &input_bytes(4, 1)).expect("send");
    host.pump(&mut [&mut server]);
    assert_eq!(host.inputs.len(), 1);
}

#[test]
fn webduel_script_is_a_pure_valid_function() {
    for tick in 0..DUEL_TICKS {
        let a = scripted_input(tick, 0);
        assert_eq!(a, scripted_input(tick, 0));
        assert!(a.validate().is_ok(), "tick {tick}");
        assert_eq!(a.tick, tick);
    }
    assert_ne!(scripted_input(10, 0), scripted_input(70, 0));
}

#[test]
fn webduel_headless_server_on_a_spec_equals_the_plain_server() {
    let spec = duel_spec().expect("spec");
    let mut headless = HeadlessServer::from_spec(spec.clone()).expect("headless");
    let mut plain = Server::from_spec(spec).expect("server");
    let token = 11;
    headless.connect(token).expect("seat");
    for tick in 0..120 {
        let input = scripted_input(tick, 0);
        headless.submit(token, input).expect("submit");
        let (a, _) = headless.tick().expect("headless tick");
        let (b, _) = plain.tick(&[input]).expect("tick");
        assert_eq!(a, b, "tick {tick}");
    }
}

/// A pilot that is not the script: walk forward, swing every tenth tick, face one way.
struct Walker;

impl Pilot for Walker {
    fn input(&mut self, tick: u32, slot: u8) -> BodyInput {
        BodyInput {
            tick,
            fighter: BodyId::new(slot).expect("slot"),
            mx: 100,
            mz: 0,
            facing: 9_000,
            buttons: if tick % 10 == 9 {
                Buttons(ATTACK)
            } else {
                Buttons::NONE
            },
        }
    }
}

#[test]
fn webduel_ticks_are_part_of_the_build_and_a_pilot_other_than_the_script_drives_the_seat() {
    let ticks = 120;
    let (mut server, mut wire) = pair();
    let spec = duel_spec_for(ticks).expect("spec");
    let mut host = DuelHost::new(spec).expect("host");
    let mut client = DuelClient::playing(21, ticks, Box::new(Walker)).expect("client");
    play(&mut host, &mut server, &mut client, &mut wire);
    let done = client
        .outcome()
        .expect("ended")
        .as_ref()
        .expect("converged");
    assert_eq!(done.ticks, ticks);
    // The plain server over the same spec and the walker's inputs ends on the same hash.
    let mut plain = Server::from_spec(duel_spec_for(ticks).expect("spec")).expect("server");
    let mut walker = Walker;
    let mut hash = 0;
    for tick in 0..ticks {
        hash = plain.tick(&[walker.input(tick, 0)]).expect("tick").0.hash;
    }
    assert_eq!(done.hash, hash);
    assert_ne!(done.hash, reference_hash_for(ticks, &[0]).expect("script"));
    // A page that asks for another length is another match: refused, both builds named.
    let (mut server2, mut wire2) = pair();
    let mut other = DuelClient::playing(22, ticks + 1, Box::new(Script)).expect("client");
    let mut host2 = DuelHost::new(duel_spec_for(ticks).expect("spec")).expect("host");
    play(&mut host2, &mut server2, &mut other, &mut wire2);
    let failure = other
        .outcome()
        .expect("ended")
        .as_ref()
        .expect_err("refused");
    assert!(failure.0.contains("refused the join"), "{failure}");
    assert!(duel_spec_for(0).is_err() && duel_spec_for(100_001).is_err());
}

#[test]
fn webduel_two_seats_start_together_and_all_three_end_on_the_two_seat_hash() {
    let (mut server_a, mut wire_a) = pair();
    let (mut server_b, mut wire_b) = pair();
    let mut host = DuelHost::with_seats(duel_spec().expect("spec"), 2).expect("host");
    let (mut first, mut second) = (
        DuelClient::new(0xA1).expect("client"),
        DuelClient::new(0xB2).expect("client"),
    );
    let (at_a, at_b) = (server_a.me, server_b.me);
    // The first seat alone does not start the match, however long the host ticks.
    first.poll(&mut wire_a, at_a);
    for _ in 0..50 {
        host.pump(&mut [&mut server_a, &mut server_b]);
        assert_eq!(
            host.tick(&mut [&mut server_a, &mut server_b], true)
                .expect("tick"),
            None
        );
        first.poll(&mut wire_a, at_a);
    }
    assert!(
        !host.started() && host.tick_now() == 0,
        "waiting for the second seat"
    );
    for _ in 0..ROUNDS {
        host.pump(&mut [&mut server_a, &mut server_b]);
        host.tick(&mut [&mut server_a, &mut server_b], false)
            .expect("tick");
        let done_a = first.poll(&mut wire_a, at_a);
        let done_b = second.poll(&mut wire_b, at_b);
        if done_a && done_b {
            break;
        }
    }
    let reference = reference_hash_for(DUEL_TICKS, &[0, 1]).expect("reference");
    let (a, b) = (
        first.outcome().expect("ended").as_ref().expect("converged"),
        second
            .outcome()
            .expect("ended")
            .as_ref()
            .expect("converged"),
    );
    assert_eq!((a.slot, b.slot), (0, 1));
    assert_eq!((a.hash, b.hash), (reference, reference));
    assert_eq!(host.final_hash(), Some(reference));
    assert_ne!(
        reference,
        reference_hash(0).expect("one seat"),
        "a second human is a different match"
    );
    // Each client predicted the other seat as AI, so the server corrected it.
    assert!(a.rollback_max > 0 && b.rollback_max > 0, "{a:?} {b:?}");
    assert!(DuelHost::with_seats(duel_spec().expect("spec"), 0).is_err());
    assert!(DuelHost::with_seats(duel_spec().expect("spec"), 3).is_err());
}

#[test]
fn webduel_against_the_ai_never_rolls_back() {
    let (mut server, mut wire) = pair();
    let (mut host, mut client) = (host(), DuelClient::new(31).expect("client"));
    play(&mut host, &mut server, &mut client, &mut wire);
    let done = client
        .outcome()
        .expect("ended")
        .as_ref()
        .expect("converged");
    assert_eq!((done.mispredicts, done.rollback_max), (0, 0));
}

#[test]
fn webduel_times_the_join_and_the_ping_on_the_callers_clock() {
    let (mut server, mut wire) = pair();
    let (mut host, mut client) = (host(), DuelClient::new(41).expect("client"));
    let at = server.me;
    // The join leaves at 100 ms and the grant is read at 145: the handshake is the first round trip.
    client.poll_at(&mut wire, at, 100);
    host.pump(&mut [&mut server]);
    client.poll_at(&mut wire, at, 145);
    assert!(client.seated());
    assert_eq!(client.stats().rtt_ms, Some(45));
    // The first Ready ping left in that same round (at 145); its echo is read at 182.
    host.pump(&mut [&mut server]);
    client.poll_at(&mut wire, at, 182);
    assert_eq!(client.stats().rtt_ms, Some(37));
    // No second ping until a second has passed since the first.
    client.poll_at(&mut wire, at, 500);
    host.pump(&mut [&mut server]);
    client.poll_at(&mut wire, at, 520);
    assert_eq!(client.stats().rtt_ms, Some(37));
    client.poll_at(&mut wire, at, 1_200);
    host.pump(&mut [&mut server]);
    client.poll_at(&mut wire, at, 1_290);
    assert_eq!(client.stats().rtt_ms, Some(90));
}

#[test]
fn webduel_ready_from_a_stranger_gets_no_echo() {
    let (mut server, mut wire) = pair();
    let mut host = host();
    let at = server.me;
    send_lobby(
        &mut wire,
        at,
        Msg::Ready {
            token: 99,
            ready: true,
        },
    )
    .expect("send");
    host.pump(&mut [&mut server]);
    assert!(wire.recv().is_empty(), "no seat, no echo");
}

#[test]
fn webduel_join_is_sent_again_when_it_went_unanswered() {
    let (mut server, mut wire) = pair();
    let mut client = DuelClient::new(51).expect("client");
    let at = server.me;
    client.poll_at(&mut wire, at, 0);
    assert_eq!(server.recv().len(), 1, "the first join");
    client.poll_at(&mut wire, at, 500);
    assert!(server.recv().is_empty(), "too early to ask again");
    client.poll_at(&mut wire, at, 1_000);
    assert_eq!(
        server.recv().len(),
        1,
        "the join is sent again after a second"
    );
}

#[test]
fn webduel_names_the_first_divergent_tick_while_the_duel_is_still_on() {
    let (mut server, mut wire) = pair();
    server.lie_from = Some(200);
    let (mut host, mut client) = (host(), DuelClient::new(61).expect("client"));
    play(&mut host, &mut server, &mut client, &mut wire);
    let failure = client
        .outcome()
        .expect("ended")
        .as_ref()
        .expect_err("the hashes part");
    assert!(
        failure.0.starts_with("desync at tick 200:"),
        "the first tick the server lied about: {failure}"
    );
    assert!(host.tick_now() < DUEL_TICKS, "named before the last tick");
}

#[test]
fn webduel_playing_line_shows_state_ping_and_rollback() {
    let waiting = DuelStats {
        tick: 0,
        server_tick: None,
        rtt_ms: Some(12),
        rollback_last: 0,
        rollback_max: 0,
        mispredicts: 0,
        quiet_ms: 0,
        rejected: 0,
    };
    assert_eq!(
        playing_line("websocket", &waiting, 600),
        "net: seated transport=websocket rtt=12ms, waiting for the match to start"
    );
    let running = DuelStats {
        server_tick: Some(122),
        rtt_ms: None,
        rollback_last: 3,
        rollback_max: 9,
        quiet_ms: 16,
        ..waiting
    };
    assert_eq!(
        playing_line("webtransport", &running, 3600),
        "net: playing transport=webtransport tick=123/3600 rtt=? rollback=3 max=9 quiet=16ms"
    );
}

#[test]
fn webduel_a_cheating_seat_is_refused_and_counted_and_the_honest_hash_is_unchanged() {
    let (mut server, mut wire) = pair();
    let (mut host, mut client) = (host(), DuelClient::new(71).expect("client"));
    let at = server.me;
    // Join, let the honest inputs of the first ticks in, then start cheating.
    client.poll(&mut wire, at);
    host.pump(&mut [&mut server]);
    client.poll(&mut wire, at);
    host.pump(&mut [&mut server]);
    let mut rounds = 0;
    while !client.poll(&mut wire, at) {
        rounds += 1;
        assert!(
            rounds < ROUNDS,
            "the duel stalled at host tick {}",
            host.tick_now()
        );
        let now = host.tick_now();
        if rounds % 25 == 1 {
            // A seat can only speak for its own slot.
            wire.send_unreliable(at, &input_bytes(now + 3, 1))
                .expect("send");
            // Far past the window.
            wire.send_unreliable(at, &input_bytes(now + 200, 0))
                .expect("send");
            // Not an input packet at all.
            wire.send_unreliable(at, &[1, 2, 3]).expect("send");
            // A different input for a tick already admitted.
            let mut rewrite = scripted_input(now + 3, 0);
            rewrite.mx = -100;
            let bytes = crate::session::encode_input(rewrite.tick, &[rewrite]).expect("encode");
            wire.send_unreliable(at, &bytes).expect("send");
        }
        if rounds == 40 {
            // A flood on one arrival tick.
            for _ in 0..30 {
                wire.send_unreliable(at, &input_bytes(now + 3, 0))
                    .expect("send");
            }
        }
        host.pump(&mut [&mut server]);
        host.tick(&mut [&mut server], false).expect("tick");
    }
    let done = client
        .outcome()
        .expect("ended")
        .as_ref()
        .expect("converged");
    assert_eq!(
        done.hash,
        reference_hash(0).expect("reference"),
        "the cheats changed nothing"
    );
    assert_eq!(host.final_hash(), Some(done.hash));
    let receipt = host.admission()[0];
    for kind in [
        Refusal::WrongSlot,
        Refusal::Future,
        Refusal::Malformed,
        Refusal::Rewrite,
        Refusal::Rate,
    ] {
        assert!(
            receipt.count(kind) > 0,
            "{} refusals counted: {receipt:?}",
            kind.name()
        );
    }
    assert!(receipt.cheats() >= 5);
    assert_eq!(
        receipt.accepted, DUEL_TICKS,
        "every honest input was admitted once"
    );
}
