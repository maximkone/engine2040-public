//! fighter_bundle: the CT7 fighter rig as one bundle the page fetches.
//!
//! Consumer: the browser page fetches `assets/fighters.e2bundle` and opens it
//! with [`crate::asset_source::BundleSource`]; the page hands that source to
//! `engine_game::fighter_look::FighterLook::load_from`, which reads the same
//! bytes the native window reads from `content/fighters`. The pins stay with
//! the reader: the bind checks the manifest against the pinned CT7 SHAs and
//! the look checks every body against its pin, so a wrong file is refused at
//! load time, never here.
//!
//! Rig files: `manifest.json` (the bind reads it first), `glb/<variant>_body.glb`
//! (one per variant) and `glb/clips/<file>.glb` (the baked clips, shared by
//! every rig). Tiers: the manifest first, then the clips and the hero body
//! (the first rig a page can draw), then the other bodies (they refine the
//! fight later). The bytes of the tiers `0..=p` are one prefix of the file,
//! so a page can fetch the first rig alone and open it with
//! [`crate::asset_source::BundleSource::open_prefix`].
//!
//! Units: bytes and counts. Determinism: the pack is canonical - the same
//! files give the same bytes; no clock, no RNG. Errors: [`SourceError`] at
//! every boundary; a missing rig file is refused by name.
//! Budget: one-shot cook (a full read of the rig tree), never on a tick.

use std::sync::Arc;

use crate::asset_source::{pack_to, AssetSource, BundleSource, PackRules, SourceError};

/// The manifest: the bind reads it first, and it names the pins.
pub const MANIFEST: &str = "manifest.json";
/// The rig directory: the bodies and the baked clips live under it.
pub const RIG_DIR: &str = "glb";
/// The clip directory: the baked clips, shared by every rig.
pub const CLIP_DIR: &str = "glb/clips";
/// The hero body: the first rig a page can draw (slot 0 is the hero).
pub const HERO_BODY: &str = "glb/hero_body.glb";

/// Tier of the manifest.
pub const TIER_MANIFEST: u8 = 0;
/// Tier of the baked clips and the hero body.
pub const TIER_FIRST_RIG: u8 = 1;
/// Tier of the other bodies.
pub const TIER_OTHER_RIGS: u8 = 2;

/// The byte bar the first rig prefix carries: the manifest, the clips and the
/// hero body in one fetch (the page's first fighter fetch). The other bodies
/// land in a later fetch. The bar leaves room for the next content drop
/// before the page has to stream the rig in pieces.
pub const FIRST_PREFIX_BYTES: usize = 4_000_000;

/// What one cook produced.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cooked {
    /// The bundle bytes: the `E2BUNDLE` file the page fetches.
    pub bytes: Arc<[u8]>,
    /// Files packed.
    pub entries: usize,
    /// Bytes per tier, tier 0 first.
    pub tier_bytes: [u64; 3],
    /// Bytes of the first rig prefix: the header, the index and the blobs of
    /// the tiers `0..=1`.
    pub first_prefix_bytes: u64,
    /// The bar the prefix must stay under ([`FIRST_PREFIX_BYTES`]).
    pub bar: usize,
    /// Digest of the packed files, as the pack recorded it.
    pub digest: u64,
}

impl Cooked {
    /// True when the first rig prefix fits under the bar.
    #[must_use]
    pub fn first_prefix_fits(&self) -> bool {
        self.first_prefix_bytes <= u64::try_from(self.bar).unwrap_or(u64::MAX)
    }
}

/// Cook the fighter rig out of a source into one bundle the page can fetch.
///
/// The source holds the rig tree: `manifest.json` beside `glb/`. The rig files
/// the page needs first (the manifest, the clips and the hero body) must be
/// there; a missing one is refused by name.
pub fn cook(source: &dyn AssetSource) -> Result<Cooked, SourceError> {
    require(source, MANIFEST)?;
    require(source, HERO_BODY)?;
    if !source.list(CLIP_DIR).is_ok_and(|clips| !clips.is_empty()) {
        return Err(SourceError::missing(CLIP_DIR));
    }
    let rig_prefix = format!("{RIG_DIR}/");
    let clip_prefix = format!("{CLIP_DIR}/");
    let rules = PackRules::new()
        .include(MANIFEST)
        .include(&rig_prefix)
        .priority(MANIFEST, TIER_MANIFEST)
        .priority(&clip_prefix, TIER_FIRST_RIG)
        .priority(HERO_BODY, TIER_FIRST_RIG)
        .priority(&rig_prefix, TIER_OTHER_RIGS);
    let mut bytes = Vec::new();
    let report = pack_to(source, &rules, &mut bytes)?;
    let bytes: Arc<[u8]> = Arc::from(bytes);
    let bundle = BundleSource::open(Arc::clone(&bytes))?;
    let (tier_bytes, first_prefix_bytes) = tiers(&bundle);
    Ok(Cooked {
        bytes,
        entries: report.entries,
        tier_bytes,
        first_prefix_bytes,
        bar: FIRST_PREFIX_BYTES,
        digest: report.digest,
    })
}

/// Refuse a missing rig file by name.
fn require(source: &dyn AssetSource, path: &str) -> Result<(), SourceError> {
    if source.exists(path) {
        Ok(())
    } else {
        Err(SourceError::missing(path))
    }
}

/// The bytes per tier and the length of the first rig prefix (the header, the
/// index and the blobs of the tiers `0..=1`).
fn tiers(bundle: &BundleSource) -> ([u64; 3], u64) {
    let mut tier_bytes = [0u64; 3];
    let mut prefix = 0u64;
    for entry in bundle.entries_in_file_order() {
        let len = u64::try_from(entry.len).unwrap_or(u64::MAX);
        if let Some(slot) = tier_bytes.get_mut(usize::from(entry.priority)) {
            *slot += len;
        }
        if entry.priority <= TIER_FIRST_RIG {
            let end = entry.offset.saturating_add(entry.len);
            let end = u64::try_from(end).unwrap_or(u64::MAX);
            prefix = prefix.max(end);
        }
    }
    (tier_bytes, prefix)
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use crate::asset_source::MemorySource;

    /// A rig tree: the manifest, two bodies and two clips.
    fn rig() -> MemorySource {
        let mut source = MemorySource::new();
        source
            .insert(MANIFEST, b"{\"variants\":[]}".to_vec())
            .expect("manifest");
        source.insert(HERO_BODY, vec![1u8; 64]).expect("hero body");
        source
            .insert("glb/opp_male_light_body.glb", vec![2u8; 48])
            .expect("opponent body");
        source
            .insert("glb/clips/human_male_idle.glb", vec![3u8; 16])
            .expect("idle clip");
        source
            .insert("glb/clips/human_male_run.glb", vec![4u8; 16])
            .expect("run clip");
        source
    }

    #[test]
    fn the_cook_packs_the_rig_into_tiers() {
        let cooked = cook(&rig()).expect("the rig cooks");
        assert_eq!(cooked.entries, 5);
        let bundle = BundleSource::open(Arc::clone(&cooked.bytes)).expect("opens");
        assert_eq!(bundle.list("").expect("list").len(), 5);
        assert!(cooked.tier_bytes[0] > 0, "the manifest is tier 0");
        assert!(
            cooked.tier_bytes[1] > 0,
            "the clips and the hero body are tier 1"
        );
        assert!(cooked.tier_bytes[2] > 0, "the other bodies are tier 2");
        let total = u64::try_from(cooked.bytes.len()).unwrap_or(u64::MAX);
        assert!(
            cooked.first_prefix_bytes < total,
            "the first rig is a prefix of the whole bundle"
        );
        assert!(cooked.first_prefix_fits(), "the first rig fits the bar");
    }

    #[test]
    fn the_first_prefix_carries_the_first_rig_and_names_what_it_lacks() {
        let cooked = cook(&rig()).expect("the rig cooks");
        let cut = usize::try_from(cooked.first_prefix_bytes).expect("the prefix fits");
        let prefix =
            BundleSource::open_prefix(cooked.bytes[..cut].to_vec()).expect("the prefix opens");
        assert_eq!(
            prefix.read(MANIFEST).expect("the manifest").as_ref(),
            b"{\"variants\":[]}".as_slice()
        );
        assert_eq!(prefix.read(HERO_BODY).expect("the hero body").len(), 64);
        assert!(
            prefix.read("glb/clips/human_male_idle.glb").is_ok(),
            "the clips are in the first rig"
        );
        let err = prefix
            .read("glb/opp_male_light_body.glb")
            .expect_err("the other body is not here yet");
        let text = err.to_string();
        assert!(
            text.contains("opp_male_light_body.glb"),
            "the refusal names the file: {text}"
        );
    }

    #[test]
    fn the_cook_gives_the_same_bytes_twice() {
        let first = cook(&rig()).expect("the rig cooks");
        let second = cook(&rig()).expect("the rig cooks");
        assert_eq!(
            first.bytes, second.bytes,
            "the same files give the same bytes"
        );
        assert_eq!(first.digest, second.digest, "and the same digest");
    }

    #[test]
    fn a_rig_without_the_manifest_is_refused_by_name() {
        let mut source = MemorySource::new();
        source.insert(HERO_BODY, vec![1u8; 8]).expect("hero body");
        let err = cook(&source).expect_err("no manifest, no bundle");
        assert!(
            err.to_string().contains(MANIFEST),
            "the refusal names the file: {err}"
        );
    }

    #[test]
    fn a_rig_without_clips_is_refused_by_name() {
        let mut source = MemorySource::new();
        source.insert(MANIFEST, b"{}".to_vec()).expect("manifest");
        source.insert(HERO_BODY, vec![1u8; 8]).expect("hero body");
        let err = cook(&source).expect_err("no clips, no bundle");
        assert!(
            err.to_string().contains(CLIP_DIR),
            "the refusal names the dir: {err}"
        );
    }
}
