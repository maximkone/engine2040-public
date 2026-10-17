//! net-serve: the dedicated server a browser page plays a scripted duel against.
//!
//! `net-serve [--ws ADDR] [--wt ADDR] [--tick-ms N] [--wait-ms N] [--join-wait-s N]
//! [--linger-ms N] [--out FILE] [--reference]`
//!
//! Serves the window's duel match (rules v2 on the kit walls, 600 ticks, one
//! seat, the other fighter AI) over a WebSocket on `--ws` (default
//! `0.0.0.0:7050`, path `/e2040`) and, when built with `--features
//! webtransport` and given `--wt`, over WebTransport on a UDP port with a
//! freshly minted pinned certificate (its SHA-256 is printed for the page).
//! The match starts with the first join, runs in lockstep with the seat (the
//! seat's input for a tick is awaited for `--wait-ms`, default 3000, before
//! the server authors the tick without it), paces ticks at `--tick-ms`
//! (default 16, 0 = as fast as the seat answers) and ends after the last tick
//! plus `--linger-ms` (default 2000) of resending the closing state.
//!
//! Prints `net-serve: ready ws=ws://HOST:PORT/e2040` once the socket is up,
//! then one `net-serve: ...` line per join, drop and stall, and last
//! `net-serve: done ticks=N hash=H reference=R match=B` (exit 0 when the
//! match finished; the hash is the one a page prints on its `net:` line).
//! `--reference` prints the scripted duel's hash and exits: the offline
//! answer both ends must equal. `--out FILE` also writes the done line.
//!
//! Time here is only patience and pacing (sleep counts); the tick never reads
//! a clock, so the hash does not depend on how fast the machine is.

use std::io::Write;
use std::net::{SocketAddr, ToSocketAddrs};
use std::process::ExitCode;
use std::thread::sleep;
use std::time::Duration;

use engine_net::quic::Transport;
use engine_net::webduel::{duel_spec, reference_hash, DuelHost, HostEvent, SeatLink, DUEL_TICKS};
use engine_net::websocket::{DepartReason, WsServer, SESSION_PATH};

/// Default WebSocket address.
const WS_BIND: &str = "0.0.0.0:7050";
/// One polling step of every wait loop.
const STEP: Duration = Duration::from_millis(1);

struct Options {
    ws: String,
    wt: Option<String>,
    tick_ms: u64,
    wait_ms: u64,
    join_wait_s: u64,
    linger_ms: u64,
    out: Option<String>,
    reference: bool,
}

fn parse(args: &[String]) -> Result<Options, String> {
    let mut options = Options {
        ws: WS_BIND.to_owned(),
        wt: None,
        tick_ms: 16,
        wait_ms: 3_000,
        join_wait_s: 120,
        linger_ms: 2_000,
        out: None,
        reference: false,
    };
    let mut rest = args.iter();
    while let Some(flag) = rest.next() {
        if flag == "--reference" {
            options.reference = true;
            continue;
        }
        let value = rest.next().ok_or_else(|| format!("{flag} needs a value"))?;
        let number = |name: &str| {
            value
                .parse::<u64>()
                .map_err(|_| format!("bad {name} {value}"))
        };
        match flag.as_str() {
            "--ws" => options.ws.clone_from(value),
            "--wt" => options.wt = Some(value.clone()),
            "--tick-ms" => options.tick_ms = number("--tick-ms")?,
            "--wait-ms" => options.wait_ms = number("--wait-ms")?,
            "--join-wait-s" => options.join_wait_s = number("--join-wait-s")?,
            "--linger-ms" => options.linger_ms = number("--linger-ms")?,
            "--out" => options.out = Some(value.clone()),
            other => {
                return Err(format!(
                    "unknown flag {other}: --ws ADDR, --wt ADDR, --tick-ms N, --wait-ms N, \
                     --join-wait-s N, --linger-ms N, --out FILE, --reference"
                ))
            }
        }
    }
    Ok(options)
}

fn resolve(text: &str) -> Result<SocketAddr, String> {
    match text.to_socket_addrs() {
        Ok(mut found) => found
            .next()
            .ok_or_else(|| format!("{text} resolves to nothing")),
        Err(error) => Err(format!("cannot resolve {text}: {error}")),
    }
}

fn say(line: &str) {
    println!("net-serve: {line}");
    // A supervising script reads the log live.
    let _ = std::io::stdout().flush();
}

fn describe(event: &HostEvent) -> String {
    match event {
        HostEvent::Joined { slot, link, rejoin } => format!(
            "joined slot={slot} via={} peer={}{}",
            ["websocket", "webtransport"].get(link.via).unwrap_or(&"?"),
            link.peer.0,
            if *rejoin { " (rejoin)" } else { "" }
        ),
        HostEvent::Refused { link, reason } => {
            format!("refused peer={} reason={reason:?}", link.peer.0)
        }
        HostEvent::Dropped { slot, why } => format!("dropped slot={slot} why={why}"),
        HostEvent::Stalled { slot, tick } => format!("stalled slot={slot} tick={tick}"),
    }
}

/// The listening sockets: the WebSocket server (transport 0) and, when asked
/// and built for it, the WebTransport server (transport 1).
struct Listeners {
    ws: WsServer,
    #[cfg(feature = "webtransport")]
    wt: Option<engine_net::webtransport::WtServer>,
}

impl Listeners {
    fn bind(options: &Options) -> Result<Self, String> {
        let ws = WsServer::bind(resolve(&options.ws)?)
            .map_err(|error| format!("bind {}: {error}", options.ws))?;
        #[cfg(feature = "webtransport")]
        let wt = match &options.wt {
            Some(text) => {
                use engine_net::webtransport::cert::{PinnedCert, MAX_VALIDITY_DAYS};
                let cert = PinnedCert::mint(["localhost", "127.0.0.1", "::1"], MAX_VALIDITY_DAYS)
                    .map_err(|error| format!("certificate: {error:?}"))?;
                let server = engine_net::webtransport::WtServer::bind(resolve(text)?, &cert)
                    .map_err(|error| format!("bind {text}: {error}"))?;
                say(&format!(
                    "webtransport https://{}{SESSION_PATH} cert-sha256={}",
                    server.local_addr(),
                    cert.hash_hex()
                ));
                Some(server)
            }
            None => None,
        };
        #[cfg(not(feature = "webtransport"))]
        if options.wt.is_some() {
            return Err("--wt needs a build with --features webtransport".to_owned());
        }
        Ok(Self {
            ws,
            #[cfg(feature = "webtransport")]
            wt,
        })
    }

    /// Tell the host about every session that ended since the last call.
    fn report_departures(&mut self, host: &mut DuelHost) {
        for departure in self.ws.take_departures() {
            let why = match departure.reason {
                DepartReason::PeerClosed => "the page closed the socket".to_owned(),
                other => format!("{other:?}"),
            };
            let link = SeatLink {
                via: 0,
                peer: departure.peer,
            };
            host.depart(link, &why);
        }
        #[cfg(feature = "webtransport")]
        if let Some(wt) = self.wt.as_mut() {
            for departure in wt.take_departures() {
                let link = SeatLink {
                    via: 1,
                    peer: departure.peer,
                };
                host.depart(link, &format!("{:?}", departure.reason));
            }
        }
    }

    /// The transports in the order `SeatLink::via` indexes them.
    fn transports(&mut self) -> Vec<&mut dyn Transport> {
        let mut list: Vec<&mut dyn Transport> = vec![&mut self.ws];
        #[cfg(feature = "webtransport")]
        if let Some(wt) = self.wt.as_mut() {
            list.push(wt);
        }
        list
    }
}

/// Wait counters of the loop, one per millisecond step.
#[derive(Default)]
struct Waits {
    join: u64,
    patience: u64,
    pacing: u64,
    linger: u64,
}

/// One millisecond of serving: read the sockets, author a tick when the seat
/// is ready or patience ran out, resend the closing state while lingering.
/// Returns true once the match is over and the linger has passed.
fn serve_once(
    host: &mut DuelHost,
    listeners: &mut Listeners,
    options: &Options,
    waits: &mut Waits,
) -> Result<bool, String> {
    listeners.report_departures(host);
    let mut transports = listeners.transports();
    host.pump(&mut transports);
    if host.is_finished() {
        waits.linger += 1;
        if waits.linger.is_multiple_of(50) {
            host.resend_tip(&mut transports);
        }
    } else if waits.pacing > 0 {
        waits.pacing -= 1;
    } else {
        let give_up = waits.patience >= options.wait_ms;
        let authored = host
            .tick(&mut transports, give_up)
            .map_err(|error| format!("tick {}: {error}", host.tick_now()))?;
        if authored.is_some() {
            waits.pacing = options.tick_ms;
        } else if host.started() && host.blocked() {
            waits.patience += 1;
        }
        if give_up || authored.is_some() {
            waits.patience = 0;
        }
    }
    for event in host.take_events() {
        say(&describe(&event));
    }
    if !host.started() {
        waits.join += 1;
        if waits.join > options.join_wait_s * 1_000 {
            return Err(format!("nobody joined within {} s", options.join_wait_s));
        }
    }
    Ok(host.is_finished() && waits.linger > options.linger_ms)
}

fn run(args: &[String]) -> Result<(), String> {
    let options = parse(args)?;
    let reference = reference_hash(0).map_err(|error| error.to_string())?;
    if options.reference {
        say(&format!(
            "reference ticks={DUEL_TICKS} hash={reference:016x}"
        ));
        return Ok(());
    }
    let spec = duel_spec().map_err(|error| error.to_string())?;
    let mut host = DuelHost::new(spec).map_err(|error| error.to_string())?;
    let mut listeners = Listeners::bind(&options)?;
    say(&format!(
        "ready ws=ws://{}{SESSION_PATH} ticks={DUEL_TICKS} reference={reference:016x}",
        listeners.ws.local_addr()
    ));
    let mut waits = Waits::default();
    while !serve_once(&mut host, &mut listeners, &options, &mut waits)? {
        sleep(STEP);
    }
    let hash = host.final_hash().ok_or("the match did not finish")?;
    let line = format!(
        "done ticks={} hash={hash:016x} reference={reference:016x} match={}",
        host.tick_now(),
        hash == reference
    );
    say(&line);
    if let Some(path) = &options.out {
        std::fs::write(path, format!("net-serve: {line}\n"))
            .map_err(|error| format!("write {path}: {error}"))?;
    }
    Ok(())
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Err(message) = run(&args) {
        eprintln!("net-serve: failed {message}");
        return ExitCode::from(2);
    }
    ExitCode::SUCCESS
}
