//! fighter_look_flash: the hit flash on the fighter look, a short bright pulse.
//!
//! Consumer: `FighterLook::push_draws` draws a struck victim through
//! [`flash_material`] at the strength [`flash_pulse`] gives for the ticks left in
//! its window (`FLASH_TICKS`). SW23: the peak is overbright (`FLASH_BOOST`, a
//! 2.0x exposure) so the armour blows out toward white under the tone mapper, and
//! the window carries two blinks (`FLASH_BLINKS`, the second at
//! `FLASH_DECAY_PERMILLE` of the first) so a burst dies out instead of strobing.
//! Before 2026-10-24 the flash replaced the albedo (green kept 35 percent, blue
//! 30 percent) and held exposure 1.6 for the whole window, so the hero read as
//! flat salmon clay and the armour lost its detail
//! (docs/research/look-audit-2026-10-24.md row 11). Now the albedo keeps at least
//! 80 percent of every channel, a warm cast and a lift in exposure ride a pulse
//! that peaks on the hit tick and settles within a few ticks. Units: fixed ticks,
//! linear material factors. Determinism: presentation only, pure functions of the
//! ticks left. Errors: none; a finite valid material stays finite and valid.

use engine_render::v1::PbrMaterial;

use super::FLASH_TICKS;

/// Share of the green channel a flashing body loses at the peak of the pulse: a
/// warm cast over the armour, never a replacement colour.
const FLASH_GREEN_DROP: f32 = 0.10;
/// Share of the blue channel a flashing body loses at the peak of the pulse.
const FLASH_BLUE_DROP: f32 = 0.20;
/// Overbright gain at the peak: the struck body is lit past its own material
/// (a 2.0x exposure), so a white flash actually brightens an armour plate
/// instead of only shifting its hue. The albedo stays warm and detailed.
const FLASH_BOOST: f32 = 2.0;
/// Blinks one hit spends over the flash window: the blow, then a faint echo, so
/// a burst reads as a hit with weight rather than a flat hold.
const FLASH_BLINKS: u8 = 2;
/// Each later blink is this share of the one before, permille: the burst dies
/// out instead of strobing evenly.
const FLASH_DECAY_PERMILLE: u32 = 250;

/// Strength of the hit pulse with `remaining` flash ticks left: 1 on the tick of
/// the hit, then a second, fainter blink ([`FLASH_BLINKS`], each
/// [`FLASH_DECAY_PERMILLE`] of the one before) so a burst dies out instead of
/// strobing evenly. 0 once the window is spent. Pure, no libm.
#[must_use]
pub(super) fn flash_pulse(remaining: u32) -> f32 {
    flash_blink(remaining, FLASH_BLINKS, FLASH_DECAY_PERMILLE)
}

/// Strength of a blinked hit pulse with `remaining` ticks left of
/// [`FLASH_TICKS`], `flashes` equal slices, and each later slice `decay`
/// permille of the one before: 1 on the first tick of the first blink, falling
/// with the square of the ticks left in its slice, so every blink peaks at once
/// and settles. `flashes` 0 reads as 1; `decay` clamps to `0..=1000`. Pure.
#[must_use]
pub(super) fn flash_blink(remaining: u32, flashes: u8, decay_permille: u32) -> f32 {
    let window = FLASH_TICKS.max(1);
    let left = remaining.min(window);
    if left == 0 {
        return 0.0;
    }
    let slice = (window / u32::from(flashes.max(1))).max(1);
    let elapsed = window - left;
    let index = (elapsed / slice).min(u32::from(flashes.max(1)) - 1);
    // Each later blink is `decay` of the one before, multiplied out: no libm.
    let decay = f32::from(u16::try_from(decay_permille.min(1000)).unwrap_or(1000)) / 1000.0;
    let mut weight = 1.0f32;
    for _ in 0..index {
        weight *= decay;
    }
    // How far into its slice this tick sits: 0 at the blink's own peak.
    let inside = elapsed - index * slice;
    let into = u16::try_from(inside.min(slice)).unwrap_or(u16::MAX);
    let span = u16::try_from(slice).unwrap_or(u16::MAX);
    let mut share = 1.0 - f32::from(into) / f32::from(span);
    share *= share;
    weight * share
}

/// The hit-flash material for a body part at `pulse` strength (0 plain, 1 the
/// peak): the albedo keeps its detail (a light warm cast, never a colour
/// replace) and the exposure goes overbright, so the armour blows out toward
/// white under the tone mapper instead of only shifting hue. Pure; a finite
/// valid input stays finite and valid.
#[must_use]
pub(super) fn flash_material(base: &PbrMaterial, pulse: f32) -> PbrMaterial {
    let pulse = pulse.clamp(0.0, 1.0);
    let mut flash = base.clone();
    flash.base_color = [
        base.base_color[0],
        base.base_color[1] * (1.0 - FLASH_GREEN_DROP * pulse),
        base.base_color[2] * (1.0 - FLASH_BLUE_DROP * pulse),
    ];
    flash.exposure = base.exposure * (1.0 + (FLASH_BOOST - 1.0) * pulse);
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
            // The peak is overbright: the lit body reads past its own material,
            // so a white flash brightens the armour instead of only tinting it.
            assert_eq!(
                flash.exposure, FLASH_BOOST,
                "the peak rides the overbright gain"
            );
            assert_eq!(flash.base_color[0], base[0], "red is kept");
            assert!(
                flash.base_color[1] <= base[1] && flash.base_color[2] <= base[2],
                "the warm cast survives the overbright lift"
            );
            // At the peak the albedo keeps at least 75 percent of every
            // channel: the armour stays a detailed surface, not a flat tint.
            for (kept, was) in flash.base_color.iter().zip(base) {
                assert!(*kept >= 0.75 * was, "{kept} of {was}");
            }
            assert_eq!((flash.metallic, flash.roughness), (0.1, 0.6));
            assert!(flash.validate().is_ok(), "{base:?} stays a valid material");
            assert_eq!(flash_material(&plain, 0.0), plain, "no pulse, no change");
        }
    }

    #[test]
    fn flash_pulse_is_short_and_bright_first() {
        assert_eq!(flash_pulse(FLASH_TICKS), 1.0, "the hit tick is the peak");
        assert_eq!(flash_pulse(0), 0.0, "a spent window is plain");
        // Only the hit tick holds full strength: no flat 15-tick tint.
        let full = (0..=FLASH_TICKS)
            .filter(|left| flash_pulse(*left) >= 1.0)
            .count();
        assert_eq!(full, 1, "one tick at full strength");
        // The echo is a blink, not a hold: the first blink's tail is spent
        // before the second one peaks, and that peak is strictly weaker.
        let echo = flash_pulse(FLASH_TICKS - FLASH_TICKS / u32::from(FLASH_BLINKS));
        assert!(echo > 0.0 && echo < 0.5, "the echo is faint: {echo}");
        // The first blink has faded to its own tail before the echo peaks: the
        // last tick of the first slice is 1 tick before the echo's peak.
        let tail = flash_pulse(FLASH_TICKS - FLASH_TICKS / u32::from(FLASH_BLINKS) + 1);
        assert!(tail < echo, "the first blink fades into the echo: {tail}");
    }

    // SW24: the flash's blink row and overbright gain are the combat
    // presentation row's, never a second copy of the numbers.
    #[test]
    fn sw24_the_flash_row_reads_the_presentation_table() {
        let row = engine_combat::feel::PRESENTATION_V1;
        assert_eq!(
            FLASH_BLINKS, row.flash_blinks,
            "the blink count is the table's"
        );
        assert_eq!(
            FLASH_DECAY_PERMILLE,
            u32::from(row.flash_decay_permille),
            "the blink decay is the table's"
        );
        assert_eq!(
            FLASH_BOOST,
            f32::from(row.flash_boost_permille) / 1000.0,
            "the overbright gain is the table's"
        );
    }

    #[test]
    fn flash_a_burst_blinks_weaker_than_the_blow() {
        // The shipped pulse is the two-blink burst at its decay.
        assert_eq!(flash_pulse(FLASH_TICKS), flash_blink(FLASH_TICKS, 2, 250));
        // A later blink is the decay of the one before, multiplied out.
        let half = FLASH_TICKS - FLASH_TICKS / 2;
        assert!((flash_blink(half, 2, 250) - 0.25).abs() < 1e-6);
        assert!((flash_blink(half, 2, 500) - 0.5).abs() < 1e-6);
        // One blink is the old single pulse and only ever falls.
        let mut last = f32::INFINITY;
        for left in (0..=FLASH_TICKS).rev() {
            let now = flash_blink(left, 1, 250);
            assert!(now <= last, "a single blink only falls, at {left}");
            last = now;
        }
        assert_eq!(flash_blink(0, 2, 250), 0.0, "a spent window is plain");
        assert_eq!(
            flash_blink(FLASH_TICKS, 0, 250),
            1.0,
            "no blink reads as one"
        );
    }
}
