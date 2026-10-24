//! camera_stack: the director's shelf of fight-camera shots.
//!
//! Owner: game lane. Consumer: the arena window's present pass, which offers the
//! frame's directed shot and shows the rig the stack answers; the bodies the dead
//! zone watches are the ones the shot frames (`fighter_look_frame`). The shot solve
//! and its framing stay where they were: this file only decides *which* shot the eye
//! shows and *when* it moves.
//!
//! Model (donor ramokz/phantom-camera `phantom_camera_3d.gd`): a shelf of shots, each
//! with a priority; the highest live one is the active shot (`set_priority`), equal
//! priorities keep the shot already on screen, then the lower key. A shot stays live
//! [`STALE_TICKS`] after it was last offered, so a director that lets go of a shot
//! drops back to the next priority instead of holding a dead one
//! (`inactive_update_mode NEVER`). A change of the active shot is a cut: eye, look
//! point and lens tween from what is on screen to the new shot over [`BLEND_TICKS`]
//! (`tween_resource`), eased at both ends. Between cuts the eye only moves when the
//! bodies leave [`DEAD_ZONE`] in picture space or read too small or too large: a body
//! drifting inside it leaves the eye exactly where it was (`FollowMode FRAMED`), and
//! when it leaves, the eye eases toward the shot's eye on a [`DAMP_TAU_S`] follow
//! timescale, per axis (`follow_damping` with `0.1` per axis). A tick with nothing to
//! hold inside holds the eye: the dead zone cannot be breached by no body.
//!
//! Units: meters, radians, seconds, NDC (`-1..=1`, y up) and shares of the frame
//! height. Determinism: presentation only, pure over its inputs, the same offers and
//! bodies give the same rig twice, x2 equal in the tests; nothing here reaches a tick
//! hash. Budget: at most [`MAX_SHOTS`] shelf entries and one picture read per frame,
//! no heap, no allocation.

use crate::camera_frame::{read, Params, Rect, Rig, Subject, HEIGHT_FLOOR, MAX_SUBJECTS};

/// Most shots the shelf holds at once.
pub const MAX_SHOTS: usize = 4;
/// Ticks a cut eases over: 0.1 s at the 60 Hz tick.
pub const BLEND_TICKS: u8 = 6;
/// Ticks a shot stays live after it was last offered: 0.5 s.
pub const STALE_TICKS: u32 = 30;
/// Picture box the bodies may drift inside without moving the eye, NDC.
pub const DEAD_ZONE: Rect = Rect {
    x0: -0.30,
    y0: -0.25,
    x1: 0.30,
    y1: 0.25,
};
/// Share of the frame height the biggest body must stay between: under it the body is
/// a speck, over it the body fills the picture. The top is the soft refit's ceiling.
pub const READABLE: (f32, f32) = (HEIGHT_FLOOR, Params::SOFT.ceil);
/// Follow timescale of the eye when the bodies leave the dead zone, seconds.
pub const DAMP_TAU_S: f32 = 0.1;
/// One tick of the arena's fixed 60 Hz rate: the dt a bad frame time falls back to.
const TICK_S: f32 = 1.0 / 60.0;

/// One shot the director offers this tick.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShotOffer {
    /// The shot's own identity: the same key re-offered is the same shot.
    pub key: u8,
    /// Higher wins; equal priorities keep the shot on screen, then the lower key.
    pub priority: i16,
    /// The eye, look point and lens the shot asks for.
    pub rig: Rig,
}

/// One shot on the shelf, and the tick it was last offered.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Live {
    offer: ShotOffer,
    seen: u32,
}

/// A cut in progress: from what the eye showed to the shot it cuts to.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Cut {
    from: Rig,
    to: Rig,
    left: u8,
}

/// The shelf of shots plus what the eye shows: the stack the window drives once a frame.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CameraStack {
    shelf: [Option<Live>; MAX_SHOTS],
    active: Option<u8>,
    pose: Option<Rig>,
    cut: Option<Cut>,
    holds: u32,
    moves: u32,
}

impl CameraStack {
    /// Forget every shot and the eye: a restarted match starts from nothing.
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// One presented frame: shelve `offers` as seen at `tick`, then answer the rig the
    /// eye takes. `subjects` are the bodies the shot frames (what the dead zone keeps
    /// inside it); an empty set holds the eye and never breaches. `dt_s` is the frame
    /// time the follow eases over. `None` until a shot was offered.
    pub fn step(
        &mut self,
        tick: u32,
        dt_s: f32,
        offers: &[ShotOffer],
        subjects: &[Subject],
    ) -> Option<Rig> {
        for offer in offers {
            self.shelve(*offer, tick);
        }
        let (key, rig) = self.winner(tick)?;
        if self.active != Some(key) {
            self.active = Some(key);
            self.cut = self.pose.map(|from| Cut {
                from,
                to: rig,
                left: BLEND_TICKS,
            });
        }
        if let Some(cut) = self.cut {
            self.advance(cut);
            return self.pose;
        }
        let Some(pose) = self.pose else {
            self.pose = Some(rig);
            return self.pose;
        };
        if Self::breached(&pose, subjects) {
            let eased = lerp(&pose, &rig, dt_fraction(dt_s));
            self.pose = Some(eased);
            self.moves = self.moves.saturating_add(1);
        } else {
            self.holds = self.holds.saturating_add(1);
        }
        self.pose
    }

    /// One step of the cut: the eye takes the eased share of the way over, and the
    /// shot's own rig on the last tick so a finished cut lands exactly on it.
    fn advance(&mut self, cut: Cut) {
        let left = cut.left.saturating_sub(1);
        if left == 0 {
            self.pose = Some(cut.to);
            self.cut = None;
            return;
        }
        let t = 1.0 - f32::from(left) / f32::from(BLEND_TICKS);
        self.pose = Some(lerp(&cut.from, &cut.to, ease(t)));
        self.cut = Some(Cut { left, ..cut });
    }

    /// The highest live shot and its rig: priority first, then the shot already on
    /// screen, then the lower key, so the answer never depends on shelf order.
    fn winner(&self, tick: u32) -> Option<(u8, Rig)> {
        self.shelf
            .iter()
            .flatten()
            .filter(|live| !Self::stale(live, tick))
            .max_by_key(|live| {
                (
                    live.offer.priority,
                    u8::from(self.active == Some(live.offer.key)),
                    std::cmp::Reverse(live.offer.key),
                )
            })
            .map(|live| (live.offer.key, live.offer.rig))
    }

    /// True once `live` has gone [`STALE_TICKS`] without being offered.
    fn stale(live: &Live, tick: u32) -> bool {
        tick.saturating_sub(live.seen) > STALE_TICKS
    }

    /// Fold one offer onto the shelf: the same key refreshes its rig and tick; a new
    /// key takes a free slot, else the slot of the lowest ranked shot. A rig that is
    /// not finite apart from its lens is refused.
    fn shelve(&mut self, offer: ShotOffer, tick: u32) {
        if !finite(&offer.rig) {
            return;
        }
        if let Some(live) = self
            .shelf
            .iter_mut()
            .flatten()
            .find(|live| live.offer.key == offer.key)
        {
            *live = Live { offer, seen: tick };
            return;
        }
        let free = self.shelf.iter().position(Option::is_none);
        let slot = free.or_else(|| {
            self.shelf
                .iter()
                .enumerate()
                .filter_map(|(at, live)| live.as_ref().map(|live| (at, *live)))
                .min_by_key(|(_, live)| {
                    (
                        live.offer.priority,
                        std::cmp::Reverse(live.seen),
                        std::cmp::Reverse(live.offer.key),
                    )
                })
                .map(|(at, _)| at)
        });
        if let Some(at) = slot {
            self.shelf[at] = Some(Live { offer, seen: tick });
        }
    }

    /// True when `pose` no longer keeps the bodies where the dead zone wants them: a
    /// body behind the eye or cut by the near plane, the group's middle outside
    /// [`DEAD_ZONE`], or the biggest body outside [`READABLE`].
    fn breached(pose: &Rig, subjects: &[Subject]) -> bool {
        if subjects.is_empty() {
            return false;
        }
        let Some(read) = read(pose, subjects) else {
            return true;
        };
        let count = subjects.len().min(MAX_SUBJECTS);
        let behind = read
            .bodies
            .iter()
            .take(count)
            .any(|body| body.is_none_or(|body| body.near_cut));
        let (cx, cy) = (
            (read.union.x0 + read.union.x1) * 0.5,
            (read.union.y0 + read.union.y1) * 0.5,
        );
        behind || !DEAD_ZONE.holds(cx, cy) || !(READABLE.0..=READABLE.1).contains(&read.size_max)
    }
}

/// True when the shot's eye and look point are finite (`fov_y` must be too, or the
/// picture cannot be read).
fn finite(rig: &Rig) -> bool {
    rig.eye
        .iter()
        .chain(rig.target.iter())
        .all(|v| v.is_finite())
        && rig.fov_y.is_finite()
}

/// Share of the distance the eye closes on the shot in `dt_s` of a [`DAMP_TAU_S`]
/// follow; a time that is not a positive finite number takes one 60 Hz tick.
fn dt_fraction(dt_s: f32) -> f32 {
    let dt = if dt_s.is_finite() && dt_s > 0.0 {
        dt_s
    } else {
        TICK_S
    };
    (1.0 - (-dt / DAMP_TAU_S).exp()).clamp(0.0, 1.0)
}

/// Smoothstep: the cut starts and ends still, so a cut twice in a row does not kick.
fn ease(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn mix(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
    ]
}

/// `from` moved `t` of the way to `to`: the eye and look point move, the lens keeps
/// its aspect and near clip (they are the same camera on both shots).
fn lerp(from: &Rig, to: &Rig, t: f32) -> Rig {
    Rig {
        eye: mix(from.eye, to.eye, t),
        target: mix(from.target, to.target, t),
        fov_y: from.fov_y + (to.fov_y - from.fov_y) * t,
        aspect: from.aspect,
        near: from.near,
    }
}

#[cfg(test)]
#[path = "camera_stack_tests.rs"]
mod tests;
