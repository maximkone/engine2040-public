//! gamefeel: integer port of the deepnight/gamefeel demo's presentation feel.
//!
//! PORT(deepnight/gamefeel@09d24aa:src/game/Camera.hx): `shakeS`, `bump`,
//! `bumpAng`, `bumpZoom` and the per-frame decay in `apply`/`update`.
//! PORT(deepnight/gamefeel@09d24aa:src/game/Game.hx): `stopFrame` hit-stop and
//! the slow-mo map with its smoothed `curGameSpeed`. MIT, read 2026-10-17; no
//! bytes vendored, the formulas are rewritten in integers.
//!
//! Owner: engine-combat. Consumer: [`crate::feel_flow::FeelFlow`] folds fight
//! events into a [`GameFeel`]; the presenter reads one [`FeelFrame`] per
//! presented tick and applies it on its own clock. Units: ticks at 60 Hz,
//! offsets in thousandths of a reference pixel (milli), scales in permille.
//! Determinism: integer math only (the reference's `cos`/`sin` are rebuilt on
//! a Q32-turn phase and a Taylor series), no heap, no wall clock; the same
//! calls give the same frames on every machine. Presentation only: nothing
//! here reads or writes the hashed fight state.
//!
//! Step order matches the reference frame: the caller starts effects during
//! the update, then calls [`GameFeel::step`] once as the post-update. Shake and
//! hit-stop sample before they count down; bump decays before it is read.

/// Reference shake amplitude in pixels times 1000: `2.5 * power * ratio`.
pub const SHAKE_AMP_MILLI: i64 = 2500;
/// Shake x phase step per tick: 1.1 rad as Q32 turns (`1.1 / 2pi * 2^32`).
const SHAKE_X_STEP: u64 = 751_921_803;
/// Shake y phase step per tick: 1.7 rad as Q32 turns.
const SHAKE_Y_STEP: u64 = 1_162_010_968;
/// Shake y phase offset: 0.3 rad as Q32 turns.
const SHAKE_Y_PHASE: u64 = 205_069_583;
/// Hard cap on a shake or slow-mo length in ticks (ten seconds).
pub const MAX_EFFECT_TICKS: u32 = 600;
/// Hard cap on a shake power in permille (four times the reference 1.0).
pub const MAX_SHAKE_POWER_PERMILLE: u32 = 4000;
/// Bump offset decay per tick in permille (`bumpFrict` 0.85).
pub const BUMP_FRICTION_PERMILLE: i64 = 850;
/// Bump zoom decay per tick in permille (0.9).
pub const BUMP_ZOOM_FRICTION_PERMILLE: i64 = 900;
/// Hit-stop length in ticks: the reference `stopFrame` holds 4 frames.
pub const STOP_FRAME_TICKS: u32 = 4;
/// Time scale during hit-stop in permille (the reference multiplies by 0.1).
pub const STOP_SCALE_PERMILLE: u32 = 100;
/// Slow-mo speed factor in permille when none is given (`speedFactor=0.3`).
pub const SLOWMO_DEFAULT_PERMILLE: u32 = 300;
/// Smoothing toward a faster game speed, permille of the gap per tick.
pub const SLOWMO_RISE_PERMILLE: i32 = 200;
/// Smoothing toward a slower game speed, permille of the gap per tick.
pub const SLOWMO_FALL_PERMILLE: i32 = 600;
/// Slow-mo never drops the base time below this permille (`0.2 + 0.8 * speed`).
pub const TIME_FLOOR_PERMILLE: i32 = 200;
/// Concurrent slow-mo ids the map holds (stack only).
pub const SLOWMO_SLOTS: usize = 4;

/// Gamefeel failures; bad data never panics.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GameFeelError {
    /// All [`SLOWMO_SLOTS`] ids are live and the new id is not one of them.
    SlowMoFull,
}

const ONE_Q30: i64 = 1 << 30;
const QUARTER_TURN: u32 = 1 << 30;
const HALF_PI_Q30: i64 = 1_686_629_713;

fn mul_q30(a: i64, b: i64) -> i64 {
    (a * b) >> 30
}

/// Sine of a phase given in Q32 turns (`u32::MAX + 1` is one full turn),
/// result in Q16 (`-65536..=65536`). Quadrant fold plus a nine-term Taylor
/// series in Q30; worst error is about 3 parts in 100000. Pure, no heap.
#[must_use]
pub fn sin_q16(phase: u32) -> i32 {
    let quadrant = phase >> 30;
    let mut frac = phase & (QUARTER_TURN - 1);
    if quadrant & 1 == 1 {
        frac = QUARTER_TURN - frac;
    }
    let x = (i64::from(frac) * HALF_PI_Q30) >> 30;
    let x2 = mul_q30(x, x);
    let mut s = ONE_Q30;
    for divisor in [72, 42, 20, 6] {
        s = ONE_Q30 - mul_q30(x2, s) / divisor;
    }
    let magnitude = mul_q30(x, s) >> 14;
    let signed = if quadrant >= 2 { -magnitude } else { magnitude };
    i32::try_from(signed).unwrap_or(0)
}

/// Cosine of a Q32-turn phase in Q16; see [`sin_q16`].
#[must_use]
pub fn cos_q16(phase: u32) -> i32 {
    sin_q16(phase.wrapping_add(QUARTER_TURN))
}

/// Low 32 bits of `ftime * step`: the reference phase `ftime * rate` in turns.
fn phase_of(ftime: u32, step: u64, offset: u64) -> u32 {
    u32::try_from((u64::from(ftime) * step + offset) & 0xFFFF_FFFF).unwrap_or(0)
}

/// The two shake waves at presented tick `ftime`, in Q16 units: x is
/// `cos(ftime * 1.1)` and y is `sin(0.3 + ftime * 1.7)` (radians), the
/// reference pair. Pure. [`Shake::step`] and the trauma-curve stage in
/// `crate::shake_stage` both read it, so they share one wave.
#[must_use]
pub fn shake_wave_q16(ftime: u32) -> (i32, i32) {
    (
        cos_q16(phase_of(ftime, SHAKE_X_STEP, 0)),
        sin_q16(phase_of(ftime, SHAKE_Y_STEP, SHAKE_Y_PHASE)),
    )
}

/// Camera shake timer: `shakeS(t, pow)` plus the per-frame offset.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Shake {
    remaining: u32,
    total: u32,
    power_permille: u32,
}

impl Shake {
    /// Start a shake. As in the reference (`cd.setS(.., false)`) a longer
    /// running shake is never cut short, but the power always updates.
    pub fn start(&mut self, ticks: u32, power_permille: u32) {
        let ticks = ticks.min(MAX_EFFECT_TICKS);
        self.power_permille = power_permille.min(MAX_SHAKE_POWER_PERMILLE);
        if ticks > self.remaining {
            self.remaining = ticks;
            self.total = ticks;
        }
    }

    /// Offset for presented tick `ftime`, then count down one tick. Returns
    /// `(x, y)` in milli pixels: `cos(ftime*1.1) * 2.5 * power * ratio` and
    /// `sin(0.3 + ftime*1.7) * 2.5 * power * ratio`, ratio = remaining/total.
    pub fn step(&mut self, ftime: u32) -> (i32, i32) {
        if self.remaining == 0 {
            return (0, 0);
        }
        let scale = SHAKE_AMP_MILLI * i64::from(self.power_permille) * i64::from(self.remaining);
        let denom = 65_536 * 1000 * i64::from(self.total.max(1));
        let (wave_x, wave_y) = shake_wave_q16(ftime);
        let x = i64::from(wave_x) * scale / denom;
        let y = i64::from(wave_y) * scale / denom;
        self.remaining -= 1;
        (i32::try_from(x).unwrap_or(0), i32::try_from(y).unwrap_or(0))
    }

    /// True while a shake is running.
    #[must_use]
    pub const fn active(&self) -> bool {
        self.remaining > 0
    }
}

/// Camera bump: a decaying offset plus a decaying zoom kick. Stored in micro
/// pixels so the 0.85 decay loses under one micro pixel per tick.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Bump {
    x_micro: i64,
    y_micro: i64,
    zoom_micro: i64,
}

impl Bump {
    /// `bump(x, y)`: add a raw offset in milli pixels.
    pub fn add(&mut self, x_milli: i32, y_milli: i32) {
        self.x_micro += i64::from(x_milli) * 1000;
        self.y_micro += i64::from(y_milli) * 1000;
    }

    /// `bumpAng(a, dist)`: add `dist` milli pixels along a Q32-turn angle.
    pub fn add_angle(&mut self, angle: u32, dist_milli: i32) {
        let dist = i64::from(dist_milli) * 1000;
        self.x_micro += i64::from(cos_q16(angle)) * dist / 65_536;
        self.y_micro += i64::from(sin_q16(angle)) * dist / 65_536;
    }

    /// `bumpZoom(z)`: replace (not add to) the zoom kick, in milli zoom units.
    pub fn set_zoom(&mut self, zoom_milli: i32) {
        self.zoom_micro = i64::from(zoom_milli) * 1000;
    }

    /// Decay one tick and return `(x, y, zoom)` in milli units. The presenter
    /// subtracts x and y from the scroll, as the reference `apply` does.
    pub fn step(&mut self) -> (i32, i32, i32) {
        self.x_micro = self.x_micro * BUMP_FRICTION_PERMILLE / 1000;
        self.y_micro = self.y_micro * BUMP_FRICTION_PERMILLE / 1000;
        self.zoom_micro = self.zoom_micro * BUMP_ZOOM_FRICTION_PERMILLE / 1000;
        let milli = |micro: i64| i32::try_from(micro / 1000).unwrap_or(0);
        (
            milli(self.x_micro),
            milli(self.y_micro),
            milli(self.zoom_micro),
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SlowMo {
    id: u8,
    ticks_left: u32,
    factor_permille: u32,
}

/// Presentation time scale: hit-stop plus the smoothed slow-mo map.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TimeScale {
    stop_left: u32,
    slow: [Option<SlowMo>; SLOWMO_SLOTS],
    speed_permille: i32,
}

impl Default for TimeScale {
    fn default() -> Self {
        Self {
            stop_left: 0,
            slow: [None; SLOWMO_SLOTS],
            speed_permille: 1000,
        }
    }
}

impl TimeScale {
    /// `stopFrame()`: hold the presented clock at [`STOP_SCALE_PERMILLE`] for
    /// [`STOP_FRAME_TICKS`] steps. A second call restarts the hold.
    pub fn stop_frame(&mut self) {
        self.stop_frame_for(STOP_FRAME_TICKS);
    }

    /// Hold the presented clock at [`STOP_SCALE_PERMILLE`] for `ticks` steps:
    /// the live fight's hit-stop count for the strike that landed (FEEL3).
    /// A second call restarts the hold; zero ticks holds nothing.
    pub fn stop_frame_for(&mut self, ticks: u32) {
        self.stop_left = ticks;
    }

    /// `addSlowMo(id, sec, factor)`: a repeated id takes the new factor and
    /// the longer duration; a new id needs a free slot.
    ///
    /// # Errors
    ///
    /// [`GameFeelError::SlowMoFull`] when every slot is live under other ids.
    pub fn add_slow_mo(
        &mut self,
        id: u8,
        ticks: u32,
        factor_permille: u32,
    ) -> Result<(), GameFeelError> {
        let ticks = ticks.min(MAX_EFFECT_TICKS);
        let factor_permille = factor_permille.min(1000);
        let mut free = None;
        for (i, slot) in self.slow.iter_mut().enumerate() {
            match slot {
                Some(s) if s.id == id => {
                    s.factor_permille = factor_permille;
                    s.ticks_left = s.ticks_left.max(ticks);
                    return Ok(());
                }
                None if free.is_none() => free = Some(i),
                _ => {}
            }
        }
        let i = free.ok_or(GameFeelError::SlowMoFull)?;
        self.slow[i] = Some(SlowMo {
            id,
            ticks_left: ticks,
            factor_permille,
        });
        Ok(())
    }

    /// One post-update: age the slow-mos, ease the game speed toward the
    /// product of the live factors, and return the presented clock rate in
    /// permille of a tick: `(0.2 + 0.8 * speed) * (0.1 while stopped)`.
    pub fn step(&mut self) -> u32 {
        let mut target = 1000_i32;
        for slot in &mut self.slow {
            let Some(s) = slot else { continue };
            s.ticks_left = s.ticks_left.saturating_sub(1);
            if s.ticks_left == 0 {
                *slot = None;
                continue;
            }
            let f = i32::try_from(s.factor_permille).unwrap_or(1000);
            target = (target * f + 500) / 1000;
        }
        let gap = target - self.speed_permille;
        let rate = if gap > 0 {
            SLOWMO_RISE_PERMILLE
        } else {
            SLOWMO_FALL_PERMILLE
        };
        // Never stall short of the snap band: a gap under the rounding step still moves one.
        let eased = gap * rate / 1000;
        self.speed_permille += if eased == 0 { gap.signum() } else { eased };
        if (self.speed_permille - target).abs() <= 1 {
            self.speed_permille = target;
        }
        let mut scale =
            TIME_FLOOR_PERMILLE + (1000 - TIME_FLOOR_PERMILLE) * self.speed_permille / 1000;
        if self.stop_left > 0 {
            self.stop_left -= 1;
            scale = scale * i32::try_from(STOP_SCALE_PERMILLE).unwrap_or(100) / 1000;
        }
        u32::try_from(scale).unwrap_or(0)
    }
}

/// What the presenter applies for one presented tick.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct FeelFrame {
    /// Shake offset in milli pixels.
    pub shake_milli: (i32, i32),
    /// Bump offset in milli pixels; the presenter subtracts it from the scroll.
    pub bump_milli: (i32, i32),
    /// Zoom kick in milli zoom units, added to the base zoom.
    pub zoom_milli: i32,
    /// Presented clock rate in permille of a tick (1000 is real time).
    pub time_permille: u32,
}

/// The gamefeel presentation state: shake, bump and time scale together.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct GameFeel {
    /// Camera shake timer.
    pub shake: Shake,
    /// Camera bump offsets.
    pub bump: Bump,
    /// Hit-stop and slow-mo.
    pub time: TimeScale,
}

impl GameFeel {
    /// Post-update for presented tick `ftime`: one [`FeelFrame`].
    pub fn step(&mut self, ftime: u32) -> FeelFrame {
        let (bx, by, zoom) = self.bump.step();
        FeelFrame {
            shake_milli: self.shake.step(ftime),
            bump_milli: (bx, by),
            zoom_milli: zoom,
            time_permille: self.time.step(),
        }
    }
}

#[cfg(test)]
#[expect(
    clippy::disallowed_methods,
    reason = "f64 libm sin/cos is the reference the Q16 tables are checked against"
)]
mod tests {
    use super::*;

    /// Reference sine in f64 for a Q32-turn phase.
    fn ref_sin(phase: u32) -> f64 {
        (f64::from(phase) / 4_294_967_296.0 * std::f64::consts::TAU).sin()
    }

    #[test]
    fn sine_tracks_f64_within_a_part_in_ten_thousand() {
        let mut worst = 0.0_f64;
        for i in 0..4096_u32 {
            let phase = i.wrapping_mul(1 << 20) ^ (i * 7919);
            let got = f64::from(sin_q16(phase)) / 65536.0;
            worst = worst.max((got - ref_sin(phase)).abs());
            let c = f64::from(cos_q16(phase)) / 65536.0;
            let want = (f64::from(phase) / 4_294_967_296.0 * std::f64::consts::TAU).cos();
            worst = worst.max((c - want).abs());
        }
        assert!(worst < 1e-4, "worst sine error {worst}");
        assert_eq!(sin_q16(0), 0);
        assert_eq!(sin_q16(1 << 30), 65536);
        assert_eq!(sin_q16(3 << 30), -65536);
    }

    #[test]
    fn shake_matches_the_reference_formula() {
        // Reference: x = cos(ftime*1.1) * 2.5 * power * ratio, y = sin(0.3 + ftime*1.7) * ..
        let (total, power) = (30_u32, 1.0_f64);
        let mut shake = Shake::default();
        shake.start(total, 1000);
        for ftime in 0..total {
            let ratio = f64::from(total - ftime) / f64::from(total);
            let rx = (f64::from(ftime) * 1.1).cos() * 2.5 * power * ratio;
            let ry = (0.3 + f64::from(ftime) * 1.7).sin() * 2.5 * power * ratio;
            let (x, y) = shake.step(ftime);
            assert!(
                (f64::from(x) / 1000.0 - rx).abs() < 0.003,
                "x {ftime}: {x} vs {rx}"
            );
            assert!(
                (f64::from(y) / 1000.0 - ry).abs() < 0.003,
                "y {ftime}: {y} vs {ry}"
            );
        }
        assert_eq!(shake.step(total), (0, 0));
        assert!(!shake.active());
    }

    #[test]
    fn shake_phase_stays_exact_late_in_a_session() {
        // A ten-minute session: the Q32 phase must not drift off the f64 curve.
        let ftime = 36_000_u32;
        let mut shake = Shake::default();
        shake.start(10, 1000);
        let (x, _) = shake.step(ftime);
        let want = (f64::from(ftime) * 1.1).cos() * 2.5;
        assert!(
            (f64::from(x) / 1000.0 - want).abs() < 0.005,
            "{x} vs {want}"
        );
    }

    #[test]
    fn longer_shake_is_not_cut_short_but_power_updates() {
        let mut shake = Shake::default();
        shake.start(30, 1000);
        shake.start(10, 2000);
        assert_eq!(
            (shake.remaining, shake.total, shake.power_permille),
            (30, 30, 2000)
        );
        shake.start(60, 500);
        assert_eq!((shake.remaining, shake.total), (60, 60));
    }

    #[test]
    fn bump_decays_like_the_reference() {
        let mut bump = Bump::default();
        bump.add(10_000, -4_000);
        let mut reference = 10.0_f64;
        for _ in 0..40 {
            reference *= 0.85;
            let (x, y, _) = bump.step();
            assert!(
                (f64::from(x) / 1000.0 - reference).abs() < 0.001,
                "{x} vs {reference}"
            );
            assert!((f64::from(y) / 1000.0 + reference * 0.4).abs() < 0.001);
        }
        let mut angled = Bump::default();
        angled.add_angle(1 << 30, 8_000);
        angled.step();
        assert_eq!(angled.step().0, 0, "a quarter-turn bump has no x");
        assert!(angled.step().1 > 0);
    }

    #[test]
    fn bump_zoom_replaces_and_decays_by_nine_tenths() {
        let mut bump = Bump::default();
        bump.set_zoom(500);
        bump.set_zoom(200);
        let (_, _, z) = bump.step();
        assert_eq!(z, 180);
    }

    #[test]
    fn hit_stop_holds_four_ticks_at_a_tenth() {
        let mut time = TimeScale::default();
        assert_eq!(time.step(), 1000);
        time.stop_frame();
        let run: Vec<u32> = (0..6).map(|_| time.step()).collect();
        assert_eq!(run, [100, 100, 100, 100, 1000, 1000]);
    }

    #[test]
    fn slow_mo_follows_the_reference_easing() {
        // Reference: target = product of factors; speed eases 0.2 up / 0.6 down;
        // time = (0.2 + 0.8 * speed) * (0.1 while stopped).
        let mut time = TimeScale::default();
        time.add_slow_mo(1, 30, 300).unwrap();
        time.add_slow_mo(2, 10, 500).unwrap();
        let (mut speed, mut left1, mut left2) = (1.0_f64, 30, 10);
        for tick in 0..80 {
            left1 -= 1;
            left2 -= 1;
            let mut target = 1.0;
            if left1 > 0 {
                target *= 0.3;
            }
            if left2 > 0 {
                target *= 0.5;
            }
            speed += (target - speed) * if target > speed { 0.2 } else { 0.6 };
            if (speed - target).abs() <= 0.001 {
                speed = target;
            }
            let want = (0.2 + 0.8 * speed) * 1000.0;
            let got = f64::from(time.step());
            assert!((got - want).abs() <= 3.0, "tick {tick}: {got} vs {want}");
        }
        assert_eq!(time.step(), 1000);
    }

    #[test]
    fn slow_mo_repeat_id_keeps_the_longer_run_and_slots_are_bounded() {
        let mut time = TimeScale::default();
        time.add_slow_mo(7, 20, 300).unwrap();
        time.add_slow_mo(7, 5, 400).unwrap();
        assert_eq!(time.slow[0].unwrap().ticks_left, 20);
        assert_eq!(time.slow[0].unwrap().factor_permille, 400);
        for id in 0..3 {
            time.add_slow_mo(id, 5, 500).unwrap();
        }
        assert_eq!(time.add_slow_mo(99, 5, 500), Err(GameFeelError::SlowMoFull));
    }

    #[test]
    fn frames_replay_identically_without_heap() {
        let run = || {
            let mut feel = GameFeel::default();
            feel.shake.start(20, 1500);
            feel.bump.add_angle(1 << 29, 6_000);
            feel.time.stop_frame();
            feel.time.add_slow_mo(1, 12, 300).unwrap();
            (0..120).map(|t| feel.step(t)).collect::<Vec<_>>()
        };
        assert_eq!(run(), run());
        let mut feel = GameFeel::default();
        feel.shake.start(20, 1500);
        feel.time.add_slow_mo(1, 12, 300).unwrap();
        let ((), heap) = engine_core::alloc_probe::measure(|| {
            for t in 0..120 {
                std::hint::black_box(feel.step(t));
            }
        });
        assert_eq!(heap.calls, 0);
    }
}
