//! spear_window: the spear grapple in the playable window. It draws the spear, its trail and the
//! rope, follows a tethered pair with the camera and says what to play.
//!
//! [`SpearScene`] wraps [`SpearLook`] (what the picture is) with the window's needs: one
//! [`SpearScene::observe`] per accepted tick, one [`SpearScene::push`] per drawn frame, a
//! [`SpearScene::frame`] camera follow, and [`spear_cue`] mapping an authority event onto a cue of
//! the existing audio bank (no new samples). The renderer's draws carry no model transform, so
//! every mesh is baked in world space and uploaded under a fixed key each frame (a handful of
//! small meshes), the way `break_window` does.
//!
//! Caller: `bin/arena.rs`. Units: meters, Y up. Presentation only: nothing here is hashed and the
//! sim never reads it. Errors: [`SceneError`]; the window drops the scene on the first one and
//! keeps running. Budget: at most one spear (two meshes), one rope and one trail per slot.

use crate::fighter_look::{ArmState, FighterLook, MAX_LOOK_FIGHTERS};
use crate::lock_on::{bearing_cdeg, whole};
use crate::spear_look::{Carry, SpearDraw, SpearLook, Ties, DEFAULT_TIER, SPEAR_LEN_M};
use crate::view_truth::audit::SpearTruth;
use crate::FighterView;
use engine_arena::spear::{Phase, SpearEvent, SpearSlot};
use engine_arena::LiveMatch;
use engine_core::vec3::cross3;
use engine_input::fighter::THROW_SPEAR;
use engine_input::{Buttons, FighterId, FighterInput};
use engine_render::present::{DrawItem, DrawList, MeshKey, PresentError, Presenter};
use engine_render::v1::{upload_mesh, Camera, MeshData, MeshError, PbrMaterial};

/// Mesh ids of the spear parts: four per slot.
pub const SPEAR_MESH_ID_BASE: u64 = 0x5BEA_0000;
/// Digest every spear mesh key carries (the content changes under a fixed key).
const DIGEST: [u8; 32] = [0x5B; 32];
const SIDES: usize = 8;

/// A small count as a float; shapes here hold a few dozen points.
fn num(n: usize) -> f32 {
    f32::from(u16::try_from(n).unwrap_or(u16::MAX))
}

/// A vertex index for the mesh index buffer.
fn idx(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

mod parts;
mod rope;

use engine_physfx::{TetherState, Tier};
use parts::{head_mesh, shaft_mesh};
use rope::{Lay, RopeStyle};

/// Radius of the streak behind a flying spear, meters.
const TRAIL_R_M: f32 = 0.02;
/// A spear in a hand stands this far in front of the feet, meters, tilted forward, when the
/// look cannot say where the palm is.
const HELD_FORWARD_M: f32 = 0.45;
const HELD_FOOT_Y_M: f32 = 0.15;
/// How far the palm grips the shaft from its tail, meters.
const HELD_GRIP_M: f32 = 0.6;
/// Height of the shaft's axis when a fallen holder's spear lies in the sand, meters.
const LYING_Y_M: f32 = 0.03;
/// Camera follow: share of the way the look point moves to the pair, and the pull-back.
const FOLLOW_LOOK: f32 = 0.7;
const FOLLOW_BACK: f32 = 0.35;
/// Per-tick ease of the follow envelope.
const FOLLOW_EASE: f32 = 0.12;

const WOOD: PbrMaterial = PbrMaterial {
    base_color: [0.45, 0.30, 0.15],
    metallic: 0.0,
    roughness: 0.8,
    exposure: 1.0,
};
const STEEL: PbrMaterial = PbrMaterial {
    base_color: [0.72, 0.74, 0.78],
    metallic: 0.9,
    roughness: 0.35,
    exposure: 1.0,
};
/// Hemp, a shade darker than the sand so a rope reads against it, rough and unshiny.
const HEMP: PbrMaterial = PbrMaterial {
    base_color: [0.50, 0.38, 0.22],
    metallic: 0.0,
    roughness: 0.95,
    exposure: 1.0,
};
const STREAK: PbrMaterial = PbrMaterial {
    base_color: [0.9, 0.92, 0.98],
    metallic: 0.0,
    roughness: 0.5,
    exposure: 1.6,
};

/// Why a frame of the picture could not be made.
#[derive(Debug)]
pub enum SceneError {
    /// A baked shape was not a valid mesh.
    Mesh(MeshError),
    /// The renderer refused an upload.
    Upload(PresentError),
}

impl std::fmt::Display for SceneError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Mesh(err) => write!(f, "spear shape is not a mesh: {err}"),
            Self::Upload(err) => write!(f, "spear upload refused: {err}"),
        }
    }
}

impl std::error::Error for SceneError {}

type Mesh = (Vec<[f32; 3]>, Vec<[f32; 3]>, Vec<u32>);

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn add(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn scale(a: [f32; 3], k: f32) -> [f32; 3] {
    a.map(|c| c * k)
}

fn norm(v: [f32; 3]) -> [f32; 3] {
    let n = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if n.is_finite() && n > 1.0e-6 {
        scale(v, 1.0 / n)
    } else {
        [0.0, 1.0, 0.0]
    }
}

/// Two unit vectors at right angles to the unit `dir` and to each other.
fn basis(dir: [f32; 3], roll: f32) -> ([f32; 3], [f32; 3]) {
    let helper = if dir[1].abs() < 0.9 {
        [0.0, 1.0, 0.0]
    } else {
        [1.0, 0.0, 0.0]
    };
    let u = norm(cross3(dir, helper));
    let v = cross3(dir, u);
    let (s, c) = roll.sin_cos();
    (
        add(scale(u, c), scale(v, s)),
        add(scale(v, c), scale(u, -s)),
    )
}

/// A tube along `points` with a radius per point (0 closes it to a point).
fn tube(points: &[[f32; 3]], radius: &dyn Fn(usize) -> f32, roll: f32) -> Mesh {
    let mut positions = Vec::with_capacity(points.len() * SIDES);
    let mut normals = Vec::with_capacity(points.len() * SIDES);
    let mut indices = Vec::new();
    for (i, p) in points.iter().enumerate() {
        let prev = points[i.saturating_sub(1)];
        let next = points[(i + 1).min(points.len() - 1)];
        let dir = norm(sub(next, prev));
        let (u, v) = basis(dir, roll);
        for k in 0..SIDES {
            let a = num(k) / num(SIDES) * std::f32::consts::TAU;
            let radial = add(scale(u, a.cos()), scale(v, a.sin()));
            positions.push(add(*p, scale(radial, radius(i))));
            normals.push(radial);
        }
    }
    for i in 0..points.len().saturating_sub(1) {
        for k in 0..SIDES {
            let (a, b) = (i * SIDES + k, i * SIDES + (k + 1) % SIDES);
            let (c, d) = (a + SIDES, b + SIDES);
            // Counter-clockwise seen from outside: the renderer culls back faces.
            indices.extend([idx(a), idx(b), idx(c), idx(b), idx(d), idx(c)]);
        }
    }
    (positions, normals, indices)
}

fn join(parts: Vec<Mesh>) -> Mesh {
    let mut out: Mesh = (Vec::new(), Vec::new(), Vec::new());
    for (p, n, i) in parts {
        let base = idx(out.0.len());
        out.0.extend(p);
        out.1.extend(n);
        out.2.extend(i.into_iter().map(|x| x + base));
    }
    out
}

/// Length of a vector, meters.
fn length(v: [f32; 3]) -> f32 {
    (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt()
}

/// The streak behind a flying spear: newest point first, thinning to nothing.
fn trail_mesh(points: &[[f32; 3]]) -> Mesh {
    let last = num(points.len().saturating_sub(1).max(1));
    tube(points, &|i| TRAIL_R_M * (1.0 - num(i) / last).max(0.0), 0.0)
}

fn to_mesh(shape: Mesh) -> Result<MeshData, MeshError> {
    let uvs = vec![[0.0, 0.0]; shape.0.len()];
    upload_mesh(&shape.0, &shape.1, &uvs, &shape.2)
}

/// Spear throws go out at an enemy no further than this, meters (a throw reaches 12 m).
const DEMO_THROW_M: f32 = 9.0;

/// One axis of the walk toward the target: full speed until the throw is in reach, then none.
fn wish(delta_m: f32, dist_m: f32) -> i8 {
    if dist_m > DEMO_THROW_M - 1.0 {
        i8::try_from(whole(delta_m / dist_m * 100.0)).unwrap_or(0)
    } else {
        0
    }
}

/// What each slot's throwing arm is doing, from the match's spear phases.
fn arm_states(view: &[SpearSlot]) -> [ArmState; MAX_LOOK_FIGHTERS] {
    std::array::from_fn(|slot| match view.get(slot).map(|s| s.phase) {
        Some(Phase::Flight(_)) => ArmState::Flying,
        Some(Phase::Tether(_)) => ArmState::Tethered,
        _ => ArmState::Free,
    })
}

/// The input of `--spear-demo`: the human slot faces the nearest other fighter, walks at it until
/// the throw reaches and throws whenever the spear is ready and no tether holds. A script for
/// captures and for trying the grapple without a keyboard; it reads only drawn places.
#[must_use]
pub fn demo_input(
    tick: u32,
    human: FighterId,
    fighters: &[FighterView],
    view: &[SpearSlot],
) -> FighterInput {
    let me = fighters.iter().find(|f| f.slot == human.0);
    let nearest = me.and_then(|me| {
        fighters
            .iter()
            .filter(|f| f.slot != me.slot)
            .map(|f| {
                (
                    f,
                    (f.pos_m[0] - me.pos_m[0]).hypot(f.pos_m[2] - me.pos_m[2]),
                )
            })
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(f, dist)| (me, f, dist))
    });
    let Some((me, other, dist)) = nearest else {
        return FighterInput {
            tick,
            fighter: human,
            mx: 0,
            mz: 0,
            facing: 0,
            buttons: Buttons::NONE,
        };
    };
    let facing =
        bearing_cdeg(other.pos_m[0] - me.pos_m[0], other.pos_m[2] - me.pos_m[2]).unwrap_or(0);
    let ready = view
        .get(usize::from(human.0))
        .is_some_and(|s| s.phase == Phase::Idle && s.cooldown == 0);
    let throw = ready && dist <= DEMO_THROW_M && tick.is_multiple_of(2);
    FighterInput {
        tick,
        fighter: human,
        // Wishes are world axes, percent of top speed.
        mx: wish(other.pos_m[0] - me.pos_m[0], dist),
        mz: wish(other.pos_m[2] - me.pos_m[2], dist),
        facing,
        buttons: Buttons(if throw { THROW_SPEAR } else { 0 }),
    }
}

/// The window's spear state: whether the grapple is on, whether the human slot is scripted
/// (`--spear-demo`) and the picture, which exists exactly while the grapple is on.
#[derive(Default)]
pub struct SpearMode {
    /// The grapple is on: Q throws.
    pub on: bool,
    /// The human slot throws by script.
    pub demo: bool,
    /// The spear, trail and rope picture.
    pub scene: Option<SpearScene>,
}

/// The spear grapple in the window. See the module docs.
pub struct SpearScene {
    look: SpearLook,
    /// Phase of each slot as the last tick left it (held spears, receipts).
    view: Vec<SpearSlot>,
    /// Drawn places of the fighters at the last tick.
    fighters: Vec<FighterView>,
    /// Where each right palm was drawn at the tick before the last and at the last one.
    palms: [[Option<[f32; 3]>; MAX_LOOK_FIGHTERS]; 2],
    /// Slots drawn as a ragdoll at the last tick: a fallen holder lets its spear go.
    down: [bool; MAX_LOOK_FIGHTERS],
    /// Follow envelope in `0..=1`: how much the camera has turned to the pair.
    follow: f32,
    /// Midpoint of the tethered pair at the last tick.
    pair_m: [f32; 3],
    draws: usize,
    /// How the ropes are cut into rings and sides: the tier's.
    rope: RopeStyle,
}

impl SpearScene {
    /// An empty scene on the default tier.
    #[must_use]
    pub fn new() -> Self {
        Self::with_tier(DEFAULT_TIER)
    }

    /// An empty scene on `tier`: the rope world's node count and how finely a rope is drawn
    /// (a potato draws a round tube in a few rings, a desktop the twisted strands).
    #[must_use]
    pub fn with_tier(tier: Tier) -> Self {
        Self {
            look: SpearLook::new(tier),
            view: Vec::new(),
            fighters: Vec::new(),
            palms: [[None; MAX_LOOK_FIGHTERS]; 2],
            down: [false; MAX_LOOK_FIGHTERS],
            follow: 0.0,
            pair_m: [0.0; 3],
            draws: 0,
            rope: RopeStyle::for_tier(tier),
        }
    }

    /// One accepted tick with the fighter look: the look throws with the thrower's arm, the
    /// picture takes the match's spear events and the drawn palms, then the look tows the
    /// tethered bodies as ragdolls. Call it after `observe_tick` and before the balance poll that
    /// steps the float world.
    pub fn observe_tick(
        &mut self,
        look: &mut FighterLook,
        live: &LiveMatch,
        fighters: &[FighterView],
    ) {
        look.observe_throws(&arm_states(live.spear_view()));
        self.observe(
            live.spear_events(),
            live.spear_view(),
            fighters,
            Ties {
                pelvis: &look.ragdoll_roots(),
                palms: &look.palms(1.0),
                bones: &look.ragdoll_bone_frames(),
            },
        );
        look.observe_carries(&self.tows());
    }

    /// The picture underneath.
    #[must_use]
    pub fn look(&self) -> &SpearLook {
        &self.look
    }

    /// Spears and ropes alive now, for the view line.
    #[must_use]
    pub fn counts(&self) -> (usize, usize) {
        (self.look.spears(), self.look.ropes())
    }

    /// Per slot, how high the arc lifts a tethered body: what the ragdoll layer is told.
    #[must_use]
    pub fn tows(&self) -> [Option<f32>; MAX_LOOK_FIGHTERS] {
        self.look
            .carries()
            .map(|carry| carry.map(|c: Carry| c.lift_m()))
    }

    /// Takes one accepted tick of the match (`events`, `view`), the fighters' drawn places and
    /// where each ragdolled body's root is.
    pub fn observe(
        &mut self,
        events: &[SpearEvent],
        view: &[SpearSlot],
        fighters: &[FighterView],
        ties: Ties<'_>,
    ) {
        self.look.observe(events, view, fighters, ties);
        self.palms = [self.palms[1], *ties.palms];
        self.down = ties.pelvis.map(|pelvis| pelvis.is_some());
        self.view.clear();
        self.view.extend_from_slice(view);
        self.fighters.clear();
        self.fighters.extend_from_slice(fighters);
        let pair: Vec<[f32; 3]> = self
            .look
            .carries()
            .iter()
            .enumerate()
            .filter(|(_, c)| c.is_some())
            .filter_map(|(slot, _)| self.fighters.iter().find(|f| usize::from(f.slot) == slot))
            .map(|f| add(f.pos_m, [0.0, 1.1, 0.0]))
            .collect();
        let goal = if pair.is_empty() { 0.0 } else { 1.0 };
        self.follow += (goal - self.follow) * FOLLOW_EASE;
        if !pair.is_empty() {
            let sum = pair.iter().fold([0.0; 3], |acc, p| add(acc, *p));
            self.pair_m = scale(sum, 1.0 / num(pair.len()));
        }
    }

    /// Turns the camera toward a tethered pair and pulls it back a little while the tether
    /// holds, and eases back after. Eye and look point move together in the view plane.
    pub fn frame(&self, camera: &mut Camera) {
        let w = self.follow;
        if w < 0.01 {
            return;
        }
        let target = camera.target.to_array();
        let eye = camera.eye.to_array();
        let look = [
            target[0] + (self.pair_m[0] - target[0]) * FOLLOW_LOOK * w,
            target[1] + (self.pair_m[1] - target[1]) * FOLLOW_LOOK * w,
            target[2] + (self.pair_m[2] - target[2]) * FOLLOW_LOOK * w,
        ];
        let back = sub(eye, look);
        let eye = add(look, scale(back, 1.0 + FOLLOW_BACK * w));
        camera.eye = eye.into();
        camera.target = look.into();
    }

    /// Spears standing in a hand: a slot idle with no cooldown holds its spear up in its drawn
    /// right palm (`alpha` blends the last two ticks), else beside it.
    fn held(&self, alpha: f32) -> Vec<SpearDraw> {
        self.view
            .iter()
            .enumerate()
            .filter(|(_, s)| s.phase == Phase::Idle && s.cooldown == 0)
            .filter_map(|(slot, _)| {
                let fighter = self.fighters.iter().find(|f| usize::from(f.slot) == slot)?;
                let (s, c) = fighter.yaw.sin_cos();
                if self.down[slot] {
                    // A ragdoll holds nothing: the spear lies in the sand along the body's facing,
                    // its grip where the hand fell (it stood 83 deg up out of a fallen holder).
                    let dir = [s, 0.0, c];
                    let hand = self.palms[1][slot].unwrap_or(fighter.pos_m);
                    let tail = sub([hand[0], LYING_Y_M, hand[2]], scale(dir, HELD_GRIP_M));
                    return Some(SpearDraw {
                        slot: u8::try_from(slot).ok()?,
                        tip: add(tail, scale(dir, SPEAR_LEN_M)),
                        dir,
                        roll: 0.0,
                        flying: false,
                        stuck_in: None,
                        held_by: None,
                    });
                }
                let dir = norm([s * 0.12, 1.0, c * 0.12]);
                let tail = match (self.palms[0][slot], self.palms[1][slot]) {
                    (Some(was), Some(now)) => {
                        let palm = add(was, scale(sub(now, was), alpha.clamp(0.0, 1.0)));
                        sub(palm, scale(dir, HELD_GRIP_M))
                    }
                    _ => add(
                        fighter.pos_m,
                        [s * HELD_FORWARD_M, HELD_FOOT_Y_M, c * HELD_FORWARD_M],
                    ),
                };
                Some(SpearDraw {
                    slot: u8::try_from(slot).ok()?,
                    tip: add(tail, scale(dir, SPEAR_LEN_M)),
                    dir,
                    roll: 0.0,
                    flying: false,
                    stuck_in: None,
                    held_by: u8::try_from(slot).ok(),
                })
            })
            .collect()
    }

    /// Uploads this frame's shapes and pushes their draws onto `list`; returns how many.
    /// `alpha` blends the last two ticks. An error means the scene is unusable.
    pub fn push(
        &mut self,
        presenter: &mut Presenter,
        list: &mut DrawList,
        alpha: f32,
    ) -> Result<usize, SceneError> {
        let mut spears = self.look.draws(alpha);
        spears.extend(self.held(alpha));
        let mut pushed = 0;
        let mut put = |part: u64, slot: u8, shape: Mesh, material: PbrMaterial| {
            let key = MeshKey::new(SPEAR_MESH_ID_BASE + u64::from(slot) * 4 + part, DIGEST);
            let mesh = to_mesh(shape).map_err(SceneError::Mesh)?;
            presenter
                .upload_mesh(key, &mesh)
                .map_err(SceneError::Upload)?;
            list.items.push(DrawItem {
                mesh: key,
                material,
                base_color: None,
                normal_map: None,
                orm: None,
                emissive: None,
            });
            pushed += 1;
            Ok::<(), SceneError>(())
        };
        for spear in &spears {
            put(0, spear.slot, shaft_mesh(spear.tip, spear.dir), WOOD)?;
            put(
                1,
                spear.slot,
                head_mesh(spear.tip, spear.dir, spear.roll),
                STEEL,
            )?;
            let trail = self.look.trail(spear.slot);
            if spear.flying && trail.len() >= 2 {
                put(3, spear.slot, trail_mesh(trail), STREAK)?;
            }
        }
        for slot in 0..MAX_LOOK_FIGHTERS {
            let Ok(slot) = u8::try_from(slot) else {
                continue;
            };
            if let Some(shape) = self.rope_shape(slot, alpha) {
                put(2, slot, shape, HEMP)?;
            }
        }
        self.draws = pushed;
        Ok(pushed)
    }

    /// Every spear the frame draws as audit rows: held, flying, stuck or at rest, with the
    /// slot that holds it or the body it is stuck in. View only; `alpha` blends the last two ticks.
    #[must_use]
    pub fn audit_rows(&self, alpha: f32) -> Vec<SpearTruth> {
        let mut spears = self.look.draws(alpha);
        spears.extend(self.held(alpha));
        spears
            .iter()
            .map(|s| SpearTruth {
                slot: s.slot,
                kind: match (s.held_by, s.stuck_in, s.flying) {
                    (Some(_), _, _) => "held",
                    (_, Some(_), _) => "stuck",
                    (_, _, true) => "flying",
                    _ => "rest",
                },
                tip: s.tip,
                dir: s.dir,
                len_m: SPEAR_LEN_M,
                holder: s.held_by,
                victim: s.stuck_in,
            })
            .collect()
    }

    /// Draws pushed by the last frame.
    #[must_use]
    pub const fn draws(&self) -> usize {
        self.draws
    }

    /// The mesh of the rope of `slot`, or `None` when it has none: its nodes smoothed into a curve
    /// that sags while the rope is slack, runs straight while it is taut and turns round the
    /// corner it snagged on, swept into a lit twisted tube.
    fn rope_shape(&self, slot: u8, alpha: f32) -> Option<Mesh> {
        let nodes = self.look.rope(slot, alpha)?;
        let lay = self.rope_lay(slot)?;
        rope::mesh(&rope::curve(&nodes, lay, &self.rope), &self.rope)
    }

    /// How the rope of `slot` lies: round the corner it snagged on, hanging, fallen or straight.
    fn rope_lay(&self, slot: u8) -> Option<Lay> {
        let state = self.look.rope_state(slot)?.0;
        Some(match (self.wrap_corner(usize::from(slot)), state) {
            (Some(corner), _) => Lay::Wrapped(corner),
            (None, TetherState::Slack) => Lay::Slack,
            (None, TetherState::Loose) => Lay::Loose,
            (None, TetherState::Taut) => Lay::Taut,
        })
    }

    /// The corner the slot's rope is snagged on this tick, meters: set while
    /// the tether holds, all zero while the rope runs straight to its anchor.
    fn wrap_corner(&self, slot: usize) -> Option<[f32; 3]> {
        let view = self.view.get(slot)?;
        if !matches!(view.phase, Phase::Tether(_)) || view.wrap_mm == [0, 0, 0] {
            return None;
        }
        Some(view.wrap_mm.map(meters))
    }
}

/// A millimetre length as meters, for a drawn rope.
fn meters(mm: i32) -> f32 {
    f32::from(i16::try_from(mm).unwrap_or(0)) / 1000.0
}

impl Default for SpearScene {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::window_audio_space::spear_cue;
    use engine_arena::spear::EndWhy;

    #[test]
    fn spear_window_shapes_are_valid_meshes_in_world_space() {
        let tip = [1.0, 1.3, -2.0];
        let dir = norm([0.5, 0.1, 0.8]);
        let rope_style = RopeStyle::for_tier(DEFAULT_TIER);
        let rope = rope::curve(
            &[[0.0, 1.0, 0.0], [1.0, 0.5, 0.2], [2.0, 1.0, 0.4]],
            Lay::Slack,
            &rope_style,
        );
        for (name, shape) in [
            ("shaft", shaft_mesh(tip, dir)),
            ("head", head_mesh(tip, dir, 0.7)),
            ("rope", rope::mesh(&rope, &rope_style).expect("a rope")),
            (
                "trail",
                trail_mesh(&[tip, [0.5, 1.3, -2.5], [0.0, 1.3, -3.0]]),
            ),
        ] {
            let mesh = to_mesh(shape).unwrap_or_else(|err| panic!("{name}: {err}"));
            assert!(!mesh.positions.is_empty(), "{name}");
        }
        // The head's tip is exactly the spear's tip; the shaft starts inside the butt shoe, a
        // hand's breadth in from the tail.
        let (positions, _, _) = head_mesh(tip, dir, 0.0);
        assert!(positions.contains(&tip));
        let (shaft, _, _) = shaft_mesh(tip, dir);
        let tail = sub(tip, scale(dir, SPEAR_LEN_M));
        let near = shaft.iter().map(|p| length(sub(*p, tail)));
        assert!(near.fold(f32::MAX, f32::min) < 0.12);
    }

    #[test]
    fn spear_window_draws_every_rope_of_a_real_throw_smooth_and_the_same_shape_each_frame() {
        use crate::{App, GameConfig, TickOutcome, HUMAN_SLOT};
        for tier in [Tier::Potato, Tier::Comfort] {
            let cfg = GameConfig {
                seed: 2040,
                ticks: 600,
                fighters: 2,
                half_extent_m: 20.0,
            };
            let mut app = App::new(cfg).expect("session").into_spear().expect("spear");
            let human = FighterId::new(HUMAN_SLOT).expect("slot");
            let mut scene = SpearScene::with_tier(tier);
            let mut out = TickOutcome::default();
            let (mut slack, mut taut, mut drawn) = (0, 0, 0);
            for tick in 0..600 {
                let input = demo_input(
                    tick,
                    human,
                    app.fighter_views(),
                    app.live_match().spear_view(),
                );
                app.tick_into(input, &mut out);
                let live = app.live_match();
                scene.observe(
                    live.spear_events(),
                    live.spear_view(),
                    app.fighter_views(),
                    Ties::NONE,
                );
                let Some(shape) = scene.rope_shape(0, 0.5) else {
                    continue;
                };
                drawn += 1;
                slack += i32::from(scene.rope_lay(0) == Some(Lay::Slack));
                taut += i32::from(scene.rope_lay(0) == Some(Lay::Taut));
                assert_eq!(shape.0.len(), scene.rope.vertices(), "{tier:?} tick {tick}");
                to_mesh(shape).unwrap_or_else(|err| panic!("{tier:?} tick {tick}: {err}"));
            }
            assert!(drawn > 30, "{tier:?}: the rope was drawn {drawn} ticks");
            assert!(slack > 0, "{tier:?}: a rope sagged in flight");
            assert!(taut > 0, "{tier:?}: a rope pulled straight in the tow");
        }
    }

    #[test]
    fn spear_window_every_event_maps_to_a_cue_the_bank_has() {
        use engine_audio::cue_names as cue;
        let known = [
            cue::SWING,
            cue::HIT,
            cue::BLOCK,
            cue::PARRY,
            cue::IMPACT_STONE,
            cue::FOOTSTEP_WOOD,
            cue::SEVER,
        ];
        let events = [
            SpearEvent::Thrown {
                thrower: 0,
                at_mm: [0; 3],
                vel_mm: [1, 0, 0],
            },
            SpearEvent::Hit {
                thrower: 0,
                victim: Some(1),
                at_mm: [0; 3],
            },
            SpearEvent::Hit {
                thrower: 0,
                victim: None,
                at_mm: [0; 3],
            },
            SpearEvent::Deflected {
                thrower: 0,
                victim: 1,
                parried: true,
            },
            SpearEvent::TetherPull {
                thrower: 0,
                victim: Some(1),
                speed: 300,
            },
            SpearEvent::TetherEnd {
                thrower: 0,
                victim: Some(1),
                why: EndWhy::Wall,
                damage_permille: 90,
            },
            SpearEvent::TetherEnd {
                thrower: 0,
                victim: Some(1),
                why: EndWhy::Cut,
                damage_permille: 0,
            },
        ];
        for event in events {
            let (name, gain, _) = spear_cue(event).expect("a cue");
            assert!(
                known.contains(&name) && (0.0..=1.0).contains(&gain),
                "{event:?}"
            );
        }
        assert_eq!(
            spear_cue(SpearEvent::Missed {
                thrower: 0,
                why: engine_arena::MissWhy::Range
            }),
            None
        );
    }

    #[test]
    fn spear_window_camera_follows_a_tethered_pair_and_lets_go() {
        use engine_render::v1::Camera;
        let mut scene = SpearScene::new();
        let camera = || Camera::new([0.0, 5.0, 12.0], [0.0, 1.0, 0.0], [0.0, 1.0, 0.0], 0.9);
        let mut untouched = camera();
        scene.frame(&mut untouched);
        assert_eq!(untouched.eye.to_array(), camera().eye.to_array());
        scene.follow = 1.0;
        scene.pair_m = [6.0, 1.1, 0.0];
        let mut followed = camera();
        scene.frame(&mut followed);
        assert!(followed.target.x > 3.0, "the look point moved to the pair");
        assert!(followed.eye.distance(followed.target) > camera().eye.distance(camera().target));
    }

    #[test]
    fn spear_window_holds_the_spear_in_the_drawn_palm_and_follows_it_between_ticks() {
        let mut scene = SpearScene::new();
        let ready = [SpearSlot::default(); 2];
        let fighters = [FighterView {
            slot: 0,
            pos_m: [0.0, 0.0, 0.0],
            yaw: 0.0,
        }];
        let mut palms = [None; MAX_LOOK_FIGHTERS];
        for palm in [[0.2, 1.0, 0.3], [0.4, 1.4, 0.5]] {
            palms[0] = Some(palm);
            scene.observe(
                &[],
                &ready,
                &fighters,
                Ties {
                    pelvis: &[None; MAX_LOOK_FIGHTERS],
                    palms: &palms,
                    bones: &[],
                },
            );
        }
        // The shaft passes through the palm: the grip is HELD_GRIP_M from the tail.
        let grip = |alpha: f32| {
            let held = scene.held(alpha);
            let spear = held.first().expect("a ready slot holds its spear");
            sub(spear.tip, scale(spear.dir, SPEAR_LEN_M - HELD_GRIP_M))
        };
        for (alpha, want) in [
            (0.0, [0.2, 1.0, 0.3]),
            (1.0, [0.4, 1.4, 0.5]),
            (0.5, [0.3, 1.2, 0.4]),
        ] {
            let at = grip(alpha);
            let gap = sub(at, want);
            assert!(
                gap.iter().all(|c| c.abs() < 1.0e-4),
                "alpha {alpha}: grip {at:?} vs palm {want:?}"
            );
        }
    }

    #[test]
    fn spear_window_a_fallen_holder_drops_the_spear_flat_in_the_sand() {
        let mut scene = SpearScene::new();
        let ready = [SpearSlot::default(); 2];
        let fighters = [FighterView {
            slot: 0,
            pos_m: [1.0, 0.0, 2.0],
            yaw: 0.0,
        }];
        let mut palms = [None; MAX_LOOK_FIGHTERS];
        palms[0] = Some([1.3, 0.2, 2.4]);
        let mut pelvis = [None; MAX_LOOK_FIGHTERS];
        pelvis[0] = Some([1.0, 0.25, 2.0]);
        let ties = Ties {
            pelvis: &pelvis,
            palms: &palms,
            bones: &[],
        };
        scene.observe(&[], &ready, &fighters, ties);
        let spears = scene.held(1.0);
        let spear = spears.first().expect("the dropped spear is still drawn");
        assert_eq!(spear.held_by, None, "a ragdoll holds nothing");
        assert!(
            spear.dir[1].abs() < 1.0e-6,
            "flat, not standing: {:?}",
            spear.dir
        );
        let tail = sub(spear.tip, scale(spear.dir, SPEAR_LEN_M));
        for end in [tail, spear.tip] {
            assert!(
                (end[1] - LYING_Y_M).abs() < 1.0e-6,
                "both ends lie in the sand: {end:?}"
            );
        }
        let grip = add(tail, scale(spear.dir, HELD_GRIP_M));
        assert!(
            length(sub(grip, [1.3, LYING_Y_M, 2.4])) < 1.0e-5,
            "the grip is where the hand fell"
        );
        let rows = scene.audit_rows(1.0);
        assert_eq!(
            rows.first().map(|r| r.kind),
            Some("rest"),
            "the audit reads it at rest"
        );
    }

    #[test]
    fn spear_window_demo_script_throws_hits_and_flies_the_pair() {
        use crate::{App, GameConfig, TickOutcome, HUMAN_SLOT};
        let cfg = GameConfig {
            seed: 2040,
            ticks: 600,
            fighters: 2,
            half_extent_m: 20.0,
        };
        let mut app = App::new(cfg).expect("session").into_spear().expect("spear");
        let human = FighterId::new(HUMAN_SLOT).expect("slot");
        let mut scene = SpearScene::new();
        let mut out = TickOutcome::default();
        let (mut towed, mut ropes) = (0, 0);
        for tick in 0..600 {
            let live = app.live_match();
            let input = demo_input(tick, human, app.fighter_views(), live.spear_view());
            app.tick_into(input, &mut out);
            let live = app.live_match();
            scene.observe(
                live.spear_events(),
                live.spear_view(),
                app.fighter_views(),
                Ties::NONE,
            );
            towed += usize::from(scene.tows().iter().flatten().count() == 2);
            ropes = ropes.max(scene.counts().1);
        }
        let stats = scene.look().stats();
        assert!(
            stats.thrown >= 1 && stats.hits >= 1 && stats.tethers >= 1,
            "{stats:?}"
        );
        assert!(
            towed > 5,
            "both bodies were towed together for {towed} ticks"
        );
        assert_eq!(ropes, 1);
        assert!(scene
            .look()
            .receipt_line()
            .starts_with("arena spear: thrown="));
    }
}
