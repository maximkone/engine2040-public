//! Component staging for the command service: set and remove one typed
//! component on a node, with the inverse that undo replays.
//!
//! Owner: the command service (`super`); data owner is
//! [`Document::components`](super::Document::components). Units: none (the
//! values carry their own). Determinism: the table is a pair of `BTreeMap`s,
//! so staging order never moves a digest. Errors: [`BatchError`] naming the
//! node and kind; nothing is written on a refusal because the caller stages
//! on a private copy.

use std::collections::BTreeMap;

use engine_scene::component::{Component, ComponentKind};
use engine_scene::ids::ObjectId;
use engine_scene::transform::Hierarchy;

use super::{BatchError, ComponentTable, Op};

/// Stage a component set; the inverse restores the old value of that kind,
/// or removes the kind when the node had none.
pub(super) fn stage_set_component(
    tree: &Hierarchy,
    components: &mut ComponentTable,
    id: ObjectId,
    component: Component,
) -> Result<(Vec<Op>, String), BatchError> {
    if !tree.contains(id) {
        return Err(BatchError::UnknownNode { id });
    }
    let kind = component.kind();
    component
        .validate()
        .map_err(|_| BatchError::BadComponent { id, kind })?;
    let slot = components.entry(id.to_string()).or_default();
    let old = slot.insert(kind, component);
    let line = match old {
        Some(from) => format!(
            "set-component {id} {kind} from [{}] to [{}]",
            from.describe(),
            component.describe()
        ),
        None => format!("set-component {id} add [{}]", component.describe()),
    };
    let inverse = match old {
        Some(previous) => Op::SetComponent {
            id,
            component: previous,
        },
        None => Op::RemoveComponent { id, kind },
    };
    Ok((vec![inverse], line))
}

/// Stage a component removal; the inverse sets the removed value back. A
/// kind the node does not carry refuses, so a typo never reads as success.
pub(super) fn stage_remove_component(
    tree: &Hierarchy,
    components: &mut ComponentTable,
    id: ObjectId,
    kind: ComponentKind,
) -> Result<(Vec<Op>, String), BatchError> {
    if !tree.contains(id) {
        return Err(BatchError::UnknownNode { id });
    }
    let key = id.to_string();
    let removed = components
        .get_mut(&key)
        .and_then(|slot| slot.remove(&kind))
        .ok_or(BatchError::MissingComponent { id, kind })?;
    if components.get(&key).is_some_and(BTreeMap::is_empty) {
        components.remove(&key);
    }
    let line = format!(
        "remove-component {id} {kind} (was [{}])",
        removed.describe()
    );
    Ok((
        vec![Op::SetComponent {
            id,
            component: removed,
        }],
        line,
    ))
}
