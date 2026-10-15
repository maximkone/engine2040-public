//! net-bout: runs the played proofs and posts their evidence.
//!
//! One process, every seat:
//! - `net-bout mp6 [--out DIR] [--seed N] [--ticks N]` plays the two-machine
//!   bout (2 humans + 6 AI, scripted drop and rejoin) over the loopback fault
//!   injector on a LAN and a WAN profile, each twice, then the 16-bot x600
//!   soak and the WAN fault matrix twice, and writes the receipts
//!   `tools/mp6-check.mjs` reads.
//! - `net-bout on30 [--out DIR] [--seed N]` (built with `--features quic`)
//!   plays the online bout through the relay on QUIC.
//!
//! Two machines, one command each (the bout is split: machine A plays the
//! host and seat 0, machine B seat 1, in lockstep over real UDP; the fault
//! profiles run in step time, so each bout must equal the one-process bout):
//! - A: `net-bout mp6-serve [--bind ADDR] [--out DIR]` (default bind
//!   `0.0.0.0:7040`, out `target/mp6-lan`), B: `net-bout mp6-join --server
//!   A_ADDR:7040`. A writes the same six receipts as `mp6` plus
//!   `real-net.json` (probe round trips, resends, wall time per bout).
//! - Relay host: `net-relay`. A: `net-bout on30-serve --relay R:7041`
//!   prints an invite code; B: `net-bout on30-join --relay R:7041 --code
//!   CODE`. Both punch; the bouts ride the relay, then the direct road when
//!   the punch won. A writes the online receipts to `target/on30-nat`.
//!
//! Exit code 0 only when every bout converged and every run repeated its hash.

use std::net::{SocketAddr, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use engine_net::courier::{Courier, ProbeStats, RelayLeg, Road, RunNet};
use engine_net::link::LinkProfile;
use engine_net::played::{run_loopback, BoutConfig, BoutReport, BOUT_TICKS};
use engine_net::receipt::{
    bout_json, checkpoints_json, distributions_json, drop_json, lines, matrix_json, soak_json, Obj,
};
use engine_net::relay::{InviteCode, RelayConfig, PUNCH_TIMEOUT_TICKS};
use engine_net::scale::{run_soak, SOAK_TICKS};
use engine_net::split::{host_session, invite_ids, join_session, rendezvous, Rendezvous, RunSpec};
use engine_net::tick_budget::SIEGE_TICKS;
use engine_net::wan_soak::run_matrix;

/// Soak seed of the 16-bot x600 proof.
const SOAK_SEED: u64 = 2040;
/// Fault-matrix seed (the canonical WAN soak seed).
const MATRIX_SEED: u64 = 7;
/// Default UDP address machine A listens on for a LAN bout.
const SERVE_BIND: &str = "0.0.0.0:7040";
/// Default local address of every other socket: any interface, any port.
const ANY_BIND: &str = "0.0.0.0:0";

/// Parsed command line.
struct Args {
    mode: String,
    out: Option<PathBuf>,
    seed: u64,
    ticks: u32,
    bind: Option<String>,
    server: Option<String>,
    relay: Option<String>,
    code: Option<String>,
    token: u64,
}

const MODES: &str = "mp6, mp6-serve, mp6-join, on30, on30-serve or on30-join";

fn parse(args: &[String]) -> Result<Args, String> {
    let mut parsed = Args {
        mode: args
            .first()
            .cloned()
            .ok_or_else(|| format!("missing mode: {MODES}"))?,
        out: None,
        seed: 2040,
        ticks: BOUT_TICKS,
        bind: None,
        server: None,
        relay: None,
        code: None,
        token: RelayConfig::default().token,
    };
    let mut rest = args[1..].iter();
    while let Some(flag) = rest.next() {
        let value = rest.next().ok_or_else(|| format!("{flag} needs a value"))?;
        match flag.as_str() {
            "--out" => parsed.out = Some(PathBuf::from(value)),
            "--seed" => parsed.seed = value.parse().map_err(|_| "bad --seed")?,
            "--ticks" => parsed.ticks = value.parse().map_err(|_| "bad --ticks")?,
            "--bind" => parsed.bind = Some(value.clone()),
            "--server" => parsed.server = Some(value.clone()),
            "--relay" => parsed.relay = Some(value.clone()),
            "--code" => parsed.code = Some(value.to_uppercase()),
            "--token" => parsed.token = parse_token(value)?,
            other => return Err(format!("unknown flag {other}")),
        }
    }
    Ok(parsed)
}

fn parse_token(text: &str) -> Result<u64, String> {
    let parsed = match text.strip_prefix("0x") {
        Some(hex) => u64::from_str_radix(&hex.replace('_', ""), 16),
        None => text.parse(),
    };
    parsed.map_err(|_| format!("bad --token {text}"))
}

/// First address `text` (host:port, names allowed) resolves to.
fn resolve(text: &str) -> Result<SocketAddr, String> {
    text.to_socket_addrs()
        .map_err(|err| format!("cannot resolve {text}: {err}"))?
        .next()
        .ok_or_else(|| format!("{text} resolves to nothing"))
}

fn write(dir: &Path, name: &str, body: &str) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|err| format!("create {}: {err}", dir.display()))?;
    let path = dir.join(name);
    std::fs::write(&path, body).map_err(|err| format!("write {}: {err}", path.display()))?;
    println!("wrote {}", path.display());
    Ok(())
}

/// The canonical bout for `profile`, shortened to `ticks` when asked.
fn bout_config(seed: u64, ticks: u32, profile: LinkProfile) -> BoutConfig {
    let mut cfg = BoutConfig::for_profile(seed, profile);
    if ticks != BOUT_TICKS {
        cfg.ticks = ticks;
        cfg.drop.at = ticks / 10;
        cfg.drop.back = ticks * 3 / 10;
    }
    cfg
}

/// One profile played twice: both reports, in run order.
fn play_twice(
    seed: u64,
    ticks: u32,
    profile: LinkProfile,
) -> Result<(BoutReport, BoutReport), String> {
    let cfg = bout_config(seed, ticks, profile);
    let first = run_loopback(&cfg, profile, profile).map_err(|err| err.to_string())?;
    let second = run_loopback(&cfg, profile, profile).map_err(|err| err.to_string())?;
    Ok((first, second))
}

/// Wrap two labelled receipts as one JSON object.
fn pair(a: (&str, String), b: (&str, String)) -> String {
    format!(
        "{{\n  \"{}\": {},\n  \"{}\": {}\n}}\n",
        a.0,
        a.1.trim(),
        b.0,
        b.1.trim()
    )
}

/// Write the six receipts `tools/mp6-check.mjs` reads, after running the
/// soak and the fault matrix twice here. True when everything repeated and
/// both bouts converged.
fn post_mp6(
    dir: &Path,
    lan: (&BoutReport, &BoutReport),
    wan: (&BoutReport, &BoutReport),
) -> Result<bool, String> {
    let soak = run_soak(SOAK_SEED, 16, SOAK_TICKS).map_err(|err| err.to_string())?;
    let soak_twin = run_soak(SOAK_SEED, 16, SOAK_TICKS).map_err(|err| err.to_string())?;
    println!("{soak}");
    let matrix = run_matrix(MATRIX_SEED, SIEGE_TICKS).map_err(|err| err.to_string())?;
    let matrix_twin = run_matrix(MATRIX_SEED, SIEGE_TICKS).map_err(|err| err.to_string())?;
    let summary = pair(
        ("lan", bout_json("lan", lan.0, Some(lan.1))),
        ("wan", bout_json("wan", wan.0, Some(wan.1))),
    );
    write(dir, "played-bout.json", &summary)?;
    let checkpoints = pair(
        ("lan", checkpoints_json("lan", lan.0)),
        ("wan", checkpoints_json("wan", wan.0)),
    );
    write(dir, "checkpoints.json", &checkpoints)?;
    let drops = pair(
        ("lan", drop_json("lan", lan.0)),
        ("wan", drop_json("wan", wan.0)),
    );
    write(dir, "drop-rejoin.json", &drops)?;
    write(
        dir,
        "distributions.json",
        &distributions_json(&[("lan", lan.0), ("wan", wan.0)]),
    )?;
    write(dir, "soak-hash.json", &soak_json(&soak, &soak_twin))?;
    write(
        dir,
        "fault-matrix.json",
        &matrix_json(&matrix, &matrix_twin),
    )?;
    let repeats = lan.0.digest() == lan.1.digest()
        && wan.0.digest() == wan.1.digest()
        && soak == soak_twin
        && matrix
            .iter()
            .zip(&matrix_twin)
            .all(|(a, b)| a.digest_key() == b.digest_key());
    Ok(lan.0.converged() && wan.0.converged() && repeats)
}

fn mp6(args: &Args) -> Result<bool, String> {
    let dir = args
        .out
        .clone()
        .unwrap_or_else(|| PathBuf::from("target/mp6"));
    let (lan, lan_twin) = play_twice(args.seed, args.ticks, LinkProfile::LAN)?;
    let (wan, wan_twin) = play_twice(args.seed, args.ticks, LinkProfile::WAN)?;
    println!("LAN {lan}");
    println!("WAN {wan}");
    let ok = post_mp6(&dir, (&lan, &lan_twin), (&wan, &wan_twin))?;
    println!("MP6 {}", if ok { "PASS" } else { "FAIL" });
    Ok(ok)
}

#[cfg(feature = "quic")]
fn on30(args: &Args) -> Result<bool, String> {
    let dir = args
        .out
        .clone()
        .unwrap_or_else(|| PathBuf::from("target/on30"));
    let receipts = engine_net::online::on30_receipts(args.seed).map_err(|err| err.to_string())?;
    for line in &receipts.lines {
        println!("{line}");
    }
    for (name, body) in &receipts.files {
        write(&dir, name, body)?;
    }
    println!("ON30 {}", if receipts.passed { "PASS" } else { "FAIL" });
    Ok(receipts.passed)
}

#[cfg(not(feature = "quic"))]
fn on30(_: &Args) -> Result<bool, String> {
    Err(String::from(
        "on30 needs the QUIC transport: rebuild with --features quic",
    ))
}

/// One split bout with its in-process twin: the label, the report, the
/// one-process report of the same config, and its wall cost.
struct Played<'a> {
    label: &'static str,
    report: &'a BoutReport,
    twin: BoutReport,
    net: &'a RunNet,
}

impl Played<'_> {
    fn matches(&self) -> bool {
        *self.report == self.twin
    }
}

/// Pair every split report with its one-process twin.
fn twins<'a>(
    labels: &[&'static str],
    runs: &[RunSpec],
    reports: &'a [BoutReport],
    nets: &'a [RunNet],
) -> Result<Vec<Played<'a>>, String> {
    let mut out = Vec::new();
    for (((label, spec), report), net) in labels.iter().zip(runs).zip(reports).zip(nets) {
        let twin = run_loopback(&spec.cfg, spec.up, spec.down).map_err(|err| err.to_string())?;
        out.push(Played {
            label,
            report,
            twin,
            net,
        });
    }
    Ok(out)
}

fn probe_json(probe: &ProbeStats) -> String {
    let mut o = Obj::new();
    o.text("road", probe.road.label())
        .num("sent", probe.sent)
        .num("echoed", probe.rtt_us.len())
        .num("loss_bp", probe.loss_bp())
        .num("rtt_p50_us", probe.rtt_pct(50))
        .num("rtt_p95_us", probe.rtt_pct(95))
        .num("rtt_p99_us", probe.rtt_pct(99))
        .num("rtt_max_us", probe.rtt_pct(100));
    o.inline()
}

/// The real network under a split session: probe round trips per road,
/// socket counters, and per bout the wall cost and the loopback twin check.
fn real_net_json(courier: &Courier, played: &[Played<'_>]) -> String {
    let stats = courier.stats();
    let bouts: Vec<String> = played
        .iter()
        .map(|run| {
            let mut o = Obj::new();
            o.text("profile", run.label)
                .text("road", run.net.road.label())
                .num("wall_ms", run.net.wall_us / 1000)
                .num("exchanges", run.net.waits_us.len())
                .num("exchange_wait_p50_us", run.net.wait_pct(50))
                .num("exchange_wait_p99_us", run.net.wait_pct(99))
                .num("exchange_wait_max_us", run.net.wait_pct(100))
                .num("pieces_resent", run.net.pieces_resent)
                .hex("digest", run.report.digest())
                .hex("loopback_digest", run.twin.digest())
                .flag("matches_loopback", run.matches());
            o.inline()
        })
        .collect();
    let probes: Vec<String> = courier.probes().iter().map(probe_json).collect();
    let mut o = Obj::new();
    o.text(
        "machine_b",
        &courier
            .peer()
            .map_or_else(|| String::from("relay"), |addr| addr.to_string()),
    )
    .raw("road_probes", lines(probes))
    .num("datagrams_sent", stats.datagrams_sent)
    .num("datagrams_recv", stats.datagrams_recv)
    .num("bytes_sent", stats.bytes_sent)
    .num("bytes_recv", stats.bytes_recv)
    .num("pieces_resent", stats.pieces_resent)
    .num("exchanges", stats.exchanges)
    .raw("bouts", lines(bouts))
    .flag("all_match_loopback", played.iter().all(Played::matches));
    o.finish()
}

fn mp6_serve(args: &Args) -> Result<bool, String> {
    let dir = args
        .out
        .clone()
        .unwrap_or_else(|| PathBuf::from("target/mp6-lan"));
    let bind = resolve(args.bind.as_deref().unwrap_or(SERVE_BIND))?;
    let mut courier = Courier::listen(bind).map_err(|err| err.to_string())?;
    let port = courier.local_addr().map_err(|err| err.to_string())?.port();
    println!("mp6-serve: listening on {bind}; on machine B run:");
    println!("  net_bout mp6-join --server <this machine's LAN address>:{port}");
    let spec = |profile| RunSpec {
        cfg: bout_config(args.seed, args.ticks, profile),
        up: profile,
        down: profile,
        road: Road::Direct,
    };
    let runs = [
        spec(LinkProfile::LAN),
        spec(LinkProfile::LAN),
        spec(LinkProfile::WAN),
        spec(LinkProfile::WAN),
    ];
    let session = host_session(&mut courier, &runs);
    if let Some(err) = session.failed {
        return Err(format!(
            "{err} after {} of {} bouts",
            session.reports.len(),
            runs.len()
        ));
    }
    println!("machine B: {:?}", courier.peer());
    let labels = ["lan", "lan", "wan", "wan"];
    let played = twins(&labels, &runs, &session.reports, &session.nets)?;
    let [lan, lan_twin, wan, wan_twin] = &*session.reports else {
        return Err(String::from("the session lost a bout"));
    };
    println!("LAN {lan}");
    println!("WAN {wan}");
    let posted = post_mp6(&dir, (lan, lan_twin), (wan, wan_twin))?;
    write(&dir, "real-net.json", &real_net_json(&courier, &played))?;
    let twinned = played.iter().all(Played::matches);
    println!("two machines match the one-process bouts: {twinned}");
    let ok = posted && twinned;
    println!("MP6 LAN {}", if ok { "PASS" } else { "FAIL" });
    Ok(ok)
}

/// Print machine B's view of a finished session; true when every bout
/// machine A judged converged.
fn report_join(tag: &str, session: &engine_net::split::JoinSession) -> bool {
    for (index, (verdict, net)) in session.verdicts.iter().zip(&session.nets).enumerate() {
        println!(
            "bout {index}: road={} digest={:016x} converged={} wall={}ms resent={}",
            net.road.label(),
            verdict.digest,
            verdict.converged,
            net.wall_us / 1000,
            net.pieces_resent
        );
    }
    let ok = !session.verdicts.is_empty() && session.verdicts.iter().all(|v| v.converged);
    println!("{tag} {}", if ok { "PASS" } else { "FAIL" });
    ok
}

fn mp6_join(args: &Args) -> Result<bool, String> {
    let server = resolve(
        args.server
            .as_deref()
            .ok_or("mp6-join needs --server HOST:PORT (machine A)")?,
    )?;
    let bind = resolve(args.bind.as_deref().unwrap_or(ANY_BIND))?;
    let mut courier = Courier::connect(bind, server).map_err(|err| err.to_string())?;
    println!("mp6-join: playing seat 1 against {server}");
    let session = join_session(&mut courier).map_err(|err| err.to_string())?;
    Ok(report_join("MP6 LAN JOIN", &session))
}

/// Open the relay leg for one side of an invite and meet the other machine.
fn meet(args: &Args, code: &InviteCode, host: bool) -> Result<(Courier, Rendezvous), String> {
    let relay = resolve(
        args.relay
            .as_deref()
            .ok_or("needs --relay HOST:PORT (the net_relay host)")?,
    )?;
    let bind = resolve(args.bind.as_deref().unwrap_or(ANY_BIND))?;
    let (host_id, join_id) = invite_ids(code);
    let (me, peer) = if host {
        (host_id, join_id)
    } else {
        (join_id, host_id)
    };
    let leg = RelayLeg {
        addr: relay,
        token: args.token,
        me,
        peer,
    };
    let mut courier = Courier::via_relay(bind, leg).map_err(|err| err.to_string())?;
    let met = rendezvous(&mut courier).map_err(|err| err.to_string())?;
    println!(
        "public address {:?}; punch {} after {} ticks (bound {PUNCH_TIMEOUT_TICKS}); road {}",
        met.srflx,
        if met.punch.confirmed {
            "won"
        } else {
            "fell back"
        },
        met.decided_after_ticks(),
        met.road.label()
    );
    Ok((courier, met))
}

fn punch_json(met: &Rendezvous) -> String {
    let candidates = |list: &[engine_net::relay::Candidate]| {
        let items: Vec<String> = list
            .iter()
            .map(|c| format!("\"{:?} {}\"", c.kind, c.addr))
            .collect();
        format!("[{}]", items.join(", "))
    };
    let side = |punch: &engine_net::courier::PunchReport| {
        let mut o = Obj::new();
        o.flag("confirmed", punch.confirmed)
            .num("decided_after_ticks", punch.decided_after_ticks)
            .num("wall_ms", punch.wall_us / 1000)
            .num("probes_sent", punch.probes_sent)
            .num("punches_heard", punch.punches_heard)
            .text(
                "heard_from",
                &punch
                    .heard_from
                    .map_or_else(String::new, |addr| addr.to_string()),
            );
        o.inline()
    };
    let mut o = Obj::new();
    o.text(
        "public_address",
        &met.srflx.map_or_else(String::new, |addr| addr.to_string()),
    )
    .raw("candidates_a", candidates(&met.mine))
    .raw("candidates_b", candidates(&met.theirs))
    .raw("punch_a", side(&met.punch))
    .raw("punch_b", side(&met.their_punch))
    .num("decided_after_ticks", met.decided_after_ticks())
    .num("punch_bound_ticks", PUNCH_TIMEOUT_TICKS)
    .flag(
        "punch_inside_bound",
        met.decided_after_ticks() <= PUNCH_TIMEOUT_TICKS,
    )
    .text("road", met.road.label());
    o.finish()
}

/// The online receipt of one road: its two bouts, the twin check, the punch.
fn road_json(road: Road, met: &Rendezvous, runs: &[&Played<'_>]) -> String {
    let mut o = Obj::new();
    o.text("path", road.label());
    let [first, second] = runs else {
        o.flag("available", false)
            .text("reason", "the punch fell back to the relay");
        return o.finish();
    };
    let repeated = first.report.digest() == second.report.digest();
    let ok = first.report.converged() && repeated && first.matches() && second.matches();
    o.flag("available", true)
        .num("punch_decided_after_ticks", met.decided_after_ticks())
        .num("punch_bound_ticks", PUNCH_TIMEOUT_TICKS)
        .flag(
            "punch_inside_bound",
            met.decided_after_ticks() <= PUNCH_TIMEOUT_TICKS,
        )
        .flag("invites_accepted", true)
        .flag("hash_x2_identical", repeated)
        .flag("matches_loopback", first.matches() && second.matches())
        .flag("passed", ok)
        .raw(
            "bout",
            bout_json(road.label(), first.report, Some(second.report)),
        );
    o.finish()
}

/// The invite code machine A hands out: `--code` when given, else a fresh
/// one per process (the pid is unique on this machine).
fn invite_code(args: &Args) -> Result<InviteCode, String> {
    match &args.code {
        Some(text) => {
            InviteCode::decode_str(text).map_err(|err| format!("bad --code {text}: {err}"))
        }
        None => {
            let mut state = engine_core::mix64(u64::from(std::process::id()) ^ args.seed);
            Ok(InviteCode::mint(&mut state))
        }
    }
}

fn on30_serve(args: &Args) -> Result<bool, String> {
    let dir = args
        .out
        .clone()
        .unwrap_or_else(|| PathBuf::from("target/on30-nat"));
    let code = invite_code(args)?;
    println!(
        "on30-serve: invite code {}; on machine B run:",
        code.encode_str()
    );
    println!(
        "  net_bout on30-join --relay {} --code {}",
        args.relay.as_deref().unwrap_or("<relay>"),
        code.encode_str()
    );
    let (mut courier, met) = meet(args, &code, true)?;
    let spec = |road| RunSpec {
        cfg: bout_config(args.seed, args.ticks, LinkProfile::WAN),
        up: LinkProfile::WAN,
        down: LinkProfile::WAN,
        road,
    };
    let mut runs = vec![spec(Road::Relay), spec(Road::Relay)];
    if met.road == Road::Direct {
        runs.extend([spec(Road::Direct), spec(Road::Direct)]);
    }
    let session = host_session(&mut courier, &runs);
    if let Some(err) = session.failed {
        return Err(format!(
            "{err} after {} of {} bouts",
            session.reports.len(),
            runs.len()
        ));
    }
    let labels: Vec<&'static str> = runs.iter().map(|run| run.road.label()).collect();
    let played = twins(&labels, &runs, &session.reports, &session.nets)?;
    post_on30(&dir, &met, &courier, &played)
}

/// Write the online receipts of the finished split bouts; true when every
/// road played passed and the punch decided inside its bound.
fn post_on30(
    dir: &Path,
    met: &Rendezvous,
    courier: &Courier,
    played: &[Played<'_>],
) -> Result<bool, String> {
    let on = |road: Road| -> Vec<&Played<'_>> {
        played.iter().filter(|run| run.net.road == road).collect()
    };
    let (relayed, direct) = (on(Road::Relay), on(Road::Direct));
    for run in played {
        println!("{} {}", run.label.to_uppercase(), run.report);
    }
    let relay_bout = relayed.first().map(|run| run.report);
    let Some(relay_bout) = relay_bout else {
        return Err(String::from("no relay bout finished"));
    };
    write(
        dir,
        "online-bout.json",
        &pair(
            ("relay", road_json(Road::Relay, met, &relayed)),
            ("direct", road_json(Road::Direct, met, &direct)),
        ),
    )?;
    let mut profiles = vec![("relay", relay_bout)];
    if let Some(run) = direct.first() {
        profiles.push(("direct", run.report));
    }
    let each = |render: fn(&str, &BoutReport) -> String| {
        let rows: Vec<String> = profiles
            .iter()
            .map(|(label, report)| format!("\"{label}\": {}", render(label, report).trim()))
            .collect();
        format!("{{\n  {}\n}}\n", rows.join(",\n  "))
    };
    write(dir, "checkpoints.json", &each(checkpoints_json))?;
    write(dir, "drop-rejoin.json", &each(drop_json))?;
    write(dir, "distributions.json", &distributions_json(&profiles))?;
    write(dir, "punch.json", &punch_json(met))?;
    write(dir, "real-net.json", &real_net_json(courier, played))?;
    let road_ok = |runs: &[&Played<'_>]| {
        runs.len() == 2
            && runs
                .iter()
                .all(|run| run.report.converged() && run.matches())
            && runs[0].report.digest() == runs[1].report.digest()
    };
    let ok = road_ok(&relayed)
        && (direct.is_empty() || road_ok(&direct))
        && met.decided_after_ticks() <= PUNCH_TIMEOUT_TICKS;
    println!(
        "ON30 NAT {} (relay road {}, direct road {})",
        if ok { "PASS" } else { "FAIL" },
        if road_ok(&relayed) { "ok" } else { "failed" },
        if direct.is_empty() {
            "not punched"
        } else if road_ok(&direct) {
            "ok"
        } else {
            "failed"
        }
    );
    Ok(ok)
}

fn on30_join(args: &Args) -> Result<bool, String> {
    let text = args
        .code
        .as_deref()
        .ok_or("on30-join needs --code CODE (printed by on30-serve)")?;
    let code = InviteCode::decode_str(text).map_err(|err| format!("bad --code {text}: {err}"))?;
    let (mut courier, _) = meet(args, &code, false)?;
    let session = join_session(&mut courier).map_err(|err| err.to_string())?;
    Ok(report_join("ON30 NAT JOIN", &session))
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let outcome = parse(&args).and_then(|parsed| match parsed.mode.as_str() {
        "mp6" => mp6(&parsed),
        "mp6-serve" => mp6_serve(&parsed),
        "mp6-join" => mp6_join(&parsed),
        "on30" => on30(&parsed),
        "on30-serve" => on30_serve(&parsed),
        "on30-join" => on30_join(&parsed),
        other => Err(format!("unknown mode {other}: {MODES}")),
    });
    match outcome {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::from(1),
        Err(message) => {
            eprintln!("net-bout: {message}");
            ExitCode::from(2)
        }
    }
}
