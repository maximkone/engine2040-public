//! The input tape of a script (WEB7): the same device events run through [`InputState`] in the
//! browser and on the PC give the same tape bytes.
//!
//! A script is plain text, one command per line, `#` starts a comment:
//!
//! - `key <KeyboardEvent.code> down|up`
//! - `mouse <button> down|up`
//! - `tick`: sample one tape record (fighter slot 0, facing 0, the next tick index) from what is held.
//!
//! The page replays the events as real (trusted) key and mouse events from a headless Chrome and
//! calls `inputSample()` at each `tick`; the `input_tape` bin replays the same file on the PC.
//! Both print the records as hex, so `tools/web-input-check.mjs` compares the bytes and their
//! digest. Sampling at `tick` lines, not at animation frames, keeps the result independent of
//! when the browser happens to draw.

use engine_input::ParticipantError;

use crate::input::InputState;

/// One line of a script.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Step {
    /// A key went down or up; `code` is `KeyboardEvent.code`.
    Key { code: String, down: bool },
    /// A mouse button (`MouseEvent.button`) went down or up.
    Mouse { button: i16, down: bool },
    /// Sample one record.
    Tick,
}

/// Why a script line was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScriptError {
    /// 1-based line number.
    pub line: usize,
    pub why: String,
}

impl std::fmt::Display for ScriptError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "script line {}: {}", self.line, self.why)
    }
}

fn down_up(word: &str) -> Option<bool> {
    match word {
        "down" => Some(true),
        "up" => Some(false),
        _ => None,
    }
}

/// One command, or `None` for anything else (a missing word, an extra word, an unknown word).
fn parse_line(line: &str) -> Option<Step> {
    let words: Vec<&str> = line.split_whitespace().collect();
    match words.as_slice() {
        ["tick"] => Some(Step::Tick),
        ["key", code, dir] => Some(Step::Key {
            code: (*code).to_string(),
            down: down_up(dir)?,
        }),
        ["mouse", button, dir] => Some(Step::Mouse {
            button: button.parse::<u8>().ok()?.into(),
            down: down_up(dir)?,
        }),
        _ => None,
    }
}

/// Read a script. Blank lines and `#` comments are skipped; any other line must be a command.
///
/// # Errors
/// [`ScriptError`] naming the first line that is not a command.
pub fn parse_script(text: &str) -> Result<Vec<Step>, ScriptError> {
    let mut steps = Vec::new();
    for (index, raw) in text.lines().enumerate() {
        let line = raw.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        steps.push(parse_line(line).ok_or_else(|| ScriptError {
            line: index + 1,
            why: format!("not a command: {line}"),
        })?);
    }
    Ok(steps)
}

/// The records sampled so far, as the canonical tape bytes of the participant record (10 bytes a
/// record).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Tape {
    bytes: Vec<u8>,
    ticks: u32,
}

impl Tape {
    /// Sample one record from what `input` holds now: fighter slot 0, facing 0, the next tick.
    ///
    /// # Errors
    /// [`ParticipantError`] when the record is out of range (it cannot be for slot 0 and facing 0).
    pub fn sample(&mut self, input: &InputState) -> Result<(), ParticipantError> {
        input
            .slot0_record(self.ticks)?
            .append_bytes(&mut self.bytes);
        self.ticks += 1;
        Ok(())
    }

    /// Records sampled.
    #[must_use]
    pub fn ticks(&self) -> u32 {
        self.ticks
    }

    /// The tape bytes as lowercase hex.
    #[must_use]
    pub fn hex(&self) -> String {
        self.bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    /// The line the bin and the page print: `input-tape: ticks=<n> bytes=<hex>`.
    #[must_use]
    pub fn line(&self) -> String {
        format!("input-tape: ticks={} bytes={}", self.ticks, self.hex())
    }
}

/// Run a script from nothing held: the tape its `tick` lines sample.
///
/// # Errors
/// [`ParticipantError`] from [`Tape::sample`].
pub fn replay(steps: &[Step]) -> Result<Tape, ParticipantError> {
    let mut state = InputState::default();
    let mut tape = Tape::default();
    for step in steps {
        match step {
            Step::Key { code, down } => {
                state.key_event(code, *down);
            }
            Step::Mouse { button, down } => {
                state.mouse_button(*button, *down);
            }
            Step::Tick => tape.sample(&state)?,
        }
    }
    Ok(tape)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCRIPT: &str = "# a short fight\nkey KeyW down\ntick\nkey KeyF down  # attack\ntick\nkey KeyF up\nmouse 2 down\ntick\nmouse 2 up\nkey KeyW up\ntick\n";

    #[test]
    fn a_script_reads_its_commands_and_skips_comments_and_blank_lines() {
        let steps = parse_script(SCRIPT).expect("a valid script");
        assert_eq!(steps.len(), 10);
        assert_eq!(
            steps[0],
            Step::Key {
                code: "KeyW".into(),
                down: true
            }
        );
        assert_eq!(steps[1], Step::Tick);
        assert_eq!(
            steps[5],
            Step::Mouse {
                button: 2,
                down: true
            }
        );
        assert_eq!(parse_script("\n  \n# only a comment\n").expect("empty"), []);
    }

    #[test]
    fn a_bad_line_is_named_with_its_number() {
        for (text, line) in [
            ("tick\nkey KeyW sideways\n", 2),
            ("key\n", 1),
            ("mouse left down\n", 1),
            ("mouse -1 down\n", 1),
            ("mouse 0\n", 1),
            ("jump\n", 1),
            ("tick now\n", 1),
            ("key KeyW down up\n", 1),
        ] {
            let e = parse_script(text).expect_err(text);
            assert_eq!(e.line, line, "{text:?}: {e}");
            assert!(e.to_string().starts_with("script line "), "{e}");
        }
    }

    #[test]
    fn the_tape_is_the_records_of_the_held_keys_in_ten_byte_steps() {
        let tape = replay(&parse_script(SCRIPT).expect("valid")).expect("replays");
        assert_eq!(tape.ticks(), 4);
        let hex = tape.hex();
        assert_eq!(hex.len(), 4 * 10 * 2);
        // Tick 0: W held, forward 100 (mx = 0x64), nothing pressed.
        assert_eq!(&hex[0..20], "00000000006400000000");
        // Tick 1: W and F: attack bit 1.
        assert_eq!(&hex[20..40], "01000000006400000001");
        // Tick 2: W held, F up, right mouse held: block bit 2.
        assert_eq!(&hex[40..60], "02000000006400000002");
        // Tick 3: everything up.
        assert_eq!(&hex[60..80], "03000000000000000000");
        assert!(tape.line().starts_with("input-tape: ticks=4 bytes=0000"));
    }

    #[test]
    fn the_same_script_gives_the_same_tape_and_an_unknown_key_changes_nothing() {
        let steps = parse_script(SCRIPT).expect("valid");
        assert_eq!(replay(&steps), replay(&steps));
        let with_noise =
            parse_script(&format!("key KeyZ down\n{SCRIPT}key KeyZ up\n")).expect("valid");
        assert_eq!(
            replay(&with_noise).expect("replays").hex(),
            replay(&steps).expect("replays").hex()
        );
        assert_eq!(
            replay(&[]).expect("empty").line(),
            "input-tape: ticks=0 bytes="
        );
    }
}
