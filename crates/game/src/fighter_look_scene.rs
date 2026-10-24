//! The look's half of the scene report (`scene_report`): the posed fighters as
//! scene things, and a headless bout folded to one report.
//!
//! Each seen fighter becomes a [`Thing`] with the posed triangles and normals
//! the CPU skin already produced (so a ray test sees the body the window
//! draws), the named joints placed in the world, the parts that are skinned
//! rigidly to a hand joint (what the fighter holds), and sample points on the
//! surface. [`FighterLook::scene_run`] plays a seeded bout with no window and
//! no GPU, folds the view laws the way `--view-line` does, and reports the
//! last frame: the same bytes `view.laws` pops, plus the geometry.
//!
//! Consumer: the arena binary's `--scene-report` flag and its seat `scene`
//! line, and the door's `view.scene` verb through them. Units: meters,
//! radians for yaw. Determinism: the same inputs give the same report twice;
//! presentation only, the sim hash never reads it.

use engine_input::FighterId;
use engine_render::v1::Camera;

use super::{
    fighters_content_dir, root_matrix, CamFollow, FighterLook, LookError, LookFrame, PosedBody,
    Shot, LOOK_MESH_ID_BASE, MAX_LOOK_FIGHTERS, MAX_PARTS,
};
use crate::scene_report::geom::{unit, P3};
use crate::scene_report::things::{prop_things, spear_things};
use crate::scene_report::{
    self, FighterFacts, HeldPart, Joint, Kind, Laws, Light, Report, Sample, SceneInput, ShadowMap,
    Shape, Thing,
};
use crate::showcase::Showcase;
use crate::stage_bodies::place_joint;
use crate::view_truth::ViewLaws;
use crate::{App, GameConfig, TickOutcome, HUMAN_SLOT};

/// Surface points a fighter sends rays from.
const BODY_SAMPLES: usize = 64;
/// A part whose weight sits this much on one joint is rigid on it.
const RIGID_SHARE: f32 = 0.9;
/// The camera the view laws fold with: the one `--view-line` uses, so the
/// report's laws line is the line `view.laws` pops.
const LAWS_EYE: P3 = [0.0, 5.0, 12.0];

/// What the sim knows about a slot that the look does not.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SlotExtras {
    /// Hit points.
    pub hp: Option<u16>,
    /// The weapon row the slot is armed with.
    pub armed: Option<String>,
}

/// Who owns a drawn mesh, by the id of its mesh key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MeshOwner {
    /// A body part of this slot.
    Fighter(u8),
    /// A spear shape of this slot.
    Spear(u8),
    /// The ring floor.
    RingFloor,
    /// The ring wall and its tiers.
    RingTiers,
    /// The crowd.
    Crowd,
    /// Anything else the frame draws (capes, limbs, kit, particles).
    Other,
}

/// The owner of the mesh whose key id is `id`.
#[must_use]
pub fn mesh_owner(id: u64) -> MeshOwner {
    use crate::fight_ring::{RING_CROWD_KEY_IDS, RING_FLOOR_KEY_ID, RING_TIERS_KEY_ID};
    use crate::spear_window::SPEAR_MESH_ID_BASE;
    if id == RING_FLOOR_KEY_ID {
        return MeshOwner::RingFloor;
    }
    if id == RING_TIERS_KEY_ID {
        return MeshOwner::RingTiers;
    }
    if RING_CROWD_KEY_IDS.contains(&id) {
        return MeshOwner::Crowd;
    }
    let parts = u64::try_from(MAX_PARTS).unwrap_or(u64::MAX);
    let bodies = u64::try_from(MAX_LOOK_FIGHTERS).unwrap_or(0);
    if let Some(rel) = id
        .checked_sub(LOOK_MESH_ID_BASE)
        .filter(|r| *r < parts * bodies)
    {
        if let Ok(slot) = u8::try_from(rel / parts) {
            return MeshOwner::Fighter(slot);
        }
    }
    if let Some(rel) = id
        .checked_sub(SPEAR_MESH_ID_BASE)
        .filter(|r| *r < 4 * bodies)
    {
        if let Ok(slot) = u8::try_from(rel / 4) {
            return MeshOwner::Spear(slot);
        }
    }
    MeshOwner::Other
}

/// The scene id a mesh owner answers to: `fighter_N`, `spear_N`, the ring
/// props by their label in `things`, `crowd` or `other`.
#[must_use]
pub fn owner_id(owner: MeshOwner, things: &[Thing]) -> String {
    let labelled = |label: &str| {
        things
            .iter()
            .find(|t| t.kind == Kind::Prop && t.label == label)
            .map_or_else(|| "other".to_owned(), |t| t.id.clone())
    };
    match owner {
        MeshOwner::Fighter(slot) => format!("fighter_{slot}"),
        MeshOwner::Spear(slot) => format!("spear_{slot}"),
        MeshOwner::RingFloor => labelled("ring_floor"),
        MeshOwner::RingTiers => labelled("ring_tiers"),
        MeshOwner::Crowd => "crowd".to_owned(),
        MeshOwner::Other => "other".to_owned(),
    }
}

/// What a headless scene run plays.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SceneRunSpec {
    /// Match seed.
    pub seed: u64,
    /// Ticks to play before the report.
    pub ticks: u32,
    /// Fighters (the showcase always plays two).
    pub fighters: u8,
    /// Play the showcase tour: duel tables, spear, height, the kit world and
    /// the training dummy, like `arena --showcase`.
    pub showcase: bool,
    /// The named camera shot.
    pub shot: Shot,
    /// Picture size in pixels.
    pub viewport: (u32, u32),
}

/// A joint-family name's hand: `palm_r`, `wep_pos_l`, `thumb_upper_r` ...
fn hand_of(name: &str) -> Option<char> {
    let lower = name.to_ascii_lowercase();
    const FAMILY: [&str; 8] = [
        "palm_", "wep_pos_", "hand_", "thumb", "index", "middle", "ring", "pinky",
    ];
    if !FAMILY.iter().any(|f| lower.starts_with(f)) {
        return None;
    }
    match lower.chars().last() {
        Some(side @ ('r' | 'l')) if lower.ends_with(&format!("_{side}")) => Some(side),
        _ => None,
    }
}

/// The hand a body part is skinned to, when at least [`RIGID_SHARE`] of its
/// weight sits on joints of that one hand.
fn held_hand(rig: &super::Rig, part: &super::RigPart) -> Option<char> {
    let mut weight = vec![0.0f32; rig.skeleton.joints.len()];
    let mut total = 0.0f32;
    for (joints, weights) in part.skin.joints.iter().zip(&part.skin.weights) {
        for (joint, w) in joints.iter().zip(weights) {
            if let Some(slot) = weight.get_mut(usize::from(*joint)) {
                *slot += *w;
                total += *w;
            }
        }
    }
    if total <= 0.0 {
        return None;
    }
    let (mut right, mut left) = (0.0f32, 0.0f32);
    for (joint, w) in rig.skeleton.joints.iter().zip(&weight) {
        match hand_of(&joint.name) {
            Some('r') => right += *w,
            Some('l') => left += *w,
            _ => {}
        }
    }
    if right / total >= RIGID_SHARE {
        Some('r')
    } else if left / total >= RIGID_SHARE {
        Some('l')
    } else {
        None
    }
}

/// Bounds, center and longest span of a point set, or `None` when empty.
fn bounds_of(points: &[P3]) -> Option<((P3, P3), P3, f32)> {
    let first = *points.first()?;
    let (mut lo, mut hi) = (first, first);
    for p in points {
        for k in 0..3 {
            lo[k] = lo[k].min(p[k]);
            hi[k] = hi[k].max(p[k]);
        }
    }
    let span = (0..3).map(|k| hi[k] - lo[k]).fold(0.0f32, f32::max);
    Some((
        (lo, hi),
        std::array::from_fn(|k| (lo[k] + hi[k]) * 0.5),
        span,
    ))
}

impl FighterLook {
    /// Every seen fighter as a scene thing, posed at `alpha` between the last
    /// two ticks. `extras` is indexed by slot.
    ///
    /// # Errors
    ///
    /// [`LookError`] when a seen slot has no rig.
    pub fn scene_fighters(
        &self,
        fighters: usize,
        alpha: f32,
        extras: &[SlotExtras],
    ) -> Result<Vec<Thing>, LookError> {
        let posed = self.pose_frame(fighters, alpha)?;
        posed
            .iter()
            .map(|body| self.scene_fighter(body, extras.get(usize::from(body.truth.slot))))
            .collect()
    }

    /// One posed body as a thing: triangles, normals, joints and held parts.
    fn scene_fighter(
        &self,
        body: &PosedBody,
        extra: Option<&SlotExtras>,
    ) -> Result<Thing, LookError> {
        let truth = &body.truth;
        let slot = usize::from(truth.slot);
        let rig = self
            .rigs
            .get(self.rig_for_slot(slot))
            .ok_or_else(|| LookError::Skin("rig missing".to_string()))?;
        let placed = root_matrix(truth.root, truth.yaw);
        let joints: Vec<Joint> = rig
            .skeleton
            .joints
            .iter()
            .zip(&truth.joints)
            .map(|(joint, at)| Joint {
                name: joint.name.clone(),
                at: place_joint(*at, &placed),
                parent: joint.parent,
            })
            .collect();
        let mut positions: Vec<P3> = Vec::new();
        let mut normals: Vec<P3> = Vec::new();
        let mut indices: Vec<u32> = Vec::new();
        let mut held = Vec::new();
        for (index, (frame, part)) in body.frames.iter().zip(&rig.parts).enumerate() {
            let base = u32::try_from(positions.len()).unwrap_or(u32::MAX);
            let world: Vec<P3> = frame
                .positions
                .iter()
                .map(|p| [p[0], p[1] + body.lift, p[2]])
                .collect();
            if let (Some(hand), Some((bounds, at, len_m))) =
                (held_hand(rig, part), bounds_of(&world))
            {
                held.push(HeldPart {
                    name: format!("part_{index}"),
                    hand,
                    at,
                    bounds,
                    len_m,
                });
            }
            positions.extend(world);
            normals.extend(frame.normals.iter().copied());
            indices.extend(part.mesh.indices.iter().map(|i| i.saturating_add(base)));
        }
        let step = (positions.len() / BODY_SAMPLES).max(1);
        let samples: Vec<Sample> = positions
            .iter()
            .zip(&normals)
            .step_by(step)
            .map(|(at, normal)| Sample {
                at: *at,
                normal: unit(*normal),
            })
            .collect();
        let bounds = truth.has_bounds().then_some((truth.min, truth.max));
        Ok(Thing {
            id: format!("fighter_{}", truth.slot),
            kind: Kind::Fighter,
            label: format!("slot {}", truth.slot),
            bounds,
            shape: Shape::Mesh { positions, indices },
            drawn: true,
            samples,
            joints,
            fighter: Some(FighterFacts {
                slot: truth.slot,
                act: truth.act.to_owned(),
                clip: truth.clip,
                tinted: truth.tinted,
                ragdolled: truth.ragdolled,
                hp: extra.and_then(|e| e.hp),
                root: truth.root,
                yaw: truth.yaw,
                armed: extra.and_then(|e| e.armed.clone()),
                foot_slide_m: truth.foot_slide_m,
                error_parts: truth.error_parts,
                nonfinite: truth.nonfinite,
                held,
                held_known: true,
            }),
            spear: None,
        })
    }

    /// Play a seeded bout with no window and report the last frame: bodies,
    /// props, spears, the director's camera, the light and the view laws. The
    /// laws line is the line [`FighterLook::view_run`] folds for the same
    /// seed, ticks and fighters (the plain bout) or the showcase fold.
    ///
    /// # Errors
    ///
    /// [`LookError`] when the session or the bodies cannot load.
    pub fn scene_run(spec: SceneRunSpec) -> Result<Report, LookError> {
        let mut run = SceneRun::start(spec)?;
        for tick in 0..spec.ticks {
            if !run.tick(tick)? {
                break;
            }
        }
        run.report()
    }
}

/// One headless run in flight: the sim, the look, the spear scene and the laws.
struct SceneRun {
    spec: SceneRunSpec,
    app: App,
    look: FighterLook,
    scene: crate::spear_window::SpearScene,
    show: Option<Showcase>,
    laws: ViewLaws,
    laws_camera: Camera,
    out: TickOutcome,
    human: FighterId,
    count: usize,
}

fn bind(err: crate::GameError) -> LookError {
    LookError::Bind(format!("{err:?}"))
}

impl SceneRun {
    /// The session the window would play: the showcase's duel, spear, height
    /// and kit world, or the plain bout `view_run` folds.
    fn start(spec: SceneRunSpec) -> Result<Self, LookError> {
        let fighters = if spec.showcase { 2 } else { spec.fighters };
        let config = GameConfig {
            seed: spec.seed,
            ticks: spec.ticks,
            fighters,
            half_extent_m: crate::windowed::ARENA_HALF_EXTENT_M,
        };
        let app = if spec.showcase {
            App::with_world(config, crate::arena_kit::kit_match_world_or_empty())
                .and_then(App::into_duel)
                .and_then(App::into_spear)
                .and_then(App::into_noose)
                .and_then(App::into_height)
        } else {
            App::new(config)
        }
        .map_err(bind)?;
        Ok(Self {
            spec,
            app,
            look: FighterLook::load(&fighters_content_dir())?,
            scene: crate::spear_window::SpearScene::new(),
            show: spec.showcase.then(|| Showcase::new(config.half_extent_m)),
            laws: ViewLaws::default(),
            laws_camera: Camera::new(
                LAWS_EYE,
                [0.0, 1.0, 0.0],
                [0.0, 1.0, 0.0],
                50.0f32.to_radians(),
            ),
            out: TickOutcome::default(),
            human: FighterId::new(HUMAN_SLOT).ok_or_else(|| LookError::Bind("slot".to_string()))?,
            count: usize::from(fighters),
        })
    }

    /// One tick in the window's own order, then the laws fold; false when the
    /// sim refused the input and the run ends early.
    fn tick(&mut self, tick: u32) -> Result<bool, LookError> {
        if self.show.is_some() {
            if !self.showcase_tick(tick)? {
                return Ok(false);
            }
        } else {
            self.out = self.app.tick_empty();
            self.look.observe_tick(
                tick,
                self.app.fighter_views(),
                self.app.hp(),
                &self.out.cues,
            );
        }
        self.look
            .observe_live_balances(self.app.live_match(), self.count);
        let bodies: Vec<_> = self
            .look
            .pose_frame(self.count, 0.5)?
            .into_iter()
            .map(|posed| posed.truth)
            .collect();
        self.laws
            .note(&self.laws_camera, f64::from(tick) + 0.5, &bodies);
        if self.spec.showcase {
            let (spears, ropes) = self.scene.counts();
            self.laws.note_spear(spears, ropes);
        }
        Ok(true)
    }

    /// The showcase tour's tick: the dummy is offered, the sim ticks, the tour
    /// grades, then the look takes the hitstop, landings, tick and spears.
    fn showcase_tick(&mut self, tick: u32) -> Result<bool, LookError> {
        let Some(show) = self.show.as_mut() else {
            return Ok(true);
        };
        let input = show.input(
            tick,
            self.human,
            self.app.fighter_views(),
            self.app.live_match().spear_view(),
        );
        if let Some(dummy) = show.rival_input(tick, HUMAN_SLOT, self.app.fighter_views()) {
            self.app.offer_remote(dummy).map_err(bind)?;
        }
        self.app.tick_into(input, &mut self.out);
        if !self.out.input_accepted {
            return Ok(false);
        }
        let live = self.app.live_match();
        show.observe(
            tick,
            HUMAN_SLOT,
            live.spear_view(),
            live.spear_events(),
            &self.out.cues,
        );
        let (views, hp) = (self.app.fighter_views(), self.app.hp());
        self.look
            .observe_hitstop(tick, &self.out.cues, self.app.hitstop_remaining());
        self.look.observe_landings(tick, views);
        self.look.observe_landing_rows(tick, &self.out.lands);
        self.look.observe_tick(tick, views, hp, &self.out.cues);
        self.scene
            .observe_tick(&mut self.look, self.app.live_match(), views);
        Ok(true)
    }

    /// The report of the last frame the run drew.
    fn report(&self) -> Result<Report, LookError> {
        let spec = self.spec;
        let extras: Vec<SlotExtras> = (0..self.count)
            .filter_map(|slot| u8::try_from(slot).ok())
            .map(|slot| SlotExtras {
                hp: self.app.hp().get(usize::from(slot)).copied(),
                armed: self.app.weapon_name(slot).map(str::to_owned),
            })
            .collect();
        let alpha = 1.0;
        let mut things = self.look.scene_fighters(self.count, alpha, &extras)?;
        things.extend(prop_things(spec.showcase, false));
        if spec.showcase {
            things.extend(spear_things(
                &self.scene,
                self.app.live_match().spear_view(),
                alpha,
            ));
        }
        let (w, h) = spec.viewport;
        let base = crate::windowed::dressing_camera(w, h)
            .map_err(|err| LookError::Bind(format!("{err}")))?;
        let mut follow = CamFollow::new()
            .map(CamFollow::with_look_ahead)
            .map_err(|err| LookError::Bind(format!("fight camera: {err}")))?;
        let frame = LookFrame {
            fighters: self.count,
            alpha,
            shot: spec.shot,
            frame: spec.ticks,
        };
        let camera = self.look.aim_camera(&mut follow, &base, &frame, 1.0 / 60.0);
        let input = scene_input(
            if spec.showcase {
                "headless_showcase"
            } else {
                "headless"
            },
            spec,
            camera,
            things,
            ViewLawsFold {
                line: self.laws.line().unwrap_or_default(),
                verdict: self.laws.verdict(),
                warnings: self.laws.warnings(),
            },
        );
        Ok(scene_report::build(&input))
    }
}

/// The folded view laws of a run.
struct ViewLawsFold {
    line: String,
    verdict: String,
    warnings: Vec<String>,
}

/// The headless input around the posed things: the window's default light, the
/// shipped sun map and no live renderer.
fn scene_input(
    source: &'static str,
    spec: SceneRunSpec,
    camera: Camera,
    things: Vec<Thing>,
    fold: ViewLawsFold,
) -> SceneInput {
    let light = engine_render::v1::default_scene(spec.viewport.0.max(1), spec.viewport.1.max(1))
        .map(|scene| scene.light)
        .ok();
    let sun = light
        .as_ref()
        .and_then(|l| unit(l.direction.to_array()))
        .unwrap_or([0.0, 1.0, 0.0]);
    let facts = engine_render::present::SunShadowFacts::shipped();
    SceneInput {
        source,
        tick: spec.ticks,
        seed: spec.seed,
        viewport: spec.viewport,
        camera,
        shot: shot_name(spec.shot).to_owned(),
        mode: "director".to_owned(),
        light: Light {
            sun_dir: sun,
            color: light.as_ref().map_or([1.0; 3], |l| l.color.to_array()),
            intensity: light.as_ref().map_or(1.0, |l| l.intensity),
            sky_sun_dir: unit(engine_render::arena_day::ARENA_DAY_SUN_DIR),
        },
        shadow: ShadowMap {
            on: facts.on,
            edge_px: facts.edge_px,
            half_extent_m: facts.half_extent_m,
            texel_m: facts.texel_m,
            view_proj: engine_render::present::sun_shadow_view_proj(sun),
        },
        contact: 1.0,
        passes: Vec::new(),
        things,
        laws: Laws {
            line: fold.line,
            verdict: fold.verdict,
            warnings: fold.warnings,
        },
        pixels: None,
        renderer_live: false,
    }
}

/// The wire name of a shot.
#[must_use]
pub fn shot_name(shot: Shot) -> &'static str {
    match shot {
        Shot::Hero => "hero",
        Shot::Wide => "wide",
        Shot::Action => "action",
    }
}

/// The shot a wire name selects.
#[must_use]
pub fn shot_named(name: &str) -> Option<Shot> {
    match name {
        "hero" => Some(Shot::Hero),
        "wide" => Some(Shot::Wide),
        "action" => Some(Shot::Action),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(ticks: u32, showcase: bool) -> SceneRunSpec {
        SceneRunSpec {
            seed: 2040,
            ticks,
            fighters: 2,
            showcase,
            shot: Shot::Action,
            viewport: (1280, 720),
        }
    }

    #[test]
    fn a_mesh_key_names_its_owner() {
        use crate::fight_ring::{RING_CROWD_KEY_IDS, RING_FLOOR_KEY_ID, RING_TIERS_KEY_ID};
        use crate::spear_window::SPEAR_MESH_ID_BASE;
        let part = |slot: usize, part: usize| {
            u64::try_from(slot * MAX_PARTS + part).expect("small") + LOOK_MESH_ID_BASE
        };
        assert_eq!(mesh_owner(part(0, 0)), MeshOwner::Fighter(0));
        assert_eq!(mesh_owner(part(3, 15)), MeshOwner::Fighter(3));
        assert_eq!(mesh_owner(SPEAR_MESH_ID_BASE + 9), MeshOwner::Spear(2));
        assert_eq!(mesh_owner(RING_FLOOR_KEY_ID), MeshOwner::RingFloor);
        assert_eq!(mesh_owner(RING_TIERS_KEY_ID), MeshOwner::RingTiers);
        assert_eq!(mesh_owner(RING_CROWD_KEY_IDS[2]), MeshOwner::Crowd);
        assert_eq!(mesh_owner(42), MeshOwner::Other);
        let props = crate::scene_report::things::prop_things(false, true);
        assert_eq!(owner_id(MeshOwner::RingFloor, &props), props[0].id);
        assert_eq!(owner_id(MeshOwner::RingTiers, &props), props[1].id);
        assert_eq!(owner_id(MeshOwner::Fighter(1), &props), "fighter_1");
    }

    #[test]
    fn hands_are_named_from_the_joint_family() {
        assert_eq!(hand_of("palm_r"), Some('r'));
        assert_eq!(hand_of("wep_pos_L"), Some('l'));
        assert_eq!(hand_of("thumb_upper_r"), Some('r'));
        assert_eq!(hand_of("pelvis"), None);
        assert_eq!(hand_of("arm_lower_r"), None);
        assert_eq!(hand_of("palm"), None, "a palm with no side is nobody's");
    }

    #[test]
    fn the_showcase_run_reports_the_tour_its_props_and_names_what_it_finds() {
        let a = FighterLook::scene_run(spec(360, true)).expect("showcase scene run");
        let b = FighterLook::scene_run(spec(360, true)).expect("showcase scene run again");
        assert_eq!(a, b, "the same tour reports the same bytes");
        println!("{}", a.text);
        assert!(a.json.contains("\"source\":\"headless_showcase\""));
        for id in ["fighter_0", "fighter_1"] {
            assert!(a.json.contains(&format!("\"id\":\"{id}\"")), "{id}");
        }
        assert!(a.text.contains("showcase tour"), "{}", a.text);
        assert!(a.text.contains("sun elevation"), "{}", a.text);
        let kit = crate::arena_kit::kit_prop_rows();
        if !kit.is_empty() {
            assert!(
                a.text.contains("NOT DRAWN"),
                "the kit props the match collides with are not drawn under the bound look"
            );
            assert!(
                a.findings
                    .iter()
                    .any(|f| f.contains("draws nothing for it")),
                "{:?}",
                a.findings
            );
        }
        assert!(
            a.findings
                .iter()
                .any(|f| f.contains("forward light and the sky sun")),
            "the window light and the sky sun disagree: {:?}",
            a.findings
        );
    }

    #[test]
    fn a_headless_bout_reports_both_fighters_deterministically_with_the_view_laws_line() {
        let a = FighterLook::scene_run(spec(120, false)).expect("scene run");
        let b = FighterLook::scene_run(spec(120, false)).expect("scene run again");
        assert_eq!(a, b, "the same bout reports the same bytes");
        for slot in 0..2 {
            assert!(
                a.json.contains(&format!("\"id\":\"fighter_{slot}\"")),
                "fighter_{slot}"
            );
        }
        assert!(a.json.contains("\"source\":\"headless\""));
        let line = FighterLook::view_run(2040, 120, 2).expect("view line");
        assert!(
            a.json.contains(&format!("\"line\":\"{line}\"")),
            "laws line {line}"
        );
        assert!(a.text.contains("passes:"), "{}", a.text);
    }
}
