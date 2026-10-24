//! SW112 tests: the four early returns of a room, their wall gains and the
//! zero-allocation wet-bus sum.

use super::*;
use crate::occlude::Material;
use crate::reverb::level_db;

/// The row's wall: 20 m away, hard stone.
const WALL_CM: u32 = 2_000;
const SR: u32 = 48_000;

/// dB of one tap's Q16 gain, read through the reverb module's own meter.
fn tap_db(gain_q16: i32) -> f64 {
    f64::from(level_db(q16_to_gain(gain_q16)))
}

fn wall(material: Material) -> [Tap; TAP_COUNT] {
    taps(WALL_CM, material).expect("20 m is inside the band")
}

#[test]
fn twenty_metre_wall_returns_four_taps_at_39_43_47_52_ms() {
    let returns = wall(Material::Stone);
    let expect = [39.0, 43.0, 47.0, 52.0];
    for (tap, want) in returns.iter().zip(expect.iter()) {
        assert!(
            (tap.delay_ms() - want).abs() <= 0.5,
            "tap {} lands at {:.2} ms, want {want} ms",
            tap.delay_us,
            tap.delay_ms()
        );
    }
    // Arrival order is strict: no tap overtakes the one before it.
    assert!(
        returns
            .windows(2)
            .all(|pair| pair[0].delay_us < pair[1].delay_us),
        "taps arrive in order: {returns:?}"
    );
    assert_eq!(returns[0].delay_us, 38_838);
}

#[test]
fn sand_tap_is_lower_than_stone_tap() {
    let stone = wall(Material::Stone);
    let sand = wall(Material::Sand);
    for (index, (hard, soft)) in stone.iter().zip(sand.iter()).enumerate() {
        let hard_db = tap_db(hard.gain_q16);
        let soft_db = tap_db(soft.gain_q16);
        assert!(
            hard.gain_q16 > soft.gain_q16,
            "tap {index}: stone {hard_db:.2} dB must beat sand {soft_db:.2} dB"
        );
        let gap = hard_db - soft_db;
        assert!(
            (2.5..=3.5).contains(&gap),
            "tap {index}: stone sits {gap:.2} dB above sand, want about 3"
        );
    }
    // Both walls hand back less than they take: a reflection is a loss.
    for tap in stone.iter().chain(sand.iter()) {
        assert!(tap.gain_q16 < GAIN_ONE, "a reflection never exceeds unity");
        assert!(tap.gain() > 0.0, "a wall still returns some energy");
    }
}

#[test]
fn early_taps_land_well_before_the_late_tail_blooms() {
    // The tunnel preset's tail runs 1100 ms; the quarter point is the earliest
    // the late bloom can be read, so every early tap is far inside it.
    let tail_quarter_ms = crate::reverb::TUNNEL_RT60_MS / 4.0;
    let returns = wall(Material::Wood);
    for tap in returns {
        assert!(
            tap.delay_ms() < f64::from(tail_quarter_ms),
            "tap {:.1} ms is inside the first quarter of the tail",
            tap.delay_ms()
        );
    }
    // One wall, four delays from one distance: the room sizes them, not a preset.
    let closer = taps(500, Material::Wood).expect("5 m is inside the band");
    assert!(closer[0].delay_us < returns[0].delay_us / 3);
    assert_eq!(
        closer[0].gain_q16, returns[0].gain_q16,
        "gain is the wall's"
    );
}

#[test]
fn early_lines_sum_into_the_wet_bus_without_allocating() {
    let returns = wall(Material::Stone);
    let mut lines = EarlyLines::new(&returns, SR).expect("48 kHz lines build");
    let first = usize::try_from(delay_samples(returns[0].delay_us, SR)).expect("fits");
    assert!(first > 0, "a tapped return is always late");
    assert_eq!(lines.lens()[0], first, "one line per flight time");
    let mut input = vec![0.0f32; first + 8];
    input[0] = 1.0;
    let mut wet = vec![0.0f32; first + 8];
    let ((), allocs) = engine_core::alloc_probe::measure(|| lines.add(&input, &mut wet));
    assert_eq!(allocs.calls, 0, "the audio thread path allocates nothing");
    // The impulse comes back off the wall exactly at its flight time.
    assert_eq!(wet[0], 0.0, "the dry sample is not the reflection");
    let peak = wet
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.abs().partial_cmp(&b.1.abs()).expect("finite"))
        .map(|(index, _)| index);
    assert_eq!(peak, Some(first), "the return lands at its own delay");
    assert!(wet[first] > 0.0, "the return carries the wall's gain");
}

#[test]
fn taps_refuse_a_bad_distance_or_rate() {
    assert_eq!(
        taps(0, Material::Stone).unwrap_err(),
        EarlyError::BadDistance(0)
    );
    assert_eq!(
        taps(MAX_DIST_CM + 1, Material::Stone).unwrap_err(),
        EarlyError::BadDistance(MAX_DIST_CM + 1)
    );
    let returns = wall(Material::Stone);
    assert_eq!(
        EarlyLines::new(&returns, 100).unwrap_err(),
        EarlyError::BadRate(100)
    );
    assert!(EarlyError::BadDistance(0).to_string().contains("0"));
    assert!(EarlyError::BadRate(100).to_string().contains("100"));
}

#[test]
fn same_distance_gives_the_same_taps_twice() {
    for material in [Material::Sand, Material::Wood, Material::Stone] {
        let a = taps(1_234, material).expect("inside the band");
        let b = taps(1_234, material).expect("inside the band");
        assert_eq!(a, b, "{material:?}: same distance, same taps");
        // A different material keeps the timing and moves only the gain.
        let c = taps(1_234, Material::Cloth).expect("inside the band");
        for (left, right) in a.iter().zip(c.iter()) {
            assert_eq!(left.delay_us, right.delay_us, "timing is the room's");
            assert_ne!(left.gain_q16, right.gain_q16, "gain is the wall's");
        }
    }
}

#[test]
fn early_taps_reach_the_reverb_wet_bus_before_the_tail() {
    // Two fresh tunnels fed the same impulse: the only difference the early
    // bus makes is the wall's return, sample for sample.
    let params = crate::reverb::ReverbParams::tunnel(SR).expect("tunnel preset builds");
    let returns = wall(Material::Stone);
    let mut lines = EarlyLines::new(&returns, SR).expect("lines build");
    let first = usize::try_from(delay_samples(returns[0].delay_us, SR)).expect("fits");
    let mut input = vec![0.0f32; first + 16];
    input[0] = 1.0;
    let mut with_early = vec![0.0f32; input.len()];
    let mut tail_only = vec![0.0f32; input.len()];
    let mut verb = crate::reverb::Reverb::new(&params).expect("reverb builds");
    verb.process_with_early(&input, &mut with_early, &mut lines)
        .expect("the early bus rides the same block");
    let mut tail_verb = crate::reverb::Reverb::new(&params).expect("reverb builds");
    tail_verb
        .process(&input, &mut tail_only)
        .expect("the tail alone renders");
    for index in 0..input.len() {
        let early = returns[0].gain_q16;
        let want = if index == first {
            q16_to_gain(early)
        } else {
            0.0
        };
        let added = with_early[index] - tail_only[index];
        assert!(
            (added - want).abs() <= 1e-6,
            "sample {index}: the bus gained {added}, want {want}"
        );
    }
    assert!(
        with_early[first] > tail_only[first],
        "the wall's return lands on top of the tail"
    );
}
