//! ssr tests: the row's accept numbers (SW90), on the CPU twin alone.

use super::*;

/// The row's frame: 1280x720, the potato target 640x360.
const W: u32 = 1280;
const H: u32 = 720;

/// A depth frame with three bands: sky on top, a wall, the floor under the
/// camera. Values are view depth in meters.
fn sky_wall_floor(w: u32, h: u32, sky: f32, wall: f32, floor: f32) -> Vec<f32> {
    let mut depth = vec![sky; (w * h) as usize];
    let wall_top = h / 4;
    let floor_top = h / 2;
    for y in wall_top..h {
        for x in 0..w {
            let d = if y < floor_top { wall } else { floor };
            depth[(y * w + x) as usize] = d;
        }
    }
    depth
}

/// SW90 F2P: at 1280x720 the half target is 640x360 = 230 400 texels, a quarter
/// of the 921 600 a full-res pass touches, and the half march is the row's 8
/// taps against the full march's 16.
#[test]
fn ssr_half_target_is_a_quarter_of_the_texels() {
    let full = SsrConfig::full(W, H);
    let half = SsrConfig::half(W, H);
    assert_eq!(half.target(true), [640, 360]);
    assert_eq!(full.target(false), [1280, 720]);
    assert_eq!(half.half_texels(), 230_400);
    assert_eq!(half.full_texels(), 921_600);
    assert_eq!(
        half.half_texels() * 4,
        half.full_texels(),
        "the half target is a quarter of the full pass"
    );
    assert_eq!(half.steps, SSR_STEPS_HALF);
    assert_eq!(full.steps, SSR_STEPS_FULL);
    assert_eq!(
        (half.steps, full.steps),
        (8, 16),
        "potato 8 taps, 16 elsewhere"
    );
    // An odd frame rounds up, so the last row and column still trace.
    let odd = SsrConfig::half(1279, 721);
    assert_eq!(odd.target(true), [640, 361]);
}

/// The march never walks past its step budget: a ray that is in front of the
/// floor the whole way reads every tap and then gives up (8 half, 16 full).
#[test]
fn ssr_march_stops_at_its_step_budget() {
    let depth = vec![1.0f32; (64 * 36) as usize];
    for (config, want) in [
        (SsrConfig::half(64, 36), SSR_STEPS_HALF),
        (SsrConfig::full(64, 36), SSR_STEPS_FULL),
    ] {
        // Depth grows at 0.5 m per unit t, so the ray stays in front of the
        // 1.0 m plane until t = 2, past the clipped ray.
        let ray = SsrRay {
            x_px: 32.0,
            y_px: 18.0,
            depth_m: 0.1,
            dx_px: 0.0,
            dy_px: 0.0,
            dz_m: 0.5,
        };
        let hit = march(&depth, &config, ray).expect("the flat frame marches");
        assert!(!hit.hit, "a ray in front of the plane misses");
        assert_eq!(hit.steps, want, "the march stops at the tier's taps");
        assert!(hit.steps <= config.steps);
        assert!(hit.t <= 1.0 && hit.t > 0.0);
    }
}

/// SW90 F2P: a mirror of the wall under the camera returns the wall texel, not
/// the sky, and twins of the same degenerate depth step hit at the same `t`.
#[test]
fn ssr_march_hits_the_wall_at_the_binary_twins_t() {
    const SKY: f32 = 900.0;
    const WALL: f32 = 1.6;
    const FLOOR: f32 = 1.0;
    let depth = sky_wall_floor(64, 36, SKY, WALL, FLOOR);
    let config = SsrConfig::half(64, 36);
    // The floor pixel under the camera reflects up the screen and back into
    // the wall; the sky sits above the wall's top row.
    let ray = SsrRay {
        x_px: 32.0,
        y_px: 30.0,
        depth_m: FLOOR,
        dx_px: 0.0,
        dy_px: -18.0,
        dz_m: 0.7,
    };
    let a = march(&depth, &config, ray).expect("the wall frame marches");
    let b = march(&depth, &config, ray).expect("the twin marches the same");
    assert!(a.hit, "the mirror ray lands on the wall");
    assert_eq!(a.depth_m, WALL, "the hit is the wall, not the sky");
    assert_eq!(
        (a.x_px, a.y_px),
        (b.x_px, b.y_px),
        "the twins return the same texel"
    );
    assert_eq!(a.t, b.t, "the twins hit at the same t");
    assert_eq!(a.depth_m, depth[(a.y_px * 64 + a.x_px) as usize]);
    assert!(a.y_px < 36 / 2, "the wall row sits above the floor");
    assert_ne!(a.depth_m, SKY);

    // Degenerate: a depth buffer that never moves and a ray with no depth step.
    // The step position is depth-independent, so both runs hit at tap 0's `t`.
    let flat = vec![0.0f32; (64 * 36) as usize];
    let degenerate = SsrRay {
        x_px: 32.0,
        y_px: 18.0,
        depth_m: 0.001,
        dx_px: 0.0,
        dy_px: 0.0,
        dz_m: 0.0,
    };
    let a = march(&flat, &config, degenerate).expect("the flat twin marches");
    let b = march(&flat, &config, degenerate).expect("the flat twin marches again");
    assert!(a.hit && b.hit, "a zero-thickness step still hits");
    assert_eq!(a.t, b.t, "the degenerate step hits at the same t twice");
    assert_eq!(a.t, config.hit_t(0));
}

/// A rough pixel and a pure dielectric trace nothing; the blend fades in off
/// the surface and out with distance, and a miss is zero.
#[test]
fn ssr_resolve_blend_cuts_roughness_and_fades() {
    let config = SsrConfig::half(W, H);
    assert!(!config.pixel_runs(SSR_ROUGHNESS_CUT, 0.5), "0.7 is the cut");
    assert!(config.pixel_runs(0.69, 0.5));
    assert!(!config.pixel_runs(0.2, 0.0), "no metalness, no reflection");

    assert_eq!(
        config.resolve_blend(0.5, 0.7),
        0.0,
        "rough stone mirrors nothing"
    );
    assert_eq!(config.resolve_blend(0.5, 0.95), 0.0);
    assert_eq!(
        config.resolve_blend(0.0, 0.1),
        0.0,
        "no fade-in at the surface"
    );
    let mid = config.resolve_blend(0.4, 0.1);
    assert!(mid > 0.0 && mid < 1.0, "mid ray partially blends: {mid}");
    assert_eq!(
        config.resolve_blend(config.distance_fade, 0.1),
        0.0,
        "the distance fade reaches zero at its own t"
    );
    assert_eq!(config.resolve_blend(f32::NAN, 0.1), 0.0);
}

/// Hi-Z level step: inside geometry coarsens (-1), in front refines (+1), and
/// the level never leaves `0..=max_level`.
#[test]
fn ssr_mip_offset_steps_the_level_within_bounds() {
    assert_eq!(mip_offset(true), -1);
    assert_eq!(mip_offset(false), 1);
    assert_eq!(next_mip_level(0, true, 8), 0, "level 0 stays at 0");
    assert_eq!(next_mip_level(3, false, 8), 4);
    assert_eq!(next_mip_level(8, false, 8), 8, "the coarsest level holds");
    assert_eq!(next_mip_level(1, true, 8), 0);
}

/// `t_max` clips the ray at the screen edge so no tap reads outside the frame.
#[test]
fn ssr_t_max_clips_at_the_screen_edge() {
    let ray = SsrRay {
        x_px: 10.0,
        y_px: 18.0,
        depth_m: 1.0,
        dx_px: 100.0,
        dy_px: 0.0,
        dz_m: 0.0,
    };
    let t = t_max(ray, 64, 36);
    assert!((t - 0.53).abs() < 1e-6, "x stops at column 63: {t}");
    let still = SsrRay {
        dx_px: 0.0,
        dy_px: 0.0,
        ..ray
    };
    assert_eq!(t_max(still, 64, 36), 1.0);
}

/// The WGSL twin ships and passes the CPU naga gate, so the pass cannot land
/// as a file no device would compile.
#[test]
fn ssr_wgsl_traces_through_the_shader_gate() {
    let src = include_str!("shaders/ssr.wgsl");
    assert!(src.contains("fn ssr_trace"), "the entry is ssr_trace");
    assert!(src.contains("@workgroup_size(8, 8, 1)"), "compute 8x8");
    // The naga gate is a feature of its own (`shader_gate`), so the row's
    // `--features window` line only reads the source shape here; the run with
    // `shader-validate` reaches naga, and `shader_gate`'s own walk covers it too.
    #[cfg(feature = "shader-validate")]
    crate::shader_gate::validate_source("ssr.wgsl", src).expect("ssr.wgsl validates");
}

/// The tier knob: reflections are off on Potato (the row's default), half on
/// the mid rows, full only on High; at 1280x720 that is 640x360 with 8 taps
/// against 1280x720 with 16.
#[test]
fn ssr_level_is_off_on_potato_and_sized_per_tier() {
    assert_eq!(level_for(Quality::Potato), SsrLevel::Off);
    assert!(config_for(Quality::Potato, 1280, 720).is_none());
    assert_eq!(level_for(Quality::Mobile), SsrLevel::Half);
    assert_eq!(level_for(Quality::Web), SsrLevel::Half);
    assert_eq!(level_for(Quality::Comfort), SsrLevel::Half);
    assert_eq!(level_for(Quality::High), SsrLevel::Full);
    assert_eq!(SsrLevel::Off.as_u8(), 0);
    assert!(!SsrLevel::Off.enabled() && SsrLevel::Half.enabled() && SsrLevel::Full.enabled());
    let web = config_for(Quality::Web, W, H).expect("web traces");
    assert_eq!(web.target(true), [640, 360]);
    assert_eq!(web.steps, SSR_STEPS_HALF);
    let high = config_for(Quality::High, W, H).expect("high traces");
    assert_eq!(high.target(false), [1280, 720]);
    assert_eq!(high.steps, SSR_STEPS_FULL);
    // The cut order names the pass this knob runs.
    assert!(crate::quality::CUT_ORDER.contains(&"ssr"));
}

#[test]
fn ssr_refuses_bad_config_and_depth() {
    let mut config = SsrConfig::half(W, H);
    config.steps = 0;
    assert!(config.validate().is_err());
    config.steps = SSR_STEPS_HALF;
    config.distance_fade = 1.5;
    assert!(config.validate().is_err());
    config.distance_fade = SSR_DISTANCE_FADE;
    config.roughness_cut = 0.0;
    assert!(config.validate().is_err());
    let ok = SsrConfig::half(64, 36);
    ok.validate().expect("the row's config is valid");
    let short = vec![1.0f32; 10];
    let ray = SsrRay {
        x_px: 1.0,
        y_px: 1.0,
        depth_m: 1.0,
        dx_px: 1.0,
        dy_px: 1.0,
        dz_m: 0.1,
    };
    assert!(
        march(&short, &ok, ray).is_err(),
        "a short depth buffer refuses"
    );
}
