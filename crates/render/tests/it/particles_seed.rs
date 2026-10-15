//! particles_seed: same seed x2 -> same stream.
use engine_render::particles::{Curve, EmitterDesc, Flipbook, ParticleEffect, FIXED_DT};
fn stream(seed: u64, ticks: u64) -> Vec<u64> {
    let mut e = ParticleEffect::demo(seed);
    let mut v = Vec::with_capacity(ticks as usize);
    for _ in 0..ticks {
        e.update(FIXED_DT);
        v.push(e.state_hash());
    }
    v
}
#[test]
fn hanabi_graph_60fps_seed_x2_identical() {
    assert_eq!(FIXED_DT, 1.0 / 60.0);
    let (a, b) = (stream(2040, 120), stream(2040, 120));
    assert_eq!(a, b);
    assert_ne!(a[0], *a.last().unwrap(), "sim must evolve");
}
#[test]
fn different_seed_diverges() {
    assert_ne!(stream(2040, 120), stream(2041, 120));
}
#[test]
fn flipbook_indices_bounded_and_advance() {
    let f = Flipbook { cols: 4, rows: 4 };
    assert_eq!(f.frames(), 16);
    let (i0, i1) = (f.index(0.1, false), f.index(0.9, false));
    assert!(i0 < i1 && i1 < 16);
    let mut e = ParticleEffect::new(
        99,
        vec![EmitterDesc {
            rate: 0.0,
            burst: 4,
            life: [1.0, 1.0],
            ..EmitterDesc::default()
        }],
        64,
    );
    e.update(FIXED_DT);
    assert_eq!(e.alive(), 4);
    let f0: Vec<u16> = e.particles().iter().map(|p| p.flip).collect();
    e.update(0.5);
    let f1: Vec<u16> = e.particles().iter().map(|p| p.flip).collect();
    assert!(
        f1.iter().zip(f0.iter()).all(|(a, b)| a >= b),
        "atlas must advance with age"
    );
}
#[test]
fn curves_drive_size_alpha_and_capacity_holds() {
    let c = Curve::new(vec![(0.0, 0.0), (1.0, 2.0)]);
    assert_eq!(c.sample(0.5), 1.0);
    let mut e = ParticleEffect::demo(7);
    for _ in 0..600 {
        e.update(FIXED_DT);
    }
    assert!(e.alive() <= 4096, "pool must clamp");
    assert!(e.spawned_total() > e.alive() as u64, "culling must recycle");
    assert!(e
        .particles()
        .iter()
        .all(|p| p.pos.iter().all(|v| v.is_finite())
            && p.alpha.is_finite()
            && (0.0..=1.5).contains(&p.age)));
}
