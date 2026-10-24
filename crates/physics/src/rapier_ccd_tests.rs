//! PX48: a teleport or a snapshot restore must clear the CCD solver's cached fixed-target
//! list, and CCD (`max_ccd_substeps > 0`, the default) must stop a fast body at a thin wall.
//!
//! Reads the world's own private state on purpose: the accept for this row is a tunnel count
//! over 100 seeds plus an identical replay after a restore, not a new public surface.

use super::*;

/// Capsule radius of the test bullet: its thinnest extent, so it sweeps every step.
const BULLET_RADIUS: f32 = 0.05;
/// Capsule cylinder half-height; the total extent is twice this plus the two caps.
const BULLET_HALF_HEIGHT: f32 = 0.1;
/// The wall: 0.2 m thick along z, tall and wide enough to be crossed, never walked around.
const WALL_HALF: [f32; 3] = [6.0, 6.0, 0.1];
/// Distance from the wall's near face to the bullet's start.
const START_Z: f32 = -1.0;

/// A wall of 0.2 m thickness across z with earth gravity and no floor.
fn wall_world() -> World {
    let mut w = World::new();
    w.add_static_box(WALL_HALF, [0.0, 0.0, 0.0]).expect("wall");
    w
}

/// Spawn the bullet on the wall's -z side and send it at `z_speed` m/s through the wall.
fn bullet(world: &mut World, z_speed: f32) -> usize {
    let id = world
        .spawn_capsule([0.0, 0.0, START_Z], BULLET_RADIUS, BULLET_HALF_HEIGHT)
        .expect("bullet");
    set_linvel(world, id, z_speed);
    id
}

/// One velocity write on a body in the world's order.
fn set_linvel(world: &mut World, id: usize, z_speed: f32) {
    let handle = *world.order.get(id).expect("body handle");
    let body = world.bodies.get_mut(handle).expect("body");
    body.set_linvel(Vector::new(0.0, 0.0, z_speed), true);
}

/// Speed for one seed: 60..258 m/s, all of them far past a 0.2 m step at `FIXED_DT`.
fn speed_for(seed: u32) -> f32 {
    60.0 + f32::from(u16::try_from(seed).expect("seed fits u16")) * 2.0
}

/// CCD stops every fast bullet at the wall's near face: no seed ends up past it.
#[test]
fn ccd_stops_fast_capsule_at_wall_over_100_seeds() {
    for seed in 0..100u32 {
        let mut w = wall_world();
        let id = bullet(&mut w, speed_for(seed));
        w.step(FIXED_DT).expect("step");
        let z = w.body_position(id).expect("position")[2];
        assert!(z <= -WALL_HALF[2], "seed {seed}: tunnelled to z={z}");
    }
}

/// The accept: teleport a fast bullet across the wall between ticks, 100 seeds, zero tunnels.
/// Every teleport clears the CCD fixed-target cache exactly once.
#[test]
fn teleport_across_a_wall_clears_ccd_targets_over_100_seeds() {
    for seed in 0..100u32 {
        let speed = speed_for(seed);
        let mut w = wall_world();
        let id = bullet(&mut w, speed);
        w.step(FIXED_DT).expect("first step");
        let near = w.body_position(id).expect("position")[2];
        assert!(near <= -WALL_HALF[2], "seed {seed}: tunnelled to z={near}");

        // Teleport to the far side with the velocity reversed: a rollback lands here.
        let clears = w.ccd_clears();
        assert!(
            w.set_body_translation(id, [0.0, 0.0, -START_Z]),
            "seed {seed}: teleport refused"
        );
        assert_eq!(w.ccd_clears(), clears + 1, "seed {seed}: cache kept");
        let handle = *w.order.get(id).expect("handle");
        w.bodies
            .get_mut(handle)
            .expect("body")
            .set_linvel(Vector::new(0.0, 0.0, -speed), true);
        w.step(FIXED_DT).expect("second step");
        let far = w.body_position(id).expect("position")[2];
        assert!(
            far >= WALL_HALF[2],
            "seed {seed}: tunnelled back to z={far}"
        );
    }
}

/// `max_ccd_substeps = 0` is the documented off switch: the same shot tunnels.
#[test]
fn max_ccd_substeps_zero_disables_ccd() {
    let mut w = wall_world();
    w.params.max_ccd_substeps = 0;
    let id = bullet(&mut w, 200.0);
    w.step(FIXED_DT).expect("step");
    let z = w.body_position(id).expect("position")[2];
    assert!(z > WALL_HALF[2], "CCD was not off: stopped at z={z}");
}

/// A restore is a teleport to the solver: it clears the cache once, and the replay after it is
/// identical to the run it interrupted.
#[test]
fn restore_clears_ccd_targets_and_replays_identically() {
    let mut w = wall_world();
    let id = bullet(&mut w, 120.0);
    for _ in 0..30 {
        w.step(FIXED_DT).expect("step");
    }
    let saved = w.snapshot();
    for _ in 0..20 {
        w.step(FIXED_DT).expect("live step");
    }
    let live = w.state_hash();

    let clears = w.ccd_clears();
    w.restore(saved).expect("restore");
    assert_eq!(w.ccd_clears(), clears + 1, "restore kept the CCD cache");
    for _ in 0..20 {
        w.step(FIXED_DT).expect("replay step");
    }
    assert_eq!(w.state_hash(), live, "replay after restore diverged");
    // CCD still holds the bullet on the wall's near face: no tunnel through the replay.
    let z = w.body_position(id).expect("position")[2];
    assert!(z <= -WALL_HALF[2], "bullet tunnelled to z={z}");
}
