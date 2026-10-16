//! voice: offline voice-bank bake. Runtime reads only; live TTS always refuses.
use crate::dsp::xorshift_step;
use engine_core::hash::GOLDEN_GAMMA;
use engine_core::{hash_bytes, Fnv64};
use std::fmt;
/// Bake frame grid in ms (whisper.cpp 10ms pattern).
pub const FRAME_MS: u32 = 10;
/// Dialogue loudness cap in LUFS (integrated). Bank passes when all <= cap.
pub const MAX_DIALOGUE_LUFS: f32 = -16.0;
/// Floor returned for empty/silent clips (avoids -inf in hashes).
pub const SILENCE_LUFS: f32 = -120.0;
/// Silence phoneme id (reserved; real phonemes are 1..=MAX).
pub const SILENCE_PH: u8 = 0;
/// Max baked phoneme id (compact viseme-bridge range).
pub const MAX_PHONEME_ID: u8 = 39;
/// One baked phoneme span; monotonic, non-overlapping, 10ms-quantized.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Phoneme {
    pub id: u8,
    pub start_ms: u32,
    pub end_ms: u32,
}
/// Baked dialogue line: lookup key + baked clip ref + phoneme timeline.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VoiceLine {
    pub key: String,
    pub clip_id: u32,
    pub phonemes: Vec<Phoneme>,
    pub dur_ms: u32,
}
/// WAV-bank registry row: baked clip meta. PCM lives on disk, never here.
#[derive(Clone, Debug, PartialEq)]
pub struct WavEntry {
    pub id: u32,
    pub path: String,
    pub content_hash: u64,
    pub bytes: u64,
    pub lufs: f32,
    pub sample_rate: u32,
}
/// Baked bank: lines + wavs. Insertion order feeds SHA; bake sorted for stability.
#[derive(Clone, Debug, Default)]
pub struct VoiceBank {
    pub sample_rate: u32,
    pub lines: Vec<VoiceLine>,
    pub wavs: Vec<WavEntry>,
}
/// Voice errors; `LiveSynthRefused` is the runtime TTS ban.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VoiceError {
    LiveSynthRefused,
    Duplicate,
    NotFound,
}
impl fmt::Display for VoiceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::LiveSynthRefused => write!(f, "live synth refused: bake-only"),
            Self::Duplicate => write!(f, "duplicate id/key"),
            Self::NotFound => write!(f, "not found"),
        }
    }
}
impl std::error::Error for VoiceError {}
/// Length as `u64`; `usize` never exceeds 64 bits, so the clamp is unreachable.
fn len_u64(len: usize) -> u64 {
    u64::try_from(len).unwrap_or(u64::MAX)
}
/// Deterministic char->phoneme id; 0 reserved for silence.
pub fn phoneme_id_for_char(c: u8) -> u8 {
    let l = c.to_ascii_lowercase();
    if l == b' ' {
        return SILENCE_PH;
    }
    1 + (l.wrapping_mul(31).wrapping_add(7) % MAX_PHONEME_ID)
}
/// Bake a phoneme timeline from text (offline forced-align stub).
/// Lowercase a-z0-9 kept, space = 40ms gap, other bytes skipped.
/// Durations 60..120ms on 10ms grid, seeded via (seed,key,text).
pub fn bake_line(key: &str, text: &str, clip_id: u32, seed: u64) -> VoiceLine {
    let mut st = seed ^ (hash_bytes(key.as_bytes()).wrapping_mul(GOLDEN_GAMMA));
    st ^= hash_bytes(text.as_bytes()).wrapping_add(0xbf58476d1ce4e5b9);
    let mut cursor = 0u32;
    let mut ph = Vec::new();
    for b in text.bytes() {
        let l = b.to_ascii_lowercase();
        let keep = matches!(l, b'a'..=b'z' | b'0'..=b'9' | b' ');
        if !keep {
            continue;
        }
        if l == b' ' {
            cursor += 4 * FRAME_MS;
            continue;
        }
        let dur = 60 + ((xorshift_step(&mut st) % 7) as u32) * FRAME_MS;
        ph.push(Phoneme {
            id: phoneme_id_for_char(l),
            start_ms: cursor,
            end_ms: cursor + dur,
        });
        cursor += dur + FRAME_MS;
    }
    let dur_ms = ph.last().map(|p| p.end_ms).unwrap_or(0);
    VoiceLine {
        key: key.into(),
        clip_id,
        phonemes: ph,
        dur_ms,
    }
}
/// Sample baked timeline at t_ms; None on gap/out-of-range (viseme bridge).
pub fn sample_line(line: &VoiceLine, t_ms: u32) -> Option<u8> {
    for p in &line.phonemes {
        if t_ms >= p.start_ms && t_ms < p.end_ms {
            return Some(p.id);
        }
    }
    None
}
/// Direct-form I biquad with a0 already divided out.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Biquad {
    b: [f64; 3],
    a1: f64,
    a2: f64,
}

#[derive(Clone, Copy, Debug, Default)]
struct BiquadState {
    x1: f64,
    x2: f64,
    y1: f64,
    y2: f64,
}

impl Biquad {
    fn step(&self, st: &mut BiquadState, x: f64) -> f64 {
        let y = self.b[0] * x + self.b[1] * st.x1 + self.b[2] * st.x2
            - self.a1 * st.y1
            - self.a2 * st.y2;
        st.x2 = st.x1;
        st.x1 = x;
        st.y2 = st.y1;
        st.y1 = y;
        y
    }
}

/// BS.1770 K-weighting stages for `sample_rate`: the high-shelf pre-filter and
/// the RLB high-pass. 48 kHz returns the standard's published coefficients;
/// other rates recompute them from the same analog prototypes.
fn k_stages(sample_rate: u32) -> (Biquad, Biquad) {
    if sample_rate == 48_000 {
        let shelf = Biquad {
            b: [1.535_124_859_586_97, -2.691_696_189_406_38, 1.198_392_810_852_85],
            a1: -1.690_659_293_182_41,
            a2: 0.732_480_774_215_85,
        };
        let rlb = Biquad {
            b: [1.0, -2.0, 1.0],
            a1: -1.990_047_454_833_98,
            a2: 0.990_072_250_366_21,
        };
        return (shelf, rlb);
    }
    prototype_stages(f64::from(sample_rate))
}

/// The K-weighting prototypes bilinear-transformed at `fs` Hz.
fn prototype_stages(fs: f64) -> (Biquad, Biquad) {
    // Stage 1: high shelf, f0 1681.97 Hz, +3.99984 dB, Q 0.707175.
    let (f0, gain_db, q) = (
        1_681.974_450_955_533,
        3.999_843_853_973_347,
        0.707_175_236_955_419_6,
    );
    let k = (std::f64::consts::PI * f0 / fs).tan();
    let vh = 10f64.powf(gain_db / 20.0);
    let vb = vh.powf(0.499_666_774_154_541_6);
    let a0 = 1.0 + k / q + k * k;
    let shelf = Biquad {
        b: [
            (vh + vb * k / q + k * k) / a0,
            2.0 * (k * k - vh) / a0,
            (vh - vb * k / q + k * k) / a0,
        ],
        a1: 2.0 * (k * k - 1.0) / a0,
        a2: (1.0 - k / q + k * k) / a0,
    };
    // Stage 2: RLB high-pass, f0 38.135 Hz, Q 0.500327. Numerator [1, -2, 1].
    let (f0, q) = (38.135_470_876_024_44, 0.500_327_037_323_877_3);
    let k = (std::f64::consts::PI * f0 / fs).tan();
    let a0 = 1.0 + k / q + k * k;
    let rlb = Biquad {
        b: [1.0, -2.0, 1.0],
        a1: 2.0 * (k * k - 1.0) / a0,
        a2: (1.0 - k / q + k * k) / a0,
    };
    (shelf, rlb)
}

/// Integrated LUFS at 48 kHz; see [`integrated_lufs_at`].
pub fn integrated_lufs(samples: &[f32]) -> f32 {
    integrated_lufs_at(samples, 48_000)
}

/// Ungated integrated LUFS of mono `samples` at `sample_rate` Hz: ITU-R BS.1770
/// K-weighting (high shelf, then RLB high-pass), mean square, minus 0.691 dB.
/// No gating is applied. Empty/silent -> SILENCE_LUFS. Rate 0 falls back to
/// 48 kHz. Deterministic on samples.
pub fn integrated_lufs_at(samples: &[f32], sample_rate: u32) -> f32 {
    if samples.is_empty() {
        return SILENCE_LUFS;
    }
    let rate = if sample_rate == 0 { 48_000 } else { sample_rate };
    let (shelf, rlb) = k_stages(rate);
    let (mut shelf_st, mut rlb_st) = (BiquadState::default(), BiquadState::default());
    let mut sum = 0f64;
    for &s in samples {
        let y = rlb.step(&mut rlb_st, shelf.step(&mut shelf_st, f64::from(s)));
        sum += y * y;
    }
    let ms = sum / samples.len() as f64;
    if ms <= 1e-12 {
        return SILENCE_LUFS;
    }
    (10.0 * ms.log10() - 0.691) as f32
}
/// LUFS cap check: passes when lufs <= cap.
pub fn passes_lufs_cap(lufs: f32, cap: f32) -> bool {
    lufs <= cap
}
/// Bake a registry row; content_hash = FNV over WAV bytes (baked offline).
pub fn bake_wav(id: u32, path: &str, wav_bytes: &[u8], lufs: f32, sample_rate: u32) -> WavEntry {
    WavEntry {
        id,
        path: path.into(),
        content_hash: hash_bytes(wav_bytes),
        bytes: len_u64(wav_bytes.len()),
        lufs,
        sample_rate,
    }
}
/// Bank SHA (FNV-1a64) over sample_rate, wavs, lines, phonemes.
pub fn bank_sha(b: &VoiceBank) -> u64 {
    let mut h = Fnv64::new();
    h.write_u32(b.sample_rate);
    h.write_u64(len_u64(b.wavs.len()));
    for w in &b.wavs {
        h.write_u32(w.id);
        h.write_str(&w.path);
        h.write_u64(w.content_hash);
        h.write_u64(w.bytes);
        h.write_f32(w.lufs);
        h.write_u32(w.sample_rate);
    }
    h.write_u64(len_u64(b.lines.len()));
    for l in &b.lines {
        h.write_str(&l.key);
        h.write_u8(0xff);
        h.write_u32(l.clip_id);
        h.write_u64(len_u64(l.phonemes.len()));
        for p in &l.phonemes {
            h.write_u8(p.id);
            h.write_u32(p.start_ms);
            h.write_u32(p.end_ms);
        }
        h.write_u32(l.dur_ms);
    }
    h.finish()
}
/// Bank SHA as 16-char hex (accept artifact).
pub fn bank_sha_hex(b: &VoiceBank) -> String {
    format!("{:016x}", bank_sha(b))
}
/// Live synth entry point: always refuses (bake-only law, no runtime TTS).
pub fn synthesize_live(_text: &str) -> Result<VoiceLine, VoiceError> {
    Err(VoiceError::LiveSynthRefused)
}
impl VoiceBank {
    /// New empty bank at `sample_rate`.
    pub fn new(sample_rate: u32) -> Self {
        Self {
            sample_rate,
            lines: Vec::new(),
            wavs: Vec::new(),
        }
    }
    /// Register a baked WAV; Err on duplicate id.
    pub fn push_wav(&mut self, e: WavEntry) -> Result<(), VoiceError> {
        if self.wavs.iter().any(|w| w.id == e.id) {
            return Err(VoiceError::Duplicate);
        }
        self.wavs.push(e);
        Ok(())
    }
    /// Register a baked line; Err on duplicate key.
    pub fn push_line(&mut self, l: VoiceLine) -> Result<(), VoiceError> {
        if self.lines.iter().any(|x| x.key == l.key) {
            return Err(VoiceError::Duplicate);
        }
        self.lines.push(l);
        Ok(())
    }
    /// Runtime lookup: baked line by key (pure read, never synths).
    pub fn find_line(&self, key: &str) -> Option<&VoiceLine> {
        self.lines.iter().find(|l| l.key == key)
    }
    /// Runtime lookup: baked wav by id (pure read).
    pub fn find_wav(&self, id: u32) -> Option<&WavEntry> {
        self.wavs.iter().find(|w| w.id == id)
    }
    /// Runtime lookup: clip id for line key; NotFound if unbaked (no fallback).
    pub fn clip_for(&self, key: &str) -> Result<u32, VoiceError> {
        self.find_line(key)
            .map(|l| l.clip_id)
            .ok_or(VoiceError::NotFound)
    }
    /// Peak (max) baked LUFS; empty bank -> SILENCE_LUFS.
    pub fn peak_lufs(&self) -> f32 {
        self.wavs
            .iter()
            .map(|w| w.lufs)
            .fold(SILENCE_LUFS, f32::max)
    }
    /// Bank cap check: true when every entry <= cap (vacuous on empty).
    pub fn bank_passes_cap(&self, cap: f32) -> bool {
        self.wavs.iter().all(|w| passes_lufs_cap(w.lufs, cap))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn bank() -> VoiceBank {
        let mut b = VoiceBank::new(48_000);
        b.push_wav(bake_wav(7, "vo/hi.wav", b"RIFF....hi", -20.0, 48_000))
            .unwrap();
        b.push_line(bake_line("hi", "hi there", 7, 2040)).unwrap();
        b
    }
    #[test]
    fn bake_deterministic_bits() {
        let (a, b) = (
            bake_line("hi", "Hi, there!", 7, 2040),
            bake_line("hi", "Hi, there!", 7, 2040),
        );
        assert_eq!(a, b);
        assert_ne!(
            bake_line("hi", "hi there", 7, 2040),
            bake_line("hi", "hi there!", 7, 2041)
        );
    }
    #[test]
    fn timeline_monotonic_quantized() {
        let l = bake_line("t", "abc 123", 1, 17);
        assert!(!l.phonemes.is_empty());
        let mut prev = 0;
        for p in &l.phonemes {
            assert!(p.id != SILENCE_PH && p.id <= MAX_PHONEME_ID);
            assert!(p.start_ms % FRAME_MS == 0 && p.end_ms % FRAME_MS == 0);
            assert!(p.start_ms >= prev && p.end_ms > p.start_ms);
            prev = p.start_ms;
            assert_eq!(Some(p.id), sample_line(&l, p.start_ms));
        }
        assert_eq!(l.dur_ms, l.phonemes.last().unwrap().end_ms);
        assert_eq!(sample_line(&l, u32::MAX), None);
        assert_eq!(bake_line("e", "", 1, 1).phonemes.len(), 0);
    }
    #[test]
    fn lufs_floor_and_cap() {
        assert_eq!(integrated_lufs(&[]), SILENCE_LUFS);
        assert_eq!(integrated_lufs(&[0.0; 64]), SILENCE_LUFS);
        let loud: Vec<f32> = (0..48_000)
            .map(|i| (2.0 * std::f32::consts::PI * 440.0 * i as f32 / 48_000.0).sin() * 0.5)
            .collect();
        let l = integrated_lufs(&loud);
        assert!((l - -9.7).abs() < 0.3, "l={l}");
        assert!(!passes_lufs_cap(l, MAX_DIALOGUE_LUFS));
        assert!(passes_lufs_cap(SILENCE_LUFS, MAX_DIALOGUE_LUFS));
        assert!(passes_lufs_cap(-16.0, -16.0) && !passes_lufs_cap(-15.9, -16.0));
    }
    #[test]
    fn k_weighting_reads_1khz_minus20_peak_as_minus23_lufs() {
        // 1 kHz sine, sample peak -20 dBFS (amplitude 0.1), one second.
        for rate in [48_000u32, 44_100, 96_000] {
            let sine: Vec<f32> = (0..rate)
                .map(|i| {
                    let t = f64::from(i) / f64::from(rate);
                    ((2.0 * std::f64::consts::PI * 1000.0 * t).sin() * 0.1) as f32
                })
                .collect();
            let l = integrated_lufs_at(&sine, rate);
            assert!((l + 23.0).abs() <= 0.5, "rate {rate}: l={l}");
        }
    }
    #[test]
    fn k_prototype_reproduces_published_48k_coefficients() {
        let (shelf, rlb) = prototype_stages(48_000.0);
        let (pub_shelf, pub_rlb) = k_stages(48_000);
        let close = |x: f64, y: f64| (x - y).abs() < 1e-6;
        for (x, y) in shelf.b.iter().zip(pub_shelf.b.iter()) {
            assert!(close(*x, *y), "shelf b {x} vs {y}");
        }
        assert!(close(shelf.a1, pub_shelf.a1) && close(shelf.a2, pub_shelf.a2));
        for (x, y) in rlb.b.iter().zip(pub_rlb.b.iter()) {
            assert!(close(*x, *y), "rlb b {x} vs {y}");
        }
        assert!(close(rlb.a1, pub_rlb.a1) && close(rlb.a2, pub_rlb.a2));
    }
    #[test]
    fn bank_sha_stable_and_sensitive() {
        let (a, b) = (bank(), bank());
        assert_eq!(bank_sha(&a), bank_sha(&b));
        assert_eq!(bank_sha_hex(&a), bank_sha_hex(&b));
        assert_eq!(bank_sha_hex(&a).len(), 16);
        let mut c = bank();
        c.wavs[0].lufs = -10.0;
        assert_ne!(bank_sha(&a), bank_sha(&c));
    }
    #[test]
    fn registry_lookup_and_dupes() {
        let mut b = bank();
        assert_eq!(b.clip_for("hi"), Ok(7));
        assert_eq!(b.clip_for("missing"), Err(VoiceError::NotFound));
        assert_eq!(b.find_wav(7).unwrap().path, "vo/hi.wav");
        assert_eq!(b.find_wav(99), None);
        assert_eq!(
            b.push_wav(bake_wav(7, "vo/dup.wav", b"x", -20.0, 48_000)),
            Err(VoiceError::Duplicate)
        );
        assert_eq!(
            b.push_line(bake_line("hi", "yo", 7, 1)),
            Err(VoiceError::Duplicate)
        );
        assert!(b.bank_passes_cap(MAX_DIALOGUE_LUFS));
        assert!(!b.bank_passes_cap(-30.0));
        assert_eq!(b.peak_lufs(), -20.0);
    }
    #[test]
    fn live_synth_refused() {
        assert_eq!(synthesize_live("hello"), Err(VoiceError::LiveSynthRefused));
        assert_eq!(synthesize_live(""), Err(VoiceError::LiveSynthRefused));
    }
}
