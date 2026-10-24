//! torch_acceptance: the VS7 proof (the frame light list holds every torch
//! actor; at night the sand near a torch is far brighter than the far sand).
//!
//! Owner: game lane; consumer is the night capture and the row's gate.
//! Units: meters for distances, linear radiance for luminance. The CPU
//! mirror of the shader's torch falloff (`eg12_torch_falloff` and
//! `eg12_torch_sum` in `shaders/forward_maps.wgsl`) is the oracle, so the
//! numbers here are the numbers the forward pass draws with.
//!
//! Determinism: no clock, no RNG, no image. Presentation only: nothing
//! here feeds the tick or the hash.

/// The shader's torch falloff, mirrored: the inverse-square term with the
/// edge fade of `eg12_torch_falloff`.
#[must_use]
pub fn torch_falloff(dist_m: f32, range_m: f32) -> f32 {
    if range_m <= 0.0 || dist_m >= range_m {
        return 0.0;
    }
    let att = 1.0 / (1.0 + dist_m * dist_m);
    let edge = 1.0 - dist_m / range_m;
    att * edge * edge
}

/// The torch light on one sand point: the sum of every torch's Lambert
/// term under the falloff, mirrored from `eg12_torch_sum`. The sand
/// normal points up, so only torches above the point light it.
#[must_use]
pub fn sand_luminance_at(
    point_m: [f32; 3],
    lights: &[engine_render::torch_light::TorchLight],
) -> f32 {
    let mut sum = 0.0f32;
    for light in lights {
        let delta = [
            light.pos[0] - point_m[0],
            light.pos[1] - point_m[1],
            light.pos[2] - point_m[2],
        ];
        let dist = (delta[0] * delta[0] + delta[1] * delta[1] + delta[2] * delta[2]).sqrt();
        if dist <= 0.0 {
            continue;
        }
        // The sand normal is up, so the cosine term is the vertical part.
        let ndot_l = delta[1] / dist;
        if ndot_l <= 0.0 {
            continue;
        }
        let falloff = torch_falloff(dist, light.range_m);
        let radiance = light.color[1] * light.intensity * ndot_l * falloff;
        sum += 0.2126 * radiance + 0.7152 * radiance + 0.0722 * radiance;
    }
    sum
}

/// The mean sand luminance over a disc of points around a torch: the
/// "within 2 m" arm of the row's Accept.
#[must_use]
pub fn mean_luminance_within(
    center_m: [f32; 3],
    radius_m: f32,
    lights: &[engine_render::torch_light::TorchLight],
) -> f32 {
    if radius_m <= 0.0 {
        return 0.0;
    }
    // A small fixed ring of samples: no RNG, so the same points every run.
    let samples = 8usize;
    let mut sum = 0.0f32;
    for i in 0..samples {
        let angle = std::f32::consts::TAU * (f32::from(u16::try_from(i).unwrap_or(0)))
            / (f32::from(u16::try_from(samples).unwrap_or(1)));
        let point = [
            center_m[0] + radius_m * angle.cos(),
            center_m[1],
            center_m[2] + radius_m * angle.sin(),
        ];
        sum += sand_luminance_at(point, lights);
    }
    sum / f32::from(u16::try_from(samples).unwrap_or(1))
}

/// The far sand point: the same sand, far from every torch, so the far
/// luminance is the baseline the near sand is compared against.
#[must_use]
pub fn far_sand_luminance(lights: &[engine_render::torch_light::TorchLight]) -> f32 {
    // The arena's far corner, well past every torch's reach.
    let point = [18.0, 0.0, 18.0];
    sand_luminance_at(point, lights)
}

/// The row's night ratio: the mean luminance within 2 m of a torch over
/// the far sand luminance. The lights are the frame's own list, so the
/// ratio measures what the frame draws. Zero when the scene has no
/// torches, so the caller can tell "no torch light" from "torch light".
#[must_use]
pub fn night_ratio(lights: &[engine_render::torch_light::TorchLight], tick: u32) -> f32 {
    if lights.is_empty() {
        return 0.0;
    }
    let flickering: Vec<engine_render::torch_light::TorchLight> = lights
        .iter()
        .enumerate()
        .map(|(i, light)| {
            light
                .scaled(engine_render::torch_light::flicker_scale(
                    tick,
                    f32::from(u16::try_from(i).unwrap_or(0)) * 1.7,
                ))
                .unwrap_or(*light)
        })
        .collect();
    let mut near = 0.0f32;
    for light in &flickering {
        near += mean_luminance_within(light.pos, 2.0, &flickering);
    }
    let near = near / f32::from(u16::try_from(flickering.len()).unwrap_or(1));
    let far = far_sand_luminance(&flickering);
    if far <= 0.0 {
        return f32::INFINITY;
    }
    near / far
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arena_kit::{kit_actors, KitActor, KitAsset};

    fn torch_actor(id: u32, x: f32, z: f32) -> KitActor {
        KitActor {
            id,
            asset: KitAsset::Torch,
            pos_m: [x, 1.0, z],
            size_m: [0.5, 2.0, 0.5],
            yaw: 0.0,
            physical: false,
            texture_scale: 1.0,
        }
    }

    #[test]
    fn the_falloff_matches_the_shader_rule() {
        assert_eq!(torch_falloff(0.0, 12.0), 1.0);
        assert_eq!(torch_falloff(12.0, 12.0), 0.0, "the edge is dark");
        assert_eq!(torch_falloff(20.0, 12.0), 0.0, "past the reach is dark");
        let near = torch_falloff(1.0, 12.0);
        let far = torch_falloff(6.0, 12.0);
        assert!(near > far, "the falloff shrinks with distance");
    }

    #[test]
    fn the_frame_light_list_holds_every_torch_actor() {
        let actors = kit_actors().expect("embedded placement parses");
        let torch_count = actors.iter().filter(|a| a.asset == KitAsset::Torch).count();
        assert!(torch_count > 0, "the placement has torch actors");
        let lights =
            crate::arena_kit::torch_feed::torch_lights(&actors).expect("the lights are valid");
        assert_eq!(
            lights.len(),
            torch_count,
            "every torch actor is a light, and nothing else is"
        );
        for light in &lights {
            assert!(light.validate().is_ok());
        }
    }

    #[test]
    fn the_near_sand_is_much_brighter_than_the_far_sand() {
        let actors = kit_actors().expect("embedded placement parses");
        let lights =
            crate::arena_kit::torch_feed::torch_lights(&actors).expect("the lights are valid");
        assert!(!lights.is_empty(), "the placement lights the arena");
        let ratio = night_ratio(&lights, 0);
        assert!(
            ratio >= 2.0,
            "the sand within 2 m of a torch is at least twice the far sand (ratio {ratio})"
        );
    }

    #[test]
    fn the_ratio_holds_across_the_flicker() {
        let actors = kit_actors().expect("embedded placement parses");
        let lights =
            crate::arena_kit::torch_feed::torch_lights(&actors).expect("the lights are valid");
        for tick in [0u32, 17, 120, 401, 900] {
            let ratio = night_ratio(&lights, tick);
            assert!(
                ratio >= 2.0,
                "tick {tick}: the near sand stays at least twice the far sand (ratio {ratio})"
            );
        }
    }

    #[test]
    fn a_scene_without_torches_has_no_night_ratio() {
        assert_eq!(night_ratio(&[], 0), 0.0, "no torches, no ratio");
    }

    #[test]
    fn the_torch_actor_burns_above_its_stand() {
        let actor = torch_actor(101, 5.5, -2.0);
        let light = crate::arena_kit::torch_feed::torch_light_for(&actor).expect("a torch burns");
        assert_eq!(light.pos, [5.5, 2.0, -2.0]);
    }
}
