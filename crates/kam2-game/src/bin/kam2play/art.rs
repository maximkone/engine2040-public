//! The soldier and cottage art at runtime: manifest, atlas PNG and fallback.
//!
//! `content/kam2/soldier/` ships one packed atlas and `content/kam2/house/` the
//! cottage atlas. [`Art::load`] and [`HouseArt::load`] read their manifest and
//! PNG once, at startup; a missing soldier file falls back to engine-render's
//! stand-in cells, while an atlas that is present but broken fails the run
//! instead of quietly drawing the wrong art. The house atlas is required: the
//! field has no house boxes to fall back to. [`Art::source`] names the path
//! taken and goes into the receipt.
//!
//! The PNG decoder is the `image` crate this binary already asks for (feature
//! `window`, PNG only), not a new dependency.

use std::path::Path;

use engine_render::kam2_sprite_atlas::SpriteArt;
use kam2_game::{HouseManifest, SoldierManifest, Units};

use crate::PlayError;

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

/// The cottage art: one manifest, one atlas and the family's build states.
pub struct HouseArt {
    manifest: HouseManifest,
    sheet: Sheet,
    source: &'static str,
}

impl HouseArt {
    /// Load the packed cottage atlas from `dir`.
    ///
    /// # Errors
    ///
    /// Returns [`PlayError::Art`] for a missing manifest, an atlas that does not
    /// match the manifest's own size, or a manifest that refuses to parse;
    /// [`PlayError::Io`] when a present file cannot be read; and
    /// [`PlayError::Image`] when the PNG does not decode. The house art is not
    /// optional: the field no longer draws house boxes.
    pub fn load(dir: &Path) -> Result<Self, PlayError> {
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
        Ok(Self {
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

    /// The cottage manifest the scene reads cells from.
    pub fn houses(&self) -> &HouseManifest {
        &self.manifest
    }

    /// Atlas bytes for the sprite pass.
    pub fn sheet(&self) -> &Sheet {
        &self.sheet
    }

    /// Which path was taken; the cottage atlas is always required.
    pub fn source(&self) -> &'static str {
        self.source
    }
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

    /// The committed atlas folder, from the crate manifest.
    fn soldier_dir() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("content")
            .join("kam2")
            .join("soldier")
    }

    #[test]
    fn the_committed_atlas_decodes_to_its_manifest_size() {
        let art = Art::load(&soldier_dir()).expect("the committed atlas loads");
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

    /// The committed cottage folder, from the crate manifest.
    fn house_dir() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("content")
            .join("kam2")
            .join("house")
    }

    #[test]
    fn the_committed_cottage_atlas_decodes_to_its_manifest_size() {
        let art = HouseArt::load(&house_dir()).expect("the committed cottage atlas loads");
        assert_eq!(art.source(), "atlas");
        assert_eq!(art.houses().id(), "house.cottage");
        assert_eq!((art.sheet().width(), art.sheet().height()), (130, 390));
        assert_eq!(
            art.sheet().base().len(),
            130 * 390 * 4,
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
    fn a_missing_cottage_atlas_fails_the_run() {
        let missing = std::env::temp_dir().join("kam2-cottage-that-is-not-there");
        let error = match HouseArt::load(&missing) {
            Ok(_) => panic!("houses cannot fall back to boxes"),
            Err(error) => error,
        };
        assert!(matches!(error, PlayError::Art(_)), "{error:?}");
    }
}
