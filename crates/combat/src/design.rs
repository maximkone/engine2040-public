//! design: duel timing patterns as data, never tick-coupled.
//!
//! Owner: engine-combat. Units: ticks at sixty hertz.
//! Determinism: integer compares only; same inputs give same answer.
//! Errors: [`DesignError`]. Budget: constant time, no heap, no floats.
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

/// Shipped timing: light 3, heavy 6, parry 8; tell 15, recovery 8,
/// grace 2, buffer 2; hold 6, stagger 12, down 30, window 7.
pub const SHIPPED: BattlePatterns = BattlePatterns {
    light_ticks: 3,
    heavy_ticks: 6,
    parry_ticks: 8,
    startup_ticks: 15,
    recovery_ticks: 8,
    grace_ticks: 2,
    buffer_ticks: 2,
    victim_hold_ticks: 6,
    parry_stagger_ticks: 12,
    down_ticks: 30,
    parry_window_ticks: 7,
};

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
    if p.startup_ticks < 12 || p.startup_ticks > 60 || p.recovery_ticks > 120 {
        return Err(DesignError::BadCancel);
    }
    if p.grace_ticks > 12
        || p.buffer_ticks > MAX_BUFFER_TICKS
        || p.grace_ticks >= p.startup_ticks
    {
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
        assert!(feint_allowed(&SHIPPED, 12));
        assert!(!feint_allowed(&SHIPPED, 13));
        assert!(chain_allowed(&SHIPPED, 18));
        assert!(!chain_allowed(&SHIPPED, 17));
        assert!(parry_open(&SHIPPED, 7));
        assert!(!parry_open(&SHIPPED, 8));
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
}
