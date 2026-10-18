//! spear_look: the picture of the spear grapple, a spear in flight, stuck, or dropped, and the
//! rope between the two fighters it joins (`docs/plan/spear-grapple.md`).
//!
//! The authority (`engine-arena`, integers, hashed) decides when a spear leaves a hand, what it
//! hits, who is tethered to whom and where both bodies go. This module reads only what the match
//! exposes (`LiveMatch::spear_events`, `LiveMatch::spear_view`) plus the fighters' drawn places,
//! and keeps floats: the spear's pose and spin, a trail behind it, a verlet rope from
//! `engine-physfx` that sags while slack and straightens when taut, and for each tethered body
//! a [`Carry`] that tells the ragdoll layer where the authority is taking it and how high the
//! arc lifts it. It never feeds the sim: the golden `d57b5c8ae3096614` cannot move.
//!
//! Callers: `spear_window::SpearScene` (one `observe` per accepted tick, the draws per frame) and
//! `FighterLook::observe_carries` (the ragdoll layer). Units: meters, seconds, Y up; authority
//! millimeters are converted at the door. Determinism: the same events and views give the same
//! picture twice on one machine. Budget: at most one rope per slot, the tier's node count each;
//! no allocation per tick beyond the rope's own node buffer.

use crate::fighter_look::MAX_LOOK_FIGHTERS;
use crate::FighterView;
use engine_arena::spear::{Phase, SpearEvent, SpearSlot};
use engine_physfx::{PhysFx, TetherState, Tier};

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

/// What a spear is stuck in.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Hold {
    /// A fighter: the tip sits at `offset` from the fighter's feet.
    Fighter { slot: u8, offset: [f32; 3] },
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
}

const EMPTY_LANE: Lane = Lane {
    spear: None,
    rope: false,
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
    /// places and, for a ragdolled body, where its pelvis is (the rope ties to it). Steps the
    /// rope world once.
    pub fn observe(
        &mut self,
        events: &[SpearEvent],
        view: &[SpearSlot],
        fighters: &[FighterView],
        pelvis: &[Option<[f32; 3]>; MAX_LOOK_FIGHTERS],
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
            self.fold_slot(thrower, slot, &feet, pelvis);
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
                let hold = match victim.and_then(|v| feet(v).map(|(pos, _)| (v, pos))) {
                    Some((slot, pos)) => Hold::Fighter {
                        slot,
                        offset: sub(at, pos),
                    },
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
        pelvis: &[Option<[f32; 3]>; MAX_LOOK_FIGHTERS],
    ) {
        let index = usize::from(thrower);
        let Some((pos, yaw)) = feet(thrower) else {
            return;
        };
        let hand = self.anchor(thrower, pos, yaw, pelvis, true);
        match slot.phase {
            Phase::Idle => {
                if self.lanes[index].rope {
                    self.fx.release_tether(u32::from(thrower));
                    self.lanes[index].rope = false;
                }
                self.age_dropped(index);
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
                self.tie(thrower, hand, tail, gap * FLIGHT_SLACK + FLIGHT_EXTRA_M);
            }
            Phase::Tether(tether) => {
                let speed_ms = f32::from(tether.speed) * TICK_HZ / MM;
                let carry = |role| Carry {
                    role,
                    age: tether.age,
                    speed_ms,
                };
                self.bodies[index] = Some(carry(Role::Thrower));
                let far = match tether.victim {
                    Some(victim) => {
                        if let Some(slot) = self.bodies.get_mut(usize::from(victim)) {
                            *slot = Some(carry(Role::Victim));
                        }
                        feet(victim)
                            .map(|(vpos, vyaw)| self.anchor(victim, vpos, vyaw, pelvis, false))
                    }
                    None => self.lanes[index].spear.map(|s| s.tip),
                };
                if let Some(spear) = self.lanes[index].spear.as_mut() {
                    if let Fate::Stuck(Hold::Fighter { slot, offset }) = spear.fate {
                        if let Some((vpos, _)) = feet(slot) {
                            spear.prev = spear.tip;
                            spear.tip = add(vpos, offset);
                        }
                    }
                }
                if let Some(far) = far {
                    let gap = norm(sub(far, hand));
                    let slack = if tether.latched { LATCHED_SLACK } else { 1.0 };
                    self.tie(thrower, hand, far, gap * slack);
                }
            }
        }
    }

    /// Where a rope ties to a body: its ragdoll pelvis lifted to the chest, else the
    /// authority place at chest height; the thrower's end is the hand in front of it.
    fn anchor(
        &self,
        slot: u8,
        pos: [f32; 3],
        yaw: f32,
        pelvis: &[Option<[f32; 3]>; MAX_LOOK_FIGHTERS],
        hand: bool,
    ) -> [f32; 3] {
        if let Some(Some(root)) = pelvis.get(usize::from(slot)) {
            return add(*root, [0.0, PELVIS_TO_CHEST_M, 0.0]);
        }
        if hand {
            let (s, c) = yaw.sin_cos();
            add(pos, [s * HAND_FORWARD_M, HAND_Y_M, c * HAND_FORWARD_M])
        } else {
            add(pos, [0.0, CHEST_Y_M, 0.0])
        }
    }

    /// Ties the rope of `thrower` from `a` to `b`, or moves and reels the one already tied.
    fn tie(&mut self, thrower: u8, a: [f32; 3], b: [f32; 3], rest_m: f32) {
        let id = u32::from(thrower);
        let rest = rest_m.max(0.3);
        let index = usize::from(thrower);
        if self.lanes[index].rope && self.fx.tether_info(id).is_some() {
            let moved = self.fx.move_tether(id, a, b);
            let reeled = self.fx.retune_tether(id, rest);
            if moved.is_ok() && reeled.is_ok() {
                return;
            }
        }
        self.lanes[index].rope = self.fx.attach_tether(id, a, b, rest).is_ok();
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{App, GameConfig, TickOutcome, HUMAN_SLOT};
    use engine_input::fighter::THROW_SPEAR;
    use engine_input::{Buttons, FighterId, FighterInput};

    const NO_PELVIS: [Option<[f32; 3]>; MAX_LOOK_FIGHTERS] = [None; MAX_LOOK_FIGHTERS];

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
                &NO_PELVIS,
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
                &NO_PELVIS,
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
                &NO_PELVIS,
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
                &NO_PELVIS,
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
}
