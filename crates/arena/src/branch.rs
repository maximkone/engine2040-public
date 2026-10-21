//! branch: fork-ID plumbing and recorded branch-merge receipts over the
//! frozen [`Fork`] API (AX12).
//!
//! Consumer: speculative level edits - one prefix snapshot, N tagged branches
//! opened off it, all run together, one receipt per branch. The MCP `sim.fork`
//! door (MCP4) reads the receipt vector once this row is green; balance agents
//! read the same set for 20-way sweeps.
//! Data owner: [`BranchSet`] owns the branch list, [`speculate`](BranchSet::speculate)
//! owns the receipt vector and publishes it only when every branch came back
//! (merge atomic: no partial set, no zeroed run read as a clean one).
//! Units: fixed ticks ([`TICK_HZ`](crate::TICK_HZ)), fighter slots, permille.
//! [`ForkId`] is a dense slot number inside its set; [`IntervalTag`] is the
//! half-open match-tick span `from..to` a receipt covers.
//!
//! The tags are bolt-on: [`Fork`] carries no id and no span, so the caller
//! passes the snapshot tick it forked at and the tags ride beside it. The fork
//! API stays frozen.
//! Determinism: the same fork plus the same branch set twice gives equal
//! receipts, whatever worker ran which branch; results land in branch order,
//! never completion order.
//! Error policy: a refused proposal, a crashed worker or a bad shape refuses
//! the whole set as [`BranchError`]; nothing partial is ever returned.
//! Threading: one scope sized to the core count, each worker running a
//! contiguous run of branches into its own slots (the [`vec_step`](crate::vec_step)
//! shape, so no slot needs a lock), and every branch running the frozen
//! [`Fork::try_run_n`] path with its own fork clone. The fixed tick never
//! waits on a branch set.

use super::fork::{Fork, ForkError, ForkMetrics, ForkProposal};
use engine_core::Fnv64;

/// Branch slot id inside one [`BranchSet`]: dense from zero in set order, so a
/// receipt names its branch without carrying any fork state.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct ForkId(u16);

impl ForkId {
    /// Id for slot `index`, or `None` over the `u16` cap. A set that large is
    /// a caller bug, never a branch count, so it is refused instead of wrapped.
    #[must_use]
    pub fn new(index: usize) -> Option<Self> {
        u16::try_from(index).ok().map(Self)
    }

    /// The slot number, always exact: the id was built from it.
    #[must_use]
    pub fn slot(self) -> usize {
        usize::from(self.0)
    }
}

impl std::fmt::Display for ForkId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "fork{}", self.0)
    }
}

/// Half-open match-tick span a branch receipt covers: `from..to`, absolute
/// ticks. Bolt-on, so the frozen [`Fork`] API never learns about it.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct IntervalTag {
    /// Branch this span belongs to.
    pub fork: ForkId,
    /// First tick of the window (the snapshot tick the branch forked at).
    pub from: u32,
    /// One past the last tick of the window.
    pub to: u32,
}

impl IntervalTag {
    /// Ticks the span holds.
    #[must_use]
    pub fn len(self) -> u32 {
        self.to.saturating_sub(self.from)
    }

    /// True when the span holds no tick (a window that ran nothing).
    #[must_use]
    pub fn is_empty(self) -> bool {
        self.to <= self.from
    }

    /// True when `tick` sits inside the span.
    #[must_use]
    pub fn covers(self, tick: u32) -> bool {
        tick >= self.from && tick < self.to
    }
}

/// One speculative branch: its fork id plus the proposals queued on the fork
/// before the window runs, in order.
#[derive(Clone, PartialEq, Debug)]
pub struct Branch {
    /// Id this branch answers to inside its set.
    pub fork: ForkId,
    /// Proposals applied to the branch's fork clone before its first tick.
    pub proposals: Vec<ForkProposal>,
}

impl Branch {
    /// Branch `fork` applying `proposals` before it runs.
    #[must_use]
    pub fn new(fork: ForkId, proposals: Vec<ForkProposal>) -> Self {
        Self { fork, proposals }
    }
}

/// A recorded branch receipt: the interval tag plus the metrics the branch
/// produced. Two runs of the same branch set give equal receipts.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct BranchReceipt {
    /// Window this branch ran, tagged with its fork id.
    pub tag: IntervalTag,
    /// Metrics the branch produced, in fighter-slot order.
    pub metrics: ForkMetrics,
}

impl BranchReceipt {
    /// Deterministic digest over the tag and the metrics: integer bytes only,
    /// so a recorded receipt replays and compares on any machine.
    #[must_use]
    pub fn digest(&self) -> u64 {
        let mut hasher = Fnv64::new();
        hasher.write_u16(self.tag.fork.0);
        hasher.write_u32(self.tag.from);
        hasher.write_u32(self.tag.to);
        hasher.write_len(self.metrics.damage_dealt.len());
        for (slot, damage) in &self.metrics.damage_dealt {
            hasher.write_u8(*slot);
            hasher.write_u16(*damage);
        }
        match self.metrics.ticks_to_kill {
            Some(ticks) => {
                hasher.write_u8(1);
                hasher.write_u32(ticks);
            }
            None => hasher.write_u8(0),
        }
        hasher.write_len(self.metrics.stuck_ticks.len());
        for (slot, stuck) in &self.metrics.stuck_ticks {
            hasher.write_u8(*slot);
            hasher.write_u32(*stuck);
        }
        hasher.finish()
    }
}

/// Why a branch set was refused. Every variant names the culprit; a refusal
/// publishes no receipt at all.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BranchError {
    /// A branch set with no branch: an empty merge is refused, never an empty
    /// success that reads as a clean sweep.
    EmptySet,
    /// Branch at `index` does not answer to id `index`: ids must be dense from
    /// zero in set order, or two receipts could tag alike.
    SparseFork {
        /// Position in the set.
        index: usize,
        /// Id the branch carried there.
        fork: ForkId,
    },
    /// Window `from..to` holds no tick.
    EmptyWindow {
        /// Offered window start.
        from: u32,
        /// Offered window end.
        to: u32,
    },
    /// One branch slot stayed empty after every worker joined; impossible by
    /// construction, kept as a loud error so a coverage hole can never ship as
    /// a short receipt vector.
    Uncovered {
        /// Position in the set that was never written.
        index: usize,
    },
    /// The fork refused one branch proposal; the whole set is refused.
    Refused {
        /// Branch whose proposal was refused.
        fork: ForkId,
        /// The fork's own typed error.
        error: ForkError,
    },
    /// The branch's fork run crashed: the fork worker died, so the run holds
    /// no metrics. Its zeros are not a run, so nothing merges.
    Crashed(ForkId),
}

impl std::fmt::Display for BranchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptySet => f.write_str("branch set holds no branch"),
            Self::SparseFork { index, fork } => {
                write!(f, "branch {index} carries {fork}; ids run dense from fork0")
            }
            Self::EmptyWindow { from, to } => {
                write!(f, "branch window {from}..{to} holds no tick")
            }
            Self::Uncovered { index } => {
                write!(f, "branch {index} left its slot unwritten")
            }
            Self::Refused { fork, error } => {
                write!(f, "{fork} refused a proposal: {error:?}")
            }
            Self::Crashed(fork) => write!(f, "{fork} worker panicked; no receipt"),
        }
    }
}

impl std::error::Error for BranchError {}

/// A dense branch set over one window, ready to run. Built with
/// [`BranchSet::new`] (which refuses a bad shape once), run with
/// [`speculate`](BranchSet::speculate).
#[derive(Clone, PartialEq, Debug)]
pub struct BranchSet {
    branches: Vec<Branch>,
}

impl BranchSet {
    /// Check the set shape: at least one branch, and branch `index` answers to
    /// [`ForkId::new`]`(index)`. A sparse or duplicated id would tag two
    /// receipts alike, so it is refused here rather than merged later.
    pub fn new(branches: Vec<Branch>) -> Result<Self, BranchError> {
        if branches.is_empty() {
            return Err(BranchError::EmptySet);
        }
        for (index, branch) in branches.iter().enumerate() {
            if Some(branch.fork) != ForkId::new(index) {
                return Err(BranchError::SparseFork {
                    index,
                    fork: branch.fork,
                });
            }
        }
        Ok(Self { branches })
    }

    /// Branches in the set, in id order.
    #[must_use]
    pub fn len(&self) -> usize {
        self.branches.len()
    }

    /// True when the set holds no branch; [`BranchSet::new`] never builds one.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.branches.is_empty()
    }

    /// The branch list, in id order.
    #[must_use]
    pub fn branches(&self) -> &[Branch] {
        &self.branches
    }

    /// Run every branch off `fork` over `from..to` and merge the receipts
    /// atomically: either one receipt per branch in id order, or one
    /// [`BranchError`] and nothing published.
    ///
    /// `from` must be the tick the fork was taken at (the argument
    /// [`Fork::from_snapshot`] accepted); the tags record that window. Each
    /// branch runs its own fork clone, so the caller's fork is untouched and
    /// the live match never moves.
    pub fn speculate(
        &self,
        fork: &Fork,
        from: u32,
        to: u32,
    ) -> Result<Vec<BranchReceipt>, BranchError> {
        if self.branches.is_empty() {
            return Err(BranchError::EmptySet);
        }
        if to <= from {
            return Err(BranchError::EmptyWindow { from, to });
        }
        let ticks = to - from;
        // One scope sized to the core count, each worker owning a contiguous
        // run of branches and their slots, so receipts land in id order with no
        // lock and completion order never decides a byte.
        let count = self.branches.len();
        let mut outcomes: Vec<Option<Result<ForkMetrics, BranchFailure>>> = Vec::new();
        outcomes.resize_with(count, || None);
        let workers = std::thread::available_parallelism()
            .map_or(1, std::num::NonZeroUsize::get)
            .clamp(1, count);
        let chunk = count.div_ceil(workers);
        std::thread::scope(|scope| {
            for (batch, slots) in self.branches.chunks(chunk).zip(outcomes.chunks_mut(chunk)) {
                scope.spawn(move || {
                    for (branch, slot) in batch.iter().zip(slots.iter_mut()) {
                        *slot = Some(run_branch(fork, ticks, branch));
                    }
                });
            }
        });
        // Merge: one refusal drops the whole set, so a caller never reads a
        // short vector as a full sweep.
        let mut receipts = Vec::with_capacity(count);
        for (index, outcome) in outcomes.into_iter().enumerate() {
            let tag = IntervalTag {
                fork: self.branches[index].fork,
                from,
                to,
            };
            let Some(outcome) = outcome else {
                return Err(BranchError::Uncovered { index });
            };
            let metrics = match outcome {
                Ok(metrics) => metrics,
                Err(BranchFailure::Refused(error)) => {
                    return Err(BranchError::Refused {
                        fork: tag.fork,
                        error,
                    })
                }
                Err(BranchFailure::Crashed) => return Err(BranchError::Crashed(tag.fork)),
            };
            receipts.push(BranchReceipt { tag, metrics });
        }
        Ok(receipts)
    }
}

/// Why one branch produced no metrics. Kept private: the set maps it onto
/// [`BranchError`] with the branch's id attached.
enum BranchFailure {
    /// A proposal the fork refused before the first tick.
    Refused(ForkError),
    /// The fork's own worker crashed while running the window.
    Crashed,
}

/// Clone the caller's fork, queue this branch's proposals, run the window.
fn run_branch(fork: &Fork, ticks: u32, branch: &Branch) -> Result<ForkMetrics, BranchFailure> {
    let mut fork = fork.clone();
    for proposal in &branch.proposals {
        fork.apply_proposal(proposal.clone())
            .map_err(BranchFailure::Refused)?;
    }
    fork.try_run_n(ticks).map_err(|_| BranchFailure::Crashed)
}

/// One measured branch-set report: the two ways to produce the same receipts
/// and how far apart they land. Consumer: the AX12 timing probe and the MCP4
/// `sim.fork` receipt line; [`BranchReport::text`] is the one-line record.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct BranchReport {
    /// Branches in the set.
    pub branches: usize,
    /// Receipts the merge published.
    pub merged: usize,
    /// Ticks each branch ran.
    pub window: u32,
    /// Ticks of prefix the fork path replays once for the whole set.
    pub prefix: u32,
    /// Naive path: one prefix replay per branch plus its run, milliseconds.
    pub serial_ms: f64,
    /// Fork path: one prefix plus the parallel branch set, milliseconds.
    pub branch_ms: f64,
    /// `serial_ms / branch_ms`: the create speedup of the fork path.
    pub speedup_x: f64,
    /// Window-only factor: the same windows run one after another against
    /// one fork, over the parallel set. The shared prefix is out of it, so
    /// this is the parallel part alone.
    pub parallel_x: f64,
    /// True when a second run gave equal receipts.
    pub x2_equal: bool,
}

impl BranchReport {
    /// One-line record of the report: branches, merge count, window, speedup,
    /// parallel factor and the repeat check.
    #[must_use]
    pub fn text(&self) -> String {
        format!(
            "branches={} merged={}/{} window={} prefix={} speedup={:.1}x parallel={:.1}x \
             serial_ms={:.1} branch_ms={:.1} x2_equal={}",
            self.branches,
            self.merged,
            self.branches,
            self.window,
            self.prefix,
            self.speedup_x,
            self.parallel_x,
            self.serial_ms,
            self.branch_ms,
            self.x2_equal
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{run_prefix, ArenaConfig, FighterId, FighterInput, FighterTape};
    use engine_input::fighter::{Buttons, ATTACK};

    fn config() -> ArenaConfig {
        ArenaConfig {
            seed: 2040,
            ticks: 600,
            fighters: 2,
            half_extent_m: 20.0,
            table_version: engine_combat::tables::TABLES_VERSION_V1,
        }
    }

    /// Fork the neutral prefix at `ticks`.
    fn fork_at(ticks: u32) -> Fork {
        let (snapshot, _) =
            run_prefix(&config(), &FighterTape::default(), ticks).expect("valid prefix");
        Fork::from_snapshot(snapshot, ticks).expect("tick matches snapshot")
    }

    fn id(slot: usize) -> ForkId {
        ForkId::new(slot).expect("test sets stay under the u16 cap")
    }

    /// Relentless attack script for one fighter over the fork window.
    fn attack_spam(fighter: u8, base: u32, steps: u32) -> Vec<FighterInput> {
        let id = FighterId::new(fighter).expect("course uses slots 0..2");
        (0..steps)
            .map(|index| FighterInput {
                tick: base + index,
                fighter: id,
                mx: 0,
                mz: 0,
                facing: 0,
                buttons: Buttons(ATTACK),
            })
            .collect()
    }

    /// 20 distinct branches: sixteen morale rungs plus four frozen pairs, so
    /// the set holds more than one outcome without depending on luck. The
    /// swapped tape starts at `from`, the window the branch forks at.
    fn twenty(from: u32) -> Vec<Branch> {
        (0..20)
            .map(|slot| {
                let proposals = match slot {
                    0..=15 => vec![ForkProposal::SetAggression {
                        fighter: u8::try_from(slot % 2).expect("slots 0..2"),
                        permille: u16::try_from(slot * 60).expect("under 1001"),
                    }],
                    16..=19 => vec![
                        ForkProposal::FreezeFighter { fighter: 0 },
                        ForkProposal::FreezeFighter { fighter: 1 },
                        ForkProposal::SwapTape {
                            fighter: 0,
                            inputs: attack_spam(0, from, 60),
                        },
                    ],
                    _ => Vec::new(),
                };
                Branch::new(id(slot), proposals)
            })
            .collect()
    }

    /// The naive way to open one branch: replay the prefix from tick zero,
    /// fork it, queue the proposals, run the window.
    fn serial_receipt(fork: &Branch, from: u32, to: u32) -> BranchReceipt {
        let (snapshot, _) =
            run_prefix(&config(), &FighterTape::default(), from).expect("valid prefix");
        let mut forked = Fork::from_snapshot(snapshot, from).expect("tick matches snapshot");
        for proposal in &fork.proposals {
            forked
                .apply_proposal(proposal.clone())
                .expect("test proposals apply");
        }
        BranchReceipt {
            tag: IntervalTag {
                fork: fork.fork,
                from,
                to,
            },
            metrics: forked.run_n(to - from),
        }
    }

    #[test]
    fn branch_set_keeps_fork_ids_dense_from_zero() {
        let set = BranchSet::new(twenty(240)).expect("20 dense branches build");
        assert_eq!(set.len(), 20);
        assert!(!set.is_empty());
        for (index, branch) in set.branches().iter().enumerate() {
            assert_eq!(branch.fork.slot(), index, "slot {index} keeps its id");
        }

        let empty = BranchSet::new(Vec::new());
        assert_eq!(empty, Err(BranchError::EmptySet), "an empty set is refused");

        let duplicate = BranchSet::new(vec![
            Branch::new(id(0), Vec::new()),
            Branch::new(id(0), Vec::new()),
        ]);
        assert_eq!(
            duplicate,
            Err(BranchError::SparseFork {
                index: 1,
                fork: id(0),
            }),
            "two receipts never share one id"
        );

        let skipped = BranchSet::new(vec![
            Branch::new(id(0), Vec::new()),
            Branch::new(id(2), Vec::new()),
        ]);
        assert_eq!(
            skipped,
            Err(BranchError::SparseFork {
                index: 1,
                fork: id(2),
            }),
            "a skipped id is refused, never renumbered"
        );
    }

    #[test]
    fn twenty_branches_tag_their_window_and_repeat_equal_receipts() {
        let set = BranchSet::new(twenty(240)).expect("20 dense branches build");
        let fork = fork_at(240);
        let first = set.speculate(&fork, 240, 300).expect("every branch merges");
        assert_eq!(first.len(), 20, "one receipt per branch");
        for (index, receipt) in first.iter().enumerate() {
            let tag = receipt.tag;
            assert_eq!(tag.fork.slot(), index, "receipt order is branch order");
            assert_eq!((tag.from, tag.to), (240, 300), "every tag names the window");
            assert_eq!(tag.len(), 60, "the window is the tick count");
            assert!(tag.covers(240) && !tag.covers(300), "the span is half-open");
            assert_ne!(receipt.digest(), 0, "a receipt carries its digest");
        }
        // The set twice is the same set: same tags, same metrics, same digests.
        let second = set.speculate(&fork, 240, 300).expect("the set runs again");
        assert_eq!(first, second, "two runs of one set agree");
        // And the fork path equals the naive per-branch replay path.
        let serial: Vec<BranchReceipt> = set
            .branches()
            .iter()
            .map(|branch| serial_receipt(branch, 240, 300))
            .collect();
        assert_eq!(first, serial, "parallel branches equal replayed branches");
        // The set is not one repeated outcome: the frozen pairs differ.
        assert_ne!(first[0].metrics, first[16].metrics, "branches differ");
        // The caller's fork survives the set: it still runs its own window.
        assert_eq!(
            fork.run_n(60),
            fork.run_n(60),
            "the caller's fork is untouched"
        );
    }

    #[test]
    fn one_bad_branch_refuses_the_whole_merge() {
        let mut branches = twenty(240);
        branches[7] = Branch::new(
            id(7),
            vec![ForkProposal::SetAggression {
                fighter: 9,
                permille: 500,
            }],
        );
        let set = BranchSet::new(branches).expect("the shape still builds");
        let fork = fork_at(240);
        assert_eq!(
            set.speculate(&fork, 240, 300),
            Err(BranchError::Refused {
                fork: id(7),
                error: ForkError::UnknownFighter(9),
            }),
            "one refused proposal publishes no receipt"
        );
        // A crashed worker is a refusal too: it reads as no receipt, never as
        // a zeroed run that looks like a clean sweep.
        assert_eq!(
            BranchError::Crashed(id(7)).to_string(),
            "fork7 worker panicked; no receipt"
        );
        // A window that holds no tick is refused before any job is queued.
        assert_eq!(
            set.speculate(&fork, 300, 300),
            Err(BranchError::EmptyWindow { from: 300, to: 300 })
        );
    }

    /// One branch run off the caller's own fork, one after another: the same
    /// window the set runs, without the parallel pool and without a prefix
    /// replay.
    fn window_receipt(fork: &Fork, branch: &Branch, from: u32, to: u32) -> BranchReceipt {
        let mut forked = fork.clone();
        for proposal in &branch.proposals {
            forked
                .apply_proposal(proposal.clone())
                .expect("test proposals apply");
        }
        BranchReceipt {
            tag: IntervalTag {
                fork: branch.fork,
                from,
                to,
            },
            metrics: forked.run_n(to - from),
        }
    }

    /// The fastest of `trials` timed runs of `f`, with the value it produced.
    /// A busy machine can only make a path look slower than it is, so both
    /// sides of the report take their best run and the comparison stays fair.
    #[expect(
        clippy::disallowed_types,
        clippy::disallowed_methods,
        reason = "timing test reads the wall clock; it never enters the tick"
    )]
    fn best_of<F, T>(trials: usize, mut f: F) -> (T, f64)
    where
        F: FnMut() -> T,
    {
        let mut best: Option<(T, f64)> = None;
        for _ in 0..trials {
            let start = std::time::Instant::now();
            let value = f();
            let ms = start.elapsed().as_secs_f64() * 1_000.0;
            let faster = match &best {
                None => true,
                Some((_, best_ms)) => ms < *best_ms,
            };
            if faster {
                best = Some((value, ms));
            }
        }
        best.expect("at least one trial")
    }

    /// AX12 timing probe: the 20-branch report. Ignored in the default suite
    /// because it asserts wall-clock time; run it alone with
    /// `cargo test -p engine-arena --release --lib branch:: -- --ignored
    /// --nocapture --test-threads=1`.
    #[test]
    #[ignore = "timing: AX12 20-branch create speedup; run alone with --ignored --test-threads=1"]
    fn twenty_branch_speedup_report() {
        const BRANCHES: usize = 20;
        const TRIALS: usize = 3;
        // The shape a level edit takes: one match, then a speculative branch
        // off its last state. The fork path replays the prefix once for the
        // whole set.
        let (from, to) = (360u32, 600u32);
        let set = BranchSet::new(twenty(from)).expect("20 dense branches build");
        let tape = FighterTape::default();

        // Naive path: every branch replays the prefix from tick zero first.
        let (serial, serial_ms) = best_of(TRIALS, || {
            set.branches()
                .iter()
                .map(|branch| serial_receipt(branch, from, to))
                .collect::<Vec<BranchReceipt>>()
        });

        // Fork path: one prefix, then the whole set on the pool.
        let (receipts, branch_ms) = best_of(TRIALS, || {
            let (snapshot, _) = run_prefix(&config(), &tape, from).expect("valid prefix");
            let fork = Fork::from_snapshot(snapshot, from).expect("tick matches snapshot");
            set.speculate(&fork, from, to).expect("every branch merges")
        });

        // Window only: the same 20 windows run one after another off one
        // fork, against the same parallel set. The shared prefix is out of
        // it, so this is the parallel part alone.
        let (snapshot, _) = run_prefix(&config(), &tape, from).expect("valid prefix");
        let fork = Fork::from_snapshot(snapshot, from).expect("tick matches snapshot");
        let (serial_window, serial_window_ms) = best_of(TRIALS, || {
            set.branches()
                .iter()
                .map(|branch| window_receipt(&fork, branch, from, to))
                .collect::<Vec<BranchReceipt>>()
        });
        let (parallel_window, parallel_window_ms) = best_of(TRIALS, || {
            set.speculate(&fork, from, to).expect("every branch merges")
        });

        let again = set.speculate(&fork, from, to).expect("the set runs again");
        let report = BranchReport {
            branches: BRANCHES,
            merged: receipts.len(),
            window: to - from,
            prefix: from,
            serial_ms,
            branch_ms,
            speedup_x: if branch_ms > 0.0 {
                serial_ms / branch_ms
            } else {
                0.0
            },
            parallel_x: if parallel_window_ms > 0.0 {
                serial_window_ms / parallel_window_ms
            } else {
                0.0
            },
            x2_equal: receipts == again,
        };
        println!("AX12 {}", report.text());

        assert_eq!(report.merged, BRANCHES, "{}", report.text());
        assert!(report.x2_equal, "{}", report.text());
        assert_eq!(receipts, serial, "{}", report.text());
        assert_eq!(parallel_window, receipts, "{}", report.text());
        assert_eq!(serial_window, receipts, "{}", report.text());
        assert!(
            report.speedup_x >= 4.0,
            "the fork path must be 4x faster: {}",
            report.text()
        );
        assert!(
            report.parallel_x >= 1.5,
            "the set must not run slower than the same windows one by one: {}",
            report.text()
        );
    }
}
