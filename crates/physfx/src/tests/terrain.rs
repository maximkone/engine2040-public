//! terrain: a cooked height record becomes a collider a body can land on (WG5's accept).
//!
//! The record crosses the seam as plain integers, so these tests hand the same fields the
//! terrain cook writes and check three things: a ball rests at the cooked height, the same
//! record builds the same shape, and a malformed record is refused by name.

use crate::terrain::{HeightTerrain, TerrainError, TerrainShape};
use engine_physics::rapier::{FIXED_DT, GRAVITY_Y};
use rapier3d::dynamics::RigidBodyBuilder;
use rapier3d::geometry::ColliderBuilder;
use rapier3d::math::Vector;
use rapier3d::pipeline::PhysicsWorld;

/// Radius of the test ball, meters.
const BALL_R: f32 = 0.15;
/// The plateau's cooked height, millimetres: 1.5 m.
const PLATEAU_MM: i32 = 1500;

/// The cook's own shape: 3 x 3 samples one meter apart, every sample at 1.5 m.
fn plateau() -> HeightTerrain {
    HeightTerrain::new(3, 3, 1000, vec![PLATEAU_MM; 9]).expect("record")
}

/// A world on the fixed step whose only floor is `terrain`, with a ball dropped above the
/// middle sample at `(1.0, 3.0, 1.0)`, stepped for ten seconds.
fn drop_ball(
    terrain: &HeightTerrain,
    shape: TerrainShape,
) -> (PhysicsWorld, rapier3d::dynamics::RigidBodyHandle) {
    let mut world = PhysicsWorld::new();
    world.gravity = Vector::new(0.0, GRAVITY_Y, 0.0);
    world.integration_parameters.dt = FIXED_DT;
    let (ground_body, ground_collider) = terrain
        .body_and_collider([0, 0, 0], shape)
        .expect("collider");
    let ground = world.bodies.insert(ground_body.build());
    world
        .colliders
        .insert_with_parent(ground_collider.build(), ground, &mut world.bodies);
    let ball = world.bodies.insert(
        RigidBodyBuilder::dynamic()
            .translation(Vector::new(1.0, 3.0, 1.0))
            .build(),
    );
    let collider = ColliderBuilder::ball(BALL_R)
        .density(900.0)
        .friction(0.8)
        .restitution(0.0)
        .build();
    world
        .colliders
        .insert_with_parent(collider, ball, &mut world.bodies);
    for _ in 0..600 {
        world.step();
    }
    (world, ball)
}

/// The ball falls onto the middle sample and comes to rest one radius above the cooked
/// height: nothing else holds it up.
#[test]
fn a_ball_dropped_over_a_3x3_grid_rests_at_the_cooked_height() {
    let (world, ball) = drop_ball(&plateau(), TerrainShape::Heightfield);
    let y = world.bodies[ball].translation().y;
    // 1500 mm is 1.5 m, and the ball rests its center one radius above it.
    let want = 1.5 + BALL_R;
    assert!(
        (y - want).abs() < 0.02,
        "ball rests at {y:.3} m; the cooked height says {want:.3} m"
    );
}

/// The triangle mesh is the same integer grid, so it rests the ball at the same height.
#[test]
fn the_mesh_shape_rests_the_ball_at_the_same_height() {
    let (world, ball) = drop_ball(&plateau(), TerrainShape::Mesh);
    let y = world.bodies[ball].translation().y;
    let want = 1.5 + BALL_R;
    assert!(
        (y - want).abs() < 0.02,
        "mesh rests the ball at {y:.3} m; the cooked height says {want:.3} m"
    );
}

/// The same record and origin give the same shape parameters bit for bit, both shapes.
#[test]
fn the_same_record_builds_the_same_shape_parameters() {
    let terrain = HeightTerrain::new(3, 3, 500, vec![100, 200, 300, 400, 500, 600, 700, 800, 900])
        .expect("record");
    let first = terrain.params([-2000, 100, 3000]);
    let second = terrain.params([-2000, 100, 3000]);
    assert_eq!(first, second, "params differ between two calls");
    assert_eq!(
        first
            .heights_m
            .iter()
            .map(|h| h.to_bits())
            .collect::<Vec<_>>(),
        second
            .heights_m
            .iter()
            .map(|h| h.to_bits())
            .collect::<Vec<_>>(),
        "heights differ bit for bit"
    );
    // The body sits at the grid's center: half of the 1 m span each way from the origin.
    assert!(
        (first.center_m[0] + 1.5).abs() < 1e-6,
        "center x {:?}",
        first.center_m[0]
    );
    assert!(
        (first.center_m[2] - 3.5).abs() < 1e-6,
        "center z {:?}",
        first.center_m[2]
    );
    // The record keeps integers: the heights the seam hands out are the cook's own.
    assert_eq!(
        terrain.heights_mm(),
        &[100, 200, 300, 400, 500, 600, 700, 800, 900]
    );

    let two_shapes = |shape| {
        let (_, collider) = terrain
            .body_and_collider([0, 0, 0], shape)
            .expect("collider");
        collider.build()
    };
    let heightfield = two_shapes(TerrainShape::Heightfield);
    let mesh = two_shapes(TerrainShape::Mesh);
    let aabb = |collider: &rapier3d::geometry::Collider| {
        let aabb = collider
            .shape()
            .compute_aabb(&rapier3d::math::Pose::IDENTITY);
        [
            aabb.mins.x,
            aabb.mins.y,
            aabb.mins.z,
            aabb.maxs.x,
            aabb.maxs.y,
            aabb.maxs.z,
        ]
    };
    let (a, b) = (aabb(&heightfield), aabb(&mesh));
    for axis in 0..6 {
        assert!(
            (a[axis] - b[axis]).abs() < 1e-4,
            "shapes disagree on axis {axis}: {:?} vs {:?}",
            a[axis],
            b[axis]
        );
    }
}

/// Every wrong field is refused, and the message names it.
#[test]
fn a_malformed_record_is_refused_by_name() {
    let short = HeightTerrain::new(3, 3, 1000, vec![0; 8]).expect_err("8 heights for 9 samples");
    assert_eq!(
        short,
        TerrainError::ShortHeights {
            rows: 3,
            cols: 3,
            got: 8
        }
    );
    assert!(
        short.to_string().contains("8 heights for a 3x3 grid"),
        "{short}"
    );

    let flat = HeightTerrain::new(3, 3, 0, vec![0; 9]).expect_err("zero spacing");
    assert_eq!(flat, TerrainError::ZeroSpacing);
    assert!(flat.to_string().contains("spacing is zero"), "{flat}");

    let empty = HeightTerrain::new(0, 3, 1000, Vec::new()).expect_err("no rows");
    assert_eq!(empty, TerrainError::EmptyGrid);

    let line = HeightTerrain::new(1, 3, 1000, vec![0; 3]).expect_err("one row has no quad");
    assert_eq!(line, TerrainError::FlatGrid { rows: 1, cols: 3 });

    let huge = HeightTerrain::new(4096, 4096, 1, Vec::new()).expect_err("past the cap");
    assert_eq!(
        huge,
        TerrainError::TooManySamples {
            samples: 4096 * 4096,
            cap: crate::terrain::MAX_SAMPLES
        }
    );
}
