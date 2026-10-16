//! tier: RT0 startup lighting-tier probe plus RA8 manifest stamp.
//!
//! Owner: render lane (RT0). Consumer: RT9 tier-aware graph wiring.
//! Units: bytes for the Ultra memory gate; tiers are labels only.
//! Determinism: pure function of [`AdapterCaps`]; no clock, no ordering.
//! Errors: typed [`TierError`]; forced tiers lacking features refuse loudly.
//! Budget: probe is O(1) flag checks under the 0.1 ms startup budget; it
//! never runs per frame and allocates nothing on the probe path.

use core::fmt;

use crate::manifest::CaptureManifest;

/// Device memory at or above which ray-capable adapters qualify for Ultra.
/// The RTX 3050 6GB reference GPU (6 GiB) lands on Rt; stronger cards with
/// ray queries plus accel plus this much memory land on Ultra.
pub const ULTRA_MIN_MEMORY_BYTES: u64 = 8 * 1024 * 1024 * 1024;

/// Startup lighting tier picked once per process before any graph runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Tier {
    /// Any adapter, including software rasterizers without ray queries.
    Base,
    /// Ray-query shadows, ReSTIR DI/GI, reflections plus the denoiser.
    Rt,
    /// Rt at higher internal resolution with more bounces (photo converges).
    Ultra,
}

impl Tier {
    /// Every tier in ladder order; doubles as the parse table.
    pub const ALL: &'static [Self] = &[Self::Base, Self::Rt, Self::Ultra];

    /// Manifest label quoted on captures: `base`, `rt`, `ultra`.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Base => "base",
            Self::Rt => "rt",
            Self::Ultra => "ultra",
        }
    }

    /// Parse a manifest label, refusing anything else loudly.
    pub fn from_name(name: &str) -> Result<Self, TierError> {
        Self::ALL
            .iter()
            .find(|tier| tier.name() == name)
            .copied()
            .ok_or_else(|| TierError::UnknownTier(name.to_owned()))
    }

    /// True when these adapter capabilities can run this tier.
    /// Base always holds; Rt needs ray queries plus accel structures;
    /// Ultra additionally needs [`ULTRA_MIN_MEMORY_BYTES`] of device memory.
    #[must_use]
    pub fn supports(self, caps: &AdapterCaps) -> bool {
        match self {
            Self::Base => true,
            Self::Rt => caps.ray_query && caps.accel,
            Self::Ultra => caps.ray_query && caps.accel && caps.has_ultra_memory(),
        }
    }

    /// Why this tier cannot run here, or `None` when it can.
    #[must_use]
    pub fn missing_reason(self, caps: &AdapterCaps) -> Option<&'static str> {
        if self.supports(caps) {
            return None;
        }
        Some(match self {
            Self::Base => "base tier is always available",
            Self::Rt if !caps.ray_query => "adapter lacks wgpu EXPERIMENTAL_RAY_QUERY support",
            Self::Rt => "adapter lacks acceleration-structure support",
            Self::Ultra if !caps.ray_query => "adapter lacks wgpu EXPERIMENTAL_RAY_QUERY support",
            Self::Ultra if !caps.accel => "adapter lacks acceleration-structure support",
            Self::Ultra => "adapter reports less than 8 GiB of device memory",
        })
    }
}

impl fmt::Display for Tier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// Adapter ray-tracing capabilities read once at startup.
///
/// `ray_query` mirrors `wgpu::Features::EXPERIMENTAL_RAY_QUERY` and `accel`
/// mirrors `wgpu::Features::ACCELERATION_STRUCTURE_BINDING_ARRAY` (BLAS/TLAS
/// build plus bind). `device_memory_bytes` is 0 when the backend reports no
/// memory figure, which never qualifies for Ultra. The struct is backend
/// neutral so the probe unit-tests headless; the `window` bridge below fills
/// it from a live `wgpu::Adapter`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdapterCaps {
    /// Adapter name as reported by the backend (for example `llvmpipe` or
    /// `NVIDIA GeForce RTX 3050 6GB Laptop GPU`).
    pub adapter: String,
    /// Ray-query shader support (wgpu experimental ray query).
    pub ray_query: bool,
    /// Acceleration-structure build plus bind support.
    pub accel: bool,
    /// Device memory in bytes, or 0 when unknown.
    pub device_memory_bytes: u64,
}

impl AdapterCaps {
    /// Describe one adapter. Empty names are kept verbatim: heuristics only
    /// downgrade recognized software prefixes, never refuse a real device.
    pub fn new(adapter: &str, ray_query: bool, accel: bool, device_memory_bytes: u64) -> Self {
        Self {
            adapter: adapter.to_owned(),
            ray_query,
            accel,
            device_memory_bytes,
        }
    }

    /// Software-rasterizer fixture (llvmpipe): no ray queries, no accel.
    pub fn llvmpipe() -> Self {
        Self::new("llvmpipe (LLVM 18, 256 bits)", false, false, 0)
    }

    /// Reference-GPU fixture (RTX 3050 6GB Laptop): ray queries plus accel
    /// with 6 GiB, which probes to [`Tier::Rt`], never Ultra.
    pub fn reference_rt() -> Self {
        Self::new(
            "NVIDIA GeForce RTX 3050 6GB Laptop GPU",
            true,
            true,
            6 * 1024 * 1024 * 1024,
        )
    }

    /// True for recognized software rasterizers, matched case-insensitively
    /// on the adapter name (`llvmpipe`, `lavapipe`, `swiftshader`,
    /// `softpipe`, `software`, `basic render`).
    #[must_use]
    pub fn is_software(&self) -> bool {
        let lower = self.adapter.to_lowercase();
        [
            "llvmpipe",
            "lavapipe",
            "swiftshader",
            "softpipe",
            "software",
            "basic render",
        ]
        .iter()
        .any(|needle| lower.contains(needle))
    }

    /// True when device memory clears the Ultra gate.
    #[must_use]
    pub fn has_ultra_memory(&self) -> bool {
        self.device_memory_bytes >= ULTRA_MIN_MEMORY_BYTES
    }
}

/// Probe outcome: the picked tier plus the honest human-readable reason.
/// The reason always names the deciding capability so a fallback on capable
/// hardware (for example a reference GPU behind a wgpu build without the
/// experimental flags) stays diagnosable instead of silent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TierSelection {
    /// The picked tier.
    pub tier: Tier,
    /// Why this tier was picked; a fallback reason when below Rt.
    pub reason: &'static str,
}

impl TierSelection {
    /// Tier label quoted on captures.
    #[must_use]
    pub fn name(self) -> &'static str {
        self.tier.name()
    }
}

/// Typed tier failure. Forced tiers lacking features refuse loudly with the
/// request, the adapter's honest fallback and the missing capability named.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TierError {
    /// A forced tier cannot run on this adapter.
    Unsupported {
        /// The tier that was forced.
        requested: Tier,
        /// What the adapter honestly supports instead.
        fallback: Tier,
        /// The missing capability.
        reason: &'static str,
        /// Adapter name behind the refusal.
        adapter: String,
    },
    /// A forced tier label is not a [`Tier::name`].
    UnknownTier(String),
}

impl fmt::Display for TierError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unsupported {
                requested,
                fallback,
                reason,
                adapter,
            } => write!(
                f,
                "refused to force lighting tier {requested} on adapter \"{adapter}\": \
                 {reason} (honest fallback: {fallback})"
            ),
            Self::UnknownTier(name) => {
                write!(f, "unknown lighting tier: {name} (want base, rt or ultra)")
            }
        }
    }
}

impl std::error::Error for TierError {}

/// Pick the highest tier this adapter supports: Ultra when ray queries plus
/// accel plus Ultra memory hold, Rt when ray queries plus accel hold, else
/// Base. Pure flag checks; allocates nothing.
#[must_use]
pub fn probe(caps: &AdapterCaps) -> Tier {
    probe_detailed(caps).tier
}

/// Pick the tier with the honest reason attached. Software adapters and any
/// adapter without ray queries fall back to Base with the missing capability
/// named, so a reference GPU on a wgpu build without the experimental flags
/// reports Base plus its reason instead of a silent downgrade.
#[must_use]
pub fn probe_detailed(caps: &AdapterCaps) -> TierSelection {
    if Tier::Ultra.supports(caps) {
        return TierSelection {
            tier: Tier::Ultra,
            reason: "ray queries plus accel plus >= 8 GiB device memory",
        };
    }
    if Tier::Rt.supports(caps) {
        return TierSelection {
            tier: Tier::Rt,
            reason: "ray queries plus accel present",
        };
    }
    if caps.is_software() {
        return TierSelection {
            tier: Tier::Base,
            reason: "software adapter without ray queries; base cascade plus cluster path",
        };
    }
    if !caps.ray_query {
        return TierSelection {
            tier: Tier::Base,
            reason:
                "adapter lacks wgpu EXPERIMENTAL_RAY_QUERY support; base cascade plus cluster path",
        };
    }
    if !caps.accel {
        return TierSelection {
            tier: Tier::Base,
            reason: "adapter lacks acceleration-structure support; base cascade plus cluster path",
        };
    }
    TierSelection {
        tier: Tier::Base,
        reason: "adapter below the ultra memory gate without rt fallback; base cascade plus cluster path",
    }
}

/// Resolve the startup tier: `None` probes the adapter, `Some(tier)` forces
/// it and refuses loudly when the adapter lacks the features. Forced Base
/// always succeeds.
pub fn resolve(caps: &AdapterCaps, forced: Option<Tier>) -> Result<TierSelection, TierError> {
    let Some(requested) = forced else {
        return Ok(probe_detailed(caps));
    };
    if requested.supports(caps) {
        let reason: &'static str = match requested {
            Tier::Base => "base tier forced and always available",
            Tier::Rt => "rt tier forced and adapter reports ray queries plus accel",
            Tier::Ultra => "ultra tier forced and adapter clears the ultra gate",
        };
        return Ok(TierSelection {
            tier: requested,
            reason,
        });
    }
    Err(TierError::Unsupported {
        requested,
        fallback: probe(caps),
        reason: requested
            .missing_reason(caps)
            .unwrap_or("unsupported adapter"),
        adapter: caps.adapter.clone(),
    })
}

/// Resolve a forced tier by label (`base`, `rt`, `ultra`); `None` probes.
/// Unknown labels fail as [`TierError::UnknownTier`].
pub fn resolve_name(caps: &AdapterCaps, forced: Option<&str>) -> Result<TierSelection, TierError> {
    match forced {
        None => Ok(probe_detailed(caps)),
        Some(name) => resolve(caps, Some(Tier::from_name(name)?)),
    }
}

/// One-line RA8 manifest stamp for capture logs: the [`CaptureManifest`]
/// summary plus the probed `tier=<name>` field, so every RA8 manifest names
/// the lighting tier behind the capture. Allocates (startup/capture only,
/// never per frame).
#[must_use]
pub fn stamp_summary(manifest: &CaptureManifest, tier: Tier) -> String {
    format!("{} tier={}", manifest.summary(), tier.name())
}

/// An RA8 capture manifest stamped with its lighting tier. The manifest
/// struct itself is owned by RA8 (`manifest.rs`); this wrapper carries the
/// RT0 stamp RT9 reads for tier-aware graph wiring without duplicating the
/// manifest fields.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StampedManifest {
    manifest: CaptureManifest,
    tier: Tier,
}

impl StampedManifest {
    /// Attach the probed tier to a manifest. No validation beyond what
    /// [`CaptureManifest`] already enforces.
    pub fn new(manifest: CaptureManifest, tier: Tier) -> Self {
        Self { manifest, tier }
    }

    /// Probe the adapter and stamp the result in one step.
    pub fn probed(manifest: CaptureManifest, caps: &AdapterCaps) -> (Self, TierSelection) {
        let selection = probe_detailed(caps);
        (Self::new(manifest, selection.tier), selection)
    }

    /// Force a tier and stamp it, refusing loudly when unsupported.
    pub fn forced(
        manifest: CaptureManifest,
        caps: &AdapterCaps,
        forced: Tier,
    ) -> Result<Self, TierError> {
        let selection = resolve(caps, Some(forced))?;
        Ok(Self::new(manifest, selection.tier))
    }

    /// The stamped lighting tier.
    #[must_use]
    pub fn tier(&self) -> Tier {
        self.tier
    }

    /// The underlying RA8 manifest.
    #[must_use]
    pub fn manifest(&self) -> &CaptureManifest {
        &self.manifest
    }

    /// Fixed-order one-line summary ending in `tier=<name>`.
    #[must_use]
    pub fn summary(&self) -> String {
        stamp_summary(&self.manifest, self.tier)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::QualityTier;

    const DIGEST: &str = "d57b5c8ae3096614d57b5c8ae3096614d57b5c8ae3096614d57b5c8ae3096614";

    fn manifest() -> CaptureManifest {
        CaptureManifest::new(
            "NVIDIA GeForce RTX 3050 6GB Laptop GPU",
            "537.13",
            1920,
            1080,
            QualityTier::High,
            DIGEST,
        )
        .expect("fixture manifest builds")
    }

    #[test]
    fn tier_strings_report_base_rt_ultra() {
        assert_eq!(
            (Tier::Base.name(), Tier::Rt.name(), Tier::Ultra.name()),
            ("base", "rt", "ultra")
        );
        for tier in Tier::ALL {
            assert_eq!(Tier::from_name(tier.name()), Ok(*tier));
            assert_eq!(tier.to_string(), tier.name());
        }
    }

    #[test]
    fn unknown_tier_label_refused() {
        assert_eq!(
            Tier::from_name("extreme"),
            Err(TierError::UnknownTier("extreme".to_owned()))
        );
        assert!(Tier::from_name("extreme")
            .expect_err("bad label")
            .to_string()
            .contains("extreme"));
    }

    #[test]
    fn llvmpipe_reports_base() {
        let caps = AdapterCaps::llvmpipe();
        assert!(caps.is_software());
        assert_eq!(probe(&caps), Tier::Base);
        let detailed = probe_detailed(&caps);
        assert_eq!(detailed.tier, Tier::Base);
        assert!(!detailed.reason.is_empty());
    }

    #[test]
    fn reference_gpu_reports_rt() {
        let caps = AdapterCaps::reference_rt();
        assert!(!caps.is_software());
        assert_eq!(probe(&caps), Tier::Rt);
        assert_eq!(
            probe_detailed(&caps),
            TierSelection {
                tier: Tier::Rt,
                reason: "ray queries plus accel present",
            }
        );
    }

    #[test]
    fn reference_gpu_without_experimental_falls_back_with_reason() {
        let caps = AdapterCaps::new(
            "NVIDIA GeForce RTX 3050 6GB Laptop GPU",
            false,
            false,
            6 * 1024 * 1024 * 1024,
        );
        let detailed = probe_detailed(&caps);
        assert_eq!(detailed.tier, Tier::Base);
        assert!(
            detailed.reason.contains("EXPERIMENTAL_RAY_QUERY"),
            "honest reason names the missing flag, got: {}",
            detailed.reason
        );
    }

    #[test]
    fn accel_without_ray_query_still_base() {
        let caps = AdapterCaps::new("some discrete gpu", false, true, 12 * 1024 * 1024 * 1024);
        assert_eq!(probe(&caps), Tier::Base);
    }

    #[test]
    fn ultra_needs_memory_above_the_gate() {
        let strong = AdapterCaps::new("big discrete gpu", true, true, 16 * 1024 * 1024 * 1024);
        assert_eq!(probe(&strong), Tier::Ultra);
        let reference = AdapterCaps::reference_rt();
        assert_eq!(probe(&reference), Tier::Rt);
        assert_eq!(
            Tier::Ultra.missing_reason(&reference),
            Some("adapter reports less than 8 GiB of device memory")
        );
    }

    #[test]
    fn forced_rt_on_llvmpipe_refused_loudly() {
        let caps = AdapterCaps::llvmpipe();
        let err = resolve(&caps, Some(Tier::Rt)).expect_err("forced rt must refuse");
        assert_eq!(
            err,
            TierError::Unsupported {
                requested: Tier::Rt,
                fallback: Tier::Base,
                reason: "adapter lacks wgpu EXPERIMENTAL_RAY_QUERY support",
                adapter: caps.adapter.clone(),
            }
        );
        let text = err.to_string();
        assert!(text.contains("rt"), "names the request: {text}");
        assert!(text.contains("llvmpipe"), "names the adapter: {text}");
        assert!(text.contains("base"), "names the fallback: {text}");
    }

    #[test]
    fn forced_ultra_on_reference_refused_with_memory_reason() {
        let caps = AdapterCaps::reference_rt();
        let err = resolve(&caps, Some(Tier::Ultra)).expect_err("6 gib is not ultra");
        assert!(err.to_string().contains("8 GiB"), "names the gate: {err}");
    }

    #[test]
    fn forced_base_always_succeeds() {
        let caps = AdapterCaps::llvmpipe();
        assert_eq!(
            resolve(&caps, Some(Tier::Base)).expect("base").tier,
            Tier::Base
        );
        assert_eq!(resolve(&caps, None).expect("probe").tier, Tier::Base);
        let rt = AdapterCaps::reference_rt();
        assert_eq!(resolve(&rt, Some(Tier::Rt)).expect("rt").tier, Tier::Rt);
    }

    #[test]
    fn forced_unknown_label_refused() {
        let caps = AdapterCaps::llvmpipe();
        assert_eq!(
            resolve_name(&caps, Some("extreme")),
            Err(TierError::UnknownTier("extreme".to_owned()))
        );
        assert!(resolve_name(&caps, Some("rt"))
            .expect_err("rt on llvmpipe")
            .to_string()
            .contains("rt"));
    }

    #[test]
    fn stamp_appends_tier_to_ra8_summary() {
        let stamped = StampedManifest::new(manifest(), Tier::Rt);
        assert_eq!(stamped.tier(), Tier::Rt);
        let summary = stamped.summary();
        assert!(
            summary.contains("adapter="),
            "keeps the RA8 fields: {summary}"
        );
        assert!(
            summary.contains("scene_digest="),
            "keeps the digest: {summary}"
        );
        assert!(
            summary.ends_with("tier=rt"),
            "stamps the tier last: {summary}"
        );
        assert_eq!(summary, stamp_summary(stamped.manifest(), Tier::Rt));
    }

    #[test]
    fn probed_and_forced_stamp_helpers() {
        let (stamped, selection) =
            StampedManifest::probed(manifest(), &AdapterCaps::reference_rt());
        assert_eq!((stamped.tier(), selection.tier), (Tier::Rt, Tier::Rt));
        assert!(StampedManifest::forced(manifest(), &AdapterCaps::llvmpipe(), Tier::Rt).is_err());
        let base = StampedManifest::forced(manifest(), &AdapterCaps::llvmpipe(), Tier::Base)
            .expect("base");
        assert!(base.summary().ends_with("tier=base"));
    }
}
