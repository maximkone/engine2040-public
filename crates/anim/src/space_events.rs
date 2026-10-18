//! space_events: blend-space cues chosen by integer weight.
//!
//! Owner: `engine-anim`; consumer: the footstep and hit-cue path through
//! [`SpaceCues::footstep_hit`]. A sample's cues ride the event track keyed by
//! its clip id, so a clip and its footfalls are authored together. Units:
//! integer ticks (`u32`), per-sample integer weights summing to
//! [`WEIGHT_SCALE`](super::space::WEIGHT_SCALE). Determinism: integer compares
//! only, canonical sample order, the lowest sample index wins a tie, no clock.
//! Errors: [`EventError`] names a weight row of the wrong length or a tape that
//! runs backwards. Budget: one pass over the samples, the caller owns scratch,
//! zero steady-state allocation once reserved.
//!
//! Call order per animation tick: [`SpacePlayer::tick`](super::space::SpacePlayer::tick),
//! then [`collect_space_cues`] with [`SpacePlayer::weights`](super::space::SpacePlayer::weights).

use super::events::{CallbackLog, EventError, Router};
use super::graph::{EventBank, FiredEvent, NodeId};
use super::space::BlendSpace;

/// Which blended poses speak on a tick. A pose with zero weight never speaks.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum EventStrategy {
    /// Every blended pose speaks, in canonical sample order. Each clip fires
    /// its own cues, so a shared footfall fires once per clip.
    #[default]
    All,
    /// Only the heaviest pose speaks. Equal weights go to the lowest index.
    MaxWeight,
    /// Only the lightest non-zero pose speaks. Equal weights go to the lowest index.
    MinWeight,
}

/// Where one blend space's cues go and how they are chosen. Plain data,
/// built once per space and borrowed for each tick.
#[derive(Clone, Copy, Debug)]
pub struct SpaceCues<'a> {
    node: NodeId,
    events: &'a EventBank,
    router: &'a Router,
    strategy: EventStrategy,
}

impl<'a> SpaceCues<'a> {
    /// Cues reported as `node`, read from `events`, routed by `router`,
    /// chosen by `strategy`.
    pub fn new(
        node: NodeId,
        events: &'a EventBank,
        router: &'a Router,
        strategy: EventStrategy,
    ) -> Self {
        Self {
            node,
            events,
            router,
            strategy,
        }
    }

    /// Footstep and hit cues: the dominant pose speaks, so a shared footfall
    /// in a gait blend fires once instead of once per clip.
    pub fn footstep_hit(node: NodeId, events: &'a EventBank, router: &'a Router) -> Self {
        Self::new(node, events, router, EventStrategy::MaxWeight)
    }

    /// Strategy in force for this space.
    pub fn strategy(&self) -> EventStrategy {
        self.strategy
    }
}

/// Index of the heaviest weight; equal weights keep the lowest index.
fn heaviest(weights: &[u32]) -> Option<usize> {
    let mut best: Option<(usize, u32)> = None;
    for (index, &weight) in weights.iter().enumerate() {
        let replace = match best {
            None => true,
            Some((_, top)) => weight > top,
        };
        if replace {
            best = Some((index, weight));
        }
    }
    best.map(|(index, _)| index)
}

/// Index of the lightest non-zero weight; equal weights keep the lowest index.
fn lightest(weights: &[u32]) -> Option<usize> {
    let mut best: Option<(usize, u32)> = None;
    for (index, &weight) in weights.iter().enumerate() {
        if weight == 0 {
            continue;
        }
        let replace = match best {
            None => true,
            Some((_, low)) => weight < low,
        };
        if replace {
            best = Some((index, weight));
        }
    }
    best.map(|(index, _)| index)
}

/// Record one tick's cues from a blend space.
///
/// `weights` is the row [`SpacePlayer::weights`](super::space::SpacePlayer::weights)
/// holds after this tick's space tick; its length must equal the sample count.
/// Speaking samples push their cues at `tick` into `scratch` (cleared first,
/// capacity kept) and go through [`CallbackLog::collect`], so the tape stays
/// monotonic and every cue is stamped `tick`. Returns the routed count.
pub fn collect_space_cues(
    log: &mut CallbackLog,
    tick: u32,
    cues: &SpaceCues<'_>,
    space: &BlendSpace,
    weights: &[u32],
    scratch: &mut Vec<FiredEvent>,
) -> Result<usize, EventError> {
    if weights.len() != space.len() {
        return Err(EventError::WeightCount {
            samples: space.len(),
            weights: weights.len(),
        });
    }
    let lead = match cues.strategy {
        EventStrategy::All => None,
        EventStrategy::MaxWeight => heaviest(weights),
        EventStrategy::MinWeight => lightest(weights),
    };
    scratch.clear();
    for (index, (sample, &weight)) in space.samples().iter().zip(weights).enumerate() {
        let speaks = weight != 0 && (cues.strategy == EventStrategy::All || lead == Some(index));
        if !speaks {
            continue;
        }
        for event in cues.events.at(sample.clip(), tick) {
            scratch.push(FiredEvent {
                node: cues.node,
                tick,
                code: event.code,
            });
        }
    }
    log.collect(tick, scratch.as_slice(), cues.router)
}
