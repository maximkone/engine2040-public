//! tether: a rope between two points, a verlet chain with a spring-damper pull.
//!
//! The authority decides that a spear holds two fighters together and where each of them is; this
//! file only draws the rope between them and says how hard it pulls. Both ends are pinned to the
//! points the caller gives every tick. Between them a chain of light nodes sags under gravity
//! while the ends are closer than the rope is long (`Slack`) and lies straight once they are
//! not (`Taut`). A released rope has no pinned end and falls to the floor. A rope whose nodes
//! and ends stand still for [`QUIET_TICKS`] ticks sleeps and costs nothing until an end moves.
//!
//! Plain float math, no clock, no randomness: the same ends give the same nodes on one machine.

use crate::error::Error;
use std::f32::consts::PI;

/// Gravity on a rope node, m/s^2 (the world's).
const GRAVITY: f32 = -9.81;
/// Fixed step, seconds.
const DT: f32 = 1.0 / 60.0;
/// Share of a node's speed kept each tick: the air and the rope's own stiffness.
const KEEP: f32 = 0.985;
/// A node never moves further than this in one tick, meters: a bad input cannot launch the chain.
const MAX_STEP_M: f32 = 2.0;
/// Nodes rest at least this high, meters: the floor under the sand.
const FLOOR_M: f32 = 0.02;
/// Ticks of stillness before a rope sleeps.
pub(crate) const QUIET_TICKS: u32 = 20;
/// A released rope is removed after this many ticks even if it never quite settles.
const LOOSE_TICKS: u32 = 240;
/// Passes of the length projection a slack rope takes before its first tick, so its nodes start
/// on their length and the first tick does not throw them.
const SEAT_PASSES: u32 = 4;
/// An end that moved less than this since the last step, meters, counts as still.
const STILL_M: f32 = 0.001;
/// Spring stiffness of the pull, newtons per meter of stretch.
const PULL_STIFFNESS: f32 = 400.0;
/// Damping of the pull, newtons per m/s of stretching speed.
const PULL_DAMPING: f32 = 40.0;

/// How tight a rope is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TetherState {
    /// The ends are closer than the rope is long: it sags.
    Slack,
    /// The ends are as far as the rope reaches, or farther: it lies straight and pulls.
    Taut,
    /// Neither end is held: the rope falls and settles.
    Loose,
}

/// One rope. `prev` and `curr` are the node positions one tick ago and now, which is both the
/// verlet velocity and what the render blend reads.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Tether {
    pub(crate) id: u32,
    rest: f32,
    prev: Vec<[f32; 3]>,
    curr: Vec<[f32; 3]>,
    /// Where the two ends are asked to be at the next step; `None` once released.
    ends: Option<([f32; 3], [f32; 3])>,
    /// End distance at the last two steps, newest last, for the damping of the pull.
    reach: [f32; 2],
    state: TetherState,
    quiet: u32,
    /// Ticks since both ends were let go.
    loose: u32,
    sweeps: usize,
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn len(v: [f32; 3]) -> f32 {
    (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt()
}

/// `i / n` as a float; a rope has a few dozen nodes, far below `u16::MAX`.
fn share(i: usize, n: usize) -> f32 {
    let f = |k: usize| f32::from(u16::try_from(k).unwrap_or(u16::MAX));
    f(i) / f(n).max(1.0)
}

fn lerp(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [0, 1, 2].map(|k| a[k] + (b[k] - a[k]) * t)
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// The `nodes` points of a rope of length `rest` hung between `a` and `b`, ends included: a circle
/// arc of that length bowing down from the chord. A chord with no down side (stacked ends) bows
/// toward +x. Nearly straight or degenerate ends fall back to the chord itself.
fn sag_arc(a: [f32; 3], b: [f32; 3], rest: f32, nodes: usize) -> Vec<[f32; 3]> {
    let chord = sub(b, a);
    let c = len(chord);
    if c < 1.0e-3 || c >= rest {
        return (0..nodes)
            .map(|i| lerp(a, b, share(i, nodes - 1)))
            .collect();
    }
    // sin(t) / t = chord / rest picks the arc's half angle t; the ratio falls from 1 to 0 on (0, pi).
    let want = c / rest;
    let (mut lo, mut hi) = (0.0_f32, PI);
    for _ in 0..40 {
        let t = 0.5 * (lo + hi);
        if t.sin() / t > want {
            lo = t;
        } else {
            hi = t;
        }
    }
    let t = 0.5 * (lo + hi);
    let radius = c / (2.0 * t.sin());
    let d = chord.map(|x| x / c);
    let across = |axis: [f32; 3]| {
        let k = dot(axis, d);
        [0, 1, 2].map(|i| axis[i] - k * d[i])
    };
    let mut g = across([0.0, -1.0, 0.0]);
    if len(g) < 1.0e-6 {
        g = across([1.0, 0.0, 0.0]);
    }
    let gl = len(g);
    let g = g.map(|x| x / gl);
    let mid = [0, 1, 2].map(|k| (a[k] + b[k]) * 0.5);
    let center = [0, 1, 2].map(|k| mid[k] - g[k] * radius * t.cos());
    let last = nodes - 1;
    (0..nodes)
        .map(|i| {
            let psi = (2.0 * share(i, last) - 1.0) * t;
            [0, 1, 2].map(|k| center[k] + radius * (d[k] * psi.sin() + g[k] * psi.cos()))
        })
        .collect()
}

impl Tether {
    /// A rope from `a` to `b` of `nodes` points and `rest` meters of length: straight when the
    /// ends are as far apart as the rope is long, hung in its arc when they are closer. The caller
    /// has checked the numbers (`nodes >= 2`, `rest > 0`, finite ends).
    pub(crate) fn new(
        id: u32,
        a: [f32; 3],
        b: [f32; 3],
        rest: f32,
        nodes: usize,
        sweeps: usize,
    ) -> Self {
        let line: Vec<[f32; 3]> = (0..nodes)
            .map(|i| lerp(a, b, share(i, nodes - 1)))
            .collect();
        let reach = len(sub(b, a));
        let mut rope = Self {
            id,
            rest,
            prev: line.clone(),
            curr: line,
            ends: Some((a, b)),
            reach: [reach, reach],
            state: if reach >= rest {
                TetherState::Taut
            } else {
                TetherState::Slack
            },
            quiet: 0,
            loose: 0,
            sweeps,
        };
        if rope.state == TetherState::Slack {
            // A straight chord shorter than the rope would buckle on the first tick and throw its
            // nodes, so a slack rope starts hung in its arc and settled onto its length.
            rope.lay_sag(a, b);
            for _ in 0..SEAT_PASSES {
                rope.satisfy(true);
            }
            rope.prev.clone_from(&rope.curr);
        }
        rope
    }

    pub(crate) fn state(&self) -> TetherState {
        self.state
    }

    pub(crate) fn asleep(&self) -> bool {
        self.quiet >= QUIET_TICKS
    }

    /// A released rope that has lain still, or has lain [`LOOSE_TICKS`] ticks: done with.
    pub(crate) fn spent(&self) -> bool {
        self.state == TetherState::Loose && (self.asleep() || self.loose >= LOOSE_TICKS)
    }

    pub(crate) fn rest(&self) -> f32 {
        self.rest
    }

    /// Asks for the ends of the next step. A released rope ignores it.
    pub(crate) fn set_ends(&mut self, a: [f32; 3], b: [f32; 3]) {
        if let Some(ends) = self.ends.as_mut() {
            if sub(a, ends.0)
                .iter()
                .chain(&sub(b, ends.1))
                .any(|d| d.abs() > STILL_M)
            {
                self.quiet = 0;
            }
            *ends = (a, b);
        }
    }

    /// Takes in or pays out rope: the new length, meters. The state follows at the next step.
    pub(crate) fn set_rest(&mut self, rest: f32) {
        self.rest = rest;
        self.quiet = 0;
    }

    /// Lets go of both ends: the rope falls from here on.
    pub(crate) fn release(&mut self) {
        if self.ends.take().is_some() {
            self.state = TetherState::Loose;
            self.quiet = 0;
        }
    }

    /// The nodes blended between the last two ticks (`alpha` already in `[0, 1]`).
    pub(crate) fn nodes(&self, alpha: f32) -> Vec<[f32; 3]> {
        self.prev
            .iter()
            .zip(&self.curr)
            .map(|(p, c)| lerp(*p, *c, alpha))
            .collect()
    }

    /// The pull on the first end, toward the second, newtons: a spring on the stretch past the
    /// rope's length plus a damper on how fast it is stretching. A slack rope pulls with nothing,
    /// and a rope never pushes.
    pub(crate) fn pull(&self) -> [f32; 3] {
        let Some((a, b)) = self.ends else {
            return [0.0; 3];
        };
        let toward = sub(b, a);
        let dist = len(toward);
        let stretch = dist - self.rest;
        if stretch <= 0.0 || dist < 1.0e-6 {
            return [0.0; 3];
        }
        let speed = (self.reach[1] - self.reach[0]) / DT;
        let force = (PULL_STIFFNESS * stretch + PULL_DAMPING * speed).max(0.0);
        toward.map(|c| c / dist * force)
    }

    /// One tick: integrate the free nodes, pin the ends, satisfy the length.
    pub(crate) fn step(&mut self) {
        if self.asleep() {
            return;
        }
        if self.ends.is_none() {
            self.loose += 1;
        }
        let last = self.curr.len() - 1;
        let before = self.curr.clone();
        for i in 0..=last {
            let pinned = self.ends.is_some() && (i == 0 || i == last);
            if pinned {
                continue;
            }
            let [px, py, pz] = self.prev[i];
            let [cx, cy, cz] = self.curr[i];
            let mut v = [
                (cx - px) * KEEP,
                (cy - py) * KEEP + GRAVITY * DT * DT,
                (cz - pz) * KEEP,
            ];
            let speed = len(v);
            if speed > MAX_STEP_M {
                v = v.map(|c| c / speed * MAX_STEP_M);
            }
            self.prev[i] = self.curr[i];
            self.curr[i] = [cx + v[0], (cy + v[1]).max(FLOOR_M), cz + v[2]];
        }
        if let Some((a, b)) = self.ends {
            self.prev[0] = self.curr[0];
            self.prev[last] = self.curr[last];
            self.curr[0] = a;
            self.curr[last] = b;
            let reach = len(sub(b, a));
            self.reach = [self.reach[1], reach];
            if reach >= self.rest {
                self.state = TetherState::Taut;
                self.lay_straight(a, b);
            } else {
                if self.state == TetherState::Taut {
                    // A taut rope that goes slack hangs in its arc at once; projecting its straight
                    // nodes instead buckles them and throws them.
                    self.lay_sag(a, b);
                }
                self.state = TetherState::Slack;
                self.satisfy(true);
            }
        } else {
            self.satisfy(false);
        }
        self.settle(&before);
    }

    /// The slack rope hung in its arc between the ends, with no velocity: the nodes start where
    /// the rope's length puts them.
    fn lay_sag(&mut self, a: [f32; 3], b: [f32; 3]) {
        let last = self.curr.len() - 1;
        let arc = sag_arc(a, b, self.rest, last + 1);
        for (i, at) in arc.into_iter().enumerate() {
            self.prev[i] = at;
            self.curr[i] = at;
        }
        self.prev[0] = a;
        self.curr[0] = a;
        self.prev[last] = b;
        self.curr[last] = b;
    }

    /// The taut rope: every node on the line between the ends, evenly spaced. Interior nodes
    /// keep no velocity, so the rope sags from rest when it slackens again.
    fn lay_straight(&mut self, a: [f32; 3], b: [f32; 3]) {
        let last = self.curr.len() - 1;
        for i in 1..last {
            let at = lerp(a, b, share(i, last));
            self.prev[i] = at;
            self.curr[i] = at;
        }
    }

    /// Gauss-Seidel sweeps over the segments, forward then back, each moving both nodes half the
    /// error (a pinned end does not move).
    fn satisfy(&mut self, pinned: bool) {
        let last = self.curr.len() - 1;
        let seg = self.rest / share(last, 1);
        for _ in 0..self.sweeps {
            for k in 0..2 * last {
                let i = if k < last { k } else { 2 * last - 1 - k };
                let d = sub(self.curr[i + 1], self.curr[i]);
                let dist = len(d);
                if dist < 1.0e-6 {
                    continue;
                }
                let fix = d.map(|c| c * (dist - seg) / dist);
                let (mine, theirs) = match (pinned, i == 0, i + 1 == last) {
                    (true, true, _) => (0.0, 1.0),
                    (true, _, true) => (1.0, 0.0),
                    _ => (0.5, 0.5),
                };
                for (c, f) in fix.iter().enumerate() {
                    self.curr[i][c] += f * mine;
                    self.curr[i + 1][c] -= f * theirs;
                }
            }
            for node in &mut self.curr {
                node[1] = node[1].max(FLOOR_M);
            }
        }
    }

    /// Counts a still tick, wakes on motion, and repairs a chain that went non-finite.
    fn settle(&mut self, before: &[[f32; 3]]) {
        let finite = self.curr.iter().flatten().all(|c| c.is_finite());
        if !finite {
            let (a, b) = self.ends.unwrap_or((before[0], before[before.len() - 1]));
            let rebuilt = Self::new(self.id, a, b, self.rest, self.curr.len(), self.sweeps);
            self.prev = rebuilt.prev;
            self.curr = rebuilt.curr;
            self.quiet = 0;
            return;
        }
        let moved = self
            .curr
            .iter()
            .zip(before)
            .map(|(c, b)| len(sub(*c, *b)))
            .fold(0.0, f32::max);
        if moved < STILL_M {
            self.quiet += 1;
            if self.asleep() {
                self.prev.clone_from(&self.curr);
            }
        } else {
            self.quiet = 0;
        }
    }
}

/// A rope with a length above zero and a few nodes to hold it.
pub(crate) fn check_rest(rest_m: f32) -> Result<(), Error> {
    use crate::error::Field;
    if !rest_m.is_finite() {
        Err(Error::NonFinite(Field::Extent))
    } else if !(0.1..=200.0).contains(&rest_m) {
        Err(Error::OutOfRange(Field::Extent))
    } else {
        Ok(())
    }
}
