//! E9 console panel: one input line, a bounded transcript and typed errors.
//!
//! Owner: the dock. The door is the seam: the host supplies an adapter with the
//! [`ConsoleDoor`] shape (`DevConsole::take_line` plus its reply lines), so this
//! panel runs every command through the real console without the editor naming
//! that crate. Reply lines land as [`ConsoleRowKind::Reply`] rows; a refused
//! line lands as a [`ConsoleRowKind::Error`] row and is also returned, so a
//! hazard check is drawn and never swallowed.
//! Determinism: rows keep push order and the ring drops the oldest at
//! [`CONSOLE_CAP`], the same ring style as the activity feed and the log.
//! Errors: [`ConsoleError`]; a refused line changes the transcript by one echo
//! and one error row and nothing else.

use std::collections::VecDeque;

/// Most transcript rows one console keeps; the oldest leaves first.
pub const CONSOLE_CAP: usize = 200;
/// Longest input line in chars.
pub const MAX_LINE_CHARS: usize = 512;

/// What the console refused, already carrying its recovery.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConsoleError {
    /// The door knows no such verb.
    UnknownVerb {
        /// The verb text as typed.
        verb: String,
    },
    /// The verb exists but the line was bad.
    BadArgs {
        /// Why the door refused it.
        reason: String,
    },
    /// The input line went over [`MAX_LINE_CHARS`].
    LineTooLong {
        /// Chars typed.
        len: usize,
    },
}

impl std::fmt::Display for ConsoleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownVerb { verb } => write!(
                f,
                "unknown console verb `{verb}`; type `help` for the verb list"
            ),
            Self::BadArgs { reason } => {
                write!(
                    f,
                    "console refused the line: {reason}; check the verb's args"
                )
            }
            Self::LineTooLong { len } => write!(
                f,
                "console line holds {len} chars over the {MAX_LINE_CHARS} bound; shorten it"
            ),
        }
    }
}

impl std::error::Error for ConsoleError {}

/// The console door the host hands the panel: one line in, reply lines out.
pub trait ConsoleDoor {
    /// Run one line and return its reply lines, or the typed refusal.
    fn take_line(&mut self, line: &str) -> Result<Vec<String>, ConsoleError>;
}

/// Kind of one transcript row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConsoleRowKind {
    /// The line the person typed (echoed before it runs).
    Input,
    /// One reply line from the door.
    Reply,
    /// A typed refusal, drawn as its own row.
    Error,
}

/// One transcript row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConsoleRow {
    /// Where the row came from.
    pub kind: ConsoleRowKind,
    /// The row text.
    pub text: String,
}

/// Console panel state: the draft line plus a bounded transcript.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ConsolePanel {
    input: String,
    rows: VecDeque<ConsoleRow>,
    dropped: u64,
}

impl ConsolePanel {
    /// Empty console with an empty draft.
    pub fn new() -> Self {
        Self::default()
    }

    /// Current draft line.
    pub fn input(&self) -> &str {
        &self.input
    }

    /// Replace the draft line. Over-long text refuses and keeps the draft.
    pub fn set_input(&mut self, text: &str) -> Result<(), ConsoleError> {
        let len = text.chars().count();
        if len > MAX_LINE_CHARS {
            return Err(ConsoleError::LineTooLong { len });
        }
        self.input.clear();
        self.input.push_str(text);
        Ok(())
    }

    /// Append one typed character to the draft.
    pub fn type_char(&mut self, c: char) -> Result<(), ConsoleError> {
        let len = self.input.chars().count();
        if len >= MAX_LINE_CHARS {
            return Err(ConsoleError::LineTooLong { len: len + 1 });
        }
        self.input.push(c);
        Ok(())
    }

    /// Drop the last character of the draft.
    pub fn backspace(&mut self) {
        self.input.pop();
    }

    /// Transcript rows oldest first.
    pub fn rows(&self) -> Vec<&ConsoleRow> {
        self.rows.iter().collect()
    }

    /// How many rows the ring dropped.
    pub fn dropped(&self) -> u64 {
        self.dropped
    }

    /// Run the draft line through `door`.
    ///
    /// The line is echoed as an [`ConsoleRowKind::Input`] row, then every reply
    /// line lands as a `Reply` row. A refusal lands as an `Error` row and comes
    /// back as `Err`, so the panel draws it and the caller sees it. An empty
    /// draft runs nothing and returns `Ok(0)`; a successful run clears the
    /// draft, a refused one keeps it for a fix.
    pub fn submit(&mut self, door: &mut impl ConsoleDoor) -> Result<usize, ConsoleError> {
        let line = self.input.trim().to_string();
        if line.is_empty() {
            return Ok(0);
        }
        self.push_row(ConsoleRowKind::Input, &line);
        match door.take_line(&line) {
            Ok(replies) => {
                for reply in &replies {
                    self.push_row(ConsoleRowKind::Reply, reply);
                }
                self.input.clear();
                Ok(replies.len())
            }
            Err(error) => {
                self.push_row(ConsoleRowKind::Error, &error.to_string());
                Err(error)
            }
        }
    }

    /// Push one row, evicting the oldest past [`CONSOLE_CAP`].
    fn push_row(&mut self, kind: ConsoleRowKind, text: &str) {
        self.rows.push_back(ConsoleRow {
            kind,
            text: text.to_string(),
        });
        if self.rows.len() > CONSOLE_CAP {
            self.rows.pop_front();
            self.dropped = self.dropped.saturating_add(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Fake door: `hash` answers a digest, `count` answers a fresh number,
    /// `hold` refuses its args, anything else is an unknown verb.
    struct FakeDoor {
        calls: u32,
    }

    impl ConsoleDoor for FakeDoor {
        fn take_line(&mut self, line: &str) -> Result<Vec<String>, ConsoleError> {
            match line {
                "hash" => Ok(vec!["d57b5c8ae3096614".to_string()]),
                "count" => {
                    self.calls += 1;
                    Ok(vec![self.calls.to_string()])
                }
                "hold" => Err(ConsoleError::BadArgs {
                    reason: "`hold` needs a tick count".to_string(),
                }),
                other => Err(ConsoleError::UnknownVerb {
                    verb: other.to_string(),
                }),
            }
        }
    }

    fn door() -> FakeDoor {
        FakeDoor { calls: 0 }
    }

    #[test]
    fn hash_lands_one_reply_line_and_clears_the_draft() {
        let mut panel = ConsolePanel::new();
        let mut door = door();
        for c in "hash".chars() {
            panel.type_char(c).unwrap();
        }
        assert_eq!(panel.input(), "hash");
        assert_eq!(panel.submit(&mut door).unwrap(), 1);
        assert_eq!(panel.input(), "", "a good line clears the draft");
        let rows = panel.rows();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].kind, ConsoleRowKind::Input);
        assert_eq!(rows[0].text, "hash");
        assert_eq!(rows[1].kind, ConsoleRowKind::Reply);
        assert_eq!(rows[1].text, "d57b5c8ae3096614");
        // An empty draft runs nothing.
        assert_eq!(panel.submit(&mut door).unwrap(), 0);
        assert_eq!(panel.rows().len(), 2);
    }

    #[test]
    fn unknown_verb_shows_its_recovery_row_and_keeps_the_draft() {
        let mut panel = ConsolePanel::new();
        let mut door = door();
        panel.set_input("nope").unwrap();
        let error = panel.submit(&mut door).unwrap_err();
        assert_eq!(
            error,
            ConsoleError::UnknownVerb {
                verb: "nope".to_string()
            }
        );
        assert_eq!(panel.input(), "nope", "a refused line stays for a fix");
        let rows = panel.rows();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[1].kind, ConsoleRowKind::Error);
        assert!(rows[1].text.contains("type `help`"), "{}", rows[1].text);

        panel.set_input("hold").unwrap();
        let error = panel.submit(&mut door).unwrap_err();
        assert!(error.to_string().contains("needs a tick count"));
        assert_eq!(panel.rows().last().unwrap().kind, ConsoleRowKind::Error);
    }

    #[test]
    fn transcript_cap_drops_the_oldest_row() {
        let mut panel = ConsolePanel::new();
        let mut door = door();
        for _ in 0..CONSOLE_CAP {
            panel.set_input("count").unwrap();
            panel.submit(&mut door).unwrap();
        }
        // Two rows per run: the ring keeps the last 100 runs out of the 200.
        assert_eq!(panel.rows().len(), CONSOLE_CAP);
        assert!(panel.dropped() > 0);
        let first = panel.rows()[1].text.clone();
        assert_eq!(
            first,
            (CONSOLE_CAP / 2 + 1).to_string(),
            "the oldest kept reply is the 101st count"
        );
        panel.set_input("count").unwrap();
        panel.submit(&mut door).unwrap();
        assert_eq!(panel.rows().len(), CONSOLE_CAP);
        let second = panel.rows()[1].text.clone();
        assert_ne!(second, first, "the oldest reply left");
    }

    #[test]
    fn the_draft_is_bounded_and_editable() {
        let mut panel = ConsolePanel::new();
        panel.set_input("abc").unwrap();
        panel.backspace();
        assert_eq!(panel.input(), "ab");
        let long = "x".repeat(MAX_LINE_CHARS + 1);
        assert_eq!(
            panel.set_input(&long),
            Err(ConsoleError::LineTooLong {
                len: MAX_LINE_CHARS + 1
            })
        );
        assert_eq!(panel.input(), "ab", "a refused draft keeps the old text");
        let full = "x".repeat(MAX_LINE_CHARS);
        panel.set_input(&full).unwrap();
        assert_eq!(
            panel.type_char('y'),
            Err(ConsoleError::LineTooLong {
                len: MAX_LINE_CHARS + 1
            })
        );
        assert_eq!(panel.input().chars().count(), MAX_LINE_CHARS);
    }
}
