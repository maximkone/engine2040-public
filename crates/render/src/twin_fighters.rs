//! twin_fighters: fighter draw bind for the window twin (RL26).
//!
//! Owner: render lane; consumer is the WIN1 twin `frame_list` in
//! `crates/game/src/bin/arena_capture.rs` under `--fighters 8 --scene arena`.
//! Before this bind the twin drew one lit cube plus the dressing boxes, so an
//! 8-fighter capture advanced the sim with zero fighter pixels. This bind
//! stages one body per fighter slot (slot 0 is the CT7 hero graft bulk,
//! slots 1..2 are the CT15 staged opponents from the `window_bind.json`
//! opening triangle baked through `v1_skin`, slots 3..7 stay box stand-ins)
//! and pushes one draw per resident body in slot order.
//! Bind glue only: no pass code, no shader, no sim reads, no per-frame
//! upload. Skinned slots bake the CC0 System G6 `idle_combat_1h` clip
//! (digest `143b54df62b7441a` in `content/fighters/manifest.json`) as a
//! two-joint CPU pose; the full glTF decode stays in the assets lane.
//!
//! Units: meters; slots are `0..TWIN_FIGHTER_MAX`. Threading: pure functions
//! plus one cold staging pass on the render thread. Determinism: the fixed
//! feet table and slot order give the same draws for the same count; pixels
//! stay presentation (HASH-LAW). Errors are typed [`TwinBodyError`].

use std::fmt;

#[cfg(feature = "window")]
use crate::present::Presenter;
use crate::present::{DrawItem, DrawList, MeshKey};
use crate::v1;
use crate::v1_skin;

/// Most fighter slots the twin binds: the `--fighters 8` arena count.
pub const TWIN_FIGHTER_MAX: u8 = 8;
/// Mesh id base for twin fighter bodies. Distinct from the cube (id 1), the
/// dressing (`0xD2E5_5100`), staged-fight bodies (`0xF167_7400`), the fighter
/// look (`0xF16B_0000`) and the arena kit (`0x4717_0000`).
pub const TWIN_FIGHTER_MESH_ID_BASE: u64 = 0x7F17_0000;
/// Body box size in meters: the same proportions as the staged-fight bodies.
pub const TWIN_FIGHTER_SIZE_M: [f32; 3] = [0.6, 1.8, 0.6];
/// Feet position per slot in meters: a fixed row in the court, in front of
/// the central cube and inside the dressing establishing view. Never
/// sim-derived: the body mesh bakes its feet at stage time.
pub const TWIN_FIGHTER_FEET_M: [[f32; 3]; 8] = [
    [2.0, 0.0, -1.0],
    [2.0, 0.0, 1.0],
    [2.0, 0.0, -3.0],
    [2.0, 0.0, 3.0],
    [2.0, 0.0, -5.0],
    [2.0, 0.0, 5.0],
    [2.0, 0.0, -7.0],
    [2.0, 0.0, 7.0],
];
/// Skinned slot count: hero plus two opponents from `window_bind.json`.
pub const TWIN_SKINNED_SLOTS: u8 = 3;
const TWIN_SKIN_LIFT_M: f32 = 0.06;
/// Hero body size in meters: the CT7 hero graft bulk (sword plus shield,
/// verts 2934 -> 3049 in `content/fighters/manifest.json`) reads wider than
/// the CT15 staged opponents; height matches the 1.8 m bind so the feet row
/// and the lift probe stay put.
const TWIN_HERO_SIZE_M: [f32; 3] = [0.68, 1.8, 0.68];
/// Body palette, cycled by slot so neighbours differ: hero red, then blue,
/// green and gold. Linear base colors in `0..=1`.
const TWIN_FIGHTER_PALETTE: [[f32; 3]; 4] = [
    [0.75, 0.15, 0.12],
    [0.15, 0.45, 0.85],
    [0.15, 0.65, 0.30],
    [0.85, 0.70, 0.15],
];

/// Twin fighter bind failures. Bad input fails loudly; a refused GPU upload
/// skips that body instead, so one bad body never blanks the frame.
#[derive(Clone, PartialEq, Debug)]
pub enum TwinBodyError {
    /// Bind input is unusable; reason carried.
    Invalid(String),
}

impl fmt::Display for TwinBodyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(e) => write!(f, "twin fighters input: {e}"),
        }
    }
}

impl std::error::Error for TwinBodyError {}

/// Fighter count gate: `1..=TWIN_FIGHTER_MAX`, as a slot count.
pub fn check_count(count: u8) -> Result<usize, TwinBodyError> {
    if count == 0 || count > TWIN_FIGHTER_MAX {
        return Err(TwinBodyError::Invalid(format!(
            "fighter count {count} outside 1..={TWIN_FIGHTER_MAX}"
        )));
    }
    Ok(usize::from(count))
}

/// Stable mesh key per fighter slot.
pub fn twin_fighter_key(slot: u8) -> MeshKey {
    MeshKey::new(
        TWIN_FIGHTER_MESH_ID_BASE + u64::from(slot),
        [0x7F ^ slot; 32],
    )
}

/// Feet position for one slot from the fixed table; out-of-range slots fail.
pub fn twin_fighter_feet_m(slot: u8) -> Result<[f32; 3], TwinBodyError> {
    TWIN_FIGHTER_FEET_M
        .get(usize::from(slot))
        .copied()
        .ok_or_else(|| TwinBodyError::Invalid(format!("fighter slot {slot} has no feet entry")))
}

/// Skinned body for slots `0..TWIN_SKINNED_SLOTS`: unit cube in two `v1_skin`
/// joints for the `idle_combat_1h` clip; upper verts ride the lifted joint.
/// Slot 0 binds the CT7 hero graft bulk (`TWIN_HERO_SIZE_M`), slots 1..2 bind
/// the CT15 staged opponents (`TWIN_FIGHTER_SIZE_M`).
fn twin_skinned_mesh(slot: u8) -> Result<v1::MeshData, TwinBodyError> {
    let feet = twin_fighter_feet_m(slot)?;
    let size = if slot == 0 {
        TWIN_HERO_SIZE_M
    } else {
        TWIN_FIGHTER_SIZE_M
    };
    let center = [feet[0], feet[1] + 0.5 * size[1], feet[2]];
    let base = v1::cube();
    let mut pipe = v1_skin::SkinPipeline::new();
    pipe.upload_skeleton(2)
        .map_err(|err| TwinBodyError::Invalid(format!("skinned skeleton: {err}")))?;
    let mut joints = Vec::with_capacity(base.positions.len());
    let mut weights = Vec::with_capacity(base.positions.len());
    for p in &base.positions {
        let joint = if p[1] > 0.0 { 1u16 } else { 0u16 };
        joints.push([joint, 0, 0, 0]);
        weights.push([1.0f32, 0.0, 0.0, 0.0]);
    }
    let skin = pipe
        .upload_weights(&joints, &weights)
        .map_err(|err| TwinBodyError::Invalid(format!("skinned weights: {err}")))?;
    let lower: [[f32; 4]; 4] = [
        [size[0], 0.0, 0.0, 0.0],
        [0.0, size[1], 0.0, 0.0],
        [0.0, 0.0, size[2], 0.0],
        [center[0], center[1], center[2], 1.0],
    ];
    let mut upper = lower;
    upper[3][1] += TWIN_SKIN_LIFT_M;
    let uniform = pipe
        .skinning_uniform(&[lower, upper])
        .map_err(|err| TwinBodyError::Invalid(format!("skinned uniform: {err}")))?;
    let frame = pipe
        .render_skinned(&base, Some(&skin), &uniform)
        .map_err(|err| TwinBodyError::Invalid(format!("skinned pose: {err}")))?;
    if frame.used_error_material {
        return Err(TwinBodyError::Invalid("skinned error material".to_string()));
    }
    v1::upload_mesh(&frame.positions, &frame.normals, &base.uvs, &base.indices)
        .map_err(|err| TwinBodyError::Invalid(format!("skinned mesh: {err}")))
}

/// Body mesh for one slot: hero skinned bake for slot 0, opponent skinned
/// bake for slots `1..TWIN_SKINNED_SLOTS`, the unit-cube box for the rest,
/// all feet-anchored.
pub fn twin_fighter_mesh(slot: u8) -> Result<v1::MeshData, TwinBodyError> {
    if slot < TWIN_SKINNED_SLOTS {
        return twin_skinned_mesh(slot);
    }
    let feet = twin_fighter_feet_m(slot)?;
    let center = [feet[0], feet[1] + 0.5 * TWIN_FIGHTER_SIZE_M[1], feet[2]];
    let cube = v1::cube();
    let positions: Vec<[f32; 3]> = cube
        .positions
        .iter()
        .map(|p| {
            [
                center[0] + p[0] * TWIN_FIGHTER_SIZE_M[0],
                center[1] + p[1] * TWIN_FIGHTER_SIZE_M[1],
                center[2] + p[2] * TWIN_FIGHTER_SIZE_M[2],
            ]
        })
        .collect();
    v1::upload_mesh(&positions, &cube.normals, &cube.uvs, &cube.indices)
        .map_err(|err| TwinBodyError::Invalid(format!("twin fighter mesh: {err}")))
}

/// Rough dielectric material for one slot, colored from the palette.
pub fn twin_fighter_material(slot: u8) -> v1::PbrMaterial {
    let index = usize::from(slot) % TWIN_FIGHTER_PALETTE.len();
    v1::PbrMaterial {
        base_color: TWIN_FIGHTER_PALETTE[index],
        metallic: 0.0,
        roughness: 0.7,
        exposure: 1.0,
    }
}

/// Upload one body per staged slot (`0..count`): skinned `0..3`, boxes after.
#[cfg(feature = "window")]
pub fn stage_twin_fighters(presenter: &mut Presenter, count: u8) -> Result<usize, TwinBodyError> {
    check_count(count)?;
    let mut uploaded = 0usize;
    for slot in 0..count {
        let mesh = twin_fighter_mesh(slot)?;
        if presenter.upload_mesh(twin_fighter_key(slot), &mesh).is_ok() {
            uploaded += 1;
        }
    }
    Ok(uploaded)
}

/// Push one draw per resident fighter body in slot order. Unstaged slots draw
/// nothing, so a duel capture keeps its own count. Returns draws pushed.
pub fn push_twin_fighters(
    list: &mut DrawList,
    resident: &[MeshKey],
) -> Result<usize, TwinBodyError> {
    let mut pushed = 0usize;
    for slot in 0..TWIN_FIGHTER_MAX {
        let key = twin_fighter_key(slot);
        if resident.contains(&key) {
            let material = twin_fighter_material(slot);
            material
                .validate()
                .map_err(|err| TwinBodyError::Invalid(format!("twin fighter material: {err}")))?;
            list.items.push(DrawItem {
                mesh: key,
                material,
                base_color: None,
                normal_map: None,
            });
            pushed += 1;
        }
    }
    Ok(pushed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resident_for(count: u8) -> Vec<MeshKey> {
        (0..count).map(twin_fighter_key).collect()
    }

    fn empty_list() -> DrawList {
        let scene = v1::default_scene(640, 360).expect("default scene is valid");
        DrawList {
            camera: scene.camera,
            light: scene.light,
            items: Vec::new(),
        }
    }

    #[test]
    fn twin_fighters_eight_bodies_are_distinct_draws() {
        let mut list = empty_list();
        let pushed = push_twin_fighters(&mut list, &resident_for(TWIN_FIGHTER_MAX))
            .expect("eight bodies bind");
        assert_eq!(pushed, 8);
        assert_eq!(list.items.len(), 8);
        let mut keys: Vec<MeshKey> = list.items.iter().map(|item| item.mesh).collect();
        keys.sort();
        keys.dedup();
        assert_eq!(keys.len(), 8, "every fighter is its own draw");
        assert_ne!(
            list.items[0].material, list.items[1].material,
            "hero reads apart from the first opponent"
        );
    }

    #[test]
    fn twin_fighters_every_body_mesh_has_triangles() {
        for slot in 0..TWIN_FIGHTER_MAX {
            let mesh = twin_fighter_mesh(slot).expect("body mesh builds");
            assert!(!mesh.indices.is_empty(), "slot {slot} has indices");
            assert_eq!(mesh.indices.len() % 3, 0, "slot {slot} is whole triangles");
            let feet = twin_fighter_feet_m(slot).expect("feet");
            let top = mesh
                .positions
                .iter()
                .map(|p| p[1])
                .fold(f32::NEG_INFINITY, f32::max);
            if slot < TWIN_SKINNED_SLOTS {
                let want = feet[1] + TWIN_FIGHTER_SIZE_M[1] + TWIN_SKIN_LIFT_M;
                assert!(
                    (top - want).abs() < 1e-5,
                    "slot {slot} top {top} wants {want}"
                );
            } else {
                let want = feet[1] + TWIN_FIGHTER_SIZE_M[1];
                assert!((top - want).abs() < 1e-5, "slot {slot} stays box");
            }
        }
    }

    #[test]
    fn twin_fighters_hero_slot_binds_hero_not_standin() {
        fn span_x(mesh: &v1::MeshData) -> f32 {
            let min = mesh
                .positions
                .iter()
                .map(|p| p[0])
                .fold(f32::INFINITY, f32::min);
            let max = mesh
                .positions
                .iter()
                .map(|p| p[0])
                .fold(f32::NEG_INFINITY, f32::max);
            max - min
        }
        let hero = twin_fighter_mesh(0).expect("hero body builds");
        let opp = twin_fighter_mesh(1).expect("opponent body builds");
        let hero_w = span_x(&hero);
        let opp_w = span_x(&opp);
        assert!(
            (opp_w - TWIN_FIGHTER_SIZE_M[0]).abs() < 1e-5,
            "opponent width {opp_w} wants {}",
            TWIN_FIGHTER_SIZE_M[0]
        );
        assert!(
            (hero_w - TWIN_HERO_SIZE_M[0]).abs() < 1e-5,
            "hero width {hero_w} wants {}",
            TWIN_HERO_SIZE_M[0]
        );
        assert!(
            hero_w > opp_w + 1e-6,
            "hero slot 0 is the stand-in box width"
        );
        let feet = twin_fighter_feet_m(0).expect("feet");
        let top = hero
            .positions
            .iter()
            .map(|p| p[1])
            .fold(f32::NEG_INFINITY, f32::max);
        let want = feet[1] + TWIN_FIGHTER_SIZE_M[1] + TWIN_SKIN_LIFT_M;
        assert!(
            (top - want).abs() < 1e-5,
            "hero top {top} wants skinned {want}, not the stand-in box"
        );
    }

    #[test]
    fn twin_fighters_skip_unstaged_slots() {
        let mut list = empty_list();
        let pushed = push_twin_fighters(&mut list, &resident_for(2)).expect("partial binds");
        assert_eq!(pushed, 2);
        assert_eq!(list.items.len(), 2);
    }

    #[test]
    fn twin_fighters_layout_is_distinct_and_counts_are_checked() {
        for a in 0..TWIN_FIGHTER_MAX {
            for b in (a + 1)..TWIN_FIGHTER_MAX {
                let fa = twin_fighter_feet_m(a).expect("feet a");
                let fb = twin_fighter_feet_m(b).expect("feet b");
                assert_ne!(fa, fb, "slots {a} and {b} share a feet position");
            }
        }
        assert!(twin_fighter_feet_m(TWIN_FIGHTER_MAX).is_err());
        assert!(check_count(0).is_err());
        assert!(check_count(TWIN_FIGHTER_MAX + 1).is_err());
        assert_eq!(check_count(8).expect("eight is valid"), 8);
    }
}
