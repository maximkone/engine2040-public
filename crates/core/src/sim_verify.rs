//! sim_verify: the deterministic simulation testing harness.
//! Owner: engine-core (std-only; no dependency, no IO, no clock in the sim path).
//! Units: ticks (u32), seeds, words and digests (u64), overhead in nanoseconds.
//! Determinism: one tick order, one event order, one seeded counter stream per
//! run. The same seed and the same schedule give the same checkpoints on every
//! platform, so two runs of one seed are compared tick for tick.
//! Errors: none by construction; a mismatch is a report, never a panic.
//! Budget: O(ticks + events) per run; the steady tick path allocates nothing.
//!
//! Shape: the deterministic sim-test shape of madsim in plain integer Rust: a
//! seeded run, the same seed run twice with the checkpoints compared (the
//! determinism check), and seeded failure injection (the injected events), so
//! one seed names one failure and replays it.
//!
//! PORT(madsim-rs/madsim@519950ef:madsim-macros/src/lib.rs and
//! madsim/src/sim/buggify.rs, Apache-2.0): the seeded sim-test shape and the
//! seeded failure injection, reimplemented std-only. No vendor bytes, no
//! dependency, no async runtime.
//! EVIDENCE: docs/EVIDENCE.md, madsim row.

use crate::checked::{saturating_u32_to_u16, saturating_usize_to_u32, u32_from_u64};
use crate::counter_rng::{CounterCursor, CounterRng};
use crate::digest::{DigestChain, StableId};

/// The slot every tick folds under when no injected event names a subject:
/// the world record of that tick.
const WORLD_SLOT: u32 = 0;

/// The capacity a run of `ticks` ticks needs. Every supported target's
/// `usize` holds a `u32`, so the fallback (a zero capacity, which only means
/// the vector grows) never runs.
fn capacity_of(ticks: u32) -> usize {
    usize::try_from(ticks).unwrap_or_default()
}

/// One event injected into a run at a fixed tick.
///
/// The event is data: the harness delivers it, the scenario decides what it
/// means. A seed names the event stream, so one seed replays one failure.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct InjectedEvent {
    /// The tick the event fires at.
    pub tick: u32,
    /// The subject the event addresses, as a slot index.
    pub subject: u32,
    /// The event kind: an opaque tag the scenario reads.
    pub kind: u16,
    /// The event payload: an opaque word the scenario reads.
    pub payload: u64,
}

/// The events one run injects, kept in tick order.
///
/// Two events of one tick keep the order they were added in, so the same
/// schedule always delivers the same events in the same order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EventSchedule {
    events: Vec<InjectedEvent>,
}

impl EventSchedule {
    /// An empty schedule: a run with no injected event.
    #[must_use]
    pub fn new() -> Self {
        Self { events: Vec::new() }
    }

    /// A seeded schedule: `count` events spread over `ticks` ticks, drawn from
    /// the counter stream of `seed`.
    ///
    /// The same seed gives the same schedule on every platform, so a seed
    /// names one failure and replays it. Every event lands inside the run.
    #[must_use]
    pub fn seeded(seed: u64, count: u32, ticks: u32) -> Self {
        let rng = CounterRng::new(seed);
        let span = u64::from(ticks.max(1));
        let mut events = Vec::with_capacity(capacity_of(count));
        for index in 0..count {
            let word = rng.sample(u64::from(index));
            // Every draw is reduced below the span or a small bound first, so
            // the checked narrowings below never refuse.
            let tick = u32_from_u64(word % span);
            let subject = u32_from_u64((word >> 32) % 16);
            let kind = saturating_u32_to_u16(u32_from_u64((word >> 52) % 4));
            events.push(InjectedEvent {
                tick,
                subject,
                kind,
                payload: word,
            });
        }
        // A stable sort keeps the events of one tick in the order they were
        // drawn, so the delivery order is the drawing order.
        events.sort_by_key(|event| event.tick);
        Self { events }
    }

    /// Add one event, keeping the schedule in tick order.
    ///
    /// The event joins the events of its own tick at the end of them, so the
    /// delivery order of a tick never changes under a later insert.
    pub fn push(&mut self, event: InjectedEvent) {
        let at = self
            .events
            .partition_point(|other| other.tick <= event.tick);
        self.events.insert(at, event);
    }

    /// The whole schedule, in tick order.
    #[must_use]
    pub fn events(&self) -> &[InjectedEvent] {
        &self.events
    }

    /// The events firing at `tick`, in delivery order.
    #[must_use]
    pub fn at(&self, tick: u32) -> &[InjectedEvent] {
        let start = self.events.partition_point(|event| event.tick < tick);
        let end = start + self.events[start..].partition_point(|event| event.tick == tick);
        &self.events[start..end]
    }

    /// The number of events in the schedule.
    #[must_use]
    pub fn len(&self) -> usize {
        self.events.len()
    }

    /// True when the schedule holds no event.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }
}

/// The simulation the harness drives.
///
/// The scenario owns its state and decides what an injected event means; the
/// harness owns the tick order, the event order and the checkpoint chain.
pub trait Scenario {
    /// Advance one tick with the events firing at it and return the digest of
    /// the new state.
    ///
    /// The digest is folded into the run's checkpoint chain, so it must cover
    /// every part of the state a later tick depends on. The stream is the
    /// run's seeded counter stream: a scenario that draws from it draws the
    /// same words in every run of the same seed.
    fn step(&mut self, tick: u32, events: &[InjectedEvent], stream: &mut CounterCursor) -> u64;
}

/// A monotonic clock the harness reads to measure its own overhead.
///
/// The harness never reads a wall clock itself: the caller owns the clock, so
/// a simulation crate keeps its no-clock rule and the harness stays
/// deterministic. A clock reading never enters a checkpoint.
pub trait Clock {
    /// Nanoseconds since an arbitrary fixed origin. Only differences matter.
    fn now_ns(&self) -> u64;
}

/// Any closure that reads a monotonic nanosecond counter is a clock.
impl<F: Fn() -> u64> Clock for F {
    fn now_ns(&self) -> u64 {
        self()
    }
}

/// One run of a scenario under the harness.
#[derive(Clone, Debug)]
pub struct SimRun {
    chain: DigestChain,
    fired: u32,
    overhead_ns: Vec<u64>,
}

impl SimRun {
    /// Run `scenario` for `ticks` ticks under `seed`, injecting `schedule`.
    ///
    /// With a clock the harness also measures its own bookkeeping per tick
    /// (the event dispatch and the checkpoint fold) and never the scenario's
    /// own work: the overhead is what the harness adds to the simulation.
    #[must_use]
    pub fn run<S: Scenario>(
        scenario: &mut S,
        seed: u64,
        ticks: u32,
        schedule: &EventSchedule,
        clock: Option<&dyn Clock>,
    ) -> Self {
        let count = capacity_of(ticks);
        let mut chain = DigestChain::with_capacity(count, count);
        let mut stream = CounterCursor::new(seed);
        let mut overhead_ns = Vec::with_capacity(count);
        let mut cursor = 0usize;
        let mut fired = 0u32;
        for tick in 0..ticks {
            let opened = clock.map(Clock::now_ns);
            let start = cursor;
            while cursor < schedule.len() && schedule.events()[cursor].tick <= tick {
                cursor += 1;
            }
            let due = &schedule.events()[start..cursor];
            fired = fired.saturating_add(saturating_usize_to_u32(due.len()));
            let dispatched = clock.map(Clock::now_ns);
            let digest = scenario.step(tick, due, &mut stream);
            let stepped = clock.map(Clock::now_ns);
            chain.push_tick(tick, &[(StableId::slot(WORLD_SLOT), digest)]);
            let folded = clock.map(Clock::now_ns);
            if let (Some(opened), Some(dispatched), Some(stepped), Some(folded)) =
                (opened, dispatched, stepped, folded)
            {
                let overhead = dispatched
                    .saturating_sub(opened)
                    .saturating_add(folded.saturating_sub(stepped));
                overhead_ns.push(overhead);
            }
        }
        Self {
            chain,
            fired,
            overhead_ns,
        }
    }

    /// The checkpoint chain: one chain digest per tick.
    #[must_use]
    pub fn chain(&self) -> &DigestChain {
        &self.chain
    }

    /// The chain digest after the last tick.
    #[must_use]
    pub fn digest(&self) -> u64 {
        self.chain.digest()
    }

    /// The number of events delivered during the run.
    #[must_use]
    pub fn fired(&self) -> u32 {
        self.fired
    }

    /// The harness overhead of each tick in nanoseconds; empty without a clock.
    #[must_use]
    pub fn overhead_ns(&self) -> &[u64] {
        &self.overhead_ns
    }
}

/// Where two runs of one scenario parted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SimDivergence {
    /// The first tick whose checkpoint chain differs.
    pub tick: u32,
    /// The stable id of the record that differs there.
    pub id: StableId,
}

/// The verdict of a verification sweep.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Verdict {
    /// The seeds that ran, in order.
    pub seeds: Vec<u64>,
    /// The ticks every run covered.
    pub ticks: u32,
    /// The events the first run delivered.
    pub events: u32,
    /// The seed whose two runs parted, if any.
    pub seed: Option<u64>,
    /// The first divergence found, if any.
    pub divergence: Option<SimDivergence>,
}

/// Run one plan under every seed, twice per seed, and compare the two runs.
///
/// `plan` builds the scenario and its schedule from the seed alone, so two
/// calls with one seed give the same run. Every seed's two runs must match
/// tick for tick: the first pair that parts is the verdict's divergence.
///
/// This is the determinism check. A scenario that drifts (a wall clock, a hash
/// order, an unseeded draw) parts the two runs and is reported, never hidden.
#[must_use]
pub fn verify<S, P>(ticks: u32, seeds: &[u64], mut plan: P) -> Verdict
where
    S: Scenario,
    P: FnMut(u64) -> (S, EventSchedule),
{
    let mut verdict = Verdict {
        seeds: seeds.to_vec(),
        ticks,
        events: 0,
        seed: None,
        divergence: None,
    };
    let mut counted = false;
    for &seed in seeds {
        let (mut scenario, schedule) = plan(seed);
        let first = SimRun::run(&mut scenario, seed, ticks, &schedule, None);
        let (mut scenario, again) = plan(seed);
        let second = SimRun::run(&mut scenario, seed, ticks, &again, None);
        if !counted {
            verdict.events = first.fired();
            counted = true;
        }
        if let Some(found) = first.chain().first_divergence(second.chain()) {
            verdict.seed = Some(seed);
            verdict.divergence = Some(SimDivergence {
                tick: found.tick,
                id: found.id,
            });
            break;
        }
    }
    verdict
}

/// The `percentile`-th percentile of the samples (0 to 100), in nanoseconds.
///
/// The samples are copied before the sort, so the caller keeps its own order.
#[must_use]
pub fn percentile_ns(samples: &[u64], percentile: u32) -> u64 {
    if samples.is_empty() {
        return 0;
    }
    let mut sorted = samples.to_vec();
    sorted.sort_unstable();
    let length = u64::try_from(sorted.len()).unwrap_or(u64::MAX);
    let rank = u64::from(percentile.min(100)) * length / 100;
    let index = usize::try_from(rank)
        .unwrap_or(usize::MAX)
        .min(sorted.len() - 1);
    sorted[index]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    /// A rules-as-code scenario: a rule table, one state word per subject and
    /// the events that fire an extra rule at a tick. The rules are data, so
    /// the same table and the same events give the same state everywhere.
    struct RuleSim {
        rules: Vec<(u16, u64)>,
        state: Vec<u64>,
    }

    impl RuleSim {
        fn new(seed: u64) -> Self {
            let rng = CounterRng::new(seed);
            let rules = (0..4u64)
                .map(|index| {
                    let word = rng.sample(index);
                    (
                        u16::try_from(word % 4).expect("rule kind fits in u16"),
                        word | 1,
                    )
                })
                .collect();
            Self {
                rules,
                state: vec![seed; 8],
            }
        }
    }

    impl Scenario for RuleSim {
        fn step(&mut self, tick: u32, events: &[InjectedEvent], stream: &mut CounterCursor) -> u64 {
            let mut digest = u64::from(tick);
            for (slot, value) in self.state.iter_mut().enumerate() {
                let (kind, factor) = self.rules[slot % self.rules.len()];
                *value = value
                    .wrapping_mul(factor)
                    .wrapping_add(u64::from(kind))
                    .wrapping_add(stream.next_u64());
                digest = digest.rotate_left(7) ^ *value;
            }
            let subjects = u64::try_from(self.state.len()).expect("subject count fits in u64");
            for event in events {
                let slot = usize::try_from(u64::from(event.subject) % subjects)
                    .expect("slot fits in usize");
                self.state[slot] = self.state[slot]
                    .wrapping_add(event.payload)
                    .wrapping_mul(u64::from(event.kind) | 1);
                digest ^= event.payload.rotate_left(17);
            }
            digest
        }
    }

    /// The soak the row asks for: 10,000 ticks with injected events under
    /// three seeds, every seed run twice, zero divergence.
    #[test]
    fn ten_thousand_ticks_with_injected_events_have_zero_divergence() {
        const TICKS: u32 = 10_000;
        const SEEDS: [u64; 3] = [0, 1, 2];
        const EVENTS: u32 = 256;
        let verdict = verify(TICKS, &SEEDS, |seed| {
            (
                RuleSim::new(seed),
                EventSchedule::seeded(seed, EVENTS, TICKS),
            )
        });
        assert_eq!(verdict.ticks, TICKS);
        assert_eq!(verdict.seeds, SEEDS.to_vec());
        assert_eq!(verdict.events, EVENTS, "every injected event must fire");
        assert_eq!(verdict.seed, None);
        assert_eq!(verdict.divergence, None, "the soak must not diverge");
    }

    /// The kill rule: a scenario that drifts parts the two runs of one seed,
    /// and the harness names the tick where the drift started.
    #[test]
    fn a_drifting_scenario_is_reported_and_never_hidden() {
        static CALLS: AtomicU64 = AtomicU64::new(0);

        struct Drifting {
            state: u64,
        }

        impl Scenario for Drifting {
            fn step(
                &mut self,
                tick: u32,
                _events: &[InjectedEvent],
                _stream: &mut CounterCursor,
            ) -> u64 {
                self.state = self
                    .state
                    .wrapping_add(CALLS.fetch_add(1, Ordering::Relaxed));
                self.state ^ u64::from(tick)
            }
        }

        let verdict = verify(64, &[3], |_seed| {
            (Drifting { state: 0 }, EventSchedule::new())
        });
        assert_eq!(verdict.seed, Some(3), "the drifting seed must be named");
        let divergence = verdict.divergence.expect("the drift must be reported");
        assert_eq!(divergence.tick, 0, "the drift starts at the first tick");
    }

    /// One injected event moves the run, and the part is named exactly.
    #[test]
    fn an_injected_event_moves_the_run_and_names_its_tick() {
        const TICKS: u32 = 600;
        let seed = 7u64;
        let schedule = EventSchedule::seeded(seed, 8, TICKS);
        let mut changed = schedule.clone();
        changed.push(InjectedEvent {
            tick: 100,
            subject: 3,
            kind: 1,
            payload: 0xdead_beef,
        });
        let mut left = RuleSim::new(seed);
        let run_left = SimRun::run(&mut left, seed, TICKS, &schedule, None);
        let mut right = RuleSim::new(seed);
        let run_right = SimRun::run(&mut right, seed, TICKS, &changed, None);
        let divergence = run_left
            .chain()
            .first_divergence(run_right.chain())
            .expect("the two runs must part");
        assert_eq!(divergence.tick, 100, "the part is at the event's tick");
        assert_eq!(schedule.at(100).len() + 1, changed.at(100).len());
    }

    /// Every injected event reaches the scenario at its own tick, in order.
    #[test]
    fn every_injected_event_reaches_the_scenario_at_its_tick() {
        struct Recorder {
            seen: Vec<u32>,
        }

        impl Scenario for Recorder {
            fn step(
                &mut self,
                tick: u32,
                events: &[InjectedEvent],
                _stream: &mut CounterCursor,
            ) -> u64 {
                for event in events {
                    self.seen.push(tick);
                    assert_eq!(event.tick, tick, "the event fires at its own tick");
                }
                u64::from(tick)
            }
        }

        let schedule = EventSchedule::seeded(11, 40, 500);
        let mut recorder = Recorder { seen: Vec::new() };
        let run = SimRun::run(&mut recorder, 11, 500, &schedule, None);
        assert_eq!(run.fired(), 40, "every event fires");
        let ticks: Vec<u32> = schedule.events().iter().map(|event| event.tick).collect();
        assert_eq!(recorder.seen, ticks, "the events arrive in schedule order");
    }

    /// The seeded schedule is the same every time, and another seed gives
    /// another schedule.
    #[test]
    fn the_seeded_schedule_is_the_same_every_time_and_in_tick_order() {
        let one = EventSchedule::seeded(2040, 32, 1000);
        let two = EventSchedule::seeded(2040, 32, 1000);
        assert_eq!(one.events(), two.events());
        assert_eq!(one.len(), 32);
        assert!(
            one.events()
                .windows(2)
                .all(|pair| pair[0].tick <= pair[1].tick),
            "the schedule is in tick order"
        );
        let other = EventSchedule::seeded(2041, 32, 1000);
        assert_ne!(
            one.events(),
            other.events(),
            "another seed gives another schedule"
        );
    }

    /// The overhead bar: the harness adds at most 0.1 ms per tick at p95.
    ///
    /// Run: cargo test -p engine-core --lib sim_verify --release -- --ignored
    #[test]
    #[ignore = "timing: p95 harness overhead under the 0.1 ms bar; run in release"]
    #[expect(
        clippy::disallowed_types,
        clippy::disallowed_methods,
        reason = "the wall clock times the experiment, never the tick"
    )]
    fn p95_harness_overhead_stays_under_the_bar() {
        const TICKS: u32 = 10_000;
        const P95_BAR_NS: u64 = 100_000;
        let schedule = EventSchedule::seeded(2040, 256, TICKS);
        let mut sim = RuleSim::new(2040);
        let start = std::time::Instant::now();
        let clock = || u64::try_from(start.elapsed().as_nanos()).unwrap_or(u64::MAX);
        let run = SimRun::run(&mut sim, 2040, TICKS, &schedule, Some(&clock));
        let p95 = percentile_ns(run.overhead_ns(), 95);
        println!(
            "sim_verify overhead: p95 {p95} ns per tick over {} ticks",
            run.overhead_ns().len()
        );
        assert!(
            p95 <= P95_BAR_NS,
            "p95 {p95} ns per tick is over the {P95_BAR_NS} ns (0.1 ms) bar"
        );
    }

    /// The seeded fields stay inside their bounds: subject is 0..16 and kind
    /// is 0..4, and neither field collapses to a constant.
    #[test]
    fn seeded_event_fields_stay_inside_their_bounds() {
        let schedule = EventSchedule::seeded(2040, 256, 1000);
        let subjects: Vec<u32> = schedule
            .events()
            .iter()
            .map(|event| event.subject)
            .collect();
        let kinds: Vec<u32> = schedule
            .events()
            .iter()
            .map(|event| u32::from(event.kind))
            .collect();
        assert_eq!(subjects.len(), 256);
        assert_eq!(
            subjects.iter().filter(|subject| **subject < 16).count(),
            256,
            "subject is (word >> 32) % 16"
        );
        assert!(
            subjects.iter().any(|subject| *subject != 0),
            "subjects are not all zero"
        );
        assert_eq!(
            kinds.iter().filter(|kind| **kind < 4).count(),
            256,
            "kind is (word >> 52) % 4"
        );
        assert!(
            kinds.iter().any(|kind| *kind != 0),
            "kinds are not all zero"
        );
    }

    /// `at` returns the events of its own tick and nothing else.
    #[test]
    fn at_returns_only_its_own_tick() {
        let mut schedule = EventSchedule::new();
        for (tick, payload) in [(5u32, 0xa1u64), (5, 0xb2), (9, 0xc3)] {
            schedule.push(InjectedEvent {
                tick,
                subject: 1,
                kind: 0,
                payload,
            });
        }
        let at_five: Vec<u64> = schedule.at(5).iter().map(|event| event.payload).collect();
        assert_eq!(at_five, vec![0xa1, 0xb2]);
        let at_nine: Vec<u64> = schedule.at(9).iter().map(|event| event.payload).collect();
        assert_eq!(at_nine, vec![0xc3]);
        assert_eq!(schedule.at(6).len(), 0, "a tick with no event is empty");
    }

    /// `is_empty` follows the event count.
    #[test]
    fn is_empty_tracks_the_schedule() {
        assert!(EventSchedule::new().is_empty());
        let one = EventSchedule::seeded(1, 1, 10);
        assert!(!one.is_empty());
        assert_eq!(one.len(), 1);
    }

    /// A plain closure is a clock, and it returns its own reading.
    #[test]
    fn a_closure_is_a_clock() {
        let clock = || 42u64;
        let as_clock: &dyn Clock = &clock;
        assert_eq!(as_clock.now_ns(), 42);
    }

    /// The run digest is the chain digest, and the harness overhead per tick is
    /// exactly the two clock readings it takes around dispatch and fold.
    #[test]
    fn run_digest_and_overhead_are_the_measured_ones() {
        let mut sim = RuleSim::new(7);
        let run = SimRun::run(&mut sim, 7, 5, &EventSchedule::new(), None);
        assert_eq!(run.digest(), run.chain().digest());
        assert_ne!(run.digest(), 0, "a five-tick run folds a real chain");

        let calls = AtomicU64::new(0);
        let clock = || calls.fetch_add(1, Ordering::Relaxed);
        let as_clock: &dyn Clock = &clock;
        let mut sim = RuleSim::new(7);
        let timed = SimRun::run(&mut sim, 7, 5, &EventSchedule::new(), Some(as_clock));
        assert_eq!(timed.overhead_ns(), &[2, 2, 2, 2, 2]);
        let mut sim = RuleSim::new(7);
        let untimed = SimRun::run(&mut sim, 7, 5, &EventSchedule::new(), None);
        assert_eq!(untimed.overhead_ns(), &[] as &[u64]);
    }

    /// The percentile reads an exact ranked sample, keeps the caller's order and
    /// clamps a rank past the end to the last sample.
    #[test]
    fn percentile_picks_the_ranked_sample() {
        assert_eq!(percentile_ns(&[], 50), 0);
        let samples = [50u64, 10, 30, 40, 20];
        assert_eq!(percentile_ns(&samples, 0), 10);
        assert_eq!(percentile_ns(&samples, 50), 30);
        assert_eq!(percentile_ns(&samples, 100), 50);
        assert_eq!(samples, [50, 10, 30, 40, 20], "the caller keeps its order");
    }
}
