//! The packed cottage atlas: manifest fields, build states, cell and pivot.
//!
//! `content/kam2/house/` holds one real building atlas built by
//! `tools/kam2/pack_atlas.mjs`: one column (buildings do not rotate), one row
//! per state (finished, half_built, damaged), one transparent gutter pixel
//! around every 128 px cell. This module owns the arithmetic that turns a sim
//! house and the sim tick into one atlas cell and that cell into a screen
//! rectangle whose pivot sits on the footprint centre. It reads no file and
//! holds no GPU type, so the window, the scene and the tests share one answer.
//!
//! The manifest is the authority: cell size, padding, pivot, footprint and the
//! state rows come from it, never from a constant here.
//!
//! `kam2_sim::House` exposes no damage field, so [`HouseState::Damaged`] is
//! never chosen at runtime: a house that is no longer half built draws the
//! finished cell. The damaged row exists in the atlas and in [`HouseManifest`]
//! so a future public damage value can select it without a new asset.

use serde_json::Value;

use crate::soldier::{pair_at, ManifestError, SoldierManifest};

/// Which cell row of the cottage atlas a house draws.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HouseState {
    /// The working cottage.
    Finished,
    /// Foundations and walls before the first working tick.
    HalfBuilt,
    /// The ruined cell; the sim has no public damage field yet.
    Damaged,
}

impl HouseState {
    /// Manifest animation name of this state.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Finished => "finished",
            Self::HalfBuilt => "half_built",
            Self::Damaged => "damaged",
        }
    }

    /// The state of a house at sim `tick`: half built before its first working
    /// tick, finished from `built_at` on.
    #[must_use]
    pub const fn for_tick(tick: u32, built_at: u32) -> Self {
        if tick < built_at {
            Self::HalfBuilt
        } else {
            Self::Finished
        }
    }
}

/// The cottage manifest, parsed and checked against its atlas size.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HouseManifest {
    /// Grid, pivot and state rows: the shared atlas manifest parser owns them.
    sprite: SoldierManifest,
    /// Footprint in tiles, width and height.
    footprint: [u32; 2],
}

impl HouseManifest {
    /// Parse one manifest text and check it against its own atlas size.
    ///
    /// # Errors
    ///
    /// Returns [`ManifestError`] for a missing or mistyped field, a cell size of
    /// zero, an atlas that is not a whole number of cells, an empty state list,
    /// a state row outside the atlas, a pivot outside its frame, or an empty
    /// footprint side.
    pub fn parse(text: &str) -> Result<Self, ManifestError> {
        let root: Value =
            serde_json::from_str(text).map_err(|error| ManifestError::Json(error.to_string()))?;
        let sprite = SoldierManifest::parse(text)?;
        if sprite.anims.is_empty() {
            return Err(ManifestError::Grid("the manifest lists no build state"));
        }
        let footprint = pair_at(&root, "footprint", "footprint")?;
        if footprint[0] == 0 || footprint[1] == 0 {
            return Err(ManifestError::Grid("a footprint side is zero"));
        }
        Ok(Self { sprite, footprint })
    }

    /// Stable asset id, e.g. `house.cottage`.
    #[must_use]
    pub fn id(&self) -> &str {
        &self.sprite.id
    }

    /// Atlas file name beside the manifest.
    #[must_use]
    pub fn atlas(&self) -> &str {
        &self.sprite.atlas
    }

    /// Optional team-mask atlas file name; `None` means no tint pass.
    #[must_use]
    pub fn mask_atlas(&self) -> Option<&str> {
        self.sprite.mask_atlas.as_deref()
    }

    /// Atlas size in pixels.
    #[must_use]
    pub const fn atlas_size(&self) -> [u32; 2] {
        self.sprite.atlas_size
    }

    /// One cell without the gutter, in pixels.
    #[must_use]
    pub const fn tile(&self) -> [u32; 2] {
        self.sprite.tile
    }

    /// Ground anchor inside one cell, from its top-left.
    #[must_use]
    pub const fn pivot(&self) -> [u32; 2] {
        self.sprite.pivot
    }

    /// Footprint in tiles, width and height.
    #[must_use]
    pub const fn footprint(&self) -> [u32; 2] {
        self.footprint
    }

    /// Atlas rows: one per build state, in the manifest's row order.
    #[must_use]
    pub fn rows(&self) -> u32 {
        self.sprite.rows()
    }

    /// Atlas columns: one per facing; buildings keep the first.
    #[must_use]
    pub fn columns(&self) -> u32 {
        self.sprite.columns()
    }

    /// First atlas row of `state`, or of the finished cell when the manifest
    /// does not name that state. `None` only when the manifest has no cells.
    #[must_use]
    pub fn row(&self, state: HouseState) -> Option<u32> {
        let anim = self
            .sprite
            .anim(state.name())
            .or_else(|| self.sprite.anim(HouseState::Finished.name()))?;
        Some(anim.row)
    }

    /// Normalized UV origin and size of the state cell. Buildings never rotate,
    /// so the cell is in column zero.
    #[must_use]
    pub fn cell_uv(&self, row: u32) -> Option<[[f32; 2]; 2]> {
        self.sprite.cell_uv(row, 0)
    }

    /// Sprite size in framebuffer pixels at 100% zoom: the cell, not the pitch.
    #[must_use]
    pub fn size(&self) -> [f32; 2] {
        self.sprite.size()
    }

    /// Top-left framebuffer pixel of a cell whose pivot sits on `anchor`, the
    /// footprint centre on the ground plane.
    #[must_use]
    pub fn top_left(&self, anchor: [f32; 2]) -> [f32; 2] {
        self.sprite.top_left(anchor)
    }
}

/// The committed manifest, for tests that must catch content drift.
#[cfg(test)]
pub const MANIFEST_JSON: &str = include_str!("../../../content/kam2/house/manifest.json");

#[cfg(test)]
mod tests {
    use super::*;

    const EPS: f32 = 1e-6;

    fn manifest() -> HouseManifest {
        HouseManifest::parse(MANIFEST_JSON).expect("the committed manifest parses")
    }

    #[test]
    fn the_committed_manifest_parses_into_the_cottage_grid() {
        let manifest = manifest();
        assert_eq!(manifest.id(), "house.cottage");
        assert_eq!(manifest.atlas(), "house.cottage.atlas.png");
        assert_eq!(manifest.mask_atlas(), None, "this look pass ships no mask");
        assert_eq!(manifest.atlas_size(), [130, 390]);
        assert_eq!(manifest.tile(), [128, 128]);
        assert_eq!(manifest.pivot(), [64, 95]);
        assert_eq!(manifest.footprint(), [2, 2]);
        assert_eq!(manifest.columns(), 1, "buildings do not rotate");
        assert_eq!(manifest.rows(), 3, "one row per build state");
        assert_eq!(manifest.row(HouseState::Finished), Some(0));
        assert_eq!(manifest.row(HouseState::HalfBuilt), Some(1));
        assert_eq!(manifest.row(HouseState::Damaged), Some(2));

        let [origin, size] = manifest
            .cell_uv(1)
            .expect("the half-built row is inside the grid");
        assert!((origin[0] - 1.0 / 130.0).abs() < EPS, "{origin:?}");
        assert!((origin[1] - 131.0 / 390.0).abs() < EPS, "{origin:?}");
        assert!((size[0] - 128.0 / 130.0).abs() < EPS);
        assert!((size[1] - 128.0 / 390.0).abs() < EPS);
        assert_eq!(manifest.cell_uv(3), None, "the atlas holds three rows");
    }

    #[test]
    fn the_state_follows_the_first_working_tick() {
        assert_eq!(HouseState::for_tick(0, 30), HouseState::HalfBuilt);
        assert_eq!(HouseState::for_tick(29, 30), HouseState::HalfBuilt);
        assert_eq!(HouseState::for_tick(30, 30), HouseState::Finished);
        assert_eq!(HouseState::for_tick(31, 30), HouseState::Finished);
        assert_eq!(
            HouseState::for_tick(0, 0),
            HouseState::Finished,
            "a warehouse works from the tick it is placed"
        );
        assert_eq!(HouseState::HalfBuilt.name(), "half_built");
    }

    #[test]
    fn a_manifest_without_a_state_row_falls_back_to_finished() {
        let mut value: Value =
            serde_json::from_str(MANIFEST_JSON).expect("the committed manifest is JSON");
        value["anims"] = serde_json::json!([{
            "name": "finished", "frames": 1, "fps": 8, "loop": true, "row": 0
        }]);
        let sparse = HouseManifest::parse(&value.to_string()).expect("one state still parses");
        assert_eq!(sparse.row(HouseState::HalfBuilt), Some(0));
        assert_eq!(sparse.row(HouseState::Damaged), Some(0));

        let empty = serde_json::json!({
            "id": "house.cottage", "atlas": "a.png", "mask_atlas": null,
            "atlas_size": [130, 390], "tile_w": 128, "tile_h": 128,
            "padding": 1, "pivot": [64, 95], "footprint": [2, 2],
            "facings": ["S"], "anims": []
        });
        assert_eq!(
            HouseManifest::parse(&empty.to_string()),
            Err(ManifestError::Grid("the manifest lists no build state"))
        );
    }

    #[test]
    fn a_zero_footprint_is_refused() {
        let mut value: Value =
            serde_json::from_str(MANIFEST_JSON).expect("the committed manifest is JSON");
        value["footprint"] = serde_json::json!([2, 0]);
        assert_eq!(
            HouseManifest::parse(&value.to_string()),
            Err(ManifestError::Grid("a footprint side is zero"))
        );
        value["footprint"] = serde_json::json!([2]);
        assert_eq!(
            HouseManifest::parse(&value.to_string()),
            Err(ManifestError::Field("footprint"))
        );
    }

    #[test]
    fn the_pivot_puts_the_footprint_centre_on_the_ground_pixel() {
        let manifest = manifest();
        let anchor = [640.0, 360.0];
        let top_left = manifest.top_left(anchor);
        assert_eq!(top_left, [576.0, 265.0], "the pivot is 64 x 95 inside");
        assert_eq!(manifest.size(), [128.0, 128.0]);
        let pivot = [
            top_left[0] + manifest.pivot()[0] as f32,
            top_left[1] + manifest.pivot()[1] as f32,
        ];
        assert_eq!(pivot, anchor, "the footprint centre sits on the anchor");
    }
}
