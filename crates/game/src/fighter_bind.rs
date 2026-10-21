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
//! Budget: one-shot setup IO (one manifest read plus 4 body reads that
//! prove the body is there and not empty); never on the fixed tick.
//!
//! WEB6: every read goes through an [`AssetSource`], so the native build
//! reads the files under `content/fighters` and a browser reads the same
//! bytes out of a fetched bundle. The source names the body; the binding
//! keeps the logical path, never a host path.

use std::path::Path;

use engine_assets::asset_source::{AssetSource, SourceError};

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
    /// Checked body GLB, named by its logical path in the source
    /// (`glb/hero_body.glb`).
    pub body_glb: String,
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
    /// The source refused a read; the logical path and the reason are
    /// carried.
    Source(String),
    /// Manifest malformed or not the pinned bytes; reason carried.
    BadManifest(String),
    /// Body GLB missing; file name only (never an absolute path).
    MissingBody(String),
}

impl core::fmt::Display for BindError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Io(kind) => write!(f, "fighter bind io ({kind:?})"),
            Self::Source(reason) => write!(f, "fighter bind source ({reason})"),
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

/// Logical path of one body inside the fighters content tree
/// (`glb/hero_body.glb`).
#[must_use]
pub fn body_logical_path(variant: &str) -> String {
    format!("glb/{}", body_file_name(variant))
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
/// beside it) through the file-system source: the native door into
/// [`bind_all_from`]. A browser has no file system, so the same bind runs
/// there against a fetched bundle.
pub fn bind_all(content_dir: &Path) -> Result<Vec<FighterBinding>, BindError> {
    bind_all_from(&engine_assets::asset_source::FsSource::new(content_dir))
}

/// Map a source failure to a bind failure: a missing name is the file
/// system kind the native path always reported, anything else is loud.
fn source_error(path: &str, err: &SourceError) -> BindError {
    match err {
        SourceError::Missing { .. } => BindError::Io(std::io::ErrorKind::NotFound),
        SourceError::Io { kind, .. } => BindError::Io(*kind),
        other => BindError::Source(cap_message(&format!("{path}: {other}"))),
    }
}

/// Bind all 4 bodies out of a source (`manifest.json` plus `glb/` beside
/// it): parse each variant block, pin both SHAs against the pinned
/// constants, check joints/clips and require the body GLB to be there and
/// non-empty. Manifest order is preserved in the returned vector.
pub fn bind_all_from(source: &dyn AssetSource) -> Result<Vec<FighterBinding>, BindError> {
    let json = source
        .read(MANIFEST_NAME)
        .map_err(|err| source_error(MANIFEST_NAME, &err))?;
    let json = String::from_utf8(json.into_owned())
        .map_err(|_| BindError::BadManifest(format!("{MANIFEST_NAME} is not utf-8")))?;
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
        let logical = body_logical_path(variant);
        let bytes = source.read(&logical).map_err(|err| match err {
            SourceError::Missing { .. } => BindError::MissingBody(name.clone()),
            other => source_error(&logical, &other),
        })?;
        if bytes.is_empty() {
            return Err(BindError::MissingBody(name.clone()));
        }
        bindings.push(FighterBinding {
            variant: (*variant).to_string(),
            bundle_sha256: bundle,
            body_glb_sha256: body_sha,
            body_glb: logical,
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
    use engine_assets::asset_source::{content_digest, FsSource, MemorySource};
    use std::path::PathBuf;

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
            assert_eq!(binding.body_glb, body_logical_path(&binding.variant));
            assert!(
                content_dir().join(&binding.body_glb).is_file(),
                "body missing: {}",
                binding.body_glb
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

    /// WEB6: the same bytes give the same bindings through any source, so a
    /// browser bind and a native bind agree.
    #[test]
    fn the_seam_binds_the_same_bodies_as_the_files() {
        let dir = fixture_dir("seam");
        let manifest = synth_manifest(EXPECTED_BUNDLE_SHA256[0]);
        std::fs::write(dir.join(MANIFEST_NAME), &manifest).unwrap();
        let mut memory = MemorySource::new();
        memory
            .insert(MANIFEST_NAME, manifest.into_bytes())
            .expect("the manifest is a logical path");
        for variant in VARIANTS {
            let name = body_file_name(variant);
            std::fs::write(dir.join("glb").join(&name), b"glb-bytes").unwrap();
            memory
                .insert(&body_logical_path(variant), b"glb-bytes".to_vec())
                .expect("the body is a logical path");
        }
        let from_files = bind_all(&dir).expect("the files bind");
        let from_seam = bind_all_from(&memory).expect("the seam binds");
        assert_eq!(
            from_files, from_seam,
            "the same bytes give the same bindings"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// WEB6: a body the source does not hold fails loudly, with its name.
    #[test]
    fn a_missing_body_through_the_seam_fails_by_name() {
        let mut memory = MemorySource::new();
        memory
            .insert(
                MANIFEST_NAME,
                synth_manifest(EXPECTED_BUNDLE_SHA256[0]).into_bytes(),
            )
            .expect("the manifest is a logical path");
        let err = bind_all_from(&memory).expect_err("no body is a refusal");
        assert!(
            matches!(err, BindError::MissingBody(_)),
            "wrong error: {err}"
        );
        assert!(
            err.to_string().contains("hero_body.glb"),
            "culprit lost: {err}"
        );
    }

    /// WEB6: the same files give the same content digest through any source,
    /// so a browser load can be compared with the native one.
    #[test]
    fn the_content_digest_is_the_same_through_any_source() {
        let dir = fixture_dir("digest");
        let manifest = synth_manifest(EXPECTED_BUNDLE_SHA256[0]);
        std::fs::write(dir.join(MANIFEST_NAME), &manifest).unwrap();
        let mut memory = MemorySource::new();
        memory
            .insert(MANIFEST_NAME, manifest.into_bytes())
            .expect("the manifest is a logical path");
        for variant in VARIANTS {
            let name = body_file_name(variant);
            std::fs::write(dir.join("glb").join(&name), b"glb-bytes").unwrap();
            memory
                .insert(&body_logical_path(variant), b"glb-bytes".to_vec())
                .expect("the body is a logical path");
        }
        let from_files = content_digest(&FsSource::new(&dir), "").expect("the files digest");
        let from_seam = content_digest(&memory, "").expect("the seam digests");
        assert_eq!(from_files, from_seam, "the same files give the same digest");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
