//! Mirror slice, game-development shape: ten tasks in the style of a public
//! benchmark of tutorial-derived game-development tasks (gameplay edits and
//! graphics checks, a success rate per task, visual feedback as a lever).
//!
//! Consumer: the 30-task aggregate. Owner: this file owns the ten tasks and
//! the published figure; shapes and scoring rules only, no suite task text or
//! assets. Every task submits through the eval door, then a deterministic
//! scene query or frame check decides; no model is called. Gameplay tasks
//! edit the shared scene (positions, labels); graphics tasks run the frame
//! checks, with a feedback loop that retries once from the named fault.
//! Units: centimeters on the wire, meters in the scene. Determinism: every
//! outcome is a pure function of the recorded submission.

use engine_provider::protocol::{AiError, ProposalAction};

use crate::door::{
    block_prop, fixture_scene, set_label, set_position, Delivery, Door, Scene, Submit,
};
use crate::frame_check::{check_scripted, FrameFault, FrameMetrics, Script, Thresholds};
use crate::mirror_kit::{
    admit, ck, point_symmetric, run_slice, scene_outcome, sealed_scene, text_hash, Claim,
    MirrorTask, Outcome, SliceReport, Submission, SuiteInfo,
};

/// First eval fixture slot of this slice.
const BASE_INDEX: u64 = 0;
/// Capture edge in pixels for the graphics tasks.
const CAPTURE_PX: u32 = 64;
/// Capture seed shared by the graphics tasks.
const CAPTURE_SEED: u64 = 2040;

/// The suite this slice mirrors. The figures are the suite's own, quoted
/// from its abstract page (v2, revised 2026-06-30, read 2026-10-10); rival
/// MCP doors were not run on these tasks and stay unknown.
pub static SUITE: SuiteInfo = SuiteInfo {
    id: "gamedevbench",
    name: "GameDevBench",
    published: Claim::Quoted {
        text: "333 tasks; best agent and method 53.8%; GPT-5.4 41.1% -> 52.0% with visual feedback",
        source: "https://arxiv.org/abs/2602.11103",
    },
    rivals: [Claim::Unknown; 3],
};

/// The ten tasks, in report order.
pub(crate) const TASKS: [MirrorTask; 10] = [
    MirrorTask {
        id: "gdb01",
        category: "gameplay",
        shape: "scene edit: put a prop beside a wall",
        run: place_beside_wall,
    },
    MirrorTask {
        id: "gdb02",
        category: "gameplay",
        shape: "scene edit: open a sealed path",
        run: open_sealed_path,
    },
    MirrorTask {
        id: "gdb03",
        category: "gameplay",
        shape: "scene edit: restore a symmetric layout",
        run: restore_symmetry,
    },
    MirrorTask {
        id: "gdb04",
        category: "gameplay",
        shape: "scene edit: rename with unique names",
        run: rename_unique,
    },
    MirrorTask {
        id: "gdb05",
        category: "gameplay",
        shape: "scene edit: clear an overlap",
        run: clear_overlap,
    },
    MirrorTask {
        id: "gdb06",
        category: "gameplay",
        shape: "door: a stale concurrent edit lands nowhere",
        run: concurrent_edit,
    },
    MirrorTask {
        id: "gdb07",
        category: "graphics",
        shape: "capture: the frame is lit, not black",
        run: lit_capture,
    },
    MirrorTask {
        id: "gdb08",
        category: "graphics",
        shape: "capture: retry once from the named fault",
        run: feedback_loop,
    },
    MirrorTask {
        id: "gdb09",
        category: "graphics",
        shape: "capture: the same seed gives the same frame",
        run: stable_capture,
    },
    MirrorTask {
        id: "gdb10",
        category: "graphics",
        shape: "capture: label the broken prop with its fault",
        run: label_the_fault,
    },
];

/// Run the slice on every backend.
pub fn run() -> SliceReport {
    run_slice(&SUITE, &TASKS, BASE_INDEX, notes)
}

/// Feedback lift count: reference runs on `mock` whose blind attempt failed
/// and whose retry from the named fault passed.
///
/// Owner: this file; consumer: [`notes`] (the slice report line) plus the
/// lift test. Units: tasks (0..=10). Determinism: pure row filter in fixed
/// order; the same report gives the same count on every run.
/// Caller: `notes` in `crates/playtest/src/mirror_gdb.rs`.
pub fn lifted_count(report: &SliceReport) -> usize {
    report
        .rows
        .iter()
        .filter(|row| row.backend == "mock")
        .filter(|row| {
            row.reference.stage("blind") == Some(false)
                && row.reference.stage("feedback") == Some(true)
        })
        .count()
}

/// Fully green tasks: reference solves and weak is rejected on every backend.
///
/// Owner: this file; consumer: [`notes`] (the slice report line) plus the
/// green test. Units: tasks (0..=10). Determinism: pure row filter in fixed
/// TASKS order; the same report gives the same count on every run.
/// Caller: `notes` in `crates/playtest/src/mirror_gdb.rs`.
#[must_use]
pub fn green_count(report: &SliceReport) -> usize {
    TASKS
        .iter()
        .filter(|task| {
            ["mock", "local", "remote"].iter().all(|backend| {
                report
                    .rows
                    .iter()
                    .any(|row| row.task == task.id && row.backend == *backend && row.green())
            })
        })
        .count()
}

/// Feedback lift: reference runs whose first attempt failed and whose retry
/// from the named fault passed.
fn notes(report: &SliceReport) -> Vec<String> {
    let lifted = lifted_count(report);
    let green = green_count(report);
    vec![
        format!("visual feedback: {lifted} task(s) fail blind and pass after one retry"),
        format!("green: {green} task(s) solve with weak rejected on every backend"),
    ]
}

/// Submit one clean batch against the scene's live revision.
fn land(door: &mut Door, scene: &mut Scene, actions: &[ProposalAction]) -> Delivery {
    let base = scene.revision;
    door.submit(scene, &Submit::clean(base, actions))
}

fn place_beside_wall(door: &mut Door, sub: Submission) -> Outcome {
    let mut scene = fixture_scene();
    let z_cm = sub.pick(-1930, -1500);
    let delivery = land(door, &mut scene, &[set_position(20, [-800, 50, z_cm])]);
    let gap_cm = scene.north_gap_m(20).map(|gap| gap * 100.0);
    let beside = gap_cm.is_some_and(|gap| (-0.5..=50.5).contains(&gap));
    let checks = vec![
        ck("applied", delivery == Delivery::Applied(true)),
        ck("beside-wall", beside),
        ck("clear", scene.prop_findings("overlap") == 0),
    ];
    scene_outcome(&scene, &delivery, checks)
}

fn open_sealed_path(door: &mut Door, sub: Submission) -> Outcome {
    let mut scene = sealed_scene();
    let x_cm = sub.pick(1800, 30);
    let delivery = land(door, &mut scene, &[set_position(30, [x_cm, 100, 0])]);
    let checks = vec![
        ck("applied", delivery == Delivery::Applied(true)),
        ck("walkable", scene.walkable()),
        ck("inside", scene.inside_arena()),
    ];
    scene_outcome(&scene, &delivery, checks)
}

fn restore_symmetry(door: &mut Door, sub: Submission) -> Outcome {
    let mut scene = fixture_scene();
    scene.variant.props[3].x = 6.0;
    scene.variant.props[3].z = 9.0;
    let z_cm = sub.pick(800, -800);
    let delivery = land(door, &mut scene, &[set_position(23, [800, 50, z_cm])]);
    let checks = vec![
        ck("applied", delivery == Delivery::Applied(true)),
        ck("symmetric", point_symmetric(&scene)),
        ck("clear", scene.prop_findings("overlap") == 0),
    ];
    scene_outcome(&scene, &delivery, checks)
}

fn rename_unique(door: &mut Door, sub: Submission) -> Outcome {
    let mut scene = fixture_scene();
    scene.variant.props[2].name = "torch_sw".to_owned();
    let label = sub.pick("torch_ne", "torch_sw");
    let delivery = land(door, &mut scene, &[set_label(21, label)]);
    let mut names: Vec<&str> = scene
        .variant
        .props
        .iter()
        .map(|prop| prop.name.as_str())
        .collect();
    names.sort_unstable();
    let total = names.len();
    names.dedup();
    let renamed = scene
        .prop(21)
        .is_some_and(|prop| prop.name.starts_with("torch_"));
    let checks = vec![
        ck("applied", delivery == Delivery::Applied(true)),
        ck("renamed", renamed),
        ck("unique-names", names.len() == total),
    ];
    scene_outcome(&scene, &delivery, checks)
}

fn clear_overlap(door: &mut Door, sub: Submission) -> Outcome {
    let mut scene = fixture_scene();
    scene
        .variant
        .props
        .push(block_prop(31, "crate", [-8.2, -8.1], [1.0, 1.0, 1.0]));
    let to = sub.pick([-300, 50, 300], [800, 50, -800]);
    let delivery = land(door, &mut scene, &[set_position(31, to)]);
    let checks = vec![
        ck("applied", delivery == Delivery::Applied(true)),
        ck("no-overlap", scene.prop_findings("overlap") == 0),
        ck("inside", scene.inside_arena()),
    ];
    scene_outcome(&scene, &delivery, checks)
}

fn concurrent_edit(door: &mut Door, sub: Submission) -> Outcome {
    let mut scene = fixture_scene();
    let first = [set_position(20, [-800, 50, -1930])];
    let second = [set_position(20, [0, 50, 0])];
    let stale_base = scene.revision;
    let landed = land(door, &mut scene, &first);
    let rebased = sub.pick(stale_base, scene.revision);
    let late = door.submit(&mut scene, &Submit::clean(rebased, &second));
    let oracle = fixture_scene().direct(&first);
    let checks = vec![
        ck("first-landed", landed == Delivery::Applied(true)),
        ck(
            "stale-refused",
            late == Delivery::Refused(AiError::StaleContext),
        ),
        ck(
            "hash-matches-single-edit",
            oracle.is_some_and(|only| only.hash() == scene.hash()),
        ),
    ];
    let detail = format!("first {} second {}", landed.label(), late.label());
    Outcome::from_checks(checks, scene.hash(), detail)
}

/// The frame check at the slice's fixed size and seed.
fn capture(script: Script, seed: u64) -> Result<FrameMetrics, FrameFault> {
    check_scripted(script, CAPTURE_PX, CAPTURE_PX, seed, &Thresholds::default())
}

/// Stable fault label for details and annotations.
fn fault_name(fault: &FrameFault) -> &'static str {
    match fault {
        FrameFault::BlackScreen(_) => "black-screen",
        FrameFault::WhiteScreen(_) => "white-screen",
        FrameFault::MissingMesh(_) => "missing-mesh",
        FrameFault::Clipping(_) => "clipping",
        FrameFault::Frozen(_) => "frozen",
        FrameFault::BadCapture(_) => "bad-capture",
    }
}

/// The script that repairs a named fault, if the fault is repairable.
fn repair_for(fault: &FrameFault) -> Option<Script> {
    match fault {
        FrameFault::BlackScreen(_)
        | FrameFault::WhiteScreen(_)
        | FrameFault::MissingMesh(_)
        | FrameFault::Clipping(_) => Some(Script::Clean),
        FrameFault::Frozen(_) | FrameFault::BadCapture(_) => None,
    }
}

fn lit_capture(door: &mut Door, sub: Submission) -> Outcome {
    let admitted = admit(door, "lit-capture");
    let script = sub.pick(Script::Clean, Script::Black);
    let shot = capture(script, CAPTURE_SEED);
    let (ok, hash, detail) = match &shot {
        Ok(metrics) => (true, text_hash(&metrics.digest), "capture clean".to_owned()),
        Err(fault) => (
            false,
            text_hash(&fault.to_string()),
            fault_name(fault).to_owned(),
        ),
    };
    Outcome::from_checks(vec![admitted, ck("lit", ok)], hash, detail)
}

fn feedback_loop(door: &mut Door, sub: Submission) -> Outcome {
    let admitted = admit(door, "feedback-loop");
    let blind_script = sub.pick(Script::Black, Script::White);
    let blind = capture(blind_script, CAPTURE_SEED);
    let Err(fault) = blind else {
        return Outcome::decided(
            false,
            vec![admitted, ck("blind", true)],
            0,
            "no fault to learn from".to_owned(),
        );
    };
    let retry = sub.pick(repair_for(&fault), None);
    let (feedback, hash) = match retry.map(|script| capture(script, CAPTURE_SEED)) {
        Some(Ok(metrics)) => (true, text_hash(&metrics.digest)),
        _ => (false, text_hash(&fault.to_string())),
    };
    let detail = format!(
        "blind {} then {}",
        fault_name(&fault),
        if feedback { "repaired" } else { "gave up" }
    );
    let mut checks = vec![admitted, ck("blind", false)];
    if retry.is_some() {
        checks.push(ck("feedback", feedback));
    }
    Outcome::decided(feedback, checks, hash, detail)
}

fn stable_capture(door: &mut Door, sub: Submission) -> Outcome {
    let admitted = admit(door, "stable-capture");
    // The mutant's seed must change the frame: the clean capture's box jitter
    // rounds to the same origin for CAPTURE_SEED and CAPTURE_SEED + 1, so
    // the mutant reruns at + 2.
    let second_seed = sub.pick(CAPTURE_SEED, CAPTURE_SEED + 2);
    let first = capture(Script::Clean, CAPTURE_SEED).map(|metrics| metrics.digest);
    let second = capture(Script::Clean, second_seed).map(|metrics| metrics.digest);
    let stable = matches!((&first, &second), (Ok(a), Ok(b)) if a == b);
    let hash = first.as_deref().map_or(0, text_hash);
    let detail = if stable {
        "digest stable"
    } else {
        "digest drifted"
    };
    Outcome::from_checks(
        vec![admitted, ck("stable", stable)],
        hash,
        detail.to_owned(),
    )
}

fn label_the_fault(door: &mut Door, sub: Submission) -> Outcome {
    let mut scene = fixture_scene();
    let truth = capture(Script::MissingMesh, CAPTURE_SEED)
        .err()
        .map_or("none", |fault| fault_name(&fault));
    let claim = sub.pick(truth, "black-screen");
    let label = format!("fault:{claim}");
    let delivery = land(door, &mut scene, &[set_label(20, &label)]);
    let named = scene
        .prop(20)
        .is_some_and(|prop| prop.name == format!("fault:{truth}"));
    let checks = vec![
        ck("applied", delivery == Delivery::Applied(true)),
        ck("fault-seen", truth == "missing-mesh"),
        ck("named", named),
    ];
    scene_outcome(&scene, &delivery, checks)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_slice_has_ten_tasks_in_two_categories() {
        let report = run();
        assert_eq!(report.tasks(), 10);
        assert_eq!(report.rows.len(), 30);
        let split = report.by_category("mock");
        assert_eq!(split, [("gameplay", 6, 6), ("graphics", 4, 4)]);
    }

    #[test]
    fn the_slice_reproduces_and_matches_across_backends() {
        let first = run();
        assert_eq!(first.to_text(), run().to_text());
        let unsolved: Vec<(&str, &str)> = first
            .rows
            .iter()
            .filter(|row| !row.reference.pass)
            .map(|row| (row.task, row.backend))
            .collect();
        assert_eq!(
            unsolved,
            Vec::new(),
            "reference submissions that do not solve"
        );
        let accepted: Vec<(&str, &str)> = first
            .rows
            .iter()
            .filter(|row| row.weak.pass)
            .map(|row| (row.task, row.backend))
            .collect();
        assert_eq!(accepted, Vec::new(), "weak submissions the checker accepts");
        assert_eq!(first.parity(), (10, Vec::new()));
        for backend in ["mock", "local", "remote"] {
            assert_eq!(first.solved(backend), 10, "{backend}");
            assert_eq!(first.caught(backend), 10, "{backend}");
        }
    }

    #[test]
    fn slice_report_text_names_published_figure() {
        let report = run();
        let text = report.to_text();
        assert!(text.contains(SUITE.name), "report names its suite: {text}");
        assert!(
            text.contains("333 tasks"),
            "report names its published figure: {text}"
        );
        assert!(
            text.contains("published:"),
            "report prints its published line: {text}"
        );
        let mut ids: Vec<&str> = TASKS.iter().map(|task| task.id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), 10, "ten unique task ids");
        assert!(
            SUITE.rivals.iter().all(|claim| claim.is_unknown()),
            "mirrored rivals stay unknown"
        );
    }

    #[test]
    fn lifted_count_is_one_and_notes_names_it_x2() {
        let first = run();
        let lifted = super::lifted_count(&first);
        assert_eq!(lifted, 1, "one task lifts blind-fail to feedback-pass");
        let text = first.to_text();
        assert!(
            text.contains("visual feedback: 1 task(s)"),
            "notes names the lift: {text}"
        );
        let second = run();
        assert_eq!(second.to_text(), text, "report repeats x2");
        assert_eq!(super::lifted_count(&second), lifted, "lift repeats");
    }

    #[test]
    fn green_count_is_ten_and_notes_names_it_x2() {
        let first = run();
        let green = super::green_count(&first);
        assert_eq!(green, 10, "ten tasks green on every backend");
        let text = first.to_text();
        assert!(
            text.contains("green: 10 task(s)"),
            "notes names the green: {text}"
        );
        let second = run();
        assert_eq!(second.to_text(), text, "report repeats x2");
        assert_eq!(super::green_count(&second), green, "green repeats");
    }
}
