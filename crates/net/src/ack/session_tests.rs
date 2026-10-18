//! The tracker against the sim link and the session's input types.

use engine_core::alloc_probe::measure;
use engine_input::Buttons;

use super::input::{DeliveryStats, InputDelivery};
use super::tests::header_ack;
use super::*;
use crate::link::{FaultLink, Frame, LinkProfile};
use crate::session::{redundancy_window, BodyId, BodyInput};

const COVER: usize = 8;

fn input(tick: u32, fighter: u8) -> BodyInput {
    BodyInput {
        tick,
        fighter: BodyId::new(fighter).expect("slot"),
        mx: 0,
        mz: 0,
        facing: 0,
        buttons: Buttons::NONE,
    }
}

/// The authoritative tape: fighter 5 every tick, fighter 3 (the client under
/// test) every tick except the `missing` ones.
fn tape(ticks: u32, missing: &[u32]) -> Vec<BodyInput> {
    let mut rows = Vec::new();
    for tick in 0..ticks {
        rows.push(input(tick, 3));
        rows.push(input(tick, 5));
    }
    rows.retain(|row| row.fighter.get() != 3 || !missing.contains(&row.tick));
    rows
}

/// Feed the state datagram the server would send for checkpoint `tick`.
fn state(delivery: &mut InputDelivery, rows: &[BodyInput], tick: u32) -> Result<Arrival, AckError> {
    delivery.on_state(tick, &redundancy_window(rows, tick, COVER), COVER)
}

#[test]
fn ack_input_delivery_reads_the_authoritative_window() {
    let rows = tape(30, &[5, 6, 20]);
    let mut delivery = InputDelivery::new(BodyId::new(3).expect("slot"));
    for tick in 0..30 {
        delivery.on_input_sent(tick).expect("ticks rise");
    }
    // The state datagrams of ticks 10, 11 and 12 are lost on the way down.
    for tick in (0..30).filter(|t| !(10..=12).contains(t)) {
        assert_eq!(state(&mut delivery, &rows, tick), Ok(Arrival::Fresh));
    }
    let stats = delivery.stats();
    // Ticks 5, 6 and 20 never made the tape: lost. Ticks 10..=12 were still
    // named by the window of tick 13: delivered. Fighter 5 never counts.
    assert_eq!((stats.input_delivered, stats.input_lost), (27, 3));
    assert_eq!(stats.input_loss_permille, permille(3, 30));
    // Downlink: 27 judged sequences, 10..=12 missing.
    assert_eq!(stats.state_loss_permille, permille(3, 27));
    assert_eq!(stats.state_latest, 29);
    assert_eq!(stats.state_bits, 0x1FF8_FFFF);

    // A duplicate, an old packet and a late one resolve nothing twice.
    assert_eq!(state(&mut delivery, &rows, 29), Ok(Arrival::Duplicate));
    assert_eq!(state(&mut delivery, &rows, 11), Ok(Arrival::Fresh));
    let after = delivery.stats();
    assert_eq!(after.input_delivered, 27);
    assert_eq!(after.input_lost, 3);
    assert_eq!(after.state_loss_permille, permille(2, 27));
}

#[test]
fn ack_input_delivery_counts_a_dropped_seat_as_lost() {
    // The server stopped taking this client's input from tick 10 on.
    let rows = tape(24, &(10..24).collect::<Vec<_>>());
    let mut delivery = InputDelivery::new(BodyId::new(3).expect("slot"));
    for tick in 0..24 {
        delivery.on_input_sent(tick).expect("ticks rise");
    }
    for tick in 0..24 {
        state(&mut delivery, &rows, tick).expect("state");
    }
    let stats = delivery.stats();
    assert_eq!((stats.input_delivered, stats.input_lost), (10, 14));
    assert_eq!(stats.input_loss_permille, permille(14, 24));
}

#[test]
fn ack_input_delivery_refuses_a_checkpoint_from_the_future() {
    let rows = tape(100, &[]);
    let mut delivery = InputDelivery::new(BodyId::new(3).expect("slot"));
    for tick in 0..4 {
        delivery.on_input_sent(tick).expect("ticks rise");
    }
    assert_eq!(
        state(&mut delivery, &rows, 50),
        Err(AckError::FromTheFuture {
            head: 50,
            newest: 3
        })
    );
    let stats = delivery.stats();
    assert_eq!((stats.input_delivered, stats.input_lost), (0, 0));
    assert_eq!(
        delivery.on_input_sent(3),
        Err(AckError::NotNewer { seq: 3, newest: 3 })
    );
}

#[test]
fn ack_stats_words_round_trip() {
    let stats = DeliveryStats {
        input_delivered: 1,
        input_lost: 2,
        input_loss_permille: 3,
        state_loss_permille: 4,
        state_latest: 5,
        state_bits: 6,
    };
    assert_eq!(DeliveryStats::from_words(stats.to_words()), stats);
    assert_eq!(stats.to_words(), [1, 2, 3, 4, 5, 6]);
}

/// One tick of the closed loop: deliver what is due on the data link, answer
/// each arrival with the header on the return link, apply the acks that are
/// due, then advance both links.
fn pump(
    data: &mut FaultLink,
    back: &mut FaultLink,
    ledger: &mut SentLedger,
    recv: &mut RecvWindow,
) {
    for frame in data.due() {
        let Frame::Datagram(bytes) = frame else {
            continue;
        };
        recv.on_receive(Seq::from_le_bytes([bytes[0], bytes[1]]));
        let (latest, bits) = recv.header().expect("one arrival");
        let mut header = latest.to_le_bytes().to_vec();
        header.extend_from_slice(&bits.to_le_bytes());
        back.send_datagram(&header);
    }
    for frame in back.due() {
        let Frame::Datagram(bytes) = frame else {
            continue;
        };
        let ack = Ack::from_header(
            Seq::from_le_bytes([bytes[0], bytes[1]]),
            u32::from_le_bytes([bytes[2], bytes[3], bytes[4], bytes[5]]),
        );
        ledger.on_ack(&ack).expect("acks never name the future");
    }
    data.tick();
    back.tick();
}

#[test]
fn ack_loop_over_a_lossy_link_tracks_the_link_counters() {
    const SENT: u32 = 3_000;
    let up = LinkProfile {
        latency: 3,
        jitter: 0,
        loss_pct: 20,
        reorder_pct: 0,
    };
    let down = LinkProfile { loss_pct: 10, ..up };
    let run = || {
        let (mut data, mut back) = (FaultLink::new(up, 0xACE), FaultLink::new(down, 0xBEE));
        let (mut ledger, mut recv) = (SentLedger::new(), RecvWindow::default());
        // Start near the top of the range so the run crosses the wrap.
        for n in 0..SENT {
            let seq = 65_000u16.wrapping_add(Seq::try_from(n).expect("below 65536"));
            ledger.on_send(seq).expect("sequence rises");
            data.send_datagram(&seq.to_le_bytes());
            pump(&mut data, &mut back, &mut ledger, &mut recv);
        }
        for _ in 0..16 {
            pump(&mut data, &mut back, &mut ledger, &mut recv);
        }
        (ledger, recv, *data.stats())
    };
    let (ledger, recv, truth) = run();
    // Every sent packet is delivered, lost or still waiting: none vanish.
    let pending = u32::try_from(ledger.pending()).expect("small");
    assert_eq!(ledger.delivered() + ledger.lost() + pending, SENT);
    // Ack-derived loss equals the link's own count, which the sender cannot
    // see, up to the few packets the final acks never named.
    let seen = permille(ledger.lost(), ledger.delivered() + ledger.lost());
    let real = truth.loss_bp() / 10;
    assert!(
        seen.abs_diff(real) <= 25,
        "ack-derived {seen} permille, link {real} permille"
    );
    assert!(
        ledger.lost() <= truth.dropped,
        "no false loss without reorder"
    );
    assert!(truth.dropped - ledger.lost() <= pending);
    // The receiver's own estimate of the inbound stream sits near it too.
    let heard = recv.loss_permille();
    assert!((50..=400).contains(&heard), "receiver estimate {heard}");
    let again = run();
    assert_eq!((ledger, recv, truth), again, "same seeds, same run");
}

#[test]
fn ack_tracking_allocates_nothing() {
    let (_, heap) = measure(|| {
        let (mut ledger, mut recv) = (SentLedger::new(), RecvWindow::default());
        for seq in 0..20_000u16 {
            ledger.on_send(seq).expect("rises");
            if seq % 3 != 0 {
                recv.on_receive(seq);
                if let Some(ack) = header_ack(&recv) {
                    ledger.on_ack(&ack).expect("ack");
                }
            }
        }
        assert!(ledger.pending() <= WINDOW);
        std::hint::black_box((ledger.loss_permille(), recv.loss_permille()))
    });
    assert_eq!(heap.calls, 0, "send, receive and ack stay off the heap");
}
