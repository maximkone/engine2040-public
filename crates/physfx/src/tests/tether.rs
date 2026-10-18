//! tether: the rope between two fighters holds its caps, lies straight when taut, sags when
//! slack, never explodes, sleeps, falls when released and pulls like a spring and a damper.

use super::*;

/// The rope's lowest allowed node height, meters (the module's own constant is private).
const FLOOR: f32 = 0.02;

const A: [f32; 3] = [0.0, 1.5, 0.0];

fn end(x: f32) -> [f32; 3] {
    [x, 1.5, 0.0]
}

fn length(nodes: &[[f32; 3]]) -> f32 {
    nodes
        .windows(2)
        .map(|w| {
            (0..3)
                .map(|k| (w[1][k] - w[0][k]).powi(2))
                .sum::<f32>()
                .sqrt()
        })
        .sum()
}

fn rope(fx: &PhysFx, id: u32) -> Vec<[f32; 3]> {
    fx.tether_nodes(id, 1.0).expect("alpha").expect("rope")
}

fn hold(fx: &mut PhysFx, id: u32, a: [f32; 3], b: [f32; 3], ticks: usize) {
    for _ in 0..ticks {
        fx.move_tether(id, a, b).expect("ends");
        fx.step();
    }
}

#[test]
fn tether_caps_nodes_and_ropes_per_tier_and_recycles_the_oldest() {
    for tier in Tier::ALL {
        let caps = tier.caps();
        let mut fx = PhysFx::new(tier);
        for id in 0..caps.tethers as u32 + 3 {
            fx.attach_tether(id, A, end(2.0), 3.0).expect("attach");
            assert!(
                fx.tether_count() <= caps.tethers,
                "{tier:?} over its rope cap"
            );
        }
        assert_eq!(fx.tether_count(), caps.tethers);
        assert_eq!(rope(&fx, caps.tethers as u32 + 2).len(), caps.tether_nodes);
        assert!(
            fx.tether_nodes(0, 1.0).expect("alpha").is_none(),
            "oldest recycled"
        );
        assert_eq!(fx.stats().recycled, 3);
        // The same id replaces the rope instead of adding one.
        fx.attach_tether(caps.tethers as u32 + 2, A, end(1.0), 2.0)
            .expect("again");
        assert_eq!(fx.tether_count(), caps.tethers);
    }
}

#[test]
fn tether_taut_rope_is_straight_and_as_long_as_the_gap() {
    let mut fx = PhysFx::new(Tier::Comfort);
    fx.attach_tether(1, A, end(5.0), 4.0).expect("attach");
    hold(&mut fx, 1, A, end(5.0), 5);
    let info = fx.tether_info(1).expect("info");
    assert_eq!(info.state, TetherState::Taut);
    let nodes = rope(&fx, 1);
    assert!((length(&nodes) - 5.0).abs() < 1e-3, "{}", length(&nodes));
    assert!(nodes.iter().all(|n| (n[1] - 1.5).abs() < 1e-4), "straight");
    assert_eq!(nodes[0], A);
    assert_eq!(*nodes.last().expect("end"), end(5.0));
}

#[test]
fn tether_slack_rope_sags_and_keeps_its_length() {
    let mut fx = PhysFx::new(Tier::Comfort);
    fx.attach_tether(1, A, end(3.0), 4.0).expect("attach");
    hold(&mut fx, 1, A, end(3.0), 240);
    assert_eq!(fx.tether_info(1).expect("info").state, TetherState::Slack);
    let nodes = rope(&fx, 1);
    let lowest = nodes.iter().map(|n| n[1]).fold(f32::MAX, f32::min);
    assert!(lowest < 1.0, "the middle hangs under the ends: {lowest}");
    assert!(lowest > FLOOR, "and rests above the floor");
    let total = length(&nodes);
    assert!(total > 3.0 && total < 4.0 * 1.06, "length {total}");
    assert_eq!(fx.tether_info(1).expect("info").pull, [0.0; 3]);
}

#[test]
fn tether_survives_ends_that_jump_without_blowing_up() {
    let mut fx = PhysFx::new(Tier::Potato);
    fx.attach_tether(1, A, end(2.0), 3.0).expect("attach");
    for tick in 0..600 {
        // The ends teleport across the arena every few ticks, inside and outside the length.
        let jump = if tick % 7 < 3 { 40.0 } else { -40.0 };
        let a = [jump * 0.5, 1.5 + (tick % 5) as f32, 0.0];
        let b = [-jump * 0.5, 0.2, (tick % 3) as f32 * 9.0];
        fx.move_tether(1, a, b).expect("ends");
        fx.step();
        for node in rope(&fx, 1) {
            assert!(node.iter().all(|c| c.is_finite()), "tick {tick}");
            assert!(node.iter().all(|c| c.abs() < 80.0), "tick {tick}: {node:?}");
        }
    }
}

#[test]
fn tether_sleeps_when_still_and_wakes_when_an_end_moves() {
    let mut fx = PhysFx::new(Tier::Comfort);
    fx.attach_tether(1, A, end(3.0), 4.0).expect("attach");
    hold(&mut fx, 1, A, end(3.0), 600);
    assert!(fx.tether_info(1).expect("info").asleep);
    let (early, late) = (
        fx.tether_nodes(1, 0.0).expect("a").expect("r"),
        fx.tether_nodes(1, 1.0).expect("a").expect("r"),
    );
    assert_eq!(early, late, "a sleeping rope reads the same at every alpha");
    fx.move_tether(1, A, end(3.5)).expect("ends");
    assert!(!fx.tether_info(1).expect("info").asleep, "an end moved");
    fx.step();
    assert_ne!(rope(&fx, 1), late);
}

#[test]
fn tether_same_ends_give_the_same_rope_twice() {
    let run = || {
        let mut fx = PhysFx::new(Tier::Web);
        fx.attach_tether(4, A, end(3.0), 4.0).expect("attach");
        for tick in 0..200 {
            let x = 3.0 + 1.5 * (tick as f32 * 0.07).sin();
            fx.move_tether(4, A, end(x)).expect("ends");
            fx.step();
        }
        rope(&fx, 4)
    };
    assert_eq!(run(), run());
}

#[test]
fn tether_released_rope_falls_to_the_floor_and_goes_away() {
    let mut fx = PhysFx::new(Tier::Comfort);
    fx.attach_tether(1, A, end(3.0), 4.0).expect("attach");
    hold(&mut fx, 1, A, end(3.0), 5);
    assert!(fx.release_tether(1));
    assert!(!fx.release_tether(9), "no such rope");
    fx.step();
    assert_eq!(fx.tether_info(1).expect("info").state, TetherState::Loose);
    step_n(&mut fx, 30);
    assert!(rope(&fx, 1).iter().any(|n| n[1] < 1.4), "falling");
    step_n(&mut fx, 300);
    assert_eq!(fx.tether_count(), 0, "settled, or out of time, and removed");
}

#[test]
fn tether_bad_input_is_refused_and_changes_nothing() {
    let mut fx = PhysFx::new(Tier::Comfort);
    fx.attach_tether(1, A, end(3.0), 4.0).expect("attach");
    let before = rope(&fx, 1);
    let nan = [f32::NAN, 0.0, 0.0];
    assert_eq!(
        fx.attach_tether(2, nan, A, 3.0),
        Err(Error::NonFinite(Field::Position))
    );
    assert_eq!(
        fx.attach_tether(2, A, end(1.0), f32::INFINITY),
        Err(Error::NonFinite(Field::Extent))
    );
    assert_eq!(
        fx.attach_tether(2, A, end(1.0), 0.0),
        Err(Error::OutOfRange(Field::Extent))
    );
    assert_eq!(
        fx.move_tether(1, nan, A),
        Err(Error::NonFinite(Field::Position))
    );
    assert_eq!(
        fx.tether_nodes(1, f32::NAN),
        Err(Error::NonFinite(Field::Alpha))
    );
    assert_eq!(fx.move_tether(7, A, A), Ok(false));
    assert_eq!(fx.tether_count(), 1);
    assert_eq!(rope(&fx, 1), before);
}

#[test]
fn tether_pull_is_a_spring_with_a_damper_and_never_pushes() {
    let mut fx = PhysFx::new(Tier::Comfort);
    fx.attach_tether(1, A, end(3.0), 4.0).expect("attach");
    hold(&mut fx, 1, A, end(3.0), 3);
    assert_eq!(
        fx.tether_info(1).expect("info").pull,
        [0.0; 3],
        "slack pulls nothing"
    );
    // Still ends, 1 m past the length: the spring alone, along +x from A toward B.
    hold(&mut fx, 1, A, end(5.0), 30);
    let held = fx.tether_info(1).expect("info").pull;
    assert!(
        (held[0] - 400.0).abs() < 1.0 && held[1].abs() < 1e-3,
        "{held:?}"
    );
    // The ends still moving apart add the damper on top of the spring.
    fx.move_tether(1, A, end(5.2)).expect("ends");
    fx.step();
    let stretching = fx.tether_info(1).expect("info").pull[0];
    assert!(stretching > held[0] + 100.0, "{stretching}");
    // Closing on a barely taut rope would push: it pulls nothing instead.
    fx.move_tether(1, A, end(4.01)).expect("ends");
    fx.step();
    assert!(fx.tether_info(1).expect("info").pull[0] >= 0.0);
}

#[test]
fn tether_carry_drags_a_ragdoll_along_the_track_and_the_limbs_trail() {
    let (rig, pose) = humanoid();
    let mut fx = PhysFx::new(Tier::Comfort);
    fx.spawn_ragdoll(0, &rig, &moved(&pose, [0.0, 3.0, 0.0]), [0.0; 3], [0.0; 3])
        .expect("ragdoll");
    assert_eq!(
        fx.carry_ragdoll(5, A, [0.0; 3], true),
        Ok(false),
        "no ragdoll in slot 5"
    );
    let pelvis = |fx: &PhysFx| bones_of(fx, 0)[0].at.pos;
    let hand = |fx: &PhysFx| bones_of(fx, 0)[7].at.pos;
    // The track runs along +x at 12 m/s, level.
    let mut trail = 0.0f32;
    for tick in 0..90 {
        let track = [12.0 * tick as f32 / 60.0, 4.1, 0.0];
        assert_eq!(fx.carry_ragdoll(0, track, [12.0, 0.0, 0.0], true), Ok(true));
        fx.step();
        if tick == 30 {
            trail = pelvis(&fx)[0] - hand(&fx)[0];
        }
    }
    let track = [12.0 * 89.0 / 60.0, 4.1, 0.0];
    let at = pelvis(&fx);
    assert!(
        (0..3).all(|k| (at[k] - track[k]).abs() < 0.25),
        "{at:?} against {track:?}"
    );
    assert!(trail > 0.0, "the hand lags behind the pelvis: {trail}");
    assert_eq!(
        fx.carry_ragdoll(0, [0.0, f32::NAN, 0.0], [0.0; 3], true),
        Err(Error::NonFinite(Field::Position))
    );
}

#[test]
fn tether_retune_reels_rope_in_and_out_and_a_free_carry_lets_the_body_fall() {
    let mut fx = PhysFx::new(Tier::Comfort);
    fx.attach_tether(1, A, end(3.0), 4.0).expect("attach");
    hold(&mut fx, 1, A, end(3.0), 3);
    assert_eq!(fx.tether_info(1).expect("info").state, TetherState::Slack);
    assert_eq!(fx.retune_tether(1, 2.5), Ok(true));
    hold(&mut fx, 1, A, end(3.0), 1);
    assert_eq!(fx.tether_info(1).expect("info").state, TetherState::Taut);
    assert_eq!(fx.retune_tether(9, 2.5), Ok(false));
    assert_eq!(
        fx.retune_tether(1, f32::NAN),
        Err(Error::NonFinite(Field::Extent))
    );

    let (rig, pose) = humanoid();
    let mut fx = arena(Tier::Comfort);
    fx.spawn_ragdoll(0, &rig, &moved(&pose, [0.0, 3.0, 0.0]), [0.0; 3], [0.0; 3])
        .expect("ragdoll");
    assert_eq!(fx.ragdoll_speed(4), None);
    assert_eq!(fx.ragdoll_root(4), None);
    let start = fx.ragdoll_root(0).expect("root");
    assert!((start[1] - 4.1).abs() < 0.2, "{start:?}");
    let height = |fx: &PhysFx| bones_of(fx, 0)[0].at.pos[1];
    for _ in 0..60 {
        fx.carry_ragdoll(0, [0.0, 4.1, 0.0], [0.0; 3], false)
            .expect("carry");
        fx.step();
    }
    assert!(height(&fx) < 1.5, "nothing holds it up: {}", height(&fx));
    step_n(&mut fx, 240);
    assert!(fx.ragdoll_speed(0).expect("speed") < 0.2, "it lay down");
}
