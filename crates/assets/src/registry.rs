//! registry: asset identity plus dependency invalidation.
//!
//! The content-addressed helpers (`Guid`, `Handle`, `ManifestRow`,
//! `content_hash`, `sha_hex`, `Registry`) serve the current importer: refs
//! store a handle, never a path, and the id is the content hash.
//!
//! The persistent-identity registry below (`AssetId`, `AssetRecord`,
//! `AssetRegistry`) separates the two concerns: an asset mints its id once
//! and keeps it across reimport, while each import recomputes the blake3
//! content digest of the bytes. The registry maps id to digest plus
//! dependency edges. A digest change reports the transitive dependent closure
//! as the minimal invalidated set, and a batch that names an unknown id or a
//! missing dependency applies nothing. No IO, std plus engine-scene only.

use engine_core::hash_u64s;
#[cfg(feature = "registry")]
use engine_scene::ids::{content_digest, ObjectId};

#[cfg(feature = "registry")]
use std::cmp::Ordering;
use std::collections::BTreeMap;
#[cfg(feature = "registry")]
use std::collections::BTreeSet;
#[cfg(feature = "registry")]
use std::fmt;

/// Stable asset id. Value IS the FNV-1a 64 content hash.
pub type Guid = u64;

/// Opaque ref into [`Registry`]. Refs store this, never paths.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Handle(pub u32);

/// A [`Registry`] lookup failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HandleError {
    /// The handle indexes no imported row.
    Bad(Handle),
}

impl std::fmt::Display for HandleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Bad(handle) => write!(f, "bad handle {}", handle.0),
        }
    }
}

impl std::error::Error for HandleError {}

/// Manifest row: content-addressed id + hex sha + license tag.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ManifestRow {
    /// Content hash; equals [`content_hash`] of the imported bytes.
    pub id: Guid,
    /// Hex of `id` (`sha` per data skill: manifest + SHA per asset).
    pub sha: String,
    /// License tag (e.g. `MIT`). Never a path.
    pub license: String,
}

impl ManifestRow {
    /// Build a row from raw bytes plus a license tag.
    pub fn new(bytes: &[u8], license: &str) -> Self {
        let id = content_hash(bytes);
        Self {
            id,
            sha: sha_hex(id),
            license: license.into(),
        }
    }
    /// True when `bytes` no longer match this row (staleness = content hash).
    pub fn stale(&self, bytes: &[u8]) -> bool {
        content_hash(bytes) != self.id
    }
}

/// Hex sha for a content hash (16 lowercase digits).
pub fn sha_hex(hash: Guid) -> String {
    format!("{hash:016x}")
}

/// FNV-1a 64 content hash (a [`Guid`]): the engine-core hash is this crate's
/// single canon for asset identity; `lib.rs` re-exports it.
pub use engine_core::hash_bytes as content_hash;

/// GUID registry: dedup by content hash. SoA storage, borrowed args, no unwrap.
#[derive(Clone, Debug, Default)]
pub struct Registry {
    /// Guid per handle (index = `Handle.0`).
    ids: Vec<Guid>,
    /// Hex sha per handle.
    shas: Vec<String>,
    /// License tag per handle.
    licenses: Vec<String>,
    /// Content hash -> first handle (dedup).
    index: BTreeMap<Guid, Handle>,
}

impl Registry {
    /// Empty registry.
    pub fn new() -> Self {
        Self::default()
    }
    /// Row count.
    pub fn len(&self) -> usize {
        self.ids.len()
    }
    /// True when no rows imported.
    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }
    /// Import bytes; same content returns the same handle (dedup by hash).
    pub fn import(&mut self, bytes: &[u8], license: &str) -> Handle {
        let id = content_hash(bytes);
        if let Some(h) = self.index.get(&id) {
            return *h;
        }
        let h = Handle(self.ids.len() as u32);
        self.ids.push(id);
        self.shas.push(sha_hex(id));
        self.licenses.push(license.into());
        self.index.insert(id, h);
        h
    }
    /// Row view for a handle; `None` on out-of-range handle.
    pub fn get(&self, h: Handle) -> Option<ManifestRow> {
        let i = h.0 as usize;
        let id = *self.ids.get(i)?;
        Some(ManifestRow {
            id,
            sha: self.shas.get(i)?.clone(),
            license: self.licenses.get(i)?.clone(),
        })
    }
    /// Guid for a handle; `None` on out-of-range handle.
    pub fn guid(&self, h: Handle) -> Option<Guid> {
        self.ids.get(h.0 as usize).copied()
    }
    /// True when `bytes` still match the stored row; `Err` on bad handle.
    /// Test-only: no production caller reads freshness here.
    #[cfg(test)]
    pub fn is_fresh(&self, h: Handle, bytes: &[u8]) -> Result<bool, HandleError> {
        let id = self.ids.get(h.0 as usize).ok_or(HandleError::Bad(h))?;
        Ok(content_hash(bytes) == *id)
    }
    /// Deterministic fold over ids in handle order (construct -> step -> hash).
    pub fn hash(&self) -> Guid {
        hash_u64s(&self.ids)
    }
}

/// Persistent asset identity, minted once and kept across reimport.
///
/// A thin wrapper over [`ObjectId`] from engine-scene so asset references stay
/// valid while content changes underneath. Copying an asset mints a new id;
/// reimporting the same asset keeps it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg(feature = "registry")]
pub struct AssetId(pub ObjectId);

#[cfg(feature = "registry")]
impl AssetId {
    /// Mint a fresh random id.
    pub fn new() -> Self {
        Self(ObjectId::new())
    }

    /// The nil id. Never minted; useful as an explicit empty slot.
    pub const NIL: Self = Self(ObjectId::NIL);

    /// True for the nil id.
    pub fn is_nil(self) -> bool {
        self.0.is_nil()
    }

    /// Parse hyphenated or simple-hex UUID text.
    pub fn parse(text: &str) -> Result<Self, AssetIdParseError> {
        ObjectId::parse(text)
            .map(Self)
            .map_err(|_| AssetIdParseError)
    }
}

#[cfg(feature = "registry")]
impl Default for AssetId {
    fn default() -> Self {
        Self::NIL
    }
}

#[cfg(feature = "registry")]
impl PartialOrd for AssetId {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[cfg(feature = "registry")]
impl Ord for AssetId {
    fn cmp(&self, other: &Self) -> Ordering {
        self.0
            .as_uuid()
            .as_bytes()
            .cmp(other.0.as_uuid().as_bytes())
    }
}

#[cfg(feature = "registry")]
impl fmt::Display for AssetId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[cfg(feature = "registry")]
impl From<ObjectId> for AssetId {
    fn from(id: ObjectId) -> Self {
        Self(id)
    }
}

#[cfg(feature = "registry")]
impl From<AssetId> for ObjectId {
    fn from(id: AssetId) -> Self {
        id.0
    }
}

/// Rejected [`AssetId`] text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg(feature = "registry")]
pub struct AssetIdParseError;

#[cfg(feature = "registry")]
impl fmt::Display for AssetIdParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("invalid asset id")
    }
}

#[cfg(feature = "registry")]
impl std::error::Error for AssetIdParseError {}

/// Stored per-asset state: blake3 hex digest of the last import plus the
/// dependency edges declared by that import. Deps are kept sorted and deduped.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg(feature = "registry")]
pub struct AssetRecord {
    /// `content_digest` of the last imported bytes; versions the content.
    pub digest: String,
    /// Direct dependencies, sorted and deduped.
    pub deps: Vec<AssetId>,
}

/// Single-asset failure: unknown target or missing dependency references.
/// The registry is unchanged when these are returned.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg(feature = "registry")]
pub enum RegistryError {
    /// The target id names no registered asset.
    Unknown(AssetId),
    /// The import names dependencies that are not registered, sorted.
    Missing {
        /// Every referenced id with no registry entry, sorted and deduped.
        missing: Vec<AssetId>,
    },
}

#[cfg(feature = "registry")]
impl fmt::Display for RegistryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unknown(id) => write!(f, "unknown asset {id}"),
            Self::Missing { missing } => {
                let ids: Vec<String> = missing.iter().map(|id| id.to_string()).collect();
                write!(f, "missing reference: {}", ids.join(", "))
            }
        }
    }
}

#[cfg(feature = "registry")]
impl std::error::Error for RegistryError {}

/// One reimport step inside [`AssetRegistry::apply_batch`].
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg(feature = "registry")]
pub struct BatchOp {
    /// `None` mints a fresh id; `Some` reimports that registered asset.
    pub target: Option<AssetId>,
    /// Raw content bytes; the digest is recomputed from these.
    pub content: Vec<u8>,
    /// Declared dependencies; every entry must already be registered.
    pub deps: Vec<AssetId>,
}

/// Successful batch result. `invalidated` is the transitive dependent closure
/// of the changed ids, minus the changed ids themselves.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg(feature = "registry")]
pub struct BatchOutcome {
    /// Freshly minted ids, sorted.
    pub minted: Vec<AssetId>,
    /// Reimported ids (changed or not), sorted.
    pub updated: Vec<AssetId>,
    /// Dependents needing revalidation, sorted.
    pub invalidated: Vec<AssetId>,
}

/// Failed batch diagnostic. Nothing was applied; fix the listed ids and retry.
/// Both sets are sorted and deduped, so the conflict set is minimal.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg(feature = "registry")]
pub struct BatchConflict {
    /// Reimport targets with no registry entry.
    pub unknown: Vec<AssetId>,
    /// Dependency references with no registry entry.
    pub missing: Vec<AssetId>,
}

#[cfg(feature = "registry")]
impl fmt::Display for BatchConflict {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let list = |ids: &[AssetId]| {
            ids.iter()
                .map(|id| id.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        };
        match (self.unknown.as_slice(), self.missing.as_slice()) {
            ([], []) => f.write_str("empty conflict"),
            (unknown, []) => write!(f, "unknown asset: {}", list(unknown)),
            ([], missing) => write!(f, "missing reference: {}", list(missing)),
            (unknown, missing) => write!(
                f,
                "unknown asset: {}; missing reference: {}",
                list(unknown),
                list(missing)
            ),
        }
    }
}

#[cfg(feature = "registry")]
impl std::error::Error for BatchConflict {}

/// Single-reimport result. `kept` always equals the requested id.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg(feature = "registry")]
pub struct Reimport {
    /// The reimported id (identity is kept by design).
    pub kept: AssetId,
    /// True when the digest or the deps differed from the stored record.
    pub changed: bool,
    /// Transitive dependents needing revalidation, sorted.
    pub invalidated: Vec<AssetId>,
}

/// Persistent-identity asset registry: id maps to digest plus deps.
///
/// Identity comes from [`AssetId`]; content versioning comes from the blake3
/// [`content_digest`] hex. Iteration order is id-sorted, so repeated builds
/// over the same inputs report the same sets.
#[derive(Clone, Debug, Default)]
#[cfg(feature = "registry")]
pub struct AssetRegistry {
    entries: BTreeMap<AssetId, AssetRecord>,
}

#[cfg(feature = "registry")]
impl AssetRegistry {
    /// Empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Asset count.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True when no assets are registered.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// True when `id` names a registered asset.
    pub fn contains(&self, id: AssetId) -> bool {
        self.entries.contains_key(&id)
    }

    /// Record view for an id; `None` on unknown id.
    pub fn get(&self, id: AssetId) -> Option<&AssetRecord> {
        self.entries.get(&id)
    }

    /// Stored digest for an id; `None` on unknown id.
    pub fn digest(&self, id: AssetId) -> Option<&str> {
        self.entries.get(&id).map(|record| record.digest.as_str())
    }

    /// Direct dependents of `id` (assets listing it in `deps`), sorted.
    pub fn dependents_of(&self, id: AssetId) -> Vec<AssetId> {
        let mut out: BTreeSet<AssetId> = BTreeSet::new();
        for (candidate, record) in &self.entries {
            if record.deps.contains(&id) {
                out.insert(*candidate);
            }
        }
        out.into_iter().collect()
    }

    /// Register fresh content. Mints one id; every dep must already exist.
    /// On missing deps nothing is stored and the missing set is reported.
    pub fn register(&mut self, content: &[u8], deps: &[AssetId]) -> Result<AssetId, RegistryError> {
        let missing = missing_of(&self.entries, deps);
        if !missing.is_empty() {
            return Err(RegistryError::Missing { missing });
        }
        let id = AssetId::new();
        self.entries.insert(
            id,
            AssetRecord {
                digest: content_digest(content),
                deps: sorted_unique(deps),
            },
        );
        Ok(id)
    }

    /// Reimport new bytes for a known id. The id is kept; the digest and deps
    /// are recomputed. Unknown ids and missing deps fail without mutation.
    /// A content change reports the transitive dependent closure.
    pub fn reimport(
        &mut self,
        id: AssetId,
        content: &[u8],
        deps: &[AssetId],
    ) -> Result<Reimport, RegistryError> {
        if !self.entries.contains_key(&id) {
            return Err(RegistryError::Unknown(id));
        }
        let missing = missing_of(&self.entries, deps);
        if !missing.is_empty() {
            return Err(RegistryError::Missing { missing });
        }
        let digest = content_digest(content);
        let deps = sorted_unique(deps);
        let record = self.entries.get_mut(&id).expect("presence checked above");
        let changed = record.digest != digest || record.deps != deps;
        if changed {
            record.digest = digest;
            record.deps = deps;
        }
        let mut seeds = BTreeSet::new();
        if changed {
            seeds.insert(id);
        }
        Ok(Reimport {
            kept: id,
            changed,
            invalidated: self.dependents_closure(&seeds),
        })
    }

    /// Atomically apply a batch of mints and reimports. Validation runs over
    /// the whole batch first: any unknown target or missing dep fails the
    /// entire batch with the minimal conflict set and applies nothing.
    pub fn apply_batch(&mut self, ops: &[BatchOp]) -> Result<BatchOutcome, BatchConflict> {
        let mut unknown: BTreeSet<AssetId> = BTreeSet::new();
        let mut missing: BTreeSet<AssetId> = BTreeSet::new();
        for op in ops {
            if let Some(id) = op.target {
                if !self.entries.contains_key(&id) {
                    unknown.insert(id);
                }
            }
            for dep in &op.deps {
                if !self.entries.contains_key(dep) {
                    missing.insert(*dep);
                }
            }
        }
        if !unknown.is_empty() || !missing.is_empty() {
            return Err(BatchConflict {
                unknown: unknown.into_iter().collect(),
                missing: missing.into_iter().collect(),
            });
        }
        let mut minted = Vec::new();
        let mut updated = Vec::new();
        let mut changed: BTreeSet<AssetId> = BTreeSet::new();
        for op in ops {
            let digest = content_digest(&op.content);
            let deps = sorted_unique(&op.deps);
            match op.target {
                None => {
                    let id = AssetId::new();
                    self.entries.insert(id, AssetRecord { digest, deps });
                    minted.push(id);
                }
                Some(id) => {
                    let record = self.entries.get_mut(&id).expect("target validated present");
                    if record.digest != digest || record.deps != deps {
                        record.digest = digest;
                        record.deps = deps;
                        changed.insert(id);
                    }
                    updated.push(id);
                }
            }
        }
        minted.sort();
        updated.sort();
        Ok(BatchOutcome {
            minted,
            updated,
            invalidated: self.dependents_closure(&changed),
        })
    }

    /// Transitive dependents of `changed`, minus the changed ids themselves.
    /// `seen` starts full so dependency cycles terminate.
    fn dependents_closure(&self, changed: &BTreeSet<AssetId>) -> Vec<AssetId> {
        let mut reverse: BTreeMap<AssetId, Vec<AssetId>> = BTreeMap::new();
        for (id, record) in &self.entries {
            for dep in &record.deps {
                reverse.entry(*dep).or_default().push(*id);
            }
        }
        let mut seen = changed.clone();
        let mut out: BTreeSet<AssetId> = BTreeSet::new();
        let mut queue: Vec<AssetId> = changed.iter().copied().collect();
        while let Some(current) = queue.pop() {
            if let Some(next) = reverse.get(&current) {
                for id in next {
                    if seen.insert(*id) {
                        out.insert(*id);
                        queue.push(*id);
                    }
                }
            }
        }
        out.into_iter().collect()
    }
}

/// Sorted, deduped copy of a dep list for deterministic storage.
#[cfg(feature = "registry")]
fn sorted_unique(deps: &[AssetId]) -> Vec<AssetId> {
    deps.iter()
        .copied()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

/// Sorted, deduped subset of `deps` with no registry entry.
#[cfg(feature = "registry")]
fn missing_of(entries: &BTreeMap<AssetId, AssetRecord>, deps: &[AssetId]) -> Vec<AssetId> {
    let mut out: BTreeSet<AssetId> = BTreeSet::new();
    for dep in deps {
        if !entries.contains_key(dep) {
            out.insert(*dep);
        }
    }
    out.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    #[test]
    fn guid_is_content_hash() {
        let bytes = b"chaoyang-slate";
        let row = ManifestRow::new(bytes, "MIT");
        assert_eq!(row.id, content_hash(bytes));
        assert_eq!(row.sha, sha_hex(row.id));
        assert!(!row.stale(bytes));
        assert!(row.stale(b"chaoyang-slate!"));
    }

    #[test]
    fn import_x2_same_hash() {
        let mut r = Registry::new();
        let a = r.import(b"mesh-bytes-1", "MIT");
        let b = r.import(b"mesh-bytes-1", "MIT");
        assert_eq!(a, b);
        assert_eq!(r.len(), 1);
        assert_eq!(r.guid(a), r.guid(b));
        assert_eq!(r.hash(), r.hash());
    }

    #[test]
    fn different_bytes_different_guid() {
        let mut r = Registry::new();
        let a = r.import(b"bytes-a", "MIT");
        let b = r.import(b"bytes-b", "MIT");
        assert_ne!(a, b);
        assert_ne!(r.guid(a), r.guid(b));
        assert_eq!(r.len(), 2);
    }

    #[test]
    fn refs_store_handle_never_path() {
        let mut r = Registry::new();
        let h = r.import(b"ref-bytes", "Apache-2.0");
        let row = r.get(h).expect("handle resolves");
        assert_eq!(row.license, "Apache-2.0");
        assert!(r.get(Handle(999)).is_none());
        assert!(r.guid(Handle(999)).is_none());
        assert!(r.is_fresh(Handle(999), b"ref-bytes").is_err());
        assert_eq!(r.is_fresh(h, b"ref-bytes"), Ok(true));
        assert_eq!(r.is_fresh(h, b"other"), Ok(false));
    }

    #[test]
    fn bad_handle_error_is_typed_and_printable() {
        let r = Registry::new();
        let err = r.is_fresh(Handle(7), b"x").unwrap_err();
        assert_eq!(err, HandleError::Bad(Handle(7)));
        assert_eq!(err.to_string(), "bad handle 7");
    }

    #[test]
    #[ignore = "timing: run `node tools/proof.mjs --perf` (release, one thread)"]
    fn reload_under_100ms() {
        let mut r = Registry::new();
        for i in 0..256u32 {
            let bytes = format!("asset-payload-{i:04}-pad-to-grow-bytes");
            r.import(bytes.as_bytes(), "MIT");
        }
        let t = Instant::now();
        for i in 0..256u32 {
            let bytes = format!("asset-payload-{i:04}-pad-to-grow-bytes");
            assert_eq!(r.is_fresh(Handle(i), bytes.as_bytes()), Ok(true));
        }
        assert!(t.elapsed().as_millis() < 100, "reload {:?}", t.elapsed());
    }

    #[test]
    fn deterministic_hash_x2() {
        let mut a = Registry::new();
        let mut b = Registry::new();
        for payload in [b"one".as_slice(), b"two".as_slice(), b"three".as_slice()] {
            a.import(payload, "MIT");
            b.import(payload, "MIT");
        }
        assert_eq!(a.hash(), b.hash());
        assert_eq!(a.get(Handle(1)), b.get(Handle(1)));
    }
}

#[cfg(all(test, feature = "registry"))]
mod asset_tests {
    use super::*;

    fn sorted(mut ids: Vec<AssetId>) -> Vec<AssetId> {
        ids.sort();
        ids
    }

    #[test]
    fn reimport_keeps_id_and_reversions_digest() {
        let mut registry = AssetRegistry::new();
        let id = registry.register(b"mesh v1", &[]).unwrap();
        assert_eq!(registry.digest(id).unwrap(), content_digest(b"mesh v1"));

        // Same bytes: id kept, nothing changed, nobody invalidated.
        let same = registry.reimport(id, b"mesh v1", &[]).unwrap();
        assert_eq!(same.kept, id);
        assert!(!same.changed);
        assert!(same.invalidated.is_empty());

        // New bytes: id kept, digest recomputed to the blake3 hex.
        let next = registry.reimport(id, b"mesh v2", &[]).unwrap();
        assert_eq!(next.kept, id);
        assert!(next.changed);
        assert_eq!(registry.digest(id).unwrap(), content_digest(b"mesh v2"));
        assert_eq!(registry.len(), 1);
    }

    #[test]
    fn content_change_invalidates_transitive_dependents() {
        let mut registry = AssetRegistry::new();
        let a = registry.register(b"a1", &[]).unwrap();
        let b = registry.register(b"b1", &[a]).unwrap();
        let c = registry.register(b"c1", &[b]).unwrap();
        let unrelated = registry.register(b"u", &[]).unwrap();

        // No-op reimport invalidates nobody.
        let noop = registry.reimport(a, b"a1", &[]).unwrap();
        assert!(!noop.changed);
        assert!(noop.invalidated.is_empty());

        // Changed content invalidates the transitive closure only.
        let changed = registry.reimport(a, b"a2", &[]).unwrap();
        assert!(changed.changed);
        assert_eq!(changed.invalidated, sorted(vec![b, c]));
        assert!(!changed.invalidated.contains(&a));
        assert!(!changed.invalidated.contains(&unrelated));

        // Direct-dependents view agrees on the first edge.
        assert_eq!(registry.dependents_of(a), vec![b]);
        assert_eq!(registry.dependents_of(b), vec![c]);
        assert!(registry.dependents_of(c).is_empty());
    }

    #[test]
    fn missing_reference_names_culprit_and_stores_nothing() {
        let mut registry = AssetRegistry::new();
        let ghost = AssetId::new();

        let err = registry.register(b"orphan", &[ghost]).unwrap_err();
        assert_eq!(
            err,
            RegistryError::Missing {
                missing: vec![ghost]
            }
        );
        assert!(format!("{err}").contains(&ghost.to_string()));
        assert!(registry.is_empty());

        // Unknown reimport target is diagnosed the same way.
        let unknown_err = registry.reimport(ghost, b"x", &[]).unwrap_err();
        assert_eq!(unknown_err, RegistryError::Unknown(ghost));
        assert!(format!("{unknown_err}").contains(&ghost.to_string()));

        // Failed reimport leaves the stored record untouched.
        let ok = registry.register(b"kept", &[]).unwrap();
        let before = registry.get(ok).unwrap().clone();
        let dep_err = registry.reimport(ok, b"changed", &[ghost]).unwrap_err();
        assert!(format!("{dep_err}").contains(&ghost.to_string()));
        assert_eq!(registry.get(ok).unwrap(), &before);
    }

    #[test]
    fn failed_batch_applies_nothing() {
        let mut registry = AssetRegistry::new();
        let a = registry.register(b"a1", &[]).unwrap();
        let b = registry.register(b"b1", &[a]).unwrap();
        let before_b = registry.get(b).unwrap().clone();
        let ghost = AssetId::new();

        // One bad op poisons the batch: minimal conflict, zero writes.
        let ops = vec![
            BatchOp {
                target: Some(b),
                content: b"b2".to_vec(),
                deps: vec![],
            },
            BatchOp {
                target: None,
                content: b"new".to_vec(),
                deps: vec![ghost],
            },
        ];
        let conflict = registry.apply_batch(&ops).unwrap_err();
        assert_eq!(conflict.unknown, vec![]);
        assert_eq!(conflict.missing, vec![ghost]);
        assert!(format!("{conflict}").contains(&ghost.to_string()));
        assert_eq!(registry.get(b).unwrap(), &before_b);
        assert_eq!(registry.len(), 2);

        // Clean batch applies fully and reports the dependent closure.
        let ops = vec![BatchOp {
            target: Some(a),
            content: b"a2".to_vec(),
            deps: vec![],
        }];
        let outcome = registry.apply_batch(&ops).unwrap();
        assert_eq!(outcome.updated, vec![a]);
        assert!(outcome.minted.is_empty());
        assert_eq!(outcome.invalidated, vec![b]);
    }

    #[test]
    fn asset_id_parse_roundtrip_and_nil() {
        let id = AssetId::new();
        assert_eq!(AssetId::parse(&id.to_string()).unwrap(), id);
        assert!(!id.is_nil());
        assert!(AssetId::NIL.is_nil());
        assert!(AssetId::parse("not-an-id").is_err());
    }
}
