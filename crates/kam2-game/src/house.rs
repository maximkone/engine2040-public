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
//!
//! [`HouseArts`] is the registry on top of that: slot 0 holds the cottage and
//! slot `kind.index() + 1` the art of one house kind, so a kind with its own
//! building folder draws its own atlas and a kind without one falls back to
//! the cottage. The registry holds manifests only; the atlas bytes live in the
//! window binary, which uploads one sheet per slot.

use kam2_sim::HouseKind;
use serde_json::Value;

use crate::soldier::{pair_at, ManifestError, SoldierManifest};

/// House art slots: the cottage and one per house kind.
pub const HOUSE_SLOTS: usize = 6;

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

    /// Transparent gutter around one cell, in pixels.
    #[must_use]
    pub const fn padding(&self) -> u32 {
        self.sprite.padding
    }

    /// Cell pitch in atlas pixels: the cell plus its gutter on both sides.
    #[must_use]
    pub fn pitch(&self) -> [u32; 2] {
        self.sprite.pitch()
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

/// The house art registry: slot 0 holds the cottage, slot `kind.index() + 1`
/// the manifest of one house kind.
///
/// The scene asks the registry which atlas a house draws from, so a kind with
/// its own building folder draws its own art and a kind without one falls back
/// to the cottage. Only manifests live here; the atlas bytes belong to the
/// window binary, which uploads one sheet per slot.
#[derive(Clone, Debug, Default)]
pub struct HouseArts {
    slots: Vec<Option<HouseManifest>>,
}

impl HouseArts {
    /// An empty registry: no slot holds art yet.
    #[must_use]
    pub fn new() -> Self {
        Self {
            slots: vec![None; HOUSE_SLOTS],
        }
    }

    /// Put the cottage manifest in slot 0, the fallback of every kind.
    pub fn set_cottage(&mut self, manifest: HouseManifest) {
        self.set_slot(0, manifest);
    }

    /// Put one kind's own manifest in the slot of that kind.
    pub fn set_kind(&mut self, kind: HouseKind, manifest: HouseManifest) {
        self.set_slot(usize::from(kind.index()) + 1, manifest);
    }

    /// The manifest of one slot, when that slot holds art.
    #[must_use]
    pub fn manifest(&self, slot: usize) -> Option<&HouseManifest> {
        self.slots.get(slot).and_then(Option::as_ref)
    }

    /// The slot a kind draws from: its own when it has one, else the cottage.
    /// `None` when neither the kind nor the cottage is loaded.
    #[must_use]
    pub fn slot_for(&self, kind: HouseKind) -> Option<usize> {
        let own = usize::from(kind.index()) + 1;
        if self.slots.get(own).and_then(Option::as_ref).is_some() {
            Some(own)
        } else if self.slots.first().and_then(Option::as_ref).is_some() {
            Some(0)
        } else {
            None
        }
    }

    /// The art one kind draws: the slot, the manifest of that slot and whether
    /// the manifest is the kind's own rather than the cottage fallback.
    #[must_use]
    pub fn resolve(&self, kind: HouseKind) -> Option<HouseArt<'_>> {
        let slot = self.slot_for(kind)?;
        let manifest = self.slots.get(slot).and_then(Option::as_ref)?;
        Some(HouseArt {
            slot,
            manifest,
            own: slot != 0,
        })
    }

    /// Fill one slot; a slot outside the registry is ignored.
    fn set_slot(&mut self, slot: usize, manifest: HouseManifest) {
        if let Some(cell) = self.slots.get_mut(slot) {
            *cell = Some(manifest);
        }
    }
}

/// The art one house kind draws: a slot, its manifest and where the manifest
/// came from.
#[derive(Clone, Copy, Debug)]
pub struct HouseArt<'a> {
    /// The atlas slot the kind draws from.
    pub slot: usize,
    /// The manifest of that slot.
    pub manifest: &'a HouseManifest,
    /// `true` when the manifest is the kind's own, `false` for the cottage.
    pub own: bool,
}

/// The committed manifest, for tests that must catch content drift.
#[cfg(test)]
pub const MANIFEST_JSON: &str = include_str!("../../../content/kam2/house/manifest.json");

/// The committed manifest of every house kind, beside the cottage.
#[cfg(test)]
pub const BUILDING_MANIFESTS: [(HouseKind, &str); 5] = [
    (
        HouseKind::Warehouse,
        include_str!("../../../content/kam2/buildings/warehouse/manifest.json"),
    ),
    (
        HouseKind::Farm,
        include_str!("../../../content/kam2/buildings/farm/manifest.json"),
    ),
    (
        HouseKind::Mill,
        include_str!("../../../content/kam2/buildings/mill/manifest.json"),
    ),
    (
        HouseKind::Bakery,
        include_str!("../../../content/kam2/buildings/bakery/manifest.json"),
    ),
    (
        HouseKind::Woodcutter,
        include_str!("../../../content/kam2/buildings/woodcutter/manifest.json"),
    ),
];

#[cfg(test)]
mod tests {
    use super::*;

    const EPS: f32 = 1e-6;

    fn manifest() -> HouseManifest {
        HouseManifest::parse(MANIFEST_JSON).expect("the committed manifest parses")
    }

    /// The committed manifest as raw JSON: the authority every number below
    /// is derived from, so the test never repeats a hand-typed constant.
    fn manifest_json() -> Value {
        serde_json::from_str(MANIFEST_JSON).expect("the committed manifest is JSON")
    }

    /// One unsigned number of the committed JSON.
    fn number(json: &Value, name: &str) -> u32 {
        json.get(name)
            .and_then(Value::as_u64)
            .and_then(|value| u32::try_from(value).ok())
            .unwrap_or(0)
    }

    /// One `[x, y]` pair of the committed JSON.
    fn pair(json: &Value, name: &str) -> [u32; 2] {
        let values = json
            .get(name)
            .and_then(Value::as_array)
            .expect("the manifest holds the pair");
        [
            u32::try_from(values[0].as_u64().unwrap_or(0)).unwrap_or(0),
            u32::try_from(values[1].as_u64().unwrap_or(0)).unwrap_or(0),
        ]
    }

    /// The state rows the committed JSON lists.
    fn state_rows(json: &Value) -> u32 {
        u32::try_from(
            json.get("anims")
                .and_then(Value::as_array)
                .map(Vec::len)
                .unwrap_or(0),
        )
        .unwrap_or(0)
    }

    #[test]
    fn the_committed_manifest_parses_into_the_cottage_grid() {
        let manifest = manifest();
        let json = manifest_json();
        let tile = [number(&json, "tile_w"), number(&json, "tile_h")];
        let padding = number(&json, "padding");
        let rows = state_rows(&json);
        let atlas = [
            tile[0] + padding * 2,
            rows * (tile[1] + padding * 2),
        ];

        assert_eq!(manifest.id(), "house.cottage");
        assert_eq!(manifest.atlas(), "house.cottage.atlas.png");
        assert_eq!(manifest.mask_atlas(), None, "this look pass ships no mask");
        assert_eq!(manifest.tile(), tile);
        assert_eq!(manifest.padding(), padding);
        assert_eq!(manifest.pitch(), [tile[0] + padding * 2, tile[1] + padding * 2]);
        assert_eq!(manifest.atlas_size(), atlas);
        assert_eq!(manifest.pivot(), pair(&json, "pivot"));
        assert_eq!(manifest.footprint(), pair(&json, "footprint"));
        assert_eq!(manifest.columns(), 1, "buildings do not rotate");
        assert_eq!(manifest.rows(), rows, "one row per build state");
        assert_eq!(manifest.row(HouseState::Finished), Some(0));
        assert_eq!(manifest.row(HouseState::HalfBuilt), Some(1));
        assert_eq!(manifest.row(HouseState::Damaged), Some(2));
    }

    #[test]
    fn the_cell_uv_follows_the_committed_grid() {
        let manifest = manifest();
        let json = manifest_json();
        let tile = [number(&json, "tile_w"), number(&json, "tile_h")];
        let padding = number(&json, "padding");
        let rows = state_rows(&json);
        let atlas = [
            f64::from(tile[0] + padding * 2),
            f64::from(rows * (tile[1] + padding * 2)),
        ];
        for row in 0..rows {
            let [origin, size] = manifest.cell_uv(row).expect("the row is inside the grid");
            let expected_x = f64::from(padding) / atlas[0];
            let expected_y =
                f64::from(row * (tile[1] + padding * 2) + padding) / atlas[1];
            assert!(
                (f64::from(origin[0]) - expected_x).abs() < f64::from(EPS),
                "row {row} origin {origin:?}"
            );
            assert!(
                (f64::from(origin[1]) - expected_y).abs() < f64::from(EPS),
                "row {row} origin {origin:?}"
            );
            assert!((f64::from(size[0]) - f64::from(tile[0]) / atlas[0]).abs() < f64::from(EPS));
            assert!((f64::from(size[1]) - f64::from(tile[1]) / atlas[1]).abs() < f64::from(EPS));
        }
        assert_eq!(manifest.cell_uv(rows), None, "the atlas holds no row past the last");
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
        let pivot = manifest.pivot();
        let anchor = [640.0, 360.0];
        let top_left = manifest.top_left(anchor);
        assert_eq!(
            top_left,
            [
                anchor[0] - pivot[0] as f32,
                anchor[1] - pivot[1] as f32,
            ],
            "the cell starts that far above the anchor"
        );
        assert_eq!(
            manifest.size(),
            [manifest.tile()[0] as f32, manifest.tile()[1] as f32]
        );
        let ground = [
            top_left[0] + pivot[0] as f32,
            top_left[1] + pivot[1] as f32,
        ];
        assert_eq!(ground, anchor, "the footprint centre sits on the anchor");
    }

    #[test]
    fn every_committed_building_manifest_matches_the_sim_footprint() {
        for (kind, text) in BUILDING_MANIFESTS {
            let manifest = HouseManifest::parse(text).expect("the committed manifest parses");
            let (w, h) = kind.footprint();
            assert_eq!(
                manifest.footprint(),
                [
                    u32::try_from(w).unwrap_or(0),
                    u32::try_from(h).unwrap_or(0),
                ],
                "the {} manifest must draw the footprint the sim reserves",
                kind.name()
            );
            assert_eq!(manifest.columns(), 1, "{} does not rotate", kind.name());
        }
    }

    #[test]
    fn the_registry_serves_a_kind_its_own_slot_or_the_cottage() {
        let mut arts = HouseArts::new();
        assert_eq!(arts.slot_for(HouseKind::Farm), None, "no art is loaded yet");
        assert!(arts.resolve(HouseKind::Farm).is_none());

        arts.set_cottage(manifest());
        let cottage = arts.resolve(HouseKind::Farm).expect("the cottage answers");
        assert_eq!(cottage.slot, 0);
        assert!(!cottage.own, "a kind without its own art draws the cottage");

        let farm = HouseManifest::parse(BUILDING_MANIFESTS[1].1)
            .expect("the committed farm manifest parses");
        arts.set_kind(HouseKind::Farm, farm);
        let own = arts.resolve(HouseKind::Farm).expect("the farm answers");
        assert_eq!(own.slot, usize::from(HouseKind::Farm.index()) + 1);
        assert!(own.own, "the farm draws its own atlas");
        let mill = arts.resolve(HouseKind::Mill).expect("the mill falls back");
        assert_eq!(mill.slot, 0, "the mill has no art of its own");
        assert!(!mill.own);
        assert!(arts.manifest(0).is_some(), "the cottage stays in slot zero");
    }
}
