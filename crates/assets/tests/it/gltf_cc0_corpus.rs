// Idea: KhronosGroup/glTF-Sample-Assets CC0-1.0 subset,
// https://github.com/KhronosGroup/glTF-Sample-Assets
// License: CC0-1.0 (public domain dedication). Only the CC0 subset idea is
// ported: tiny triangle quad box sample vectors re-expressed fresh below as
// inline minimal glTF envelopes. No vendored bytes, no binary blobs, and no
// non-CC0 models are copied into this file.
//! glTF CC0-only sample corpus vectors beside inline verdicts.
//!
//! Owner: engine-assets gate (crates/assets/src/gltf.rs). Units: bytes in,
//! verdict strings plus cooked bytes out. Deterministic: fixed inline docs,
//! engine-core FNV only, no RNG, no clock, no IO. Errors: every malformed
//! vector maps to one GltfError debug name; valid CC0 style docs say ok.
use engine_assets::gltf::{validate_gltf, GLB_HEADER_LEN, GLB_JSON_TYPE, GLB_MAGIC, GLTF_VERSION};
use engine_core::{hash_bytes, Fnv64};
/// Fresh minimal triangle envelope in the spirit of the CC0 sample set.
const TRIANGLE_CC0: &[u8] =
    br#"{"asset":{"version":"2.0"},"meshes":[{"primitives":[{"attributes":{"POSITION":0}}]}]}"#;
/// Fresh minimal quad envelope in the spirit of the CC0 sample set.
const QUAD_CC0: &[u8] = br#"{"asset":{"version":"2.0"},"meshes":[{"primitives":[{"attributes":{"POSITION":0},"indices":1}]}]}"#;
/// Fresh minimal box envelope in the spirit of the CC0 sample set.
const BOX_CC0: &[u8] = br#"{"asset":{"version":"2.0"},"scenes":[{"nodes":[0]}],"meshes":[{"primitives":[{"attributes":{"POSITION":0,"NORMAL":1}}]}]}"#;
/// Total rows in this corpus.
const COUNT: usize = 5;
/// Row names in order.
const NAMES: [&str; 5] = [
    "triangle-cc0",
    "quad-cc0",
    "box-cc0",
    "missing-asset",
    "wrong-version",
];
/// Expected verdict per row: ok or the GltfError debug name.
const EXPECTED: [&str; 5] = ["ok", "ok", "ok", "MissingAsset", "WrongVersion"];
const _: () = assert!(NAMES.len() == COUNT && EXPECTED.len() == COUNT);
/// Inline doc bytes by row index as owned bytes to keep signatures simple.
fn doc_by_index(index: usize) -> Vec<u8> {
    match index {
        0 => TRIANGLE_CC0.to_vec(),
        1 => QUAD_CC0.to_vec(),
        2 => BOX_CC0.to_vec(),
        3 => br#"{"scene":0}"#.to_vec(),
        4 => br#"{"asset":{"version":"1.0"}}"#.to_vec(),
        _ => Vec::new(),
    }
}
/// Verdict string for one doc: ok or the GltfError debug name.
fn verdict(doc: &[u8]) -> String {
    match validate_gltf(doc) {
        Ok(()) => "ok".to_owned(),
        Err(error) => format!("{error:?}"),
    }
}
/// Deterministic cook of one doc: FNV digest prefix plus the doc bytes.
/// Same input always yields identical bytes on every PC.
fn cook(doc: &[u8]) -> Vec<u8> {
    let digest = hash_bytes(doc);
    let mut out = Vec::with_capacity(8 + doc.len());
    out.extend_from_slice(&digest.to_le_bytes());
    out.extend_from_slice(doc);
    out
}
/// Build a minimal glb with one JSON chunk around json.
fn make_glb(json: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&GLB_MAGIC);
    out.extend_from_slice(&GLTF_VERSION.to_le_bytes());
    let total = (GLB_HEADER_LEN + 8 + json.len()) as u32;
    out.extend_from_slice(&total.to_le_bytes());
    out.extend_from_slice(&(json.len() as u32).to_le_bytes());
    out.extend_from_slice(&GLB_JSON_TYPE.to_le_bytes());
    out.extend_from_slice(json);
    out
}
/// Corpus digest over names, verdicts and cooked bytes plus one framing probe.
fn corpus_digest() -> u64 {
    let mut hasher = Fnv64::new();
    for (index, name) in NAMES.iter().enumerate() {
        hasher.write_str(name);
        let doc = doc_by_index(index);
        hasher.write_str(&verdict(&doc));
        hasher.write(&cook(&doc));
    }
    hasher.write(&make_glb(br#"{"asset":{"version":"2.0"}}"#));
    hasher.finish()
}
/// Golden digest of corpus_digest; bump only with a receipt.
const GOLDEN: u64 = 4806097676379560270;
#[test]
fn cc0_valid_fixtures_pass_gate() {
    for (index, expected) in EXPECTED.iter().enumerate() {
        if *expected == "ok" {
            let doc = doc_by_index(index);
            validate_gltf(&doc).unwrap_or_else(|e| panic!("CC0 fixture must pass: {e}"));
        }
    }
}
#[test]
fn cc0_verdict_strings_match_expected() {
    for (index, expected) in EXPECTED.iter().enumerate() {
        let doc = doc_by_index(index);
        let actual = verdict(&doc);
        assert_eq!(actual.as_str(), *expected);
    }
}
#[test]
fn cc0_cook_bytes_identical_x2() {
    for (index, expected) in EXPECTED.iter().enumerate() {
        if *expected == "ok" {
            let doc = doc_by_index(index);
            let first = cook(&doc);
            let second = cook(&doc);
            assert_eq!(first, second);
            assert_eq!(&first[0..8], &hash_bytes(&doc).to_le_bytes());
            assert_eq!(&first[8..], doc.as_slice());
        }
    }
    assert_ne!(cook(TRIANGLE_CC0), cook(BOX_CC0));
}
#[test]
fn cc0_golden_hash_held() {
    let digest = corpus_digest();
    println!("gltf_cc0_corpus digest = {digest:#018x}");
    assert_eq!(digest, GOLDEN);
}
