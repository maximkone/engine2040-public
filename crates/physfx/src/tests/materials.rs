//! materials: the table's rows reach the colliders: bounce, weight and the default spawns.

use super::*;

/// One 20 cm cube dropped flat from `drop` meters onto the floor, no impulse.
fn cube(drop: f32) -> Shard {
    Shard {
        at: at([0.0, drop + 0.1, 0.0]),
        half: [0.1; 3],
        velocity: [0.0; 3],
    }
}

/// How high a cube of `material` rises after its first impact, meters.
fn first_bounce(material: Material) -> f32 {
    let mut fx = arena(Tier::Comfort);
    fx.spawn_debris_of(material, &[cube(3.0)], [0.0; 3])
        .expect("debris");
    let mut heights = Vec::new();
    for _ in 0..240 {
        fx.step();
        heights.push(fx.transforms(1.0).expect("transforms")[0].at.pos[1]);
    }
    // The first local low is the impact; the bounce ends where the cube starts falling again.
    let low = (1..heights.len() - 1)
        .find(|i| heights[*i] < heights[i - 1] && heights[*i] <= heights[i + 1])
        .expect("the cube landed");
    let mut top = low;
    while top + 1 < heights.len() && heights[top + 1] > heights[top] {
        top += 1;
    }
    heights[top] - heights[low]
}

#[test]
fn material_wood_and_stone_debris_bounce_to_different_heights() {
    let (wood, stone) = (first_bounce(Material::Wood), first_bounce(Material::Stone));
    println!("physfx bounce from 3 m: wood {wood:.3} m, stone {stone:.3} m");
    assert!(stone > 0.005, "stone still clacks: {stone}");
    assert!(
        wood > 2.5 * stone,
        "wood {wood} m should leap well past stone {stone} m"
    );
    assert!(wood < 3.0, "a bounce never beats the drop: {wood}");
}

#[test]
fn material_sand_and_flesh_do_not_bounce() {
    for material in [Material::Sand, Material::Flesh, Material::Cloth] {
        let rise = first_bounce(material);
        assert!(rise < 0.005, "{material:?} bounced {rise} m");
    }
}

/// Speed of the only body after one step, m/s along x.
fn speed_after_one_step(fx: &mut PhysFx) -> f32 {
    fx.step();
    let (before, after) = (
        fx.transforms(0.0).expect("transforms")[0].at.pos[0],
        fx.transforms(1.0).expect("transforms")[0].at.pos[0],
    );
    (after - before) / FIXED_DT
}

#[test]
fn material_density_sets_the_mass_an_impulse_moves() {
    // 100 N s into one 20 cm cube: v = J / (density * 0.008 m^3).
    let launch = |material| {
        let mut fx = PhysFx::new(Tier::Comfort);
        let shard = Shard {
            at: at([0.0, 50.0, 0.0]),
            ..cube(0.0)
        };
        fx.spawn_debris_of(material, &[shard], [100.0, 0.0, 0.0])
            .expect("debris");
        speed_after_one_step(&mut fx)
    };
    let (wood, metal) = (launch(Material::Wood), launch(Material::Metal));
    assert!((wood - 100.0 / 9.6).abs() < 0.3, "wood {wood} m/s");
    assert!((metal - 100.0 / 62.4).abs() < 0.1, "metal {metal} m/s");
    assert!(
        wood > 5.0 * metal,
        "the heavy shard is slow: {wood} vs {metal}"
    );
}

#[test]
fn material_default_spawns_are_stone_debris_and_flesh_limbs() {
    let run = |custom: bool| {
        let mut fx = arena(Tier::Comfort);
        let shards = debris_field(6, 1.5);
        if custom {
            fx.spawn_debris_of(Material::Stone, &shards, [0.0, 30.0, 0.0])
                .expect("debris");
            fx.spawn_limb_of(
                Material::Flesh,
                1,
                limb_shape(),
                at([0.0, 2.0, 0.0]),
                [1.0; 3],
            )
            .expect("limb");
        } else {
            fx.spawn_debris(&shards, [0.0, 30.0, 0.0]).expect("debris");
            fx.spawn_limb(1, limb_shape(), at([0.0, 2.0, 0.0]), [1.0; 3])
                .expect("limb");
        }
        step_n(&mut fx, 90);
        fx.transforms(0.5).expect("transforms")
    };
    assert_eq!(run(false), run(true));
}

#[test]
fn material_damping_drags_a_cloth_limb_harder_than_a_metal_one() {
    // Thrown flat at 20 m/s with nothing to land on: only the air drag of the row separates them.
    let reach = |material| {
        let mut fx = PhysFx::new(Tier::Comfort);
        fx.spawn_limb_of(
            material,
            1,
            limb_shape(),
            at([0.0, 50.0, 0.0]),
            [20.0, 0.0, 0.0],
        )
        .expect("limb");
        step_n(&mut fx, 60);
        fx.transforms(1.0).expect("transforms")[0].at.pos[0]
    };
    let (metal, cloth) = (reach(Material::Metal), reach(Material::Cloth));
    assert!(metal > 19.0, "metal barely slows: {metal} m");
    assert!(metal - cloth > 2.0, "metal {metal} m, cloth {cloth} m");
}

#[test]
fn material_a_bad_shard_is_refused_for_every_material() {
    for material in Material::ALL {
        let mut fx = arena(Tier::Web);
        let nan = Shard {
            velocity: [f32::NAN, 0.0, 0.0],
            ..cube(1.0)
        };
        assert_eq!(
            fx.spawn_debris_of(material, &[nan], [0.0; 3]).err(),
            Some(Error::NonFinite(Field::Velocity)),
            "{material:?}"
        );
        assert_eq!(fx.stats().bodies, 0);
    }
}
