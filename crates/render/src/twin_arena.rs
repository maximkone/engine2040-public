//! twin_arena: arena kit draw bind for the window twin.
//!
//! Owner: render lane; consumer is the twin `frame_list` in
//! `crates/game/src/bin/arena_capture.rs` under `--scene arena`.
//! Before this bind the capture drew one synthetic central cube; when the
//! recooked arena kit (CT8) is resident this bind swaps the cube for the kit
//! draws in placement order, otherwise the cube stays as fallback.
//! Bind glue only: no pass code, no shader, no sim reads, no per-frame
//! upload. Units: meters for meshes, 0..=1 factors for materials.
//! Determinism: placement order in gives the same draws out; pixels stay
//! presentation (HASH-LAW). Errors are typed [`TwinArenaError`].

use std::fmt;

use crate::present::{DrawItem, DrawList, MeshKey, TextureKey};
use crate::v1::PbrMaterial;

/// Twin arena bind failures. Bad bind input fails loudly; a refused GPU
/// upload never reaches here because staging skips it, so one bad draw never
/// blanks the frame.
#[derive(Clone, PartialEq, Debug)]
pub enum TwinArenaError {
    /// Bind input is unusable; reason carried.
    Invalid(String),
}

impl fmt::Display for TwinArenaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(e) => write!(f, "twin arena input: {e}"),
        }
    }
}

impl std::error::Error for TwinArenaError {}

/// Maximum kit draws that may replace the synthetic cube in one frame.
/// Units: count of draws; determinism: same limit every call; pixels stay
/// presentation (HASH-LAW). An over-budget kit keeps the cube and fails.
pub const TWIN_ARENA_MAX_DRAWS: usize = 256;

/// Synthetic central cube key the kit replaces when resident.
/// Units: none (identity); determinism: same bytes every call.
#[must_use]
pub fn twin_arena_cube_key() -> MeshKey {
    MeshKey::new(1, [0xA1; 32])
}

/// Count resident kit draws in placement order, excluding the fallback cube.
///
/// Pure; same resident set plus pairs give the same count. The cube key never
/// counts, so polluted kit data cannot inflate the budget. Determinism: order
/// independent, integer only.
/// Caller: [`push_twin_arena`] budget check, so the budget counts what the
/// push would stage.
#[must_use]
pub fn twin_arena_resident_count(resident: &[MeshKey], draws: &[(MeshKey, PbrMaterial)]) -> usize {
    let cube = twin_arena_cube_key();
    draws
        .iter()
        .filter(|(key, _)| *key != cube && resident.contains(key))
        .count()
}

/// Count resident textured kit draws, excluding the fallback cube.
///
/// Same semantics as [`twin_arena_resident_count`] for triples carrying the
/// base-color lane. Pure; same inputs give the same count.
/// Caller: [`push_twin_arena_textured`] budget check.
#[must_use]
pub fn twin_arena_resident_count_textured(
    resident: &[MeshKey],
    draws: &[(MeshKey, PbrMaterial, Option<TextureKey>)],
) -> usize {
    let cube = twin_arena_cube_key();
    draws
        .iter()
        .filter(|(key, _, _)| *key != cube && resident.contains(key))
        .count()
}

/// Kit present check: true when staged kit would replace the cube.
///
/// Pure, integer only, no allocation. Same inputs give the same bool;
/// empty stays false because the count is zero.
/// Caller: capture gate single check (tests-only in this slice).
#[must_use]
pub fn twin_arena_kit_present(resident: &[MeshKey], draws: &[(MeshKey, PbrMaterial)]) -> bool {
    twin_arena_resident_count(resident, draws) > 0
}

/// Kit present check for textured draws: true when staged textured kit
/// would replace the cube.
///
/// Pure, integer only, no allocation. Same inputs give the same bool;
/// empty stays false because the count is zero. Cube pollution never
/// counts, so fallback data cannot force a bind.
/// Caller: [`push_twin_arena_textured`] budget gate below, so the gate
/// counts what the push would stage.
#[must_use]
pub fn twin_arena_kit_present_textured(
    resident: &[MeshKey],
    draws: &[(MeshKey, PbrMaterial, Option<TextureKey>)],
) -> bool {
    twin_arena_resident_count_textured(resident, draws) > 0
}

/// Swap the synthetic cube for resident arena kit draws.
///
/// Takes the per-frame draw list (which holds the cube fallback), the
/// resident mesh keys from the presenter, plus the staged arena draws in
/// placement order as `(MeshKey, PbrMaterial)` pairs. When at least one arena
/// draw is resident, removes the cube item and pushes every resident arena
/// draw in order; otherwise leaves the list alone so the cube stays.
/// Returns draws pushed. Invalid materials fail loudly; missing keys draw
/// nothing. Re-binding draws already in the list pushes nothing and returns
/// 0, so the headless twin's double bind draws once. Determinism: same
/// resident set plus pairs give the same list.
pub fn push_twin_arena(
    list: &mut DrawList,
    resident: &[MeshKey],
    draws: &[(MeshKey, PbrMaterial)],
) -> Result<usize, TwinArenaError> {
    // Fallback pollution guard: kit draws never contain the cube key, so the
    // cube in `draws` is ignored everywhere (residency, validation, budget,
    // push). This keeps the cube-swap guarantee: the cube leaves whenever
    // real kit is resident, never returns from polluted input.
    let cube = twin_arena_cube_key();
    let is_kit = |key: &MeshKey| *key != cube && resident.contains(key);
    let any_resident = draws.iter().any(|(key, _)| is_kit(key));
    if !any_resident {
        return Ok(0);
    }
    for (key, material) in draws {
        if !is_kit(key) {
            continue;
        }
        material
            .validate()
            .map_err(|err| TwinArenaError::Invalid(format!("twin arena material: {err}")))?;
    }
    let would_push = twin_arena_resident_count(resident, draws);
    if would_push > TWIN_ARENA_MAX_DRAWS {
        return Err(TwinArenaError::Invalid(format!(
            "twin arena over budget: {would_push} draws exceed {TWIN_ARENA_MAX_DRAWS}"
        )));
    }
    list.items.retain(|item| item.mesh != cube);
    // Idempotent re-bind: the headless twin binds the kit in `frame_list` and
    // then binds the same kit again, so keys already present stay single;
    // only the pre-bind prefix counts, so two copies of one mesh inside
    // `draws` still push twice for the RL32 group loop.
    let bound = list.items.len();
    let mut pushed = 0usize;
    for (key, material) in draws {
        if !is_kit(key) {
            continue;
        }
        if list.items[..bound].iter().any(|item| item.mesh == *key) {
            continue;
        }
        list.items.push(DrawItem {
            mesh: *key,
            material: material.clone(),
            base_color: None,
            normal_map: None,
        });
        pushed += 1;
    }
    Ok(pushed)
}

/// Swap the synthetic cube for resident arena kit draws with textures.
///
/// Same cube-swap semantics as [`push_twin_arena`], except each staged draw
/// carries its base-color texture key in the tuple's third lane and that key
/// lands in [`DrawItem::base_color`]. A `None` lane binds the 1x1 white
/// fallback, so untextured draws render as before. Takes the per-frame draw
/// list (which holds the cube fallback), the resident mesh keys from the
/// presenter, plus the staged arena draws in placement order as
/// `(MeshKey, PbrMaterial, Option<TextureKey>)` triples. When at least one
/// arena draw is resident, removes the cube item and pushes every resident
/// arena draw in order; otherwise leaves the list alone so the cube stays.
/// Returns draws pushed. Invalid materials fail loudly; missing keys draw
/// nothing. Re-binding draws already in the list pushes nothing and returns
/// 0, so the headless twin's double bind draws once. Determinism: same
/// resident set plus triples give the same list.
pub fn push_twin_arena_textured(
    list: &mut DrawList,
    resident: &[MeshKey],
    draws: &[(MeshKey, PbrMaterial, Option<TextureKey>)],
) -> Result<usize, TwinArenaError> {
    // Same cube-pollution guard as [`push_twin_arena`]: the cube key in
    // `draws` is fallback, never kit, so every stage ignores it. The cube
    // leaves whenever real kit is resident.
    let cube = twin_arena_cube_key();
    let is_kit = |key: &MeshKey| *key != cube && resident.contains(key);
    let any_resident = twin_arena_kit_present_textured(resident, draws);
    if !any_resident {
        return Ok(0);
    }
    for (key, material, _) in draws {
        if !is_kit(key) {
            continue;
        }
        material
            .validate()
            .map_err(|err| TwinArenaError::Invalid(format!("twin arena material: {err}")))?;
    }
    let would_push = twin_arena_resident_count_textured(resident, draws);
    if would_push > TWIN_ARENA_MAX_DRAWS {
        return Err(TwinArenaError::Invalid(format!(
            "twin arena over budget: {would_push} draws exceed {TWIN_ARENA_MAX_DRAWS}"
        )));
    }
    list.items.retain(|item| item.mesh != cube);
    // Idempotent re-bind, same contract as [`push_twin_arena`]: keys already
    // present from the first bind stay single; the pre-bind prefix alone
    // counts, so two copies of one mesh inside `draws` still push twice.
    let bound = list.items.len();
    let mut pushed = 0usize;
    for (key, material, base_color) in draws {
        if !is_kit(key) {
            continue;
        }
        if list.items[..bound].iter().any(|item| item.mesh == *key) {
            continue;
        }
        list.items.push(DrawItem {
            mesh: *key,
            material: material.clone(),
            base_color: *base_color,
            normal_map: None,
        });
        pushed += 1;
    }
    Ok(pushed)
}

/// Count distinct mesh groups in placement order.
///
/// One group per distinct [`MeshKey`], first-seen order, stable under input
/// permutation. The live RL32 draw loop draws each group once, so 256 copies
/// of one kit mesh make 1 group. Pure, no alloc, integer only.
/// The fallback cube key never counts: kit groups exclude fallback pollution.
/// Determinism: same pairs give the same count.
#[must_use]
pub fn twin_arena_group_count(draws: &[(MeshKey, PbrMaterial)]) -> usize {
    let cube = twin_arena_cube_key();
    let mut groups = 0usize;
    let mut i = 0usize;
    while i < draws.len() {
        let key = draws[i].0;
        if key == cube {
            i += 1;
            continue;
        }
        let mut seen = false;
        let mut j = 0usize;
        while j < i {
            if draws[j].0 == key {
                seen = true;
                break;
            }
            j += 1;
        }
        if !seen {
            groups += 1;
        }
        i += 1;
    }
    groups
}

/// Count distinct textured mesh groups in placement order.
///
/// One group per distinct [`MeshKey`], first-seen order, stable under input
/// permutation. Texture keys never split groups, so the same kit counts the
/// same with or without texture keys for the RL32 one-draw-per-group loop;
/// 256 copies of one kit mesh make 1 group. Pure, no alloc, integer only.
/// The fallback cube key never counts: kit groups exclude fallback pollution.
/// Determinism: same triples give the same count.
/// UNLOCKS RL32 one-draw-per-group loop (parity with [`twin_arena_group_count`]).
#[must_use]
pub fn twin_arena_group_count_textured(
    draws: &[(MeshKey, PbrMaterial, Option<TextureKey>)],
) -> usize {
    let cube = twin_arena_cube_key();
    let mut groups = 0usize;
    for (index, draw) in draws.iter().enumerate() {
        let key = draw.0;
        if cube == key {
            continue;
        }
        let mut prior = false;
        for prior_draw in draws.iter().take(index) {
            if key == prior_draw.0 {
                prior = true;
                break;
            }
        }
        if !prior {
            groups += 1;
        }
    }
    groups
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v1;

    fn empty_list_with_cube() -> DrawList {
        let scene = v1::default_scene(640, 360).expect("default scene is valid");
        let mut list = DrawList {
            camera: scene.camera,
            light: scene.light,
            items: Vec::new(),
        };
        list.items.push(DrawItem {
            mesh: twin_arena_cube_key(),
            material: scene.material,
            base_color: None,
            normal_map: None,
        });
        list
    }

    fn arena_pair(id: u64, digest: u8) -> (MeshKey, PbrMaterial) {
        (
            MeshKey::new(id, [digest; 32]),
            PbrMaterial {
                base_color: [0.5, 0.5, 0.5],
                metallic: 0.0,
                roughness: 0.9,
                exposure: 1.0,
            },
        )
    }

    #[test]
    fn twin_arena_replaces_cube_when_kit_resident() {
        let mut list = empty_list_with_cube();
        let draws = [arena_pair(0x4717_0001, 0x11), arena_pair(0x4717_0002, 0x22)];
        let resident = [twin_arena_cube_key(), draws[0].0, draws[1].0];
        let pushed = push_twin_arena(&mut list, &resident, &draws).expect("kit resident binds");
        assert_eq!(pushed, 2);
        assert_eq!(list.items.len(), 2);
        assert!(
            !list
                .items
                .iter()
                .any(|item| item.mesh == twin_arena_cube_key()),
            "cube leaves when the kit lands"
        );
        assert_eq!(list.items[0].mesh, draws[0].0);
        assert_eq!(list.items[1].mesh, draws[1].0);
    }

    #[test]
    fn twin_arena_kit_present_tracks_push_decision() {
        let kit_a = arena_pair(0x4717_0001, 0x11);
        let kit_b = arena_pair(0x4717_0002, 0x22);
        let draws = [kit_a.clone(), kit_b.clone()];
        let resident = [twin_arena_cube_key(), kit_a.0, kit_b.0];
        assert!(twin_arena_kit_present(&resident, &draws));
        let cube_only = [twin_arena_cube_key()];
        assert!(!twin_arena_kit_present(&cube_only, &draws));
        let empty: [(MeshKey, PbrMaterial); 0] = [];
        assert!(!twin_arena_kit_present(&cube_only, &empty));
        let pollution = arena_pair(1, 0xA1);
        assert_eq!(pollution.0, twin_arena_cube_key());
        let polluted = [pollution, kit_a.clone()];
        assert!(twin_arena_kit_present(&resident, &polluted));
    }

    #[test]
    fn twin_arena_kit_present_rerun_matches() {
        let kit = arena_pair(0x4717_0001, 0x11);
        let draws = [kit.clone()];
        let resident = [twin_arena_cube_key(), kit.0];
        assert_eq!(
            twin_arena_kit_present(&resident, &draws),
            twin_arena_kit_present(&resident, &draws)
        );
    }

    #[test]
    fn twin_arena_textured_present_tracks_push_decision() {
        let a = arena_textured_pair(0x4717_0001, 0x11, 0xB001, 0xA1);
        let b = arena_textured_pair(0x4717_0002, 0x22, 0xB002, 0xA2);
        let draws = [a.clone(), b.clone()];
        let resident = [twin_arena_cube_key(), a.0, b.0];
        assert!(twin_arena_kit_present_textured(&resident, &draws));
        let cube_only = [twin_arena_cube_key()];
        assert!(!twin_arena_kit_present_textured(&cube_only, &draws));
        let empty: [(MeshKey, PbrMaterial, Option<TextureKey>); 0] = [];
        assert!(!twin_arena_kit_present_textured(&cube_only, &empty));
        let pollution = (twin_arena_cube_key(), a.1.clone(), None);
        let polluted = [pollution, a.clone()];
        assert!(twin_arena_kit_present_textured(&resident, &polluted));
        assert_eq!(
            twin_arena_kit_present_textured(&resident, &draws),
            twin_arena_kit_present_textured(&resident, &draws)
        );
    }

    #[test]
    fn twin_arena_keeps_cube_without_kit() {
        let mut list = empty_list_with_cube();
        let draws = [arena_pair(0x4717_0001, 0x11), arena_pair(0x4717_0002, 0x22)];
        let resident = [twin_arena_cube_key()];
        let pushed = push_twin_arena(&mut list, &resident, &draws).expect("fallback binds");
        assert_eq!(pushed, 0);
        assert_eq!(list.items.len(), 1);
        assert_eq!(list.items[0].mesh, twin_arena_cube_key());
    }

    #[test]
    fn twin_arena_rejects_invalid_material() {
        let mut list = empty_list_with_cube();
        let bad = (
            MeshKey::new(0x4717_0001, [0x11; 32]),
            PbrMaterial {
                base_color: [2.0, 0.0, 0.0],
                metallic: 0.0,
                roughness: 0.9,
                exposure: 1.0,
            },
        );
        let resident = [twin_arena_cube_key(), bad.0];
        assert!(push_twin_arena(&mut list, &resident, &[bad]).is_err());
    }

    #[test]
    fn twin_arena_invalid_material_leaves_cube() {
        let mut list = empty_list_with_cube();
        let bad = (
            MeshKey::new(0x4717_0001, [0x11; 32]),
            PbrMaterial {
                base_color: [2.0, 0.0, 0.0],
                metallic: 0.0,
                roughness: 0.9,
                exposure: 1.0,
            },
        );
        let resident = [twin_arena_cube_key(), bad.0];
        assert!(push_twin_arena(&mut list, &resident, &[bad]).is_err());
        assert_eq!(list.items.len(), 1, "invalid bind pushes nothing");
        assert!(
            list.items
                .iter()
                .any(|item| item.mesh == twin_arena_cube_key()),
            "cube stays when the kit bind fails"
        );
    }

    #[test]
    fn twin_arena_textured_invalid_material_leaves_cube() {
        let mut list = empty_list_with_cube();
        let bad = (
            MeshKey::new(0x4717_0001, [0x11; 32]),
            PbrMaterial {
                base_color: [2.0, 0.0, 0.0],
                metallic: 0.0,
                roughness: 0.9,
                exposure: 1.0,
            },
            Some(TextureKey::new(0xB001, [0xA1; 32])),
        );
        let resident = [twin_arena_cube_key(), bad.0];
        assert!(push_twin_arena_textured(&mut list, &resident, &[bad]).is_err());
        assert_eq!(list.items.len(), 1, "invalid textured bind pushes nothing");
        assert!(
            list.items
                .iter()
                .any(|item| item.mesh == twin_arena_cube_key()),
            "cube stays when the textured kit bind fails"
        );
    }

    #[test]
    fn twin_arena_groups_one_mesh_256_is_one() {
        let one = arena_pair(0x4717_0001, 0x11);
        let draws: [(MeshKey, PbrMaterial); 256] = std::array::from_fn(|_| one.clone());
        assert_eq!(twin_arena_group_count(&draws), 1);
        assert_eq!(
            twin_arena_group_count(&draws),
            twin_arena_group_count(&draws)
        );
    }

    #[test]
    fn twin_arena_groups_two_meshes_is_two_and_empty_is_zero() {
        let draws = [arena_pair(0x4717_0001, 0x11), arena_pair(0x4717_0002, 0x22)];
        assert_eq!(twin_arena_group_count(&draws), 2);
        let empty: [(MeshKey, PbrMaterial); 0] = [];
        assert_eq!(twin_arena_group_count(&empty), 0);
    }

    #[test]
    fn twin_arena_groups_stable_under_permutation() {
        let a = arena_pair(0x4717_0001, 0x11);
        let b = arena_pair(0x4717_0002, 0x22);
        let c = arena_pair(0x4717_0003, 0x33);
        let fwd = [a.clone(), b.clone(), c.clone(), a.clone(), b.clone()];
        let rev = [b.clone(), a.clone(), c.clone(), b.clone(), a.clone()];
        assert_eq!(twin_arena_group_count(&fwd), 3);
        assert_eq!(twin_arena_group_count(&fwd), twin_arena_group_count(&rev));
    }

    #[test]
    fn twin_arena_groups_single_is_one() {
        let one = arena_pair(0x4717_0001, 0x11);
        let draws = [one.clone()];
        assert_eq!(twin_arena_group_count(&draws), 1);
        assert_eq!(
            twin_arena_group_count(&draws),
            twin_arena_group_count(&draws)
        );
    }

    #[test]
    fn twin_arena_groups_textured_two_meshes_is_two_and_empty_is_zero() {
        let a = arena_textured_pair(0x4717_0001, 0x11, 0xB001, 0xA1);
        let b = arena_textured_pair(0x4717_0002, 0x22, 0xB002, 0xA2);
        let draws = [a.clone(), b.clone()];
        assert_eq!(twin_arena_group_count_textured(&draws), 2);
        let empty: [(MeshKey, PbrMaterial, Option<TextureKey>); 0] = [];
        assert_eq!(twin_arena_group_count_textured(&empty), 0);
        let one = arena_textured_pair(0x4717_0001, 0x11, 0xB001, 0xA1);
        let many: [(MeshKey, PbrMaterial, Option<TextureKey>); 256] =
            std::array::from_fn(|_| one.clone());
        assert_eq!(twin_arena_group_count_textured(&many), 1);
        let cube = (twin_arena_cube_key(), one.1.clone(), None);
        let polluted = [cube, a.clone(), b.clone()];
        assert_eq!(twin_arena_group_count_textured(&polluted), 2);
        let fwd = [a.clone(), b.clone(), a.clone()];
        let rev = [b.clone(), a.clone(), b.clone()];
        assert_eq!(
            twin_arena_group_count_textured(&fwd),
            twin_arena_group_count_textured(&rev)
        );
        let pairs = [(a.0, a.1.clone()), (b.0, b.1.clone())];
        assert_eq!(
            twin_arena_group_count_textured(&draws),
            twin_arena_group_count(&pairs)
        );
        assert_eq!(
            twin_arena_group_count_textured(&draws),
            twin_arena_group_count_textured(&draws)
        );
    }

    fn arena_textured_pair(
        id: u64,
        digest: u8,
        tex_id: u64,
        tex_digest: u8,
    ) -> (MeshKey, PbrMaterial, Option<TextureKey>) {
        (
            MeshKey::new(id, [digest; 32]),
            PbrMaterial {
                base_color: [0.5, 0.5, 0.5],
                metallic: 0.0,
                roughness: 0.9,
                exposure: 1.0,
            },
            Some(TextureKey::new(tex_id, [tex_digest; 32])),
        )
    }

    #[test]
    fn twin_arena_textured_bind_carries_base_color() {
        let mut list = empty_list_with_cube();
        let draws = [
            arena_textured_pair(0x4717_0001, 0x11, 0xB001, 0xA1),
            arena_textured_pair(0x4717_0002, 0x22, 0xB002, 0xA2),
        ];
        let resident = [twin_arena_cube_key(), draws[0].0, draws[1].0];
        let pushed =
            push_twin_arena_textured(&mut list, &resident, &draws).expect("textured kit binds");
        assert_eq!(pushed, 2);
        assert_eq!(list.items.len(), 2);
        assert!(
            !list
                .items
                .iter()
                .any(|item| item.mesh == twin_arena_cube_key()),
            "cube leaves when the kit lands"
        );
        assert_eq!(list.items[0].base_color, draws[0].2);
        assert_eq!(list.items[1].base_color, draws[1].2);
    }

    #[test]
    fn twin_arena_textured_none_draws_untextured() {
        let mut list = empty_list_with_cube();
        let key = MeshKey::new(0x4717_0001, [0x11; 32]);
        let material = PbrMaterial {
            base_color: [0.5, 0.5, 0.5],
            metallic: 0.0,
            roughness: 0.9,
            exposure: 1.0,
        };
        let draws = [(key, material, None)];
        let resident = [twin_arena_cube_key(), key];
        let pushed =
            push_twin_arena_textured(&mut list, &resident, &draws).expect("none lane binds");
        assert_eq!(pushed, 1);
        assert_eq!(list.items.len(), 1);
        assert_eq!(list.items[0].base_color, None);
    }

    #[test]
    fn twin_arena_rejects_over_budget_leaves_cube() {
        let mut list = empty_list_with_cube();
        let draws: Vec<(MeshKey, PbrMaterial)> = (0..257u64)
            .map(|i| {
                (
                    MeshKey::new(0x4717_0000 + i, [(i % 251) as u8; 32]),
                    PbrMaterial {
                        base_color: [0.5, 0.5, 0.5],
                        metallic: 0.0,
                        roughness: 0.9,
                        exposure: 1.0,
                    },
                )
            })
            .collect();
        let mut resident = Vec::with_capacity(258);
        resident.push(twin_arena_cube_key());
        for (key, _) in &draws {
            resident.push(*key);
        }
        assert!(push_twin_arena(&mut list, &resident, &draws).is_err());
        assert_eq!(list.items.len(), 1, "over-budget bind pushes nothing");
        assert!(
            list.items
                .iter()
                .any(|item| item.mesh == twin_arena_cube_key()),
            "cube stays when the kit would overflow the frame budget"
        );
    }

    #[test]
    fn twin_arena_textured_rejects_over_budget_leaves_cube() {
        let mut list = empty_list_with_cube();
        let draws: Vec<(MeshKey, PbrMaterial, Option<TextureKey>)> = (0..257u64)
            .map(|i| {
                (
                    MeshKey::new(0x4717_0000 + i, [(i % 251) as u8; 32]),
                    PbrMaterial {
                        base_color: [0.5, 0.5, 0.5],
                        metallic: 0.0,
                        roughness: 0.9,
                        exposure: 1.0,
                    },
                    None,
                )
            })
            .collect();
        let mut resident = Vec::with_capacity(258);
        resident.push(twin_arena_cube_key());
        for (key, _, _) in &draws {
            resident.push(*key);
        }
        assert!(push_twin_arena_textured(&mut list, &resident, &draws).is_err());
        assert_eq!(
            list.items.len(),
            1,
            "over-budget textured bind pushes nothing"
        );
        assert!(
            list.items
                .iter()
                .any(|item| item.mesh == twin_arena_cube_key()),
            "cube stays when the textured kit would overflow the frame budget"
        );
    }

    #[test]
    fn twin_arena_cube_in_kit_draws_never_returns() {
        // Cube pollution in the kit set never reintroduces the fallback:
        // the bind ignores the cube key so the cube still leaves when real
        // kit is resident. Fails before the cube filter lands.
        let cube_pollution = arena_pair(1, 0xA1);
        assert_eq!(
            cube_pollution.0,
            twin_arena_cube_key(),
            "pollution uses the fallback key"
        );
        let kit = arena_pair(0x4717_0001, 0x11);
        let draws = [cube_pollution.clone(), kit.clone()];
        let resident = [twin_arena_cube_key(), kit.0];
        let mut list = empty_list_with_cube();
        let pushed = push_twin_arena(&mut list, &resident, &draws).expect("kit binds");
        assert_eq!(pushed, 1, "cube pollution pushes nothing");
        assert!(
            !list
                .items
                .iter()
                .any(|item| item.mesh == twin_arena_cube_key()),
            "cube leaves even when kit data contains the cube"
        );
        assert_eq!(list.items.len(), 1);
        assert_eq!(list.items[0].mesh, kit.0);
        let mut textured_list = empty_list_with_cube();
        let textured_draws = [
            (cube_pollution.0, cube_pollution.1.clone(), None),
            (kit.0, kit.1.clone(), None),
        ];
        let textured_pushed =
            push_twin_arena_textured(&mut textured_list, &resident, &textured_draws)
                .expect("textured kit binds");
        assert_eq!(textured_pushed, 1, "textured cube pollution pushes nothing");
        assert!(
            !textured_list
                .items
                .iter()
                .any(|item| item.mesh == twin_arena_cube_key()),
            "textured cube leaves even when kit data contains the cube"
        );
        assert_eq!(
            twin_arena_group_count(&draws),
            1,
            "cube excluded from kit groups"
        );
    }

    #[test]
    fn twin_arena_resident_count_counts_only_resident_kit() {
        let kit_a = arena_pair(0x4717_0001, 0x11);
        let kit_b = arena_pair(0x4717_0002, 0x22);
        let kit_c = arena_pair(0x4717_0003, 0x33);
        let cube_pollution = arena_pair(1, 0xA1);
        assert_eq!(
            cube_pollution.0,
            twin_arena_cube_key(),
            "pollution uses the fallback key"
        );
        let ghost = arena_pair(0x4717_0009, 0x99);
        let draws = [
            kit_a.clone(),
            cube_pollution.clone(),
            kit_b.clone(),
            kit_c.clone(),
            ghost.clone(),
        ];
        let resident = [twin_arena_cube_key(), kit_a.0, kit_c.0];
        assert_eq!(
            twin_arena_resident_count(&resident, &draws),
            2,
            "only resident kit counts, cube and ghost excluded"
        );
        assert_eq!(
            twin_arena_resident_count(&resident, &draws),
            twin_arena_resident_count(&resident, &draws),
            "same inputs repeat"
        );
        let textured = [
            (kit_a.0, kit_a.1.clone(), None),
            (cube_pollution.0, cube_pollution.1.clone(), None),
            (kit_b.0, kit_b.1.clone(), None),
        ];
        assert_eq!(
            twin_arena_resident_count_textured(&resident, &textured),
            1,
            "textured counts resident kit only"
        );
    }

    #[test]
    fn twin_arena_second_push_is_noop() {
        // Double bind (frame_list plus the headless twin re-bind) must not
        // duplicate kit draws: the second push with the same kit is a no-op.
        // Fails before the idempotency guard lands (second push returns 2).
        let mut list = empty_list_with_cube();
        let draws = [arena_pair(0x4717_0001, 0x11), arena_pair(0x4717_0002, 0x22)];
        let resident = [twin_arena_cube_key(), draws[0].0, draws[1].0];
        let first = push_twin_arena(&mut list, &resident, &draws).expect("first kit bind");
        assert_eq!(first, 2);
        assert_eq!(list.items.len(), 2);
        let second = push_twin_arena(&mut list, &resident, &draws).expect("second bind is noop");
        assert_eq!(second, 0, "same kit pushed twice draws once");
        assert_eq!(list.items.len(), 2, "re-bind pushes nothing");
        assert!(
            !list
                .items
                .iter()
                .any(|item| item.mesh == twin_arena_cube_key()),
            "cube stays out after the re-bind"
        );
    }

    #[test]
    fn twin_arena_textured_second_push_is_noop() {
        // Same no-duplicate contract for the textured path the live
        // frame_list plus twin_headless_capture both call. Fails before.
        let mut list = empty_list_with_cube();
        let draws = [
            arena_textured_pair(0x4717_0001, 0x11, 0xB001, 0xA1),
            arena_textured_pair(0x4717_0002, 0x22, 0xB002, 0xA2),
        ];
        let resident = [twin_arena_cube_key(), draws[0].0, draws[1].0];
        let first =
            push_twin_arena_textured(&mut list, &resident, &draws).expect("first textured bind");
        assert_eq!(first, 2);
        assert_eq!(list.items.len(), 2);
        let second = push_twin_arena_textured(&mut list, &resident, &draws)
            .expect("second textured bind is noop");
        assert_eq!(second, 0, "same textured kit pushed twice draws once");
        assert_eq!(list.items.len(), 2, "textured re-bind pushes nothing");
    }
}
