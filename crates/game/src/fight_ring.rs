//! fight_ring: the sand pit and stone tiers the fighters stand in.
//!
//! Consumer: [`crate::fighter_look`] (window frames and their captures).
//! Data owner: this module owns the ring geometry, sized from the match
//! half-extent so the drawn walls sit where the match clamps the fighters.
//! Textures are the CC0 arena albedo sets under `content/arena/textures`
//! (provenance in `SAND-PROVENANCE.md` / `STONE-PROVENANCE.md`).
//! Units: meters, unit normals, per-tile UVs in `0..=1` (the presenter
//! sampler clamps, so every tile carries its own full UV square).
//! Determinism: presentation only; same half-extent gives the same meshes.
//! Errors: a missing or undecodable texture or a refused upload is loud.
//! Budget: cold build (one floor mesh, one tier mesh, two textures); per
//! frame two draws.

use std::path::Path;

use engine_render::present::{DrawItem, DrawList, MeshKey, Presenter, TextureKey, TextureUpload};
use engine_render::v1::{self, MeshData, PbrMaterial};

/// Floor mesh key (clear of every other staged key).
pub const RING_FLOOR_KEY_ID: u64 = 0xF16D_0001;
/// Wall plus tiers mesh key.
pub const RING_TIERS_KEY_ID: u64 = 0xF16D_0002;
/// Texture ids for the sand and stone albedo maps.
const RING_SAND_TEX_ID: u64 = 0xF16D_0101;
const RING_STONE_TEX_ID: u64 = 0xF16D_0102;
/// Meters between the match clamp and the wall face.
const WALL_GAP_M: f32 = 1.0;
/// Pit wall height in meters.
const WALL_HEIGHT_M: f32 = 3.0;
/// Seating tiers above the wall: count, rise and run in meters.
const TIERS: usize = 4;
const TIER_RISE_M: f32 = 0.9;
const TIER_RUN_M: f32 = 1.4;
/// Target tile edge in meters (floor and wall texture repeat).
const TILE_M: f32 = 4.0;
/// Half wall span of the fixed ring-floor view row in meters: the duel
/// default half-extent (20 m, `ARENA_HALF_EXTENT_M`) plus [`WALL_GAP_M`].
const RING_FLOOR_WALL_M: f32 = 21.0;

/// One view row for the ring floor draw: kind plus fixed bounds.
///
/// Pure reader: fixed bounds only, no presenter, no file IO. View only;
/// feeds the brief text via `crate::view_truth::prop_row`, never the sim
/// hash. Units: meters. Determinism: same call, same bytes. Budget: one
/// flat row per frame, built on demand for the brief.
///
/// Consumer: the `--view-brief` prop list.
#[must_use]
pub fn ring_prop_row() -> (&'static str, [[f32; 3]; 2]) {
    let wall = RING_FLOOR_WALL_M;
    ("ring_floor", [[-wall, 0.0, -wall], [wall, 0.0, wall]])
}

/// Ring build or stage failure; the text names the file or mesh.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum RingError {
    /// Half-extent outside the accepted range.
    Extent,
    /// Generated geometry failed mesh validation.
    Mesh(String),
    /// A texture file is missing or does not decode.
    Texture(String),
    /// The presenter refused a mesh or texture upload.
    Upload(String),
}

impl core::fmt::Display for RingError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Extent => write!(f, "ring half extent out of range"),
            Self::Mesh(msg) => write!(f, "ring mesh: {msg}"),
            Self::Texture(msg) => write!(f, "ring texture: {msg}"),
            Self::Upload(msg) => write!(f, "ring upload: {msg}"),
        }
    }
}

impl std::error::Error for RingError {}

/// The ring: floor plus tiers meshes, their textures and materials.
pub struct FightRing {
    floor: MeshData,
    tiers: MeshData,
    sand: (TextureKey, TextureUpload),
    stone: (TextureKey, TextureUpload),
    /// Inner wall face distance from the center (meters).
    pub wall_m: f32,
}

/// Quad builder: four corners in order, one normal, one full UV square.
#[derive(Default)]
struct Builder {
    positions: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
    uvs: Vec<[f32; 2]>,
    indices: Vec<u32>,
}

impl Builder {
    /// Quad `a b c d` (counter-clockwise seen from the normal side).
    fn quad(&mut self, corners: [[f32; 3]; 4], normal: [f32; 3], uv_max: [f32; 2]) {
        let base = u32::try_from(self.positions.len()).unwrap_or(u32::MAX - 4);
        self.positions.extend_from_slice(&corners);
        self.normals.extend_from_slice(&[normal; 4]);
        self.uvs.extend_from_slice(&[
            [0.0, uv_max[1]],
            [uv_max[0], uv_max[1]],
            [uv_max[0], 0.0],
            [0.0, 0.0],
        ]);
        self.indices
            .extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }

    fn finish(self, what: &str) -> Result<MeshData, RingError> {
        v1::upload_mesh(&self.positions, &self.normals, &self.uvs, &self.indices)
            .map_err(|err| RingError::Mesh(format!("{what}: {err}")))
    }
}

/// Exact small count as `f32` (tile counts stay far below `u16::MAX`).
fn count_f32(n: usize) -> f32 {
    f32::from(u16::try_from(n).unwrap_or(u16::MAX))
}

/// Split `0..len` into whole tiles near [`TILE_M`]; returns (count, size).
fn tiles(len: f32) -> (usize, f32) {
    let mut count = 1usize;
    while count_f32(count) * TILE_M < len - 0.5 * TILE_M && count < 64 {
        count += 1;
    }
    (count, len / count_f32(count))
}

/// Sand floor: a square of tiles out to the wall face, at `y = 0`.
fn floor_mesh(wall_m: f32) -> Result<MeshData, RingError> {
    let mut b = Builder::default();
    let (count, size) = tiles(2.0 * wall_m);
    for i in 0..count {
        for j in 0..count {
            let x0 = -wall_m + count_f32(i) * size;
            let z0 = -wall_m + count_f32(j) * size;
            let (x1, z1) = (x0 + size, z0 + size);
            b.quad(
                [[x0, 0.0, z1], [x1, 0.0, z1], [x1, 0.0, z0], [x0, 0.0, z0]],
                [0.0, 1.0, 0.0],
                [1.0, 1.0],
            );
        }
    }
    b.finish("floor")
}

/// One side of the pit, seen from the center: `along` runs the side,
/// `inward` points back at the center. Wall face, then stepped tiers.
fn side(b: &mut Builder, wall_m: f32, along: [f32; 2], inward: [f32; 2]) {
    // Point on this side at distance `d` from the center and offset `t`
    // along the side, height `y`.
    let at = |d: f32, t: f32, y: f32| {
        [
            -inward[0] * d + along[0] * t,
            y,
            -inward[1] * d + along[1] * t,
        ]
    };
    let face = [inward[0], 0.0, inward[1]];
    let mut depth = wall_m;
    let mut top = 0.0f32;
    for step in 0..=TIERS {
        let rise = if step == 0 {
            WALL_HEIGHT_M
        } else {
            TIER_RISE_M
        };
        let half = depth + TIER_RUN_M;
        let (count, size) = tiles(2.0 * half);
        let v = (rise / size).clamp(0.05, 1.0);
        for k in 0..count {
            let t0 = -half + count_f32(k) * size;
            let t1 = t0 + size;
            // Riser facing the center.
            b.quad(
                [
                    at(depth, t0, top),
                    at(depth, t1, top),
                    at(depth, t1, top + rise),
                    at(depth, t0, top + rise),
                ],
                face,
                [1.0, v],
            );
            // Tread on top, running back to the next riser.
            b.quad(
                [
                    at(depth, t0, top + rise),
                    at(depth, t1, top + rise),
                    at(depth + TIER_RUN_M, t1, top + rise),
                    at(depth + TIER_RUN_M, t0, top + rise),
                ],
                [0.0, 1.0, 0.0],
                [1.0, (TIER_RUN_M / size).clamp(0.05, 1.0)],
            );
        }
        top += rise;
        depth += TIER_RUN_M;
    }
}

/// Stone wall plus seating tiers on all four sides.
fn tiers_mesh(wall_m: f32) -> Result<MeshData, RingError> {
    let mut b = Builder::default();
    side(&mut b, wall_m, [1.0, 0.0], [0.0, 1.0]);
    side(&mut b, wall_m, [-1.0, 0.0], [0.0, -1.0]);
    side(&mut b, wall_m, [0.0, -1.0], [1.0, 0.0]);
    side(&mut b, wall_m, [0.0, 1.0], [-1.0, 0.0]);
    b.finish("tiers")
}

fn texture(dir: &Path, file: &str, id: u64) -> Result<(TextureKey, TextureUpload), RingError> {
    let bytes = std::fs::read(dir.join(file))
        .map_err(|err| RingError::Texture(format!("{file}: {err}")))?;
    let upload = TextureUpload::from_encoded(&bytes)
        .map_err(|err| RingError::Texture(format!("{file}: {err}")))?;
    let mut digest = [0u8; 32];
    let hash = engine_core::hash_bytes(&bytes).to_le_bytes();
    digest[..8].copy_from_slice(&hash);
    Ok((TextureKey::new(id, digest), upload))
}

impl FightRing {
    /// Build the ring for a match clamped at `half_extent_m`, with albedo
    /// maps from `arena_dir/textures`. Loud on a bad extent or texture.
    pub fn load(arena_dir: &Path, half_extent_m: f32) -> Result<Self, RingError> {
        if !(1.0..=200.0).contains(&half_extent_m) {
            return Err(RingError::Extent);
        }
        let wall_m = half_extent_m + WALL_GAP_M;
        let dir = arena_dir.join("textures");
        Ok(Self {
            floor: floor_mesh(wall_m)?,
            tiers: tiers_mesh(wall_m)?,
            sand: texture(&dir, "sand_albedo_2k.jpg", RING_SAND_TEX_ID)?,
            stone: texture(&dir, "stone_albedo_2k.jpg", RING_STONE_TEX_ID)?,
            wall_m,
        })
    }

    fn keys() -> (MeshKey, MeshKey) {
        (
            MeshKey::new(RING_FLOOR_KEY_ID, [0xD1; 32]),
            MeshKey::new(RING_TIERS_KEY_ID, [0xD2; 32]),
        )
    }

    /// Upload both meshes and both textures. Cold path; loud on refusal.
    pub fn stage(&self, presenter: &mut Presenter) -> Result<(), RingError> {
        let (floor, tiers) = Self::keys();
        presenter
            .upload_mesh(floor, &self.floor)
            .map_err(|err| RingError::Upload(format!("floor: {err}")))?;
        presenter
            .upload_mesh(tiers, &self.tiers)
            .map_err(|err| RingError::Upload(format!("tiers: {err}")))?;
        for (key, upload) in [&self.sand, &self.stone] {
            presenter
                .upload_texture(*key, upload)
                .map_err(|err| RingError::Upload(format!("texture: {err}")))?;
        }
        Ok(())
    }

    /// Push the floor and tier draws when resident; returns draws pushed.
    pub fn push_draws(&self, list: &mut DrawList, resident: &[MeshKey]) -> usize {
        let (floor, tiers) = Self::keys();
        let mut pushed = 0usize;
        // The CC0 sand scan reads grey under the arena sun: a warm factor
        // pulls it toward packed arena sand; the sandstone keeps its color.
        for (key, texture, roughness, tint) in [
            (floor, self.sand.0, 0.95f32, [1.0, 0.84, 0.62]),
            (tiers, self.stone.0, 0.85f32, [1.0, 1.0, 1.0]),
        ] {
            if !resident.contains(&key) {
                continue;
            }
            list.items.push(DrawItem {
                mesh: key,
                material: PbrMaterial {
                    base_color: tint,
                    metallic: 0.0,
                    roughness,
                    exposure: 1.0,
                },
                base_color: Some(texture),
            });
            pushed += 1;
        }
        pushed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn floor_reaches_the_wall_and_tiles_stay_square() {
        let wall = 21.0;
        let floor = floor_mesh(wall).expect("floor builds");
        let max_x = floor
            .positions
            .iter()
            .map(|p| p[0])
            .fold(f32::MIN, f32::max);
        let min_z = floor
            .positions
            .iter()
            .map(|p| p[2])
            .fold(f32::MAX, f32::min);
        assert!((max_x - wall).abs() < 1e-4 && (min_z + wall).abs() < 1e-4);
        assert!(floor.normals.iter().all(|n| *n == [0.0, 1.0, 0.0]));
        let (count, size) = tiles(2.0 * wall);
        assert!(
            count > 1 && (size - TILE_M).abs() < 0.5 * TILE_M,
            "{count} x {size}"
        );
    }

    #[test]
    fn tiers_face_the_center_and_rise_outward() {
        let wall = 21.0;
        let tiers = tiers_mesh(wall).expect("tiers build");
        // Every riser normal points back toward the center.
        for (p, n) in tiers.positions.iter().zip(tiers.normals.iter()) {
            if n[1] == 0.0 {
                assert!(p[0] * n[0] + p[2] * n[2] < 0.0, "riser at {p:?} faces out");
            }
        }
        let top = tiers
            .positions
            .iter()
            .map(|p| p[1])
            .fold(f32::MIN, f32::max);
        let want = WALL_HEIGHT_M + TIER_RISE_M * count_f32(TIERS);
        assert!((top - want).abs() < 1e-4, "{top} vs {want}");
        // Nothing hangs inside the pit.
        assert!(tiers
            .positions
            .iter()
            .all(|p| p[0].abs().max(p[2].abs()) >= wall - 1e-3));
    }

    #[test]
    fn ring_loads_cc0_textures_and_refuses_bad_extents() {
        let arena =
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../content/arena");
        let ring = FightRing::load(&arena, 20.0).expect("ring loads");
        assert_eq!(ring.wall_m, 21.0);
        assert!(ring.sand.1.width >= 1024 && ring.stone.1.width >= 1024);
        assert!(FightRing::load(&arena, f32::NAN).is_err());
        assert!(FightRing::load(&arena.join("missing"), 20.0).is_err());
    }
}
