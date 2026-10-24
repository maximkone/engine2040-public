//! manifest: a seeded bake manifest from inputs and settings to one artifact.
//!
//! Every cook writes one canonical text manifest beside its artifact: the
//! seed, the thread count, the target profile, the importer and its version,
//! the settings bytes, one record per input (relative path, registry id,
//! blake3 digest, byte count) and one record per artifact, with a blake3
//! checksum over every line above it. Reading it back is a byte-for-byte
//! round-trip, so the same inputs and settings rebuild the same artifact and a
//! changed input or setting is the only thing that shows up as stale.
//!
//! Two hashes, both existing canons: [`crate::registry::content_hash`]
//! (FNV-1a, the registry identity) and [`content_digest`](crate::asset_source::content_digest) (blake3, the digest
//! the bundle format uses). This module adds no new hash.
//!
//! The format is canonical: LF endings only, one line per field, records
//! sorted by path, no blank lines, a trailing `checksum` line over all lines
//! above it. Paths are relative, forward-slash and carry no whitespace; a
//! bundle entry name outside that set is percent-escaped into its manifest
//! path.
//!
//! Invalidation is not reimplemented here: [`crate::cooker::Cooker`] and
//! [`crate::registry::AssetRegistry`] own the transitive closure and the
//! last-good rule. This module reports which recorded inputs and artifacts
//! differ between two manifests, so a caller can show that exactly the closure
//! rebuilt.

use std::collections::BTreeMap;
use std::fmt;
use std::path::PathBuf;

// A browser has no file system: the manifest read/write helpers exist only on
// the native tools that cook and verify artifacts.
#[cfg(not(target_arch = "wasm32"))]
use std::path::Path;

use engine_scene::ids::content_digest;

#[cfg(all(feature = "import", feature = "registry"))]
use super::bundle::LoadedBundle;
#[cfg(all(feature = "import", feature = "registry"))]
use super::bundle_cook::{
    cook_bundle_bytes, verify_bundle_bytes, write_bundle_file, BundleCookError, BundleReceipt,
    StagedCooked,
};
use super::cooker::COOK_VERSION;
use super::registry::{content_hash, sha_hex};
use super::wire::bound_chars;

/// Manifest format version this build writes and reads.
pub const MANIFEST_VERSION: u32 = 1;
/// First token of a manifest file: `e2040-manifest <version>`.
pub const MANIFEST_MAGIC: &str = "e2040-manifest";
/// Importer name recorded in every manifest written here.
pub const IMPORTER: &str = "cooker";
/// Seed the bundled cook paths post: the engine's golden seed.
pub const DEFAULT_SEED: u64 = 2040;
/// Thread count a reproducible bake posts: one, so interleaving cannot move bytes.
pub const DEFAULT_THREADS: usize = 1;
/// Thread ceiling a manifest accepts.
pub const MAX_THREADS: usize = 256;
/// Target profile the demo bake posts.
pub const PROFILE_POTATO: &str = "potato";
/// Most records (inputs plus artifacts) one manifest holds.
pub const MAX_RECORDS: usize = 8192;
/// Most settings bytes one manifest records.
pub const MAX_SETTINGS_BYTES: usize = 4096;
/// Largest manifest file this module writes or reads.
pub const MAX_MANIFEST_BYTES: usize = 4 * 1024 * 1024;
/// Most characters in a profile, importer or role token.
pub const MAX_TOKEN_CHARS: usize = 32;
/// Most characters in one record path.
pub const MAX_PATH_CHARS: usize = 512;

/// Bounded echo length for error details.
const MAX_DETAIL_CHARS: usize = 128;
/// Lowercase hex length of a registry id.
const ID_HEX: usize = 16;
/// Lowercase hex length of a blake3 digest.
const DIGEST_HEX: usize = 64;
/// Header fields before the first record line.
const HEADER_LINES: usize = 7;
/// Role recorded for a plain source file.
const FILE_ROLE: &str = "file";
/// Role recorded for an artifact.
const ARTIFACT_ROLE: &str = "artifact";
/// Lowercase hex digits, shared by the encoders.
const HEX_DIGITS: &[u8; 16] = b"0123456789abcdef";

/// One recorded input or artifact: its manifest path, role, registry id,
/// blake3 digest and byte count. The bytes themselves live elsewhere (a source
/// file or the artifact beside the manifest), so the digest is the contract.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Record {
    path: String,
    role: String,
    id: String,
    digest: String,
    bytes: u64,
}

impl Record {
    fn of(path: &str, role: &str, bytes: &[u8]) -> Result<Self, ManifestError> {
        let len = u64::try_from(bytes.len())
            .map_err(|_| ManifestError::TooLarge { bytes: bytes.len() })?;
        Ok(Self {
            path: path.to_string(),
            role: role.to_string(),
            id: sha_hex(content_hash(bytes)),
            digest: content_digest(bytes),
            bytes: len,
        })
    }

    /// Manifest-relative path, forward slashes only.
    pub fn path(&self) -> &str {
        &self.path
    }

    /// Role that names what kind of input this is (`file`, `mesh`, ...).
    pub fn role(&self) -> &str {
        &self.role
    }

    /// Registry identity as 16 lowercase hex digits ([`content_hash`] canon).
    pub fn id(&self) -> &str {
        &self.id
    }

    /// blake3 digest as 64 lowercase hex digits ([`content_digest`] canon).
    pub fn digest(&self) -> &str {
        &self.digest
    }

    /// Recorded byte count.
    pub fn bytes(&self) -> u64 {
        self.bytes
    }
}

/// Why a manifest was refused or could not be written.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ManifestError {
    /// A thread count outside `1..=MAX_THREADS`.
    BadThreads {
        /// The refused thread count.
        threads: usize,
    },
    /// A field that must be a short token (profile, importer, role) was not.
    BadToken {
        /// Which field was refused.
        field: &'static str,
        /// The refused text, bounded.
        text: String,
    },
    /// A record path was empty, absolute, escaping, or held whitespace.
    BadPath(String),
    /// A manifest line could not be read; `line` is 1-based.
    Corrupt {
        /// 1-based line number of the refusal.
        line: usize,
        /// Short reason, never caller bytes.
        reason: &'static str,
    },
    /// The trailing checksum did not match the lines above it.
    Checksum,
    /// Two records named the same path.
    Duplicate(String),
    /// The record ceiling was reached.
    Full {
        /// The ceiling that was hit.
        records: usize,
    },
    /// Settings bytes above [`MAX_SETTINGS_BYTES`].
    SettingsTooLarge {
        /// The refused settings length.
        bytes: usize,
    },
    /// The manifest is larger than [`MAX_MANIFEST_BYTES`].
    TooLarge {
        /// The refused byte length.
        bytes: usize,
    },
    /// A manifest version this build cannot read.
    UnsupportedVersion(u32),
    /// An artifact the manifest names is not on disk beside it.
    ArtifactMissing {
        /// The artifact's manifest path.
        path: String,
    },
    /// An artifact's bytes no longer match its recorded blake3 and length.
    ArtifactMismatch {
        /// The artifact's manifest path.
        path: String,
    },
    /// A file operation failed (bounded detail).
    Io(String),
}

impl fmt::Display for ManifestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadThreads { threads } => {
                write!(f, "thread count {threads} is outside 1..={MAX_THREADS}")
            }
            Self::BadToken { field, text } => write!(f, "{field} {text:?} is not a token"),
            Self::BadPath(path) => write!(f, "path {path:?} is not a relative manifest path"),
            Self::Corrupt { line, reason } => write!(f, "manifest line {line}: {reason}"),
            Self::Checksum => write!(f, "manifest checksum does not match its lines"),
            Self::Duplicate(path) => write!(f, "path {path} is recorded twice"),
            Self::Full { records } => write!(f, "manifest holds at most {records} records"),
            Self::SettingsTooLarge { bytes } => {
                write!(f, "settings {bytes} bytes exceed {MAX_SETTINGS_BYTES}")
            }
            Self::TooLarge { bytes } => {
                write!(
                    f,
                    "{bytes} bytes exceed the {MAX_MANIFEST_BYTES} byte bound"
                )
            }
            Self::UnsupportedVersion(version) => {
                write!(f, "manifest version {version} is not {MANIFEST_VERSION}")
            }
            Self::ArtifactMissing { path } => write!(f, "artifact {path} is missing"),
            Self::ArtifactMismatch { path } => {
                write!(f, "artifact {path} does not match its digest")
            }
            Self::Io(detail) => write!(f, "io: {detail}"),
        }
    }
}

impl std::error::Error for ManifestError {}

/// Why a bundled bake could not finish: the bundle cook or the manifest.
#[cfg(all(feature = "import", feature = "registry"))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BakeError {
    /// The manifest could not be built, written or verified.
    Manifest(ManifestError),
    /// The bundle cook or its verification refused the staged inputs.
    Bundle(BundleCookError),
}

#[cfg(all(feature = "import", feature = "registry"))]
impl From<ManifestError> for BakeError {
    fn from(error: ManifestError) -> Self {
        Self::Manifest(error)
    }
}

#[cfg(all(feature = "import", feature = "registry"))]
impl From<BundleCookError> for BakeError {
    fn from(error: BundleCookError) -> Self {
        Self::Bundle(error)
    }
}

#[cfg(all(feature = "import", feature = "registry"))]
impl fmt::Display for BakeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Manifest(error) => write!(f, "manifest: {error}"),
            Self::Bundle(error) => write!(f, "bundle: {error}"),
        }
    }
}

#[cfg(all(feature = "import", feature = "registry"))]
impl std::error::Error for BakeError {}

/// One finished bundled bake: the bundle receipt, the manifest beside it and
/// the blake3 digest over the manifest file.
#[cfg(all(feature = "import", feature = "registry"))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BakedManifest {
    /// The written bundle: entry count, byte count, path.
    pub bundle: BundleReceipt,
    /// The manifest written beside the bundle.
    pub manifest: PathBuf,
    /// blake3 hex over the manifest file bytes.
    pub digest: String,
    /// Input records recorded (one per bundle entry).
    pub sources: usize,
}

#[cfg(all(feature = "import", feature = "registry"))]
impl fmt::Display for BakedManifest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} manifest:{} sources:{} blake3:{}",
            self.bundle,
            self.manifest.display(),
            self.sources,
            self.digest
        )
    }
}

/// Seeded bake manifest: inputs plus settings and versions to artifacts.
///
/// Source and artifact paths share one namespace: a path is one record, so a
/// manifest never records the same file as both. Iteration order is path
/// order, so repeated builds write identical bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Manifest {
    seed: u64,
    threads: usize,
    profile: String,
    importer: String,
    importer_version: u32,
    settings: Vec<u8>,
    sources: BTreeMap<String, Record>,
    artifacts: BTreeMap<String, Record>,
}

impl Manifest {
    /// Empty manifest with the importer name and [`COOK_VERSION`] posted.
    ///
    /// Errors: [`ManifestError::BadThreads`] outside `1..=MAX_THREADS` and
    /// [`ManifestError::BadToken`] when `profile` is not a short token.
    pub fn new(seed: u64, threads: usize, profile: &str) -> Result<Self, ManifestError> {
        check_threads(threads)?;
        check_token("profile", profile, MAX_TOKEN_CHARS)?;
        Ok(Self {
            seed,
            threads,
            profile: profile.to_string(),
            importer: IMPORTER.to_string(),
            importer_version: COOK_VERSION,
            settings: Vec::new(),
            sources: BTreeMap::new(),
            artifacts: BTreeMap::new(),
        })
    }

    /// Record the cook settings bytes that produced the artifacts. They are
    /// stored verbatim so a read gives the exact same bytes back.
    pub fn set_settings(&mut self, settings: &[u8]) -> Result<(), ManifestError> {
        if settings.len() > MAX_SETTINGS_BYTES {
            return Err(ManifestError::SettingsTooLarge {
                bytes: settings.len(),
            });
        }
        self.settings = settings.to_vec();
        Ok(())
    }

    /// Record one source input under its relative path and role. The bytes are
    /// hashed, never stored.
    pub fn add_source(
        &mut self,
        path: &str,
        role: &str,
        bytes: &[u8],
    ) -> Result<(), ManifestError> {
        self.insert(false, path, role, bytes)
    }

    /// Record one source file with the `file` role.
    pub fn add_file_source(&mut self, path: &str, bytes: &[u8]) -> Result<(), ManifestError> {
        self.insert(false, path, FILE_ROLE, bytes)
    }

    /// Record one artifact under its path relative to the manifest. Writing
    /// the manifest re-reads the file and refuses a digest or length mismatch.
    pub fn add_artifact(&mut self, path: &str, bytes: &[u8]) -> Result<(), ManifestError> {
        self.insert(true, path, ARTIFACT_ROLE, bytes)
    }

    /// Record one source per verified bundle entry: role is the entry kind and
    /// the path is `entries/<kind>/<escaped name>`. Returns the count added.
    #[cfg(all(feature = "import", feature = "registry"))]
    pub fn add_bundle(&mut self, bundle: &LoadedBundle) -> Result<usize, ManifestError> {
        let mut added = 0;
        for entry in &bundle.entries {
            let role = entry.kind.label();
            let path = format!("entries/{role}/{}", escape_name(&entry.name));
            self.add_source(&path, role, &entry.bytes)?;
            added += 1;
        }
        Ok(added)
    }

    fn insert(
        &mut self,
        artifact: bool,
        path: &str,
        role: &str,
        bytes: &[u8],
    ) -> Result<(), ManifestError> {
        validate_path(path)?;
        check_token("role", role, MAX_TOKEN_CHARS)?;
        if self.len() >= MAX_RECORDS {
            return Err(ManifestError::Full {
                records: MAX_RECORDS,
            });
        }
        if self.sources.contains_key(path) || self.artifacts.contains_key(path) {
            return Err(ManifestError::Duplicate(bound_chars(
                path,
                MAX_DETAIL_CHARS,
            )));
        }
        let record = Record::of(path, role, bytes)?;
        let table = if artifact {
            &mut self.artifacts
        } else {
            &mut self.sources
        };
        table.insert(path.to_string(), record);
        Ok(())
    }

    /// Seed the bake was run with.
    pub fn seed(&self) -> u64 {
        self.seed
    }

    /// Thread count the bake was run with.
    pub fn threads(&self) -> usize {
        self.threads
    }

    /// Target profile the bake was run with.
    pub fn profile(&self) -> &str {
        &self.profile
    }

    /// Importer name that produced the artifacts.
    pub fn importer(&self) -> &str {
        &self.importer
    }

    /// Importer version ([`COOK_VERSION`] at `new`) that produced the artifacts.
    pub fn importer_version(&self) -> u32 {
        self.importer_version
    }

    /// Settings bytes exactly as recorded.
    pub fn settings(&self) -> &[u8] {
        &self.settings
    }

    /// Source records in path order.
    pub fn sources(&self) -> impl Iterator<Item = &Record> + '_ {
        self.sources.values()
    }

    /// Artifact records in path order.
    pub fn artifacts(&self) -> impl Iterator<Item = &Record> + '_ {
        self.artifacts.values()
    }

    /// Record count, sources plus artifacts.
    pub fn len(&self) -> usize {
        self.sources.len() + self.artifacts.len()
    }

    /// True when no record is recorded.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Canonical file bytes: header, settings, sorted records, then the blake3
    /// checksum over every line above it.
    pub fn to_bytes(&self) -> Result<Vec<u8>, ManifestError> {
        let mut text = self.lines().join("\n");
        text.push('\n');
        let checksum = content_digest(text.as_bytes());
        text.push_str("checksum ");
        text.push_str(&checksum);
        text.push('\n');
        if text.len() > MAX_MANIFEST_BYTES {
            return Err(ManifestError::TooLarge { bytes: text.len() });
        }
        Ok(text.into_bytes())
    }

    /// blake3 hex over the whole manifest file, checksum line included.
    pub fn digest(&self) -> Result<String, ManifestError> {
        Ok(content_digest(&self.to_bytes()?))
    }

    fn lines(&self) -> Vec<String> {
        let mut lines = Vec::with_capacity(self.len() + HEADER_LINES);
        lines.push(format!("{MANIFEST_MAGIC} {MANIFEST_VERSION}"));
        lines.push(format!("seed {}", self.seed));
        lines.push(format!("threads {}", self.threads));
        lines.push(format!("profile {}", self.profile));
        lines.push(format!("importer {}", self.importer));
        lines.push(format!("importer-version {}", self.importer_version));
        lines.push(format!("settings {}", settings_text(&self.settings)));
        for record in self.sources.values() {
            lines.push(format!(
                "source {} {} {} {} {}",
                record.role, record.path, record.id, record.digest, record.bytes
            ));
        }
        for record in self.artifacts.values() {
            lines.push(format!(
                "artifact {} {} {} {}",
                record.path, record.id, record.digest, record.bytes
            ));
        }
        lines
    }

    /// Parse canonical manifest bytes. Refuses a corrupt body, a checksum
    /// mismatch, a future version, a truncated file, a non-canonical order and
    /// any path or token outside the recorded set.
    pub fn parse(bytes: &[u8]) -> Result<Self, ManifestError> {
        if bytes.len() > MAX_MANIFEST_BYTES {
            return Err(ManifestError::TooLarge { bytes: bytes.len() });
        }
        let text = std::str::from_utf8(bytes).map_err(|_| corrupt(0, "not utf-8"))?;
        if text.contains('\r') {
            return Err(corrupt(0, "carriage return"));
        }
        let body = text
            .strip_suffix('\n')
            .ok_or_else(|| corrupt(0, "no final newline"))?;
        let (body, checksum) = body
            .rsplit_once('\n')
            .ok_or_else(|| corrupt(1, "no checksum line"))?;
        let checksum = checksum
            .strip_prefix("checksum ")
            .ok_or_else(|| corrupt(1, "no checksum line"))?;
        if !is_hex(checksum, DIGEST_HEX) {
            return Err(corrupt(1, "checksum is not blake3 hex"));
        }
        if content_digest(format!("{body}\n").as_bytes()) != checksum {
            return Err(ManifestError::Checksum);
        }
        let lines: Vec<&str> = body.split('\n').collect();
        let header = parse_header(&lines)?;
        let (sources, artifacts) = parse_records(&lines, HEADER_LINES)?;
        let manifest = Self {
            seed: header.seed,
            threads: header.threads,
            profile: header.profile,
            importer: header.importer,
            importer_version: header.importer_version,
            settings: header.settings,
            sources,
            artifacts,
        };
        if manifest.to_bytes()?.as_slice() != bytes {
            return Err(corrupt(lines.len(), "not canonical"));
        }
        Ok(manifest)
    }

    /// Write the manifest to `path`, verifying every artifact first. Writes a
    /// sibling temp file, renames it into place and reads it back, so a failed
    /// write never leaves a manifest that claims artifacts it does not match.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn write(&self, path: &Path) -> Result<PathBuf, ManifestError> {
        let file = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| ManifestError::BadPath(String::new()))?;
        validate_path(file)?;
        let dir = match path.parent() {
            Some(parent) if !parent.as_os_str().is_empty() => parent.to_path_buf(),
            _ => PathBuf::from("."),
        };
        self.verify_artifacts(&dir)?;
        let bytes = self.to_bytes()?;
        let temp = dir.join(format!(".{file}.tmp"));
        std::fs::write(&temp, &bytes).map_err(io_err)?;
        std::fs::rename(&temp, path).map_err(io_err)?;
        let back = std::fs::read(path).map_err(io_err)?;
        if back != bytes {
            return Err(ManifestError::Io(bound_chars(
                "read-back differs",
                MAX_DETAIL_CHARS,
            )));
        }
        Ok(path.to_path_buf())
    }

    /// Read a manifest file. Artifact bytes are not checked here; call
    /// [`Manifest::verify_artifacts`] when the files must match.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn read(path: &Path) -> Result<Self, ManifestError> {
        let meta = std::fs::metadata(path).map_err(io_err)?;
        let len = usize::try_from(meta.len()).map_err(|_| ManifestError::TooLarge {
            bytes: MAX_MANIFEST_BYTES,
        })?;
        if len > MAX_MANIFEST_BYTES {
            return Err(ManifestError::TooLarge { bytes: len });
        }
        let bytes = std::fs::read(path).map_err(io_err)?;
        Self::parse(&bytes)
    }

    /// Re-read every artifact under `root` and refuse a length or blake3
    /// mismatch, so a manifest never vouches for bytes it did not hash.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn verify_artifacts(&self, root: &Path) -> Result<(), ManifestError> {
        for record in self.artifacts.values() {
            let file = root.join(&record.path);
            let meta = std::fs::metadata(&file).map_err(|_| ManifestError::ArtifactMissing {
                path: record.path.clone(),
            })?;
            let bytes = std::fs::read(&file).map_err(|_| ManifestError::ArtifactMissing {
                path: record.path.clone(),
            })?;
            if meta.len() != record.bytes || content_digest(&bytes) != record.digest {
                return Err(ManifestError::ArtifactMismatch {
                    path: record.path.clone(),
                });
            }
        }
        Ok(())
    }

    /// Source paths whose digest, length or role differs from `previous`,
    /// including paths added or dropped. Sorted, so the answer is stable.
    pub fn changed_sources(&self, previous: &Self) -> Vec<String> {
        changed_paths(&self.sources, &previous.sources)
    }

    /// Artifact paths that need a new bake relative to `previous`: bytes
    /// changed, artifact added or dropped, or any bake key (seed, thread count,
    /// profile, importer, importer version, settings) changed, which redoes
    /// every artifact. Sorted.
    pub fn stale_artifacts(&self, previous: &Self) -> Vec<String> {
        if self.bake_key_differs(previous) {
            let mut all: Vec<String> = self.artifacts.keys().cloned().collect();
            for path in previous.artifacts.keys() {
                if !self.artifacts.contains_key(path) {
                    all.push(path.clone());
                }
            }
            all.sort();
            all.dedup();
            return all;
        }
        changed_paths(&self.artifacts, &previous.artifacts)
    }

    fn bake_key_differs(&self, other: &Self) -> bool {
        self.seed != other.seed
            || self.threads != other.threads
            || self.profile != other.profile
            || self.importer != other.importer
            || self.importer_version != other.importer_version
            || self.settings != other.settings
    }
}

/// Cook `staged` into a bundle at `out`, verify it, then write
/// `<out>.manifest` beside it: one source per bundle entry plus the bundle as
/// the artifact, with the seed, thread count and profile posted.
#[cfg(all(feature = "import", feature = "registry"))]
pub fn cook_bundle_with_manifest(
    staged: &StagedCooked,
    out: &Path,
    allow_stub: bool,
    seed: u64,
    threads: usize,
    profile: &str,
) -> Result<BakedManifest, BakeError> {
    let bytes = cook_bundle_bytes(staged)?;
    let bundle = write_bundle_file(&bytes, out, allow_stub)?;
    let loaded = verify_bundle_bytes(&bytes, allow_stub)?;
    let mut manifest = Manifest::new(seed, threads, profile)?;
    let sources = manifest.add_bundle(&loaded)?;
    let artifact = out
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| ManifestError::BadPath(String::new()))?;
    manifest.add_artifact(artifact, &bytes)?;
    let manifest_path = out.with_file_name(format!("{artifact}.manifest"));
    let written = manifest.write(&manifest_path)?;
    let digest = manifest.digest()?;
    Ok(BakedManifest {
        bundle,
        manifest: written,
        digest,
        sources,
    })
}

/// Header fields of one manifest; built by `parse_header`.
struct Header {
    seed: u64,
    threads: usize,
    profile: String,
    importer: String,
    importer_version: u32,
    settings: Vec<u8>,
}

/// One parsed record line.
enum LineRecord {
    Source(Record),
    Artifact(Record),
}

fn parse_header(lines: &[&str]) -> Result<Header, ManifestError> {
    let version = parse_count(take_field(lines, 0, MANIFEST_MAGIC)?)
        .ok_or_else(|| corrupt(1, "bad version"))?;
    let version = u32::try_from(version).map_err(|_| corrupt(1, "bad version"))?;
    if version != MANIFEST_VERSION {
        return Err(ManifestError::UnsupportedVersion(version));
    }
    let seed = parse_count(take_field(lines, 1, "seed")?).ok_or_else(|| corrupt(2, "bad seed"))?;
    let threads =
        parse_count(take_field(lines, 2, "threads")?).ok_or_else(|| corrupt(3, "bad threads"))?;
    let threads = usize::try_from(threads).map_err(|_| corrupt(3, "bad threads"))?;
    check_threads(threads)?;
    let profile = take_field(lines, 3, "profile")?;
    check_token("profile", profile, MAX_TOKEN_CHARS)?;
    let importer = take_field(lines, 4, "importer")?;
    check_token("importer", importer, MAX_TOKEN_CHARS)?;
    let importer_version = parse_count(take_field(lines, 5, "importer-version")?)
        .ok_or_else(|| corrupt(6, "bad importer version"))?;
    let importer_version =
        u32::try_from(importer_version).map_err(|_| corrupt(6, "bad importer version"))?;
    let settings = parse_settings(take_field(lines, 6, "settings")?)?;
    Ok(Header {
        seed,
        threads,
        profile: profile.to_string(),
        importer: importer.to_string(),
        importer_version,
        settings,
    })
}

fn parse_settings(text: &str) -> Result<Vec<u8>, ManifestError> {
    if text == "-" {
        return Ok(Vec::new());
    }
    let bytes = unhex(text).ok_or_else(|| corrupt(7, "settings are not lowercase hex"))?;
    if bytes.len() > MAX_SETTINGS_BYTES {
        return Err(ManifestError::SettingsTooLarge { bytes: bytes.len() });
    }
    Ok(bytes)
}

/// Source and artifact record tables, each keyed by manifest path.
type RecordTables = (BTreeMap<String, Record>, BTreeMap<String, Record>);

fn parse_records(lines: &[&str], start: usize) -> Result<RecordTables, ManifestError> {
    let mut sources: BTreeMap<String, Record> = BTreeMap::new();
    let mut artifacts: BTreeMap<String, Record> = BTreeMap::new();
    for (offset, line) in lines.iter().enumerate().skip(start) {
        let number = offset + 1;
        if sources.len() + artifacts.len() >= MAX_RECORDS {
            return Err(ManifestError::Full {
                records: MAX_RECORDS,
            });
        }
        match parse_record_line(number, line)? {
            LineRecord::Source(record) => push_sorted(&mut sources, number, record)?,
            LineRecord::Artifact(record) => {
                if sources.contains_key(&record.path) {
                    return Err(corrupt(number, "path is both source and artifact"));
                }
                push_sorted(&mut artifacts, number, record)?;
            }
        }
    }
    Ok((sources, artifacts))
}

fn parse_record_line(number: usize, line: &str) -> Result<LineRecord, ManifestError> {
    if line.split(' ').any(str::is_empty) {
        return Err(corrupt(number, "record spacing"));
    }
    let tokens: Vec<&str> = line.split(' ').collect();
    let (artifact, role, path, id, digest, bytes) = match tokens.as_slice() {
        ["source", role, path, id, digest, bytes] => (false, *role, *path, *id, *digest, *bytes),
        ["artifact", path, id, digest, bytes] => (true, ARTIFACT_ROLE, *path, *id, *digest, *bytes),
        _ => return Err(corrupt(number, "unknown record")),
    };
    if !artifact {
        check_token("role", role, MAX_TOKEN_CHARS)?;
    }
    validate_path(path)?;
    if !is_hex(id, ID_HEX) {
        return Err(corrupt(number, "id is not 16 lowercase hex"));
    }
    if !is_hex(digest, DIGEST_HEX) {
        return Err(corrupt(number, "digest is not 64 lowercase hex"));
    }
    let bytes = parse_count(bytes).ok_or_else(|| corrupt(number, "bad byte count"))?;
    let record = Record {
        path: path.to_string(),
        role: role.to_string(),
        id: id.to_string(),
        digest: digest.to_string(),
        bytes,
    };
    Ok(if artifact {
        LineRecord::Artifact(record)
    } else {
        LineRecord::Source(record)
    })
}

fn push_sorted(
    table: &mut BTreeMap<String, Record>,
    number: usize,
    record: Record,
) -> Result<(), ManifestError> {
    if let Some(last) = table.keys().next_back() {
        if record.path.as_str() <= last.as_str() {
            return Err(corrupt(number, "records out of order"));
        }
    }
    table.insert(record.path.clone(), record);
    Ok(())
}

fn take_field<'a>(lines: &[&'a str], index: usize, key: &str) -> Result<&'a str, ManifestError> {
    let line = lines
        .get(index)
        .ok_or_else(|| corrupt(index + 1, "missing field"))?;
    let (got, value) = line
        .split_once(' ')
        .ok_or_else(|| corrupt(index + 1, "field without value"))?;
    if got != key {
        return Err(corrupt(index + 1, "field out of order"));
    }
    Ok(value)
}

fn changed_paths(
    current: &BTreeMap<String, Record>,
    previous: &BTreeMap<String, Record>,
) -> Vec<String> {
    let mut out = Vec::new();
    for (path, record) in current {
        let same = previous.get(path).is_some_and(|old| old == record);
        if !same {
            out.push(path.clone());
        }
    }
    for path in previous.keys() {
        if !current.contains_key(path) {
            out.push(path.clone());
        }
    }
    out.sort();
    out.dedup();
    out
}

fn check_threads(threads: usize) -> Result<(), ManifestError> {
    if threads == 0 || threads > MAX_THREADS {
        return Err(ManifestError::BadThreads { threads });
    }
    Ok(())
}

fn check_token(field: &'static str, text: &str, max: usize) -> Result<(), ManifestError> {
    let ok = !text.is_empty()
        && text.chars().count() <= max
        && text.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_' || byte == b'.'
        });
    if ok {
        Ok(())
    } else {
        Err(ManifestError::BadToken {
            field,
            text: bound_chars(text, MAX_DETAIL_CHARS),
        })
    }
}

fn validate_path(path: &str) -> Result<(), ManifestError> {
    let ok = !path.is_empty()
        && path.chars().count() <= MAX_PATH_CHARS
        && !path.starts_with('/')
        && !path.contains('\\')
        && !path.contains(':')
        && !path.chars().any(|ch| ch.is_whitespace() || ch.is_control())
        && path
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..");
    if ok {
        Ok(())
    } else {
        Err(ManifestError::BadPath(bound_chars(path, MAX_DETAIL_CHARS)))
    }
}

fn settings_text(settings: &[u8]) -> String {
    if settings.is_empty() {
        "-".to_string()
    } else {
        hex_of(settings)
    }
}

fn hex_of(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(char::from(HEX_DIGITS[usize::from(byte >> 4)]));
        out.push(char::from(HEX_DIGITS[usize::from(byte & 0x0f)]));
    }
    out
}

fn unhex(text: &str) -> Option<Vec<u8>> {
    let bytes = text.as_bytes();
    if !bytes.len().is_multiple_of(2) {
        return None;
    }
    let mut out = Vec::with_capacity(bytes.len() / 2);
    let mut index = 0;
    while index < bytes.len() {
        let high = hex_value(bytes[index])?;
        let low = hex_value(bytes[index + 1])?;
        out.push((high << 4) | low);
        index += 2;
    }
    Some(out)
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    }
}

fn is_hex(text: &str, len: usize) -> bool {
    text.len() == len && text.bytes().all(|byte| hex_value(byte).is_some())
}

fn parse_count(text: &str) -> Option<u64> {
    if text.is_empty() || text.len() > 20 {
        return None;
    }
    if text.len() > 1 && text.starts_with('0') {
        return None;
    }
    text.parse::<u64>().ok()
}

fn corrupt(line: usize, reason: &'static str) -> ManifestError {
    ManifestError::Corrupt { line, reason }
}

#[cfg(not(target_arch = "wasm32"))]
fn io_err(error: std::io::Error) -> ManifestError {
    ManifestError::Io(bound_chars(&error.to_string(), MAX_DETAIL_CHARS))
}

/// Escape one bundle entry name into a manifest path component: ASCII
/// alphanumerics and `._-` stay, every other byte becomes `%XX`.
fn escape_name(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for byte in name.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'.' | b'-' | b'_' => {
                out.push(char::from(byte));
            }
            _ => {
                out.push('%');
                out.push(char::from(HEX_DIGITS[usize::from(byte >> 4)]));
                out.push(char::from(HEX_DIGITS[usize::from(byte & 0x0f)]));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("manifest_unit_{}_{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create fixture dir");
        dir
    }

    fn sample() -> Manifest {
        let mut manifest =
            Manifest::new(DEFAULT_SEED, DEFAULT_THREADS, PROFILE_POTATO).expect("manifest builds");
        manifest.set_settings(b"quality=potato").expect("settings");
        manifest
            .add_file_source("src/demo.gltf", b"gltf-source")
            .expect("source");
        manifest
            .add_source("entries/mesh/demo_mesh", "mesh", b"mesh-payload")
            .expect("entry source");
        manifest
            .add_artifact("demo.e2bundle", b"bundle-bytes")
            .expect("artifact");
        manifest
    }

    fn sample_bytes() -> Vec<u8> {
        sample().to_bytes().expect("manifest bytes")
    }

    fn resum(body: &str) -> Vec<u8> {
        let mut signed = String::from(body);
        signed.push('\n');
        let checksum = content_digest(signed.as_bytes());
        format!("{signed}checksum {checksum}\n").into_bytes()
    }

    #[test]
    fn round_trip_posts_seed_threads_versions_and_settings() {
        let dir = fixture_dir("roundtrip");
        let artifact = b"bundle-bytes";
        std::fs::write(dir.join("demo.e2bundle"), artifact).expect("write artifact");
        let manifest = sample();
        let path = manifest
            .write(&dir.join("demo.e2bundle.manifest"))
            .expect("manifest writes");
        let text = std::fs::read_to_string(&path).expect("manifest text");
        assert!(text.contains(&format!("\nseed {DEFAULT_SEED}\n")), "{text}");
        assert!(text.contains(&format!("\nthreads {DEFAULT_THREADS}\n")));
        assert!(text.contains(&format!("\nimporter-version {COOK_VERSION}\n")));
        assert!(text.contains(&format!("\nprofile {PROFILE_POTATO}\n")));
        assert!(text.contains("\nimporter cooker\n"));
        let back = Manifest::read(&path).expect("manifest reads back");
        assert_eq!(back.seed(), DEFAULT_SEED);
        assert_eq!(back.threads(), DEFAULT_THREADS);
        assert_eq!(back.profile(), PROFILE_POTATO);
        assert_eq!(back.importer(), IMPORTER);
        assert_eq!(back.importer_version(), COOK_VERSION);
        assert_eq!(back.settings(), b"quality=potato");
        assert_eq!(back.sources().count(), 2);
        assert_eq!(back.artifacts().count(), 1);
        assert_eq!(
            back.to_bytes().expect("bytes"),
            manifest.to_bytes().expect("bytes")
        );
        assert_eq!(
            back.digest().expect("digest"),
            manifest.digest().expect("digest")
        );
        back.verify_artifacts(&dir).expect("artifact verifies");
        let record = back.artifacts().next().expect("one artifact");
        assert_eq!(record.path(), "demo.e2bundle");
        assert_eq!(record.role(), ARTIFACT_ROLE);
        assert_eq!(record.id(), sha_hex(content_hash(artifact)));
        assert_eq!(record.digest(), content_digest(artifact));
        assert_eq!(record.bytes(), u64::try_from(artifact.len()).expect("len"));
    }

    #[test]
    fn corrupt_checksum_truncation_and_future_version_are_refused() {
        let bytes = sample_bytes();
        let text = String::from_utf8(bytes.clone()).expect("utf8");
        let tampered = text.replace("profile potato", "profile potat0");
        assert_eq!(
            Manifest::parse(tampered.as_bytes()),
            Err(ManifestError::Checksum)
        );
        let truncated = &bytes[..bytes.len() - 12];
        assert!(matches!(
            Manifest::parse(truncated),
            Err(ManifestError::Corrupt { .. })
        ));
        let body = text
            .strip_suffix('\n')
            .and_then(|rest| rest.rsplit_once('\n'))
            .map(|(body, _)| body)
            .expect("body");
        let future = body.replacen("e2040-manifest 1", "e2040-manifest 2", 1);
        assert_eq!(
            Manifest::parse(&resum(&future)),
            Err(ManifestError::UnsupportedVersion(2))
        );
    }

    #[test]
    fn absolute_escaping_and_duplicate_paths_are_refused() {
        let mut manifest =
            Manifest::new(DEFAULT_SEED, DEFAULT_THREADS, PROFILE_POTATO).expect("manifest builds");
        for path in [
            "/abs.bin",
            "../up.bin",
            "a/../../b.bin",
            "a b.bin",
            "C:/outside.bin",
            "a\\b.bin",
            "",
        ] {
            assert!(
                matches!(
                    manifest.add_file_source(path, b"x"),
                    Err(ManifestError::BadPath(_))
                ),
                "path {path:?} must be refused"
            );
        }
        manifest
            .add_file_source("a.bin", b"first")
            .expect("relative path");
        assert!(matches!(
            manifest.add_file_source("a.bin", b"second"),
            Err(ManifestError::Duplicate(_))
        ));
        assert!(matches!(
            manifest.add_artifact("a.bin", b"second"),
            Err(ManifestError::Duplicate(_))
        ));
        assert!(matches!(
            Manifest::new(DEFAULT_SEED, 0, PROFILE_POTATO),
            Err(ManifestError::BadThreads { threads: 0 })
        ));
        assert!(matches!(
            Manifest::new(DEFAULT_SEED, MAX_THREADS + 1, PROFILE_POTATO),
            Err(ManifestError::BadThreads { .. })
        ));
        assert!(matches!(
            Manifest::new(DEFAULT_SEED, DEFAULT_THREADS, "not a token"),
            Err(ManifestError::BadToken { .. })
        ));
    }

    #[test]
    fn settings_round_trip_at_the_bound() {
        let mut manifest =
            Manifest::new(DEFAULT_SEED, DEFAULT_THREADS, PROFILE_POTATO).expect("manifest builds");
        assert!(matches!(
            manifest.set_settings(&vec![0u8; MAX_SETTINGS_BYTES + 1]),
            Err(ManifestError::SettingsTooLarge { .. })
        ));
        let settings = vec![0x5au8; MAX_SETTINGS_BYTES];
        manifest.set_settings(&settings).expect("bound settings");
        let back = Manifest::parse(&manifest.to_bytes().expect("bytes")).expect("parse");
        assert_eq!(back.settings(), settings.as_slice());
        let empty =
            Manifest::new(DEFAULT_SEED, DEFAULT_THREADS, PROFILE_POTATO).expect("manifest builds");
        let text = String::from_utf8(empty.to_bytes().expect("bytes")).expect("utf8");
        assert!(text.contains("\nsettings -\n"), "{text}");
    }

    #[test]
    fn write_verifies_artifacts_on_disk() {
        let dir = fixture_dir("verify");
        let mut manifest =
            Manifest::new(DEFAULT_SEED, DEFAULT_THREADS, PROFILE_POTATO).expect("manifest builds");
        manifest.add_artifact("out.bin", b"good").expect("artifact");
        let path = dir.join("out.bin.manifest");
        assert!(matches!(
            manifest.write(&path),
            Err(ManifestError::ArtifactMissing { .. })
        ));
        std::fs::write(dir.join("out.bin"), b"evil").expect("write evil");
        assert!(matches!(
            manifest.write(&path),
            Err(ManifestError::ArtifactMismatch { .. })
        ));
        std::fs::write(dir.join("out.bin"), b"good").expect("write good");
        manifest.write(&path).expect("manifest writes");
        let back = Manifest::read(&path).expect("reads back");
        back.verify_artifacts(&dir).expect("artifact verifies");
    }

    #[test]
    fn stale_artifacts_report_exactly_the_changed_bytes() {
        let mut before =
            Manifest::new(DEFAULT_SEED, DEFAULT_THREADS, PROFILE_POTATO).expect("manifest builds");
        before.add_artifact("a.bin", b"a").expect("a");
        before.add_artifact("b.bin", b"b").expect("b");
        let mut after =
            Manifest::new(DEFAULT_SEED, DEFAULT_THREADS, PROFILE_POTATO).expect("manifest builds");
        after.add_artifact("a.bin", b"a").expect("a");
        after.add_artifact("b.bin", b"b2").expect("b2");
        assert_eq!(after.stale_artifacts(&before), vec!["b.bin".to_string()]);
        assert_eq!(after.changed_sources(&before), Vec::<String>::new());
        let mut settings =
            Manifest::new(DEFAULT_SEED, DEFAULT_THREADS, PROFILE_POTATO).expect("manifest builds");
        settings.set_settings(b"other").expect("settings");
        settings.add_artifact("a.bin", b"a").expect("a");
        settings.add_artifact("b.bin", b"b").expect("b");
        assert_eq!(
            settings.stale_artifacts(&before),
            vec!["a.bin".to_string(), "b.bin".to_string()]
        );
    }

    #[test]
    fn entry_names_escape_to_relative_paths() {
        assert_eq!(escape_name("hero_body-01.gltf"), "hero_body-01.gltf");
        assert_eq!(escape_name("a b"), "a%20b");
        assert_eq!(escape_name("a/b"), "a%2fb");
        assert_eq!(escape_name("h\u{e9}"), "h%c3%a9");
    }

    #[cfg(all(feature = "import", feature = "registry"))]
    #[test]
    fn bundle_entries_become_sorted_sources() {
        use crate::bundle_cook::{cook_bundle_bytes, demo_staged, verify_bundle_bytes};

        let bytes = cook_bundle_bytes(&demo_staged()).expect("demo cooks");
        let loaded = verify_bundle_bytes(&bytes, false).expect("bundle verifies");
        let mut manifest =
            Manifest::new(DEFAULT_SEED, DEFAULT_THREADS, PROFILE_POTATO).expect("manifest builds");
        let added = manifest.add_bundle(&loaded).expect("entries recorded");
        assert_eq!(added, loaded.len());
        assert_eq!(manifest.sources().count(), loaded.len());
        let paths: Vec<String> = manifest
            .sources()
            .map(|record| record.path().to_string())
            .collect();
        assert!(paths.iter().any(|path| path == "entries/mesh/demo_mesh"));
        assert!(paths.iter().any(|path| path == "entries/clip/demo_clip"));
        assert!(paths.windows(2).all(|pair| pair[0] < pair[1]));
    }
}
