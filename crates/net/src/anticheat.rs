//! anticheat: the sanitize rule of the dedicated server and its flag log.
//!
//! Owner: net crate (flags only; admission authority is [`crate::guard`]).
//! Caller: [`InputGuard::admit`](crate::guard::InputGuard::admit) runs every
//! admitted client input through [`sanitize`] and notes each changed input
//! in the guard's [`FlagLog`], read back through
//! [`HeadlessServer::flagged_inputs`](crate::dedicated::HeadlessServer::flagged_inputs).
//! The wire codec already refuses impossible fields; this rule normalizes
//! what is legal on the wire but not in this match (a spear press in a match
//! without the spear), so the authoritative tape is the one the match plays
//! and a replay names every tick the server changed.
//! Units: fixed ticks, `F_*` bit flags. Determinism: integer bit ops and
//! admission order only. Errors: none. Budget: O(1) per input; the log is
//! bounded at [`MAX_FLAGGED`] and counts the rest.

use engine_core::Fnv64;
use engine_input::fighter::{Buttons, THROW_SPEAR};

use crate::session::BodyInput;

/// Sanitize flag: the spear bit was masked off (the match has no spear).
pub const F_SPEAR: u8 = 0x01;
/// Flagged inputs a log keeps; later ones are counted only.
pub const MAX_FLAGGED: usize = 256;

/// Normalize one input to the match's rules: in a match without the spear
/// the spear bit is masked off and [`F_SPEAR`] is returned. The arena masks
/// the same bit, so the state hash is the same either way.
#[must_use]
pub fn sanitize(input: BodyInput, spear: bool) -> (BodyInput, u8) {
    if spear || input.buttons.0 & THROW_SPEAR == 0 {
        return (input, 0);
    }
    let buttons = Buttons(input.buttons.0 & !THROW_SPEAR);
    (BodyInput { buttons, ..input }, F_SPEAR)
}

/// One admitted input [`sanitize`] changed: where, and which flags.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Flagged {
    /// Tick of the input.
    pub tick: u32,
    /// Slot that sent it.
    pub slot: u8,
    /// `F_*` bits.
    pub flags: u8,
}

/// Bounded log of flagged inputs in admission order.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct FlagLog {
    entries: Vec<Flagged>,
    dropped: u32,
}

impl FlagLog {
    /// Note one changed input; past [`MAX_FLAGGED`] it is only counted.
    pub fn note(&mut self, entry: Flagged) {
        if self.entries.len() < MAX_FLAGGED {
            self.entries.push(entry);
        } else {
            self.dropped = self.dropped.saturating_add(1);
        }
    }

    /// Kept entries in admission order.
    #[must_use]
    pub fn entries(&self) -> &[Flagged] {
        &self.entries
    }

    /// Entries past [`MAX_FLAGGED`], counted but not kept.
    #[must_use]
    pub fn dropped(&self) -> u32 {
        self.dropped
    }

    /// Fold every entry and the dropped count into `fold`.
    pub fn fold(&self, fold: &mut Fnv64) {
        for entry in &self.entries {
            fold.write_u32(entry.tick);
            fold.write_u8(entry.slot);
            fold.write_u8(entry.flags);
        }
        fold.write_u32(self.dropped);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::BodyId;
    use engine_input::fighter::{ATTACK, BLOCK, DODGE};

    fn press(buttons: u8) -> BodyInput {
        BodyInput {
            tick: 7,
            fighter: BodyId::new(1).expect("slot"),
            mx: -100,
            mz: 100,
            facing: -18_000,
            buttons: Buttons(buttons),
        }
    }

    #[test]
    fn only_a_spear_press_without_the_spear_is_masked() {
        for buttons in [0, ATTACK, BLOCK | DODGE, ATTACK | BLOCK | DODGE] {
            for spear in [false, true] {
                assert_eq!(sanitize(press(buttons), spear), (press(buttons), 0));
            }
        }
        let (clean, flags) = sanitize(press(ATTACK | THROW_SPEAR), false);
        assert_eq!((clean, flags), (press(ATTACK), F_SPEAR));
        assert_eq!(
            sanitize(press(THROW_SPEAR), true),
            (press(THROW_SPEAR), 0),
            "a spear match keeps the throw"
        );
    }

    #[test]
    fn the_log_is_bounded_and_counts_the_rest() {
        let mut log = FlagLog::default();
        for tick in 0..MAX_FLAGGED as u32 + 3 {
            log.note(Flagged {
                tick,
                slot: 0,
                flags: F_SPEAR,
            });
        }
        assert_eq!(log.entries().len(), MAX_FLAGGED);
        assert_eq!(log.dropped(), 3);
        let mut a = Fnv64::new();
        let mut b = Fnv64::new();
        log.fold(&mut a);
        log.clone().fold(&mut b);
        assert_eq!(a.finish(), b.finish());
    }
}
