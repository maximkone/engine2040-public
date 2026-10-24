//! fighter_look_frame: which bodies the fight camera frames and how it holds them.
//!
//! Owner: game lane, a grandchild of `fighter_look` (declared by `fighter_look_cam`,
//! so it reads the look's slots and drawn bounds). Consumers: [`FighterLook::aim_camera`]
//! (the soft refit before the springs, the hard containment after them), the shot
//! builder in `fighter_look` (which bodies the shoulder or side-on shot aims at) and
//! the `arena framing:` law ([`FighterLook::frame_verdict`]). The picture math lives in
//! [`crate::camera_frame`]; this file only picks the bodies and hands it their bounds.
//!
//! Which bodies: the hero and the threat nearest him. With three or more fighters the
//! old group shot backed off until every body was a speck (staged four-fighter bout:
//! 8 percent of the frame), so the camera now follows the hero's fight and the others
//! only when they walk into it. A hero who lies down for good (floored for
//! [`DOWN_HOLD_TICKS`]) hands the camera to the closest standing pair, the killer
//! first, instead of staring at the corpse. A look with no hero keeps the group shot.
//! Bodies come from the drawn bounds of the last pose (a body in the air is where it
//! is drawn, not where the floor point under it is); a look that drew nothing falls
//! back to a standing stand-in at the sim position.
//!
//! Units: meters, ticks at 60 Hz. Determinism: presentation only, pure over the look
//! and the follower's own small state (velocity lead, held threat); the same inputs
//! give the same camera twice and the tick hash never sees it. Budget: a few hundred
//! float ops per frame, one short `Vec` for the seen slots.

use engine_render::v1::Camera;

use super::CamFollow;
use crate::camera_frame::{
    self, Lead, Params, Rig, Subject, Subjects, Verdict, HEIGHT_FLOOR, MAX_SUBJECTS,
};
use crate::fighter_look::{Act, FighterLook, LookFrame, Shot, MAX_LOOK_FIGHTERS, TICK_HZ};

/// Ticks a hero stays floored before the camera takes the fight of the others
/// instead (a knockdown that gets up inside it never moves the camera): 1.5 s.
const DOWN_HOLD_TICKS: u32 = 90;
/// How much nearer (meters) another threat must be before the camera lets go of the
/// one it holds, so two threats at one distance never flip the shot every tick.
const STICK_M: f32 = 1.5;
/// The two slots the action shot is about: the one the camera favors and the one
/// it keeps beside it (`None` for a lone body).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::fighter_look) struct Pair {
    primary: usize,
    other: Option<usize>,
}

/// The containment pass after the springs: [`Params::HARD`] with the readable-size
/// floor, because the springs can lag a throw for most of a second and the biggest
/// body would read under [`HEIGHT_FLOOR`] the whole way. It dollies in only: it
/// never backs off and never opens the lens. A dolly moves eye and look point
/// together, so it cannot change the aim the follower chose.
const HOLD: Params = Params {
    comfort: Params::HARD.comfort,
    floor: HEIGHT_FLOOR,
    aim_small: 0.2,
    ceil: f32::MAX,
    aim_big: f32::MAX,
    eye_follow: Params::HARD.eye_follow,
    pit_m: f32::MAX,
    eye_clear_m: Params::HARD.eye_clear_m,
    eye_max_y_m: Params::HARD.eye_max_y_m,
    fov_max: 0.0,
    dolly_in: true,
};

/// The bodies a frame keeps: current bounds, the slots they belong to, and the
/// threat slot the follower holds for next frame.
struct Framed {
    subjects: Subjects,
    ids: [u8; MAX_SUBJECTS],
    other: Option<u8>,
}

impl Framed {
    fn centers(&self) -> Vec<[f32; 3]> {
        self.subjects
            .as_slice()
            .iter()
            .map(|s| s.center())
            .collect()
    }
}

/// The rig of a render camera.
fn rig_of(camera: &Camera) -> Rig {
    Rig {
        eye: camera.eye.to_array(),
        target: camera.target.to_array(),
        fov_y: camera.fov_y,
        aspect: camera.aspect,
        near: camera.near,
    }
}

/// `camera` moved to `rig`, kept only when the result still validates.
fn moved_to(camera: &Camera, rig: &Rig) -> Camera {
    let mut next = camera.clone();
    next.eye = rig.eye.into();
    next.target = rig.target.into();
    next.fov_y = rig.fov_y;
    if next.validate().is_ok() {
        next
    } else {
        camera.clone()
    }
}

impl FighterLook {
    /// Seen slots under `fighters`, in slot order.
    fn seen_slots(&self, fighters: usize) -> Vec<usize> {
        (0..fighters.min(MAX_LOOK_FIGHTERS))
            .filter(|&index| self.slots.get(index).is_some_and(|slot| slot.seen))
            .collect()
    }

    /// True when `index` has lain floored for [`DOWN_HOLD_TICKS`].
    fn down_for_good(&self, index: usize) -> bool {
        self.slots.get(index).is_some_and(|slot| {
            slot.act == Act::Down && self.tick.saturating_sub(slot.act_start) >= DOWN_HOLD_TICKS
        })
    }

    /// Floor distance between two slots, meters.
    fn gap_m(&self, a: usize, b: usize) -> f32 {
        let (a, b) = (self.slots[a].pos, self.slots[b].pos);
        (a[0] - b[0]).hypot(a[2] - b[2])
    }

    /// The threat of `primary` among `others`: a standing body before a floored one,
    /// then the nearest, keeping `hint` while it is not clearly worse.
    fn threat_of(&self, primary: usize, others: &[usize], hint: Option<u8>) -> Option<usize> {
        let key = |index: usize| {
            (
                self.slots[index].act == Act::Down,
                self.gap_m(primary, index),
            )
        };
        let best = others.iter().copied().min_by(|&a, &b| {
            let (ka, kb) = (key(a), key(b));
            ka.0.cmp(&kb.0).then(ka.1.total_cmp(&kb.1))
        })?;
        let held = hint.map(usize::from).filter(|index| others.contains(index));
        Some(match held {
            Some(h) if key(h).0 == key(best).0 && key(h).1 <= key(best).1 + STICK_M => h,
            _ => best,
        })
    }

    /// The pair the fight shot frames when the hero is gone for good: the closest
    /// two standing bodies, else the one standing body and its nearest neighbor.
    fn spectate_pair(&self, seen: &[usize], hero: usize) -> Pair {
        let standing: Vec<usize> = seen
            .iter()
            .copied()
            .filter(|&index| self.slots[index].act != Act::Down)
            .collect();
        let mut best: Option<(f32, usize, usize)> = None;
        for (n, &a) in standing.iter().enumerate() {
            for &b in &standing[n + 1..] {
                let gap = self.gap_m(a, b);
                if best.is_none_or(|(least, _, _)| gap < least) {
                    best = Some((gap, a, b));
                }
            }
        }
        if let Some((_, a, b)) = best {
            return Pair {
                primary: a,
                other: Some(b),
            };
        }
        let primary = standing.first().copied().unwrap_or(hero);
        let rest: Vec<usize> = seen.iter().copied().filter(|&i| i != primary).collect();
        Pair {
            primary,
            other: self.threat_of(primary, &rest, None),
        }
    }

    /// The pair the fight shot is about, `None` when nothing is seen or the look has
    /// no hero among three or more (the group shot then holds them all). `hint` is the
    /// threat slot the follower held last frame.
    pub(in crate::fighter_look) fn action_pair(
        &self,
        fighters: usize,
        hint: Option<u8>,
    ) -> Option<Pair> {
        let seen = self.seen_slots(fighters);
        let hero = seen.contains(&self.hero).then_some(self.hero);
        match (seen.len(), hero) {
            (0, _) => None,
            (1, _) => Some(Pair {
                primary: seen[0],
                other: None,
            }),
            (2, _) => {
                let primary = hero.unwrap_or(seen[0]);
                let other = seen.iter().copied().find(|&index| index != primary);
                Some(Pair { primary, other })
            }
            (_, Some(hero)) if self.down_for_good(hero) => Some(self.spectate_pair(&seen, hero)),
            (_, Some(hero)) => {
                let others: Vec<usize> = seen.iter().copied().filter(|&i| i != hero).collect();
                Some(Pair {
                    primary: hero,
                    other: self.threat_of(hero, &others, hint),
                })
            }
            (_, None) => None,
        }
    }

    /// The floor positions the shot aims at: the hero first, then the rest in slot
    /// order; in the fight shot with a hero among three or more, only the hero's
    /// pair. Empty when nothing is seen.
    pub(in crate::fighter_look) fn framed_positions(
        &self,
        fighters: usize,
        shot: Shot,
        hint: Option<u8>,
    ) -> Vec<[f32; 3]> {
        let fighters = fighters.min(MAX_LOOK_FIGHTERS);
        let seen = self.seen_slots(fighters);
        let pair = (shot == Shot::Action && seen.len() >= 3)
            .then(|| self.action_pair(fighters, hint))
            .flatten();
        if let Some(pair) = pair {
            return std::iter::once(pair.primary)
                .chain(pair.other)
                .map(|index| self.slots[index].pos)
                .collect();
        }
        std::iter::once(self.hero)
            .filter(|hero| *hero < fighters)
            .chain((0..fighters).filter(|index| *index != self.hero))
            .filter_map(|index| self.slots.get(index))
            .filter(|slot| slot.seen)
            .map(|slot| slot.pos)
            .collect()
    }

    /// The drawn bounds of slot `index` as a framing subject: the last pose's bounds
    /// trimmed to a body's reach, else a standing stand-in at the sim position. `None`
    /// for a slot that is not seen.
    pub(in crate::fighter_look) fn subject_of(&self, index: usize) -> Option<Subject> {
        let slot = self.slots.get(index).filter(|slot| slot.seen)?;
        let drawn = self
            .truth
            .iter()
            .find(|body| usize::from(body.slot) == index && body.has_bounds());
        let Some(body) = drawn else {
            return Some(Subject::standing(slot.pos));
        };
        let whole = Subject {
            min: body.min,
            max: body.max,
        };
        if body.ragdolled || !whole.valid() {
            return Some(whole).filter(|s| s.valid());
        }
        Some(whole.trimmed(body.root))
    }

    /// The bodies the shot `shot` keeps this frame: the hero alone in the hero shot,
    /// else the action pair. `None` for the establishing wide shot (it stays as
    /// directed) or when nothing is seen.
    fn framed(&self, fighters: usize, shot: Shot, hint: Option<u8>) -> Option<Framed> {
        if shot == Shot::Wide {
            return None;
        }
        let pair = self.action_pair(fighters, hint)?;
        let primary = if shot == Shot::Hero && self.slots.get(self.hero).is_some_and(|s| s.seen) {
            self.hero
        } else {
            pair.primary
        };
        let mut framed = Framed {
            subjects: Subjects::new(),
            ids: [0; MAX_SUBJECTS],
            other: None,
        };
        let others = if shot == Shot::Hero { None } else { pair.other };
        for index in std::iter::once(primary).chain(others) {
            if let (Some(subject), Ok(id)) = (self.subject_of(index), u8::try_from(index)) {
                let at = framed.subjects.len();
                framed.subjects.push(subject);
                if framed.subjects.len() > at {
                    framed.ids[at] = id;
                }
            }
        }
        framed.other = others.and_then(|index| u8::try_from(index).ok());
        (!framed.subjects.is_empty()).then_some(framed)
    }

    /// The calm shot refit so the framed bodies sit inside the comfort frame at a
    /// readable size, leading their velocity by [`camera_frame::LEAD_S`]; the follower
    /// remembers the threat it frames for the next frame. A shot with nothing to frame
    /// comes back as it was. Runs before the springs, so the damping smooths it.
    pub(super) fn frame_calm(
        &self,
        follow: &mut CamFollow,
        calm: Camera,
        frame: &LookFrame,
    ) -> Camera {
        let Some(framed) = self.framed(frame.fighters, frame.shot, follow.hint) else {
            return calm;
        };
        follow.hint = framed.other;
        let ids: Vec<u8> = framed.ids[..framed.subjects.len()].to_vec();
        let vel = follow
            .lead
            .step(self.tick, TICK_HZ, &ids, &framed.centers());
        let mut ahead = Subjects::new();
        for (subject, vel) in framed.subjects.as_slice().iter().zip(vel) {
            ahead.push(Lead::ahead(*subject, vel));
        }
        self.refit_soft(&calm, ahead.as_slice())
    }

    /// The shot of [`FighterLook::fight_camera`] refit the same way with no follower state:
    /// no velocity lead and no held threat, so the first frame of the follower is this
    /// camera bit for bit. A shot with nothing to frame comes back as it was.
    pub(in crate::fighter_look) fn frame_still(
        &self,
        camera: Camera,
        shot: Shot,
        fighters: usize,
    ) -> Camera {
        match self.framed(fighters, shot, None) {
            Some(framed) => self.refit_soft(&camera, framed.subjects.as_slice()),
            None => camera,
        }
    }

    /// `camera` refit into the comfort frame around `subjects`, the eye held in the pit.
    fn refit_soft(&self, camera: &Camera, subjects: &[Subject]) -> Camera {
        let params = Params::SOFT.in_pit(self.pit_limit_m());
        let rig = camera_frame::refit(&rig_of(camera), subjects, &params, &[]);
        moved_to(camera, &rig)
    }

    /// The smoothed camera held inside the safe frame: the springs lag a body that
    /// swings or is towed, so this pass moves the view just far enough that every
    /// framed body is inside it now. The camera it is given comes back when it
    /// already holds them.
    pub(super) fn frame_hold(&self, follow: &CamFollow, camera: &mut Camera, frame: &LookFrame) {
        let Some(framed) = self.framed(frame.fighters, frame.shot, follow.hint) else {
            return;
        };
        let params = HOLD.in_pit(self.pit_limit_m());
        let rig = camera_frame::refit(&rig_of(camera), framed.subjects.as_slice(), &params, &[]);
        *camera = moved_to(camera, &rig);
    }

    /// How the frame `camera` shows the framed bodies, for the `arena framing:` law:
    /// `None` for a shot the law does not judge (the wide establishing shot, nothing
    /// seen); a frame whose bodies are all behind the eye reads as lost.
    #[must_use]
    pub fn frame_verdict(
        &self,
        follow: &CamFollow,
        camera: &Camera,
        frame: &LookFrame,
    ) -> Option<Verdict> {
        let framed = self.framed(frame.fighters, frame.shot, follow.hint)?;
        Some(
            camera_frame::read(&rig_of(camera), framed.subjects.as_slice()).map_or(
                Verdict {
                    in_safe: false,
                    size_max: 0.0,
                },
                |read| read.verdict(framed.subjects.len()),
            ),
        )
    }

    /// The bodies the fight camera frames this frame, the same set [`Self::frame_hold`]
    /// keeps inside the safe frame: what [`crate::camera_stack`]'s dead zone watches.
    #[must_use]
    pub fn framed_subjects(&self, follow: &CamFollow, frame: &LookFrame) -> Subjects {
        self.framed(frame.fighters, frame.shot, follow.hint)
            .map_or_else(Subjects::new, |framed| framed.subjects)
    }

    /// The shelf identity and priority of `shot` for [`crate::camera_stack`]: the
    /// close-up outranks the fight shot, which outranks the establishing wide.
    #[must_use]
    pub fn shot_offer(shot: Shot) -> (u8, i16) {
        match shot {
            Shot::Hero => (1, 20),
            Shot::Wide => (3, 0),
            Shot::Action => (2, 10),
        }
    }

    /// The framing rig of a render camera, for [`crate::camera_stack`].
    #[must_use]
    pub fn camera_rig(camera: &Camera) -> Rig {
        rig_of(camera)
    }

    /// `camera` moved to the stacked `rig`, kept as it was when the move no longer
    /// validates: the stacked shot back onto the render camera.
    #[must_use]
    pub fn camera_at(camera: &Camera, rig: &Rig) -> Camera {
        moved_to(camera, rig)
    }

    /// Seen fighters whose body is inside `camera`'s picture: the window's framing check.
    /// A drawn body counts by the middle of its bounds, so a body in the air counts where
    /// it is drawn, not where the floor point under it is; a body the look has not drawn
    /// yet counts by its chest, 1 m over the sim position.
    #[must_use]
    pub fn bodies_in_view(&self, camera: &Camera, fighters: usize) -> usize {
        let middles = self.seen_slots(fighters).into_iter().map(|index| {
            let drawn = self
                .truth
                .iter()
                .any(|body| usize::from(body.slot) == index && body.has_bounds());
            match self.subject_of(index) {
                Some(subject) if drawn => subject.center(),
                _ => {
                    let at = self.slots[index].pos;
                    [at[0], at[1] + 1.0, at[2]]
                }
            }
        });
        crate::fighter_look::count_in_picture(camera, middles)
    }

    /// What the picture of `camera` shows of the bodies in `slots` (test reading).
    #[cfg(test)]
    pub(in crate::fighter_look) fn frame_read(
        &self,
        camera: &Camera,
        slots: &[usize],
    ) -> Option<camera_frame::Read> {
        let subjects: Vec<Subject> = slots
            .iter()
            .filter_map(|&index| self.subject_of(index))
            .collect();
        camera_frame::read(&rig_of(camera), &subjects)
    }
}

#[cfg(test)]
#[path = "fighter_look_frame_tests.rs"]
mod tests;
