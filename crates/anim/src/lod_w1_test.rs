//! AM13 wave-1 accept: the LOD tier table keeps the 8 nearest fighters
//! at full rate on a 64-fighter tape, with the far cut disabling IK and
//! springs. Consumer: the AM18 crowd follow-up reads `assign_tiers` to
//! gate eval; this file pins the table contract first.

use super::lod::{assign_tiers, tier_for, LodError, Tier, FULL_RATE_NEAREST, MAX_FIGHTERS};
use engine_core::alloc_probe::measure;

/// Named `usize` to `f32` conversion for fixture indices (all tapes fit
/// in `u16`); saturates instead of wrapping so fixtures stay finite.
fn idx_f32(v: usize) -> f32 {
    f32::from(u16::try_from(v).unwrap_or(u16::MAX))
}

#[test]
fn lod_nearest_eight_full_rate_on_all_far_tape() {
    // All 64 read far (80 m plus index), so every base tier is Far;
    // only the nearest-eight rule may report Full.
    let n = MAX_FIGHTERS;
    let dists: Vec<f32> = (0..n).map(|i| 80.0 + idx_f32(i)).collect();
    let screens = vec![0.002f32; n];
    let mut tiers = vec![Tier::Far; n];
    assign_tiers(&dists, &screens, &mut tiers).expect("all-far tape assigns");
    let full: Vec<usize> = tiers
        .iter()
        .enumerate()
        .filter(|(_, &t)| t == Tier::Full)
        .map(|(i, _)| i)
        .collect();
    assert_eq!(
        full.len(),
        FULL_RATE_NEAREST,
        "exactly the 8 nearest stay full rate, got {full:?}"
    );
    assert_eq!(
        full,
        (0..FULL_RATE_NEAREST).collect::<Vec<usize>>(),
        "the 8 nearest indices win, got {full:?}"
    );
    for (i, &t) in tiers.iter().enumerate().skip(FULL_RATE_NEAREST) {
        assert_eq!(t, Tier::Far, "fighter {i} holds far");
    }
}

#[test]
fn lod_far_cut_disables_ik_and_springs() {
    let far = tier_for(100.0, 0.002).expect("far tape tiers");
    assert_eq!(far, Tier::Far, "100 m with a tiny screen reads far");
    assert!(!far.ik_on(), "far IK must be off");
    assert!(!far.springs_on(), "far springs must be off");
    let near = tier_for(5.0, 0.002).expect("near tape tiers");
    assert_eq!(near, Tier::Full);
    assert!(
        near.ik_on() && near.springs_on(),
        "near keeps IK and springs"
    );
    let mid = tier_for(20.0, 0.002).expect("mid tape tiers");
    assert_eq!(mid, Tier::Half);
    assert!(mid.ik_on() && mid.springs_on(), "mid keeps IK and springs");
}

#[test]
fn lod_screen_size_uptiers_a_far_fighter() {
    // Same 100 m distance: a big screen keeps full rate, a tiny one holds.
    let big = tier_for(100.0, 0.08).expect("big-screen tape tiers");
    assert_eq!(big, Tier::Full, "big screen uptiers to full");
    let small = tier_for(100.0, 0.002).expect("small-screen tape tiers");
    assert_eq!(small, Tier::Far, "small screen holds far");
    // Distance still tiers small screens: near, mid, quarter, far.
    assert_eq!(tier_for(5.0, 0.002).expect("near"), Tier::Full);
    assert_eq!(tier_for(20.0, 0.002).expect("mid"), Tier::Half);
    assert_eq!(tier_for(45.0, 0.002).expect("quarter"), Tier::Quarter);
    assert_eq!(tier_for(80.0, 0.002).expect("far"), Tier::Far);
}

#[test]
fn lod_strobe_periods_gate_updates() {
    assert!(
        Tier::Full.updates_this_tick(7, 3),
        "full updates every tick"
    );
    assert!(
        Tier::Half.updates_this_tick(4, 0),
        "half updates on even ticks"
    );
    assert!(!Tier::Half.updates_this_tick(5, 0), "half skips odd ticks");
    assert!(Tier::Half.updates_this_tick(5, 1), "half staggers by slot");
    assert!(Tier::Quarter.updates_this_tick(8, 0));
    assert!(
        !Tier::Quarter.updates_this_tick(9, 0),
        "quarter skips 3 of 4"
    );
    assert!(Tier::Far.updates_this_tick(16, 0));
    assert!(!Tier::Far.updates_this_tick(17, 0), "far skips 7 of 8");
    assert_eq!(Tier::Full.period_ticks(), 1);
    assert_eq!(Tier::Half.period_ticks(), 2);
    assert_eq!(Tier::Quarter.period_ticks(), 4);
    assert_eq!(Tier::Far.period_ticks(), 8);
}

#[test]
fn lod_rejects_bad_input() {
    assert_eq!(
        tier_for(f32::NAN, 0.01),
        Err(LodError::NonFinite("distance_m")),
        "NaN distance refuses"
    );
    assert_eq!(
        tier_for(-1.0, 0.01),
        Err(LodError::NegativeDistance(-1.0)),
        "negative distance refuses"
    );
    assert_eq!(
        tier_for(10.0, 2.0),
        Err(LodError::BadScreen(2.0)),
        "screen past 1 refuses"
    );
    let dists = vec![10.0f32; 4];
    let screens = vec![0.01f32; 3];
    let mut tiers = vec![Tier::Far; 4];
    assert_eq!(
        assign_tiers(&dists, &screens, &mut tiers),
        Err(LodError::LengthMismatch {
            expected: 4,
            got: 3
        }),
        "lane mismatch refuses"
    );
    let mut short = vec![Tier::Far; 2];
    assert_eq!(
        assign_tiers(&dists, &screens[..2.min(screens.len())], &mut short),
        Err(LodError::LengthMismatch {
            expected: 4,
            got: 2
        }),
        "short output refuses"
    );
    let wide = vec![80.0f32; MAX_FIGHTERS + 1];
    let wide_s = vec![0.002f32; MAX_FIGHTERS + 1];
    let mut wide_out = vec![Tier::Far; MAX_FIGHTERS + 1];
    assert_eq!(
        assign_tiers(&wide, &wide_s, &mut wide_out),
        Err(LodError::TooManyEntries(MAX_FIGHTERS + 1)),
        "65 fighters refuse"
    );
}

#[test]
fn lod_sixty_four_table_holds_budget_shape() {
    // Mixed tape: 8 near (full by distance), mid band, far ring.
    let n = MAX_FIGHTERS;
    let mut dists = Vec::with_capacity(n);
    for i in 0..n {
        dists.push(4.0 + idx_f32(i) * 2.0);
    }
    let screens = vec![0.002f32; n];
    let mut tiers = vec![Tier::Far; n];
    let (result, allocs) = measure(|| assign_tiers(&dists, &screens, &mut tiers));
    result.expect("mixed 64-fighter tape assigns");
    assert_eq!(allocs.calls, 0, "table assigns with no heap");
    let full_count = tiers.iter().filter(|&&t| t == Tier::Full).count();
    assert_eq!(
        full_count, FULL_RATE_NEAREST,
        "table keeps exactly 8 full-rate, got {full_count}"
    );
    // The far ring holds with IK and springs off: the budget mechanism.
    let far_count = tiers.iter().filter(|&&t| t == Tier::Far).count();
    assert!(far_count > 0, "far ring exists on a 64-fighter tape");
    for (i, &t) in tiers.iter().enumerate() {
        if t == Tier::Far {
            assert!(
                !t.ik_on() && !t.springs_on(),
                "far fighter {i} must cut IK and springs"
            );
        }
    }
    // Same tape twice assigns identically.
    let mut again = vec![Tier::Far; n];
    assign_tiers(&dists, &screens, &mut again).expect("reassign holds");
    assert_eq!(tiers, again, "table repeats byte-identically");
}

#[test]
fn lod_alloc_probe_sees_a_real_allocation() {
    let (_, allocs) = measure(|| vec![0u8; 64]);
    assert_eq!(allocs.calls, 1, "probe counts a real allocation");
}
