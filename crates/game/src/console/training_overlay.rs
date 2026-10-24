//! training_overlay: why the exchange went that way, as HUD rows.
//!
//! Consumer: the dev console overlay ([`crate::console::overlay`]), staged by
//! the windowed presenter behind a toggle key. Data owner: `engine-game`;
//! nothing here enters the tick, the tape or the state hash. Units: ticks at
//! 60 Hz, advantage in frames (positive is the attacker's favour). Press
//! history is bounded by the buffered-input window the feel tables already use
//! ([`engine_combat::feel::MAX_BUFFER_TICKS`], 12 ticks), so the readout can
//! never outgrow the buffer the tick reads.
//!
//! Determinism: fixed arrays, oldest-to-newest order, no clock and no hash
//! order; the same presses and exchanges give the same rows twice. Budget:
//! `step` is O(rows) with no allocation; `rows` allocates only the strings it
//! draws.

/// Press rows kept: the buffered-input window of the feel tables
/// (`engine_combat::feel::MAX_BUFFER_TICKS`, 12 ticks), so the readout can
/// never outgrow the buffer the tick reads.
pub const PRESS_ROWS: usize = 12;

/// One remembered press: the tick it landed on and its one-word label.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct PressRow {
    /// Fixed tick the press was taken on.
    pub tick: u32,
    /// What was pressed (`"light"`, `"block"`, ...).
    pub label: &'static str,
}

/// The answer one exchange took, with the advantage it left.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Exchange {
    /// Held block soaked the blow: the attacker is minus three frames.
    Blocked,
    /// Block inside the parry window: the defender answers, plus nine.
    Parried,
    /// Nothing connected: even frames.
    Whiff,
    /// A landed blow, with the frame advantage the tick derived from the move's
    /// total duration and its last hit frame (never authored twice here).
    Landed {
        /// Frames the attacker is plus after the blow.
        frames: i8,
    },
}

impl Exchange {
    /// Frame advantage this answer leaves the attacker, in frames.
    #[must_use]
    pub fn advantage_frames(self) -> i16 {
        match self {
            Self::Blocked => -3,
            Self::Parried => 9,
            Self::Whiff => 0,
            Self::Landed { frames } => i16::from(frames),
        }
    }

    /// One-word row label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Blocked => "blocked",
            Self::Parried => "parry",
            Self::Whiff => "whiff",
            Self::Landed { .. } => "landed",
        }
    }
}

/// One exchange the overlay remembers.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ExchangeRow {
    /// Fixed tick the exchange resolved on.
    pub tick: u32,
    /// The answer the tick took.
    pub answer: Exchange,
}

/// One drawn row: its age in ticks and the text to stage.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct OverlayRow {
    /// Ticks since the press or exchange (`0` on the tick it happened).
    pub age_ticks: u32,
    /// Row text, e.g. `"light -3t"` or `"blocked -3"`.
    pub text: String,
}

/// The training readout: the last presses, the last exchanges and the toggle.
/// Pure data beside the tick; presentation only.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct TrainingOverlay {
    enabled: bool,
    presses: [PressRow; PRESS_ROWS],
    presses_len: usize,
    exchanges: [ExchangeRow; PRESS_ROWS],
    exchanges_len: usize,
}

impl Default for TrainingOverlay {
    fn default() -> Self {
        Self::new()
    }
}

impl TrainingOverlay {
    /// A closed overlay with no history.
    #[must_use]
    pub fn new() -> Self {
        Self {
            enabled: false,
            presses: [PressRow { tick: 0, label: "" }; PRESS_ROWS],
            presses_len: 0,
            exchanges: [ExchangeRow {
                tick: 0,
                answer: Exchange::Whiff,
            }; PRESS_ROWS],
            exchanges_len: 0,
        }
    }

    /// True when the readout draws.
    #[must_use]
    pub const fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Flip the readout: the key that turns it on and off. A closed overlay
    /// keeps its history, so opening it again shows what just happened.
    pub fn toggle(&mut self) {
        self.enabled = !self.enabled;
    }

    /// Fold one tick: every press taken on it and at most one exchange answer.
    /// A tick with no press and no exchange adds nothing (idle ticks never
    /// grow the readout). Oldest rows fall off at the buffer window.
    pub fn step(&mut self, tick: u32, presses: &[&'static str], answer: Option<Exchange>) {
        for label in presses {
            push_row(
                &mut self.presses,
                &mut self.presses_len,
                PressRow { tick, label },
            );
        }
        if let Some(answer) = answer {
            push_row(
                &mut self.exchanges,
                &mut self.exchanges_len,
                ExchangeRow { tick, answer },
            );
        }
    }

    /// Presses kept, oldest first.
    #[must_use]
    pub fn presses(&self) -> &[PressRow] {
        &self.presses[..self.presses_len]
    }

    /// Exchanges kept, oldest first.
    #[must_use]
    pub fn exchanges(&self) -> &[ExchangeRow] {
        &self.exchanges[..self.exchanges_len]
    }

    /// Rows to stage at `now`: press rows oldest first, then exchange rows,
    /// each carrying its age. A closed overlay draws nothing.
    #[must_use]
    pub fn rows(&self, now: u32) -> Vec<OverlayRow> {
        if !self.enabled {
            return Vec::new();
        }
        let mut rows = Vec::with_capacity(self.presses_len + self.exchanges_len);
        for press in self.presses() {
            rows.push(OverlayRow {
                age_ticks: now.saturating_sub(press.tick),
                text: format!("{} -{}t", press.label, now.saturating_sub(press.tick)),
            });
        }
        for exchange in self.exchanges() {
            let answer = exchange.answer;
            rows.push(OverlayRow {
                age_ticks: now.saturating_sub(exchange.tick),
                text: format!("{} {:+}", answer.label(), answer.advantage_frames()),
            });
        }
        rows
    }
}

/// Append one row to a bounded oldest-first window, shifting the oldest out
/// when it is full.
fn push_row<R: Copy>(rows: &mut [R; PRESS_ROWS], len: &mut usize, row: R) {
    if *len < PRESS_ROWS {
        rows[*len] = row;
        *len += 1;
        return;
    }
    rows.copy_within(1..PRESS_ROWS, 0);
    rows[PRESS_ROWS - 1] = row;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(count: usize) -> Vec<&'static str> {
        const NAMES: [&str; 16] = [
            "p0", "p1", "p2", "p3", "p4", "p5", "p6", "p7", "p8", "p9", "p10", "p11", "p12", "p13",
            "p14", "p15",
        ];
        NAMES[..count].to_vec()
    }

    #[test]
    fn training_overlay_keeps_press_order_and_saturates_at_twelve() {
        let mut overlay = TrainingOverlay::new();
        overlay.toggle();
        // Exactly one window of presses keeps every row, in press order.
        for (index, label) in labels(PRESS_ROWS).into_iter().enumerate() {
            overlay.step(u32::try_from(index).unwrap_or(0), &[label], None);
        }
        let kept: Vec<&str> = overlay.presses().iter().map(|row| row.label).collect();
        assert_eq!(kept, labels(PRESS_ROWS));
        // Three more presses: the window saturates, the newest twelve stay.
        let all = labels(PRESS_ROWS + 3);
        for (index, label) in all.iter().enumerate().skip(PRESS_ROWS) {
            overlay.step(u32::try_from(index).unwrap_or(0), &[label], None);
        }
        let kept: Vec<&str> = overlay.presses().iter().map(|row| row.label).collect();
        assert_eq!(kept.len(), PRESS_ROWS);
        assert_eq!(kept, all[3..].to_vec());
        // Tick order survives the shift: oldest first, newest last.
        let ticks: Vec<u32> = overlay.presses().iter().map(|row| row.tick).collect();
        assert!(ticks.windows(2).all(|pair| pair[0] < pair[1]), "{ticks:?}");
    }

    #[test]
    fn training_overlay_blocked_exchange_reads_minus_three() {
        let mut overlay = TrainingOverlay::new();
        overlay.toggle();
        overlay.step(600, &["block"], Some(Exchange::Blocked));
        let rows = overlay.rows(600);
        assert!(
            rows.iter().any(|row| row.text == "blocked -3"),
            "{:?}",
            rows.iter().map(|row| &row.text).collect::<Vec<_>>()
        );
        // Parry and whiff carry their own numbers; a landed blow carries the
        // number the tick derived.
        overlay.step(601, &[], Some(Exchange::Parried));
        overlay.step(602, &[], Some(Exchange::Whiff));
        overlay.step(603, &[], Some(Exchange::Landed { frames: 5 }));
        let texts: Vec<String> = overlay.rows(603).into_iter().map(|row| row.text).collect();
        assert!(texts.contains(&"parry +9".to_owned()), "{texts:?}");
        assert!(texts.contains(&"whiff +0".to_owned()), "{texts:?}");
        assert!(texts.contains(&"landed +5".to_owned()), "{texts:?}");
        assert_eq!(Exchange::Blocked.advantage_frames(), -3);
    }

    #[test]
    fn training_overlay_idle_tick_adds_no_row() {
        let mut overlay = TrainingOverlay::new();
        overlay.toggle();
        overlay.step(10, &["light"], Some(Exchange::Whiff));
        let before = overlay.clone();
        // A tick with no press and no exchange changes nothing.
        overlay.step(11, &[], None);
        assert_eq!(overlay, before);
        // Ages still move on an idle tick: the rows are the same, older.
        assert_eq!(overlay.rows(10), before.rows(10));
        assert_eq!(overlay.rows(11)[0].age_ticks, 1);
        // A closed overlay draws nothing at all.
        overlay.toggle();
        assert!(overlay.rows(11).is_empty());
    }

    #[test]
    fn training_overlay_same_history_is_equal() {
        let build = || {
            let mut overlay = TrainingOverlay::new();
            overlay.toggle();
            for (index, label) in labels(PRESS_ROWS + 4).into_iter().enumerate() {
                let tick = u32::try_from(index).unwrap_or(0);
                let answer = match index % 4 {
                    0 => Some(Exchange::Blocked),
                    1 => Some(Exchange::Parried),
                    2 => Some(Exchange::Whiff),
                    _ => None,
                };
                let presses: &[&'static str] = &[label];
                overlay.step(tick, presses, answer);
            }
            overlay
        };
        let first = build();
        let second = build();
        assert_eq!(first, second, "same history, same overlay");
        let now = u32::try_from(PRESS_ROWS).unwrap_or(0);
        assert_eq!(first.rows(now), second.rows(now));
        assert_eq!(first.presses().len(), PRESS_ROWS);
    }
}
