//! wait_stats: blocking-wait and GPU-object-creation tally per frame kind
//! plus latency percentiles.
//!
//! Owner: render lane, consumer the windowed presenter and the arena smoke.
//! Units: call counts for waits and for buffers and bind groups created,
//! milliseconds for timings. A "blocking
//! call" is a buffer `map_async` that the CPU then waits on, or a device
//! poll that waits for the GPU: the two things a live frame must never do
//! (Game Engine Gems 1 ch 9.6, 11.3). Frames fall in three kinds: live
//! (no probe work), probe (the opt-in look probes ran) and capture (the
//! offscreen twin, which reads pixels back by design).
//! Determinism: pure counters; the same call sequence gives the same tally.
//! Errors: none. Budget: a handful of integer adds per frame, no allocation.

/// What kind of frame a tally entry belongs to.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FrameKind {
    /// A window frame that ran no probe work: must never block.
    Live,
    /// A window frame that ran the opt-in look probes or fog tranche.
    Probe,
    /// An offscreen capture frame (reads pixels back by design).
    Capture,
}

/// Blocking calls and GPU objects created inside one frame.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct FrameWaits {
    /// Buffer maps the CPU waited on.
    pub maps: u32,
    /// Device polls that waited for the GPU.
    pub waits: u32,
    /// GPU buffers created since the previous frame ended (uploads the
    /// game made between frames count toward the frame that follows).
    pub buffers: u32,
    /// GPU bind groups created since the previous frame ended.
    pub bind_groups: u32,
}

impl FrameWaits {
    /// Record one blocking readback: a map request plus the poll wait that
    /// completes it.
    pub fn map_and_wait(&mut self) {
        self.maps = self.maps.saturating_add(1);
        self.waits = self.waits.saturating_add(1);
    }

    /// Record `n` blocking readbacks made by submitters in other modules
    /// (each does one submit, one map and one wait).
    pub fn map_and_wait_n(&mut self, n: u32) {
        self.maps = self.maps.saturating_add(n);
        self.waits = self.waits.saturating_add(n);
    }

    /// True when the frame never blocked.
    #[must_use]
    pub fn is_clear(&self) -> bool {
        self.maps == 0 && self.waits == 0
    }

    /// True when the frame created no buffer and no bind group.
    #[must_use]
    pub fn created_nothing(&self) -> bool {
        self.buffers == 0 && self.bind_groups == 0
    }
}

/// Buffers and bind groups the presenter created, counted at every
/// creation site: a per-frame count the frame end drains into
/// [`FrameWaits`] plus cumulative totals for tests. Lock-free so `&self`
/// helpers can count.
#[derive(Debug, Default)]
pub struct CreateTally {
    frame_buffers: std::sync::atomic::AtomicU32,
    frame_bind_groups: std::sync::atomic::AtomicU32,
    total_buffers: std::sync::atomic::AtomicU64,
    total_bind_groups: std::sync::atomic::AtomicU64,
}

impl CreateTally {
    /// Count one buffer creation.
    pub fn buffer(&self) {
        use std::sync::atomic::Ordering::Relaxed;
        self.frame_buffers.fetch_add(1, Relaxed);
        self.total_buffers.fetch_add(1, Relaxed);
    }

    /// Count one bind group creation.
    pub fn bind_group(&self) {
        use std::sync::atomic::Ordering::Relaxed;
        self.frame_bind_groups.fetch_add(1, Relaxed);
        self.total_bind_groups.fetch_add(1, Relaxed);
    }

    /// Drain the per-frame counts: (buffers, bind groups) since the last
    /// drain. The totals keep running.
    pub fn take_frame(&self) -> (u32, u32) {
        use std::sync::atomic::Ordering::Relaxed;
        (
            self.frame_buffers.swap(0, Relaxed),
            self.frame_bind_groups.swap(0, Relaxed),
        )
    }

    /// Cumulative (buffers, bind groups) created since the presenter was
    /// built.
    pub fn totals(&self) -> (u64, u64) {
        use std::sync::atomic::Ordering::Relaxed;
        (
            self.total_buffers.load(Relaxed),
            self.total_bind_groups.load(Relaxed),
        )
    }
}

/// Running totals per frame kind since the presenter was built.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct WaitStats {
    /// Live frames presented.
    pub live_frames: u64,
    /// Buffer maps waited on across live frames (must stay 0).
    pub live_maps: u64,
    /// Blocking device polls across live frames (must stay 0).
    pub live_waits: u64,
    /// Buffers created across live frames (the first frame builds the
    /// targets, the rest should add none).
    pub live_buffers: u64,
    /// Bind groups created across live frames.
    pub live_bind_groups: u64,
    /// Buffers created on live frames after the first one: 0 means no
    /// per-frame buffer creation.
    pub steady_buffers: u64,
    /// Bind groups created on live frames after the first one: 0 means no
    /// per-frame bind group creation.
    pub steady_bind_groups: u64,
    /// Probe frames presented.
    pub probe_frames: u64,
    /// Buffer maps waited on across probe frames.
    pub probe_maps: u64,
    /// Blocking device polls across probe frames.
    pub probe_waits: u64,
    /// Capture frames rendered.
    pub capture_frames: u64,
    /// Buffer maps waited on across capture frames.
    pub capture_maps: u64,
    /// Blocking device polls across capture frames.
    pub capture_waits: u64,
}

impl WaitStats {
    /// Fold one finished frame into the totals for its kind.
    pub fn fold(&mut self, kind: FrameKind, frame: FrameWaits) {
        let (frames, maps, waits) = match kind {
            FrameKind::Live => (
                &mut self.live_frames,
                &mut self.live_maps,
                &mut self.live_waits,
            ),
            FrameKind::Probe => (
                &mut self.probe_frames,
                &mut self.probe_maps,
                &mut self.probe_waits,
            ),
            FrameKind::Capture => (
                &mut self.capture_frames,
                &mut self.capture_maps,
                &mut self.capture_waits,
            ),
        };
        let after_first = *frames > 0;
        *frames = frames.saturating_add(1);
        *maps = maps.saturating_add(u64::from(frame.maps));
        *waits = waits.saturating_add(u64::from(frame.waits));
        if kind == FrameKind::Live {
            let (buffers, groups) = (u64::from(frame.buffers), u64::from(frame.bind_groups));
            self.live_buffers = self.live_buffers.saturating_add(buffers);
            self.live_bind_groups = self.live_bind_groups.saturating_add(groups);
            if after_first {
                self.steady_buffers = self.steady_buffers.saturating_add(buffers);
                self.steady_bind_groups = self.steady_bind_groups.saturating_add(groups);
            }
        }
    }

    /// Buffers plus bind groups created on live frames after the first:
    /// the number the frame-cost rows hold at zero.
    #[must_use]
    pub fn steady_creates(&self) -> u64 {
        self.steady_buffers.saturating_add(self.steady_bind_groups)
    }

    /// Blocking calls (maps plus waits) across live frames: the number the
    /// arena smoke fails on when it is above zero.
    #[must_use]
    pub fn live_blocking(&self) -> u64 {
        self.live_maps.saturating_add(self.live_waits)
    }

    /// One-line receipt for the smoke log.
    #[must_use]
    pub fn summary(&self) -> String {
        format!(
            "live_frames={} live_maps={} live_waits={} live_buffers={} live_bind_groups={} steady_buffers={} steady_bind_groups={} probe_frames={} probe_maps={} probe_waits={} capture_frames={} capture_maps={} capture_waits={}",
            self.live_frames,
            self.live_maps,
            self.live_waits,
            self.live_buffers,
            self.live_bind_groups,
            self.steady_buffers,
            self.steady_bind_groups,
            self.probe_frames,
            self.probe_maps,
            self.probe_waits,
            self.capture_frames,
            self.capture_maps,
            self.capture_waits,
        )
    }
}

/// Median and 99th percentile of a sample set, in the samples' unit.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct Percentiles {
    /// Median.
    pub p50: f32,
    /// 99th percentile (nearest rank).
    pub p99: f32,
}

/// Percentiles of `samples` (sorted in place). `None` for an empty set or
/// when any sample is not finite. Nearest-rank: the value at index
/// `ceil(q * n) - 1`.
#[must_use]
pub fn percentiles(samples: &mut [f32]) -> Option<Percentiles> {
    if samples.is_empty() || samples.iter().any(|s| !s.is_finite()) {
        return None;
    }
    // `sort_unstable_by`, not `sort_by`: the stable sort monomorphises
    // `core::slice::sort::shared::smallsort`, a std-private shim that this
    // toolchain (rustc 1.97.1) fails to emit in this crate, so the render
    // test binary failed to link with an undefined
    // `smallsort::insert_tail` symbol. The unstable sort uses the pdqsort
    // path instead. The result is the same array: `total_cmp` is a total
    // order, so no two samples compare equal and the sorted order is
    // unique - stability cannot change it.
    samples.sort_unstable_by(f32::total_cmp);
    let rank = |numerator: usize| {
        let n = samples.len();
        let idx = (n * numerator).div_ceil(100).max(1) - 1;
        samples[idx.min(n - 1)]
    };
    Some(Percentiles {
        p50: rank(50),
        p99: rank(99),
    })
}

/// GPU timing summary of the timestamped passes, resolved late and
/// non-blocking. Milliseconds on the device clock.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct GpuTimingReport {
    /// Frames with a resolved sample.
    pub frames: u64,
    /// Forward pass plus own-pass sky plus blit per frame.
    pub total: Percentiles,
    /// Forward pass alone.
    pub forward: Percentiles,
    /// Blit alone.
    pub blit: Percentiles,
    /// Own-pass sky alone over the frames that ran it; zero when none did.
    pub sky: Percentiles,
    /// GTAO compute pass alone over the frames that ran it; zero when none did.
    pub gtao: Percentiles,
    /// QL12 particle draw pass alone over the frames that ran it; zero when
    /// none did.
    pub particle: Percentiles,
    /// Frames that ran the particle draw pass (its query pair was written).
    pub particle_frames: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fold_keeps_live_probe_and_capture_apart() {
        let mut stats = WaitStats::default();
        stats.fold(FrameKind::Live, FrameWaits::default());
        let mut probe = FrameWaits::default();
        probe.map_and_wait_n(4);
        probe.map_and_wait();
        stats.fold(FrameKind::Probe, probe);
        let mut capture = FrameWaits::default();
        capture.map_and_wait();
        stats.fold(FrameKind::Capture, capture);
        assert_eq!(stats.live_frames, 1);
        assert_eq!(stats.live_blocking(), 0, "a clear live frame adds nothing");
        assert_eq!(
            (stats.probe_frames, stats.probe_maps, stats.probe_waits),
            (1, 5, 5)
        );
        assert_eq!((stats.capture_frames, stats.capture_maps), (1, 1));
        assert!(probe.maps == 5 && !probe.is_clear());
        assert!(stats.summary().contains("probe_maps=5"));
    }

    #[test]
    fn steady_creates_skip_the_first_live_frame() {
        let mut stats = WaitStats::default();
        // First live frame builds targets: 3 buffers, 2 bind groups.
        stats.fold(
            FrameKind::Live,
            FrameWaits {
                buffers: 3,
                bind_groups: 2,
                ..FrameWaits::default()
            },
        );
        assert_eq!((stats.live_buffers, stats.live_bind_groups), (3, 2));
        assert_eq!(stats.steady_creates(), 0, "frame 1 is the warm-up");
        stats.fold(FrameKind::Live, FrameWaits::default());
        assert_eq!(stats.steady_creates(), 0);
        // A probe frame never counts toward the live tally.
        stats.fold(
            FrameKind::Probe,
            FrameWaits {
                buffers: 9,
                ..FrameWaits::default()
            },
        );
        assert_eq!(stats.live_buffers, 3);
        // A live frame that creates a bind group later is a regression.
        stats.fold(
            FrameKind::Live,
            FrameWaits {
                bind_groups: 1,
                ..FrameWaits::default()
            },
        );
        assert_eq!((stats.steady_buffers, stats.steady_bind_groups), (0, 1));
        assert_eq!(stats.steady_creates(), 1);
        assert!(stats.summary().contains("steady_bind_groups=1"));
    }

    #[test]
    fn create_tally_drains_per_frame_and_keeps_totals() {
        let tally = CreateTally::default();
        tally.buffer();
        tally.buffer();
        tally.bind_group();
        assert_eq!(tally.take_frame(), (2, 1));
        assert_eq!(tally.take_frame(), (0, 0), "drained");
        tally.buffer();
        assert_eq!(tally.totals(), (3, 1), "totals survive the drain");
        assert!(FrameWaits::default().created_nothing());
    }

    #[test]
    fn a_blocking_live_frame_is_counted_loudly() {
        let mut stats = WaitStats::default();
        let mut frame = FrameWaits::default();
        frame.map_and_wait();
        stats.fold(FrameKind::Live, frame);
        assert_eq!(stats.live_blocking(), 2, "one map plus one wait");
    }

    #[test]
    fn percentiles_use_nearest_rank_and_refuse_bad_input() {
        let mut v: Vec<f32> = (1..=100).map(|i| i as f32).collect();
        let p = percentiles(&mut v).expect("percentiles");
        assert_eq!((p.p50, p.p99), (50.0, 99.0));
        let mut one = [7.0f32];
        let p = percentiles(&mut one).expect("single sample");
        assert_eq!((p.p50, p.p99), (7.0, 7.0));
        assert!(percentiles(&mut []).is_none());
        assert!(percentiles(&mut [1.0, f32::NAN]).is_none());
        let mut unsorted = [9.0f32, 1.0, 5.0];
        assert_eq!(percentiles(&mut unsorted).map(|p| p.p50), Some(5.0));
    }
}
