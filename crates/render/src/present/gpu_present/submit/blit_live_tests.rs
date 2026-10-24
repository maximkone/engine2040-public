//! GS34 live blit entries: text checks on `blit.wgsl` (the same source the
//! pipelines compile). Kept beside `submit.rs` so that file stays under its
//! line pin. Test names keep their module path:
//! `present::gpu_present::submit::blit_live_tests::<test>`.

use crate::present::gpu_present::BLIT_WGSL;

/// The source of `fn <name>(` through its closing `}` line. Every function in
/// the blit shader closes with `}` at column 0. CRLF checkouts are read as LF
/// first, so the closing-line search matches on Windows too.
fn fn_body(source: &str, name: &str) -> String {
    let text = source.replace("\r\n", "\n");
    let head = format!("fn {name}(");
    let start = text
        .find(&head)
        .unwrap_or_else(|| panic!("blit.wgsl has fn {name}"));
    let rest = &text[start..];
    let end = rest
        .find("\n}\n")
        .unwrap_or_else(|| panic!("fn {name} closes"));
    rest[..end].to_string()
}

#[test]
fn live_entries_never_read_the_fog_texel() {
    // Caller: `fs_live` and `fs_hw_srgb_live` (GS34), picked by the live
    // blit route only while the fog pair is neutral.
    // Behavior: the live chain reads the HDR alone; the shared `composite`
    // keeps its fog read, so the baseline entries are unchanged.
    assert!(
        fn_body(BLIT_WGSL, "composite").contains("fog_tex"),
        "the baseline composite still reads the fog texel"
    );
    for name in ["live_lit", "fs_live", "fs_hw_srgb_live"] {
        let body = fn_body(BLIT_WGSL, name);
        assert!(!body.contains("fog_tex"), "{name} must not read fog_tex");
        assert!(!body.contains("ao_tex"), "{name} must not read ao_tex");
    }
}

#[test]
fn live_entries_keep_the_shared_chain_and_the_encode() {
    // Behavior: `fs_live` matches `fs` step for step (bloom, one tone map,
    // grade, dither, sRGB encode on the write); `fs_hw_srgb_live` leaves the
    // encode to a `*Srgb` view exactly as `fs_hw_srgb` does.
    let live = fn_body(BLIT_WGSL, "fs_live");
    for step in [
        "live_lit(in)",
        "blit_bloom(lit.rgb, in.uv)",
        "display_tonemap(bloomed)",
        "blit_grade(toned)",
        "dither4(px.x, px.y)",
        "linear_to_srgb_term(dithered)",
    ] {
        assert!(live.contains(step), "fs_live keeps `{step}`");
    }
    let hw = fn_body(BLIT_WGSL, "fs_hw_srgb_live");
    assert!(hw.contains("blit_grade(toned)"), "hw live keeps the grade");
    assert!(
        !hw.contains("linear_to_srgb_term"),
        "a *Srgb view encodes on write"
    );
    let lit = fn_body(BLIT_WGSL, "live_lit");
    assert!(
        lit.contains("hdr.rgb * fog.exposure"),
        "neutral fog is the HDR times exposure"
    );
}
