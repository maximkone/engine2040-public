//! fighter_look_proofs: end-to-end proofs on the real CT7 bodies, no window.
//!
//! Consumer: `cargo test` only (child of [`super`], so it reads the private
//! pose path the window draws through). Data owner: none; it owns no state.
//! Units: meters, fixed ticks at 60 Hz. Determinism: every run is a seeded
//! headless fight or a scripted tick list, so the same code gives the same
//! numbers twice. Errors: a failed assertion names the tick and the number.
#![cfg(test)]

use std::path::PathBuf;

use engine_arena::GaitPhase;
use engine_audio::FightCue;
use engine_combat::fight::HITSTOP_TICKS;
use engine_render::v1::Camera;

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
fn sand_rest_real_fight_dead_lie_on_the_sand_and_the_living_are_not_lifted() {
    // The window's own session: 8 fighters, the duel's tables, 900 ticks of
    // brains fighting. Every body the sim has floored or killed rests on the
    // sand; a standing body is never moved by the dead-body rest.
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
    let (mut dead_frames, mut worst_cm) = (0u32, 0.0f32);
    for tick in 0..900u32 {
        let out = app.tick_empty();
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
                assert_eq!(
                    body.lift, 0.0,
                    "tick {tick} slot {}: a living body is never lifted",
                    body.truth.slot
                );
            }
        }
    }
    assert!(dead_frames > 100, "the fight floored bodies: {dead_frames}");
    assert!(worst_cm < 1.0, "worst rest error {worst_cm:.2} cm");
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
    let (mut bystanders_moved, mut blends_held, mut resumed) = (0u32, 0u32, 0u32);
    for tick in 0..300u32 {
        let out = app.tick_empty();
        look.observe_tick(tick, app.fighter_views(), app.hp(), &out.cues);
        look.observe_live_balances(app.live_match(), 4);
        let remaining = app.hitstop_remaining();
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
                    if age == 3 {
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
            } else if remaining == 0 && impacts == 0 && age == 4 {
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
            }
        }
        let single_hit = impacts == 1 && remaining == 4;
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
        held_ticks + 3 >= 3 * windows,
        "held ticks {held_ticks} for {windows} windows"
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

/// The `arena view:` line of a 300-tick four-fighter duel posed at
/// `frames_per_tick` sub-tick frames per sim tick, counted by [`ViewLaws`]
/// exactly as the window counts its frames.
fn duel_view_line(frames_per_tick: u16) -> String {
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
    let mut laws = ViewLaws::default();
    let camera = arena_camera();
    for tick in 0..300u32 {
        let out = app.tick_empty();
        look.observe_tick(tick, app.fighter_views(), app.hp(), &out.cues);
        look.observe_live_balances(app.live_match(), 4);
        for frame in 0..frames_per_tick {
            let alpha = (f32::from(frame) + 0.5) / f32::from(frames_per_tick);
            let bodies = look.pose_frame_joints(4, alpha).expect("joints pose");
            laws.note(&camera, f64::from(tick) + f64::from(alpha), &bodies);
        }
    }
    laws.line().expect("the duel folded frames")
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

#[test]
fn zz_probe_foot_speeds() {
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
    let names: Vec<String> = look.rigs[0].skeleton.joints.iter().map(|j| j.name.clone()).collect();
    let find = |n: &str| names.iter().position(|x| x == n).expect("joint");
    let feet = [find("foot_l"), find("foot_r")];
    // per slot, per foot: last model pos, run (contact world xz, ticks, max dev, act)
    let mut last: Vec<[Option<[f32; 3]>; 2]> = vec![[None; 2]; 4];
    let mut runs: Vec<[Option<([f32; 2], u32, f32)>; 2]> = vec![[None; 2]; 4];
    let mut log: Vec<(String, u32, f32)> = Vec::new();
    let mut ticks_by_act = std::collections::BTreeMap::<String, (u32, u32)>::new();
    let mut devs_all: Vec<f32> = Vec::new();
    for tick in 0..300u32 {
        let out = app.tick_empty();
        look.observe_tick(tick, app.fighter_views(), app.hp(), &out.cues);
        look.observe_live_balances(app.live_match(), 4);
        for body in look.pose_frame_joints(4, 1.0).expect("pose") {
            let s = usize::from(body.slot);
            if look.slots[s].held {
                continue;
            }
            let (sn, cs) = body.yaw.sin_cos();
            for (f, &j) in feet.iter().enumerate() {
                let p = body.joints[j];
                let w = [p[0] * cs + p[2] * sn + body.root[0], -p[0] * sn + p[2] * cs + body.root[2]];
                let e = ticks_by_act.entry(body.act.to_string()).or_default();
                e.0 += 1;
                if let Some(q) = last[s][f] {
                    let v = ((p[0] - q[0]).hypot(p[2] - q[2])) * 60.0;
                    let planted = v < 0.15 && p[1] < 0.075 + 0.03 && body.act != "down";
                    if planted {
                        e.1 += 1;
                        let r = runs[s][f].get_or_insert((w, 0, 0.0));
                        r.1 += 1;
                        let dev = (w[0] - r.0[0]).hypot(w[1] - r.0[1]);
                        r.2 = r.2.max(dev);
                        devs_all.push(dev * 100.0);
                    } else if let Some(r) = runs[s][f].take() {
                        log.push((body.act.to_string(), r.1, r.2 * 100.0));
                    }
                }
                last[s][f] = Some(p);
            }
        }
    }
    eprintln!("planted by act (feet-ticks, planted): {ticks_by_act:?}");
    devs_all.sort_by(|a, b| a.total_cmp(b));
    let n = devs_all.len();
    eprintln!("IK-off slide per planted foot-tick: n {n} p50 {:?} p95 {:?} max {:?}", devs_all.get(n/2), devs_all.get(n*95/100), devs_all.last());
    eprintln!("{} runs", log.len());
    for r in log.iter().take(60) {
        eprintln!("  {} {} ticks maxdev {:.2} cm", r.0, r.1, r.2);
    }
}
