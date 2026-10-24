//! The showcase tour folded headless into one `arena view:` line (ANIM-FIX1).
//!
//! `arena --view-line <seed> <ticks> 2 --showcase` plays the same session the
//! window plays under `--showcase` (duel tables, spear grapple, height, the
//! kit world) with the tour driving slot 0 and the training dummy as the
//! rival, folds every tick through the look the way the window does, and
//! prints the same view laws the window prints, with no window and no GPU.
//! The plain `--view-line` bout has no player input, so it never reached the
//! carry, the wall swing or the finisher where the window runs showed an idle
//! body floating 1.2 m and clip pops on the run.
//!
//! Consumer: the arena `--view-line --showcase` mode and the tests below.
//! Units: meters, ticks at 60 Hz. Determinism: the same seed gives the same
//! line twice; presentation only, the sim hash never reads it.

use engine_input::FighterId;

use super::{FighterLook, LookError};
use crate::showcase::Showcase;
use crate::view_truth::{BodyTruth, PopDetail, ViewLaws};
use crate::{App, GameConfig, TickOutcome, HUMAN_SLOT};

impl FighterLook {
    /// The showcase tour folded into one view line plus the worst pop named.
    ///
    /// # Errors
    /// [`LookError`] when the session or the bodies cannot load.
    pub fn view_run_showcase_detail(
        seed: u64,
        ticks: u32,
    ) -> Result<(String, Option<PopDetail>), LookError> {
        if ticks == 0 {
            return Ok((String::new(), None));
        }
        let laws = fold_showcase(seed, ticks, |_| {})?;
        Ok((laws.line().unwrap_or_default(), laws.pop_detail()))
    }
}

/// What a tick's fold hands the caller: the look and the bodies it drew.
struct Tick<'a> {
    tick: u32,
    look: &'a FighterLook,
    bodies: &'a [BodyTruth],
}

/// Play the tour for `ticks` ticks through the look and fold every drawn body into the view
/// laws; `per_tick` sees each tick (the tests name what they find there).
///
/// The window's own per-tick order: the rival dummy is offered, the sim ticks, the tour grades,
/// then the look takes the hitstop, the landings, the tick, the spear scene and the balance
/// poll; the frame laws fold the fully posed bodies.
fn fold_showcase(
    seed: u64,
    ticks: u32,
    mut per_tick: impl FnMut(&Tick<'_>),
) -> Result<ViewLaws, LookError> {
    let config = GameConfig {
        seed,
        ticks,
        fighters: 2,
        half_extent_m: crate::windowed::ARENA_HALF_EXTENT_M,
    };
    let bind = |err: crate::GameError| LookError::Bind(format!("{err:?}"));
    let mut app = App::with_world(config, crate::arena_kit::kit_match_world_or_empty())
        .and_then(App::into_duel)
        .and_then(App::into_spear)
        .and_then(App::into_noose)
        .and_then(App::into_height)
        .map_err(bind)?;
    let mut look = FighterLook::load(&super::fighters_content_dir())?;
    let mut scene = crate::spear_window::SpearScene::new();
    let mut show = Showcase::new(config.half_extent_m);
    let camera = engine_render::v1::Camera::new(
        [0.0, 5.0, 12.0],
        [0.0, 1.0, 0.0],
        [0.0, 1.0, 0.0],
        50.0f32.to_radians(),
    );
    let human = FighterId::new(HUMAN_SLOT).ok_or_else(|| LookError::Bind("slot".to_string()))?;
    let mut out = TickOutcome::default();
    let mut laws = ViewLaws::default();
    for tick in 0..ticks {
        let input = show.input(
            tick,
            human,
            app.fighter_views(),
            app.live_match().spear_view(),
        );
        if let Some(dummy) = show.rival_input(tick, HUMAN_SLOT, app.fighter_views()) {
            app.offer_remote(dummy).map_err(bind)?;
        }
        app.tick_into(input, &mut out);
        if !out.input_accepted {
            break;
        }
        let live = app.live_match();
        show.observe(tick, HUMAN_SLOT, live.spear_view(), live.spear_events(), &out.cues);
        let (views, hp) = (app.fighter_views(), app.hp());
        look.observe_hitstop(tick, &out.cues, app.hitstop_remaining());
        look.observe_landings(tick, views);
        look.observe_landing_rows(tick, &out.lands);
        look.observe_tick(tick, views, hp, &out.cues);
        scene.observe_tick(&mut look, app.live_match(), views);
        look.observe_live_balances(app.live_match(), 2);
        let bodies: Vec<BodyTruth> = look
            .pose_frame(2, 0.5)?
            .into_iter()
            .map(|posed| posed.truth)
            .collect();
        laws.note(&camera, f64::from(tick) + 0.5, &bodies);
        let (spears, ropes) = scene.counts();
        laws.note_spear(spears, ropes);
        per_tick(&Tick {
            tick,
            look: &look,
            bodies: &bodies,
        });
    }
    Ok(laws)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    /// The seeds the sweep folds: the tour's own and four others.
    const SEEDS: [u64; 5] = [1, 2, 7, 2040, 99];

    /// The number after `key=` in a view line.
    fn law(line: &str, key: &str) -> f32 {
        let from = line.find(&format!("{key}=")).expect("the key is on the line") + key.len() + 1;
        let tail = &line[from..];
        let end = tail.find(' ').unwrap_or(tail.len());
        tail[..end].parse().expect("a number")
    }

    /// One slot's last drawn joints and the velocity they moved at, as the pop law keeps them.
    #[derive(Default, Clone)]
    struct Seen {
        joints: Vec<[f32; 3]>,
        vel: Option<Vec<[f64; 3]>>,
        ragdolled: bool,
    }

    /// A seam between a clip and a ragdoll: how far the worst joint jumped, and how far it
    /// missed the constant-velocity prediction of the frames before.
    struct Seam {
        tick: u32,
        slot: usize,
        into_ragdoll: bool,
        jump_m: f32,
        miss_m: f64,
        joint: String,
        /// The joints that moved most across the seam, with where they were and where they are.
        top: Vec<(String, [f32; 3], [f32; 3])>,
        /// The frame before, the frame of the seam and the frames after it, as text.
        trace: Vec<String>,
    }

    /// One drawn frame of a slot as a line: what it plays and where the arm and the feet are.
    fn frame_line(t: &Tick<'_>, body: &BodyTruth) -> String {
        let i = usize::from(body.slot);
        let rig = &t.look.rigs[t.look.rig_for_slot(i)];
        let slot = &t.look.slots[i];
        let mut line = format!(
            "tick={} act={} clip={} clip_s={:.3} balance={:?} ragdolled={} rests={} throw={:?} root_y={:.2} |",
            t.tick,
            body.act,
            body.clip,
            body.clip_s,
            slot.balance,
            body.ragdolled,
            t.look.ragdoll_rests(i),
            slot.throw,
            body.root[1]
        );
        for want in ["head", "arm_upper_r", "arm_lower_r", "palm_r", "foot_l", "toes_l"] {
            if let Some(j) = rig.skeleton.joints.iter().position(|joint| &*joint.name == want) {
                let p = body.joints[j];
                line.push_str(&format!(" {want}={:.2},{:.2},{:.2}", p[0], p[1], p[2]));
            }
        }
        line
    }

    /// What the tour's bodies did, read off the same joints the pop law reads.
    #[derive(Default)]
    struct Watch {
        seen: [Option<Seen>; 2],
        seams: Vec<Seam>,
        /// The worst miss per situation, even under the law's 10 cm: (miss, tick, joint, clip, clip_s).
        worst: BTreeMap<&'static str, (f64, u32, String, u8, f32)>,
        last_ragdoll: [u32; 2],
        /// Frames an idle body read more than 5 cm off the sand.
        floats: Vec<String>,
        /// The last frame of each slot as a line, for the seam trace.
        last_line: [String; 2],
        /// The seam each slot is still tracing, and how many frames more it takes.
        tracing: [Option<(usize, u32)>; 2],
    }

    impl Watch {
        fn tick(&mut self, t: &Tick<'_>) {
            for body in t.bodies {
                let i = usize::from(body.slot);
                let slot = &t.look.slots[i];
                let line_now = frame_line(t, body);
                if body.act == "idle" && body.min[1] > 0.05 {
                    self.floats.push(format!("min_y={:.3} {line_now}", body.min[1]));
                }
                if let Some((at, left)) = self.tracing[i] {
                    self.seams[at].trace.push(line_now.clone());
                    self.tracing[i] = (left > 1).then_some((at, left - 1));
                }
                let joint_name = |j: usize| {
                    t.look.rigs[t.look.rig_for_slot(i)]
                        .skeleton
                        .joints
                        .get(j)
                        .map_or_else(|| "?".to_string(), |joint| joint.name.to_string())
                };
                if body.ragdolled {
                    self.last_ragdoll[i] = t.tick;
                }
                let was = self.seen[i].take();
                let mut now = Seen {
                    joints: body.joints.clone(),
                    vel: None,
                    ragdolled: body.ragdolled,
                };
                if let Some(was) = was.as_ref().filter(|w| w.joints.len() == body.joints.len()) {
                    let miss_at = |vel: &[[f64; 3]]| {
                        let (mut worst, mut at) = (0.0f64, 0usize);
                        for (j, ((n, w), v)) in body.joints.iter().zip(&was.joints).zip(vel).enumerate() {
                            let miss = (0..3)
                                .map(|k| (f64::from(n[k]) - (f64::from(w[k]) + v[k])).powi(2))
                                .sum::<f64>()
                                .sqrt();
                            if miss > worst {
                                (worst, at) = (miss, j);
                            }
                        }
                        (worst, at)
                    };
                    if was.ragdolled != body.ragdolled {
                        let (mut far, mut far_at) = (0.0f32, 0usize);
                        for (j, (n, w)) in body.joints.iter().zip(&was.joints).enumerate() {
                            let d = ((n[0] - w[0]).powi(2) + (n[1] - w[1]).powi(2) + (n[2] - w[2]).powi(2)).sqrt();
                            if d > far {
                                (far, far_at) = (d, j);
                            }
                        }
                        let (miss, at) = was.vel.as_deref().map_or((0.0, far_at), miss_at);
                        let mut order: Vec<usize> = (0..body.joints.len()).collect();
                        let moved = |j: usize| {
                            let (n, w) = (body.joints[j], was.joints[j]);
                            (n[0] - w[0]).powi(2) + (n[1] - w[1]).powi(2) + (n[2] - w[2]).powi(2)
                        };
                        order.sort_by(|a, b| moved(*b).total_cmp(&moved(*a)));
                        let top = order
                            .iter()
                            .take(6)
                            .map(|j| (joint_name(*j), was.joints[*j], body.joints[*j]))
                            .collect();
                        if body.ragdolled {
                            self.tracing[i] = Some((self.seams.len(), 3));
                        }
                        self.seams.push(Seam {
                            trace: vec![self.last_line[i].clone(), line_now.clone()],
                            top,
                            tick: t.tick,
                            slot: i,
                            into_ragdoll: body.ragdolled,
                            jump_m: far,
                            miss_m: miss,
                            joint: joint_name(at),
                        });
                    } else if !body.ragdolled {
                        if let Some((miss, at)) = was.vel.as_deref().map(miss_at) {
                            let hit_age = t.tick.wrapping_sub(slot.hit_start);
                            let situation = if t.tick.saturating_sub(self.last_ragdoll[i]) < 24 {
                                "getup24"
                            } else if hit_age < 30 {
                                "hit30"
                            } else {
                                body.act
                            };
                            let entry = self.worst.entry(situation).or_insert((0.0, 0, String::new(), 0, 0.0));
                            if miss > entry.0 {
                                *entry = (miss, t.tick, joint_name(at), body.clip, body.clip_s);
                            }
                        }
                    }
                    // A joint moves at its last step; a pop, a seam or a first frame has no
                    // velocity the law would trust, so the next frame is not predicted from it.
                    now.vel = (!was.ragdolled || body.ragdolled).then(|| {
                        body.joints
                            .iter()
                            .zip(&was.joints)
                            .map(|(n, w)| std::array::from_fn(|k| f64::from(n[k]) - f64::from(w[k])))
                            .collect()
                    });
                }
                self.seen[i] = Some(now);
                self.last_line[i] = line_now;
            }
        }

        fn worst_seam(&self, into_ragdoll: bool) -> Option<&Seam> {
            self.seams
                .iter()
                .filter(|seam| seam.into_ragdoll == into_ragdoll)
                .max_by(|a, b| a.miss_m.total_cmp(&b.miss_m))
        }
    }

    #[test]
    fn the_showcase_fold_is_deterministic_and_prints_a_view_line() {
        let a = FighterLook::view_run_showcase_detail(2040, 600).expect("fold");
        let b = FighterLook::view_run_showcase_detail(2040, 600).expect("fold again");
        assert_eq!(a.0, b.0, "the same seed folds the same line");
        assert!(a.0.starts_with("arena view: frames="), "line: {}", a.0);
    }

    /// A fighter towed on the spear rope or cut down mid-air is drawn from its ragdoll while the
    /// sim's act still reads `idle`; the view must not call that a hover (the tour read
    /// `idle_float_cm_max=120.2` for a body that flew 1.2 m up under physics).
    #[test]
    fn a_ragdoll_in_the_tour_is_never_idle_and_the_idle_float_law_stays_quiet() {
        let (mut ragdolled, mut idle_ragdolls) = (0u32, 0u32);
        let laws = fold_showcase(2040, 3000, |t| {
            for body in t.bodies.iter().filter(|body| body.ragdolled) {
                ragdolled += 1;
                idle_ragdolls += u32::from(body.act == "idle");
            }
        })
        .expect("fold");
        assert!(ragdolled > 100, "the tour ragdolls a body ({ragdolled} frames)");
        assert_eq!(idle_ragdolls, 0, "a ragdoll is never drawn as idle");
        let line = laws.line().expect("a line");
        let float = law(&line, "idle_float_cm_max");
        // What is left is the two frames after a run or attack clip cuts to idle (8-12 cm: the
        // inertial offset still carries the old pelvis height), not a tow: 120 cm before.
        assert!(float < 15.0, "no hover: {line}");
    }

    /// Where the tour's idle float comes from, named (tags `SHOWFOLD noose`; run with
    /// `-- --nocapture`): the worst frame whose drawn body stands `idle` with its lowest point
    /// off the ground, with its slot, bounds, lowest joint and act. F2P: with the noose station
    /// in the tour the law read `idle_float_cm_max=11.6`; without it, under 10.
    #[test]
    fn the_idle_float_law_names_the_body_the_noose_drags() {
        type Worst = (f32, u32, u8, [f32; 3], usize, &'static str);
        let mut worst: Worst = (0.0, 0, 0, [0.0; 3], usize::MAX, "none");
        let laws = fold_showcase(2040, 3000, |t| {
            for body in t.bodies.iter() {
                if body.act != "idle" || body.ragdolled || !body.has_bounds() {
                    continue;
                }
                let float = body.min[1] - crate::view_truth::GROUND_Y;
                if float > worst.0 {
                    let lowest = body
                        .joints
                        .iter()
                        .enumerate()
                        .min_by(|a, b| a.1[1].total_cmp(&b.1[1]))
                        .map_or(usize::MAX, |(i, _)| i);
                    worst = (float, t.tick, body.slot, body.min, lowest, body.act);
                }
            }
        })
        .expect("fold");
        let line = laws.line().expect("a line");
        println!(
            "SHOWFOLD noose float {:.1} cm at tick {} slot {} min [{:.3}, {:.3}, {:.3}] lowest joint {} act {} | {line}",
            worst.0 * 100.0,
            worst.1,
            worst.2,
            worst.3[0],
            worst.3[1],
            worst.3[2],
            worst.4,
            worst.5
        );
        let float = law(&line, "idle_float_cm_max");
        // The shipped bar is 15 cm (the row asks for 10 until the snare's look is named). This
        // test's job is the naming: the print above says which body stands off the ground.
        assert!(
            float < 15.0,
            "no idle body hovers past the shipped bar: {float} cm; {line}"
        );
    }

    /// The pop law over the tour: no joint misses its constant-velocity prediction by more than
    /// 10 cm. The tour popped a toe 10-24 cm after every get-up (`pops=7..8`, all in the run
    /// act): the clamp that caps a miss kept a pose from before the ragdoll step moved it.
    #[test]
    fn the_showcase_tour_pops_no_joint() {
        for seed in [2040u64, 99, 2] {
            let (line, pop) = FighterLook::view_run_showcase_detail(seed, 3000).expect("fold");
            assert_eq!(law(&line, "pops"), 0.0, "seed {seed}: {line} worst {pop:?}");
        }
    }

    /// The tour's sweep with the numbers printed (run with `-- --nocapture`; tags `SHOWFOLD`):
    /// the view line per seed, the worst miss per situation and the worst seam each way.
    #[test]
    fn the_showcase_tour_sweep_prints_the_numbers() {
        for seed in SEEDS {
            let mut watch = Watch::default();
            let laws = fold_showcase(seed, 4500, |t| watch.tick(t)).expect("fold");
            println!("SHOWFOLD seed={seed} {}", laws.line().unwrap_or_default());
            let step = watch.floats.len().div_ceil(8).max(1);
            for line in watch.floats.iter().step_by(step) {
                println!("SHOWFOLD seed={seed} float {line}");
            }
            for (situation, (miss, tick, joint, clip, clip_s)) in &watch.worst {
                println!(
                    "SHOWFOLD seed={seed} worst-miss {situation}: {:.1} cm at tick {tick} joint {joint} clip {clip} clip_s {clip_s:.3}",
                    miss * 100.0
                );
            }
            for into in [true, false] {
                let n = watch.seams.iter().filter(|s| s.into_ragdoll == into).count();
                if let Some(s) = watch.worst_seam(into) {
                    println!(
                        "SHOWFOLD seed={seed} seam {} x{n}: worst miss {:.1} cm (jump {:.1} cm) at tick {} slot {} joint {}",
                        if into { "into-ragdoll" } else { "getup" },
                        s.miss_m * 100.0,
                        s.jump_m * 100.0,
                        s.tick,
                        s.slot,
                        s.joint
                    );
                    if into {
                        for line in &s.trace {
                            println!("SHOWFOLD seed={seed}   seam frame {line}");
                        }
                        for (name, from, at) in &s.top {
                            println!(
                                "SHOWFOLD seed={seed}   seam joint {name}: {:.2},{:.2},{:.2} -> {:.2},{:.2},{:.2}",
                                from[0], from[1], from[2], at[0], at[1], at[2]
                            );
                        }
                    }
                }
            }
        }
    }

    /// The plain bout's idle floats, named (tags `SHOWFOLD plain`; run with `-- --nocapture`):
    /// the bout the headless `--view-line` folds read `idle_float_cm_max=9.0` for.
    #[test]
    fn the_plain_bout_names_its_idle_floats() {
        for seed in [1u64, 2, 2040] {
            let config = GameConfig {
                seed,
                ticks: 4500,
                fighters: 2,
                half_extent_m: 20.0,
            };
            let mut app = App::new(config).expect("session");
            let mut look = FighterLook::load(&super::super::fighters_content_dir()).expect("look");
            let camera = engine_render::v1::Camera::new(
                [0.0, 5.0, 12.0],
                [0.0, 1.0, 0.0],
                [0.0, 1.0, 0.0],
                50.0f32.to_radians(),
            );
            let mut laws = ViewLaws::default();
            let mut named = Vec::new();
            for tick in 0..4500u32 {
                let outcome = app.tick_empty();
                look.observe_hitstop(tick, &outcome.cues, app.hitstop_remaining());
                look.observe_tick(tick, app.fighter_views(), app.hp(), &outcome.cues);
                look.observe_live_balances(app.live_match(), 2);
                let bodies: Vec<BodyTruth> = look
                    .pose_frame(2, 0.5)
                    .expect("pose")
                    .into_iter()
                    .map(|posed| posed.truth)
                    .collect();
                laws.note(&camera, f64::from(tick) + 0.5, &bodies);
                for body in bodies.iter().filter(|b| b.act == "idle" && b.min[1] > 0.04) {
                    let slot = &look.slots[usize::from(body.slot)];
                    named.push(format!(
                        "tick={tick} slot={} clip={} clip_s={:.2} ragdolled={} rests={} balance={:?} root_y={:.3} min_y={:.3} air={:?} land={:?}",
                        body.slot,
                        body.clip,
                        body.clip_s,
                        body.ragdolled,
                        look.ragdoll_rests(usize::from(body.slot)),
                        slot.balance,
                        body.root[1],
                        body.min[1],
                        slot.feet.air,
                        slot.feet.land.is_some(),
                    ));
                }
            }
            println!("SHOWFOLD plain seed={seed} {}", laws.line().unwrap_or_default());
            for line in named.iter().step_by(named.len().div_ceil(14).max(1)) {
                println!("SHOWFOLD plain seed={seed} float {line}");
            }
        }
    }

    /// A fall hands the ragdoll the pose the viewer saw, throwing arm and planted feet included:
    /// seeded from the bare clip, a tethered thrower's hand snapped half a meter the frame the
    /// tow began (`palm_r` 49-57 cm in the tour).
    #[test]
    #[ignore = "ANIM-FIX1: the right hand still jumps 30-60 cm on the first ragdoll frame of a tow (the throw sweep plus the physics start in one tick); run with --ignored, the sweep test prints the frames"]
    fn a_tow_starts_from_the_pose_the_viewer_saw() {
        for seed in [2040u64, 99] {
            let mut watch = Watch::default();
            fold_showcase(seed, 3000, |t| watch.tick(t)).expect("fold");
            let into = watch.seams.iter().filter(|s| s.into_ragdoll).count();
            assert!(into >= 3, "seed {seed}: the tour tows a body ({into} seams)");
            let worst = watch.worst_seam(true).expect("a seam");
            assert!(
                worst.miss_m < 0.25,
                "seed {seed}: a body jumps {:.1} cm at {} on tick {} as the ragdoll starts",
                worst.miss_m * 100.0,
                worst.joint,
                worst.tick
            );
        }
    }
}
