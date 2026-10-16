//! fighter_bind: bind of the hero + 3 opponents (content bundles) for the staged fight.
//!
//! Consumer: `src/bin/staged_fight.rs` binds all 4 bodies, runs the scripted
//! headless fight and files the captures plus receipt. Owner: this module owns
//! manifest parsing plus the body-file checks. Units: SHAs as lowercase hex,
//! joints and clip counts as plain counts. Determinism: the same manifest
//! bytes give the same bindings in manifest order; no wall clock, no RNG.
//! Errors: [`BindError`] at every boundary; a bad manifest or a missing body
//! fails loudly, never substitutes another character.
//!
//! Budget: one-shot setup IO (one manifest read plus 4 existence checks);
//! never on the fixed tick.

use std::path::{Path, PathBuf};

use crate::capture::cap_message;

/// Variant names in manifest order: the hero plus 3 opponents.
pub const VARIANTS: [&str; 4] = [
    "hero",
    "opp_male_light",
    "opp_female_medium",
    "opp_female_light",
];
/// Bundle SHAs in [`VARIANTS`] order, read from
/// `content/fighters/manifest.json` and pinned here so a recooked manifest
/// with different bytes fails loudly instead of staging the wrong bodies.
/// Hero entry carries the fixed in-hand sword/shield graft; the three
/// opponent entries keep their pinned bytes.
pub const EXPECTED_BUNDLE_SHA256: [&str; 4] = [
    "522e78a82eef3e8b63cd353de89e6889e5c73ea311f0b9c18107185e53b9ea2b",
    "0c1b528c56f0e9d12d6ebda793cef19b9e463919f9f89db6e0d25f67c742e89b",
    "9a341a41d4aa42f27171800865dcd12a06c62c4e8e6773ae114355072c72a632",
    "a9d1f3f97dffd89e90bc54fcccad01b6ec349e5bde93b081a36fcf9b5da0c99e",
];
/// Assembled body GLB SHAs in [`VARIANTS`] order (same manifest source).
/// Hero entry carries the fixed in-hand sword/shield graft.
pub const EXPECTED_BODY_SHA256: [&str; 4] = [
    "4bdc600a7acb1fa057f33e5c680bff9f74d047f5d2de22df6a8addc1e94bbbd5",
    "9f4d71e67365af68d85ad2e17541ca22a87956e7a235171e27953c32a04a7ce1",
    "08427dd6aaef4bdb9a317c023306778555503c1ea16d82747aaea9ef84cb6886",
    "ef908b0fa2b4af5e9880dc87ccbd77002178c64bae4dfebec06b78e5e6595bd2",
];
/// One skeleton for all 4 bodies: the 45-joint rig (armature + meshes).
pub const EXPECTED_JOINTS: u32 = 45;
/// Baked FK clips per variant (idle, run, jump, death, combat set, taunt).
pub const EXPECTED_CLIPS: u32 = 14;
/// Manifest file name inside the fighters content directory.
pub const MANIFEST_NAME: &str = "manifest.json";

/// One bound fighter body: manifest identity plus the checked GLB path.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct FighterBinding {
    /// Variant name (`hero`, `opp_male_light`, ...).
    pub variant: String,
    /// Bundle SHA-256 (pinned against [`EXPECTED_BUNDLE_SHA256`]).
    pub bundle_sha256: String,
    /// Assembled body GLB SHA-256 (pinned against [`EXPECTED_BODY_SHA256`]).
    pub body_glb_sha256: String,
    /// Checked body GLB file (`<variant>_body.glb` under `glb/`).
    pub body_glb: PathBuf,
    /// Rig joints (always [`EXPECTED_JOINTS`]).
    pub joints: u32,
    /// LOD0 triangles of the assembled body.
    pub tris_lod0: u32,
    /// Baked clips (always [`EXPECTED_CLIPS`]).
    pub clips: u32,
}

/// Bind failures: bounded diagnostics, never panics.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum BindError {
    /// Filesystem failure; kind carried (paths stay in the caller).
    Io(std::io::ErrorKind),
    /// Manifest malformed or not the pinned bytes; reason carried.
    BadManifest(String),
    /// Body GLB missing; file name only (never an absolute path).
    MissingBody(String),
}

impl core::fmt::Display for BindError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Io(kind) => write!(f, "fighter bind io ({kind:?})"),
            Self::BadManifest(reason) => write!(f, "bad fighter manifest ({reason})"),
            Self::MissingBody(name) => write!(f, "missing fighter body ({name})"),
        }
    }
}

impl std::error::Error for BindError {}

/// Body GLB file name for a variant (`hero` -> `hero_body.glb`).
#[must_use]
pub fn body_file_name(variant: &str) -> String {
    format!("{variant}_body.glb")
}

/// True for 64 lowercase hex chars (a SHA-256 row).
fn is_hex64(text: &str) -> bool {
    text.len() == 64 && text.chars().all(|c| c.is_ascii_hexdigit())
}

/// Locate `"name": "<variant>"` in the manifest; the variant block runs to
/// the next variant marker (or EOF), so field search never crosses bodies.
fn variant_block<'a>(json: &'a str, variant: &str) -> Result<&'a str, BindError> {
    let marker = format!("\"name\": \"{variant}\"");
    let start = json.find(marker.as_str()).ok_or_else(|| {
        BindError::BadManifest(cap_message(&format!("variant {variant} not in manifest")))
    })?;
    let rest = &json[start..];
    let mut end = rest.len();
    for other in VARIANTS {
        if other == variant {
            continue;
        }
        let next = format!("\"name\": \"{other}\"");
        if let Some(at) = rest[marker.len()..].find(next.as_str()) {
            end = end.min(marker.len() + at);
        }
    }
    Ok(&rest[..end])
}

/// Extract `"key": "<hex64>"` from a variant block.
fn extract_hex64(block: &str, variant: &str, key: &str) -> Result<String, BindError> {
    let marker = format!("\"{key}\"");
    let at = block
        .find(marker.as_str())
        .ok_or_else(|| BindError::BadManifest(cap_message(&format!("{variant} has no {key}"))))?;
    let after = &block[at + marker.len()..];
    let colon = after.find(':').ok_or_else(|| {
        BindError::BadManifest(cap_message(&format!("{variant} {key} has no value")))
    })?;
    let value = after[colon + 1..].trim_start();
    let quoted = value.strip_prefix('"').ok_or_else(|| {
        BindError::BadManifest(cap_message(&format!("{variant} {key} is not a string")))
    })?;
    let end = quoted.find('"').ok_or_else(|| {
        BindError::BadManifest(cap_message(&format!("{variant} {key} is unterminated")))
    })?;
    let hex = &quoted[..end];
    if !is_hex64(hex) {
        return Err(BindError::BadManifest(cap_message(&format!(
            "{variant} {key} is not a SHA-256"
        ))));
    }
    Ok(hex.to_string())
}

/// Extract `"key": <digits>` from a variant block.
fn extract_count(block: &str, variant: &str, key: &str) -> Result<u32, BindError> {
    let marker = format!("\"{key}\"");
    let at = block
        .find(marker.as_str())
        .ok_or_else(|| BindError::BadManifest(cap_message(&format!("{variant} has no {key}"))))?;
    let after = &block[at + marker.len()..];
    let colon = after.find(':').ok_or_else(|| {
        BindError::BadManifest(cap_message(&format!("{variant} {key} has no value")))
    })?;
    let value = after[colon + 1..].trim_start();
    let digits: String = value.chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        return Err(BindError::BadManifest(cap_message(&format!(
            "{variant} {key} is not a count"
        ))));
    }
    digits
        .parse::<u32>()
        .map_err(|_| BindError::BadManifest(cap_message(&format!("{variant} {key} overflows u32"))))
}

/// Bind all 4 bodies under `content_dir` (`manifest.json` plus `glb/`
/// beside it): parse each variant block, pin both SHAs against the pinned
/// constants, check joints/clips and require the body GLB to exist and be
/// non-empty. Manifest order is preserved in the returned vector.
pub fn bind_all(content_dir: &Path) -> Result<Vec<FighterBinding>, BindError> {
    let path = content_dir.join(MANIFEST_NAME);
    let json = std::fs::read_to_string(&path).map_err(|err| BindError::Io(err.kind()))?;
    let mut bindings = Vec::with_capacity(VARIANTS.len());
    for (index, variant) in VARIANTS.iter().enumerate() {
        let block = variant_block(&json, variant)?;
        let bundle = extract_hex64(block, variant, "bundle_sha256")?;
        if bundle != EXPECTED_BUNDLE_SHA256[index] {
            return Err(BindError::BadManifest(cap_message(&format!(
                "{variant} bundle is not the CT7 bytes"
            ))));
        }
        let body_sha = extract_hex64(block, variant, "body_glb_sha256")?;
        if body_sha != EXPECTED_BODY_SHA256[index] {
            return Err(BindError::BadManifest(cap_message(&format!(
                "{variant} body is not the CT7 bytes"
            ))));
        }
        let joints = extract_count(block, variant, "joints")?;
        if joints != EXPECTED_JOINTS {
            return Err(BindError::BadManifest(cap_message(&format!(
                "{variant} joints {joints} != {EXPECTED_JOINTS}"
            ))));
        }
        let clips = extract_count(block, variant, "clips")?;
        if clips != EXPECTED_CLIPS {
            return Err(BindError::BadManifest(cap_message(&format!(
                "{variant} clips {clips} != {EXPECTED_CLIPS}"
            ))));
        }
        let tris = extract_count(block, variant, "tris_lod0")?;
        if tris == 0 {
            return Err(BindError::BadManifest(cap_message(&format!(
                "{variant} has no triangles"
            ))));
        }
        let name = body_file_name(variant);
        let body_path = content_dir.join("glb").join(&name);
        let bytes = std::fs::metadata(&body_path).map_err(|err| {
            if err.kind() == std::io::ErrorKind::NotFound {
                BindError::MissingBody(name.clone())
            } else {
                BindError::Io(err.kind())
            }
        })?;
        if bytes.len() == 0 {
            return Err(BindError::MissingBody(name.clone()));
        }
        bindings.push(FighterBinding {
            variant: (*variant).to_string(),
            bundle_sha256: bundle,
            body_glb_sha256: body_sha,
            body_glb: body_path,
            joints,
            tris_lod0: tris,
            clips,
        });
    }
    Ok(bindings)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn content_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../content/fighters")
    }

    #[test]
    fn binds_all_four_ct7_bodies_from_manifest() {
        let bindings = bind_all(&content_dir()).expect("CT7 manifest binds");
        assert_eq!(bindings.len(), 4);
        for (index, binding) in bindings.iter().enumerate() {
            assert_eq!(binding.variant, VARIANTS[index]);
            assert_eq!(binding.bundle_sha256, EXPECTED_BUNDLE_SHA256[index]);
            assert_eq!(binding.body_glb_sha256, EXPECTED_BODY_SHA256[index]);
            assert_eq!(binding.joints, EXPECTED_JOINTS);
            assert_eq!(binding.clips, EXPECTED_CLIPS);
            assert!(binding.tris_lod0 > 0, "{}", binding.variant);
            assert!(
                binding.body_glb.is_file(),
                "body missing: {}",
                binding.body_glb.display()
            );
            assert_eq!(
                binding.body_glb.file_name().and_then(|n| n.to_str()),
                Some(body_file_name(&binding.variant).as_str())
            );
        }
    }

    /// Minimal manifest carrying the pinned SHAs (the parser only needs the
    /// variant blocks, so failure fixtures stay small and fast).
    fn synth_manifest(hero_bundle: &str) -> String {
        let mut out = String::from("{\"task\":\"CT15 fixture\",\"variants\":[");
        for (index, variant) in VARIANTS.iter().enumerate() {
            if index > 0 {
                out.push(',');
            }
            let bundle = if index == 0 {
                hero_bundle.to_string()
            } else {
                EXPECTED_BUNDLE_SHA256[index].to_string()
            };
            out.push_str(&format!(
                "{{\"name\": \"{variant}\", \"joints\": 45, \"tris_lod0\": 5000, \"clips\": 14, \"bundle_sha256\": \"{bundle}\", \"body_glb_sha256\": \"{}\"}}",
                EXPECTED_BODY_SHA256[index]
            ));
        }
        out.push_str("]}");
        out
    }

    fn fixture_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("e2040-bind-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("glb")).unwrap();
        dir
    }

    #[test]
    fn refuses_tampered_bundle_sha() {
        let dir = fixture_dir("tampered");
        std::fs::write(dir.join(MANIFEST_NAME), synth_manifest(&"0".repeat(64))).unwrap();
        let err = bind_all(&dir).expect_err("zeroed hero bundle must fail");
        assert!(
            matches!(err, BindError::BadManifest(_)),
            "wrong error: {err}"
        );
        assert!(err.to_string().contains("hero"), "culprit lost: {err}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn refuses_missing_body_glb() {
        let dir = fixture_dir("nobody");
        std::fs::write(
            dir.join(MANIFEST_NAME),
            synth_manifest(EXPECTED_BUNDLE_SHA256[0]),
        )
        .unwrap();
        // Manifest is the pinned bytes but no GLB was staged beside it.
        let err = bind_all(&dir).expect_err("missing bodies must fail");
        assert!(
            matches!(err, BindError::MissingBody(_)),
            "wrong error: {err}"
        );
        assert!(
            err.to_string().contains("hero_body.glb"),
            "culprit lost: {err}"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
