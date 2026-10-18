//! fight_ring: the sand pit and stone tiers the fighters stand in.
//!
//! Consumer: [`crate::fighter_look`] (window frames and their captures).
//! Data owner: this module owns the ring geometry, sized from the match
//! half-extent so the drawn walls sit where the match clamps the fighters;
//! the GP36 zone classifier (`zone_at`, `ZoneTally`) is owned by
//! `engine-arena` and re-exported here so the ring's own zone paths stay
//! valid for the tournament metric.
//! Textures are the CC0 arena albedo sets under `content/arena/textures`
//! (provenance in `SAND-PROVENANCE.md` / `STONE-PROVENANCE.md`).
//! Units: meters, unit normals, per-tile UVs in `0..=1` (the presenter
//! sampler clamps, so every tile carries its own full UV square).
//! Determinism: presentation only; same half-extent gives the same meshes.
//! Errors: a missing or undecodable texture or a refused upload is loud.
//! Budget: cold build (one floor mesh, one tier mesh, one mesh per crowd
//! palette slot, two textures); per frame six draws.

use std::path::Path;

use engine_assets::bc::srgb_byte_to_linear;
use engine_assets::{AssetSource, FsSource};
use engine_render::draw::crowd_seats::{
    crowd_palette_index, crowd_palette_rgba, crowd_seat_pos, CROWD_BODY_D_M, CROWD_BODY_H_M,
    CROWD_BODY_W_M, CROWD_HEAD_M, CROWD_PALETTE_LEN, CROWD_ROWS_PER_SECTOR, CROWD_SEATS_PER_ROW,
    CROWD_SECTORS, CROWD_WALL_M,
};
use engine_render::present::{DrawItem, DrawList, MeshKey, Presenter, TextureKey, TextureUpload};
use engine_render::v1::{self, MeshData, PbrMaterial};

/// The GP36 zone classifier lives in `engine-arena` (one owner, shared with
/// the tournament metric); re-exported so every `fight_ring::` path and the
/// ring's own tests stay valid.
pub use engine_arena::zones::{zone_at, ArenaZone, ZoneTally, CENTRE_R_M, RING_R_M};

/// Classify one floor point into its zone under the shipped gate mouth: the
/// `gate_tunnel` row of `content/design/arena-zones.rows` read through
/// [`crate::arena_kit::kit_zone_rows`], so the ring's zone reading and the
/// design gate share one anchor; falls back to
/// [`engine_arena::zones::shipped_gate_anchor`] when that row is absent.
/// Units: meters. Determinism: same rows give the same zone.
///
/// Caller: `crates/game/src/bin/arena.rs` `zone_report_lines` names each
/// row's zone through this.
pub fn ring_zone_at(x_m: f32, z_m: f32) -> ArenaZone {
    let gate = crate::arena_kit::kit_zone_rows().ok().and_then(|rows| {
        rows.into_iter()
            .find(|row| row.id == crate::arena_kit::KIT_ZONE_GATE_ID)
    });
    let (mouth, half) = match gate {
        Some(row) => (
            [f32::from(row.cx_cm) / 100.0, f32::from(row.cz_cm) / 100.0],
            f32::from(row.radius_cm) / 100.0,
        ),
        None => {
            let [x, z, half] = engine_arena::zones::shipped_gate_anchor();
            ([x, z], half)
        }
    };
    zone_at(x_m, z_m, mouth, half)
}

/// Floor mesh key (clear of every other staged key).
pub const RING_FLOOR_KEY_ID: u64 = 0xF16D_0001;
/// Wall plus tiers mesh key.
pub const RING_TIERS_KEY_ID: u64 = 0xF16D_0002;
/// One crowd mesh key per palette slot (four), clear of the ring keys.
pub const RING_CROWD_KEY_IDS: [u64; CROWD_PALETTE_LEN] =
    [0xF16D_0201, 0xF16D_0202, 0xF16D_0203, 0xF16D_0204];
/// Gap between a spectator's shoulders and its head, in meters.
const CROWD_HEAD_GAP_M: f32 = 0.04;
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

/// One view row for the ring wall-plus-tiers draw: kind plus fixed bounds.
///
/// The tiers draw rises [`WALL_HEIGHT_M`] plus one [`TIER_RISE_M`] per tier
/// and runs out one [`TIER_RUN_M`] per side step (wall face plus tiers, so
/// [`TIERS`] + 1 runs), over the same fixed span as [`ring_prop_row`].
/// Same fixed-duel default (half-extent 20 m): a custom-extent ring keeps
/// its own mesh while this row names the duel default, exactly like the
/// floor row. Pure reader: fixed bounds only, no presenter, no file IO.
/// View only; feeds the brief text via `crate::view_truth::prop_row`, never
/// the sim hash. Units: meters. Determinism: same call, same bytes. Budget:
/// one flat row per frame, built on demand for the brief.
///
/// Caller: `crates/game/src/bin/arena.rs` `--view-brief` writer, which
/// pushes this beside [`ring_prop_row`] so every ring draw lists once.
#[must_use]
pub fn ring_wall_row() -> (&'static str, [[f32; 3]; 2]) {
    let span = RING_FLOOR_WALL_M + count_f32(TIERS + 1) * TIER_RUN_M;
    let top = WALL_HEIGHT_M + count_f32(TIERS) * TIER_RISE_M;
    ("ring_tiers", [[-span, 0.0, -span], [span, top, span]])
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

/// The ring: floor plus tiers meshes, the crowd on the tiers, and their
/// textures and materials.
pub struct FightRing {
    floor: MeshData,
    tiers: MeshData,
    /// Spectator boxes on the tiers, one mesh per palette slot.
    crowd: [MeshData; CROWD_PALETTE_LEN],
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

/// One spectator part as six outward quads (24 verts, 36 indices), `uv`
/// being the full texture square the stone albedo tiles at.
fn seat_box(b: &mut Builder, center: [f32; 3], half: [f32; 3], uv: f32) {
    let (cx, cy, cz) = (center[0], center[1], center[2]);
    let (hx, hy, hz) = (half[0], half[1], half[2]);
    let faces: [([f32; 3], [[f32; 3]; 4]); 6] = [
        (
            [1.0, 0.0, 0.0],
            [
                [cx + hx, cy - hy, cz - hz],
                [cx + hx, cy - hy, cz + hz],
                [cx + hx, cy + hy, cz + hz],
                [cx + hx, cy + hy, cz - hz],
            ],
        ),
        (
            [-1.0, 0.0, 0.0],
            [
                [cx - hx, cy - hy, cz + hz],
                [cx - hx, cy - hy, cz - hz],
                [cx - hx, cy + hy, cz - hz],
                [cx - hx, cy + hy, cz + hz],
            ],
        ),
        (
            [0.0, 1.0, 0.0],
            [
                [cx - hx, cy + hy, cz - hz],
                [cx - hx, cy + hy, cz + hz],
                [cx + hx, cy + hy, cz + hz],
                [cx + hx, cy + hy, cz - hz],
            ],
        ),
        (
            [0.0, -1.0, 0.0],
            [
                [cx - hx, cy - hy, cz + hz],
                [cx - hx, cy - hy, cz - hz],
                [cx + hx, cy - hy, cz - hz],
                [cx + hx, cy - hy, cz + hz],
            ],
        ),
        (
            [0.0, 0.0, 1.0],
            [
                [cx - hx, cy - hy, cz + hz],
                [cx + hx, cy - hy, cz + hz],
                [cx + hx, cy + hy, cz + hz],
                [cx - hx, cy + hy, cz + hz],
            ],
        ),
        (
            [0.0, 0.0, -1.0],
            [
                [cx + hx, cy - hy, cz - hz],
                [cx - hx, cy - hy, cz - hz],
                [cx - hx, cy + hy, cz - hz],
                [cx + hx, cy + hy, cz - hz],
            ],
        ),
    ];
    for (normal, corners) in faces {
        b.quad(corners, normal, [uv, uv]);
    }
}

/// Spectator body plus head boxes for one palette slot, on the ring treads.
///
/// The seat layout is owned by `engine_render::draw::crowd_seats` and its
/// wall, rise and run match this ring's exactly (tier `r` tread top
/// `CROWD_WALL_H_M + RISE * (r + 1)`, run `CROWD_WALL_M + RUN * (r + 1)`),
/// so every seat's feet land on a drawn tread rather than inside the stone.
/// A custom-extent ring shifts the whole layout outward by the wall
/// difference, so the seats ride its treads too. Presentation only, never
/// hashed; same inputs give the same mesh twice.
fn crowd_mesh(slot: usize, wall_m: f32) -> Result<MeshData, RingError> {
    let mut b = Builder::default();
    let push = wall_m - CROWD_WALL_M;
    for sector in 0..CROWD_SECTORS {
        // Seats of a north/south stand face along z, an east/west stand
        // along x, so the body box turns with its sector.
        let (half_x, half_z) = if sector < 2 {
            (CROWD_BODY_W_M, CROWD_BODY_D_M)
        } else {
            (CROWD_BODY_D_M, CROWD_BODY_W_M)
        };
        for row in 0..CROWD_ROWS_PER_SECTOR {
            for seat in 0..CROWD_SEATS_PER_ROW {
                if crowd_palette_index(sector, row, seat) != slot % CROWD_PALETTE_LEN {
                    continue;
                }
                let at = crowd_seat_pos(sector, row, seat);
                let (x, z) = match sector {
                    0 => (at[0], at[2] - push),
                    1 => (at[0], at[2] + push),
                    2 => (at[0] - push, at[2]),
                    _ => (at[0] + push, at[2]),
                };
                let body_center = [x, at[1] + CROWD_BODY_H_M * 0.5, z];
                seat_box(
                    &mut b,
                    body_center,
                    [half_x * 0.5, CROWD_BODY_H_M * 0.5, half_z * 0.5],
                    0.25,
                );
                let head_center = [
                    x,
                    at[1] + CROWD_BODY_H_M + CROWD_HEAD_GAP_M + CROWD_HEAD_M * 0.5,
                    z,
                ];
                seat_box(&mut b, head_center, [CROWD_HEAD_M * 0.5; 3], 0.25);
            }
        }
    }
    b.finish("crowd")
}

/// Linear base-color factors for one crowd palette slot: the shared palette
/// owns the sRGB bytes, the presenter multiplies them into the stone albedo.
fn crowd_tint(slot: usize) -> [f32; 3] {
    let [r, g, b, _] = crowd_palette_rgba(slot);
    [
        srgb_byte_to_linear(r),
        srgb_byte_to_linear(g),
        srgb_byte_to_linear(b),
    ]
}

fn texture(
    source: &dyn AssetSource,
    file: &str,
    id: u64,
) -> Result<(TextureKey, TextureUpload), RingError> {
    let bytes = source
        .read(&format!("textures/{file}"))
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
        Self::load_from(&FsSource::new(arena_dir), half_extent_m)
    }

    /// [`FightRing::load`] over any content source: the albedo maps are read
    /// as `textures/<name>`, so a browser build hands in a fetched bundle.
    pub fn load_from(source: &dyn AssetSource, half_extent_m: f32) -> Result<Self, RingError> {
        if !(1.0..=200.0).contains(&half_extent_m) {
            return Err(RingError::Extent);
        }
        let wall_m = half_extent_m + WALL_GAP_M;
        let mut crowd = Vec::with_capacity(CROWD_PALETTE_LEN);
        for slot in 0..CROWD_PALETTE_LEN {
            crowd.push(crowd_mesh(slot, wall_m)?);
        }
        let crowd: [MeshData; CROWD_PALETTE_LEN] = crowd
            .try_into()
            .map_err(|_| RingError::Mesh("crowd slot count".to_string()))?;
        Ok(Self {
            floor: floor_mesh(wall_m)?,
            tiers: tiers_mesh(wall_m)?,
            crowd,
            sand: texture(source, "sand_albedo_2k.jpg", RING_SAND_TEX_ID)?,
            stone: texture(source, "stone_albedo_2k.jpg", RING_STONE_TEX_ID)?,
            wall_m,
        })
    }

    fn keys() -> (MeshKey, MeshKey) {
        (
            MeshKey::new(RING_FLOOR_KEY_ID, [0xD1; 32]),
            MeshKey::new(RING_TIERS_KEY_ID, [0xD2; 32]),
        )
    }

    fn crowd_keys() -> [MeshKey; CROWD_PALETTE_LEN] {
        std::array::from_fn(|slot| MeshKey::new(RING_CROWD_KEY_IDS[slot], [0xD3; 32]))
    }

    /// Upload both ring meshes, the four crowd meshes and both textures.
    /// Cold path; loud on refusal.
    pub fn stage(&self, presenter: &mut Presenter) -> Result<(), RingError> {
        let (floor, tiers) = Self::keys();
        presenter
            .upload_mesh(floor, &self.floor)
            .map_err(|err| RingError::Upload(format!("floor: {err}")))?;
        presenter
            .upload_mesh(tiers, &self.tiers)
            .map_err(|err| RingError::Upload(format!("tiers: {err}")))?;
        for (key, mesh) in Self::crowd_keys().iter().zip(self.crowd.iter()) {
            presenter
                .upload_mesh(*key, mesh)
                .map_err(|err| RingError::Upload(format!("crowd: {err}")))?;
        }
        for (key, upload) in [&self.sand, &self.stone] {
            presenter
                .upload_texture(*key, upload)
                .map_err(|err| RingError::Upload(format!("texture: {err}")))?;
        }
        Ok(())
    }

    /// Push the floor, tier and crowd draws when resident; returns draws
    /// pushed. The crowd rides the same stone albedo under the shared
    /// palette tints, so the tiers read as spectators instead of bare stone.
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
                normal_map: None,
            });
            pushed += 1;
        }
        for (slot, key) in Self::crowd_keys().iter().enumerate() {
            if !resident.contains(key) {
                continue;
            }
            list.items.push(DrawItem {
                mesh: *key,
                material: PbrMaterial {
                    base_color: crowd_tint(slot),
                    metallic: 0.0,
                    roughness: 0.9,
                    exposure: 1.0,
                },
                base_color: Some(self.stone.0),
                normal_map: None,
            });
            pushed += 1;
        }
        pushed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_render::draw::crowd_seats::{crowd_total, CROWD_WALL_H_M};

    #[test]
    fn crowd_seats_stand_on_the_ring_treads() {
        // The layout's feet heights and depths must match this ring's own
        // tier rise and run, or the boxes float or sink into the stone.
        let mut seats = 0usize;
        for sector in 0..CROWD_SECTORS {
            for row in 0..CROWD_ROWS_PER_SECTOR {
                let at = crowd_seat_pos(sector, row, 0);
                let row_f = count_f32(row + 1);
                let want_top = WALL_HEIGHT_M + TIER_RISE_M * row_f;
                assert!(
                    (at[1] - want_top).abs() < 1e-4,
                    "row {row} feet {at:?} miss tread top {want_top}"
                );
                let inner = CROWD_WALL_M + TIER_RUN_M * row_f;
                let outer = inner + TIER_RUN_M;
                let depth = at[0].abs().max(at[2].abs());
                assert!(
                    depth > inner && depth < outer,
                    "row {row} depth {depth} leaves tread {inner}..{outer}"
                );
                assert_eq!(WALL_HEIGHT_M, CROWD_WALL_H_M, "wall tops agree");
                for seat in 0..CROWD_SEATS_PER_ROW {
                    seats += 1;
                    let _ = crowd_seat_pos(sector, row, seat);
                }
            }
        }
        assert_eq!(seats, crowd_total(), "every laid-out seat is drawn");
    }

    #[test]
    fn crowd_mesh_rises_out_of_the_pit_on_every_slot() {
        let wall = 21.0;
        let mut seats = 0usize;
        for slot in 0..CROWD_PALETTE_LEN {
            let mesh = crowd_mesh(slot, wall).expect("crowd builds");
            assert!(
                !mesh.positions.is_empty(),
                "palette slot {slot} seats no one"
            );
            // Two boxes (body plus head) per seat: 24 verts and 36 indices each.
            assert_eq!(mesh.positions.len() % 48, 0, "slot {slot} halves a seat");
            assert_eq!(mesh.indices.len(), mesh.positions.len() / 48 * 72);
            assert_eq!(mesh.uvs.len(), mesh.positions.len());
            for (p, n) in mesh.positions.iter().zip(mesh.normals.iter()) {
                assert!(
                    p[0].abs().max(p[2].abs()) >= wall - 1e-3,
                    "seat hangs inside the pit at {p:?}"
                );
                assert!(p[1] > WALL_HEIGHT_M, "seat sits below the wall top");
                assert!(n.iter().all(|c| c.is_finite()));
            }
            seats += mesh.positions.len() / 48;
        }
        assert_eq!(seats, crowd_total(), "the four slots hold all 96 seats");
        // A custom-extent ring keeps its seats on its own treads.
        let wider = crowd_mesh(0, 25.0).expect("wider ring crowd builds");
        let narrow = crowd_mesh(0, 21.0).expect("duel crowd builds");
        assert_eq!(wider.positions.len(), narrow.positions.len());
        assert!(
            wider
                .positions
                .iter()
                .map(|p| p[0].abs().max(p[2].abs()))
                .fold(f32::MAX, f32::min)
                > narrow
                    .positions
                    .iter()
                    .map(|p| p[0].abs().max(p[2].abs()))
                    .fold(f32::MAX, f32::min),
            "wider ring pushes the seats outward"
        );
        // Empty slots refuse instead of uploading nothing silently.
        assert!(crowd_mesh(CROWD_PALETTE_LEN, wall).is_ok());
    }

    #[test]
    fn crowd_tints_are_linear_and_distinct() {
        let slots: Vec<[f32; 3]> = (0..CROWD_PALETTE_LEN).map(crowd_tint).collect();
        for tint in &slots {
            assert!(
                tint.iter().all(|c| (0.0..=1.0).contains(c)),
                "tint {tint:?} escapes the linear range"
            );
        }
        assert!(slots.iter().any(|t| *t != slots[0]), "slots must differ");
        assert_eq!(srgb_byte_to_linear(0), 0.0);
        assert!((srgb_byte_to_linear(255) - 1.0).abs() < 1e-6);
    }

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
    fn tiers_row_holds_the_second_ring_draw() {
        // SP64: the ring draws twice per frame (floor plus wall/tiers) but
        // the brief row covered only the floor. The tiers row must hold
        // every wall/tiers vertex of the default duel ring, snugly.
        let (kind, [min, max]) = ring_wall_row();
        assert_eq!(kind, "ring_tiers");
        let tiers = tiers_mesh(RING_FLOOR_WALL_M).expect("tiers build");
        assert!(!tiers.positions.is_empty(), "tiers draw is real");
        for p in &tiers.positions {
            assert!(
                (0..3).all(|k| min[k] - 1e-3 <= p[k] && p[k] <= max[k] + 1e-3),
                "tiers vertex {p:?} outside {min:?}..{max:?}"
            );
        }
        let top = tiers
            .positions
            .iter()
            .map(|p| p[1])
            .fold(f32::MIN, f32::max);
        assert!(
            (max[1] - top).abs() < 1e-2,
            "row top {} hugs the mesh top {top}",
            max[1]
        );
        let span = tiers
            .positions
            .iter()
            .map(|p| p[0].abs().max(p[2].abs()))
            .fold(f32::MIN, f32::max);
        assert!(
            (max[0] - span).abs() < 1e-2 && (max[2] - span).abs() < 1e-2,
            "row span {} hugs the mesh span {span}",
            max[0]
        );
    }

    #[test]
    fn zones_follow_the_3_8_bands() {
        let gate = [50.0, 50.0];
        assert_eq!(zone_at(0.0, 0.0, gate, 1.5), ArenaZone::Centre);
        assert_eq!(zone_at(2.9, 0.0, gate, 1.5), ArenaZone::Centre);
        assert_eq!(zone_at(3.0, 0.0, gate, 1.5), ArenaZone::Ring);
        assert_eq!(zone_at(0.0, -5.0, gate, 1.5), ArenaZone::Ring);
        assert_eq!(zone_at(8.0, 0.0, gate, 1.5), ArenaZone::Ring);
        assert_eq!(zone_at(8.1, 0.0, gate, 1.5), ArenaZone::Wall);
        assert_eq!(zone_at(-15.0, 15.0, gate, 1.5), ArenaZone::Wall);
    }

    #[test]
    fn gate_mouth_wins_over_the_radial_band() {
        let gate = [5.0, 0.0];
        assert_eq!(zone_at(5.0, 0.0, gate, 1.5), ArenaZone::Gate);
        assert_eq!(zone_at(6.5, 1.5, gate, 1.5), ArenaZone::Gate);
        // Just past the mouth edge: radial band decides again.
        assert_eq!(zone_at(5.0, 1.6, gate, 1.5), ArenaZone::Ring);
        assert_eq!(zone_at(3.4, 0.0, gate, 1.5), ArenaZone::Ring);
    }

    /// The ring reads its gate mouth from the hand-placed row (10 m out, 2 m
    /// half-edge), not a caller constant: the same file the design gate gates.
    #[test]
    fn ring_zone_at_reads_the_shipped_gate_row() {
        assert_eq!(ring_zone_at(10.0, 0.0), ArenaZone::Gate);
        assert_eq!(ring_zone_at(11.9, 1.9), ArenaZone::Gate);
        assert_eq!(ring_zone_at(12.1, 0.0), ArenaZone::Wall);
        assert_eq!(ring_zone_at(5.0, 0.0), ArenaZone::Ring);
        assert_eq!(ring_zone_at(0.0, 0.0), ArenaZone::Centre);
    }

    #[test]
    fn tally_shares_sum_to_1000_and_names_the_entrance() {
        let mut tally = ZoneTally::default();
        assert_eq!(tally.total(), 0);
        assert_eq!(tally.entrance_share(), 0);
        for zone in [
            ArenaZone::Centre,
            ArenaZone::Ring,
            ArenaZone::Wall,
            ArenaZone::Gate,
        ] {
            tally.add(zone);
        }
        assert_eq!(tally.total(), 4);
        let shares = [
            tally.permille(ArenaZone::Centre),
            tally.permille(ArenaZone::Ring),
            tally.permille(ArenaZone::Wall),
            tally.entrance_share(),
        ];
        assert_eq!(shares, [250, 250, 250, 250]);
        assert_eq!(shares.iter().sum::<u32>(), 1000);
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

    // The ring loads the same from a packed bundle as from the directory, and
    // a bundle without a map fails with that map's name.
    #[test]
    fn ring_from_a_bundle_matches_the_directory_and_names_a_missing_map() {
        use engine_assets::asset_source::{pack_to, PackRules};
        let arena =
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../content/arena");
        let fs = FsSource::new(&arena);
        let bundle_of = |rules: &PackRules| {
            let mut bytes = Vec::new();
            pack_to(&fs, rules, &mut bytes).expect("textures pack");
            engine_assets::BundleSource::open(bytes).expect("bundle opens")
        };
        let sand = "textures/sand_albedo_2k.jpg";
        let both = bundle_of(
            &PackRules::new()
                .include(sand)
                .include("textures/stone_albedo_2k.jpg"),
        );
        let from_dir = FightRing::load(&arena, 20.0).expect("ring loads");
        let from_bundle = FightRing::load_from(&both, 20.0).expect("ring loads from the bundle");
        assert_eq!(from_bundle.sand, from_dir.sand);
        assert_eq!(from_bundle.stone, from_dir.stone);
        assert_eq!(from_bundle.wall_m, from_dir.wall_m);
        let sand_only = bundle_of(&PackRules::new().include(sand));
        let err = FightRing::load_from(&sand_only, 20.0).err();
        assert!(
            matches!(&err, Some(RingError::Texture(text)) if text.contains("stone_albedo_2k.jpg")),
            "a missing map fails by name, got {err:?}"
        );
    }
}
