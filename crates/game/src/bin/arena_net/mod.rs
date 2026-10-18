//! arena_net: the window's network seat. `--host` plays slot 0 and prints the
//! invitation, `--join ADDR CODE` plays slot 1, `--relay ADDR` hosts through a
//! self-hosted relay (`net_relay`), `--bind ADDR` picks the host's socket,
//! `--net-fault LAT,JIT,LOSS,REORDER` (or `lan`/`wan`) drops, delays and
//! reorders what this process sends, and `--headless` plays the scripted
//! session with no window (the loopback proof). The seat itself lives in
//! `engine_game::net_seat`; this module parses, opens, drives and prints.
//!
//! Automated runs (`--exit`, `--headless`) drive the local slot with the pure
//! `net_seat::script_input`, so two runs send the same streams and must end
//! on the same hash; interactive play reads the keyboard and the pad.

use std::net::{SocketAddr, ToSocketAddrs};

use engine_game::net_seat::{lan_ip, play_headless, script_input, NetSeat, SeatNote, DEFAULT_PORT};
use engine_game::netgame::{Invite, NetRules};
use engine_game::GameError;
use engine_net::link::LinkProfile;

use super::*;

/// Wall bound of a headless session on top of its ticks at 60 Hz (joining,
/// stalls on a faulty link, the host's linger).
const HEADLESS_SLACK_MS: u64 = 60_000;
/// Fault-link seeds: fixed per role, so a faulted run is repeatable.
const FAULT_SEED_HOST: u64 = 0xA2E4_0001;
const FAULT_SEED_GUEST: u64 = 0xA2E4_0002;

/// Which seat the flags asked for.
#[derive(Clone, PartialEq, Eq, Debug)]
enum NetRole {
    Host,
    Join { target: SocketAddr, code: String },
}

/// The network flags, split off before `WinArgs` (which refuses unknown flags).
#[derive(Clone, PartialEq, Debug)]
pub(super) struct NetArgs {
    role: NetRole,
    bind: Option<SocketAddr>,
    relay: Option<SocketAddr>,
    fault: Option<LinkProfile>,
    /// Play the scripted session without a window.
    pub(super) headless: bool,
}

/// A running seat inside the window.
pub(super) struct NetPlay {
    seat: NetSeat,
    /// Drive the local slot with the script (automated `--exit` runs).
    script: bool,
    /// The seat's clock origin.
    clock: Instant,
    /// The receipt line already printed.
    reported: bool,
    /// A seat failure waiting for the window to end on it.
    fatal: Option<WinError>,
}

/// One host or relay address: `ip:port` or `name:port`, resolved now.
fn resolve(flag: &str, text: &str) -> Result<SocketAddr, String> {
    text.to_socket_addrs()
        .map_err(|err| format!("{flag} {text}: {err}"))?
        .next()
        .ok_or_else(|| format!("{flag} {text}: resolves to nothing"))
}

/// A fault profile: `lan`, `wan`, or `LATENCY,JITTER,LOSS,REORDER` (ticks,
/// ticks, percent, percent).
fn parse_fault(text: &str) -> Result<LinkProfile, String> {
    match text {
        "lan" => return Ok(LinkProfile::LAN),
        "wan" => return Ok(LinkProfile::WAN),
        _ => {}
    }
    let parts: Vec<u32> = text
        .split(',')
        .map(|part| part.trim().parse::<u32>())
        .collect::<Result<_, _>>()
        .map_err(|_| format!("--net-fault {text}: want lan, wan or LAT,JIT,LOSS,REORDER"))?;
    let [latency, jitter, loss_pct, reorder_pct] = parts[..] else {
        return Err(format!("--net-fault {text}: want four numbers"));
    };
    if latency + jitter > 30 || loss_pct > 50 || reorder_pct > 50 {
        return Err(format!(
            "--net-fault {text}: at most 30 ticks of delay and 50 percent"
        ));
    }
    Ok(LinkProfile {
        latency,
        jitter,
        loss_pct,
        reorder_pct,
    })
}

/// Split the network flags off `raw`: `None` when neither `--host` nor
/// `--join` is given (and then no other network flag may be). Refuses a
/// missing value, both roles at once, and host-only flags on a guest.
pub(super) fn split_net_args(raw: &[String]) -> Result<(Option<NetArgs>, Vec<String>), String> {
    let mut rest = Vec::with_capacity(raw.len());
    let (mut host, mut join, mut bind, mut relay, mut fault, mut headless) =
        (false, None, None, None, None, false);
    let mut words = raw.iter();
    let value = |words: &mut std::slice::Iter<'_, String>, flag: &str| {
        words
            .next()
            .filter(|word| !word.starts_with("--"))
            .cloned()
            .ok_or_else(|| format!("{flag} needs a value"))
    };
    while let Some(word) = words.next() {
        match word.as_str() {
            "--host" => host = true,
            "--headless" => headless = true,
            "--join" => {
                let target = value(&mut words, "--join ADDR CODE")?;
                let code = value(&mut words, "--join ADDR CODE")?;
                join = Some((resolve("--join", &target)?, code));
            }
            "--bind" => bind = Some(resolve("--bind", &value(&mut words, "--bind")?)?),
            "--relay" => relay = Some(resolve("--relay", &value(&mut words, "--relay")?)?),
            "--net-fault" => fault = Some(parse_fault(&value(&mut words, "--net-fault")?)?),
            _ => rest.push(word.clone()),
        }
    }
    let role = match (host, join) {
        (true, Some(_)) => return Err("--host and --join exclude each other".to_string()),
        (true, None) => NetRole::Host,
        (false, Some((target, code))) => NetRole::Join { target, code },
        (false, None) => {
            if bind.is_some() || relay.is_some() || fault.is_some() || headless {
                return Err(
                    "--bind, --relay, --net-fault and --headless need --host or --join".to_string(),
                );
            }
            return Ok((None, rest));
        }
    };
    if role != NetRole::Host && (bind.is_some() || relay.is_some()) {
        return Err(
            "--bind and --relay are host flags; a guest's code names the relay".to_string(),
        );
    }
    let net = NetArgs {
        role,
        bind,
        relay,
        fault,
        headless,
    };
    Ok((Some(net), rest))
}

impl NetArgs {
    /// The match to build: the host's own flags, or what the guest's code
    /// names (its config and its spear; the guest's own `--spear` is moot).
    pub(super) fn session(
        &self,
        config: GameConfig,
        spear: bool,
    ) -> Result<(GameConfig, bool), String> {
        let NetRole::Join { code, .. } = &self.role else {
            return Ok((config, spear));
        };
        let invite = Invite::decode(code).map_err(|err| format!("--join code: {err}"))?;
        match invite.rules {
            NetRules::V1 => {
                Err("--join code: a v1 proof code, not an arena window code".to_string())
            }
            rules => Ok((invite.config, rules == NetRules::DuelSpear)),
        }
    }

    /// Open the seat for `config` (with the spear when `spear`), fault link set.
    fn open(&self, config: GameConfig, spear: bool) -> Result<NetSeat, GameError> {
        let mut seat = match &self.role {
            NetRole::Host => {
                let unspecified = SocketAddr::from(([0, 0, 0, 0], DEFAULT_PORT));
                let bind = self.bind.unwrap_or(unspecified);
                NetSeat::host(config, NetRules::duel(spear), bind, self.relay)?
            }
            NetRole::Join { target, code } => NetSeat::join(*target, code)?,
        };
        if let Some(profile) = self.fault {
            let seed = if seat.is_host() {
                FAULT_SEED_HOST
            } else {
                FAULT_SEED_GUEST
            };
            seat.set_fault(profile, seed);
        }
        Ok(seat)
    }

    /// Say how to reach this seat: the host prints the code and the exact
    /// command a guest types (LAN, same PC, or through the relay).
    fn announce(&self, seat: &NetSeat) {
        let invite = seat.invite();
        let local = seat.local_addr().ok();
        let rules = invite.rules.label();
        let (ticks, fighters) = (invite.config.ticks, invite.config.fighters);
        let code = invite.code();
        match (&self.role, self.relay) {
            (NetRole::Join { target, .. }, _) => {
                say!(
                    "arena net: joining {target} as slot 1 ({rules}, {ticks} ticks, {fighters} fighters)"
                );
            }
            (NetRole::Host, Some(relay)) => {
                say!(
                    "arena net: hosting slot 0 through the relay {relay} ({rules}, {ticks} ticks, {fighters} fighters)"
                );
                say!("arena net: invite code {code}");
                say!("arena net: the guest types: arena.exe --scene arena --join {relay} {code}");
            }
            (NetRole::Host, None) => {
                let port = local.map_or(DEFAULT_PORT, |addr| addr.port());
                let at = local.map_or_else(|| "?".to_string(), |addr| addr.to_string());
                say!(
                    "arena net: hosting slot 0 on {at} ({rules}, {ticks} ticks, {fighters} fighters), waiting for a guest"
                );
                say!("arena net: invite code {code}");
                let loopback = local.is_some_and(|addr| addr.ip().is_loopback());
                if let Some(ip) = lan_ip().filter(|_| !loopback) {
                    say!(
                        "arena net: a guest on the LAN types: arena.exe --scene arena --join {ip}:{port} {code}"
                    );
                }
                say!(
                    "arena net: a guest on this PC types: arena.exe --scene arena --join 127.0.0.1:{port} {code}"
                );
            }
        }
    }
}

/// Say one seat note as an `arena net:` line (refusals and desyncs on stderr).
fn say_note(note: &SeatNote, host: bool) {
    let peer = if host { "guest" } else { "host" };
    match note {
        SeatNote::Paired { guest_code } => {
            say!("arena net: a guest joined ({guest_code}), the match starts");
        }
        SeatNote::Refused { guest_code } => {
            tell!("arena net: REFUSED a guest whose code names another match: {guest_code}");
        }
        SeatNote::Welcomed { rtt_ms, lead } => {
            say!("arena net: joined, round trip {rtt_ms} ms, inputs sent {lead} ticks ahead");
        }
        SeatNote::RelayRegistered => say!("arena net: the relay registered this seat"),
        SeatNote::PeerGone if host => {
            tell!("arena net: the guest is silent; AI plays its slot until it is back");
        }
        SeatNote::PeerGone => tell!("arena net: the host is silent; waiting"),
        SeatNote::PeerBack => say!("arena net: the {peer} is back"),
        SeatNote::Desync(report) => tell!(
            "arena net: DESYNC at tick {} (this view {:016x}, host {:016x})",
            report.first_tick,
            report.local_hash,
            report.remote_hash
        ),
    }
}

/// A seat failure for the window: a guest that never heard its host gets the
/// usual reasons in the line.
fn seat_error(err: GameError, connected: bool) -> WinError {
    match err {
        GameError::Io(std::io::ErrorKind::TimedOut) if !connected => WinError::BadArgs(
            "no answer from the host: is it running, is the address right, does its firewall let UDP in?"
                .to_string(),
        ),
        other => WinError::Game(other),
    }
}

/// Milliseconds since `clock`, saturating.
fn ms_since(clock: Instant) -> u64 {
    u64::try_from(clock.elapsed().as_millis()).unwrap_or(u64::MAX)
}

/// `--headless`: play the scripted session on a seat with no window, print
/// the receipt and the `arena sim:` line the window prints, exit 0 when it
/// ended clean, 1 on a desync or a failure.
pub(super) fn run_headless(config: GameConfig, spear: bool, net: &NetArgs) -> i32 {
    let mut seat = match net.open(config, spear) {
        Ok(seat) => seat,
        Err(err) => {
            tell!("arena net: {err}");
            return 2;
        }
    };
    net.announce(&seat);
    let host = seat.is_host();
    let deadline = u64::from(config.ticks)
        .saturating_mul(17)
        .saturating_add(HEADLESS_SLACK_MS);
    let played = play_headless(&mut seat, deadline, |note| say_note(note, host));
    say!("arena net: receipt {}", seat.receipt_line());
    if let Err(err) = played {
        tell!("arena net: {}", seat_error(err, seat.connected()));
        return 1;
    }
    if let Some(point) = seat.game().app().checkpoints().last() {
        say!(
            "arena sim: ticks={} hash={:016x}",
            seat.cursor(),
            point.hash
        );
    }
    i32::from(seat.game().last_report().is_some())
}

/// Open the seat inside the window: the seat's view replaces the window's
/// session (the window then draws what the seat ticks), the bout starts at
/// once (no title screen for a network match).
pub(super) fn attach(app: &mut ArenaApp, net: &NetArgs) -> Result<(), WinError> {
    let mut seat = net
        .open(app.config, app.spear.on)
        .map_err(|err| seat_error(err, false))?;
    net.announce(&seat);
    seat.swap_view(&mut app.app);
    app.flow = Flow::Match;
    app.last = Instant::now();
    app.net = Some(NetPlay {
        seat,
        script: app.args.exit,
        clock: Instant::now(),
        reported: false,
        fatal: None,
    });
    Ok(())
}

impl ArenaApp {
    /// The local slot's input: the script on an automated run, else the
    /// keyboard and the pad exactly like single play.
    fn net_input(&mut self, tick: u32, human: FighterId) -> FighterInput {
        if self.net.as_ref().is_some_and(|play| play.script) {
            script_input(tick, human.0)
        } else {
            self.human_input(tick, human)
        }
    }

    /// Frame start of a network seat: pump it and say its notes; a guest in
    /// a running bout sends the inputs it owes. Returns the fixed steps the
    /// tick loop may run (a guest: authority ticks to replay). Without a seat
    /// the steps pass through.
    pub(super) fn net_begin(&mut self, steps: u32, human: FighterId) -> Result<u32, WinError> {
        let Some(play) = self.net.as_mut() else {
            return Ok(steps);
        };
        let now_ms = ms_since(play.clock);
        let host = play.seat.is_host();
        let notes = play
            .seat
            .pump(now_ms)
            .map_err(|err| seat_error(err, play.seat.connected()))?;
        for note in &notes {
            say_note(note, host);
        }
        if host {
            return Ok(steps);
        }
        if !self.flow.ticks_run() {
            return Ok(0);
        }
        let owed = play.seat.sends_owed();
        for _ in 0..owed {
            let Some(tick) = self.net.as_ref().map(|play| play.seat.next_send_tick()) else {
                break;
            };
            let input = self.net_input(tick, human);
            if let Some(play) = self.net.as_mut() {
                play.seat.send(input)?;
            }
        }
        Ok(self
            .net
            .as_ref()
            .map_or(0, |play| play.seat.views_owed(steps)))
    }

    /// One network tick into the window's view: the host authors on the
    /// local input once the guest's is in, the guest replays authority.
    /// `None` when the seat cannot run a tick yet (the loop stops for this
    /// frame). Returns the input the crash ring records: the guest's own
    /// inputs live on the host, so its ring holds the tick with an idle input.
    pub(super) fn net_tick(
        &mut self,
        tick: u32,
        human: FighterId,
    ) -> Result<Option<FighterInput>, WinError> {
        let Some(play) = self.net.as_mut() else {
            return Ok(None);
        };
        if play.seat.is_host() {
            if !play.seat.can_author() {
                if play.seat.connected() {
                    play.seat.note_stall();
                }
                return Ok(None);
            }
            let input = self.net_input(tick, human);
            self.check_forced_tick(tick);
            let Some(play) = self.net.as_mut() else {
                return Ok(None);
            };
            play.seat.swap_view(&mut self.app);
            let authored = play.seat.author(input);
            play.seat.swap_view(&mut self.app);
            self.tick_scratch = authored?;
            return Ok(Some(input));
        }
        play.seat.swap_view(&mut self.app);
        let replayed = play.seat.replay();
        play.seat.swap_view(&mut self.app);
        let Some(outcome) = replayed? else {
            return Ok(None);
        };
        self.tick_scratch = outcome;
        Ok(Some(FighterInput {
            tick,
            fighter: human,
            mx: 0,
            mz: 0,
            facing: 0,
            buttons: engine_input::Buttons::NONE,
        }))
    }

    /// Frame end of a network seat: send what the frame queued; once the
    /// seat finished, print its receipt line one time.
    pub(super) fn net_end(&mut self) -> Result<(), WinError> {
        let Some(play) = self.net.as_mut() else {
            return Ok(());
        };
        play.seat.flush()?;
        if !play.reported && play.seat.finished(ms_since(play.clock)) {
            play.reported = true;
            say!("arena net: receipt {}", play.seat.receipt_line());
        }
        Ok(())
    }

    /// `--exit` with a seat: leave once the seat finished (the host also
    /// waits for the guest to hold every tick). `None` without a seat.
    pub(super) fn net_finished(&self) -> Option<bool> {
        self.net
            .as_ref()
            .map(|play| play.reported && play.seat.finished(ms_since(play.clock)))
    }

    /// Keep a seat failure (with its receipt line) for the window to end on.
    pub(super) fn net_fail(&mut self, err: WinError) {
        if let Some(play) = self.net.as_mut() {
            say!("arena net: receipt {}", play.seat.receipt_line());
            play.fatal = Some(err);
        }
    }

    /// The kept seat failure, taken once.
    pub(super) fn net_fatal(&mut self) -> Option<WinError> {
        self.net.as_mut().and_then(|play| play.fatal.take())
    }

    /// Rematch over the network needs both sides: refuse it with a line.
    pub(super) fn net_refuses_rematch(&self) -> bool {
        if self.net.is_some() {
            say!("arena net: a rematch needs a new session: restart both sides");
        }
        self.net.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(args: &[&str]) -> Vec<String> {
        args.iter().map(|arg| (*arg).to_string()).collect()
    }

    #[test]
    fn net_flags_split_off_and_refuse_bad_mixes() {
        let raw = strings(&[
            "arena",
            "--scene",
            "arena",
            "--host",
            "--bind",
            "127.0.0.1:0",
            "--exit",
        ]);
        let (net, rest) = split_net_args(&raw).expect("host flags parse");
        let net = net.expect("a host");
        assert_eq!(net.role, NetRole::Host);
        assert_eq!(net.bind, Some(SocketAddr::from(([127, 0, 0, 1], 0))));
        assert_eq!(rest, strings(&["arena", "--scene", "arena", "--exit"]));
        let raw = strings(&[
            "arena",
            "--join",
            "127.0.0.1:7043",
            "CODE",
            "--net-fault",
            "2,1,5,5",
        ]);
        let (net, rest) = split_net_args(&raw).expect("join flags parse");
        let net = net.expect("a guest");
        assert!(matches!(net.role, NetRole::Join { ref code, .. } if code == "CODE"));
        assert_eq!(net.fault.map(|link| link.loss_pct), Some(5));
        assert_eq!(rest, strings(&["arena"]));
        let (none, same) = split_net_args(&strings(&["arena", "--exit"])).expect("no net flags");
        assert!(none.is_none() && same == strings(&["arena", "--exit"]));
        for bad in [
            &["arena", "--host", "--join", "127.0.0.1:1", "C"][..],
            &["arena", "--join", "127.0.0.1:1"],
            &[
                "arena",
                "--join",
                "127.0.0.1:1",
                "C",
                "--relay",
                "127.0.0.1:2",
            ],
            &["arena", "--headless"],
            &["arena", "--host", "--net-fault", "1,2,3"],
            &["arena", "--host", "--net-fault", "1,2,90,0"],
        ] {
            assert!(split_net_args(&strings(bad)).is_err(), "accepted {bad:?}");
        }
    }

    #[test]
    fn a_guest_builds_the_match_its_code_names() {
        let config = GameConfig {
            seed: 9,
            ticks: 300,
            fighters: 3,
            half_extent_m: engine_game::windowed::ARENA_HALF_EXTENT_M,
        };
        let code = Invite {
            config,
            rules: NetRules::DuelSpear,
            relay_host: None,
        }
        .code();
        let raw = strings(&["arena", "--join", "127.0.0.1:7043", &code]);
        let (net, _) = split_net_args(&raw).expect("parses");
        let net = net.expect("a guest");
        let mine = GameConfig { seed: 1, ..config };
        assert_eq!(net.session(mine, false), Ok((config, true)));
        let host = split_net_args(&strings(&["arena", "--host"]))
            .expect("parses")
            .0
            .expect("host");
        assert_eq!(host.session(mine, false), Ok((mine, false)));
    }
}
