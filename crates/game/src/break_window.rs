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
//! `d57b5c8ae3096614` cannot move). Errors: [`LookError`] from the look, [`String`] from the
//! upload; the window drops the picture on the first error and says so. Budget: at most the
//! tier's debris cap in bodies, 16 draws for the eight-chunk crate, no work per frame after
//! the pile sleeps.

use crate::break_look::{BreakLook, BreakProp, LookError, BREAK_MESH_ID_BASE};
use crate::TickOutcome;
use engine_arena::{crate_rest, ArenaConfig};
use engine_physfx::{Material, Stats, Tier};
use engine_render::present::{DrawItem, DrawList, MeshKey, Presenter};
use engine_render::v1::{cube, upload_mesh, MeshData, PbrMaterial};

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
    /// Boxes already on the GPU and the model they were baked with.
    sent: Vec<(MeshKey, [f32; 12])>,
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
        self.observed += 1;
        if !outcome.breaks.is_empty() {
            self.breaks = self
                .breaks
                .saturating_add(u32::try_from(outcome.breaks.len()).unwrap_or(u32::MAX));
            self.broke_at.get_or_insert(outcome.tick);
        }
        Ok(())
    }

    /// The boxes to draw now, `alpha` in `[0, 1]` blending the last two ticks. Before the
    /// first break event: the whole crate. From the break tick on: the chunks still standing
    /// and the debris, and never the whole crate.
    pub fn draws(&self, alpha: f32) -> Result<Vec<SceneDraw>, LookError> {
        let frame = self.look.frame(alpha)?;
        if frame.hidden.contains(&self.intact) {
            let halves: Vec<(MeshKey, [f32; 3])> = self
                .look
                .meshes()
                .iter()
                .flat_map(|m| [(m.standing.key, m.standing.half), (m.flying.key, m.flying.half)])
                .collect();
            return Ok(frame
                .draws
                .iter()
                .filter_map(|draw| {
                    let half = halves.iter().find(|(key, _)| *key == draw.mesh)?.1;
                    Some(SceneDraw {
                        mesh: draw.mesh,
                        half,
                        model: draw.model_3x4(),
                    })
                })
                .collect());
        }
        let [x, y, z] = self.center_m;
        Ok(vec![SceneDraw {
            mesh: self.intact,
            half: [self.half_m; 3],
            model: [1.0, 0.0, 0.0, x, 0.0, 1.0, 0.0, y, 0.0, 0.0, 1.0, z],
        }])
    }

    /// Uploads what moved since the last frame and pushes one draw per box onto `list`.
    /// Returns the draws pushed. An error means the picture is unusable: the caller drops the
    /// scene (the window keeps running without the crate).
    pub fn push(
        &mut self,
        presenter: &mut Presenter,
        list: &mut DrawList,
        alpha: f32,
    ) -> Result<usize, String> {
        let draws = self.draws(alpha).map_err(|err| err.to_string())?;
        for draw in &draws {
            let fresh = self
                .sent
                .iter()
                .position(|(key, _)| *key == draw.mesh)
                .map_or(true, |at| self.sent[at].1 != draw.model);
            if fresh {
                let mesh = baked_box(draw.half, &draw.model)?;
                presenter
                    .upload_mesh(draw.mesh, &mesh)
                    .map_err(|err| err.to_string())?;
                match self.sent.iter_mut().find(|(key, _)| *key == draw.mesh) {
                    Some(entry) => entry.1 = draw.model,
                    None => self.sent.push((draw.mesh, draw.model)),
                }
            }
            list.items.push(DrawItem {
                mesh: draw.mesh,
                material: WOOD,
                base_color: None,
                normal_map: None,
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

    /// Counters of the physical layer.
    #[must_use]
    pub fn stats(&self) -> Stats {
        self.look.stats()
    }

    /// The window's receipt line: `arena break: breaks=<n> debris=<n> awake=<n>`.
    #[must_use]
    pub fn receipt_line(&self) -> String {
        format!(
            "arena break: breaks={} debris={} awake={}",
            self.breaks,
            self.debris(),
            self.stats().awake
        )
    }
}

/// A box of half-extents `half` placed by `model` (3 rows of 4, row-major), as a mesh in world
/// space with outward normals: the renderer draws meshes where they were baked.
pub fn baked_box(half: [f32; 3], model: &[f32; 12]) -> Result<MeshData, String> {
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
            let r = turn([p[0] * 2.0 * half[0], p[1] * 2.0 * half[1], p[2] * 2.0 * half[2]]);
            [r[0] + model[3], r[1] + model[7], r[2] + model[11]]
        })
        .collect();
    let normals: Vec<[f32; 3]> = unit.normals.iter().map(|n| turn(*n)).collect();
    upload_mesh(&positions, &normals, &unit.uvs, &unit.indices).map_err(|err| format!("{err}"))
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

    fn scene_for(app: &App) -> BreakScene {
        BreakScene::new(&app.live_match().config()).expect("the crate bakes")
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
            let draws = scene.draws(1.0).expect("frame");
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
                    assert_eq!(draws.len(), chunk_count, "tick {tick}: every chunk, no more");
                    assert!(draws.iter().all(|d| d.mesh != intact), "tick {tick}: crate gone");
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
        assert_eq!(scene.debris(), freed_total, "one debris body per freed chunk");
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
        let cfg = GameConfig { ticks: 400, ..config() };
        let mut app = App::new(cfg).expect("app");
        let mut scene = scene_for(&app);
        let mut first: Option<Vec<SceneDraw>> = None;
        let mut asleep_at = None;
        for tick in 0..cfg.ticks {
            let out = app.tick(script(tick));
            scene.observe_outcome(&out).expect("tick");
            if tick == 4 {
                first = Some(scene.draws(0.0).expect("frame"));
            }
            if tick > 4 && asleep_at.is_none() && scene.stats().awake == 0 {
                asleep_at = Some(tick);
            }
        }
        let first = first.expect("break tick frame");
        let end = scene.draws(1.0).expect("frame");
        assert!(asleep_at.is_some(), "the pile sleeps inside {} ticks", cfg.ticks);
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
        assert_eq!(scene.draws(0.3).expect("frame"), end, "asleep is alpha independent");
    }

    /// F2P: the picture never touches the match. Two sessions play the same script, one with
    /// the break picture observing every tick and baking every frame; every checkpoint hash,
    /// the hp and every tick outcome are equal.
    #[test]
    fn break_wire_hashes_with_and_without_the_picture_are_equal() {
        let cfg = GameConfig { ticks: 600, ..config() };
        let mut bare = App::new(cfg).expect("app");
        let mut painted = App::new(cfg).expect("app");
        let mut scene = scene_for(&painted);
        for tick in 0..cfg.ticks {
            let a = bare.tick(script(tick));
            let b = painted.tick(script(tick));
            scene.observe_outcome(&b).expect("tick");
            for draw in scene.draws(0.5).expect("frame") {
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

    /// The baked box lands where the model puts it, with unit outward normals.
    #[test]
    fn break_wire_baked_box_is_where_the_model_says() {
        let identity = [1.0, 0.0, 0.0, 1.0, 0.0, 1.0, 0.0, 2.0, 0.0, 0.0, 1.0, 3.0];
        let mesh = baked_box([0.5, 0.25, 1.0], &identity).expect("bakes");
        let span = |axis: usize| {
            let lo = mesh.positions.iter().map(|p| p[axis]).fold(f32::MAX, f32::min);
            let hi = mesh.positions.iter().map(|p| p[axis]).fold(f32::MIN, f32::max);
            (lo, hi)
        };
        assert_eq!(span(0), (0.5, 1.5));
        assert_eq!(span(1), (1.75, 2.25));
        assert_eq!(span(2), (2.0, 4.0));
        // A quarter turn about Y swaps the X and Z extents of the same box.
        let turned = [0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, -1.0, 0.0, 0.0, 0.0];
        let mesh = baked_box([0.5, 0.25, 1.0], &turned).expect("bakes");
        let hi = |axis: usize| mesh.positions.iter().map(|p| p[axis]).fold(f32::MIN, f32::max);
        assert!((hi(0) - 1.0).abs() < 1e-5 && (hi(2) - 0.5).abs() < 1e-5);
        for n in &mesh.normals {
            let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
            assert!((len - 1.0).abs() < 1e-5);
        }
    }
}

