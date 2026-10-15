//! optimize vectors: dup-heavy fixture shrinks, x2 identical, control no-op.
//!
//! Owner: engine-assets cook (`crates/assets/src/optimize.rs`). Units: docs
//! in, bytes plus digests plus counts out. Determinism: fixed literals only,
//! no RNG, no clock, no IO. Errors: over-bound inputs refuse with typed error.
use engine_assets::optimize::{
    cook_normalized_bytes, document_bytes, normalize_digest, normalize_document, DocImage,
    DocMaterial, DocMesh, Document, OptimizeError, MAX_TEXTURE_PX,
};

/// One triangle mesh with the given UV fill plus material slot.
fn tri_mesh(uv: [f32; 2], material: usize) -> DocMesh {
    DocMesh {
        positions: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
        indices: vec![0, 1, 2],
        uvs: vec![uv, uv, uv],
        material,
    }
}

/// Dup-heavy fixture: 3 byte-identical meshes plus materials, 1 referenced
/// 4x4 texture, 1 unreferenced 2048px texture, 1 unused UV set elsewhere.
fn dup_heavy() -> (Document, u64, u64) {
    let mesh = tri_mesh([0.25, 0.75], 0);
    let material = DocMaterial {
        factor: [1.0, 1.0, 1.0, 1.0],
        texture: Some(7),
    };
    let small = DocImage {
        id: 7,
        bytes: vec![1, 2, 3, 4],
        width: 4,
        height: 4,
    };
    let big = DocImage {
        id: 9,
        bytes: vec![5u8; 256],
        width: 2048,
        height: 2048,
    };
    let doc = Document {
        meshes: vec![mesh.clone(), mesh.clone(), mesh],
        materials: vec![material.clone(), material.clone(), material],
        images: vec![small, big],
    };
    (doc, 7, 9)
}

#[test]
fn dup_heavy_shrinks_primitives_fall_unreferenced_drops() {
    let (doc, kept, dropped) = dup_heavy();
    assert_eq!(doc.meshes.len(), 3, "fixture carries 3 duplicated meshes");
    let pre = document_bytes(&doc);
    let normalized = normalize_document(&doc).expect("dup-heavy normalizes");
    let post = document_bytes(&normalized);
    assert_eq!(
        normalized.meshes.len(),
        1,
        "3 duplicated meshes fall to 1 shared mesh"
    );
    assert_eq!(
        normalized.materials.len(),
        1,
        "identical materials merge to 1"
    );
    assert!(
        normalized.images.iter().any(|i| i.id == kept),
        "referenced texture survives"
    );
    assert!(
        !normalized.images.iter().any(|i| i.id == dropped),
        "unreferenced texture drops"
    );
    let unreferenced_bytes: usize = doc
        .images
        .iter()
        .filter(|i| i.id == dropped)
        .map(|i| i.bytes.len())
        .sum();
    assert!(
        unreferenced_bytes > 0,
        "dropped texture carried bytes pre-pass"
    );
    let kept_pre = doc
        .images
        .iter()
        .find(|i| i.id == kept)
        .expect("kept pre")
        .bytes
        .clone();
    let kept_post = normalized
        .images
        .iter()
        .find(|i| i.id == kept)
        .expect("kept post")
        .bytes
        .clone();
    assert_eq!(kept_pre, kept_post, "referenced texture pixels unchanged");
    let shrink = pre.saturating_sub(post);
    assert!(
        shrink.saturating_mul(100) >= pre.saturating_mul(15),
        "cooked bytes shrink >=15%: {pre} -> {post}"
    );
    let (again, again_bytes, again_digest) =
        cook_normalized_bytes(&doc).expect("cook entry normalizes");
    assert_eq!(again, normalized, "cook entry returns the normalized doc");
    assert_eq!(again_bytes, post, "cook entry reports post bytes");
    assert_eq!(
        again_digest,
        normalize_digest(&normalized),
        "cook entry reports the digest"
    );
    let second = normalize_document(&doc).expect("second run normalizes");
    assert_eq!(
        normalize_digest(&second),
        normalize_digest(&normalized),
        "normalize digest x2 byte-equal"
    );
    assert_eq!(document_bytes(&second), post, "normalize bytes x2 equal");
}

#[test]
fn unused_uvs_zero_out_and_control_round_trips() {
    let uv_mesh = tri_mesh([0.9, 0.1], 0);
    let plain = Document {
        meshes: vec![uv_mesh],
        materials: vec![DocMaterial {
            factor: [0.5, 0.5, 0.5, 1.0],
            texture: None,
        }],
        images: vec![],
    };
    let pruned = normalize_document(&plain).expect("uv-only doc normalizes");
    assert!(
        pruned.meshes[0].uvs.iter().all(|uv| *uv == [0.0, 0.0]),
        "unused TEXCOORD set prunes to zero"
    );
    assert_eq!(
        pruned.meshes[0].positions, plain.meshes[0].positions,
        "referenced positions unchanged"
    );
    let control = Document {
        meshes: vec![tri_mesh([0.0, 0.0], 0)],
        materials: vec![DocMaterial {
            factor: [1.0, 1.0, 1.0, 1.0],
            texture: None,
        }],
        images: vec![],
    };
    let again = normalize_document(&control).expect("control normalizes");
    assert_eq!(again, control, "deduped control normalizes to itself");
    assert_eq!(
        normalize_digest(&again),
        normalize_digest(&control),
        "control digest stable"
    );
    assert!(
        MAX_TEXTURE_PX >= 1_048_576,
        "pixel cap holds the potato tier"
    );
}

#[test]
fn oversize_inputs_refuse_typed() {
    let (mut doc, _, _) = dup_heavy();
    doc.meshes = vec![tri_mesh([0.0, 0.0], 0); 2048];
    let err = normalize_document(&doc).expect_err("over-mesh input must refuse");
    assert_eq!(
        err,
        OptimizeError::TooLarge { count: 2048 },
        "mesh bound names the count"
    );
    assert!(!format!("{err}").is_empty(), "refusal names the reason");
}
