//! music: seeded adaptive music (layers + bar transitions + beat bus). Same seed->same SHA.

use crate::dsp::xorshift_step;
use engine_core::hash::GOLDEN_GAMMA;
use engine_core::{hash_bytes, Fnv64};

/// Baked-song magic: `MUS1`.
pub const MAGIC: [u8; 4] = *b"MUS1";
/// Baked-song version (bump = rehash all).
pub const VERSION: u8 = 1;
/// Beats per bar (4/4 only, barelymusician grid).
pub const BEATS_PER_BAR: u32 = 4;
/// 16th-note steps per beat.
pub const STEPS_PER_BEAT: u32 = 4;
/// Steps per bar (16).
pub const STEPS_PER_BAR: u32 = BEATS_PER_BAR * STEPS_PER_BEAT;
/// Max bakeable bars (cap keeps render bounded).
pub const MAX_BARS: u8 = 16;
/// Layer count: 0 pad, 1 bass, 2 drums, 3 lead (kira mixer shape).
pub const N_LAYERS: usize = 4;
/// Intensity gate per layer (CDDA-budget pattern, data not code).
pub const LAYER_THRESH: [f32; N_LAYERS] = [0.0, 0.3, 0.55, 0.8];

const PROG: [i32; 4] = [0, 8, 3, 10];
const PENTA: [i32; 7] = [0, 3, 5, 7, 10, 12, 15];

/// Clamp to 0..1; NaN → 0 (never panics, never leaks).
fn clamp01(t: f32) -> f32 {
    if t.is_nan() {
        0.0
    } else {
        t.clamp(0.0, 1.0)
    }
}
/// MIDI note → Hz (equal temperament, A4 = 440).
fn midi_freq(midi: u8) -> f32 {
    440.0 * 2.0f32.powf((f32::from(midi) - 69.0) / 12.0)
}

/// One baked note event (midly-as-data pattern): 16th-step grid + layer tag.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Note {
    /// 16th-step index from song start.
    pub step: u16,
    /// Layer 0 pad, 1 bass, 2 drums, 3 lead.
    pub layer: u8,
    /// MIDI pitch 21..108 (36 kick / 40 snare / 42 hat on layer 2).
    pub midi: u8,
    /// Velocity 1..127.
    pub vel: u8,
    /// Length in 16th steps.
    pub len: u8,
}
/// Baked offline song data: tempo map + note events. Deterministic from seed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Song {
    /// Bake seed (same seed → same SHA).
    pub seed: u64,
    /// Tempo 96..128 BPM, derived from seed.
    pub bpm: u16,
    /// Bass root MIDI 40..51, derived from seed.
    pub root: u8,
    /// Bar count 1..=MAX_BARS.
    pub bars: u8,
    /// Canonical order: (step, layer, midi).
    pub notes: Vec<Note>,
}
/// Bake offline song data from `seed` (fixed rhythms + seeded lead arp).
pub fn bake(seed: u64, bars: u8) -> Song {
    let bars = bars.clamp(1, MAX_BARS);
    let bpm = u16::try_from(96 + (seed % 33)).unwrap_or(96);
    let root = (40 + ((seed >> 3) % 12)) as u8;
    let base0 = i32::from(root);
    let mut rng = seed | 1;
    let mut notes: Vec<Note> = Vec::new();
    for bar in 0..bars {
        let s0 = u16::from(bar) * STEPS_PER_BAR as u16;
        let base = base0 + PROG[usize::from(bar) % PROG.len()];
        for &iv in &[0, 3, 7] {
            let m = base + 12 + iv;
            if (21..=108).contains(&m) {
                notes.push(Note {
                    step: s0,
                    layer: 0,
                    midi: m as u8,
                    vel: 78,
                    len: 16,
                });
            }
        }
        for &t in &[0u16, 4, 8, 12] {
            let m = base + if t == 12 { 12 } else { 0 };
            if (21..=108).contains(&m) {
                notes.push(Note {
                    step: s0 + t,
                    layer: 1,
                    midi: m as u8,
                    vel: 100,
                    len: 3,
                });
            }
        }
        for &t in &[0u16, 4, 8, 12] {
            notes.push(Note {
                step: s0 + t,
                layer: 2,
                midi: 36,
                vel: 110,
                len: 1,
            });
        }
        for &t in &[4u16, 12] {
            notes.push(Note {
                step: s0 + t,
                layer: 2,
                midi: 40,
                vel: 100,
                len: 1,
            });
        }
        for t in 0..16u16 {
            if t.is_multiple_of(2) {
                notes.push(Note {
                    step: s0 + t,
                    layer: 2,
                    midi: 42,
                    vel: if t.is_multiple_of(4) { 78 } else { 58 },
                    len: 1,
                });
            }
        }
        for &t in &[0u16, 2, 4, 6, 8, 10, 12, 14] {
            let r = xorshift_step(&mut rng);
            if r.is_multiple_of(3) {
                continue;
            }
            let m = base
                + 24
                + PENTA[(r >> 11) as usize % PENTA.len()]
                + if (r >> 17) & 1 == 1 { 12 } else { 0 };
            if !(21..=108).contains(&m) {
                continue;
            }
            notes.push(Note {
                step: s0 + t,
                layer: 3,
                midi: m as u8,
                vel: (88 + (r >> 23) % 40) as u8,
                len: 2,
            });
        }
    }
    notes.sort_by_key(|a| (a.step, a.layer, a.midi));
    Song {
        seed,
        bpm,
        root,
        bars,
        notes,
    }
}
/// Versioned bake bytes: MAGIC(4)+ver(1)+seed(8)+bpm(2)+root(1)+bars(1)+count(2)+6/note.
pub fn bake_bytes(song: &Song) -> Vec<u8> {
    let mut out = Vec::with_capacity(19 + song.notes.len() * 6);
    out.extend_from_slice(&MAGIC);
    out.push(VERSION);
    out.extend_from_slice(&song.seed.to_le_bytes());
    out.extend_from_slice(&song.bpm.to_le_bytes());
    out.push(song.root);
    out.push(song.bars);
    out.extend_from_slice(&(song.notes.len() as u16).to_le_bytes());
    for n in &song.notes {
        out.extend_from_slice(&n.step.to_le_bytes());
        out.push(n.layer);
        out.push(n.midi);
        out.push(n.vel);
        out.push(n.len);
    }
    out
}
/// Song SHA: FNV-1a64 over bake bytes. Accept gate: identical ×2 runs.
pub fn song_hash(song: &Song) -> u64 {
    hash_bytes(&bake_bytes(song))
}

/// One beat-bus tick: bar/beat grid + integer sample offset (kira clock pattern).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Beat {
    /// Bar index from 0.
    pub bar: u16,
    /// Beat in bar 0..BEATS_PER_BAR.
    pub beat: u8,
    /// Sample offset at beat time (60*sr/bpm integer math, no drift).
    pub sample: u64,
}
/// Full beat grid for a song at `sample_rate` Hz: bars*BEATS_PER_BAR beats.
pub fn beat_list(song: &Song, sample_rate: u32) -> Vec<Beat> {
    let per = (60 * u64::from(sample_rate.max(1)) / u64::from(song.bpm)).max(1);
    let n = u64::from(song.bars) * u64::from(BEATS_PER_BAR);
    let mut v = Vec::with_capacity(n as usize);
    for i in 0..n {
        v.push(Beat {
            bar: (i / u64::from(BEATS_PER_BAR)) as u16,
            beat: (i % u64::from(BEATS_PER_BAR)) as u8,
            sample: i * per,
        });
    }
    v
}
/// Beat hash: FNV-1a64 over (bar, beat, sample) grid. Accept gate with song SHA.
pub fn beat_hash(song: &Song, sample_rate: u32) -> u64 {
    let mut hasher = Fnv64::new();
    for b in beat_list(song, sample_rate) {
        hasher.write_u16(b.bar);
        hasher.write_u8(b.beat);
        hasher.write_u64(b.sample);
    }
    hasher.finish()
}

/// Samples per beat on the integer grid: `60 * sample_rate / bpm` (min 1).
/// Units: samples. Determinism: same inputs give the same count on every
/// platform (integer math only, no float time). Never panics.
pub fn samples_per_beat(sample_rate: u32, bpm: u16) -> u64 {
    let sr = u64::from(sample_rate.max(1));
    let tempo = u64::from(bpm.max(1));
    (60u64.saturating_mul(sr) / tempo).max(1)
}

/// Sample offset of global beat `beat` from song start: `beat * beat length`.
/// Integer grid with no drift (every beat multiplies the same beat length).
/// Units: samples. Determinism: same inputs give the same offset. Never panics.
pub fn sample_for_beat(beat: u32, sample_rate: u32, bpm: u16) -> u64 {
    u64::from(beat).saturating_mul(samples_per_beat(sample_rate, bpm))
}

/// Samples per bar: one crossfade span (`BEATS_PER_BAR` beat lengths).
/// Units: samples. Determinism: same inputs give the same span. Never panics.
pub fn bar_samples(sample_rate: u32, bpm: u16) -> u64 {
    u64::from(BEATS_PER_BAR)
        .saturating_mul(samples_per_beat(sample_rate, bpm))
        .max(1)
}

/// Active-layer bitmask for `intensity` 0..1 (bit i = layer i audible).
pub fn active_mask(intensity: f32) -> u8 {
    let v = clamp01(intensity);
    let mut m = 0u8;
    for (i, &th) in LAYER_THRESH.iter().enumerate() {
        if v >= th {
            m |= 1 << i;
        }
    }
    m
}
/// Coarse tension level for the music bed, one per layer gate: level `n`
/// plays layers `0..=n` (pad, then bass, drums, lead).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Tension {
    /// Nobody fighting: pad only.
    Calm,
    /// Fighters closing: pad and bass.
    Build,
    /// Exchanges landing: drums join.
    Fight,
    /// Heavy heat: the full arrangement.
    Climax,
}

/// How far intensity must fall below a level's gate before the level drops,
/// so a value hovering on a gate does not flap between levels.
pub const TENSION_HYSTERESIS: f32 = 0.05;

impl Tension {
    /// Number of levels (one per layer).
    pub const COUNT: usize = N_LAYERS;
    /// Every level, lowest first.
    pub const ALL: [Tension; Tension::COUNT] = [Self::Calm, Self::Build, Self::Fight, Self::Climax];

    /// Dense index `0..COUNT`, also the number of layers above the pad.
    pub fn index(self) -> usize {
        match self {
            Self::Calm => 0,
            Self::Build => 1,
            Self::Fight => 2,
            Self::Climax => 3,
        }
    }

    /// Integer level `0..=3` (what the game sends and the ring carries).
    pub fn level(self) -> u8 {
        u8::try_from(self.index()).unwrap_or(0)
    }

    /// The level for integer `level`, `None` past `3`.
    pub fn from_level(level: u8) -> Option<Self> {
        Self::ALL.get(usize::from(level)).copied()
    }

    /// Intensity that plays exactly this level's layers: its layer gate.
    pub fn intensity(self) -> f32 {
        LAYER_THRESH[self.index()]
    }

    /// Highest level whose gate `intensity` has reached (NaN counts as 0).
    pub fn from_intensity(intensity: f32) -> Self {
        let v = clamp01(intensity);
        Self::ALL
            .into_iter()
            .rev()
            .find(|t| v >= t.intensity())
            .unwrap_or(Self::Calm)
    }

    /// Follow a moving `intensity` from `self`: rise as soon as a gate is
    /// reached, fall only once intensity is [`TENSION_HYSTERESIS`] under the
    /// current level's gate.
    #[must_use]
    pub fn follow(self, intensity: f32) -> Self {
        let target = Self::from_intensity(intensity);
        if target > self
            || (target < self && clamp01(intensity) < self.intensity() - TENSION_HYSTERESIS)
        {
            target
        } else {
            self
        }
    }
}

/// Ticks (2 s at 60 Hz) intensity must stay under a level before
/// [`TensionTracker`] lets the level drop, so the gaps between a fight's
/// exchanges do not pull the music down and up again.
pub const TENSION_HOLD_TICKS: u32 = 120;

/// Turns a per-tick intensity into level changes worth sending to the music
/// bed: rises at once, falls only after intensity has stayed under the level
/// for [`TENSION_HOLD_TICKS`] ticks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TensionTracker {
    level: Tension,
    below: u32,
}

impl Default for TensionTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl TensionTracker {
    /// Start at [`Tension::Calm`].
    pub fn new() -> Self {
        Self {
            level: Tension::Calm,
            below: 0,
        }
    }

    /// The level last reported.
    pub fn level(&self) -> Tension {
        self.level
    }

    /// Feed one tick's intensity (0..1). Returns the new level on the tick
    /// it changes, `None` otherwise.
    pub fn update(&mut self, intensity: f32) -> Option<Tension> {
        let target = self.level.follow(intensity);
        if target > self.level {
            self.level = target;
            self.below = 0;
            return Some(target);
        }
        if target == self.level {
            self.below = 0;
            return None;
        }
        self.below += 1;
        if self.below < TENSION_HOLD_TICKS {
            return None;
        }
        self.level = target;
        self.below = 0;
        Some(target)
    }
}

/// Adaptive player: intensity commits only at bar lines (oaml pattern).
#[derive(Clone, Debug, PartialEq)]
pub struct Player {
    /// Committed intensity (audible now).
    pub cur: f32,
    /// Requested intensity (audible after next `advance_bar`).
    pub pending: f32,
    /// Completed bars (transition clock).
    pub bar: u32,
}
impl Player {
    /// Construct at intensity `i` (clamped; NaN → 0).
    pub fn new(i: f32) -> Self {
        let v = clamp01(i);
        Self {
            cur: v,
            pending: v,
            bar: 0,
        }
    }
    /// Request new intensity; takes effect only at the next bar line.
    pub fn request(&mut self, i: f32) {
        self.pending = clamp01(i);
    }
    /// Commit pending at bar line; returns true if the mix changed.
    pub fn advance_bar(&mut self) -> bool {
        self.bar += 1;
        if self.cur != self.pending {
            self.cur = self.pending;
            true
        } else {
            false
        }
    }
    /// Per-layer 1-bar crossfade gains at progress `t` 0..1 (kira tween pattern).
    /// Rides the shared integer [`Tween`](crate::Tween): `t` quantizes to
    /// [`Tween::STEPS`] frames, each layer lerps its Q24 endpoints through
    /// the same ramp the duck uses. Bar-line commit behavior unchanged.
    pub fn gains(&self, t: f32) -> [f32; N_LAYERS] {
        let f = clamp01(t);
        let pos = (f * crate::f32_of(crate::Tween::STEPS)) as u32;
        let pos = pos.min(crate::Tween::STEPS);
        let mut g = [0.0; N_LAYERS];
        for (g_slot, &th) in g.iter_mut().zip(LAYER_THRESH.iter()) {
            let from = if self.cur >= th { crate::Tween::ONE } else { 0 };
            let to = if self.pending >= th {
                crate::Tween::ONE
            } else {
                0
            };
            *g_slot = crate::Tween::staged(from, to, crate::Tween::STEPS, pos).gain();
        }
        g
    }
}

/// Samples per sequencer step at `sample_rate` (rate 0 counts as 1 Hz).
fn samples_per_step(song: &Song, sample_rate: u32) -> u64 {
    let sr = u64::from(sample_rate.max(1));
    (60 * sr / (u64::from(song.bpm) * u64::from(STEPS_PER_BEAT))).max(1)
}

/// Mono sample count of the bake: the whole song at `sample_rate`, capped
/// at `max_samples`. Sizes the buffer for [`render_pcm_into`].
pub fn pcm_len(song: &Song, sample_rate: u32, max_samples: usize) -> usize {
    let sps = samples_per_step(song, sample_rate);
    let song_samples = u64::from(song.bars) * u64::from(STEPS_PER_BAR) * sps;
    usize::try_from(song_samples).map_or(max_samples, |n| n.min(max_samples))
}

/// Offline bake render to mono i16 PCM (rustysynth idea, integer osc, no SoundFont
/// dep): triangle melodic layers + decay kick + hashed-noise hats, master 0.5.
/// Allocates: the owned-buffer form of [`render_pcm_into`].
pub fn render_pcm(song: &Song, intensity: f32, sample_rate: u32, max_samples: usize) -> Vec<i16> {
    let mut out = crate::zeroed(pcm_len(song, sample_rate, max_samples));
    render_pcm_into(song, intensity, sample_rate, &mut out);
    out
}

/// Most notes one 16th step can sound at once in [`render_pcm_into`]; bakes
/// stay far under it, and a hand-built song's extras past it are skipped.
const MAX_LIVE_NOTES: usize = 64;

/// Render the bake into `out` (`out.len()` mono samples from the song
/// start; size it with [`pcm_len`]). Allocates nothing. The notes sounding in
/// a step and their pitches are found once per step, not once per sample.
pub fn render_pcm_into(song: &Song, intensity: f32, sample_rate: u32, out: &mut [i16]) {
    let sr = sample_rate.max(1);
    let mask = active_mask(intensity);
    let sps = samples_per_step(song, sample_rate);
    let mut live: [Option<(&Note, f32)>; MAX_LIVE_NOTES] = [None; MAX_LIVE_NOTES];
    let mut live_len = 0;
    let mut live_step = None;
    for (s, slot) in (0u64..).zip(out.iter_mut()) {
        let st = (s / sps) as u16;
        if live_step != Some(st) {
            live_step = Some(st);
            live_len = 0;
            let sounding = song.notes.iter().filter(|n| mask & (1u8 << n.layer) != 0);
            for n in sounding.filter(|n| st >= n.step && st < n.step + u16::from(n.len)) {
                if live_len < MAX_LIVE_NOTES {
                    live[live_len] = Some((n, midi_freq(n.midi)));
                    live_len += 1;
                }
            }
        }
        let ph_step = (s % sps) as f32 / sps as f32;
        let mut mix = 0.0f32;
        for &(n, freq) in live[..live_len].iter().flatten() {
            let age = f32::from(st - n.step) + ph_step;
            let env = (1.0 - age / f32::from(n.len.max(1))).max(0.0);
            let amp = f32::from(n.vel) / 127.0 * env;
            if n.layer == 2 {
                if n.midi == 42 {
                    let nz = (s.wrapping_mul(GOLDEN_GAMMA) >> 33) as f32 / 2_147_483_600.0 - 1.0;
                    mix += nz * amp * 0.5;
                } else {
                    let p = (s as f32 * freq / sr as f32) % 1.0;
                    mix += (1.0 - ph_step) * (1.0 - 4.0 * (p - 0.5).abs()) * amp;
                }
            } else {
                let p = (s as f32 * freq / sr as f32 + f32::from(n.step) * 0.13) % 1.0;
                mix += (1.0 - 4.0 * (p - 0.5).abs()) * amp * 0.8;
            }
        }
        *slot = ((mix * 0.5).clamp(-1.0, 1.0) * 32767.0).round() as i16;
    }
}

/// PCM hash: FNV-1a64 over i16 LE bytes (replay/SHA gate helper).
pub fn pcm_hash(pcm: &[i16]) -> u64 {
    let mut hasher = Fnv64::new();
    for sample in pcm {
        hasher.write(&sample.to_le_bytes());
    }
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn song() -> Song {
        bake(2040, 2)
    }
    #[test]
    fn pcm_into_rewrites_stale_samples_and_shorter_buffers_are_prefixes() {
        let s = song();
        let owned = render_pcm(&s, 1.0, 8000, 24000);
        assert_eq!(owned.len(), pcm_len(&s, 8000, 24000));
        let mut dirty = vec![i16::MAX; owned.len()];
        render_pcm_into(&s, 1.0, 8000, &mut dirty);
        assert_eq!(dirty, owned, "every slot is rewritten");
        let mut head = vec![0i16; 100];
        render_pcm_into(&s, 1.0, 8000, &mut head);
        assert_eq!(head[..], owned[..100], "a shorter buffer is a prefix");
        assert_eq!(pcm_len(&s, 8000, 0), 0);
    }
    #[test]
    fn song_sha_x2() {
        let (a, b) = (bake(2040, 4), bake(2040, 4));
        assert_eq!(bake_bytes(&a), bake_bytes(&b));
        assert_eq!(song_hash(&a), song_hash(&b));
        assert_eq!(&bake_bytes(&a)[..4], &MAGIC);
        assert_eq!(bake_bytes(&a)[4], VERSION);
    }
    #[test]
    fn seed_moves_song() {
        assert_ne!(song_hash(&bake(2040, 4)), song_hash(&bake(2041, 4)));
        assert_ne!(bake_bytes(&bake(7, 2)), bake_bytes(&bake(8, 2)));
    }
    #[test]
    fn layers_gated() {
        assert_eq!(active_mask(0.0), 0b0001);
        assert_eq!(active_mask(0.5), 0b0011);
        assert_eq!(active_mask(0.6), 0b0111);
        assert_eq!(active_mask(1.0), 0b1111);
        assert_eq!(active_mask(f32::NAN), 0b0001);
        assert_eq!(active_mask(-3.0), 0b0001);
        assert_eq!(active_mask(99.0), 0b1111);
    }
    #[test]
    fn bar_quantized() {
        let mut p = Player::new(0.0);
        p.request(1.0);
        assert_eq!((p.cur, p.pending), (0.0, 1.0));
        assert_eq!(p.gains(0.0), [1.0, 0.0, 0.0, 0.0]);
        assert_eq!(p.gains(1.0), [1.0, 1.0, 1.0, 1.0]);
        assert!(p.advance_bar());
        assert_eq!((p.cur, p.bar), (1.0, 1));
        assert!(!p.advance_bar());
        assert_eq!(p.gains(0.37), [1.0, 1.0, 1.0, 1.0]);
    }
    #[test]
    fn beat_grid_hash_mono() {
        let s = song();
        let (a, b) = (beat_list(&s, 8000), beat_list(&s, 8000));
        assert_eq!(a, b);
        assert_eq!(beat_hash(&s, 8000), beat_hash(&s, 8000));
        assert_eq!(a.len(), s.bars as usize * BEATS_PER_BAR as usize);
        assert!(a.windows(2).all(|w| w[0].sample < w[1].sample));
    }
    #[test]
    fn render_deterministic_bounded() {
        let s = song();
        let (lo, hi) = (
            render_pcm(&s, 0.0, 8000, 24000),
            render_pcm(&s, 1.0, 8000, 24000),
        );
        assert_eq!(lo, render_pcm(&s, 0.0, 8000, 24000));
        assert_eq!(pcm_hash(&lo), pcm_hash(&lo));
        assert!(!lo.iter().all(|&v| v == 0), "audible");
        assert_ne!(lo, hi, "layers change mix");
        assert!(hi.iter().map(|v| v.unsigned_abs()).max().unwrap() <= 32767);
        assert!(render_pcm(&s, 0.5, 8000, 0).is_empty());
    }
    #[test]
    fn integer_beat_grid_hit_sync() {
        // Four bars at 120 BPM: seed 24 pins the bake tempo to 96 + 24.
        let song = bake(24, 4);
        assert_eq!((song.bpm, song.bars), (120, 4));
        let sr = 48_000u32;
        assert_eq!(samples_per_beat(sr, song.bpm), 24_000);
        assert_eq!(bar_samples(sr, song.bpm), 96_000);
        let grid = beat_list(&song, sr);
        assert_eq!(grid.len(), 16);
        // Integer grid: beat offsets match the beat list within one sample.
        for (beat, entry) in [(2u32, &grid[2]), (4u32, &grid[4])] {
            let off = sample_for_beat(beat, sr, song.bpm).abs_diff(entry.sample);
            assert!(off <= 1, "hit-sync offset {off} samples is over budget");
        }
        assert_eq!(sample_for_beat(2, sr, song.bpm), grid[2].sample);
        // One-bar crossfade still rides the layer gains end to end.
        let mut player = Player::new(0.0);
        player.request(1.0);
        assert_eq!(player.gains(0.0), [1.0, 0.0, 0.0, 0.0]);
        assert_eq!(player.gains(1.0), [1.0, 1.0, 1.0, 1.0]);
        assert_eq!(sample_for_beat(4, sr, song.bpm), grid[4].sample);
        assert_eq!(beat_hash(&song, sr), beat_hash(&song, sr));
    }

    #[test]
    fn tension_levels_map_to_layer_gates() {
        for (i, level) in Tension::ALL.into_iter().enumerate() {
            assert_eq!(level.index(), i);
            assert_eq!(Tension::from_level(level.level()), Some(level));
            assert_eq!(
                Tension::from_intensity(level.intensity()),
                level,
                "{level:?}"
            );
            assert_eq!(
                usize::from(active_mask(level.intensity())).count_ones(),
                u32::try_from(i + 1).unwrap_or(0),
                "level {i} plays layers 0..={i}"
            );
        }
        assert_eq!(Tension::from_level(4), None);
        assert_eq!(Tension::from_intensity(f32::NAN), Tension::Calm);
        assert_eq!(Tension::from_intensity(-3.0), Tension::Calm);
        assert_eq!(Tension::from_intensity(9.0), Tension::Climax);
        assert_eq!(Tension::from_intensity(0.299), Tension::Calm);
        assert_eq!(Tension::from_intensity(0.3), Tension::Build);
    }

    #[test]
    fn tension_follows_intensity_with_hysteresis() {
        // Rises as soon as a gate is reached, falls only 0.05 below it.
        assert_eq!(Tension::Calm.follow(0.30), Tension::Build);
        assert_eq!(
            Tension::Build.follow(0.29),
            Tension::Build,
            "hovering under the gate holds"
        );
        assert_eq!(Tension::Build.follow(0.251), Tension::Build);
        assert_eq!(Tension::Build.follow(0.249), Tension::Calm);
        assert_eq!(Tension::Climax.follow(0.76), Tension::Climax);
        assert_eq!(
            Tension::Climax.follow(0.0),
            Tension::Calm,
            "a real drop skips levels"
        );
        assert_eq!(Tension::Fight.follow(0.95), Tension::Climax);
        // A value wobbling on a gate changes level once, not every tick.
        let mut level = Tension::Calm;
        let mut switches = 0;
        for wobble in [0.29, 0.31, 0.29, 0.31, 0.28, 0.32] {
            let next = level.follow(wobble);
            switches += usize::from(next != level);
            level = next;
        }
        assert_eq!((level, switches), (Tension::Build, 1));
    }

    #[test]
    fn tracker_rises_at_once_and_falls_only_after_the_hold() {
        let mut t = TensionTracker::default();
        assert_eq!(t.level(), Tension::Calm);
        assert_eq!(t.update(0.1), None);
        assert_eq!(
            t.update(0.6),
            Some(Tension::Fight),
            "rises on the tick it is reached"
        );
        assert_eq!(t.update(0.9), Some(Tension::Climax));
        // Quiet for just under the hold: still climax.
        for _ in 0..TENSION_HOLD_TICKS - 1 {
            assert_eq!(t.update(0.0), None);
        }
        assert_eq!(t.level(), Tension::Climax);
        // One exchange resets the hold clock.
        assert_eq!(t.update(0.85), None);
        for _ in 0..TENSION_HOLD_TICKS - 1 {
            assert_eq!(t.update(0.0), None);
        }
        assert_eq!(
            t.update(0.0),
            Some(Tension::Calm),
            "falls after the full hold"
        );
        assert_eq!(t.update(0.0), None);
        assert_eq!(t.level(), Tension::Calm);
    }

    /// The straightforward renderer: every sample scans every note and
    /// recomputes its pitch.
    fn reference_render(song: &Song, intensity: f32, sample_rate: u32, len: usize) -> Vec<i16> {
        let sr = sample_rate.max(1);
        let mask = active_mask(intensity);
        let sps = samples_per_step(song, sample_rate);
        (0u64..)
            .take(len)
            .map(|s| {
                let st = (s / sps) as u16;
                let ph_step = (s % sps) as f32 / sps as f32;
                let mut mix = 0.0f32;
                for n in song.notes.iter().filter(|n| mask & (1u8 << n.layer) != 0) {
                    if st < n.step || st >= n.step + u16::from(n.len) {
                        continue;
                    }
                    let age = f32::from(st - n.step) + ph_step;
                    let env = (1.0 - age / f32::from(n.len.max(1))).max(0.0);
                    let amp = f32::from(n.vel) / 127.0 * env;
                    if n.layer == 2 {
                        if n.midi == 42 {
                            let nz =
                                (s.wrapping_mul(GOLDEN_GAMMA) >> 33) as f32 / 2_147_483_600.0 - 1.0;
                            mix += nz * amp * 0.5;
                        } else {
                            let p = (s as f32 * midi_freq(n.midi) / sr as f32) % 1.0;
                            mix += (1.0 - ph_step) * (1.0 - 4.0 * (p - 0.5).abs()) * amp;
                        }
                    } else {
                        let p = (s as f32 * midi_freq(n.midi) / sr as f32
                            + f32::from(n.step) * 0.13)
                            % 1.0;
                        mix += (1.0 - 4.0 * (p - 0.5).abs()) * amp * 0.8;
                    }
                }
                ((mix * 0.5).clamp(-1.0, 1.0) * 32767.0).round() as i16
            })
            .collect()
    }

    #[test]
    fn per_step_note_lookup_renders_the_same_bytes_as_the_per_sample_scan() {
        for (seed, bars, rate) in [(2040u64, 2u8, 8_000u32), (7, 3, 11_025), (24, 1, 8_000)] {
            let s = bake(seed, bars);
            let len = pcm_len(&s, rate, 400_000);
            for intensity in [0.0, 0.3, 0.55, 1.0] {
                let fast = render_pcm(&s, intensity, rate, len);
                let slow = reference_render(&s, intensity, rate, len);
                assert_eq!(fast, slow, "seed {seed} intensity {intensity}");
            }
        }
    }
}
