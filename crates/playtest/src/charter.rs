//! Charter runner: one question file per bout, PASS/FAIL lines.
//!
//! Caller: `crates/playtest/src/tournament.rs` (`TournamentReport::to_text`).
//! Same text gives same charter; same log gives same verdict.

use std::fmt::{self, Display, Formatter};

/// Bug severity: `A` blocks, `B` hurts, `C` polishes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Severity {
    A,
    B,
    C,
}

impl Severity {
    /// Parse one `A|B|C` letter; anything else is refused.
    fn parse(text: &str) -> Option<Self> {
        match text {
            "A" => Some(Self::A),
            "B" => Some(Self::B),
            "C" => Some(Self::C),
            _ => None,
        }
    }

    /// Severity as the single letter the TOML carries.
    pub const fn letter(self) -> char {
        match self {
            Self::A => 'A',
            Self::B => 'B',
            Self::C => 'C',
        }
    }
}

/// One charter question with its run and its pass/fail lines.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Charter {
    pub id: String,
    pub question: String,
    pub run: String,
    pub pass_line: String,
    pub fail_line: String,
    pub sev: Severity,
}

/// One charter verdict: which line printed.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct CharterVerdict {
    pub pass: bool,
    pub line: String,
}

/// Charter parse failures; every variant names the culprit.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum CharterError {
    MissingField(&'static str),
    BadSev(String),
}

impl Display for CharterError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingField(key) => write!(f, "charter is missing {key}"),
            Self::BadSev(text) => write!(f, "charter sev is not A|B|C: {text}"),
        }
    }
}

impl std::error::Error for CharterError {}

/// Value of one `key = "value"` line; `None` when the line is not one.
fn field_value<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let (name, value) = line.split_once('=')?;
    if name.trim() != key {
        return None;
    }
    let value = value.trim();
    let inner = value.strip_prefix('"')?.strip_suffix('"')?;
    Some(inner)
}

/// Find one required field in the text; error names the key.
fn find_field(text: &str, key: &'static str) -> Result<String, CharterError> {
    for line in text.lines() {
        if let Some(value) = field_value(line, key) {
            return Ok(value.to_string());
        }
    }
    Err(CharterError::MissingField(key))
}

/// Parse one charter file: `id`, `question`, `run`, `pass`, `fail`, `sev`.
pub fn parse_charter(text: &str) -> Result<Charter, CharterError> {
    let id = find_field(text, "id")?;
    let question = find_field(text, "question")?;
    let run = find_field(text, "run")?;
    let pass_line = find_field(text, "pass")?;
    let fail_line = find_field(text, "fail")?;
    let sev_text = find_field(text, "sev")?;
    let sev = Severity::parse(&sev_text).ok_or(CharterError::BadSev(sev_text))?;
    Ok(Charter {
        id,
        question,
        run,
        pass_line,
        fail_line,
        sev,
    })
}

/// Score one log: passes when it contains the pass line, else FAIL.
pub fn evaluate(output: &str, charter: &Charter) -> CharterVerdict {
    if output.contains(charter.pass_line.as_str()) {
        CharterVerdict {
            pass: true,
            line: format!("PASS {}: {}", charter.id, charter.pass_line),
        }
    } else {
        CharterVerdict {
            pass: false,
            line: format!(
                "FAIL {}: {} (sev {})",
                charter.id,
                charter.fail_line,
                charter.sev.letter()
            ),
        }
    }
}

/// Blessed C1 charter source; compile-time so a missing file fails the build.
const C1_SRC: &str = include_str!("../charters/c1.toml");

/// Load the C1 charter through the production path every harness reads.
pub fn load_c1() -> Result<Charter, CharterError> {
    parse_charter(C1_SRC)
}

/// Run the C1 charter over one observed log through the production path.
pub fn run_c1(output: &str) -> Result<CharterVerdict, CharterError> {
    let charter = load_c1()?;
    Ok(evaluate(output, &charter))
}

/// Bout log of the scripted parry probe: what the tournament observed in
/// its first exchange, in the words C1 reads.
pub fn first_bout_log(parry_seen: bool) -> &'static str {
    if parry_seen {
        "round 1: parry landed in first bout"
    } else {
        "round 1: quiet bout"
    }
}

/// Blessed C2 charter source; compile-time so a missing file fails the build.
const C2_SRC: &str = include_str!("../charters/c2.toml");

/// Load the C2 charter through the production path every harness reads.
pub fn load_c2() -> Result<Charter, CharterError> {
    parse_charter(C2_SRC)
}

/// Run the C2 charter over one observed log through the production path.
pub fn run_c2(output: &str) -> Result<CharterVerdict, CharterError> {
    let charter = load_c2()?;
    Ok(evaluate(output, &charter))
}

/// C2 shake rule: a light hit shakes at most a quarter of a heavy hit.
/// Integer millimetres, so the same pair gives the same verdict twice.
/// A zero heavy refuses a pass (no heavy seen, nothing to compare).
pub fn light_within_quarter(light_mm: u32, heavy_mm: u32) -> bool {
    heavy_mm > 0 && light_mm.saturating_mul(4) <= heavy_mm
}

/// Shake probe log C2 reads: the pass or fail line sits inside the text,
/// so `run_c2` over this log prints the matching verdict line.
pub fn shake_log(light_mm: u32, heavy_mm: u32) -> String {
    let tail = if light_within_quarter(light_mm, heavy_mm) {
        "light shake at most quarter of heavy"
    } else {
        "light shake over quarter of heavy"
    };
    format!("shake light={light_mm}mm heavy={heavy_mm}mm: {tail}")
}

/// One charter run log: what ran, where it ran, why it matters, and the
/// conclusion. The conclusion carries the PASS/FAIL verdict line.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct CharterLog {
    pub what: String,
    pub where_run: String,
    pub why: String,
    pub conclusion: String,
}

impl CharterLog {
    /// Render the log as four lines: what, where, why, conclusion.
    pub fn render(&self) -> String {
        format!(
            "what: {}\nwhere: {}\nwhy: {}\nconclusion: {}\n",
            self.what, self.where_run, self.why, self.conclusion
        )
    }
}

/// Build the run log for one charter verdict: the question is the why,
/// the verdict line is the conclusion.
pub fn log_for(charter: &Charter, verdict: &CharterVerdict, where_run: &str) -> CharterLog {
    CharterLog {
        what: format!("{} run {}", charter.id, charter.run),
        where_run: where_run.to_string(),
        why: charter.question.clone(),
        conclusion: verdict.line.clone(),
    }
}

/// Charter entry point: run every blessed charter over its log, one
/// PASS or FAIL line each. Same logs give the same lines twice.
pub fn run_all(c1_log: &str, c2_log: &str) -> Result<Vec<CharterVerdict>, CharterError> {
    Ok(vec![run_c1(c1_log)?, run_c2(c2_log)?])
}

/// Print one PASS/FAIL line per verdict (the `charter` entry-point output).
pub fn print_lines(verdicts: &[CharterVerdict]) -> String {
    let mut out = String::new();
    for verdict in verdicts {
        out.push_str(&verdict.line);
        out.push('\n');
    }
    out
}

/// One bug packet filed from a failed charter: severity plus repro steps.
/// The sev letter rides the line so triage never guesses.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct BugPacket {
    pub charter_id: String,
    pub sev: Severity,
    pub repro: String,
}

impl BugPacket {
    /// File one line for the packet: id, sev letter, and repro.
    pub fn line(&self) -> String {
        format!(
            "BUG {} sev {}: {}",
            self.charter_id,
            self.sev.letter(),
            self.repro
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// C1 parses with its question and prints its pass line on a good log.
    #[test]
    fn charter_c1_parses_and_prints_pass() {
        let charter = load_c1().expect("c1 ships");
        assert_eq!(charter.id, "c1");
        assert_eq!(
            charter.question,
            "a bot with human noise lands a parry in its first bout"
        );
        let verdict = evaluate("round 1: parry landed in first bout", &charter);
        assert!(verdict.pass);
        assert_eq!(verdict.line, "PASS c1: parry landed in first bout");
        assert_eq!(
            verdict,
            evaluate("round 1: parry landed in first bout", &charter)
        );
    }

    /// A log without the pass line prints FAIL with the fail line and sev.
    #[test]
    fn charter_broken_expectation_prints_fail() {
        let charter = load_c1().expect("c1 ships");
        let verdict = evaluate("round 1: quiet bout", &charter);
        assert!(!verdict.pass);
        assert_eq!(verdict.line, "FAIL c1: no parry in first bout (sev B)");
    }

    /// The probe log feeds C1 both ways: a seen parry passes, a miss fails.
    #[test]
    fn charter_first_bout_log_drives_both_verdicts() {
        let seen = run_c1(first_bout_log(true)).expect("c1 ships");
        assert!(seen.pass);
        let missed = run_c1(first_bout_log(false)).expect("c1 ships");
        assert!(!missed.pass);
        assert!(missed.line.starts_with("FAIL c1:"));
    }

    /// A charter without its question is refused with the key name.
    #[test]
    fn charter_missing_field_refuses() {
        let text = "id = \"c9\"\nrun = \"x\"\npass = \"p\"\nfail = \"f\"\nsev = \"B\"\n";
        assert_eq!(
            parse_charter(text),
            Err(CharterError::MissingField("question"))
        );
        let bad_sev =
            "id = \"c9\"\nquestion = \"q\"\nrun = \"x\"\npass = \"p\"\nfail = \"f\"\nsev = \"Z\"\n";
        assert!(matches!(
            parse_charter(bad_sev),
            Err(CharterError::BadSev(_))
        ));
    }

    /// C2 parses with its question and prints its pass line on a good shake.
    #[test]
    fn charter_c2_parses_and_prints_pass() {
        let charter = load_c2().expect("c2 ships");
        assert_eq!(charter.id, "c2");
        assert_eq!(
            charter.question,
            "a light hit shakes the camera at most a quarter of a heavy"
        );
        let verdict = run_c2(&shake_log(8, 40)).expect("c2 ships");
        assert!(verdict.pass);
        assert_eq!(verdict.line, format!("PASS c2: {}", charter.pass_line));
        let edge = run_c2(&shake_log(10, 40)).expect("c2 ships");
        assert!(edge.pass, "exactly a quarter still passes");
        assert!(!run_c2(&shake_log(8, 0)).expect("c2 ships").pass);
    }

    /// A shake over a quarter prints FAIL with the fail line and sev.
    #[test]
    fn charter_c2_broken_expectation_prints_fail() {
        let charter = load_c2().expect("c2 ships");
        let verdict = run_c2(&shake_log(15, 40)).expect("c2 ships");
        assert!(!verdict.pass);
        assert_eq!(
            verdict.line,
            format!("FAIL c2: {} (sev {})", charter.fail_line, charter.sev.letter())
        );
        assert_eq!(verdict, evaluate(&shake_log(15, 40), &charter));
    }

    /// The entry point prints one line per charter; the log carries what,
    /// where, why and conclusion; bug packets carry sev A|B|C.
    #[test]
    fn charter_entry_point_prints_both_lines_with_log_and_sev() {
        let c1 = run_c1(first_bout_log(true)).expect("c1 ships");
        let c2 = run_c2(&shake_log(8, 40)).expect("c2 ships");
        let lines = print_lines(&run_all(first_bout_log(true), &shake_log(8, 40)).expect("both ship"));
        assert!(lines.contains(&c1.line));
        assert!(lines.contains(&c2.line));
        assert_eq!(lines.lines().count(), 2);
        let charter = load_c2().expect("c2 ships");
        let log = log_for(&charter, &c2, "headless bout light-vs-heavy");
        let text = log.render();
        assert!(text.contains("what: c2 run camera_shake_light_vs_heavy"));
        assert!(text.contains("where: headless bout light-vs-heavy"));
        assert!(text.contains("why: a light hit shakes the camera"));
        assert!(text.contains(&c2.line));
        for (letter, sev) in [("A", Severity::A), ("B", Severity::B), ("C", Severity::C)] {
            let packet = BugPacket {
                charter_id: "c2".to_string(),
                sev,
                repro: "light 15mm over quarter of heavy 40mm".to_string(),
            };
            assert!(packet.line().contains(&format!("sev {letter}")));
            assert_eq!(sev.letter(), letter.chars().next().expect("letter"));
        }
    }
}
