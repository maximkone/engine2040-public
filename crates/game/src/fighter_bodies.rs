//! fighter bodies: one box-built body per fighter slot, its upload, and the
//! per-frame dots and draws over the match the caller steps.
//!
//! Consumer: the native window (`bin/arena.rs`), the staged frame receipt and
//! the browser page (`crates/web/src/fight.rs`), so the mapping from a running
//! [`App`](crate::App) to drawable bodies lives here once. Data owner: the body
//! stance, the mesh key base and the per-slot material; match authority stays in
//! `engine-arena`. Units: meters from the feet, 60 Hz frame dots, mesh keys as
//! u64 ids plus a digest. Determinism scope: the same dots build the same mesh
//! and the same draw order twice (no wall clock, no RNG). Errors: typed
//! [`BodyError`]; a body that cannot build or draw is refused loudly, never
//! panics, and one bad body never blanks the frame.
//!
//! Budget: the mesh builds once per body at session load (heap per call, never
//! per frame); the per-frame path allocates one Vec for the dots and nothing
//! else. App and page legs only: it draws through the presenter, which the
//! std-only headless leg never compiles, and the page must not need the
//! window's cook or device audio to share it.

use engine_audio::FightCue;
use engine_input::FighterId;
use engine_render::{
    present::{DrawItem, DrawList, MeshKey, Presenter},
    v1::{self, MeshData, PbrMaterial},
};

/// Mesh id base for staged fighter bodies (never collides with the central
/// cube key or the dressing base).
pub const FIGHT_MESH_ID_BASE: u64 = 0xF167_7400;
/// Fighter body size in meters `[sx, sy, sz]`: the box the stance in
/// [`BODY_BOXES`] fits inside, fists and feet included. The box stands on
/// the floor: its width and depth are measured around the feet, and its
/// height runs up from them.
pub const FIGHT_BODY_SIZE_M: [f32; 3] = [0.9, 1.8, 0.9];

/// A body mesh, an upload or a body draw refused: message carried. Displayed
/// and filed by the caller's own error type; this module never panics.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct BodyError(String);

impl BodyError {
    /// One refused body step, message carried verbatim.
    #[must_use]
    pub fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }

    /// The carried message (plain words, no prefix).
    #[must_use]
    pub fn message(&self) -> &str {
        &self.0
    }
}

impl core::fmt::Display for BodyError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for BodyError {}

/// One fighter's frame state: sim position plus health plus hit flash.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct StagedFighterDot {
    /// Fighter slot (`0..STAGED_FIGHT_FIGHTERS`).
    pub slot: u8,
    /// Feet position `[x, y, z]` in meters from the authoritative dataset.
    pub pos_m: [f32; 3],
    /// Health in permille (`0` reads as knocked down, drawn lying).
    pub hp: u16,
    /// True when this fighter took a hit or knockdown on this frame's tick.
    pub hit: bool,
}

/// Stable mesh key per fighter slot. Never collides with the central cube
/// key or the dressing keys: a distinct base id plus the slot in the digest.
#[must_use]
pub fn fight_mesh_key(slot: u8) -> MeshKey {
    MeshKey::new(FIGHT_MESH_ID_BASE + u64::from(slot), [0xF1 ^ slot; 32])
}

/// The fighter proxy's parts: `[min_x, min_y, min_z, max_x, max_y, max_z]` in
/// meters from the feet, plus that box's face subdivision. A standing guard:
/// legs apart, hips, torso and head up, both arms forward with fists high. The
/// silhouette reads as a person from any side at fight distance; one stretched
/// box never did.
const BODY_BOXES: [([f32; 6], u16); 13] = [
    // hips
    ([-0.17, 0.86, -0.12, 0.17, 1.08, 0.12], 6),
    // torso
    ([-0.23, 1.08, -0.14, 0.23, 1.50, 0.16], 8),
    // head
    ([-0.12, 1.53, -0.10, 0.12, 1.77, 0.14], 8),
    // left leg forward, right leg back
    ([-0.26, 0.00, 0.02, -0.08, 0.88, 0.22], 6),
    ([0.08, 0.00, -0.20, 0.26, 0.88, 0.00], 6),
    // upper arms hanging from the shoulders
    ([-0.34, 1.18, -0.10, -0.20, 1.46, 0.10], 6),
    ([0.20, 1.18, -0.10, 0.34, 1.46, 0.10], 6),
    // forearms held forward
    ([-0.36, 1.16, 0.06, -0.22, 1.30, 0.34], 6),
    ([0.22, 1.16, 0.06, 0.36, 1.30, 0.34], 6),
    // fists
    ([-0.38, 1.14, 0.30, -0.20, 1.32, 0.44], 6),
    ([0.20, 1.14, 0.30, 0.38, 1.32, 0.44], 6),
    // feet
    ([-0.28, 0.00, 0.06, -0.06, 0.10, 0.34], 6),
    ([0.06, 0.00, -0.30, 0.28, 0.10, -0.02], 6),
];
/// Total vertices over [`BODY_BOXES`]: 2 boxes at 8 subdivisions and 11 at 6
/// (6 faces times `(seg + 1)` squared each).
pub const BODY_VERTS: usize = 4_206;
/// Total indices over [`BODY_BOXES`]: 12 times `seg` squared triangles per box
/// times 3.
pub const BODY_INDICES: usize = 18_864;

/// The six box faces as `(normal, along, up)`, with `along x up = normal`: a
/// grid built with `u` along `along` and `v` along `up` then runs
/// counter-clockwise seen from outside, the winding the forward pipeline
/// keeps. The arena's own box builder owns the same rule.
const BODY_FACES: [([f32; 3], [f32; 3], [f32; 3]); 6] = [
    ([1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]),
    ([-1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, 1.0, 0.0]),
    ([0.0, 1.0, 0.0], [0.0, 0.0, 1.0], [1.0, 0.0, 0.0]),
    ([0.0, -1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]),
    ([0.0, 0.0, 1.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
    ([0.0, 0.0, -1.0], [0.0, 1.0, 0.0], [1.0, 0.0, 0.0]),
];

/// Push one axis-aligned subdivided box with outward face normals and
/// `0..=1` UVs. Faces are emitted in `+X -X +Y -Y +Z -Z` order with a
/// `(seg + 1)` squared grid each, wound counter-clockwise seen from outside:
/// the forward pipeline culls back faces, so a face wound the other way is
/// dropped and the box vanishes from that side. Pure, headless std-only.
/// Out-of-range vertex counts are typed, never panics.
fn push_subdivided_box(
    min_m: [f32; 3],
    max_m: [f32; 3],
    seg: u16,
    positions: &mut Vec<[f32; 3]>,
    normals: &mut Vec<[f32; 3]>,
    uvs: &mut Vec<[f32; 2]>,
    indices: &mut Vec<u32>,
) -> Result<(), BodyError> {
    let seg_f = f32::from(seg);
    let stride = u32::from(seg) + 1;
    for (normal, along, up) in BODY_FACES {
        let base = u32::try_from(positions.len())
            .map_err(|_| BodyError::new("staged body verts out of range"))?;
        for iy in 0..=seg {
            let v = f32::from(iy) / seg_f;
            for ix in 0..=seg {
                let u = f32::from(ix) / seg_f;
                positions.push(face_point(min_m, max_m, normal, along, up, u, v));
                normals.push(normal);
                uvs.push([u, v]);
            }
        }
        for iy in 0..seg {
            for ix in 0..seg {
                let a = base + u32::from(iy) * stride + u32::from(ix);
                let b = a + 1;
                let c = a + stride;
                let d = c + 1;
                indices.extend_from_slice(&[a, b, d, a, d, c]);
            }
        }
    }
    Ok(())
}

/// One point of a box face: the normal axis holds the face plane, `along`
/// carries the `u` span and `up` the `v` span.
fn face_point(
    min_m: [f32; 3],
    max_m: [f32; 3],
    normal: [f32; 3],
    along: [f32; 3],
    up: [f32; 3],
    u: f32,
    v: f32,
) -> [f32; 3] {
    std::array::from_fn(|axis| {
        let span = max_m[axis] - min_m[axis];
        if normal[axis] > 0.0 {
            max_m[axis]
        } else if normal[axis] < 0.0 {
            min_m[axis]
        } else if along[axis] > 0.0 {
            min_m[axis] + span * u
        } else if along[axis] < 0.0 {
            max_m[axis] - span * u
        } else if up[axis] > 0.0 {
            min_m[axis] + span * v
        } else {
            max_m[axis] - span * v
        }
    })
}

/// A staged fighter body: the [`BODY_BOXES`] stance welded into one mesh with
/// feet at `feet_m`, inside the [`FIGHT_BODY_SIZE_M`] volume, 6,288 triangles.
/// Deterministic subdivided grids (no wall clock, no RNG); headless std-only.
/// Validation failures are typed, never panics.
pub fn fight_body_mesh(feet_m: [f32; 3]) -> Result<MeshData, BodyError> {
    if !feet_m.iter().all(|v| v.is_finite()) {
        return Err(BodyError::new("fight body position is not finite"));
    }
    let mut positions = Vec::with_capacity(BODY_VERTS);
    let mut normals = Vec::with_capacity(BODY_VERTS);
    let mut uvs = Vec::with_capacity(BODY_VERTS);
    let mut indices = Vec::with_capacity(BODY_INDICES);
    for (part, seg) in BODY_BOXES {
        let min = [
            feet_m[0] + part[0],
            feet_m[1] + part[1],
            feet_m[2] + part[2],
        ];
        let max = [
            feet_m[0] + part[3],
            feet_m[1] + part[4],
            feet_m[2] + part[5],
        ];
        push_subdivided_box(
            min,
            max,
            seg,
            &mut positions,
            &mut normals,
            &mut uvs,
            &mut indices,
        )?;
    }
    v1::upload_mesh(&positions, &normals, &uvs, &indices)
        .map_err(|err| BodyError::new(format!("fight body mesh: {err}")))
}

/// Body material per slot: hero red, then blue/green/gold opponents, all
/// rough dielectrics. A hit tick flashes white so hits read at a glance.
/// Always valid.
#[must_use]
pub fn fight_body_material(slot: u8, hit: bool) -> PbrMaterial {
    if hit {
        return PbrMaterial {
            base_color: [1.0, 1.0, 1.0],
            metallic: 0.0,
            roughness: 0.5,
            exposure: 1.0,
        };
    }
    let base_color = match slot {
        0 => [0.75, 0.15, 0.12],
        1 => [0.15, 0.45, 0.85],
        2 => [0.15, 0.65, 0.30],
        _ => [0.85, 0.70, 0.15],
    };
    PbrMaterial {
        base_color,
        metallic: 0.0,
        roughness: 0.7,
        exposure: 1.0,
    }
}

/// Upload every staged body mesh into the twin. Cold init path only (heap per
/// mesh here, zero per frame later). A refused GPU upload skips that body
/// (like the central cube) so one bad body never blanks the frame. Returns
/// staged bodies uploaded.
pub fn stage_fight_bodies(
    presenter: &mut Presenter,
    dots: &[StagedFighterDot],
) -> Result<usize, BodyError> {
    let mut uploaded = 0usize;
    for dot in dots {
        let key = fight_mesh_key(dot.slot);
        let mesh = fight_body_mesh(dot.pos_m)?;
        if presenter.upload_mesh(key, &mesh).is_ok() {
            uploaded += 1;
        }
    }
    Ok(uploaded)
}

/// Push one draw per resident fighter body with its hit-flash material.
/// Missing keys draw nothing, so the chart probe without staging stays
/// clear-only. Returns draws pushed.
pub fn push_fight_draws(
    list: &mut DrawList,
    resident: &[MeshKey],
    dots: &[StagedFighterDot],
) -> Result<usize, BodyError> {
    let mut pushed = 0usize;
    for dot in dots {
        let key = fight_mesh_key(dot.slot);
        if resident.contains(&key) {
            let material = fight_body_material(dot.slot, dot.hit);
            material
                .validate()
                .map_err(|err| BodyError::new(format!("fight body material: {err}")))?;
            list.items.push(DrawItem {
                mesh: key,
                material,
                base_color: None,
                normal_map: None,
                orm: None,
                emissive: None,
            });
            pushed += 1;
        }
    }
    Ok(pushed)
}

/// Live fight bodies: one dot per simulated fighter from the match's own fighter views (feet
/// position on the floor, live hp, this tick's flash).
///
/// Caller: the native window (`bin/arena.rs`) and the browser shell
/// (`crates/web/src/fight.rs`) each draw the match they step, so the mapping
/// from a running `App` to drawable dots lives here once.
#[must_use]
pub fn live_fight_dots(app: &crate::App, cues: &[FightCue]) -> Vec<StagedFighterDot> {
    let views = app.fighter_views();
    let mut dots = Vec::with_capacity(views.len());
    for view in views {
        let Some(id) = FighterId::new(view.slot) else {
            continue;
        };
        if !view.pos_m.iter().all(|v| v.is_finite()) {
            continue;
        }
        let hit = cues.iter().any(|cue| match cue {
            FightCue::Hit { victim, .. } => *victim == id.get(),
            FightCue::Knockdown { victim, .. } => *victim == id.get(),
            _ => false,
        });
        dots.push(StagedFighterDot {
            slot: id.get(),
            pos_m: [view.pos_m[0], 0.0, view.pos_m[2]],
            hp: app
                .hp()
                .get(usize::from(view.slot))
                .copied()
                .unwrap_or(1000),
            hit,
        });
    }
    dots
}

// The window files a refused body step as its own render error, like every
// other staging step; the page keeps the message and prints it.
#[cfg(feature = "app")]
impl From<BodyError> for crate::windowed::WinError {
    fn from(err: BodyError) -> Self {
        Self::Render(err.message().to_string())
    }
}
