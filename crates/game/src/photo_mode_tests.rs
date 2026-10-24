//! photo_mode tests: the fight freezes at its tick, every knob comes back
//! byte-equal, and a still is supersampled and capped (SW141). Each test goes
//! red without the mode: a camera with no snapshot flies off with the knobs.

use crate::camera::{CameraMode, DirectorParams};
use crate::photo_mode::{
    PhotoBounds, PhotoError, PhotoMode, PhotoState, MAX_CAPTURE_DIM, MAX_EXPOSURE_EV, MAX_FOCAL_MM,
    MAX_ROLL_RAD, MIN_FOCAL_MM,
};

/// The arena's own camera, as the window builds it.
fn camera() -> DirectorParams {
    DirectorParams::arena()
}

#[test]
fn photo_mode_freezes_the_tick_and_restores_every_knob() {
    let mut params = camera();
    let mut photo = PhotoMode::new();
    let before = params;

    photo
        .enter(&mut params, CameraMode::Free, 640, PhotoBounds::arena(20.0))
        .expect("photo mode opens");
    assert_eq!(photo.state(), PhotoState::Entered);
    assert!(photo.sim_paused(), "the sim holds while the mode is on");

    // Thirty photo frames of flight with the stick held: the camera moves, the
    // tick and the hash do not.
    for frame in 0..30 {
        let yaw = 0.01 * f32::from(u16::try_from(frame).unwrap_or(0));
        photo
            .fly(&mut params, [1.0, 0.5, 1.0], (yaw, 0.1))
            .expect("a frame flies");
        assert_eq!(photo.held_tick(), 640, "the tick stays at 640");
        assert!(photo.sim_paused(), "the sim is still held");
        assert!(
            PhotoBounds::arena(20.0).holds(params.free_eye_m),
            "in bounds"
        );
    }
    // The sim's own hash is outside this module: the frozen tick never stepped.
    assert_ne!(params.free_eye_m, before.free_eye_m, "the eye did fly");

    // Every knob the mode touched comes back byte-equal.
    let mode = photo.exit(&mut params).expect("the mode closes");
    assert_eq!(mode, CameraMode::Free);
    assert_eq!(photo.state(), PhotoState::Restored);
    assert!(!photo.sim_paused(), "the sim steps again");
    assert_eq!(params, before, "every restored field compares equal");
}

#[test]
fn photo_capture_supersamples_and_caps() {
    // A 1280x720 still is drawn at 2x, inside the default cap.
    assert_eq!(
        PhotoMode::capture_size((1280, 720), MAX_CAPTURE_DIM, false),
        Some((2560, 1440))
    );
    // A 1920x1080 still is 3840x2160, exactly the cap.
    assert_eq!(
        PhotoMode::capture_size((1920, 1080), MAX_CAPTURE_DIM, false),
        Some((3840, 2160))
    );
    // A capped base keeps its aspect ratio.
    assert_eq!(
        PhotoMode::capture_size((1920, 1080), 2560, false),
        Some((2560, 1440))
    );
    // A potato tier captures 1x: the same still, half the pixels.
    assert_eq!(
        PhotoMode::capture_size((1280, 720), MAX_CAPTURE_DIM, true),
        Some((1280, 720))
    );
    // Rubbish in, nothing out.
    assert_eq!(
        PhotoMode::capture_size((0, 720), MAX_CAPTURE_DIM, false),
        None
    );
    assert_eq!(PhotoMode::capture_size((1280, 720), 0, false), None);

    let mut params = camera();
    let mut photo = PhotoMode::new();
    assert_eq!(
        photo.shoot(&params, params.free_eye_m, false),
        Err(PhotoError::NotOn),
        "a still needs the mode on"
    );
    photo
        .enter(&mut params, CameraMode::Free, 640, PhotoBounds::arena(20.0))
        .expect("photo mode opens");
    photo
        .set_capture((1280, 720), MAX_CAPTURE_DIM)
        .expect("size");
    let plan = photo
        .shoot(&params, params.free_eye_m, false)
        .expect("a still");
    assert_eq!((plan.width, plan.height), (2560, 1440));
    assert_eq!(photo.state(), PhotoState::Shooting);
    assert!(plan.sidecar.contains("\"tick\":640"), "{}", plan.sidecar);
    assert!(
        plan.sidecar.contains("\"tier\":\"full\""),
        "{}",
        plan.sidecar
    );
    let potato = photo
        .shoot(&params, params.free_eye_m, true)
        .expect("1x still");
    assert_eq!((potato.width, potato.height), (1280, 720));
    assert!(potato.sidecar.contains("\"tier\":\"potato\""));
    assert!(potato.sidecar.contains("\"supersample\":1.0"));
}

#[test]
fn the_lens_maps_focal_length_onto_the_field_of_view() {
    let mut params = camera();
    let mut photo = PhotoMode::new();
    photo
        .enter(&mut params, CameraMode::Free, 0, PhotoBounds::arena(20.0))
        .expect("opens");
    let wide = params.fov_y_rad;
    photo
        .set_focal_mm(&mut params, MAX_FOCAL_MM)
        .expect("200 mm");
    let tight = params.fov_y_rad;
    assert!(tight < wide, "a longer lens is tighter");
    assert!(tight > 0.0 && tight < core::f32::consts::PI, "{tight}");
    assert_eq!(photo.focal_mm(), MAX_FOCAL_MM);
    assert_eq!(
        photo.set_focal_mm(&mut params, MIN_FOCAL_MM - 1.0),
        Err(PhotoError::BadFocalMm)
    );
    // 24 mm is a 53 degree vertical field on a full-frame sensor.
    photo
        .set_focal_mm(&mut params, MIN_FOCAL_MM)
        .expect("24 mm");
    let deg = params.fov_y_rad.to_degrees();
    assert!((deg - 53.13).abs() < 0.1, "24 mm reads {deg} degrees");
    // The lens went into the camera row, so leaving restores it.
    let before = camera();
    photo.exit(&mut params).expect("closes");
    assert_eq!(params.fov_y_rad, before.fov_y_rad);
}

#[test]
fn the_roll_and_the_exposure_are_bounded_and_restored() {
    let mut params = camera();
    let before = params;
    let mut photo = PhotoMode::new();
    photo
        .enter(&mut params, CameraMode::Free, 12, PhotoBounds::arena(20.0))
        .expect("opens");
    photo.set_roll_rad(MAX_ROLL_RAD).expect("a quarter turn");
    assert_eq!(photo.roll_rad(), MAX_ROLL_RAD);
    assert_eq!(
        photo.set_roll_rad(MAX_ROLL_RAD + 0.1),
        Err(PhotoError::BadRoll)
    );
    photo
        .set_exposure_ev(-2.5)
        .expect("two and a half stops down");
    assert_eq!(photo.exposure_ev(), -2.5);
    assert_eq!(
        photo.set_exposure_ev(MAX_EXPOSURE_EV + 0.1),
        Err(PhotoError::BadExposure)
    );
    let line = photo.grade_line(false);
    assert!(line.contains("exposure -2.50 EV"), "{line}");
    assert!(line.contains("supersample 2.0"), "{line}");
    photo.exit(&mut params).expect("closes");
    assert_eq!(params, before);
    assert_eq!(photo.roll_rad(), 0.0, "roll is gone with the mode");
    assert_eq!(photo.exposure_ev(), 0.0);
}

#[test]
fn flight_stays_inside_the_bounds_and_the_mode_refuses_a_second_enter() {
    let mut params = camera();
    let mut photo = PhotoMode::new();
    let bounds = PhotoBounds {
        center_m: crate::camera::Vec3::new(0.0, 1.0, 0.0),
        radius_m: 3.0,
    };
    photo
        .enter(&mut params, CameraMode::Free, 7, bounds)
        .expect("opens");
    assert_eq!(
        photo.enter(&mut params, CameraMode::Free, 8, bounds),
        Err(PhotoError::AlreadyOn)
    );
    // Hold the forward stick for a thousand frames: the eye stops on the
    // sphere, never outside it.
    for _ in 0..1000 {
        photo
            .fly(&mut params, [0.0, 0.0, 60.0], (0.0, 0.0))
            .expect("flies");
    }
    assert!(bounds.holds(params.free_eye_m), "eye left the sphere");
    let plan = photo.sidecar(&params, bounds.center_m, false);
    assert!(plan.contains("\"fov_y_rad\""), "{plan}");
    photo.exit(&mut params).expect("closes");
    assert_eq!(
        photo.exit(&mut params),
        Err(PhotoError::NotOn),
        "a second exit is refused"
    );
}

#[test]
fn photo_still_writes_the_png_and_its_sidecar_named_by_the_tick() {
    let mut params = camera();
    let mut photo = PhotoMode::new();
    photo
        .enter(&mut params, CameraMode::Free, 640, PhotoBounds::arena(20.0))
        .expect("photo mode opens");
    let plan = photo
        .shoot(&params, params.free_eye_m, false)
        .expect("a still");
    let dir = std::env::temp_dir().join(format!("photo-still-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let files = photo
        .write_still(&plan, b"\x89PNG\r\n\x1a\nstill", &dir)
        .expect("the still writes");
    assert!(files.png.is_file(), "{}", files.png.display());
    assert!(files.sidecar.is_file(), "{}", files.sidecar.display());
    let name = files.png.file_name().and_then(|n| n.to_str()).unwrap_or("");
    assert!(
        name.contains("t640"),
        "the file names the parked tick: {name}"
    );
    assert!(name.ends_with(".png"));
    // The sidecar is the plan's own JSON, and a re-write overwrites one file.
    let sidecar = std::fs::read_to_string(&files.sidecar).expect("the sidecar reads");
    assert_eq!(sidecar, plan.sidecar);
    photo
        .write_still(&plan, b"\x89PNG\r\n\x1a\nstill", &dir)
        .expect("a retry overwrites");
    let pngs = std::fs::read_dir(&dir)
        .expect("the folder reads")
        .flatten()
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "png"))
        .count();
    assert_eq!(pngs, 1, "a retry doubled no file");
    let refused = photo
        .write_still(&plan, b"", &dir)
        .expect_err("empty bytes are not a still");
    assert!(matches!(
        refused,
        PhotoError::Write(_) | PhotoError::BadCaptureSize
    ));
    let _ = std::fs::remove_dir_all(&dir);
}
