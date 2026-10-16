//! cooker: deterministic cook cache over the persistent asset registry.
//!
//! Same content digest cooks once; a changed digest recooks and reports the
//! transitive dependents from the registry. A failed job keeps the last-good
//! cached payload and leaves the registry record untouched, so callers fall
//! back to the previous artifact. Unknown targets and missing dependencies
//! store nothing and never run the cook closure. Iteration is id-sorted via
//! `BTreeMap`/`BTreeSet`, so repeated builds report the same sets.

/// Sever-cook templates (body segments plus stump caps) compiled from
/// `sever_cook.rs`: the cut presentation reads interior and cap rings from
/// the canonical payload below, so the templates ship with the cook cache.
#[path = "sever_cook.rs"]
pub mod sever_cook;

/// Canonical cache payload for a sever cook: digest plus a per-segment
/// census (name bytes plus interior/cap face counts). Little-endian
/// fixed-width integers only, segments already name-sorted by the cook, so
/// the same rig encodes byte-identical payloads on every PC.
pub fn sever_cache_payload(output: &sever_cook::SeverCookOutput) -> Vec<u8> {
    let mut bytes: Vec<u8> = Vec::with_capacity(16 + output.segments.len() * 96);
    bytes.extend_from_slice(&output.digest.to_le_bytes());
    let count = u64::try_from(output.segments.len()).unwrap_or(u64::MAX);
    bytes.extend_from_slice(&count.to_le_bytes());
    for segment in &output.segments {
        let name = segment.name.as_bytes();
        let len = u64::try_from(name.len()).unwrap_or(u64::MAX);
        bytes.extend_from_slice(&len.to_le_bytes());
        bytes.extend_from_slice(name);
        let interior = u64::try_from(segment.interior_tris.len()).unwrap_or(u64::MAX);
        bytes.extend_from_slice(&interior.to_le_bytes());
        let cap = u64::try_from(segment.cap_tris.len()).unwrap_or(u64::MAX);
        bytes.extend_from_slice(&cap.to_le_bytes());
    }
    bytes
}

#[cfg(feature = "registry")]
use std::collections::{BTreeMap, BTreeSet};
#[cfg(feature = "registry")]
use std::fmt;

#[cfg(feature = "registry")]
use super::registry::{AssetId, AssetRegistry, RegistryError};
#[cfg(feature = "registry")]
use engine_scene::ids::content_digest;

/// Cook failure: unknown target, missing dependency refs, or a failed job.
///
/// Mirrors [`RegistryError`] shapes for the first two variants so callers can
/// translate them one-to-one. `Failed` carries the cook closure's message;
/// the registry record and the last-good cache entry are unchanged then.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg(feature = "registry")]
pub enum CookError {
    /// The target id names no registered asset.
    Unknown(AssetId),
    /// Dependencies with no registry entry, sorted and deduped.
    Missing {
        /// Every referenced id with no registry entry.
        missing: Vec<AssetId>,
    },
    /// The cook closure refused the content; nothing was stored.
    Failed(String),
}

#[cfg(feature = "registry")]
impl fmt::Display for CookError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unknown(id) => write!(f, "unknown asset {id}"),
            Self::Missing { missing } => {
                let ids: Vec<String> = missing.iter().map(|id| id.to_string()).collect();
                write!(f, "missing reference: {}", ids.join(", "))
            }
            Self::Failed(detail) => write!(f, "cook failed: {detail}"),
        }
    }
}

#[cfg(feature = "registry")]
impl std::error::Error for CookError {}

/// Refusal a cook closure returns for content it cannot cook; the message
/// reaches callers as [`CookError::Failed`].
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg(feature = "registry")]
pub struct CookFailure {
    /// Human-readable reason; the registry and cache stay unchanged.
    pub message: String,
}

#[cfg(feature = "registry")]
impl CookFailure {
    /// Failure carrying `message`.
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

#[cfg(feature = "registry")]
impl fmt::Display for CookFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

#[cfg(feature = "registry")]
impl std::error::Error for CookFailure {}

/// Last-good cooked artifact for one asset: digest of the source content that
/// produced it plus the cooked payload bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg(feature = "registry")]
pub struct Cached {
    /// `content_digest` of the source bytes that produced `payload`.
    pub digest: String,
    /// Cooked artifact bytes; served while a recook fails.
    pub payload: Vec<u8>,
}

/// Single cook result. `fresh` is true exactly when `cook_fn` ran.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg(feature = "registry")]
pub struct CookOutcome {
    /// The cooked asset id (minted by `cook`, kept by `recook`).
    pub id: AssetId,
    /// True when the cook closure ran for this call.
    pub fresh: bool,
    /// Transitive dependents needing revalidation, sorted.
    pub invalidated: Vec<AssetId>,
}

/// Deterministic cook cache: registry owns identity plus digests, `cache`
/// owns last-good payloads, `cooks` counts cook closure runs (including
/// failed jobs; cache hits, unknown targets and missing deps never run it).
#[derive(Clone, Debug, Default)]
#[cfg(feature = "registry")]
pub struct Cooker {
    /// Persistent identities, digests and dependency edges.
    pub registry: AssetRegistry,
    /// Last-good payload per asset, in id order.
    pub cache: BTreeMap<AssetId, Cached>,
    /// Number of times the cook closure has run.
    pub cooks: u64,
}

#[cfg(feature = "registry")]
impl Cooker {
    /// Empty cooker: no assets, no cached payloads, zero cooks.
    pub fn new() -> Self {
        Self::default()
    }

    /// Asset count (registry entries).
    pub fn len(&self) -> usize {
        self.registry.len()
    }

    /// True when no assets are registered.
    pub fn is_empty(&self) -> bool {
        self.registry.is_empty()
    }

    /// Cook fresh content: always runs `cook_fn` once, then mints one id.
    /// Missing deps fail before `cook_fn` runs and store nothing; a failed
    /// job stores nothing either. A new id has no dependents yet.
    pub fn cook(
        &mut self,
        content: &[u8],
        deps: &[AssetId],
        cook_fn: impl FnOnce(&[u8]) -> Result<Vec<u8>, CookFailure>,
    ) -> Result<CookOutcome, CookError> {
        let missing = missing_of(&self.registry, deps);
        if !missing.is_empty() {
            return Err(CookError::Missing { missing });
        }
        let digest = content_digest(content);
        // Run the job before touching the registry so failure stores nothing.
        let payload = run_job(&mut self.cooks, content, cook_fn)?;
        let id = self
            .registry
            .register(content, deps)
            .map_err(map_registry_error)?;
        // Digest recomputed by register matches the one above; the cache key
        // is the freshly minted id, so insertion cannot collide.
        self.cache.insert(id, Cached { digest, payload });
        Ok(CookOutcome {
            id,
            fresh: true,
            invalidated: Vec::new(),
        })
    }

    /// Recook a known id: cache hit (same digest plus cached payload) skips
    /// `cook_fn` and reports `fresh: false`; otherwise run it exactly once.
    /// The job runs before `reimport` so failure preserves the last-good
    /// cache entry and the registry record. Success refreshes both and
    /// reports the transitive dependent closure.
    pub fn recook(
        &mut self,
        id: AssetId,
        content: &[u8],
        deps: &[AssetId],
        cook_fn: impl FnOnce(&[u8]) -> Result<Vec<u8>, CookFailure>,
    ) -> Result<CookOutcome, CookError> {
        if !self.registry.contains(id) {
            return Err(CookError::Unknown(id));
        }
        let missing = missing_of(&self.registry, deps);
        if !missing.is_empty() {
            return Err(CookError::Missing { missing });
        }
        let digest = content_digest(content);
        let hit = self.registry.digest(id) == Some(digest.as_str())
            && self.cache.get(&id).map(|entry| entry.digest.as_str()) == Some(digest.as_str());
        if hit {
            // No cook needed; still reimport to sync dep edges (a deps-only
            // change reports dependents without rerunning the job).
            let reimported = self
                .registry
                .reimport(id, content, deps)
                .map_err(map_registry_error)?;
            return Ok(CookOutcome {
                id,
                fresh: false,
                invalidated: reimported.invalidated,
            });
        }
        // Miss: run the job before mutating registry or cache.
        let payload = run_job(&mut self.cooks, content, cook_fn)?;
        let reimported = self
            .registry
            .reimport(id, content, deps)
            .map_err(map_registry_error)?;
        self.cache.insert(id, Cached { digest, payload });
        Ok(CookOutcome {
            id,
            fresh: true,
            invalidated: reimported.invalidated,
        })
    }
}

/// Run the cook closure once and count the run, including failed jobs.
/// Cache hits and validation errors never reach here, so `cooks` counts jobs.
#[cfg(feature = "registry")]
fn run_job(
    cooks: &mut u64,
    content: &[u8],
    cook_fn: impl FnOnce(&[u8]) -> Result<Vec<u8>, CookFailure>,
) -> Result<Vec<u8>, CookError> {
    let result = cook_fn(content);
    *cooks = cooks.saturating_add(1);
    result.map_err(|failure| CookError::Failed(failure.message))
}

/// Translate registry validation errors into the matching cook error.
#[cfg(feature = "registry")]
fn map_registry_error(err: RegistryError) -> CookError {
    match err {
        RegistryError::Unknown(id) => CookError::Unknown(id),
        RegistryError::Missing { missing } => CookError::Missing { missing },
    }
}

/// Sorted, deduped subset of `deps` with no registry entry.
#[cfg(feature = "registry")]
fn missing_of(registry: &AssetRegistry, deps: &[AssetId]) -> Vec<AssetId> {
    let mut out: BTreeSet<AssetId> = BTreeSet::new();
    for dep in deps {
        if !registry.contains(*dep) {
            out.insert(*dep);
        }
    }
    out.into_iter().collect()
}

#[cfg(all(test, feature = "registry"))]
mod tests {
    use super::*;

    fn cook_ok(content: &[u8]) -> Result<Vec<u8>, CookFailure> {
        Ok(content.to_vec())
    }

    #[test]
    fn same_digest_cooks_once() {
        let mut cooker = Cooker::new();
        let first = cooker.cook(b"mesh v1", &[], cook_ok).unwrap();
        assert!(first.fresh);
        assert_eq!(cooker.cooks, 1);

        // Same bytes for the same id: cache hit, closure must not run.
        let second = cooker
            .recook(first.id, b"mesh v1", &[], |_| {
                panic!("cache hit must not run the cook closure")
            })
            .unwrap();
        assert_eq!(second.id, first.id);
        assert!(!second.fresh);
        assert!(second.invalidated.is_empty());
        assert_eq!(cooker.cooks, 1);
        assert_eq!(
            cooker.cache.get(&first.id).unwrap().payload,
            b"mesh v1".to_vec()
        );
    }

    #[test]
    fn empty_cache_recooks_same_digest() {
        let mut cooker = Cooker::new();
        let id = cooker.cook(b"mesh v1", &[], cook_ok).unwrap().id;
        assert_eq!(cooker.cooks, 1);
        // Evicted payload: same digest still reruns the job to repopulate.
        cooker.cache.remove(&id);
        let repopulated = cooker.recook(id, b"mesh v1", &[], cook_ok).unwrap();
        assert!(repopulated.fresh);
        assert_eq!(cooker.cooks, 2);
        assert_eq!(cooker.cache.get(&id).unwrap().payload, b"mesh v1".to_vec());
    }

    #[test]
    fn changed_digest_recooks_and_reports_dependents() {
        let mut cooker = Cooker::new();
        let a = cooker.cook(b"a1", &[], cook_ok).unwrap().id;
        let b = cooker.cook(b"b1", &[a], cook_ok).unwrap().id;
        let c = cooker.cook(b"c1", &[b], cook_ok).unwrap().id;
        assert_eq!(cooker.cooks, 3);

        let out = cooker.recook(a, b"a2", &[], cook_ok).unwrap();
        assert_eq!(out.id, a);
        assert!(out.fresh);
        // Transitive closure only: b and c, sorted, without a itself.
        assert_eq!(
            out.invalidated,
            vec![b, c]
                .into_iter()
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect::<Vec<_>>()
        );
        assert_eq!(cooker.cooks, 4);
        assert_eq!(cooker.cache.get(&a).unwrap().payload, b"a2".to_vec());
        assert_eq!(
            cooker.registry.digest(a).unwrap(),
            content_digest(b"a2").as_str()
        );
    }

    #[test]
    fn failed_job_preserves_last_good() {
        let mut cooker = Cooker::new();
        let id = cooker
            .cook(b"good", &[], |_| Ok(b"cooked-good".to_vec()))
            .unwrap()
            .id;
        let digest_before = cooker.registry.digest(id).unwrap().to_string();
        let payload_before = cooker.cache.get(&id).unwrap().payload.clone();
        let cooks_before = cooker.cooks;

        let err = cooker
            .recook(id, b"bad", &[], |_| Err(CookFailure::new("oven exploded")))
            .unwrap_err();
        assert_eq!(err, CookError::Failed("oven exploded".to_string()));
        assert!(format!("{err}").contains("oven exploded"));
        // Last-good payload and registry digest are untouched.
        assert_eq!(cooker.cache.get(&id).unwrap().payload, payload_before);
        assert_eq!(cooker.registry.digest(id).unwrap(), digest_before);
        // The failed run still counts as a cook closure run.
        assert_eq!(cooker.cooks, cooks_before + 1);

        // A later good job for the same bytes succeeds and refreshes both.
        let retry = cooker.recook(id, b"bad", &[], cook_ok).unwrap();
        assert!(retry.fresh);
        assert_eq!(cooker.cache.get(&id).unwrap().payload, b"bad".to_vec());
        assert_eq!(
            cooker.registry.digest(id).unwrap(),
            content_digest(b"bad").as_str()
        );
    }

    #[test]
    fn missing_dep_stores_nothing_and_runs_nothing() {
        let mut cooker = Cooker::new();
        let ghost = AssetId::new();

        let err = cooker
            .cook(b"orphan", &[ghost], |_| {
                panic!("missing dep must not run the cook closure")
            })
            .unwrap_err();
        assert_eq!(
            err,
            CookError::Missing {
                missing: vec![ghost]
            }
        );
        assert!(cooker.is_empty());
        assert!(cooker.cache.is_empty());
        assert_eq!(cooker.cooks, 0);

        // Recook with a missing dep also runs nothing and keeps last-good.
        let kept = cooker.cook(b"kept", &[], cook_ok).unwrap().id;
        let cooks_after_mint = cooker.cooks;
        let digest_before = cooker.registry.digest(kept).unwrap().to_string();
        let err = cooker
            .recook(kept, b"changed", &[ghost], |_| {
                panic!("missing dep must not run the cook closure")
            })
            .unwrap_err();
        assert!(matches!(err, CookError::Missing { .. }));
        assert_eq!(cooker.cache.get(&kept).unwrap().payload, b"kept".to_vec());
        assert_eq!(cooker.registry.digest(kept).unwrap(), digest_before);
        assert_eq!(cooker.cooks, cooks_after_mint);
        assert_eq!(cooker.len(), 1);
    }

    #[test]
    fn unknown_recook_runs_nothing() {
        let mut cooker = Cooker::new();
        let ghost = AssetId::new();
        let err = cooker
            .recook(ghost, b"x", &[], |_| {
                panic!("unknown target must not run the cook closure")
            })
            .unwrap_err();
        assert_eq!(err, CookError::Unknown(ghost));
        assert!(format!("{err}").contains(&ghost.to_string()));
        assert_eq!(cooker.cooks, 0);
        assert!(cooker.is_empty());
    }
}
