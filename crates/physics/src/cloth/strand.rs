//! strand: Q16 hair and rope, the same idea as the sheet in fixed point.
//!
//! A chain of particles, the root pinned to a scalp or both ends pinned between two points,
//! tension-only links, a sagging hang and one-way colliders that push the strand out of a skull
//! or a post. Presentation only: never hashed.

use super::collide::cloth_meets;
use super::convert::{from_fixed, to_fixed};
use super::{ClothCollider, MAX_ITERATIONS};
use engine_core::fixed::Fixed;
use engine_core::Fnv64;

/// Largest particle count of one hair lock or rope.
pub const MAX_STRAND_NODES: usize = 64;

/// Downward pull added to a free strand particle each tick, Q16 meters: 9.81 m/s squared at
/// 60 Hz is 179 / 65536 m.
const STRAND_GRAVITY_Q16: i64 = 179;
/// Fraction of the last tick's motion a free strand particle keeps, Q16: 99% for hair and rope.
const STRAND_KEEP_Q16: i64 = 64_881;
/// Largest gravity a caller may set, Q16 meters per tick squared (40 m/s squared).
const MAX_STRAND_GRAVITY_Q16: i64 = 728;
/// Largest push a caller may set per axis, Q16 meters per tick squared (one meter).
const MAX_STRAND_PUSH_Q16: i64 = 65_536;
/// Largest per-tick displacement of a free strand particle, Q16 meters (half a meter).
const MAX_STRAND_STEP_Q16: i64 = 32_768;
/// Largest strand length, meters, in Q16; matches the cloth sheet cap.
const MAX_STRAND_LEN_Q16: i64 = 20 * 65_536;
/// Largest absolute coordinate of a strand pin, Q16 meters; matches `WORLD_LIMIT`.
const STRAND_WORLD_LIMIT_Q16: i64 = 1000 * 65_536;
/// Percent of a rope's span added as slack between its pins.
const ROPE_SLACK_PERCENT: i64 = 1;

/// Typed errors for strand setup and pins. The stepper cannot fail.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StrandError {
    /// A strand has `2..=MAX_STRAND_NODES` particles.
    BadNodes,
    /// A named value is not in range.
    BadParam(&'static str),
    /// Particle index is out of range.
    BadIndex(usize),
}

impl core::fmt::Display for StrandError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            StrandError::BadNodes => {
                write!(f, "strand: particle count must be 2..={MAX_STRAND_NODES}")
            }
            StrandError::BadParam(what) => write!(f, "strand: bad {what}"),
            StrandError::BadIndex(i) => write!(f, "strand: particle {i} out of range"),
        }
    }
}

impl std::error::Error for StrandError {}

/// Tune of one strand, in Q16 per-tick units: the step takes no time argument, so a caller's
/// seconds become tick-squared displacements here (60 Hz).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StrandParams {
    /// Downward displacement added to a free particle each tick, Q16 meters, `0..=728`.
    pub gravity_q16: i64,
    /// Fraction of the last tick's motion kept, Q16, `0..=65_536` (zero stops on a dime).
    pub keep_q16: i64,
    /// Wind and drag push per tick, Q16 meters per axis, within `+-65_536`.
    pub push_q16: [i64; 3],
    /// Link passes per tick, `1..=MAX_ITERATIONS`.
    pub iterations: usize,
}

impl StrandParams {
    /// Hair and rope tune: full gravity, 99% of the motion kept, no wind, six link passes.
    pub const fn calm() -> Self {
        Self {
            gravity_q16: STRAND_GRAVITY_Q16,
            keep_q16: STRAND_KEEP_Q16,
            push_q16: [0; 3],
            iterations: 6,
        }
    }

    /// Range-check every field; a failing tune is never applied.
    pub fn validate(&self) -> Result<(), StrandError> {
        let in_range = |v: i64, lo: i64, hi: i64| (lo..=hi).contains(&v);
        if !in_range(self.gravity_q16, 0, MAX_STRAND_GRAVITY_Q16) {
            return Err(StrandError::BadParam("gravity"));
        }
        if !in_range(self.keep_q16, 0, 65_536) {
            return Err(StrandError::BadParam("keep"));
        }
        if self
            .push_q16
            .iter()
            .any(|p| !in_range(*p, -MAX_STRAND_PUSH_Q16, MAX_STRAND_PUSH_Q16))
        {
            return Err(StrandError::BadParam("push"));
        }
        if !(1..=MAX_ITERATIONS).contains(&self.iterations) {
            return Err(StrandError::BadParam("iterations"));
        }
        Ok(())
    }
}

/// A presentation hair lock or rope: a chain of Q16 Verlet particles.
///
/// The first particle starts pinned to a root; [`pin`](Strand::pin) pins more (both ends of a
/// rope) and [`unpin`](Strand::unpin) frees them again. Links resist stretching only, so a strand
/// sags and swings like a lock of hair or a slack rope. Presentation only: floats leave only
/// through [`positions`](Strand::positions), nothing here is hashed or snapshotted, and no sim
/// state reads it. [`digest`](Strand::digest) is for tests and tapes.
#[derive(Clone, Debug)]
pub struct Strand {
    pos: Vec<[Fixed; 3]>,
    prev: Vec<[Fixed; 3]>,
    anchor: Vec<[Fixed; 3]>,
    pinned: Vec<bool>,
    pub(in crate::cloth) rest: Fixed,
    params: StrandParams,
    ticks: u64,
    view: Vec<[f32; 3]>,
}

impl Strand {
    /// A straight chain of `nodes` particles hanging down -Y from `root`, its root pinned.
    /// `length` is the whole chain, meters, split evenly between the links.
    pub fn new(
        nodes: usize,
        length: Fixed,
        root: [Fixed; 3],
        params: StrandParams,
    ) -> Result<Self, StrandError> {
        if !(2..=MAX_STRAND_NODES).contains(&nodes) {
            return Err(StrandError::BadNodes);
        }
        params.validate()?;
        check_strand_point(root, "root")?;
        let len = length.to_bits();
        if !(1..=MAX_STRAND_LEN_Q16).contains(&len) {
            return Err(StrandError::BadParam("length"));
        }
        let links = i64::try_from(nodes - 1).unwrap_or(i64::MAX);
        if len / links < 1 {
            return Err(StrandError::BadParam("length"));
        }
        let mut strand = Self {
            pos: Vec::with_capacity(nodes),
            prev: Vec::with_capacity(nodes),
            anchor: Vec::with_capacity(nodes),
            pinned: Vec::with_capacity(nodes),
            rest: Fixed::from_bits(len / links),
            params,
            ticks: 0,
            view: vec![[0.0; 3]; nodes],
        };
        strand.layout(root);
        Ok(strand)
    }

    /// A hair lock: eight particles over `length` meters, the root pinned at `root`.
    pub fn hair(root: [Fixed; 3], length: Fixed) -> Result<Self, StrandError> {
        Self::new(8, length, root, StrandParams::calm())
    }

    /// A rope of `nodes` particles hung between `a` and `b`, both ends pinned, with one percent
    /// of slack over the span so gravity bows it.
    pub fn rope(a: [Fixed; 3], b: [Fixed; 3], nodes: usize) -> Result<Self, StrandError> {
        if !(2..=MAX_STRAND_NODES).contains(&nodes) {
            return Err(StrandError::BadNodes);
        }
        check_strand_point(a, "root")?;
        check_strand_point(b, "pin")?;
        let span = dist_q16(a, b);
        if span <= 0 {
            return Err(StrandError::BadParam("length"));
        }
        let length = span + span / 100 * ROPE_SLACK_PERCENT;
        if length > MAX_STRAND_LEN_Q16 {
            return Err(StrandError::BadParam("length"));
        }
        let mut rope = Self::new(nodes, Fixed::from_bits(length), a, StrandParams::calm())?;
        rope.layout_between(a, b);
        Ok(rope)
    }

    /// Lay the chain straight down from `root`, root pinned, every other particle free.
    fn layout(&mut self, root: [Fixed; 3]) {
        let nodes = self.view.len();
        self.pos.clear();
        self.prev.clear();
        self.anchor.clear();
        self.pinned.clear();
        for k in 0..nodes {
            let p = [
                root[0],
                root[1] - self.rest * Fixed::from_int(i64::try_from(k).unwrap_or(0)),
                root[2],
            ];
            self.pos.push(p);
            self.prev.push(p);
            self.anchor.push(root);
            self.pinned.push(k == 0);
        }
        self.ticks = 0;
        self.refresh_view();
    }

    /// Lay the chain along the segment `a` to `b`, both ends pinned.
    fn layout_between(&mut self, a: [Fixed; 3], b: [Fixed; 3]) {
        let nodes = self.view.len();
        let last = i64::try_from(nodes - 1).unwrap_or(i64::MAX);
        self.pos.clear();
        self.prev.clear();
        self.anchor.clear();
        self.pinned.clear();
        for k in 0..nodes {
            let at = Fixed::from_int(i64::try_from(k).unwrap_or(0)) / Fixed::from_int(last);
            let p = [
                a[0] + mul_sym(b[0] - a[0], at),
                a[1] + mul_sym(b[1] - a[1], at),
                a[2] + mul_sym(b[2] - a[2], at),
            ];
            self.pos.push(p);
            self.prev.push(p);
            self.anchor.push(if k + 1 == nodes { b } else { p });
            self.pinned.push(k == 0 || k + 1 == nodes);
        }
        self.ticks = 0;
        self.refresh_view();
    }

    /// Pin particle `particle` to `at`, or move its pin. A pinned particle lands there at once.
    pub fn pin(&mut self, particle: usize, at: [Fixed; 3]) -> Result<(), StrandError> {
        if particle >= self.pos.len() {
            return Err(StrandError::BadIndex(particle));
        }
        check_strand_point(at, "pin")?;
        self.anchor[particle] = at;
        self.pinned[particle] = true;
        self.pos[particle] = at;
        self.prev[particle] = at;
        self.refresh_view();
        Ok(())
    }

    /// Release a particle back to free motion; it keeps its place and loses its velocity.
    pub fn unpin(&mut self, particle: usize) -> Result<(), StrandError> {
        let Some(pinned) = self.pinned.get_mut(particle) else {
            return Err(StrandError::BadIndex(particle));
        };
        if *pinned {
            *pinned = false;
            self.prev[particle] = self.pos[particle];
        }
        Ok(())
    }

    /// Replace the tune; a bad one changes nothing.
    pub fn set_params(&mut self, params: StrandParams) -> Result<(), StrandError> {
        params.validate()?;
        self.params = params;
        Ok(())
    }

    /// Replace the wind push (a gust); a bad one changes nothing.
    pub fn set_push(&mut self, push: [i64; 3]) -> Result<(), StrandError> {
        let params = StrandParams {
            push_q16: push,
            ..self.params
        };
        self.set_params(params)
    }

    /// Particle count.
    pub fn particle_count(&self) -> usize {
        self.pos.len()
    }

    /// Link count (`particles - 1`).
    pub fn link_count(&self) -> usize {
        self.pos.len().saturating_sub(1)
    }

    /// True when the particle is pinned; false for an index out of range.
    pub fn is_pinned(&self, particle: usize) -> bool {
        self.pinned.get(particle).copied().unwrap_or(false)
    }

    /// Ticks stepped since construction or the last layout.
    pub fn ticks(&self) -> u64 {
        self.ticks
    }

    /// Current tune.
    pub fn params(&self) -> StrandParams {
        self.params
    }

    /// Particle positions in meters for the renderer, root first, converted from Q16.
    pub fn positions(&self) -> &[[f32; 3]] {
        &self.view
    }

    /// FNV-1a over every particle's Q16 position bits: the replay check for tests and tapes.
    /// Presentation only; the match hash never reads it.
    pub fn digest(&self) -> u64 {
        let mut h = Fnv64::new();
        for p in &self.pos {
            for v in p {
                h.write_i64(v.to_bits());
            }
        }
        h.finish()
    }

    /// Advance one fixed 1/60 s tick: free particles keep 99% of their motion and fall, then the
    /// links pull taut pairs back toward rest, then the colliders push the strand out of them.
    /// Never allocates.
    pub fn step(&mut self, colliders: &[ClothCollider]) {
        self.integrate();
        for _ in 0..self.params.iterations {
            self.solve_links();
        }
        self.push_out(colliders);
        self.clamp_world();
        self.refresh_view();
        self.ticks += 1;
    }

    /// Push every free particle out of the colliders the strand may meet: the hair lock off the
    /// skull, the rope off the wall. The strand is Q16, so the push goes through the float
    /// metres and back.
    fn push_out(&mut self, colliders: &[ClothCollider]) {
        for collider in colliders {
            if !cloth_meets(collider) {
                continue;
            }
            for i in 0..self.pos.len() {
                if self.pinned[i] {
                    continue;
                }
                let p = [
                    from_fixed(self.pos[i][0]),
                    from_fixed(self.pos[i][1]),
                    from_fixed(self.pos[i][2]),
                ];
                let out = collider.push_out(p);
                if out == p {
                    continue;
                }
                self.pos[i] = [to_fixed(out[0]), to_fixed(out[1]), to_fixed(out[2])];
            }
        }
    }

    /// Q16 position Verlet with gravity and wind push; pins land on their anchors exactly. A
    /// free particle's per-tick move is capped so a yanked scalp can never fling the lock.
    fn integrate(&mut self) {
        let keep = Fixed::from_bits(self.params.keep_q16);
        let push = [
            Fixed::from_bits(self.params.push_q16[0]),
            Fixed::from_bits(self.params.push_q16[1] - self.params.gravity_q16),
            Fixed::from_bits(self.params.push_q16[2]),
        ];
        let cap = Fixed::from_bits(MAX_STRAND_STEP_Q16);
        let cap2 = cap * cap;
        let particles = self
            .pos
            .iter_mut()
            .zip(&mut self.prev)
            .zip(&self.anchor)
            .zip(&self.pinned);
        for (((pos, prev), anchor), pinned) in particles {
            if *pinned {
                *pos = *anchor;
                *prev = *anchor;
                continue;
            }
            let cur = *pos;
            let mut moved = [
                mul_sym(cur[0] - prev[0], keep),
                mul_sym(cur[1] - prev[1], keep),
                mul_sym(cur[2] - prev[2], keep),
            ];
            let moved2 = moved[0] * moved[0] + moved[1] * moved[1] + moved[2] * moved[2];
            if moved2 > cap2 {
                let len = moved2.sqrt();
                if len > Fixed::ZERO {
                    let k = cap / len;
                    moved = [
                        mul_sym(moved[0], k),
                        mul_sym(moved[1], k),
                        mul_sym(moved[2], k),
                    ];
                }
            }
            *prev = cur;
            *pos = [
                cur[0] + moved[0] + push[0],
                cur[1] + moved[1] + push[1],
                cur[2] + moved[2] + push[2],
            ];
        }
    }

    /// One pass over the links in index order. A link longer than its rest length pulls its ends
    /// together, split between the free ends (a pinned end never moves); slack links do nothing.
    fn solve_links(&mut self) {
        let rest = self.rest;
        for i in 0..self.pos.len() - 1 {
            let (pa, pb) = (self.pos[i], self.pos[i + 1]);
            let d = [pb[0] - pa[0], pb[1] - pa[1], pb[2] - pa[2]];
            let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
            if len <= rest {
                continue;
            }
            let share = (len - rest) / len;
            let (ka, kb) = match (self.pinned[i], self.pinned[i + 1]) {
                (true, true) => continue,
                (true, false) => (Fixed::ZERO, share),
                (false, true) => (share, Fixed::ZERO),
                (false, false) => (share * Fixed::HALF, share * Fixed::HALF),
            };
            self.pos[i] = [
                pa[0] + mul_sym(d[0], ka),
                pa[1] + mul_sym(d[1], ka),
                pa[2] + mul_sym(d[2], ka),
            ];
            self.pos[i + 1] = [
                pb[0] - mul_sym(d[0], kb),
                pb[1] - mul_sym(d[1], kb),
                pb[2] - mul_sym(d[2], kb),
            ];
        }
    }

    /// Hard stop at the world limit, the same +-1000 m the cloth pins obey.
    fn clamp_world(&mut self) {
        let limit = Fixed::from_bits(STRAND_WORLD_LIMIT_Q16);
        for p in &mut self.pos {
            for v in p {
                *v = (*v).clamp(-limit, limit);
            }
        }
    }

    /// Copy the Q16 state to the f32 view the renderer reads.
    fn refresh_view(&mut self) {
        for (out, p) in self.view.iter_mut().zip(&self.pos) {
            *out = [from_fixed(p[0]), from_fixed(p[1]), from_fixed(p[2])];
        }
    }
}

/// Q16 product rounded toward zero. `Fixed`'s `*` floors, so a product with a negative operand
/// lands one step low on one side of a mirror and not on the other; this keeps a strand's mirror
/// image exact bit for bit.
fn mul_sym(a: Fixed, b: Fixed) -> Fixed {
    let p = i128::from(a.to_bits()) * i128::from(b.to_bits());
    let m = i64::try_from(p.unsigned_abs() >> 16).unwrap_or(i64::MAX);
    Fixed::from_bits(if p < 0 { -m } else { m })
}

/// Squared length of `b - a` in Q32, square rooted back to Q16 meters.
fn dist_q16(a: [Fixed; 3], b: [Fixed; 3]) -> i64 {
    let d = [
        i128::from((b[0] - a[0]).to_bits()),
        i128::from((b[1] - a[1]).to_bits()),
        i128::from((b[2] - a[2]).to_bits()),
    ];
    let sq: i128 = d.iter().map(|v| v * v).sum();
    let n = u128::try_from(sq).unwrap_or(0);
    i64::try_from(engine_core::fixed::isqrt(n)).unwrap_or(i64::MAX)
}

/// A strand pin must be finite and inside the world.
fn check_strand_point(p: [Fixed; 3], what: &'static str) -> Result<(), StrandError> {
    if p.iter()
        .all(|v| (-STRAND_WORLD_LIMIT_Q16..=STRAND_WORLD_LIMIT_Q16).contains(&v.to_bits()))
    {
        Ok(())
    } else {
        Err(StrandError::BadParam(what))
    }
}
