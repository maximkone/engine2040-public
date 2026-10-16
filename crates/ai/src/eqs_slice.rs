//! eqs_slice: Geqo-style chained generation plus tick-sliced scoring.
//! Consumer: the tactics brain builds an [`EqQuery`] via [`Tactics`], then
//! drives it across perception-clock ticks via [`SlicedRunner`]. Owner:
//! [`SlicedRunner`] owns ranking state; generators borrow spots only.
//! Units: mm, scores `0..=1000`, points per tick. Determinism: integer math
//! via `eqs`, index order, no floats, no wall clock. Errors: [`EqError`];
//! refused arms skip, empty ranks empty. Budget: `per_tick` per tick.
//! PORT(geqo-godot/geqo@86d62a57:generator_composite3d.h,query_instance.h).

use engine_core::hash::Fnv64;

use crate::eqs::{EqContext, EqError, EqPoint, EqQuery, Generator, Vec3Mm, MAX_POINTS};

/// Borrowed-spot verbatim emitter (donor `generator_in_array3d` shape).
#[derive(Clone, Copy, Debug)]
pub struct SensorArrayGenerator<'a> {
    spots: &'a [Vec3Mm],
}

impl<'a> SensorArrayGenerator<'a> {
    /// New emitter over borrowed spots.
    pub const fn new(spots: &'a [Vec3Mm]) -> Self {
        Self { spots }
    }
    /// Spot count.
    pub const fn len(&self) -> usize {
        self.spots.len()
    }
    /// True when no spots are held.
    pub const fn is_empty(&self) -> bool {
        self.spots.is_empty()
    }
    /// Append spots verbatim; cap-checked. Empty emits nothing.
    pub fn generate_into(&self, out: &mut Vec<EqPoint>) -> Result<(), EqError> {
        if self.is_empty() {
            return Ok(());
        }
        for p in self.spots {
            if out.len() >= MAX_POINTS {
                return Err(EqError::TooManyPoints { got: out.len() + 1 });
            }
            out.push(EqPoint::new(*p));
        }
        Ok(())
    }
}

/// Ordered chain of up to 4 generator arms (donor `GeneratorComposite3D`).
#[derive(Clone, Copy, Debug)]
pub struct CompositeGenerator<'a> {
    arms: [Option<Generator<'a>>; 4],
    len: u8,
}

impl<'a> Default for CompositeGenerator<'a> {
    /// Empty chain; [`CompositeGenerator::new`] matches it.
    fn default() -> Self {
        Self {
            arms: [None, None, None, None],
            len: 0,
        }
    }
}

impl<'a> CompositeGenerator<'a> {
    /// Empty chain; same as `Default`.
    pub fn new() -> Self {
        Self::default()
    }
    /// Arm count.
    pub fn len(&self) -> usize {
        usize::from(self.len)
    }
    /// True when no arms are held.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
    /// Push one arm; refuses past 4.
    pub fn with_arm(mut self, arm: Generator<'a>) -> Result<Self, EqError> {
        if self.len >= 4 {
            return Err(EqError::TooManyPoints { got: 5 });
        }
        self.arms[usize::from(self.len)] = Some(arm);
        self.len += 1;
        Ok(self)
    }
    /// Append chained candidates in arm order; refused arms skip clean.
    /// An empty chain emits nothing.
    pub fn generate_into(&self, ctx: &EqContext, out: &mut Vec<EqPoint>) -> Result<(), EqError> {
        if self.is_empty() {
            return Ok(());
        }
        for arm in self.arms.iter().take(usize::from(self.len)).flatten() {
            let base = out.len();
            if arm.generate_crate(ctx, out).is_err() {
                out.truncate(base);
            }
        }
        Ok(())
    }
}

/// Tick-sliced scorer over an owned query (donor `QueryInstanceBase`).
#[derive(Clone, Debug)]
pub struct SlicedRunner<'a> {
    query: EqQuery<'a>,
    per_tick: usize,
    cursor: usize,
    ctx: Option<EqContext>,
    points: Vec<EqPoint>,
    started: bool,
    head: Option<EqPoint>,
    last_scored: usize,
}

impl<'a> SlicedRunner<'a> {
    /// New runner; `per_tick` of 0 refuses.
    pub fn new(query: EqQuery<'a>, per_tick: usize) -> Result<Self, EqError> {
        if per_tick == 0 {
            return Err(EqError::BadInterval);
        }
        Ok(Self {
            query,
            per_tick,
            cursor: 0,
            ctx: None,
            points: Vec::new(),
            started: false,
            head: None,
            last_scored: 0,
        })
    }
    /// Points scored per tick.
    pub const fn per_tick(&self) -> usize {
        self.per_tick
    }
    /// Next unscored index.
    pub const fn cursor(&self) -> usize {
        self.cursor
    }
    /// Points scored by the last tick.
    pub const fn last_scored(&self) -> usize {
        self.last_scored
    }
    /// True when every candidate is scored.
    pub fn done(&self) -> bool {
        self.started && self.cursor >= self.points.len()
    }
    /// Ranked head among scored-so-far, if any.
    pub const fn best(&self) -> Option<EqPoint> {
        self.head
    }
    /// FNV-1a64 over scored positions and scores; twice identical.
    pub fn digest(&self) -> u64 {
        let mut h = Fnv64::new();
        let end = self.cursor.min(self.points.len());
        for p in &self.points[..end] {
            h.write_i32(p.pos.x);
            h.write_i32(p.pos.y);
            h.write_i32(p.pos.z);
            h.write_i32(p.score);
        }
        h.finish()
    }
    /// Forget progress; the next tick regenerates.
    pub fn reset(&mut self) {
        self.cursor = 0;
        self.ctx = None;
        self.points.clear();
        self.started = false;
        self.head = None;
        self.last_scored = 0;
    }
    /// Score the next slice; remembers the ranked head.
    pub fn tick(&mut self, ctx: &EqContext) -> Result<Option<EqPoint>, EqError> {
        if !self.started {
            self.points.clear();
            let g = self.query.generator_crate();
            g.generate_crate(ctx, &mut self.points)?;
            self.ctx = Some(*ctx);
            self.started = true;
        }
        let owned = self.ctx.unwrap_or(*ctx);
        let end = (self.cursor + self.per_tick).min(self.points.len());
        if self.cursor < end {
            let n = self.query.tests_crate().len();
            for i in 0..n {
                let t = self.query.tests_crate()[i];
                t.score_crate(&mut self.points[self.cursor..end], &owned);
            }
        }
        self.last_scored = end - self.cursor;
        self.cursor = end;
        let mut best: Option<EqPoint> = None;
        for p in self.points.iter().take(self.cursor) {
            match best {
                None => best = Some(*p),
                Some(b) if p.score > b.score => best = Some(*p),
                _ => {}
            }
        }
        self.head = best;
        Ok(self.head)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::eqs::{
        AlwaysVisible, CoverSpotGenerator, GridGenerator, PointListGenerator, RingGenerator,
        Tactics,
    };

    const THREAT: Vec3Mm = Vec3Mm::new(10_000, 0, 0);
    const SELF: Vec3Mm = Vec3Mm::new(-8_000, 0, 0);

    fn ctx() -> EqContext {
        EqContext::new(SELF, THREAT)
    }

    fn drive(r: &mut SlicedRunner<'_>, ctx: &EqContext) -> (Option<EqPoint>, usize) {
        let mut max = 0;
        let mut head = None;
        while !r.done() {
            head = r.tick(ctx).expect("slice scores");
            max = max.max(r.last_scored());
        }
        (head, max)
    }

    #[test]
    fn eqs_sliced_cover_flank() {
        // (a) 4-ring + refused huge-grid (skipped) + 9-grid = 13 chained.
        let ring = RingGenerator::new(4_000, 4).expect("ring is legal");
        let grid = GridGenerator::new(2_000, 2_000).expect("grid is legal");
        let huge = GridGenerator::new(8_000, 1_000).expect("huge builds");
        let chain = CompositeGenerator::new()
            .with_arm(Generator::Ring(ring))
            .expect("arm 1")
            .with_arm(Generator::Grid(huge))
            .expect("arm 2")
            .with_arm(Generator::Grid(grid))
            .expect("arm 3");
        let mut out = Vec::new();
        chain.generate_into(&ctx(), &mut out).expect("chain runs");
        assert_eq!(out.len(), 13, "refused arm skips, order kept");
        assert_eq!(out[0].pos, Vec3Mm::new(-4_000, 0, 0));
        assert_eq!(out[4].pos, Vec3Mm::new(-10_000, 0, -2_000));
        let sensor_spots = [
            Vec3Mm::new(1_000, 0, 1_000),
            Vec3Mm::new(2_000, 0, 2_000),
            Vec3Mm::new(3_000, 0, 3_000),
            Vec3Mm::new(4_000, 0, 4_000),
        ];
        SensorArrayGenerator::new(&sensor_spots)
            .generate_into(&mut out)
            .expect("sensor appends");
        assert_eq!(out.len(), 17, "13 chained + 4 sensor");
        assert_eq!(out[16].pos, Vec3Mm::new(4_000, 0, 4_000));
        // (b) cover query (8 covers) sliced at 8/tick matches one-shot.
        let covers = [
            Vec3Mm::new(-6_000, 0, 0),
            Vec3Mm::new(-4_000, 0, 2_000),
            Vec3Mm::new(-2_000, 0, -2_000),
            Vec3Mm::new(0, 0, 0),
            Vec3Mm::new(2_000, 0, 2_000),
            Vec3Mm::new(4_000, 0, -2_000),
            Vec3Mm::new(5_000, 0, 0),
            Vec3Mm::new(6_000, 0, 2_000),
        ];
        let cover_q = Tactics::build_cover_query(&covers, 2_000, 30_000, 0).expect("cover builds");
        let mut scratch = Vec::new();
        let cover_winner = cover_q.best_into(&ctx(), &mut scratch).expect("cover runs");
        let mut cover_run = SlicedRunner::new(cover_q.clone(), 8).expect("sliced builds");
        let (cover_head, cover_max) = drive(&mut cover_run, &ctx());
        assert_eq!(cover_head, cover_winner, "sliced cover matches one-shot");
        assert!(cover_max <= 8, "cover slice cap holds");
        // (b) flank query (64 spots) sliced at 8/tick matches one-shot.
        let mut spots = Vec::with_capacity(64);
        for ix in 0..8 {
            for iz in 0..8 {
                spots.push(Vec3Mm::new(
                    -8_000 + ix * 2_000 - 7_000,
                    0,
                    iz * 2_000 - 7_000,
                ));
            }
        }
        let vis = AlwaysVisible;
        let flank_q = Tactics::build_ambush_query(&spots, &vis, Vec3Mm::new(1000, 0, 0), 30_000, 2)
            .expect("flank builds");
        let mut fscratch = Vec::new();
        let flank_winner = flank_q
            .best_into(&ctx(), &mut fscratch)
            .expect("flank runs");
        let mut flank_run = SlicedRunner::new(flank_q.clone(), 8).expect("sliced builds");
        let (flank_head, flank_max) = drive(&mut flank_run, &ctx());
        assert_eq!(flank_head, flank_winner, "sliced flank matches one-shot");
        assert!(flank_max <= 8, "flank slice cap holds");
        assert_eq!(
            flank_run.digest(),
            flank_run.digest(),
            "digest x2 identical"
        );
        // (c) replay from reset gives identical winner and digest.
        let d1 = flank_run.digest();
        flank_run.reset();
        let (again_head, _) = drive(&mut flank_run, &ctx());
        assert_eq!(again_head, flank_winner, "replay winner identical");
        assert_eq!(flank_run.digest(), d1, "replay digest identical");
        // (d) fresh 64-point run at 8/tick takes 8 ticks, none over budget.
        let mut capped = SlicedRunner::new(flank_q, 8).expect("sliced builds");
        let mut ticks = 0;
        while !capped.done() {
            capped.tick(&ctx()).expect("tick scores");
            assert!(capped.last_scored() <= 8, "no tick exceeds budget");
            ticks += 1;
        }
        assert_eq!(ticks, 8, "64 points at 8/tick take 8 ticks");
    }

    #[test]
    fn eqs_slice_refusals_stay_typed() {
        let tiny = RingGenerator::new(1_000, 4).expect("ring is legal");
        assert!(
            SlicedRunner::new(EqQuery::new(Generator::Ring(tiny)), 0).is_err(),
            "zero budget refuses"
        );
        let full = CompositeGenerator::new()
            .with_arm(Generator::Ring(tiny))
            .expect("1")
            .with_arm(Generator::Grid(
                GridGenerator::new(1_000, 1_000).expect("grid is legal"),
            ))
            .expect("2")
            .with_arm(Generator::List(PointListGenerator::new(&[])))
            .expect("3")
            .with_arm(Generator::Cover(
                CoverSpotGenerator::new(&[], 1_000, 0).expect("cover builds"),
            ))
            .expect("4");
        assert!(full.len() == 4, "four arms fit");
        assert!(
            full.with_arm(Generator::Ring(tiny)).is_err(),
            "fifth arm refuses"
        );
    }

    #[test]
    fn wired_empty_helpers_skip_and_refuse() {
        // Behaviour: new, default and len agree on empty; guards emit nothing.
        let fresh = CompositeGenerator::default();
        assert!(fresh.is_empty());
        assert_eq!((CompositeGenerator::new().len(), fresh.len()), (0, 0));
        let mut out = Vec::new();
        fresh
            .generate_into(&ctx(), &mut out)
            .expect("empty chain emits nothing");
        assert!(out.is_empty());
        let sensor = SensorArrayGenerator::new(&[]);
        assert!(sensor.is_empty());
        sensor
            .generate_into(&mut out)
            .expect("empty sensor emits nothing");
        assert!(out.is_empty());
        // Failure: a live sensor past the point cap refuses typed, while an
        // empty one still skips the cap check.
        let spots = [Vec3Mm::new(1_000, 0, 0)];
        let live = SensorArrayGenerator::new(&spots);
        assert!(!live.is_empty());
        let mut packed = vec![EqPoint::new(Vec3Mm::new(0, 0, 0)); MAX_POINTS];
        assert!(matches!(
            live.generate_into(&mut packed),
            Err(EqError::TooManyPoints { .. })
        ));
        assert!(
            sensor.generate_into(&mut packed).is_ok(),
            "empty skips the cap"
        );
    }
}
