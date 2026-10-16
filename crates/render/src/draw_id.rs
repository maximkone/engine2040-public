//! draw_id: SP63 slice 1 CPU contract for the draw-id target (std-only).
//!
//! Owner: render lane; slice 1 writes the target, slices b/c reduce it.
//! Units: ids are plain `u32` texels, one per pixel, in draw-list order.
//! Determinism: integer-only mapping, so same list gives same ids x2.
//! Errors: saturation keeps every id below [`CLEAR`], never a wrap.
//! Budget: mapping is O(1); the tally is one pass, caller-owned buffer.
//! GPU glue (format, clear, second attachment) lives once in
//! `present/gpu_present/frame.rs` plus `shaders/forward.wgsl`; this module
//! owns the CPU mapping and the readback oracle, nowhere else.

/// Texel no draw claims: the id target clears to this, and no valid id
/// ever equals it. Mirrors `ID_CLEAR` in `present/gpu_present/frame.rs`.
pub const CLEAR: u32 = u32::MAX;

/// Largest id a draw may carry; one below [`CLEAR`] so saturation can
/// never collide with the clear value.
pub const MAX_VALID_ID: u32 = u32::MAX - 1;

/// Id for the draw at `index` in the submitted list: the index itself,
/// saturated at [`MAX_VALID_ID`]. Fighter parts are tagged with their
/// slot by list position: the caller stages one draw per fighter slot in
/// slot order, so the id reads back as the slot. Never returns [`CLEAR`].
#[must_use]
pub fn id_of_draw(index: usize) -> u32 {
    let max = MAX_VALID_ID as usize;
    if index >= max {
        MAX_VALID_ID
    } else {
        index as u32
    }
}

/// Id for fighter slot `slot`: the slot itself, saturated at
/// [`MAX_VALID_ID`]. The caller stages one draw per fighter slot in slot
/// order, so the id reads back as the slot. Never returns [`CLEAR`].
#[must_use]
pub fn id_of_fighter_slot(slot: u8) -> u32 {
    let id = u32::from(slot);
    if id >= MAX_VALID_ID {
        MAX_VALID_ID
    } else {
        id
    }
}

/// True when the read-back texel holds [`CLEAR`], meaning no draw landed
/// on that pixel.
#[must_use]
pub fn is_clear(id: u32) -> bool {
    id == CLEAR
}

/// Tally read-back ids into per-draw pixel counts (CPU oracle for the
/// slice-b GPU reduction): `counts[draw]` gains one per texel holding
/// that draw's id. `counts.len()` is the draw count; texels holding
/// [`CLEAR`] or an id outside the list are skipped, never indexed.
/// Returns `(tallied, skipped)` so the caller sees dropped texels loudly.
/// One pass, no allocation; the caller reuses `counts` across frames.
pub fn tally_into(ids: &[u32], counts: &mut [u64]) -> (u64, u64) {
    let mut tallied: u64 = 0;
    let mut skipped: u64 = 0;
    for id in ids {
        if is_clear(*id) {
            skipped = skipped.saturating_add(1);
            continue;
        }
        let slot = *id as usize;
        if slot < counts.len() {
            counts[slot] = counts[slot].saturating_add(1);
            tallied = tallied.saturating_add(1);
        } else {
            skipped = skipped.saturating_add(1);
        }
    }
    (tallied, skipped)
}

/// Verify the offscreen box-plus-plane partition through the tally oracle:
/// `counts` holds two draws (box at 0, plane at 1); `ids` holds box, plane
/// or clear texels. Calls [`tally_into`] once, then reads box and plane
/// hits back. Returns `(tallied, skipped, box_hits, plane_hits, partitioned)`
/// where `partitioned` is true when `counts` has length 2, `tallied` equals
/// box plus plane, `tallied` plus `skipped` covers every texel, and no stray
/// id (outside box, plane or clear) appears. Units are
/// texels in, counts out. Integer-only, one pass via the oracle, no
/// allocation; same inputs give same outputs x2. The caller owns `counts`
/// and reuses it across frames.
#[must_use]
pub fn verify_box_plane_partition(ids: &[u32], counts: &mut [u64]) -> (u64, u64, u64, u64, bool) {
    if counts.len() != 2 {
        return (0, 0, 0, 0, false);
    }
    let (tallied, skipped) = tally_into(ids, counts);
    let box_hits = counts[0];
    let plane_hits = counts[1];
    let total = u64::try_from(ids.len()).map_or(u64::MAX, |v| v);
    let covers = tallied.saturating_add(skipped) == total;
    let sums = tallied == box_hits.saturating_add(plane_hits);
    let stray = ids.iter().any(|id| *id != 0 && *id != 1 && !is_clear(*id));
    (
        tallied,
        skipped,
        box_hits,
        plane_hits,
        covers && sums && !stray,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clear_is_max_and_valid_stays_below_it() {
        assert_eq!(CLEAR, u32::MAX);
        assert_eq!(MAX_VALID_ID, u32::MAX - 1);
    }

    #[test]
    fn mapping_is_identity_then_saturates_never_clear() {
        assert_eq!(id_of_draw(0), 0);
        assert_eq!(id_of_draw(1), 1);
        assert_eq!(id_of_draw(255), 255);
        assert_eq!(id_of_draw(MAX_VALID_ID as usize), MAX_VALID_ID);
        assert_eq!(id_of_draw(usize::MAX), MAX_VALID_ID);
        assert_ne!(id_of_draw(usize::MAX), CLEAR);
        assert!(is_clear(CLEAR));
        assert!(!is_clear(0));
        assert!(!is_clear(MAX_VALID_ID));
    }

    #[test]
    fn fighter_slot_ids_cover_all_eight_twin_slots() {
        for slot in 0..8u8 {
            let id = id_of_fighter_slot(slot);
            assert_eq!(id, u32::from(slot));
            assert!(!is_clear(id));
        }
        assert_ne!(id_of_fighter_slot(u8::MAX), CLEAR);
    }

    #[test]
    fn tally_counts_draws_skips_clear_and_strays_loudly() {
        let ids = vec![0, 1, CLEAR, 1, 7, CLEAR, 0, 0];
        let mut counts = vec![0u64; 2];
        let (tallied, skipped) = tally_into(&ids, &mut counts);
        assert_eq!(counts, vec![3, 2]);
        assert_eq!((tallied, skipped), (5, 3));
    }

    #[test]
    fn box_plane_partition_scores_through_tally() {
        let ids = vec![0, 1, CLEAR, 1, 0, CLEAR, 0, 1];
        let mut first = vec![0u64; 2];
        let mut second = vec![0u64; 2];
        let a = verify_box_plane_partition(&ids, &mut first);
        let b = verify_box_plane_partition(&ids, &mut second);
        assert_eq!(a, b);
        assert_eq!(first, second);
        assert_eq!(a, (6, 2, 3, 3, true));
        assert_eq!(first, vec![3, 3]);
    }

    #[test]
    fn box_plane_partition_rejects_stray_ids_loudly() {
        let ids = vec![0, 1, 7, CLEAR, 0, 1];
        let mut counts = vec![0u64; 2];
        let (tallied, skipped, box_hits, plane_hits, partitioned) =
            verify_box_plane_partition(&ids, &mut counts);
        assert_eq!((tallied, skipped), (4, 2));
        assert_eq!((box_hits, plane_hits), (2, 2));
        assert!(
            !partitioned,
            "stray id 7 must fail the partition, got tallied={tallied} skipped={skipped}"
        );
    }

    #[test]
    fn box_plane_partition_empty_stays_empty() {
        let mut counts = vec![0u64; 2];
        assert_eq!(
            verify_box_plane_partition(&[], &mut counts),
            (0, 0, 0, 0, true)
        );
        assert_eq!(counts, vec![0, 0]);
        let mut wrong = vec![0u64; 3];
        assert_eq!(
            verify_box_plane_partition(&[0, 1], &mut wrong),
            (0, 0, 0, 0, false)
        );
    }

    #[test]
    fn tally_twice_matches_and_empty_stays_empty() {
        let ids = vec![0, CLEAR, 2, 2, 1, CLEAR];
        let mut first = vec![0u64; 3];
        let mut second = vec![0u64; 3];
        let a = tally_into(&ids, &mut first);
        let b = tally_into(&ids, &mut second);
        assert_eq!((first, a), (second, b));
        let mut empty = vec![0u64; 3];
        assert_eq!(tally_into(&[], &mut empty), (0, 0));
        assert_eq!(empty, vec![0, 0, 0]);
    }
}
