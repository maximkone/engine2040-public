//! Differential receipt comparison: two builds, one curated scenario list.
//!
//! Owner: engine-proof. Consumers: the ship differential job and the
//! `differential` binary. Data owner: this module owns the receipt log
//! format, the volatile key list and the scenario list format; the sim owns
//! nothing here.
//! Units: one step per scenario, one line per receipt line, ticks never
//! enter. Determinism scope: the same two logs give the same report text on
//! every machine, and the same scenario list gives the same log.
//!
//! A receipt log is what a build printed while it played the curated
//! scenarios. The comparison replaces the value of every volatile key (a
//! measured time, a path, a run id) and every host label with a marker, so a
//! build that only moved its clock, or that ran on another machine, still
//! matches. A seeded value keeps its digits, so a behaviour change shows up
//! as a divergence and is never normalized away.

use std::collections::BTreeSet;
use std::fmt;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The receipt log format: the first line names the format and the tier.
pub const LOG_FORMAT: &str = "differential receipt log v1";

/// The scenario list format: the first line names the format.
pub const SCENARIO_FORMAT: &str = "differential scenario list v1";

/// The keys whose value is measured or bound to the environment. The
/// comparison replaces the value with `<key>`. Every other value, a hash or a
/// seed among them, is compared as it stands.
pub const VOLATILE_KEYS: &[&str] = &[
    "build_id",
    "build_ms",
    "delay_max",
    "delay_p50",
    "dir",
    "elapsed_ms",
    "file",
    "finished_at",
    "fps",
    "host",
    "lead_max",
    "micros",
    "ms",
    "p50",
    "p95",
    "p99",
    "path",
    "pid",
    "port",
    "rebuild_ms",
    "rtt_max_ms",
    "rtt_ms",
    "rtt_p50",
    "run_id",
    "started_at",
    "timestamp",
    "us",
    "wall_ms",
];

/// The operating system names a host label can start with. A receipt line
/// that names the machine it ran on is compared as `<host>`, so two builds on
/// two machines still match.
pub const HOST_OS_NAMES: &[&str] = &[
    "android", "freebsd", "ios", "linux", "macos", "netbsd", "openbsd", "wasi", "windows",
];

/// A rule set: the keys whose value the comparison replaces.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Normalizer {
    keys: Vec<&'static str>,
}

impl Normalizer {
    /// A rule set with the given volatile keys.
    pub fn new(keys: &[&'static str]) -> Self {
        Self {
            keys: keys.to_vec(),
        }
    }

    /// The default rule set: [`VOLATILE_KEYS`].
    pub fn volatile() -> Self {
        Self::new(VOLATILE_KEYS)
    }

    /// One more volatile key.
    pub fn with(mut self, key: &'static str) -> Self {
        self.keys.push(key);
        self
    }

    /// The line with the value of every volatile key and every host label
    /// replaced by a marker.
    pub fn normalize_line(&self, line: &str) -> String {
        let mut out = String::with_capacity(line.len());
        for token in line.split_whitespace() {
            if !out.is_empty() {
                out.push(' ');
            }
            match token.split_once('=') {
                Some((key, _)) if self.is_volatile(key) => {
                    out.push_str(key);
                    out.push_str("=<");
                    out.push_str(key);
                    out.push('>');
                }
                _ if is_host_token(token) => out.push_str("<host>"),
                _ => out.push_str(token),
            }
        }
        out
    }

    /// True when the key carries a measured or environment value.
    fn is_volatile(&self, key: &str) -> bool {
        self.keys.contains(&key)
    }
}

/// True when the token names the machine: an operating system name, a dash
/// and an architecture.
fn is_host_token(token: &str) -> bool {
    if token.contains('=') {
        return false;
    }
    let Some((os, _)) = token.split_once('-') else {
        return false;
    };
    HOST_OS_NAMES.contains(&os)
}

/// One scenario's receipt lines.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Step {
    /// The scenario name; the step name in the log.
    pub name: String,
    /// False when the step is out of scope: the comparison skips it.
    pub in_scope: bool,
    /// The receipt lines, in the order the build printed them.
    pub lines: Vec<String>,
}

/// A receipt log: the tier it was collected for and its steps.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Log {
    /// The tier the scenarios ran for.
    pub tier: String,
    /// The steps, in the order the scenarios ran.
    pub steps: Vec<Step>,
}

/// Why a receipt log or a scenario list could not be used.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DiffError {
    /// The text does not follow the format.
    Format(String),
    /// The two logs were collected for different tiers.
    Tier(String),
    /// A file could not be read or written, or a scenario could not run.
    Io(String),
}

impl fmt::Display for DiffError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Format(why) => write!(f, "format: {why}"),
            Self::Tier(why) => write!(f, "tier: {why}"),
            Self::Io(why) => write!(f, "io: {why}"),
        }
    }
}

impl std::error::Error for DiffError {}

/// How a step or a line diverged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// The reference has the step, the candidate does not.
    MissingStep,
    /// The candidate has the step, the reference does not.
    ExtraStep,
    /// The line reads differently.
    ChangedLine,
    /// The reference has the line, the candidate does not.
    MissingLine,
    /// The candidate has the line, the reference does not.
    ExtraLine,
}

impl fmt::Display for Kind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::MissingStep => "missing-step",
            Self::ExtraStep => "extra-step",
            Self::ChangedLine => "changed-line",
            Self::MissingLine => "missing-line",
            Self::ExtraLine => "extra-line",
        };
        f.write_str(name)
    }
}

/// One divergence between two receipt logs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Divergence {
    /// The step the divergence sits in.
    pub step: String,
    /// The line inside the step, counted from one; zero for a whole step.
    pub line: usize,
    /// How it diverged.
    pub kind: Kind,
    /// The reference line, as it was printed.
    pub reference: String,
    /// The candidate line, as it was printed.
    pub candidate: String,
}

/// The verdict over two receipt logs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Report {
    /// The tier both logs were collected for.
    pub tier: String,
    /// The steps the comparison walked.
    pub steps: usize,
    /// The divergences, in reference order.
    pub divergences: Vec<Divergence>,
}

impl Report {
    /// True when every in-scope step matched.
    pub fn is_clean(&self) -> bool {
        self.divergences.is_empty()
    }

    /// The exit code the tool returns: 0 clean, 1 one divergence or more.
    pub fn exit_code(&self) -> i32 {
        i32::from(!self.is_clean())
    }

    /// The report as text: one headline and one block per divergence.
    pub fn text(&self) -> String {
        let mut out = String::new();
        if self.is_clean() {
            out.push_str(&format!(
                "DIFFERENTIAL clean tier={} steps={}\n",
                self.tier, self.steps
            ));
            return out;
        }
        out.push_str(&format!(
            "DIFFERENTIAL diverged tier={} steps={} divergences={}\n",
            self.tier,
            self.steps,
            self.divergences.len()
        ));
        for divergence in &self.divergences {
            out.push_str(&format!(
                "DIVERGENCE step={} line={} kind={}\n",
                divergence.step, divergence.line, divergence.kind
            ));
            out.push_str(&format!("  reference: {}\n", divergence.reference));
            out.push_str(&format!("  candidate: {}\n", divergence.candidate));
        }
        out
    }
}

/// The header line of a receipt log.
fn log_header(tier: &str) -> String {
    format!("{LOG_FORMAT} tier={tier}")
}

/// Reads a receipt log.
pub fn parse_log(text: &str) -> Result<Log, DiffError> {
    let mut tier = String::new();
    let mut steps: Vec<Step> = Vec::new();
    for (number, raw) in text.lines().enumerate() {
        let line = raw.trim_end_matches('\r');
        if line.trim().is_empty() || line.starts_with('#') {
            continue;
        }
        if steps.is_empty() && tier.is_empty() {
            tier = parse_log_header(line, number)?;
            continue;
        }
        if let Some(rest) = line.strip_prefix("STEP ") {
            steps.push(parse_step_header(rest, number)?);
            continue;
        }
        let Some(step) = steps.last_mut() else {
            return Err(DiffError::Format(format!(
                "line {} holds a receipt line before any step",
                number + 1
            )));
        };
        step.lines.push(line.to_string());
    }
    if tier.is_empty() {
        return Err(DiffError::Format("the log holds no header".into()));
    }
    if steps.is_empty() {
        return Err(DiffError::Format("the log holds no step".into()));
    }
    Ok(Log { tier, steps })
}

/// Reads the header line of a receipt log.
fn parse_log_header(line: &str, number: usize) -> Result<String, DiffError> {
    let rest = line.strip_prefix(LOG_FORMAT).ok_or_else(|| {
        DiffError::Format(format!("line {} must start with {LOG_FORMAT}", number + 1))
    })?;
    for field in rest.split_whitespace() {
        if let Some(tier) = field.strip_prefix("tier=") {
            if !tier.is_empty() {
                return Ok(tier.to_string());
            }
        }
    }
    Err(DiffError::Format(format!(
        "line {} must name the tier, for example {LOG_FORMAT} tier=fast",
        number + 1
    )))
}

/// Reads the header line of one step.
fn parse_step_header(rest: &str, number: usize) -> Result<Step, DiffError> {
    let mut in_scope = true;
    let mut name = String::new();
    for field in rest.split_whitespace() {
        if let Some(scope) = field.strip_prefix("scope=") {
            in_scope = scope != "out";
            continue;
        }
        if name.is_empty() {
            name = field.to_string();
        }
    }
    if name.is_empty() {
        return Err(DiffError::Format(format!(
            "line {} names no step",
            number + 1
        )));
    }
    Ok(Step {
        name,
        in_scope,
        lines: Vec::new(),
    })
}

/// Writes a receipt log: the header line, then one block per step.
pub fn write_log(tier: &str, steps: &[Step]) -> String {
    let mut out = String::new();
    out.push_str(&log_header(tier));
    out.push('\n');
    for step in steps {
        out.push_str("STEP ");
        out.push_str(&step.name);
        if !step.in_scope {
            out.push_str(" scope=out");
        }
        out.push('\n');
        for line in &step.lines {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

/// Compares two receipt logs step by step.
pub fn diff(reference: &str, candidate: &str) -> Result<Report, DiffError> {
    let reference = parse_log(reference)?;
    let candidate = parse_log(candidate)?;
    if reference.tier != candidate.tier {
        return Err(DiffError::Tier(format!(
            "the reference is tier {} and the candidate tier {}",
            reference.tier, candidate.tier
        )));
    }
    Ok(compare(&reference, &candidate, &Normalizer::volatile()))
}

/// Compares two parsed receipt logs with the given rule set.
pub fn compare(reference: &Log, candidate: &Log, normalizer: &Normalizer) -> Report {
    let mut divergences = Vec::new();
    let reference_names: BTreeSet<&str> = reference
        .steps
        .iter()
        .map(|step| step.name.as_str())
        .collect();
    let mut compared = 0;
    for step in &reference.steps {
        if !step.in_scope {
            continue;
        }
        let Some(other) = candidate.steps.iter().find(|other| other.name == step.name) else {
            compared += 1;
            divergences.push(Divergence {
                step: step.name.clone(),
                line: 0,
                kind: Kind::MissingStep,
                reference: step.lines.join("\n"),
                candidate: String::new(),
            });
            continue;
        };
        if !other.in_scope {
            // The scope decides: a step out of scope on either side is not
            // compared.
            continue;
        }
        compared += 1;
        compare_lines(step, other, normalizer, &mut divergences);
    }
    for step in &candidate.steps {
        if !step.in_scope || reference_names.contains(step.name.as_str()) {
            continue;
        }
        divergences.push(Divergence {
            step: step.name.clone(),
            line: 0,
            kind: Kind::ExtraStep,
            reference: String::new(),
            candidate: step.lines.join("\n"),
        });
    }
    Report {
        tier: reference.tier.clone(),
        steps: compared,
        divergences,
    }
}

/// Compares the lines of two steps, line by line.
fn compare_lines(
    reference: &Step,
    candidate: &Step,
    normalizer: &Normalizer,
    out: &mut Vec<Divergence>,
) {
    let count = reference.lines.len().max(candidate.lines.len());
    for index in 0..count {
        let left = reference.lines.get(index);
        let right = candidate.lines.get(index);
        let kind = match (left, right) {
            (Some(left), Some(right)) => {
                if normalizer.normalize_line(left) == normalizer.normalize_line(right) {
                    continue;
                }
                Kind::ChangedLine
            }
            (Some(_), None) => Kind::MissingLine,
            (None, Some(_)) => Kind::ExtraLine,
            (None, None) => continue,
        };
        out.push(Divergence {
            step: reference.name.clone(),
            line: index + 1,
            kind,
            reference: left.cloned().unwrap_or_default(),
            candidate: right.cloned().unwrap_or_default(),
        });
    }
}

/// One scenario: the binary to run and the arguments it takes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Scenario {
    /// The scenario name; the step name in the receipt log.
    pub name: String,
    /// The binary to run: a file name under the build directory, or a path.
    pub bin: String,
    /// The arguments the binary takes.
    pub args: Vec<String>,
}

/// The curated scenario list, tier by tier.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ScenarioList {
    tiers: Vec<(String, Vec<Scenario>)>,
}

impl ScenarioList {
    /// A list with the given tiers, in order.
    pub fn new(tiers: Vec<(String, Vec<Scenario>)>) -> Self {
        Self { tiers }
    }

    /// The scenarios of one tier.
    pub fn tier(&self, name: &str) -> Option<&[Scenario]> {
        self.tiers
            .iter()
            .find(|(tier, _)| tier == name)
            .map(|(_, scenarios)| scenarios.as_slice())
    }

    /// The tier names, in order.
    pub fn tier_names(&self) -> Vec<&str> {
        self.tiers.iter().map(|(tier, _)| tier.as_str()).collect()
    }

    /// Reads a scenario list.
    pub fn parse(text: &str) -> Result<Self, DiffError> {
        let mut tiers: Vec<(String, Vec<Scenario>)> = Vec::new();
        let mut header_seen = false;
        for (number, raw) in text.lines().enumerate() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if !header_seen {
                if line != SCENARIO_FORMAT {
                    return Err(DiffError::Format(format!(
                        "line {} must start with {SCENARIO_FORMAT}",
                        number + 1
                    )));
                }
                header_seen = true;
                continue;
            }
            let (word, rest) = split_word(line);
            match word {
                "tier" => {
                    let name = parse_tier(rest, number)?;
                    if tiers.iter().any(|(tier, _)| tier == &name) {
                        return Err(DiffError::Format(format!(
                            "line {} repeats the tier {name}",
                            number + 1
                        )));
                    }
                    tiers.push((name, Vec::new()));
                }
                "scenario" => {
                    let scenario = parse_scenario(rest, number)?;
                    let Some((_, scenarios)) = tiers.last_mut() else {
                        return Err(DiffError::Format(format!(
                            "line {} holds a scenario before any tier",
                            number + 1
                        )));
                    };
                    scenarios.push(scenario);
                }
                other => {
                    return Err(DiffError::Format(format!(
                        "line {} holds the unknown word {other}",
                        number + 1
                    )))
                }
            }
        }
        if !header_seen {
            return Err(DiffError::Format(format!(
                "the list must start with {SCENARIO_FORMAT}"
            )));
        }
        Ok(Self { tiers })
    }
}

/// Splits a line into its first word and the rest.
fn split_word(line: &str) -> (&str, &str) {
    match line.split_once(' ') {
        Some((word, rest)) => (word, rest.trim()),
        None => (line, ""),
    }
}

/// Reads one tier line of a scenario list and returns the tier name.
fn parse_tier(rest: &str, number: usize) -> Result<String, DiffError> {
    let name = rest.trim();
    if name.is_empty() {
        return Err(DiffError::Format(format!(
            "line {} names no tier",
            number + 1
        )));
    }
    Ok(name.to_string())
}

/// Reads one scenario line of a scenario list and returns the scenario.
fn parse_scenario(rest: &str, number: usize) -> Result<Scenario, DiffError> {
    let (name, fields) = split_word(rest);
    if name.is_empty() {
        return Err(DiffError::Format(format!(
            "line {} names no scenario",
            number + 1
        )));
    }
    let mut bin: Option<String> = None;
    let mut args: Vec<String> = Vec::new();
    for field in fields.split_whitespace() {
        let (key, value) = field.split_once('=').ok_or_else(|| {
            DiffError::Format(format!(
                "line {} holds the field {field} without a value",
                number + 1
            ))
        })?;
        match key {
            "bin" => bin = Some(value.to_string()),
            "args" => args.push(value.to_string()),
            other => {
                return Err(DiffError::Format(format!(
                    "line {} holds the unknown field {other}",
                    number + 1
                )))
            }
        }
    }
    let Some(bin) = bin else {
        return Err(DiffError::Format(format!(
            "line {} names no binary",
            number + 1
        )));
    };
    Ok(Scenario {
        name: name.to_string(),
        bin,
        args,
    })
}

/// Runs one tier of a scenario list against a build and returns its log.
///
/// The build directory holds the binaries the scenarios run; the root is the
/// working directory the scenarios see.
pub fn run_tier(
    list: &ScenarioList,
    tier: &str,
    build: &Path,
    root: &Path,
) -> Result<Log, DiffError> {
    let Some(scenarios) = list.tier(tier) else {
        return Err(DiffError::Format(format!(
            "the list holds no tier {tier}, it holds {:?}",
            list.tier_names()
        )));
    };
    let mut steps = Vec::new();
    for scenario in scenarios {
        steps.push(run_scenario(scenario, build, root)?);
    }
    Ok(Log {
        tier: tier.to_string(),
        steps,
    })
}

/// Runs one scenario and returns its step.
fn run_scenario(scenario: &Scenario, build: &Path, root: &Path) -> Result<Step, DiffError> {
    let path = binary_path(&scenario.bin, build);
    let output = Command::new(&path)
        .args(&scenario.args)
        .current_dir(root)
        .output()
        .map_err(|why| DiffError::Io(format!("{}: {why}", path.display())))?;
    let mut lines: Vec<String> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(|line| line.trim_end_matches('\r').to_string())
        .collect();
    if !output.status.success() {
        for line in String::from_utf8_lossy(&output.stderr).lines() {
            lines.push(format!("stderr {}", line.trim_end_matches('\r')));
        }
    }
    lines.push(format!("exit={}", output.status.code().unwrap_or(-1)));
    Ok(Step {
        name: scenario.name.clone(),
        in_scope: true,
        lines,
    })
}

/// The path of a scenario binary: under the build directory, with the
/// platform suffix the build left off.
fn binary_path(bin: &str, build: &Path) -> PathBuf {
    let mut path = build.join(bin);
    if cfg!(windows) && path.extension().is_none() {
        path.set_extension("exe");
    }
    path
}

/// Runs the tool and returns its exit code.
///
/// 0 means every in-scope step matched, 1 means one divergence or more and
/// 2 means the tool could not run. The report goes to `out`, the reason a run
/// failed goes to `err`.
pub fn cli(args: &[String], out: &mut dyn Write, err: &mut dyn Write) -> i32 {
    match run(args, out) {
        Ok(code) => code,
        Err(why) => {
            let _ = writeln!(err, "differential: {why}");
            2
        }
    }
}

/// The usage text of the tool.
fn usage() -> String {
    format!(
        "differential: compare two receipt logs, or run a scenario list against a build\n  \
         differential --reference <log> --candidate <log>\n  \
         differential --scenarios <list> --tier <tier> --build <dir> --out <log>\n  \
         the log format is {LOG_FORMAT}, the list format is {SCENARIO_FORMAT}\n"
    )
}

/// Runs the tool: the arguments, the report writer, the exit code.
fn run(args: &[String], out: &mut dyn Write) -> Result<i32, DiffError> {
    let mut reference: Option<String> = None;
    let mut candidate: Option<String> = None;
    let mut scenarios: Option<String> = None;
    let mut tier: Option<String> = None;
    let mut build: Option<String> = None;
    let mut out_path: Option<String> = None;
    let mut index = 1;
    while index < args.len() {
        let flag = args[index].as_str();
        if flag == "--help" || flag == "-h" {
            let _ = write!(out, "{}", usage());
            return Ok(0);
        }
        let value = value_at(args, index, flag)?;
        match flag {
            "--reference" => reference = Some(value),
            "--candidate" => candidate = Some(value),
            "--scenarios" => scenarios = Some(value),
            "--tier" => tier = Some(value),
            "--build" => build = Some(value),
            "--out" => out_path = Some(value),
            other => return Err(DiffError::Format(format!("the flag {other} is not known"))),
        }
        index += 2;
    }
    match (scenarios, reference, candidate) {
        (Some(list), None, None) => collect(
            &list,
            tier.as_deref(),
            build.as_deref(),
            out_path.as_deref(),
            out,
        ),
        (None, Some(reference), Some(candidate)) => {
            let reference_text = read_text(&reference)?;
            let candidate_text = read_text(&candidate)?;
            let report = diff(&reference_text, &candidate_text)?;
            let _ = write!(out, "{}", report.text());
            Ok(report.exit_code())
        }
        _ => Err(DiffError::Format(usage())),
    }
}

/// Reads the value of a flag.
fn value_at(args: &[String], index: usize, flag: &str) -> Result<String, DiffError> {
    args.get(index + 1)
        .cloned()
        .ok_or_else(|| DiffError::Format(format!("the flag {flag} needs a value")))
}

/// Runs a scenario list and writes its receipt log.
fn collect(
    list_path: &str,
    tier: Option<&str>,
    build: Option<&str>,
    out_path: Option<&str>,
    out: &mut dyn Write,
) -> Result<i32, DiffError> {
    let Some(tier) = tier else {
        return Err(DiffError::Format("the run needs --tier".into()));
    };
    let Some(build) = build else {
        return Err(DiffError::Format("the run needs --build".into()));
    };
    let list = ScenarioList::parse(&read_text(list_path)?)?;
    let root = std::env::current_dir()
        .map_err(|why| DiffError::Io(format!("the working directory: {why}")))?;
    let log = run_tier(&list, tier, Path::new(build), &root)?;
    let text = write_log(&log.tier, &log.steps);
    match out_path {
        Some(path) => {
            fs::write(path, text).map_err(|why| DiffError::Io(format!("{path}: {why}")))?;
        }
        None => {
            let _ = write!(out, "{text}");
        }
    }
    Ok(0)
}

/// Reads a file as text.
fn read_text(path: &str) -> Result<String, DiffError> {
    fs::read_to_string(path).map_err(|why| DiffError::Io(format!("{path}: {why}")))
}

#[cfg(test)]
mod tests {
    use super::{
        cli, compare, diff, parse_log, run_tier, write_log, DiffError, Kind, Log, Normalizer,
        Scenario, ScenarioList, Step,
    };
    use std::fs;
    use std::path::{Path, PathBuf};

    /// A two step log: the golden receipt and the arena row.
    fn sample_log(tier: &str, final_hash: &str) -> String {
        let steps = vec![
            Step {
                name: "golden".into(),
                in_scope: true,
                lines: vec![
                    "engine2040 headless proof:".into(),
                    format!("  ticks=600 final_hash={final_hash}"),
                    "  seed=2040 inputs=0 state_hash=abc backend=null mode=run verified=true"
                        .into(),
                ],
            },
            Step {
                name: "arena".into(),
                in_scope: true,
                lines: vec!["windows-x86_64 arena=aabbccdd ticks=600".into()],
            },
        ];
        write_log(tier, &steps)
    }

    /// The scenario list the ship differential plays, as it is committed.
    ///
    /// The bytes ride in the test binary, so a moved or unreadable file fails
    /// the build of this crate and not the CI job that plays the list.
    const COMMITTED_LIST: &str = include_str!("../../../fixtures/differential/scenarios.txt");

    /// The crate root: the manifest directory of the proof crate.
    fn crate_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
    }

    /// The bins the manifest declares, with the features each one needs.
    ///
    /// The manifest is read as text: the crate carries no TOML parser and
    /// the manifest keeps one shape. A `[[bin]]` block holds a `name` and
    /// `required-features`; the scanner reads those two keys and nothing else.
    fn declared_bins() -> Vec<(String, Vec<String>)> {
        let text = fs::read_to_string(crate_root().join("Cargo.toml")).expect("the manifest");
        let mut bins: Vec<(String, Vec<String>)> = Vec::new();
        let mut block: Option<(String, Vec<String>)> = None;
        for line in text.lines() {
            let line = line.trim();
            if line.starts_with('[') {
                if let Some(bin) = block.take() {
                    bins.push(bin);
                }
                if line == "[[bin]]" {
                    block = Some((String::new(), Vec::new()));
                }
                continue;
            }
            let Some((name, features)) = block.as_mut() else {
                continue;
            };
            if let Some(rest) = line.strip_prefix("name = \"") {
                *name = rest.trim_end_matches('"').to_string();
            } else if let Some(rest) = line.strip_prefix("required-features = [") {
                for item in rest.trim_end_matches(']').split(',') {
                    let item = item.trim().trim_matches('"');
                    if !item.is_empty() {
                        features.push(item.to_string());
                    }
                }
            }
        }
        if let Some(bin) = block.take() {
            bins.push(bin);
        }
        bins
    }

    /// The feature names the manifest declares.
    fn declared_features() -> Vec<String> {
        let text = fs::read_to_string(crate_root().join("Cargo.toml")).expect("the manifest");
        let mut features = Vec::new();
        let mut in_features = false;
        for line in text.lines() {
            let line = line.trim();
            if line.starts_with('[') {
                in_features = line == "[features]";
                continue;
            }
            if !in_features || line.starts_with('#') {
                continue;
            }
            let Some((key, _)) = line.split_once('=') else {
                continue;
            };
            let key = key.trim();
            if !key.is_empty() {
                features.push(key.to_string());
            }
        }
        features
    }

    /// The directory the test writes its files into.
    fn scratch_dir() -> PathBuf {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/differential-test")
            .join(format!("pid-{}", std::process::id()));
        fs::create_dir_all(&dir).expect("the scratch directory");
        dir
    }

    /// Writes a text file into the scratch directory and returns its path.
    fn scratch_file(name: &str, text: &str) -> PathBuf {
        let path = scratch_dir().join(name);
        fs::write(&path, text).expect("the scratch file");
        path
    }

    #[test]
    fn two_runs_of_the_same_build_give_the_same_receipt() {
        let exe = std::env::current_exe().expect("the test binary path");
        let list = ScenarioList::new(vec![(
            "fast".to_string(),
            vec![Scenario {
                name: "self".into(),
                bin: exe.to_string_lossy().into_owned(),
                args: vec!["--list".into()],
            }],
        )]);
        let root = scratch_dir();
        let first = run_tier(&list, "fast", &root, &root).expect("the first run");
        let second = run_tier(&list, "fast", &root, &root).expect("the second run");
        assert_eq!(first, second, "two runs of the same build differ");
        let report = diff(
            &write_log(&first.tier, &first.steps),
            &write_log(&second.tier, &second.steps),
        )
        .expect("the two logs compare");
        assert!(report.is_clean(), "the two logs diverge: {}", report.text());
        assert_eq!(report.exit_code(), 0);
    }

    #[test]
    fn a_seeded_change_shows_up_and_is_not_normalized_away() {
        let reference = sample_log("fast", "d57b5c8ae3096614");
        let candidate = sample_log("fast", "0000000000000001");
        let report = diff(&reference, &candidate).expect("the two logs compare");
        assert!(!report.is_clean());
        let divergence = report.divergences.first().expect("a divergence");
        assert_eq!(divergence.step, "golden");
        assert_eq!(divergence.line, 2);
        assert_eq!(divergence.kind, Kind::ChangedLine);
        assert!(divergence.reference.contains("d57b5c8ae3096614"));
        assert!(divergence.candidate.contains("0000000000000001"));
    }

    #[test]
    fn a_seed_value_is_never_normalized_away() {
        let normalizer = Normalizer::volatile();
        let line = "  seed=2040 inputs=0 state_hash=abc backend=null mode=run";
        let seeded = "  seed=2041 inputs=0 state_hash=abc backend=null mode=run";
        let kept = normalizer.normalize_line(line);
        assert!(kept.contains("seed=2040"), "the kept line is {kept}");
        assert_ne!(kept, normalizer.normalize_line(seeded));
    }

    #[test]
    fn a_measured_value_never_hides_a_change() {
        let normalizer = Normalizer::volatile();
        let reference = "physfx: bodies=12 contacts=30 us=420";
        let candidate = "physfx: bodies=12 contacts=30 us=980";
        assert_eq!(
            normalizer.normalize_line(reference),
            normalizer.normalize_line(candidate)
        );
        let hash_change = "role=server hash=aaaa rtt_ms=4 desync=0";
        let hash_other = "role=server hash=bbbb rtt_ms=9 desync=0";
        assert_ne!(
            normalizer.normalize_line(hash_change),
            normalizer.normalize_line(hash_other)
        );
    }

    #[test]
    fn a_behaviour_count_is_never_normalized_away() {
        let normalizer = Normalizer::volatile();
        let reference = "windows-x86_64 arena=aabbccdd ticks=600 hits=12 blocks=3 breaks=1";
        let candidate = "windows-x86_64 arena=aabbccdd ticks=600 hits=12 blocks=4 breaks=1";
        assert_ne!(
            normalizer.normalize_line(reference),
            normalizer.normalize_line(candidate)
        );
    }

    #[test]
    fn two_hosts_compare_equal() {
        let normalizer = Normalizer::volatile();
        let windows = "windows-x86_64 tick=0 hash=aaaa";
        let linux = "linux-x86_64 tick=0 hash=aaaa";
        assert_eq!(
            normalizer.normalize_line(windows),
            normalizer.normalize_line(linux)
        );
    }

    #[test]
    fn a_missing_step_is_a_divergence() {
        let reference = sample_log("fast", "d57b5c8ae3096614");
        let log = parse_log(&reference).expect("the reference parses");
        let shorter = Log {
            tier: log.tier.clone(),
            steps: vec![log.steps[0].clone()],
        };
        let report = compare(&log, &shorter, &Normalizer::volatile());
        assert_eq!(report.divergences.len(), 1);
        assert_eq!(report.divergences[0].kind, Kind::MissingStep);
        assert_eq!(report.divergences[0].step, "arena");
        assert_eq!(report.exit_code(), 1);
    }

    #[test]
    fn an_out_of_scope_step_is_skipped() {
        let reference = sample_log("fast", "d57b5c8ae3096614");
        let mut log = parse_log(&reference).expect("the reference parses");
        log.steps[1].in_scope = false;
        let candidate = write_log(&log.tier, &log.steps);
        let report = diff(&reference, &candidate).expect("the two logs compare");
        assert!(report.is_clean(), "the out of scope step diverged");
    }

    #[test]
    fn the_report_names_the_step_and_the_line() {
        let reference = sample_log("fast", "d57b5c8ae3096614");
        let candidate = sample_log("fast", "0000000000000001");
        let report = diff(&reference, &candidate).expect("the two logs compare");
        let text = report.text();
        assert!(text.contains("step=golden"), "the text is {text}");
        assert!(text.contains("line=2"), "the text is {text}");
        assert!(text.contains("kind=changed-line"), "the text is {text}");
    }

    #[test]
    fn the_tool_exits_one_on_one_in_scope_divergence() {
        let reference = scratch_file("reference.log", &sample_log("fast", "d57b5c8ae3096614"));
        let candidate = scratch_file("candidate.log", &sample_log("fast", "0000000000000001"));
        let args = [
            "differential".to_string(),
            "--reference".to_string(),
            reference.to_string_lossy().into_owned(),
            "--candidate".to_string(),
            candidate.to_string_lossy().into_owned(),
        ];
        let mut out: Vec<u8> = Vec::new();
        let mut err: Vec<u8> = Vec::new();
        let code = cli(&args, &mut out, &mut err);
        assert_eq!(code, 1, "the tool must exit 1 on a divergence");
        let text = String::from_utf8_lossy(&out).into_owned();
        assert!(
            text.contains("DIVERGENCE step=golden"),
            "the text is {text}"
        );
    }

    #[test]
    fn the_tool_exits_zero_when_the_logs_match() {
        let reference = scratch_file(
            "match-reference.log",
            &sample_log("fast", "d57b5c8ae3096614"),
        );
        let candidate = scratch_file(
            "match-candidate.log",
            &sample_log("fast", "d57b5c8ae3096614"),
        );
        let args = [
            "differential".to_string(),
            "--reference".to_string(),
            reference.to_string_lossy().into_owned(),
            "--candidate".to_string(),
            candidate.to_string_lossy().into_owned(),
        ];
        let mut out: Vec<u8> = Vec::new();
        let mut err: Vec<u8> = Vec::new();
        assert_eq!(cli(&args, &mut out, &mut err), 0);
    }

    #[test]
    fn the_tool_exits_two_when_it_cannot_run() {
        let args = ["differential".to_string(), "--reference".to_string()];
        let mut out: Vec<u8> = Vec::new();
        let mut err: Vec<u8> = Vec::new();
        assert_eq!(cli(&args, &mut out, &mut err), 2);
    }

    #[test]
    fn two_tiers_may_not_be_compared() {
        let reference = sample_log("fast", "d57b5c8ae3096614");
        let candidate = sample_log("deep", "d57b5c8ae3096614");
        let error = diff(&reference, &candidate).expect_err("the tiers differ");
        assert!(matches!(error, DiffError::Tier(_)), "the error is {error}");
    }

    #[test]
    fn the_log_round_trips() {
        let text = sample_log("fast", "d57b5c8ae3096614");
        let log = parse_log(&text).expect("the log parses");
        assert_eq!(log.tier, "fast");
        assert_eq!(log.steps.len(), 2);
        assert_eq!(log.steps[0].name, "golden");
        assert_eq!(log.steps[0].lines.len(), 3);
        assert_eq!(write_log(&log.tier, &log.steps), text);
    }

    #[test]
    fn a_log_without_a_header_is_refused() {
        let error = parse_log("STEP golden\nhash=1\n").expect_err("the header is missing");
        assert!(
            matches!(error, DiffError::Format(_)),
            "the error is {error}"
        );
    }

    #[test]
    fn the_scenario_list_holds_its_tiers() {
        let list = ScenarioList::parse(
            "differential scenario list v1\n\
             # a comment\n\
             tier fast\n\
             scenario golden bin=engine2040 args=--verify\n\
             tier deep\n\
             scenario arena bin=arena_line\n",
        )
        .expect("the list parses");
        assert_eq!(list.tier_names(), vec!["fast", "deep"]);
        let fast = list.tier("fast").expect("the fast tier");
        assert_eq!(fast.len(), 1);
        assert_eq!(fast[0].name, "golden");
        assert_eq!(fast[0].bin, "engine2040");
        assert_eq!(fast[0].args, vec!["--verify".to_string()]);
        assert!(list.tier("missing").is_none());
    }

    #[test]
    fn an_unknown_field_is_refused() {
        let error = ScenarioList::parse(
            "differential scenario list v1\ntier fast\nscenario golden bin=x colour=red\n",
        )
        .expect_err("the field is unknown");
        assert!(
            matches!(error, DiffError::Format(_)),
            "the error is {error}"
        );
    }

    /// The curated scenario list the repo ships, parsed. A typo in the file
    /// fails here, not in CI.
    fn committed_scenario_list() -> ScenarioList {
        ScenarioList::parse(COMMITTED_LIST).expect("the committed scenario list parses")
    }

    #[test]
    fn the_committed_scenario_list_parses_and_names_its_tiers() {
        let list = committed_scenario_list();
        assert_eq!(list.tier_names(), vec!["fast", "deep"]);
        let fast = list.tier("fast").expect("the fast tier");
        assert_eq!(fast.len(), 2);
        assert_eq!(fast[0].name, "matrix");
        assert_eq!(fast[0].bin, "matrix_line");
        assert_eq!(fast[1].name, "arena");
        assert_eq!(fast[1].bin, "arena_line");
        let deep = list.tier("deep").expect("the deep tier");
        assert_eq!(deep.len(), 2);
        assert_eq!(deep[0].name, "spine");
        assert_eq!(deep[0].bin, "spine_parity_line");
        assert_eq!(deep[1].name, "ragdoll");
        assert_eq!(deep[1].bin, "ragdoll_line");
    }

    #[test]
    fn every_committed_scenario_names_a_binary_the_crate_builds() {
        let list = committed_scenario_list();
        let bins = declared_bins();
        let features = declared_features();
        for tier in list.tier_names() {
            let scenarios = list.tier(tier).expect("the tier");
            for scenario in scenarios {
                // The bin has a source file: a name the crate never builds
                // fails here, not in the CI job that plays the list.
                let source = crate_root()
                    .join("src/bin")
                    .join(format!("{}.rs", scenario.bin));
                assert!(
                    source.is_file(),
                    "the scenario {tier}/{} names the binary {}, but {} is missing",
                    scenario.name,
                    scenario.bin,
                    source.display()
                );
                // The features that bin needs are features the crate declares,
                // so the CI build can turn them on.
                if let Some((_, needed)) = bins.iter().find(|(name, _)| name == &scenario.bin) {
                    for feature in needed {
                        assert!(
                            features.contains(feature),
                            "the binary {} needs the feature {}, which the crate does not declare",
                            scenario.bin,
                            feature
                        );
                    }
                }
            }
        }
    }
}
