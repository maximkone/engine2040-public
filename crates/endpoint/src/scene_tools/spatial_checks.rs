//! `scene.spatial` read path: the wire tables and finding lines the verb
//! reports, plus which frozen catalog checks one call ran, which it could not
//! drive (each with one short reason) and where each staged box got its size.
//!
//! Consumer: `scene_tools::ServerState::scene_brief` (the index-to-UUID and
//! label tables) plus `scene_spatial` (tables, finding lines and the check
//! partition), so a caller can tell a clean scene from a check that never ran.
//! Owner: the 18-name catalog and every check body stay in `engine-spatial`;
//! this module only decides which of those names the door's posting can give
//! an input to: boxes named after their mesh asset (`lamp_2`), sized by the one
//! scene adapter ([`engine_editor::spatial_view`]), with no velocity and no
//! baked nav view.

use std::collections::{BTreeMap, BTreeSet};

use engine_editor::commands::Document;
use engine_scene::component::{Component, ComponentKind};
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

/// Where each staged node's box size came from: a scene collider component
/// (times world scale) or the labelled fallback ([`engine_editor::spatial_view`]
/// sizes a bare node by its world scale, 1 m at unit scale).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct SizeSource {
    /// Nodes sized from a collider component.
    pub collider: u64,
    /// Nodes posted as the fallback box (no collider; labelled `default size`).
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

/// Labels for the staged nodes, in the order given (id-text sort). Names are
/// `<mesh asset>_<n>` with `n` counting 1.. per base name in that order, so a
/// node's label moves only when a node of the SAME base name sorts before it;
/// an unrelated insert never relabels anything.
pub(crate) fn staged_names(doc: &Document, ids: &[ObjectId]) -> Vec<String> {
    let mut seen: BTreeMap<String, u32> = BTreeMap::new();
    let mut names = Vec::with_capacity(ids.len());
    for id in ids {
        let base = name_base(doc, *id);
        let n = seen.entry(base.clone()).or_insert(0);
        *n += 1;
        names.push(format!("{base}_{n}"));
    }
    names
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

/// Honesty rows for one page: every id with the labelled default box, worded
/// by the one scene adapter ([`engine_editor::spatial_view::default_row`]).
/// A caller reading an "does X overlap Y" answer learns which boxes were
/// assumed, not measured. Index-aligned with `ids`/`labels`.
pub(crate) fn default_sized(
    ids: &[String],
    labels: &[String],
    defaults: &BTreeMap<String, [f32; 3]>,
) -> Vec<String> {
    ids.iter()
        .zip(labels)
        .filter_map(|(id, label)| {
            defaults
                .get(id)
                .map(|size| engine_editor::spatial_view::default_row(label, *size))
        })
        .collect()
}

// Document snapshot: actors, the id table and the count of unresolved poses.
pub(crate) struct Snapshot {
    pub(crate) actors: Vec<engine_spatial::Actor>,
    pub(crate) ids: Vec<ObjectId>,
    pub(crate) missing_pose: u64,
    pub(crate) sizes: SizeSource,
    /// Default-sized nodes: id text -> the assumed box in meters. A report can
    /// name them instead of letting a reader believe the box was measured.
    pub(crate) defaults: BTreeMap<String, [f32; 3]>,
}

impl Snapshot {
    /// True when the actor at `index` carries the labelled default box, not a
    /// box measured from a collider component.
    pub(crate) fn is_default(&self, index: usize) -> bool {
        self.ids
            .get(index)
            .is_some_and(|id| self.defaults.contains_key(&id.to_string()))
    }
}

/// SP66: the document's lights as the reach query reads them, joined with each
/// carrying node's pose by the snapshot's id order. A light whose node has no
/// finite pose is left out: it has no place to reach from, and the reach report
/// names that node `unplaced`.
///
/// Units: the light's place in whole cm (the same rounding as every actor), its
/// reach in whole cm; a point or spot light with an unreadable or non-positive
/// range keeps `None`, so its points read `unknown`, never `dark`.
pub(crate) fn reach_lights(
    doc: &Document,
    ids: &[ObjectId],
    actors: &[engine_spatial::Actor],
) -> Vec<engine_spatial::ReachLight> {
    use engine_scene::component::LightKind;

    let mut lights = Vec::new();
    for (index, id) in ids.iter().enumerate() {
        let Some(actor) = actors.get(index) else {
            break;
        };
        let Some(table) = doc.components.get(&id.to_string()) else {
            continue;
        };
        let Some(Component::Light(light)) = table.get(&ComponentKind::Light) else {
            continue;
        };
        let (Some(x), Some(y), Some(z)) = (
            engine_spatial::cm_of(actor.x),
            engine_spatial::cm_of(actor.y),
            engine_spatial::cm_of(actor.z),
        ) else {
            continue;
        };
        let family = match light.kind {
            LightKind::Directional => engine_spatial::LightFamily::Directional,
            LightKind::Point => engine_spatial::LightFamily::Point,
            LightKind::Spot => engine_spatial::LightFamily::Spot,
        };
        lights.push(engine_spatial::ReachLight::new(
            family,
            [x, y, z],
            light.range,
        ));
    }
    lights
}

// Sorted-id snapshot of every reachable node, sized by the one scene adapter
// ([`engine_editor::spatial_view`]): a collider component gives the real
// extents, a bare node its world scale (the labelled default). Nodes whose
// world pose is missing or non-finite post a NaN actor, never a drop, so
// `unknown-position` fires with the id still in the table.
pub(crate) fn snapshot(doc: &Document) -> Snapshot {
    use engine_editor::spatial_view;

    let scene = spatial_view::rows_of(doc);
    // One slot per reachable node: finite rows plus the missing-pose ids,
    // merged in id-text order so the table and the actor ids stay aligned.
    let mut sized: BTreeMap<String, spatial_view::NodeRow> = scene
        .rows
        .iter()
        .map(|row| (row.id.to_string(), *row))
        .collect();
    let mut order: Vec<ObjectId> = scene
        .rows
        .iter()
        .map(|row| row.id)
        .chain(scene.missing.iter().copied())
        .collect();
    order.sort_by_key(|id| id.to_string());
    let names = staged_names(doc, &order);
    let mut actors = Vec::with_capacity(order.len());
    let mut ids = Vec::with_capacity(order.len());
    let mut defaults = BTreeMap::new();
    for (position, id) in order.iter().enumerate() {
        let Ok(index) = u32::try_from(position) else {
            break;
        };
        let name = names.get(position).cloned().unwrap_or_default();
        ids.push(*id);
        match sized.remove(&id.to_string()) {
            Some(row) => {
                if row.basis.is_default() {
                    defaults.insert(id.to_string(), row.size);
                }
                actors.push(spatial_view::actor(&row, index, name));
            }
            None => {
                // A missing or non-finite pose posts a NaN actor, never a drop.
                defaults.insert(id.to_string(), [1.0; 3]);
                actors.push(engine_spatial::Actor {
                    id: index,
                    name,
                    x: f32::NAN,
                    y: f32::NAN,
                    z: f32::NAN,
                    sx: 1.0,
                    sy: 1.0,
                    sz: 1.0,
                    parent: None,
                    physical: true,
                    vx: 0.0,
                    vz: 0.0,
                    yaw: 0.0,
                });
            }
        }
    }
    let counts = spatial_view::SizeCounts::of(&scene.rows);
    let missing = wide(scene.missing.len());
    Snapshot {
        actors,
        ids,
        missing_pose: missing,
        sizes: SizeSource {
            collider: counts.collider,
            default: counts.default + missing,
        },
        defaults,
    }
}

/// Table byte allowance for a brief of `budget_bytes`: a quarter of it, so
/// text plus tables stay near one and a quarter budgets.
pub(crate) fn table_allowance(budget_bytes: u32) -> usize {
    index_of(budget_bytes / 4)
}
