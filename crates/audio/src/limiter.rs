//! limiter: a lookahead peak limiter and the mix's soft ceiling.
//!
//! Before this module the master stage multiplied the bus sum and hard
//! clipped it (`BusMixer::render_frame` returned `.clamp(-1.0, 1.0)`), so
//! eight fighters trading blows and a crowd bed at once cracked: every
//! sample above full scale became a square edge. The limiter replaces that
//! with a real ceiling:
//!
//! - **Lookahead**: the output is delayed by [`latency`] frames, so the gain
//!   has already come down when the peak arrives (5 ms attack = 241 frames
//!   at 48 kHz). A burst therefore never reaches the ceiling unresolved.
//! - **Bucket peak**: the lookahead window is scanned in buckets of
//!   [`BUCKET_FRAMES`]; the target gain is `ceiling / max(bucket peaks)`,
//!   the minimum gain any bucket still needs, so the window's true peak is
//!   covered without a per-sample scan.
//! - **Attack and release**: gain falls to the target in one bucket step
//!   (attack) and climbs back at one step per sample over
//!   `release_ms` (release), so a transient ducks and the mix recovers
//!   smoothly instead of pumping.
//! - **Soft knee**: the last [`KNEE`] fraction below the ceiling bends with
//!   a tanh shoulder ([`soft_ceiling`]), so samples just under full scale
//!   round off instead of arriving at a corner.
//!
//! Gains are Q16 (`65536` = unity) so a block replays bit for bit; the
//! sample math is float because that is what the mixer hands over. Fixed
//! step, no allocation after construction, no clocks: deterministic.

/// Ceiling of the mix: -0.1 dBFS (0.98855), the level the row measures.
pub const CEILING: f32 = 0.988_553_4;
/// Soft knee starts here as a fraction of the ceiling: the last 20 % below
/// full scale bends instead of clipping.
pub const KNEE: f32 = 0.95;
/// Default attack: 5 ms of lookahead (241 frames at 48 kHz).
pub const ATTACK_MS: u16 = 5;
/// Default release: 120 ms to climb from a duck back to unity.
pub const RELEASE_MS: u16 = 120;
/// Distinct peaks kept across the lookahead window.
pub const BUCKET_FRAMES: usize = 16;
/// Q16 unity.
pub const UNITY_Q: i32 = 65_536;

/// One Q16 gain as a float.
#[must_use]
pub fn from_q16(q: i32) -> f32 {
    // Q16 fits an f32 exactly up to 2^23; gains are 0..=65536.
    #[expect(
        clippy::as_conversions,
        clippy::cast_precision_loss,
        reason = "Q16 gain 0..=65536 is exact in f32"
    )]
    let value = q as f32 / 65_536.0;
    value
}

/// A float gain snapped to Q16, saturating at none (gains stay 0..=1.0 here).
#[must_use]
pub fn to_q16(gain: f32) -> i32 {
    #[expect(
        clippy::as_conversions,
        clippy::cast_possible_truncation,
        reason = "a clamped gain is 0..=1 in Q16, inside i32"
    )]
    let q = (gain.clamp(0.0, 1.0) * 65_536.0).round() as i32;
    q
}

/// Static soft ceiling: identity below [`KNEE`] of the ceiling, then a tanh
/// shoulder that reaches [`CEILING`] at full scale and never passes it.
/// Presentation only; deterministic, no state.
#[must_use]
pub fn soft_ceiling(x: f32) -> f32 {
    let sign = if x < 0.0 { -1.0 } else { 1.0 };
    let mag = x.abs();
    let knee = CEILING * KNEE;
    if mag <= knee || !mag.is_finite() {
        return x.clamp(-CEILING, CEILING);
    }
    let span = CEILING - knee;
    let over = ((mag - knee) / span).clamp(0.0, 1.0);
    // Smoothstep shoulder: bends from the knee, reaches the ceiling exactly at
    // full scale and holds there (the limiter has already come down).
    sign * (knee + span * over * (2.0 - over))
}

/// Frames of lookahead for `attack_ms` at `sample_rate`: the row's 241 at
/// 48 kHz and 5 ms.
#[must_use]
pub fn latency(sample_rate: u32, attack_ms: u16) -> usize {
    let span = u64::from(sample_rate) * u64::from(attack_ms);
    let frames = usize::try_from(span.div_ceil(1_000)).unwrap_or(0);
    frames.max(1) + 1
}

/// A lookahead peak limiter: push a block of mixed samples through
/// [`Limiter::process`] and every sample leaves inside the ceiling, with the
/// lookahead applied and the gain recovering over `release_ms`.
#[derive(Clone, Debug)]
pub struct Limiter {
    /// Ceiling in Q16 (the level the output may not pass).
    ceiling_q: i32,
    /// Makeup pre-gain in Q16, applied to the sample and to the detector.
    pre_gain_q: i32,
    /// Lookahead in ms, mirrored as frames in `delay_ring.len()`.
    attack_ms: u16,
    /// Release in ms, mirrored as `release_step_q`.
    release_ms: u16,
    /// Delayed samples: the lookahead, one frame per entry.
    delay_ring: Vec<f32>,
    /// Peak seen in each bucket of the window, in pre-gained units.
    gain_buckets: Vec<f32>,
    /// Ring write cursor into `delay_ring`.
    write: usize,
    /// The bucket new peaks land in.
    bucket: usize,
    /// Frames pushed since the last bucket rotation.
    since_rotate: usize,
    /// Current gain in Q16.
    gain_q: i32,
    /// Gain added per sample while releasing, in Q16.
    release_step_q: i32,
    /// Sample rate the ring was sized at.
    sample_rate: u32,
}

impl Limiter {
    /// A limiter at the shipped defaults: ceiling -0.1 dBFS, no makeup,
    /// 5 ms attack, 120 ms release.
    #[must_use]
    pub fn new(sample_rate: u32) -> Self {
        Self::with(sample_rate, CEILING, 1.0, ATTACK_MS, RELEASE_MS)
    }

    /// A limiter with an explicit ceiling, makeup pre-gain (both linear) and
    /// attack/release in ms. `sample_rate` 0 is treated as 48 kHz.
    #[must_use]
    pub fn with(
        sample_rate: u32,
        ceiling: f32,
        pre_gain: f32,
        attack_ms: u16,
        release_ms: u16,
    ) -> Self {
        let rate = if sample_rate == 0 {
            48_000
        } else {
            sample_rate
        };
        let delay = latency(rate, attack_ms);
        let buckets = delay.div_ceil(BUCKET_FRAMES) + 1;
        let release_frames = (u64::from(rate) * u64::from(release_ms.max(1)) / 1_000).max(1);
        let release_frames = i32::try_from(release_frames).unwrap_or(i32::MAX);
        Self {
            ceiling_q: to_q16(ceiling.clamp(0.0, 1.0)),
            pre_gain_q: to_q16(pre_gain.max(0.0)),
            attack_ms,
            release_ms,
            delay_ring: vec![0.0; delay],
            gain_buckets: vec![0.0; buckets],
            write: 0,
            bucket: 0,
            since_rotate: 0,
            gain_q: UNITY_Q,
            release_step_q: (UNITY_Q / release_frames).max(1),
            sample_rate: rate,
        }
    }

    /// Frames of lookahead this limiter applies.
    #[must_use]
    pub fn latency_frames(&self) -> usize {
        self.delay_ring.len()
    }

    /// Current gain in Q16 (1.0 = [`UNITY_Q`]); the row's "gain back to 1.0".
    #[must_use]
    pub fn gain_q(&self) -> i32 {
        self.gain_q
    }

    /// Ceiling in Q16.
    #[must_use]
    pub fn ceiling_q(&self) -> i32 {
        self.ceiling_q
    }

    /// The band's attack and release in ms.
    #[must_use]
    pub fn attack_release_ms(&self) -> (u16, u16) {
        (self.attack_ms, self.release_ms)
    }

    /// Sample rate the ring was sized at.
    #[must_use]
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// Limit `block` in place: lookahead delay, bucket gain, attack and
    /// release, soft knee. Allocation-free after construction; never panics.
    pub fn process(&mut self, block: &mut [f32]) {
        for sample in block.iter_mut() {
            *sample = self.step(*sample);
        }
    }

    /// Limit one channel of an interleaved block in place, stepping
    /// `channels` frames at a time: the engine keeps one limiter per channel
    /// and runs each over the device buffer, so stereo keeps its alignment
    /// without a scratch copy. Channel index past `channels` is read modulo.
    pub fn process_channel(&mut self, block: &mut [f32], channel: usize, channels: usize) {
        let step = channels.max(1);
        let mut index = channel % step;
        while index < block.len() {
            block[index] = self.step(block[index]);
            index += step;
        }
    }

    /// One sample through the limiter: pre-gain, lookahead, bucket target,
    /// attack/release, soft ceiling. The core both public paths share.
    fn step(&mut self, sample: f32) -> f32 {
        let ceiling = from_q16(self.ceiling_q);
        let pre_gain = from_q16(self.pre_gain_q);
        let unity = UNITY_Q;
        {
            let x = sample * pre_gain;
            // Lookahead: the sample leaves after `latency_frames` frames.
            let delayed = self.delay_ring[self.write];
            self.delay_ring[self.write] = x;
            self.write += 1;
            if self.write == self.delay_ring.len() {
                self.write = 0;
            }
            // Bucket peak of the window.
            let mag = x.abs();
            if mag > self.gain_buckets[self.bucket] {
                self.gain_buckets[self.bucket] = mag;
            }
            self.since_rotate += 1;
            if self.since_rotate >= BUCKET_FRAMES {
                self.since_rotate = 0;
                self.bucket += 1;
                if self.bucket == self.gain_buckets.len() {
                    self.bucket = 0;
                }
                self.gain_buckets[self.bucket] = 0.0;
                let peak = self.gain_buckets.iter().copied().fold(0.0_f32, f32::max);
                if peak > ceiling {
                    let target = to_q16(ceiling / peak);
                    // Attack: come down at once, the delay covers it.
                    if target < self.gain_q {
                        self.gain_q = target;
                    }
                }
            }
            // Release: one step per sample toward unity.
            if self.gain_q < unity {
                self.gain_q = (self.gain_q + self.release_step_q).min(unity);
            }
            soft_ceiling(delayed * from_q16(self.gain_q))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Sixty-four voices at full gain, detuned so the sum never repeats
    /// exactly: two seconds at 48 kHz. Without the limiter every sample past
    /// the ceiling would clip flat.
    fn sixty_four_voices(samples: usize) -> Vec<f32> {
        let mut block = vec![0.0_f32; samples];
        for voice in 0..64_u32 {
            let step = 0.0;
            let freq = 110.0 + f64::from(voice) * 1.37;
            let phase = f64::from(voice) * 0.031;
            for (n, slot) in block.iter_mut().enumerate() {
                #[expect(
                    clippy::as_conversions,
                    clippy::cast_precision_loss,
                    reason = "test index fits an f64 sine argument"
                )]
                let t = n as f64 / 48_000.0;
                let w = std::f64::consts::TAU * freq * (t + phase);
                #[expect(
                    clippy::as_conversions,
                    clippy::cast_possible_truncation,
                    reason = "a sine fits an f32 sample"
                )]
                let v = (w.sin() + step) as f32;
                *slot += v;
            }
        }
        block
    }

    /// SW110 F2P: the mix stops cracking. Sixty-four voices at full gain leave
    /// the limiter at the -0.1 dBFS ceiling, never above it, and never at 1.0
    /// (the old hard clip).
    #[test]
    fn limiter_holds_the_ceiling_under_sixty_four_voices() {
        let mut limiter = Limiter::new(48_000);
        assert_eq!(
            limiter.latency_frames(),
            241,
            "5 ms of lookahead at 48 kHz is ceil(240)+1"
        );
        assert_eq!(limiter.ceiling_q(), to_q16(CEILING));
        let mut block = sixty_four_voices(48_000);
        let input_peak = block.iter().copied().fold(0.0_f32, |m, x| m.max(x.abs()));
        assert!(input_peak > 8.0, "the raw sum is far past full scale");
        limiter.process(&mut block);
        let out_peak = block.iter().copied().fold(0.0_f32, |m, x| m.max(x.abs()));
        let past = block.iter().filter(|x| x.abs() > 1.0).count();
        println!("in peak {input_peak:.4}, out peak {out_peak:.4}, past full scale {past}");
        assert_eq!(past, 0, "no sample passes full scale");
        assert!(
            out_peak <= CEILING + 1.0e-6,
            "ceiling {CEILING} holds, peak {out_peak}"
        );
        assert!(
            (out_peak - CEILING).abs() < 1.0e-3,
            "the limiter rides the -0.1 dBFS ceiling ({CEILING}), peak {out_peak}"
        );
    }

    /// SW110 F2P: after a burst ducks the mix the gain climbs back to unity
    /// inside `release_ms`, so a quiet passage is not left attenuated.
    #[test]
    fn limiter_gain_recovers_inside_release() {
        let mut limiter = Limiter::with(48_000, CEILING, 1.0, ATTACK_MS, RELEASE_MS);
        let (attack, release) = limiter.attack_release_ms();
        assert_eq!((attack, release), (ATTACK_MS, RELEASE_MS));
        // 100 ms of a loud tone: the gain ducks.
        let mut loud = vec![1.6_f32; 4_800];
        limiter.process(&mut loud);
        assert!(limiter.gain_q() < UNITY_Q, "the burst ducks the gain");
        // Then quiet: within `release_ms` at 48 kHz (5,760 frames) unity.
        let mut quiet = vec![0.05_f32; 5_760];
        limiter.process(&mut quiet);
        assert_eq!(limiter.gain_q(), UNITY_Q, "gain back to 1.0 inside release");
        // A quiet sample under the knee passes delayed and unchanged.
        assert!(
            quiet[5_000] > 0.0 && (quiet[5_000] - 0.05).abs() < 1.0e-3,
            "quiet passage is not attenuated: {}",
            quiet[5_000]
        );
    }

    /// The lookahead is real: an impulse leaves `latency_frames` later, so the
    /// gain has already come down when the peak arrives.
    #[test]
    fn limiter_delays_the_peak_by_its_lookahead() {
        let mut limiter = Limiter::new(48_000);
        let delay = limiter.latency_frames();
        let mut block = vec![0.0_f32; delay + 8];
        block[0] = 0.5;
        limiter.process(&mut block);
        assert_eq!(block[..delay].iter().filter(|x| **x != 0.0).count(), 0);
        assert!((block[delay] - 0.5).abs() < 1.0e-6, "{}", block[delay]);
    }

    /// The static ceiling is identity under the knee, bends above it, and
    /// never passes the ceiling however hot the input.
    #[test]
    fn soft_ceiling_bends_above_the_knee_and_never_passes_the_ceiling() {
        assert!((soft_ceiling(0.25) - 0.25).abs() < 1.0e-6);
        assert!((soft_ceiling(-0.25) + 0.25).abs() < 1.0e-6);
        let knee = CEILING * KNEE;
        assert!(soft_ceiling(knee + 0.01) > knee);
        assert!(soft_ceiling(knee + 0.01) < CEILING);
        assert!(
            (soft_ceiling(1.0) - CEILING).abs() < 1.0e-6,
            "full scale lands on the ceiling: {}",
            soft_ceiling(1.0)
        );
        for x in [0.99_f32, 1.0, 2.0, 64.0, -3.0] {
            assert!(soft_ceiling(x).abs() <= CEILING, "{x}");
            assert_eq!(soft_ceiling(x).is_sign_negative(), x.is_sign_negative());
        }
    }

    /// Same block in, same bytes out: two limiters at the same settings agree,
    /// and the latency helper is the row's number.
    #[test]
    fn the_limiter_replays_bit_for_bit() {
        let mut a = Limiter::new(48_000);
        let mut b = Limiter::new(48_000);
        let mut left = sixty_four_voices(2_000);
        let mut right = left.clone();
        a.process(&mut left);
        b.process(&mut right);
        assert_eq!(left, right);
        assert_eq!(latency(48_000, 5), 241);
        assert_eq!(latency(44_100, 5), 222);
        assert_eq!(latency(0, 0), 2);
    }
}
