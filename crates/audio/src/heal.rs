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

use engine_observe::heal::HealError;
use engine_observe::supervisor::{SupervisedDecision, Supervisor};
use engine_observe::{EventCode, RingBuffer};

use crate::Mixer;

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
        if self.state != DeviceState::Live {
            let delta = u64::try_from(frames).unwrap_or(u64::MAX);
            self.silent_frames = self.silent_frames.saturating_add(delta);
        }
        self.silent_frames
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
                self.supervisor.reset(DEVICE_FAULT_CODE);
                self.state = DeviceState::Live;
                let message = bound_notice(&format!(
                    "audio recovered: device reopened at tick {tick} after {attempts} faults (last backoff {backoff_ms}ms); mixer rendered {} silent frames while down",
                    self.silent_frames
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
}
