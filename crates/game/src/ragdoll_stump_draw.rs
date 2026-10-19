//! ragdoll stump draw: the cap that closes a ragdoll arm a go-down cue took (QR12).
//!
//! The look layer severs one arm of a floored fighter's ragdoll on the cue that names the
//! fall and reports the wound in world meters ([`RagdollStump`]). This module owns only the
//! picture of one cap: a short closed cylinder of the stump's own radius
//! ([`engine_render::stump::stump_cap`]), placed at the stump pose, uploaded under one mesh
//! key per slot and pushed into a [`DrawList`] with a dark-red material. A key the presenter
//! does not hold draws nothing, so a refused upload never fails the frame and a frame before
//! the first sever stays clear.
//!
//! Consumer: the `arena` window and the browser match page call [`stump_draws`] once per
//! frame beside the fighter draws. Data owner: `crates/game/src/ragdoll_look.rs` owns which
//! bone left and where the cap rides; this module owns only the cap mesh and its draw. Units:
//! meters, unit quaternions `[x, y, z, w]`, Y up. Determinism: presentation only; nothing
//! here is hashed and no value reaches the fixed tick, and the same stumps give the same
//! meshes and the same draws twice. Errors: [`WinError::Render`] from [`stump_cap_mesh`] on a
//! bad radius or pose; [`stump_draws`] and [`push_stump_caps`] skip such a cap rather than
//! fail the frame.

use engine_render::present::{DrawItem, DrawList, MeshKey, Presenter};
use engine_render::stump::{place_mesh, stump_cap};
use engine_render::v1::{upload_mesh_tiled, MeshData, PbrMaterial};

use crate::fighter_look::RagdollStump;
use crate::windowed::WinError;

/// Mesh id base for the ragdoll stump caps, clear of the fighter-part (`0xF16B_0000`), the
/// cape (`0xF16E_0000`) and the authority-limb (`0xF16D_0000`) keys.
pub const RAGDOLL_STUMP_MESH_ID_BASE: u64 = 0xF16F_0000;
/// Wall segments of one cap: the same count the authority-stump cap uses.
const CAP_SEGMENTS: u32 = 12;
/// Depth of one cap along its axis, meters: a stub that closes the hole, not a limb.
const CAP_DEPTH_M: f32 = 0.06;
/// Narrowest cap radius kept, meters: the cap builder's own floor.
const CAP_RADIUS_M_MIN: f32 = 0.005;
/// Widest cap radius kept, meters: the cap builder's own ceiling.
const CAP_RADIUS_M_MAX: f32 = 0.5;
/// Base color of the dried blood on a cut: a dark red, well under the cape's cloth red.
const CAP_COLOR: [f32; 3] = [0.30, 0.03, 0.03];
/// Roughness of that blood: matte, so a cap never reads as metal.
const CAP_ROUGHNESS: f32 = 0.8;

/// Mesh key of one slot's stump cap: `base + slot`, so a slot's cap replaces in place. The
/// key is fixed per slot, so a page never uploads a second key for one wound.
#[must_use]
pub fn stump_mesh_key(slot: u8) -> MeshKey {
    let mut digest = [0x52u8; 32];
    digest[0] = slot;
    MeshKey::new(RAGDOLL_STUMP_MESH_ID_BASE + u64::from(slot), digest)
}

/// The dark-red material every stump cap draws with. Always valid.
#[must_use]
pub fn stump_material() -> PbrMaterial {
    PbrMaterial {
        base_color: CAP_COLOR,
        metallic: 0.0,
        roughness: CAP_ROUGHNESS,
        exposure: 1.0,
    }
}

/// One closed cap mesh for `stump`, placed at its world pose. The radius is the stump's own,
/// clamped into the cap builder's table. Errors: a radius or pose the builder refuses.
pub fn stump_cap_mesh(stump: &RagdollStump) -> Result<MeshData, WinError> {
    let radius = stump.radius_m.clamp(CAP_RADIUS_M_MIN, CAP_RADIUS_M_MAX);
    let base = stump_cap(radius, CAP_DEPTH_M, CAP_SEGMENTS)
        .map_err(|err| WinError::Render(format!("ragdoll stump cap: {err}")))?;
    let placed = place_mesh(&base, stump.pos, stump.rot)
        .map_err(|err| WinError::Render(format!("ragdoll stump cap pose: {err}")))?;
    upload_mesh_tiled(
        &placed.positions,
        &placed.normals,
        &placed.uvs,
        &placed.indices,
    )
    .map_err(|err| WinError::Render(format!("ragdoll stump cap mesh: {err}")))
}

/// Push one dark-red cap draw per stump whose slot key is resident. A key the presenter does
/// not hold draws nothing, so a frame before the first sever (and a refused upload) stays
/// clear. Returns the draws pushed.
pub fn push_stump_caps(
    list: &mut DrawList,
    resident: &[MeshKey],
    stumps: &[RagdollStump],
) -> Result<usize, WinError> {
    let material = stump_material();
    material
        .validate()
        .map_err(|err| WinError::Render(format!("ragdoll stump material: {err}")))?;
    let mut pushed = 0usize;
    for stump in stumps {
        let key = stump_mesh_key(stump.slot);
        if !resident.contains(&key) {
            continue;
        }
        list.items.push(DrawItem {
            mesh: key,
            material: material.clone(),
            base_color: None,
            normal_map: None,
            orm: None,
            emissive: None,
        });
        pushed += 1;
    }
    Ok(pushed)
}

/// Upload one cap mesh per stump's slot and push one cap draw per stump: the one call a
/// window or page makes each frame beside the fighter draws. A cap the mesh builder or the
/// upload refuses is skipped, never fails the frame. Returns the cap draws pushed.
pub fn stump_draws(
    presenter: &mut Presenter,
    list: &mut DrawList,
    stumps: &[RagdollStump],
) -> Result<usize, WinError> {
    if stumps.is_empty() {
        return Ok(0);
    }
    for stump in stumps {
        let Ok(mesh) = stump_cap_mesh(stump) else {
            continue;
        };
        // A refused upload drops that cap's key, so the push below skips its draw.
        let _ = presenter.upload_mesh(stump_mesh_key(stump.slot), &mesh);
    }
    let resident = presenter.mesh_keys();
    push_stump_caps(list, &resident, stumps)
}
