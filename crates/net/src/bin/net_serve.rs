//! net-serve: the dedicated server a browser page (or a native client) plays a duel against.
//!
//! `net-serve [--ws ADDR] [--wt ADDR] [--udp ADDR] [--ticks N] [--seats N] [--live]
//! [--host NAME] [--page URL] [--repeat] [--tick-ms N] [--wait-ms N] [--join-wait-s N]
//! [--linger-ms N] [--out FILE] [--reference]`
//!
//! Serves the window's duel match (rules v2 on the kit walls, `--ticks` ticks, default
//! 600, two fighters) over a WebSocket on `--ws` (default `0.0.0.0:7050`, path `/e2040`),
//! when built with `--features webtransport` and given `--wt`, over WebTransport on a UDP
//! port with a freshly minted pinned certificate (its SHA-256 is printed for the page), and
//! when given `--udp`, to native clients (`net_join`, in the web crate) on a plain UDP socket. All three feed
//! the same match. The match starts when `--seats` seats (default 1) have joined; an empty
//! fighter is AI-filled. It runs in lockstep with the seats (a seat's input for a tick is
//! awaited for `--wait-ms`, default 3000, before the server authors the tick without it),
//! paces ticks at `--tick-ms` (default 16, 0 = as fast as the seats answer) and ends after
//! the last tick plus `--linger-ms` (default 2000) of resending the closing state.
//! `--repeat` then starts a fresh match on the same sockets instead of exiting.
//!
//! Prints `net-serve: ready ws=ws://HOST:PORT/e2040 ...` once the sockets are up, then
//! one `net-serve: ...` line per join, drop and stall, and last `net-serve: done ticks=N
//! hash=H reference=R match=B` (exit 0 when the match finished; the hash is the one a page
//! prints on its `net:` line). `reference` is the hash of the scripted duel for this
//! length and seat count, which scripted seats must equal. `--live` is for people at the
//! keys: the seats are not scripted, so the done line says `reference=none match=n/a`.
//! With `--wt` a `link` line carries the address to open: the Pages site with
//! `?join=HOST:PORT&cert=HASH` (`--host` names this machine for it and for the
//! certificate; `--page` replaces the site). `--reference` prints the scripted duel's
//! hash and exits. `--out FILE` also writes the done line.
//!
//! Time here is only patience and pacing (sleep counts); the tick never reads
//! a clock, so the hash does not depend on how fast the machine is.

use std::io::Write;
use std::net::{SocketAddr, ToSocketAddrs, UdpSocket};
use std::process::ExitCode;
use std::thread::sleep;
use std::time::Duration;

use engine_net::guard::{Refusal, SlotReceipt};
use engine_net::quic::{Transport, UdpTransport};
use engine_net::webduel::{
    duel_spec_for, reference_hash_for, DuelHost, HostEvent, SeatLink, DUEL_TICKS,
};
use engine_net::websocket::{DepartReason, WsServer, SESSION_PATH};

/// Default WebSocket address.
const WS_BIND: &str = "0.0.0.0:7050";
/// The public page a pinned-certificate link opens.
const PAGE: &str = "https://maximkone.github.io/engine2040-public/";
/// One polling step of every wait loop.
const STEP: Duration = Duration::from_millis(1);

struct Options {
    ws: String,
    wt: Option<String>,
    udp: Option<String>,
    ticks: u32,
    seats: u8,
    live: bool,
    host: Option<String>,
    page: String,
    repeat: bool,
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
        udp: None,
        ticks: DUEL_TICKS,
        seats: 1,
        live: false,
        host: None,
        page: PAGE.to_owned(),
        repeat: false,
        tick_ms: 16,
        wait_ms: 3_000,
        join_wait_s: 120,
        linger_ms: 2_000,
        out: None,
        reference: false,
    };
    let mut rest = args.iter();
    while let Some(flag) = rest.next() {
        match flag.as_str() {
            "--reference" => {
                options.reference = true;
                continue;
            }
            "--live" => {
                options.live = true;
                continue;
            }
            "--repeat" => {
                options.repeat = true;
                continue;
            }
            _ => {}
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
            "--udp" => options.udp = Some(value.clone()),
            "--host" => options.host = Some(value.clone()),
            "--page" => options.page.clone_from(value),
            "--ticks" => {
                options.ticks = u32::try_from(number("--ticks")?)
                    .map_err(|_| format!("bad --ticks {value}"))?;
            }
            "--seats" => {
                options.seats =
                    u8::try_from(number("--seats")?).map_err(|_| format!("bad --seats {value}"))?;
            }
            "--tick-ms" => options.tick_ms = number("--tick-ms")?,
            "--wait-ms" => options.wait_ms = number("--wait-ms")?,
            "--join-wait-s" => options.join_wait_s = number("--join-wait-s")?,
            "--linger-ms" => options.linger_ms = number("--linger-ms")?,
            "--out" => options.out = Some(value.clone()),
            other => {
                return Err(format!(
                    "unknown flag {other}: --ws ADDR, --wt ADDR, --udp ADDR, --ticks N, --seats N, \
                     --live, --host NAME, --page URL, --repeat, --tick-ms N, --wait-ms N, \
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

fn describe(event: &HostEvent, names: &[&str]) -> String {
    match event {
        HostEvent::Joined { slot, link, rejoin } => format!(
            "joined slot={slot} via={} peer={}{}",
            names.get(link.via).unwrap_or(&"?"),
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

/// The address a page opens to join: the Pages site with the shorthand `join=HOST:PORT`
/// when one port serves both legs, else the explicit keys. `cert` is the pin (64 hex
/// digits), `host` the machine's name as the operator's browser reaches it.
fn page_link(options: &Options, host: &str, ws_port: u16, wt_port: u16, cert: &str) -> String {
    let mut query = if ws_port == wt_port {
        format!("join={host}:{ws_port}&cert={cert}")
    } else {
        format!(
            "server=ws://{host}:{ws_port}&wt=https://{host}:{wt_port}{SESSION_PATH}&cert={cert}"
        )
    };
    if options.ticks != DUEL_TICKS {
        query.push_str(&format!("&ticks={}", options.ticks));
    }
    if options.live {
        query.push_str("&play");
    }
    format!("{}?{query}", options.page)
}

/// The listening sockets: the WebSocket server first, then WebTransport, then plain UDP,
/// the ones that are off left out. `names` is that list, which `SeatLink::via` indexes.
struct Listeners {
    ws: WsServer,
    #[cfg(feature = "webtransport")]
    wt: Option<engine_net::webtransport::WtServer>,
    udp: Option<UdpTransport>,
    names: Vec<&'static str>,
}

impl Listeners {
    fn bind(options: &Options) -> Result<Self, String> {
        let ws = WsServer::bind(resolve(&options.ws)?)
            .map_err(|error| format!("bind {}: {error}", options.ws))?;
        #[cfg(feature = "webtransport")]
        let wt = match &options.wt {
            Some(text) => Some(bind_webtransport(options, text, ws.local_addr().port())?),
            None => None,
        };
        #[cfg(not(feature = "webtransport"))]
        if options.wt.is_some() {
            return Err("--wt needs a build with --features webtransport".to_owned());
        }
        let udp = match &options.udp {
            Some(text) => {
                let socket = UdpSocket::bind(resolve(text)?)
                    .map_err(|error| format!("bind {text}: {error}"))?;
                Some(UdpTransport::new(socket).map_err(|error| format!("udp {text}: {error}"))?)
            }
            None => None,
        };
        let mut names = vec!["websocket"];
        #[cfg(feature = "webtransport")]
        if wt.is_some() {
            names.push("webtransport");
        }
        if udp.is_some() {
            names.push("udp");
        }
        Ok(Self {
            ws,
            #[cfg(feature = "webtransport")]
            wt,
            udp,
            names,
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

    /// The transports in the order `SeatLink::via` indexes them (`names`).
    fn transports(&mut self) -> Vec<&mut dyn Transport> {
        let mut list: Vec<&mut dyn Transport> = vec![&mut self.ws];
        #[cfg(feature = "webtransport")]
        if let Some(wt) = self.wt.as_mut() {
            list.push(wt);
        }
        if let Some(udp) = self.udp.as_mut() {
            list.push(udp);
        }
        list
    }
}

/// Bind the WebTransport server with a fresh pinned certificate and print the pin and
/// the link a page opens. `ws_port` is the WebSocket port, to pick the link's shape.
#[cfg(feature = "webtransport")]
fn bind_webtransport(
    options: &Options,
    text: &str,
    ws_port: u16,
) -> Result<engine_net::webtransport::WtServer, String> {
    use engine_net::webtransport::cert::{PinnedCert, MINT_VALIDITY_DAYS};
    let mut names = vec!["localhost", "127.0.0.1", "::1"];
    names.extend(options.host.as_deref());
    let cert = PinnedCert::mint(names, MINT_VALIDITY_DAYS)
        .map_err(|error| format!("certificate: {error:?}"))?;
    let server = engine_net::webtransport::WtServer::bind(resolve(text)?, &cert)
        .map_err(|error| format!("bind {text}: {error}"))?;
    say(&format!(
        "webtransport https://{}{SESSION_PATH} cert-sha256={}",
        server.local_addr(),
        cert.hash_hex()
    ));
    let host = options.host.as_deref().unwrap_or("<this-machine>");
    say(&format!(
        "link {}",
        page_link(
            options,
            host,
            ws_port,
            server.local_addr().port(),
            &cert.hash_hex()
        )
    ));
    Ok(server)
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
        say(&describe(&event, &listeners.names));
    }
    if !host.started() {
        waits.join += 1;
        if waits.join > options.join_wait_s * 1_000 {
            return Err(format!("nobody joined within {} s", options.join_wait_s));
        }
    }
    Ok(host.is_finished() && waits.linger > options.linger_ms)
}

/// One line of what the input guard did with a slot's traffic: packets, inputs admitted and
/// every refusal by kind (a cheating seat shows here, an honest one reads `refused=none`).
fn admission_line(receipt: &SlotReceipt) -> String {
    let refused: Vec<String> = Refusal::ALL
        .iter()
        .filter(|kind| receipt.count(**kind) > 0)
        .map(|kind| format!("{}={}", kind.name(), receipt.count(*kind)))
        .collect();
    format!(
        "admission slot={} packets={} accepted={} repeats={} sanitized={} cheats={} refused={}",
        receipt.slot,
        receipt.packets,
        receipt.accepted,
        receipt.repeats,
        receipt.sanitized,
        receipt.cheats(),
        if refused.is_empty() {
            "none".to_owned()
        } else {
            refused.join(",")
        },
    )
}

/// The hash a scripted match of these options ends on: every seat plays the script.
fn scripted_hash(options: &Options) -> Result<u64, String> {
    let slots: Vec<u8> = (0..options.seats).collect();
    reference_hash_for(options.ticks, &slots).map_err(|error| error.to_string())
}

fn run(args: &[String]) -> Result<(), String> {
    let options = parse(args)?;
    if options.reference {
        say(&format!(
            "reference ticks={} seats={} hash={:016x}",
            options.ticks,
            options.seats,
            scripted_hash(&options)?
        ));
        return Ok(());
    }
    let reference = (!options.live)
        .then(|| scripted_hash(&options))
        .transpose()?;
    let mut listeners = Listeners::bind(&options)?;
    say(&format!(
        "ready ws=ws://{}{SESSION_PATH} ticks={} seats={} reference={}",
        listeners.ws.local_addr(),
        options.ticks,
        options.seats,
        reference.map_or_else(|| "none".to_owned(), |hash| format!("{hash:016x}")),
    ));
    loop {
        let spec = duel_spec_for(options.ticks).map_err(|error| error.to_string())?;
        let mut host =
            DuelHost::with_seats(spec, options.seats).map_err(|error| error.to_string())?;
        let mut waits = Waits::default();
        while !serve_once(&mut host, &mut listeners, &options, &mut waits)? {
            sleep(STEP);
        }
        let hash = host.final_hash().ok_or("the match did not finish")?;
        for receipt in host.admission().iter().filter(|r| r.packets > 0) {
            say(&admission_line(receipt));
        }
        let line = format!(
            "done ticks={} hash={hash:016x} reference={} match={}",
            host.tick_now(),
            reference.map_or_else(|| "none".to_owned(), |hash| format!("{hash:016x}")),
            reference.map_or_else(|| "n/a".to_owned(), |want| (want == hash).to_string()),
        );
        say(&line);
        // The latest-only WebSocket queue: frames the writer actually sent and
        // frames a newer snapshot superseded while the writer was stalled.
        say(&listeners.ws.latest_only().line());
        if let Some(path) = &options.out {
            std::fs::write(path, format!("net-serve: {line}\n"))
                .map_err(|error| format!("write {path}: {error}"))?;
        }
        if !options.repeat {
            return Ok(());
        }
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Err(message) = run(&args) {
        eprintln!("net-serve: failed {message}");
        return ExitCode::from(2);
    }
    ExitCode::SUCCESS
}
