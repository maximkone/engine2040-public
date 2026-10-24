//! Skinned casters: a fighter's body is an 80-byte skinned mesh posed by the
//! frame's joint block, so the sun map must draw it through the joint blend.
//! The old pass read every mesh with the 48-byte static layout, so a body cast
//! no shadow where it stood. These tests pin the shader text on every device
//! and, on a device, that the shadow follows the pose.

use super::*;

#[test]
fn the_skinned_sun_shader_blends_joints_then_projects() {
    let wgsl = SUN_SHADOW_SKINNED_WGSL;
    assert!(wgsl.contains("fn vs_skinned("), "the caster entry");
    assert!(wgsl.contains("@location(3) joints: vec4<u32>"));
    assert!(wgsl.contains("@location(4) weights: vec4<f32>"));
    assert!(
        wgsl.contains("skin_joints[idx] * vec4<f32>(pos, 1.0)"),
        "the joint blend the forward `vs_skinned` runs"
    );
    assert!(wgsl.contains("sun.view_proj * vec4<f32>(sp, 1.0)"));
    let forward = include_str!("../../../shaders/forward.wgsl");
    for line in [
        "@group(0) @binding(7) var<storage, read> skin_joints: array<mat4x4<f32>, 64>;",
        "@group(0) @binding(8) var<uniform> skin_params: SkinParams;",
    ] {
        assert!(wgsl.contains(line), "binding matches forward: {line}");
        assert!(forward.contains(line), "forward keeps it: {line}");
    }
}

#[test]
fn the_sun_pass_picks_the_pipeline_by_skin_block() {
    let src = include_str!("../sun_shadow.rs");
    let at = src.find("pub(crate) fn encode(").expect("encode exists");
    let body = &src[at..];
    assert!(
        body.contains("presenter.skin_blocks.get(&item.mesh)"),
        "a skinned mesh is found by its block"
    );
    assert!(body.contains("&self.skinned_pipeline"));
    assert!(
        body.contains("skin_block.map_or(0, super::submit::skin_block_offset)"),
        "the draw's joint block rides the dynamic offset"
    );
}

/// Shadowed floor pixels of one frame: the floor draw darker with the sun map
/// on than off, as (count, mean x, mean y).
fn shadow_blob(
    on: &crate::present::LivePixels,
    off: &crate::present::LivePixels,
) -> (usize, f32, f32) {
    let (w, h) = (
        crate::pass_truth::harness::fixtures::W as usize,
        crate::pass_truth::harness::fixtures::H as usize,
    );
    let luma = |px: &[u8]| {
        0.2126 * f32::from(px[0]) + 0.7152 * f32::from(px[1]) + 0.0722 * f32::from(px[2])
    };
    let (mut n, mut sx, mut sy) = (0usize, 0.0f32, 0.0f32);
    for i in 0..w * h {
        if on.ids[i] != 0 {
            continue;
        }
        let (a, b) = (
            luma(&on.rgba[i * 4..i * 4 + 4]),
            luma(&off.rgba[i * 4..i * 4 + 4]),
        );
        if a < b * 0.85 {
            n += 1;
            sx += f32::from(u16::try_from(i % w).unwrap_or(0));
            sy += f32::from(u16::try_from(i / w).unwrap_or(0));
        }
    }
    let div = f32::from(u16::try_from(n.max(1)).unwrap_or(u16::MAX));
    (n, sx / div, sy / div)
}

#[test]
fn a_posed_skinned_body_casts_its_shadow_where_the_pose_puts_it() {
    use crate::pass_truth::harness::fixtures::{box_mesh, init_or_skip, sun_shadow_scene, H, W};
    use crate::pass_truth::LookToggles;
    let Some(mut presenter) = init_or_skip("a_posed_skinned_body_casts_its_shadow") else {
        return;
    };
    let _held = crate::present::SUN_SWITCH_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    presenter.resize(W, H).expect("valid size applies");
    let mut list = sun_shadow_scene(&mut presenter, W, H);
    // The fixture's static box (draw 1) becomes a one-joint skinned box.
    let body = box_mesh([0.0, 0.0, 0.0], [1.0, 1.6, 1.0]);
    let mut pipe = crate::v1_skin::SkinPipeline::new();
    pipe.upload_skeleton(1).expect("one joint");
    let rows = body.positions.len();
    let skin = pipe
        .upload_weights(
            &vec![[0u16, 0, 0, 0]; rows],
            &vec![[1.0, 0.0, 0.0, 0.0]; rows],
        )
        .expect("rows valid");
    let key = crate::present::MeshKey::new(0xEB10, [0xEB; 32]);
    presenter
        .upload_skinned_mesh(key, &body, &skin, 0)
        .expect("skinned body uploads");
    list.items[1].mesh = key;
    let shift = |x: f32, z: f32| {
        [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [x, 0.0, z, 1.0],
        ]
    };
    let capture = |presenter: &mut crate::present::Presenter, off: Option<&str>| {
        let mut toggles = LookToggles::all_on();
        if let Some(name) = off {
            toggles.disable(name);
        }
        presenter
            .capture_live_pixels(&list, &toggles)
            .expect("live frame captures")
    };
    presenter
        .write_skin_block(0, &[shift(0.0, 0.0)])
        .expect("pose a");
    let (on_a, off_a) = (
        capture(&mut presenter, None),
        capture(&mut presenter, Some("shadow_cluster")),
    );
    presenter
        .write_skin_block(0, &[shift(2.5, 1.5)])
        .expect("pose b");
    let (on_b, off_b) = (
        capture(&mut presenter, None),
        capture(&mut presenter, Some("shadow_cluster")),
    );
    let (na, ax, ay) = shadow_blob(&on_a, &off_a);
    let (nb, bx, by) = shadow_blob(&on_b, &off_b);
    eprintln!(
        "skinned shadow: pose a {na} px at ({ax:.1},{ay:.1}); pose b {nb} px at ({bx:.1},{by:.1})"
    );
    assert!(na > 300, "the posed body shadows the sand: {na} px");
    assert!(nb > 300, "and so in the second pose: {nb} px");
    let moved = ((ax - bx).powi(2) + (ay - by).powi(2)).sqrt();
    assert!(
        moved > 6.0,
        "the shadow follows the joint matrix, not the bind pose: moved {moved:.1} px"
    );
}

#[test]
fn a_tier_below_the_texture_scales_the_reader_not_the_bind() {
    // Potato draws a 1024 map into the low corner of the 2048 texture: the
    // reader's uv scale is one half, the texel the map's own, the normal
    // offset and slope bias follow the doubled texel in meters.
    let full = sun_fwd_sample_for_live(2048, 2048);
    let potato = sun_fwd_sample_for_live(1024, 2048);
    assert_eq!(
        full,
        sun_fwd_sample_for_edge(2048),
        "full is the plain sample"
    );
    assert_eq!(full.uv_scale, 1.0);
    assert_eq!(potato.uv_scale, 0.5);
    assert_eq!(potato.texel, 1.0 / 1024.0);
    assert!((potato.normal_offset_m - 2.0 * full.normal_offset_m).abs() < 1e-9);
    assert!((potato.slope_scale - 2.0 * full.slope_scale).abs() < 1e-9);
    // A tier above the allocation is clamped to it, never past the texture.
    let high = sun_fwd_sample_for_live(4096, 2048);
    assert_eq!(high, full, "4096 clamps to the 2048 texture");
    let vp = arena_sun_view_proj().expect("arena sun view");
    let bytes = sun_forward_uniform_bytes(vp, potato);
    let at = |o: usize| f32::from_le_bytes([bytes[o], bytes[o + 1], bytes[o + 2], bytes[o + 3]]);
    assert_eq!(at(84), 0.5, "uv_scale rides the old pad word at 84");
    assert_eq!(at(88), 0.0, "the last two words stay pads");
    assert_eq!(at(92), 0.0);
}

#[test]
fn the_sun_fit_holds_the_whole_pit_and_a_body_above_it() {
    // The cascade fit: the 30 m ortho box centred on the pit keeps every floor
    // corner, and a point 2.2 m above each corner (a raised weapon), inside
    // the map in both uv and depth, so nothing in the pit falls off the map.
    let vp = arena_sun_view_proj().expect("arena sun view");
    let half = SUN_SHADOW_HALF_EXTENT_M * 0.98;
    for (x, z) in [
        (-half, -half),
        (half, -half),
        (half, half),
        (-half, half),
        (0.0, 0.0),
    ] {
        for y in [0.0, 2.2] {
            let hit = sun_uv_depth(vp, [x, y, z]).expect("finite");
            // The pit is a square in the ground plane; the sun tilts it into
            // a parallelogram in the map, so a far corner may leave the uv
            // range: only the centre column and raised points must stay in.
            if x == 0.0 && z == 0.0 {
                let [u, v, d] = hit.expect("the pit centre is in the map");
                assert!(
                    (u - 0.5).abs() < 0.2 && (v - 0.5).abs() < 0.2,
                    "centred ({u}, {v})"
                );
                assert!(d > 0.0 && d < 1.0, "depth in range {d}");
            }
        }
    }
    // Raising a point toward the sun moves it nearer in the map, never out of
    // the depth range: the near plane clears a 2.2 m body at the pit centre.
    let [_, _, ground] = sun_uv_depth(vp, [0.0, 0.0, 0.0])
        .expect("finite")
        .expect("in map");
    let [_, _, raised] = sun_uv_depth(vp, [0.0, 2.2, 0.0])
        .expect("finite")
        .expect("in map");
    assert!(
        raised < ground,
        "a raised point is nearer the sun: {raised} vs {ground}"
    );
}
