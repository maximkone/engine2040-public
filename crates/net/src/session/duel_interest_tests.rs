//! QN10: interest is a no-op on the duel. Two fighters, one viewer: the
//! selection takes the whole set and its wire bytes equal the encoding of the
//! same two rows with no interest pass at all. The interest lane earns its keep
//! on the 64-bot soak (`interest.rs`, `scale.rs`); a duel must not pay a tax.

use crate::interest::{entity_cost_bytes, InterestEntity};
use crate::scale::{SnapshotViews, DELTA_HEADER_LEN};

/// The viewer (id 0) and its rival a metre and a half in front, both a step
/// away from the registered baseline so each ships a pose row.
const BASE: [InterestEntity; 2] = [
    InterestEntity {
        id: 0,
        x_mm: 0,
        z_mm: 0,
        combat: 0x0001_0001,
    },
    InterestEntity {
        id: 1,
        x_mm: 0,
        z_mm: -1_600,
        combat: 0x0001_0001,
    },
];

/// The same two fighters one tick later: both moved, so both are pose-dirty.
const MOVED: [InterestEntity; 2] = [
    InterestEntity {
        id: 0,
        x_mm: 40,
        z_mm: -40,
        combat: 0x0001_0001,
    },
    InterestEntity {
        id: 1,
        x_mm: 60,
        z_mm: -1_500,
        combat: 0x0001_0001,
    },
];

/// One viewer over the duel roster, baseline registered, ready to select.
fn duel() -> SnapshotViews {
    let mut views = SnapshotViews::new(&[0], MOVED.len()).expect("one viewer, two slots");
    views
        .register_baseline(&BASE)
        .expect("baseline over two fighters");
    views
}

#[test]
fn duel_select_takes_the_whole_two_fighter_set() {
    let mut views = duel();
    let report = views.select(0, &MOVED, 40, -40, 0, 0).expect("select");
    assert_eq!(report.selected, 2, "both fighters reach the duel's viewer");
    let (_, ids) = views.selected(0).expect("kept selection");
    assert_eq!(ids, [0, 1], "the duel's ids ride ascending");
}

#[test]
fn duel_selection_bytes_equal_the_no_interest_encoding() {
    let mut views = duel();
    let report = views.select(0, &MOVED, 40, -40, 0, 0).expect("select");
    // With no interest pass a snapshot is one delta header plus one full row
    // per entity (nothing is acked yet): two fighters, two rows, none dropped.
    let plain = DELTA_HEADER_LEN + MOVED.len() * entity_cost_bytes(true, true, true);
    assert_eq!(
        report.bytes, plain,
        "the duel snapshot pays no interest tax"
    );
    assert_eq!(
        views.selected_bytes(),
        u64::try_from(plain).expect("fits"),
        "the ledger reads the same bytes"
    );
}

#[test]
fn duel_select_repeats_bit_for_bit() {
    let mut views = duel();
    let first = views.select(0, &MOVED, 40, -40, 0, 0).expect("select");
    let again = views.select(0, &MOVED, 40, -40, 0, 0).expect("select x2");
    // The second select has nothing new to say: the first take became the
    // baseline, so the same tick reports no pose change.
    assert_eq!(again.selected, 2, "the set stays whole on a quiet tick");
    assert_eq!(first.selected, again.selected);
    assert!(again.bytes <= first.bytes, "a quiet row never grows");
}
