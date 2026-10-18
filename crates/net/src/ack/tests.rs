//! Pure state-machine tests of the ack tracker: no link, no session.

use super::*;

/// The ack a sender reads out of the receiver's next header.
pub(super) fn header_ack(recv: &RecvWindow) -> Option<Ack> {
    recv.header()
        .map(|(latest, bits)| Ack::from_header(latest, bits))
}

/// Send `count` packets from `start`, deliver the ones `keep` accepts and
/// ack every arrival at once (a zero-latency return path).
fn closed_loop(start: Seq, count: u32, keep: impl Fn(u32) -> bool) -> (SentLedger, RecvWindow) {
    let (mut ledger, mut recv) = (SentLedger::new(), RecvWindow::default());
    for n in 0..count {
        let seq = start.wrapping_add(Seq::try_from(n).expect("count below 65536"));
        ledger.on_send(seq).expect("sequence rises");
        if keep(n) {
            assert_eq!(recv.on_receive(seq), Arrival::Fresh);
            let ack = header_ack(&recv).expect("one arrival");
            ledger.on_ack(&ack).expect("ack inside the sent range");
        }
    }
    (ledger, recv)
}

#[test]
fn ack_in_order_delivery_reads_zero_loss() {
    let (ledger, recv) = closed_loop(0, 300, |_| true);
    assert_eq!((ledger.delivered(), ledger.lost()), (300, 0));
    assert_eq!(ledger.pending(), 0);
    assert_eq!(ledger.loss_permille(), 0);
    assert_eq!(recv.loss_permille(), 0);
    assert_eq!(recv.header(), Some((299, u32::MAX)));
}

#[test]
fn ack_drop_every_fourth_packet_reads_250_permille() {
    let (ledger, recv) = closed_loop(0, 400, |n| n % 4 != 3);
    let windowed = ledger.loss_permille();
    assert!(
        (230..=270).contains(&windowed),
        "windowed loss {windowed} permille"
    );
    // The last packet (399) is dropped and no later ack names it: pending.
    assert_eq!(ledger.pending(), 1);
    let whole = permille(ledger.lost(), ledger.delivered() + ledger.lost());
    assert!((230..=270).contains(&whole), "whole-run loss {whole}");
    let seen = recv.loss_permille();
    assert!((230..=270).contains(&seen), "receiver estimate {seen}");
}

#[test]
fn ack_reordered_and_duplicated_acks_never_double_count() {
    let (mut ledger, mut recv) = (SentLedger::new(), RecvWindow::default());
    let mut acks = Vec::new();
    for seq in 0..40u16 {
        ledger.on_send(seq).expect("rises");
        if seq % 5 != 4 {
            recv.on_receive(seq);
            let ack = header_ack(&recv).expect("arrival");
            acks.push(ack);
            ledger.on_ack(&ack).expect("ack");
        }
    }
    let settled = (ledger.delivered(), ledger.lost(), ledger.pending());
    assert_eq!(
        settled.0 + settled.1 + u32::try_from(settled.2).expect("small"),
        40
    );
    // Replay every ack: newest first, each three times, then oldest first.
    let replay = acks.iter().rev().chain(acks.iter()).flat_map(|a| [a, a, a]);
    for ack in replay {
        assert_eq!(
            ledger.on_ack(ack).expect("old acks stay legal"),
            Resolved::default()
        );
    }
    assert_eq!(
        (ledger.delivered(), ledger.lost(), ledger.pending()),
        settled
    );
}

#[test]
fn ack_refuses_an_ack_from_the_future_and_changes_nothing() {
    let mut ledger = SentLedger::new();
    let early = Ack::from_header(3, 0);
    assert_eq!(ledger.on_ack(&early), Err(AckError::NothingSent));
    for seq in 10..14 {
        ledger.on_send(seq).expect("rises");
    }
    let before = ledger.clone();
    let future = Ack::from_header(14, u32::MAX);
    assert_eq!(
        ledger.on_ack(&future),
        Err(AckError::FromTheFuture {
            head: 14,
            newest: 13
        })
    );
    assert_eq!(ledger, before, "a refused ack changes nothing");
    assert_eq!(
        ledger.on_send(13),
        Err(AckError::NotNewer {
            seq: 13,
            newest: 13
        })
    );
    assert_eq!(ledger, before, "a refused send changes nothing");
}

#[test]
fn ack_sequence_wrap_at_u16_max_resolves_correctly() {
    assert!(is_newer(0, u16::MAX));
    assert!(!is_newer(u16::MAX, 0));
    assert!(is_newer(5, 65_530));
    assert!(!is_newer(7, 7));
    assert!(is_newer(0x7FFF, 0) && !is_newer(0x8000, 0));
    // 100 packets that cross 65_535 -> 0, every fifth one lost.
    let (ledger, recv) = closed_loop(65_500, 100, |n| n % 5 != 4);
    assert_eq!(recv.latest(), Some(65_500u16.wrapping_add(98)));
    assert_eq!(ledger.lost(), 19, "packets 4, 9, ..., 94 are lost");
    assert_eq!(ledger.delivered(), 80);
    assert_eq!(ledger.pending(), 1, "packet 99 is dropped and never named");
    // The newest 64 resolutions hold 12 of the 19 lost: 188 permille.
    assert_eq!(ledger.loss_permille(), permille(12, 64));
    // The donor bit order across the wrap: latest 1, 0 missing, 65_535 and
    // 65_534 present.
    let mut win = RecvWindow::default();
    for seq in [65_534u16, 65_535, 1] {
        assert_eq!(win.on_receive(seq), Arrival::Fresh);
    }
    assert_eq!(win.header(), Some((1, 0b110)));
    assert_eq!(win.on_receive(0), Arrival::Fresh, "the late one fills in");
    assert_eq!(win.header(), Some((1, 0b111)));
}

#[test]
fn ack_window_of_64_bounds_memory() {
    let mut ledger = SentLedger::new();
    for seq in 0..10_000u16 {
        ledger.on_send(seq).expect("rises");
        assert!(ledger.pending() <= WINDOW, "the ring never grows");
    }
    // Nothing was ever acked: all but the newest 64 aged out as lost.
    assert_eq!(ledger.pending(), WINDOW);
    assert_eq!(ledger.lost(), 10_000 - 64);
    assert_eq!(ledger.loss_permille(), 1_000);
    // A jump past the whole ring evicts every pending record at once.
    ledger.on_send(10_000 + 500).expect("rises");
    assert_eq!(ledger.pending(), 1);
    assert_eq!(ledger.lost(), 10_000 - 64 + 64);
    // The receiver window is a fixed few words whatever arrives, and a
    // packet far behind the window is refused, not remembered.
    let mut win = RecvWindow::default();
    for seq in (0..60_000u16).step_by(7) {
        win.on_receive(seq);
    }
    assert_eq!(win.latest(), Some(59_997));
    assert_eq!(
        win.on_receive(59_897),
        Arrival::Stale,
        "100 behind the window"
    );
}

#[test]
fn ack_same_input_twice_gives_identical_output() {
    let run = || {
        let (ledger, recv) = closed_loop(65_000, 700, |n| (n * 7 + n / 3) % 5 != 0);
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&ledger.delivered().to_le_bytes());
        bytes.extend_from_slice(&ledger.lost().to_le_bytes());
        bytes.extend_from_slice(&ledger.loss_permille().to_le_bytes());
        bytes.extend_from_slice(&recv.loss_permille().to_le_bytes());
        let (latest, bits) = recv.header().expect("arrivals");
        bytes.extend_from_slice(&latest.to_le_bytes());
        bytes.extend_from_slice(&bits.to_le_bytes());
        (ledger, recv, bytes)
    };
    let (la, ra, ba) = run();
    let (lb, rb, bb) = run();
    assert_eq!((la, ra, ba), (lb, rb, bb));
}

#[test]
fn ack_header_uses_the_donor_bit_order() {
    let mut win = RecvWindow::default();
    assert_eq!(win.header(), None);
    assert_eq!(header_ack(&win), None);
    for seq in [10u16, 9, 7] {
        assert_eq!(win.on_receive(seq), Arrival::Fresh);
    }
    // Bit i means "latest - 1 - i arrived": 9 yes, 8 no, 7 yes.
    assert_eq!(win.header(), Some((10, 0b101)));
    assert_eq!(win.on_receive(9), Arrival::Duplicate);
    assert_eq!(win.on_receive(10), Arrival::Duplicate);
    assert_eq!(win.on_receive(8), Arrival::Fresh);
    assert_eq!(win.header(), Some((10, 0b111)));
    assert_eq!(win.on_receive(10u16.wrapping_sub(64)), Arrival::Stale);

    let ack = Ack::from_header(10, 0b101);
    assert_eq!(ack.head(), 10);
    assert_eq!(ack.span(), 33);
    assert_eq!(ack.arrived(10), Some(true), "the latest itself arrived");
    assert_eq!(ack.arrived(9), Some(true));
    assert_eq!(ack.arrived(8), Some(false));
    assert_eq!(ack.arrived(7), Some(true));
    assert_eq!(
        ack.arrived(10u16.wrapping_sub(32)),
        Some(false),
        "the 32nd bit is in range"
    );
    assert_eq!(
        ack.arrived(10u16.wrapping_sub(33)),
        None,
        "past the header field"
    );
    assert_eq!(ack.arrived(11), None, "newer than the head");
    assert_eq!(header_ack(&win), Some(Ack::from_header(10, 0b111)));
}

#[test]
fn ack_a_shift_of_32_or_more_keeps_only_what_the_field_can_name() {
    let mut win = RecvWindow::default();
    win.on_receive(100);
    win.on_receive(132);
    // 100 sits exactly 32 behind: bit 31, the last one the field has.
    assert_eq!(win.header(), Some((132, 1 << 31)));
    win.on_receive(133);
    // 132 is bit 0; 100 is 33 behind and fell out of the 32-bit field.
    assert_eq!(win.header(), Some((133, 1)));
    win.on_receive(1_000);
    assert_eq!(win.header(), Some((1_000, 0)));
}

#[test]
fn ack_receiver_estimate_leaves_the_in_flight_tail_out() {
    let mut win = RecvWindow::default();
    // Seqs 0..=63 with 61 and 62 still in flight and 10 lost.
    for seq in (0..64u16).filter(|s| ![10, 61, 62].contains(s)) {
        win.on_receive(seq);
    }
    // Judged: the 61 positions behind the newest three; 10 is the only miss.
    assert_eq!(win.loss_permille(), permille(1, 61));
    // 61 and 62 arriving late change nothing; 10 arriving late clears it.
    for seq in [61u16, 62] {
        assert_eq!(win.on_receive(seq), Arrival::Fresh);
    }
    assert_eq!(win.loss_permille(), permille(1, 61));
    win.on_receive(10);
    assert_eq!(win.loss_permille(), 0);
}

#[test]
fn ack_from_membership_names_only_its_span() {
    let ack = Ack::from_membership(20, 8, |seq| seq % 2 == 0);
    assert_eq!(ack.span(), 8);
    assert_eq!(ack.arrived(20), Some(true));
    assert_eq!(ack.arrived(19), Some(false));
    assert_eq!(ack.arrived(14), Some(true));
    assert_eq!(ack.arrived(13), Some(false));
    assert_eq!(
        ack.arrived(12),
        None,
        "older than the span: not vouched for"
    );
    // A span past the ring clamps; a zero span still names the head.
    assert_eq!(Ack::from_membership(5, 200, |_| true).span(), 64);
    assert_eq!(Ack::from_membership(5, 0, |_| true).span(), 1);
}

#[test]
fn ack_an_ack_resolves_only_what_it_vouches_for() {
    let mut ledger = SentLedger::new();
    for seq in 0..20 {
        ledger.on_send(seq).expect("rises");
    }
    // Vouches for 15..=19 only; 0..=14 stay pending (unknown, not lost).
    let ack = Ack::from_membership(19, 5, |seq| seq != 17);
    let got = ledger.on_ack(&ack).expect("ack");
    assert_eq!(
        got,
        Resolved {
            delivered: 4,
            lost: 1
        }
    );
    assert_eq!(ledger.pending(), 15);
    assert_eq!(ledger.loss_permille(), 200);
}

#[test]
fn ack_error_text_names_the_sequences() {
    let text = AckError::FromTheFuture { head: 9, newest: 4 }.to_string();
    assert!(text.contains('9') && text.contains('4'), "{text}");
    let text = AckError::NotNewer { seq: 3, newest: 8 }.to_string();
    assert!(text.contains('3') && text.contains('8'), "{text}");
    assert!(!AckError::NothingSent.to_string().is_empty());
}
