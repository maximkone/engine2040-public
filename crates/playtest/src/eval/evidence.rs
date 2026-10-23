//! A6 evidence paths and the keyed seal over the manifest.
//!
//! Consumer: [`write_evidence`](super::write_evidence) and
//! [`verify_evidence`](super::verify_evidence); the operator reads the same
//! paths. Owner: this module owns the file names, the directory and the
//! HMAC-SHA256 seal from the `hmac` crate (never hand-rolled). The operator
//! key arrives through [`OPERATOR_KEY_ENV`], and a missing, malformed or
//! mismatched seal fails typed [`EvalError::DigestMismatch`]; nothing verifies
//! bare. Units: bytes; the manifest is text. Determinism: pure over its
//! inputs.

use std::path::PathBuf;

use engine_core::MIN_SEAL_KEY_BYTES;
use engine_core::{decode_seal_hex, SealKeyError};
use hmac::{Hmac, Mac};
use sha2::Sha256;

use super::EvalError;

/// Evidence report file name under `target/evidence/a6/`.
pub const EVIDENCE_REPORT: &str = "report.txt";
/// Evidence tape file name under `target/evidence/a6/`: one line per row.
pub const EVIDENCE_TAPE: &str = "tape.log";
/// Digest manifest beside the A6 evidence files.
pub const EVIDENCE_MANIFEST: &str = "digests.txt";
/// Seal over the manifest bytes; the manifest is never trusted bare.
pub const EVIDENCE_SEAL: &str = "digests.sha";

/// Directory holding the A6 evidence, resolved from the crate location so
/// the proof runs from any working directory.
pub fn evidence_dir() -> PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/evidence/a6")
}

/// Keyed-seal scheme for `digests.sha`: HMAC-SHA256 over the manifest bytes
/// under the operator key. Seals without this prefix are rejected outright.
pub const SEAL_SCHEME: &str = "hmac-sha256";
/// Environment holding the operator seal key as hex (64+ chars for 32+
/// bytes). The key lives with the operator, never in the repo.
pub const OPERATOR_KEY_ENV: &str = "E2040_EVIDENCE_KEY";

/// A refused operator key with the remediation attached.
fn seal_key_error(detail: &str) -> EvalError {
    let reason = format!("{detail}; set {OPERATOR_KEY_ENV} to {MIN_SEAL_KEY_BYTES}+ bytes as hex");
    EvalError::SealKey { reason }
}

/// The eval error for a key the shared parser refused.
fn key_refusal(error: SealKeyError) -> EvalError {
    seal_key_error(match error {
        SealKeyError::NotHex => "key must be hex",
        SealKeyError::TooShort => "key is too short",
        SealKeyError::Missing => "operator key is missing",
    })
}

/// Parse an operator seal key from hex text (even-length hex decoding to at
/// least [`MIN_SEAL_KEY_BYTES`] bytes). Malformed keys fail loudly.
pub fn parse_seal_key(text: &str) -> Result<Vec<u8>, EvalError> {
    engine_core::parse_seal_key(text).map_err(key_refusal)
}

/// Read the operator seal key from [`OPERATOR_KEY_ENV`]; verify fails closed
/// on a missing key, never open.
pub fn operator_seal_key() -> Result<Vec<u8>, EvalError> {
    seal_key_from_env(OPERATOR_KEY_ENV)
}

/// Named-variable read for [`operator_seal_key`]; testable without mutation.
pub(super) fn seal_key_from_env(var: &str) -> Result<Vec<u8>, EvalError> {
    engine_core::seal_key_from_env(var).map_err(key_refusal)
}

/// Seal manifest bytes with the operator key: HMAC-SHA256 from the `hmac`
/// crate (never hand-rolled), rendered as `hmac-sha256:<64 hex>\n`.
pub(super) fn seal_manifest(manifest: &[u8], key: &[u8]) -> Result<String, EvalError> {
    let mut mac = Hmac::<Sha256>::new_from_slice(key)
        .map_err(|_| seal_key_error("key was rejected by the HMAC backend"))?;
    mac.update(manifest);
    let tag = mac.finalize().into_bytes();
    let hex: String = tag.iter().map(|byte| format!("{byte:02x}")).collect();
    Ok(format!("{SEAL_SCHEME}:{hex}\n"))
}

/// Check a seal against manifest bytes under the operator key. Unkeyed,
/// malformed or mismatched seals fail typed [`EvalError::DigestMismatch`]
/// naming [`EVIDENCE_SEAL`]; comparison runs in constant time.
pub(super) fn verify_manifest_seal(
    manifest: &[u8],
    seal: &str,
    key: &[u8],
) -> Result<(), EvalError> {
    let scheme = format!("{SEAL_SCHEME}:<64 hex>");
    let mismatch = |want: &str, got: &str| EvalError::DigestMismatch {
        file: EVIDENCE_SEAL.to_owned(),
        want: want.to_owned(),
        got: got.to_owned(),
    };
    let Some(tag_hex) = seal
        .trim()
        .strip_prefix(SEAL_SCHEME)
        .and_then(|rest| rest.strip_prefix(':'))
    else {
        return Err(mismatch(&scheme, "unkeyed-seal"));
    };
    let Some(tag) = decode_seal_hex(tag_hex).filter(|tag| tag.len() == 32) else {
        return Err(mismatch(&scheme, "malformed-seal"));
    };
    let mut mac = Hmac::<Sha256>::new_from_slice(key)
        .map_err(|_| seal_key_error("key was rejected by the HMAC backend"))?;
    mac.update(manifest);
    if mac.verify_slice(&tag).is_err() {
        return Err(mismatch("valid-hmac-sha256-tag", tag_hex));
    }
    Ok(())
}
