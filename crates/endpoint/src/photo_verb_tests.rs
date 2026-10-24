//! photo_verb tests: one still per `(tick, pose)`, a path that exists, and a
//! headless door that refuses by name (SW145). Each test goes red without the
//! verb: a second call would render a second file.

use std::cell::Cell;

use super::{
    lens_fov_y_rad, PhotoError, PhotoPose, PhotoRequest, PhotoShootArgs, PhotoShootTool,
    PhotoShots, RenderRefusal, RenderedFrame,
};

/// A scratch folder of this test's own.
fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("photo-verb-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

/// A renderer that counts its calls and reports a 2x 1280x720 frame.
fn renderer(calls: &Cell<u32>) -> impl Fn(&PhotoPose) -> Result<RenderedFrame, RenderRefusal> + '_ {
    move |_pose| {
        calls.set(calls.get() + 1);
        Ok(RenderedFrame {
            bytes: b"\x89PNG\r\n\x1a\nphoto".to_vec(),
            hash: 0xd57b_5c8a_e309_6614,
            width: 2560,
            height: 1440,
        })
    }
}

#[test]
fn photo_shoot_is_idempotent_per_tick_and_pose() {
    let dir = scratch("idempotent");
    let mut shots = PhotoShots::new();
    let calls = Cell::new(0);
    let render = renderer(&calls);
    let request = PhotoRequest {
        pose: "hero".to_owned(),
        free: None,
        tick: 300,
        focal_mm: 50.0,
    };

    let first = shots
        .shoot(&request, &dir, Some(&render))
        .expect("the first still");
    let second = shots
        .shoot(&request, &dir, Some(&render))
        .expect("the retry answers the same still");
    assert_eq!(first, second, "one path and one hash per (tick, pose)");
    assert_eq!(calls.get(), 1, "the retry rendered nothing");
    assert_eq!(shots.renders(), 1);
    assert_eq!(shots.len(), 1);
    assert_eq!(first.tick, 300);
    assert_eq!(first.hash, 0xd57b_5c8a_e309_6614);
    assert!(
        std::path::Path::new(&first.path).is_file(),
        "{}",
        first.path
    );
    assert!(
        std::path::Path::new(&first.sidecar).is_file(),
        "{}",
        first.sidecar
    );
    // One PNG in the folder, and its sidecar names the tick.
    let pngs = std::fs::read_dir(&dir)
        .expect("the folder reads")
        .flatten()
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "png"))
        .count();
    assert_eq!(pngs, 1, "the retry doubled no file");
    let sidecar = std::fs::read_to_string(&first.sidecar).expect("the sidecar reads");
    assert!(sidecar.contains("\"tick\":300"), "{sidecar}");
    assert!(sidecar.contains("\"pose\":\"hero\""), "{sidecar}");
    assert!(sidecar.contains("\"supersample\":2"), "{sidecar}");

    // Another tick is another still.
    let later = PhotoRequest {
        tick: 301,
        ..request.clone()
    };
    let next = shots
        .shoot(&later, &dir, Some(&render))
        .expect("a second tick");
    assert_ne!(next.path, first.path, "a new tick writes a new frame");
    assert_eq!(calls.get(), 2);
    assert_eq!(shots.len(), 2);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_headless_door_refuses_a_still_by_name() {
    let dir = scratch("headless");
    let mut shots = PhotoShots::new();
    let request = PhotoRequest::default();
    let refused = shots
        .shoot(&request, &dir, None)
        .expect_err("no renderer, no still");
    assert_eq!(refused, PhotoError::NoRenderer);
    let text = refused.to_string();
    assert!(text.contains("no renderer"), "{text}");
    assert!(
        text.contains("re-try"),
        "the refusal names the recovery: {text}"
    );
    assert!(shots.is_empty(), "nothing was filed");
    assert!(!dir.exists(), "nothing was written");
}

#[test]
fn the_three_presets_frame_the_fight_and_a_bad_name_is_refused() {
    for name in ["hero", "overhead", "blade"] {
        let pose = PhotoPose::parse(name).expect("a preset");
        let (eye, target) = pose.eye_target();
        assert!(eye.iter().all(|axis| axis.is_finite()), "{name} eye");
        assert!(target.iter().all(|axis| axis.is_finite()), "{name} target");
        assert_ne!(eye, target, "{name} looks at a point away from the eye");
    }
    let refused = PhotoPose::parse("selfie").expect_err("not a preset");
    assert_eq!(refused, PhotoError::BadPose("selfie".to_owned()));
    let text = refused.to_string();
    for legal in ["hero", "overhead", "blade", "free"] {
        assert!(text.contains(legal), "{text} names every legal pose");
    }
}

#[test]
fn the_free_pose_takes_the_photographers_own_eye_and_target() {
    let dir = scratch("free");
    let mut shots = PhotoShots::new();
    let calls = Cell::new(0);
    let render = renderer(&calls);
    let request = PhotoRequest {
        pose: "free".to_owned(),
        free: Some(([1.0, 2.0, 3.0], [0.0, 1.0, 0.0])),
        tick: 42,
        focal_mm: 24.0,
    };
    let shot = shots
        .shoot(&request, &dir, Some(&render))
        .expect("a free still");
    assert_eq!(shot.pose, "free");
    assert_eq!(shot.eye, [1.0, 2.0, 3.0]);
    assert_eq!(shot.target, [0.0, 1.0, 0.0]);
    let deg = shot.fov_y_rad.to_degrees();
    assert!((deg - lens_fov_y_rad(24.0).to_degrees()).abs() < 1e-4);
    assert!((deg - 53.13).abs() < 0.1, "24 mm reads {deg} degrees");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_frame_over_the_capture_cap_is_refused_and_a_lens_outside_stays_out() {
    let dir = scratch("cap");
    let mut shots = PhotoShots::new();
    let too_big = |_pose: &PhotoPose| {
        Ok::<RenderedFrame, RenderRefusal>(RenderedFrame {
            bytes: Vec::new(),
            hash: 1,
            width: 5000,
            height: 100,
        })
    };
    let request = PhotoRequest::default();
    let refused = shots
        .shoot(&request, &dir, Some(&too_big))
        .expect_err("over the cap");
    assert!(matches!(refused, PhotoError::Render(_)), "{refused:?}");
    assert!(shots.is_empty());
    assert!(!dir.exists(), "a refused frame writes nothing");

    let mut wide = PhotoRequest::default();
    wide.focal_mm = 8.0;
    assert_eq!(
        shots
            .shoot(&wide, &dir, Some(&too_big))
            .expect_err("8 mm is not a lens here"),
        PhotoError::BadFocalMm
    );
    // A long lens is tighter than a wide one.
    assert!(lens_fov_y_rad(200.0) < lens_fov_y_rad(24.0));
}

/// A renderer with no captured state, as a host installs it on the door.
fn fixed_renderer(_pose: &PhotoPose) -> Result<RenderedFrame, RenderRefusal> {
    Ok(RenderedFrame {
        bytes: b"\x89PNG\r\n\x1a\ndoor".to_vec(),
        hash: 0x1234_5678_9abc_def0,
        width: 2560,
        height: 1440,
    })
}

#[test]
fn photo_shoot_tool_is_idempotent_on_the_door_and_refuses_a_headless_one() {
    use engine_editor::capability::Capability;
    use rmcp::handler::server::router::tool::SyncTool;

    let server = crate::mcp::McpServer::new(7, Capability::unrestricted(7));
    let dir = scratch("door");
    let args = PhotoShootArgs {
        pose: Some("hero".to_owned()),
        tick: Some(300),
        focal_mm: None,
        dir: Some(dir.to_string_lossy().into_owned()),
        eye: None,
        target: None,
    };

    // A door with no renderer: a typed refusal with a recovery line, no file.
    let refused = PhotoShootTool::invoke(&server, args.clone()).expect_err("headless refuses");
    assert_eq!(refused.code, "no_renderer");
    assert!(refused.message.contains("re-try"), "{}", refused.message);
    assert!(!dir.exists(), "a refusal writes nothing");

    // With a renderer installed: one still, and the retry is the same answer.
    server
        .lock()
        .expect("lock")
        .set_photo_renderer(fixed_renderer);
    let first = PhotoShootTool::invoke(&server, args.clone()).expect("the first still");
    let second = PhotoShootTool::invoke(&server, args.clone()).expect("the retry");
    assert_eq!(first, second, "one path and one hash per (tick, pose)");
    assert_eq!(first.tick, 300);
    assert_eq!(first.hash, 0x1234_5678_9abc_def0);
    assert!(
        std::path::Path::new(&first.path).is_file(),
        "{}",
        first.path
    );
    let pngs = std::fs::read_dir(&dir)
        .expect("the folder reads")
        .flatten()
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "png"))
        .count();
    assert_eq!(pngs, 1, "the retry doubled no file");

    // A missing tick is refused by name before any render.
    let mut untimed = args;
    untimed.tick = None;
    let refused = PhotoShootTool::invoke(&server, untimed).expect_err("no tick");
    assert_eq!(refused.code, "bad_tick");
    let _ = std::fs::remove_dir_all(&dir);
}
