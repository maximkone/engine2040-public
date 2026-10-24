//! torch_light: the point lights a scene feeds the forward pass.
//!
//! Owner: render lane; consumer is the game crate's placement feed (VS7).
//! Units: meters for positions and ranges, linear RGB for color, a
//! dimensionless scale for intensity. Determinism: fixed slot order, no
//! clock and no RNG in the values; the flicker is a function of the tick
//! and the light's own phase, so the same tick gives the same light.
//! Presentation only: nothing here feeds the tick or the hash.
//!
//! Shape: the forward shader reads up to `TORCH_MAX` torches in one
//! uniform block, so the scene list is capped and the cap is refused
//! loudly instead of silently dropping lights. A torch that fails
//! validation is refused by name: a non-finite position, a negative range
//! or a negative color component never reaches the frame.
//!
//! The night preset: `night_torches` is the arena's night pair (the
//! gate torches) with the warm color and reach the arena ships, and
//! `flicker_scale` is the presentation-only wobble a torch flame gets
//! per tick, bounded so the frame never pulses.

use core::fmt;

/// Most torches per frame; matches the shader's uniform block.
pub const TORCH_MAX: usize = 8;

/// One torch point light: where it burns, how warm it is, how far it
/// reaches and how bright it is.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct TorchLight {
    /// World position of the flame in meters.
    pub pos: [f32; 3],
    /// Linear RGB color, components `>= 0`.
    pub color: [f32; 3],
    /// Influence radius in meters, `> 0` when lit.
    pub range_m: f32,
    /// Brightness scale, `>= 0`.
    pub intensity: f32,
}

impl TorchLight {
    /// A dark torch: burns nothing, reaches nothing.
    pub const OFF: Self = Self {
        pos: [0.0; 3],
        color: [0.0; 3],
        range_m: 0.0,
        intensity: 0.0,
    };

    /// Check finiteness and ranges before the light reaches the frame.
    pub fn validate(&self) -> Result<(), TorchError> {
        if !self.pos.iter().all(|c| c.is_finite()) {
            return Err(TorchError::Invalid("torch position is not finite".into()));
        }
        if !self.color.iter().all(|c| c.is_finite() && *c >= 0.0) {
            return Err(TorchError::Invalid(
                "torch color is not finite non-negative".into(),
            ));
        }
        if !self.range_m.is_finite() || self.range_m <= 0.0 {
            return Err(TorchError::Invalid(format!(
                "torch range {} is not finite positive",
                self.range_m
            )));
        }
        if !self.intensity.is_finite() || self.intensity < 0.0 {
            return Err(TorchError::Invalid(format!(
                "torch intensity {} is not finite non-negative",
                self.intensity
            )));
        }
        Ok(())
    }

    /// This light dimmed or brightened by a factor: a negative factor
    /// would make the light negative, so it is refused.
    pub fn scaled(&self, factor: f32) -> Result<Self, TorchError> {
        if !factor.is_finite() || factor < 0.0 {
            return Err(TorchError::Invalid(format!(
                "torch scale {factor} is not finite non-negative"
            )));
        }
        Ok(Self {
            pos: self.pos,
            color: self.color,
            range_m: self.range_m,
            intensity: self.intensity * factor,
        })
    }
}

/// Torch refusals: bounded diagnostics, never panics.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum TorchError {
    /// A light or a list breaks a rule; the reason carried.
    Invalid(String),
    /// More lights than the shader block holds; the count carried.
    TooMany(usize),
}

impl fmt::Display for TorchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(reason) => write!(f, "torch light: {reason}"),
            Self::TooMany(count) => write!(f, "torch light: {count} lights, at most {TORCH_MAX}"),
        }
    }
}

impl std::error::Error for TorchError {}

/// The warm color the arena torches burn with, linear RGB.
pub const ARENA_TORCH_COLOR: [f32; 3] = [1.0, 0.55, 0.22];

/// How far an arena torch reaches, in meters.
pub const ARENA_TORCH_RANGE_M: f32 = 12.0;

/// How bright an arena torch burns.
pub const ARENA_TORCH_INTENSITY: f32 = 1.0;

/// The wobble a flame gets per tick: the scale moves inside this band
/// around 1.0, so the frame never pulses to black and never blows out.
pub const FLICKER_BAND: f32 = 0.08;

/// The presentation-only flicker of one torch at one tick: a smooth
/// function of the tick and the torch's phase, bounded by
/// [`FLICKER_BAND`]. Same tick, same light, same value; no clock, no RNG.
#[must_use]
pub fn flicker_scale(tick: u32, phase: f32) -> f32 {
    if !phase.is_finite() {
        return 1.0;
    }
    let t = f32::from(u16::try_from(tick % 1024).unwrap_or(0)) * 0.061;
    let wave = (t + phase).sin() * 0.6 + (t * 2.3 + phase * 1.7).sin() * 0.4;
    1.0 + wave * FLICKER_BAND
}

/// The arena's night torches: the gate pair, warm and reaching.
#[must_use]
pub fn night_torches() -> [TorchLight; 2] {
    [
        TorchLight {
            pos: [20.0, 1.0, -3.0],
            color: ARENA_TORCH_COLOR,
            range_m: ARENA_TORCH_RANGE_M,
            intensity: ARENA_TORCH_INTENSITY,
        },
        TorchLight {
            pos: [20.0, 1.0, 3.0],
            color: ARENA_TORCH_COLOR,
            range_m: ARENA_TORCH_RANGE_M,
            intensity: ARENA_TORCH_INTENSITY,
        },
    ]
}

/// Check a scene's torch list: it must fit the shader block and every light
/// must be valid. A list that breaks a rule is refused by name. No
/// allocation: the caller keeps the list it already has, so a per-frame
/// feed grows nothing.
pub fn check_torch_list(lights: &[TorchLight]) -> Result<(), TorchError> {
    if lights.len() > TORCH_MAX {
        return Err(TorchError::TooMany(lights.len()));
    }
    for light in lights {
        light.validate()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn light() -> TorchLight {
        TorchLight {
            pos: [5.5, 2.0, -2.0],
            color: ARENA_TORCH_COLOR,
            range_m: ARENA_TORCH_RANGE_M,
            intensity: 1.0,
        }
    }

    #[test]
    fn a_valid_light_passes_and_a_broken_one_is_named() {
        assert!(light().validate().is_ok());
        let mut broken = light();
        broken.pos[0] = f32::NAN;
        assert!(broken.validate().is_err());
        let mut no_range = light();
        no_range.range_m = 0.0;
        assert!(no_range.validate().is_err());
        let mut dark = light();
        dark.color[1] = -1.0;
        assert!(dark.validate().is_err());
    }

    #[test]
    fn the_list_refuses_more_lights_than_the_block_holds() {
        let many = vec![light(); TORCH_MAX + 1];
        assert!(check_torch_list(&many).is_err());
        let exact = vec![light(); TORCH_MAX];
        assert!(check_torch_list(&exact).is_ok());
        assert!(check_torch_list(&[]).is_ok(), "no torches is fine");
    }

    #[test]
    fn the_list_refuses_a_broken_light() {
        let mut broken = light();
        broken.intensity = -1.0;
        assert!(check_torch_list(&[broken]).is_err());
    }

    #[test]
    fn the_night_pair_burns_at_the_gate() {
        let torches = night_torches();
        assert_eq!(torches.len(), 2);
        for torch in torches {
            assert!(torch.validate().is_ok());
        }
        assert_eq!(torches[0].pos[0], 20.0);
        assert_eq!(torches[1].pos[2], 3.0);
    }

    #[test]
    fn the_flicker_stays_inside_its_band_and_moves() {
        let mut low: f32 = 2.0;
        let mut high: f32 = 0.0;
        for tick in 0..600 {
            let scale = flicker_scale(tick, 0.3);
            low = low.min(scale);
            high = high.max(scale);
            assert!((1.0 - FLICKER_BAND..=1.0 + FLICKER_BAND).contains(&scale));
        }
        assert!(high - low > 0.01, "the flame actually moves");
        assert!(low >= 1.0 - FLICKER_BAND);
    }

    #[test]
    fn the_flicker_is_a_function_of_the_tick_only() {
        assert_eq!(flicker_scale(77, 0.4), flicker_scale(77, 0.4));
        assert_ne!(flicker_scale(77, 0.4), flicker_scale(78, 0.4));
    }

    #[test]
    fn scaling_a_light_keeps_it_valid_and_refuses_a_bad_scale() {
        let dimmed = light().scaled(0.5).expect("half is fine");
        assert_eq!(dimmed.intensity, 0.5);
        assert!(light().scaled(-1.0).is_err());
    }
}

/// SW5: the vertex-rate twin of the torch sum. `quality::TorchLighting::Vertex`
/// picks it on the potato row, `shaders/forward_torch_vertex.wgsl` runs it on
/// the GPU, and this module is the CPU model its accept measures. Std-only.
#[path = "torch_vertex.rs"]
pub mod vertex;
