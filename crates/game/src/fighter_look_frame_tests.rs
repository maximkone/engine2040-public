//! Tests of `fighter_look_frame`: which bodies the fight camera frames and that the follower
//! keeps them in the picture however high the rope throws them.

use super::*;
use crate::fighter_look::tests::balance_look_two;
use crate::view_truth::BodyTruth;

fn base() -> Camera {
    Camera::new(
        [0.0, 5.0, 12.0],
        [0.0, 1.0, 0.0],
        [0.0, 1.0, 0.0],
        50.0f32.to_radians(),
    )
}

fn frame(fighters: usize, shot: Shot) -> LookFrame {
    LookFrame {
        fighters,
        alpha: 0.5,
        shot,
        frame: 0,
    }
}

/// Seat slot `slot` on the floor at `at`, the sim position of this tick and the last.
fn put(look: &mut FighterLook, slot: usize, at: [f32; 3]) {
    let seat = &mut look.slots[slot];
    seat.seen = true;
    seat.pos = at;
    seat.prev_pos = at;
}

/// Draw slot `slot` as a body standing with its feet at `at`, as the last pose left it.
fn draw(look: &mut FighterLook, slot: usize, at: [f32; 3]) {
    let id = u8::try_from(slot).expect("a look slot");
    look.truth.retain(|body| body.slot != id);
    let mut body = BodyTruth::new(id, "idle", at, 0.0, Vec::new());
    body.fold_vertices(&[
        [at[0] - 0.3, at[1], at[2] - 0.3],
        [at[0] + 0.3, at[1] + 1.8, at[2] + 0.3],
    ]);
    look.truth.push(body);
}

/// Four seen fighters: the hero and a rival close together, two more far off.
fn four() -> FighterLook {
    let mut look = balance_look_two();
    put(&mut look, 0, [-2.0, 0.0, 0.0]);
    put(&mut look, 1, [-1.0, 0.0, 0.0]);
    put(&mut look, 2, [6.0, 0.0, 2.0]);
    put(&mut look, 3, [7.0, 0.0, 2.0]);
    look
}

#[test]
fn a_hero_among_four_follows_his_pair_and_a_look_with_no_hero_keeps_the_group() {
    let mut look = four();
    let pair = look.action_pair(4, None).expect("a pair");
    assert_eq!((pair.primary, pair.other), (0, Some(1)));
    assert_eq!(look.framed_positions(4, Shot::Action, None).len(), 2);
    assert_eq!(
        look.framed_positions(4, Shot::Wide, None).len(),
        4,
        "only the fight shot"
    );
    look.slots[0].seen = false;
    assert_eq!(look.action_pair(4, None), None, "no hero, the group shot");
    assert_eq!(look.framed_positions(4, Shot::Action, None).len(), 3);
}

#[test]
fn two_bodies_keep_the_old_pair_and_one_body_stands_alone() {
    let mut look = balance_look_two();
    let pair = look.action_pair(2, None).expect("a pair");
    assert_eq!((pair.primary, pair.other), (0, Some(1)));
    look.slots[1].seen = false;
    let alone = look.action_pair(2, None).expect("a body");
    assert_eq!((alone.primary, alone.other), (0, None));
    look.slots[0].seen = false;
    assert_eq!(look.action_pair(2, None), None);
}

#[test]
fn a_threat_is_held_until_another_is_clearly_nearer() {
    let mut look = balance_look_two();
    put(&mut look, 0, [0.0, 0.0, 0.0]);
    put(&mut look, 1, [5.0, 0.0, 0.0]);
    put(&mut look, 2, [-4.2, 0.0, 0.0]);
    put(&mut look, 3, [0.0, 0.0, 30.0]);
    let first = look.action_pair(4, None).expect("a pair");
    assert_eq!(first.other, Some(2), "the nearest threat first");
    let held = look.action_pair(4, Some(1)).expect("a pair");
    assert_eq!(held.other, Some(1), "0.8 m nearer does not flip the shot");
    put(&mut look, 2, [-2.0, 0.0, 0.0]);
    let flipped = look.action_pair(4, Some(1)).expect("a pair");
    assert_eq!(flipped.other, Some(2), "3 m nearer does");
}

#[test]
fn a_standing_threat_beats_a_nearer_floored_one() {
    let mut look = balance_look_two();
    put(&mut look, 0, [0.0, 0.0, 0.0]);
    put(&mut look, 1, [2.0, 0.0, 0.0]);
    put(&mut look, 2, [6.0, 0.0, 0.0]);
    put(&mut look, 3, [0.0, 0.0, 12.0]);
    look.slots[1].act = Act::Down;
    let pair = look.action_pair(4, None).expect("a pair");
    assert_eq!(pair.other, Some(2));
}

#[test]
fn a_hero_floored_for_good_hands_the_camera_to_the_closest_standing_pair() {
    let mut look = four();
    put(&mut look, 3, [-9.0, 0.0, 5.0]);
    put(&mut look, 1, [10.0, 0.0, 0.0]);
    put(&mut look, 2, [12.0, 0.0, 1.0]);
    look.slots[0].act = Act::Down;
    look.slots[0].act_start = 0;
    look.tick = DOWN_HOLD_TICKS - 1;
    let still = look.action_pair(4, None).expect("a pair");
    assert_eq!(
        still.primary, 0,
        "a knockdown that may get up keeps the hero"
    );
    look.tick = DOWN_HOLD_TICKS;
    let spectate = look.action_pair(4, None).expect("a pair");
    assert_eq!((spectate.primary, spectate.other), (1, Some(2)));
    look.slots[2].act = Act::Down;
    look.slots[3].act = Act::Down;
    let lone = look.action_pair(4, None).expect("a pair");
    assert_eq!(lone.primary, 1, "one winner standing is the primary");
    assert_eq!(lone.other, Some(2), "beside its nearest neighbor");
}

#[test]
fn a_body_in_the_air_is_where_it_is_drawn() {
    let mut look = balance_look_two();
    let eye = Camera::new(
        [0.0, 1.9, 8.0],
        [0.0, 1.0, 0.0],
        [0.0, 1.0, 0.0],
        60.0f32.to_radians(),
    );
    put(&mut look, 0, [0.0, 0.0, 0.0]);
    look.slots[1].seen = false;
    assert_eq!(
        look.bodies_in_view(&eye, 2),
        1,
        "undrawn: the chest over the sim spot"
    );
    draw(&mut look, 0, [0.0, 9.0, 0.0]);
    assert_eq!(
        look.bodies_in_view(&eye, 2),
        0,
        "drawn 9 m up it is above the picture"
    );
    let middle = look.subject_of(0).map(|s| s.center()[1]).expect("seen");
    assert!((middle - 9.9).abs() < 1.0e-4, "{middle}");
    draw(&mut look, 0, [0.0, 0.0, 0.0]);
    assert_eq!(look.bodies_in_view(&eye, 2), 1);
}

#[test]
fn a_stray_vertex_does_not_widen_the_body_the_camera_keeps() {
    let mut look = balance_look_two();
    draw(&mut look, 0, [0.0; 3]);
    look.truth[0].fold_vertices(&[[-30.0, 0.0, 0.0], [30.0, 0.5, 0.0]]);
    let subject = look.subject_of(0).expect("seen");
    assert!(subject.max[0] - subject.min[0] <= 2.0 * camera_frame::REACH_M + 1.0e-4);
}

#[test]
fn the_follower_holds_two_bodies_in_picture_through_a_rope_swing() {
    // The rope throws both bodies 6 m up and back down over two seconds while the
    // sim's floor point stays put: the floor camera showed bare sand at the top.
    let mut look = balance_look_two();
    let mut follow = CamFollow::new().expect("follower builds");
    let shot = frame(2, Shot::Action);
    let (mut apex_look, mut worst) = (0.0f32, 1.0f32);
    let mut law = camera_frame::Law::new();
    for n in 0..120u8 {
        let height = 6.0 * (std::f32::consts::PI * f32::from(n) / 120.0).sin();
        draw(&mut look, 0, [-1.0, height, 0.0]);
        draw(&mut look, 1, [1.0, height * 0.8, 0.0]);
        look.tick = u32::from(n);
        let camera = look.aim_camera(&mut follow, &base(), &shot, 1.0 / 60.0);
        let verdict = look
            .frame_verdict(&follow, &camera, &shot)
            .expect("two bodies framed");
        assert!(
            verdict.in_safe,
            "frame {n} at {height:.1} m: bodies left the picture"
        );
        apex_look = apex_look.max(camera.target.y);
        worst = worst.min(verdict.size_max);
        law.note(&verdict);
    }
    assert!(
        apex_look > 2.0,
        "the look point followed them up: {apex_look}"
    );
    // The springs lag a throw for a few frames, so the biggest body may dip a little under
    // the floor; the law allows a run of them, not a long one.
    assert!(worst >= 0.1, "and kept them readable: {worst}");
    assert_eq!(
        law.failure(),
        None,
        "the framing law holds: {}",
        law.fields()
    );
}

#[test]
fn the_first_frame_with_a_body_in_the_air_is_still_the_stateless_shot() {
    let mut look = balance_look_two();
    draw(&mut look, 0, [-1.0, 5.0, 0.0]);
    draw(&mut look, 1, [1.0, 5.5, 0.0]);
    let raw = look.fight_camera(&base(), Shot::Action, 2, 0);
    let mut follow = CamFollow::new().expect("follower builds");
    let first = look.aim_camera(&mut follow, &base(), &frame(2, Shot::Action), 1.0 / 60.0);
    assert_eq!(first, raw);
    let read = look.frame_read(&raw, &[0, 1]).expect("in front");
    assert!(read.verdict(2).in_safe, "{read:?}");
}

#[test]
fn the_wide_establishing_shot_is_left_as_directed_and_not_judged() {
    let mut look = balance_look_two();
    draw(&mut look, 0, [-1.0, 5.0, 0.0]);
    let follow = CamFollow::new().expect("follower builds");
    let wide = frame(2, Shot::Wide);
    assert_eq!(look.frame_verdict(&follow, &base(), &wide), None);
    assert_eq!(look.frame_still(base(), Shot::Wide, 2), base());
}

#[test]
fn a_pair_lost_behind_the_eye_reads_as_lost() {
    let mut look = balance_look_two();
    draw(&mut look, 0, [0.0, 0.0, 40.0]);
    draw(&mut look, 1, [1.0, 0.0, 40.0]);
    let follow = CamFollow::new().expect("follower builds");
    let verdict = look
        .frame_verdict(&follow, &base(), &frame(2, Shot::Action))
        .expect("judged");
    assert!(!verdict.in_safe);
    assert_eq!(verdict.size_max, 0.0);
}
