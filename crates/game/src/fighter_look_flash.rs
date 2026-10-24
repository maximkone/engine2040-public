//! fighter_look_flash: the hit flash on the fighter look, a short bright pulse.
//!
//! Consumer: `FighterLook::push_draws` draws a struck victim through
//! [`flash_material`] at the strength [`flash_pulse`] gives for the ticks left in
//! its window (`FLASH_TICKS`). Before 2026-10-24 the flash replaced the albedo
//! (green kept 35 percent, blue 30 percent) and held exposure 1.6 for the whole
//! window, so the hero read as flat salmon clay and the armour lost its detail
//! (docs/research/look-audit-2026-10-24.md row 11). Now the albedo keeps at least
//! 80 percent of every channel, a warm cast and a lift in exposure ride a pulse that
//! peaks on the hit tick and settles within a few ticks. Units: fixed ticks, linear
//! material factors. Determinism: presentation only, pure functions of the ticks
//! left. Errors: none; a finite valid material stays finite and valid.

use engine_render::v1::PbrMaterial;

use super::FLASH_TICKS;

/// Share of the green channel a flashing body loses at the peak of the pulse: a
/// warm cast over the armour, never a replacement colour.
const FLASH_GREEN_DROP: f32 = 0.10;
/// Share of the blue channel a flashing body loses at the peak of the pulse.
const FLASH_BLUE_DROP: f32 = 0.20;
/// Exposure a flashing body gains at the peak of the pulse (1 + this).
const FLASH_LIFT: f32 = 0.45;

/// Strength of the hit pulse with `remaining` flash ticks left: 1 on the tick of
/// the hit, falling with the square of the time left, so the body lights up at
/// once and settles within a few ticks instead of holding a flat tint for the
/// whole window. 0 once the window is spent. Pure.
#[must_use]
pub(super) fn flash_pulse(remaining: u32) -> f32 {
    let left = remaining.min(FLASH_TICKS) as f32 / FLASH_TICKS as f32;
    left * left
}

/// The hit-flash material for a body part at `pulse` strength (0 plain, 1 the
/// peak): the albedo keeps its detail (a light warm cast, never a colour replace)
/// and the exposure lifts, so the armour reads as lit by a flash and not
/// repainted. Pure; a finite valid input stays finite and valid.
#[must_use]
pub(super) fn flash_material(base: &PbrMaterial, pulse: f32) -> PbrMaterial {
    let pulse = pulse.clamp(0.0, 1.0);
    let mut flash = base.clone();
    flash.base_color = [
        base.base_color[0],
        base.base_color[1] * (1.0 - FLASH_GREEN_DROP * pulse),
        base.base_color[2] * (1.0 - FLASH_BLUE_DROP * pulse),
    ];
    flash.exposure = base.exposure * (1.0 + FLASH_LIFT * pulse);
    flash
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flash_material_warms_lifts_and_stays_valid() {
        for base in [[1.0, 1.0, 1.0], [0.2, 0.5, 0.9], [0.0, 0.0, 0.0]] {
            let plain = PbrMaterial {
                base_color: base,
                metallic: 0.1,
                roughness: 0.6,
                exposure: 1.0,
            };
            let flash = flash_material(&plain, 1.0);
            assert_eq!(flash.base_color[0], base[0], "red is kept");
            assert!(flash.base_color[1] <= base[1] && flash.base_color[2] <= base[2]);
            // At the peak the albedo keeps at least 75 percent of every
            // channel: the armour stays a detailed surface, not a flat tint.
            for (kept, was) in flash.base_color.iter().zip(base) {
                assert!(*kept >= 0.75 * was, "{kept} of {was}");
            }
            assert_eq!(flash.exposure, 1.0 + FLASH_LIFT);
            assert_eq!((flash.metallic, flash.roughness), (0.1, 0.6));
            assert!(flash.validate().is_ok(), "{base:?} stays a valid material");
            assert_eq!(flash_material(&plain, 0.0), plain, "no pulse, no change");
        }
    }

    #[test]
    fn flash_pulse_is_short_and_bright_first() {
        assert_eq!(flash_pulse(FLASH_TICKS), 1.0, "the hit tick is the peak");
        assert_eq!(flash_pulse(0), 0.0, "a spent window is plain");
        let mut last = f32::INFINITY;
        for left in (0..=FLASH_TICKS).rev() {
            let now = flash_pulse(left);
            assert!(now <= last, "the pulse only falls");
            last = now;
        }
        // Half the window left is under a third of the strength: a pulse, not a
        // hold (the old flash drew full strength for all 15 ticks).
        assert!(flash_pulse(FLASH_TICKS / 2) < 0.3);
    }
}
