//! delay: input-delay helpers driven by frame advantage.
//!
//! Owner: `engine-net`; consumer: `snapshot.rs`, which pins its hold-back
//! through [`clamp_delay`] and [`DEFAULT_DELAY`]. Units: frames (`u32`), delay ticks (`u32`), signed frame
//! advantage (`i32`). Determinism: integer-only stepping (`+ -`, compares,
//! clamps); no float, no wall clock, no hashing. Errors: inputs clamp
//! loudly in range (never panic), out-of-range delays pin to
//! `MIN_DELAY..=MAX_DELAY`.
//!
//! Frame advantage is `local_frame - remote_frame`: positive means the
//! local side is ahead and should add delay; negative means behind and
//! should drop delay. [`delay_for_advantage`] steps the knob by at most one
//! tick per call toward the target, so callers drive it once per frame with
//! the latest advantage.

/// Smallest input delay in ticks (render immediately).
pub const MIN_DELAY: u32 = 0;
/// Largest input delay in ticks (keeps the knob inside the rollback window).
pub const MAX_DELAY: u32 = 8;
/// Starting input delay in ticks (wired through `session::INPUT_DELAY_TICKS`).
pub const DEFAULT_DELAY: u32 = super::session::INPUT_DELAY_TICKS;
/// Starting frame-advantage target: drive local and remote frames even.
pub const DEFAULT_TARGET_ADVANTAGE: i32 = 0;
/// Clamp bound for an advantage target (keeps the knob from chasing noise).
pub const MAX_TARGET_ABS: i32 = 8;

/// Clamp a delay into `MIN_DELAY..=MAX_DELAY`.
#[must_use]
pub fn clamp_delay(delay: u32) -> u32 {
    delay.clamp(MIN_DELAY, MAX_DELAY)
}

/// Signed frame advantage: `local_frame - remote_frame` in ticks.
///
/// Positive means local is ahead (add delay); negative means behind (drop
/// delay). Integer-only: widens through `i64`, then pins to `i32` range.
#[must_use]
pub fn frame_advantage(local_frame: u32, remote_frame: u32) -> i32 {
    let diff = i64::from(local_frame) - i64::from(remote_frame);
    let clamped = diff.clamp(i64::from(i32::MIN), i64::from(i32::MAX));
    i32::try_from(clamped).unwrap_or(i32::MAX)
}

/// One integer step of the knob toward `target`.
///
/// Returns `current` pinned to range when `advantage == target`, `+1` when
/// ahead, `-1` when behind, then pinned to `MIN_DELAY..=MAX_DELAY`.
#[must_use]
pub fn delay_for_advantage(current: u32, advantage: i32, target: i32) -> u32 {
    let cur = clamp_delay(current);
    if advantage > target {
        cur.saturating_add(1).min(MAX_DELAY)
    } else if advantage < target {
        cur.saturating_sub(1)
    } else {
        cur
    }
}

/// Fault profile of the input-delay sweep: the WAN base latency (4 ticks,
/// 67 ms) with 30 ms jitter (2 ticks), no reordering, at `loss_pct` loss.
#[cfg(test)]
fn sweep_profile(loss_pct: u32) -> super::link::LinkProfile {
    super::link::LinkProfile {
        latency: 4,
        jitter: 2,
        loss_pct,
        reorder_pct: 0,
    }
}

/// One seeded loopback bout per delay in `MIN_DELAY..=MAX_DELAY`.
#[cfg(test)]
fn sweep(loss_pct: u32) -> Vec<super::rollback::loopback::Receipt> {
    (MIN_DELAY..=MAX_DELAY)
        .map(|delay| super::rollback::loopback::run(delay, sweep_profile(loss_pct), 0x5EE9, 600))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_advantage_sign_and_math() {
        assert_eq!(frame_advantage(10, 8), 2);
        assert_eq!(frame_advantage(8, 10), -2);
        assert_eq!(frame_advantage(7, 7), 0);
        assert_eq!(frame_advantage(u32::MAX, 0), i32::MAX);
        assert_eq!(frame_advantage(0, u32::MAX), i32::MIN);
    }

    #[test]
    fn free_stepper_matches_knob() {
        assert_eq!(delay_for_advantage(2, 1, 0), 3);
        assert_eq!(delay_for_advantage(2, -1, 0), 1);
        assert_eq!(delay_for_advantage(2, 0, 0), 2);
        assert_eq!(delay_for_advantage(MAX_DELAY, 9, 0), MAX_DELAY);
        assert_eq!(delay_for_advantage(MIN_DELAY, -9, 0), MIN_DELAY);
        assert_eq!(delay_for_advantage(99, 0, 0), MAX_DELAY);
    }

    #[test]
    fn default_delay_matches_session_ticks() {
        assert_eq!(DEFAULT_DELAY, crate::session::INPUT_DELAY_TICKS);
        assert_eq!(DEFAULT_DELAY, 2);
    }

    #[test]
    fn input_delay_sweep_posts_misprediction_and_resim_per_setting() {
        // QN2 receipt: delay 0..=8 over the WAN base latency with 30 ms
        // jitter at 0, 2 and 5 percent loss. Every cell converges to the
        // truth hash and more delay never predicts more frames. The data
        // does not pick the default: resim depth falls one tick per delay
        // tick and already sits under the 9-tick window at delay 0, so the
        // default is held as safe (inside the window), not as the minimum.
        for loss in [0, 2, 5] {
            let rows = sweep(loss);
            for (delay, r) in (MIN_DELAY..).zip(&rows) {
                let per_1000 = r.mispredicted * 1000 / u64::from(r.frames);
                println!(
                    "sweep loss={loss}% d={delay} pred={} mispred={} per1000={per_1000} resims={} max={}",
                    r.predicted, r.mispredicted, r.rollbacks, r.max_rollback,
                );
                assert!(r.tip_matches_truth, "loss {loss} delay {delay}: {r:?}");
            }
            for pair in rows.windows(2) {
                assert!(pair[1].predicted <= pair[0].predicted, "{pair:?}");
            }
            let at_default = &rows[DEFAULT_DELAY as usize];
            assert!(
                at_default.max_rollback <= crate::netcode::MAX_ROLLBACK,
                "loss {loss}: the default resims past the window: {at_default:?}"
            );
            assert!(
                at_default.rollbacks <= rows[0].rollbacks
                    && at_default.max_rollback < rows[0].max_rollback,
                "loss {loss}: the default must resim less deep than delay 0"
            );
        }
    }
}
