//! SW121: the scanned marble bust is real content, not a sandstone stand-in.
//!
//! Poly Haven `Marble Bust 01` (Rico Cilliers, CC0) ships as a `.gltf`, a `.bin`
//! and three 1k maps; `tools/polyhaven-glb.mjs` wraps them into the one GLB the
//! intake reads, so the bytes are the same on every machine. The test runs the
//! real intake path: licence gate, repair, simplify to the prop budget, prop
//! cook, bundle and provenance row. It stays under `PROP_MAX_TRIS` (5,000) as
//! the row asks and cooks byte-identical twice.
#![cfg(all(feature = "import", feature = "registry"))]

use engine_assets::gen_intake::{
    intake, IntakeCache, IntakeOutput, IntakeRequest, IntakeSettings, ModelRef, Provenance,
};
use engine_assets::mesh_cook::PROP_MAX_TRIS;
use std::path::{Path, PathBuf};

/// The wrapped model under `content/props/polyhaven/marble_bust_01/`.
fn glb_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../content/props/polyhaven/marble_bust_01/marble_bust_01.glb")
}

/// What produced this model: the Poly Haven CC0 listing, the wrapper and the
/// source glTF digest the fetch pinned (`md5 599d6dd7…`, 2,793 B).
fn provenance() -> Provenance {
    Provenance {
        model: ModelRef {
            id: "polyhaven/marble_bust_01".to_owned(),
            license: "CC0-1.0".to_owned(),
        },
        components: vec![
            ModelRef {
                id: "polyhaven/marble_bust_01/diff_1k".to_owned(),
                license: "CC0-1.0".to_owned(),
            },
            ModelRef {
                id: "polyhaven/marble_bust_01/nor_gl_1k".to_owned(),
                license: "CC0-1.0".to_owned(),
            },
            ModelRef {
                id: "polyhaven/marble_bust_01/rough_1k".to_owned(),
                license: "CC0-1.0".to_owned(),
            },
        ],
        source_digest: "599d6dd71c2dcdfa064b1c1d85882f4".to_owned(),
        seed: 2040,
        conditions: Vec::new(),
        code_ref: "tools/polyhaven-glb.mjs".to_owned(),
    }
}

fn cook(settings: IntakeSettings) -> IntakeOutput {
    let glb = std::fs::read(glb_path()).expect("wrapped bust glb");
    let prov = provenance();
    let request = IntakeRequest {
        name: "marble_bust_01",
        glb: &glb,
        provenance: &prov,
        accepted_conditions: &[],
        settings,
    };
    intake(&request, &mut IntakeCache::memory()).expect("bust intake")
}

/// F2P: the model is 17,456 source triangles; the intake must bring LOD0 to at
/// most 5,000 and still cook the same bytes twice.
#[test]
fn marble_bust_cooks_under_5000_tris_and_twice_equal() {
    let first = cook(IntakeSettings::default());
    let second = cook(IntakeSettings::default());
    assert!(
        first.tris > 0 && first.tris <= PROP_MAX_TRIS,
        "LOD0 {} tris over the {PROP_MAX_TRIS} budget",
        first.tris
    );
    assert!(
        first.lod_tris.windows(2).all(|w| w[0] >= w[1]),
        "LOD chain not descending: {:?}",
        first.lod_tris
    );
    assert_eq!(first.bundle, second.bundle, "cook twice differs");
    assert_eq!(first.row, second.row, "provenance row differs twice");
    // The row names the licence and the source digest it was made from.
    for needle in [
        "\"id\":\"polyhaven/marble_bust_01\"",
        "\"license\":\"CC0-1.0\"",
        "\"source_digest\":\"599d6dd71c2dcdfa064b1c1d85882f4\"",
        "\"seed\":2040",
    ] {
        assert!(
            first.row.contains(needle),
            "{needle} missing in {}",
            first.row
        );
    }
    // The intake's own bundle artifact (E204BNDL) is non-empty and its hash is
    // the blake3 of those bytes, so the bytes a later load reads are the ones
    // the cook reported.
    assert!(!first.bundle.is_empty(), "bundle holds entries");
    assert_eq!(
        first.bundle_hash,
        engine_scene::ids::content_digest(&first.bundle),
        "bundle hash is the digest of the bundle bytes"
    );
}
