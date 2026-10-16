//! net-relay: the self-hosted relay for bouts across two networks.
//!
//! `net-relay [--bind ADDR] [--token N]` answers STUN binding requests with
//! the caller's public address, registers clients under the token, and
//! forwards between registered clients on one UDP port (default
//! `0.0.0.0:7041`) until killed. Run it where both machines can reach it: a
//! public host, or a port forwarded through one router. Both machines then
//! run `net-bout on30-serve` / `on30-join` with `--relay HOST:PORT` and the
//! same `--token` (default: the relay default token).
//! Prints one line whenever a client registers and every 1000 forwards.

use std::net::{SocketAddr, ToSocketAddrs, UdpSocket};
use std::process::ExitCode;

use engine_net::courier::serve_relay;
use engine_net::relay::{RelayConfig, RelayHub};

/// Default UDP address the relay listens on.
const RELAY_BIND: &str = "0.0.0.0:7041";

fn parse(args: &[String]) -> Result<(SocketAddr, RelayConfig), String> {
    let mut bind = String::from(RELAY_BIND);
    let mut config = RelayConfig::default();
    let mut rest = args.iter();
    while let Some(flag) = rest.next() {
        let value = rest.next().ok_or_else(|| format!("{flag} needs a value"))?;
        match flag.as_str() {
            "--bind" => bind.clone_from(value),
            "--token" => {
                let token = match value.strip_prefix("0x") {
                    Some(hex) => u64::from_str_radix(&hex.replace('_', ""), 16),
                    None => value.parse(),
                };
                config.token = token.map_err(|_| format!("bad --token {value}"))?;
            }
            other => return Err(format!("unknown flag {other}: --bind ADDR, --token N")),
        }
    }
    let config = RelayConfig::new(config.token, config.max_peers, config.queue_cap)
        .map_err(|err| format!("relay config: {err}"))?;
    let addr = bind
        .to_socket_addrs()
        .map_err(|err| format!("cannot resolve {bind}: {err}"))?
        .next()
        .ok_or_else(|| format!("{bind} resolves to nothing"))?;
    Ok((addr, config))
}

fn run(args: &[String]) -> Result<(), String> {
    let (bind, config) = parse(args)?;
    let sock = UdpSocket::bind(bind).map_err(|err| format!("bind {bind}: {err}"))?;
    let local = sock.local_addr().map_err(|err| err.to_string())?;
    println!("net-relay: serving on {local} (token {:#x})", config.token);
    let mut hub = RelayHub::new(config);
    let mut shown = (0, 0);
    serve_relay(&sock, &mut hub, |hub| {
        let (stats, relay) = (hub.stats(), hub.server().stats());
        let now = (stats.registered, relay.forwarded / 1000);
        if now != shown {
            shown = now;
            println!(
                "net-relay: peers={} registered={} denied={} stun={} forwarded={} bytes={} refused={}",
                hub.server().peer_count(),
                stats.registered,
                stats.denied,
                stats.stun,
                relay.forwarded,
                relay.bytes_forwarded,
                stats.refused
            );
        }
        true
    })
    .map_err(|err| err.to_string())
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("net-relay: {message}");
            ExitCode::from(2)
        }
    }
}
