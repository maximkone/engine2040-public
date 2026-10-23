use super::*;
use std::collections::HashMap;

/// The position of one vertex.
fn position(mesh: &RimMesh, index: u32) -> [f32; 3] {
    mesh.vertices[usize::try_from(index).unwrap_or(0)].position_m
}

/// Every directed edge of the mesh, one per triangle corner.
///
/// The rim is flat shaded, so two faces that share an edge share its
/// points, not their vertex indices: the closure proof walks the points.
fn directed_edges(mesh: &RimMesh) -> Vec<([u32; 3], [u32; 3])> {
    let mut edges = Vec::with_capacity(mesh.indices.len());
    for triangle in mesh.indices.chunks_exact(3) {
        let first = bits(position(mesh, triangle[0]));
        let second = bits(position(mesh, triangle[1]));
        let third = bits(position(mesh, triangle[2]));
        edges.push((first, second));
        edges.push((second, third));
        edges.push((third, first));
    }
    edges
}

/// The bits of one point: the key of an edge in the closure proof.
fn bits(point: [f32; 3]) -> [u32; 3] {
    [point[0].to_bits(), point[1].to_bits(), point[2].to_bits()]
}

/// The signed volume of the mesh, in cubic meters: positive when the
/// faces look out of the solid.
fn signed_volume(mesh: &RimMesh) -> f32 {
    let mut total = 0.0f32;
    for triangle in mesh.indices.chunks_exact(3) {
        let first = position(mesh, triangle[0]);
        let second = position(mesh, triangle[1]);
        let third = position(mesh, triangle[2]);
        let cross = [
            second[1] * third[2] - second[2] * third[1],
            second[2] * third[0] - second[0] * third[2],
            second[0] * third[1] - second[1] * third[0],
        ];
        total += first[0] * cross[0] + first[1] * cross[1] + first[2] * cross[2];
    }
    total / 6.0
}

/// The area of one triangle, in square meters.
fn triangle_area(mesh: &RimMesh, triangle: &[u32]) -> f32 {
    let first = position(mesh, triangle[0]);
    let second = position(mesh, triangle[1]);
    let third = position(mesh, triangle[2]);
    let one = [
        second[0] - first[0],
        second[1] - first[1],
        second[2] - first[2],
    ];
    let two = [
        third[0] - first[0],
        third[1] - first[1],
        third[2] - first[2],
    ];
    let cross = [
        one[1] * two[2] - one[2] * two[1],
        one[2] * two[0] - one[0] * two[2],
        one[0] * two[1] - one[1] * two[0],
    ];
    (cross[0] * cross[0] + cross[1] * cross[1] + cross[2] * cross[2]).sqrt() * 0.5
}

/// The accept bar: the outline table builds a closed mesh with finite
/// normals, for every preset.
#[test]
fn viewmodel_outline_table_builds_a_closed_mesh_with_finite_normals() {
    for preset in AperturePreset::all() {
        let mesh = build_rim_mesh(preset).expect("the table row builds");
        assert!(
            mesh.triangle_count() > 0,
            "{:?} built no triangle at all",
            preset
        );
        // Every normal is a finite unit vector.
        for vertex in &mesh.vertices {
            assert!(
                vertex.normal.iter().all(|value| value.is_finite()),
                "{:?}: a normal is not finite: {:?}",
                preset,
                vertex.normal
            );
            let length = (vertex.normal[0] * vertex.normal[0]
                + vertex.normal[1] * vertex.normal[1]
                + vertex.normal[2] * vertex.normal[2])
                .sqrt();
            assert!(
                length > 0.999 && length < 1.001,
                "{:?}: a normal is not unit length: {length}",
                preset
            );
            assert!(
                vertex.position_m.iter().all(|value| value.is_finite()),
                "{:?}: a position is not finite",
                preset
            );
        }
        // No triangle is degenerate.
        for triangle in mesh.indices.chunks_exact(3) {
            assert!(
                triangle_area(&mesh, triangle) > 1e-9,
                "{:?}: a triangle is degenerate",
                preset
            );
        }
        // The mesh is closed: every edge is used once and its opposite
        // once, so no edge of the rim is open.
        let mut counts: HashMap<([u32; 3], [u32; 3]), usize> = HashMap::new();
        for edge in directed_edges(&mesh) {
            *counts.entry(edge).or_insert(0) += 1;
        }
        for (edge, count) in &counts {
            assert_eq!(*count, 1, "{:?}: an edge is used twice: {edge:?}", preset);
            let opposite = counts.get(&(edge.1, edge.0)).copied().unwrap_or(0);
            assert_eq!(
                opposite, 1,
                "{:?}: the edge {edge:?} has no opposite edge",
                preset
            );
        }
        // The faces look out of the solid: the volume is positive.
        let volume = signed_volume(&mesh);
        assert!(
            volume > 0.0,
            "{:?}: the signed volume is {volume}, the faces are inside out",
            preset
        );
    }
}

/// The accept bar: the depth range never lets a wall in front of the eye
/// cover the rim.
#[test]
fn viewmodel_depth_range_never_lets_a_wall_in_front_of_the_eye_cover_the_rim() {
    // The world's own target: the wall five centimetres in front of the
    // eye is drawn, and so is a wall nearer than the rim.
    let mut world = DepthTarget::new(DepthRange::new(0.001, 100.0).expect("a world range"));
    assert!(
        world.write(0.05),
        "the wall five centimetres in front of the eye is drawn"
    );
    assert!(
        world.write(0.01),
        "the wall one centimetre in front of the eye is drawn"
    );
    // The rim draws in its own range on its own target, so the world
    // never reaches it. Nearer points write first: the depth of a point
    // falls as the point comes closer.
    let mut rim = DepthTarget::new(VIEWMODEL_DEPTH);
    for z in [0.070, 0.060, 0.054] {
        assert!(rim.write(z), "the rim at {z} is drawn");
    }
    // In the world's target the nearer wall covers the rim: the world's
    // depth would have hidden it.
    assert!(
        !world.write(0.060),
        "the wall covers the rim in the world's target"
    );
    // The rim's own target still holds the rim.
    assert!(rim.depth() < 1.0, "the rim wrote its depth");
    // The rim is inside its own range, so its near plane does not
    // clip it, and the wall is inside that range too: only the
    // separate target saves the rim.
    assert!(
        VIEWMODEL_DEPTH.depth_at(0.060).is_some(),
        "the rim must sit inside its own range"
    );
    assert!(
        VIEWMODEL_DEPTH.depth_at(0.05).is_some(),
        "the wall sits inside the rim's range too"
    );
    const {
        assert!(VIEWMODEL_DEPTH.near_m > 0.0 && VIEWMODEL_DEPTH.far_m > 0.05);
    }
    // A range refuses a bad pair of planes.
    assert!(DepthRange::new(0.0, 1.0).is_err());
    assert!(DepthRange::new(1.0, 0.5).is_err());
    assert!(DepthRange::new(f32::NAN, 1.0).is_err());
}

/// The rim takes the scene light and the dirt and the blood.
#[test]
fn viewmodel_rim_takes_the_scene_light_and_the_dirt_and_blood_decals() {
    let light = RimLight::scene();
    let away = [
        -light.direction[0],
        -light.direction[1],
        -light.direction[2],
    ];
    let facing = light.shade([0.5, 0.5, 0.5], light.direction, 0.5);
    let shadowed = light.shade([0.5, 0.5, 0.5], away, 0.5);
    assert!(
        facing[0] > shadowed[0],
        "the scene light must brighten the side it hits: {facing:?} against {shadowed:?}"
    );
    assert!(
        facing.iter().all(|value| value.is_finite()),
        "the lit color must be finite"
    );
    // The decals paint the iron: blood reddens it, dirt browns it.
    let mut mesh = build_rim_mesh(AperturePreset::FaceMask).expect("the table row builds");
    let bare = mesh.vertices[0];
    let blood = RimDecal {
        kind: DecalKind::Blood,
        at_mm: [bare.position_m[0] * 1000.0, bare.position_m[1] * 1000.0],
        radius_mm: 4.0,
        strength: 1.0,
    };
    apply_decals(&mut mesh, &[blood]);
    let painted = mesh.vertices[0];
    assert!(
        painted.color[0] > bare.color[0],
        "blood must redden the iron: {:?} against {:?}",
        painted.color,
        bare.color
    );
    assert!(
        painted.color[1] < bare.color[1],
        "blood must take the green away"
    );
    assert!(
        painted.roughness < bare.roughness,
        "wet blood is glossier than the iron"
    );
    let dirt = RimDecal {
        kind: DecalKind::Dirt,
        at_mm: [bare.position_m[0] * 1000.0, bare.position_m[1] * 1000.0],
        radius_mm: 4.0,
        strength: 1.0,
    };
    let mut dirty = build_rim_mesh(AperturePreset::FaceMask).expect("the table row builds");
    let clean = dirty.vertices[0];
    apply_decals(&mut dirty, &[dirt]);
    let soiled = dirty.vertices[0];
    assert!(
        soiled.color[0] > clean.color[0] && soiled.color[2] < clean.color[2],
        "dirt must brown the iron: {:?} against {:?}",
        soiled.color,
        clean.color
    );
    assert!(
        soiled.roughness > clean.roughness,
        "dirt is rougher than the iron"
    );
    // The worn set paints every preset the same way.
    assert!(
        !worn_decals().is_empty(),
        "the wearer carries dirt and blood"
    );
}

/// The table holds the two presets the row names.
#[test]
fn viewmodel_outline_table_holds_the_two_presets() {
    assert_eq!(AperturePreset::all().len(), 2, "two presets ship");
    assert_eq!(AperturePreset::GaleaCheekGuards.name(), "galea");
    assert_eq!(AperturePreset::FaceMask.name(), "face_mask");
    let galea = AperturePreset::GaleaCheekGuards.table_row();
    assert_eq!(galea.holes.len(), 0, "the galea has no hole");
    assert_eq!(galea.bars.len(), 2, "the galea has two cheek guards");
    assert!(
        galea.bars.iter().all(|bar| bar.kind == BarKind::CheekGuard),
        "the galea's bars are cheek guards"
    );
    let mask = AperturePreset::FaceMask.table_row();
    assert_eq!(mask.holes.len(), 2, "the face mask has two eye holes");
    assert!(
        mask.holes.iter().all(|hole| hole.kind == HoleKind::Eye),
        "the mask's holes are eye holes"
    );
    assert_eq!(mask.bars.len(), 1, "the face mask has one nose bar");
    assert_eq!(mask.bars[0].kind, BarKind::NoseBar);
    for preset in AperturePreset::all() {
        let row = preset.table_row();
        assert!(
            row.band_mm > 0.0 && row.band_mm < row.half_width_mm,
            "{:?}: the band must fit the opening",
            preset
        );
        assert!(row.depth_mm > 0.0, "{:?}: the rim has depth", preset);
        for hole in row.holes {
            assert!(
                hole.surround_mm > 0.0,
                "{:?}: a hole needs a ring around it",
                preset
            );
            assert!(
                hole.center_mm[0].abs() + hole.half_mm[0] + hole.surround_mm < row.half_width_mm,
                "{:?}: the eye ring eats the opening",
                preset
            );
        }
    }
}

/// The same preset gives the same mesh twice.
#[test]
fn viewmodel_mesh_replays_byte_exact_twice() {
    for preset in AperturePreset::all() {
        let first = build_rim_mesh(preset).expect("the table row builds");
        let second = build_rim_mesh(preset).expect("the table row builds");
        assert_eq!(first, second, "{:?}: the mesh replay split", preset);
        let mut painted = first.clone();
        apply_decals(&mut painted, &worn_decals());
        let mut again = build_rim_mesh(preset).expect("the table row builds");
        apply_decals(&mut again, &worn_decals());
        assert_eq!(painted, again, "{:?}: the decals replay split", preset);
    }
}

/// The projection and the depth range agree: the depth a point gets
/// is the one the pass writes.
#[test]
fn viewmodel_projection_agrees_with_the_depth_range() {
    let projection =
        viewmodel_view_proj(VIEWMODEL_DEPTH, 1.05, 16.0 / 9.0).expect("a valid projection");
    for z in [0.006, 0.02, 0.1, 0.5] {
        let depth = VIEWMODEL_DEPTH
            .depth_at(z)
            .expect("the point sits inside the range");
        let clip_z = projection[10] * z + projection[14];
        let clip_w = projection[11] * z + projection[15];
        let projected = clip_z / clip_w;
        assert!(
            (projected - depth).abs() < 1e-6,
            "the projection and the range disagree at {z}: {projected} against {depth}"
        );
    }
    // A point behind the eye or past the far plane is outside.
    assert!(VIEWMODEL_DEPTH.depth_at(-0.1).is_none());
    assert!(VIEWMODEL_DEPTH.depth_at(0.6).is_none());
    // A bad field of view or aspect is refused.
    assert!(viewmodel_view_proj(VIEWMODEL_DEPTH, 0.0, 1.0).is_err());
    assert!(viewmodel_view_proj(VIEWMODEL_DEPTH, 1.0, 0.0).is_err());
    assert!(viewmodel_view_proj(VIEWMODEL_DEPTH, f32::NAN, 1.0).is_err());
}

/// The naga gate takes the viewmodel shader.
#[cfg(feature = "shader-validate")]
#[test]
fn viewmodel_shader_validates_under_the_naga_gate() {
    crate::shader_gate::validate_source("viewmodel.wgsl", VIEWMODEL_WGSL)
        .expect("the viewmodel shader validates");
}

/// The capture pair: the helmet in view beside each reference
/// picture, with the wall five centimetres in front of the eye.
#[cfg(feature = "gpu")]
#[test]
fn viewmodel_capture_pair_shows_the_helmet_in_view() {
    let Some((device, queue)) = device_or_skip("viewmodel_capture_pair_shows_the_helmet_in_view")
    else {
        return;
    };
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/leg2/viewmodel");
    let captures = gpu::capture_presets(&device, &queue, &dir, &AperturePreset::all(), 320, 180)
        .expect("the captures run");
    assert_eq!(captures.len(), 2, "one capture per preset");
    for capture in &captures {
        assert_eq!(
            capture.validation_errors, 0,
            "{:?}: the pass must be validation clean",
            capture.preset
        );
        assert!(
            capture.rim_pixels > 1000,
            "{:?}: the rim must be in view, got {} pixels",
            capture.preset,
            capture.rim_pixels
        );
        assert!(
            capture.rim_pixels_over_wall > 0,
            "{:?}: the wall five centimetres in front must not cover the rim",
            capture.preset
        );
        assert!(
            capture.mean_luma > 0.01,
            "{:?}: the capture is blank",
            capture.preset
        );
        assert!(
            std::path::Path::new(&capture.path).exists(),
            "{:?}: the capture file is missing: {}",
            capture.preset,
            capture.path
        );
        assert_eq!(
            capture.pixels.len(),
            320 * 180,
            "{:?}: one capture pixel per texel",
            capture.preset
        );
        println!(
            "viewmodel capture: preset={:?} file={} rim_px={} rim_px_over_wall={} mean_luma={:.4}",
            capture.preset,
            capture.path,
            capture.rim_pixels,
            capture.rim_pixels_over_wall,
            capture.mean_luma,
        );
    }
}

/// A device for the GPU tests, or a skip when the machine has none.
#[cfg(feature = "gpu")]
fn device_or_skip(test: &str) -> Option<(wgpu::Device, wgpu::Queue)> {
    use crate::test_gpu::{require_or_skip, NoAdapter};
    crate::test_gpu::hold_gpu_slot();
    // PRIMARY: the test builds skip GL, whose driver DLL can wedge the
    // loader lock.
    let backends = wgpu::Backends::PRIMARY;
    let instance = crate::backend::new_instance(backends);
    let probe = pollster::block_on(crate::presenter_pick::pick_adapter(
        &instance,
        backends,
        &crate::adapter::SelectionRequest::from_env(),
    ))
    .map_err(|error| NoAdapter(error.to_string()));
    let (adapter, _report) = require_or_skip(test, probe)?;
    match pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("viewmodel-test"),
        ..Default::default()
    })) {
        Ok((device, queue)) => Some((device, queue)),
        Err(error) => panic!("adapter present, device request must succeed: {error}"),
    }
}
