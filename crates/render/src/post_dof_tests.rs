//! post_dof tests: the row's accept numbers (SW173), on the CPU twin alone.

use super::*;

/// 16x16 frame: four RGB lanes at four depths, one half texel per four pixels.
fn quad_frame(focus: f32) -> (Vec<f32>, Vec<f32>, DofConfig) {
    const W: u32 = 8;
    const H: u32 = 8;
    let mut depth = vec![focus; (W * H) as usize];
    let mut colour = vec![0.0f32; (W * H * 4) as usize];
    for y in 0..H as usize {
        for x in 0..W as usize {
            let i = y * W as usize + x;
            // Left half at the focus, right half far behind it, with the far
            // side's red rising per column so a gather of neighbours cannot
            // equal the centre texel by accident.
            let (d, c) = if x < 4 {
                (focus, [0.2, 0.4, 0.6])
            } else {
                (focus + 10.0, [0.3 + 0.05 * (x - 4) as f32, 0.2, 0.1])
            };
            depth[i] = d;
            colour[i * 4] = c[0];
            colour[i * 4 + 1] = c[1];
            colour[i * 4 + 2] = c[2];
            colour[i * 4 + 3] = 1.0;
        }
    }
    (colour, depth, DofConfig::focused(W, H, focus))
}

/// SW173 accept: the pass is off at Potato and off outside the beat, so the
/// arena pays nothing for a look it never shows.
#[test]
fn dof_runs_only_in_the_beat_and_never_at_potato() {
    assert!(!dof_runs(false, Quality::High), "no beat, no focus");
    assert!(!dof_runs(true, Quality::Potato), "potato keeps no DOF");
    for tier in [
        Quality::Mobile,
        Quality::Web,
        Quality::Comfort,
        Quality::High,
    ] {
        assert!(dof_runs(true, tier), "{tier:?} focuses in the beat");
    }
    // The half target is a quarter of the frame's texels and both dispatches
    // cover it in 8x8 workgroups.
    let config = DofConfig::focused(1280, 720, 3.0);
    assert_eq!((config.half_width(), config.half_height()), (640, 360));
    assert_eq!(config.half_texels(), 230_400);
    let plan = dof_pass_plan(&config).expect("the row's frame plans");
    assert_eq!(plan.coc_groups, [80, 45]);
    assert_eq!(plan.gather_groups, plan.coc_groups);
    assert_eq!(plan.width_half, 640);
}

/// SW173 accept: a pixel 10 m behind the focus blurs (radius > 2 px) and the
/// focus pixel comes through unchanged, bit for bit.
#[test]
fn dof_blurs_ten_metres_behind_and_leaves_the_focus_unaf_fected() {
    let focus = 3.0;
    let (colour, depth, config) = quad_frame(focus);
    assert_eq!(config.coc_px(focus), 0.0, "the focus plane is sharp");
    let far = config.coc_px(focus + 10.0);
    assert!(far > 2.0, "10 m behind the focus blurs: radius {far} px");
    assert!(far <= config.max_coc_px);

    let image = dof_half_res(&colour, &depth, &config).expect("the frame gathers");
    // A half texel on the left is the focus plane: exact copy, no blend.
    let sharp = image.texel(0, 0).expect("texel");
    assert_eq!(sharp, [0.2, 0.4, 0.6], "the focus pixel is unchanged");
    assert_eq!(image.texel(1, 0).expect("texel"), sharp, "row stays sharp");
    // A half texel on the right is 10 m behind: it blends its far neighbours,
    // so it is neither its own colour nor a copy.
    let centre = 0.3 + 0.05 * 2.0f32; // full-res column 6, the tap at (3, 2)
    let soft = image.texel(3, 2).expect("texel");
    assert!(
        (soft[0] - centre).abs() > 1e-6,
        "the far texel is gathered, not copied: {soft:?}"
    );
    assert!(
        soft[2] < 0.2,
        "the far blend keeps the blue it gathered: {soft:?}"
    );
    // And the pass is reproducible: twins of the same frame match to the bit.
    let twin = dof_half_res(&colour, &depth, &config).expect("the twin gathers");
    assert_eq!(image, twin, "the same frame gathers alike");
}

/// A focus depth of zero and a bad frame refuse loudly before any gather.
#[test]
fn dof_refuses_bad_frames() {
    assert!(DofConfig::focused(8, 8, 0.0).validate().is_err());
    assert!(DofConfig::focused(0, 8, 3.0).validate().is_err());
    let mut config = DofConfig::focused(8, 8, 3.0);
    config.aperture_px = 0.0;
    assert!(config.validate().is_err());
    config.aperture_px = DOF_APERTURE_PX;
    let colour = vec![0.0f32; 4];
    let depth = vec![3.0f32; 1];
    assert!(
        dof_half_res(&colour, &depth, &config).is_err(),
        "short buffers refuse"
    );
    let (colour, depth, config) = quad_frame(3.0);
    let image = dof_half_res(&colour, &depth, &config).expect("the frame gathers");
    assert!(
        image.texel(8, 0).is_err(),
        "a texel outside the half target refuses"
    );
}

/// SW173 accept: the on-vs-off frame differs where the blur reaches, and the
/// off frame is the source pixel itself.
#[test]
fn dof_on_and_off_frames_differ_at_the_blurred_pixels() {
    let focus = 3.0;
    let (colour, depth, config) = quad_frame(focus);
    // Off (no beat, or Potato): the pass is skipped and the frame is untouched.
    for (holds, tier) in [(false, Quality::High), (true, Quality::Potato)] {
        assert!(
            !dof_runs(holds, tier),
            "{holds} {tier:?} keeps the frame sharp"
        );
    }
    // On: the far half changes, the focused half does not.
    assert!(dof_runs(true, Quality::High));
    let image = dof_half_res(&colour, &depth, &config).expect("the frame gathers");
    let source = |hx: u32, hy: u32| {
        let (cx, cy) = ((hx * 2) as usize, (hy * 2) as usize);
        let i = (cy * 8 + cx) * 4;
        [colour[i], colour[i + 1], colour[i + 2]]
    };
    assert_eq!(
        image.texel(0, 2).expect("texel"),
        source(0, 2),
        "focus half"
    );
    assert_ne!(image.texel(3, 2).expect("texel"), source(3, 2), "far half");
}

/// The shader ships as the two dispatches this module plans: CoC then gather.
#[test]
fn dof_wgsl_names_the_coc_and_gather_entries() {
    let src = include_str!("shaders/dof.wgsl");
    assert!(src.contains("fn dof_coc"), "the CoC entry ships");
    assert!(src.contains("fn dof_gather"), "the gather entry ships");
    assert!(src.contains("@workgroup_size(8, 8, 1)"), "compute 8x8");
}
