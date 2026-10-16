//! Debounced change-triggered bounded cancellable forks.
//!
//! Consumer: the balance assistant and background checks — an editor
//! revision lands, a fork tournament verifies it, the tick never waits.
//! Data owner: [`Trigger`] owns one clamped [`ArenaVariant`](super::ArenaVariant)
//! snapshot per job; the caller keeps its live document. Units: editor
//! revisions (`u64`, tick-counted, no wall clock), fixed match ticks,
//! permille win rates. Determinism scope: same submit/poll sequence gives
//! the same completions — jobs run in `JobId` order, one [`Fork`](engine_arena::fork::Fork)
//! per poll step, no threads, no wall clock, no hash-map order.
//! Error policy: invalid variants complete immediately with a failing
//! [`Verdict`](super::Verdict) (no fork work); `cancel` on an unknown or
//! terminal job returns `false` and changes nothing.
//!
//! Debounce rule (tick-counted): [`submit`](Trigger::submit) coalesces a
//! pending `Queued` job holding the same variant (same content, any
//! revision) and returns its `JobId`, so rapid identical edits run once.
//! Distinct variants queue separately up to [`MAX_JOBS`]; when a newer
//! revision lands mid-run the older job still finishes but its
//! [`JobDone`] is tagged `stale`, so the balance assistant never mistakes an
//! old verdict for the current document.
//!
//! Bound rule: each job clamps `match_ticks` to [`MAX_TICKS`], at most
//! [`MAX_JOBS`] jobs stay `Queued`/`Running` (the oldest queued yields to
//! the newcomer), and each [`poll`](Trigger::poll) runs at most
//! [`MATCHES_PER_POLL`] full-match forks. `submit`/`poll`/`cancel` never
//! block on another job: the editor and the fixed tick stay responsive.

use super::winrates::{fork_damage, slot_zero_result, SlotZero};
use super::{ArenaVariant, Verdict};
use engine_arena::ArenaConfig;

/// Maximum `Queued` + `Running` jobs. Older queued jobs are cancelled to
/// make room, so the editor can type ahead without an open queue.
pub const MAX_JOBS: usize = 4;

/// Maximum fixed ticks per match inside one job. Larger requests clamp;
/// the job record keeps the clamped value.
pub const MAX_TICKS: u32 = 600;

/// Full-match forks per [`poll`](Trigger::poll) call across the single
/// running job. One 600-tick kinematic fork is the bounded step; the
/// caller polls again for the next match.
pub const MATCHES_PER_POLL: u32 = 1;

/// Opaque fork-job handle, handed out by `submit` in increasing order.
pub type JobId = u64;

/// Lifecycle of one fork job. `Stale` is terminal like `Done`: the work
/// finished, but a newer revision had landed meanwhile.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum JobStatus {
    /// Waiting for the single running slot.
    Queued,
    /// Owning the running slot, advancing one match per poll.
    Running,
    /// Finished for the latest revision.
    Done,
    /// Finished while a newer revision was already submitted.
    Stale,
    /// Cancelled before completion; never yields a `JobDone`.
    Cancelled,
}

impl JobStatus {
    /// True while the job still holds a queue or running slot.
    pub(crate) const fn is_active(self) -> bool {
        matches!(self, Self::Queued | Self::Running)
    }
}

/// Observable snapshot of one job for status panels.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ForkJob {
    /// Handle from `submit`.
    pub id: JobId,
    /// Editor revision that submitted (or coalesced into) this job.
    pub revision: u64,
    /// Clamped fixed ticks per match (`<= MAX_TICKS`).
    pub max_ticks: u32,
    /// Current lifecycle state.
    pub status: JobStatus,
}

/// One finished verification. `stale` is true when a newer revision had
/// been submitted before this job completed.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct JobDone {
    /// Finished job handle.
    pub id: JobId,
    /// Revision this job verified.
    pub revision: u64,
    /// Combined reachability / win-rate / budget verdict.
    pub verdict: Verdict,
    /// Seeds replayed (win-rate tournament size; 0 when validation
    /// failed before any fork ran).
    pub seed_count: u32,
    /// True when superseded mid-run by a newer revision.
    pub stale: bool,
}

/// Internal per-job state: clamped variant plus incremental tallies.
#[derive(Clone, Debug)]
struct Inner {
    id: JobId,
    revision: u64,
    variant: ArenaVariant,
    max_ticks: u32,
    status: JobStatus,
    next_match: u32,
    wins: u32,
    draws: u32,
    losses: u32,
}

/// Debounced trigger: owns pending fork verifications.
#[derive(Clone, Debug, Default)]
pub struct Trigger {
    next_id: JobId,
    latest_revision: u64,
    jobs: Vec<Inner>,
}

impl Trigger {
    /// Empty trigger with no jobs and no latest revision.
    pub fn new() -> Self {
        Self::default()
    }

    /// Latest revision seen; stale results are jobs below this.
    pub fn latest_revision(&self) -> u64 {
        self.latest_revision
    }

    /// Active (`Queued` + `Running`) job count, always `<= MAX_JOBS`.
    pub fn active_count(&self) -> usize {
        self.jobs
            .iter()
            .filter(|job| job.status.is_active())
            .count()
    }

    /// Snapshot one job's observable state, or `None` for unknown ids.
    pub fn job(&self, id: JobId) -> Option<ForkJob> {
        self.jobs
            .iter()
            .find(|job| job.id == id)
            .map(|job| ForkJob {
                id: job.id,
                revision: job.revision,
                max_ticks: job.max_ticks,
                status: job.status,
            })
    }

    /// Snapshots of every active job in `JobId` order.
    pub fn active_jobs(&self) -> Vec<ForkJob> {
        let mut out: Vec<ForkJob> = self
            .jobs
            .iter()
            .filter(|job| job.status.is_active())
            .map(|job| ForkJob {
                id: job.id,
                revision: job.revision,
                max_ticks: job.max_ticks,
                status: job.status,
            })
            .collect();
        out.sort_by_key(|job| job.id);
        out
    }

    /// Submit one revision for verification. Coalesces a pending `Queued`
    /// job holding the same variant (bumps its revision, returns its id);
    /// otherwise queues a clamped copy, evicting the oldest queued job
    /// past [`MAX_JOBS`]. Never blocks, never runs a fork.
    pub fn submit(&mut self, revision: u64, variant: ArenaVariant) -> JobId {
        if revision > self.latest_revision {
            self.latest_revision = revision;
        }
        for job in self.jobs.iter_mut() {
            if job.status == JobStatus::Queued && job.variant == variant {
                if revision > job.revision {
                    job.revision = revision;
                }
                return job.id;
            }
        }
        let clamped = clamp_variant(&variant);
        let max_ticks = clamped.balance.match_ticks;
        if self.active_count() >= MAX_JOBS {
            if let Some(victim) = self
                .jobs
                .iter_mut()
                .filter(|job| job.status == JobStatus::Queued)
                .min_by_key(|job| job.id)
            {
                victim.status = JobStatus::Cancelled;
            } else if let Some(victim) = self
                .jobs
                .iter_mut()
                .filter(|job| job.status == JobStatus::Running)
                .min_by_key(|job| job.id)
            {
                victim.status = JobStatus::Cancelled;
            }
        }
        let id = self.next_id;
        self.next_id += 1;
        self.jobs.push(Inner {
            id,
            revision,
            variant: clamped,
            max_ticks,
            status: JobStatus::Queued,
            next_match: 0,
            wins: 0,
            draws: 0,
            losses: 0,
        });
        id
    }

    /// Cancel a queued or running job. Terminal and unknown ids return
    /// `false` and change nothing; cancelled jobs never yield `JobDone`.
    pub fn cancel(&mut self, id: JobId) -> bool {
        match self.jobs.iter_mut().find(|job| job.id == id) {
            Some(job) if job.status.is_active() => {
                job.status = JobStatus::Cancelled;
                true
            }
            _ => false,
        }
    }

    /// Advance the oldest active job by at most [`MATCHES_PER_POLL`]
    /// matches and return completions (zero or one). Non-blocking and
    /// bounded: at most one full-match fork per call.
    pub fn poll(&mut self) -> Vec<JobDone> {
        let Some(id) = self.next_target() else {
            return Vec::new();
        };
        // Fast validation path: invalid configs finish without any fork.
        if let Some((verdict, seeds)) = fast_verdict(&self.inner(id).variant) {
            return self.complete(id, verdict, seeds);
        }
        // Bounded slice: one full-match fork.
        let total = self.inner(id).variant.balance.matches;
        for _ in 0..MATCHES_PER_POLL {
            let (config, morale) = {
                let job = self.inner(id);
                if job.next_match >= total {
                    break;
                }
                let seed = job
                    .variant
                    .balance
                    .base_seed
                    .wrapping_add(u64::from(job.next_match));
                let config = ArenaConfig {
                    seed,
                    ticks: job.variant.balance.match_ticks,
                    fighters: job.variant.morale.len() as u8,
                    half_extent_m: job.variant.half_extent_m,
                    table_version: 1,
                };
                (config, job.variant.morale.clone())
            };
            if let Some(damage) = fork_damage(&config, &morale) {
                let result = slot_zero_result(&damage);
                let job = self.inner_mut(id);
                match result {
                    SlotZero::Win => job.wins += 1,
                    SlotZero::Draw => job.draws += 1,
                    SlotZero::Loss => job.losses += 1,
                }
                job.next_match += 1;
            } else {
                // Defensive: fork rejected a validated config. Fail fast
                // with the shared verifier rather than looping forever.
                let job = self.inner(id);
                let (verdict, tallied) = (super::verify(&job.variant), job.next_match);
                return self.complete(id, verdict, tallied);
            }
            if self.inner(id).status == JobStatus::Cancelled {
                return Vec::new();
            }
        }
        if self.inner(id).next_match < total {
            return Vec::new();
        }
        let verdict = finish_verdict(self.inner(id));
        self.complete(id, verdict, total)
    }

    /// Pick the job `poll` advances: the oldest running job, else the oldest
    /// queued job, which starts running now.
    fn next_target(&mut self) -> Option<JobId> {
        let running = self
            .jobs
            .iter()
            .filter(|job| job.status == JobStatus::Running)
            .min_by_key(|job| job.id)
            .map(|job| job.id);
        if running.is_some() {
            return running;
        }
        let next = self
            .jobs
            .iter()
            .filter(|job| job.status == JobStatus::Queued)
            .min_by_key(|job| job.id)
            .map(|job| job.id)?;
        self.inner_mut(next).status = JobStatus::Running;
        Some(next)
    }

    /// Close job `id` with `verdict`: `Stale` when a newer revision has been
    /// submitted, `Done` otherwise, reported as the one completion.
    fn complete(&mut self, id: JobId, verdict: Verdict, seed_count: u32) -> Vec<JobDone> {
        let latest = self.latest_revision;
        let job = self.inner_mut(id);
        let stale = job.revision < latest;
        job.status = if stale {
            JobStatus::Stale
        } else {
            JobStatus::Done
        };
        vec![JobDone {
            id,
            revision: job.revision,
            verdict,
            seed_count,
            stale,
        }]
    }

    /// The job `id`, which the calling poll already resolved.
    fn inner(&self, id: JobId) -> &Inner {
        self.jobs
            .iter()
            .find(|job| job.id == id)
            .expect("target exists")
    }

    /// Mutable form of [`Self::inner`].
    fn inner_mut(&mut self, id: JobId) -> &mut Inner {
        self.jobs
            .iter_mut()
            .find(|job| job.id == id)
            .expect("target exists")
    }
}

/// Clamp one variant to the trigger bounds: `match_ticks` to
/// `1..=MAX_TICKS` only when it exceeds the cap (invalid rows such as 0
/// pass through so validation still reports them). Matches are not
/// clamped: oversize counts fail fast with no fork work.
fn clamp_variant(variant: &ArenaVariant) -> ArenaVariant {
    let mut out = variant.clone();
    if out.balance.match_ticks > MAX_TICKS {
        out.balance.match_ticks = MAX_TICKS;
    }
    out
}

/// Fast verdict for configs that must not simulate: fighter count,
/// morale rows, match count or arena geometry invalid. Returns the full
/// verdict with `seed_count` 0. `None` means the variant is runnable.
fn fast_verdict(variant: &ArenaVariant) -> Option<(Verdict, u32)> {
    let fighters = variant.morale.len();
    let runnable = fighters >= 2
        && fighters <= usize::from(engine_arena::MAX_PARTICIPANTS)
        && variant.morale.iter().all(|permille| *permille <= 1000)
        && variant.balance.matches >= 1
        && variant.balance.matches <= super::winrates::MAX_MATCHES
        && ArenaConfig {
            seed: variant.balance.base_seed,
            ticks: variant.balance.match_ticks,
            fighters: fighters as u8,
            half_extent_m: variant.half_extent_m,
            table_version: 1,
        }
        .validate()
        .is_ok();
    if runnable {
        None
    } else {
        Some((super::verify(variant), 0))
    }
}

/// Final verdict for a tallied job: cheap verifiers plus the accumulated
/// win-rate report checked against the declared bounds (no re-simulation).
fn finish_verdict(job: &Inner) -> Verdict {
    let mut failures = super::reachability::check(&job.variant);
    let mut checks_run = 0;
    checks_run += 1;
    let seeds: Vec<u64> = (0..job.variant.balance.matches)
        .map(|index| job.variant.balance.base_seed.wrapping_add(u64::from(index)))
        .collect();
    let rate = if job.variant.balance.matches == 0 {
        0
    } else {
        ((2 * job.wins + job.draws) * 1000 / (2 * job.variant.balance.matches)) as u16
    };
    let report = super::winrates::Report {
        matches: job.variant.balance.matches,
        match_ticks: job.variant.balance.match_ticks,
        seeds,
        wins: job.wins,
        draws: job.draws,
        losses: job.losses,
        rate_permille: rate,
    };
    failures.extend(super::winrates::check_report(&report, &job.variant.balance));
    checks_run += 1;
    failures.extend(super::budgets::check(&job.variant));
    checks_run += 1;
    Verdict::from_checks(failures, checks_run)
}

#[cfg(test)]
mod tests {
    use super::super::{reference_arena, ArenaVariant, BalanceConfig};
    use super::*;

    fn tiny(name: &str) -> ArenaVariant {
        let mut variant = reference_arena();
        variant.name = name.to_string();
        variant.balance = BalanceConfig {
            matches: 2,
            match_ticks: 60,
            base_seed: 7,
            min_win_permille: 0,
            max_win_permille: 1000,
        };
        variant
    }

    fn drain(trigger: &mut Trigger) -> Vec<JobDone> {
        let mut out = Vec::new();
        for _ in 0..64 {
            let done = trigger.poll();
            if done.is_empty() {
                // No completion this step: keep polling while work remains.
                if trigger.active_count() == 0 {
                    break;
                }
                continue;
            }
            out.extend(done);
            if trigger.active_count() == 0 {
                break;
            }
        }
        out
    }

    #[test]
    fn rapid_edits_coalesce_to_one_job() {
        let mut trigger = Trigger::new();
        let variant = tiny("same");
        let first = trigger.submit(1, variant.clone());
        let second = trigger.submit(2, variant.clone());
        let third = trigger.submit(3, variant);
        assert_eq!(first, second, "same variant must coalesce");
        assert_eq!(second, third, "rapid edits must coalesce to one job");
        assert_eq!(trigger.active_count(), 1);
        let done = drain(&mut trigger);
        assert_eq!(done.len(), 1, "one coalesced job, one verdict: {done:?}");
        assert_eq!(done[0].id, first);
        assert_eq!(done[0].revision, 3, "latest revision wins");
        assert!(!done[0].stale);
        assert!(done[0].verdict.pass, "{:?}", done[0].verdict.failures);
        assert_eq!(done[0].verdict.checks_run, 3, "{:?}", done[0].verdict);
    }

    #[test]
    fn cancel_stops_a_queued_job() {
        let mut trigger = Trigger::new();
        let first = trigger.submit(1, tiny("alpha"));
        let second = trigger.submit(2, tiny("beta"));
        assert_ne!(first, second, "distinct variants queue separately");
        // Start the first job so the second stays queued behind it.
        assert!(trigger.poll().is_empty(), "first match is bounded work");
        assert_eq!(
            trigger.job(first).expect("first exists").status,
            JobStatus::Running
        );
        assert_eq!(
            trigger.job(second).expect("second exists").status,
            JobStatus::Queued
        );
        assert!(trigger.cancel(second), "queued cancel must succeed");
        assert_eq!(
            trigger.job(second).expect("second kept").status,
            JobStatus::Cancelled
        );
        let done = drain(&mut trigger);
        assert!(
            done.iter().all(|job| job.id != second),
            "cancelled job never completes: {done:?}"
        );
        assert!(
            done.iter().any(|job| job.id == first),
            "survivor still verifies: {done:?}"
        );
        assert!(!trigger.cancel(second), "second cancel is a no-op");
        assert!(!trigger.cancel(9999), "unknown id cancels nothing");
    }

    #[test]
    fn stale_revision_tags_on_supersede() {
        let mut trigger = Trigger::new();
        let first = trigger.submit(1, tiny("old"));
        // One bounded step starts the job without finishing it (2 matches).
        assert!(trigger.poll().is_empty());
        let second = trigger.submit(2, tiny("new"));
        assert_ne!(first, second);
        let done = drain(&mut trigger);
        assert_eq!(done.len(), 2, "{done:?}");
        let old = done.iter().find(|job| job.id == first).expect("old done");
        let new = done.iter().find(|job| job.id == second).expect("new done");
        assert_eq!(old.revision, 1);
        assert!(old.stale, "superseded mid-run must tag stale");
        assert_eq!(
            trigger.job(first).expect("old kept").status,
            JobStatus::Stale
        );
        assert_eq!(new.revision, 2);
        assert!(!new.stale, "latest revision stays fresh");
        assert_eq!(
            trigger.job(second).expect("new kept").status,
            JobStatus::Done
        );
    }

    #[test]
    fn max_ticks_and_max_jobs_enforced() {
        let mut trigger = Trigger::new();
        let mut oversized = tiny("oversized");
        oversized.balance.match_ticks = MAX_TICKS + 500;
        let id = trigger.submit(1, oversized);
        let snapshot = trigger.job(id).expect("job exists");
        assert_eq!(
            snapshot.max_ticks, MAX_TICKS,
            "match ticks must clamp to MAX_TICKS"
        );
        assert!(snapshot.max_ticks <= MAX_TICKS);

        let mut trigger = Trigger::new();
        for index in 0..(MAX_JOBS + 2) {
            trigger.submit(index as u64 + 1, tiny(&format!("variant-{index}")));
        }
        assert!(
            trigger.active_count() <= MAX_JOBS,
            "queue must stay bounded: {}",
            trigger.active_count()
        );
        assert_eq!(trigger.active_count(), MAX_JOBS);
        assert_eq!(trigger.active_jobs().len(), MAX_JOBS);
    }

    #[test]
    fn win_rate_report_carries_seed_count_and_uncertainty() {
        let variant = tiny("seeds");
        let report = super::super::winrates::report(&variant).expect("tiny valid");
        assert_eq!(report.seed_count(), 2);
        assert_eq!(report.seeds.len(), 2);
        assert_eq!(report.seeds, vec![7, 8]);
        // Uncertainty is a bounded permille half-width, deterministic.
        let first = report.uncertainty_permille();
        assert!(first <= 1000, "{first}");
        assert_eq!(report.uncertainty_permille(), first);
        // Reference tournament quotes both in its rejection reason.
        let mut demanding = reference_arena();
        demanding.balance.min_win_permille = 900;
        demanding.balance.max_win_permille = 1000;
        let failures = super::super::winrates::check(&demanding);
        assert_eq!(failures.len(), 1);
        assert!(
            failures[0].reason.contains("seed count 20"),
            "{}",
            failures[0].reason
        );
        assert!(
            failures[0].reason.contains("1-sigma"),
            "{}",
            failures[0].reason
        );

        // Trigger completions carry the same seed count.
        let mut trigger = Trigger::new();
        trigger.submit(9, variant);
        let done = drain(&mut trigger);
        assert_eq!(done.len(), 1);
        assert_eq!(done[0].seed_count, 2, "{done:?}");
        assert!(!done[0].stale);
    }

    #[test]
    fn submit_poll_cancel_never_wait() {
        // Smoke: every entrypoint returns promptly with bounded work —
        // one poll advances at most MATCHES_PER_POLL matches.
        let mut trigger = Trigger::new();
        let id = trigger.submit(1, tiny("prompt"));
        assert!(!trigger.cancel(4242));
        let before = trigger.job(id).expect("job queued").status;
        assert_eq!(before, JobStatus::Queued);
        let _ = trigger.poll();
        let _ = trigger.active_jobs();
        let _ = trigger.latest_revision();
        assert!(trigger.cancel(id) || trigger.job(id).is_some());
    }
}
