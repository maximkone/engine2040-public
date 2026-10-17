//! dispatch_plan: an explicit stage plan for bake/verify with barriers.
//!
//! Owner: engine-ecs. Caller: bake and verify lanes describe their stages
//! once as a [`DispatchPlan`] and run it through [`DispatchPlan::run_par`]
//! (the pool), [`DispatchPlan::run_seq`] (the sequential fallback) or
//! [`DispatchPlan::run_checked`] (both, refusing on the first diverging
//! stage). The sim tick never calls in. Pattern: the shred dispatcher
//! (stages ordered by explicit barriers), reimplemented std-only.
//!
//! Units: items per stage (one item = one `run` call) and chunk items per job.
//! Determinism: a barrier closes every stage: all items finish, their outputs
//! sit in item order ([`collect_indexed_on`]) and fold on the caller before
//! the next stage starts, so a stage reads the whole previous result through
//! `&Acc` and thread count, chunking and finish order never enter state. The
//! sequential fallback runs the same items in index order and must hash equal
//! at every barrier. Errors: [`PlanError`]; empty plans, empty or repeated
//! stages and any seq/par digest gap refuse, naming the stage.
//! Spans: one PF2 span per stage, keyed `ecs.dispatch.<stage>`, reported
//! through a caller hook (the profiler crate depends on this one, so the key
//! is a plain string joined operator-side).

use std::fmt::Write as _;
use std::ops::Range;

use crate::jobs::ChunkSize;
use crate::parallel::{collect_indexed_on, JobsError};
use crate::pool::JobPool;

/// One stage: `items` independent work items run `chunk` per job.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stage {
    /// Stable stage name (also the span key suffix).
    pub name: &'static str,
    /// Work items in the stage, at least 1.
    pub items: usize,
    /// Items per pool job.
    pub chunk: ChunkSize,
}

impl Stage {
    /// Pool jobs the stage plans: `ceil(items / chunk)`.
    pub fn jobs(&self) -> usize {
        self.items.div_ceil(self.chunk.value())
    }

    /// PF2 span key of the stage.
    pub fn span_key(&self) -> String {
        format!("ecs.dispatch.{}", self.name)
    }

    /// One dump line; the plan and the executed trace both print this shape.
    fn line(&self, index: usize) -> String {
        format!(
            "stage {index} {} items={} chunk={} jobs={} span={}",
            self.name,
            self.items,
            self.chunk.value(),
            self.jobs(),
            self.span_key()
        )
    }
}

/// Which side of a stage a span hook call marks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpanEdge {
    /// The stage starts.
    Enter,
    /// The stage and its barrier finished (or the stage failed).
    Exit,
}

/// Why a plan refused to build or run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PlanError {
    /// The plan holds no stage.
    NoStages,
    /// A stage with zero items.
    EmptyStage(&'static str),
    /// Two stages share a name (their spans would collide).
    DuplicateStage(&'static str),
    /// The pool collect failed.
    Jobs(JobsError),
    /// Sequential and parallel digests differ after a barrier.
    Mismatch {
        /// Index of the first diverging stage.
        stage: usize,
        /// Name of that stage.
        name: &'static str,
        /// Digest after the barrier on the sequential path.
        seq: u64,
        /// Digest after the barrier on the parallel path.
        par: u64,
    },
}

impl core::fmt::Display for PlanError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NoStages => write!(f, "dispatch plan holds no stage"),
            Self::EmptyStage(name) => write!(f, "dispatch stage {name} has no items"),
            Self::DuplicateStage(name) => write!(f, "dispatch stage {name} appears twice"),
            Self::Jobs(e) => write!(f, "dispatch stage failed: {e}"),
            Self::Mismatch {
                stage,
                name,
                seq,
                par,
            } => write!(
                f,
                "dispatch stage {stage} {name} diverged: seq {seq:016x} par {par:016x}"
            ),
        }
    }
}

impl std::error::Error for PlanError {}

impl From<JobsError> for PlanError {
    fn from(e: JobsError) -> Self {
        Self::Jobs(e)
    }
}

/// The work a plan drives. `Acc` is the state between barriers: stage `k`
/// reads the folded result of stages `0..k` through `&Acc`.
pub trait StageWork: Sync {
    /// State carried across barriers.
    type Acc: Sync;
    /// One item's output.
    type Out: Send;

    /// Run item `item` of `stage` against the state after the last barrier.
    fn run(&self, acc: &Self::Acc, stage: usize, item: usize) -> Self::Out;

    /// The barrier fold: all of the stage's outputs in item order.
    fn fold(&self, acc: Self::Acc, stage: usize, outs: Vec<Self::Out>) -> Self::Acc;

    /// Digest of the state after a barrier (compared seq against par).
    fn digest(&self, acc: &Self::Acc) -> u64;
}

/// One executed stage, the shape [`DispatchPlan::dump`] prints.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StageRun {
    /// The stage that ran.
    pub stage: Stage,
    /// State digest after its barrier.
    pub digest: u64,
}

/// What a run executed, in order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Trace {
    /// Executed stages in run order.
    pub runs: Vec<StageRun>,
}

impl Trace {
    /// Printed trace; equals [`DispatchPlan::dump`] when execution followed
    /// the plan.
    pub fn dump(&self) -> String {
        dump_stages(self.runs.iter().map(|r| &r.stage))
    }
}

/// Final state plus the executed trace.
#[derive(Debug)]
pub struct Run<A> {
    /// State after the last barrier.
    pub acc: A,
    /// Stages executed.
    pub trace: Trace,
}

/// Print stages with a barrier line between neighbours.
fn dump_stages<'a>(stages: impl Iterator<Item = &'a Stage>) -> String {
    let mut out = String::new();
    for (index, stage) in stages.enumerate() {
        if index > 0 {
            let _ = writeln!(out, "barrier");
        }
        let _ = writeln!(out, "{}", stage.line(index));
    }
    out
}

/// Ordered stages separated by barriers.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DispatchPlan {
    stages: Vec<Stage>,
}

impl DispatchPlan {
    /// Empty plan.
    pub fn new() -> Self {
        Self::default()
    }

    /// Append a stage. Refuses zero items and a repeated name.
    pub fn stage(
        mut self,
        name: &'static str,
        items: usize,
        chunk: ChunkSize,
    ) -> Result<Self, PlanError> {
        if items == 0 {
            return Err(PlanError::EmptyStage(name));
        }
        if self.stages.iter().any(|s| s.name == name) {
            return Err(PlanError::DuplicateStage(name));
        }
        self.stages.push(Stage { name, items, chunk });
        Ok(self)
    }

    /// Stages in run order.
    pub fn stages(&self) -> &[Stage] {
        &self.stages
    }

    /// Printed plan: one line per stage, a `barrier` line between stages.
    pub fn dump(&self) -> String {
        dump_stages(self.stages.iter())
    }

    /// Run every stage on `pool`; item outputs gather by slot and fold at
    /// each barrier. `spans` sees `Enter` then `Exit` around every stage.
    pub fn run_par<W: StageWork>(
        &self,
        pool: &JobPool,
        work: &W,
        init: W::Acc,
        spans: &mut dyn FnMut(&str, SpanEdge),
    ) -> Result<Run<W::Acc>, PlanError> {
        self.execute(work, init, spans, |stage, index, acc| {
            collect_indexed_on(pool, 0..stage.items, stage.chunk, |item| {
                work.run(acc, index, item)
            })
        })
    }

    /// The sequential fallback: the same stages and barriers, every item on
    /// the caller in index order. Byte-equal to [`DispatchPlan::run_par`].
    pub fn run_seq<W: StageWork>(
        &self,
        work: &W,
        init: W::Acc,
        spans: &mut dyn FnMut(&str, SpanEdge),
    ) -> Result<Run<W::Acc>, PlanError> {
        self.execute(work, init, spans, |stage, index, acc| {
            let items: Range<usize> = 0..stage.items;
            Ok(items.map(|item| work.run(acc, index, item)).collect())
        })
    }

    /// Run sequentially, then on `pool`, and refuse at the first stage whose
    /// barrier digests differ, returning the parallel run otherwise.
    pub fn run_checked<W: StageWork>(
        &self,
        pool: &JobPool,
        work: &W,
        init: W::Acc,
        spans: &mut dyn FnMut(&str, SpanEdge),
    ) -> Result<Run<W::Acc>, PlanError>
    where
        W::Acc: Clone,
    {
        let seq = self.run_seq(work, init.clone(), &mut |_, _| {})?;
        let par = self.run_par(pool, work, init, spans)?;
        for (index, (s, p)) in seq.trace.runs.iter().zip(&par.trace.runs).enumerate() {
            if s.digest != p.digest {
                return Err(PlanError::Mismatch {
                    stage: index,
                    name: s.stage.name,
                    seq: s.digest,
                    par: p.digest,
                });
            }
        }
        Ok(par)
    }

    /// Shared stage loop: span in, gather, barrier fold, digest, span out.
    fn execute<W: StageWork>(
        &self,
        work: &W,
        init: W::Acc,
        spans: &mut dyn FnMut(&str, SpanEdge),
        gather: impl Fn(&Stage, usize, &W::Acc) -> Result<Vec<W::Out>, JobsError>,
    ) -> Result<Run<W::Acc>, PlanError> {
        if self.stages.is_empty() {
            return Err(PlanError::NoStages);
        }
        let mut acc = init;
        let mut runs = Vec::with_capacity(self.stages.len());
        for (index, stage) in self.stages.iter().enumerate() {
            let key = stage.span_key();
            spans(&key, SpanEdge::Enter);
            let outs = match gather(stage, index, &acc) {
                Ok(outs) => outs,
                Err(e) => {
                    spans(&key, SpanEdge::Exit);
                    return Err(e.into());
                }
            };
            acc = work.fold(acc, index, outs);
            runs.push(StageRun {
                stage: *stage,
                digest: work.digest(&acc),
            });
            spans(&key, SpanEdge::Exit);
        }
        Ok(Run {
            acc,
            trace: Trace { runs },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_core::{hash_u64s, CounterRng};
    use std::sync::atomic::{AtomicUsize, Ordering};

    const SEED: u64 = 2040;

    /// State between barriers of the fixture.
    #[derive(Clone, Debug, PartialEq, Eq, Default)]
    struct Words {
        baked: Vec<u64>,
        checks: Vec<u64>,
        total: u64,
        pass: u64,
    }

    /// bake -> verify -> seal. Verify reads the whole bake result, so a
    /// missing barrier would index an empty vector and panic.
    struct Fixture {
        folds: AtomicUsize,
        skew_verify: bool,
    }

    impl Fixture {
        fn new() -> Self {
            Self {
                folds: AtomicUsize::new(0),
                skew_verify: false,
            }
        }
    }

    impl StageWork for Fixture {
        type Acc = Words;
        type Out = u64;

        fn run(&self, acc: &Words, stage: usize, item: usize) -> u64 {
            match stage {
                0 => CounterRng::new(SEED).sample(item as u64),
                1 => acc.baked[item] ^ acc.baked[(item + 1) % acc.baked.len()],
                _ => acc.checks[item % acc.checks.len()].rotate_left(item as u32 % 64),
            }
        }

        fn fold(&self, mut acc: Words, stage: usize, outs: Vec<u64>) -> Words {
            match stage {
                0 => acc.baked = outs,
                1 => {
                    acc.checks = outs;
                    if self.skew_verify {
                        // Seq runs first and par second: a pass-dependent
                        // fold makes the two paths diverge here, on purpose.
                        acc.pass = self.folds.fetch_add(1, Ordering::SeqCst) as u64;
                    }
                }
                _ => acc.total = outs.iter().fold(0, |a, x| a.wrapping_add(*x)),
            }
            acc
        }

        fn digest(&self, acc: &Words) -> u64 {
            hash_u64s(&[
                hash_u64s(&acc.baked),
                hash_u64s(&acc.checks),
                acc.total,
                acc.pass,
            ])
        }
    }

    fn fixture_plan(chunk: usize) -> DispatchPlan {
        DispatchPlan::new()
            .stage("bake", 1000, ChunkSize::new(chunk))
            .and_then(|p| p.stage("verify", 1000, ChunkSize::new(chunk)))
            .and_then(|p| p.stage("seal", 64, ChunkSize::new(chunk)))
            .expect("three distinct stages")
    }

    /// Hand-rolled oracle: the fixture with plain loops and no plan.
    fn oracle() -> Words {
        let baked: Vec<u64> = (0..1000u64)
            .map(|i| CounterRng::new(SEED).sample(i))
            .collect();
        let checks: Vec<u64> = (0..1000)
            .map(|i| baked[i] ^ baked[(i + 1) % baked.len()])
            .collect();
        let total = (0..64usize)
            .map(|i| checks[i % checks.len()].rotate_left(i as u32 % 64))
            .fold(0, |a: u64, x| a.wrapping_add(x));
        Words {
            baked,
            checks,
            total,
            pass: 0,
        }
    }

    #[test]
    fn plan_dump_matches_execution_on_fixtures() {
        let want = "\
stage 0 bake items=1000 chunk=128 jobs=8 span=ecs.dispatch.bake
barrier
stage 1 verify items=1000 chunk=128 jobs=8 span=ecs.dispatch.verify
barrier
stage 2 seal items=64 chunk=128 jobs=1 span=ecs.dispatch.seal
";
        let plan = fixture_plan(128);
        assert_eq!(plan.dump(), want);
        let work = Fixture::new();
        let pool = JobPool::new(4).expect("pool starts");
        let par = plan
            .run_par(&pool, &work, Words::default(), &mut |_, _| {})
            .expect("parallel run");
        let seq = plan
            .run_seq(&work, Words::default(), &mut |_, _| {})
            .expect("sequential run");
        assert_eq!(par.trace.dump(), plan.dump(), "execution followed the plan");
        assert_eq!(seq.trace.dump(), plan.dump());
        // Not vacuous: a different plan prints differently.
        assert_ne!(fixture_plan(7).dump(), plan.dump());
    }

    #[test]
    fn seq_bytes_equal_par_bytes_twice_across_workers_and_chunks() {
        let want = oracle();
        for workers in [0, 1, 2, 4, 8] {
            let pool = JobPool::new(workers).expect("pool starts");
            for chunk in [1, 7, 128, 5000] {
                let plan = fixture_plan(chunk);
                let work = Fixture::new();
                let first = plan
                    .run_checked(&pool, &work, Words::default(), &mut |_, _| {})
                    .expect("seq equals par");
                let second = plan
                    .run_checked(&pool, &work, Words::default(), &mut |_, _| {})
                    .expect("seq equals par again");
                assert_eq!(first.acc, want, "workers {workers} chunk {chunk}");
                assert_eq!(first.acc, second.acc, "run twice");
                assert_eq!(first.trace, second.trace);
            }
        }
    }

    #[test]
    fn a_digest_gap_refuses_naming_the_stage() {
        let pool = JobPool::new(2).expect("pool starts");
        let work = Fixture {
            folds: AtomicUsize::new(0),
            skew_verify: true,
        };
        let err = fixture_plan(64)
            .run_checked(&pool, &work, Words::default(), &mut |_, _| {})
            .expect_err("skewed verify must refuse");
        match err {
            PlanError::Mismatch {
                stage,
                name,
                seq,
                par,
            } => {
                assert_eq!((stage, name), (1, "verify"));
                assert_ne!(seq, par);
            }
            other => panic!("wrong refusal: {other}"),
        }
        let text = err.to_string();
        assert!(text.contains("stage 1 verify diverged"), "{text}");
        assert!(text.contains("seq ") && text.contains("par "), "{text}");
    }

    #[test]
    fn one_span_per_stage_in_barrier_order() {
        let pool = JobPool::new(2).expect("pool starts");
        let work = Fixture::new();
        let mut seen: Vec<(String, SpanEdge)> = Vec::new();
        fixture_plan(100)
            .run_par(&pool, &work, Words::default(), &mut |key, edge| {
                seen.push((key.to_string(), edge));
            })
            .expect("run");
        let want: Vec<(String, SpanEdge)> = ["bake", "verify", "seal"]
            .iter()
            .flat_map(|n| {
                let key = format!("ecs.dispatch.{n}");
                [(key.clone(), SpanEdge::Enter), (key, SpanEdge::Exit)]
            })
            .collect();
        assert_eq!(seen, want);
    }

    #[test]
    fn bad_plans_refuse_loudly() {
        assert_eq!(
            DispatchPlan::new().stage("a", 0, ChunkSize::new(4)).err(),
            Some(PlanError::EmptyStage("a"))
        );
        let twice = DispatchPlan::new()
            .stage("a", 4, ChunkSize::new(4))
            .and_then(|p| p.stage("a", 4, ChunkSize::new(4)));
        assert_eq!(twice.err(), Some(PlanError::DuplicateStage("a")));
        let work = Fixture::new();
        let none = DispatchPlan::new().run_seq(&work, Words::default(), &mut |_, _| {});
        assert_eq!(none.err(), Some(PlanError::NoStages));
    }
}
