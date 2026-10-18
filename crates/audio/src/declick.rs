//! Click-free gain ramps. A [`Declicker`] moves a Q24 gain between silence and
//! unity over a whole number of frames. Asked to change direction mid-ramp it
//! resumes from the gain it has reached, so the gain never jumps, and the
//! slope stays `UNITY_Q24 / len` per frame. Integer math only: a fading gain
//! is `UNITY_Q24 * frames_left / len`, so there are no tables, no floats and
//! no heap on the mix path.

/// Q24 unity gain (1.0). Mixer levels are Q24 throughout.
pub const UNITY_Q24: u32 = 1 << 24;

/// Gain state of one click-free ramp; read the gain with [`Declicker::level`].
///
/// `frames_left` counts down the frames still to run of a `len`-frame ramp.
/// Reversing keeps the gain and the slope, only the frames left change, so a
/// fade that turns around never steps faster than the fade it interrupts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Declicker {
    /// Silent and at rest.
    SettledAt0,
    /// Full level and at rest.
    SettledAt1,
    /// Falling to silence.
    FadingTo0 { frames_left: u32, len: u32 },
    /// Rising to full level.
    FadingTo1 { frames_left: u32, len: u32 },
}

impl Declicker {
    /// At rest at full level when `enabled`, silent otherwise.
    pub const fn settled(enabled: bool) -> Self {
        if enabled {
            Self::SettledAt1
        } else {
            Self::SettledAt0
        }
    }

    /// Current Q24 gain: exactly `0` at rest silent, exactly
    /// [`UNITY_Q24`] at rest full.
    pub fn level(&self) -> u32 {
        match *self {
            Self::SettledAt0 => 0,
            Self::SettledAt1 => UNITY_Q24,
            Self::FadingTo0 { frames_left, len } => ramp_q24(frames_left, len),
            Self::FadingTo1 { frames_left, len } => {
                UNITY_Q24.saturating_sub(ramp_q24(frames_left, len))
            }
        }
    }

    /// True when at rest silent, or falling to silence.
    pub fn trending_towards_zero(&self) -> bool {
        matches!(self, Self::SettledAt0 | Self::FadingTo0 { .. })
    }

    /// Ramp toward full (`enabled`) or toward silence over `len` frames,
    /// starting from the current gain. A ramp already heading that way keeps
    /// its slope. The other direction resumes from the gain reached. `len` 0,
    /// or a gain within half a step of the target, settles at once.
    pub fn fade_to(&mut self, enabled: bool, len: u32) {
        let heading_there = match *self {
            Self::SettledAt1 | Self::FadingTo1 { .. } => enabled,
            Self::SettledAt0 | Self::FadingTo0 { .. } => !enabled,
        };
        if heading_there {
            return;
        }
        let level = self.level();
        let need = if enabled {
            UNITY_Q24.saturating_sub(level)
        } else {
            level
        };
        let frames_left = ramp_frames(need, len);
        *self = if frames_left == 0 {
            Self::settled(enabled)
        } else if enabled {
            Self::FadingTo1 { frames_left, len }
        } else {
            Self::FadingTo0 { frames_left, len }
        };
    }

    /// Move one frame along the running ramp. The last frame settles at the
    /// target; a settled state is unchanged.
    pub fn advance(&mut self) {
        *self = match *self {
            Self::FadingTo0 { frames_left, len } if frames_left > 1 => Self::FadingTo0 {
                frames_left: frames_left - 1,
                len,
            },
            Self::FadingTo1 { frames_left, len } if frames_left > 1 => Self::FadingTo1 {
                frames_left: frames_left - 1,
                len,
            },
            Self::FadingTo0 { .. } => Self::SettledAt0,
            Self::FadingTo1 { .. } => Self::SettledAt1,
            settled => settled,
        };
    }
}

/// Gain of a falling ramp with `frames_left` of `len` frames to run.
fn ramp_q24(frames_left: u32, len: u32) -> u32 {
    let len = len.max(1);
    u32::try_from(u64::from(UNITY_Q24) * u64::from(frames_left.min(len)) / u64::from(len))
        .unwrap_or(UNITY_Q24)
}

/// Frames of a `len`-frame ramp that cover `need` Q24 gain, rounded to the
/// nearest frame and never more than `len`.
fn ramp_frames(need: u32, len: u32) -> u32 {
    let half = u64::from(UNITY_Q24) / 2;
    u32::try_from((u64::from(need) * u64::from(len) + half) / u64::from(UNITY_Q24))
        .unwrap_or(len)
        .min(len)
}

/// Two-gain crossfade law for one frame: `(dry, wet)` Q24 gains for a `wet`
/// level, with `dry + wet == UNITY_Q24` at every level. `wet` clamps to
/// unity. Mixes an unprocessed path with a processed one from one ramp.
pub fn dry_wet(wet: u32) -> (u32, u32) {
    let wet = wet.min(UNITY_Q24);
    (UNITY_Q24 - wet, wet)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Gain before each frame of a ramp run to rest, then the rest gain.
    fn trace(mut d: Declicker, frames: usize) -> Vec<u32> {
        let mut out = Vec::with_capacity(frames + 1);
        for _ in 0..frames {
            out.push(d.level());
            d.advance();
        }
        out.push(d.level());
        out
    }

    fn at_rest(d: &Declicker) -> bool {
        matches!(d, Declicker::SettledAt0 | Declicker::SettledAt1)
    }

    fn largest_step(levels: &[u32]) -> u32 {
        levels
            .windows(2)
            .map(|w| w[0].abs_diff(w[1]))
            .max()
            .unwrap_or(0)
    }

    #[test]
    fn fade_up_is_monotone_and_lands_on_unity_in_n_frames() {
        let n = 240;
        let mut d = Declicker::settled(false);
        d.fade_to(true, n);
        let levels = trace(d, n as usize);
        assert_eq!(levels[0], 0, "starts silent");
        assert_eq!(levels[n as usize], UNITY_Q24, "lands on unity exactly");
        assert!(levels.windows(2).all(|w| w[1] > w[0]), "rises every frame");
        let mut d = Declicker::settled(false);
        d.fade_to(true, n);
        for _ in 0..n {
            d.advance();
        }
        assert_eq!(d, Declicker::SettledAt1, "at rest after n frames");
    }

    #[test]
    fn no_step_is_larger_than_one_ramp_slope() {
        for n in [1u32, 7, 64, 240, 4_800] {
            let slope = UNITY_Q24.div_ceil(n);
            let mut up = Declicker::settled(false);
            up.fade_to(true, n);
            assert!(largest_step(&trace(up, n as usize)) <= slope, "up n={n}");
            let mut down = Declicker::settled(true);
            down.fade_to(false, n);
            assert!(
                largest_step(&trace(down, n as usize)) <= slope,
                "down n={n}"
            );
        }
    }

    #[test]
    fn reversing_mid_fade_continues_from_the_gain_reached() {
        let n = 300;
        let slope = UNITY_Q24.div_ceil(n);
        let mut d = Declicker::settled(false);
        d.fade_to(true, n);
        for _ in 0..120 {
            d.advance();
        }
        let reached = d.level();
        assert!(reached > 0 && reached < UNITY_Q24, "mid-fade: {reached}");

        d.fade_to(false, n);
        let turned = d.level();
        assert!(
            turned.abs_diff(reached) <= slope / 2 + 1,
            "no jump at the reversal: {reached} -> {turned}"
        );

        let mut levels = vec![reached];
        levels.extend(trace(d, n as usize));
        assert!(
            largest_step(&levels) <= slope,
            "reversal keeps the original step: {}",
            largest_step(&levels)
        );
        assert_eq!(levels.last().copied(), Some(0), "back to silence");
    }

    #[test]
    fn reversal_takes_as_long_as_the_gain_it_has_to_give_back() {
        let n = 300;
        let mut d = Declicker::settled(false);
        d.fade_to(true, n);
        for _ in 0..100 {
            d.advance();
        }
        d.fade_to(false, n);
        let mut frames: u32 = 0;
        while !at_rest(&d) {
            d.advance();
            frames += 1;
        }
        assert!(
            frames.abs_diff(100) <= 1,
            "one third of the ramp went up, so one third comes back: {frames}"
        );
        assert_eq!(d, Declicker::SettledAt0);
    }

    #[test]
    fn same_direction_keeps_its_ramp() {
        let mut d = Declicker::settled(false);
        d.fade_to(true, 100);
        d.advance();
        let before = d;
        d.fade_to(true, 10);
        assert_eq!(d, before, "already heading to unity");
    }

    #[test]
    fn partial_level_fade_out_runs_at_the_requested_slope() {
        // Rise from silence over 100 frames, stop 40 frames in (gain 0.4),
        // then fade out over 50 frames: the fade runs 20 frames, not 40.
        let mut d = Declicker::settled(false);
        d.fade_to(true, 100);
        for _ in 0..40 {
            d.advance();
        }
        let level = d.level();
        assert!(level > 0 && level < UNITY_Q24 / 2, "level {level}");
        d.fade_to(false, 50);
        assert!(
            d.level().abs_diff(level) <= UNITY_Q24.div_ceil(50) / 2 + 1,
            "no jump when the fade turns: {level} -> {}",
            d.level()
        );
        let mut frames: u32 = 0;
        while !at_rest(&d) {
            d.advance();
            frames += 1;
        }
        assert_eq!(d, Declicker::SettledAt0);
        assert!(
            frames.abs_diff(((u64::from(level) * 50) / u64::from(UNITY_Q24)) as u32) <= 1,
            "{frames} frames left from {level}"
        );
    }

    #[test]
    fn settled_states_are_exact_silence_and_q24_unity() {
        assert_eq!(Declicker::settled(false).level(), 0);
        assert_eq!(Declicker::settled(true).level(), 1 << 24);
        assert_eq!(UNITY_Q24, 16_777_216);
        assert!(at_rest(&Declicker::settled(true)));
        assert!(Declicker::settled(false).trending_towards_zero());
        assert!(!Declicker::settled(true).trending_towards_zero());
    }

    #[test]
    fn dry_wet_pair_sums_to_unity_at_every_level() {
        for wet in (0..=UNITY_Q24).step_by(9_973).chain([UNITY_Q24, 1]) {
            let (dry, w) = dry_wet(wet);
            assert_eq!(dry + w, UNITY_Q24, "wet {wet}");
        }
        assert_eq!(dry_wet(0), (UNITY_Q24, 0), "all dry at rest");
        assert_eq!(dry_wet(UNITY_Q24 * 3), (0, UNITY_Q24), "clamped to unity");

        let mut d = Declicker::settled(false);
        d.fade_to(true, 333);
        for _ in 0..333 {
            let (dry, wet) = dry_wet(d.level());
            assert_eq!(dry + wet, UNITY_Q24);
            d.advance();
        }
    }

    #[test]
    fn zero_length_fade_snaps_to_its_target() {
        let mut d = Declicker::settled(false);
        d.fade_to(true, 0);
        assert_eq!(d, Declicker::SettledAt1, "no ramp at all");
        d.fade_to(false, 0);
        assert_eq!(d, Declicker::SettledAt0);

        let mut mid = Declicker::settled(false);
        mid.fade_to(true, 64);
        mid.advance();
        mid.fade_to(false, 0);
        assert_eq!(mid, Declicker::SettledAt0, "a mid-ramp snap settles too");
    }
}
