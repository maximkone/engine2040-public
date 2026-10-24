//! AG1 agent scorecard: pass rate, cost per row and tokens per task for each
//! suite, by model tier, written as one dated markdown card; a drop against
//! the last card is a red line.
//!
//! One command writes the card and prints it:
//! `SCORECARD_DATE=2026-10-18 cargo test -p engine-playtest --lib scorecard_command -- --ignored --nocapture`.
//! The card goes to `target/reports/scorecard-<date>.md`; the last card is the
//! newest earlier file in that folder. A plain cargo line with no
//! `SCORECARD_DATE` dates the card `1970-01-01`: the scorecard CI leg then needs
//! no environment, and the date only names the heading and the file.
//!
//! Suites today: EV1 (the 75-row eval), the 30-task public-benchmark mirrors
//! (HS29-HS32) and the 100-task spatial gold eval. BreakBench (BX25) is not
//! built yet and prints `unbuilt`, never a made-up rate. Tiers: the backend
//! label picks the tier (`mock` is `replay`, `local` is `free`, `remote` is
//! `strong`); the proposals of all three replay recorded tapes through the
//! real provider contract, so today a free or strong row is the contract path
//! of that tier, not a live model. Tokens per task come from live recordings
//! only; a replayed suite prints `unknown`, never zero.
//!
//! Determinism: the same results give the same card bytes; no wall clock is
//! read (the caller names the date).

use super::*;

/// Which model tier served a result.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Tier {
    /// A free model.
    Free,
    /// A strong (paid) model.
    Strong,
    /// A recorded tape replayed, no model.
    Replay,
}

impl Tier {
    /// Card spelling.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Free => "free",
            Self::Strong => "strong",
            Self::Replay => "replay",
        }
    }

    fn parse(text: &str) -> Option<Self> {
        [Self::Free, Self::Strong, Self::Replay]
            .into_iter()
            .find(|tier| tier.name() == text)
    }

    /// Tier a backend label is scored as.
    pub const fn of_backend(backend: Backend) -> Self {
        match backend {
            Backend::Mock => Self::Replay,
            Backend::Local => Self::Free,
            Backend::Remote => Self::Strong,
        }
    }
}

/// One card line: a suite at a tier with its pass count, cost and token spend.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct CardLine {
    /// Suite name.
    pub suite: String,
    /// Tier the results were served at.
    pub tier: Tier,
    /// Tasks passed.
    pub passed: u32,
    /// Tasks run; `0` means the suite is unbuilt.
    pub total: u32,
    /// Cost of one row in minor units; `None` when no row reports a cost or
    /// any row's cost is unknown, which is never counted as zero.
    pub cost_per_row: Option<u64>,
    /// Tokens per task; `None` is unknown, never zero.
    pub tokens_per_task: Option<u64>,
}

impl CardLine {
    /// Pass rate in permille; `None` for an unbuilt suite.
    pub fn rate_permille(&self) -> Option<u32> {
        (self.total > 0).then(|| self.passed * 1000 / self.total)
    }
}

/// A line that dropped against the last card.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Red {
    /// Suite name.
    pub suite: String,
    /// Tier.
    pub tier: Tier,
    /// Last card's rate in permille.
    pub was_permille: u32,
    /// This card's rate in permille; `None` when the line lost its results.
    pub now_permille: Option<u32>,
}

/// Why a card could not be built, read or written.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum CardError {
    /// The card text has no `# Agent scorecard <date>` heading.
    Heading,
    /// A table row could not be read; the 1-based line.
    Row(usize),
    /// A suite refused to run.
    Suite(String),
    /// A file could not be read or written.
    Io(String),
}

impl fmt::Display for CardError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Heading => write!(f, "scorecard: no `# Agent scorecard <date>` heading"),
            Self::Row(line) => write!(f, "scorecard: table row at line {line} is unreadable"),
            Self::Suite(why) => write!(f, "scorecard: a suite refused to run: {why}"),
            Self::Io(why) => write!(f, "scorecard: io refused: {why}"),
        }
    }
}

impl std::error::Error for CardError {}

/// One day's card.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Scorecard {
    /// Date the caller named, `YYYY-MM-DD`.
    pub date: String,
    /// Lines in suite then tier order.
    pub lines: Vec<CardLine>,
}

fn percent(permille: u32) -> String {
    format!("{}.{}%", permille / 10, permille % 10)
}

impl Scorecard {
    /// Lines whose pass rate dropped against `last`, or whose results
    /// vanished. A line `last` did not have is new, never red.
    pub fn reds(&self, last: &Scorecard) -> Vec<Red> {
        last.lines
            .iter()
            .filter_map(|old| {
                let was = old.rate_permille()?;
                let now = self
                    .lines
                    .iter()
                    .find(|line| line.suite == old.suite && line.tier == old.tier)
                    .and_then(CardLine::rate_permille);
                now.is_none_or(|rate| rate < was).then(|| Red {
                    suite: old.suite.clone(),
                    tier: old.tier,
                    was_permille: was,
                    now_permille: now,
                })
            })
            .collect()
    }

    /// The card as markdown, with a `vs last card` column when `last` is given.
    pub fn to_markdown(&self, last: Option<&Scorecard>) -> String {
        let reds = last.map(|last| self.reds(last)).unwrap_or_default();
        let mut out = format!("# Agent scorecard {}\n\n", self.date);
        out.push_str(
            "| suite | tier | passed | total | pass rate | cost per row | tokens per task | vs last card |\n",
        );
        out.push_str("|---|---|---|---|---|---|---|---|\n");
        for line in &self.lines {
            let rate = line
                .rate_permille()
                .map_or_else(|| "unbuilt".to_owned(), percent);
            let cost = line
                .cost_per_row
                .map_or_else(|| "unknown".to_owned(), |cost| cost.to_string());
            let tokens = line
                .tokens_per_task
                .map_or_else(|| "unknown".to_owned(), |tokens| tokens.to_string());
            let against = match last {
                None => "-".to_owned(),
                Some(last) => self.against(line, last, &reds),
            };
            out.push_str(&format!(
                "| {} | {} | {} | {} | {} | {} | {} | {} |\n",
                line.suite,
                line.tier.name(),
                line.passed,
                line.total,
                rate,
                cost,
                tokens,
                against,
            ));
        }
        out.push_str(&format!("\nRED lines: {}\n", reds.len()));
        for red in &reds {
            let now = red.now_permille.map_or_else(|| "lost".to_owned(), percent);
            out.push_str(&format!(
                "RED {} {}: {} -> {}\n",
                red.suite,
                red.tier.name(),
                percent(red.was_permille),
                now,
            ));
        }
        out
    }

    /// The last-card cell of one line.
    fn against(&self, line: &CardLine, last: &Scorecard, reds: &[Red]) -> String {
        if reds
            .iter()
            .any(|red| red.suite == line.suite && red.tier == line.tier)
        {
            return "RED".to_owned();
        }
        let before = last
            .lines
            .iter()
            .find(|old| old.suite == line.suite && old.tier == line.tier)
            .and_then(CardLine::rate_permille);
        match (before, line.rate_permille()) {
            (None, _) => "new".to_owned(),
            (Some(was), Some(now)) if now > was => format!("up {}", percent(now - was)),
            _ => "same".to_owned(),
        }
    }

    /// Read a card back from its markdown (the first six columns).
    pub fn from_markdown(text: &str) -> Result<Self, CardError> {
        let date = text
            .lines()
            .find_map(|line| line.strip_prefix("# Agent scorecard "))
            .ok_or(CardError::Heading)?
            .trim()
            .to_owned();
        let mut lines = Vec::new();
        let mut in_table = false;
        for (number, row) in text.lines().enumerate() {
            if row.starts_with("|---") {
                in_table = true;
            } else if in_table && row.starts_with('|') {
                lines.push(read_row(row).ok_or(CardError::Row(number + 1))?);
            } else if in_table {
                break;
            }
        }
        Ok(Self { date, lines })
    }
}

/// One table row back into a line; `None` when a cell does not read. An old
/// card has 7 cells and no cost column; a new one has 8.
fn read_row(row: &str) -> Option<CardLine> {
    let cells: Vec<&str> = row.trim_matches('|').split('|').map(str::trim).collect();
    if cells.len() != 7 && cells.len() != 8 {
        return None;
    }
    let tokens_at = cells.len() - 2;
    let cost_at = cells.len() - 3;
    Some(CardLine {
        suite: cells[0].to_owned(),
        tier: Tier::parse(cells[1])?,
        passed: cells[2].parse().ok()?,
        total: cells[3].parse().ok()?,
        cost_per_row: if cells.len() == 8 {
            optional_count(cells[cost_at])?
        } else {
            None
        },
        tokens_per_task: optional_count(cells[tokens_at])?,
    })
}

/// `unknown` or a plain number; `None` when the cell is neither.
fn optional_count(cell: &str) -> Option<Option<u64>> {
    if cell == "unknown" {
        Some(None)
    } else {
        Some(Some(cell.parse().ok()?))
    }
}

fn line(suite: &str, tier: Tier, passed: usize, total: usize) -> CardLine {
    CardLine {
        suite: suite.to_owned(),
        tier,
        passed: u32::try_from(passed).unwrap_or(u32::MAX),
        total: u32::try_from(total).unwrap_or(u32::MAX),
        cost_per_row: None,
        tokens_per_task: None,
    }
}

/// Cost of one row in minor units: the mean of the rows that report a cost.
/// `None` when there are no rows or any row's cost is unknown, so an unknown
/// is never counted as zero.
fn cost_per_row(rows: &[&EvalRow]) -> Option<u64> {
    if rows.is_empty() {
        return None;
    }
    let mut sum = 0u64;
    for row in rows {
        match row.cost {
            Cost::Units(units) => sum = sum.saturating_add(units),
            Cost::Unknown => return None,
        }
    }
    Some(sum / u64::try_from(rows.len()).unwrap_or(1))
}

/// Run the suites and collect their lines: EV1, the mirrors and the spatial
/// gold eval, plus the BreakBench line that says it is unbuilt.
pub fn collect() -> Result<Vec<CardLine>, CardError> {
    let mut lines = Vec::new();
    let ev1 = run_rows();
    let mirror = crate::mirror::run_mirror();
    for backend in BACKENDS {
        let label = backend_label(backend);
        let tier = Tier::of_backend(backend);
        let rows: Vec<&EvalRow> = ev1.iter().filter(|row| row.backend == label).collect();
        let green = rows
            .iter()
            .filter(|row| EvalReport::row_is_green(row))
            .count();
        let cost = cost_per_row(&rows);
        let mut ev1_line = line("EV1", tier, green, rows.len());
        ev1_line.cost_per_row = cost;
        lines.push(ev1_line);
        lines.push(line("mirror", tier, mirror.solved(label), mirror.tasks()));
    }
    let spatial = crate::spatial_eval::proof_gate()
        .map_err(|error| CardError::Suite(format!("spatial_eval: {error}")))?;
    lines.push(line(
        "spatial_eval",
        Tier::Replay,
        spatial.passed,
        spatial.total,
    ));
    lines.push(line("BreakBench", Tier::Replay, 0, 0));
    lines.sort_by(|a, b| (&a.suite, a.tier).cmp(&(&b.suite, b.tier)));
    Ok(lines)
}

fn card_name(date: &str) -> String {
    format!("scorecard-{date}.md")
}

/// The newest card in `dir` dated before `date`, if any.
fn last_card(dir: &Path, date: &str) -> Result<Option<Scorecard>, CardError> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Ok(None);
    };
    let mine = card_name(date);
    let newest = entries
        .filter_map(|entry| entry.ok()?.file_name().into_string().ok())
        .filter(|name| name.starts_with("scorecard-") && name.ends_with(".md") && *name < mine)
        .max();
    let Some(name) = newest else {
        return Ok(None);
    };
    let text = std::fs::read_to_string(dir.join(name))
        .map_err(|error| CardError::Io(error.to_string()))?;
    Scorecard::from_markdown(&text).map(Some)
}

/// The one command: collect the suites, compare with the last card in `dir`,
/// write `scorecard-<date>.md` and return the card, its reds and its path.
pub fn run_scorecard(dir: &Path, date: &str) -> Result<(Scorecard, Vec<Red>, PathBuf), CardError> {
    let card = Scorecard {
        date: date.to_owned(),
        lines: collect()?,
    };
    let last = last_card(dir, date)?;
    std::fs::create_dir_all(dir).map_err(|error| CardError::Io(error.to_string()))?;
    let path = dir.join(card_name(date));
    std::fs::write(&path, card.to_markdown(last.as_ref()))
        .map_err(|error| CardError::Io(error.to_string()))?;
    let reds = last.map(|last| card.reds(&last)).unwrap_or_default();
    Ok((card, reds, path))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(ev1_free: usize) -> Scorecard {
        Scorecard {
            date: "2026-10-18".to_owned(),
            lines: vec![
                line("EV1", Tier::Free, ev1_free, 25),
                CardLine {
                    tokens_per_task: Some(1200),
                    ..line("EV1", Tier::Strong, 25, 25)
                },
                line("BreakBench", Tier::Replay, 0, 0),
            ],
        }
    }

    #[test]
    fn the_scorecard_prints_the_cost_of_one_row() {
        let mut card = fixture(25);
        card.lines[0].cost_per_row = Some(7);
        let text = card.to_markdown(None);
        assert!(
            text.contains("| EV1 | free | 25 | 25 | 100.0% | 7 | unknown | - |"),
            "{text}"
        );
        let back = Scorecard::from_markdown(&text).expect("a card reads back");
        assert_eq!(back.lines[0].cost_per_row, Some(7));
        assert_eq!(back.lines[1].cost_per_row, None, "no cost stays unknown");
    }

    #[test]
    fn cost_per_row_never_counts_an_unknown_as_zero() {
        let a = row_with_cost("local", Cost::Units(4));
        let b = row_with_cost("local", Cost::Units(6));
        assert_eq!(cost_per_row(&[&a, &b]), Some(5), "mean of the known costs");
        let unknown = row_with_cost("mock", Cost::Unknown);
        assert_eq!(
            cost_per_row(&[&a, &unknown]),
            None,
            "one unknown makes the line unknown, never zero"
        );
        assert_eq!(cost_per_row(&[]), None, "an unbuilt suite has no cost");
    }

    /// One report row carrying only what the cost column reads.
    fn row_with_cost(backend: &'static str, cost: Cost) -> EvalRow {
        EvalRow {
            task: "ev1-place",
            flow: "place",
            backend,
            completed: true,
            rejected: false,
            latency_ms: 1,
            cost,
            privacy_failure: false,
            detail: String::new(),
        }
    }

    #[test]
    fn scorecard_table_is_the_same_twice_on_fixture_results() {
        let card = fixture(24);
        let text = card.to_markdown(None);
        assert_eq!(text, fixture(24).to_markdown(None), "same table x2");
        let want = "# Agent scorecard 2026-10-18\n\n\
| suite | tier | passed | total | pass rate | cost per row | tokens per task | vs last card |\n\
|---|---|---|---|---|---|---|---|\n\
| EV1 | free | 24 | 25 | 96.0% | unknown | unknown | - |\n\
| EV1 | strong | 25 | 25 | 100.0% | unknown | 1200 | - |\n\
| BreakBench | replay | 0 | 0 | unbuilt | unknown | unknown | - |\n\
\nRED lines: 0\n";
        assert_eq!(text, want);
    }

    #[test]
    fn scorecard_lower_pass_rate_prints_red() {
        let last = fixture(25);
        let text = fixture(24).to_markdown(Some(&last));
        assert!(
            text.contains("| EV1 | free | 24 | 25 | 96.0% | unknown | unknown | RED |"),
            "{text}"
        );
        assert!(text.contains("RED lines: 1\nRED EV1 free: 100.0% -> 96.0%\n"));
        // The other lines did not drop, so they are not red.
        assert!(text.contains("| EV1 | strong | 25 | 25 | 100.0% | unknown | 1200 | same |"));
        // Negative control: the same rate is not red, a better one is up.
        assert!(fixture(25)
            .to_markdown(Some(&last))
            .contains("RED lines: 0\n"));
        let worse = fixture(20);
        let better = fixture(25);
        assert!(better.to_markdown(Some(&worse)).contains("up 20.0%"));
    }

    #[test]
    fn scorecard_lost_lines_are_red_and_new_lines_are_new() {
        let last = fixture(25);
        let mut now = fixture(25);
        now.lines.remove(1);
        now.lines.push(line("mirror", Tier::Replay, 30, 30));
        let reds = now.reds(&last);
        assert_eq!(reds.len(), 1);
        assert_eq!(
            (reds[0].suite.as_str(), reds[0].now_permille),
            ("EV1", None)
        );
        let text = now.to_markdown(Some(&last));
        assert!(text.contains("| mirror | replay | 30 | 30 | 100.0% | unknown | unknown | new |"));
        assert!(text.contains("RED EV1 strong: 100.0% -> lost\n"));
    }

    #[test]
    fn scorecard_reads_back_what_it_wrote() {
        let card = fixture(24);
        let back = Scorecard::from_markdown(&card.to_markdown(Some(&fixture(25))));
        assert_eq!(back, Ok(card));
        assert_eq!(
            Scorecard::from_markdown("no heading"),
            Err(CardError::Heading)
        );
        let broken = "# Agent scorecard 2026-10-18\n\n|---|\n| EV1 | free | x |\n";
        assert_eq!(Scorecard::from_markdown(broken), Err(CardError::Row(4)));
    }

    #[test]
    fn scorecard_tiers_follow_the_backend_labels() {
        assert_eq!(Tier::of_backend(Backend::Mock), Tier::Replay);
        assert_eq!(Tier::of_backend(Backend::Local), Tier::Free);
        assert_eq!(Tier::of_backend(Backend::Remote), Tier::Strong);
    }

    #[test]
    fn scorecard_collects_every_suite_green_and_unbuilt_breakbench() {
        let lines = collect().expect("suites run");
        let of = |suite: &str, tier: Tier| {
            lines
                .iter()
                .find(|l| l.suite == suite && l.tier == tier)
                .unwrap_or_else(|| panic!("{suite} {tier:?} on the card"))
        };
        for tier in [Tier::Free, Tier::Strong, Tier::Replay] {
            let ev1 = of("EV1", tier);
            assert_eq!((ev1.passed, ev1.total), (25, 25), "EV1 {tier:?}");
            let mirror = of("mirror", tier);
            assert_eq!(mirror.total, 30);
            assert_eq!(mirror.passed, 30);
        }
        assert_eq!(of("spatial_eval", Tier::Replay).total, 100);
        assert_eq!(of("BreakBench", Tier::Replay).rate_permille(), None);
        assert_eq!(lines, collect().expect("suites run x2"));
    }

    #[test]
    fn scorecard_command_writes_the_card_and_compares_with_the_last() {
        let dir = std::env::temp_dir().join("e2040-scorecard-test");
        let _ = std::fs::remove_dir_all(&dir);
        // The last card claims a suite that is gone: one red.
        let mut old = Scorecard {
            date: "2026-10-17".to_owned(),
            lines: collect().expect("suites run"),
        };
        old.lines.push(line("ghost_suite", Tier::Replay, 5, 5));
        std::fs::create_dir_all(&dir).expect("dir");
        std::fs::write(dir.join(card_name("2026-10-17")), old.to_markdown(None)).expect("old");

        let (card, reds, path) = run_scorecard(&dir, "2026-10-18").expect("command runs");
        assert_eq!(reds.len(), 1);
        assert_eq!(reds[0].suite, "ghost_suite");
        let first = std::fs::read_to_string(&path).expect("card written");
        assert!(first.contains("RED ghost_suite replay: 100.0% -> lost"));
        assert_eq!(first, card.to_markdown(Some(&old)));
        // Same day again: the same card bytes.
        run_scorecard(&dir, "2026-10-18").expect("command runs x2");
        assert_eq!(first, std::fs::read_to_string(&path).expect("card x2"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    #[ignore = "the operator command: writes target/reports/scorecard-<date>.md, set SCORECARD_DATE (default 1970-01-01)"]
    fn scorecard_command() {
        // CI runs this as one plain cargo line, so an unset date falls back to a
        // fixed one; the date never reads the clock either way.
        let date = std::env::var("SCORECARD_DATE").unwrap_or_else(|_| "1970-01-01".to_owned());
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/reports");
        let (card, reds, path) = run_scorecard(&dir, &date).expect("command runs");
        println!("{}", std::fs::read_to_string(&path).expect("card"));
        let rows = card.lines.len();
        let green: usize = card
            .lines
            .iter()
            .filter(|line| line.rate_permille() == Some(1000))
            .count();
        println!(
            "scorecard rows: {rows}, at 100%: {green}, red: {}",
            reds.len()
        );
        assert_eq!(card.date, date);
        assert!(reds.is_empty(), "red lines against the last card: {reds:?}");
    }
}
