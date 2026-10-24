//! fighter_look_throw: the throwing arm of the spear grapple on the fighter look (SG3).
//!
//! Consumer: `spear_window::SpearScene::observe_tick` tells each slot what its
//! throwing arm is doing once per accepted tick ([`FighterLook::observe_throws`]);
//! `slot_sample` lays the throw over the last pose ([`FighterLook::lay_throw`]),
//! after the planted feet, so the arm layer never feeds an inertializer; the
//! window then asks where the right palm is drawn ([`FighterLook::palms`]) so
//! a held spear and a rope end sit in the real hand. Data owner: each slot owns
//! its [`Throw`] clock, each rig its arm ([`throw_arm`]). The timeline and the arm
//! solve are `engine_anim::throw_layer`. A ragdolled or lying slot keeps its
//! own bones. Units: meters, fixed ticks at 60 Hz. Determinism: presentation
//! only; a frame is a pure fold of the tick state and `alpha`. Errors: a solve
//! the rig refuses is a loud [`LookError::Skin`]; a rig with no arm named
//! throws nothing. Heap: none per frame beyond the pose.

use engine_anim::contract::Skeleton;
use engine_anim::ik::Limb;
use engine_anim::throw_layer::{apply_throw, throw_cue, HOLD_END_TICKS, TOTAL_TICKS};
use engine_arena::GaitPhase;

use super::{clip_alpha, FighterLook, LookError, Rig, SlotAnim, MAX_LOOK_FIGHTERS};
use crate::stage_bodies::{clamp_alpha, place_joint};

/// What a slot's throwing arm is doing, as the spear grapple's phase says.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum ArmState {
    /// No spear out: the clip owns the arm.
    #[default]
    Free,
    /// The spear is flying: the arm loads and sweeps.
    Flying,
    /// A tether holds: the arm keeps pointing at the far end.
    Tethered,
}

/// One slot's throw clock: ticks since the spear left the hand.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(super) struct Throw {
    /// The age at the start and at the end of the last tick; `None` when no
    /// throw shows.
    span: Option<[f32; 2]>,
    /// The spear was out at the last fold.
    out: bool,
}

impl Throw {
    pub(super) const NONE: Self = Self {
        span: None,
        out: false,
    };

    /// One accepted tick: a spear leaving the hand starts the clock, a
    /// tether holds it at the end of the hold, and the clock runs out with
    /// the layer.
    fn fold(self, arm: ArmState) -> Self {
        let out = arm != ArmState::Free;
        let span = if out && !self.out {
            Some([0.0, 0.0])
        } else {
            self.span.and_then(|[_, now]| {
                let next = if arm == ArmState::Tethered {
                    (now + 1.0).min(HOLD_END_TICKS)
                } else {
                    now + 1.0
                };
                (next < TOTAL_TICKS).then_some([now, next])
            })
        };
        Self { span, out }
    }

    /// The age `alpha` of the way through the last tick.
    fn age_at(&self, alpha: f32) -> Option<f32> {
        self.span.map(|[from, to]| from + (to - from) * alpha)
    }
}

/// The right arm of a skeleton found by joint name, or `None` when a joint is
/// missing or the three are not a chain. Cold path: once per rig at load.
pub(super) fn throw_arm(skeleton: &Skeleton) -> Option<Limb> {
    let at = |name: &str| skeleton.joints.iter().position(|joint| joint.name == name);
    Limb::new(
        skeleton,
        at("arm_upper_r")?,
        at("arm_lower_r")?,
        at("palm_r")?,
    )
    .ok()
}

impl FighterLook {
    /// Tell every slot what its throwing arm is doing this tick. Call it once
    /// per accepted tick after `observe_tick`. Presentation only; the sim
    /// hash never reads it.
    pub fn observe_throws(&mut self, arms: &[ArmState; MAX_LOOK_FIGHTERS]) {
        let mut moved = false;
        for (slot, arm) in self.slots.iter_mut().zip(arms) {
            let next = slot.throw.fold(*arm);
            moved |= next != slot.throw;
            slot.throw = next;
        }
        // The throw lays over the last pose, behind the clamp's sample of this tick: retake it.
        if moved {
            self.resample_limit_positions();
        }
    }

    /// Lay the throw over `pose`, the very last stage of a slot's pose. A
    /// ragdolled or lying slot, and a rig with no arm, keep the pose as it is.
    pub(super) fn lay_throw(
        &self,
        rig: &Rig,
        index: usize,
        slot: &SlotAnim,
        pose: &mut engine_anim::contract::LocalPose,
        alpha: f32,
    ) -> Result<(), LookError> {
        let lying = slot.balance == GaitPhase::Fallen || slot.act == super::Act::Down;
        let (Some(arm), Some(age)) = (rig.throw_arm, slot.throw.age_at(clip_alpha(slot, alpha)))
        else {
            return Ok(());
        };
        let Some(cue) = throw_cue(age).filter(|_| !lying && !self.ragdolled(index)) else {
            return Ok(());
        };
        apply_throw(&rig.skeleton, pose, arm, cue).map_err(|err| LookError::Skin(err.to_string()))
    }

    /// Where each seen slot's right palm is drawn at sub-tick `alpha`, in
    /// meters on the sand: what a held spear and a rope end tie to. `None`
    /// for an unseen slot or a rig with no arm.
    #[must_use]
    pub fn palms(&self, alpha: f32) -> [Option<[f32; 3]>; MAX_LOOK_FIGHTERS] {
        let alpha = clamp_alpha(alpha);
        std::array::from_fn(|index| {
            let slot = self.slots[index].seen.then_some(&self.slots[index])?;
            let rig = self.rigs.get(self.rig_for_slot(index))?;
            let palm = rig.throw_arm?.end;
            let sample = self.slot_sample(rig, index, slot, alpha).ok()?;
            let m = sample.world.get(palm)?;
            Some(place_joint([m[3][0], m[3][1], m[3][2]], &sample.root))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::FighterView;
    use engine_anim::throw_layer::{RELEASE_TICKS, WIND_TICKS};
    use std::path::PathBuf;

    fn standing_look() -> FighterLook {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../content/fighters");
        let mut look = FighterLook::load(&dir).expect("CT7 bodies load");
        let views = [
            FighterView {
                slot: 0,
                pos_m: [0.0, 0.0, 0.0],
                yaw: 0.0,
            },
            FighterView {
                slot: 1,
                pos_m: [0.0, 0.0, 4.0],
                yaw: 0.0,
            },
        ];
        look.observe_tick(0, &views, &[1000, 1000], &[]);
        look
    }

    fn arms(slot0: ArmState) -> [ArmState; MAX_LOOK_FIGHTERS] {
        let mut out = [ArmState::Free; MAX_LOOK_FIGHTERS];
        out[0] = slot0;
        out
    }

    /// Slot 0's joints in model space, as the window draws them at the end of the last tick.
    fn joints(look: &FighterLook) -> Vec<[f32; 3]> {
        look.pose_frame_joints(2, 1.0).expect("CPU pose")[0]
            .joints
            .clone()
    }

    /// The right palm relative to the right shoulder, in model space.
    fn reach(look: &FighterLook) -> [f32; 3] {
        let arm = look.rigs[0].throw_arm.expect("the body has a right arm");
        let at = joints(look);
        let (shoulder, palm) = (at[arm.root], at[arm.end]);
        [
            palm[0] - shoulder[0],
            palm[1] - shoulder[1],
            palm[2] - shoulder[2],
        ]
    }

    /// Fold `ticks` ticks of `arm` into the throw clock.
    fn run(look: &mut FighterLook, arm: ArmState, ticks: u32) {
        for _ in 0..ticks {
            look.observe_throws(&arms(arm));
        }
    }

    #[test]
    fn a_thrown_spear_loads_the_arm_back_then_points_it_and_gives_it_back() {
        let mut look = standing_look();
        let clip = joints(&look);
        // The tick the spear leaves the hand shows the clip's arm.
        run(&mut look, ArmState::Flying, 1);
        assert_eq!(joints(&look), clip, "weight zero at the release tick");
        // Loaded: the hand sits behind and above the shoulder.
        run(&mut look, ArmState::Flying, WIND_TICKS as u32);
        let loaded = reach(&look);
        assert!(loaded[2] < -0.05 && loaded[1] > 0.0, "loaded {loaded:?}");
        // Pointing: the hand is well ahead of the shoulder.
        run(&mut look, ArmState::Free, RELEASE_TICKS as u32);
        let pointing = reach(&look);
        assert!(pointing[2] > 0.3, "pointing {pointing:?}");
        // Only the arm and what hangs off it moved.
        let arm = look.rigs[0].throw_arm.expect("arm");
        let skeleton = &look.rigs[0].skeleton;
        let under_arm = |mut joint: usize| loop {
            if joint == arm.root {
                break true;
            }
            match skeleton.joints[joint].parent {
                Some(parent) => joint = parent,
                None => break false,
            }
        };
        let after = joints(&look);
        for (joint, (a, b)) in clip.iter().zip(&after).enumerate() {
            assert!(
                under_arm(joint) || a == b,
                "joint {joint} moved off the arm"
            );
        }
        assert!(clip[arm.end] != after[arm.end]);
        // The clip gets its arm back once the layer has run out.
        run(&mut look, ArmState::Free, 40);
        assert_eq!(joints(&look), clip, "the throw layer is gone");
    }

    #[test]
    fn a_held_tether_keeps_the_arm_pointing_until_it_ends() {
        let mut look = standing_look();
        let clip = joints(&look);
        run(&mut look, ArmState::Tethered, 1 + HOLD_END_TICKS as u32);
        let pointing = joints(&look);
        assert!(pointing != clip);
        run(&mut look, ArmState::Tethered, 90);
        assert_eq!(joints(&look), pointing, "the tether holds the pose");
        run(&mut look, ArmState::Free, 40);
        assert_eq!(
            joints(&look),
            clip,
            "the tether let go and the arm came home"
        );
    }

    #[test]
    fn a_lying_slot_keeps_its_own_bones() {
        let mut look = standing_look();
        look.slots[0].act = super::super::Act::Down;
        let without = joints(&look);
        run(&mut look, ArmState::Flying, 8);
        assert_eq!(joints(&look), without, "a body on the sand throws nothing");
    }

    #[test]
    fn the_palm_follows_the_drawn_hand() {
        let mut look = standing_look();
        let palm = look.palms(1.0)[0].expect("a seen slot has a palm");
        let arm = look.rigs[0].throw_arm.expect("arm");
        let model = joints(&look)[arm.end];
        // The slot stands at the origin facing +z: the model's palm is the world's.
        for (a, b) in palm.iter().zip(model) {
            assert!((a - b).abs() < 1.0e-4, "palm {palm:?} vs joint {model:?}");
        }
        assert!(look.palms(1.0)[5].is_none(), "an unseen slot has no palm");
        run(&mut look, ArmState::Flying, 8);
        let thrown = look.palms(1.0)[0].expect("palm");
        let moved = (0..3).map(|k| (thrown[k] - palm[k]).powi(2)).sum::<f32>();
        assert!(moved.sqrt() > 0.2, "the palm went with the arm: {moved}");
    }

    #[test]
    fn the_clock_starts_on_a_release_and_runs_out() {
        let mut throw = Throw::NONE;
        assert_eq!(throw.age_at(0.5), None);
        throw = throw.fold(ArmState::Flying);
        assert_eq!(throw.age_at(1.0), Some(0.0));
        throw = throw.fold(ArmState::Flying);
        assert_eq!(throw.age_at(0.5), Some(0.5));
        for _ in 0..200 {
            throw = throw.fold(ArmState::Tethered);
        }
        assert_eq!(throw.age_at(1.0), Some(HOLD_END_TICKS));
        for _ in 0..40 {
            throw = throw.fold(ArmState::Free);
        }
        assert_eq!(throw, Throw::NONE);
        // A second throw starts the clock again from nothing.
        throw = throw.fold(ArmState::Flying);
        assert_eq!(throw.age_at(1.0), Some(0.0));
    }
}
