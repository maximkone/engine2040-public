//! capture_point tests: the mirror law, the hardness law and the edge cases
//! the ARX01 acceptance names, plus the pendulum time and the allocation bill.

use super::*;
use crate::balance::{BalanceError, GaitVerdict, MAX_IMPULSE_Q8, SUPPORT_HALF_MM};
use engine_core::alloc_probe;

/// Standing fighter for every law below: the arena stance rig's center of
/// mass (525 mm up) over feet centred on the origin.
const STANDING_COM_MM: [i32; 3] = [0, 525, 0];

/// Pendulum time of the standing rig, ticks at 60 Hz: `sqrt(0.525 m / g)` is
/// 0.231 s, which floors to 13 ticks. Hand-computed so the lead assertions
/// below read against a fixed number, not against the function itself.
const STANDING_TICKS: u32 = 13;

fn stance() -> SupportRegion {
    SupportRegion::stance()
}

#[test]
fn capture_point_leads_by_one_pendulum_time() {
    // Pendulum time is the floor of sqrt(h / g) in ticks, no float.
    assert_eq!(pendulum_ticks(0), 0, "a body on the ground has no pendulum");
    assert_eq!(pendulum_ticks(-40), 0, "a height under zero reads none");
    assert_eq!(pendulum_ticks(STANDING_COM_MM[1]), STANDING_TICKS);
    // 1 m reads 19 ticks (0.319 s) and 1.75 m reads 25 (0.417 s).
    assert_eq!(pendulum_ticks(1000), 19);
    assert_eq!(pendulum_ticks(1750), 25);
    // Non-decreasing in height: a taller body takes longer to tip.
    let mut previous = 0;
    for height in [0, 1, 250, 525, 1000, 1750, 3000, 4000, i32::MAX] {
        let ticks = pendulum_ticks(height);
        assert!(ticks >= previous, "height {height} reads {ticks}");
        previous = ticks;
    }
    assert!(previous > 600, "the height ceiling still reads a time");
    // A standing body carries its velocity one pendulum time: 60 mm/tick over
    // 13 ticks leads 780 mm along +x, and the z lane is untouched.
    let cp = capture_point_mm(STANDING_COM_MM, [60, 0]);
    assert_eq!(cp, [60 * i32::try_from(STANDING_TICKS).expect("13"), 0]);
    assert_eq!(cp, [780, 0]);
    // The lead saturates instead of wrapping on a wild caller.
    assert_eq!(
        capture_point_mm([0, 1750, 0], [i32::MAX, i32::MIN]),
        [i32::MAX, i32::MIN]
    );
}

#[test]
fn zero_velocity_standing_still_holds() {
    let choice = capture_choice(STANDING_COM_MM, stance(), [0, 0]);
    assert_eq!(choice.verdict, CaptureVerdict::Hold);
    assert_eq!(choice.cp_mm, [0, 0], "at rest the capture point is the com");
    assert_eq!(choice.slack_mm, SUPPORT_HALF_MM, "the whole box is slack");
    assert_eq!(choice.pendulum_ticks, STANDING_TICKS);
    // A blow too light to move the capture point past the hold margin holds.
    let light = strike_choice(STANDING_COM_MM, stance(), 16, [1024, 0]).expect("light blow");
    assert_eq!(light.verdict, CaptureVerdict::Hold, "{light:?}");
}

#[test]
fn step_sits_between_hold_and_fall() {
    // 1 m tall, 19 ticks: 4 mm/tick leads 76 mm (74 mm slack) and holds,
    // 7 mm/tick leads 133 mm (17 mm slack) and steps, 10 mm/tick leads
    // 190 mm (40 mm past the edge) and falls.
    let tall = [0, 1000, 0];
    assert_eq!(
        capture_choice(tall, stance(), [4, 0]).verdict,
        CaptureVerdict::Hold
    );
    let stepping = capture_choice(tall, stance(), [7, 0]);
    assert_eq!(stepping.slack_mm, 17);
    assert_eq!(stepping.verdict, CaptureVerdict::Step, "{stepping:?}");
    let falling = capture_choice(tall, stance(), [10, 0]);
    assert_eq!(falling.slack_mm, -40);
    assert_eq!(falling.verdict, CaptureVerdict::Fall, "{falling:?}");
    // The two boundaries read exactly: a capture point on the hold margin and
    // one millimetre nearer the edge still hold, one millimetre in from the
    // margin steps, the edge itself falls and a millimetre inside it steps.
    let margin = STANCE_HALF_MM - STEP_MARGIN_MM;
    let on_margin = capture_choice([margin, 1000, 0], stance(), [0, 0]);
    assert_eq!(on_margin.slack_mm, STEP_MARGIN_MM);
    assert_eq!(on_margin.verdict, CaptureVerdict::Hold);
    let just_in = capture_choice([margin + 1, 1000, 0], stance(), [0, 0]);
    assert_eq!(just_in.verdict, CaptureVerdict::Step);
    let on_edge = capture_choice([STANCE_HALF_MM, 1000, 0], stance(), [0, 0]);
    assert_eq!(on_edge.slack_mm, 0);
    assert_eq!(
        on_edge.verdict,
        CaptureVerdict::Fall,
        "the edge itself falls"
    );
    let just_inside = capture_choice([STANCE_HALF_MM - 1, 1000, 0], stance(), [0, 0]);
    assert_eq!(just_inside.verdict, CaptureVerdict::Step);
    assert_eq!(STANCE_HALF_MM, SUPPORT_HALF_MM, "one owner for the width");
    assert_eq!(
        SupportRegion::square([0, 0], STANCE_HALF_MM),
        SupportRegion::stance()
    );
}

#[test]
fn mirrored_blow_mirrors_the_choice() {
    // Mirroring a blow swaps left and right: com x, support centre x and the
    // velocity's x lane flip sign, and the z lanes stay. The choice must read
    // the same slack and the same reaction on both sides.
    let support = SupportRegion::new([30, -20], [150, 120]);
    let mirrored = SupportRegion::new([-30, -20], [150, 120]);
    let com = [70, 700, -40];
    let mirrored_com = [-70, 700, -40];
    for impulse in [0u16, 16, 128, 384, 640, 1024, 2048, MAX_IMPULSE_Q8] {
        for dir in [
            [1024, 0],
            [-1024, 0],
            [724, 724],
            [-724, 724],
            [724, -724],
            [-724, -724],
        ] {
            let plain = strike_choice(com, support, impulse, dir).expect("blow");
            let turned =
                strike_choice(mirrored_com, mirrored, impulse, [-dir[0], dir[1]]).expect("blow");
            assert_eq!(
                turned.cp_mm,
                [-plain.cp_mm[0], plain.cp_mm[1]],
                "impulse {impulse} dir {dir:?}: the capture point mirrors"
            );
            assert_eq!(
                turned.slack_mm, plain.slack_mm,
                "impulse {impulse} dir {dir:?}"
            );
            assert_eq!(
                turned.verdict, plain.verdict,
                "impulse {impulse} dir {dir:?} must mirror: {plain:?} vs {turned:?}"
            );
            assert_eq!(turned.pendulum_ticks, plain.pendulum_ticks);
        }
    }
    // A pure axis blow mirrored is the opposite axis blow on the other side.
    let left = capture_choice(com, support, [40, 0]);
    let right = capture_choice(mirrored_com, mirrored, [-40, 0]);
    assert_eq!(right.cp_mm, [-left.cp_mm[0], left.cp_mm[1]]);
    assert_eq!(right.verdict, left.verdict);
}

#[test]
fn harder_blow_never_picks_a_milder_reaction() {
    // One direction, growing impulse: the capture point only travels farther,
    // so the slack only shrinks and the rung only climbs hold < step < fall.
    let support = stance();
    for dir in [[1024, 0], [724, 724], [0, 1024]] {
        let mut previous = CaptureVerdict::Hold;
        let mut previous_slack = i32::MAX;
        for impulse in [
            0u16,
            8,
            16,
            32,
            64,
            96,
            128,
            192,
            256,
            384,
            512,
            640,
            1024,
            2048,
            MAX_IMPULSE_Q8,
        ] {
            let choice = strike_choice(STANDING_COM_MM, support, impulse, dir).expect("blow");
            assert!(
                choice.verdict >= previous,
                "dir {dir:?} impulse {impulse} dropped to {:?} from {previous:?}",
                choice.verdict
            );
            assert!(
                choice.slack_mm <= previous_slack,
                "dir {dir:?} impulse {impulse} slack rose to {}",
                choice.slack_mm
            );
            previous = choice.verdict;
            previous_slack = choice.slack_mm;
        }
        assert_eq!(
            previous,
            CaptureVerdict::Fall,
            "the ceiling falls, dir {dir:?}"
        );
    }
    // The sweep walks every rung at the standing height: hold, then step,
    // then fall, in that order and no other. 64 Q8 carries 10 mm/tick, 130 mm
    // over 13 ticks, which leaves 20 mm of slack: on the toes, still inside.
    let rungs: Vec<CaptureVerdict> = [0u16, 64, 384, MAX_IMPULSE_Q8]
        .iter()
        .map(|impulse| {
            strike_choice(STANDING_COM_MM, stance(), *impulse, [1024, 0])
                .expect("blow")
                .verdict
        })
        .collect();
    assert_eq!(
        rungs,
        vec![
            CaptureVerdict::Hold,
            CaptureVerdict::Step,
            CaptureVerdict::Fall,
            CaptureVerdict::Fall
        ]
    );
}

#[test]
fn huge_blow_falls_and_a_down_body_holds() {
    let falling =
        strike_choice(STANDING_COM_MM, stance(), MAX_IMPULSE_Q8, [1024, 0]).expect("ceiling blow");
    assert_eq!(falling.verdict, CaptureVerdict::Fall);
    // 4096 Q8 caps at 240 mm/tick, 13 ticks lead 3120 mm past a 150 mm box.
    assert_eq!(
        falling.cp_mm,
        [240 * i32::try_from(STANDING_TICKS).expect("13"), 0]
    );
    println!(
        "ARX01 receipt: impulse={MAX_IMPULSE_Q8} cp={:?} ticks={} slack={} verdict={:?}",
        falling.cp_mm, falling.pendulum_ticks, falling.slack_mm, falling.verdict
    );
    // A body with no standing height (on the ground, com y = 0) has no
    // pendulum lead, so the same blow over a support box holds: a downed
    // fighter never re-falls from its own velocity.
    let downed =
        strike_choice([0, 0, 0], stance(), MAX_IMPULSE_Q8, [1024, 0]).expect("ceiling blow");
    assert_eq!(downed.pendulum_ticks, 0);
    assert_eq!(downed.cp_mm, [0, 0]);
    assert_eq!(downed.verdict, CaptureVerdict::Hold);
    // A body whose momentum already left the box falls even at rest.
    let leaning = capture_choice([400, 525, 0], stance(), [0, 0]);
    assert_eq!(leaning.verdict, CaptureVerdict::Fall);
}

#[test]
fn over_ceiling_impulse_is_refused() {
    assert_eq!(
        strike_choice(STANDING_COM_MM, stance(), MAX_IMPULSE_Q8 + 1, [1024, 0]).unwrap_err(),
        BalanceError::BadImpulse(MAX_IMPULSE_Q8 + 1)
    );
    // The ceiling itself is accepted.
    strike_choice(STANDING_COM_MM, stance(), MAX_IMPULSE_Q8, [1024, 0]).expect("ceiling blow");
}

#[test]
fn verdict_reads_the_gait_ladder() {
    assert_eq!(CaptureVerdict::Hold.gait(), GaitVerdict::Staggered);
    assert_eq!(CaptureVerdict::Step.gait(), GaitVerdict::Stumbling);
    assert_eq!(CaptureVerdict::Fall.gait(), GaitVerdict::Fallen);
    assert_eq!(
        [
            CaptureVerdict::Hold.code(),
            CaptureVerdict::Step.code(),
            CaptureVerdict::Fall.code()
        ],
        [0, 1, 2]
    );
    assert!(CaptureVerdict::Hold < CaptureVerdict::Step);
    assert!(CaptureVerdict::Step < CaptureVerdict::Fall);
}

#[test]
fn decision_replays_and_allocates_nothing() {
    let support = stance();
    let (first, allocs) = alloc_probe::measure(|| {
        let mut last = CaptureVerdict::Hold;
        for impulse in [0u16, 128, 256, 384, 640, MAX_IMPULSE_Q8] {
            for dir in [[1024, 0], [724, 724], [0, -1024]] {
                last = strike_choice(STANDING_COM_MM, support, impulse, dir)
                    .expect("blow")
                    .verdict;
            }
        }
        last
    });
    assert_eq!(first, CaptureVerdict::Fall);
    assert_eq!(
        allocs.calls, 0,
        "the decision must not allocate: {allocs:?}"
    );
    // Same inputs, same choice: the path holds no clock and no state.
    for impulse in [0u16, 384, MAX_IMPULSE_Q8] {
        let a = strike_choice(STANDING_COM_MM, support, impulse, [724, 724]).expect("a");
        let b = strike_choice(STANDING_COM_MM, support, impulse, [724, 724]).expect("b");
        assert_eq!(a, b, "impulse {impulse} replays");
    }
}
