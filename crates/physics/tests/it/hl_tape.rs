//! FEEL12: the walk answers to a known game, not to our taste.
//!
//! One 120-tick tape (run up, stop, turn, jump, air strafe) runs through the
//! integer controller (`engine_physics::controller`, today's step: the air
//! channel unarmed, so `AirTuning` and the golden hash are untouched) and
//! through the published Half-Life movement model
//! (`engine_physics::oracle`, offline f64, never in the tick). The table is
//! printed per tick under `--nocapture`; the band is checked at the settled
//! tick of each channel the row names:
//!
//! - ground speed after the accel ramp (`RUN_END`),
//! - stopped speed after friction (`STOP_END`),
//! - jump apex height (the peak over the flight), where the gap must be the
//!   `v0 * dt / 2` half-step of our integrator and nothing more.
//!
//! Our controller commands a displacement per tick, not a velocity, so its
//! curve is the tape's wish until the air phase scales it: the transient of
//! the ramp and of the friction decay belongs to the model, and the table
//! prints that gap rather than hiding it. The air row is printed and not
//! asserted: our air channel is a 0.3 displacement scale, HL's is
//! `PM_AirAccelerate`, an acceleration with a 30 u/s addspeed cap; the two are
//! different laws and the gap is the flight column.
#![cfg(feature = "rapier")]

use engine_physics::controller::Controller;
use engine_physics::oracle::{HlMove, Params};
use engine_physics::rapier::{FIXED_DT, GRAVITY_Y};

/// Capsule radius of the fixture, meters.
const RADIUS: f32 = 0.3;
/// Capsule mid-section, meters (total height is `HEIGHT + 2 * RADIUS`).
const HEIGHT: f32 = 0.8;
/// Capsule half height: the feet hold the floor when the center is here.
const HALF: f32 = HEIGHT * 0.5 + RADIUS;

/// Tape length, one tick per frame at `FIXED_DT`.
const TICKS: usize = 120;
/// Last tick of the run-up (ticks 0..=23, 0.4 s): the oracle reaches its cap.
const RUN_END: usize = 23;
/// Last tick of the stop (ticks 24..=59, 0.6 s): friction zeroes both.
const STOP_END: usize = 59;
/// Last tick of the turn (ticks 60..=75, 0.27 s): a 90 degree wish change.
const TURN_END: usize = 75;
/// The jump press (a fresh edge); the wish is zero, so both models take off
/// from the same line and the strafe starts on the next frame.
const JUMP_TICK: usize = 76;
/// Last tick of the air strafe (ticks 77..=96): wish across the flight line.
const STRAFE_END: usize = 96;

/// Ground-speed band after the accel ramp, m/s: 0.5% of the 3.924 m/s cap,
/// wide enough for our f32 step against the oracle's f64.
const GROUND_SPEED_TOL_M_S: f64 = 0.02;
/// Stopped-speed band after friction, m/s: the model's own floor is 0.1 u/s
/// (1.2 mm/s) and both settle at zero on this tape.
const STOPPED_SPEED_TOL_M_S: f64 = 0.002;
/// Jump apex band, m: our semi-implicit step advances the height with the
/// post-gravity speed, so at the same take-off speed it sheds `v0 * dt / 2` =
/// 27.3 mm of the apex (4.9%); the model's split gravity is exact. 30 mm holds
/// that known bias, which the table and the band line print.
const APEX_TOL_M: f64 = 0.03;

/// The tape's input at one tick: forward/side stick as a world direction, and
/// the jump button's state. Same input for both models.
fn input(tick: usize) -> ([f64; 2], bool) {
    let dir = if tick <= RUN_END {
        [1.0, 0.0]
    } else if tick <= STOP_END {
        [0.0, 0.0]
    } else if tick <= TURN_END {
        [0.0, 1.0]
    } else if tick <= STRAFE_END {
        // The jump tick itself holds no wish, so both models take off from the
        // same line; the strafe starts on the next frame.
        if tick == JUMP_TICK {
            [0.0, 0.0]
        } else {
            [-1.0, 0.0]
        }
    } else {
        [0.0, 0.0]
    };
    (dir, tick == JUMP_TICK)
}

/// Phase name for the table.
fn phase(tick: usize, airborne: bool) -> &'static str {
    if tick <= RUN_END {
        "run"
    } else if tick <= STOP_END {
        "stop"
    } else if tick <= TURN_END {
        "turn"
    } else if tick == JUMP_TICK {
        "jump"
    } else if tick <= STRAFE_END {
        "strafe"
    } else if airborne {
        "air"
    } else {
        "land"
    }
}

/// One tick side by side: planar speed, height above the floor and vertical
/// speed, meters and meters per second (the oracle's HL units converted by
/// `Params::unit_m`).
#[derive(Clone, Copy, Debug)]
struct Sample {
    /// Tick index in the tape.
    tick: usize,
    /// Phase name.
    phase: &'static str,
    /// Oracle planar speed, m/s.
    hl_speed: f64,
    /// Our planar speed, m/s.
    our_speed: f64,
    /// Oracle height above the floor, m.
    hl_height: f64,
    /// Our feet above the floor, m.
    our_height: f64,
    /// Oracle vertical speed, m/s.
    hl_rise: f64,
    /// Our vertical speed, m/s.
    our_rise: f64,
}

/// Run the tape through both models and return one sample per tick.
fn tape(params: Params) -> Vec<Sample> {
    let dt = f64::from(FIXED_DT);
    let mut hl = HlMove::new(params);
    let mut ours = Controller::spawn([0.0, HALF, 0.0], RADIUS, HEIGHT).expect("the capsule spawns");
    ours.add_static_box([10.0, 0.5, 10.0], [0.0, -0.5, 0.0])
        .expect("the floor box");
    ours.move_and_slide([0.0, 0.0], FIXED_DT);
    assert!(ours.grounded(), "the tape starts on the floor");

    let takeoff = params.meters(params.jump_speed_u_s);
    let mut samples = Vec::with_capacity(TICKS);
    for tick in 0..TICKS {
        let (dir, jump) = input(tick);
        hl.tick(dir, jump, dt);
        if jump {
            // The published take-off speed, in meters for our step.
            ours.jump(f64_to_f32(takeoff));
        }
        // The same stick as this tick's requested displacement.
        let step = params.meters(params.maxspeed_u_s) * dt;
        let wish = [f64_to_f32(dir[0] * step), f64_to_f32(dir[1] * step)];
        let resolved = ours.move_and_slide(wish, FIXED_DT);
        let our_speed = f64::from(
            (resolved.displacement[0] * resolved.displacement[0]
                + resolved.displacement[1] * resolved.displacement[1])
                .sqrt(),
        ) / dt;
        let our_height = f64::from(ours.position()[1] - HALF);
        samples.push(Sample {
            tick,
            phase: phase(tick, our_height > 0.0),
            hl_speed: params.meters(hl.speed_u_s()),
            our_speed,
            hl_height: params.meters(hl.height_u()),
            our_height,
            hl_rise: params.meters(hl.rise_u_s()),
            our_rise: f64::from(ours.velocity_y()),
        });
    }
    samples
}

/// The fixture's f64 tape input to the controller's f32 step. Both sides sit
/// far inside f32's range (one tick of the cap is 65 mm), so the rounding is
/// the only loss.
fn f64_to_f32(v: f64) -> f32 {
    v as f32
}

/// Print the table and the three band points: the runner reads this under
/// `--nocapture`.
fn print_report(params: &Params, samples: &[Sample]) {
    let dt = f64::from(FIXED_DT);
    println!(
        "HL_TAPE ticks={TICKS} dt={dt:.6}s unit_m={:.7}m oracle: maxspeed={:.3}u/s \
         ({:.4}m/s) jump={:.3}u/s ({:.4}m/s) apex={:.2}u ({:.4}m) gravity={:.3}u/s2 \
         (our GRAVITY_Y={GRAVITY_Y})",
        params.unit_m,
        params.maxspeed_u_s,
        params.meters(params.maxspeed_u_s),
        params.jump_speed_u_s,
        params.meters(params.jump_speed_u_s),
        params.jump_apex_u(),
        params.meters(params.jump_apex_u()),
        params.gravity_u_s2,
    );
    println!(
        "{:>4} {:<7} {:>8} {:>8} {:>8} {:>8} {:>8} {:>8} {:>8} {:>8}",
        "tick", "phase", "hl_v", "our_v", "d_v", "hl_y", "our_y", "d_y", "hl_vy", "our_vy"
    );
    for s in samples {
        println!(
            "{:>4} {:<7} {:>8.4} {:>8.4} {:>8.4} {:>8.4} {:>8.4} {:>8.4} {:>8.4} {:>8.4}",
            s.tick,
            s.phase,
            s.hl_speed,
            s.our_speed,
            s.our_speed - s.hl_speed,
            s.hl_height,
            s.our_height,
            s.our_height - s.hl_height,
            s.hl_rise,
            s.our_rise
        );
    }
    let run = samples[RUN_END];
    let stop = samples[STOP_END];
    let hl_apex = samples[JUMP_TICK..]
        .iter()
        .fold(0.0f64, |m, s| m.max(s.hl_height));
    let our_apex = samples[JUMP_TICK..]
        .iter()
        .fold(0.0f64, |m, s| m.max(s.our_height));
    println!(
        "BAND run@{} hl_v={:.4} our_v={:.4} d={:.4} | stop@{} hl_v={:.4} our_v={:.4} \
         d={:.4} | apex hl={:.4} our={:.4} d={:.4} | strafe@{} hl_v={:.4} our_v={:.4} d={:.4}",
        run.tick,
        run.hl_speed,
        run.our_speed,
        run.our_speed - run.hl_speed,
        stop.tick,
        stop.hl_speed,
        stop.our_speed,
        stop.our_speed - stop.hl_speed,
        hl_apex,
        our_apex,
        our_apex - hl_apex,
        STRAFE_END,
        samples[STRAFE_END].hl_speed,
        samples[STRAFE_END].our_speed,
        samples[STRAFE_END].our_speed - samples[STRAFE_END].hl_speed
    );
}

#[test]
fn tape_prints_both_curves_and_holds_the_band() {
    let params = Params::hl1();
    let samples = tape(params);
    print_report(&params, &samples);
    let dt = f64::from(FIXED_DT);

    assert_eq!(samples.len(), TICKS, "one sample per tick");

    // 1. Ground speed after the accel ramp: the oracle is at its cap, and our
    // stepped speed is inside the band of it.
    let run = samples[RUN_END];
    let cap = params.meters(params.maxspeed_u_s);
    assert!(
        run.hl_speed > 0.99 * cap,
        "the oracle never reached its cap: {:.4} of {cap:.4} m/s",
        run.hl_speed
    );
    let d_run = (run.our_speed - run.hl_speed).abs();
    assert!(
        d_run <= GROUND_SPEED_TOL_M_S,
        "ground speed after the ramp: ours {:.4} m/s, oracle {:.4} m/s, gap {d_run:.4} m/s \
         over the {GROUND_SPEED_TOL_M_S} band",
        run.our_speed,
        run.hl_speed
    );

    // 2. Stopped speed after friction: the model has bled to its floor and we
    // stand still.
    let stop = samples[STOP_END];
    assert!(
        stop.hl_speed < STOPPED_SPEED_TOL_M_S,
        "the oracle still rolls at {:.4} m/s on the stop tick",
        stop.hl_speed
    );
    let d_stop = (stop.our_speed - stop.hl_speed).abs();
    assert!(
        d_stop <= STOPPED_SPEED_TOL_M_S,
        "stopped speed: ours {:.4} m/s, oracle {:.4} m/s, gap {d_stop:.4} m/s over the \
         {STOPPED_SPEED_TOL_M_S} band",
        stop.our_speed,
        stop.hl_speed
    );

    // 3. Jump apex height: the peak of the flight for both.
    let hl_apex = samples[JUMP_TICK..]
        .iter()
        .fold(0.0f64, |m, s| m.max(s.hl_height));
    let our_apex = samples[JUMP_TICK..]
        .iter()
        .fold(0.0f64, |m, s| m.max(s.our_height));
    assert!(
        hl_apex > params.meters(params.jump_apex_u()) * 0.95,
        "the oracle's flight is too short to be a jump: {hl_apex:.4} m"
    );
    let d_apex = (our_apex - hl_apex).abs();
    assert!(
        d_apex <= APEX_TOL_M,
        "jump apex: ours {our_apex:.4} m, oracle {hl_apex:.4} m, gap {d_apex:.4} m over the \
         {APEX_TOL_M} band"
    );
    // The gap is not slack: it is the half-step of a semi-implicit step, so it
    // must equal `v0 * dt / 2` to a millimetre. Anything else is a real drift.
    let bias = params.meters(params.jump_speed_u_s) * dt * 0.5;
    assert!(
        (hl_apex - our_apex - bias).abs() <= 0.002,
        "the apex gap {:.4} m is not the {bias:.4} m half-step of our step",
        hl_apex - our_apex
    );

    // The tape really left the floor and came back: the three points above are
    // not standing still on the ground.
    let airborne = samples[JUMP_TICK..]
        .iter()
        .filter(|s| s.our_height > 0.0)
        .count();
    assert!(airborne >= 38, "only {airborne} airborne ticks in the tape");
    assert!(
        samples[TICKS - 1].our_height <= 1e-6,
        "the tape must land before it ends, ends at {:.4} m",
        samples[TICKS - 1].our_height
    );

    // The air column is printed, not asserted: our air channel is a
    // displacement scale and HL's is an acceleration, so fast strafing
    // diverges by design. Post it so no reader takes the band for the air.
    let strafe = samples[STRAFE_END];
    println!(
        "AIR_GAP strafe@{} hl_v={:.4} our_v={:.4} ratio={:.2} (not asserted: different air laws)",
        strafe.tick,
        strafe.hl_speed,
        strafe.our_speed,
        strafe.our_speed / strafe.hl_speed
    );
}

#[test]
fn oracle_pins_the_published_numbers() {
    let p = Params::hl1();
    // PM_Jump: sqrt(2 * 800 * 45.0) units/s, an apex of 45 units.
    assert!(
        (p.jump_speed_u_s - 268.328).abs() < 0.001,
        "jump take-off {:.6} u/s",
        p.jump_speed_u_s
    );
    assert!(
        (p.jump_apex_u() - 45.0).abs() < 1.0e-9,
        "apex {:.9} units",
        p.jump_apex_u()
    );
    // PM_PreventMegaBunnyJumping: the cap is 1.7 * maxspeed.
    assert!(
        (p.bunnyhop_cap_u_s() - 1.7 * p.maxspeed_u_s).abs() < 1.0e-9,
        "crop cap {:.3} u/s",
        p.bunnyhop_cap_u_s()
    );
    // The fixture's unit scale holds acceleration equal, so a tick of the
    // oracle and a tick of our step compare as the same physical step.
    assert!(
        (p.unit_m * p.gravity_u_s2 - -f64::from(GRAVITY_Y)).abs() < 1.0e-9,
        "the fixture's gravity {:.9} m/s2 is not ours",
        p.unit_m * p.gravity_u_s2
    );
    // The adoptable numbers, at our gravity: a run cap and a jump apex.
    assert!(
        (p.meters(p.maxspeed_u_s) - 3.924).abs() < 0.001,
        "run cap {:.4} m/s",
        p.meters(p.maxspeed_u_s)
    );
    assert!(
        (p.meters(p.jump_apex_u()) - 0.5518).abs() < 0.001,
        "apex {:.4} m",
        p.meters(p.jump_apex_u())
    );
}
