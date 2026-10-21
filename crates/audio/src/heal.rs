//! heal: default-device loss/change recovery through the observe supervisor.
//!
//! Consumer: the windowed game keeps playing across device changes; its
//! windowed leg reopens through [`DeviceHeal::reopen_live`] (or its own opener via
//! [`DeviceHeal::try_reopen`]) while the headless [`Mixer`](crate::Mixer)
//! keeps rendering. Data owner: [`DeviceHeal`] owns the supervisor, ring,
//! state, silent-frame count and latest notice; the caller owns the mixer.
//! Units: backoff in ms, silent mixer frames. Determinism: notices derive
//! from ticks, attempts and frames only, never the clock. Errors: typed
//! [`AudioHealError`]; over-budget faults quarantine loudly with a notice.
//!
//! [`HoldLastMixer`] is the callback side of the same heal: when the game
//! thread holds the shared mixer the chunk repeats the last rendered frame
//! (repeat-last concealment) instead of dropping to silence, so the speaker
//! sees no step at the start of the gap. It copies one stereo frame, waits
//! for no lock and allocates nothing.

use std::sync::{Arc, Mutex};

use engine_observe::heal::HealError;
use engine_observe::recovery::{HealValidation, VerifiedRecovery};
use engine_observe::supervisor::{SupervisedDecision, Supervisor};
use engine_observe::{EventCode, RingBuffer};

use crate::{FrameSource, Mixer};

/// Fault row audio device loss/change reports through: the catalog's
/// device-reconnect row (reconnect, 3 attempts, 100-2000 ms backoff). The
/// observe registry has no audio-specific code, so audio reuses this row;
/// the heal kind and budgets are what the wiring depends on.
pub const DEVICE_FAULT_CODE: EventCode = EventCode::RENDER_DEVICE_LOST;

/// Ring capacity for `heal.applied` / `heal.failed` evidence.
pub const HEAL_RING_CAP: usize = 64;

/// Cap for human-readable notices and recovery reports.
pub const MAX_NOTICE_LEN: usize = 256;

/// Typed audio-heal failure. Loss/change itself is not an error: it degrades
/// to the silent mixer with a backoff. Errors surface only when a reopen is
/// refused, the budget is spent, or the supervisor tape needs exporting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AudioHealError {
    /// Reopen refused while budget remains; wait `backoff_ms`, keep mixing,
    /// then retry. `attempt` is the one-based fault count so far.
    ReopenFailed {
        /// Backoff the supervisor advised for the next attempt, in ms.
        backoff_ms: u64,
        /// One-based fault count so far.
        attempt: u32,
    },
    /// Budget spent: the device stays quarantined on the silent mixer and
    /// the loud notice names the fault count. Recovery needs a fresh heal
    /// after the operator notice is acknowledged.
    Quarantined {
        /// Total fault count that spent the budget.
        attempts: u32,
    },
    /// Supervisor capacity failure (tape full): export history, then retry.
    /// Consumes no attempt budget.
    Supervisor(HealError),
}

impl std::fmt::Display for AudioHealError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ReopenFailed {
                backoff_ms,
                attempt,
            } => write!(
                f,
                "audio: device reopen refused (attempt {attempt}); retry after {backoff_ms}ms, mixer keeps rendering"
            ),
            Self::Quarantined { attempts } => write!(
                f,
                "audio: device quarantined after {attempts} faults; silent mixer, operator notice raised"
            ),
            Self::Supervisor(err) => write!(f, "audio: supervisor fault: {err}"),
        }
    }
}

impl std::error::Error for AudioHealError {}

impl From<HealError> for AudioHealError {
    fn from(err: HealError) -> Self {
        Self::Supervisor(err)
    }
}

/// Device playback state. The mixer runs in every state; `Silent` and
/// `Quarantined` render into the void while counting silent frames.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeviceState {
    /// Device open; frames reach the speakers.
    Live,
    /// Device down; silent mixer keeps rendering within budget.
    Silent,
    /// Budget spent; silent mixer keeps rendering, notice raised loudly.
    Quarantined,
}

/// Recovery report returned when a reopen succeeds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HealReport {
    /// Tick the reopen succeeded at.
    pub tick: u64,
    /// Faults seen before this recovery (budget spent, then reset).
    pub attempts: u32,
    /// Last backoff advised before recovery, in ms.
    pub backoff_ms: u64,
    /// Mixer frames rendered while the device was down.
    pub silent_frames: u64,
    /// Human-readable recovery line, bounded to [`MAX_NOTICE_LEN`].
    pub message: String,
}

/// Supervisor-backed device heal. Loss/change reports route to the catalog
/// reconnect policy (backoff, degrade to the silent mixer); each refused
/// reopen escalates the backoff; a verified reopen resets the budget and
/// reports; past the budget the device quarantines loudly. The mixer is
/// caller-owned and never stopped: [`Self::render_silent_frames`] advances
/// it while the device is down.
#[derive(Clone, Debug)]
pub struct DeviceHeal {
    supervisor: Supervisor,
    ring: RingBuffer,
    state: DeviceState,
    silent_frames: u64,
    notice: String,
}

impl DeviceHeal {
    /// Fresh heal: device live, empty ring, full reconnect budget.
    #[must_use]
    pub fn new() -> Self {
        Self {
            supervisor: Supervisor::new(),
            ring: RingBuffer::new(HEAL_RING_CAP),
            state: DeviceState::Live,
            silent_frames: 0,
            notice: "audio live: device open".to_owned(),
        }
    }

    /// Current playback state.
    #[must_use]
    pub fn state(&self) -> DeviceState {
        self.state
    }

    /// True while frames reach the speakers.
    #[must_use]
    pub fn is_live(&self) -> bool {
        self.state == DeviceState::Live
    }

    /// True once the budget is spent and the loud notice is raised.
    #[must_use]
    pub fn is_quarantined(&self) -> bool {
        self.state == DeviceState::Quarantined
    }

    /// Mixer frames rendered while the device was down. Monotonic.
    #[must_use]
    pub fn silent_frames(&self) -> u64 {
        self.silent_frames
    }

    /// Faults seen under the current budget (zero after recovery).
    #[must_use]
    pub fn attempts(&self) -> u32 {
        self.supervisor.attempts(DEVICE_FAULT_CODE)
    }

    /// Backoff advised for the next attempt, in ms (zero when live).
    #[must_use]
    pub fn next_backoff_ms(&self) -> u64 {
        self.supervisor
            .escalation_of(DEVICE_FAULT_CODE)
            .map_or(0, |escalation| escalation.backoff_ms)
    }

    /// Latest human-readable status: live line, degrade line, recovery
    /// report or quarantine notice. Bounded to [`MAX_NOTICE_LEN`].
    #[must_use]
    pub fn notice(&self) -> &str {
        &self.notice
    }

    /// Heal tape rows recorded so far (evidence, never dropped silently).
    #[must_use]
    pub fn tape_len(&self) -> usize {
        self.supervisor.tape_len()
    }

    /// Report a lost default device: supervisor reconnect advice with
    /// backoff, state to silent, mixer untouched. Past the budget this
    /// quarantines loudly instead.
    ///
    /// # Errors
    ///
    /// Returns [`AudioHealError::Quarantined`] past the budget and
    /// [`AudioHealError::Supervisor`] when the tape needs exporting.
    pub fn report_loss(&mut self, tick: u64) -> Result<SupervisedDecision, AudioHealError> {
        self.fault(tick, "default device lost")
    }

    /// Report a default-device change (headphones plugged, HDMI switched):
    /// same reconnect policy as a loss, with its own notice wording.
    ///
    /// # Errors
    ///
    /// Same as [`Self::report_loss`].
    pub fn report_change(&mut self, tick: u64) -> Result<SupervisedDecision, AudioHealError> {
        self.fault(tick, "default device changed")
    }

    /// Advance `mixer` by `frames` stereo frames while the device is down.
    /// Output is discarded (silence on the wire) but voices advance exactly
    /// as a live callback would, so recovery resumes mid-phrase. Counts into
    /// [`Self::silent_frames`] only while silent or quarantined; returns the
    /// cumulative silent-frame count. Never panics, never blocks.
    pub fn render_silent_frames(&mut self, mixer: &mut Mixer, frames: usize) -> u64 {
        for _ in 0..frames {
            let _ = mixer.render_frame_stereo();
        }
        self.count_silent_frames(frames)
    }

    /// Count `frames` the device never heard, without rendering anything: an
    /// owner that renders into its own buffer and then drops it (the audio
    /// engine keeps voices and the music bed advancing) counts the frames
    /// here instead of rendering a second time. Counts only while silent or
    /// quarantined; returns the cumulative silent-frame count.
    pub fn count_silent_frames(&mut self, frames: usize) -> u64 {
        if self.state != DeviceState::Live {
            let delta = u64::try_from(frames).unwrap_or(u64::MAX);
            self.silent_frames = self.silent_frames.saturating_add(delta);
        }
        self.silent_frames
    }

    /// Digest of the heal's own state (device state plus silent frames): the
    /// before/after pair a recovery proof carries. Deterministic integer
    /// math over two fields, no clock.
    #[must_use]
    pub fn state_digest(&self) -> u64 {
        let mut hasher = engine_core::Fnv64::new();
        hasher.write_u64(match self.state {
            DeviceState::Live => 0,
            DeviceState::Silent => 1,
            DeviceState::Quarantined => 2,
        });
        hasher.write_u64(self.silent_frames);
        hasher.finish()
    }

    /// Try `opener` once: on success reset the budget, go live and report
    /// the recovery; on refusal escalate through the supervisor (backoff or
    /// loud quarantine). The opener is caller-supplied so tests stay
    /// simulated; see [`Self::reopen_live`] for the real cpal opener.
    ///
    /// # Errors
    ///
    /// Returns [`AudioHealError::ReopenFailed`] while budget remains,
    /// [`AudioHealError::Quarantined`] past it, and
    /// [`AudioHealError::Supervisor`] when the tape needs exporting.
    pub fn try_reopen<T, E>(
        &mut self,
        tick: u64,
        opener: impl FnOnce() -> Result<T, E>,
    ) -> Result<(T, HealReport), AudioHealError> {
        match opener() {
            Ok(device) => {
                let attempts = self.supervisor.attempts(DEVICE_FAULT_CODE);
                let backoff_ms = self.next_backoff_ms();
                let before = self.state_digest();
                self.state = DeviceState::Live;
                let after = self.state_digest();
                // The budget is only cleared by a proved recovery: the opener
                // handed the device back, so the heal carries that proof. A
                // refused proof keeps the budget spent and says so in the
                // notice, so the refusal is never silent.
                let proof = VerifiedRecovery::new(
                    DEVICE_FAULT_CODE,
                    attempts.max(1),
                    HealValidation::Recovered,
                    before,
                    after,
                );
                let cleared = self.supervisor.reset_verified(&proof).is_ok();
                let message = bound_notice(&format!(
                    "audio recovered: device reopened at tick {tick} after {attempts} faults (last backoff {backoff_ms}ms); mixer rendered {} silent frames while down{}",
                    self.silent_frames,
                    if cleared {
                        ""
                    } else {
                        "; the reconnect budget was NOT cleared"
                    }
                ));
                self.notice.clone_from(&message);
                Ok((
                    device,
                    HealReport {
                        tick,
                        attempts,
                        backoff_ms,
                        silent_frames: self.silent_frames,
                        message,
                    },
                ))
            }
            Err(_) => match self.fault(tick, "device reopen refused") {
                Ok(advised) => Err(AudioHealError::ReopenFailed {
                    backoff_ms: advised.backoff_ms,
                    attempt: advised.attempt,
                }),
                Err(quarantined) => Err(quarantined),
            },
        }
    }

    /// Reopen the real default device through the supervisor: pulls stereo
    /// frames from `source` on the audio callback once open. Headless hosts
    /// observe `Err` (typed refusal or quarantine) and keep mixing offline.
    ///
    /// # Errors
    ///
    /// Same as [`Self::try_reopen`]; a missing device degrades, it never
    /// panics.
    #[cfg(feature = "device")]
    pub fn reopen_live<S: crate::FrameSource>(
        &mut self,
        tick: u64,
        source: S,
    ) -> Result<(crate::device::DeviceOutput, HealReport), AudioHealError> {
        self.try_reopen(tick, || crate::device::DeviceOutput::open_default(source))
    }

    /// Record one device fault with the supervisor and route its decision.
    fn fault(&mut self, tick: u64, reason: &str) -> Result<SupervisedDecision, AudioHealError> {
        let decision = self
            .supervisor
            .report_fault(DEVICE_FAULT_CODE, tick, &mut self.ring)?;
        self.advise(tick, decision, reason)
    }

    /// Route one supervisor decision to state plus notice: reconnect advice
    /// degrades to the silent mixer, an exhausted budget quarantines loudly.
    fn advise(
        &mut self,
        tick: u64,
        decision: SupervisedDecision,
        reason: &str,
    ) -> Result<SupervisedDecision, AudioHealError> {
        if decision.is_quarantine() {
            self.state = DeviceState::Quarantined;
            self.notice = bound_notice(&format!(
                "audio quarantined: {reason} at tick {tick} after {} faults; silent mixer keeps rendering, operator notice raised",
                decision.attempt
            ));
            return Err(AudioHealError::Quarantined {
                attempts: decision.attempt,
            });
        }
        self.state = DeviceState::Silent;
        let budget = self
            .supervisor
            .policy(DEVICE_FAULT_CODE)
            .map_or(0, |policy| policy.attempts_max);
        self.notice = bound_notice(&format!(
            "audio degraded: {reason} at tick {tick} (attempt {}/{}; backoff {}ms); silent mixer keeps rendering",
            decision.attempt, budget, decision.backoff_ms
        ));
        Ok(decision)
    }
}

impl Default for DeviceHeal {
    /// Fresh heal with a full reconnect budget.
    fn default() -> Self {
        Self::new()
    }
}

/// Device-callback source over the shared mixer that hides lock-contention
/// gaps. An uncontended chunk renders the mixer and remembers its last
/// stereo frame; a contended chunk (the game thread holds the lock, an XRUN
/// gap) repeats that frame instead of writing silence, so the entry edge of
/// the gap is zero rather than the full last sample. The callback never
/// waits and never allocates. Replaces the bare `Arc<Mutex<Mixer>>` source
/// for the windowed path; the bare impl still renders silence.
pub struct HoldLastMixer {
    mixer: Arc<Mutex<Mixer>>,
    last: [f32; 2],
    gaps: u64,
}

impl HoldLastMixer {
    /// Wrap the shared mixer; the first gap before any render holds silence.
    #[must_use]
    pub fn new(mixer: Arc<Mutex<Mixer>>) -> Self {
        Self {
            mixer,
            last: [0.0; 2],
            gaps: 0,
        }
    }

    /// Chunks concealed so far. Monotonic.
    #[must_use]
    pub fn gaps(&self) -> u64 {
        self.gaps
    }
}

impl FrameSource for HoldLastMixer {
    fn render_stereo_into(&mut self, out: &mut [f32]) {
        if let Ok(mut mixer) = self.mixer.try_lock() {
            mixer.render_stereo_into(out);
            if let Some(frame) = out.rchunks_exact(2).next() {
                self.last = [frame[0], frame[1]];
            }
            return;
        }
        self.gaps = self.gaps.saturating_add(1);
        let mut frames = out.chunks_exact_mut(2);
        for frame in frames.by_ref() {
            frame.copy_from_slice(&self.last);
        }
        frames.into_remainder().fill(0.0);
    }
}

/// Truncate a notice to [`MAX_NOTICE_LEN`] bytes on a `char` boundary.
fn bound_notice(text: &str) -> String {
    text[..text.floor_char_boundary(MAX_NOTICE_LEN)].to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Pcm;
    use engine_observe::heal::HealKind;
    use engine_observe::supervisor::backoff_ms;

    fn tone() -> Pcm {
        Pcm::new(8_000, 1, vec![0.25, 0.5, -0.25, -0.5])
    }

    fn policy_backoff(heal: &DeviceHeal, attempt: u32) -> u64 {
        let policy = heal
            .supervisor
            .policy(DEVICE_FAULT_CODE)
            .expect("catalog must hold the device row");
        backoff_ms(policy.backoff_base_ms, policy.backoff_cap_ms, attempt)
    }

    #[test]
    fn loss_degrades_with_reconnect_backoff_and_mixer_keeps_running() {
        let mut heal = DeviceHeal::new();
        assert!(heal.is_live());
        let decision = heal.report_loss(10).expect("first loss must heal");
        assert_eq!(decision.kind, HealKind::Reconnect);
        assert_eq!(decision.attempt, 1);
        assert_eq!(decision.backoff_ms, policy_backoff(&heal, 1));
        assert_eq!(decision.backoff_ms, 100);
        assert!(!decision.is_quarantine());
        assert_eq!(heal.state(), DeviceState::Silent);
        assert!(heal.notice().contains("degraded"));
        assert!(heal.notice().contains("mixer keeps rendering"));

        // Mixer keeps running through the outage: voices advance to idle
        // while every frame counts as silent.
        let mut mixer = Mixer::new(8_000);
        mixer.play(&tone(), 1.0);
        assert_eq!(heal.render_silent_frames(&mut mixer, 4), 4);
        assert!(mixer.is_idle(), "silent render advances voices like live");
        assert_eq!(heal.render_silent_frames(&mut mixer, 6), 10);
        assert_eq!(heal.silent_frames(), 10);
        assert_eq!(heal.next_backoff_ms(), 100);
        assert_eq!(heal.tape_len(), 1);
    }

    #[test]
    fn device_change_routes_to_the_same_reconnect_policy() {
        let mut heal = DeviceHeal::new();
        let decision = heal.report_change(7).expect("change must heal");
        assert_eq!(decision.kind, HealKind::Reconnect);
        assert_eq!(decision.attempt, 1);
        assert_eq!(decision.backoff_ms, 100);
        assert_eq!(heal.state(), DeviceState::Silent);
        assert!(heal.notice().contains("changed"));
    }

    #[test]
    fn refused_reopens_escalate_backoff_200_then_400ms() {
        let mut heal = DeviceHeal::new();
        heal.report_loss(1).expect("loss degrades");
        let mut mixer = Mixer::new(8_000);
        mixer.play(&tone(), 1.0);

        let err = heal
            .try_reopen(2, || Result::<(), &str>::Err("busy"))
            .expect_err("refused reopen must fail within budget");
        assert_eq!(
            err,
            AudioHealError::ReopenFailed {
                backoff_ms: policy_backoff(&heal, 2),
                attempt: 2
            }
        );
        assert_eq!(
            err,
            AudioHealError::ReopenFailed {
                backoff_ms: 200,
                attempt: 2
            }
        );
        assert_eq!(heal.state(), DeviceState::Silent);

        // Mixer frames between attempts: still rendering while down.
        mixer.play(&tone(), 1.0);
        heal.render_silent_frames(&mut mixer, 4);

        let err = heal
            .try_reopen(3, || Result::<(), &str>::Err("busy"))
            .expect_err("second refusal must escalate");
        assert_eq!(
            err,
            AudioHealError::ReopenFailed {
                backoff_ms: 400,
                attempt: 3
            }
        );
        assert_eq!(heal.attempts(), 3);
        assert_eq!(heal.next_backoff_ms(), 400);
        assert!(heal.silent_frames() >= 4);
    }

    #[test]
    fn budget_spent_quarantines_loudly_with_notice() {
        let mut heal = DeviceHeal::new();
        heal.report_loss(1).expect("loss degrades");
        for tick in 2..=3 {
            let _ = heal.try_reopen(tick, || Result::<(), &str>::Err("gone"));
        }
        let err = heal
            .try_reopen(4, || Result::<(), &str>::Err("gone"))
            .expect_err("third refusal spends the 3-attempt budget");
        assert_eq!(err, AudioHealError::Quarantined { attempts: 4 });
        assert!(heal.is_quarantined());
        assert!(heal.notice().contains("quarantined"));
        assert!(heal.notice().contains("operator notice"));
        // Loud, never silent: a heal.failed row is taped.
        assert!(
            heal.ring
                .query(Some(engine_observe::EventCode::HEAL_FAILED))
                .len()
                == 1
        );
        // Further faults stay quarantined with a typed error, not a reopen.
        // Fast reject inside the cooldown keeps the quarantine count at the
        // budget edge (attempts_max + 1 = 4), it never increments past it.
        let err = heal
            .report_loss(5)
            .expect_err("quarantined loss must error");
        assert_eq!(err, AudioHealError::Quarantined { attempts: 4 });
    }

    #[test]
    fn verified_reopen_recovers_reports_and_resets_budget() {
        let mut heal = DeviceHeal::new();
        heal.report_loss(1).expect("loss degrades");
        let mut mixer = Mixer::new(8_000);
        mixer.play(&tone(), 1.0);
        heal.render_silent_frames(&mut mixer, 8);

        let (device, report) = heal
            .try_reopen(9, || Result::<&str, &str>::Ok("stream"))
            .expect("verified reopen must recover");
        assert_eq!(device, "stream");
        assert!(heal.is_live());
        assert_eq!(report.tick, 9);
        assert_eq!(report.attempts, 1);
        assert_eq!(report.backoff_ms, 100);
        assert_eq!(report.silent_frames, 8);
        assert!(report.message.contains("recovered"));
        assert!(report.message.contains("8 silent frames"));
        assert_eq!(heal.notice(), report.message.as_str());
        // Fresh budget: the next loss is attempt 1 with the base backoff.
        assert_eq!(heal.attempts(), 0);
        let decision = heal.report_loss(20).expect("fresh budget heals");
        assert_eq!(decision.attempt, 1);
        assert_eq!(decision.backoff_ms, 100);
    }

    #[test]
    fn same_loss_sequence_twice_gives_same_backoffs_and_notices() {
        fn sequence() -> (Vec<u64>, Vec<String>, u64) {
            let mut heal = DeviceHeal::new();
            let mut backoffs = Vec::new();
            let mut mixer = Mixer::new(8_000);
            mixer.play(&tone(), 1.0);
            backoffs.push(heal.report_loss(1).expect("loss").backoff_ms);
            heal.render_silent_frames(&mut mixer, 8);
            for tick in 2..=3 {
                let err = heal
                    .try_reopen(tick, || Result::<(), &str>::Err("busy"))
                    .expect_err("refused");
                if let AudioHealError::ReopenFailed { backoff_ms, .. } = err {
                    backoffs.push(backoff_ms);
                }
            }
            let notices = heal
                .ring
                .query(None)
                .iter()
                .map(|event| event.message.clone())
                .collect();
            (backoffs, notices, heal.silent_frames())
        }
        let (backoffs, notices, frames) = sequence();
        assert_eq!(backoffs, vec![100, 200, 400]);
        assert_eq!((backoffs.clone(), notices.clone(), frames), {
            let again = sequence();
            (again.0, again.1, again.2)
        });
        assert_eq!(frames, 8);
    }

    #[test]
    fn live_render_never_counts_silent_frames() {
        let mut heal = DeviceHeal::new();
        let mut mixer = Mixer::new(8_000);
        mixer.play(&tone(), 1.0);
        assert_eq!(heal.render_silent_frames(&mut mixer, 4), 0);
        assert_eq!(heal.silent_frames(), 0);
        assert!(mixer.is_idle(), "frames still advance the mixer while live");
    }

    #[test]
    fn errors_are_typed_and_printable() {
        for err in [
            AudioHealError::ReopenFailed {
                backoff_ms: 200,
                attempt: 2,
            },
            AudioHealError::Quarantined { attempts: 4 },
            AudioHealError::Supervisor(HealError::TapeFull),
        ] {
            assert!(!err.to_string().is_empty(), "{err:?} must describe itself");
        }
        let _: &dyn std::error::Error = &AudioHealError::Quarantined { attempts: 4 };
        assert_eq!(
            AudioHealError::from(HealError::TapeFull),
            AudioHealError::Supervisor(HealError::TapeFull)
        );
        assert!(DeviceHeal::default().is_live());
    }

    /// 8 kHz, 1 kHz sine at 0.5: eight frames per period, near a peak at the
    /// frames that precede a gap below.
    fn sine_8k() -> Pcm {
        let samples = (0..96)
            .map(|k| 0.5 * (std::f32::consts::TAU * (k as f32 + 2.5) / 8.0).sin())
            .collect();
        Pcm::new(8_000, 1, samples)
    }

    const BLOCK: usize = 4;
    /// Three 4-frame gaps (blocks 2, 5 and 8 of 10).
    const GAP_BLOCKS: [usize; 3] = [2, 5, 8];

    /// Render ten 4-frame blocks, holding the mixer lock on the gap blocks.
    /// Returns the left channel and, per gap, the jump from the frame before
    /// the gap to its first frame.
    fn run_gapped(
        source: &mut impl FrameSource,
        shared: &Arc<Mutex<Mixer>>,
    ) -> (Vec<f32>, Vec<f32>) {
        let mut left: Vec<f32> = Vec::new();
        let mut block = [0.0f32; BLOCK * 2];
        for index in 0..10 {
            let guard = GAP_BLOCKS
                .contains(&index)
                .then(|| shared.lock().expect("lock"));
            source.render_stereo_into(&mut block);
            drop(guard);
            left.extend(block.iter().step_by(2));
        }
        let edges = GAP_BLOCKS
            .iter()
            .map(|b| (left[b * BLOCK] - left[b * BLOCK - 1]).abs())
            .collect();
        (left, edges)
    }

    /// What the callback did before: a contended chunk writes zeros.
    struct SilentOnGap(Arc<Mutex<Mixer>>);

    impl FrameSource for SilentOnGap {
        fn render_stereo_into(&mut self, out: &mut [f32]) {
            match self.0.try_lock() {
                Ok(mut mixer) => mixer.render_stereo_into(out),
                Err(_) => out.fill(0.0),
            }
        }
    }

    fn shared_with_tone() -> Arc<Mutex<Mixer>> {
        let shared = Arc::new(Mutex::new(Mixer::new(8_000)));
        shared.lock().expect("lock").play(&sine_8k(), 1.0);
        shared
    }

    #[test]
    fn repeat_last_removes_the_silent_edge_of_xrun_gaps() {
        let mean = |edges: &[f32]| edges.iter().sum::<f32>() / edges.len() as f32;

        let shared = shared_with_tone();
        let (_, silent) = run_gapped(&mut SilentOnGap(Arc::clone(&shared)), &shared);
        let silent_edge = mean(&silent);
        assert!(
            silent_edge > 0.4,
            "silence steps by the last sample: {silent_edge}"
        );

        let shared = shared_with_tone();
        let mut held = HoldLastMixer::new(Arc::clone(&shared));
        let (left, edges) = run_gapped(&mut held, &shared);
        assert_eq!(mean(&edges), 0.0, "repeat-last has no entry edge");
        assert!(
            1.0 - mean(&edges) / silent_edge >= 0.5,
            "bar is a 50 percent cut"
        );
        assert_eq!(held.gaps(), 3);
        for b in GAP_BLOCKS {
            let gap = &left[b * BLOCK..(b + 1) * BLOCK];
            assert!(
                gap.iter().all(|s| *s == left[b * BLOCK - 1]),
                "gap {b} holds the last frame"
            );
        }
    }

    #[test]
    fn repeat_last_run_twice_gives_identical_bits() {
        let run = || {
            let shared = shared_with_tone();
            let mut held = HoldLastMixer::new(Arc::clone(&shared));
            let (left, _) = run_gapped(&mut held, &shared);
            left.iter().map(|s| s.to_bits()).collect::<Vec<_>>()
        };
        assert_eq!(run(), run());
    }

    #[test]
    fn a_gap_before_any_frame_holds_silence_and_odd_tails_stay_zero() {
        let shared = shared_with_tone();
        let mut held = HoldLastMixer::new(Arc::clone(&shared));
        let guard = shared.lock().expect("lock");
        let mut out = [9.0f32; 5];
        held.render_stereo_into(&mut out);
        assert_eq!(out, [0.0; 5]);
        drop(guard);
        held.render_stereo_into(&mut out);
        assert!(
            out[0] != 0.0 && out[4] == 0.0,
            "live chunk renders, odd tail is zero"
        );
        assert_eq!(held.gaps(), 1);
    }

    #[test]
    fn hold_last_callback_allocates_nothing_on_live_or_gap_chunks() {
        let shared = shared_with_tone();
        let mut held = HoldLastMixer::new(Arc::clone(&shared));
        let mut out = [0.0f32; BLOCK * 2];
        held.render_stereo_into(&mut out);
        let guard = shared.lock().expect("lock");
        let ((), gap) = engine_core::alloc_probe::measure(|| held.render_stereo_into(&mut out));
        drop(guard);
        let ((), live) = engine_core::alloc_probe::measure(|| held.render_stereo_into(&mut out));
        assert_eq!(
            (gap.calls, live.calls),
            (0, 0),
            "no heap calls in the callback"
        );
    }
}
