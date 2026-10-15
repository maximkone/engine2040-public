//! nerve: small morale meter beside the feel survey.
//!
//! Owner: engine-combat. Consumer: presentation and harness only -- the fight
//! step never reads this; callers fold it beside feel preset rows and draw a
//! shaken or steady stance on their own clock. Units: four u8 levels (0..=255),
//! ratios as Fixed integer ratios. Determinism: u8 saturating math plus Fixed
//! integer ratios only; same meter plus same inputs always agree. Errors:
//! none -- every level saturates, never panics. Budget: const construction,
//! O(1) per call, no heap, no float.

use engine_core::fixed::Fixed;

/// Top of every nerve level.
pub const MAX_NERVE: u8 = 255;
/// Divisor for nerve ratios: one full meter as Fixed.
pub const NERVE_SCALE: i64 = 255;

/// One fighter nerve: resolve against dread with an anchor line.
///
/// resolve is the courage left, dread the fear piled on, anchor the support
/// that soaks dread first, break_line the effective dread that breaks the
/// stance. All four are plain u8 levels; every move saturates.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Nerve {
    /// Courage left, 0..=255.
    pub resolve: u8,
    /// Effective dread that breaks the stance, 0..=255.
    pub break_line: u8,
    /// Fear piled on, 0..=255.
    pub dread: u8,
    /// Support that soaks dread first, 0..=255.
    pub anchor: u8,
}

impl Nerve {
    /// Shipped stance: full-ish resolve, a high break line, no dread, no anchor.
    pub const STEADY: Self = Self {
        resolve: 200,
        break_line: 200,
        dread: 0,
        anchor: 0,
    };

    /// Explicit levels; every combination is valid, saturation keeps it so.
    pub const fn new(resolve: u8, break_line: u8, dread: u8, anchor: u8) -> Self {
        Self {
            resolve,
            break_line,
            dread,
            anchor,
        }
    }

    /// Dread past the anchor: dread minus anchor, floored at zero.
    pub const fn effective_dread(self) -> u8 {
        self.dread.saturating_sub(self.anchor)
    }

    /// True while the stance breaks: effective dread at or past the line.
    /// A zero line breaks at once, so callers that want no break set it high.
    pub const fn is_broken(self) -> bool {
        self.effective_dread() >= self.break_line
    }

    /// Pressure as a Fixed ratio: effective dread over a full meter.
    pub fn pressure(self) -> Fixed {
        Fixed::from_int(i64::from(self.effective_dread())) / Fixed::from_int(NERVE_SCALE)
    }

    /// Steadiness as a Fixed ratio: resolve over a full meter.
    pub fn steadiness(self) -> Fixed {
        Fixed::from_int(i64::from(self.resolve)) / Fixed::from_int(NERVE_SCALE)
    }

    /// Room left before the break as a Fixed ratio.
    pub fn hold_line(self) -> Fixed {
        let room = self.break_line.saturating_sub(self.effective_dread());
        Fixed::from_int(i64::from(room)) / Fixed::from_int(NERVE_SCALE)
    }

    /// Pile fear on; saturates at MAX_NERVE, never wraps.
    pub fn add_dread(&mut self, amount: u8) {
        self.dread = self.dread.saturating_add(amount);
    }

    /// Spend resolve; saturates at zero, never wraps.
    pub fn wear(&mut self, amount: u8) {
        self.resolve = self.resolve.saturating_sub(amount);
    }

    /// Rally: dread falls and resolve climbs by the same amount, both
    /// saturating -- dread floors at zero, resolve caps at MAX_NERVE.
    pub fn rally(&mut self, amount: u8) {
        self.dread = self.dread.saturating_sub(amount);
        self.resolve = self.resolve.saturating_add(amount);
    }

    /// Set the anchor support level.
    pub fn set_anchor(&mut self, anchor: u8) {
        self.anchor = anchor;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steady_holds_with_zero_pressure() {
        let nerve = Nerve::STEADY;
        assert!(!nerve.is_broken());
        assert_eq!(nerve.effective_dread(), 0);
        assert_eq!(nerve.pressure(), Fixed::ZERO);
        assert_eq!(
            nerve.hold_line(),
            Fixed::from_int(200) / Fixed::from_int(255)
        );
    }

    #[test]
    fn dread_saturates_and_breaks_past_the_line() {
        let mut nerve = Nerve::STEADY;
        nerve.add_dread(100);
        assert!(!nerve.is_broken());
        nerve.add_dread(100);
        assert!(nerve.is_broken());
        nerve.add_dread(200);
        assert_eq!(nerve.dread, MAX_NERVE);
        assert!(nerve.is_broken());
    }

    #[test]
    fn anchor_soaks_dread_first() {
        let mut nerve = Nerve::new(200, 100, 120, 0);
        assert!(nerve.is_broken());
        nerve.set_anchor(30);
        assert_eq!(nerve.effective_dread(), 90);
        assert!(!nerve.is_broken());
    }

    #[test]
    fn rally_and_wear_saturate() {
        let mut nerve = Nerve::new(250, 200, 10, 0);
        nerve.rally(20);
        assert_eq!(nerve.dread, 0);
        assert_eq!(nerve.resolve, MAX_NERVE);
        nerve.wear(255);
        assert_eq!(nerve.resolve, 0);
        assert_eq!(nerve.steadiness(), Fixed::ZERO);
    }

    #[test]
    fn fixed_ratios_span_zero_to_one() {
        let empty = Nerve::new(0, 255, 0, 0);
        assert_eq!(empty.pressure(), Fixed::ZERO);
        assert_eq!(empty.steadiness(), Fixed::ZERO);
        let full = Nerve::new(255, 255, 255, 0);
        assert_eq!(full.pressure(), Fixed::ONE);
        assert_eq!(full.steadiness(), Fixed::ONE);
        assert!(full.is_broken());
    }
}
