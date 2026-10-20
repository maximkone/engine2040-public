//! `scene.spatial` read path: the wire tables and finding lines the verb
//! reports, plus which frozen catalog checks one call ran, which it could not
//! drive (each with one short reason) and where each staged box got its size.
//!
//! Consumer: `scene_tools::ServerState::scene_brief` (the index-to-UUID and
//! label tables) plus `scene_spatial` (tables, finding lines and the check
//! partition), so a caller can tell a clean scene from a check that never ran.
//! Owner: the 18-name catalog and every check body stay in `engine-spatial`;
//! this module only decides which of those names the door's posting can give
//! an input to: boxes named after their mesh asset (`lamp_2`), sized from a
//! collider component or the 1 m default, with no velocity and no baked nav
//! view.

use std::collections::{BTreeMap, BTreeSet};

use engine_editor::commands::Document;
use engine_scene::component::{ColliderShape, Component, ComponentKind};
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

/// A count as `u64`, saturating.
pub(crate) fn wide(n: usize) -> u64 {
    u64::try_from(n).unwrap_or(u64::MAX)
}

/// An index or cursor as `usize`, saturating.
pub(crate) fn index_of(n: u32) -> usize {
    usize::try_from(n).unwrap_or(usize::MAX)
}

/// Findings per actor for the worst-actor rollup.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct WorstActor {
    /// `name [id]` label of the actor.
    pub id: String,
    /// Findings that name this actor.
    pub findings: u32,
    /// Red findings among them.
    pub red: u32,
    /// Check names that fired on it, sorted.
    pub checks: Vec<String>,
}

/// Finding counts of one `scene.spatial` call.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct SpatialCounts {
    /// Findings after the `matched_only` filter, across all pages.
    pub total: u64,
    /// Finding rows in this reply.
    pub returned: u64,
    /// Findings held back: `total - returned`.
    pub suppressed: u64,
}

/// Actors in the worst-actor rollup.
const WORST_ACTORS: usize = 10;

/// Top actors by finding count (red count, then table index, break ties), each
/// with the checks that fired on it. A finding counts for both its actors.
/// Pure: the same findings in any order give the same rollup.
pub(crate) fn worst_actors(
    findings: &[&engine_spatial::Finding],
    ids: &[ObjectId],
    actors: &[engine_spatial::Actor],
) -> Vec<WorstActor> {
    // Per actor: findings it is in, how many are red, the checks that fired.
    type Tally<'a> = (u32, u32, BTreeSet<&'a str>);
    let mut per: BTreeMap<usize, Tally<'_>> = BTreeMap::new();
    for finding in findings {
        for index in [finding.a, finding.b] {
            let Some(index) = usize::try_from(index).ok().filter(|i| *i < actors.len()) else {
                continue;
            };
            let entry = per.entry(index).or_default();
            entry.0 += 1;
            entry.1 += u32::from(finding.severity == "red");
            entry.2.insert(finding.check);
        }
    }
    let mut rows: Vec<(usize, Tally<'_>)> = per.into_iter().collect();
    rows.sort_by_key(|(index, (count, red, _))| (std::cmp::Reverse((*count, *red)), *index));
    rows.into_iter()
        .take(WORST_ACTORS)
        .map(|(index, (count, red, checks))| WorstActor {
            id: object_label(&actors[index].name, &ids[index]),
            findings: count,
            red,
            checks: checks.into_iter().map(str::to_owned).collect(),
        })
        .collect()
}

/// Most chars kept from a mesh asset name in a label base.
const NAME_BASE_CHARS: usize = 40;

/// Label base of one node: its Mesh asset name cut to a bracket-free token
/// (ASCII letters, digits, `_`, `-`, `.`; anything else becomes `_`), or
/// `prop` when the node carries no mesh. Pure; same document, same base.
fn name_base(doc: &Document, id: ObjectId) -> String {
    let mesh = doc
        .components
        .get(&id.to_string())
        .and_then(|kinds| kinds.get(&ComponentKind::Mesh));
    let Some(Component::Mesh(mesh)) = mesh else {
        return "prop".to_owned();
    };
    let token: String = mesh
        .asset
        .as_str()
        .chars()
        .take(NAME_BASE_CHARS)
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.') {
                c
            } else {
                '_'
            }
        })
        .collect();
    if token.is_empty() {
        "prop".to_owned()
    } else {
        token
    }
}

/// Box size in meters from the node's Collider component times the world
/// scale, or `None` when it has no collider (the 1 m default cube stands in).
/// Sphere and capsule collapse to their bounding box.
fn collider_size(doc: &Document, id: ObjectId, scale: [f32; 3]) -> Option<[f32; 3]> {
    let kinds = doc.components.get(&id.to_string())?;
    let Some(Component::Collider(c)) = kinds.get(&ComponentKind::Collider) else {
        return None;
    };
    let [a, b, d] = c.half;
    let own = match c.shape {
        ColliderShape::Box => [2.0 * a, 2.0 * b, 2.0 * d],
        ColliderShape::Sphere => [2.0 * a, 2.0 * a, 2.0 * a],
        ColliderShape::Capsule => [2.0 * a, 2.0 * (a + b), 2.0 * a],
    };
    Some([
        own[0] * scale[0].abs(),
        own[1] * scale[1].abs(),
        own[2] * scale[2].abs(),
    ])
}

/// Labels and sizes for the staged nodes, in the order given (id-text sort).
/// Names are `<mesh asset>_<n>` with `n` counting 1.. per base name in that
/// order, so a node's label moves only when a node of the SAME base name
/// sorts before it; an unrelated insert never relabels anything. `scales`
/// holds each node's world scale, index-aligned with `ids`. Returns the
/// names, the box sizes (1 m default without a collider) and where the sizes
/// came from.
pub(crate) fn staged_names_sizes(
    doc: &Document,
    ids: &[ObjectId],
    scales: &[[f32; 3]],
) -> (Vec<String>, Vec<[f32; 3]>, SizeSource) {
    let mut seen: BTreeMap<String, u32> = BTreeMap::new();
    let mut names = Vec::with_capacity(ids.len());
    let mut sizes = Vec::with_capacity(ids.len());
    let mut source = SizeSource::default();
    for (id, scale) in ids.iter().zip(scales) {
        let base = name_base(doc, *id);
        let n = seen.entry(base.clone()).or_insert(0);
        *n += 1;
        names.push(format!("{base}_{n}"));
        match collider_size(doc, *id, *scale).filter(|size| size.iter().all(|v| v.is_finite())) {
            Some(size) => {
                source.collider += 1;
                sizes.push(size);
            }
            None => {
                source.default += 1;
                sizes.push([1.0; 3]);
            }
        }
    }
    (names, sizes, source)
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

/// One page of the wire tables: bare UUIDs, `name [id]` labels and pose rows,
/// index-aligned, starting at `from`.
pub(crate) struct TablePage {
    /// Bare UUID text per object.
    pub(crate) ids: Vec<String>,
    /// `name [id]` labels, aligned with `ids`.
    pub(crate) objects: Vec<String>,
    /// Pose rows, aligned with `ids`.
    pub(crate) poses: Vec<String>,
    /// Index of the first object on the next page, `None` on the last page.
    pub(crate) next_cursor: Option<u32>,
}

/// Page of the wire tables for a staged scene: objects from `cursor` while
/// the page's bytes (ids + labels + pose rows) stay within `allowance`, at
/// least one object so a page always moves forward. `only` keeps just those
/// object indexes. A cursor past the end answers an empty last page.
/// Units: bytes, object indexes.
pub(crate) fn table_page(
    ids: &[ObjectId],
    actors: &[engine_spatial::Actor],
    cursor: u32,
    allowance: usize,
    only: Option<&BTreeSet<usize>>,
) -> TablePage {
    let mut page = TablePage {
        ids: Vec::new(),
        objects: Vec::new(),
        poses: Vec::new(),
        next_cursor: None,
    };
    let mut used = 0usize;
    for (index, (id, actor)) in ids.iter().zip(actors).enumerate().skip(index_of(cursor)) {
        if only.is_some_and(|keep| !keep.contains(&index)) {
            continue;
        }
        let uuid = id.to_string();
        let label = object_label(&actor.name, id);
        let pose = engine_spatial::brief::place_row(actor);
        let cost = uuid.len() + label.len() + pose.len();
        if !page.ids.is_empty() && used + cost > allowance {
            page.next_cursor = u32::try_from(index).ok();
            break;
        }
        used += cost;
        page.ids.push(uuid);
        page.objects.push(label);
        page.poses.push(pose);
    }
    page
}

/// Table byte allowance for a brief of `budget_bytes`: a quarter of it, so
/// text plus tables stay near one and a quarter budgets.
pub(crate) fn table_allowance(budget_bytes: u32) -> usize {
    index_of(budget_bytes / 4)
}
