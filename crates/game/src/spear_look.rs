//! spear_look: the picture of the spear grapple, a spear in flight, stuck, or dropped, and the
//! rope between the two fighters it joins (`docs/plan/spear-grapple.md`).
//!
//! The authority (`engine-arena`, integers, hashed) decides when a spear leaves a hand, what it
//! hits, who is tethered to whom and where both bodies go. This module reads only what the match
//! exposes (`LiveMatch::spear_events`, `LiveMatch::spear_view`) plus the fighters' drawn places
//! and, through [`Ties`], where each body is drawn (a ragdolled pelvis, a standing thrower's right
//! palm, which the rope's thrower end ties to), and keeps floats: the spear's pose and spin, a trail behind it, a verlet rope from
//! `engine-physfx` that sags while slack and straightens when taut, and for each tethered body
//! a [`Carry`] that tells the ragdoll layer where the authority is taking it and how high the
//! arc lifts it. A yank (row SG10) tows only the victim, on a rope strained shorter than the
//! gap, and leaves the planted thrower standing. It never feeds the sim: the golden
//! `d57b5c8ae3096614` cannot move.
//!
//! Callers: `spear_window::SpearScene` (one `observe` per accepted tick, the draws per frame) and
//! `FighterLook::observe_carries` (the ragdoll layer). Units: meters, seconds, Y up; authority
//! millimeters are converted at the door. Determinism: the same events and views give the same
//! picture twice on one machine. Budget: at most one rope per slot, the tier's node count each;
//! no allocation per tick beyond the rope's own node buffer.

use crate::fighter_look::MAX_LOOK_FIGHTERS;
use crate::FighterView;
use engine_arena::spear::{Phase, SpearEvent, SpearSlot};
use engine_core::{quat_conj, rotate_vec};
use engine_physfx::{PhysFx, RopeBlock, TetherState, Tier};

mod noose;

/// Tier a look starts on: the comfortable desktop tier.
pub const DEFAULT_TIER: Tier = Tier::Comfort;
/// Spear length from tail to tip, meters.
pub const SPEAR_LEN_M: f32 = 1.9;
/// Where a hand holds the spear above the feet, meters.
const HAND_Y_M: f32 = 1.3;
/// How far in front of the feet the throwing hand is, meters.
const HAND_FORWARD_M: f32 = 0.3;
/// Chest height of a tethered fighter above the feet, meters.
const CHEST_Y_M: f32 = 1.15;
/// A ragdoll's pelvis sits this far under the rope's anchor on the body, meters.
const PELVIS_TO_CHEST_M: f32 = 0.4;
/// Rope slack paid out in flight: the rope is this much longer than the gap.
const FLIGHT_SLACK: f32 = 1.08;
/// Extra rope in flight, meters, so a short gap still sags.
const FLIGHT_EXTRA_M: f32 = 0.3;
/// Rope slack of a latched pair: the pair rides together and the rope hangs between.
const LATCHED_SLACK: f32 = 1.12;
/// Rope length of a yank (row SG10) as a share of the gap: shorter than the gap, so the
/// rope is strained straight from the catch to the stop and never sags while it hauls.
const YANK_TAUT: f32 = 0.96;
/// The spear's tail must be this much nearer the thrower's hand than the body or wall it is
/// stuck in before a rope ties to it, meters: close in, the rope ties to the body.
const TAIL_MARGIN_M: f32 = 0.25;
/// Share of the way the rope's knot moves toward where it belongs, per tick.
const TAIL_EASE: f32 = 0.25;
/// Tip positions kept behind a flying spear.
pub const TRAIL_LEN: usize = 10;
/// Spin of a flying spear about its axis, radians per tick.
const SPIN_PER_TICK: f32 = 0.9;
/// Ticks a dropped spear lies in the sand before it is gone.
const DROPPED_TICKS: u16 = 80;
/// Gravity, m/s^2.
const GRAVITY_M_S2: f32 = 9.81;
/// Ticks per second.
const TICK_HZ: f32 = 60.0;
/// Highest the arc lifts a tethered body, meters.
const ARC_PEAK_M: f32 = 0.8;
/// Ticks the arc takes to rise.
const ARC_RISE_TICKS: f32 = 14.0;
/// Speed (m/s) at which the arc is full: a slow pull lifts less.
const ARC_SPEED_REF_M_S: f32 = 8.0;
/// Millimeters per meter.
const MM: f32 = 1000.0;
/// Share of its sideways slide a rope node keeps when a body pushes it out: half, so a rope slips
/// over a shoulder instead of skating across it.
const ROPE_SLIDE_KEEP: f32 = 0.5;

/// Where the drawn bodies are this tick: what the rope ends tie to.
#[derive(Clone, Copy, Debug)]
pub struct Ties<'a> {
    /// A ragdolled body's pelvis, meters; `None` for a body on its feet.
    pub pelvis: &'a [Option<[f32; 3]>; MAX_LOOK_FIGHTERS],
    /// A body's right palm as drawn, meters; `None` when the look cannot say.
    pub palms: &'a [Option<[f32; 3]>; MAX_LOOK_FIGHTERS],
    /// Every bone of every ragdolled body as drawn; empty while nobody is down. A spear
    /// stuck in a body that falls rides the bone nearest its wound.
    pub bones: &'a [BoneFrame],
}

/// One bone of a ragdolled body as drawn: the frame a spear stuck in it rides.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BoneFrame {
    /// Fighter slot the ragdoll belongs to.
    pub slot: u8,
    /// Physical bone index (the ragdoll table's).
    pub bone: u16,
    /// Bone centre, meters.
    pub pos: [f32; 3],
    /// Bone turn, unit quaternion `[x, y, z, w]`.
    pub rot: [f32; 4],
}

/// A place and a turn in the world: the frame a stuck spear is held in.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Frame {
    pos: [f32; 3],
    rot: [f32; 4],
}

impl Frame {
    /// A standing body: the feet, turned about +Y by `yaw` (forward +Z goes to `(sin, 0, cos)`).
    fn stand(pos: [f32; 3], yaw: f32) -> Self {
        let (s, c) = (yaw * 0.5).sin_cos();
        Self {
            pos,
            rot: [0.0, s, 0.0, c],
        }
    }

    /// The frame of one drawn bone.
    fn bone(b: &BoneFrame) -> Self {
        Self {
            pos: b.pos,
            rot: b.rot,
        }
    }

    /// World tip and direction of a spear held at `local` and `dir` in this frame.
    fn to_world(self, local: [f32; 3], dir: [f32; 3]) -> ([f32; 3], [f32; 3]) {
        (
            add(self.pos, rotate_vec(&local, &self.rot)),
            rotate_vec(&dir, &self.rot),
        )
    }

    /// The `local` and `dir` that hold a spear at world `tip` and `dir` in this frame.
    fn to_local(self, tip: [f32; 3], dir: [f32; 3]) -> ([f32; 3], [f32; 3]) {
        let back = quat_conj(&self.rot);
        (
            rotate_vec(&sub(tip, self.pos), &back),
            rotate_vec(&dir, &back),
        )
    }
}

impl Ties<'static> {
    /// No body is placed: every rope end falls back to the authority place.
    pub const NONE: Self = Self {
        pelvis: &[None; MAX_LOOK_FIGHTERS],
        palms: &[None; MAX_LOOK_FIGHTERS],
        bones: &[],
    };
}

/// Which end of the tether a body is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    /// The fighter who threw.
    Thrower,
    /// The fighter the spear stuck in.
    Victim,
}

/// A tethered body: where the authority is taking it, for the ragdoll layer to follow.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Carry {
    /// Thrower or victim.
    pub role: Role,
    /// Ticks since the spear stuck.
    pub age: u16,
    /// Thrower speed after the last tick, meters per second.
    pub speed_ms: f32,
}

impl Carry {
    /// How high the arc lifts this body above its authority place, meters: it rises over
    /// [`ARC_RISE_TICKS`] and scales with speed, so a slow reel hops less than a fast one.
    #[must_use]
    pub fn lift_m(&self) -> f32 {
        let rise = (f32::from(self.age) / ARC_RISE_TICKS).min(1.0);
        let eased = rise * rise * (3.0 - 2.0 * rise);
        ARC_PEAK_M * eased * (self.speed_ms / ARC_SPEED_REF_M_S).clamp(0.0, 1.0)
    }
}

/// What part of a fighter a spear is stuck in.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Anchor {
    /// The standing body: its feet and facing.
    Body,
    /// One bone of the fallen body.
    Bone(u16),
}

/// What a spear is stuck in.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Hold {
    /// A fighter: the tip sits at `local` in the frame of `anchor`, the shaft along `dir`
    /// there. The spear keeps its wound and its angle when the body turns or falls.
    Fighter {
        slot: u8,
        anchor: Anchor,
        local: [f32; 3],
        dir: [f32; 3],
    },
    /// Geometry: the tip stays where it hit.
    World,
}

/// Where a slot's spear is in its life.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Fate {
    Flying,
    Stuck(Hold),
    /// Fell out: ticks since, then gone.
    Dropped(u16),
}

/// One spear in the picture.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Spear {
    prev: [f32; 3],
    tip: [f32; 3],
    dir: [f32; 3],
    fate: Fate,
    age: u16,
    trail: [[f32; 3]; TRAIL_LEN],
    trail_len: usize,
}

/// A spear to draw this frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpearDraw {
    /// Thrower slot.
    pub slot: u8,
    /// Tip, meters.
    pub tip: [f32; 3],
    /// Unit direction from tail to tip.
    pub dir: [f32; 3],
    /// Roll about the axis, radians.
    pub roll: f32,
    /// True while the spear is in flight (it trails).
    pub flying: bool,
    /// The fighter it is stuck in, if any (the frame audit checks the tip sits in that body).
    pub stuck_in: Option<u8>,
    /// The fighter whose palm holds it, if any.
    pub held_by: Option<u8>,
}

/// What the picture counted over a run, for the exit receipt and the tests.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SpearStats {
    /// Spears thrown.
    pub thrown: u32,
    /// Spears that stuck (fighter or world).
    pub hits: u32,
    /// Tethers that formed with a fighter or anchor.
    pub tethers: u32,
    /// Tether ends.
    pub ends: u32,
    /// Most spears alive at once.
    pub spears_max: u32,
    /// Most ropes alive at once.
    pub ropes_max: u32,
}

/// One slot's lane.
#[derive(Clone, Copy, Debug)]
struct Lane {
    spear: Option<Spear>,
    /// A rope is tied for this slot.
    rope: bool,
    /// How far the rope's far end has moved from the body or wall onto the spear's tail, `0..=1`:
    /// a rope is tied to the butt of the spear it hauls on, not to the point inside the wound.
    tail_w: f32,
}

const EMPTY_LANE: Lane = Lane {
    spear: None,
    rope: false,
    tail_w: 1.0,
};

/// The spear grapple's picture. See the module docs.
pub struct SpearLook {
    fx: PhysFx,
    lanes: [Lane; MAX_LOOK_FIGHTERS],
    /// Carries by body, thrower and victim both.
    bodies: [Option<Carry>; MAX_LOOK_FIGHTERS],
    stats: SpearStats,
}

fn mm3(v: [i32; 3]) -> [f32; 3] {
    v.map(|c| c as f32 / MM)
}

fn add(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn scale(a: [f32; 3], k: f32) -> [f32; 3] {
    a.map(|c| c * k)
}

fn norm(v: [f32; 3]) -> f32 {
    (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt()
}

fn unit(v: [f32; 3]) -> [f32; 3] {
    let n = norm(v);
    if n.is_finite() && n > 1.0e-6 {
        scale(v, 1.0 / n)
    } else {
        [0.0, 0.0, 1.0]
    }
}

fn lerp(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    add(a, scale(sub(b, a), t))
}

/// A stuck spear's hold, as it was kept.
#[derive(Clone, Copy)]
struct Held {
    slot: u8,
    anchor: Anchor,
    local: [f32; 3],
    dir: [f32; 3],
}

/// Where a stuck spear is this tick and the hold to keep.
struct Ridden {
    anchor: Anchor,
    local: [f32; 3],
    dir: [f32; 3],
    tip: [f32; 3],
    world_dir: [f32; 3],
}

/// Moves a spear stuck in a fighter with the body part it hit. While the body stands the
/// spear rides the feet and facing. When the body falls (its bones are drawn) the spear
/// takes the bone nearest its wound and rides that bone, so a body that tumbles carries
/// the spear with the angle it was hit at. `last` is the spear's world tip and direction a
/// tick ago: a bone that vanishes hands the spear back to the body frame from there, with no
/// jump. Pure; no allocation.
fn ride(held: Held, last: ([f32; 3], [f32; 3]), stand: Frame, bones: &[BoneFrame]) -> Ridden {
    let mine = bones.iter().filter(|b| b.slot == held.slot);
    let (mut anchor, mut local, mut dir) = (held.anchor, held.local, held.dir);
    if let Anchor::Bone(bone) = anchor {
        if let Some(b) = mine.clone().find(|b| b.bone == bone) {
            let (tip, world_dir) = Frame::bone(b).to_world(local, dir);
            return Ridden {
                anchor,
                local,
                dir,
                tip,
                world_dir,
            };
        }
        (local, dir) = stand.to_local(last.0, last.1);
        anchor = Anchor::Body;
    }
    let (tip, world_dir) = stand.to_world(local, dir);
    let nearest = mine.fold(None::<(f32, &BoneFrame)>, |best, b| {
        let gap = norm(sub(tip, b.pos));
        match best {
            Some((g, _)) if g <= gap => best,
            _ => Some((gap, b)),
        }
    });
    if let Some((_, b)) = nearest {
        (local, dir) = Frame::bone(b).to_local(tip, world_dir);
        anchor = Anchor::Bone(b.bone);
    }
    Ridden {
        anchor,
        local,
        dir,
        tip,
        world_dir,
    }
}

impl SpearLook {
    /// An empty picture on `tier`.
    #[must_use]
    pub fn new(tier: Tier) -> Self {
        Self {
            fx: PhysFx::new(tier),
            lanes: [EMPTY_LANE; MAX_LOOK_FIGHTERS],
            bodies: [None; MAX_LOOK_FIGHTERS],
            stats: SpearStats::default(),
        }
    }

    /// The rope world's tier caps bound the ropes and their nodes.
    #[must_use]
    pub fn stats(&self) -> SpearStats {
        self.stats
    }

    /// Spears alive now (flying, stuck or lying).
    #[must_use]
    pub fn spears(&self) -> usize {
        self.lanes.iter().filter(|l| l.spear.is_some()).count()
    }

    /// Ropes alive now.
    #[must_use]
    pub fn ropes(&self) -> usize {
        self.fx.tether_count()
    }

    /// The tethered bodies this tick: `Some` for the thrower and the victim of each tether.
    #[must_use]
    pub fn carries(&self) -> &[Option<Carry>; MAX_LOOK_FIGHTERS] {
        &self.bodies
    }

    /// One receipt line: `arena spear: thrown=.. hits=.. tethers=.. ends=.. ropes_max=..`.
    #[must_use]
    pub fn receipt_line(&self) -> String {
        let s = self.stats;
        format!(
            "arena spear: thrown={} hits={} tethers={} ends={} spears_max={} ropes_max={}",
            s.thrown, s.hits, s.tethers, s.ends, s.spears_max, s.ropes_max
        )
    }

    /// Takes one accepted tick: the match's spear events and per-slot view, the fighters' drawn
    /// places and where the bodies are drawn (`ties`): a ragdolled body's pelvis, else a
    /// standing thrower's palm, is where the rope ties. Steps the rope world once.
    pub fn observe(
        &mut self,
        events: &[SpearEvent],
        view: &[SpearSlot],
        fighters: &[FighterView],
        ties: Ties<'_>,
    ) {
        let feet = |slot: u8| {
            fighters
                .iter()
                .find(|f| f.slot == slot)
                .map(|f| (f.pos_m, f.yaw))
        };
        for event in events {
            self.fold_event(*event, &feet);
        }
        self.bodies = [None; MAX_LOOK_FIGHTERS];
        for (index, slot) in view.iter().enumerate().take(MAX_LOOK_FIGHTERS) {
            let Ok(thrower) = u8::try_from(index) else {
                continue;
            };
            self.fold_slot(thrower, slot, &feet, ties);
        }
        self.fx.step();
        let spears = u32::try_from(self.spears()).unwrap_or(u32::MAX);
        let ropes = u32::try_from(self.ropes()).unwrap_or(u32::MAX);
        self.stats.spears_max = self.stats.spears_max.max(spears);
        self.stats.ropes_max = self.stats.ropes_max.max(ropes);
    }

    fn fold_event(&mut self, event: SpearEvent, feet: &dyn Fn(u8) -> Option<([f32; 3], f32)>) {
        match event {
            SpearEvent::Thrown {
                thrower,
                at_mm,
                vel_mm,
            } => {
                self.stats.thrown += 1;
                let tip = mm3(at_mm);
                self.set_spear(
                    thrower,
                    Spear {
                        prev: tip,
                        tip,
                        dir: unit(mm3(vel_mm)),
                        fate: Fate::Flying,
                        age: 0,
                        trail: [tip; TRAIL_LEN],
                        trail_len: 0,
                    },
                );
            }
            SpearEvent::Hit {
                thrower,
                victim,
                at_mm,
            } => {
                self.stats.hits += 1;
                let at = mm3(at_mm);
                let flight = self.spear_mut(thrower).map_or([0.0, 0.0, 1.0], |s| s.dir);
                let hold = match victim.and_then(|v| feet(v).map(|(pos, yaw)| (v, pos, yaw))) {
                    Some((slot, pos, yaw)) => {
                        let (local, dir) = Frame::stand(pos, yaw).to_local(at, flight);
                        Hold::Fighter {
                            slot,
                            anchor: Anchor::Body,
                            local,
                            dir,
                        }
                    }
                    None => Hold::World,
                };
                if let Some(spear) = self.spear_mut(thrower) {
                    spear.tip = at;
                    spear.prev = at;
                    spear.fate = Fate::Stuck(hold);
                }
                self.stats.tethers += 1;
            }
            SpearEvent::Missed { thrower, .. } | SpearEvent::Deflected { thrower, .. } => {
                self.drop_spear(thrower);
            }
            SpearEvent::TetherEnd { thrower, .. } => {
                self.stats.ends += 1;
                // The spear comes out of what held it and falls.
                self.drop_spear(thrower);
            }
            SpearEvent::TetherPull { .. } => {}
        }
    }

    fn fold_slot(
        &mut self,
        thrower: u8,
        slot: &SpearSlot,
        feet: &dyn Fn(u8) -> Option<([f32; 3], f32)>,
        ties: Ties<'_>,
    ) {
        let index = usize::from(thrower);
        let Some((pos, yaw)) = feet(thrower) else {
            return;
        };
        // The downed bodies the rope may cross, taken once for this tick: every live ragdoll
        // bone's own capsule, so the rope a spear hauls goes round a body instead of through it.
        let blocks = self.fx.tether_blocks();
        let hand = self.anchor(thrower, pos, yaw, ties, true);
        match slot.phase {
            Phase::Idle => {
                if self.lanes[index].rope {
                    self.fx.release_tether(u32::from(thrower));
                    self.lanes[index].rope = false;
                }
                self.age_dropped(index);
            }
            Phase::Flight(flight) if flight.noose => self.fold_loop(thrower, &flight, hand),
            Phase::Tether(tether) if tether.noose => {
                self.fold_snare(thrower, &tether, hand, feet, ties);
            }
            Phase::Flight(flight) => {
                let tip = mm3(flight.pos_mm);
                let dir = unit(mm3(flight.vel_mm));
                if let Some(spear) = self.lanes[index].spear.as_mut() {
                    spear.prev = spear.tip;
                    spear.tip = tip;
                    spear.dir = dir;
                    spear.age = flight.age;
                    spear.push_trail(tip);
                }
                let tail = sub(tip, scale(dir, SPEAR_LEN_M));
                let gap = norm(sub(tail, hand));
                self.lanes[index].tail_w = 1.0;
                self.tie(
                    thrower,
                    hand,
                    tail,
                    gap * FLIGHT_SLACK + FLIGHT_EXTRA_M,
                    &blocks,
                );
            }
            Phase::Tether(tether) => {
                let speed_ms = f32::from(tether.speed) * TICK_HZ / MM;
                let carry = |role| Carry {
                    role,
                    age: tether.age,
                    speed_ms,
                };
                // A yank (row SG9) plants the thrower: only the victim is towed and lifted,
                // the thrower stays on its feet with the arm hauling.
                if !tether.yank {
                    self.bodies[index] = Some(carry(Role::Thrower));
                }
                let far = match tether.victim {
                    Some(victim) => {
                        if let Some(slot) = self.bodies.get_mut(usize::from(victim)) {
                            *slot = Some(carry(Role::Victim));
                        }
                        feet(victim)
                            .map(|(vpos, vyaw)| self.anchor(victim, vpos, vyaw, ties, false))
                    }
                    None => self.lanes[index].spear.map(|s| s.tip),
                };
                if let Some(spear) = self.lanes[index].spear.as_mut() {
                    spear.ride_victim(feet, ties.bones);
                }
                if let Some(anchor) = far {
                    let far = self.rope_end(index, hand, anchor);
                    let gap = norm(sub(far, hand));
                    let slack = if tether.yank {
                        YANK_TAUT
                    } else if tether.latched {
                        LATCHED_SLACK
                    } else {
                        1.0
                    };
                    self.tie(thrower, hand, far, gap * slack, &blocks);
                }
            }
        }
    }

    /// Where the far end of the rope of lane `index` ties while its spear is stuck: the spear's
    /// tail while the shaft stands out toward the thrower, else `anchor`, the body or wall.
    fn rope_end(&mut self, index: usize, hand: [f32; 3], anchor: [f32; 3]) -> [f32; 3] {
        let lane = &mut self.lanes[index];
        let Some(spear) = lane.spear.filter(|s| matches!(s.fate, Fate::Stuck(_))) else {
            return anchor;
        };
        let (end, weight) = rope_far(hand, anchor, (spear.tip, spear.dir), lane.tail_w);
        lane.tail_w = weight;
        end
    }

    /// Where a rope ties to a body: its ragdoll pelvis lifted to the chest, else the
    /// authority place at chest height; the thrower's end is its drawn palm, else the hand in
    /// front of the authority place.
    fn anchor(&self, slot: u8, pos: [f32; 3], yaw: f32, ties: Ties<'_>, hand: bool) -> [f32; 3] {
        let at = usize::from(slot);
        if let Some(Some(root)) = ties.pelvis.get(at) {
            return add(*root, [0.0, PELVIS_TO_CHEST_M, 0.0]);
        }
        if hand {
            if let Some(Some(palm)) = ties.palms.get(at) {
                return *palm;
            }
            let (s, c) = yaw.sin_cos();
            add(pos, [s * HAND_FORWARD_M, HAND_Y_M, c * HAND_FORWARD_M])
        } else {
            add(pos, [0.0, CHEST_Y_M, 0.0])
        }
    }

    /// Ties the rope of `thrower` from `a` to `b`, or moves and reels the one already tied. The
    /// bodies in `blocks` are the capsules the rope may not be drawn through.
    fn tie(&mut self, thrower: u8, a: [f32; 3], b: [f32; 3], rest_m: f32, blocks: &[RopeBlock]) {
        let id = u32::from(thrower);
        let rest = rest_m.max(0.3);
        let index = usize::from(thrower);
        if self.lanes[index].rope && self.fx.tether_info(id).is_some() {
            let moved = self
                .fx
                .move_tether_beside(id, a, b, blocks, ROPE_SLIDE_KEEP);
            let reeled = self.fx.retune_tether(id, rest);
            if moved.is_ok() && reeled.is_ok() {
                return;
            }
        }
        self.lanes[index].rope = self.fx.attach_tether(id, a, b, rest).is_ok();
        if self.lanes[index].rope {
            // The first tick already knows the bodies, not only the second.
            let _ = self
                .fx
                .move_tether_beside(id, a, b, blocks, ROPE_SLIDE_KEEP);
        }
    }

    fn set_spear(&mut self, slot: u8, spear: Spear) {
        if let Some(lane) = self.lanes.get_mut(usize::from(slot)) {
            lane.spear = Some(spear);
        }
    }

    fn spear_mut(&mut self, slot: u8) -> Option<&mut Spear> {
        self.lanes.get_mut(usize::from(slot))?.spear.as_mut()
    }

    fn drop_spear(&mut self, slot: u8) {
        if let Some(spear) = self.spear_mut(slot) {
            if !matches!(spear.fate, Fate::Dropped(_)) {
                spear.fate = Fate::Dropped(0);
            }
        }
    }

    fn age_dropped(&mut self, index: usize) {
        let Some(spear) = self.lanes[index].spear.as_mut() else {
            return;
        };
        if let Fate::Dropped(age) = spear.fate {
            if age >= DROPPED_TICKS {
                self.lanes[index].spear = None;
            } else {
                spear.fate = Fate::Dropped(age + 1);
            }
        }
    }

    /// The spears to draw, `alpha` in `[0, 1]` blending the last two ticks.
    #[must_use]
    pub fn draws(&self, alpha: f32) -> Vec<SpearDraw> {
        let t = alpha.clamp(0.0, 1.0);
        let mut out = Vec::with_capacity(MAX_LOOK_FIGHTERS);
        for (index, lane) in self.lanes.iter().enumerate() {
            let (Some(spear), Ok(slot)) = (lane.spear, u8::try_from(index)) else {
                continue;
            };
            let (tip, dir) = match spear.fate {
                Fate::Dropped(age) => spear.dropped_pose(age, t),
                _ => (lerp(spear.prev, spear.tip, t), spear.dir),
            };
            out.push(SpearDraw {
                slot,
                tip,
                dir,
                roll: f32::from(spear.age) * SPIN_PER_TICK,
                flying: spear.fate == Fate::Flying,
                stuck_in: match spear.fate {
                    Fate::Stuck(Hold::Fighter { slot, .. }) => Some(slot),
                    _ => None,
                },
                held_by: None,
            });
        }
        out
    }

    /// The trail behind a flying spear of `slot`, newest first, oldest last; empty otherwise.
    #[must_use]
    pub fn trail(&self, slot: u8) -> &[[f32; 3]] {
        match self
            .lanes
            .get(usize::from(slot))
            .and_then(|l| l.spear.as_ref())
        {
            Some(spear) if spear.fate == Fate::Flying => &spear.trail[..spear.trail_len],
            _ => &[],
        }
    }

    /// The rope nodes of `slot`, end to end, `alpha` blending the last two ticks; `None` when
    /// the slot has no rope.
    #[must_use]
    pub fn rope(&self, slot: u8, alpha: f32) -> Option<Vec<[f32; 3]>> {
        self.fx.tether_nodes(u32::from(slot), alpha).ok().flatten()
    }

    /// How the rope of `slot` is doing: slack, taut or loose, and its pull in newtons.
    #[must_use]
    pub fn rope_state(&self, slot: u8) -> Option<(TetherState, f32)> {
        let info = self.fx.tether_info(u32::from(slot))?;
        Some((info.state, norm(info.pull)))
    }
}

impl Spear {
    /// A spear stuck in a fighter follows the body part it hit this tick (`feet`: where each
    /// fighter stands, `bones`: the fallen bodies' bones). Anything else stays as it is.
    fn ride_victim(&mut self, feet: &dyn Fn(u8) -> Option<([f32; 3], f32)>, bones: &[BoneFrame]) {
        let Fate::Stuck(Hold::Fighter {
            slot,
            anchor,
            local,
            dir,
        }) = self.fate
        else {
            return;
        };
        let Some((vpos, vyaw)) = feet(slot) else {
            return;
        };
        let held = Held {
            slot,
            anchor,
            local,
            dir,
        };
        let ridden = ride(held, (self.tip, self.dir), Frame::stand(vpos, vyaw), bones);
        self.prev = self.tip;
        self.tip = ridden.tip;
        self.dir = ridden.world_dir;
        self.fate = Fate::Stuck(Hold::Fighter {
            slot,
            anchor: ridden.anchor,
            local: ridden.local,
            dir: ridden.dir,
        });
    }

    fn push_trail(&mut self, tip: [f32; 3]) {
        self.trail.copy_within(0..TRAIL_LEN - 1, 1);
        self.trail[0] = tip;
        self.trail_len = (self.trail_len + 1).min(TRAIL_LEN);
    }

    /// A dropped spear falls tip first under gravity, then lies on the sand, tilted.
    fn dropped_pose(&self, age: u16, t: f32) -> ([f32; 3], [f32; 3]) {
        let secs = (f32::from(age) + t) / TICK_HZ;
        let fall = 0.5 * GRAVITY_M_S2 * secs * secs;
        let y = (self.tip[1] - fall).max(0.05);
        let settled = (fall / self.tip[1].max(0.1)).clamp(0.0, 1.0);
        let dir = unit([
            self.dir[0],
            self.dir[1] * (1.0 - settled) - 0.25 * settled,
            self.dir[2],
        ]);
        ([self.tip[0], y, self.tip[2]], dir)
    }
}

/// Where the far end of a rope ties to a stuck spear, and how far it has moved onto the tail
/// (`0..=1`, from `was`): the spear's butt while that is clearly nearer the thrower's `hand` than
/// the `anchor` (the shaft stands out toward the thrower), else the anchor itself. The weight
/// eases, so a spear that turns with its body slides the knot back instead of popping it.
/// `spear` is the world tip and the unit direction of the shaft. Pure.
fn rope_far(
    hand: [f32; 3],
    anchor: [f32; 3],
    spear: ([f32; 3], [f32; 3]),
    was: f32,
) -> ([f32; 3], f32) {
    let tail = sub(spear.0, scale(spear.1, SPEAR_LEN_M));
    let nearer = norm(sub(hand, tail)) + TAIL_MARGIN_M < norm(sub(hand, anchor));
    let goal = if nearer { 1.0 } else { 0.0 };
    let weight = (was + (goal - was) * TAIL_EASE).clamp(0.0, 1.0);
    (lerp(anchor, tail, weight), weight)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{App, GameConfig, TickOutcome, HUMAN_SLOT};
    use engine_input::fighter::THROW_SPEAR;
    use engine_input::{Buttons, FighterId, FighterInput};

    fn config() -> GameConfig {
        GameConfig {
            seed: 2040,
            ticks: 400,
            fighters: 2,
            half_extent_m: 5.0,
        }
    }

    fn input(tick: u32, buttons: u8, facing: i16) -> FighterInput {
        FighterInput {
            tick,
            fighter: FighterId::new(HUMAN_SLOT).expect("slot"),
            mx: 0,
            mz: 0,
            facing,
            buttons: Buttons(buttons),
        }
    }

    /// A spear session where slot 0 throws at tick 1 straight at the other fighter, followed
    /// for `ticks` ticks into one picture. Returns the picture and every tick's carries.
    fn run(ticks: u32) -> (SpearLook, Vec<[Option<Carry>; MAX_LOOK_FIGHTERS]>, App) {
        let mut app = App::new(config())
            .expect("session")
            .into_spear()
            .expect("spear");
        let mut look = SpearLook::new(DEFAULT_TIER);
        let mut out = TickOutcome::default();
        let mut carries = Vec::new();
        for tick in 0..ticks {
            let buttons = if tick == 1 { THROW_SPEAR } else { 0 };
            app.tick_into(input(tick, buttons, facing_to_other(&app)), &mut out);
            let live = app.live_match();
            look.observe(
                live.spear_events(),
                live.spear_view(),
                app.fighter_views(),
                Ties::NONE,
            );
            carries.push(*look.carries());
        }
        (look, carries, app)
    }

    /// Centidegree facing from slot 0 toward slot 1 (0 is +z, 9000 is +x).
    fn facing_to_other(app: &App) -> i16 {
        let views = app.fighter_views();
        let (Some(a), Some(b)) = (views.first(), views.get(1)) else {
            return 0;
        };
        let deg = (b.pos_m[0] - a.pos_m[0])
            .atan2(b.pos_m[2] - a.pos_m[2])
            .to_degrees();
        (deg * 100.0).round() as i16
    }

    #[test]
    fn spear_look_follows_a_throw_from_flight_to_a_tether_and_back() {
        let (look, carries, _) = run(400);
        let stats = look.stats();
        assert_eq!(stats.thrown, 1, "{stats:?}");
        assert!(stats.hits >= 1 && stats.tethers >= 1, "{stats:?}");
        assert_eq!(stats.ends, 1, "the tether ended: {stats:?}");
        assert!(stats.spears_max == 1 && stats.ropes_max == 1, "{stats:?}");
        // Both ends of the tether were carried at once, thrower and victim.
        let both = carries
            .iter()
            .filter(|c| c[0].is_some_and(|c| c.role == Role::Thrower) && c[1].is_some())
            .count();
        assert!(both > 5, "{both} ticks carried both bodies");
        // After the end the carries are gone and the rope has settled and been removed.
        assert!(carries.last().expect("ticks")[0].is_none());
        assert_eq!(look.ropes(), 0);
    }

    #[test]
    fn spear_look_rope_ties_the_two_fighters_and_stays_finite_and_capped() {
        let mut app = App::new(config())
            .expect("session")
            .into_spear()
            .expect("spear");
        let mut look = SpearLook::new(Tier::Potato);
        let mut out = TickOutcome::default();
        let mut taut = 0;
        for tick in 0..120 {
            let buttons = if tick == 1 { THROW_SPEAR } else { 0 };
            app.tick_into(input(tick, buttons, facing_to_other(&app)), &mut out);
            let live = app.live_match();
            look.observe(
                live.spear_events(),
                live.spear_view(),
                app.fighter_views(),
                Ties::NONE,
            );
            if let Some(nodes) = look.rope(0, 1.0) {
                assert_eq!(nodes.len(), Tier::Potato.caps().tether_nodes);
                assert!(nodes.iter().flatten().all(|c| c.is_finite()));
                taut += usize::from(
                    look.rope_state(0)
                        .is_some_and(|(s, _)| s == TetherState::Taut),
                );
            }
        }
        assert!(taut > 0, "the rope pulled taut while reeling");
        assert!(look.ropes() <= Tier::Potato.caps().tethers);
    }

    #[test]
    fn spear_look_draws_the_flying_spear_between_its_ticks_and_trails_it() {
        let mut app = App::new(config())
            .expect("session")
            .into_spear()
            .expect("spear");
        let mut look = SpearLook::new(DEFAULT_TIER);
        let mut out = TickOutcome::default();
        for tick in 0..6 {
            let buttons = if tick == 1 { THROW_SPEAR } else { 0 };
            app.tick_into(input(tick, buttons, facing_to_other(&app)), &mut out);
            let live = app.live_match();
            look.observe(
                live.spear_events(),
                live.spear_view(),
                app.fighter_views(),
                Ties::NONE,
            );
        }
        let (early, late) = (look.draws(0.0), look.draws(1.0));
        let (a, b) = (early.first().expect("spear"), late.first().expect("spear"));
        assert!(a.flying && b.flying);
        let moved = norm(sub(b.tip, a.tip));
        assert!(
            moved > 0.3 && moved < 0.7,
            "36 m/s is 0.6 m a tick: {moved}"
        );
        let mid = look.draws(0.5);
        assert!((norm(sub(mid[0].tip, a.tip)) - moved * 0.5).abs() < 0.02);
        assert!(look.trail(0).len() >= 3, "a flying spear trails");
        assert!((norm(b.dir) - 1.0).abs() < 1e-4);
    }

    #[test]
    fn spear_look_ties_the_thrower_end_of_the_rope_to_the_drawn_palm() {
        let mut app = App::new(config())
            .expect("session")
            .into_spear()
            .expect("spear");
        let mut look = SpearLook::new(DEFAULT_TIER);
        let mut out = TickOutcome::default();
        let mut palms = [None; MAX_LOOK_FIGHTERS];
        let mut tied = 0;
        for tick in 0..12 {
            let buttons = if tick == 1 { THROW_SPEAR } else { 0 };
            app.tick_into(input(tick, buttons, facing_to_other(&app)), &mut out);
            let live = app.live_match();
            // A palm well off the authority place's guess (0.3 m ahead at 1.3 m up).
            palms[0] = app
                .fighter_views()
                .first()
                .map(|f| add(f.pos_m, [0.5, 1.6, -0.4]));
            look.observe(
                live.spear_events(),
                live.spear_view(),
                app.fighter_views(),
                Ties {
                    pelvis: &[None; MAX_LOOK_FIGHTERS],
                    palms: &palms,
                    bones: &[],
                },
            );
            if let (Some(nodes), Some(palm)) = (look.rope(0, 1.0), palms[0]) {
                let end = nodes.first().expect("a rope has nodes");
                assert!(
                    norm(sub(*end, palm)) < 0.03,
                    "tick {tick}: rope end {end:?} vs palm {palm:?}"
                );
                tied += 1;
            }
        }
        assert!(tied >= 3, "the rope was tied for {tied} ticks");
    }

    #[test]
    fn spear_look_yank_tows_the_victim_and_leaves_the_thrower_standing() {
        let mut app = App::new(config())
            .expect("session")
            .into_spear()
            .expect("spear");
        let mut look = SpearLook::new(DEFAULT_TIER);
        let mut out = TickOutcome::default();
        let (mut hooked, mut towed, mut taut, mut thrower_carried) = (false, 0, 0, 0);
        for tick in 0..200 {
            let facing = facing_to_other(&app);
            let mut step = input(tick, if tick == 1 { THROW_SPEAR } else { 0 }, facing);
            if !hooked {
                // Hold back, away from the throw, until the spear sticks: the yank.
                let (s, c) = (f32::from(facing) / 100.0).to_radians().sin_cos();
                step.mx = (-s * 100.0).round() as i8;
                step.mz = (-c * 100.0).round() as i8;
            }
            app.tick_into(step, &mut out);
            let live = app.live_match();
            hooked |= live.spear_events().iter().any(|e| {
                matches!(
                    e,
                    SpearEvent::Hit {
                        victim: Some(_),
                        ..
                    }
                )
            });
            look.observe(
                live.spear_events(),
                live.spear_view(),
                app.fighter_views(),
                Ties::NONE,
            );
            let carries = look.carries();
            thrower_carried += usize::from(carries[0].is_some());
            towed += usize::from(carries[1].is_some_and(|c| c.role == Role::Victim));
            taut += usize::from(
                look.rope_state(0)
                    .is_some_and(|(s, _)| s == TetherState::Taut),
            );
        }
        assert!(hooked, "the spear stuck");
        assert!(towed > 3, "the victim was towed for {towed} ticks");
        assert_eq!(thrower_carried, 0, "a yank never tows the thrower");
        assert!(taut > 0, "the rope hauls taut");
        assert_eq!(look.stats().ends, 1, "{:?}", look.stats());
    }

    /// Two points less than a tenth of a millimeter apart.
    fn within(a: [f32; 3], b: [f32; 3]) -> bool {
        norm(sub(a, b)) < 1.0e-4
    }

    #[test]
    fn a_rope_ties_to_the_spears_butt_while_it_stands_out_toward_the_thrower() {
        let hand = [0.0, 1.3, 0.0];
        let anchor = [8.0, 1.1, 0.0];
        let tail_of = |tip: [f32; 3], dir: [f32; 3]| sub(tip, scale(dir, SPEAR_LEN_M));
        // The shaft points from the thrower into the victim: its butt is 1.9 m short of them.
        let toward = ([8.0, 1.1, 0.0], [1.0, 0.0, 0.0]);
        let (end, weight) = rope_far(hand, anchor, toward, 1.0);
        assert_eq!(weight, 1.0);
        assert!(within(end, tail_of(toward.0, toward.1)), "{end:?}");
        // A shaft that points back at the thrower has its butt behind the victim: the rope ties
        // to the body, easing over a few ticks, not in one jump.
        let away = ([8.0, 1.1, 0.0], [-1.0, 0.0, 0.0]);
        let (first, w) = rope_far(hand, anchor, away, 1.0);
        assert!((w - (1.0 - TAIL_EASE)).abs() < 1.0e-6);
        assert!(norm(sub(first, anchor)) < SPEAR_LEN_M && !within(first, anchor));
        let settled = (0..40).fold((first, w), |(_, w), _| rope_far(hand, anchor, away, w));
        assert!(
            within(settled.0, anchor) && settled.1 < 1.0e-3,
            "{settled:?}"
        );
        // Close in, the butt is not clearly nearer than the body: the rope ties to the body.
        let close = ([1.0, 1.1, 0.0], [1.0, 0.0, 0.0]);
        let (end, w) = rope_far(hand, [1.0, 1.1, 0.0], close, 0.0);
        assert_eq!((end, w), ([1.0, 1.1, 0.0], 0.0));
        // Whatever the weight, the end lies on the shaft between the body and the butt.
        for was in [0.0, 0.3, 1.0] {
            let (end, _) = rope_far(hand, anchor, toward, was);
            let (a, b) = (sub(end, anchor), sub(tail_of(toward.0, toward.1), anchor));
            assert!(norm(a) <= norm(b) + 1.0e-5);
        }
    }

    #[test]
    fn the_rope_of_a_real_throw_hangs_from_the_spears_tail_once_it_sticks() {
        use crate::spear_window::demo_input;
        let cfg = GameConfig {
            seed: 2040,
            ticks: 600,
            fighters: 2,
            half_extent_m: 20.0,
        };
        let mut app = App::new(cfg).expect("session").into_spear().expect("spear");
        let human = engine_input::FighterId::new(HUMAN_SLOT).expect("slot");
        let mut look = SpearLook::new(DEFAULT_TIER);
        let mut out = TickOutcome::default();
        let (mut on_tail, mut stuck_ticks) = (0, 0);
        for tick in 0..600 {
            let step = demo_input(
                tick,
                human,
                app.fighter_views(),
                app.live_match().spear_view(),
            );
            app.tick_into(step, &mut out);
            let live = app.live_match();
            look.observe(
                live.spear_events(),
                live.spear_view(),
                app.fighter_views(),
                Ties::NONE,
            );
            let (Some(spear), Some(nodes)) = (look.lanes[0].spear, look.rope(0, 1.0)) else {
                continue;
            };
            let Some(end) = nodes.last().copied() else {
                continue;
            };
            if matches!(spear.fate, Fate::Stuck(_)) {
                stuck_ticks += 1;
                if look.lanes[0].tail_w > 0.999 {
                    on_tail += 1;
                    let tail = sub(spear.tip, scale(spear.dir, SPEAR_LEN_M));
                    assert!(
                        norm(sub(end, tail)) < 0.02,
                        "tick {tick}: rope ends {end:?}, the butt is at {tail:?}"
                    );
                }
            }
        }
        assert!(stuck_ticks > 5, "the spear stuck for {stuck_ticks} ticks");
        assert!(
            on_tail > 0,
            "the rope hung from the butt for {on_tail} ticks"
        );
    }

    #[test]
    fn spear_look_same_match_gives_the_same_picture_twice() {
        let picture = || {
            let (look, _, _) = run(90);
            (look.draws(0.5), look.rope(0, 0.5), look.stats())
        };
        assert_eq!(picture(), picture());
    }

    #[test]
    fn spear_look_idle_match_draws_nothing() {
        let mut app = App::new(config()).expect("session");
        let mut look = SpearLook::new(DEFAULT_TIER);
        let mut out = TickOutcome::default();
        for tick in 0..30 {
            app.tick_into(input(tick, 0, 0), &mut out);
            let live = app.live_match();
            look.observe(
                live.spear_events(),
                live.spear_view(),
                app.fighter_views(),
                Ties::NONE,
            );
        }
        assert!(look.draws(1.0).is_empty());
        assert_eq!((look.spears(), look.ropes()), (0, 0));
        assert_eq!(look.stats(), SpearStats::default());
    }

    #[test]
    fn spear_look_carry_arc_rises_with_age_and_speed_and_is_bounded() {
        let at = |age, speed_ms| {
            Carry {
                role: Role::Victim,
                age,
                speed_ms,
            }
            .lift_m()
        };
        assert_eq!(at(0, 20.0), 0.0);
        assert!(at(7, 20.0) < at(14, 20.0));
        assert!(at(14, 4.0) < at(14, 20.0));
        assert!((at(200, 99.0) - ARC_PEAK_M).abs() < 1e-6);
        assert_eq!(at(14, 0.0), 0.0);
    }

    const FEET: [f32; 3] = [1.0, 0.0, 2.0];

    /// A hit 1.1 m up and 0.2 m ahead of `FEET`, the shaft going down and forward.
    fn wound() -> ([f32; 3], [f32; 3]) {
        ([1.0, 1.1, 2.2], unit([0.0, -0.4, 0.9]))
    }

    /// The hold of that wound on a standing body facing +Z, kept the way a hit keeps it.
    fn standing_hold() -> Held {
        let (at, flight) = wound();
        let (local, dir) = Frame::stand(FEET, 0.0).to_local(at, flight);
        Held {
            slot: 1,
            anchor: Anchor::Body,
            local,
            dir,
        }
    }

    fn near(a: [f32; 3], b: [f32; 3]) -> bool {
        norm(sub(a, b)) < 1.0e-4
    }

    /// A quarter turn about +X: a standing body's up becomes its forward.
    const LYING: [f32; 4] = [
        std::f32::consts::FRAC_1_SQRT_2,
        0.0,
        0.0,
        std::f32::consts::FRAC_1_SQRT_2,
    ];

    #[test]
    fn a_stuck_spear_keeps_its_wound_and_angle_when_the_body_turns() {
        let (at, flight) = wound();
        let held = standing_hold();
        let still = ride(held, (at, flight), Frame::stand(FEET, 0.0), &[]);
        assert!(near(still.tip, at) && near(still.world_dir, flight));
        // A quarter turn about +Y: the wound that was ahead is now to the side (+X).
        let turned = ride(
            held,
            (at, flight),
            Frame::stand(FEET, std::f32::consts::FRAC_PI_2),
            &[],
        );
        assert!(near(turned.tip, [1.2, 1.1, 2.0]), "{:?}", turned.tip);
        assert!(near(turned.world_dir, unit([0.9, -0.4, 0.0])));
    }

    #[test]
    fn a_stuck_spear_takes_the_nearest_bone_and_follows_it_down() {
        let (at, flight) = wound();
        let stand = Frame::stand(FEET, 0.0);
        let bone = |slot, bone, pos, rot| BoneFrame {
            slot,
            bone,
            pos,
            rot,
        };
        let standing = [
            bone(1, 0, [1.0, 0.95, 2.0], [0.0, 0.0, 0.0, 1.0]),
            bone(1, 3, [1.0, 1.15, 2.0], [0.0, 0.0, 0.0, 1.0]),
            // Another fighter's bone right at the wound: never taken.
            bone(0, 3, [1.0, 1.1, 2.2], [0.0, 0.0, 0.0, 1.0]),
        ];
        let first = ride(standing_hold(), (at, flight), stand, &standing);
        assert_eq!(first.anchor, Anchor::Bone(3), "the chest is nearest");
        assert!(near(first.tip, at), "no jump when the bone takes over");
        assert!(near(first.world_dir, flight));
        // The body falls: the chest is 3 m away and turned a quarter.
        let fallen = [bone(1, 3, [4.0, 0.2, 2.0], LYING)];
        let held = Held {
            anchor: first.anchor,
            local: first.local,
            dir: first.dir,
            ..standing_hold()
        };
        let down = ride(held, (first.tip, first.world_dir), stand, &fallen);
        assert!(
            norm(sub(down.tip, [4.0, 0.2, 2.0])) < 0.5,
            "the spear is in the fallen body: {:?}",
            down.tip
        );
        assert!(
            (norm(sub(down.tip, [4.0, 0.2, 2.0])) - norm(sub(at, [1.0, 1.15, 2.0]))).abs() < 1e-4,
            "the wound keeps its depth from the bone"
        );
        // The angle between the shaft and the bone's own axis is the one it was hit at.
        let axis = |rot: [f32; 4]| rotate_vec(&[0.0, 1.0, 0.0], &rot);
        let dot = |a: [f32; 3], b: [f32; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
        let hit = dot(flight, axis([0.0, 0.0, 0.0, 1.0]));
        let now = dot(down.world_dir, axis(LYING));
        assert!((hit - now).abs() < 1e-4, "hit {hit} now {now}");
        assert!(
            dot(down.world_dir, [0.0, 1.0, 0.0]).abs() < 0.99 && down.world_dir != flight,
            "the shaft turned with the body"
        );
    }

    #[test]
    fn a_bone_that_vanishes_hands_the_spear_back_without_a_jump() {
        let (at, _flight) = wound();
        let held = Held {
            anchor: Anchor::Bone(3),
            ..standing_hold()
        };
        let last = ([4.0, 0.3, 2.0], unit([1.0, 0.0, 0.0]));
        let back = ride(held, last, Frame::stand(FEET, 0.0), &[]);
        assert_eq!(back.anchor, Anchor::Body);
        assert!(near(back.tip, last.0) && near(back.world_dir, last.1));
        assert!(!near(back.tip, at));
    }

    #[test]
    fn a_spear_in_a_fallen_victim_is_drawn_in_the_fallen_body_not_over_the_feet() {
        let mut app = App::new(config())
            .expect("session")
            .into_spear()
            .expect("spear");
        let mut look = SpearLook::new(DEFAULT_TIER);
        let mut out = TickOutcome::default();
        let (mut stuck_ticks, mut checked) = (0, false);
        for tick in 0..200 {
            let buttons = if tick == 1 { THROW_SPEAR } else { 0 };
            app.tick_into(input(tick, buttons, facing_to_other(&app)), &mut out);
            let live = app.live_match();
            let victim = app
                .fighter_views()
                .iter()
                .find(|f| f.slot == 1)
                .map(|f| f.pos_m);
            // The ragdoll appears at the chest on the second stuck tick, where the wound is, then
            // the body lies 6 m off to the side from the fifth.
            let at = |off: [f32; 3], rot: [f32; 4]| {
                victim.map(|v| BoneFrame {
                    slot: 1,
                    bone: 5,
                    pos: add(v, off),
                    rot,
                })
            };
            let lying = at([6.0, 0.2, 0.0], LYING);
            let bones: Vec<BoneFrame> = match stuck_ticks {
                0..=1 => None,
                2..=4 => at([0.0, 1.1, 0.0], [0.0, 0.0, 0.0, 1.0]),
                _ => lying,
            }
            .into_iter()
            .collect();
            look.observe(
                live.spear_events(),
                live.spear_view(),
                app.fighter_views(),
                Ties {
                    bones: &bones,
                    ..Ties::NONE
                },
            );
            let stuck = look.carries()[1].is_some_and(|c| c.role == Role::Victim);
            stuck_ticks += i32::from(stuck);
            if let (true, Some(draw), Some(body)) =
                (stuck && stuck_ticks > 6, look.draws(1.0).first(), lying)
            {
                assert!(
                    norm(sub(draw.tip, body.pos)) < 2.0,
                    "tip {:?} is nowhere near the fallen body at {:?}",
                    draw.tip,
                    body.pos
                );
                checked = true;
            }
        }
        assert!(checked, "the spear stuck in the victim for a few ticks");
    }

    /// A standing body of three capsules: pelvis at 1.1 m, spine, chest.
    fn test_body() -> (engine_physfx::Rig, engine_anim::contract::LocalPose) {
        use engine_anim::contract::{Joint, LocalPose, Skeleton};
        use engine_physfx::{BoneShape, Rig as FxRig};
        let rows = [
            ("pelvis", None, [0.0, 1.1, 0.0], (0.14, 0.05)),
            ("spine", Some(0), [0.0, 0.25, 0.0], (0.13, 0.08)),
            ("chest", Some(1), [0.0, 0.25, 0.0], (0.15, 0.10)),
        ];
        let skeleton = Skeleton {
            joints: rows
                .iter()
                .map(|(name, parent, _, _)| Joint {
                    name: (*name).into(),
                    parent: *parent,
                })
                .collect(),
        };
        let shapes = rows
            .iter()
            .map(|(_, _, _, (radius, half_height))| BoneShape {
                radius: *radius,
                half_height: *half_height,
            })
            .collect();
        let pose = LocalPose {
            positions: rows.iter().map(|r| r.2).collect(),
            rotations: vec![[0.0, 0.0, 0.0, 1.0]; rows.len()],
        };
        (FxRig::new(skeleton, shapes).expect("a body"), pose)
    }

    /// How far `p` is outside `block`: negative inside it.
    fn outside(block: &RopeBlock, p: [f32; 3]) -> f32 {
        let axis = sub(block.b, block.a);
        let l2: f32 = axis.iter().map(|c| c * c).sum();
        let along = if l2 < 1.0e-12 {
            0.0
        } else {
            (sub(p, block.a)
                .iter()
                .zip(axis)
                .map(|(d, c)| d * c)
                .sum::<f32>()
                / l2)
                .clamp(0.0, 1.0)
        };
        let foot = add(block.a, scale(axis, along));
        norm(sub(p, foot)) - block.r
    }

    #[test]
    fn a_rope_tied_across_a_body_never_draws_a_node_inside_it() {
        // The body is held standing (the look's carry is what pins a fallen fighter) and the rope
        // runs from a hand 2 m to its left, through its chest, to a hold 2 m to its right. Its
        // nodes take the body's own capsules once a tick, so the drawn rope goes round the chest.
        let (rig, pose) = test_body();
        let (a, b) = ([-2.0, 1.35, 0.0], [2.0, 1.35, 0.0]);
        let mut look = SpearLook::new(DEFAULT_TIER);
        look.fx
            .spawn_ragdoll(0, &rig, &pose, [0.0; 3], [0.0; 3])
            .expect("a body");
        let standing = [0.0, 1.1, 0.0];
        let mut closest = f32::MAX;
        for _ in 0..60 {
            look.fx
                .carry_ragdoll(0, standing, [0.0; 3], true)
                .expect("carry");
            let blocks = look.fx.tether_blocks();
            look.tie(0, a, b, 4.0, &blocks);
            look.fx.step();
            let nodes = look.fx.tether_nodes(0, 1.0).expect("alpha").expect("rope");
            for node in nodes {
                for block in look.fx.tether_blocks() {
                    closest = closest.min(outside(&block, node));
                }
            }
        }
        assert!(
            closest >= -1.0e-3,
            "a node sits {} m inside a body's capsule",
            -closest
        );

        // Without the capsules the same rope hangs through the same chest: the test is not vacuous.
        let mut plain = SpearLook::new(DEFAULT_TIER);
        plain
            .fx
            .spawn_ragdoll(0, &rig, &pose, [0.0; 3], [0.0; 3])
            .expect("a body");
        let mut through = 0;
        plain.fx.attach_tether(0, a, b, 4.0).expect("a plain rope");
        for _ in 0..60 {
            plain
                .fx
                .carry_ragdoll(0, standing, [0.0; 3], true)
                .expect("carry");
            plain.fx.move_tether(0, a, b).expect("ends");
            plain.fx.step();
            let nodes = plain.fx.tether_nodes(0, 1.0).expect("alpha").expect("rope");
            for node in nodes {
                for block in plain.fx.tether_blocks() {
                    if outside(&block, node) < -0.01 {
                        through += 1;
                    }
                }
            }
        }
        assert!(through > 0, "a plain rope must hang through the body");
    }
}
