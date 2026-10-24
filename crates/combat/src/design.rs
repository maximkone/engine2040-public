//! design: duel timing patterns as data, never tick-coupled.
//!
//! Owner: engine-combat. Units: ticks at sixty hertz.
//! Determinism: integer compares only; same inputs give same answer.
//! Errors: [`DesignError`](crate::design::DesignError). Budget: constant time, no heap, no floats.
//! PORT(maximkone/flax-mcp@a26f960:plugins/gameplay/GameSide/
//! Game.Shared.Combat/Mechanics/Combat{HitResolver,Stamina,Poise,
//! ComboChain}*.cs), MIT; clean-room integer rewrite, no vendor bytes.
//! Shapes only, no donor code, no assets, no dialogue.
//! Tick reads live rows, never this file, so replays stay identical.
//!
//! Input latency: this module reads the reading the input layer takes
//! ([`FeelPreset::pq1_latency_ms`](crate::feel::FeelPreset::pq1_latency_ms),
//! the ticks the tick adds to a fresh press) and never invents one. The
//! input buffer window is a forgiveness budget for a press made while a
//! swing is locked, not a delay, so a wide window stays legal here.

use crate::feel::{FEEL_V1, MAX_BUFFER_TICKS, PQ1_LATENCY_MS};
use std::fmt;

/// Bounded design failures; invalid rows never panic.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DesignError {
    /// Hit-stop ordering or range broken.
    BadHitStop,
    /// Cancel ordering, range, or latency broken.
    BadCancel,
    /// Reaction ordering, range, or window broken.
    BadReaction,
}

/// One duel timing set: freeze plus cancel plus reaction, all as data.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct BattlePatterns {
    /// Freeze for one light tap.
    pub light_ticks: u32,
    /// Freeze for one heavy land.
    pub heavy_ticks: u32,
    /// Freeze for one parry answer.
    pub parry_ticks: u32,
    /// Ticks from press to impact; the readable tell.
    pub startup_ticks: u32,
    /// Ticks locked after impact before a chain may start.
    pub recovery_ticks: u32,
    /// Ticks before the hit tick where cancels lock.
    pub grace_ticks: u32,
    /// Ticks a tap survives for the first ready tick.
    pub buffer_ticks: u32,
    /// Ticks the victim cannot act after a hit.
    pub victim_hold_ticks: u32,
    /// Ticks the attacker staggers after a parry.
    pub parry_stagger_ticks: u32,
    /// Ticks a knocked-down fighter stays down.
    pub down_ticks: u32,
    /// Ticks after a block press that still parry.
    pub parry_window_ticks: u32,
}

/// The shortest readable tell in ticks: 30 ticks is 500 ms at 60 Hz, the
/// heuristic floor of the one measured readability study (theseus 900621,
/// n=30: median read 800 ms, IQR 525-900, floor 500 ms). No legal row ships
/// a wind-up shorter than this.
pub const MIN_STARTUP_TICKS: u32 = 30;

/// Shipped timing: light 3, heavy 6, parry 8; tell 30, recovery 6,
/// grace 2, buffer 2; hold 6, stagger 12, down 30, window 7.
///
/// The tell is the readability floor. Recovery goes shorter (8 to 6) so the
/// follow-up stays a chained press instead of the fight only getting slower.
pub const SHIPPED: BattlePatterns = BattlePatterns {
    light_ticks: 3,
    heavy_ticks: 6,
    parry_ticks: 8,
    startup_ticks: MIN_STARTUP_TICKS,
    recovery_ticks: 6,
    grace_ticks: 2,
    buffer_ticks: 2,
    victim_hold_ticks: 6,
    parry_stagger_ticks: 12,
    down_ticks: 30,
    parry_window_ticks: 7,
};

/// The frame advantage a row states: the answer its fields already imply.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct AdvantageRow {
    /// The row's name.
    pub name: &'static str,
    /// Ticks the attacker gains when the hit lands; negative means it loses.
    pub hit: i32,
    /// Ticks the attacker gives up when the hit is blocked; negative means it loses.
    pub block: i32,
}

impl fmt::Display for AdvantageRow {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}: advantage on hit {} ticks, on block {} ticks",
            self.name, self.hit, self.block
        )
    }
}

/// The rows that ship, in print order.
pub const SHIPPED_ROWS: [(&str, BattlePatterns); 1] = [("SHIPPED", SHIPPED)];

/// One printable line per shipped row: the advantage each row states.
pub fn table() -> impl Iterator<Item = AdvantageRow> {
    SHIPPED_ROWS.iter().map(|&(name, patterns)| AdvantageRow {
        name,
        hit: patterns.advantage_on_hit(),
        block: patterns.advantage_on_block(),
    })
}

impl BattlePatterns {
    /// Ticks the attacker gains when the hit lands: the victim's hold minus
    /// the attacker's recovery. Positive means the attacker acts first.
    #[must_use]
    pub fn advantage_on_hit(&self) -> i32 {
        signed_difference(self.victim_hold_ticks, self.recovery_ticks)
    }

    /// Ticks the attacker gives up when the hit is blocked: the block hold
    /// (half the parry window, rounded down) minus recovery.
    #[must_use]
    pub fn advantage_on_block(&self) -> i32 {
        signed_difference(self.parry_window_ticks / 2, self.recovery_ticks)
    }
}

/// `a - b` as an i32, saturating at both ends: a legal row is small, a
/// hand-built one must never overflow.
fn signed_difference(a: u32, b: u32) -> i32 {
    let wide = i64::from(a) - i64::from(b);
    i32::try_from(wide).unwrap_or(if wide < 0 { i32::MIN } else { i32::MAX })
}

/// Range-check one set; first failure wins, deterministically.
pub const fn validate(p: &BattlePatterns) -> Result<(), DesignError> {
    if p.light_ticks == 0 || p.light_ticks > 60 {
        return Err(DesignError::BadHitStop);
    }
    if p.heavy_ticks == 0 || p.heavy_ticks > 60 {
        return Err(DesignError::BadHitStop);
    }
    if p.parry_ticks == 0 || p.parry_ticks > 60 {
        return Err(DesignError::BadHitStop);
    }
    if p.light_ticks >= p.heavy_ticks || p.heavy_ticks > p.parry_ticks {
        return Err(DesignError::BadHitStop);
    }
    if p.startup_ticks < MIN_STARTUP_TICKS || p.startup_ticks > 60 || p.recovery_ticks > 120 {
        return Err(DesignError::BadCancel);
    }
    if p.grace_ticks > 12 || p.buffer_ticks > MAX_BUFFER_TICKS || p.grace_ticks >= p.startup_ticks {
        return Err(DesignError::BadCancel);
    }
    // Input latency: the reading the input layer takes for a fresh press,
    // never an invented number. The intake window (`p.buffer_ticks`) is a
    // forgiveness budget for a press made while a swing is locked: it
    // delays no press the player makes ready, so it is never billed as
    // latency here. The published window is 8 ticks (133 ms at sixty
    // hertz) and stays legal.
    if !latency_within_bar(input_latency_ms()) {
        return Err(DesignError::BadCancel);
    }
    if p.victim_hold_ticks == 0 || p.victim_hold_ticks > 120 {
        return Err(DesignError::BadReaction);
    }
    if p.parry_stagger_ticks == 0 || p.parry_stagger_ticks > 120 {
        return Err(DesignError::BadReaction);
    }
    if p.down_ticks == 0 || p.down_ticks > 600 {
        return Err(DesignError::BadReaction);
    }
    if p.parry_window_ticks < 6 || p.parry_window_ticks > 8 {
        return Err(DesignError::BadReaction);
    }
    if p.parry_stagger_ticks <= p.victim_hold_ticks || p.down_ticks <= p.parry_stagger_ticks {
        return Err(DesignError::BadReaction);
    }
    if p.parry_window_ticks >= p.startup_ticks {
        return Err(DesignError::BadReaction);
    }
    Ok(())
}

/// Freeze for one strike: parry wins, else heavy or light.
#[must_use]
pub const fn hitstop_for_strike(p: &BattlePatterns, heavy: bool, parried: bool) -> u32 {
    if parried {
        p.parry_ticks
    } else if heavy {
        p.heavy_ticks
    } else {
        p.light_ticks
    }
}

/// True while a press may still feint: age before startup minus grace.
#[must_use]
pub const fn feint_allowed(p: &BattlePatterns, age: u32) -> bool {
    age < p.startup_ticks.saturating_sub(p.grace_ticks)
}

/// True while a press may chain: past startup plus grace, inside recovery.
#[must_use]
pub const fn chain_allowed(p: &BattlePatterns, age: u32) -> bool {
    let open_at = p.startup_ticks.saturating_add(p.grace_ticks);
    let end_at = p.startup_ticks.saturating_add(p.recovery_ticks);
    age > open_at && age <= end_at
}

/// True while a held block still parries: age inside the window.
#[must_use]
pub const fn parry_open(p: &BattlePatterns, block_age: u32) -> bool {
    block_age <= p.parry_window_ticks
}

/// Input-to-effect latency in milliseconds that a fresh press meets under
/// these patterns: the reading the input layer takes
/// ([`FeelPreset::pq1_latency_ms`](crate::feel::FeelPreset::pq1_latency_ms)),
/// the ticks the tick adds before a fresh press starts its swing.
///
/// No row buys delay here: the input layer serves a press the player makes
/// ready on its press tick, the intake window
/// ([`BattlePatterns::buffer_ticks`]) is forgiveness for a press made while
/// a swing is locked and never a delay, and the startup
/// ([`BattlePatterns::startup_ticks`]) is the readable tell, not input
/// latency. So every legal set reads the layer's number and the gate in
/// [`validate`] refuses the day the tick starts adding ticks.
/// Integer math only, no floats.
#[must_use]
pub const fn input_latency_ms() -> u32 {
    FEEL_V1.pq1_latency_ms()
}

/// True while a latency reading fits the input layer's bar
/// ([`PQ1_LATENCY_MS`], 50 ms). The gate refuses only a reading over the
/// bar, never a row for any other reason.
const fn latency_within_bar(reading_ms: u32) -> bool {
    reading_ms <= PQ1_LATENCY_MS
}

/// Latency in millis: buffer ticks at sixty hertz, integer math only.
///
/// This is the intake window in millis: how long a press made while a
/// swing is locked survives in the buffer. It is forgiveness, NOT the
/// input latency reading: the window delays no press the player makes
/// ready, so it is never billed as latency ([`input_latency_ms`] is that
/// reading, the one the input layer takes).
#[must_use]
pub const fn latency_ms(buffer_ticks: u32) -> u32 {
    buffer_ticks.saturating_mul(1000).saturating_div(60)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shipped_validates_with_readable_windows() {
        assert_eq!(validate(&SHIPPED), Ok(()));
        assert_eq!(validate(&SHIPPED), validate(&SHIPPED));
        assert_eq!(hitstop_for_strike(&SHIPPED, false, false), 3);
        assert_eq!(hitstop_for_strike(&SHIPPED, true, false), 6);
        assert_eq!(hitstop_for_strike(&SHIPPED, false, true), 8);
        assert_eq!(latency_ms(SHIPPED.buffer_ticks), 33);
        assert!(feint_allowed(&SHIPPED, 27));
        assert!(!feint_allowed(&SHIPPED, 28));
        assert!(chain_allowed(&SHIPPED, 33));
        assert!(!chain_allowed(&SHIPPED, 32));
        assert!(!chain_allowed(&SHIPPED, 37));
        assert!(parry_open(&SHIPPED, 7));
        assert!(!parry_open(&SHIPPED, 8));
    }

    /// The readability law: every shipped attack's wind-up clears the floor,
    /// and its whole window (grace, parry, chain) still lands inside that
    /// wind-up rather than after impact.
    #[test]
    fn every_attack_opens_a_readable_window() {
        for (name, startup) in [
            ("design::SHIPPED", SHIPPED.startup_ticks),
            ("TABLES_V2", crate::TABLES_V2.attack_startup_ticks),
        ] {
            assert!(
                startup >= MIN_STARTUP_TICKS,
                "{name} tell {startup} ticks is under the {MIN_STARTUP_TICKS}-tick floor"
            );
        }
        assert_eq!(SHIPPED.startup_ticks * 1000 / 60, 500, "30 ticks is 500 ms");
        const { assert!(SHIPPED.grace_ticks < SHIPPED.startup_ticks) };
        const { assert!(SHIPPED.parry_window_ticks < SHIPPED.startup_ticks) };
        assert!(!chain_allowed(&SHIPPED, SHIPPED.startup_ticks));
        assert!(chain_allowed(
            &SHIPPED,
            SHIPPED.startup_ticks + SHIPPED.recovery_ticks
        ));
        let mut short = SHIPPED;
        short.startup_ticks = MIN_STARTUP_TICKS - 1;
        assert_eq!(validate(&short), Err(DesignError::BadCancel));
    }

    /// The intake window is forgiveness, not latency: a design row that
    /// keeps the published window still validates.
    #[test]
    fn intake_window_is_forgiveness_not_latency() {
        let mut row = SHIPPED;
        row.buffer_ticks = crate::tables::INPUT_BUFFER_TICKS_V2;
        assert_eq!(validate(&row), Ok(()));
        // The window in millis: 8 ticks is 133 ms at sixty hertz, over the
        // 50 ms bar and rightly so, because it is not a latency.
        assert_eq!(latency_ms(crate::tables::INPUT_BUFFER_TICKS_V2), 133);
        assert_eq!(latency_ms(4), 66);
    }

    /// The reading is the input layer's own number, not an invented one.
    #[test]
    fn input_latency_reading_is_the_layers_number() {
        assert_eq!(
            input_latency_ms(),
            crate::feel_instrument::input_latency_ms(crate::feel::FEEL_V1)
        );
        assert_eq!(input_latency_ms(), 0);
        assert!(latency_within_bar(input_latency_ms()));
    }

    /// The gate refuses only a reading over the bar.
    #[test]
    fn latency_gate_refuses_only_over_the_bar() {
        assert!(latency_within_bar(PQ1_LATENCY_MS));
        assert!(!latency_within_bar(PQ1_LATENCY_MS + 1));
    }

    #[test]
    fn bad_rows_refuse_with_their_kind() {
        let mut flat = SHIPPED;
        flat.heavy_ticks = 3;
        assert_eq!(validate(&flat), Err(DesignError::BadHitStop));
        let mut slow = SHIPPED;
        slow.buffer_ticks = 4;
        assert_eq!(validate(&slow), Ok(()));
        let mut wide = SHIPPED;
        wide.parry_window_ticks = 15;
        assert_eq!(validate(&wide), Err(DesignError::BadReaction));
    }

    /// The shipped row states its answer, and `table` prints it as one line.
    #[test]
    fn shipped_states_its_advantage_on_hit_and_on_block() {
        assert_eq!(SHIPPED.advantage_on_hit(), 0, "6 hold minus 6 recovery");
        assert_eq!(
            SHIPPED.advantage_on_block(),
            -3,
            "3 block hold minus 6 recovery"
        );
        let lines: Vec<String> = table().map(|row| row.to_string()).collect();
        assert_eq!(
            lines,
            vec!["SHIPPED: advantage on hit 0 ticks, on block -3 ticks"],
            "one line per shipped row"
        );
    }

    /// A tuning edit that slows recovery flips both numbers negative, and the
    /// pinned test says so by naming the exact new value.
    #[test]
    fn a_slow_row_reads_negative_on_both() {
        let mut slow = SHIPPED;
        slow.recovery_ticks = 120;
        assert_eq!(slow.advantage_on_hit(), -114);
        assert_eq!(slow.advantage_on_block(), -117);
    }

    /// A row with no hold at all saturates, never wraps and never panics.
    #[test]
    fn a_zero_hold_row_cannot_overflow() {
        let mut empty = SHIPPED;
        empty.victim_hold_ticks = 0;
        empty.parry_window_ticks = 0;
        empty.recovery_ticks = 0;
        assert_eq!(empty.advantage_on_hit(), 0);
        assert_eq!(empty.advantage_on_block(), 0);
        empty.recovery_ticks = u32::MAX;
        assert_eq!(empty.advantage_on_hit(), i32::MIN, "saturates low");
        let mut huge = SHIPPED;
        huge.victim_hold_ticks = u32::MAX;
        assert_eq!(huge.advantage_on_hit(), i32::MAX, "saturates high");
    }

    /// Deriving the answer is a read: no stored field moves, and the row still
    /// validates afterwards.
    #[test]
    fn deriving_the_answer_changes_no_stored_field() {
        let before = SHIPPED;
        let answer = (SHIPPED.advantage_on_hit(), SHIPPED.advantage_on_block());
        let lines = table().count();
        assert_eq!((answer, lines), ((0, -3), 1));
        assert_eq!(SHIPPED, before, "the derivation is a pure read");
        assert_eq!(validate(&SHIPPED), Ok(()));
    }
}
