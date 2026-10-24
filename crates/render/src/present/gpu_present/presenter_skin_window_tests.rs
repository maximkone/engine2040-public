//! The presenter's skinned-mesh tests (skin blocks, the live window frame, text), moved out of
//! presenter.rs so the file stays under its line pin. `super` is the presenter module.

use super::super::submit::{
    skin_block_offset, SKINNED_VERTEX_STRIDE, SKIN_BLOCKS, SKIN_BLOCK_BYTES, SKIN_BLOCK_JOINTS,
    SKIN_BLOCK_STRIDE,
};
use crate::present::{DrawItem, DrawList, MeshKey, PresentError, Presenter};
use crate::v1::{self, MeshData};
use crate::v1_skin::{SkinPipeline, SkinWeights};

type Mat = [[f32; 4]; 4];

const IDENTITY: Mat = [
    [1.0, 0.0, 0.0, 0.0],
    [0.0, 1.0, 0.0, 0.0],
    [0.0, 0.0, 1.0, 0.0],
    [0.0, 0.0, 0.0, 1.0],
];

/// AN1 row-vector turn about +Z by `angle`, then a move by `t`.
fn turn_z(angle: f32, t: [f32; 3]) -> Mat {
    let (s, c) = angle.sin_cos();
    [
        [c, s, 0.0, 0.0],
        [-s, c, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [t[0], t[1], t[2], 1.0],
    ]
}

/// The unit cube bound to two joints: weight on joint 1 grows with
/// height and a little with x, so the top follows joint 1, the bottom
/// stays on joint 0 and the middle band blends.
fn two_joint_cube() -> (MeshData, SkinWeights) {
    let mesh = v1::cube();
    let mut joints = Vec::new();
    let mut weights = Vec::new();
    for p in &mesh.positions {
        let w1 = (0.5 + p[1] + 0.4 * p[0]).clamp(0.0, 1.0);
        joints.push([0u16, 1, 0, 0]);
        weights.push([1.0 - w1, w1, 0.0, 0.0]);
    }
    let mut pipe = SkinPipeline::new();
    pipe.upload_skeleton(2).expect("two joints");
    let skin = pipe.upload_weights(&joints, &weights).expect("rows valid");
    (mesh, skin)
}

/// The CPU skin the old window path uploaded: posed positions and
/// normals over the bind uvs and indices.
fn cpu_posed(mesh: &MeshData, skin: &SkinWeights, matrices: &[Mat]) -> MeshData {
    let mut pipe = SkinPipeline::new();
    pipe.upload_skeleton(matrices.len()).expect("joints");
    let uniform = pipe.skinning_uniform(matrices).expect("matrices");
    let posed = pipe
        .render_skinned(mesh, Some(skin), &uniform)
        .expect("skins");
    v1::upload_mesh(&posed.positions, &posed.normals, &mesh.uvs, &mesh.indices)
        .expect("posed mesh is valid")
}

fn presenter_or_skip(test: &str) -> Option<Presenter> {
    let probe = match Presenter::init_offscreen() {
        Ok(mut presenter) => {
            presenter.resize(128, 128).expect("valid size applies");
            Ok(presenter)
        }
        Err(PresentError::NoAdapter) => Err(crate::test_gpu::NoAdapter(
            "no wgpu adapter on this machine".into(),
        )),
        Err(err) => panic!("presenter init failed (must fail loudly): {err}"),
    };
    crate::test_gpu::require_or_skip(test, probe)
}

fn list_of(keys: &[MeshKey]) -> DrawList {
    let scene = v1::default_scene(128, 128).expect("default scene is valid");
    DrawList {
        camera: scene.camera,
        light: scene.light,
        items: keys
            .iter()
            .map(|key| DrawItem {
                mesh: *key,
                material: scene.material.clone(),
                base_color: None,
                normal_map: None,
                orm: None,
                emissive: None,
            })
            .collect(),
    }
}

fn pixels(presenter: &mut Presenter, list: &DrawList) -> Vec<u8> {
    let out = presenter.render_offscreen(list).expect("frame renders");
    assert!(
        out.receipt.is_clean(),
        "no validation errors: {:?}",
        out.receipt
    );
    let png = out.png.expect("capture has pixels");
    image::load_from_memory(&png)
        .expect("capture is a PNG")
        .to_rgba8()
        .into_raw()
}

/// (largest channel difference, pixels differing by more than `slack`).
fn compare(a: &[u8], b: &[u8], slack: u8) -> (u8, usize) {
    assert_eq!(a.len(), b.len(), "same frame size");
    let mut worst = 0u8;
    let mut over = 0usize;
    for (pa, pb) in a.chunks_exact(4).zip(b.chunks_exact(4)) {
        let d = pa
            .iter()
            .zip(pb)
            .map(|(x, y)| x.abs_diff(*y))
            .max()
            .unwrap_or(0);
        worst = worst.max(d);
        over += usize::from(d > slack);
    }
    (worst, over)
}

/// Pixels that left the background color.
fn covered(rgba: &[u8]) -> usize {
    rgba.chunks_exact(4)
        .filter(|p| {
            let off = (f32::from(p[0]) - 7.0)
                .abs()
                .max((f32::from(p[1]) - 8.0).abs())
                .max((f32::from(p[2]) - 15.0).abs());
            off > 5.0
        })
        .count()
}

#[test]
fn a_presenter_on_a_caller_opened_device_draws_the_frame_init_offscreen_draws() {
    let Some(mut classic) = presenter_or_skip("from-device parity") else {
        return;
    };
    // The route the browser takes: the caller picks the adapter and opens the device with
    // the shared descriptor, then hands all three over.
    let (choice, _) = crate::backend::resolve_from_env(None);
    let pick = crate::backend::pick_backend(choice, None);
    let instance = crate::backend::new_instance(crate::backend::wgpu_backends(pick.primary));
    let (adapter, _) = pollster::block_on(crate::presenter_pick::pick_adapter(
        &instance,
        crate::backend::wgpu_backends(pick.primary),
        &crate::adapter::SelectionRequest::from_env(),
    ))
    .expect("the adapter init_offscreen found is found again");
    let (device, queue) =
        pollster::block_on(adapter.request_device(&Presenter::device_descriptor(&adapter)))
            .expect("the shared descriptor opens a device");
    let mut parts = Presenter::init_offscreen_from_device(&adapter, device, queue)
        .expect("a caller-opened device builds a presenter");
    parts.resize(128, 128).expect("valid size applies");
    assert_eq!(parts.adapter_name(), classic.adapter_name());
    assert_eq!(
        parts.backend_selection().source,
        crate::adapter::SelectionSource::Caller,
        "a caller-opened device reports that the caller picked it"
    );
    assert_eq!(
        parts.backend_selection().selected,
        classic.backend_selection().selected,
        "both routes name the same adapter"
    );
    let key = MeshKey::new(0x5C00_0100, [0x44; 32]);
    for presenter in [&mut classic, &mut parts] {
        presenter
            .upload_mesh(key, &v1::cube())
            .expect("cube uploads");
    }
    let list = list_of(&[key]);
    let toggles = crate::pass_truth::LookToggles::all_on();
    let a = classic
        .capture_live_pixels(&list, &toggles)
        .expect("classic live frame");
    let b = parts
        .capture_live_pixels(&list, &toggles)
        .expect("caller-device live frame");
    assert!(a.receipt.is_clean() && b.receipt.is_clean());
    assert!(
        a.rgba.chunks_exact(4).any(|px| px[0] > 40),
        "the cube is lit"
    );
    assert_eq!(
        a.rgba, b.rgba,
        "both routes draw the same live frame, byte for byte"
    );
    assert_eq!(
        pixels(&mut classic, &list),
        pixels(&mut parts, &list),
        "and the same capture frame"
    );
}

#[test]
fn skin_block_layout_matches_the_shader_palette() {
    let wgsl = crate::v1_gpu::FORWARD_PBR_WGSL;
    assert!(
        wgsl.contains(
            "@group(0) @binding(7) var<storage, read> skin_joints: array<mat4x4<f32>, 64>"
        ),
        "the shader palette is 64 matrices at binding 7"
    );
    assert!(wgsl.contains("@location(3) joints: vec4<u32>"));
    assert!(wgsl.contains("@location(4) weights: vec4<f32>"));
    assert_eq!(SKIN_BLOCK_JOINTS, 64);
    assert_eq!(SKIN_BLOCK_BYTES, 64 * 64, "one mat4 is 64 bytes");
    assert_eq!(u64::from(SKIN_BLOCK_STRIDE), SKIN_BLOCK_BYTES);
    assert_eq!(SKINNED_VERTEX_STRIDE, 12 + 12 + 8 + 16 + 16 + 16);
    assert!(wgsl.contains("@location(5) tangent: vec4<f32>"));
    for block in 0..SKIN_BLOCKS {
        let offset = skin_block_offset(u8::try_from(block).expect("block fits u8"));
        assert_eq!(offset % 256, 0, "block {block} is a legal storage offset");
        assert_eq!(
            u64::from(offset),
            SKIN_BLOCK_BYTES * u64::try_from(block).expect("block fits u64")
        );
    }
}

#[test]
fn skin_window_draw_matches_the_cpu_skinned_mesh() {
    let Some(mut presenter) = presenter_or_skip("skin window draw") else {
        return;
    };
    let (mesh, skin) = two_joint_cube();
    let matrices = [IDENTITY, turn_z(0.55, [0.25, 0.1, 0.0])];
    let cpu_key = MeshKey::new(0x5C00_0001, [0x11; 32]);
    let gpu_key = MeshKey::new(0x5C00_0002, [0x22; 32]);
    presenter
        .upload_mesh(cpu_key, &cpu_posed(&mesh, &skin, &matrices))
        .expect("cpu skinned mesh uploads");
    presenter
        .upload_skinned_mesh(gpu_key, &mesh, &skin, 0)
        .expect("bind mesh uploads");
    presenter
        .write_skin_block(0, &matrices)
        .expect("block writes");
    assert_eq!(presenter.skin_block_of(&gpu_key), Some(0));
    assert_eq!(presenter.skin_block_of(&cpu_key), None);
    let cpu = pixels(&mut presenter, &list_of(&[cpu_key]));
    let gpu = pixels(&mut presenter, &list_of(&[gpu_key]));
    assert!(covered(&cpu) > 400, "the posed cube is on screen");
    assert!(covered(&gpu) > 400, "the GPU-skinned cube is on screen");
    // The golden rule (mean error and far-off share), with the far-off
    // level this test always held: 3 levels, tighter than the default 12.
    let rule = crate::pass_truth::frame_tolerance::FrameTolerance {
        over_level: 3,
        ..crate::pass_truth::frame_tolerance::FrameTolerance::GOLDEN
    };
    let verdict = crate::pass_truth::frame_tolerance::frames_match(&cpu, &gpu, &rule)
        .expect("same-size RGBA frames compare");
    eprintln!("skin window vs cpu: {}", verdict.describe());
    assert!(
        verdict.matches(),
        "GPU skin drifts from the CPU skin: {}",
        verdict.describe()
    );
    assert!(
        verdict.worst <= 40,
        "no pixel is far off, got {}",
        verdict.worst
    );
    // The joints drive the picture: identity matrices draw the bind
    // pose, which must differ from the posed frame on many pixels.
    presenter
        .write_skin_block(0, &[IDENTITY, IDENTITY])
        .expect("bind pose writes");
    let bind = pixels(&mut presenter, &list_of(&[gpu_key]));
    let (_, moved) = compare(&bind, &gpu, 3);
    assert!(
        moved > 150,
        "the pose changes the frame, got {moved} pixels"
    );
}

#[test]
fn a_skinned_part_with_no_triangles_uploads_and_draws_nothing() {
    // The hero kit takes the suit's triangles off a body part: its
    // vertices stay and its index list is empty. Every pass slices the
    // index buffer, so a zero-size one panicked the arena's first frame.
    let Some(mut presenter) = presenter_or_skip("empty skinned part") else {
        return;
    };
    let (mut stripped, skin) = two_joint_cube();
    stripped.indices.clear();
    let (body, body_skin) = two_joint_cube();
    let empty = MeshKey::new(0x5C00_0020, [0x41; 32]);
    let full = MeshKey::new(0x5C00_0021, [0x42; 32]);
    presenter
        .upload_skinned_mesh(empty, &stripped, &skin, 0)
        .expect("a part with no triangles uploads");
    presenter
        .upload_skinned_mesh(full, &body, &body_skin, 1)
        .expect("full body");
    presenter
        .write_skin_block(0, &[IDENTITY, IDENTITY])
        .expect("block 0");
    presenter
        .write_skin_block(1, &[IDENTITY, IDENTITY])
        .expect("block 1");
    let alone = pixels(&mut presenter, &list_of(&[empty]));
    assert_eq!(covered(&alone), 0, "no triangles draw no pixels");
    let receipt = presenter
        .render_live_offscreen(&list_of(&[empty, full]))
        .expect("the live frame renders past the empty part");
    assert!(receipt.is_clean(), "{receipt:?}");
}

#[test]
fn never_empty_pads_only_an_empty_buffer() {
    assert_eq!(super::never_empty(Vec::new()), vec![0; 4]);
    assert_eq!(super::never_empty(vec![7, 8]), vec![7, 8]);
}

#[test]
fn skin_blocks_pose_two_bodies_independently_in_one_frame() {
    let Some(mut presenter) = presenter_or_skip("skin two bodies") else {
        return;
    };
    let (mesh, skin) = two_joint_cube();
    let a = MeshKey::new(0x5C00_0010, [0x31; 32]);
    let b = MeshKey::new(0x5C00_0011, [0x32; 32]);
    let plain = MeshKey::new(0x5C00_0012, [0x33; 32]);
    presenter
        .upload_skinned_mesh(a, &mesh, &skin, 0)
        .expect("a");
    presenter
        .upload_skinned_mesh(b, &mesh, &skin, 3)
        .expect("b");
    presenter.upload_mesh(plain, &v1::cube()).expect("plain");
    let pose_a = [IDENTITY, turn_z(0.5, [0.2, 0.0, 0.0])];
    let pose_b = [IDENTITY, turn_z(-0.5, [-0.2, 0.0, 0.0])];
    presenter.write_skin_block(0, &pose_a).expect("block 0");
    presenter.write_skin_block(3, &pose_b).expect("block 3");
    // Mixed order: static and skinned groups interleave in the list.
    let list = list_of(&[a, plain, b]);
    let frame = pixels(&mut presenter, &list);
    // Each body alone reads its own block: the two-body frame holds
    // at least the pixels either body covers.
    let only_a = pixels(&mut presenter, &list_of(&[a]));
    let only_b = pixels(&mut presenter, &list_of(&[b]));
    assert!(covered(&only_a) > 300 && covered(&only_b) > 300);
    let (_, apart) = compare(&only_a, &only_b, 3);
    assert!(
        apart > 100,
        "two blocks pose two shapes, got {apart} pixels"
    );
    assert!(covered(&frame) >= covered(&only_a).max(covered(&only_b)));
    // The live window frame takes the same draws without a validation error.
    let receipt = presenter
        .render_live_offscreen(&list)
        .expect("live frame renders");
    assert!(receipt.is_clean(), "{receipt:?}");
    assert_eq!(receipt.draws, 3);
    // And so does the 4x path (the plain 1x frame stands in when the
    // adapter cannot build the multisample pair).
    presenter.set_msaa_enabled(true);
    let receipt = presenter
        .render_live_offscreen(&list)
        .expect("4x live frame renders");
    assert!(receipt.is_clean(), "{receipt:?}");
}

#[test]
fn skin_uploads_and_blocks_refuse_bad_input() {
    let Some(mut presenter) = presenter_or_skip("skin refusals") else {
        return;
    };
    let (mesh, skin) = two_joint_cube();
    let key = MeshKey::new(0x5C00_0020, [0x41; 32]);
    let before = presenter.mesh_count();
    // A block past the buffer.
    assert!(matches!(
        presenter.upload_skinned_mesh(key, &mesh, &skin, SKIN_BLOCKS),
        Err(PresentError::InvalidMesh(_))
    ));
    // A skin row count that differs from the vertex count.
    let mut short = skin.clone();
    short.joints.pop();
    short.weights.pop();
    assert!(matches!(
        presenter.upload_skinned_mesh(key, &mesh, &short, 0),
        Err(PresentError::InvalidMesh(_))
    ));
    // A joint index past the palette.
    let mut wild = skin.clone();
    wild.joints[0][0] = u16::try_from(SKIN_BLOCK_JOINTS).expect("fits");
    assert!(matches!(
        presenter.upload_skinned_mesh(key, &mesh, &wild, 0),
        Err(PresentError::InvalidMesh(_))
    ));
    assert_eq!(presenter.mesh_count(), before, "a refusal changes nothing");
    assert_eq!(presenter.skin_block_of(&key), None);
    // Block writes: past the buffer, too many matrices, a NaN component.
    assert!(presenter
        .write_skin_block(SKIN_BLOCKS, &[IDENTITY])
        .is_err());
    let too_many = vec![IDENTITY; SKIN_BLOCK_JOINTS + 1];
    assert!(presenter.write_skin_block(0, &too_many).is_err());
    let mut nan = IDENTITY;
    nan[3][1] = f32::NAN;
    assert!(presenter.write_skin_block(0, &[IDENTITY, nan]).is_err());
    presenter
        .write_skin_block(SKIN_BLOCKS - 1, &vec![IDENTITY; SKIN_BLOCK_JOINTS])
        .expect("the last block takes a full palette");
    presenter
        .write_skin_block(0, &[])
        .expect("empty write is a no-op");
}

#[test]
fn skin_key_reuploaded_static_leaves_the_skin_path() {
    let Some(mut presenter) = presenter_or_skip("skin key replaced") else {
        return;
    };
    let (mesh, skin) = two_joint_cube();
    let key = MeshKey::new(0x5C00_0030, [0x51; 32]);
    presenter
        .upload_skinned_mesh(key, &mesh, &skin, 1)
        .expect("skinned");
    assert_eq!(presenter.skin_block_of(&key), Some(1));
    // The same key as a 48-byte static mesh: the 80-byte buffer is
    // replaced, never rewritten in place, and the draw uses `vs`.
    presenter.upload_mesh(key, &v1::cube()).expect("static");
    assert_eq!(presenter.skin_block_of(&key), None);
    let frame = pixels(&mut presenter, &list_of(&[key]));
    assert!(covered(&frame) > 400, "the replaced cube draws");
    // And back: a skinned re-upload takes the key over again.
    presenter
        .upload_skinned_mesh(key, &mesh, &skin, 2)
        .expect("skinned again");
    assert_eq!(presenter.skin_block_of(&key), Some(2));
}

#[test]
fn presenter_text_line_reaches_the_capture() {
    let Some(mut presenter) = presenter_or_skip("presenter text line") else {
        return;
    };
    let list = list_of(&[]);
    presenter.clear_overlay();
    let bare = pixels(&mut presenter, &list);
    presenter.clear_overlay();
    assert!(presenter.push_text(0.1, 0.1, 0.2, [255, 255, 255, 255], "III"));
    let text = pixels(&mut presenter, &list);
    let (worst, over) = compare(&bare, &text, 3);
    assert!(over > 0, "text pixels differ, worst {worst}");
}
