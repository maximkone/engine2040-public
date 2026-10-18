//! The tracker on the real played-bout path: seats send input ticks, the host
//! answers with state windows, the sim links lose and delay.

use crate::link::{LinkProfile, LinkStats, Peer};
use crate::played::{run_loopback, BoutConfig, BoutReport, DropScript};
use crate::receipt::distributions_json;

const TICKS: u32 = 300;

/// A link of 2 to 3 ticks one way losing `loss_pct` percent.
fn lane(loss_pct: u32) -> LinkProfile {
    LinkProfile {
        latency: 2,
        jitter: 1,
        loss_pct,
        reorder_pct: 0,
    }
}

/// A 300-tick bout; seat 1 drops at tick 100 and rejoins at 160, seat 0 is
/// never cut off.
fn bout(seed: u64, up: LinkProfile, down: LinkProfile) -> BoutReport {
    let cfg = BoutConfig {
        ticks: TICKS,
        drop: DropScript {
            seat: 1,
            at: 100,
            back: 160,
        },
        ..BoutConfig::for_profile(seed, up)
    };
    run_loopback(&cfg, up, down).expect("bout runs")
}

/// Input ticks of `seat` that the server's authoritative tape holds.
fn on_tape(report: &BoutReport, seat: usize) -> u32 {
    let slot = report.seats[seat].slot;
    let rows = report
        .tape
        .iter()
        .filter(|row| row.fighter.get() == slot)
        .count();
    u32::try_from(rows).expect("a few hundred rows")
}

fn link(report: &BoutReport, from: Peer, to: Peer) -> LinkStats {
    report
        .links
        .iter()
        .find(|link| link.from == from && link.to == to)
        .expect("the bout measures every directed link")
        .stats
}

#[test]
fn ack_bout_clean_links_deliver_every_input_of_the_steady_seat() {
    let report = bout(2040, lane(0), lane(0));
    assert!(report.converged(), "{report}");
    let steady = report.seats[0].stats.delivery;
    assert_eq!((steady.input_delivered, steady.input_lost), (TICKS, 0));
    assert_eq!(steady.input_delivered, on_tape(&report, 0));
    assert_eq!(steady.input_loss_permille, 0);
    assert_eq!(steady.state_loss_permille, 0);
    // The header a later wire change would carry: tip 299, 32 states before it.
    assert_eq!((steady.state_latest, steady.state_bits), (299, u32::MAX));

    // Seat 1 sat in a dark 60-tick outage: those inputs never reached the
    // tape, so the acks call them lost.
    let cut = report.seats[1].stats.delivery;
    assert!(cut.input_lost >= 60, "{cut:?}");
    assert!(cut.input_delivered <= on_tape(&report, 1), "{cut:?}");
    assert!(cut.input_delivered + cut.input_lost <= TICKS, "{cut:?}");
}

#[test]
fn ack_bout_lossy_uplink_matches_the_server_tape() {
    let report = bout(7, lane(40), lane(0));
    let steady = report.seats[0].stats.delivery;
    // Delivered means the server took the input: exactly the tape rows.
    assert_eq!(steady.input_delivered, on_tape(&report, 0), "{steady:?}");
    assert_eq!(steady.input_delivered + steady.input_lost, TICKS);
    assert!(
        steady.input_lost > 0,
        "a 40 percent uplink must cost inputs"
    );
    assert!(
        (1..=600).contains(&steady.input_loss_permille),
        "{steady:?}"
    );
    // Every lost tick needs its first two packets gone, and one packet is
    // the first of only one tick: lost ticks never outnumber dropped packets.
    let wire = link(&report, Peer::Client(0), Peer::Server);
    assert!(
        steady.input_lost <= wire.dropped,
        "{steady:?} wire {wire:?}"
    );
}

#[test]
fn ack_bout_lossy_downlink_still_resolves_the_inputs() {
    let report = bout(11, lane(0), lane(25));
    let steady = report.seats[0].stats.delivery;
    // Eight states cover each tick, so a quarter of them lost hides nothing.
    assert!(steady.input_delivered >= TICKS - 2, "{steady:?}");
    assert!(steady.input_lost <= 1, "{steady:?}");
    // The receiver's own estimate of the inbound states follows the link.
    let wire = link(&report, Peer::Server, Peer::Client(0));
    let real = wire.loss_bp() / 10;
    assert!(
        steady.state_loss_permille.abs_diff(real) <= 200,
        "estimate {} permille, link {real} permille",
        steady.state_loss_permille
    );
    assert!(steady.state_loss_permille > 0, "{steady:?}");
}

#[test]
fn ack_bout_delivery_is_logged_and_never_hashed() {
    let report = bout(2040, lane(5), lane(5));
    let line = report.to_string();
    assert!(line.contains(" in_loss="), "{line}");
    let json = distributions_json(&[("wan", &report)]);
    for seat in &report.seats {
        let d = seat.stats.delivery;
        for needle in [
            format!("\"input_ticks_delivered\": {}", d.input_delivered),
            format!("\"input_ticks_lost\": {}", d.input_lost),
            format!("\"input_loss_permille\": {}", d.input_loss_permille),
            format!("\"state_loss_permille\": {}", d.state_loss_permille),
            format!("\"state_ack_bits\": {}", d.state_bits),
        ] {
            assert!(json.contains(&needle), "{needle} missing from {json}");
        }
    }
    // The digest is what two runs must agree on; it ignores the counters.
    let mut moved = report.clone();
    moved.seats[0].stats.delivery.input_lost += 5;
    moved.seats[1].stats.delivery.state_bits ^= 1;
    assert_eq!(report.digest(), moved.digest());
}

#[test]
fn ack_bout_delivery_is_a_pure_function_of_the_config() {
    let first = bout(3, lane(10), lane(10));
    let again = bout(3, lane(10), lane(10));
    for (a, b) in first.seats.iter().zip(&again.seats) {
        assert_eq!(a.stats.delivery, b.stats.delivery);
    }
    assert_eq!(first, again);
}
