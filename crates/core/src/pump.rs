//! pump: off-tick jobs stepped one slice per frame, with the tick every
//! slice arrived on recorded.
//! Owner: engine-core (std-only). Units: frames (u64), ticks (u32), digests
//! (u64). Determinism: the pump never reads a clock; the caller names the
//! tick of every frame, so the same frames give the same arrivals and the
//! same chain. Errors: none. Budget: one slice per frame; an idle pump
//! allocates nothing and touches no job.

use crate::digest::{DigestChain, Divergence, StableId};

/// A job that runs outside the fixed tick, one slice per frame.
///
/// The job hashes one stable subject per slice and hands the digest back,
/// so the pump can fold it into a [`DigestChain`]. A slice that returns
/// `None` ends the job: the pump then does nothing at all.
pub trait SliceJob {
    /// The stable id the job hashes; a divergence names it.
    fn id(&self) -> StableId;

    /// One slice of work at `tick`; the digest of what the slice hashed.
    /// `None` when the job has nothing left to do.
    fn step(&mut self, tick: u32) -> Option<u64>;
}

/// Steps an off-tick job one slice per frame and records the tick every
/// slice arrived on.
///
/// The pump is the glue between the frame loop and the work that must not
/// run inside the fixed tick. It never reads a clock: the caller names the
/// tick of the frame, and the pump records that arrival, so a run can be
/// replayed and a divergence can be named by tick.
///
/// The chain and the arrival list grow as the pump runs. A caller that knows
/// the tick count should build the pump with [`SlicePump::with_capacity`],
/// which keeps the frame path free of allocation.
#[derive(Clone, Debug)]
pub struct SlicePump<J: SliceJob> {
    /// The job the pump drives.
    job: J,
    /// The chain of the job's per-tick digests.
    chain: DigestChain,
    /// The tick every slice arrived on, in slice order.
    arrivals: Vec<u32>,
    /// The frames the pump ran.
    frames: u64,
    /// `true` once the job reported that it is done.
    done: bool,
}

impl<J: SliceJob> SlicePump<J> {
    /// A pump over `job`, with room for `ticks` frames.
    #[must_use]
    pub fn with_capacity(job: J, ticks: usize) -> Self {
        Self {
            job,
            chain: DigestChain::with_capacity(ticks, ticks),
            arrivals: Vec::with_capacity(ticks),
            frames: 0,
            done: false,
        }
    }

    /// A pump over `job`.
    #[must_use]
    pub fn new(job: J) -> Self {
        Self::with_capacity(job, 0)
    }

    /// Run one slice of the job at `tick`.
    ///
    /// Returns `true` when the job is done: it finished on this frame, or
    /// it was already done. A done job is never stepped again, so an idle
    /// pump costs one branch per frame.
    pub fn frame(&mut self, tick: u32) -> bool {
        self.frames += 1;
        if self.done {
            return true;
        }
        let Some(digest) = self.job.step(tick) else {
            self.done = true;
            return true;
        };
        self.arrivals.push(tick);
        self.chain.push_tick(tick, &[(self.job.id(), digest)]);
        false
    }

    /// The tick every slice arrived on, in slice order.
    #[must_use]
    pub fn arrivals(&self) -> &[u32] {
        &self.arrivals
    }

    /// The chain of the job's per-tick digests.
    #[must_use]
    pub fn chain(&self) -> &DigestChain {
        &self.chain
    }

    /// The job the pump drives.
    #[must_use]
    pub fn job(&self) -> &J {
        &self.job
    }

    /// The frames the pump ran.
    #[must_use]
    pub fn frames(&self) -> u64 {
        self.frames
    }

    /// `true` when the job has nothing left to do.
    #[must_use]
    pub fn is_done(&self) -> bool {
        self.done
    }

    /// Where this pump and another parted: the first divergent tick and the
    /// stable id of the subject that differs there.
    #[must_use]
    pub fn divergence(&self, other: &Self) -> Option<Divergence> {
        self.chain.first_divergence(&other.chain)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::alloc_probe::measure;

    /// A job that hashes the tick it runs at and stops after `slices` slices.
    struct CountingJob {
        /// The stable id the job hashes.
        subject: StableId,
        /// The slices left before the job is done.
        left: u32,
        /// The slices the job ran.
        ran: u32,
    }

    impl CountingJob {
        /// A job with `slices` slices left to run.
        fn new(subject: StableId, slices: u32) -> Self {
            Self {
                subject,
                left: slices,
                ran: 0,
            }
        }
    }

    impl SliceJob for CountingJob {
        fn id(&self) -> StableId {
            self.subject
        }

        fn step(&mut self, tick: u32) -> Option<u64> {
            if self.left == 0 {
                return None;
            }
            self.left -= 1;
            self.ran += 1;
            Some(u64::from(tick).wrapping_mul(7).wrapping_add(1))
        }
    }

    /// A job whose digest follows the tick until `flip`, then differs.
    struct FlippingJob {
        /// The stable id the job hashes.
        subject: StableId,
        /// The tick the job starts to differ at.
        flip: u32,
    }

    impl SliceJob for FlippingJob {
        fn id(&self) -> StableId {
            self.subject
        }

        fn step(&mut self, tick: u32) -> Option<u64> {
            let base = u64::from(tick);
            Some(if tick >= self.flip {
                base.wrapping_add(1_000)
            } else {
                base
            })
        }
    }

    #[test]
    fn a_pump_with_slices_left_is_not_done() {
        let mut pump = SlicePump::with_capacity(CountingJob::new(StableId::slot(2), 2), 4);
        assert!(!pump.is_done(), "nothing ran yet");
        assert!(!pump.frame(1), "the first slice is not the last");
        assert!(!pump.is_done(), "one slice left");
        assert!(!pump.frame(2), "the second slice is still work");
        assert!(!pump.is_done(), "the job is done only when a step reports it");
        assert!(pump.frame(3), "the step after the last slice ends the job");
        assert!(pump.is_done());
    }

    #[test]
    fn pump_steps_one_slice_per_frame_and_records_the_tick_arrivals() {
        let mut pump = SlicePump::with_capacity(CountingJob::new(StableId::slot(3), 3), 8);
        assert!(!pump.frame(10), "the job has slices left");
        assert!(!pump.frame(11));
        assert!(!pump.frame(12));
        assert!(pump.frame(13), "the job is done after its last slice");
        assert_eq!(pump.arrivals(), &[10, 11, 12], "one arrival per slice");
        assert_eq!(pump.chain().tick_count(), 3, "one chain tick per slice");
        assert_eq!(pump.frames(), 4, "the frames ran");
        assert_eq!(pump.job().ran, 3, "the job ran once per slice");
        assert!(pump.is_done());
    }

    #[test]
    fn the_same_frames_give_the_same_arrivals_and_the_same_chain() {
        let run = || {
            let mut pump = SlicePump::with_capacity(CountingJob::new(StableId::slot(5), 4), 8);
            for tick in 20..24 {
                pump.frame(tick);
            }
            (pump.arrivals().to_vec(), pump.chain().digest())
        };
        let (first_arrivals, first_chain) = run();
        let (second_arrivals, second_chain) = run();
        assert_eq!(first_arrivals, second_arrivals);
        assert_eq!(
            first_chain, second_chain,
            "the same frames give the same chain"
        );
    }

    #[test]
    fn an_idle_pump_allocates_nothing_and_calls_no_job() {
        let mut pump = SlicePump::with_capacity(CountingJob::new(StableId::slot(1), 0), 4);
        assert!(pump.frame(0), "a job with no slices left is done");
        let ((), heap) = measure(|| {
            for tick in 1..1_000u32 {
                assert!(pump.frame(tick));
            }
        });
        assert_eq!(heap.calls, 0, "the idle pump must not allocate");
        assert_eq!(pump.job().ran, 0, "the idle pump must not call the job");
        assert!(pump.arrivals().is_empty(), "no slice, no arrival");
        assert_eq!(pump.chain().tick_count(), 0, "no slice, no chain tick");
    }

    #[test]
    fn pump_divergence_names_the_first_divergent_tick_and_the_subject() {
        let mut left = SlicePump::with_capacity(
            FlippingJob {
                subject: StableId::slot(7),
                flip: 50,
            },
            16,
        );
        let mut right = SlicePump::with_capacity(
            FlippingJob {
                subject: StableId::slot(7),
                flip: u32::MAX,
            },
            16,
        );
        for tick in 40..60u32 {
            left.frame(tick);
            right.frame(tick);
        }
        assert_eq!(
            left.divergence(&right),
            Some(Divergence {
                tick: 50,
                id: StableId::slot(7)
            }),
            "the first divergent tick and the subject are named"
        );
        assert_eq!(
            right.divergence(&left).map(|found| found.tick),
            Some(50),
            "the tick is symmetric"
        );
    }

    /// Idle pump cost: the wall cost of a frame with no job left.
    ///
    /// The perf budgets are measured in milliseconds per frame, so an idle
    /// frame below one microsecond sits far under their noise floor.
    /// Run: cargo test -p engine-core --lib pump --release -- --ignored
    #[test]
    #[ignore = "timing experiment: prints ns per idle frame against a bound"]
    #[expect(
        clippy::disallowed_types,
        clippy::disallowed_methods,
        reason = "the wall clock times the experiment, never the tick"
    )]
    fn idle_pump_frame_cost_stays_under_the_perf_noise_floor() {
        use std::time::Instant;
        let mut pump = SlicePump::with_capacity(CountingJob::new(StableId::slot(1), 0), 4);
        assert!(pump.frame(0), "the job is done");
        let frames = 1_000_000u32;
        let start = Instant::now();
        for tick in 1..=frames {
            std::hint::black_box(pump.frame(tick));
        }
        let elapsed = start.elapsed();
        let per_frame = elapsed.as_nanos() as f64 / f64::from(frames);
        println!("idle pump: {per_frame:.1} ns per frame over {frames} frames");
        assert!(
            per_frame < 1_000.0,
            "the idle pump frame cost {per_frame} ns is over the one microsecond bound"
        );
    }
}
