//! torch_feed: the placement's torch actors become lights (VS7).
//!
//! Owner: game lane; consumer is the window and capture legs, which feed
//! the presenter's torch list. Units: meters for positions and ranges,
//! linear RGB for color. Determinism: the actor order of the placement
//! file is the light order, and the flicker is a function of the tick and
//! the light's own phase, so the same tick gives the same lights. Nothing
//! here feeds the tick or the hash: presentation only.
//!
//! Shape: a torch actor is a prop with a flame at its head, so the light
//! sits at the actor's box center plus half its height: the flame burns
//! above the stand, not inside it. The reach and the color come from the
//! arena's torch values, so the night picture matches the shipped look.

use super::{KitActor, KitAsset};
use engine_render::torch_light::{
    flicker_scale, TorchError, TorchLight, ARENA_TORCH_COLOR, ARENA_TORCH_RANGE_M,
};

/// How far above the actor's box center the flame burns, in meters,
/// before the actor's own half height is added.
pub const FLAME_OFFSET_M: f32 = 0.0;

/// The light one torch actor burns: the flame at the head of the prop,
/// warm and reaching the arena's torch range. A non-torch actor burns
/// nothing.
#[must_use]
pub fn torch_light_for(actor: &KitActor) -> Option<TorchLight> {
    if actor.asset != KitAsset::Torch {
        return None;
    }
    let half_height = actor.size_m[1] * 0.5;
    Some(TorchLight {
        pos: [
            actor.pos_m[0],
            actor.pos_m[1] + half_height + FLAME_OFFSET_M,
            actor.pos_m[2],
        ],
        color: ARENA_TORCH_COLOR,
        range_m: ARENA_TORCH_RANGE_M,
        intensity: 1.0,
    })
}

/// The lights the placement's torch actors burn: one per torch actor, in
/// placement order. Refuses a broken list loudly (the frame must never
/// carry a half-valid light set).
pub fn torch_lights(actors: &[KitActor]) -> Result<Vec<TorchLight>, TorchError> {
    let mut lights = Vec::new();
    for actor in actors {
        if let Some(light) = torch_light_for(actor) {
            lights.push(light);
        }
    }
    engine_render::torch_light::check_torch_list(&lights)?;
    Ok(lights)
}

/// The same lights with the presentation-only flicker of the frame's
/// tick: each light keeps its own phase from its actor id, so the two
/// torches do not pulse together. The scale stays inside the flicker
/// band, so the frame never pulses to black.
pub fn torch_lights_at(actors: &[KitActor], tick: u32) -> Result<Vec<TorchLight>, TorchError> {
    let lights = torch_lights(actors)?;
    let mut flickering = Vec::with_capacity(lights.len());
    for (i, light) in lights.iter().enumerate() {
        let phase = f32::from(u16::try_from(i).unwrap_or(0)) * 1.7;
        flickering.push(light.scaled(flicker_scale(tick, phase))?);
    }
    engine_render::torch_light::check_torch_list(&flickering)?;
    Ok(flickering)
}

/// The sun's share of the light under the night preset: the moon is a
/// fraction of the noon sun, so the torches carry the picture.
pub const NIGHT_SUN_SCALE: f32 = 0.12;

/// The night preset's sun strength: the frame's sun scaled to a moonlit
/// fraction. Pure: the same sun gives the same night sun.
#[must_use]
pub fn night_sun_intensity(sun_intensity: f32) -> f32 {
    if !sun_intensity.is_finite() {
        return 0.0;
    }
    sun_intensity * NIGHT_SUN_SCALE
}

/// The night preset's sun for one draw list: the sun drops to a moonlit
/// fraction, so the placement's torches carry the picture. Pure: the same
/// list gives the same night list. Presentation only, never hashed.
///
/// Gated on `app`: the draw list lives behind the render crate's `v1`
/// feature, and only the window leg (`app`, which turns on
/// `engine-render/window`) has one. The headless default has no draw list,
/// so it has no night sun to apply.
#[cfg(feature = "app")]
pub fn apply_night_sun(list: &mut engine_render::present::DrawList) {
    list.light.intensity = night_sun_intensity(list.light.intensity);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arena_kit::kit_actors;
    fn torch_actor(id: u32, x: f32, z: f32) -> KitActor {
        KitActor {
            id,
            asset: KitAsset::Torch,
            pos_m: [x, 1.0, z],
            size_m: [0.5, 2.0, 0.5],
            yaw: 0.0,
            physical: false,
        }
    }

    #[test]
    fn only_torch_actors_burn() {
        let actor = torch_actor(101, 5.5, -2.0);
        let light = torch_light_for(&actor).expect("a torch burns");
        assert_eq!(light.pos, [5.5, 2.0, -2.0], "the flame sits at the head");
        assert_eq!(light.range_m, ARENA_TORCH_RANGE_M);
        let mut banner = actor;
        banner.asset = KitAsset::Banner;
        assert!(torch_light_for(&banner).is_none(), "a banner burns nothing");
    }

    #[test]
    fn the_placement_gives_two_torches() {
        let actors = kit_actors().expect("embedded placement parses");
        let lights = torch_lights(&actors).expect("the placement lights are valid");
        assert_eq!(lights.len(), 2, "the placement has two torch actors");
        assert!(lights.iter().all(|l| l.validate().is_ok()));
    }

    #[test]
    fn the_flicker_keeps_the_lights_valid_and_moves_them() {
        let actors = kit_actors().expect("embedded placement parses");
        let first = torch_lights_at(&actors, 10).expect("valid at tick 10");
        let again = torch_lights_at(&actors, 10).expect("valid at tick 10");
        assert_eq!(first, again, "the same tick gives the same lights");
        let later = torch_lights_at(&actors, 400).expect("valid at tick 400");
        assert_ne!(first, later, "the flame moves");
        for light in first {
            assert!(light.validate().is_ok());
            assert!(light.intensity > 0.0, "a torch never goes dark");
        }
    }

    #[test]
    fn the_two_torches_do_not_pulse_together() {
        let actors = kit_actors().expect("embedded placement parses");
        let lights = torch_lights_at(&actors, 3).expect("valid at tick 3");
        assert_eq!(lights.len(), 2);
        assert_ne!(
            lights[0].intensity, lights[1].intensity,
            "the two torches keep their own phase"
        );
    }

    #[test]
    fn the_night_preset_drops_the_sun_to_a_moonlit_fraction() {
        // The scale is a constant, so these two checks are compile-time ones:
        // a const block keeps them as asserts without a runtime assert on a
        // value the compiler already knows.
        const { assert!(NIGHT_SUN_SCALE < 1.0, "the night sun is a fraction of noon") };
        const { assert!(NIGHT_SUN_SCALE > 0.0, "the night sun is a moon, not black") };
        assert_eq!(night_sun_intensity(3.0), 3.0 * NIGHT_SUN_SCALE);
        assert_eq!(night_sun_intensity(f32::NAN), 0.0, "a broken sun is dark");
    }

    #[test]
    fn the_night_preset_keeps_the_torches_reachable() {
        let actors = kit_actors().expect("embedded placement parses");
        let lights = torch_lights(&actors).expect("valid");
        let ratio = crate::arena_kit::torch_acceptance::night_ratio(&lights, 0);
        assert!(ratio >= 2.0, "the near sand is at least twice the far sand");
    }
}
