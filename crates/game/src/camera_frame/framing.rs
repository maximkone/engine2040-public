//! framing: the `arena framing:` receipt tally of what the fight camera showed.
//!
//! Owner: game lane, moved here from `bin/arena.rs` (a pinned file) with the framing
//! law added. Consumer: the arena window, which folds one frame per presented frame
//! and prints [`Framing::line`] at exit; `--require-framing` fails the run on
//! [`Framing::failure`]. Determinism: view only, never hashed. Budget: a few integer
//! and float ops per frame, no heap.

use super::{Law, Verdict};

/// What the fight camera showed over a run: frames tallied, frames with no
/// fighter in the picture (and the first), the fewest fighters seen, the
/// steepest look down, how many drawn bodies wore the hit tint (a red strobe at
/// the start of an 8-fighter run, 2026-10-11) and the framing law over the
/// bodies the camera follows (see [`Law`]). Tallied only while the fighter look draws.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Framing {
    /// Frames tallied.
    pub frames: u32,
    /// Frames with no fighter in the picture.
    pub empty: u32,
    /// First frame with no fighter in the picture.
    pub first_empty: Option<u32>,
    /// Fewest fighters in the picture at any tallied frame.
    pub fewest: usize,
    steepest_deg: f32,
    bodies: u32,
    tinted: u32,
    /// Fighters in the match the tally was taken over (`0` until counted).
    fighters: usize,
    /// Fewest fighters the look had folded at any tallied frame.
    fewest_seen: usize,
    law: Law,
}

impl Default for Framing {
    fn default() -> Self {
        Self {
            frames: 0,
            empty: 0,
            first_empty: None,
            fewest: usize::MAX,
            steepest_deg: f32::MIN,
            bodies: 0,
            tinted: 0,
            fighters: 0,
            fewest_seen: usize::MAX,
            law: Law::new(),
        }
    }
}

impl Framing {
    /// One presented frame: `in_view` fighters in the picture, the camera looking
    /// `down_deg` below the horizon.
    pub fn note(&mut self, frame: u32, in_view: usize, down_deg: f32) {
        self.frames += 1;
        if in_view == 0 {
            self.empty += 1;
            self.first_empty.get_or_insert(frame);
        }
        self.fewest = self.fewest.min(in_view);
        self.steepest_deg = self.steepest_deg.max(down_deg);
    }

    /// One frame's `(tinted, drawn)` bodies from the look.
    pub fn note_tint(&mut self, (tinted, drawn): (usize, usize)) {
        let count = |n: usize| u32::try_from(n).unwrap_or(u32::MAX);
        self.tinted = self.tinted.saturating_add(count(tinted));
        self.bodies = self.bodies.saturating_add(count(drawn));
    }

    /// One frame's denominator: the fighters in the match and how many of
    /// them the look had folded, so `fewest_in_view` reads against a count.
    pub fn note_count(&mut self, fighters: usize, seen: usize) {
        self.fighters = self.fighters.max(fighters);
        self.fewest_seen = self.fewest_seen.min(seen);
    }

    /// One frame's framing verdict (`None` for a shot the law does not judge).
    pub fn note_shot(&mut self, verdict: Option<Verdict>) {
        if let Some(verdict) = verdict {
            self.law.note(&verdict);
        }
    }

    /// The `arena framing:` receipt line; `None` when nothing was tallied.
    /// `fighters=` and `seen=` ride only once counted: `fewest_in_view` of 2
    /// is fine out of 2 and a fault out of 8. The law fields (`law=ok judged=N
    /// body_h=lo..hi out=a/b small=a/b fills=a/b`) ride once a frame was judged;
    /// they follow `tinted_share` so a reader of the older fields is unmoved.
    #[must_use]
    pub fn line(&self) -> Option<String> {
        (self.frames > 0).then(|| {
            let first = self
                .first_empty
                .map_or_else(|| "-".to_string(), |f| f.to_string());
            let share = f64::from(self.tinted) / f64::from(self.bodies.max(1));
            let counted = if self.fighters > 0 {
                format!(" fighters={} seen={}", self.fighters, self.fewest_seen)
            } else {
                String::new()
            };
            format!(
                "arena framing: frames={}{counted} empty={} first_empty={first} fewest_in_view={} steepest_down_deg={:.1} tinted_share={share:.2}{}",
                self.frames,
                self.empty,
                self.fewest,
                self.steepest_deg,
                self.law.fields()
            )
        })
    }

    /// Why `--require-framing` fails this run; `None` when every tallied frame
    /// showed a fighter and the framing law held.
    #[must_use]
    pub fn failure(&self) -> Option<String> {
        let empty = self.first_empty.map(|first| {
            format!(
                "{} of {} frames showed no fighter (first at frame {first})",
                self.empty, self.frames
            )
        });
        let law = self.law.failure().map(|why| format!("framing law: {why}"));
        match (empty, law) {
            (None, None) => None,
            (Some(a), None) | (None, Some(a)) => Some(a),
            (Some(a), Some(b)) => Some(format!("{a}; {b}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lost() -> Verdict {
        Verdict {
            in_safe: false,
            size_max: 0.0,
        }
    }

    fn good() -> Verdict {
        Verdict {
            in_safe: true,
            size_max: 0.3,
        }
    }

    #[test]
    fn the_line_keeps_its_old_fields_until_a_frame_is_judged() {
        let mut framing = Framing::default();
        assert_eq!(framing.line(), None);
        framing.note(0, 2, 21.0);
        framing.note_shot(None);
        assert_eq!(
            framing.line().as_deref(),
            Some("arena framing: frames=1 empty=0 first_empty=- fewest_in_view=2 steepest_down_deg=21.0 tinted_share=0.00")
        );
    }

    #[test]
    fn a_judged_run_prints_the_law_after_the_old_fields() {
        let mut framing = Framing::default();
        for frame in 0..3 {
            framing.note(frame, 2, 15.0);
            framing.note_shot(Some(good()));
        }
        let line = framing.line().expect("tallied");
        assert!(
            line.ends_with(
                "tinted_share=0.00 law=ok judged=3 body_h=0.30..0.30 out=0/0 small=0/0 fills=0/0"
            ),
            "{line}"
        );
        assert_eq!(framing.failure(), None);
    }

    #[test]
    fn bodies_lost_for_longer_than_the_allowance_fail_the_run() {
        let mut framing = Framing::default();
        for frame in 0..=super::super::OUT_RUN_MAX {
            framing.note(frame, 2, 15.0);
            framing.note_shot(Some(lost()));
        }
        let why = framing.failure().expect("the law fails");
        assert!(
            why.starts_with("framing law: bodies left the safe frame"),
            "{why}"
        );
        assert!(framing.line().expect("tallied").contains(" law=FAIL "));
    }

    #[test]
    fn an_empty_frame_and_a_failed_law_both_name_themselves() {
        let mut framing = Framing::default();
        for frame in 0..=super::super::OUT_RUN_MAX {
            framing.note(frame, 0, 15.0);
            framing.note_shot(Some(lost()));
        }
        let why = framing.failure().expect("both fail");
        assert!(why.contains("showed no fighter"), "{why}");
        assert!(why.contains("framing law"), "{why}");
    }
}
