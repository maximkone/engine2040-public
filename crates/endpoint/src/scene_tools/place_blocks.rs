//! SPR6: the solver's per-want blockers mapped to the `scene.place` door shape.
//!
//! Consumer: `scene.place` conflict answers (`blocked_by`, beside the repair
//! hints). Names come from the snapshot the solve ran on (`name [id]` labels);
//! every figure stays integer centimeters. Honesty: when the mover or the
//! blocker has no collider, its box is the labelled default, and the fix says
//! so instead of letting the figure pose as a measurement. Determinism: the
//! same snapshot and conflict always map to the same DTOs in conflict order.

use rmcp::schemars::JsonSchema;
use rmcp::serde::{Deserialize, Serialize};

use super::{object_label, Snapshot};
use crate::intents::wall_word;

/// Why one unmet want of a `scene.place` conflict failed, in `conflicts` order.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct WantBlockDto {
    /// Index into `conflicts`.
    pub want: usize,
    /// Blocking object `name [id]`, `wall north` or `point x,z`.
    pub blocker: String,
    /// Plan overlap in cm when an object blocks, else 0.
    pub overlap_cm: i32,
    /// cm the nearest clean pose that keeps the other wants misses by.
    pub miss_cm: i32,
    /// That pose, `[x_cm, z_cm]`.
    pub near_miss_cm: [i32; 2],
    /// The fix, one sentence (it names any box that is the default size).
    pub fix: String,
}

/// Name the boxes the figure assumed when `mover` or the blocker has no
/// collider; empty when both were measured.
fn default_note(snap: &Snapshot, mover: u32, blocker: Option<(&str, u32)>) -> String {
    let sized = |actor: u32| snap.is_default(usize::try_from(actor).unwrap_or(usize::MAX));
    let mut names: Vec<String> = Vec::new();
    if sized(mover) {
        names.push("this node".to_string());
    }
    if let Some((label, id)) = blocker {
        if sized(id) {
            names.push(label.to_string());
        }
    }
    match names.len() {
        0 => String::new(),
        1 => format!(
            "; {} has the default size (no collider), so this figure assumes that box",
            names[0]
        ),
        _ => format!(
            "; {} have the default size (no collider), so this figure assumes those boxes",
            names.join(" and ")
        ),
    }
}

pub(super) fn block_output(
    snap: &Snapshot,
    conflict: &engine_spatial::ConflictSet,
    mover: u32,
) -> Vec<WantBlockDto> {
    use engine_spatial::place::Blocker;
    let label = |actor: u32| -> String {
        let at = usize::try_from(actor).ok();
        match at.and_then(|at| snap.actors.get(at).zip(snap.ids.get(at))) {
            Some((entry, id)) => object_label(&entry.name, id),
            None => format!("object {actor}"),
        }
    };
    conflict
        .blocks
        .iter()
        .map(|block| {
            let [x, z] = block.near_miss;
            let miss = block.miss_cm;
            let pose = format!("[{x}, {z}] cm");
            // Contradicting wants: the want misses what it measures against.
            let misses = |what: &str| {
                format!(
                    "the nearest clean pose {pose} that keeps the other wants misses {what} \
                     by {miss} cm; widen this want's range by that much or drop one of the wants"
                )
            };
            // `sized` is the object whose box the figure rests on, when one.
            let (name, overlap_cm, sentence, sized) = match block.blocker {
                Blocker::Object {
                    id,
                    overlap_cm,
                    fix,
                } => {
                    let name = label(id);
                    let sentence = format!(
                        "move {name} {} cm {} to free the spot, or drop this want; \
                         the nearest clean pose {pose} misses it by {miss} cm",
                        fix.cm, fix.dir
                    );
                    (name, overlap_cm, sentence, Some(id))
                }
                Blocker::Wall(wall) => {
                    let name = format!("wall {}", wall_word(wall));
                    let sentence = misses(&name);
                    (name, 0, sentence, None)
                }
                Blocker::Target(id) => {
                    let name = label(id);
                    let sentence = misses(&name);
                    (name, 0, sentence, Some(id))
                }
                Blocker::Point { x_cm, z_cm } => {
                    let name = format!("point {x_cm},{z_cm}");
                    let sentence = misses(&name);
                    (name, 0, sentence, None)
                }
            };
            let note = default_note(snap, mover, sized.map(|id| (name.as_str(), id)));
            WantBlockDto {
                want: block.want,
                blocker: name,
                overlap_cm,
                miss_cm: miss,
                near_miss_cm: block.near_miss,
                fix: format!("{sentence}{note}"),
            }
        })
        .collect()
}
