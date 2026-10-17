//! The visible parts of a frame, as pure functions of time so they run and test without a GPU:
//! the clear colour, the moving square, the frame counter line and the canvas backing size.

use std::f64::consts::TAU;
use std::time::Duration;

/// Seconds for the clear colour to go once around the hue wheel.
const HUE_PERIOD_S: f64 = 12.0;

/// Seconds for the square to finish one lap.
const ORBIT_PERIOD_S: f64 = 5.0;

/// Half the square's height in clip space (the width follows the canvas aspect).
const HALF_HEIGHT: f64 = 0.12;

/// An RGB colour with `s` and `v` in 0..=1 and `hue` in turns (wraps).
#[must_use]
pub fn hsv(hue: f64, s: f64, v: f64) -> [f64; 3] {
    let h = hue.rem_euclid(1.0) * 6.0;
    let c = v * s;
    let x = c * (1.0 - ((h % 2.0) - 1.0).abs());
    let (r, g, b) = match h {
        h if h < 1.0 => (c, x, 0.0),
        h if h < 2.0 => (x, c, 0.0),
        h if h < 3.0 => (0.0, c, x),
        h if h < 4.0 => (0.0, x, c),
        h if h < 5.0 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = v - c;
    [r + m, g + m, b + m]
}

/// The clear colour at `t_ms`: a slow hue walk at middle brightness, never near black, so a
/// blank canvas is a visible failure and a screenshot can tell it from the page background.
#[must_use]
pub fn clear_colour(t_ms: f64) -> [f64; 3] {
    hsv(t_ms / 1000.0 / HUE_PERIOD_S, 0.55, 0.62)
}

/// The square's colour: the opposite hue of the clear colour, full brightness.
#[must_use]
pub fn square_colour(t_ms: f64) -> [f64; 3] {
    hsv(t_ms / 1000.0 / HUE_PERIOD_S + 0.5, 0.9, 1.0)
}

/// Where the square is in clip space and what it looks like: what the shader reads.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Square {
    pub centre: [f32; 2],
    pub half: [f32; 2],
    pub rgba: [f32; 4],
}

impl Square {
    /// The square at `t_ms` on a canvas `aspect` (width over height) wide. It stays a square
    /// in pixels and wholly inside the canvas for any aspect.
    #[must_use]
    pub fn at(t_ms: f64, aspect: f64) -> Self {
        let aspect = if aspect.is_finite() && aspect > 0.0 {
            aspect
        } else {
            1.0
        };
        let hy = HALF_HEIGHT;
        let hx = (HALF_HEIGHT / aspect).min(0.5);
        let phase = TAU * (t_ms / 1000.0) / ORBIT_PERIOD_S;
        let cx = (1.0 - hx) * 0.8 * phase.cos();
        let cy = (1.0 - hy) * 0.8 * (phase * 1.3).sin();
        let c = square_colour(t_ms);
        Self {
            centre: [cx as f32, cy as f32],
            half: [hx as f32, hy as f32],
            rgba: [c[0] as f32, c[1] as f32, c[2] as f32, 1.0],
        }
    }

    /// The 32 bytes of the uniform block `shader.wgsl` declares: centre, half size, colour.
    #[must_use]
    pub fn to_bytes(&self) -> [u8; 32] {
        let floats = [
            self.centre[0],
            self.centre[1],
            self.half[0],
            self.half[1],
            self.rgba[0],
            self.rgba[1],
            self.rgba[2],
            self.rgba[3],
        ];
        let mut out = [0u8; 32];
        for (i, f) in floats.iter().enumerate() {
            out[i * 4..i * 4 + 4].copy_from_slice(&f.to_le_bytes());
        }
        out
    }
}

/// Frame counter and frame time from the browser's animation timestamps (milliseconds).
#[derive(Clone, Copy, Debug, Default)]
pub struct FrameStats {
    frames: u64,
    last_now: Option<f64>,
    last_ms: f64,
    smooth_ms: f64,
}

impl FrameStats {
    /// Count a frame that started at `now_ms`. The first frame has no duration yet.
    pub fn tick(&mut self, now_ms: f64) {
        self.frames += 1;
        if let Some(prev) = self.last_now {
            let dt = (now_ms - prev).max(0.0);
            self.last_ms = dt;
            self.smooth_ms = if self.smooth_ms == 0.0 {
                dt
            } else {
                self.smooth_ms * 0.9 + dt * 0.1
            };
        }
        self.last_now = Some(now_ms);
    }

    #[must_use]
    pub fn frames(&self) -> u64 {
        self.frames
    }

    #[must_use]
    pub fn last_ms(&self) -> f64 {
        self.last_ms
    }

    /// The line the page shows: frame number, frame time, rate from the smoothed time.
    #[must_use]
    pub fn line(&self) -> String {
        if self.smooth_ms <= 0.0 {
            return format!("frame {} | -- ms", self.frames);
        }
        format!(
            "frame {} | {:.1} ms | {:.0} fps",
            self.frames,
            self.last_ms,
            1000.0 / self.smooth_ms
        )
    }
}

/// A browser time reading (milliseconds since the page started) as a [`Duration`]. A reading
/// that is not a finite non-negative number counts as zero, never a panic.
#[must_use]
pub fn duration_from_ms(ms: f64) -> Duration {
    if ms.is_finite() && ms > 0.0 {
        Duration::from_secs_f64(ms / 1000.0)
    } else {
        Duration::ZERO
    }
}

/// The canvas backing store size for a CSS box and a device pixel ratio, kept inside what the
/// device can make (`max_dim`) and never zero (a zero surface cannot be configured).
#[must_use]
pub fn backing_size(css_w: f64, css_h: f64, dpr: f64, max_dim: u32) -> (u32, u32) {
    let dpr = if dpr.is_finite() && dpr > 0.0 {
        dpr
    } else {
        1.0
    };
    let side = |css: f64| -> u32 {
        let px = (css * dpr).round();
        if !px.is_finite() || px < 1.0 {
            1
        } else if px > f64::from(max_dim) {
            max_dim
        } else {
            px as u32
        }
    };
    (side(css_w), side(css_h))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn luma(c: [f64; 3]) -> f64 {
        0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
    }

    #[test]
    fn hsv_hits_the_primaries() {
        let near = |a: [f64; 3], b: [f64; 3]| a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-9);
        assert!(near(hsv(0.0, 1.0, 1.0), [1.0, 0.0, 0.0]));
        assert!(near(hsv(1.0 / 3.0, 1.0, 1.0), [0.0, 1.0, 0.0]));
        assert!(near(hsv(2.0 / 3.0, 1.0, 1.0), [0.0, 0.0, 1.0]));
        assert!(near(hsv(1.25, 0.0, 0.4), [0.4, 0.4, 0.4]));
        assert!(near(hsv(-0.5, 1.0, 1.0), hsv(0.5, 1.0, 1.0)));
    }

    #[test]
    fn clear_colour_is_always_visible_and_in_range() {
        let mut moved = false;
        let first = clear_colour(0.0);
        for step in 0..=240 {
            let c = clear_colour(f64::from(step) * 100.0);
            assert!(c.iter().all(|v| (0.0..=1.0).contains(v)), "{c:?}");
            // Never black: the page behind the canvas is black.
            assert!(luma(c) > 0.2, "too dark at step {step}: {c:?}");
            moved |= c != first;
        }
        assert!(moved, "the clear colour never changes");
    }

    #[test]
    fn clear_colour_goes_round_the_wheel_in_one_period() {
        let a = clear_colour(0.0);
        let b = clear_colour(HUE_PERIOD_S * 1000.0);
        assert!(a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-9));
        assert_ne!(a, clear_colour(HUE_PERIOD_S * 500.0));
    }

    #[test]
    fn the_square_differs_from_the_clear_colour() {
        for step in 0..48 {
            let t = f64::from(step) * 250.0;
            let (bg, sq) = (clear_colour(t), square_colour(t));
            let dist: f64 = bg.iter().zip(sq).map(|(a, b)| (a - b).abs()).sum();
            assert!(dist > 0.5, "square hidden at {t} ms: {bg:?} vs {sq:?}");
        }
    }

    #[test]
    fn the_square_stays_a_square_inside_the_canvas_at_any_aspect() {
        for aspect in [0.25, 0.5, 1.0, 16.0 / 9.0, 4.0, 21.0 / 9.0] {
            for step in 0..200 {
                let s = Square::at(f64::from(step) * 50.0, aspect);
                assert!(
                    s.centre[0].abs() + s.half[0] <= 1.0,
                    "x out at aspect {aspect}"
                );
                assert!(
                    s.centre[1].abs() + s.half[1] <= 1.0,
                    "y out at aspect {aspect}"
                );
                // Equal on screen: clip half width times width equals half height times height.
                let on_screen_w = f64::from(s.half[0]) * aspect;
                let on_screen_h = f64::from(s.half[1]);
                if f64::from(s.half[0]) < 0.5 {
                    assert!(
                        (on_screen_w - on_screen_h).abs() < 1e-5,
                        "not square at {aspect}"
                    );
                }
            }
        }
    }

    #[test]
    fn the_square_moves_and_a_bad_aspect_falls_back() {
        assert_ne!(Square::at(0.0, 1.0).centre, Square::at(700.0, 1.0).centre);
        assert_eq!(Square::at(0.0, f64::NAN), Square::at(0.0, 1.0));
        assert_eq!(Square::at(0.0, 0.0), Square::at(0.0, 1.0));
    }

    #[test]
    fn square_bytes_are_the_eight_floats_the_shader_reads() {
        let s = Square::at(1234.0, 1.5);
        let b = s.to_bytes();
        let f = |i: usize| f32::from_le_bytes([b[i * 4], b[i * 4 + 1], b[i * 4 + 2], b[i * 4 + 3]]);
        assert_eq!([f(0), f(1)], s.centre);
        assert_eq!([f(2), f(3)], s.half);
        assert_eq!([f(4), f(5), f(6), f(7)], s.rgba);
    }

    #[test]
    fn frame_stats_count_and_time_frames() {
        let mut s = FrameStats::default();
        assert_eq!(s.line(), "frame 0 | -- ms");
        s.tick(1000.0);
        assert_eq!(s.frames(), 1);
        assert_eq!(s.line(), "frame 1 | -- ms");
        s.tick(1020.0);
        assert_eq!(s.frames(), 2);
        assert!((s.last_ms() - 20.0).abs() < 1e-9);
        assert_eq!(s.line(), "frame 2 | 20.0 ms | 50 fps");
    }

    #[test]
    fn frame_stats_smooth_the_rate_and_survive_a_clock_step_back() {
        let mut s = FrameStats::default();
        let mut t = 0.0;
        for _ in 0..120 {
            s.tick(t);
            t += 10.0;
        }
        assert!(s.line().ends_with("100 fps"), "{}", s.line());
        s.tick(t - 500.0);
        assert_eq!(s.last_ms(), 0.0);
        assert_eq!(s.frames(), 121);
    }

    #[test]
    fn browser_milliseconds_become_a_duration_and_bad_readings_become_zero() {
        assert_eq!(duration_from_ms(1500.0), Duration::from_millis(1500));
        assert_eq!(duration_from_ms(0.25), Duration::from_micros(250));
        for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -3.0, 0.0] {
            assert_eq!(duration_from_ms(bad), Duration::ZERO, "{bad}");
        }
    }

    #[test]
    fn backing_size_scales_clamps_and_never_reaches_zero() {
        assert_eq!(backing_size(800.0, 600.0, 1.0, 8192), (800, 600));
        assert_eq!(backing_size(800.0, 600.0, 2.0, 8192), (1600, 1200));
        assert_eq!(backing_size(800.0, 600.0, 1.25, 8192), (1000, 750));
        assert_eq!(backing_size(0.0, 0.2, 1.0, 8192), (1, 1));
        assert_eq!(backing_size(f64::NAN, -5.0, 1.0, 8192), (1, 1));
        assert_eq!(backing_size(9000.0, 100.0, 2.0, 8192), (8192, 200));
        assert_eq!(backing_size(100.0, 100.0, f64::NAN, 8192), (100, 100));
        assert_eq!(backing_size(100.0, 100.0, 0.0, 8192), (100, 100));
    }
}
