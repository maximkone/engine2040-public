//! content: the cooked arena content, fetched once and read as an asset source.
//!
//! Consumer: the played page (`crates/web/src/arena.rs` fight branch) stages the
//! arena kit from a fetched `.e2bundle` because the native window's path
//! (`engine_game::arena_kit::stage_kit`) reads the file system, and `FsSource` is
//! `Unsupported` on wasm32. Data owner: `engine-assets` owns the bundle format and
//! its digests; this module owns the fetch and the typed refusal.
//! Errors: typed [`ContentError`]; a missing or torn bundle leaves the page on
//! the procedural ring instead of blanking the frame.
//!
//! WEB6: the network is the page's, so the request for an asset lives in
//! `crates/web/loader.js` and a node test drives it. The engine asks the page
//! for the bytes through the page's own fetcher (`window.engine2040FetchAsset`)
//! and opens them here, on the same seam the native path uses: a
//! [`BundleSource`] is an [`AssetSource`], so a loader reads logical names and
//! never a file path, and a missing asset is a typed refusal that names it.

use engine_assets::asset_source::{AssetSource, BundleSource, SourceError};
use std::future::Future;

/// Where the packer puts the bundle inside `dist/web`.
pub const ARENA_BUNDLE_URL: &str = "assets/arena.e2bundle";

/// Where the packer puts the cooked rig inside `dist/web`: four bodies and
/// their atlas quadrants, the lane `rig_cook` writes.
pub const RIG_BUNDLE_URL: &str = "assets/fighters.e2bundle";

/// The name the page gives its fetcher: `window.engine2040FetchAsset`.
pub const PAGE_FETCHER: &str = "engine2040FetchAsset";

/// The QW4 fetcher: it answers with the bundle's tier-0 prefix while the rest
/// of the body keeps streaming for the page's progress bar.
pub const PAGE_PREFIX_FETCHER: &str = "engine2040FetchAssetPrefix";

/// Why the page has no cooked content: every failure is typed, none panics.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ContentError {
    /// The fetch itself failed (an offline tab, a blocked scheme): bounded detail.
    Fetch(String),
    /// The server answered, but not with the asset.
    Status(u16),
    /// The response carried no bytes.
    Empty,
    /// The source does not hold this asset.
    Missing {
        /// The logical path that was asked for.
        path: String,
    },
    /// The bytes are not a readable bundle: the format's own reason.
    Bundle(SourceError),
}

impl core::fmt::Display for ContentError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Fetch(detail) => write!(f, "content fetch: {detail}"),
            Self::Status(status) => write!(f, "content fetch: http {status}"),
            Self::Empty => write!(f, "content fetch: empty body"),
            Self::Missing { path } => write!(f, "content asset {path:?} is missing"),
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

/// The fetched tier-0 prefix as an asset source (QW4). Every entry is named, but
/// reading one from a later tier fails with its name, so the first frame can
/// start on the coarse level while the HDRI tier is still in flight.
///
/// # Errors
/// [`ContentError::Empty`] for a zero-length body, or the format's own refusal
/// when the bytes stop inside a blob.
pub fn open_bundle_prefix(bytes: Vec<u8>) -> Result<BundleSource, ContentError> {
    if bytes.is_empty() {
        return Err(ContentError::Empty);
    }
    Ok(BundleSource::open_prefix(bytes)?)
}

/// The bytes of `name` in `source`. A missing asset is a typed refusal that names
/// it; a damaged blob is the format's own reason.
///
/// # Errors
/// [`ContentError::Missing`] when the source holds no such asset, or the
/// format's refusal for a blob that no longer matches its hash.
pub fn asset_bytes(source: &dyn AssetSource, name: &str) -> Result<Vec<u8>, ContentError> {
    match source.read(name) {
        Ok(bytes) => Ok(bytes.into_owned()),
        Err(SourceError::Missing { path }) => Err(ContentError::Missing { path }),
        Err(err) => Err(ContentError::Bundle(err)),
    }
}

/// Open the bundle the host fetched for `url`.
///
/// `fetch` is how this host gets bytes: the browser page's own fetch, or a fake in
/// a test. It answers with the bytes or a typed reason, and this never turns a
/// refusal into a panic and never into an empty bundle.
///
/// # Errors
/// The reason the host gave, [`ContentError::Empty`] for an empty body, or the
/// format's refusal when the bytes are not a bundle.
pub async fn open_fetched_bundle<F, Fut>(url: &str, fetch: F) -> Result<BundleSource, ContentError>
where
    F: FnOnce(&str) -> Fut,
    Fut: Future<Output = Result<Vec<u8>, ContentError>>,
{
    open_bundle(fetch(url).await?)
}

/// Open the tier-0 prefix the host fetched for `url` (QW4). The fetch answers
/// before the later tiers arrive; the source names every entry but reads only
/// the bytes in hand.
///
/// # Errors
/// The reason the host gave, [`ContentError::Empty`] for an empty body, or the
/// format's refusal when the prefix stops inside a blob.
pub async fn open_fetched_bundle_prefix<F, Fut>(
    url: &str,
    fetch: F,
) -> Result<BundleSource, ContentError>
where
    F: FnOnce(&str) -> Fut,
    Fut: Future<Output = Result<Vec<u8>, ContentError>>,
{
    open_bundle_prefix(fetch(url).await?)
}

/// Fetch the bundle at `url` through the page's own fetcher and open it.
///
/// The page installs `window.engine2040FetchAsset` (see `crates/web/loader.js`);
/// without it this is a typed refusal, never a panic.
///
/// # Errors
/// [`ContentError::Fetch`] when the page installed no fetcher or the request
/// failed, [`ContentError::Status`] when the server answered with another
/// status, [`ContentError::Empty`] for an empty body, and the format's refusal
/// for bytes that are not a bundle.
#[cfg(target_arch = "wasm32")]
pub async fn fetch_bundle(url: &str) -> Result<BundleSource, ContentError> {
    open_fetched_bundle(url, |url| page_fetch(url.to_string())).await
}

/// Fetch the arena bundle's tier-0 prefix through the page's QW4 fetcher and
/// open it. The page resolves this call as soon as the coarse level is in hand;
/// the rest of the bundle keeps streaming behind it.
///
/// # Errors
/// The same refusals as [`fetch_bundle`], plus the format's refusal when the
/// prefix stops inside a blob.
#[cfg(target_arch = "wasm32")]
pub async fn fetch_bundle_prefix(url: &str) -> Result<BundleSource, ContentError> {
    open_fetched_bundle_prefix(url, |url| page_fetch_prefix(url.to_string())).await
}

/// The page's own fetch: it answers with the asset's bytes or throws. It takes
/// the URL by value, so the future it returns does not borrow the caller's
/// string.
#[cfg(target_arch = "wasm32")]
async fn page_fetch(url: String) -> Result<Vec<u8>, ContentError> {
    use wasm_bindgen::JsValue;

    page_call(PAGE_FETCHER, vec![JsValue::from_str(&url)]).await
}

/// The QW4 prefix fetch: the page answers at the tier-0 boundary, before the
/// HDRI tier arrives.
#[cfg(target_arch = "wasm32")]
async fn page_fetch_prefix(url: String) -> Result<Vec<u8>, ContentError> {
    use wasm_bindgen::JsValue;

    page_call(PAGE_PREFIX_FETCHER, vec![JsValue::from_str(&url)]).await
}

/// Call one of the page's fetchers by name with `args` and read the bytes it
/// answers. A missing function, a non-promise and a non-bytes answer are typed
/// refusals; a thrown JS error keeps its HTTP status when it names one.
#[cfg(target_arch = "wasm32")]
async fn page_call(
    fetcher_name: &str,
    args: Vec<wasm_bindgen::JsValue>,
) -> Result<Vec<u8>, ContentError> {
    use wasm_bindgen::JsCast;
    use wasm_bindgen::JsValue;

    let window = web_sys::window().ok_or_else(|| ContentError::Fetch("no window".into()))?;
    let fetcher = js_sys::Reflect::get(&window, &JsValue::from_str(fetcher_name))
        .ok()
        .and_then(|value| value.dyn_into::<js_sys::Function>().ok())
        .ok_or_else(|| ContentError::Fetch(format!("the page installed no {fetcher_name}")))?;
    let promise = fetcher
        .apply(&JsValue::NULL, &js_sys::Array::from_iter(args))
        .map_err(|err| ContentError::Fetch(describe(&err)))?;
    if !promise.is_instance_of::<js_sys::Promise>() {
        return Err(ContentError::Fetch(
            "the page's fetcher answered with no promise".into(),
        ));
    }
    let value = wasm_bindgen_futures::JsFuture::from(js_sys::Promise::from(promise))
        .await
        .map_err(|err| js_failure(&err))?;
    let bytes = value
        .dyn_ref::<js_sys::Uint8Array>()
        .map(|array| array.to_vec())
        .ok_or_else(|| ContentError::Fetch("the page's fetcher answered with no bytes".into()))?;
    if bytes.is_empty() {
        return Err(ContentError::Empty);
    }
    Ok(bytes)
}

/// A JS error the page's fetcher threw: its HTTP status when it names one, its
/// message otherwise.
#[cfg(target_arch = "wasm32")]
fn js_failure(err: &wasm_bindgen::JsValue) -> ContentError {
    let status = js_sys::Reflect::get(err, &wasm_bindgen::JsValue::from_str("status"))
        .ok()
        .and_then(|value| whole_number(&value));
    match status {
        Some(status) => ContentError::Status(status),
        None => ContentError::Fetch(describe(err)),
    }
}

/// A whole number the page sent, read without a numeric cast so nothing wraps.
#[cfg(target_arch = "wasm32")]
fn whole_number(value: &wasm_bindgen::JsValue) -> Option<u16> {
    value
        .as_f64()
        .and_then(|number| number.to_string().parse::<u16>().ok())
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
    use engine_assets::asset_source::{bundle_pack, PackRules};
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::task::{Context, Poll, Waker};

    /// Drive a future to its answer. The fakes below are ready at once, so
    /// this never parks.
    fn ready<F: Future>(future: F) -> F::Output {
        let mut cx = Context::from_waker(Waker::noop());
        let mut pinned = std::pin::pin!(future);
        loop {
            match pinned.as_mut().poll(&mut cx) {
                Poll::Ready(out) => return out,
                Poll::Pending => std::thread::yield_now(),
            }
        }
    }

    /// A packed bundle with one cooked file in it: the bytes a runtime fetch
    /// brings back.
    fn packed_bundle(tag: &str) -> Vec<u8> {
        let base = std::env::temp_dir().join(format!("e2040-web-content-{tag}"));
        let root = base.join("content");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(root.join("cooked")).expect("temp dir");
        std::fs::write(root.join("cooked/sand_base.ktx2"), b"ktx2 bytes").expect("write");
        // The packer refuses an output inside the root it walks, so the bundle is a sibling.
        let out = base.join("arena.e2bundle");
        bundle_pack(&root, &out, &PackRules::new()).expect("pack");
        let bytes = std::fs::read(&out).expect("read bundle");
        let _ = std::fs::remove_dir_all(&base);
        bytes
    }

    /// A bundle with a coarse tier and an HDRI tier: the QW4 prefix source
    /// reads the coarse files and names the tier it does not hold yet.
    fn packed_bundle_with_hdri(tag: &str) -> Vec<u8> {
        let base = std::env::temp_dir().join(format!("e2040-web-content-tier-{tag}"));
        let root = base.join("content");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(root.join("cooked")).expect("temp dir");
        std::fs::create_dir_all(root.join("hdri")).expect("temp dir");
        std::fs::write(root.join("cooked/sand_base.ktx2"), b"ktx2 bytes").expect("write");
        std::fs::write(root.join("hdri/studio.hdr"), b"hdri bytes").expect("write");
        let out = base.join("arena.e2bundle");
        bundle_pack(
            &root,
            &out,
            &PackRules::new().priority("hdri/studio.hdr", 1),
        )
        .expect("pack");
        let bytes = std::fs::read(&out).expect("read bundle");
        let _ = std::fs::remove_dir_all(&base);
        bytes
    }

    /// The byte at which the coarse tier ends, from the full bundle's own index.
    fn tier_zero_end(bytes: &[u8]) -> usize {
        BundleSource::open(bytes.to_vec())
            .expect("open")
            .entries_in_file_order()
            .filter(|entry| entry.priority == 0)
            .map(|entry| entry.offset + entry.len)
            .max()
            .expect("tier 0")
    }

    /// QW4: a fetched prefix reads the coarse tier and refuses the HDRI tier by
    /// name, so a torn first frame is impossible and the later tier is honest.
    #[test]
    fn a_tier_zero_prefix_reads_the_kit_and_names_the_hdri_tier() {
        let bytes = packed_bundle_with_hdri("prefix");
        let first = tier_zero_end(&bytes);
        let source = open_bundle_prefix(bytes[..first].to_vec()).expect("prefix");
        assert!(source.is_prefix());
        assert_eq!(
            &*source.read("cooked/sand_base.ktx2").expect("tier 0"),
            b"ktx2 bytes"
        );
        let err = source
            .read("hdri/studio.hdr")
            .expect_err("tier 1 is not here");
        assert!(
            err.to_string().contains("not in the fetched prefix"),
            "{err}"
        );
    }

    /// QW4: the engine's prefix path asks the host and opens only what came.
    #[test]
    fn the_engine_prefix_path_opens_the_fetched_prefix() {
        let bytes = packed_bundle_with_hdri("fetch-prefix");
        let first = tier_zero_end(&bytes);
        let prefix = bytes[..first].to_vec();
        let asked = Rc::new(RefCell::new(Vec::new()));
        let asked_in = Rc::clone(&asked);
        let source = ready(open_fetched_bundle_prefix(ARENA_BUNDLE_URL, move |url| {
            asked_in.borrow_mut().push(url.to_string());
            let prefix = prefix.clone();
            async move { Ok(prefix) }
        }))
        .expect("the prefix opens");
        assert_eq!(*asked.borrow(), [ARENA_BUNDLE_URL.to_string()]);
        assert_eq!(
            &*source.read("cooked/sand_base.ktx2").expect("tier 0"),
            b"ktx2 bytes"
        );
        assert!(source.read("hdri/studio.hdr").is_err());
    }

    /// A directory with one file, packed and read back through [`open_bundle`].
    #[test]
    fn a_packed_bundle_opens_and_reads_its_entries() {
        let source = open_bundle(packed_bundle("open")).expect("open");
        assert_eq!(
            &*source.read("cooked/sand_base.ktx2").expect("entry"),
            b"ktx2 bytes".as_slice()
        );
        assert!(source.read("cooked/missing.ktx2").is_err());
        // A torn body is refused, not silently empty.
        assert!(matches!(open_bundle(Vec::new()), Err(ContentError::Empty)));
        assert!(open_bundle(vec![0u8; 16]).is_err());
    }

    /// WEB6: the engine path asks the host for the asset and hands back its bytes.
    #[test]
    fn the_engine_path_asks_for_the_asset_and_returns_its_bytes() {
        let asked = RefCell::new(Vec::new());
        let bytes = packed_bundle("fetch");
        let source = ready(open_fetched_bundle(ARENA_BUNDLE_URL, |url| {
            asked.borrow_mut().push(url.to_string());
            let bytes = bytes.clone();
            async move { Ok(bytes) }
        }))
        .expect("the bundle opens");
        assert_eq!(asked.into_inner(), [ARENA_BUNDLE_URL.to_string()]);
        assert_eq!(
            asset_bytes(&source, "cooked/sand_base.ktx2").expect("the asset"),
            b"ktx2 bytes".to_vec()
        );
    }

    /// A missing asset is a typed refusal that names it: no panic, no empty bytes.
    #[test]
    fn a_missing_asset_is_a_typed_refusal() {
        let source = open_bundle(packed_bundle("missing")).expect("open");
        let err = asset_bytes(&source, "cooked/missing.ktx2").expect_err("no such asset");
        assert_eq!(
            err,
            ContentError::Missing {
                path: "cooked/missing.ktx2".into()
            }
        );
        assert!(err.to_string().contains("cooked/missing.ktx2"));
    }

    /// A refused fetch stays a typed refusal, and bytes that are not a bundle are
    /// refused by the format's own reason.
    #[test]
    fn a_refused_fetch_is_a_typed_refusal() {
        let err = ready(open_fetched_bundle(ARENA_BUNDLE_URL, |_url| async {
            Err(ContentError::Status(404))
        }))
        .expect_err("the fetch failed");
        assert_eq!(err, ContentError::Status(404));
        let err = ready(open_fetched_bundle(ARENA_BUNDLE_URL, |_url| async {
            Ok(vec![0u8; 16])
        }))
        .expect_err("not a bundle");
        assert!(matches!(err, ContentError::Bundle(_)), "{err:?}");
    }
}
