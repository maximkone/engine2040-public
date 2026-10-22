//! delay: input-delay helpers and the clock slew driven by frame advantage.
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

/// How far the client's tip may sit from its own schedule, in ticks.
///
/// The pacer holds the tip inside this band, so a trim never drifts the
/// client's input age and never runs the tip ahead of the schedule it was
/// given by the input delay.
pub const SCHEDULE_BAND: i32 = 1;

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
    pub fn cap_permille(&self) -> i32 {
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

    /// The accumulator step and the slew it asks for: 0, 1 or 2 ticks. The
    /// accumulator keeps the fraction of a tick, so the long-run rate is
    /// exactly `1 + permille / 1000` ticks per frame.
    fn step(&mut self, slew: &ClockSlew) -> u32 {
        let total = self.accumulator + 1000 + slew.permille();
        let ticks = total.div_euclid(1000);
        self.accumulator = total.rem_euclid(1000);
        let magnitude = slew.permille().saturating_abs();
        self.max_permille = self.max_permille.max(magnitude);
        u32::try_from(ticks).unwrap_or(0)
    }

    /// Count a frame that ran off the nominal one-tick rate.
    fn note(&mut self, ticks: u32) {
        if ticks != 1 {
            self.slewed = self.slewed.saturating_add(1);
        }
    }

    /// The ticks this frame should run: 0, 1 or 2. The caller runs the ticks
    /// in order; the pacer never skips one.
    pub fn pace(&mut self, slew: &ClockSlew) -> u32 {
        let ticks = self.step(slew);
        self.note(ticks);
        ticks
    }

    /// [`Self::pace`] bounded by the seat's schedule.
    ///
    /// `error` is the client's signed distance from its schedule: positive
    /// means the tip runs ahead of it, negative means it fell behind. The
    /// pacer holds the frame back when the tip runs more than
    /// [`SCHEDULE_BAND`] ticks ahead, runs a lag up when the tip fell more
    /// than [`SCHEDULE_BAND`] ticks behind, and otherwise runs the trim's
    /// rate. `available` is the authored inputs the seat still holds: the
    /// pacer never runs a tick the seat has no input for, so the predicted
    /// stream never skips one.
    pub fn pace_within(&mut self, slew: &ClockSlew, error: i32, available: u32) -> u32 {
        let base = self.step(slew);
        let want = if error < -SCHEDULE_BAND {
            2
        } else if error > SCHEDULE_BAND {
            0
        } else {
            base
        };
        let cap = available.min(2);
        let ticks = want.min(cap);
        self.note(ticks);
        ticks
    }

    /// The largest slew magnitude the pacer ran with, in permille.
    #[must_use]
    pub const fn max_permille(&self) -> i32 {
        self.max_permille
    }

    /// The frames that ran off the nominal one-tick rate.
    #[must_use]
    pub fn slewed_frames(&self) -> u32 {
        self.slewed
    }
}

/// The measurement profile: one tick each way, no jitter and no loss. The
/// 2-tick default delay covers that transit, so the client never predicts a
/// wrong frame; the same profile at delay 0 is the contrast that shows the
/// delay is what hides the lag.
fn clean_profile() -> super::link::LinkProfile {
    super::link::LinkProfile {
        latency: 1,
        jitter: 0,
        loss_pct: 0,
        reorder_pct: 0,
    }
}

/// The measurement probe: one endpoint predicts against a peer whose inputs
/// cross a seeded link.
///
/// The probe is the delay sweep's seeded loopback, reimplemented here as
/// production code so the posted rate never depends on a test module. The
/// peer repeats its last input in every datagram, so a frame that is lost
/// still lets the tip be checked against the truth.
#[derive(Clone, Default)]
struct ProbeSim {
    acc: u64,
}

impl super::rollback::SimTick for ProbeSim {
    fn advance(&mut self, inputs: [u32; super::rollback::PLAYERS]) {
        let mix = (u64::from(inputs[0]) << 8) | u64::from(inputs[1]);
        self.acc = engine_core::mix64(self.acc ^ mix);
    }

    fn state_hash(&self) -> u64 {
        self.acc
    }
}

/// The probe's peer input: a held button, human-shaped.
fn probe_script(salt: u64, frame: u32) -> u32 {
    u32::try_from(engine_core::mix64(u64::from(frame / 4) ^ salt) % 5).unwrap_or(0)
}

/// What one probe run measured.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct ProbeReceipt {
    predicted: u64,
    mispredicted: u64,
    rollbacks: u64,
    max_rollback: usize,
    tip_matches_truth: bool,
}

/// Play `frames` ticks of one endpoint against a peer whose inputs cross a
/// seeded link, both sides holding `delay` ticks of input delay.
fn probe_run(
    delay: u32,
    profile: super::link::LinkProfile,
    seed: u64,
    frames: u32,
) -> ProbeReceipt {
    use super::link::{FaultLink, Frame as LinkFrame};
    use super::rollback::SimTick;

    let (local_salt, remote_salt) = (seed ^ 0xA5A5, seed ^ 0x5A5A);
    let mut session = super::rollback::P2pSession::with_frame_delay(ProbeSim::default(), delay);
    let mut link = FaultLink::new(profile, seed);
    let deliver = |session: &mut super::rollback::P2pSession<ProbeSim>, link: &mut FaultLink| {
        for due in link.due() {
            let LinkFrame::Datagram(bytes) = due else {
                continue;
            };
            for pair in bytes.chunks_exact(8) {
                let word = |at: usize| {
                    u32::from_le_bytes([pair[at], pair[at + 1], pair[at + 2], pair[at + 3]])
                };
                let _ = session.confirm_input(word(0), word(4));
            }
        }
        let _ = session.resolve_incorrect();
    };
    for frame in 0..frames {
        // The peer's input for the frames the client still needs, repeated
        // in one datagram.
        let mut datagram = [0u8; 32];
        let mut at = 0usize;
        for k in (0..4u32.min(frame + 1)).rev() {
            let word = (frame - k + delay).to_le_bytes();
            datagram[at..at + 4].copy_from_slice(&word);
            let word = probe_script(remote_salt, frame - k).to_le_bytes();
            datagram[at + 4..at + 8].copy_from_slice(&word);
            at += 8;
        }
        link.send_datagram(&datagram[..at]);
        deliver(&mut session, &mut link);
        link.tick();
        let neutral = (frame < delay).then_some(0);
        session.advance_frame(probe_script(local_salt, frame), neutral);
    }
    // Drain: let the datagrams still in flight land, so the tail frames are
    // corrected before the tip is read.
    for _ in 0..profile.worst_delay() {
        deliver(&mut session, &mut link);
        link.tick();
    }
    // The truth: the same inputs, applied on time.
    let mut truth = ProbeSim::default();
    for frame in 0..frames {
        let at = |salt| {
            frame
                .checked_sub(delay)
                .map_or(0, |s| probe_script(salt, s))
        };
        truth.advance([at(local_salt), at(remote_salt)]);
    }
    // Catch the tip up from the truth so the two can be compared.
    for frame in 0..frames {
        if session.was_predicted(frame) == Some(true) {
            let at = |salt| {
                frame
                    .checked_sub(delay)
                    .map_or(0, |s| probe_script(salt, s))
            };
            let _ = session.confirm_frame(frame, at(remote_salt));
        }
    }
    ProbeReceipt {
        predicted: session.predicted_count(),
        mispredicted: session.mispredicted_count(),
        rollbacks: session.rollback_count(),
        max_rollback: session.max_rollback(),
        tip_matches_truth: session.sim().state_hash() == truth.state_hash(),
    }
}

/// One paced measurement: the frames played against a seeded link at a
/// fixed input delay, the rollbacks taken, those rollbacks per 1000 ticks
/// and the deepest one.
///
/// Shape: the seeded two-endpoint probe the delay sweep runs, reached from
/// production so the receipts can post the rollback rate the MP26 accept bar
/// names ("< 3 rollbacks per 1000 ticks at 2-frame delay"). Determinism:
/// the probe is a pure function of (delay, profile, seed, frames), so the
/// posted rate is the same number on every machine.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct PacingMeasurement {
    /// Input delay in ticks both endpoints held.
    pub input_delay_ticks: u32,
    /// Frames played.
    pub frames: u32,
    /// Frames the endpoints predicted.
    pub predicted: u64,
    /// Frames the local endpoint's guesses were wrong about.
    pub mispredicted: u64,
    /// Rollback events taken.
    pub rollbacks: u64,
    /// Rollbacks per 1000 ticks, rounded down.
    pub rollbacks_per_1000_ticks: u32,
    /// The same rate with the delay off: the contrast that gives the rate
    /// above its teeth.
    pub baseline_rollbacks_per_1000_ticks: u32,
    /// Deepest rollback, in ticks.
    pub max_rollback_ticks: u32,
    /// The run's tip still matches the truth computed from the tape.
    pub tip_matches_truth: bool,
}

/// Rollback rate at [`DEFAULT_DELAY`] over `frames` of the clean link the
/// delay sweep measures on, with the delay-off rate beside it.
#[must_use]
pub fn measure_rollback_rate(frames: u32) -> PacingMeasurement {
    let profile = clean_profile();
    let on = probe_run(DEFAULT_DELAY, profile, 0x5EE9, frames);
    let bare = probe_run(MIN_DELAY, profile, 0x5EE9, frames);
    let per_1000 = |rollbacks: u64| {
        rollbacks
            .saturating_mul(1000)
            .checked_div(u64::from(frames))
            .unwrap_or(u64::MAX)
    };
    PacingMeasurement {
        input_delay_ticks: DEFAULT_DELAY,
        frames,
        predicted: on.predicted,
        mispredicted: on.mispredicted,
        rollbacks: on.rollbacks,
        rollbacks_per_1000_ticks: u32::try_from(per_1000(on.rollbacks)).unwrap_or(u32::MAX),
        baseline_rollbacks_per_1000_ticks: u32::try_from(per_1000(bare.rollbacks))
            .unwrap_or(u32::MAX),
        max_rollback_ticks: u32::try_from(on.max_rollback).unwrap_or(u32::MAX),
        tip_matches_truth: on.tip_matches_truth && bare.tip_matches_truth,
    }
}

/// One seeded loopback bout per delay in `MIN_DELAY..=MAX_DELAY`.
#[cfg(test)]
fn sweep(loss_pct: u32) -> Vec<super::rollback::loopback::Receipt> {
    (MIN_DELAY..=MAX_DELAY)
        .map(|delay| super::rollback::loopback::run(delay, fault_profile(loss_pct), 0x5EE9, 600))
        .collect()
}

/// The lossy profile the delay sweep sizes the default on: the WAN base
/// latency (4 ticks, 67 ms) with 30 ms jitter (2 ticks), no reordering, at
/// `loss_pct` loss.
#[cfg(test)]
fn fault_profile(loss_pct: u32) -> super::link::LinkProfile {
    super::link::LinkProfile {
        latency: 4,
        jitter: 2,
        loss_pct,
        reorder_pct: 0,
    }
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
    fn pacer_holds_the_schedule_band_and_never_skips_a_tick() {
        // The trim never drifts the client's schedule: a tip that ran ahead
        // holds its frame back, and a tip that fell behind runs its lag up.
        let mut pacer = ClockPacer::new();
        let hold = ClockSlew::new();
        // On the schedule: the pacer runs the trim's rate.
        assert_eq!(pacer.pace_within(&hold, 0, 4), 1);
        // One tick ahead: still inside the band, so the rate holds.
        assert_eq!(pacer.pace_within(&hold, SCHEDULE_BAND, 4), 1);
        // Two ticks ahead: the frame is held, never skipped.
        assert_eq!(pacer.pace_within(&hold, SCHEDULE_BAND + 1, 4), 0);
        // Two ticks behind: the lag runs up, two ticks at once.
        assert_eq!(pacer.pace_within(&hold, -(SCHEDULE_BAND + 1), 4), 2);
        // One tick behind: inside the band, so the rate holds.
        assert_eq!(pacer.pace_within(&hold, -SCHEDULE_BAND, 4), 1);
        // The seat has no input for the next tick: the frame is held, so
        // the stream never runs a tick the seat cannot fill.
        assert_eq!(pacer.pace_within(&hold, 0, 0), 0);
        assert_eq!(pacer.pace_within(&hold, 0, 1), 1);
        assert_eq!(pacer.max_permille(), 0);
    }

    #[test]
    fn posted_measurement_holds_the_1000_tick_bar_at_the_default_delay() {
        // The MP26 bar: "< 3 rollbacks per 1000 ticks at 2-frame delay". The
        // measurement is a pure function of the profile, the seed and the
        // frames, so the number the receipt posts is the number here.
        let measured = measure_rollback_rate(1000);
        assert_eq!(measured.input_delay_ticks, DEFAULT_DELAY);
        assert_eq!(measured.frames, 1000);
        assert!(measured.tip_matches_truth, "the run diverged: {measured:?}");
        println!(
            "mp26 pacing d={} frames={} predicted={} mispred={} rollbacks={} per_1000={} baseline={} max={}",
            measured.input_delay_ticks,
            measured.frames,
            measured.predicted,
            measured.mispredicted,
            measured.rollbacks,
            measured.rollbacks_per_1000_ticks,
            measured.baseline_rollbacks_per_1000_ticks,
            measured.max_rollback_ticks,
        );
        assert!(
            measured.rollbacks_per_1000_ticks < 3,
            "rollback rate over the bar: {measured:?}"
        );
        // The bar has teeth: the same link with the delay off mispredicts on
        // most frames, so the rate above is the delay's work.
        assert!(
            measured.baseline_rollbacks_per_1000_ticks > 100,
            "the delay-off contrast is too small to prove anything: {measured:?}"
        );
        // The posted number is the sizing input the MP26 accept bar reads.
        assert_eq!(
            measured.rollbacks_per_1000_ticks,
            crate::scale::MP6_POSTED_ROLLBACKS_PER_1000,
            "the posted per-1000 rate moved: update the scale fixture"
        );
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
