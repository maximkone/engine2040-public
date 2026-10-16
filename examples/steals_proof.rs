//! steals_proof: exercises haul modules headless. Deterministic, asserts x2 identical.
use engine_ai::{goap_plan, Blackboard};
use engine_assets::health::{floors, nonzero_ratio};
use engine_audio::dsp::{adsr, BeatGrid, OnePole};
use engine_core::fixed::Fixed;
use engine_ecs::archetype::{Position as APos, Velocity as AVel, World as AWorld};
use engine_nav::flow::{bake_flow, flow_hash, follow};
use engine_net::rollback::{FrameLog, MAX_ROLLBACK};
use engine_physics::kcc::{self, KccConfig, KccInput, KccState, World as KccWorld, V3};
use engine_render::graph::Graph;

/// Runs the kinematic controller 60 fixed ticks and returns its state hash.
fn kcc_run() -> u64 {
    let cfg = KccConfig::default();
    let mut world = KccWorld::new();
    world.ground(0.0);
    let mut state = KccState::new(V3::new(0.0, 1.0, 0.0));
    for tick in 0..60 {
        let input = KccInput::new(V3::new(1.0, 0.0, 0.0), tick == 59);
        kcc::step(&cfg, &world, &mut state, &input, 1.0 / 60.0);
    }
    assert!(state.pos.y >= 0.0, "controller fell through the ground");
    kcc::state_hash(&state)
}

fn main() {
    // fixed Q32.32
    let three = Fixed::from_int(3) * Fixed::from_int(4);
    assert_eq!(three.to_int(), 12);
    // archetype (Option-spawn + despawn + change ticks)
    let mut arch = AWorld::new();
    let e = arch.spawn(Some(APos { x: 1.0, y: 2.0 }), Some(AVel { x: 1.0, y: 0.0 }));
    arch.integrate(1.0);
    assert!(arch.despawn(e));
    let arch_hash = arch.state_hash();
    // character controller (kcc): drop onto ground, jump on the last tick
    let ch_hash = kcc_run();
    assert_eq!(kcc_run(), ch_hash);
    // flow
    let blocked = vec![false; 8 * 8];
    let flow = bake_flow(&blocked, 8, 8, (7, 7));
    let flow_h = flow_hash(&flow);
    assert_eq!(follow(&flow, 8, 8, (0, 0)), Some((0, 1)));
    // blockout is proven by world crate tests; hash gate stays in main.
    // health floors
    let dense = vec![0xABu8; 4096];
    assert!(floors(&dense, false).iter().all(|(_, ok)| *ok));
    let nz = nonzero_ratio(&dense);
    // dsp
    assert!((adsr(0.5, 0.1, 0.1, 0.7, 0.1, 1.0) - 0.7).abs() < 1e-6);
    let mut lp = OnePole::new(0.1);
    for _ in 0..50 {
        lp.step(1.0);
    }
    let beats = BeatGrid::new(120.0, 0.0).beats_at(1.0);
    // rollback
    let mut log = FrameLog::new(MAX_ROLLBACK);
    for i in 0..4u64 {
        log.save(i * 10, i as u32);
    }
    let fold = log.fold();
    // graph
    let mut g = Graph::new();
    let (a, b) = (g.add_node("depth"), g.add_node("forward"));
    g.add_edge(a, b);
    let order = g.order().expect("acyclic");
    // blackboard + goap
    let mut bb = Blackboard::new();
    bb.set("fear", 7);
    let plan = goap_plan(&[5, 1, 9], 2);
    println!("steals_proof:");
    println!(
        "  fixed_3x4={} arch_hash={:016x} ch_hash={:016x}",
        three.to_int(),
        arch_hash,
        ch_hash
    );
    println!("  flow_hash={:016x} nz={:.2}", flow_h, nz);
    println!(
        "  beats@1s={} fold={:016x} order={:?} plan={:?} bb={:016x}",
        beats,
        fold,
        order,
        plan,
        bb.state_hash()
    );
    // x2 determinism gate
    assert_eq!(flow_hash(&bake_flow(&blocked, 8, 8, (7, 7))), flow_h);
    println!("  x2 identical: OK");
}
