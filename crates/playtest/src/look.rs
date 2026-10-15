//! Look harness over the production compose rankings.
//! Owner: playtest crate. Units: composition totals `0..100`, panel scores as
//! `u32` ranks-highest-best, agreement in permille (`-1000..=1000`).
//! Determinism: delegates to `engine_spatial::compose` order walks; the same
//! panel twice ranks the same on every build. Errors: ragged or tiny panels
//! refuse as [`ComposeError::BadPanel`]. Edge: playtest -> spatial (compose
//! only, std-only). Budget: one order pass plus one agreement pass.

use engine_spatial::compose::{rank_order, spearman_permille, ComposeError};

pub mod judge;
pub mod manifest;
pub mod metric;

/// Per-frame look hook the play host owns, re-exported for the window host.
///
/// Hands each validated PNG to [`LookHook::note_frame`] once per frame.
/// Caller: `crates/game/src/bin/arena.rs` `capture_offscreen` (remainder).
pub use judge::{HookScore, LookHook};

/// Shots in the review panel the harness ranks.
pub const PANEL_SHOTS: usize = 20;
/// Review bar for panel agreement, in permille (mirrors the compose gate).
pub const REVIEW_BAR_PERMILLE: i32 = 700;

/// Ranked look panel: best-first order plus agreement with the review panel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LookReport {
    /// Shot indices best-first; ties break toward the lower index.
    pub order: Vec<usize>,
    /// Spearman agreement between panel and computed totals, in permille.
    pub rho_permille: i32,
    /// True when agreement clears [`REVIEW_BAR_PERMILLE`].
    pub passes_bar: bool,
}

/// Rank one look panel through the production compose order.
///
/// Calls [`rank_order`] over `computed` for the best-first order and
/// [`spearman_permille`] against `panel` for the agreement, then checks the
/// review bar. Both slices align by shot; higher is better on each side.
///
/// # Errors
///
/// [`ComposeError::BadPanel`] when the slices disagree in length or hold
/// fewer than two shots.
///
/// # Determinism
///
/// Pure order math; the same panel twice gives the same report.
pub fn rank_look_panel(panel: &[u32], computed: &[u32]) -> Result<LookReport, ComposeError> {
    let rho_permille = spearman_permille(panel, computed)?;
    let order = rank_order(computed);
    Ok(LookReport {
        order,
        passes_bar: rho_permille >= REVIEW_BAR_PERMILLE,
        rho_permille,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_spatial::compose::{rank_order, spearman_permille};

    fn review_panel() -> [u32; PANEL_SHOTS] {
        let mut panel = [0u32; PANEL_SHOTS];
        for (i, slot) in panel.iter_mut().enumerate() {
            let rank = PANEL_SHOTS - i;
            *slot = u32::try_from(rank).expect("panel rank fits u32");
        }
        panel
    }

    fn graded_totals() -> [u32; PANEL_SHOTS] {
        let mut totals = [0u32; PANEL_SHOTS];
        for (i, slot) in totals.iter_mut().enumerate() {
            let score = 95u32 - u32::try_from(i).expect("shot index fits u32");
            *slot = score;
        }
        totals
    }

    #[test]
    fn harness_order_matches_direct_compose_order() {
        let panel = review_panel();
        let totals = graded_totals();
        let report = rank_look_panel(&panel, &totals).expect("panel ranks");
        assert_eq!(report.order, rank_order(&totals));
        assert_eq!(report.order, (0..PANEL_SHOTS).collect::<Vec<usize>>());
    }

    #[test]
    fn harness_agreement_matches_direct_spearman() {
        let panel = review_panel();
        let totals = graded_totals();
        let report = rank_look_panel(&panel, &totals).expect("panel ranks");
        let direct = spearman_permille(&panel, &totals).expect("direct rho");
        assert_eq!(report.rho_permille, direct);
        assert_eq!(direct, 1000);
        assert!(report.passes_bar);
    }

    #[test]
    fn reversed_panel_fails_the_review_bar() {
        let panel = review_panel();
        let mut flipped = graded_totals();
        flipped.reverse();
        let report = rank_look_panel(&panel, &flipped).expect("flipped ranks");
        assert_eq!(report.order, rank_order(&flipped));
        assert_eq!(report.rho_permille, -1000);
        assert!(!report.passes_bar);
    }

    #[test]
    fn ragged_and_tiny_panels_refuse() {
        let panel = review_panel();
        let totals = graded_totals();
        assert_eq!(
            rank_look_panel(&panel[..10], &totals).expect_err("ragged refuses"),
            ComposeError::BadPanel
        );
        assert_eq!(
            rank_look_panel(&[7u32], &[9u32]).expect_err("single refuses"),
            ComposeError::BadPanel
        );
    }

    #[test]
    fn same_panel_twice_reports_identically() {
        let panel = review_panel();
        let totals = graded_totals();
        let first = rank_look_panel(&panel, &totals).expect("first ranks");
        let second = rank_look_panel(&panel, &totals).expect("second ranks");
        assert_eq!(first, second);
    }

    #[test]
    fn look_metric_play_host_hands_each_frame_once() {
        use super::metric::{Frame, Region, SSIM_ONE};
        use image::{codecs::png::PngEncoder, ExtendedColorType, ImageEncoder};

        const W: u32 = 64;
        const H: u32 = 48;
        fn sand_at(x: u32, y: u32) -> [u8; 3] {
            let t = i32::try_from((x * 7 + y * 13 + (x * y) % 5) % 23).expect("small") - 11;
            [194i32, 160, 104].map(|c| u8::try_from((c + t).clamp(0, 255)).expect("clamped"))
        }
        fn rgba_of(paint: impl Fn(u32, u32) -> [u8; 3]) -> Vec<u8> {
            let mut rgba = Vec::new();
            for y in 0..H {
                for x in 0..W {
                    let [r, g, b] = paint(x, y);
                    rgba.extend_from_slice(&[r, g, b, 255]);
                }
            }
            rgba
        }
        fn png_of(rgba: &[u8]) -> Vec<u8> {
            let mut out = Vec::new();
            PngEncoder::new(&mut out)
                .write_image(rgba, W, H, ExtendedColorType::Rgba8)
                .expect("png encodes");
            out
        }

        let reference = png_of(&rgba_of(sand_at));
        let brightened = png_of(&rgba_of(|x, y| {
            if y >= 8 {
                sand_at(x, y)
                    .map(|v| u8::try_from(((u32::from(v) * 6 + 2) / 5).min(255)).expect("clamped"))
            } else {
                sand_at(x, y)
            }
        }));

        // A frame equal to its reference scores SSIM 10000 and deltaE 0.
        let frame = Frame::decode_png(&reference).expect("reference decodes");
        let same = super::metric::compare(&frame, &frame, Region::Full).expect("equal compares");
        assert_eq!((same.ssim_x10000, same.mean_de_x100), (SSIM_ONE, 0));

        // The sand region brightened 20% scores SSIM below 9800.
        let live = Frame::decode_png(&brightened).expect("live decodes");
        let diff =
            super::metric::compare(&live, &frame, Region::Full).expect("brightened compares");
        assert!(
            diff.ssim_x10000 < 9800,
            "brightened sand must drop SSIM, got {}",
            diff.ssim_x10000
        );

        // The hook runs once per captured frame.
        let mut hook = super::LookHook::register(Some(&reference));
        let one = hook.note_frame(&reference);
        let two = hook.note_frame(&brightened);
        assert_eq!((one.calls, two.calls, hook.calls()), (1, 2, 2));
        assert_eq!(one.comparison.expect("scores").ssim_x10000, SSIM_ONE);
        assert!(
            two.comparison.expect("scores").ssim_x10000 < 9800,
            "hook scores the brightened frame below 9800"
        );
    }
}
