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
    if p.grace_ticks > 12 || p.buffer_ticks > 12 || p.grace_ticks >= p.startup_ticks {
        return Err(DesignError::BadCancel);
    }
    if latency_ms(p.buffer_ticks) > 50 {
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

/// Latency in millis: buffer ticks at sixty hertz, integer math only.
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

    #[test]
    fn bad_rows_refuse_with_their_kind() {
        let mut flat = SHIPPED;
        flat.heavy_ticks = 3;
        assert_eq!(validate(&flat), Err(DesignError::BadHitStop));
        let mut slow = SHIPPED;
        slow.buffer_ticks = 4;
        assert_eq!(validate(&slow), Err(DesignError::BadCancel));
        assert_eq!(latency_ms(4), 66);
        let mut wide = SHIPPED;
        wide.parry_window_ticks = 15;
        assert_eq!(validate(&wide), Err(DesignError::BadReaction));
    }
}
