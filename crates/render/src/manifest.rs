//! manifest: PF2 capture manifest for Q1 reference captures.
//!
//! Purpose: carry the four required capture fields (adapter, driver,
//! resolution + quality tier, scene digest) so every capture names the
//! device and the scene it ran. RA8 owns the shape; the orchestrator
//! registers it in `lib.rs` (this file stays unregistered until then).
//! Owner: render lane (RA8). Consumer: PF2 frame captures / Q1.
//! Units: pixels for resolution, bytes for capture blobs held elsewhere
//! (never here), ms for pass timings held in `executor.rs`.
//! Determinism: field order in `CaptureManifest::summary` is fixed
//! (adapter, driver, resolution, quality, digest); no timestamps.
//! Errors: `ManifestError` names the missing or invalid field loudly.
//!
//! Receipt-shape note (RA8 reconciles by documenting, never renaming):
//! - `graph::FrameReceipt` is the compiled-graph receipt (draws +
//!   transient aliasing bytes/pct).
//! - `present::FrameReceipt` is the presenter receipt (resolution +
//!   validation errors + cache-budget rows).
//! - `executor::ExecReceipt` is the CPU-executor receipt (per-pass CPU
//!   ms plus GPU timestamp scaffolding, `costliest_pass` for captures).
//! - This manifest wraps a capture with the device + scene identity
//!   none of those receipts carries alone.
//!
//! Std only: zero crate imports, so this file also compiles alone via
//! `rustc --edition 2021 --test crates/render/src/manifest.rs`.

use std::error::Error;
use std::fmt;

/// Quality tier recorded on the capture. Minimal is the AH2
/// out-of-memory floor; the rest are the content quality ladder.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QualityTier {
    /// AH2 degrade floor after out-of-memory.
    Minimal,
    /// Low content quality.
    Low,
    /// Medium content quality.
    Medium,
    /// High content quality.
    High,
    /// Ultra content quality.
    Ultra,
}

impl QualityTier {
    /// Every tier in manifest order; doubles as the parse table.
    pub const ALL: &'static [Self] = &[
        Self::Minimal,
        Self::Low,
        Self::Medium,
        Self::High,
        Self::Ultra,
    ];

    /// Manifest label quoted on the capture.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Minimal => "minimal",
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Ultra => "ultra",
        }
    }

    /// Parse a manifest label, refusing anything else loudly.
    pub fn from_name(name: &str) -> Result<Self, ManifestError> {
        Self::ALL
            .iter()
            .find(|tier| tier.name() == name)
            .copied()
            .ok_or_else(|| ManifestError::UnknownQuality(name.to_owned()))
    }
}

/// One PF2 capture manifest: device identity plus scene identity.
/// All four field groups are required: adapter, driver, resolution +
/// quality tier, scene digest. Missing or empty fields never default.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CaptureManifest {
    /// Adapter name behind the capture (for example `NVIDIA RTX 3050`).
    pub adapter: String,
    /// Driver string behind the capture (for example `537.13`).
    pub driver: String,
    /// Capture width in pixels.
    pub width_px: u32,
    /// Capture height in pixels.
    pub height_px: u32,
    /// Content quality tier at capture time.
    pub quality: QualityTier,
    /// Scene digest (hex content digest of the captured scene).
    pub scene_digest: String,
}

impl CaptureManifest {
    /// Build a manifest, refusing every missing or invalid field loudly.
    /// Empty adapter/driver, zero resolution and empty digests all fail
    /// with the field named; unknown quality labels fail at parse time.
    pub fn new(
        adapter: &str,
        driver: &str,
        width_px: u32,
        height_px: u32,
        quality: QualityTier,
        scene_digest: &str,
    ) -> Result<Self, ManifestError> {
        if adapter.is_empty() {
            return Err(ManifestError::EmptyField("adapter"));
        }
        if driver.is_empty() {
            return Err(ManifestError::EmptyField("driver"));
        }
        if width_px == 0 || height_px == 0 {
            return Err(ManifestError::InvalidResolution {
                width_px,
                height_px,
            });
        }
        if scene_digest.is_empty() {
            return Err(ManifestError::EmptyField("scene_digest"));
        }
        if scene_digest.len() > 256 || !scene_digest.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(ManifestError::InvalidDigest(scene_digest.to_owned()));
        }
        Ok(Self {
            adapter: adapter.to_owned(),
            driver: driver.to_owned(),
            width_px,
            height_px,
            quality,
            scene_digest: scene_digest.to_owned(),
        })
    }

    /// Start a field-by-field builder; [`CaptureManifestBuilder::build`]
    /// refuses unset fields with [`ManifestError::MissingField`].
    #[must_use]
    pub fn builder() -> CaptureManifestBuilder {
        CaptureManifestBuilder::default()
    }

    /// Fixed-order one-line summary for capture logs:
    /// adapter, driver, resolution, quality, digest.
    #[must_use]
    pub fn summary(&self) -> String {
        format!(
            "adapter={} driver={} resolution={}x{} quality={} scene_digest={}",
            self.adapter,
            self.driver,
            self.width_px,
            self.height_px,
            self.quality.name(),
            self.scene_digest
        )
    }
}

/// Field-by-field builder; unset fields fail loudly at [`Self::build`].
#[derive(Clone, Debug, Default)]
pub struct CaptureManifestBuilder {
    adapter: Option<String>,
    driver: Option<String>,
    width_px: Option<u32>,
    height_px: Option<u32>,
    quality: Option<QualityTier>,
    scene_digest: Option<String>,
}

impl CaptureManifestBuilder {
    /// Set the adapter name.
    #[must_use]
    pub fn adapter(mut self, adapter: &str) -> Self {
        self.adapter = Some(adapter.to_owned());
        self
    }

    /// Set the driver string.
    #[must_use]
    pub fn driver(mut self, driver: &str) -> Self {
        self.driver = Some(driver.to_owned());
        self
    }

    /// Set the capture resolution in pixels.
    #[must_use]
    pub fn resolution(mut self, width_px: u32, height_px: u32) -> Self {
        self.width_px = Some(width_px);
        self.height_px = Some(height_px);
        self
    }

    /// Set the content quality tier.
    #[must_use]
    pub fn quality(mut self, quality: QualityTier) -> Self {
        self.quality = Some(quality);
        self
    }

    /// Set the scene digest (hex).
    #[must_use]
    pub fn scene_digest(mut self, digest: &str) -> Self {
        self.scene_digest = Some(digest.to_owned());
        self
    }

    /// Build the manifest, refusing the first unset field group loudly.
    pub fn build(self) -> Result<CaptureManifest, ManifestError> {
        let adapter = self.adapter.ok_or(ManifestError::MissingField("adapter"))?;
        let driver = self.driver.ok_or(ManifestError::MissingField("driver"))?;
        let width_px = self
            .width_px
            .ok_or(ManifestError::MissingField("resolution"))?;
        let height_px = self
            .height_px
            .ok_or(ManifestError::MissingField("resolution"))?;
        let quality = self.quality.ok_or(ManifestError::MissingField("quality"))?;
        let scene_digest = self
            .scene_digest
            .ok_or(ManifestError::MissingField("scene_digest"))?;
        CaptureManifest::new(
            &adapter,
            &driver,
            width_px,
            height_px,
            quality,
            &scene_digest,
        )
    }
}

/// Typed manifest failures; every variant names the offending field.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ManifestError {
    /// A builder field was never set.
    MissingField(&'static str),
    /// A required string field was empty.
    EmptyField(&'static str),
    /// Resolution has a zero dimension.
    InvalidResolution { width_px: u32, height_px: u32 },
    /// Scene digest is not lowercase/uppercase hex or is too long.
    InvalidDigest(String),
    /// Quality label is not a [`QualityTier::name`].
    UnknownQuality(String),
    /// The capture is a known placeholder, not a real scene (RL28).
    /// Carries the pattern name: `cube-sha`, `noise-64x48` or
    /// `identical-frames`.
    PlaceholderPattern(&'static str),
}

impl fmt::Display for ManifestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingField(field) => write!(f, "capture manifest missing field: {field}"),
            Self::EmptyField(field) => write!(f, "capture manifest empty field: {field}"),
            Self::InvalidResolution {
                width_px,
                height_px,
            } => write!(
                f,
                "capture manifest invalid resolution: {width_px}x{height_px}"
            ),
            Self::InvalidDigest(digest) => {
                write!(f, "capture manifest invalid scene digest: {digest}")
            }
            Self::UnknownQuality(name) => {
                write!(f, "capture manifest unknown quality tier: {name}")
            }
            Self::PlaceholderPattern(pattern) => {
                write!(f, "capture manifest placeholder refused: {pattern}")
            }
        }
    }
}

impl Error for ManifestError {}

/// Smallest frame run that still counts as a stuck capture (RL28).
pub const IDENTICAL_FRAMES_FLOOR: u32 = 30;

/// One capture fingerprint for placeholder refusal (RL28): the scene
/// digest plus the frame shape Q1 checks before accepting a capture.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CaptureFingerprint {
    /// Scene digest of the capture.
    pub scene_digest: String,
    /// Capture width in pixels.
    pub width_px: u32,
    /// Capture height in pixels.
    pub height_px: u32,
    /// Frames in the run.
    pub frame_count: u32,
    /// Frames byte-identical to the first frame.
    pub identical_frames: u32,
}

impl CaptureFingerprint {
    /// Build a fingerprint; empty digests fail loudly like the manifest.
    pub fn new(
        scene_digest: &str,
        width_px: u32,
        height_px: u32,
        frame_count: u32,
        identical_frames: u32,
    ) -> Result<Self, ManifestError> {
        if scene_digest.is_empty() {
            return Err(ManifestError::EmptyField("scene_digest"));
        }
        Ok(Self {
            scene_digest: scene_digest.to_owned(),
            width_px,
            height_px,
            frame_count,
            identical_frames,
        })
    }
}

/// Refuse a known placeholder capture, naming its pattern (RL28).
/// `known_cube_shas` holds the measured single-cube capture digests;
/// the Q1 pilot passes the real ones, tests pass seeded ones.
/// A 64x48 frame is the noise placeholder; a run of
/// [`IDENTICAL_FRAMES_FLOOR`] or more identical frames is a stuck
/// capture. Real captures pass with `Ok(())`.
pub fn refuse_placeholder(
    fp: &CaptureFingerprint,
    known_cube_shas: &[&str],
) -> Result<(), ManifestError> {
    if known_cube_shas.contains(&fp.scene_digest.as_str()) {
        return Err(ManifestError::PlaceholderPattern("cube-sha"));
    }
    if fp.width_px == 64 && fp.height_px == 48 {
        return Err(ManifestError::PlaceholderPattern("noise-64x48"));
    }
    if fp.frame_count >= IDENTICAL_FRAMES_FLOOR && fp.identical_frames >= fp.frame_count {
        return Err(ManifestError::PlaceholderPattern("identical-frames"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const DIGEST: &str = "d57b5c8ae3096614d57b5c8ae3096614d57b5c8ae3096614d57b5c8ae3096614";

    fn full_builder() -> CaptureManifestBuilder {
        CaptureManifest::builder()
            .adapter("NVIDIA RTX 3050 6GB Laptop GPU")
            .driver("537.13")
            .resolution(1920, 1080)
            .quality(QualityTier::High)
            .scene_digest(DIGEST)
    }

    #[test]
    fn full_manifest_builds_and_summarizes_in_field_order() {
        let manifest = full_builder().build().expect("full fields build");
        assert_eq!(manifest.width_px, 1920);
        assert_eq!(manifest.height_px, 1080);
        assert_eq!(manifest.quality, QualityTier::High);
        let summary = manifest.summary();
        assert_eq!(
            summary,
            format!(
                "adapter=NVIDIA RTX 3050 6GB Laptop GPU driver=537.13 \
                 resolution=1920x1080 quality=high scene_digest={DIGEST}"
            )
        );
        // Fixed order: adapter before driver before resolution.
        let adapter_at = summary.find("adapter=").expect("adapter in summary");
        let driver_at = summary.find("driver=").expect("driver in summary");
        let res_at = summary.find("resolution=").expect("resolution in summary");
        let digest_at = summary.find("scene_digest=").expect("digest in summary");
        assert!(adapter_at < driver_at && driver_at < res_at && res_at < digest_at);
    }

    #[test]
    fn missing_adapter_refused_loudly() {
        let err = CaptureManifest::builder()
            .driver("537.13")
            .resolution(1920, 1080)
            .quality(QualityTier::High)
            .scene_digest(DIGEST)
            .build()
            .expect_err("adapter missing");
        assert_eq!(err, ManifestError::MissingField("adapter"));
        assert!(err.to_string().contains("adapter"));
    }

    #[test]
    fn missing_driver_refused_loudly() {
        let err = CaptureManifest::builder()
            .adapter("llvmpipe")
            .resolution(1280, 720)
            .quality(QualityTier::Low)
            .scene_digest(DIGEST)
            .build()
            .expect_err("driver missing");
        assert_eq!(err, ManifestError::MissingField("driver"));
    }

    #[test]
    fn missing_resolution_refused_loudly() {
        let err = CaptureManifest::builder()
            .adapter("llvmpipe")
            .driver("23.3")
            .quality(QualityTier::Medium)
            .scene_digest(DIGEST)
            .build()
            .expect_err("resolution missing");
        assert_eq!(err, ManifestError::MissingField("resolution"));
    }

    #[test]
    fn missing_quality_and_digest_refused_loudly() {
        let no_quality = full_builder_with_quality_removed();
        assert_eq!(no_quality, ManifestError::MissingField("quality"));
        let no_digest = CaptureManifest::builder()
            .adapter("llvmpipe")
            .driver("23.3")
            .resolution(1280, 720)
            .quality(QualityTier::Medium)
            .build()
            .expect_err("digest missing");
        assert_eq!(no_digest, ManifestError::MissingField("scene_digest"));
    }

    fn full_builder_with_quality_removed() -> ManifestError {
        CaptureManifest::builder()
            .adapter("llvmpipe")
            .driver("23.3")
            .resolution(1280, 720)
            .scene_digest(DIGEST)
            .build()
            .expect_err("quality missing")
    }

    #[test]
    fn empty_and_invalid_fields_refused() {
        assert_eq!(
            CaptureManifest::new("", "d", 8, 8, QualityTier::Low, DIGEST)
                .expect_err("empty adapter"),
            ManifestError::EmptyField("adapter")
        );
        assert_eq!(
            CaptureManifest::new("a", "", 8, 8, QualityTier::Low, DIGEST)
                .expect_err("empty driver"),
            ManifestError::EmptyField("driver")
        );
        assert_eq!(
            CaptureManifest::new("a", "d", 0, 1080, QualityTier::Low, DIGEST)
                .expect_err("zero width"),
            ManifestError::InvalidResolution {
                width_px: 0,
                height_px: 1080
            }
        );
        assert_eq!(
            CaptureManifest::new("a", "d", 8, 8, QualityTier::Low, "").expect_err("empty digest"),
            ManifestError::EmptyField("scene_digest")
        );
        assert_eq!(
            CaptureManifest::new("a", "d", 8, 8, QualityTier::Low, "not-hex!!")
                .expect_err("bad digest"),
            ManifestError::InvalidDigest("not-hex!!".to_owned())
        );
        assert_eq!(
            QualityTier::from_name("extreme").expect_err("bad quality"),
            ManifestError::UnknownQuality("extreme".to_owned())
        );
    }

    #[test]
    fn quality_round_trips_through_names() {
        for tier in QualityTier::ALL {
            assert_eq!(QualityTier::from_name(tier.name()), Ok(*tier));
        }
    }

    const CUBE_SHA: &str = "cube-cube-cube-cube-cube-cube-cube00";

    fn real_fingerprint(digest: &str) -> CaptureFingerprint {
        CaptureFingerprint::new(digest, 1920, 1080, 120, 3).expect("real fingerprint builds")
    }

    #[test]
    fn cube_sha_placeholder_refused_naming_the_pattern() {
        let fp =
            CaptureFingerprint::new(CUBE_SHA, 1920, 1080, 120, 3).expect("cube fingerprint builds");
        let err = refuse_placeholder(&fp, &[CUBE_SHA]).expect_err("cube sha refused");
        assert_eq!(err, ManifestError::PlaceholderPattern("cube-sha"));
        assert!(err.to_string().contains("cube-sha"));
    }

    #[test]
    fn noise_64x48_placeholder_refused_naming_the_pattern() {
        let fp =
            CaptureFingerprint::new("abcd1234", 64, 48, 3, 1).expect("noise fingerprint builds");
        let err = refuse_placeholder(&fp, &[CUBE_SHA]).expect_err("noise refused");
        assert_eq!(err, ManifestError::PlaceholderPattern("noise-64x48"));
        assert!(err.to_string().contains("noise-64x48"));
    }

    #[test]
    fn identical_frames_placeholder_refused_naming_the_pattern() {
        let fp = CaptureFingerprint::new("abcd1234", 1920, 1080, 30, 30)
            .expect("stuck fingerprint builds");
        let err = refuse_placeholder(&fp, &[CUBE_SHA]).expect_err("stuck run refused");
        assert_eq!(err, ManifestError::PlaceholderPattern("identical-frames"));
        assert!(err.to_string().contains("identical-frames"));
    }

    #[test]
    fn three_real_captures_pass() {
        for digest in ["aa00ff11", "bb11ee22", "cc22dd33"] {
            assert_eq!(
                refuse_placeholder(&real_fingerprint(digest), &[CUBE_SHA]),
                Ok(())
            );
        }
    }

    #[test]
    fn empty_digest_fingerprint_refused() {
        assert_eq!(
            CaptureFingerprint::new("", 1920, 1080, 120, 3).expect_err("empty digest"),
            ManifestError::EmptyField("scene_digest")
        );
    }
}
