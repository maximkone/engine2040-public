//! warp_ground_slope accept sweep (LM6): the LM2 foot-slide bar holds on
//! 20 degree slopes up and down through the full locomotion + slope-warp +
//! foot-lock chain, stair plants land on their treads and count steps, sand
//! sink reads as sink instead of skate, replay x2 is exact and steady ticks
//! allocate nothing.
#![cfg(test)]

use crate::clip::TICK_DT;
use crate::feet::{feet_hash, planted_from_contacts, slide_mm, FootLocker};
use crate::locomotion::{
    LocoOutput, LocoState, Locomotion, LOCO_LEG_JOINTS as LEGS, MAX_WISH_SPEED,
};
use crate::warp_ground::{warp_hash, GroundWarp, WarpError, SLIDE_BUDGET_MM};
use crate::warp_ground_slope::{
    dist_mm, ground_replay_hash, Grade, GroundFeet, GroundTick, Sand, SlopeError, SlopeWarp,
    StairFlight, MAX_GRADE_Q, MAX_SINK_MM, MAX_STAIR_RISE_MM, SLOPE_20_GRADE_Q, SOLE_MM,
};
use engine_core::alloc_probe::measure;

/// Matched clip per capsule speed, the way motion matching picks a clip near
/// the speed and the warp covers the residual: walk 1.5, run 4.0, sprint
/// 6.5 m/s.
fn clip_for(speed: f32) -> f32 {
    let clips = [1.5f32, 4.0, 6.5];
    let mut best = clips[0];
    for candidate in clips.iter().skip(1) {
        if (speed - *candidate).abs() < (speed - best).abs() {
            best = *candidate;
        }
    }
    best
}

/// Capsule speed (m/s) of one output root.
fn capsule_speed(out: &LocoOutput) -> f32 {
    let dp = out.root.dp;
    (dp[0] * dp[0] + dp[1] * dp[1]).sqrt() / TICK_DT
}

/// One locomotion output at `tick` for a unit wish, for receipt and rejection
/// checks.
fn sample_output(tick: u32) -> LocoOutput {
    Locomotion::new()
        .update(1.5, [0.0, 1.0], tick, None)
        .expect("loco")
}

/// One slope treadmill run: worst 3D slide past tick 40, the raw and held foot
/// points, and the clip-step replay hash.
struct SlopeRun {
    worst_mm: u32,
    raw: Vec<[f32; 3]>,
    held: Vec<[f32; 3]>,
    clip_hash: u64,
}

/// Full chain on one grade: locomotion emits the capsule intent, the slope warp
/// (or the plain warp when `cling` is false) covers the clip cursor, and the
/// planted treadmill feet ride the surface under the capsule root. The foot's
/// plane height is re-derived from its raw run, so the raw point always sits on
/// the incline; with the secant feed it is world-fixed and the plain warp drags
/// it up-slope. Returns the worst 3D slide metered from tick 40 on.
fn slope_run(grade: Grade, speed: f32, ticks: u32, cling: bool) -> SlopeRun {
    let clip = clip_for(speed);
    let mut slope = SlopeWarp::new(grade);
    slope
        .begin_transient(LocoState::Start, 30, clip, speed)
        .expect("slope window");
    let mut plain = GroundWarp::new();
    plain
        .begin_transient(LocoState::Start, 30, clip, speed)
        .expect("plain window");
    let mut machine = Locomotion::new();
    let mut locker = FootLocker::new(-10.0).expect("slope locker below any grade");
    let dir = grade.surface_dir();
    let plane_q = f32::from(i16::try_from(grade.grade_q()).expect("grade fits i16")) / 1000.0;
    let (mut root_z, mut belt_z) = (0.0f32, 0.0f32);
    let mut planted_now = [false; 2];
    let (mut worst, mut raw, mut held, mut clip_steps) = (0u32, Vec::new(), Vec::new(), Vec::new());
    for t in 0..ticks {
        if t == 40 {
            // Meter past the start transient: fresh anchors in the steady
            // regime, so only steady match error counts.
            locker.reset();
            planted_now = [false; 2];
        }
        let mut out = machine.update(speed, [0.0, 1.0], t, None).expect("loco");
        let capsule = capsule_speed(&out);
        let clip_step = if cling {
            slope
                .warp_output(&mut out, capsule, [0.0, 1.0], &LEGS)
                .expect("slope tick")
                .warp
                .clip_step
        } else {
            plain
                .warp_output(&mut out, capsule, [0.0, 1.0], &LEGS)
                .expect("plain tick")
                .clip_step
        };
        root_z += capsule * TICK_DT;
        belt_z += dir[2] * clip_step;
        clip_steps.push(clip_step);
        for foot in 0..2u8 {
            let slot = usize::from(foot);
            let down = planted_from_contacts(&out.contacts, t, foot);
            let side = if foot == 0 { 0.11 } else { -0.11 };
            let foot_z = root_z - belt_z;
            let point = [side, plane_q * foot_z + 0.03, foot_z];
            if !down {
                planted_now[slot] = false;
                locker.update(slot, point, false).expect("release");
                continue;
            }
            let held_point = locker.update(slot, point, true).expect("lock");
            if t >= 40 {
                worst = worst.max(slide_mm(point, held_point));
            }
            raw.push(point);
            held.push(held_point);
        }
    }
    SlopeRun {
        worst_mm: worst,
        raw,
        held,
        clip_hash: warp_hash(&clip_steps),
    }
}

/// One stair fixture run: the tick stream, replay hash, step counters, worst
/// placement error against the real tread, worst on-tread slide and the flat
/// baseline (how far below the tread a flat-ground planter would sit).
struct StairRun {
    ticks: Vec<GroundTick>,
    hash: u64,
    plants: u32,
    expected_plants: u32,
    steps_up: u32,
    steps_down: u32,
    placement_worst: u32,
    slide_worst: u32,
    flat_worst: u32,
}

/// Climb (`downhill` false) or descend a 180 mm / 300 mm flight at `speed` m/s
/// for `ticks` ticks through the real gait contacts. The clip is perfect during
/// a stance, so each plant is world-fixed at its world spot; the planter must
/// put it on the tread under that spot. Slide stays a within-tread meter, and
/// the flat baseline is the tread height a flat-ground planter would miss.
fn stair_run(speed: f32, ticks: u32, downhill: bool) -> StairRun {
    let flight = StairFlight::new(180, 300).expect("fixture stair");
    let mut feet = GroundFeet::new(flight);
    let mut machine = Locomotion::new();
    // Start mid-tread so both foot leads sit on the first tread.
    let mut root_mm = 150i64;
    let wish = if downhill { [0.0, -1.0] } else { [0.0, 1.0] };
    let (mut was, mut held, mut expected_plants) = ([false; 2], [[0i32; 3]; 2], 0u32);
    let (mut placement_worst, mut slide_worst, mut flat_worst) = (0u32, 0u32, 0u32);
    let mut stream = Vec::new();
    for t in 0..ticks {
        let out = machine.update(speed, wish, t, None).expect("loco");
        root_mm += i64::from((out.root.dp[1] * 1000.0).round() as i32);
        let mut raw = [[0i32; 3]; 2];
        let mut planted = [false; 2];
        for foot in 0..2u8 {
            let slot = usize::from(foot);
            let down = planted_from_contacts(&out.contacts, t, foot);
            planted[slot] = down;
            let side = if foot == 0 { 110 } else { -110 };
            let lead = if foot == 0 { 40 } else { -40 };
            let run = root_mm + lead;
            let z = i32::try_from(run).expect("fixture run fits i32");
            let y = i32::try_from(flight.height_mm(run) + i64::from(SOLE_MM)).expect("tread y");
            if down && !was[slot] {
                held[slot] = [side, y, z];
                expected_plants += 1;
            }
            raw[slot] = if down { held[slot] } else { [side, SOLE_MM, z] };
            was[slot] = down;
        }
        let tick = feet.tick(raw, planted, [0, 0]).expect("ground tick");
        for slot in 0..2 {
            if !tick.locked[slot] {
                continue;
            }
            let top = flight.height_mm(i64::from(tick.contact_mm[slot][2]));
            let expect = [
                tick.contact_mm[slot][0],
                i32::try_from(top + i64::from(SOLE_MM)).expect("tread y"),
                tick.contact_mm[slot][2],
            ];
            placement_worst = placement_worst.max(dist_mm(tick.contact_mm[slot], expect));
            slide_worst = slide_worst.max(tick.slide_mm[slot]);
            flat_worst =
                flat_worst.max(u32::try_from(tick.contact_mm[slot][1] - SOLE_MM).unwrap_or(0));
        }
        let mut lowest: Option<i64> = None;
        for slot in 0..2 {
            if tick.locked[slot] {
                let top = tick.tread[slot] * i64::from(flight.rise_mm());
                lowest = Some(lowest.map_or(top, |low| low.min(top)));
            }
        }
        assert_eq!(
            feet.pelvis_ground_mm(),
            lowest,
            "pelvis must drop to the lowest planted tread at t={t}"
        );
        stream.push(tick);
    }
    let hash = ground_replay_hash(&stream);
    StairRun {
        ticks: stream,
        hash,
        plants: feet.plants(),
        expected_plants,
        steps_up: feet.steps_up(),
        steps_down: feet.steps_down(),
        placement_worst,
        slide_worst,
        flat_worst,
    }
}

/// Seven planted ticks on flat sand: sink grows 0, 4, 8, 12, 16, 20, 24 mm while
/// the plant x/z stay fixed. Returns the tick stream and its replay hash.
fn sand_ticks() -> (Vec<GroundTick>, u64) {
    let sand = Sand::new(24, 6).expect("fixture sand");
    let flight = StairFlight::new(1, 1000).expect("flat tread");
    let mut feet = GroundFeet::new(flight);
    let mut stream = Vec::new();
    for age in 0..=sand.settle_ticks() {
        let sink = sand.sink_mm(age);
        let tick = feet
            .tick([[0, 0, 0], [0, 0, 0]], [true, false], [sink, 0])
            .expect("sand tick");
        assert_eq!(tick.sink_mm[0], sink);
        assert_eq!(tick.slide_mm[0], 0, "sink read as skate at age {age}");
        assert_eq!(tick.contact_mm[0], [0, SOLE_MM - sink, 0]);
        stream.push(tick);
    }
    let hash = ground_replay_hash(&stream);
    (stream, hash)
}

#[test]
fn grade_and_stair_math_is_integer() {
    let up = Grade::new(SLOPE_20_GRADE_Q).expect("20 degree grade");
    assert_eq!(up.grade_q(), 364);
    assert_eq!(up.sec_q(), 1064);
    assert!((up.surface_speed_m_s(1.0) - 1.064).abs() < 1e-6);
    assert!((up.surface_dir()[1] - 364.0 / 1064.0).abs() < 1e-6);
    assert!((up.surface_dir()[2] - 1000.0 / 1064.0).abs() < 1e-6);
    let down = Grade::new(-SLOPE_20_GRADE_Q).expect("downhill grade");
    assert_eq!(down.sec_q(), up.sec_q(), "secant is even in the grade");
    assert!(down.surface_dir()[1] < 0.0, "downhill points down");
    assert_eq!(Grade::FLAT.surface_speed_m_s(3.0), 3.0);
    assert!(Grade::FLAT.is_flat());
    assert_eq!(Grade::default(), Grade::FLAT);
    let stairs = StairFlight::new(180, 300).expect("fixture stair");
    assert_eq!(stairs.rise_mm(), 180);
    assert_eq!(stairs.tread_mm(), 300);
    assert_eq!(stairs.tread_index(0), 0);
    assert_eq!(stairs.tread_index(299), 0);
    assert_eq!(stairs.tread_index(300), 1);
    assert_eq!(stairs.height_mm(299), 0);
    assert_eq!(stairs.height_mm(300), 180);
    assert_eq!(stairs.height_mm(599), 180);
    assert_eq!(stairs.height_mm(600), 360);
    assert_eq!(stairs.tread_index(-1), -1);
    assert_eq!(stairs.height_mm(-1), -180);
    assert_eq!(dist_mm([0, 0, 0], [3, 4, 0]), 5);
    assert_eq!(dist_mm([0, 0, 0], [-3, -4, 0]), 5);
    assert_eq!(dist_mm([1, 2, 3], [1, 2, 3]), 0);
}

#[test]
fn slope_receipt_reports_grade_and_surface_speed() {
    let grade = Grade::new(SLOPE_20_GRADE_Q).expect("20 degree grade");
    let mut warp = SlopeWarp::new(grade);
    warp.begin_transient(LocoState::Start, 10, 1.5, 1.5)
        .expect("slope window");
    let mut out = sample_output(0);
    let receipt = warp
        .warp_output(&mut out, 1.5, [0.0, 1.0], &LEGS)
        .expect("slope tick");
    assert_eq!(receipt.grade_q, SLOPE_20_GRADE_Q);
    assert_eq!(receipt.sec_q, 1064);
    assert!((receipt.surface_speed_m_s - 1.596).abs() < 1e-5);
    assert_eq!(warp.surface_speed_m_s(), receipt.surface_speed_m_s);
    assert_eq!(warp.grade(), grade);
    // The clip cursor covers the surface: sec * planar * dt.
    assert!(
        (receipt.warp.clip_step - 1.596 * TICK_DT).abs() < 1e-6,
        "clip step {} misses the surface advance",
        receipt.warp.clip_step
    );
    // The root stays authoritative and bit-identical on the slope feed.
    let mut twin = Locomotion::new();
    let plain = twin.update(1.5, [0.0, 1.0], 1, None).expect("plain loco");
    let mut moved = Locomotion::new()
        .update(1.5, [0.0, 1.0], 1, None)
        .expect("moved loco");
    let before = moved.root;
    warp.warp_output(&mut moved, 1.5, [0.0, 1.0], &LEGS)
        .expect("slope tick");
    assert_eq!(moved.root, before, "slope warp moved the root");
    assert_eq!(moved.root, plain.root, "twin roots differ");
}

#[test]
fn slope_20_clings_under_1cm_up_and_down() {
    let mut plain_worst = 0u32;
    for grade_q in [SLOPE_20_GRADE_Q, -SLOPE_20_GRADE_Q] {
        let grade = Grade::new(grade_q).expect("20 degree grade");
        for speed in [0.5f32, 1.0, 1.5, 2.0, 4.0, 6.0, 7.0] {
            let clung = slope_run(grade, speed, 120, true);
            let plain = slope_run(grade, speed, 120, false);
            println!(
                "LM6 slope q={grade_q} v={speed}: clung={} mm plain={} mm",
                clung.worst_mm, plain.worst_mm
            );
            assert!(
                clung.worst_mm <= SLIDE_BUDGET_MM,
                "q={grade_q} v={speed}: slope slide {} mm past 1 cm",
                clung.worst_mm
            );
            plain_worst = plain_worst.max(plain.worst_mm);
        }
    }
    assert!(
        plain_worst > SLIDE_BUDGET_MM,
        "the plain warp must drag on the slope ({plain_worst} mm) or the fixture proves nothing"
    );
}

#[test]
fn mid_stance_riser_steps_up_without_dragging() {
    let flight = StairFlight::new(180, 300).expect("fixture stair");
    let mut feet = GroundFeet::new(flight);
    let raw = |z: i32| [0, 0, z];
    // Plant 2 mm before the first riser on tread 0.
    let plant = feet
        .tick([raw(298), [0, 0, 0]], [true, false], [0, 0])
        .expect("plant");
    assert_eq!(plant.tread[0], 0);
    assert_eq!(plant.slide_mm[0], 0);
    assert_eq!(plant.contact_mm[0], [0, SOLE_MM, 298]);
    // One millimeter of clip drift on the same tread is metered honestly.
    let drift = feet
        .tick([raw(299), [0, 0, 0]], [true, false], [0, 0])
        .expect("drift");
    assert_eq!(drift.tread[0], 0);
    assert_eq!(drift.slide_mm[0], 1);
    // The raw run crosses the riser: the planter steps up instead of dragging.
    let step = feet
        .tick([raw(300), [0, 0, 0]], [true, false], [0, 0])
        .expect("step");
    assert_eq!(step.tread[0], 1);
    assert_eq!(step.contact_mm[0], [0, 180 + SOLE_MM, 300]);
    assert_eq!(step.slide_mm[0], 0, "a step must not read as slide");
    assert_eq!(feet.steps_up(), 1);
    assert_eq!(feet.steps_down(), 0);
    assert_eq!(feet.plants(), 1, "a re-anchor is a step, not a new plant");
    // The meter still catches real skate after the step.
    assert_eq!(
        feet.tick([raw(304), [0, 0, 0]], [true, false], [0, 0])
            .expect("hold")
            .slide_mm[0],
        4
    );
    assert_eq!(
        feet.tick([raw(308), [0, 0, 0]], [true, false], [0, 0])
            .expect("hold")
            .slide_mm[0],
        8
    );
    let skate = feet
        .tick([raw(312), [0, 0, 0]], [true, false], [0, 0])
        .expect("hold");
    assert_eq!(skate.slide_mm[0], 12);
    assert!(skate.slide_mm[0] > SLIDE_BUDGET_MM, "meter went blind");
    // A no-step locker drags the foot through the riser: the fixture bites.
    let mut flat = FootLocker::with_defaults();
    flat.update(0, [0.0, 0.03, 0.298], true)
        .expect("flat plant");
    let stepped = [0.0, 0.210, 0.300];
    flat.update(0, stepped, true).expect("flat hold");
    let anchor = flat.anchor(0).expect("anchor read").expect("anchored");
    let baseline = slide_mm(stepped, anchor);
    assert!(
        baseline >= 180,
        "no-step locker drags {baseline} mm through a 180 mm riser"
    );
    // Descending crosses the same boundary downward.
    let mut down = GroundFeet::new(flight);
    down.tick([raw(302), [0, 0, 0]], [true, false], [0, 0])
        .expect("high plant");
    let lower = down
        .tick([raw(299), [0, 0, 0]], [true, false], [0, 0])
        .expect("step down");
    assert_eq!(lower.tread[0], 0);
    assert_eq!(lower.contact_mm[0], [0, SOLE_MM, 299]);
    assert_eq!(lower.slide_mm[0], 0);
    assert_eq!(down.steps_down(), 1);
    assert_eq!(down.steps_up(), 0);
}

#[test]
fn stair_climb_plants_on_treads_and_counts_steps() {
    let climb = stair_run(1.2, 240, false);
    println!(
        "LM6 stairs: plants={} up={} down={} placement={} mm slide={} mm flat={} mm hash={:016x}",
        climb.plants,
        climb.steps_up,
        climb.steps_down,
        climb.placement_worst,
        climb.slide_worst,
        climb.flat_worst,
        climb.hash
    );
    assert_eq!(climb.plants, climb.expected_plants);
    assert!(climb.plants > 4, "fixture planted too rarely");
    assert!(climb.steps_up > 0, "climb counted no steps");
    assert_eq!(climb.steps_down, 0, "a climb stepped down");
    assert_eq!(
        climb.steps_up + climb.steps_down,
        climb.plants - 2,
        "every plant after each foot's first must count one step"
    );
    assert_eq!(climb.placement_worst, 0, "plant missed its tread");
    assert!(
        climb.slide_worst <= SLIDE_BUDGET_MM,
        "stair plants skate {} mm",
        climb.slide_worst
    );
    assert!(
        climb.flat_worst >= 180,
        "flat baseline misses the tread by only {} mm",
        climb.flat_worst
    );
    // Descending counts downward steps only.
    let descent = stair_run(1.2, 180, true);
    println!(
        "LM6 stairs down: plants={} up={} down={} placement={} mm slide={} mm",
        descent.plants,
        descent.steps_up,
        descent.steps_down,
        descent.placement_worst,
        descent.slide_worst
    );
    assert_eq!(descent.plants, descent.expected_plants);
    assert!(descent.steps_down > 0, "descent counted no steps");
    assert_eq!(descent.steps_up, 0, "a descent stepped up");
    assert_eq!(descent.steps_up + descent.steps_down, descent.plants - 2);
    assert_eq!(descent.placement_worst, 0);
    assert!(descent.slide_worst <= SLIDE_BUDGET_MM);
}

#[test]
fn sand_sink_is_reported_not_skated() {
    let sand = Sand::new(24, 6).expect("fixture sand");
    assert_eq!(
        [0u32, 1, 2, 3, 4, 5, 6, 9].map(|age| sand.sink_mm(age)),
        [0, 4, 8, 12, 16, 20, 24, 24]
    );
    let mut flat = FootLocker::with_defaults();
    let mut baseline_worst = 0u32;
    for age in 0..=sand.settle_ticks() {
        let sink = sand.sink_mm(age);
        let contact = [
            0.0,
            f32::from(i16::try_from(SOLE_MM - sink).expect("mm fits")) / 1000.0,
            0.0,
        ];
        flat.update(0, contact, true).expect("flat hold");
        let anchor = flat.anchor(0).expect("anchor read").expect("anchored");
        baseline_worst = baseline_worst.max(slide_mm(contact, anchor));
    }
    assert_eq!(
        baseline_worst,
        u32::try_from(sand.depth_mm()).expect("depth fits"),
        "a flat locker must read the full sink as slide"
    );
    assert!(baseline_worst > SLIDE_BUDGET_MM);
    let (first, hash) = sand_ticks();
    let (second, twin) = sand_ticks();
    assert_eq!(first, second);
    assert_eq!(hash, twin);
    assert_eq!(first.last().expect("ticks").sink_mm[0], 24);
    println!("LM6 sand replay: hash={hash:016x} ticks={}", first.len());
}

#[test]
fn slope_replay_exact_x2() {
    let grade = Grade::new(SLOPE_20_GRADE_Q).expect("20 degree grade");
    let first = slope_run(grade, 4.0, 90, true);
    let second = slope_run(grade, 4.0, 90, true);
    assert_eq!(first.raw, second.raw);
    assert_eq!(first.held, second.held);
    assert_eq!(first.clip_hash, second.clip_hash);
    assert_eq!(feet_hash(&first.raw), feet_hash(&second.raw));
    println!(
        "LM6 slope replay: hash={:016x} feet={:016x} ticks={} worst={} mm",
        first.clip_hash,
        feet_hash(&first.raw),
        first.raw.len(),
        first.worst_mm
    );
}

#[test]
fn stair_replay_exact_x2() {
    let first = stair_run(1.2, 180, false);
    let second = stair_run(1.2, 180, false);
    assert_eq!(first.ticks, second.ticks);
    assert_eq!(first.hash, second.hash);
    assert_eq!(first.plants, second.plants);
    assert_eq!(first.steps_up, second.steps_up);
    println!(
        "LM6 stair replay: hash={:016x} ticks={} plants={} steps={}",
        first.hash,
        first.ticks.len(),
        first.plants,
        first.steps_up
    );
}

#[test]
fn failures_are_typed_never_panics() {
    assert_eq!(
        Grade::new(MAX_GRADE_Q + 1),
        Err(SlopeError::GradeTooSteep(MAX_GRADE_Q + 1))
    );
    assert_eq!(
        Grade::new(-MAX_GRADE_Q - 1),
        Err(SlopeError::GradeTooSteep(-MAX_GRADE_Q - 1))
    );
    assert_eq!(StairFlight::new(0, 300), Err(SlopeError::BadStair("rise")));
    assert_eq!(
        StairFlight::new(MAX_STAIR_RISE_MM + 1, 600),
        Err(SlopeError::BadStair("rise"))
    );
    assert_eq!(
        StairFlight::new(180, 179),
        Err(SlopeError::BadStair("tread"))
    );
    assert_eq!(Sand::new(0, 6), Err(SlopeError::BadSand("depth")));
    assert_eq!(
        Sand::new(MAX_SINK_MM + 1, 6),
        Err(SlopeError::BadSand("depth"))
    );
    assert_eq!(Sand::new(24, 0), Err(SlopeError::BadSand("settle")));
    let mut feet = GroundFeet::new(StairFlight::new(180, 300).expect("fixture stair"));
    let before = feet.clone();
    assert_eq!(
        feet.tick([[0, 0, 0], [0, 0, 0]], [true, true], [MAX_SINK_MM + 1, 0]),
        Err(SlopeError::BadSink(MAX_SINK_MM + 1))
    );
    assert_eq!(feet, before, "a rejected sink touched the planter");
    let grade = Grade::new(SLOPE_20_GRADE_Q).expect("20 degree grade");
    let mut warp = SlopeWarp::new(grade);
    warp.begin_transient(LocoState::Start, 10, 1.5, 1.5)
        .expect("slope window");
    let too_fast = grade.surface_speed_m_s(MAX_WISH_SPEED);
    assert_eq!(
        warp.begin_transient(LocoState::Start, 10, 1.5, MAX_WISH_SPEED),
        Err(SlopeError::SurfaceTooFast(too_fast))
    );
    assert_eq!(
        warp.begin_transient(LocoState::Start, 10, 1.5, f32::NAN),
        Err(SlopeError::Warp(WarpError::NonFinite("capsule")))
    );
    assert_eq!(
        warp.begin_transient(LocoState::Start, 10, 1.5, -1.0),
        Err(SlopeError::Warp(WarpError::NegativeSpeed(-1.0)))
    );
    let mut out = sample_output(0);
    let snapshot = format!("{out:?}");
    let before_warp = warp.clone();
    assert_eq!(
        warp.warp_output(&mut out, MAX_WISH_SPEED + 1.0, [0.0, 1.0], &LEGS),
        Err(SlopeError::Warp(WarpError::SpeedTooHigh(
            MAX_WISH_SPEED + 1.0
        )))
    );
    assert_eq!(format!("{out:?}"), snapshot, "reject touched the output");
    assert_eq!(warp, before_warp, "reject touched the slope warp");
    assert!(SlopeError::BadSink(MAX_SINK_MM + 1)
        .to_string()
        .contains("201"));
    assert_eq!(
        SlopeError::from(WarpError::BadWishDir),
        SlopeError::Warp(WarpError::BadWishDir)
    );
}

#[test]
fn steady_tick_uses_no_heap() {
    // Calibration: the probe must see a real allocation.
    let ((), heap) = measure(|| {
        std::hint::black_box(vec![0u8; 64]);
    });
    assert!(heap.calls >= 1, "alloc probe is blind");
    let grade = Grade::new(SLOPE_20_GRADE_Q).expect("20 degree grade");
    let mut warp = SlopeWarp::new(grade);
    warp.begin_transient(LocoState::Start, 10, 1.5, 1.5)
        .expect("slope window");
    let mut out = sample_output(0);
    let mut feet = GroundFeet::new(StairFlight::new(180, 300).expect("fixture stair"));
    let ((), heap) = measure(|| {
        for _ in 0..10 {
            std::hint::black_box(
                warp.warp_output(&mut out, 1.5, [0.0, 1.0], &LEGS)
                    .expect("slope tick"),
            );
            std::hint::black_box(
                feet.tick([[0, 0, 0], [0, 0, 0]], [true, true], [3, 3])
                    .expect("ground tick"),
            );
        }
    });
    assert_eq!(heap.calls, 0, "LM6 steady tick allocated");
}

/// Cost report for the 0.05 ms bar: `cargo test -p engine-anim --release
/// --lib slope_cost -- --ignored --nocapture`.
#[test]
#[ignore = "timing report, run in release on the reference CPU"]
#[expect(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    reason = "wall clock in a test-only timing report, never in the tick"
)]
fn slope_cost_per_tick() {
    let grade = Grade::new(SLOPE_20_GRADE_Q).expect("20 degree grade");
    let mut warp = SlopeWarp::new(grade);
    warp.begin_transient(LocoState::Start, 10, 1.5, 1.5)
        .expect("slope window");
    let mut out = sample_output(0);
    let t = std::time::Instant::now();
    for _ in 0..10_000 {
        warp.warp_output(&mut out, 1.5, [0.0, 1.0], &LEGS)
            .expect("slope tick");
    }
    let ns = t.elapsed().as_nanos() / 10_000;
    println!("LM6 slope: {ns} ns per tick (bar 50000)");
    assert!(ns < 50_000, "{ns} ns past the 0.05 ms bar");
}
