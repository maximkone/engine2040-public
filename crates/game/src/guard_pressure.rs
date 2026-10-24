//! guard_pressure: the posture meter behind a guard, read off the exchange
//! (row SW81). A block spends guard, a deflect refunds none and relieves some,
//! three quiet seconds start a heal, and a full meter breaks the guard and
//! opens the riposte window the exchange already tracks.
//!
//! HUD only: this is view state beside the fight, never a sim rule, so the
//! tick and the golden `d57b5c8ae3096614` never read it. Donor:
//! Shadow-46/adaptive-boss-arena `Combat/DefenceResolver.cs` (`ResolveDefence`,
//! `BlockPostureMultiplier` 0.6, `ResistsStagger`), read 2026-10-24; the 3 s
//! quiet window is Sekiro's rule that a guard recovers once the exchange
//! stops. Units: meter points out of [`BREAK_AT`] 1000, ticks at 60 Hz.
//! Determinism: integer points and ticks; the same blocks give the same meter
//! twice. Errors: none; every function is total.

/// Meter points at which the guard breaks.
pub const BREAK_AT: u16 = 1_000;
/// Points one block costs with the guard not held.
pub const BLOCK_COST: u16 = 340;
/// Share of the block cost paid while the guard is held (the donor's
/// `BlockPostureMultiplier` 0.6), in permille.
pub const HELD_COST_PERMILLE: u16 = 600;
/// Points a deflect takes back off the meter (a deflect costs nothing).
pub const DEFLECT_RELIEF: u16 = 40;
/// Quiet ticks before the guard heals: three seconds at 60 Hz.
pub const QUIET_TICKS: u32 = 180;
/// Points a quiet tick heals once the window is open.
pub const HEAL_PER_TICK: u16 = 25;

/// One fighter's guard meter.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GuardPressure {
    pressure: u16,
    quiet: u32,
    broken: bool,
    riposte: bool,
}

impl GuardPressure {
    /// A fresh, unbroken guard.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            pressure: 0,
            quiet: 0,
            broken: false,
            riposte: false,
        }
    }

    /// Meter points spent so far, `0..=1000`.
    #[must_use]
    pub const fn pressure(&self) -> u16 {
        self.pressure
    }

    /// True once the meter filled: the guard is broken.
    #[must_use]
    pub const fn broken(&self) -> bool {
        self.broken
    }

    /// True while a break has opened the riposte window and nobody consumed it.
    #[must_use]
    pub const fn riposte_open(&self) -> bool {
        self.riposte
    }

    /// Take the riposte window, closing it: the caller answers once per break.
    pub fn take_riposte(&mut self) -> bool {
        let open = self.riposte;
        self.riposte = false;
        open
    }

    /// Meter points one block costs: [`BLOCK_COST`], scaled to
    /// [`HELD_COST_PERMILLE`] while the guard is held (a sustained guard
    /// wears faster than a walked-off one).
    #[must_use]
    pub fn block_cost(held: bool) -> u16 {
        if held {
            let scaled = u32::from(BLOCK_COST) * u32::from(HELD_COST_PERMILLE) / 1000;
            u16::try_from(scaled).unwrap_or(BLOCK_COST)
        } else {
            BLOCK_COST
        }
    }

    /// One blocked blow: the meter fills, the quiet window restarts, and
    /// filling it breaks the guard and opens the riposte.
    pub fn block(&mut self, held: bool) {
        self.quiet = 0;
        self.pressure = self
            .pressure
            .saturating_add(Self::block_cost(held))
            .min(BREAK_AT);
        if self.pressure >= BREAK_AT {
            self.broken = true;
            self.riposte = true;
        }
    }

    /// One deflected blow: costs nothing and takes [`DEFLECT_RELIEF`] back
    /// off. A deflect on a fresh meter leaves it byte-equal.
    pub fn deflect(&mut self) {
        self.quiet = 0;
        self.pressure = self.pressure.saturating_sub(DEFLECT_RELIEF);
    }

    /// One quiet tick: after [`QUIET_TICKS`] with no blow the meter heals
    /// [`HEAL_PER_TICK`], and a broken guard reopens once it drops below
    /// [`BREAK_AT`].
    pub fn tick(&mut self) {
        self.quiet = self.quiet.saturating_add(1);
        if self.quiet < QUIET_TICKS || self.pressure == 0 {
            return;
        }
        self.pressure = self.pressure.saturating_sub(HEAL_PER_TICK);
        if self.pressure < BREAK_AT {
            self.broken = false;
        }
    }

    /// The bar's fill in permille, `0..=1000`, for the HUD.
    #[must_use]
    pub const fn fill_permille(&self) -> u16 {
        self.pressure
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn five_blocks_inside_forty_ticks_break_the_guard() {
        // A sustained guard (held, the row's 0.6 scale) pays 204 a block:
        // four blocks hold, the fifth fills the meter inside 40 ticks.
        let mut guard = GuardPressure::new();
        assert_eq!(GuardPressure::block_cost(true), 204, "0.6 of 340");
        assert_eq!(
            GuardPressure::block_cost(false),
            340,
            "a free guard pays all"
        );
        for block in 0..4 {
            for _ in 0..(40 / 4) {
                guard.tick();
            }
            guard.block(true);
            assert!(!guard.broken(), "block {block} holds");
        }
        assert_eq!(guard.pressure(), 4 * 204, "four blocks on the meter");
        guard.block(true);
        assert_eq!(guard.pressure(), BREAK_AT, "the fifth fills it");
        assert!(guard.broken(), "five blocks break the guard");
        assert!(guard.riposte_open(), "the break opens the riposte window");
        assert!(guard.take_riposte(), "read once");
        assert!(!guard.riposte_open(), "and closed after it");

        // Tick count matters: 40 quiet ticks would already start the heal, so
        // the five blocks must land inside that window to break.
        let mut slow = GuardPressure::new();
        for _ in 0..4 {
            for _ in 0..QUIET_TICKS {
                slow.tick();
            }
            slow.block(true);
        }
        slow.block(true);
        assert!(
            !slow.broken(),
            "a healed guard survives the same five blocks"
        );
    }

    #[test]
    fn a_deflect_leaves_the_meter_byte_equal() {
        // A deflect costs nothing: on a fresh meter it is a byte-equal no-op.
        let mut guard = GuardPressure::new();
        let before = guard;
        guard.deflect();
        assert_eq!(guard.pressure(), before.pressure(), "no cost on a deflect");
        assert_eq!(guard, before, "a deflect from zero changes nothing");
        // On a charged meter it takes exactly the relief back off.
        guard.block(false);
        guard.block(false);
        let charged = guard.pressure();
        guard.deflect();
        assert_eq!(guard.pressure(), charged - DEFLECT_RELIEF);
        // A deflect never breaks: it cannot fill the meter.
        assert!(!guard.broken());
    }

    #[test]
    fn three_quiet_seconds_heal_the_guard() {
        // 179 quiet ticks heal nothing; on the 180th the meter starts healing
        // 25 a tick, and a broken guard is whole again below the break.
        let mut guard = GuardPressure::new();
        guard.block(true);
        guard.block(true);
        guard.block(true);
        guard.block(true);
        guard.block(true);
        assert!(guard.broken(), "five held blocks break it");
        for _ in 0..QUIET_TICKS - 1 {
            guard.tick();
        }
        assert_eq!(
            guard.pressure(),
            BREAK_AT,
            "still full before three seconds"
        );
        guard.tick();
        assert_eq!(
            guard.pressure(),
            BREAK_AT - HEAL_PER_TICK,
            "the heal starts"
        );
        assert!(!guard.broken(), "one point back and the guard reopens");
        for _ in 0..4 {
            guard.tick();
        }
        assert_eq!(guard.pressure(), BREAK_AT - 5 * HEAL_PER_TICK);
        // A blow restarts the quiet window.
        guard.block(true);
        for _ in 0..QUIET_TICKS - 1 {
            guard.tick();
        }
        let after = guard.pressure();
        guard.tick();
        assert_eq!(guard.pressure(), after - HEAL_PER_TICK, "quiet restarted");
    }

    #[test]
    fn the_same_blocks_twice_give_the_same_meter() {
        // x2: one tape of blocks, deflects and quiet ticks gives one meter.
        fn tape() -> GuardPressure {
            let mut guard = GuardPressure::new();
            for round in 0..6 {
                guard.block(round % 2 == 0);
                for _ in 0..10 {
                    guard.tick();
                }
                if round == 2 {
                    guard.deflect();
                }
            }
            for _ in 0..QUIET_TICKS {
                guard.tick();
            }
            guard
        }
        let (first, second) = (tape(), tape());
        assert_eq!(first, second, "same tape, same meter");
        assert_eq!(first.fill_permille(), first.pressure(), "the bar reads it");
        assert!(first.pressure() <= BREAK_AT);
    }
}
