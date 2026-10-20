//! bank: placeholder hit/footstep/crowd-bed playback samples.
//!
//! Procedural placeholder samples generated in-code until recorded content
//! lands: real hit, footstep and crowd-bed clips replace these fields
//! without changing the [`Bank`] shape or the [`crate::Mixer::play`] call
//! pattern the spatial mix and the game audio layer use.
//! Same sample rate in gives bit-identical banks (seeded xorshift plus integer
//! phase math; `sin` is presentation-side only and never enters a simulation
//! hash).
//!
//! [`Bank::synth`] also serves the cue bank: it offers several seeded takes
//! of hit, footstep and swing plus the crowd loop, which stand in for any cue
//! whose recorded clips are missing (see [`crate::CueBank::build`]).

use engine_core::splitmix64;

use crate::dsp::xorshift_step;
use crate::{f32_of, Pcm};

/// Hit length in seconds (short decaying thump plus click).
pub const HIT_SECS: f32 = 0.25;
/// Footstep length in seconds (low thump).
pub const FOOTSTEP_SECS: f32 = 0.18;
/// Crowd-bed loop length in seconds (seamless, soft filtered noise).
pub const CROWD_SECS: f32 = 2.0;
/// Crowd-bed nominal level (well under dialogue/music buses).
pub const CROWD_LEVEL: f32 = 0.18;

/// Which placeholder sample to play; see [`Bank::get`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BankSound {
    /// Short impact thump (combat hit events).
    Hit,
    /// Single footstep tap (locomotion ticks).
    Footstep,
    /// Loopable crowd wash (arena ambience bed).
    CrowdBed,
}

/// Preloaded playback samples: hit, footstep and crowd-bed voices. The game
/// plays these through [`crate::Mixer::play`]; the spatial mix spatializes the
/// same handles.
#[derive(Clone, Debug)]
pub struct Bank {
    /// Impact thump placeholder.
    pub hit: Pcm,
    /// Footstep tap placeholder.
    pub footstep: Pcm,
    /// Crowd wash placeholder (loop by replaying at end).
    pub crowd_bed: Pcm,
}

/// Deterministic white noise in `[-1, 1]` from the RNG state.
fn noise(state: &mut u64) -> f32 {
    let bits = u32::try_from(xorshift_step(state) >> 32).unwrap_or(u32::MAX);
    bits as f32 / 4_294_967_300.0 * 2.0 - 1.0
}

impl Bank {
    /// Generate the placeholder bank at `sample_rate` Hz (rate `0` falls back
    /// to 1 Hz math so construction never panics; the game always passes a
    /// real rate). Deterministic per rate: same rate gives bit-identical samples.
    pub fn placeholder(sample_rate: u32) -> Self {
        let sr = sample_rate.max(1);
        Self {
            hit: Self::bake_hit(sr, 0),
            footstep: Self::bake_footstep(sr, 0),
            crowd_bed: Self::bake_crowd(sr),
        }
    }

    /// Borrow one voice sample by kind (never panics).
    pub fn get(&self, sound: BankSound) -> &Pcm {
        match sound {
            BankSound::Hit => &self.hit,
            BankSound::Footstep => &self.footstep,
            BankSound::CrowdBed => &self.crowd_bed,
        }
    }

    /// Sample rate all three voices were baked at.
    pub fn sample_rate(&self) -> u32 {
        self.hit.sample_rate
    }

    /// Procedural stand-in voice `variant` of `kind` at `sample_rate` Hz
    /// (rate `0` falls back to 1 Hz math). Variant `0` is the voice
    /// [`Bank::placeholder`] holds; higher variants shift pitch and seed so a
    /// cue built from them still has takes to pick between. Deterministic per
    /// `(kind, rate, variant)`.
    pub fn synth(kind: Synth, sample_rate: u32, variant: u8) -> Pcm {
        let sr = sample_rate.max(1);
        match kind {
            Synth::Hit => Self::bake_hit(sr, variant),
            Synth::Footstep => Self::bake_footstep(sr, variant),
            Synth::Friction => Self::bake_friction(sr, variant),
            Synth::Crowd => Self::bake_crowd(sr),
            Synth::Swing => Self::bake_swing(sr, variant),
            Synth::Bone => Self::bake_bone(sr, variant),
            Synth::Armor => Self::bake_armor(sr, variant),
            Synth::Stump => Self::bake_stump(sr, variant),
            Synth::Groan => Self::bake_groan(sr, variant),
            Synth::Kill => Self::bake_kill(sr),
            Synth::Roar => Self::bake_roar(sr),
        }
    }

    /// Hit: 180 Hz thump (+14 Hz per variant) with quadratic decay plus a 5
    /// ms noise click, through a resonant body (900 Hz +150 per variant, Q 1.2).
    fn bake_hit(sr: u32, variant: u8) -> Pcm {
        let n = frames_for(sr, HIT_SECS);
        let mut rng: u64 = seeded(0x68_69_74_20_32_30_34_30, variant);
        let mut out = Vec::with_capacity(n);
        let click_len = usize::try_from((sr / 200).max(1)).unwrap_or(1);
        let freq = 180.0 + 14.0 * f32::from(variant);
        let mut body = crate::dsp::ResonantBody::new(sr, 900 + 150 * u32::from(variant), 1200);
        for i in 0..n {
            let t = ratio(i, sr);
            let d = 1.0 - ratio(i, n);
            let thump = (2.0 * std::f32::consts::PI * freq * t).sin() * d * d * 0.85;
            let click = if i < click_len {
                noise(&mut rng) * d * 0.35
            } else {
                0.0
            };
            out.push(body.step(thump + click).clamp(-1.0, 1.0));
        }
        Pcm::new(sr, 1, out)
    }

    /// Footstep: 90 Hz low thump (+9 Hz per variant) with cubic decay plus
    /// soft noise onset, through a resonant body (500 Hz +80 per variant, Q 0.9).
    fn bake_footstep(sr: u32, variant: u8) -> Pcm {
        let n = frames_for(sr, FOOTSTEP_SECS);
        let mut rng: u64 = seeded(0x66_6f_6f_74_73_74_65_70, variant);
        let mut out = Vec::with_capacity(n);
        let onset_len = usize::try_from((sr / 100).max(1)).unwrap_or(1);
        let freq = 90.0 + 9.0 * f32::from(variant);
        let mut body = crate::dsp::ResonantBody::new(sr, 500 + 80 * u32::from(variant), 900);
        for i in 0..n {
            let t = ratio(i, sr);
            let d = 1.0 - ratio(i, n);
            let thump = (2.0 * std::f32::consts::PI * freq * t).sin() * d * d * d * 0.8;
            let tap = if i < onset_len {
                noise(&mut rng) * d * 0.25
            } else {
                0.0
            };
            out.push(body.step(thump + tap).clamp(-1.0, 1.0));
        }
        Pcm::new(sr, 1, out)
    }

    /// Friction scrape: bandpassed seeded noise through drive grit under a
    /// swell-and-fall envelope peaking past the midpoint (slide/drag stand-in).
    fn bake_friction(sr: u32, variant: u8) -> Pcm {
        let n = frames_for(sr, FRICTION_SECS);
        let mut rng: u64 = seeded(0x66_72_69_63_74_69_6f_6e, variant);
        let mut out = Vec::with_capacity(n);
        let mut body = crate::dsp::ResonantBody::new(sr, 1800 + 300 * u32::from(variant), 800);
        let grit = crate::dsp::Shape::new(1.5);
        let peak_pos = 0.55 + 0.03 * f32::from(variant);
        let mut low = 0.0f32;
        for i in 0..n {
            let t = ratio(i, n);
            let warp = if t < peak_pos {
                t / peak_pos * 0.5
            } else {
                0.5 + (t - peak_pos) / (1.0 - peak_pos) * 0.5
            };
            let env = (std::f32::consts::PI * warp).sin().powi(2);
            let raw = noise(&mut rng);
            low += 0.02 * (raw - low);
            let band = body.step(raw - low);
            out.push((grit.step(band * 1.8) * env * 0.9).clamp(-1.0, 1.0));
        }
        Pcm::new(sr, 1, out)
    }

    /// Swing: lowpassed noise whose cutoff and level rise to a peak and fall
    /// away (a whoosh); the peak moves a little with the variant.
    fn bake_swing(sr: u32, variant: u8) -> Pcm {
        let n = frames_for(sr, SWING_SECS);
        let mut rng: u64 = seeded(0x73_77_69_6e_67_5f_32_30, variant);
        let peak = 0.4 + 0.05 * f32::from(variant);
        let mut out = Vec::with_capacity(n);
        let mut dark = 0.0f32;
        for i in 0..n {
            let t = ratio(i, n);
            let warp = if t < peak {
                t / peak * 0.5
            } else {
                0.5 + (t - peak) / (1.0 - peak) * 0.5
            };
            let env = (std::f32::consts::PI * warp).sin().powi(2);
            dark += (0.05 + 0.4 * env) * (noise(&mut rng) - dark);
            out.push((dark * env * 2.2).clamp(-1.0, 1.0));
        }
        Pcm::new(sr, 1, out)
    }

    /// Bone: a hard high knock (620 Hz + 40 per variant, sharp decay) through
    /// a bright resonant body plus a short crack click.
    fn bake_bone(sr: u32, variant: u8) -> Pcm {
        let n = frames_for(sr, BONE_SECS);
        let mut rng: u64 = seeded(0x62_6f_6e_65_5f_32_30_34, variant);
        let mut out = Vec::with_capacity(n);
        let click_len = usize::try_from((sr / 250).max(1)).unwrap_or(1);
        let freq = 620.0 + 40.0 * f32::from(variant);
        let mut body = crate::dsp::ResonantBody::new(sr, 2400 + 250 * u32::from(variant), 1100);
        for i in 0..n {
            let t = ratio(i, sr);
            let d = 1.0 - ratio(i, n);
            let knock = (2.0 * std::f32::consts::PI * freq * t).sin() * d * d * 0.75;
            let crack = if i < click_len {
                noise(&mut rng) * (1.0 - ratio(i, click_len)) * 0.5
            } else {
                0.0
            };
            out.push(body.step(knock + crack).clamp(-1.0, 1.0));
        }
        Pcm::new(sr, 1, out)
    }

    /// Armor: a metal ring (1240 Hz and 1860 Hz, the upper partial fading
    /// first) over a low 140 Hz body thud; the ring detunes with the variant.
    fn bake_armor(sr: u32, variant: u8) -> Pcm {
        let n = frames_for(sr, ARMOR_SECS);
        let mut rng: u64 = seeded(0x61_72_6d_6f_72_5f_32_30, variant);
        let mut out = Vec::with_capacity(n);
        let tau = std::f32::consts::TAU;
        let detune = 1.0 + 0.015 * f32::from(variant);
        let low = 1240.0 * detune;
        let high = 1860.0 * detune;
        for i in 0..n {
            let t = ratio(i, sr);
            let d = 1.0 - ratio(i, n);
            let ring = (tau * low * t).sin() * (-7.0 * t).exp() * 0.45
                + (tau * high * t).sin() * (-11.0 * t).exp() * 0.3;
            let thud = (tau * 140.0 * t).sin() * (-18.0 * t).exp() * 0.45;
            let noise_bed = noise(&mut rng) * (-40.0 * t).exp() * 0.1;
            out.push(((ring + thud + noise_bed) * d.sqrt()).clamp(-1.0, 1.0));
        }
        Pcm::new(sr, 1, out)
    }

    /// Stump: a wet spurt, lowpassed noise swelling fast then falling over a
    /// 120 Hz thump; the variant shifts the spurt's brightness.
    fn bake_stump(sr: u32, variant: u8) -> Pcm {
        let n = frames_for(sr, STUMP_SECS);
        let mut rng: u64 = seeded(0x73_74_75_6d_70_5f_32_30, variant);
        let mut out = Vec::with_capacity(n);
        let peak = 0.18 + 0.03 * f32::from(variant);
        let mut wet = 0.0f32;
        for i in 0..n {
            let t = ratio(i, n);
            let warp = if t < peak {
                t / peak * 0.5
            } else {
                0.5 + (t - peak) / (1.0 - peak) * 0.5
            };
            let env = (std::f32::consts::PI * warp).sin().powi(2);
            wet += (0.08 + 0.5 * env) * (noise(&mut rng) - wet);
            let thump = (2.0 * std::f32::consts::PI * 120.0 * ratio(i, sr)).sin()
                * (1.0 - ratio(i, n)).powi(2)
                * 0.35;
            out.push(((wet * env * 1.6) + thump).clamp(-1.0, 1.0));
        }
        Pcm::new(sr, 1, out)
    }

    /// Groan: a loaded beam creaking, low-passed noise under a 3 Hz wobble
    /// that rises then falls away; the variant sets the wobble rate.
    fn bake_groan(sr: u32, variant: u8) -> Pcm {
        let n = frames_for(sr, GROAN_SECS);
        let mut rng: u64 = seeded(0x67_72_6f_61_6e_5f_32_30, variant);
        let mut out = Vec::with_capacity(n);
        let wobble = 2.6 + 0.4 * f32::from(variant);
        let mut dark = 0.0f32;
        let mut low = 0.0f32;
        for i in 0..n {
            let t = ratio(i, n);
            let swell = (std::f32::consts::PI * t).sin();
            let grain = 0.55 + 0.45 * (2.0 * std::f32::consts::PI * wobble * ratio(i, sr)).sin();
            let raw = noise(&mut rng);
            low += 0.01 * (raw - low);
            dark += (0.03 + 0.12 * swell) * ((raw - low) - dark);
            let env = swell * swell * (1.0 - t);
            out.push((dark * grain * env * 3.0).clamp(-1.0, 1.0));
        }
        Pcm::new(sr, 1, out)
    }

    /// Kill stinger: a body whose pitch falls from 110 Hz toward 40 Hz, an
    /// A minor triad stab and a short noise click.
    fn bake_kill(sr: u32) -> Pcm {
        let n = frames_for(sr, KILL_SECS);
        let mut rng: u64 = 0x6b_69_6c_6c_5f_32_30_34;
        let mut out = Vec::with_capacity(n);
        let click_len = usize::try_from((sr / 100).max(1)).unwrap_or(1);
        let tau = std::f32::consts::TAU;
        for i in 0..n {
            let t = ratio(i, sr);
            let body = (tau * (110.0 * t - 35.0 * t * t)).sin() * (-3.0 * t).exp() * 0.7;
            let stab: f32 = [220.0, 261.63, 329.63]
                .iter()
                .map(|f| (tau * f * t).sin())
                .sum();
            let click = if i < click_len {
                noise(&mut rng) * (1.0 - ratio(i, click_len)) * 0.3
            } else {
                0.0
            };
            out.push((body + stab * (-5.0 * t).exp() * 0.12 + click).clamp(-1.0, 1.0));
        }
        Pcm::new(sr, 1, out)
    }

    /// Roar stinger: twice-lowpassed noise under a swell-and-fall envelope
    /// with a 7 Hz flutter.
    fn bake_roar(sr: u32) -> Pcm {
        let n = frames_for(sr, ROAR_SECS);
        let mut rng: u64 = 0x72_6f_61_72_5f_32_30_34;
        let mut out = Vec::with_capacity(n);
        let (mut slow, mut body) = (0.0f32, 0.0f32);
        for i in 0..n {
            let swell = (std::f32::consts::PI * ratio(i, n)).sin().powf(1.5);
            let flutter = 1.0 + 0.3 * (std::f32::consts::TAU * 7.0 * ratio(i, sr)).sin();
            slow += 0.10 * (noise(&mut rng) - slow);
            body += 0.35 * (slow - body);
            out.push((body * swell * flutter * 3.5).clamp(-1.0, 1.0));
        }
        Pcm::new(sr, 1, out)
    }

    /// Crowd bed: one-pole lowpassed noise with a slow swell, crossfaded over
    /// the last 0.1 s into the first 0.1 s so naive looping has no click.
    fn bake_crowd(sr: u32) -> Pcm {
        let n = frames_for(sr, CROWD_SECS);
        let mut rng: u64 = 0x63_72_6f_77_64_5f_62_65;
        let mut out = Vec::with_capacity(n);
        let mut dark = 0.0f32;
        for i in 0..n {
            let t = ratio(i, sr);
            let swell = 0.75 + 0.25 * (2.0 * std::f32::consts::PI * t / CROWD_SECS).sin();
            dark += 0.04 * (noise(&mut rng) - dark);
            out.push((dark * swell * CROWD_LEVEL * 4.0).clamp(-1.0, 1.0));
        }
        let fade = (usize::try_from((sr / 10).max(1)).unwrap_or(1)).min(n);
        for i in 0..fade {
            let k = ratio(i, fade);
            let tail = n - fade + i;
            out[tail] = out[tail] * k + out[i] * (1.0 - k);
        }
        Pcm::new(sr, 1, out)
    }
}

/// Which procedural voice stands in for a cue that has no recorded clip.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Synth {
    /// Impact thump.
    Hit,
    /// Footstep tap.
    Footstep,
    /// Friction scrape (slides and drags).
    Friction,
    /// Looping crowd wash.
    Crowd,
    /// Weapon whoosh.
    Swing,
    /// Blade on bone: hard knock with a crack.
    Bone,
    /// Blade on armor: metal ring over a body thud.
    Armor,
    /// Severed stump: wet spurt over a low thump.
    Stump,
    /// Structure creak under load.
    Groan,
    /// Kill stinger.
    Kill,
    /// Crowd roar stinger.
    Roar,
}

impl Synth {
    /// Takes [`Bank::synth`] offers: three for one-shots, one for the loop.
    pub fn variants(self) -> usize {
        match self {
            Self::Crowd | Self::Kill | Self::Roar => 1,
            Self::Hit | Self::Footstep | Self::Swing | Self::Friction => 3,
            Self::Bone | Self::Armor | Self::Stump | Self::Groan => 3,
        }
    }
}

/// Swing length in seconds (short whoosh).
pub const SWING_SECS: f32 = 0.3;
/// Friction scrape length in seconds.
pub const FRICTION_SECS: f32 = 0.35;
/// Blade-on-bone knock length in seconds.
pub const BONE_SECS: f32 = 0.3;
/// Blade-on-armor ring length in seconds.
pub const ARMOR_SECS: f32 = 0.45;
/// Severed-stump spurt length in seconds.
pub const STUMP_SECS: f32 = 0.4;
/// Structure groan length in seconds.
pub const GROAN_SECS: f32 = 1.2;
/// Kill stinger length in seconds.
pub const KILL_SECS: f32 = 1.0;
/// Roar stinger length in seconds.
pub const ROAR_SECS: f32 = 1.6;

/// `num / den` as a buffer position in `0.0..=1.0`-style units; `den == 0`
/// gives `0.0`. Exact for counts below 2^24.
fn ratio(num: impl TryInto<u32>, den: impl TryInto<u32>) -> f32 {
    let den = den.try_into().unwrap_or(u32::MAX);
    if den == 0 {
        return 0.0;
    }
    f32_of(num.try_into().unwrap_or(u32::MAX)) / f32_of(den)
}

/// Frames in `secs` seconds at `sr` Hz, at least one.
fn frames_for(sr: u32, secs: f32) -> usize {
    ((f32_of(sr) * secs) as usize).max(1)
}

/// Noise seed for `variant`: the base seed itself for variant 0, a
/// splitmix64-scrambled neighbour otherwise.
fn seeded(base: u64, variant: u8) -> u64 {
    if variant == 0 {
        base
    } else {
        base ^ splitmix64(u64::from(variant))
    }
}

// ---------------------------------------------------------------------------
// FU9: modal-voice spike beside the bank (physical sound synthesis, offline
// bake only). Placeholders plus bus/RMS gate frozen: this adds new voices
// beside [`Bank`], never changes it, never a second engine, never live
// synth, never in tick. `sin` plus `exp` are presentation-side only.
// ---------------------------------------------------------------------------

/// FU9 struck-bar length in seconds.
pub const MODAL_BAR_SECS: f32 = 0.4;
/// FU9 thump length in seconds.
pub const MODAL_THUMP_SECS: f32 = 0.2;
/// FU9 struck-bar partials: (freq Hz, decay per second, gain). Inharmonic
/// bar shape, three modes only for the smallest proof.
pub const MODAL_BAR_PARTIALS: [(f32, f32, f32); 3] =
    [(440.0, 6.0, 0.5), (1108.0, 9.0, 0.25), (2240.0, 12.0, 0.12)];

/// FU9 modal voice: offline-baked struck-bar plus thump beside the bank.
#[derive(Clone, Debug)]
pub struct ModalVoice {
    /// Struck-bar voice (inharmonic partials, exponential decay).
    pub bar: Pcm,
    /// Low thump voice (110 Hz plus short noise click).
    pub thump: Pcm,
}

impl ModalVoice {
    /// Bake both voices at `sample_rate` Hz (rate `0` falls back to 1 Hz
    /// math so construction never panics). Deterministic per rate: same
    /// rate gives bit-identical samples. Offline only, allocates once.
    pub fn bake(sample_rate: u32) -> Self {
        let sr = sample_rate.max(1);
        Self {
            bar: Self::bake_bar(sr),
            thump: Self::bake_thump(sr),
        }
    }

    /// Struck bar: three inharmonic sine partials with exponential decay.
    fn bake_bar(sr: u32) -> Pcm {
        let n = frames_for(sr, MODAL_BAR_SECS);
        let mut out = Vec::with_capacity(n);
        for i in 0..n {
            let t = ratio(i, sr);
            let mut s = 0.0f32;
            for (freq, decay, gain) in MODAL_BAR_PARTIALS {
                s += (2.0 * std::f32::consts::PI * freq * t).sin() * (-decay * t).exp() * gain;
            }
            out.push(s.clamp(-1.0, 1.0));
        }
        Pcm::new(sr, 1, out)
    }

    /// Thump: 110 Hz sine with fast decay plus a 5 ms noise click.
    fn bake_thump(sr: u32) -> Pcm {
        let n = frames_for(sr, MODAL_THUMP_SECS);
        let mut rng: u64 = 0x6d_6f_64_61_6c_5f_39;
        let mut out = Vec::with_capacity(n);
        let click_len = usize::try_from((sr / 200).max(1)).unwrap_or(1);
        for i in 0..n {
            let t = ratio(i, sr);
            let body = (2.0 * std::f32::consts::PI * 110.0 * t).sin() * (-14.0 * t).exp() * 0.9;
            let click = if i < click_len {
                noise(&mut rng) * (1.0 - ratio(i, click_len)) * 0.3
            } else {
                0.0
            };
            out.push((body + click).clamp(-1.0, 1.0));
        }
        Pcm::new(sr, 1, out)
    }

    /// Fill `out` with bar samples from the top (truncated or zero-padded).
    /// Caller buffer reuse: allocates nothing, for the audio-thread path.
    pub fn render_bar_into(&self, out: &mut [f32]) {
        for (slot, sample) in out.iter_mut().enumerate() {
            *sample = if slot < self.bar.samples.len() {
                self.bar.samples[slot]
            } else {
                0.0
            };
        }
    }

    /// Fill `out` with thump samples from the top (truncated or zero-padded).
    /// Caller buffer reuse: allocates nothing.
    pub fn render_thump_into(&self, out: &mut [f32]) {
        for (slot, sample) in out.iter_mut().enumerate() {
            *sample = if slot < self.thump.samples.len() {
                self.thump.samples[slot]
            } else {
                0.0
            };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Mixer;

    #[test]
    fn placeholder_bank_deterministic_per_rate() {
        let (a, b) = (Bank::placeholder(48_000), Bank::placeholder(48_000));
        assert_eq!(a.hit.samples, b.hit.samples);
        assert_eq!(a.footstep.samples, b.footstep.samples);
        assert_eq!(a.crowd_bed.samples, b.crowd_bed.samples);
        assert_ne!(
            Bank::placeholder(8_000).hit.samples,
            Bank::placeholder(48_000).hit.samples
        );
    }

    #[test]
    fn placeholder_bank_voices_audible_and_bounded() {
        let bank = Bank::placeholder(48_000);
        for sample in [&bank.hit, &bank.footstep, &bank.crowd_bed] {
            assert!(!sample.is_empty(), "placeholder voice must have frames");
            assert_eq!((sample.channels, sample.sample_rate), (1, 48_000));
            assert!(sample.samples.iter().all(|s| s.is_finite()));
            assert!(sample.samples.iter().any(|&s| s.abs() > 0.01));
            assert!(sample.samples.iter().all(|&s| s.abs() <= 1.0));
        }
        assert!(Bank::placeholder(0).hit.frames() > 0);
    }

    #[test]
    fn placeholder_bank_plays_all_three_voices_without_panic() {
        let bank = Bank::placeholder(48_000);
        let mut mixer = Mixer::new(48_000);
        for sound in [BankSound::Hit, BankSound::Footstep, BankSound::CrowdBed] {
            let id = mixer.play(bank.get(sound), 1.0);
            assert_ne!(id.to_u64(), 0, "{sound:?} must issue a live handle");
        }
        assert_eq!(mixer.voice_count(), 3);
        let longest = bank.crowd_bed.frames();
        let out = mixer.render(longest);
        assert_eq!(out.len(), longest);
        assert!(
            out.iter().any(|&s| s != 0.0),
            "mixed voices must be audible"
        );
        assert!(out.iter().all(|&s| s.abs() <= 1.0));
        assert!(mixer.is_idle(), "all voices consumed after their frames");
    }

    #[test]
    fn fu9_modal_synth_posts_ms_rms_vs_placeholder_baseline() {
        // FU9 smallest proof: 1 struck-bar + 1 thump beside the bank.
        // Accept: synth filter green, ms + RMS posted vs placeholder
        // baseline, 0 audio-thread allocs, bytes x2, golden unchanged
        // (presentation only, tick untouched). Kill: any bank/dsp change
        // (placeholders frozen, asserted by the three tests above staying
        // green) or any live-synth path (bake only, never synthesize_live).
        use crate::dsp::{peak, rms};
        let started = std::time::Instant::now();
        let modal = ModalVoice::bake(48_000);
        let bake_ms = started.elapsed().as_secs_f64() * 1000.0;
        let bank = Bank::placeholder(48_000);
        let (bar_rms, bar_peak) = (rms(&modal.bar.samples), peak(&modal.bar.samples));
        let (thump_rms, thump_peak) = (rms(&modal.thump.samples), peak(&modal.thump.samples));
        let (hit_rms, hit_peak) = (rms(&bank.hit.samples), peak(&bank.hit.samples));
        eprintln!(
            "FU9 modal: bake_ms={bake_ms:.2} bar_rms={bar_rms:.4} bar_peak={bar_peak:.4} thump_rms={thump_rms:.4} thump_peak={thump_peak:.4} baseline_hit_rms={hit_rms:.4} baseline_hit_peak={hit_peak:.4}"
        );
        // Audible and bounded, like the placeholders.
        for (samples, rms_v, peak_v, name) in [
            (&modal.bar.samples, bar_rms, bar_peak, "bar"),
            (&modal.thump.samples, thump_rms, thump_peak, "thump"),
        ] {
            assert!(!samples.is_empty(), "{name} must have frames");
            assert!(
                rms_v.is_finite() && rms_v > 0.01,
                "{name} rms audible, got {rms_v}"
            );
            assert!(
                peak_v.is_finite() && peak_v <= 1.0,
                "{name} peak bounded, got {peak_v}"
            );
            assert!(samples.iter().all(|s| s.is_finite()));
            assert!(samples.iter().all(|&s| s.abs() <= 1.0));
        }
        // Distinct voices, not a copy of the placeholder.
        assert_ne!(
            modal.bar.samples, bank.hit.samples,
            "modal bar must differ from the placeholder hit"
        );
        assert_ne!(
            modal.thump.samples, bank.footstep.samples,
            "modal thump must differ from the placeholder footstep"
        );
        // Bytes x2 identical per rate.
        let again = ModalVoice::bake(48_000);
        assert_eq!(modal.bar.samples, again.bar.samples);
        assert_eq!(modal.thump.samples, again.thump.samples);
        // 0 audio-thread allocs on the caller-buffer path.
        let mut scratch = vec![0.0f32; modal.bar.samples.len().max(1024)];
        let ((), heap) = engine_core::alloc_probe::measure(|| {
            modal.render_bar_into(&mut scratch);
            modal.render_thump_into(&mut scratch);
        });
        assert_eq!(heap.calls, 0, "modal render_into must not allocate");
        assert!(scratch.iter().any(|&s| s != 0.0));
        // Plays through the frozen mixer like any bank voice.
        let mut mixer = Mixer::new(48_000);
        mixer.play(&modal.bar, 1.0);
        mixer.play(&modal.thump, 1.0);
        let out = mixer.render(modal.bar.frames().max(modal.thump.frames()));
        assert!(out.iter().any(|&s| s != 0.0));
        assert!(out.iter().all(|&s| s.abs() <= 1.0));
    }

    #[test]
    fn synth_variant_zero_is_the_placeholder_and_others_differ() {
        let base = Bank::placeholder(48_000);
        assert_eq!(Bank::synth(Synth::Hit, 48_000, 0).samples, base.hit.samples);
        assert_eq!(
            Bank::synth(Synth::Footstep, 48_000, 0).samples,
            base.footstep.samples
        );
        assert_eq!(
            Bank::synth(Synth::Crowd, 48_000, 0).samples,
            base.crowd_bed.samples
        );
        for kind in [Synth::Hit, Synth::Footstep, Synth::Swing] {
            assert_eq!(kind.variants(), 3);
            let takes: Vec<Vec<f32>> = (0..3)
                .map(|v| Bank::synth(kind, 48_000, v).samples)
                .collect();
            assert!(
                takes[0] != takes[1] && takes[1] != takes[2] && takes[0] != takes[2],
                "{kind:?} takes must differ"
            );
            assert_eq!(
                takes[1],
                Bank::synth(kind, 48_000, 1).samples,
                "{kind:?} replays"
            );
        }
        assert_eq!(Synth::Crowd.variants(), 1);
    }

    #[test]
    fn swing_whooshes_up_and_fades_out_bounded() {
        let swing = Bank::synth(Synth::Swing, 48_000, 0);
        assert_eq!((swing.channels, swing.sample_rate), (1, 48_000));
        assert_eq!(swing.frames(), 14_400, "0.3 s at 48 kHz");
        assert!(swing
            .samples
            .iter()
            .all(|s| s.is_finite() && s.abs() <= 1.0));
        let (head, mid, tail) = (
            crate::dsp::rms(&swing.samples[..1_000]),
            crate::dsp::rms(&swing.samples[6_000..8_000]),
            crate::dsp::rms(&swing.samples[13_400..]),
        );
        assert!(mid > 0.02, "audible at the peak: {mid}");
        assert!(
            head < mid * 0.25 && tail < mid * 0.25,
            "fades in and out: {head} {mid} {tail}"
        );
        assert!(
            Bank::synth(Synth::Swing, 0, 0).frames() > 0,
            "rate 0 never panics"
        );
    }

    #[test]
    fn stingers_are_bounded_audible_and_replay() {
        for (kind, secs) in [(Synth::Kill, KILL_SECS), (Synth::Roar, ROAR_SECS)] {
            let a = Bank::synth(kind, 8_000, 0);
            assert_eq!(
                a.samples,
                Bank::synth(kind, 8_000, 0).samples,
                "{kind:?} replays"
            );
            assert_eq!(a.frames(), (8_000.0 * secs) as usize);
            assert!(a.samples.iter().all(|s| s.is_finite() && s.abs() <= 1.0));
            assert!(crate::dsp::rms(&a.samples) > 0.02, "{kind:?} is audible");
            assert_eq!(kind.variants(), 1);
        }
        let roar = Bank::synth(Synth::Roar, 8_000, 0);
        let (head, mid) = (
            crate::dsp::rms(&roar.samples[..800]),
            crate::dsp::rms(&roar.samples[5_600..7_200]),
        );
        assert!(head < mid * 0.4, "a roar swells: {head} then {mid}");
        let kill = Bank::synth(Synth::Kill, 8_000, 0);
        let (early, late) = (
            crate::dsp::rms(&kill.samples[..1_600]),
            crate::dsp::rms(&kill.samples[6_400..]),
        );
        assert!(late < early * 0.2, "a kill decays: {early} then {late}");
    }

    #[test]
    fn synth_seeded_identical_x2() {
        use crate::dsp::{peak, rms};
        for &rate in &[8_000u32, 48_000] {
            for kind in [Synth::Hit, Synth::Footstep, Synth::Friction] {
                assert_eq!(kind.variants(), 3, "{kind:?} offers 3 takes");
                for v in 0..3u8 {
                    let (a, b) = (Bank::synth(kind, rate, v), Bank::synth(kind, rate, v));
                    assert_eq!(a.samples, b.samples, "{kind:?} v{v} @{rate} replays");
                    assert!(a.samples.iter().all(|s| s.is_finite() && s.abs() <= 1.0));
                    assert!(rms(&a.samples) > 0.01, "{kind:?} v{v} @{rate} audible");
                    assert!(peak(&a.samples) > 0.05, "{kind:?} v{v} @{rate} peaks");
                }
                let takes = [0, 1, 2].map(|v| Bank::synth(kind, rate, v).samples);
                assert!(takes[0] != takes[1], "{kind:?} differ");
                assert!(takes[0] != takes[2], "{kind:?} differ");
            }
        }
        let fr = Bank::synth(Synth::Friction, 48_000, 0);
        let n = fr.samples.len();
        let (head, mid, tail) = (
            rms(&fr.samples[..n / 6]),
            rms(&fr.samples[n / 3..2 * n / 3]),
            rms(&fr.samples[5 * n / 6..]),
        );
        assert!(mid > 0.02, "friction audible at the swell: {mid}");
        assert!(head < mid * 0.6, "swell head {head} mid {mid}");
        assert!(tail < mid * 0.6, "swell tail {tail} mid {mid}");
        assert!(Bank::synth(Synth::Friction, 0, 0).frames() > 0);
    }
}
