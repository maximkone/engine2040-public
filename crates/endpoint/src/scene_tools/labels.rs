//! `name [uuid]` display labels for staged objects, re-exported from
//! `scene_tools` so `scene_tools::object_label` keeps its path.

use engine_scene::ids::ObjectId;

/// Display label for one staged object: its name plus bracketed id,
/// `lamp_2 [<uuid>]`. The bracketed id is the only resolution key; the
/// name part is display-only and never decides placement, so two objects
/// sharing a display name still resolve to different nodes.
///
/// Ownership: pure function, caller owns the returned label. Determinism:
/// same name and id always render the same label.
///
/// # Example
///
/// ```rust
/// use engine_endpoint::scene_tools::{check_scene_op, object_label, parse_object_ref};
///
/// assert!(check_scene_op("move").is_ok());
/// assert!(check_scene_op("insert").is_err());
/// let id = parse_object_ref("object", "00000000-0000-0000-0000-000000000007")
///     .expect("bare UUID parses");
/// let label = object_label("lamp_2", &id);
/// assert_eq!(label, format!("lamp_2 [{id}]"));
/// assert_eq!(
///     parse_object_ref("object", &label).expect("label parses by id"),
///     id
/// );
/// ```
pub fn object_label(name: &str, id: &ObjectId) -> String {
    format!("{name} [{id}]")
}
