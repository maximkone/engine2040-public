//! netplay (wasm32): the page plays a scripted duel against a native dedicated server.
//!
//! `?server=ws://host:port` (WEB9, WEB12) starts it: the page races a WebTransport leg
//! against a WebSocket leg ([`engine_net::connect_race`]), keeps the first that connects,
//! joins the server's duel, plays 600 scripted ticks through the same client the native
//! tests drive ([`engine_net::webduel::DuelClient`]: prediction, rollback onto the server's
//! state, a rebuild when datagrams were lost) and prints one line to `#net`, the console
//! and `data-net` on `<body>`:
//!
//! - `net: transport=<webtransport|websocket> ticks=<n> hash=<hex> peers=<n>`
//! - `net: failed transport=<leg|none> reason=<text>` when the race, the join or the
//!   duel failed; a failed race names every leg that failed and why.
//!
//! The hash is the server's state hash of the last tick, which this page's own simulation
//! computed too. The script is the tape; the live input path (WEB7) is not mixed in, so the
//! hash is the same on every run and the native server can print the same number.
//!
//! The driver is a `setTimeout` loop on the page's one thread. It does not need WebGPU, so
//! it runs on a page whose GPU failed too. Called from `index.html`, which reads the query.

use std::net::SocketAddr;

use engine_net::browser::ws::WsClient;
use engine_net::browser::wt::WtClient;
use engine_net::browser::ClientState;
use engine_net::connect_race::{bounded, race, BoundedError, Leg, LegError};
use engine_net::quic::{ControlKind, Inbound, PeerId, Transport};
use engine_net::session::NetError;
use engine_net::webduel::{net_line, DuelClient};
use wasm_bindgen::prelude::*;

use crate::netquery::parse_target;
use crate::shell::{now_ms, pause, set_body_attr, set_text};

/// How long a leg may stay silent before the race calls it failed: a firewall that drops
/// UDP looks exactly like this on WebTransport.
const LEG_DEADLINE_MS: i32 = 5_000;
/// How long the whole duel may take before the page gives up and says so.
const DUEL_DEADLINE_MS: f64 = 90_000.0;
/// Pause between two rounds of the driver loop.
const ROUND_MS: i32 = 2;

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

/// Join the dedicated server named by the page address and play the scripted duel. Resolves
/// with the line it printed. `wt` and `cert` are the optional WebTransport leg (see
/// `netquery`); without them only the WebSocket leg runs.
#[wasm_bindgen]
pub async fn run_net(server: String, wt: Option<String>, cert: Option<String>) -> String {
    set_body_attr("data-net", "running");
    set_text("net", "net: connecting...");
    let target = match parse_target(&server, wt.as_deref(), cert.as_deref()) {
        Ok(target) => target,
        Err(error) => return fail("none", &error.to_string()),
    };
    let peer = server_peer();

    let ws_url = target.ws.clone();
    let ws_leg = async move {
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
                    reason: "not offered: the page address has no wt= and cert= keys".to_owned(),
                }),
            }
        };
        bounded(connect, pause(LEG_DEADLINE_MS), "5 s")
            .await
            .map_err(reason)
    };
    let connected = match race(wt_leg, ws_leg).await {
        Ok(connected) => connected,
        Err(error) => return fail("none", &error.to_string()),
    };
    let (leg, mut link) = (connected.leg.name(), connected.value);

    let mut duel = match DuelClient::new(random_token()) {
        Ok(duel) => duel,
        Err(error) => return fail(leg, &error.to_string()),
    };
    let started = now_ms();
    let mut closing = false;
    loop {
        if duel.poll(&mut link, peer) {
            break;
        }
        if let ClientState::Closed(why) = link.state() {
            // One more round reads whatever arrived before the close.
            if closing {
                duel.fail(format!("the connection ended before the duel did: {why}"));
                break;
            }
            closing = true;
        }
        if now_ms() - started > DUEL_DEADLINE_MS {
            duel.fail(format!(
                "no result within {} s (server tick {:?})",
                DUEL_DEADLINE_MS / 1000.0,
                duel.server_tick()
            ));
            break;
        }
        pause(ROUND_MS).await;
    }
    let Some(outcome) = duel.outcome() else {
        return fail(leg, "the duel ended without a verdict");
    };
    let line = net_line(leg, outcome);
    show(&line, if outcome.is_ok() { "ok" } else { "failed" });
    line
}
