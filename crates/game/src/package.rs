//! package: versioned Windows tree with the `arena` binary and asset bundle.
//!
//! Consumer: the release operator (clean-machine run) and quality review. Owner:
//! this module owns the tree under `dist_dir`. Units: bytes (`u64`), digests
//! (hex FNV-1a 64). Determinism: same profile plus same sources give identical
//! bytes and digests. Errors: [`PackageError`]; [`verify_clean`] reports a
//! [`CleanViolation`] per problem, [`verify_package`] fails loudly instead.
//!
//! Layout (`/`-separated names, joined portably so Linux proofs keep running):
//!
//! ```text
//! dist/
//!   bin/arena.exe              `arena` binary (`app` feature) or stub
//!   content/arena-bundle.bin   asset bundle (`E204BNDL` v1) or stub
//!   content/contents.txt       bundle listing (relative names only)
//!   saves/progress-schema-v1.txt  versioned saves schema descriptor
//!   launch.txt                 launch manifest (binary, bundle, window args)
//!   package-manifest.txt       headers (`STUB`/`REAL`) plus digest rows
//! ```
//!
//! Bundle authority stays in `engine-assets`: the packager checks bundle framing
//! (magic, version, entry frame) without decoding payloads, and records the
//! tree digest; `BundleLoader` verifies every blake3 digest at startup. No new
//! dependency edge is added here on purpose.
//!
//! Operator run: `cargo build --release -p engine-game --features app`, set
//! `E2040_PACKAGE_BIN` to the release `arena` exe and `E2040_PACKAGE_BUNDLE`
//! to the asset bundle, run the packager over an empty folder, then copy the
//! whole folder to the clean machine and run `bin/arena.exe` from inside it.
//! The automated tests simulate this with fresh TEMP dirs; the real
//! clean-machine run stays operator-verified.

use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use super::capture::fnv_hex as digest_hex;
use super::progress::{PROGRESS_BYTES, PROGRESS_VERSION};

/// Relative binary slot: the `arena` binary (`app` feature; `arena.exe` on
/// Windows, read back through the same portable join on Linux).
pub const BIN_REL: &str = "bin/arena.exe";
/// Relative asset content bundle payload (`E204BNDL` v1).
pub const BUNDLE_REL: &str = "content/arena-bundle.bin";
/// Relative bundle listing (relative names only, never absolute paths).
pub const CONTENTS_REL: &str = "content/contents.txt";
/// Relative versioned saves schema descriptor.
pub const SCHEMA_REL: &str = "saves/progress-schema-v1.txt";
/// Relative launch manifest (binary, bundle and window CLI, relative only).
pub const LAUNCH_REL: &str = "launch.txt";
/// Relative package manifest (headers plus `name bytes digest` rows, UTF-8).
pub const MANIFEST_REL: &str = "package-manifest.txt";
/// Environment variable naming the real release `arena` exe to stage. Unset
/// (or not a file) means the deterministic stub is staged and marked `STUB`.
pub const BINARY_ENV_VAR: &str = "E2040_PACKAGE_BIN";
/// Environment variable naming the real asset bundle file to stage. Unset (or
/// not a file) means the deterministic stub bundle is staged and marked
/// `STUB`.
pub const BUNDLE_ENV_VAR: &str = "E2040_PACKAGE_BUNDLE";
/// Game version stamped into staged files (the crate version, never a path).
pub const PACKAGE_VERSION: &str = env!("CARGO_PKG_VERSION");
/// Default smoke length in the launch manifest (`--frames N --exit`).
pub const SMOKE_FRAMES: u32 = 120;

/// Maximum files scanned by [`verify_clean`] (bounded buffers; anything more
/// reports [`CleanViolation::TooManyFiles`]).
pub const MAX_SCAN_FILES: usize = 4096;
/// Maximum bytes scanned per file (longer files scan the leading prefix).
pub const MAX_SCAN_BYTES: usize = 16 * 1024 * 1024;
/// Bound for echoed details carried in [`PackageError`].
pub const MAX_DETAIL_CHARS: usize = 128;
/// Asset bundle magic opening every bundle (`E204BNDL`; mirrors `engine-assets`
/// framing, which owns entry verification).
pub const BUNDLE_MAGIC: &[u8; 8] = b"E204BNDL";
/// Asset bundle schema version staged and accepted here.
pub const BUNDLE_VERSION: u32 = 1;
/// Most entries a staged bundle may declare (mirrors the asset bundle budget).
pub const MAX_BUNDLE_ENTRIES: usize = 4096;
/// Longest entry name accepted while walking the bundle frame, in bytes.
pub const MAX_BUNDLE_NAME_BYTES: usize = 512;
/// Largest single entry payload walked in the bundle frame, in bytes.
pub const MAX_BUNDLE_PAYLOAD_BYTES: usize = 64 * 1024 * 1024;
/// Largest whole bundle walked by the framing check, in bytes.
pub const MAX_BUNDLE_BYTES: usize = 512 * 1024 * 1024;

/// Packaging profile: which build variant the tree was assembled from.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum PackageProfile {
    /// Optimized release build (`cargo build --release`, `app` feature).
    Release,
}

impl PackageProfile {
    /// Stable profile name recorded in staged files and the manifest.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Release => "release",
        }
    }
}

impl core::str::FromStr for PackageProfile {
    type Err = PackageError;

    /// Parse a profile name; anything unknown fails with the name carried.
    fn from_str(name: &str) -> Result<Self, Self::Err> {
        match name {
            "release" => Ok(Self::Release),
            _ => Err(PackageError::UnknownProfile),
        }
    }
}

/// Packaging failures: bounded diagnostics, never panics. Missing pieces are
/// refused (never shipped silently); stub output is marked `STUB` in the
/// manifest instead.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum PackageError {
    /// Filesystem failure, kind carried (paths stay in the caller).
    Io(ErrorKind),
    /// Unknown profile name (only `release` exists today).
    UnknownProfile,
    /// The binary source named a path that is not a readable file, or the
    /// staged real binary was empty.
    BadBinarySource,
    /// Strict assembly required a real binary but none was given or found.
    MissingBinary,
    /// Strict assembly required a real bundle but none was given or found.
    MissingBundle,
    /// The bundle source was not readable.
    BadBundleSource,
    /// Bundle framing failure (bad magic, future version, truncated frame);
    /// bounded reason carried.
    BadBundle(String),
    /// Package manifest missing, unreadable or malformed; bounded reason.
    BadManifest(String),
    /// Launch manifest missing or malformed; bounded reason carried.
    BadLaunch(String),
    /// Staged bytes differ from the manifest row; digests carried.
    DigestMismatch {
        /// Portable `/`-separated artifact name.
        name: String,
        /// Hex digest recorded in the manifest.
        expected: String,
        /// Hex digest recomputed from the staged bytes.
        actual: String,
    },
}

impl core::fmt::Display for PackageError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Io(kind) => write!(f, "package io error ({kind:?})"),
            Self::UnknownProfile => write!(f, "unknown package profile (want \"release\")"),
            Self::BadBinarySource => write!(f, "binary source is not a readable file"),
            Self::MissingBinary => write!(f, "missing binary: refuse to ship without arena"),
            Self::MissingBundle => write!(f, "missing bundle: refuse to ship without AS1"),
            Self::BadBundleSource => write!(f, "bundle source is not a readable file"),
            Self::BadBundle(detail) => write!(f, "bad bundle ({detail})"),
            Self::BadManifest(detail) => write!(f, "bad manifest ({detail})"),
            Self::BadLaunch(detail) => write!(f, "bad launch manifest ({detail})"),
            Self::DigestMismatch {
                name,
                expected,
                actual,
            } => write!(
                f,
                "digest mismatch for {name} (want {expected}, got {actual})"
            ),
        }
    }
}

impl std::error::Error for PackageError {}

/// One shipped artifact: portable name, exact bytes, content digest.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct ArtifactRecord {
    /// Portable `/`-separated path relative to `dist_dir`.
    pub name: String,
    /// Exact file length in bytes.
    pub bytes: u64,
    /// Hex FNV-1a 64 over the exact file bytes (integrity, not signing).
    pub digest: String,
}

/// Manifest for one assembled tree: profile, stub markers, digest rows.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Manifest {
    /// Profile the tree was assembled from ([`PackageProfile::as_str`]).
    pub profile: String,
    /// Crate version staged with the tree.
    pub version: String,
    /// True when the binary slot holds the deterministic stub, not `arena`.
    pub binary_stub: bool,
    /// True when the bundle slot holds the stub, not the real asset bundle.
    pub bundle_stub: bool,
    /// One row per shipped artifact, in layout order.
    pub artifacts: Vec<ArtifactRecord>,
}

/// Parsed launch manifest: what the clean machine runs. Pure data.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct LaunchInfo {
    /// Profile the tree was assembled from.
    pub profile: String,
    /// True when the binary slot is the stub.
    pub binary_stub: bool,
    /// True when the bundle slot is the stub.
    pub bundle_stub: bool,
}

/// A clean-machine violation: one concrete problem in the tree.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum CleanViolation {
    /// Required binary slot missing or empty (`bin/arena.exe`).
    MissingBinary {
        /// Portable relative path expected.
        expected: String,
    },
    /// Required content bundle missing or empty.
    MissingContent {
        /// Portable relative path expected.
        expected: String,
    },
    /// Required saves schema descriptor missing or empty.
    MissingSchema {
        /// Portable relative path expected.
        expected: String,
    },
    /// Required launch manifest missing or empty.
    MissingLaunch {
        /// Portable relative path expected.
        expected: String,
    },
    /// Package manifest missing or empty.
    MissingManifest {
        /// Portable relative path expected.
        expected: String,
    },
    /// Absolute development path baked into a shipped file.
    AbsoluteDevPath {
        /// Portable relative path of the offending file.
        file: String,
        /// Matched marker (lowercased).
        marker: String,
    },
    /// Editor-only, model-account or development-path reference shipped.
    ForbiddenReference {
        /// Portable relative path of the offending file.
        file: String,
        /// Matched token (lowercased).
        marker: String,
    },
    /// A file under the tree could not be read.
    Unreadable {
        /// Portable relative path of the offending file.
        file: String,
    },
    /// More files than [`MAX_SCAN_FILES`] (scan stopped at the bound).
    TooManyFiles {
        /// Files scanned before stopping.
        scanned: usize,
    },
}

impl core::fmt::Display for CleanViolation {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::MissingBinary { expected } => write!(f, "missing binary ({expected})"),
            Self::MissingContent { expected } => write!(f, "missing content ({expected})"),
            Self::MissingSchema { expected } => write!(f, "missing saves schema ({expected})"),
            Self::MissingLaunch { expected } => write!(f, "missing launch ({expected})"),
            Self::MissingManifest { expected } => write!(f, "missing manifest ({expected})"),
            Self::AbsoluteDevPath { file, marker } => {
                write!(f, "absolute dev path in {file} ({marker})")
            }
            Self::ForbiddenReference { file, marker } => {
                write!(f, "forbidden reference in {file} ({marker})")
            }
            Self::Unreadable { file } => write!(f, "unreadable file ({file})"),
            Self::TooManyFiles { scanned } => write!(f, "too many files (scanned {scanned})"),
        }
    }
}

/// Absolute development-path markers (byte substrings, lowercased before
/// matching so `C:\` and `c:\` both fail). Drive-letter markers also occur
/// as x86-64 machine-code immediates no remap can remove, so they count
/// only with a path-shaped run (see [`drive_marker_shaped`]); longer
/// markers stay plain substrings.
const DEV_PATH_MARKERS: &[&str] = &[
    "c:\\",
    "c:/",
    "d:\\",
    "d:/",
    "e:\\",
    "e:/",
    "/home/",
    "/users/",
    "/opt/",
    "/mnt/",
    "/target/",
    "\\target\\",
    ".git/",
];

/// True for the six 3-byte drive-letter markers.
fn is_drive_marker(marker: &str) -> bool {
    matches!(marker, "c:\\" | "c:/" | "d:\\" | "d:/" | "e:\\" | "e:/")
}

/// Length of the path-shaped run starting at `at` (`X:\` or `X:/` plus
/// `[a-z0-9_.\-/\\]` chars), or 0 when no drive marker opens there.
/// ASCII-only on purpose: the lowercased scan text is UTF-8 and every
/// marker plus path char is ASCII, so byte indexing at an ASCII marker
/// is always a char boundary. Pure.
fn drive_run_len(text: &str, at: usize) -> usize {
    fn is_path_byte(byte: u8) -> bool {
        matches!(
            byte,
            b'a'..=b'z' | b'0'..=b'9' | b'_' | b'.' | b'-' | b'/' | b'\\'
        )
    }
    let bytes = text.as_bytes();
    if at.saturating_add(3) > bytes.len() {
        return 0;
    }
    if !bytes[at].is_ascii_lowercase() || bytes[at + 1] != b':' {
        return 0;
    }
    if bytes[at + 2] != b'\\' && bytes[at + 2] != b'/' {
        return 0;
    }
    let mut len = 3;
    while at + len < bytes.len() && is_path_byte(bytes[at + len]) {
        len += 1;
    }
    len
}

/// True when a drive-letter marker at `at` opens a genuine path: a run of
/// at least 8 chars holding another separator past the marker. Raw 3-byte
/// matches are x86-64 immediates (arena `44 3A 5C` at offset 394023 with
/// `18 FA` after it runs 3) or compressed-asset noise (`c:/9pgr` plus a
/// control byte runs 7); every genuine baked absolute path is a
/// manifest/registry dir join with more separators (the remapped editor
/// leak `c:\engine2040\.lanes\gm3\crates\editor../../contentrows.txt`
/// runs 59). Pure.
fn drive_marker_shaped(text: &str, at: usize) -> bool {
    let len = drive_run_len(text, at);
    if len < 8 {
        return false;
    }
    text.as_bytes()[at + 3..at + len]
        .iter()
        .any(|byte| *byte == b'/' || *byte == b'\\')
}
/// Editor-only / model-account / development-path reference tokens.
const FORBIDDEN_TOKENS: &[&str] = &["editor-only", "model-account", "development-path"];
/// Artifacts covered by the manifest, in layout order (the manifest file
/// itself is the record, never a row).
const MANIFESTED: &[&str] = &[BIN_REL, BUNDLE_REL, CONTENTS_REL, SCHEMA_REL, LAUNCH_REL];

/// Truncate echoed text to at most [`MAX_DETAIL_CHARS`] chars.
fn bound_detail(text: &str) -> String {
    text.chars().take(MAX_DETAIL_CHARS).collect()
}

// FNV-1a hex digest: reused from `super::capture::fnv_hex` (imported above
// as `digest_hex`, so manifest rows below read unchanged).

/// Join a portable `/`-separated relative name onto `dist_dir` portably
/// (split on `/`, push each segment, so Linux and Windows agree).
fn join_dist(dist_dir: &Path, rel: &str) -> PathBuf {
    let mut out = PathBuf::from(dist_dir);
    for segment in rel.split('/') {
        out.push(segment);
    }
    out
}

/// Portable `/`-separated name of `path` relative to `dist_dir`, or `None`
/// when `path` escapes the tree (never reported, never scanned).
fn rel_name(dist_dir: &Path, path: &Path) -> Option<String> {
    let rel = path.strip_prefix(dist_dir).ok()?;
    let mut name = String::new();
    for component in rel.components() {
        if !name.is_empty() {
            name.push('/');
        }
        name.push_str(component.as_os_str().to_str()?);
    }
    Some(name)
}

/// Deterministic stub binary payload: version plus profile, relative paths
/// only (never an absolute dev path, never a forbidden token). Pure. The
/// `kind:STUB` line keeps stub bytes self-describing; the manifest is the
/// record the verifier reads.
fn stub_binary(profile: PackageProfile) -> Vec<u8> {
    format!(
        "E2040ARENABIN\nversion:{PACKAGE_VERSION}\nprofile:{}\nrun:{BIN_REL}\nkind:STUB\n",
        profile.as_str()
    )
    .into_bytes()
}

/// Deterministic stub bundle: a valid but empty asset bundle (magic, version,
/// zero entries), so `BundleLoader` still parses it. Pure.
fn stub_bundle_bytes() -> Vec<u8> {
    let mut out = Vec::with_capacity(16);
    out.extend_from_slice(BUNDLE_MAGIC);
    out.extend_from_slice(&BUNDLE_VERSION.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out
}

fn read_u32_le(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

/// Check asset bundle framing without decoding payloads: magic, version, entry
/// count, then every entry frame (kind, name length, payload length, hex
/// digest) with truncation refused. Per-entry blake3 equality stays with
/// `BundleLoader` at startup. Pure.
fn validate_bundle_bytes(bytes: &[u8]) -> Result<usize, PackageError> {
    let bad = |detail: &str| PackageError::BadBundle(bound_detail(detail));
    if bytes.len() > MAX_BUNDLE_BYTES {
        return Err(bad("bundle too large"));
    }
    if bytes.len() < 16 {
        return Err(bad("bundle truncated: need 16 header bytes"));
    }
    if &bytes[..8] != BUNDLE_MAGIC {
        return Err(bad("bundle bad magic: want E204BNDL"));
    }
    let version = read_u32_le(bytes, 8);
    if version > BUNDLE_VERSION {
        return Err(PackageError::BadBundle(bound_detail(&format!(
            "bundle version {version} is newer than v{BUNDLE_VERSION}",
        ))));
    }
    if version != BUNDLE_VERSION {
        return Err(PackageError::BadBundle(bound_detail(&format!(
            "unsupported bundle version {version}",
        ))));
    }
    let count = read_u32_le(bytes, 12) as usize;
    if count > MAX_BUNDLE_ENTRIES {
        return Err(bad("bundle too many entries"));
    }
    let mut pos = 16usize;
    for _ in 0..count {
        // Kind byte plus name length.
        if pos.saturating_add(3) > bytes.len() {
            return Err(bad("bundle truncated mid-frame"));
        }
        let kind = bytes[pos];
        if !matches!(kind, 1..=5) {
            return Err(bad("bundle corrupt: unknown entry kind"));
        }
        let name_len = usize::from(u16::from_le_bytes([bytes[pos + 1], bytes[pos + 2]]));
        if name_len == 0 || name_len > MAX_BUNDLE_NAME_BYTES {
            return Err(bad("bundle corrupt: entry name out of range"));
        }
        pos = pos.saturating_add(3);
        // Name bytes plus payload length.
        if pos.saturating_add(name_len.saturating_add(4)) > bytes.len() {
            return Err(bad("bundle truncated mid-frame"));
        }
        if std::str::from_utf8(&bytes[pos..pos.saturating_add(name_len)]).is_err() {
            return Err(bad("bundle corrupt: entry name not utf-8"));
        }
        pos = pos.saturating_add(name_len);
        let payload_len = read_u32_le(bytes, pos) as usize;
        if payload_len > MAX_BUNDLE_PAYLOAD_BYTES {
            return Err(bad("bundle entry too large"));
        }
        pos = pos.saturating_add(4);
        // Payload bytes plus the 64-byte hex digest.
        if pos.saturating_add(payload_len.saturating_add(64)) > bytes.len() {
            return Err(bad("bundle truncated mid-frame"));
        }
        let digest = &bytes
            [pos.saturating_add(payload_len)..pos.saturating_add(payload_len).saturating_add(64)];
        if digest.iter().any(|byte| !byte.is_ascii_hexdigit()) {
            return Err(bad("bundle corrupt: entry digest not hex"));
        }
        pos = pos.saturating_add(payload_len.saturating_add(64));
    }
    if pos != bytes.len() {
        return Err(bad("bundle corrupt: trailing bytes"));
    }
    Ok(count)
}

/// Bundle listing: relative names only. Pure.
fn contents_listing() -> Vec<u8> {
    format!("{BUNDLE_REL}\n{CONTENTS_REL}\n").into_bytes()
}

/// Versioned saves schema descriptor: progression payload version and exact
/// byte length (see [`progress`](super::progress)). Pure.
fn schema_descriptor() -> Vec<u8> {
    format!("progress-schema v{PROGRESS_VERSION} bytes {PROGRESS_BYTES} little-endian\n")
        .into_bytes()
}

/// Stub-or-real marker recorded in staged files. Pure.
fn stub_marker(stub: bool) -> &'static str {
    if stub {
        "STUB"
    } else {
        "REAL"
    }
}

/// Launch manifest bytes: version, profile, binary, bundle, stub markers and
/// the window CLI (`--frames N --exit` smoke; empty `args:` means interactive
/// play with the arena defaults). Relative paths only. Pure.
fn launch_descriptor(profile: PackageProfile, binary_stub: bool, bundle_stub: bool) -> Vec<u8> {
    format!(
        "version:{PACKAGE_VERSION}\nprofile:{}\nbinary:{BIN_REL}\nbundle:{BUNDLE_REL}\n\
         binary_kind:{}\nbundle_kind:{}\nargs:\nsmoke:{BIN_REL} --frames {SMOKE_FRAMES} --exit\n",
        profile.as_str(),
        stub_marker(binary_stub),
        stub_marker(bundle_stub),
    )
    .into_bytes()
}

/// Parse launch manifest bytes into [`LaunchInfo`]; anything malformed fails
/// loudly with a bounded reason. Pure.
fn parse_launch(bytes: &[u8]) -> Result<LaunchInfo, PackageError> {
    let bad = |detail: &str| PackageError::BadLaunch(bound_detail(detail));
    let text = std::str::from_utf8(bytes).map_err(|_| bad("launch not utf-8"))?;
    let lines: Vec<&str> = text.lines().collect();
    if lines.len() < 8 {
        return Err(bad("launch truncated"));
    }
    let version = lines[0]
        .strip_prefix("version:")
        .ok_or_else(|| bad("launch version row"))?;
    if version != PACKAGE_VERSION {
        return Err(bad("launch version mismatch"));
    }
    let profile = lines[1]
        .strip_prefix("profile:")
        .ok_or_else(|| bad("launch profile row"))?;
    if profile.parse::<PackageProfile>().is_err() {
        return Err(bad("launch unknown profile"));
    }
    let binary = lines[2]
        .strip_prefix("binary:")
        .ok_or_else(|| bad("launch binary row"))?;
    if binary != BIN_REL {
        return Err(bad("launch binary mismatch"));
    }
    let bundle = lines[3]
        .strip_prefix("bundle:")
        .ok_or_else(|| bad("launch bundle row"))?;
    if bundle != BUNDLE_REL {
        return Err(bad("launch bundle mismatch"));
    }
    let binary_kind = lines[4]
        .strip_prefix("binary_kind:")
        .ok_or_else(|| bad("launch binary_kind row"))?;
    let bundle_kind = lines[5]
        .strip_prefix("bundle_kind:")
        .ok_or_else(|| bad("launch bundle_kind row"))?;
    if !matches!(binary_kind, "STUB" | "REAL") || !matches!(bundle_kind, "STUB" | "REAL") {
        return Err(bad("launch stub markers"));
    }
    if lines[6] != "args:" {
        return Err(bad("launch args row"));
    }
    let smoke = lines[7]
        .strip_prefix("smoke:")
        .ok_or_else(|| bad("launch smoke row"))?;
    if !smoke.starts_with(BIN_REL) || !smoke.contains("--frames") || !smoke.contains("--exit") {
        return Err(bad("launch smoke command"));
    }
    Ok(LaunchInfo {
        profile: profile.to_owned(),
        binary_stub: binary_kind == "STUB",
        bundle_stub: bundle_kind == "STUB",
    })
}

/// Manifest file bytes: headers (profile, version, `STUB`/`REAL` markers)
/// plus one `name bytes digest` row per artifact. Pure.
fn manifest_bytes(manifest: &Manifest) -> Vec<u8> {
    let mut out = format!(
        "profile:{}\nversion:{}\nbinary:{}\nbundle:{}\n",
        manifest.profile,
        manifest.version,
        stub_marker(manifest.binary_stub),
        stub_marker(manifest.bundle_stub),
    )
    .into_bytes();
    for artifact in &manifest.artifacts {
        out.extend_from_slice(
            format!("{} {} {}\n", artifact.name, artifact.bytes, artifact.digest).as_bytes(),
        );
    }
    out
}

/// Parse manifest bytes back into [`Manifest`]; rows must name every staged
/// artifact in layout order. Pure.
fn parse_manifest(bytes: &[u8]) -> Result<Manifest, PackageError> {
    let bad = |detail: &str| PackageError::BadManifest(bound_detail(detail));
    let text = std::str::from_utf8(bytes).map_err(|_| bad("manifest not utf-8"))?;
    let lines: Vec<&str> = text.lines().collect();
    if lines.len() < 4 + MANIFESTED.len() {
        return Err(bad("manifest truncated"));
    }
    let profile = lines[0]
        .strip_prefix("profile:")
        .ok_or_else(|| bad("manifest profile row"))?;
    if profile.parse::<PackageProfile>().is_err() {
        return Err(bad("manifest unknown profile"));
    }
    let version = lines[1]
        .strip_prefix("version:")
        .ok_or_else(|| bad("manifest version row"))?;
    if version != PACKAGE_VERSION {
        return Err(bad("manifest version mismatch"));
    }
    let binary_kind = lines[2]
        .strip_prefix("binary:")
        .ok_or_else(|| bad("manifest binary row"))?;
    let bundle_kind = lines[3]
        .strip_prefix("bundle:")
        .ok_or_else(|| bad("manifest bundle row"))?;
    if !matches!(binary_kind, "STUB" | "REAL") || !matches!(bundle_kind, "STUB" | "REAL") {
        return Err(bad("manifest stub markers"));
    }
    let mut artifacts = Vec::with_capacity(MANIFESTED.len());
    for (line, want) in lines[4..].iter().zip(MANIFESTED.iter()) {
        let mut parts = line.split_whitespace();
        let name = parts.next().ok_or_else(|| bad("manifest row shape"))?;
        let count = parts.next().ok_or_else(|| bad("manifest row shape"))?;
        let digest = parts.next().ok_or_else(|| bad("manifest row shape"))?;
        if parts.next().is_some() {
            return Err(bad("manifest row shape"));
        }
        if name != *want {
            return Err(bad("manifest layout mismatch"));
        }
        let count: u64 = count.parse().map_err(|_| bad("manifest bad byte count"))?;
        if digest.len() != 16 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(bad("manifest bad digest"));
        }
        artifacts.push(ArtifactRecord {
            name: name.to_owned(),
            bytes: count,
            digest: digest.to_owned(),
        });
    }
    if lines.len() != 4 + MANIFESTED.len() {
        return Err(bad("manifest trailing rows"));
    }
    Ok(Manifest {
        profile: profile.to_owned(),
        version: version.to_owned(),
        binary_stub: binary_kind == "STUB",
        bundle_stub: bundle_kind == "STUB",
        artifacts,
    })
}

/// Write `bytes` to `dist_dir/rel`, creating parent directories. The write is
/// atomic enough for packaging: parents are created, then the file is fully
/// written (a failed write returns [`PackageError::Io`]).
fn stage_file(dist_dir: &Path, rel: &str, bytes: &[u8]) -> Result<(), PackageError> {
    let path = join_dist(dist_dir, rel);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| PackageError::Io(err.kind()))?;
    }
    std::fs::write(&path, bytes).map_err(|err| PackageError::Io(err.kind()))?;
    Ok(())
}

/// Assemble the distributable tree under `dist_dir` from `profile`: stages
/// the `arena` release binary (copied from `E2040_PACKAGE_BIN` when it names
/// a file, deterministic stub otherwise), the asset content bundle (copied from
/// `E2040_PACKAGE_BUNDLE` when it names a file, stub otherwise), the bundle
/// listing, the versioned saves schema, the launch manifest and the package
/// manifest. Stub slots are marked `STUB` in the manifest, never silent.
/// Missing parents are created; staged files are overwritten deterministically.
pub fn assemble(dist_dir: &Path, profile: PackageProfile) -> Result<Manifest, PackageError> {
    let binary_source = std::env::var_os(BINARY_ENV_VAR)
        .map(PathBuf::from)
        .filter(|path| path.is_file());
    let bundle_source = std::env::var_os(BUNDLE_ENV_VAR)
        .map(PathBuf::from)
        .filter(|path| path.is_file());
    assemble_full(
        dist_dir,
        profile,
        binary_source.as_deref(),
        bundle_source.as_deref(),
    )
}

/// [`assemble`] with explicit sources (`None` stages that slot's stub and
/// marks it `STUB`). Tests use this to stay hermetic instead of touching the
/// process environment; the operator path goes through [`assemble`].
/// A real bundle source is framing-checked and refused when corrupt.
pub fn assemble_full(
    dist_dir: &Path,
    profile: PackageProfile,
    binary_source: Option<&Path>,
    bundle_source: Option<&Path>,
) -> Result<Manifest, PackageError> {
    let (binary, binary_stub) = match binary_source {
        Some(source) => (
            std::fs::read(source).map_err(|_| PackageError::BadBinarySource)?,
            false,
        ),
        None => (stub_binary(profile), true),
    };
    if !binary_stub && binary.is_empty() {
        return Err(PackageError::BadBinarySource);
    }
    let (bundle, bundle_stub) = match bundle_source {
        Some(source) => {
            let bytes = std::fs::read(source).map_err(|_| PackageError::BadBundleSource)?;
            validate_bundle_bytes(&bytes)?;
            (bytes, false)
        }
        None => (stub_bundle_bytes(), true),
    };
    let listing = contents_listing();
    let schema = schema_descriptor();
    let launch = launch_descriptor(profile, binary_stub, bundle_stub);
    stage_file(dist_dir, BIN_REL, &binary)?;
    stage_file(dist_dir, BUNDLE_REL, &bundle)?;
    stage_file(dist_dir, CONTENTS_REL, &listing)?;
    stage_file(dist_dir, SCHEMA_REL, &schema)?;
    stage_file(dist_dir, LAUNCH_REL, &launch)?;
    let mut manifest = Manifest {
        profile: profile.as_str().to_owned(),
        version: PACKAGE_VERSION.to_owned(),
        binary_stub,
        bundle_stub,
        artifacts: Vec::with_capacity(MANIFESTED.len()),
    };
    for (rel, bytes) in [
        (BIN_REL, binary.as_slice()),
        (BUNDLE_REL, bundle.as_slice()),
        (CONTENTS_REL, listing.as_slice()),
        (SCHEMA_REL, schema.as_slice()),
        (LAUNCH_REL, launch.as_slice()),
    ] {
        manifest.artifacts.push(ArtifactRecord {
            name: rel.to_owned(),
            bytes: bytes.len() as u64,
            digest: digest_hex(bytes),
        });
    }
    let manifest_file = manifest_bytes(&manifest);
    stage_file(dist_dir, MANIFEST_REL, &manifest_file)?;
    Ok(manifest)
}

/// Strict assembly for the release run: both sources must name readable
/// files, otherwise [`PackageError::MissingBinary`] or
/// [`PackageError::MissingBundle`] refuses before anything is staged.
/// Corrupt bundles are refused by framing; empty binaries are refused.
pub fn assemble_strict(
    dist_dir: &Path,
    profile: PackageProfile,
    binary_source: &Path,
    bundle_source: &Path,
) -> Result<Manifest, PackageError> {
    if !binary_source.is_file() {
        return Err(PackageError::MissingBinary);
    }
    if !bundle_source.is_file() {
        return Err(PackageError::MissingBundle);
    }
    assemble_full(dist_dir, profile, Some(binary_source), Some(bundle_source))
}

/// [`assemble`] with an explicit binary source (`None` stages the stub).
/// Hermetic helper kept for the operator path; new code prefers
/// [`assemble_full`], which also takes the bundle source.
#[cfg(test)]
fn assemble_inner(
    dist_dir: &Path,
    profile: PackageProfile,
    binary_source: Option<&Path>,
) -> Result<Manifest, PackageError> {
    assemble_full(dist_dir, profile, binary_source, None)
}

/// Read a required staged file; missing or empty fails loudly with the
/// typed error for that slot.
fn read_required(
    dist_dir: &Path,
    rel: &str,
    missing: PackageError,
    corrupt_detail: &str,
) -> Result<Vec<u8>, PackageError> {
    let path = join_dist(dist_dir, rel);
    let bytes = std::fs::read(&path).map_err(|err| {
        if err.kind() == ErrorKind::NotFound {
            missing
        } else {
            PackageError::Io(err.kind())
        }
    })?;
    if bytes.is_empty() {
        return Err(PackageError::BadManifest(bound_detail(corrupt_detail)));
    }
    Ok(bytes)
}

/// Verify `dist_dir` is shippable: every artifact present and non-empty, the
/// manifest parses and matches the staged bytes row for row, the bundle
/// framing holds, and the launch manifest names the staged binary and bundle
/// with agreeing stub markers. Returns the verified manifest. The REAL
/// clean-machine run is operator-verified afterward — automated tests simulate
/// it with a fresh TEMP dir plus [`verify_clean`].
pub fn verify_package(dist_dir: &Path) -> Result<Manifest, PackageError> {
    let binary = read_required(
        dist_dir,
        BIN_REL,
        PackageError::MissingBinary,
        "binary slot empty",
    )?;
    let bundle = read_required(
        dist_dir,
        BUNDLE_REL,
        PackageError::MissingBundle,
        "bundle slot empty",
    )?;
    let contents = read_required(
        dist_dir,
        CONTENTS_REL,
        PackageError::BadManifest(bound_detail("missing content/contents.txt")),
        "contents listing empty",
    )?;
    let schema = read_required(
        dist_dir,
        SCHEMA_REL,
        PackageError::BadManifest(bound_detail("missing saves schema")),
        "saves schema empty",
    )?;
    let launch = std::fs::read(join_dist(dist_dir, LAUNCH_REL)).map_err(|err| {
        if err.kind() == ErrorKind::NotFound {
            PackageError::BadLaunch(bound_detail("missing launch.txt"))
        } else {
            PackageError::Io(err.kind())
        }
    })?;
    if launch.is_empty() {
        return Err(PackageError::BadLaunch(bound_detail("launch.txt empty")));
    }
    let manifest_raw = std::fs::read(join_dist(dist_dir, MANIFEST_REL)).map_err(|err| {
        if err.kind() == ErrorKind::NotFound {
            PackageError::BadManifest(bound_detail("missing package-manifest.txt"))
        } else {
            PackageError::Io(err.kind())
        }
    })?;
    if manifest_raw.is_empty() {
        return Err(PackageError::BadManifest(bound_detail("manifest empty")));
    }
    let manifest = parse_manifest(&manifest_raw)?;
    let staged: &[(&str, &[u8])] = &[
        (BIN_REL, &binary),
        (BUNDLE_REL, &bundle),
        (CONTENTS_REL, &contents),
        (SCHEMA_REL, &schema),
        (LAUNCH_REL, &launch),
    ];
    for (artifact, (_, bytes)) in manifest.artifacts.iter().zip(staged.iter()) {
        if artifact.bytes != bytes.len() as u64 || artifact.digest != digest_hex(bytes) {
            return Err(PackageError::DigestMismatch {
                name: artifact.name.clone(),
                expected: artifact.digest.clone(),
                actual: digest_hex(bytes),
            });
        }
    }
    // A swapped-in bundle with matching digests still has to be an asset bundle frame.
    validate_bundle_bytes(&bundle)?;
    let launch_info = parse_launch(&launch)?;
    if launch_info.binary_stub != manifest.binary_stub
        || launch_info.bundle_stub != manifest.bundle_stub
    {
        return Err(PackageError::BadLaunch(bound_detail(
            "launch stub markers disagree with manifest",
        )));
    }
    Ok(manifest)
}

/// Verify `dist_dir` is clean-machine safe: required artifacts present and
/// non-empty, and no shipped file contains an absolute development path or a
/// forbidden (editor-only / model-account / development-path) reference.
/// Returns every violation found; an empty vector means clean. Never fails
/// itself: unreadable trees report violations instead of panicking. The REAL
/// clean-machine run is operator-verified afterward — automated tests simulate
/// it with a fresh TEMP dir.
pub fn verify_clean(dist_dir: &Path) -> Vec<CleanViolation> {
    let mut violations = Vec::new();
    for (rel, kind) in [
        (BIN_REL, "binary"),
        (BUNDLE_REL, "content"),
        (CONTENTS_REL, "content"),
        (SCHEMA_REL, "schema"),
        (LAUNCH_REL, "launch"),
        (MANIFEST_REL, "manifest"),
    ] {
        let path = join_dist(dist_dir, rel);
        let present = std::fs::metadata(&path).map(|meta| meta.len()).unwrap_or(0);
        if present == 0 {
            violations.push(match kind {
                "binary" => CleanViolation::MissingBinary {
                    expected: rel.to_owned(),
                },
                "content" => CleanViolation::MissingContent {
                    expected: rel.to_owned(),
                },
                "schema" => CleanViolation::MissingSchema {
                    expected: rel.to_owned(),
                },
                "launch" => CleanViolation::MissingLaunch {
                    expected: rel.to_owned(),
                },
                _ => CleanViolation::MissingManifest {
                    expected: rel.to_owned(),
                },
            });
        }
    }
    violations.extend(scan_tree(dist_dir));
    violations
}

/// Recursively scan every file under `dist_dir` for dev-path and forbidden
/// markers. Bounded: stops collecting file contents past [`MAX_SCAN_FILES`]
/// with [`CleanViolation::TooManyFiles`], and scans at most
/// [`MAX_SCAN_BYTES`] per file. Paths escaping the tree are skipped.
fn scan_tree(dist_dir: &Path) -> Vec<CleanViolation> {
    let mut violations = Vec::new();
    let mut stack = vec![PathBuf::from(dist_dir)];
    let mut scanned = 0usize;
    while let Some(dir) = stack.pop() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(_) => {
                if dir.as_path() == dist_dir {
                    return violations;
                }
                continue;
            }
        };
        for entry in entries {
            let Ok(entry) = entry else { continue };
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if !path.is_file() {
                continue;
            }
            let Some(name) = rel_name(dist_dir, &path) else {
                continue;
            };
            scanned += 1;
            if scanned > MAX_SCAN_FILES {
                violations.push(CleanViolation::TooManyFiles {
                    scanned: scanned - 1,
                });
                return violations;
            }
            let bytes = match std::fs::read(&path) {
                Ok(bytes) => bytes,
                Err(_) => {
                    violations.push(CleanViolation::Unreadable { file: name });
                    continue;
                }
            };
            let window = if bytes.len() > MAX_SCAN_BYTES {
                &bytes[..MAX_SCAN_BYTES]
            } else {
                &*bytes
            };
            let text = String::from_utf8_lossy(window).to_lowercase();
            let mut flagged = false;
            for marker in DEV_PATH_MARKERS {
                if !is_drive_marker(marker) {
                    if text.contains(marker) {
                        violations.push(CleanViolation::AbsoluteDevPath {
                            file: name.clone(),
                            marker: (*marker).to_owned(),
                        });
                        flagged = true;
                        break;
                    }
                    continue;
                }
                let mut search_from = 0;
                while let Some(rel) = text[search_from..].find(marker) {
                    let at = search_from + rel;
                    if drive_marker_shaped(&text, at) {
                        violations.push(CleanViolation::AbsoluteDevPath {
                            file: name.clone(),
                            marker: (*marker).to_owned(),
                        });
                        flagged = true;
                        break;
                    }
                    search_from = at + 1;
                }
                if flagged {
                    break;
                }
            }
            if flagged {
                continue;
            }
            for token in FORBIDDEN_TOKENS {
                if text.contains(token) {
                    violations.push(CleanViolation::ForbiddenReference {
                        file: name.clone(),
                        marker: (*token).to_owned(),
                    });
                    break;
                }
            }
        }
    }
    violations
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

    /// Fresh TEMP dir per test (simulates the clean machine; unique per
    /// test so parallel tests never share a tree).
    fn fresh_temp_dir(tag: &str) -> PathBuf {
        let id = NEXT_DIR.fetch_add(1, Ordering::SeqCst);
        let dir =
            std::env::temp_dir().join(format!("e2040-package-{tag}-{}-{id}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Valid asset bundle fixture: the empty bundle (magic, v1, zero entries),
    /// which `BundleLoader` parses. Bundles with entries come from the asset
    /// pipeline through `E2040_PACKAGE_BUNDLE`.
    fn bundle_fixture() -> Vec<u8> {
        stub_bundle_bytes()
    }

    /// Write a source file outside the dist tree (keeps the packaged scan
    /// hermetic) and return its path.
    fn write_source(dir: &Path, name: &str, bytes: &[u8]) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, bytes).unwrap();
        path
    }

    fn recompute(path: &Path) -> (u64, String) {
        let bytes = std::fs::read(path).unwrap();
        let digest = digest_hex(&bytes);
        (bytes.len() as u64, digest)
    }

    #[test]
    fn assemble_then_verify_clean_passes_on_a_fresh_temp_dir() {
        let dir = fresh_temp_dir("clean");
        let manifest = assemble_inner(&dir, PackageProfile::Release, None).unwrap();
        assert_eq!(manifest.profile, "release");
        assert_eq!(manifest.artifacts.len(), 5);
        // Stub output is marked, never silent.
        assert!(manifest.binary_stub && manifest.bundle_stub);
        let raw = std::fs::read(join_dist(&dir, MANIFEST_REL)).unwrap();
        let text = String::from_utf8(raw).unwrap();
        assert!(text.contains("STUB"), "stub must be marked: {text}");
        // Layout, digests and launch all verify strictly.
        let verified = verify_package(&dir).unwrap();
        assert_eq!(verified, manifest);
        assert_eq!(verify_clean(&dir), Vec::new());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn manifest_digests_match_recomputed_bytes() {
        let dir = fresh_temp_dir("digests");
        let manifest = assemble_inner(&dir, PackageProfile::Release, None).unwrap();
        assert!(!manifest.artifacts.is_empty());
        for artifact in &manifest.artifacts {
            let (bytes, digest) = recompute(&join_dist(&dir, &artifact.name));
            assert_eq!(bytes, artifact.bytes, "byte count for {}", artifact.name);
            assert_eq!(digest, artifact.digest, "digest for {}", artifact.name);
        }
        // One changed byte changes the recorded digest.
        let first = &manifest.artifacts[0];
        let mut tampered = std::fs::read(join_dist(&dir, &first.name)).unwrap();
        tampered[0] ^= 0xFF;
        assert_ne!(digest_hex(&tampered), first.digest);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn planted_absolute_path_artifact_fails_verification() {
        let dir = fresh_temp_dir("abspath");
        assemble_inner(&dir, PackageProfile::Release, None).unwrap();
        std::fs::write(
            join_dist(&dir, "content/evil.txt"),
            b"debug notes at C:\\dev\\checkout\\target\\build",
        )
        .unwrap();
        let violations = verify_clean(&dir);
        assert!(
            violations.iter().any(|violation| matches!(
                violation,
                CleanViolation::AbsoluteDevPath { file, .. } if file == "content/evil.txt"
            )),
            "planted absolute path must fail: {violations:?}"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn machine_code_immediates_do_not_flag_but_real_paths_do() {
        let dir = fresh_temp_dir("shaped");
        assemble_inner(&dir, PackageProfile::Release, None).unwrap();
        // Byte-exact x86-64 immediate context from the remapped arena
        // (`D:\` file offset 394023 followed by `18 FA`): a raw 3-byte
        // drive match without a path-shaped run must stay silent.
        std::fs::write(
            join_dist(&dir, "content/code.bin"),
            [0x44u8, 0x31, 0xFF, 0x44, 0x3A, 0x5C, 0x18, 0xFA, 0x41],
        )
        .unwrap();
        assert_eq!(verify_clean(&dir), Vec::new());
        // A genuine baked absolute path (manifest dir join) still flags.
        std::fs::write(
            join_dist(&dir, "content/evil.txt"),
            b"root C:\\engine2040\\.lanes\\gm3\\crates\\editor\\..\\..\\x",
        )
        .unwrap();
        let violations = verify_clean(&dir);
        assert!(
            violations.iter().any(|violation| matches!(
                violation,
                CleanViolation::AbsoluteDevPath { file, .. } if file == "content/evil.txt"
            )),
            "shaped absolute path must fail: {violations:?}"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn non_c_dev_paths_rejected_consistently() {
        let dir = fresh_temp_dir("nonc");
        assemble_inner(&dir, PackageProfile::Release, None).unwrap();
        // One planted file per non-C: marker: an `E:` drive join plus the
        // Unix dev roots. Uppercase `E:` proves the lowercase scan catches
        // every case, and every drive join is path-shaped (long run with
        // more separators) so the shaped gate fires.
        for (name, bytes) in [
            (
                "content/e.txt",
                b"bake E:\\dev\\checkout\\content\\rows.txt end".as_slice(),
            ),
            (
                "content/opt.txt",
                b"bake /opt/engine2040/content/rows.txt end".as_slice(),
            ),
            (
                "content/mnt.txt",
                b"bake /mnt/data/engine2040/content/rows.txt end".as_slice(),
            ),
            (
                "content/home.txt",
                b"bake /home/dev/engine2040/content/rows.txt end".as_slice(),
            ),
        ] {
            std::fs::write(join_dist(&dir, name), bytes).unwrap();
        }
        let violations = verify_clean(&dir);
        for file in [
            "content/e.txt",
            "content/opt.txt",
            "content/mnt.txt",
            "content/home.txt",
        ] {
            assert!(
                violations.iter().any(|violation| matches!(
                    violation,
                    CleanViolation::AbsoluteDevPath { file: flagged, .. } if flagged == file
                )),
                "non-C dev path must fail for {file}: {violations:?}"
            );
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn planted_forbidden_reference_fails_verification() {
        let dir = fresh_temp_dir("forbidden");
        assemble_inner(&dir, PackageProfile::Release, None).unwrap();
        std::fs::write(
            join_dist(&dir, "content/notes.txt"),
            b"needs an editor-only model-account key and a development-path fix",
        )
        .unwrap();
        let violations = verify_clean(&dir);
        assert!(
            violations.iter().any(|violation| matches!(
                violation,
                CleanViolation::ForbiddenReference { file, .. } if file == "content/notes.txt"
            )),
            "planted forbidden reference must fail: {violations:?}"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn missing_artifacts_are_named() {
        let dir = fresh_temp_dir("missing");
        assemble_inner(&dir, PackageProfile::Release, None).unwrap();
        std::fs::remove_file(join_dist(&dir, BIN_REL)).unwrap();
        std::fs::remove_file(join_dist(&dir, BUNDLE_REL)).unwrap();
        let violations = verify_clean(&dir);
        assert!(
            violations.contains(&CleanViolation::MissingBinary {
                expected: BIN_REL.to_owned(),
            }),
            "missing binary must be named: {violations:?}"
        );
        assert!(
            violations.contains(&CleanViolation::MissingContent {
                expected: BUNDLE_REL.to_owned(),
            }),
            "missing content must be named: {violations:?}"
        );
        // The strict verifier refuses loudly, naming the binary first.
        assert_eq!(
            verify_package(&dir).unwrap_err(),
            PackageError::MissingBinary
        );
        // A missing tree reports violations instead of panicking.
        let gone = dir.join("no-such-tree");
        assert!(!verify_clean(&gone).is_empty());
        assert!(verify_package(&gone).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn explicit_binary_source_is_copied_byte_for_byte() {
        let dir = fresh_temp_dir("binsrc");
        let source = dir.join("real-exe.bin");
        std::fs::write(&source, b"REALRELEASEBINARY").unwrap();
        let manifest = assemble_inner(&dir, PackageProfile::Release, Some(&source)).unwrap();
        let staged = std::fs::read(join_dist(&dir, BIN_REL)).unwrap();
        assert_eq!(staged, b"REALRELEASEBINARY");
        let binary = manifest
            .artifacts
            .iter()
            .find(|artifact| artifact.name == BIN_REL)
            .unwrap();
        assert_eq!(
            (binary.bytes, &binary.digest),
            (17, &digest_hex(b"REALRELEASEBINARY"))
        );
        // Per-slot markers: real binary, stub bundle.
        assert!(!manifest.binary_stub);
        assert!(manifest.bundle_stub);
        assert_eq!(verify_package(&dir).unwrap(), manifest);
        assert_eq!(verify_clean(&dir), Vec::new());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn real_binary_and_bundle_verify_as_real() {
        let sources = fresh_temp_dir("realsrc");
        let dist = fresh_temp_dir("realdist");
        let binary = write_source(&sources, "arena.exe", b"REALARENA96BYTES...........");
        let bundle = write_source(&sources, "arena-bundle.bin", &bundle_fixture());
        let manifest =
            assemble_full(&dist, PackageProfile::Release, Some(&binary), Some(&bundle)).unwrap();
        assert!(!manifest.binary_stub && !manifest.bundle_stub);
        // No STUB marker anywhere in a real manifest.
        let raw = std::fs::read(join_dist(&dist, MANIFEST_REL)).unwrap();
        let text = String::from_utf8(raw).unwrap();
        assert!(
            !text.contains("STUB"),
            "real tree must not say STUB: {text}"
        );
        assert!(text.contains("REAL"));
        // Launch names the arena slot, the bundle and the window smoke args.
        let launch = std::fs::read(join_dist(&dist, LAUNCH_REL)).unwrap();
        let launch_text = String::from_utf8(launch).unwrap();
        assert!(launch_text.contains(BIN_REL));
        assert!(launch_text.contains(BUNDLE_REL));
        assert!(launch_text.contains("--frames") && launch_text.contains("--exit"));
        let info = parse_launch(&std::fs::read(join_dist(&dist, LAUNCH_REL)).unwrap()).unwrap();
        assert!(!info.binary_stub && !info.bundle_stub);
        // Strict layout, digest and launch checks all pass.
        assert_eq!(verify_package(&dist).unwrap(), manifest);
        assert_eq!(verify_clean(&dist), Vec::new());
        std::fs::remove_dir_all(&sources).unwrap();
        std::fs::remove_dir_all(&dist).unwrap();
    }

    #[test]
    fn strict_assembly_refuses_missing_sources() {
        let sources = fresh_temp_dir("strictsrc");
        let dist = fresh_temp_dir("strictdist");
        let binary = write_source(&sources, "arena.exe", b"REALARENA");
        let bundle = write_source(&sources, "arena-bundle.bin", &bundle_fixture());
        // Both missing: the binary is named first.
        assert_eq!(
            assemble_strict(
                &dist,
                PackageProfile::Release,
                &sources.join("no-bin.exe"),
                &sources.join("no-bundle.bin"),
            )
            .unwrap_err(),
            PackageError::MissingBinary
        );
        // Real binary but missing bundle: the bundle is refused.
        assert_eq!(
            assemble_strict(
                &dist,
                PackageProfile::Release,
                &binary,
                &sources.join("no-bundle.bin"),
            )
            .unwrap_err(),
            PackageError::MissingBundle
        );
        // Both present: the real tree assembles.
        let manifest = assemble_strict(&dist, PackageProfile::Release, &binary, &bundle).unwrap();
        assert!(!manifest.binary_stub && !manifest.bundle_stub);
        assert_eq!(verify_package(&dist).unwrap(), manifest);
        std::fs::remove_dir_all(&sources).unwrap();
        std::fs::remove_dir_all(&dist).unwrap();
    }

    #[test]
    fn corrupt_bundles_are_refused() {
        let sources = fresh_temp_dir("badsrc");
        let dist = fresh_temp_dir("baddist");
        let binary = write_source(&sources, "arena.exe", b"REALARENA");
        // Bad magic.
        let mut bad_magic = bundle_fixture();
        bad_magic[0] ^= 0xFF;
        let bad = write_source(&sources, "bad-magic.bin", &bad_magic);
        assert!(matches!(
            assemble_full(&dist, PackageProfile::Release, Some(&binary), Some(&bad)).unwrap_err(),
            PackageError::BadBundle(_)
        ));
        // Future version.
        let mut future = bundle_fixture();
        future[8..12].copy_from_slice(&2u32.to_le_bytes());
        let future_path = write_source(&sources, "future.bin", &future);
        assert!(matches!(
            assemble_full(
                &dist,
                PackageProfile::Release,
                Some(&binary),
                Some(&future_path)
            )
            .unwrap_err(),
            PackageError::BadBundle(_)
        ));
        // Truncated header.
        let truncated = write_source(&sources, "truncated.bin", &bundle_fixture()[..10]);
        assert!(matches!(
            assemble_full(
                &dist,
                PackageProfile::Release,
                Some(&binary),
                Some(&truncated)
            )
            .unwrap_err(),
            PackageError::BadBundle(_)
        ));
        // Declared entry with no frame: truncated, not shippable.
        let mut ghost = bundle_fixture();
        ghost[12..16].copy_from_slice(&1u32.to_le_bytes());
        let ghost_path = write_source(&sources, "ghost.bin", &ghost);
        assert!(matches!(
            assemble_full(
                &dist,
                PackageProfile::Release,
                Some(&binary),
                Some(&ghost_path)
            )
            .unwrap_err(),
            PackageError::BadBundle(_)
        ));
        // Empty bundle bytes are refused, never staged as real.
        let empty = write_source(&sources, "empty.bin", b"");
        assert!(matches!(
            assemble_full(&dist, PackageProfile::Release, Some(&binary), Some(&empty)).unwrap_err(),
            PackageError::BadBundle(_)
        ));
        std::fs::remove_dir_all(&sources).unwrap();
        std::fs::remove_dir_all(&dist).unwrap();
    }

    #[test]
    fn tampered_artifact_fails_strict_verification() {
        let sources = fresh_temp_dir("tampsrc");
        let dist = fresh_temp_dir("tampdist");
        let binary = write_source(&sources, "arena.exe", b"REALARENA-TAMPER-ME");
        let bundle = write_source(&sources, "arena-bundle.bin", &bundle_fixture());
        let manifest =
            assemble_full(&dist, PackageProfile::Release, Some(&binary), Some(&bundle)).unwrap();
        assert_eq!(verify_package(&dist).unwrap(), manifest);
        // One flipped byte in the staged binary fails with its name carried.
        let mut tampered = std::fs::read(join_dist(&dist, BIN_REL)).unwrap();
        tampered[0] ^= 0xFF;
        std::fs::write(join_dist(&dist, BIN_REL), &tampered).unwrap();
        let err = verify_package(&dist).unwrap_err();
        match err {
            PackageError::DigestMismatch { name, .. } => assert_eq!(name, BIN_REL),
            other => panic!("want DigestMismatch, got {other}"),
        }
        std::fs::remove_dir_all(&sources).unwrap();
        std::fs::remove_dir_all(&dist).unwrap();
    }

    #[test]
    fn missing_launch_and_manifest_fail_strict_verification() {
        let dist = fresh_temp_dir("launchmiss");
        assemble_inner(&dist, PackageProfile::Release, None).unwrap();
        // Missing launch manifest fails loudly.
        std::fs::remove_file(join_dist(&dist, LAUNCH_REL)).unwrap();
        assert!(matches!(
            verify_package(&dist).unwrap_err(),
            PackageError::BadLaunch(_)
        ));
        assert!(
            verify_clean(&dist).contains(&CleanViolation::MissingLaunch {
                expected: LAUNCH_REL.to_owned(),
            })
        );
        // Missing manifest fails loudly too.
        assemble_inner(&dist, PackageProfile::Release, None).unwrap();
        std::fs::remove_file(join_dist(&dist, MANIFEST_REL)).unwrap();
        assert!(matches!(
            verify_package(&dist).unwrap_err(),
            PackageError::BadManifest(_)
        ));
        // A manifest row with a forged digest fails on the recompute.
        assemble_inner(&dist, PackageProfile::Release, None).unwrap();
        let raw = std::fs::read(join_dist(&dist, MANIFEST_REL)).unwrap();
        let mut text = String::from_utf8(raw).unwrap();
        text = text.replacen("binary:STUB", "binary:REAL", 1);
        std::fs::write(join_dist(&dist, MANIFEST_REL), text.as_bytes()).unwrap();
        assert!(matches!(
            verify_package(&dist).unwrap_err(),
            PackageError::BadLaunch(_)
        ));
        std::fs::remove_dir_all(&dist).unwrap();
    }

    #[test]
    fn same_profile_twice_gives_identical_manifests() {
        let first_dir = fresh_temp_dir("det-a");
        let second_dir = fresh_temp_dir("det-b");
        let first = assemble_inner(&first_dir, PackageProfile::Release, None).unwrap();
        let second = assemble_inner(&second_dir, PackageProfile::Release, None).unwrap();
        assert_eq!(first, second);
        std::fs::remove_dir_all(&first_dir).unwrap();
        std::fs::remove_dir_all(&second_dir).unwrap();
    }

    #[test]
    fn unknown_profile_name_rejected() {
        use core::str::FromStr;
        assert_eq!(
            PackageProfile::from_str("debug"),
            Err(PackageError::UnknownProfile)
        );
        assert_eq!(
            PackageProfile::from_str("release").unwrap(),
            PackageProfile::Release
        );
    }

    #[test]
    fn stub_bundle_is_a_loadable_empty_frame() {
        // The stub bundle carries the asset bundle magic and v1 header with zero
        // entries (the hermetic stand-in for the pipeline bundle).
        let stub = stub_bundle_bytes();
        assert_eq!(&stub[..8], BUNDLE_MAGIC);
        assert_eq!(validate_bundle_bytes(&stub).unwrap(), 0);
    }

    #[test]
    fn errors_display_without_panic() {
        for err in [
            PackageError::Io(ErrorKind::NotFound),
            PackageError::UnknownProfile,
            PackageError::BadBinarySource,
            PackageError::MissingBinary,
            PackageError::MissingBundle,
            PackageError::BadBundleSource,
            PackageError::BadBundle("why".to_string()),
            PackageError::BadManifest("why".to_string()),
            PackageError::BadLaunch("why".to_string()),
            PackageError::DigestMismatch {
                name: "n".to_string(),
                expected: "e".to_string(),
                actual: "a".to_string(),
            },
        ] {
            assert!(!format!("{err}").is_empty(), "display empty for {err:?}");
        }
        assert!(!format!(
            "{}",
            CleanViolation::MissingLaunch {
                expected: LAUNCH_REL.to_owned(),
            }
        )
        .is_empty());
    }
}
