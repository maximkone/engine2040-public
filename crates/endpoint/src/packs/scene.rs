//! `scene` pack: layout authoring beyond the core transaction tools.
//!
//! Loaded by default: the door's own domain. Holds the brief, spatial
//! findings, intent placement, brief-to-level build, spatial questions and
//! the digest watch. Component verbs (typed `set_component` /
//! `remove_component` ops) ride `tx.preview` / `tx.apply` in the core.

use super::{PackDef, PackError, VerbSpec};
use crate::build_level::{BuildLevelTool, TOOL_BUILD_LEVEL};
use crate::discover::{WatchTool, TOOL_SCENE_WATCH};
use crate::scene_query::{SceneQueryTool, TOOL_SCENE_QUERY};
use crate::scene_tools::{
    SceneBriefTool, ScenePlaceTool, SceneSpatialTool, TOOL_SCENE_BRIEF, TOOL_SCENE_PLACE,
    TOOL_SCENE_SPATIAL,
};
use crate::tool_capabilities::CapabilityCard;
use engine_scene::component::ComponentKind;

/// Pack name.
pub const PACK: &str = "scene";

/// One component schema as the index lists it: the JSON shape and the
/// legal ranges, so `api.describe component:<kind>` answers without a source
/// read. Authored through `set_component` ops in `tx.preview` / `tx.apply`.
pub fn component_doc(kind: ComponentKind) -> (&'static str, &'static str) {
    match kind {
        ComponentKind::Mesh => (
            r#"{"asset": name, "cast_shadows": bool=true}"#,
            "Mesh asset to draw. Names are 1..=64 bytes of 0-9 A-Z a-z - _ . /.",
        ),
        ComponentKind::Collider => (
            r#"{"shape": "box"|"sphere"|"capsule", "half": [x,y,z], "sensor": bool=false}"#,
            "Collision shape. half is meters: box half extents, sphere radius in x, capsule radius in x and half height in y; each in 0.0001..=100000. A sensor reports overlaps and never blocks.",
        ),
        ComponentKind::Material => (
            r#"{"asset": name, "tint": [r,g,b]=[1,1,1]}"#,
            "Material reference plus a linear RGB tint, each channel 0..=1.",
        ),
        ComponentKind::Light => (
            r#"{"kind": "point"|"spot"|"directional", "color": [r,g,b], "intensity": n, "range": n}"#,
            "Light source. color channels 0..=1, intensity 0..=1000000, range meters 0..=100000 (ignored by directional lights).",
        ),
        ComponentKind::Spawner => (
            r#"{"archetype": name, "count": n, "period_ticks": n}"#,
            "Entity spawner. count 1..=65535 entities per burst, period_ticks at least 1 (integer ticks, never seconds).",
        ),
        ComponentKind::Destructible => (
            r#"{"material": name, "health": n, "chunks": n}"#,
            "Destructible body. health is an integer of at least 1; chunks is the fracture budget 1..=4096; the material names the physical table row.",
        ),
        ComponentKind::Anatomy => (
            r#"{"template": name, "limbs": n}"#,
            "Body plan: anatomy template name and limb count 1..=32.",
        ),
        ComponentKind::Rig => (
            r#"{"skeleton": name, "clips": name}"#,
            "Skeleton asset plus clip set asset.",
        ),
    }
}

/// The scene pack: six verbs, loaded by default.
pub fn pack() -> Result<PackDef, PackError> {
    let mut pack = PackDef::new(
        PACK,
        "layout authoring: brief, spatial findings, intent placement, brief-to-level build, \
         spatial questions and the digest watch",
        true,
    );
    pack.add(VerbSpec::of::<SceneBriefTool>(
        TOOL_SCENE_BRIEF,
        PACK,
        CapabilityCard::Observe,
    )?);
    pack.add(VerbSpec::of::<SceneSpatialTool>(
        TOOL_SCENE_SPATIAL,
        PACK,
        CapabilityCard::Observe,
    )?);
    pack.add(VerbSpec::of::<ScenePlaceTool>(
        TOOL_SCENE_PLACE,
        PACK,
        CapabilityCard::Commit,
    )?);
    pack.add(VerbSpec::of::<BuildLevelTool>(
        TOOL_BUILD_LEVEL,
        PACK,
        CapabilityCard::Commit,
    )?);
    pack.add(VerbSpec::of::<SceneQueryTool>(
        TOOL_SCENE_QUERY,
        PACK,
        CapabilityCard::Observe,
    )?);
    pack.add(VerbSpec::of::<WatchTool>(
        TOOL_SCENE_WATCH,
        PACK,
        CapabilityCard::Observe,
    )?);
    Ok(pack)
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_scene::component::ALL_KINDS;

    /// Every kind documents a non-empty shape plus prose; the index answers
    /// `api.describe component:<kind>` from this table alone, so a blank or
    /// missing row would serve an empty answer.
    #[test]
    fn component_doc_covers_every_kind() {
        for kind in ALL_KINDS {
            let (shape, docs) = component_doc(kind);
            assert!(!shape.is_empty(), "{kind:?} shape is empty");
            assert!(!docs.is_empty(), "{kind:?} docs are empty");
        }
        let (mesh_shape, _) = component_doc(ComponentKind::Mesh);
        assert!(
            mesh_shape.contains("asset"),
            "mesh shape names the asset: {mesh_shape}"
        );
    }
}
