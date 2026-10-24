//! SW114 tests: the tunnel-to-street feather, its equal-power gains, the
//! hysteresis walk and the crossfade that keeps the wet level steady.

use super::*;
use crate::reverb::{Reverb, ReverbParams, TUNNEL_RT60_MS};

const SR: u32 = 48_000;
const BLOCK: usize = 512;
/// The street canyon's wet level.
const STREET_WET: f32 = 0.30;

fn tunnel() -> ReverbParams {
    ReverbParams::tunnel(SR).expect("tunnel preset builds")
}

fn street() -> ReverbParams {
    // The street canyon's own numbers: 600 ms tail, 12 ms pre-delay.
    ReverbParams {
        sample_rate_hz: SR,
        pre_delay_ms: 12.0,
        rt60_ms: 600.0,
        damping: 0.58,
        wet: STREET_WET,
    }
}

/// A deterministic send: one hit every 97 frames over seeded noise, no clock.
fn send(frames: usize, phase: usize) -> Vec<f32> {
    let mut state = 0x2545_F491_4F6C_DD1Du64 ^ u64::try_from(phase).unwrap_or(0);
    let mut out = Vec::with_capacity(frames);
    for frame in 0..frames {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        let lane = u16::try_from((state >> 40) & 0xFFFF).unwrap_or(0);
        let noise = f32::from(lane) / 65_535.0 - 0.5;
        let hit = if frame % 97 == 0 { 0.8 } else { 0.0 };
        out.push(hit + noise * 1e-3);
    }
    out
}

/// Energy RMS of a block: the meter the walk is judged by.
fn rms(block: &[f32]) -> f32 {
    let sum: f64 = block.iter().map(|s| f64::from(*s) * f64::from(*s)).sum();
    let frames = u32::try_from(block.len()).unwrap_or(1).max(1);
    (sum / f64::from(frames)).sqrt() as f32
}

fn db(ratio: f32) -> f32 {
    20.0 * ratio.max(1e-9).log10()
}

#[test]
fn zone_weights_crossfade_without_a_wet_dip() {
    // The row's number: 50 cm inside the mouth reads 25/75 tunnel/street.
    let at_50 = blend(50);
    assert_eq!(at_50.zones, [TUNNEL as u8, STREET as u8]);
    let tunnel_pct = f64::from(at_50.weight_q16[TUNNEL]) / f64::from(WEIGHT_ONE) * 100.0;
    let street_pct = f64::from(at_50.weight_q16[STREET]) / f64::from(WEIGHT_ONE) * 100.0;
    assert!(
        (tunnel_pct - 25.0).abs() < 0.5 && (street_pct - 75.0).abs() < 0.5,
        "50 cm inside reads {tunnel_pct:.1}/{street_pct:.1}"
    );
    // The ends are pure rooms, and the pair never stops summing to unity.
    assert_eq!(weights(FEATHER_CM), [WEIGHT_ONE, 0]);
    assert_eq!(weights(0), [0, WEIGHT_ONE]);
    // The tail levels are divided out, so the pool changes room, not volume.
    let norm = normalized_gains(&blend(50), [1.0, 0.5]);
    let plain = blend(50).gains();
    assert!((norm[TUNNEL] - plain[TUNNEL]).abs() < 1e-6);
    assert!((norm[STREET] - plain[STREET] * 2.0).abs() < 1e-6);
    assert_eq!(normalized_gains(&blend(50), [0.0, 1.0])[TUNNEL], 0.0);
    for inside in -50..=FEATHER_CM + 50 {
        let pair = weights(inside);
        assert_eq!(
            pair[0] + pair[1],
            WEIGHT_ONE,
            "inside {inside}: the pair stays unity"
        );
        assert!(pair[0] >= 0 && pair[1] >= 0, "inside {inside}: no negative");
        let gains = blend(inside).gains();
        assert!(
            (gains[TUNNEL] * gains[TUNNEL] + gains[STREET] * gains[STREET] - 1.0).abs() < 1e-3,
            "inside {inside}: gains square to one, got {gains:?}"
        );
    }

    // Walk the mouth with both tails live: the wet never dips more than 3 dB.
    let mut near = Reverb::new(&tunnel()).expect("tunnel builds");
    let mut far = Reverb::new(&street()).expect("street builds");
    let mut ramp = Ramp::new(TUNNEL_RT60_MS, SR).expect("ramp builds");
    // Warm both tails, then read each one's own steady level solo, so the
    // crossfade is matched to the rooms instead of guessed.
    let mut phase = 0usize;
    let mut level = [0.0f32; ZONE_PAIR];
    for _ in 0..24 {
        let input = send(BLOCK, phase);
        phase += 1;
        let mut wet_near = vec![0.0f32; BLOCK];
        let mut wet_far = vec![0.0f32; BLOCK];
        near.process(&input, &mut wet_near).expect("near tail");
        far.process(&input, &mut wet_far).expect("far tail");
        level = [rms(&wet_near), rms(&wet_far)];
        ramp.aim(&blend(FEATHER_CM));
        ramp.advance(BLOCK);
    }
    assert!(
        level[TUNNEL] > 0.0 && level[STREET] > 0.0,
        "both tails ring"
    );
    let mut levels: Vec<f32> = Vec::new();
    for inside in (0..=320).rev().step_by(5) {
        let target = blend(inside);
        let input = send(BLOCK, phase);
        phase += 1;
        let mut wet_near = vec![0.0f32; BLOCK];
        let mut wet_far = vec![0.0f32; BLOCK];
        near.process(&input, &mut wet_near).expect("near tail");
        far.process(&input, &mut wet_far).expect("far tail");
        ramp.aim(&target);
        let mut blended = vec![0.0f32; BLOCK];
        for (frame, sample) in blended.iter_mut().enumerate() {
            let gains = ramp.advance(1);
            // Each tail is matched by its own steady level, the rule
            // `normalized_gains` documents: the pool changes room, not volume.
            *sample = gains[TUNNEL] / level[TUNNEL] * wet_near[frame]
                + gains[STREET] / level[STREET] * wet_far[frame];
        }
        levels.push(rms(&blended));
    }
    assert!(levels.len() >= 40, "the walk covers the whole feather");
    let peak = levels.iter().copied().fold(0.0f32, f32::max);
    assert!(peak > 0.0, "the walk carries the tails");
    for (index, level) in levels.iter().enumerate() {
        assert!(
            db(level / peak) > -3.0,
            "window {index} dips {:.1} dB below the peak",
            db(level / peak)
        );
    }
}

#[test]
fn hysteresis_stops_flip_on_a_jitter_walk() {
    // Without the band the dominant zone follows whichever tail is louder, so
    // a 3 cm jitter on the mid-point flips it on every crossing.
    let jitter: Vec<i32> = (0..9)
        .map(|step| if step % 2 == 0 { 97 } else { 103 })
        .collect();
    let mut flips_without = 0u32;
    let mut last = weights(jitter[0])[STREET] > weights(jitter[0])[TUNNEL];
    for inside in &jitter[1..] {
        let now = weights(*inside)[STREET] > weights(*inside)[TUNNEL];
        if now != last {
            flips_without += 1;
            last = now;
        }
    }
    assert_eq!(flips_without, 8, "the raw weights flip on every crossing");

    // With the band: one real walk out, then the same jitter, one switch total.
    let mut walker = Walker::new();
    assert_eq!(walker.dominant(), TUNNEL as u8);
    for inside in (60..=140).rev().step_by(5) {
        walker.walk(inside);
    }
    assert_eq!(
        walker.dominant(),
        STREET as u8,
        "walking out past the band turns the room over once"
    );
    for inside in &jitter {
        walker.walk(*inside);
    }
    assert_eq!(walker.switches(), 1, "the jitter adds no switch of its own");
    // Walking back in past the far edge of the band turns it back exactly once.
    for inside in (140..=200).rev().step_by(5) {
        walker.walk(inside);
    }
    assert_eq!(walker.dominant(), TUNNEL as u8);
    assert_eq!(walker.switches(), 2);
}

#[test]
fn crossfade_takes_one_quarter_rt60_and_keeps_the_pair_normalized() {
    let mut ramp = Ramp::new(TUNNEL_RT60_MS, SR).expect("ramp builds");
    // 1100 ms / 4 at 48 kHz is exactly 13 200 frames.
    let quarter_frames = 13_200usize;
    ramp.aim(&blend(0));
    let half = ramp.advance(quarter_frames / 2);
    assert!(
        half[STREET] > 0.6 && half[STREET] < 0.8,
        "halfway through the fade the street is at {:.3}",
        half[STREET]
    );
    assert!(!ramp.settled(), "the fade is still moving halfway through");
    ramp.advance(quarter_frames);
    assert!(ramp.settled(), "one RT60/4 completes the fade");
    let done = ramp.gains();
    assert!(
        (done[STREET] - 1.0).abs() < 1e-3,
        "the street tail owns it now: {done:?}"
    );
    assert!(done[TUNNEL].abs() < 1e-3, "the tunnel tail is gone");
    // Equal power at every step of the fade, not just at the ends.
    let mut probe = Ramp::new(TUNNEL_RT60_MS, SR).expect("ramp builds");
    probe.aim(&blend(0));
    for step in 0..40 {
        let gains = probe.advance(quarter_frames / 40);
        assert!(
            (gains[TUNNEL] * gains[TUNNEL] + gains[STREET] * gains[STREET] - 1.0).abs() < 1e-5,
            "step {step}: gains square to one, got {gains:?}"
        );
    }
}

#[test]
fn blend_refuses_a_hand_built_pair_of_zero_power() {
    let empty = Blend {
        zones: [TUNNEL as u8, STREET as u8],
        weight_q16: [0, 0],
    };
    assert_eq!(empty.gains(), [0.0, 0.0], "no power, no gain");
    assert_eq!(empty.live(), [false, false]);
    assert_eq!(empty.street_share(), 0.0);
    let odd = Blend {
        zones: [TUNNEL as u8, STREET as u8],
        weight_q16: [WEIGHT_ONE / 2, WEIGHT_ONE / 2],
    };
    let gains = odd.gains();
    assert!(
        (gains[TUNNEL] * gains[TUNNEL] + gains[STREET] * gains[STREET] - 1.0).abs() < 1e-6,
        "a hand-built pair still normalizes: {gains:?}"
    );
    assert_eq!(odd.total_weight_q16(), WEIGHT_ONE);
    assert_eq!(odd.live(), [true, true]);
}

#[test]
fn ramp_refuses_a_tail_shorter_than_its_own_bar() {
    assert!(
        Ramp::new(1.0, SR).is_err(),
        "a 1 ms tail has no quarter to fade over"
    );
    assert!(
        Ramp::new(TUNNEL_RT60_MS, 100).is_err(),
        "a 100 Hz rate is refused"
    );
}
