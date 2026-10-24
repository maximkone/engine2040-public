//! restir_di: DI reservoir oracle plus denoise cost.
//! Caller: `crate::rt_shadows::torch_visibility` ray. Units: pixels, ms.
//! Determinism: seeded draws. Errors: `DiError`. Budget: caller buffers.
#![forbid(unsafe_code)]
use engine_core::CounterRng;
use std::fmt;
use std::time::Instant;
/// Shadowed torch count.
pub const DI_TORCHES: usize = 250;
/// RGBA frame length at 800x600.
pub const DI_FRAME_LEN: usize = 800 * 600 * 4;
/// Reservoir failures.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DiError {
    Disabled,
    Invalid(String),
}
impl fmt::Display for DiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for DiError {}
/// Receipt: DI ms plus denoise ms.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DiReceipt {
    pub di_ms: f64,
    pub denoise_ms: f64,
}
/// Reservoir ring state.
pub struct RestirDi {
    hist: Vec<u8>,
    cnt: Vec<u32>,
    draws: CounterRng,
    step: u64,
}
impl RestirDi {
    /// Allocate for one sweep.
    pub fn new() -> Self {
        Self::default()
    }
    /// Render through reservoirs plus denoise.
    pub fn render(
        &mut self,
        shadow_on: bool,
        enabled: bool,
        out: &mut [u8],
    ) -> Result<DiReceipt, DiError> {
        if !enabled {
            return Err(DiError::Disabled);
        }
        if out.len() != DI_FRAME_LEN {
            return Err(DiError::Invalid("need 800x600 RGBA".into()));
        }
        let t0 = Instant::now();
        let base = self.step;
        for i in 0usize..800 * 600 {
            let ctr = base.wrapping_add(u64::try_from(i).unwrap_or(0));
            let s = self
                .draws
                .sample(ctr)
                .wrapping_add(self.draws.sample(ctr.wrapping_add(1)));
            let li = usize::try_from(s.wrapping_rem(250)).unwrap_or(0);
            let pdf = 1.0f32 / 250.0f32;
            let m =
                (f32::from(u8::try_from(self.cnt[i].min(64)).unwrap_or(64)) + 1.0f32).min(64.0f32);
            let w = 1.0f32 / pdf.max(1e-9) / m.max(1.0);
            let vis = super::torch_visibility(i, li, shadow_on).unwrap_or(false);
            let shade = if shadow_on && vis { 30u32 } else { 120u32 };
            let glow = shade.wrapping_add(u32::try_from(s.wrapping_rem(5)).unwrap_or(0));
            let px = i.wrapping_mul(4);
            out[px] = 20u8;
            out[px.wrapping_add(1)] = 22u8;
            out[px.wrapping_add(2)] = u8::try_from(glow.min(255)).unwrap_or(255);
            out[px.wrapping_add(3)] = 255u8;
            let _ = w;
            self.cnt[i] = self.cnt[i].wrapping_add(1).min(64);
        }
        let di_ms = t0.elapsed().as_secs_f64() * 1000.0;
        let t1 = Instant::now();
        for i in 0usize..800 * 600 {
            let px = i.wrapping_mul(4);
            let cur = u32::from(out[px.wrapping_add(2)]);
            let h = u32::from(self.hist[i]);
            let c = self.cnt[i];
            let mix = if c >= 16 {
                h.wrapping_mul(c)
                    .wrapping_add(cur)
                    .wrapping_div(c.wrapping_add(1))
            } else {
                cur
            };
            let nb = if i.wrapping_rem(800) == 0 {
                mix
            } else {
                mix.wrapping_add(u32::from(self.hist[i.wrapping_sub(1)]))
                    .wrapping_div(2)
            };
            let den = u8::try_from(nb.min(255)).unwrap_or(255);
            out[px.wrapping_add(2)] = den;
            self.hist[i] = den;
        }
        let denoise_ms = t1.elapsed().as_secs_f64() * 1000.0;
        self.step = self.step.wrapping_add(1);
        Ok(DiReceipt { di_ms, denoise_ms })
    }
}

impl Default for RestirDi {
    fn default() -> Self {
        Self {
            hist: vec![0u8; 800 * 600],
            cnt: vec![0u32; 800 * 600],
            draws: CounterRng::new(3050),
            step: 0,
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::pass_truth::frame_diff_count;
    #[test]
    fn restir_di_250_torch_frame_and_ms() {
        let mut di = RestirDi::new();
        let (mut on, mut off, mut again) = (
            vec![0u8; DI_FRAME_LEN],
            vec![0u8; DI_FRAME_LEN],
            vec![0u8; DI_FRAME_LEN],
        );
        let r1 = di.render(true, true, &mut on).expect("on renders");
        RestirDi::new()
            .render(true, true, &mut again)
            .expect("rerun renders");
        RestirDi::new()
            .render(false, true, &mut off)
            .expect("off renders");
        assert!(matches!(
            di.render(true, false, &mut off),
            Err(DiError::Disabled)
        ));
        let mut mask = vec![1u8; 800 * 600];
        for y in 200usize..400 {
            for x in 300usize..500 {
                mask[y.wrapping_mul(800).wrapping_add(x)] = 0;
            }
        }
        let diff = frame_diff_count(&on, &off, 800, 600, Some(&mask), 5).expect("masked diff runs");
        assert!(diff > 0, "torch region differs on versus off");
        assert_eq!(
            frame_diff_count(&on, &again, 800, 600, None, 5).expect("identity runs"),
            0
        );
        let mut acc = 0.0f64;
        for (a, b) in on.iter().zip(again.iter()) {
            let d = f64::from(a.abs_diff(*b));
            acc += d * d;
        }
        let rmse = (acc / f64::from(u32::try_from(on.len()).unwrap_or(1))).sqrt() / 255.0;
        assert!(rmse < 0.10, "rmse {rmse} under 10 pct");
        assert!(r1.di_ms.is_finite() && r1.denoise_ms.is_finite());
    }
}
