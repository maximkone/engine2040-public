//! delay: fixed input-delay knob driven by frame advantage.
//!
//! Owner: `engine-net`; consumer: P2P sessions that hold local inputs `D`
//! ticks before render so late remote inputs land inside the rollback
//! window. Units: frames (`u32`), delay ticks (`u32`), signed frame
//! advantage (`i32`). Determinism: integer-only stepping (`+ -`, compares,
//! clamps); no float, no wall clock, no hashing. Errors: inputs clamp
//! loudly in range (never panic), out-of-range delays pin to
//! `MIN_DELAY..=MAX_DELAY`.
//!
//! Frame advantage is `local_frame - remote_frame`: positive means the
//! local side is ahead and should add delay; negative means behind and
//! should drop delay. [`FixedDelay::update`] steps the knob by at most one
//! tick per call toward [`FixedDelay::target`], so callers drive it once
//! per frame with the latest advantage.

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

/// Clamp an advantage target into `-MAX_TARGET_ABS..=MAX_TARGET_ABS`.
#[must_use]
pub fn clamp_target(target: i32) -> i32 {
    target.clamp(-MAX_TARGET_ABS, MAX_TARGET_ABS)
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
        cur.saturating_sub(1).max(MIN_DELAY)
    } else {
        cur
    }
}

/// Fixed input-delay knob driven by frame advantage.
///
/// Holds one integer delay in ticks. Call [`Self::update`] once per frame
/// with the latest [`frame_advantage`]; the knob steps by at most one tick
/// toward the target, so it cannot jump on a single noisy sample.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FixedDelay {
    delay: u32,
    target: i32,
}

impl FixedDelay {
    /// New knob at `delay` ticks, pinned to range, targeting even frames.
    #[must_use]
    pub fn new(delay: u32) -> Self {
        Self {
            delay: clamp_delay(delay),
            target: DEFAULT_TARGET_ADVANTAGE,
        }
    }

    /// New knob at `delay` ticks with an explicit advantage `target`.
    #[must_use]
    pub fn with_target(delay: u32, target: i32) -> Self {
        Self {
            delay: clamp_delay(delay),
            target: clamp_target(target),
        }
    }

    /// Current delay in ticks.
    #[must_use]
    pub fn delay(&self) -> u32 {
        self.delay
    }

    /// Frame-advantage target the knob steps toward.
    #[must_use]
    pub fn target(&self) -> i32 {
        self.target
    }

    /// Set the delay directly, pinned to `MIN_DELAY..=MAX_DELAY`.
    pub fn set_delay(&mut self, delay: u32) {
        self.delay = clamp_delay(delay);
    }

    /// Set the advantage target, pinned to `-MAX_TARGET_ABS..=MAX_TARGET_ABS`.
    pub fn set_target(&mut self, target: i32) {
        self.target = clamp_target(target);
    }

    /// Step the knob once from `frame_advantage`; returns the new delay.
    pub fn update(&mut self, advantage: i32) -> u32 {
        self.delay = delay_for_advantage(self.delay, advantage, self.target);
        self.delay
    }
}

impl Default for FixedDelay {
    fn default() -> Self {
        Self::new(DEFAULT_DELAY)
    }
}

/// Genre-neutral integer probe for the delay reaction table.
#[derive(Clone, Default)]
struct DelayProbe {
    acc: u64,
}

impl super::rollback::SimTick for DelayProbe {
    fn advance(&mut self, inputs: [u32; super::rollback::PLAYERS]) {
        let mix = u64::from(inputs[0])
            .wrapping_mul(31)
            .wrapping_add(u64::from(inputs[1]).wrapping_mul(131))
            .wrapping_add(7);
        self.acc = self
            .acc
            .wrapping_add(mix)
            .wrapping_add(0x9E37_79B9_7F4A_7C15 & 0x00FF_FFFF_FFFF_FFFF);
    }

    fn state_hash(&self) -> u64 {
        let mut fold = engine_core::Fnv64::new();
        fold.write_u64(self.acc);
        fold.finish()
    }
}

fn probe_pair(n: usize) -> (Vec<u32>, Vec<u32>) {
    let mut local = Vec::with_capacity(n);
    let mut remote = Vec::with_capacity(n);
    for f in 0..n {
        local.push((f as u32).wrapping_mul(3).wrapping_add(2) % 5);
        remote.push((f as u32).wrapping_mul(5).wrapping_add(1) % 5);
    }
    (local, remote)
}

fn count_rollbacks(delay: u32) -> u64 {
    let (local, remote) = probe_pair(48);
    let mut session = super::rollback::P2pSession::new(DelayProbe::default());
    session.enable_sparse_saving(false);
    assert!(session.verify_full_window(0).is_ok());
    for (f, (&l, &r)) in local.iter().zip(remote.iter()).enumerate() {
        let lost = f as u32 % 5 == 4 && delay < 2;
        let avail = if lost { None } else { Some(r) };
        session.advance_frame(l, avail);
        if f >= 2 {
            let late = (f - 2) as u32;
            if late % 5 == 4 && delay < 2 {
                let _ = session.confirm_frame(late, remote[f - 2]);
            }
        }
    }
    for (f, &r) in remote.iter().enumerate() {
        if f as u32 % 5 == 4 && delay < 2 {
            let _ = session.confirm_frame(f as u32, r);
        }
    }
    session.rollback_count()
}

fn reaction_table() -> [(u32, u64); 3] {
    [
        (0, count_rollbacks(0)),
        (2, count_rollbacks(2)),
        (4, count_rollbacks(4)),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_clamps_to_range() {
        assert_eq!(FixedDelay::new(99).delay(), MAX_DELAY);
        assert_eq!(FixedDelay::new(MIN_DELAY).delay(), MIN_DELAY);
        assert_eq!(FixedDelay::default().delay(), DEFAULT_DELAY);
        assert_eq!(FixedDelay::default().target(), DEFAULT_TARGET_ADVANTAGE);
    }

    #[test]
    fn with_target_clamps_both() {
        let knob = FixedDelay::with_target(99, 99);
        assert_eq!(knob.delay(), MAX_DELAY);
        assert_eq!(knob.target(), MAX_TARGET_ABS);
        let knob = FixedDelay::with_target(2, -99);
        assert_eq!(knob.target(), -MAX_TARGET_ABS);
    }

    #[test]
    fn frame_advantage_sign_and_math() {
        assert_eq!(frame_advantage(10, 8), 2);
        assert_eq!(frame_advantage(8, 10), -2);
        assert_eq!(frame_advantage(7, 7), 0);
        assert_eq!(frame_advantage(u32::MAX, 0), i32::MAX);
        assert_eq!(frame_advantage(0, u32::MAX), i32::MIN);
    }

    #[test]
    fn update_steps_one_tick_toward_target() {
        let mut knob = FixedDelay::new(2);
        assert_eq!(knob.update(3), 3, "ahead adds one tick");
        assert_eq!(knob.update(3), 4, "still ahead adds one more");
        assert_eq!(knob.update(0), 4, "on target holds");
        assert_eq!(knob.update(-1), 3, "behind drops one tick");
    }

    #[test]
    fn update_pins_at_range_edges() {
        let mut top = FixedDelay::new(MAX_DELAY);
        assert_eq!(top.update(5), MAX_DELAY, "at max stays pinned");
        let mut bottom = FixedDelay::new(MIN_DELAY);
        assert_eq!(bottom.update(-5), MIN_DELAY, "at min stays pinned");
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
    fn custom_target_drives_to_target() {
        let mut knob = FixedDelay::with_target(2, 2);
        assert_eq!(knob.update(2), 2, "on custom target holds");
        assert_eq!(knob.update(3), 3, "above custom target adds");
        assert_eq!(knob.update(1), 2, "below custom target drops");
    }

    #[test]
    fn setters_clamp() {
        let mut knob = FixedDelay::default();
        knob.set_delay(u32::MAX);
        assert_eq!(knob.delay(), MAX_DELAY);
        knob.set_target(i32::MAX);
        assert_eq!(knob.target(), MAX_TARGET_ABS);
    }

    #[test]
    fn default_delay_matches_session_ticks() {
        assert_eq!(DEFAULT_DELAY, crate::session::INPUT_DELAY_TICKS);
        assert_eq!(DEFAULT_DELAY, 2);
    }

    #[test]
    fn delay_reaction_table_posts_counts() {
        let table = reaction_table();
        assert_eq!([table[0].0, table[1].0, table[2].0], [0, 2, 4]);
        assert!(table[0].1 > 0, "delay 0 must resim on seeded loss");
        assert!(
            table[1].1 < table[0].1,
            "delay 2 hides the 2-tick lateness: {:?}",
            table
        );
        assert_eq!(table[1].1, 0, "delay 2 covers seeded lateness: {:?}", table);
        assert_eq!(table[2].1, 0, "delay 4 stays covered: {:?}", table);
        let per_1000 = table[1].1 * 1000 / 48;
        assert!(per_1000 < 3, "delay 2 under 3 per 1000: {per_1000}");
        println!(
            "delay-reaction: d0={} d2={} d4={} per48",
            table[0].1, table[1].1, table[2].1
        );
    }
}
