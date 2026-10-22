//! FV6: the scripted 600-tick first-person eye tape and its receipt.
//!
//! One tape drives the fighter and the look deltas for 600 ticks. The
//! fighter track runs through the real arena match, so the feet position and
//! the facing come from the authority, never from a hand-written path. The
//! look deltas ride the presentation-only channel of `engine-input` (FV2),
//! beside the hashed input and never inside it.
//!
//! Every tick builds the eye the first-person view wears (FV1), folds the
//! eye-space trauma kick into it (FV5) and hands it to the render camera
//! feed (FV3). The eye bytes replay exactly twice, the per-frame eye travel
//! stays inside the framing budget the fight-camera spring test uses (peak
//! at or under 25 cm at 30 fps) and the receipt prints the eye travel, the
//! shake peak and the submit cost.
//!
//! Presentation only: the eye never enters a tick hash, so the golden
//! `d57b5c8ae3096614` does not move.
//!
//! The camera feed is the windowed leg's (`engine-game`'s `app` feature), so
//! the camera and the submit receipt ride that feature. The tape itself
//! (fighter, look deltas, eye bytes, travel and shake) runs in the default
//! headless binary, so the proof line stays pure CPU work.
use engine_game::fps_view::{
    render_eye, render_eye_shaken, EyeTrauma, LogicalFrame, RenderEye, EYE_HEIGHT_MM,
    EYE_SHAKE_GAIN_Q,
};
use engine_game::{App, GameConfig, HUMAN_SLOT};
use engine_input::look::{LookState as ChannelLook, LOOK_RATES};
use engine_input::{Buttons, FighterId, FighterInput};
use std::f32::consts::{PI, TAU};

/// Ticks the tape drives: the legacy fixture's window.
const TICKS: u32 = 600;
/// Presented frames per second the travel budget is written for.
const PRESENT_FPS: u32 = 30;
/// Sim ticks per presented frame at 30 fps (the sim runs at 60 Hz).
const TICKS_PER_FRAME: u32 = 60 / PRESENT_FPS;
/// Per-frame eye travel ceiling in centimeters at 30 fps: the framing
/// budget the fight-camera spring test uses (`camera_rig.rs`).
const EYE_TRAVEL_BUDGET_CM: f32 = 25.0;
/// Vertical field of view the eye camera takes, radians.
#[cfg(feature = "app")]
const FOV_Y_RAD: f32 = 1.05;
/// Picture aspect the eye camera takes, width over height.
#[cfg(feature = "app")]
const ASPECT: f32 = 16.0 / 9.0;

/// The tape: the recorded fighter track and the look-delta track, side by
/// side.
struct Tape {
    /// One recorded fighter input per tick.
    fighter: Vec<FighterInput>,
    /// One scripted look delta pair per tick, in millidegrees.
    look: Vec<(i32, i32)>,
}

/// The look track: the scripted deltas the eye follows, in millidegrees.
/// The yaw sweeps right for a minute and left for a minute; the pitch ramps
/// up and eases back. Fixed, so the look part of the tape is a recording.
fn scripted_look() -> Vec<(i32, i32)> {
    (0..TICKS)
        .map(|tick| {
            let dx = if tick % 120 < 60 { 900 } else { -900 };
            let dy = if tick.is_multiple_of(3) { 400 } else { -150 };
            (dx, dy)
        })
        .collect()
}

/// Record the fighter track: the script closes on the rival, so the fight
/// really happens and the eye really takes blows. The recorded inputs are
/// the tape, so the second run replays the same bytes.
///
/// The wish follows the arena's own convention (`arena/src/lib.rs`): the
/// forward wish rides the world x axis, the aside wish the world z axis,
/// and the facing is the heading toward the rival in centidegrees (0 is
/// +z, 9000 is +x).
fn record_fighter_track() -> Vec<FighterInput> {
    let config = GameConfig {
        seed: 2040,
        ticks: TICKS,
        fighters: 2,
        half_extent_m: 20.0,
    };
    let mut app = App::new(config).expect("the session builds");
    let mut track = Vec::with_capacity(TICKS as usize);
    for tick in 0..TICKS {
        // Before the first tick the session has no views yet, so the script
        // starts with a neutral heading and no wish.
        let (facing, mx, mz, in_reach) = match app
            .fighter_views()
            .iter()
            .find(|view| view.slot == HUMAN_SLOT)
        {
            Some(me) => {
                let foe = app
                    .fighter_views()
                    .iter()
                    .find(|view| view.slot != HUMAN_SLOT)
                    .copied()
                    .expect("the rival has a view");
                let (dx, dz) = (foe.pos_m[0] - me.pos_m[0], foe.pos_m[2] - me.pos_m[2]);
                // The heading toward the rival, in centidegrees (0 is +z).
                let heading = (dx.atan2(dz) * 18_000.0 / PI).round() as i16;
                let len = (dx * dx + dz * dz).sqrt();
                // Advance until the blades can reach, then hold.
                let wish = if len > 1.2 {
                    (axis(dx / len), axis(dz / len))
                } else {
                    (0, 0)
                };
                (heading, wish.0, wish.1, len < 1.6)
            }
            None => (0, 0, 0, false),
        };
        let input = FighterInput {
            tick,
            fighter: FighterId::new(HUMAN_SLOT).expect("the human slot is inside the cap"),
            mx,
            mz,
            facing,
            // A swing every fifth tick once the blades can reach, so the
            // fight lands real cues.
            buttons: if in_reach && tick.is_multiple_of(5) {
                Buttons(engine_input::fighter::ATTACK)
            } else {
                Buttons::NONE
            },
        };
        app.tick(input);
        track.push(input);
    }
    track
}

/// One world axis as a wish percent, signed the way the arena reads it.
/// The percent is of the top speed along that axis.
fn axis(unit: f32) -> i8 {
    let percent = (unit * 100.0).round();
    i8::try_from(percent.clamp(-100.0, 100.0) as i32).unwrap_or(0)
}

/// The recorded tape: 600 fighter inputs plus 600 look deltas.
fn recorded_tape() -> Tape {
    Tape {
        fighter: record_fighter_track(),
        look: scripted_look(),
    }
}

/// One tick's eye plus what the receipt measures.
struct EyeTick {
    /// The eye the view wears this tick.
    eye: RenderEye,
    /// The eye-space kick the trauma folded in, millimeters.
    kick_mm: [f32; 3],
    /// The trauma level this tick carried, permille.
    trauma_permille: u32,
}

/// The whole tape: 600 eyes, plus the camera each eye fed on the windowed
/// leg.
struct EyeRun {
    ticks: Vec<EyeTick>,
    /// The camera each eye fed, as its eye and target in meters.
    #[cfg(feature = "app")]
    cameras: Vec<([f32; 3], [f32; 3])>,
    /// The best per-frame submit cost over the run, milliseconds.
    #[cfg(feature = "app")]
    submit_ms: f64,
}

/// Run the tape once: the fighter track through the real match, the look
/// deltas through the presentation channel, the eye (and, on the windowed
/// leg, the camera) per tick. Pure over the two tracks: the same tracks give
/// the same bytes twice.
fn run_tape(tape: &Tape) -> EyeRun {
    let config = GameConfig {
        seed: 2040,
        ticks: TICKS,
        fighters: 2,
        half_extent_m: 20.0,
    };
    let mut app = App::new(config).expect("the session builds");
    let mut channel = ChannelLook::new(LOOK_RATES).expect("the table rates validate");
    let mut trauma = EyeTrauma::default();
    let mut ticks = Vec::with_capacity(TICKS as usize);
    #[cfg(feature = "app")]
    let mut cameras = Vec::with_capacity(TICKS as usize);
    let mut prev_mm = [0i32; 3];
    // The best per-frame submit cost the game pays: the look fold, the eye
    // pose with its shake, the camera and the matrices the submit path
    // uploads. Measured inside the run, so the number covers the real path.
    #[cfg(feature = "app")]
    let mut best_submit_ms = f64::INFINITY;
    for tick in 0..TICKS {
        let outcome = app.tick(tape.fighter[tick as usize]);
        assert!(outcome.input_accepted, "the scripted input is accepted");
        let view = app
            .fighter_views()
            .iter()
            .find(|view| view.slot == HUMAN_SLOT)
            .copied()
            .expect("the human slot has a view");
        #[cfg(feature = "app")]
        let started = std::time::Instant::now();
        // The look channel rides beside the tape: it never touches the
        // hashed input record.
        let (dx, dy) = tape.look[tick as usize];
        channel
            .apply_deltas(dx, dy)
            .expect("the scripted look delta is in range");
        // The channel speaks centidegrees; the eye speaks radians, and the
        // eye yaw is the body facing plus the look channel's own turn.
        let look = engine_game::fps_view::LookState {
            yaw_rad: wrap_pi(view.yaw + channel.yaw_centideg as f32 * PI / 18_000.0),
            pitch_rad: channel.pitch_centideg as f32 * PI / 18_000.0,
        };
        let pos_mm = mm_of(view.pos_m);
        let frame = LogicalFrame {
            pos_mm,
            prev_mm,
            look,
        };
        prev_mm = pos_mm;
        // The trauma the fight cues gave the eye: a blow taken counts in
        // full, a blow landed a third, everything else nothing. The level
        // decays between the blows, so the shake fades instead of popping.
        let level = trauma.feed(tick, &outcome.cues, HUMAN_SLOT);
        let eye = render_eye_shaken(
            &frame,
            1000,
            EYE_HEIGHT_MM,
            level,
            tick,
            u32::from(EYE_SHAKE_GAIN_Q),
        );
        // The applied kick: the shaken eye against the calm one.
        let calm = render_eye(&frame, 1000, EYE_HEIGHT_MM);
        let kick_mm = [
            eye.eye_mm[0] - calm.eye_mm[0],
            eye.eye_mm[1] - calm.eye_mm[1],
            eye.eye_mm[2] - calm.eye_mm[2],
        ];
        #[cfg(feature = "app")]
        {
            let camera =
                engine_game::fps_view::camera_for_eye(&eye, FOV_Y_RAD, ASPECT).expect("a camera");
            // The matrices the submit path uploads: the last work the game
            // does before the frame leaves its hands.
            let matrix = camera.view_proj().expect("the camera has a view-proj");
            std::hint::black_box(matrix);
            cameras.push((camera.eye.to_array(), camera.target.to_array()));
            let ms = started.elapsed().as_secs_f64() * 1000.0;
            if ms < best_submit_ms {
                best_submit_ms = ms;
            }
        }
        ticks.push(EyeTick {
            eye,
            kick_mm,
            trauma_permille: level,
        });
    }
    #[cfg(feature = "app")]
    let run = EyeRun {
        ticks,
        cameras,
        submit_ms: best_submit_ms,
    };
    #[cfg(not(feature = "app"))]
    let run = EyeRun { ticks };
    run
}

/// Whole millimeters of a meters position: the eye lane's own unit.
fn mm_of(pos_m: [f32; 3]) -> [i32; 3] {
    [
        (pos_m[0] * 1000.0).round() as i32,
        (pos_m[1] * 1000.0).round() as i32,
        (pos_m[2] * 1000.0).round() as i32,
    ]
}

/// Wrap an angle into the -pi..pi range the eye's yaw uses.
fn wrap_pi(rad: f32) -> f32 {
    let mut wrapped = rad.rem_euclid(TAU);
    if wrapped > PI {
        wrapped -= TAU;
    }
    wrapped
}

/// The eye bytes of one tick: the three position lanes plus the three
/// angles, as their exact bit patterns.
fn eye_bytes(eye: &RenderEye, out: &mut Vec<u8>) {
    for lane in eye.eye_mm {
        out.extend_from_slice(&lane.to_bits().to_le_bytes());
    }
    out.extend_from_slice(&eye.yaw_rad.to_bits().to_le_bytes());
    out.extend_from_slice(&eye.pitch_rad.to_bits().to_le_bytes());
    out.extend_from_slice(&eye.roll_rad.to_bits().to_le_bytes());
}

/// Distance between two eye positions in centimeters. The eye lanes are
/// millimeters, so the distance is millimeters over ten.
fn travel_cm(a: [f32; 3], b: [f32; 3]) -> f32 {
    let dx = b[0] - a[0];
    let dy = b[1] - a[1];
    let dz = b[2] - a[2];
    (dx * dx + dy * dy + dz * dz).sqrt() / 10.0
}

/// The peak per-frame eye travel in centimeters at 30 fps: the presented
/// frames are two sim ticks apart, so the travel is the distance from one
/// presented eye to the next.
fn peak_travel_cm(run: &EyeRun) -> f32 {
    let mut peak = 0.0f32;
    let mut prev: Option<[f32; 3]> = None;
    for (index, tick) in run.ticks.iter().enumerate() {
        if !(index as u32).is_multiple_of(TICKS_PER_FRAME) {
            continue;
        }
        let at = tick.eye.eye_mm;
        if let Some(before) = prev {
            let cm = travel_cm(before, at);
            if cm > peak {
                peak = cm;
            }
        }
        prev = Some(at);
    }
    peak
}

/// The peak applied eye-space kick in centimeters: the shake the trauma
/// folded into the eye.
fn peak_shake_cm(run: &EyeRun) -> f32 {
    let mut peak = 0.0f32;
    for tick in &run.ticks {
        let [x, y, z] = tick.kick_mm;
        let mm = (x * x + y * y + z * z).sqrt();
        if mm > peak {
            peak = mm;
        }
    }
    peak / 10.0
}

/// The peak trauma level the tape carried, permille: the number the shake
/// peak rides on.
fn peak_trauma_permille(run: &EyeRun) -> u32 {
    run.ticks
        .iter()
        .map(|tick| tick.trauma_permille)
        .max()
        .unwrap_or(0)
}

#[test]
fn fps_view_tape_drives_600_ticks_and_replays_exactly_twice() {
    let tape = recorded_tape();
    let first = run_tape(&tape);
    let second = run_tape(&tape);
    assert_eq!(
        first.ticks.len(),
        TICKS as usize,
        "the tape covers 600 ticks"
    );
    // The eye bytes replay exactly: the same tracks give the same bytes.
    let mut first_bytes = Vec::new();
    let mut second_bytes = Vec::new();
    for (a, b) in first.ticks.iter().zip(second.ticks.iter()) {
        eye_bytes(&a.eye, &mut first_bytes);
        eye_bytes(&b.eye, &mut second_bytes);
    }
    assert_eq!(
        first_bytes, second_bytes,
        "the 600-tick eye bytes must replay exactly"
    );
    assert!(!first_bytes.is_empty(), "the tape recorded eye bytes");
    // The camera the eye fed replays too: same eye, same camera bytes.
    #[cfg(feature = "app")]
    assert_eq!(
        first.cameras, second.cameras,
        "the camera feed must replay exactly"
    );
    // The tape really moved the eye: a still tape would prove nothing.
    let first_eye = first.ticks[0].eye.eye_mm;
    let last_eye = first.ticks[TICKS as usize - 1].eye.eye_mm;
    assert_ne!(
        first_eye, last_eye,
        "the tape must move the eye over the run"
    );
    // The fight landed real cues, so the shake path really ran.
    let shaken = first
        .ticks
        .iter()
        .any(|tick| tick.kick_mm != [0.0, 0.0, 0.0]);
    assert!(shaken, "the tape must shake the eye at least once");
    // The eye took a real blow: the trauma reached a landed hit, so the
    // shake is a fight's shake and not a stray number.
    let trauma = peak_trauma_permille(&first);
    assert!(
        trauma >= engine_game::camera_feed::TRAUMA_HIT,
        "the eye must take a blow (trauma {trauma} under the hit's {})",
        engine_game::camera_feed::TRAUMA_HIT
    );
}

#[test]
fn fps_view_tape_eye_travel_stays_inside_the_framing_budget() {
    let tape = recorded_tape();
    let run = run_tape(&tape);
    let peak = peak_travel_cm(&run);
    assert!(
        peak <= EYE_TRAVEL_BUDGET_CM,
        "per-frame eye travel peaked at {peak:.2} cm, over the {EYE_TRAVEL_BUDGET_CM} cm budget at {PRESENT_FPS} fps"
    );
    // The budget only means something when the tape really moves the eye.
    assert!(
        peak > 0.0,
        "the tape must move the eye for the travel budget to mean anything"
    );
}

#[test]
fn fps_view_tape_receipt_prints_eye_travel_shake_peak_and_submit_ms() {
    let tape = recorded_tape();
    let run = run_tape(&tape);
    let travel = peak_travel_cm(&run);
    let shake = peak_shake_cm(&run);
    let trauma = peak_trauma_permille(&run);
    assert!(
        travel <= EYE_TRAVEL_BUDGET_CM,
        "eye travel {travel:.2} cm over the {EYE_TRAVEL_BUDGET_CM} cm budget"
    );
    // The receipt names all three numbers the row asks for.
    #[cfg(feature = "app")]
    {
        let submit = run.submit_ms;
        println!(
            "FV6 eye tape: ticks={} presented_fps={} eye_travel_peak_cm={travel:.2} \
             shake_peak_cm={shake:.2} submit_ms={submit:.4} trauma_peak_permille={trauma} \
             (submit: look->eye->camera->view-proj, budget {:.1} ms)",
            TICKS,
            PRESENT_FPS,
            engine_render::graph::GPU_SUBMIT_BUDGET_MS
        );
        assert!(submit.is_finite(), "the submit cost must be a real number");
        assert!(submit >= 0.0, "the submit cost cannot be negative");
    }
    #[cfg(not(feature = "app"))]
    println!(
        "FV6 eye tape: ticks={} presented_fps={} eye_travel_peak_cm={travel:.2} \
         shake_peak_cm={shake:.2} trauma_peak_permille={trauma}",
        TICKS, PRESENT_FPS
    );
}

/// The submit bar: the eye feed's own cost against the posted submit
/// budget. A wall-clock bar, so it stays out of the default suite and runs
/// under `--ignored`.
#[test]
#[cfg(feature = "app")]
#[ignore = "timing: the submit bar runs under --ignored"]
fn fps_view_tape_submit_cost_stays_inside_the_posted_budget() {
    let tape = recorded_tape();
    let run = run_tape(&tape);
    let submit = run.submit_ms;
    println!(
        "FV6 eye tape submit: {submit:.4} ms (budget {:.1} ms)",
        engine_render::graph::GPU_SUBMIT_BUDGET_MS
    );
    assert!(
        submit <= engine_render::graph::GPU_SUBMIT_BUDGET_MS,
        "the eye feed cost {submit:.4} ms, over the {:.1} ms budget",
        engine_render::graph::GPU_SUBMIT_BUDGET_MS
    );
}
