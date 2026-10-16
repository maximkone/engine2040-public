//! motiondb: bounded motion-DB query slices for traversal approaches.
//!
//! Caller: `crate::traversal::rank_approach` ranks one approach clip per
//! committed traversal plan over a slice built here; future consumer:
//! `crates/arena/src/spine_step.rs` (`sys_motion`) drives the rank from
//! locomotion state. Data owner: `engine-anim`; this module never imports
//! assets or arena. Units: slice bounds in entries; cost lanes in the Q10
//! integer units of [`crate::motion_match`].
//!
//! Determinism: the slice scan is integer-only over ascending DB indices
//! with strict `<` replacement, so a whole-DB slice picks exactly what
//! [`crate::motion_match::search`] picks. Errors are [`SliceError`].

use crate::motion_match::{
    cost_capped, MatchResult, MotionDb, Query, SearchReceipt, MAX_ENTRIES, MAX_WEIGHT, PENALTY_MAX,
};

/// Motion-slice failures: bounded diagnostics, never panics.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SliceError {
    /// The database holds no entries.
    EmptyDatabase,
    /// A slice bound is out of range; the lane names the problem.
    BadSlice(&'static str),
    /// A query input is out of bounds; the lane names the problem.
    BadQuery(&'static str),
}

impl core::fmt::Display for SliceError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            SliceError::EmptyDatabase => write!(f, "motion slice: empty database"),
            SliceError::BadSlice(lane) => write!(f, "motion slice: bad slice ({lane})"),
            SliceError::BadQuery(lane) => write!(f, "motion slice: bad query ({lane})"),
        }
    }
}

impl std::error::Error for SliceError {}

/// Bounded index window over a [`MotionDb`]: `start` plus `len` entries.
/// A traversal plan commits to one move; its approach clip lives in one
/// window (one clip's frames), so the per-tick scan stays bounded while the
/// pick stays bit-identical to a full scan over the same window.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct DbSlice {
    start: u32,
    len: u32,
}

impl DbSlice {
    /// New window of `len` entries from `start`. Empty and oversize windows
    /// are refused; `start` is checked against the database in
    /// [`DbSlice::resolve`], which knows its length.
    pub fn new(start: u32, len: u32) -> Result<Self, SliceError> {
        if len == 0 {
            return Err(SliceError::BadSlice("empty"));
        }
        if usize::try_from(len).is_ok_and(|wide| wide > MAX_ENTRIES) {
            return Err(SliceError::BadSlice("too long"));
        }
        Ok(Self { start, len })
    }

    /// Window covering a whole database of `db_len` entries.
    pub fn whole(db_len: usize) -> Result<Self, SliceError> {
        if db_len == 0 {
            return Err(SliceError::BadSlice("empty"));
        }
        if db_len > MAX_ENTRIES {
            return Err(SliceError::BadSlice("too long"));
        }
        let len = u32::try_from(db_len).map_err(|_| SliceError::BadSlice("too long"))?;
        Ok(Self { start: 0, len })
    }

    /// First entry index of the window.
    pub fn start(&self) -> u32 {
        self.start
    }

    /// Entry count of the window.
    pub fn len(&self) -> u32 {
        self.len
    }

    /// Empty windows are refused by [`DbSlice::new`], so this stays false
    /// for a built slice; provided for [`DbSlice::len`] callers.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Resolve to a `[start, end)` index range over a database of `db_len`
    /// entries. Windows past the end are refused, never clamped, so a stale
    /// window cannot silently rank the wrong clip.
    pub fn resolve(&self, db_len: usize) -> Result<(usize, usize), SliceError> {
        let start = usize::try_from(self.start).map_err(|_| SliceError::BadSlice("start"))?;
        let len = usize::try_from(self.len).map_err(|_| SliceError::BadSlice("length"))?;
        if len == 0 {
            return Err(SliceError::BadSlice("empty"));
        }
        let end = start.checked_add(len).ok_or(SliceError::BadSlice("span"))?;
        if end > db_len {
            return Err(SliceError::BadSlice("past end"));
        }
        Ok((start, end))
    }
}

/// Validate a hand-built query with the bounds
/// [`crate::motion_match::build_query`] enforces, so a slice search never
/// runs on looser inputs than a full search.
fn validate_query(query: &Query) -> Result<(), SliceError> {
    if query.contact > 3 {
        return Err(SliceError::BadQuery("contact"));
    }
    if query.weights.iter().any(|w| *w < 0 || *w > MAX_WEIGHT) {
        return Err(SliceError::BadQuery("weights"));
    }
    if query.jump_cost < 0 || query.contact_cost < 0 {
        return Err(SliceError::BadQuery("penalty"));
    }
    if query.jump_cost > PENALTY_MAX || query.contact_cost > PENALTY_MAX {
        return Err(SliceError::BadQuery("penalty range"));
    }
    Ok(())
}

/// Deterministic slice search: the same two passes
/// ([`crate::motion_match::search`]) over `[start, end)` only — seed the
/// running best from the contact-matching entries in ascending order, then
/// scan the rest in ascending order with the running-best bound. Strict `<`
/// replacement keeps the lowest index on ties, so a whole-DB slice equals a
/// full search while the receipt records the skipped work.
pub fn search_slice(
    db: &MotionDb,
    slice: &DbSlice,
    query: &Query,
) -> Result<(MatchResult, SearchReceipt), SliceError> {
    if db.entries.is_empty() {
        return Err(SliceError::EmptyDatabase);
    }
    validate_query(query)?;
    let (start, end) = slice.resolve(db.entries.len())?;
    let mut receipt = SearchReceipt {
        total: end - start,
        ..SearchReceipt::default()
    };
    let mut best = i64::MAX;
    let mut best_index = start;
    for to in start..end {
        let entry = db
            .entries
            .get(to)
            .ok_or(SliceError::BadSlice("stale range"))?;
        if entry.contact != query.contact {
            continue;
        }
        match cost_capped(db, entry, to, query, best, &mut receipt) {
            Some(cost) => {
                if cost < best {
                    best = cost;
                    best_index = to;
                }
                receipt.full_evals += 1;
            }
            None => receipt.early_cuts += 1,
        }
    }
    for to in start..end {
        let entry = db
            .entries
            .get(to)
            .ok_or(SliceError::BadSlice("stale range"))?;
        if entry.contact == query.contact {
            receipt.reuse_skips += 1;
            continue;
        }
        match cost_capped(db, entry, to, query, best, &mut receipt) {
            Some(cost) => {
                if cost < best {
                    best = cost;
                    best_index = to;
                }
                receipt.full_evals += 1;
            }
            None => receipt.early_cuts += 1,
        }
    }
    let winner = db
        .entries
        .get(best_index)
        .ok_or(SliceError::BadSlice("stale range"))?;
    Ok((
        MatchResult {
            index: best_index,
            clip: winner.clip,
            tick: winner.tick,
            cost: best,
        },
        receipt,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::motion_match::{bake, default_weights, search, Entry, FEAT_N, NO_HIST};

    fn raw_entry(clip: u16, tick: u16, lane0: i32, contact: u8) -> Entry {
        let mut feat = [0i32; FEAT_N];
        feat[0] = lane0;
        Entry {
            feat,
            clip,
            tick,
            contact,
        }
    }

    fn raw_query(lane0: i32, contact: u8) -> Query {
        let mut feat = [0i32; FEAT_N];
        feat[0] = lane0;
        Query {
            feat,
            contact,
            curr: NO_HIST,
            weights: default_weights(),
            jump_cost: 0,
            contact_cost: 0,
        }
    }

    fn lane_db() -> MotionDb {
        bake(vec![
            raw_entry(0, 0, -30, 3),
            raw_entry(0, 1, -10, 3),
            raw_entry(0, 2, 10, 1),
            raw_entry(1, 0, 30, 3),
            raw_entry(1, 1, 50, 0),
            raw_entry(1, 2, 70, 3),
        ])
        .expect("fixture bakes")
    }

    #[test]
    fn whole_slice_equals_full_search() {
        let db = lane_db();
        for (lane0, contact) in [(-30, 3), (-10, 3), (10, 1), (30, 3), (50, 0), (0, 2)] {
            let query = raw_query(lane0, contact);
            let sliced = search_slice(
                &db,
                &DbSlice::whole(db.entries.len()).expect("whole"),
                &query,
            )
            .expect("slice searches");
            let full = search(&db, &query).expect("full searches");
            assert_eq!(sliced.0, full.0, "lane {lane0} contact {contact}");
            assert_eq!(sliced.1.lane_ops, full.1.lane_ops, "same scan work");
        }
    }

    #[test]
    fn sub_slice_confines_the_winner() {
        let db = lane_db();
        // Global best for lane 100 sits at the last entry; the head window
        // must hold its own best instead of reaching past the slice.
        let query = raw_query(100, 3);
        let (global, _) = search(&db, &query).expect("full searches");
        assert_eq!(global.index, 5);
        let head = DbSlice::new(0, 4).expect("head window");
        let (held, receipt) = search_slice(&db, &head, &query).expect("slice searches");
        assert_eq!(held.index, 3, "best of the window, not of the DB");
        assert_eq!(receipt.total, 4);
        assert_eq!(receipt.full_evals + receipt.early_cuts, receipt.total);
    }

    #[test]
    fn bad_slices_name_the_lane() {
        let db = lane_db();
        let query = raw_query(0, 3);
        assert_eq!(DbSlice::new(0, 0), Err(SliceError::BadSlice("empty")));
        assert_eq!(DbSlice::whole(0), Err(SliceError::BadSlice("empty")));
        let past = DbSlice::new(4, 4).expect("window builds");
        assert_eq!(
            past.resolve(db.entries.len()),
            Err(SliceError::BadSlice("past end"))
        );
        assert_eq!(
            search_slice(&db, &past, &query),
            Err(SliceError::BadSlice("past end"))
        );
        assert_eq!(
            search_slice(
                &MotionDb::default(),
                &DbSlice::new(0, 1).expect("window"),
                &query
            ),
            Err(SliceError::EmptyDatabase)
        );
    }

    #[test]
    fn bad_query_mirrors_full_search() {
        let db = lane_db();
        let slice = DbSlice::whole(db.entries.len()).expect("whole");
        let mut weights = default_weights();
        weights[0] = -1;
        let bad = Query {
            feat: [0; FEAT_N],
            contact: 3,
            curr: NO_HIST,
            weights,
            jump_cost: 0,
            contact_cost: 0,
        };
        assert_eq!(
            search_slice(&db, &slice, &bad),
            Err(SliceError::BadQuery("weights"))
        );
        assert_eq!(
            search(&db, &bad).map_err(|_| SliceError::BadQuery("weights")),
            Err(SliceError::BadQuery("weights"))
        );
        let mut contact_bad = raw_query(0, 3);
        contact_bad.contact = 9;
        assert_eq!(
            search_slice(&db, &slice, &contact_bad),
            Err(SliceError::BadQuery("contact"))
        );
    }

    #[test]
    fn slice_replay_is_exact() {
        let db = lane_db();
        let slice = DbSlice::new(1, 4).expect("window");
        assert_eq!((slice.start(), slice.len()), (1, 4));
        assert_eq!(slice.resolve(db.entries.len()), Ok((1, 5)));
        let query = raw_query(-10, 3);
        let first = search_slice(&db, &slice, &query).expect("slice searches");
        let second = search_slice(&db, &slice, &query).expect("slice searches");
        assert_eq!(first, second);
        assert_eq!(
            first.1.reuse_skips,
            db.entries[1..5].iter().filter(|e| e.contact == 3).count()
        );
        // Continuity cursors replay too: history on entry 1 holds entry 2.
        let mut biased = raw_query(70, 3);
        biased.curr = 1;
        biased.jump_cost = crate::motion_match::cost_units(10.0);
        let held = search_slice(&db, &slice, &biased).expect("biased searches");
        assert_eq!(search_slice(&db, &slice, &biased).expect("repeat"), held);
    }

    #[test]
    fn whole_slice_matches_full_search_with_history_and_penalties() {
        // Pins the jump/penalty lanes (`jump_add` plus the capped-cost early
        // cut): with a continuity cursor and nonzero penalties steering the
        // pick, the whole-DB slice and the full scan must agree bit for bit.
        let db = lane_db();
        let slice = DbSlice::whole(db.entries.len()).expect("whole");
        for (lane0, contact, curr) in [(10, 1, 1u32), (30, 3, 2), (50, 0, 4), (0, 2, 0)] {
            let mut query = raw_query(lane0, contact);
            let mut weights = default_weights();
            weights[0] = 7;
            weights[1] = 3;
            query.weights = weights;
            query.curr = curr;
            query.jump_cost = crate::motion_match::cost_units(10.0);
            query.contact_cost = crate::motion_match::cost_units(2.0);
            let sliced = search_slice(&db, &slice, &query).expect("slice searches");
            let full = search(&db, &query).expect("full searches");
            assert_eq!(
                sliced.0, full.0,
                "lane {lane0} contact {contact} curr {curr}"
            );
            assert_eq!(sliced.1.lane_ops, full.1.lane_ops, "same scan work");
        }
    }
}
