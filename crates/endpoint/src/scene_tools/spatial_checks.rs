//! `scene.spatial` read path: the wire tables and finding lines the verb
//! reports, plus which frozen catalog checks one call ran, which it could not
//! drive (each with one short reason) and where each staged box got its size.
//!
//! Consumer: `scene_tools::ServerState::scene_brief` (the index-to-UUID and
//! label tables) plus `scene_spatial` (tables, finding lines and the check
//! partition), so a caller can tell a clean scene from a check that never ran.
//! Owner: the 18-name catalog and every check body stay in `engine-spatial`;
//! this module only decides which of those names the door's posting can give
//! an input to — synthetic `prop_NNNN` unit cubes with no velocity, no
//! collider extents and no baked nav view.

use engine_scene::ids::ObjectId;
use rmcp::schemars::JsonSchema;
use rmcp::serde::{Deserialize, Serialize};

use super::object_label;

/// One catalog check that did not run this call, with a short stable reason.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct SkippedCheck {
    /// Frozen catalog check name.
    pub check: String,
    /// Why it could not run; never empty.
    pub reason: String,
}

/// Where each staged node's box size came from: a scene collider component or
/// the 1 m default cube the door posts until real extents are shared.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct SizeSource {
    /// Nodes sized from a collider component.
    pub collider: u64,
    /// Nodes posted as the 1 m default cube.
    pub default: u64,
}

/// Size provenance for a staged scene of `nodes` actors: the door has no
/// collider extents yet, so every node counts as the 1 m default and
/// `collider` stays zero. Errors: none.
pub(crate) fn staged_sizes(nodes: usize) -> SizeSource {
    SizeSource {
        collider: 0,
        default: u64::try_from(nodes).unwrap_or(u64::MAX),
    }
}

/// Why one catalog check has no input to fail on over this staging, or `None`
/// when the door can drive it. `any_missing` is true when at least one staged
/// node carried a missing or non-finite world pose (it posts a NaN actor, so
/// `unknown-position` fires and the id stays in the table).
fn withhold_reason(check: &str, any_missing: bool) -> Option<&'static str> {
    match check {
        "navConnect" | "flowPath" | "accessFront" | "doorClear" => Some("needs_nav_view"),
        "scale" | "giantObject" => Some("no_collider_sizes"),
        "orphan" => Some("no_parent_links"),
        "dupeSpam" => Some("synthetic_unique_names"),
        "grammar" => Some("synthetic_names"),
        "sweep" => Some("no_velocity"),
        "unknown-position" if !any_missing => Some("all_poses_finite"),
        _ => None,
    }
}

/// Split the frozen catalog plus any listed opt-in name into the checks this
/// call runs and the catalog names it must report as skipped, each with one
/// short reason: every catalog name lands on exactly one list, so a caller
/// reads what was checked instead of trusting a bare clean.
///
/// Units: check names, catalog order. `selected` is the caller's `checks`
/// filter (`None` = the whole catalog). Names the caller did not ask for are
/// skipped as `not_requested`; an explicitly requested nav check still runs
/// and keeps its loud `needs_nav_view` refusal from `engine_spatial`.
pub(crate) fn partition<'a>(
    selected: Option<&'a [&'a str]>,
    any_missing: bool,
) -> (Vec<&'a str>, Vec<SkippedCheck>) {
    let mut run: Vec<&'a str> = Vec::new();
    let mut skipped: Vec<SkippedCheck> = Vec::new();
    for name in engine_spatial::SPATIAL_CHECKS {
        let asked = selected.is_none_or(|names| names.contains(&name));
        let reason = if asked {
            withhold_reason(name, any_missing)
        } else {
            Some("not_requested")
        };
        // A requested nav check runs anyway so the refusal stays loud.
        if reason == Some("needs_nav_view") && selected.is_some() {
            run.push(name);
        } else if let Some(reason) = reason {
            skipped.push(SkippedCheck {
                check: name.into(),
                reason: reason.into(),
            });
        } else {
            run.push(name);
        }
    }
    // Opt-in names (`relationCycle`) ride outside the frozen catalog; the door
    // can always drive them over the staged graph.
    for name in selected.into_iter().flatten() {
        if !engine_spatial::SPATIAL_CHECKS.contains(name) {
            run.push(name);
        }
    }
    (run, skipped)
}

/// Bare UUID text per staged object, id-text order: the index-to-UUID table
/// every numeric finding indexes into.
pub(crate) fn id_table(ids: &[ObjectId]) -> Vec<String> {
    ids.iter().map(ObjectId::to_string).collect()
}

/// MCP8 labels for the wire: one `name [id]` entry per staged object,
/// index-aligned with [`id_table`], so numeric findings map to labels by the
/// same position.
pub(crate) fn label_table(ids: &[ObjectId]) -> Vec<String> {
    ids.iter()
        .enumerate()
        .map(|(index, id)| {
            let index = u32::try_from(index).unwrap_or(u32::MAX);
            object_label(index, id)
        })
        .collect()
}
