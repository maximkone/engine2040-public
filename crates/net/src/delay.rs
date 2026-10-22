//! delay: input-delay helpers driven by frame advantage.
//!
//! Owner: `engine-net`; consumers: `snapshot.rs`, which pins its hold-back
//! through [`clamp_delay`] and [`DEFAULT_DELAY`], and `played.rs`, which
//! paces the client clock through [`ClockPacer`]. Units: frames (`u32`),
//! delay ticks (`u32`), signed frame advantage (`i32`), slew in permille of
//! the tick rate (`i32`). Determinism: integer-only stepping (`+ -`,
//! compares, clamps); no float, no wall clock, no hashing. Errors: inputs
//! clamp loudly in range (never panic), out-of-range delays pin to
//! `MIN_DELAY..=MAX_DELAY`.
//!
//! Frame advantage is `local_frame - remote_frame`: positive means the
//! local side is ahead and should add delay; negative means behind and
//! should drop delay. [`delay_for_advantage`] steps the knob by at most one
//! tick per call toward the target, so callers drive it once per frame with
//! the latest advantage.
//!
//! Two knobs, two speeds. The input delay is the slow knob: it moves one
//! tick at a time and only on the *averaged* advantage, never on one
//! sample. [`AdvantageWindow`] keeps the last [`WINDOW_FRAMES`] samples of
//! the two peers' confirmed frames and averages them; the average is the
//! ggrs frame advantage over a 30-frame window.
//!
//! [`ClockSlew`] turns that average into a bounded clock slew of at most
//! [`MAX_SLEW_PERMILLE`] permille (5 percent of the tick rate), and
//! [`ClockPacer`] applies it: a frame runs 0, 1 or 2 client ticks, always
//! the next ticks in sequence, so the predicted tick stream never has a
//! hole and no frame is skipped. The slew is the fast knob; the delay
//! stays the slow one.

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
///
/// The knob is the slow one: it moves one tick per call, and the caller
/// drives it with the *averaged* advantage ([`AdvantageWindow`]), never with
/// a single noisy sample.
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

/// Frames in the advantage window (the ggrs `time_sync.rs` window size).
pub const WINDOW_FRAMES: usize = 30;

/// Largest clock slew in permille of the tick rate: 5 percent. A slew above
/// this shows as an animation speed-up, so the cap is the row's kill bar.
pub const MAX_SLEW_PERMILLE: i32 = 50;

/// Permille of a tick the slew asks for per tick of averaged advantage: one
/// percent of the tick rate per tick of averaged frame advantage.
pub const SLEW_PER_ADVANTAGE_PERMILLE: i32 = 10;

/// The averaged advantage inside this many ticks counts as no advantage at
/// all (the lightyear `sync.rs` error margin): the clock holds its rate
/// instead of chasing noise.
pub const SLEW_DEAD_ZONE_TICKS: i32 = 1;

/// The 30-frame advantage window: the confirmed frame of each peer, sampled
/// once per frame, averaged over the window.
///
/// Shape: the ggrs `time_sync.rs` window, reimplemented std-only and
/// integer-only. The two confirmed frames meet in the middle, so the result
/// is the signed frame gap between the peers (positive means the local side
/// is ahead). A partial window counts the zeroed slots, exactly like the
/// donor, so the average grows smoothly from zero.
///
/// Determinism: the same sample sequence gives the same average on every
/// peer, because the arithmetic is integer-only and the window is a plain
/// ring. The window is *not* hashed and never touches the sim.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct AdvantageWindow {
    local: [i32; WINDOW_FRAMES],
    remote: [i32; WINDOW_FRAMES],
    at: usize,
    filled: usize,
}

impl Default for AdvantageWindow {
    fn default() -> Self {
        Self::new()
    }
}

impl AdvantageWindow {
    /// An empty window: every slot reads zero.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            local: [0; WINDOW_FRAMES],
            remote: [0; WINDOW_FRAMES],
            at: 0,
            filled: 0,
        }
    }

    /// Sample one frame: the confirmed frame of the local peer and of the
    /// remote one.
    pub fn advance(&mut self, local_frame: i32, remote_frame: i32) {
        self.local[self.at] = local_frame;
        self.remote[self.at] = remote_frame;
        self.at = (self.at + 1) % WINDOW_FRAMES;
        if self.filled < WINDOW_FRAMES {
            self.filled += 1;
        }
    }

    /// Frames sampled so far (never more than [`WINDOW_FRAMES`]).
    #[must_use]
    pub fn filled(&self) -> usize {
        self.filled
    }

    /// The averaged frame advantage over the window: positive means the
    /// local side is ahead of the remote one.
    ///
    /// The donor averages each side over the whole window and then takes
    /// the difference; the difference of the sums is the same number without
    /// the float, and it keeps the sign. The donor halves the difference
    /// because the two peers share one correction; here each peer runs its
    /// own clock, so the local side takes the whole difference and the two
    /// windows stay exact mirrors: the peers' averages are equal and
    /// opposite, and so are their slews.
    #[must_use]
    pub fn average_frame_advantage(&self) -> i32 {
        let local: i64 = self.local.iter().map(|value| i64::from(*value)).sum();
        let remote: i64 = self.remote.iter().map(|value| i64::from(*value)).sum();
        let gap = local - remote;
        let scaled = gap / i64::try_from(WINDOW_FRAMES).unwrap_or(i64::MAX);
        i32::try_from(scaled.clamp(i64::from(i32::MIN), i64::from(i32::MAX))).unwrap_or(0)
    }
}

/// The clock slew: the local tick rate is stretched or squeezed by at most
/// [`MAX_SLEW_PERMILLE`] permille to close the averaged frame advantage.
///
/// Shape: the lightyear `sync.rs` speed controller, reimplemented std-only.
/// The error (the averaged advantage) is the input; the output is a bounded
/// relative speed. A positive error (the local side is ahead) slows the
/// local clock down, so the remote side catches up; a negative error speeds
/// it up. The error inside [`SLEW_DEAD_ZONE_TICKS`] counts as no error at
/// all, so the clock holds its rate instead of chasing noise.
///
/// The slew is applied as pacing, never as a skip: a frame runs 0, 1 or 2
/// local ticks, and the tick sequence stays contiguous.
///
/// Determinism: integer-only, no float, no wall clock. The same averaged
/// advantage gives the same slew on every peer.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct ClockSlew {
    permille: i32,
}

impl ClockSlew {
    /// A slew that holds the clock at its nominal rate.
    #[must_use]
    pub const fn new() -> Self {
        Self { permille: 0 }
    }

    /// The slew that closes `advantage`: positive means the local side is
    /// ahead, so the clock slows down.
    #[must_use]
    pub fn for_advantage(advantage: i32) -> Self {
        let dead = if advantage.saturating_abs() <= SLEW_DEAD_ZONE_TICKS {
            0
        } else {
            advantage
        };
        let permille = dead
            .saturating_mul(SLEW_PER_ADVANTAGE_PERMILLE)
            .clamp(-MAX_SLEW_PERMILLE, MAX_SLEW_PERMILLE);
        Self {
            permille: -permille,
        }
    }

    /// The slew in permille of the tick rate: positive runs the clock
    /// faster, negative slower, zero holds it.
    #[must_use]
    pub const fn permille(&self) -> i32 {
        self.permille
    }

    /// The largest slew magnitude this controller can ask for, in permille.
    #[must_use]
    pub const fn cap_permille(&self) -> i32 {
        MAX_SLEW_PERMILLE
    }
}

/// The clock pacer: the client tick clock runs at `1 + permille / 1000`
/// ticks per frame, so a frame runs 0, 1 or 2 client ticks.
///
/// The pacer is a permille accumulator, so the rate is exact over a long
/// run and the remainder never grows. Every tick it hands out is the next
/// tick in sequence: the predicted stream stays contiguous and no frame is
/// skipped. The counters are the posted evidence: the largest slew the
/// pacer ran with and the frames that ran off the nominal rate.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct ClockPacer {
    accumulator: i32,
    max_permille: i32,
    slewed: u32,
}

impl ClockPacer {
    /// A pacer at the nominal rate with no history.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            accumulator: 0,
            max_permille: 0,
            slewed: 0,
        }
    }

    /// The ticks this frame should run: 0, 1 or 2.
    ///
    /// The accumulator keeps the fraction of a tick, so the long-run rate is
    /// exactly `1 + permille / 1000` ticks per frame. The caller runs the
    /// ticks in order; the pacer never skips one.
    pub fn pace(&mut self, slew: &ClockSlew) -> u32 {
        let total = self.accumulator + 1000 + slew.permille();
        let ticks = total.div_euclid(1000);
        self.accumulator = total.rem_euclid(1000);
        let magnitude = slew.permille().saturating_abs();
        self.max_permille = self.max_permille.max(magnitude);
        if ticks != 1 {
            self.slewed = self.slewed.saturating_add(1);
        }
        u32::try_from(ticks).unwrap_or(0)
    }

    /// The largest slew magnitude the pacer ran with, in permille.
    #[must_use]
    pub const fn max_permille(&self) -> i32 {
        self.max_permille
    }

    /// The frames that ran off the nominal one-tick rate.
    #[must_use]
    pub const fn slewed_frames(&self) -> u32 {
        self.slewed
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
    use engine_arena::{ArenaConfig, MatchSpec, DEFAULT_INPUT_DELAY_TICKS};
    use engine_input::FighterId;

    use crate::lobby::{BuildId, LobbyError};
    use crate::session::{Client, NetError, Server};

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
    fn window_averages_thirty_frames_and_ignores_one_spike() {
        // The window is the donor's: 30 samples, one per frame. A single
        // spike moves the average by one part in thirty, so the knob never
        // steps on one noisy sample.
        let mut window = AdvantageWindow::new();
        assert_eq!(window.filled(), 0);
        assert_eq!(window.average_frame_advantage(), 0);
        for _ in 0..WINDOW_FRAMES {
            window.advance(100, 100);
        }
        assert_eq!(window.filled(), WINDOW_FRAMES);
        assert_eq!(window.average_frame_advantage(), 0);
        // One spike of ten ticks: the average moves by ten parts in thirty.
        window.advance(110, 100);
        assert_eq!(window.average_frame_advantage(), 0);
        // Thirty spikes: the average reads the whole gap.
        for _ in 0..WINDOW_FRAMES {
            window.advance(110, 100);
        }
        assert_eq!(window.average_frame_advantage(), 10);
    }

    #[test]
    fn window_ring_keeps_the_last_thirty_samples() {
        let mut window = AdvantageWindow::new();
        for _ in 0..(WINDOW_FRAMES * 2) {
            window.advance(50, 40);
        }
        assert_eq!(window.filled(), WINDOW_FRAMES);
        assert_eq!(window.average_frame_advantage(), 10);
    }

    #[test]
    fn mirrored_windows_give_opposite_slews() {
        // The symmetry the row asks for: both peers sample the same frames,
        // so their windows are exact mirrors and their slews are equal and
        // opposite. The local side ahead slows its clock down; the remote
        // side, behind, speeds its clock up.
        let mut local = AdvantageWindow::new();
        let mut remote = AdvantageWindow::new();
        for step in 0..WINDOW_FRAMES {
            let mine = 100 + i32::try_from(step).unwrap_or(0);
            let theirs = 96 + i32::try_from(step).unwrap_or(0);
            local.advance(mine, theirs);
            remote.advance(theirs, mine);
        }
        assert_eq!(local.average_frame_advantage(), 4);
        assert_eq!(remote.average_frame_advantage(), -4);
        let mine = ClockSlew::for_advantage(local.average_frame_advantage());
        let theirs = ClockSlew::for_advantage(remote.average_frame_advantage());
        assert_eq!(mine.permille(), -40);
        assert_eq!(theirs.permille(), 40);
        assert_eq!(mine.permille(), -theirs.permille());
    }

    #[test]
    fn slew_holds_inside_the_dead_zone_and_stops_at_the_cap() {
        // The lightyear error margin: one tick of error is no error.
        assert_eq!(ClockSlew::for_advantage(0).permille(), 0);
        assert_eq!(ClockSlew::for_advantage(1).permille(), 0);
        assert_eq!(ClockSlew::for_advantage(-1).permille(), 0);
        // Ahead means slow down; behind means speed up.
        assert_eq!(ClockSlew::for_advantage(2).permille(), -20);
        assert_eq!(ClockSlew::for_advantage(-2).permille(), 20);
        // The cap is 5 percent, both ways.
        assert_eq!(ClockSlew::for_advantage(9).permille(), -MAX_SLEW_PERMILLE);
        assert_eq!(ClockSlew::for_advantage(-9).permille(), MAX_SLEW_PERMILLE);
        assert_eq!(
            ClockSlew::for_advantage(i32::MAX).permille(),
            -MAX_SLEW_PERMILLE
        );
        assert_eq!(
            ClockSlew::for_advantage(i32::MIN).permille(),
            MAX_SLEW_PERMILLE
        );
        assert_eq!(ClockSlew::new().permille(), 0);
        assert_eq!(ClockSlew::new().cap_permille(), MAX_SLEW_PERMILLE);
    }

    #[test]
    fn pacer_runs_the_nominal_rate_and_never_skips_a_tick() {
        // At the nominal rate the pacer runs exactly one tick per frame.
        let mut pacer = ClockPacer::new();
        let hold = ClockSlew::new();
        for _ in 0..100 {
            assert_eq!(pacer.pace(&hold), 1);
        }
        assert_eq!(pacer.slewed_frames(), 0);
        assert_eq!(pacer.max_permille(), 0);

        // A fast clock runs two ticks in one frame and none in the next:
        // the tick count over the run is exact, and no tick is missing.
        let mut fast = ClockPacer::new();
        let speed = ClockSlew::for_advantage(-5);
        let mut ticks = 0u32;
        for _ in 0..100 {
            ticks += fast.pace(&speed);
        }
        assert_eq!(ticks, (100 + 100 * speed.permille() / 1000) as u32);
        assert_eq!(fast.max_permille(), 50);
        assert!(fast.slewed_frames() > 0);

        // A slow clock runs one tick and holds one frame: the tick count
        // over the run is exact too, and the stream never has a hole.
        let mut slow = ClockPacer::new();
        let drag = ClockSlew::for_advantage(5);
        let mut ticks = 0u32;
        for _ in 0..100 {
            ticks += slow.pace(&drag);
        }
        assert_eq!(ticks, (100 + 100 * drag.permille() / 1000) as u32);
    }

    #[test]
    fn input_delay_sweep_posts_misprediction_and_resim_per_setting() {
        // QN2 receipt: delay 0..=8 over the WAN base latency with 30 ms
        // jitter at 0, 2 and 5 percent loss. Every cell converges to the
        // truth hash and more delay never predicts more frames. This static
        // profile alone does not pick the default: its deepest resim is
        // already inside the 9-tick window at delay 0. The default follows
        // from the 2-percent-loss soak profile in
        // `default_delay_is_the_smallest_with_no_resim_beyond_the_window_at_2_percent_loss`.
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

    #[test]
    fn default_delay_is_the_smallest_with_no_resim_beyond_the_window_at_2_percent_loss() {
        // QN2: the bar is "no resim beyond 9 ticks at 2 percent loss". The
        // 2-percent-loss profile the net layer soaks is the WAN one: 150 ms
        // round trip (9 ticks) plus 30 ms jitter (2 ticks), so a client's
        // staleness can reach RTT + jitter = 11 ticks (the bound
        // `wan_soak_jitter_stays_inside_the_rollback_window` pins) and the
        // resim distance is that staleness minus the delay. The smallest
        // delay that holds every resim inside the 9-tick window is
        // 11 - 9 = 2, the shipped default.
        let worst_staleness = crate::wan_soak::RTT_TICKS + crate::wan_soak::JITTER_TICKS;
        let window = crate::netcode::MAX_ROLLBACK;
        let window_ticks = u32::try_from(window).expect("the window fits u32");
        assert_eq!(worst_staleness - window_ticks, DEFAULT_DELAY);
        assert!(
            worst_staleness - (DEFAULT_DELAY - 1) > window_ticks,
            "one tick less delay resims past the window"
        );
        // The seeded loopback sweep at 2 percent loss agrees on the direction
        // and on the window: the default sits inside it, shallower than no
        // delay at all.
        let rows = sweep(2);
        let default_index = usize::try_from(DEFAULT_DELAY).expect("a tick count fits usize");
        assert!(default_index < rows.len(), "the sweep covers the default");
        let at_default = &rows[default_index];
        assert!(at_default.max_rollback <= window);
        assert!(at_default.max_rollback < rows[0].max_rollback);
        println!(
            "qn2 default delay {DEFAULT_DELAY} ticks: worst staleness {worst_staleness}, \
             window {window_ticks}; delay {DEFAULT_DELAY} max resim {} vs delay 0 {}",
            at_default.max_rollback, rows[0].max_rollback
        );
    }

    #[test]
    fn input_delay_sits_in_the_match_spec_and_bumps_the_wire_version_gate() {
        // The delay is not a local choice: both peers must hold it, so it
        // sits in the match spec. The version check the wire carries is the
        // versioned build id (executable version plus the match identity), so
        // a spec on another delay is another version of the match and its
        // peer is refused with both digests named, before tick 0.
        let config = ArenaConfig {
            seed: 0x5EE9,
            ticks: 24,
            fighters: 2,
            half_extent_m: 12.0,
            table_version: 1,
        };
        let base = MatchSpec::v1(config);
        assert_eq!(base.input_delay_ticks, DEFAULT_DELAY);
        assert_eq!(DEFAULT_INPUT_DELAY_TICKS, DEFAULT_DELAY);
        let slower = base.clone().with_input_delay(DEFAULT_DELAY + 1);
        assert_ne!(slower.identity(), base.identity(), "another match");
        let want = BuildId::of_spec(1, &base);
        let got = BuildId::of_spec(1, &slower);
        assert_eq!(
            got.check_against(want),
            Err(LobbyError::VersionMismatch {
                want_version: 1,
                got_version: 1,
                want_rules: base.identity(),
                got_rules: slower.identity(),
            })
        );
        // The knob stays inside MIN_DELAY..=MAX_DELAY: the net layer refuses
        // a spec outside it, and the default spec builds on both roles.
        assert!(Server::from_spec(base.clone()).is_ok());
        assert!(Client::from_spec(base, FighterId::new(0).expect("slot 0")).is_ok());
        let wild = slower.with_input_delay(MAX_DELAY + 1);
        assert!(matches!(
            Server::from_spec(wild.clone()),
            Err(NetError::BadConfig)
        ));
        assert!(matches!(
            Client::from_spec(wild, FighterId::new(0).expect("slot 0")),
            Err(NetError::BadConfig)
        ));
    }
}
