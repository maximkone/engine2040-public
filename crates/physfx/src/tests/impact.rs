//! impact: an authority projectile hit shoves the bodies around the hit point along its heading
//! with a real impulse, weaker with distance, capped per tier, the same every run.

use super::*;
use engine_physics::rapier::GRAVITY_Y;

/// Where every impact test hangs its bodies, meters above a world with no floor.
const AIR: f32 = 50.0;

/// A hit at the origin column, heading +x, 160 N s (10 m/s on a 16 kg stone cube) over 1 m.
const HIT: Impact = Impact {
    at_mm: [0, 50_000, 0],
    direction: [1, 0, 0],
    impulse_ns: 160,
    radius_mm: 1_000,
};

/// A 20 cm cube floating `z` meters along the row.
fn cube_at(z: f32) -> Shard {
    Shard {
        at: at([0.0, AIR, z]),
        half: [0.1; 3],
        velocity: [0.0; 3],
    }
}

/// Velocity of every body after one more step, in spawn order, m/s (gravity taken out).
fn velocities(fx: &mut PhysFx) -> Vec<[f32; 3]> {
    fx.step();
    let before = fx.transforms(0.0).expect("transforms");
    let after = fx.transforms(1.0).expect("transforms");
    let sag = GRAVITY_Y * FIXED_DT;
    before
        .iter()
        .zip(&after)
        .map(|(b, a)| {
            let mut v = [0, 1, 2].map(|k| (a.at.pos[k] - b.at.pos[k]) / FIXED_DT);
            v[1] -= sag;
            v
        })
        .collect()
}

fn row(material: Material, zs: &[f32]) -> PhysFx {
    let mut fx = PhysFx::new(Tier::Comfort);
    for z in zs {
        fx.spawn_debris_of(material, &[cube_at(*z)], [0.0; 3])
            .expect("debris");
    }
    fx
}

#[test]
fn impact_pushes_along_the_heading_weaker_with_distance_and_none_outside() {
    let zs = [0.0, 0.3, 0.6, 0.9, 1.2];
    let mut fx = row(Material::Stone, &zs);
    assert_eq!(fx.apply_impact(HIT), Ok(4), "the 1.2 m cube is outside 1 m");
    let v = velocities(&mut fx);
    for (z, v) in zs.iter().zip(&v) {
        let want = if *z < 1.0 { 10.0 * (1.0 - z) } else { 0.0 };
        assert!(
            (v[0] - want).abs() < 0.3,
            "{z} m: {} m/s along x, wanted {want}",
            v[0]
        );
        assert!(v[2].abs() < 0.5, "{z} m drifted sideways: {v:?}");
    }
    for pair in v[..4].windows(2) {
        assert!(pair[0][0] > pair[1][0], "{v:?}");
    }
}

#[test]
fn impact_a_heavy_body_gains_less_speed_than_a_light_one() {
    let speed = |material| {
        let mut fx = row(material, &[0.0]);
        fx.apply_impact(HIT).expect("impact");
        velocities(&mut fx)[0][0]
    };
    let (wood, metal) = (speed(Material::Wood), speed(Material::Metal));
    // 160 N s into 9.6 kg wood and 62.4 kg metal.
    assert!((wood - 160.0 / 9.6).abs() < 0.3, "wood {wood} m/s");
    assert!((metal - 160.0 / 62.4).abs() < 0.1, "metal {metal} m/s");
}

#[test]
fn impact_off_center_hit_spins_the_body_and_a_center_hit_does_not() {
    let mut fx = row(Material::Stone, &[0.0, 0.5]);
    let hit = Impact {
        radius_mm: 2_000,
        ..HIT
    };
    assert_eq!(fx.apply_impact(hit), Ok(2));
    fx.step();
    let spins: Vec<f32> = fx
        .transforms(1.0)
        .expect("transforms")
        .iter()
        .map(|p| {
            let [x, y, z, _] = p.at.rot;
            (x * x + y * y + z * z).sqrt()
        })
        .collect();
    assert!(
        spins[0] < 1.0e-4,
        "a hit at the center turns nothing: {spins:?}"
    );
    assert!(
        spins[1] > 1.0e-4,
        "a hit off the center turns the cube: {spins:?}"
    );
}

#[test]
fn impact_never_throws_a_body_faster_than_the_speed_cap() {
    // A 2 cm cloth cube weighs two grams: 1000 N s would be half a million m/s.
    let mut fx = PhysFx::new(Tier::Comfort);
    let tiny = Shard {
        half: [0.01; 3],
        ..cube_at(0.0)
    };
    fx.spawn_debris_of(Material::Cloth, &[tiny], [0.0; 3])
        .expect("debris");
    let hit = Impact {
        impulse_ns: 1_000,
        ..HIT
    };
    assert_eq!(fx.apply_impact(hit), Ok(1));
    let v = velocities(&mut fx)[0][0];
    assert!((v - 100.0).abs() < 2.0, "capped at 100 m/s, got {v}");
}

#[test]
fn impact_shoves_only_the_nearest_bodies_of_the_tier() {
    let cap = Tier::Potato.caps().impact_bodies;
    let mut fx = PhysFx::new(Tier::Potato);
    // Farthest spawned first, so the nearest ones are the youngest: the cap picks by distance.
    let zs: Vec<f32> = (0..cap + 3).rev().map(|k| 0.1 + 0.25 * k as f32).collect();
    for z in &zs {
        fx.spawn_debris_of(Material::Stone, &[cube_at(*z)], [0.0; 3])
            .expect("debris");
    }
    let hit = Impact {
        radius_mm: 5_000,
        ..HIT
    };
    assert_eq!(fx.apply_impact(hit), Ok(cap));
    let v = velocities(&mut fx);
    let (far, near) = v.split_at(3);
    assert!(far.iter().all(|v| v[0] == 0.0), "{far:?}");
    assert!(near.iter().all(|v| v[0] > 1.0), "{near:?}");
}

#[test]
fn impact_the_same_hit_twice_gives_the_same_motion() {
    let run = || {
        let mut fx = scene();
        let hit = Impact {
            at_mm: [-1_800, 900, 0],
            direction: [3, 1, -1],
            impulse_ns: 60,
            radius_mm: 1_500,
        };
        let struck = fx.apply_impact(hit).expect("impact");
        step_n(&mut fx, 60);
        (struck, fx.transforms(0.4).expect("transforms"), fx.stats())
    };
    let (a, b) = (run(), run());
    assert!(a.0 > 0, "the scene has bodies near the hit");
    assert_eq!(a, b);
}

#[test]
fn impact_refusals_name_the_input_and_leave_the_world_unchanged() {
    let mut touched = scene();
    let mut control = scene();
    let refused = [
        (
            Impact {
                radius_mm: 0,
                ..HIT
            },
            Field::Extent,
        ),
        (
            Impact {
                radius_mm: 5_001,
                ..HIT
            },
            Field::Extent,
        ),
        (
            Impact {
                at_mm: [0, 1_000_001, 0],
                ..HIT
            },
            Field::Position,
        ),
        (
            Impact {
                direction: [0; 3],
                ..HIT
            },
            Field::Direction,
        ),
        (
            Impact {
                impulse_ns: 1_001,
                ..HIT
            },
            Field::Impulse,
        ),
    ];
    for (hit, field) in refused {
        assert_eq!(
            touched.apply_impact(hit),
            Err(Error::OutOfRange(field)),
            "{hit:?}"
        );
    }
    step_n(&mut touched, 60);
    step_n(&mut control, 60);
    assert_eq!(touched.transforms(0.5), control.transforms(0.5));
    // The edge of the accepted range is accepted, and a hit with nobody near pushes nobody.
    let edge = Impact {
        at_mm: [1_000_000, -1_000_000, 0],
        direction: [i16::MIN, i16::MAX, 1],
        impulse_ns: 1_000,
        radius_mm: 5_000,
    };
    assert_eq!(touched.apply_impact(edge), Ok(0));
    assert_eq!(PhysFx::new(Tier::Web).apply_impact(HIT), Ok(0));
}
