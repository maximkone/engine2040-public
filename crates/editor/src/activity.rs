//! Activity feed (human or agent) plus a scene diff: a person and an AI working
//! on the same scene see each other's edits.
//!
//! Purpose: one plain row per recorded action in a bounded ring, and a
//! three-way diff (added, removed, changed) between two scene snapshots.
//! Consumer: the editor `activity` dock panel
//! ([`Panel::Activity`](crate::shell::Panel::Activity)) renders
//! [`ActivityRing::text_rows`] and [`SceneDiff::text_rows`]; the same pure
//! functions feed the future door verbs `session.activity` and `scene.diff`.
//! Units: `tick` is a fixed-tick index, `subject` a scene object id, `revision`
//! a scene revision.
//! Determinism: the ring keeps oldest-first order and [`ActivityRing::digest`]
//! is FNV-1a over the ring in that order, so the same pushes always give the
//! same digest; [`diff_rows`] sorts both snapshots by id first, so the result
//! never depends on input order.
//! Errors: none. The ring never fails (it evicts the oldest past
//! [`ACTIVITY_CAP`]) and the diff is total.
//! Budget: O(cap) per digest and O(n log n) per diff, both far under a panel
//! frame at siege scale.
//!
//! Sources (ideas only): web: godot-mcp-bridge `scene_diff` +
//! `get_editor_activity` (MIT), and `docs/plan/finish-line.md` item 17,
//! read 2026-10-19.

use std::collections::VecDeque;

use engine_core::Fnv64;

/// Most activities one [`ActivityRing`] keeps; the oldest is evicted first.
pub const ACTIVITY_CAP: usize = 200;

/// Who did it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Author {
    /// A person at the dock.
    Human,
    /// An AI through the door.
    Agent,
}

impl Author {
    /// Stable lowercase tag, used in every row text and the digest.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Human => "human",
            Self::Agent => "agent",
        }
    }

    /// One byte for the digest (`0` human, `1` agent); never hash the tag text.
    fn code(self) -> u8 {
        match self {
            Self::Human => 0,
            Self::Agent => 1,
        }
    }
}

/// One recorded action, no heap: a bounded ring stores these.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Activity {
    /// Who did it.
    pub author: Author,
    /// What was done, e.g. `scene.place` (a static tag, never a formatted string).
    pub verb: &'static str,
    /// The object it touched, when the verb has one.
    pub subject: Option<u64>,
    /// Fixed tick the action was recorded at.
    pub tick: u64,
}

impl Activity {
    /// One plain row for the panel, e.g. `t0042 agent scene.place id=7`.
    #[must_use]
    pub fn text(&self) -> String {
        let head = format!("t{:04} {} {}", self.tick, self.author.name(), self.verb);
        match self.subject {
            Some(id) => format!("{head} id={id}"),
            None => head,
        }
    }
}

/// Bounded ring of [`Activity`]: oldest evicted, render order oldest first.
#[derive(Clone, Debug, Default)]
pub struct ActivityRing {
    rows: VecDeque<Activity>,
}

impl ActivityRing {
    /// Empty ring.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Push one activity, evicting the oldest past [`ACTIVITY_CAP`].
    pub fn push(&mut self, activity: Activity) {
        self.rows.push_back(activity);
        if self.rows.len() > ACTIVITY_CAP {
            self.rows.pop_front();
        }
    }

    /// Number of kept activities.
    #[must_use]
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    /// True when no activity is kept.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// The oldest kept activity.
    #[must_use]
    pub fn oldest(&self) -> Option<Activity> {
        self.rows.front().copied()
    }

    /// The newest kept activity.
    #[must_use]
    pub fn newest(&self) -> Option<Activity> {
        self.rows.back().copied()
    }

    /// Kept activities oldest first (render order).
    #[must_use]
    pub fn rows(&self) -> Vec<Activity> {
        self.rows.iter().copied().collect()
    }

    /// One panel row per activity, oldest first.
    #[must_use]
    pub fn text_rows(&self) -> Vec<String> {
        self.rows.iter().map(Activity::text).collect()
    }

    /// How many kept activities `author` recorded.
    #[must_use]
    pub fn count_by(&self, author: Author) -> usize {
        self.rows
            .iter()
            .filter(|activity| activity.author == author)
            .count()
    }

    /// FNV-1a 64 over (author, verb, subject, tick) in ring order.
    ///
    /// The verb is length-prefixed so two field splits cannot collide; the
    /// subject is flag-prefixed so `None` and `Some(0)` differ.
    #[must_use]
    pub fn digest(&self) -> u64 {
        let mut hasher = Fnv64::new();
        for activity in &self.rows {
            hasher.write_u8(activity.author.code());
            hasher.write_len(activity.verb.len());
            hasher.write_str(activity.verb);
            match activity.subject {
                Some(id) => {
                    hasher.write_u8(1);
                    hasher.write_u64(id);
                }
                None => hasher.write_u8(0),
            }
            hasher.write_u64(activity.tick);
        }
        hasher.finish()
    }
}

/// A scene row as the diff sees it: identity plus the revision that changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Row {
    /// Stable scene object id.
    pub id: u64,
    /// Scene revision the row carried in the snapshot it came from.
    pub revision: u64,
}

/// What changed between two snapshots.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SceneDiff {
    /// Rows only in `after`, sorted by id.
    pub added: Vec<Row>,
    /// Rows only in `before`, sorted by id.
    pub removed: Vec<Row>,
    /// Rows in both whose revision rose, sorted by id. Each carries the
    /// `after` revision (a [`Row`] holds one number, so the before revision is
    /// not carried).
    pub changed: Vec<Row>,
}

impl SceneDiff {
    /// True when nothing was added, removed or changed.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.removed.is_empty() && self.changed.is_empty()
    }

    /// Total changed rows across the three lists.
    #[must_use]
    pub fn len(&self) -> usize {
        self.added.len() + self.removed.len() + self.changed.len()
    }

    /// One panel row per change: `+ id=7 rev=1`, `- id=3 rev=2`,
    /// `~ id=5 rev=3` (the revision the row moved to).
    #[must_use]
    pub fn text_rows(&self) -> Vec<String> {
        let mut out = Vec::with_capacity(self.len());
        out.extend(
            self.added
                .iter()
                .map(|row| format!("+ id={} rev={}", row.id, row.revision)),
        );
        out.extend(
            self.removed
                .iter()
                .map(|row| format!("- id={} rev={}", row.id, row.revision)),
        );
        out.extend(
            self.changed
                .iter()
                .map(|row| format!("~ id={} rev={}", row.id, row.revision)),
        );
        out
    }
}

/// Diff two snapshots by id: `added` is in `after` only, `removed` is in
/// `before` only, `changed` is a same id whose revision rose.
///
/// Both inputs are sorted by id first (a stable merge walk), so every output
/// list comes out sorted by id and the result never depends on input order.
/// A row whose revision is unchanged appears in no list.
#[must_use]
pub fn diff_rows(before: &[Row], after: &[Row]) -> SceneDiff {
    let mut old = before.to_vec();
    let mut new = after.to_vec();
    old.sort_by_key(|row| row.id);
    new.sort_by_key(|row| row.id);

    let mut diff = SceneDiff::default();
    let (mut i, mut j) = (0usize, 0usize);
    while i < old.len() && j < new.len() {
        let (was, now) = (old[i], new[j]);
        if now.id == was.id {
            if now.revision > was.revision {
                diff.changed.push(now);
            }
            i += 1;
            j += 1;
        } else if now.id < was.id {
            diff.added.push(now);
            j += 1;
        } else {
            diff.removed.push(was);
            i += 1;
        }
    }
    diff.added.extend(new[j..].iter().copied());
    diff.removed.extend(old[i..].iter().copied());
    diff
}

#[cfg(test)]
mod tests {
    use super::*;

    fn act(author: Author, verb: &'static str, subject: Option<u64>, tick: u64) -> Activity {
        Activity {
            author,
            verb,
            subject,
            tick,
        }
    }

    fn row(id: u64, revision: u64) -> Row {
        Row { id, revision }
    }

    #[test]
    fn ring_push_past_cap_evicts_oldest_and_len_stays_at_cap() {
        let mut ring = ActivityRing::new();
        assert!(ring.is_empty());
        for tick in 0..u64::try_from(ACTIVITY_CAP + 25).unwrap() {
            ring.push(act(Author::Human, "scene.place", Some(tick), tick));
        }
        assert_eq!(ring.len(), ACTIVITY_CAP);
        // 25 oldest pushes are gone; the newest survives.
        assert_eq!(ring.oldest().unwrap().tick, 25);
        assert_eq!(
            ring.newest().unwrap().tick,
            u64::try_from(ACTIVITY_CAP + 24).unwrap()
        );
        assert_eq!(ring.rows().len(), ACTIVITY_CAP);
    }

    #[test]
    fn rows_are_oldest_first() {
        let mut ring = ActivityRing::new();
        ring.push(act(Author::Human, "scene.place", Some(1), 3));
        ring.push(act(Author::Agent, "scene.move", Some(2), 1));
        ring.push(act(Author::Human, "scene.remove", None, 2));
        let ticks: Vec<u64> = ring.rows().iter().map(|activity| activity.tick).collect();
        assert_eq!(ticks, vec![3, 1, 2]);
        assert_eq!(ring.oldest().unwrap().tick, 3);
        assert_eq!(ring.newest().unwrap().tick, 2);
    }

    #[test]
    fn count_by_splits_human_and_agent() {
        let mut ring = ActivityRing::new();
        ring.push(act(Author::Human, "scene.place", Some(1), 1));
        ring.push(act(Author::Agent, "scene.place", Some(2), 2));
        ring.push(act(Author::Agent, "pose.drag", None, 3));
        assert_eq!(ring.count_by(Author::Human), 1);
        assert_eq!(ring.count_by(Author::Agent), 2);
        assert_eq!(
            ring.count_by(Author::Human) + ring.count_by(Author::Agent),
            ring.len()
        );
    }

    #[test]
    fn digest_is_stable_and_moves_when_one_field_changes() {
        let base = [
            act(Author::Human, "scene.place", Some(7), 42),
            act(Author::Agent, "scene.move", Some(7), 43),
        ];
        let mut a = ActivityRing::new();
        let mut b = ActivityRing::new();
        for activity in base {
            a.push(activity);
            b.push(activity);
        }
        // The same pushes give the same digest twice.
        assert_eq!(a.digest(), b.digest());

        // A tick change alone moves the digest.
        let mut tick_moved = ActivityRing::new();
        tick_moved.push(base[0]);
        tick_moved.push(act(Author::Agent, "scene.move", Some(7), 44));
        assert_ne!(a.digest(), tick_moved.digest());

        // An author change alone moves it too.
        let mut author_moved = ActivityRing::new();
        author_moved.push(act(Author::Agent, "scene.place", Some(7), 42));
        author_moved.push(base[1]);
        assert_ne!(a.digest(), author_moved.digest());

        // A subject change alone moves it too.
        let mut subject_moved = ActivityRing::new();
        subject_moved.push(act(Author::Human, "scene.place", Some(8), 42));
        subject_moved.push(base[1]);
        assert_ne!(a.digest(), subject_moved.digest());

        // A verb change alone moves it too.
        let mut verb_moved = ActivityRing::new();
        verb_moved.push(act(Author::Human, "scene.spawn", Some(7), 42));
        verb_moved.push(base[1]);
        assert_ne!(a.digest(), verb_moved.digest());
    }

    #[test]
    fn text_rows_has_one_row_per_activity() {
        let mut ring = ActivityRing::new();
        ring.push(act(Author::Human, "scene.place", Some(7), 42));
        ring.push(act(Author::Agent, "pose.drag", None, 0));
        let rows = ring.text_rows();
        assert_eq!(rows.len(), ring.len());
        assert_eq!(rows[0], "t0042 human scene.place id=7");
        assert_eq!(rows[1], "t0000 agent pose.drag");
    }

    #[test]
    fn diff_reports_added_removed_and_changed_once() {
        let before = [row(3, 2), row(5, 1), row(9, 4)];
        let after = [row(3, 3), row(7, 1), row(9, 4)];
        let diff = diff_rows(&before, &after);
        assert_eq!(diff.added, vec![row(7, 1)]);
        assert_eq!(diff.removed, vec![row(5, 1)]);
        assert_eq!(diff.changed, vec![row(3, 3)]);
        // The unchanged id 9 appears in no list; each change is reported once.
        assert_eq!(diff.len(), 3);
        assert!(!diff.is_empty());
        assert_eq!(
            diff.text_rows(),
            vec![
                "+ id=7 rev=1".to_string(),
                "- id=5 rev=1".to_string(),
                "~ id=3 rev=3".to_string(),
            ]
        );
    }

    #[test]
    fn diff_sorts_by_id_and_never_depends_on_input_order() {
        let before = [row(9, 4), row(2, 1), row(5, 1)];
        let after = [row(7, 1), row(9, 4), row(2, 1), row(6, 1)];
        let forward = diff_rows(&before, &after);
        let mut before_rev = before;
        before_rev.reverse();
        let mut after_rev = after;
        after_rev.reverse();
        let reversed = diff_rows(&before_rev, &after_rev);

        assert_eq!(forward, reversed);
        assert_eq!(forward.added, vec![row(6, 1), row(7, 1)]);
        assert_eq!(forward.removed, vec![row(5, 1)]);
        assert!(forward.changed.is_empty());
        // Sorted by id, and the same call twice is equal.
        let ids: Vec<u64> = forward.added.iter().map(|row| row.id).collect();
        assert_eq!(ids, vec![6, 7]);
        assert_eq!(diff_rows(&before, &after), forward);
    }

    #[test]
    fn empty_diff_reports_nothing() {
        let before = [row(1, 1)];
        assert!(diff_rows(&before, &before).is_empty());
        assert_eq!(diff_rows(&before, &before).len(), 0);
        assert!(diff_rows(&[], &[]).is_empty());
        assert_eq!(diff_rows(&[], &before).added, before);
        assert_eq!(diff_rows(&before, &[]).removed, before);
    }
}
