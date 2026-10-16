//! W1 resim-parity proof for [`super::NetPose`].
//!
//! Two clients at 150 ms RTT (pure delay, plus delay with drops) record the
//! same pose checkpoints as the authority after rollback resim, and the
//! stream costs one charted kbps-per-fighter line. Test-only: integer math
//! over replicated picks, no IO, no wall clock.

use super::*;

/// Authority tape without locomotion: raw `(clip, tick)` picks plus a
/// sweeping gait phase, so the proof never depends on the match search.
fn authority_tape(ticks: usize) -> Vec<NetPose> {
    (0..ticks)
        .map(|k| {
            // Named conversion via TryFrom: `k % 3` is 0..2, always fits `u16`.
            let clip = u16::try_from(k % 3).unwrap_or(u16::MAX);
            // Named conversion via TryFrom: `(k * 7) % 64` is 0..63, always fits `u16`.
            let tick = u16::try_from((k * 7) % 64).unwrap_or(u16::MAX);
            // Named conversion via From: `u16` to `f32` is exact for 0..16.
            let phase = f32::from(u16::try_from(k % 16).unwrap_or(0)) / 16.0;
            // Fixture is infallible via unwrap_or_else: phase stays finite in
            // `[0, 1)`, so `from_pick` always succeeds; the assert keeps a
            // loud failure if that ever changes.
            NetPose::from_pick(clip, tick, phase)
                .unwrap_or_else(|e| panic!("fixture phase finite: {e}"))
        })
        .collect()
}

/// First index where the degraded stream diverges, if any.
fn first_divergence(seen: &[NetPose], authority: &[NetPose]) -> Option<usize> {
    seen.iter().zip(authority.iter()).position(|(a, b)| a != b)
}

#[test]
fn two_clients_match_after_resim_at_150ms_rtt() {
    let auth = authority_tape(32);
    let start = 5_000u32;
    let auth_points = checkpoints_for_stream(&auth, start);
    // Same stream twice checkpoints identically (replay x2).
    assert_eq!(checkpoints_for_stream(&auth, start), auth_points);

    // Client A sees the stream 150 ms late (9 ticks of pure delay).
    let delay = RTT_TICKS_150MS as usize;
    let mut late_a = vec![auth[0]; delay];
    late_a.extend_from_slice(&auth[..auth.len() - delay]);
    // Client B jitters worse: same delay plus a dropped packet every 11
    // ticks, holding the last good pose through the gap.
    let mut late_b = Vec::with_capacity(auth.len());
    let mut last = auth[0];
    for k in 0..auth.len() {
        if k >= delay && k % 11 != 0 {
            last = auth[k - delay];
        }
        late_b.push(last);
    }
    // Before resim the degraded clients visibly diverge from authority.
    assert_ne!(
        checkpoints_for_stream(&late_a, start),
        auth_points,
        "pure delay must shift checkpoints before resim"
    );
    assert_ne!(
        checkpoints_for_stream(&late_b, start),
        auth_points,
        "drops must diverge checkpoints before resim"
    );
    // Rollback resim replays the authoritative log from the first uncertain
    // tick: both clients land back on the authority points.
    let split_a = first_divergence(&late_a, &auth).expect("delay diverges");
    let split_b = first_divergence(&late_b, &auth).expect("drops diverge");
    let fixed_a = resimulate(&late_a, &auth, split_a);
    let fixed_b = resimulate(&late_b, &auth, split_b);
    assert_ne!(fixed_a, late_a, "resim repairs the delayed stream");
    assert_ne!(fixed_b, late_b, "resim repairs the lossy stream");
    let resim_a = checkpoints_for_stream(&fixed_a, start);
    let resim_b = checkpoints_for_stream(&fixed_b, start);
    assert_eq!(resim_a, auth_points);
    assert_eq!(resim_b, auth_points);
    assert_eq!(resim_a, resim_b);
    // Replay twice: the same fix, bit-for-bit.
    assert_eq!(
        checkpoints_for_stream(&resimulate(&late_a, &auth, split_a), start),
        auth_points,
        "replay twice: same fix bit-for-bit"
    );

    // Bandwidth is charted: 6 bytes per tick at 60 Hz is 2.88 kbps.
    assert!(
        (kbps_per_fighter() - 2.88).abs() < 1e-9,
        "one kbps-per-fighter line: {}",
        kbps_per_fighter()
    );
}

#[test]
fn resimulate_splits_clamp_without_panic() {
    let auth = authority_tape(8);
    // Certain from tick zero replays the authority exactly.
    assert_eq!(resimulate(&auth, &auth, 0), auth);
    // Empty logs replay nothing; over-long splits clamp to the shorter log.
    assert!(resimulate(&[], &[], 0).is_empty());
    assert_eq!(resimulate(&auth, &auth, 1_000_000), auth);
}
