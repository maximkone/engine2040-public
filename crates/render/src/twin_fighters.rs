//! twin_fighters: fighter draw bind for the window twin (RL26).
//!
//! Owner: render lane; consumer is the WIN1 twin `frame_list` in
//! `crates/game/src/bin/arena_capture.rs` under `--fighters 8 --scene arena`.
//! Before this bind the twin drew one lit cube plus the dressing boxes, so an
//! 8-fighter capture advanced the sim with zero fighter pixels. This bind
//! stages one box body per fighter slot (slot 0 is the hero, slots 1..7 the
//! staged opponents) and pushes one draw per resident body in slot order.
//! Bind glue only: no pass code, no shader, no sim reads, no per-frame
//! upload. The skinned CT7 look needs `FighterLook` with `&mut Presenter`,
//! which the twin frame list cannot reach, so the hero is a red stand-in box.
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

/// Box body mesh for one slot: the unit cube scaled by
/// [`TWIN_FIGHTER_SIZE_M`] and translated so its feet sit at the slot's feet.
pub fn twin_fighter_mesh(slot: u8) -> Result<v1::MeshData, TwinBodyError> {
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

/// Upload one body mesh per staged slot (`0..count`). Cold init path only; a
/// refused GPU upload skips that body. Returns bodies uploaded.
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
        }
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
