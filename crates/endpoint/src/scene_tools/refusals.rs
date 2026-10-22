//! Refusal text for a staged spatial finding that a proposal adds. The
//! culprit objects are named in the `name [uuid]` label form that
//! `scene.spatial` prints, so the next proposal can move one of them
//! without a read first.

use engine_editor::commands::Document;
use engine_scene::ids::ObjectId;

use super::labels::object_label;
use super::{snapshot, Snapshot};
use crate::mcp::McpToolError;

/// Label for one culprit id text: its `name [uuid]` from the scene snapshot
/// when the document holds the object, `staged [uuid]` for an object the
/// proposal itself inserts. Unparseable text passes through unchanged.
fn culprit_label(snap: &Snapshot, text: &str) -> String {
    let Ok(id) = ObjectId::parse(text) else {
        return text.to_owned();
    };
    let name = snap
        .ids
        .iter()
        .position(|known| *known == id)
        .and_then(|index| snap.actors.get(index))
        .map_or("staged", |actor| actor.name.as_str());
    object_label(name, &id)
}

/// Refusal for a staged spatial finding the proposal adds. `check` is the
/// failed check (`overlap` or `floating`) and `culprits` the object id texts
/// the editor gate named. The message names every culprit and says the fix.
/// Pure over the document; the document is never written here.
pub(crate) fn spatial_refusal(doc: &Document, check: &str, culprits: &[String]) -> McpToolError {
    let snap = snapshot(doc);
    let labels: Vec<String> = culprits
        .iter()
        .map(|text| culprit_label(&snap, text))
        .collect();
    let message = match (check, labels.as_slice()) {
        ("overlap", [first, second]) => format!(
            "staged spatial check `overlap` failed: the proposal makes {first} overlap \
             {second}; move one of the pair apart (document unchanged)"
        ),
        ("floating", [only]) => format!(
            "staged spatial check `floating` failed: the proposal leaves {only} floating; \
             set its height so its base sits on the floor or on a support (document unchanged)"
        ),
        _ => format!(
            "staged spatial check `{check}` failed: the proposal adds {check}; \
             document unchanged"
        ),
    };
    McpToolError::new("spatial_rejected", message)
}
