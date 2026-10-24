//! hit: a standing blow moves the body that took it (FEEL9).
//!
//! A second, smaller world next to the fall: a chain of the pelvis, torso, head and both arms
//! seeded from the pose the viewer saw, its chest moved by [`PhysFx::hit_ragdoll`] through the
//! blow's own brace motors, carried at the fighter's pelvis while the blow plays out, then blended
//! back over the clip inside [`REACT_TICKS`]. The sim never reads it and a fall wins over it on
//! the same tick. Presentation only: nothing here is hashed.

use engine_anim::contract::LocalPose;
use engine_arena::GaitPhase;
use engine_audio::FightCue;
use engine_core::quat_mul;
use engine_physfx::{Error as FxError, Key, PhysFx, Rig as FxRig, Transform};

use super::{
    add, bone_parent, conj, forward, length, placed, planar_unit, rig_of, rotate, slot_root, sub,
    unit, wall_micros, yaw_quat, Kick, RagdollLook, BONES, MAX_DAMAGE_PERMILLE, MAX_START_MS,
    TICK_HZ,
};
use crate::fighter_look::{FighterLook, LookError, MAX_LOOK_FIGHTERS};

/// Ticks a standing hit reaction shows: the blow's brace window (`HIT_FADE_S`, 0.3 s at 60 Hz),
/// after which the clip is whole again (FEEL9).
pub(super) const REACT_TICKS: u32 = 18;
/// Ticks the reaction weight rises from zero: jumping from the clip to a struck chain in one frame
/// would pop (the view law judges a joint 10 cm off its own constant-velocity prediction).
pub(super) const REACT_RISE_TICKS: u8 = 3;
/// Ticks the reaction holds full weight before it blends back.
pub(super) const REACT_HOLD_TICKS: u8 = 6;
/// Ticks the blend back takes; with the hold it sums to [`REACT_TICKS`] (a test pins that).
pub(super) const REACT_FADE_TICKS: u8 = 12;
/// Reach of the reaction blow around the chest, meters: the torso, head and both upper arms.
const REACT_RADIUS_M: f32 = 0.5;
/// Newton-seconds of chest blow per permille of damage: lighter than the fall's root kick, so the
/// chain sways and returns instead of folding.
const REACT_NS_PER_PERMILLE: f32 = 0.1;
/// Share of the reaction blow that goes up, of its planar length.
const REACT_LIFT: f32 = 0.15;

/// The upper chain a torso hit moves (FEEL9), as indices of [`BONES`], parents first: pelvis,
/// torso, head, both arms. The sim's fight cue carries no struck bone, so a leg hit is not
/// modelled: every landed blow is a chest blow here.
const REACT_CHAIN: [usize; 9] = [0, 1, 2, 9, 10, 11, 12, 13, 14];

/// First [`BONES`] index of the arm chain: its light bones are simulated but not shown, because a
/// blow flings them past the view law's frame-to-frame bound.
const REACT_ARM_FROM: usize = 9;

/// The standing hit reaction chain (FEEL9): the upper body only.
pub(super) fn reaction_rig() -> Option<FxRig> {
    rig_of(&REACT_CHAIN)
}

/// The parent of reaction-chain bone `index`, as a chain index.
fn react_parent(index: usize) -> Option<usize> {
    bone_parent(&REACT_CHAIN, REACT_CHAIN[index])
}

/// One standing hit reaction (FEEL9): a reduced chain carried at the fighter's pelvis, its chest
/// moved by `PhysFx::hit_ragdoll` and blended back over the clip inside [`REACT_TICKS`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Reaction {
    /// Look rig (skeleton variant) the chain was taken from.
    rig: usize,
    /// Ticks since the blow.
    age: u32,
    /// Pelvis capsule center at the blow, world meters: the carry target moves from here.
    anchor: [f32; 3],
    /// Slot position at the blow, meters: the chain rides the fighter's own travel.
    at: [f32; 3],
    /// Each chain bone's orientation in the pelvis frame at the blow: the blow's own bend is
    /// measured from here, so a clip swing never reads as a hit.
    spawn: [[f32; 4]; REACT_CHAIN.len()],
    /// The clip pose the chain was driven toward last tick (world transforms, chain order), or
    /// `None` before the first drive. SW170: the pose drive needs two ticks of the clip.
    prev_clip: Option<[Transform; REACT_CHAIN.len()]>,
}

/// The chain a blow is seeded from (FEEL9): the local pose the ragdoll takes, the world place of
/// every chain bone, the chest to hit and the carry velocity.
struct Seed {
    /// Chain pose in the pelvis frame, as `spawn_ragdoll` takes it.
    local: LocalPose,
    /// Every chain bone's world transform at the blow, in chain order.
    place: Vec<Transform>,
    /// The chest to land the blow on, world meters.
    chest: [f32; 3],
    /// Velocity the chain inherits from the fighter's travel, m/s.
    velocity: [f32; 3],
    /// Slot position at the blow: the chain rides the fighter's own travel.
    at: [f32; 3],
}

impl Reaction {
    /// A fresh chain reaction at the blow: age zero, no drive pose yet (SW170 drives from the
    /// second tick the chain lives).
    fn new(
        rig: usize,
        anchor: [f32; 3],
        at: [f32; 3],
        spawn: [[f32; 4]; REACT_CHAIN.len()],
    ) -> Self {
        Self {
            rig,
            age: 0,
            anchor,
            at,
            spawn,
            prev_clip: None,
        }
    }
}

impl RagdollLook {
    /// The reaction world (FEEL9), made on first use: the upper chain alone and no floor, so a
    /// standing blow costs nine bodies for [`REACT_TICKS`] and nothing before it.
    fn react_world(&mut self) -> Option<&mut PhysFx> {
        if self.react.is_none() {
            self.react = Some(PhysFx::new(self.tier).with_clock(wall_micros));
        }
        self.react.as_mut()
    }
}

/// The chest blow of one standing hit (FEEL9): planar away from the attacker, a little up.
fn react_impulse(kick: &Kick) -> [f32; 3] {
    let ns = f32::from(kick.damage.min(MAX_DAMAGE_PERMILLE)) * REACT_NS_PER_PERMILLE;
    [kick.dir[0] * ns, ns * REACT_LIFT, kick.dir[1] * ns]
}

/// The blend weight of a standing hit reaction `age` ticks after the blow, with `alpha` the frame's
/// share of the next tick (FEEL9): rise, hold, then fall to zero at [`REACT_TICKS`].
pub(super) fn react_weight(age: u32, alpha: f32) -> f32 {
    if age >= REACT_TICKS {
        return 0.0;
    }
    let at = f32::from(u16::try_from(age).unwrap_or(u16::MAX)) + alpha.clamp(0.0, 1.0);
    let rise = (at / f32::from(REACT_RISE_TICKS)).min(1.0);
    let hold = f32::from(REACT_HOLD_TICKS);
    let fade = if at <= hold {
        1.0
    } else {
        (1.0 - (at - hold) / f32::from(REACT_FADE_TICKS)).max(0.0)
    };
    rise * fade
}

/// The pose-drive weight of a standing hit reaction `age` ticks after the blow (SW170): the
/// complement of [`react_weight`]. The blow tick and the full-weight window keep the chain free
/// (exactly the bytes this file wrote before the drive existed), and the fade hands the chain to the
/// drive, which rises to the clip at the clip's own speed instead of a blend step. Zero at the blow
/// and zero once the reaction is over.
pub(super) fn react_drive_weight(age: u32) -> f32 {
    if age == 0 || age >= REACT_TICKS {
        return 0.0;
    }
    1.0 - react_weight(age, 0.0)
}

/// Turns `q` by `weight` of its angle about the same axis: a normalized lerp from the identity,
/// the short way round, so a blend never flips a joint.
fn turn_by(q: [f32; 4], weight: f32) -> [f32; 4] {
    let w = weight.clamp(0.0, 1.0);
    let q = if q[3] < 0.0 { q.map(|c| -c) } else { q };
    unit([q[0] * w, q[1] * w, q[2] * w, 1.0 + (q[3] - 1.0) * w])
}

impl FighterLook {
    /// A `Hit` cue on a standing fighter starts a hit reaction (FEEL9): the blow is queued and
    /// applied once the clips have folded. A floored, latched or falling slot, and one whose own
    /// same-tick `Knockdown` cue is present, is left to the fall. Called from the cue fold.
    pub(crate) fn ragdoll_note_hit(
        &mut self,
        attacker: u8,
        victim: u8,
        damage: u16,
        cues: &[FightCue],
    ) {
        let index = usize::from(victim);
        if !self.ragdoll.on || index >= MAX_LOOK_FIGHTERS {
            return;
        }
        if self.ragdoll.has(index) || self.ragdoll.latched[index] {
            return;
        }
        if self.slots[index].balance == GaitPhase::Fallen {
            return;
        }
        let falling = cues
            .iter()
            .any(|cue| matches!(*cue, FightCue::Knockdown { victim: v, .. } if v == victim));
        if falling {
            return;
        }
        let dir = match (self.slots.get(usize::from(attacker)), self.slots.get(index)) {
            (Some(a), Some(v)) if a.seen && v.seen => {
                planar_unit(v.pos[0] - a.pos[0], v.pos[2] - a.pos[2])
            }
            _ => [0.0, 0.0],
        };
        self.ragdoll.react_pending[index] = Some(Kick {
            dir,
            damage,
            standing: self.slots[index],
        });
    }

    /// Applies the standing hit reactions queued this tick (FEEL9): spawns a chain for a first
    /// blow, hits a live one again where its chest is now, carries every chain at its fighter's
    /// pelvis, steps the reaction world once and ages the blends. A held slot's chain waits with
    /// its pair (LF10), like the fall does. Presentation only.
    pub(super) fn ragdoll_reactions(&mut self) {
        for index in 0..MAX_LOOK_FIGHTERS {
            let Some(kick) = self.ragdoll.react_pending[index].take() else {
                continue;
            };
            self.ragdoll_react(index, kick);
        }
        if self.ragdoll.reactions.iter().all(Option::is_none) {
            return;
        }
        let held = (0..MAX_LOOK_FIGHTERS)
            .any(|i| self.ragdoll.reactions[i].is_some() && self.slots[i].held);
        for index in 0..MAX_LOOK_FIGHTERS {
            let Some(reaction) = self.ragdoll.reactions[index] else {
                continue;
            };
            let slot = self.slots[index];
            let target = [
                reaction.anchor[0] + (slot.pos[0] - reaction.at[0]),
                reaction.anchor[1],
                reaction.anchor[2] + (slot.pos[2] - reaction.at[2]),
            ];
            let velocity = [
                (slot.pos[0] - slot.prev_pos[0]) * TICK_HZ,
                0.0,
                (slot.pos[2] - slot.prev_pos[2]) * TICK_HZ,
            ]
            .map(|v| {
                if v.is_finite() {
                    v.clamp(-MAX_START_MS, MAX_START_MS)
                } else {
                    0.0
                }
            });
            if let (Ok(slot_u8), Some(fx)) = (u8::try_from(index), self.ragdoll.react.as_mut()) {
                // A refused track leaves the chain free; the next tick tries again.
                let _ = fx.carry_ragdoll(slot_u8, target, velocity, true);
            }
        }
        if !held {
            if let Some(fx) = self.ragdoll.react.as_mut() {
                fx.step();
            }
            self.drive_reactions();
        }
        for index in 0..MAX_LOOK_FIGHTERS {
            let Some(reaction) = self.ragdoll.reactions[index].as_mut() else {
                continue;
            };
            if !self.slots[index].held {
                reaction.age = reaction.age.saturating_add(1);
            }
            if reaction.age >= REACT_TICKS {
                self.ragdoll.reactions[index] = None;
            }
        }
    }

    /// Applies one standing blow (FEEL9): a live chain takes it at its chest again, a slot with no
    /// chain gets one seeded from the pose it shows. A refusal leaves the slot on its clip.
    fn ragdoll_react(&mut self, index: usize, kick: Kick) {
        let Ok(slot_u8) = u8::try_from(index) else {
            return;
        };
        let impulse = react_impulse(&kick);
        if self.recharge_reaction(index, slot_u8, impulse) {
            return;
        }
        let Some(slot) = self.slots.get(index).copied() else {
            return;
        };
        if !slot.seen {
            return;
        }
        // A first blow, or a chain the world recycled: build a fresh one from the shown pose.
        self.ragdoll.reactions[index] = None;
        let rig_index = self.rig_for_slot(index);
        let Some(layout) = self
            .ragdoll
            .layouts
            .get(rig_index)
            .and_then(Option::as_ref)
            .cloned()
        else {
            return;
        };
        let Some(seed) = self.seed_reaction(index, rig_index, &layout.joint_of, &layout.tip_of)
        else {
            return;
        };
        let Some(shape) = self.ragdoll.react_rig.take() else {
            return;
        };
        let result = match self.ragdoll.react_world() {
            Some(fx) => fx
                .spawn_ragdoll(slot_u8, &shape, &seed.local, seed.velocity, [0.0; 3])
                .and_then(|()| {
                    fx.hit_ragdoll(slot_u8, seed.chest, REACT_RADIUS_M, impulse)
                        .map(|_| ())
                }),
            None => Err(FxError::BadCaps),
        };
        self.ragdoll.react_rig = Some(shape);
        if result.is_err() {
            return;
        }
        let mut spawn = [[0.0, 0.0, 0.0, 1.0]; REACT_CHAIN.len()];
        let sim_root = seed.place[0].rot;
        for (index, one) in seed.place.iter().enumerate().skip(1) {
            spawn[index] = unit(quat_mul(conj(sim_root), one.rot));
        }
        self.ragdoll.reactions[index] =
            Some(Reaction::new(rig_index, seed.place[0].pos, seed.at, spawn));
        self.ragdoll.reacted = self.ragdoll.reacted.saturating_add(1);
    }

    /// A live chain takes the blow again at its chest (FEEL9): false when the slot has no chain or
    /// no reaction world, so the caller seeds a fresh one.
    fn recharge_reaction(&mut self, index: usize, slot_u8: u8, impulse: [f32; 3]) -> bool {
        if self.ragdoll.reactions[index].is_none() || self.ragdoll.react.is_none() {
            return false;
        }
        let chest = self
            .ragdoll
            .react
            .as_ref()
            .and_then(|fx| {
                placed(
                    fx,
                    Key::Bone {
                        slot: slot_u8,
                        bone: 1,
                    },
                    1.0,
                )
            })
            .map(|at| at.pos);
        let hit = chest.is_some_and(|chest| {
            self.ragdoll.react.as_mut().is_some_and(|fx| {
                fx.hit_ragdoll(slot_u8, chest, REACT_RADIUS_M, impulse)
                    .is_ok()
            })
        });
        if hit {
            if let Some(reaction) = self.ragdoll.reactions[index].as_mut() {
                reaction.age = 0;
            }
        }
        hit
    }

    /// The chain seeded at the blow (FEEL9): the local pose `spawn_ragdoll` takes, the world place
    /// of every chain bone, the chest to hit and the carry velocity. `None` when the slot or its
    /// rig is gone.
    fn seed_reaction(
        &self,
        index: usize,
        rig_index: usize,
        joint_of: &[usize],
        tip_of: &[Option<usize>],
    ) -> Option<Seed> {
        let slot = self.slots.get(index).copied()?;
        let rig = self.rigs.get(rig_index)?;
        let pose = self.inertial_pose(rig, index, &slot, 0.0).ok()?;
        let (wp, wq) = forward(&rig.skeleton, &pose);
        // The chain goes into the world like a fall does: the slot root places it, or its
        // rotations would be read back without the fighter's yaw.
        let root = slot_root(&slot, 0.0);
        let turn = yaw_quat(root.1);
        let world_pos: Vec<[f32; 3]> = wp.iter().map(|p| add(root.0, rotate(turn, *p))).collect();
        let world_rot: Vec<[f32; 4]> = wq.iter().map(|q| unit(quat_mul(turn, *q))).collect();
        let mut place: Vec<Transform> = Vec::with_capacity(REACT_CHAIN.len());
        for bone in REACT_CHAIN {
            let joint = *joint_of.get(bone)?;
            let len = match tip_of.get(bone).copied().flatten() {
                Some(tip) => length(sub(world_pos[tip], world_pos[joint])),
                None => BONES[bone].fixed_len_m,
            };
            let center = 0.5 * len.clamp(0.04, 1.5);
            place.push(Transform {
                pos: add(
                    world_pos[joint],
                    rotate(world_rot[joint], [0.0, center, 0.0]),
                ),
                rot: world_rot[joint],
            });
        }
        let mut local = LocalPose {
            positions: vec![[0.0; 3]; REACT_CHAIN.len()],
            rotations: vec![[0.0, 0.0, 0.0, 1.0]; REACT_CHAIN.len()],
        };
        for (at, one) in place.iter().enumerate() {
            match react_parent(at) {
                Some(parent) => {
                    let up = conj(place[parent].rot);
                    local.positions[at] = rotate(up, sub(one.pos, place[parent].pos));
                    local.rotations[at] = unit(quat_mul(up, one.rot));
                }
                None => {
                    local.positions[at] = one.pos;
                    local.rotations[at] = one.rot;
                }
            }
        }
        let velocity = [
            (slot.pos[0] - slot.prev_pos[0]) * TICK_HZ,
            0.0,
            (slot.pos[2] - slot.prev_pos[2]) * TICK_HZ,
        ]
        .map(|v| {
            if v.is_finite() {
                v.clamp(-MAX_START_MS, MAX_START_MS)
            } else {
                0.0
            }
        });
        Some(Seed {
            local,
            chest: place[1].pos,
            place,
            velocity,
            at: slot.pos,
        })
    }

    /// The clip pose the reaction chain of `index` is driven toward this tick (SW170): the look's
    /// own world transform of every chain bone, the same joints the chain was seeded from. `None`
    /// when the slot, rig or layout is gone.
    fn react_clip(
        &self,
        index: usize,
        reaction: &Reaction,
    ) -> Option<[Transform; REACT_CHAIN.len()]> {
        let slot = self.slots.get(index).copied()?;
        let rig = self.rigs.get(reaction.rig)?;
        let layout = self
            .ragdoll
            .layouts
            .get(reaction.rig)
            .and_then(Option::as_ref)?;
        let pose = self.inertial_pose(rig, index, &slot, 0.0).ok()?;
        let (world_pos, world_rot) = forward(&rig.skeleton, &pose);
        let mut out = [Transform::IDENTITY; REACT_CHAIN.len()];
        for (at, bone) in REACT_CHAIN.iter().enumerate() {
            let joint = layout.joint_of[*bone];
            out[at] = Transform {
                pos: *world_pos.get(joint)?,
                rot: *world_rot.get(joint)?,
            };
        }
        Some(out)
    }

    /// Drives every live standing reaction chain toward the clip with the clip's own speed (SW170):
    /// the pose drive from `crates/physfx/src/fx/follow.rs`, weighted by [`react_drive_weight`], so
    /// the chain rejoins the animation at the speed it is moving instead of lagging a tick and
    /// snapping back. The blow tick reads weight 0, which is a no-op, so the shock lands exactly as
    /// it did before this call existed. Presentation only.
    pub(super) fn drive_reactions(&mut self) {
        let mut clips: [Option<[Transform; REACT_CHAIN.len()]>; MAX_LOOK_FIGHTERS] =
            [None; MAX_LOOK_FIGHTERS];
        let mut any = false;
        for (index, clip) in clips.iter_mut().enumerate() {
            let Some(reaction) = self.ragdoll.reactions[index].as_ref() else {
                continue;
            };
            if react_drive_weight(reaction.age) <= 0.0 {
                continue;
            }
            *clip = self.react_clip(index, reaction);
            any |= clip.is_some();
        }
        if !any {
            return;
        }
        let Some(fx) = self.ragdoll.react.as_mut() else {
            return;
        };
        for (index, clip) in clips.iter().enumerate() {
            let Some(clip) = *clip else {
                continue;
            };
            let Ok(slot) = u8::try_from(index) else {
                continue;
            };
            let Some(reaction) = self.ragdoll.reactions[index].as_mut() else {
                continue;
            };
            // The first driven tick has no previous pose yet: drive from the same pose, which asks
            // for no speed and just sets the target.
            let prev = reaction.prev_clip.unwrap_or(clip);
            let weight = react_drive_weight(reaction.age);
            let _ = fx.drive_pose(slot, &prev, &clip, 1.0 / TICK_HZ, weight);
            reaction.prev_clip = Some(clip);
        }
    }

    /// Blends the standing hit reaction of `index` onto `pose` (FEEL9): each chain bone turns
    /// toward the physical chain's bend since the blow, weighted by [`react_weight`], and back to
    /// the clip as the weight falls. The bend is measured in the chain's own pelvis frame and laid
    /// over the clip, so a clip swing carries the struck chain instead of reading as a hit. A
    /// floored slot shows its fall instead. Errors: none of its own; a world without the body
    /// (recycled) leaves the clip whole.
    pub(crate) fn reaction_offsets(
        &self,
        index: usize,
        pose: &mut LocalPose,
        alpha: f32,
    ) -> Result<(), LookError> {
        let Some(reaction) = self.ragdoll.reactions.get(index).and_then(Option::as_ref) else {
            return Ok(());
        };
        if !self.ragdoll.on || self.ragdoll.has(index) {
            return Ok(());
        }
        let weight = react_weight(reaction.age, alpha);
        if weight <= 0.0 {
            return Ok(());
        }
        let Some(fx) = self.ragdoll.react.as_ref() else {
            return Ok(());
        };
        let Some(layout) = self
            .ragdoll
            .layouts
            .get(reaction.rig)
            .and_then(Option::as_ref)
        else {
            return Ok(());
        };
        let Ok(placed) = fx.transforms(alpha) else {
            return Ok(());
        };
        let mut sim: [Option<[f32; 4]>; REACT_CHAIN.len()] = [None; REACT_CHAIN.len()];
        for one in placed {
            if let Key::Bone { slot: live, bone } = one.key {
                if usize::from(live) == index {
                    if let Some(at) = sim.get_mut(usize::from(bone)) {
                        *at = Some(one.at.rot);
                    }
                }
            }
        }
        let (Some(sim_root), Some(&first)) =
            (sim[0], pose.rotations.get(layout.joint_of[REACT_CHAIN[0]]))
        else {
            return Ok(());
        };
        let sim_root = unit(sim_root);
        let clip_root = unit(first);
        // The clip's own world orientations and the shown ones, both parents first. The pelvis is
        // the clip's; every bone above it takes the blow's bend as a delta over the clip.
        let mut clip_world: [Option<[f32; 4]>; REACT_CHAIN.len()] = [None; REACT_CHAIN.len()];
        let mut shown: [Option<[f32; 4]>; REACT_CHAIN.len()] = [None; REACT_CHAIN.len()];
        clip_world[0] = Some(clip_root);
        shown[0] = Some(clip_root);
        for bone in 1..REACT_CHAIN.len() {
            let joint = layout.joint_of[REACT_CHAIN[bone]];
            let Some(clip_local) = pose.rotations.get(joint).copied() else {
                continue;
            };
            let parent_at = react_parent(bone);
            let clip_parent = parent_at.and_then(|at| clip_world[at]).unwrap_or(clip_root);
            let clip_here = unit(quat_mul(clip_parent, clip_local));
            clip_world[bone] = Some(clip_here);
            // The arms are light enough that the blow flings them faster than the view law
            // allows: they ride the clip, the torso and head carry the hit (FEEL9).
            if REACT_CHAIN[bone] >= REACT_ARM_FROM {
                shown[bone] = Some(clip_here);
                continue;
            }
            let Some(sim_rot) = sim[bone] else {
                shown[bone] = Some(clip_here);
                continue;
            };
            let local = unit(quat_mul(conj(sim_root), sim_rot));
            let delta = unit(quat_mul(local, conj(reaction.spawn[bone])));
            let want_world = unit(quat_mul(
                clip_root,
                quat_mul(delta, unit(quat_mul(conj(clip_root), clip_here))),
            ));
            let parent = parent_at.and_then(|at| shown[at]).unwrap_or(clip_root);
            let want = unit(quat_mul(conj(parent), want_world));
            let turn = turn_by(unit(quat_mul(want, conj(clip_local))), weight);
            let out = unit(quat_mul(turn, clip_local));
            pose.rotations[joint] = out;
            shown[bone] = Some(unit(quat_mul(parent, out)));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_physfx::Tier;

    /// Nine chain bones stacked 0.25 m apart: any upright pose the chain can be spawned from.
    const HEIGHTS: [f32; REACT_CHAIN.len()] = [0.0, 0.25, 0.5, 0.75, 1.0, 1.25, 1.5, 1.75, 2.0];

    /// The upright chain pose, one bone per [`REACT_CHAIN`] entry.
    fn chain_pose() -> LocalPose {
        LocalPose {
            positions: HEIGHTS.iter().map(|height| [0.0, *height, 0.0]).collect(),
            rotations: vec![[0.0, 0.0, 0.0, 1.0]; REACT_CHAIN.len()],
        }
    }

    /// The chain's world transforms in chain order, as the drive reads and writes them.
    fn chain_world(fx: &PhysFx) -> Vec<Transform> {
        let mut out = vec![Transform::IDENTITY; REACT_CHAIN.len()];
        for placed in fx.transforms(1.0).expect("transforms") {
            if let Key::Bone { bone, .. } = placed.key {
                out[usize::from(bone)] = placed.at;
            }
        }
        out
    }

    /// The angle between two unit quaternions, radians.
    fn angle_between(a: [f32; 4], b: [f32; 4]) -> f32 {
        let dot: f32 = (0..4).map(|i| a[i] * b[i]).sum();
        2.0 * dot.abs().clamp(-1.0, 1.0).acos()
    }

    #[test]
    fn look_follow_weight_is_zero_while_the_blow_shows_and_rises_as_it_fades() {
        assert_eq!(
            react_drive_weight(0),
            0.0,
            "the blow tick must stay exactly the bytes this file wrote before the drive"
        );
        assert_eq!(
            react_drive_weight(REACT_TICKS),
            0.0,
            "a finished reaction drives nothing"
        );
        // The full-weight window belongs to the blow: the chain must be free to carry it.
        for age in u32::from(REACT_RISE_TICKS)..=u32::from(REACT_HOLD_TICKS) {
            assert_eq!(
                react_drive_weight(age),
                0.0,
                "the chain must be free while the blow shows: age {age}"
            );
        }
        // The fade hands the chain to the drive, and the drive never lets go of it again.
        let mut last = 0.0f32;
        for age in u32::from(REACT_HOLD_TICKS) + 1..REACT_TICKS {
            let weight = react_drive_weight(age);
            assert!(
                weight >= last,
                "the drive must not fall back while the reaction fades: age {age} reads {weight}"
            );
            last = weight;
        }
        let near_end = react_drive_weight(REACT_TICKS - 1);
        println!("drive weight: blow 0.00, full-weight window 0.00, last tick {near_end:.2}");
        assert!(
            near_end > 0.9,
            "the chain rejoins the clip driven at the end of the reaction, not blended: {near_end}"
        );
    }

    #[test]
    fn look_follow_tracks_the_clip_and_weight_zero_keeps_the_shown_bytes() {
        let rig = reaction_rig().expect("reaction chain");
        let pose = chain_pose();
        let mut driven = PhysFx::new(Tier::Comfort);
        driven
            .spawn_ragdoll(0, &rig, &pose, [0.0; 3], [0.0; 3])
            .expect("chain");
        let mut plain = PhysFx::new(Tier::Comfort);
        plain
            .spawn_ragdoll(0, &rig, &pose, [0.0; 3], [0.0; 3])
            .expect("chain");
        let rest = chain_world(&driven);
        // The clip turns the lower arm 25 degrees at its joint between two ticks.
        let mut clip = rest.clone();
        let half = 25.0_f32.to_radians() * 0.5;
        let turn = [half.sin(), 0.0, 0.0, half.cos()];
        clip[4].rot = unit(quat_mul(turn, rest[4].rot));
        let chest = rest[1].pos;
        for fx in [&mut driven, &mut plain] {
            fx.hit_ragdoll(0, chest, REACT_RADIUS_M, [12.0, 0.0, 0.0])
                .expect("blow");
        }
        driven
            .drive_pose(0, &rest, &clip, 1.0 / TICK_HZ, 0.8)
            .expect("drive");
        let before = plain.transforms(1.0).expect("transforms");
        assert_eq!(
            plain.drive_pose(0, &rest, &clip, 1.0 / TICK_HZ, 0.0),
            Ok(0),
            "weight 0 drives nothing"
        );
        assert_eq!(
            plain.transforms(1.0).expect("transforms"),
            before,
            "weight 0 writes no byte"
        );
        driven.step();
        plain.step();
        let with = angle_between(chain_world(&driven)[4].rot, clip[4].rot);
        let without = angle_between(chain_world(&plain)[4].rot, clip[4].rot);
        println!(
            "lower arm to the clip one tick on: no drive {without:.4} rad, with the drive \
             {with:.4} rad"
        );
        assert!(
            with < without,
            "the pose drive must cut the lag: {with} rad against {without} rad"
        );
    }
}
