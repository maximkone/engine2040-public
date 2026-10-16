//! dsp: bus mixer (SFX/Music/Dialogue->Master) + RMS gate + seeded song. Same seed->same samples.
use engine_core::{Fnv64, Rng};
use std::f32::consts::TAU;

/// Gate opens when rms exceeds this (-60dBFS ~= 0.001).
pub const GATE_THRESHOLD: f32 = 0.001;
/// Silence ceiling: gate stays closed below this.
pub const GATE_SILENCE: f32 = 1e-4;
/// Gate window in ms (20ms -> rate/50 frames).
pub const GATE_WINDOW_MS: u32 = 20;

/// dB gain to linear amplitude.
pub fn db_to_linear(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

/// 20ms gate window in frames (min 1).
pub fn gate_window_len(sample_rate: u32) -> usize {
    (sample_rate / (1000 / GATE_WINDOW_MS)).max(1) as usize
}

/// RMS over samples (f64 accumulate like `Mixer::render_sine`); empty -> 0.0.
pub fn rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let mut sum = 0f64;
    for &s in samples {
        sum += f64::from(s) * f64::from(s);
    }
    (sum / samples.len() as f64).sqrt() as f32
}

/// Peak absolute amplitude; empty -> 0.0.
pub fn peak(samples: &[f32]) -> f32 {
    let mut p = 0f32;
    for &s in samples {
        p = p.max(s.abs());
    }
    p
}

/// Gate decision, delegates to crate canon `rms_gate_open`.
pub fn gate_open(rms_v: f32, threshold: f32) -> bool {
    crate::rms_gate_open(rms_v, threshold)
}

/// Mix buses: SFX, Music (ducked), Dialogue (priority).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bus {
    Sfx,
    Music,
    Dialogue,
}

impl Bus {
    /// Gain in dB (canon from crate root).
    pub fn gain_db(self) -> f32 {
        match self {
            Bus::Sfx => crate::GAIN_SFX_DB,
            Bus::Music => crate::GAIN_MUSIC_DB,
            Bus::Dialogue => crate::GAIN_DIALOGUE_DB,
        }
    }
    /// Gain as linear amplitude.
    pub fn gain_linear(self) -> f32 {
        db_to_linear(self.gain_db())
    }
}

/// Stats for one rendered master buffer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MixStats {
    pub peak: f32,
    pub rms: f32,
    pub gate_open: bool,
}

/// Deterministic 3-bus mixer; fixed order SFX->Music->Dialogue->Master.
pub struct BusMixer {
    pub sample_rate: u32,
    pub g_sfx: f32,
    pub g_music: f32,
    pub g_dlg: f32,
    pub g_master: f32,
}

impl BusMixer {
    /// New with canon gains (Master 0dB, Music -12, SFX -3, Dialogue -6).
    pub fn new(sample_rate: u32) -> Self {
        Self {
            sample_rate,
            g_sfx: Bus::Sfx.gain_linear(),
            g_music: Bus::Music.gain_linear(),
            g_dlg: Bus::Dialogue.gain_linear(),
            g_master: db_to_linear(crate::GAIN_MASTER_DB),
        }
    }
    /// One frame: gained sum + master, hard-clipped to [-1,1] (rodio clip).
    pub fn render_frame(&self, sfx: f32, music: f32, dlg: f32) -> f32 {
        ((sfx * self.g_sfx + music * self.g_music + dlg * self.g_dlg) * self.g_master)
            .clamp(-1.0, 1.0)
    }
    /// Mix three equal-or-longer buses into master; length = shortest input.
    /// The plain loop stays because a measured 4-wide blocked spike with
    /// identical per-lane [`Self::render_frame`] ops ran ~17-22% slower at
    /// 800 and 48000 frames (release, same box), so the spike was reverted;
    /// [`Self::mix_into_scalar`] is the bit-identical fallback the equality
    /// tests pin.
    /// Presentation only, never hashed. No fast-math.
    pub fn mix_into(&self, sfx: &[f32], music: &[f32], dlg: &[f32], out: &mut Vec<f32>) {
        self.mix_into_scalar(sfx, music, dlg, out);
    }
    /// Scalar fallback for [`Self::mix_into`]: one [`Self::render_frame`]
    /// per sample, no blocking. Always present; the blocked path above must
    /// match these bytes exactly (see `pf5_block_matches_scalar`).
    pub fn mix_into_scalar(&self, sfx: &[f32], music: &[f32], dlg: &[f32], out: &mut Vec<f32>) {
        let n = sfx.len().min(music.len()).min(dlg.len());
        out.clear();
        out.reserve(n);
        for i in 0..n {
            out.push(self.render_frame(sfx[i], music[i], dlg[i]));
        }
    }
    /// Mix three buses into a new master buffer. Allocates: offline use
    /// only, the callback reuses a buffer through [`Self::mix_into`].
    pub fn mix(&self, sfx: &[f32], music: &[f32], dlg: &[f32]) -> Vec<f32> {
        let mut out = crate::zeroed(sfx.len().min(music.len()).min(dlg.len()));
        self.mix_into(sfx, music, dlg, &mut out);
        out
    }
    /// Peak/RMS/gate over a master buffer (gate = rms > `GATE_THRESHOLD`).
    pub fn stats(&self, master: &[f32]) -> MixStats {
        let (p, r) = (peak(master), rms(master));
        MixStats {
            peak: p,
            rms: r,
            gate_open: gate_open(r, GATE_THRESHOLD),
        }
    }
}

/// Track count for the integer-tick tween bank: one fader per music layer.
pub const N_TRACKS: usize = 4;
/// Permille resolution for integer track gains: 1000 is full.
pub const TRACK_GAIN_Q: u16 = 1000;

/// Integer-tick gain tween for one track: the clock stays in samples and the
/// gain rides as permille `0..=TRACK_GAIN_Q`, so the same sample counts give
/// the same gain steps on every platform. The caller derives the span from
/// integer sample counts (for example `crate::music::bar_samples` for a
/// one-bar crossfade) and converts the permille result to linear gain only
/// at the mix edge. No float time and no wall clock inside. Never allocates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TrackTween {
    /// Start gain in permille.
    pub from_q: u16,
    /// End gain in permille.
    pub to_q: u16,
    /// Span in samples (zero snaps to the end gain).
    pub total_samples: u64,
    /// Elapsed samples (saturates at the span).
    pub elapsed_samples: u64,
}

impl TrackTween {
    /// Build a tween from `from_q` to `to_q` over `total_samples`; gains
    /// clamp to permille. Never panics.
    pub fn new(from_q: u16, to_q: u16, total_samples: u64) -> Self {
        Self {
            from_q: from_q.min(TRACK_GAIN_Q),
            to_q: to_q.min(TRACK_GAIN_Q),
            total_samples,
            elapsed_samples: 0,
        }
    }
    /// Total span in samples.
    pub fn total(&self) -> u64 {
        self.total_samples
    }
    /// Elapsed samples.
    pub fn elapsed(&self) -> u64 {
        self.elapsed_samples
    }
    /// True once the span has run out (also true for a zero span).
    pub fn is_done(&self) -> bool {
        self.elapsed_samples >= self.total_samples
    }
    /// Move the clock forward by `samples`; saturates at the span.
    /// Never panics.
    pub fn elapse(&mut self, samples: u64) {
        self.elapsed_samples = self
            .elapsed_samples
            .saturating_add(samples)
            .min(self.total_samples);
    }
    /// Back to the start gain. Never panics.
    pub fn reset(&mut self) {
        self.elapsed_samples = 0;
    }
    /// Gain now in permille: integer lerp of the elapsed share, so the same
    /// sample counts give bit-identical steps. Never panics.
    pub fn value_q(&self) -> u16 {
        if self.total_samples == 0 {
            return self.to_q;
        }
        let capped = self.elapsed_samples.min(self.total_samples);
        let from_i = i64::from(self.from_q);
        let delta = i64::from(self.to_q) - from_i;
        let num = i64::try_from(capped).unwrap_or(i64::MAX);
        let den = i64::try_from(self.total_samples).unwrap_or(i64::MAX);
        if den <= 0 {
            return self.to_q;
        }
        let stepped = from_i + delta.saturating_mul(num) / den;
        u16::try_from(stepped.clamp(0, i64::from(TRACK_GAIN_Q))).unwrap_or(0)
    }
}

/// One xorshift64 step (13/7/17) that forces the low bit before every step.
/// The placeholder bank noise, the music bake and the voice timing replay
/// this exact stream; `engine_core::Rng` forces the bit only at seeding, so
/// it is a different stream and cannot stand in.
pub(crate) fn xorshift_step(state: &mut u64) -> u64 {
    let mut x = *state | 1;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    *state = x;
    x
}

/// Seeded 16-step pentatonic bake (0.25s/step, sine+2nd harmonic
/// with per-step decay). Same seed+rate+n -> bit-identical samples.
/// Allocates: the owned-buffer form of [`render_song_into`].
pub fn render_song(seed: u64, sample_rate: u32, n: usize) -> Vec<f32> {
    let mut out = crate::zeroed(n);
    render_song_into(seed, sample_rate, &mut out);
    out
}

/// Render the seeded song into `out` (`out.len()` frames); a zero
/// `sample_rate` or empty `out` yields silence. Allocates nothing.
pub fn render_song_into(seed: u64, sample_rate: u32, out: &mut [f32]) {
    const TABLE: [f32; 8] = [220.0, 261.63, 293.66, 329.63, 392.0, 440.0, 523.25, 587.33];
    const STEPS: usize = 16;
    if out.is_empty() || sample_rate == 0 {
        out.fill(0.0);
        return;
    }
    let mut rng = Rng::new(seed);
    let mut seq = [0usize; STEPS];
    for s in seq.iter_mut() {
        *s = (rng.next_u64() % TABLE.len() as u64) as usize;
    }
    let sr = sample_rate as f32;
    let step_len = (sample_rate / 4).max(1) as usize;
    for (i, v) in out.iter_mut().enumerate() {
        let step = (i / step_len) % STEPS;
        let pos = ((i % step_len) as f32) / (step_len as f32);
        let f = TABLE[seq[step]];
        let t = (i as f32) / sr;
        let env = 1.0 - 0.5 * pos;
        let s = (TAU * f * t).sin() * 0.4 + (2.0 * TAU * f * t).sin() * 0.15;
        *v = (s * env * 0.8).clamp(-1.0, 1.0);
    }
}

/// FNV-1a64 over the samples' f32 bits (little-endian); song SHA x2 gate.
pub fn song_hash(samples: &[f32]) -> u64 {
    let mut hasher = Fnv64::new();
    for &sample in samples {
        hasher.write_f32(sample);
    }
    hasher.finish()
}

impl crate::Mixer {
    /// Build a canon-gain bus mixer at this mixer's rate.
    pub fn bus_mixer(&self) -> BusMixer {
        BusMixer::new(self.sample_rate)
    }
    /// Render the seeded song at this mixer's rate (fundsp bake shape).
    pub fn song(&self, seed: u64, n: usize) -> Vec<f32> {
        render_song(seed, self.sample_rate, n)
    }
}

/// ADSR envelope gain 0..=1 at time `t` (seconds).
/// `note_len` includes attack+decay+sustain; release runs after.
pub fn adsr(
    t: f32,
    attack: f32,
    decay: f32,
    sustain_level: f32,
    release: f32,
    note_len: f32,
) -> f32 {
    if t < 0.0 {
        return 0.0;
    }
    if t < attack {
        return (t / attack).clamp(0.0, 1.0);
    }
    let t2 = t - attack;
    if t2 < decay {
        return 1.0 - (1.0 - sustain_level) * (t2 / decay).clamp(0.0, 1.0);
    }
    let t3 = t2 - decay;
    let sustain = (note_len - attack - decay).max(0.0);
    if t3 < sustain {
        return sustain_level;
    }
    let t4 = t3 - sustain;
    if t4 < release {
        return sustain_level * (1.0 - (t4 / release).clamp(0.0, 1.0));
    }
    0.0
}

/// One-pole lowpass state (bake helper, deterministic).
pub struct OnePole {
    /// Smoothing factor 0..1 (higher = darker).
    pub a: f32,
    y: f32,
}

impl OnePole {
    /// Construct with smoothing factor.
    pub fn new(a: f32) -> Self {
        Self {
            a: a.clamp(0.0, 1.0),
            y: 0.0,
        }
    }
    /// Step one sample, returns filtered value.
    pub fn step(&mut self, x: f32) -> f32 {
        self.y += self.a * (x - self.y);
        self.y
    }
}

/// Beat grid: BPM + offset (seconds). Bake-only metronome.
pub struct BeatGrid {
    /// Beats per minute.
    pub bpm: f32,
    /// First downbeat offset in seconds.
    pub offset: f32,
}

impl BeatGrid {
    /// Construct.
    pub fn new(bpm: f32, offset: f32) -> Self {
        Self { bpm, offset }
    }
    /// Time of beat `n` (n=0 is first downbeat).
    pub fn beat_time(&self, n: u32) -> f32 {
        self.offset + n as f32 * 60.0 / self.bpm
    }
    /// Count of beats with `beat_time <= t`.
    pub fn beats_at(&self, t: f32) -> u32 {
        if t < self.offset {
            return 0;
        }
        ((t - self.offset) * self.bpm / 60.0).floor() as u32 + 1
    }
}

#[cfg(test)]
mod worker_tests {
    use super::*;

    #[test]
    fn song_into_overwrites_stale_samples_and_silences_degenerate_input() {
        let fresh = render_song(2040, 8_000, 1_000);
        let mut dirty = vec![9.0f32; 1_000];
        render_song_into(2040, 8_000, &mut dirty);
        assert_eq!(dirty, fresh, "every slot is rewritten");
        let mut stale = vec![9.0f32; 16];
        render_song_into(2040, 0, &mut stale);
        assert!(stale.iter().all(|&s| s == 0.0), "rate 0 is silence");
    }

    #[test]
    fn adsr_shape() {
        assert_eq!(adsr(0.0, 0.1, 0.1, 0.7, 0.1, 1.0), 0.0);
        assert!((adsr(0.05, 0.1, 0.1, 0.7, 0.1, 1.0) - 0.5).abs() < 1e-6);
        assert!((adsr(0.5, 0.1, 0.1, 0.7, 0.1, 1.0) - 0.7).abs() < 1e-6);
        assert_eq!(adsr(5.0, 0.1, 0.1, 0.7, 0.1, 1.0), 0.0);
    }

    #[test]
    fn onepole_converges_to_dc() {
        let mut f = OnePole::new(0.1);
        let mut y = 0.0;
        for _ in 0..200 {
            y = f.step(1.0);
        }
        assert!((y - 1.0).abs() < 1e-3);
    }

    #[test]
    fn beat_grid_120bpm() {
        let g = BeatGrid::new(120.0, 0.0);
        assert_eq!(g.beat_time(0), 0.0);
        assert!((g.beat_time(2) - 1.0).abs() < 1e-6);
        assert_eq!(
            (g.beats_at(0.0), g.beats_at(0.99), g.beats_at(1.0)),
            (1, 2, 3)
        );
    }

    #[test]
    fn track_tween_bar_crossfade() {
        use crate::music::bar_samples;
        assert_eq!(N_TRACKS, crate::music::N_LAYERS);
        // One bar at 120 BPM and 48 kHz is four integer beat lengths.
        let total = bar_samples(48_000, 120);
        assert_eq!(total, 96_000);
        let mut tw = TrackTween::new(0, TRACK_GAIN_Q, total);
        assert_eq!((tw.total(), tw.elapsed()), (total, 0));
        assert!(!tw.is_done());
        assert_eq!(tw.value_q(), 0);
        tw.elapse(total / 2);
        assert_eq!(tw.value_q(), 500, "half the samples, half the gain");
        tw.elapse(total / 2);
        assert_eq!(tw.value_q(), TRACK_GAIN_Q);
        assert!(tw.is_done());
        // Second run replays bit-identically (integer lerp, no float clock).
        let mut again = TrackTween::new(0, TRACK_GAIN_Q, total);
        again.elapse(total);
        assert_eq!(again.value_q(), tw.value_q());
        // Failure edges: zero span snaps to the end, overruns saturate,
        // out-of-range gains clamp, reset returns to the start.
        assert_eq!(TrackTween::new(0, 750, 0).value_q(), 750);
        let mut short = TrackTween::new(2000, 0, 100);
        assert_eq!((short.from_q, short.to_q), (TRACK_GAIN_Q, 0));
        short.elapse(u64::MAX);
        assert_eq!(short.value_q(), 0);
        assert!(short.is_done());
        short.reset();
        assert_eq!((short.elapsed(), short.value_q()), (0, TRACK_GAIN_Q));
        // A downward slope midpoints exactly (would drift with float time).
        let mut down = TrackTween::new(TRACK_GAIN_Q, 0, 1000);
        down.elapse(250);
        assert_eq!(down.value_q(), 750);
    }

    /// Deterministic finite fixture: seeded LCG words mapped into
    /// `[-1.5, 1.5]` so clipping lanes are covered without NaN.
    fn pf5_fixture(len: usize, seed: u64) -> Vec<f32> {
        let mut state = seed | 1;
        let mut out = Vec::with_capacity(len);
        for _ in 0..len {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let word = (state >> 33) as u32;
            out.push(word as f32 / u32::MAX as f32 * 3.0 - 1.5);
        }
        out
    }

    fn pf5_bits(samples: &[f32]) -> u64 {
        song_hash(samples)
    }

    #[test]
    fn pf5_block_matches_scalar_bit_identical() {
        let bus = BusMixer::new(48_000);
        // Lengths cover empty, tails 1..3, one block, block+tail, tick size.
        for &len in &[0usize, 1, 2, 3, 4, 5, 7, 8, 63, 64, 65, 800, 801] {
            let sfx = pf5_fixture(len, 0x9E3779B97F4A7C15);
            let music = pf5_fixture(len, 0xC2B2AE3D27D4EB4F);
            let dlg = pf5_fixture(len, 0x165667B19E3779F9);
            let mut blocked = Vec::new();
            let mut scalar = Vec::new();
            bus.mix_into(&sfx, &music, &dlg, &mut blocked);
            bus.mix_into_scalar(&sfx, &music, &dlg, &mut scalar);
            assert_eq!(blocked.len(), len, "length {len}");
            assert_eq!(scalar.len(), len, "scalar length {len}");
            assert_eq!(blocked, scalar, "bytes differ at length {len}");
            assert_eq!(
                pf5_bits(&blocked),
                pf5_bits(&scalar),
                "hash differs at length {len}"
            );
            // `mix` stays the same constructor path as `mix_into`.
            assert_eq!(blocked, bus.mix(&sfx, &music, &dlg), "mix() moved");
        }
    }

    #[test]
    fn pf5_mix_shortest_wins_and_empty_stays_empty() {
        let bus = BusMixer::new(48_000);
        let sfx = vec![0.5f32; 8];
        let music = vec![0.25f32; 5];
        let dlg = vec![0.1f32; 10];
        let mut blocked = Vec::new();
        let mut scalar = Vec::new();
        bus.mix_into(&sfx, &music, &dlg, &mut blocked);
        bus.mix_into_scalar(&sfx, &music, &dlg, &mut scalar);
        assert_eq!(blocked.len(), 5, "must take the shortest input");
        assert_eq!(blocked, scalar, "fallback must agree on ragged inputs");
        let empty: Vec<f32> = Vec::new();
        bus.mix_into(&empty, &empty, &empty, &mut blocked);
        assert!(blocked.is_empty(), "empty in, empty out");
    }
}
