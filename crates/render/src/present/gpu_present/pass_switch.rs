//! pass_switch: the renderer's own pass registry, readable and steerable as data.
//!
//! An AI that investigates a picture needs two things from the renderer: what
//! each switchable pass is doing right now, and a way to flip one and look
//! again. Before this file the switches sat behind eight different setters
//! with eight different getters, and the sun shadow map's switch was a
//! private static only a test could flip. This is the single owner of the
//! list: [`Presenter::pass_switches`] reads it, [`Presenter::set_pass`] flips
//! one by name and hands back the state it replaced (so a caller can undo),
//! and [`Presenter::render_facts`] reads the sun shadow map's real numbers
//! (edge in texels, ortho box, texel size) from the constants the live pass
//! draws with.
//!
//! Consumer: the arena window's scene desk (`engine_game::scene_desk`), which
//! the door reaches through the agent seat (`view.scene`, `view.steer`).
//! Units: texels, meters, share `0..=1`. Presentation only, never hashed: a
//! switch changes pixels, never the simulation. Errors: an unknown pass name
//! is [`PassSwitchError::Unknown`] naming every legal name, never a no-op.

use super::presenter::Presenter;
use super::submit::{forward_sun_enabled, set_forward_sun_enabled};
use super::sun_shadow::{
    sun_shadow_size_for_half, sun_shadow_texel_m, sun_view_proj_for_dir, SUN_SHADOW_CENTER_M,
    SUN_SHADOW_FAR_M, SUN_SHADOW_HALF_EXTENT_M, SUN_SHADOW_HALF_RES, SUN_SHADOW_NEAR_M,
};

/// Every pass name [`Presenter::set_pass`] takes, in report order.
pub const PASS_NAMES: [&str; 8] = [
    "sun_shadow",
    "contact_shadow",
    "fog",
    "bloom",
    "auto_exposure",
    "msaa",
    "taa",
    "debug_lines",
];

/// One switchable pass and whether it draws now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PassSwitch {
    /// Name [`Presenter::set_pass`] takes.
    pub name: &'static str,
    /// True while the pass runs.
    pub on: bool,
    /// One line on what the pass paints.
    pub what: &'static str,
}

/// A refused pass switch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PassSwitchError {
    /// The name is not in [`PASS_NAMES`].
    Unknown(String),
}

impl core::fmt::Display for PassSwitchError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Unknown(name) => write!(
                f,
                "no render pass `{name}`; the passes are {}",
                PASS_NAMES.join(", ")
            ),
        }
    }
}

impl std::error::Error for PassSwitchError {}

/// The live sun shadow map: one orthographic depth map over the pit.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SunShadowFacts {
    /// The map is recorded this frame (the `sun_shadow` switch).
    pub on: bool,
    /// Map edge in texels.
    pub edge_px: u32,
    /// Half the ortho box edge in meters.
    pub half_extent_m: f32,
    /// Ortho box center in world meters.
    pub center_m: [f32; 3],
    /// Near plane from the sun eye, meters.
    pub near_m: f32,
    /// Far plane from the sun eye, meters.
    pub far_m: f32,
    /// One texel in meters at the map's edge.
    pub texel_m: f32,
}

impl SunShadowFacts {
    /// The map as the shipped renderer builds it on a `edge_px` map, for a
    /// caller with no presenter (a headless run) or with the live edge in hand.
    #[must_use]
    pub fn of_edge(on: bool, edge_px: u32) -> Self {
        Self {
            on,
            edge_px,
            half_extent_m: SUN_SHADOW_HALF_EXTENT_M,
            center_m: SUN_SHADOW_CENTER_M,
            near_m: SUN_SHADOW_NEAR_M,
            far_m: SUN_SHADOW_FAR_M,
            texel_m: sun_shadow_texel_m(edge_px.max(1)),
        }
    }

    /// The shipped map: the pinned edge (2048 texels, or the 1024 cost arm when
    /// the build selects it), switched on.
    #[must_use]
    pub fn shipped() -> Self {
        Self::of_edge(true, sun_shadow_size_for_half(SUN_SHADOW_HALF_RES))
    }
}

/// What the renderer does with the frame, as numbers.
#[derive(Clone, Debug, PartialEq)]
pub struct RenderFacts {
    /// Backing size in pixels.
    pub size: (u32, u32),
    /// Every switchable pass, in [`PASS_NAMES`] order.
    pub passes: Vec<PassSwitch>,
    /// The sun shadow map.
    pub sun_shadow: SunShadowFacts,
    /// Contact-shadow strength `0..=1` (the screen-space march in the AO field).
    pub contact_strength: f32,
}

/// The sun's orthographic view-projection for a direction toward the sun
/// (row-vector layout, translation in the last row): world point `p` lands at
/// `[p, 1] * m` with x and y in `-1..=1` and depth in `0..=1` when the map
/// covers it. `None` for a non-finite or zero direction.
#[must_use]
pub fn sun_shadow_view_proj(sun_dir: [f32; 3]) -> Option<[[f32; 4]; 4]> {
    sun_view_proj_for_dir(sun_dir).ok()
}

impl Presenter {
    /// Every switchable pass and its state, in [`PASS_NAMES`] order.
    #[must_use]
    pub fn pass_switches(&self) -> Vec<PassSwitch> {
        let state = |name: &'static str| match name {
            "sun_shadow" => (
                forward_sun_enabled(),
                "the sun's depth map: shadows cast by every draw",
            ),
            "contact_shadow" => (
                self.contact_strength > 0.0,
                "screen-space march that darkens ambient where surfaces meet",
            ),
            "fog" => (self.fog_enabled(), "height fog and aerial haze"),
            "bloom" => (self.bloom_enabled(), "bright-pixel glow"),
            "auto_exposure" => (
                self.auto_exposure_enabled(),
                "histogram exposure that adapts to the frame",
            ),
            "msaa" => (self.msaa_enabled(), "4x edge smoothing"),
            "taa" => (
                self.taa_resolve_enabled(),
                "temporal resolve over jittered frames",
            ),
            _ => (
                self.debug_lines_enabled(),
                "debug capsules and facing lines",
            ),
        };
        PASS_NAMES
            .iter()
            .map(|&name| {
                let (on, what) = state(name);
                PassSwitch { name, on, what }
            })
            .collect()
    }

    /// Switch one pass by name and return the state it was in (the undo
    /// value). `msaa` can read back off when the adapter cannot build the 4x
    /// targets; the caller compares the next [`Self::pass_switches`] read.
    ///
    /// # Errors
    ///
    /// [`PassSwitchError::Unknown`] for a name outside [`PASS_NAMES`].
    pub fn set_pass(&mut self, name: &str, on: bool) -> Result<bool, PassSwitchError> {
        let previous = match name {
            "sun_shadow" => set_forward_sun_enabled(on),
            "contact_shadow" => {
                let was = self.contact_strength > 0.0;
                self.set_contact_shadow_strength(if on { 1.0 } else { 0.0 });
                was
            }
            "fog" => {
                let was = self.fog_enabled();
                self.set_fog_enabled(on);
                was
            }
            "bloom" => {
                let was = self.bloom_enabled();
                self.set_bloom_enabled(on);
                was
            }
            "auto_exposure" => {
                let was = self.auto_exposure_enabled();
                self.set_auto_exposure_enabled(on);
                was
            }
            "msaa" => {
                let was = self.msaa_enabled();
                self.set_msaa_enabled(on);
                was
            }
            "taa" => {
                let was = self.taa_resolve_enabled();
                self.set_taa_resolve_enabled(on);
                was
            }
            "debug_lines" => self.debug_line_enable(on),
            other => return Err(PassSwitchError::Unknown(other.to_owned())),
        };
        Ok(previous)
    }

    /// The renderer's own numbers for the frame: size, every pass switch and
    /// the sun shadow map's box and texel size.
    #[must_use]
    pub fn render_facts(&self) -> RenderFacts {
        RenderFacts {
            size: self.size(),
            passes: self.pass_switches(),
            sun_shadow: SunShadowFacts::of_edge(forward_sun_enabled(), self.shadow_map_edge()),
            contact_strength: self.contact_strength,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_pass_name_is_listed_once_and_unknown_names_name_the_legal_ones() {
        let mut seen = PASS_NAMES.to_vec();
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(seen.len(), PASS_NAMES.len(), "names are unique");
        let why = PassSwitchError::Unknown("shadows".to_owned()).to_string();
        for name in PASS_NAMES {
            assert!(why.contains(name), "{name} missing from: {why}");
        }
    }

    #[test]
    fn the_sun_matrix_maps_the_pit_centre_inside_the_map_and_a_far_point_outside() {
        let m = sun_shadow_view_proj([-0.62, 0.55, 0.45]).expect("arena sun builds a matrix");
        let at = |p: [f32; 3]| -> [f32; 3] {
            std::array::from_fn(|k| p[0] * m[0][k] + p[1] * m[1][k] + p[2] * m[2][k] + m[3][k])
        };
        let centre = at([0.0, 0.0, 0.0]);
        assert!(
            centre[0].abs() < 1e-3 && centre[1].abs() < 1e-3,
            "{centre:?}"
        );
        assert!((0.0..=1.0).contains(&centre[2]), "depth {centre:?}");
        let far = at([60.0, 0.0, 0.0]);
        assert!(far[0].abs() > 1.0 || far[1].abs() > 1.0, "{far:?}");
        assert!(sun_shadow_view_proj([0.0; 3]).is_none());
        assert!(sun_shadow_view_proj([f32::NAN, 1.0, 0.0]).is_none());
    }
}
