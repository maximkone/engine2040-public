//! coverage: the per-object event tape behind the coverage channels.
//!
//! Owner: engine-material (presentation only, never read back into the tick).
//! [`Coverage`] is a pure function of an event tape up to a tick
//! ([`Coverage::replay`]); this module keeps one tape per object id (a fighter
//! slot or a prop), so a host feeds events as the frame publishes them and
//! asks for the coverage at any tick. A rollback rewinds a tape with
//! [`CoverageTracker::rollback`] and the resim pushes the new branch: `at`
//! recomputes from the tape alone, so no stale wetness survives.
//!
//! The hit half of the feed is `engine_app::PresentationFrame`'s `ActorFrame::hit`
//! (blood on the struck object); water, dust and heat come from the host's own
//! event source. The feed itself lives with the host that owns the frame, not
//! here: `engine-app` sits above this crate, so a feed in this direction would
//! close a cycle.
//!
//! Units: ticks (`u32`), ids (`u32`), channel bytes (`u8`). Errors: none; a
//! tick before the tape replays as dry. Budget: one `Vec` per seen id; `at`
//! allocates nothing.

use std::collections::BTreeMap;

use crate::channels::{Coverage, CoverageEvent, TimedEvent};

/// One event tape per object id, asked for coverage at any tick.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CoverageTracker {
    tapes: BTreeMap<u32, Vec<TimedEvent>>,
}

impl CoverageTracker {
    /// An empty tracker.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends one event for `id`, keeping the tape in tick order: an event
    /// older than the last lands before it, and equal ticks keep insertion
    /// order, which is the order [`Coverage::replay`] folds ties in.
    pub fn push(&mut self, id: u32, tick: u32, event: CoverageEvent) {
        let tape = self.tapes.entry(id).or_default();
        let at = tape.partition_point(|timed| timed.tick <= tick);
        tape.insert(at, TimedEvent { tick, event });
    }

    /// The rollback rewind: drops every event of `id` after `tick`, so the
    /// resim pushes the new branch onto the rewound tape. Replaying after this
    /// leaves no coverage from the timeline that was rewound away.
    pub fn rollback(&mut self, id: u32, tick: u32) {
        if let Some(tape) = self.tapes.get_mut(&id) {
            tape.retain(|timed| timed.tick <= tick);
        }
    }

    /// The coverage of `id` at `tick`, from its tape alone. Never reads an
    /// event after `tick`; an id nobody fed reads as dry.
    #[must_use]
    pub fn at(&self, id: u32, tick: u32) -> Coverage {
        match self.tapes.get(&id) {
            Some(tape) => Coverage::replay(tape, tick),
            None => Coverage::default(),
        }
    }

    /// The tape of `id`, oldest first (empty for an id nobody fed).
    #[must_use]
    pub fn events(&self, id: u32) -> &[TimedEvent] {
        self.tapes.get(&id).map_or(&[], |tape| tape.as_slice())
    }

    /// The ids with a tape, ascending.
    pub fn ids(&self) -> impl Iterator<Item = u32> + '_ {
        self.tapes.keys().copied()
    }

    /// How many objects hold a tape.
    #[must_use]
    pub fn len(&self) -> usize {
        self.tapes.len()
    }

    /// True when nothing was pushed.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.tapes.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A seeded list over 40 ticks: three blood events and one water event.
    fn seeded(seed: u64) -> Vec<(u32, CoverageEvent)> {
        let mut rng = seed;
        let mut out = Vec::new();
        for index in 0..4u32 {
            rng = engine_core::splitmix64(rng);
            let tick = 5 + u32::try_from(rng % 30).unwrap_or(0);
            let event = if index == 3 {
                CoverageEvent::Water(120)
            } else {
                CoverageEvent::Blood(40 + u8::try_from(rng % 20).unwrap_or(0))
            };
            out.push((tick, event));
        }
        out.sort_by_key(|(tick, _)| *tick);
        out
    }

    fn feed(id: u32, events: &[(u32, CoverageEvent)]) -> CoverageTracker {
        let mut tracker = CoverageTracker::new();
        for (tick, event) in events {
            tracker.push(id, *tick, *event);
        }
        tracker
    }

    #[test]
    fn three_blood_events_and_one_water_replay_equal_twice() {
        let list = seeded(0x5EED_0001);
        let (a, b) = (feed(7, &list), feed(7, &list));
        assert_eq!(a.events(7).len(), 4, "three blood events and one water");
        assert_eq!(a.at(7, 40), b.at(7, 40), "the same list replays equal");
        assert_eq!(a.at(7, 40), a.at(7, 40), "and stable on a second ask");
        assert_eq!(a.at(7, 40).blood, b.at(7, 40).blood);
        assert!(a.at(7, 40).blood > 0, "the blood events landed");
        assert_ne!(a.at(7, 40).wetness, 0, "the water event landed");
        // An id nobody fed reads dry, and the tracker knows its ids.
        assert_eq!(a.at(9, 40), Coverage::default());
        assert_eq!(a.ids().collect::<Vec<_>>(), vec![7]);
        assert_eq!(a.len(), 1);
        assert!(!a.is_empty());
    }

    #[test]
    fn a_rolled_back_tape_replays_to_the_new_timeline() {
        let mut tracker = CoverageTracker::new();
        tracker.push(1, 10, CoverageEvent::Blood(60));
        tracker.push(1, 20, CoverageEvent::Water(200));
        tracker.push(1, 30, CoverageEvent::Blood(60));
        let before = tracker.at(1, 40);
        assert_eq!(before.blood, 120, "two blood events on the first timeline");
        assert_eq!(before.wetness, 195, "the water aged eight units by tick 40");
        // The sim rolls back to tick 15: the water at 20 and the blood at 30
        // never happened, and the resim pushes its own branch instead.
        tracker.rollback(1, 15);
        assert_eq!(tracker.events(1).len(), 1, "one event survives the rewind");
        assert_eq!(
            tracker.at(1, 40).blood,
            60,
            "the rewound tape holds one blood"
        );
        assert_eq!(tracker.at(1, 40).wetness, 0, "no stale wetness");
        tracker.push(1, 22, CoverageEvent::Dust(90));
        let after = tracker.at(1, 40);
        assert_ne!(after, before, "the new timeline answers differently");
        assert_eq!((after.blood, after.wetness, after.dirt), (60, 0, 90));
    }

    #[test]
    fn at_never_reads_a_tick_after_the_asked_one() {
        let mut tracker = CoverageTracker::new();
        tracker.push(2, 100, CoverageEvent::Blood(200));
        assert_eq!(tracker.at(2, 99), Coverage::default(), "before the event");
        assert_eq!(tracker.at(2, 100).blood, 200, "on the event tick");
        tracker.push(2, 300, CoverageEvent::Strain(77));
        assert_eq!(tracker.at(2, 250).strain, 0, "the later event is invisible");
        assert_eq!(tracker.at(2, 250).blood, 200, "blood is permanent");
        assert_eq!(tracker.at(2, 300).strain, 77, "and visible on its own tick");
        assert_eq!(tracker.at(2, 400).strain, 77);
    }

    #[test]
    fn push_keeps_the_tape_in_tick_order_for_out_of_order_feeds() {
        let mut tracker = CoverageTracker::new();
        tracker.push(3, 30, CoverageEvent::Blood(60));
        tracker.push(3, 10, CoverageEvent::Blood(60));
        tracker.push(3, 20, CoverageEvent::Blood(60));
        let ticks: Vec<u32> = tracker.events(3).iter().map(|timed| timed.tick).collect();
        assert_eq!(
            ticks,
            vec![10, 20, 30],
            "oldest first whatever the feed order"
        );
        // Ties keep insertion order, so a replay folds them in feed order.
        tracker.push(3, 20, CoverageEvent::Dust(10));
        let ticks: Vec<u32> = tracker.events(3).iter().map(|timed| timed.tick).collect();
        assert_eq!(ticks, vec![10, 20, 20, 30]);
        assert_eq!(tracker.at(3, 20).blood, 120, "both blood events fold in");
        assert_eq!(tracker.at(3, 20).dirt, 10);
    }
}
