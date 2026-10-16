//! Mirror slice, game-logic shape: ten tasks in the style of a public
//! benchmark of runtime game-logic tasks scored by tick-level state
//! assertions over seeded scenario variations, where the evaluator must
//! accept different correct implementations and reject mutants.
//!
//! Consumer: the 30-task aggregate. Owner: this file owns the ten rules, the
//! frame extraction and the published figure; shapes and scoring rules only,
//! no suite task text or assets. A submission is a recorded input tape (or a
//! replay of one); the real arena runs it per seed, and a rule is asserted on
//! the state of every tick. A mutant is a submission with a required piece
//! missing, or a trace with one seeded defect; the rule must name the tick.
//! Units: fixed ticks at 60 Hz, meters, health in permille. Determinism:
//! frames come from the headless arena dataset and carry no wall clock.

use engine_arena::controller::{SlotInput, SlotTape};
use engine_arena::recorder::Dataset;
use engine_arena::{run_dataset, run_prefix, ArenaConfig, Checkpoint};
use engine_core::vec3::dist2;
use engine_core::Fnv64;

use crate::agent::{scan_desync, scan_impossible, Trajectory};
use crate::door::Door;
use crate::mirror_kit::{
    admit, ck, run_slice, Claim, MirrorTask, Outcome, SliceReport, Submission, SuiteInfo,
};

/// First eval fixture slot of this slice.
const BASE_INDEX: u64 = 20;
/// Arena half-extent for every scenario, meters.
const HALF_EXTENT_M: f32 = 20.0;
/// Scenario seeds, one test case each.
const SEEDS: [u64; 6] = [2040, 2041, 2042, 2043, 2044, 2045];
/// Case labels, parallel to [`SEEDS`].
const CASE_NAMES: [&str; 6] = ["seed-0", "seed-1", "seed-2", "seed-3", "seed-4", "seed-5"];
/// Ticks for the short rules.
const SHORT_TICKS: u32 = 120;
/// Ticks for the knockout rules (an idle foe needs time to fall).
const KO_TICKS: u32 = 900;
/// Ticks the walk tape pushes fighter 0 forward.
const WALK_TICKS: u32 = 30;
/// Least planar distance, meters, a held forward wish must move fighter 0.
const WALK_MIN_M: f32 = 0.25;

/// The suite this slice mirrors. The figures are the suite's own, quoted
/// from its abstract page (v2, revised 2026-09-21, read 2026-10-10); rival
/// MCP doors were not run on these tasks and stay unknown.
pub static SUITE: SuiteInfo = SuiteInfo {
    id: "gamelogicbench",
    name: "GameLogicBench",
    published: Claim::Quoted {
        text: "72 tasks, 403 scenarios, 1,451 test cases; best of 20 model and scaffold runs solves 52.78%",
        source: "https://arxiv.org/abs/2609.21562",
    },
    rivals: [Claim::Unknown; 3],
};

/// One tick of recorded match state, every fighter in slot order.
#[derive(Clone, PartialEq, Debug)]
struct Frame {
    tick: u32,
    pos: Vec<[f32; 3]>,
    hp: Vec<u16>,
    damage: Vec<u16>,
    idle: Vec<bool>,
    killed: bool,
}

/// First tick a rule broke and why.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Violation {
    tick: u32,
    why: &'static str,
}

/// A tick-level rule over one scenario's frames.
type Rule = fn(&[Frame]) -> Result<(), Violation>;

/// One scenario: a label and its recorded frames.
struct Case {
    name: &'static str,
    frames: Vec<Frame>,
}

// ---- recording ---------------------------------------------------------

/// Frames of one headless match, tick by tick. Empty when the arena refuses
/// the config, which every rule reads as a failure.
fn frames_of(dataset: &Dataset, fighters: usize) -> Vec<Frame> {
    let mut frames = Vec::new();
    for chunk in dataset.rows.chunks(fighters) {
        let Some(lead) = chunk.first() else { continue };
        let entries = lead.observation.fighters.as_slice();
        if chunk.len() != fighters || entries.len() != fighters {
            return Vec::new();
        }
        frames.push(Frame {
            tick: lead.observation.tick,
            pos: entries.iter().map(|entry| entry.pos).collect(),
            hp: entries.iter().map(|entry| entry.hp).collect(),
            damage: chunk.iter().map(|row| row.outcome.damage_after).collect(),
            idle: chunk
                .iter()
                .map(|row| {
                    let input = row.action.input;
                    input.mx == 0 && input.mz == 0 && input.buttons.0 == 0
                })
                .collect(),
            killed: chunk.iter().any(|row| row.outcome.killed),
        });
    }
    frames
}

fn config(seed: u64, ticks: u32) -> ArenaConfig {
    ArenaConfig {
        seed,
        ticks,
        fighters: 2,
        half_extent_m: HALF_EXTENT_M,
        table_version: 1,
    }
}

/// Run the real arena under `tape` and record every tick.
fn record(seed: u64, ticks: u32, tape: &SlotTape) -> Vec<Frame> {
    run_dataset(&config(seed, ticks), tape)
        .map(|dataset| frames_of(&dataset, 2))
        .unwrap_or_default()
}

/// One recorded first-tick input per slot. Slot ids come from the arena, so
/// the tapes below never name them.
fn slot_inputs() -> Vec<SlotInput> {
    run_dataset(&config(SEEDS[0], 1), &SlotTape::default())
        .map(|dataset| {
            dataset
                .rows
                .iter()
                .take(2)
                .map(|row| row.action.input)
                .collect()
        })
        .unwrap_or_default()
}

/// Wishes held on `slots` for ticks `0..until`; later ticks stay brain-driven.
fn hold_tape(slots: &[u8], until: u32, forward: i8) -> SlotTape {
    let templates = slot_inputs();
    let mut inputs = Vec::new();
    for tick in 0..until {
        for slot in slots {
            if let Some(template) = templates.get(usize::from(*slot)) {
                inputs.push(SlotInput {
                    tick,
                    mx: forward,
                    mz: 0,
                    facing: 0,
                    buttons: Default::default(),
                    ..*template
                });
            }
        }
    }
    SlotTape(inputs)
}

/// Index into [`SEEDS`] of the one seed whose match reaches a knockout
/// within [`KO_TICKS`] under both the default brains and a pinned fighter.
const KO_SEED: usize = 3;

/// The single knockout case under `tape`: knockout rules need a knockout to
/// assert on, and the other seeds produce none in that window.
fn ko_cases(tape: &SlotTape) -> Vec<Case> {
    vec![Case {
        name: CASE_NAMES[KO_SEED],
        frames: record(SEEDS[KO_SEED], KO_TICKS, tape),
    }]
}

/// Recorded cases for the first `count` seeds under `tape`.
fn cases(count: usize, ticks: u32, tape: &SlotTape) -> Vec<Case> {
    SEEDS
        .iter()
        .zip(CASE_NAMES)
        .take(count)
        .map(|(seed, name)| Case {
            name,
            frames: record(*seed, ticks, tape),
        })
        .collect()
}

// ---- rules -------------------------------------------------------------

fn need_frames(frames: &[Frame]) -> Result<(), Violation> {
    if frames.is_empty() {
        return Err(Violation {
            tick: 0,
            why: "no frames recorded",
        });
    }
    Ok(())
}

/// Every pose is finite; NaN slips past distance checks (`NaN < x` and
/// `NaN > x` are both false), so each movement rule rejects non-finite
/// poses before measuring distance.
fn require_finite(frames: &[Frame]) -> Result<(), Violation> {
    for frame in frames {
        if !frame
            .pos
            .iter()
            .all(|pose| pose.iter().all(|v| v.is_finite()))
        {
            return Err(Violation {
                tick: frame.tick,
                why: "pose not finite",
            });
        }
    }
    Ok(())
}

/// Health never rises from one tick to the next.
fn hp_never_rises(frames: &[Frame]) -> Result<(), Violation> {
    need_frames(frames)?;
    for pair in frames.windows(2) {
        if pair[0].hp.iter().zip(&pair[1].hp).any(|(a, b)| b > a) {
            return Err(Violation {
                tick: pair[1].tick,
                why: "health rose",
            });
        }
    }
    Ok(())
}

/// Each fighter's damage-dealt counter only grows.
fn damage_only_grows(frames: &[Frame]) -> Result<(), Violation> {
    need_frames(frames)?;
    for pair in frames.windows(2) {
        if pair[0]
            .damage
            .iter()
            .zip(&pair[1].damage)
            .any(|(a, b)| b < a)
        {
            return Err(Violation {
                tick: pair[1].tick,
                why: "damage counter fell",
            });
        }
    }
    Ok(())
}

/// Every pose is finite and inside the arena.
fn stays_in_arena(frames: &[Frame]) -> Result<(), Violation> {
    need_frames(frames)?;
    let trajectory = Trajectory {
        ticks: u32::try_from(frames.len()).unwrap_or(u32::MAX),
        fighters: 2,
        half_extent_m: HALF_EXTENT_M,
        pos: frames.iter().map(|frame| frame.pos.clone()).collect(),
    };
    match scan_impossible(&trajectory).first() {
        Some((tick, _, _)) => Err(Violation {
            tick: *tick,
            why: "pose left the arena or was not finite",
        }),
        None => Ok(()),
    }
}

/// The pinned fighter (slot 1) issues no input and deals no damage.
fn pinned_fighter_is_idle(frames: &[Frame]) -> Result<(), Violation> {
    need_frames(frames)?;
    for frame in frames {
        if frame.idle.get(1) != Some(&true) || frame.damage.get(1).is_none_or(|d| *d > 0) {
            return Err(Violation {
                tick: frame.tick,
                why: "pinned fighter acted",
            });
        }
    }
    Ok(())
}

/// Planar squared distance, for walk progress only. The pose holds below pin
/// the full 3D pose through the engine-core squared distance owner, so a body
/// that sinks or floats without moving on the plane still breaks the hold.
fn planar_sq(a: [f32; 3], b: [f32; 3]) -> f32 {
    let (dx, dz) = (a[0] - b[0], a[2] - b[2]);
    dx * dx + dz * dz
}

/// A held forward wish moves fighter 0 at least [`WALK_MIN_M`] in the walk
/// window. Seed 2040 starts 0.4 m short of the east wall, and the arena's
/// walk speed covers about 0.6 m in [`WALK_TICKS`], so the bar is a quarter
/// meter, not a full one.
fn walk_input_moves_fighter(frames: &[Frame]) -> Result<(), Violation> {
    need_frames(frames)?;
    require_finite(frames)?;
    let end = usize::try_from(WALK_TICKS).unwrap_or(usize::MAX);
    let (Some(start), Some(stop)) = (frames.first(), frames.get(end)) else {
        return Err(Violation {
            tick: 0,
            why: "run shorter than the walk window",
        });
    };
    if planar_sq(start.pos[0], stop.pos[0]) < WALK_MIN_M * WALK_MIN_M {
        return Err(Violation {
            tick: stop.tick,
            why: "held input did not move the fighter",
        });
    }
    Ok(())
}

/// First frame where some fighter is at zero health, with its slot.
fn first_knockout(frames: &[Frame]) -> Option<(usize, usize)> {
    frames.iter().enumerate().find_map(|(index, frame)| {
        frame
            .hp
            .iter()
            .position(|hp| *hp == 0)
            .map(|slot| (index, slot))
    })
}

/// A knocked-out fighter keeps its pose and stays at zero health.
fn knockout_holds_pose(frames: &[Frame]) -> Result<(), Violation> {
    need_frames(frames)?;
    require_finite(frames)?;
    let Some((at, slot)) = first_knockout(frames) else {
        return Err(Violation {
            tick: 0,
            why: "no knockout to assert on",
        });
    };
    let pose = frames[at].pos[slot];
    for frame in &frames[at..] {
        if frame.hp[slot] != 0 || dist2(frame.pos[slot], pose) > 1e-4 {
            return Err(Violation {
                tick: frame.tick,
                why: "knocked-out fighter moved or healed",
            });
        }
    }
    Ok(())
}

/// The kill flag is set on exactly the tick before health first hits zero.
fn kill_flag_matches_health(frames: &[Frame]) -> Result<(), Violation> {
    need_frames(frames)?;
    if first_knockout(frames).is_none() {
        return Err(Violation {
            tick: 0,
            why: "no knockout to assert on",
        });
    }
    for pair in frames.windows(2) {
        let fell = pair[0]
            .hp
            .iter()
            .zip(&pair[1].hp)
            .any(|(before, after)| *before > 0 && *after == 0);
        if pair[0].killed != fell {
            return Err(Violation {
                tick: pair[0].tick,
                why: "kill flag disagrees with health",
            });
        }
    }
    if let Some(last) = frames.last() {
        if last.killed {
            return Err(Violation {
                tick: last.tick,
                why: "kill flag disagrees with health",
            });
        }
    }
    Ok(())
}

/// Health stays in 0..=1000 permille and poses stay finite.
fn state_stays_sane(frames: &[Frame]) -> Result<(), Violation> {
    need_frames(frames)?;
    for frame in frames {
        let finite = frame
            .pos
            .iter()
            .all(|pose| pose.iter().all(|v| v.is_finite()));
        if !finite || frame.hp.iter().any(|hp| *hp > 1000) {
            return Err(Violation {
                tick: frame.tick,
                why: "health or pose out of range",
            });
        }
    }
    Ok(())
}

/// With every slot pinned, nobody deals damage and nobody leaves the spot.
fn idle_match_stays_still(frames: &[Frame]) -> Result<(), Violation> {
    need_frames(frames)?;
    require_finite(frames)?;
    let first = &frames[0];
    for frame in frames {
        let drifted = frame
            .pos
            .iter()
            .zip(&first.pos)
            .any(|(now, then)| dist2(*now, *then) > 1e-4);
        if drifted || frame.damage.iter().any(|d| *d > 0) {
            return Err(Violation {
                tick: frame.tick,
                why: "idle match moved or dealt damage",
            });
        }
    }
    Ok(())
}

// ---- seeded trace defects ---------------------------------------------

/// Health rises by 50 permille on tick `at`.
fn inject_heal(frames: &mut [Frame], at: usize) -> bool {
    let Some(before) = at
        .checked_sub(1)
        .and_then(|prev| frames.get(prev))
        .map(|f| f.hp[0])
    else {
        return false;
    };
    match frames.get_mut(at) {
        Some(frame) => {
            frame.hp[0] = before.saturating_add(50);
            true
        }
        None => false,
    }
}

/// The damage counter of the busiest fighter resets on the last tick.
fn inject_counter_reset(frames: &mut [Frame]) -> bool {
    let Some(slot) = frames
        .last()
        .and_then(|last| last.damage.iter().position(|d| *d > 0))
    else {
        return false;
    };
    if let Some(last) = frames.last_mut() {
        last.damage[slot] = 0;
        return true;
    }
    false
}

/// Fighter 1 teleports out of the arena on tick `at`.
fn inject_teleport(frames: &mut [Frame], at: usize) -> bool {
    match frames.get_mut(at) {
        Some(frame) => {
            frame.pos[1] = [99.0, 0.0, 0.0];
            true
        }
        None => false,
    }
}

/// The knocked-out fighter drifts half a meter ten ticks after the fall.
fn inject_drift(frames: &mut [Frame]) -> bool {
    let Some((at, slot)) = first_knockout(frames) else {
        return false;
    };
    match frames.get_mut(at + 10) {
        Some(frame) => {
            frame.pos[slot][0] += 0.5;
            true
        }
        None => false,
    }
}

/// The kill flag fires five ticks before the fall.
fn inject_early_flag(frames: &mut [Frame]) -> bool {
    let Some((at, _)) = first_knockout(frames) else {
        return false;
    };
    match at.checked_sub(6).and_then(|early| frames.get_mut(early)) {
        Some(frame) => {
            frame.killed = true;
            true
        }
        None => false,
    }
}

/// Health reads 1500 permille on tick `at`.
fn inject_overheal(frames: &mut [Frame], at: usize) -> bool {
    match frames.get_mut(at) {
        Some(frame) => {
            frame.hp[0] = 1500;
            true
        }
        None => false,
    }
}

// ---- judging -----------------------------------------------------------

fn digest_frames(hash: &mut Fnv64, frames: &[Frame]) {
    hash.write_u64(u64::try_from(frames.len()).unwrap_or(u64::MAX));
    for frame in frames {
        hash.write_u32(frame.tick);
        for pose in &frame.pos {
            for axis in pose {
                hash.write_f32(*axis);
            }
        }
        for (hp, damage) in frame.hp.iter().zip(&frame.damage) {
            hash.write_u16(*hp);
            hash.write_u16(*damage);
        }
        hash.write_bool(frame.killed);
    }
}

/// Admit the submission at the door, then assert `rule` on every case.
fn judge(door: &mut Door, note: &str, rule: Rule, cases: &[Case]) -> Outcome {
    let mut checks = vec![admit(door, note)];
    let mut hash = Fnv64::new();
    let mut first = None;
    for case in cases {
        digest_frames(&mut hash, &case.frames);
        let verdict = rule(&case.frames);
        checks.push(ck(case.name, verdict.is_ok()));
        if let Err(violation) = verdict {
            first.get_or_insert_with(|| {
                format!(
                    "{}: {} at tick {}",
                    case.name, violation.why, violation.tick
                )
            });
        }
    }
    let detail = first.unwrap_or_else(|| format!("{} case(s) hold", cases.len()));
    Outcome::from_checks(checks, hash.finish(), detail)
}

/// Apply a seeded defect to one case when the run is the weak one.
fn mutate(sub: Submission, cases: &mut [Case], at: usize, defect: fn(&mut [Frame]) -> bool) {
    if sub == Submission::Weak {
        if let Some(case) = cases.get_mut(at) {
            let applied = defect(&mut case.frames);
            debug_assert!(applied, "the seeded defect must apply to {}", case.name);
        }
    }
}

// ---- the ten tasks -----------------------------------------------------

fn health_never_rises(door: &mut Door, sub: Submission) -> Outcome {
    let mut all = cases(3, SHORT_TICKS, &SlotTape::default());
    mutate(sub, &mut all, 1, |frames| inject_heal(frames, 40));
    judge(door, "hp-never-rises", hp_never_rises, &all)
}

fn damage_counter_grows(door: &mut Door, sub: Submission) -> Outcome {
    let mut all = cases(KO_SEED + 1, KO_TICKS, &SlotTape::default());
    mutate(sub, &mut all, KO_SEED, inject_counter_reset);
    judge(door, "damage-only-grows", damage_only_grows, &all)
}

fn poses_stay_in_arena(door: &mut Door, sub: Submission) -> Outcome {
    let mut all = cases(3, SHORT_TICKS, &SlotTape::default());
    mutate(sub, &mut all, 0, |frames| inject_teleport(frames, 50));
    judge(door, "stays-in-arena", stays_in_arena, &all)
}

fn pinned_fighter_stays_idle(door: &mut Door, sub: Submission) -> Outcome {
    let pinned = hold_tape(&[1], SHORT_TICKS, 0);
    let mut cases = vec![
        Case {
            name: "pinned-a",
            frames: record(SEEDS[0], SHORT_TICKS, &pinned),
        },
        Case {
            name: "pinned-b",
            frames: record(SEEDS[1], SHORT_TICKS, &pinned),
        },
    ];
    if sub == Submission::Weak {
        // The mutant never pins the fighter: its brain keeps acting.
        cases[1].frames = record(SEEDS[1], SHORT_TICKS, &SlotTape::default());
    }
    judge(door, "pin-fighter-one", pinned_fighter_is_idle, &cases)
}

fn held_input_moves(door: &mut Door, sub: Submission) -> Outcome {
    let walk = hold_tape(&[0], WALK_TICKS, 100);
    let tape = sub.pick(walk, hold_tape(&[0], WALK_TICKS, 0));
    let all = cases(2, SHORT_TICKS, &tape);
    judge(door, "walk-fighter-zero", walk_input_moves_fighter, &all)
}

/// One pinned-foe knockout case judged by `rule`; the weak run carries
/// `defect`. Both knockout tasks share this setup so the flag and the pose
/// rules cannot drift apart.
fn knockout_outcome(
    door: &mut Door,
    sub: Submission,
    defect: fn(&mut [Frame]) -> bool,
    note: &str,
    rule: Rule,
) -> Outcome {
    let pinned = hold_tape(&[1], KO_TICKS, 0);
    let mut all = ko_cases(&pinned);
    mutate(sub, &mut all, 0, defect);
    judge(door, note, rule, &all)
}

fn knockout_holds(door: &mut Door, sub: Submission) -> Outcome {
    knockout_outcome(
        door,
        sub,
        inject_drift,
        "knockout-holds-pose",
        knockout_holds_pose,
    )
}

fn replay_identical(door: &mut Door, sub: Submission) -> Outcome {
    let walk = hold_tape(&[0], WALK_TICKS, 100);
    let mut replay = walk.clone();
    if sub == Submission::Weak {
        // The mutant replay drops the input at tick 10.
        replay.0.retain(|input| input.tick != 10);
    }
    let control = run_prefix(&config(SEEDS[0], 60), &walk, 60).map(|(_, points)| points);
    let probe = run_prefix(&config(SEEDS[0], 60), &replay, 60).map(|(_, points)| points);
    let (Some(control), Some(probe)) = (control, probe) else {
        return Outcome::from_checks(vec![admit(door, "replay")], 0, "arena refused".to_owned());
    };
    let diverged = scan_desync(&control, &probe);
    let hash = last_hash(&probe);
    let detail = diverged.map_or_else(
        || format!("{} checkpoints identical", control.len()),
        |tick| format!("replay diverged at checkpoint {tick}"),
    );
    let checks = vec![admit(door, "replay"), ck("replay", diverged.is_none())];
    Outcome::from_checks(checks, hash, detail)
}

fn last_hash(points: &[Checkpoint]) -> u64 {
    points.last().map_or(0, |point| point.hash)
}

fn kill_flag_consistent(door: &mut Door, sub: Submission) -> Outcome {
    knockout_outcome(
        door,
        sub,
        inject_early_flag,
        "kill-flag",
        kill_flag_matches_health,
    )
}

fn variations_stay_sane(door: &mut Door, sub: Submission) -> Outcome {
    let mut all = cases(6, SHORT_TICKS, &SlotTape::default());
    mutate(sub, &mut all, 4, |frames| inject_overheal(frames, 20));
    judge(door, "six-seeds", state_stays_sane, &all)
}

fn idle_match_holds(door: &mut Door, sub: Submission) -> Outcome {
    let both = hold_tape(&[0, 1], SHORT_TICKS, 0);
    let one = hold_tape(&[1], SHORT_TICKS, 0);
    let tape = sub.pick(both, one);
    let all = cases(2, SHORT_TICKS, &tape);
    judge(door, "pin-everyone", idle_match_stays_still, &all)
}

/// The ten tasks, in report order.
pub(crate) const TASKS: [MirrorTask; 10] = [
    MirrorTask {
        id: "gl01",
        category: "invariant",
        shape: "tick assertion: health never rises, three seeds",
        run: health_never_rises,
    },
    MirrorTask {
        id: "gl02",
        category: "invariant",
        shape: "tick assertion: the damage counter only grows",
        run: damage_counter_grows,
    },
    MirrorTask {
        id: "gl03",
        category: "invariant",
        shape: "tick assertion: every pose stays in the arena",
        run: poses_stay_in_arena,
    },
    MirrorTask {
        id: "gl04",
        category: "mechanic",
        shape: "two correct implementations pin a fighter; the mutant does not",
        run: pinned_fighter_stays_idle,
    },
    MirrorTask {
        id: "gl05",
        category: "mechanic",
        shape: "a held input moves the fighter; the mutant drops it",
        run: held_input_moves,
    },
    MirrorTask {
        id: "gl06",
        category: "interaction",
        shape: "a knocked-out fighter keeps its pose",
        run: knockout_holds,
    },
    MirrorTask {
        id: "gl07",
        category: "interaction",
        shape: "replay: every checkpoint matches; the mutant drops an input",
        run: replay_identical,
    },
    MirrorTask {
        id: "gl08",
        category: "interaction",
        shape: "the kill flag fires exactly when health reaches zero",
        run: kill_flag_consistent,
    },
    MirrorTask {
        id: "gl09",
        category: "scope",
        shape: "seeded variations: state stays in range on six seeds",
        run: variations_stay_sane,
    },
    MirrorTask {
        id: "gl10",
        category: "scope",
        shape: "a fully pinned match stays still; the mutant pins one slot",
        run: idle_match_holds,
    },
];

/// Run the slice on every backend.
pub fn run() -> SliceReport {
    run_slice(&SUITE, &TASKS, BASE_INDEX, notes)
}

/// Test cases held and mutants rejected, the evaluator-validity numbers.
fn notes(report: &SliceReport) -> Vec<String> {
    let (held, run) = report.check_totals("mock");
    vec![format!(
        "test cases (door admission plus scenarios): {held}/{run}; mutants rejected {}/{}",
        report.caught("mock"),
        report.tasks()
    )]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::Backend;

    fn frame(tick: u32, hp: [u16; 2], pos: [[f32; 3]; 2]) -> Frame {
        Frame {
            tick,
            pos: pos.to_vec(),
            hp: hp.to_vec(),
            damage: vec![0, 0],
            idle: vec![true, true],
            killed: false,
        }
    }

    const HERE: [[f32; 3]; 2] = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0]];

    fn outcome(id: &str, sub: Submission) -> Outcome {
        let task = TASKS.iter().find(|task| task.id == id).expect("task id");
        (task.run)(&mut Door::new(Backend::Mock, 0), sub)
    }

    #[test]
    fn health_rule_names_the_tick_it_rose() {
        let frames = vec![
            frame(0, [1000, 1000], HERE),
            frame(1, [900, 1000], HERE),
            frame(2, [950, 1000], HERE),
        ];
        let violation = hp_never_rises(&frames).expect_err("a heal must be caught");
        assert_eq!(violation.tick, 2);
        assert!(hp_never_rises(&frames[..2]).is_ok());
    }

    #[test]
    fn every_rule_refuses_an_empty_recording() {
        let rules: [Rule; 9] = [
            hp_never_rises,
            damage_only_grows,
            stays_in_arena,
            pinned_fighter_is_idle,
            walk_input_moves_fighter,
            knockout_holds_pose,
            kill_flag_matches_health,
            state_stays_sane,
            idle_match_stays_still,
        ];
        for rule in rules {
            assert!(rule(&[]).is_err(), "an empty run must never satisfy a rule");
        }
    }

    #[test]
    fn knockout_rules_refuse_a_run_with_no_knockout() {
        let frames = vec![frame(0, [1000, 1000], HERE), frame(1, [1000, 900], HERE)];
        assert_eq!(
            knockout_holds_pose(&frames).expect_err("vacuous run").why,
            "no knockout to assert on"
        );
        assert!(kill_flag_matches_health(&frames).is_err());
    }

    #[test]
    fn knockout_rule_catches_a_fallen_fighter_that_moves() {
        let mut fallen = HERE;
        let still = vec![
            frame(0, [1000, 100], HERE),
            frame(1, [1000, 0], HERE),
            frame(2, [1000, 0], HERE),
        ];
        assert!(knockout_holds_pose(&still).is_ok());
        fallen[1][0] += 0.5;
        let drift = vec![
            frame(0, [1000, 100], HERE),
            frame(1, [1000, 0], HERE),
            frame(2, [1000, 0], fallen),
        ];
        assert_eq!(knockout_holds_pose(&drift).expect_err("drift").tick, 2);
    }

    #[test]
    fn kill_flag_rule_pins_the_firing_tick() {
        let mut good = vec![
            frame(0, [100, 100], HERE),
            frame(1, [0, 100], HERE),
            frame(2, [0, 100], HERE),
        ];
        good[0].killed = true;
        assert!(kill_flag_matches_health(&good).is_ok());
        let mut early = vec![
            frame(0, [100, 100], HERE),
            frame(1, [100, 100], HERE),
            frame(2, [0, 100], HERE),
        ];
        early[0].killed = true;
        assert_eq!(
            kill_flag_matches_health(&early)
                .expect_err("early flag")
                .tick,
            0
        );
    }

    #[test]
    fn kill_flag_rule_rejects_a_spurious_flag_on_the_final_tick() {
        let mut frames = vec![
            frame(0, [100, 100], HERE),
            frame(1, [0, 100], HERE),
            frame(2, [0, 100], HERE),
        ];
        frames[0].killed = true;
        assert!(
            kill_flag_matches_health(&frames).is_ok(),
            "the clean knockout must hold: {:?}",
            kill_flag_matches_health(&frames)
        );
        frames[2].killed = true;
        assert_eq!(
            kill_flag_matches_health(&frames)
                .expect_err("a kill flag on the last tick has no next tick to explain it")
                .tick,
            2
        );
    }

    #[test]
    fn arena_rule_reuses_the_agent_scan() {
        let mut out = HERE;
        out[1] = [HALF_EXTENT_M + 1.0, 0.0, 0.0];
        let frames = vec![frame(0, [1000, 1000], HERE), frame(1, [1000, 1000], out)];
        assert_eq!(stays_in_arena(&frames).expect_err("out of bounds").tick, 1);
        let mut nan = HERE;
        nan[0][2] = f32::NAN;
        assert!(stays_in_arena(&[frame(0, [1000, 1000], nan)]).is_err());
    }

    #[test]
    fn nonfinite_poses_are_rejected_by_movement_rules() {
        let mut nan = HERE;
        nan[0][0] = f32::NAN;
        let mut walk_frames = Vec::new();
        for tick in 0..=WALK_TICKS {
            walk_frames.push(frame(tick, [1000, 1000], nan));
        }
        assert!(
            walk_input_moves_fighter(&walk_frames).is_err(),
            "NaN walk must not satisfy the move bar"
        );
        let fallen_nan = vec![
            frame(0, [1000, 100], HERE),
            frame(1, [1000, 0], HERE),
            frame(2, [1000, 0], nan),
        ];
        assert!(
            knockout_holds_pose(&fallen_nan).is_err(),
            "NaN pose must not hold the knockout"
        );
        let idle_nan = vec![frame(0, [1000, 1000], HERE), frame(1, [1000, 1000], nan)];
        assert!(
            idle_match_stays_still(&idle_nan).is_err(),
            "NaN idle must not stay still"
        );
        assert!(state_stays_sane(&[frame(0, [1000, 1000], HERE)]).is_ok());
    }

    #[test]
    fn vertical_drift_breaks_knockout_and_idle() {
        let mut sunk = HERE;
        sunk[1][1] -= 0.5;
        let fallen_sunk = vec![
            frame(0, [1000, 100], HERE),
            frame(1, [1000, 0], HERE),
            frame(2, [1000, 0], sunk),
        ];
        assert!(
            knockout_holds_pose(&fallen_sunk).is_err(),
            "sunk knockout must not hold the pose"
        );
        let idle_sunk = vec![frame(0, [1000, 1000], HERE), frame(1, [1000, 1000], sunk)];
        assert!(
            idle_match_stays_still(&idle_sunk).is_err(),
            "sunk idle must not stay still"
        );
    }

    #[test]
    fn pose_distance_comes_from_the_core_owner_bit_exact() {
        use engine_core::vec3::dist2;
        // The pose holds below pin the full 3D pose through the engine-core
        // squared distance owner, so this pins the owner wiring bit-exactly:
        // same inputs must give the same bits, and a vertical-only sink must
        // read dy squared while the planar walk helper stays blind to it.
        let fixed = [
            ([0.0_f32, 0.0, 0.0], [1.0_f32, 2.0, 3.0]),
            ([1.0_f32, 0.0, 0.0], [1.0_f32, -0.5, 0.0]),
            ([-2.0_f32, 5.0, 1.0], [3.0_f32, -1.0, 4.0]),
        ];
        for (a, b) in fixed {
            let (dx, dy, dz) = (a[0] - b[0], a[1] - b[1], a[2] - b[2]);
            let hand = dx * dx + dy * dy + dz * dz;
            assert_eq!(dist2(a, b).to_bits(), hand.to_bits());
            assert_eq!(dist2(b, a).to_bits(), hand.to_bits());
        }
        let (hi, sunk) = ([0.0_f32, 0.0, 0.0], [0.0_f32, -0.5, 0.0]);
        assert_eq!(planar_sq(hi, sunk).to_bits(), 0.0_f32.to_bits());
        assert_eq!(dist2(hi, sunk).to_bits(), 0.25_f32.to_bits());
    }

    #[test]
    fn real_arena_runs_satisfy_every_invariant_on_every_seed() {
        let brains = SlotTape::default();
        for seed in SEEDS {
            let frames = record(seed, SHORT_TICKS, &brains);
            assert_eq!(frames.len(), 120, "seed {seed}");
            hp_never_rises(&frames).expect("health");
            damage_only_grows(&frames).expect("damage");
            stays_in_arena(&frames).expect("bounds");
            state_stays_sane(&frames).expect("range");
        }
    }

    #[test]
    fn reference_submissions_pass_and_every_mutant_is_rejected() {
        for task in &TASKS {
            let reference = outcome(task.id, Submission::Reference);
            assert!(reference.pass, "{}: {reference:?}", task.id);
            let weak = outcome(task.id, Submission::Weak);
            assert!(!weak.pass, "{}: the mutant must fail: {weak:?}", task.id);
            assert_ne!(
                reference.hash, weak.hash,
                "{}: the mutant left no trace",
                task.id
            );
        }
    }

    #[test]
    fn mutants_are_named_at_the_seeded_tick_and_case() {
        let weak = outcome("gl01", Submission::Weak);
        assert_eq!(weak.stage("seed-1"), Some(false));
        assert_eq!(weak.stage("seed-0"), Some(true));
        assert!(
            weak.detail.contains("seed-1: health rose at tick 40"),
            "{}",
            weak.detail
        );
        let drift = outcome("gl06", Submission::Weak);
        assert!(drift.detail.contains("moved or healed"), "{}", drift.detail);
        let replay = outcome("gl07", Submission::Weak);
        assert!(
            replay.detail.contains("diverged at checkpoint"),
            "{}",
            replay.detail
        );
    }

    #[test]
    fn two_different_correct_pins_are_both_accepted() {
        let reference = outcome("gl04", Submission::Reference);
        assert_eq!(reference.stage("pinned-a"), Some(true));
        assert_eq!(reference.stage("pinned-b"), Some(true));
        let weak = outcome("gl04", Submission::Weak);
        assert_eq!(
            weak.stage("pinned-a"),
            Some(true),
            "the correct half stays accepted"
        );
        assert_eq!(weak.stage("pinned-b"), Some(false));
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
