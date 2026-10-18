//! contact_live: the live contact feet reaching the forward shader.
//!
//! Owner: render lane. Consumer: `submit::bind_twin_shadow`, which writes the
//! feet block behind the 32 sun bytes of the shadow uniform (group 0 binding
//! 3) on every window-path frame, and the game's fighter look, which calls
//! [`Presenter::set_contact_feet`] with where the bodies stand. Units: meters.
//! Presentation only, never hashed. The pack and the CPU oracle live in
//! [`crate::contact_feet`].

use super::presenter::Presenter;
use crate::contact_feet::{contact_feet_bytes, ContactFoot};

/// Byte offset of the feet block inside the shadow uniform: right behind the
/// 32-byte sun splits (`ShadowU` in `shaders/forward.wgsl`).
pub(super) const CONTACT_FEET_OFFSET: u64 = 32;

/// Size of the shadow uniform buffer: sun splits plus the feet block.
pub(super) const TWIN_SHADOW_BUF_BYTES: u64 = CONTACT_FEET_OFFSET + 128;

impl Presenter {
    /// Set where the fighters stand for the grounding shade: sand within each
    /// foot's radius keeps less of the light (see [`crate::contact_feet`]).
    /// Takes effect on the next frame and holds until the next call, so a
    /// frame that reuses its last fighter draws keeps its shade. An empty
    /// slice clears it: with no feet all the sand stays lit.
    pub fn set_contact_feet(&mut self, feet: &[ContactFoot]) {
        self.contact_feet = contact_feet_bytes(feet);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contact_feet::CONTACT_FEET_BYTES;
    use crate::pass_truth::harness::fixtures::{arena_like_scene, init_or_skip, FLOOR_DRAW, H, W};
    use crate::pass_truth::LookToggles;
    use crate::present::{DrawList, LivePixels};

    /// Frame index of the floor point `at`, `None` when it leaves the picture.
    fn pixel_of(list: &DrawList, pixels: &LivePixels, at: [f32; 3]) -> Option<usize> {
        let clip = list.camera.view_proj().ok()? * glam::Vec4::new(at[0], at[1], at[2], 1.0);
        if clip.w <= 0.0 {
            return None;
        }
        let (nx, ny) = (clip.x / clip.w, clip.y / clip.w);
        let x = ((nx * 0.5 + 0.5) * pixels.width as f32).round();
        let y = ((1.0 - (ny * 0.5 + 0.5)) * pixels.height as f32).round();
        if x < 0.0 || y < 0.0 || x >= pixels.width as f32 || y >= pixels.height as f32 {
            return None;
        }
        Some(y as usize * pixels.width as usize + x as usize)
    }

    fn luma(pixels: &LivePixels, texel: usize) -> f32 {
        let px = &pixels.rgba[texel * 4..texel * 4 + 3];
        (f32::from(px[0]) + f32::from(px[1]) + f32::from(px[2])) / (3.0 * 255.0)
    }

    #[test]
    fn the_blob_follows_the_fed_feet_and_open_sand_has_no_fixed_discs() {
        // The live arena showed dark discs in an X across the sand (24 baked-in
        // world positions) and none under the fighters. Behavior: with no
        // feet the sand has no step edges where those discs were, a fed foot
        // darkens the sand under it only, and clearing the feet restores the
        // open frame byte for byte.
        let Some(mut presenter) = init_or_skip("the_blob_follows_the_fed_feet") else {
            return;
        };
        presenter.resize(W, H).expect("valid size applies");
        let list = arena_like_scene(&mut presenter, W, H);
        let toggles = LookToggles::all_on();
        let shot = |presenter: &mut Presenter| {
            presenter
                .capture_live_pixels(&list, &toggles)
                .expect("live frame captures")
        };
        let open = shot(&mut presenter);
        // No step edge along the old fixture rows (x = -4..6 at z = 0.5 crossed
        // the edges of the z = 0 disk line every two meters).
        let mut steps = 0usize;
        let mut worst = 0.0f32;
        let mut last: Option<f32> = None;
        for i in 0..=100u8 {
            let at = [-4.0 + 0.1 * f32::from(i), 0.0, 0.5];
            let texel = pixel_of(&list, &open, at).filter(|t| open.ids[*t] == FLOOR_DRAW);
            let Some(texel) = texel else {
                last = None;
                continue;
            };
            let here = luma(&open, texel);
            if let Some(before) = last {
                worst = worst.max((here - before).abs() / before.max(1e-3));
                steps += 1;
            }
            last = Some(here);
        }
        assert!(steps > 60, "the row is in view: {steps} steps");
        assert!(worst < 0.03, "open sand has no step edge, worst {worst}");

        // A fed foot darkens its own sand and nothing 1.1 m away.
        let foot = ContactFoot {
            x_m: 1.0,
            z_m: 1.5,
            radius_m: 0.9,
        };
        presenter.set_contact_feet(&[foot]);
        let fed = shot(&mut presenter);
        let under = pixel_of(&list, &open, [1.0, 0.0, 1.5]).expect("foot sand in view");
        let clear = pixel_of(&list, &open, [1.0, 0.0, 2.7]).expect("clear sand in view");
        assert!(
            luma(&fed, under) < 0.8 * luma(&open, under),
            "sand under the foot darkens: {} vs {}",
            luma(&fed, under),
            luma(&open, under)
        );
        assert_eq!(
            &fed.rgba[clear * 4..clear * 4 + 3],
            &open.rgba[clear * 4..clear * 4 + 3],
            "sand clear of the foot does not move"
        );
        assert_eq!(
            fed.rgba,
            shot(&mut presenter).rgba,
            "the frame repeats exactly"
        );

        // Moved away, the old spot is open again; cleared, the frame is the open one.
        presenter.set_contact_feet(&[ContactFoot {
            x_m: -3.0,
            z_m: 2.5,
            radius_m: 0.9,
        }]);
        let moved = shot(&mut presenter);
        assert_eq!(
            &moved.rgba[under * 4..under * 4 + 3],
            &open.rgba[under * 4..under * 4 + 3],
            "the shade left the old spot with the foot"
        );
        presenter.set_contact_feet(&[]);
        assert_eq!(shot(&mut presenter).rgba, open.rgba, "no feet, open sand");
    }

    #[test]
    fn the_shadow_uniform_holds_the_sun_splits_and_the_feet_block() {
        // The forward shader reads `ShadowU`: 32 sun bytes, then 8 vec4 feet.
        assert_eq!(CONTACT_FEET_OFFSET, 32);
        assert_eq!(TWIN_SHADOW_BUF_BYTES, 32 + 128);
        assert_eq!(
            usize::try_from(TWIN_SHADOW_BUF_BYTES - CONTACT_FEET_OFFSET),
            Ok(CONTACT_FEET_BYTES),
            "the buffer holds exactly the feet block behind the sun bytes"
        );
        let shader = include_str!("../../shaders/forward.wgsl");
        assert!(shader.contains("feet: array<vec4<f32>, 8>"));
    }
}
