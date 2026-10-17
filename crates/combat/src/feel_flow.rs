//! feel_flow: register feel effects against fight triggers, apply them in a
//! fixed order (the Game Feel Flow pattern, rebuilt without any engine).
//!
//! PATTERN(Game Feel Flow, MIT): a flow is a list of (trigger, effect) pairs;
//! one trigger fires every effect registered for it. Here the list is a
//! fixed-size slot array, so registering is one store: an effect registered
//! during frame N already fires on the events of frame N's own [`FeelFlow::step`]
//! and in any case no later than frame N+1, never a rebuild or a scan.
//!
//! Owner: engine-combat. Consumer: the presenter calls [`FeelFlow::step`] once
//! per presented tick with that tick's [`FightEvent`] slice and reads the
//! [`FeelFrame`] back. Determinism: effects fire in slot order, then event
//! order; integer math only (see [`crate::gamefeel`]); no heap, so the tick path
//! never allocates. Presentation only: the fight state is never read back.

use crate::fight::FightEvent;
use crate::gamefeel::{FeelFrame, GameFeel, GameFeelError};

/// Registered effects a flow holds (stack only).
pub const MAX_EFFECTS: usize = 16;
/// Slow-mo id the shipped knockdown effect uses.
const KNOCKDOWN_SLOWMO_ID: u8 = 1;

/// What fires an effect.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trigger {
    /// Full damage landed.
    Hit,
    /// A block soaked the hit.
    Blocked,
    /// A parry answered the hit.
    Parried,
    /// A fighter dropped to zero health.
    Knockdown,
}

impl Trigger {
    /// The trigger a fight event raises, if any (dodges and stamina-outs stay quiet).
    #[must_use]
    pub const fn of(event: FightEvent) -> Option<Self> {
        match event {
            FightEvent::Hit { .. } => Some(Self::Hit),
            FightEvent::Blocked { .. } => Some(Self::Blocked),
            FightEvent::Parried { .. } => Some(Self::Parried),
            FightEvent::Knockdown { .. } => Some(Self::Knockdown),
            FightEvent::Dodged { .. } | FightEvent::StaminaOut { .. } => None,
        }
    }
}

/// One feel effect; every variant maps to one gamefeel call.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Effect {
    /// Camera shake of `ticks` at `power_permille` (1000 is the reference 1.0).
    Shake {
        /// Length in ticks.
        ticks: u32,
        /// Power in permille.
        power_permille: u32,
    },
    /// Camera bump along a Q32-turn angle.
    Bump {
        /// Direction in Q32 turns.
        angle: u32,
        /// Distance in milli pixels.
        dist_milli: i32,
    },
    /// Four-tick hit-stop.
    StopFrame,
    /// Slow-mo under an id (a repeated id extends instead of stacking).
    SlowMo {
        /// Slow-mo id.
        id: u8,
        /// Length in ticks.
        ticks: u32,
        /// Speed factor in permille.
        factor_permille: u32,
    },
}

/// Handle to a registered effect, valid until it is unregistered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EffectHandle(usize);

/// Flow failures; bad data never panics.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlowError {
    /// All [`MAX_EFFECTS`] slots are in use.
    Full,
    /// A slow-mo effect found no free slow-mo slot.
    SlowMoFull,
}

/// The registered effects plus the feel state they drive.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct FeelFlow {
    effects: [Option<(Trigger, Effect)>; MAX_EFFECTS],
    feel: GameFeel,
}

impl FeelFlow {
    /// Empty flow: nothing registered, every frame quiet.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The shipped flow: a hit shakes and freezes, a parry shakes less and
    /// freezes, a block only nudges, a knockdown shakes hard and slows time.
    #[must_use]
    pub fn v1() -> Self {
        const PRESET: [(Trigger, Effect); 7] = [
            (Trigger::Hit, Effect::StopFrame),
            (Trigger::Hit, Effect::Shake { ticks: 12, power_permille: 1000 }),
            (Trigger::Parried, Effect::StopFrame),
            (Trigger::Parried, Effect::Shake { ticks: 8, power_permille: 700 }),
            (Trigger::Blocked, Effect::Bump { angle: 0, dist_milli: 1500 }),
            (Trigger::Knockdown, Effect::Shake { ticks: 30, power_permille: 2000 }),
            (
                Trigger::Knockdown,
                Effect::SlowMo { id: KNOCKDOWN_SLOWMO_ID, ticks: 45, factor_permille: 300 },
            ),
        ];
        let mut flow = Self::new();
        for (trigger, effect) in PRESET {
            // Seven of sixteen slots: cannot fail on an empty flow.
            let _ = flow.register(trigger, effect);
        }
        flow
    }

    /// Register an effect: one slot store, live from the next [`Self::step`].
    ///
    /// # Errors
    ///
    /// [`FlowError::Full`] when every slot is taken.
    pub fn register(&mut self, trigger: Trigger, effect: Effect) -> Result<EffectHandle, FlowError> {
        let free = self.effects.iter().position(Option::is_none).ok_or(FlowError::Full)?;
        self.effects[free] = Some((trigger, effect));
        Ok(EffectHandle(free))
    }

    /// Remove a registered effect; true when the handle was live.
    pub fn unregister(&mut self, handle: EffectHandle) -> bool {
        self.effects.get_mut(handle.0).and_then(Option::take).is_some()
    }

    /// Registered effect count.
    #[must_use]
    pub fn len(&self) -> usize {
        self.effects.iter().flatten().count()
    }

    /// True when nothing is registered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn fire(&mut self, effect: Effect) -> Result<(), GameFeelError> {
        match effect {
            Effect::Shake { ticks, power_permille } => self.feel.shake.start(ticks, power_permille),
            Effect::Bump { angle, dist_milli } => self.feel.bump.add_angle(angle, dist_milli),
            Effect::StopFrame => self.feel.time.stop_frame(),
            Effect::SlowMo { id, ticks, factor_permille } => {
                return self.feel.time.add_slow_mo(id, ticks, factor_permille);
            }
        }
        Ok(())
    }

    /// One presented tick: fire every registered effect for every event (event
    /// order, then slot order), then step the feel state.
    ///
    /// # Errors
    ///
    /// [`FlowError::SlowMoFull`] when a slow-mo effect found no slot; the other
    /// effects of the tick still fire and the frame is still stepped.
    pub fn step(&mut self, events: &[FightEvent], ftime: u32) -> (FeelFrame, Result<(), FlowError>) {
        let mut status = Ok(());
        for trigger in events.iter().filter_map(|e| Trigger::of(*e)) {
            for slot in 0..MAX_EFFECTS {
                if let Some((t, effect)) = self.effects[slot] {
                    if t == trigger && self.fire(effect).is_err() {
                        status = Err(FlowError::SlowMoFull);
                    }
                }
            }
        }
        (self.feel.step(ftime), status)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hit() -> FightEvent {
        FightEvent::Hit { attacker: 0, victim: 1, damage: 100 }
    }

    fn knockdown() -> FightEvent {
        FightEvent::Knockdown { attacker: 0, victim: 1 }
    }

    #[test]
    fn registration_is_live_on_the_same_frames_events() {
        // Latency from register to first effect is zero frames, under the
        // one-frame bar: the very next step already shakes.
        let mut flow = FeelFlow::new();
        assert_eq!(flow.step(&[hit()], 0).0.shake_milli, (0, 0), "nothing registered");
        flow.register(Trigger::Hit, Effect::Shake { ticks: 10, power_permille: 1000 }).unwrap();
        let (frame, status) = flow.step(&[hit()], 0);
        assert_eq!(status, Ok(()));
        assert_ne!(frame.shake_milli, (0, 0));
    }

    #[test]
    fn v1_hit_freezes_four_ticks_and_knockdown_slows_time() {
        let mut flow = FeelFlow::v1();
        let times: Vec<u32> = (0..6)
            .map(|t| flow.step(if t == 0 { &[hit()] } else { &[] }, t).0.time_permille)
            .collect();
        assert_eq!(times, [100, 100, 100, 100, 1000, 1000]);
        let mut flow = FeelFlow::v1();
        let slow = flow.step(&[knockdown()], 0).0.time_permille;
        assert!(slow < 1000, "{slow}");
    }

    #[test]
    fn quiet_events_fire_nothing() {
        let mut flow = FeelFlow::v1();
        let dodge = FightEvent::Dodged { attacker: 0, victim: 1 };
        let out = FightEvent::StaminaOut { fighter: 0 };
        assert_eq!(flow.step(&[dodge, out], 0).0, FeelFrame { time_permille: 1000, ..FeelFrame::default() });
    }

    #[test]
    fn capacity_is_bounded_and_handles_free_slots() {
        let mut flow = FeelFlow::new();
        let mut handles = Vec::new();
        for _ in 0..MAX_EFFECTS {
            handles.push(flow.register(Trigger::Hit, Effect::StopFrame).unwrap());
        }
        assert_eq!(flow.register(Trigger::Hit, Effect::StopFrame), Err(FlowError::Full));
        assert!(flow.unregister(handles[3]));
        assert!(!flow.unregister(handles[3]));
        assert_eq!(flow.len(), MAX_EFFECTS - 1);
        assert_eq!(flow.register(Trigger::Hit, Effect::StopFrame), Ok(handles[3]));
    }

    #[test]
    fn effects_are_deterministic_and_never_allocate() {
        let script = |flow: &mut FeelFlow| {
            (0..90_u32)
                .map(|t| flow.step(if t % 20 == 0 { &[hit(), knockdown()] } else { &[] }, t).0)
                .collect::<Vec<_>>()
        };
        let (a, b) = (script(&mut FeelFlow::v1()), script(&mut FeelFlow::v1()));
        assert_eq!(a, b);
        let mut flow = FeelFlow::v1();
        let (_, heap) = engine_core::alloc_probe::measure(|| {
            std::hint::black_box(script_no_collect(&mut flow));
        });
        assert_eq!(heap.calls, 0);
    }

    fn script_no_collect(flow: &mut FeelFlow) -> u32 {
        let events = [hit(), knockdown()];
        let mut sum = 0_u32;
        for t in 0..90_u32 {
            sum = sum.wrapping_add(flow.step(if t % 20 == 0 { &events } else { &[] }, t).0.time_permille);
        }
        sum
    }

    #[test]
    fn full_slow_mo_slots_report_without_dropping_other_effects() {
        let mut flow = FeelFlow::new();
        for id in 0..5_u8 {
            flow.register(Trigger::Hit, Effect::SlowMo { id, ticks: 9, factor_permille: 500 }).unwrap();
        }
        flow.register(Trigger::Hit, Effect::Shake { ticks: 5, power_permille: 1000 }).unwrap();
        let (frame, status) = flow.step(&[hit()], 0);
        assert_eq!(status, Err(FlowError::SlowMoFull));
        assert_ne!(frame.shake_milli, (0, 0));
    }
}
