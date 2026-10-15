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

use crate::present::{DrawItem, DrawList, MeshKey};
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

/// Synthetic central cube key the kit replaces when resident.
/// Units: none (identity); determinism: same bytes every call.
#[must_use]
pub fn twin_arena_cube_key() -> MeshKey {
    MeshKey::new(1, [0xA1; 32])
}

/// Swap the synthetic cube for resident arena kit draws.
///
/// Takes the per-frame draw list (which holds the cube fallback), the
/// resident mesh keys from the presenter, plus the staged arena draws in
/// placement order as `(MeshKey, PbrMaterial)` pairs. When at least one arena
/// draw is resident, removes the cube item and pushes every resident arena
/// draw in order; otherwise leaves the list alone so the cube stays.
/// Returns draws pushed. Invalid materials fail loudly; missing keys draw
/// nothing. Determinism: same resident set plus pairs give the same list.
pub fn push_twin_arena(
    list: &mut DrawList,
    resident: &[MeshKey],
    draws: &[(MeshKey, PbrMaterial)],
) -> Result<usize, TwinArenaError> {
    let any_resident = draws.iter().any(|(key, _)| resident.contains(key));
    if !any_resident {
        return Ok(0);
    }
    let cube = twin_arena_cube_key();
    list.items.retain(|item| item.mesh != cube);
    let mut pushed = 0usize;
    for (key, material) in draws {
        if !resident.contains(key) {
            continue;
        }
        material
            .validate()
            .map_err(|err| TwinArenaError::Invalid(format!("twin arena material: {err}")))?;
        list.items.push(DrawItem {
            mesh: *key,
            material: material.clone(),
            base_color: None,
        });
        pushed += 1;
    }
    Ok(pushed)
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
}
