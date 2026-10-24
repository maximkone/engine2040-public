//! audio: offline synth plus headless voice mixer. Frames into stats (peak/RMS).
//!
//! Headless (always available): [`Mixer`] voice mixing over [`Pcm`] samples,
//! [`Bank`] procedural placeholder hit/footstep/crowd-bed samples, [`Pcm`],
//! [`VoiceId`] and [`play`](Mixer::play), plus [`SpatialMixer`] positional
//! mix and [`Crowd`] fight reactions for the game audio layer. The live
//! path is cues, not clips: a [`CueBank`] of named [`Cue`]s, a lock-free
//! [`FeedTx`] the game thread writes tick-stamped commands into, and an
//! [`AudioEngine`] that owns the mixer and drains the feed on the audio
//! callback (no lock, no allocation). Default-
//! device loss/change heals through [`heal`] (supervisor reconnect with
//! backoff, silent mixer keeps rendering). Vorbis/WAV decode
//! (`decode_vorbis`, `decode_wav`) and cpal output ([`decode`],
//! [`device`]) live behind the `device` feature so the headless default
//! stays std-only.

pub mod bank;
pub mod coherence;
/// Fight events to named cue plays: which cue, how loud, where.
pub mod combat_cues;
pub mod crowd;
pub mod cue;
/// Cue anchors plus offsets resolved to a tick, and a forward-only cue clock.
pub mod cue_anchor;
/// Authored cue rows: lanes, gains, duck, loops, the pick order and the gap
/// report. Refuses overlapping loops on one lane.
pub mod cue_sheet;
/// Click-free gain ramps that reverse from the gain reached. Integer only.
mod declick;
#[cfg(feature = "device")]
pub mod decode;
#[cfg(feature = "device")]
pub mod device;
pub mod director;
pub mod dsp;
/// Early reflections sized from the room: four tap delays from the wall
/// distance, each gained by the wall's absorption, summed into the wet bus
/// with no audio-thread allocation. Std-only.
pub mod early_reflections;
pub mod engine;
pub mod feed;
/// Gunshot, crack and shout acoustics: distance filter, delay, zone tail,
/// wall and door occlusion. Std-only, presentation data.
pub mod gunfire;
pub mod heal;
/// Headphone 3D ears over mono sources: HRIR-sphere FIR pair plus
/// ITD/ILD per octant, speakers passthrough unchanged. Std-only.
pub mod hrtf;
/// Combat mix over shared clips: 1/r distance gain, priority stealing.
pub mod mix;
pub mod music;
/// Wall-muffle gains per voice per tick: fixed-point, heap-free.
/// Std-only.
pub mod occlude;
pub mod playlist;
/// Named mixer settings as data: bus gains in permille, applied to a [`dsp::BusMixer`].
pub mod preset;
/// Parametric reverb tails with tunnel preset: delay-line pure fns,
/// zero audio-thread allocs. Std-only.
pub mod reverb;
pub mod spatial;
/// Cut, break and collapse events to material and tissue cue plays.
pub mod strike_sound;
pub mod trace;
pub mod voice;

/// Test-binary allocator: lets `engine_core::alloc_probe::measure` count heap
/// calls on the measuring thread only, so parallel tests never interfere.
#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: engine_core::alloc_probe::CountingAlloc =
    engine_core::alloc_probe::CountingAlloc;

use std::sync::Arc;

use declick::{dry_wet, Declicker};
use dsp::Bus;

pub use bank::{Bank, BankSound, Synth};
pub use crowd::{Crowd, FightCue, Reaction};
pub use cue::{
    names as cue_names, Clip, Cue, CueBank, CueError, CueId, Group, ImpactRouter, LoadReport,
    RoutedImpact, Spatial, VariantCycle, VariantPicker, ARENA_CUES, IMPACT_CUES, MAX_VARIANTS,
};
#[cfg(feature = "device")]
pub use decode::{
    decode_vorbis, decode_wav, load_cue_bank, DecodeError, MAX_DECODE_BYTES, MAX_PCM_FRAMES,
};
#[cfg(feature = "device")]
pub use device::{AudioSink, DeviceError, DeviceOutput};
pub use engine::{AudioEngine, FrameSource, DRAIN_LIMIT};
pub use feed::{AudioCommand, Command, FeedError, FeedRx, FeedStats, FeedTx, FEED_CAPACITY};
pub use heal::{
    AudioHealError, DeviceHeal, DeviceState, HealReport, HoldLastMixer, DEVICE_FAULT_CODE,
};
pub use music::{Tension, TensionTracker};
pub use playlist::{MusicBank, MusicBed, MusicError, MusicState};
pub use spatial::{attenuation, falloff_gain, isqrt, pan_for, MixCmd, Pos2, SpatialMixer, NEAR_CM};

/// Bus gains (dB): Master 0, Music -12 ducked, SFX -3, Dialogue -6 priority.
pub const GAIN_MASTER_DB: f32 = 0.0;
/// Music bus gain in dB: ducked under the effects and dialogue buses.
pub const GAIN_MUSIC_DB: f32 = -12.0;
/// Sound-effects bus gain in dB.
pub const GAIN_SFX_DB: f32 = -3.0;
/// Dialogue bus gain in dB: the priority bus sits above music, below master.
pub const GAIN_DIALOGUE_DB: f32 = -6.0;

/// Max simultaneous voices in a [`Mixer`]; beyond this a plain voice evicts
/// the oldest and a cue voice steals by priority, so playback stays bounded
/// and never panics.
pub const MAX_VOICES: usize = 64;
/// Max per-voice gain (linear); gains sanitize into `0.0..=MAX_GAIN`.
pub const MAX_GAIN: f32 = 2.0;

/// Zero-filled owned output for the offline owned-buffer conveniences
/// (`render`, `mix`, `render_song`, `render_pcm`). The one place those
/// allocate; every render path takes a caller buffer and never calls this.
pub(crate) fn zeroed<T: Copy + Default>(len: usize) -> Vec<T> {
    vec![T::default(); len]
}

/// RMS gate: open when rms > threshold (default -60dBFS ≈ 0.001). Silence <1e-4.
pub fn rms_gate_open(rms: f32, threshold: f32) -> bool {
    rms > threshold
}

/// Hard-clip one mixed sample to `[-1, 1]`; NaN becomes silence, never panics.
pub fn clip_sample(s: f32) -> f32 {
    if s.is_nan() {
        0.0
    } else {
        s.clamp(-1.0, 1.0)
    }
}

/// Sanitize a per-voice gain: NaN/negative become `0.0`, large values clamp to
/// [`MAX_GAIN`]. Never panics; keeps the mix deterministic on bad input.
pub fn sanitize_gain(gain: f32) -> f32 {
    if gain.is_nan() {
        0.0
    } else {
        gain.clamp(0.0, MAX_GAIN)
    }
}

/// Decoded or procedural audio: interleaved f32 samples nominally in
/// `[-1, 1]`, `channels` 1 (mono) or 2 (stereo, LRLR...). `sample_rate` is
/// informational: the [`Mixer`] plays samples 1:1 with no resampling (see
/// [`spatial`] for the positional layer).
#[derive(Clone, Debug, PartialEq)]
pub struct Pcm {
    /// Sample rate in Hz (informational; mixer plays 1 sample per frame).
    pub sample_rate: u32,
    /// Channel count, always `>= 1` (`new` coerces 0 to mono).
    pub channels: u16,
    /// Interleaved samples, truncated to whole frames, NaN-free, clipped.
    pub samples: Vec<f32>,
}

impl Pcm {
    /// Build from interleaved samples; coerces `channels == 0` to mono,
    /// truncates trailing partial frames, maps NaN to silence and clips to
    /// `[-1, 1]`. Never panics.
    pub fn new(sample_rate: u32, channels: u16, samples: Vec<f32>) -> Self {
        let channels = channels.max(1);
        let whole = samples.len() - samples.len() % usize::from(channels);
        let samples = samples[..whole]
            .iter()
            .map(|&s| clip_sample(s))
            .collect::<Vec<_>>();
        Self {
            sample_rate,
            channels,
            samples,
        }
    }

    /// Whole-frame count (`samples.len() / channels`).
    pub fn frames(&self) -> usize {
        self.samples.len() / usize::from(self.channels)
    }

    /// True when there are no frames to play.
    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    /// Mono value of `frame` (mean across channels); out of range gives `0.0`.
    pub fn mono_at(&self, frame: usize) -> f32 {
        mono_in(&self.samples, self.channels, frame)
    }

    /// Stereo `(left, right)` of `frame`: mono duplicates, stereo direct,
    /// 3+ channels fold to the first two. Out of range gives `(0.0, 0.0)`.
    pub fn stereo_at(&self, frame: usize) -> (f32, f32) {
        stereo_in(&self.samples, self.channels, frame)
    }
}

/// Typed voice handle returned by [`Mixer::play`]; compare by value, never by
/// timing. `0` is never issued (reserved as invalid).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct VoiceId(pub u64);

impl VoiceId {
    /// Numeric handle value (for logs/tests, not ordering).
    pub fn to_u64(self) -> u64 {
        self.0
    }
}

/// Exact `u32` to `f32` for values below 2^24 (centimeters, frame counts,
/// sample rates); larger values round to the nearest `f32`.
pub(crate) fn f32_of(n: u32) -> f32 {
    n as f32
}

/// Mono value of `frame` in interleaved `samples` (mean across channels);
/// out of range gives `0.0`.
fn mono_in(samples: &[f32], channels: u16, frame: usize) -> f32 {
    let ch = usize::from(channels.max(1));
    let base = frame.saturating_mul(ch);
    if base.saturating_add(ch) > samples.len() {
        return 0.0;
    }
    let mut acc = 0.0f32;
    for c in 0..ch {
        acc += samples[base + c];
    }
    acc / f32::from(channels.max(1))
}

/// Stereo `(left, right)` of `frame` in interleaved `samples`: mono
/// duplicates, stereo direct, 3+ channels fold to the first two; out of
/// range gives `(0.0, 0.0)`.
fn stereo_in(samples: &[f32], channels: u16, frame: usize) -> (f32, f32) {
    let ch = usize::from(channels.max(1));
    let base = frame.saturating_mul(ch);
    if base.saturating_add(ch) > samples.len() {
        return (0.0, 0.0);
    }
    match ch {
        1 => (samples[base], samples[base]),
        _ => (samples[base], samples[base + 1]),
    }
}

/// How a cue voice routes through the mixer and how it is stolen.
#[derive(Clone, Copy, Debug)]
pub struct VoiceRoute {
    /// Cue the voice belongs to (for [`Mixer::stop_cue`]).
    pub cue: CueId,
    /// Bus the voice sums into.
    pub bus: Bus,
    /// Instance-limit group.
    pub group: Group,
    /// Higher survives stealing.
    pub priority: u8,
    /// Wrap to the start at the end of the clip.
    pub looped: bool,
    /// While this voice sounds the music bus ducks (dialogue, stingers).
    pub ducks_music: bool,
}

/// Where and how a cue voice begins.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct VoiceStart {
    /// Frame of the clip to start from (a loop joining mid-pattern).
    pub offset_frames: usize,
    /// Ramp the voice in over this many frames; 0 starts at full level.
    pub fade_in_frames: u32,
}

/// One live voice: shared sample data, a frame cursor and per-ear gains.
/// `route` is `None` for plain [`Mixer::play`] voices, which sum straight to
/// the master at priority 0.
struct Voice {
    id: VoiceId,
    data: Arc<[f32]>,
    channels: u16,
    cursor: usize,
    /// Whole-frame count of `data`, fixed when the voice starts: cached so the
    /// per-frame mix loop reads a field instead of dividing by `channels`.
    frame_count: usize,
    gain_l: f32,
    gain_r: f32,
    fold_mono: bool,
    route: Option<VoiceRoute>,
    /// Click-free level ramp on top of the gains: at rest at unity for a
    /// plain start, ramped in on a fade-in and out on a steal or a stop.
    gate: Declicker,
    /// Live multiplier set by [`Mixer::set_cue_trim`] (`1.0` untouched): how
    /// the game rides a looped bed up and down without restarting it.
    trim: f32,
    /// One-pole lowpass coefficient set by [`Mixer::set_voice_lowpass`]:
    /// `1.0` is open (bypass), smaller darkens.
    lp_coeff: f32,
    /// Filter memory (left, right): the last output, or the last source frame
    /// while open, so closing the filter starts from the sound playing.
    lp_state: (f32, f32),
}

impl Voice {
    /// Whole-frame count of `data` (cached at voice start; `data` and
    /// `channels` never change after [`Mixer::push_voice`]).
    #[must_use]
    fn frames(&self) -> usize {
        self.frame_count
    }

    /// Fading out after a steal or a stop: already gone as far as limits
    /// are concerned.
    fn retiring(&self) -> bool {
        self.gate.trending_towards_zero()
    }

    /// Out of samples, or faded to silence.
    fn finished(&self) -> bool {
        self.cursor >= self.frames() || (self.retiring() && self.gate.level() == 0)
    }

    /// Ramp multiplier for this frame (exactly 1.0 at rest at unity).
    fn fade_gain(&self) -> f32 {
        q24_gain(self.gate.level())
    }

    fn priority(&self) -> u8 {
        self.route.map_or(0, |route| route.priority)
    }

    fn in_group(&self, group: Group) -> bool {
        self.route.is_some_and(|route| route.group == group)
    }

    /// Run the voice lowpass over one source frame. Open returns the frame
    /// untouched.
    fn lowpass(&mut self, (l, r): (f32, f32)) -> (f32, f32) {
        if self.lp_coeff >= 1.0 {
            self.lp_state = (l, r);
        } else {
            self.lp_state.0 += self.lp_coeff * (l - self.lp_state.0);
            self.lp_state.1 += self.lp_coeff * (r - self.lp_state.1);
        }
        self.lp_state
    }

    /// Current `(left, right)` source frame, mono-folded when asked.
    fn stereo(&self) -> (f32, f32) {
        if self.fold_mono {
            let m = mono_in(&self.data, self.channels, self.cursor);
            (m, m)
        } else {
            stereo_in(&self.data, self.channels, self.cursor)
        }
    }
}

/// One-pole lowpass coefficient `1 - e^(-2 pi fc / fs)` for `cutoff_hz`; `1.0`
/// (open, bypass) at or above [`occlude::OPEN_CUTOFF_HZ`] or the Nyquist rate.
fn lowpass_coeff(cutoff_hz: u32, sample_rate: u32) -> f32 {
    if cutoff_hz >= occlude::OPEN_CUTOFF_HZ || u64::from(cutoff_hz) * 2 >= u64::from(sample_rate) {
        return 1.0;
    }
    let x = std::f32::consts::TAU * f32_of(cutoff_hz) / f32_of(sample_rate);
    1.0 - (-x).exp()
}

/// Music level while dialogue plays, percent of full.
pub const DUCK_FLOOR_PERCENT: u32 = 35;
/// Time for the music duck to reach its floor once dialogue starts.
pub const DUCK_ATTACK_MS: u32 = 40;
/// Time for the music duck to recover once dialogue ends.
pub const DUCK_RELEASE_MS: u32 = 600;
/// Fixed-point one (Q24) for duck and fade levels.
const LEVEL_ONE: u32 = declick::UNITY_Q24;
/// Voices that may fade out after a steal at the same time; past this a
/// steal cuts the victim instead, so voice storage never grows.
pub const FADE_RESERVE: usize = 16;
/// Length of the fade a stolen voice plays out, in milliseconds of samples.
pub const STEAL_FADE_MS: u32 = 8;

/// Frames in `ms` milliseconds at `sample_rate`, at least one.
fn ms_to_frames(sample_rate: u32, ms: u32) -> u32 {
    u32::try_from(u64::from(sample_rate) * u64::from(ms) / 1000)
        .unwrap_or(u32::MAX)
        .max(1)
}

/// Linear gain of a Q24 `level`, exact through Q15.
fn q24_gain(level: u32) -> f32 {
    f32::from(u16::try_from(level >> 9).unwrap_or(u16::MAX)) / 32_768.0
}

/// Integer linear tween over frames (Kira `Tween` pattern, std-only).
/// Consumer: [`music::Player::gains`](crate::music::Player::gains) (1-bar
/// crossfade). Ramps that may reverse mid-way use [`Declicker`] instead.
/// Units: Q24
/// levels for `from`/`to`/`value`, frames for `frames`/`pos`. Determinism:
/// integer math only (a `u64` multiply then divide, no float time, no heap);
/// the same inputs give the same levels on every platform. Never panics
/// (`frames` clamps to at least 1, `pos` clamps to `frames`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tween {
    from: u32,
    to: u32,
    frames: u32,
    pos: u32,
}

impl Tween {
    /// Full Q24 level (matches the duck `LEVEL_ONE`).
    pub const ONE: u32 = 1 << 24;
    /// Steps a `Player::gains` fade quantizes into (power of two, ~0.001).
    pub const STEPS: u32 = 1024;

    /// Ramp `from` to `to` over `frames` (at least 1), parked at the start.
    pub fn new(from: u32, to: u32, frames: u32) -> Self {
        Self {
            from,
            to,
            frames: frames.max(1),
            pos: 0,
        }
    }

    /// Ramp `from` to `to` over `frames` (at least 1), already `pos` frames
    /// in (clamped). Evaluates a fade at progress `t` without stepping.
    pub fn staged(from: u32, to: u32, frames: u32, pos: u32) -> Self {
        let frames = frames.max(1);
        Self {
            from,
            to,
            frames,
            pos: pos.min(frames),
        }
    }

    /// Target level (`to`).
    pub fn target(&self) -> u32 {
        self.to
    }

    /// True once the ramp has reached `to`.
    pub fn is_done(&self) -> bool {
        self.pos >= self.frames
    }

    /// Current Q24 level (exact `from` at 0, exact `to` at the end).
    pub fn value(&self) -> u32 {
        if self.pos == 0 {
            return self.from;
        }
        if self.pos >= self.frames {
            return self.to;
        }
        if self.from == self.to {
            return self.to;
        }
        let span = self.to.abs_diff(self.from);
        let delta = u32::try_from(u64::from(span) * u64::from(self.pos) / u64::from(self.frames))
            .unwrap_or(span);
        if self.to > self.from {
            self.from.saturating_add(delta)
        } else {
            self.from.saturating_sub(delta)
        }
    }

    /// Advance one frame toward `to`, returning the new level.
    pub fn step(&mut self) -> u32 {
        if self.pos < self.frames {
            self.pos += 1;
        }
        self.value()
    }

    /// Linear gain of the current level, exact through Q15.
    pub fn gain(&self) -> f32 {
        q24_gain(self.value())
    }
}

/// Music duck: a [`Declicker`] ramp (0 released, 1 ducked) mixed between the
/// unity path and the floor path by the dry/wet law. A dialogue line that
/// ends mid-attack resumes the release from the gain reached, without a jump.
struct Duck {
    gate: Declicker,
    floor: u32,
    attack_frames: u32,
    release_frames: u32,
}

impl Duck {
    fn new(sample_rate: u32) -> Self {
        let floor = LEVEL_ONE / 100 * DUCK_FLOOR_PERCENT;
        Self {
            gate: Declicker::settled(false),
            floor,
            attack_frames: ms_to_frames(sample_rate, DUCK_ATTACK_MS),
            release_frames: ms_to_frames(sample_rate, DUCK_RELEASE_MS),
        }
    }

    /// Advance one frame toward the floor (`dialogue`) or back to unity.
    fn step(&mut self, dialogue: bool) -> f32 {
        let frames = if dialogue {
            self.attack_frames
        } else {
            self.release_frames
        };
        self.gate.fade_to(dialogue, frames);
        self.gate.advance();
        self.gain()
    }

    /// Linear gain `floor..=1.0`: the unity path is dry, the floor path wet.
    fn gain(&self) -> f32 {
        let (dry, wet) = dry_wet(self.gate.level());
        let floored = u32::try_from(u64::from(wet) * u64::from(self.floor) / u64::from(LEVEL_ONE))
            .unwrap_or(0);
        q24_gain(dry + floored)
    }
}

/// Dense bus index for the per-bus tables.
fn bus_index(bus: Bus) -> usize {
    match bus {
        Bus::Sfx => 0,
        Bus::Music => 1,
        Bus::Dialogue => 2,
    }
}

/// Voice counters since the mixer was built.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MixerStats {
    /// Cue voices started.
    pub started: u64,
    /// Voices stopped to make room for a higher or equal priority play.
    pub stolen: u64,
    /// Cue plays refused: group muted, or nothing weaker to steal.
    pub rejected: u64,
}

/// One frame's sums before bus gains: master-direct voices, the three buses
/// and whether a voice that ducks the music was live.
struct FrameMix {
    raw: (f32, f32),
    bus: [(f32, f32); 3],
    duck: bool,
}

/// Voice mixer: sums live voices in issue order into caller buffers with
/// per-ear gains and a hard clip. Plain [`Mixer::play`] voices go straight
/// to the master; cue voices ([`Mixer::play_clip`]) sum into the SFX, music
/// or dialogue bus, with the music bus ducked under dialogue. The sample
/// rate is informational; voices play 1:1 with no resampling. Voice storage
/// is reserved up front, so rendering and starting cue voices never allocate.
pub struct Mixer {
    /// Sample rate in Hz this mixer renders at.
    pub sample_rate: u32,
    next_voice: u64,
    voices: Vec<Voice>,
    group_limits: [u8; Group::COUNT],
    bus_gain: [f32; 3],
    duck: Duck,
    steal_fade_frames: u32,
    stats: MixerStats,
}

impl Mixer {
    /// Empty mixer at `sample_rate` Hz: no voices, the first [`VoiceId`] is 1.
    pub fn new(sample_rate: u32) -> Self {
        let mut group_limits = [0u8; Group::COUNT];
        for group in Group::ALL {
            group_limits[group.index()] = group.default_limit();
        }
        Self {
            sample_rate,
            next_voice: 1,
            voices: Vec::with_capacity(MAX_VOICES + FADE_RESERVE),
            group_limits,
            bus_gain: [
                Bus::Sfx.gain_linear(),
                Bus::Music.gain_linear(),
                Bus::Dialogue.gain_linear(),
            ],
            duck: Duck::new(sample_rate),
            steal_fade_frames: ms_to_frames(sample_rate, STEAL_FADE_MS),
            stats: MixerStats::default(),
        }
    }
    /// render `n` frames of a sine at `hz`; returns (peak, rms)
    pub fn render_sine(&self, hz: f32, n: usize) -> (f32, f32) {
        let mut peak = 0f32;
        let mut sum = 0f64;
        for i in 0..n {
            let t = i as f32 / self.sample_rate as f32;
            let s = (2.0 * std::f32::consts::PI * hz * t).sin() * 0.5;
            peak = peak.max(s.abs());
            sum += f64::from(s) * f64::from(s);
        }
        (peak, (sum / n as f64).sqrt() as f32)
    }

    /// Start `sample` now at per-voice `gain` (sanitized via
    /// [`sanitize_gain`]) and return its [`VoiceId`]. Finished voices are
    /// reaped first; past [`MAX_VOICES`] the oldest voice is evicted. The
    /// samples are copied once into shared storage, so callers keep
    /// ownership; the cue path ([`Self::play_clip`]) copies nothing. Never
    /// panics.
    pub fn play(&mut self, sample: &Pcm, gain: f32) -> VoiceId {
        self.reap();
        if self.active_voices() >= MAX_VOICES {
            if let Some(oldest) = self.voices.iter().position(|v| !v.retiring()) {
                self.voices.remove(oldest);
            }
        }
        let gain = sanitize_gain(gain);
        self.push_voice(
            Arc::from(sample.samples.as_slice()),
            sample.channels,
            (gain, gain),
            false,
            None,
            VoiceStart::default(),
        )
    }

    /// Start a cue voice sharing `clip`'s samples (no copy) with per-ear
    /// `gains` (sanitized), mono-folded first when `fold_mono`. The group
    /// limit is enforced first, then the pool limit: a full group or pool
    /// stops its weakest voice (lowest priority, oldest among equals) when
    /// that voice's priority is at most `route.priority`; otherwise the play
    /// is refused and `None` comes back. A stolen voice does not cut: it
    /// fades out over [`STEAL_FADE_MS`] while the new voice takes a reserve
    /// slot at once, so the steal leaves no click. Never panics.
    pub fn play_clip(
        &mut self,
        clip: &Clip,
        route: VoiceRoute,
        gains: (f32, f32),
        fold_mono: bool,
    ) -> Option<VoiceId> {
        self.play_clip_with(clip, route, gains, fold_mono, VoiceStart::default())
    }

    /// [`Self::play_clip`] starting at `start`: from a frame offset and/or
    /// ramped in, which is how a music loop joins mid-pattern without a step.
    pub fn play_clip_with(
        &mut self,
        clip: &Clip,
        route: VoiceRoute,
        gains: (f32, f32),
        fold_mono: bool,
        start: VoiceStart,
    ) -> Option<VoiceId> {
        self.reap();
        if !self.make_room(route) {
            self.stats.rejected += 1;
            return None;
        }
        self.stats.started += 1;
        Some(self.push_voice(
            Arc::clone(&clip.samples),
            clip.channels,
            (sanitize_gain(gains.0), sanitize_gain(gains.1)),
            fold_mono,
            Some(route),
            start,
        ))
    }

    /// Fade voice `id` out over `frames` and drop it, from whatever level
    /// it has reached. True when the voice was live and not already fading;
    /// when the fade reserve is full the voice is cut instead.
    pub fn fade_out(&mut self, id: VoiceId, frames: u32) -> bool {
        let Some(idx) = self.voices.iter().position(|v| v.id == id && !v.retiring()) else {
            return false;
        };
        if self.fading_voices() < FADE_RESERVE {
            self.voices[idx].gate.fade_to(false, frames);
        } else {
            self.voices.remove(idx);
        }
        true
    }

    /// Append a voice at the end of the issue order.
    fn push_voice(
        &mut self,
        data: Arc<[f32]>,
        channels: u16,
        gains: (f32, f32),
        fold_mono: bool,
        route: Option<VoiceRoute>,
        start: VoiceStart,
    ) -> VoiceId {
        let id = VoiceId(self.next_voice);
        self.next_voice = self.next_voice.wrapping_add(1).max(1);
        let frames = data.len() / usize::from(channels.max(1));
        let mut gate = Declicker::settled(false);
        gate.fade_to(true, start.fade_in_frames);
        self.voices.push(Voice {
            id,
            data,
            channels,
            cursor: start.offset_frames.min(frames),
            frame_count: frames,
            gain_l: gains.0,
            gain_r: gains.1,
            fold_mono,
            route,
            gate,
            trim: 1.0,
            lp_coeff: 1.0,
            lp_state: (0.0, 0.0),
        });
        id
    }

    /// Drop voices that ran out of frames or finished fading out.
    fn reap(&mut self) {
        self.voices.retain(|v| !v.finished());
    }

    /// Free a slot for `route`, stealing if needed; false when it must be
    /// refused.
    fn make_room(&mut self, route: VoiceRoute) -> bool {
        let limit = usize::from(self.group_limits[route.group.index()]);
        if limit == 0 {
            return false;
        }
        while self.group_voices(route.group) >= limit {
            if !self.steal(Some(route.group), route.priority) {
                return false;
            }
        }
        self.active_voices() < MAX_VOICES || self.steal(None, route.priority)
    }

    /// Retire the weakest voice (in `group`, or anywhere) when its priority
    /// is at most `priority`.
    fn steal(&mut self, group: Option<Group>, priority: u8) -> bool {
        let weakest = self
            .voices
            .iter()
            .enumerate()
            .filter(|(_, v)| !v.retiring() && group.is_none_or(|g| v.in_group(g)))
            .min_by_key(|(_, v)| v.priority())
            .map(|(idx, _)| idx);
        match weakest {
            Some(idx) if self.voices[idx].priority() <= priority => {
                self.retire(idx);
                self.stats.stolen += 1;
                true
            }
            _ => false,
        }
    }

    /// Start fading voice `idx` out, or cut it when the reserve of fading
    /// voices is full.
    fn retire(&mut self, idx: usize) {
        if self.fading_voices() < FADE_RESERVE {
            self.voices[idx].gate.fade_to(false, self.steal_fade_frames);
        } else {
            self.voices.remove(idx);
        }
    }

    /// Voices that count against limits: everything not fading out.
    pub fn active_voices(&self) -> usize {
        self.voices.iter().filter(|v| !v.retiring()).count()
    }

    /// Stolen voices still fading out (at most [`FADE_RESERVE`]).
    pub fn fading_voices(&self) -> usize {
        self.voices.iter().filter(|v| v.retiring()).count()
    }

    /// Live voices in `group`; a fading voice no longer counts.
    pub fn group_voices(&self, group: Group) -> usize {
        self.voices
            .iter()
            .filter(|v| !v.retiring() && v.in_group(group))
            .count()
    }

    /// Cap simultaneous voices of `group` (0 mutes the group).
    pub fn set_group_limit(&mut self, group: Group, limit: u8) {
        self.group_limits[group.index()] = limit;
    }

    /// Current instance limit of `group`.
    pub fn group_limit(&self, group: Group) -> u8 {
        self.group_limits[group.index()]
    }

    /// Stop one voice; true when it was live.
    pub fn stop(&mut self, id: VoiceId) -> bool {
        let before = self.voices.len();
        self.voices.retain(|v| v.id != id);
        self.voices.len() != before
    }

    /// Stop every voice of `cue`; returns how many stopped.
    pub fn stop_cue(&mut self, cue: CueId) -> usize {
        let before = self.voices.len();
        self.voices
            .retain(|v| v.route.map(|route| route.cue) != Some(cue));
        before - self.voices.len()
    }

    /// Set the live multiplier of every voice of `cue` to `trim`
    /// (sanitized into `0.0..=MAX_GAIN`); returns how many voices took it.
    /// Absolute, not cumulative: `1.0` puts the voices back at their play
    /// level. A cue with no live voice is a no-op (`0`).
    pub fn set_cue_trim(&mut self, cue: CueId, trim: f32) -> usize {
        let trim = sanitize_gain(trim);
        let mut touched = 0;
        for voice in &mut self.voices {
            if voice.route.map(|route| route.cue) == Some(cue) {
                voice.trim = trim;
                touched += 1;
            }
        }
        touched
    }

    /// Darken voice `id` with a one-pole lowpass at `cutoff_hz` (the wall
    /// muffle of [`occlude::Muffle`]). Absolute, not cumulative: a cutoff at
    /// or above [`occlude::OPEN_CUTOFF_HZ`] opens the voice again. True when
    /// the voice is live. Never allocates.
    pub fn set_voice_lowpass(&mut self, id: VoiceId, cutoff_hz: u32) -> bool {
        let coeff = lowpass_coeff(cutoff_hz, self.sample_rate);
        let Some(voice) = self.voices.iter_mut().find(|v| v.id == id) else {
            return false;
        };
        voice.lp_coeff = coeff;
        true
    }

    /// Start voice `id` `frames` into its clip: the head is skipped, never
    /// more than half the clip so a short hit keeps its body. True when the
    /// voice is live. Never allocates.
    pub fn skip_voice(&mut self, id: VoiceId, frames: u32) -> bool {
        let Some(voice) = self.voices.iter_mut().find(|v| v.id == id) else {
            return false;
        };
        let skip = usize::try_from(frames).unwrap_or(usize::MAX);
        let target = voice.cursor.saturating_add(skip).min(voice.frames() / 2);
        voice.cursor = voice.cursor.max(target);
        true
    }

    /// Music duck gain right now (`1.0` untouched, falling toward
    /// [`DUCK_FLOOR_PERCENT`] percent while dialogue plays).
    pub fn duck_gain(&self) -> f32 {
        self.duck.gain()
    }

    /// Voice counters since construction.
    pub fn stats(&self) -> MixerStats {
        self.stats
    }

    /// Live (unfinished) voice count, fading voices included: they are
    /// still audible.
    pub fn voice_count(&self) -> usize {
        self.voices.iter().filter(|v| !v.finished()).count()
    }

    /// True when no voice has frames left to render.
    pub fn is_idle(&self) -> bool {
        self.voice_count() == 0
    }

    /// Sum the current frame of every voice, mono or stereo, into the master
    /// and bus accumulators.
    fn accumulate(&mut self, mono: bool) -> FrameMix {
        let mut mix = FrameMix {
            raw: (0.0, 0.0),
            bus: [(0.0, 0.0); 3],
            duck: false,
        };
        for v in &mut self.voices {
            let ramp = v.fade_gain() * v.trim;
            let (l, r) = if mono {
                let gain = 0.5 * (v.gain_l + v.gain_r) * ramp;
                let m = mono_in(&v.data, v.channels, v.cursor);
                (v.lowpass((m, m)).0 * gain, 0.0)
            } else {
                let (sl, sr) = v.stereo();
                let (sl, sr) = v.lowpass((sl, sr));
                (sl * v.gain_l * ramp, sr * v.gain_r * ramp)
            };
            let slot = match v.route {
                None => &mut mix.raw,
                Some(route) => {
                    mix.duck |= route.ducks_music;
                    &mut mix.bus[bus_index(route.bus)]
                }
            };
            slot.0 += l;
            slot.1 += r;
        }
        mix
    }

    /// Apply bus gains and the music duck to one frame's sums.
    fn finish(&mut self, mix: &FrameMix) -> (f32, f32) {
        let duck = self.duck.step(mix.duck);
        let g = self.bus_gain;
        let sum = |raw: f32, sfx: f32, music: f32, dialogue: f32| {
            raw + sfx * g[0] + music * (g[1] * duck) + dialogue * g[2]
        };
        (
            sum(mix.raw.0, mix.bus[0].0, mix.bus[1].0, mix.bus[2].0),
            sum(mix.raw.1, mix.bus[0].1, mix.bus[1].1, mix.bus[2].1),
        )
    }

    /// Move every cursor and fade on one frame, wrap looped voices and drop
    /// the finished ones.
    fn advance(&mut self) {
        let mut ended = false;
        for v in &mut self.voices {
            v.cursor += 1;
            if v.cursor >= v.frames() && v.route.is_some_and(|route| route.looped) {
                v.cursor = 0;
            }
            v.gate.advance();
            ended |= v.finished();
        }
        if ended {
            self.reap();
        }
    }

    /// Advance one mono frame: deterministic sum over voices in issue order,
    /// per-voice gain applied, hard-clipped. Finished voices drop.
    pub fn render_frame_mono(&mut self) -> f32 {
        let mix = self.accumulate(true);
        self.advance();
        clip_sample(self.finish(&mix).0)
    }

    /// Advance one stereo frame `(left, right)`; same order/clip/drop rules
    /// as [`render_frame_mono`](Self::render_frame_mono).
    pub fn render_frame_stereo(&mut self) -> (f32, f32) {
        let mix = self.accumulate(false);
        self.advance();
        let (l, r) = self.finish(&mix);
        (clip_sample(l), clip_sample(r))
    }

    /// Mix `out.len()` mono frames in place (sum, per-voice gain, clip).
    pub fn render_into(&mut self, out: &mut [f32]) {
        for slot in out.iter_mut() {
            *slot = self.render_frame_mono();
        }
    }

    /// Mix `n` mono frames into a new buffer. Allocates: offline use only,
    /// the callback renders into its own buffer via [`Self::render_into`].
    pub fn render(&mut self, n: usize) -> Vec<f32> {
        let mut out = zeroed(n);
        self.render_into(&mut out);
        out
    }

    /// Mix interleaved stereo (`LRLR...`) in place; `out.len() / 2` frames. A
    /// trailing odd slot, if any, is zeroed. Never panics.
    pub fn render_stereo_into(&mut self, out: &mut [f32]) {
        let frames = out.len() / 2;
        for i in 0..frames {
            let (l, r) = self.render_frame_stereo();
            out[2 * i] = l;
            out[2 * i + 1] = r;
        }
        if out.len() % 2 == 1 {
            if let Some(last) = out.last_mut() {
                *last = 0.0;
            }
        }
    }

    /// Mix `n` stereo frames into a new interleaved buffer. Allocates:
    /// offline use only, the callback renders into its own buffer via
    /// [`Self::render_stereo_into`].
    pub fn render_stereo(&mut self, n: usize) -> Vec<f32> {
        let mut out = zeroed(n.saturating_mul(2));
        self.render_stereo_into(&mut out);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sine_exact_peak_rms_2k_at_8k() {
        let m = Mixer::new(8_000);
        let (peak, rms) = m.render_sine(2_000.0, 8);
        assert!((peak - 0.5).abs() < 1e-6, "peak={peak}");
        let expected = 0.5 / std::f32::consts::SQRT_2;
        assert!((rms - expected).abs() < 1e-6, "rms={rms}");
    }

    #[test]
    fn sine_440_at_48k_peak_rms() {
        let m = Mixer::new(48_000);
        let (peak, rms) = m.render_sine(440.0, 48_000);
        assert!((peak - 0.5).abs() < 1e-3, "peak={peak}");
        let expected = 0.5 / std::f32::consts::SQRT_2;
        assert!((rms - expected).abs() < 1e-4, "rms={rms}");
    }

    #[test]
    fn determinism_same_params_same_bits() {
        let m = Mixer::new(48_000);
        let a = m.render_sine(440.0, 48_000);
        assert_eq!(a, m.render_sine(440.0, 48_000));
        assert_eq!(a, Mixer::new(48_000).render_sine(440.0, 48_000));
    }

    #[test]
    fn silence_and_edges() {
        let m = Mixer::new(48_000);
        assert_eq!(m.render_sine(0.0, 48_000), (0.0, 0.0));
        assert_eq!(m.render_sine(440.0, 1), (0.0, 0.0));
        let (peak, rms) = m.render_sine(440.0, 0);
        assert_eq!(peak, 0.0);
        assert!(rms.is_nan(), "rms={rms}");
        assert!(!rms_gate_open(0.0, 0.001));
        assert!(!rms_gate_open(0.001, 0.001));
        assert!(rms_gate_open(0.001_001, 0.001));
    }

    fn mono(rate: u32, samples: &[f32]) -> Pcm {
        Pcm::new(rate, 1, samples.to_vec())
    }

    // -----------------------------------------------------------------------
    // PF5 mix receipt: the cached frame count against the same render with the
    // count recomputed from `data.len() / channels` on every voice-frame. The
    // recompute path is a test-only twin of the production loop, kept so both
    // sides of the receipt run on one fixture in one process. Only the source
    // of the frame count differs; the float work is bit-for-bit the same.
    // -----------------------------------------------------------------------

    /// Voices in the PF5 mix receipt: a busy combat frame.
    const PF5_VOICES: usize = 32;
    /// Frames per receipt block (10 ms at 48 kHz).
    const PF5_FRAMES: usize = 480;
    /// Timed rounds; the p50 of each side is the receipt.
    const PF5_ITERS: usize = 200;
    /// Frames in every fixture clip (long enough to outlast one block).
    const PF5_CLIP_FRAMES: usize = 9_600;

    impl Mixer {
        /// The pre-change render: same frame walk as
        /// [`Mixer::render_stereo_into`], frame count from `legacy_frames`.
        fn render_stereo_into_legacy(&mut self, out: &mut [f32]) {
            let frames = out.len() / 2;
            for i in 0..frames {
                let mix = self.accumulate(false);
                self.advance_legacy();
                let (l, r) = self.finish(&mix);
                out[2 * i] = clip_sample(l);
                out[2 * i + 1] = clip_sample(r);
            }
            if out.len() % 2 == 1 {
                if let Some(last) = out.last_mut() {
                    *last = 0.0;
                }
            }
        }

        /// [`Mixer::advance`] over the recomputed frame count.
        fn advance_legacy(&mut self) {
            let mut ended = false;
            for v in &mut self.voices {
                v.cursor += 1;
                if v.cursor >= v.legacy_frames() && v.route.is_some_and(|route| route.looped) {
                    v.cursor = 0;
                }
                v.gate.advance();
                ended |= v.cursor >= v.legacy_frames() || (v.retiring() && v.gate.level() == 0);
            }
            if ended {
                self.reap();
            }
        }
    }

    impl Voice {
        /// The frame count the change used to recompute per voice-frame.
        #[must_use]
        fn legacy_frames(&self) -> usize {
            self.data.len() / usize::from(self.channels.max(1))
        }
    }

    /// The receipt's shared clip set: mono and stereo takes of equal length.
    fn pf5_clips() -> Vec<Clip> {
        (0..PF5_VOICES)
            .map(|i| {
                let channels = if i % 3 == 0 { 2u16 } else { 1 };
                let samples = vec![0.05f32; PF5_CLIP_FRAMES * usize::from(channels)];
                Clip {
                    sample_rate: 48_000,
                    channels,
                    samples: Arc::from(samples),
                }
            })
            .collect()
    }

    /// One armed mixer for the receipt: every clip routed, some looped, some
    /// mono-folded, both buses plus dialogue (which ducks music), and two
    /// plain master voices, so the per-frame voice walk is the real mix loop.
    fn pf5_mixer(clips: &[Clip]) -> Mixer {
        let mut m = Mixer::new(48_000);
        m.set_group_limit(Group::Impact, u8::MAX);
        for (i, clip) in clips.iter().enumerate() {
            let bus = if i % 3 == 0 {
                Bus::Dialogue
            } else if i % 2 == 0 {
                Bus::Sfx
            } else {
                Bus::Music
            };
            let route = VoiceRoute {
                cue: CueId(u16::try_from(i).unwrap_or(0)),
                bus,
                group: Group::Impact,
                priority: u8::try_from(i % 8).unwrap_or(0),
                looped: i % 4 == 0,
                ducks_music: bus == Bus::Dialogue,
            };
            m.play_clip(clip, route, (0.5, 0.25), i % 5 == 0)
                .expect("the fixture leaves room for every voice");
        }
        let plain = Pcm::new(48_000, 1, vec![0.03f32; PF5_CLIP_FRAMES]);
        for _ in 0..2 {
            m.play(&plain, 0.25);
        }
        m
    }

    /// `ns` in microseconds for the receipt line (no cast).
    fn pf5_micros(ns: u128) -> f64 {
        engine_core::checked::f64_from_u64(u64::try_from(ns).unwrap_or(u64::MAX)) / 1000.0
    }

    /// How many times slower `slow` is than `fast`, as a receipt figure.
    fn pf5_ratio(slow: u128, fast: u128) -> f64 {
        if fast == 0 {
            return 0.0;
        }
        engine_core::checked::f64_from_u64(u64::try_from(slow).unwrap_or(u64::MAX))
            / engine_core::checked::f64_from_u64(u64::try_from(fast).unwrap_or(u64::MAX))
    }

    /// The cached frame count must not move a rounded sample: the production
    /// render equals the recompute twin on one fixture, byte for byte.
    #[test]
    fn pf5_voice_frames_cache_renders_the_same_bytes() {
        let clips = pf5_clips();
        let mut cached = pf5_mixer(&clips);
        let mut recomputed = pf5_mixer(&clips);
        for v in &cached.voices {
            assert_eq!(v.frames(), v.legacy_frames(), "cache drifted from data");
        }
        let mut a = vec![0.0f32; PF5_FRAMES * 2];
        let mut b = vec![0.0f32; PF5_FRAMES * 2];
        cached.render_stereo_into(&mut a);
        recomputed.render_stereo_into_legacy(&mut b);
        assert_eq!(a, b, "the cached render moved a sample");
        assert_eq!(cached.voice_count(), recomputed.voice_count());
        assert!(a.iter().any(|&s| s != 0.0), "the fixture is audible");
    }

    /// PF5 mix receipt: p50 microseconds for one 480-frame stereo block, the
    /// cached frame count against the same render recomputing
    /// `data.len() / channels` on every voice-frame, alternating rounds on one
    /// fixture so one machine load sits under both sides.
    /// Run: cargo test --release -p engine-audio --lib -- --ignored --nocapture
    /// pf5_mix_voice_frames_bench_receipt
    #[test]
    #[ignore = "timing: run with --ignored --nocapture (release) for the receipt"]
    fn pf5_mix_voice_frames_bench_receipt() {
        let clips = pf5_clips();
        let mut cached_runs = Vec::with_capacity(PF5_ITERS);
        let mut recompute_runs = Vec::with_capacity(PF5_ITERS);
        for _ in 0..PF5_ITERS {
            let mut recomputed = pf5_mixer(&clips);
            let mut cached = pf5_mixer(&clips);
            let mut a = vec![0.0f32; PF5_FRAMES * 2];
            let mut b = vec![0.0f32; PF5_FRAMES * 2];
            let t = std::time::Instant::now();
            recomputed.render_stereo_into_legacy(&mut a);
            recompute_runs.push(t.elapsed().as_nanos());
            let t = std::time::Instant::now();
            cached.render_stereo_into(&mut b);
            cached_runs.push(t.elapsed().as_nanos());
            assert_eq!(a, b, "identical output on every receipt round");
            std::hint::black_box(a.len() + b.len());
        }
        recompute_runs.sort_unstable();
        cached_runs.sort_unstable();
        let recompute = recompute_runs[PF5_ITERS / 2];
        let cached = cached_runs[PF5_ITERS / 2];
        eprintln!(
            "pf5_mix_frames cached_p50_us={:.3} recompute_p50_us={:.3} speedup={:.3}x voices={PF5_VOICES} frames={PF5_FRAMES}",
            pf5_micros(cached),
            pf5_micros(recompute),
            pf5_ratio(recompute, cached)
        );
    }

    #[test]
    fn mixer_sums_voices_deterministically() {
        let a = mono(48_000, &[0.25, 0.25, 0.25, 0.25]);
        let b = mono(48_000, &[0.5, 0.5, 0.5, 0.5]);
        let run = || {
            let mut m = Mixer::new(48_000);
            m.play(&a, 1.0);
            m.play(&b, 1.0);
            m.render(4)
        };
        let (first, second) = (run(), run());
        assert_eq!(first, vec![0.75; 4]);
        assert_eq!(first, second, "same play order gives bit-identical mix");
    }

    #[test]
    fn mixer_clips_and_applies_per_voice_gain() {
        let loud = mono(8_000, &[0.9, 0.9]);
        let mut m = Mixer::new(8_000);
        m.play(&loud, 1.0);
        m.play(&loud, 1.0);
        assert_eq!(m.render(2), vec![1.0, 1.0], "0.9 + 0.9 hard-clips to 1.0");
        let half = mono(8_000, &[0.8, 0.8]);
        let mut m = Mixer::new(8_000);
        m.play(&half, 0.5);
        assert_eq!(m.render(2), vec![0.4, 0.4]);
    }

    #[test]
    fn mixer_stereo_folds_mono_and_keeps_channels() {
        let m = mono(8_000, &[0.5, -0.5]);
        let mut mix = Mixer::new(8_000);
        mix.play(&m, 1.0);
        assert_eq!(mix.render_stereo(2), vec![0.5, 0.5, -0.5, -0.5]);
        let st = Pcm::new(8_000, 2, vec![0.25, -0.25, 0.5, -0.5]);
        let mut mix = Mixer::new(8_000);
        mix.play(&st, 1.0);
        assert_eq!(mix.render_stereo(2), vec![0.25, -0.25, 0.5, -0.5]);
        let mut mix = Mixer::new(8_000);
        mix.play(&st, 1.0);
        assert_eq!(mix.render(2), vec![0.0, 0.0], "mono render averages L/R");
    }

    #[test]
    fn mixer_gain_and_sample_edges_never_panic() {
        assert_eq!(clip_sample(f32::NAN), 0.0);
        assert_eq!(clip_sample(9.0), 1.0);
        assert_eq!(clip_sample(-9.0), -1.0);
        assert_eq!(sanitize_gain(f32::NAN), 0.0);
        assert_eq!(sanitize_gain(-3.0), 0.0);
        assert_eq!(sanitize_gain(f32::INFINITY), MAX_GAIN);
        let nan_pcm = Pcm::new(8_000, 1, vec![f32::NAN, f32::INFINITY, f32::NEG_INFINITY]);
        assert!(nan_pcm.samples.iter().all(|s| s.is_finite()));
        let mut m = Mixer::new(8_000);
        m.play(&nan_pcm, f32::NAN);
        assert_eq!(m.render(3), vec![0.0, 0.0, 0.0]);
        let empty = Pcm::new(8_000, 0, vec![]);
        assert_eq!(empty.channels, 1);
        let mut m = Mixer::new(8_000);
        let id = m.play(&empty, 1.0);
        assert!(m.render(4).iter().all(|&s| s == 0.0));
        assert!(m.is_idle());
        assert!(!m.stop(id), "finished empty voice already reaped");
    }

    #[test]
    fn mixer_stop_and_voice_cap() {
        let s = mono(8_000, &vec![0.1; 64]);
        let mut m = Mixer::new(8_000);
        let id = m.play(&s, 1.0);
        assert_eq!(m.voice_count(), 1);
        assert!(m.stop(id));
        assert!(!m.stop(id));
        assert!(m.is_idle());
        let mut m = Mixer::new(8_000);
        for _ in 0..MAX_VOICES + 8 {
            m.play(&s, 1.0);
        }
        assert!(m.voice_count() <= MAX_VOICES);
        let out = m.render(64);
        assert_eq!(out.len(), 64);
        assert!(m.is_idle(), "all voices consumed after their frames");
    }

    #[test]
    fn open_lowpass_leaves_the_mix_bit_identical_and_dead_ids_refuse() {
        let s = mono(
            48_000,
            &(0..256u32)
                .map(|i| (f32_of(i) * 0.37).sin())
                .collect::<Vec<_>>(),
        );
        let mut plain = Mixer::new(48_000);
        plain.play(&s, 0.8);
        let want = plain.render(256);
        let mut opened = Mixer::new(48_000);
        let id = opened.play(&s, 0.8);
        assert!(opened.set_voice_lowpass(id, occlude::OPEN_CUTOFF_HZ));
        assert_eq!(
            opened.render(256),
            want,
            "open is a bypass, not a near-flat filter"
        );
        let mut dark = Mixer::new(48_000);
        let id = dark.play(&s, 0.8);
        assert!(dark.set_voice_lowpass(id, 500));
        assert_ne!(dark.render(256), want, "a closed filter changes the mix");
        assert!(
            !dark.set_voice_lowpass(id, 500),
            "the voice ran out of frames"
        );
    }

    /// Run one render call under the probe and require zero heap calls.
    fn silent_call(label: &str, call: impl FnOnce()) {
        let ((), heap) = engine_core::alloc_probe::measure(call);
        assert_eq!(heap.calls, 0, "{label} allocated {} bytes", heap.bytes);
    }

    #[test]
    fn every_render_call_allocates_nothing_after_warmup() {
        use crate::dsp::{render_song_into, BusMixer};
        use crate::music::{bake, pcm_len, render_pcm_into};
        const FRAMES: usize = 480;
        // Unequal voices: the short one finishes and drops inside the calls.
        let short = mono(48_000, &vec![0.25; FRAMES + 7]);
        let long = Pcm::new(48_000, 2, vec![0.125; FRAMES * 2 * 4]);
        let loudest = |buf: &[f32]| buf.iter().fold(0.0f32, |m, s| m.max(s.abs()));

        let mut mixer = Mixer::new(48_000);
        mixer.play(&short, 1.0);
        mixer.play(&long, 0.5);
        let mut mono_buf = vec![0.0f32; FRAMES];
        let mut heard = 0.0f32;
        for _ in 0..6 {
            silent_call("Mixer::render_into", || mixer.render_into(&mut mono_buf));
            heard = heard.max(loudest(&mono_buf));
        }
        assert!(
            heard > 0.0 && mixer.is_idle(),
            "mixed, then every voice ended"
        );

        let mut mixer = Mixer::new(48_000);
        mixer.play(&short, 1.0);
        mixer.play(&long, 0.5);
        let mut stereo_buf = vec![0.0f32; FRAMES * 2];
        let mut heard = 0.0f32;
        for _ in 0..6 {
            silent_call("Mixer::render_stereo_into", || {
                mixer.render_stereo_into(&mut stereo_buf);
            });
            heard = heard.max(loudest(&stereo_buf));
        }
        assert!(
            heard > 0.0 && mixer.is_idle(),
            "stereo mixed, then every voice ended"
        );

        let mut spatial = SpatialMixer::new(48_000);
        spatial.set_listener(Pos2::new(0, 0), 0);
        spatial.play_at(&short, Pos2::new(150, 40), 1.0);
        spatial.play_at(&long, Pos2::new(-90, 10), 1.0);
        let mut heard = 0.0f32;
        for _ in 0..6 {
            silent_call("SpatialMixer::render_stereo_into", || {
                spatial.render_stereo_into(&mut stereo_buf);
            });
            heard = heard.max(loudest(&stereo_buf));
        }
        assert!(heard > 0.0 && spatial.is_idle(), "spatial mixed, then idle");

        let bus = BusMixer::new(48_000);
        let (sfx, music, dlg) = (vec![0.5; FRAMES], vec![0.25; FRAMES], vec![0.1; FRAMES]);
        let mut master = Vec::new();
        bus.mix_into(&sfx, &music, &dlg, &mut master);
        for _ in 0..4 {
            silent_call("BusMixer::mix_into", || {
                bus.mix_into(&sfx, &music, &dlg, &mut master);
            });
        }
        assert!(loudest(&master) > 0.0, "the bus mixed audio");

        let mut song_buf = vec![0.0f32; FRAMES];
        silent_call("render_song_into", || {
            render_song_into(2040, 48_000, &mut song_buf);
        });
        assert!(loudest(&song_buf) > 0.0, "the song rendered audio");
        let song = bake(2040, 2);
        let mut pcm = vec![0i16; pcm_len(&song, 8_000, 4_000)];
        silent_call("render_pcm_into", || {
            render_pcm_into(&song, 1.0, 8_000, &mut pcm);
        });
        assert!(pcm.iter().any(|&s| s != 0), "the bake rendered audio");
    }

    #[test]
    fn alloc_probe_sees_a_real_allocation() {
        let (_, heap) = engine_core::alloc_probe::measure(|| std::hint::black_box(vec![0u8; 64]));
        assert_eq!(
            heap.calls, 1,
            "the test allocator counts, so zeros are real"
        );
    }

    #[test]
    fn steady_state_render_into_allocates_nothing() {
        use engine_core::alloc_probe::measure;
        // The per-tick mono hot path fills a reused buffer via
        // `render_into` (cpal reuse pattern). One long voice stays live for
        // the whole window so every tick mixes, never idles.
        const FRAMES: usize = 800;
        const TICKS: usize = 600;
        let sample = mono(48_000, &vec![0.25; FRAMES * TICKS]);
        let mut mixer = Mixer::new(48_000);
        mixer.play(&sample, 1.0);
        let mut scratch = vec![0.0f32; FRAMES];
        mixer.render_into(&mut scratch);
        assert!(scratch.iter().any(|&s| s != 0.0));
        let (folded, heap) = measure(|| {
            let mut folded = 0u64;
            for tick in 0..TICKS {
                mixer.render_into(&mut scratch);
                for &s in &scratch {
                    folded = folded
                        .wrapping_add(s.to_bits() as u64)
                        .wrapping_add(tick as u64);
                }
            }
            folded
        });
        std::hint::black_box(&scratch);
        assert_eq!(heap.calls, 0, "steady-state render_into must not allocate");
        assert!(folded != 0, "sanity: ticks mixed audio during the window");
    }

    #[test]
    fn steady_state_render_stereo_into_allocates_nothing() {
        use engine_core::alloc_probe::measure;
        // Stereo leg plus spatial delegation: `SpatialMixer` fills a reused
        // interleaved buffer via `render_stereo_into`.
        const FRAMES: usize = 800;
        const TICKS: usize = 600;
        let sample = mono(48_000, &vec![0.5; FRAMES * TICKS]);
        let mut mix = SpatialMixer::new(48_000);
        mix.set_listener(Pos2::new(0, 0), 0);
        mix.play_at(&sample, Pos2::new(0, 0), 1.0);
        let mut scratch = vec![0.0f32; FRAMES * 2];
        mix.render_stereo_into(&mut scratch);
        assert!(scratch.iter().any(|&s| s != 0.0));
        let (folded, heap) = measure(|| {
            let mut folded = 0u64;
            for tick in 0..TICKS {
                mix.render_stereo_into(&mut scratch);
                for &s in &scratch {
                    folded = folded
                        .wrapping_add(s.to_bits() as u64)
                        .wrapping_add(tick as u64);
                }
            }
            folded
        });
        std::hint::black_box(&scratch);
        assert_eq!(
            heap.calls, 0,
            "steady-state render_stereo_into must not allocate"
        );
        assert!(folded != 0, "sanity: ticks mixed audio during the window");
    }

    #[test]
    fn steady_state_bus_mix_into_reuses_buffer() {
        use crate::dsp::BusMixer;
        use engine_core::alloc_probe::measure;
        // The bus hot path reuses one `Vec` via `mix_into`. Capacity is
        // warmed once outside the window, so `reserve` inside is a no-op.
        const FRAMES: usize = 800;
        const TICKS: usize = 600;
        let sfx = vec![0.5f32; FRAMES];
        let music = vec![0.25f32; FRAMES];
        let dlg = vec![0.0f32; FRAMES];
        let bus = BusMixer::new(48_000);
        let mut out = Vec::new();
        bus.mix_into(&sfx, &music, &dlg, &mut out);
        assert_eq!(out, bus.mix(&sfx, &music, &dlg));
        assert_eq!(out.len(), FRAMES);
        let (folded, heap) = measure(|| {
            let mut folded = 0u64;
            for tick in 0..TICKS {
                bus.mix_into(&sfx, &music, &dlg, &mut out);
                for &s in &out {
                    folded = folded
                        .wrapping_add(s.to_bits() as u64)
                        .wrapping_add(tick as u64);
                }
            }
            folded
        });
        std::hint::black_box(&out);
        assert_eq!(heap.calls, 0, "steady-state mix_into must not allocate");
        assert!(folded != 0, "sanity: buses mixed audio during the window");
    }

    fn clip(value: f32, frames: usize) -> Clip {
        Clip::from_pcm(&mono(8_000, &vec![value; frames]))
    }

    fn route(cue: u16, bus: Bus, group: Group, priority: u8, looped: bool) -> VoiceRoute {
        VoiceRoute {
            cue: CueId(cue),
            bus,
            group,
            priority,
            looped,
            ducks_music: bus == Bus::Dialogue,
        }
    }

    const BOTH: (f32, f32) = (1.0, 1.0);

    /// True while `id` is a voice that still counts (not fading out).
    fn has(mixer: &Mixer, id: VoiceId) -> bool {
        mixer.voices.iter().any(|v| v.id == id && !v.retiring())
    }

    #[test]
    fn cue_voices_share_samples_and_start_without_heap() {
        use engine_core::alloc_probe::measure;
        let shared = clip(0.25, 64);
        let r = route(0, Bus::Sfx, Group::Impact, 10, false);
        let mut m = Mixer::new(8_000);
        m.play_clip(&shared, r, BOTH, false).expect("room");
        let ((), heap) = measure(|| {
            for _ in 0..3 {
                m.play_clip(&shared, r, BOTH, false);
            }
        });
        assert_eq!(heap.calls, 0, "starting a cue voice copies no samples");
        assert!(Arc::ptr_eq(&shared.samples, &m.voices[0].data));
        assert_eq!(Arc::strong_count(&shared.samples), 1 + m.voice_count());
    }

    #[test]
    fn full_pool_steals_lowest_priority_then_oldest() {
        let c = clip(0.01, 4_000);
        let r = |p| route(0, Bus::Sfx, Group::Impact, p, false);
        let mut m = Mixer::new(8_000);
        m.set_group_limit(Group::Impact, 255);
        let first = m.play_clip(&c, r(5), BOTH, false).expect("room");
        let second = m.play_clip(&c, r(5), BOTH, false).expect("room");
        for _ in 2..MAX_VOICES {
            m.play_clip(&c, r(9), BOTH, false).expect("room");
        }
        assert_eq!(m.voice_count(), MAX_VOICES);
        let a = m.play_clip(&c, r(7), BOTH, false).expect("steals");
        assert!(
            !has(&m, first) && has(&m, second),
            "oldest of the lowest goes first"
        );
        let b = m.play_clip(&c, r(7), BOTH, false).expect("steals");
        assert!(!has(&m, second), "then the next lowest");
        let c3 = m
            .play_clip(&c, r(7), BOTH, false)
            .expect("equal priority steals the oldest");
        assert!(!has(&m, a) && has(&m, b) && has(&m, c3));
        assert!(
            m.play_clip(&c, r(3), BOTH, false).is_none(),
            "weaker than everything: refused"
        );
        assert_eq!(m.active_voices(), MAX_VOICES);
        let d = m.play_clip(&c, r(8), BOTH, false).expect("beats the 7s");
        assert!(!has(&m, b) && has(&m, c3) && has(&m, d));
        assert_eq!(
            m.stats(),
            MixerStats {
                started: u64::try_from(MAX_VOICES + 4).expect("small"),
                stolen: 4,
                rejected: 1
            }
        );
    }

    #[test]
    fn group_limit_steals_inside_the_group_only() {
        let c = clip(0.01, 4_000);
        let step = |p| route(1, Bus::Sfx, Group::Footstep, p, false);
        let mut m = Mixer::new(8_000);
        m.set_group_limit(Group::Footstep, 2);
        let weak_impact = m
            .play_clip(&c, route(0, Bus::Sfx, Group::Impact, 1, false), BOTH, false)
            .expect("room");
        let s10 = m.play_clip(&c, step(10), BOTH, false).expect("room");
        let s20 = m.play_clip(&c, step(20), BOTH, false).expect("room");
        let s15 = m
            .play_clip(&c, step(15), BOTH, false)
            .expect("steals the weakest step");
        assert!(!has(&m, s10) && has(&m, s20) && has(&m, s15));
        assert_eq!(m.group_voices(Group::Footstep), 2);
        assert!(
            m.play_clip(&c, step(5), BOTH, false).is_none(),
            "weaker than the group"
        );
        assert!(
            has(&m, weak_impact),
            "a full group never steals from another group"
        );
        let s15b = m
            .play_clip(&c, step(15), BOTH, false)
            .expect("equal priority steals the oldest");
        assert!(!has(&m, s15) && has(&m, s15b));
    }

    #[test]
    fn muted_group_refuses_every_play() {
        let mut m = Mixer::new(8_000);
        m.set_group_limit(Group::Swing, 0);
        assert_eq!(m.group_limit(Group::Swing), 0);
        let c = clip(0.5, 8);
        assert!(m
            .play_clip(
                &c,
                route(0, Bus::Sfx, Group::Swing, 255, false),
                BOTH,
                false
            )
            .is_none());
        assert_eq!(m.stats().rejected, 1);
        assert!(m.is_idle());
    }

    #[test]
    fn looped_cue_wraps_until_its_cue_is_stopped() {
        let tone = Clip::from_pcm(&mono(8_000, &[0.1, 0.2, 0.3]));
        let mut m = Mixer::new(8_000);
        let id = m
            .play_clip(
                &tone,
                route(4, Bus::Sfx, Group::Crowd, 50, true),
                BOTH,
                false,
            )
            .expect("room");
        let g = Bus::Sfx.gain_linear();
        let left: Vec<f32> = m.render_stereo(7).chunks(2).map(|f| f[0]).collect();
        let want: Vec<f32> = [0.1, 0.2, 0.3, 0.1, 0.2, 0.3, 0.1]
            .iter()
            .map(|v| v * g)
            .collect();
        assert_eq!(left, want, "bus gain applies to cue voices");
        assert_eq!(m.voice_count(), 1);
        assert_eq!(m.stop_cue(CueId(9)), 0, "other cues unaffected");
        assert_eq!(m.stop_cue(CueId(4)), 1);
        assert!(m.is_idle() && !m.stop(id));
        let empty = Clip::from_pcm(&Pcm::new(8_000, 1, vec![]));
        m.play_clip(
            &empty,
            route(4, Bus::Sfx, Group::Crowd, 50, true),
            BOTH,
            false,
        );
        m.render_stereo(2);
        assert!(m.is_idle(), "an empty loop is reaped, not spun forever");
    }

    #[test]
    fn plain_voices_bypass_the_bus_gains() {
        let c = Pcm::new(8_000, 1, vec![0.4; 2]);
        let mut m = Mixer::new(8_000);
        m.play(&c, 1.0);
        assert_eq!(
            m.render_stereo(1),
            vec![0.4, 0.4],
            "master-direct voice, no bus trim"
        );
        assert_eq!(m.duck_gain(), 1.0);
    }

    #[test]
    fn music_ducks_under_dialogue_then_recovers() {
        let mut m = Mixer::new(8_000);
        let music = clip(0.5, 64);
        m.play_clip(
            &music,
            route(1, Bus::Music, Group::Music, 200, true),
            BOTH,
            false,
        )
        .expect("room");
        let (g_music, g_dlg) = (Bus::Music.gain_linear(), Bus::Dialogue.gain_linear());
        let next = |m: &mut Mixer| m.render_frame_stereo().0;
        assert_eq!(next(&mut m), 0.5 * g_music, "no dialogue, no duck");
        assert_eq!(m.duck_gain(), 1.0);

        let line = clip(0.1, 400);
        m.play_clip(
            &line,
            route(2, Bus::Dialogue, Group::Dialogue, 250, false),
            BOTH,
            false,
        )
        .expect("room");
        let mut previous = 1.0;
        for _ in 0..324 {
            next(&mut m);
            let d = m.duck_gain();
            assert!(d <= previous, "attack only falls: {d} after {previous}");
            previous = d;
        }
        let floor = f32_of(DUCK_FLOOR_PERCENT) / 100.0;
        assert!(
            (m.duck_gain() - floor).abs() < 1e-3,
            "40 ms attack reaches {floor}"
        );
        let ducked = next(&mut m);
        let want = 0.5 * (g_music * m.duck_gain()) + 0.1 * g_dlg;
        assert!(
            (ducked - want).abs() < 1e-6,
            "ducked mix {ducked} vs {want}"
        );
        assert!(
            ducked < 0.5 * g_music + 0.1 * g_dlg,
            "music is quieter than undipped"
        );

        // The line ends at frame 400; the 600 ms release is 4800 frames.
        for _ in 0..80 {
            next(&mut m);
        }
        assert_eq!(m.voice_count(), 1, "only the music loop remains");
        let mut previous = m.duck_gain();
        for _ in 0..4_900 {
            next(&mut m);
            let d = m.duck_gain();
            assert!(d >= previous, "release only rises");
            previous = d;
        }
        assert_eq!(m.duck_gain(), 1.0, "fully recovered");
        assert_eq!(next(&mut m), 0.5 * g_music);
    }

    #[test]
    fn duck_released_mid_attack_steps_no_more_than_one_fade_step() {
        let mut m = Mixer::new(8_000);
        m.play_clip(
            &clip(0.5, 20_000),
            route(1, Bus::Music, Group::Music, 200, true),
            BOTH,
            false,
        )
        .expect("room");
        let line = m
            .play_clip(
                &clip(0.1, 20_000),
                route(2, Bus::Dialogue, Group::Dialogue, 250, false),
                BOTH,
                false,
            )
            .expect("room");
        for _ in 0..100 {
            m.render_frame_stereo();
        }
        let mid_attack = m.duck_gain();
        assert!(
            mid_attack < 1.0 && mid_attack > 0.36,
            "mid-attack {mid_attack}"
        );
        assert!(m.stop(line), "the line ends mid-attack");

        // The 40 ms attack covers (1 - floor) in 320 frames at 8 kHz; no
        // frame of the release may move the duck by more than that step.
        let floor = f32_of(DUCK_FLOOR_PERCENT) / 100.0;
        let fade_step = (1.0 - floor) / 320.0;
        let mut previous = mid_attack;
        let mut largest = 0.0f32;
        for _ in 0..4_900 {
            m.render_frame_stereo();
            let d = m.duck_gain();
            largest = largest.max((d - previous).abs());
            previous = d;
        }
        assert!(
            largest <= fade_step + 1e-4,
            "largest release step {largest} vs fade-step {fade_step}"
        );
        assert_eq!(m.duck_gain(), 1.0, "released back to unity");
    }

    #[test]
    fn sfx_alone_never_ducks_the_music() {
        let mut m = Mixer::new(8_000);
        m.play_clip(
            &clip(0.5, 64),
            route(1, Bus::Music, Group::Music, 200, true),
            BOTH,
            false,
        );
        m.play_clip(
            &clip(0.5, 64),
            route(2, Bus::Sfx, Group::Impact, 100, false),
            BOTH,
            false,
        );
        m.render_stereo(64);
        assert_eq!(m.duck_gain(), 1.0);
    }

    /// A mono sine of `amp` at `hz`, `frames` long, starting at phase 0.
    fn sine_clip(hz: f32, amp: f32, frames: usize) -> Clip {
        let samples: Vec<f32> = (0..frames)
            .map(|i| amp * (std::f32::consts::TAU * hz * f32_of(i as u32) / 8_000.0).sin())
            .collect();
        Clip::from_pcm(&mono(8_000, &samples))
    }

    #[test]
    fn stolen_voice_fades_out_and_the_steal_leaves_no_click() {
        // 100 Hz at 8 kHz peaks at frame 20: steal exactly there, where a hard
        // cut would drop the mix by the victim's full 0.5.
        let tone = sine_clip(100.0, 0.5, 4_000);
        let r = |p| route(0, Bus::Sfx, Group::Impact, p, false);
        let mut m = Mixer::new(8_000);
        m.set_group_limit(Group::Impact, 1);
        let victim = m.play_clip(&tone, r(5), BOTH, false).expect("room");
        let mut left: Vec<f32> = m.render_stereo(20).chunks(2).map(|f| f[0]).collect();
        let newcomer = m
            .play_clip(&tone, r(9), BOTH, false)
            .expect("steals the victim");
        assert!(!has(&m, victim) && has(&m, newcomer));
        assert_eq!((m.active_voices(), m.fading_voices()), (1, 1));
        assert_eq!(
            m.group_voices(Group::Impact),
            1,
            "a fading voice no longer counts"
        );
        assert_eq!(m.voice_count(), 2, "but it is still audible");
        left.extend(m.render_stereo(100).chunks(2).map(|f| f[0]));
        let worst = left
            .windows(2)
            .map(|w| (w[1] - w[0]).abs())
            .fold(0.0f32, f32::max);
        let g = Bus::Sfx.gain_linear();
        assert!(
            worst < 0.1 * g,
            "largest step {worst} (a cut would step {})",
            0.5 * g
        );
        assert_eq!(
            m.fading_voices(),
            0,
            "gone once the 8 ms (64 frames) fade ran out"
        );
        assert_eq!(m.voice_count(), 1);
        assert_eq!(m.stats().stolen, 1);
    }

    #[test]
    fn steal_fade_lasts_eight_milliseconds_of_samples_at_any_rate() {
        for (rate, frames) in [(8_000u32, 64usize), (48_000, 384), (44_100, 352)] {
            let held = Clip::from_pcm(&Pcm::new(rate, 1, vec![0.25; 20_000]));
            let r = |p| route(0, Bus::Sfx, Group::Swing, p, false);
            let mut m = Mixer::new(rate);
            m.set_group_limit(Group::Swing, 1);
            m.play_clip(&held, r(1), BOTH, false).expect("room");
            m.play_clip(&held, r(2), BOTH, false).expect("steals");
            let mut ran = 0;
            while m.fading_voices() > 0 {
                m.render_frame_stereo();
                ran += 1;
                assert!(ran <= frames + 1, "fade overran at {rate} Hz");
            }
            assert_eq!(ran, frames, "{rate} Hz");
        }
    }

    #[test]
    fn fade_reserve_caps_fading_voices_and_storage_never_grows() {
        let held = clip(0.1, 20_000);
        let r = |p| route(0, Bus::Sfx, Group::Footstep, p, false);
        let mut m = Mixer::new(8_000);
        m.set_group_limit(Group::Footstep, 1);
        let capacity = m.voices.capacity();
        for p in 0..40u8 {
            m.play_clip(&held, r(p), BOTH, false)
                .expect("equal or higher steals");
            assert!(m.fading_voices() <= FADE_RESERVE);
        }
        assert_eq!(m.fading_voices(), FADE_RESERVE, "excess steals cut instead");
        assert_eq!(m.stats().stolen, 39);
        assert_eq!(m.voices.capacity(), capacity, "no reallocation");
        assert_eq!(m.active_voices(), 1);
    }

    #[test]
    fn stealing_and_fading_allocate_nothing() {
        use engine_core::alloc_probe::measure;
        let held = clip(0.1, 20_000);
        let r = |p| route(0, Bus::Sfx, Group::Swing, p, false);
        let mut m = Mixer::new(8_000);
        m.set_group_limit(Group::Swing, 1);
        m.play_clip(&held, r(0), BOTH, false).expect("room");
        let mut out = [0.0f32; 128];
        let ((), heap) = measure(|| {
            for p in 1..30u8 {
                m.play_clip(&held, r(p), BOTH, false);
                m.render_stereo_into(&mut out);
            }
        });
        assert_eq!(heap.calls, 0, "steal, fade and reap stay off the heap");
        assert!(out.iter().any(|&s| s != 0.0));
    }

    #[test]
    fn a_voice_can_start_mid_clip_and_ramp_in() {
        let wave: Vec<f32> = (0..100u8).map(|i| f32::from(i) / 100.0).collect();
        let ramp = Clip::from_pcm(&mono(8_000, &wave));
        let g = Bus::Sfx.gain_linear();
        let r = route(0, Bus::Sfx, Group::Ambient, 10, false);
        let mut m = Mixer::new(8_000);
        let start = VoiceStart {
            offset_frames: 40,
            fade_in_frames: 0,
        };
        m.play_clip_with(&ramp, r, BOTH, false, start)
            .expect("room");
        assert_eq!(
            m.render_stereo(1)[0],
            0.40 * g,
            "joins at frame 40, full level"
        );
        let late = VoiceStart {
            offset_frames: 500,
            fade_in_frames: 0,
        };
        m.play_clip_with(&ramp, r, BOTH, false, late).expect("room");
        m.render_stereo(1);
        assert_eq!(m.voice_count(), 1, "an offset past the end plays nothing");

        let held = clip(1.0, 400);
        let mut m = Mixer::new(8_000);
        let fade = VoiceStart {
            offset_frames: 0,
            fade_in_frames: 100,
        };
        m.play_clip_with(&held, r, BOTH, false, fade).expect("room");
        let left: Vec<f32> = m.render_stereo(200).chunks(2).map(|f| f[0]).collect();
        assert_eq!(left[0], 0.0, "starts silent");
        assert!(
            left[..100].windows(2).all(|w| w[1] > w[0]),
            "rises every frame"
        );
        assert!(left[..100].iter().all(|&s| s <= g));
        assert_eq!(left[100], g, "full level after the ramp");
        assert!(left[100..].iter().all(|&s| s == g));
    }

    #[test]
    fn fade_out_takes_a_voice_from_its_current_level_to_silence() {
        let held = clip(1.0, 1_000);
        let r = route(0, Bus::Sfx, Group::Ambient, 10, false);
        let g = Bus::Sfx.gain_linear();
        let mut m = Mixer::new(8_000);
        let fade = VoiceStart {
            offset_frames: 0,
            fade_in_frames: 100,
        };
        let id = m.play_clip_with(&held, r, BOTH, false, fade).expect("room");
        let before = m.render_stereo(40);
        let at_call = before[78];
        assert!(m.fade_out(id, 50));
        assert!(!m.fade_out(id, 50), "already fading");
        assert!(!m.fade_out(VoiceId(9_999), 50), "no such voice");
        assert_eq!(
            m.group_voices(Group::Ambient),
            0,
            "a fading voice stops counting"
        );
        let after: Vec<f32> = m.render_stereo(60).chunks(2).map(|f| f[0]).collect();
        assert!(
            after[0] <= 0.40 * g + 1e-4 && after[0] > at_call,
            "continues from the ramp level (frame 40 of 100), no jump"
        );
        // From 40% a 50-frame fade-out runs dry after 20 frames.
        assert!(
            after[..19].windows(2).all(|w| w[1] < w[0]),
            "falls every frame"
        );
        assert_eq!(after[22], 0.0);
        assert!(m.is_idle());
        assert!(at_call < g);
    }

    #[test]
    fn fade_out_cuts_when_the_fade_reserve_is_full() {
        let held = clip(0.1, 20_000);
        let r = |p| route(0, Bus::Sfx, Group::Footstep, p, false);
        let mut m = Mixer::new(8_000);
        m.set_group_limit(Group::Footstep, 1);
        for p in 0..=u8::try_from(FADE_RESERVE).unwrap_or(16) {
            m.play_clip(&held, r(p), BOTH, false).expect("steals");
        }
        assert_eq!(m.fading_voices(), FADE_RESERVE);
        let bed = route(1, Bus::Music, Group::Music, 5, true);
        let extra = m.play_clip(&held, bed, BOTH, false).expect("room");
        assert!(m.fade_out(extra, 100), "reported as handled");
        assert!(!has(&m, extra));
        assert_eq!(m.fading_voices(), FADE_RESERVE, "cut, not queued");
    }

    #[test]
    fn any_route_flagged_ducks_music_ducks_it() {
        let mut m = Mixer::new(8_000);
        let bed = route(1, Bus::Music, Group::Music, 200, true);
        m.play_clip(&clip(0.5, 20_000), bed, BOTH, false)
            .expect("room");
        let mut sting = route(2, Bus::Sfx, Group::Stinger, 230, false);
        sting.ducks_music = true;
        m.play_clip(&clip(0.2, 800), sting, BOTH, false)
            .expect("room");
        m.render_stereo(800);
        assert!(
            m.duck_gain() < 0.4,
            "stinger ducked the bed: {}",
            m.duck_gain()
        );
        m.render_stereo(6_000);
        assert_eq!(m.duck_gain(), 1.0, "and let go");
    }
}
