//! phase_warp tests: the impact frame meets the sim's impact tick, the windup
//! stretch stops at 2x, and the map never runs backwards. Each test goes red
//! without the warp: a 1:1 clip reaches its impact frame at its own tick only.

use crate::events::{IMPACT_MARKER_CODE, RECOVER_MARKER_CODE, WINDUP_MARKER_CODE};
use crate::graph::TimedEvent;
use crate::phase_warp::{ClipMarkers, PhaseWarp, PhaseWarpError, MAX_STRETCH};

/// A dagger-speed clip: 4 clip ticks of windup, impact at 12, recover at 24.
const MARKERS: ClipMarkers = ClipMarkers {
    windup_start: 4,
    impact: 12,
    recover: 24,
};

fn warp(ticks_to_impact: u32) -> PhaseWarp {
    PhaseWarp::new(MARKERS, ticks_to_impact)
}

fn cues(pairs: &[(u32, u32)]) -> Vec<TimedEvent> {
    pairs
        .iter()
        .map(|&(tick, code)| TimedEvent { tick, code })
        .collect()
}

#[test]
fn the_impact_frame_meets_the_sim_impact_tick_for_every_lead() {
    // The shipped startups: dagger 12, gladius 15, spatha 18, club 21, pilum
    // 24 ticks; the clip's impact frame sits 12 clip ticks in.
    for lead in [0_u32, 4, 8, 12, 15, 18, 21, 24, 40] {
        let phase = warp(lead);
        assert_eq!(
            phase.clip_time(f32::from(u16::try_from(lead).unwrap())),
            12.0,
            "lead {lead}: the impact frame must sit on the hit tick"
        );
    }
}

#[test]
fn the_impact_frame_is_never_drawn_early_and_never_late() {
    for lead in [2_u32, 6, 12, 20, 33] {
        let phase = warp(lead);
        let at_lead = f32::from(u16::try_from(lead).unwrap());
        assert!(phase.clip_time(at_lead) == 12.0, "lead {lead}: lands late");
        assert!(
            phase.clip_time(at_lead - 0.25) < 12.0,
            "lead {lead}: impact frame drawn before the hit"
        );
        assert!(
            phase.clip_time(at_lead + 0.25) > 12.0,
            "lead {lead}: clip stalls after the hit"
        );
    }
}

#[test]
fn a_long_lead_holds_the_peak_at_the_two_x_cap() {
    // 4 clip ticks of windup over 24 sim ticks would ask for 6x; the cap
    // plays it at 2x and holds the last windup frame (impact - 1).
    let phase = warp(24);
    assert_eq!(phase.stretch(), MAX_STRETCH);
    let mut last = phase.clip_time(0.0);
    assert_eq!(last, 4.0, "the windup starts on its own marker");
    for step in 1..=95 {
        let since = f32::from(u16::try_from(step).unwrap()) / 4.0;
        let now = phase.clip_time(since);
        assert!(now >= last, "the map runs backwards at {since}");
        assert!(
            now - last <= 0.5 + 1e-5,
            "stretched past 2x between {since} and one quarter tick later"
        );
        assert!(
            now <= 11.0 + 1e-5,
            "the impact pose is drawn at {since}, before the hit"
        );
        last = now;
    }
    // The hold releases on the hit tick alone: one clip tick, then 1:1.
    assert_eq!(phase.clip_time(23.75), 11.0);
    assert_eq!(phase.clip_time(24.0), 12.0);
    assert_eq!(phase.clip_time(24.25), 12.25);
}

#[test]
fn a_two_x_lead_keeps_the_authored_even_spread() {
    // Lead exactly twice the windup: the cap is met, not exceeded, so the
    // windup spreads evenly over the whole lead with no hold.
    let phase = warp(16);
    assert_eq!(phase.stretch(), 2.0);
    for step in 0..=16 {
        let since = f32::from(u16::try_from(step).unwrap());
        assert_eq!(phase.clip_time(since), 4.0 + since * 0.5);
    }
}

#[test]
fn time_is_monotonic_across_the_impact_and_the_whole_clip() {
    for lead in [0_u32, 3, 12, 24, 60] {
        let phase = warp(lead);
        let lead_f = f32::from(u16::try_from(lead).unwrap());
        let mut last = phase.clip_time(0.0);
        let mut step = 0;
        while step <= 160 {
            let since = f32::from(u16::try_from(step).unwrap()) / 4.0;
            let now = phase.clip_time(since);
            assert!(now >= last, "lead {lead}: backwards at {since}");
            assert!(now - last <= 1.0, "lead {lead}: jump at {since}");
            // Never ahead of 1:1 after the hit, never past the windup end
            // before it.
            assert!(
                now <= 12.0 + since + 1e-5,
                "lead {lead}: ahead of the clip at {since}"
            );
            if since < lead_f {
                assert!(now <= 12.0, "lead {lead}: past the windup at {since}");
            }
            last = now;
            step += 1;
        }
    }
}

#[test]
fn a_zero_length_windup_is_safe() {
    // Impact on the windup-start marker: nothing to warp, no divide by zero.
    let markers = ClipMarkers::new(5, 5, 20).expect("ordered markers");
    let phase = PhaseWarp::new(markers, 10);
    assert_eq!(phase.markers().windup_ticks(), 0);
    assert_eq!(phase.stretch(), 1.0);
    assert_eq!(phase.clip_time(0.0), 5.0);
    assert_eq!(phase.clip_time(9.0), 5.0);
    assert_eq!(phase.clip_time(10.0), 5.0);
    assert_eq!(phase.clip_time(13.0), 8.0);
    // A lead of zero has no windup either: the clip starts on the hit.
    let now = PhaseWarp::new(markers, 0);
    assert_eq!(now.clip_time(0.0), 5.0);
    assert_eq!(now.clip_time(2.0), 7.0);
    assert_eq!(now.telegraph(0.0), 1.0);
}

#[test]
fn a_short_lead_squeezes_the_windup_to_the_hit() {
    // 12 clip ticks of windup over a 6 tick lead: twice the authored rate, no
    // lower cap, because the hit tick is the truth.
    let markers = ClipMarkers::new(0, 12, 30).expect("ordered markers");
    let phase = PhaseWarp::new(markers, 6);
    assert!((phase.stretch() - 0.5).abs() < 1e-6);
    assert_eq!(phase.clip_time(0.0), 0.0);
    assert_eq!(phase.clip_time(3.0), 6.0);
    assert_eq!(phase.clip_time(6.0), 12.0);
    assert_eq!(phase.clip_time(9.0), 15.0);
}

#[test]
fn the_telegraph_meter_runs_zero_to_one_over_the_windup() {
    let phase = warp(15);
    assert_eq!(phase.telegraph(0.0), 0.0);
    assert_eq!(phase.telegraph(5.0), 5.0 / 15.0);
    assert_eq!(phase.telegraph(15.0), 1.0);
    assert_eq!(phase.telegraph(20.0), 1.0, "the meter saturates at the hit");
    assert_eq!(phase.telegraph(f32::NAN), 0.0);
    assert_eq!(phase.telegraph(-2.0), 0.0);
    // No lead: nothing to telegraph.
    assert_eq!(warp(0).telegraph(0.0), 1.0);
}

#[test]
fn markers_come_off_the_event_track_in_any_cue_order() {
    let track = cues(&[
        (24, RECOVER_MARKER_CODE),
        (12, IMPACT_MARKER_CODE),
        (3, 1),
        (4, WINDUP_MARKER_CODE),
    ]);
    let markers = ClipMarkers::from_cues(&track).expect("three markers");
    assert_eq!(markers, ClipMarkers::new(4, 12, 24).expect("ordered"));
}

#[test]
fn a_broken_track_is_refused_by_name() {
    let missing = cues(&[(4, WINDUP_MARKER_CODE), (12, IMPACT_MARKER_CODE)]);
    assert_eq!(
        ClipMarkers::from_cues(&missing),
        Err(PhaseWarpError::MissingMarker {
            code: RECOVER_MARKER_CODE
        })
    );
    let twice = cues(&[
        (4, WINDUP_MARKER_CODE),
        (5, WINDUP_MARKER_CODE),
        (12, IMPACT_MARKER_CODE),
        (24, RECOVER_MARKER_CODE),
    ]);
    assert_eq!(
        ClipMarkers::from_cues(&twice),
        Err(PhaseWarpError::DuplicateMarker {
            code: WINDUP_MARKER_CODE
        })
    );
    assert_eq!(
        ClipMarkers::new(12, 4, 24),
        Err(PhaseWarpError::Order {
            windup_start: 12,
            impact: 4,
            recover: 24,
        })
    );
    assert_eq!(
        ClipMarkers::new(0, 30, 20),
        Err(PhaseWarpError::Order {
            windup_start: 0,
            impact: 30,
            recover: 20,
        })
    );
}

#[test]
fn non_finite_time_reads_as_the_press_tick() {
    let phase = warp(12);
    assert_eq!(phase.clip_time(f32::NAN), 4.0);
    assert_eq!(phase.clip_time(f32::INFINITY), 4.0);
    assert_eq!(phase.clip_time(-5.0), 4.0);
    // Past the hit the clip keeps its own rate, however far the look asks.
    assert_eq!(phase.clip_time(112.0), 112.0);
}
