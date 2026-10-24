//! Real-body proofs of the first-person slot: the head hidden, the mount points on the posed
//! head, and the view laws held over a whole duel. No window; the same pose path the window draws.
use super::*;
use crate::fighter_look::fighters_content_dir;
use crate::fps_view::pov::{CENTRE_FRAMES_MAX, COVER_MAX, EYE_NEAR_M};

const FOV_Y: f32 = 1.047_197_5;
const ASPECT: f32 = 16.0 / 9.0;

/// A seed-2 duel of two on the real CT7 bodies, ready to tick.
fn duel() -> (crate::App, FighterLook) {
    let config = crate::GameConfig {
        seed: 2,
        ticks: 300,
        fighters: 2,
        half_extent_m: 20.0,
    };
    let app = crate::App::new(config)
        .and_then(crate::App::into_duel)
        .expect("duel builds");
    let mut look = FighterLook::load(&fighters_content_dir()).expect("CT7 bodies load");
    look.set_ragdoll(false);
    (app, look)
}

/// Fold one more tick of the duel into the look.
fn step(app: &mut crate::App, look: &mut FighterLook, tick: u32) {
    let out = app.tick_empty();
    look.observe_hitstop(tick, &out.cues, app.hitstop_remaining());
    look.observe_tick(tick, app.fighter_views(), app.hp(), &out.cues);
    look.observe_live_balances(app.live_match(), 2);
}

/// The first-person lens of a posed wearer: at the head's eye point, along the body's facing.
fn lens_of(body: &BodyTruth) -> PovLens {
    let pov = body.pov.as_deref().expect("the wearer carries its pose");
    let eye = pov.head_eye_m;
    let (s, c) = body.yaw.sin_cos();
    PovLens::look_at(
        eye,
        [eye[0] + s, eye[1], eye[2] + c],
        [0.0, 1.0, 0.0],
        FOV_Y,
        ASPECT,
        EYE_NEAR_M,
    )
    .expect("a sane lens")
}

fn nearest_to(positions: &[[f32; 3]], eye: [f32; 3]) -> f32 {
    positions
        .iter()
        .map(|p| {
            ((p[0] - eye[0]).powi(2) + (p[1] - eye[1]).powi(2) + (p[2] - eye[2]).powi(2)).sqrt()
        })
        .fold(f32::MAX, f32::min)
}

/// What a seed-2 duel does to the picture of the wearer's own eye: near plane cuts, the most
/// own geometry covers of the picture, the longest run of frames with own geometry on the
/// screen centre, the nearest own vertex and the frames the wearer swings.
struct DuelCover {
    cut: u32,
    share_max: f32,
    run_max: u32,
    nearest_m: f32,
    attack_frames: u32,
    /// The body part the nearest own vertex belongs to, for a failure message.
    nearest_part: String,
}

fn duel_cover() -> DuelCover {
    let (mut app, mut look) = duel();
    look.set_pov_slot(Some(0));
    let (mut cut, mut share_max, mut run, mut run_max) = (0u32, 0.0f32, 0u32, 0u32);
    let mut nearest = f32::MAX;
    let mut nearest_part = String::from("none");
    let mut attack_frames = 0u32;
    for tick in 0..300u32 {
        step(&mut app, &mut look, tick);
        for alpha in [0.25f32, 0.75] {
            let bodies: Vec<BodyTruth> = look
                .pose_frame(2, alpha)
                .expect("pose")
                .into_iter()
                .map(|posed| posed.truth)
                .collect();
            let me = bodies.iter().find(|body| body.slot == 0).expect("slot 0");
            let rival = bodies.iter().find(|body| body.slot == 1).expect("slot 1");
            assert!(me.pov.is_some(), "tick {tick}: the wearer has no pose");
            assert!(rival.pov.is_none(), "tick {tick}: the rival carries a pose");
            let lens = lens_of(me);
            let cover = look.pov_cover_of(&bodies, 0, lens).expect("a cover");
            cut += cover.near_cut;
            share_max = share_max.max(cover.share);
            run = if cover.centre_m.is_some() { run + 1 } else { 0 };
            run_max = run_max.max(run);
            let pov = me.pov.as_deref().expect("pose");
            let rig = &look.rigs[look.rig_for_slot(0)];
            let mut from = 0usize;
            for part in &rig.parts {
                let to = from + part.mesh.positions.len();
                if let Some(points) = pov.positions.get(from..to) {
                    let near = nearest_to(points, pov.head_eye_m);
                    if near < nearest {
                        nearest = near;
                        nearest_part = part.name.to_string();
                    }
                }
                from = to;
            }
            attack_frames += u32::from(me.act == "attack");
        }
    }
    DuelCover {
        cut,
        share_max,
        run_max,
        nearest_m: nearest,
        attack_frames,
        nearest_part,
    }
}

#[test]
fn pov_the_wearers_head_is_hidden_and_the_near_plane_cuts_nothing_over_a_real_duel() {
    let seen = duel_cover();
    eprintln!(
        "pov duel: near_cut={} share_max={:.3} centre_run_max={} nearest_m={:.3} nearest_part={} attack_frames={}",
        seen.cut, seen.share_max, seen.run_max, seen.nearest_m, seen.nearest_part, seen.attack_frames
    );
    assert!(
        seen.attack_frames > 0,
        "the duel never swung: the law saw no sword"
    );
    assert_eq!(
        seen.cut, 0,
        "the near plane cut own geometry (nearest {} at {:.3} m)",
        seen.nearest_part, seen.nearest_m
    );
    assert!(
        seen.nearest_m >= 0.10,
        "own geometry {} m from the eye (part {}): the head is not hidden",
        seen.nearest_m,
        seen.nearest_part
    );
}

/// The look audit's bar for the first person: own geometry covers at most [`COVER_MAX`] of the
/// picture and sits on the screen centre for at most [`CENTRE_FRAMES_MAX`] frames in a row.
/// Not met yet: the first run on the real bodies read 74 percent cover and the centre covered
/// 496 of 600 frames, with the head hidden and no near plane cut. What is left in front of the
/// eye is most likely the grafted shield and sword at their body places (not yet told apart per
/// part). A gear view-model (shield low on the left, sword low on the right, smaller than the
/// body's own) is the work that turns this green; `ViewLaws::note_frame_pov` already fails the
/// window run on the same numbers.
#[test]
#[ignore = "POV view-model: the real shield and sword still cover the first-person picture"]
fn pov_the_wearers_gear_keeps_the_picture_and_the_centre_clear_over_a_real_duel() {
    let seen = duel_cover();
    assert!(
        seen.share_max <= COVER_MAX,
        "own geometry covered {} of the picture",
        seen.share_max
    );
    assert!(
        seen.run_max <= CENTRE_FRAMES_MAX,
        "the centre was covered {} frames in a row",
        seen.run_max
    );
}

#[test]
fn pov_without_the_hide_the_head_stands_around_the_eye() {
    // The control: the same tick posed with and without the first-person slot. The head's
    // vertices ring the eye point until the hide collapses them.
    let (mut app, mut look) = duel();
    for tick in 0..40u32 {
        step(&mut app, &mut look, tick);
    }
    look.set_pov_slot(Some(0));
    let hidden = look.pose_frame(2, 0.5).expect("pose");
    let eye = hidden[0].truth.pov.as_deref().expect("pose").head_eye_m;
    look.set_pov_slot(None);
    let shown = look.pose_frame(2, 0.5).expect("pose");
    assert!(shown[0].truth.pov.is_none(), "no pose without the slot");
    let head_dist = shown[0]
        .frames
        .iter()
        .map(|frame| nearest_to(&frame.positions, eye))
        .fold(f32::MAX, f32::min);
    assert!(
        head_dist < 0.12,
        "the head must ring the eye point, nearest {head_dist} m"
    );
    let hidden_dist = hidden[0]
        .frames
        .iter()
        .map(|frame| nearest_to(&frame.positions, eye))
        .fold(f32::MAX, f32::min);
    assert!(hidden_dist >= 0.10, "hidden, nearest {hidden_dist} m");
}

#[test]
fn pov_the_rival_and_the_pose_stay_as_they_were() {
    let (mut app, mut look) = duel();
    for tick in 0..60u32 {
        step(&mut app, &mut look, tick);
    }
    let plain = look.pose_frame(2, 0.5).expect("pose");
    look.set_pov_slot(Some(0));
    let first = look.pose_frame(2, 0.5).expect("pose");
    let again = look.pose_frame(2, 0.5).expect("pose");
    // The slot that does not wear the eye draws exactly as before.
    for (a, b) in plain[1].frames.iter().zip(&first[1].frames) {
        assert_eq!(a.positions, b.positions, "the rival's vertices moved");
    }
    assert_eq!(
        plain[1].matrices, first[1].matrices,
        "the rival's joint block moved"
    );
    // The wearer's torso and legs draw exactly as before: only the head joint's block changed.
    let changed: Vec<usize> = plain[0]
        .matrices
        .iter()
        .zip(&first[0].matrices)
        .enumerate()
        .filter(|(_, (a, b))| a != b)
        .map(|(joint, _)| joint)
        .collect();
    assert_eq!(
        changed.len(),
        1,
        "only one joint block may change, got {changed:?}"
    );
    // The same pose twice gives the same body bytes.
    assert_eq!(
        first[0].truth.pov, again[0].truth.pov,
        "the pose replay split"
    );
    // Taking the slot away gives the plain body back.
    look.set_pov_slot(None);
    let back = look.pose_frame(2, 0.5).expect("pose");
    assert_eq!(plain[0].matrices, back[0].matrices);
    assert!(back[0].truth.pov.is_none());
}

#[test]
fn pov_the_mount_points_ride_the_head_and_the_chest_of_the_pose() {
    let (mut app, mut look) = duel();
    for tick in 0..30u32 {
        step(&mut app, &mut look, tick);
    }
    look.set_pov_slot(Some(0));
    let posed = look.pose_frame(2, 0.5).expect("pose");
    let me = &posed[0].truth;
    let pov = me.pov.as_deref().expect("pose");
    // The eye sits over the feet at about the model's eye height, in front of the head joint.
    let up = pov.head_eye_m[1] - pov.root_m[1];
    assert!((1.4..1.9).contains(&up), "the eye is {up} m over the feet");
    let off = (pov.head_eye_m[0] - pov.root_m[0]).hypot(pov.head_eye_m[2] - pov.root_m[2]);
    assert!(off < 0.5, "the eye is {off} m off the root");
    // The chest camera sits lower and in front of the torso.
    assert!(
        pov.chest_cam_m[1] < pov.head_eye_m[1],
        "the chest is below the eye"
    );
    assert!(
        pov.chest_cam_m[1] > pov.root_m[1] + 0.8,
        "the chest camera sits over the hips"
    );
    assert!(
        !pov.positions.is_empty()
            && pov
                .positions
                .iter()
                .all(|p| p.iter().all(|v| v.is_finite()))
    );
}
