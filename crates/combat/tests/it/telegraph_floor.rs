//! Telegraph floors report: per-move signal-plus-attack time vs readable floors.
//!
//! Reads the shipped phased rows, prints one line per move with its
//! signal-plus-attack milliseconds, and checks the table reads identical
//! twice. Default mode only reports; `TELEGRAPH_STRICT=1` turns the floor
//! into a failure. Report only: no sim state changes, no hash input.

use engine_combat::exchange::{signal_attack_ms, PHASED_V1};

// Readable-attack floors in milliseconds: full telegraph, bait, end pose.
const FLOOR_MS: u32 = 340;
const BAIT_MS: u32 = 240;
const END_POSE_MS: u32 = 170;

// Highest readable floor cleared by one signal-plus-attack time in ms:
// full at 340, bait at 240, end pose at 170, else unreadable. Pure,
// no allocation, same input gives same verdict on every run.
fn floor_verdict(ms: u32) -> &'static str {
    if ms >= FLOOR_MS {
        "full"
    } else if ms >= BAIT_MS {
        "bait"
    } else if ms >= END_POSE_MS {
        "end-pose"
    } else {
        "unreadable"
    }
}

// One printed row per live move: id, startup, active, signal-plus-attack ms.
// Reads only `moves[0..count]`; spare slots hold placeholders, never steps.
fn telegraph_table() -> Vec<(u8, u32, u32, u32)> {
    let mut rows = Vec::new();
    let mut i: usize = 0;
    while i < usize::from(PHASED_V1.count) {
        let row = PHASED_V1.moves[i];
        rows.push((
            row.move_id,
            row.startup_ticks,
            row.active_ticks,
            signal_attack_ms(row.startup_ticks, row.active_ticks),
        ));
        i += 1;
    }
    rows
}

fn print_table(rows: &[(u8, u32, u32, u32)]) {
    for (id, startup, active, ms) in rows {
        println!(
            "move {id}: startup {startup} + active {active} = {ms} ms \
             vs floors {FLOOR_MS}/{BAIT_MS}/{END_POSE_MS} verdict {}",
            floor_verdict(*ms)
        );
    }
}

#[test]
fn telegraph_table_is_identical_x2() {
    let first = telegraph_table();
    let second = telegraph_table();
    assert_eq!(first, second);
    assert_eq!(usize::from(PHASED_V1.count), first.len());
    assert!(!first.is_empty());
    print_table(&first);
    // Pinned oracle for the shipped rows: jab 100 ms, cross 150 ms.
    // A retune must re-bless these numbers beside the floors.
    assert_eq!(first.len(), 2);
    assert_eq!(first[0], (0, 4, 2, 100));
    assert_eq!(first[1], (1, 6, 3, 150));
}

#[test]
fn telegraph_strict_fails_below_floor() {
    let rows = telegraph_table();
    print_table(&rows);
    let strict = std::env::var("TELEGRAPH_STRICT").unwrap_or_default() == "1";
    if strict {
        for (id, _, _, ms) in &rows {
            assert!(
                *ms >= FLOOR_MS,
                "strict: move {id} at {ms} ms sits below the {FLOOR_MS} ms floor"
            );
        }
    }
}

#[test]
fn tables_v1_signal_matches_fight_cadence() {
    // GS32 row text names TABLES_V1 while the live timing rides the phased
    // rows: pin them together. The jab whiff equals the fight-core startup
    // so the report can never drift from the stepped cadence, and both
    // shipped signals sit below the 340 ms floor (why strict fails today).
    use engine_combat::exchange::{PHASED_CROSS, PHASED_JAB};
    use engine_combat::fight::ATTACK_STARTUP_TICKS;
    let rows = telegraph_table();
    let second = telegraph_table();
    assert_eq!(rows, second, "TABLES_V1-derived report repeats x2");
    assert_eq!(
        PHASED_JAB.startup_ticks, ATTACK_STARTUP_TICKS,
        "jab whiff must match the fight-core startup"
    );
    assert_eq!(rows[0].3, 100, "jab signal 100 ms");
    assert_eq!(rows[1].3, 150, "cross signal 150 ms");
    assert_eq!(
        (PHASED_CROSS.startup_ticks + PHASED_CROSS.active_ticks) * 1_000 / 60,
        rows[1].3,
        "cross signal follows (startup + active) at 60 Hz"
    );
    for (id, _, _, ms) in &rows {
        assert!(
            *ms < FLOOR_MS,
            "move {id} at {ms} ms must sit below {FLOOR_MS} ms while heavies stay unreadable"
        );
    }
}

#[test]
fn signal_attack_ms_saturates_without_panic() {
    // Report-only ms must never panic, even on unvalidated rows, and must
    // clamp to u32::MAX when the exact (startup + active) * 1000 / 60
    // overflows u32 (validation rejects such rows before they ever step).
    assert_eq!(signal_attack_ms(4, 2), 100, "jab stays 100 ms");
    assert_eq!(signal_attack_ms(6, 3), 150, "cross stays 150 ms");
    assert_eq!(
        signal_attack_ms(u32::MAX, u32::MAX),
        u32::MAX,
        "saturated rows clamp to u32::MAX"
    );
    assert_eq!(
        signal_attack_ms(u32::MAX, 0),
        u32::MAX,
        "single saturated row clamps to u32::MAX"
    );
}

#[test]
fn telegraph_floors_classify_jab_cross_bait_full() {
    // Every floor constant must classify: jab and cross sit unreadable,
    // bait clears 240, full clears 340, end pose clears 170.
    let rows = telegraph_table();
    assert_eq!(floor_verdict(rows[0].3), "unreadable");
    assert_eq!(floor_verdict(rows[1].3), "unreadable");
    assert_eq!(floor_verdict(170), "end-pose");
    assert_eq!(floor_verdict(250), "bait");
    assert_eq!(floor_verdict(340), "full");
    assert_eq!(floor_verdict(500), "full");
}
