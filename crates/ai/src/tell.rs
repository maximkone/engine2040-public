//! tell: the readable wind-up floor and the shipped duel rows that read it.
//!
//! Owner: engine-ai. Units: ticks at 60 Hz. Data: the shipped ladder rows
//! mirror `content/design/enemies.rows`; the game test `design_enemy` checks
//! the file against them. Determinism: constants only, no state, no floats.

use crate::depth::TierRow;

/// The shortest readable tell a tier may ship, in ticks: 30 ticks is 500 ms
/// at 60 Hz, the heuristic floor of the one measured readability study
/// (theseus 900621, n=30: median read 800 ms, IQR 525-900, floor 500 ms).
/// The designer-file fence and the shipped rows both read it.
pub const MIN_TELL_TICKS: u32 = 30;

/// Shipped duel tiers matching the designer rows file.
#[rustfmt::skip]
pub const SHIPPED_TIERS: [TierRow; 3] = [
    TierRow { tier: 1, hp_permille: 950, tell_ticks: MIN_TELL_TICKS, stagger_permille: 400, counter: 1 },
    TierRow { tier: 2, hp_permille: 1_000, tell_ticks: MIN_TELL_TICKS, stagger_permille: 500, counter: 2 },
    TierRow { tier: 3, hp_permille: 1_050, tell_ticks: MIN_TELL_TICKS, stagger_permille: 600, counter: 1 },
];

/// Final-foe primus row: skill, not stats. Health sits inside the narrow
/// `900..=1200` band, the tell matches the ladder's readable floor
/// ([`MIN_TELL_TICKS`], 30 ticks = 500 ms), and the phase-1 answer is parry
/// (`1`), the same school as the T3 elite. Notice is faster (reaction 1) and
/// nerve is bolder (700, 750 in phase 2). The designer file carries this row
/// as `tier_4`; a file without one still parses, defaulting the fourth slot
/// to this const.
#[rustfmt::skip]
pub const PRIMUS_TIER: TierRow = TierRow {
    tier: 4, hp_permille: 1100, tell_ticks: MIN_TELL_TICKS, stagger_permille: 700, counter: 1,
};
