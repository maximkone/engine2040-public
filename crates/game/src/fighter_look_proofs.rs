//! fighter_look_proofs: end-to-end proofs on the real CT7 bodies, no window.
//!
//! Consumer: `cargo test` only (child of [`super`], so it reads the private
//! pose path the window draws through). Data owner: none; it owns no state.
//! Units: meters, fixed ticks at 60 Hz. Determinism: every run is a seeded
//! headless fight or a scripted tick list, so the same code gives the same
//! numbers twice. Errors: a failed assertion names the tick and the number.
#![cfg(test)]

use std::path::PathBuf;

use engine_anim::contract::skinning_matrices;
use engine_anim::ik::{limb_positions, solve_limb, Limb};
use engine_arena::GaitPhase;
use engine_audio::FightCue;
use engine_combat::fight::HITSTOP_TICKS;
use engine_core::vec3::dist2;
use engine_render::v1::Camera;

use super::fighter_look_feet::{Legs, Plant};
use super::{run_phase_scale, Act, FighterLook, RUN_REF_M_PER_TICK, TICK_HZ};
use crate::view_truth::{BodyTruth, ViewLaws, GROUND_Y};
use crate::FighterView;

fn content_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../content/fighters")
}

fn view(slot: u8, x: f32, z: f32) -> FighterView {
    FighterView {
        slot,
        pos_m: [x, 0.0, z],
        yaw: 0.0,
    }
}

fn arena_camera() -> Camera {
    Camera::new(
        [0.0, 5.0, 12.0],
        [0.0, 1.0, 0.0],
        [0.0, 1.0, 0.0],
        50.0f32.to_radians(),
    )
}

/// What a viewer sees of one body's pose, bit for bit: clip, clip clock,
/// model-space joints and facing. Root travel is left out on purpose: the
/// sim owns where the feet are, the look owns how the body is posed.
#[derive(Clone, PartialEq, Debug)]
struct Shape {
    clip: u8,
    clip_s: f32,
    yaw: f32,
    joints: Vec<[f32; 3]>,
}

/// The shapes of every seen slot at sub-tick `alpha`, indexed by slot.
fn shapes(look: &FighterLook, fighters: usize, alpha: f32) -> Vec<Option<Shape>> {
    let mut out = vec![None; fighters];
    for body in look.pose_frame(fighters, alpha).expect("CPU pose") {
        let truth = body.truth;
        out[usize::from(truth.slot)] = Some(Shape {
            clip: truth.clip,
            clip_s: truth.clip_s,
            yaw: truth.yaw,
            joints: truth.joints,
        });
    }
    out
}

/// The `sunk_cm_max` the view laws print for these bodies.
fn sunk_cm_max(bodies: &[BodyTruth]) -> f32 {
    let mut laws = ViewLaws::default();
    laws.note(&arena_camera(), 1.0, bodies);
    let line = laws.line().expect("one frame folded");
    let tail = line
        .split("sunk_cm_max=")
        .nth(1)
        .expect("the receipt names sunk_cm_max");
    tail.split_whitespace()
        .next()
        .and_then(|number| number.parse().ok())
        .expect("sunk_cm_max is a number")
}

fn is_dead(look: &FighterLook, slot: u8) -> bool {
    let anim = look.slots[usize::from(slot)];
    anim.act == Act::Down || anim.balance == GaitPhase::Fallen
}

#[test]
fn sand_rest_scripted_deaths_lie_on_the_sand_through_the_whole_fall() {
    // Four bodies die one after another. Even slots die with the clip
    // playing from the standing pose, odd slots with the sim's `Fallen`
    // verdict (the flat pose at once). The lowest vertex of every dead body
    // sits on the sand on every frame of the fall and at rest.
    let mut look = FighterLook::load(&content_dir()).expect("CT7 bodies load");
    let ring = [
        view(0, -1.0, 0.0),
        view(1, 1.0, 0.0),
        view(2, 0.0, 1.8),
        view(3, 0.0, -1.8),
    ];
    let mut hp = [1000u16; 4];
    let mut dead_frames = 0u32;
    let mut worst_cm = 0.0f32;
    for tick in 0..140u32 {
        let mut cues = Vec::new();
        for victim in 0..4u8 {
            if tick == 20 + 10 * u32::from(victim) {
                hp[usize::from(victim)] = 0;
                cues.push(FightCue::Knockdown {
                    attacker: (victim + 1) % 4,
                    victim,
                });
            }
        }
        look.observe_tick(tick, &ring, &hp, &cues);
        for victim in [1u8, 3] {
            if tick == 20 + 10 * u32::from(victim) {
                look.observe_balance(victim, GaitPhase::Fallen);
            }
        }
        for alpha in [0.0f32, 0.5, 0.99] {
            let posed = look.pose_frame(4, alpha).expect("CPU pose");
            let dead: Vec<BodyTruth> = posed
                .iter()
                .filter(|body| is_dead(&look, body.truth.slot))
                .map(|body| body.truth.clone())
                .collect();
            for body in &dead {
                dead_frames += 1;
                let off_cm = (body.min[1] - GROUND_Y).abs() * 100.0;
                worst_cm = worst_cm.max(off_cm);
                assert!(
                    off_cm < 1.0,
                    "tick {tick} alpha {alpha} slot {}: lowest vertex {off_cm:.1} cm off the sand",
                    body.slot
                );
            }
            assert!(
                sunk_cm_max(&dead) < 2.0,
                "tick {tick}: the receipt must read sunk_cm_max under 2"
            );
        }
    }
    assert!(
        dead_frames > 300,
        "the script posed {dead_frames} dead frames"
    );
    assert!(worst_cm < 1.0, "worst rest error {worst_cm:.2} cm");
}

#[test]
fn sand_rest_real_fight_dead_lie_on_the_sand_and_the_living_never_sink() {
    // The window's own session: 8 fighters, the duel's tables, 900 ticks of
    // brains fighting. Every body the sim has floored or killed rests on the
    // sand, and no living body is drawn under it: a dip is lifted to the sand
    // and a flight phase is never pulled down.
    let config = crate::GameConfig {
        seed: 2040,
        ticks: 900,
        fighters: 8,
        half_extent_m: 20.0,
    };
    let mut app = crate::App::new(config)
        .and_then(crate::App::into_duel)
        .expect("duel builds");
    let mut look = FighterLook::load(&content_dir()).expect("CT7 bodies load");
    // This proof is the flat-pose rest: a ragdoll rests by its physics instead (RZ2w, proved in
    // `ragdoll_look`), so the layer is off here.
    look.set_ragdoll(false);
    let (mut dead_frames, mut worst_cm, mut living_lifted) = (0u32, 0.0f32, 0u32);
    for tick in 0..900u32 {
        let out = app.tick_empty();
        look.observe_hitstop(tick, &out.cues, app.hitstop_remaining());
        look.observe_tick(tick, app.fighter_views(), app.hp(), &out.cues);
        look.observe_live_balances(app.live_match(), 8);
        for body in look.pose_frame(8, 0.5).expect("CPU pose") {
            if is_dead(&look, body.truth.slot) {
                dead_frames += 1;
                let off_cm = (body.truth.min[1] - GROUND_Y).abs() * 100.0;
                worst_cm = worst_cm.max(off_cm);
                assert!(
                    off_cm < 1.0,
                    "tick {tick} slot {}: dead body {off_cm:.1} cm off the sand",
                    body.truth.slot
                );
            } else {
                assert!(
                    body.truth.min[1] >= GROUND_Y - 0.01,
                    "tick {tick} slot {}: a living body is {:.1} cm under the sand",
                    body.truth.slot,
                    (GROUND_Y - body.truth.min[1]) * 100.0
                );
                assert!(
                    body.lift >= 0.0,
                    "tick {tick} slot {}: the sand lift never pushes a body down",
                    body.truth.slot
                );
                if body.lift > 0.0 {
                    living_lifted += 1;
                }
            }
        }
    }
    assert!(dead_frames > 100, "the fight floored bodies: {dead_frames}");
    assert!(worst_cm < 1.0, "worst rest error {worst_cm:.2} cm");
    assert!(
        living_lifted > 0,
        "the fight dipped a living body in some frame (the law must bite): 0 lifts"
    );
}

/// The three integer clocks that drive a slot's pose: clip phase in ticks,
/// blend ticks left and the flinch age (`None` while no flinch is recorded).
#[derive(Clone, Copy, PartialEq, Debug)]
struct Clocks {
    act: Act,
    since: u32,
    fade: u32,
    flinch: Option<u32>,
}

fn clocks(look: &FighterLook, slot: u8) -> Clocks {
    let anim = look.slots[usize::from(slot)];
    Clocks {
        act: anim.act,
        since: look.tick.saturating_sub(anim.act_start),
        fade: anim.fade_left,
        flinch: (anim.hit_start != u32::MAX).then(|| look.tick.wrapping_sub(anim.hit_start)),
    }
}

/// How far two shapes differ, for a failure message: clip, yaw and the worst
/// joint gap in centimeters.
fn gap(now: &Option<Shape>, then: &Option<Shape>) -> String {
    let (Some(now), Some(then)) = (now, then) else {
        return "a body vanished".to_string();
    };
    let worst_m = now
        .joints
        .iter()
        .zip(&then.joints)
        .map(|(a, b)| {
            ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
        })
        .fold(0.0f32, f32::max);
    format!(
        "clip {} vs {}, clip clock gap {:.4} s, yaw gap {:.4} rad, worst joint gap {:.1} cm",
        now.clip,
        then.clip,
        (now.clip_s - then.clip_s).abs(),
        (now.yaw - then.yaw).abs(),
        worst_m * 100.0
    )
}

/// One qualifying impact window: a single Hit between two living fighters,
/// the shapes (per alpha) and clocks at the impact tick.
struct Window {
    attacker: u8,
    victim: u8,
    impact: u32,
    /// The live freeze the impact set (3 light, 6 heavy, 8 parry under v2).
    freeze: u32,
    frozen: Vec<Vec<Option<Shape>>>,
    clocks: [Clocks; 2],
}

const ALPHAS: [f32; 3] = [0.0, 0.5, 0.99];

#[test]
fn hitstop_hold_posed_pair_does_not_move_through_the_freeze() {
    // LF10 accept, on the real bodies: in a 300-tick four-fighter duel, every
    // time one Hit lands the attacker and the victim hold the exact pose of
    // the impact tick for the whole freeze, at every sub-tick alpha, while
    // the bystanders keep moving; the tick the sim thaws, the pair's clocks
    // resume one tick on from where the hit froze them, no jump.
    let config = crate::GameConfig {
        seed: 2,
        ticks: 300,
        fighters: 4,
        half_extent_m: 20.0,
    };
    let mut app = crate::App::new(config)
        .and_then(crate::App::into_duel)
        .expect("duel builds");
    let mut look = FighterLook::load(&content_dir()).expect("CT7 bodies load");
    let mut window: Option<Window> = None;
    let (mut windows, mut held_ticks, mut thaws) = (0u32, 0u32, 0u32);
    let mut held_owed = 0u32;
    let (mut bystanders_moved, mut blends_held, mut resumed) = (0u32, 0u32, 0u32);
    for tick in 0..300u32 {
        let out = app.tick_empty();
        // FEEL3: the pair holds the live fight's count (3, 6 or 8 under v2), read after the tick.
        let remaining = app.hitstop_remaining();
        look.observe_hitstop(tick, &out.cues, remaining);
        look.observe_tick(tick, app.fighter_views(), app.hp(), &out.cues);
        look.observe_live_balances(app.live_match(), 4);
        let impacts = out
            .cues
            .iter()
            .filter(|cue| matches!(cue, FightCue::Hit { .. } | FightCue::Parried { .. }))
            .count();
        if let Some(open) = window.take() {
            let age = tick - open.impact;
            let pair = [open.attacker, open.victim];
            if remaining > 0 && impacts == 0 {
                // A held tick of the sim's freeze: the pair must not move.
                for (index, alpha) in ALPHAS.iter().enumerate() {
                    let now = shapes(&look, 4, *alpha);
                    for slot in pair {
                        let got = &now[usize::from(slot)];
                        let want = &open.frozen[index][usize::from(slot)];
                        assert!(
                            got == want,
                            "tick {tick} (impact {} + {age}) alpha {alpha}: slot {slot} moved in the freeze: {}",
                            open.impact,
                            gap(got, want)
                        );
                    }
                    if age + 1 == open.freeze {
                        for (other, before) in open.frozen[index].iter().enumerate() {
                            let paired = pair.contains(&u8::try_from(other).unwrap_or(u8::MAX));
                            if let (false, Some(before), Some(after)) =
                                (paired, before, &now[other])
                            {
                                if before.clip == after.clip && after.clip_s > before.clip_s {
                                    bystanders_moved += 1;
                                }
                            }
                        }
                    }
                }
                for (which, slot) in pair.into_iter().enumerate() {
                    assert_eq!(
                        clocks(&look, slot),
                        open.clocks[which],
                        "tick {tick} (impact {} + {age}): slot {slot} clocks moved in the freeze",
                        open.impact
                    );
                }
                blends_held += u32::from(open.clocks.iter().any(|c| c.fade > 0));
                held_ticks += 1;
                window = Some(open);
            } else if remaining == 0 && impacts == 0 && age == open.freeze {
                // The sim thawed: every clock resumes one tick on.
                let now = shapes(&look, 4, 0.5);
                for (which, slot) in pair.into_iter().enumerate() {
                    let (was, is) = (open.clocks[which], clocks(&look, slot));
                    // A slot whose act changes on the thaw tick (the victim
                    // stops running) starts a fresh clip; the rest resume.
                    if was.act == is.act {
                        assert_eq!(
                            is.since,
                            was.since + 1,
                            "tick {tick}: slot {slot} clip clock"
                        );
                        assert_eq!(
                            is.fade,
                            was.fade.saturating_sub(1),
                            "tick {tick}: slot {slot} blend"
                        );
                        resumed += 1;
                    }
                    if let (Some(was), Some(is)) = (was.flinch, is.flinch) {
                        assert_eq!(is, was + 1, "tick {tick}: slot {slot} flinch clock");
                    }
                    assert!(
                        now[usize::from(slot)] != open.frozen[1][usize::from(slot)],
                        "tick {tick}: slot {slot} must move once the freeze is over"
                    );
                }
                thaws += 1;
                held_owed += open.freeze.saturating_sub(1);
            }
        }
        let single_hit = impacts == 1 && remaining > 0;
        if window.is_none() && single_hit {
            let pair = out.cues.iter().find_map(|cue| match *cue {
                FightCue::Hit {
                    attacker, victim, ..
                } => Some((attacker, victim)),
                _ => None,
            });
            let both_up = pair.is_some_and(|(a, v)| {
                !matches!(look.act(usize::from(a)), None | Some(Act::Down))
                    && !matches!(look.act(usize::from(v)), None | Some(Act::Down))
            });
            if let (Some((attacker, victim)), true) = (pair, both_up) {
                // Even the impact tick's own frames hold one pose: the same
                // shape at every sub-tick alpha, no frame plays on.
                let steady = ALPHAS.map(|a| shapes(&look, 4, a));
                for slot in [attacker, victim] {
                    let at = |index: usize| &steady[index][usize::from(slot)];
                    assert!(
                        at(0) == at(1) && at(1) == at(2),
                        "tick {tick}: slot {slot} plays on between ticks: {}",
                        gap(at(0), at(2))
                    );
                }
                window = Some(Window {
                    attacker,
                    victim,
                    impact: tick,
                    freeze: remaining,
                    frozen: ALPHAS.iter().map(|a| shapes(&look, 4, *a)).collect(),
                    clocks: [clocks(&look, attacker), clocks(&look, victim)],
                });
                windows += 1;
            }
        }
    }
    eprintln!(
        "hitstop proof: {windows} windows, {held_ticks} held ticks ({blends_held} with a blend in flight), {thaws} thaws ({resumed} clocks resumed in place), {bystanders_moved} bystander advances"
    );
    assert!(windows >= 3, "the fight landed {windows} clean impacts");
    assert!(
        held_ticks >= held_owed,
        "held ticks {held_ticks} for {held_owed} owed by the thawed freezes"
    );
    assert!(thaws >= 2, "the freeze thawed {thaws} times");
    assert!(resumed >= 2, "{resumed} clocks resumed in place");
    assert!(
        bystanders_moved > 0,
        "bystanders keep moving while the pair holds"
    );
    assert!(
        blends_held > 0,
        "a freeze caught a blend in flight, so the blend hold is exercised"
    );
}

/// One field of the `arena view:` receipt line, as text.
fn receipt_field<'a>(line: &'a str, key: &str) -> &'a str {
    line.split(key)
        .nth(1)
        .and_then(|tail| tail.split_whitespace().next())
        .unwrap_or_else(|| panic!("the receipt names {key}: {line}"))
}

/// The three-frame-or-one-frame view of a 300-tick four-fighter duel: the
/// `arena view:` line and, for every frame a foot is at full plant, how far
/// the drawn ankle sits (in world centimeters) from where the drawn ankle was
/// when that plant began, read off the posed joints and not off the plant's
/// own bookkeeping.
struct DuelFeet {
    line: String,
    drift_cm: Vec<f32>,
}

/// Joint indices of the two ankles on the hero rig, by name.
fn ankle_joints(look: &FighterLook) -> [usize; 2] {
    let joints = &look.rigs[0].skeleton.joints;
    ["foot_l", "foot_r"].map(|name| {
        joints
            .iter()
            .position(|joint| joint.name == name)
            .unwrap_or_else(|| panic!("the rig has a {name} joint"))
    })
}

/// World `xz` of a body's joint: model space under the body's own root.
fn world_xz(body: &BodyTruth, joint: usize) -> [f32; 2] {
    let (s, c) = body.yaw.sin_cos();
    let p = body.joints[joint];
    [
        p[0] * c + p[2] * s + body.root[0],
        -p[0] * s + p[2] * c + body.root[2],
    ]
}

/// A 300-tick four-fighter duel (seed 2) posed at `frames_per_tick` sub-tick
/// frames per sim tick, counted by [`ViewLaws`] exactly as the window counts
/// its frames. With `ik` off the plants are tracked and metered but the pose
/// stays the clip's.
fn duel_feet(frames_per_tick: u16, ik: bool) -> DuelFeet {
    let config = crate::GameConfig {
        seed: 2,
        ticks: 300,
        fighters: 4,
        half_extent_m: 20.0,
    };
    let mut app = crate::App::new(config)
        .and_then(crate::App::into_duel)
        .expect("duel builds");
    let mut look = FighterLook::load(&content_dir()).expect("CT7 bodies load");
    // The clip-switch and planted-feet laws are judged on the clip pipeline; a ragdoll's rise
    // has its own budget in `ragdoll_look` (RZ2w).
    look.set_ragdoll(false);
    for slot in &mut look.slots {
        slot.feet.ik = ik;
    }
    let ankles = ankle_joints(&look);
    let mut laws = ViewLaws::default();
    let camera = arena_camera();
    let mut starts: [[Option<[f32; 2]>; 2]; 4] = [[None; 2]; 4];
    let mut drift_cm = Vec::new();
    for tick in 0..300u32 {
        let out = app.tick_empty();
        look.observe_hitstop(tick, &out.cues, app.hitstop_remaining());
        look.observe_tick(tick, app.fighter_views(), app.hp(), &out.cues);
        look.observe_live_balances(app.live_match(), 4);
        if frames_per_tick == 1 && ik && tick % 2 == 0 {
            let slot = &look.slots[0];
            let feet: Vec<String> = slot
                .feet
                .plants
                .iter()
                .map(|p| {
                    format!(
                        "pl={} st={} a={:?} w={:?}",
                        u8::from(p.planted),
                        p.still,
                        p.ankle.map(|a| [a[0], a[1], a[2]].map(|v| (v * 1000.0).round())),
                        p.world.map(|w| w.map(|v| (v * 1000.0).round()))
                    )
                })
                .collect();
            eprintln!(
                "zzfeet t{tick} {:?}/{:?} rest={:?} {}",
                slot.act,
                slot.balance,
                look.rigs[0].legs.map(|l| l.rest_y),
                feet.join(" | ")
            );
        }
        for frame in 0..frames_per_tick {
            let alpha = (f32::from(frame) + 0.5) / f32::from(frames_per_tick);
            let bodies = look.pose_frame_joints(4, alpha).expect("joints pose");
            laws.note(&camera, f64::from(tick) + f64::from(alpha), &bodies);
            for body in &bodies {
                let slot = usize::from(body.slot);
                for (foot, joint) in ankles.into_iter().enumerate() {
                    if body.foot_slide_m[foot].is_none() {
                        starts[slot][foot] = None;
                        continue;
                    }
                    let here = world_xz(body, joint);
                    let start = *starts[slot][foot].get_or_insert(here);
                    drift_cm.push(100.0 * (here[0] - start[0]).hypot(here[1] - start[1]));
                }
            }
        }
    }
    eprintln!(
        "zzdiag duel_feet {frames_per_tick} ik={ik}: worst pop {:?}",
        laws.pop_detail()
    );
    DuelFeet {
        line: laws.line().expect("the duel folded frames"),
        drift_cm,
    }
}

/// The `arena view:` line of a 300-tick four-fighter duel posed at
/// `frames_per_tick` sub-tick frames per sim tick, counted by [`ViewLaws`]
/// exactly as the window counts its frames.
fn duel_view_line(frames_per_tick: u16) -> String {
    duel_feet(frames_per_tick, true).line
}

/// Pops the receipt counts in one act (`pop_acts=attack:3/run:9`), 0 when none.
fn pops_in(line: &str, act: &str) -> u32 {
    receipt_field(line, "pop_acts=")
        .split('/')
        .find_map(|part| part.strip_prefix(act)?.strip_prefix(':')?.parse().ok())
        .unwrap_or(0)
}

#[test]
fn clip_switches_do_not_pop_in_a_real_duel() {
    // LF9 accept, on the real bodies: the `arena view:` pop count of a
    // 300-tick four-fighter duel (seed 2). With the crossfade it read 148
    // pops at one frame per tick (attack 38, idle 46, run 64) and 333 at
    // three. Inertialized switches take the attack and idle ones to none: a
    // joint keeps its position and speed across the cut. The one idle pop
    // left is the sim's `Fallen` verdict snapping a body flat (EG45), not a
    // clip switch. The run clip carries a phase summed tick by tick, so a
    // speed change no longer jumps it: with elapsed time scaled instead the
    // run pops were 36 at one frame per tick and 19 at three. At three
    // frames per tick none are left. The 20 left at one frame per tick all
    // sit in the first 11 ticks of a run (clip time 0.07 to 0.32 s) at about
    // 1.65x rate, where the 24 Hz keys of the run clip put a corner every
    // 1.5 frames and the constant-velocity predictor misses it by 11 to 15 cm;
    // they read the same with the switch offsets taken away.
    for (frames, before, bound, run_bound) in [(1u16, 148u32, 21u32, 20u32), (3, 333, 1, 0)] {
        let line = duel_view_line(frames);
        eprintln!("lf9 {frames} frame(s) per tick: {line}");
        let pops: u32 = receipt_field(&line, "pops=")
            .parse()
            .expect("pops is a number");
        // The joints-only bounds floor at the lowest joint less the sole: the
        // skin-radius pad once read every idle body 1.1 m under the sand.
        let sunk = receipt_number(&line, "sunk_cm_max=");
        assert!(
            sunk < 3.0,
            "{frames} frame(s) per tick: bodies sunk {sunk} cm: {line}"
        );
        assert_eq!(
            pops_in(&line, "attack"),
            0,
            "{frames} frame(s) per tick: attack switches must not pop: {line}"
        );
        assert!(
            pops_in(&line, "idle") <= 1,
            "{frames} frame(s) per tick: idle switches must not pop: {line}"
        );
        assert!(
            pops_in(&line, "run") <= run_bound,
            "{frames} frame(s) per tick: run pops past the {run_bound} reached: {line}"
        );
        assert!(
            pops <= bound,
            "{frames} frame(s) per tick: {pops} pops past the {bound} reached: {line}"
        );
        assert!(
            pops < before,
            "{frames} frame(s) per tick: {pops} pops, no better than the crossfade's {before}"
        );
    }
}

/// Sub-tick alphas a scripted runner is sampled at: four frames per tick.
const QUARTERS: [f32; 4] = [0.0, 0.25, 0.5, 0.75];
/// Slack for float sums of clip seconds.
const CLIP_EPS_S: f32 = 1.0e-5;

/// Clip seconds the run clip shows for `slot` at `alpha`; `None` while the
/// slot is not running.
fn run_time(look: &FighterLook, slot: usize, alpha: f32) -> Option<f32> {
    let anim = look.slots[slot];
    if anim.act != Act::Run {
        return None;
    }
    let rig = &look.rigs[look.rig_for_slot(slot)];
    let (clip, time) = look.clip_pick(rig, &anim, alpha);
    assert_eq!(clip, 1, "the run act plays the run clip");
    Some(time)
}

/// Clip seconds a quarter tick covers when the run clip plays at `scale`.
fn quarter_tick_s(scale: f32) -> f32 {
    0.25 * scale / TICK_HZ
}

#[test]
fn run_clip_time_stays_continuous_through_a_speed_change() {
    // A speed change changes the stride rate, never the clip phase. A runner
    // goes 0.5x, 1.5x, 0.5x of the reference speed for a dozen ticks each,
    // stands still, then runs again. Sampled four frames per tick, every
    // frame's clip clock moves on forward by a quarter tick at a rate the
    // runner shows, across the ticks where the speed flips too; a fresh run
    // starts at phase 0.
    let mut look = FighterLook::load(&content_dir()).expect("CT7 bodies load");
    let shares: Vec<f32> = [(0.5f32, 12usize), (1.5, 12), (0.5, 12), (0.0, 10), (1.0, 8)]
        .iter()
        .flat_map(|&(share, ticks)| vec![share; ticks])
        .collect();
    let hp = [1000u16, 1000];
    let mut x = -3.0f32;
    look.observe_tick(0, &[view(0, x, 0.0), view(1, 5.0, 0.0)], &hp, &[]);
    let mut before: Option<(f32, f32)> = None;
    let (mut flips, mut runs, mut frames) = (0u32, 0u32, 0u32);
    for (tick, share) in (1u32..).zip(&shares) {
        x += share * RUN_REF_M_PER_TICK;
        look.observe_tick(tick, &[view(0, x, 0.0), view(1, 5.0, 0.0)], &hp, &[]);
        let scale = run_phase_scale(share * RUN_REF_M_PER_TICK);
        for alpha in QUARTERS {
            let Some(time) = run_time(&look, 0, alpha) else {
                before = None;
                continue;
            };
            if let Some((was, was_scale)) = before {
                let step = time - was;
                let (slow, fast) = (was_scale.min(scale), was_scale.max(scale));
                assert!(
                    step >= quarter_tick_s(slow) - CLIP_EPS_S
                        && step <= quarter_tick_s(fast) + CLIP_EPS_S,
                    "tick {tick} alpha {alpha}: the clip clock moved {step} s in a quarter tick at rates {was_scale} then {scale}"
                );
                flips += u32::from((was_scale - scale).abs() > 0.5);
            } else {
                assert!(
                    time.abs() <= CLIP_EPS_S,
                    "tick {tick}: a fresh run starts at phase 0, not {time} s"
                );
                runs += 1;
            }
            before = Some((time, scale));
            frames += 1;
        }
    }
    assert_eq!(flips, 2, "both speed flips were sampled");
    assert_eq!(runs, 2, "the runner stopped and started again once");
    assert!(frames > 150, "{frames} running frames sampled");
}

#[test]
fn run_clip_time_holds_in_hitstop_and_resumes_in_phase() {
    // The impact freezes a running victim for HITSTOP_TICKS ticks: every frame
    // of the freeze shows one clip time, the end of the tick before, and the
    // thaw tick picks up exactly there, even when the sim thaws at a new speed.
    let mut look = FighterLook::load(&content_dir()).expect("CT7 bodies load");
    let hp = [1000u16, 1000];
    let (impact, thaw) = (10u32, 10 + HITSTOP_TICKS);
    let mut x = 8.0f32;
    look.observe_tick(0, &[view(0, 0.0, 0.0), view(1, x, 0.0)], &hp, &[]);
    let (mut last_free, mut held_s) = (None, None);
    let (mut held_frames, mut thawed_frames) = (0u32, 0u32);
    for tick in 1..=thaw + 2 {
        // The sim stands still while it holds, then runs on faster.
        let share = if tick <= impact {
            0.5
        } else if tick < thaw {
            0.0
        } else {
            1.5
        };
        x -= share * RUN_REF_M_PER_TICK;
        let cues = if tick == impact {
            vec![FightCue::Parried {
                attacker: 0,
                victim: 1,
            }]
        } else {
            Vec::new()
        };
        look.observe_tick(tick, &[view(0, 0.0, 0.0), view(1, x, 0.0)], &hp, &cues);
        for alpha in QUARTERS {
            let time = run_time(&look, 1, alpha)
                .unwrap_or_else(|| panic!("tick {tick}: the victim stopped running"));
            if tick < impact {
                last_free = Some(time);
            } else if tick < thaw {
                assert!(look.slots[1].held, "tick {tick}: the victim is held");
                let first = *held_s.get_or_insert_with(|| {
                    let free: f32 = last_free.expect("a free frame came first");
                    let want = free + quarter_tick_s(0.5);
                    assert!(
                        (time - want).abs() <= CLIP_EPS_S,
                        "the freeze starts at the end of the free tick: {time} s, wanted {want} s"
                    );
                    time
                });
                assert!(
                    (time - first).abs() <= CLIP_EPS_S,
                    "tick {tick} alpha {alpha}: the clip clock moved in the freeze, {first} s to {time} s"
                );
                held_frames += 1;
            } else {
                let held: f32 = held_s.expect("the freeze came first");
                let ticks_on = f32::from(u16::try_from(tick - thaw).expect("a few ticks"));
                let want =
                    held + (ticks_on + alpha) * run_phase_scale(1.5 * RUN_REF_M_PER_TICK) / TICK_HZ;
                assert!(
                    (time - want).abs() <= CLIP_EPS_S,
                    "tick {tick} alpha {alpha}: after the thaw {time} s, wanted {want} s"
                );
                thawed_frames += 1;
            }
        }
    }
    assert_eq!(
        held_frames,
        4 * HITSTOP_TICKS,
        "every frame of the freeze was sampled"
    );
    assert_eq!(thawed_frames, 12, "three ticks after the thaw were sampled");
}

/// The 95th percentile of `values` (the smallest one covering 95 of 100).
fn p95(values: &[f32]) -> f32 {
    let mut sorted = values.to_vec();
    sorted.sort_by(f32::total_cmp);
    let rank = (sorted.len() * 95).div_ceil(100).max(1);
    sorted[rank - 1]
}

/// A number field of the `arena view:` receipt.
fn receipt_number(line: &str, key: &str) -> f32 {
    receipt_field(line, key)
        .parse()
        .unwrap_or_else(|_| panic!("{key} is a number: {line}"))
}

#[test]
fn planted_feet_hold_their_spot_in_a_real_duel() {
    // LF12 accept on the real bodies, in the 300-tick four-fighter duel (seed
    // 2). Feet the clip holds still plant, and the slide the receipt meters
    // (`foot_slide_cm`, the p95 over planted foot-frames of how far the drawn
    // ankle sits from its spot) stays under 2 cm with the plant on and reads
    // higher with it off. The same drawn ankles, read off the posed joints
    // and not off the plant's bookkeeping, agree. The pop counts of the LF9
    // proof (attack 0, idle 1, run 20 at one frame per tick) do not move.
    // Measured 2026-10-17, one frame per tick: 108 planted foot-frames, p95
    // 0.7 cm on against 1.0 cm off (median 0.06 against 0.26 cm); three
    // frames per tick: 324 planted, 0.7 against 0.9 cm.
    for (frames, pop_bound, run_bound) in [(1u16, 21u32, 20u32), (3, 1, 0)] {
        let on = duel_feet(frames, true);
        let off = duel_feet(frames, false);
        eprintln!("lf12 {frames} frame(s) per tick, plant on : {}", on.line);
        eprintln!("lf12 {frames} frame(s) per tick, plant off: {}", off.line);
        let planted = receipt_number(&on.line, "planted_feet=");
        assert!(
            planted >= 100.0,
            "{frames} frame(s) per tick: only {planted} planted foot-frames"
        );
        assert_eq!(
            planted,
            receipt_number(&off.line, "planted_feet="),
            "the plant decides the same feet with the pose moved or not"
        );
        let (slide_on, slide_off) = (
            receipt_number(&on.line, "foot_slide_cm="),
            receipt_number(&off.line, "foot_slide_cm="),
        );
        assert!(
            slide_on < 2.0,
            "{frames} frame(s) per tick: planted feet slide {slide_on} cm"
        );
        assert!(
            slide_off > slide_on,
            "{frames} frame(s) per tick: the plant must remove slide ({slide_on} on, {slide_off} off)"
        );
        let (drift_on, drift_off) = (p95(&on.drift_cm), p95(&off.drift_cm));
        assert!(
            drift_on < 2.0 && drift_off > drift_on + 0.3,
            "{frames} frame(s) per tick: drawn ankle drift p95 {drift_on} cm on, {drift_off} cm off"
        );
        assert!(
            (drift_on - slide_on).abs() < 0.5,
            "the receipt ({slide_on} cm) and the drawn ankles ({drift_on} cm) agree"
        );
        let pops: u32 = receipt_field(&on.line, "pops=")
            .parse()
            .expect("pops is a number");
        assert_eq!(pops_in(&on.line, "attack"), 0, "{}", on.line);
        assert!(pops_in(&on.line, "idle") <= 1, "{}", on.line);
        assert!(pops_in(&on.line, "run") <= run_bound, "{}", on.line);
        assert!(
            pops <= pop_bound,
            "{pops} pops past {pop_bound}: {}",
            on.line
        );
        assert!(
            pops <= receipt_field(&off.line, "pops=").parse().unwrap_or(0),
            "the plant adds no pop: {} then {}",
            off.line,
            on.line
        );
    }
}

#[test]
fn bg5_a_steady_run_hands_its_foot_plants_to_the_audio_side() {
    // The runner moves at the reference speed, so the run clip plays at its
    // authored rate: one clip tick a sim tick. The lane hands the audio feed
    // its two plants an eighth and five eighths into the cycle, then again
    // one half cycle later, on the sim tick each phase crossed them.
    let mut look = loaded_look(true);
    let hp = [1000u16, 1000];
    let mut x = -3.0f32;
    let (mut first_run, mut steps) = (None, Vec::new());
    for tick in 0..=90u32 {
        if tick > 0 {
            x += RUN_REF_M_PER_TICK;
        }
        look.observe_tick(tick, &[view(0, x, 0.0), view(1, 5.0, 0.0)], &hp, &[]);
        if first_run.is_none() && look.slots[0].act == Act::Run {
            first_run = Some(tick);
        }
        steps.extend(
            look.clip_footsteps()
                .iter()
                .map(|step| (step.tick, step.slot)),
        );
    }
    let first_run = first_run.expect("the runner starts");
    let cycle_ticks = look.rigs[look.rig_for_slot(0)].clips[1].duration * TICK_HZ;
    eprintln!("bg5 plants: {steps:?}, first run {first_run}, cycle {cycle_ticks} ticks");
    assert!(steps.len() >= 3, "a run plants its feet: {steps:?}");
    assert!(steps.iter().all(|&(_, slot)| slot == 0), "{steps:?}");
    // One tick of slack: the phase is a float sum of speeds.
    let close = |got: u32, want: f32| (got as f32 - want).abs() <= 1.0;
    assert!(
        close(steps[0].0, first_run as f32 + cycle_ticks / 8.0),
        "first plant {} is an eighth cycle after the run start: {steps:?}",
        steps[0].0
    );
    for pair in steps.windows(2) {
        let gap = pair[1].0 - pair[0].0;
        assert!(
            close(gap, cycle_ticks / 2.0),
            "plants sit half a cycle apart: {pair:?} of {steps:?}"
        );
    }
}

/// A look with the plant switched `ik` (on or off) and no match behind it.
fn loaded_look(ik: bool) -> FighterLook {
    let mut look = FighterLook::load(&content_dir()).expect("CT7 bodies load");
    for slot in &mut look.slots {
        slot.feet.ik = ik;
    }
    look
}

/// Slot 0 stands in guard with the rival straight ahead (+z, so it never
/// turns) and is shoved sideways 1 cm a tick (0.6 m/s) from tick 40 to 119;
/// slot 1 is the rival. Returns the `arena view:` line of slot 0 at three
/// frames per tick and how many times its feet let go of a plant.
fn shoved_guard(ik: bool) -> (String, u32) {
    let mut look = loaded_look(ik);
    let hp = [1000u16, 1000];
    let mut laws = ViewLaws::default();
    let camera = arena_camera();
    let (mut let_gos, mut was) = (0u32, [false; 2]);
    for tick in 0..120u32 {
        let shove = tick.saturating_sub(40);
        let views = [
            view(0, 0.01 * f32::from(u16::try_from(shove).unwrap_or(0)), 0.0),
            view(1, 0.0, 4.0),
        ];
        look.observe_tick(tick, &views, &hp, &[]);
        for (foot, held) in was.iter_mut().enumerate() {
            let now = look.slots[0].feet.plants[foot].planted;
            let_gos += u32::from(*held && !now);
            *held = now;
        }
        for frame in 0..3u16 {
            let alpha = (f32::from(frame) + 0.5) / 3.0;
            let bodies = look.pose_frame_joints(2, alpha).expect("joints pose");
            let guard: Vec<BodyTruth> = bodies.into_iter().filter(|b| b.slot == 0).collect();
            laws.note(&camera, f64::from(tick) + f64::from(alpha), &guard);
        }
    }
    (laws.line().expect("frames folded"), let_gos)
}

#[test]
fn a_shoved_guard_keeps_its_feet_on_the_sand_and_steps_when_stretched() {
    // The clip holds a guarding fighter's feet still while the sim shoves it
    // 0.6 m/s sideways for 80 ticks. Without the plant the planted feet ride
    // the shove: the slide reaches the 12 cm the plant lets go at. With it
    // the feet stay on their spots, let go and step home when the body has
    // carried them past 12 cm, and no joint pops (the foot never snaps).
    // Measured 2026-10-17: foot_slide_cm 0.0 on against 10.3 off, 8 let-gos,
    // pop_cm_max 0.5 on.
    let (on, let_gos) = shoved_guard(true);
    let (off, _) = shoved_guard(false);
    eprintln!("lf12 shoved guard, plant on : {on} ({let_gos} let-gos)");
    eprintln!("lf12 shoved guard, plant off: {off}");
    let (slide_on, slide_off) = (
        receipt_number(&on, "foot_slide_cm="),
        receipt_number(&off, "foot_slide_cm="),
    );
    assert!(slide_on < 2.0, "planted feet slide {slide_on} cm: {on}");
    assert!(
        slide_off >= 5.0,
        "without the plant the shove drags the feet {slide_off} cm: {off}"
    );
    assert!(
        receipt_number(&on, "planted_feet=") >= 300.0,
        "the guard plants: {on}"
    );
    assert!(
        let_gos >= 4,
        "a stretched plant lets go and steps: {let_gos}"
    );
    assert_eq!(receipt_field(&on, "pops="), "0", "{on}");
    assert!(
        receipt_number(&on, "pop_cm_max=") < 3.0,
        "the foot must not snap: {on}"
    );
}

#[test]
fn a_pulled_foot_drops_the_pelvis_and_the_free_leg_keeps_the_sand() {
    // A planted left foot dragged `pull` meters from where the clip puts it.
    // Inside what the leg reaches the pelvis stays; past it the pelvis drops
    // by the gap and the ankle lands on the spot; past the deepest drop the
    // leg stretches straight and stays finite. The right foot, which has no
    // plant, keeps the clip's place on the sand through every drop.
    let mut look = loaded_look(true);
    let hp = [1000u16, 1000];
    look.observe_tick(0, &[view(0, 0.0, 0.0), view(1, 0.0, 4.0)], &hp, &[]);
    let rig = &look.rigs[0];
    let joint = |name: &str| {
        rig.skeleton
            .joints
            .iter()
            .position(|j| j.name == name)
            .unwrap_or_else(|| panic!("the rig has a {name} joint"))
    };
    let (pelvis, left, right) = (joint("pelvis"), joint("foot_l"), joint("foot_r"));
    let at = |world: &[[[f32; 4]; 4]], j: usize| [world[j][3][0], world[j][3][1], world[j][3][2]];
    let base = look.slots[0];
    let mut clip_only = base;
    clip_only.feet.ik = false;
    let clip = look
        .slot_sample(rig, 0, &clip_only, 0.5)
        .expect("clip pose");
    let gap = |a: [f32; 3], b: [f32; 3]| {
        ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
    };
    let mut drops = Vec::new();
    for pull in [0.1f32, 0.2, 0.3, 0.45] {
        let mut slot = base;
        slot.feet.plants[0] = Plant {
            planted: true,
            offset: [[pull, 0.0]; 2],
            ..Plant::NEW
        };
        let on = look.slot_sample(rig, 0, &slot, 0.5).expect("planted pose");
        let drop = at(&clip.world, pelvis)[1] - at(&on.world, pelvis)[1];
        drops.push(drop);
        let moved = [
            at(&on.world, left)[0] - at(&clip.world, left)[0],
            at(&on.world, left)[1] - at(&clip.world, left)[1],
            at(&on.world, left)[2] - at(&clip.world, left)[2],
        ];
        assert!(
            moved.iter().all(|v| v.is_finite()),
            "pull {pull}: {moved:?}"
        );
        assert!(
            gap(at(&on.world, right), at(&clip.world, right)) < 1.0e-3,
            "pull {pull}: the free foot left the sand"
        );
        if pull <= 0.3 {
            assert!(
                (moved[0] - pull).abs() < 2.0e-3
                    && moved[1].abs() < 2.0e-3
                    && moved[2].abs() < 2.0e-3,
                "pull {pull}: the ankle moved {moved:?}"
            );
        } else {
            assert!(
                moved[0] < pull,
                "pull {pull}: a stretched leg falls short, moved {moved:?}"
            );
        }
    }
    assert!(
        drops[0].abs() < 1.0e-4,
        "a short pull keeps the pelvis: {drops:?}"
    );
    assert!(
        drops[2] > 0.02 && drops[2] < 0.079,
        "a long reach drops the pelvis by the gap: {drops:?}"
    );
    assert!(
        (drops[3] - 0.08).abs() < 1.0e-4,
        "past the reach the drop stops at 8 cm: {drops:?}"
    );
    assert!(
        drops.windows(2).all(|w| w[1] >= w[0] - 1.0e-4),
        "drops grow with the pull: {drops:?}"
    );
}

#[test]
fn a_refused_solve_is_loud_and_a_legless_or_lying_body_keeps_the_clip() {
    let mut look = loaded_look(true);
    let hp = [1000u16, 1000];
    look.observe_tick(0, &[view(0, 0.0, 0.0), view(1, 0.0, 4.0)], &hp, &[]);
    let mut slot = look.slots[0];
    slot.feet.plants[0] = Plant {
        planted: true,
        offset: [[0.05, 0.0]; 2],
        ..Plant::NEW
    };
    let legs = look.rigs[0].legs.expect("the hero rig has legs");
    let clip = look
        .inertial_pose(&look.rigs[0], 0, &slot, 0.5)
        .expect("the clip poses");
    // A shin of no length cannot be solved: the frame fails loudly.
    let mut folded = clip.clone();
    folded.positions[legs.limbs[0].end] = [0.0; 3];
    let refused = look.plant_feet(&look.rigs[0], &slot, &mut folded, 0.0, 0.5);
    assert!(
        matches!(&refused, Err(super::LookError::Skin(msg)) if msg.contains("bone")),
        "{refused:?}"
    );
    // A skeleton with a leg missing finds no legs.
    let mut skeleton = look.rigs[0].skeleton.clone();
    let bind = vec![[0.0f32; 3]; skeleton.len()];
    assert!(Legs::find(&skeleton, &bind).is_some());
    let paw = skeleton.joints.iter_mut().find(|j| j.name == "foot_r");
    paw.expect("foot_r").name = "paw_r".to_string();
    assert!(Legs::find(&skeleton, &bind).is_none());
    // A rig with no legs plants nothing, plants or not.
    let mut legless = look.rigs.remove(0);
    legless.legs = None;
    let mut untouched = clip.clone();
    let contacts = look
        .plant_feet(&legless, &slot, &mut untouched, 0.0, 0.5)
        .expect("a legless rig plants nothing");
    assert_eq!(contacts, [None; 2]);
    assert_eq!(untouched, clip, "the pose is the clip's");
    // A body that is down keeps the clip's feet, plant state or not.
    legless.legs = Some(legs);
    let mut down = slot;
    down.act = Act::Down;
    let mut lying = clip.clone();
    let contacts = look
        .plant_feet(&legless, &down, &mut lying, 0.0, 0.5)
        .expect("a down body plants nothing");
    assert_eq!(contacts, [None; 2]);
    assert_eq!(lying, clip, "a body that lies keeps the clip's feet");
    // And the same rig and slot do plant when it stands.
    let mut standing = clip.clone();
    let contacts = look
        .plant_feet(&legless, &slot, &mut standing, 0.0, 0.5)
        .expect("a standing body plants");
    assert!(contacts[0].is_some() && contacts[1].is_none());
    assert_ne!(standing, clip, "a planted foot moves the pose");
}

#[test]
fn planting_a_frame_allocates_nothing() {
    use engine_core::alloc_probe::measure;
    let (_, seen) = measure(|| std::hint::black_box(vec![0u8; 64]));
    assert!(seen.calls > 0, "the probe must see a real allocation");
    let mut look = loaded_look(true);
    let hp = [1000u16, 1000];
    look.observe_tick(0, &[view(0, 0.0, 0.0), view(1, 0.0, 4.0)], &hp, &[]);
    let mut slot = look.slots[0];
    slot.feet.plants = [Plant {
        planted: true,
        offset: [[0.2, 0.0]; 2],
        ..Plant::NEW
    }; 2];
    let rig = &look.rigs[0];
    let mut pose = look
        .inertial_pose(rig, 0, &slot, 0.5)
        .expect("the clip poses");
    let (result, heap) = measure(|| {
        let mut out = Ok([None; 2]);
        for _ in 0..200 {
            out = look.plant_feet(rig, &slot, &mut pose, 0.3, 0.5);
            std::hint::black_box(&pose);
        }
        out
    });
    assert!(result.is_ok(), "{result:?}");
    assert_eq!(heap.calls, 0, "200 planted frames touch no heap");
}

#[test]
fn planted_feet_stand_still_in_hitstop_while_the_sim_goes_on_shoving() {
    // LF10 and LF12 together: slot 0 plants in a shoved guard, a Hit freezes
    // slot 0 and its attacker for HITSTOP_TICKS ticks, and the sim keeps
    // shoving slot 0 sideways all the while. Every frame of the freeze, at
    // every sub-tick alpha, shows slot 0's impact-tick pose bit for bit: the
    // plants stand still with the clip clock. The tick the freeze thaws the
    // pose moves again.
    let mut look = loaded_look(true);
    let hp = [1000u16, 1000];
    let impact = 80u32;
    let mut frozen: Option<Vec<Vec<Option<Shape>>>> = None;
    let (mut held_ticks, mut planted_in_freeze) = (0u32, 0u32);
    for tick in 0..impact + HITSTOP_TICKS + 2 {
        let shove = tick.saturating_sub(40);
        let x = 0.01 * f32::from(u16::try_from(shove).unwrap_or(0));
        let cues = if tick == impact {
            vec![FightCue::Hit {
                attacker: 1,
                victim: 0,
                damage: 100,
            }]
        } else {
            Vec::new()
        };
        look.observe_tick(tick, &[view(0, x, 0.0), view(1, 0.0, 4.0)], &hp, &cues);
        if tick == impact {
            frozen = Some(ALPHAS.iter().map(|a| shapes(&look, 2, *a)).collect());
        }
        let Some(frozen) = &frozen else {
            continue;
        };
        let held = look.slots[0].held;
        if tick < impact + HITSTOP_TICKS {
            assert!(held, "tick {tick}: slot 0 is held");
            held_ticks += 1;
            planted_in_freeze += u32::from(look.slots[0].feet.plants.iter().any(|p| p.planted));
            for (index, alpha) in ALPHAS.iter().enumerate() {
                let now = shapes(&look, 2, *alpha);
                assert!(
                    now[0] == frozen[index][0],
                    "tick {tick} alpha {alpha}: slot 0 moved in the freeze: {}",
                    gap(&now[0], &frozen[index][0])
                );
            }
        } else if tick == impact + HITSTOP_TICKS {
            assert!(!held, "tick {tick}: the freeze is over");
            assert!(
                shapes(&look, 2, 0.5)[0] != frozen[1][0],
                "tick {tick}: slot 0 must move once the freeze is over"
            );
        }
    }
    assert_eq!(held_ticks, HITSTOP_TICKS);
    assert!(
        planted_in_freeze > 0,
        "slot 0 had a foot planted through the freeze"
    );
}

#[test]
fn bodies_that_lie_are_never_planted_and_a_still_guard_barely_changes() {
    // Four fighters in a ring, one dying every ten ticks (even slots with the
    // clip playing, odd slots with the sim's `Fallen` verdict), run twice, plant
    // on and off. A dead or fallen body is the clip's, bit for bit. A living
    // body that stands in its guard with nothing moving it is within 1.5 cm of
    // the clip: the plant only holds what the clip already holds.
    let ring = [
        view(0, -1.0, 0.0),
        view(1, 1.0, 0.0),
        view(2, 0.0, 1.8),
        view(3, 0.0, -1.8),
    ];
    let (mut on, mut off) = (loaded_look(true), loaded_look(false));
    let mut hp = [1000u16; 4];
    let (mut dead_frames, mut living_frames, mut worst_cm) = (0u32, 0u32, 0.0f32);
    for tick in 0..160u32 {
        let mut cues = Vec::new();
        for victim in 0..4u8 {
            if tick == 60 + 10 * u32::from(victim) {
                hp[usize::from(victim)] = 0;
                cues.push(FightCue::Knockdown {
                    attacker: (victim + 1) % 4,
                    victim,
                });
            }
        }
        for look in [&mut on, &mut off] {
            look.observe_tick(tick, &ring, &hp, &cues);
            for victim in [1u8, 3] {
                if tick == 60 + 10 * u32::from(victim) {
                    look.observe_balance(victim, GaitPhase::Fallen);
                }
            }
        }
        for alpha in [0.0f32, 0.5, 0.99] {
            let (a, b) = (
                on.pose_frame(4, alpha).expect("planted pose"),
                off.pose_frame(4, alpha).expect("clip pose"),
            );
            for (planted, clip) in a.iter().zip(&b) {
                assert_eq!(planted.truth.slot, clip.truth.slot);
                if is_dead(&on, planted.truth.slot) {
                    dead_frames += 1;
                    assert!(
                        planted.truth.joints == clip.truth.joints
                            && planted.matrices == clip.matrices,
                        "tick {tick} slot {}: a dead body was planted",
                        planted.truth.slot
                    );
                } else if tick >= 40 {
                    living_frames += 1;
                    let off_cm = planted
                        .truth
                        .joints
                        .iter()
                        .zip(&clip.truth.joints)
                        .map(|(p, q)| {
                            100.0
                                * ((p[0] - q[0]).powi(2)
                                    + (p[1] - q[1]).powi(2)
                                    + (p[2] - q[2]).powi(2))
                                .sqrt()
                        })
                        .fold(0.0f32, f32::max);
                    worst_cm = worst_cm.max(off_cm);
                    assert!(
                        off_cm < 1.5,
                        "tick {tick} slot {}: a still guard moved {off_cm:.2} cm",
                        planted.truth.slot
                    );
                }
            }
        }
    }
    eprintln!("lf12 still bodies: {dead_frames} dead frames, {living_frames} guard frames, worst {worst_cm:.2} cm");
    assert!(dead_frames > 100, "{dead_frames} dead frames");
    assert!(living_frames > 100, "{living_frames} guard frames");
}

/// Euclidean distance in meters between two `[x, y, z]` points.
fn span(a: [f32; 3], b: [f32; 3]) -> f32 {
    dist2(a, b).sqrt()
}

/// Unit vector in the direction of `v`; the zero vector reads as zero.
fn unit3(v: [f32; 3]) -> [f32; 3] {
    let n = span(v, [0.0; 3]);
    if n > 0.0 {
        [v[0] / n, v[1] / n, v[2] / n]
    } else {
        [0.0; 3]
    }
}

/// `a . b`.
fn dot3(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

#[test]
fn the_weapon_hand_holds_the_grip_socket_through_a_swing() {
    // LF12's hand half on the real CT7 hero. The sword is skinned to the
    // weapon socket `wep_pos_r`, a child of the right palm, so the grip sits
    // in the hand and keeps one offset from the palm while a swing clip runs
    // (the sword never slides in the grip). The same right arm then reaches a
    // grip target at 0.9 of its chain length within 1e-3 m, with the elbow
    // kept on the pole's side: the hand solve the weapon socket needs.
    let mut look = FighterLook::load(&content_dir()).expect("CT7 bodies load");
    let (palm, socket, arm) = {
        let joints = &look.rigs[0].skeleton.joints;
        let at = |name: &str| {
            joints
                .iter()
                .position(|joint| joint.name == name)
                .unwrap_or_else(|| panic!("the hero rig names {name}"))
        };
        let shoulder = at("arm_upper_r");
        let elbow = at("arm_lower_r");
        let palm = at("palm_r");
        let socket = at("wep_pos_r");
        let arm =
            Limb::new(&look.rigs[0].skeleton, shoulder, elbow, palm).expect("right arm chain");
        (palm, socket, arm)
    };
    assert_eq!(
        look.rigs[0].skeleton.joints[socket].parent,
        Some(palm),
        "the sword's grip socket rides on the right palm"
    );
    let views = [view(0, 0.0, 0.0), view(1, 0.0, 1.2)];
    let hit = FightCue::Hit {
        attacker: 0,
        victim: 1,
        damage: 100,
    };
    let mut spans = Vec::new();
    for tick in 0..30u32 {
        let cues = if tick == 0 { vec![hit] } else { Vec::new() };
        look.observe_tick(tick, &views, &[1000, 900], &cues);
        let rig = &look.rigs[look.rig_for_slot(0)];
        let slot = look.slots[0];
        for frame in 0..3u16 {
            let alpha = (f32::from(frame) + 0.5) / 3.0;
            let pose = look.slot_pose(rig, &slot, alpha).expect("pose samples");
            let world = skinning_matrices(&rig.skeleton, &pose).expect("FK");
            let place = |joint: usize| [world[joint][3][0], world[joint][3][1], world[joint][3][2]];
            spans.push(span(place(palm), place(socket)));
        }
    }
    let grip = spans[0];
    assert!(
        (0.03..0.15).contains(&grip),
        "the grip socket sits in the hand: {grip} m from the palm"
    );
    let drift = spans
        .iter()
        .map(|d| (d - grip).abs())
        .fold(0.0f32, f32::max);
    assert!(
        drift < 1.0e-3,
        "the sword slid {drift} m in the hand across the swing"
    );
    // The right arm reaches a grip target 0.9 of its chain away.
    let rig = &look.rigs[0];
    let slot = look.slots[0];
    let mut pose = look.slot_pose(rig, &slot, 1.0).expect("pose samples");
    let before = limb_positions(&rig.skeleton, &pose, arm).expect("FK");
    let chain = span(before[0], before[1]) + span(before[1], before[2]);
    let reach = 0.9 * chain;
    let aim = unit3([0.25, -0.9, 0.35]);
    let target = [
        before[0][0] + aim[0] * reach,
        before[0][1] + aim[1] * reach,
        before[0][2] + aim[2] * reach,
    ];
    let pole = [0.0, 0.0, 1.0];
    solve_limb(&rig.skeleton, &mut pose, arm, target, pole).expect("the grip target solves");
    let after = limb_positions(&rig.skeleton, &pose, arm).expect("FK");
    let miss = span(after[2], target);
    assert!(miss < 1.0e-3, "the weapon hand misses the grip by {miss} m");
    let axis = unit3([
        target[0] - after[0][0],
        target[1] - after[0][1],
        target[2] - after[0][2],
    ]);
    let rel = [
        after[1][0] - after[0][0],
        after[1][1] - after[0][1],
        after[1][2] - after[0][2],
    ];
    let along = dot3(rel, axis);
    let off = [
        rel[0] - axis[0] * along,
        rel[1] - axis[1] * along,
        rel[2] - axis[2] * along,
    ];
    let want = [
        pole[0] - axis[0] * dot3(pole, axis),
        pole[1] - axis[1] * dot3(pole, axis),
        pole[2] - axis[2] * dot3(pole, axis),
    ];
    assert!(
        dot3(off, want) > 0.0,
        "the elbow left the axis on the wrong side"
    );
    eprintln!(
        "lf12 weapon hand: grip {grip} m from the palm, drift {drift} m, reach miss {miss} m"
    );
}

/// WEB6: the rig loads out of a fetched bundle exactly as it loads out of the
/// directory. The browser has no file system, so the page opens the cooked
/// bundle and hands it to [`FighterLook::load_from`]; the bytes it reads must
/// be the same bytes the native window reads, or the browser would stage a
/// different fighter than the desktop does.
///
/// The bundle is the one the cook makes (`engine_assets::fighter_bundle`),
/// tier for tier, so this proof covers the cook, the pack, the reader and the
/// loader in one line.
#[test]
fn the_rig_loads_out_of_a_cooked_bundle_with_the_native_bytes() {
    use engine_assets::asset_source::{AssetSource, FsSource};
    use engine_assets::fighter_bundle::cook;

    let dir = content_dir();
    let from_files = FsSource::new(&dir);
    // The cook reads the same tree the native window reads.
    let cooked = cook(&from_files).expect("the rig tree cooks");
    let bundle =
        engine_assets::BundleSource::open(cooked.bytes.to_vec()).expect("the bundle opens");

    // The bundle holds the manifest and the rig tree, and every file in it is
    // byte for byte the file the native window reads. (The digest of the whole
    // directory would not match: the directory also holds the atlas PNGs and
    // the previews, which the rig does not need.)
    let packed = bundle.list("").expect("the bundle lists its entries");
    assert!(
        packed.contains(&"manifest.json".to_string()),
        "the manifest is in the bundle"
    );
    for path in &packed {
        let from_bundle = bundle.read(path).expect("the bundle holds the file");
        let from_dir = from_files.read(path).expect("the directory holds the file");
        assert_eq!(
            from_bundle.as_ref(),
            from_dir.as_ref(),
            "{path} is the same bytes through either source"
        );
    }
    let rig_files = from_files.list("glb").expect("the rig tree lists").len() + 1;
    assert_eq!(
        packed.len(),
        rig_files,
        "the bundle holds the manifest and every rig file, and nothing else"
    );

    // The rig itself loads from the bundle: the same four variants, the same
    // pinned bundle SHAs and the same skeleton per variant.
    let from_dir = FighterLook::load(&dir).expect("CT7 bodies load from the directory");
    let from_bundle = FighterLook::load_from(&bundle).expect("CT7 bodies load from the bundle");
    assert_eq!(
        from_dir.bundle_shas(),
        from_bundle.bundle_shas(),
        "the same variants are bound, in the same order"
    );
    assert_eq!(
        from_dir.rig_count(),
        from_bundle.rig_count(),
        "the same number of rigs is staged"
    );
    let dir_summaries = from_dir.rig_summaries();
    let bundle_summaries = from_bundle.rig_summaries();
    for (index, rig) in dir_summaries.iter().enumerate() {
        let other = bundle_summaries
            .get(index)
            .expect("the bundle stages the same rigs");
        assert_eq!(
            rig, other,
            "rig {index} is the same rig through either source"
        );
    }
}
