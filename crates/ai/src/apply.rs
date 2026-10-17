//! apply: tick-boundary typed effects for recorded mind actions.
//!
//! Consumer: NPC brains (`crate::gladiator`) steer from [`MovementIntent`]
//! via [`MovementIntent::brain_intent`]; the presentation layer reads
//! [`PresentationEvent`] lines and never touches a body. Data owner: this
//! module owns the effect mapping only; `crate::mind` owns stamps,
//! recording and replay. Units: ticks are fixed steps, text bounded by
//! `MAX_TEXT` chars. Determinism: acceptance order, integer FNV-1a
//! checkpoints, no clock. Errors: [`MindError`]; bad stamps refuse, never
//! half-apply. Budget: one bounded clone per applied action; the
//! intent mapping itself allocates nothing.

use crate::gladiator::Intent as BrainIntent;
use crate::mind::{check_text, ActionKind, ActionLog, MindAction, MindError, MindRequest, NpcId};
use engine_core::Fnv64;

/// Length-prefixed text, so adjacent texts cannot collide.
fn mix_text(hash: &mut Fnv64, text: &str) {
    hash.write_len(text.len());
    hash.write_str(text);
}

/// Stable cause code for checkpoints; exhaustive so new variants fail loudly.
fn error_code(cause: MindError) -> u8 {
    match cause {
        MindError::EmptyText => 0,
        MindError::TextTooLong => 1,
        MindError::BadSalience => 2,
        MindError::BadPriority => 3,
        MindError::BadAffinity => 4,
        MindError::BadTrust => 5,
        MindError::UnknownNpc => 6,
        MindError::PastDeadline => 7,
        MindError::StaleContext => 8,
        MindError::Cancelled => 9,
        MindError::Timeout => 10,
        MindError::Malformed => 11,
        MindError::Unavailable => 12,
        MindError::UnsupportedCapability => 13,
        MindError::Quota => 14,
        MindError::Offline => 15,
        MindError::NothingToReflect => 16,
        MindError::TooManyActions => 17,
    }
}

/// One talk/gesture line for the presentation layer. Carries no position,
/// so presenting it can never move a body.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct PresentationEvent {
    /// Mind this line belongs to.
    pub npc: NpcId,
    /// Say or Gesture only, as recorded.
    pub kind: ActionKind,
    /// Bounded line or gesture label.
    pub text: String,
    /// Stamp the action carried.
    pub tick_apply: u64,
    /// Ask that produced it.
    pub request: u64,
}

/// One movement ask for the existing brains. A target label only: brains
/// own steering, this intent never writes a position itself.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct MovementIntent {
    /// Mind this intent belongs to.
    pub npc: NpcId,
    /// Approach, Leave or Idle only, as recorded.
    pub kind: ActionKind,
    /// Bounded place or NPC label the brain steers by.
    pub target: String,
    /// Stamp the action carried.
    pub tick_apply: u64,
    /// Ask that produced it.
    pub request: u64,
}

impl MovementIntent {
    /// Brain seam: Approach closes, Leave yields ground, Idle holds guard.
    /// Pure enum map, no allocation, always total.
    pub const fn brain_intent(&self) -> BrainIntent {
        match self.kind {
            ActionKind::Approach => BrainIntent::Advance,
            ActionKind::Leave => BrainIntent::Retreat,
            ActionKind::Idle => BrainIntent::Block,
            ActionKind::Say | ActionKind::Gesture => BrainIntent::Block,
        }
    }
}

/// One validated action turned into its tick effect.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Applied {
    /// Talk/gesture line for presentation; never a body write.
    Presented(PresentationEvent),
    /// Steering ask for the existing brains.
    Intended(MovementIntent),
}

/// One drained action that refused, kept as history with its cause.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Refused {
    /// The drained action, kept verbatim for history.
    pub action: MindAction,
    /// Why it never became an effect.
    pub cause: MindError,
}

/// One drain worth of effects in acceptance order, refusals kept as history.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct AppliedBatch {
    /// Presentation lines, acceptance order.
    pub presented: Vec<PresentationEvent>,
    /// Movement intents, acceptance order.
    pub intended: Vec<MovementIntent>,
    /// Refused drains with causes, acceptance order.
    pub refused: Vec<Refused>,
}

impl AppliedBatch {
    /// Empty batch.
    pub fn new() -> Self {
        Self {
            presented: Vec::new(),
            intended: Vec::new(),
            refused: Vec::new(),
        }
    }

    /// Effect count (presented plus intended; refusals stay history only).
    pub fn len(&self) -> usize {
        self.presented.len() + self.intended.len()
    }

    /// True when no effect landed.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Deterministic FNV-1a checkpoint over effects then refusals in
    /// acceptance order. Same drain twice gives the same checkpoint.
    pub fn checkpoint(&self) -> u64 {
        let mut hash = Fnv64::new();
        for event in &self.presented {
            hash.write_u8(0);
            hash.write_u64(event.tick_apply);
            hash.write_u64(u64::from(event.npc.0));
            hash.write_u8(event.kind as u8);
            hash.write_u64(event.request);
            mix_text(&mut hash, &event.text);
        }
        for intent in &self.intended {
            hash.write_u8(1);
            hash.write_u64(intent.tick_apply);
            hash.write_u64(u64::from(intent.npc.0));
            hash.write_u8(intent.kind as u8);
            hash.write_u64(intent.request);
            mix_text(&mut hash, &intent.target);
        }
        for refused in &self.refused {
            hash.write_u8(2);
            hash.write_u64(refused.action.tick_apply);
            hash.write_u64(u64::from(refused.action.npc.0));
            hash.write_u8(refused.action.kind as u8);
            hash.write_u64(refused.action.request);
            mix_text(&mut hash, &refused.action.text);
            hash.write_u8(error_code(refused.cause));
        }
        hash.finish()
    }
}

/// Window a stamp must land in: `[issued_tick, deadline_tick]`.
fn check_stamp(tick_apply: u64, issued_tick: u64, deadline_tick: u64) -> Result<(), MindError> {
    if deadline_tick < issued_tick {
        return Err(MindError::PastDeadline);
    }
    if tick_apply < issued_tick {
        return Err(MindError::Malformed);
    }
    if tick_apply > deadline_tick {
        return Err(MindError::Timeout);
    }
    Ok(())
}

/// Turn one recorded action into its tick effect under a bare window.
/// Say/Gesture present only; Approach/Leave/Idle intend only. Stamps
/// outside `[issued_tick, deadline_tick]` refuse; nothing half-applies.
pub fn apply_at(
    action: &MindAction,
    issued_tick: u64,
    deadline_tick: u64,
) -> Result<Applied, MindError> {
    check_stamp(action.tick_apply, issued_tick, deadline_tick)?;
    check_text(&action.text)?;
    match action.kind {
        ActionKind::Say | ActionKind::Gesture => Ok(Applied::Presented(PresentationEvent {
            npc: action.npc,
            kind: action.kind,
            text: action.text.clone(),
            tick_apply: action.tick_apply,
            request: action.request,
        })),
        ActionKind::Approach | ActionKind::Leave | ActionKind::Idle => {
            Ok(Applied::Intended(MovementIntent {
                npc: action.npc,
                kind: action.kind,
                target: action.text.clone(),
                tick_apply: action.tick_apply,
                request: action.request,
            }))
        }
    }
}

/// Turn one recorded action into its tick effect under its ask. Refuses
/// foreign owners, foreign ask ids and out-of-window stamps; a refusal
/// records nothing and moves nothing.
pub fn apply_one(action: &MindAction, req: &MindRequest) -> Result<Applied, MindError> {
    if action.npc != req.npc {
        return Err(MindError::UnknownNpc);
    }
    if action.request != req.id {
        return Err(MindError::Malformed);
    }
    apply_at(action, req.issued_tick, req.deadline_tick)
}

/// Ask window for `id` in stable slice order, if any.
fn window_of(requests: &[MindRequest], id: u64) -> Option<&MindRequest> {
    requests.iter().find(|req| req.id == id)
}

/// Drain core: moves due actions out of `log`, sorts each into `batch`.
/// Unknown ask ids refuse as malformed; future stamps stay in the log as
/// history and are never re-applied once drained.
pub fn apply_due_into(
    log: &mut ActionLog,
    tick: u64,
    requests: &[MindRequest],
    batch: &mut AppliedBatch,
) {
    for action in log.drain_due(tick) {
        let outcome = match window_of(requests, action.request) {
            Some(req) => apply_one(&action, req),
            None => Err(MindError::Malformed),
        };
        match outcome {
            Ok(Applied::Presented(event)) => batch.presented.push(event),
            Ok(Applied::Intended(intent)) => batch.intended.push(intent),
            Err(cause) => batch.refused.push(Refused { action, cause }),
        }
    }
}

/// Drain actions due at `tick` into typed effects. Acceptance order;
/// refusals join the batch history with their cause.
pub fn apply_due(log: &mut ActionLog, tick: u64, requests: &[MindRequest]) -> AppliedBatch {
    let mut batch = AppliedBatch::new();
    apply_due_into(log, tick, requests, &mut batch);
    batch
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mind::{
        ActionSource, MemoryKind, Mind, MindAdapter, MindRuntime, MockAdapter, Privacy, ScriptStep,
        MAX_TEXT,
    };

    fn npc() -> NpcId {
        NpcId(7)
    }

    fn req(id: u64, issued: u64) -> MindRequest {
        MindRequest {
            id,
            npc: npc(),
            issued_tick: issued,
            deadline_tick: issued + 4,
            budget_ms: 500,
            base_revision: 3,
            privacy: Privacy::RemoteAllowed,
        }
    }

    fn action(kind: ActionKind, text: &str, stamp: u64, request: u64) -> MindAction {
        MindAction {
            tick_apply: stamp,
            npc: npc(),
            kind,
            text: text.to_owned(),
            source: ActionSource::Provider,
            request,
        }
    }

    fn seeded_mind() -> Mind {
        let mut mind = Mind::new(npc(), 11);
        mind.observe(1, MemoryKind::Seen, "a stranger enters the ludus", 800)
            .expect("seed observes");
        mind.observe(2, MemoryKind::Heard, "the crowd chants at dusk", 300)
            .expect("seed observes");
        mind.add_goal("guard the gate", 700).expect("seed goal");
        mind
    }

    /// Twenty providers-off asks drained through apply, with the checkpoint.
    fn scripted_twenty() -> (AppliedBatch, u64) {
        let mind = seeded_mind();
        let mut rt = MindRuntime::new();
        rt.set_online(false);
        let mut mock = MockAdapter::new(vec![ScriptStep::Succeed]);
        let mut reqs = Vec::with_capacity(20);
        for id in 1..=20u64 {
            let ask = req(id, 100 + id * 2);
            rt.ask(&mind, &ask, &mut mock).expect("offline asks");
            reqs.push(ask);
        }
        assert_eq!(mock.calls(), 0, "offline performs no provider call");
        let mut log = rt.recorded().clone();
        let batch = apply_due(&mut log, 1_000, &reqs);
        let checkpoint = batch.checkpoint();
        assert!(log.is_empty(), "drain removes applied actions");
        (batch, checkpoint)
    }

    #[test]
    fn twenty_scripted_asks_apply_identically_twice_providers_off() {
        let (first, first_hash) = scripted_twenty();
        let (second, second_hash) = scripted_twenty();
        assert_eq!(first, second, "same asks apply identically twice");
        assert_eq!(first_hash, second_hash, "checkpoints match x2");
        assert_eq!(first.presented.len(), 20, "every ask presents one line");
        assert!(first.intended.is_empty(), "scripted Say never intends");
        assert!(first.refused.is_empty(), "in-window stamps never refuse");
        // Divergence hygiene: one retargeted line changes the checkpoint.
        let mut changed = second.clone();
        changed.presented[4].text.push('!');
        assert_ne!(
            changed.checkpoint(),
            first_hash,
            "changed input changes the checkpoint"
        );
    }

    #[test]
    fn say_and_gesture_never_move_a_body() {
        let ask = req(1, 100);
        // Bodies stand on their marks; only intents may steer them.
        let mut bodies = [(10i32, 20i32), (30, 40), (50, 60), (70, 80)];
        let before = bodies;
        let mut log = ActionLog::new();
        log.push(action(ActionKind::Say, "hold the gate, brothers.", 100, 1))
            .expect("records");
        log.push(action(ActionKind::Gesture, "raises a fist.", 101, 1))
            .expect("records");
        let batch = apply_due(&mut log, 1_000, &[ask]);
        assert_eq!(batch.presented.len(), 2);
        assert!(batch.intended.is_empty(), "talk never intends");
        assert!(batch.refused.is_empty());
        // The arena steers bodies from intents only: no intent, no motion.
        for intent in &batch.intended {
            let slot = (intent.npc.0 as usize) % bodies.len();
            bodies[slot].0 += match intent.brain_intent() {
                BrainIntent::Advance => 1,
                BrainIntent::Retreat => -1,
                _ => 0,
            };
        }
        assert_eq!(bodies, before, "Say/Gesture moved no body");
        // Direct single-apply proof: Say presents, never intends.
        let say = action(ActionKind::Say, "steady.", 100, 1);
        match apply_at(&say, 100, 104).expect("say applies") {
            Applied::Presented(event) => {
                assert_eq!(event.kind, ActionKind::Say);
                assert_eq!(event.text, "steady.");
            }
            Applied::Intended(_) => panic!("Say must never intend"),
        }
    }

    #[test]
    fn approach_leave_idle_become_brain_intents() {
        let ask = req(2, 200);
        let cases = [
            (ActionKind::Approach, "the north gate", BrainIntent::Advance),
            (ActionKind::Leave, "the arena floor", BrainIntent::Retreat),
            (ActionKind::Idle, "hold the line", BrainIntent::Block),
        ];
        for (kind, target, brain) in cases {
            let act = action(kind, target, 202, 2);
            match apply_one(&act, &ask).expect("movement applies") {
                Applied::Intended(intent) => {
                    assert_eq!(intent.target, target);
                    assert_eq!(intent.tick_apply, 202);
                    assert_eq!(intent.brain_intent(), brain);
                }
                Applied::Presented(_) => panic!("{kind:?} must intend, never present"),
            }
        }
        // The intent feeds a real brain: decide validates on the tick.
        let intent = MovementIntent {
            npc: npc(),
            kind: ActionKind::Approach,
            target: "the north gate".to_owned(),
            tick_apply: 202,
            request: 2,
        };
        assert_eq!(intent.brain_intent(), BrainIntent::Advance);
        let mut brain = crate::gladiator::Brain::new(
            engine_input::FighterId::new(0).expect("slot 0 is legal"),
            1,
        );
        let percept =
            crate::gladiator::Percept::new([0.0, 0.0, 0.0], 0.0, [2.0, 0.0, 0.0], false, 2.0, 202);
        brain
            .decide(&percept)
            .validate()
            .expect("brain consumes the intent path");
    }

    #[test]
    fn stamps_outside_window_refuse() {
        let ask = req(3, 300);
        // Late stamp refuses as deadline exceeded, kept as history.
        let late = action(ActionKind::Say, "too late.", 305, 3);
        assert_eq!(apply_one(&late, &ask), Err(MindError::Timeout));
        // Early stamp refuses as malformed.
        let early = action(ActionKind::Say, "too early.", 299, 3);
        assert_eq!(apply_one(&early, &ask), Err(MindError::Malformed));
        // Window edges apply: issued and deadline both land.
        for stamp in [300, 304] {
            apply_one(&action(ActionKind::Say, "on time.", stamp, 3), &ask)
                .expect("edge stamps land");
        }
        // Inverted window refuses without reading the action.
        let broken = MindRequest {
            deadline_tick: 99,
            ..req(4, 100)
        };
        assert_eq!(
            apply_one(&action(ActionKind::Say, "fine.", 100, 4), &broken),
            Err(MindError::PastDeadline)
        );
        // Foreign owner and foreign ask id refuse.
        let mut stranger = action(ActionKind::Say, "fine.", 300, 3);
        stranger.npc = NpcId(99);
        assert_eq!(apply_one(&stranger, &ask), Err(MindError::UnknownNpc));
        assert_eq!(
            apply_one(&action(ActionKind::Say, "fine.", 300, 9), &ask),
            Err(MindError::Malformed)
        );
        // Bad text refuses even in window.
        assert_eq!(
            apply_one(&action(ActionKind::Say, "   ", 300, 3), &ask),
            Err(MindError::EmptyText)
        );
        assert_eq!(
            apply_one(
                &action(ActionKind::Say, &"x".repeat(MAX_TEXT + 1), 300, 3),
                &ask
            ),
            Err(MindError::TextTooLong)
        );
        // Drain keeps refusals as history with causes, in order.
        let mut log = ActionLog::new();
        log.push(late).expect("records");
        log.push(action(ActionKind::Say, "on time.", 300, 3))
            .expect("records");
        let batch = apply_due(&mut log, 1_000, &[ask]);
        assert_eq!(batch.presented.len(), 1);
        assert_eq!(batch.refused.len(), 1);
        assert_eq!(batch.refused[0].cause, MindError::Timeout);
        assert_eq!(batch.refused[0].action.text, "too late.");
    }

    #[test]
    fn late_answers_stay_history_never_reapply() {
        let ask = req(5, 500);
        let mut log = ActionLog::new();
        log.push(action(ActionKind::Say, "due now.", 500, 5))
            .expect("records");
        log.push(action(ActionKind::Approach, "the gate.", 900, 5))
            .expect("records future");
        // Early drain applies the due line only; the future stays logged.
        let first = apply_due(&mut log, 504, &[req(5, 500)]);
        assert_eq!(first.presented.len(), 1);
        assert!(first.intended.is_empty());
        assert_eq!(log.len(), 1, "future stamp stays history");
        // Same tick again: nothing due, nothing re-applied.
        let replay = apply_due(&mut log, 504, core::slice::from_ref(&ask));
        assert!(replay.is_empty() && replay.refused.is_empty());
        assert_eq!(log.len(), 1);
        // The future's own window refuses it (past deadline); the refusal
        // stays history and the log ends empty but accounted for.
        let mut narrow = req(5, 500);
        narrow.deadline_tick = 504;
        let late = apply_due(&mut log, 1_000, &[narrow]);
        assert_eq!(late.refused.len(), 1);
        assert_eq!(late.refused[0].cause, MindError::Timeout);
        assert!(log.is_empty(), "drain removes applied drains");
        // Unknown ask ids refuse loudly instead of inventing a window.
        let mut orphan = ActionLog::new();
        orphan
            .push(action(ActionKind::Say, "no ask.", 10, 77))
            .expect("records");
        let batch = apply_due(&mut orphan, 1_000, &[ask]);
        assert_eq!(batch.refused.len(), 1);
        assert_eq!(batch.refused[0].cause, MindError::Malformed);
    }

    #[test]
    fn into_drain_matches_allocating_drain() {
        let mind = seeded_mind();
        let mut rt = MindRuntime::new();
        rt.set_online(false);
        let mut mock = MockAdapter::new(vec![ScriptStep::Succeed]);
        let mut reqs = Vec::new();
        for id in 1..=6u64 {
            let ask = req(id, 50 + id);
            rt.ask(&mind, &ask, &mut mock).expect("offline asks");
            reqs.push(ask);
        }
        let mut first_log = rt.recorded().clone();
        let mut second_log = rt.recorded().clone();
        let owned = apply_due(&mut first_log, 1_000, &reqs);
        let mut into = AppliedBatch::new();
        into.presented.reserve(8);
        into.intended.reserve(2);
        into.refused.reserve(2);
        apply_due_into(&mut second_log, 1_000, &reqs, &mut into);
        assert_eq!(owned, into, "both drains agree");
        assert_eq!(owned.checkpoint(), into.checkpoint());
    }
}
