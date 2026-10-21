//! asset_source: content reaches the engine through a source, never a bare
//! file path.
//!
//! Owner: `engine-assets`. Callers name content by logical path (`/`
//! separated, relative, no `.` or `..`); a source maps the name to bytes.
//! [`FsSource`] reads a directory tree (native); [`BundleSource`] reads a
//! packed `.e2bundle` held in memory (browser: fetched; native: tools and
//! tests); [`FetchSource`] fetches that bundle through a host fetcher and
//! serves it; [`MemorySource`] holds what a cook staged, so a packer never
//! needs a staging directory. All of them return the same bytes for the same
//! name, list names in the same bytewise order and fail with the missing or
//! damaged name in the error, so a gap never becomes a silent default.
//! Std-only; no clock, no RNG; the browser build never compiles the file
//! reader of [`FsSource`].

use super::wire::bound_chars;
use engine_core::{hash_bytes, Fnv64};
use std::borrow::Cow;
use std::fmt;

pub mod bundle;
pub mod fetch;
#[cfg(not(target_arch = "wasm32"))]
pub use bundle::bundle_pack;
pub use bundle::{pack_to, BundleSource, EntryInfo, PackReport, PackRules};
pub use fetch::FetchSource;

/// Longest logical path in bytes; longer names are refused, never cut.
pub const MAX_SOURCE_PATH_BYTES: usize = 512;
/// Chars of a path echoed inside an error.
const ECHO_CHARS: usize = 128;

/// Why a source could not answer; every variant names the path involved.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SourceError {
    /// No file has this name in the source.
    Missing {
        /// Logical path asked for (bounded echo).
        path: String,
    },
    /// The name is not a logical path (absolute, `..`, backslash, empty part).
    BadPath {
        /// Offending path (bounded echo).
        path: String,
        /// Which rule it broke.
        reason: &'static str,
    },
    /// The host refused the read or write.
    Io {
        /// Logical path, or the output name when packing.
        path: String,
        /// Host error kind.
        kind: std::io::ErrorKind,
    },
    /// Blob bytes differ from the hash recorded when the bundle was packed.
    HashMismatch {
        /// Logical path of the damaged blob.
        path: String,
        /// Hash recorded in the bundle index.
        expected: u64,
        /// Hash of the bytes actually present.
        actual: u64,
    },
    /// The bytes are not a valid bundle.
    BadBundle {
        /// Bounded reason; names the entry when one is at fault.
        detail: String,
    },
    /// A pack request that cannot be honoured (nothing selected, too many
    /// files, unreadable tier flag).
    BadPack {
        /// Bounded reason.
        detail: String,
    },
}

impl SourceError {
    /// Missing-name error with a bounded echo.
    pub(crate) fn missing(path: &str) -> Self {
        Self::Missing {
            path: bound_chars(path, ECHO_CHARS),
        }
    }

    /// Bundle-level error with a bounded reason.
    pub(crate) fn bad_bundle(detail: &str) -> Self {
        Self::BadBundle {
            detail: bound_chars(detail, ECHO_CHARS * 2),
        }
    }

    /// Pack-request error with a bounded reason.
    pub(crate) fn bad_pack(detail: &str) -> Self {
        Self::BadPack {
            detail: bound_chars(detail, ECHO_CHARS * 2),
        }
    }

    /// Host error for `path`; "not found" is the typed missing-name error.
    /// Only the file-system source and the packer hit the host.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn from_io(path: &str, err: &std::io::Error) -> Self {
        if err.kind() == std::io::ErrorKind::NotFound {
            Self::missing(path)
        } else {
            Self::Io {
                path: bound_chars(path, ECHO_CHARS),
                kind: err.kind(),
            }
        }
    }
}

impl fmt::Display for SourceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing { path } => write!(f, "asset {path:?} not found"),
            Self::BadPath { path, reason } => write!(f, "asset path {path:?} refused: {reason}"),
            Self::Io { path, kind } => write!(f, "asset {path:?} io failure ({kind:?})"),
            Self::HashMismatch {
                path,
                expected,
                actual,
            } => write!(
                f,
                "asset {path:?} hash mismatch: bundle has {expected:016x}, bytes hash to {actual:016x}"
            ),
            Self::BadBundle { detail } => write!(f, "bad bundle: {detail}"),
            Self::BadPack { detail } => write!(f, "bad pack request: {detail}"),
        }
    }
}

impl std::error::Error for SourceError {}

/// Check that `path` is a logical path: non-empty `/`-separated parts, none
/// empty, `.`, `..` or holding a backslash, colon or control char. This is
/// the one rule that keeps a name inside its source (no drive prefix, no
/// parent escape) on every host.
pub fn validate_path(path: &str) -> Result<(), SourceError> {
    let refuse = |reason: &'static str| SourceError::BadPath {
        path: bound_chars(path, ECHO_CHARS),
        reason,
    };
    if path.is_empty() {
        return Err(refuse("empty"));
    }
    if path.len() > MAX_SOURCE_PATH_BYTES {
        return Err(refuse("longer than MAX_SOURCE_PATH_BYTES"));
    }
    for part in path.split('/') {
        if part.is_empty() {
            return Err(refuse("empty part (leading, trailing or doubled slash)"));
        }
        if part == "." || part == ".." {
            return Err(refuse("dot part"));
        }
        if part
            .chars()
            .any(|c| c == '\\' || c == ':' || c.is_control())
        {
            return Err(refuse("backslash, colon or control char"));
        }
    }
    Ok(())
}

/// Read-only view of game content by logical path. Object safe: callers
/// take `&dyn AssetSource` so the same load code runs from disk or a bundle.
pub trait AssetSource {
    /// Bytes of the file at `path`. A name the source does not hold is
    /// [`SourceError::Missing`] with that name; a bundle blob whose bytes no
    /// longer match its recorded hash is [`SourceError::HashMismatch`].
    fn read(&self, path: &str) -> Result<Cow<'_, [u8]>, SourceError>;

    /// True when `path` names a file here; a bad path is simply not there.
    fn exists(&self, path: &str) -> bool;

    /// Every file under directory `dir` (`""` is the whole source) as full
    /// logical paths, sorted bytewise. A non-empty `dir` holding no file is
    /// [`SourceError::Missing`] (a source cannot tell an empty directory
    /// from an absent one).
    fn list(&self, dir: &str) -> Result<Vec<String>, SourceError>;
}

/// Digest of every file under `dir` (`""` for all), in path order. The same
/// files give the same digest through any source, so a browser load can be
/// compared with the native one. Reads each file once.
pub fn content_digest(source: &dyn AssetSource, dir: &str) -> Result<u64, SourceError> {
    let mut digest = ContentDigest::new();
    for path in source.list(dir)? {
        let bytes = source.read(&path)?;
        digest.add(&path, &bytes);
    }
    Ok(digest.finish())
}

/// Incremental form of [`content_digest`]: add files in path order, then
/// finish. Each file folds in as path, length and byte hash, so a renamed,
/// resized, edited or dropped file all move the digest. The packer folds the
/// same way, so its report matches a later read.
pub(crate) struct ContentDigest {
    hasher: Fnv64,
    files: u64,
}

impl ContentDigest {
    pub(crate) fn new() -> Self {
        Self {
            hasher: Fnv64::new(),
            files: 0,
        }
    }

    pub(crate) fn add(&mut self, path: &str, bytes: &[u8]) {
        self.add_hashed(path, bytes.len(), hash_bytes(bytes));
    }

    /// [`ContentDigest::add`] for a caller that already hashed the bytes.
    pub(crate) fn add_hashed(&mut self, path: &str, len: usize, hash: u64) {
        self.hasher
            .write_u64(u64::try_from(path.len()).unwrap_or(u64::MAX));
        self.hasher.write_str(path);
        self.hasher
            .write_u64(u64::try_from(len).unwrap_or(u64::MAX));
        self.hasher.write_u64(hash);
        self.files += 1;
    }

    pub(crate) fn finish(mut self) -> u64 {
        self.hasher.write_u64(self.files);
        self.hasher.finish()
    }
}

pub use fs_source::FsSource;

/// An [`AssetSource`] over files a cook already holds in memory.
///
/// Consumer: `arena_bundle --format packed` stages the compact kit glTF and
/// the cooked KTX2 files here and hands the source to [`pack_to`], so the
/// bundle is built without a staging directory and the same staged files give
/// the same bytes on every machine. Paths are validated on insert; iteration
/// is `BTreeMap` order, which is the bytewise order every source lists in.
#[derive(Clone, Debug, Default)]
pub struct MemorySource {
    files: std::collections::BTreeMap<String, Vec<u8>>,
}

impl MemorySource {
    /// An empty source.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Add (or replace) the file at `path`; a bad logical path is refused.
    ///
    /// # Errors
    /// [`SourceError::BadPath`] when `path` is not a logical path.
    pub fn insert(&mut self, path: &str, bytes: Vec<u8>) -> Result<(), SourceError> {
        validate_path(path)?;
        self.files.insert(path.to_string(), bytes);
        Ok(())
    }

    /// Files held.
    #[must_use]
    pub fn len(&self) -> usize {
        self.files.len()
    }

    /// True when nothing was inserted.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }
}

impl AssetSource for MemorySource {
    fn read(&self, path: &str) -> Result<Cow<'_, [u8]>, SourceError> {
        validate_path(path)?;
        self.files
            .get(path)
            .map(|bytes| Cow::Borrowed(bytes.as_slice()))
            .ok_or_else(|| SourceError::missing(path))
    }

    fn exists(&self, path: &str) -> bool {
        validate_path(path).is_ok() && self.files.contains_key(path)
    }

    fn list(&self, dir: &str) -> Result<Vec<String>, SourceError> {
        if dir.is_empty() {
            return Ok(self.files.keys().cloned().collect());
        }
        validate_path(dir)?;
        let prefix = format!("{dir}/");
        let found: Vec<String> = self
            .files
            .range(prefix.clone()..)
            .take_while(|(path, _)| path.starts_with(prefix.as_str()))
            .map(|(path, _)| path.clone())
            .collect();
        if found.is_empty() {
            return Err(SourceError::missing(dir));
        }
        Ok(found)
    }
}

/// The browser has no file system: the same type exists so callers compile on
/// every target, and every read says so by name (`Unsupported`). A browser
/// build reads through [`BundleSource`] instead.
#[cfg(target_arch = "wasm32")]
mod fs_source {
    use super::{bound_chars, AssetSource, SourceError, ECHO_CHARS};
    use std::borrow::Cow;
    use std::path::PathBuf;

    /// A directory root that cannot be read on this target.
    #[derive(Clone, Debug)]
    pub struct FsSource {
        _root: PathBuf,
    }

    impl FsSource {
        /// Source rooted at `root`; every read on this target is refused.
        pub fn new(root: impl Into<PathBuf>) -> Self {
            Self { _root: root.into() }
        }
    }

    fn unsupported(path: &str) -> SourceError {
        SourceError::Io {
            path: bound_chars(path, ECHO_CHARS),
            kind: std::io::ErrorKind::Unsupported,
        }
    }

    impl AssetSource for FsSource {
        fn read(&self, path: &str) -> Result<Cow<'_, [u8]>, SourceError> {
            Err(unsupported(path))
        }

        fn exists(&self, _path: &str) -> bool {
            false
        }

        fn list(&self, dir: &str) -> Result<Vec<String>, SourceError> {
            Err(unsupported(if dir.is_empty() { "(root)" } else { dir }))
        }
    }
}

/// Directory-backed source: today's native file reads, byte for byte.
#[cfg(not(target_arch = "wasm32"))]
mod fs_source {
    use super::{validate_path, AssetSource, SourceError};
    use std::borrow::Cow;
    use std::path::PathBuf;

    /// Files under one root directory, named by logical path.
    #[derive(Clone, Debug)]
    pub struct FsSource {
        root: PathBuf,
    }

    impl FsSource {
        /// Source rooted at `root`; the directory is not touched until a read.
        pub fn new(root: impl Into<PathBuf>) -> Self {
            Self { root: root.into() }
        }

        /// Host path of a validated logical path, one component at a time.
        fn resolve(&self, path: &str) -> Result<PathBuf, SourceError> {
            validate_path(path)?;
            let mut full = self.root.clone();
            full.extend(path.split('/'));
            Ok(full)
        }
    }

    impl AssetSource for FsSource {
        fn read(&self, path: &str) -> Result<Cow<'_, [u8]>, SourceError> {
            let full = self.resolve(path)?;
            std::fs::read(full)
                .map(Cow::Owned)
                .map_err(|err| SourceError::from_io(path, &err))
        }

        fn exists(&self, path: &str) -> bool {
            self.resolve(path).is_ok_and(|full| full.is_file())
        }

        fn list(&self, dir: &str) -> Result<Vec<String>, SourceError> {
            let start = if dir.is_empty() {
                self.root.clone()
            } else {
                self.resolve(dir)?
            };
            if !dir.is_empty() && !start.is_dir() {
                return Err(SourceError::missing(dir));
            }
            let label = if dir.is_empty() { "(root)" } else { dir };
            let mut files = Vec::new();
            let mut pending = vec![(start, dir.to_string())];
            while let Some((host, logical)) = pending.pop() {
                let entries =
                    std::fs::read_dir(&host).map_err(|err| SourceError::from_io(label, &err))?;
                for entry in entries {
                    let entry = entry.map_err(|err| SourceError::from_io(label, &err))?;
                    let name =
                        entry
                            .file_name()
                            .into_string()
                            .map_err(|raw| SourceError::BadPath {
                                path: raw.to_string_lossy().chars().take(32).collect(),
                                reason: "file name is not UTF-8",
                            })?;
                    let child = if logical.is_empty() {
                        name
                    } else {
                        format!("{logical}/{name}")
                    };
                    let kind = entry
                        .file_type()
                        .map_err(|err| SourceError::from_io(&child, &err))?;
                    if kind.is_dir() {
                        pending.push((entry.path(), child));
                    } else if kind.is_file() {
                        validate_path(&child)?;
                        files.push(child);
                    }
                    // Links and device files are not content: skipped.
                }
            }
            if files.is_empty() && !dir.is_empty() {
                return Err(SourceError::missing(dir));
            }
            files.sort();
            Ok(files)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logical_paths_stay_inside_their_source() {
        for ok in ["a", "a/b.gltf", "src/x y.png", "a/b/c/d"] {
            assert_eq!(validate_path(ok), Ok(()), "{ok}");
        }
        for (bad, why) in [
            ("", "empty"),
            ("/abs", "leading"),
            ("a/", "trailing"),
            ("a//b", "doubled"),
            ("../up", "dot part"),
            ("a/./b", "dot part"),
            ("a\\b", "backslash"),
            ("C:/x", "colon"),
            ("a\u{0}b", "control"),
        ] {
            assert!(
                matches!(validate_path(bad), Err(SourceError::BadPath { .. })),
                "{bad:?} must be refused ({why})"
            );
        }
        let long = "a/".repeat(MAX_SOURCE_PATH_BYTES);
        assert!(validate_path(&long).is_err(), "over the byte cap");
    }

    #[test]
    fn errors_name_the_path() {
        let missing = SourceError::missing("textures/sand.jpg");
        assert!(missing.to_string().contains("textures/sand.jpg"));
        let mismatch = SourceError::HashMismatch {
            path: "src/crate.gltf".to_string(),
            expected: 1,
            actual: 2,
        };
        let text = mismatch.to_string();
        assert!(text.contains("src/crate.gltf") && text.contains("0000000000000001"));
        let long = "x".repeat(10_000);
        assert!(
            SourceError::missing(&long).to_string().len() < 400,
            "echo is bounded"
        );
    }

    #[test]
    fn digest_moves_with_name_size_and_bytes() {
        let digest = |files: &[(&str, &[u8])]| {
            let mut d = ContentDigest::new();
            for (path, bytes) in files {
                d.add(path, bytes);
            }
            d.finish()
        };
        let base = digest(&[("a", b"xy"), ("b", b"z")]);
        assert_eq!(base, digest(&[("a", b"xy"), ("b", b"z")]));
        assert_ne!(base, digest(&[("a", b"xy"), ("c", b"z")]), "renamed");
        assert_ne!(base, digest(&[("a", b"x"), ("b", b"yz")]), "split moved");
        assert_ne!(base, digest(&[("a", b"xy"), ("b", b"Z")]), "edited");
        assert_ne!(base, digest(&[("a", b"xy")]), "dropped");
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod fs_tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU32, Ordering};

    static COUNTER: AtomicU32 = AtomicU32::new(0);

    /// Fresh fixture dir with a nested tree; callers delete it.
    pub(crate) fn fixture(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "asset_source_{tag}_{}_{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        for (path, bytes) in [
            ("placement.json", &b"{\"actors\":[]}"[..]),
            ("src/crate.gltf", &[7u8; 300][..]),
            ("src/banner.gltf", &[9u8; 40][..]),
            ("textures/sand_2k.jpg", &[1u8, 2, 3, 4, 5][..]),
            ("textures/deep/stone.jpg", &[0u8; 0][..]),
        ] {
            let host = dir.join(path);
            std::fs::create_dir_all(host.parent().expect("fixture parent")).expect("mkdir");
            std::fs::write(host, bytes).expect("fixture file");
        }
        dir
    }

    #[test]
    fn fs_source_reads_exactly_the_file_bytes() {
        let dir = fixture("read");
        let source = FsSource::new(&dir);
        let on_disk = std::fs::read(dir.join("src").join("crate.gltf")).expect("disk read");
        assert_eq!(
            source.read("src/crate.gltf").expect("read").as_ref(),
            on_disk
        );
        assert!(source
            .read("textures/deep/stone.jpg")
            .expect("empty file")
            .is_empty());
        assert!(source.exists("placement.json"));
        assert!(!source.exists("src"), "a directory is not a file");
        assert!(!source.exists("src/none.gltf"));
        assert!(
            !source.exists("../escape"),
            "a bad path is simply not there"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn fs_source_names_what_is_missing_or_refused() {
        let dir = fixture("missing");
        let source = FsSource::new(&dir);
        let err = source.read("src/none.gltf").expect_err("absent file");
        assert_eq!(err, SourceError::missing("src/none.gltf"));
        assert!(err.to_string().contains("src/none.gltf"));
        let escape = source.read("../outside").expect_err("escape refused");
        assert!(matches!(escape, SourceError::BadPath { .. }), "{escape}");
        let dir_read = source.read("src").expect_err("a directory is not content");
        assert!(
            matches!(&dir_read, SourceError::Io { path, .. } if path == "src"),
            "{dir_read}"
        );
        assert_eq!(
            source.list("nowhere").expect_err("no such directory"),
            SourceError::missing("nowhere")
        );
        assert!(matches!(
            FsSource::new(dir.join("absent_root")).list(""),
            Err(SourceError::Missing { .. })
        ));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn fs_source_lists_sorted_logical_paths() {
        let dir = fixture("list");
        let source = FsSource::new(&dir);
        assert_eq!(
            source.list("").expect("list all"),
            [
                "placement.json",
                "src/banner.gltf",
                "src/crate.gltf",
                "textures/deep/stone.jpg",
                "textures/sand_2k.jpg",
            ]
        );
        assert_eq!(
            source.list("textures").expect("list dir"),
            ["textures/deep/stone.jpg", "textures/sand_2k.jpg"]
        );
        assert!(matches!(
            source.list("src/"),
            Err(SourceError::BadPath { .. })
        ));
        assert_eq!(
            source
                .list("placement.json")
                .expect_err("a file is not a directory"),
            SourceError::missing("placement.json")
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn content_digest_is_stable_and_sees_an_edit() {
        let dir = fixture("digest");
        let source = FsSource::new(&dir);
        let before = content_digest(&source, "").expect("digest");
        assert_eq!(before, content_digest(&source, "").expect("again"));
        assert_ne!(before, content_digest(&source, "src").expect("subset"));
        std::fs::write(dir.join("src").join("banner.gltf"), [9u8; 41]).expect("edit");
        assert_ne!(before, content_digest(&source, "").expect("edited"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
