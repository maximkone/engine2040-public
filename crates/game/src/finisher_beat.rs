//! finisher_beat: the screen-wide white flash on a killing blow (row SW22).
//!
//! A killing blow earns a 4-tick beat that freezes the frame, then a
//! full-screen white overlay at 0.9 alpha fades over 5 ticks: the screen is
//! white on the beat tick and exactly clear on beat + 9. Donor: Kelpekk/Juicee
//! `addons/juicee/effects/freeze_frame_effect.gd` `_apply` (0.06 s freeze,
//! flash 0.9, fade 0.08 s), read 2026-10-24; 0.06 s is four ticks at 60 Hz and
//! 0.08 s is five, which is where the row's numbers land.
//!
//! Pure and integer: no tick, no renderer, no heap. The caller strikes the
//! beat when a fatal `FightEvent` lands, steps it once per tick, and reads the
//! alpha to composite the overlay. Presentation only: the sim never reads
//! this, the tick count and the golden `d57b5c8ae3096614` never move. Alpha is
//! permille here and one float at the edge ([`FinisherBeat::alpha`]).
//! Determinism: integer counters; the same strike tick gives the same fade.
//! Errors: none; every function is total.
//!
//! SW174 rides the same beat: [`Letterbox`] eases two black bars in over six
//! ticks from the strike and out over nine after the beat ends, at 8% of the
//! height each. The caller draws them as two HUD quads ([`Letterbox::quads`]),
//! so the post chain gains no pass; [`Letterbox::paint`] is the CPU twin the
//! accept test reads rows from.
//!
//! SW173 rides it too: [`FinisherBeat::dof_focus_m`] gives the lens the impact
//! point's depth while the beat is live, and `None` once the frame is sharp.

/// Ticks the beat freezes the frame (row: "a 4-tick beat").
pub const BEAT_TICKS: u32 = 4;
/// Overlay alpha at full flash, in permille (row: 0.9 alpha).
pub const FLASH_MILLI: u32 = 900;
/// Ticks the flash fades over once the beat ends (row: fades over 5 ticks).
pub const FADE_TICKS: u32 = 5;
/// Ticks from the strike to a clear screen: the row's "exactly 0 by beat+9".
pub const BEAT_END_TICKS: u32 = BEAT_TICKS + FADE_TICKS;
/// The overlay's own color: full-screen white (row).
pub const OVERLAY_RGB: [u8; 3] = [255, 255, 255];

/// The finisher beat and its white overlay, as view state the caller owns.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FinisherBeat {
    age: u32,
    live: bool,
}

impl FinisherBeat {
    /// No beat: the screen is clear.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            age: 0,
            live: false,
        }
    }

    /// A killing blow landed on this tick: start the beat, or restart it when
    /// a second finisher lands inside the fade.
    pub fn strike(&mut self) {
        self.age = 0;
        self.live = true;
    }

    /// One tick of the beat. The beat goes clear on the tick it reaches
    /// [`BEAT_END_TICKS`], so the same strike always clears at the same tick.
    pub fn step(&mut self) {
        if !self.live {
            return;
        }
        self.age = self.age.saturating_add(1);
        if self.age >= BEAT_END_TICKS {
            self.age = 0;
            self.live = false;
        }
    }

    /// Overlay alpha now, in permille: full 0.9 through the beat, a linear
    /// ramp down over the fade, zero once clear.
    #[must_use]
    pub const fn alpha_milli(&self) -> u32 {
        if !self.live {
            return 0;
        }
        if self.age < BEAT_TICKS {
            return FLASH_MILLI;
        }
        let faded = self.age - BEAT_TICKS;
        if faded >= FADE_TICKS {
            return 0;
        }
        FLASH_MILLI * (FADE_TICKS - faded) / FADE_TICKS
    }

    /// The overlay alpha as the renderer wants it, `0.0..=0.9`: the one float
    /// at the presentation edge, never hashed.
    #[must_use]
    pub fn alpha(&self) -> f32 {
        let milli = u16::try_from(self.alpha_milli()).unwrap_or(0);
        f32::from(milli) / 1000.0
    }

    /// True while the beat freezes the frame: the first [`BEAT_TICKS`] ticks.
    #[must_use]
    pub const fn holds(&self) -> bool {
        self.live && self.age < BEAT_TICKS
    }

    /// True while the screen is not clear (beat or fade still running).
    #[must_use]
    pub const fn live(&self) -> bool {
        self.live
    }

    /// Ticks since the strike ([`BEAT_END_TICKS`] or more reads as clear).
    #[must_use]
    pub const fn age(&self) -> u32 {
        self.age
    }
}

impl FinisherBeat {
    /// Focus depth for the current frame: the impact point's distance while the
    /// beat is live, `None` before a strike and once the screen is clear, so
    /// the lens is sharp again after the beat (SW173).
    #[must_use]
    pub fn dof_focus_m(&self, eye: [f32; 3], forward: [f32; 3], impact: [f32; 3]) -> Option<f32> {
        if self.live {
            impact_focus_m(eye, forward, impact)
        } else {
            None
        }
    }
}

/// Distance in metres from `eye` to `point` along the camera axis: the focus
/// depth of a depth-of-field lens looking down `forward` (SW173). Sideways
/// offset never changes it; a point behind the lens or a degenerate axis gives
/// `None`, so a caller never focuses on something it cannot see. Pure; a finite
/// input gives a finite answer.
#[must_use]
pub fn impact_focus_m(eye: [f32; 3], forward: [f32; 3], point: [f32; 3]) -> Option<f32> {
    let [fx, fy, fz] = forward;
    let len = (fx * fx + fy * fy + fz * fz).sqrt();
    if !len.is_finite() || len <= 1e-6 {
        return None;
    }
    let along =
        ((point[0] - eye[0]) * fx + (point[1] - eye[1]) * fy + (point[2] - eye[2]) * fz) / len;
    if along > 0.0 && along.is_finite() {
        Some(along)
    } else {
        None
    }
}

/// The letterbox's own color: full black (SW174).
pub const LETTERBOX_RGB: [u8; 3] = [0, 0, 0];
/// Bar height at full, in permille of the frame height: the row's 8% each.
pub const LETTERBOX_FULL_PERMILLE: u32 = 80;
/// Ticks the bars ease in after the strike (row: eased in over 6 ticks).
pub const LETTERBOX_IN_TICKS: u32 = 6;
/// Ticks the bars ease out after the beat ends (row: eased out over 9).
pub const LETTERBOX_OUT_TICKS: u32 = 9;
/// Ticks from the strike to a clear frame: the beat, then the out ramp.
pub const LETTERBOX_END_TICKS: u32 = BEAT_END_TICKS + LETTERBOX_OUT_TICKS;

/// Smoothstep in permille: `t` of 1000 maps to `t^2 (3000 - 2t) / 10^6`, so
/// the ramp starts and ends flat and an in-eased bar never snaps. Integer
/// only; the widest term is `10^9`, inside `u32`.
fn ease_permille(t: u32) -> u32 {
    let x = t.min(1000);
    x.saturating_mul(x).saturating_mul(3000 - 2 * x) / 1_000_000
}

/// The finisher's letterbox bars: two black rectangles, 8% of the height
/// each, eased in over [`LETTERBOX_IN_TICKS`] from the strike and out over
/// [`LETTERBOX_OUT_TICKS`] after the beat ends. Presentation only, never read
/// by the sim; the caller pushes [`Self::quads`] through the HUD quad ring, so
/// the post chain gains no pass.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Letterbox {
    age: u32,
    live: bool,
}

impl Letterbox {
    /// No beat: both bars are gone.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            age: 0,
            live: false,
        }
    }

    /// A finisher landed on this tick: start the bars, or restart them when a
    /// second finisher lands before they clear.
    pub fn strike(&mut self) {
        self.age = 0;
        self.live = true;
    }

    /// One tick of the bars. The frame is clear on the tick the ramp reaches
    /// [`LETTERBOX_END_TICKS`], so the same strike always clears alike.
    pub fn step(&mut self) {
        if !self.live {
            return;
        }
        self.age = self.age.saturating_add(1);
        if self.age >= LETTERBOX_END_TICKS {
            self.age = 0;
            self.live = false;
        }
    }

    /// Bar height now, in permille of the frame height: the eased in ramp, the
    /// full 8% through the beat, then the eased out ramp, zero once clear.
    #[must_use]
    pub fn height_permille(&self) -> u32 {
        if !self.live {
            return 0;
        }
        if self.age < LETTERBOX_IN_TICKS {
            let ramp = self.age * 1000 / LETTERBOX_IN_TICKS;
            return LETTERBOX_FULL_PERMILLE * ease_permille(ramp) / 1000;
        }
        if self.age < BEAT_END_TICKS {
            return LETTERBOX_FULL_PERMILLE;
        }
        let out = self.age - BEAT_END_TICKS;
        if out >= LETTERBOX_OUT_TICKS {
            return 0;
        }
        let ramp = (LETTERBOX_OUT_TICKS - out) * 1000 / LETTERBOX_OUT_TICKS;
        LETTERBOX_FULL_PERMILLE * ease_permille(ramp) / 1000
    }

    /// Bar height in whole rows for one frame height.
    #[must_use]
    pub fn height_px(&self, frame_height: u32) -> u32 {
        frame_height * self.height_permille() / 1000
    }

    /// The two bar row spans as `[start, end)` pairs for one frame height:
    /// the top bar, then the bottom bar. Both are empty when the bars are out.
    #[must_use]
    pub fn bars_px(&self, frame_height: u32) -> [(u32, u32); 2] {
        let bar = self.height_px(frame_height);
        [(0, bar), (frame_height.saturating_sub(bar), frame_height)]
    }

    /// The two bars as HUD quads in the overlay's 0..1 space, top edge down:
    /// `[x0, y0, x1, y1]` each, the top bar then the bottom one.
    #[must_use]
    pub fn quads(&self) -> [[f32; 4]; 2] {
        let milli = u16::try_from(self.height_permille()).unwrap_or(0);
        let h = f32::from(milli) / 1000.0;
        [[0.0, 0.0, 1.0, h], [0.0, 1.0 - h, 1.0, 1.0]]
    }

    /// Paint the bars onto an RGB frame of `width` by `height` pixels and
    /// return the rows painted (both bars). The CPU twin of the HUD quads: it
    /// touches the bar rows only, so every other pixel keeps its byte.
    pub fn paint(&self, frame: &mut [u8], width: u32, height: u32) -> u32 {
        let (top, bottom) = {
            let [top, bottom] = self.bars_px(height);
            (top, bottom)
        };
        let stride = usize::try_from(width).unwrap_or(0).saturating_mul(3);
        let rows = usize::try_from(height).unwrap_or(0);
        let mut painted = 0u32;
        for (start, end) in [top, bottom] {
            for row in
                usize::try_from(start).unwrap_or(0)..usize::try_from(end).unwrap_or(0).min(rows)
            {
                let at = row * stride;
                if let Some(span) = frame.get_mut(at..at + stride) {
                    for pixel in span.chunks_exact_mut(3) {
                        pixel.copy_from_slice(&LETTERBOX_RGB);
                    }
                    painted = painted.saturating_add(1);
                }
            }
        }
        painted
    }

    /// True while the bars are on screen (in ramp, beat or out ramp).
    #[must_use]
    pub const fn live(&self) -> bool {
        self.live
    }

    /// Ticks since the strike ([`LETTERBOX_END_TICKS`] or more reads as clear).
    #[must_use]
    pub const fn age(&self) -> u32 {
        self.age
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finisher_overlay_flashes_white_and_fades() {
        // SW22: white on the beat tick, clear on beat + 9, on a killing blow.
        let mut beat = FinisherBeat::new();
        assert_eq!(
            (beat.live(), beat.alpha_milli()),
            (false, 0),
            "a clear screen before any finisher"
        );
        beat.strike();
        // Beat ticks 0..4: full flash, frame held.
        for tick in 0..BEAT_TICKS {
            assert_eq!(beat.alpha_milli(), FLASH_MILLI, "white on tick {tick}");
            assert!(beat.alpha_milli() > 0, "the screen is white on the beat");
            assert!(beat.holds(), "the beat holds the frame at {tick}");
            assert_eq!(beat.age(), tick);
            if tick + 1 < BEAT_TICKS {
                beat.step();
            }
        }
        assert_eq!(beat.alpha(), 0.9, "the overlay is 0.9 alpha at full flash");
        // The beat ends and the fade runs five ticks, never rising.
        beat.step();
        assert_eq!((beat.live(), beat.holds()), (true, false), "fade, no hold");
        assert_eq!(beat.alpha_milli(), 900, "the fade starts at full flash");
        let mut last = beat.alpha_milli();
        for offset in 1..FADE_TICKS {
            beat.step();
            let now = beat.alpha_milli();
            assert!(now < last, "fade {offset} steps down: {last} -> {now}");
            last = now;
        }
        assert_eq!(
            beat.alpha_milli(),
            180,
            "one fifth left on the last fade tick"
        );
        // Beat + 9: exactly clear, and it stays clear.
        beat.step();
        assert_eq!(beat.age(), 0, "the beat is over");
        assert_eq!(beat.alpha_milli(), 0, "exactly 0 by beat + 9");
        assert!(!beat.live());
        assert!(!beat.holds());
        beat.step();
        assert_eq!(beat.alpha_milli(), 0, "a clear screen stays clear");
    }

    #[test]
    fn no_killing_blow_means_no_overlay() {
        // Nothing strikes, nothing flashes: stepping alone never lights it up.
        let mut beat = FinisherBeat::new();
        for _ in 0..20 {
            beat.step();
            assert_eq!(beat.alpha_milli(), 0);
            assert!(!beat.live());
        }
        // The overlay is the row's white, full screen, at the row's numbers.
        assert_eq!(OVERLAY_RGB, [255, 255, 255], "the overlay is white");
        assert_eq!(
            (BEAT_TICKS, FADE_TICKS, FLASH_MILLI),
            (4, 5, 900),
            "the row's 4-tick beat, 5-tick fade and 0.9 alpha"
        );
        assert_eq!(BEAT_END_TICKS, 9, "clear on beat + 9");
        assert_eq!(beat.alpha(), 0.0);
    }

    #[test]
    fn a_second_finisher_restarts_the_beat_from_white() {
        // Two killing blows inside one fade: the second starts a fresh beat,
        // never a stretched one, so the clear tick stays a fixed distance out.
        let mut beat = FinisherBeat::new();
        beat.strike();
        for _ in 0..BEAT_TICKS {
            beat.step();
        }
        assert_eq!(beat.alpha_milli(), 900, "mid-fade");
        beat.strike();
        assert_eq!(beat.alpha_milli(), FLASH_MILLI, "back to full white");
        assert_eq!(beat.age(), 0);
        assert!(beat.holds(), "the new beat holds the frame again");
        for _ in 0..BEAT_END_TICKS {
            beat.step();
        }
        assert_eq!(beat.alpha_milli(), 0, "clear again nine ticks later");
    }

    /// SW174 accept: at the beat's peak the bar rows read black, and outside
    /// the beat the frame keeps every byte it had.
    #[test]
    fn letterbox_bar_rows_read_black_at_the_peak_and_leave_the_frame_alone() {
        let (width, height) = (4u32, 100u32);
        let clear: Vec<u8> = vec![128u8; (width * height * 3) as usize];
        let mut bars = Letterbox::new();
        let mut frame = clear.clone();
        assert_eq!(
            bars.paint(&mut frame, width, height),
            0,
            "no bars before a finisher"
        );
        assert_eq!(frame, clear, "a clear beat leaves the frame byte for byte");

        bars.strike();
        for _ in 0..LETTERBOX_IN_TICKS {
            bars.step();
        }
        assert_eq!(
            bars.height_permille(),
            LETTERBOX_FULL_PERMILLE,
            "8% at the peak"
        );
        assert_eq!(
            bars.bars_px(height),
            [(0, 8), (92, 100)],
            "8 rows each on 100"
        );
        let mut frame = clear.clone();
        assert_eq!(bars.paint(&mut frame, width, height), 16, "sixteen rows");
        for row in 0..height as usize {
            let span = &frame[row * 12..(row + 1) * 12];
            let want = if row < 8 || row >= 92 {
                [0u8; 3]
            } else {
                [128u8; 3]
            };
            for pixel in span.chunks_exact(3) {
                assert_eq!(pixel, want, "row {row} at the beat's peak");
            }
        }
        // Run the out ramp: every bar row gives the frame back.
        for _ in 0..LETTERBOX_END_TICKS {
            bars.step();
        }
        let mut frame = clear.clone();
        assert_eq!(bars.paint(&mut frame, width, height), 0);
        assert_eq!(frame, clear, "the frame is untouched outside the beat");
        assert_eq!(LETTERBOX_RGB, [0, 0, 0], "the bars are black");
    }

    /// SW174: the bars ease in over six ticks and out over nine, and the same
    /// strike replays the same heights.
    #[test]
    fn letterbox_eases_in_over_six_and_out_over_nine() {
        let ramp = || {
            let mut bars = Letterbox::new();
            bars.strike();
            let mut heights = vec![bars.height_permille()];
            for _ in 0..LETTERBOX_END_TICKS {
                bars.step();
                heights.push(bars.height_permille());
            }
            heights
        };
        let heights = ramp();
        assert_eq!(*heights.first().unwrap(), 0, "no bar on the strike tick");
        for tick in 1..=LETTERBOX_IN_TICKS as usize {
            assert!(
                heights[tick] >= heights[tick - 1],
                "in ramp rises at tick {tick}: {heights:?}"
            );
        }
        assert!(
            heights[1] > 0 && heights[1] < heights[2],
            "the in ramp is eased, never a step: {heights:?}"
        );
        assert_eq!(
            heights[LETTERBOX_IN_TICKS as usize], LETTERBOX_FULL_PERMILLE,
            "full 8% after six ticks"
        );
        for tick in (BEAT_END_TICKS as usize)..heights.len() {
            assert!(
                heights[tick] <= heights[tick - 1],
                "out ramp falls at tick {tick}: {heights:?}"
            );
        }
        assert_eq!(*heights.last().unwrap(), 0, "clear at the end");
        assert_eq!(heights, ramp(), "the bars replay alike");
    }

    /// SW174: the bars are two HUD quads, 8% each at the peak, full width.
    #[test]
    fn letterbox_quads_cover_eight_percent_each_at_the_peak() {
        let mut bars = Letterbox::new();
        bars.strike();
        for _ in 0..LETTERBOX_IN_TICKS {
            bars.step();
        }
        let [top, bottom] = bars.quads();
        assert!((top[3] - 0.08).abs() < 1e-6, "top bar is 8% tall: {top:?}");
        assert!(
            (bottom[1] - 0.92).abs() < 1e-6,
            "bottom bar starts at 92%: {bottom:?}"
        );
        assert_eq!(
            (top[0], top[2], bottom[0], bottom[2]),
            (0.0, 1.0, 0.0, 1.0),
            "both bars span the width"
        );
        assert_eq!(
            (LETTERBOX_IN_TICKS, LETTERBOX_OUT_TICKS),
            (6, 9),
            "the row's ramp ticks"
        );
        assert_eq!(LETTERBOX_END_TICKS, 18, "beat plus the out ramp");
        // Out of the beat both quads are flat, so a HUD push draws nothing.
        assert_eq!(Letterbox::new().quads()[0][3], 0.0);
    }
}

/// SW173: the depth of field focuses on the impact point, and only while the
/// beat holds it.
#[cfg(test)]
mod dof_tests {
    use super::*;

    const EYE: [f32; 3] = [0.0, 1.6, -4.0];
    const FORWARD: [f32; 3] = [0.0, 0.0, 1.0];

    #[test]
    fn impact_point_sets_the_focus_depth_in_metres() {
        // A point 10 m down the camera axis focuses at 10 m, whatever the
        // sideways offset.
        assert_eq!(impact_focus_m(EYE, FORWARD, [0.0, 1.6, 6.0]), Some(10.0));
        assert_eq!(impact_focus_m(EYE, FORWARD, [3.0, 1.6, 6.0]), Some(10.0));
        // Behind the lens, or a degenerate axis, no focus at all.
        assert_eq!(impact_focus_m(EYE, FORWARD, [0.0, 1.6, -9.0]), None);
        assert_eq!(impact_focus_m(EYE, [0.0, 0.0, 0.0], [0.0, 0.0, 6.0]), None);
    }

    #[test]
    fn dof_focus_exists_only_while_the_beat_holds() {
        let mut beat = FinisherBeat::new();
        let impact = [0.0, 1.6, 6.0];
        assert_eq!(
            beat.dof_focus_m(EYE, FORWARD, impact),
            None,
            "no strike, no focus"
        );
        beat.strike();
        assert_eq!(
            beat.dof_focus_m(EYE, FORWARD, impact),
            Some(10.0),
            "the strike focuses on the impact point"
        );
        for _ in 0..BEAT_END_TICKS + 1 {
            beat.step();
        }
        assert_eq!(
            beat.dof_focus_m(EYE, FORWARD, impact),
            None,
            "the frame is sharp again after the beat"
        );
    }
}
