//! Semantic diff: describe a command batch as relation changes.
//!
//! Consumer: the selective-undo display and the editor assistant's proposal
//! summaries. The caller owns the data: inputs are relation [`Graph`]
//! snapshots taken before and after a batch, output is a plain
//! [`SemanticDiff`]. Determinism scope: the same two
//! graphs in any edge order give the same diff, the same describe lines and
//! the same summary string. A move-only batch keeps the same node set, so the
//! diff shows relation deltas, not object churn.
//!
//! Std-only. Never imports scene or editor types.

use super::relation::{build, Graph, Layer, Relation};
use super::Actor;
use std::collections::{BTreeMap, BTreeSet};

/// One directed relation triple: `(subject, relation, object)`, in the same
/// frozen `(a, relation, b)` order as [`Graph::edges`].
pub type Edge = (u32, Relation, u32);

/// Relation changes between two graphs. `added` holds edges only in `after`,
/// `removed` holds edges only in `before`, both in frozen `(a, relation, b)`
/// order. `relabeled` holds `(id, old_layer, new_layer)` for ids present in
/// both graphs whose containment layer changed, sorted by id. Ids that appear
/// on only one side are object add/removes: their edges show up in
/// added/removed, never in relabeled.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SemanticDiff {
    pub added: Vec<Edge>,
    pub removed: Vec<Edge>,
    pub relabeled: Vec<(u32, Layer, Layer)>,
}

impl SemanticDiff {
    /// True when nothing changed: no edge delta and no relabel.
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.removed.is_empty() && self.relabeled.is_empty()
    }

    /// Total change count: added plus removed edges plus relabels.
    pub fn len(&self) -> usize {
        self.added.len() + self.removed.len() + self.relabeled.len()
    }
}

fn layer_name(layer: Layer) -> &'static str {
    match layer {
        Layer::Arena => "arena",
        Layer::Zone => "zone",
        Layer::Object => "object",
    }
}

/// Edge-set delta plus layer relabels between two graphs. Order-invariant:
/// both edge lists are treated as sets, so shuffled inputs diff identically.
/// Outputs are frozen: edges sort by `(a, relation, b)`, relabels by id.
pub fn diff_graph(before: &Graph, after: &Graph) -> SemanticDiff {
    let before_set: BTreeSet<Edge> = before.edges.iter().copied().collect();
    let after_set: BTreeSet<Edge> = after.edges.iter().copied().collect();
    let added: Vec<Edge> = after_set.difference(&before_set).copied().collect();
    let removed: Vec<Edge> = before_set.difference(&after_set).copied().collect();

    // Last row wins on duplicate ids, matching the builder's id-sorted nodes.
    let before_layers: BTreeMap<u32, Layer> = before
        .nodes
        .iter()
        .map(|(id, layer)| (*id, *layer))
        .collect();
    let after_layers: BTreeMap<u32, Layer> = after
        .nodes
        .iter()
        .map(|(id, layer)| (*id, *layer))
        .collect();
    let mut relabeled = Vec::new();
    for (id, old_layer) in &before_layers {
        if let Some(new_layer) = after_layers.get(id) {
            if old_layer != new_layer {
                relabeled.push((*id, *old_layer, *new_layer));
            }
        }
    }
    // BTreeMap iteration is id-sorted, so relabeled is frozen already.

    SemanticDiff {
        added,
        removed,
        relabeled,
    }
}

/// Dry-run placement diff: build the after-graph from `after_actors` in
/// memory and diff it against the precomputed `before` graph.
///
/// Consumer: selective undo display, proposal summaries, hint previews.
/// The caller owns the data: `before` is the graph over the live
/// scene, `after_actors` is the same snapshot with the candidate move
/// applied in memory (a hint placement, a staged batch). Output equals
/// `diff_graph(before, &build(after_actors))` exactly, so a dry-run preview
/// and the applied result through the command service always agree.
/// Determinism scope: the same two actor sets in any order give the same
/// diff twice. Pure: touches no scene, no tick, no IO.
pub fn dry_run_diff(before: &Graph, after_actors: &[Actor]) -> SemanticDiff {
    diff_graph(before, &build(after_actors))
}

/// One stable line per change for undo/preview display: `-` for a lost
/// relation, `+` for a gained one, `=` for a layer relabel. Removed rows come
/// first (frozen edge order), then added rows (frozen edge order), then
/// relabels (id order). Empty diff gives no lines.
pub fn describe(diff: &SemanticDiff) -> Vec<String> {
    let mut out = Vec::with_capacity(diff.len());
    for (a, rel, b) in &diff.removed {
        out.push(format!("- relation {a} {} {b}", rel.name()));
    }
    for (a, rel, b) in &diff.added {
        out.push(format!("+ relation {a} {} {b}", rel.name()));
    }
    for (id, old, new) in &diff.relabeled {
        out.push(format!(
            "= {}: {} -> {}",
            id,
            layer_name(*old),
            layer_name(*new)
        ));
    }
    out
}

/// Single bounded line with change counts for digests and logs. Bounded by
/// construction: a fixed prefix plus three decimal counters, never a newline.
pub fn summarize(diff: &SemanticDiff) -> String {
    format!(
        "diff +{} -{} ~{}",
        diff.added.len(),
        diff.removed.len(),
        diff.relabeled.len()
    )
}

#[cfg(test)]
mod tests {
    use super::super::relation::{build, Layer, Relation};
    use super::*;

    #[expect(
        clippy::too_many_arguments,
        reason = "test fixture builder takes each field positionally"
    )]
    fn mk(
        id: u32,
        name: &str,
        x: f32,
        y: f32,
        z: f32,
        sx: f32,
        sy: f32,
        sz: f32,
        yaw: f32,
    ) -> Actor {
        Actor {
            id,
            name: name.into(),
            x,
            y,
            z,
            sx,
            sy,
            sz,
            parent: None,
            physical: true,
            vx: 0.0,
            vz: 0.0,
            yaw,
        }
    }

    fn prop(id: u32, name: &str, x: f32, y: f32, z: f32) -> Actor {
        mk(id, name, x, y, z, 1.0, 1.0, 1.0, 0.0)
    }

    use super::super::Actor;

    fn table() -> Actor {
        mk(10, "table", 5.0, 0.5, 5.0, 2.0, 1.0, 2.0, 0.0)
    }

    #[test]
    fn identical_graphs_give_empty_diff() {
        let scene = vec![table(), prop(11, "crate", 5.0, 1.5, 5.0)];
        let g = build(&scene);
        let diff = diff_graph(&g, &g);
        assert!(diff.is_empty());
        assert_eq!(diff.len(), 0);
        assert!(diff.added.is_empty());
        assert!(diff.removed.is_empty());
        assert!(diff.relabeled.is_empty());
        assert!(describe(&diff).is_empty());
        assert_eq!(summarize(&diff), "diff +0 -0 ~0");
        // A fresh build of the same scene diffs empty too.
        let again = build(&scene);
        assert!(diff_graph(&g, &again).is_empty());
    }

    #[test]
    fn single_move_batch_yields_exactly_its_relation_delta() {
        let t = table();
        let stacked = prop(11, "crate", 5.0, 1.5, 5.0);
        let aside = prop(11, "crate", 8.0, 0.5, 5.0);
        let before = build(&[t.clone(), stacked]);
        let after = build(&[t, aside]);
        let diff = diff_graph(&before, &after);

        // Move-only: same node set, so no relabel churn from add/remove.
        assert_eq!(before.nodes, after.nodes);
        assert!(diff.relabeled.is_empty());

        // Exact set delta: added holds after-minus-before, removed the reverse.
        let before_set: BTreeSet<Edge> = before.edges.iter().copied().collect();
        let after_set: BTreeSet<Edge> = after.edges.iter().copied().collect();
        let want_added: Vec<Edge> = after_set.difference(&before_set).copied().collect();
        let want_removed: Vec<Edge> = before_set.difference(&after_set).copied().collect();
        assert_eq!(diff.added, want_added);
        assert_eq!(diff.removed, want_removed);

        // The stack contact is the delta: table held the crate, now it does not.
        assert!(diff.removed.contains(&(10, Relation::Supports, 11)));
        assert!(diff.removed.contains(&(11, Relation::OnTopOf, 10)));
        assert!(
            !diff.added.contains(&(10, Relation::Supports, 11)),
            "aside crate rests on nothing"
        );
        assert!(!diff.is_empty());
        // Frozen order on both lists.
        let mut sorted = diff.added.clone();
        sorted.sort_by_key(|(a, r, b)| (*a, *r, *b));
        assert_eq!(sorted, diff.added);
        let mut sorted = diff.removed.clone();
        sorted.sort_by_key(|(a, r, b)| (*a, *r, *b));
        assert_eq!(sorted, diff.removed);
        // One line per change, and the summary counts match.
        let lines = describe(&diff);
        assert_eq!(lines.len(), diff.len());
        assert_eq!(
            summarize(&diff),
            format!("diff +{} -{} ~0", diff.added.len(), diff.removed.len())
        );
    }

    #[test]
    fn add_remove_object_shows_edge_sets() {
        let t = table();
        let stacked = prop(11, "crate", 5.0, 1.5, 5.0);
        let before = build(&[t.clone(), stacked.clone()]);
        let after_add = build(&[t.clone(), stacked, prop(12, "extra", 7.0, 0.5, 5.0)]);
        let grown = diff_graph(&before, &after_add);
        assert!(grown.removed.is_empty(), "adding touches no old edge");
        assert!(!grown.added.is_empty(), "new edges mention the newcomer");
        assert!(
            grown.added.iter().all(|(a, _, b)| *a == 12 || *b == 12),
            "single addition: every added edge touches id 12, got {:?}",
            grown.added
        );
        // Newcomer ids never relabel: they exist on one side only.
        assert!(
            !grown.relabeled.iter().any(|(id, _, _)| *id == 12),
            "added ids are edge churn, not relabels"
        );

        let after_remove = build(&[t]);
        let shrunk = diff_graph(&before, &after_remove);
        assert!(shrunk.added.is_empty(), "removal adds no edge");
        assert!(!shrunk.removed.is_empty(), "old edges drop out");
        assert!(
            shrunk.removed.iter().all(|(a, _, b)| *a == 11 || *b == 11),
            "single removal: every removed edge touches id 11, got {:?}",
            shrunk.removed
        );
        assert!(
            !shrunk.relabeled.iter().any(|(id, _, _)| *id == 11),
            "removed ids are edge churn, not relabels"
        );
    }

    #[test]
    fn relabel_tracks_layer_change_by_id() {
        // Genuine containment promotion: hand-built graphs where id 7 moves
        // from object to zone while edges stay put.
        let before = Graph {
            nodes: vec![(0, Layer::Arena), (7, Layer::Object)],
            edges: vec![],
        };
        let after = Graph {
            nodes: vec![(0, Layer::Arena), (7, Layer::Zone)],
            edges: vec![],
        };
        let diff = diff_graph(&before, &after);
        assert!(diff.added.is_empty());
        assert!(diff.removed.is_empty());
        assert_eq!(diff.relabeled, vec![(7, Layer::Object, Layer::Zone)]);
        assert_eq!(describe(&diff), vec!["= 7: object -> zone".to_string()]);
        assert_eq!(summarize(&diff), "diff +0 -0 ~1");
        assert_eq!(diff.len(), 1);
        assert!(!diff.is_empty());
    }

    #[test]
    fn describe_lines_stable_across_runs() {
        let t = table();
        let before = build(&[t.clone(), prop(11, "crate", 5.0, 1.5, 5.0)]);
        let after = build(&[t, prop(11, "crate", 8.0, 0.5, 5.0)]);
        let d1 = diff_graph(&before, &after);
        let d2 = diff_graph(&before, &after);
        assert_eq!(describe(&d1), describe(&d2));
        assert_eq!(summarize(&d1), summarize(&d2));
        // Shuffled edge storage diffs identically: sets, not lists.
        let mut shuffled = before.clone();
        shuffled.edges.reverse();
        let d3 = diff_graph(&shuffled, &after);
        assert_eq!(d1, d3);
        assert_eq!(describe(&d1), describe(&d3));
        // Every line is a whole single-line row with a stable prefix.
        for line in describe(&d1) {
            assert!(!line.contains('\n'), "one line per change");
            assert!(
                line.starts_with("- relation ")
                    || line.starts_with("+ relation ")
                    || line.starts_with("= "),
                "stable row prefix, got: {line}"
            );
        }
        // Summary is a single bounded line with counts.
        let summary = summarize(&d1);
        assert!(!summary.contains('\n'), "summary is one line");
        assert!(summary.len() <= 64, "summary stays bounded");
        assert!(summary.starts_with("diff +"), "count line for logs");
    }

    #[test]
    fn empty_graphs_diff_empty() {
        let g = build(&[]);
        assert!(diff_graph(&g, &g).is_empty());
        assert_eq!(summarize(&diff_graph(&g, &g)), "diff +0 -0 ~0");
    }

    /// Deterministic drift of the stacked crate across the table's east side:
    /// ten seeded rest spots, all grounded and clear of the table volume.
    fn seeded_spot(seed: usize) -> (f32, f32) {
        let i = seed as f32;
        (6.5 + (i % 5.0) * 0.3, 3.0 + (i / 5.0).floor() * 0.5)
    }

    #[test]
    fn dry_run_diff_equals_applied_ten_seeded() {
        let t = table();
        let before_scene = vec![t.clone(), prop(11, "crate", 5.0, 1.5, 5.0)];
        let before = build(&before_scene);
        // The untouched scene dry-runs empty.
        assert!(dry_run_diff(&before, &before_scene).is_empty());
        for seed in 0..10 {
            let (x, z) = seeded_spot(seed);
            let after_scene = vec![t.clone(), mk(11, "crate", x, 0.5, z, 1.0, 1.0, 1.0, 0.0)];
            let applied = diff_graph(&before, &build(&after_scene));
            let dry = dry_run_diff(&before, &after_scene);
            assert_eq!(dry, applied, "seed {seed}: dry-run must equal applied");
            // Stable twice: the wrapper never drifts.
            assert_eq!(
                dry,
                dry_run_diff(&before, &after_scene),
                "seed {seed}: dry-run repeats"
            );
            // The move-only delta still names the stack contact.
            assert!(
                dry.removed.contains(&(10, Relation::Supports, 11)),
                "seed {seed}"
            );
            assert!(
                dry.removed.contains(&(11, Relation::OnTopOf, 10)),
                "seed {seed}"
            );
            assert_eq!(
                summarize(&dry),
                format!("diff +{} -{} ~0", dry.added.len(), dry.removed.len()),
                "seed {seed}"
            );
        }
        // Order-invariant: reversed actor storage diffs identically.
        let (x, z) = seeded_spot(3);
        let after_scene = vec![t.clone(), mk(11, "crate", x, 0.5, z, 1.0, 1.0, 1.0, 0.0)];
        let mut rev = after_scene.clone();
        rev.reverse();
        assert_eq!(
            dry_run_diff(&before, &rev),
            dry_run_diff(&before, &after_scene),
            "reversed storage diffs identically"
        );
    }
}
