//! fighter_gear: the hero's grafted gladius and scutum, cut out of the body part and given
//! materials of their own.
//!
//! The cooked hero carries its sword and shield inside the body mesh, on the body's atlas
//! quadrant, where their UVs point at an empty corner: they draw as flat black slabs. This
//! module finds the weapon triangles by skin weight (every vertex of the triangle rides the
//! `wep_pos_r` or `wep_pos_l` joint), takes them out of the body part, shrinks the sword to a
//! gladius, and returns four small parts: iron blade, bronze fittings, wood and the painted
//! scutum face. Each part has its own texture and metal or rough factors, so iron, bronze and
//! wood read as what they are.
//!
//! Owner: this module; consumer: `FighterLook::load_rig` (cold path, once per rig), which
//! finds the cut pieces among the body's parts like any other. The realistic gear comes from
//! the CC-BY table in [`table`] (a gladius and a scutum under `content/props/sketchfab`);
//! the procedural graft pieces are the fallback when a file does not load. Units:
//! meters, bind pose. Determinism: pure, the same body gives the same parts. A body with no
//! weapon joint or no weapon triangle (the three opponents) is left untouched and gives none.

use std::collections::HashMap;

use engine_assets::body_import::{BodyPart, SkinnedBody};
use engine_assets::cooked::{CookedMesh, MATERIAL_KIND_STONE};
use engine_assets::skin_import::Skin;

use super::fighter_gear_tex::{self as tex, Texels};
use super::fighter_kit::joint_origin;

/// The gear table: which CC-BY glb fills each slot and how it sits on the grip.
#[rustfmt::skip]
#[path = "fighter_gear_table.rs"] pub(crate) mod table;

/// Joint the grafted sword rides (the importer lowercases joint names).
const SWORD_JOINT: &str = "wep_pos_r";
/// Joint the grafted shield rides.
const SHIELD_JOINT: &str = "wep_pos_l";
/// A vertex belongs to a hand when the hand joint carries more than this weight.
const HAND_WEIGHT: f32 = 0.5;
/// Longest the sword may be, meters, grip to tip: a gladius (the graft shipped at 0.95 m).
pub const SWORD_LEN_M: f32 = 0.70;
/// Share of the sword's length (pommel end first) where the pommel ends.
const POMMEL_END: f32 = 0.10;
/// Where the grip ends and the guard starts.
const GRIP_END: f32 = 0.27;
/// Where the guard ends and the blade starts.
const GUARD_END: f32 = 0.38;
/// A shield triangle is on the outer face when its normal is this aligned with the outer side.
const FACE_DOT: f32 = 0.5;

/// What a part is made of.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    Steel,
    Bronze,
    Wood,
    Face,
}

/// Kinds in the order their parts are emitted.
const KINDS: [Kind; 4] = [Kind::Steel, Kind::Bronze, Kind::Wood, Kind::Face];

impl Kind {
    fn name(self) -> &'static str {
        match self {
            Self::Steel => "gladius_blade",
            Self::Bronze => "gladius_fittings",
            Self::Wood => "gear_wood",
            Self::Face => "scutum_face",
        }
    }

    /// `(metallic, roughness)`: iron and bronze are metals, wood and paint are not.
    fn factors(self) -> (f32, f32) {
        match self {
            Self::Steel => (0.85, 0.38),
            Self::Bronze => (0.75, 0.45),
            Self::Wood => (0.0, 0.75),
            Self::Face => (0.05, 0.55),
        }
    }

    fn texels(self) -> Texels {
        match self {
            Self::Steel => tex::steel(),
            Self::Bronze => tex::bronze(),
            Self::Wood => tex::wood(),
            Self::Face => tex::shield_face(),
        }
    }
}

/// One weapon part ready to skin: bind mesh, skin rows, texels and factors.
#[derive(Clone, PartialEq, Debug)]
pub struct GearPart {
    /// Part name for the view audit (`gladius_blade`, `scutum_face`, ...).
    pub name: &'static str,
    /// Bind positions, meters.
    pub positions: Vec<[f32; 3]>,
    /// Unit normals.
    pub normals: Vec<[f32; 3]>,
    /// UVs in `0..=1`.
    pub uv0: Vec<[f32; 2]>,
    /// Triangle indices.
    pub indices: Vec<u32>,
    /// Skin joints per vertex.
    pub joints: Vec<[u16; 4]>,
    /// Skin weights per vertex.
    pub weights: Vec<[f32; 4]>,
    /// Base-colour map, the fallback when [`Self::image`] is absent.
    pub texels: Texels,
    /// Encoded base-colour image from the source glb (PNG or JPEG), when it carries one.
    pub image: Option<Vec<u8>>,
    /// Metallic factor.
    pub metallic: f32,
    /// Roughness factor.
    pub roughness: f32,
}

/// One weapon vertex lifted out of a body part.
#[derive(Clone, Copy)]
struct Vert {
    pos: [f32; 3],
    normal: [f32; 3],
    joints: [u16; 4],
    weights: [f32; 4],
}

/// The triangles of one hand's weapon with their vertices.
#[derive(Default)]
struct Cloud {
    verts: Vec<Vert>,
    tris: Vec<[u32; 3]>,
}

/// A vertex index as a slice index (a mesh never holds more than `u32::MAX` vertices).
fn idx(i: u32) -> usize {
    usize::try_from(i).unwrap_or(usize::MAX)
}

fn hand_has(joints: [u16; 4], weights: [f32; 4], joint: u16) -> bool {
    joints
        .iter()
        .zip(weights)
        .any(|(j, w)| *j == joint && w > HAND_WEIGHT)
}

fn bounds(verts: &[Vert]) -> ([f32; 3], [f32; 3]) {
    let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
    for vert in verts {
        for axis in 0..3 {
            lo[axis] = lo[axis].min(vert.pos[axis]);
            hi[axis] = hi[axis].max(vert.pos[axis]);
        }
    }
    (lo, hi)
}

/// Axes sorted by extent, longest first.
fn axes_by_extent(lo: [f32; 3], hi: [f32; 3]) -> [usize; 3] {
    let mut axes = [0usize, 1, 2];
    axes.sort_by(|a, b| (hi[*b] - lo[*b]).total_cmp(&(hi[*a] - lo[*a])));
    axes
}

/// Lift every triangle whose three vertices ride `joint` out of the body parts.
fn lift(body: &mut SkinnedBody, joint: u16) -> Cloud {
    let mut cloud = Cloud::default();
    for part in &mut body.parts {
        let mut remap: HashMap<u32, u32> = HashMap::new();
        let mut kept = Vec::with_capacity(part.mesh.indices.len());
        for tri in part.mesh.indices.chunks_exact(3) {
            let ridden = tri.iter().all(|i| {
                let row = idx(*i);
                match (part.skin.joints.get(row), part.skin.weights.get(row)) {
                    (Some(j), Some(w)) => hand_has(*j, *w, joint),
                    _ => false,
                }
            });
            if !ridden {
                kept.extend_from_slice(tri);
                continue;
            }
            let mut out = [0u32; 3];
            for (slot, index) in tri.iter().enumerate() {
                out[slot] = *remap.entry(*index).or_insert_with(|| {
                    let row = idx(*index);
                    cloud.verts.push(Vert {
                        pos: part.mesh.positions[row],
                        normal: part.mesh.normals[row],
                        joints: part.skin.joints[row],
                        weights: part.skin.weights[row],
                    });
                    u32::try_from(cloud.verts.len() - 1).unwrap_or(u32::MAX)
                });
            }
            cloud.tris.push(out);
        }
        part.mesh.indices = kept;
    }
    cloud
}

/// Shrink `cloud` about `pivot` so its longest side is at most `max_len_m`.
fn shrink(cloud: &mut Cloud, pivot: [f32; 3], max_len_m: f32) {
    let (lo, hi) = bounds(&cloud.verts);
    let longest = axes_by_extent(lo, hi)[0];
    let len = hi[longest] - lo[longest];
    if !(len.is_finite() && len > max_len_m) {
        return;
    }
    let scale = max_len_m / len;
    for vert in &mut cloud.verts {
        for (axis, p) in vert.pos.iter_mut().enumerate() {
            *p = pivot[axis] + (*p - pivot[axis]) * scale;
        }
    }
}

/// Planar UV of a point in the weapon's frame: `u` across the short side, `v` along the long.
fn planar_uv(pos: [f32; 3], lo: [f32; 3], hi: [f32; 3], across: usize, along: usize) -> [f32; 2] {
    let unit = |axis: usize| {
        let span = (hi[axis] - lo[axis]).max(1.0e-6);
        ((pos[axis] - lo[axis]) / span).clamp(0.0, 1.0)
    };
    [unit(across), unit(along)]
}

/// The side of the thin axis the shield's outer face looks to, `+1.0` or `-1.0`: opposite
/// the half that holds the narrow handle bar.
fn outer_sign(verts: &[Vert], thin: usize, across: usize) -> f32 {
    let (lo, hi) = bounds(verts);
    let mid = 0.5 * (lo[thin] + hi[thin]);
    let span = |upper: bool| {
        let (mut a, mut b) = (f32::MAX, f32::MIN);
        for vert in verts.iter().filter(|v| (v.pos[thin] > mid) == upper) {
            a = a.min(vert.pos[across]);
            b = b.max(vert.pos[across]);
        }
        b - a
    };
    if span(true) < span(false) {
        -1.0
    } else {
        1.0
    }
}

/// Kind and bind position along the sword for each triangle, pommel end first.
fn sword_kind(t: f32) -> Kind {
    if t < POMMEL_END || (GRIP_END..GUARD_END).contains(&t) {
        Kind::Bronze
    } else if t < GRIP_END {
        Kind::Wood
    } else {
        Kind::Steel
    }
}

/// Accumulates the parts, one vertex pool per kind.
struct Builder {
    parts: [Option<GearPart>; 4],
    remap: [HashMap<(u32, usize), u32>; 4],
}

impl Builder {
    fn new() -> Self {
        Self {
            parts: [None, None, None, None],
            remap: [(); 4].map(|()| HashMap::new()),
        }
    }

    /// Add one triangle of `cloud` to the part of `kind`, UVs from `uv`; `tag` keeps the
    /// two weapons' vertices apart when they share a kind.
    fn add(
        &mut self,
        kind: Kind,
        cloud: &Cloud,
        tri: [u32; 3],
        tag: usize,
        uv: &dyn Fn([f32; 3]) -> [f32; 2],
    ) {
        let at = KINDS.iter().position(|k| *k == kind).unwrap_or(0);
        let part = self.parts[at].get_or_insert_with(|| {
            let (metallic, roughness) = kind.factors();
            GearPart {
                name: kind.name(),
                positions: Vec::new(),
                normals: Vec::new(),
                uv0: Vec::new(),
                indices: Vec::new(),
                joints: Vec::new(),
                weights: Vec::new(),
                texels: kind.texels(),
                image: None,
                metallic,
                roughness,
            }
        });
        for index in tri {
            let next = u32::try_from(part.positions.len()).unwrap_or(u32::MAX);
            let slot = *self.remap[at].entry((index, tag)).or_insert(next);
            if slot == next {
                let vert = cloud.verts[idx(index)];
                part.positions.push(vert.pos);
                part.normals.push(vert.normal);
                part.uv0.push(uv(vert.pos));
                part.joints.push(vert.joints);
                part.weights.push(vert.weights);
            }
            part.indices.push(slot);
        }
    }
}

fn add_sword(builder: &mut Builder, cloud: &Cloud) {
    let (lo, hi) = bounds(&cloud.verts);
    let [along, across, _] = axes_by_extent(lo, hi);
    let uv = move |pos: [f32; 3]| planar_uv(pos, lo, hi, across, along);
    for tri in &cloud.tris {
        let mid: f32 = tri
            .iter()
            .map(|i| cloud.verts[idx(*i)].pos[along])
            .sum::<f32>()
            / 3.0;
        let span = (hi[along] - lo[along]).max(1.0e-6);
        builder.add(sword_kind((mid - lo[along]) / span), cloud, *tri, 0, &uv);
    }
}

fn add_shield(builder: &mut Builder, cloud: &Cloud) {
    let (lo, hi) = bounds(&cloud.verts);
    let [along, across, thin] = axes_by_extent(lo, hi);
    let outer = outer_sign(&cloud.verts, thin, across);
    let uv = move |pos: [f32; 3]| planar_uv(pos, lo, hi, across, along);
    for tri in &cloud.tris {
        let facing: f32 = tri
            .iter()
            .map(|i| cloud.verts[idx(*i)].normal[thin])
            .sum::<f32>()
            / 3.0;
        let kind = if facing * outer > FACE_DOT {
            Kind::Face
        } else {
            Kind::Wood
        };
        builder.add(kind, cloud, *tri, 1, &uv);
    }
}

/// The two graft clouds taken out of the body, with the bind origin and the joint each rides.
struct Grafts {
    sword: Cloud,
    sword_pivot: [f32; 3],
    sword_joint: u16,
    shield: Cloud,
    shield_pivot: [f32; 3],
    shield_joint: u16,
}

/// Take the hero's sword and shield triangles out of the body. A body without the weapon
/// joints gives `None` and is not changed.
fn lift_grafts(body: &mut SkinnedBody) -> Option<Grafts> {
    let find = |name: &str| {
        body.skeleton
            .joints
            .iter()
            .position(|j| j.name.eq_ignore_ascii_case(name))
    };
    let (Some(sword), Some(shield)) = (find(SWORD_JOINT), find(SHIELD_JOINT)) else {
        return None;
    };
    let (Ok(sword_id), Ok(shield_id)) = (u16::try_from(sword), u16::try_from(shield)) else {
        return None;
    };
    let sword_pivot = body.inverse_binds.get(sword).map_or([0.0; 3], joint_origin);
    let shield_pivot = body
        .inverse_binds
        .get(shield)
        .map_or([0.0; 3], joint_origin);
    let sword_cloud = lift(body, sword_id);
    let shield_cloud = lift(body, shield_id);
    Some(Grafts {
        sword: sword_cloud,
        sword_pivot,
        sword_joint: sword_id,
        shield: shield_cloud,
        shield_pivot,
        shield_joint: shield_id,
    })
}

/// Today's procedural gear from the graft clouds: a shrunk gladius cut into iron, bronze and
/// wood, and the scutum cut into wood and painted face. The fallback when a table file does
/// not load, and the gear of the held proof body.
fn procedural(grafts: &mut Grafts) -> Vec<GearPart> {
    shrink(&mut grafts.sword, grafts.sword_pivot, SWORD_LEN_M);
    let mut builder = Builder::new();
    if !grafts.sword.tris.is_empty() {
        add_sword(&mut builder, &grafts.sword);
    }
    if !grafts.shield.tris.is_empty() {
        add_shield(&mut builder, &grafts.shield);
    }
    builder.parts.into_iter().flatten().collect()
}

/// Cut the hero's sword and shield out of `body` and return their pieces. Bodies without the
/// weapon joints or without weapon triangles give nothing and are not changed.
fn cut(body: &mut SkinnedBody) -> Vec<GearPart> {
    let Some(mut grafts) = lift_grafts(body) else {
        return Vec::new();
    };
    procedural(&mut grafts)
}

/// The bind directions of one graft: the longest extent is the blade (or the shield's
/// height), the second its width. The length points the way the graft's own triangles mass
/// lies from the grip, so the table's model is held the way the graft was.
fn graft_dirs(cloud: &Cloud, pivot: [f32; 3]) -> Option<([f32; 3], [f32; 3])> {
    if cloud.verts.is_empty() {
        return None;
    }
    let (lo, hi) = bounds(&cloud.verts);
    let [along, across, _] = axes_by_extent(lo, hi);
    let mut dir = [0.0; 3];
    dir[along] = 1.0;
    let mut mass = 0.0f32;
    for vert in &cloud.verts {
        for (axis, value) in vert.pos.iter().enumerate() {
            mass += (*value - pivot[axis]) * dir[axis];
        }
    }
    if mass < 0.0 {
        dir[along] = -1.0;
    }
    let mut wide = [0.0; 3];
    wide[across] = 1.0;
    Some((dir, wide))
}

/// The gear from the CC-BY table, placed on the hand joints, when every hand slot's file
/// loads. `None` when one does not: the caller keeps [`procedural`]. A missing file is not
/// an error, the gear is a look.
fn fitted(grafts: &Grafts, dir: &std::path::Path) -> Option<Vec<GearPart>> {
    let hands = [
        (
            table::GearSlot::SwordHand,
            &grafts.sword,
            grafts.sword_pivot,
            grafts.sword_joint,
        ),
        (
            table::GearSlot::ShieldArm,
            &grafts.shield,
            grafts.shield_pivot,
            grafts.shield_joint,
        ),
    ];
    let mut parts = Vec::with_capacity(hands.len());
    for (slot, cloud, pivot, joint) in hands {
        let row = table::row(slot);
        let import = table::load(dir, row.model)?;
        let (along, across) = graft_dirs(cloud, pivot)?;
        let fit = table::GripFit::of(row, along, across);
        let mut part = table::part(row, &import, &fit, joint)?;
        for pos in &mut part.positions {
            for (axis, value) in pos.iter_mut().enumerate() {
                *value += pivot[axis];
            }
        }
        parts.push(part);
    }
    Some(parts)
}

/// One cut piece as a body part: the template's material record with the piece's mesh, skin
/// rows, factors and generated map.
fn body_part(template: &BodyPart, piece: GearPart) -> BodyPart {
    let mut part = template.clone();
    part.name = piece.name.to_string();
    let count = piece.positions.len();
    part.mesh = CookedMesh {
        positions: piece.positions,
        normals: piece.normals,
        uv0: piece.uv0,
        tangents: vec![[1.0, 0.0, 0.0, 1.0]; count],
        indices: piece.indices,
    };
    part.skin = Skin {
        joints: piece.joints,
        weights: piece.weights,
    };
    part.material.base_color_factor = [1.0; 4];
    part.material.metallic_factor = piece.metallic;
    part.material.roughness_factor = piece.roughness;
    part.material.kind = MATERIAL_KIND_STONE;
    part.base_color_image = piece.image.or_else(|| piece.texels.png());
    part
}

/// How many parts [`arm_hero`] adds to the hero's body with the gear under `dir`: one per
/// armed hand slot when the table's files load, four procedural graft parts when they do
/// not. A caller that has to size its part list before arming asks this.
#[must_use]
pub fn armed_parts(dir: &std::path::Path) -> usize {
    if table::available(dir) {
        table::HAND_SLOTS.len()
    } else {
        4
    }
}

/// Cut the hero's sword and shield out of `body` and add them as body parts of their own:
/// the CC-BY Sketchfab gear from [`table`] when its files load (a gladius and a scutum, one
/// part each), else the procedural iron, bronze, wood and painted scutum face. Bodies
/// without the weapon joints or weapon triangles (the opponents) are left as they are.
/// Returns the parts added.
pub fn arm_hero(body: &mut SkinnedBody) -> usize {
    arm_hero_from(body, &table::source_dir())
}

/// [`arm_hero`] against an explicit gear directory: the table when every hand slot's file
/// under `dir` loads, the procedural graft gear otherwise. A missing directory is not an
/// error and never panics.
pub fn arm_hero_from(body: &mut SkinnedBody, dir: &std::path::Path) -> usize {
    let Some(template) = body.parts.first().cloned() else {
        return 0;
    };
    let Some(mut grafts) = lift_grafts(body) else {
        return 0;
    };
    let pieces = fitted(&grafts, dir).unwrap_or_else(|| procedural(&mut grafts));
    let added = pieces.len();
    body.parts
        .extend(pieces.into_iter().map(|piece| body_part(&template, piece)));
    added
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_assets::body_import::import_skinned_body;

    fn body(variant: &str) -> SkinnedBody {
        let path = crate::fighter_look::fighters_content_dir()
            .join("glb")
            .join(format!("{variant}_body.glb"));
        let bytes = std::fs::read(path).expect("body file");
        import_skinned_body(&bytes).expect("body imports")
    }

    fn luma(map: &Texels, uv: [f32; 2]) -> f32 {
        let x = ((uv[0] * map.width as f32) as u32).min(map.width - 1);
        let y = ((uv[1] * map.height as f32) as u32).min(map.height - 1);
        let at = ((y * map.width + x) * 4) as usize;
        let t = &map.rgba[at..at + 4];
        (0.2126 * f32::from(t[0]) + 0.7152 * f32::from(t[1]) + 0.0722 * f32::from(t[2])) / 255.0
    }

    #[test]
    fn the_hero_weapons_leave_the_body_and_become_four_materials() {
        let mut hero = body("hero");
        let before: usize = hero.parts.iter().map(|p| p.mesh.indices.len()).sum();
        let gear = cut(&mut hero);
        let names: Vec<_> = gear.iter().map(|p| p.name).collect();
        assert_eq!(
            names,
            [
                "gladius_blade",
                "gladius_fittings",
                "gear_wood",
                "scutum_face"
            ]
        );
        let after: usize = hero.parts.iter().map(|p| p.mesh.indices.len()).sum();
        let gear_indices: usize = gear.iter().map(|p| p.indices.len()).sum();
        assert_eq!(before, after + gear_indices, "every weapon triangle moved");
        assert!(gear_indices >= 3 * 200, "the graft is 222 triangles");
        let metals = gear.iter().filter(|p| p.metallic > 0.5).count();
        assert_eq!(
            metals, 2,
            "iron and bronze are metals, wood and paint are not"
        );
    }

    #[test]
    fn no_weapon_uv_lands_on_a_black_or_empty_texel() {
        let mut hero = body("hero");
        for part in cut(&mut hero) {
            assert_eq!(part.uv0.len(), part.positions.len(), "{}", part.name);
            for uv in &part.uv0 {
                assert!((0.0..=1.0).contains(&uv[0]) && (0.0..=1.0).contains(&uv[1]));
                let l = luma(&part.texels, *uv);
                assert!(l > 0.12, "{} samples a dark texel ({l})", part.name);
            }
            for row in &part.indices {
                assert!((*row as usize) < part.positions.len(), "{}", part.name);
            }
        }
    }

    #[test]
    fn the_sword_is_a_gladius_long_and_the_shield_keeps_its_size() {
        let mut hero = body("hero");
        let gear = cut(&mut hero);
        let long = |parts: &[&GearPart]| {
            let verts: Vec<Vert> = parts
                .iter()
                .flat_map(|p| p.positions.iter())
                .map(|pos| Vert {
                    pos: *pos,
                    normal: [0.0, 1.0, 0.0],
                    joints: [0; 4],
                    weights: [1.0, 0.0, 0.0, 0.0],
                })
                .collect();
            let (lo, hi) = bounds(&verts);
            (0..3).map(|a| hi[a] - lo[a]).fold(0.0f32, f32::max)
        };
        let blade = gear.iter().find(|p| p.name == "gladius_blade").unwrap();
        let fittings = gear.iter().find(|p| p.name == "gladius_fittings").unwrap();
        let sword = long(&[blade, fittings]);
        assert!(sword <= SWORD_LEN_M + 0.02, "sword is {sword} m");
        assert!(sword >= 0.5, "sword is not a stump: {sword} m");
        let face = gear.iter().find(|p| p.name == "scutum_face").unwrap();
        let shield = long(&[face]);
        assert!((0.7..=1.0).contains(&shield), "shield is {shield} m");
    }

    #[test]
    fn the_sword_stays_in_the_hand_when_it_shrinks() {
        let mut hero = body("hero");
        let joint = hero
            .skeleton
            .joints
            .iter()
            .position(|j| j.name == SWORD_JOINT)
            .unwrap();
        let pivot = joint_origin(&hero.inverse_binds[joint]);
        let gear = cut(&mut hero);
        let nearest = gear
            .iter()
            .filter(|p| p.name.starts_with("gladius"))
            .flat_map(|p| p.positions.iter())
            .map(|v| {
                (0..3)
                    .map(|a| (v[a] - pivot[a]).powi(2))
                    .sum::<f32>()
                    .sqrt()
            })
            .fold(f32::MAX, f32::min);
        assert!(
            nearest < 0.12,
            "the grip is {nearest} m from the palm joint"
        );
    }

    #[test]
    fn an_opponent_has_no_weapon_and_is_left_alone() {
        let mut opp = body("opp_male_light");
        let before = opp.clone();
        assert!(cut(&mut opp).is_empty());
        assert_eq!(arm_hero(&mut opp), 0, "no gear added");
        assert_eq!(opp, before);
    }

    #[test]
    fn the_same_body_gives_the_same_gear_twice() {
        let (mut a, mut b) = (body("hero"), body("hero"));
        assert_eq!(cut(&mut a), cut(&mut b));
    }

    #[test]
    fn arm_hero_arms_the_table_when_its_files_load_and_the_graft_when_they_do_not() {
        let dir = table::source_dir();
        let loaded = table::HAND_SLOTS
            .iter()
            .all(|slot| table::load(&dir, table::row(*slot).model).is_some());
        let mut hero = body("hero");
        let before = hero.parts.len();
        let added = arm_hero(&mut hero);
        if loaded {
            assert_eq!(
                added,
                table::HAND_SLOTS.len(),
                "one glb part per armed hand slot"
            );
            let names: Vec<&str> = hero.parts[before..]
                .iter()
                .map(|part| part.name.as_str())
                .collect();
            assert_eq!(names, ["gladius_blade", "scutum_face"]);
        } else {
            assert_eq!(added, 4, "the procedural graft gear");
        }
        assert_eq!(hero.parts.len(), before + added);
        for part in &hero.parts[before..] {
            let image = part.base_color_image.as_ref().expect("a base-colour image");
            engine_render::present::TextureUpload::from_encoded(image).expect("the image decodes");
            assert_eq!(
                part.skin.joints.len(),
                part.mesh.positions.len(),
                "{}",
                part.name
            );
            assert_eq!(part.mesh.tangents.len(), part.mesh.positions.len());
        }
    }

    #[test]
    fn a_missing_gear_directory_keeps_the_procedural_gear_without_a_panic() {
        let missing = std::path::Path::new("target/no-such-gear-directory");
        let mut hero = body("hero");
        let before = hero.parts.len();
        assert_eq!(arm_hero_from(&mut hero, missing), 4, "the graft gear");
        let names: Vec<&str> = hero.parts[before..]
            .iter()
            .map(|part| part.name.as_str())
            .collect();
        assert_eq!(
            names,
            [
                "gladius_blade",
                "gladius_fittings",
                "gear_wood",
                "scutum_face"
            ]
        );
        // The generated wood map survives the png round trip on the fallback path.
        let wood = tex::wood();
        let found = hero.parts[before..].iter().any(|part| {
            let png = part.base_color_image.as_ref().expect("a base-colour image");
            let upload =
                engine_render::present::TextureUpload::from_encoded(png).expect("png decodes");
            (upload.width, upload.height) == (wood.width, wood.height) && upload.rgba == wood.rgba
        });
        assert!(found, "the wood map survives the png round trip");
        // An opponent still has no gear, with or without a directory.
        let mut opp = body("opp_male_light");
        assert_eq!(arm_hero_from(&mut opp, missing), 0);
        assert_eq!(arm_hero_from(&mut opp, &table::source_dir()), 0);
    }
}
