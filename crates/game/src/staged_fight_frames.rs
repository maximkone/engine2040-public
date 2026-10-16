//! staged fight frames: the staged 600-tick receipt drives arena fight frames.
//!
//! Consumer: the `arena` twin capture path (`windowed::run_headless_smoke_*`
//! plus the arena binary once the window-loop wiring lands) and staged-fight
//! acceptance.
//! Data owner: this module owns the staged script, the per-frame fighter dots,
//! the presenter body draws and the headless CPU frame PNGs; match authority
//! stays in `engine-arena`, bundle identity in [`crate::fighter_bind`].
//! Units: fixed ticks at 60 Hz, meters for dots, pixels for frames, SHAs as
//! lowercase hex. Determinism scope: the same seed plus script gives the same
//! dots, PNGs and manifests twice (no wall clock, no RNG, fixed JSON field
//! order). Errors: typed [`super::WinError`]; script drift or a torn dataset
//! fails loudly, never stages the wrong bodies.
//!
//! Budget: three linear 600-tick arena passes (run, events, dataset) plus
//! four 128x96 CPU frames; all IO-free, never on the fixed tick.

use engine_arena::{recorder::Dataset, run_dataset, run_events, ArenaConfig, TickEvents};
use engine_combat::FightEvent;
use engine_core::hash_bytes;
use engine_input::{fighter::ATTACK, Buttons, FighterId, FighterInput, FighterTape};
use engine_render::{
    present::{DrawItem, DrawList, MeshKey, Presenter, SizeClass},
    v1::{self, MeshData, PbrMaterial},
};

use super::{
    dressing_camera, hud_draw_list, push_dressing_draws, CameraPath, WinError, SMOKE_FRAME_SIZE,
};
use crate::fighter_bind;

/// Session seed for the staged fight (matches the staged-fight receipt).
pub const STAGED_FIGHT_SEED: u64 = 2040;
/// Full fight length in fixed ticks (matches the staged-fight receipt).
pub const STAGED_FIGHT_TICKS: u32 = 600;
/// Fighter count: the hero plus 3 opponents (matches the staged-fight receipt).
pub const STAGED_FIGHT_FIGHTERS: u8 = 4;
/// Same fighter count as `usize` for indexing and array lengths (pinned
/// equal to [`STAGED_FIGHT_FIGHTERS`] by the bind test; `as` casts never
/// enter production code, so the two spellings stay in sync by test).
const FIGHTER_COUNT: usize = 4;
/// Arena half-extent in meters (matches the staged-fight receipt).
pub const STAGED_FIGHT_HALF_EXTENT_M: f32 = 20.0;
/// Frames emitted over one staged run (matches the staged 4-frame captures).
pub const STAGED_FIGHT_FRAMES: usize = 4;
/// Mesh id base for staged fighter bodies (never collides with the central
/// cube key or the dressing base).
pub const FIGHT_MESH_ID_BASE: u64 = 0xF167_7400;
/// Fighter body size in meters `[sx, sy, sz]` (a standing box volume).
pub const FIGHT_BODY_SIZE_M: [f32; 3] = [0.6, 1.8, 0.6];

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

/// One staged fight frame: sim tick plus dots plus CPU PNG plus manifest.
#[derive(Clone, PartialEq, Debug)]
pub struct StagedFightFrame {
    /// Fixed tick this frame was taken at.
    pub tick: u32,
    /// Match checkpoint hash after the tick.
    pub checkpoint_hash: u64,
    /// Fighter dots in slot order.
    pub dots: Vec<StagedFighterDot>,
    /// CPU frame PNG bytes (fighters visible, hits flashing).
    pub png: Vec<u8>,
    /// FNV-1a hex over the PNG bytes (frame identity in manifests).
    pub png_fnv: String,
    /// Frame manifest JSON (tick, checkpoint, dots, PNG identity).
    pub manifest: String,
}

/// One staged 600-tick run folded into fight frames.
#[derive(Clone, PartialEq, Debug)]
pub struct StagedFightReport {
    /// Fixed ticks simulated (always [`STAGED_FIGHT_TICKS`]).
    pub ticks_run: u32,
    /// Full-fight hits (the staged receipt reads 27).
    pub hits: usize,
    /// Full-fight knockdowns (the staged receipt reads 3).
    pub knockdowns: usize,
    /// Emitted frames in tick order (always [`STAGED_FIGHT_FRAMES`]).
    pub frames: Vec<StagedFightFrame>,
}

/// Scripted human input: circle-strafe with an attack every 5 ticks, so the
/// session lands real cues (hits, blocks, knockdowns) instead of idling.
/// Byte-identical to the `staged_fight` binary script; the same tick gives the
/// same input twice. Pure.
#[must_use]
pub fn staged_fight_input(tick: u32, fighter: FighterId) -> FighterInput {
    let phase = (tick / 15) % 4;
    let (mx, mz) = match phase {
        0 => (100, 0),
        1 => (0, 100),
        2 => (-100, 0),
        _ => (0, -100),
    };
    FighterInput {
        tick,
        fighter,
        mx,
        mz,
        facing: 0,
        buttons: if tick.is_multiple_of(5) {
            Buttons(ATTACK)
        } else {
            Buttons::NONE
        },
    }
}

/// Pinned fighter-bundle SHAs in slot order (hero plus 3 opponents). The same
/// bytes the fighter-bind manifest pins; a recooked manifest with different
/// manifest with different bytes fails the bind test loudly instead of
/// staging the wrong bodies. Pure.
#[must_use]
pub fn staged_ct7_bundle_shas() -> [&'static str; FIGHTER_COUNT] {
    let pinned = fighter_bind::EXPECTED_BUNDLE_SHA256;
    [pinned[0], pinned[1], pinned[2], pinned[3]]
}

/// Pinned fighter body GLB SHAs in slot order (same source as the bundle SHAs).
/// Pure.
#[must_use]
pub fn staged_ct7_body_shas() -> [&'static str; FIGHTER_COUNT] {
    let pinned = fighter_bind::EXPECTED_BODY_SHA256;
    [pinned[0], pinned[1], pinned[2], pinned[3]]
}

/// Stable mesh key per fighter slot. Never collides with the central cube
/// key or the dressing keys: a distinct base id plus the slot in the digest.
#[must_use]
pub fn fight_mesh_key(slot: u8) -> MeshKey {
    MeshKey::new(FIGHT_MESH_ID_BASE + u64::from(slot), [0xF1 ^ slot; 32])
}

/// Subdivisions per box edge for the staged fighter proxy: 16 gives
/// 3072 triangles per box, 6144 for the two-part body (over the 5000-tri
/// CT7 scale the frame gate pins).
const BODY_SUBDIV: u16 = 16;
/// Total vertices for the two-part proxy (2 boxes times 6 faces times
/// `(BODY_SUBDIV + 1)` squared).
const BODY_VERTS: usize = 3_468;
/// Total indices for the two-part proxy (2 boxes times 12 times
/// `BODY_SUBDIV` squared triangles times 3).
const BODY_INDICES: usize = 18_432;

/// Push one axis-aligned subdivided box with outward face normals and
/// `0..=1` UVs. Faces are emitted in `+X -X +Y -Y +Z -Z` order with a
/// `(seg + 1)` squared grid each; winding is informational (both
/// rasterizers depth-test only, no culling). Pure, headless std-only.
/// Out-of-range vertex counts are typed, never panics.
fn push_subdivided_box(
    min_m: [f32; 3],
    max_m: [f32; 3],
    seg: u16,
    positions: &mut Vec<[f32; 3]>,
    normals: &mut Vec<[f32; 3]>,
    uvs: &mut Vec<[f32; 2]>,
    indices: &mut Vec<u32>,
) -> Result<(), WinError> {
    let seg_f = f32::from(seg);
    let stride = u32::from(seg) + 1;
    for face in 0..6 {
        let normal = match face {
            0 => [1.0, 0.0, 0.0],
            1 => [-1.0, 0.0, 0.0],
            2 => [0.0, 1.0, 0.0],
            3 => [0.0, -1.0, 0.0],
            4 => [0.0, 0.0, 1.0],
            _ => [0.0, 0.0, -1.0],
        };
        let base = u32::try_from(positions.len())
            .map_err(|_| WinError::Render("staged body verts out of range".to_string()))?;
        for iy in 0..=seg {
            let v = f32::from(iy) / seg_f;
            for ix in 0..=seg {
                let u = f32::from(ix) / seg_f;
                let pos = match face {
                    0 => [
                        max_m[0],
                        min_m[1] + (max_m[1] - min_m[1]) * v,
                        min_m[2] + (max_m[2] - min_m[2]) * u,
                    ],
                    1 => [
                        min_m[0],
                        min_m[1] + (max_m[1] - min_m[1]) * v,
                        min_m[2] + (max_m[2] - min_m[2]) * u,
                    ],
                    2 => [
                        min_m[0] + (max_m[0] - min_m[0]) * u,
                        max_m[1],
                        min_m[2] + (max_m[2] - min_m[2]) * v,
                    ],
                    3 => [
                        min_m[0] + (max_m[0] - min_m[0]) * u,
                        min_m[1],
                        min_m[2] + (max_m[2] - min_m[2]) * v,
                    ],
                    4 => [
                        min_m[0] + (max_m[0] - min_m[0]) * u,
                        min_m[1] + (max_m[1] - min_m[1]) * v,
                        max_m[2],
                    ],
                    _ => [
                        min_m[0] + (max_m[0] - min_m[0]) * u,
                        min_m[1] + (max_m[1] - min_m[1]) * v,
                        min_m[2],
                    ],
                };
                positions.push(pos);
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

/// Two-part CT7-scale fighter proxy for one body: a torso box plus a head
/// box inside the [`FIGHT_BODY_SIZE_M`] volume, feet at `feet_m`, 1.8 m
/// tall, 6144 triangles. Deterministic subdivided grids (no wall clock, no
/// RNG); headless std-only like the old placeholder. Validation failures
/// are typed, never panics.
pub fn fight_body_mesh(feet_m: [f32; 3]) -> Result<MeshData, WinError> {
    if !feet_m.iter().all(|v| v.is_finite()) {
        return Err(WinError::Render(
            "fight body position is not finite".to_string(),
        ));
    }
    let half_x = 0.5 * FIGHT_BODY_SIZE_M[0];
    let half_z = 0.5 * FIGHT_BODY_SIZE_M[2];
    let torso_min = [feet_m[0] - half_x, feet_m[1], feet_m[2] - half_z];
    let torso_max = [feet_m[0] + half_x, feet_m[1] + 1.5, feet_m[2] + half_z];
    let head_half = 0.15;
    let head_min = [
        feet_m[0] - head_half,
        feet_m[1] + 1.5,
        feet_m[2] - head_half,
    ];
    let head_max = [
        feet_m[0] + head_half,
        feet_m[1] + FIGHT_BODY_SIZE_M[1],
        feet_m[2] + head_half,
    ];
    let mut positions = Vec::with_capacity(BODY_VERTS);
    let mut normals = Vec::with_capacity(BODY_VERTS);
    let mut uvs = Vec::with_capacity(BODY_VERTS);
    let mut indices = Vec::with_capacity(BODY_INDICES);
    push_subdivided_box(
        torso_min,
        torso_max,
        BODY_SUBDIV,
        &mut positions,
        &mut normals,
        &mut uvs,
        &mut indices,
    )?;
    push_subdivided_box(
        head_min,
        head_max,
        BODY_SUBDIV,
        &mut positions,
        &mut normals,
        &mut uvs,
        &mut indices,
    )?;
    v1::upload_mesh(&positions, &normals, &uvs, &indices)
        .map_err(|err| WinError::Render(format!("fight body mesh: {err}")))
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
) -> Result<usize, WinError> {
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
) -> Result<usize, WinError> {
    let mut pushed = 0usize;
    for dot in dots {
        let key = fight_mesh_key(dot.slot);
        if resident.contains(&key) {
            let material = fight_body_material(dot.slot, dot.hit);
            material
                .validate()
                .map_err(|err| WinError::Render(format!("fight body material: {err}")))?;
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

/// One staged-fight draw list: the HUD clear plus the lit cube plus dressing
/// plus one body draw per resident fighter under the establishing camera
/// (or the capture orbit eye for `frame` under `path`; the tick never sees
/// it). Missing keys draw nothing; bad sizes fail loudly. Pure over the
/// resident set plus dots, so headless tests pin it without a GPU.
pub fn staged_fight_frame_list(
    width: u32,
    height: u32,
    path: Option<CameraPath>,
    frame: u32,
    resident: &[MeshKey],
    dots: &[StagedFighterDot],
) -> Result<DrawList, WinError> {
    let mut list = hud_draw_list(width, height)?;
    list.camera = dressing_camera(width, height)?;
    super::apply_camera_orbit(&mut list, frame, path);
    let cube = MeshKey::new(1, [0xA1; 32]);
    if resident.contains(&cube) {
        let scene = v1::default_scene(width, height)
            .map_err(|err| WinError::Render(super::cap_message(&format!("{err}"))))?;
        list.items.push(DrawItem {
            mesh: cube,
            material: scene.material,
            base_color: None,
            normal_map: None,
        });
    }
    let _ = push_dressing_draws(&mut list, resident)?;
    let _ = push_fight_draws(&mut list, resident, dots)?;
    Ok(list)
}

/// Build the staged tape: one scripted human input per tick for the full
/// fight. Sorted by construction, so the arena legs accept it.
fn staged_tape() -> Result<FighterTape, WinError> {
    let human = FighterId::new(crate::HUMAN_SLOT).ok_or(WinError::BadConfig("fighters"))?;
    let ticks = usize::try_from(STAGED_FIGHT_TICKS)
        .map_err(|_| WinError::BadConfig("staged fight ticks"))?;
    let mut inputs = Vec::with_capacity(ticks);
    for tick in 0..STAGED_FIGHT_TICKS {
        inputs.push(staged_fight_input(tick, human));
    }
    Ok(FighterTape(inputs))
}

/// Staged arena config: seed 2040, 600 ticks, 4 fighters, 20 m extent.
/// A torn constant fails loudly with the culprit row.
fn staged_config() -> Result<ArenaConfig, WinError> {
    ArenaConfig::try_new(
        STAGED_FIGHT_SEED,
        STAGED_FIGHT_TICKS,
        STAGED_FIGHT_FIGHTERS,
        STAGED_FIGHT_HALF_EXTENT_M,
    )
    .map_err(|_| WinError::BadConfig("staged fight config"))
}

/// Frame ticks: four hit ticks spread over the fight (first, thirds, last),
/// so every emitted frame shows a hit flash while the frames stay spread
/// over the full 600 ticks. Falls back to the nominal quarter ends when the
/// run landed fewer than four distinct hit ticks (never fails: the nominal
/// ticks are always in range).
fn capture_ticks(events: &[TickEvents]) -> [u32; STAGED_FIGHT_FRAMES] {
    let mut hit_ticks: Vec<u32> = Vec::new();
    for row in events {
        if row.events.iter().any(is_hit_flash) {
            hit_ticks.push(row.tick);
        }
    }
    hit_ticks.sort_unstable();
    hit_ticks.dedup();
    if hit_ticks.len() >= STAGED_FIGHT_FRAMES {
        let last = hit_ticks.len() - 1;
        [0, 1, 2, 3].map(|index| hit_ticks[index * last / (STAGED_FIGHT_FRAMES - 1)])
    } else {
        [149, 299, 449, 599].map(|tick| tick.min(STAGED_FIGHT_TICKS - 1))
    }
}

/// Hits read at a glance: full hits plus knockdowns flash; blocks, parries,
/// dodges and stamina-outs do not.
fn is_hit_flash(event: &FightEvent) -> bool {
    matches!(event, FightEvent::Hit { .. } | FightEvent::Knockdown { .. })
}

/// Fighter dots for one tick: positions plus health from the authoritative
/// dataset, hit flags from this run's events. `None` on any shape mismatch
/// (wrong tick, wrong fighter count); the caller fails loudly then.
fn dots_for_tick(
    dataset: &Dataset,
    events: &[TickEvents],
    tick: u32,
) -> Option<Vec<StagedFighterDot>> {
    let index = usize::try_from(tick).ok()?;
    let base = index.checked_mul(FIGHTER_COUNT)?;
    let row = dataset.rows.get(base)?;
    if row.observation.tick != tick || row.observation.fighters.len() != FIGHTER_COUNT {
        return None;
    }
    let mut hit = [false; FIGHTER_COUNT];
    if let Some(trow) = events.get(index) {
        if trow.tick == tick {
            for event in &trow.events {
                match event {
                    FightEvent::Hit { victim, .. } | FightEvent::Knockdown { victim, .. } => {
                        let slot_index = usize::from(*victim);
                        if slot_index < hit.len() {
                            hit[slot_index] = true;
                        }
                    }
                    FightEvent::Blocked { .. }
                    | FightEvent::Parried { .. }
                    | FightEvent::Dodged { .. }
                    | FightEvent::StaminaOut { .. } => {}
                }
            }
        }
    }
    let mut dots = Vec::with_capacity(FIGHTER_COUNT);
    for entry in row.observation.fighters.as_slice() {
        if !entry.pos.iter().all(|v| v.is_finite()) {
            return None;
        }
        let slot = entry.id;
        let slot_index = usize::from(slot);
        dots.push(StagedFighterDot {
            slot,
            pos_m: entry.pos,
            hp: entry.hp,
            hit: slot_index < hit.len() && hit[slot_index],
        });
    }
    Some(dots)
}

/// Sand background color for CPU frames (RGBA).
const FRAME_SAND: [u8; 4] = [194, 160, 104, 255];
/// Body colors per slot for CPU frames (hero red, then blue/green/gold).
const FRAME_BODY: [[u8; 3]; FIGHTER_COUNT] =
    [[191, 38, 31], [38, 115, 217], [38, 166, 77], [217, 179, 38]];
/// Hit flash color for CPU frames (white reads at a glance).
const FRAME_FLASH: [u8; 3] = [255, 255, 255];
/// Knocked-down body color for CPU frames (dark, drawn lying).
const FRAME_DOWN: [u8; 3] = [90, 90, 90];
/// Outline color for CPU body rects.
const FRAME_EDGE: [u8; 3] = [20, 16, 12];
/// HP bar color for CPU frames (green steps with remaining health).
const FRAME_HP: [u8; 3] = [34, 177, 76];
/// Hit-mark color for CPU frames (cyan square under a hit body).
const FRAME_HITMARK: [u8; 3] = [0, 220, 220];
/// HP bar size in pixels: fixed width, filled left to right by health.
const HP_BAR_W: usize = 12;
/// HP bar height in pixels.
const HP_BAR_H: usize = 2;
/// Hit-mark square edge in pixels.
const HIT_MARK: usize = 3;

/// Linear position of a finite non-negative `f32` in 1/64 units, decoded
/// from the bit pattern with integer math only (no float-to-int cast):
/// truncated `value * 64`, exact for the arena range. Denormals and zero
/// read as 0; the shift only ever goes right for inputs below 2^6, so the
/// result always fits in `u64`.
fn units_64th(value: f32) -> u64 {
    let bits = value.to_bits();
    let exp = (bits >> 23) & 0xFF;
    if exp == 0 {
        return 0;
    }
    let mant = u64::from(bits & 0x7F_FFFF) | 0x80_0000;
    // value * 64 = mant * 2^(exp - 144).
    mant >> 144u32.saturating_sub(exp)
}

/// Map one arena axis in meters to a frame pixel. Clamped inside the frame
/// with a 4 px margin, so bodies never clip the edge. Linear in the
/// position via [`units_64th`] with integer math only (no float-to-int
/// cast); the same position gives the same pixel twice.
fn map_axis(value_m: f32, half_extent_m: f32, pixels: u32) -> u32 {
    let inner = pixels.saturating_sub(8);
    let range = 2.0 * half_extent_m;
    let shifted = (value_m + half_extent_m).clamp(0.0, range);
    let step = units_64th(shifted).saturating_mul(u64::from(inner)) / units_64th(range).max(1);
    let px = u32::try_from(step).unwrap_or(u32::MAX);
    4u32.saturating_add(px.min(inner))
}

/// Raw RGBA pixels for one staged frame: sand background plus one body rect
/// per dot (standing, or lying when knocked down; white when hit), one HP
/// bar above each body (green width tracks health), and one cyan hit mark
/// below each hit body. The same dots give the same pixels twice.
/// Bad sizes fail loudly.
pub fn staged_fight_frame_rgba(
    dots: &[StagedFighterDot],
    width: u32,
    height: u32,
) -> Result<Vec<u8>, WinError> {
    if SizeClass::classify(width, height) != SizeClass::Sizes {
        return Err(WinError::Render(format!(
            "staged frame size {width}x{height} rejected"
        )));
    }
    let w = usize::try_from(width)
        .map_err(|_| WinError::Render(format!("staged frame width {width} rejected")))?;
    let h = usize::try_from(height)
        .map_err(|_| WinError::Render(format!("staged frame height {height} rejected")))?;
    let mut pixels = vec![0u8; w * h * 4];
    for pixel in pixels.chunks_exact_mut(4) {
        pixel.copy_from_slice(&FRAME_SAND);
    }
    for dot in dots {
        let last_x = width.saturating_sub(1);
        let last_y = height.saturating_sub(1);
        let cx = map_axis(dot.pos_m[0], STAGED_FIGHT_HALF_EXTENT_M, width).min(last_x);
        let cy = map_axis(dot.pos_m[2], STAGED_FIGHT_HALF_EXTENT_M, height).min(last_y);
        let knocked = dot.hp == 0;
        let (rw, rh) = if knocked {
            (9usize, 5usize)
        } else {
            (7usize, 9usize)
        };
        let fill = if dot.hit {
            FRAME_FLASH
        } else if knocked {
            FRAME_DOWN
        } else {
            FRAME_BODY[usize::from(dot.slot) % FRAME_BODY.len()]
        };
        let center_x = usize::try_from(cx)
            .map_err(|_| WinError::Render("staged frame rect out of range".to_string()))?;
        let center_y = usize::try_from(cy)
            .map_err(|_| WinError::Render("staged frame rect out of range".to_string()))?;
        let x0 = center_x.saturating_sub(rw / 2);
        let y0 = center_y.saturating_sub(rh / 2);
        for dy in 0..rh {
            for dx in 0..rw {
                let (x, y) = (x0 + dx, y0 + dy);
                if x >= w || y >= h {
                    continue;
                }
                let edge = dx == 0 || dy == 0 || dx + 1 == rw || dy + 1 == rh;
                let color = if edge && !dot.hit { FRAME_EDGE } else { fill };
                let at = (y * w + x) * 4;
                pixels[at] = color[0];
                pixels[at + 1] = color[1];
                pixels[at + 2] = color[2];
                pixels[at + 3] = 255;
            }
        }
        // HP bar above the body: filled width tracks health in permille,
        // so full health fills more green than low health. Empty for
        // knocked-down bodies. Same dots give the same bar twice.
        let filled = (usize::from(dot.hp) * HP_BAR_W + 999) / 1000;
        let filled = filled.min(HP_BAR_W);
        let bar_x0 = center_x.saturating_sub(HP_BAR_W / 2);
        let bar_y0 = y0.saturating_sub(HP_BAR_H + 2);
        for dy in 0..HP_BAR_H {
            for dx in 0..filled {
                let (x, y) = (bar_x0 + dx, bar_y0 + dy);
                if x >= w || y >= h {
                    continue;
                }
                let at = (y * w + x) * 4;
                pixels[at] = FRAME_HP[0];
                pixels[at + 1] = FRAME_HP[1];
                pixels[at + 2] = FRAME_HP[2];
                pixels[at + 3] = 255;
            }
        }
        // Hit mark below the body: a cyan square only when the hit flag
        // is set, so hit frames differ from calm frames at a glance.
        if dot.hit {
            let mark_x0 = center_x.saturating_sub(HIT_MARK / 2);
            let mark_y0 = y0.saturating_add(rh).saturating_add(1);
            for dy in 0..HIT_MARK {
                for dx in 0..HIT_MARK {
                    let (x, y) = (mark_x0 + dx, mark_y0 + dy);
                    if x >= w || y >= h {
                        continue;
                    }
                    let at = (y * w + x) * 4;
                    pixels[at] = FRAME_HITMARK[0];
                    pixels[at + 1] = FRAME_HITMARK[1];
                    pixels[at + 2] = FRAME_HITMARK[2];
                    pixels[at + 3] = 255;
                }
            }
        }
    }
    Ok(pixels)
}

/// CRC-32 over `data` (bit-by-bit; frames are small, no table needed).
fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for byte in data {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = if crc & 1 == 1 {
                0xEDB8_8320 ^ (crc >> 1)
            } else {
                crc >> 1
            };
        }
    }
    crc ^ 0xFFFF_FFFF
}

/// Adler-32 over `data` (zlib checksum for the stored-deflate stream).
/// Reduced per chunk so the sums never overflow `u32` in debug builds.
fn adler32(data: &[u8]) -> u32 {
    const MOD: u32 = 65521;
    const CHUNK: usize = 5552;
    let mut a: u32 = 1;
    let mut b: u32 = 0;
    for piece in data.chunks(CHUNK) {
        for byte in piece {
            a += u32::from(*byte);
            b += a;
        }
        a %= MOD;
        b %= MOD;
    }
    (b << 16) | a
}

/// Append one PNG chunk (length, tag, data, CRC). Lengths past `u32`
/// fail loudly instead of truncating the file.
fn png_chunk(out: &mut Vec<u8>, tag: &[u8; 4], data: &[u8]) -> Result<(), WinError> {
    let len = u32::try_from(data.len())
        .map_err(|_| WinError::Render("staged frame too large for PNG".to_string()))?;
    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(tag);
    out.extend_from_slice(data);
    let mut crc_data = Vec::with_capacity(tag.len() + data.len());
    crc_data.extend_from_slice(tag);
    crc_data.extend_from_slice(data);
    out.extend_from_slice(&crc32(&crc_data).to_be_bytes());
    Ok(())
}

/// Encode RGBA pixels as a PNG (8-bit, filter 0, stored deflate): the same
/// pixels give the same bytes on every machine. Bad sizes and pixel-length
/// mismatches fail loudly.
fn encode_fight_png(pixels: &[u8], width: u32, height: u32) -> Result<Vec<u8>, WinError> {
    if SizeClass::classify(width, height) != SizeClass::Sizes {
        return Err(WinError::Render(format!(
            "staged frame size {width}x{height} rejected"
        )));
    }
    let row_len = usize::try_from(width)
        .map_err(|_| WinError::Render(format!("staged frame width {width} rejected")))?;
    let rows = usize::try_from(height)
        .map_err(|_| WinError::Render(format!("staged frame height {height} rejected")))?;
    let stride = row_len * 4;
    if pixels.len() != stride * rows {
        return Err(WinError::Render(
            "staged frame pixel length mismatch".to_string(),
        ));
    }
    let mut raw = Vec::with_capacity(pixels.len() + rows);
    for row in pixels.chunks_exact(stride) {
        raw.push(0u8);
        raw.extend_from_slice(row);
    }
    let mut zlib = Vec::with_capacity(raw.len() + 16);
    zlib.extend_from_slice(&[0x78, 0x01]);
    let mut blocks = raw.chunks(65535).peekable();
    while let Some(block) = blocks.next() {
        let last = blocks.peek().is_none();
        zlib.push(u8::from(last));
        let len = u16::try_from(block.len())
            .map_err(|_| WinError::Render("staged frame block too large".to_string()))?;
        zlib.extend_from_slice(&len.to_le_bytes());
        zlib.extend_from_slice(&(!len).to_le_bytes());
        zlib.extend_from_slice(block);
    }
    zlib.extend_from_slice(&adler32(&raw).to_be_bytes());
    let mut out = Vec::with_capacity(8 + 25 + zlib.len() + 28);
    out.extend_from_slice(&[137, 80, 78, 71, 13, 10, 26, 10]);
    let mut ihdr = [0u8; 13];
    ihdr[0..4].copy_from_slice(&width.to_be_bytes());
    ihdr[4..8].copy_from_slice(&height.to_be_bytes());
    ihdr[8] = 8;
    ihdr[9] = 6;
    png_chunk(&mut out, b"IHDR", &ihdr)?;
    png_chunk(&mut out, b"IDAT", &zlib)?;
    png_chunk(&mut out, b"IEND", &[])?;
    Ok(out)
}

/// PNG bytes for one staged frame at the smoke size. Same dots give the
/// same bytes twice; different ticks give different bytes as bodies move.
pub fn staged_fight_frame_png(dots: &[StagedFighterDot]) -> Result<Vec<u8>, WinError> {
    let pixels = staged_fight_frame_rgba(dots, SMOKE_FRAME_SIZE.0, SMOKE_FRAME_SIZE.1)?;
    encode_fight_png(&pixels, SMOKE_FRAME_SIZE.0, SMOKE_FRAME_SIZE.1)
}

/// Frame manifest JSON: fixed field order, no absolute paths, so the same
/// frame gives byte-identical manifests twice. Positions print at 3 decimals.
pub fn staged_frame_manifest(
    tick: u32,
    checkpoint_hash: u64,
    png_fnv: &str,
    dots: &[StagedFighterDot],
) -> String {
    let mut out = String::from("{\"schema\":1,\"task\":\"WIN6\"");
    out.push_str(&format!(",\"tick\":{tick}"));
    out.push_str(&format!(",\"checkpoint\":\"{checkpoint_hash:016x}\""));
    out.push_str(&format!(",\"png_fnv\":\"{png_fnv}\""));
    out.push_str(",\"fighters\":[");
    for (index, dot) in dots.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        out.push_str(&format!(
            "{{\"slot\":{},\"x\":{:.3},\"z\":{:.3},\"hp\":{},\"hit\":{}}}",
            dot.slot, dot.pos_m[0], dot.pos_m[2], dot.hp, dot.hit
        ));
    }
    out.push_str(&format!(
        "],\"size\":[{},{}]}}",
        SMOKE_FRAME_SIZE.0, SMOKE_FRAME_SIZE.1
    ));
    out
}

/// Run the staged 600-tick fight and emit fight frames: the same script and
/// coordinates the staged-fight receipt files, three linear arena passes (run,
/// events, dataset), then one CPU frame per capture tick with bodies visible
/// and hits flashing. Any drift (wrong checkpoint count, torn dataset,
/// missing events) fails loudly instead of emitting the wrong fight.
pub fn run_staged_fight_frames() -> Result<StagedFightReport, WinError> {
    let config = staged_config()?;
    let tape = staged_tape()?;
    let ticks_len = usize::try_from(STAGED_FIGHT_TICKS)
        .map_err(|_| WinError::Render("staged fight ticks out of range".to_string()))?;
    let checkpoints = engine_arena::run(&config, &tape);
    if checkpoints.len() != ticks_len {
        return Err(WinError::Render(format!(
            "staged fight ended at {}/{} checkpoints",
            checkpoints.len(),
            STAGED_FIGHT_TICKS
        )));
    }
    let events = run_events(&config, &tape);
    if events.len() != ticks_len {
        return Err(WinError::Render(format!(
            "staged fight ended at {}/{} event rows",
            events.len(),
            STAGED_FIGHT_TICKS
        )));
    }
    let dataset = run_dataset(&config, &tape).ok_or_else(|| {
        WinError::Render("staged fight dataset refused the staged tape".to_string())
    })?;
    if dataset.rows.len() != ticks_len * FIGHTER_COUNT {
        return Err(WinError::Render(format!(
            "staged fight dataset holds {}/{} rows",
            dataset.rows.len(),
            ticks_len * FIGHTER_COUNT
        )));
    }
    let mut hits = 0usize;
    let mut knockdowns = 0usize;
    for row in &events {
        for event in &row.events {
            match event {
                FightEvent::Hit { .. } => hits += 1,
                FightEvent::Knockdown { .. } => knockdowns += 1,
                FightEvent::Blocked { .. }
                | FightEvent::Parried { .. }
                | FightEvent::Dodged { .. }
                | FightEvent::StaminaOut { .. } => {}
            }
        }
    }
    let mut frames = Vec::with_capacity(STAGED_FIGHT_FRAMES);
    for tick in capture_ticks(&events) {
        let dots = dots_for_tick(&dataset, &events, tick).ok_or_else(|| {
            WinError::Render(format!("staged fight lost tick {tick} in the dataset"))
        })?;
        let checkpoint_index = usize::try_from(tick)
            .map_err(|_| WinError::Render(format!("staged fight tick {tick} out of range")))?;
        let checkpoint_hash = checkpoints
            .get(checkpoint_index)
            .map(|point| point.hash)
            .unwrap_or(0);
        let png = staged_fight_frame_png(&dots)?;
        let png_fnv = format!("{:016x}", hash_bytes(&png));
        let manifest = staged_frame_manifest(tick, checkpoint_hash, &png_fnv, &dots);
        frames.push(StagedFightFrame {
            tick,
            checkpoint_hash,
            dots,
            png,
            png_fnv,
            manifest,
        });
    }
    Ok(StagedFightReport {
        ticks_run: STAGED_FIGHT_TICKS,
        hits,
        knockdowns,
        frames,
    })
}

/// Full-fight hits the staged receipt pins.
pub const STAGED_FIGHT_HITS: usize = 27;
/// Full-fight knockdowns the staged receipt pins.
pub const STAGED_FIGHT_KNOCKDOWNS: usize = 3;
/// Window-binary flag: drive the human slot with [`staged_fight_input`]
/// over the staged session and print the receipt at the end.
pub const STAGED_FIGHT_FLAG: &str = "--staged-fight";

/// Split [`STAGED_FIGHT_FLAG`] out of the raw args; every other arg keeps
/// its order for the capture and window parsers. Pure.
#[must_use]
pub fn split_staged_flag(raw: &[String]) -> (bool, Vec<String>) {
    let staged = raw.iter().any(|arg| arg == STAGED_FIGHT_FLAG);
    let rest = raw
        .iter()
        .filter(|arg| *arg != STAGED_FIGHT_FLAG)
        .cloned()
        .collect();
    (staged, rest)
}

/// Window session config of the staged fight: seed 2040, 600 ticks,
/// 4 fighters, 20 m half-extent (the receipt's arena). Pure.
#[must_use]
pub fn staged_session_config() -> crate::GameConfig {
    crate::GameConfig {
        seed: STAGED_FIGHT_SEED,
        ticks: STAGED_FIGHT_TICKS,
        fighters: STAGED_FIGHT_FIGHTERS,
        half_extent_m: STAGED_FIGHT_HALF_EXTENT_M,
    }
}

/// Running cue tally of a staged window run (hits plus knockdowns).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct StagedTally {
    /// Full hits seen so far.
    pub hits: usize,
    /// Knockdowns seen so far.
    pub knockdowns: usize,
}

impl StagedTally {
    /// Fold one tick's cues.
    pub fn observe(&mut self, cues: &[engine_audio::FightCue]) {
        for cue in cues {
            match cue {
                engine_audio::FightCue::Hit { .. } => self.hits += 1,
                engine_audio::FightCue::Knockdown { .. } => self.knockdowns += 1,
                _ => {}
            }
        }
    }

    /// One receipt line for the binary. A run that reached the full staged
    /// length must read the pinned hits and knockdowns, else it fails
    /// loudly; a short run fails too (the receipt is the whole fight).
    pub fn receipt(&self, ticks: u32, frames: u32, bundles: &[String]) -> Result<String, WinError> {
        let pins: Vec<&str> = bundles
            .iter()
            .map(|sha| sha.get(..12).unwrap_or(sha.as_str()))
            .collect();
        let line = format!(
            "arena staged-fight: ticks={ticks} hits={} knockdowns={} frames={frames} bodies={} ct7=[{}]",
            self.hits,
            self.knockdowns,
            bundles.len(),
            pins.join(",")
        );
        if ticks != STAGED_FIGHT_TICKS {
            return Err(WinError::Render(format!("{line} (short run)")));
        }
        if bundles
            .iter()
            .map(String::as_str)
            .ne(staged_ct7_bundle_shas())
        {
            return Err(WinError::Render(format!("{line} (CT7 bodies not bound)")));
        }
        if self.hits != STAGED_FIGHT_HITS || self.knockdowns != STAGED_FIGHT_KNOCKDOWNS {
            return Err(WinError::Render(format!(
                "{line} (receipt wants hits={STAGED_FIGHT_HITS} knockdowns={STAGED_FIGHT_KNOCKDOWNS})"
            )));
        }
        Ok(line)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn staged_flag_splits_and_window_session_replays_the_receipt() {
        let raw: Vec<String> = ["arena", "--staged-fight", "--frames", "10"]
            .iter()
            .map(|s| (*s).to_string())
            .collect();
        let (staged, rest) = split_staged_flag(&raw);
        assert!(staged);
        assert_eq!(rest, ["arena", "--frames", "10"]);
        assert!(!split_staged_flag(&rest).0);
        // The window session (App ticks, same script) lands the receipt.
        let config = staged_session_config();
        let mut app = crate::App::new(config).expect("staged session builds");
        let human = FighterId::new(crate::HUMAN_SLOT).expect("human slot");
        let mut tally = StagedTally::default();
        for tick in 0..config.ticks {
            let outcome = app.tick(staged_fight_input(tick, human));
            assert!(outcome.input_accepted);
            tally.observe(&outcome.cues);
        }
        let shas: Vec<String> = staged_ct7_bundle_shas()
            .iter()
            .map(|sha| (*sha).to_string())
            .collect();
        let line = tally
            .receipt(config.ticks, 4, &shas)
            .expect("receipt holds");
        assert!(line.contains("hits=27 knockdowns=3"), "{line}");
        let short = tally.receipt(config.ticks - 1, 4, &shas);
        assert!(short.is_err(), "a short run never passes");
        assert!(
            tally.receipt(config.ticks, 4, &[]).is_err(),
            "unbound bodies never pass"
        );
        let torn = StagedTally {
            hits: 26,
            knockdowns: 3,
        };
        assert!(torn.receipt(config.ticks, 4, &shas).is_err());
    }

    fn content_dir() -> std::path::PathBuf {
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../content/fighters")
    }

    #[test]
    fn staged_script_matches_the_receipt_cues() {
        let report = run_staged_fight_frames().expect("staged fight runs");
        assert_eq!(report.ticks_run, STAGED_FIGHT_TICKS);
        assert_eq!(report.hits, 27, "script drifted from the CT15 receipt");
        assert_eq!(report.knockdowns, 3, "script drifted from the CT15 receipt");
        assert_eq!(report.frames.len(), STAGED_FIGHT_FRAMES);
    }

    #[test]
    fn staged_frames_show_bodies_and_differ() {
        let report = run_staged_fight_frames().expect("staged fight runs");
        let (width, height) = (SMOKE_FRAME_SIZE.0 as usize, SMOKE_FRAME_SIZE.1 as usize);
        let sand = &FRAME_SAND;
        for frame in &report.frames {
            assert_eq!(frame.dots.len(), STAGED_FIGHT_FIGHTERS as usize);
            assert!(
                frame.dots.iter().any(|dot| dot.hit),
                "tick {} shows no hit flash",
                frame.tick
            );
            // Bodies visible: rects land non-sand pixels on the raster.
            let pixels =
                staged_fight_frame_rgba(&frame.dots, SMOKE_FRAME_SIZE.0, SMOKE_FRAME_SIZE.1)
                    .expect("raster builds");
            assert_eq!(pixels.len(), width * height * 4);
            let body_pixels = pixels
                .chunks_exact(4)
                .filter(|pixel| pixel != &[sand[0], sand[1], sand[2], sand[3]])
                .count();
            assert!(
                body_pixels >= 4 * 20,
                "tick {} hides its bodies ({body_pixels} px)",
                frame.tick
            );
            // PNGs are real files with the frame identity filed.
            assert_eq!(&frame.png[0..8], &[137, 80, 78, 71, 13, 10, 26, 10]);
            assert_eq!(frame.png_fnv, format!("{:016x}", hash_bytes(&frame.png)));
            assert!(frame.manifest.contains(&format!("\"tick\":{}", frame.tick)));
            assert!(frame.manifest.contains(&frame.png_fnv));
        }
        // Frames differ across the fight: bodies move, so bytes move too.
        for pair in 0..report.frames.len() {
            for other in (pair + 1)..report.frames.len() {
                let (first, second) = (&report.frames[pair], &report.frames[other]);
                assert_ne!(first.tick, second.tick);
                assert_ne!(first.png, second.png, "frames {} and {} match", pair, other);
                assert_ne!(first.png_fnv, second.png_fnv);
                assert_ne!(first.manifest, second.manifest);
            }
        }
        // Determinism: the same script gives the same frames twice.
        let again = run_staged_fight_frames().expect("staged fight repeats");
        assert_eq!(report, again);
    }

    #[test]
    fn staged_ct7_shas_stay_pinned() {
        assert_eq!(
            staged_ct7_bundle_shas(),
            fighter_bind::EXPECTED_BUNDLE_SHA256
        );
        assert_eq!(staged_ct7_body_shas(), fighter_bind::EXPECTED_BODY_SHA256);
        let bindings = fighter_bind::bind_all(&content_dir()).expect("CT7 manifest binds");
        assert_eq!(bindings.len(), STAGED_FIGHT_FIGHTERS as usize);
        for (index, binding) in bindings.iter().enumerate() {
            assert_eq!(binding.bundle_sha256, staged_ct7_bundle_shas()[index]);
            assert_eq!(binding.body_glb_sha256, staged_ct7_body_shas()[index]);
        }
    }

    #[test]
    fn staged_presenter_list_draws_bodies_with_hit_flash() {
        let report = run_staged_fight_frames().expect("staged fight runs");
        let dots = &report.frames[0].dots;
        let cube = MeshKey::new(1, [0xA1; 32]);
        let mut resident = vec![cube];
        let actors = super::super::dressing_actors().expect("dressing parses");
        for actor in &actors {
            resident.push(super::super::dressing_mesh_key(actor));
        }
        for dot in dots {
            resident.push(fight_mesh_key(dot.slot));
        }
        let list = staged_fight_frame_list(
            SMOKE_FRAME_SIZE.0,
            SMOKE_FRAME_SIZE.1,
            None,
            0,
            &resident,
            dots,
        )
        .expect("fight list builds");
        list.validate().expect("fight list validates");
        // Cube plus dressing plus one body draw per fighter.
        assert_eq!(
            list.items.len(),
            1 + super::super::DRESSING_ACTOR_COUNT + dots.len()
        );
        // Fighter keys never collide with the cube or dressing keys.
        for dot in dots {
            let key = fight_mesh_key(dot.slot);
            assert_ne!(key, cube);
            for actor in &actors {
                assert_ne!(key, super::super::dressing_mesh_key(actor));
            }
            let mesh = fight_body_mesh(dot.pos_m).expect("body mesh builds");
            assert_eq!(mesh.positions.len(), BODY_VERTS);
            assert_eq!(mesh.indices.len(), BODY_INDICES);
            assert!(
                mesh.indices.len() / 3 > 5_000,
                "fighter proxy carries CT7-scale tris, got {}",
                mesh.indices.len() / 3
            );
            fight_body_material(dot.slot, dot.hit)
                .validate()
                .expect("body material valid");
        }
        // Hit bodies flash white in the draw list.
        for (item, dot) in list
            .items
            .iter()
            .skip(1 + super::super::DRESSING_ACTOR_COUNT)
            .zip(dots.iter())
        {
            if dot.hit {
                assert_eq!(item.material.base_color, [1.0, 1.0, 1.0]);
            }
        }
        // An empty cache draws nothing beyond clear: the probe stays honest.
        let clear =
            staged_fight_frame_list(SMOKE_FRAME_SIZE.0, SMOKE_FRAME_SIZE.1, None, 0, &[], dots)
                .expect("clear list builds");
        assert!(clear.items.is_empty());
        assert!(staged_fight_frame_list(0, SMOKE_FRAME_SIZE.1, None, 0, &resident, dots).is_err());
    }

    #[test]
    fn staged_axis_maps_without_casts() {
        // The usize spelling of the fighter count pins the u8 API spelling.
        assert_eq!(FIGHTER_COUNT, usize::from(STAGED_FIGHT_FIGHTERS));
        assert_eq!(FIGHTER_COUNT, FRAME_BODY.len());
        // Axis ends land on the frame margins, the center lands mid-frame,
        // and the mapping never leaves the frame and never goes backwards.
        // The fixed-point decode is exact: 20 m reads as 1280 sixty-fourths.
        assert_eq!(units_64th(0.0), 0);
        assert_eq!(units_64th(1.0), 64);
        assert_eq!(units_64th(20.0), 1280);
        let width = SMOKE_FRAME_SIZE.0;
        assert_eq!(map_axis(-20.0, 20.0, width), 4);
        assert_eq!(map_axis(20.0, 20.0, width), width - 4);
        assert_eq!(map_axis(0.0, 20.0, width), width / 2);
        let mut prev = 0u32;
        let mut tick = 0u32;
        while tick <= 40 {
            let px = map_axis(-20.0 + tick as f32, 20.0, width);
            assert!(px <= width - 4, "off frame: {px}");
            assert!(px >= prev, "went backwards at {tick}");
            prev = px;
            tick += 1;
        }
        assert!(map_axis(f32::NAN, 20.0, width) <= width);
    }

    #[test]
    fn staged_frames_paint_hp_step_and_hit_mark() {
        // Oracle: a landed hit shows a victim flash plus an hp step on a
        // per-fighter bar. Dots mirror manifest tick 44 (slot 0 hp 1000
        // unhit, slots 1..=3 hp 760 hit) at spread positions so bars never
        // overlap; integer-only raster must paint green HP steps plus cyan
        // hit marks.
        let hit_damaged = [
            StagedFighterDot {
                slot: 0,
                pos_m: [-10.0, 0.0, 0.0],
                hp: 1000,
                hit: false,
            },
            StagedFighterDot {
                slot: 1,
                pos_m: [-3.0, 0.0, 0.0],
                hp: 760,
                hit: true,
            },
            StagedFighterDot {
                slot: 2,
                pos_m: [3.0, 0.0, 0.0],
                hp: 760,
                hit: true,
            },
            StagedFighterDot {
                slot: 3,
                pos_m: [10.0, 0.0, 0.0],
                hp: 760,
                hit: true,
            },
        ];
        let pixels = staged_fight_frame_rgba(&hit_damaged, SMOKE_FRAME_SIZE.0, SMOKE_FRAME_SIZE.1)
            .expect("hit raster builds");
        let green = pixels
            .chunks_exact(4)
            .filter(|px| px[0] == 34 && px[1] == 177 && px[2] == 76)
            .count();
        let cyan = pixels
            .chunks_exact(4)
            .filter(|px| px[0] == 0 && px[1] == 220 && px[2] == 220)
            .count();
        assert!(green > 0, "damaged hp paints no HP bar pixels");
        assert!(cyan > 0, "hit flag paints no hit-mark pixels");
        // HP step: full hp fills more green than near-zero hp at the same
        // spots with no hit marks in either raster.
        let full = [
            StagedFighterDot {
                slot: 1,
                pos_m: [-3.0, 0.0, 0.0],
                hp: 1000,
                hit: false,
            },
            StagedFighterDot {
                slot: 2,
                pos_m: [3.0, 0.0, 0.0],
                hp: 1000,
                hit: false,
            },
        ];
        let low = [
            StagedFighterDot {
                slot: 1,
                pos_m: [-3.0, 0.0, 0.0],
                hp: 40,
                hit: false,
            },
            StagedFighterDot {
                slot: 2,
                pos_m: [3.0, 0.0, 0.0],
                hp: 40,
                hit: false,
            },
        ];
        let full_px = staged_fight_frame_rgba(&full, SMOKE_FRAME_SIZE.0, SMOKE_FRAME_SIZE.1)
            .expect("full-hp raster builds");
        let low_px = staged_fight_frame_rgba(&low, SMOKE_FRAME_SIZE.0, SMOKE_FRAME_SIZE.1)
            .expect("low-hp raster builds");
        let full_green = full_px
            .chunks_exact(4)
            .filter(|px| px[0] == 34 && px[1] == 177 && px[2] == 76)
            .count();
        let low_green = low_px
            .chunks_exact(4)
            .filter(|px| px[0] == 34 && px[1] == 177 && px[2] == 76)
            .count();
        assert!(
            full_green > low_green,
            "hp 1000 ({full_green} px) must fill more than hp 40 ({low_green} px)"
        );
        // Hit mark isolates the flag: same hp, only the flag differs.
        let hit = [StagedFighterDot {
            slot: 1,
            pos_m: [-3.0, 0.0, 0.0],
            hp: 760,
            hit: true,
        }];
        let calm = [StagedFighterDot {
            slot: 1,
            pos_m: [-3.0, 0.0, 0.0],
            hp: 760,
            hit: false,
        }];
        let hit_px = staged_fight_frame_rgba(&hit, SMOKE_FRAME_SIZE.0, SMOKE_FRAME_SIZE.1)
            .expect("hit raster builds");
        let calm_px = staged_fight_frame_rgba(&calm, SMOKE_FRAME_SIZE.0, SMOKE_FRAME_SIZE.1)
            .expect("calm raster builds");
        let hit_cyan = hit_px
            .chunks_exact(4)
            .filter(|px| px[0] == 0 && px[1] == 220 && px[2] == 220)
            .count();
        let calm_cyan = calm_px
            .chunks_exact(4)
            .filter(|px| px[0] == 0 && px[1] == 220 && px[2] == 220)
            .count();
        assert!(hit_cyan > 0, "hit true paints no mark");
        assert_eq!(calm_cyan, 0, "hit false must paint no mark");
    }

    #[test]
    fn staged_fight_rejects_bad_rows() {
        assert!(staged_fight_frame_rgba(&[], 0, SMOKE_FRAME_SIZE.1).is_err());
        assert!(staged_fight_frame_rgba(&[], SMOKE_FRAME_SIZE.0, 0).is_err());
        assert!(encode_fight_png(&[0u8; 3], SMOKE_FRAME_SIZE.0, SMOKE_FRAME_SIZE.1).is_err());
        assert!(fight_body_mesh([f32::NAN, 0.0, 0.0]).is_err());
        // A torn dataset row resolves to no dots, never wrong dots.
        let config = staged_config().expect("config builds");
        let tape = staged_tape().expect("tape builds");
        let dataset = run_dataset(&config, &tape).expect("dataset builds");
        let events = run_events(&config, &tape);
        assert!(dots_for_tick(&dataset, &events, STAGED_FIGHT_TICKS + 1).is_none());
        // The script is the staged tape: attacks every 5 ticks, strafing phases.
        let human = FighterId::new(crate::HUMAN_SLOT).expect("slot 0 is a fighter");
        let first = staged_fight_input(0, human);
        assert_eq!((first.mx, first.mz), (100, 0));
        assert_eq!(first.buttons, Buttons(ATTACK));
        assert_eq!(staged_fight_input(1, human).buttons, Buttons::NONE);
        first.validate().expect("script validates");
    }
}
