//! Dev console: text-line door verbs plus time and presentation knobs.
//!
//! Consumer: the arena window (`bin/arena.rs`). The console is a cold-path
//! tool: one heap line per typed character, no per-frame work, never hashed.
//!
//! Read-only verbs (`hash`, `tick`, `stats`) run against [`ConsoleDoor`], a
//! small seam so `engine-game` never depends on `engine-endpoint`; the window
//! implements it over the live [`App`](crate::App), and the endpoint door can
//! implement the same trait later.
//!
//! Time verbs (`pause`, `step`, `slow`) change only how many whole fixed ticks
//! the wall-frame accumulator is owed. They never change the fixed dt inside a
//! tick, so the authority hash is the same whether the fight ran at real time,
//! half speed or one stepped tick at a time.
//!
//! `set-knob` returns typed presentation effects (`msaa`, `contact-shadow`,
//! `auto-exposure`). The window applies them to the presenter; none of them
//! reach the sim.
//!
//! Errors: [`ConsoleError`] for every refused line, never a panic. Units:
//! time speed in permille of real time (1000 = real time), contact-shadow in
//! permille of full strength.

#[cfg(feature = "app")]
pub mod overlay;

use core::fmt;

/// Longest accepted input line, in bytes. A longer line is refused at execute.
const MAX_INPUT: usize = 256;
/// Transcript lines kept for the overlay: a small bounded tail.
const MAX_LINES: usize = 8;
/// Slowest accepted time scale, permille (one hundredth of real time).
const MIN_SPEED_PERMILLE: u32 = 10;
/// Fastest accepted time scale, permille (four times real time).
const MAX_SPEED_PERMILLE: u32 = 4000;
/// Real-time scale, permille.
const REAL_SPEED_PERMILLE: u32 = 1000;

/// Dark panel that backs the console rows, 0..1 HUD space.
#[cfg(any(feature = "app", test))]
pub(crate) const OVERLAY_PANEL: [f32; 4] = [0.02, 0.62, 0.98, 0.96];
/// Left edge of the console text, 0..1 HUD space.
#[cfg(any(feature = "app", test))]
pub(crate) const OVERLAY_TEXT_X: f32 = 0.035;
/// Glyph height, 0..1 HUD space.
#[cfg(any(feature = "app", test))]
pub(crate) const OVERLAY_TEXT_HEIGHT: f32 = 0.034;
/// Vertical step between rows, 0..1 HUD space.
#[cfg(any(feature = "app", test))]
pub(crate) const OVERLAY_LINE_STEP: f32 = 0.046;
/// Top of the input row, 0..1 HUD space.
#[cfg(any(feature = "app", test))]
pub(crate) const OVERLAY_INPUT_Y: f32 = 0.91;
/// Transcript rows shown above the input.
#[cfg(any(feature = "app", test))]
pub(crate) const OVERLAY_TRANSCRIPT_ROWS: usize = 3;
/// Longest text drawn on one row, in characters.
#[cfg(any(feature = "app", test))]
pub(crate) const OVERLAY_MAX_CHARS: usize = 28;
/// Transcript text color, sRGB bytes.
#[cfg(any(feature = "app", test))]
pub(crate) const OVERLAY_TEXT_COLOR: [u8; 4] = [230, 235, 240, 255];
/// Input text color, sRGB bytes.
#[cfg(any(feature = "app", test))]
pub(crate) const OVERLAY_INPUT_COLOR: [u8; 4] = [170, 225, 170, 255];

/// One short help line per verb, in the order `help` prints them.
const HELP_LINES: [&str; 9] = [
    "hash                 authority hash of the last simulated tick",
    "tick                 fixed tick cursor",
    "stats                door statistics line",
    "pause [on|off]       hold the frame accumulator; bare pause toggles",
    "resume               same as `pause off`",
    "step                 pause and run exactly one tick",
    "slow <scale>         time scale 0.01..4.0; 0.5 is half speed",
    "set-knob <name> <v>  msaa 0|1, contact-shadow 0..1000, auto-exposure 0|1",
    "help                 this list",
];

/// Read-only door verbs the console may ask. The game defines the trait so it
/// does not depend on the endpoint crate; the window and, later, the endpoint
/// door implement it over their own authority state.
pub trait ConsoleDoor {
    /// Authority hash of the last simulated tick, zero before the first tick.
    fn hash(&self) -> u64;
    /// Fixed tick cursor, ticks simulated so far.
    fn tick(&self) -> u32;
    /// One short statistics line, without the `stats` prefix.
    fn stats(&self) -> String;
}

/// A presentation knob `set-knob` accepts. Values are integers in the knob's
/// own range; the window maps the contact-shadow permille to `0.0..=1.0`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ConsoleKnob {
    /// 4x MSAA on the window leg (`0` or `1`).
    Msaa,
    /// Contact-shadow strength in permille (`0..=1000`).
    ContactShadow,
    /// Auto-exposure meter (`0` or `1`).
    AutoExposure,
}

impl ConsoleKnob {
    /// Every knob the console knows, for help and tests.
    pub const ALL: [Self; 3] = [Self::Msaa, Self::ContactShadow, Self::AutoExposure];

    /// Stable name typed on the console line.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Msaa => "msaa",
            Self::ContactShadow => "contact-shadow",
            Self::AutoExposure => "auto-exposure",
        }
    }

    /// Parse one stable knob name.
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "msaa" => Some(Self::Msaa),
            "contact-shadow" => Some(Self::ContactShadow),
            "auto-exposure" => Some(Self::AutoExposure),
            _ => None,
        }
    }

    /// Inclusive integer range the knob accepts.
    #[must_use]
    pub const fn range(self) -> (i64, i64) {
        match self {
            Self::Msaa | Self::AutoExposure => (0, 1),
            Self::ContactShadow => (0, 1000),
        }
    }
}

/// One side effect of a successful console line, applied by the window at a
/// safe point (outside a tick). Time effects are already reflected in the
/// console's own pause/step/speed state; the window only applies the clock and
/// presenter rows.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ConsoleEffect {
    /// Set the virtual clock's target speed, permille of real time.
    Slow {
        /// Accepted speed, `10..=4000`.
        permille: u32,
    },
    /// Set one presentation knob to an accepted value.
    SetKnob {
        /// Which presenter knob.
        knob: ConsoleKnob,
        /// Value in the knob's inclusive range.
        value: i64,
    },
}

/// A successful line: reply lines for the transcript/overlay, plus side
/// effects the window must apply.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct ConsoleReply {
    /// Reply lines, without the `> command` echo.
    pub lines: Vec<String>,
    /// Side effects in command order.
    pub effects: Vec<ConsoleEffect>,
}

/// One console overlay row, already laid out for the window.
#[cfg(any(feature = "app", test))]
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ConsoleRow<'a> {
    /// Text to draw, clipped to [`OVERLAY_MAX_CHARS`] characters.
    pub text: &'a str,
    /// Top edge in 0..1 HUD space.
    pub y: f32,
    /// sRGB text color.
    pub color: [u8; 4],
    /// True for the unsubmitted input row, which draws a `> ` prompt.
    pub prompt: bool,
}

/// A refused console line. Every variant names the verb and the bad part, so
/// the window can print one useful line and keep the console open.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ConsoleError {
    /// The first word is not a verb.
    UnknownVerb(String),
    /// A verb that needs an argument got none.
    MissingArg {
        /// Verb name.
        verb: &'static str,
        /// What was missing.
        arg: &'static str,
    },
    /// A verb got more arguments than it accepts.
    TooManyArgs(&'static str),
    /// A numeric argument did not parse.
    BadNumber {
        /// Verb name.
        verb: &'static str,
        /// The refused text.
        value: String,
    },
    /// A named argument was outside its accepted words.
    BadArg {
        /// Verb name.
        verb: &'static str,
        /// The refused text.
        value: String,
        /// The accepted words, for the message.
        want: &'static str,
    },
    /// A parsed scale is outside the clock's accepted range.
    SlowOutOfRange(u32),
    /// A `set-knob` name is not one of [`ConsoleKnob::ALL`].
    UnknownKnob(String),
    /// A parsed knob value is outside the knob's range.
    KnobOutOfRange {
        /// Which knob.
        knob: ConsoleKnob,
        /// The refused value.
        value: i64,
        /// Inclusive lower bound.
        min: i64,
        /// Inclusive upper bound.
        max: i64,
    },
}

impl fmt::Display for ConsoleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownVerb(verb) => write!(f, "unknown verb `{verb}` (try `help`)"),
            Self::MissingArg { verb, arg } => write!(f, "{verb}: missing {arg}"),
            Self::TooManyArgs(verb) => write!(f, "{verb}: too many arguments"),
            Self::BadNumber { verb, value } => {
                write!(f, "{verb}: `{value}` is not a number")
            }
            Self::BadArg { verb, value, want } => {
                write!(f, "{verb}: `{value}` is not one of {want}")
            }
            Self::SlowOutOfRange(permille) => write!(
                f,
                "slow: {permille} permille is outside {MIN_SPEED_PERMILLE}..={MAX_SPEED_PERMILLE}"
            ),
            Self::UnknownKnob(name) => write!(
                f,
                "set-knob: unknown knob `{name}` (msaa, contact-shadow, auto-exposure)"
            ),
            Self::KnobOutOfRange {
                knob,
                value,
                min,
                max,
            } => write!(
                f,
                "set-knob {}: {value} is outside {min}..={max}",
                knob.name()
            ),
        }
    }
}

impl std::error::Error for ConsoleError {}

/// The dev console: line editor, transcript, pause/step gate and time scale.
///
/// The window asks [`Self::is_open`] before routing keys, then
/// [`Self::take_line`] on Enter and [`Self::execute`] against its door. While
/// the console is paused, [`Self::take_step`] returns exactly one tick for one
/// `step` line and zero otherwise; the window runs that many ticks and never
/// changes the fixed dt.
#[derive(Clone, Debug)]
pub struct DevConsole {
    open: bool,
    input: String,
    lines: Vec<String>,
    paused: bool,
    step_owed: bool,
    speed_permille: u32,
}

impl Default for DevConsole {
    fn default() -> Self {
        Self::new()
    }
}

impl DevConsole {
    /// A closed console at real-time speed, not paused, with an empty
    /// transcript.
    #[must_use]
    pub fn new() -> Self {
        Self {
            open: false,
            input: String::new(),
            lines: Vec::new(),
            paused: false,
            step_owed: false,
            speed_permille: REAL_SPEED_PERMILLE,
        }
    }

    /// Whether keys should go to the console instead of the fight.
    #[must_use]
    pub const fn is_open(&self) -> bool {
        self.open
    }

    /// Open or close the console. Closing drops the unsubmitted line.
    pub fn set_open(&mut self, open: bool) {
        self.open = open;
        if !open {
            self.input.clear();
        }
    }

    /// Toggle the console open/closed.
    pub fn toggle_open(&mut self) {
        self.set_open(!self.open);
    }

    /// The line being typed, for the overlay prompt.
    #[must_use]
    pub fn input(&self) -> &str {
        &self.input
    }

    /// Append one typed character, bounded by [`MAX_INPUT`]. Control
    /// characters are ignored; the window filters them too.
    pub fn push_char(&mut self, ch: char) {
        if !ch.is_control() && self.input.len() + ch.len_utf8() <= MAX_INPUT {
            self.input.push(ch);
        }
    }

    /// Delete the last typed character, if any.
    pub fn backspace(&mut self) {
        self.input.pop();
    }

    /// Clear the unsubmitted line without closing the console.
    pub fn clear_input(&mut self) {
        self.input.clear();
    }

    /// Take the typed line on Enter, leaving the input empty.
    #[must_use]
    pub fn take_line(&mut self) -> String {
        core::mem::take(&mut self.input)
    }

    /// Bounded transcript tail, oldest first, including commands and errors.
    #[must_use]
    pub fn lines(&self) -> &[String] {
        &self.lines
    }

    /// Overlay rows, newest first: the input line then up to
    /// [`OVERLAY_TRANSCRIPT_ROWS`] transcript lines. Each row is clipped to
    /// [`OVERLAY_MAX_CHARS`] characters so the fixed overlay ring never
    /// starves the HP bars.
    #[cfg(any(feature = "app", test))]
    pub(crate) fn overlay_rows(&self) -> impl Iterator<Item = ConsoleRow<'_>> {
        let input = ConsoleRow {
            text: overlay_clip(&self.input),
            y: OVERLAY_INPUT_Y,
            color: OVERLAY_INPUT_COLOR,
            prompt: true,
        };
        let transcript = self
            .lines
            .iter()
            .rev()
            .take(OVERLAY_TRANSCRIPT_ROWS)
            .enumerate()
            .map(|(row, line)| {
                let row = u8::try_from(row).unwrap_or(u8::MAX);
                ConsoleRow {
                    text: overlay_clip(line),
                    y: OVERLAY_INPUT_Y - (f32::from(row) + 1.0) * OVERLAY_LINE_STEP,
                    color: OVERLAY_TEXT_COLOR,
                    prompt: false,
                }
            });
        core::iter::once(input).chain(transcript)
    }

    /// Whether the console holds the frame accumulator.
    #[must_use]
    pub const fn paused(&self) -> bool {
        self.paused
    }

    /// The time scale the console last set, permille of real time.
    #[must_use]
    pub const fn speed_permille(&self) -> u32 {
        self.speed_permille
    }

    /// Ticks the console owes this frame: `None` means running (the caller's
    /// accumulator decides), `Some(0)` means held, `Some(1)` serves one
    /// pending `step`. Testing seam and the window's gate.
    #[must_use]
    pub fn frame_hold(&mut self) -> Option<u32> {
        if !self.paused {
            return None;
        }
        Some(self.take_step())
    }

    /// Consume one owed step, if paused and requested. Never returns more
    /// than one: a second `step` before the frame still advances one tick.
    #[must_use]
    pub fn take_step(&mut self) -> u32 {
        let owed = core::mem::take(&mut self.step_owed);
        u32::from(self.paused && owed)
    }

    /// Resume the frame accumulator; a pending step is dropped so a later
    /// resume never bursts.
    pub fn resume(&mut self) {
        self.paused = false;
        self.step_owed = false;
    }

    /// Execute one complete line against `door`, echoing the command and its
    /// reply/error into the transcript.
    ///
    /// # Errors
    ///
    /// Returns [`ConsoleError`] when the verb is unknown, an argument is
    /// missing, extra, malformed or out of range. A refused line changes no
    /// console state.
    pub fn execute(
        &mut self,
        line: &str,
        door: &dyn ConsoleDoor,
    ) -> Result<ConsoleReply, ConsoleError> {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            return Ok(ConsoleReply::default());
        }
        self.push_line(format!("> {trimmed}"));
        if trimmed.len() > MAX_INPUT {
            let err = ConsoleError::BadArg {
                verb: "line",
                value: trimmed.chars().take(16).collect(),
                want: "a line at most 256 bytes",
            };
            self.push_line(format!("error: {err}"));
            return Err(err);
        }
        match self.run(trimmed, door) {
            Ok(reply) => {
                for reply_line in &reply.lines {
                    self.push_line(reply_line.clone());
                }
                Ok(reply)
            }
            Err(err) => {
                self.push_line(format!("error: {err}"));
                Err(err)
            }
        }
    }

    /// Parse and run one non-empty, trimmed line.
    fn run(&mut self, line: &str, door: &dyn ConsoleDoor) -> Result<ConsoleReply, ConsoleError> {
        let mut words = line.split_whitespace();
        let verb = words.next().unwrap_or_default();
        let mut reply = ConsoleReply::default();
        match verb {
            "hash" => {
                no_more("hash", &mut words)?;
                reply.lines.push(format!("hash {:016x}", door.hash()));
            }
            "tick" => {
                no_more("tick", &mut words)?;
                reply.lines.push(format!("tick {}", door.tick()));
            }
            "stats" => {
                no_more("stats", &mut words)?;
                reply.lines.push(format!("stats {}", door.stats()));
            }
            "pause" => {
                let arg = words.next();
                no_more("pause", &mut words)?;
                let paused = match arg {
                    None => !self.paused,
                    Some("on") => true,
                    Some("off") => false,
                    Some(other) => {
                        return Err(ConsoleError::BadArg {
                            verb: "pause",
                            value: other.to_string(),
                            want: "on or off",
                        });
                    }
                };
                self.set_paused(paused);
                let state = if paused { "paused" } else { "running" };
                reply.lines.push(format!("{state} at tick {}", door.tick()));
            }
            "resume" => {
                no_more("resume", &mut words)?;
                self.set_paused(false);
                reply.lines.push(format!("running at tick {}", door.tick()));
            }
            "step" => {
                no_more("step", &mut words)?;
                self.paused = true;
                self.step_owed = true;
                reply
                    .lines
                    .push(format!("step: one tick at tick {}", door.tick()));
            }
            "slow" => self.run_slow(&mut words, &mut reply)?,
            "set-knob" => self.run_set_knob(&mut words, &mut reply)?,
            "help" => {
                no_more("help", &mut words)?;
                reply
                    .lines
                    .extend(HELP_LINES.iter().map(|line| (*line).to_string()));
            }
            other => return Err(ConsoleError::UnknownVerb(other.to_string())),
        }
        Ok(reply)
    }

    /// `slow <scale>`: parse exact permille, range it, and emit the clock
    /// effect the window applies outside the tick.
    fn run_slow(
        &mut self,
        words: &mut core::str::SplitWhitespace<'_>,
        reply: &mut ConsoleReply,
    ) -> Result<(), ConsoleError> {
        let Some(raw) = words.next() else {
            return Err(ConsoleError::MissingArg {
                verb: "slow",
                arg: "scale",
            });
        };
        no_more("slow", words)?;
        let permille = parse_scale_permille(raw).ok_or_else(|| ConsoleError::BadNumber {
            verb: "slow",
            value: raw.to_string(),
        })?;
        if !(MIN_SPEED_PERMILLE..=MAX_SPEED_PERMILLE).contains(&permille) {
            return Err(ConsoleError::SlowOutOfRange(permille));
        }
        self.speed_permille = permille;
        reply.lines.push(format!("slow {permille} permille"));
        reply.effects.push(ConsoleEffect::Slow { permille });
        Ok(())
    }

    /// `set-knob <name> <value>`: parse the named knob, range the value, and
    /// emit the presenter effect.
    fn run_set_knob(
        &mut self,
        words: &mut core::str::SplitWhitespace<'_>,
        reply: &mut ConsoleReply,
    ) -> Result<(), ConsoleError> {
        let Some(name) = words.next() else {
            return Err(ConsoleError::MissingArg {
                verb: "set-knob",
                arg: "knob",
            });
        };
        let Some(raw) = words.next() else {
            return Err(ConsoleError::MissingArg {
                verb: "set-knob",
                arg: "value",
            });
        };
        no_more("set-knob", words)?;
        let knob =
            ConsoleKnob::parse(name).ok_or_else(|| ConsoleError::UnknownKnob(name.to_string()))?;
        let value: i64 = raw.parse().map_err(|_| ConsoleError::BadNumber {
            verb: "set-knob",
            value: raw.to_string(),
        })?;
        let (min, max) = knob.range();
        if !(min..=max).contains(&value) {
            return Err(ConsoleError::KnobOutOfRange {
                knob,
                value,
                min,
                max,
            });
        }
        reply
            .lines
            .push(format!("set-knob {} {value}", knob.name()));
        reply.effects.push(ConsoleEffect::SetKnob { knob, value });
        Ok(())
    }

    /// Set the hold state; turning it off drops any pending step.
    fn set_paused(&mut self, paused: bool) {
        self.paused = paused;
        if !paused {
            self.step_owed = false;
        }
    }

    /// Append one transcript line, dropping the oldest past [`MAX_LINES`].
    fn push_line(&mut self, line: String) {
        self.lines.push(line);
        if self.lines.len() > MAX_LINES {
            self.lines.remove(0);
        }
    }
}

/// Refuse any word after `verb` has taken its accepted arguments.
fn no_more<'a>(
    verb: &'static str,
    words: &mut impl Iterator<Item = &'a str>,
) -> Result<(), ConsoleError> {
    if words.next().is_some() {
        return Err(ConsoleError::TooManyArgs(verb));
    }
    Ok(())
}

/// Parse a decimal time scale as exact permille: `0.5` -> 500, `1` -> 1000,
/// `0.25` -> 250. At most three decimal places; no signs, spaces or exponent.
/// Returns `None` for malformed text (the caller ranges it separately).
fn parse_scale_permille(text: &str) -> Option<u32> {
    let (whole, fraction) = match text.split_once('.') {
        Some((whole, fraction)) => (whole, fraction),
        None => (text, ""),
    };
    if whole.is_empty() && fraction.is_empty() {
        return None;
    }
    if !whole.chars().all(|ch| ch.is_ascii_digit())
        || !fraction.chars().all(|ch| ch.is_ascii_digit())
    {
        return None;
    }
    let whole: u32 = if whole.is_empty() {
        0
    } else {
        whole.parse().ok()?
    };
    let (fraction, scale) = match fraction.len() {
        0 => (0_u32, 1000_u32),
        1 => (fraction.parse().ok()?, 100_u32),
        2 => (fraction.parse().ok()?, 10_u32),
        3 => (fraction.parse().ok()?, 1_u32),
        _ => return None,
    };
    whole
        .checked_mul(1000)?
        .checked_add(fraction.checked_mul(scale)?)
}

/// Clip one overlay row to [`OVERLAY_MAX_CHARS`] characters without a heap
/// allocation; UTF-8 boundaries stay whole.
#[cfg(any(feature = "app", test))]
pub(crate) fn overlay_clip(line: &str) -> &str {
    match line.char_indices().nth(OVERLAY_MAX_CHARS) {
        Some((at, _)) => &line[..at],
        None => line,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::App;
    use crate::GameConfig;

    /// A 60 Hz wall frame and the fixed step it feeds, milliseconds.
    const FRAME_MS: f64 = 1000.0 / 60.0;
    const FIXED_MS: f64 = 1000.0 / 60.0;

    /// The window's read-only door over the real App authority.
    struct AppDoor<'a>(&'a App);

    impl ConsoleDoor for AppDoor<'_> {
        fn hash(&self) -> u64 {
            self.0.checkpoints().last().map_or(0, |point| point.hash)
        }

        fn tick(&self) -> u32 {
            self.0.cursor()
        }

        fn stats(&self) -> String {
            format!(
                "fighters={} sim_steps={} hp={:?}",
                self.0.fighter_views().len(),
                self.0.sim_steps(),
                self.0.hp()
            )
        }
    }

    /// One real session at the proof fixture's seed and length.
    fn authority_app(ticks: u32) -> App {
        App::new(GameConfig {
            seed: 2040,
            ticks,
            fighters: 2,
            half_extent_m: 20.0,
        })
        .expect("a valid session config")
    }

    /// One wall frame through the console gate: paused serves at most one
    /// owed step, otherwise half-speed frames accumulate exact milliseconds.
    fn run_frame(app: &mut App, console: &mut DevConsole, acc_ms: &mut f64) -> u32 {
        if let Some(steps) = console.frame_hold() {
            for _ in 0..steps {
                app.tick_empty();
            }
            return steps;
        }
        *acc_ms += FRAME_MS * f64::from(console.speed_permille()) / 1000.0;
        let mut steps = 0;
        while *acc_ms >= FIXED_MS && steps < 2 {
            *acc_ms -= FIXED_MS;
            app.tick_empty();
            steps += 1;
        }
        steps
    }

    #[test]
    fn console_step_advances_exactly_one_tick_on_the_real_app() {
        let mut app = authority_app(8);
        let mut console = DevConsole::new();
        let mut acc_ms = 0.0;
        {
            let door = AppDoor(&app);
            console.execute("pause", &door).expect("pause");
        }
        assert!(console.paused());
        assert_eq!(run_frame(&mut app, &mut console, &mut acc_ms), 0);
        assert_eq!(app.cursor(), 0);

        {
            let door = AppDoor(&app);
            console.execute("step", &door).expect("step");
        }
        assert_eq!(run_frame(&mut app, &mut console, &mut acc_ms), 1);
        assert_eq!(app.cursor(), 1);
        assert_eq!(run_frame(&mut app, &mut console, &mut acc_ms), 0);
        assert_eq!(app.cursor(), 1, "one step line never runs a second tick");

        {
            let door = AppDoor(&app);
            console.execute("resume", &door).expect("resume");
        }
        assert!(!console.paused());
        assert_eq!(run_frame(&mut app, &mut console, &mut acc_ms), 1);
        assert_eq!(app.cursor(), 2, "resume returns to real-time frames");
        assert_eq!(run_frame(&mut app, &mut console, &mut acc_ms), 1);
        assert_eq!(app.cursor(), 3, "real time keeps one tick per 60 Hz frame");
    }

    #[test]
    fn console_step_slow_half_gives_one_tick_per_two_frames() {
        let mut app = authority_app(8);
        let mut console = DevConsole::new();
        let mut acc_ms = 0.0;
        {
            let door = AppDoor(&app);
            let reply = console.execute("slow 0.5", &door).expect("slow");
            assert_eq!(reply.effects, vec![ConsoleEffect::Slow { permille: 500 }]);
        }
        assert_eq!(console.speed_permille(), 500);
        assert_eq!(run_frame(&mut app, &mut console, &mut acc_ms), 0);
        assert_eq!(app.cursor(), 0);
        assert_eq!(run_frame(&mut app, &mut console, &mut acc_ms), 1);
        assert_eq!(app.cursor(), 1);
        assert_eq!(run_frame(&mut app, &mut console, &mut acc_ms), 0);
        assert_eq!(run_frame(&mut app, &mut console, &mut acc_ms), 1);
        assert_eq!(app.cursor(), 2, "two half-speed frames give one tick");
    }

    #[test]
    fn console_step_hash_at_600_matches_the_app_authority() {
        let mut direct = authority_app(600);
        for _ in 0..600 {
            direct.tick_empty();
        }
        let authority = direct
            .checkpoints()
            .last()
            .expect("600 ticks leave a checkpoint")
            .hash;
        assert_ne!(authority, 0, "the authority hash is a real digest");

        let mut paced = authority_app(600);
        let mut console = DevConsole::new();
        let mut acc_ms = 0.0;
        {
            let door = AppDoor(&paced);
            console.execute("slow 0.5", &door).expect("slow");
            console.execute("pause", &door).expect("pause");
            console.execute("step", &door).expect("step");
        }
        assert_eq!(run_frame(&mut paced, &mut console, &mut acc_ms), 1);
        {
            let door = AppDoor(&paced);
            console.execute("resume", &door).expect("resume");
        }
        while paced.cursor() < 600 {
            run_frame(&mut paced, &mut console, &mut acc_ms);
        }
        assert_eq!(paced.cursor(), 600);
        let paced_hash = paced
            .checkpoints()
            .last()
            .expect("600 ticks leave a checkpoint")
            .hash;
        assert_eq!(
            paced_hash, authority,
            "console pacing never moves the authority hash"
        );

        let door = AppDoor(&paced);
        assert_eq!(
            console.execute("hash", &door).expect("hash").lines,
            vec![format!("hash {authority:016x}")]
        );
        assert_eq!(
            console.execute("tick", &door).expect("tick").lines,
            vec!["tick 600".to_string()]
        );
        let stats = console.execute("stats", &door).expect("stats").lines;
        assert!(stats[0].contains("fighters=2"), "{stats:?}");
        assert!(stats[0].contains("sim_steps=600"), "{stats:?}");
    }

    #[test]
    fn console_bad_lines_are_typed_and_never_panic() {
        let app = authority_app(2);
        let door = AppDoor(&app);
        let mut console = DevConsole::new();
        assert_eq!(
            console.execute("nope", &door),
            Err(ConsoleError::UnknownVerb("nope".to_string()))
        );
        assert_eq!(
            console.execute("slow", &door),
            Err(ConsoleError::MissingArg {
                verb: "slow",
                arg: "scale"
            })
        );
        assert_eq!(
            console.execute("slow nope", &door),
            Err(ConsoleError::BadNumber {
                verb: "slow",
                value: "nope".to_string()
            })
        );
        assert_eq!(
            console.execute("slow 0", &door),
            Err(ConsoleError::SlowOutOfRange(0))
        );
        assert_eq!(
            console.execute("slow 4.5", &door),
            Err(ConsoleError::SlowOutOfRange(4500))
        );
        assert_eq!(
            console.execute("pause maybe", &door),
            Err(ConsoleError::BadArg {
                verb: "pause",
                value: "maybe".to_string(),
                want: "on or off"
            })
        );
        assert_eq!(
            console.execute("set-knob msaa 2", &door),
            Err(ConsoleError::KnobOutOfRange {
                knob: ConsoleKnob::Msaa,
                value: 2,
                min: 0,
                max: 1
            })
        );
        assert_eq!(
            console.execute("hash now", &door),
            Err(ConsoleError::TooManyArgs("hash"))
        );
        assert_eq!(console.speed_permille(), REAL_SPEED_PERMILLE);
        assert!(!console.paused());
        assert!(console.execute("help", &door).is_ok());
        let reply = console
            .execute("set-knob contact-shadow 250", &door)
            .expect("knob");
        assert_eq!(
            reply.effects,
            vec![ConsoleEffect::SetKnob {
                knob: ConsoleKnob::ContactShadow,
                value: 250
            }]
        );
    }

    #[test]
    fn console_scale_parser_is_exact_and_bounded() {
        assert_eq!(parse_scale_permille("0.5"), Some(500));
        assert_eq!(parse_scale_permille("1"), Some(1000));
        assert_eq!(parse_scale_permille("2.25"), Some(2250));
        assert_eq!(parse_scale_permille(".5"), Some(500));
        assert_eq!(parse_scale_permille("0.01"), Some(10));
        assert_eq!(parse_scale_permille("4.0"), Some(4000));
        assert_eq!(parse_scale_permille("0.1234"), None);
        assert_eq!(parse_scale_permille("1e2"), None);
        assert_eq!(parse_scale_permille("-1"), None);
        assert_eq!(parse_scale_permille("1.2.3"), None);
        assert_eq!(parse_scale_permille("."), None);
    }

    #[test]
    fn console_overlay_rows_show_input_then_newest_transcript() {
        let app = authority_app(2);
        let door = AppDoor(&app);
        let mut console = DevConsole::new();
        console.set_open(true);
        for ch in "hash 0".chars() {
            console.push_char(ch);
        }
        let rows: Vec<_> = console.overlay_rows().collect();
        assert_eq!(rows.len(), 1);
        assert!(rows[0].prompt);
        assert_eq!(rows[0].text, "hash 0");
        assert_eq!(rows[0].y, OVERLAY_INPUT_Y);
        assert_eq!(rows[0].color, OVERLAY_INPUT_COLOR);

        console.execute("hash", &door).expect("hash");
        let rows: Vec<_> = console.overlay_rows().collect();
        assert_eq!(rows.len(), 3, "input plus the two transcript lines");
        assert!(rows[0].prompt, "the input is the newest row");
        assert_eq!(rows[0].text, "hash 0");
        assert!(!rows[1].prompt);
        assert!(rows[1].text.starts_with("hash "), "{:?}", rows[1].text);
        assert_eq!(rows[1].color, OVERLAY_TEXT_COLOR);
        assert!(!rows[2].prompt);
        assert_eq!(rows[2].text, "> hash");
        assert!(rows[0].y > rows[1].y);
        assert!(rows[1].y > rows[2].y);

        for _ in 0..OVERLAY_MAX_CHARS + 4 {
            console.push_char('x');
        }
        let row = console.overlay_rows().next().expect("input row");
        assert_eq!(row.text.chars().count(), OVERLAY_MAX_CHARS);
        assert!(row.text.is_ascii());
    }
}
