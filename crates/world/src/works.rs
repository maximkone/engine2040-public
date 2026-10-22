//! works: build jobs as data, worked by crews (LEG7).
//!
//! A Roman camp is four kinds of work along one perimeter: the ditch, the turf
//! rampart behind it, the stake line on the rampart, and one job per gate. A
//! job is data - what it builds, how much work it needs and how much is done -
//! and a crew advances it: more hands go faster, an unfinished work holds less
//! than a finished one, and deconstructing gives the invested material back.
//! The camp is done when the perimeter closes, which is every job done.
//!
//! Owner: engine-world. Units: millimetres, crew-ticks (one crew-removes-one-
//! work-step per tick) and crew hands. Determinism: integer math only, the job
//! order is the plan order, no clock and no map: the same plan worked the same
//! way replays byte for byte. Errors: none - every accessor saturates inside
//! its bounds instead of refusing.
//!
//! The camp plan's millimetres arrive as plain numbers: the Rome crate's own
//! table loader (`crates/rome/src/camp.rs`) owns `content/rome/camp.toml`, and
//! this crate stays engine-core-only, so the seam is a struct the caller fills.
//! The stake line and the gates have no table entry of their own yet, so their
//! shares of the perimeter are fixed permilles here.

use engine_core::Fnv64;

/// One stake per this many millimetres of perimeter: 888 mm, the rank gap the
/// line table gives for the spacing between stakes.
pub const STAKE_STEP_MM: i32 = 888;

/// Cells of ditch one crew step digs, in millimetres of perimeter per step.
pub const DITCH_STEP_MM: i32 = 200;

/// Cells of rampart one crew step raises, in millimetres of perimeter per step:
/// turf is slower than digging.
pub const RAMPART_STEP_MM: i32 = 120;

/// Cells of stake line one crew step plants: the fastest of the three.
pub const STAKE_STEP_MM_PER_CREW: i32 = STAKE_STEP_MM;

/// One gate this many perimeter millimetres: the clear space between the
/// ditch's ends at the gate, 60 Roman feet in the shipped table.
pub const GATE_SPACE_MM: i32 = 17_760;

/// Gates a camp lays, one job each: the two the march always opens.
pub const GATES: u32 = 2;

/// Material one work step of each kind costs, in the same units the camp's
/// ledger counts: deconstruct refunds what is invested, no more.
pub const MATERIAL_PER_STEP: u32 = 1;

/// One build step's work, in the unit the job counts.
pub type Step = u32;

/// The four kinds of work a camp perimeter is built from.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum WorkKind {
    /// The ditch ahead of the rampart.
    Ditch,
    /// The turf rampart raised from the ditch's earth.
    Rampart,
    /// The stake line planted along the rampart.
    StakeLine,
    /// One gate opening, the ditch and rampart broken for the road.
    Gate,
}

impl WorkKind {
    /// Every kind, in plan order.
    pub const ALL: [Self; 4] = [Self::Ditch, Self::Rampart, Self::StakeLine, Self::Gate];

    /// The millimetres of perimeter one crew step of this kind builds.
    #[must_use]
    pub const fn step_mm(self) -> i32 {
        match self {
            Self::Ditch => DITCH_STEP_MM,
            Self::Rampart => RAMPART_STEP_MM,
            Self::StakeLine => STAKE_STEP_MM_PER_CREW,
            Self::Gate => GATE_SPACE_MM,
        }
    }

    /// Plan position of this kind, 0..3: the digest's byte for one job.
    #[must_use]
    pub const fn ord(self) -> u8 {
        match self {
            Self::Ditch => 0,
            Self::Rampart => 1,
            Self::StakeLine => 2,
            Self::Gate => 3,
        }
    }
}

/// One build job: what it builds, its work in steps, and how many steps are
/// done. Pure data: no clock, no map, no allocation.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Job {
    /// The kind of work.
    pub kind: WorkKind,
    /// Work steps this job needs, at least one.
    pub total: Step,
    /// Work steps done, never past `total`.
    pub done: Step,
}

impl Job {
    /// A fresh job that needs `total` steps of work.
    #[must_use]
    pub const fn new(kind: WorkKind, total: Step) -> Self {
        let total = if total == 0 { 1 } else { total };
        Self {
            kind,
            total,
            done: 0,
        }
    }

    /// Advance by one crew tick: `hands` steps of work, saturating at the job's
    /// total. Returns the steps actually applied.
    pub fn crew_tick(&mut self, hands: u32) -> Step {
        let want = hands.min(self.total - self.done);
        self.done += want;
        want
    }

    /// True when every step is done.
    #[must_use]
    pub const fn finished(&self) -> bool {
        self.done >= self.total
    }

    /// Work progress in permille, 0..=1000.
    #[must_use]
    pub fn progress_permille(&self) -> u32 {
        let permille = u64::from(self.done) * 1000 / u64::from(self.total);
        u32::try_from(permille).unwrap_or(1000)
    }

    /// What this job holds back while it stands: the full strength of its kind
    /// when finished, half of it at half progress, so an unfinished work
    /// always blocks less than a finished one.
    #[must_use]
    pub fn blocking_permille(&self) -> u32 {
        if self.finished() {
            1000
        } else {
            self.progress_permille() / 2
        }
    }

    /// The material this job has consumed so far: the invested steps.
    #[must_use]
    pub const fn invested(&self) -> u32 {
        self.done * MATERIAL_PER_STEP
    }
}

/// The camp plan's numbers, in millimetres: the perimeter and the work widths
/// the table gives. The caller fills this from `crates/rome`'s camp table.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct CampPlan {
    /// The camp perimeter's length in millimetres: a 50 x 50 m camp is 200 000.
    pub perimeter_mm: i32,
    /// Gates the camp opens; one job each.
    pub gates: u32,
}

impl CampPlan {
    /// The one-night camp the campaign opens: a 50 x 50 m perimeter (200 m of
    /// ditch), two gates.
    pub const ONE_NIGHT: Self = Self {
        perimeter_mm: 200_000,
        gates: GATES,
    };

    /// The camp's work laid out as jobs, in plan order: the ditch, the rampart
    /// and the stake line along the perimeter, then one job per gate.
    #[must_use]
    pub fn jobs(&self) -> Vec<Job> {
        let mut jobs = Vec::with_capacity(WorkKind::ALL.len());
        for kind in [WorkKind::Ditch, WorkKind::Rampart, WorkKind::StakeLine] {
            let steps = self.perimeter_mm.max(0) / kind.step_mm().max(1);
            let total = u32::try_from(steps).unwrap_or(u32::MAX);
            jobs.push(Job::new(kind, total));
        }
        for _ in 0..self.gates.min(64) {
            jobs.push(Job::new(WorkKind::Gate, 1));
        }
        jobs
    }
}

/// The camp under construction: its plan's jobs, in plan order.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct CampWorks {
    plan: CampPlan,
    jobs: Vec<Job>,
}

impl CampWorks {
    /// Lay out the plan's jobs.
    #[must_use]
    pub fn new(plan: CampPlan) -> Self {
        Self {
            plan,
            jobs: plan.jobs(),
        }
    }

    /// The plan the camp was laid out with.
    #[must_use]
    pub const fn plan(&self) -> &CampPlan {
        &self.plan
    }

    /// The jobs, in plan order.
    #[must_use]
    pub fn jobs(&self) -> &[Job] {
        &self.jobs
    }

    /// Work one crew tick over every job in order: `hands` steps on the first
    /// unfinished job, the remainder on the next, and so on. A crew of one
    /// advances one job per tick.
    pub fn crew_tick(&mut self, hands: u32) {
        let mut left = hands;
        for job in &mut self.jobs {
            if left == 0 {
                break;
            }
            left -= job.crew_tick(left);
        }
    }

    /// True when every job is done: the perimeter is closed.
    #[must_use]
    pub fn perimeter_closed(&self) -> bool {
        self.jobs.iter().all(Job::finished)
    }

    /// Ticks one crew of `hands` still needs to close the perimeter: the
    /// remaining work of every job divided by the crew's throughput per tick,
    /// rounded up (u64 math, no overflow on a 200 m perimeter).
    #[must_use]
    pub fn ticks_to_close(&self, hands: u32) -> u32 {
        let hands = u64::from(hands.max(1));
        let remaining: u64 = self
            .jobs
            .iter()
            .map(|job| u64::from(job.total - job.done))
            .sum();
        u32::try_from(remaining.div_ceil(hands)).unwrap_or(u32::MAX)
    }

    /// Deconstruct the job at `index`: its invested material comes back and the
    /// work is scored back to fresh. `None` for an index past the plan.
    pub fn deconstruct(&mut self, index: usize) -> Option<u32> {
        let job = self.jobs.get_mut(index)?;
        let refund = job.invested();
        job.done = 0;
        Some(refund)
    }

    /// A digest of the camp's jobs: the same layout worked the same way
    /// replays byte for byte.
    #[must_use]
    pub fn hash(&self) -> u64 {
        let mut hasher = Fnv64::new();
        hasher.write_i32(self.plan.perimeter_mm);
        hasher.write_u32(self.plan.gates);
        hasher.write_u32(u32::try_from(self.jobs.len()).unwrap_or(u32::MAX));
        for job in &self.jobs {
            hasher.write_u8(job.kind.ord());
            hasher.write_u32(job.total);
            hasher.write_u32(job.done);
        }
        hasher.finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eight_diggers_close_the_camp_faster_than_four() {
        let mut four = CampWorks::new(CampPlan::ONE_NIGHT);
        let mut eight = CampWorks::new(CampPlan::ONE_NIGHT);
        let mut ticks_four: u32 = 0;
        let mut ticks_eight: u32 = 0;
        while !four.perimeter_closed() {
            four.crew_tick(4);
            ticks_four = ticks_four.saturating_add(1);
        }
        while !eight.perimeter_closed() {
            eight.crew_tick(8);
            ticks_eight = ticks_eight.saturating_add(1);
        }
        assert!(
            ticks_eight < ticks_four,
            "8 diggers ({ticks_eight}) must beat 4 ({ticks_four})"
        );
        assert_eq!(
            four.ticks_to_close(4),
            0,
            "the four-digger camp closes inside its counted ticks"
        );
    }

    #[test]
    fn an_unfinished_rampart_blocks_less_than_a_finished_one() {
        let mut half = Job::new(WorkKind::Rampart, 100);
        half.crew_tick(50);
        let mut full = Job::new(WorkKind::Rampart, 100);
        full.crew_tick(100);
        assert!(half.blocking_permille() < full.blocking_permille());
        assert_eq!(full.blocking_permille(), 1000);
        assert!(half.blocking_permille() > 0, "half a rampart still holds");
        let fresh = Job::new(WorkKind::Rampart, 100);
        assert_eq!(fresh.blocking_permille(), 0, "a fresh work holds nothing");
    }

    #[test]
    fn the_camp_replays_twice_equal() {
        let mut a = CampWorks::new(CampPlan::ONE_NIGHT);
        let mut b = CampWorks::new(CampPlan::ONE_NIGHT);
        for hands in [3, 7, 4, 8, 5, 9, 2, 6, 1, 4] {
            a.crew_tick(hands);
            b.crew_tick(hands);
        }
        assert_eq!(a.hash(), b.hash(), "worked the same way, the same camp");
        let mid = a.hash();
        for hands in [2, 8, 6, 4, 12, 7, 3, 5] {
            a.crew_tick(hands);
            b.crew_tick(hands);
        }
        assert_eq!(a.hash(), b.hash());
        assert_ne!(mid, a.hash(), "the work changed the camp");
    }

    #[test]
    fn deconstruct_refunds_what_is_invested() {
        let mut camp = CampWorks::new(CampPlan::ONE_NIGHT);
        camp.crew_tick(40);
        let index = 0;
        let before = camp.jobs()[index].done;
        let refund = camp.deconstruct(index).expect("the ditch job exists");
        assert_eq!(refund, before, "the refund is the invested work");
        assert_eq!(camp.jobs()[index].done, 0, "the job is fresh again");
        let again = camp.deconstruct(index).expect("the ditch job exists");
        assert_eq!(again, 0, "a fresh job refunds nothing");
        assert!(
            !camp.perimeter_closed(),
            "the scored-back ditch reopens the camp"
        );
    }

    #[test]
    fn the_plan_lays_the_four_kinds_and_the_perimeter_closes() {
        let camp = CampWorks::new(CampPlan::ONE_NIGHT);
        let kinds: Vec<WorkKind> = camp.jobs().iter().map(|j| j.kind).collect();
        assert_eq!(
            &kinds[..3],
            &[WorkKind::Ditch, WorkKind::Rampart, WorkKind::StakeLine]
        );
        assert_eq!(kinds[3], WorkKind::Gate);
        assert_eq!(kinds.len(), 5, "three perimeter works and two gates");
        assert!(!camp.perimeter_closed(), "a fresh camp is not closed");
        // The 50 x 50 m perimeter in ditch steps: 200 000 mm of ditch at 200 mm
        // per crew step is 1000 steps, so 2000 ticks at two hands per work unit.
        let ditch = camp.jobs()[0];
        assert_eq!(ditch.total, 1000);
        assert_eq!(ditch.kind, WorkKind::Ditch);
        let mut worked = camp.clone();
        while !worked.perimeter_closed() {
            worked.crew_tick(2000);
        }
        assert!(worked.perimeter_closed());
        assert!(worked.ticks_to_close(8) == 0);
    }
}
