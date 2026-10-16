//! Lobby screen over the real net lobby: two screens, two UDP sockets on
//! loopback, one fight request. The screens only ever see host ids, slot rows
//! and commands; every address comes from a datagram, none is typed.

use std::net::{SocketAddr, UdpSocket};
use std::thread::sleep;
use std::time::Duration;

use engine_net::lobby::{
    Beacon, Browser, BuildId, Lobby, Msg, Phase, Slot, MAX_BEACONS, REFRESH_PERIOD_MS,
};
use engine_ui::lobby_ui::{
    HostId, HostRow, LobbyCommand, LobbyInput, LobbyLimits, LobbyPhase, LobbyScreen, LobbyUiError,
    SlotRow,
};
use engine_ui::menus::FightRequest;

const BUILD: BuildId = BuildId {
    version: 3,
    rules: 0x00C0_FFEE,
};
const HOST_TOKEN: u64 = 0xB0517;
const GUEST_TOKEN: u64 = 0xC11E47;
const SEED: u64 = 777;
const FIGHTERS: u8 = 4;

/// The screens take pacing and caps from the net lobby's own constants.
fn limits() -> LobbyLimits {
    LobbyLimits {
        refresh_period_ms: REFRESH_PERIOD_MS,
        max_hosts: MAX_BEACONS,
    }
}

/// Host side: the lobby's slots as screen rows for the player holding `me`.
fn host_rows(lobby: &Lobby, me: u64) -> Vec<SlotRow> {
    lobby.slots()[..usize::from(lobby.fighters())]
        .iter()
        .map(|slot| match *slot {
            Slot::Closed => SlotRow::Closed,
            Slot::Open => SlotRow::Open,
            Slot::Ai => SlotRow::Ai,
            Slot::Human { token, ready } => SlotRow::Human {
                you: token == me,
                ready,
            },
        })
        .collect()
}

/// Guest side: the wire carries no roster, so a guest knows only its own
/// slot and the match size; the other rows stay open.
fn guest_rows(slot: u8, fighters: u8, ready: bool) -> Vec<SlotRow> {
    (0..fighters)
        .map(|index| {
            if index == slot {
                SlotRow::Human { you: true, ready }
            } else {
                SlotRow::Open
            }
        })
        .collect()
}

/// Poll a nonblocking socket until datagrams arrive: loopback delivery is
/// fast but not instant under load. Bounded, no wall clock read.
fn recv_settle(sock: &UdpSocket) -> Vec<(Vec<u8>, SocketAddr)> {
    let mut out = Vec::new();
    let mut buf = [0u8; 64];
    for _ in 0..400 {
        while let Ok((n, from)) = sock.recv_from(&mut buf) {
            out.push((buf[..n].to_vec(), from));
        }
        if !out.is_empty() {
            break;
        }
        sleep(Duration::from_millis(5));
    }
    out
}

fn bound() -> (UdpSocket, SocketAddr) {
    let sock = UdpSocket::bind("127.0.0.1:0").expect("bind loopback");
    sock.set_nonblocking(true).expect("nonblocking");
    let addr = sock.local_addr().expect("local addr");
    (sock, addr)
}

#[test]
fn two_screens_go_from_the_host_list_to_one_fight_request_with_no_address_typed() {
    let (host_sock, host_addr) = bound();
    let (guest_sock, guest_addr) = bound();
    let mut host_screen = LobbyScreen::new(limits()).expect("host screen");
    let mut guest_screen = LobbyScreen::new(limits()).expect("guest screen");

    // The host presses Host: the glue forms the lobby, seats the host in it
    // and shows the rows.
    assert_eq!(
        host_screen.input(LobbyInput::Host),
        Ok(Some(LobbyCommand::Host))
    );
    let mut lobby = Lobby::new(BUILD, FIGHTERS, SEED).expect("lobby");
    assert_eq!(lobby.join(HOST_TOKEN, BUILD), Ok(0));
    host_screen
        .joined(&host_rows(&lobby, HOST_TOKEN), true)
        .expect("host in lobby");

    // The host beacons its lobby. The guest screen asks for a refresh, the
    // glue listens and learns the host from the datagram.
    let beacon = Beacon {
        port: host_addr.port(),
        fighters: FIGHTERS,
        tick: 0,
        hash: 0,
    };
    beacon.announce(&host_sock, guest_addr).expect("announce");
    let mut browser = Browser::new();
    assert_eq!(guest_screen.poll(0), Some(LobbyCommand::Refresh));
    assert!(
        browser.mark_refreshed(0),
        "net pacing accepts the first refresh"
    );
    let mut seen = None;
    for _ in 0..400 {
        seen = Beacon::listen_once(&guest_sock).expect("listen");
        if seen.is_some() {
            break;
        }
        sleep(Duration::from_millis(5));
    }
    let (heard, from) = seen.expect("beacon arrives");
    browser.observe(heard, from, 0).expect("observe");
    let sightings = browser.sightings();
    let host_list: Vec<HostRow> = sightings
        .iter()
        .enumerate()
        .map(|(index, sight)| HostRow {
            id: HostId(u16::try_from(index).expect("few hosts")),
            fighters: sight.beacon.fighters,
        })
        .collect();
    guest_screen.set_hosts(&host_list).expect("host list");
    assert_eq!(guest_screen.status(), "Pick a game and press confirm");

    // The guest presses confirm on the only host. The screen answers with a
    // host id; the glue maps it back to the sender it heard.
    let Ok(Some(LobbyCommand::Join(picked))) = guest_screen.input(LobbyInput::Confirm) else {
        panic!("confirm joins the highlighted host");
    };
    let chosen = sightings[usize::from(picked.0)];
    let target = SocketAddr::new(chosen.from.ip(), chosen.beacon.port);
    let request = Msg::JoinRequest {
        token: GUEST_TOKEN,
        build: BUILD,
    };
    guest_sock
        .send_to(&request.encode().expect("encode"), target)
        .expect("send join");
    let mut join = None;
    for (bytes, sender) in recv_settle(&host_sock) {
        if let Ok(Msg::JoinRequest { token, build }) = Msg::decode(&bytes) {
            join = Some((token, build, sender));
        }
    }
    let (token, build, guest_from) = join.expect("host sees the join");
    let slot = lobby.join(token, build).expect("grant");
    assert_eq!(slot, 1);
    let grant = Msg::JoinGrant {
        token,
        slot,
        fighters: FIGHTERS,
        build: BUILD,
    };
    host_sock
        .send_to(&grant.encode().expect("encode"), guest_from)
        .expect("send grant");
    host_screen
        .set_slots(&host_rows(&lobby, HOST_TOKEN))
        .expect("host sees the guest");
    let mut granted = None;
    for (bytes, _) in recv_settle(&guest_sock) {
        if let Ok(Msg::JoinGrant { slot, fighters, .. }) = Msg::decode(&bytes) {
            granted = Some((slot, fighters));
        }
    }
    let (my_slot, fighters) = granted.expect("guest sees the grant");
    guest_screen
        .joined(&guest_rows(my_slot, fighters, false), false)
        .expect("guest in lobby");
    assert_eq!(guest_screen.phase(), LobbyPhase::InLobby);

    // Roles hold: the guest cannot fill or start, the host cannot start
    // before everyone is ready.
    assert_eq!(
        guest_screen.input(LobbyInput::Start),
        Err(LobbyUiError::NotHost)
    );
    assert_eq!(
        host_screen.input(LobbyInput::Start),
        Err(LobbyUiError::NotReady)
    );

    // The guest readies up through its screen and posts the flag.
    let Ok(Some(LobbyCommand::SetReady(on))) = guest_screen.input(LobbyInput::ToggleReady) else {
        panic!("toggle posts a ready flag");
    };
    assert!(on);
    let ready = Msg::Ready {
        token: GUEST_TOKEN,
        ready: on,
    };
    guest_sock
        .send_to(&ready.encode().expect("encode"), target)
        .expect("send ready");
    guest_screen
        .set_slots(&guest_rows(my_slot, fighters, true))
        .expect("guest ready shown");
    for (bytes, _) in recv_settle(&host_sock) {
        if let Ok(Msg::Ready { token, ready }) = Msg::decode(&bytes) {
            lobby.set_ready(token, ready).expect("guest ready lands");
        }
    }

    // The host readies up and fills the open slots with AI, as its screen
    // commands.
    let Ok(Some(LobbyCommand::SetReady(host_on))) = host_screen.input(LobbyInput::ToggleReady)
    else {
        panic!("toggle posts a ready flag");
    };
    lobby.set_ready(HOST_TOKEN, host_on).expect("host ready");
    host_screen
        .set_slots(&host_rows(&lobby, HOST_TOKEN))
        .expect("rows after ready");
    assert_eq!(
        host_screen.input(LobbyInput::FillAi),
        Ok(Some(LobbyCommand::FillAi))
    );
    assert_eq!(lobby.ai_fill(), 2);
    host_screen
        .set_slots(&host_rows(&lobby, HOST_TOKEN))
        .expect("rows after fill");
    assert!(host_screen.can_start());
    assert_eq!(host_screen.status(), "Everyone is ready");

    // Start: the host's screen allows it, the net lobby issues the params
    // both sides load, the guest hears the start message.
    assert_eq!(
        host_screen.input(LobbyInput::Start),
        Ok(Some(LobbyCommand::Start))
    );
    let params = lobby.try_start().expect("start");
    host_screen
        .started(FightRequest {
            seed: params.seed,
            fighters: params.fighters,
        })
        .expect("host starts");
    host_sock
        .send_to(&params.as_msg().encode().expect("encode"), guest_from)
        .expect("send start");
    let mut heard_start = None;
    for (bytes, _) in recv_settle(&guest_sock) {
        if let Ok(Msg::Start { fighters, seed, .. }) = Msg::decode(&bytes) {
            heard_start = Some(FightRequest { seed, fighters });
        }
    }
    let guest_fight = heard_start.expect("guest hears the start");
    guest_screen.started(guest_fight).expect("guest starts");

    // One fight request, identical on both screens, valid for the menu hook.
    let host_fight = host_screen.take_fight().expect("host fight");
    assert_eq!(guest_screen.take_fight(), Some(host_fight));
    assert_eq!(
        host_fight,
        FightRequest {
            seed: SEED,
            fighters: FIGHTERS
        }
    );
    host_fight.validate().expect("legal fight request");
    assert_eq!(
        (host_screen.phase(), guest_screen.phase()),
        (LobbyPhase::Starting, LobbyPhase::Starting)
    );
    assert_eq!((lobby.human_count(), lobby.ai_count()), (2, 2));
    assert_eq!(lobby.phase(), Phase::Starting);
}

#[test]
fn the_screen_never_asks_for_a_refresh_faster_than_the_net_browser_allows() {
    let mut screen = LobbyScreen::new(limits()).expect("screen");
    let mut browser = Browser::new();
    let mut asked = 0u32;
    // Ten seconds of 16 ms frames.
    let mut now_ms = 0u64;
    while now_ms < 10_000 {
        if screen.poll(now_ms) == Some(LobbyCommand::Refresh) {
            asked += 1;
            assert!(
                browser.mark_refreshed(now_ms),
                "net pacing refused a refresh the screen asked for at {now_ms} ms"
            );
        }
        now_ms += 16;
    }
    // 4 Hz is 40 refreshes in 10 s; frame steps of 16 ms land on 256 ms.
    assert!((38..=41).contains(&asked), "{asked} refreshes in 10 s");
}
