//! vfx_r5: R5 acceptance over the public API — fixed-dt particles with
//! oldest-reuse, torch flames on the same core, bounded cell-hashed decals
//! with tick fade, instanced-quad upload via the v1 mesh path.
#![cfg(feature = "v1")]

use engine_render::particles::FIXED_DT;
use engine_render::v1_vfx::{
    cell_of, DecalKind, DecalSet, ParticleKind, Particles, BLOOD_FADE_TICKS, INSTANCE_STRIDE,
    MAX_DECALS, MAX_PARTICLES,
};

#[test]
fn particles_pool_cap_reuses_oldest_with_exact_counter() {
    let mut p = Particles::with_capacity(2);
    assert!(p.spawn([0.0, 0.0, 0.0], [0.0, 0.0, 0.0], 10.0, ParticleKind::Dust));
    assert!(p.spawn([1.0, 0.0, 0.0], [0.0, 0.0, 0.0], 10.0, ParticleKind::Dust));
    assert!(p.spawn([2.0, 0.0, 0.0], [0.0, 0.0, 0.0], 10.0, ParticleKind::Dust));
    assert_eq!((p.alive_count(), p.reused_count()), (2, 1));
    assert_eq!(p.capacity(), 2);
    assert_eq!(MAX_PARTICLES, 1024);
}

#[test]
fn same_spawn_script_twice_identical() {
    let script = |p: &mut Particles| {
        p.spawn([0.0, 0.1, 0.0], [1.0, 2.0, 0.0], 1.0, ParticleKind::Dust);
        p.spawn(
            [0.0, 1.0, 0.0],
            [0.0, 0.5, 0.0],
            0.7,
            ParticleKind::TorchSpark,
        );
        p.spawn_torch_flame([2.0, 0.0, -1.0]);
    };
    let mut a = Particles::new();
    let mut b = Particles::new();
    script(&mut a);
    script(&mut b);
    for _ in 0..90 {
        a.step(FIXED_DT);
        b.step(FIXED_DT);
    }
    assert_eq!(a.state_hash(), b.state_hash());
    assert_eq!(a.upload_instances(), b.upload_instances());
}

#[test]
fn decals_bounded_with_exact_cell_query() {
    let mut d = DecalSet::with_capacity(2);
    assert!(d.splat([0.2, 0.0, 0.3], [0.0, 1.0, 0.0], 0.3, DecalKind::Blood));
    assert!(d.splat([5.0, 0.0, 5.0], [0.0, 1.0, 0.0], 0.3, DecalKind::Blood));
    assert!(d.splat([0.8, 0.0, 0.8], [0.0, 1.0, 0.0], 0.3, DecalKind::Blood));
    assert_eq!((d.len(), d.evicted_count()), (2, 1));
    assert_eq!(cell_of([0.8, 0.0, 0.8]), (0, 0));
    // Oldest (cell (0,0) at 0.2) was evicted; the surviving (0,0) is 0.8.
    let got = d.query((0, 0));
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].pos[0], 0.8);
    assert_eq!(d.query((5, 5)).len(), 1);
    assert_eq!(MAX_DECALS, 256);
}

#[test]
fn decal_fade_schedule_exact_on_tick_script() {
    let mut d = DecalSet::new();
    assert!(d.splat([0.0, 0.0, 0.0], [0.0, 1.0, 0.0], 0.5, DecalKind::Blood));
    for _ in 0..100 {
        d.tick();
    }
    assert_eq!(d.decals()[0].age_ticks, 100);
    let bytes = d.upload_instances();
    let alpha = f32::from_le_bytes(bytes[28..32].try_into().unwrap());
    assert_eq!(
        alpha,
        (BLOOD_FADE_TICKS - 100) as f32 / BLOOD_FADE_TICKS as f32
    );
    for _ in 100..BLOOD_FADE_TICKS {
        d.tick();
    }
    assert!(d.is_empty());
    assert_eq!(bytes.len(), INSTANCE_STRIDE);
}
