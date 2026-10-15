// Donor: KhronosGroup/glTF-Validator test manifest shape (Apache-2.0,
// licence verified), https://github.com/KhronosGroup/glTF-Validator.
// Pattern only, no donor code copied: `assets.json`
// group -> dir -> file -> expected plus a `.report.json` compare that
// ignores the version stamp. Fresh inline minimal glTF docs below
// (valid + malformed pairs); the harness re-runs each vector and diffs
// the verdict string. Minor `"2.0"` vs `"2.1"` stamps both expect `ok`.

//! Manifest-driven glTF conformance vectors: Khronos shape, local verdicts.
//!
//! Owner: `engine-assets`; consumer: the glTF cook gate keeps malformed
//! containers out before mesh import. Units: bytes/shapes in, verdict
//! strings out. Deterministic: fixed inline docs, no RNG, no wall clock.
//! Errors: every malformed input maps to one `GltfError` debug name.

use engine_assets::gltf::{
    validate_accessor_bytes, validate_glb, validate_gltf, validate_joint_shape,
    validate_sparse_shape, validate_weights_shape, GLB_HEADER_LEN, GLB_JSON_TYPE, GLB_MAGIC,
    GLTF_VERSION,
};

/// One manifest row: where the vector lives and the verdict it must give.
/// Mirrors the donor `assets.json` nesting group -> dir -> file -> expected.
struct ManifestEntry {
    group: &'static str,
    dir: &'static str,
    file: &'static str,
    expected: &'static str,
}

/// Fresh manifest table over inline minimal docs: valid + malformed pairs.
/// `expected` is `"ok"` or the `GltfError` debug name (`"Truncated"`, ...).
const MANIFEST: &[ManifestEntry] = &[
    // Text envelope group: `.gltf` UTF-8 + asset + `2.x` version sniffing.
    ManifestEntry {
        group: "conformance",
        dir: "text",
        file: "valid-minimal.gltf",
        expected: "ok",
    },
    ManifestEntry {
        group: "conformance",
        dir: "text",
        file: "valid-spaced.gltf",
        expected: "ok",
    },
    ManifestEntry {
        group: "conformance",
        dir: "text",
        file: "valid-multiline.gltf",
        expected: "ok",
    },
    ManifestEntry {
        group: "conformance",
        dir: "text",
        file: "not-utf8.gltf",
        expected: "NotUtf8",
    },
    ManifestEntry {
        group: "conformance",
        dir: "text",
        file: "missing-asset-empty.gltf",
        expected: "MissingAsset",
    },
    ManifestEntry {
        group: "conformance",
        dir: "text",
        file: "missing-asset-no-key.gltf",
        expected: "MissingAsset",
    },
    ManifestEntry {
        group: "conformance",
        dir: "text",
        file: "wrong-version-1-0.gltf",
        expected: "WrongVersion",
    },
    ManifestEntry {
        group: "conformance",
        dir: "text",
        file: "wrong-version-missing.gltf",
        expected: "WrongVersion",
    },
    ManifestEntry {
        group: "conformance",
        dir: "text",
        file: "wrong-version-no-dot.gltf",
        expected: "WrongVersion",
    },
    ManifestEntry {
        group: "conformance",
        dir: "text",
        file: "wrong-version-3-0.gltf",
        expected: "WrongVersion",
    },
    // Binary framing group: `.glb` magic, version, length, chunk table.
    ManifestEntry {
        group: "conformance",
        dir: "framing",
        file: "valid-single-chunk.glb",
        expected: "ok",
    },
    ManifestEntry {
        group: "conformance",
        dir: "framing",
        file: "bad-magic.glb",
        expected: "BadMagic",
    },
    ManifestEntry {
        group: "conformance",
        dir: "framing",
        file: "truncated-empty.glb",
        expected: "Truncated",
    },
    ManifestEntry {
        group: "conformance",
        dir: "framing",
        file: "truncated-short-header.glb",
        expected: "Truncated",
    },
    ManifestEntry {
        group: "conformance",
        dir: "framing",
        file: "truncated-declared-past-end.glb",
        expected: "Truncated",
    },
    ManifestEntry {
        group: "conformance",
        dir: "framing",
        file: "wrong-version-1.glb",
        expected: "WrongVersion",
    },
    ManifestEntry {
        group: "conformance",
        dir: "framing",
        file: "bad-length-below-header.glb",
        expected: "BadLength",
    },
    ManifestEntry {
        group: "conformance",
        dir: "framing",
        file: "bad-length-trailing-bytes.glb",
        expected: "BadLength",
    },
    ManifestEntry {
        group: "conformance",
        dir: "framing",
        file: "bad-chunk-no-chunks.glb",
        expected: "BadChunk",
    },
    ManifestEntry {
        group: "conformance",
        dir: "framing",
        file: "bad-chunk-bin-first.glb",
        expected: "BadChunk",
    },
    ManifestEntry {
        group: "conformance",
        dir: "framing",
        file: "bad-chunk-empty-json.glb",
        expected: "BadChunk",
    },
    ManifestEntry {
        group: "conformance",
        dir: "framing",
        file: "bad-chunk-overrun.glb",
        expected: "BadChunk",
    },
    // Accessor-shape group: counts and index bounds only, no IO.
    ManifestEntry {
        group: "conformance",
        dir: "shape",
        file: "accessor-length-mismatch.shape",
        expected: "AccessorLengthMismatch",
    },
    ManifestEntry {
        group: "conformance",
        dir: "shape",
        file: "accessor-ok.shape",
        expected: "ok",
    },
    ManifestEntry {
        group: "conformance",
        dir: "shape",
        file: "accessor-overflow.shape",
        expected: "AccessorLengthMismatch",
    },
    ManifestEntry {
        group: "conformance",
        dir: "shape",
        file: "sparse-count-mismatch.shape",
        expected: "SparseCountMismatch",
    },
    ManifestEntry {
        group: "conformance",
        dir: "shape",
        file: "sparse-index-oob.shape",
        expected: "SparseIndexOutOfBounds",
    },
    ManifestEntry {
        group: "conformance",
        dir: "shape",
        file: "sparse-ok.shape",
        expected: "ok",
    },
    ManifestEntry {
        group: "conformance",
        dir: "shape",
        file: "sparse-empty-ok.shape",
        expected: "ok",
    },
    ManifestEntry {
        group: "conformance",
        dir: "shape",
        file: "joint-index-oob.shape",
        expected: "JointIndexOutOfBounds",
    },
    ManifestEntry {
        group: "conformance",
        dir: "shape",
        file: "joint-count-mismatch.shape",
        expected: "JointCountMismatch",
    },
    ManifestEntry {
        group: "conformance",
        dir: "shape",
        file: "joint-ok.shape",
        expected: "ok",
    },
    ManifestEntry {
        group: "conformance",
        dir: "shape",
        file: "weight-count-mismatch.shape",
        expected: "WeightCountMismatch",
    },
    ManifestEntry {
        group: "conformance",
        dir: "shape",
        file: "weight-ok.shape",
        expected: "ok",
    },
];

/// Minimal valid envelope used as the `.glb` JSON payload.
fn envelope() -> Vec<u8> {
    br#"{"asset":{"version":"2.0"}}"#.to_vec()
}

/// Build a minimal `.glb` with one JSON chunk around `json`.
fn make_glb(json: &[u8], version: u32) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&GLB_MAGIC);
    out.extend_from_slice(&version.to_le_bytes());
    let total = (GLB_HEADER_LEN + 8 + json.len()) as u32;
    out.extend_from_slice(&total.to_le_bytes());
    out.extend_from_slice(&(json.len() as u32).to_le_bytes());
    out.extend_from_slice(&GLB_JSON_TYPE.to_le_bytes());
    out.extend_from_slice(json);
    out
}

/// Inline `.gltf` text bytes for one manifest file name.
fn text_bytes(file: &str) -> Vec<u8> {
    match file {
        "valid-minimal.gltf" => br#"{"asset":{"version":"2.0"}}"#.to_vec(),
        // Version stamp differs (`2.1`); the report compare ignores it.
        "valid-spaced.gltf" => br#"{ "asset" : { "version" : "2.1" } }"#.to_vec(),
        "valid-multiline.gltf" => b"{\"asset\":\n\t\"version\"\n:\n\t\"2.0\"}".to_vec(),
        "not-utf8.gltf" => vec![0xFF, 0xFE, 0x7B],
        "missing-asset-empty.gltf" => vec![],
        "missing-asset-no-key.gltf" => br#"{"scene":0}"#.to_vec(),
        "wrong-version-1-0.gltf" => br#"{"asset":{"version":"1.0"}}"#.to_vec(),
        "wrong-version-missing.gltf" => br#"{"asset":{}}"#.to_vec(),
        "wrong-version-no-dot.gltf" => br#"{"asset":{"version":"20"}}"#.to_vec(),
        "wrong-version-3-0.gltf" => br#"{"asset":{"version":"3.0"}}"#.to_vec(),
        other => panic!("skipped: unknown text vector {other}"),
    }
}

/// Inline `.glb` bytes for one manifest file name.
fn glb_bytes(file: &str) -> Vec<u8> {
    match file {
        "valid-single-chunk.glb" => make_glb(&envelope(), GLTF_VERSION),
        "bad-magic.glb" => {
            let mut bytes = make_glb(&envelope(), GLTF_VERSION);
            bytes[0..4].copy_from_slice(b"GLB ");
            bytes
        }
        "truncated-empty.glb" => vec![],
        "truncated-short-header.glb" => vec![0u8; 5],
        "truncated-declared-past-end.glb" => {
            let mut bytes = make_glb(&envelope(), GLTF_VERSION);
            let too_long = (bytes.len() + 16) as u32;
            bytes[8..12].copy_from_slice(&too_long.to_le_bytes());
            bytes
        }
        "wrong-version-1.glb" => make_glb(&envelope(), 1),
        "bad-length-below-header.glb" => {
            let mut bytes = make_glb(&envelope(), GLTF_VERSION);
            bytes[8..12].copy_from_slice(&8u32.to_le_bytes());
            bytes
        }
        "bad-length-trailing-bytes.glb" => {
            let mut bytes = make_glb(&envelope(), GLTF_VERSION);
            bytes.extend_from_slice(&[0u8; 4]);
            bytes
        }
        "bad-chunk-no-chunks.glb" => {
            let mut bare = vec![0u8; GLB_HEADER_LEN];
            bare[0..4].copy_from_slice(&GLB_MAGIC);
            bare[4..8].copy_from_slice(&GLTF_VERSION.to_le_bytes());
            bare[8..12].copy_from_slice(&(GLB_HEADER_LEN as u32).to_le_bytes());
            bare
        }
        "bad-chunk-bin-first.glb" => {
            let mut bytes = make_glb(&envelope(), GLTF_VERSION);
            bytes[16..20].copy_from_slice(&0x004E_4942u32.to_le_bytes());
            bytes
        }
        "bad-chunk-empty-json.glb" => {
            let mut empty = vec![0u8; GLB_HEADER_LEN + 8];
            empty[0..4].copy_from_slice(&GLB_MAGIC);
            empty[4..8].copy_from_slice(&GLTF_VERSION.to_le_bytes());
            let len = empty.len() as u32;
            empty[8..12].copy_from_slice(&len.to_le_bytes());
            empty[16..20].copy_from_slice(&GLB_JSON_TYPE.to_le_bytes());
            empty
        }
        "bad-chunk-overrun.glb" => {
            let mut bytes = make_glb(&envelope(), GLTF_VERSION);
            bytes[12..16].copy_from_slice(&u32::MAX.to_le_bytes());
            bytes
        }
        other => panic!("skipped: unknown framing vector {other}"),
    }
}

/// Verdict string for one shape vector: `"ok"` or the error debug name.
fn shape_verdict(file: &str) -> String {
    let result = match file {
        "accessor-length-mismatch.shape" => validate_accessor_bytes(4, 12, 47),
        "accessor-ok.shape" => validate_accessor_bytes(4, 12, 48),
        "accessor-overflow.shape" => validate_accessor_bytes(usize::MAX, 2, 0),
        "sparse-count-mismatch.shape" => validate_sparse_shape(3, &[0, 1, 2, 3]),
        "sparse-index-oob.shape" => validate_sparse_shape(3, &[0, 5]),
        "sparse-ok.shape" => validate_sparse_shape(3, &[0, 2]),
        "sparse-empty-ok.shape" => validate_sparse_shape(0, &[]),
        "joint-index-oob.shape" => validate_joint_shape(&[0, 9], 8, 2),
        "joint-count-mismatch.shape" => validate_joint_shape(&[0, 1], 8, 3),
        "joint-ok.shape" => validate_joint_shape(&[0, 7], 8, 2),
        "weight-count-mismatch.shape" => validate_weights_shape(2, 3),
        "weight-ok.shape" => validate_weights_shape(3, 3),
        other => panic!("skipped: unknown shape vector {other}"),
    };
    match result {
        Ok(()) => "ok".to_owned(),
        Err(error) => format!("{error:?}"),
    }
}

/// Re-run one manifest vector and return its actual verdict string.
fn actual_verdict(entry: &ManifestEntry) -> String {
    match entry.dir {
        "text" => {
            let bytes = text_bytes(entry.file);
            match validate_gltf(&bytes) {
                Ok(()) => "ok".to_owned(),
                Err(error) => format!("{error:?}"),
            }
        }
        "framing" => {
            let bytes = glb_bytes(entry.file);
            match validate_glb(&bytes) {
                Ok(_) => "ok".to_owned(),
                Err(error) => format!("{error:?}"),
            }
        }
        "shape" => shape_verdict(entry.file),
        other => panic!("skipped: unknown manifest dir {other}"),
    }
}

/// Look one manifest row up by its group, dir and file names.
fn find_entry(group: &str, dir: &str, file: &str) -> &'static ManifestEntry {
    MANIFEST
        .iter()
        .find(|entry| entry.group == group && entry.dir == dir && entry.file == file)
        .unwrap_or_else(|| panic!("skipped: manifest has no {group}/{dir}/{file}"))
}

/// Re-run one vector and diff its verdict against the manifest expectation.
fn assert_one(group: &str, dir: &str, file: &str) {
    let entry = find_entry(group, dir, file);
    let actual = actual_verdict(entry);
    assert_eq!(
        actual.as_str(),
        entry.expected,
        "vector {}/{}/{} verdict drift: report says {:?}, re-run says {actual:?}",
        entry.group,
        entry.dir,
        entry.file,
        entry.expected,
    );
}

macro_rules! conformance_test {
    ($name:ident, $dir:expr, $file:expr) => {
        #[test]
        fn $name() {
            assert_one("conformance", $dir, $file);
        }
    };
}

conformance_test!(text_valid_minimal, "text", "valid-minimal.gltf");
conformance_test!(text_valid_spaced, "text", "valid-spaced.gltf");
conformance_test!(text_valid_multiline, "text", "valid-multiline.gltf");
conformance_test!(text_not_utf8, "text", "not-utf8.gltf");
conformance_test!(text_missing_asset_empty, "text", "missing-asset-empty.gltf");
conformance_test!(
    text_missing_asset_no_key,
    "text",
    "missing-asset-no-key.gltf"
);
conformance_test!(text_wrong_version_1_0, "text", "wrong-version-1-0.gltf");
conformance_test!(
    text_wrong_version_missing,
    "text",
    "wrong-version-missing.gltf"
);
conformance_test!(
    text_wrong_version_no_dot,
    "text",
    "wrong-version-no-dot.gltf"
);
conformance_test!(text_wrong_version_3_0, "text", "wrong-version-3-0.gltf");
conformance_test!(
    framing_valid_single_chunk,
    "framing",
    "valid-single-chunk.glb"
);
conformance_test!(framing_bad_magic, "framing", "bad-magic.glb");
conformance_test!(framing_truncated_empty, "framing", "truncated-empty.glb");
conformance_test!(
    framing_truncated_short_header,
    "framing",
    "truncated-short-header.glb"
);
conformance_test!(
    framing_truncated_declared_past_end,
    "framing",
    "truncated-declared-past-end.glb"
);
conformance_test!(framing_wrong_version_1, "framing", "wrong-version-1.glb");
conformance_test!(
    framing_bad_length_below_header,
    "framing",
    "bad-length-below-header.glb"
);
conformance_test!(
    framing_bad_length_trailing_bytes,
    "framing",
    "bad-length-trailing-bytes.glb"
);
conformance_test!(
    framing_bad_chunk_no_chunks,
    "framing",
    "bad-chunk-no-chunks.glb"
);
conformance_test!(
    framing_bad_chunk_bin_first,
    "framing",
    "bad-chunk-bin-first.glb"
);
conformance_test!(
    framing_bad_chunk_empty_json,
    "framing",
    "bad-chunk-empty-json.glb"
);
conformance_test!(
    framing_bad_chunk_overrun,
    "framing",
    "bad-chunk-overrun.glb"
);
conformance_test!(
    shape_accessor_length_mismatch,
    "shape",
    "accessor-length-mismatch.shape"
);
conformance_test!(shape_accessor_ok, "shape", "accessor-ok.shape");
conformance_test!(shape_accessor_overflow, "shape", "accessor-overflow.shape");
conformance_test!(
    shape_sparse_count_mismatch,
    "shape",
    "sparse-count-mismatch.shape"
);
conformance_test!(shape_sparse_index_oob, "shape", "sparse-index-oob.shape");
conformance_test!(shape_sparse_ok, "shape", "sparse-ok.shape");
conformance_test!(shape_sparse_empty_ok, "shape", "sparse-empty-ok.shape");
conformance_test!(shape_joint_index_oob, "shape", "joint-index-oob.shape");
conformance_test!(
    shape_joint_count_mismatch,
    "shape",
    "joint-count-mismatch.shape"
);
conformance_test!(shape_joint_ok, "shape", "joint-ok.shape");
conformance_test!(
    shape_weight_count_mismatch,
    "shape",
    "weight-count-mismatch.shape"
);
conformance_test!(shape_weight_ok, "shape", "weight-ok.shape");

/// Harness: re-run the whole manifest and diff every verdict at once.
#[test]
fn manifest_verdicts_match_expected() {
    let total = MANIFEST.len();
    assert!(total >= 20, "manifest must hold 20+ vectors, has {total}");
    let mut passed = 0_usize;
    for entry in MANIFEST {
        let actual = actual_verdict(entry);
        assert_eq!(
            actual.as_str(),
            entry.expected,
            "vector {}/{}/{} verdict drift: report says {:?}, re-run says {actual:?}",
            entry.group,
            entry.dir,
            entry.file,
            entry.expected,
        );
        passed += 1;
    }
    println!("passed {passed}/{total} gltf conformance vectors");
    assert_eq!(passed, total);
}
