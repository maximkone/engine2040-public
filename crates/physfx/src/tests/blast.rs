//! blast: an authority blast shoves bodies in its radius, nobody else; an arrow or a thrown
//! object flies the arc that ends on the authority's hit.

use super::*;
use engine_core::laws;
use engine_physics::rapier::GRAVITY_Y;
use std::fmt;

/// Where every blast test hangs its bodies, meters above a world with no floor.
const AIR: f32 = 50.0;

/// A blast 50 m up, in the middle of the test row: 5 m reach, 3500 N s (50 m/s at the center).
const BLAST: Blast = Blast {
    center_mm: [0, 50_000, 0],
    radius_mm: 5_000,
    impulse_ns: 3_500,
};

/// A 20 cm cube floating at `x` meters from the blast center.
fn shard_at(x: f32) -> Shard {
    Shard {
        at: at([x, AIR, 0.0]),
        half: [0.1; 3],
        velocity: [0.0; 3],
    }
}

/// Speed along x of every body after one more step, in spawn order, m/s.
fn x_speeds(fx: &mut PhysFx) -> Vec<f32> {
    fx.step();
    let before = fx.transforms(0.0).expect("transforms");
    let after = fx.transforms(1.0).expect("transforms");
    before
        .iter()
        .zip(&after)
        .map(|(b, a)| (a.at.pos[0] - b.at.pos[0]) / FIXED_DT)
        .collect()
}

/// Stone shards at +1..+4 m, metal ones at -1..-4 m (mirrored), then rim, outside: +5 m and +8 m.
fn row() -> (PhysFx, Vec<f32>) {
    let mut fx = PhysFx::new(Tier::Comfort);
    let mut xs = Vec::new();
    for d in [1.0, 2.0, 3.0, 4.0] {
        fx.spawn_debris_of(Material::Stone, &[shard_at(d)], [0.0; 3])
            .expect("stone");
        xs.push(d);
    }
    for d in [1.0, 2.0, 3.0, 4.0] {
        fx.spawn_debris_of(Material::Metal, &[shard_at(-d)], [0.0; 3])
            .expect("metal");
        xs.push(-d);
    }
    for d in [5.0, 8.0] {
        fx.spawn_debris_of(Material::Wood, &[shard_at(d)], [0.0; 3])
            .expect("wood");
        xs.push(d);
    }
    (fx, xs)
}

#[test]
fn blast_bodies_in_radius_gain_outward_speed_in_order_of_distance_none_outside_move() {
    let (mut fx, xs) = row();
    assert_eq!(fx.apply_blast(BLAST), Ok(8), "eight bodies are inside 5 m");
    let speeds = x_speeds(&mut fx);
    for (x, v) in xs.iter().zip(&speeds) {
        let d = x.abs();
        let outward = v * x.signum();
        if d >= 5.0 {
            assert_eq!(*v, 0.0, "{x} m is outside the radius");
            continue;
        }
        // Linear falloff from 50 m/s at the center to nothing at the radius.
        let want = 50.0 * (1.0 - d / 5.0);
        assert!(
            (outward - want).abs() < 0.02 * want,
            "{x} m: {outward} m/s, wanted {want}"
        );
    }
    // Stone side: strictly slower the farther out. Metal side mirrors it whatever its weight.
    for pair in speeds[..4].windows(2) {
        assert!(pair[0] > pair[1], "{speeds:?}");
    }
    for (stone, metal) in speeds[..4].iter().zip(&speeds[4..8]) {
        assert!((stone + metal).abs() < 0.02 * stone, "{stone} vs {metal}");
    }
}

#[test]
fn blast_pushes_straight_out_from_the_center_in_three_dimensions() {
    let mut fx = PhysFx::new(Tier::Comfort);
    // 3-4-5 triangle in the XZ plane, 1 m above the center: distance sqrt(26) = 5.099 m.
    let blast = Blast {
        radius_mm: 8_000,
        impulse_ns: 7_000,
        ..BLAST
    };
    let shard = Shard {
        at: at([3.0, AIR + 1.0, 4.0]),
        ..shard_at(0.0)
    };
    fx.spawn_debris_of(Material::Stone, &[shard], [0.0; 3])
        .expect("stone");
    assert_eq!(fx.apply_blast(blast), Ok(1));
    fx.step();
    let (a, b) = (
        fx.transforms(0.0).expect("transforms")[0].at.pos,
        fx.transforms(1.0).expect("transforms")[0].at.pos,
    );
    // Gravity pulls the first step down by g dt^2 (n + 1) / 2n for n solver substeps.
    let n = Tier::Comfort.caps().solver_iterations as f32;
    let sag = GRAVITY_Y * FIXED_DT * FIXED_DT * (n + 1.0) / (2.0 * n);
    let push = [b[0] - a[0], b[1] - a[1] - sag, b[2] - a[2]];
    let length = push.iter().map(|c| c * c).sum::<f32>().sqrt();
    let want = [3.0, 1.0, 4.0].map(|c| c / 26.0_f32.sqrt());
    for k in 0..3 {
        assert!((push[k] / length - want[k]).abs() < 0.02, "{push:?}");
    }
    // 100 m/s at the center, falloff 1 - 5.099 / 8.
    let speed = length / FIXED_DT;
    assert!(
        (speed - 100.0 * (1.0 - 26.0_f32.sqrt() / 8.0)).abs() < 1.0,
        "{speed}"
    );
}

#[test]
fn blast_wakes_a_sleeping_body_in_the_radius_and_leaves_the_far_ones_asleep() {
    let mut fx = arena(Tier::Comfort);
    let near = Shard {
        at: at([0.0, 0.1, 0.0]),
        half: [0.1; 3],
        velocity: [0.0; 3],
    };
    let far = Shard {
        at: at([30.0, 0.1, 0.0]),
        ..near
    };
    fx.spawn_debris(&[near, far], [0.0; 3]).expect("debris");
    step_n(&mut fx, 2);
    assert_eq!(fx.stats().awake, 2, "freshly spawned bodies are awake");
    let mut ticks = 0;
    while fx.stats().awake > 0 {
        fx.step();
        ticks += 1;
        assert!(ticks < 600, "debris never settled");
    }
    let rest = fx.transforms(1.0).expect("transforms");
    let blast = Blast {
        center_mm: [1_000, 0, 0],
        radius_mm: 4_000,
        impulse_ns: 700,
    };
    assert_eq!(fx.apply_blast(blast), Ok(1), "only the near cube is inside");
    fx.step();
    assert_eq!(fx.stats().awake, 1, "the far cube sleeps on");
    let after = fx.transforms(1.0).expect("transforms");
    assert_ne!(after[0], rest[0], "the near cube was thrown");
    assert!(after[0].at.pos[0] < 0.0, "away from a center at +x");
    assert_eq!(after[1], rest[1], "the far cube did not move");
}

#[test]
fn blast_the_same_blast_twice_gives_the_same_velocities() {
    let blast = Blast {
        center_mm: [0, 300, 0],
        radius_mm: 4_500,
        impulse_ns: 1_500,
    };
    let run = |blast: Blast| {
        let mut fx = scene();
        let pushed = fx.apply_blast(blast).expect("blast");
        let mut seen = Vec::new();
        for tick in 0..120 {
            fx.step();
            if tick % 20 == 0 {
                seen.push(fx.transforms(0.37).expect("transforms"));
            }
        }
        (pushed, seen, fx.stats())
    };
    let (a, b) = (run(blast), run(blast));
    assert!(a.0 > 0, "the scene has bodies inside the radius");
    assert_eq!(a, b);
    let harder = run(Blast {
        impulse_ns: 3_000,
        ..blast
    });
    assert_eq!(harder.0, a.0, "same bodies in range");
    assert_ne!(harder.1, a.1, "a harder blast moved them differently");
}

#[test]
fn blast_shoves_only_the_nearest_bodies_of_the_tier() {
    let cap = Tier::Potato.caps().blast_bodies;
    let mut fx = PhysFx::new(Tier::Potato);
    // Farthest spawned first, so the nearest ones are the youngest: the cap picks by distance.
    let total = cap + 10;
    for k in (0..total).rev() {
        fx.spawn_debris_of(
            Material::Stone,
            &[shard_at(0.5 + 0.25 * k as f32)],
            [0.0; 3],
        )
        .expect("stone");
    }
    let blast = Blast {
        radius_mm: 20_000,
        ..BLAST
    };
    assert_eq!(fx.apply_blast(blast), Ok(cap), "the tier shoves {cap}");
    let speeds = x_speeds(&mut fx);
    let (far, near) = speeds.split_at(total - cap);
    assert!(
        far.iter().all(|v| *v == 0.0),
        "the far ten stay put: {far:?}"
    );
    assert!(
        near.iter().all(|v| *v > 1.0),
        "the nearest are thrown: {near:?}"
    );
}

#[test]
fn blast_refusals_name_the_input_and_leave_the_world_unchanged() {
    let mut touched = scene();
    let mut control = scene();
    let refused = [
        (
            Blast {
                radius_mm: 0,
                ..BLAST
            },
            Field::Extent,
        ),
        (
            Blast {
                radius_mm: 100_001,
                ..BLAST
            },
            Field::Extent,
        ),
        (
            Blast {
                center_mm: [0, i32::MAX, 0],
                ..BLAST
            },
            Field::Position,
        ),
        (
            Blast {
                center_mm: [i32::MIN, 0, 0],
                ..BLAST
            },
            Field::Position,
        ),
        // One N s past the cap is 100.01 m/s for a 70 kg body: a bug upstream, not a blast.
        (
            Blast {
                impulse_ns: MAX_BLAST_NS + 1,
                ..BLAST
            },
            Field::Impulse,
        ),
        (
            Blast {
                impulse_ns: u32::MAX,
                ..BLAST
            },
            Field::Impulse,
        ),
    ];
    for (blast, field) in refused {
        assert_eq!(
            touched.apply_blast(blast),
            Err(Error::OutOfRange(field)),
            "{blast:?}"
        );
    }
    assert_eq!(touched.transforms(0.5), control.transforms(0.5));
    step_n(&mut touched, 60);
    step_n(&mut control, 60);
    assert_eq!(touched.transforms(0.5), control.transforms(0.5));
    assert_eq!(touched.stats(), control.stats());
    // The edge of the accepted range is accepted, and a blast with nobody near pushes nobody.
    let edge = Blast {
        center_mm: [1_000_000, -1_000_000, 0],
        radius_mm: 100_000,
        impulse_ns: MAX_BLAST_NS,
    };
    assert_eq!(touched.apply_blast(edge), Ok(0));
    assert_eq!(PhysFx::new(Tier::Web).apply_blast(BLAST), Ok(0));
}

/// Where a flight starts and ends.
const SHOT: Flight = Flight {
    from: [0.0, 2.0, 0.0],
    to: [20.0, 1.5, 3.0],
    ticks: 40,
};

fn arrow() -> BoneShape {
    BoneShape {
        radius: 0.015,
        half_height: 0.45,
    }
}

fn position(fx: &PhysFx) -> [f32; 3] {
    fx.transforms(1.0).expect("transforms")[0].at.pos
}

#[test]
fn thrown_body_arrives_on_the_authority_hit_on_the_hit_tick_at_every_tier() {
    let flights = [
        SHOT,
        Flight {
            from: [-5.0, 1.0, -5.0],
            to: [5.0, 6.0, 5.0],
            ticks: 90,
        },
        Flight {
            from: [0.0, 10.0, 0.0],
            to: [0.0, 0.0, 0.0],
            ticks: 30,
        },
        Flight {
            from: [3.0, 1.0, 3.0],
            to: [3.0, 1.0, 3.0],
            ticks: 45,
        },
        Flight {
            from: [0.0, 1.0, 0.0],
            to: [60.0, 4.0, -30.0],
            ticks: 100,
        },
    ];
    for tier in Tier::ALL {
        for flight in flights {
            let mut fx = PhysFx::new(tier);
            fx.spawn_thrown(9, arrow(), Material::Wood, flight)
                .expect("thrown");
            step_n(&mut fx, flight.ticks as usize);
            let here = position(&fx);
            for k in 0..3 {
                assert!(
                    (here[k] - flight.to[k]).abs() < 0.01,
                    "{tier:?} {flight:?}: arrived at {here:?}"
                );
            }
        }
    }
}

#[test]
fn thrown_body_flies_an_arc_not_a_straight_line() {
    let mut fx = PhysFx::new(Tier::Comfort);
    fx.spawn_thrown(1, arrow(), Material::Wood, SHOT)
        .expect("thrown");
    step_n(&mut fx, 20);
    let mid = position(&fx);
    let chord_y = (SHOT.from[1] + SHOT.to[1]) / 2.0;
    // g t^2 / 8 above the chord for a 2/3 s flight: 0.545 m.
    let lift = mid[1] - chord_y;
    assert!((0.45..0.65).contains(&lift), "{lift} m above the chord");
    let chord_x = (SHOT.from[0] + SHOT.to[0]) / 2.0;
    assert!(
        (mid[0] - chord_x).abs() < 0.2,
        "half way along at half time"
    );
}

#[test]
fn thrown_arrow_points_along_its_flight_at_launch() {
    let straight_down = Flight {
        from: [0.0, 10.0, 0.0],
        to: [0.0, 0.0, 0.0],
        ticks: 30,
    };
    let straight_up = Flight {
        to: SHOT.from,
        ..SHOT
    };
    for flight in [SHOT, straight_down, straight_up] {
        let mut fx = PhysFx::new(Tier::Comfort);
        fx.spawn_thrown(1, arrow(), Material::Wood, flight)
            .expect("thrown");
        let [x, y, z, w] = fx.transforms(1.0).expect("transforms")[0].at.rot;
        let axis = [
            2.0 * (x * y - w * z),
            1.0 - 2.0 * (x * x + z * z),
            2.0 * (y * z + w * x),
        ];
        let start = position(&fx);
        fx.step();
        let moved = position(&fx);
        let step = [0, 1, 2].map(|k| moved[k] - start[k]);
        let length = step.iter().map(|c| c * c).sum::<f32>().sqrt();
        let dot: f32 = (0..3).map(|k| axis[k] * step[k] / length).sum();
        assert!(
            dot > 0.999,
            "{flight:?}: the capsule axis is {dot} along the flight"
        );
    }
}

#[test]
fn thrown_ids_are_their_own_names_and_a_respawn_replaces() {
    let mut fx = PhysFx::new(Tier::Comfort);
    fx.spawn_limb(3, limb_shape(), at([0.0, 40.0, 0.0]), [0.0; 3])
        .expect("limb");
    fx.spawn_thrown(3, arrow(), Material::Wood, SHOT)
        .expect("thrown");
    assert_eq!(fx.stats().bodies, 2, "thrown 3 is not limb 3");
    fx.spawn_thrown(3, arrow(), Material::Metal, SHOT)
        .expect("again");
    assert_eq!(fx.stats().bodies, 2, "thrown 3 was replaced");
    let keys: Vec<Key> = fx
        .transforms(1.0)
        .expect("transforms")
        .iter()
        .map(|p| p.key)
        .collect();
    assert_eq!(keys, [Key::Limb { part: 3 }, Key::Thrown { id: 3 }]);
}

/// The error a flight is refused with; the world it was offered to stays empty.
fn refusal(flight: Flight) -> Error {
    let mut fx = PhysFx::new(Tier::Web);
    let err = fx
        .spawn_thrown(1, arrow(), Material::Wood, flight)
        .expect_err("refused");
    assert_eq!(fx.stats().bodies, 0, "{flight:?} left a body behind");
    err
}

#[test]
fn thrown_refusals_name_the_input_and_leave_the_world_unchanged() {
    assert_eq!(
        refusal(Flight { ticks: 0, ..SHOT }),
        Error::OutOfRange(Field::Ticks)
    );
    assert_eq!(
        refusal(Flight {
            ticks: 1_201,
            ..SHOT
        }),
        Error::OutOfRange(Field::Ticks)
    );
    assert_eq!(
        refusal(Flight {
            from: [f32::NAN, 0.0, 0.0],
            ..SHOT
        }),
        Error::NonFinite(Field::Position)
    );
    assert_eq!(
        refusal(Flight {
            to: [0.0, f32::INFINITY, 0.0],
            ..SHOT
        }),
        Error::NonFinite(Field::Position)
    );
    assert_eq!(
        refusal(Flight {
            to: [0.0, 2.0e6, 0.0],
            ..SHOT
        }),
        Error::OutOfRange(Field::Position)
    );
    // 200 m in one tick is 12000 m/s: no throw reaches that hit.
    assert_eq!(
        refusal(Flight {
            to: [200.0, 2.0, 0.0],
            ticks: 1,
            ..SHOT
        }),
        Error::OutOfRange(Field::Velocity)
    );
    let mut fx = PhysFx::new(Tier::Web);
    let flat = BoneShape {
        radius: 0.0,
        half_height: 0.4,
    };
    assert_eq!(
        fx.spawn_thrown(1, flat, Material::Wood, SHOT),
        Err(Error::OutOfRange(Field::Extent))
    );
    assert_eq!(fx.stats().bodies, 0);
    // Both ends of the accepted time range are fine: 20 s up and down is 98 m/s at launch.
    let end = [SHOT.from[0], SHOT.from[1], SHOT.from[2] + 0.5];
    for (id, ticks) in [(2, 1), (3, 1_200)] {
        let edge = Flight {
            to: end,
            ticks,
            ..SHOT
        };
        assert_eq!(
            fx.spawn_thrown(id, arrow(), Material::Wood, edge),
            Ok(()),
            "{ticks}"
        );
    }
}

/// A world held as one value, so the mirror law can compare two of them. Its Debug line names
/// the bodies' positions, which is all a failing law needs to show.
struct Scene(PhysFx);

impl fmt::Debug for Scene {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let positions: Vec<[f32; 3]> = self
            .0
            .transforms(1.0)
            .map(|v| v.iter().map(|p| p.at.pos).collect::<Vec<[f32; 3]>>())
            .unwrap_or_default();
        write!(f, "{positions:?}")
    }
}

/// The angles from +x, radians, of the four shards of a mirror test. They sit on one ring, so one
/// blast pushes them with equal force, straight out from the center: no two ever meet.
const RING_ANGLES: [f32; 4] = [0.3, 0.9, 1.5, 2.1];

/// A stone shard on a 2 m ring about `center` at `angle`; `sign` -1 gives its x-mirror.
fn ring_shard(center: [f32; 3], angle: f32, sign: f32) -> Shard {
    Shard {
        at: at([
            center[0] + sign * 2.0 * angle.cos(),
            center[1] + 2.0 * angle.sin(),
            center[2],
        ]),
        half: [0.1; 3],
        velocity: [0.0; 3],
    }
}

/// Every body of `a` sits at the x-mirror of the same body of `b`, to a millimetre.
fn mirrored(a: &Scene, b: &Scene) -> bool {
    let (Ok(pa), Ok(pb)) = (a.0.transforms(1.0), b.0.transforms(1.0)) else {
        return false;
    };
    pa.len() == pb.len()
        && pa.iter().zip(&pb).all(|(p, q)| {
            let (x, y) = (p.at.pos, q.at.pos);
            (x[0] + y[0]).abs() < 1.0e-3
                && (x[1] - y[1]).abs() < 1.0e-3
                && (x[2] - y[2]).abs() < 1.0e-3
        })
}

#[test]
fn blast_debris_mirrored_in_x_flies_mirrored() {
    // Stone on a ring, and its x-mirror on the other side, the same blast on both, in the air
    // where nothing touches them: every body stays the x-mirror of its twin for two seconds.
    let build = |sign: f32| {
        let mut fx = PhysFx::new(Tier::Comfort);
        for angle in RING_ANGLES {
            let shard = ring_shard([0.0, AIR, 0.0], angle, sign);
            fx.spawn_debris_of(Material::Stone, &[shard], [0.0; 3])
                .expect("stone");
        }
        assert_eq!(fx.apply_blast(BLAST), Ok(4), "all four are in the radius");
        Scene(fx)
    };
    laws::mirror(build(1.0), build(-1.0), 120, |s, _| s.0.step(), mirrored)
        .expect("mirrored debris flies mirrored");
}

#[test]
#[ignore = "law break: a cube sliding on the floor tips by rounding, 4 mm off its mirror at tick 56 (rapier contact), see lead"]
fn blast_debris_mirrored_in_x_lands_mirrored() {
    // Stone on a ring and its x-mirror, the same blast on both, down to the floor.
    let blast = Blast {
        center_mm: [0, 1_000, 0],
        radius_mm: 3_000,
        impulse_ns: 2_000,
    };
    let build = |sign: f32| {
        let mut fx = arena(Tier::Comfort);
        for angle in RING_ANGLES {
            let shard = ring_shard([0.0, 1.0, 0.0], angle, sign);
            fx.spawn_debris_of(Material::Stone, &[shard], [0.0; 3])
                .expect("stone");
        }
        assert_eq!(fx.apply_blast(blast), Ok(4), "all four are in the radius");
        Scene(fx)
    };
    laws::mirror(build(1.0), build(-1.0), 240, |s, _| s.0.step(), mirrored)
        .expect("mirrored debris lands mirrored");
}

#[test]
fn a_thrown_arc_lands_on_its_hit_whatever_the_substep_count() {
    // The launch velocity is worked out for the world's substeps. A world with n substeps and one
    // with 2n must both land on the authority's hit, a millimetre off at most.
    for n in [1, 2, 4, 8] {
        for substeps in [n, 2 * n] {
            let caps = Caps {
                solver_iterations: substeps,
                ..generous()
            };
            let mut fx = PhysFx::with_caps(caps).expect("caps");
            fx.spawn_thrown(9, arrow(), Material::Wood, SHOT)
                .expect("thrown");
            step_n(&mut fx, usize::try_from(SHOT.ticks).expect("ticks"));
            let here = position(&fx);
            for k in 0..3 {
                assert!(
                    (here[k] - SHOT.to[k]).abs() < 1.0e-3,
                    "{substeps} substeps: arrived at {here:?}"
                );
            }
        }
    }
}
