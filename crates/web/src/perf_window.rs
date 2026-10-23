//! QW10: draw calls and CPU milliseconds per frame, measured by the page itself. The stand-in
//! ring draws the same parts every frame, so a frame that records that list once and only swaps
//! in the clock's camera does less work than one that rebuilds every item. `?bundle=ab` measures
//! both ways in a single run: the arms alternate in blocks after a warm-up, and the receipt
//! carries each arm's median and tail plus the two captures' hashes.
//!
//! Pure numbers, no wasm: the page calls in with its frame times, the receipt prints [`Run::json`].

use std::collections::VecDeque;

use crate::report::Object;

/// Frames a run draws before the first measured block: pipelines, the tier clause and the
/// allocator settle, so a block measures the frame and not the page's start.
pub const WARMUP_FRAMES: u64 = 45;
/// Frames in one block. Long enough for a median to mean something, short enough that both arms
/// interleave inside one run (a slow drift in the tab hits both).
pub const BLOCK_FRAMES: u64 = 20;
/// Blocks per arm: three gives each arm 60 samples.
pub const BLOCKS_PER_ARM: u32 = 3;
/// Samples a single-arm receipt keeps (a plain page, or `?bundle=off`).
pub const WINDOW_FRAMES: usize = 60;
/// Fewest samples a single-arm receipt needs before it prints numbers.
pub const FEWEST_SAMPLES: usize = 8;

/// Which way the page builds its static draw list for a frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Every frame rebuilds the list, as the page did before QW10.
    Off,
    /// Every frame replays the list recorded once and swaps in the camera.
    On,
    /// Both, in alternating blocks, so one run measures the difference.
    Ab,
}

impl Mode {
    /// `?bundle=off` (the default, the page as it was), `?bundle=on` or `?bundle=ab`; anything
    /// else reads as `off`.
    #[must_use]
    pub fn parse(word: Option<&str>) -> Self {
        match word {
            Some("on") => Self::On,
            Some("ab") => Self::Ab,
            _ => Self::Off,
        }
    }

    /// The word the receipt prints for this mode.
    #[must_use]
    pub fn word(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::On => "on",
            Self::Ab => "ab",
        }
    }
}

/// One side of the A/B.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Arm {
    /// The list is rebuilt every frame.
    Off,
    /// The list recorded once is replayed with this frame's camera.
    On,
}

impl Arm {
    /// The word the receipt prints for this arm.
    #[must_use]
    pub fn word(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::On => "on",
        }
    }

    /// The slot this arm's samples and capture live in.
    #[must_use]
    pub fn index(self) -> usize {
        match self {
            Self::Off => 0,
            Self::On => 1,
        }
    }
}

/// One frame's numbers: the page's own CPU timer around the whole frame, the part of it that
/// built the draw list, and the draw calls the presenter issued.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sample {
    /// CPU milliseconds from the frame's first step to its submit.
    pub cpu_ms: f64,
    /// Milliseconds of `cpu_ms` that built the draw list (pose plus list).
    pub list_ms: f64,
    /// Draw calls the frame issued.
    pub draws: u32,
}

/// One arm's numbers, as the receipt prints them.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Facts {
    /// Frames measured.
    pub frames: u32,
    /// Median CPU milliseconds per frame.
    pub cpu_p50_ms: f64,
    /// 95th percentile CPU milliseconds per frame.
    pub cpu_p95_ms: f64,
    /// Worst frame measured.
    pub cpu_max_ms: f64,
    /// Median microseconds spent building the draw list.
    pub list_p50_us: f64,
    /// Most draw calls any measured frame issued.
    pub draws_max: u32,
}

impl Facts {
    /// The facts of one arm's samples (percentiles over whole arrays; a run is at most a few
    /// hundred frames).
    #[must_use]
    pub fn of(samples: &[Sample]) -> Self {
        if samples.is_empty() {
            return Self::default();
        }
        let mut cpu: Vec<f64> = samples.iter().map(|s| s.cpu_ms).collect();
        cpu.sort_by(f64::total_cmp);
        let mut list: Vec<f64> = samples.iter().map(|s| s.list_ms * 1000.0).collect();
        list.sort_by(f64::total_cmp);
        Self {
            frames: u32::try_from(samples.len()).unwrap_or(u32::MAX),
            cpu_p50_ms: percentile(&cpu, 50),
            cpu_p95_ms: percentile(&cpu, 95),
            cpu_max_ms: cpu.last().copied().unwrap_or(0.0),
            list_p50_us: percentile(&list, 50),
            draws_max: samples.iter().map(|s| s.draws).max().unwrap_or(0),
        }
    }

    /// This arm as a JSON object (the receipt nests it under the arm's word).
    #[must_use]
    pub fn json(&self) -> String {
        let mut o = Object::new();
        o.uint("frames", u64::from(self.frames))
            .raw("cpuP50Ms", &format!("{:.3}", self.cpu_p50_ms))
            .raw("cpuP95Ms", &format!("{:.3}", self.cpu_p95_ms))
            .raw("cpuMaxMs", &format!("{:.3}", self.cpu_max_ms))
            .raw("listP50Us", &format!("{:.1}", self.list_p50_us))
            .uint("draws", u64::from(self.draws_max));
        o.finish()
    }
}

/// `num`th percentile of a sorted slice, `num` in 0..=100: the nearest sample, no interpolation.
fn percentile(sorted: &[f64], num: usize) -> f64 {
    let last = sorted.len().saturating_sub(1);
    let index = (last * num + 50) / 100;
    sorted.get(index.min(last)).copied().unwrap_or(0.0)
}

/// One page's measurement: the arm schedule, the samples and the captures.
pub struct Run {
    mode: Mode,
    /// Rolling window for a single-arm page.
    window: VecDeque<Sample>,
    off: Vec<Sample>,
    on: Vec<Sample>,
    /// Capture hashes by [`Arm::index`], as 16 lowercase hex digits.
    captures: [Option<String>; 2],
}

impl Run {
    /// A run in `mode`. The clock starts at frame 0.
    #[must_use]
    pub fn new(mode: Mode) -> Self {
        Self {
            mode,
            window: VecDeque::with_capacity(WINDOW_FRAMES),
            off: Vec::new(),
            on: Vec::new(),
            captures: [None, None],
        }
    }

    /// The mode this run measures.
    #[must_use]
    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// The arm frame `frame` plays under, in `ab` mode: `None` during the warm-up and after the
    /// last block. A single-arm page has no schedule (`replay` answers it).
    #[must_use]
    pub fn arm_at(&self, frame: u64) -> Option<Arm> {
        if self.mode != Mode::Ab {
            return None;
        }
        let at = frame.checked_sub(WARMUP_FRAMES)?;
        let block = at / BLOCK_FRAMES;
        if block >= u64::from(BLOCKS_PER_ARM * 2) {
            return None;
        }
        Some(if block.is_multiple_of(2) {
            Arm::Off
        } else {
            Arm::On
        })
    }

    /// Does frame `frame` replay the recorded list?
    #[must_use]
    pub fn replay(&self, frame: u64) -> bool {
        match self.mode {
            Mode::Off => false,
            Mode::On => true,
            Mode::Ab => self.arm_at(frame) == Some(Arm::On),
        }
    }

    /// The frames an `ab` run measures, warm-up and tail included, so the page knows when to
    /// stop asking about it.
    #[must_use]
    pub fn measured_frames() -> u64 {
        WARMUP_FRAMES + BLOCK_FRAMES * u64::from(BLOCKS_PER_ARM * 2)
    }

    /// Take one frame's numbers. Frames outside the measured blocks are ignored.
    pub fn note(&mut self, frame: u64, sample: Sample) {
        if let Some(arm) = self.arm_at(frame) {
            let samples = match arm {
                Arm::Off => &mut self.off,
                Arm::On => &mut self.on,
            };
            let cap = usize::try_from(BLOCK_FRAMES * u64::from(BLOCKS_PER_ARM)).unwrap_or(0);
            if samples.len() < cap {
                samples.push(sample);
            }
        } else if self.mode != Mode::Ab && frame >= WARMUP_FRAMES {
            if self.window.len() == WINDOW_FRAMES {
                self.window.pop_front();
            }
            self.window.push_back(sample);
        }
    }

    /// Keep a frame's capture hash under the arm that drew it.
    pub fn note_capture(&mut self, arm: Arm, hash: &str) {
        self.captures[arm.index()] = Some(hash.to_string());
    }

    /// Both arms have their full blocks: the A/B is done.
    #[must_use]
    pub fn finished(&self, frame: u64) -> bool {
        self.mode == Mode::Ab && frame >= Self::measured_frames()
    }

    /// The receipt's `perf` object, or `None` while there is not enough to print.
    #[must_use]
    pub fn json(&self) -> Option<String> {
        match self.mode {
            Mode::Ab => self.ab_json(),
            mode => {
                if self.window.len() < FEWEST_SAMPLES {
                    return None;
                }
                let samples: Vec<Sample> = self.window.iter().copied().collect();
                let facts = Facts::of(&samples);
                let mut o = Object::new();
                o.str("mode", mode.word()).raw("arm", &facts.json());
                Some(o.finish())
            }
        }
    }

    /// The `ab` receipt: each arm's numbers, the difference and the two captures.
    fn ab_json(&self) -> Option<String> {
        let least = usize::try_from(BLOCK_FRAMES).unwrap_or(usize::MAX);
        if self.off.len() < least || self.on.len() < least {
            return None;
        }
        let off = Facts::of(&self.off);
        let on = Facts::of(&self.on);
        let mut o = Object::new();
        o.str("mode", Mode::Ab.word())
            .uint("warmup", WARMUP_FRAMES)
            .uint("block", BLOCK_FRAMES)
            .raw("off", &off.json())
            .raw("on", &on.json())
            .raw(
                "deltaCpuMs",
                &format!("{:.3}", off.cpu_p50_ms - on.cpu_p50_ms),
            )
            .raw(
                "deltaListUs",
                &format!("{:.1}", off.list_p50_us - on.list_p50_us),
            )
            .bool("drawsEqual", off.draws_max == on.draws_max)
            .raw("pixels", &self.pixels_json());
        Some(o.finish())
    }

    /// The two captures' hashes and whether they are the same picture. A missing capture is not
    /// a match.
    fn pixels_json(&self) -> String {
        let mut o = Object::new();
        let off = self.captures[Arm::Off.index()].as_deref();
        let on = self.captures[Arm::On.index()].as_deref();
        o.str("off", off.unwrap_or("none"))
            .str("on", on.unwrap_or("none"))
            .bool("equal", off.is_some() && off == on);
        o.finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(cpu_ms: f64, list_ms: f64, draws: u32) -> Sample {
        Sample {
            cpu_ms,
            list_ms,
            draws,
        }
    }

    #[test]
    fn modes_parse_and_default_to_the_plain_rebuild() {
        assert_eq!(Mode::parse(Some("off")), Mode::Off);
        assert_eq!(Mode::parse(Some("ab")), Mode::Ab);
        assert_eq!(Mode::parse(Some("on")), Mode::On);
        assert_eq!(
            Mode::parse(None),
            Mode::Off,
            "a page without the flag behaves as it did"
        );
        assert_eq!(Mode::parse(Some("nonsense")), Mode::Off);
        assert_eq!(Mode::Ab.word(), "ab");
    }

    #[test]
    fn the_ab_schedule_alternates_after_a_warm_up() {
        let run = Run::new(Mode::Ab);
        assert_eq!(run.arm_at(0), None, "the warm-up is not measured");
        assert_eq!(run.arm_at(WARMUP_FRAMES - 1), None);
        assert_eq!(run.arm_at(WARMUP_FRAMES), Some(Arm::Off), "block 0 is off");
        assert_eq!(run.arm_at(WARMUP_FRAMES + BLOCK_FRAMES - 1), Some(Arm::Off));
        assert_eq!(run.arm_at(WARMUP_FRAMES + BLOCK_FRAMES), Some(Arm::On));
        let last = Run::measured_frames();
        assert_eq!(
            run.arm_at(last - 1),
            Some(Arm::On),
            "the run ends on an on block"
        );
        assert_eq!(run.arm_at(last), None, "the tail is not measured");
    }

    #[test]
    fn replay_follows_the_mode_and_the_schedule() {
        assert!(!Run::new(Mode::Off).replay(100));
        assert!(Run::new(Mode::On).replay(0));
        let ab = Run::new(Mode::Ab);
        assert!(!ab.replay(10), "the warm-up rebuilds the list");
        assert!(!ab.replay(WARMUP_FRAMES));
        assert!(ab.replay(WARMUP_FRAMES + BLOCK_FRAMES));
    }

    #[test]
    fn only_block_frames_reach_an_arm() {
        let mut run = Run::new(Mode::Ab);
        run.note(0, sample(9.0, 9.0, 1));
        assert_eq!(run.off.len() + run.on.len(), 0, "the warm-up is dropped");
        for frame in WARMUP_FRAMES..WARMUP_FRAMES + BLOCK_FRAMES {
            run.note(frame, sample(1.0, 0.01, 14));
        }
        assert_eq!(run.off.len(), usize::try_from(BLOCK_FRAMES).unwrap());
        assert_eq!(run.on.len(), 0);
    }

    #[test]
    fn a_single_arm_window_keeps_the_last_sixty() {
        let mut run = Run::new(Mode::On);
        for frame in 0..(WARMUP_FRAMES + WINDOW_FRAMES as u64 + 10) {
            run.note(frame, sample(0.5, 0.002, 14));
        }
        assert_eq!(run.window.len(), WINDOW_FRAMES);
    }

    #[test]
    fn percentiles_take_the_nearest_sample() {
        let sorted = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0];
        assert_eq!(percentile(&sorted, 50), 6.0);
        assert_eq!(percentile(&sorted, 95), 10.0);
        assert_eq!(percentile(&sorted, 0), 1.0);
        assert_eq!(percentile(&sorted, 100), 10.0);
        assert_eq!(percentile(&[], 50), 0.0);
    }

    #[test]
    fn the_ab_receipt_carries_both_arms_and_the_pixel_verdict() {
        let mut run = Run::new(Mode::Ab);
        let block = usize::try_from(BLOCK_FRAMES).unwrap();
        for frame in WARMUP_FRAMES..Run::measured_frames() {
            let cpu = if run.arm_at(frame) == Some(Arm::On) {
                0.30
            } else {
                0.40
            };
            run.note(frame, sample(cpu, 0.008, 14));
        }
        let json = run.json().expect("both arms ran");
        assert!(json.contains("\"mode\":\"ab\""), "{json}");
        assert!(json.contains("\"deltaCpuMs\":0.100"), "{json}");
        assert!(json.contains("\"drawsEqual\":true"), "{json}");
        assert!(
            json.contains("\"equal\":false"),
            "no captures, no match: {json}"
        );
        assert_eq!(run.off.len(), block * 3);
        run.note_capture(Arm::Off, "0123456789abcdef");
        run.note_capture(Arm::On, "0123456789abcdef");
        let json = run.json().expect("both arms ran");
        assert!(json.contains("\"equal\":true"), "{json}");
    }

    #[test]
    fn a_plain_page_prints_one_arm() {
        let mut run = Run::new(Mode::On);
        assert!(run.json().is_none(), "too few frames to print");
        for frame in 0..(WARMUP_FRAMES + 20) {
            run.note(frame, sample(0.42, 0.006, 14));
        }
        let json = run.json().expect("twenty frames print");
        assert!(json.contains("\"mode\":\"on\""), "{json}");
        assert!(json.contains("\"draws\":14"), "{json}");
        assert!(json.contains("\"cpuP50Ms\":0.420"), "{json}");
        assert!(json.contains("\"listP50Us\":6.0"), "{json}");
    }
}
