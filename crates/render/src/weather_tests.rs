//! weather tests (SW142): the ring field's four bars - world-space stability
//! and gauge 0 zero, the 25 live cells of a 5x5 window, the 0.05 UV peak, and
//! the same tick twice giving the same bytes.
//!
//! Every assert is an exact value or a byte comparison, never "it did not
//! panic".

use super::{
    ground_wet, pass_cells, rain_streak_len, ring_cells, ripple_displacement, ripple_frame,
    ripple_peak_offset, tier_taps, unit_of_permille, Weather, WeatherState, HALF_RES_TAPS,
    POTATO_TAPS, RIPPLE_OFFSET, WET_ALBEDO_SCALE, WET_ROUGHNESS_DELTA,
};

/// F2P: at gauge 0 the field is exactly zero at any tick, and in rain the same
/// world point earns the same displacement whichever way it is reached.
#[test]
fn ripple_field_is_stable_in_world_space_and_zero_at_clear() {
    let state = WeatherState::new(Weather::Clear);
    assert_eq!(state.intensity(), 0.0);
    for tick in [0u64, 1, 37, 120] {
        for point in [[0.0f32, 0.0], [0.25, 0.75], [11.5, -3.25], [-40.0, 8.0]] {
            assert_eq!(
                ripple_displacement(point, tick, state.intensity()),
                [0.0, 0.0],
                "clear leaves the point untouched at tick {tick}"
            );
        }
    }
    let clear_frame = ripple_frame(8, 0.5, 3, state.intensity());
    assert!(
        clear_frame.iter().all(|byte| *byte == 0),
        "a clear frame is all zero"
    );

    // World space, not screen space: the drop pattern belongs to the ground, so
    // the same world point gives the same offset however it was sampled.
    let rain = Weather::Rain.intensity();
    let here = ripple_displacement([3.25, -1.75], 42, rain);
    assert_eq!(here, ripple_displacement([3.25, -1.75], 42, rain));
    assert_ne!(here, [0.0, 0.0], "rain moves the point");
    assert_ne!(
        here,
        ripple_displacement([3.25, 6.75], 42, rain),
        "a different world point sees a different drop pattern"
    );
    assert_eq!(
        Weather::Clear.permille(),
        0,
        "the gauge is permille, so a receipt prints a whole number"
    );
}

/// The 5x5 window lights all of its cells at rain, and the tier counts match
/// the row: 25 half-res taps, 7 on potato.
#[test]
fn rain_1000_lights_all_25_ring_cells_in_a_5x5_window() {
    assert_eq!(Weather::Rain.permille(), 1000);
    assert_eq!(unit_of_permille(1000), 1.0);
    assert_eq!(unit_of_permille(350), Weather::Gust.intensity());
    for tick in [0u64, 7, 61, 119] {
        assert_eq!(
            ring_cells(tick, 1.0),
            HALF_RES_TAPS,
            "a 5x5 window at rain carries 25 live rings at tick {tick}"
        );
    }
    assert_eq!(ring_cells(7, 0.0), 0, "clear carries no ring");
    assert_eq!(tier_taps(false), 25);
    assert_eq!(tier_taps(true), 7);
    assert_eq!(POTATO_TAPS, 7);
    assert_eq!(pass_cells(1920, 1080, false), (960, 540), "half res");
    assert_eq!(pass_cells(1920, 1080, true), (640, 360), "potato third");
}

/// A ring's peak displaces the UV by exactly 0.05 at half res, and no sampled
/// point ever exceeds the ring's own bound.
#[test]
fn a_ripple_peak_displaces_the_uv_by_005_at_half_res() {
    assert_eq!(ripple_peak_offset(1.0), 0.05);
    assert_eq!(RIPPLE_OFFSET, 0.05);
    assert_eq!(ripple_peak_offset(0.0), 0.0);
    assert_eq!(ripple_peak_offset(1.0) * 0.5, ripple_peak_offset(0.5));
    // Overlapping rings are clamped to the ring's own peak, so no sample ever
    // moves the UV past 0.05.
    let bound = RIPPLE_OFFSET;
    let mut peak = 0.0f32;
    for y in 0..16 {
        for x in 0..16 {
            let world = [
                f32::from(u16::try_from(x).expect("16 fits")) * 0.5,
                f32::from(u16::try_from(y).expect("16 fits")) * 0.5,
            ];
            let offset = ripple_displacement(world, 11, 1.0);
            peak = peak.max(offset[0].abs()).max(offset[1].abs());
        }
    }
    assert!(peak > 0.0, "rain displaces the frame, got {peak}");
    assert!(
        peak <= bound,
        "a sample never exceeds the ring's bound, got {peak} > {bound}"
    );
}

/// Gauge 0 is byte-equal to the frame as it was: no ring, no wetness, no
/// streak.
#[test]
fn gauge_zero_leaves_the_frame_byte_equal() {
    let zero = vec![0u8; 12 * 12 * 8];
    assert_eq!(
        ripple_frame(12, 0.5, 0, 0.0),
        zero,
        "tick 0 at clear is zero"
    );
    assert_eq!(
        ripple_frame(12, 0.5, 97, 0.0),
        zero,
        "any tick at clear is zero"
    );
    assert_eq!(
        ground_wet(0.8, 0.65, 0.0),
        (0.8, 0.65),
        "clear leaves the ground exactly as it was"
    );
    assert_eq!(rain_streak_len(9.0, 0.0), 0.0, "clear draws no streak");
}

/// The same tick seed and the same world patch give the same displacement
/// twice, and rain's wet numbers are the row's 0.75 albedo and -0.3 roughness.
#[test]
fn the_same_tick_seed_gives_the_same_displacement_twice() {
    let first = ripple_frame(16, 0.5, 33, 1.0);
    let again = ripple_frame(16, 0.5, 33, 1.0);
    assert!(first.iter().any(|byte| *byte != 0), "rain carries a field");
    assert_eq!(first, again, "the same tick x2 is byte-equal");
    assert_ne!(
        first,
        ripple_frame(16, 0.5, 34, 1.0),
        "the next tick is a different field"
    );
    let (albedo, roughness) = ground_wet(0.8, 0.65, 1.0);
    assert_eq!(albedo, 0.8f32 * WET_ALBEDO_SCALE);
    assert_eq!(roughness, 0.65f32 + WET_ROUGHNESS_DELTA);
    assert_eq!(
        rain_streak_len(9.0, 1.0),
        9.0 * 0.25,
        "the donor's stretch gain"
    );
    let mut state = WeatherState::new(Weather::Rain);
    state.step();
    state.step();
    assert_eq!(state.tick(), 2, "the state steps on ticks, not seconds");
    state.set(Weather::Gust);
    assert_eq!((state.tick(), state.weather()), (2, Weather::Gust));
}
