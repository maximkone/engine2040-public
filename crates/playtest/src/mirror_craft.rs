//! Mirror slice, engine-build shape: ten tasks in the style of a public
//! benchmark that builds gameplay in a real engine and scores each submission
//! through deterministic stages: it builds, its assets are well formed, its
//! runtime assertions hold. No model judge, no task text, no assets.
//!
//! Consumer: the 30-task aggregate. Owner: this file owns the ten specs and
//! the published figure. A submission clears stages in order and stops at the
//! first failure: `on-time` (the door answered inside its budget), `build`
//! (the door admitted it and the scene applied it whole), `asset` (structure
//! only: the named things exist) and `runtime` (behavior: walkable, symmetric,
//! overlap-free). Deliverable forms mirror the suite's split: direct edits,
//! labeled assets and batch scripts; paired tasks share one runtime test.
//! Determinism: every outcome is a pure function of the recorded submission.

use engine_provider::protocol::ProposalAction;

use crate::door::{
    block_prop, fixture_scene, set_label, set_position, Delivery, Door, Scene, Submit,
};
use crate::eval::TaskScript;
use crate::mirror_kit::{
    ck, point_symmetric, run_slice, scene_outcome, sealed_scene, Check, Claim, MirrorTask, Outcome,
    SliceReport, Submission, SuiteInfo,
};

/// First eval fixture slot of this slice.
const BASE_INDEX: u64 = 10;

/// The suite this slice mirrors. The figures are the suite's own, quoted
/// from its abstract page (v1, submitted 2026-09-19, read 2026-10-10); rival
/// MCP doors were not run on these tasks and stay unknown.
pub static SUITE: SuiteInfo = SuiteInfo {
    id: "craftbench-ue",
    name: "CraftBench-UE",
    published: Claim::Quoted {
        text: "70 tasks; C++ completion above Blueprint by 30.0 and 42.9 points in two tool setups; 42.2% and 50.0% of asset-passing on-time paired Blueprint submissions fail runtime assertions",
        source: "https://arxiv.org/abs/2609.23142",
    },
    rivals: [Claim::Unknown; 3],
};

/// How the weak submission reaches the door.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Mode {
    /// Weak content on a clean call.
    Normal,
    /// The provider times out: a late submission.
    Late,
    /// A peer lands first; the weak client submits against the old revision.
    Stale,
}

/// One task: fixture, two recorded deliverables and the two behavioral stages.
struct Spec {
    scene: fn() -> Scene,
    reference: fn() -> Vec<ProposalAction>,
    weak: fn() -> Vec<ProposalAction>,
    asset: fn(&Scene) -> bool,
    runtime: fn(&Scene) -> bool,
    mode: Mode,
}

/// Stage list that stops at the first failure, like the suite's pipeline.
struct Stages {
    checks: Vec<Check>,
    alive: bool,
}

impl Stages {
    fn new() -> Self {
        Self {
            checks: Vec::new(),
            alive: true,
        }
    }

    fn run(&mut self, name: &'static str, held: impl FnOnce() -> bool) -> &mut Self {
        if self.alive {
            let ok = held();
            self.checks.push(ck(name, ok));
            self.alive = ok;
        }
        self
    }
}

/// Peer client that lands first in [`Mode::Stale`] runs.
fn peer_edit() -> Vec<ProposalAction> {
    vec![set_label(23, "peer_edit")]
}

/// Submit the deliverable through the door, then clear the stages in order.
fn run_spec(spec: &Spec, door: &mut Door, sub: Submission) -> Outcome {
    let mut scene = (spec.scene)();
    let actions = sub.pick((spec.reference)(), (spec.weak)());
    let weak = sub == Submission::Weak;
    let mut base = scene.revision;
    if spec.mode == Mode::Stale {
        let peer = peer_edit();
        let landed = door.submit(&mut scene, &Submit::clean(base, &peer));
        debug_assert_eq!(landed, Delivery::Applied(true), "the peer edit must land");
        if !weak {
            base = scene.revision;
        }
    }
    let script = if weak && spec.mode == Mode::Late {
        TaskScript::Timeout
    } else {
        TaskScript::Clean
    };
    let submit = Submit {
        script,
        ..Submit::clean(base, &actions)
    };
    let delivery = door.submit(&mut scene, &submit);
    let mut stages = Stages::new();
    stages
        .run("on-time", || delivery != Delivery::Fallback("timeout"))
        .run("build", || delivery == Delivery::Applied(true))
        .run("asset", || (spec.asset)(&scene))
        .run("runtime", || (spec.runtime)(&scene));
    scene_outcome(&scene, &delivery, stages.checks)
}

// ---- fixtures and stage tests ------------------------------------------

/// Brazier 23 knocked off its symmetric spot.
fn lopsided() -> Scene {
    let mut scene = fixture_scene();
    scene.variant.props[3].x = 6.0;
    scene.variant.props[3].z = 9.0;
    scene
}

/// A crate overlapping brazier 20.
fn cluttered() -> Scene {
    let mut scene = fixture_scene();
    scene
        .variant
        .props
        .push(block_prop(31, "crate", [-8.2, -8.1], [1.0, 1.0, 1.0]));
    scene
}

fn walkable(scene: &Scene) -> bool {
    scene.walkable()
}

fn tidy(scene: &Scene) -> bool {
    scene.prop_findings("overlap") == 0 && scene.inside_arena()
}

fn named(scene: &Scene, id: u32, name: &str) -> bool {
    scene.prop(id).is_some_and(|prop| prop.name == name)
}

/// Every brazier sits 7.9 to 8.1 m from the center and no two coincide.
fn on_ring(scene: &Scene) -> bool {
    let props = &scene.variant.props;
    let radial = props.iter().all(|prop| {
        let squared = prop.x * prop.x + prop.z * prop.z;
        (62.41..=65.61).contains(&squared)
    });
    let apart = props.iter().enumerate().all(|(index, prop)| {
        props.iter().skip(index + 1).all(|other| {
            let (dx, dz) = (prop.x - other.x, prop.z - other.z);
            dx * dx + dz * dz >= 1.0
        })
    });
    radial && apart
}

// ---- recorded deliverables ---------------------------------------------

fn clear_barrier() -> Vec<ProposalAction> {
    vec![set_position(30, [1800, 100, 0])]
}

fn rename_barrier_only() -> Vec<ProposalAction> {
    vec![set_label(30, "gate_open")]
}

fn rename_and_clear_barrier() -> Vec<ProposalAction> {
    vec![set_label(30, "gate_open"), set_position(30, [1800, 100, 0])]
}

fn restore_pair() -> Vec<ProposalAction> {
    vec![set_position(23, [800, 50, 800])]
}

fn wrong_pair() -> Vec<ProposalAction> {
    vec![set_position(23, [800, 50, -800])]
}

fn ring_labels_and_pair() -> Vec<ProposalAction> {
    let mut actions: Vec<ProposalAction> = (0..4u64)
        .map(|slot| set_label(20 + slot, &format!("ring_{slot}")))
        .collect();
    actions.push(set_position(23, [800, 50, 800]));
    actions
}

fn ring_positions() -> Vec<ProposalAction> {
    vec![
        set_position(20, [-566, 50, -566]),
        set_position(21, [566, 50, -566]),
        set_position(22, [-566, 50, 566]),
        set_position(23, [566, 50, 566]),
    ]
}

fn ring_positions_short() -> Vec<ProposalAction> {
    ring_positions().into_iter().take(3).collect()
}

fn lantern_names() -> Vec<ProposalAction> {
    (0..4u64)
        .map(|slot| set_label(20 + slot, &format!("lantern_{slot}")))
        .collect()
}

fn lantern_names_with_ghost() -> Vec<ProposalAction> {
    let mut actions = lantern_names();
    actions.push(set_label(999, "lantern_ghost"));
    actions
}

fn crate_inside() -> Vec<ProposalAction> {
    vec![set_position(31, [-300, 50, 300])]
}

fn crate_outside() -> Vec<ProposalAction> {
    vec![set_position(31, [2500, 50, 300])]
}

fn crate_named_inside() -> Vec<ProposalAction> {
    vec![set_label(31, "crate_a"), set_position(31, [-300, 50, 300])]
}

fn crate_named_only() -> Vec<ProposalAction> {
    vec![set_label(31, "crate_a")]
}

// ---- the ten specs ------------------------------------------------------

const DIRECT_UNBLOCK: Spec = Spec {
    scene: sealed_scene,
    reference: clear_barrier,
    weak: rename_barrier_only,
    asset: |scene| scene.prop(30).is_some(),
    runtime: walkable,
    mode: Mode::Normal,
};

const ASSET_UNBLOCK: Spec = Spec {
    scene: sealed_scene,
    reference: rename_and_clear_barrier,
    weak: rename_barrier_only,
    asset: |scene| named(scene, 30, "gate_open"),
    runtime: walkable,
    mode: Mode::Normal,
};

const DIRECT_SYMMETRY: Spec = Spec {
    scene: lopsided,
    reference: restore_pair,
    weak: wrong_pair,
    asset: |scene| scene.variant.props.len() == 4,
    runtime: point_symmetric,
    mode: Mode::Normal,
};

const ASSET_SYMMETRY: Spec = Spec {
    scene: lopsided,
    reference: ring_labels_and_pair,
    weak: restore_pair,
    asset: |scene| (0..4u32).all(|slot| named(scene, 20 + slot, &format!("ring_{slot}"))),
    runtime: point_symmetric,
    mode: Mode::Normal,
};

const SCRIPT_RING: Spec = Spec {
    scene: fixture_scene,
    reference: ring_positions,
    weak: ring_positions_short,
    asset: |scene| scene.variant.props.len() == 4,
    runtime: on_ring,
    mode: Mode::Normal,
};

const SCRIPT_RENAME_ALL: Spec = Spec {
    scene: fixture_scene,
    reference: lantern_names,
    weak: lantern_names_with_ghost,
    asset: |scene| scene.variant.props.len() == 4,
    runtime: |scene| {
        scene.verified()
            && (0..4u32).all(|slot| named(scene, 20 + slot, &format!("lantern_{slot}")))
    },
    mode: Mode::Normal,
};

const SCRIPT_ON_TIME: Spec = Spec {
    scene: cluttered,
    reference: crate_inside,
    weak: crate_inside,
    asset: |scene| scene.prop(31).is_some(),
    runtime: tidy,
    mode: Mode::Late,
};

const DIRECT_TIDY: Spec = Spec {
    scene: cluttered,
    reference: crate_inside,
    weak: crate_outside,
    asset: |scene| scene.prop(31).is_some(),
    runtime: tidy,
    mode: Mode::Normal,
};

const ASSET_TIDY: Spec = Spec {
    scene: cluttered,
    reference: crate_named_inside,
    weak: crate_named_only,
    asset: |scene| named(scene, 31, "crate_a"),
    runtime: tidy,
    mode: Mode::Normal,
};

const SCRIPT_CONFLICT: Spec = Spec {
    scene: cluttered,
    reference: crate_inside,
    weak: crate_inside,
    asset: |scene| named(scene, 23, "peer_edit") && scene.prop(31).is_some(),
    runtime: tidy,
    mode: Mode::Stale,
};

fn direct_unblock(door: &mut Door, sub: Submission) -> Outcome {
    run_spec(&DIRECT_UNBLOCK, door, sub)
}

fn asset_unblock(door: &mut Door, sub: Submission) -> Outcome {
    run_spec(&ASSET_UNBLOCK, door, sub)
}

fn direct_symmetry(door: &mut Door, sub: Submission) -> Outcome {
    run_spec(&DIRECT_SYMMETRY, door, sub)
}

fn asset_symmetry(door: &mut Door, sub: Submission) -> Outcome {
    run_spec(&ASSET_SYMMETRY, door, sub)
}

fn script_ring(door: &mut Door, sub: Submission) -> Outcome {
    run_spec(&SCRIPT_RING, door, sub)
}

fn script_rename_all(door: &mut Door, sub: Submission) -> Outcome {
    run_spec(&SCRIPT_RENAME_ALL, door, sub)
}

fn script_on_time(door: &mut Door, sub: Submission) -> Outcome {
    run_spec(&SCRIPT_ON_TIME, door, sub)
}

fn direct_tidy(door: &mut Door, sub: Submission) -> Outcome {
    run_spec(&DIRECT_TIDY, door, sub)
}

fn asset_tidy(door: &mut Door, sub: Submission) -> Outcome {
    run_spec(&ASSET_TIDY, door, sub)
}

fn script_conflict(door: &mut Door, sub: Submission) -> Outcome {
    run_spec(&SCRIPT_CONFLICT, door, sub)
}

/// The ten tasks, in report order. Tasks 1-2 and 3-4 are paired: one runtime
/// test, a direct deliverable and a labeled one.
pub(crate) const TASKS: [MirrorTask; 10] = [
    MirrorTask {
        id: "cr01",
        category: "direct",
        shape: "paired: open a sealed path by moving the blocker",
        run: direct_unblock,
    },
    MirrorTask {
        id: "cr02",
        category: "asset",
        shape: "paired: open a sealed path, blocker labeled as an asset",
        run: asset_unblock,
    },
    MirrorTask {
        id: "cr03",
        category: "direct",
        shape: "paired: restore a symmetric layout by moving one prop",
        run: direct_symmetry,
    },
    MirrorTask {
        id: "cr04",
        category: "asset",
        shape: "paired: restore symmetry, props labeled as assets",
        run: asset_symmetry,
    },
    MirrorTask {
        id: "cr05",
        category: "script",
        shape: "batch script: arrange four props on a ring",
        run: script_ring,
    },
    MirrorTask {
        id: "cr06",
        category: "script",
        shape: "batch script: rename all props, then verify",
        run: script_rename_all,
    },
    MirrorTask {
        id: "cr07",
        category: "script",
        shape: "on-time rule: a late submission does not count",
        run: script_on_time,
    },
    MirrorTask {
        id: "cr08",
        category: "direct",
        shape: "clear an overlap and stay inside the arena",
        run: direct_tidy,
    },
    MirrorTask {
        id: "cr09",
        category: "asset",
        shape: "clear an overlap, prop labeled as an asset",
        run: asset_tidy,
    },
    MirrorTask {
        id: "cr10",
        category: "script",
        shape: "conflict: rebase on the peer's revision before submitting",
        run: script_conflict,
    },
];

/// Run the slice on every backend.
pub fn run() -> SliceReport {
    run_slice(&SUITE, &TASKS, BASE_INDEX, notes)
}

/// Where weak submissions stop, and how many clear asset yet fail runtime
/// (the gap between structure checks and behavior the suite reports).
fn notes(report: &SliceReport) -> Vec<String> {
    let stages: Vec<String> = report
        .weak_failure_stages("mock")
        .into_iter()
        .map(|(name, count)| format!("{name} {count}"))
        .collect();
    let gap = report.weak_cleared_then_failed("mock", "asset", "runtime");
    vec![
        format!("weak submissions stop at: {}", stages.join(", ")),
        format!(
            "asset-pass runtime-fail: {gap} of {} weak submissions",
            report.tasks()
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::Backend;

    const SPECS: [(&str, &Spec); 10] = [
        ("cr01", &DIRECT_UNBLOCK),
        ("cr02", &ASSET_UNBLOCK),
        ("cr03", &DIRECT_SYMMETRY),
        ("cr04", &ASSET_SYMMETRY),
        ("cr05", &SCRIPT_RING),
        ("cr06", &SCRIPT_RENAME_ALL),
        ("cr07", &SCRIPT_ON_TIME),
        ("cr08", &DIRECT_TIDY),
        ("cr09", &ASSET_TIDY),
        ("cr10", &SCRIPT_CONFLICT),
    ];

    fn outcome(id: &str, sub: Submission) -> Outcome {
        let task = TASKS.iter().find(|task| task.id == id).expect("task id");
        (task.run)(&mut Door::new(Backend::Mock, 0), sub)
    }

    #[test]
    fn fixtures_are_not_solved_before_the_submission() {
        for (id, spec) in SPECS {
            if spec.mode != Mode::Normal {
                continue;
            }
            let before = (spec.scene)();
            assert!(
                !(spec.runtime)(&before),
                "{id}: the runtime test already holds on the fixture"
            );
        }
    }

    #[test]
    fn reference_clears_every_stage_and_weak_stops_early() {
        for (id, _) in SPECS {
            let reference = outcome(id, Submission::Reference);
            assert!(reference.pass, "{id}: {reference:?}");
            assert_eq!(reference.stage("runtime"), Some(true), "{id}");
            let weak = outcome(id, Submission::Weak);
            assert!(!weak.pass, "{id}: weak must fail: {weak:?}");
        }
    }

    #[test]
    fn weak_failures_land_on_the_stage_each_task_targets() {
        let expect = [
            ("cr01", "runtime"),
            ("cr02", "runtime"),
            ("cr03", "runtime"),
            ("cr04", "asset"),
            ("cr05", "runtime"),
            ("cr06", "build"),
            ("cr07", "on-time"),
            ("cr08", "runtime"),
            ("cr09", "runtime"),
            ("cr10", "build"),
        ];
        for (id, stage) in expect {
            let weak = outcome(id, Submission::Weak);
            assert_eq!(weak.first_failure(), Some(stage), "{id}: {weak:?}");
        }
    }

    #[test]
    fn asset_checks_pass_while_runtime_fails_for_the_paired_blueprint_shape() {
        let report = run();
        assert!(
            report.weak_cleared_then_failed("mock", "asset", "runtime") >= 4,
            "structure-only checks must be shown to miss behavior: {:?}",
            report.weak_failure_stages("mock")
        );
    }

    #[test]
    fn stages_stop_at_the_first_failure() {
        let weak = outcome("cr06", Submission::Weak);
        assert_eq!(weak.stage("build"), Some(false));
        assert_eq!(
            weak.stage("asset"),
            None,
            "asset must not run after a build failure"
        );
        assert_eq!(weak.stage("runtime"), None);
    }

    #[test]
    fn a_failed_batch_leaves_the_scene_untouched() {
        let weak = outcome("cr06", Submission::Weak);
        let untouched = fixture_scene().hash();
        assert_eq!(
            weak.hash, untouched,
            "atomic apply must roll the batch back"
        );
    }

    #[test]
    fn the_slice_reproduces_and_matches_across_backends() {
        let first = run();
        let second = run();
        assert_eq!(first.to_text(), second.to_text());
        assert_eq!(first.parity(), (10, Vec::new()));
        for backend in ["mock", "local", "remote"] {
            assert_eq!(first.solved(backend), 10, "{backend}");
            assert_eq!(first.caught(backend), 10, "{backend}");
        }
    }
}
