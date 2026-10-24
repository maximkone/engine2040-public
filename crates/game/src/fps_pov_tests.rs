//! Tests of the first-person rig: the helmet spring (POV2), the spawn facing, the cover law and
//! the reticle. Pure arrays; the law fold into the view line rides the `app` leg.
use super::*;
use std::f32::consts::FRAC_PI_2;

const ASPECT: f32 = 16.0 / 9.0;

fn lens(near: f32) -> PovLens {
    PovLens::look_at(
        [0.0; 3],
        [0.0, 0.0, 1.0],
        [0.0, 1.0, 0.0],
        1.0,
        ASPECT,
        near,
    )
    .expect("a sane lens")
}

fn cover_of(positions: &[[f32; 3]], indices: &[u32]) -> PovCover {
    let mut acc = CoverAcc::new(lens(EYE_NEAR_M));
    acc.add_part(positions, indices);
    acc.finish()
}

/// A wearer at `root` whose head and chest mount points both sit `rest + bob` over it.
fn body(root: [f32; 3], rest: [f32; 3], bob_m: f32) -> PovBody {
    let at = [
        root[0] + rest[0],
        root[1] + rest[1] + bob_m,
        root[2] + rest[2],
    ];
    PovBody {
        root_m: root,
        head_eye_m: at,
        chest_cam_m: at,
        positions: Vec::new(),
    }
}

/// One scripted head bob: a sine at `hz` and `amp_m` over 3 s at 60 frames a second, folded
/// through `mount`'s rig. Returns the picture's eye bits, the aim eye bits, the largest sway of the
/// picture off its rest eye and the largest lag off the rigid mount, both in meters.
fn bob_tape(mount: Mount, hz: f32, amp_m: f32) -> (Vec<[u32; 3]>, Vec<[u32; 3]>, f32, f32) {
    let feet = [1.0, 0.0, 2.0];
    let rest = [0.0, 1.62, 0.05];
    let mut rig = PovRig::new(mount).expect("a rig");
    let (mut views, mut aims) = (Vec::new(), Vec::new());
    let (mut sway, mut lag) = (0.0f32, 0.0f32);
    let flat_y = (feet[1] + rest[1]) * 1000.0;
    for frame in 0..180u16 {
        let t = f32::from(frame) / 60.0;
        let bob = amp_m * (std::f32::consts::TAU * hz * t).sin();
        let wearer = body(feet, rest, bob);
        let eye = rig
            .eye(feet, 0.4, Some(&wearer), 0, u32::from(frame), 1.0 / 60.0)
            .expect("a frame");
        views.push(eye.view.eye_mm.map(f32::to_bits));
        aims.push(eye.aim.eye_mm.map(f32::to_bits));
        if frame >= 60 {
            sway = sway.max((eye.view.eye_mm[1] - flat_y).abs() / 1000.0);
            lag = lag.max(eye.lag_m);
        }
    }
    (views, aims, sway, lag)
}

#[test]
fn pov2_a_3cm_head_bob_moves_the_picture_over_1cm_and_the_aim_ray_by_nothing() {
    let (_, calm_aim, ..) = bob_tape(Mount::Helmet, 2.0, 0.0);
    for mount in [Mount::Helmet, Mount::Chest] {
        let (_, aim, sway, _) = bob_tape(mount, 2.0, 0.03);
        assert!(sway > 0.01, "{mount:?}: the picture moved only {sway} m");
        assert_eq!(aim, calm_aim, "{mount:?}: the bob moved the aim ray");
    }
}

#[test]
fn pov2_the_chest_mount_shakes_more_than_the_helmet() {
    let (.., helmet) = bob_tape(Mount::Helmet, 2.0, 0.03);
    let (.., chest) = bob_tape(Mount::Chest, 2.0, 0.03);
    assert!(
        chest > helmet,
        "chest lag {chest} m must pass helmet lag {helmet} m (halflives {CHEST_HALFLIFE_S} and {HELMET_HALFLIFE_S} s)"
    );
    assert!(helmet > 0.0, "the helmet mount is sprung too");
}

#[test]
fn pov2_the_same_tape_gives_the_same_camera_bytes_twice() {
    for mount in [Mount::Helmet, Mount::Chest] {
        let first = bob_tape(mount, 2.0, 0.03);
        let second = bob_tape(mount, 2.0, 0.03);
        assert_eq!(first.0, second.0, "{mount:?}: the picture replay split");
        assert_eq!(first.1, second.1, "{mount:?}: the aim replay split");
    }
}

#[test]
fn pov2_the_picture_sits_on_the_heads_real_eye_point_and_a_run_drags_nothing() {
    let mut rig = PovRig::new(Mount::Helmet).expect("a rig");
    let rest = [0.0, 1.62, 0.05];
    // A run at 8 m/s along x: the head moves with the root, so the offset to the root is
    // constant and the picture must sit exactly on the head, never behind it.
    for frame in 0..90u16 {
        let root = [f32::from(frame) * (8.0 / 60.0), 0.0, 0.0];
        let wearer = body(root, rest, 0.0);
        let eye = rig
            .eye(root, 0.0, Some(&wearer), 0, u32::from(frame), 1.0 / 60.0)
            .expect("a frame");
        for lane in 0..3 {
            let want = (root[lane] + rest[lane]) * 1000.0;
            assert!(
                (eye.view.eye_mm[lane] - want).abs() < 0.05,
                "frame {frame} lane {lane}: {} vs {want}",
                eye.view.eye_mm[lane]
            );
        }
        assert!(eye.lag_m < 1e-4, "frame {frame}: a run dragged the picture");
    }
}

#[test]
fn pov2_a_respawn_snaps_and_a_stalled_clock_holds() {
    let mut spring = MountSpring::new(Mount::Helmet).expect("a spring");
    assert_eq!(
        spring.step([0.0, 1.6, 0.0], 1.0 / 60.0),
        Ok([0.0, 1.6, 0.0])
    );
    // A 10 m jump is a respawn: the first frame there is the mount itself.
    assert_eq!(
        spring.step([10.0, 1.6, 0.0], 1.0 / 60.0),
        Ok([10.0, 1.6, 0.0])
    );
    // A stalled clock holds the last value instead of dividing by nothing.
    assert_eq!(spring.step([10.0, 1.7, 0.0], 0.0), Ok([10.0, 1.6, 0.0]));
    assert_eq!(
        spring.step([10.0, 1.7, 0.0], f32::NAN),
        Ok([10.0, 1.6, 0.0])
    );
    // A mount that is not a number refuses by name.
    assert_eq!(
        spring.step([f32::NAN, 0.0, 0.0], 1.0 / 60.0),
        Err(CameraError::BadParam("POV2 mount"))
    );
    // The first frame after a reset snaps again.
    spring.reset();
    assert_eq!(
        spring.step([2.0, 1.6, 0.0], 1.0 / 60.0),
        Ok([2.0, 1.6, 0.0])
    );
}

#[test]
fn pov2_trauma_kicks_the_picture_and_leaves_the_aim_ray_alone() {
    let feet = [0.0, 0.0, 0.0];
    let wearer = body(feet, [0.0, 1.62, 0.05], 0.0);
    let mut calm = PovRig::new(Mount::Helmet).expect("a rig");
    let mut hit = PovRig::new(Mount::Helmet).expect("a rig");
    let still = calm
        .eye(feet, 0.3, Some(&wearer), 0, 7, 1.0 / 60.0)
        .expect("calm");
    let shaken = hit
        .eye(feet, 0.3, Some(&wearer), 600, 7, 1.0 / 60.0)
        .expect("hit");
    assert_ne!(
        shaken.view.eye_mm, still.view.eye_mm,
        "the kick must move the picture"
    );
    assert!(shaken.view.roll_rad > 0.0, "the kick must roll the picture");
    assert_eq!(shaken.aim, still.aim, "the kick moved the aim ray");
    assert_eq!(shaken.aim.roll_rad, 0.0, "the aim ray never rolls");
    // The aim ray is the FV1 eye: feet plus the eye height, along the facing.
    let (origin, dir) = aim_ray(&shaken.aim);
    assert_eq!(origin, [0.0, 1.6, 0.0], "FV1 eye height");
    assert!((dir[0] - 0.3f32.sin()).abs() < 1e-6 && (dir[2] - 0.3f32.cos()).abs() < 1e-6);
}

#[test]
fn pov2_without_a_body_the_picture_takes_the_fv1_eye() {
    let mut rig = PovRig::new(Mount::Helmet).expect("a rig");
    let eye = rig
        .eye([2.0, 0.0, 3.0], 0.0, None, 0, 1, 1.0 / 60.0)
        .expect("a frame");
    assert_eq!(eye.view, eye.aim, "no posed body, no sprung picture");
    assert_eq!(eye.lag_m, 0.0);
    // A bad position refuses by name, never a NaN picture.
    let err = rig.eye([f32::NAN, 0.0, 0.0], 0.0, None, 0, 1, 1.0 / 60.0);
    assert!(matches!(err, Err(CameraError::BadParam(row)) if row.contains("FV1")));
}

#[test]
fn spawn_yaw_faces_the_rival_the_sim_swings_at() {
    // The lock-on bearing in centidegrees, 0 is +z and +9000 is +x.
    let yaw = spawn_yaw([19.0, 0.0, 3.0], 0.0, Some(9_000), 20.0);
    assert!(
        (yaw - FRAC_PI_2).abs() < 1e-6,
        "9000 cdeg is a quarter turn: {yaw}"
    );
    let yaw = spawn_yaw([0.0, 0.0, 0.0], 1.0, Some(-4_500), 20.0);
    assert!((yaw + FRAC_PI_2 * 0.5).abs() < 1e-6, "-4500 cdeg: {yaw}");
}

#[test]
fn spawn_yaw_never_stares_into_a_wall() {
    let half = 20.0;
    for xi in -6i16..=6 {
        for zi in -6i16..=6 {
            let feet = [f32::from(xi) * 3.1, 0.0, f32::from(zi) * 3.1];
            for step in 0..24u16 {
                let body_yaw = f32::from(step) * std::f32::consts::TAU / 24.0 - PI;
                let yaw = spawn_yaw(feet, body_yaw, None, half);
                let room = wall_distance_m(feet, yaw, half);
                assert!(
                    room >= WALL_CLEAR_M,
                    "feet {feet:?} body yaw {body_yaw}: {room} m of floor ahead"
                );
            }
        }
    }
    // A yaw with open floor ahead is the body's own, untouched.
    let open = spawn_yaw([0.0, 0.0, 0.0], 0.7, None, 20.0);
    assert_eq!(open, 0.7);
    // The audit's spawn stood 0.8 m off the east wall: facing the wall it turns west, facing
    // along the wall (open floor ahead) it keeps its yaw.
    let spawn = spawn_yaw([19.18, 0.0, 3.44], FRAC_PI_2, None, 20.0);
    assert!(
        spawn < -FRAC_PI_2 * 0.5,
        "the east wall spawn turns west: {spawn}"
    );
    assert_eq!(spawn_yaw([19.18, 0.0, 3.44], 0.0, None, 20.0), 0.0);
    // A body yaw that is not a number reads as +z.
    assert_eq!(spawn_yaw([0.0; 3], f32::NAN, None, 20.0), 0.0);
}

#[test]
fn the_wall_distance_reads_each_wall_and_open_directions() {
    let at = [5.0, 0.0, -4.0];
    assert!(
        (wall_distance_m(at, 0.0, 20.0) - 24.0).abs() < 1e-4,
        "+z wall"
    );
    assert!(
        (wall_distance_m(at, FRAC_PI_2, 20.0) - 15.0).abs() < 1e-4,
        "+x wall"
    );
    assert!(
        (wall_distance_m(at, -FRAC_PI_2, 20.0) - 25.0).abs() < 1e-4,
        "-x wall"
    );
    assert!(
        (wall_distance_m(at, PI, 20.0) - 16.0).abs() < 1e-3,
        "-z wall"
    );
}

#[test]
fn law_a_clear_view_passes() {
    // A triangle far to the side of the view, past the near plane.
    let tri = [[3.0, 0.0, 1.0], [3.0, 0.5, 1.0], [3.5, 0.0, 1.0]];
    let cover = cover_of(&tri, &[0, 1, 2]);
    assert_eq!(cover.near_cut, 0);
    assert_eq!(cover.centre_m, None);
    assert!(!cover.broken(), "{cover:?}");
}

#[test]
fn law_a_triangle_the_near_plane_cuts_inside_the_picture_fails() {
    // The shard of a camera inside a body: one corner behind the plane, two ahead.
    let tri = [[-0.02, -0.02, 0.05], [0.02, -0.02, 0.12], [0.0, 0.03, 0.10]];
    let cover = cover_of(&tri, &[0, 1, 2]);
    assert_eq!(cover.near_cut, 1, "{cover:?}");
    assert!(cover.broken());
    // The same triangle past the plane is no cut.
    let past = [[-0.02, -0.02, 0.20], [0.02, -0.02, 0.22], [0.0, 0.03, 0.21]];
    assert_eq!(cover_of(&past, &[0, 1, 2]).near_cut, 0);
    // A cut far outside the picture is no shard either.
    let wide = [[9.0, 0.0, 0.05], [9.0, 0.5, 0.12], [9.5, 0.0, 0.10]];
    assert_eq!(cover_of(&wide, &[0, 1, 2]).near_cut, 0);
}

#[test]
fn law_geometry_behind_the_eye_or_wholly_clipped_is_ignored() {
    let behind = [[0.0, 0.0, -1.0], [0.1, 0.0, -1.0], [0.0, 0.1, -1.0]];
    assert_eq!(cover_of(&behind, &[0, 1, 2]), PovCover::default());
    let clipped = [[0.0, 0.0, 0.02], [0.01, 0.0, 0.03], [0.0, 0.01, 0.04]];
    assert_eq!(cover_of(&clipped, &[0, 1, 2]), PovCover::default());
}

#[test]
fn law_a_slab_across_the_view_covers_the_centre_and_the_picture() {
    let quad = [
        [-3.0, -3.0, 0.5],
        [3.0, -3.0, 0.5],
        [3.0, 3.0, 0.5],
        [-3.0, 3.0, 0.5],
    ];
    let cover = cover_of(&quad, &[0, 1, 2, 0, 2, 3]);
    assert!(cover.share > 0.99, "{cover:?}");
    let centre = cover.centre_m.expect("the centre is covered");
    assert!((centre - 0.5).abs() < 1e-4, "{centre}");
    assert!(cover.broken());
}

#[test]
fn law_a_thin_blade_on_the_centre_covers_the_centre_and_not_the_picture() {
    let blade = [
        [-0.01, -0.5, 1.0],
        [0.01, -0.5, 1.0],
        [0.01, 0.5, 1.0],
        [-0.01, 0.5, 1.0],
    ];
    let cover = cover_of(&blade, &[0, 1, 2, 0, 2, 3]);
    assert!((cover.centre_m.expect("on the centre") - 1.0).abs() < 1e-4);
    assert!(cover.share < COVER_MAX, "{cover:?}");
    assert!(!cover.broken(), "one frame of blade is not a broken frame");
}

#[test]
fn law_bad_indices_and_corners_are_skipped_not_trusted() {
    let tri = [[0.0, 0.0, 1.0], [f32::NAN, 0.0, 1.0], [0.0, 1.0, 1.0]];
    assert_eq!(cover_of(&tri, &[0, 1, 2]), PovCover::default());
    assert_eq!(cover_of(&tri, &[0, 9, 2]), PovCover::default());
    assert_eq!(
        cover_of(&tri, &[0, 1]),
        PovCover::default(),
        "a stray index pair"
    );
}

#[test]
fn lens_refuses_a_camera_it_cannot_judge() {
    let ok = PovLens::look_at([0.0; 3], [0.0, 0.0, 1.0], [0.0, 1.0, 0.0], 1.0, ASPECT, 0.1);
    assert!(ok.is_some());
    for bad in [
        PovLens::look_at(
            [f32::NAN, 0.0, 0.0],
            [0.0, 0.0, 1.0],
            [0.0, 1.0, 0.0],
            1.0,
            ASPECT,
            0.1,
        ),
        PovLens::look_at([0.0; 3], [0.0; 3], [0.0, 1.0, 0.0], 1.0, ASPECT, 0.1),
        PovLens::look_at([0.0; 3], [0.0, 1.0, 0.0], [0.0, 1.0, 0.0], 1.0, ASPECT, 0.1),
        PovLens::look_at([0.0; 3], [0.0, 0.0, 1.0], [0.0, 1.0, 0.0], 0.0, ASPECT, 0.1),
        PovLens::look_at([0.0; 3], [0.0, 0.0, 1.0], [0.0, 1.0, 0.0], 1.0, 0.0, 0.1),
        PovLens::look_at([0.0; 3], [0.0, 0.0, 1.0], [0.0, 1.0, 0.0], 1.0, ASPECT, 0.0),
    ] {
        assert!(bad.is_none(), "{bad:?}");
    }
}

#[test]
fn the_reticle_is_square_centred_and_leaves_the_centre_open() {
    for aspect in [1.0, ASPECT, 2.4] {
        let rects = reticle_rects(aspect);
        for rect in rects {
            assert!(rect.iter().all(|v| (0.0..=1.0).contains(v)), "{rect:?}");
            assert!(rect[0] < rect[2] && rect[1] < rect[3], "{rect:?}");
        }
        // Symmetric about the centre, left against right and top against bottom.
        assert!(
            (rects[0][0] - (1.0 - rects[2][2])).abs() < 1e-6,
            "ticks mirror"
        );
        assert!(
            (rects[4][1] - (1.0 - rects[6][3])).abs() < 1e-6,
            "ticks mirror"
        );
        // The dot (last element) is square in pixels: width times aspect is the height.
        let dot = rects[9];
        assert!(((dot[2] - dot[0]) * aspect - (dot[3] - dot[1])).abs() < 1e-6);
        // The gap between the dot and the nearest tick is open.
        assert!(rects[2][0] > dot[2], "the right tick starts past the dot");
    }
    // Garbage aspects read as square instead of poisoning the overlay.
    assert_eq!(reticle_rects(f32::NAN), reticle_rects(1.0));
    assert_eq!(reticle_rects(-3.0), reticle_rects(1.0));
}

#[cfg(feature = "app")]
mod view_law_fold {
    use super::*;
    use crate::view_truth::ViewLaws;
    use engine_render::v1::Camera;

    fn camera() -> Camera {
        Camera::new([0.0; 3], [0.0, 0.0, 1.0], [0.0, 1.0, 0.0], 1.0)
    }

    fn fold(laws: &mut ViewLaws, cover: &PovCover) {
        laws.note_frame_pov(&camera(), 1.0, &[], 0, cover);
    }

    #[test]
    fn a_cut_near_plane_fails_the_frame_through_the_eye_in_body_floor() {
        let mut laws = ViewLaws::default();
        fold(&mut laws, &PovCover::default());
        assert_eq!(laws.verdict(), "ok");
        fold(
            &mut laws,
            &PovCover {
                near_cut: 3,
                ..PovCover::default()
            },
        );
        assert_eq!(laws.verdict(), "FAIL(view_eye_in_body)");
        let line = laws.line().expect("two frames folded");
        assert!(line.contains("eye_in_body=1"), "{line}");
    }

    #[test]
    fn a_cover_over_30_percent_fails_one_frame_once() {
        let mut laws = ViewLaws::default();
        let big = PovCover {
            share: 0.31,
            near_cut: 2,
            ..PovCover::default()
        };
        fold(&mut laws, &big);
        // Two reasons on one frame are still one frame.
        assert!(laws.line().expect("a frame").contains("eye_in_body=1"));
        fold(
            &mut laws,
            &PovCover {
                share: 0.30,
                ..PovCover::default()
            },
        );
        assert!(
            laws.line().expect("a frame").contains("eye_in_body=1"),
            "30 percent is the edge"
        );
    }

    #[test]
    fn the_centre_covered_past_the_run_fails_and_a_blink_does_not() {
        let covered = PovCover {
            centre_m: Some(0.6),
            ..PovCover::default()
        };
        let mut blink = ViewLaws::default();
        for _ in 0..CENTRE_FRAMES_MAX {
            fold(&mut blink, &covered);
        }
        fold(&mut blink, &PovCover::default());
        assert_eq!(blink.verdict(), "ok", "a run of exactly N frames passes");
        let mut held = ViewLaws::default();
        for _ in 0..CENTRE_FRAMES_MAX + 5 {
            fold(&mut held, &covered);
        }
        assert_eq!(held.verdict(), "FAIL(view_eye_in_body)");
        // Only the frames past the run count.
        assert!(held.line().expect("frames").contains("eye_in_body=5"));
        let pov = held.pov_line().expect("a first-person run prints its line");
        assert!(pov.contains("centre_run_max=41"), "{pov}");
        // A break in the run starts the count again.
        fold(&mut held, &PovCover::default());
        fold(&mut held, &covered);
        assert!(held.line().expect("frames").contains("eye_in_body=5"));
    }

    #[test]
    fn the_directors_frame_is_not_folded_while_the_first_person_plays() {
        let mut laws = ViewLaws::default();
        laws.set_pov_slot(Some(0));
        laws.note_frame(&camera(), 1.0, &[]);
        assert!(laws.line().is_none(), "a director frame was folded");
        fold(&mut laws, &PovCover::default());
        assert!(laws.line().is_some(), "the eye's own frame folds");
        laws.set_pov_slot(None);
        laws.note_frame(&camera(), 2.0, &[]);
        let line = laws.line().expect("frames folded");
        assert!(line.contains("frames=2 "), "{line}");
    }

    #[test]
    fn a_run_with_no_first_person_frame_prints_no_pov_line() {
        let mut laws = ViewLaws::default();
        laws.note_frame(&camera(), 1.0, &[]);
        assert!(laws.pov_line().is_none());
    }
}
