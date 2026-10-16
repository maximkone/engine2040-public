//! Mirror kit: task, outcome and report types shared by the three
//! public-benchmark slices (shapes and scoring rules only, no suite text).
//!
//! Consumer: `mirror_gdb`, `mirror_craft`, `mirror_logic` and the 30-task
//! aggregate. Owner: slices own their tasks and published numbers; this
//! module owns the row, parity and print rules. Each task runs a reference
//! submission (must solve) and a weak one (the checker must reject it), on
//! every eval backend, so a checker that accepts anything cannot score.
//! Units: counts of tasks and checks; latency and cost are the eval fixtures.
//! Determinism: rows hold no wall clock; the same slice twice prints the
//! same text. Unknown rival numbers stay [`Claim::Unknown`] and print
//! `UNKNOWN`; a quoted number carries its public source.

use std::collections::BTreeMap;

use engine_core::Fnv64;
use engine_provider::protocol::ProposalAction;

use crate::door::{block_prop, fixture_scene, Delivery, Door, Scene, Submit};
use crate::eval::{backend_label, bound_detail, fixture_cost, fixture_latency_ms, Cost, BACKENDS};

/// Rival MCP doors the aggregate prints beside each suite, in column order.
pub const RIVALS: [&str; 3] = ["Unreal 5.8 MCP", "Unity AI MCP", "Godot MCP"];

/// A number about someone else: quoted from a public source, or unknown.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Claim {
    /// Not run and not read by us; never printed as a number.
    Unknown,
    /// A published figure with the page it was read from.
    Quoted {
        /// The figure as the source states it.
        text: &'static str,
        /// Public page the figure was read from.
        source: &'static str,
    },
}

impl Claim {
    /// Report text: `UNKNOWN`, or the figure with its source.
    pub fn text(self) -> String {
        match self {
            Self::Unknown => "UNKNOWN".to_owned(),
            Self::Quoted { text, source } => format!("{text} [{source}]"),
        }
    }

    /// True only for [`Claim::Unknown`].
    pub const fn is_unknown(self) -> bool {
        matches!(self, Self::Unknown)
    }
}

/// One public suite a slice shapes its tasks after.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SuiteInfo {
    /// Stable lowercase id.
    pub id: &'static str,
    /// Suite name as published.
    pub name: &'static str,
    /// Headline figure the suite publishes for its own best run.
    pub published: Claim,
    /// Rival MCP door scores on the mirrored tasks, in [`RIVALS`] order.
    pub rivals: [Claim; 3],
}

/// One named stage result inside a task outcome.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Check {
    /// Stage or assertion name.
    pub name: &'static str,
    /// True when the stage held.
    pub ok: bool,
}

/// What a task decided for one submission.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Outcome {
    /// True when the submission solves the task.
    pub pass: bool,
    /// Named stage results in the order they ran.
    pub checks: Vec<Check>,
    /// Digest of the state the submission left behind.
    pub hash: u64,
    /// Bounded one-line explanation.
    pub detail: String,
}

impl Outcome {
    /// Pass exactly when there is at least one check and all hold. An empty
    /// check list never passes, so a task that checks nothing cannot score.
    pub(crate) fn from_checks(checks: Vec<Check>, hash: u64, detail: String) -> Self {
        let pass = !checks.is_empty() && checks.iter().all(|check| check.ok);
        Self::decided(pass, checks, hash, detail)
    }

    /// Outcome whose verdict the task sets itself (a feedback loop passes on
    /// its last attempt even when the first stage failed).
    pub(crate) fn decided(pass: bool, checks: Vec<Check>, hash: u64, detail: String) -> Self {
        Self {
            pass,
            checks,
            hash,
            detail: bound_detail(detail),
        }
    }

    /// Result of the stage called `name`, if it ran.
    pub fn stage(&self, name: &str) -> Option<bool> {
        self.checks
            .iter()
            .find(|check| check.name == name)
            .map(|check| check.ok)
    }

    /// Name of the first stage that failed, if any.
    pub fn first_failure(&self) -> Option<&'static str> {
        self.checks
            .iter()
            .find(|check| !check.ok)
            .map(|check| check.name)
    }
}

/// Which submission a task run replays.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Submission {
    /// The recorded answer that solves the task.
    Reference,
    /// A recorded wrong answer the checker must reject.
    Weak,
}

impl Submission {
    /// The reference value for a reference run, the weak one otherwise.
    pub(crate) fn pick<T>(self, reference: T, weak: T) -> T {
        match self {
            Self::Reference => reference,
            Self::Weak => weak,
        }
    }
}

/// Fixture with a full-width wall across the middle: nothing is reachable.
pub(crate) fn sealed_scene() -> Scene {
    let mut scene = fixture_scene();
    scene
        .variant
        .props
        .push(block_prop(30, "barrier", [0.0, 0.0], [1.0, 2.0, 40.0]));
    scene
}

/// True when every prop has a partner at the point-mirrored position.
pub(crate) fn point_symmetric(scene: &Scene) -> bool {
    let near = |a: f32, b: f32| (a - b).abs() < 0.005;
    scene.variant.props.iter().all(|prop| {
        scene
            .variant
            .props
            .iter()
            .any(|other| near(other.x, -prop.x) && near(other.z, -prop.z))
    })
}

/// One named stage result.
pub(crate) fn ck(name: &'static str, ok: bool) -> Check {
    Check { name, ok }
}

/// Admit a submission at the door: a no-op batch carrying `note` must be
/// accepted and change nothing. Failing it means the door refused the task.
pub(crate) fn admit(door: &mut Door, note: &str) -> Check {
    let mut spare = fixture_scene();
    let notes = [note.to_owned()];
    let actions = [ProposalAction::Noop];
    let submit = Submit {
        diagnostics: &notes,
        ..Submit::clean(spare.revision, &actions)
    };
    let delivery = door.submit(&mut spare, &submit);
    ck("admitted", delivery == Delivery::Applied(false))
}

/// Digest of a text, for outcomes whose state is a message.
pub(crate) fn text_hash(text: &str) -> u64 {
    let mut hash = Fnv64::new();
    hash.write_str(text);
    hash.finish()
}

/// Outcome over a scene: the door's delivery leads the detail and the scene
/// hash is the state digest.
pub(crate) fn scene_outcome(scene: &Scene, delivery: &Delivery, checks: Vec<Check>) -> Outcome {
    Outcome::from_checks(checks, scene.hash(), delivery.label())
}

/// One mirrored task: a public-suite shape plus the code that runs it.
pub(crate) struct MirrorTask {
    /// Stable task id.
    pub id: &'static str,
    /// Score bucket (the suite's own split where it publishes one).
    pub category: &'static str,
    /// Public-suite shape this task mirrors, in a few words.
    pub shape: &'static str,
    /// Runs one submission through the door and scores it.
    pub run: fn(&mut Door, Submission) -> Outcome,
}

/// One task on one backend: reference and weak outcomes plus eval fixtures.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct MirrorRow {
    /// Task id.
    pub task: &'static str,
    /// Score bucket.
    pub category: &'static str,
    /// Public-suite shape.
    pub shape: &'static str,
    /// Backend label (`mock`, `local`, `remote`).
    pub backend: &'static str,
    /// The reference submission's outcome.
    pub reference: Outcome,
    /// The weak submission's outcome.
    pub weak: Outcome,
    /// Serving latency in ms (fixture table, not wall clock).
    pub latency_ms: u64,
    /// Serving cost; unknown stays unknown.
    pub cost: Cost,
}

impl MirrorRow {
    /// True when the reference solves and the weak submission is rejected.
    pub fn green(&self) -> bool {
        self.reference.pass && !self.weak.pass
    }
}

/// One suite slice: every task on every backend.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct SliceReport {
    /// Suite the tasks mirror.
    pub suite: &'static SuiteInfo,
    /// One row per task per backend, task-major in [`BACKENDS`] order.
    pub rows: Vec<MirrorRow>,
    /// Suite-specific summary lines, printed under the common ones.
    pub notes: Vec<String>,
}

/// Run every task on every backend. `base_index` offsets the eval fixture
/// slot so request ids and latencies stay distinct across slices.
pub(crate) fn run_slice(
    suite: &'static SuiteInfo,
    tasks: &[MirrorTask],
    base_index: u64,
    notes: fn(&SliceReport) -> Vec<String>,
) -> SliceReport {
    let mut rows = Vec::with_capacity(tasks.len() * BACKENDS.len());
    for (offset, task) in tasks.iter().enumerate() {
        let index = base_index + u64::try_from(offset).unwrap_or(u64::MAX);
        for backend in BACKENDS {
            let reference = (task.run)(&mut Door::new(backend, index), Submission::Reference);
            let weak = (task.run)(&mut Door::new(backend, index), Submission::Weak);
            rows.push(MirrorRow {
                task: task.id,
                category: task.category,
                shape: task.shape,
                backend: backend_label(backend),
                reference,
                weak,
                latency_ms: fixture_latency_ms(backend, index),
                cost: fixture_cost(backend, index),
            });
        }
    }
    let mut report = SliceReport {
        suite,
        rows,
        notes: Vec::new(),
    };
    report.notes = notes(&report);
    report
}

impl SliceReport {
    fn on<'a>(&'a self, backend: &'a str) -> impl Iterator<Item = &'a MirrorRow> {
        self.rows.iter().filter(move |row| row.backend == backend)
    }

    /// Tasks in the slice.
    pub fn tasks(&self) -> usize {
        self.on(backend_label(BACKENDS[0])).count()
    }

    /// Tasks whose reference submission solves, on one backend.
    pub fn solved(&self, backend: &str) -> usize {
        self.on(backend).filter(|row| row.reference.pass).count()
    }

    /// Tasks whose weak submission the checker rejects, on one backend.
    pub fn caught(&self, backend: &str) -> usize {
        self.on(backend).filter(|row| !row.weak.pass).count()
    }

    /// Solved and total per category on one backend, in name order.
    pub fn by_category(&self, backend: &str) -> Vec<(&'static str, usize, usize)> {
        let mut tally: BTreeMap<&'static str, (usize, usize)> = BTreeMap::new();
        for row in self.on(backend) {
            let entry = tally.entry(row.category).or_default();
            entry.0 += usize::from(row.reference.pass);
            entry.1 += 1;
        }
        tally
            .into_iter()
            .map(|(name, (solved, total))| (name, solved, total))
            .collect()
    }

    /// Reference checks held and run, summed over one backend's tasks.
    pub fn check_totals(&self, backend: &str) -> (usize, usize) {
        let mut held = 0;
        let mut run = 0;
        for row in self.on(backend) {
            held += row.reference.checks.iter().filter(|check| check.ok).count();
            run += row.reference.checks.len();
        }
        (held, run)
    }

    /// Weak submissions that cleared stage `cleared` and failed stage
    /// `failed`, on one backend.
    pub fn weak_cleared_then_failed(&self, backend: &str, cleared: &str, failed: &str) -> usize {
        self.on(backend)
            .filter(|row| {
                row.weak.stage(cleared) == Some(true) && row.weak.stage(failed) == Some(false)
            })
            .count()
    }

    /// Weak submissions by the first stage they failed, on one backend.
    pub fn weak_failure_stages(&self, backend: &str) -> Vec<(&'static str, usize)> {
        let mut tally: BTreeMap<&'static str, usize> = BTreeMap::new();
        for row in self.on(backend) {
            if let Some(name) = row.weak.first_failure() {
                *tally.entry(name).or_default() += 1;
            }
        }
        tally.into_iter().collect()
    }

    /// Tasks whose reference and weak outcomes are identical on every
    /// backend, plus the ids that diverge.
    pub fn parity(&self) -> (usize, Vec<&'static str>) {
        let mut identical = 0;
        let mut divergent = Vec::new();
        let first = backend_label(BACKENDS[0]);
        for lead in self.on(first) {
            let same = BACKENDS.iter().all(|backend| {
                self.on(backend_label(*backend))
                    .find(|row| row.task == lead.task)
                    .is_some_and(|row| row.reference == lead.reference && row.weak == lead.weak)
            });
            if same {
                identical += 1;
            } else {
                divergent.push(lead.task);
            }
        }
        (identical, divergent)
    }

    /// Reproducible text: summary lines, then one line per task.
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        let tasks = self.tasks();
        out.push_str(&format!(
            "{} slice: {tasks} tasks x mock/local/remote, recorded replay, no model call\n",
            self.suite.name
        ));
        out.push_str(&format!("  published: {}\n", self.suite.published.text()));
        out.push_str(&format!(
            "  solved {}  caught {}\n",
            self.per_backend(Self::solved, tasks),
            self.per_backend(Self::caught, tasks),
        ));
        let (identical, divergent) = self.parity();
        let divergent = if divergent.is_empty() {
            "none".to_owned()
        } else {
            divergent.join(",")
        };
        out.push_str(&format!(
            "  parity {identical}/{tasks} divergent={divergent}\n"
        ));
        let mock = backend_label(BACKENDS[0]);
        let categories: Vec<String> = self
            .by_category(mock)
            .into_iter()
            .map(|(name, solved, total)| format!("{name} {solved}/{total}"))
            .collect();
        let (held, run) = self.check_totals(mock);
        out.push_str(&format!(
            "  categories {}  checks {held}/{run}\n",
            categories.join(" ")
        ));
        for note in &self.notes {
            out.push_str(&format!("  {note}\n"));
        }
        out.push_str("  task category reference weak latency_ms hash shape | detail\n");
        for lead in self.on(mock) {
            out.push_str(&self.task_line(lead));
        }
        out
    }

    fn per_backend(&self, count: fn(&Self, &str) -> usize, tasks: usize) -> String {
        BACKENDS
            .iter()
            .map(|backend| {
                let label = backend_label(*backend);
                format!("{label}={}/{tasks}", count(self, label))
            })
            .collect::<Vec<_>>()
            .join(" ")
    }

    fn task_line(&self, lead: &MirrorRow) -> String {
        let across = |pick: fn(&MirrorRow) -> bool| -> String {
            BACKENDS
                .iter()
                .filter_map(|backend| {
                    self.on(backend_label(*backend))
                        .find(|row| row.task == lead.task)
                        .map(|row| if pick(row) { "pass" } else { "fail" })
                })
                .collect::<Vec<_>>()
                .join("/")
        };
        let latency: Vec<String> = BACKENDS
            .iter()
            .filter_map(|backend| {
                self.on(backend_label(*backend))
                    .find(|row| row.task == lead.task)
                    .map(|row| row.latency_ms.to_string())
            })
            .collect();
        format!(
            "  {} {} {} {} {} {:016x} {} | ref: {} | weak: {}\n",
            lead.task,
            lead.category,
            across(|row| row.reference.pass),
            across(|row| row.weak.pass),
            latency.join("/"),
            lead.reference.hash,
            lead.shape,
            lead.reference.detail,
            lead.weak.detail,
        )
    }
}

/// `a/b/c` of a per-backend count, in [`BACKENDS`] report order.
///
/// Shared owner for the mirror reports: `mirror` and `bench_mirror` print
/// the same solved/caught columns from the same backends, so both call
/// this instead of keeping a private copy.
pub(crate) fn triple(count: impl Fn(&str) -> usize) -> String {
    BACKENDS
        .iter()
        .map(|backend| count(backend_label(*backend)).to_string())
        .collect::<Vec<_>>()
        .join("/")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::Backend;

    static SUITE: SuiteInfo = SuiteInfo {
        id: "kit",
        name: "KitSuite",
        published: Claim::Quoted {
            text: "fixture figure",
            source: "https://example.invalid/kit",
        },
        rivals: [Claim::Unknown; 3],
    };

    fn solve(_door: &mut Door, _sub: Submission) -> Outcome {
        Outcome::from_checks(
            vec![Check {
                name: "only",
                ok: true,
            }],
            7,
            "ok".to_owned(),
        )
    }

    fn reject_weak(_door: &mut Door, sub: Submission) -> Outcome {
        let ok = sub == Submission::Reference;
        Outcome::from_checks(vec![Check { name: "only", ok }], 7, "ok".to_owned())
    }

    fn fixture_notes(_report: &SliceReport) -> Vec<String> {
        vec!["fixture note".to_owned()]
    }

    fn tasks() -> [MirrorTask; 2] {
        [
            MirrorTask {
                id: "k1",
                category: "a",
                shape: "accepts anything",
                run: solve,
            },
            MirrorTask {
                id: "k2",
                category: "b",
                shape: "rejects the weak answer",
                run: reject_weak,
            },
        ]
    }

    #[test]
    fn triple_counts_each_backend_in_report_order() {
        assert_eq!(
            triple(|label| match label {
                "mock" => 1,
                "local" => 2,
                "remote" => 3,
                _ => 0,
            }),
            "1/2/3"
        );
        assert_eq!(triple(|_| 0), "0/0/0");
    }

    #[test]
    fn a_checker_that_accepts_the_weak_answer_does_not_count_as_caught() {
        let report = run_slice(&SUITE, &tasks(), 0, fixture_notes);
        for backend in BACKENDS {
            let label = backend_label(backend);
            assert_eq!(report.solved(label), 2);
            assert_eq!(report.caught(label), 1, "k1 accepts its weak answer");
        }
        let green: Vec<&str> = report
            .rows
            .iter()
            .filter(|row| row.backend == "mock" && row.green())
            .map(|row| row.task)
            .collect();
        assert_eq!(green, ["k2"]);
    }

    #[test]
    fn empty_check_lists_never_pass() {
        let outcome = Outcome::from_checks(Vec::new(), 0, String::new());
        assert!(!outcome.pass, "a task that checks nothing must not score");
    }

    #[test]
    fn parity_names_the_task_whose_remote_row_diverges() {
        let mut report = run_slice(&SUITE, &tasks(), 0, fixture_notes);
        assert_eq!(report.parity(), (2, Vec::new()));
        let remote = report
            .rows
            .iter_mut()
            .find(|row| row.task == "k2" && row.backend == "remote")
            .expect("k2 remote row exists");
        remote.reference.hash ^= 1;
        let (identical, divergent) = report.parity();
        assert_eq!(identical, 1);
        assert_eq!(divergent, ["k2"]);
        assert!(report.to_text().contains("divergent=k2"));
    }

    #[test]
    fn stage_helpers_read_named_checks() {
        let outcome = Outcome::from_checks(
            vec![
                Check {
                    name: "build",
                    ok: true,
                },
                Check {
                    name: "asset",
                    ok: true,
                },
                Check {
                    name: "runtime",
                    ok: false,
                },
            ],
            0,
            String::new(),
        );
        assert_eq!(outcome.stage("asset"), Some(true));
        assert_eq!(outcome.stage("missing"), None);
        assert_eq!(outcome.first_failure(), Some("runtime"));
        assert!(!outcome.pass);
    }

    #[test]
    fn unknown_claims_print_unknown_and_quotes_print_their_source() {
        assert_eq!(Claim::Unknown.text(), "UNKNOWN");
        assert!(Claim::Unknown.is_unknown());
        let quoted = SUITE.published.text();
        assert!(quoted.contains("fixture figure") && quoted.contains("https://"));
        assert!(!SUITE.published.is_unknown());
    }

    #[test]
    fn slice_text_reproduces_and_names_every_backend() {
        let first = run_slice(&SUITE, &tasks(), 0, fixture_notes).to_text();
        let second = run_slice(&SUITE, &tasks(), 0, fixture_notes).to_text();
        assert_eq!(first, second);
        for backend in [Backend::Mock, Backend::Local, Backend::Remote] {
            assert!(first.contains(backend_label(backend)), "{first}");
        }
        assert!(first.contains("KitSuite slice: 2 tasks"));
    }
}
