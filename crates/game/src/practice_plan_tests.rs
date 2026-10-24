//! practice_plan tests: the dummy's four drill promises (SW80).

use super::{
    Distance, Dummy, Guard, PracticePlan, Seen, Stance, AUTO_GUARD_TICKS, CLOSE_PX_PER_TICK,
};
use engine_input::{
    fighter::{BLOCK, JUMP},
    Buttons, FighterId,
};

fn slot() -> FighterId {
    FighterId::new(2).expect("slot 2 exists")
}

fn seen(gap_px: i32, grounded: bool, incoming: bool) -> Seen {
    Seen {
        fighter: slot(),
        gap_px,
        facing: 0,
        grounded,
        incoming,
        floored: false,
    }
}

fn blocks(input: &engine_input::FighterInput) -> bool {
    input.buttons.0 & BLOCK != 0
}

fn jumps(input: &engine_input::FighterInput) -> bool {
    input.buttons.0 & JUMP != 0
}

/// 40 attacks, one every 6 ticks: a random-guard dummy blocks 18-22 of them (donor 50%).
#[test]
fn a_random_guard_dummy_blocks_about_half_of_forty_attacks() {
    let mut plan = PracticePlan::new(
        Guard::Random,
        Dummy::Stand,
        Distance::Medium,
        false,
        false,
        2040,
    );
    let mut blocked = 0;
    for tick in 0..240u32 {
        // The attack is in flight for one tick every six.
        let incoming = tick % 6 == 0;
        let input = plan.input(tick, seen(80, true, incoming));
        if incoming && blocks(&input) {
            blocked += 1;
        }
    }
    println!("practice_plan: random guard blocked {blocked} of 40 attacks (seed 2040)");
    assert!(
        (18..=22).contains(&blocked),
        "a 50% guard blocks 18-22 of 40, got {blocked}"
    );
}

/// Close mode: a mover that reads the plan's step holds the 10 px band after 20 ticks.
#[test]
fn close_mode_holds_the_body_distance_at_ten_px() {
    let plan = PracticePlan::new(
        Guard::None,
        Dummy::Stand,
        Distance::Close,
        false,
        false,
        2040,
    );
    let mut gap = 60;
    for _ in 0..20 {
        gap -= plan.spacing_step_px(gap);
    }
    println!("practice_plan: close band gap after 20 ticks {gap} px");
    assert_eq!(plan.distance().band_px(), 10);
    assert!(
        (gap - 10).abs() <= 1,
        "close mode lands on the 10 px band, got {gap}"
    );
    // It was moving, not teleporting: the step is the plan's own bound.
    assert!(plan.spacing_step_px(60).abs() <= CLOSE_PX_PER_TICK);
}

/// Jump mode: the dummy asks to jump on its first grounded tick and leaves the ground.
#[test]
fn a_jump_mode_dummy_leaves_the_ground() {
    let mut plan = PracticePlan::new(
        Guard::None,
        Dummy::Jump,
        Distance::Medium,
        false,
        false,
        2040,
    );
    let first = plan.input(0, seen(80, true, false));
    assert!(jumps(&first), "the grounded dummy asks for the jump");
    // The caller's own arc (300 px/s up, 900 px/s^2 down at 60 Hz) leaves the sand.
    let mut height_px = 0.0f32;
    let mut speed_px_s = 300.0f32;
    let mut airborne = false;
    for _ in 0..20 {
        if !airborne && jumps(&plan.input(1, seen(80, true, false))) {
            airborne = true;
        }
        speed_px_s -= 900.0 / 60.0;
        height_px += speed_px_s / 60.0;
    }
    println!("practice_plan: jump arc height after 20 ticks {height_px:.1} px");
    assert!(height_px > 1.0, "the dummy leaves the ground: {height_px}");
    // Airborne, it does not ask again.
    let up = plan.input(21, seen(80, false, false));
    assert!(!jumps(&up), "no jump is asked while airborne");
}

/// Auto guard holds the guard exactly AUTO_GUARD_TICKS ticks after an attack.
#[test]
fn auto_guard_holds_for_its_timer_then_releases() {
    let mut plan = PracticePlan::new(
        Guard::Auto,
        Dummy::Stand,
        Distance::Medium,
        false,
        false,
        2040,
    );
    let hit = plan.input(0, seen(80, true, true));
    assert!(blocks(&hit), "an incoming attack raises the guard");
    let mut held = 0;
    for tick in 1..=AUTO_GUARD_TICKS {
        if blocks(&plan.input(tick, seen(80, true, false))) {
            held += 1;
        }
    }
    assert!(
        held >= AUTO_GUARD_TICKS - 1,
        "the timer holds the guard for {AUTO_GUARD_TICKS} ticks, got {held}"
    );
    let down = plan.input(AUTO_GUARD_TICKS + 2, seen(80, true, false));
    assert!(!blocks(&down), "the guard drops when the timer runs out");
}

/// The same plan over the same seen ticks gives the same input bytes.
#[test]
fn the_same_plan_twice_is_byte_equal() {
    let run = || {
        let mut plan = PracticePlan::new(
            Guard::Random,
            Dummy::Crouch,
            Distance::Far,
            true,
            true,
            2040,
        );
        let mut bytes = Vec::new();
        for tick in 0..120u32 {
            let gap = 40 + i32::try_from(tick % 60).unwrap_or(0);
            let input = plan.input(tick, seen(gap, tick % 7 != 0, tick % 5 == 0));
            bytes.extend_from_slice(&input.tick.to_le_bytes());
            bytes.push(input.mx.to_le_bytes()[0]);
            bytes.push(input.mz.to_le_bytes()[0]);
            bytes.extend_from_slice(&input.facing.to_le_bytes());
            bytes.push(input.buttons.0);
        }
        bytes
    };
    assert_eq!(run(), run(), "the same plan gives the same inputs twice");
}

/// Crouch mode names its stance and blocks nothing with guard none.
#[test]
fn a_crouching_dummy_keeps_its_stance_and_a_still_guard() {
    let mut plan = PracticePlan::new(
        Guard::None,
        Dummy::Crouch,
        Distance::Medium,
        false,
        false,
        2040,
    );
    let input = plan.input(0, seen(80, true, true));
    assert_eq!(plan.stance(), Stance::Crouch);
    assert_eq!(input.buttons, Buttons::NONE);
    assert_eq!(plan.answered(), 1);
}
