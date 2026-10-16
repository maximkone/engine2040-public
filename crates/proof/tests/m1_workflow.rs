//! MV1 workflow receipt: one licensed imported scene end to end.
//!
//! Consumer: the MV1 gate on the sprint board. Data owner: the runtime lanes
//! (assets import, editor commands/save/child-play, scene codec, render V1
//! manifest) — this harness only drives their public APIs and records the
//! receipt; any missing runtime API returns to its owner lane, nothing is
//! reimplemented here. Units follow each owner lane (meters/radians for
//! poses, bytes for digests). Determinism scope: every input is a fixed
//! fixture or a seeded flow, so the same checkout always records the same
//! artifact digests; only the editor's random [`ObjectId`]s differ per run,
//! and the receipt carries counts and digests, never raw ids.
//!
//! The licensed fixture is one Khronos CC0 glTF scene
//! (`fixtures/khronos/Triangle.gltf`, byte length pinned to the `EVIDENCE.md`
//! row) plus one Poly Haven CC0 HDRI (`content/arena/hdri/polyhaven-studio-small-09-1k.hdr`)
//! whose digest is recorded as provenance. The HDRI bytes are hashed, not
//! decoded: texture decode lives in the assets lane and IBL sampling in the
//! render lane; the proof lane records that the licensed bytes arrived intact.
//! Real pixels stay R1/Q1's receipt — `v1_capture` validates the R1 capture
//! manifest shape and the mesh-upload data path, never a GPU frame.
//!
//! [`ObjectId`]: engine_scene::ids::ObjectId

use engine_assets::{content_hash, import_gltf_bytes, sha_hex, texture_asset_id, AssetRegistry};
use engine_editor::commands::{apply, redo, undo, validate, Batch, Document, Journal, Op};
use engine_editor::play::{ChildPlay, PlayEntrypoint};
use engine_editor::save::{load_from_bytes, save_to_bytes};
use engine_proof::m1::{emit_receipt, fnv_hex, Receipt};
use engine_render::v1::{upload_mesh, Manifest, RecreateOutcome, SwapState};
use engine_scene::codec::Scene;
use engine_scene::ids::{content_digest, ObjectId};
use engine_scene::transform::Transform;

/// Licensed scene fixture: Khronos CC0 glTF (EVIDENCE.md: 1192 B).
static TRIANGLE: &[u8] = include_bytes!("../../../fixtures/khronos/Triangle.gltf");
/// Licensed HDRI fixture: Poly Haven CC0 (EVIDENCE.md: 1615248 B).
static STUDIO_HDRI: &[u8] =
    include_bytes!("../../../content/arena/hdri/polyhaven-studio-small-09-1k.hdr");

/// Byte length pinned to the EVIDENCE.md provenance row for Triangle.gltf.
const TRIANGLE_LEN: usize = 1192;
/// Byte length pinned to the EVIDENCE.md provenance row for the HDRI.
const STUDIO_HDRI_LEN: usize = 1615248;

/// A stub game: sleeps ~30 s, long enough to watch and kill. Same shape as
/// the editor lane's own child-play test (`ping` on Windows, `sleep` on unix).
#[cfg(windows)]
fn stub_entrypoint() -> PlayEntrypoint {
    PlayEntrypoint::new("cmd", ["/C", "ping", "-n", "30", "127.0.0.1"])
}

/// A stub game: sleeps ~30 s, long enough to watch and kill.
#[cfg(not(windows))]
fn stub_entrypoint() -> PlayEntrypoint {
    PlayEntrypoint::new("sleep", ["30"])
}

fn pose_at(x: f32) -> Transform {
    Transform::new([x, 0.0, 0.0], [0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0])
}

fn staged_batch(ops: Vec<Op>, base_revision: u64, key: u64) -> Batch {
    Batch {
        ops,
        base_revision,
        idempotency_key: key,
        provenance: None,
    }
}

/// Normalize the non-hierarchy lanes of an editor save so two saves compare
/// by hierarchy bytes only. The layout is magic plus version plus a u64
/// revision plus nodes plus a trailing CRC32. The revision is monotonic by
/// design (undo/redo publish new revisions) and the CRC covers the revision
/// lane, so both lanes are zeroed: what remains is the version plus the node
/// payload, compared byte for byte.
fn mask_save_metadata(mut bytes: Vec<u8>) -> Vec<u8> {
    bytes[8..16].copy_from_slice(&[0u8; 8]);
    let tail = bytes.len();
    bytes[tail - 4..].copy_from_slice(&[0u8; 4]);
    bytes
}

/// Everything the workflow records, ending in the emitted receipt text.
struct Workflow {
    receipt: Receipt,
    text: String,
    doc: Document,
    tri_hex: String,
    hdr_hex: String,
    manifest_json: String,
}

/// Run the whole M1 workflow on the licensed fixture and emit the receipt.
///
/// Each numbered block is one contracted step; every block asserts its own
/// postcondition, so a failure names the step that broke, not the receipt.
fn run_workflow() -> Workflow {
    let mut receipt = Receipt::new(
        "Triangle.gltf (Khronos CC0-1.0) + studio_small_09_1k.hdr (Poly Haven CC0-1.0)",
    );

    // 1. import_fixture: the licensed glTF imports; both licensed byte blobs
    // are digest-recorded; garbage is still rejected.
    assert_eq!(
        TRIANGLE.len(),
        TRIANGLE_LEN,
        "Triangle.gltf drifted from EVIDENCE.md"
    );
    assert_eq!(
        STUDIO_HDRI.len(),
        STUDIO_HDRI_LEN,
        "studio_small_09_1k.hdr drifted from EVIDENCE.md"
    );
    let imported = import_gltf_bytes(TRIANGLE).expect("licensed glTF must import");
    assert!(!imported.meshes.is_empty(), "fixture must yield meshes");
    assert_eq!(imported.meshes.len(), imported.materials.len());
    let mesh = &imported.meshes[0];
    assert!(!mesh.positions.is_empty() && !mesh.indices.is_empty());
    assert!(mesh.indices.len().is_multiple_of(3));
    assert!(
        import_gltf_bytes(b"").is_err(),
        "empty bytes must be rejected"
    );
    let tri_hex = sha_hex(content_hash(TRIANGLE));
    let hdr_hex = sha_hex(content_hash(STUDIO_HDRI));
    let mut geometry = Vec::new();
    for point in &mesh.positions {
        for lane in point {
            geometry.extend_from_slice(&lane.to_le_bytes());
        }
    }
    let import_hex = sha_hex(content_hash(&geometry));
    receipt.push_step(
        "import_fixture",
        true,
        &format!(
            "{} mesh, {} verts, gltf {} hdri {}",
            imported.meshes.len(),
            mesh.positions.len(),
            tri_hex,
            hdr_hex
        ),
    );
    receipt.push_digest("fixture_gltf", &tri_hex);
    receipt.push_digest("fixture_hdri", &hdr_hex);
    receipt.push_digest("import_mesh", &import_hex);

    // 2. place_edit: staged batches through the editor command service —
    // preview first (no side effects), then atomic apply. Three placements
    // plus one pose edit on a tiny scene.
    let mut doc = Document::new();
    let mut journal = Journal::new();
    let root = ObjectId::new();
    let child = ObjectId::new();
    let grand = ObjectId::new();
    let placements = [(root, ObjectId::NIL), (child, root), (grand, child)];
    for (index, (id, parent)) in placements.into_iter().enumerate() {
        let batch = staged_batch(
            vec![Op::Insert {
                id,
                parent,
                local: pose_at(index as f32),
            }],
            doc.revision,
            100 + index as u64,
        );
        let preview = validate(&doc, &batch).expect("placement batch must preview");
        assert_eq!(preview.effects.len(), 1);
        assert_eq!(preview.resulting_revision, doc.revision + 1);
        let before = doc.clone();
        apply(&mut doc, &mut journal, &batch).expect("placement batch must apply");
        assert_eq!(doc.revision, before.revision + 1);
    }
    let edit = staged_batch(
        vec![Op::SetLocal {
            id: child,
            local: pose_at(7.0),
        }],
        doc.revision,
        200,
    );
    let preview = validate(&doc, &edit).expect("edit batch must preview");
    assert_eq!(preview.effects.len(), 1);
    apply(&mut doc, &mut journal, &edit).expect("edit batch must apply");
    assert_eq!(doc.hierarchy.len(), 3);
    assert_eq!(doc.revision, 4);
    receipt.push_step(
        "place_edit",
        true,
        "3 staged inserts + 1 edit, revision 4, 3 nodes",
    );

    // 3. save_reopen: editor save bytes round-trip with stable ids, and the
    // scene codec migrates plus decodes the same hierarchy.
    let bytes = save_to_bytes(&doc);
    let back = load_from_bytes(&bytes).expect("save must reopen");
    assert_eq!(back, doc, "reopen must restore ids and revision");
    assert_eq!(
        save_to_bytes(&back),
        bytes,
        "re-save must be byte-identical"
    );
    let mut scene = Scene::encode(&doc.hierarchy);
    scene.migrate().expect("current scene version must migrate");
    let tree = scene.decode().expect("codec must decode the save");
    assert_eq!(
        tree, doc.hierarchy,
        "codec roundtrip must keep the hierarchy"
    );
    receipt.push_step(
        "save_reopen",
        true,
        &format!(
            "{} save bytes roundtrip, ids stable, codec hierarchy equal",
            bytes.len()
        ),
    );

    // 4. child_play: E3 child play with the stub program; killing the child
    // mid-run leaves the authoring document byte-identical.
    let snapshot = save_to_bytes(&doc);
    let mut play = ChildPlay::spawn(&stub_entrypoint()).expect("stub child must start");
    assert!(play.is_alive(), "fresh stub must read alive");
    play.kill().expect("kill must stop the stub");
    assert!(!play.is_alive(), "killed stub must read dead");
    play.kill().expect("repeat kill must stay idempotent");
    assert_eq!(
        save_to_bytes(&doc),
        snapshot,
        "killed child must preserve the document"
    );
    receipt.push_step(
        "child_play",
        true,
        "stub spawned, killed, idempotent; document unchanged",
    );

    // 5. undo_redo_100: 100 edits, full undo, full redo on a fresh document.
    // Hierarchy bytes (revision lane masked) restore exactly both ways.
    let mut hundred = Document::new();
    let mut hundred_journal = Journal::new();
    let initial_masked = mask_save_metadata(save_to_bytes(&hundred));
    let mut parent = ObjectId::NIL;
    for i in 0..100u64 {
        let id = ObjectId::new();
        let batch = staged_batch(
            vec![Op::Insert {
                id,
                parent,
                local: pose_at(i as f32),
            }],
            hundred.revision,
            7000 + i,
        );
        apply(&mut hundred, &mut hundred_journal, &batch).expect("edit must apply");
        parent = id;
    }
    assert_eq!(hundred.revision, 100);
    let snapshot_masked = mask_save_metadata(save_to_bytes(&hundred));
    for _ in 0..100 {
        undo(&mut hundred, &mut hundred_journal).expect("undo stack must hold 100 batches");
    }
    assert!(
        hundred.hierarchy.is_empty(),
        "full undo must empty the hierarchy"
    );
    assert_eq!(
        mask_save_metadata(save_to_bytes(&hundred)),
        initial_masked,
        "full undo must restore the initial bytes"
    );
    for _ in 0..100 {
        redo(&mut hundred, &mut hundred_journal).expect("redo stack must hold 100 batches");
    }
    assert_eq!(hundred.hierarchy.len(), 100);
    assert_eq!(
        mask_save_metadata(save_to_bytes(&hundred)),
        snapshot_masked,
        "full redo must restore the snapshot bytes"
    );
    receipt.push_step(
        "undo_redo_100",
        true,
        "100 edits, undo to initial bytes, redo to snapshot bytes",
    );

    // 6. reimport_ids: identical bytes keep every id — at the importer level
    // and at the registry level (where even changed bytes keep the id).
    let again = import_gltf_bytes(TRIANGLE).expect("reimport must succeed");
    assert_eq!(again, imported, "identical bytes must import identically");
    assert_eq!(texture_asset_id(b"m1-tex"), texture_asset_id(b"m1-tex"));
    assert_ne!(texture_asset_id(b"m1-a"), texture_asset_id(b"m1-b"));
    let mut registry = AssetRegistry::new();
    let asset = registry.register(TRIANGLE, &[]).expect("register fixture");
    let kept = registry
        .reimport(asset, TRIANGLE, &[])
        .expect("reimport same bytes");
    assert_eq!(kept.kept, asset, "reimport must keep the id");
    assert!(!kept.changed, "identical bytes must change nothing");
    assert!(kept.invalidated.is_empty());
    let changed = registry
        .reimport(asset, TRIANGLE_ANOTHER, &[])
        .expect("reimport changed bytes");
    assert_eq!(changed.kept, asset, "changed bytes must still keep the id");
    assert!(changed.changed, "changed bytes must report the change");
    let reimport_hex = registry
        .digest(asset)
        .expect("digest must be stored")
        .to_string();
    receipt.push_step(
        "reimport_ids",
        true,
        &format!("import identical, registry id {asset} kept"),
    );
    receipt.push_digest("reimport", &reimport_hex);

    // 7. v1_capture: the R1 receipt shape validated as data. The imported
    // mesh uploads to the V1 mesh path, the manifest prints its stable JSON
    // with zero validation errors, and the headless swap logic survives
    // resize plus minimize. Real pixels remain R1/Q1's receipt.
    upload_mesh(&mesh.positions, &mesh.normals, &mesh.uv0, &mesh.indices)
        .expect("imported mesh must upload to the V1 path");
    let manifest = Manifest::new("m1-software", "test-driver", 320, 180, 0);
    assert!(
        manifest.is_clean(),
        "green capture needs zero validation errors"
    );
    let manifest_json = manifest.to_json();
    assert_eq!(
        manifest_json,
        "{\"adapter\":\"m1-software\",\"driver\":\"test-driver\",\"size\":[320,180],\"validation_errors\":0}"
    );
    let long = Manifest::new(&"a".repeat(300), "d", 64, 64, 0);
    assert_eq!(
        long.adapter.chars().count(),
        256,
        "adapter names stay bounded"
    );
    let mut swap = SwapState::new(320, 180);
    assert!(swap.is_live());
    assert_eq!(swap.recreate(320, 180), RecreateOutcome::Unchanged);
    assert_eq!(swap.recreate(0, 0), RecreateOutcome::Suspended);
    assert!(!swap.is_live(), "minimize must suspend drawing");
    assert_eq!(
        swap.size(),
        (320, 180),
        "suspend must keep the last good size"
    );
    assert_eq!(
        swap.recreate(640, 360),
        RecreateOutcome::Reconfigured { generation: 1 }
    );
    assert!(swap.is_live());
    receipt.push_step(
        "v1_capture",
        true,
        "imported mesh uploads; manifest clean; resize/minimize survive",
    );
    receipt.push_digest("manifest", &fnv_hex(manifest_json.as_bytes()));

    let text = emit_receipt(&receipt);
    Workflow {
        receipt,
        text,
        doc,
        tri_hex,
        hdr_hex,
        manifest_json,
    }
}

/// Same length as the fixture but different bytes: proves changed content
/// keeps the registry id while reporting the change.
static TRIANGLE_ANOTHER: &[u8] =
    include_bytes!("../../../fixtures/khronos/TriangleWithoutIndices.gltf");

#[test]
fn m1_import_licensed_fixture_passes_and_records_digests() {
    let flow = run_workflow();
    assert_eq!(flow.tri_hex, sha_hex(content_hash(TRIANGLE)));
    assert_eq!(flow.hdr_hex, sha_hex(content_hash(STUDIO_HDRI)));
    assert!(flow
        .text
        .contains("\"name\":\"import_fixture\",\"pass\":true"));
    assert!(flow.text.contains(&flow.tri_hex));
    assert!(flow.text.contains(&flow.hdr_hex));
}

#[test]
fn m1_place_edit_through_staged_command_batches() {
    let flow = run_workflow();
    assert_eq!(flow.doc.hierarchy.len(), 3);
    assert_eq!(flow.doc.revision, 4);
    assert!(flow.text.contains("\"name\":\"place_edit\",\"pass\":true"));
}

#[test]
fn m1_save_reopen_roundtrip_keeps_stable_ids() {
    let flow = run_workflow();
    let bytes = save_to_bytes(&flow.doc);
    let back = load_from_bytes(&bytes).expect("save must reopen");
    assert_eq!(back, flow.doc);
    assert_eq!(save_to_bytes(&back), bytes);
    assert!(flow.text.contains("\"name\":\"save_reopen\",\"pass\":true"));
}

#[test]
fn m1_child_play_kill_preserves_document() {
    // The workflow already killed a stub mid-run and proved the document
    // byte-identical; here the survival postcondition is rechecked directly.
    let flow = run_workflow();
    let snapshot = save_to_bytes(&flow.doc);
    let mut play = ChildPlay::spawn(&stub_entrypoint()).expect("stub child must start");
    assert!(play.is_alive());
    play.kill().expect("kill must stop the stub");
    assert!(!play.is_alive());
    assert_eq!(save_to_bytes(&flow.doc), snapshot);
    assert!(flow.text.contains("\"name\":\"child_play\",\"pass\":true"));
}

#[test]
fn m1_hundred_undo_redo_restores_exactly() {
    let mut doc = Document::new();
    let mut journal = Journal::new();
    let initial = mask_save_metadata(save_to_bytes(&doc));
    let mut parent = ObjectId::NIL;
    for i in 0..100u64 {
        let rev = doc.revision;
        let id = ObjectId::new();
        apply(
            &mut doc,
            &mut journal,
            &staged_batch(
                vec![Op::Insert {
                    id,
                    parent,
                    local: pose_at(i as f32),
                }],
                rev,
                9000 + i,
            ),
        )
        .expect("edit must apply");
        parent = id;
    }
    let snapshot = mask_save_metadata(save_to_bytes(&doc));
    assert_ne!(snapshot, initial);
    for _ in 0..100 {
        undo(&mut doc, &mut journal).expect("undo must be available");
    }
    assert_eq!(mask_save_metadata(save_to_bytes(&doc)), initial);
    for _ in 0..100 {
        redo(&mut doc, &mut journal).expect("redo must be available");
    }
    assert_eq!(mask_save_metadata(save_to_bytes(&doc)), snapshot);
    let flow = run_workflow();
    assert!(flow
        .text
        .contains("\"name\":\"undo_redo_100\",\"pass\":true"));
}

#[test]
fn m1_reimport_same_bytes_keeps_ids() {
    let first = import_gltf_bytes(TRIANGLE).expect("first import");
    let second = import_gltf_bytes(TRIANGLE).expect("second import");
    assert_eq!(first, second);
    let mut registry = AssetRegistry::new();
    let asset = registry.register(TRIANGLE, &[]).expect("register");
    let kept = registry.reimport(asset, TRIANGLE, &[]).expect("reimport");
    assert_eq!(kept.kept, asset);
    assert!(!kept.changed);
    let flow = run_workflow();
    assert!(flow
        .text
        .contains("\"name\":\"reimport_ids\",\"pass\":true"));
    // The receipt records the finally stored digest (changed-bytes reimport),
    // which the registry versions with the BLAKE3 content digest, not FNV.
    assert!(flow.text.contains(&content_digest(TRIANGLE_ANOTHER)));
}

#[test]
fn m1_v1_capture_manifest_shape_validates_as_data() {
    let manifest = Manifest::new("m1-software", "test-driver", 320, 180, 0);
    assert!(manifest.is_clean());
    assert_eq!(
        manifest.to_json(),
        "{\"adapter\":\"m1-software\",\"driver\":\"test-driver\",\"size\":[320,180],\"validation_errors\":0}"
    );
    let dirty = Manifest::new("gpu", "drv", 64, 64, 2);
    assert!(!dirty.is_clean());
    let flow = run_workflow();
    assert_eq!(flow.manifest_json, manifest.to_json());
    assert!(flow.text.contains("\"name\":\"v1_capture\",\"pass\":true"));
}

#[test]
fn m1_receipt_emits_all_steps_with_digests() {
    let flow = run_workflow();
    assert!(flow.receipt.all_pass(), "every workflow step must pass");
    for step in [
        "import_fixture",
        "place_edit",
        "save_reopen",
        "child_play",
        "undo_redo_100",
        "reimport_ids",
        "v1_capture",
    ] {
        assert!(
            flow.text
                .contains(&format!("\"name\":\"{step}\",\"pass\":true")),
            "receipt must record a passing {step}"
        );
    }
    for label in [
        "fixture_gltf",
        "fixture_hdri",
        "import_mesh",
        "reimport",
        "manifest",
    ] {
        assert!(
            flow.text.contains(&format!("\"label\":\"{label}\"")),
            "receipt must record the {label} digest"
        );
    }
    assert!(flow.text.contains("\"workflow\":\"m1_workflow\""));
    assert!(flow.text.contains("\"all_pass\":true"));
    // The emitter is deterministic: same receipt, same text.
    assert_eq!(flow.text, emit_receipt(&flow.receipt));
}
