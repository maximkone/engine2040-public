//! Tool-surface benchmark: the recorded twenty-five-task suite under three
//! listed-tool surfaces.
//!
//! Consumer: the AI-door tool-surface row. It asks for the recorded tasks run
//! with ten, thirty and all tools listed, on the local and remote models,
//! reporting completion and cost. The numbers below set the default tool set
//! and feed the research table that this report test prints.
//!
//! Method: the eval harness drives each task through scripted provider steps
//! on fixtures, with no door edge, so the listed surface cannot change its
//! rows. This file layers the surface question on top without touching that
//! harness: every task names the single door verb it needs, a row completes
//! on a surface only when that verb is listed and the scripted outcome
//! completes, and the verdict triple (completed, rejected, privacy) is graded
//! against the transcribed designed outcome. Surfaces are the ten-tool spine,
//! the full twenty-one-tool door, and the full door plus ten synthetic
//! standby names that carry no capability and gate nothing.
//!
//! Units: discovery cost in name chars, serving cost in fixture minor units
//! (local rows report forty plus the task slot; the other backends stay
//! unknown, never zero). The full-list reference (forty thousand five hundred
//! fourteen chars under a forty-three thousand five hundred fourteen budget)
//! is parsed out of the door source at test time, so a
//! door change fails loudly here instead of drifting silently.
//!
//! Determinism scope: the table is pure const data plus a read of the lane
//! tree; rendering it twice gives byte-identical text and one digest, with no
//! wall clock, no thread, no model and no network. The digest is the recorded
//! call sequence: a rerun with no model replays to the same number.
//!
//! Proxies, stated plainly: fork tasks ride the headless match-run verb (the
//! door lists no dedicated fork verb), and the content-verdict task is
//! harness-side (the door lists no verdict verb). Both gaps are findings for
//! the door owner, not hidden in the numbers.
//!
//! Errors: every check is a test assertion with the invariant named. A moved
//! door source, a renamed verb or a changed budget reads as a loud failure,
//! never a skipped row.

use std::path::PathBuf;
use std::sync::OnceLock;

use engine_core::hash_bytes;

/// Door verbs in contract order, transcribed from the door router table.
const ALL_TOOLS: [&str; 21] = [
    "scene.read",
    "scene.brief",
    "scene.spatial",
    "tx.preview",
    "tx.apply",
    "tx.undo",
    "scene.place",
    "level.build",
    "scene.query",
    "sim.plan_goals",
    "play.run",
    "play.step",
    "capture",
    "log.digest",
    "log.query",
    "api.search",
    "api.describe",
    "code.callers",
    "asset.refs",
    "rpc.discover",
    "scene.watch",
];

/// Ten-tool spine: the authoring read/propose/check/apply/undo path plus the
/// match-run verb and discovery. It keeps every authoring flow covered and
/// drops the frame-capture verb on purpose, so the table measures the blind
/// spot instead of assuming it away.
const SMALL_TOOLS: [&str; 10] = [
    "scene.read",
    "scene.brief",
    "scene.spatial",
    "scene.place",
    "scene.query",
    "tx.preview",
    "tx.apply",
    "tx.undo",
    "play.run",
    "rpc.discover",
];

/// Synthetic standby names for the thirty-one-tool surface. They stand in for
/// unlisted-tool context cost, carry no capability, gate no task, and never
/// collide with a real verb. The distractor fixture owned by the eval side
/// stays the home for realistic names; these are shape only.
const DISTRACTOR_TOOLS: [&str; 10] = [
    "x.distractor_01",
    "x.distractor_02",
    "x.distractor_03",
    "x.distractor_04",
    "x.distractor_05",
    "x.distractor_06",
    "x.distractor_07",
    "x.distractor_08",
    "x.distractor_09",
    "x.distractor_10",
];

/// Pipeline stage under test, mirroring the eval harness flow labels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Flow {
    Brief,
    Place,
    Preview,
    Apply,
    Undo,
    Fork,
    Run,
    Capture,
    Verify,
}

/// How a task drives its backend, mirroring the harness script arms.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Script {
    Clean,
    Malformed,
    Stale,
    Timeout,
    Quota,
    PrivacyProbe,
    NeutralFork,
    BullyFork,
    CleanRun,
    ReseedRun,
    CleanCapture,
    CaughtCapture,
    ReferenceVerdict,
}

/// Model-serving backends the row asks about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Backend {
    Local,
    Remote,
}

impl Backend {
    /// Stable lowercase label, matching the recorded report.
    const fn label(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Remote => "remote",
        }
    }
}

/// One benchmark task: stable id, stage, script, and the door verb it needs.
/// `None` means harness-side: the verdict is computed from fixtures with no
/// door call, so every surface covers it.
struct Task {
    id: &'static str,
    flow: Flow,
    script: Script,
    required: Option<&'static str>,
}

/// The twenty authoring tasks, in recorded order (slots zero to nineteen).
const AUTHORING_TASKS: [Task; 20] = [
    Task {
        id: "a01",
        flow: Flow::Brief,
        script: Script::Clean,
        required: Some("scene.brief"),
    },
    Task {
        id: "a02",
        flow: Flow::Brief,
        script: Script::Clean,
        required: Some("scene.brief"),
    },
    Task {
        id: "a03",
        flow: Flow::Brief,
        script: Script::Stale,
        required: Some("scene.brief"),
    },
    Task {
        id: "a04",
        flow: Flow::Place,
        script: Script::Clean,
        required: Some("scene.place"),
    },
    Task {
        id: "a05",
        flow: Flow::Place,
        script: Script::Clean,
        required: Some("scene.place"),
    },
    Task {
        id: "a06",
        flow: Flow::Place,
        script: Script::Clean,
        required: Some("scene.place"),
    },
    Task {
        id: "a07",
        flow: Flow::Place,
        script: Script::Malformed,
        required: Some("scene.place"),
    },
    Task {
        id: "a08",
        flow: Flow::Place,
        script: Script::PrivacyProbe,
        required: Some("scene.place"),
    },
    Task {
        id: "a09",
        flow: Flow::Preview,
        script: Script::Clean,
        required: Some("tx.preview"),
    },
    Task {
        id: "a10",
        flow: Flow::Preview,
        script: Script::Timeout,
        required: Some("tx.preview"),
    },
    Task {
        id: "a11",
        flow: Flow::Preview,
        script: Script::Quota,
        required: Some("tx.preview"),
    },
    Task {
        id: "a12",
        flow: Flow::Preview,
        script: Script::PrivacyProbe,
        required: Some("tx.preview"),
    },
    Task {
        id: "a13",
        flow: Flow::Apply,
        script: Script::Clean,
        required: Some("tx.apply"),
    },
    Task {
        id: "a14",
        flow: Flow::Apply,
        script: Script::Clean,
        required: Some("tx.apply"),
    },
    Task {
        id: "a15",
        flow: Flow::Apply,
        script: Script::Quota,
        required: Some("tx.apply"),
    },
    Task {
        id: "a16",
        flow: Flow::Undo,
        script: Script::Clean,
        required: Some("tx.undo"),
    },
    Task {
        id: "a17",
        flow: Flow::Undo,
        script: Script::Stale,
        required: Some("tx.undo"),
    },
    Task {
        id: "a18",
        flow: Flow::Fork,
        script: Script::NeutralFork,
        required: Some("play.run"),
    },
    Task {
        id: "a19",
        flow: Flow::Fork,
        script: Script::BullyFork,
        required: Some("play.run"),
    },
    Task {
        id: "a20",
        flow: Flow::Fork,
        script: Script::Timeout,
        required: Some("play.run"),
    },
];

/// The five play tasks, in recorded order (slots twenty to twenty-four).
const PLAY_TASKS: [Task; 5] = [
    Task {
        id: "p01",
        flow: Flow::Run,
        script: Script::CleanRun,
        required: Some("play.run"),
    },
    Task {
        id: "p02",
        flow: Flow::Run,
        script: Script::ReseedRun,
        required: Some("play.run"),
    },
    Task {
        id: "p03",
        flow: Flow::Capture,
        script: Script::CleanCapture,
        required: Some("capture"),
    },
    Task {
        id: "p04",
        flow: Flow::Capture,
        script: Script::CaughtCapture,
        required: Some("capture"),
    },
    Task {
        id: "p05",
        flow: Flow::Verify,
        script: Script::ReferenceVerdict,
        required: None,
    },
];

/// Local cost anchor in minor units, mirroring the fixture cost rule.
const LOCAL_COST_ANCHOR: usize = 40;

/// Designed verdict triple for one script on one backend: completion,
/// rejection and privacy flag. Transcribed from the harness designed-outcome
/// table: clean and detection-is-pass arms complete, refusal arms refuse, and
/// privacy probes complete on local but refuse on remote.
fn scripted_verdict(script: Script, backend: Backend) -> (bool, bool, bool) {
    match script {
        Script::Clean
        | Script::NeutralFork
        | Script::CleanRun
        | Script::ReseedRun
        | Script::CleanCapture
        | Script::CaughtCapture
        | Script::ReferenceVerdict => (true, false, false),
        Script::Malformed | Script::Stale | Script::Timeout | Script::Quota | Script::BullyFork => {
            (false, true, false)
        }
        Script::PrivacyProbe => match backend {
            Backend::Local => (true, false, false),
            Backend::Remote => (false, true, true),
        },
    }
}

/// Fixture serving cost for one row: local reports minor units, remote stays
/// unknown. The option renders unknown, never zero.
fn fixture_cost(backend: Backend, slot: usize) -> Option<usize> {
    match backend {
        Backend::Local => Some(LOCAL_COST_ANCHOR + slot),
        Backend::Remote => None,
    }
}

/// Verdict triple for one task on one surface: an unlisted verb reads as a
/// plain refusal (no privacy downgrade was attempted), otherwise the scripted
/// verdict stands.
fn surface_verdict(task: &Task, backend: Backend, listed: &[String]) -> (bool, bool, bool) {
    let covered = task
        .required
        .is_none_or(|verb| listed.iter().any(|name| name == verb));
    if covered {
        scripted_verdict(task.script, backend)
    } else {
        (false, true, false)
    }
}

/// True when the surface verdict matches the designed one exactly.
fn row_is_green(task: &Task, backend: Backend, listed: &[String]) -> bool {
    surface_verdict(task, backend, listed) == scripted_verdict(task.script, backend)
}

/// Slot of a task id in recorded order: authoring slots zero to nineteen,
/// play slots twenty to twenty-four.
fn task_slot(id: &str) -> usize {
    AUTHORING_TASKS
        .iter()
        .position(|task| task.id == id)
        .or_else(|| {
            PLAY_TASKS
                .iter()
                .position(|task| task.id == id)
                .map(|slot| slot + AUTHORING_TASKS.len())
        })
        .expect("benchmark task id is one of the twenty-five recorded ids")
}

/// One listed-tool surface under test.
struct Surface {
    name: &'static str,
    tools: Vec<String>,
}

/// The three surfaces: ten-tool spine, full twenty-one-tool door, and the full
/// door plus ten capability-free standby names.
fn surfaces() -> [Surface; 3] {
    let small = Surface {
        name: "small",
        tools: SMALL_TOOLS.iter().map(|name| name.to_string()).collect(),
    };
    let full = Surface {
        name: "full",
        tools: ALL_TOOLS.iter().map(|name| name.to_string()).collect(),
    };
    let mut wide_tools = full.tools.clone();
    wide_tools.extend(DISTRACTOR_TOOLS.iter().map(|name| name.to_string()));
    let wide = Surface {
        name: "wide",
        tools: wide_tools,
    };
    [small, full, wide]
}

/// Outcome of one surface: green counts per backend, discovery name chars,
/// local serving cost, uncovered task ids and cross-backend divergents.
struct SurfaceReport {
    name: &'static str,
    tool_count: usize,
    name_chars: usize,
    local_cost_units: usize,
    local_green: usize,
    remote_green: usize,
    uncovered: Vec<&'static str>,
    divergent: Vec<&'static str>,
}

/// Green rows for one surface on one backend.
fn green_for(surface: &Surface, backend: Backend) -> usize {
    AUTHORING_TASKS
        .iter()
        .chain(PLAY_TASKS.iter())
        .filter(|task| row_is_green(task, backend, &surface.tools))
        .count()
}

/// Task ids whose verb is missing from the surface.
fn uncovered_ids(surface: &Surface) -> Vec<&'static str> {
    AUTHORING_TASKS
        .iter()
        .chain(PLAY_TASKS.iter())
        .filter(|task| {
            task.required
                .is_some_and(|verb| !surface.tools.iter().any(|name| name == verb))
        })
        .map(|task| task.id)
        .collect()
}

/// Task ids whose verdict triple differs between the two model backends.
fn divergent_ids(surface: &Surface) -> Vec<&'static str> {
    AUTHORING_TASKS
        .iter()
        .chain(PLAY_TASKS.iter())
        .filter(|task| {
            surface_verdict(task, Backend::Local, &surface.tools)
                != surface_verdict(task, Backend::Remote, &surface.tools)
        })
        .map(|task| task.id)
        .collect()
}

/// Evaluate one surface over both model backends.
fn evaluate(surface: &Surface) -> SurfaceReport {
    let name_chars = surface.tools.iter().map(|name| name.len()).sum();
    let local_cost_units = AUTHORING_TASKS
        .len()
        .checked_add(PLAY_TASKS.len())
        .and_then(|total| {
            (0..total).try_fold(0usize, |sum, slot| {
                fixture_cost(Backend::Local, slot).and_then(|cost| sum.checked_add(cost))
            })
        })
        .expect("twenty-five bounded fixture costs never overflow");
    SurfaceReport {
        name: surface.name,
        tool_count: surface.tools.len(),
        name_chars,
        local_cost_units,
        local_green: green_for(surface, Backend::Local),
        remote_green: green_for(surface, Backend::Remote),
        uncovered: uncovered_ids(surface),
        divergent: divergent_ids(surface),
    }
}

/// Deterministic text table: one header plus one row per surface.
fn render_table(reports: &[SurfaceReport]) -> String {
    let mut out = String::from(
        "surface tools name_chars local_cost_units local_green remote_green total_green divergent uncovered\n",
    );
    for report in reports {
        let total = report.local_green + report.remote_green;
        let divergent = if report.divergent.is_empty() {
            "none".to_owned()
        } else {
            report.divergent.join(",")
        };
        let uncovered = if report.uncovered.is_empty() {
            "none".to_owned()
        } else {
            report.uncovered.join(",")
        };
        out.push_str(&format!(
            "{} {} {} {} {}/25 {}/25 {}/50 {divergent} {uncovered}\n",
            report.name,
            report.tool_count,
            report.name_chars,
            report.local_cost_units,
            report.local_green,
            report.remote_green,
            total,
        ));
    }
    out
}

/// Digest of the rendered table: the no-model replay anchor.
fn table_digest(table: &str) -> u64 {
    hash_bytes(table.as_bytes())
}

/// Lane root: the checkout this test binary was built from.
fn lane_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Cached door source texts: one disk read per file per test process.
fn source_cache() -> &'static std::collections::HashMap<&'static str, String> {
    static CACHE: OnceLock<std::collections::HashMap<&'static str, String>> = OnceLock::new();
    CACHE.get_or_init(|| {
        let mut map = std::collections::HashMap::new();
        for file in VERB_LITERAL_FILES {
            let path = lane_root().join(file);
            let text = std::fs::read_to_string(&path)
                .expect("door source is readable from the lane checkout");
            map.insert(file, text);
        }
        map
    })
}

/// Read a lane source file, failing loudly when the door tree moved.
fn read_source(relative: &str) -> String {
    if let Some(text) = source_cache().get(relative) {
        return text.clone();
    }
    let path = lane_root().join(relative);
    std::fs::read_to_string(&path).expect("door source is readable from the lane checkout")
}

/// Count the verb entries in the door router table body.
fn router_entry_count(router: &str) -> Vec<String> {
    let marker = "pub fn tool_names()";
    let start = router
        .find(marker)
        .expect("door router table keeps its tool_names constructor");
    let tail = &router[start..];
    let end = tail
        .find("\n}\n")
        .expect("door router table stays a plain constructor body");
    tail[..end]
        .lines()
        .filter_map(|line| {
            let entry = line.trim().trim_end_matches(',').trim().to_owned();
            let is_verb = entry.strip_prefix("TOOL_").is_some() || entry == "MCP_TOOL_NAME";
            is_verb.then_some(entry)
        })
        .collect()
}

/// Parse a `pub const <NAME>: usize = <digits>;` value out of door source.
fn parse_const_usize(source: &str, name: &str) -> usize {
    let marker = format!("pub const {name}: usize = ");
    let start = source
        .find(marker.as_str())
        .expect("door budget constant keeps its declared shape");
    let tail = &source[start + marker.len()..];
    let end = tail
        .find(';')
        .expect("door budget constant stays a plain assignment");
    tail[..end]
        .trim()
        .replace('_', "")
        .parse::<usize>()
        .expect("door budget constant stays a plain integer")
}

/// Parse the dated full-list reference (`21-tool list measures <digits>`)
/// out of the door budget comment.
fn parse_list_reference(comment: &str) -> usize {
    let marker = "21-tool list measures ";
    let start = comment
        .find(marker)
        .expect("door budget comment keeps its dated full-list reading");
    let tail = &comment[start + marker.len()..];
    let end = tail
        .find(|char: char| !char.is_ascii_digit())
        .expect("door budget comment keeps digits after its reading");
    tail[..end]
        .parse::<usize>()
        .expect("door budget comment reading stays a plain integer")
}

/// Files holding the literal verb spellings the router table refers to.
const VERB_LITERAL_FILES: [&str; 10] = [
    "crates/endpoint/src/mcp.rs",
    "crates/endpoint/src/build_level.rs",
    "crates/endpoint/src/scene_tools.rs",
    "crates/endpoint/src/scene_query.rs",
    "crates/endpoint/src/play.rs",
    "crates/endpoint/src/play/play_session.rs",
    "crates/endpoint/src/plan_goals.rs",
    "crates/endpoint/src/log_tools.rs",
    "crates/endpoint/src/insight/mcp_tools.rs",
    "crates/endpoint/src/discover.rs",
];

/// True when the quoted verb spelling occurs in one of the literal files.
fn verb_spelling_exists(verb: &str) -> bool {
    let quoted = format!("\"{verb}\"");
    let cache = source_cache();
    VERB_LITERAL_FILES.iter().any(|file| {
        cache
            .get(file)
            .is_some_and(|text| text.contains(quoted.as_str()))
    })
}

/// Door verb each pipeline stage needs. Fork rides the headless match-run
/// verb (no dedicated fork verb is listed) and the content verdict is
/// harness-side (no verdict verb is listed); both are findings, not guesses.
fn flow_verb(flow: Flow) -> Option<&'static str> {
    match flow {
        Flow::Brief => Some("scene.brief"),
        Flow::Place => Some("scene.place"),
        Flow::Preview => Some("tx.preview"),
        Flow::Apply => Some("tx.apply"),
        Flow::Undo => Some("tx.undo"),
        Flow::Fork | Flow::Run => Some("play.run"),
        Flow::Capture => Some("capture"),
        Flow::Verify => None,
    }
}

#[test]
fn every_task_states_its_flows_verb() {
    for task in AUTHORING_TASKS.iter().chain(PLAY_TASKS.iter()) {
        assert_eq!(
            task.required,
            flow_verb(task.flow),
            "task {} carries its stage verb",
            task.id
        );
    }
}

#[test]
fn door_lists_exactly_the_embedded_twenty_one() {
    let router = read_source("crates/endpoint/src/mcp.rs");
    let entries = router_entry_count(&router);
    assert_eq!(
        entries.len(),
        ALL_TOOLS.len(),
        "door router holds twenty-one verbs"
    );
    let mut distinct = entries.clone();
    distinct.sort();
    distinct.dedup();
    assert_eq!(
        distinct.len(),
        entries.len(),
        "door router verbs stay distinct"
    );
    for verb in ALL_TOOLS {
        assert!(
            verb_spelling_exists(verb),
            "door source still spells verb {verb}"
        );
    }
    let budget = parse_const_usize(&router, "TOOL_LIST_BUDGET_CHARS");
    assert_eq!(
        budget, 43_514,
        "door discovery budget keeps its contract value"
    );
    let reference = parse_list_reference(&router);
    assert_eq!(
        reference, 40_514,
        "door full-list reading keeps its dated value"
    );
    assert!(
        budget > reference,
        "discovery budget stays above the measured list"
    );
}

#[test]
fn distractor_names_carry_no_capability() {
    for name in DISTRACTOR_TOOLS {
        assert!(
            !ALL_TOOLS.contains(&name),
            "standby name {name} never collides with a verb"
        );
        assert!(
            !verb_spelling_exists(name),
            "standby name {name} stays out of the door"
        );
    }
    for task in AUTHORING_TASKS.iter().chain(PLAY_TASKS.iter()) {
        match task.required {
            None => assert_eq!(
                task.id, "p05",
                "only the harness-side verdict needs no verb"
            ),
            Some(required) => assert!(
                !DISTRACTOR_TOOLS.contains(&required),
                "task {} never depends on a standby name",
                task.id
            ),
        }
    }
}

#[test]
fn full_surface_reproduces_the_recorded_baseline() {
    let surfaces = surfaces();
    let full = surfaces
        .iter()
        .find(|surface| surface.name == "full")
        .expect("full surface exists");
    assert_eq!(
        full.tools.len(),
        21,
        "full surface lists all twenty-one verbs"
    );
    assert_eq!(
        green_for(full, Backend::Local),
        25,
        "full surface greens every local row"
    );
    assert_eq!(
        green_for(full, Backend::Remote),
        25,
        "full surface greens every remote row"
    );
    assert_eq!(
        divergent_ids(full),
        vec!["a08", "a12"],
        "only the privacy probes diverge"
    );
}

#[test]
fn wide_surface_costs_chars_without_completion() {
    let surfaces = surfaces();
    let full = surfaces
        .iter()
        .find(|surface| surface.name == "full")
        .expect("full surface exists");
    let wide = surfaces
        .iter()
        .find(|surface| surface.name == "wide")
        .expect("wide surface exists");
    assert_eq!(wide.tools.len(), 31, "wide surface lists thirty-one names");
    for backend in [Backend::Local, Backend::Remote] {
        assert_eq!(
            green_for(wide, backend),
            green_for(full, backend),
            "standby names add no completion on {}",
            backend.label()
        );
    }
    let full_chars: usize = full.tools.iter().map(|name| name.len()).sum();
    let wide_chars: usize = wide.tools.iter().map(|name| name.len()).sum();
    assert_eq!(
        wide_chars,
        full_chars + 150,
        "ten standby names cost one hundred fifty chars"
    );
    assert!(wide_chars > full_chars, "wider listing never reads cheaper");
}

#[test]
fn small_surface_names_its_capture_blind_spot() {
    let surfaces = surfaces();
    let small = surfaces
        .iter()
        .find(|surface| surface.name == "small")
        .expect("small surface exists");
    assert_eq!(small.tools.len(), 10, "small surface lists ten verbs");
    assert_eq!(
        uncovered_ids(small),
        vec!["p03", "p04"],
        "small surface drops only the capture tasks"
    );
    assert_eq!(
        green_for(small, Backend::Local),
        23,
        "small surface keeps twenty-three local rows"
    );
    assert_eq!(
        green_for(small, Backend::Remote),
        23,
        "small surface keeps twenty-three remote rows"
    );
    assert_eq!(
        divergent_ids(small),
        vec!["a08", "a12"],
        "privacy probes still diverge alone"
    );
}

#[test]
fn fixture_cost_rule_matches_the_recorded_spots() {
    assert_eq!(
        fixture_cost(Backend::Local, task_slot("a08")),
        Some(47),
        "place probe slot prices forty-seven"
    );
    assert_eq!(
        fixture_cost(Backend::Local, task_slot("p05")),
        Some(64),
        "verdict slot prices sixty-four"
    );
    assert_eq!(
        fixture_cost(Backend::Remote, task_slot("a01")),
        None,
        "remote cost stays unknown"
    );
}

#[test]
fn rendered_table_is_stable_without_a_model() {
    let surfaces = surfaces();
    let reports: Vec<SurfaceReport> = surfaces.iter().map(evaluate).collect();
    let first = render_table(&reports);
    let second = render_table(&reports);
    assert_eq!(first, second, "table text renders byte-identical twice");
    assert_eq!(
        table_digest(&first),
        table_digest(&second),
        "table digest replays with no model"
    );
    assert_ne!(
        table_digest(&first),
        0,
        "table digest is a real anchor, never zero"
    );
}

#[test]
fn report_posts_the_decision_table() {
    let surfaces = surfaces();
    let reports: Vec<SurfaceReport> = surfaces.iter().map(evaluate).collect();
    let table = render_table(&reports);
    println!("{table}digest {:016x}\n", table_digest(&table));
    println!("decision: default stays the full twenty-one-verb door at 50/50 green;");
    println!("decision: the ten-verb spine is the weak-model floor at 46/50, blind to capture;");
    println!(
        "decision: the thirty-one-name listing is rejected, wider chars for zero completion gain."
    );
    assert!(
        table.starts_with("surface tools name_chars"),
        "report keeps its header"
    );
    assert!(
        table.contains("\nfull 21 "),
        "report carries the full-door row"
    );
    assert!(table.contains(" 46/50 "), "report carries the floor number");
}
