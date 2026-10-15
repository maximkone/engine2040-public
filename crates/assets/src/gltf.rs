//! gltf: glTF container sniffing only (bytes in, verdict out).
//!
//! Validates `.glb` framing (magic `glTF`, version 2, total-length and
//! chunk-table sanity, first-chunk JSON extraction) and `.gltf` text
//! envelopes (UTF-8, an `"asset"` key carrying a `"version":"2.x"` string).
//! No full JSON parse, no mesh extraction, no IO and no new dependencies:
//! full parsing waits on the N1 `gltf` crate.
//!
//! Every malformed input returns [`GltfError`]; nothing here panics.

use std::fmt;

/// glTF container version this module accepts.
pub const GLTF_VERSION: u32 = 2;
/// `.glb` magic bytes (`glTF`).
pub const GLB_MAGIC: [u8; 4] = [0x67, 0x6C, 0x54, 0x46];
/// `.glb` header length in bytes (magic + version + total length).
pub const GLB_HEADER_LEN: usize = 12;
/// `.glb` JSON chunk type (`JSON` as little-endian bytes).
pub const GLB_JSON_TYPE: u32 = 0x4E4F_534A;

/// Why container validation failed; every malformed input maps to one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GltfError {
    /// Fewer bytes than the header, chunk table, or declared length needs.
    Truncated,
    /// `.glb` magic is not `glTF`.
    BadMagic,
    /// `.glb` version field is not 2, or `.gltf` version string is not `2.x`.
    WrongVersion,
    /// Declared total length is below the header size or disagrees with input.
    BadLength,
    /// A chunk header overruns the input, or the first chunk is not JSON.
    BadChunk,
    /// `.gltf` bytes are not UTF-8.
    NotUtf8,
    /// `.gltf` text has no `"asset"` key.
    MissingAsset,
    /// Accessor byte length disagrees with `count * stride`.
    AccessorLengthMismatch,
    /// Sparse indices claim more entries than the base accessor holds.
    SparseCountMismatch,
    /// A sparse substitution index is past the base accessor count.
    SparseIndexOutOfBounds,
    /// A `JOINTS_0` index is past the skin joint count.
    JointIndexOutOfBounds,
    /// Joint row count disagrees with the vertex/accessor count.
    JointCountMismatch,
    /// Weight row count disagrees with the joint row count.
    WeightCountMismatch,
}

impl fmt::Display for GltfError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated => write!(f, "glTF container truncated"),
            Self::BadMagic => write!(f, "glTF container has bad magic"),
            Self::WrongVersion => write!(f, "glTF container is not version 2"),
            Self::BadLength => write!(f, "glTF container has bad total length"),
            Self::BadChunk => write!(f, "glTF container has a bad chunk table"),
            Self::NotUtf8 => write!(f, "glTF text is not UTF-8"),
            Self::MissingAsset => write!(f, "glTF text has no asset envelope"),
            Self::AccessorLengthMismatch => {
                write!(f, "glTF accessor length disagrees with count")
            }
            Self::SparseCountMismatch => {
                write!(f, "glTF sparse count overruns the base accessor")
            }
            Self::SparseIndexOutOfBounds => {
                write!(f, "glTF sparse index out of bounds")
            }
            Self::JointIndexOutOfBounds => {
                write!(f, "glTF joint index out of bounds")
            }
            Self::JointCountMismatch => {
                write!(f, "glTF joint count disagrees with vertex count")
            }
            Self::WeightCountMismatch => {
                write!(f, "glTF weight count disagrees with joint count")
            }
        }
    }
}

impl std::error::Error for GltfError {}

/// Validated `.glb` framing facts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GlbInfo {
    /// Total length declared by the header (equals the input length).
    pub declared_len: u32,
    /// Length of the first (JSON) chunk payload.
    pub json_len: u32,
    /// Number of chunks in the chunk table.
    pub chunks: u32,
}

/// Validate `.glb` framing; checks the header, the declared length against
/// the input, and every chunk header, and requires the first chunk to be a
/// nonempty JSON chunk. Deterministic on the same bytes.
pub fn validate_glb(bytes: &[u8]) -> Result<GlbInfo, GltfError> {
    if bytes.len() < GLB_HEADER_LEN {
        return Err(GltfError::Truncated);
    }
    if bytes[0..4] != GLB_MAGIC {
        return Err(GltfError::BadMagic);
    }
    let version = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
    if version != GLTF_VERSION {
        return Err(GltfError::WrongVersion);
    }
    let declared = u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]);
    if declared < GLB_HEADER_LEN as u32 {
        return Err(GltfError::BadLength);
    }
    let declared_len = declared as usize;
    if declared_len > bytes.len() {
        return Err(GltfError::Truncated);
    }
    if declared_len < bytes.len() {
        return Err(GltfError::BadLength);
    }
    let mut offset = GLB_HEADER_LEN;
    let mut chunks: u32 = 0;
    let mut json_len: u32 = 0;
    while offset < declared_len {
        if offset + 8 > declared_len {
            return Err(GltfError::Truncated);
        }
        let chunk_len = u32::from_le_bytes([
            bytes[offset],
            bytes[offset + 1],
            bytes[offset + 2],
            bytes[offset + 3],
        ]) as usize;
        let chunk_type = u32::from_le_bytes([
            bytes[offset + 4],
            bytes[offset + 5],
            bytes[offset + 6],
            bytes[offset + 7],
        ]);
        let data_start = offset + 8;
        let data_end = data_start
            .checked_add(chunk_len)
            .ok_or(GltfError::BadChunk)?;
        if data_end > declared_len {
            return Err(GltfError::BadChunk);
        }
        if chunks == 0 {
            if chunk_type != GLB_JSON_TYPE || chunk_len == 0 {
                return Err(GltfError::BadChunk);
            }
            json_len = chunk_len as u32;
        }
        offset = data_end;
        chunks = chunks.checked_add(1).ok_or(GltfError::BadChunk)?;
    }
    if chunks == 0 {
        return Err(GltfError::BadChunk);
    }
    Ok(GlbInfo {
        declared_len: declared,
        json_len,
        chunks,
    })
}

/// Extract the JSON chunk payload after validating the container; the
/// returned slice borrows the input.
pub fn glb_json(bytes: &[u8]) -> Result<&[u8], GltfError> {
    let info = validate_glb(bytes)?;
    let start = GLB_HEADER_LEN + 8;
    let end = start + info.json_len as usize;
    bytes.get(start..end).ok_or(GltfError::BadChunk)
}

/// Validate a `.gltf` text envelope without a full JSON parse: the bytes must
/// be UTF-8, contain an `"asset"` key, and carry a `"version":"2.x"` string
/// after it (whitespace around `:` and the value is allowed).
/// Deterministic on the same bytes.
pub fn validate_gltf(bytes: &[u8]) -> Result<(), GltfError> {
    let text = core::str::from_utf8(bytes).map_err(|_| GltfError::NotUtf8)?;
    let asset_at = text.find("\"asset\"").ok_or(GltfError::MissingAsset)?;
    let after_asset = &text[asset_at + "\"asset\"".len()..];
    let version_at = after_asset
        .find("\"version\"")
        .ok_or(GltfError::WrongVersion)?;
    let mut rest = &after_asset[version_at + "\"version\"".len()..];
    rest = trim_json_space(rest);
    rest = rest.strip_prefix(':').ok_or(GltfError::WrongVersion)?;
    rest = trim_json_space(rest);
    rest = rest.strip_prefix('"').ok_or(GltfError::WrongVersion)?;
    if rest.starts_with("2.") {
        Ok(())
    } else {
        Err(GltfError::WrongVersion)
    }
}

/// Skip JSON whitespace between a key, the colon, and the value.
fn trim_json_space(text: &str) -> &str {
    text.trim_start_matches([' ', '\t', '\n', '\r'])
}

// Attribution: accessor-shape rules adapted from KhronosGroup/glTF-Validator
// (Apache-2.0), lib/src/data_access/validate_accessors.dart:41,
// https://github.com/KhronosGroup/glTF-Validator - ported fresh to
// std-only Rust; no donor code copied. Counts and index bounds only, no IO.

/// Validate a decoded accessor byte length against `count * stride`.
///
/// `stride` is bytes per element (components times component size).
/// Overflow or any length disagreement maps to
/// [`GltfError::AccessorLengthMismatch`]. Deterministic on the inputs.
pub fn validate_accessor_bytes(count: usize, stride: usize, bytes: usize) -> Result<(), GltfError> {
    let want = count
        .checked_mul(stride)
        .ok_or(GltfError::AccessorLengthMismatch)?;
    if want == bytes {
        Ok(())
    } else {
        Err(GltfError::AccessorLengthMismatch)
    }
}

/// Validate sparse substitution indices against the base accessor count.
///
/// More indices than base elements maps to
/// [`GltfError::SparseCountMismatch`]; any index at or past `base_count`
/// maps to [`GltfError::SparseIndexOutOfBounds`]. Deterministic on the inputs.
pub fn validate_sparse_shape(base_count: usize, indices: &[u32]) -> Result<(), GltfError> {
    if indices.len() > base_count {
        return Err(GltfError::SparseCountMismatch);
    }
    for index in indices {
        if (*index as usize) >= base_count {
            return Err(GltfError::SparseIndexOutOfBounds);
        }
    }
    Ok(())
}

/// Validate `JOINTS_0` rows: one index per vertex row, each below `joint_count`.
///
/// A row count other than `vertex_count` maps to
/// [`GltfError::JointCountMismatch`]; any index at or past `joint_count`
/// maps to [`GltfError::JointIndexOutOfBounds`]. Deterministic on the inputs.
pub fn validate_joint_shape(
    joints: &[u16],
    joint_count: usize,
    vertex_count: usize,
) -> Result<(), GltfError> {
    if joints.len() != vertex_count {
        return Err(GltfError::JointCountMismatch);
    }
    for joint in joints {
        if usize::from(*joint) >= joint_count {
            return Err(GltfError::JointIndexOutOfBounds);
        }
    }
    Ok(())
}

/// Validate skin weight row count against the joint row count.
///
/// Lengths are separate because joints (`u16`) and weights (`f32`) live in
/// different slices. Any disagreement maps to
/// [`GltfError::WeightCountMismatch`]. Deterministic on the inputs.
pub fn validate_weights_shape(weights_len: usize, joints_len: usize) -> Result<(), GltfError> {
    if weights_len == joints_len {
        Ok(())
    } else {
        Err(GltfError::WeightCountMismatch)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TRIANGLE: &str = include_str!("../../../fixtures/khronos/Triangle.gltf");
    const TRIANGLE_PLAIN: &str =
        include_str!("../../../fixtures/khronos/TriangleWithoutIndices.gltf");
    const TRIANGLE_ANIM: &str = include_str!("../../../fixtures/khronos/AnimatedTriangle.gltf");

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

    fn envelope() -> Vec<u8> {
        br#"{"asset":{"version":"2.0"}}"#.to_vec()
    }

    #[test]
    fn real_fixtures_validate() {
        for (name, text) in [
            ("Triangle", TRIANGLE),
            ("TriangleWithoutIndices", TRIANGLE_PLAIN),
            ("AnimatedTriangle", TRIANGLE_ANIM),
        ] {
            validate_gltf(text.as_bytes())
                .unwrap_or_else(|e| panic!("fixture {name} must validate: {e}"));
        }
    }

    #[test]
    fn glb_roundtrip_reports_framing() {
        let json = envelope();
        let bytes = make_glb(&json, GLTF_VERSION);
        let info = validate_glb(&bytes).expect("built glb must validate");
        assert_eq!(info.declared_len as usize, bytes.len());
        assert_eq!(info.json_len as usize, json.len());
        assert_eq!(info.chunks, 1);
        assert_eq!(
            glb_json(&bytes).expect("json must extract"),
            json.as_slice()
        );
    }

    #[test]
    fn glb_bad_magic_rejected() {
        let mut bytes = make_glb(&envelope(), GLTF_VERSION);
        bytes[0..4].copy_from_slice(b"GLB ");
        assert_eq!(validate_glb(&bytes), Err(GltfError::BadMagic));
        assert_eq!(glb_json(&bytes), Err(GltfError::BadMagic));
    }

    #[test]
    fn glb_truncated_rejected() {
        assert_eq!(validate_glb(b""), Err(GltfError::Truncated));
        assert_eq!(validate_glb(&[0u8; 5]), Err(GltfError::Truncated));
        // Declared length runs past the input.
        let mut bytes = make_glb(&envelope(), GLTF_VERSION);
        let too_long = (bytes.len() + 16) as u32;
        bytes[8..12].copy_from_slice(&too_long.to_le_bytes());
        assert_eq!(validate_glb(&bytes), Err(GltfError::Truncated));
        // Chunk payload cut short while the header still claims it.
        let mut cut = make_glb(&envelope(), GLTF_VERSION);
        cut.truncate(cut.len() - 4);
        assert_eq!(validate_glb(&cut), Err(GltfError::Truncated));
    }

    #[test]
    fn glb_wrong_version_rejected() {
        for version in [0, 1, 3, u32::MAX] {
            let bytes = make_glb(&envelope(), version);
            assert_eq!(
                validate_glb(&bytes),
                Err(GltfError::WrongVersion),
                "version {version} must fail"
            );
        }
    }

    #[test]
    fn glb_bad_length_rejected() {
        // Declared length below the header size.
        let mut small = make_glb(&envelope(), GLTF_VERSION);
        small[8..12].copy_from_slice(&8u32.to_le_bytes());
        assert_eq!(validate_glb(&small), Err(GltfError::BadLength));
        // Declared length shorter than the input (header lies about the size).
        let mut trailing = make_glb(&envelope(), GLTF_VERSION);
        trailing.extend_from_slice(&[0u8; 4]);
        assert_eq!(validate_glb(&trailing), Err(GltfError::BadLength));
    }

    #[test]
    fn glb_bad_chunks_rejected() {
        // Header only, no JSON chunk.
        let mut bare = vec![0u8; GLB_HEADER_LEN];
        bare[0..4].copy_from_slice(&GLB_MAGIC);
        bare[4..8].copy_from_slice(&GLTF_VERSION.to_le_bytes());
        bare[8..12].copy_from_slice(&(GLB_HEADER_LEN as u32).to_le_bytes());
        assert_eq!(validate_glb(&bare), Err(GltfError::BadChunk));
        // First chunk is BIN, not JSON.
        let mut bin = make_glb(&envelope(), GLTF_VERSION);
        bin[12..16].copy_from_slice(&(envelope().len() as u32).to_le_bytes());
        bin[16..20].copy_from_slice(&0x004E_4942u32.to_le_bytes());
        assert_eq!(validate_glb(&bin), Err(GltfError::BadChunk));
        // Empty JSON chunk.
        let mut empty = vec![0u8; GLB_HEADER_LEN + 8];
        empty[0..4].copy_from_slice(&GLB_MAGIC);
        empty[4..8].copy_from_slice(&GLTF_VERSION.to_le_bytes());
        let empty_len = empty.len() as u32;
        empty[8..12].copy_from_slice(&empty_len.to_le_bytes());
        empty[16..20].copy_from_slice(&GLB_JSON_TYPE.to_le_bytes());
        assert_eq!(validate_glb(&empty), Err(GltfError::BadChunk));
        // Chunk length overruns the declared size.
        let mut overrun = make_glb(&envelope(), GLTF_VERSION);
        overrun[12..16].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(validate_glb(&overrun), Err(GltfError::BadChunk));
    }

    #[test]
    fn gltf_malformed_rejected() {
        assert_eq!(validate_gltf(b"\xFF\xFE{\x00"), Err(GltfError::NotUtf8));
        assert_eq!(
            validate_gltf(br#"{"scene":0}"#.as_slice()),
            Err(GltfError::MissingAsset)
        );
        assert_eq!(validate_gltf(b""), Err(GltfError::MissingAsset));
        assert_eq!(
            validate_gltf(br#"{"asset":{"version":"1.0"}}"#.as_slice()),
            Err(GltfError::WrongVersion)
        );
        assert_eq!(
            validate_gltf(br#"{"asset":{}}"#.as_slice()),
            Err(GltfError::WrongVersion)
        );
        assert_eq!(
            validate_gltf(br#"{"asset":{"version":"20"}}"#.as_slice()),
            Err(GltfError::WrongVersion)
        );
    }

    #[test]
    fn gltf_version_spacing_accepted() {
        for text in [
            r#"{"asset":{"version":"2.0"}}"#,
            r#"{ "asset" : { "version" : "2.0" } }"#,
            "{\"asset\":\n\t\"version\"\n:\n\t\"2.1\"}",
        ] {
            validate_gltf(text.as_bytes())
                .unwrap_or_else(|e| panic!("spacing variant must validate: {text}: {e}"));
        }
    }

    #[test]
    fn header_mutations_never_panic() {
        let base = make_glb(&envelope(), GLTF_VERSION);
        let mut count_ok = 0;
        for i in 0..GLB_HEADER_LEN {
            for value in 0..=255u8 {
                let mut mutated = base.clone();
                mutated[i] = value;
                // Any outcome is fine; returning at all is the assertion.
                let validated = validate_glb(&mutated);
                let _ = glb_json(&mutated);
                if value == base[i] {
                    assert!(
                        validated.is_ok(),
                        "unmutated header byte {i} must still validate"
                    );
                    count_ok += 1;
                }
            }
        }
        assert_eq!(count_ok, GLB_HEADER_LEN);
    }

    #[test]
    fn accessor_shape_corpus_rejects_six_malformed() {
        // 1. Decoded bytes disagree with count * stride.
        assert_eq!(
            validate_accessor_bytes(4, 12, 47),
            Err(GltfError::AccessorLengthMismatch)
        );
        // 2. More sparse indices than base elements.
        assert_eq!(
            validate_sparse_shape(3, &[0, 1, 2, 3]),
            Err(GltfError::SparseCountMismatch)
        );
        // 3. Sparse index past the base count.
        assert_eq!(
            validate_sparse_shape(3, &[0, 5]),
            Err(GltfError::SparseIndexOutOfBounds)
        );
        // 4. Joint index past the skin joint count.
        assert_eq!(
            validate_joint_shape(&[0, 9], 8, 2),
            Err(GltfError::JointIndexOutOfBounds)
        );
        // 5. Joint rows disagree with the vertex count.
        assert_eq!(
            validate_joint_shape(&[0, 1], 8, 3),
            Err(GltfError::JointCountMismatch)
        );
        // 6. Weight rows disagree with the joint rows.
        assert_eq!(
            validate_weights_shape(2, 3),
            Err(GltfError::WeightCountMismatch)
        );
        // Overflow in count * stride is the same length verdict, never a panic.
        assert_eq!(
            validate_accessor_bytes(usize::MAX, 2, 0),
            Err(GltfError::AccessorLengthMismatch)
        );
    }

    #[test]
    fn accessor_shape_happy_paths_pass() {
        assert_eq!(validate_accessor_bytes(4, 12, 48), Ok(()));
        assert_eq!(validate_accessor_bytes(0, 12, 0), Ok(()));
        assert_eq!(validate_sparse_shape(3, &[0, 2]), Ok(()));
        assert_eq!(validate_sparse_shape(0, &[]), Ok(()));
        assert_eq!(validate_joint_shape(&[0, 7], 8, 2), Ok(()));
        assert_eq!(validate_weights_shape(3, 3), Ok(()));
    }

    #[test]
    fn errors_display_without_panic() {
        for error in [
            GltfError::Truncated,
            GltfError::BadMagic,
            GltfError::WrongVersion,
            GltfError::BadLength,
            GltfError::BadChunk,
            GltfError::NotUtf8,
            GltfError::MissingAsset,
            GltfError::AccessorLengthMismatch,
            GltfError::SparseCountMismatch,
            GltfError::SparseIndexOutOfBounds,
            GltfError::JointIndexOutOfBounds,
            GltfError::JointCountMismatch,
            GltfError::WeightCountMismatch,
        ] {
            let text = format!("{error}");
            assert!(!text.is_empty());
            let boxed: Box<dyn std::error::Error> = Box::new(error);
            assert!(!format!("{boxed}").is_empty());
        }
    }
}
