//! draw_id_reduce: SP63 slice b, per-body pixel counts (std-only).
//!
//! Owner: render lane; slice 1 (`draw_id`) writes the target, this slice
//! reduces the readback. Units: texels are `u32` ids in draw-list order;
//! `draw_to_body[draw]` names the fighter slot that staged the draw.
//! Determinism: integer-only, one pass, so same bytes count equal x2.
//! Errors: every add saturates; stray ids skip loudly, never index.
//! Budget: one pass, no allocation; the caller owns all three buffers.
//! GPU pins (read-only, owned elsewhere): `ID_CLEAR` in
//! `present/gpu_present/frame.rs:14`, the base in `submit.rs:826`,
//! the id output in `shaders/forward.wgsl:555`.

/// Clear mirror: texels holding this claimed no draw. Mirrors `ID_CLEAR`
/// (`present/gpu_present/frame.rs:14`); no valid draw id ever equals it,
/// so the reduce never mistakes background for a body.
pub const CLEAR_MIRROR: u32 = u32::MAX;

/// Reduce read-back ids to per-body pixel counts (CPU score for slice c):
/// `body_counts[draw_to_body[draw]]` gains one per texel holding that
/// draw's id. `body_counts.len()` is the body count; texels holding
/// [`CLEAR_MIRROR`], ids past `draw_to_body`, or slots past `body_counts`
/// skip loudly, never index. Returns `(tallied, skipped, background)`.
/// Same shape as `tally_into` in `draw_id`, grouped one level up.
/// One pass, no allocation; the caller reuses `body_counts` per frame.
pub fn reduce_to_bodies(
    ids: &[u32],
    draw_to_body: &[u32],
    body_counts: &mut [u64],
) -> (u64, u64, u64) {
    let mut tallied: u64 = 0;
    let mut skipped: u64 = 0;
    let mut background: u64 = 0;
    for id in ids {
        if *id == CLEAR_MIRROR {
            background = background.saturating_add(1);
            continue;
        }
        let slot = match draw_to_body.get(*id as usize) {
            Some(body) => *body as usize,
            None => {
                skipped = skipped.saturating_add(1);
                continue;
            }
        };
        match body_counts.get_mut(slot) {
            Some(count) => {
                *count = count.saturating_add(1);
                tallied = tallied.saturating_add(1);
            }
            None => {
                skipped = skipped.saturating_add(1);
            }
        }
    }
    (tallied, skipped, background)
}

/// Basis-point scale: 10 000 bp covers every counted texel.
pub const COVERAGE_BP: u32 = 10_000;

/// Visibility floor: 50 bp is half a percent, mirroring the SP63c
/// missing-body floor the later receipt uses.
pub const VISIBLE_BP: u32 = 50;

/// Score per-body pixel counts into basis-point coverage (SP63 slice c):
/// `scores_bp[i]` becomes `body_counts[i] * 10 000 / total_px`, computed
/// in `u64` then saturated to `u32`. Units: counts are texels, scores are
/// basis points. Returns how many scores reach [`VISIBLE_BP`]. Zero when
/// `total_px` is 0 (scores stay 0); length mismatch returns 0 without
/// writing, never panics. Integer-only, one pass, no allocation; same
/// inputs give same scores x2. The caller owns `scores_bp` and reuses it
/// per frame; the SP63c receipt floors missing bodies from these scores.
pub fn score_bodies(body_counts: &[u64], total_px: u64, scores_bp: &mut [u32]) -> usize {
    if body_counts.len() != scores_bp.len() {
        return 0;
    }
    if total_px == 0 {
        for score in scores_bp.iter_mut() {
            *score = 0;
        }
        return 0;
    }
    let mut visible: usize = 0;
    for (count, score) in body_counts.iter().zip(scores_bp.iter_mut()) {
        let scaled = count.saturating_mul(u64::from(COVERAGE_BP)) / total_px;
        let bp = u32::try_from(scaled).map_or(u32::MAX, |v| v);
        *score = bp;
        if bp >= VISIBLE_BP {
            visible += 1;
        }
    }
    visible
}

/// Summarize one frame of read-back ids into per-body counts plus scores:
/// `body_counts` gains per-body texels via [`reduce_to_bodies`], then
/// `scores_bp` gains basis-point coverage via [`score_bodies`] with
/// `total_px` as `ids.len()`. Units: ids are texels, counts are texels,
/// scores are basis points. Returns `(tallied, skipped, background, visible)`.
/// Length mismatch returns 0 visible without writing, never panics.
/// Integer-only, one pass per helper, no allocation; same inputs give
/// same outputs x2. The caller owns both buffers and reuses them per frame.
pub fn summarize_frame(
    ids: &[u32],
    draw_to_body: &[u32],
    body_counts: &mut [u64],
    scores_bp: &mut [u32],
) -> (u64, u64, u64, usize) {
    let (tallied, skipped, background) = reduce_to_bodies(ids, draw_to_body, body_counts);
    let total_px = u64::try_from(ids.len()).map_or(u64::MAX, |v| v);
    let visible = score_bodies(body_counts, total_px, scores_bp);
    (tallied, skipped, background, visible)
}

/// Fill a caller-owned draw-to-body map with the identity (`draw_to_body[i]`
/// is body `i`) so [`summarize_frame`] can score a frame without a
/// hand-written map. Units: entries are fighter slots in draw-list order.
/// Integer-only, one pass, no allocation; same length gives same bytes x2;
/// empty stays empty. Indices below `u32::MAX` never equal
/// [`CLEAR_MIRROR`]; larger indices fall back to `CLEAR_MIRROR - 1`.
pub fn fill_identity_body_map(draw_to_body: &mut [u32]) {
    for (index, slot) in draw_to_body.iter_mut().enumerate() {
        *slot = u32::try_from(index).unwrap_or(CLEAR_MIRROR - 1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mirror_pins_max_and_holds_no_valid_id() {
        assert_eq!(CLEAR_MIRROR, u32::MAX);
        assert_ne!(u32::MAX - 1, CLEAR_MIRROR);
    }

    #[test]
    fn two_draws_group_into_one_body() {
        let ids = [0, 1, 0, 1];
        let map = [0, 0];
        let mut bodies = [0u64];
        assert_eq!(reduce_to_bodies(&ids, &map, &mut bodies), (4, 0, 0));
        assert_eq!(bodies, [4]);
    }

    #[test]
    fn clear_texels_count_as_background() {
        let ids = [CLEAR_MIRROR, 0, CLEAR_MIRROR];
        let map = [0];
        let mut bodies = [0u64];
        assert_eq!(reduce_to_bodies(&ids, &map, &mut bodies), (1, 0, 2));
        assert_eq!(bodies, [1]);
    }

    #[test]
    fn stray_draw_and_stray_slot_skip_loudly() {
        let ids = [7, 0, 1];
        let map = [0, 5];
        let mut bodies = [0u64];
        assert_eq!(reduce_to_bodies(&ids, &map, &mut bodies), (1, 2, 0));
        assert_eq!(bodies, [1]);
    }

    #[test]
    fn rerun_matches_and_empty_stays_empty() {
        let ids = [0, CLEAR_MIRROR, 1];
        let map = [0, 1];
        let mut first = [0u64, 0u64];
        let mut second = [0u64, 0u64];
        let a = reduce_to_bodies(&ids, &map, &mut first);
        let b = reduce_to_bodies(&ids, &map, &mut second);
        assert_eq!((a, first), (b, second));
        let mut bodies = [9u64];
        let empty: [u32; 0] = [];
        assert_eq!(reduce_to_bodies(&empty, &map, &mut bodies), (0, 0, 0));
        assert_eq!(bodies, [9]);
    }

    #[test]
    fn score_bodies_covers_half_percent_floor() {
        let counts = [50u64, 49, 0];
        let mut scores = [0u32; 3];
        let visible = score_bodies(&counts, 10_000, &mut scores);
        assert_eq!(scores, [50, 49, 0]);
        assert_eq!(visible, 1);
    }

    #[test]
    fn score_bodies_zero_total_stays_zero() {
        let counts = [7u64, 3];
        let mut scores = [9u32, 9];
        let visible = score_bodies(&counts, 0, &mut scores);
        assert_eq!(scores, [0, 0]);
        assert_eq!(visible, 0);
        let mut short = [7u32; 1];
        assert_eq!(score_bodies(&counts, 10, &mut short), 0);
        assert_eq!(short, [7]);
    }

    #[test]
    fn score_bodies_rerun_matches_and_saturates() {
        let counts = [50u64, 9_999];
        let mut first = [0u32; 2];
        let mut second = [0u32; 2];
        let a = score_bodies(&counts, 10_000, &mut first);
        let b = score_bodies(&counts, 10_000, &mut second);
        assert_eq!((a, first), (b, second));
        let big = [u64::MAX];
        let mut scores = [0u32];
        assert_eq!(score_bodies(&big, 1, &mut scores), 1);
        assert_eq!(scores, [u32::MAX]);
    }

    #[test]
    fn summarize_groups_and_scores_half_percent() {
        let ids = [0, 1, 0, CLEAR_MIRROR];
        let map = [0, 0];
        let mut bodies = [0u64];
        let mut scores = [0u32];
        let summary = summarize_frame(&ids, &map, &mut bodies, &mut scores);
        assert_eq!(summary, (3, 0, 1, 1));
        assert_eq!(bodies, [3]);
        assert_eq!(scores, [7_500]);
    }

    #[test]
    fn identity_map_covers_eight_twin_slots() {
        let mut map = [0u32; 8];
        fill_identity_body_map(&mut map);
        assert_eq!(map, [0, 1, 2, 3, 4, 5, 6, 7]);
        let ids = [0, 1, 2, 3, 4, 5, 6, 7];
        let mut bodies = [0u64; 8];
        let mut scores = [0u32; 8];
        let summary = summarize_frame(&ids, &map, &mut bodies, &mut scores);
        assert_eq!(summary, (8, 0, 0, 8));
        assert_eq!(bodies, [1, 1, 1, 1, 1, 1, 1, 1]);
        assert_eq!(scores, [1250, 1250, 1250, 1250, 1250, 1250, 1250, 1250]);
    }

    #[test]
    fn identity_map_rerun_matches_and_empty_stays_empty() {
        let mut first = [0u32; 8];
        let mut second = [0u32; 8];
        fill_identity_body_map(&mut first);
        fill_identity_body_map(&mut second);
        assert_eq!(first, second);
        let mut empty: [u32; 0] = [];
        fill_identity_body_map(&mut empty);
        assert_eq!(empty, [0u32; 0]);
    }

    #[test]
    fn summarize_empty_stays_empty() {
        let ids: [u32; 0] = [];
        let map = [0];
        let mut bodies = [0u64];
        let mut scores = [0u32];
        let summary = summarize_frame(&ids, &map, &mut bodies, &mut scores);
        assert_eq!(summary, (0, 0, 0, 0));
        assert_eq!(bodies, [0]);
        assert_eq!(scores, [0]);
    }

    #[test]
    fn summarize_rerun_matches() {
        let ids = [0, CLEAR_MIRROR, 1, 0];
        let map = [0, 1];
        let mut first_bodies = [0u64, 0u64];
        let mut first_scores = [0u32, 0u32];
        let mut second_bodies = [0u64, 0u64];
        let mut second_scores = [0u32, 0u32];
        let a = summarize_frame(&ids, &map, &mut first_bodies, &mut first_scores);
        let b = summarize_frame(&ids, &map, &mut second_bodies, &mut second_scores);
        assert_eq!(a, b);
        assert_eq!(first_bodies, second_bodies);
        assert_eq!(first_scores, second_scores);
    }
}
