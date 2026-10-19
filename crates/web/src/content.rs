//! content: the cooked arena content, fetched once and read as an asset source.
//!
//! Consumer: the played page (`crates/web/src/arena.rs` fight branch) stages the
//! arena kit from a fetched `.e2bundle` because the native window's path
//! (`engine_game::arena_kit::stage_kit`) reads the file system, and `FsSource` is
//! `Unsupported` on wasm32. Data owner: `engine-assets` owns the bundle format and
//! its digests; this module owns the fetch and the typed refusal.
//! Errors: typed [`ContentError`]; a missing or torn bundle leaves the page on
//! the procedural ring instead of blanking the frame.

use engine_assets::asset_source::{BundleSource, SourceError};

/// Where the packer puts the bundle inside `dist/web`.
pub const ARENA_BUNDLE_URL: &str = "assets/arena.e2bundle";

/// Why the page has no cooked content: every failure is typed, none panics.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ContentError {
    /// The fetch itself failed (an offline tab, a blocked scheme): bounded detail.
    Fetch(String),
    /// The server answered, but not with the bundle.
    Status(u16),
    /// The response carried no bytes.
    Empty,
    /// The bytes are not a readable bundle: the format's own reason.
    Bundle(SourceError),
}

impl core::fmt::Display for ContentError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Fetch(detail) => write!(f, "content fetch: {detail}"),
            Self::Status(status) => write!(f, "content fetch: http {status}"),
            Self::Empty => write!(f, "content fetch: empty body"),
            Self::Bundle(err) => write!(f, "content bundle: {err}"),
        }
    }
}

impl std::error::Error for ContentError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Bundle(err) => Some(err),
            _ => None,
        }
    }
}

impl From<SourceError> for ContentError {
    fn from(err: SourceError) -> Self {
        Self::Bundle(err)
    }
}

/// The fetched bundle as an asset source. The bundle verifies its own digests on
/// read, so a torn download fails at the entry, not in the picture.
///
/// # Errors
/// [`ContentError::Empty`] for a zero-length body, or the format's own refusal.
pub fn open_bundle(bytes: Vec<u8>) -> Result<BundleSource, ContentError> {
    if bytes.is_empty() {
        return Err(ContentError::Empty);
    }
    Ok(BundleSource::open(bytes)?)
}

/// Fetch the bundle at `url` and open it. One request per page; the caller stages
/// from the result and keeps it.
///
/// # Errors
/// [`ContentError::Fetch`] when the request itself fails (the detail is the JS
/// message, bounded), [`ContentError::Status`] on a non-OK response, and the
/// [`open_bundle`] refusals.
#[cfg(target_arch = "wasm32")]
pub async fn fetch_bundle(url: &str) -> Result<BundleSource, ContentError> {
    use wasm_bindgen::JsCast;
    use wasm_bindgen_futures::JsFuture;

    let window = web_sys::window().ok_or_else(|| ContentError::Fetch("no window".into()))?;
    let response = JsFuture::from(window.fetch_with_str(url))
        .await
        .map_err(|err| ContentError::Fetch(describe(&err)))?;
    let response: web_sys::Response = response
        .dyn_into()
        .map_err(|_| ContentError::Fetch("the response is not a Response".into()))?;
    if !response.ok() {
        return Err(ContentError::Status(response.status()));
    }
    let buffer = response
        .array_buffer()
        .map_err(|err| ContentError::Fetch(describe(&err)))?;
    let buffer = JsFuture::from(buffer)
        .await
        .map_err(|err| ContentError::Fetch(describe(&err)))?;
    let bytes = js_sys::Uint8Array::new(&buffer).to_vec();
    open_bundle(bytes)
}

/// A JS error's message (its `toString`), bounded to 120 chars for a log line.
#[cfg(target_arch = "wasm32")]
fn describe(err: &wasm_bindgen::JsValue) -> String {
    use wasm_bindgen::JsCast;
    let text = err
        .dyn_ref::<js_sys::Error>()
        .and_then(|err| err.message().as_string())
        .or_else(|| err.as_string())
        .unwrap_or_else(|| format!("{err:?}"));
    text.chars().take(120).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_assets::asset_source::{bundle_pack, AssetSource, PackRules};

    /// A directory with one file, packed and read back through [`open_bundle`].
    #[test]
    fn a_packed_bundle_opens_and_reads_its_entries() {
        let base = std::env::temp_dir().join("e2040-web-content-open");
        let root = base.join("content");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(root.join("cooked")).expect("temp dir");
        std::fs::write(root.join("cooked/sand_base.ktx2"), b"ktx2 bytes").expect("write");
        // The packer refuses an output inside the root it walks, so the bundle is a sibling.
        let out = base.join("arena.e2bundle");
        bundle_pack(&root, &out, &PackRules::new()).expect("pack");
        let bytes = std::fs::read(&out).expect("read bundle");
        let source = open_bundle(bytes).expect("open");
        assert_eq!(
            &*source.read("cooked/sand_base.ktx2").expect("entry"),
            b"ktx2 bytes".as_slice()
        );
        assert!(source.read("cooked/missing.ktx2").is_err());
        // A torn body is refused, not silently empty.
        assert!(matches!(open_bundle(Vec::new()), Err(ContentError::Empty)));
        assert!(open_bundle(vec![0u8; 16]).is_err());
        let _ = std::fs::remove_dir_all(&base);
    }
}
