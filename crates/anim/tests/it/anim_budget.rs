//! Animation budget allocator pin: who drops, who never drops, how long a
//! rung holds, and that the same tape gives the same tiers twice.
//!
//! Owner: engine anim. Units: milliseconds per frame, meters, screen fraction
//! zero to one, frames. Determinism: no clock, the cost is fed in as a number.
//! Errors: typed budget errors, bad input refuses.

use engine_anim::anim_budget::{
    significance, AnimActor, AnimBudget, BudgetConfig, BudgetError, FIGHT_BONUS, MAX_BUDGET_ACTORS,
};
use engine_anim::lod::Tier;

const UNIT: f32 = 0.14;

fn cfg() -> BudgetConfig {
    BudgetConfig {
        budget_ms: 1.0,
        hold_frames: 8,
        headroom: 0.8,
        emergency: 2.5,
        smoothing: 1.0,
        initial_unit_ms: UNIT,
    }
}

/// Eight actors, index 0 nearest and biggest, index 7 farthest and smallest.
fn eight() -> Vec<AnimActor> {
    (0..8u8)
        .map(|i| AnimActor::new(5.0 + 6.0 * f32::from(i), 0.05 - 0.005 * f32::from(i)))
        .collect()
}

/// The pose cost the assignment would measure, given the true unit cost.
fn cost(actors: &[AnimActor], tiers: &[Tier], unit_ms: f32) -> f32 {
    actors
        .iter()
        .zip(tiers)
        .map(|(a, t)| a.weight * unit_ms / t.period_ticks() as f32)
        .sum()
}

#[test]
fn over_budget_least_significant_drop_first() {
    let actors = eight();
    let mut budget = AnimBudget::new(cfg()).expect("config");
    let mut tiers = vec![Tier::Full; actors.len()];
    // 8 actors at 0.14 ms each is 1.12 ms against a 1.0 ms budget: two drop.
    let report = budget.step(&actors, 0.0, &mut tiers).expect("step");
    assert_eq!(report.changed, 2, "{tiers:?}");
    for (i, tier) in tiers.iter().enumerate() {
        let want = if i >= 6 { Tier::Half } else { Tier::Full };
        assert_eq!(*tier, want, "actor {i}: {tiers:?}");
    }
    assert!(!report.over_budget);
    assert!(report.predicted_ms <= 1.0);
    // Heavier load walks the ladder in order: half first, then quarter.
    let mut heavy = AnimBudget::new(BudgetConfig {
        initial_unit_ms: 0.4,
        ..cfg()
    })
    .expect("config");
    heavy.step(&actors, 0.0, &mut tiers).expect("step");
    let rung = |t: &Tier| t.period_ticks();
    for pair in tiers.windows(2) {
        assert!(rung(&pair[0]) <= rung(&pair[1]), "nearer never coarser: {tiers:?}");
    }
    assert!(tiers.contains(&Tier::Quarter), "{tiers:?}");
}

#[test]
fn the_player_and_the_opponent_never_drop() {
    // The worst-placed pair: farthest, smallest, so the lowest significance
    // without the pin. The budget is a tenth of what full rate costs.
    let mut actors: Vec<AnimActor> = (0..20u8)
        .map(|i| AnimActor::new(10.0 + f32::from(i), 0.02))
        .collect();
    actors[18].player = true;
    actors[19].opponent = true;
    let mut budget = AnimBudget::new(BudgetConfig {
        budget_ms: 0.3,
        ..cfg()
    })
    .expect("config");
    let mut tiers = vec![Tier::Full; actors.len()];
    let mut measured = 0.0;
    for frame in 0..60 {
        budget.step(&actors, measured, &mut tiers).expect("step");
        assert_eq!(tiers[18], Tier::Full, "player at frame {frame}");
        assert_eq!(tiers[19], Tier::Full, "opponent at frame {frame}");
        measured = cost(&actors, &tiers, UNIT);
    }
    // The rest paid for it: every other actor sits on the far rung.
    assert!(tiers[..18].iter().all(|t| *t == Tier::Far), "{tiers:?}");
    // A pinned actor beats any unpinned score.
    let loud = AnimActor {
        fighting: true,
        ..AnimActor::new(0.0, 1.0)
    };
    assert!(significance(&actors[18]) > significance(&loud));
}

#[test]
fn a_rung_holds_for_the_hold_frames() {
    let actors = eight();
    let mut budget = AnimBudget::new(cfg()).expect("config");
    let mut tiers = vec![Tier::Full; actors.len()];
    budget.step(&actors, 0.0, &mut tiers).expect("step");
    assert_eq!(tiers[7], Tier::Half);
    // The scene turns nearly free at once; the dropped pair must still wait.
    let cheap = 0.001;
    for frame in 1..8 {
        let measured = cost(&actors, &tiers, cheap);
        let report = budget.step(&actors, measured, &mut tiers).expect("step");
        assert_eq!(report.changed, 0, "frame {frame} moved a rung early");
        assert_eq!(tiers[7], Tier::Half, "frame {frame}");
        assert_eq!(tiers[6], Tier::Half, "frame {frame}");
    }
    let measured = cost(&actors, &tiers, cheap);
    let report = budget.step(&actors, measured, &mut tiers).expect("step");
    assert_eq!(report.changed, 2, "both return on the eighth frame");
    assert!(tiers.iter().all(|t| *t == Tier::Full), "{tiers:?}");
}

#[test]
fn the_dead_band_never_flips_a_rung() {
    // A load that sits between "fits" and "over" must not make a rung
    // chatter: after the first plan settles, nothing moves for 200 frames.
    let actors = eight();
    let mut budget = AnimBudget::new(cfg()).expect("config");
    let mut tiers = vec![Tier::Full; actors.len()];
    let mut measured = 0.0;
    let mut moves = 0;
    for _ in 0..200 {
        let report = budget.step(&actors, measured, &mut tiers).expect("step");
        moves += report.changed;
        measured = cost(&actors, &tiers, UNIT);
    }
    assert_eq!(moves, 2, "two drops on frame 0 and then stillness");
}

#[test]
fn under_budget_everyone_returns_to_full_rate() {
    let actors = eight();
    let mut budget = AnimBudget::new(BudgetConfig {
        initial_unit_ms: 0.6,
        ..cfg()
    })
    .expect("config");
    let mut tiers = vec![Tier::Full; actors.len()];
    let mut measured = 0.0;
    // Heavy scene first: most of the crowd is throttled.
    for _ in 0..20 {
        budget.step(&actors, measured, &mut tiers).expect("step");
        measured = cost(&actors, &tiers, 0.6);
    }
    assert!(tiers.iter().filter(|t| **t != Tier::Full).count() >= 5, "{tiers:?}");
    // The scene gets cheap: everyone is back at full rate within the hold.
    for _ in 0..40 {
        budget.step(&actors, measured, &mut tiers).expect("step");
        measured = cost(&actors, &tiers, 0.02);
    }
    assert!(tiers.iter().all(|t| *t == Tier::Full), "{tiers:?}");
}

#[test]
fn geometry_base_is_a_floor_the_budget_never_beats() {
    let mut actors = eight();
    actors[3].base = Tier::Quarter;
    let mut budget = AnimBudget::new(BudgetConfig {
        initial_unit_ms: 0.0001,
        ..cfg()
    })
    .expect("config");
    let mut tiers = vec![Tier::Full; actors.len()];
    for _ in 0..30 {
        budget.step(&actors, 0.0, &mut tiers).expect("step");
    }
    assert_eq!(tiers[3], Tier::Quarter, "a free budget keeps the base tier");
    assert_eq!(tiers[0], Tier::Full);
}

#[test]
fn emergency_skips_the_hold_going_coarser() {
    // A heavy first frame moves every actor, so every hold is fresh.
    let actors = eight();
    let start = |measured_unit: f32| {
        let mut budget = AnimBudget::new(BudgetConfig {
            initial_unit_ms: 0.5,
            ..cfg()
        })
        .expect("config");
        let mut tiers = vec![Tier::Full; actors.len()];
        let first = budget.step(&actors, 0.0, &mut tiers).expect("step");
        assert_eq!(first.changed, 8);
        assert!(tiers.iter().all(|t| *t == Tier::Quarter), "{tiers:?}");
        let measured = cost(&actors, &tiers, measured_unit);
        let report = budget.step(&actors, measured, &mut tiers).expect("step");
        (report, tiers)
    };
    // 1.6 ms against 1.0 ms is over budget but inside the 2.5x emergency line:
    // the hold keeps every rung.
    let (mild, tiers) = start(0.8);
    assert!(!mild.emergency);
    assert_eq!(mild.changed, 0);
    assert!(tiers.iter().all(|t| *t == Tier::Quarter), "{tiers:?}");
    // 3.2 ms is past the line: the drop is immediate.
    let (spike, tiers) = start(1.6);
    assert!(spike.emergency);
    assert_eq!(spike.changed, 8);
    assert!(tiers.iter().all(|t| *t == Tier::Far), "{tiers:?}");
}

#[test]
fn significance_orders_screen_distance_and_fight() {
    let near = AnimActor::new(5.0, 0.04);
    let far = AnimActor::new(55.0, 0.04);
    assert!(significance(&near) > significance(&far));
    let big = AnimActor::new(30.0, 0.04);
    let small = AnimActor::new(30.0, 0.005);
    assert!(significance(&big) > significance(&small));
    let fighting = AnimActor {
        fighting: true,
        ..small
    };
    assert!((significance(&fighting) - significance(&small) - FIGHT_BONUS).abs() < 1e-6);
    // A distant fighter outranks an idle one at the same spot.
    assert!(significance(&fighting) > significance(&AnimActor::new(30.0, 0.005)));
}

#[test]
fn the_same_tape_gives_the_same_tiers_twice() {
    fn run() -> Vec<u8> {
        let mut actors: Vec<AnimActor> = (0..64u32)
            .map(|i| AnimActor::new(3.0 + (i * 37 % 90) as f32, 0.002 + (i * 13 % 40) as f32 * 0.001))
            .collect();
        actors[0].player = true;
        actors[1].opponent = true;
        actors[2].fighting = true;
        let mut budget = AnimBudget::new(BudgetConfig {
            budget_ms: 0.8,
            ..cfg()
        })
        .expect("config");
        let mut tiers = vec![Tier::Full; actors.len()];
        let mut trace = Vec::new();
        let mut measured = 0.0;
        for frame in 0..240u32 {
            // The camera walks, so distances change and rungs have to react.
            for (i, a) in actors.iter_mut().enumerate().skip(2) {
                a.distance_m = 3.0 + ((i as u32 * 37 + frame) % 90) as f32;
            }
            let unit = if frame < 120 { 0.05 } else { 0.01 };
            budget.step(&actors, measured, &mut tiers).expect("step");
            measured = cost(&actors, &tiers, unit);
            trace.extend(tiers.iter().map(|t| t.period_ticks() as u8));
        }
        trace
    }
    let first = run();
    assert_eq!(first, run());
    assert!(first.iter().any(|&p| p > 1), "the tape must throttle someone");
}

#[test]
fn bad_input_refuses() {
    let mut budget = AnimBudget::new(cfg()).expect("config");
    let mut one = [Tier::Full];
    let nan = [AnimActor::new(f32::NAN, 0.1)];
    assert_eq!(
        budget.step(&nan, 0.0, &mut one),
        Err(BudgetError::NonFinite("distance_m"))
    );
    let neg = [AnimActor::new(-1.0, 0.1)];
    assert_eq!(
        budget.step(&neg, 0.0, &mut one),
        Err(BudgetError::NegativeDistance(-1.0))
    );
    let screen = [AnimActor::new(1.0, 1.5)];
    assert_eq!(budget.step(&screen, 0.0, &mut one), Err(BudgetError::BadScreen(1.5)));
    let ok = [AnimActor::new(1.0, 0.1)];
    assert_eq!(
        budget.step(&ok, -1.0, &mut one),
        Err(BudgetError::NegativeMeasured(-1.0))
    );
    let mut two = [Tier::Full; 2];
    assert_eq!(
        budget.step(&ok, 0.0, &mut two),
        Err(BudgetError::LengthMismatch { expected: 1, got: 2 })
    );
    let many = vec![AnimActor::new(1.0, 0.1); MAX_BUDGET_ACTORS + 1];
    let mut out = vec![Tier::Full; many.len()];
    assert_eq!(
        budget.step(&many, 0.0, &mut out),
        Err(BudgetError::TooManyActors(MAX_BUDGET_ACTORS + 1))
    );
    assert_eq!(
        AnimBudget::new(BudgetConfig {
            budget_ms: 0.0,
            ..cfg()
        })
        .err(),
        Some(BudgetError::BadConfig("budget_ms"))
    );
}
