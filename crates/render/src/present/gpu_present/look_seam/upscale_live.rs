//! upscale_live: the live frame's render scale and the upscaler that lands it at output size.
//!
//! Owner: render lane. Consumer: `present_frame_live`. Units: render scale in percent of the
//! output edge, pixels. Determinism: the render size is a pure function of the output size and
//! the scale; nothing here reads a clock. Presentation only, never hashed.
//!
//! Off by default: scale 100 and no upscaler leave the frame exactly as it was (the blit's own
//! bilinear read stretches a smaller render when only a scale is set). A measured run opts in
//! with `E2040_RENDER_SCALE` (percent, 50 to 100), `E2040_UPSCALE` (`fsr1` or `bilinear`) and
//! `E2040_SHARPEN` (0 to 1, the sharpen after the upscale; 0 skips its pass).

use super::super::OUTPUT_FORMAT;
use super::upscale_gpu::{SpanStamps, UpscalePass, UpscaleSized};
use super::LookStage;
use crate::dynres_gov::{LiveDynRes, LiveDynResConfig, LIVE_TARGET_MS};

/// Env switch: render scale in percent.
pub(crate) const RENDER_SCALE_ENV: &str = "E2040_RENDER_SCALE";
/// Env switch: `fsr1` runs the EASU pass, `bilinear` leaves the blit's own stretch.
pub(crate) const UPSCALE_ENV: &str = "E2040_UPSCALE";
/// Env switch: sharpness of the pass after the upscale, `0` to `1`.
pub(crate) const SHARPEN_ENV: &str = "E2040_SHARPEN";

/// Env switch: `1` lets measured GPU ms pick the render scale (see `LiveDynRes`).
pub(crate) const DYNRES_ENV: &str = "E2040_DYNRES";
/// Env switch: the GPU ms budget the picker holds, 1 to 100.
pub(crate) const DYNRES_MS_ENV: &str = "E2040_DYNRES_MS";

/// Lowest render scale the frame accepts, in percent.
pub(crate) const MIN_SCALE_PCT: u8 = 50;
/// Full resolution, in percent.
pub(crate) const FULL_SCALE_PCT: u8 = 100;

/// A render scale from env text: whole percent from 50 to 100, else `None`.
pub(crate) fn parse_scale_pct(text: Option<&str>) -> Option<u8> {
    let pct: u8 = text?.trim().parse().ok()?;
    (MIN_SCALE_PCT..=FULL_SCALE_PCT)
        .contains(&pct)
        .then_some(pct)
}

/// An upscaler choice from env text: `Some(true)` for FSR1, `Some(false)` for bilinear.
pub(crate) fn parse_upscale(text: Option<&str>) -> Option<bool> {
    match text?.trim().to_ascii_lowercase().as_str() {
        "fsr1" | "easu" | "fsr" | "1" | "on" => Some(true),
        "bilinear" | "off" | "0" => Some(false),
        _ => None,
    }
}

/// A sharpen strength from env text: a finite number in `0..=1`, else `None`.
pub(crate) fn parse_sharpen(text: Option<&str>) -> Option<f32> {
    let v: f32 = text?.trim().parse().ok()?;
    (v.is_finite() && (0.0..=1.0).contains(&v)).then_some(v)
}

/// A picker budget from env text: a finite ms count from 1 to 100, else `None`.
pub(crate) fn parse_budget_ms(text: Option<&str>) -> Option<f32> {
    let v: f32 = text?.trim().parse().ok()?;
    (v.is_finite() && (1.0..=100.0).contains(&v)).then_some(v)
}

/// True for `1`, `on` or `true`.
pub(crate) fn parse_flag(text: Option<&str>) -> bool {
    matches!(
        text.map(|t| t.trim().to_ascii_lowercase()).as_deref(),
        Some("1" | "on" | "true")
    )
}

/// The scale and upscaler choice of the live frame, free of any device.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) struct ScaleConfig {
    scale_pct: u8,
    fsr1: bool,
    sharpness: f32,
    /// An env switch set the scale or the upscaler: a quality tier does not override it.
    pinned: bool,
}

impl ScaleConfig {
    /// Full scale, bilinear, no sharpen.
    pub(crate) const OFF: Self = Self {
        scale_pct: FULL_SCALE_PCT,
        fsr1: false,
        sharpness: 0.0,
        pinned: false,
    };

    /// `OFF`, then the env switches (a measured run's opt-in); `env` reads one variable.
    pub(crate) fn from_env(env: impl Fn(&str) -> Option<String>) -> Self {
        let scale = parse_scale_pct(env(RENDER_SCALE_ENV).as_deref());
        let fsr1 = parse_upscale(env(UPSCALE_ENV).as_deref());
        let sharpen = parse_sharpen(env(SHARPEN_ENV).as_deref());
        let off = Self::OFF;
        Self {
            scale_pct: scale.unwrap_or(off.scale_pct),
            fsr1: fsr1.unwrap_or(off.fsr1),
            sharpness: sharpen.unwrap_or(off.sharpness),
            pinned: scale.is_some() || fsr1.is_some(),
        }
    }

    /// Render scale in percent of the output edge.
    pub(crate) fn scale_pct(&self) -> u8 {
        self.scale_pct
    }

    /// Set the render scale (clamped to 50..=100).
    pub(crate) fn set_scale_pct(&mut self, pct: u8) {
        self.scale_pct = pct.clamp(MIN_SCALE_PCT, FULL_SCALE_PCT);
    }

    /// True when the EASU pass runs on a frame that renders below the output.
    pub(crate) fn fsr1(&self) -> bool {
        self.fsr1
    }

    /// Switch the EASU pass.
    pub(crate) fn set_fsr1(&mut self, on: bool) {
        self.fsr1 = on;
    }

    /// Sharpen strength after the upscale (0 skips the pass).
    pub(crate) fn sharpness(&self) -> f32 {
        self.sharpness
    }

    /// Set the sharpen strength, clamped to `0..=1`; a non-finite value reads as 0.
    pub(crate) fn set_sharpness(&mut self, sharpness: f32) {
        self.sharpness = if sharpness.is_finite() {
            sharpness.clamp(0.0, 1.0)
        } else {
            0.0
        };
    }

    /// A quality tier's scale: ignored when an env switch pinned the choice. A scale under 100
    /// turns the EASU pass on (that is what the tier means), 100 turns it off.
    pub(crate) fn apply_tier_scale(&mut self, pct: u8) {
        if self.pinned {
            return;
        }
        self.set_scale_pct(pct);
        self.fsr1 = self.scale_pct < FULL_SCALE_PCT;
    }

    /// Render size for an output of `out_w` by `out_h` (even edges, never below the floor).
    pub(crate) fn render_size(&self, out_w: u32, out_h: u32) -> (u32, u32) {
        if self.scale_pct >= FULL_SCALE_PCT {
            return (out_w, out_h);
        }
        let scale = f32::from(self.scale_pct) / 100.0;
        crate::aa_scale::render_size_for_scale(out_w, out_h, scale).unwrap_or((out_w, out_h))
    }

    /// True when a frame that renders at `render` for an output of `out` runs the EASU chain.
    pub(crate) fn active(&self, render: (u32, u32), out: (u32, u32)) -> bool {
        self.fsr1 && render != out && render.0 > 0 && render.1 > 0
    }
}

/// The config plus the pass resources of the current sizes.
pub(crate) struct UpscaleLive {
    pass: UpscalePass,
    sized: Option<UpscaleSized>,
    /// The scale and upscaler choice; the presenter's setters write it.
    pub(crate) config: ScaleConfig,
    /// This frame renders below the output and the upscale chain runs.
    armed: bool,
    /// Frames the chain ran, for the receipt.
    armed_frames: u32,
    /// Measured GPU ms pick the scale when set (`E2040_DYNRES=1`).
    dynres: Option<LiveDynRes>,
    /// Resolved-frame count of the last GPU sample the picker took.
    last_seen: usize,
}

impl UpscaleLive {
    /// Build the pass objects; the config reads the env switches.
    pub(crate) fn new(device: &wgpu::Device) -> Self {
        let env = |name: &str| std::env::var(name).ok();
        let mut config = ScaleConfig::from_env(env);
        let dynres = parse_flag(env(DYNRES_ENV).as_deref()).then(|| {
            // A picked scale below 100 needs the upscaler, unless the run pinned bilinear.
            if parse_upscale(env(UPSCALE_ENV).as_deref()) != Some(false) {
                config.set_fsr1(true);
            }
            LiveDynRes::new(LiveDynResConfig {
                target_ms: parse_budget_ms(env(DYNRES_MS_ENV).as_deref()).unwrap_or(LIVE_TARGET_MS),
                ..LiveDynResConfig::default()
            })
        });
        Self {
            pass: UpscalePass::new(device),
            sized: None,
            config,
            armed: false,
            armed_frames: 0,
            dynres,
            last_seen: 0,
        }
    }

    /// Feed the newest resolved GPU ms to the picker (when on) and take its scale for the next
    /// frame. A sample is used once: `latest` carries the resolved-frame count.
    pub(crate) fn tick_dynres(&mut self, latest: Option<(usize, f32)>) {
        let (Some(ctl), Some((count, ms))) = (self.dynres.as_mut(), latest) else {
            return;
        };
        if count == self.last_seen {
            return;
        }
        self.last_seen = count;
        let pct = ctl.observe(ms);
        self.config.set_scale_pct(pct);
    }

    /// The run's upscale and picker receipt, or `None` when both are off and never ran.
    pub(crate) fn receipt_line(&self) -> Option<String> {
        if !self.config.fsr1() && self.dynres.is_none() {
            return None;
        }
        let mut line = format!(
            "upscale: fsr1={} scale_pct={} sharpen={:.2} chain_frames={}",
            if self.config.fsr1() { "on" } else { "off" },
            self.config.scale_pct(),
            self.config.sharpness(),
            self.armed_frames,
        );
        if let Some(ctl) = &self.dynres {
            line.push('\n');
            line.push_str(&ctl.receipt_line());
        }
        Some(line)
    }

    /// Get the pass resources ready for this frame (rebuilt only when a size or the sharpness
    /// moved) and arm or disarm the chain. Call before the frame borrows its targets.
    pub(crate) fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        surface: bool,
        render: (u32, u32),
        out: (u32, u32),
        want_final: bool,
    ) {
        self.armed = surface && self.config.active(render, out);
        if !self.armed {
            return;
        }
        self.armed_frames = self.armed_frames.saturating_add(1);
        let sharpness = self.config.sharpness();
        let ready = self
            .sized
            .as_ref()
            .is_some_and(|s| s.matches(render, out, sharpness, want_final));
        if !ready {
            self.sized = Some(
                self.pass
                    .sized(device, queue, render, out, sharpness, want_final),
            );
        }
    }

    /// The chain for this frame over the look stage the frame planned, or `None` when the frame
    /// runs at the output size.
    pub(crate) fn route<'a>(&'a self, stage: LookStage<'a>) -> Option<UpscaleRoute<'a>> {
        if !self.armed {
            return None;
        }
        let sized = self.sized.as_ref()?;
        let next = if stage.display.is_some() {
            LookStage {
                blit: sized.final_view()?,
                format: OUTPUT_FORMAT,
                ..stage
            }
        } else {
            stage
        };
        Some(UpscaleRoute {
            pass: &self.pass,
            sized,
            stage: next,
        })
    }
}

/// One frame's upscale chain: where the blit lands, and the stage the overlays and the look pass
/// continue with once the chain has written the output-size picture.
pub(crate) struct UpscaleRoute<'a> {
    pass: &'a UpscalePass,
    sized: &'a UpscaleSized,
    /// The stage after the chain: its `blit` is the output-size target.
    pub(crate) stage: LookStage<'a>,
}

impl<'a> UpscaleRoute<'a> {
    /// The render-size target the blit writes (display bytes).
    pub(crate) fn lo(&self) -> &'a wgpu::TextureView {
        self.sized.lo_view()
    }

    /// Record EASU (and the sharpen) from the render-size target into the output-size target.
    pub(crate) fn encode(
        &self,
        device: &wgpu::Device,
        enc: &mut wgpu::CommandEncoder,
        stamps: SpanStamps<'_>,
    ) {
        self.sized.encode(
            self.pass,
            device,
            enc,
            self.stage.blit,
            self.stage.format,
            stamps,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn no_env(_: &str) -> Option<String> {
        None
    }

    #[test]
    fn scale_text_is_a_whole_percent_from_50_to_100() {
        assert_eq!(parse_scale_pct(Some("67")), Some(67));
        assert_eq!(parse_scale_pct(Some(" 100 ")), Some(100));
        assert_eq!(parse_scale_pct(Some("50")), Some(50));
        for bad in ["49", "101", "0.67", "", "abc", "-5"] {
            assert_eq!(parse_scale_pct(Some(bad)), None, "{bad:?}");
        }
        assert_eq!(parse_scale_pct(None), None);
    }

    #[test]
    fn upscale_text_names_fsr1_or_bilinear() {
        for on in ["fsr1", "FSR1", "easu", "1", "on"] {
            assert_eq!(parse_upscale(Some(on)), Some(true), "{on:?}");
        }
        for off in ["bilinear", "off", "0"] {
            assert_eq!(parse_upscale(Some(off)), Some(false), "{off:?}");
        }
        assert_eq!(parse_upscale(Some("dlss")), None);
        assert_eq!(parse_upscale(None), None);
    }

    #[test]
    fn sharpen_text_is_a_unit_number() {
        assert_eq!(parse_sharpen(Some("0")), Some(0.0));
        assert_eq!(parse_sharpen(Some("0.4")), Some(0.4));
        for bad in ["1.5", "-0.1", "NaN", "inf", "x"] {
            assert_eq!(parse_sharpen(Some(bad)), None, "{bad:?}");
        }
    }

    #[test]
    fn picker_switches_read_a_flag_and_a_budget() {
        for on in ["1", "on", "TRUE", " true "] {
            assert!(parse_flag(Some(on)), "{on:?}");
        }
        for off in ["0", "off", "", "yes"] {
            assert!(!parse_flag(Some(off)), "{off:?}");
        }
        assert!(!parse_flag(None));
        assert_eq!(parse_budget_ms(Some("8")), Some(8.0));
        assert_eq!(parse_budget_ms(Some("16.6")), Some(16.6));
        for bad in ["0", "0.5", "101", "NaN", "x", ""] {
            assert_eq!(parse_budget_ms(Some(bad)), None, "{bad:?}");
        }
    }

    #[test]
    fn the_default_frame_is_untouched() {
        let c = ScaleConfig::from_env(no_env);
        assert_eq!(c, ScaleConfig::OFF);
        assert_eq!(c.render_size(1920, 1080), (1920, 1080));
        assert!(!c.active((1920, 1080), (1920, 1080)));
    }

    #[test]
    fn env_switches_set_scale_and_pin_it_against_a_tier() {
        let env = |name: &str| match name {
            RENDER_SCALE_ENV => Some("67".to_string()),
            UPSCALE_ENV => Some("fsr1".to_string()),
            SHARPEN_ENV => Some("0.3".to_string()),
            _ => None,
        };
        let mut c = ScaleConfig::from_env(env);
        assert_eq!((c.scale_pct(), c.fsr1(), c.sharpness()), (67, true, 0.3));
        c.apply_tier_scale(100);
        assert_eq!(c.scale_pct(), 67, "a pinned choice outranks a tier");
    }

    #[test]
    fn a_tier_scale_turns_the_upscaler_on_below_100_and_off_at_100() {
        let mut c = ScaleConfig::OFF;
        c.apply_tier_scale(67);
        assert_eq!((c.scale_pct(), c.fsr1()), (67, true));
        c.apply_tier_scale(100);
        assert_eq!((c.scale_pct(), c.fsr1()), (100, false));
        c.apply_tier_scale(10);
        assert_eq!(c.scale_pct(), MIN_SCALE_PCT, "the floor holds");
    }

    #[test]
    fn render_size_follows_the_scale_with_even_edges() {
        let mut c = ScaleConfig::OFF;
        c.apply_tier_scale(67);
        let (rw, rh) = c.render_size(1920, 1080);
        assert!(
            (1280..=1290).contains(&rw) && (720..=728).contains(&rh),
            "{rw}x{rh}"
        );
        for (w, h) in [(1920, 1061), (1366, 768), (801, 451)] {
            let (rw, rh) = c.render_size(w, h);
            assert!(
                rw % 2 == 0 && rh % 2 == 0 && rw <= w && rh <= h,
                "{w}x{h} -> {rw}x{rh}"
            );
        }
        assert!(c.active(c.render_size(1920, 1080), (1920, 1080)));
        c.set_fsr1(false);
        assert!(!c.active(c.render_size(1920, 1080), (1920, 1080)));
    }
}
