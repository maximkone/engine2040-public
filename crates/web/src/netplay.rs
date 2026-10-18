//! netplay (wasm32): the page plays a duel against a native dedicated server.
//!
//! `?join=host:port&cert=<sha256>` or `?server=ws://host:port` (WEB9, WEB12; see [`crate::netquery`])
//! starts it: the page races a WebTransport leg against a WebSocket leg
//! ([`engine_net::connect_race`]), keeps the first that connects, joins the server's duel
//! and plays it through the same client the native tests drive
//! ([`engine_net::webduel::DuelClient`]: prediction, rollback onto the server's state, a
//! ping, a rebuild when datagrams were lost, a desync named at its first divergent tick).
//! Without `&play` the seat plays the 600-tick script the proofs compare; with it the
//! keyboard, mouse and pad drive the seat ([`crate::browser::LivePilot`]). While it runs the
//! page keeps `#net` and `#netstat` current, and at the end prints one line to `#net`, the
//! console and `data-net` on `<body>`:
//!
//! - `net: transport=<webtransport|websocket> ticks=<n> hash=<hex> peers=<n>`
//! - `net: failed transport=<leg|none> reason=<text>` when the race, the join or the
//!   duel failed; a failed race names every leg that failed and why.
//!
//! The hash is the server's state hash of the last tick, which this page's own simulation
//! computed too. For the script it is the same on every run and the native server can print
//! the same number; for a live seat it is the number the server prints for that match.
//! `data-net-tick` carries the newest server tick for the headless checks.
//!
//! The driver is a `setTimeout` loop on the page's one thread. It does not need WebGPU, so
//! it runs on a page whose GPU failed too. Called from `index.html`, which hands over the
//! page address.

use std::net::SocketAddr;
use std::time::Duration;

use engine_net::browser::ws::WsClient;
use engine_net::browser::wt::WtClient;
use engine_net::browser::{ClientState, GateReceipt};
use engine_net::connect_race::{bounded, race, BoundedError, Leg, LegError, RaceError};
use engine_net::quic::{ControlKind, Inbound, PeerId, Transport};
use engine_net::session::NetError;
use engine_net::webduel::{net_line, playing_line, DuelClient, DuelStats, Pilot, Script};
use wasm_bindgen::prelude::*;

use crate::browser::LivePilot;
use crate::netquery::{parse_search, ws_blocked, NetTarget};
use crate::shell::{browser_clock, pause, set_body_attr, set_text};

/// How long a leg may stay silent before the race calls it failed: a firewall that drops
/// UDP looks exactly like this on WebTransport.
const LEG_DEADLINE_MS: i32 = 5_000;
/// How long the duel may take before the page gives up and says so, plus this much per
/// tick of the match (a live seat plays at the pace of the person and the link).
const DUEL_DEADLINE: Duration = Duration::from_secs(90);
const DEADLINE_PER_TICK: Duration = Duration::from_millis(20);
/// A live seat may also wait this long for the other player before the match starts.
const WAIT_FOR_PLAYER: Duration = Duration::from_secs(180);
/// Pause between two rounds of the driver loop.
const ROUND_MS: i32 = 2;
/// How often the status lines are rewritten.
const HUD: Duration = Duration::from_millis(100);

/// The connected leg behind one `Transport`.
enum Link {
    Ws(WsClient),
    Wt(WtClient),
}

impl Link {
    fn state(&self) -> ClientState {
        match self {
            Self::Ws(link) => link.state(),
            Self::Wt(link) => link.state(),
        }
    }

    /// Datagrams this leg shed because the link could not keep up.
    fn dropped(&self) -> u64 {
        match self {
            Self::Ws(link) => link.dropped_datagrams(),
            Self::Wt(link) => link.dropped_datagrams(),
        }
    }
}

impl Transport for Link {
    fn send_unreliable(&mut self, to: PeerId, bytes: &[u8]) -> Result<(), NetError> {
        match self {
            Self::Ws(link) => link.send_unreliable(to, bytes),
            Self::Wt(link) => link.send_unreliable(to, bytes),
        }
    }

    fn send_reliable(
        &mut self,
        to: PeerId,
        kind: ControlKind,
        bytes: &[u8],
    ) -> Result<(), NetError> {
        match self {
            Self::Ws(link) => link.send_reliable(to, kind, bytes),
            Self::Wt(link) => link.send_reliable(to, kind, bytes),
        }
    }

    fn recv(&mut self) -> Vec<Inbound> {
        match self {
            Self::Ws(link) => link.recv(),
            Self::Wt(link) => link.recv(),
        }
    }
}

/// The server's peer id as the client transports report it: they have one peer.
fn server_peer() -> PeerId {
    PeerId(SocketAddr::from(([0, 0, 0, 0], 0)))
}

/// A nonzero seat token from the page's random source.
fn random_token() -> u64 {
    let (high, low) = (
        js_sys::Math::random().to_bits(),
        js_sys::Math::random().to_bits(),
    );
    (high.rotate_left(32) ^ low) | 1
}

/// A leg's failure as the plain reason text the race names it by.
fn reason(error: BoundedError<LegError>) -> String {
    match error {
        BoundedError::Failed(leg) => leg.reason,
        silent @ BoundedError::Silent(_) => silent.to_string(),
    }
}

/// Whether the page itself was loaded over https (it then may not open `ws://` elsewhere).
fn page_is_https() -> bool {
    web_sys::window()
        .and_then(|window| window.location().protocol().ok())
        .is_some_and(|protocol| protocol == "https:")
}

fn show(line: &str, state: &str) {
    set_text("net", line);
    set_body_attr("data-net", state);
    if state == "failed" {
        web_sys::console::error_1(&JsValue::from_str(line));
    } else {
        web_sys::console::log_1(&JsValue::from_str(line));
    }
}

fn fail(transport: &str, reason: &str) -> String {
    let line = format!("net: failed transport={transport} reason={reason}");
    show(&line, "failed");
    line
}

/// The lines of a running duel: `#net` carries the status, `#netstat` the detail a person
/// wants (what to press, what the link is doing) and `data-net-tick` the newest server tick.
fn show_progress(leg: &str, play: bool, stats: &DuelStats, ticks: u32, dropped: u64) {
    set_text("net", &playing_line(leg, stats, ticks));
    if let Some(tick) = stats.server_tick {
        set_body_attr("data-net-tick", &tick.to_string());
    }
    let seat = if play {
        "you play: WASD move, F attack, Shift block, Space dodge, Q/E turn (click the page for mouse look)"
    } else {
        "the seat plays the proof script"
    };
    set_text(
        "netstat",
        &format!(
            "{seat}\nmispredicted ticks={} bad packets={} shed datagrams={dropped}\n{}",
            stats.mispredicts,
            stats.rejected,
            GateReceipt::read().line()
        ),
    );
}

/// Race the WebTransport leg against the WebSocket leg and keep the first that connects. A
/// failure names every leg that failed and why.
async fn connect(target: NetTarget, peer: PeerId) -> Result<(Leg, Link), RaceError> {
    let ws_url = target.ws.clone();
    let blocked = ws_blocked(page_is_https(), &ws_url);
    let ws_leg = async move {
        if let Some(why) = blocked {
            return Err(why);
        }
        bounded(
            async move { WsClient::connect(&ws_url, peer).await.map(Link::Ws) },
            pause(LEG_DEADLINE_MS),
            "5 s",
        )
        .await
        .map_err(reason)
    };
    let wt_leg = async move {
        let connect = async move {
            match target.wt {
                Some(url) => WtClient::connect(&url, target.cert, peer)
                    .await
                    .map(Link::Wt),
                None => Err(LegError {
                    leg: Leg::WebTransport,
                    reason: "not offered: the page address has no cert= key (join=host:port&cert=<sha256>)"
                        .to_owned(),
                }),
            }
        };
        bounded(connect, pause(LEG_DEADLINE_MS), "5 s")
            .await
            .map_err(reason)
    };
    let connected = race(wt_leg, ws_leg).await?;
    Ok((connected.leg, connected.value))
}

/// Drive the duel until it ends: poll, keep the status lines current, give up at the deadline
/// or when the connection closed under it.
async fn drive(
    duel: &mut DuelClient,
    link: &mut Link,
    leg: &str,
    peer: PeerId,
    play: bool,
    deadline: Duration,
) {
    let ticks = duel.ticks();
    let started = browser_clock();
    let mut last_hud: Option<Duration> = None;
    let mut closing = false;
    loop {
        let now = browser_clock();
        let now_ms = u64::try_from(now.as_millis()).unwrap_or(u64::MAX);
        if duel.poll_at(link, peer, now_ms) {
            return;
        }
        if last_hud.is_none_or(|last| now.saturating_sub(last) >= HUD) {
            last_hud = Some(now);
            show_progress(leg, play, &duel.stats(), ticks, link.dropped());
        }
        if let ClientState::Closed(why) = link.state() {
            // One more round reads whatever arrived before the close.
            if closing {
                duel.fail(format!("the connection ended before the duel did: {why}"));
                return;
            }
            closing = true;
        }
        if now.saturating_sub(started) > deadline {
            duel.fail(format!(
                "no result within {} s (server tick {:?})",
                deadline.as_secs(),
                duel.server_tick()
            ));
            return;
        }
        pause(ROUND_MS).await;
    }
}

/// Join the dedicated server named by the page address (`location.search`) and play the duel.
/// Resolves with the line it printed.
#[wasm_bindgen]
pub async fn run_net(search: String) -> String {
    set_body_attr("data-net", "running");
    set_text("net", "net: connecting...");
    let query = match parse_search(&search) {
        Ok(Some(query)) => query,
        Ok(None) => return fail("none", "the page address names no server (join=host:port)"),
        Err(error) => return fail("none", &error.to_string()),
    };
    let peer = server_peer();
    let (ticks, play) = (query.ticks, query.play);
    let (leg, mut link) = match connect(query.target, peer).await {
        Ok((leg, link)) => (leg.name(), link),
        Err(error) => return fail("none", &error.to_string()),
    };
    let pilot: Box<dyn Pilot> = if play {
        Box::new(LivePilot::new())
    } else {
        Box::new(Script)
    };
    let token = query.token.unwrap_or_else(random_token);
    let mut duel = match DuelClient::playing(token, ticks, pilot) {
        Ok(duel) => duel,
        Err(error) => return fail(leg, &error.to_string()),
    };
    let mut deadline = DUEL_DEADLINE + DEADLINE_PER_TICK * ticks;
    if play {
        deadline += WAIT_FOR_PLAYER;
    }
    drive(&mut duel, &mut link, leg, peer, play, deadline).await;
    let Some(outcome) = duel.outcome() else {
        return fail(leg, "the duel ended without a verdict");
    };
    let line = net_line(leg, outcome);
    show(&line, if outcome.is_ok() { "ok" } else { "failed" });
    let stats = duel.stats();
    set_text(
        "netstat",
        &format!(
            "ended: rollback max={} mispredicted ticks={} rtt={}\n{}",
            stats.rollback_max,
            stats.mispredicts,
            stats
                .rtt_ms
                .map_or_else(|| "?".to_owned(), |ms| format!("{ms}ms")),
            GateReceipt::read().line()
        ),
    );
    line
}
