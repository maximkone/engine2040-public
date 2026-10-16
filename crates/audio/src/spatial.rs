//! spatial: headless positional mix over [`Mixer`](crate::Mixer).
//!
//! The game audio layer sets the listener (the camera) with
//! [`SpatialMixer::set_listener_3d`] each frame and plays positioned cues
//! with [`SpatialMixer::play_cue`]. Units: positions are integer
//! centimeters (`x` east, `y` up, `z` south), yaw is integer degrees
//! clockwise from north (`-z`). A cue's gain is 1 inside its falloff min,
//! falls as `min / distance` out to its falloff max, then is 0 (Gregory,
//! Game Engine Architecture 13.4.3.1-13.4.3.2); the lateral offset in the
//! listener frame sets the pan. Determinism scope: same listener plus same
//! play order gives bit-identical stereo bytes and an identical [`MixCmd`]
//! log; the log is what the game replays for proof. Math is integer-friendly
//! only (`+ - * /`, comparisons, [`isqrt`], a fixed sine table): no
//! `sin`/`cos`/`exp`/`powf`, so every platform renders the same placement.
//! The legacy [`SpatialMixer::play_at`] keeps its 2D linear falloff.

use crate::cue::{Cue, CueId, Spatial};
use crate::{f32_of, sanitize_gain, Mixer, MixerStats, Pcm, VoiceId, VoiceRoute};

/// Full gain inside this radius (1 m): closer sources never get louder.
pub const NEAR_CM: u32 = 100;
/// Silence at and beyond this radius (30 m): the arena plus stands.
pub const FAR_CM: u32 = 3_000;
/// Lateral offset that pans fully to one side (10 m at yaw 0).
pub const PAN_FULL_CM: i32 = 1_000;

/// `sin(d degrees) * 16384` for `d` in `0..=90`, rounded. Other quadrants
/// mirror this table, so yaw needs no transcendental call.
const SIN_Q14: [i16; 91] = [
    0, 286, 572, 857, 1143, 1428, 1713, 1997, 2280, 2563, 2845, 3126, 3406, 3686, 3964, 4240, 4516,
    4790, 5063, 5334, 5604, 5872, 6138, 6402, 6664, 6924, 7182, 7438, 7692, 7943, 8192, 8438, 8682,
    8923, 9162, 9397, 9630, 9860, 10087, 10311, 10531, 10749, 10963, 11174, 11381, 11585, 11786,
    11982, 12176, 12365, 12551, 12733, 12911, 13085, 13255, 13421, 13583, 13741, 13894, 14044,
    14189, 14330, 14466, 14598, 14726, 14849, 14968, 15082, 15191, 15296, 15396, 15491, 15582,
    15668, 15749, 15826, 15897, 15964, 16026, 16083, 16135, 16182, 16225, 16262, 16294, 16322,
    16344, 16362, 16374, 16382, 16384,
];
/// One in the Q14 sine table.
const TRIG_ONE: i64 = 16_384;

/// Floor-plane position in integer centimeters (`x` east, `z` south).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Pos2 {
    /// East offset in centimeters.
    pub x_cm: i32,
    /// South offset in centimeters.
    pub z_cm: i32,
}

impl Pos2 {
    /// Build from centimeter offsets.
    pub fn new(x_cm: i32, z_cm: i32) -> Self {
        Self { x_cm, z_cm }
    }
}

/// Integer square root (floor): `isqrt(n)^2 <= n < (isqrt(n)+1)^2`.
/// Newton iteration from a power-of-two overestimate; deterministic on all
/// platforms, never panics (division is always by a nonzero `x`).
pub fn isqrt(n: u64) -> u64 {
    if n < 2 {
        return n;
    }
    // Overestimate: 2^ceil(bitlen / 2), so `x >= sqrt(n)` holds throughout.
    let mut x = 1u64 << (u64::BITS - n.leading_zeros()).div_ceil(2);
    loop {
        let y = (x + n / x) / 2;
        if y >= x {
            return x;
        }
        x = y;
    }
}

/// Distance attenuation `0.0..=1.0`: full gain inside [`NEAR_CM`], linear
/// falloff to silence at [`FAR_CM`]. Pure rational math on the centimeter
/// distance, so scripted distances assert exactly (midpoint gives `0.5`).
pub fn attenuation(dist_cm: u32) -> f32 {
    if dist_cm <= NEAR_CM {
        1.0
    } else if dist_cm >= FAR_CM {
        0.0
    } else {
        f32_of(FAR_CM - dist_cm) / f32_of(FAR_CM - NEAR_CM)
    }
}

/// Cue distance gain: `1.0` out to `min_cm`, then `min_cm / dist_cm` (the
/// amplitude law, 6 dB per doubling) out to `max_cm`, then `0.0`. The curve
/// is clamped at `max_cm` rather than shifted to reach zero there.
pub fn falloff_gain(dist_cm: u32, min_cm: u32, max_cm: u32) -> f32 {
    if dist_cm <= min_cm {
        1.0
    } else if dist_cm > max_cm {
        0.0
    } else {
        f32_of(min_cm) / f32_of(dist_cm)
    }
}

/// Stereo pan `-1.0` (hard left) to `1.0` (hard right) from the lateral
/// offset in listener space; `>= PAN_FULL_CM` pins to the rail.
pub fn pan_for(right_cm: i32) -> f32 {
    (right_cm as f32 / PAN_FULL_CM as f32).clamp(-1.0, 1.0)
}

/// Left and right gains for a pan position at overall level `eff`
/// (constant-sum linear pan: the far ear fades, the near ear holds).
fn pan_gains(eff: f32, pan: f32) -> (f32, f32) {
    (eff * (1.0 - pan.max(0.0)), eff * (1.0 - (-pan).max(0.0)))
}

/// `sin(deg) * 16384` for any integer degree.
fn sin_q14(deg: i32) -> i64 {
    let d = deg.rem_euclid(360);
    let at = |x: i32| i64::from(SIN_Q14[usize::try_from(x).unwrap_or(0)]);
    match d {
        0..=90 => at(d),
        91..=180 => at(180 - d),
        181..=270 => -at(d - 180),
        _ => -at(360 - d),
    }
}

/// Lateral offset (positive right of the listener's facing) of a source
/// `dx` east and `dz` south of a listener at `yaw_deg`.
fn right_cm(dx: i64, dz: i64, yaw_deg: i32) -> i64 {
    (dx * sin_q14(yaw_deg.saturating_add(90)) + dz * sin_q14(yaw_deg)) / TRIG_ONE
}

/// Radial distance in centimeters for per-axis offsets; the squares
/// saturate instead of wrapping on planet-sized coordinates.
fn dist_cm_of(delta: [i64; 3]) -> u32 {
    let square = |v: i64| v.unsigned_abs().saturating_mul(v.unsigned_abs());
    let dist2 = delta
        .iter()
        .fold(0u64, |acc, &v| acc.saturating_add(square(v)));
    u32::try_from(isqrt(dist2)).unwrap_or(u32::MAX)
}

/// One recorded play call, in issue order. The game replays this log for
/// proof: the same event script twice gives the same commands.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MixCmd {
    /// Voice handle issued by the inner mixer.
    pub id: VoiceId,
    /// Radial distance in centimeters (saturated to `u32::MAX`).
    pub dist_cm: u32,
    /// Distance attenuation applied (`0.0..=1.0`).
    pub atten: f32,
    /// Stereo pan applied (`-1.0..=1.0`).
    pub pan: f32,
    /// Final left-channel gain (already includes caller gain).
    pub left_gain: f32,
    /// Final right-channel gain (already includes caller gain).
    pub right_gain: f32,
}

/// Where a source sits relative to the listener.
#[derive(Clone, Copy, Debug)]
struct Placed {
    dist_cm: u32,
    atten: f32,
    pan: f32,
}

/// Positional mix over a [`Mixer`]: per-play distance attenuation plus
/// constant-sum linear pan. Legacy [`Self::play_at`] bakes both into one
/// stereo voice; [`Self::play_cue`] shares the clip and applies them as
/// per-ear voice gains. Headless-safe (std-only); the `device` feature only
/// changes how the bytes leave.
pub struct SpatialMixer {
    mixer: Mixer,
    listener: [i32; 3],
    yaw_deg: i32,
    log: Vec<MixCmd>,
    log_on: bool,
    culled: u64,
}

impl SpatialMixer {
    /// New mixer at `sample_rate` Hz, listener at the origin facing north,
    /// recording every play in [`Self::commands`].
    pub fn new(sample_rate: u32) -> Self {
        Self {
            mixer: Mixer::new(sample_rate),
            listener: [0; 3],
            yaw_deg: 0,
            log: Vec::new(),
            log_on: true,
            culled: 0,
        }
    }

    /// Like [`Self::new`] without the proof log, which grows with every
    /// play: the form the audio callback owns.
    pub fn live(sample_rate: u32) -> Self {
        Self {
            log_on: false,
            ..Self::new(sample_rate)
        }
    }

    /// Move the listener on the floor plane; the game calls this once per
    /// tick before `play_at`. Never panics.
    pub fn set_listener(&mut self, pos: Pos2, yaw_deg: i32) {
        self.set_listener_3d([pos.x_cm, 0, pos.z_cm], yaw_deg);
    }

    /// Move the listener in 3D (the camera): `pos_cm` is `[east, up, south]`
    /// centimeters, `yaw_deg` degrees clockwise from north. Never panics.
    pub fn set_listener_3d(&mut self, pos_cm: [i32; 3], yaw_deg: i32) {
        self.listener = pos_cm;
        self.yaw_deg = yaw_deg;
    }

    /// Current listener position on the floor plane.
    pub fn listener(&self) -> Pos2 {
        Pos2::new(self.listener[0], self.listener[2])
    }

    /// Current listener position, `[east, up, south]` centimeters.
    pub fn listener_3d(&self) -> [i32; 3] {
        self.listener
    }

    /// Current listener yaw in degrees (as passed to [`set_listener`](Self::set_listener)).
    pub fn yaw_deg(&self) -> i32 {
        self.yaw_deg
    }

    /// Distance and pan of `src` (`[east, up, south]` cm) against the
    /// listener; the height axis counts only when `with_height`.
    fn place(&self, src: [i32; 3], with_height: bool) -> (u32, f32) {
        let delta = |axis: usize| i64::from(src[axis]) - i64::from(self.listener[axis]);
        let (dx, dz) = (delta(0), delta(2));
        let dy = if with_height { delta(1) } else { 0 };
        let right = right_cm(dx, dz, self.yaw_deg);
        let right = i32::try_from(right).unwrap_or(if right < 0 { i32::MIN } else { i32::MAX });
        (dist_cm_of([dx, dy, dz]), pan_for(right))
    }

    /// Append to the proof log when it is on.
    fn record(&mut self, id: VoiceId, placed: Placed, gains: (f32, f32)) {
        if self.log_on {
            self.log.push(MixCmd {
                id,
                dist_cm: placed.dist_cm,
                atten: placed.atten,
                pan: placed.pan,
                left_gain: gains.0,
                right_gain: gains.1,
            });
        }
    }

    /// Play `sample` positioned at `pos` with caller `gain`: attenuate by
    /// radial distance, pan by lateral offset in the listener frame, fold
    /// to mono first when the source is stereo, and issue one stereo voice
    /// at gain `1.0` (spatial gains are baked in). Returns the [`VoiceId`]
    /// and appends a [`MixCmd`]. Never panics.
    pub fn play_at(&mut self, sample: &Pcm, pos: Pos2, gain: f32) -> VoiceId {
        let (dist_cm, pan) = self.place([pos.x_cm, 0, pos.z_cm], false);
        let atten = attenuation(dist_cm);
        let (left_gain, right_gain) = pan_gains(sanitize_gain(gain) * atten, pan);
        let frames = sample.frames();
        let mut stereo = Vec::with_capacity(frames.saturating_mul(2));
        for i in 0..frames {
            let mono = sample.mono_at(i);
            stereo.push(mono * left_gain);
            stereo.push(mono * right_gain);
        }
        let stereo = Pcm::new(sample.sample_rate, 2, stereo);
        let id = self.mixer.play(&stereo, 1.0);
        let placed = Placed {
            dist_cm,
            atten,
            pan,
        };
        self.record(id, placed, (left_gain, right_gain));
        id
    }

    /// Play variant `variant` of `cue` at level `gain` (times the cue's own
    /// trim). A positional cue given a position is attenuated by its falloff
    /// range over the 3D distance, panned, and mono-folded; it is skipped
    /// (counted in [`Self::culled`]) beyond its falloff max. A flat cue, or a
    /// positional one without a position, plays at the same level in both
    /// ears. `None` when the variant is missing, the source is out of range,
    /// or the mixer refused the voice (group limit, or nothing weaker to
    /// steal). Shares the clip's samples; never panics.
    pub fn play_cue(
        &mut self,
        id: CueId,
        cue: &Cue,
        variant: usize,
        gain: f32,
        pos_cm: Option<[i32; 3]>,
    ) -> Option<VoiceId> {
        let clip = cue.variants.get(variant)?;
        let level = sanitize_gain(gain * cue.gain);
        let route = VoiceRoute {
            cue: id,
            bus: cue.bus,
            group: cue.group,
            priority: cue.priority,
            looped: cue.looped,
            ducks_music: cue.ducks_music,
        };
        let (gains, fold, placed) = match (cue.spatial, pos_cm) {
            (
                Spatial::Positional {
                    falloff_min_cm,
                    falloff_max_cm,
                },
                Some(src),
            ) => {
                let (dist_cm, pan) = self.place(src, true);
                let atten = falloff_gain(dist_cm, falloff_min_cm, falloff_max_cm);
                if atten <= 0.0 {
                    self.culled += 1;
                    return None;
                }
                let placed = Placed {
                    dist_cm,
                    atten,
                    pan,
                };
                (pan_gains(level * atten, pan), true, placed)
            }
            _ => {
                let flat = Placed {
                    dist_cm: 0,
                    atten: 1.0,
                    pan: 0.0,
                };
                ((level, level), false, flat)
            }
        };
        let voice = self.mixer.play_clip(clip, route, gains, fold)?;
        self.record(voice, placed, gains);
        Some(voice)
    }

    /// Stop every voice of `cue`; returns how many stopped.
    pub fn stop_cue(&mut self, cue: CueId) -> usize {
        self.mixer.stop_cue(cue)
    }

    /// Cue plays skipped for being beyond their falloff max.
    pub fn culled(&self) -> u64 {
        self.culled
    }

    /// Voice counters of the inner mixer.
    pub fn mixer_stats(&self) -> MixerStats {
        self.mixer.stats()
    }

    /// The inner mixer (voice limits, duck level).
    pub fn mixer(&self) -> &Mixer {
        &self.mixer
    }

    /// The inner mixer, mutable: group limits.
    pub fn mixer_mut(&mut self) -> &mut Mixer {
        &mut self.mixer
    }

    /// Recorded mix commands in issue order (proof log).
    pub fn commands(&self) -> &[MixCmd] {
        &self.log
    }

    /// Live (unfinished) voice count in the inner mixer.
    pub fn voice_count(&self) -> usize {
        self.mixer.voice_count()
    }

    /// True when no voice has frames left to render.
    pub fn is_idle(&self) -> bool {
        self.mixer.is_idle()
    }

    /// Mix `n` stereo frames into a new interleaved buffer.
    pub fn render_stereo(&mut self, n: usize) -> Vec<f32> {
        self.mixer.render_stereo(n)
    }

    /// Mix interleaved stereo (`LRLR...`) in place; `out.len() / 2` frames.
    pub fn render_stereo_into(&mut self, out: &mut [f32]) {
        self.mixer.render_stereo_into(out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crowd::{Crowd, FightCue};

    fn mono_hit() -> Pcm {
        Pcm::new(8_000, 1, vec![0.5, 0.5, 0.5, 0.5])
    }

    #[test]
    fn isqrt_exact_floor() {
        assert_eq!((isqrt(0), isqrt(1), isqrt(4)), (0, 1, 2));
        assert_eq!((isqrt(8), isqrt(9), isqrt(15), isqrt(16)), (2, 3, 3, 4));
        assert_eq!(isqrt(3_000u64 * 3_000), 3_000);
        assert_eq!(isqrt(u64::MAX), 4_294_967_295);
        // Floor law on a sweep: r^2 <= n < (r+1)^2.
        for n in (0..50_000u64).step_by(997) {
            let r = isqrt(n);
            assert!(r * r <= n, "n={n} r={r}");
            assert!((r + 1) * (r + 1) > n, "n={n} r={r}");
        }
    }

    #[test]
    fn attenuation_exact_script() {
        assert_eq!(attenuation(0), 1.0);
        assert_eq!(attenuation(NEAR_CM), 1.0);
        assert!(attenuation(NEAR_CM + 1) > 0.99);
        assert_eq!(attenuation((NEAR_CM + FAR_CM) / 2), 0.5);
        assert_eq!(attenuation(FAR_CM), 0.0);
        assert_eq!(attenuation(FAR_CM + 1), 0.0);
        assert_eq!(attenuation(u32::MAX), 0.0);
    }

    #[test]
    fn pan_exact_script() {
        assert_eq!(pan_for(0), 0.0);
        assert_eq!(pan_for(PAN_FULL_CM), 1.0);
        assert_eq!(pan_for(-PAN_FULL_CM), -1.0);
        assert_eq!(pan_for(PAN_FULL_CM * 4), 1.0);
        assert_eq!(pan_for(-PAN_FULL_CM * 4), -1.0);
        assert_eq!(pan_for(PAN_FULL_CM / 2), 0.5);
        assert_eq!(pan_for(-PAN_FULL_CM / 2), -0.5);
    }

    #[test]
    fn center_near_is_dual_mono_at_caller_gain() {
        let sample = mono_hit();
        let mut m = SpatialMixer::new(8_000);
        m.set_listener(Pos2::new(0, 0), 0);
        let id = m.play_at(&sample, Pos2::new(0, 0), 1.0);
        assert_ne!(id.to_u64(), 0);
        let cmd = m.commands()[0];
        assert_eq!((cmd.dist_cm, cmd.atten, cmd.pan), (0, 1.0, 0.0));
        assert_eq!((cmd.left_gain, cmd.right_gain), (1.0, 1.0));
        assert_eq!(
            m.render_stereo(4),
            vec![0.5, 0.5, 0.5, 0.5, 0.5, 0.5, 0.5, 0.5]
        );
        assert!(m.is_idle());
    }

    #[test]
    fn left_right_hard_pan_on_scripted_positions() {
        let sample = mono_hit();
        let mut m = SpatialMixer::new(8_000);
        m.set_listener(Pos2::new(0, 0), 0);
        m.play_at(&sample, Pos2::new(-PAN_FULL_CM, 0), 1.0);
        m.play_at(&sample, Pos2::new(PAN_FULL_CM, 0), 1.0);
        let (left, right) = (m.commands()[0], m.commands()[1]);
        // Rail distance is past NEAR_CM, so both voices carry the same
        // falloff: (3000 - 1000) / (3000 - 100), panned to opposite rails.
        let rail = (FAR_CM - PAN_FULL_CM as u32) as f32 / (FAR_CM - NEAR_CM) as f32;
        assert_eq!(
            (left.pan, left.left_gain, left.right_gain),
            (-1.0, rail, 0.0)
        );
        assert_eq!(
            (right.pan, right.left_gain, right.right_gain),
            (1.0, 0.0, rail)
        );
        // Frame 0 sums both voices: left gets the left voice, right the right.
        assert_eq!(m.render_stereo(1), vec![0.5 * rail, 0.5 * rail]);
    }

    #[test]
    fn planet_sized_offsets_saturate_distance_and_pan() {
        let sample = mono_hit();
        let mut m = SpatialMixer::new(8_000);
        // dx = dz = 2^32 - 1: the squared sum saturates, so the distance pins
        // to `u32::MAX` and the lateral offset to the `i32` rail.
        m.set_listener(Pos2::new(i32::MIN, i32::MIN), 0);
        m.play_at(&sample, Pos2::new(i32::MAX, i32::MAX), 1.0);
        m.set_listener(Pos2::new(i32::MAX, i32::MAX), 0);
        m.play_at(&sample, Pos2::new(i32::MIN, i32::MIN), 1.0);
        let (right, left) = (m.commands()[0], m.commands()[1]);
        assert_eq!(
            (right.dist_cm, right.atten, right.pan),
            (u32::MAX, 0.0, 1.0)
        );
        assert_eq!((left.dist_cm, left.atten, left.pan), (u32::MAX, 0.0, -1.0));
    }

    #[test]
    fn near_far_attenuation_on_scripted_positions() {
        let sample = mono_hit();
        let mut m = SpatialMixer::new(8_000);
        m.set_listener(Pos2::new(0, 0), 0);
        // Half-falloff distance due north (pan 0): atten exactly 0.5.
        let mid = ((NEAR_CM + FAR_CM) / 2) as i32;
        m.play_at(&sample, Pos2::new(0, -mid), 1.0);
        // Beyond FAR_CM: silence, gains exactly 0.
        m.play_at(&sample, Pos2::new(0, FAR_CM as i32 + 500), 1.0);
        let (half, far) = (m.commands()[0], m.commands()[1]);
        assert_eq!(half.atten, 0.5);
        assert_eq!((half.left_gain, half.right_gain), (0.5, 0.5));
        assert_eq!((far.atten, far.left_gain, far.right_gain), (0.0, 0.0, 0.0));
        // Mixed frame: 0.5 * 0.5 (half voice) + 0 (far voice), both ears.
        assert_eq!(m.render_stereo(1), vec![0.25, 0.25]);
    }

    #[test]
    fn yaw_quarter_turn_moves_east_to_center() {
        let sample = mono_hit();
        let mut m = SpatialMixer::new(8_000);
        // Facing east: a source due east is dead ahead (pan 0).
        m.set_listener(Pos2::new(0, 0), 90);
        m.play_at(&sample, Pos2::new(500, 0), 1.0);
        assert_eq!(m.commands()[0].pan, 0.0);
        // Facing east: a source due south is hard right.
        m.play_at(&sample, Pos2::new(0, 500), 1.0);
        assert_eq!(m.commands()[1].pan, 0.5);
        // Yaw wraps: 450 behaves as 90, -270 too.
        m.set_listener(Pos2::new(0, 0), 450);
        m.play_at(&sample, Pos2::new(500, 0), 1.0);
        assert_eq!(m.commands()[2].pan, 0.0);
        m.set_listener(Pos2::new(0, 0), -270);
        m.play_at(&sample, Pos2::new(500, 0), 1.0);
        assert_eq!(m.commands()[3].pan, 0.0);
    }

    #[test]
    fn event_script_twice_gives_same_mix_commands() {
        // Same scripted fight twice: identical commands and bytes.
        let script: Vec<(FightCue, Pos2)> = vec![
            (
                FightCue::Hit {
                    attacker: 0,
                    victim: 1,
                    damage: 120,
                },
                Pos2::new(200, 0),
            ),
            (
                FightCue::Parried {
                    attacker: 1,
                    victim: 0,
                },
                Pos2::new(-300, 100),
            ),
            (
                FightCue::Knockdown {
                    attacker: 0,
                    victim: 1,
                },
                Pos2::new(200, 0),
            ),
        ];
        let run = || {
            let mut crowd = Crowd::new();
            let mut mix = SpatialMixer::new(8_000);
            mix.set_listener(Pos2::new(0, -800), 0);
            let sample = mono_hit();
            let mut reactions = Vec::new();
            for (cue, pos) in &script {
                reactions.push(crowd.on_fight_event(*cue));
                mix.play_at(&sample, *pos, 1.0);
                crowd.tick();
            }
            let bytes = mix.render_stereo(4);
            let cmds = mix.commands().to_vec();
            (reactions, cmds, bytes)
        };
        let (r1, c1, b1) = run();
        let (r2, c2, b2) = run();
        assert_eq!(r1, r2, "crowd reactions must replay");
        assert_eq!(c1, c2, "mix commands must replay");
        assert_eq!(b1, b2, "rendered bytes must replay");
        assert_eq!(c1.len(), 3);
    }

    use crate::cue::{Clip, Group};
    use crate::dsp::Bus;

    /// One-take cue over `samples` on the SFX bus.
    fn test_cue(spatial: Spatial, samples: Pcm) -> Cue {
        Cue {
            name: "t".to_string(),
            variants: vec![Clip::from_pcm(&samples)],
            spatial,
            group: Group::Impact,
            looped: false,
            priority: 100,
            bus: Bus::Sfx,
            gain: 1.0,
            ducks_music: false,
        }
    }

    fn ranged(min: u32, max: u32) -> Spatial {
        Spatial::Positional {
            falloff_min_cm: min,
            falloff_max_cm: max,
        }
    }

    #[test]
    fn falloff_gain_follows_one_over_r_between_min_and_max() {
        let g = |d| falloff_gain(d, 100, 3_200);
        assert_eq!((g(0), g(100)), (1.0, 1.0), "full volume inside min");
        assert_eq!(g(200), 0.5, "6 dB per doubling");
        assert_eq!(g(400), 0.25);
        assert_eq!(g(3_200), 0.031_25, "last step before the cut");
        assert_eq!((g(3_201), g(u32::MAX)), (0.0, 0.0), "silent beyond max");
        let mut prev = 1.0;
        for d in (0..4_000).step_by(7) {
            let v = g(d);
            assert!(
                (0.0..=1.0).contains(&v) && v <= prev,
                "monotone at {d}: {v}"
            );
            prev = v;
        }
    }

    #[test]
    fn sine_table_is_exact_on_the_axes_and_unit_length_everywhere() {
        let at = [
            (0, 0),
            (30, 8_192),
            (90, 16_384),
            (150, 8_192),
            (180, 0),
            (210, -8_192),
        ];
        for (deg, want) in at {
            assert_eq!(sin_q14(deg), want, "sin({deg})");
        }
        assert_eq!(
            (sin_q14(270), sin_q14(-90), sin_q14(360), sin_q14(450)),
            (-16_384, -16_384, 0, 16_384)
        );
        for deg in -720..720 {
            let (s, c) = (sin_q14(deg), sin_q14(deg + 90));
            let off = (s * s + c * c - TRIG_ONE * TRIG_ONE).abs();
            assert!(off < 1 << 16, "sin^2 + cos^2 off by {off} at {deg}");
        }
    }

    #[test]
    fn yaw_between_quarter_turns_pans_by_the_exact_angle() {
        let mut m = SpatialMixer::new(8_000);
        m.set_listener(Pos2::new(0, 0), 45);
        // Facing north-east, a source due east is 45 degrees to the right.
        m.play_at(&mono_hit(), Pos2::new(1_000, 0), 1.0);
        assert_eq!(m.commands()[0].pan, 0.707);
        m.set_listener(Pos2::new(0, 0), -45);
        m.play_at(&mono_hit(), Pos2::new(1_000, 0), 1.0);
        assert_eq!(
            m.commands()[1].pan,
            0.707,
            "facing north-west it is right too"
        );
        m.play_at(&mono_hit(), Pos2::new(-1_000, 0), 1.0);
        assert_eq!(m.commands()[2].pan, -0.707);
    }

    #[test]
    fn camera_height_counts_toward_cue_distance() {
        let cue = test_cue(ranged(100, 1_000), mono_hit());
        let mut m = SpatialMixer::new(8_000);
        m.set_listener_3d([0, 300, 0], 0);
        assert_eq!(m.listener_3d(), [0, 300, 0]);
        assert_eq!(m.listener(), Pos2::new(0, 0));
        // 400 east, 300 below: the 3-4-5 triangle puts it at 500 cm.
        m.play_cue(CueId(0), &cue, 0, 1.0, Some([400, 0, 0]))
            .expect("in range");
        let cmd = m.commands()[0];
        assert_eq!(cmd.dist_cm, 500);
        assert_eq!(cmd.atten, 0.2);
        assert_eq!(cmd.pan, 0.4);
        assert_eq!((cmd.left_gain, cmd.right_gain), (0.2 * (1.0 - 0.4), 0.2));
    }

    #[test]
    fn cue_beyond_falloff_max_is_culled_without_a_voice() {
        let cue = test_cue(ranged(100, 1_000), mono_hit());
        let mut m = SpatialMixer::new(8_000);
        assert!(m
            .play_cue(CueId(0), &cue, 0, 1.0, Some([1_001, 0, 0]))
            .is_none());
        assert_eq!((m.culled(), m.voice_count(), m.commands().len()), (1, 0, 0));
        assert!(m
            .play_cue(CueId(0), &cue, 0, 1.0, Some([1_000, 0, 0]))
            .is_some());
        assert_eq!(m.culled(), 1, "exactly at max still sounds");
        assert!(
            m.play_cue(CueId(0), &cue, 9, 1.0, None).is_none(),
            "no such variant"
        );
    }

    #[test]
    fn flat_cues_and_positionless_plays_use_both_ears_equally() {
        let mut m = SpatialMixer::new(8_000);
        m.set_listener_3d([5_000, 0, 5_000], 123);
        let flat = test_cue(Spatial::Flat, mono_hit());
        let near = test_cue(ranged(100, 1_000), mono_hit());
        m.play_cue(CueId(0), &flat, 0, 0.5, Some([0, 0, 0]))
            .expect("flat ignores position");
        m.play_cue(CueId(1), &near, 0, 0.5, None)
            .expect("no position, no falloff");
        for cmd in m.commands() {
            assert_eq!(
                (cmd.atten, cmd.pan, cmd.left_gain, cmd.right_gain),
                (1.0, 0.0, 0.5, 0.5)
            );
        }
    }

    #[test]
    fn positional_stereo_clip_folds_to_mono_then_pans() {
        let stereo = Pcm::new(8_000, 2, vec![0.2, 0.6, 0.2, 0.6]);
        let cue = test_cue(ranged(100, 1_000), stereo);
        let mut m = SpatialMixer::new(8_000);
        // Hard right at the falloff max: gain 100/1000 on the right ear only.
        m.play_cue(CueId(0), &cue, 0, 1.0, Some([1_000, 0, 0]))
            .expect("in range");
        let out = m.render_stereo(1);
        let want = 0.4 * 0.1 * Bus::Sfx.gain_linear();
        assert_eq!(out[0], 0.0);
        assert!((out[1] - want).abs() < 1e-6, "{} vs {want}", out[1]);
    }

    #[test]
    fn live_mixer_records_no_commands() {
        let cue = test_cue(Spatial::Flat, mono_hit());
        let mut live = SpatialMixer::live(8_000);
        live.play_cue(CueId(0), &cue, 0, 1.0, None).expect("room");
        live.play_at(&mono_hit(), Pos2::new(0, 0), 1.0);
        assert!(
            live.commands().is_empty(),
            "the callback form keeps no growing log"
        );
        assert_eq!(live.voice_count(), 2);
        assert_eq!(live.stop_cue(CueId(0)), 1);
        assert_eq!(live.mixer_stats().started, 1);
        live.mixer_mut().set_group_limit(Group::Impact, 0);
        assert_eq!(live.mixer().group_limit(Group::Impact), 0);
    }
}
