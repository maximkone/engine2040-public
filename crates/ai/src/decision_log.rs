//! Off-by-default ring of AI decisions, plus the diff two tapes need.
//!
//! A decision row is one *evaluation*: the tick, the fighter, the five utility
//! scores in [`ACTION_NAMES`](crate::decision_log) order (attack, block, dodge,
//! advance, retreat) and the chosen lane (5 means the anti-stuck reposition).
//! Held ticks between evaluations are not rows: they repeat the last input and
//! change nothing.
//!
//! The ring is off by default. While off, deciding does no extra scoring and
//! allocates nothing; [`Brain::record_decisions`](crate::gladiator::Brain::record_decisions)
//! turns it on with a fixed capacity and drops the oldest row when full, so a
//! long fight costs one fixed buffer.
//!
//! Bytes are canonical: row count as `u32` little-endian, then per row the tick
//! (`u32`), fighter, chosen lane, and five `f32` score bits, all little-endian.
//! Same rows always encode the same bytes, and [`DecisionLog::from_bytes`]
//! refuses a short, long or oversized tape instead of guessing.

use core::fmt;
use std::collections::VecDeque;

/// Score lanes per decision, in utility order.
pub const ACTION_COUNT: usize = 5;

/// Lane names; index 5 is the anti-stuck reposition, which has no score lane.
pub const ACTION_NAMES: [&str; 6] = [
    "attack",
    "block",
    "dodge",
    "advance",
    "retreat",
    "reposition",
];

/// Largest ring a log will hold, and the largest tape it will decode.
pub const MAX_ROWS: usize = 4_096;

/// Bytes per encoded row: tick, fighter, chosen, five score words.
pub const ROW_BYTES: usize = 4 + 1 + 1 + ACTION_COUNT * 4;

/// One AI evaluation, ready to show or diff.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Decision {
    /// Tick the evaluation ran on.
    pub tick: u32,
    /// Fighter slot the brain drove.
    pub fighter: u8,
    /// Utility score per lane, in [`ACTION_NAMES`] order.
    pub scores: [f32; ACTION_COUNT],
    /// Chosen lane index: 0..=4 score lanes, 5 reposition.
    pub chosen: u8,
}

impl Decision {
    /// Score of the chosen lane, or 0 for the unscored reposition.
    pub fn chosen_score(&self) -> f32 {
        self.scores
            .get(usize::from(self.chosen))
            .copied()
            .unwrap_or(0.0)
    }

    /// Name of the chosen lane.
    pub fn chosen_name(&self) -> &'static str {
        ACTION_NAMES
            .get(usize::from(self.chosen))
            .copied()
            .unwrap_or("unknown")
    }

    /// Same row, comparing score bits so a NaN never compares unequal by accident.
    fn same(&self, other: &Decision) -> bool {
        self.tick == other.tick
            && self.fighter == other.fighter
            && self.chosen == other.chosen
            && self
                .scores
                .iter()
                .zip(other.scores.iter())
                .all(|(left, right)| left.to_bits() == right.to_bits())
    }
}

/// Fixed-capacity ring; the newest [`MAX_ROWS`] rows survive.
///
/// Equality compares the rows only: capacity is a buffer choice, not tape
/// content, and a decoded tape keeps the capacity its row count implies.
#[derive(Clone, Debug)]
pub struct DecisionLog {
    cap: usize,
    rows: VecDeque<Decision>,
}

impl PartialEq for DecisionLog {
    fn eq(&self, other: &Self) -> bool {
        self.rows.len() == other.rows.len()
            && self
                .rows
                .iter()
                .zip(other.rows.iter())
                .all(|(left, right)| left.same(right))
    }
}

impl DecisionLog {
    /// New ring holding at most `cap` rows, clamped to `1..=MAX_ROWS`.
    pub fn new(cap: usize) -> Self {
        Self {
            cap: cap.clamp(1, MAX_ROWS),
            rows: VecDeque::new(),
        }
    }

    /// Capacity in rows.
    pub const fn cap(&self) -> usize {
        self.cap
    }

    /// Rows held, oldest first.
    pub fn rows(&self) -> impl Iterator<Item = &Decision> {
        self.rows.iter()
    }

    /// Rows held.
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    /// True when no row was recorded.
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// Record one evaluation, dropping the oldest row when full.
    pub fn push(&mut self, decision: Decision) {
        if self.rows.len() == self.cap {
            self.rows.pop_front();
        }
        self.rows.push_back(decision);
    }

    /// Canonical bytes, see the module docs.
    pub fn to_bytes(&self) -> Vec<u8> {
        let count = u32::try_from(self.rows.len()).unwrap_or(u32::MAX);
        let mut out = Vec::with_capacity(4 + self.rows.len() * ROW_BYTES);
        out.extend_from_slice(&count.to_le_bytes());
        for row in &self.rows {
            out.extend_from_slice(&row.tick.to_le_bytes());
            out.push(row.fighter);
            out.push(row.chosen);
            for score in row.scores {
                out.extend_from_slice(&score.to_bits().to_le_bytes());
            }
        }
        out
    }

    /// Decode canonical bytes. Refuses a short tape, trailing bytes and a count
    /// above [`MAX_ROWS`] instead of allocating on the sender's word.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, LogError> {
        let head = bytes.get(..4).ok_or(LogError::Short {
            needed: 4,
            got: bytes.len(),
        })?;
        let count = u32::from_le_bytes([head[0], head[1], head[2], head[3]]);
        let count = usize::try_from(count).map_err(|_| LogError::TooManyRows(u32::MAX))?;
        if count > MAX_ROWS {
            return Err(LogError::TooManyRows(
                u32::try_from(count).unwrap_or(u32::MAX),
            ));
        }
        let needed = 4 + count * ROW_BYTES;
        if bytes.len() < needed {
            return Err(LogError::Short {
                needed,
                got: bytes.len(),
            });
        }
        if bytes.len() > needed {
            return Err(LogError::Long {
                expected: needed,
                got: bytes.len(),
            });
        }
        let mut log = Self::new(count.max(1));
        let mut at = 4;
        for _ in 0..count {
            let tick = u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]]);
            let fighter = bytes[at + 4];
            let chosen = bytes[at + 5];
            let mut scores = [0.0_f32; ACTION_COUNT];
            for (lane, score) in scores.iter_mut().enumerate() {
                let base = at + 6 + lane * 4;
                let word = u32::from_le_bytes([
                    bytes[base],
                    bytes[base + 1],
                    bytes[base + 2],
                    bytes[base + 3],
                ]);
                *score = f32::from_bits(word);
            }
            log.rows.push_back(Decision {
                tick,
                fighter,
                scores,
                chosen,
            });
            at += ROW_BYTES;
        }
        Ok(log)
    }

    /// One line per row, oldest first; the door's `show` answer.
    pub fn show_text(&self) -> String {
        let mut out = format!("{} row(s), capacity {}\n", self.rows.len(), self.cap);
        for row in &self.rows {
            out.push_str(&format!(
                "tick={} fighter={} chose={} scores=[{:.1}, {:.1}, {:.1}, {:.1}, {:.1}]\n",
                row.tick,
                row.fighter,
                row.chosen_name(),
                row.scores[0],
                row.scores[1],
                row.scores[2],
                row.scores[3],
                row.scores[4],
            ));
        }
        out
    }
}

/// First tick two tapes differ, with both rows and the chosen-score gap.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DecisionDiff {
    /// First tick that differs; the earlier tape's tick when one ran longer.
    pub tick: u32,
    /// Fighter of the differing row.
    pub fighter: u8,
    /// The left row, when the left tape has one at this tick.
    pub left: Option<Decision>,
    /// The right row, when the right tape has one at this tick.
    pub right: Option<Decision>,
    /// Absolute gap between the two chosen scores, 0 when a row is missing.
    pub gap: f32,
}

impl DecisionDiff {
    /// One printable line naming the tick, both rows and the gap.
    pub fn line(&self) -> String {
        let row = |side: Option<Decision>| match side {
            Some(decision) => format!(
                "chose={} scores=[{:.1}, {:.1}, {:.1}, {:.1}, {:.1}]",
                decision.chosen_name(),
                decision.scores[0],
                decision.scores[1],
                decision.scores[2],
                decision.scores[3],
                decision.scores[4],
            ),
            None => "no row".to_owned(),
        };
        format!(
            "first divergence tick={} fighter={} left {} right {} gap={:.1}",
            self.tick,
            self.fighter,
            row(self.left),
            row(self.right),
            self.gap,
        )
    }
}

/// First differing row of two tick-ordered tapes, or `None` when they match.
pub fn diff(left: &DecisionLog, right: &DecisionLog) -> Option<DecisionDiff> {
    let mut index = 0usize;
    for (a, b) in left.rows().zip(right.rows()) {
        if !a.same(b) {
            return Some(DecisionDiff {
                tick: a.tick.min(b.tick),
                fighter: a.fighter,
                left: Some(*a),
                right: Some(*b),
                gap: (a.chosen_score() - b.chosen_score()).abs(),
            });
        }
        index += 1;
    }
    let extra_left = left.rows().nth(index).copied();
    let extra_right = right.rows().nth(index).copied();
    let extra = extra_left.or(extra_right)?;
    let gap = extra_left
        .map(|row| row.chosen_score())
        .zip(extra_right.map(|row| row.chosen_score()))
        .map_or(0.0, |(a, b)| (a - b).abs());
    Some(DecisionDiff {
        tick: extra.tick,
        fighter: extra.fighter,
        left: extra_left,
        right: extra_right,
        gap,
    })
}

/// Why a tape was refused. The door maps each to a static code.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogError {
    /// Fewer bytes than the header and row count promise.
    Short {
        /// Bytes the tape needs.
        needed: usize,
        /// Bytes it carries.
        got: usize,
    },
    /// Bytes after the last promised row.
    Long {
        /// Bytes the tape promises.
        expected: usize,
        /// Bytes it carries.
        got: usize,
    },
    /// Row count above [`MAX_ROWS`].
    TooManyRows(u32),
}

impl fmt::Display for LogError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Short { needed, got } => {
                write!(formatter, "tape is short: needs {needed} bytes, has {got}")
            }
            Self::Long { expected, got } => {
                write!(
                    formatter,
                    "tape has trailing bytes: {got} for {expected} promised"
                )
            }
            Self::TooManyRows(count) => {
                write!(formatter, "tape claims {count} rows, cap is {MAX_ROWS}")
            }
        }
    }
}

impl std::error::Error for LogError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::curves::AiConfig;
    use crate::gladiator::Brain;
    use crate::Percept;
    use engine_input::FighterId;

    fn row(tick: u32, chosen: u8, scores: [f32; ACTION_COUNT]) -> Decision {
        Decision {
            tick,
            fighter: 2,
            scores,
            chosen,
        }
    }

    fn tape(rows: &[(u32, u8, [f32; ACTION_COUNT])]) -> DecisionLog {
        let mut log = DecisionLog::new(8);
        for (tick, chosen, scores) in rows {
            log.push(row(*tick, *chosen, *scores));
        }
        log
    }

    #[test]
    fn ring_keeps_the_newest_rows_at_capacity() {
        let mut log = DecisionLog::new(2);
        log.push(row(10, 0, [1.0; 5]));
        log.push(row(20, 1, [2.0; 5]));
        log.push(row(30, 2, [3.0; 5]));
        let ticks: Vec<u32> = log.rows().map(|decision| decision.tick).collect();
        assert_eq!(ticks, vec![20, 30]);
        assert_eq!(log.len(), 2);
        assert_eq!(log.cap(), 2);
        assert_eq!(DecisionLog::new(0).cap(), 1);
        assert_eq!(DecisionLog::new(usize::MAX).cap(), MAX_ROWS);
    }

    #[test]
    fn one_changed_row_is_the_first_divergent_tick_with_both_vectors() {
        let left = tape(&[
            (10, 0, [9.0, 1.0, 0.0, 0.0, 0.0]),
            (20, 3, [0.0, 0.0, 0.0, 8.0, 1.0]),
            (30, 1, [0.0, 7.0, 0.0, 0.0, 0.0]),
        ]);
        let right = tape(&[
            (10, 0, [9.0, 1.0, 0.0, 0.0, 0.0]),
            (20, 3, [0.0, 0.0, 0.0, 8.0, 1.0]),
            (30, 1, [0.0, 4.0, 0.0, 0.0, 0.0]),
        ]);
        let found = diff(&left, &right).expect("the tapes differ");
        assert_eq!(found.tick, 30);
        assert_eq!(found.fighter, 2);
        assert_eq!(found.left.unwrap().scores[1], 7.0);
        assert_eq!(found.right.unwrap().scores[1], 4.0);
        assert_eq!(found.gap, 3.0);
        assert!(found.line().contains("tick=30"), "{}", found.line());
        assert!(diff(&left, &left).is_none(), "a tape equals itself");
    }

    #[test]
    fn identical_tapes_encode_to_equal_bytes() {
        let rows = [
            (7, 4, [0.0, 0.0, 0.0, 0.0, 6.5]),
            (13, 5, [0.0; 5]),
            (21, 2, [1.5, 0.0, 2.5, 0.0, 0.0]),
        ];
        let first = tape(&rows);
        let second = tape(&rows);
        assert_eq!(first.to_bytes(), second.to_bytes());
        assert_eq!(first.to_bytes().len(), 4 + 3 * ROW_BYTES);
        assert_eq!(first, second);
    }

    #[test]
    fn bytes_round_trip_and_bad_tapes_are_refused() {
        let log = tape(&[(7, 4, [0.0, 0.0, 0.0, 0.0, 6.5]), (13, 5, [0.0; 5])]);
        let bytes = log.to_bytes();
        let back = DecisionLog::from_bytes(&bytes).expect("round trip");
        assert_eq!(back, log);
        assert_eq!(
            DecisionLog::from_bytes(&bytes[..bytes.len() - 1]),
            Err(LogError::Short {
                needed: bytes.len(),
                got: bytes.len() - 1,
            })
        );
        let mut long = bytes.clone();
        long.push(0);
        assert_eq!(
            DecisionLog::from_bytes(&long),
            Err(LogError::Long {
                expected: bytes.len(),
                got: bytes.len() + 1,
            })
        );
        let oversized = u32::try_from(MAX_ROWS + 1)
            .unwrap_or(u32::MAX)
            .to_le_bytes();
        assert_eq!(
            DecisionLog::from_bytes(&oversized),
            Err(LogError::TooManyRows(
                u32::try_from(MAX_ROWS + 1).unwrap_or(u32::MAX)
            ))
        );
    }

    #[test]
    fn recording_is_off_by_default_and_costs_zero_allocations() {
        let mut brain = Brain::with_config(FighterId(2), 1, AiConfig::scripted_defaults());
        assert!(brain.decisions().is_none(), "off by default");
        let percept = Percept::new([0.0, 0.0, 0.0], 0.0, [1.2, 0.0, 0.0], true, 1.2, 10);
        brain.decide(&percept);
        brain.decide(&Percept::new(
            [0.0, 0.0, 0.0],
            0.0,
            [1.3, 0.0, 0.0],
            true,
            1.3,
            11,
        ));
        let (_, counted) = engine_core::alloc_probe::measure(|| {
            brain.decide(&Percept::new(
                [0.0, 0.0, 0.0],
                0.0,
                [1.4, 0.0, 0.0],
                true,
                1.4,
                12,
            ))
        });
        assert_eq!(counted.calls, 0, "a disabled ring allocates nothing");

        brain.record_decisions(2);
        for tick in 13..17 {
            brain.decide(&Percept::new(
                [0.0, 0.0, 0.0],
                0.0,
                [1.5, 0.0, 0.0],
                true,
                1.5,
                tick,
            ));
        }
        let log = brain.decisions().expect("recording is on");
        assert_eq!(log.len(), 2, "the ring keeps the newest two");
        assert!(log.rows().all(|decision| decision.fighter == 2));
        assert!(log
            .rows()
            .all(|decision| usize::from(decision.chosen) < ACTION_NAMES.len()));
    }
}
