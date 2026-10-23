//! EG11 live sun depth tests, moved out of `submit.rs` (EG47) so that file
//! stays under its line pin. Test names keep their module path:
//! `present::gpu_present::submit::sun_live_tests::<test>`.

use super::*;

fn set_sun(on: bool) {
    super::FORWARD_SUN_ENABLED.store(on, core::sync::atomic::Ordering::Relaxed);
}

#[test]
fn sun_depth_switch_defaults_on_and_off_restores() {
    // Caller: `encode_forward` above (the EG11 sun depth map).
    // Behavior: the switch defaults on, so live frames record the map
    // the row Accept measures; off reads back off and on restores it.
    let _held = super::super::SUN_SWITCH_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    set_sun(true);
    assert!(forward_sun_enabled(), "switch defaults on");
    set_sun(false);
    assert!(!forward_sun_enabled(), "off reads back off");
    set_sun(true);
    assert!(forward_sun_enabled(), "switch restores on");
}

#[test]
fn encode_forward_consults_the_switch_before_the_cached_sun_encode() {
    // Caller: `encode_forward` above.
    // Behavior: the live forward submit reads the switch and then records
    // the cached EG11 map (`sun_sub.record`, which answers 0 when the switch
    // or the look asks for no map), so the frame posts a real cost. Fails if
    // the live call site is gated off or the record is dropped, which is
    // exactly the blocker EG11 was stuck on (an unwired sun API and a
    // constant-zero `arena per-pass shadow`).
    let src = include_str!("../submit.rs");
    let at = src
        .find("fn encode_forward")
        .expect("forward submit exists");
    let body = &src[at..];
    let gate = body
        .find("forward_sun_enabled()")
        .expect("live path consults the switch");
    let record = body
        .find("self.sun_sub.record(")
        .expect("EG11 cached sun map on the live path");
    assert!(gate < record, "switch runs before the sun map");
    assert_eq!(
        body.matches("self.sun_sub.record(").count(),
        1,
        "one sun map record per live frame"
    );
    assert!(
        body.contains("sun_last_us_store(self.sun_sub.record("),
        "the frame posts the cost the record measured"
    );
}

#[test]
fn live_frame_encodes_the_sun_depth_map() {
    // Caller: `encode_forward` above; EG11 Accept, "`arena per-pass`
    // shadow above 0 ms on live frames".
    // Behavior: one offscreen live frame (the same encode path the
    // windowed arena runs) records the row's 2048 sun depth map from the
    // live `sun_sub` pair (the half arm can run it at 1024), so the receipt
    // carries a real cost and 0 validation errors. Before the live call was
    // wired the slot stayed a constant zero (newest capture:
    // `shadow=0.000ms`).
    let _held = super::super::SUN_SWITCH_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    set_sun(true);
    let probe = Presenter::init_offscreen().map_err(|error| match error {
        crate::present::PresentError::NoAdapter => {
            crate::test_gpu::NoAdapter("no wgpu adapter on this machine".into())
        }
        other => panic!("presenter init failed (must fail loudly): {other}"),
    });
    let Some(mut presenter) =
        crate::test_gpu::require_or_skip("live_frame_encodes_the_sun_depth_map", probe)
    else {
        return;
    };
    let key = MeshKey::new(1, [0xB7; 32]);
    presenter
        .upload_mesh(key, &v1::cube())
        .expect("cube uploads");
    presenter.resize(160, 120).expect("valid size applies");
    let scene = v1::default_scene(160, 120).expect("default scene is valid");
    let list = DrawList {
        camera: scene.camera,
        light: scene.light,
        items: vec![DrawItem {
            mesh: key,
            material: scene.material,
            base_color: None,
            normal_map: None,
            orm: None,
            emissive: None,
        }],
    };
    let out = presenter
        .render_offscreen(&list)
        .expect("live frame renders");
    assert!(
        out.receipt.is_clean(),
        "validation errors must be 0, got {:?}",
        out.receipt
    );
    let us = out.receipt.shadow_submit_us;
    eprintln!("live sun depth map: shadow_submit_us={us}");
    assert!(
        us > 0,
        "live frame records the EG11 sun depth map, shadow_submit_us={us}"
    );
    // P2P: the sample is a measurement, not a constant. A second frame
    // keeps the map (no per-frame texture plus pipeline build) and stays
    // inside the same order of magnitude.
    assert!(
        out.receipt.presented,
        "the frame presented with the sun depth map recorded"
    );
    assert!(
        presenter.frames_presented() >= 1,
        "the cached map did not block the frame"
    );
}

/// A live window receipt shape: every row 0 except the shadow slot, the
/// value `live_shadow_row` is handed.
fn live_receipt(shadow_submit_us: u64) -> FrameReceipt {
    FrameReceipt {
        width: 160,
        height: 120,
        draws: 1,
        validation_errors: 0,
        generation: 0,
        presented: true,
        adapter: String::new(),
        driver: String::new(),
        mesh_count: 1,
        texture_count: 0,
        mesh_high_water: 1,
        texture_high_water: 0,
        mesh_evicted: 0,
        texture_evicted: 0,
        evicted_mesh_id: None,
        evicted_texture_id: None,
        look_passes: 0,
        look_validation_errors: 0,
        ibl_passes: 0,
        ibl_validation_errors: 0,
        fog_passes: 0,
        fog_validation_errors: 0,
        fog_submit_us: 0,
        gtao_passes: 0,
        gtao_validation_errors: 0,
        gtao_submit_us: 0,
        sky_submit_us: 0,
        layer_submit_us: 0,
        particle_submit_us: 0,
        decal_submit_us: 0,
        ibl_submit_us: 0,
        shadow_submit_us,
    }
}

#[test]
fn window_receipt_posts_the_live_sun_depth_row() {
    // Caller: `render_surface` above through `live_shadow_row`.
    // Behavior: the live frame's receipt carries the us the sun depth pass
    // recorded, so `arena per-pass shadow` reads above 0 ms on live frames
    // (EG11 Accept) instead of the receipt builder's constant 0; a frame
    // whose encode skipped (switch off or a bad sun direction) posts 0 and
    // never the reading from the frame before.
    let hit = live_shadow_row(live_receipt(0), 1_500);
    assert_eq!(hit.shadow_submit_us, 1_500, "live frame posts the pass");
    assert_eq!(
        hit.per_pass_us()[7],
        ("shadow", 1_500),
        "the per-pass table's shadow row"
    );
    assert_eq!(hit.draws, 1, "the other rows ride through untouched");
    let skip = live_shadow_row(live_receipt(1_500), 0);
    assert_eq!(skip.shadow_submit_us, 0, "a skip overwrites, never stale");
    assert_eq!(skip.per_pass_us()[7], ("shadow", 0), "a skip posts 0 ms");
}

#[test]
fn render_surface_posts_the_live_sun_row_on_every_presenting_arm() {
    // Caller: `render_surface` above. Wiring check over the shipped
    // source: the window's two presenting arms (Success and Suboptimal)
    // both hand the live receipt through the helper and read the store
    // `encode_forward` writes, so a window frame cannot keep the builder's
    // 0 shadow row. The pixel leg is the window run (no adapter here).
    let src = include_str!("../submit.rs");
    let at = src
        .find("pub fn render_surface")
        .expect("surface submit exists");
    let end = at
        + src[at..]
            .find("\n    fn reconfigure_surface")
            .expect("surface body ends");
    let body = &src[at..end];
    assert_eq!(
        body.matches("live_shadow_row(").count(),
        2,
        "both presenting arms post the live sun row"
    );
    assert!(
        body.contains("sun_last_us()"),
        "the row reads the live store"
    );
}
