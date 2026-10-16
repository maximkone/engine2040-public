//! resim: resimulation-window ledger beside the delay knob.
//!
//! Owner: `engine-net`; consumer: predicting clients that hold local inputs
//! back by the round trip minus [`crate::scale::RESIM_CAP_TICKS`] so late authority lands inside
//! the rollback window. Units: whole ticks (`u32`). Determinism:
//! integer-only counters (`+`, compares, `max`); no float, no wall clock,
//! no hashing, no allocation. Errors: over-window depths count loudly in
//! `over_cap` (never panic, never silent).
//!
//! One [`ResimWindow`] ledgers a single session: [`ResimWindow::with_rtt`]
//! opens it with the MP24 hold-back for the simulated round trip, and each
//! applied resimulation folds through [`ResimWindow::observe`]. The gate
//! reads [`ResimWindow::all_inside`]: every observed depth at or under
//! [`crate::scale::RESIM_CAP_TICKS`].

/// Ledger of one session's applied resimulation depths against the window.
///
/// Opened with the MP24 hold-back for the simulated round trip, then fed
/// one [`Self::observe`] call per applied resimulation. Plain counters:
/// no heap, no clock, no hash input.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResimWindow {
    delay: u32,
    applied: u32,
    deepest: u32,
    over_cap: u32,
}

impl ResimWindow {
    /// Open a window holding predictions back for `rtt_ticks` (MP24 rule).
    #[must_use]
    pub fn with_rtt(rtt_ticks: u32) -> Self {
        Self {
            delay: rtt_ticks.saturating_sub(crate::scale::RESIM_CAP_TICKS),
            applied: 0,
            deepest: 0,
            over_cap: 0,
        }
    }

    /// Fold one applied resimulation depth in ticks: `deepest` tracks the
    /// max, depths past the window count in `over_cap`, all saturating.
    pub fn observe(&mut self, depth_ticks: u32) {
        self.applied = self.applied.saturating_add(1);
        self.deepest = self.deepest.max(depth_ticks);
        self.over_cap = self
            .over_cap
            .saturating_add(u32::from(depth_ticks > crate::scale::RESIM_CAP_TICKS));
    }

    /// Hold-back in ticks this window opened with.
    #[must_use]
    pub fn delay(&self) -> u32 {
        self.delay
    }

    /// Applied resimulations folded so far.
    #[must_use]
    pub fn applied(&self) -> u32 {
        self.applied
    }

    /// Deepest applied resimulation in ticks.
    #[must_use]
    pub fn deepest(&self) -> u32 {
        self.deepest
    }

    /// Applied resimulations deeper than the window.
    #[must_use]
    pub fn over_cap(&self) -> u32 {
        self.over_cap
    }

    /// True when every observed depth landed inside the window.
    #[must_use]
    pub fn all_inside(&self) -> bool {
        self.over_cap == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scale::RESIM_CAP_TICKS;

    #[test]
    fn window_opens_with_the_rtt_hold_back() {
        let window = ResimWindow::with_rtt(19);
        assert_eq!(window.delay(), 11, "WAN deepest 19 holds back 11");
        assert_eq!(
            (window.applied(), window.deepest(), window.over_cap()),
            (0, 0, 0),
            "a fresh window has folded nothing"
        );
        assert!(
            window.all_inside(),
            "an unobserved window is trivially inside"
        );
        let lan = ResimWindow::with_rtt(5);
        assert_eq!(lan.delay(), 0, "LAN round trip stays undelayed");
    }

    #[test]
    fn window_folds_depths_and_names_the_deepest() {
        let mut window = ResimWindow::with_rtt(19);
        for depth in [0, 3, 8, 5] {
            window.observe(depth);
        }
        assert_eq!(window.applied(), 4);
        assert_eq!(window.deepest(), 8);
        assert_eq!(window.over_cap(), 0);
        assert!(window.all_inside());
    }

    #[test]
    fn window_counts_over_cap_loudly() {
        // Failure shape: a 19-tick resim without hold-back lands outside the
        // window and must count, never panic or vanish.
        let mut window = ResimWindow::with_rtt(5);
        window.observe(19);
        assert_eq!(window.deepest(), 19);
        assert_eq!(window.over_cap(), 1);
        assert!(!window.all_inside());
        window.observe(RESIM_CAP_TICKS);
        assert_eq!(window.over_cap(), 1, "depths on the cap stay inside");
        assert_eq!(window.deepest(), 19);
    }

    #[test]
    fn window_handles_tick_edges() {
        let mut window = ResimWindow::with_rtt(0);
        assert_eq!(window.delay(), 0);
        window.observe(u32::MAX);
        assert_eq!(window.deepest(), u32::MAX);
        assert_eq!(window.over_cap(), 1);
        assert!(!window.all_inside());
    }
}
