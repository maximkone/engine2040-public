//! Repair-loop eval: ten seeded scenes through the engine repair loop.
//!
//! Each scene runs dual-proposal plus self-consistent decode plus at most
//! three [`repair_round`](super::repair::repair_round)s. The report posts the
//! clean-first-pass lift with byte-identical replay. Std-only plus engine-core.

use super::repair::{dual_propose, repair_round, self_consistent_decode};
use super::Actor;

/// Scenes in the harness. Six solvable, two clean, one grammar, one NaN.
pub const SCENE_COUNT: usize = 10;
/// Repair rounds per scene. The loop stops early when clean or refused.
pub const MAX_ROUNDS: u32 = 3;

/// One seeded scene: a name plus its actor snapshot.
#[derive(Debug, Clone)]
pub struct EvalScene {
    pub name: &'static str,
    pub actors: Vec<Actor>,
}

/// One scored scene: rounds used, dual-lane agreement, clean flags.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvalRow {
    pub name: &'static str,
    pub rounds: u32,
    pub agreed: bool,
    pub clean_first: bool,
    pub clean_after: bool,
}

/// Whole-harness report: one row per scene plus clean counts.
#[derive(Debug, Clone)]
pub struct EvalReport {
    pub rows: Vec<EvalRow>,
    pub clean_first_pass: u32,
    pub clean_after_loop: u32,
}

/// Resting 1 m prop. Field overrides set y, sizes, names per scene.
fn resting(id: u32, name: &str, x: f32, z: f32) -> Actor {
    Actor {
        id,
        name: name.into(),
        x,
        y: 0.5,
        z,
        sx: 1.0,
        sy: 1.0,
        sz: 1.0,
        parent: None,
        physical: true,
        vx: 0.0,
        vz: 0.0,
        yaw: 0.0,
    }
}

/// The ten fixtures: six solvable, two clean, one grammar, one NaN.
pub fn eval_scenes() -> Vec<EvalScene> {
    let mut scenes = vec![
        EvalScene {
            name: "overlap_pair",
            actors: vec![
                resting(0, "crate_a", 0.0, 0.0),
                resting(1, "crate_b", 0.5, 0.0),
            ],
        },
        EvalScene {
            name: "overlap_floating",
            actors: vec![
                resting(0, "crate_a", 0.0, 0.0),
                resting(1, "crate_b", 0.5, 0.0),
                Actor {
                    y: 5.0,
                    ..resting(2, "prop_c", 9.0, 9.0)
                },
            ],
        },
        EvalScene {
            name: "floating_torch",
            actors: vec![Actor {
                y: 5.0,
                ..resting(0, "torch_01", 1.03, 2.07)
            }],
        },
        EvalScene {
            name: "sunken_crate",
            actors: vec![Actor {
                y: -5.0,
                ..resting(0, "crate_sunk", 0.0, 0.0)
            }],
        },
        EvalScene {
            name: "bounds_outlier",
            actors: vec![resting(0, "far_prop", 150.0, -150.0)],
        },
        EvalScene {
            name: "giant_resize",
            actors: vec![Actor {
                sx: 60.0,
                sy: 60.0,
                sz: 60.0,
                y: 30.0,
                ..resting(0, "hall_01", 0.0, 0.0)
            }],
        },
        EvalScene {
            name: "dup_room_clean",
            actors: vec![
                resting(0, "wall_n", 0.0, 0.0),
                resting(1, "wall_s", 0.0, 10.0),
                resting(2, "crate", 5.0, 0.0),
                resting(3, "crate", 5.0, 10.0),
            ],
        },
        EvalScene {
            name: "far_props_clean",
            actors: vec![
                resting(0, "prop_a", 0.0, 0.0),
                resting(1, "prop_b", 20.0, 20.0),
            ],
        },
        EvalScene {
            name: "grammar_stuck",
            actors: vec![resting(0, "Bad Name!", 0.0, 0.0)],
        },
    ];
    scenes.push(EvalScene {
        name: "nan_refused",
        actors: vec![Actor {
            x: f32::NAN,
            ..resting(0, "lost_prop", 0.0, 0.0)
        }],
    });
    scenes
}

/// Score one scene: dual-propose, require full decode agreement, then loop
/// at most [`MAX_ROUNDS`] repair rounds, re-running checks each round.
pub fn eval_scene(scene: &EvalScene, revision: u64) -> EvalRow {
    let first = super::run(&scene.actors);
    let clean_first = first.findings.is_empty();
    let (lane_a, lane_b) = dual_propose(&first.findings, &scene.actors, revision);
    let agreed = self_consistent_decode(&lane_a, &lane_b).len() == first.findings.len();
    let mut cur = scene.actors.clone();
    let mut rounds: u32 = 0;
    loop {
        let rep = super::run(&cur);
        if rep.findings.is_empty() || rounds >= MAX_ROUNDS {
            break;
        }
        match repair_round(&cur, &rep.findings, revision, rounds) {
            Some(next) => {
                cur = next;
                rounds = rounds.saturating_add(1);
            }
            None => break,
        }
    }
    let clean_after = super::run(&cur).findings.is_empty();
    EvalRow {
        name: scene.name,
        rounds,
        agreed,
        clean_first,
        clean_after,
    }
}

/// Score all ten scenes at one revision.
///
/// Consumer: [`eval_summary`] posts these rows as text; tests pin the lift.
pub fn eval_report() -> EvalReport {
    let mut rows: Vec<EvalRow> = Vec::with_capacity(SCENE_COUNT);
    rows.extend(eval_scenes().iter().map(|s| eval_scene(s, 1)));
    let mut clean_first_pass = 0u32;
    let mut clean_after_loop = 0u32;
    for r in &rows {
        if r.clean_first {
            clean_first_pass = clean_first_pass.saturating_add(1);
        }
        if r.clean_after {
            clean_after_loop = clean_after_loop.saturating_add(1);
        }
    }
    EvalReport {
        rows,
        clean_first_pass,
        clean_after_loop,
    }
}

/// Clean-first-pass lift: scenes clean after the loop minus scenes clean
/// on the first pass, saturating. Pure over the report counts; same report
/// gives the same lift twice. Caller: [`report_text`] (the posted readout
/// carries it) plus the lift tests.
///
/// Consumer: [`eval_summary`] posts it through [`report_text`]; the bytes-x2
/// test pins the full line. Units: scenes. Determinism: integer only, no
/// clock, no allocation. Errors: none.
#[must_use]
pub fn clean_lift(report: &EvalReport) -> u32 {
    report
        .clean_after_loop
        .saturating_sub(report.clean_first_pass)
}

/// One digest line plus one line per scene: name, rounds, agreed, clean flags.
pub fn report_text(report: &EvalReport) -> String {
    let mut body = String::new();
    for r in &report.rows {
        body.push_str(&format!(
            "{} rounds={} agreed={} first={} after={}\n",
            r.name, r.rounds, r.agreed, r.clean_first, r.clean_after
        ));
    }
    let digest = report_digest(&body);
    let lift = clean_lift(report);
    format!(
        "sp43-eval digest={digest:016x} scenes={} lift={lift}\n{body}",
        report.rows.len()
    )
}

/// Digest of report bytes under the harness tag.
pub fn report_digest(text: &str) -> u64 {
    let mut h = engine_core::Fnv64::new();
    h.write_str("e2sp43-eval:1");
    h.write_str(text);
    h.finish()
}

/// Posted harness readout: full report text for the ten seeded scenes.
///
/// Consumer: the SP43 receipt posts this string (one digest line plus one
/// line per scene). Same scenes give the same bytes; no clock, no map order.
/// This is the one non-test call path through [`eval_report`] and
/// [`report_text`]; the bytes-x2 test pins it twice.
pub fn eval_summary() -> String {
    report_text(&eval_report())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sp43_repair_dual_agree_decode_keeps_all() {
        let scenes = eval_scenes();
        assert_eq!(scenes.len(), SCENE_COUNT);
        for scene in &scenes {
            let rep = super::super::run(&scene.actors);
            let (a, b) = dual_propose(&rep.findings, &scene.actors, 7);
            assert_eq!(a, b, "{}: lanes disagree", scene.name);
            let kept = self_consistent_decode(&a, &b);
            assert_eq!(
                kept.len(),
                rep.findings.len(),
                "{}: decode dropped",
                scene.name
            );
        }
    }

    #[test]
    fn sp43_repair_bytes_x2() {
        let first = eval_summary();
        let second = eval_summary();
        assert_eq!(first, second, "report bytes drifted");
        assert_eq!(
            first,
            report_text(&eval_report()),
            "summary left the call path"
        );
    }

    #[test]
    fn sp43_repair_rows_stable_under_reorder() {
        for scene in &eval_scenes() {
            let row = eval_scene(scene, 1);
            let mut rev = scene.clone();
            rev.actors.reverse();
            let moved = eval_scene(&rev, 1);
            assert_eq!(moved, row, "{}: order moved row", scene.name);
        }
    }

    #[test]
    fn sp43_repair_round_caps_and_end_states() {
        for row in &eval_report().rows {
            assert!(row.rounds <= MAX_ROUNDS, "{}: over cap", row.name);
        }
        let rows = eval_report().rows;
        for row in &rows {
            match row.name {
                "grammar_stuck" => {
                    assert!(
                        !row.clean_first && !row.clean_after && row.rounds == 3,
                        "{row:?}"
                    );
                }
                "nan_refused" => {
                    assert!(
                        !row.clean_first && !row.clean_after && row.rounds == 0,
                        "{row:?}"
                    );
                }
                _ => assert!(row.clean_after, "{row:?}"),
            }
        }
    }

    #[test]
    fn sp43_repair_lift_at_least_three() {
        let rep = eval_report();
        assert!(
            rep.clean_after_loop.saturating_sub(rep.clean_first_pass) >= 3,
            "{rep:?}"
        );
    }

    #[test]
    fn sp43_repair_lift_reports_in_summary() {
        let rep = eval_report();
        let lift = clean_lift(&rep);
        assert!(
            lift >= 3,
            "repair loop must lift at least three scenes: {rep:?}"
        );
        let text = eval_summary();
        assert!(
            text.contains(&format!("lift={lift}")),
            "summary must carry the lift: {text}"
        );
    }
}
