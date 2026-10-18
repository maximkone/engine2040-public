//! The server guard and the lag window on the real played-bout path: a
//! cheating seat is refused and counted while the honest match does not
//! move, and a seat 80 ms away lands every press on the tick it pressed it.

use super::*;
use crate::guard::{Refusal, RATE_MAX_PACKETS};
use crate::lagcomp::MAX_WINDOW_TICKS;
use crate::link::FaultLink;

/// Bout steps on which seat 0 tries every cheat.
const CHEAT_STEPS: [u32; 4] = [50, 75, 200, 250];

/// Seat 0 forges one datagram of every cheat kind against the host, then a
/// flood of honest copies past the rate bound. Injected beside the wire, so
/// the honest datagrams draw the same loss and delay as a clean bout.
fn cheat(host: &mut Host, step: u32, wire: &mut LoopbackWire) {
    let open = host.server.tick_now();
    let Some(kept) = host
        .inbox
        .iter()
        .rev()
        .find(|((_, seat), _)| *seat == 0)
        .map(|(_, input)| *input)
    else {
        return;
    };
    let mine = |tick: u32| BodyInput {
        tick,
        buttons: Buttons(ATTACK),
        ..kept
    };
    let foe = BodyInput {
        fighter: BodyId(host.slots[1]),
        ..kept
    };
    let rewrite = BodyInput {
        buttons: Buttons(kept.buttons.0 ^ ATTACK),
        ..kept
    };
    // Stamped with the current step, so no forged datagram reads as lag.
    let encode = |input: BodyInput| encode_input(step.max(input.tick), &[input]).expect("encode");
    let mut impossible = encode(kept);
    if let Some(buttons) = impossible.last_mut() {
        *buttons = 0x08;
    }
    let forged = [
        vec![1, 2, 3],
        impossible,
        encode(foe),
        encode(mine(open + 100)),
        encode(mine(open - 40)),
        encode(rewrite),
    ];
    for bytes in forged {
        host.on_frame(0, Frame::Datagram(bytes), wire, step);
    }
    let copy = encode(kept);
    for _ in 0..RATE_MAX_PACKETS {
        host.on_frame(0, Frame::Datagram(copy.clone()), wire, step);
    }
}

fn cheating_bout(cfg: &BoutConfig) -> BoutReport {
    let mut wire = loopback_wire(cfg, LinkProfile::LAN, LinkProfile::LAN);
    let mut part = BoutPart::new(cfg, true, &[0, 1]).expect("bout");
    for step in 0..part.steps() {
        part.step(step, &mut wire).expect("step");
        if let Some(host) = part.host.as_mut().filter(|_| CHEAT_STEPS.contains(&step)) {
            cheat(host, step, &mut wire);
        }
        wire.tick();
    }
    part.report(&part.outcomes(), wire.report())
        .expect("report")
}

#[test]
fn played_cheating_seat_is_refused_counted_and_the_honest_match_holds_x2() {
    let cfg = BoutConfig {
        ticks: 300,
        drop: DropScript {
            seat: 1,
            at: 100,
            back: 160,
        },
        ..BoutConfig::for_profile(11, LinkProfile::LAN)
    };
    let clean = run_loopback(&cfg, LinkProfile::LAN, LinkProfile::LAN).expect("clean bout");
    assert!(clean.converged(), "{clean}");
    // Honest clients never trip a cheat refusal, on either seat.
    for receipt in &clean.admission {
        assert_eq!(receipt.cheats(), 0, "{receipt}");
        assert!(receipt.accepted > 0 && receipt.repeats > 0, "{receipt}");
    }
    let first = cheating_bout(&cfg);
    let second = cheating_bout(&cfg);
    assert_eq!(
        first.digest(),
        second.digest(),
        "same cheats, same receipts"
    );
    assert_eq!(first, second);
    // Nothing forged reached the match: the server authored the clean tape.
    assert!(first.converged(), "{first}");
    assert_eq!(first.server_tip, clean.server_tip);
    assert_eq!(first.tape_hash, clean.tape_hash);
    assert_eq!(first.checkpoints, clean.checkpoints);
    // Every cheat was refused and counted on the cheater's slot only.
    let cheater = first.admission[0];
    let tries = u32::try_from(CHEAT_STEPS.len()).expect("small");
    for kind in [
        Refusal::Malformed,
        Refusal::BadField,
        Refusal::WrongSlot,
        Refusal::Future,
        Refusal::Stale,
        Refusal::Rewrite,
    ] {
        assert_eq!(cheater.count(kind), tries, "{} in {cheater}", kind.name());
    }
    assert!(cheater.count(Refusal::Rate) >= tries, "{cheater}");
    assert_eq!(cheater.accepted, clean.admission[0].accepted, "{cheater}");
    assert_eq!(first.admission[1], clean.admission[1]);
    println!("GUARD cheater {cheater}");
    println!("GUARD honest  {}", first.admission[1]);
}

/// A wire whose seat 0 sits beside the server and whose seat 1 links both
/// ways with `far`.
fn split_wire(seed: u64, far: LinkProfile) -> LoopbackWire {
    let near = LinkProfile {
        latency: 0,
        jitter: 0,
        loss_pct: 0,
        reorder_pct: 0,
    };
    let mut wire = LoopbackWire::new(SEATS, near, near, seed);
    for (from, to, lane) in [
        (Peer::Client(1), Peer::Server, 3),
        (Peer::Server, Peer::Client(1), 4),
    ] {
        if let Some(link) = wire.link_mut(from, to) {
            *link = FaultLink::new(far, mix64(seed ^ lane));
        }
    }
    wire
}

/// A bout with a one-tick floor (no drop: the rejoin step is never reached).
fn lag_bout(far: LinkProfile) -> BoutReport {
    let cfg = BoutConfig {
        fighters: 4,
        ticks: 240,
        hold: 1,
        drop: DropScript {
            seat: 1,
            at: 1,
            back: u32::MAX,
        },
        ..BoutConfig::canonical(21, 0)
    };
    run_bout(&cfg, &mut split_wire(cfg.seed, far)).expect("bout runs")
}

#[test]
fn played_80ms_seat_lands_every_press_on_its_own_tick_x2() {
    let beside = LinkProfile {
        latency: 0,
        jitter: 0,
        loss_pct: 0,
        reorder_pct: 0,
    };
    // 2 to 3 ticks each way: an 80 ms round trip on average.
    let far = LinkProfile {
        latency: 2,
        jitter: 1,
        loss_pct: 0,
        reorder_pct: 0,
    };
    let reference = lag_bout(beside);
    let first = lag_bout(far);
    let second = lag_bout(far);
    assert_eq!(first.digest(), second.digest());
    // The far seat's tape is the tape it would have played beside the
    // server: every press landed on the tick it was made for.
    assert_eq!(first.tape, reference.tape);
    assert_eq!(first.server_tip, reference.server_tip);
    assert!(first.seats.iter().all(|seat| seat.equal), "{first}");
    // The window was sized from the measured lag (3 ticks one way) and
    // every attack press of the far seat has a fair receipt naming it.
    assert_eq!(first.hold_ticks, 4);
    assert_eq!(first.admission[1].lag_max, 3);
    assert_eq!(first.admission[1].count(Refusal::Late), 0);
    let far_slot = first.seats[1].slot;
    let presses: Vec<_> = first
        .hits
        .receipts()
        .iter()
        .filter(|hit| hit.claim.attacker == far_slot)
        .collect();
    assert!(presses.len() >= 4, "{} presses", presses.len());
    assert!(presses.iter().all(|hit| hit.accepted
        && hit.claim.lag() <= u32::from(hit.window_ticks)
        && hit.window_ticks <= 5));
    assert!(first.hits.verify_replay());
    let (fair, refused) = first.hits.counts();
    println!(
        "LAGCOMP far seat lag_max={} hold={} presses={} fair={fair} refused={refused}",
        first.admission[1].lag_max,
        first.hold_ticks,
        presses.len()
    );
}

#[test]
fn played_seat_past_the_cap_has_its_presses_refused_and_counted() {
    // 300 ms one way is past the 16-tick bound: the server will not hold a
    // tick that long, so every press arrives after its tick and is refused.
    let past = LinkProfile {
        latency: 18,
        jitter: 0,
        loss_pct: 0,
        reorder_pct: 0,
    };
    let report = lag_bout(past);
    let far = report.admission[1];
    assert_eq!(far.accepted, 0, "{far}");
    assert!(far.count(Refusal::Late) > 0 && far.cheats() == 0, "{far}");
    assert_eq!(far.window().ticks, MAX_WINDOW_TICKS);
    let far_slot = report.seats[1].slot;
    let refused: Vec<_> = report
        .hits
        .receipts()
        .iter()
        .filter(|hit| hit.claim.attacker == far_slot)
        .collect();
    assert!(!refused.is_empty());
    assert!(refused.iter().all(|hit| !hit.accepted));
    // While the seat was connected the server held ticks the full 16; the
    // stall watch then handed its slot to the AI.
    assert!(refused
        .iter()
        .any(|hit| hit.window_ticks == MAX_WINDOW_TICKS));
    assert!(report.host.stalled_at.is_some());
    assert!(report.hits.verify_replay());
}

/// Slowest one-way transit on a seat's uplink: the top bucket of the delay
/// distribution the bout posts.
fn uplink_max_delay(report: &BoutReport, seat: u8) -> u32 {
    report
        .links
        .iter()
        .find(|link| link.from == Peer::Client(seat) && link.to == Peer::Server)
        .and_then(|link| link.stats.delay.iter().rposition(|count| *count > 0))
        .and_then(|bucket| u32::try_from(bucket).ok())
        .expect("the uplink delivered")
}

#[test]
fn played_window_is_sized_by_the_posted_delay_distribution_on_lan_and_wan() {
    // The window each seat gets is its slowest measured one-way trip plus
    // one tick; on the canonical bouts that is exactly the hold the bout was
    // configured with (worst delay plus one). Pinned per profile: a change in
    // the delay distribution moves these numbers and fails here.
    for (profile, window) in [(LinkProfile::LAN, 3), (LinkProfile::WAN, 10)] {
        let cfg = BoutConfig {
            ticks: 300,
            drop: DropScript {
                seat: 1,
                at: 100,
                back: 160,
            },
            ..BoutConfig::for_profile(2040, profile)
        };
        let report = run_loopback(&cfg, profile, profile).expect("bout runs");
        assert!(report.converged(), "{report}");
        // Seat 0 never drops, so its uplink histogram holds datagrams and
        // nothing held across an outage: the measured lag is its top bucket.
        assert_eq!(report.admission[0].lag_max, uplink_max_delay(&report, 0));
        for receipt in &report.admission {
            assert!(receipt.lag_max <= profile.worst_delay(), "{receipt}");
            assert_eq!(receipt.cheats(), 0, "{receipt}");
        }
        let widest = report.admission.iter().map(|r| r.window().ticks).max();
        assert_eq!(widest, Some(window));
        assert_eq!(report.hold_ticks, cfg.hold);
        assert_eq!(cfg.hold, u32::from(window));
        assert!(report.hits.verify_replay());
        assert!(report
            .hits
            .receipts()
            .iter()
            .all(|hit| hit.window_ticks <= window.max(5)));
    }
}
