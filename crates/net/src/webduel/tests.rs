//! The duel over in-memory transports: the host and the client are the same
//! state machines the binary and the page run, so these pin the protocol, the
//! hash the page and the native server compare, and the loss and rejoin paths.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::net::SocketAddr;
use std::rc::Rc;

use engine_core::mix64;

use super::*;
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
        },
        End {
            me: client,
            peer: server,
            inbox: to_client,
            outbox: to_server,
            loss: 0,
            draws: 0,
            dropped: 0,
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
        self.outbox.borrow_mut().push_back(Inbound {
            from: self.me,
            frame: InboundFrame::Datagram(bytes.to_vec()),
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
    let done = client.outcome().expect("ended").as_ref().expect("converged");
    let reference = reference_hash(0).expect("reference");
    assert_eq!(done.ticks, DUEL_TICKS);
    assert_eq!(done.hash, reference, "the page's hash is the plain server's");
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
    let done = client.outcome().expect("ended").as_ref().expect("converged");
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
