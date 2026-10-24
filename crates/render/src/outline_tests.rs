//! Tests for [`super`]: the mask at half res, the Sobel edge colour chosen by
//! the depth compare through the berm, the 4 px band, the composite, the pulse
//! and the shader-text twins. Numbers are the row's Accept numbers.

use super::*;

/// The sand berm the arena camera watches, `ARENA_WALL_BOX` of
/// `crates/game/src/camera.rs:243`. Copied, not imported: render never depends
/// on game. It is 0.5 m thick on x, so a target whose near surface is 1 m
/// behind the x = 10 front face is covered by the prefill.
const WALL_MIN: [f32; 3] = [10.0, -5.0, -5.0];
const WALL_MAX: [f32; 3] = [10.5, 5.0, 5.0];

/// The oracle frame the pixel tests run at: 320x180 keeps the suite quick and
/// its half-res mask is 160x90.
const FRAME_W: u32 = 320;
const FRAME_H: u32 = 180;

/// The frame aspect of the tests, the arena camera's 16:9.
const ASPECT: f32 = 16.0 / 9.0;

/// The arena camera: eye at the ring edge looking along +x at the berm.
fn view() -> OutlineView {
    OutlineView::new(
        [0.0, 1.5, 0.0],
        [10.0, 1.5, 0.0],
        60.0_f32.to_radians(),
        ASPECT,
        0.1,
        200.0,
    )
    .expect("the arena view has a projection")
}

/// The prefill of the berm alone: a depth field every ray through the box hits.
fn berm_prefill() -> SceneDepth {
    let mut scene = SceneDepth::sky(FRAME_W, FRAME_H);
    scene.prefill_box(&view(), WALL_MIN, WALL_MAX);
    scene
}

/// One selected target alone, rendered through the whole CPU chain.
fn edges_for(center: [f32; 3]) -> OutlineEdge {
    let plan = outline_plan(Quality::High, FRAME_W, FRAME_H, 1);
    assert!(plan.enabled, "one id at High draws");
    let targets = [OutlineTarget {
        id: 7,
        center,
        radius_m: 0.5,
    }];
    let mask = mask_pass(
        &view(),
        &berm_prefill(),
        &targets,
        plan.mask_size[0],
        plan.mask_size[1],
    );
    edge_pass(&mask)
}

#[test]
fn outline_mask_runs_at_half_res_on_1280_720() {
    let plan = outline_plan(Quality::High, 1280, 720, 1);
    assert!(plan.enabled);
    assert_eq!(plan.mask_size, [640, 360], "the row's 640x360 mask");
    assert_eq!(plan.draws, 1, "one mask draw per selected id");
    // An odd frame rounds up on both axes, the `gtao_half` rule.
    assert_eq!(
        outline_plan(Quality::Comfort, 1921, 1081, 2).mask_size,
        [961, 541]
    );
    // The rendered oracle agrees with the plan's size at the test frame.
    let mask = mask_pass(
        &view(),
        &SceneDepth::sky(FRAME_W, FRAME_H),
        &[OutlineTarget {
            id: 1,
            center: [8.0, 1.5, 0.0],
            radius_m: 0.5,
        }],
        160,
        90,
    );
    assert_eq!([mask.width, mask.height], [160, 90]);
}

/// F2P: a target 1 m behind the berm still wears an outline, and that outline
/// reads the hidden (dim warm) colour, not the visible white.
#[test]
fn outline_edge_picks_the_hidden_color_through_the_wall() {
    let edge = edges_for([11.5, 1.5, 0.0]);
    let (color, _, _) = edge.peak().expect("the covered target still has an edge");
    // The hidden colour's ratios: 0.1 / 0.04 / 0.02.
    assert!(color[0] > 0.0, "hidden edge is drawn, got {color:?}");
    assert!(
        (color[1] / color[0] - 0.4).abs() < 1e-4,
        "green/red is the hidden hue, got {color:?}"
    );
    assert!(
        (color[2] / color[0] - 0.2).abs() < 1e-4,
        "blue/red is the hidden hue, got {color:?}"
    );
    // Dim: the edge field carries no strength multiplier, so the hidden peak is
    // a tenth of the visible one.
    assert!(color[0] < 0.2, "the hidden peak is dim, got {color:?}");
    // Through the whole composite it stays a warm ember on a black frame.
    let band = outline_band(&edge, EDGE_THICKNESS, true);
    let (_, x, y) = edge.peak().expect("peak");
    let out = composite([0.0; 3], &band, x, y, EDGE_STRENGTH, 1.0, 1.0);
    assert!(
        out[0] < 0.5 && out[0] > out[1] && out[1] > out[2],
        "the hidden outline is a dim warm band, got {out:?}"
    );
}

/// The same target in front of the berm reads the visible white: the colour is
/// the depth compare's, not the target's.
#[test]
fn outline_edge_picks_the_visible_color_when_nothing_covers() {
    let edge = edges_for([8.0, 1.5, 0.0]);
    let (color, x, y) = edge.peak().expect("the open target has an edge");
    assert!(
        (color[0] - color[1]).abs() < 1e-6 && (color[1] - color[2]).abs() < 1e-6,
        "an uncovered target is white, got {color:?}"
    );
    assert!(color[0] > 0.2, "the visible edge is bright, got {color:?}");
    let band = outline_band(&edge, EDGE_THICKNESS, true);
    let out = composite([0.0; 3], &band, x, y, EDGE_STRENGTH, 1.0, 1.0);
    assert!(
        out.iter().all(|c| *c > 0.9),
        "the visible outline is white, got {out:?}"
    );
}

/// F2P: nothing selected costs no draw and changes no pixel, on every tier.
#[test]
fn no_selection_draws_nothing() {
    for tier in Quality::ALL {
        let plan = outline_plan(tier, 1280, 720, 0);
        assert!(!plan.enabled, "{tier:?} with no id runs nothing");
        assert_eq!(plan.draws, 0, "{tier:?} with no id draws nothing");
        assert_eq!(plan.mask_size, [0, 0], "{tier:?} with no id has no mask");
    }
    let mask = mask_pass(&view(), &berm_prefill(), &[], 160, 90);
    assert!(
        mask.depth.iter().all(|d| !d.is_finite()),
        "an empty selection covers no texel"
    );
    let edge = edge_pass(&mask);
    assert!(edge.peak().is_none(), "an empty mask has no edge texel");
    let band = outline_band(&edge, EDGE_THICKNESS, true);
    let frame = [0.25f32, 0.5, 0.75];
    for (x, y) in [(0u32, 0u32), (80, 45), (159, 89)] {
        assert_eq!(
            composite(frame, &band, x, y, EDGE_STRENGTH, 1.0, 1.0),
            frame,
            "the composite is the identity at {x},{y}"
        );
    }
}

/// A mask with one vertical coverage step: the Sobel band is 2 texels wide, and
/// the thickness blur at the quality default spreads it to the row's 4 px.
fn step_mask() -> OutlineMask {
    let (width, height) = (64u32, 9u32);
    let half = width / 2;
    OutlineMask {
        width,
        height,
        depth: (0..width * height)
            .map(|i| {
                if i % width >= half {
                    8.0
                } else {
                    f32::INFINITY
                }
            })
            .collect(),
        visible: vec![1.0; texels(width, height)],
    }
}

#[test]
fn outline_edge_band_is_four_pixels_at_default_thickness() {
    let mask = step_mask();
    let edge = edge_pass(&mask);
    let row = mask.height / 2;
    let support = (0..mask.width)
        .filter(|x| edge.at(*x, row).iter().any(|c| *c > 0.0))
        .count();
    assert_eq!(support, 2, "the Sobel itself is a 2 texel step");
    let radius = kernel_radius_px(EDGE_THICKNESS, MAX_EDGE_THICKNESS);
    assert_eq!(radius, 1, "the default thickness kernel is 1 texel");
    let blurred = blur_separable(&edge, OutlineAxis::X, radius);
    let band = (0..mask.width)
        .filter(|x| blurred.at(*x, row).iter().any(|c| *c > 0.0))
        .count();
    assert_eq!(band, 4, "the drawn edge band is 4 px at mask res");
    // The peak keeps the step's magnitude through the blur (sigma 0.5 pulls it
    // under 1), so the band dims at its tails instead of flattening to nothing.
    let peak = blurred.peak().map_or(0.0, |(c, _, _)| c[0]);
    assert!((0.8..=1.0).contains(&peak), "blurred peak {peak}");
}

#[test]
fn outline_thickness_and_glow_kernels_cap_at_four() {
    assert_eq!(kernel_radius_px(EDGE_THICKNESS, MAX_EDGE_THICKNESS), 1);
    assert_eq!(kernel_radius_px(0.0, MAX_EDGE_THICKNESS), 1, "a hairline");
    assert_eq!(kernel_radius_px(-3.0, MAX_EDGE_THICKNESS), 1);
    assert_eq!(
        kernel_radius_px(10.0, MAX_EDGE_THICKNESS),
        MAX_EDGE_THICKNESS,
        "the donor's MAX_EDGE_THICKNESS caps a thick request"
    );
    assert_eq!(kernel_radius_px(2.5, MAX_EDGE_GLOW), 3, "rounded to texels");
    // At the cap the support is the step plus twice the radius, so a thicker
    // request can never draw a wider band than 10 px at mask res.
    let edge = edge_pass(&step_mask());
    let row = edge.height / 2;
    let capped = blur_separable(&edge, OutlineAxis::X, MAX_EDGE_THICKNESS);
    let support = (0..edge.width)
        .filter(|x| capped.at(*x, row).iter().any(|c| *c > 0.0))
        .count();
    assert_eq!(
        support,
        2 + 2 * usize::try_from(MAX_EDGE_THICKNESS).unwrap_or(0)
    );
}

#[test]
fn outline_glow_band_spreads_wider_than_the_thickness_band() {
    let edge = edge_pass(&step_mask());
    let row = edge.height / 2;
    let band = outline_band(&edge, EDGE_THICKNESS, true);
    let width_of = |field: &OutlineEdge| {
        (0..field.width)
            .filter(|x| field.at(*x, row).iter().any(|c| *c > 0.0))
            .count()
    };
    assert!(
        width_of(&band.glow) > width_of(&band.thickness),
        "glow {} vs thickness {}",
        width_of(&band.glow),
        width_of(&band.thickness)
    );
    // A tier that skips the glow pair pays two blurs, not four, and the
    // composite then adds the same field twice less one glow weight.
    let no_glow = outline_band(&edge, EDGE_THICKNESS, false);
    assert_eq!(no_glow.glow, no_glow.thickness);
}

#[test]
fn outline_quality_knob_is_off_on_potato_and_half_on_the_rest() {
    assert_eq!(outline_tier(Quality::Potato), OutlineTier::Off);
    assert_eq!(outline_tier(Quality::Web), OutlineTier::Half);
    assert_eq!(outline_tier(Quality::Mobile), OutlineTier::Half);
    assert_eq!(outline_tier(Quality::Comfort), OutlineTier::Half);
    assert_eq!(outline_tier(Quality::High), OutlineTier::Half);
    // The potato floor spends no draw and no mask texel on it.
    let plan = outline_plan(Quality::Potato, 1920, 1080, 1);
    assert!(!plan.enabled && plan.draws == 0 && plan.mask_size == [0, 0]);
    // A zero-area frame is the suspend path: no pass either.
    assert!(!outline_plan(Quality::High, 0, 720, 1).enabled);
    assert!(!outline_plan(Quality::High, 1280, 0, 1).enabled);
}

#[test]
fn outline_tier_of_knobs_reads_every_table_row() {
    // The presenter holds the knob row, not the tier, so the hook inverts it.
    for tier in Quality::ALL {
        assert_eq!(
            tier_of_knobs(&crate::quality::knobs(tier)),
            tier,
            "{tier:?} round-trips"
        );
    }
    // A row the dynamic resolution governor moved matches no tier: the top tier
    // keeps the outline rather than guessing it off.
    let mut moved = crate::quality::knobs(Quality::Potato);
    moved.render_scale_pct = 3;
    assert_eq!(tier_of_knobs(&moved), Quality::High);
}

#[test]
fn outline_texture_matrix_puts_the_looked_at_point_at_the_frame_centre() {
    let v = view();
    // `0.5 bias * projection * view` on the camera target is the uv centre,
    // which is what the mask pass samples the prefill with.
    assert_eq!(v.project_uv([10.0, 1.5, 0.0]), Some([0.5, 0.5]));
    // A point above the target is above the frame centre, at the same u.
    let up = v.project_uv([10.0, 3.0, 0.0]).expect("above the centre");
    assert!((up[0] - 0.5).abs() < 1e-6 && up[1] < 0.5, "{up:?}");
    // Behind the eye has no uv: the mask pass writes no prefill lookup there.
    assert_eq!(v.project_uv([-1.0, 1.5, 0.0]), None);
    // The centre ray is the camera forward.
    let centre = v.ray(0.5, 0.5).expect("centre ray");
    assert!((centre[0] - 1.0).abs() < 1e-6, "{centre:?}");
    assert!(
        centre[1].abs() < 1e-6 && centre[2].abs() < 1e-6,
        "{centre:?}"
    );
}

#[test]
fn outline_pulse_is_a_cosine_over_the_period() {
    // The donor's pulsePeriod: 1 at the period boundary, 0.25 half way.
    assert!((pulse_scale(3.0, 0.0) - 1.0).abs() < 1e-6);
    assert!((pulse_scale(3.0, 1.5) - 0.25).abs() < 1e-6);
    assert!((pulse_scale(3.0, 3.0) - 1.0).abs() < 1e-6);
    assert_eq!(pulse_scale(0.0, 1.0), 1.0, "no period is a steady band");
    assert_eq!(pulse_scale(3.0, f32::NAN), 1.0, "a bad clock never blinks");
    // Falling from the period start to the trough at half the period ...
    let mut prev = pulse_scale(3.0, 0.0);
    for step in 1..=6u8 {
        let t = 0.25 * f32::from(step);
        let now = pulse_scale(3.0, t);
        assert!(now < prev, "falling at {t}: {now} vs {prev}");
        assert!((0.25..=1.0).contains(&now), "{now} at {t}");
        prev = now;
    }
    // ... and rising back to the next period start.
    let mut prev = pulse_scale(3.0, 1.5);
    for step in 1..=6u8 {
        let t = 1.5 + 0.25 * f32::from(step);
        let now = pulse_scale(3.0, t);
        assert!(now > prev, "rising at {t}: {now} vs {prev}");
        assert!((0.25..=1.0).contains(&now), "{now} at {t}");
        prev = now;
    }
}

#[test]
fn outline_constants_match_the_shader_text() {
    let mask: &str = include_str!("shaders/outline_mask.wgsl");
    let edge: &str = include_str!("shaders/outline_edge.wgsl");
    // The mask writes the override white and the donor's depth-compare slack.
    assert!(
        mask.contains("override MASK_WHITE: f32 = 1.0;"),
        "mask white"
    );
    assert!(
        mask.contains(&format!("const DEPTH_BIAS_M: f32 = {DEPTH_BIAS_M:.2};")),
        "the bias twin"
    );
    assert!(mask.contains("texture_matrix * world"), "the donor matrix");
    assert!(
        mask.contains("1.0 - proj.y"),
        "the bottom-up donor v flips to the texture's top row"
    );
    assert!(mask.contains("fn fs_mask("), "the mask entry point");
    // The edge pass carries the same colours and strengths as this module.
    assert!(edge.contains("const VISIBLE_EDGE_COLOR: vec3<f32> = vec3(1.0, 1.0, 1.0);"));
    assert!(edge.contains("const HIDDEN_EDGE_COLOR: vec3<f32> = vec3(0.1, 0.04, 0.02);"));
    assert!(edge.contains(&format!(
        "override EDGE_STRENGTH: f32 = {EDGE_STRENGTH:.1};"
    )));
    assert!(edge.contains("fn sobel_edge(uv: vec2<f32>) -> f32"));
    assert!(
        edge.contains("fn window_visibility(uv: vec2<f32>) -> f32"),
        "the 3x3 visibility window"
    );
    assert!(edge.contains("fn fs_edge("));
    assert!(edge.contains("fn fs_blur("));
    assert!(edge.contains("fn fs_composite("));
    // The caps the CPU oracle clamps to are the shader's constants.
    assert!(edge.contains(&format!(
        "const MAX_EDGE_THICKNESS: u32 = {MAX_EDGE_THICKNESS}u;"
    )));
    assert!(edge.contains(&format!("const MAX_EDGE_GLOW: i32 = {MAX_EDGE_GLOW};")));
    // The half-res rule the plan uses is named in the mask shader's header.
    assert!(mask.contains("mask pass"));
}

#[test]
fn outline_mask_is_the_same_twice() {
    let targets = [OutlineTarget {
        id: 3,
        center: [9.0, 1.5, 0.0],
        radius_m: 0.6,
    }];
    let first = mask_pass(&view(), &berm_prefill(), &targets, 160, 90);
    let second = mask_pass(&view(), &berm_prefill(), &targets, 160, 90);
    assert_eq!(first, second, "no clock, no map, no order dependence");
    // A target in front of the berm is visible in the mask channel, one behind
    // is not: the whole hidden-colour decision lives in this one number.
    let front = mask_pass(&view(), &berm_prefill(), &targets, 160, 90);
    let behind = mask_pass(
        &view(),
        &berm_prefill(),
        &[OutlineTarget {
            id: 3,
            center: [12.0, 1.5, 0.0],
            radius_m: 0.6,
        }],
        160,
        90,
    );
    let visible_count = |mask: &OutlineMask| {
        let mut n = 0;
        for y in 0..mask.height {
            for x in 0..mask.width {
                if mask.coverage(x, y) > 0.0 && mask.visible_at(x, y) > 0.5 {
                    n += 1;
                }
            }
        }
        n
    };
    assert!(visible_count(&front) > 0, "the near target is visible");
    assert_eq!(visible_count(&behind), 0, "the far target is covered");
}
