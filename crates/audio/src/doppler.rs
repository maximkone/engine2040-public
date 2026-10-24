//! doppler: the pitch of a passing source, in Q12.
//!
//! Nothing in the mix changed a source's playback rate before this module: a
//! spear thrown past the camera sounded exactly like a spear standing still,
//! and a charging fighter slid by at one pitch. The only rate change was at
//! load (`Clip::from_pcm_at_rate`) and the bank's variant offsets.
//!
//! [`ratio`] is the classic moving-source ratio with the speed of sound
//! `c = 34_300 cm/s` (the same constant the gunfire layer uses for its travel
//! delay):
//!
//! ```text
//! r = (c + v_listener_closes) / (c - v_source_closes)
//! ```
//!
//! Both velocities are closing speeds in cm/s: positive means the source (or
//! the listener) is getting nearer. A source closing at 10 m/s therefore
//! reads 1.030 (the pitch rises) and one receding at 10 m/s reads 0.971. The
//! result is Q12 (`4096` = 1.0), clamped to 0.5..=2.0 so a wildly fast source
//! cannot turn a voice inside out, and snapped to exactly 1.0 when neither
//! side moves faster than [`SNAP_CM_S`], so an idle fighter is not detuned by
//! rounding.
//!
//! Deterministic: integers only, no allocation, no clock.

use crate::gunfire::SPEED_OF_SOUND_CM_PER_S;

/// Q12 unity: a source at rest plays at its recorded rate.
pub const UNITY_Q12: i32 = 4_096;
/// Below this closing speed (either side) the ratio snaps to unity: 50 cm/s.
pub const SNAP_CM_S: i32 = 50;
/// Fastest the ratio may run (a source closing at the speed of sound): 2.0.
pub const MAX_Q12: i32 = 2 * UNITY_Q12;
/// Slowest the ratio may run: 0.5.
pub const MIN_Q12: i32 = UNITY_Q12 / 2;

/// Integer square root, floor.
pub(crate) fn isqrt(value: u64) -> i64 {
    if value == 0 {
        return 0;
    }
    let mut x = value;
    let mut y = (x + 1) / 2;
    while y < x {
        x = y;
        y = (x + value / x) / 2;
    }
    i64::try_from(x).unwrap_or(i64::MAX)
}

/// Playback rate in Q12 for a source at `speaker_cm` (offset from the
/// listener, cm) whose own velocity is `source_vel_cm_s`, while the listener
/// moves at `listener_vel_cm_s` (both cm/s on the world axes).
///
/// A velocity component along the line from the listener to the source that
/// shortens the distance is a closing speed: the source's raises the pitch,
/// the listener's raises it too. A source at the listener's own position has
/// no line to project on and reads unity.
#[must_use]
pub fn ratio(speaker_cm: [i32; 3], source_vel_cm_s: [i32; 3], listener_vel_cm_s: [i32; 3]) -> i32 {
    let [dx, dy, dz] = speaker_cm;
    let distance = isqrt(
        u64::try_from(
            i64::from(dx) * i64::from(dx)
                + i64::from(dy) * i64::from(dy)
                + i64::from(dz) * i64::from(dz),
        )
        .unwrap_or(0),
    );
    if distance == 0 {
        return UNITY_Q12;
    }
    // Projection of a velocity on the line: sum(v * speaker_cm) / distance.
    let dot = |v: [i32; 3]| -> i64 {
        i64::from(v[0]) * i64::from(dx)
            + i64::from(v[1]) * i64::from(dy)
            + i64::from(v[2]) * i64::from(dz)
    };
    // Closing is the negative of the along-the-line component: moving toward
    // the other party shortens the distance.
    let source_closes = -dot(source_vel_cm_s) / distance;
    let listener_closes = -dot(listener_vel_cm_s) / distance;
    let snap = |closes: i64| closes.abs() < i64::from(SNAP_CM_S);
    if snap(source_closes) && snap(listener_closes) {
        return UNITY_Q12;
    }
    let c = i64::from(SPEED_OF_SOUND_CM_PER_S);
    let numerator = c + listener_closes;
    let denominator = (c - source_closes).max(1);
    let q12 = numerator * i64::from(UNITY_Q12) / denominator;
    i32::try_from(q12.clamp(i64::from(MIN_Q12), i64::from(MAX_Q12))).unwrap_or(UNITY_Q12)
}

/// A playback rate in Q12 as a float, for the read cursor.
#[must_use]
pub fn ratio_f32(q12: i32) -> f32 {
    #[expect(
        clippy::as_conversions,
        clippy::cast_precision_loss,
        reason = "Q12 2048..=8192 is exact in f32"
    )]
    let value = q12 as f32 / 4_096.0;
    value
}

#[cfg(test)]
mod tests {
    use super::*;

    /// SW113 F2P: a source closing at 10 m/s reads 1.030 (Q12 4219) and one
    /// receding at the same speed reads 0.971 (Q12 3979); the listener moving
    /// toward the source raises the pitch too.
    #[test]
    fn closing_source_speeds_up_and_receding_slows_down() {
        let ahead = [1_000, 0, 0];
        let closing = ratio(ahead, [-1_000, 0, 0], [0, 0, 0]);
        let receding = ratio(ahead, [1_000, 0, 0], [0, 0, 0]);
        println!("closing {closing} Q12 (1.030 = 4219), receding {receding} Q12 (0.971 = 3979)");
        assert_eq!(closing, 4_219, "closing at 10 m/s is 1.030");
        assert_eq!(receding, 3_979, "receding at 10 m/s is 0.971");
        // The listener closing on a still source also rises.
        let listener = ratio(ahead, [0, 0, 0], [-1_000, 0, 0]);
        assert_eq!(listener, 4_215, "a closing listener is 1.029");
        // Motion across the line does not change the pitch.
        assert_eq!(ratio(ahead, [0, 1_000, 0], [0, -1_000, 0]), UNITY_Q12);
    }

    /// SW113 F2P: under 50 cm/s either side snaps to exactly unity, so a
    /// fighter shifting their weight is not detuned by rounding.
    #[test]
    fn doppler_snaps_to_unity_at_rest() {
        let ahead = [1_000, 0, 0];
        assert_eq!(ratio(ahead, [0, 0, 0], [0, 0, 0]), UNITY_Q12);
        assert_eq!(ratio(ahead, [-40, 0, 0], [0, 0, 0]), UNITY_Q12);
        assert_eq!(ratio(ahead, [0, 0, 0], [49, 0, 0]), UNITY_Q12);
        assert_eq!(ratio(ahead, [0, 0, 0], [-49, 0, 0]), UNITY_Q12);
        // Just past the snap it moves.
        assert!(ratio(ahead, [-60, 0, 0], [0, 0, 0]) > UNITY_Q12);
        assert!(ratio(ahead, [60, 0, 0], [0, 0, 0]) < UNITY_Q12);
        // A source at the listener's own position has no line to project on.
        assert_eq!(ratio([0, 0, 0], [-9_000, 0, 0], [0, 0, 0]), UNITY_Q12);
    }

    /// The clamp holds both ends: a 300 m/s closing source is 2.0, not 9.5.
    #[test]
    fn the_clamp_holds_at_both_ends() {
        let ahead = [1_000, 0, 0];
        assert_eq!(ratio(ahead, [-30_000, 0, 0], [0, 0, 0]), MAX_Q12);
        assert_eq!(ratio(ahead, [100_000, 0, 0], [0, 0, 0]), MIN_Q12);
        assert_eq!(MIN_Q12, 2_048);
        assert_eq!(MAX_Q12, 8_192);
        assert!((ratio_f32(4_219) - 1.030).abs() < 0.001);
    }
}
