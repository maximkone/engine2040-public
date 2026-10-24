//! lagcomp: bounded lag window for remote attacks on the dedicated server.
//!
//! Owner: net crate. Callers: [`crate::guard`] judges every attack press a
//! client sends ([`HeadlessServer::offer`](crate::dedicated::HeadlessServer::offer)),
//! and the played bout authors with [`HeadlessServer::hold_ticks`](crate::dedicated::HeadlessServer::hold_ticks).
//! The duel is rollback and needs no rewind. On the dedicated server an input
//! only counts when it reaches the server before its tick is authored, so the
//! server holds each tick open for the widest window of its connected slots:
//! a slot's window is its one-way trip in whole ticks plus one (the server
//! reads, then authors, inside one step), sized from the slot's round trip
//! and capped at [`MAX_WINDOW_TICKS`](crate::lagcomp::MAX_WINDOW_TICKS). A remote player's attack then lands on
//! the tick it was pressed, the same tick a player beside the server gets.
//! Every attack press (attack or spear throw) gets a [`HitReceipt`](crate::lagcomp::HitReceipt): fair
//! when it arrived inside the window while its tick was still open, refused
//! and counted otherwise. Whom the attack strikes stays the arena's sweep at
//! that tick; this module only decides whether the press counts on time.
//! Units: integer ticks at the arena rate, whole ms by integer math only.
//! Determinism: integer fields in arrival order, no clock; the same claims
//! give the same receipts and the same tape hash.
//! Errors: [`LagError`](crate::lagcomp::LagError); a late claim is a refused receipt, not an error.
//! Budget: [`adjudicate`](crate::lagcomp::adjudicate) O(1) with no heap; the tape is bounded at
//! [`MAX_HITS_PER_TAPE`](crate::lagcomp::MAX_HITS_PER_TAPE) and counts what it could not keep.

use engine_arena::TICK_HZ;
use engine_core::Fnv64;

use crate::dedicated::{ticks_to_ms, MAX_LATENCY_MS};

/// Hard bound on the lag window in fixed ticks (266 ms): a tick is never
/// held open longer, so an attack older than this never lands.
pub const MAX_WINDOW_TICKS: u8 = 16;
/// [`MAX_WINDOW_TICKS`] in whole milliseconds at the arena rate (pinned
/// against the tick bound by a test).
pub const MAX_WINDOW_MS: u64 = ticks_to_ms(16);
/// Window of a slot whose round trip is not known yet (83 ms): wide enough
/// for an 80 ms player's first presses before the first measurement.
pub const DEFAULT_WINDOW_TICKS: u8 = 5;
/// Receipts one tape keeps; later ones are counted in [`LagTape::dropped`].
pub const MAX_HITS_PER_TAPE: usize = 4096;

/// Lag-window failures: bounded diagnostics, never panics.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LagError {
    /// Window above [`MAX_WINDOW_TICKS`], culprit carried.
    WindowTooLarge(u8),
}

impl std::fmt::Display for LagError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match *self {
            Self::WindowTooLarge(ticks) => {
                write!(f, "window {ticks} ticks exceeds max {MAX_WINDOW_TICKS}")
            }
        }
    }
}

impl std::error::Error for LagError {}

/// Bounded lag window in fixed ticks.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct LagWindow {
    /// Ticks a press may arrive after the tick it was pressed for,
    /// `0..=MAX_WINDOW_TICKS`.
    pub ticks: u8,
}

impl LagWindow {
    /// The window of a slot whose round trip is not known yet.
    pub const DEFAULT: Self = Self {
        ticks: DEFAULT_WINDOW_TICKS,
    };

    /// A window of `ticks`; `None` above [`MAX_WINDOW_TICKS`].
    #[must_use]
    pub const fn new(ticks: u8) -> Option<Self> {
        if ticks <= MAX_WINDOW_TICKS {
            Some(Self { ticks })
        } else {
            None
        }
    }

    /// Window for a one-way trip of `one_way` ticks: the trip plus the one
    /// tick between reading a datagram and authoring, capped.
    #[must_use]
    pub fn for_one_way_ticks(one_way: u32) -> Self {
        let ticks = one_way.saturating_add(1).min(u32::from(MAX_WINDOW_TICKS));
        Self {
            ticks: u8::try_from(ticks).unwrap_or(MAX_WINDOW_TICKS),
        }
    }

    /// Window for a round trip of `rtt_ms`: half of it, rounded up to whole
    /// ticks, through [`Self::for_one_way_ticks`]. The claim is clamped to
    /// [`MAX_LATENCY_MS`] first; integer math only.
    #[must_use]
    pub fn for_rtt_ms(rtt_ms: u64) -> Self {
        let one_way_ms = rtt_ms.min(MAX_LATENCY_MS) / 2;
        let one_way = (one_way_ms * u64::from(TICK_HZ)).div_ceil(1_000);
        Self::for_one_way_ticks(u32::try_from(one_way).unwrap_or(u32::MAX))
    }

    /// Window in whole milliseconds (integer math only).
    #[must_use]
    pub fn window_ms(self) -> u64 {
        ticks_to_ms(u32::from(self.ticks))
    }

    /// Range-check the bound; anything above [`MAX_WINDOW_TICKS`] fails.
    ///
    /// # Errors
    ///
    /// [`LagError::WindowTooLarge`] above the bound.
    pub const fn validate(self) -> Result<(), LagError> {
        if self.ticks > MAX_WINDOW_TICKS {
            return Err(LagError::WindowTooLarge(self.ticks));
        }
        Ok(())
    }
}

/// One attack press as the server saw it arrive, in integer ticks.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct HitClaim {
    /// Attacker slot.
    pub attacker: u8,
    /// Tick the press was made for (the input's stamp).
    pub view_tick: u32,
    /// Server clock tick the datagram arrived on.
    pub arrival_tick: u32,
    /// Next tick the server had still to author when it arrived: the
    /// earliest tick a press can still land on.
    pub open_tick: u32,
}

impl HitClaim {
    /// Ticks between the press and its arrival; zero when the client runs
    /// ahead of the server clock.
    #[must_use]
    pub const fn lag(self) -> u32 {
        self.arrival_tick.saturating_sub(self.view_tick)
    }
}

/// One adjudicated attack press: the claim, the window it ran under and the
/// verdict, so the tape re-adjudicates to the same bytes.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct HitReceipt {
    /// What arrived.
    pub claim: HitClaim,
    /// Window the server held ticks open for when it arrived, in ticks.
    pub window_ticks: u8,
    /// The same window in whole milliseconds.
    pub window_ms: u64,
    /// True when the press lands on its own tick.
    pub accepted: bool,
}

impl HitReceipt {
    fn fold(self, fold: &mut Fnv64) {
        fold.write_u8(self.claim.attacker);
        fold.write_u32(self.claim.view_tick);
        fold.write_u32(self.claim.arrival_tick);
        fold.write_u32(self.claim.open_tick);
        fold.write_u8(self.window_ticks);
        fold.write_u64(self.window_ms);
        fold.write_bool(self.accepted);
    }
}

/// Adjudicate one press: fair when its tick was still open on arrival and
/// it arrived no more than `window` ticks after it. Integer ticks only.
///
/// # Errors
///
/// [`LagError::WindowTooLarge`] for a window above the bound.
pub fn adjudicate(claim: HitClaim, window: LagWindow) -> Result<HitReceipt, LagError> {
    window.validate()?;
    let accepted = claim.view_tick >= claim.open_tick && claim.lag() <= u32::from(window.ticks);
    Ok(HitReceipt {
        claim,
        window_ticks: window.ticks,
        window_ms: window.window_ms(),
        accepted,
    })
}

/// Bounded record of adjudicated presses in arrival order.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct LagTape {
    receipts: Vec<HitReceipt>,
    dropped: u32,
}

impl LagTape {
    /// An empty tape.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adjudicate `claim` under `window` and keep its receipt.
    ///
    /// # Errors
    ///
    /// [`LagError::WindowTooLarge`] for a window above the bound.
    pub fn record(&mut self, claim: HitClaim, window: LagWindow) -> Result<HitReceipt, LagError> {
        let receipt = adjudicate(claim, window)?;
        if self.receipts.len() < MAX_HITS_PER_TAPE {
            self.receipts.push(receipt);
        } else {
            self.dropped = self.dropped.saturating_add(1);
        }
        Ok(receipt)
    }

    /// Kept receipts in arrival order.
    #[must_use]
    pub fn receipts(&self) -> &[HitReceipt] {
        &self.receipts
    }

    /// Receipts past [`MAX_HITS_PER_TAPE`], counted but not kept.
    #[must_use]
    pub fn dropped(&self) -> u32 {
        self.dropped
    }

    /// Kept receipts that landed and that were refused.
    #[must_use]
    pub fn counts(&self) -> (u32, u32) {
        let fair = self.receipts.iter().filter(|r| r.accepted).count();
        let fair = u32::try_from(fair).unwrap_or(u32::MAX);
        let all = u32::try_from(self.receipts.len()).unwrap_or(u32::MAX);
        (fair, all - fair)
    }

    /// Re-adjudicate every receipt from its claim and window: true when the
    /// tape reproduces exactly.
    #[must_use]
    pub fn verify_replay(&self) -> bool {
        self.receipts.iter().all(|receipt| {
            LagWindow::new(receipt.window_ticks)
                .and_then(|window| adjudicate(receipt.claim, window).ok())
                .is_some_and(|fresh| fresh == *receipt)
        })
    }

    /// FNV-1a over the receipts in order plus the dropped count.
    #[must_use]
    pub fn hash(&self) -> u64 {
        let mut fold = Fnv64::new();
        for receipt in &self.receipts {
            receipt.fold(&mut fold);
        }
        fold.write_u32(self.dropped);
        fold.finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn claim(view_tick: u32, arrival_tick: u32, open_tick: u32) -> HitClaim {
        HitClaim {
            attacker: 1,
            view_tick,
            arrival_tick,
            open_tick,
        }
    }

    #[test]
    fn window_is_whole_ticks_capped_at_sixteen() {
        assert_eq!(MAX_WINDOW_MS, ticks_to_ms(u32::from(MAX_WINDOW_TICKS)));
        assert_eq!(MAX_WINDOW_MS, 266);
        assert_eq!(LagWindow::DEFAULT.window_ms(), 83);
        // 80 ms round trip: 40 ms one way is 2.4 ticks, rounded up to 3,
        // plus the read-then-author tick.
        assert_eq!(LagWindow::for_rtt_ms(80).ticks, 4);
        assert_eq!(LagWindow::for_rtt_ms(80).window_ms(), 66);
        assert_eq!(LagWindow::for_rtt_ms(0).ticks, 1);
        assert_eq!(LagWindow::for_one_way_ticks(3), LagWindow::for_rtt_ms(100));
        for rtt in [600, MAX_LATENCY_MS, u64::MAX] {
            assert_eq!(LagWindow::for_rtt_ms(rtt).ticks, MAX_WINDOW_TICKS);
        }
        assert_eq!(LagWindow::for_one_way_ticks(u32::MAX).ticks, 16);
        assert_eq!(LagWindow::new(17), None);
        assert_eq!(
            LagWindow { ticks: 17 }.validate(),
            Err(LagError::WindowTooLarge(17))
        );
        assert_eq!(
            LagError::WindowTooLarge(17).to_string(),
            "window 17 ticks exceeds max 16"
        );
    }

    #[test]
    fn an_80ms_press_inside_the_window_is_fair_and_one_tick_past_is_refused() {
        let window = LagWindow::for_rtt_ms(80);
        // Pressed for tick 100, arrived 3 ticks later, tick 100 still open.
        let fair = adjudicate(claim(100, 103, 99), window).expect("adjudicates");
        assert!(fair.accepted);
        assert_eq!((fair.window_ticks, fair.window_ms), (4, 66));
        // The window edge (lag 4) is fair, lag 5 is not.
        assert!(
            adjudicate(claim(100, 104, 100), window)
                .expect("edge")
                .accepted
        );
        assert!(
            !adjudicate(claim(100, 105, 100), window)
                .expect("past")
                .accepted
        );
        // A tick already authored never takes the press, whatever the lag.
        assert!(
            !adjudicate(claim(100, 101, 101), window)
                .expect("closed")
                .accepted
        );
        // A client ahead of the server clock has no lag.
        assert_eq!(claim(110, 100, 100).lag(), 0);
        assert!(
            adjudicate(claim(110, 100, 100), window)
                .expect("ahead")
                .accepted
        );
        assert_eq!(
            adjudicate(claim(0, 0, 0), LagWindow { ticks: 17 }),
            Err(LagError::WindowTooLarge(17))
        );
    }

    #[test]
    fn tape_replays_exactly_and_hashes_the_same_twice() {
        let record = || {
            let mut tape = LagTape::new();
            let window = LagWindow::for_rtt_ms(80);
            for view in 100..108 {
                tape.record(claim(view, view + 3, view - 1), window)
                    .expect("fair");
            }
            tape.record(claim(90, 108, 105), window).expect("late");
            tape
        };
        let (first, second) = (record(), record());
        assert_eq!(first.counts(), (8, 1));
        assert!(first.verify_replay());
        assert_eq!(first.hash(), second.hash());
        assert_ne!(first.hash(), LagTape::new().hash());
        // A forged verdict no longer re-adjudicates.
        let mut forged = first.clone();
        forged.receipts[8].accepted = true;
        assert!(!forged.verify_replay());
        assert_ne!(forged.hash(), first.hash());
    }

    #[test]
    fn a_full_tape_counts_what_it_could_not_keep() {
        let mut tape = LagTape::new();
        for view in 0..MAX_HITS_PER_TAPE as u32 + 2 {
            tape.record(claim(view, view, view), LagWindow::DEFAULT)
                .expect("record");
        }
        assert_eq!(tape.receipts().len(), MAX_HITS_PER_TAPE);
        assert_eq!(tape.dropped(), 2);
    }

    /// Grep gate: the window has production callers, outside any test
    /// module. The guard judges every press, the server sizes the hold, the
    /// played bout authors with it.
    #[test]
    fn the_lag_window_has_production_callers() {
        let production = |source: &'static str| source.split("#[cfg(test)]").next().unwrap_or("");
        let guard = production(include_str!("guard.rs"));
        assert!(guard.contains("self.hits.record(claim, hold)"));
        assert!(guard.contains("adjudicate(claim, hold)"));
        let server = production(include_str!("dedicated.rs"));
        assert!(server.contains("receipt.window().ticks"));
        assert!(server.contains(".admit(slot, open, arrival, hold"));
        let bout = production(include_str!("played.rs"));
        assert!(bout.contains("self.server.hold_ticks()"));
        assert!(bout.contains("self.server.offer(token, bytes, step)"));
    }
}
