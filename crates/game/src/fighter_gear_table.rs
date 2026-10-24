//! fighter_gear_table: which CC-BY Sketchfab model fills each gear slot, and how it sits on
//! the grip.
//!
//! One row per slot (sword hand, off-hand dagger, spear, shield arm, helmet, torso): the
//! glb under [`GEAR_SOURCE_DIR`], the bounds measured off that file in its own units, which
//! model axis carries the slot's length and its width, the target metres, where the grip
//! sits along the length, and the rig-space directions the length and width map to. The
//! files and their licences are `content/props/sketchfab/PROVENANCE.md` (CC-BY, credited in
//! `CREDITS.md`); this module owns only the table and the fit.
//!
//! The grip transform is pure: origin at the grip, the slot's length on the slot's axis,
//! scaled so a gladius comes out [`super::SWORD_LEN_M`] long and a scutum about 1.0 x 0.65 m.
//! Nothing here touches the fixed tick; a body that cannot read a file keeps the procedural
//! gear in [`super::arm_hero`] and never panics.
//!
//! Units: metres out, model units in (the table records the source bounds to convert).
//! Determinism: the same file gives the same part twice.

use std::path::{Path, PathBuf};

use engine_assets::budget::{COMFORT, POTATO};
use engine_assets::cooked::TextureSlot;
use engine_assets::mesh_import::{import_gltf_path, GltfImport};

use super::super::fighter_gear_tex as gear_tex;
use super::GearPart;

/// Directory, from the repo root, that holds the CC-BY Sketchfab gear models.
pub const GEAR_SOURCE_DIR: &str = "content/props/sketchfab";
/// A file the gear directory must hold, used to find the content root from the exe.
const ANCHOR_FILE: &str = "gladius_samize.glb";

/// A gear slot on the fighter.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum GearSlot {
    /// Right hand: the gladius.
    SwordHand,
    /// Off hand, belt height: the pugio.
    OffHandDagger,
    /// Either hand: the pilum.
    Spear,
    /// Left arm: the scutum.
    ShieldArm,
    /// Head: the galea.
    Helmet,
    /// Chest: the lorica segmentata.
    Torso,
}

/// Every slot, in the order the table holds them.
pub const SLOTS: [GearSlot; 6] = [
    GearSlot::SwordHand,
    GearSlot::OffHandDagger,
    GearSlot::Spear,
    GearSlot::ShieldArm,
    GearSlot::Helmet,
    GearSlot::Torso,
];

/// Slot names, in [`SLOTS`] order, for the audit line.
const SLOT_NAMES: [&str; 6] = [
    "sword_hand",
    "off_hand_dagger",
    "spear",
    "shield_arm",
    "helmet",
    "torso",
];

/// Part names the look draws, in [`SLOTS`] order. It reads the shading family off the name
/// (`fighter_look::part_material_kind`), so the armed slots keep the names the graft used;
/// the other four name themselves until the kit draws them.
const PART_NAMES: [&str; 6] = [
    "gladius_blade",
    "pugio",
    "pilum",
    "scutum_face",
    "galea",
    "lorica",
];

impl GearSlot {
    /// Position in [`SLOTS`].
    fn at(self) -> usize {
        match self {
            Self::SwordHand => 0,
            Self::OffHandDagger => 1,
            Self::Spear => 2,
            Self::ShieldArm => 3,
            Self::Helmet => 4,
            Self::Torso => 5,
        }
    }

    /// The joint the slot's part rides (the importer lowercases joint names).
    #[must_use]
    pub fn joint_name(self) -> &'static str {
        match self {
            Self::SwordHand | Self::Spear => "wep_pos_r",
            Self::OffHandDagger | Self::ShieldArm => "wep_pos_l",
            Self::Helmet => "head",
            Self::Torso => "torso",
        }
    }

    /// Slot name for the audit line.
    #[must_use]
    pub fn name(self) -> &'static str {
        SLOT_NAMES[self.at()]
    }

    /// Part name the look draws.
    #[must_use]
    pub fn part_name(self) -> &'static str {
        PART_NAMES[self.at()]
    }

    /// `(metallic, roughness)` of the slot's material.
    #[must_use]
    pub fn factors(self) -> (f32, f32) {
        match self {
            Self::SwordHand | Self::OffHandDagger => (0.85, 0.38),
            Self::Spear => (0.70, 0.45),
            Self::ShieldArm => (0.05, 0.55),
            Self::Helmet => (0.75, 0.45),
            Self::Torso => (0.85, 0.42),
        }
    }

    /// Procedural texels, the fallback when the model carries no base-colour image.
    #[must_use]
    pub fn texels(self) -> gear_tex::Texels {
        match self {
            Self::SwordHand | Self::OffHandDagger | Self::Spear => gear_tex::steel(),
            Self::ShieldArm => gear_tex::shield_face(),
            Self::Helmet => gear_tex::bronze(),
            Self::Torso => gear_tex::wood(),
        }
    }

    /// The caps row the slot's triangles are held to: weapons are held to the potato
    /// budget a fighter draws under, worn armour to comfort (a galea is 2,000 source
    /// triangles, and it is one piece of a fighter, not the whole fighter).
    #[must_use]
    pub fn tris_cap(self) -> u32 {
        match self {
            Self::SwordHand | Self::OffHandDagger | Self::Spear | Self::ShieldArm => {
                POTATO.max_tris
            }
            Self::Helmet | Self::Torso => COMFORT.max_tris,
        }
    }
}

/// A source model under [`GEAR_SOURCE_DIR`]. The gladius direraven, gladius aren boada and
/// scutum Dennis van Malderen models are the same gear and stay in the folder, measured and
/// credited, for a later row to swap in.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum GearModel {
    /// Gladius, 1,500 triangles (Sketchfab, samize).
    GladiusSamize,
    /// Pugio, 1,500 triangles (Sketchfab, davicolt).
    PugioDavicolt,
    /// Pilum, 662 triangles (Sketchfab, paulohare).
    PilumPaulohare,
    /// Scutum, 990 triangles (Sketchfab, paulohare).
    ScutumPaulohare,
    /// Lorica segmentata, 1,499 triangles (Sketchfab).
    LoricaSegmentata,
    /// Galea, 2,000 triangles (Sketchfab, Killband).
    GaleaKillband,
}

impl GearModel {
    /// File name under [`GEAR_SOURCE_DIR`].
    #[must_use]
    pub fn file_name(self) -> &'static str {
        match self {
            Self::GladiusSamize => "gladius_samize.glb",
            Self::PugioDavicolt => "pugio_davicolt.glb",
            Self::PilumPaulohare => "pilum_paulohare.glb",
            Self::ScutumPaulohare => "scutum_paulohare.glb",
            Self::LoricaSegmentata => "lorica_segmentata.glb",
            Self::GaleaKillband => "galea_killband.glb",
        }
    }
}

/// One gear slot's model and fit.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct GearRow {
    /// Slot this row fills.
    pub slot: GearSlot,
    /// Source model.
    pub model: GearModel,
    /// Measured source bounds, model units (x, y, z), from the file itself.
    pub src_min: [f32; 3],
    /// Measured source bounds, upper corner, model units.
    pub src_max: [f32; 3],
    /// Model-local axis carrying the slot's length (blade, shaft, shield height, torso).
    pub long_axis: usize,
    /// Model-local axis carrying the slot's width.
    pub across_axis: usize,
    /// Target length in metres along the slot's axis.
    pub length_m: f32,
    /// Target width in metres, or `None` to keep the model's own proportions.
    pub across_m: Option<f32>,
    /// Where the grip sits along the long axis: 0 at `src_min`, 1 at `src_max`.
    pub grip_at: f32,
    /// Rig-space direction the long axis maps to, unit (the table's canonical frame).
    pub slot_axis: [f32; 3],
    /// Rig-space direction the across axis maps to, unit and perpendicular to `slot_axis`.
    pub slot_across: [f32; 3],
    /// Measured source triangle count.
    pub tris: usize,
}

/// Up the rig: the blade, the shaft, the shield's height.
const UP: [f32; 3] = [0.0, 1.0, 0.0];
/// The rig's right: the sword's flat, the shield's width.
const RIGHT: [f32; 3] = [1.0, 0.0, 0.0];

/// The gear table: one row per slot. The bounds are the measured ones (the tests re-measure
/// every file and refuse a drift over 1%); `length_m` and `across_m` are the sizes the fight
/// poses need.
pub const GEAR_TABLE: [GearRow; 6] = [
    // A gladius: 0.70 m grip to tip (SWORD_LEN_M), hand a tenth of the way up from the
    // pommel. The source lies along +Y, pommel at -Y.
    GearRow {
        slot: GearSlot::SwordHand,
        model: GearModel::GladiusSamize,
        src_min: [-0.380_890_82, -1.000_237_5, -0.089_782_69],
        src_max: [0.384_691, 1.000_237_7, 0.089_823_29],
        long_axis: 1,
        across_axis: 0,
        length_m: super::SWORD_LEN_M,
        across_m: None,
        grip_at: 0.12,
        slot_axis: UP,
        slot_across: RIGHT,
        tris: 1_500,
    },
    // A pugio: 0.35 m total, hand at the guard. The source is a scabbard-and-dagger in
    // one mesh, so the guard sits a little further up than a bare blade's.
    GearRow {
        slot: GearSlot::OffHandDagger,
        model: GearModel::PugioDavicolt,
        src_min: [-19.247_055, -22.407_783, -0.737_535_66],
        src_max: [8.551_571, 31.050_49, 0.742_705_9],
        long_axis: 1,
        across_axis: 0,
        length_m: 0.35,
        across_m: None,
        grip_at: 0.14,
        slot_axis: UP,
        slot_across: RIGHT,
        tris: 1_500,
    },
    // A pilum: 2.0 m shaft and point, held 42% back from the point end.
    GearRow {
        slot: GearSlot::Spear,
        model: GearModel::PilumPaulohare,
        src_min: [-0.036_193, -1.0, -0.036_193],
        src_max: [0.036_193, 1.0, 0.036_193],
        long_axis: 1,
        across_axis: 0,
        length_m: 2.0,
        across_m: None,
        grip_at: 0.42,
        slot_axis: UP,
        slot_across: RIGHT,
        tris: 662,
    },
    // A scutum: 1.0 m tall and 0.65 m wide (the source is 2:1, so the width is set
    // explicitly), held at its centre. Height runs along the source's Z, width along X.
    GearRow {
        slot: GearSlot::ShieldArm,
        model: GearModel::ScutumPaulohare,
        src_min: [-0.502_908, -0.180_533, -1.0],
        src_max: [0.502_908, 0.180_533, 1.0],
        long_axis: 2,
        across_axis: 0,
        length_m: 1.0,
        across_m: Some(0.65),
        grip_at: 0.5,
        slot_axis: UP,
        slot_across: RIGHT,
        tris: 990,
    },
    // A galea: 0.30 m from the brow to the neck guard, sat on the crown.
    GearRow {
        slot: GearSlot::Helmet,
        model: GearModel::GaleaKillband,
        src_min: [-0.995_1, -0.902_123, -0.997_321],
        src_max: [0.995_1, 0.902_311, 1.000_492],
        long_axis: 1,
        across_axis: 0,
        length_m: 0.30,
        across_m: None,
        grip_at: 0.5,
        slot_axis: UP,
        slot_across: RIGHT,
        tris: 2_000,
    },
    // A lorica segmentata: 0.62 m from the collar to the girdle, centred on the chest.
    GearRow {
        slot: GearSlot::Torso,
        model: GearModel::LoricaSegmentata,
        src_min: [-1.118_141_3, -8.779_62, -1.741_431],
        src_max: [4.664_474_5, 3.210_859, 1.741_431],
        long_axis: 1,
        across_axis: 2,
        length_m: 0.62,
        across_m: None,
        grip_at: 0.5,
        slot_axis: UP,
        slot_across: RIGHT,
        tris: 1_499,
    },
];

/// The row that fills `slot`.
#[must_use]
pub fn row(slot: GearSlot) -> &'static GearRow {
    GEAR_TABLE
        .iter()
        .find(|row| row.slot == slot)
        .unwrap_or(&GEAR_TABLE[0])
}

/// The slots the hero is armed with from this table: the two hands and the worn armour the
/// kit used to draw procedurally (`fighter_kit::dress_hero`'s `galea` and `lorica`).
pub const ARMED_SLOTS: [GearSlot; 4] = [
    GearSlot::SwordHand,
    GearSlot::ShieldArm,
    GearSlot::Helmet,
    GearSlot::Torso,
];

/// A grip transform: model units to metres, origin at the grip.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct GripFit {
    /// Metres per model unit, per model axis.
    pub scale: [f32; 3],
    /// Rig-space image of each model axis, unit.
    pub axes: [[f32; 3]; 3],
    /// The model point that lands on the origin (the grip).
    pub grip: [f32; 3],
}

impl GripFit {
    /// The fit of `row` onto the given rig-space directions: `along` is the slot's length
    /// axis, `across` its width axis. Both are normalised here, so a caller may pass
    /// measured directions straight from a bind pose.
    #[must_use]
    pub fn of(row: &GearRow, along: [f32; 3], across: [f32; 3]) -> Self {
        let along = unit(along, UP);
        let across = unit(perp(across, along), RIGHT);
        let normal = engine_core::vec3::cross3(along, across);
        let mut axes = [[0.0; 3]; 3];
        axes[row.long_axis] = along;
        axes[row.across_axis] = across;
        axes[leftover(row.long_axis, row.across_axis)] = normal;
        let span = |axis: usize| (row.src_max[axis] - row.src_min[axis]).max(1.0e-6);
        let long_span = span(row.long_axis);
        let across_span = span(row.across_axis);
        let s_long = row.length_m / long_span;
        let s_across = row.across_m.map_or(s_long, |w| w / across_span);
        let s_third = (s_long * s_across).sqrt();
        let mut scale = [s_long; 3];
        scale[row.long_axis] = s_long;
        scale[row.across_axis] = s_across;
        scale[leftover(row.long_axis, row.across_axis)] = s_third;
        let mut grip = [0.0; 3];
        for axis in 0..3 {
            grip[axis] = if axis == row.long_axis {
                row.src_min[axis] + (row.src_max[axis] - row.src_min[axis]) * row.grip_at
            } else {
                0.5 * (row.src_min[axis] + row.src_max[axis])
            };
        }
        Self { scale, axes, grip }
    }

    /// The table's canonical fit: the blade up, the flat facing across.
    #[must_use]
    pub fn canonical(row: &GearRow) -> Self {
        Self::of(row, row.slot_axis, row.slot_across)
    }

    /// Model point in metres, origin at the grip.
    #[must_use]
    pub fn point(&self, p: [f32; 3]) -> [f32; 3] {
        let mut out = [0.0; 3];
        for axis in 0..3 {
            let d = (p[axis] - self.grip[axis]) * self.scale[axis];
            for (component, value) in out.iter_mut().enumerate() {
                *value += d * self.axes[axis][component];
            }
        }
        out
    }

    /// Model normal in rig space: the inverse transpose (divide by the axis scale) then
    /// normalised, so a width squash does not tilt a face.
    #[must_use]
    pub fn normal(&self, n: [f32; 3]) -> [f32; 3] {
        let mut out = [0.0; 3];
        for axis in 0..3 {
            let d = n[axis] / self.scale[axis].max(1.0e-6);
            for (component, value) in out.iter_mut().enumerate() {
                *value += d * self.axes[axis][component];
            }
        }
        unit(out, UP)
    }
}

fn leftover(a: usize, b: usize) -> usize {
    (0..3).find(|axis| *axis != a && *axis != b).unwrap_or(2)
}

fn unit(v: [f32; 3], fallback: [f32; 3]) -> [f32; 3] {
    let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if len.is_finite() && len > 1.0e-6 {
        [v[0] / len, v[1] / len, v[2] / len]
    } else {
        fallback
    }
}

/// `v` without its component along the unit vector `along`.
fn perp(v: [f32; 3], along: [f32; 3]) -> [f32; 3] {
    let dot = v[0] * along[0] + v[1] * along[1] + v[2] * along[2];
    [
        v[0] - along[0] * dot,
        v[1] - along[1] * dot,
        v[2] - along[2] * dot,
    ]
}

/// Measured bounds of every triangle position in `import`.
#[must_use]
pub fn bounds(import: &GltfImport) -> Option<([f32; 3], [f32; 3])> {
    let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
    let mut any = false;
    for mesh in &import.meshes {
        for pos in &mesh.positions {
            any = true;
            for (axis, value) in pos.iter().enumerate() {
                if value.is_finite() {
                    lo[axis] = lo[axis].min(*value);
                    hi[axis] = hi[axis].max(*value);
                }
            }
        }
    }
    any.then_some((lo, hi))
}

/// Triangle count of an import.
#[must_use]
pub fn tri_count(import: &GltfImport) -> usize {
    import
        .meshes
        .iter()
        .map(|mesh| mesh.indices.len() / 3)
        .sum()
}

/// The directory holding the gear glbs: walked up from the running exe to the repo root,
/// then the manifest directory. Never panics; a directory that is not there gives a path
/// that does not open, and [`load`] returns `None`.
#[must_use]
pub fn source_dir() -> PathBuf {
    if let Ok(exe) = std::env::current_exe() {
        let mut anchor = exe.parent().map(Path::to_path_buf);
        while let Some(dir) = anchor {
            let candidate = dir.join(GEAR_SOURCE_DIR);
            if candidate.join(ANCHOR_FILE).is_file() {
                return candidate;
            }
            anchor = dir.parent().map(Path::to_path_buf);
        }
    }
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(GEAR_SOURCE_DIR)
}

/// Read and import one model. `None` on a missing file or a refused import: the caller
/// keeps the procedural gear.
#[must_use]
pub fn load(dir: &Path, model: GearModel) -> Option<GltfImport> {
    let bytes = std::fs::read(dir.join(model.file_name())).ok()?;
    import_gltf_path(model.file_name(), &bytes).ok()
}

/// The base-colour image of an import, when the file carries one (encoded PNG or JPEG,
/// exactly as stored).
#[must_use]
pub fn base_color_image(import: &GltfImport) -> Option<Vec<u8>> {
    for material in &import.materials {
        let Some(asset) = material
            .textures
            .iter()
            .find(|texture| texture.slot == TextureSlot::BaseColor)
            .map(|texture| texture.asset)
        else {
            continue;
        };
        if let Some(bytes) = import.images.get(&asset) {
            return Some(bytes.clone());
        }
    }
    None
}

/// One gear part from an imported model and a grip fit, rigid to `joint`.
#[must_use]
pub fn part(row: &GearRow, import: &GltfImport, fit: &GripFit, joint: u16) -> Option<GearPart> {
    let (metallic, roughness) = row.slot.factors();
    let mut part = GearPart {
        name: row.slot.part_name(),
        positions: Vec::new(),
        normals: Vec::new(),
        uv0: Vec::new(),
        indices: Vec::new(),
        joints: Vec::new(),
        weights: Vec::new(),
        texels: row.slot.texels(),
        metallic,
        roughness,
        image: base_color_image(import),
    };
    for mesh in &import.meshes {
        let base = u32::try_from(part.positions.len()).ok()?;
        for (index, pos) in mesh.positions.iter().enumerate() {
            if !pos.iter().all(|value| value.is_finite()) {
                return None;
            }
            part.positions.push(fit.point(*pos));
            let normal = mesh.normals.get(index).copied().unwrap_or(UP);
            part.normals.push(fit.normal(normal));
            part.uv0
                .push(mesh.uv0.get(index).copied().unwrap_or([0.0, 0.0]));
            part.joints.push([joint; 4]);
            part.weights.push([1.0, 0.0, 0.0, 0.0]);
        }
        if mesh.indices.len() % 3 != 0 {
            return None;
        }
        for index in &mesh.indices {
            part.indices.push(base.checked_add(*index)?);
        }
    }
    if part.positions.is_empty() || part.indices.is_empty() {
        return None;
    }
    Some(part)
}

/// How many table parts [`super::arm_hero_budget`] adds with the gear under `dir` and the
/// `caps` budget: every armed slot whose file loads and whose triangles fit the row. A
/// weapon slot that will not load falls back to the four procedural graft parts, so the
/// hands count four then.
#[must_use]
pub fn armed_parts(dir: &Path, caps: engine_assets::budget::Caps) -> usize {
    let hand = |slot: GearSlot| {
        load(dir, row(slot).model).is_some_and(|import| fits(row(slot), &import, caps))
    };
    if !hand(GearSlot::SwordHand) || !hand(GearSlot::ShieldArm) {
        return 4;
    }
    let worn = [GearSlot::Helmet, GearSlot::Torso]
        .iter()
        .filter(|slot| hand(**slot))
        .count();
    2 + worn
}

/// Whether the imported model of `row` stays inside `caps`' triangle budget.
#[must_use]
pub fn fits(row: &GearRow, import: &GltfImport, caps: engine_assets::budget::Caps) -> bool {
    let cap = usize::try_from(caps.max_tris.min(row.slot.tris_cap())).unwrap_or(usize::MAX);
    tri_count(import) <= cap
}

#[cfg(test)]
#[rustfmt::skip]
#[path = "fighter_gear_table_tests.rs"] mod tests;
