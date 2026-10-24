use engine_render::v1::Camera;

use super::*;
use crate::fighter_look::Shot;
use crate::scene_report::geom::sub;

fn frame() -> Camera {
    let mut camera = Camera::new(
        [0.0, 5.0, 12.0],
        [0.0, 1.0, 0.0],
        [0.0, 1.0, 0.0],
        50.0f32.to_radians(),
    );
    camera.aspect = 16.0 / 9.0;
    camera.far = 200.0;
    camera
}

const ROOTS: [(u8, P3); 2] = [(0, [-2.0, 0.0, 0.0]), (1, [2.0, 0.0, 0.0])];
const SUN: P3 = [-0.45, 0.75, 0.55];

#[test]
fn every_legal_line_parses_to_its_request() {
    let cases: [(&str, Request); 14] = [
        ("scene", Request::Scene { pixels: false }),
        ("scene pixels", Request::Scene { pixels: true }),
        (
            "steer shot auto",
            Request::Steer(Steer::Shot(ShotPick::Auto)),
        ),
        (
            "steer shot hero",
            Request::Steer(Steer::Shot(ShotPick::Director(Shot::Hero))),
        ),
        (
            "steer shot wide",
            Request::Steer(Steer::Shot(ShotPick::Director(Shot::Wide))),
        ),
        (
            "steer shot top",
            Request::Steer(Steer::Shot(ShotPick::Named("top".to_owned()))),
        ),
        (
            "steer shot close:1",
            Request::Steer(Steer::Shot(ShotPick::Named("close:1".to_owned()))),
        ),
        (
            "steer pose 1 2 3 4 5 6 40",
            Request::Steer(Steer::Pose {
                eye: [1.0, 2.0, 3.0],
                target: [4.0, 5.0, 6.0],
                fov_deg: Some(40.0),
            }),
        ),
        (
            "steer pass sun_shadow off",
            Request::Steer(Steer::Pass {
                name: "sun_shadow".to_owned(),
                on: false,
            }),
        ),
        (
            "steer capture",
            Request::Steer(Steer::Capture {
                tag: "shot".to_owned(),
            }),
        ),
        (
            "steer capture sag-1",
            Request::Steer(Steer::Capture {
                tag: "sag-1".to_owned(),
            }),
        ),
        ("steer hold", Request::Steer(Steer::Hold)),
        ("steer step", Request::Steer(Steer::Step)),
        ("steer undo", Request::Steer(Steer::Undo)),
    ];
    for (line, want) in cases {
        assert_eq!(parse_request(line).as_ref(), Ok(&want), "{line}");
    }
    assert_eq!(
        parse_request("  steer   resume  "),
        Ok(Request::Steer(Steer::Resume))
    );
    assert_eq!(
        parse_request("steer state"),
        Ok(Request::Steer(Steer::State))
    );
}

#[test]
fn a_bad_line_is_refused_with_the_forms_and_never_half_applied() {
    for (line, why) in [
        ("", "empty request"),
        ("look", "`look` is not a request"),
        ("scene all", "scene takes"),
        ("scene pixels now", "scene takes"),
        ("steer", "needs an operation"),
        ("steer jump", "no steer operation `jump`"),
        ("steer shot", "shot needs a name"),
        ("steer shot sideways", "no shot `sideways`"),
        ("steer shot close:x", "no shot `close:x`"),
        ("steer pose 1 2 3", "tx is missing"),
        ("steer pose 1 2 3 4 5 nope", "tz `nope` is not a number"),
        ("steer pose 1 2 3 4 5 6 0", "fov must be"),
        ("steer pose 1 2 3 4 5 99999", "outside"),
        ("steer pass fog", "on or off"),
        ("steer pass fog maybe", "on or off"),
        ("steer capture bad/tag", "capture tag"),
        (
            "steer capture aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "capture tag",
        ),
        ("steer hold now", "extra fields"),
    ] {
        let err = parse_request(line).expect_err(line).to_string();
        assert!(err.contains(why), "{line}: {err}");
        assert!(err.contains("the forms are:"), "{line}: {err}");
    }
}

#[test]
fn a_director_pin_forces_the_shot_and_a_named_pin_builds_a_camera_that_looks_at_the_pair() {
    let mut state = SteerState::default();
    assert_eq!(state.director_shot(), None);
    assert!(state.camera(&frame(), &ROOTS, SUN).is_none());
    state.pick(&ShotPick::Director(Shot::Wide));
    assert_eq!(state.director_shot(), Some(Shot::Wide));
    assert!(
        state.camera(&frame(), &ROOTS, SUN).is_none(),
        "the director still aims"
    );
    assert_eq!(state.pin_name(), "wide");
    state.pick(&ShotPick::Named("top".to_owned()));
    let top = state.camera(&frame(), &ROOTS, SUN).expect("top camera");
    let (eye, target) = (top.eye.to_array(), top.target.to_array());
    assert!(
        eye[1] > 10.0 && target[1].abs() < 1e-3,
        "{eye:?} -> {target:?}"
    );
    assert!(
        eye[0].abs() < 1e-3 && eye[2].abs() < 1e-3,
        "above the pair centre"
    );
    assert!((top.aspect - frame().aspect).abs() < 1e-6 && (top.far - 200.0).abs() < 1e-6);
}

#[test]
fn each_named_shot_is_a_valid_camera_and_the_sun_shot_sits_along_the_sun() {
    for name in ["top", "side", "low", "front", "sun", "close:0", "close:1"] {
        let camera = named_camera(name, &frame(), &ROOTS, SUN).unwrap_or_else(|| panic!("{name}"));
        assert!(camera.validate().is_ok(), "{name}");
        assert!(camera.eye.y > 0.0, "{name} stays above the sand");
    }
    let sun = named_camera("sun", &frame(), &ROOTS, SUN).expect("sun camera");
    let from_target = unit(sub(sun.eye.to_array(), sun.target.to_array())).expect("direction");
    let want = unit(SUN).expect("sun direction");
    let agree = from_target[0] * want[0] + from_target[1] * want[1] + from_target[2] * want[2];
    assert!(
        agree > 0.999,
        "the sun shot looks down the sun ray: {agree}"
    );
    assert!(
        named_camera("close:7", &frame(), &ROOTS, SUN).is_none(),
        "nobody in slot 7"
    );
    assert!(named_camera("diagonal", &frame(), &ROOTS, SUN).is_none());
    assert!(
        named_camera("sun", &frame(), &ROOTS, [0.0; 3]).is_none(),
        "no sun, no sun shot"
    );
}

#[test]
fn a_pose_pins_the_camera_and_undo_takes_back_pins_newest_first() {
    let mut state = SteerState::default();
    state.pose([0.0, 3.0, 8.0], [0.0, 1.0, 0.0], Some(30.0));
    let camera = state.camera(&frame(), &ROOTS, SUN).expect("pose camera");
    assert!((camera.fov_y.to_degrees() - 30.0).abs() < 1e-3);
    assert_eq!(state.pin_name(), "pose");
    state.pick(&ShotPick::Director(Shot::Hero));
    state.undo.push(Undo::Pass {
        name: "fog".to_owned(),
        was: true,
    });
    // The newest change is a pass: the caller gets it back, the pin stays.
    assert_eq!(
        state.undo(),
        Some(Undo::Pass {
            name: "fog".to_owned(),
            was: true
        })
    );
    assert_eq!(state.pin_name(), "hero");
    // Then the hero pin is taken back to the pose, then the pose to auto.
    assert!(matches!(state.undo(), Some(Undo::Pin(Pin::Pose { .. }))));
    assert_eq!(state.pin_name(), "pose");
    assert!(matches!(state.undo(), Some(Undo::Pin(Pin::Auto))));
    assert_eq!(state.pin_name(), "auto");
    assert_eq!(state.undo(), None, "nothing left");
}

#[test]
fn a_pose_with_the_eye_on_the_target_is_not_a_camera() {
    let mut state = SteerState::default();
    state.pose([1.0, 1.0, 1.0], [1.0, 1.0, 1.0], None);
    assert!(state.camera(&frame(), &ROOTS, SUN).is_none());
}

#[test]
fn captures_land_under_the_capture_dir_with_the_tag_and_tick_and_replies_are_one_json_line() {
    let path = capture_path("sag", 120, 2);
    assert_eq!(path, PathBuf::from("target/scene-captures/sag-t120-2.png"));
    let ok = reply_ok(
        "pass",
        120,
        &[("name", q("fog")), ("on", "false".to_owned())],
    );
    assert_eq!(
        ok,
        "{\"ok\":true,\"op\":\"pass\",\"tick\":120,\"name\":\"fog\",\"on\":false}"
    );
    let err = reply_err("steer", "bad_steer", "no \"such\" pass");
    assert_eq!(
        err,
        "{\"ok\":false,\"op\":\"steer\",\"code\":\"bad_steer\",\"why\":\"no \\\"such\\\" pass\"}"
    );
    assert!(!ok.contains('\n') && !err.contains('\n'));
    let field = camera_field(&frame());
    assert!(
        field.starts_with("{\"eye\":[0.000,5.000,12.000]"),
        "{field}"
    );
}
