//! presentation: the numbers the look layers ride, in one validated row (SW24).
//!
//! Owner: engine-combat. Readers: the game crate (`camera_feed` impact camera,
//! `fighter_look_flash` hit flash); nothing on the tick path reads it, so the
//! golden `d57b5c8ae3096614` never sees a byte. Units: milli-degrees, permille,
//! ticks at 60 Hz. Integer only, so the sim crate holds no floats.
use crate::feel::{range, FeelError};

/// Largest milli-degree budget a rotation rail may carry (60 deg): a feel shake
/// past a sixth of a turn would read as the camera losing the bout.
pub const PRESENTATION_MAX_MILLI_DEG: u16 = 60_000;

/// Presentation feel table (SW24): every number the look layers ride - the
/// impact camera rails, the hit flash, the knockdown slow-mo factor and the
/// play-time pitch spread - in one validated row. Same row, same presentation
/// twice.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct PresentationFeel {
    /// Impact camera tilt budget for yaw and pitch, milli-degrees.
    pub shake_tilt_milli_deg: u16,
    /// Impact camera roll budget, milli-degrees.
    pub shake_roll_milli_deg: u16,
    /// QL8 shared rotation cap the split budgets sit inside, milli-degrees.
    pub shake_cap_milli_deg: u16,
    /// Blinks a landed hit spends over its flash window.
    pub flash_blinks: u8,
    /// Each later blink as a share of the one before, permille.
    pub flash_decay_permille: u16,
    /// Overbright exposure gain at the flash peak, permille (`2000` = 2.0x).
    pub flash_boost_permille: u16,
    /// Knockdown slow-mo factor, permille of real time (`300` = a third speed).
    pub slowmo_permille: u16,
    /// Play-time pitch spread either side of unity, permille (`80` = +-8%).
    pub pitch_variation_permille: u16,
}

impl PresentationFeel {
    /// The shipped presentation row: the numbers the look layers ship today.
    #[must_use]
    pub const fn v1() -> Self {
        Self {
            shake_tilt_milli_deg: 1_600,
            shake_roll_milli_deg: 900,
            shake_cap_milli_deg: 5_729,
            flash_blinks: 2,
            flash_decay_permille: 250,
            flash_boost_permille: 2_000,
            slowmo_permille: 300,
            pitch_variation_permille: 80,
        }
    }

    /// Range-check this row; first failure wins, deterministically.
    ///
    /// # Errors
    ///
    /// [`FeelError::BadValue`] naming the first field outside its fence.
    pub fn validate(&self) -> Result<(), FeelError> {
        range(
            "shake_tilt_milli_deg",
            u32::from(self.shake_tilt_milli_deg),
            1,
            u32::from(PRESENTATION_MAX_MILLI_DEG),
        )?;
        range(
            "shake_roll_milli_deg",
            u32::from(self.shake_roll_milli_deg),
            1,
            u32::from(self.shake_tilt_milli_deg),
        )?;
        range(
            "shake_cap_milli_deg",
            u32::from(self.shake_cap_milli_deg),
            u32::from(self.shake_tilt_milli_deg),
            u32::from(PRESENTATION_MAX_MILLI_DEG),
        )?;
        range("flash_blinks", u32::from(self.flash_blinks), 1, 4)?;
        range(
            "flash_decay_permille",
            u32::from(self.flash_decay_permille),
            1,
            1_000,
        )?;
        range(
            "flash_boost_permille",
            u32::from(self.flash_boost_permille),
            1_000,
            4_000,
        )?;
        range("slowmo_permille", u32::from(self.slowmo_permille), 1, 1_000)?;
        range(
            "pitch_variation_permille",
            u32::from(self.pitch_variation_permille),
            0,
            200,
        )?;
        Ok(())
    }
}

/// The shipped presentation row.
pub const PRESENTATION_V1: PresentationFeel = PresentationFeel::v1();

#[cfg(test)]
mod tests {
    use super::*;
    use crate::feel::FEEL_V1;

    /// SW24: one validated presentation row holds the camera rails, the flash
    /// blink and boost, the slow-mo factor and the pitch spread.
    #[test]
    fn sw24_one_presentation_row_holds_every_feel_number() {
        let row = PRESENTATION_V1;
        assert_eq!(
            row.validate(),
            Ok(()),
            "the shipped row is inside its fence"
        );
        // The rails nest: roll inside tilt inside the QL8 cap.
        assert!(row.shake_roll_milli_deg <= row.shake_tilt_milli_deg);
        assert!(row.shake_tilt_milli_deg <= row.shake_cap_milli_deg);
        let refused = |row: PresentationFeel| -> Option<&'static str> {
            match row.validate() {
                Err(FeelError::BadValue { field, .. }) => Some(field),
                _ => None,
            }
        };
        let mut bad = row;
        bad.shake_tilt_milli_deg = 0;
        assert_eq!(refused(bad), Some("shake_tilt_milli_deg"));
        let mut bad = row;
        bad.shake_roll_milli_deg = row.shake_tilt_milli_deg + 1;
        assert_eq!(refused(bad), Some("shake_roll_milli_deg"));
        let mut bad = row;
        bad.shake_cap_milli_deg = row.shake_tilt_milli_deg - 1;
        assert_eq!(refused(bad), Some("shake_cap_milli_deg"));
        let mut bad = row;
        bad.flash_blinks = 0;
        assert_eq!(refused(bad), Some("flash_blinks"));
        let mut bad = row;
        bad.flash_decay_permille = 1_001;
        assert_eq!(refused(bad), Some("flash_decay_permille"));
        let mut bad = row;
        bad.flash_boost_permille = 999;
        assert_eq!(refused(bad), Some("flash_boost_permille"));
        let mut bad = row;
        bad.slowmo_permille = 0;
        assert_eq!(refused(bad), Some("slowmo_permille"));
        let mut bad = row;
        bad.pitch_variation_permille = 201;
        assert_eq!(refused(bad), Some("pitch_variation_permille"));
        // The feel row beside it is untouched, and the PQ1 pin still reads 33 ms
        // for the two buffered ticks.
        assert_eq!(FEEL_V1.validate(), Ok(()));
        assert_eq!(crate::design::latency_ms(2), 33);
    }
}
