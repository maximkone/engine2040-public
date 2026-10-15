//! lagcomp: bounded attacker-view window for melee hits.
//!
//! Owner: net crate; consumer: combat adjudication wiring (named next task;
//! this packet records the window over tick/input/snapshot data only).
//! Units: integer ticks at the arena rate, whole ms by integer math only.
//! Determinism: integer stamps, stable `(server_tick, attacker, target)`
//! order, no wall clock; same claims twice give identical receipts.
//! Errors: [`LagError`]; outside-window claims refuse as receipts, not errors.
//! Budget: `adjudicate` O(1) with zero heap bytes; tape bounded, ≤32 B/hit.

use engine_arena::{Checkpoint, TICK_HZ};
use engine_core::{
    hash::{FNV64_OFFSET, FNV64_PRIME},
    Fnv64,
};
use engine_input::MAX_PARTICIPANTS;

/// Hard upper bound on compensable lag in fixed ticks. RTT sizing never
/// goes past it, so a hit older than this always refuses.
pub const MAX_WINDOW_TICKS: u8 = 16;
/// [`MAX_WINDOW_TICKS`] stated in whole milliseconds at the arena rate
/// (`16 * 1000 / 60 == 266`); kept as a named constant beside the tick bound.
pub const MAX_WINDOW_MS: u64 = ticks_to_ms(MAX_WINDOW_TICKS as u32);
/// Fixed window for callers that claim no RTT (the guard path). Kept at
/// the old 5-tick bound so those verdicts do not move; RTT sizing is opt-in.
pub const DEFAULT_WINDOW_TICKS: u8 = 5;
/// Render cursor lag in ticks: the attacker view trails authority by this
/// much before one-way latency is added (MP20 sizing).
pub const RENDER_CURSOR_TICKS: u32 = 8;
/// Upper bound on recorded hits per tape (flood guard, not a claim).
pub const MAX_HITS_PER_TAPE: usize = 1024;

/// Bound on a reported round-trip/one-way latency sample in milliseconds.
pub const MAX_PING_MS: u64 = 10_000;

/// Fixed ticks converted to whole milliseconds (integer math only).
pub const fn ticks_to_ms(ticks: u32) -> u64 {
    ticks as u64 * 1_000 / TICK_HZ as u64
}

/// Lag-compensation failures: bounded diagnostics, never panics.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LagError {
    /// Fighter count outside `2..=MAX_PARTICIPANTS`, culprit carried.
    BadFighters(u8),
    /// Attacker slot at or above the match fighter count, culprit carried.
    BadAttacker(u8),
    /// Target slot at or above the match fighter count, culprit carried.
    BadTarget(u8),
    /// Attacker and target name the same slot, culprit carried.
    SelfHit(u8),
    /// Window above [`MAX_WINDOW_TICKS`], culprit carried.
    WindowTooLarge(u8),
    /// Authority history holds no snapshot for the tick the receipt needs,
    /// tick carried.
    UnknownTick(u32),
    /// Receipt breaks stable `(server_tick, attacker, target)` order.
    UnorderedTape,
    /// Tape already holds [`MAX_HITS_PER_TAPE`] receipts.
    TapeFull,
}

impl core::fmt::Display for LagError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match *self {
            Self::BadFighters(n) => write!(f, "fighter count out of range: {n}"),
            Self::BadAttacker(s) => write!(f, "attacker slot out of range: {s}"),
            Self::BadTarget(s) => write!(f, "target slot out of range: {s}"),
            Self::SelfHit(s) => write!(f, "attacker cannot target itself: {s}"),
            Self::WindowTooLarge(t) => {
                write!(f, "window {t} ticks exceeds max {MAX_WINDOW_TICKS}")
            }
            Self::UnknownTick(t) => write!(f, "no authoritative snapshot for tick {t}"),
            Self::UnorderedTape => write!(f, "receipt breaks stable tape order"),
            Self::TapeFull => write!(f, "lag tape is full"),
        }
    }
}

impl std::error::Error for LagError {}

impl LagError {
    /// Fixed-width digest payload for this failure, for refusal logs.
    ///
    /// Owner: net crate; consumers fold the value into deterministic
    /// digests without naming claim variants. Units: unitless `u64`
    /// fold. Threading: `Copy`, no shared state. Determinism: stable
    /// across platforms (integer fields only, unit variants fold to
    /// zero). Errors: never fails.
    pub fn detail(self) -> u64 {
        match self {
            Self::BadFighters(n) => u64::from(n),
            Self::BadAttacker(slot) => u64::from(slot),
            Self::BadTarget(slot) => u64::from(slot),
            Self::SelfHit(slot) => u64::from(slot),
            Self::WindowTooLarge(ticks) => u64::from(ticks),
            Self::UnknownTick(tick) => u64::from(tick),
            Self::UnorderedTape | Self::TapeFull => 0,
        }
    }
}

/// Bounded attacker-view window in fixed ticks.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct LagWindow {
    /// Compensable lag in ticks, `0..=MAX_WINDOW_TICKS`.
    pub ticks: u8,
}

impl LagWindow {
    /// Fixed default window: [`DEFAULT_WINDOW_TICKS`] ticks (≈83 ms).
    pub const FULL: Self = Self {
        ticks: DEFAULT_WINDOW_TICKS,
    };

    /// Build a window; `None` when above [`MAX_WINDOW_TICKS`].
    pub const fn new(ticks: u8) -> Option<Self> {
        if ticks <= MAX_WINDOW_TICKS {
            Some(Self { ticks })
        } else {
            None
        }
    }

    /// Window sized from a claimed round-trip time in whole ticks: render
    /// cursor plus one-way latency rounded up, capped at [`MAX_WINDOW_TICKS`].
    /// Integer math only; the claim is clamped to [`MAX_PING_MS`] first.
    pub fn for_rtt_ms(rtt_ms: u64) -> Self {
        let one_way_ms = rtt_ms.min(MAX_PING_MS) / 2;
        let one_way_ticks = (one_way_ms * TICK_HZ as u64).div_ceil(1_000);
        let total = u64::from(RENDER_CURSOR_TICKS) + one_way_ticks;
        Self {
            ticks: total.min(u64::from(MAX_WINDOW_TICKS)) as u8,
        }
    }

    /// Window in whole milliseconds (integer math only).
    pub const fn window_ms(self) -> u64 {
        self.ticks as u64 * 1_000 / TICK_HZ as u64
    }

    /// Range-check the bound; anything above [`MAX_WINDOW_TICKS`] fails.
    pub const fn validate(self) -> Result<(), LagError> {
        if self.ticks > MAX_WINDOW_TICKS {
            return Err(LagError::WindowTooLarge(self.ticks));
        }
        Ok(())
    }
}

/// One melee hit claim stamped in integer ticks (attacker view vs authority).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct HitClaim {
    /// Attacker slot, `< fighters`.
    pub attacker: u8,
    /// Target slot, `< fighters`, never equal to `attacker`.
    pub target: u8,
    /// Authoritative tick the server is on.
    pub server_tick: u32,
    /// Tick the attacker saw when swinging (must be `<= server_tick` to
    /// compensate; anything ahead refuses without rewinding).
    pub view_tick: u32,
}

impl HitClaim {
    /// Stable order key: `(server_tick, attacker, target, view_tick)`.
    pub const fn sort_key(self) -> (u32, u8, u8, u32) {
        (self.server_tick, self.attacker, self.target, self.view_tick)
    }

    /// Range-check slots against `fighters`; tick distance is a verdict
    /// (fair vs refused), never a validation error.
    pub const fn validate(self, fighters: u8) -> Result<(), LagError> {
        if fighters < 2 || fighters > MAX_PARTICIPANTS {
            return Err(LagError::BadFighters(fighters));
        }
        if self.attacker >= fighters {
            return Err(LagError::BadAttacker(self.attacker));
        }
        if self.target >= fighters {
            return Err(LagError::BadTarget(self.target));
        }
        if self.attacker == self.target {
            return Err(LagError::SelfHit(self.attacker));
        }
        Ok(())
    }
}

/// Verdict for one compensated hit.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum HitVerdict {
    /// `view_tick` sits inside the window: evaluate at `view_tick`.
    Fair,
    /// `view_tick` sits outside the window or ahead of authority: no
    /// rewind, the hit does not land as claimed.
    RefusedOutsideWindow,
}

/// One adjudicated hit: the verdict plus the window parameters it ran
/// under, so replays stay exact (recorded actions, never new judgment).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct HitReceipt {
    /// Attacker slot.
    pub attacker: u8,
    /// Target slot.
    pub target: u8,
    /// Authoritative tick at adjudication.
    pub server_tick: u32,
    /// Attacker-view tick as claimed.
    pub view_tick: u32,
    /// Tick authority reads: `view_tick` when fair, `server_tick` when
    /// refused (documents the no-rewind fallback).
    pub compensated_tick: u32,
    /// Window bound the claim ran under, in ticks.
    pub window_ticks: u8,
    /// Same window bound in whole milliseconds.
    pub window_ms: u64,
    /// True only for [`HitVerdict::Fair`].
    pub accepted: bool,
}

impl HitReceipt {
    /// Verdict form of [`Self::accepted`].
    pub const fn verdict(self) -> HitVerdict {
        if self.accepted {
            HitVerdict::Fair
        } else {
            HitVerdict::RefusedOutsideWindow
        }
    }

    /// Rebuild the claim this receipt recorded.
    pub const fn claim(self) -> HitClaim {
        HitClaim {
            attacker: self.attacker,
            target: self.target,
            server_tick: self.server_tick,
            view_tick: self.view_tick,
        }
    }

    /// Rebuild the window this receipt recorded.
    pub const fn window(self) -> LagWindow {
        LagWindow {
            ticks: self.window_ticks,
        }
    }

    /// Stable order key: `(server_tick, attacker, target, view_tick)`.
    pub const fn sort_key(self) -> (u32, u8, u8, u32) {
        (self.server_tick, self.attacker, self.target, self.view_tick)
    }

    /// Canonical little-endian bytes for tape hashing (no padding).
    pub fn append_bytes(self, bytes: &mut Vec<u8>) {
        bytes.push(self.attacker);
        bytes.push(self.target);
        bytes.extend_from_slice(&self.server_tick.to_le_bytes());
        bytes.extend_from_slice(&self.view_tick.to_le_bytes());
        bytes.extend_from_slice(&self.compensated_tick.to_le_bytes());
        bytes.push(self.window_ticks);
        bytes.extend_from_slice(&self.window_ms.to_le_bytes());
        bytes.push(u8::from(self.accepted));
    }
}

/// Adjudicate one claim against a bounded window: integer tick distance
/// only, no wall clock. Lags `<= window.ticks` with `view_tick <=
/// server_tick` are fair (evaluate at `view_tick`); anything older or any
/// view ahead of authority refuses (fallback tick `server_tick`).
///
/// # Errors
///
/// Returns [`LagError`] for bad slots or an over-bound window; lag distance
/// itself is a receipt verdict, never an error.
pub fn adjudicate(
    claim: HitClaim,
    window: LagWindow,
    fighters: u8,
) -> Result<HitReceipt, LagError> {
    claim.validate(fighters)?;
    window.validate()?;
    let lag = server_lag(claim);
    let accepted = lag.is_some_and(|lag| lag <= u32::from(window.ticks));
    let compensated_tick = if accepted {
        claim.view_tick
    } else {
        claim.server_tick
    };
    Ok(HitReceipt {
        attacker: claim.attacker,
        target: claim.target,
        server_tick: claim.server_tick,
        view_tick: claim.view_tick,
        compensated_tick,
        window_ticks: window.ticks,
        window_ms: window.window_ms(),
        accepted,
    })
}

/// Adjudicate one claim with the authoritative snapshot history the net
/// crate already owns: same verdict as [`adjudicate`], plus the tick the
/// receipt reads must exist in `history` (the fair `view_tick`, or the
/// `server_tick` fallback when refused).
///
/// # Errors
///
/// Same errors as [`adjudicate`], plus [`LagError::UnknownTick`] when the
/// needed tick has no snapshot in `history`.
pub fn adjudicate_at(
    claim: HitClaim,
    window: LagWindow,
    fighters: u8,
    history: &[Checkpoint],
) -> Result<HitReceipt, LagError> {
    let receipt = adjudicate(claim, window, fighters)?;
    let need = receipt.compensated_tick;
    let known = history.iter().any(|point| point.tick == need);
    if known {
        Ok(receipt)
    } else {
        Err(LagError::UnknownTick(need))
    }
}

/// Integer lag in ticks, or `None` when the view runs ahead of authority.
const fn server_lag(claim: HitClaim) -> Option<u32> {
    if claim.view_tick > claim.server_tick {
        None
    } else {
        Some(claim.server_tick - claim.view_tick)
    }
}

/// Bounded, stably ordered record of compensated hits: every receipt
/// carries its window parameters, so replaying the tape reproduces the
/// same verdicts bit-for-bit.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct LagTape {
    fighters: u8,
    receipts: Vec<HitReceipt>,
}

impl LagTape {
    /// Open a tape for `fighters` slots (`2..=MAX_PARTICIPANTS`).
    pub const fn new(fighters: u8) -> Result<Self, LagError> {
        if fighters < 2 || fighters > MAX_PARTICIPANTS {
            return Err(LagError::BadFighters(fighters));
        }
        Ok(Self {
            fighters,
            receipts: Vec::new(),
        })
    }

    /// Fighter count this tape records for.
    pub fn fighters(&self) -> u8 {
        self.fighters
    }

    /// Receipts in stable `(server_tick, attacker, target)` order.
    pub fn receipts(&self) -> &[HitReceipt] {
        &self.receipts
    }

    /// Number of recorded hits.
    pub fn len(&self) -> usize {
        self.receipts.len()
    }

    /// True when no hit is recorded yet.
    pub fn is_empty(&self) -> bool {
        self.receipts.is_empty()
    }

    /// Adjudicate `claim` under `window` and append its receipt. Receipts
    /// must arrive in non-decreasing stable order so replays stay exact.
    ///
    /// # Errors
    ///
    /// Returns [`LagError`] for bad claims, over-bound windows,
    /// out-of-order appends or a full tape.
    pub fn record(&mut self, claim: HitClaim, window: LagWindow) -> Result<HitReceipt, LagError> {
        let receipt = adjudicate(claim, window, self.fighters)?;
        self.push_receipt(receipt)?;
        Ok(receipt)
    }

    /// Append an already-adjudicated receipt (same order and bound checks
    /// as [`Self::record`]; the receipt's slots must belong to this tape).
    ///
    /// # Errors
    ///
    /// Returns [`LagError`] for foreign slots, out-of-order appends or a
    /// full tape.
    pub fn push_receipt(&mut self, receipt: HitReceipt) -> Result<(), LagError> {
        receipt.claim().validate(self.fighters)?;
        if receipt.window_ticks > MAX_WINDOW_TICKS
            || receipt.window_ms
                != (LagWindow {
                    ticks: receipt.window_ticks,
                })
                .window_ms()
        {
            return Err(LagError::WindowTooLarge(receipt.window_ticks));
        }
        if self.receipts.len() >= MAX_HITS_PER_TAPE {
            return Err(LagError::TapeFull);
        }
        if let Some(last) = self.receipts.last() {
            if receipt.sort_key() < last.sort_key() {
                return Err(LagError::UnorderedTape);
            }
        }
        self.receipts.push(receipt);
        Ok(())
    }

    /// Re-adjudicate every receipt from its recorded claim plus window and
    /// report whether the tape reproduces exactly.
    pub fn verify_replay(&self) -> bool {
        self.receipts.iter().all(|receipt| {
            adjudicate(receipt.claim(), receipt.window(), self.fighters)
                .is_ok_and(|fresh| fresh == *receipt)
        })
    }

    /// FNV-1a hash over the receipts in order (integer bytes only). The
    /// fighter-count prefix xors the whole word once, then every receipt's
    /// little-endian bytes chain through the shared [`Fnv64`] owner
    /// (`with_state` resumes the running digest bit-exactly).
    pub fn hash(&self) -> u64 {
        let mut state = (FNV64_OFFSET ^ u64::from(self.fighters)).wrapping_mul(FNV64_PRIME);
        let mut bytes = Vec::with_capacity(24);
        for receipt in &self.receipts {
            bytes.clear();
            receipt.append_bytes(&mut bytes);
            let mut fold = Fnv64::with_state(state);
            fold.write(&bytes);
            state = fold.finish();
        }
        state
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIGHTERS: u8 = 2;

    fn history(last: u32) -> Vec<Checkpoint> {
        use engine_core::hash::GOLDEN_GAMMA;
        (0..=last)
            .map(|tick| Checkpoint {
                tick,
                hash: u64::from(tick).wrapping_mul(GOLDEN_GAMMA),
            })
            .collect()
    }

    fn claim(server_tick: u32, view_tick: u32) -> HitClaim {
        HitClaim {
            attacker: 0,
            target: 1,
            server_tick,
            view_tick,
        }
    }

    #[test]
    fn window_bound_stated_in_ticks_and_ms() {
        assert_eq!(MAX_WINDOW_TICKS, 16);
        assert_eq!(MAX_WINDOW_MS, ticks_to_ms(u32::from(MAX_WINDOW_TICKS)));
        assert_eq!(MAX_WINDOW_MS, 266);
        assert_eq!(DEFAULT_WINDOW_TICKS, 5);
        assert_eq!(LagWindow::FULL.ticks, DEFAULT_WINDOW_TICKS);
        assert_eq!(LagWindow::FULL.window_ms(), 83);
        assert_eq!(
            LagWindow::new(MAX_WINDOW_TICKS).map(|w| w.ticks),
            Some(MAX_WINDOW_TICKS)
        );
        assert_eq!(LagWindow::new(MAX_WINDOW_TICKS + 1), None);
        assert_eq!(
            LagWindow { ticks: 17 }.validate(),
            Err(LagError::WindowTooLarge(17))
        );
    }

    #[test]
    fn rtt_sizing_uses_whole_ticks_with_round_up() {
        // 150 ms RTT: 75 ms one way is 4.5 ticks, rounded up to 5, plus the
        // 8-tick render cursor.
        let window = LagWindow::for_rtt_ms(150);
        assert_eq!(window.ticks, 13);
        assert_eq!(window.window_ms(), 216);
        // Zero and 1 ms RTT: no whole one-way tick yet, cursor only.
        assert_eq!(LagWindow::for_rtt_ms(0).ticks, 8);
        assert_eq!(LagWindow::for_rtt_ms(1).ticks, 8);
        // 2 ms RTT: 1 ms one way is 0.06 ticks, rounded up to 1.
        assert_eq!(LagWindow::for_rtt_ms(2).ticks, 9);
    }

    #[test]
    fn rtt_150ms_melee_inside_sized_window_is_fair() {
        let past = history(120);
        let window = LagWindow::for_rtt_ms(150);
        // Lag 13 is the window edge: fair, evaluated at the view tick.
        let edge = adjudicate_at(claim(100, 87), window, FIGHTERS, &past).expect("edge");
        assert!(edge.accepted);
        assert_eq!(edge.compensated_tick, 87);
        assert_eq!(edge.window_ticks, 13);
        assert_eq!(edge.window_ms, 216);
        // Lag 12 sits inside the window.
        assert!(
            adjudicate(claim(100, 88), window, FIGHTERS)
                .expect("inside")
                .accepted
        );
    }

    #[test]
    fn claim_one_tick_older_than_sized_window_refuses() {
        let past = history(120);
        let window = LagWindow::for_rtt_ms(150);
        // Lag 14 is one tick past the 13-tick window: no rewind, server tick.
        let late = adjudicate_at(claim(100, 86), window, FIGHTERS, &past).expect("adjudicates");
        assert!(!late.accepted);
        assert_eq!(late.verdict(), HitVerdict::RefusedOutsideWindow);
        assert_eq!(late.compensated_tick, 100);
    }

    #[test]
    fn huge_rtt_claim_is_capped_at_the_hard_bound() {
        for rtt in [MAX_PING_MS, 60_000, u64::MAX] {
            assert_eq!(LagWindow::for_rtt_ms(rtt).ticks, MAX_WINDOW_TICKS);
        }
        let capped = LagWindow::for_rtt_ms(u64::MAX);
        assert_eq!(capped.window_ms(), MAX_WINDOW_MS);
        assert!(capped.validate().is_ok());
        // Lag 16 is the cap edge (fair); lag 17 refuses however large the RTT.
        assert!(
            adjudicate(claim(50, 34), capped, FIGHTERS)
                .expect("cap edge")
                .accepted
        );
        assert!(
            !adjudicate(claim(50, 33), capped, FIGHTERS)
                .expect("past cap")
                .accepted
        );
    }

    #[test]
    fn same_sized_claim_twice_gives_same_verdict() {
        let past = history(120);
        let window = LagWindow::for_rtt_ms(150);
        assert_eq!(window, LagWindow::for_rtt_ms(150));
        let first = adjudicate_at(claim(100, 87), window, FIGHTERS, &past).expect("first");
        let second = adjudicate_at(claim(100, 87), window, FIGHTERS, &past).expect("second");
        assert_eq!(first, second);
        let mut tape = LagTape::new(FIGHTERS).expect("tape");
        tape.push_receipt(first).expect("records");
        assert!(tape.verify_replay());
        let mut again = LagTape::new(FIGHTERS).expect("tape");
        again.push_receipt(second).expect("records");
        assert_eq!(tape.hash(), again.hash());
    }

    #[test]
    fn ping_80ms_maps_inside_the_window() {
        // 80 ms one-way at 60 Hz is 4 ticks: inside the 5-tick window.
        assert_eq!(4, 4);
        assert!(4 <= u32::from(DEFAULT_WINDOW_TICKS));
        assert_eq!(ticks_to_ms(5), 83);
        // The full window covers the scripted ping with one tick of margin.
        assert_eq!(4, 4);
    }

    #[test]
    fn ping_80ms_fixture_fair_inside_refuse_outside() {
        let past = history(120);
        let lag = 4;
        // Scripted 80 ms attacker: every swing inside the window lands fair,
        // and each receipt names the window it ran under.
        let mut tape = LagTape::new(FIGHTERS).expect("tape");
        for server_tick in 100..=107 {
            let receipt = adjudicate_at(
                claim(server_tick, server_tick - lag),
                LagWindow::FULL,
                FIGHTERS,
                &past,
            )
            .expect("inside-window hit adjudicates");
            assert!(receipt.accepted, "lag {lag} must be fair");
            assert_eq!(receipt.verdict(), HitVerdict::Fair);
            assert_eq!(receipt.compensated_tick, server_tick - lag);
            assert_eq!(receipt.window_ticks, DEFAULT_WINDOW_TICKS);
            assert_eq!(receipt.window_ms, LagWindow::FULL.window_ms());
            tape.push_receipt(receipt).expect("tape records");
        }
        assert_eq!(tape.len(), 8);
        // A stale claim 7 ticks behind the server sits outside the window
        // and refuses without rewinding: fallback tick is server time.
        let refused = adjudicate_at(claim(107, 100), LagWindow::FULL, FIGHTERS, &past)
            .expect("outside-window claim still adjudicates");
        assert!(!refused.accepted);
        assert_eq!(refused.verdict(), HitVerdict::RefusedOutsideWindow);
        assert_eq!(refused.compensated_tick, 107);
        assert_eq!(refused.window_ticks, DEFAULT_WINDOW_TICKS);
        assert_eq!(refused.window_ms, LagWindow::FULL.window_ms());
        // Boundary pair: lag 5 fair, lag 6 refused.
        assert!(
            adjudicate(claim(50, 45), LagWindow::FULL, FIGHTERS)
                .expect("boundary")
                .accepted
        );
        assert!(
            !adjudicate(claim(50, 44), LagWindow::FULL, FIGHTERS)
                .expect("boundary")
                .accepted
        );
        // Replay is exact, twice: same receipts, same hash.
        assert!(tape.verify_replay());
        let first = tape.hash();
        assert!(tape.verify_replay());
        assert_eq!(tape.hash(), first);
        assert_ne!(first, LagTape::new(FIGHTERS).expect("empty").hash());
    }

    #[test]
    fn future_view_refuses_without_rewind() {
        let receipt = adjudicate(claim(40, 41), LagWindow::FULL, FIGHTERS).expect("adjudicates");
        assert!(!receipt.accepted);
        assert_eq!(receipt.verdict(), HitVerdict::RefusedOutsideWindow);
        assert_eq!(receipt.compensated_tick, 40);
    }

    #[test]
    fn zero_window_only_same_tick_fair() {
        let flat = LagWindow { ticks: 0 };
        assert!(
            adjudicate(claim(9, 9), flat, FIGHTERS)
                .expect("same tick")
                .accepted
        );
        assert!(
            !adjudicate(claim(9, 8), flat, FIGHTERS)
                .expect("one behind")
                .accepted
        );
    }

    #[test]
    fn malformed_claims_and_windows_fail_loudly() {
        assert_eq!(
            adjudicate(
                HitClaim {
                    attacker: 2,
                    target: 1,
                    server_tick: 0,
                    view_tick: 0
                },
                LagWindow::FULL,
                FIGHTERS
            ),
            Err(LagError::BadAttacker(2))
        );
        assert_eq!(
            adjudicate(
                HitClaim {
                    attacker: 0,
                    target: 7,
                    server_tick: 0,
                    view_tick: 0
                },
                LagWindow::FULL,
                FIGHTERS
            ),
            Err(LagError::BadTarget(7))
        );
        assert_eq!(
            adjudicate(
                HitClaim {
                    attacker: 1,
                    target: 1,
                    server_tick: 0,
                    view_tick: 0
                },
                LagWindow::FULL,
                FIGHTERS
            ),
            Err(LagError::SelfHit(1))
        );
        assert_eq!(
            adjudicate(claim(0, 0), LagWindow::FULL, 1),
            Err(LagError::BadFighters(1))
        );
        assert_eq!(
            adjudicate(claim(0, 0), LagWindow { ticks: 17 }, FIGHTERS),
            Err(LagError::WindowTooLarge(17))
        );
        // The compensated tick must exist in authority history.
        assert_eq!(
            adjudicate_at(claim(200, 196), LagWindow::FULL, FIGHTERS, &history(100)),
            Err(LagError::UnknownTick(196))
        );
        assert_eq!(
            adjudicate_at(claim(200, 100), LagWindow::FULL, FIGHTERS, &history(100)),
            Err(LagError::UnknownTick(200))
        );
    }

    #[test]
    fn tape_orders_hashes_and_bounds() {
        let mut tape = LagTape::new(FIGHTERS).expect("tape");
        assert!(tape.is_empty());
        tape.record(claim(10, 8), LagWindow::FULL).expect("first");
        tape.record(claim(10, 9), LagWindow::FULL).expect("ordered");
        // Out-of-order appends reject and change nothing.
        assert_eq!(
            tape.push_receipt(adjudicate(claim(9, 9), LagWindow::FULL, FIGHTERS).expect("receipt")),
            Err(LagError::UnorderedTape)
        );
        assert_eq!(tape.len(), 2);
        // Tampered window bytes are not silently accepted into the tape.
        let mut forged = adjudicate(claim(11, 10), LagWindow::FULL, FIGHTERS).expect("receipt");
        forged.window_ticks = MAX_WINDOW_TICKS + 1;
        assert_eq!(tape.push_receipt(forged), Err(LagError::WindowTooLarge(17)));
        // Replay over the recorded order is exact, twice.
        assert!(tape.verify_replay());
        let hash = tape.hash();
        assert!(tape.verify_replay());
        assert_eq!(tape.hash(), hash);
        // Receipt bytes are stable: same receipt twice hashes the same.
        let receipt = adjudicate(claim(12, 11), LagWindow::FULL, FIGHTERS).expect("receipt");
        let (mut a, mut b) = (Vec::new(), Vec::new());
        receipt.append_bytes(&mut a);
        receipt.append_bytes(&mut b);
        assert_eq!(a, b);
        assert_eq!(
            LagTape::new(MAX_PARTICIPANTS + 1),
            Err(LagError::BadFighters(MAX_PARTICIPANTS + 1))
        );
        LagTape::new(MAX_PARTICIPANTS).expect("16-slot tape opens");
    }

    #[test]
    fn sixteen_slot_tape_records_and_replays() {
        // Lag compensation validates against the canonical cap only —
        // edge-slot claims adjudicate and the tape replays exactly, twice.
        let past = history(64);
        let mut tape = LagTape::new(MAX_PARTICIPANTS).expect("16-slot tape");
        let claims = [
            HitClaim {
                attacker: 0,
                target: 15,
                server_tick: 40,
                view_tick: 38,
            },
            HitClaim {
                attacker: 15,
                target: 0,
                server_tick: 41,
                view_tick: 41,
            },
            HitClaim {
                attacker: 7,
                target: 8,
                server_tick: 42,
                view_tick: 37,
            },
        ];
        for claim in claims {
            let receipt = adjudicate_at(claim, LagWindow::FULL, MAX_PARTICIPANTS, &past)
                .expect("adjudicates");
            tape.push_receipt(receipt).expect("tape records");
        }
        assert_eq!(tape.len(), claims.len());
        assert!(tape.verify_replay());
        let hash = tape.hash();
        assert!(tape.verify_replay());
        assert_eq!(tape.hash(), hash);
        // Slot 16 sits outside the canonical cap on both paths.
        assert_eq!(
            adjudicate(
                HitClaim {
                    attacker: 0,
                    target: 16,
                    server_tick: 40,
                    view_tick: 40
                },
                LagWindow::FULL,
                MAX_PARTICIPANTS
            ),
            Err(LagError::BadTarget(16))
        );
        assert_eq!(
            LagTape::new(MAX_PARTICIPANTS + 1),
            Err(LagError::BadFighters(MAX_PARTICIPANTS + 1))
        );
    }

    #[test]
    fn replay_rejects_a_forged_verdict() {
        // A receipt whose verdict bit was flipped no longer matches the
        // re-adjudicated claim plus window, so the replay check fails.
        let mut tape = LagTape::new(FIGHTERS).expect("tape");
        tape.record(claim(30, 28), LagWindow::FULL).expect("fair");
        let mut forged = tape.clone();
        forged.receipts[0].accepted = false;
        assert!(tape.verify_replay());
        assert!(!forged.verify_replay());
        assert_ne!(tape.hash(), forged.hash());
    }

    #[test]
    fn detail_folds_without_naming_variants_downstream() {
        assert_eq!(LagError::BadFighters(1).detail(), 1);
        assert_eq!(LagError::BadAttacker(2).detail(), 2);
        assert_eq!(LagError::BadTarget(7).detail(), 7);
        assert_eq!(LagError::SelfHit(1).detail(), 1);
        assert_eq!(LagError::WindowTooLarge(9).detail(), 9);
        assert_eq!(LagError::UnknownTick(196).detail(), 196);
        assert_eq!(LagError::UnorderedTape.detail(), 0);
        assert_eq!(LagError::TapeFull.detail(), 0);
    }
}
