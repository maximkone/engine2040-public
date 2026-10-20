//! The packed soldier atlas: manifest fields, cell grid, walk frame and pivot.
//!
//! `content/kam2/soldier/` holds one real atlas built by `tools/kam2/pack_atlas.mjs`:
//! column = facing in style-lock order (S..SE), row = animation frame, one
//! transparent gutter pixel around every cell. This module owns the arithmetic
//! that turns a facing, a clip and a sim tick into one cell and that cell into a
//! screen rectangle with its feet on the tile. It reads no file and holds no GPU
//! type, so the window, the scene and the tests share one answer.
//!
//! The manifest is the authority: cell size, padding, pivot, facings and the
//! clips come from it, never from a constant here.

use std::fmt;

use engine_render::kam2_sprites::Facing;
use serde_json::Value;

/// Style-lock facing names, atlas column order.
pub const FACING_NAMES: [&str; 8] = ["S", "SW", "W", "NW", "N", "NE", "E", "SE"];
/// Sim ticks per second: the fixed 10 Hz shell tick.
pub const SIM_HZ: u32 = 10;

/// Which of a unit's clips a billboard plays.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnitClip {
    /// The walk cycle, one frame per tick at 10 fps.
    Walk,
    /// The held pose of a unit that stands still.
    Idle,
}

impl UnitClip {
    /// Manifest animation name of this clip.
    pub fn name(self) -> &'static str {
        match self {
            Self::Walk => "walk",
            Self::Idle => "walk_pose",
        }
    }
}

/// One animation of the manifest: a run of atlas rows, one row per frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SoldierAnim {
    /// Stable clip name, e.g. `walk`.
    pub name: String,
    /// Frame count, at least one.
    pub frames: u32,
    /// Playback rate in frames per second.
    pub fps: u32,
    /// `true` wraps at the last frame, `false` holds it.
    pub looped: bool,
    /// First atlas row of the clip; the frames follow down the column.
    pub row: u32,
}

/// The soldier manifest, parsed and checked against its atlas size.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SoldierManifest {
    /// Stable asset id, e.g. `unit.soldier`.
    pub id: String,
    /// Atlas file name beside the manifest.
    pub atlas: String,
    /// Optional team-mask atlas file name; `None` means no tint pass.
    pub mask_atlas: Option<String>,
    /// Atlas size in pixels.
    pub atlas_size: [u32; 2],
    /// One frame without the gutter, in pixels.
    pub tile: [u32; 2],
    /// Transparent gutter around one frame, in pixels.
    pub padding: u32,
    /// Ground anchor inside one frame, from its top-left.
    pub pivot: [u32; 2],
    /// Facing names in atlas column order.
    pub facings: Vec<String>,
    /// Clips in atlas row order.
    pub anims: Vec<SoldierAnim>,
}

/// Why a soldier manifest refused to load.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ManifestError {
    /// The text is not JSON, or not a JSON object.
    Json(String),
    /// A required field is missing or carries another type.
    Field(&'static str),
    /// The numbers disagree with the atlas size or the facing list.
    Grid(&'static str),
}

impl fmt::Display for ManifestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Json(error) => write!(f, "manifest is not JSON: {error}"),
            Self::Field(name) => write!(f, "manifest field missing or wrong type: {name}"),
            Self::Grid(reason) => write!(f, "atlas grid refused: {reason}"),
        }
    }
}

impl std::error::Error for ManifestError {}

impl SoldierManifest {
    /// Parse one manifest text and check it against its own atlas size.
    ///
    /// # Errors
    ///
    /// Returns [`ManifestError`] for a missing or mistyped field, a cell size of
    /// zero, an atlas that is not a whole number of cells, fewer columns than
    /// facings, a clip that overruns the rows, or a pivot outside its frame.
    pub fn parse(text: &str) -> Result<Self, ManifestError> {
        let root: Value =
            serde_json::from_str(text).map_err(|error| ManifestError::Json(error.to_string()))?;
        let facings_value = field(&root, "facings")?;
        let facings = facings_value
            .as_array()
            .ok_or(ManifestError::Field("facings"))?
            .iter()
            .map(|name| {
                name.as_str()
                    .map(str::to_owned)
                    .ok_or(ManifestError::Field("facings"))
            })
            .collect::<Result<Vec<String>, ManifestError>>()?;
        let anims_value = field(&root, "anims")?;
        let mut anims = Vec::new();
        for entry in anims_value
            .as_array()
            .ok_or(ManifestError::Field("anims"))?
        {
            anims.push(SoldierAnim {
                name: text_at(entry, "name", "anims.name")?.to_owned(),
                frames: u32_at(entry, "frames", "anims.frames")?,
                fps: u32_at(entry, "fps", "anims.fps")?,
                looped: entry
                    .get("loop")
                    .and_then(Value::as_bool)
                    .ok_or(ManifestError::Field("anims.loop"))?,
                row: u32_at(entry, "row", "anims.row")?,
            });
        }
        let manifest = Self {
            id: text_at(&root, "id", "id")?.to_owned(),
            atlas: text_at(&root, "atlas", "atlas")?.to_owned(),
            mask_atlas: {
                let mask = field(&root, "mask_atlas")?;
                if mask.is_null() {
                    None
                } else {
                    Some(
                        mask.as_str()
                            .ok_or(ManifestError::Field("mask_atlas"))?
                            .to_owned(),
                    )
                }
            },
            atlas_size: pair_at(&root, "atlas_size", "atlas_size")?,
            tile: [
                u32_at(&root, "tile_w", "tile_w")?,
                u32_at(&root, "tile_h", "tile_h")?,
            ],
            padding: u32_at(&root, "padding", "padding")?,
            pivot: pair_at(&root, "pivot", "pivot")?,
            facings,
            anims,
        };
        manifest.check()?;
        Ok(manifest)
    }

    /// Cell pitch in atlas pixels: the frame plus its gutter on both sides.
    pub fn pitch(&self) -> [u32; 2] {
        [
            self.tile[0].saturating_add(self.padding.saturating_mul(2)),
            self.tile[1].saturating_add(self.padding.saturating_mul(2)),
        ]
    }

    /// Atlas columns, one per facing; zero when the pitch is zero.
    pub fn columns(&self) -> u32 {
        let [pitch_w, _] = self.pitch();
        if pitch_w == 0 {
            return 0;
        }
        self.atlas_size[0] / pitch_w
    }

    /// Atlas rows, one per animation frame; zero when the pitch is zero.
    pub fn rows(&self) -> u32 {
        let [_, pitch_h] = self.pitch();
        if pitch_h == 0 {
            return 0;
        }
        self.atlas_size[1] / pitch_h
    }

    /// The clip with this manifest name.
    pub fn anim(&self, name: &str) -> Option<&SoldierAnim> {
        self.anims.iter().find(|anim| anim.name == name)
    }

    /// The clip a billboard plays, or the first clip when the manifest does
    /// not name it. `None` only when the manifest carries no clips at all.
    pub fn clip(&self, clip: UnitClip) -> Option<&SoldierAnim> {
        self.anim(clip.name()).or_else(|| self.anims.first())
    }

    /// Atlas column of a facing: its index in the manifest's own facing order.
    /// `None` when the manifest does not list the style-lock name there.
    pub fn facing_column(&self, facing: Facing) -> Option<u32> {
        let index = usize::from(facing.index());
        let expected = FACING_NAMES.get(index)?;
        (self.facings.get(index)? == expected).then_some(u32::from(facing.index()))
    }

    /// Atlas pixel rectangle of one cell: `[x, y, w, h]` of the frame without
    /// its gutter. `None` outside the grid.
    pub fn cell_rect(&self, row: u32, column: u32) -> Option<[u32; 4]> {
        if row >= self.rows() || column >= self.columns() {
            return None;
        }
        let [pitch_w, pitch_h] = self.pitch();
        Some([
            column * pitch_w + self.padding,
            row * pitch_h + self.padding,
            self.tile[0],
            self.tile[1],
        ])
    }

    /// Normalized UV origin and size of one cell. `None` outside the grid.
    pub fn cell_uv(&self, row: u32, column: u32) -> Option<[[f32; 2]; 2]> {
        let [x, y, w, h] = self.cell_rect(row, column)?;
        let atlas_w = self.atlas_size[0] as f32;
        let atlas_h = self.atlas_size[1] as f32;
        Some([
            [x as f32 / atlas_w, y as f32 / atlas_h],
            [w as f32 / atlas_w, h as f32 / atlas_h],
        ])
    }

    /// Sprite size in framebuffer pixels at 100% zoom: the frame, not the cell.
    pub fn size(&self) -> [f32; 2] {
        [self.tile[0] as f32, self.tile[1] as f32]
    }

    /// Top-left framebuffer pixel of a sprite whose pivot sits on `ground`.
    pub fn top_left(&self, ground: [f32; 2]) -> [f32; 2] {
        [
            ground[0] - self.pivot[0] as f32,
            ground[1] - self.pivot[1] as f32,
        ]
    }

    /// The grid must hold every clip and the pivot must sit inside a frame.
    fn check(&self) -> Result<(), ManifestError> {
        let [pitch_w, pitch_h] = self.pitch();
        if pitch_w == 0 || pitch_h == 0 {
            return Err(ManifestError::Grid("a cell side is zero"));
        }
        if !self.atlas_size[0].is_multiple_of(pitch_w)
            || !self.atlas_size[1].is_multiple_of(pitch_h)
        {
            return Err(ManifestError::Grid(
                "the atlas is not a whole number of cells",
            ));
        }
        let columns = u32::try_from(self.facings.len()).unwrap_or(u32::MAX);
        if self.columns() != columns || columns == 0 {
            return Err(ManifestError::Grid("one atlas column per listed facing"));
        }
        for anim in &self.anims {
            if anim.frames == 0 || anim.fps == 0 {
                return Err(ManifestError::Grid("a clip has no frames or no rate"));
            }
            let end = anim.row.saturating_add(anim.frames);
            if end > self.rows() {
                return Err(ManifestError::Grid("a clip overruns the atlas rows"));
            }
        }
        if self.pivot[0] >= self.tile[0] || self.pivot[1] >= self.tile[1] {
            return Err(ManifestError::Grid("the pivot is outside its frame"));
        }
        Ok(())
    }
}

/// Frame of `anim` at sim `tick`: `tick * fps / 10`, wrapped by the clip.
///
/// The sim tick is the only clock, so the same tick always picks the same
/// frame. A clip that does not loop holds its last frame.
pub fn frame_index(tick: u32, anim: &SoldierAnim) -> u32 {
    let elapsed = u64::from(tick) * u64::from(anim.fps) / u64::from(SIM_HZ);
    let frames = u64::from(anim.frames.max(1));
    let index = if anim.looped {
        elapsed % frames
    } else {
        elapsed.min(frames - 1)
    };
    // A remainder below `frames`, which is a u32, always fits.
    u32::try_from(index).unwrap_or(0)
}

/// One field of the manifest object.
fn field<'a>(root: &'a Value, name: &'static str) -> Result<&'a Value, ManifestError> {
    root.get(name).ok_or(ManifestError::Field(name))
}

/// One string field, with its own error label.
fn text_at<'a>(
    root: &'a Value,
    name: &'static str,
    label: &'static str,
) -> Result<&'a str, ManifestError> {
    root.get(name)
        .and_then(Value::as_str)
        .ok_or(ManifestError::Field(label))
}

/// One unsigned 32-bit field, with its own error label.
fn u32_at(root: &Value, name: &'static str, label: &'static str) -> Result<u32, ManifestError> {
    root.get(name)
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
        .ok_or(ManifestError::Field(label))
}

/// One `[x, y]` pair of unsigned 32-bit fields; the house manifest shares it.
pub(crate) fn pair_at(
    root: &Value,
    name: &'static str,
    label: &'static str,
) -> Result<[u32; 2], ManifestError> {
    let values = root
        .get(name)
        .and_then(Value::as_array)
        .ok_or(ManifestError::Field(label))?;
    if values.len() != 2 {
        return Err(ManifestError::Field(label));
    }
    let x = values[0]
        .as_u64()
        .and_then(|value| u32::try_from(value).ok())
        .ok_or(ManifestError::Field(label))?;
    let y = values[1]
        .as_u64()
        .and_then(|value| u32::try_from(value).ok())
        .ok_or(ManifestError::Field(label))?;
    Ok([x, y])
}

/// The committed manifest, for tests that must catch content drift.
#[cfg(test)]
pub const MANIFEST_JSON: &str = include_str!("../../../content/kam2/soldier/manifest.json");

#[cfg(test)]
mod tests {
    use super::*;

    const EPS: f32 = 1e-6;

    fn manifest() -> SoldierManifest {
        SoldierManifest::parse(MANIFEST_JSON).expect("the committed manifest parses")
    }

    #[test]
    fn the_committed_manifest_parses_into_the_atlas_grid() {
        let manifest = manifest();
        assert_eq!(manifest.id, "unit.soldier");
        assert_eq!(manifest.atlas, "unit.soldier.atlas.png");
        assert_eq!(manifest.mask_atlas, None, "this look pass ships no mask");
        assert_eq!(manifest.atlas_size, [528, 594]);
        assert_eq!(manifest.tile, [64, 64]);
        assert_eq!(manifest.padding, 1);
        assert_eq!(manifest.pivot, [32, 56]);
        assert_eq!(manifest.facings, FACING_NAMES.to_vec());
        assert_eq!(manifest.pitch(), [66, 66]);
        assert_eq!(manifest.columns(), 8);
        assert_eq!(manifest.rows(), 9);
        let walk = manifest.anim("walk").expect("walk clip");
        assert_eq!(walk.frames, 8);
        assert_eq!(walk.fps, 10);
        assert!(walk.looped);
        assert_eq!(walk.row, 1);
        let pose = manifest.anim("walk_pose").expect("held pose");
        assert_eq!((pose.frames, pose.row), (1, 0));
        assert_eq!(
            manifest.clip(UnitClip::Walk).map(|anim| anim.name.as_str()),
            Some("walk")
        );
        assert_eq!(
            manifest.clip(UnitClip::Idle).map(|anim| anim.name.as_str()),
            Some("walk_pose")
        );

        let [origin, size] = manifest.cell_uv(1, 6).expect("walk row, east column");
        assert!((origin[0] - 397.0 / 528.0).abs() < EPS, "{origin:?}");
        assert!((origin[1] - 67.0 / 594.0).abs() < EPS, "{origin:?}");
        assert!((size[0] - 64.0 / 528.0).abs() < EPS);
        assert!((size[1] - 64.0 / 594.0).abs() < EPS);
    }

    #[test]
    fn a_manifest_that_breaks_its_atlas_size_is_refused() {
        fn mutated(edit: impl FnOnce(&mut serde_json::Value)) -> String {
            let mut value: serde_json::Value =
                serde_json::from_str(MANIFEST_JSON).expect("the committed manifest is JSON");
            edit(&mut value);
            value.to_string()
        }

        let missing = mutated(|value| {
            if let Some(object) = value.as_object_mut() {
                object.remove("atlas_size");
            }
        });
        assert_eq!(
            SoldierManifest::parse(&missing),
            Err(ManifestError::Field("atlas_size"))
        );
        let wrong_grid = mutated(|value| value["padding"] = serde_json::json!(2));
        assert_eq!(
            SoldierManifest::parse(&wrong_grid),
            Err(ManifestError::Grid(
                "the atlas is not a whole number of cells"
            )),
            "528 px is not a whole number of 68 px cells"
        );
        let outside = mutated(|value| value["pivot"] = serde_json::json!([32, 64]));
        assert_eq!(
            SoldierManifest::parse(&outside),
            Err(ManifestError::Grid("the pivot is outside its frame"))
        );
        assert!(matches!(
            SoldierManifest::parse("not json"),
            Err(ManifestError::Json(_))
        ));
    }

    #[test]
    fn a_facing_picks_its_atlas_column() {
        let manifest = manifest();
        for facing in Facing::ALL {
            assert_eq!(
                manifest.facing_column(facing),
                Some(u32::from(facing.index())),
                "{facing:?}"
            );
        }
        assert_eq!(manifest.facing_column(Facing::S), Some(0));
        assert_eq!(manifest.facing_column(Facing::E), Some(6));
        assert_eq!(manifest.facing_column(Facing::SE), Some(7));
        let [origin, _] = manifest
            .cell_uv(1, manifest.facing_column(Facing::E).unwrap_or(9))
            .expect("east column");
        assert!((origin[0] - 397.0 / 528.0).abs() < EPS);

        let mut rotated = manifest;
        rotated.facings.swap(0, 2);
        assert_eq!(
            rotated.facing_column(Facing::S),
            None,
            "a reordered facing list cannot stay in atlas order"
        );
        let mut short = rotated;
        short.facings.truncate(4);
        assert!(matches!(short.check(), Err(ManifestError::Grid(_))));
    }

    #[test]
    fn the_walk_frame_follows_the_tick_and_wraps() {
        let manifest = manifest();
        let walk = manifest.anim("walk").expect("walk clip");
        let expected = [0, 1, 2, 3, 4, 5, 6, 7, 0, 1, 2, 3];
        for (tick, frame) in expected.into_iter().enumerate() {
            assert_eq!(frame_index(tick as u32, walk), frame, "tick {tick}");
        }
        assert_eq!(frame_index(15, walk), 7);
        assert_eq!(frame_index(16, walk), 0);
        assert_eq!(frame_index(80, walk), 0, "eight loops land on frame zero");

        let pose = manifest.anim("walk_pose").expect("held pose");
        assert_eq!(frame_index(7, pose), 0, "a one-frame clip never moves");

        let fast = SoldierAnim {
            name: "attack".into(),
            frames: 6,
            fps: 12,
            looped: true,
            row: 0,
        };
        assert_eq!(frame_index(0, &fast), 0);
        assert_eq!(
            frame_index(5, &fast),
            0,
            "5 ticks at 12 fps lands on frame 6"
        );
        assert_eq!(frame_index(6, &fast), 1);
        let once = SoldierAnim {
            name: "die".into(),
            frames: 8,
            fps: 10,
            looped: false,
            row: 0,
        };
        assert_eq!(frame_index(3, &once), 3);
        assert_eq!(
            frame_index(40, &once),
            7,
            "a one-shot clip holds its last frame"
        );
    }

    #[test]
    fn the_pivot_puts_the_feet_on_the_ground_pixel() {
        let manifest = manifest();
        let ground = [640.0, 360.0];
        let top_left = manifest.top_left(ground);
        assert_eq!(top_left, [608.0, 304.0]);
        assert_eq!(manifest.size(), [64.0, 64.0]);
        let pivot = [
            top_left[0] + manifest.pivot[0] as f32,
            top_left[1] + manifest.pivot[1] as f32,
        ];
        assert_eq!(pivot, ground, "the ground pixel sits on the pivot");
        let [x, y, w, h] = manifest.cell_rect(2, 3).expect("inside the grid");
        assert_eq!([x, y, w, h], [3 * 66 + 1, 2 * 66 + 1, 64, 64]);
        assert_eq!(manifest.cell_rect(9, 0), None);
        assert_eq!(manifest.cell_rect(0, 8), None);
    }
}
