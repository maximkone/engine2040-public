//! museum: version registry plus fixture template for format releases.
//!
//! Owner: `engine-assets`; consumer: the proof gate, which starts from the
//! seed corpus, records each fixture migration test, then calls `prove`.
//! Units: format versions as `u32`, fixture bytes, path chars. Entries iterate
//! key-sorted, so the same declarations always give the same receipt digest.
//! Every refusal is typed and names its culprit versions; a declared version
//! without its fixture (or without its recorded migration) fails `prove` by
//! construction, never silently. Offline gate only: never in the tick.

use std::collections::BTreeMap;
use std::fmt;

use super::registry::content_hash;

/// Most declared versions held; bounds gate-time scans.
pub const MAX_VERSIONS: usize = 32;
/// Smallest legal format version; zero is refused, never stored.
pub const MIN_VERSION: u32 = 1;
/// Largest legal format version; bounds template path rendering.
pub const MAX_VERSION: u32 = 1024;
/// Longest fixture path in chars; longer paths are refused, never cut.
pub const MAX_PATH_CHARS: usize = 128;
/// Largest fixture payload in bytes; bounds gate-time digest work.
pub const MAX_FIXTURE_BYTES: usize = 16 * 1024 * 1024;
/// Upper bound on any refusal text; culprit lists name the first few.
pub const MAX_DIAGNOSTIC_CHARS: usize = 256;
/// Prefix stamped on template payloads; attaching it is refused so a
/// template alone can never satisfy the gate.
const TEMPLATE_MARKER: &[u8] = b"E204TEMPLATE:";
/// Most culprits named in one refusal before an "and N more" tail.
const MAX_NAMED_CULPRITS: usize = 3;

/// Versioned format tracked by the museum. Discriminants are stable wire
/// numbers; do not renumber.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(u8)]
pub enum FormatKind {
    /// Input-tape envelope plus fixture version.
    Tape = 1,
    /// Scene save document.
    Scene = 2,
    /// Content bundle of cooked assets.
    Bundle = 3,
}

impl FormatKind {
    /// Lowercase kind name for paths, test names and diagnostics.
    pub fn label(self) -> &'static str {
        match self {
            Self::Tape => "tape",
            Self::Scene => "scene",
            Self::Bundle => "bundle",
        }
    }

    /// Canonical fixture file extension for this kind.
    pub fn extension(self) -> &'static str {
        match self {
            Self::Tape | Self::Bundle => "bin",
            Self::Scene => "json",
        }
    }
}

impl fmt::Display for FormatKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

/// One declared format version: the registry key. Sorts kind-major, so
/// receipts iterate tape, then scene, then bundle, ascending in version.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct VersionKey {
    /// Which versioned format this key names.
    pub kind: FormatKind,
    /// Declared format version, always within `MIN_VERSION..=MAX_VERSION`.
    pub version: u32,
}

impl fmt::Display for VersionKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} v{}", self.kind.label(), self.version)
    }
}

/// Canonical fixture path for a version: `fixtures/<kind>-v<version>.<ext>`.
pub fn canonical_path(kind: FormatKind, version: u32) -> String {
    format!(
        "fixtures/{}-v{}.{}",
        kind.label(),
        version,
        kind.extension()
    )
}

/// Canonical migration-test name for a version, so the template and the gate
/// agree on what must exist before a release.
pub fn template_test_name(kind: FormatKind, version: u32) -> String {
    format!(
        "museum_{}_v{version}_loads_migrates_and_replays",
        kind.label()
    )
}

/// Validated fixture payload: a bounded path plus non-empty, bounded bytes.
/// Template-marker bytes are refused here, so a template alone is never a
/// fixture; record real bytes, then record the migration test.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FixtureSpec {
    /// Fixture file path, charset-checked and length-bounded.
    pub path: String,
    /// Fixture bytes: non-empty, bounded, never template output.
    pub bytes: Vec<u8>,
}

impl FixtureSpec {
    /// Validate borrowed input and store owned copies. Every invalid shape
    /// is a static reason; input bytes and paths are never echoed.
    pub fn new(path: &str, bytes: &[u8]) -> Result<Self, MuseumError> {
        if path.is_empty() {
            return Err(MuseumError::Invalid("fixture path is empty"));
        }
        if path.chars().count() > MAX_PATH_CHARS {
            return Err(MuseumError::Invalid("fixture path is too long"));
        }
        let legal = path
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'/' | b'.' | b'-' | b'_'));
        if !legal {
            return Err(MuseumError::Invalid("fixture path holds illegal chars"));
        }
        if bytes.is_empty() {
            return Err(MuseumError::Invalid("fixture bytes are empty"));
        }
        if bytes.len() > MAX_FIXTURE_BYTES {
            return Err(MuseumError::Invalid(
                "fixture bytes exceed the museum bound",
            ));
        }
        if bytes.starts_with(TEMPLATE_MARKER) {
            return Err(MuseumError::Invalid("template bytes are not a fixture"));
        }
        Ok(Self {
            path: path.into(),
            bytes: bytes.into(),
        })
    }
}

/// Registration-time failure: unknown or duplicate versions, invalid input,
/// or a full registry. The registry is unchanged when these are returned.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MuseumError {
    /// The version was never declared.
    Unknown(VersionKey),
    /// The version is already declared.
    Duplicate(VersionKey),
    /// Invalid input; the static reason names the bound that was crossed.
    Invalid(&'static str),
    /// The registry already holds `limit` versions.
    TooMany {
        /// The capacity that was hit.
        limit: usize,
    },
}

impl fmt::Display for MuseumError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unknown(key) => write!(f, "unknown museum version {key}"),
            Self::Duplicate(key) => write!(f, "duplicate museum version {key}"),
            Self::Invalid(reason) => write!(f, "invalid museum entry: {reason}"),
            Self::TooMany { limit } => {
                write!(f, "museum holds too many versions (limit {limit})")
            }
        }
    }
}

impl std::error::Error for MuseumError {}

/// Gate-time refusal: the release is missing a fixture or a migration test.
/// Loud by construction: every variant names its culprit versions plus the
/// fixture path that would fix each of the first few.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MuseumRefusal {
    /// Declared versions with no fixture attached, sorted.
    Gateless {
        /// Every declared version missing its fixture, sorted and deduped.
        missing: Vec<VersionKey>,
    },
    /// Versions with a fixture but no recorded migration test, sorted.
    Unmigrated {
        /// Every version still waiting on its migration test, sorted.
        pending: Vec<VersionKey>,
    },
}

impl fmt::Display for MuseumRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Gateless { missing } => {
                write!(f, "museum refuses {} gateless version", missing.len())?;
                if missing.len() != 1 {
                    write!(f, "s")?;
                }
                write!(f, ": ")?;
                fmt_culprits(f, missing, "needs its fixture at")?;
                write!(f, "; add each fixture plus its migration test")?;
            }
            Self::Unmigrated { pending } => {
                write!(f, "museum refuses {} unmigrated version", pending.len())?;
                if pending.len() != 1 {
                    write!(f, "s")?;
                }
                write!(f, ": ")?;
                fmt_culprits(f, pending, "needs its migration test")?;
                write!(f, "; record each migration before release")?;
            }
        }
        Ok(())
    }
}

impl std::error::Error for MuseumRefusal {}

/// Name the first few culprits with their fix, then an "and N more" tail so
/// the text stays within [`MAX_DIAGNOSTIC_CHARS`].
fn fmt_culprits(f: &mut fmt::Formatter<'_>, culprits: &[VersionKey], need: &str) -> fmt::Result {
    let named = culprits.len().min(MAX_NAMED_CULPRITS);
    for (index, key) in culprits.iter().take(named).enumerate() {
        if index > 0 {
            write!(f, ", ")?;
        }
        write!(f, "{key} {need} {}", canonical_path(key.kind, key.version))?;
    }
    if culprits.len() > named {
        write!(f, "; and {} more", culprits.len() - named)?;
    }
    Ok(())
}

/// Starter kit for a new format version: canonical path, migration-test name
/// and deterministic placeholder bytes. The bytes carry the template marker,
/// so attaching them is refused: record real fixture bytes first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FixtureTemplate {
    /// Which versioned format the template covers.
    pub kind: FormatKind,
    /// The new format version.
    pub version: u32,
    /// Where the real fixture file must land.
    pub path: String,
    /// The migration test that must exist and call `record_migration`.
    pub test_name: String,
    /// Placeholder bytes; refused at attach time by construction.
    pub bytes: Vec<u8>,
}

/// Build the starter kit for a new format version. Fails on out-of-range
/// versions before any path is rendered.
pub fn fixture_template(kind: FormatKind, version: u32) -> Result<FixtureTemplate, MuseumError> {
    check_version(version)?;
    let mut bytes = TEMPLATE_MARKER.to_vec();
    bytes.extend_from_slice(kind.label().as_bytes());
    bytes.extend_from_slice(format!(":v{version}:record-real-fixture-bytes").as_bytes());
    Ok(FixtureTemplate {
        kind,
        version,
        path: canonical_path(kind, version),
        test_name: template_test_name(kind, version),
        bytes,
    })
}

/// Successful gate receipt: every declared version ships its fixture plus a
/// recorded migration test. The digest folds the sorted entries, so the same
/// declarations always give the same receipt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MuseumReceipt {
    /// Declared versions proven (fixtures present, migrations recorded).
    pub versions: usize,
    /// Deterministic fold over the sorted entries and fixture digests.
    pub digest: u64,
}

/// One registry row: an optional fixture plus whether its migration test ran.
/// Attaching fresh bytes clears the flag, so replaced bytes re-prove.
#[derive(Clone, Debug)]
struct MuseumEntry {
    fixture: Option<FixtureSpec>,
    migrated: bool,
}

/// Version registry: declared format versions mapped to their fixtures plus
/// migration-test record. Declaring and proving are separate steps, which is
/// what makes a gateless release fail by construction: `prove` walks the
/// declared set and refuses every version without a complete fixture.
#[derive(Clone, Debug, Default)]
pub struct MuseumRegistry {
    entries: BTreeMap<VersionKey, MuseumEntry>,
}

impl MuseumRegistry {
    /// Empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// The format-museum seed corpus: tape v1, scene v1 and bundle v1, each
    /// with representative bytes and a recorded migration. Real fixture bytes
    /// bind in the proof consumer; the registry owns presence and order.
    pub fn seed() -> Self {
        const SEED: [(FormatKind, &str, &[u8]); 3] = [
            (
                FormatKind::Tape,
                "fixtures/tape-v1.bin",
                b"E204MUSEUM:TAPE:V1",
            ),
            (
                FormatKind::Scene,
                "fixtures/scene-v1.json",
                br#"{"version":1,"nodes":[]}"#,
            ),
            (
                FormatKind::Bundle,
                "fixtures/bundle-v1.bin",
                b"E204BNDL\x01\x00\x00\x00",
            ),
        ];
        let mut entries = BTreeMap::new();
        for (kind, path, bytes) in SEED {
            entries.insert(
                VersionKey { kind, version: 1 },
                MuseumEntry {
                    fixture: Some(FixtureSpec {
                        path: path.into(),
                        bytes: bytes.into(),
                    }),
                    migrated: true,
                },
            );
        }
        Self { entries }
    }

    /// Declared version count.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True when no version is declared.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// True when the version is declared (fixture or not).
    pub fn contains(&self, kind: FormatKind, version: u32) -> bool {
        self.entries.contains_key(&VersionKey { kind, version })
    }

    /// Declare a new format version. Starts gateless: `prove` refuses it
    /// until a fixture is attached and its migration recorded.
    pub fn declare(&mut self, kind: FormatKind, version: u32) -> Result<(), MuseumError> {
        check_version(version)?;
        let key = VersionKey { kind, version };
        if self.entries.contains_key(&key) {
            return Err(MuseumError::Duplicate(key));
        }
        if self.entries.len() >= MAX_VERSIONS {
            return Err(MuseumError::TooMany {
                limit: MAX_VERSIONS,
            });
        }
        self.entries.insert(
            key,
            MuseumEntry {
                fixture: None,
                migrated: false,
            },
        );
        Ok(())
    }

    /// Attach a validated fixture to a declared version. Fresh bytes clear a
    /// previous migration record, so replaced bytes re-prove before release.
    pub fn attach_fixture(
        &mut self,
        kind: FormatKind,
        version: u32,
        fixture: FixtureSpec,
    ) -> Result<(), MuseumError> {
        let key = VersionKey { kind, version };
        let entry = self
            .entries
            .get_mut(&key)
            .ok_or(MuseumError::Unknown(key))?;
        entry.fixture = Some(fixture);
        entry.migrated = false;
        Ok(())
    }

    /// Record that the fixture migration test ran for a version. Needs the
    /// fixture first; a bare declaration cannot be marked migrated.
    pub fn record_migration(&mut self, kind: FormatKind, version: u32) -> Result<(), MuseumError> {
        let key = VersionKey { kind, version };
        let entry = self
            .entries
            .get_mut(&key)
            .ok_or(MuseumError::Unknown(key))?;
        if entry.fixture.is_none() {
            return Err(MuseumError::Invalid("migration needs its fixture first"));
        }
        entry.migrated = true;
        Ok(())
    }

    /// Declared versions with no fixture, sorted. Empty means every version
    /// ships bytes; the proof gate fails on anything else.
    pub fn gateless(&self) -> Vec<VersionKey> {
        self.entries
            .iter()
            .filter(|(_, entry)| entry.fixture.is_none())
            .map(|(key, _)| *key)
            .collect()
    }

    /// Versions with a fixture but no recorded migration test, sorted.
    pub fn pending_migration(&self) -> Vec<VersionKey> {
        self.entries
            .iter()
            .filter(|(_, entry)| entry.fixture.is_some() && !entry.migrated)
            .map(|(key, _)| *key)
            .collect()
    }

    /// The proof gate: every declared version needs its fixture plus a
    /// recorded migration test. Gateless versions fail first, unmigrated
    /// ones second; success returns the receipt for the release log.
    pub fn prove(&self) -> Result<MuseumReceipt, MuseumRefusal> {
        let missing = self.gateless();
        if !missing.is_empty() {
            return Err(MuseumRefusal::Gateless { missing });
        }
        let pending = self.pending_migration();
        if !pending.is_empty() {
            return Err(MuseumRefusal::Unmigrated { pending });
        }
        Ok(MuseumReceipt {
            versions: self.entries.len(),
            digest: self.digest(),
        })
    }

    /// Deterministic fold over the sorted entries: kind, version, migration
    /// bit, path bytes and the fixture content digest (raw bytes never enter
    /// the fold twice, so big fixtures stay cheap). Same declarations always
    /// give the same digest; any byte or flag change moves it.
    pub fn digest(&self) -> u64 {
        let mut folded: Vec<u8> = Vec::new();
        for (key, entry) in &self.entries {
            let mut row = Vec::new();
            row.push(key.kind as u8);
            row.extend_from_slice(&key.version.to_le_bytes());
            row.push(u8::from(entry.migrated));
            if let Some(fixture) = &entry.fixture {
                row.extend_from_slice(fixture.path.as_bytes());
                row.push(0);
                row.extend_from_slice(&content_hash(&fixture.bytes).to_le_bytes());
            }
            folded.extend_from_slice(&content_hash(&row).to_le_bytes());
        }
        content_hash(&folded)
    }
}

/// Shared version-range check for declarations and templates.
fn check_version(version: u32) -> Result<(), MuseumError> {
    if version < MIN_VERSION {
        return Err(MuseumError::Invalid("format version is zero"));
    }
    if version > MAX_VERSION {
        return Err(MuseumError::Invalid(
            "format version exceeds the museum bound",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn real_bytes(kind: FormatKind) -> Vec<u8> {
        match kind {
            FormatKind::Tape => b"E204TEST:TAPE:V2:24TICKS".to_vec(),
            FormatKind::Scene => br#"{"version":1,"nodes":[{"id":"x"}]}"#.to_vec(),
            FormatKind::Bundle => b"E204BNDL\x02\x00\x00\x00".to_vec(),
        }
    }

    fn attach_real(registry: &mut MuseumRegistry, kind: FormatKind, version: u32) {
        let spec = FixtureSpec::new(&canonical_path(kind, version), &real_bytes(kind))
            .expect("test fixture bytes are valid");
        registry
            .attach_fixture(kind, version, spec)
            .expect("version is declared");
    }

    fn key(kind: FormatKind, version: u32) -> VersionKey {
        VersionKey { kind, version }
    }

    #[test]
    fn seed_corpus_proves_twice_with_same_receipt() {
        let registry = MuseumRegistry::seed();
        assert_eq!(registry.len(), 3);
        assert!(registry.gateless().is_empty());
        assert!(registry.pending_migration().is_empty());
        let first = registry
            .prove()
            .expect("seed ships fixtures plus migrations");
        assert_eq!(first.versions, 3);
        let second = MuseumRegistry::seed()
            .prove()
            .expect("seed proves on a fresh build too");
        assert_eq!(first, second);
    }

    #[test]
    fn version_without_fixture_fails_proof_by_construction() {
        let mut registry = MuseumRegistry::seed();
        registry
            .declare(FormatKind::Tape, 2)
            .expect("fresh version declares");
        let refusal = registry.prove().unwrap_err();
        assert_eq!(
            refusal,
            MuseumRefusal::Gateless {
                missing: vec![key(FormatKind::Tape, 2)]
            }
        );
        // Loud: names the version and the fixture path that fixes it, bounded.
        let report = format!("{refusal}");
        assert!(report.contains("tape v2"), "report: {report}");
        assert!(report.contains("fixtures/tape-v2.bin"), "report: {report}");
        assert!(report.len() <= MAX_DIAGNOSTIC_CHARS, "report: {report}");
        // The failed proof changed nothing: the declaration stands, alone.
        assert_eq!(registry.len(), 4);
        assert_eq!(registry.gateless(), vec![key(FormatKind::Tape, 2)]);
    }

    #[test]
    fn fixture_without_migration_test_still_refused() {
        let mut registry = MuseumRegistry::seed();
        registry
            .declare(FormatKind::Tape, 2)
            .expect("fresh version declares");
        attach_real(&mut registry, FormatKind::Tape, 2);
        let refusal = registry.prove().unwrap_err();
        assert_eq!(
            refusal,
            MuseumRefusal::Unmigrated {
                pending: vec![key(FormatKind::Tape, 2)]
            }
        );
        let report = format!("{refusal}");
        assert!(report.contains("tape v2"), "report: {report}");
        assert!(report.contains("migration"), "report: {report}");
        assert!(report.len() <= MAX_DIAGNOSTIC_CHARS, "report: {report}");
        // The migration test runs, records, and the gate opens.
        registry
            .record_migration(FormatKind::Tape, 2)
            .expect("fixture is attached");
        let receipt = registry.prove().expect("migration is recorded");
        assert_eq!(receipt.versions, 4);
    }

    #[test]
    fn template_bytes_are_not_a_fixture() {
        let template = fixture_template(FormatKind::Tape, 2).expect("fresh version templates");
        assert_eq!(template.path, "fixtures/tape-v2.bin");
        assert!(!template.bytes.is_empty());
        let err = FixtureSpec::new(&template.path, &template.bytes).unwrap_err();
        assert_eq!(
            err,
            MuseumError::Invalid("template bytes are not a fixture")
        );
        let report = format!("{err}");
        assert!(report.contains("template"), "report: {report}");
    }

    #[test]
    fn reattached_bytes_require_a_fresh_migration() {
        let mut registry = MuseumRegistry::seed();
        let before = registry.digest();
        let spec = FixtureSpec::new("fixtures/tape-v1.bin", &real_bytes(FormatKind::Tape))
            .expect("test fixture bytes are valid");
        registry
            .attach_fixture(FormatKind::Tape, 1, spec)
            .expect("seed declares tape v1");
        // Fresh bytes clear the old migration record: the gate shuts again.
        assert_eq!(registry.pending_migration(), vec![key(FormatKind::Tape, 1)]);
        assert!(registry.prove().is_err());
        registry
            .record_migration(FormatKind::Tape, 1)
            .expect("replaced fixture re-records");
        registry.prove().expect("re-recorded migration proves");
        assert_ne!(registry.digest(), before);
    }

    #[test]
    fn duplicate_declare_refused_and_stores_nothing_new() {
        let mut registry = MuseumRegistry::seed();
        registry
            .declare(FormatKind::Tape, 2)
            .expect("first declare lands");
        let err = registry.declare(FormatKind::Tape, 2).unwrap_err();
        assert_eq!(err, MuseumError::Duplicate(key(FormatKind::Tape, 2)));
        assert!(format!("{err}").contains("tape v2"));
        assert_eq!(registry.len(), 4);
    }

    #[test]
    fn unknown_version_operations_refused() {
        let mut registry = MuseumRegistry::new();
        let ghost = key(FormatKind::Scene, 9);
        let spec = FixtureSpec::new("fixtures/scene-v9.json", &real_bytes(FormatKind::Scene))
            .expect("test fixture bytes are valid");
        assert_eq!(
            registry
                .attach_fixture(FormatKind::Scene, 9, spec)
                .unwrap_err(),
            MuseumError::Unknown(ghost)
        );
        assert_eq!(
            registry.record_migration(FormatKind::Scene, 9).unwrap_err(),
            MuseumError::Unknown(ghost)
        );
        assert!(format!("{}", MuseumError::Unknown(ghost)).contains("scene v9"));
        assert!(registry.is_empty());
    }

    #[test]
    fn migration_before_fixture_refused() {
        let mut registry = MuseumRegistry::seed();
        registry
            .declare(FormatKind::Tape, 2)
            .expect("fresh version declares");
        let err = registry.record_migration(FormatKind::Tape, 2).unwrap_err();
        assert_eq!(
            err,
            MuseumError::Invalid("migration needs its fixture first")
        );
        // Still gateless: the refusal did not mark anything.
        assert!(matches!(
            registry.prove().unwrap_err(),
            MuseumRefusal::Gateless { .. }
        ));
    }

    #[test]
    fn bad_versions_paths_and_bytes_refused() {
        let mut registry = MuseumRegistry::new();
        assert_eq!(
            registry.declare(FormatKind::Tape, 0).unwrap_err(),
            MuseumError::Invalid("format version is zero")
        );
        assert_eq!(
            registry
                .declare(FormatKind::Tape, MAX_VERSION + 1)
                .unwrap_err(),
            MuseumError::Invalid("format version exceeds the museum bound")
        );
        assert!(fixture_template(FormatKind::Tape, 0).is_err());
        let long_path = format!("fixtures/{}.bin", "a".repeat(MAX_PATH_CHARS));
        for bad_path in [
            "",
            long_path.as_str(),
            "fixtures/a b.bin",
            "fixtures/fixtüre.bin",
        ] {
            assert!(
                FixtureSpec::new(bad_path, b"x").is_err(),
                "path accepted: {bad_path:?}"
            );
        }
        assert!(FixtureSpec::new("fixtures/a.bin", b"").is_err());
        let huge = vec![0u8; MAX_FIXTURE_BYTES + 1];
        assert!(FixtureSpec::new("fixtures/a.bin", &huge).is_err());
        assert!(registry.is_empty());
    }

    #[test]
    fn receipt_digest_moves_with_content_not_declaration_order() {
        let left_digest = {
            let mut registry = MuseumRegistry::seed();
            registry
                .declare(FormatKind::Tape, 2)
                .expect("fresh version declares");
            registry
                .declare(FormatKind::Scene, 2)
                .expect("fresh version declares");
            attach_real(&mut registry, FormatKind::Tape, 2);
            attach_real(&mut registry, FormatKind::Scene, 2);
            registry
                .record_migration(FormatKind::Tape, 2)
                .expect("migration recorded");
            registry
                .record_migration(FormatKind::Scene, 2)
                .expect("migration recorded");
            registry.prove().expect("complete registry proves").digest
        };
        // Opposite declaration order reaches the same digest: sorted, stable.
        let mut flipped = MuseumRegistry::seed();
        flipped
            .declare(FormatKind::Scene, 2)
            .expect("fresh version declares");
        flipped
            .declare(FormatKind::Tape, 2)
            .expect("fresh version declares");
        attach_real(&mut flipped, FormatKind::Scene, 2);
        attach_real(&mut flipped, FormatKind::Tape, 2);
        flipped
            .record_migration(FormatKind::Scene, 2)
            .expect("migration recorded");
        flipped
            .record_migration(FormatKind::Tape, 2)
            .expect("migration recorded");
        assert_eq!(
            flipped.prove().expect("complete registry proves").digest,
            left_digest
        );
        // Different bytes move the digest: the receipt actually covers content.
        let mut changed = MuseumRegistry::seed();
        let spec = FixtureSpec::new("fixtures/tape-v1.bin", b"E204MUSEUM:TAPE:V1:OTHER")
            .expect("test fixture bytes are valid");
        changed
            .attach_fixture(FormatKind::Tape, 1, spec)
            .expect("seed declares tape v1");
        assert_ne!(changed.digest(), left_digest);
    }

    #[test]
    fn template_names_are_canonical_and_deterministic() {
        let first = fixture_template(FormatKind::Tape, 2).expect("fresh version templates");
        let second = fixture_template(FormatKind::Tape, 2).expect("template is pure");
        assert_eq!(first, second);
        assert_eq!(first.path, "fixtures/tape-v2.bin");
        assert_eq!(first.test_name, "museum_tape_v2_loads_migrates_and_replays");
        assert!(first.bytes.starts_with(TEMPLATE_MARKER));
        let scene = fixture_template(FormatKind::Scene, 3).expect("scene templates");
        assert_eq!(scene.path, "fixtures/scene-v3.json");
        let bundle = fixture_template(FormatKind::Bundle, 4).expect("bundle templates");
        assert_eq!(bundle.path, "fixtures/bundle-v4.bin");
    }

    #[test]
    fn refusal_diagnostics_stay_bounded() {
        let mut registry = MuseumRegistry::seed();
        // Fill to capacity across all three kinds: every one is gateless.
        for version in 2..=11 {
            registry
                .declare(FormatKind::Tape, version)
                .expect("capacity holds");
        }
        for version in 2..=11 {
            registry
                .declare(FormatKind::Scene, version)
                .expect("capacity holds");
        }
        for version in 2..=10 {
            registry
                .declare(FormatKind::Bundle, version)
                .expect("capacity holds");
        }
        assert_eq!(registry.len(), MAX_VERSIONS);
        let report = format!("{}", registry.prove().unwrap_err());
        assert!(report.contains("gateless"), "report: {report}");
        assert!(report.contains("and"), "report: {report}");
        assert!(
            report.len() <= MAX_DIAGNOSTIC_CHARS,
            "report too long ({}): {report}",
            report.len()
        );
        // One past capacity is refused loudly, storing nothing.
        assert_eq!(
            registry.declare(FormatKind::Bundle, 11).unwrap_err(),
            MuseumError::TooMany {
                limit: MAX_VERSIONS
            }
        );
        assert_eq!(registry.len(), MAX_VERSIONS);
    }

    #[test]
    fn empty_registry_proves_vacuous() {
        let registry = MuseumRegistry::new();
        assert!(registry.is_empty());
        let receipt = registry.prove().expect("nothing declared, nothing missing");
        assert_eq!(receipt.versions, 0);
    }
}
