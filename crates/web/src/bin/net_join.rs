//! `net_join --server HOST:PORT [--ticks N] [--token N] [--deadline-s N] [--quiet]`: a native
//! client of the dedicated server, the other seat beside a browser page (WEB9).
//!
//! Joins the `net_serve --udp` socket at `HOST:PORT`, plays the scripted duel through the
//! same client the page runs (`engine_net::webduel::DuelClient`: prediction, rollback onto
//! the server's state, ping, a rebuild when datagrams were lost) and prints one line when
//! the match ends: `net: transport=udp ticks=N hash=H peers=1`, the line a page prints for
//! the same match. Exit 0 when the client ended on the server's hash, 1 when it could not
//! (the line then says why, with the first divergent tick for a desync), 2 for a bad
//! command line or socket. Once a second a `net_join: net: playing ...` line goes to
//! standard error unless `--quiet`.
//!
//! `--ticks` must equal the server's (a different length is a different match and the
//! join is refused with both builds named). `--token` is the seat token (the same token
//! reclaims the same seat); the default is made from the process id and the clock.
//! `--deadline-s` (default 120) is how long to wait for the match, which with `--seats 2`
//! includes waiting for the other player. This bin lives in the web crate, not in
//! `engine-net`, because it reads the wall clock (the ping, the deadline); the simulation
//! crate never does.

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::net::{ToSocketAddrs, UdpSocket};
    use std::process::ExitCode;
    use std::thread::sleep;
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

    use engine_net::quic::{PeerId, UdpTransport};
    use engine_net::webduel::{net_line, playing_line, DuelClient, Script, DUEL_TICKS};

    /// One polling step of the loop.
    const STEP: Duration = Duration::from_millis(1);
    /// The transport name the line carries.
    const TRANSPORT: &str = "udp";

    struct Options {
        server: String,
        ticks: u32,
        token: u64,
        deadline_s: u64,
        quiet: bool,
    }

    fn parse(args: &[String]) -> Result<Options, String> {
        let mut server = None;
        let mut ticks = DUEL_TICKS;
        let mut token = clock_token();
        let mut deadline_s = 120;
        let mut quiet = false;
        let mut rest = args.iter();
        while let Some(flag) = rest.next() {
            if flag == "--quiet" {
                quiet = true;
                continue;
            }
            let value = rest.next().ok_or_else(|| format!("{flag} needs a value"))?;
            let bad = || format!("bad {flag} {value}");
            match flag.as_str() {
                "--server" => server = Some(value.clone()),
                "--ticks" => ticks = value.parse().map_err(|_| bad())?,
                "--token" => token = value.parse().map_err(|_| bad())?,
                "--deadline-s" => deadline_s = value.parse().map_err(|_| bad())?,
                other => {
                    return Err(format!(
                        "unknown flag {other}: --server HOST:PORT, --ticks N, --token N, \
                         --deadline-s N, --quiet"
                    ))
                }
            }
        }
        let server = server.ok_or("--server HOST:PORT is required")?;
        if token == 0 {
            return Err("--token must not be zero".to_owned());
        }
        Ok(Options {
            server,
            ticks,
            token,
            deadline_s,
            quiet,
        })
    }

    /// A nonzero token from the clock and the process id: two clients started together
    /// still differ.
    fn clock_token() -> u64 {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since| since.as_nanos());
        let low = u64::try_from(nanos & u128::from(u64::MAX)).unwrap_or(0);
        (low ^ u64::from(std::process::id()).rotate_left(40)) | 1
    }

    /// Play the duel; true when the client ended on the server's hash.
    fn run(args: &[String]) -> Result<bool, String> {
        let options = parse(args)?;
        let address = options
            .server
            .to_socket_addrs()
            .map_err(|error| format!("cannot resolve {}: {error}", options.server))?
            .next()
            .ok_or_else(|| format!("{} resolves to nothing", options.server))?;
        let server = PeerId(address);
        let socket = UdpSocket::bind("0.0.0.0:0").map_err(|error| format!("bind: {error}"))?;
        let mut link = UdpTransport::new(socket).map_err(|error| format!("udp: {error}"))?;
        let mut duel = DuelClient::playing(options.token, options.ticks, Box::new(Script))
            .map_err(|error| error.to_string())?;
        let started = Instant::now();
        let mut last_report = started;
        loop {
            let now = started.elapsed();
            let now_ms = u64::try_from(now.as_millis()).unwrap_or(u64::MAX);
            if duel.poll_at(&mut link, server, now_ms) {
                break;
            }
            if now > Duration::from_secs(options.deadline_s) {
                duel.fail(format!(
                    "no result within {} s (server tick {:?})",
                    options.deadline_s,
                    duel.server_tick()
                ));
                break;
            }
            if !options.quiet && last_report.elapsed() >= Duration::from_secs(1) {
                last_report = Instant::now();
                eprintln!(
                    "net_join: {}",
                    playing_line(TRANSPORT, &duel.stats(), options.ticks)
                );
            }
            sleep(STEP);
        }
        let outcome = duel.outcome().ok_or("the duel ended without a verdict")?;
        println!("{}", net_line(TRANSPORT, outcome));
        Ok(outcome.is_ok())
    }

    /// Run the client with the command line and turn the verdict into the exit code.
    pub fn main() -> ExitCode {
        let args: Vec<String> = std::env::args().skip(1).collect();
        match run(&args) {
            Ok(true) => ExitCode::SUCCESS,
            Ok(false) => ExitCode::from(1),
            Err(message) => {
                eprintln!("net_join: failed {message}");
                ExitCode::from(2)
            }
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn main() -> std::process::ExitCode {
    native::main()
}

/// The browser build has no sockets of its own to open: this bin is a PC tool.
#[cfg(target_arch = "wasm32")]
fn main() {}
