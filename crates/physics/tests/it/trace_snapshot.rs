// Attribution: snapshot shape adapted from mitsuhiko/insta (Apache-2.0),
// https://github.com/mitsuhiko/insta pattern tests/test_basic.rs
// assert_snapshot shape. LICENCE verified as Apache-2.0. Inline snapshots
// only, no new dependency. Why: pin physics tick-trace text against drift.
#![forbid(unsafe_code)]

use engine_physics::rigid::FIXED_DT;
use engine_physics::{Body, Space};

fn assert_snapshot(actual: &str, expected: &str) {
    let norm = |s: &str| s.replace("\r\n", "\n");
    assert_eq!(norm(actual), norm(expected), "tick-trace snapshot mismatch");
}

fn fmt_body(tick: u32, id: usize, b: &Body) -> String {
    format!(
        "tick={} id={} x={:.4} y={:.4} vx={:.4} vy={:.4}",
        tick, id, b.x, b.y, b.vx, b.vy
    )
}

fn redact_hash(line: &str) -> String {
    match line.find("hash=") {
        None => line.to_string(),
        Some(i) => format!("{}[HASH]\n", &line[..i + 5]),
    }
}

#[test]
fn rest_single_three_ticks() {
    let mut s = Space::new();
    s.add(Body::new(0.0, 0.5, 0.0, 0.0, 0.5, 0.5));
    let mut trace = String::new();
    for tick in 1..=3 {
        s.step(FIXED_DT);
        trace.push_str(&fmt_body(tick, 0, &s.bodies[0]));
        trace.push('\n');
    }
    assert_snapshot(&trace, "tick=1 id=0 x=0.0000 y=0.5000 vx=0.0000 vy=0.0000\ntick=2 id=0 x=0.0000 y=0.5000 vx=0.0000 vy=0.0000\ntick=3 id=0 x=0.0000 y=0.5000 vx=0.0000 vy=0.0000\n");
}

#[test]
fn rest_pair_two_ticks() {
    let mut s = Space::new();
    s.add(Body::new(-1.0, 0.5, 0.0, 0.0, 0.5, 0.5));
    s.add(Body::new(1.0, 0.5, 0.0, 0.0, 0.5, 0.5));
    let mut trace = String::new();
    for tick in 1..=2 {
        s.step(FIXED_DT);
        for (id, b) in s.bodies.iter().enumerate() {
            trace.push_str(&fmt_body(tick, id, b));
            trace.push('\n');
        }
    }
    assert_snapshot(&trace, "tick=1 id=0 x=-1.0000 y=0.5000 vx=0.0000 vy=0.0000\ntick=1 id=1 x=1.0000 y=0.5000 vx=0.0000 vy=0.0000\ntick=2 id=0 x=-1.0000 y=0.5000 vx=0.0000 vy=0.0000\ntick=2 id=1 x=1.0000 y=0.5000 vx=0.0000 vy=0.0000\n");
}

#[test]
fn overlap_trace_pins_ids() {
    let mut s = Space::new();
    s.add(Body::new(0.0, 1.0, 0.0, 0.0, 0.5, 0.5));
    let a = s.overlap(0.4, 1.0, 0.5, 0.5);
    let b = s.overlap(50.0, 50.0, 1.0, 1.0);
    let trace = format!("overlap a={a:?}\noverlap b={b:?}\n");
    assert_snapshot(&trace, "overlap a=[0]\noverlap b=[]\n");
}

#[test]
fn ground_clamp_after_fall_pins_rest() {
    let mut s = Space::new();
    s.add(Body::new(0.0, 5.0, 0.0, 0.0, 0.5, 0.5));
    for _ in 0..600 {
        s.step(FIXED_DT);
    }
    let b = &s.bodies[0];
    let trace = format!("ticks=600 y={:.4} vy={:.4}\n", b.y, b.vy);
    assert_snapshot(&trace, "ticks=600 y=0.5000 vy=0.0000\n");
}

#[test]
fn deterministic_hash_equal_pins_flag() {
    let mut a = Space::new();
    let mut b = Space::new();
    for s in [&mut a, &mut b] {
        s.add(Body::new(0.0, 5.0, 0.0, 0.0, 0.5, 0.5));
        for _ in 0..60 {
            s.step(FIXED_DT);
        }
    }
    let raw = format!(
        "ticks=60 equal={} hash={}\n",
        a.state_hash() == b.state_hash(),
        a.state_hash()
    );
    let trace = redact_hash(&raw);
    assert_snapshot(&trace, "ticks=60 equal=true hash=[HASH]\n");
}
