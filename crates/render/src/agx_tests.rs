//! SW50 accept proofs: a saturated 4.0 sand value keeps its hue and more of its
//! chroma under AgX than under the per-channel ACES fit, 0.18 grey lands on
//! 0.18 display, every channel is monotone, and the shipped shader carries the
//! curve and its branch.

use super::*;

/// The arena's sunlit sand in linear radiance: saturated, four times display
/// white. The value the row names.
const SAND: [f32; 3] = [4.0, 2.4, 1.2];

/// SW50 accept: AgX keeps the hue within five degrees of the input and more of
/// the chroma than the per-channel ACES fit, which rotates it further.
#[test]
fn tonemap_agx_keeps_hue_and_more_chroma_than_aces() {
    let input_hue = hue_degrees(SAND);
    let agx = agx_display(SAND);
    let aces = per_channel_aces(SAND);
    let agx_gap = hue_gap(hue_degrees(agx), input_hue);
    let aces_gap = hue_gap(hue_degrees(aces), input_hue);
    // The failing run must name both pairs per sample, so a reader can tell a
    // wrong AgX matrix from a wrong ACES reference in one look: hue in
    // degrees, gap in degrees from the sample's own hue, chroma as max-min.
    for (name, sample) in [
        ("sand", SAND),
        ("sand-dim", [1.0, 0.6, 0.3]),
        ("grey", [0.18, 0.18, 0.18]),
    ] {
        let agx_sample = agx_display(sample);
        let aces_sample = per_channel_aces(sample);
        println!(
            "SW50 {name} in {sample:?} hue {:.2} | AgX {agx_sample:?} hue {:.2} gap {:.2} chroma {:.3} | ACES {aces_sample:?} hue {:.2} gap {:.2} chroma {:.3}",
            hue_degrees(sample),
            hue_degrees(agx_sample),
            hue_gap(hue_degrees(agx_sample), hue_degrees(sample)),
            chroma(agx_sample),
            hue_degrees(aces_sample),
            hue_gap(hue_degrees(aces_sample), hue_degrees(sample)),
            chroma(aces_sample),
        );
    }
    assert!(
        agx_gap <= 5.0,
        "AgX hue {:.2} drifts {agx_gap:.2} deg from {input_hue:.2}, over five: AgX {agx:?} | ACES {aces:?} ACES gap {aces_gap:.2}",
        hue_degrees(agx)
    );
    assert!(
        agx_gap < aces_gap,
        "AgX ({agx_gap:.2} deg) must hold hue better than ACES ({aces_gap:.2} deg): input {SAND:?} hue {input_hue:.2} | AgX {agx:?} hue {:.2} | ACES {aces:?} hue {:.2}",
        hue_degrees(agx),
        hue_degrees(aces)
    );
    assert!(
        chroma(agx) > chroma(aces),
        "AgX chroma {:.3} must beat ACES {:.3}: input {SAND:?} | AgX {agx:?} | ACES {aces:?}",
        chroma(agx),
        chroma(aces)
    );
    // Every channel stays finite and in display range: the curve is the whole
    // transform, so no channel can leave it.
    assert!(agx.iter().all(|v| v.is_finite() && (0.0..=1.0).contains(v)));
    assert!(
        agx[0] > 0.8 && agx[0] < 0.95,
        "the sand's peak lands under display white, not clipped flat: {}",
        agx[0]
    );
}

/// SW50 accept: 0.18 grey maps within half a percent of 0.18 display, and the
/// curve never falls as its input rises.
#[test]
fn tonemap_agx_maps_eighteen_grey_and_stays_monotone() {
    let grey = agx_display([0.18, 0.18, 0.18]);
    for (channel, value) in grey.iter().enumerate() {
        let error = (value - 0.18).abs() / 0.18;
        assert!(
            error <= 0.005,
            "channel {channel} maps to {value}, {:.3}% off 0.18 display",
            error * 100.0
        );
    }
    let mut previous = 0.0_f32;
    for step in 0..=2000 {
        let x = step as f32 * 0.02;
        let y = curve(x);
        assert!(
            y >= previous - 1e-6,
            "curve fell from {previous} to {y} at {x}"
        );
        assert!((0.0..=1.0).contains(&y), "curve({x}) = {y} out of range");
        previous = y;
    }
    assert_eq!(curve(0.0), 0.0, "black stays black");
    assert!(curve(1e6) <= 1.0, "huge values clamp to display white");
}

/// SW50 F2P: the shipped WGSL carries the override, the curve and its branch,
/// the ACES path stays the default, and every constant agrees with the CPU twin.
#[test]
fn tonemap_agx_wgsl_carries_the_curve_and_branch() {
    let source = include_str!("shaders/tonemap.wgsl");
    for piece in [
        "override DISPLAY_AGX: bool = false;",
        "fn agx_curve(",
        "fn agx_display(",
        "if (DISPLAY_AGX) {",
        "const AGX_CROSSOVER: f32 = 0.18;",
    ] {
        assert!(source.contains(piece), "tonemap.wgsl misses `{piece}`");
    }
    // The constants the CPU twin pins and the shader must agree, by value.
    for (name, cpu) in [
        ("AGX_SLOPE", AGX_SLOPE),
        ("AGX_SHOULDER_MAX", AGX_SHOULDER_MAX),
        ("AGX_CROSSOVER", AGX_CROSSOVER),
        ("AGX_TOE_POWER", AGX_TOE_POWER),
    ] {
        let gpu = shader_f32(source, name);
        assert!(
            (gpu - cpu).abs() < 1e-6,
            "shader {name} = {gpu}, CPU twin {cpu}"
        );
    }
    // ACES stays the default arm: the last line of the display entry.
    assert!(
        source.contains("return stone_drain(display_knee(hue_keep_display(color)));"),
        "ACES stays the default curve"
    );
}

/// One shader `const NAME: f32 = <value>;` line's value.
fn shader_f32(source: &str, name: &str) -> f32 {
    let prefix = format!("const {name}: f32 = ");
    let line = source
        .lines()
        .map(str::trim)
        .find(|line| line.starts_with(&prefix))
        .unwrap_or_else(|| panic!("tonemap.wgsl misses `{name}`"));
    line[prefix.len()..]
        .trim_end_matches(';')
        .trim()
        .parse()
        .unwrap_or_else(|_| panic!("`{name}` does not parse: {line}"))
}
