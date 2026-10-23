//! The after-blit debug line pass, as a caller sees it: four line groups
//! staged into the presenter draw over the capture frame, while the same scene
//! with the pass off stays the beauty frame.
//!
//! Why: `debug_draw.rs` proves the quad math and the presenter stages a batch,
//! but only a real frame proves the pass reaches pixels. This renders the
//! `v1::default_scene` cube offscreen twice and counts pixels that differ.
//! Units: pixels on 192x108 frames. Skips with no adapter through `test_gpu`.
//! Errors: a frame with validation errors panics (the receipt must be clean).
//!
//! Run: `cargo test -p engine-render --features gpu --test it debug_lines::`
//! Set `E2040_KEEP_DEBUG_LINES=1` to save the pair under
//! `target/clips/debug-lines/`.

use engine_render::debug_draw::{DebugLine, LineBatch, LineGroup};
use engine_render::present::{DrawItem, DrawList, MeshKey, PresentError, Presenter};
use engine_render::test_gpu::{require_or_skip, NoAdapter};

/// Capture width in pixels.
const W: u32 = 192;
/// Capture height in pixels.
const H: u32 = 108;

/// Four named groups of a dozen HUD-space lines each: verticals, horizontals,
/// diagonals and short bars, the shapes a collider, joint, sweep and nav view
/// push. Each group carries its own colour.
fn four_groups() -> LineBatch {
    let specs: [(&'static str, [f32; 4]); 4] = [
        ("colliders", [1.0, 0.15, 0.1, 1.0]),
        ("joints", [0.2, 1.0, 0.25, 1.0]),
        ("sweeps", [1.0, 0.85, 0.1, 1.0]),
        ("nav", [0.25, 0.55, 1.0, 1.0]),
    ];
    let mut groups = Vec::new();
    for (row, (name, colour)) in specs.iter().enumerate() {
        let mut group = LineGroup::new(name, -4);
        for i in 0..12 {
            let t = -0.9 + i as f32 * 0.16;
            let (a_m, b_m) = match row {
                0 => ([t, -0.9, 0.0], [t, 0.9, 0.0]),
                1 => ([-0.9, t, 0.0], [0.9, t, 0.0]),
                2 => ([-0.9, t, 0.0], [t, 0.9, 0.0]),
                _ => ([t, -0.2, 0.0], [t + 0.1, 0.2, 0.0]),
            };
            group
                .push(DebugLine {
                    a_m,
                    b_m,
                    colour_rgba: *colour,
                })
                .expect("a finite line lands");
        }
        groups.push(group);
    }
    LineBatch::build(&groups).expect("a finite batch builds")
}

/// One offscreen capture of the cube scene, RGBA8 pixels.
fn capture(presenter: &mut Presenter, list: &DrawList) -> Vec<u8> {
    let out = presenter.render_offscreen(list).expect("a capture renders");
    assert!(
        out.receipt.is_clean(),
        "no validation errors: {:?}",
        out.receipt
    );
    let png = out.png.expect("a capture has pixels");
    image::load_from_memory(&png)
        .expect("a capture is a PNG")
        .to_rgba8()
        .into_raw()
}

/// Pixels whose RGBA differs in any channel.
fn differing(a: &[u8], b: &[u8]) -> usize {
    assert_eq!(a.len(), b.len(), "same frame size");
    a.chunks_exact(4)
        .zip(b.chunks_exact(4))
        .filter(|(pa, pb)| pa != pb)
        .count()
}

#[test]
fn debug_lines_capture_lights_1000_pixels_over_the_beauty_frame() {
    let probe = match Presenter::init_offscreen() {
        Ok(presenter) => Ok(presenter),
        Err(PresentError::NoAdapter) => Err(NoAdapter("no wgpu adapter on this machine".into())),
        Err(err) => panic!("presenter init failed (must fail loudly): {err}"),
    };
    let Some(mut presenter) = require_or_skip("debug lines capture", probe) else {
        return;
    };
    presenter.resize(W, H).expect("a valid size applies");
    let scene = engine_render::v1::default_scene(W, H).expect("the default scene is valid");
    let key = MeshKey::new(0x5C00_0100, [0x44; 32]);
    presenter
        .upload_mesh(key, &scene.mesh)
        .expect("cube uploads");
    let list = DrawList {
        camera: scene.camera.clone(),
        light: scene.light.clone(),
        items: vec![DrawItem {
            mesh: key,
            material: scene.material.clone(),
            base_color: None,
            normal_map: None,
            orm: None,
            emissive: None,
        }],
    };
    assert!(!presenter.debug_lines_enabled(), "the pass starts off");
    let beauty = capture(&mut presenter, &list);
    let beauty_again = capture(&mut presenter, &list);
    assert_eq!(beauty, beauty_again, "an off frame repeats byte for byte");

    let batch = four_groups();
    presenter.push_debug_lines(&batch);
    assert_eq!(presenter.debug_lines().line_count(), 48, "four groups land");
    assert!(
        !presenter.debug_line_enable(true),
        "the switch was off before this call"
    );
    assert!(presenter.debug_lines_enabled());
    let lit = capture(&mut presenter, &list);
    let lines = differing(&beauty, &lit);
    eprintln!("debug lines capture: {lines} pixels differ from the beauty frame");
    assert!(
        lines >= 1000,
        "four groups differ on {lines} pixels, need at least 1000"
    );

    if std::env::var_os("E2040_KEEP_DEBUG_LINES").is_some() {
        let dir = std::path::Path::new("target/clips/debug-lines");
        let _ = std::fs::create_dir_all(dir);
        for (name, rgba) in [("off.png", &beauty), ("on.png", &lit)] {
            image::save_buffer(dir.join(name), rgba, W, H, image::ExtendedColorType::Rgba8)
                .expect("the pair saves");
        }
    }

    assert!(presenter.debug_line_enable(false), "the switch was on");
    let off_again = capture(&mut presenter, &list);
    assert_eq!(beauty, off_again, "switching off restores the beauty frame");
}
