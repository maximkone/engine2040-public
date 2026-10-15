//! Rules time bars. Wall-clock measurement is test-only (the tick never reads a clock), so this
//! file is allowed the types the simulation clippy config bans.
#![expect(
    clippy::disallowed_types,
    clippy::disallowed_methods,
    reason = "timing gates measure wall time; nothing here runs inside a tick"
)]

use std::time::Instant;

use engine_rules::fixtures::ring_world;
use engine_rules::{Rules, World, MAX_FIGHTERS};

const SHIELD_BASH: &str = include_str!("../scripts/shield_bash.rhai");

fn installed() -> Rules {
    let mut rules = Rules::default();
    rules.set(SHIELD_BASH, 0).expect("shield bash installs");
    rules
}

fn p99_ms(samples: &mut [f64]) -> f64 {
    samples.sort_by(|a, b| a.partial_cmp(b).expect("no NaN"));
    samples[samples.len() * 99 / 100]
}

/// Time `ticks` rules ticks over 64 scripted fighters: p99 of the per-tick cost in ms.
fn p99_for_64(ticks: u32) -> f64 {
    let mut rules = installed();
    let worlds: Vec<World> = (0..64).map(|t| ring_world(MAX_FIGHTERS, t, 99)).collect();
    let mut samples = Vec::new();
    for tick in 0..ticks {
        let world = &worlds[(tick % 64) as usize];
        let started = Instant::now();
        let report = rules.tick(world).expect("tick");
        samples.push(started.elapsed().as_secs_f64() * 1_000.0);
        assert!(report.faults.is_empty());
    }
    p99_ms(&mut samples)
}

/// Mean ms per tick over `ticks` ticks of `source` on 64 actors, best (lowest) of `runs` runs:
/// the least noisy number on a shared PC.
fn best_mean_ms(source: &str, ticks: u32, runs: u32) -> f64 {
    let mut rules = Rules::default();
    rules.load_exact(source, 0).expect("loads");
    let worlds: Vec<World> = (0..64).map(|t| ring_world(MAX_FIGHTERS, t, 99)).collect();
    let mut best = f64::MAX;
    for _ in 0..runs {
        let started = Instant::now();
        for tick in 0..ticks {
            let _ = rules.tick(&worlds[(tick % 64) as usize]).expect("tick");
        }
        best = best.min(started.elapsed().as_secs_f64() * 1_000.0 / f64::from(ticks));
    }
    best
}

#[test]
#[ignore = "cost breakdown for tuning: cargo test --release -p engine-rules --test budget probe -- --ignored --nocapture"]
fn probe_cost_breakdown() {
    let variants = [
        ("empty script", "// nothing\n"),
        ("one variable", "if hp > 5000 { stun(0, 1); }"),
        ("two variables", "if !blocking || !attack { return; }"),
        ("shield bash", SHIELD_BASH),
    ];
    for (name, source) in variants {
        let ms = best_mean_ms(source, 1_000, 8);
        eprintln!(
            "{name:>14}: {:.4} ms per tick of 64 = {:.3} us per actor",
            ms,
            ms * 1_000.0 / 64.0
        );
    }
}

#[test]
fn rules_set_applies_in_at_most_one_second() {
    let mut rules = Rules::default();
    let started = Instant::now();
    rules.set(SHIELD_BASH, 0).expect("installs");
    let took = started.elapsed();
    eprintln!("rules.set (compile, vet, 5 smoke worlds): {took:?}");
    assert!(took.as_millis() <= 1_000, "rules.set took {took:?}");
}

#[test]
fn sixty_four_scripted_fighters_stay_cheap() {
    // The PC is shared with other jobs, so this guard is loose (2 ms, ten times the bar) and
    // takes the best of three runs; the strict gate is the ignored test below.
    let _ = p99_for_64(200);
    let best = (0..3).map(|_| p99_for_64(2_000)).fold(f64::MAX, f64::min);
    eprintln!("64 scripted fighters p99 per tick: {best:.4} ms");
    assert!(best < 2.0, "p99 {best} ms");
}

#[test]
#[ignore = "strict timing gate for a quiet PC: cargo test --release -p engine-rules --test budget -- --ignored"]
fn sixty_four_scripted_fighters_p99_under_0_2_ms() {
    let _ = p99_for_64(500);
    let best = (0..5).map(|_| p99_for_64(5_000)).fold(f64::MAX, f64::min);
    eprintln!("64 scripted fighters p99 per tick: {best:.4} ms");
    assert!(best <= 0.2, "p99 {best} ms");
}
