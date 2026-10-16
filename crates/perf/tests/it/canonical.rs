//! Canonical runs match the recorded baselines: ticks, hash and full-run
//! allocation budgets. Runs under the default parallel harness, so green
//! here already proves thread-local counting.

use engine_perf::baselines::{arena_baseline, baseline};
use engine_perf::budget::check_report;
use engine_perf::workloads::{
    AiDecide, ArenaTick, DestructionStep, PhysicsStep, SpineFullTick, SpineTransformBody,
};

#[test]
fn arena_tick_matches_baseline() {
    let report = ArenaTick::canonical().run_report();
    let recorded = arena_baseline();
    check_report(&report, &recorded).expect("arena_tick matches its baseline");
}

#[test]
fn physics_step_matches_baseline() {
    let report = PhysicsStep::canonical().run_report();
    let recorded = baseline(PhysicsStep::NAME).expect("physics_step baseline recorded");
    check_report(&report, &recorded).expect("physics_step matches its baseline");
}

#[test]
fn ai_decide_matches_baseline() {
    let report = AiDecide::canonical().run_report();
    let recorded = baseline(AiDecide::NAME).expect("ai_decide baseline recorded");
    check_report(&report, &recorded).expect("ai_decide matches its baseline");
}

#[test]
fn destruction_step_matches_baseline() {
    let report = DestructionStep::canonical().run_report();
    let recorded = baseline(DestructionStep::NAME).expect("destruction_step baseline recorded");
    check_report(&report, &recorded).expect("destruction_step matches its baseline");
}

#[test]
fn spine_transform_body_matches_baseline() {
    let report = SpineTransformBody::canonical().run_report();
    let recorded =
        baseline(SpineTransformBody::NAME).expect("spine_transform_body baseline recorded");
    check_report(&report, &recorded).expect("spine_transform_body matches its baseline");
}

#[test]
fn spine_full_tick_matches_baseline() {
    let report = SpineFullTick::canonical().run_report();
    let recorded = baseline(SpineFullTick::NAME).expect("spine_full_tick baseline recorded");
    check_report(&report, &recorded).expect("spine_full_tick matches its baseline");
}

#[test]
fn arena_tick_exercises_combat_and_breaks() {
    let workload = ArenaTick::canonical();
    let config = workload.config();
    let tape = engine_arena::FighterTape(Vec::new());
    let events: usize = engine_arena::run_events(&config, &tape)
        .iter()
        .map(|row| row.events.len())
        .sum();
    let breaks: usize = engine_arena::run_breaks(&config, &tape)
        .iter()
        .map(|row| row.events.len())
        .sum();
    assert!(events > 0, "canonical seed must land combat hits");
    assert!(breaks > 0, "sword hits must fracture the crate prop");
}
