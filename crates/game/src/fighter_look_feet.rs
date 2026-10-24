//! fighter_look_feet: planted feet for the fighter look (LF12).
//!
//! Consumer: [`FighterLook::observe_tick`] folds every seen slot's feet once a
//! tick ([`FighterLook::fold_feet`]) and `slot_sample` plants them on every
//! frame ([`FighterLook::plant_feet`]), so the window and the headless view run
//! pose a plant the same way. Data owner: each slot owns its [`Feet`], each rig
//! its [`Legs`]; the solve is `engine_anim::ik`. A foot the clip holds still
//! against the body or on the sand (ankle speed under [`PLANT_SPEED_M_S`] in
//! either frame for a few ticks, low over the sand) plants where it is drawn and stays on that spot in the world
//! while the sim moves the body. The drawn foot follows the spot through an
//! offset from the clip's foot that moves at most [`STEP_M`] a tick, so a
//! plant or a let-go never jumps; a plant the body has carried past
//! [`MAX_PULL_M`] lets go and the offset runs back to the clip. The pelvis
//! drops by the largest gap the legs cannot reach. The plant runs after the
//! inertial offsets, so a clip switch stays continuous. Units: meters, seconds,
//! fixed ticks at 60 Hz; world `xz` is the sand plane. Determinism:
//! presentation only; a frame is a pure fold of the tick state and `alpha`, and
//! a held slot's state stands still with its clip clock, so the same ticks give
//! the same pose twice. Errors: a solve the rig refuses is a loud
//! [`LookError::Skin`]; a rig with no legs named plants nothing. Heap: none per
//! frame (the lanes live on the stack).

use engine_anim::contract::{LocalPose, Skeleton};
use engine_anim::ik::{limb_positions, solve_limb, IkError, Limb};
use engine_arena::GaitPhase;
use engine_core::vec3::dist2;

use crate::FighterView;

use super::{
    clip_alpha, slot_root, Act, FighterLook, LookError, Rig, SlotAnim, LAG_TICKS, TICK_HZ,
};

/// Ankle speed against the body under which the clip has a foot planted, in
/// meters per second.
const PLANT_SPEED_M_S: f32 = 0.15;
/// Ankle speed against the body above which a planted foot lets go, in meters
/// per second: twice the plant speed, so a foot at the edge does not flicker.
const RELEASE_SPEED_M_S: f32 = 0.30;
/// Consecutive ticks the clip must hold a foot still before it plants, so a
/// foot passing through zero speed on its way somewhere never plants.
const PLANT_STILL_TICKS: u8 = 3;
/// How far above its standing height the ankle may ride and still plant, in
/// meters.
const PLANT_LIFT_M: f32 = 0.03;
/// Farthest the plant holds a foot from where the clip puts it, in meters;
/// past it the foot lets go and runs back to the clip.
const MAX_PULL_M: f32 = 0.12;
/// Most the drawn foot's offset from the clip's foot may change in one tick,
/// in meters (1.2 m/s): a plant, a let-go or a lunging root moves the foot by
/// this much a tick and never snaps it.
const STEP_M: f32 = 0.02;
/// Share of its length a leg may be asked to stretch to before the pelvis
/// drops to give it room.
const COMFORT_REACH: f32 = 0.98;
/// Deepest pelvis drop in meters; past it a leg stretches straight instead.
const MAX_DROP_M: f32 = 0.08;
/// Direction a knee bends toward when the clip's leg is straight and names no
/// side, in the body's own frame (forward is +z).
const KNEE_POLE: [f32; 3] = [0.0, 0.0, 1.0];
/// Least bend of the clip's knee off the hip-to-ankle line, in meters, that
/// still names a side to keep.
const MIN_BEND_M: f32 = 0.005;
/// Least fall, in meters, that counts as a landing worth squashing: a step
/// off a ledge, not the millimetre bob of a walk.
const LAND_MIN_FALL_M: f32 = 0.12;
/// Feet height above the sand, in meters, that starts an airborne stretch.
const LAND_AIR_MIN_M: f32 = 0.05;
/// Feet height at or below which the body is back on the sand, in meters.
const LAND_GROUND_M: f32 = 0.001;
/// Fall height that earns the full squash, in meters: a 4 m fall and every
/// longer one squash as deep as the pelvis drop allows.
const SQUASH_FULL_FALL_M: f32 = 1.5;
/// Deepest landing squash: extra pelvis drop in meters, on top of the plant
/// gap, capped by [`MAX_DROP_M`].
const SQUASH_DEPTH_M: f32 = 0.07;
/// Ticks a landing squash takes to ease back to the standing pose.
const SQUASH_TICKS: u32 = 8;

/// Both legs of a rig: their joints and each ankle's standing height.
#[derive(Clone, Copy, Debug)]
pub(super) struct Legs {
    /// Left then right leg: hip, knee and ankle joints.
    pub(super) limbs: [Limb; 2],
    /// Ankle height in the bind pose, in meters: the standing height a plant
    /// is measured from.
    pub(super) rest_y: [f32; 2],
}

impl Legs {
    /// The legs of a skeleton found by joint name, with the ankle heights
    /// read from its bind joints; `None` when a leg is missing or not a
    /// chain. Cold path: once per rig at load.
    pub(super) fn find(skeleton: &Skeleton, bind: &[[f32; 3]]) -> Option<Self> {
        let at = |name: &str| skeleton.joints.iter().position(|joint| joint.name == name);
        let leg = |side: &str| {
            let hip = at(&format!("leg_upper_{side}"))?;
            let knee = at(&format!("leg_lower_{side}"))?;
            let ankle = at(&format!("foot_{side}"))?;
            let limb = Limb::new(skeleton, hip, knee, ankle).ok()?;
            Some((limb, bind.get(ankle)?[1]))
        };
        let (left, right) = (leg("l")?, leg("r")?);
        Some(Self {
            limbs: [left.0, right.0],
            rest_y: [left.1, right.1],
        })
    }
}

/// One foot's plant. World `xz` is the sand plane.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(super) struct Plant {
    /// World spot the foot is held at while `planted`.
    pub(super) contact: [f32; 2],
    /// True while the foot is held at `contact`.
    pub(super) planted: bool,
    /// World offset of the drawn ankle from the clip's, at the start of the
    /// last tick and at its end.
    pub(super) offset: [[f32; 2]; 2],
    /// Consecutive ticks the clip has held the foot still and low.
    pub(super) still: u8,
    /// The clip's ankle in model space at the end of the last tick, for its
    /// speed against the body; `None` before the first tick.
    pub(super) ankle: Option<[f32; 3]>,
    /// The same ankle on the sand, for how fast the body carries it.
    pub(super) world: Option<[f32; 2]>,
}

impl Plant {
    pub(super) const NEW: Self = Self {
        contact: [0.0; 2],
        planted: false,
        offset: [[0.0; 2]; 2],
        still: 0,
        ankle: None,
        world: None,
    };

    /// World offset of the drawn ankle from the clip's at sub-tick `alpha`:
    /// the tick's start offset lerped to its end offset. A foot that tracks its
    /// spot while the clip foot moves in a line lands on the spot at every
    /// `alpha`.
    fn offset_at(&self, alpha: f32) -> [f32; 2] {
        let [from, to] = self.offset;
        [
            from[0] + (to[0] - from[0]) * alpha,
            from[1] + (to[1] - from[1]) * alpha,
        ]
    }

    /// True while the plant moves the drawn foot at all: held, or an offset
    /// still running back to the clip.
    fn moves(&self) -> bool {
        self.planted || self.offset != [[0.0; 2]; 2]
    }
}

/// One landing the look saw (FEEL7): the look tick the feet reached the sand
/// and the fall height that carried them there, in meters. The authority's
/// height channel charges the health; this state drives the body squash on
/// the same tick.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(super) struct Land {
    pub(super) tick: u32,
    pub(super) fall_m: f32,
}

/// A small tick count as a float for the decay curves; these counts stay
/// under a byte, and `u8 -> f32` is a lossless `From`.
pub(super) fn ticks_f32(ticks: u32) -> f32 {
    f32::from(u8::try_from(ticks).unwrap_or(u8::MAX))
}

impl Land {
    /// Extra pelvis drop for this landing at look `tick`, in meters: deepest
    /// on the landing tick, easing to zero over [`SQUASH_TICKS`]. A longer
    /// fall squashes deeper, up to [`SQUASH_FULL_FALL_M`].
    fn squash_drop_m(self, tick: u32) -> f32 {
        let age = tick.saturating_sub(self.tick);
        if age >= SQUASH_TICKS || !self.fall_m.is_finite() {
            return 0.0;
        }
        let strength = (self.fall_m / SQUASH_FULL_FALL_M).clamp(0.0, 1.0);
        let decay = 1.0 - ticks_f32(age) / ticks_f32(SQUASH_TICKS);
        SQUASH_DEPTH_M * strength * decay
    }
}

/// A slot's feet: the left and right plant, plus whether the plant moves the
/// pose. With `ik` off the plants are still tracked and metered, and the pose
/// stays the clip's, which is how the proofs read the slide the plant removes.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(super) struct Feet {
    pub(super) plants: [Plant; 2],
    pub(super) ik: bool,
    /// Highest feet height seen since takeoff, in meters; `None` while the
    /// body is on the sand. Presentation only.
    pub(super) air: Option<f32>,
    /// Last landing the feet folded, kept so its squash decays by tick.
    pub(super) land: Option<Land>,
}

impl Feet {
    pub(super) const fn new() -> Self {
        Self {
            plants: [Plant::NEW; 2],
            ik: true,
            air: None,
            land: None,
        }
    }
}

/// Foot `xz` held at full plant this frame, per foot; `None` when not planted.
pub(super) type Contacts = [Option<[f32; 2]>; 2];

/// One leg as the clip poses it this frame, plus where a plant aims its ankle.
#[derive(Clone, Copy)]
struct ClipLeg {
    hip: [f32; 3],
    /// Side the knee bends toward, from [`knee_pole`].
    pole: [f32; 3],
    ankle: [f32; 3],
    /// Thigh plus shin length in meters.
    reach: f32,
    /// Model-space ankle target of the plant; `None` while the foot is free.
    aim: Option<[f32; 3]>,
}

impl ClipLeg {
    const NONE: Self = Self {
        hip: [0.0; 3],
        pole: KNEE_POLE,
        ankle: [0.0; 3],
        reach: 0.0,
        aim: None,
    };
}

/// World `xz` of a model-space point under the slot's root.
fn to_world_xz(model: [f32; 3], pos: [f32; 3], yaw: f32) -> [f32; 2] {
    let (s, c) = yaw.sin_cos();
    [
        model[0] * c + model[2] * s + pos[0],
        -model[0] * s + model[2] * c + pos[2],
    ]
}

/// A world `xz` vector in the body's own frame: the turn half of
/// [`to_world_xz`] undone.
fn to_model_vec(world: [f32; 2], yaw: f32) -> [f32; 2] {
    let (s, c) = yaw.sin_cos();
    [world[0] * c - world[1] * s, world[0] * s + world[1] * c]
}

fn dist3(a: [f32; 3], b: [f32; 3]) -> f32 {
    dist2(a, b).sqrt()
}

/// True for a body that stands: any act but the death clip, steady or
/// stumbling. A body that lies, kneels or is down keeps the clip's feet.
fn plants_feet(slot: &SlotAnim) -> bool {
    slot.act != Act::Down && matches!(slot.balance, GaitPhase::Steady | GaitPhase::Stumbling)
}

/// `from` moved toward `to` by at most `step` meters.
fn approach(from: [f32; 2], to: [f32; 2], step: f32) -> [f32; 2] {
    let (dx, dz) = (to[0] - from[0], to[1] - from[1]);
    let gap = dx.hypot(dz);
    if gap <= step {
        to
    } else {
        [from[0] + dx * (step / gap), from[1] + dz * (step / gap)]
    }
}

/// One foot's plant after a tick. `ankle` is the clip's ankle in model space
/// at the end of the tick, `rest_y` its standing height and `world` where the
/// clip puts it on the sand. A foot plants where the clip puts it once the clip
/// has held it low and still (against the body or on the sand) for
/// [`PLANT_STILL_TICKS`] and no offset is left
/// running; a held foot lets go when the clip lifts or moves it past
/// [`RELEASE_SPEED_M_S`] or the body has carried it past [`MAX_PULL_M`]. The
/// drawn offset then moves toward the held spot (or back to zero) by at most
/// [`STEP_M`] a tick. Pure.
fn step_plant(prev: Plant, ankle: [f32; 3], rest_y: f32, world: [f32; 2], elapsed: u32) -> Plant {
    let ticks = elapsed.max(1);
    let count = f32::from(u16::try_from(ticks).unwrap_or(u16::MAX));
    let speed = prev
        .ankle
        .map(|was| (ankle[0] - was[0]).hypot(ankle[2] - was[2]) / (count / TICK_HZ));
    // The offset can follow a clip foot the body carries up to STEP_M a tick;
    // a faster one cannot be held, so it does not plant (or lets go).
    let carried = prev
        .world
        .map(|was| (world[0] - was[0]).hypot(world[1] - was[1]) / count);
    let low = ankle[1] <= rest_y + PLANT_LIFT_M;
    // Still against the body (a stance the clip holds) or still on the sand (the
    // stance foot of a run, which the clip sweeps back as fast as the body goes):
    // checking only the first starved the plants once the duel ran most ticks.
    let slower_than = |limit: f32| {
        low && carried.is_some_and(|m| m <= STEP_M)
            && (speed.is_some_and(|s| s < limit) || carried.is_some_and(|m| m * TICK_HZ < limit))
    };
    let still = if slower_than(PLANT_SPEED_M_S) {
        prev.still.saturating_add(1)
    } else {
        0
    };
    let mut next = Plant {
        ankle: Some(ankle),
        world: Some(world),
        still,
        ..prev
    };
    if prev.planted {
        let pull = (prev.contact[0] - world[0]).hypot(prev.contact[1] - world[1]);
        next.planted = slower_than(RELEASE_SPEED_M_S) && pull <= MAX_PULL_M;
    } else if still >= PLANT_STILL_TICKS && prev.offset[1] == [0.0; 2] {
        next.planted = true;
        next.contact = world;
    }
    let want = if next.planted {
        [next.contact[0] - world[0], next.contact[1] - world[1]]
    } else {
        [0.0; 2]
    };
    next.offset = [
        prev.offset[1],
        approach(prev.offset[1], want, STEP_M * count),
    ];
    next
}

/// Pelvis drop in meters that brings a plant target back inside what the leg
/// can reach. The leg may stretch to [`COMFORT_REACH`] of its length, or as far
/// as the clip already stretched it, whichever is more; past that the pelvis
/// sinks until the target sits on that sphere about the hip. `hip` and
/// `clip_ankle` are the clip's own joints, `target` the planted ankle. Pure.
fn gap_m(hip: [f32; 3], clip_ankle: [f32; 3], target: [f32; 3], reach: f32) -> f32 {
    let limit = (COMFORT_REACH * reach).max(dist3(hip, clip_ankle));
    if dist3(hip, target) <= limit {
        return 0.0;
    }
    let across = (target[0] - hip[0]).hypot(target[2] - hip[2]);
    let below = (hip[1] - target[1]).max(0.0);
    if across >= limit {
        return below;
    }
    (below - (limit * limit - across * across).sqrt()).max(0.0)
}

/// Pole for one knee: the clip's own bend, the knee's offset from the
/// hip-to-ankle line, so a foot that is not moved keeps the knee where the
/// clip put it; a leg too straight to name a side bends forward. Pure.
fn knee_pole(hip: [f32; 3], knee: [f32; 3], ankle: [f32; 3]) -> [f32; 3] {
    let chord = [ankle[0] - hip[0], ankle[1] - hip[1], ankle[2] - hip[2]];
    let along = dist3(ankle, hip);
    let thigh = [knee[0] - hip[0], knee[1] - hip[1], knee[2] - hip[2]];
    if along <= 1.0e-6 {
        return KNEE_POLE;
    }
    let share = (thigh[0] * chord[0] + thigh[1] * chord[1] + thigh[2] * chord[2]) / (along * along);
    let bend = [
        thigh[0] - chord[0] * share,
        thigh[1] - chord[1] * share,
        thigh[2] - chord[2] * share,
    ];
    if dist3(bend, [0.0; 3]) > MIN_BEND_M {
        bend
    } else {
        KNEE_POLE
    }
}

fn skin_err(err: IkError) -> LookError {
    LookError::Skin(err.to_string())
}

impl FighterLook {
    /// Fold one tick of fall landings into every seen slot's feet (FEEL7):
    /// remember the highest feet height since takeoff and record a landing on
    /// the tick the body reaches the sand. Called before
    /// [`FighterLook::observe_tick`] so the squash starts on the same tick the
    /// authority's fall table charged, and the held foot state carries it into
    /// the pose. Presentation only; the sim hash never reads it.
    pub fn observe_landings(&mut self, tick: u32, views: &[FighterView]) {
        for view in views {
            let Some(slot) = self.slots.get_mut(usize::from(view.slot)) else {
                continue;
            };
            if slot.held {
                continue;
            }
            let feet = &mut slot.feet;
            let y = view.pos_m[1];
            match feet.air {
                None => {
                    if y > LAND_AIR_MIN_M {
                        feet.air = Some(y);
                        feet.land = None;
                    }
                }
                Some(top) => {
                    let top = top.max(y);
                    if y <= LAND_GROUND_M {
                        feet.land = (top >= LAND_MIN_FALL_M).then_some(Land { tick, fall_m: top });
                        feet.air = None;
                    } else {
                        feet.air = Some(top);
                    }
                }
            }
        }
    }

    /// Fold the authority's fall landings for `tick` into every slot's feet
    /// (FEEL7): the height channel names the tick, so the squash starts on the
    /// tick the fall table charged even though the frame's views are the
    /// pre-step ones. Called after [`FighterLook::observe_landings`] so the
    /// cleared air top keeps the next pre-step view from landing the same fall
    /// twice. Presentation only; the sim hash never reads it.
    pub fn observe_landing_rows(&mut self, tick: u32, landings: &[engine_arena::FallLanding]) {
        for landing in landings {
            let Some(slot) = self.slots.get_mut(usize::from(landing.slot)) else {
                continue;
            };
            if slot.held {
                continue;
            }
            let fall_m = f32::from(u16::try_from(landing.fall_mm).unwrap_or(u16::MAX)) / 1_000.0;
            slot.feet.air = None;
            slot.feet.land = Some(Land { tick, fall_m });
        }
    }

    /// Test-only: the two-slot look fixture for tests outside the look's module
    /// tree (FEEL7's one-tick landing proof).
    #[cfg(test)]
    pub(crate) fn feel7_look_two() -> Self {
        super::tests::balance_look_two()
    }

    /// Test-only: the last landing's tick and its extra pelvis drop at `tick`,
    /// in meters. This is the same curve [`plant_feet`](Self::plant_feet)
    /// applies to the pose.
    #[cfg(test)]
    pub(crate) fn feel7_landing_squash(&self, tick: u32) -> Option<(u32, f32)> {
        self.slots
            .first()
            .and_then(|slot| slot.feet.land)
            .map(|land| (land.tick, land.squash_drop_m(tick)))
    }

    /// Fold one tick into every seen, free slot's feet: read the clip's ankles
    /// at the end of the tick and step each plant. A held slot keeps its feet
    /// exactly (they stand still with its clip clock); a slot that no longer
    /// stands drops its plants. Presentation only; the sim hash never reads it.
    pub(super) fn fold_feet(&mut self, elapsed: u32) {
        let next: [Option<Feet>; super::MAX_LOOK_FIGHTERS] =
            std::array::from_fn(|index| self.next_feet(index, elapsed));
        for (slot, feet) in self.slots.iter_mut().zip(next) {
            if let Some(feet) = feet {
                slot.feet = feet;
            }
        }
    }

    /// The feet one slot has after this tick, or `None` to keep them as they
    /// are (unseen, held, a rig without legs, a pose that will not sample: the
    /// frame path reports the last one loudly).
    fn next_feet(&self, index: usize, elapsed: u32) -> Option<Feet> {
        let slot = self.slots.get(index)?;
        if !slot.seen || slot.held {
            return None;
        }
        let rig = self.rigs.get(self.rig_for_slot(index))?;
        let legs = rig.legs?;
        // A slot lying as a ragdoll or rising out of one plants nothing (RZ2w): its ankles are
        // moving by physics or by the get-up blend, not held by a clip.
        if !plants_feet(slot) || self.ragdoll_rests(index) {
            return Some(Feet {
                plants: [Plant::NEW; 2],
                ..slot.feet
            });
        }
        // The ankle the frame draws, not the bare clip's: the end-of-tick pose
        // the angular limiter is handed, capped as it will cap it. Off the
        // clip ankle a just-switched leg trails by up to 18 cm while the cap
        // turns it, and a plant held there holds a spot the foot is not on.
        let mut pose = self.inertial_pose(rig, index, slot, 1.0 - LAG_TICKS).ok()?;
        self.limit_end_pose(index, &mut pose.rotations, elapsed);
        let (pos, yaw) = slot_root(slot, 1.0);
        let mut plants = slot.feet.plants;
        for ((plant, limb), rest_y) in plants.iter_mut().zip(legs.limbs).zip(legs.rest_y) {
            let ankle = limb_positions(&rig.skeleton, &pose, limb).ok()?[2];
            let world = to_world_xz(ankle, pos, yaw);
            *plant = step_plant(*plant, ankle, rest_y, world, elapsed);
        }
        Some(Feet {
            plants,
            ..slot.feet
        })
    }

    /// Plant the feet of a slot's pose at sub-tick `alpha`, after the inertial
    /// offsets. Each foot with a plant is aimed at the clip's ankle moved by
    /// the plant's offset (the clip's height kept); the pelvis drops by the
    /// largest gap the legs cannot reach, the other leg keeps the clip's ankle
    /// where it was, and each aimed leg is solved with the knee bending
    /// forward. `yaw` is the slot's facing this frame; the root's place on the
    /// sand never enters. Returns the spots held at full plant for the slide
    /// meter. A slot with no plant, no standing pose or `ik` off leaves the
    /// pose as the clip made it. A held slot reads the end of its tick like its
    /// clip clock does, so its feet stand still.
    pub(super) fn plant_feet(
        &self,
        rig: &Rig,
        slot: &SlotAnim,
        pose: &mut LocalPose,
        yaw: f32,
        alpha: f32,
    ) -> Result<Contacts, LookError> {
        let Some(legs) = rig.legs.filter(|_| plants_feet(slot)) else {
            return Ok([None; 2]);
        };
        // FEEL7: a fresh landing drops the pelvis a little more, so the knees
        // bend and the body reads its weight. It eases out over a few ticks.
        let squash = slot
            .feet
            .land
            .map_or(0.0, |land| land.squash_drop_m(self.tick));
        let at = clip_alpha(slot, alpha);
        let mut contacts: Contacts = [None; 2];
        let mut clip = [ClipLeg::NONE; 2];
        for (((leg, contact), limb), plant) in clip
            .iter_mut()
            .zip(contacts.iter_mut())
            .zip(legs.limbs)
            .zip(slot.feet.plants)
        {
            let [hip, knee, ankle] = limb_positions(&rig.skeleton, pose, limb).map_err(skin_err)?;
            leg.hip = hip;
            leg.pole = knee_pole(hip, knee, ankle);
            leg.ankle = ankle;
            leg.reach = dist3(hip, knee) + dist3(knee, ankle);
            if plant.moves() {
                // The offset is a world vector, so the aim moves the clip's ankle
                // by it turned into the body's frame: the root's place on the
                // sand drops out, and a body that stands still in the world
                // keeps its pose from frame to frame.
                let offset = to_model_vec(plant.offset_at(at), yaw);
                leg.aim = Some([ankle[0] + offset[0], ankle[1], ankle[2] + offset[1]]);
                *contact = plant.planted.then_some(plant.contact);
            }
        }
        if !slot.feet.ik || (squash <= 0.0 && clip.iter().all(|leg| leg.aim.is_none())) {
            return Ok(contacts);
        }
        let gap = clip
            .iter()
            .filter_map(|leg| leg.aim.map(|t| gap_m(leg.hip, leg.ankle, t, leg.reach)))
            .fold(0.0f32, f32::max);
        let drop = (gap + squash).min(MAX_DROP_M);
        if drop > 0.0 {
            pose.positions[rig.pelvis][1] -= drop;
            // The leg that has no plant keeps the clip's ankle where it was.
            for leg in &mut clip {
                leg.aim.get_or_insert(leg.ankle);
            }
        }
        for (leg, limb) in clip.iter().zip(legs.limbs) {
            if let Some(target) = leg.aim {
                solve_limb(&rig.skeleton, pose, limb, target, leg.pole).map_err(skin_err)?;
            }
        }
        Ok(contacts)
    }
}

/// How far each fully planted foot is drawn from its held spot, in meters:
/// the drawn ankle in `world` (the skinning matrices of the finished pose)
/// against the spot. `None` for a foot that is not at full plant.
pub(super) fn foot_slides(
    legs: Option<Legs>,
    world: &[[[f32; 4]; 4]],
    contacts: &Contacts,
    root: ([f32; 3], f32),
) -> [Option<f32>; 2] {
    let mut slides = [None; 2];
    let Some(legs) = legs else {
        return slides;
    };
    for ((slide, spot), limb) in slides.iter_mut().zip(contacts).zip(legs.limbs) {
        let (Some(spot), Some(joint)) = (spot, world.get(limb.end)) else {
            continue;
        };
        let drawn = to_world_xz([joint[3][0], joint[3][1], joint[3][2]], root.0, root.1);
        *slide = Some((drawn[0] - spot[0]).hypot(drawn[1] - spot[1]));
    }
    slides
}

#[cfg(test)]
mod tests {
    use super::*;

    const REST: f32 = 0.075;

    #[test]
    fn feel7_a_fall_records_one_landing_and_its_squash_drop_curve() {
        let mut look = super::super::tests::balance_look_two();
        look.observe_landings(
            10,
            &[FighterView {
                slot: 0,
                pos_m: [0.0, 1.0, 0.0],
                yaw: 0.0,
            }],
        );
        look.observe_landings(
            11,
            &[FighterView {
                slot: 0,
                pos_m: [0.0, 1.5, 0.0],
                yaw: 0.0,
            }],
        );
        look.observe_landings(
            12,
            &[FighterView {
                slot: 0,
                pos_m: [0.0, 0.0, 0.0],
                yaw: 0.0,
            }],
        );
        let land = look.slots[0].feet.land.expect("the fall lands");
        assert_eq!(land.tick, 12);
        assert!((land.fall_m - 1.5).abs() < 1e-6);
        assert!(land.squash_drop_m(12) > 0.05);
        assert!(land.squash_drop_m(16) < land.squash_drop_m(12));
        assert_eq!(land.squash_drop_m(12 + SQUASH_TICKS), 0.0);
    }

    /// An ankle at standing height `0.075`, `x` meters along the body.
    fn ankle(x: f32) -> [f32; 3] {
        [x, REST, 0.0]
    }

    /// Step a plant once a tick through ankles at the given model `x`, the
    /// body standing still in the world.
    fn run(mut plant: Plant, xs: &[f32]) -> Plant {
        for x in xs {
            plant = step_plant(plant, ankle(*x), REST, [*x, 0.0], 1);
        }
        plant
    }

    #[test]
    fn a_foot_plants_only_after_the_clip_holds_it_low_and_still() {
        // The first tick has no speed to read; a foot that moves 1 mm a tick
        // (0.06 m/s) needs PLANT_STILL_TICKS ticks of it before it plants,
        // and plants where it is drawn.
        let mut plant = run(Plant::NEW, &[0.0, 0.001, 0.002]);
        assert!(
            !plant.planted,
            "{} still tick(s) is not enough",
            plant.still
        );
        plant = run(plant, &[0.003]);
        assert!(plant.planted, "{PLANT_STILL_TICKS} still ticks plant");
        assert_eq!(plant.contact, [0.003, 0.0], "planted where it is drawn");
        assert_eq!(plant.offset, [[0.0; 2]; 2], "a fresh plant moves nothing");
        // A foot at 0.3 m/s (5 mm a tick) never plants.
        let fast = run(Plant::NEW, &[0.0, 0.005, 0.010, 0.015, 0.020, 0.025]);
        assert!(!fast.planted, "0.3 m/s is a moving foot");
        // A slow tick between fast ones (a foot turning round) never plants.
        let turning = run(Plant::NEW, &[0.0, 0.01, 0.0105, 0.02, 0.03, 0.0305, 0.04]);
        assert!(
            !turning.planted,
            "a foot passing through zero is not planted"
        );
        // A foot held still but lifted 10 cm off its standing height never
        // plants either.
        let mut lifted = Plant::NEW;
        for _ in 0..6 {
            lifted = step_plant(lifted, [0.0, REST + 0.10, 0.0], REST, [0.0, 0.0], 1);
        }
        assert!(!lifted.planted, "a raised foot is not on the sand");
    }

    #[test]
    fn a_planted_foot_lets_go_when_the_clip_moves_or_lifts_it() {
        let planted = run(Plant::NEW, &[0.0, 0.0, 0.0, 0.0]);
        assert!(planted.planted);
        // Between the plant and the release speeds the foot stays held.
        let held = step_plant(planted, ankle(0.003), REST, [0.003, 0.0], 1);
        assert!(held.planted, "0.18 m/s is under {RELEASE_SPEED_M_S}");
        // A clip foot that moves 1 cm a tick (0.6 m/s) lets go.
        let moving = step_plant(planted, ankle(0.01), REST, [0.01, 0.0], 1);
        assert!(!moving.planted);
        // So does one the clip lifts off the sand.
        let lifted = step_plant(planted, [0.0, REST + 0.10, 0.0], REST, [0.0, 0.0], 1);
        assert!(!lifted.planted);
    }

    #[test]
    fn a_stretched_plant_lets_go_runs_back_to_the_clip_and_plants_again() {
        // The body drifts 1 cm a tick (0.6 m/s) over a foot the clip holds
        // still, so the clip foot walks away from the held spot while the
        // drawn foot follows the spot through an offset.
        let mut plant = run(Plant::NEW, &[0.0, 0.0, 0.0, 0.0]);
        assert!(plant.planted);
        let (mut held, mut world_x) = (0u32, 0.0f32);
        while plant.planted && held < 100 {
            world_x += 0.01;
            plant = step_plant(plant, ankle(0.0), REST, [world_x, 0.0], 1);
            if plant.planted {
                let drawn = plant.offset[1][0] + world_x;
                assert!(
                    drawn.abs() < 1.0e-5,
                    "the drawn foot holds the spot: {drawn}"
                );
            }
            held += 1;
        }
        assert!(!plant.planted, "the plant must let go");
        assert!(
            (12..=13).contains(&held),
            "held {held} ticks to {MAX_PULL_M} m of pull"
        );
        // The offset runs back to the clip at STEP_M a tick, never jumping,
        // and the foot may not plant again until it is home.
        let mut ticks = 0u32;
        while plant.offset[1] != [0.0; 2] {
            world_x += 0.01;
            let before = plant.offset[1];
            plant = step_plant(plant, ankle(0.0), REST, [world_x, 0.0], 1);
            let moved = (plant.offset[1][0] - before[0]).hypot(plant.offset[1][1] - before[1]);
            assert!(moved <= STEP_M + 1.0e-6, "offset jumped {moved} m");
            assert!(!plant.planted, "a foot still running home does not plant");
            ticks += 1;
            assert!(ticks < 20, "the offset never got home");
        }
        assert!((5..=7).contains(&ticks), "ran home in {ticks} ticks");
        // Home and still: it plants again, at the clip's own spot.
        world_x += 0.01;
        plant = step_plant(plant, ankle(0.0), REST, [world_x, 0.0], 1);
        assert!(plant.planted);
        assert_eq!(plant.contact, [world_x, 0.0]);
    }

    #[test]
    fn the_foot_never_snaps_when_the_root_outruns_the_offset() {
        // The root lunges 8 cm in one tick over a still clip foot. The offset
        // cannot follow that, so the foot is not planted and does not move.
        let planted = run(Plant::NEW, &[0.0, 0.0, 0.0, 0.0]);
        let lunged = step_plant(planted, ankle(0.0), REST, [0.08, 0.0], 1);
        assert!(!lunged.planted, "8 cm in a tick is past {STEP_M} m");
        assert_eq!(lunged.offset, [[0.0; 2]; 2], "no jump at the let-go");
        // A foot already pulled 8 cm behind its clip foot when the lunge
        // comes runs back at STEP_M a tick, not at once.
        let mut drifting = planted;
        for k in 1..=8u8 {
            drifting = step_plant(drifting, ankle(0.0), REST, [0.01 * f32::from(k), 0.0], 1);
        }
        assert!(drifting.planted);
        assert!(
            (drifting.offset[1][0] + 0.08).abs() < 1.0e-5,
            "{:?}",
            drifting.offset
        );
        let lunged = step_plant(drifting, ankle(0.0), REST, [0.16, 0.0], 1);
        assert!(!lunged.planted);
        assert!(
            (lunged.offset[1][0] - (drifting.offset[1][0] + STEP_M)).abs() < 1.0e-6,
            "{:?}",
            lunged.offset
        );
        // The frame reads the lerp between the tick's two offsets.
        let half = lunged.offset_at(0.5);
        assert!((half[0] - (drifting.offset[1][0] + 0.5 * STEP_M)).abs() < 1.0e-6);
        assert_eq!(lunged.offset_at(1.0), lunged.offset[1]);
        // A tick that skips ticks is that many steps of room: 0.1 m of drift
        // over five ticks is 2 cm a tick, which the offset can follow.
        let skipped = step_plant(planted, ankle(0.0), REST, [0.10, 0.0], 5);
        assert!(skipped.planted);
        assert!(
            (skipped.offset[1][0] + 0.10).abs() < 1.0e-6,
            "{:?}",
            skipped.offset
        );
        // On a body that moves in a line the drawn foot sits on the spot at
        // every sub-tick: the clip foot walks 1 cm a tick along world x.
        let mut plant = planted;
        let mut clip_x = 0.0f32;
        for _ in 0..8 {
            let start = clip_x;
            clip_x += 0.01;
            plant = step_plant(plant, ankle(0.0), REST, [clip_x, 0.0], 1);
            for alpha in [0.0f32, 0.25, 0.5, 0.75, 1.0] {
                let drawn = start + 0.01 * alpha + plant.offset_at(alpha)[0];
                assert!(
                    drawn.abs() < 1.0e-5,
                    "alpha {alpha}: the foot is {drawn} m off its spot"
                );
            }
        }
    }

    #[test]
    fn a_run_stance_foot_plants_on_the_sand_and_lets_go_when_it_lifts() {
        // A 3 m/s runner: the clip sweeps the stance ankle back 5 cm a tick
        // against the body while the body carries it 5 cm forward, so on the
        // sand it stands still. It plants after PLANT_STILL_TICKS, stays held
        // through the stance, and lets go on the tick the clip lifts it.
        let mut plant = Plant::NEW;
        for k in 0..=PLANT_STILL_TICKS {
            let model_x = -0.05 * f32::from(k);
            plant = step_plant(plant, ankle(model_x), REST, [0.4, 0.0], 1);
        }
        assert!(plant.planted, "a still stance foot plants mid-run");
        assert_eq!(plant.contact, [0.4, 0.0]);
        for k in 1..=6u8 {
            let model_x = -0.05 * f32::from(PLANT_STILL_TICKS + k);
            plant = step_plant(
                plant,
                ankle(model_x),
                REST,
                [0.4 + 0.001 * f32::from(k), 0.0],
                1,
            );
            assert!(plant.planted, "stance tick {k}: still held");
        }
        let lifted = step_plant(plant, [-0.5, REST + 0.10, 0.0], REST, [0.41, 0.0], 1);
        assert!(!lifted.planted, "the swing lifts the foot off its spot");
        // A swing foot moving on the sand as fast as the body never plants.
        let mut swing = Plant::NEW;
        for k in 0..6u8 {
            let x = 0.05 * f32::from(k);
            swing = step_plant(swing, ankle(0.0), REST, [x, 0.0], 1);
        }
        assert!(!swing.planted, "3 m/s on the sand is a moving foot");
    }

    #[test]
    fn a_tick_that_skips_ticks_scales_the_speed() {
        // 1 mm moved over one tick is 0.06 m/s; 4 mm over one tick is 0.24 m/s
        // (moving), while the same 4 mm over three ticks is 0.08 m/s (still).
        let plants = |dx: f32, ticks: u32| {
            let mut plant = step_plant(Plant::NEW, ankle(0.0), REST, [0.0, 0.0], 1);
            for k in 1..=PLANT_STILL_TICKS {
                let x = dx * f32::from(k);
                plant = step_plant(plant, ankle(x), REST, [x, 0.0], ticks);
            }
            plant.planted
        };
        assert!(plants(0.001, 1));
        assert!(!plants(0.004, 1), "4 mm in one tick is 0.24 m/s");
        assert!(plants(0.004, 3), "4 mm over three ticks is 0.08 m/s");
        // A zero-tick call (the same tick folded twice) counts as one.
        assert!(plants(0.001, 0));
    }

    #[test]
    fn the_pelvis_drops_by_the_gap_the_leg_cannot_reach_and_never_without_one() {
        let hip = [0.0, 0.9, 0.0];
        let clip = [0.0, 0.075, 0.0];
        let reach = 0.95;
        // Inside the comfortable reach: no drop.
        assert_eq!(gap_m(hip, clip, [0.05, 0.075, 0.0], reach), 0.0);
        // A foot pulled 0.6 m away needs the hip to sink: afterwards the
        // target sits on the reach sphere about the lowered hip.
        let target = [0.6, 0.075, 0.0];
        let drop = gap_m(hip, clip, target, reach);
        assert!(drop > 0.05, "drop {drop}");
        let lowered = [hip[0], hip[1] - drop, hip[2]];
        let limit = COMFORT_REACH * reach;
        assert!(
            (dist3(lowered, target) - limit).abs() < 1.0e-4,
            "{} vs {limit}",
            dist3(lowered, target)
        );
        // The clip's own stretch is kept: a leg the clip already straightens
        // past the comfortable reach does not drop the pelvis to hold it there.
        let straight = [0.0, -0.04, 0.0];
        assert!(dist3(hip, straight) > COMFORT_REACH * reach);
        assert_eq!(gap_m(hip, straight, straight, reach), 0.0);
        // Out of reach for any drop: the whole height above the target.
        let far = gap_m(hip, clip, [3.0, 0.075, 0.0], reach);
        assert!((far - 0.825).abs() < 1.0e-5, "{far}");
    }

    #[test]
    fn a_world_vector_turns_into_the_body_frame_and_back_under_any_facing() {
        let pos = [3.0, 0.0, -2.0];
        for yaw in [0.0f32, 0.7, -2.4, 3.0] {
            let model = [0.12, 0.5, -0.3];
            let world = to_world_xz(model, pos, yaw);
            let back = to_model_vec([world[0] - pos[0], world[1] - pos[2]], yaw);
            assert!(
                (back[0] - model[0]).abs() < 1.0e-5 && (back[1] - model[2]).abs() < 1.0e-5,
                "yaw {yaw}: {back:?}"
            );
        }
        // Forward (+z in the model) maps to (sin yaw, cos yaw), as root_matrix does.
        let ahead = to_world_xz([0.0, 0.0, 1.0], [0.0; 3], 0.5);
        assert!(
            (ahead[0] - 0.5f32.sin()).abs() < 1.0e-6 && (ahead[1] - 0.5f32.cos()).abs() < 1.0e-6
        );
    }

    #[test]
    fn a_knee_keeps_the_clips_bend_and_a_straight_leg_bends_forward() {
        let (hip, ankle) = ([0.0, 0.9, 0.0], [0.0, 0.1, 0.0]);
        // The knee sits 5 cm out and 20 cm forward of the hip-to-ankle line:
        // that offset is the pole.
        let pole = knee_pole(hip, [0.05, 0.5, 0.2], ankle);
        assert!(
            (pole[0] - 0.05).abs() < 1.0e-6
                && pole[1].abs() < 1.0e-6
                && (pole[2] - 0.2).abs() < 1.0e-6,
            "{pole:?}"
        );
        // A leg too straight to name a side, and one with no length, bend forward.
        assert_eq!(knee_pole(hip, [0.0, 0.5, 0.001], ankle), KNEE_POLE);
        assert_eq!(knee_pole(hip, [0.05, 0.5, 0.2], hip), KNEE_POLE);
    }

    /// Seven joints: a pelvis and both legs, named as the rigs name them.
    fn two_leg_skeleton() -> Skeleton {
        let joint = |name: &str, parent: Option<usize>| engine_anim::contract::Joint {
            name: name.to_string(),
            parent,
        };
        Skeleton {
            joints: vec![
                joint("pelvis", None),
                joint("leg_upper_l", Some(0)),
                joint("leg_lower_l", Some(1)),
                joint("foot_l", Some(2)),
                joint("leg_upper_r", Some(0)),
                joint("leg_lower_r", Some(4)),
                joint("foot_r", Some(5)),
            ],
        }
    }

    #[test]
    fn legs_are_found_by_name_with_their_standing_height_and_slide_is_read_off_the_ankle() {
        let skeleton = two_leg_skeleton();
        let mut bind = vec![[0.0f32; 3]; skeleton.len()];
        bind[3][1] = 0.07;
        bind[6][1] = 0.08;
        let legs = Legs::find(&skeleton, &bind).expect("both legs are there");
        assert_eq!(legs.rest_y, [0.07, 0.08]);
        assert_eq!(legs.limbs[0].end, 3);
        assert_eq!(legs.limbs[1].end, 6);
        // A missing leg, and a bind pose too short to hold the ankles, find none.
        let mut broken = skeleton.clone();
        broken.joints[5].name = "shin_r".to_string();
        assert!(Legs::find(&broken, &bind).is_none());
        assert!(Legs::find(&skeleton, &bind[..5]).is_none());
        // The slide: the drawn left ankle at model (0.1, _, 0.2) under a root
        // at (1, 0, 2) facing +z sits at world (1.1, 2.2).
        let mut world = vec![[[0.0f32; 4]; 4]; skeleton.len()];
        world[3][3] = [0.1, 0.075, 0.2, 1.0];
        world[6][3] = [-0.1, 0.075, 0.2, 1.0];
        let root = ([1.0, 0.0, 2.0], 0.0);
        let on_spot = foot_slides(Some(legs), &world, &[Some([1.1, 2.2]), None], root);
        assert!(on_spot[0].is_some_and(|m| m < 1.0e-6) && on_spot[1].is_none());
        let off_spot = foot_slides(
            Some(legs),
            &world,
            &[Some([1.1, 2.25]), Some([0.9, 2.2])],
            root,
        );
        assert!(
            off_spot[0].is_some_and(|m| (m - 0.05).abs() < 1.0e-5),
            "{off_spot:?}"
        );
        assert!(off_spot[1].is_some_and(|m| m < 1.0e-6), "{off_spot:?}");
        // No legs, no slide; a spot with no joint behind it is skipped.
        assert_eq!(
            foot_slides(None, &world, &[Some([0.0; 2]); 2], root),
            [None; 2]
        );
        assert_eq!(
            foot_slides(Some(legs), &[], &[Some([0.0; 2]); 2], root),
            [None; 2]
        );
    }
}
