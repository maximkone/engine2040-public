//! eqs: integer Environment Query System for cover/flank/ambush.
//!
//! Consumer: the tactics brain feeds sensor positions in, reads the ranked
//! spot out, then aims the `DepthBrain` wish at it via [`Tactics`].
//! Owner: [`EqQuery`] owns ranking; generators own candidates; tests own
//! scores. Units: millimetres, scores `0..=1000`, ticks. Determinism:
//! integer `+ - * /` and `isqrt` only, index order, no floats, no clock.
//! Errors: [`EqError`]; degenerate queries refuse, empty ones rank empty.
//! Budget: at most [`MAX_POINTS`] points per query on the perception clock.
//! PORT(maximkone/flax-mcp@a26f960:plugins/gameplay/GameSide/Game.Shared.AI/EQS/Eq*.cs),
//! MIT; clean-room integer rewrite (no float `sqrt`/`pow`/`sin` in the tick).

use engine_input::fighter::FACING_RANGE;
use engine_input::{Buttons, ParticipantId, ParticipantInput};

pub use crate::eqs_slice::{CompositeGenerator, SensorArrayGenerator, SlicedRunner};

/// Top score: tests multiply factors in this scale (`0..=1000`).
pub const SCORE_MAX: i32 = 1000;
/// Cap on candidates per query; the caller owns the run buffer, so a
/// query never grows memory behind the brain's back.
pub const MAX_POINTS: usize = 256;
/// Ring generator cap: one baked 16-direction table, no runtime trig.
pub const RING_MAX_POINTS: usize = 16;
/// Highest accepted test exponent (repeated multiply, so cost stays flat).
pub const MAX_POWER: u32 = 8;

/// Failures for degenerate EQS input. Invalid external input returns
/// these; the simulation never panics on them.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EqError {
    /// Ring count is zero; nothing would be generated.
    ZeroCount,
    /// A generator would emit more than [`MAX_POINTS`]; count carried.
    TooManyPoints {
        /// Rejected candidate count.
        got: usize,
    },
    /// Grid spacing is zero or negative millimetres; culprit carried.
    BadSpacing {
        /// Rejected spacing in millimetres.
        got: i32,
    },
    /// Distance budget is zero or negative millimetres; culprit carried.
    BadBudget {
        /// Rejected budget in millimetres.
        got: i32,
    },
    /// Cover standoff is zero or negative millimetres; culprit carried.
    BadStandoff {
        /// Rejected standoff in millimetres.
        got: i32,
    },
    /// Runner interval is zero ticks; nothing would ever run.
    BadInterval,
}

impl core::fmt::Display for EqError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            EqError::ZeroCount => write!(f, "eqs: ring count is zero"),
            EqError::TooManyPoints { got } => {
                write!(f, "eqs: {got} points exceeds the cap of {MAX_POINTS}")
            }
            EqError::BadSpacing { got } => write!(f, "eqs: bad grid spacing {got} mm"),
            EqError::BadBudget { got } => write!(f, "eqs: bad distance budget {got} mm"),
            EqError::BadStandoff { got } => write!(f, "eqs: bad cover standoff {got} mm"),
            EqError::BadInterval => write!(f, "eqs: runner interval is zero ticks"),
        }
    }
}

impl std::error::Error for EqError {}

/// Integer 3D position in millimetres (the donor `EqVector` in int form).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Vec3Mm {
    /// X in millimetres.
    pub x: i32,
    /// Y (height) in millimetres.
    pub y: i32,
    /// Z in millimetres.
    pub z: i32,
}

impl Vec3Mm {
    /// New integer position.
    pub const fn new(x: i32, y: i32, z: i32) -> Self {
        Self { x, y, z }
    }

    /// Origin.
    pub const ZERO: Self = Self::new(0, 0, 0);

    /// Squared ground-plane length as `u128`, so arena-scale and
    /// `i32`-extreme inputs cannot overflow.
    fn len2_xz(dx: i64, dz: i64) -> u128 {
        let ax = u128::from(dx.unsigned_abs());
        let az = u128::from(dz.unsigned_abs());
        ax * ax + az * az
    }

    /// Ground-plane length in millimetres, rounded down.
    fn len_xz(dx: i64, dz: i64) -> i64 {
        Self::len2_xz(dx, dz).isqrt() as i64
    }

    /// Full 3D distance in millimetres, rounded down. Used by the
    /// distance tests and the travel budget, exactly like the donor.
    pub fn dist_mm(a: Self, b: Self) -> i32 {
        let dx = u128::from((i64::from(a.x) - i64::from(b.x)).unsigned_abs());
        let dy = u128::from((i64::from(a.y) - i64::from(b.y)).unsigned_abs());
        let dz = u128::from((i64::from(a.z) - i64::from(b.z)).unsigned_abs());
        let sum = dx * dx + dy * dy + dz * dz;
        sum.isqrt().min(i32::MAX as u128) as i32
    }
}

/// Query context: who asks and who matters (the donor `EqContext`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct EqContext {
    /// Querier position (budget origin, ring/grid centre).
    pub querier: Vec3Mm,
    /// Target/threat position (cover side, flank facing origin).
    pub target: Vec3Mm,
}

impl EqContext {
    /// New context.
    pub const fn new(querier: Vec3Mm, target: Vec3Mm) -> Self {
        Self { querier, target }
    }
}

/// One scored candidate (the donor `EqPoint`; score in `0..=1000`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct EqPoint {
    /// Candidate position in millimetres.
    pub pos: Vec3Mm,
    /// Score in `0..=1000`; tests only multiply it down.
    pub score: i32,
}

impl EqPoint {
    /// New candidate at full score.
    pub const fn new(pos: Vec3Mm) -> Self {
        Self {
            pos,
            score: SCORE_MAX,
        }
    }
}

/// Clamp an exponent into `1..=MAX_POWER` (power 0 would no-op every
/// test, higher powers only burn ticks for the same ranking shape).
fn clamp_power(power: u32) -> u32 {
    power.clamp(1, MAX_POWER)
}

/// `base^exp` in permille (`base` in `0..=1000`): repeated multiply,
/// so no float `pow` ever reaches the tick.
fn pow_permille(base: i32, exp: u32) -> i32 {
    let mut acc: i64 = i64::from(SCORE_MAX);
    for _ in 0..exp {
        acc = acc * i64::from(base) / i64::from(SCORE_MAX);
    }
    acc as i32
}

/// Multiply a score by a permille factor, staying in `0..=1000`.
fn mul_score(score: i32, factor: i32) -> i32 {
    ((i64::from(score) * i64::from(factor.clamp(0, SCORE_MAX))) / i64::from(SCORE_MAX)) as i32
}

/// Normalize a ground-plane vector to permille length; `None` when it
/// has no length (callers keep the old score, exactly like the donor).
fn normalize_xz(dx: i64, dz: i64) -> Option<(i32, i32)> {
    let n = Vec3Mm::len_xz(dx, dz);
    if n <= 0 {
        return None;
    }
    Some(((dx * 1000 / n) as i32, (dz * 1000 / n) as i32))
}

/// Dot product of two permille ground-plane vectors, in `-1000..=1000`.
fn dot_permille(ax: i32, az: i32, bx: i32, bz: i32) -> i32 {
    (((i64::from(ax) * i64::from(bx) + i64::from(az) * i64::from(bz)) / 1000) as i32)
        .clamp(-1000, 1000)
}

/// Baked ring directions in permille (16 evenly spaced headings, so the
/// ring generator needs no runtime trig; values fixed, never computed).
const RING_DIRS: [(i32, i32); RING_MAX_POINTS] = [
    (1000, 0),
    (924, 383),
    (707, 707),
    (383, 924),
    (0, 1000),
    (-383, 924),
    (-707, 707),
    (-924, 383),
    (-1000, 0),
    (-924, -383),
    (-707, -707),
    (-383, -924),
    (0, -1000),
    (383, -924),
    (707, -707),
    (924, -383),
];

/// Ring of evenly spaced points around the querier (donor `EqRingGenerator`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct RingGenerator {
    radius_mm: i32,
    count: usize,
}

impl RingGenerator {
    /// New ring; `count` in `1..=RING_MAX_POINTS`, radius clamped to
    /// non-negative (a zero radius stacks every point on the querier).
    pub fn new(radius_mm: i32, count: usize) -> Result<Self, EqError> {
        if count == 0 {
            return Err(EqError::ZeroCount);
        }
        if count > RING_MAX_POINTS {
            return Err(EqError::TooManyPoints { got: count });
        }
        Ok(Self {
            radius_mm: radius_mm.max(0),
            count,
        })
    }

    /// Radius in millimetres.
    pub const fn radius_mm(self) -> i32 {
        self.radius_mm
    }

    /// Point count.
    pub const fn count(self) -> usize {
        self.count
    }

    fn generate_into(&self, ctx: &EqContext, out: &mut Vec<EqPoint>) -> Result<(), EqError> {
        for i in 0..self.count {
            // Evenly spread subset of the baked table; strictly
            // increasing for count <= 16, so no two points coincide.
            let (dx, dz) = RING_DIRS[i * RING_MAX_POINTS / self.count];
            if out.len() >= MAX_POINTS {
                return Err(EqError::TooManyPoints { got: out.len() + 1 });
            }
            out.push(EqPoint::new(Vec3Mm::new(
                ctx.querier
                    .x
                    .saturating_add(dx.saturating_mul(self.radius_mm) / 1000),
                ctx.querier.y,
                ctx.querier
                    .z
                    .saturating_add(dz.saturating_mul(self.radius_mm) / 1000),
            )));
        }
        Ok(())
    }
}

/// Grid of points in a box around the querier (donor `EqGridGenerator`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct GridGenerator {
    half_extent_mm: i32,
    spacing_mm: i32,
}

impl GridGenerator {
    /// New grid; spacing must clear zero, extent clamps to non-negative.
    pub fn new(half_extent_mm: i32, spacing_mm: i32) -> Result<Self, EqError> {
        if spacing_mm <= 0 {
            return Err(EqError::BadSpacing { got: spacing_mm });
        }
        Ok(Self {
            half_extent_mm: half_extent_mm.max(0),
            spacing_mm,
        })
    }

    fn generate_into(&self, ctx: &EqContext, out: &mut Vec<EqPoint>) -> Result<(), EqError> {
        // X outer, Z inner: stable index order, so ties rank by position.
        let mut x = -self.half_extent_mm;
        loop {
            let mut z = -self.half_extent_mm;
            loop {
                if out.len() >= MAX_POINTS {
                    return Err(EqError::TooManyPoints { got: out.len() + 1 });
                }
                out.push(EqPoint::new(Vec3Mm::new(
                    ctx.querier.x.saturating_add(x),
                    ctx.querier.y,
                    ctx.querier.z.saturating_add(z),
                )));
                if z >= self.half_extent_mm {
                    break;
                }
                // Snap to the far edge so uneven spacings still end exact.
                z = z.saturating_add(self.spacing_mm).min(self.half_extent_mm);
            }
            if x >= self.half_extent_mm {
                break;
            }
            x = x.saturating_add(self.spacing_mm).min(self.half_extent_mm);
        }
        Ok(())
    }
}

/// Emits caller-supplied positions verbatim (donor `EqPointListGenerator`):
/// cover spots, ambush nests, patrol markers. No geometry of its own.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct PointListGenerator<'a> {
    points: &'a [Vec3Mm],
}

impl<'a> PointListGenerator<'a> {
    /// New list generator over borrowed spots.
    pub const fn new(points: &'a [Vec3Mm]) -> Self {
        Self { points }
    }

    fn generate_into(&self, _ctx: &EqContext, out: &mut Vec<EqPoint>) -> Result<(), EqError> {
        for p in self.points {
            if out.len() >= MAX_POINTS {
                return Err(EqError::TooManyPoints { got: out.len() + 1 });
            }
            out.push(EqPoint::new(*p));
        }
        Ok(())
    }
}

/// Cover generator (donor `EqCoverSpotGenerator`): for each cover object
/// emits one standing spot on the side away from the threat, `C = P +
/// standoff * normalize(P - T)` on the ground plane, keeping the cover's
/// height. Pillars whose threat distance does not clear the standoff are
/// skipped (the spot would sit on the threat).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct CoverSpotGenerator<'a> {
    covers: &'a [Vec3Mm],
    standoff_mm: i32,
    min_clear_mm: i32,
}

impl<'a> CoverSpotGenerator<'a> {
    /// New cover generator; standoff must clear zero, clearance clamps
    /// to non-negative.
    pub fn new(covers: &'a [Vec3Mm], standoff_mm: i32, min_clear_mm: i32) -> Result<Self, EqError> {
        if standoff_mm <= 0 {
            return Err(EqError::BadStandoff { got: standoff_mm });
        }
        Ok(Self {
            covers,
            standoff_mm,
            min_clear_mm: min_clear_mm.max(0),
        })
    }

    fn generate_into(&self, ctx: &EqContext, out: &mut Vec<EqPoint>) -> Result<(), EqError> {
        for c in self.covers {
            let dx = i64::from(c.x) - i64::from(ctx.target.x);
            let dz = i64::from(c.z) - i64::from(ctx.target.z);
            let d = Vec3Mm::len_xz(dx, dz);
            if d <= i64::from(self.standoff_mm) + i64::from(self.min_clear_mm) {
                continue;
            }
            let Some((ux, uz)) = normalize_xz(dx, dz) else {
                continue;
            };
            if out.len() >= MAX_POINTS {
                return Err(EqError::TooManyPoints { got: out.len() + 1 });
            }
            out.push(EqPoint::new(Vec3Mm::new(
                c.x.saturating_add((i64::from(ux) * i64::from(self.standoff_mm) / 1000) as i32),
                c.y,
                c.z.saturating_add((i64::from(uz) * i64::from(self.standoff_mm) / 1000) as i32),
            )));
        }
        Ok(())
    }
}

/// Candidate source for a query (the donor `IEqGenerator` seam, as a
/// plain enum so the tick pays no `dyn` dispatch per point).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Generator<'a> {
    /// Ring around the querier.
    Ring(RingGenerator),
    /// Grid box around the querier.
    Grid(GridGenerator),
    /// Caller-supplied spots verbatim.
    List(PointListGenerator<'a>),
    /// Standing spots behind cover objects.
    Cover(CoverSpotGenerator<'a>),
}

impl<'a> Generator<'a> {
    fn generate_into(&self, ctx: &EqContext, out: &mut Vec<EqPoint>) -> Result<(), EqError> {
        match self {
            Generator::Ring(g) => g.generate_into(ctx, out),
            Generator::Grid(g) => g.generate_into(ctx, out),
            Generator::List(g) => g.generate_into(ctx, out),
            Generator::Cover(g) => g.generate_into(ctx, out),
        }
    }

    /// Crate seam for [`crate::eqs_slice`]: shared generator body.
    pub(crate) fn generate_crate(
        &self,
        ctx: &EqContext,
        out: &mut Vec<EqPoint>,
    ) -> Result<(), EqError> {
        self.generate_into(ctx, out)
    }
}

/// Sightline provider (donor `IEqVisibilityProvider`): game code wraps a
/// real raycast, tests supply a stub. A cold-path trait, never the tick.
pub trait Visibility {
    /// True when `from` sees `to`.
    fn has_los(&self, from: Vec3Mm, to: Vec3Mm) -> bool;
}

/// Default provider (donor `EqAlwaysVisibleProvider`): everything is
/// visible; for headless tests and non-physical callers.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct AlwaysVisible;

impl Visibility for AlwaysVisible {
    fn has_los(&self, _from: Vec3Mm, _to: Vec3Mm) -> bool {
        true
    }
}

/// Near/far preference for [`Test::Distance`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Preference {
    /// Prefer points near the target.
    Near,
    /// Prefer points far from the target.
    Far,
}

/// Scoring tests (the donor `IEqTest` seam, as a plain enum so the tick
/// pays no `dyn` dispatch per point). Every test only multiplies scores
/// down; only [`Test::Sightline`] and [`Test::Budget`] veto to zero.
/// `Debug` is hand-written because the sightline variant borrows a trait
/// object; a shared borrow is `Copy`, so `Clone`/`Copy` derive.
#[derive(Clone, Copy)]
pub enum Test<'a> {
    /// Score by straight-line distance to the target (donor `EqDistanceTest`).
    Distance {
        /// Near or far preference.
        prefer: Preference,
        /// Distance mapping to a zero/one score, in millimetres.
        max_mm: i32,
    },
    /// Score by ground-plane dot with a world direction (donor `EqDotProductTest`).
    Dot {
        /// World direction on the ground plane.
        dir: Vec3Mm,
        /// Exponent shaping the falloff.
        power: u32,
    },
    /// Zero points past a straight-line budget from the querier
    /// (donor `EqDistanceBudgetTest`).
    Budget {
        /// Travel budget in millimetres.
        max_mm: i32,
    },
    /// Score by how well the nearest cover sits between spot and threat
    /// (donor `EqCoverAlignmentTest`); ranking only, never a veto.
    CoverAlign {
        /// Cover objects to align against.
        covers: &'a [Vec3Mm],
        /// Exponent shaping the falloff.
        power: u32,
    },
    /// Zero points with no line of sight to the target (donor `EqSightlineTest`).
    Sightline {
        /// Raycast wrapper or test stub.
        vis: &'a dyn Visibility,
    },
    /// Prefer points behind the threat facing (donor `EqFlankTest`):
    /// 1000 directly behind, 500 at the side, 0 dead ahead, then powered.
    Flank {
        /// Threat facing on the ground plane.
        facing: Vec3Mm,
        /// Exponent shaping the falloff.
        power: u32,
    },
}

impl<'a> core::fmt::Debug for Test<'a> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Test::Distance { prefer, max_mm } => f
                .debug_struct("Distance")
                .field("prefer", prefer)
                .field("max_mm", max_mm)
                .finish(),
            Test::Dot { dir, power } => f
                .debug_struct("Dot")
                .field("dir", dir)
                .field("power", power)
                .finish(),
            Test::Budget { max_mm } => f.debug_struct("Budget").field("max_mm", max_mm).finish(),
            Test::CoverAlign { covers, power } => f
                .debug_struct("CoverAlign")
                .field("covers", covers)
                .field("power", power)
                .finish(),
            Test::Sightline { .. } => f.debug_struct("Sightline").finish_non_exhaustive(),
            Test::Flank { facing, power } => f
                .debug_struct("Flank")
                .field("facing", facing)
                .field("power", power)
                .finish(),
        }
    }
}

impl<'a> Test<'a> {
    /// Distance test; budget must clear zero.
    pub fn distance(prefer: Preference, max_mm: i32) -> Result<Self, EqError> {
        if max_mm <= 0 {
            return Err(EqError::BadBudget { got: max_mm });
        }
        Ok(Test::Distance { prefer, max_mm })
    }

    /// Dot-product test; zero directions are kept but skip scoring.
    pub const fn dot(dir: Vec3Mm, power: u32) -> Self {
        Test::Dot {
            dir,
            power: if power < 1 {
                1
            } else if power > MAX_POWER {
                MAX_POWER
            } else {
                power
            },
        }
    }

    /// Budget test; budget must clear zero.
    pub fn budget(max_mm: i32) -> Result<Self, EqError> {
        if max_mm <= 0 {
            return Err(EqError::BadBudget { got: max_mm });
        }
        Ok(Test::Budget { max_mm })
    }

    /// Cover-alignment test.
    pub const fn cover_align(covers: &'a [Vec3Mm], power: u32) -> Self {
        Test::CoverAlign {
            covers,
            power: if power < 1 {
                1
            } else if power > MAX_POWER {
                MAX_POWER
            } else {
                power
            },
        }
    }

    /// Sightline test over a provider.
    pub const fn sightline(vis: &'a dyn Visibility) -> Self {
        Test::Sightline { vis }
    }

    /// Flank test; a zero facing is kept but skips scoring (the ambush
    /// builder drops it entirely instead).
    pub const fn flank(facing: Vec3Mm, power: u32) -> Self {
        Test::Flank {
            facing,
            power: if power < 1 {
                1
            } else if power > MAX_POWER {
                MAX_POWER
            } else {
                power
            },
        }
    }

    fn score(&self, points: &mut [EqPoint], ctx: &EqContext) {
        match self {
            Test::Distance { prefer, max_mm } => {
                for p in points.iter_mut() {
                    let d = Vec3Mm::dist_mm(p.pos, ctx.target);
                    let t = ((i64::from(d) * 1000 / i64::from(*max_mm)) as i32).clamp(0, SCORE_MAX);
                    let s = match prefer {
                        Preference::Far => t,
                        Preference::Near => SCORE_MAX - t,
                    };
                    p.score = mul_score(p.score, s);
                }
            }
            Test::Dot { dir, power } => {
                let Some((wx, wz)) = normalize_xz(i64::from(dir.x), i64::from(dir.z)) else {
                    return;
                };
                let power = clamp_power(*power);
                for p in points.iter_mut() {
                    let dx = i64::from(p.pos.x) - i64::from(ctx.target.x);
                    let dz = i64::from(p.pos.z) - i64::from(ctx.target.z);
                    let Some((px, pz)) = normalize_xz(dx, dz) else {
                        continue;
                    };
                    let mapped = (dot_permille(px, pz, wx, wz) + 1000) / 2;
                    p.score = mul_score(p.score, pow_permille(mapped.clamp(0, SCORE_MAX), power));
                }
            }
            Test::Budget { max_mm } => {
                for p in points.iter_mut() {
                    if Vec3Mm::dist_mm(ctx.querier, p.pos) > *max_mm {
                        p.score = 0;
                    }
                }
            }
            Test::CoverAlign { covers, power } => {
                let power = clamp_power(*power);
                for p in points.iter_mut() {
                    let mut best: i32 = 0;
                    for c in *covers {
                        let cx = i64::from(c.x) - i64::from(p.pos.x);
                        let cz = i64::from(c.z) - i64::from(p.pos.z);
                        let tx = i64::from(ctx.target.x) - i64::from(p.pos.x);
                        let tz = i64::from(ctx.target.z) - i64::from(p.pos.z);
                        let cl = Vec3Mm::len_xz(cx, cz);
                        let tl = Vec3Mm::len_xz(tx, tz);
                        if cl <= 0 || tl <= 0 {
                            continue;
                        }
                        // Dot in permille via i128: arena-scale squares fit.
                        let num = (i128::from(cx) * i128::from(tx)
                            + i128::from(cz) * i128::from(tz))
                            * 1000;
                        let dot =
                            (num / (i128::from(cl) * i128::from(tl))).clamp(-1000, 1000) as i32;
                        if dot > best {
                            best = dot;
                        }
                    }
                    p.score = mul_score(p.score, pow_permille(best.clamp(0, SCORE_MAX), power));
                }
            }
            Test::Sightline { vis } => {
                for p in points.iter_mut() {
                    if !vis.has_los(p.pos, ctx.target) {
                        p.score = 0;
                    }
                }
            }
            Test::Flank { facing, power } => {
                let Some((fx, fz)) = normalize_xz(i64::from(facing.x), i64::from(facing.z)) else {
                    return;
                };
                let power = clamp_power(*power);
                for p in points.iter_mut() {
                    let dx = i64::from(p.pos.x) - i64::from(ctx.target.x);
                    let dz = i64::from(p.pos.z) - i64::from(ctx.target.z);
                    let Some((px, pz)) = normalize_xz(dx, dz) else {
                        continue;
                    };
                    // +1000 ahead, 0 at the side, -1000 behind, mapped up.
                    let behind = (SCORE_MAX - dot_permille(px, pz, fx, fz)) / 2;
                    p.score = mul_score(p.score, pow_permille(behind.clamp(0, SCORE_MAX), power));
                }
            }
        }
    }

    /// Crate seam for [`crate::eqs_slice`]: shared scoring body.
    pub(crate) fn score_crate(&self, points: &mut [EqPoint], ctx: &EqContext) {
        self.score(points, ctx);
    }
}

/// Composable spatial query (donor `EqQuery`): generator, then tests in
/// order, then a stable rank (insertion sort, ties keep generator order,
/// so the same input ranks byte-identical twice). The test list is owned
/// (built once per plan, like the donor); the run buffer stays caller
/// owned, so steady-state ticks allocate nothing after first growth.
#[derive(Clone, Debug)]
pub struct EqQuery<'a> {
    generator: Generator<'a>,
    tests: Vec<Test<'a>>,
}

impl<'a> EqQuery<'a> {
    /// New query; chain [`EqQuery::with_test`] to add scoring.
    pub fn new(generator: Generator<'a>) -> Self {
        Self {
            generator,
            tests: Vec::new(),
        }
    }

    /// Push one test, in scoring order.
    pub fn with_test(mut self, test: Test<'a>) -> Self {
        self.tests.push(test);
        self
    }

    /// Crate seam for sliced scoring: the owned generator, copied out.
    pub(crate) const fn generator_crate(&self) -> Generator<'a> {
        self.generator
    }

    /// Crate seam for sliced scoring: tests in scoring order.
    pub(crate) fn tests_crate(&self) -> &[Test<'a>] {
        &self.tests
    }

    /// Run into the caller's buffer: clear, generate, score, stable-rank.
    pub fn run_into(&self, ctx: &EqContext, out: &mut Vec<EqPoint>) -> Result<(), EqError> {
        out.clear();
        self.generator.generate_into(ctx, out)?;
        for t in &self.tests {
            t.score(out, ctx);
        }
        // Stable descending insertion sort, like the donor.
        for i in 1..out.len() {
            let item = out[i];
            let mut j = i;
            while j > 0 && out[j - 1].score < item.score {
                out[j] = out[j - 1];
                j -= 1;
            }
            out[j] = item;
        }
        Ok(())
    }

    /// Run into scratch and copy the winner out (`None` when empty).
    pub fn best_into(
        &self,
        ctx: &EqContext,
        scratch: &mut Vec<EqPoint>,
    ) -> Result<Option<EqPoint>, EqError> {
        self.run_into(ctx, scratch)?;
        Ok(scratch.first().copied())
    }
}

/// Tick-driven runner (donor `EqRunner`): executes its query every
/// `interval_ticks` ticks and remembers the ranked results. The tick
/// counter is integer, so scheduling replays exactly.
#[derive(Clone, Debug)]
pub struct EqRunner<'a> {
    query: EqQuery<'a>,
    interval_ticks: u32,
    tick_count: u64,
    last_best: Option<EqPoint>,
    last_results: Vec<EqPoint>,
}

impl<'a> EqRunner<'a> {
    /// New runner; interval must clear zero.
    pub fn new(query: EqQuery<'a>, interval_ticks: u32) -> Result<Self, EqError> {
        if interval_ticks == 0 {
            return Err(EqError::BadInterval);
        }
        Ok(Self {
            query,
            interval_ticks,
            tick_count: 0,
            last_best: None,
            last_results: Vec::new(),
        })
    }

    /// Advance one tick; runs the query when due and returns the winner.
    pub fn tick(&mut self, ctx: &EqContext) -> Result<Option<EqPoint>, EqError> {
        self.tick_count = self.tick_count.saturating_add(1);
        if !(self.tick_count - 1).is_multiple_of(u64::from(self.interval_ticks)) {
            return Ok(self.last_best);
        }
        self.query.run_into(ctx, &mut self.last_results)?;
        self.last_best = self.last_results.first().copied();
        Ok(self.last_best)
    }

    /// Run now regardless of the schedule.
    pub fn force(&mut self, ctx: &EqContext) -> Result<Option<EqPoint>, EqError> {
        self.tick_count = self.tick_count.saturating_add(1);
        self.query.run_into(ctx, &mut self.last_results)?;
        self.last_best = self.last_results.first().copied();
        Ok(self.last_best)
    }

    /// Forget results and restart the schedule.
    pub fn reset(&mut self) {
        self.tick_count = 0;
        self.last_best = None;
        self.last_results.clear();
    }

    /// Ticks advanced so far.
    pub const fn tick_count(&self) -> u64 {
        self.tick_count
    }

    /// Winner of the last run, if any.
    pub const fn last_best(&self) -> Option<EqPoint> {
        self.last_best
    }

    /// Ranked results of the last run.
    pub fn last_results(&self) -> &[EqPoint] {
        &self.last_results
    }
}

/// One cover move bound for the tick boundary: the winning EQS spot
/// quantized once into a validated [`ParticipantInput`] plus the query
/// `request_id` the boundary uses to drop stale replies.
/// Units: positions in millimetres, facing in centidegrees,
/// steer in `-100..=100`, ticks as fixed steps. Determinism: integer
/// math only; same winner, self, tick, fighter, facing and id give
/// the same ticket. Errors: none; facing clamps into range and the
/// wish is always legal, so validation passes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct EqCoverTicket {
    /// Move input for one tick: cover steer, given tick and fighter,
    /// clamped facing, no buttons.
    pub input: ParticipantInput,
    /// Query id carried for boundary dedupe; never hashed alone.
    pub request_id: u64,
}

/// Tactics query builders (donor `EqTactics`): the API the tactics
/// brain calls for cover/ambush selection. Pure data in (positions,
/// facing), ranked query out; the brain feeds sensor values straight
/// in and aims the depth wish at the winner via [`Tactics::cover_wish`].
pub struct Tactics;

impl Tactics {
    /// Cover query: standing spots behind each cover object relative to
    /// the threat, ranked by alignment with the nearest cover, then by
    /// distance from the threat, under a travel budget from the querier.
    pub fn build_cover_query(
        covers: &[Vec3Mm],
        standoff_mm: i32,
        max_dist_mm: i32,
        min_clear_mm: i32,
    ) -> Result<EqQuery<'_>, EqError> {
        Ok(EqQuery::new(Generator::Cover(CoverSpotGenerator::new(
            covers,
            standoff_mm,
            min_clear_mm,
        )?))
        .with_test(Test::budget(max_dist_mm)?)
        .with_test(Test::cover_align(covers, 2))
        .with_test(Test::distance(Preference::Far, max_dist_mm)?))
    }

    /// Ambush query: spots kept only with line of sight to the target,
    /// ranked by flank position behind the threat facing, then by
    /// nearness. A zero facing skips the flank preference entirely.
    pub fn build_ambush_query<'x>(
        spots: &'x [Vec3Mm],
        vis: &'x dyn Visibility,
        facing: Vec3Mm,
        max_dist_mm: i32,
        flank_power: u32,
    ) -> Result<EqQuery<'x>, EqError> {
        let mut query = EqQuery::new(Generator::List(PointListGenerator::new(spots)))
            .with_test(Test::budget(max_dist_mm)?)
            .with_test(Test::sightline(vis));
        if facing.x != 0 || facing.z != 0 {
            query = query.with_test(Test::flank(facing, flank_power));
        }
        Ok(query.with_test(Test::distance(Preference::Near, max_dist_mm)?))
    }

    /// Quantize one ground-plane delta into steer units once: direction
    /// from self to the spot scaled to `-100..=100` (`(0,0)` when already
    /// there). Single owner for [`Tactics::cover_ticket`] and
    /// [`Tactics::cover_wish`]; integer math only, never panics.
    ///
    /// Caller: [`Tactics::cover_ticket`] (the GA8 tick bind) folds the
    /// winner-minus-self delta through here. Units: millimetre delta in,
    /// steer units out. Determinism: same delta gives the same steer
    /// twice. Errors: none; the refused arm holds still.
    #[must_use]
    pub fn cover_steer(dx: i64, dz: i64) -> (i8, i8) {
        let n = Vec3Mm::len_xz(dx, dz);
        if n <= 0 {
            (0, 0)
        } else {
            // Clamped to `-100..=100`, inside `i8`; the refused arm holds
            // still and never panics.
            let mx = match i8::try_from((dx * 100 / n).clamp(-100, 100)) {
                Ok(b) => b,
                Err(_) => 0,
            };
            let mz = match i8::try_from((dz * 100 / n).clamp(-100, 100)) {
                Ok(b) => b,
                Err(_) => 0,
            };
            (mx, mz)
        }
    }

    /// Bind the winning cover spot to one tick input: ground-plane
    /// direction from self to the spot scaled to `-100..=100` steer
    /// units (`(0,0)` when already there), facing clamped, buttons empty.
    /// The caller records the ticket and applies `input` at the tick
    /// boundary under `request_id`; replay reads the record.
    pub fn cover_ticket(
        winner: EqPoint,
        self_pos: Vec3Mm,
        tick: u32,
        fighter: ParticipantId,
        facing_centideg: i16,
        request_id: u64,
    ) -> EqCoverTicket {
        let dx = i64::from(winner.pos.x) - i64::from(self_pos.x);
        let dz = i64::from(winner.pos.z) - i64::from(self_pos.z);
        let (mx, mz) = Self::cover_steer(dx, dz);
        let facing = facing_centideg.clamp(-FACING_RANGE, FACING_RANGE);
        EqCoverTicket {
            input: ParticipantInput {
                tick,
                fighter,
                mx,
                mz,
                facing,
                buttons: Buttons::NONE,
            },
            request_id,
        }
    }

    /// Aim the depth wish at the winning spot: ground-plane direction from
    /// self to the spot scaled to `-100..=100` steer units (`(0,0)` when
    /// already there). Feeds steering `{ mx, mz }` like `DepthBrain`.
    /// Steering-only projection of [`Tactics::cover_ticket`] with a dummy
    /// tick, slot, facing and request id; the full bind carries the real ones.
    pub fn cover_wish(winner: EqPoint, self_pos: Vec3Mm) -> (i8, i8) {
        // Slot 0 is always legal; a refused slot holds still, never panics.
        match ParticipantId::new(0) {
            Some(slot) => {
                let ticket = Self::cover_ticket(winner, self_pos, 0, slot, 0, 0);
                (ticket.input.mx, ticket.input.mz)
            }
            None => (0, 0),
        }
    }

    /// Bind plus boundary in one call: quantize the winning spot into one
    /// tick input via [`Tactics::cover_ticket`], then dedupe through
    /// `boundary` by `request_id`. Fresh ids return the input for the tick
    /// boundary to record; duplicates and stale ids drop to `None` so
    /// replays read the same record twice. Caller: the arena tick applies
    /// the returned input under `request_id`; the EQS query itself runs on
    /// the perception clock. Integer only; never panics.
    pub fn bound_cover_input(
        winner: EqPoint,
        self_pos: Vec3Mm,
        tick: u32,
        fighter: ParticipantId,
        facing_centideg: i16,
        request_id: u64,
        boundary: &mut CoverBoundary,
    ) -> Option<ParticipantInput> {
        let ticket =
            Self::cover_ticket(winner, self_pos, tick, fighter, facing_centideg, request_id);
        boundary.apply(ticket)
    }
}

/// Tick-boundary dedupe for EQS cover binds: the arena applies each query
/// reply once, keyed by `request_id`.
///
/// Consumer: [`Tactics::bound_cover_input`] (the GA8 arena bind) threads a
/// `CoverBoundary` through the tick boundary; the arena records the returned
/// input and replays it. Owner: the boundary owns the last applied id only;
/// tickets stay caller-owned. Units: request ids as `u64` ticks. Determinism:
/// integer compare only; same ticket sequence gives same apply sequence;
/// a fresh boundary is hash-neutral (no input). Errors: none; stale and
/// duplicate ids drop to `None`, never panic. Budget: O(1) per apply, no
/// allocation.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct CoverBoundary {
    last_applied: Option<u64>,
}

impl CoverBoundary {
    /// New boundary with nothing applied (hash-neutral until first apply).
    pub const fn new() -> Self {
        Self { last_applied: None }
    }

    /// Last applied request id, if any.
    pub const fn last_applied(&self) -> Option<u64> {
        self.last_applied
    }

    /// Apply one ticket at the tick boundary: fresh ids return the input,
    /// duplicates and stale (out-of-order) ids drop to `None`. Same inputs
    /// give same outputs; never panics.
    pub fn apply(&mut self, ticket: EqCoverTicket) -> Option<ParticipantInput> {
        if let Some(last) = self.last_applied {
            if ticket.request_id <= last {
                return None;
            }
        }
        self.last_applied = Some(ticket.request_id);
        Some(ticket.input)
    }

    /// Forget the last id (a new engagement round).
    pub fn reset(&mut self) {
        self.last_applied = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::depth::DepthBrain;
    use crate::gladiator::Percept;
    use engine_input::FighterId;

    const THREAT: Vec3Mm = Vec3Mm::new(10_000, 0, 0);
    const SELF: Vec3Mm = Vec3Mm::new(-8_000, 0, 0);

    fn ctx() -> EqContext {
        EqContext::new(SELF, THREAT)
    }

    fn run(q: &EqQuery<'_>, ctx: &EqContext) -> Vec<EqPoint> {
        let mut out = Vec::new();
        q.run_into(ctx, &mut out).expect("fixture query runs");
        out
    }

    fn fighter(id: u8) -> FighterId {
        FighterId::new(id).expect("slot is legal")
    }

    /// Blank comments, strings and chars from Rust source so a token scan
    /// only sees real identifiers. Lifetimes (`'a`) pass through as code;
    /// their names never match a genre word.
    fn mask_code(src: &str) -> String {
        let b = src.as_bytes();
        let mut out = String::with_capacity(src.len());
        let mut i = 0;
        while i < b.len() {
            let c = b[i];
            if c == b'/' && i + 1 < b.len() && (b[i + 1] == b'/' || b[i + 1] == b'*') {
                let line = b[i + 1] == b'/';
                let mut depth = 1;
                out.push_str("  ");
                i += 2;
                while i < b.len() && (!line || b[i] != b'\n') && depth > 0 {
                    if !line && b[i] == b'/' && i + 1 < b.len() && b[i + 1] == b'*' {
                        depth += 1;
                        out.push_str("  ");
                        i += 2;
                    } else if !line && b[i] == b'*' && i + 1 < b.len() && b[i + 1] == b'/' {
                        depth -= 1;
                        out.push_str("  ");
                        i += 2;
                    } else {
                        out.push(if b[i] == b'\n' { '\n' } else { ' ' });
                        i += 1;
                    }
                }
                continue;
            }
            if c == b'"' {
                out.push(' ');
                i += 1;
                while i < b.len() {
                    if b[i] == b'\\' {
                        out.push_str("  ");
                        i += 2;
                    } else if b[i] == b'"' {
                        out.push(' ');
                        i += 1;
                        break;
                    } else {
                        out.push(if b[i] == b'\n' { '\n' } else { ' ' });
                        i += 1;
                    }
                }
                continue;
            }
            // A quoted span ending at the next quote is a char; a bare
            // `'a` is a lifetime and stays code.
            if c == b'\'' {
                let mut k = i + 1;
                if k < b.len() && b[k] == b'\\' {
                    while k < b.len() && b[k] != b'\'' {
                        k += 1;
                    }
                    k += 1;
                } else {
                    while k < b.len() && (b[k].is_ascii_alphanumeric() || b[k] == b'_') {
                        k += 1;
                    }
                    if !(k < b.len() && b[k] == b'\'' && k > i + 1) {
                        k = i + 1;
                    } else {
                        k += 1;
                    }
                }
                if k > i + 1 {
                    for _ in i..k.min(b.len()) {
                        out.push(' ');
                    }
                    i = k.min(b.len());
                    continue;
                }
            }
            out.push(c as char);
            i += 1;
        }
        out
    }

    #[test]
    fn eqs_production_code_stays_genre_neutral() {
        // GE2 ledger pin: production code below arena/game must not name
        // `Fighter*`/`Gladiator*` identifiers; cross-genre code uses the
        // engine-input `Participant*` aliases. Mirrors the identifier scan
        // in tools/sim-source-scan.mjs (test module split off, comments
        // and literals masked, identifier tokens matched).
        let src = include_str!("eqs.rs");
        let prod = src
            .split("#[cfg(test)]")
            .next()
            .expect("test module marker");
        let code = mask_code(prod);
        let mut bad: Vec<&str> = Vec::new();
        for tok in code.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_')) {
            if (tok.contains("Fighter") || tok.contains("Gladiator") || tok.contains("gladiator"))
                && !bad.contains(&tok)
            {
                bad.push(tok);
            }
        }
        assert!(
            bad.is_empty(),
            "genre identifiers leak into eqs production code: {bad:?}"
        );
    }

    #[test]
    fn cover_spot_stands_off_away_from_threat() {
        // Cover at x=4000, threat at x=10000: away is -X, so the spot
        // lands exactly one standoff behind the cover.
        let covers = [Vec3Mm::new(4_000, 0, 0)];
        let gen = CoverSpotGenerator::new(&covers, 2_000, 0).expect("standoff is legal");
        let q = EqQuery::new(Generator::Cover(gen));
        let out = run(&q, &ctx());
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].pos, Vec3Mm::new(2_000, 0, 0));
        assert_eq!(out[0].score, SCORE_MAX);
    }

    #[test]
    fn cover_generator_skips_pillars_too_close_to_threat() {
        // Cover 500 mm from the threat cannot fit a 2000 mm standoff.
        let covers = [Vec3Mm::new(9_500, 0, 0), Vec3Mm::new(4_000, 0, 0)];
        let gen = CoverSpotGenerator::new(&covers, 2_000, 0).expect("standoff is legal");
        let q = EqQuery::new(Generator::Cover(gen));
        let out = run(&q, &ctx());
        assert_eq!(out.len(), 1, "only the far pillar emits a spot");
        assert_eq!(out[0].pos, Vec3Mm::new(2_000, 0, 0));
        // Minimum clearance also refuses tight pillars.
        let gen = CoverSpotGenerator::new(&covers, 1_000, 5_000).expect("standoff is legal");
        let q = EqQuery::new(Generator::Cover(gen));
        assert!(run(&q, &ctx()).is_empty());
    }

    #[test]
    fn flank_scores_behind_over_side_over_front() {
        // Points dead ahead, at the side, and directly behind the target
        // facing +X score 0 / 500 / 1000 at power 1.
        let facing = Vec3Mm::new(1000, 0, 0);
        let spots = [
            Vec3Mm::new(14_000, 0, 0),
            Vec3Mm::new(10_000, 0, 4_000),
            Vec3Mm::new(6_000, 0, 0),
        ];
        let q = EqQuery::new(Generator::List(PointListGenerator::new(&spots)))
            .with_test(Test::flank(facing, 1));
        let out = run(&q, &ctx());
        assert_eq!(out.len(), 3);
        // Ranked best first: behind, side, front.
        assert_eq!(out[0].pos, Vec3Mm::new(6_000, 0, 0));
        assert_eq!(out[0].score, 1000);
        assert_eq!(out[1].pos, Vec3Mm::new(10_000, 0, 4_000));
        assert_eq!(out[1].score, 500);
        assert_eq!(out[2].pos, Vec3Mm::new(14_000, 0, 0));
        assert_eq!(out[2].score, 0);
    }

    #[test]
    fn flank_power_sharpens_the_falloff() {
        // Power 2 squares the side score: 500 -> 250, ends stay pinned.
        let facing = Vec3Mm::new(1000, 0, 0);
        let spots = [Vec3Mm::new(10_000, 0, 4_000), Vec3Mm::new(6_000, 0, 0)];
        let q = EqQuery::new(Generator::List(PointListGenerator::new(&spots)))
            .with_test(Test::flank(facing, 2));
        let out = run(&q, &ctx());
        assert_eq!((out[0].score, out[1].score), (1000, 250));
    }

    #[test]
    fn alignment_is_full_behind_cover_and_low_in_the_open() {
        let covers = [Vec3Mm::new(4_000, 0, 0)];
        // Spot at x=2000: cover sits on the spot->threat ray, dot = +1.
        // Spot at x=7000: cover lies behind the spot while the threat is
        // ahead, dot = -1, so the best stays 0 and the score floors.
        let spots = [Vec3Mm::new(2_000, 0, 0), Vec3Mm::new(7_000, 0, 0)];
        let q = EqQuery::new(Generator::List(PointListGenerator::new(&spots)))
            .with_test(Test::cover_align(&covers, 1));
        let out = run(&q, &ctx());
        assert_eq!(out[0].pos, Vec3Mm::new(2_000, 0, 0));
        assert_eq!(out[0].score, 1000);
        assert_eq!(out[1].pos, Vec3Mm::new(7_000, 0, 0));
        assert_eq!(out[1].score, 0);
    }

    struct Blind;
    impl Visibility for Blind {
        fn has_los(&self, _from: Vec3Mm, _to: Vec3Mm) -> bool {
            false
        }
    }

    struct LeftOpen;
    impl Visibility for LeftOpen {
        fn has_los(&self, from: Vec3Mm, _to: Vec3Mm) -> bool {
            from.x <= 0
        }
    }

    #[test]
    fn sightline_vetoes_blind_spots_only() {
        let spots = [Vec3Mm::new(-4_000, 0, 0), Vec3Mm::new(4_000, 0, 0)];
        let blind = Blind;
        let q = EqQuery::new(Generator::List(PointListGenerator::new(&spots)))
            .with_test(Test::sightline(&blind));
        let out = run(&q, &ctx());
        assert!(out.iter().all(|p| p.score == 0), "blind vetoes everything");
        // A selective provider keeps the visible spot at full score.
        let vis = LeftOpen;
        let q = EqQuery::new(Generator::List(PointListGenerator::new(&spots)))
            .with_test(Test::sightline(&vis));
        let out = run(&q, &ctx());
        assert_eq!(out[0].pos, Vec3Mm::new(-4_000, 0, 0));
        assert_eq!((out[0].score, out[1].score), (1000, 0));
        // The always-visible default never vetoes.
        let always = AlwaysVisible;
        let q = EqQuery::new(Generator::List(PointListGenerator::new(&spots)))
            .with_test(Test::sightline(&always));
        assert!(run(&q, &ctx()).iter().all(|p| p.score == SCORE_MAX));
    }

    #[test]
    fn budget_rejects_spots_past_the_allowance() {
        let spots = [Vec3Mm::new(-7_000, 0, 0), Vec3Mm::new(9_000, 0, 0)];
        let q = EqQuery::new(Generator::List(PointListGenerator::new(&spots)))
            .with_test(Test::budget(2_000).expect("budget is legal"));
        let out = run(&q, &ctx());
        assert_eq!((out[0].score, out[1].score), (1000, 0));
    }

    #[test]
    fn distance_prefers_near_or_far_ends() {
        // Near threat (x=9500) vs far querier side (x=-8000): each
        // preference pins its own end on top.
        let spots = [Vec3Mm::new(9_500, 0, 500), Vec3Mm::new(-8_000, 0, 0)];
        let max = 20_000;
        let q = EqQuery::new(Generator::List(PointListGenerator::new(&spots)))
            .with_test(Test::distance(Preference::Near, max).expect("budget is legal"));
        let out = run(&q, &ctx());
        assert_eq!(out[0].pos, Vec3Mm::new(9_500, 0, 500));
        assert!(out[0].score > out[1].score);
        let q = EqQuery::new(Generator::List(PointListGenerator::new(&spots)))
            .with_test(Test::distance(Preference::Far, max).expect("budget is legal"));
        let out = run(&q, &ctx());
        assert_eq!(out[0].pos, Vec3Mm::new(-8_000, 0, 0));
        assert!(out[0].score > out[1].score);
    }

    #[test]
    fn ring_counts_are_exact_and_deterministic() {
        // Four points land exactly on the axes; the same query twice is
        // byte-identical.
        let gen = RingGenerator::new(4_000, 4).expect("ring is legal");
        let q = EqQuery::new(Generator::Ring(gen));
        let (a, b) = (run(&q, &ctx()), run(&q, &ctx()));
        assert_eq!(a, b);
        assert_eq!(
            a.iter().map(|p| p.pos).collect::<Vec<_>>(),
            vec![
                Vec3Mm::new(-4_000, 0, 0),
                Vec3Mm::new(-8_000, 0, 4_000),
                Vec3Mm::new(-12_000, 0, 0),
                Vec3Mm::new(-8_000, 0, -4_000),
            ]
        );
        // Sixteen points keep every baked direction distinct.
        let gen = RingGenerator::new(1_000, 16).expect("ring is legal");
        let q = EqQuery::new(Generator::Ring(gen));
        let out = run(&q, &ctx());
        let mut dedup = out.iter().map(|p| p.pos).collect::<Vec<_>>();
        dedup.sort_by_key(|p| (p.x, p.z));
        dedup.dedup();
        assert_eq!(dedup.len(), 16);
    }

    #[test]
    fn grid_counts_match_the_box_and_cover_query_ranks() {
        // 2 m half-extent at 2 m spacing: 3x3 = 9 points.
        let gen = GridGenerator::new(2_000, 2_000).expect("grid is legal");
        let q = EqQuery::new(Generator::Grid(gen));
        assert_eq!(run(&q, &ctx()).len(), 9);
        // Full tactics cover query: two covers, both emit, the winner is
        // the ranked head of the same buffer.
        let covers = [Vec3Mm::new(4_000, 0, 0), Vec3Mm::new(0, 0, 6_000)];
        let query = Tactics::build_cover_query(&covers, 2_000, 30_000, 0).expect("cover builds");
        let mut scratch = Vec::new();
        let best = query.best_into(&ctx(), &mut scratch).expect("cover runs");
        assert_eq!(scratch.len(), 2);
        assert!(best.is_some());
        assert_eq!(best, scratch.first().copied());
    }

    #[test]
    fn ambush_query_needs_sightline_and_flanks_when_facing() {
        let spots = [
            Vec3Mm::new(14_000, 0, 0),
            Vec3Mm::new(6_000, 0, 0),
            Vec3Mm::new(60_000, 0, 0),
        ];
        let vis = AlwaysVisible;
        // Facing +X: the behind-threat spot wins over the nearer front one.
        let query = Tactics::build_ambush_query(&spots, &vis, Vec3Mm::new(1000, 0, 0), 30_000, 2)
            .expect("ambush builds");
        let mut scratch = Vec::new();
        let best = query.best_into(&ctx(), &mut scratch).expect("ambush runs");
        assert_eq!(scratch.len(), 3);
        assert_eq!(best.map(|p| p.pos), Some(Vec3Mm::new(6_000, 0, 0)));
    }

    #[test]
    fn ambush_without_facing_falls_back_to_distance() {
        let spots = [Vec3Mm::new(14_000, 0, 0), Vec3Mm::new(6_000, 0, 0)];
        let vis = AlwaysVisible;
        // Zero facing drops the flank test: nearest visible spot wins.
        let query = Tactics::build_ambush_query(&spots, &vis, Vec3Mm::ZERO, 30_000, 2)
            .expect("ambush builds");
        let mut scratch = Vec::new();
        let best = query.best_into(&ctx(), &mut scratch).expect("ambush runs");
        assert_eq!(best.map(|p| p.pos), Some(Vec3Mm::new(14_000, 0, 0)));
    }

    #[test]
    fn degenerate_queries_refuse_with_typed_errors() {
        assert_eq!(RingGenerator::new(100, 0), Err(EqError::ZeroCount));
        assert_eq!(
            RingGenerator::new(100, 17).unwrap_err(),
            EqError::TooManyPoints { got: 17 }
        );
        assert_eq!(
            GridGenerator::new(1_000, 0).unwrap_err(),
            EqError::BadSpacing { got: 0 }
        );
        assert_eq!(Test::budget(0).unwrap_err(), EqError::BadBudget { got: 0 });
        assert_eq!(
            CoverSpotGenerator::new(&[], 0, 0).unwrap_err(),
            EqError::BadStandoff { got: 0 }
        );
        assert_eq!(
            Tactics::build_cover_query(&[], 1_000, 0, 0).unwrap_err(),
            EqError::BadBudget { got: 0 }
        );
        // Empty covers still build, then rank empty with no winner.
        let query = Tactics::build_cover_query(&[], 1_000, 10_000, 0).expect("empty builds");
        let mut scratch = Vec::new();
        assert_eq!(
            query.best_into(&ctx(), &mut scratch).expect("empty ranks"),
            None
        );
        assert!(scratch.is_empty());
    }

    #[test]
    fn runner_runs_on_schedule_and_reuses_its_buffer() {
        let spots = [Vec3Mm::new(6_000, 0, 0), Vec3Mm::new(14_000, 0, 0)];
        let query = EqQuery::new(Generator::List(PointListGenerator::new(&spots)))
            .with_test(Test::flank(Vec3Mm::new(1000, 0, 0), 1));
        let mut runner = EqRunner::new(query.clone(), 3).expect("interval is legal");
        assert_eq!(EqRunner::new(query, 0).unwrap_err(), EqError::BadInterval);
        let first = runner
            .tick(&ctx())
            .expect("tick runs")
            .expect("spots exist");
        assert_eq!(first.pos, Vec3Mm::new(6_000, 0, 0));
        // Ticks 2-3 hold the cached winner without rerunning.
        assert_eq!(runner.tick(&ctx()).expect("tick holds"), Some(first));
        assert_eq!(runner.tick(&ctx()).expect("tick holds"), Some(first));
        assert_eq!(runner.tick_count(), 3);
        assert_eq!(runner.last_results().len(), 2);
        // Forced updates rerun immediately; reset clears everything.
        assert_eq!(runner.force(&ctx()).expect("force runs"), Some(first));
        runner.reset();
        assert_eq!((runner.tick_count(), runner.last_best()), (0, None));
        assert!(runner.last_results().is_empty());
    }

    #[test]
    fn tactics_cover_wish_drives_the_depth_brain() {
        // The tactics consumer: best cover spot becomes a depth steer wish,
        // the depth brain decides a valid input from it, twice identical.
        let covers = [Vec3Mm::new(4_000, 0, 0)];
        let query = Tactics::build_cover_query(&covers, 2_000, 30_000, 0).expect("cover builds");
        let mut scratch = Vec::new();
        let best = query
            .best_into(&ctx(), &mut scratch)
            .expect("cover runs")
            .expect("spot exists");
        assert_eq!(best.pos, Vec3Mm::new(2_000, 0, 0));
        let (mx, mz) = Tactics::cover_wish(best, SELF);
        assert_eq!((mx, mz), (100, 0), "due east at full steer");
        let (zx, zz) = Tactics::cover_wish(best, best.pos);
        assert_eq!((zx, zz), (0, 0), "already there holds still");
        // Depth composition: the wish rides a real depth-brain percept and
        // the emitted input validates, twice identical.
        let drive = |wish: (i8, i8)| {
            let mut brain = DepthBrain::new(fighter(0), 0);
            let percept = Percept::new([-8.0, 0.0, 0.0], 0.0, [2.0, 0.0, 0.0], false, 10.0, 7);
            let mut input = brain.decide(&percept);
            input.mx = wish.0;
            input.mz = wish.1;
            input.validate().expect("tactics wish validates");
            input
        };
        assert_eq!(drive((mx, mz)), drive((mx, mz)));
    }

    #[test]
    fn eqs_cover_ticket_binds_validated_input() {
        // First live bind: the winning cover spot becomes one tick input
        // with the query id attached; replay reads the record.
        let winner = EqPoint::new(Vec3Mm::new(2_000, 0, 0));
        let first = Tactics::cover_ticket(winner, SELF, 7, fighter(1), 9_000, 41);
        assert_eq!((first.input.mx, first.input.mz), (100, 0));
        assert_eq!(
            (first.input.tick, first.input.fighter, first.input.facing),
            (7, fighter(1), 9_000)
        );
        assert_eq!(first.input.buttons, engine_input::Buttons::NONE);
        assert_eq!(first.request_id, 41);
        first.input.validate().expect("cover bind validates");
        let second = Tactics::cover_ticket(winner, SELF, 7, fighter(1), 9_000, 41);
        assert_eq!(first, second, "same bind twice is identical");
    }

    #[test]
    fn eqs_cover_ticket_holds_and_clamps_facing() {
        // Already on the spot holds still; wild facing clamps into range
        // instead of failing validation.
        let winner = EqPoint::new(SELF);
        let held = Tactics::cover_ticket(winner, SELF, 3, fighter(0), 0, 9);
        assert_eq!((held.input.mx, held.input.mz), (0, 0));
        held.input.validate().expect("hold validates");
        let high = Tactics::cover_ticket(winner, SELF, 3, fighter(0), 20_000, 9);
        assert_eq!(high.input.facing, 18_000);
        high.input
            .validate()
            .expect("clamped high facing validates");
        let low = Tactics::cover_ticket(winner, SELF, 3, fighter(0), -20_000, 9);
        assert_eq!(low.input.facing, -18_000);
        low.input.validate().expect("clamped low facing validates");
    }

    #[test]
    fn eqs_cover_wish_rides_the_ticket_bind() {
        // The steering projection stays on the ticket bind: one owner,
        // no drift between the two spellings.
        let winner = EqPoint::new(Vec3Mm::new(2_000, 0, 0));
        let wish = Tactics::cover_wish(winner, SELF);
        let ticket = Tactics::cover_ticket(winner, SELF, 0, fighter(0), 0, 0);
        assert_eq!(wish, (ticket.input.mx, ticket.input.mz));
        let held = Tactics::cover_wish(winner, winner.pos);
        assert_eq!(held, (0, 0));
    }

    #[test]
    fn eqs_cover_steer_quantizes_once() {
        // GA8 single owner: the ground-plane delta quantizes once into
        // steer units, wired into the ticket bind with no drift.
        assert_eq!(Tactics::cover_steer(2_000, 0), (100, 0));
        assert_eq!(Tactics::cover_steer(0, 0), (0, 0));
        assert_eq!(Tactics::cover_steer(0, -3_000), (0, -100));
        let diag = Tactics::cover_steer(1_000, 1_000);
        assert_eq!(diag, Tactics::cover_steer(1_000, 1_000));
        let winner = EqPoint::new(Vec3Mm::new(2_000, 0, 0));
        let ticket = Tactics::cover_ticket(winner, SELF, 0, fighter(0), 0, 0);
        assert_eq!(
            Tactics::cover_steer(2_000 - SELF.x as i64, 0 - SELF.z as i64),
            (ticket.input.mx, ticket.input.mz)
        );
        assert_eq!(diag, Tactics::cover_steer(1_000, 1_000), "x2 equal");
    }

    #[test]
    fn eqs_cover_boundary_drives_to_cover_and_holds_x2() {
        // GA8 bind: the ranked cover spot drives a live fighter to cover
        // and holds; the boundary quantizes once per tick under a request
        // id; replay reads the same tape twice.
        let covers = [Vec3Mm::new(4_000, 0, 0)];
        let query = Tactics::build_cover_query(&covers, 2_000, 30_000, 0).expect("cover builds");
        let mut scratch = Vec::new();
        let winner = query
            .best_into(&ctx(), &mut scratch)
            .expect("cover runs")
            .expect("spot exists");
        assert_eq!(winner.pos, Vec3Mm::new(2_000, 0, 0));
        let drive = |start: Vec3Mm| {
            let mut pos = start;
            let mut boundary = CoverBoundary::new();
            let mut tape = Vec::new();
            for tick in 0..12u32 {
                let applied = Tactics::bound_cover_input(
                    winner,
                    pos,
                    tick,
                    fighter(0),
                    0,
                    u64::from(tick),
                    &mut boundary,
                )
                .expect("fresh id applies");
                applied.validate().expect("bound input validates");
                tape.push(applied);
                // Integer step: steer units (-100..=100) at 10 mm each.
                pos = Vec3Mm::new(
                    pos.x.saturating_add(i32::from(applied.mx) * 10),
                    pos.y,
                    pos.z.saturating_add(i32::from(applied.mz) * 10),
                );
            }
            (tape, pos, boundary.last_applied())
        };
        let (tape, end, last) = drive(SELF);
        assert_eq!((tape[0].mx, tape[0].mz), (100, 0));
        assert_eq!(end, winner.pos, "drive reaches the ranked spot");
        assert_eq!(
            (tape[11].mx, tape[11].mz),
            (0, 0),
            "holding at cover steers zero"
        );
        let (again, again_end, again_last) = drive(SELF);
        assert_eq!(tape, again, "replay x2 identical tape");
        assert_eq!((end, last), (again_end, again_last));
        // Never queried: a fresh boundary stays untouched (arena hash-neutral).
        assert_eq!(CoverBoundary::new().last_applied(), None);
        assert_eq!(CoverBoundary::new(), CoverBoundary::new());
    }

    #[test]
    fn eqs_cover_boundary_drops_duplicate_and_stale_ids() {
        // Failure path: the tick boundary applies each request id once;
        // replays and stale (out-of-order) replies drop to None.
        let winner = EqPoint::new(Vec3Mm::new(2_000, 0, 0));
        let mut boundary = CoverBoundary::new();
        let first = Tactics::bound_cover_input(winner, SELF, 7, fighter(1), 0, 41, &mut boundary)
            .expect("first id applies");
        assert_eq!((first.mx, first.mz), (100, 0));
        assert_eq!(boundary.last_applied(), Some(41));
        assert_eq!(
            Tactics::bound_cover_input(winner, SELF, 7, fighter(1), 0, 41, &mut boundary),
            None,
            "duplicate id drops"
        );
        assert_eq!(
            Tactics::bound_cover_input(winner, SELF, 8, fighter(1), 0, 40, &mut boundary),
            None,
            "stale id drops"
        );
        assert!(
            Tactics::bound_cover_input(winner, SELF, 8, fighter(1), 0, 42, &mut boundary).is_some(),
            "next id applies"
        );
        assert_eq!(boundary.last_applied(), Some(42));
    }
}
