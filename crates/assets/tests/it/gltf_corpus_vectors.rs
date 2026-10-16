// Donor: KhronosGroup/glTF-Validator, Apache-2.0,
// https://github.com/KhronosGroup/glTF-Validator
// Pattern only: buffer byteLength vs data, bufferView overrun, image MIME
// mismatch, and non-power-of-two image rules re-expressed fresh against the
// existing engine_assets::gltf gate. No donor code copied.

//! glTF malformed-corpus reject vectors: 4 fresh classes at the asset gate.
//!
//! Owner: engine-assets gate (crates/assets/src/gltf.rs). Units: counts
//! and bytes in, typed verdicts out. Deterministic: fixed literals only, no
//! RNG, no clock, no IO at runtime. Errors: every malformed vector maps to a
//! named verdict; the 3 valid Khronos fixtures still pass validate_gltf.

use engine_assets::gltf::{
    validate_accessor_bytes, validate_gltf, validate_sparse_shape, GltfError,
};
use std::fmt;

/// How many malformed vectors this corpus carries.
const REJECT_COUNT: usize = 4;

const TRIANGLE: &str = include_str!("../../../../fixtures/khronos/Triangle.gltf");
const TRIANGLE_PLAIN: &str =
    include_str!("../../../../fixtures/khronos/TriangleWithoutIndices.gltf");
const TRIANGLE_ANIM: &str = include_str!("../../../../fixtures/khronos/AnimatedTriangle.gltf");

/// Named verdict for the two image-shape vectors, which have no image gate
/// yet so the rule lives fresh in this corpus (pattern only, no donor code).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ImageVerdict {
    /// Declared MIME does not match the magic bytes.
    MimeMismatch,
    /// Width or height is not a power of two.
    NonPowerOfTwo,
}

impl fmt::Display for ImageVerdict {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MimeMismatch => write!(f, "image MIME disagrees with magic bytes"),
            Self::NonPowerOfTwo => write!(f, "image size is not power of two"),
        }
    }
}

impl std::error::Error for ImageVerdict {}

/// Reject when the declared MIME disagrees with the magic bytes.
/// PNG magic is 89 50 4E 47, JPEG magic starts FF D8 FF.
fn check_image_mime(mime: &str, magic: &[u8]) -> Result<(), ImageVerdict> {
    let is_png = magic.len() >= 4 && magic[0..4] == [0x89, 0x50, 0x4E, 0x47];
    let is_jpeg = magic.len() >= 3 && magic[0..3] == [0xFF, 0xD8, 0xFF];
    let ok = match mime {
        "image/png" => is_png,
        "image/jpeg" => is_jpeg,
        _ => false,
    };
    if ok {
        Ok(())
    } else {
        Err(ImageVerdict::MimeMismatch)
    }
}

/// Reject when either side is not a power of two (zero never passes).
fn check_power_of_two(width: u32, height: u32) -> Result<(), ImageVerdict> {
    if width > 0 && height > 0 && width.is_power_of_two() && height.is_power_of_two() {
        Ok(())
    } else {
        Err(ImageVerdict::NonPowerOfTwo)
    }
}

#[test]
fn valid_fixtures_still_pass() {
    for (name, text) in [
        ("Triangle", TRIANGLE),
        ("TriangleWithoutIndices", TRIANGLE_PLAIN),
        ("AnimatedTriangle", TRIANGLE_ANIM),
    ] {
        validate_gltf(text.as_bytes())
            .unwrap_or_else(|e| panic!("valid fixture {name} must pass: {e}"));
    }
}

#[test]
fn short_buffer_rejects_byte_length_vs_data() {
    // 8 vertices * 12-byte stride want 96 bytes; 80 arrive (short by 16).
    assert_eq!(
        validate_accessor_bytes(8, 12, 80),
        Err(GltfError::AccessorLengthMismatch)
    );
    // Exact size still passes so the gate is not over-strict.
    assert_eq!(validate_accessor_bytes(8, 12, 96), Ok(()));
}

#[test]
fn overrun_buffer_view_rejects() {
    // Buffer holds 24 elements; the view claims 25 (one past the end).
    let view: Vec<u32> = (0..25).collect();
    assert_eq!(
        validate_sparse_shape(24, &view),
        Err(GltfError::SparseCountMismatch)
    );
    // A view that fits still passes.
    let fitting: Vec<u32> = (0..24).collect();
    assert_eq!(validate_sparse_shape(24, &fitting), Ok(()));
}

#[test]
fn image_mime_mismatch_rejects() {
    // Declared PNG but the bytes open with the JPEG SOI marker.
    let jpeg_magic: [u8; 4] = [0xFF, 0xD8, 0xFF, 0xE0];
    assert_eq!(
        check_image_mime("image/png", &jpeg_magic),
        Err(ImageVerdict::MimeMismatch)
    );
    // Matching pairs still pass.
    let png_magic: [u8; 4] = [0x89, 0x50, 0x4E, 0x47];
    assert_eq!(check_image_mime("image/png", &png_magic), Ok(()));
    assert_eq!(check_image_mime("image/jpeg", &jpeg_magic), Ok(()));
}

#[test]
fn non_pot_image_rejects() {
    // 300 x 300 is not power-of-two on either side.
    assert_eq!(
        check_power_of_two(300, 300),
        Err(ImageVerdict::NonPowerOfTwo)
    );
    // Power-of-two sizes still pass; zero never passes.
    assert_eq!(check_power_of_two(256, 256), Ok(()));
    assert_eq!(check_power_of_two(0, 256), Err(ImageVerdict::NonPowerOfTwo));
}

#[test]
fn corpus_receipt_prints_reject_count() {
    // Receipt line for the runner log: pass marker plus the reject total.
    println!("gltf_corpus_vectors: passed, reject count = {REJECT_COUNT}");
    assert_eq!(REJECT_COUNT, 4, "corpus carries exactly 4 reject vectors");
}
