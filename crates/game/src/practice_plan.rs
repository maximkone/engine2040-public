//! practice_plan: a practice dummy that fights back on your terms (SW80).
//!
//! One [`PracticePlan`] is the whole training setup as data: guard mode, dummy stance,
//! spacing band, fall recovery and the button jam. [`PracticePlan::input`] answers one
//! [`FighterInput`] per tick for the dummy's slot, so the fixed tick keeps reading inputs
//! and never learns a practice rule; the caller (a staged session or the window) feeds the
//! result through the input seam it already has.
//!
//! Units: pixels at the donor's 240p reference (the distance bands are the donor's
//! 10/80/130 px), wishes in the percent range `FighterInput` carries, fixed ticks at
//! 60 Hz. Determinism: one seeded xorshift and a pre-shuffled guard deck, no clock, no
//! allocation after [`PracticePlan::new`], so the same plan and the same seen ticks give
//! the same inputs byte for byte.
//!
//! Donor: https://github.com/ikemen-engine/Ikemen-GO/blob/bd2ead96/data/training.zss
//! (`_iksys_trainingGuardMode`, `_iksys_trainingDistance`, `_iksys_trainingDummyMode`,
//! `_iksys_trainingFallRecovery`, `assertInput`, `assertCommand{buffer.time: 2}`):
//! random guard under `random < 500` (50 permille per-mille), the auto-guard timer of
//! 15 ticks, the distance bands, and a fall-recovery command buffered 2 frames.

use engine_input::{
    fighter::{BLOCK, JUMP},
    Buttons, FighterId, FighterInput,
};

/// Guard duty of the dummy.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Guard {
    /// Never blocks.
    #[default]
    None,
    /// Blocks for [`AUTO_GUARD_TICKS`] ticks when an attack comes in.
    Auto,
    /// Blocks every attack.
    All,
    /// Blocks about half of them, from a seeded deck ([`RANDOM_GUARD_PERMILLE`]).
    Random,
}

/// What the dummy does with its body.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Dummy {
    /// Stands in its spacing band.
    #[default]
    Stand,
    /// Crouches in its band (the caller reads [`PracticePlan::stance`]).
    Crouch,
    /// Jumps whenever it is grounded.
    Jump,
}

/// Spacing band the dummy holds, donor numbers at 240p.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Distance {
    /// 10 px: chest to chest.
    #[default]
    Close,
    /// 80 px: a step and a half.
    Medium,
    /// 130 px: the far edge of a light.
    Far,
}

/// The dummy's stance a caller draws the body with (no button bit carries it).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Stance {
    /// Upright.
    #[default]
    Stand,
    /// Lowered, the crouch mode.
    Crouch,
}

/// Share of attacks a [`Guard::Random`] dummy blocks, per mille.
pub const RANDOM_GUARD_PERMILLE: usize = 500;
/// Attacks in one shuffled guard deck: half block, so the share is the donor's 50%
/// without a per-tick coin (a 40-attack drill lands 18-22 blocks every run).
pub const GUARD_DECK: usize = 40;
/// Ticks an [`Guard::Auto`] dummy keeps its guard up after an attack comes in.
pub const AUTO_GUARD_TICKS: u32 = 15;
/// Ticks a fall-recovery command stays buffered (donor `buffer.time: 2`).
pub const FALL_RECOVERY_BUFFER_TICKS: u32 = 2;
/// Ticks a jammed button stays pressed after the plan pressed it (button jam).
pub const JAM_TICKS: u32 = 8;
/// Forward wish toward the band, percent of top speed.
pub const APPROACH_WISH: i8 = 60;
/// Backward wish out of the band, percent of top speed.
pub const RETREAT_WISH: i8 = -60;
/// Pixels the plan closes (or opens) per tick at the caller's walk speed.
pub const CLOSE_PX_PER_TICK: i32 = 5;
/// Slack around a band that reads as held, pixels.
pub const BAND_SLACK_PX: i32 = 2;
/// Forward wish while getting up from a fall, percent of top speed.
pub const RECOVER_WISH: i8 = 30;

impl Distance {
    /// The band this mode holds, pixels.
    #[must_use]
    pub const fn band_px(self) -> i32 {
        match self {
            Self::Close => 10,
            Self::Medium => 80,
            Self::Far => 130,
        }
    }
}

/// What the plan can see of the fight on one tick; the caller fills it from the sim.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Seen {
    /// The dummy's slot.
    pub fighter: FighterId,
    /// Body distance to the opponent along the facing axis, pixels.
    pub gap_px: i32,
    /// Absolute facing, centidegrees.
    pub facing: i16,
    /// Feet on the sand.
    pub grounded: bool,
    /// An attack is in flight at the dummy.
    pub incoming: bool,
    /// The dummy is floored.
    pub floored: bool,
}

/// The whole practice setup as data, plus the little state a tick needs.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct PracticePlan {
    guard: Guard,
    dummy: Dummy,
    distance: Distance,
    fall_recovery: bool,
    button_jam: bool,
    rng: u64,
    deck: [bool; GUARD_DECK],
    deck_at: usize,
    auto_left: u32,
    recover_left: u32,
    jam: u8,
    jam_left: u32,
    stance: Stance,
    ticks_answered: u64,
}

impl PracticePlan {
    /// A plan over the five knobs, seeded for its guard deck.
    #[must_use]
    pub fn new(
        guard: Guard,
        dummy: Dummy,
        distance: Distance,
        fall_recovery: bool,
        button_jam: bool,
        seed: u64,
    ) -> Self {
        let mut plan = Self {
            guard,
            dummy,
            distance,
            fall_recovery,
            button_jam,
            rng: seed ^ 0x9E37_79B9_7F4A_7C15,
            deck: [false; GUARD_DECK],
            deck_at: 0,
            auto_left: 0,
            recover_left: 0,
            jam: 0,
            jam_left: 0,
            stance: Stance::Stand,
            ticks_answered: 0,
        };
        plan.shuffle_deck();
        plan
    }

    /// The guard mode.
    #[must_use]
    pub const fn guard(&self) -> Guard {
        self.guard
    }

    /// The dummy mode.
    #[must_use]
    pub const fn dummy(&self) -> Dummy {
        self.dummy
    }

    /// The spacing band.
    #[must_use]
    pub const fn distance(&self) -> Distance {
        self.distance
    }

    /// The stance the caller draws the body with.
    #[must_use]
    pub const fn stance(&self) -> Stance {
        self.stance
    }

    /// Ticks this plan has answered, one per [`Self::input`] call.
    #[must_use]
    pub const fn answered(&self) -> u64 {
        self.ticks_answered
    }

    /// The forward wish toward the band: positive closes, negative opens, zero holds.
    #[must_use]
    pub fn spacing_wish(&self, gap_px: i32) -> i8 {
        let band = self.distance.band_px();
        if gap_px > band + BAND_SLACK_PX {
            APPROACH_WISH
        } else if gap_px < band - BAND_SLACK_PX {
            RETREAT_WISH
        } else {
            0
        }
    }

    /// Pixels to move this tick toward the band, at most [`CLOSE_PX_PER_TICK`]: the
    /// caller's own mover reads this, so the plan still only names the wish.
    #[must_use]
    pub fn spacing_step_px(&self, gap_px: i32) -> i32 {
        let err = gap_px - self.distance.band_px();
        err.clamp(-CLOSE_PX_PER_TICK, CLOSE_PX_PER_TICK)
    }

    /// One tick of the dummy: the input the fixed tick reads, never a new rule.
    pub fn input(&mut self, tick: u32, seen: Seen) -> FighterInput {
        self.ticks_answered = self.ticks_answered.saturating_add(1);
        let mut buttons = 0u8;
        let mut mx = self.spacing_wish(seen.gap_px);

        // A fall-recovery command is buffered like the donor's: the wish is held for
        // FALL_RECOVERY_BUFFER_TICKS, then the get-up jump fires on the first grounded tick.
        if seen.floored && self.fall_recovery {
            self.recover_left = FALL_RECOVERY_BUFFER_TICKS;
        }
        if self.recover_left > 0 {
            self.recover_left -= 1;
            mx = RECOVER_WISH;
            if seen.grounded && !seen.floored {
                buttons |= JUMP;
            }
        }

        // Guard duty.
        let mut guard = false;
        if seen.incoming && !seen.floored {
            match self.guard {
                Guard::None => {}
                Guard::All => guard = true,
                Guard::Auto => self.auto_left = AUTO_GUARD_TICKS,
                Guard::Random => guard = self.draw_guard(),
            }
        }
        if self.auto_left > 0 {
            guard = true;
            self.auto_left -= 1;
        }
        if guard {
            buttons |= BLOCK;
        }

        // Dummy mode.
        self.stance = match self.dummy {
            Dummy::Crouch => Stance::Crouch,
            Dummy::Stand | Dummy::Jump => Stance::Stand,
        };
        if matches!(self.dummy, Dummy::Jump) && seen.grounded && self.recover_left == 0 {
            buttons |= JUMP;
        }

        // Button jam: the last button the plan pressed stays pressed for JAM_TICKS.
        if self.button_jam {
            if self.jam_left > 0 {
                buttons |= self.jam;
                self.jam_left -= 1;
            } else if buttons != 0 {
                self.jam = buttons;
                self.jam_left = JAM_TICKS;
            }
        }

        FighterInput {
            tick,
            fighter: seen.fighter,
            mx,
            mz: 0,
            facing: seen.facing,
            buttons: Buttons(buttons),
        }
    }

    /// Next guard card: true blocks. Refills the deck when it runs out.
    fn draw_guard(&mut self) -> bool {
        if self.deck_at >= GUARD_DECK {
            self.shuffle_deck();
        }
        let card = self.deck[self.deck_at];
        self.deck_at += 1;
        card
    }

    /// A fresh deck: exactly `GUARD_DECK * RANDOM_GUARD_PERMILLE / 1000` blocking cards,
    /// shuffled by the seeded xorshift (Fisher-Yates, no allocation).
    fn shuffle_deck(&mut self) {
        let blocks = GUARD_DECK * RANDOM_GUARD_PERMILLE / 1000;
        for (index, card) in self.deck.iter_mut().enumerate() {
            *card = index < blocks;
        }
        for i in (1..GUARD_DECK).rev() {
            let bound = u64::try_from(i + 1).unwrap_or(1);
            let j = usize::try_from(self.next_u64() % bound).unwrap_or(0);
            self.deck.swap(i, j);
        }
        self.deck_at = 0;
    }

    /// xorshift64*: deterministic from the seed, no clock.
    fn next_u64(&mut self) -> u64 {
        let mut x = self.rng;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.rng = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
}

/// Counters a plan keeps that are not part of its data knobs.
impl PracticePlan {
    /// Cards drawn from the current guard deck.
    #[must_use]
    pub const fn deck_position(&self) -> usize {
        self.deck_at
    }
}

#[cfg(test)]
#[path = "practice_plan_tests.rs"]
mod tests;
