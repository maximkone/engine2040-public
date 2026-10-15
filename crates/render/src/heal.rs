//! heal (AH2): surface Outdated/Lost plus device-loss recovery through the
//! observe supervisor, keeping last-good and degrading on out-of-memory.
//!
//! Consumer: WIN1/ED1 frame loops survive GPU faults; in-crate the
//! [`SimulatedSurface`] forced-loss tests are the consumer. Data owner:
//! [`RenderHeal`] owns supervisor, ring, quality, last-good generation and
//! notices; the caller owns the presenter. Units: retries (`u32`), backoff
//! (`u64` ms), quality tiers ([`QualityLevel`]). Determinism: tick-derived
//! backoff from the shipped catalog, no clock. Errors: [`RenderHealError`];
//! over-budget faults quarantine loudly with a notice plus the last-good frame.

use engine_observe::heal::{HealError, HealKind};
use engine_observe::supervisor::{SupervisedDecision, Supervisor};
use engine_observe::{EventCode, RingBuffer};

use crate::present::{
    surface_action, surface_fault_code, PresentError, SurfaceAction, SurfaceEvent,
};

/// Fault row device loss and surface `Lost` report through: the catalog's
/// device-reconnect row (reconnect, 3 attempts, 100-2000 ms backoff).
pub const DEVICE_FAULT: EventCode = EventCode::RENDER_DEVICE_LOST;

/// Fault row surface `Outdated` and validation failures report through: the
/// catalog's pipeline row (last-good, 2 attempts, 50-500 ms backoff).
pub const SURFACE_FAULT: EventCode = EventCode::RENDER_PIPELINE_FAILED;

/// Fault row out-of-memory reports through: the catalog's frame-budget row
/// (degrade, 3 attempts, no backoff; quality steps down per fault).
pub const OOM_FAULT: EventCode = EventCode::RENDER_FRAME_OVER_BUDGET;

/// Heal event ring capacity: 64 rows of `heal.applied` / `heal.failed`.
pub const HEAL_RING_CAP: usize = 64;

/// Cap for human-readable notices and recovery reports.
pub const MAX_NOTICE_LEN: usize = 256;

/// Typed render-heal failure. The fault itself is not an error: it degrades
/// to the last-good pipeline with backoff (or a lower quality tier on
/// out-of-memory). Errors surface only when a recovery is refused while
/// budget remains, the budget is spent, or the supervisor tape needs export.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RenderHealError {
    /// Recovery refused while budget remains; wait `backoff_ms`, keep the
    /// last-good frame, then retry. `attempt` is the one-based fault count.
    RecoverFailed {
        /// Backoff the supervisor advised for the next attempt, in ms.
        backoff_ms: u64,
        /// One-based fault count so far.
        attempt: u32,
    },
    /// Budget spent: rendering stays on the last-good frame and the loud
    /// notice names the fault count. Recovery needs a verified rebuild.
    Quarantined {
        /// Total fault count that spent the budget.
        attempts: u32,
    },
    /// Supervisor capacity failure (tape full): export history, then retry.
    /// Consumes no attempt budget.
    Supervisor(HealError),
}

impl std::fmt::Display for RenderHealError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::RecoverFailed {
                backoff_ms,
                attempt,
            } => write!(
                f,
                "render: recovery refused (attempt {attempt}); retry after {backoff_ms}ms, last-good frame held"
            ),
            Self::Quarantined { attempts } => write!(
                f,
                "render: quarantined after {attempts} faults; last-good frame held, operator notice raised"
            ),
            Self::Supervisor(err) => write!(f, "render: supervisor fault: {err}"),
        }
    }
}

impl std::error::Error for RenderHealError {}

impl From<HealError> for RenderHealError {
    fn from(err: HealError) -> Self {
        Self::Supervisor(err)
    }
}

/// Presentation quality tier. Out-of-memory steps down one tier per fault to
/// the declared floor; a verified recovery restores full quality.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum QualityLevel {
    /// Declared floor: never degrades below this.
    Minimal,
    /// Reduced effects, kept above the floor.
    Performance,
    /// Reduced resolution scale, kept above the floor.
    Balanced,
    /// Full effects at native scale.
    Full,
}

impl QualityLevel {
    /// Declared floor: degradation stops here; further faults quarantine.
    pub const FLOOR: Self = Self::Minimal;
    /// Shipped default: full effects at native scale.
    pub const DEFAULT: Self = Self::Full;

    /// Tier index, higher is better; the floor is tier 0.
    #[must_use]
    pub fn tier(self) -> u8 {
        match self {
            Self::Minimal => 0,
            Self::Performance => 1,
            Self::Balanced => 2,
            Self::Full => 3,
        }
    }

    /// Short label for notices and receipts.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Minimal => "minimal",
            Self::Performance => "performance",
            Self::Balanced => "balanced",
            Self::Full => "full",
        }
    }

    /// True at the declared floor: no further degradation is possible.
    #[must_use]
    pub fn is_floor(self) -> bool {
        self == Self::FLOOR
    }

    /// Step down one tier toward the floor. Returns true when the tier
    /// changed; false at the floor (the fault still consumes budget).
    pub fn step_down(&mut self) -> bool {
        let next = match *self {
            Self::Full => Self::Balanced,
            Self::Balanced => Self::Performance,
            Self::Performance | Self::Minimal => Self::Minimal,
        };
        let changed = next != *self;
        *self = next;
        changed
    }
}

/// Render playback state. Degraded and quarantined frames hold last-good;
/// only the loudness of the notice differs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RenderState {
    /// Frames present normally.
    Live,
    /// Fault within budget; last-good frame held with backoff or lower tier.
    Degraded,
    /// Budget spent; last-good frame held, loud notice raised.
    Quarantined,
}

/// Last-good frame held while degraded or quarantined: the pipeline and
/// targets the presenter keeps, never a guess.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LastGoodFrame {
    /// Last verified presenter generation.
    pub generation: u64,
    /// Quality tier the held frame renders at.
    pub quality: QualityLevel,
    /// Frames held on last-good while down. Monotonic.
    pub held_frames: u64,
}

/// Recovery report returned when a rebuild succeeds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HealReport {
    /// Tick the recovery succeeded at.
    pub tick: u64,
    /// Faults seen before this recovery (budget spent, then reset).
    pub attempts: u32,
    /// Last backoff advised before recovery, in ms.
    pub backoff_ms: u64,
    /// Quality tier restored to (always full on verified recovery).
    pub quality: QualityLevel,
    /// Last-good generation kept across the outage.
    pub generation: u64,
    /// Frames held on last-good while down.
    pub held_frames: u64,
    /// Human-readable recovery line, bounded to [`MAX_NOTICE_LEN`].
    pub message: String,
}

/// Supervisor-backed render heal. Surface `Outdated`/`Lost` and device loss
/// report through their catalog rows (reconnect with backoff, last-good
/// kept); out-of-memory steps quality down to the floor; a refused rebuild
/// escalates the backoff; a verified rebuild resets every render budget and
/// restores full quality; past any budget the fault quarantines loudly with
/// the last-good frame held.
#[derive(Clone, Debug)]
pub struct RenderHeal {
    supervisor: Supervisor,
    ring: RingBuffer,
    state: RenderState,
    quality: QualityLevel,
    last_good_generation: u64,
    held_frames: u64,
    notice: String,
}

impl RenderHeal {
    /// Fresh heal: live at full quality, empty ring, full catalog budgets.
    #[must_use]
    pub fn new() -> Self {
        Self {
            supervisor: Supervisor::new(),
            ring: RingBuffer::new(HEAL_RING_CAP),
            state: RenderState::Live,
            quality: QualityLevel::DEFAULT,
            last_good_generation: 0,
            held_frames: 0,
            notice: "render live: pipeline clean".to_owned(),
        }
    }

    /// Current playback state.
    #[must_use]
    pub fn state(&self) -> RenderState {
        self.state
    }

    /// True while frames present normally.
    #[must_use]
    pub fn is_live(&self) -> bool {
        self.state == RenderState::Live
    }

    /// True once a budget is spent and the loud notice is raised.
    #[must_use]
    pub fn is_quarantined(&self) -> bool {
        self.state == RenderState::Quarantined
    }

    /// Current quality tier ([`QualityLevel::FLOOR`] at the earliest).
    #[must_use]
    pub fn quality(&self) -> QualityLevel {
        self.quality
    }

    /// Last verified presenter generation; untouched by faults.
    #[must_use]
    pub fn last_good_generation(&self) -> u64 {
        self.last_good_generation
    }

    /// Frames held on last-good while down. Monotonic.
    #[must_use]
    pub fn held_frames(&self) -> u64 {
        self.held_frames
    }

    /// Latest human-readable status, bounded to [`MAX_NOTICE_LEN`].
    #[must_use]
    pub fn notice(&self) -> &str {
        &self.notice
    }

    /// Heal tape rows recorded so far (evidence, never dropped silently).
    #[must_use]
    pub fn tape_len(&self) -> usize {
        self.supervisor.tape_len()
    }

    /// Borrow the supervisor (budgets, tape, escalation).
    #[must_use]
    pub fn supervisor(&self) -> &Supervisor {
        &self.supervisor
    }

    /// Borrow the heal event ring.
    #[must_use]
    pub fn ring(&self) -> &RingBuffer {
        &self.ring
    }

    /// Faults seen under the current budget for `code` (zero after reset).
    #[must_use]
    pub fn attempts(&self, code: EventCode) -> u32 {
        self.supervisor.attempts(code)
    }

    /// Backoff advised for the next attempt on `code`, in ms (zero when
    /// the code never faulted).
    #[must_use]
    pub fn next_backoff_ms(&self, code: EventCode) -> u64 {
        self.supervisor
            .escalation_of(code)
            .map_or(0, |escalation| escalation.backoff_ms)
    }

    /// Last-good frame held while degraded or quarantined.
    #[must_use]
    pub fn last_good(&self) -> LastGoodFrame {
        LastGoodFrame {
            generation: self.last_good_generation,
            quality: self.quality,
            held_frames: self.held_frames,
        }
    }

    /// Record one verified clean frame: the generation becomes last-good.
    /// Never resets budgets or quality; only [`Self::try_recover`] does.
    pub fn note_presented(&mut self, generation: u64) {
        self.last_good_generation = generation;
    }

    /// Hold `frames` last-good frames while down (degraded or quarantined):
    /// counts into [`Self::held_frames`], returns the cumulative count.
    /// While live nothing is held and the count stays put. Never panics.
    pub fn hold_last_good(&mut self, frames: u64) -> u64 {
        if self.state != RenderState::Live {
            self.held_frames = self.held_frames.saturating_add(frames);
        }
        self.held_frames
    }

    /// Route one surface-acquire outcome: fault events report through their
    /// catalog row and degrade to last-good; clean, suboptimal, timeout and
    /// occluded outcomes are not faults and return `None` without touching
    /// any budget.
    ///
    /// # Errors
    ///
    /// Returns [`RenderHealError::Quarantined`] past the budget and
    /// [`RenderHealError::Supervisor`] when the tape needs exporting.
    pub fn report_surface(
        &mut self,
        tick: u64,
        event: SurfaceEvent,
    ) -> Result<Option<SupervisedDecision>, RenderHealError> {
        let Some(code) = surface_fault_code(event) else {
            return Ok(None);
        };
        let action = surface_action(event);
        let decision = self.supervisor.report_fault(code, tick, &mut self.ring)?;
        self.advise_surface(tick, decision, code, &format!("{event:?}"), action)
            .map(Some)
    }

    /// Report a lost GPU device: supervisor reconnect advice with backoff,
    /// last-good pipeline kept. Past the budget this quarantines loudly.
    ///
    /// # Errors
    ///
    /// Returns [`RenderHealError::Quarantined`] past the budget and
    /// [`RenderHealError::Supervisor`] when the tape needs exporting.
    pub fn report_device_lost(
        &mut self,
        tick: u64,
        reason: &str,
    ) -> Result<SupervisedDecision, RenderHealError> {
        let decision = self
            .supervisor
            .report_fault(DEVICE_FAULT, tick, &mut self.ring)?;
        self.advise_surface(
            tick,
            decision,
            DEVICE_FAULT,
            &format!("device lost: {reason}"),
            SurfaceAction::ReconfigureThenRetry,
        )
    }

    /// Route one presenter failure: fault errors report through their
    /// catalog row (out-of-memory also steps quality down); non-fault
    /// errors return `None` without touching any budget.
    ///
    /// # Errors
    ///
    /// Same as [`Self::report_surface`].
    pub fn report_present_error(
        &mut self,
        tick: u64,
        error: &PresentError,
    ) -> Result<Option<SupervisedDecision>, RenderHealError> {
        let Some(code) = error.fault_code() else {
            return Ok(None);
        };
        if code == OOM_FAULT {
            return self.report_oom(tick).map(Some);
        }
        let decision = self.supervisor.report_fault(code, tick, &mut self.ring)?;
        self.advise_surface(
            tick,
            decision,
            code,
            &error.to_string(),
            error_action(error),
        )
        .map(Some)
    }

    /// Report an out-of-memory allocation failure: supervisor degrade advice
    /// with no backoff, quality steps down one tier toward the floor,
    /// last-good frame held. Past the budget this quarantines loudly at the
    /// floor.
    ///
    /// # Errors
    ///
    /// Returns [`RenderHealError::Quarantined`] past the budget and
    /// [`RenderHealError::Supervisor`] when the tape needs exporting.
    pub fn report_oom(&mut self, tick: u64) -> Result<SupervisedDecision, RenderHealError> {
        let decision = self
            .supervisor
            .report_fault(OOM_FAULT, tick, &mut self.ring)?;
        if decision.is_quarantine() {
            return self.quarantine(tick, decision, OOM_FAULT, "out of memory");
        }
        self.quality.step_down();
        self.state = RenderState::Degraded;
        let budget = self.policy_budget(OOM_FAULT);
        self.notice = bound_notice(&format!(
            "render degraded: out of memory at tick {tick} (attempt {}/{}; quality {}); last-good frame held",
            decision.attempt,
            budget,
            self.quality.label()
        ));
        Ok(decision)
    }

    /// Try `opener` once after `event`: on success reset every render
    /// budget, go live at full quality and report the recovery; on refusal
    /// escalate through the supervisor (backoff or loud quarantine). The
    /// opener is caller-supplied so tests stay simulated; WIN1/ED1 pass the
    /// real reconfigure/reacquire. Non-fault events never call the opener:
    /// they return the current last-good report unchanged.
    ///
    /// # Errors
    ///
    /// Returns [`RenderHealError::RecoverFailed`] while budget remains,
    /// [`RenderHealError::Quarantined`] past it, and
    /// [`RenderHealError::Supervisor`] when the tape needs exporting.
    pub fn try_recover<T, E>(
        &mut self,
        tick: u64,
        event: SurfaceEvent,
        opener: impl FnOnce() -> Result<T, E>,
    ) -> Result<(T, HealReport), RenderHealError> {
        let Some(code) = surface_fault_code(event) else {
            let report = HealReport {
                tick,
                attempts: 0,
                backoff_ms: 0,
                quality: self.quality,
                generation: self.last_good_generation,
                held_frames: self.held_frames,
                message: bound_notice(&format!(
                    "render live: {event:?} at tick {tick} needed no heal; last-good generation {} held",
                    self.last_good_generation
                )),
            };
            // Non-fault recoveries still need a value of `T`, which only the
            // opener can build; a failing opener here is a caller bug, so the
            // refusal degrades rather than quarantines.
            return match opener() {
                Ok(value) => Ok((value, report)),
                Err(_) => Err(RenderHealError::RecoverFailed {
                    backoff_ms: 0,
                    attempt: 0,
                }),
            };
        };
        match opener() {
            Ok(value) => {
                let attempts = self.supervisor.attempts(code);
                let backoff_ms = self.next_backoff_ms(code);
                self.reset_render_budgets();
                self.state = RenderState::Live;
                self.quality = QualityLevel::DEFAULT;
                let message = bound_notice(&format!(
                    "render recovered: {event:?} rebuilt at tick {tick} after {attempts} faults (last backoff {backoff_ms}ms); last-good generation {} held {} frames, quality restored to {}",
                    self.last_good_generation,
                    self.held_frames,
                    self.quality.label()
                ));
                self.notice.clone_from(&message);
                Ok((
                    value,
                    HealReport {
                        tick,
                        attempts,
                        backoff_ms,
                        quality: self.quality,
                        generation: self.last_good_generation,
                        held_frames: self.held_frames,
                        message,
                    },
                ))
            }
            Err(_) => {
                let decision = self.supervisor.report_fault(code, tick, &mut self.ring)?;
                match self.advise_surface(
                    tick,
                    decision,
                    code,
                    &format!("{event:?} rebuild refused"),
                    surface_action(event),
                ) {
                    Ok(advised) => Err(RenderHealError::RecoverFailed {
                        backoff_ms: advised.backoff_ms,
                        attempt: advised.attempt,
                    }),
                    Err(quarantined) => Err(quarantined),
                }
            }
        }
    }

    /// Clear one render fault budget after operator-verified recovery.
    /// Tape history is kept: history is evidence, not budget.
    pub fn reset(&mut self, code: EventCode) {
        self.supervisor.reset(code);
        if !self.any_quarantined() && self.state == RenderState::Quarantined {
            self.state = RenderState::Degraded;
        }
    }

    /// Route one supervisor decision to state plus notice: heal advice
    /// degrades to last-good, an exhausted budget quarantines loudly.
    fn advise_surface(
        &mut self,
        tick: u64,
        decision: SupervisedDecision,
        code: EventCode,
        reason: &str,
        action: SurfaceAction,
    ) -> Result<SupervisedDecision, RenderHealError> {
        if decision.is_quarantine() {
            return self.quarantine(tick, decision, code, reason);
        }
        debug_assert_eq!(decision.kind, self.policy_kind(code));
        self.state = RenderState::Degraded;
        let budget = self.policy_budget(code);
        self.notice = bound_notice(&format!(
            "render degraded: {reason} at tick {tick} (attempt {}/{}; backoff {}ms; action {action:?}); last-good generation {} held",
            decision.attempt, budget, decision.backoff_ms, self.last_good_generation
        ));
        Ok(decision)
    }

    /// Quarantine loudly: last-good frame held, notice names the budget.
    fn quarantine(
        &mut self,
        tick: u64,
        decision: SupervisedDecision,
        code: EventCode,
        reason: &str,
    ) -> Result<SupervisedDecision, RenderHealError> {
        self.state = RenderState::Quarantined;
        let held = self.last_good();
        self.notice = bound_notice(&format!(
            "render quarantined: {reason} at tick {tick} after {} faults on {}; last-good frame held (generation {}, quality {}); operator notice raised",
            decision.attempt,
            code.name().unwrap_or("unknown"),
            held.generation,
            held.quality.label()
        ));
        Err(RenderHealError::Quarantined {
            attempts: decision.attempt,
        })
    }

    /// Reset every render fault budget after a verified rebuild. History
    /// stays: tape and ring rows are evidence, not budget.
    fn reset_render_budgets(&mut self) {
        for code in [DEVICE_FAULT, SURFACE_FAULT, OOM_FAULT] {
            self.supervisor.reset(code);
        }
    }

    /// True while any render fault row spent its budget.
    fn any_quarantined(&self) -> bool {
        [DEVICE_FAULT, SURFACE_FAULT, OOM_FAULT]
            .iter()
            .any(|code| self.supervisor.is_quarantined(*code))
    }

    /// Catalog attempt budget for `code` (zero when the row is missing,
    /// which never happens with the shipped catalog).
    fn policy_budget(&self, code: EventCode) -> u32 {
        self.supervisor
            .policy(code)
            .map_or(0, |policy| policy.attempts_max)
    }

    /// Catalog heal kind for `code` (quarantine when the row is missing,
    /// which never happens with the shipped catalog).
    fn policy_kind(&self, code: EventCode) -> HealKind {
        self.supervisor
            .policy(code)
            .map_or(HealKind::Quarantine, |policy| policy.kind)
    }
}

impl Default for RenderHeal {
    /// Fresh heal with full budgets at full quality.
    fn default() -> Self {
        Self::new()
    }
}

/// Presenter action for one fault error: loss and outdated reconfigure and
/// retry, validation aborts to last-good. Non-fault errors never reach here
/// ([`PresentError::fault_code`] returns `None` for them first).
fn error_action(error: &PresentError) -> SurfaceAction {
    match error {
        PresentError::Device(_)
        | PresentError::DeviceLost(_)
        | PresentError::SurfaceLost { .. }
        | PresentError::SurfaceOutdated { .. } => SurfaceAction::ReconfigureThenRetry,
        PresentError::Validation(_) => SurfaceAction::AbortKeepLastGood,
        _ => SurfaceAction::SkipKeepLastGood,
    }
}

/// Truncate a notice to [`MAX_NOTICE_LEN`] bytes on a `char` boundary.
fn bound_notice(text: &str) -> String {
    if text.len() <= MAX_NOTICE_LEN {
        return text.to_owned();
    }
    let mut end = MAX_NOTICE_LEN;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_owned()
}

// ---------------------------------------------------------------------------
// Simulated surface: scripted acquire outcomes for forced-loss tests. The
// presenter feeds real `CurrentSurfaceTexture` values through the same
// `surface_action` bridge; tests feed this script instead, so recovery runs
// in-crate with no GPU.
// ---------------------------------------------------------------------------

/// Scripted surface-acquire outcomes plus reconfigure/present counters: the
/// in-crate stand-in for a real swapchain during forced-loss tests.
#[derive(Clone, Debug)]
pub struct SimulatedSurface {
    script: Vec<SurfaceEvent>,
    next: usize,
    reconfigures: u64,
    presented: u64,
    generation: u64,
}

impl SimulatedSurface {
    /// Script `events` in acquire order; the surface starts at generation 0.
    #[must_use]
    pub fn new(script: Vec<SurfaceEvent>) -> Self {
        Self {
            script,
            next: 0,
            reconfigures: 0,
            presented: 0,
            generation: 0,
        }
    }

    /// Next scripted outcome plus its presenter action, or `None` once the
    /// script is spent. Pure: the caller acts (heal, reconfigure, present).
    #[must_use]
    pub fn acquire(&mut self) -> Option<(SurfaceEvent, SurfaceAction)> {
        let event = *self.script.get(self.next)?;
        self.next += 1;
        Some((event, surface_action(event)))
    }

    /// Record one surface reconfigure: bumps the surface generation, exactly
    /// like the swapchain path; the heal's last-good generation is untouched.
    pub fn note_reconfigured(&mut self) {
        self.reconfigures = self.reconfigures.saturating_add(1);
        self.generation = self.generation.saturating_add(1);
    }

    /// Record one presented frame.
    pub fn note_presented(&mut self) {
        self.presented = self.presented.saturating_add(1);
    }

    /// Successful reconfigures so far.
    #[must_use]
    pub fn reconfigures(&self) -> u64 {
        self.reconfigures
    }

    /// Presented frames so far.
    #[must_use]
    pub fn presented(&self) -> u64 {
        self.presented
    }

    /// Current surface generation (moves on reconfigure; last-good does not).
    #[must_use]
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Scripted outcomes not yet acquired.
    #[must_use]
    pub fn remaining(&self) -> usize {
        self.script.len().saturating_sub(self.next)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_observe::supervisor::backoff_ms;

    fn policy_backoff(heal: &RenderHeal, code: EventCode, attempt: u32) -> u64 {
        let policy = heal
            .supervisor()
            .policy(code)
            .expect("catalog must hold the render rows");
        backoff_ms(policy.backoff_base_ms, policy.backoff_cap_ms, attempt)
    }

    #[test]
    fn surface_faults_route_to_catalog_policies_with_backoff() {
        let mut heal = RenderHeal::new();
        assert!(heal.is_live());
        // Lost reconnects: 3 attempts at 100/200/400 ms.
        let decision = heal
            .report_surface(10, SurfaceEvent::Lost)
            .expect("first loss must heal")
            .expect("loss is a fault");
        assert_eq!(decision.kind, HealKind::Reconnect);
        assert_eq!(decision.attempt, 1);
        assert_eq!(decision.backoff_ms, policy_backoff(&heal, DEVICE_FAULT, 1));
        assert_eq!(decision.backoff_ms, 100);
        assert_eq!(heal.state(), RenderState::Degraded);
        assert!(heal.notice().contains("degraded"));
        assert!(heal.notice().contains("last-good"));
        // Outdated keeps last-good: 2 attempts at 50/100 ms.
        let decision = heal
            .report_surface(11, SurfaceEvent::Outdated)
            .expect("outdated must heal")
            .expect("outdated is a fault");
        assert_eq!(decision.kind, HealKind::LastGood);
        assert_eq!(decision.attempt, 1);
        assert_eq!(decision.backoff_ms, 50);
        // Non-faults consume no budget.
        for event in [
            SurfaceEvent::AcquiredClean,
            SurfaceEvent::AcquiredSuboptimal,
            SurfaceEvent::Timeout,
            SurfaceEvent::Occluded,
        ] {
            assert_eq!(
                heal.report_surface(12, event)
                    .expect("non-fault never errors"),
                None,
                "no fault for {event:?}"
            );
        }
        assert_eq!(heal.attempts(DEVICE_FAULT), 1);
        assert_eq!(heal.attempts(SURFACE_FAULT), 1);
        assert_eq!(heal.attempts(OOM_FAULT), 0);
        assert_eq!(heal.next_backoff_ms(DEVICE_FAULT), 100);
        assert_eq!(heal.next_backoff_ms(OOM_FAULT), 0);
        assert_eq!(heal.tape_len(), 2);
    }

    #[test]
    fn device_loss_reconnects_then_quarantines_with_last_good() {
        let mut heal = RenderHeal::new();
        heal.note_presented(7);
        let mut backoffs = Vec::new();
        for tick in 0..3_u64 {
            let decision = heal
                .report_device_lost(tick, "test loss")
                .expect("within budget");
            assert_eq!(decision.kind, HealKind::Reconnect);
            backoffs.push(decision.backoff_ms);
            assert_eq!(heal.state(), RenderState::Degraded);
        }
        assert_eq!(backoffs, vec![100, 200, 400]);
        // Last-good generation survives every fault.
        assert_eq!(heal.last_good_generation(), 7);
        let err = heal
            .report_device_lost(3, "test loss")
            .expect_err("fourth loss spends the 3-attempt budget");
        assert_eq!(err, RenderHealError::Quarantined { attempts: 4 });
        assert!(heal.is_quarantined());
        assert!(heal.notice().contains("quarantined"));
        assert!(heal.notice().contains("last-good frame held"));
        assert!(heal.notice().contains("generation 7"));
        // Loud, never silent: a heal.failed row is taped.
        assert_eq!(heal.ring().query(Some(EventCode::HEAL_FAILED)).len(), 1);
        let held = heal.last_good();
        assert_eq!(
            held,
            LastGoodFrame {
                generation: 7,
                quality: QualityLevel::Full,
                held_frames: 0,
            }
        );
    }

    #[test]
    fn outdated_quarantines_past_two_with_notice() {
        let mut heal = RenderHeal::new();
        for tick in [10_u64, 11] {
            let decision = heal
                .report_surface(tick, SurfaceEvent::Outdated)
                .expect("within budget")
                .expect("outdated is a fault");
            assert_eq!(decision.kind, HealKind::LastGood);
        }
        assert_eq!(heal.attempts(SURFACE_FAULT), 2);
        let err = heal
            .report_surface(12, SurfaceEvent::Outdated)
            .expect_err("third outdated spends the 2-attempt budget");
        assert_eq!(err, RenderHealError::Quarantined { attempts: 3 });
        assert!(heal.is_quarantined());
        assert!(heal.notice().contains("render.pipeline.failed"));
    }

    #[test]
    fn oom_steps_quality_to_floor_then_quarantines() {
        let mut heal = RenderHeal::new();
        assert_eq!(heal.quality(), QualityLevel::Full);
        let tiers = [
            (QualityLevel::Balanced, 2_u8),
            (QualityLevel::Performance, 1),
            (QualityLevel::Minimal, 0),
        ];
        for (tick, (quality, tier)) in tiers.into_iter().enumerate() {
            let decision = heal.report_oom(tick as u64).expect("within budget");
            assert_eq!(decision.kind, HealKind::Degrade);
            assert_eq!(decision.backoff_ms, 0);
            assert_eq!(heal.quality(), quality);
            assert_eq!(heal.quality().tier(), tier);
            assert_eq!(heal.state(), RenderState::Degraded);
            assert!(heal.notice().contains(quality.label()));
        }
        assert!(heal.quality().is_floor());
        assert_eq!(heal.quality(), QualityLevel::FLOOR);
        // At the floor the next fault quarantines instead of degrading.
        let err = heal
            .report_oom(3)
            .expect_err("fourth OOM spends the 3-attempt budget");
        assert_eq!(err, RenderHealError::Quarantined { attempts: 4 });
        assert!(heal.is_quarantined());
        assert!(heal.quality().is_floor());
        assert!(heal.notice().contains("quarantined"));
        assert!(heal.notice().contains("minimal"));
    }

    #[test]
    fn present_errors_route_and_non_faults_cost_nothing() {
        let mut heal = RenderHeal::new();
        let decision = heal
            .report_present_error(1, &PresentError::SurfaceLost { generation: 2 })
            .expect("surface loss must heal")
            .expect("surface loss is a fault");
        assert_eq!(decision.kind, HealKind::Reconnect);
        let decision = heal
            .report_present_error(2, &PresentError::Validation("acquire".into()))
            .expect("validation must heal")
            .expect("validation is a fault");
        assert_eq!(decision.kind, HealKind::LastGood);
        let decision = heal
            .report_present_error(3, &PresentError::OutOfMemory { generation: 2 })
            .expect("OOM must heal")
            .expect("OOM is a fault");
        assert_eq!(decision.kind, HealKind::Degrade);
        assert_eq!(heal.quality(), QualityLevel::Balanced);
        for err in [
            PresentError::NoAdapter,
            PresentError::SurfaceTimeout,
            PresentError::Readback("map".into()),
            PresentError::InvalidSize {
                width: 9999,
                height: 1,
            },
            PresentError::NoSurface,
        ] {
            assert_eq!(
                heal.report_present_error(4, &err)
                    .expect("non-fault never errors"),
                None,
                "no fault for {err}"
            );
        }
        assert_eq!(heal.attempts(DEVICE_FAULT), 1);
        assert_eq!(heal.attempts(SURFACE_FAULT), 1);
        assert_eq!(heal.attempts(OOM_FAULT), 1);
    }

    #[test]
    fn forced_loss_script_recovers_with_last_good() {
        let mut heal = RenderHeal::new();
        heal.note_presented(5);
        let mut surface = SimulatedSurface::new(vec![
            SurfaceEvent::Lost,
            SurfaceEvent::Outdated,
            SurfaceEvent::AcquiredClean,
        ]);
        let mut backoffs = Vec::new();
        let mut tick = 0_u64;
        // Drive the script like the window loop: fault outcomes report and
        // reconfigure, the clean outcome presents.
        while let Some((event, action)) = surface.acquire() {
            match surface_fault_code(event) {
                Some(_) if action == SurfaceAction::ReconfigureThenRetry => {
                    // First rebuild refused (backoff), second succeeds.
                    let attempt = heal.attempts(DEVICE_FAULT) + heal.attempts(SURFACE_FAULT);
                    let recovered = heal.try_recover(tick, event, || {
                        if attempt == 0 {
                            Err("adapter busy")
                        } else {
                            Ok("swapchain")
                        }
                    });
                    match recovered {
                        Ok(_) => {
                            surface.note_reconfigured();
                            surface.note_presented();
                        }
                        Err(RenderHealError::RecoverFailed { backoff_ms, .. }) => {
                            backoffs.push(backoff_ms);
                            surface.note_reconfigured();
                        }
                        Err(other) => panic!("forced loss must recover, got {other}"),
                    }
                }
                None => {
                    surface.note_presented();
                    heal.note_presented(surface.generation());
                }
                Some(_) => panic!("script holds only retry faults, got {event:?}"),
            }
            tick += 1;
        }
        assert_eq!(surface.remaining(), 0);
        assert_eq!(backoffs, vec![100]);
        assert_eq!(surface.presented(), 2);
        assert_eq!(surface.reconfigures(), 2);
        // Last-good kept across the loss, then advanced by the clean frame.
        assert_eq!(heal.last_good_generation(), surface.generation());
        assert!(heal.is_live());
        assert!(heal.notice().contains("recovered"));
        // Verified recovery reset every render budget.
        assert_eq!(heal.attempts(DEVICE_FAULT), 0);
        assert_eq!(heal.attempts(SURFACE_FAULT), 0);
        let decision = heal
            .report_surface(99, SurfaceEvent::Lost)
            .expect("fresh budget heals")
            .expect("loss is a fault");
        assert_eq!(decision.attempt, 1);
        assert_eq!(decision.backoff_ms, 100);
    }

    #[test]
    fn held_frames_count_while_degraded_not_live() {
        let mut heal = RenderHeal::new();
        assert_eq!(heal.hold_last_good(4), 0);
        assert_eq!(heal.held_frames(), 0);
        heal.report_surface(1, SurfaceEvent::Lost)
            .expect("loss degrades")
            .expect("loss is a fault");
        assert_eq!(heal.hold_last_good(4), 4);
        assert_eq!(heal.hold_last_good(6), 10);
        assert_eq!(heal.last_good().held_frames, 10);
    }

    #[test]
    fn verified_recovery_restores_full_quality_and_resets() {
        let mut heal = RenderHeal::new();
        heal.report_oom(1).expect("OOM degrades");
        heal.report_oom(2).expect("OOM degrades");
        assert_eq!(heal.quality(), QualityLevel::Performance);
        let (_, report) = heal
            .try_recover(9, SurfaceEvent::Lost, || Ok::<_, &str>("swapchain"))
            .expect("verified rebuild must recover");
        assert!(heal.is_live());
        assert_eq!(heal.quality(), QualityLevel::Full);
        assert_eq!(report.quality, QualityLevel::Full);
        assert_eq!(report.tick, 9);
        assert_eq!(report.attempts, 0);
        assert_eq!(report.backoff_ms, 0);
        assert!(report.message.contains("recovered"));
        assert_eq!(heal.notice(), report.message.as_str());
        assert_eq!(heal.attempts(OOM_FAULT), 0);
    }

    #[test]
    fn same_loss_sequence_twice_gives_same_backoffs_and_notices() {
        fn sequence() -> (Vec<u64>, Vec<String>, QualityLevel) {
            let mut heal = RenderHeal::new();
            let mut backoffs = Vec::new();
            backoffs.push(heal.report_device_lost(1, "loss").expect("loss").backoff_ms);
            for tick in 2..=3_u64 {
                let err = heal
                    .try_recover(tick, SurfaceEvent::Lost, || Err::<(), _>("busy"))
                    .expect_err("refused");
                if let RenderHealError::RecoverFailed { backoff_ms, .. } = err {
                    backoffs.push(backoff_ms);
                }
            }
            heal.report_oom(4).expect("OOM degrades");
            let notices = heal
                .ring()
                .query(None)
                .iter()
                .map(|event| event.message.clone())
                .collect();
            (backoffs, notices, heal.quality())
        }
        let (backoffs, notices, quality) = sequence();
        assert_eq!(backoffs, vec![100, 200, 400]);
        assert_eq!((backoffs.clone(), notices.clone(), quality), {
            let again = sequence();
            (again.0, again.1, again.2)
        });
        assert_eq!(quality, QualityLevel::Balanced);
    }

    #[test]
    fn quality_tiers_step_down_to_the_declared_floor() {
        let mut quality = QualityLevel::Full;
        assert_eq!(quality.tier(), 3);
        assert!(!quality.is_floor());
        assert!(quality.step_down());
        assert_eq!((quality, quality.tier()), (QualityLevel::Balanced, 2));
        assert!(quality.step_down());
        assert_eq!((quality, quality.tier()), (QualityLevel::Performance, 1));
        assert!(quality.step_down());
        assert_eq!((quality, quality.tier()), (QualityLevel::Minimal, 0));
        assert!(quality.is_floor());
        assert!(!quality.step_down(), "the floor never moves");
        assert_eq!(quality, QualityLevel::FLOOR);
        for level in [
            QualityLevel::Full,
            QualityLevel::Balanced,
            QualityLevel::Performance,
            QualityLevel::Minimal,
        ] {
            assert!(!level.label().is_empty(), "{level:?} must name itself");
        }
    }

    #[test]
    fn errors_are_typed_and_printable() {
        for err in [
            RenderHealError::RecoverFailed {
                backoff_ms: 200,
                attempt: 2,
            },
            RenderHealError::Quarantined { attempts: 4 },
            RenderHealError::Supervisor(HealError::TapeFull),
        ] {
            assert!(!err.to_string().is_empty(), "{err:?} must describe itself");
        }
        let _: &dyn std::error::Error = &RenderHealError::Quarantined { attempts: 4 };
        assert_eq!(
            RenderHealError::from(HealError::TapeFull),
            RenderHealError::Supervisor(HealError::TapeFull)
        );
        assert!(RenderHeal::default().is_live());
        assert_eq!(RenderHeal::default().quality(), QualityLevel::Full);
    }
}
