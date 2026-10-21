//! fetch: content that arrives by fetch instead of by file.
//!
//! Owner: `engine-assets`. A [`FetchSource`] asks a host fetcher for one
//! file (the packed `.e2bundle` in the browser) and serves the logical paths
//! inside it. The browser's `fetch` is the fetcher; a native file read or a
//! test double is the same seam, so the game loads the same content either
//! way. The fetch happens once, on the first read; a failed fetch is
//! reported by name and never cached, so the next read tries again. Std-only:
//! no HTTP client, no clock, no threads.

use super::bundle::BundleSource;
use super::{AssetSource, SourceError};
use std::borrow::Cow;
use std::cell::OnceCell;

/// The host fetcher: the browser's `fetch`, a native file read, a test
/// double. It takes the logical path and returns the bytes of that file.
type Fetcher = Box<dyn Fn(&str) -> Result<Vec<u8>, SourceError>>;

/// A source whose bytes arrive through a host fetcher.
///
/// Consumer: the browser page fetches the packed bundle and hands the
/// fetcher over; the game then names content by logical path exactly as it
/// does over the native file source.
pub struct FetchSource {
    /// Logical path of the file to fetch.
    path: String,
    /// The host fetcher.
    fetcher: Fetcher,
    /// The opened bundle, once the fetch landed.
    bundle: OnceCell<BundleSource>,
}

impl FetchSource {
    /// Source over the file at `path`, fetched through `fetcher` on first use.
    ///
    /// The fetcher is called with the logical path and returns the bytes of
    /// that file; a packed `.e2bundle` is the shape the browser fetches.
    pub fn new<F>(path: &str, fetcher: F) -> Self
    where
        F: Fn(&str) -> Result<Vec<u8>, SourceError> + 'static,
    {
        Self {
            path: path.to_string(),
            fetcher: Box::new(fetcher),
            bundle: OnceCell::new(),
        }
    }

    /// The logical path this source fetches.
    pub fn path(&self) -> &str {
        &self.path
    }

    /// True once the fetch landed and the bundle opened.
    pub fn fetched(&self) -> bool {
        self.bundle.get().is_some()
    }

    /// Bytes of the fetched file, when it landed.
    pub fn byte_len(&self) -> Option<usize> {
        self.bundle.get().map(BundleSource::byte_len)
    }

    /// The opened bundle, fetching it the first time it is needed.
    fn ensure(&self) -> Result<&BundleSource, SourceError> {
        if let Some(bundle) = self.bundle.get() {
            return Ok(bundle);
        }
        let bytes = (self.fetcher)(&self.path)?;
        let bundle = BundleSource::open(bytes)?;
        // Another read may have won the race; keep the first bundle.
        let _ = self.bundle.set(bundle);
        self.bundle
            .get()
            .ok_or_else(|| SourceError::bad_bundle("fetched bundle did not open"))
    }
}

impl AssetSource for FetchSource {
    fn read(&self, path: &str) -> Result<Cow<'_, [u8]>, SourceError> {
        self.ensure()?.read(path)
    }

    fn exists(&self, path: &str) -> bool {
        self.ensure().is_ok_and(|bundle| bundle.exists(path))
    }

    fn list(&self, dir: &str) -> Result<Vec<String>, SourceError> {
        self.ensure()?.list(dir)
    }
}

#[cfg(test)]
mod tests {
    use super::super::bundle::pack_to;
    use super::super::{content_digest, MemorySource, PackRules};
    use super::*;
    use std::cell::Cell;
    use std::rc::Rc;

    /// A packed bundle holding two files.
    fn packed() -> Vec<u8> {
        let mut staged = MemorySource::new();
        staged
            .insert("src/crate.gltf", vec![7u8; 64])
            .expect("path");
        staged
            .insert("src/banner.gltf", vec![9u8; 16])
            .expect("path");
        let mut bytes = Vec::new();
        pack_to(&staged, &PackRules::new(), &mut bytes).expect("packs");
        bytes
    }

    #[test]
    fn fetch_source_serves_the_paths_of_the_fetched_bundle() {
        let bytes = packed();
        let byte_len = bytes.len();
        let calls = Rc::new(Cell::new(0usize));
        let counter = Rc::clone(&calls);
        let source = FetchSource::new("assets/arena.e2bundle", move |path| {
            counter.set(counter.get() + 1);
            assert_eq!(path, "assets/arena.e2bundle");
            Ok(bytes.clone())
        });
        assert!(!source.fetched());
        assert_eq!(source.read("src/crate.gltf").expect("read").len(), 64);
        assert_eq!(source.read("src/banner.gltf").expect("read").len(), 16);
        assert!(source.exists("src/crate.gltf"));
        assert!(!source.exists("src/none.gltf"));
        assert_eq!(
            source.list("").expect("list"),
            ["src/banner.gltf", "src/crate.gltf"]
        );
        assert_eq!(calls.get(), 1, "the file is fetched once");
        assert!(source.fetched());
        assert_eq!(source.byte_len(), Some(byte_len));
        assert_eq!(source.path(), "assets/arena.e2bundle");
    }

    #[test]
    fn fetch_source_names_a_missing_blob_and_a_refused_fetch() {
        let bytes = packed();
        let source = FetchSource::new("assets/arena.e2bundle", move |_| Ok(bytes.clone()));
        let err = source.read("src/gone.gltf").expect_err("not in the bundle");
        assert_eq!(err, SourceError::missing("src/gone.gltf"));
        assert!(err.to_string().contains("src/gone.gltf"));

        let refused = FetchSource::new("assets/arena.e2bundle", |path| {
            Err(SourceError::Io {
                path: path.to_string(),
                kind: std::io::ErrorKind::NotFound,
            })
        });
        let err = refused
            .read("src/crate.gltf")
            .expect_err("the fetch failed");
        assert!(err.to_string().contains("assets/arena.e2bundle"), "{err}");
        assert!(!refused.fetched());
    }

    #[test]
    fn fetch_source_serves_the_same_digest_as_the_files_it_came_from() {
        let bytes = packed();
        let source = FetchSource::new("assets/arena.e2bundle", move |_| Ok(bytes.clone()));
        let mut staged = MemorySource::new();
        staged
            .insert("src/crate.gltf", vec![7u8; 64])
            .expect("path");
        staged
            .insert("src/banner.gltf", vec![9u8; 16])
            .expect("path");
        assert_eq!(
            content_digest(&source, "").expect("digest"),
            content_digest(&staged, "").expect("digest")
        );
    }

    #[test]
    fn fetch_source_refuses_bytes_that_are_not_a_bundle() {
        let source = FetchSource::new("assets/arena.e2bundle", |_| Ok(vec![1u8; 8]));
        let err = source.read("src/crate.gltf").expect_err("not a bundle");
        assert!(err.to_string().contains("bad bundle"), "{err}");
        assert!(!source.fetched());
    }
}
