//! CT10 glTF conformance: every selected model imports, places, saves,
//! reopens and plays; a model the cook cannot read comes back as a typed
//! refusal, never a panic.
//!
//! Owner: engine-assets (row CT10). The corpus is the Khronos CC0 samples in
//! `fixtures/khronos/` plus the Blender-exporter variants in `fixtures/gltf/`
//! (fresh bytes; the shapes are accepted from KhronosGroup/glTF-Blender-IO,
//! Apache-2.0, acceptance only, no donor code and no donor bytes).
//!
//! Units: bytes in, cooked meshes, materials and clips out, one saved
//! `E204BNDL` bundle per model. Deterministic: fixed fixture bytes, no RNG,
//! no clock; the same model cooks the same bundle bytes twice. Errors:
//! `MeshImportError`, `SkinImportError`, `BundleError` and `BundleCookError`
//! are the only refusal shapes; a panic is a failure.

#![cfg(all(feature = "import", feature = "registry"))]

use engine_assets::bundle::{BundleWriter, EntryKind};
use engine_assets::bundle_cook::{load_bundle_file, write_bundle_file};
use engine_assets::cooked::{validate_material, validate_mesh, CookedMesh};
use engine_assets::gen_intake::check_license;
use engine_assets::gltf::validate_gltf;
use engine_assets::mesh_import::MAX_DIAGNOSTIC_CHARS;
use engine_assets::skin_import::SkinImportError;
use engine_assets::{
    content_hash, import_gltf_bytes, import_skinned_bytes, AssetRegistry, GltfImport, ImportedClip,
    Registry,
};
use std::path::PathBuf;

/// The licence ledger of the Khronos samples (the row the cook reads first).
const EVIDENCE: &str = include_str!("../../../docs/EVIDENCE.md");
/// The licence ledger of the fresh Blender-exporter variants.
const FIXTURE_README: &str = include_str!("../../../fixtures/gltf/README.md");

/// What the cook must do with one selected model.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Expect {
    /// The whole path works: import, place, save, reopen, play.
    Full,
    /// The mesh import must refuse it, typed, with no panic.
    RefuseMesh,
    /// The mesh imports but the clip import must refuse it, typed.
    RefuseClip,
}

/// One selected model: where its bytes live, its licence and the verdict.
struct Model {
    /// Stable id, used in the bundle entry names and in the messages.
    id: &'static str,
    /// Path under `fixtures/`.
    path: &'static str,
    /// SPDX licence id, read from the provenance row before the cook runs.
    license: &'static str,
    /// What the cook must do with the model.
    expect: Expect,
}

/// The selected corpus: the Khronos samples and the Blender variants.
const MODELS: &[Model] = &[
    Model {
        id: "khronos-triangle",
        path: "khronos/Triangle.gltf",
        license: "CC0-1.0",
        expect: Expect::Full,
    },
    Model {
        id: "khronos-triangle-no-indices",
        path: "khronos/TriangleWithoutIndices.gltf",
        license: "CC0-1.0",
        expect: Expect::Full,
    },
    Model {
        id: "khronos-animated-triangle",
        path: "khronos/AnimatedTriangle.gltf",
        license: "CC0-1.0",
        expect: Expect::Full,
    },
    Model {
        id: "blender-actions",
        path: "gltf/blender_actions.gltf",
        license: "CC0-1.0",
        expect: Expect::Full,
    },
    Model {
        id: "blender-broadcast",
        path: "gltf/blender_broadcast.gltf",
        license: "CC0-1.0",
        expect: Expect::Full,
    },
    Model {
        id: "blender-pbr",
        path: "gltf/blender_pbr.gltf",
        license: "CC0-1.0",
        expect: Expect::Full,
    },
    Model {
        id: "blender-step",
        path: "gltf/blender_step.gltf",
        license: "CC0-1.0",
        expect: Expect::RefuseClip,
    },
    Model {
        id: "broken-buffer",
        path: "gltf/broken_buffer.gltf",
        license: "CC0-1.0",
        expect: Expect::RefuseMesh,
    },
];

/// The bytes of one selected model.
fn bytes_of(path: &str) -> &'static [u8] {
    match path {
        "khronos/Triangle.gltf" => &include_bytes!("../../../fixtures/khronos/Triangle.gltf")[..],
        "khronos/TriangleWithoutIndices.gltf" => {
            &include_bytes!("../../../fixtures/khronos/TriangleWithoutIndices.gltf")[..]
        }
        "khronos/AnimatedTriangle.gltf" => {
            &include_bytes!("../../../fixtures/khronos/AnimatedTriangle.gltf")[..]
        }
        "gltf/blender_actions.gltf" => {
            &include_bytes!("../../../fixtures/gltf/blender_actions.gltf")[..]
        }
        "gltf/blender_broadcast.gltf" => {
            &include_bytes!("../../../fixtures/gltf/blender_broadcast.gltf")[..]
        }
        "gltf/blender_pbr.gltf" => &include_bytes!("../../../fixtures/gltf/blender_pbr.gltf")[..],
        "gltf/blender_step.gltf" => &include_bytes!("../../../fixtures/gltf/blender_step.gltf")[..],
        "gltf/broken_buffer.gltf" => {
            &include_bytes!("../../../fixtures/gltf/broken_buffer.gltf")[..]
        }
        other => panic!("skipped: unknown fixture {other}"),
    }
}

/// One selected model by id.
fn model_by_id(id: &str) -> &'static Model {
    MODELS
        .iter()
        .find(|model| model.id == id)
        .unwrap_or_else(|| panic!("skipped: the corpus has no model {id}"))
}

/// Scratch file for one saved bundle under the OS temp dir (the tests run in
/// parallel; the repo tree is never touched).
fn scratch(model: &Model) -> PathBuf {
    let mut path = std::env::temp_dir();
    path.push(format!(
        "engine-assets-ct10-{}-{}.e2040",
        model.id,
        std::process::id()
    ));
    path
}

/// Sample one position track at `time`: clamped, linear, endpoints exact.
fn sample_track(times: &[f32], values: &[[f32; 3]], time: f32) -> [f32; 3] {
    if times.is_empty() || times.len() != values.len() {
        return [0.0; 3];
    }
    if time <= times[0] {
        return values[0];
    }
    let last = times.len() - 1;
    if time >= times[last] {
        return values[last];
    }
    let mut low = 0usize;
    let mut high = last;
    while high - low > 1 {
        let mid = (low + high) / 2;
        if times[mid] <= time {
            low = mid;
        } else {
            high = mid;
        }
    }
    let span = times[high] - times[low];
    let weight = if span <= 0.0 {
        0.0
    } else {
        ((time - times[low]) / span).clamp(0.0, 1.0)
    };
    let a = values[low];
    let b = values[high];
    [
        a[0] + (b[0] - a[0]) * weight,
        a[1] + (b[1] - a[1]) * weight,
        a[2] + (b[2] - a[2]) * weight,
    ]
}

/// Sample one rotation track at `time`: clamped, normalised lerp.
fn sample_quat(times: &[f32], values: &[[f32; 4]], time: f32) -> [f32; 4] {
    if times.is_empty() || times.len() != values.len() {
        return [0.0, 0.0, 0.0, 1.0];
    }
    if time <= times[0] {
        return values[0];
    }
    let last = times.len() - 1;
    if time >= times[last] {
        return values[last];
    }
    let mut low = 0usize;
    let mut high = last;
    while high - low > 1 {
        let mid = (low + high) / 2;
        if times[mid] <= time {
            low = mid;
        } else {
            high = mid;
        }
    }
    let span = times[high] - times[low];
    let weight = if span <= 0.0 {
        0.0
    } else {
        ((time - times[low]) / span).clamp(0.0, 1.0)
    };
    let a = values[low];
    let b = values[high];
    let mut mixed = [
        a[0] + (b[0] - a[0]) * weight,
        a[1] + (b[1] - a[1]) * weight,
        a[2] + (b[2] - a[2]) * weight,
        a[3] + (b[3] - a[3]) * weight,
    ];
    let norm = mixed.iter().map(|v| v * v).sum::<f32>().sqrt();
    if norm > 1e-6 {
        for value in mixed.iter_mut() {
            *value /= norm;
        }
    } else {
        mixed = [0.0, 0.0, 0.0, 1.0];
    }
    mixed
}

/// Two quaternions describe the same rotation within the tolerance.
fn quat_close(a: [f32; 4], b: [f32; 4]) -> bool {
    a.iter().zip(b.iter()).all(|(x, y)| (x - y).abs() <= 1e-5)
}

/// Two positions are the same within the tolerance.
fn vec3_close(a: [f32; 3], b: [f32; 3]) -> bool {
    a.iter().zip(b.iter()).all(|(x, y)| (x - y).abs() <= 1e-5)
}

/// Place one model in the asset bookkeeping: the licence gate first, then a
/// persistent asset id that the same bytes keep, then the licence row.
fn place_model(model: &Model, bytes: &[u8]) {
    check_license(model.id, model.license)
        .unwrap_or_else(|error| panic!("licence refused for {}: {error}", model.id));
    let mut registry = AssetRegistry::new();
    let id = registry
        .register(bytes, &[])
        .unwrap_or_else(|error| panic!("{} must register: {error}", model.id));
    let again = registry
        .reimport(id, bytes, &[])
        .unwrap_or_else(|error| panic!("{} must reimport: {error}", model.id));
    assert_eq!(again.kept, id, "the same bytes keep the placed id");
    assert!(!again.changed, "the same bytes change nothing");
    assert!(again.invalidated.is_empty());
    // The licence rides the manifest row: an asset without one is a gap.
    let mut rows = Registry::new();
    let handle = rows.import(bytes, model.license);
    let row = rows.get(handle).expect("the placed row must exist");
    assert_eq!(row.license, model.license, "the row keeps the licence");
    assert_eq!(
        row.id,
        content_hash(bytes),
        "the row holds the content hash"
    );
}

/// Cook one model into bundle bytes: one entry per mesh, material and clip.
fn cook_bundle(model: &Model, imported: &GltfImport, clips: &[ImportedClip]) -> Vec<u8> {
    let mut writer = BundleWriter::new();
    for (index, mesh) in imported.meshes.iter().enumerate() {
        writer
            .add_mesh(&format!("{}_mesh{index}", model.id), mesh)
            .unwrap_or_else(|error| panic!("{} mesh {index} must pack: {error}", model.id));
    }
    for (index, material) in imported.materials.iter().enumerate() {
        writer
            .add_material(&format!("{}_mat{index}", model.id), material)
            .unwrap_or_else(|error| panic!("{} material {index} must pack: {error}", model.id));
    }
    for (index, clip) in clips.iter().enumerate() {
        writer
            .add_clip(&format!("{}_clip{index}", model.id), clip)
            .unwrap_or_else(|error| panic!("{} clip {index} must pack: {error}", model.id));
    }
    writer.finish()
}

/// The whole path for one model: import, place, save, reopen, play.
fn full_path(model: &Model) {
    let bytes = bytes_of(model.path);
    // 1. import: the envelope gate and the real cook agree, and the cook
    // yields cooked meshes with their materials.
    validate_gltf(bytes)
        .unwrap_or_else(|error| panic!("{} must pass the envelope gate: {error:?}", model.id));
    let imported = import_gltf_bytes(bytes)
        .unwrap_or_else(|error| panic!("{} must import: {error}", model.id));
    assert!(
        !imported.meshes.is_empty(),
        "{} must yield a mesh",
        model.id
    );
    assert_eq!(
        imported.meshes.len(),
        imported.materials.len(),
        "{} pairs every mesh with a material",
        model.id
    );
    for mesh in &imported.meshes {
        validate_mesh(mesh).unwrap_or_else(|error| panic!("{} mesh invalid: {error:?}", model.id));
    }
    for material in &imported.materials {
        validate_material(material)
            .unwrap_or_else(|error| panic!("{} material invalid: {error:?}", model.id));
    }
    // 2. place: the licence gate, the asset id and the licence row.
    place_model(model, bytes);
    // 3. the clips of the model (the play part).
    let skinned = import_skinned_bytes(bytes)
        .unwrap_or_else(|error| panic!("{} must import its clips: {error}", model.id));
    // 4. save: the cooked bundle, written to disk.
    let bundle = cook_bundle(model, &imported, &skinned.clips);
    let path = scratch(model);
    let receipt = write_bundle_file(&bundle, &path, false)
        .unwrap_or_else(|error| panic!("{} must save: {error}", model.id));
    // 5. reopen: the file comes back with every entry and its digests.
    let (loaded, back) = load_bundle_file(&path, false)
        .unwrap_or_else(|error| panic!("{} must reopen: {error}", model.id));
    assert_eq!(
        receipt.entries, back.entries,
        "{} entry count is stable",
        model.id
    );
    assert_eq!(
        loaded.len(),
        receipt.entries,
        "{} holds every entry",
        model.id
    );
    for (index, mesh) in imported.meshes.iter().enumerate() {
        let entry = loaded
            .find(EntryKind::Mesh, &format!("{}_mesh{index}", model.id))
            .unwrap_or_else(|| panic!("{} mesh {index} must be in the bundle", model.id));
        let reopened = entry
            .decode_mesh()
            .unwrap_or_else(|error| panic!("{} mesh {index} must decode: {error}", model.id));
        assert_eq!(
            reopened, *mesh,
            "{} mesh {index} survives the round trip",
            model.id
        );
    }
    for (index, material) in imported.materials.iter().enumerate() {
        let entry = loaded
            .find(EntryKind::Material, &format!("{}_mat{index}", model.id))
            .unwrap_or_else(|| panic!("{} material {index} must be in the bundle", model.id));
        let reopened = entry
            .decode_material()
            .unwrap_or_else(|error| panic!("{} material {index} must decode: {error}", model.id));
        assert_eq!(
            reopened, *material,
            "{} material {index} survives the round trip",
            model.id
        );
    }
    // 6. play: the reopened clip is the saved clip, and a clip with more than
    // one key moves: at least one key leaves the first one.
    for (index, clip) in skinned.clips.iter().enumerate() {
        let entry = loaded
            .find(EntryKind::Clip, &format!("{}_clip{index}", model.id))
            .unwrap_or_else(|| panic!("{} clip {index} must be in the bundle", model.id));
        let reopened = entry
            .decode_clip()
            .unwrap_or_else(|error| panic!("{} clip {index} must decode: {error}", model.id));
        assert_eq!(
            reopened, *clip,
            "{} clip {index} survives the round trip",
            model.id
        );
        let turns = clip
            .rotations
            .first()
            .map(|track| track.times.len())
            .unwrap_or(0);
        let slides = clip
            .positions
            .tracks
            .first()
            .map(|track| track.times.len())
            .unwrap_or(0);
        if turns > 1 {
            let track = &clip.rotations[0];
            let first = sample_quat(&track.times, &track.values, track.times[0]);
            let moved = track.values.iter().any(|value| !quat_close(first, *value));
            assert!(moved, "{} clip {index} must turn while it plays", model.id);
        }
        if slides > 1 {
            let track = &clip.positions.tracks[0];
            let first = sample_track(&track.times, &track.values, track.times[0]);
            let moved = track.values.iter().any(|value| !vec3_close(first, *value));
            assert!(moved, "{} clip {index} must slide while it plays", model.id);
        }
    }
    let _ = std::fs::remove_file(&path);
}

/// The mesh of a model, cooked.
fn meshes_of(path: &str) -> Vec<CookedMesh> {
    import_gltf_bytes(bytes_of(path))
        .unwrap_or_else(|error| panic!("{path} must import: {error}"))
        .meshes
}

/// The corpus covers the Khronos samples and the Blender-exporter variants.
#[test]
fn the_corpus_covers_the_khronos_samples_and_the_blender_variants() {
    let khronos = MODELS
        .iter()
        .filter(|model| model.path.starts_with("khronos/"))
        .count();
    let blender = MODELS
        .iter()
        .filter(|model| model.path.starts_with("gltf/"))
        .count();
    assert!(
        khronos >= 3,
        "the corpus holds the Khronos samples, has {khronos}"
    );
    assert!(
        blender >= 4,
        "the corpus holds the Blender variants, has {blender}"
    );
    assert!(
        MODELS
            .iter()
            .any(|model| model.expect == Expect::RefuseMesh),
        "the corpus holds a model the cook must refuse"
    );
    assert!(
        MODELS
            .iter()
            .any(|model| model.expect == Expect::RefuseClip),
        "the corpus holds a clip the cook must refuse"
    );
}

/// Every selected model passes the whole path, or is refused in a typed way.
#[test]
fn every_selected_model_passes_the_whole_path() {
    for model in MODELS {
        match model.expect {
            Expect::Full => full_path(model),
            Expect::RefuseMesh => {
                let bytes = bytes_of(model.path);
                let outcome = std::panic::catch_unwind(|| import_gltf_bytes(bytes));
                let error = match outcome {
                    Ok(result) => result.expect_err("a broken model must be refused"),
                    Err(_) => panic!("{} must not panic on a broken model", model.id),
                };
                assert!(
                    !format!("{error}").is_empty(),
                    "{} names the reason it refuses",
                    model.id
                );
                assert!(
                    format!("{error:?}").chars().count() <= MAX_DIAGNOSTIC_CHARS + 128,
                    "{} keeps the refusal bounded",
                    model.id
                );
            }
            Expect::RefuseClip => {
                let bytes = bytes_of(model.path);
                let imported = import_gltf_bytes(bytes)
                    .unwrap_or_else(|error| panic!("{} mesh must import: {error}", model.id));
                assert!(!imported.meshes.is_empty());
                let outcome = std::panic::catch_unwind(|| import_skinned_bytes(bytes));
                let error = match outcome {
                    Ok(result) => result.expect_err("a refused clip must be an error"),
                    Err(_) => panic!("{} must not panic on a refused clip", model.id),
                };
                assert!(
                    matches!(error, SkinImportError::UnsupportedInterpolation { .. }),
                    "{} refuses the sampling it cannot read, got {error:?}",
                    model.id
                );
            }
        }
    }
}

/// The same model cooks the same bundle bytes twice.
#[test]
fn the_saved_bundle_is_identical_twice() {
    for model in MODELS.iter().filter(|model| model.expect == Expect::Full) {
        let bytes = bytes_of(model.path);
        let imported = import_gltf_bytes(bytes)
            .unwrap_or_else(|error| panic!("{} must import: {error}", model.id));
        let skinned = import_skinned_bytes(bytes)
            .unwrap_or_else(|error| panic!("{} must import its clips: {error}", model.id));
        let first = cook_bundle(model, &imported, &skinned.clips);
        let second = cook_bundle(model, &imported, &skinned.clips);
        assert_eq!(first, second, "{} cooks identical bytes twice", model.id);
    }
}

/// Every model carries a licence the allow-list takes and the ledger names.
#[test]
fn every_model_carries_a_documented_licence() {
    for model in MODELS {
        check_license(model.id, model.license)
            .unwrap_or_else(|error| panic!("licence refused for {}: {error}", model.id));
        let ledger = if model.path.starts_with("khronos/") {
            EVIDENCE
        } else {
            FIXTURE_README
        };
        let named = ledger
            .lines()
            .any(|line| line.contains(model.path) && line.contains(model.license));
        assert!(
            named,
            "{} must be documented with the licence {}",
            model.id, model.license
        );
    }
}

/// The per-action export and the broadcast export carry the same motion.
#[test]
fn per_action_and_broadcast_carry_the_same_motion() {
    let actions = import_skinned_bytes(bytes_of("gltf/blender_actions.gltf"))
        .expect("the per-action export must import");
    let broadcast = import_skinned_bytes(bytes_of("gltf/blender_broadcast.gltf"))
        .expect("the broadcast export must import");
    assert_eq!(actions.clips.len(), 2, "one clip per Blender action");
    assert_eq!(
        broadcast.clips.len(),
        1,
        "the broadcast export holds one clip"
    );
    let turn = actions
        .clips
        .iter()
        .find(|clip| {
            clip.rotations
                .first()
                .is_some_and(|track| track.times.len() > 1)
        })
        .expect("the per-action export must turn");
    let slide = actions
        .clips
        .iter()
        .find(|clip| {
            clip.positions
                .tracks
                .first()
                .is_some_and(|track| track.times.len() > 1)
        })
        .expect("the per-action export must slide");
    let wire = &broadcast.clips[0];
    let turn_track = &turn.rotations[0];
    let wire_turn = &wire.rotations[0];
    for (index, time) in turn_track.times.iter().enumerate() {
        let want = turn_track.values[index];
        let got = sample_quat(&wire_turn.times, &wire_turn.values, *time);
        assert!(
            quat_close(want, got),
            "the broadcast rotation at {time} must match the per-action key"
        );
    }
    let slide_track = &slide.positions.tracks[0];
    let wire_slide = &wire.positions.tracks[0];
    for (index, time) in slide_track.times.iter().enumerate() {
        let want = slide_track.values[index];
        let got = sample_track(&wire_slide.times, &wire_slide.values, *time);
        assert!(
            vec3_close(want, got),
            "the broadcast translation at {time} must match the per-action key"
        );
    }
}

/// The PBR mapping of a Blender material survives the cook.
#[test]
fn the_pbr_mapping_survives_the_cook() {
    let imported =
        import_gltf_bytes(bytes_of("gltf/blender_pbr.gltf")).expect("the PBR variant must import");
    let material = imported
        .materials
        .first()
        .expect("the PBR variant must carry a material");
    assert_eq!(
        material.base_color_factor,
        [0.8, 0.4, 0.2, 1.0],
        "the base colour factor maps across"
    );
    assert_eq!(
        material.metallic_factor, 0.25,
        "the metallic factor maps across"
    );
    assert_eq!(
        material.roughness_factor, 0.75,
        "the roughness factor maps across"
    );
    assert_eq!(
        material.textures.len(),
        1,
        "the base-colour image maps to one slot"
    );
    assert_eq!(
        material.textures[0].slot,
        engine_assets::TextureSlot::BaseColor,
        "the base-colour image fills the base-colour slot"
    );
    assert_eq!(
        imported.images.len(),
        1,
        "the base-colour image bytes ride along"
    );
    let bytes = imported
        .images
        .values()
        .next()
        .expect("the base-colour image must be there");
    assert_eq!(
        &bytes[..4],
        &[0x89, 0x50, 0x4e, 0x47],
        "the image bytes are the PNG the file carries"
    );
}

/// A mesh without an index buffer cooks to the same indexed mesh.
#[test]
fn a_mesh_without_indices_cooks_to_the_same_mesh() {
    let indexed = meshes_of("khronos/Triangle.gltf");
    let plain = meshes_of("khronos/TriangleWithoutIndices.gltf");
    assert_eq!(indexed.len(), 1, "the sample holds one mesh");
    assert_eq!(plain.len(), 1, "the plain sample holds one mesh");
    assert_eq!(
        indexed[0].positions, plain[0].positions,
        "the positions are the same"
    );
    assert_eq!(
        indexed[0].indices, plain[0].indices,
        "the indices are filled in"
    );
    assert_eq!(indexed[0].indices.len(), 3, "the sample is one triangle");
}

/// The saved bundle of a model holds the model, not a stub.
#[test]
fn the_saved_bundle_holds_the_model_and_not_a_stub() {
    let model = model_by_id("khronos-triangle");
    let bytes = bytes_of(model.path);
    let imported = import_gltf_bytes(bytes).expect("the sample must import");
    let skinned = import_skinned_bytes(bytes).expect("the sample must import its clips");
    let bundle = cook_bundle(model, &imported, &skinned.clips);
    let path = scratch(model);
    let receipt = write_bundle_file(&bundle, &path, false).expect("the sample must save");
    assert!(receipt.bytes > 16, "the saved bundle is not the stub");
    assert_eq!(receipt.entries, 2, "the sample holds a mesh and a material");
    let (loaded, _) = load_bundle_file(&path, false).expect("the sample must reopen");
    assert_eq!(loaded.len(), 2, "the reopened bundle holds both entries");
    let _ = std::fs::remove_file(&path);
}
