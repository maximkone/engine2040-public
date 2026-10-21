//! The soldier and house art at runtime: manifest, atlas PNG and fallback.
//!
//! `content/kam2/soldier/` ships one packed atlas, `content/kam2/house/` the
//! cottage atlas and `content/kam2/buildings/<kind>/` one atlas per house
//! kind. [`Art::load`] reads the soldier files once, at startup; a missing
//! soldier file falls back to engine-render's stand-in cells, while an atlas
//! that is present but broken fails the run instead of quietly drawing the
//! wrong art. [`BuildingArts`] does the same for the houses: the cottage is
//! required, a house kind without a building folder falls back to the
//! cottage, and a folder that is there but broken fails the run. The
//! [`Art::source`] and [`BuildingArts::source`] paths go into the receipt.
//!
//! The PNG decoder is the `image` crate this binary already asks for (feature
//! `window`, PNG only), not a new dependency.

use std::path::{Path, PathBuf};

use engine_render::kam2_sprite_atlas::SpriteArt;
use kam2_game::{HouseArts, HouseManifest, SoldierManifest, Units, HOUSE_SLOTS};
use kam2_sim::HouseKind;

use crate::PlayError;

/// Every house kind, in registry order: the slot of a kind is its index plus
/// one, slot zero being the cottage.
pub const HOUSE_KINDS: [HouseKind; 5] = [
    HouseKind::Warehouse,
    HouseKind::Farm,
    HouseKind::Mill,
    HouseKind::Bakery,
    HouseKind::Woodcutter,
];

/// Atlas pixels the sprite pass uploads: base colour and team mask, RGBA8.
pub struct Sheet {
    width: u32,
    height: u32,
    base: Vec<u8>,
    mask: Vec<u8>,
}

impl Sheet {
    /// Atlas width in pixels.
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Atlas height in pixels.
    pub fn height(&self) -> u32 {
        self.height
    }

    /// Base colour bytes, top row first.
    pub fn base(&self) -> &[u8] {
        &self.base
    }

    /// Team mask bytes, top row first; alpha 0 means no tint.
    pub fn mask(&self) -> &[u8] {
        &self.mask
    }
}

/// The unit art the window draws and the atlas bytes behind it.
pub struct Art {
    units: Units,
    sheet: Sheet,
    source: &'static str,
}

impl Art {
    /// Load the packed soldier atlas from `dir`, or the stand-in when a file
    /// is missing.
    ///
    /// # Errors
    ///
    /// Returns [`PlayError::Art`] for a manifest or PNG that does not match the
    /// manifest's own atlas size, [`PlayError::Io`] when a present file cannot
    /// be read, and [`PlayError::Image`] when the PNG does not decode.
    pub fn load(dir: &Path) -> Result<Self, PlayError> {
        let manifest_path = dir.join("manifest.json");
        let text = match std::fs::read_to_string(&manifest_path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::stand_in())
            }
            Err(error) => return Err(PlayError::Io(error)),
        };
        let manifest = SoldierManifest::parse(&text)
            .map_err(|error| PlayError::Art(format!("{}: {error}", manifest_path.display())))?;
        let atlas_path = dir.join(&manifest.atlas);
        let Some((width, height, base)) = decode_if_present(&atlas_path)? else {
            return Ok(Self::stand_in());
        };
        if [width, height] != manifest.atlas_size {
            return Err(PlayError::Art(format!(
                "{} is {width}x{height}, the manifest says {}x{}",
                atlas_path.display(),
                manifest.atlas_size[0],
                manifest.atlas_size[1]
            )));
        }
        let mask = match manifest.mask_atlas.as_deref() {
            Some(name) => {
                let path = dir.join(name);
                let (mask_w, mask_h, mask) = decode(&path)?;
                if [mask_w, mask_h] != manifest.atlas_size {
                    return Err(PlayError::Art(format!(
                        "{} is {mask_w}x{mask_h}, the manifest says {}x{}",
                        path.display(),
                        manifest.atlas_size[0],
                        manifest.atlas_size[1]
                    )));
                }
                mask
            }
            // No mask atlas: a zero mask keeps every baked pixel as it is.
            None => vec![0u8; base.len()],
        };
        Ok(Self {
            units: Units::Soldier(manifest),
            sheet: Sheet {
                width,
                height,
                base,
                mask,
            },
            source: "atlas",
        })
    }

    /// The unit art source the scene and the GPU share.
    pub fn units(&self) -> &Units {
        &self.units
    }

    /// Atlas bytes for the sprite pass.
    pub fn sheet(&self) -> &Sheet {
        &self.sheet
    }

    /// Which path was taken: `atlas` or `standin`.
    pub fn source(&self) -> &'static str {
        self.source
    }

    /// engine-render's procedural cells, drawn when the atlas file is missing.
    fn stand_in() -> Self {
        let art = SpriteArt::build();
        Self {
            units: Units::StandIn,
            sheet: Sheet {
                width: art.width(),
                height: art.height(),
                base: art.base_bytes().to_vec(),
                mask: art.mask_bytes().to_vec(),
            },
            source: "standin",
        }
    }
}

/// One loaded house atlas: its manifest, its bytes and the path it took.
struct LoadedHouse {
    manifest: HouseManifest,
    sheet: Sheet,
    source: &'static str,
}

/// The house art of the window: the cottage in slot zero and one slot per
/// house kind.
///
/// The scene asks the [`HouseArts`] registry which atlas a house kind draws
/// from; the window uploads one sheet per slot. A kind whose building folder
/// is missing keeps slot zero and draws the cottage, so the field never shows
/// an empty hole where a house stands.
pub struct BuildingArts {
    cottage_dir: PathBuf,
    buildings_dir: PathBuf,
    slots: Vec<Option<LoadedHouse>>,
    registry: HouseArts,
}

impl BuildingArts {
    /// The empty registry; no slot holds art yet.
    #[must_use]
    pub fn new(cottage_dir: &Path, buildings_dir: &Path) -> Self {
        Self {
            cottage_dir: cottage_dir.to_path_buf(),
            buildings_dir: buildings_dir.to_path_buf(),
            slots: (0..HOUSE_SLOTS).map(|_| None).collect(),
            registry: HouseArts::new(),
        }
    }

    /// Load the cottage and every house kind that ships a building folder.
    ///
    /// # Errors
    ///
    /// Returns [`PlayError`] for a missing or broken cottage atlas and for a
    /// building folder that is there but does not match its own manifest.
    pub fn load_all(&mut self) -> Result<(), PlayError> {
        self.load_cottage()?;
        for kind in HOUSE_KINDS {
            self.ensure(kind)?;
        }
        Ok(())
    }

    /// Load the cottage atlas. The house art is not optional: the field no
    /// longer draws house boxes.
    ///
    /// # Errors
    ///
    /// Returns [`PlayError::Art`] for a missing manifest, an atlas that does
    /// not match the manifest's own size, or a manifest that refuses to parse;
    /// [`PlayError::Io`] when a present file cannot be read; and
    /// [`PlayError::Image`] when the PNG does not decode.
    pub fn load_cottage(&mut self) -> Result<(), PlayError> {
        let loaded = load_house(&self.cottage_dir)?;
        self.registry.set_cottage(loaded.manifest.clone());
        self.put(0, loaded);
        Ok(())
    }

    /// Load one kind's own atlas when its building folder ships one.
    ///
    /// # Errors
    ///
    /// Returns [`PlayError`] for a folder that is there but broken.
    pub fn ensure(&mut self, kind: HouseKind) -> Result<bool, PlayError> {
        let slot = slot_of(kind);
        if self.slots.get(slot).and_then(Option::as_ref).is_some() {
            return Ok(true);
        }
        let dir = self.buildings_dir.join(kind.name());
        if !dir.join("manifest.json").is_file() {
            return Ok(false);
        }
        let loaded = load_house(&dir)?;
        self.registry.set_kind(kind, loaded.manifest.clone());
        self.put(slot, loaded);
        Ok(true)
    }

    /// The manifest registry the scene reads.
    #[must_use]
    pub fn registry(&self) -> &HouseArts {
        &self.registry
    }

    /// The atlas bytes of one slot.
    #[must_use]
    pub fn sheet(&self, slot: usize) -> Option<&Sheet> {
        self.slots
            .get(slot)
            .and_then(Option::as_ref)
            .map(|loaded| &loaded.sheet)
    }

    /// Which path one slot took: `atlas` for the packed PNG, `cottage` when
    /// the slot fell back to the fallback art.
    #[must_use]
    pub fn source(&self, slot: usize) -> &'static str {
        self.slots
            .get(slot)
            .and_then(Option::as_ref)
            .map_or("cottage", |loaded| loaded.source)
    }

    /// Put one loaded atlas in its slot.
    fn put(&mut self, slot: usize, loaded: LoadedHouse) {
        if let Some(cell) = self.slots.get_mut(slot) {
            *cell = Some(loaded);
        }
    }
}

/// The atlas slot of one house kind: the kind's index plus one.
#[must_use]
pub fn slot_of(kind: HouseKind) -> usize {
    usize::from(kind.index()) + 1
}

/// Load one house atlas folder: its manifest and its packed PNG.
fn load_house(dir: &Path) -> Result<LoadedHouse, PlayError> {
    let manifest_path = dir.join("manifest.json");
    let text = match std::fs::read_to_string(&manifest_path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(PlayError::Art(format!(
                "{}: missing",
                manifest_path.display()
            )))
        }
        Err(error) => return Err(PlayError::Io(error)),
    };
    let manifest = HouseManifest::parse(&text)
        .map_err(|error| PlayError::Art(format!("{}: {error}", manifest_path.display())))?;
    let atlas_path = dir.join(manifest.atlas());
    let (width, height, base) = decode(&atlas_path)?;
    if [width, height] != manifest.atlas_size() {
        return Err(PlayError::Art(format!(
            "{} is {width}x{height}, the manifest says {}x{}",
            atlas_path.display(),
            manifest.atlas_size()[0],
            manifest.atlas_size()[1]
        )));
    }
    let mask = match manifest.mask_atlas() {
        Some(name) => {
            let path = dir.join(name);
            let (mask_w, mask_h, mask) = decode(&path)?;
            if [mask_w, mask_h] != manifest.atlas_size() {
                return Err(PlayError::Art(format!(
                    "{} is {mask_w}x{mask_h}, the manifest says {}x{}",
                    path.display(),
                    manifest.atlas_size()[0],
                    manifest.atlas_size()[1]
                )));
            }
            mask
        }
        // No mask atlas: a zero mask keeps every baked pixel as it is.
        None => vec![0u8; base.len()],
    };
    Ok(LoadedHouse {
        manifest,
        sheet: Sheet {
            width,
            height,
            base,
            mask,
        },
        source: "atlas",
    })
}

/// Decode one PNG, or `None` when the file does not exist.
fn decode_if_present(path: &Path) -> Result<Option<(u32, u32, Vec<u8>)>, PlayError> {
    match std::fs::read(path) {
        Ok(bytes) => {
            let image = image::load_from_memory_with_format(&bytes, image::ImageFormat::Png)
                .map_err(|error| PlayError::Image(format!("{}: {error}", path.display())))?;
            let rgba = image.to_rgba8();
            Ok(Some((rgba.width(), rgba.height(), rgba.into_raw())))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(PlayError::Io(error)),
    }
}

/// Decode one PNG that has to be there.
fn decode(path: &Path) -> Result<(u32, u32, Vec<u8>), PlayError> {
    decode_if_present(path)?
        .ok_or_else(|| PlayError::Art(format!("{}: missing beside the manifest", path.display())))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The committed content folder, from the crate manifest.
    fn content_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("content")
            .join("kam2")
    }

    #[test]
    fn the_committed_atlas_decodes_to_its_manifest_size() {
        let art = Art::load(&content_dir().join("soldier")).expect("the committed atlas loads");
        assert_eq!(art.source(), "atlas");
        assert!(matches!(art.units(), Units::Soldier(_)));
        assert_eq!((art.sheet().width(), art.sheet().height()), (528, 594));
        assert_eq!(
            art.sheet().base().len(),
            528 * 594 * 4,
            "one RGBA byte quad per atlas pixel"
        );
        assert_eq!(
            art.sheet().mask().len(),
            art.sheet().base().len(),
            "no mask atlas uploads a zero mask of the same size"
        );
        assert!(
            art.sheet().base().chunks_exact(4).any(|pixel| pixel[3] > 0),
            "the atlas is not all transparent"
        );
    }

    #[test]
    fn a_missing_atlas_falls_back_to_the_stand_in() {
        let missing = std::env::temp_dir().join("kam2-atlas-that-is-not-there");
        let art = Art::load(&missing).expect("a missing atlas is a fallback, not an error");
        assert_eq!(art.source(), "standin");
        assert_eq!(art.units(), &Units::StandIn);
        assert_eq!(
            (art.sheet().width(), art.sheet().height()),
            (48 * 8, 40 * 4)
        );
        assert_eq!(
            art.sheet().base().len(),
            art.sheet().mask().len(),
            "the stand-in carries its team mask"
        );
    }

    #[test]
    fn the_committed_house_atlases_decode_to_their_manifest_sizes() {
        let root = content_dir();
        let mut arts = BuildingArts::new(&root.join("house"), &root.join("buildings"));
        arts.load_all().expect("the committed house art loads");
        assert_eq!(arts.source(0), "atlas", "the cottage atlas is loaded");
        assert_eq!(arts.sheet(0).map(Sheet::width), Some(226));
        assert_eq!(arts.sheet(0).map(Sheet::height), Some(678));
        for kind in HOUSE_KINDS {
            let slot = slot_of(kind);
            assert_eq!(arts.source(slot), "atlas", "{}", kind.name());
            let sheet = arts.sheet(slot).expect("the slot holds an atlas");
            let manifest = arts
                .registry()
                .manifest(slot)
                .expect("the manifest of the slot is loaded");
            assert_eq!(
                (sheet.width(), sheet.height()),
                (manifest.atlas_size()[0], manifest.atlas_size()[1]),
                "{}",
                kind.name()
            );
            let pixels = usize::try_from(sheet.width() * sheet.height() * 4).unwrap_or(0);
            assert_eq!(sheet.base().len(), pixels, "one RGBA quad per pixel");
            assert_eq!(sheet.mask().len(), pixels, "a zero mask of the same size");
            assert!(
                sheet.base().chunks_exact(4).any(|pixel| pixel[3] > 0),
                "{} is not all transparent",
                kind.name()
            );
        }
    }

    #[test]
    fn every_kind_resolves_to_its_own_slot() {
        let root = content_dir();
        let mut arts = BuildingArts::new(&root.join("house"), &root.join("buildings"));
        arts.load_all().expect("the committed house art loads");
        for kind in HOUSE_KINDS {
            let art = arts.registry().resolve(kind).expect("the kind answers");
            assert_eq!(art.slot, slot_of(kind), "{}", kind.name());
            assert!(art.own, "{} draws its own atlas", kind.name());
        }
    }

    #[test]
    fn a_missing_cottage_atlas_fails_the_run() {
        let missing = std::env::temp_dir().join("kam2-cottage-that-is-not-there");
        let mut arts = BuildingArts::new(&missing, &missing);
        let error = match arts.load_cottage() {
            Ok(()) => panic!("houses cannot fall back to boxes"),
            Err(error) => error,
        };
        assert!(matches!(error, PlayError::Art(_)), "{error:?}");
    }

    #[test]
    fn a_kind_without_a_building_folder_falls_back_to_the_cottage() {
        let root = content_dir();
        let missing = root.join("no-such-buildings");
        let mut arts = BuildingArts::new(&root.join("house"), &missing);
        arts.load_cottage().expect("the cottage loads");
        assert!(
            !arts
                .ensure(HouseKind::Mill)
                .expect("a missing folder is not an error"),
            "the mill has no building folder here"
        );
        let art = arts
            .registry()
            .resolve(HouseKind::Mill)
            .expect("the cottage answers");
        assert_eq!(art.slot, 0, "the mill falls back to the cottage");
        assert!(!art.own);
        assert_eq!(arts.source(slot_of(HouseKind::Mill)), "cottage");
    }
}
