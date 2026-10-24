//! break_window: the center crate in the playable window, whole until a sword hit breaks it
//! and then a pile of debris that falls and settles (RZ4w).
//!
//! The arena's bonded crate prop sits at the arena center. [`crate::break_look::BreakLook`]
//! turns its authority break events into float-physics debris; this module is the hook that
//! puts that picture in the window. It builds the look from the crate the live match wires
//! ([`engine_arena::crate_rest`]), takes each accepted tick's [`TickOutcome::breaks`], and hands
//! the draw loop the boxes to draw: the crate as one mesh while it stands, then the chunks (and
//! the debris they became) from the break tick on, the whole mesh hidden on that same tick.
//!
//! The renderer's [`DrawItem`] carries no per-draw transform, so each box is baked into world
//! space ([`baked_box`]) and uploaded under its piece key; a piece that did not move since the
//! last upload costs nothing, and a pile that sleeps stops uploading altogether.
//!
//! Caller: the `arena` window (`bin/arena.rs`): [`BreakScene::observe_outcome`] once
//! per accepted sim tick right after `tick_into`, [`BreakScene::push`] once per drawn frame
//! after the fighters. Data owner: the authority owns what broke; this module owns only the
//! picture. Units: meters, Y up. Determinism: the same ticks and events give the same boxes on
//! one machine; nothing here is hashed and the sim never reads it (the golden
//! `d57b5c8ae3096614` cannot move). Errors: [`SceneError`]; the window drops the picture on the first error and says so. Budget: at most the
//! tier's debris cap in bodies, 16 draws for the eight-chunk crate, no work per frame after
//! the pile sleeps.

use crate::break_look::{BreakCue, BreakDust, BreakLook, BreakProp, LookError, BREAK_MESH_ID_BASE};
use crate::camera::to_mm;
use crate::TickOutcome;
use engine_arena::{crate_rest, ArenaConfig};
use engine_core::hash::hash_bytes;
use engine_core::splitmix64;
use engine_destruction::budget::{
    BudgetError, DebrisBudget, DebrisLedger, ShardSeed, MAX_BUDGET_SHARDS, MAX_PER_EVENT,
    MAX_POS_MM,
};
use engine_destruction::debris_lod::{plan_lod, LodConfig, LodError, Tier as LodTier};
use engine_physfx::{Blast, Material, Stats, Tier};
use engine_render::particles_gpu::EmitterKind;
use engine_render::present::{DrawItem, DrawList, MeshKey, PresentError, Presenter};
use engine_render::v1::{cube, upload_mesh, MeshData, MeshError, PbrMaterial};

/// Debris tier of the window. The comfortable desktop tier holds a few hundred bodies, far
/// above the crate's eight chunks.
pub const WINDOW_TIER: Tier = Tier::Comfort;
/// Mesh id of the whole crate: past the id blocks of the first fourteen props.
const INTACT_MESH_ID: u64 = BREAK_MESH_ID_BASE + 0x7000;
/// The crate is a prop numbered 0 in the break id block.
const CRATE_PROP: u16 = 0;
/// Warm wood: the crate and its splinters share one linear albedo.
const WOOD: PbrMaterial = PbrMaterial {
    base_color: [0.42, 0.27, 0.14],
    metallic: 0.0,
    roughness: 0.82,
    exposure: 1.0,
};

/// Why a frame of the picture could not be made.
#[derive(Debug)]
pub enum SceneError {
    /// The break look refused the frame.
    Look(LookError),
    /// A box did not bake into a valid mesh.
    Mesh(MeshError),
    /// The renderer refused an upload.
    Upload(PresentError),
    /// The transient debris ledger refused the frame's pieces.
    Budget(BudgetError),
    /// The debris LOD bands were refused.
    Lod(LodError),
}

impl std::fmt::Display for SceneError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Look(err) => write!(f, "{err}"),
            Self::Mesh(err) => write!(f, "break box is not a mesh: {err}"),
            Self::Upload(err) => write!(f, "break box upload refused: {err}"),
            Self::Budget(err) => write!(f, "break LOD ledger refused: {err}"),
            Self::Lod(err) => write!(f, "break LOD plan refused: {err}"),
        }
    }
}

impl std::error::Error for SceneError {}

/// One box to draw this frame: which mesh, how big, and where.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SceneDraw {
    /// Key the box uploads under.
    pub mesh: MeshKey,
    /// Half-extents of the box, meters.
    pub half: [f32; 3],
    /// Model matrix, 3 rows of 4 row-major: the rotation, then the position in the last column.
    pub model: [f32; 12],
}

/// The crate's break picture. See the module docs.
pub struct BreakScene {
    look: BreakLook,
    intact: MeshKey,
    center_m: [f32; 3],
    half_m: f32,
    /// Authority break events seen so far (bond snaps plus freed chunks).
    breaks: u32,
    /// Accepted ticks observed so far.
    observed: u32,
    /// The tick of the first break event, once one came.
    broke_at: Option<u32>,
    /// The break sound of the last accepted tick, until the audio feed takes it.
    cue: Option<BreakCue>,
    /// The dust of the last tick that freed pieces, until the frame that draws it takes it: a
    /// quiet tick must not clear a cloud the frame has not drawn yet.
    dust: Option<BreakDust>,
    /// Puffs this picture has handed the particle feed, one cloud per tick that freed pieces.
    dust_puffs: u32,
    /// Boxes already on the GPU: half-extents and model they were baked with (a merged proxy
    /// reuses its lead piece's key with a bigger box, so the half counts too).
    sent: Vec<(MeshKey, [f32; 3], [f32; 12])>,
}

impl BreakScene {
    /// A scene over the crate a match of `config` wires. `None` when the crate does not bake
    /// (the case that also rejects the match) or the look refuses the prop.
    #[must_use]
    pub fn new(config: &ArenaConfig) -> Option<Self> {
        let rest = crate_rest(config)?;
        let intact = MeshKey::new(INTACT_MESH_ID, [0xB4; 32]);
        let prop = BreakProp::new(
            CRATE_PROP,
            intact,
            Material::Wood,
            &rest.chunks,
            &rest.bonds,
            &rest.anchors,
        )
        .ok()?;
        let look = BreakLook::new(WINDOW_TIER, prop).ok()?;
        Some(Self {
            look,
            intact,
            center_m: rest.center_m,
            half_m: rest.half_m,
            breaks: 0,
            observed: 0,
            broke_at: None,
            cue: None,
            dust: None,
            dust_puffs: 0,
            sent: Vec::new(),
        })
    }

    /// Takes one accepted tick: the look sees the tick's break events (an empty slice on a
    /// quiet tick) and steps its physical layer once. A tick the app rejected is not a tick
    /// and is skipped. A tick at or before the last one is refused (see [`BreakLook::observe`]).
    pub fn observe_outcome(&mut self, outcome: &TickOutcome) -> Result<(), LookError> {
        if !outcome.input_accepted {
            return Ok(());
        }
        self.look.observe(outcome.tick, &outcome.breaks)?;
        self.cue = self.look.take_cue();
        if let Some(cloud) = self.look.take_dust() {
            self.dust = Some(cloud);
        }
        self.observed += 1;
        if !outcome.breaks.is_empty() {
            self.breaks = self
                .breaks
                .saturating_add(u32::try_from(outcome.breaks.len()).unwrap_or(u32::MAX));
            self.broke_at.get_or_insert(outcome.tick);
        }
        Ok(())
    }

    /// The boxes to draw now, `alpha` in `[0, 1]` blending the last two ticks, seen from
    /// `camera_m` (meters). Before the first break event: the whole crate. From the break tick
    /// on: the chunks still standing and the debris, never the whole crate, thinned by the
    /// debris LOD (DX6): pieces within [`LOD_DISTANCE_MM`] draw as they are, past it the
    /// pieces of one kind (standing or flying) in one grid cell draw as one box around them,
    /// and far out they are dropped.
    pub fn draws(&self, alpha: f32, camera_m: [f32; 3]) -> Result<Vec<SceneDraw>, SceneError> {
        let frame = self.look.frame(alpha).map_err(SceneError::Look)?;
        if frame.hidden.contains(&self.intact) {
            let halves: Vec<(MeshKey, [f32; 3], u32)> = self
                .look
                .meshes()
                .iter()
                .flat_map(|m| {
                    [
                        (m.standing.key, m.standing.half, STANDING_ISLAND),
                        (m.flying.key, m.flying.half, FLYING_ISLAND),
                    ]
                })
                .collect();
            let (draws, islands): (Vec<SceneDraw>, Vec<u32>) = frame
                .draws
                .iter()
                .filter_map(|draw| {
                    let (_, half, island) = halves.iter().find(|(key, ..)| *key == draw.mesh)?;
                    let scene = SceneDraw {
                        mesh: draw.mesh,
                        half: *half,
                        model: draw.model_3x4(),
                    };
                    Some((scene, *island))
                })
                .unzip();
            return lod_draws(draws, &islands, camera_m);
        }
        let [x, y, z] = self.center_m;
        Ok(vec![SceneDraw {
            mesh: self.intact,
            half: [self.half_m; 3],
            model: [1.0, 0.0, 0.0, x, 0.0, 1.0, 0.0, y, 0.0, 0.0, 1.0, z],
        }])
    }

    /// Uploads what moved since the last frame and pushes one draw per box onto `list`, as seen
    /// from `camera_m` (see [`Self::draws`]). Returns the draws pushed. An error means the
    /// picture is unusable: the caller drops the scene (the window keeps running without the
    /// crate).
    pub fn push(
        &mut self,
        presenter: &mut Presenter,
        list: &mut DrawList,
        alpha: f32,
        camera_m: [f32; 3],
    ) -> Result<usize, SceneError> {
        let draws = self.draws(alpha, camera_m)?;
        // RZ4 dust: a tick that freed pieces raised one cloud in the look; it reaches the fight
        // pool once, on the frame that draws that tick, so a break tick puffs where the blow
        // landed and a quiet frame (no cloud waiting) adds nothing. Straight up, as the
        // knockdown path throws its dust.
        if let Some(dust) = self.take_dust() {
            presenter.fight_burst(EmitterKind::Dust, dust.at_m, [0.0, 1.0, 0.0], dust.puffs);
            self.dust_puffs = self.dust_puffs.saturating_add(dust.puffs);
        }
        for draw in &draws {
            let fresh = self
                .sent
                .iter()
                .position(|(key, ..)| *key == draw.mesh)
                .is_none_or(|at| self.sent[at].1 != draw.half || self.sent[at].2 != draw.model);
            if fresh {
                let mesh = if shard_pieces() {
                    baked_shard(draw.half, &draw.model, shard_seed(&draw.mesh))
                } else {
                    baked_box(draw.half, &draw.model)
                }
                .map_err(SceneError::Mesh)?;
                presenter
                    .upload_mesh(draw.mesh, &mesh)
                    .map_err(SceneError::Upload)?;
                match self.sent.iter_mut().find(|(key, ..)| *key == draw.mesh) {
                    Some(entry) => (entry.1, entry.2) = (draw.half, draw.model),
                    None => self.sent.push((draw.mesh, draw.half, draw.model)),
                }
            }
            list.items.push(DrawItem {
                mesh: draw.mesh,
                material: WOOD,
                base_color: None,
                normal_map: None,
                orm: None,
                emissive: None,
            });
        }
        Ok(draws.len())
    }

    /// Authority break events seen so far (bond snaps plus freed chunks).
    #[must_use]
    pub const fn breaks(&self) -> u32 {
        self.breaks
    }

    /// Debris bodies alive in the physical layer.
    #[must_use]
    pub fn debris(&self) -> usize {
        self.look.stats().bodies
    }

    /// Accepted ticks observed so far.
    #[must_use]
    pub const fn observed(&self) -> u32 {
        self.observed
    }

    /// The tick of the first break event, once one came.
    #[must_use]
    pub const fn broke_at(&self) -> Option<u32> {
        self.broke_at
    }

    /// The break sound of the last accepted tick, once: the window's audio feed plays it.
    pub fn take_cue(&mut self) -> Option<BreakCue> {
        self.cue.take()
    }

    /// Applies one authority blast to the crate's debris and raises its sound and dust: call it
    /// on the tick a weapon lands, after [`BreakScene::observe_outcome`], so the blast's cue and
    /// cloud replace that tick's break ones (the louder event wins). The shove is the same as
    /// [`crate::break_look::BreakLook::apply_blast`]; the sound hands out through
    /// [`BreakScene::take_cue`] and the cloud through [`BreakScene::take_dust`]. A blast before
    /// the break moves no body but still sounds and puffs. Presentation only.
    pub fn observe_blast(&mut self, blast: Blast) -> Result<usize, LookError> {
        let pushed = self.look.observe_blast(blast)?;
        if let Some(cue) = self.look.take_cue() {
            self.cue = Some(cue);
        }
        if let Some(cloud) = self.look.take_dust() {
            self.dust = Some(cloud);
        }
        Ok(pushed)
    }

    /// The blast dust of the last [`BreakScene::observe_blast`], once.
    pub fn take_dust(&mut self) -> Option<BreakDust> {
        self.dust.take()
    }

    /// Puffs this picture has handed the particle feed: one cloud per tick that freed pieces,
    /// never growing on a quiet tick.
    #[must_use]
    pub const fn dust_puffs(&self) -> u32 {
        self.dust_puffs
    }

    /// Counters of the physical layer.
    #[must_use]
    pub fn stats(&self) -> Stats {
        self.look.stats()
    }

    /// The window's receipt line: `arena break: breaks=<n> debris=<n> awake=<n> dust=<n>`, where
    /// `dust` is the particles this picture threw for the breaks.
    #[must_use]
    pub fn receipt_line(&self) -> String {
        format!(
            "arena break: breaks={} debris={} awake={} dust={}",
            self.breaks,
            self.debris(),
            self.stats().awake,
            self.dust_puffs
        )
    }
}

/// Distance past which the crate's pieces merge (DX6), mm: farther than the pit's own shots, so
/// the merge only thins a far or wide view. Impostors (dropped here) start at three times it.
pub const LOD_DISTANCE_MM: i32 = 14_000;
/// LOD island of the pieces still standing in the wall.
const STANDING_ISLAND: u32 = 0;
/// LOD island of the flying debris: merged apart from the wall.
const FLYING_ISLAND: u32 = 1;

/// The debris LOD of one frame: the boxes are the shards of a transient `DebrisLedger`
/// (`islands[i]` is box `i`'s island), `plan_lod` tiers them by distance to `camera_m`, and the
/// answer is drawn in the boxes' own order: Full and Simplified boxes as they are (there is no
/// lower mesh to swap in), one box around each merge group (under its lead's mesh key), and
/// nothing for impostor-tier pieces.
fn lod_draws(
    draws: Vec<SceneDraw>,
    islands: &[u32],
    camera_m: [f32; 3],
) -> Result<Vec<SceneDraw>, SceneError> {
    let mm = |m: f32| to_mm(m).clamp(-MAX_POS_MM, MAX_POS_MM);
    let budget = DebrisBudget {
        per_event_cap: MAX_PER_EVENT,
        max_shards: u32::try_from(draws.len().clamp(1, MAX_BUDGET_SHARDS)).unwrap_or(1),
        sleep_ticks: 1,
        lod_distance_mm: LOD_DISTANCE_MM,
        sleep_speed_mm_per_tick: 0,
    };
    let mut ledger = DebrisLedger::new(&budget).map_err(SceneError::Budget)?;
    let seeds: Vec<ShardSeed> = draws
        .iter()
        .zip(islands)
        .enumerate()
        .map(|(id, (draw, island))| ShardSeed {
            id: u32::try_from(id).unwrap_or(u32::MAX),
            island: *island,
            pos_mm: [mm(draw.model[3]), mm(draw.model[7]), mm(draw.model[11])],
            vel_mm_per_tick: [0; 3],
        })
        .collect();
    for batch in seeds.chunks(usize::try_from(MAX_PER_EVENT).unwrap_or(1)) {
        ledger.admit(0, batch).map_err(SceneError::Budget)?;
    }
    let camera = camera_m.map(mm);
    let plan =
        plan_lod(&ledger, camera, &LodConfig::from_budget(&budget)).map_err(SceneError::Lod)?;
    let mut tier_of = vec![LodTier::Full; draws.len()];
    for (id, tier) in &plan.tiers {
        tier_of[usize::try_from(*id).unwrap_or(usize::MAX)] = *tier;
    }
    let mut out = Vec::with_capacity(plan.draws());
    for (at, tier) in tier_of.iter().enumerate() {
        match *tier {
            LodTier::Full | LodTier::Simplified(_) => out.push(draws[at]),
            LodTier::Merged(lead) if usize::try_from(lead) == Ok(at) => {
                let members = tier_of
                    .iter()
                    .zip(&draws)
                    .filter(|(t, _)| **t == LodTier::Merged(lead))
                    .map(|(_, draw)| draw);
                out.push(merged_box(draws[at].mesh, members));
            }
            LodTier::Merged(_) | LodTier::Impostor(_) => {}
        }
    }
    Ok(out)
}

/// One axis-aligned box around `members`, drawn under `mesh`.
fn merged_box<'a>(mesh: MeshKey, members: impl Iterator<Item = &'a SceneDraw>) -> SceneDraw {
    let mut low = [f32::INFINITY; 3];
    let mut high = [f32::NEG_INFINITY; 3];
    for draw in members {
        for axis in 0..3 {
            let row = &draw.model[axis * 4..axis * 4 + 3];
            let reach: f32 = row.iter().zip(draw.half).map(|(m, h)| m.abs() * h).sum();
            let center = draw.model[axis * 4 + 3];
            low[axis] = low[axis].min(center - reach);
            high[axis] = high[axis].max(center + reach);
        }
    }
    let center = [0, 1, 2].map(|a| (low[a] + high[a]) * 0.5);
    SceneDraw {
        mesh,
        half: [0, 1, 2].map(|a| (high[a] - low[a]) * 0.5),
        model: [
            1.0, 0.0, 0.0, center[0], 0.0, 1.0, 0.0, center[1], 0.0, 0.0, 1.0, center[2],
        ],
    }
}

/// A box of half-extents `half` placed by `model` (3 rows of 4, row-major), as a mesh in world
/// space with outward normals: the renderer draws meshes where they were baked.
pub fn baked_box(half: [f32; 3], model: &[f32; 12]) -> Result<MeshData, MeshError> {
    let unit = cube();
    let turn = |v: [f32; 3]| {
        [
            model[0] * v[0] + model[1] * v[1] + model[2] * v[2],
            model[4] * v[0] + model[5] * v[1] + model[6] * v[2],
            model[8] * v[0] + model[9] * v[1] + model[10] * v[2],
        ]
    };
    let positions: Vec<[f32; 3]> = unit
        .positions
        .iter()
        .map(|p| {
            let r = turn([
                p[0] * 2.0 * half[0],
                p[1] * 2.0 * half[1],
                p[2] * 2.0 * half[2],
            ]);
            [r[0] + model[3], r[1] + model[7], r[2] + model[11]]
        })
        .collect();
    let normals: Vec<[f32; 3]> = unit.normals.iter().map(|n| turn(*n)).collect();
    upload_mesh(&positions, &normals, &unit.uvs, &unit.indices)
}

/// Draw a chunk as a beveled shard (the default) or as the plain box. The box is the shard
/// look's off switch for a capture pair: both bake the same piece into the same place, so the
/// pair moves no sim state and no golden. `E2040_BREAK_BOX=1` picks the box; the environment is
/// read once per process, and the capture harness is the only caller that sets it.
fn shard_pieces() -> bool {
    static BOX: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    !*BOX.get_or_init(|| std::env::var("E2040_BREAK_BOX").is_ok_and(|v| v == "1"))
}

/// Smallest share of a half-extent a shard corner keeps: the corners are pulled inward by a
/// seeded fraction of [`1.0 - SHARD_MIN_SHARE`] of their own arm, never past the chunk's AABB,
/// so the drawn shard never pokes out of the box the physical layer simulates.
const SHARD_MIN_SHARE: f32 = 0.82;

/// Corners of a box, indexed `x + 2 * y + 4 * z` with a set bit meaning `+`.
const SHARD_FACES: [[usize; 4]; 6] = [
    [4, 5, 7, 6],
    [0, 2, 3, 1],
    [1, 3, 7, 5],
    [0, 4, 6, 2],
    [2, 6, 7, 3],
    [0, 1, 5, 4],
];

/// The four quad corners' UVs, shared by every shard face.
const SHARD_UVS: [[f32; 2]; 4] = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];

/// A beveled shard baked into world space: the chunk's box with its eight corners pulled inward
/// by a fraction seeded from `seed`, its six outside faces and the same six reversed as inside
/// faces, placed by `model` (3 rows of 4, row-major). Every vertex stays inside the box's AABB.
/// The same arguments give the same [`MeshData`] twice. `upload_mesh` validates the result.
pub fn baked_shard(half: [f32; 3], model: &[f32; 12], seed: u64) -> Result<MeshData, MeshError> {
    let h = half.map(|v| v.abs().max(1e-4));
    let mut corners = [[0.0f32; 3]; 8];
    for (i, corner) in corners.iter_mut().enumerate() {
        let sign = |bit: usize| if i & (1 << bit) != 0 { 1.0f32 } else { -1.0 };
        for (axis, c) in corner.iter_mut().enumerate() {
            let share = SHARD_MIN_SHARE + (1.0 - SHARD_MIN_SHARE) * seeded_fraction(seed, i, axis);
            *c = sign(axis) * h[axis] * share;
        }
    }
    let turn = |v: [f32; 3]| {
        [
            model[0] * v[0] + model[1] * v[1] + model[2] * v[2],
            model[4] * v[0] + model[5] * v[1] + model[6] * v[2],
            model[8] * v[0] + model[9] * v[1] + model[10] * v[2],
        ]
    };
    let mut positions = Vec::with_capacity(SHARD_FACES.len() * 8);
    let mut normals = Vec::with_capacity(SHARD_FACES.len() * 8);
    let mut uvs = Vec::with_capacity(SHARD_FACES.len() * 8);
    let mut indices = Vec::with_capacity(SHARD_FACES.len() * 12);
    for face in &SHARD_FACES {
        let world: [[f32; 3]; 4] = [0, 1, 2, 3].map(|k| {
            let r = turn(corners[face[k]]);
            [r[0] + model[3], r[1] + model[7], r[2] + model[11]]
        });
        let outward = face_normal(world[0], world[1], world[2]);
        // Outside triangles keep the face's winding; inside ones reverse it and flip the normal,
        // so a cut face reads from within when a piece tumbles.
        for (flip, order) in [
            (false, [0u32, 1, 2, 0, 2, 3]),
            (true, [0u32, 2, 1, 0, 3, 2]),
        ] {
            let base = u32::try_from(positions.len()).unwrap_or(0);
            let normal = if flip { outward.map(|c| -c) } else { outward };
            for (k, at) in world.iter().enumerate() {
                positions.push(*at);
                normals.push(normal);
                uvs.push(SHARD_UVS[k]);
            }
            indices.extend(order.map(|i| base + i));
        }
    }
    upload_mesh(&positions, &normals, &uvs, &indices)
}

/// The unit outward normal of triangle `a b c`, or `+Y` for a triangle too flat to have one (the
/// corner jitter cannot make one flat, so this only guards a bad half-extent).
fn face_normal(a: [f32; 3], b: [f32; 3], c: [f32; 3]) -> [f32; 3] {
    let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let v = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
    let n = [
        u[1] * v[2] - u[2] * v[1],
        u[2] * v[0] - u[0] * v[2],
        u[0] * v[1] - u[1] * v[0],
    ];
    let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
    if len > 1e-6 {
        [n[0] / len, n[1] / len, n[2] / len]
    } else {
        [0.0, 1.0, 0.0]
    }
}

/// A stable fraction in `0.0..=1.0` for one corner and axis of a shard seed: the same seed gives
/// the same corner every run and every machine (the core's hash and mix, no float state).
fn seeded_fraction(seed: u64, corner: usize, axis: usize) -> f32 {
    let mut raw = [0u8; 10];
    raw[..8].copy_from_slice(&seed.to_le_bytes());
    raw[8] = u8::try_from(corner).unwrap_or(0);
    raw[9] = u8::try_from(axis).unwrap_or(0);
    let z = splitmix64(hash_bytes(&raw));
    f32::from(u16::try_from((z >> 48) & 0xFFFF).unwrap_or(0)) / 65535.0
}

/// The shard seed of one piece key: the chunk's id and the box digest (the box digest already
/// follows `hx, hy, hz`), so each piece keeps its own corner pattern as it moves.
#[must_use]
pub fn shard_seed(key: &MeshKey) -> u64 {
    let mut head = [0u8; 8];
    head.copy_from_slice(&key.digest[..8]);
    key.id.rotate_left(17) ^ u64::from_le_bytes(head)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{App, GameConfig, HUMAN_SLOT};
    use engine_destruction::bonded::BreakEvent;
    use engine_input::{Buttons, FighterId, FighterInput};

    /// Seed 2040, two fighters, 64 ticks: the circle-strafe script below lands its first Hit
    /// (and so the crate's break: five events, one chunk freed) on tick 4.
    fn config() -> GameConfig {
        GameConfig {
            seed: 2040,
            ticks: 64,
            fighters: 2,
            half_extent_m: 20.0,
        }
    }

    /// Circle-strafe with an attack every 30 ticks.
    fn script(tick: u32) -> FighterInput {
        let (mx, mz) = match (tick / 15) % 4 {
            0 => (100, 0),
            1 => (0, 100),
            2 => (-100, 0),
            _ => (0, -100),
        };
        let buttons = if tick.is_multiple_of(30) {
            Buttons(engine_input::fighter::ATTACK)
        } else {
            Buttons::NONE
        };
        FighterInput {
            tick,
            fighter: FighterId::new(HUMAN_SLOT).expect("slot"),
            mx,
            mz,
            facing: 0,
            buttons,
        }
    }

    /// A camera in the pit, well inside the LOD distance.
    const NEAR_M: [f32; 3] = [0.0, 2.0, 4.0];

    fn scene_for(app: &App) -> BreakScene {
        BreakScene::new(&app.live_match().config()).expect("the crate bakes")
    }

    /// Full-speed walk at the other fighter in world axes, facing it: the wish
    /// pair is world planar (`mx` x, `mz` z), the facing looks at the target.
    fn chase(app: &App) -> (i8, i8, i16) {
        let views = app.fighter_views();
        let (me, them) = match views {
            [me, them, ..] => (me.pos_m, them.pos_m),
            _ => return (0, 0, 0),
        };
        let dx = them[0] - me[0];
        let dz = them[2] - me[2];
        let facing = (dx.atan2(dz) * 10_000.0 / std::f32::consts::PI) as i16;
        let axis = |d: f32| {
            if d > 0.2 {
                100
            } else if d < -0.2 {
                -100
            } else {
                0
            }
        };
        (axis(dx), axis(dz), facing)
    }

    /// F2P: the window's tick outcome carries the authority's break events, tick for tick the
    /// same as the arena's own break log.
    #[test]
    fn break_wire_tick_outcome_carries_the_authority_breaks() {
        let cfg = config();
        let mut app = App::new(cfg).expect("app");
        let mut taped = Vec::new();
        let mut seen = Vec::new();
        for tick in 0..cfg.ticks {
            let input = script(tick);
            taped.push(input);
            let out = app.tick(input);
            assert!(out.input_accepted);
            seen.push(out.breaks);
        }
        let arena = engine_arena::run_breaks(
            &engine_arena::ArenaConfig {
                seed: cfg.seed,
                ticks: cfg.ticks,
                fighters: cfg.fighters,
                half_extent_m: cfg.half_extent_m,
                table_version: engine_combat::TABLES_VERSION_V1,
            },
            &engine_input::FighterTape(taped),
        );
        assert_eq!(arena.len(), seen.len());
        for (row, got) in arena.iter().zip(&seen) {
            assert_eq!(&row.events, got, "tick {}", row.tick);
        }
        assert!(
            seen.iter().any(|events| !events.is_empty()),
            "the script breaks the crate"
        );
    }

    /// F2P: the window's tick outcome carries the authority's cut events, tick for tick the same
    /// as the arena's own cut log under the v2 duel.
    #[test]
    fn break_wire_tick_outcome_carries_the_authority_cuts() {
        let cfg = GameConfig {
            ticks: 240,
            fighters: 8,
            half_extent_m: 5.0,
            ..config()
        };
        let mut app = App::new(cfg).expect("app").into_duel().expect("duel");
        let mut taped = Vec::new();
        let mut seen = Vec::new();
        let mut hits = 0usize;
        for tick in 0..cfg.ticks {
            // Walk at the second fighter and hold attack: the swing resolves in
            // reach, and a landed hit severs the first joint it meets.
            let (mx, mz, facing) = chase(&app);
            let input = FighterInput {
                tick,
                fighter: FighterId::new(HUMAN_SLOT).expect("slot"),
                mx,
                mz,
                facing,
                buttons: Buttons(engine_input::fighter::ATTACK),
            };
            taped.push(input);
            let out = app.tick(input);
            assert!(out.input_accepted);
            hits += out
                .cues
                .iter()
                .filter(|cue| matches!(cue, crate::FightCue::Hit { .. }))
                .count();
            seen.push(out.cuts);
        }
        // The oracle: an independent live match armed exactly like the app's,
        // stepped with the same tape and never through `App`'s forwarding.
        let mut oracle = engine_arena::MatchSpec::duel(
            app.live_match().config(),
            engine_arena::world::MatchWorld::empty(),
        )
        .with_weapons(&vec![crate::DUEL_KIT_WEAPON; usize::from(cfg.fighters)])
        .live()
        .expect("the oracle duel builds");
        let mut rows = Vec::new();
        for input in &taped {
            let stepped = oracle
                .step(std::slice::from_ref(input))
                .expect("oracle tick");
            rows.push(stepped.cuts.cuts.clone());
        }
        assert_eq!(
            app.live_match().config().table_version,
            engine_combat::tables::TABLES_VERSION_V2,
            "the duel runs the cutting tables"
        );
        assert_eq!(rows.len(), seen.len());
        for (tick, (row, got)) in rows.iter().zip(&seen).enumerate() {
            assert_eq!(row, got, "tick {tick}");
        }
        assert!(hits > 0, "the brawl lands hits");
        assert!(
            seen.iter().any(|cuts| !cuts.is_empty()),
            "the duel severs a joint"
        );
    }

    /// F2P: one blast on the crate's debris shoves the float bodies and raises its sound and
    /// dust; the blast's cue replaces the tick's break cue and the cloud hands out once.
    #[test]
    fn break_wire_blast_shoves_the_debris_and_raises_its_sound_and_dust() {
        let cfg = config();
        let mut app = App::new(cfg).expect("app");
        let mut scene = scene_for(&app);
        for tick in 0..cfg.ticks {
            let out = app.tick(script(tick));
            scene.observe_outcome(&out).expect("tick");
            if scene.broke_at().is_some() {
                break;
            }
        }
        assert!(scene.broke_at().is_some(), "the script breaks the crate");
        // The break's own sound and cloud hand out first; the blast replaces both.
        assert!(scene.take_cue().is_some(), "the break sounds");
        assert!(scene.take_dust().is_some(), "the break raises dust");
        let blast = Blast {
            center_mm: scene.center_m.map(to_mm),
            radius_mm: 5_000,
            impulse_ns: 400,
        };
        let pushed = scene.observe_blast(blast).expect("the blast applies");
        assert!(pushed > 0, "the blast shoves the flying debris");
        assert_eq!(
            scene.take_cue(),
            Some(crate::break_look::blast_cue(blast)),
            "the blast's cue wins the tick"
        );
        let dust = scene.take_dust().expect("the blast raises dust");
        assert_eq!(dust, crate::break_look::blast_dust(blast));
        assert!(dust.puffs >= 24, "a blast never whispers: {}", dust.puffs);
        assert_eq!(scene.take_dust(), None, "one cloud per call");
        assert_eq!(scene.take_cue(), None, "one sound per call");
    }

    /// F2P: the hook's call order. One `observe` per accepted tick, ascending, an empty slice on
    /// quiet ticks; the whole crate draws until the break tick and is gone on it; a replayed
    /// tick is refused; a rejected tick is no tick.
    #[test]
    fn break_wire_one_observe_per_tick_and_the_crate_is_gone_on_the_break_tick() {
        let cfg = config();
        let mut app = App::new(cfg).expect("app");
        let mut scene = scene_for(&app);
        let intact = scene.intact;
        let chunk_count = scene.look.meshes().len();
        let mut break_tick = None;
        let mut events_total = 0usize;
        let mut freed_total = 0usize;
        for tick in 0..cfg.ticks {
            let out = app.tick(script(tick));
            scene.observe_outcome(&out).expect("tick observed");
            assert_eq!(scene.observed(), tick + 1, "one observe per accepted tick");
            events_total += out.breaks.len();
            freed_total += out
                .breaks
                .iter()
                .filter(|e| matches!(e, BreakEvent::ChunkFreed { .. }))
                .count();
            let draws = scene.draws(1.0, NEAR_M).expect("frame");
            if break_tick.is_none() && !out.breaks.is_empty() {
                break_tick = Some(tick);
            }
            match break_tick {
                None => {
                    assert_eq!(draws.len(), 1, "tick {tick}: the whole crate");
                    assert_eq!(draws[0].mesh, intact);
                    assert_eq!(draws[0].half, [0.5; 3]);
                }
                Some(_) => {
                    assert_eq!(
                        draws.len(),
                        chunk_count,
                        "tick {tick}: every chunk, no more"
                    );
                    assert!(
                        draws.iter().all(|d| d.mesh != intact),
                        "tick {tick}: crate gone"
                    );
                }
            }
            // A second look at the same tick is a replay: refused, and nothing changes.
            assert!(matches!(
                scene.observe_outcome(&out),
                Err(LookError::StaleTick { .. })
            ));
            assert_eq!(scene.observed(), tick + 1);
        }
        assert_eq!(break_tick, Some(4), "the scripted Hit lands on tick 4");
        assert_eq!(scene.broke_at(), Some(4));
        assert_eq!(scene.breaks() as usize, events_total);
        assert!(freed_total > 0, "the script frees a chunk");
        assert_eq!(
            scene.debris(),
            freed_total,
            "one debris body per freed chunk"
        );
        let line = scene.receipt_line();
        assert!(
            line.contains(&format!("breaks={events_total} debris={}", scene.debris())),
            "{line}"
        );
        // A rejected tick (past the end) is skipped, not observed.
        let rejected = app.tick(script(cfg.ticks));
        assert!(!rejected.input_accepted);
        scene.observe_outcome(&rejected).expect("skipped");
        assert_eq!(scene.observed(), cfg.ticks);
    }

    /// F2P: the debris falls and settles. Pieces start in the crate and end on the ground,
    /// asleep, at every alpha the same.
    #[test]
    fn break_wire_debris_falls_and_settles() {
        let cfg = GameConfig {
            ticks: 400,
            ..config()
        };
        let mut app = App::new(cfg).expect("app");
        let mut scene = scene_for(&app);
        let mut first: Option<Vec<SceneDraw>> = None;
        let mut asleep_at = None;
        for tick in 0..cfg.ticks {
            let out = app.tick(script(tick));
            scene.observe_outcome(&out).expect("tick");
            if tick == 4 {
                first = Some(scene.draws(0.0, NEAR_M).expect("frame"));
            }
            if tick > 4 && asleep_at.is_none() && scene.stats().awake == 0 {
                asleep_at = Some(tick);
            }
        }
        let first = first.expect("break tick frame");
        let end = scene.draws(1.0, NEAR_M).expect("frame");
        assert!(
            asleep_at.is_some(),
            "the pile sleeps inside {} ticks",
            cfg.ticks
        );
        assert_eq!(end.len(), first.len(), "nothing was lost");
        let moved = end
            .iter()
            .zip(&first)
            .filter(|(a, b)| {
                let d = [3, 7, 11].map(|k| a.model[k] - b.model[k]);
                d[0] * d[0] + d[1] * d[1] + d[2] * d[2] > 0.01
            })
            .count();
        assert!(moved > 0, "pieces left the crate");
        assert!(
            end.iter().all(|d| d.model[7] > 0.0),
            "every piece rests above the ground"
        );
        assert_eq!(
            scene.draws(0.3, NEAR_M).expect("frame"),
            end,
            "asleep is alpha independent"
        );
    }

    /// DX6: the debris LOD is live in the draw. From the pit every chunk draws; from past the
    /// merge distance the pieces of one kind in one cell become one box that holds them all;
    /// from three times as far they are dropped. The whole crate is never thinned.
    #[test]
    fn break_wire_far_cameras_merge_then_drop_the_pieces() {
        let cfg = config();
        let mut app = App::new(cfg).expect("app");
        let mut scene = scene_for(&app);
        let far_m = [0.0, 2.0, 20.0];
        for tick in 0..cfg.ticks {
            let out = app.tick(script(tick));
            scene.observe_outcome(&out).expect("tick");
            if scene.broke_at().is_none() {
                assert_eq!(scene.draws(1.0, far_m).expect("whole").len(), 1);
            }
        }
        let near = scene.draws(1.0, NEAR_M).expect("near");
        let far = scene.draws(1.0, far_m).expect("far");
        assert_eq!(
            near.len(),
            scene.look.meshes().len(),
            "the pit sees every chunk"
        );
        assert!(
            !far.is_empty() && far.len() < near.len(),
            "{} -> {}",
            near.len(),
            far.len()
        );
        let reach = |draw: &SceneDraw, axis: usize| -> f32 {
            let row = &draw.model[axis * 4..axis * 4 + 3];
            row.iter().zip(draw.half).map(|(m, h)| m.abs() * h).sum()
        };
        for piece in &near {
            let held = far.iter().any(|bin| {
                (0..3).all(|a| {
                    let (c, b) = (piece.model[a * 4 + 3], bin.model[a * 4 + 3]);
                    (c - b).abs() + reach(piece, a) <= reach(bin, a) + 1.0e-4
                })
            });
            assert!(held, "a merged box holds {piece:?}");
        }
        assert!(
            scene
                .draws(1.0, [0.0, 2.0, 100.0])
                .expect("gone")
                .is_empty(),
            "impostor-tier pieces are dropped"
        );
    }

    /// F2P: the picture never touches the match. Two sessions play the same script, one with
    /// the break picture observing every tick and baking every frame; every checkpoint hash,
    /// the hp and every tick outcome are equal.
    #[test]
    fn break_wire_hashes_with_and_without_the_picture_are_equal() {
        let cfg = GameConfig {
            ticks: 600,
            ..config()
        };
        let mut bare = App::new(cfg).expect("app");
        let mut painted = App::new(cfg).expect("app");
        let mut scene = scene_for(&painted);
        for tick in 0..cfg.ticks {
            let a = bare.tick(script(tick));
            let b = painted.tick(script(tick));
            scene.observe_outcome(&b).expect("tick");
            for draw in scene.draws(0.5, NEAR_M).expect("frame") {
                baked_box(draw.half, &draw.model).expect("bakes");
            }
            assert_eq!(a, b, "tick {tick}: outcome");
            assert_eq!(a.checkpoint, b.checkpoint, "tick {tick}: checkpoint");
        }
        assert_eq!(bare.checkpoints(), painted.checkpoints());
        assert_eq!(bare.hp(), painted.hp());
        assert!(scene.breaks() > 0, "the painted run did see the break");
        assert!(scene.debris() > 0 && scene.debris() <= WINDOW_TIER.caps().bodies);
    }

    /// RZ4 dust: the look's cloud reaches the frame once, on the tick that freed pieces, and a
    /// quiet tick raises none. The window's `push` reads this same `take_dust`, so the break
    /// tick is the only frame that adds puffs to the fight pool.
    #[test]
    fn break_wire_a_break_tick_hands_one_dust_cloud_and_a_quiet_tick_none() {
        let cfg = config();
        let mut app = App::new(cfg).expect("app");
        let mut scene = scene_for(&app);
        let mut cloud = None;
        let mut break_tick = 0;
        for tick in 0..cfg.ticks {
            let out = app.tick(script(tick));
            scene.observe_outcome(&out).expect("tick");
            if out.breaks.is_empty() && cloud.is_none() {
                assert_eq!(
                    scene.take_dust(),
                    None,
                    "tick {tick}: a quiet tick raises no cloud"
                );
            } else if cloud.is_none() {
                break_tick = tick;
                cloud = scene.take_dust();
            }
        }
        let dust = cloud.expect("the break tick raises a cloud");
        assert!(dust.puffs > 0, "{dust:?}");
        let [x, y, z] = scene.center_m;
        let reach = scene.half_m + 1.0;
        assert!(
            (dust.at_m[0] - x).abs() <= reach
                && (dust.at_m[1] - y).abs() <= reach
                && (dust.at_m[2] - z).abs() <= reach,
            "the cloud sits on the crate: {dust:?}"
        );
        assert_eq!(scene.take_dust(), None, "one cloud per break tick");
        assert_eq!(
            scene.dust_puffs(),
            0,
            "no puffs before a frame draws the cloud"
        );
        // The window draws many ticks per frame: a cloud raised on one tick must survive the
        // quiet ticks that follow it until the frame that draws it takes it.
        let mut fresh = App::new(cfg).expect("app");
        let mut held = scene_for(&fresh);
        let mut painted = None;
        for tick in 0..cfg.ticks {
            let out = fresh.tick(script(tick));
            held.observe_outcome(&out).expect("tick");
            if painted.is_none() && held.dust.is_some() {
                painted = Some(tick);
            }
            if let Some(at) = painted {
                if tick >= at + 3 {
                    let cloud = held.take_dust().unwrap_or_else(|| {
                        panic!("the cloud of tick {at} survived to tick {tick}")
                    });
                    assert!(cloud.puffs > 0);
                    assert_eq!(held.take_dust(), None, "one cloud, once");
                    assert!(at <= break_tick, "same script, same break");
                    return;
                }
            }
        }
        panic!("the script breaks the crate");
    }

    /// RZ4 shards: one beveled mesh per piece, deterministic from its key, every vertex inside
    /// the chunk's AABB, and validated by the same `upload_mesh` the presenter runs.
    #[test]
    fn break_wire_shard_mesh_is_deterministic_and_inside_its_box() {
        let identity = [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0];
        let key = MeshKey::new(7, [3; 32]);
        let seed = shard_seed(&key);
        for half in [[0.5, 0.5, 0.5], [0.2, 0.6, 0.35], [1.0, 0.15, 0.8]] {
            let shard = baked_shard(half, &identity, seed).expect("the shard validates");
            let again = baked_shard(half, &identity, seed).expect("the shard validates");
            assert_eq!(shard, again, "the same seed bakes the same mesh twice");
            assert_eq!(
                shard.positions.len(),
                SHARD_FACES.len() * 8,
                "six faces, two shells"
            );
            assert_eq!(
                shard.indices.len(),
                SHARD_FACES.len() * 12,
                "outside and inside"
            );
            for (axis, h) in half.iter().enumerate() {
                for p in &shard.positions {
                    assert!(
                        p[axis].abs() <= h + 1e-6,
                        "axis {axis}: {} pokes out of {h}",
                        p[axis]
                    );
                }
                // The shard is not the box: at least one corner was pulled inward.
                let (lo, hi) = shard
                    .positions
                    .iter()
                    .fold((f32::MAX, f32::MIN), |(lo, hi), p| {
                        (lo.min(p[axis]), hi.max(p[axis]))
                    });
                assert!(hi - lo < 2.0 * h, "axis {axis}: the box was not beveled");
            }
            for n in &shard.normals {
                let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
                assert!((len - 1.0).abs() < 1e-5, "unit normal, got {len}");
            }
        }
        // A different piece key is a different shard.
        let other = shard_seed(&MeshKey::new(8, [3; 32]));
        assert_ne!(
            baked_shard([0.5; 3], &identity, seed).expect("bakes"),
            baked_shard([0.5; 3], &identity, other).expect("bakes")
        );
        // The shard lands where the model puts it, inside the moved box.
        let moved = [1.0, 0.0, 0.0, 2.0, 0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 1.0, -3.0];
        let shard = baked_shard([0.5, 0.25, 0.5], &moved, seed).expect("bakes");
        for p in &shard.positions {
            assert!((p[0] - 2.0).abs() <= 0.5 + 1e-6);
            assert!((p[1] - 1.0).abs() <= 0.25 + 1e-6);
            assert!((p[2] + 3.0).abs() <= 0.5 + 1e-6);
        }
    }

    /// The baked box lands where the model puts it, with unit outward normals.
    #[test]
    fn break_wire_baked_box_is_where_the_model_says() {
        let identity = [1.0, 0.0, 0.0, 1.0, 0.0, 1.0, 0.0, 2.0, 0.0, 0.0, 1.0, 3.0];
        let mesh = baked_box([0.5, 0.25, 1.0], &identity).expect("bakes");
        let span = |axis: usize| {
            let lo = mesh
                .positions
                .iter()
                .map(|p| p[axis])
                .fold(f32::MAX, f32::min);
            let hi = mesh
                .positions
                .iter()
                .map(|p| p[axis])
                .fold(f32::MIN, f32::max);
            (lo, hi)
        };
        assert_eq!(span(0), (0.5, 1.5));
        assert_eq!(span(1), (1.75, 2.25));
        assert_eq!(span(2), (2.0, 4.0));
        // A quarter turn about Y swaps the X and Z extents of the same box.
        let turned = [0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, -1.0, 0.0, 0.0, 0.0];
        let mesh = baked_box([0.5, 0.25, 1.0], &turned).expect("bakes");
        let hi = |axis: usize| {
            mesh.positions
                .iter()
                .map(|p| p[axis])
                .fold(f32::MIN, f32::max)
        };
        assert!((hi(0) - 1.0).abs() < 1e-5 && (hi(2) - 0.5).abs() < 1e-5);
        for n in &mesh.normals {
            let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
            assert!((len - 1.0).abs() < 1e-5);
        }
    }
}
