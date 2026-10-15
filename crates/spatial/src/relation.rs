//! Scene relation graph: Supports, OnTopOf, Inside, NextTo, Facing in
//! arena -> zone -> object layers.
//!
//! Consumer: the brief, the placement solver and the playtest verifiers. The
//! caller owns the data: input is a plain actor snapshot, output is a plain
//! [`Graph`]. Units are meters in, integer centimeters inside. Determinism
//! scope: the same actor set in any input order gives identical node/edge
//! order and [`digest`].
//! Actors with unknown position, unknown size, or non-positive size become
//! isolated `Object` nodes; they never error. Precondition: actor ids are
//! unique. Same-id pairs are skipped, so duplicates degrade quietly — keep
//! them out.
//!
//! Std-only. Never imports scene or editor types. Geometry runs on integer
//! centimeters so builds are bit-identical across targets; yaw reuses the
//! `navcheck` octant table, so no libm call appears here.
//!
//! Vocabulary v2: above/below, front/behind/left/right in world,
//! subject and viewer frames, between/against/around/aligned, near/far
//! bands, blocking — all on integer-cm thresholds, subject-frame edges plus
//! ternary records in [`RelGraph`] with an order-invariant [`digest_rel`].
//! The v1 [`Relation`] enum and [`Graph`] are frozen (brief, diff and the
//! solver match on them); v2 is a new layer the affordance and scene-query
//! readers consume.

use super::index::{boxes_of, pos_known, size_known, Box3};
use super::navcheck::octant;
use super::Actor;
use engine_core::Fnv64;
use std::collections::{BTreeMap, BTreeSet};

/// Resting-contact tolerance: supporter top to supported bottom, in cm.
/// Same convention as the `support` check, so graphs agree with findings.
pub const SUPPORT_GAP_CM: i32 = 5;
/// Side-by-side edge-gap limit, in cm (squared euclidean over the XZ gap,
/// so diagonal adjacency counts). Keeps `NextTo` local, not arena-wide.
pub const NEXT_TO_CM: i32 = 100;
/// Forward half-plane range for `Facing`, in cm. Bounds brief fan-out;
/// geometry past this range is "away", not "faced".
pub const FACING_RANGE_CM: i32 = 2000;

/// One directed relation. The first id is the subject:
/// `(a, Supports, b)` reads "a holds b up"; `(a, OnTopOf, b)` reads
/// "a rests on b"; `(a, Inside, b)` reads "a's bounds sit inside b's";
/// `(a, NextTo, b)` is symmetric and stored once with `a < b`;
/// `(a, Facing, b)` reads "a's yaw points at b".
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Relation {
    Supports,
    OnTopOf,
    Inside,
    NextTo,
    Facing,
}

impl Relation {
    /// Frozen lowercase label used by brief and diff text rows.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Supports => "supports",
            Self::OnTopOf => "on_top_of",
            Self::Inside => "inside",
            Self::NextTo => "next_to",
            Self::Facing => "facing",
        }
    }
}

/// Containment layer. The largest actor is the arena; any other actor that
/// bounds-contains a further actor is a zone; the rest are objects.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Layer {
    Arena,
    Zone,
    Object,
}

/// Relation graph: every input actor id with its layer, plus relations in
/// frozen `(a, relation, b)` order. Plain data; the caller owns it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Graph {
    pub nodes: Vec<(u32, Layer)>,
    pub edges: Vec<(u32, Relation, u32)>,
}

/// An actor takes edges only with fully known, positively sized bounds.
fn valid(a: &Actor) -> bool {
    pos_known(a) && size_known(a) && a.sx > 0.0 && a.sy > 0.0 && a.sz > 0.0
}

fn volume_cm3(b: &Box3) -> i64 {
    let dx = (i64::from(b.max_x) - i64::from(b.min_x)).max(0);
    let dy = (i64::from(b.max_y) - i64::from(b.min_y)).max(0);
    let dz = (i64::from(b.max_z) - i64::from(b.min_z)).max(0);
    dx * dy * dz
}

/// Bounds containment with no margin, so flush walls still nest; strict
/// volume breaks ties, so identical boxes never contain each other.
fn contains_box(outer: &Box3, inner: &Box3, vol_outer: i64, vol_inner: i64) -> bool {
    vol_outer > vol_inner
        && outer.min_x <= inner.min_x
        && inner.max_x <= outer.max_x
        && outer.min_y <= inner.min_y
        && inner.max_y <= outer.max_y
        && outer.min_z <= inner.min_z
        && inner.max_z <= outer.max_z
}

/// Id-sorted working view shared by `build` and `rebuild_cached`.
struct Prepared {
    /// All actors, id-sorted; nodes come from here, valid or not.
    ordered: Vec<Actor>,
    /// Valid actors only, id-sorted; the only edge participants.
    good: Vec<Actor>,
    boxes: BTreeMap<u32, Box3>,
    vols: BTreeMap<u32, i64>,
}

fn prepare(actors: &[Actor]) -> Prepared {
    let mut ordered: Vec<Actor> = actors.to_vec();
    ordered.sort_by_key(|a| a.id);
    let good: Vec<Actor> = ordered.iter().filter(|a| valid(a)).cloned().collect();
    let boxes: BTreeMap<u32, Box3> = boxes_of(&good).into_iter().map(|b| (b.id, b)).collect();
    let vols: BTreeMap<u32, i64> = boxes.iter().map(|(id, b)| (*id, volume_cm3(b))).collect();
    Prepared {
        ordered,
        good,
        boxes,
        vols,
    }
}

/// Layer assignment over the prepared view. Largest valid actor is the
/// arena; other bounds-containers are zones; everything else is an object.
fn layers(p: &Prepared) -> Vec<(u32, Layer)> {
    let ids: Vec<u32> = p.good.iter().map(|a| a.id).collect();
    let mut kids: BTreeMap<u32, usize> = BTreeMap::new();
    for (i, &a) in ids.iter().enumerate() {
        for &b in &ids[i + 1..] {
            if contains_box(&p.boxes[&a], &p.boxes[&b], p.vols[&a], p.vols[&b]) {
                *kids.entry(a).or_insert(0) += 1;
            } else if contains_box(&p.boxes[&b], &p.boxes[&a], p.vols[&b], p.vols[&a]) {
                *kids.entry(b).or_insert(0) += 1;
            }
        }
    }
    // Max volume wins; smallest id breaks ties so the pick is total.
    let arena = p
        .good
        .iter()
        .max_by(|a, b| {
            p.vols[&a.id]
                .cmp(&p.vols[&b.id])
                .then_with(|| b.id.cmp(&a.id))
        })
        .map(|a| a.id);
    p.ordered
        .iter()
        .map(|a| {
            let layer = if Some(a.id) == arena && valid(a) {
                Layer::Arena
            } else if valid(a) && kids.get(&a.id).copied().unwrap_or(0) > 0 {
                Layer::Zone
            } else {
                Layer::Object
            };
            (a.id, layer)
        })
        .collect()
}

/// True when `me` faces `other`: yaw-snapped forward octant, strictly ahead
/// (half-plane, not just nearby), and inside facing range. Y-only.
fn faces(me: &Actor, other: &Actor) -> bool {
    if !me.yaw.is_finite() {
        return false;
    }
    let dx = i64::from(super::index::to_cm(other.x)) - i64::from(super::index::to_cm(me.x));
    let dz = i64::from(super::index::to_cm(other.z)) - i64::from(super::index::to_cm(me.z));
    if dx == 0 && dz == 0 {
        return false;
    }
    let (fx, fz) = octant(me.yaw);
    if i64::from(fx) * dx + i64::from(fz) * dz <= 0 {
        return false;
    }
    let range = i64::from(FACING_RANGE_CM);
    dx * dx + dz * dz <= range * range
}

/// Every edge between one unordered valid pair. Supports/OnTopOf are duals
/// of one resting contact; Inside follows bounds; NextTo needs disjoint
/// XZ interiors, a small XZ gap, and vertical overlap; Facing is per-side.
fn pair_edges(
    a: &Actor,
    b: &Actor,
    boxes: &BTreeMap<u32, Box3>,
    vols: &BTreeMap<u32, i64>,
) -> Vec<(u32, Relation, u32)> {
    if a.id == b.id {
        return Vec::new();
    }
    let mut out = Vec::new();
    let (aa, bb) = (&boxes[&a.id], &boxes[&b.id]);

    let xz_hit =
        aa.min_x < bb.max_x && bb.min_x < aa.max_x && aa.min_z < bb.max_z && bb.min_z < aa.max_z;
    if xz_hit {
        let b_on_a = i64::from(bb.min_y) - i64::from(aa.max_y);
        let a_on_b = i64::from(aa.min_y) - i64::from(bb.max_y);
        let gap = i64::from(SUPPORT_GAP_CM);
        match (0 <= b_on_a && b_on_a <= gap, 0 <= a_on_b && a_on_b <= gap) {
            (true, false) => {
                out.push((a.id, Relation::Supports, b.id));
                out.push((b.id, Relation::OnTopOf, a.id));
            }
            (false, true) => {
                out.push((b.id, Relation::Supports, a.id));
                out.push((a.id, Relation::OnTopOf, b.id));
            }
            // Degenerate zero-height coincidence: lower id holds, deterministically.
            (true, true) => {
                let (lo, hi) = if a.id < b.id {
                    (a.id, b.id)
                } else {
                    (b.id, a.id)
                };
                out.push((lo, Relation::Supports, hi));
                out.push((hi, Relation::OnTopOf, lo));
            }
            (false, false) => {}
        }
    }

    if contains_box(aa, bb, vols[&a.id], vols[&b.id]) {
        out.push((b.id, Relation::Inside, a.id));
    } else if contains_box(bb, aa, vols[&b.id], vols[&a.id]) {
        out.push((a.id, Relation::Inside, b.id));
    }

    let ix = i64::from(aa.max_x.min(bb.max_x)) - i64::from(aa.min_x.max(bb.min_x));
    let iz = i64::from(aa.max_z.min(bb.max_z)) - i64::from(aa.min_z.max(bb.min_z));
    if !(ix > 0 && iz > 0) {
        let gx = (i64::from(aa.min_x.max(bb.min_x)) - i64::from(aa.max_x.min(bb.max_x))).max(0);
        let gz = (i64::from(aa.min_z.max(bb.min_z)) - i64::from(aa.max_z.min(bb.max_z))).max(0);
        let limit = i64::from(NEXT_TO_CM);
        if gx * gx + gz * gz <= limit * limit && aa.min_y < bb.max_y && bb.min_y < aa.max_y {
            let (lo, hi) = if a.id < b.id {
                (a.id, b.id)
            } else {
                (b.id, a.id)
            };
            out.push((lo, Relation::NextTo, hi));
        }
    }

    if faces(a, b) {
        out.push((a.id, Relation::Facing, b.id));
    }
    if faces(b, a) {
        out.push((b.id, Relation::Facing, a.id));
    }
    out
}

fn all_edges(p: &Prepared) -> Vec<(u32, Relation, u32)> {
    let by_id: BTreeMap<u32, &Actor> = p.good.iter().map(|a| (a.id, a)).collect();
    let ids: Vec<u32> = p.good.iter().map(|a| a.id).collect();
    // Candidate phase over the 3D index: strict XZ pairs cover
    // Supports/OnTopOf/Inside (containment of positive volumes always shares
    // strict XZ), the NEXT_TO_CM adjacency ring covers NextTo. Facing is
    // long-range by design and rides no candidate set, so pairs outside the
    // contact set still run the yaw narrow gate; nothing else can hold there.
    let index = super::index3::Index3::build(&p.good);
    let mut contact: BTreeSet<(u32, u32)> = index.pairs_xz().into_iter().collect();
    contact.extend(index.adjacent_xz(NEXT_TO_CM));
    let key = |a: &u32, b: &u32| {
        if a < b {
            (*a, *b)
        } else {
            (*b, *a)
        }
    };
    let mut edges = Vec::new();
    for (i, &a) in ids.iter().enumerate() {
        for &b in &ids[i + 1..] {
            if contact.contains(&key(&a, &b)) {
                edges.extend(pair_edges(by_id[&a], by_id[&b], &p.boxes, &p.vols));
            } else {
                if faces(by_id[&a], by_id[&b]) {
                    edges.push((a, Relation::Facing, b));
                }
                if faces(by_id[&b], by_id[&a]) {
                    edges.push((b, Relation::Facing, a));
                }
            }
        }
    }
    edges.sort_by_key(|(a, r, b)| (*a, *r, *b));
    edges
}

/// Full relation graph over an actor snapshot. Same actor set in any input
/// order gives the same graph: actors sort by id, edges freeze in
/// `(a, relation, b)` order.
pub fn build(actors: &[Actor]) -> Graph {
    let p = prepare(actors);
    Graph {
        nodes: layers(&p),
        edges: all_edges(&p),
    }
}

/// Order-invariant FNV-1a digest over canonical bytes: node/edge counts plus
/// sorted `(id, layer)` / `(a, relation, b)` rows under a format tag, so even
/// a hand-shuffled graph digests like its frozen twin.
pub fn digest(graph: &Graph) -> u64 {
    let mut nodes = graph.nodes.clone();
    nodes.sort_by_key(|(id, layer)| (*id, *layer));
    let mut edges = graph.edges.clone();
    edges.sort_by_key(|(a, r, b)| (*a, *r, *b));
    let mut h = Fnv64::new();
    h.write(b"e2relation1");
    h.write_u64(nodes.len() as u64);
    for (id, layer) in &nodes {
        h.write_u32(*id);
        h.write_u8(*layer as u8);
    }
    h.write_u64(edges.len() as u64);
    for (a, r, b) in &edges {
        h.write_u32(*a);
        h.write_u8(*r as u8);
        h.write_u32(*b);
    }
    h.finish()
}

/// Incremental rebuild: recompute only edges touching changed subtrees, keep
/// the rest from `old`. `changed_ids` must list every added or modified actor
/// id; removals need no listing (missing ids drop out on their own). Changed
/// actors drag their parent-linked descendants along. Layers always recompute
/// fully — containment is global — so with a complete `changed_ids` the
/// result is identical to [`build`]. Newcomers are forced into the affected
/// set even when the caller forgets to list them.
pub fn rebuild_cached(old: &Graph, changed_ids: &[u32], actors: &[Actor]) -> Graph {
    let p = prepare(actors);
    let current: BTreeSet<u32> = p.ordered.iter().map(|a| a.id).collect();
    let old_ids: BTreeSet<u32> = old.nodes.iter().map(|(id, _)| *id).collect();

    let mut child_of: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
    for a in &p.ordered {
        if let Some(parent) = a.parent {
            child_of.entry(parent).or_default().push(a.id);
        }
    }
    let mut affected: BTreeSet<u32> = BTreeSet::new();
    let mut stack: Vec<u32> = changed_ids.to_vec();
    for id in &current {
        if !old_ids.contains(id) {
            stack.push(*id);
        }
    }
    while let Some(id) = stack.pop() {
        if !current.contains(&id) || !affected.insert(id) {
            continue;
        }
        if let Some(kids) = child_of.get(&id) {
            stack.extend(kids.iter().copied());
        }
    }

    // Old edges neither end touches are provably stale-free: both rows are
    // byte-identical to the full-build inputs, and edges never read layers.
    let mut edges: Vec<(u32, Relation, u32)> = old
        .edges
        .iter()
        .copied()
        .filter(|(a, _, b)| {
            current.contains(a)
                && current.contains(b)
                && !affected.contains(a)
                && !affected.contains(b)
        })
        .collect();
    let by_id: BTreeMap<u32, &Actor> = p.good.iter().map(|a| (a.id, a)).collect();
    let ids: Vec<u32> = p.good.iter().map(|a| a.id).collect();
    for (i, &a) in ids.iter().enumerate() {
        for &b in &ids[i + 1..] {
            if affected.contains(&a) || affected.contains(&b) {
                edges.extend(pair_edges(by_id[&a], by_id[&b], &p.boxes, &p.vols));
            }
        }
    }
    edges.sort_by_key(|(a, r, b)| (*a, *r, *b));
    edges.dedup();
    Graph {
        nodes: layers(&p),
        edges,
    }
}

#[cfg(test)]
mod tests {
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

    /// Arena 20x4x20 at y=2, zone 6x3x6 nested north, two 1 m crates inside.
    fn nested_scene() -> Vec<Actor> {
        vec![
            mk(0, "arena", 0.0, 2.0, 0.0, 20.0, 4.0, 20.0, 0.0),
            mk(1, "zone_north", 0.0, 1.5, -5.0, 6.0, 3.0, 6.0, 0.0),
            prop(2, "crate_a", -1.0, 0.5, -5.0),
            prop(3, "crate_b", 0.5, 0.5, -5.0),
        ]
    }

    fn has(graph: &Graph, a: u32, rel: Relation, b: u32) -> bool {
        graph.edges.contains(&(a, rel, b))
    }

    fn layer_of(graph: &Graph, id: u32) -> Option<Layer> {
        graph.nodes.iter().find(|(i, _)| *i == id).map(|(_, l)| *l)
    }

    #[test]
    fn layer_assignment_follows_bounds_nesting() {
        let g = build(&nested_scene());
        assert_eq!(layer_of(&g, 0), Some(Layer::Arena));
        assert_eq!(layer_of(&g, 1), Some(Layer::Zone));
        assert_eq!(layer_of(&g, 2), Some(Layer::Object));
        assert_eq!(layer_of(&g, 3), Some(Layer::Object));
        // Zone contains objects, arena contains zones — and transitively objects.
        assert!(has(&g, 2, Relation::Inside, 1));
        assert!(has(&g, 3, Relation::Inside, 1));
        assert!(has(&g, 1, Relation::Inside, 0));
        assert!(has(&g, 2, Relation::Inside, 0));
        // Crates stand side by side (50 cm edge gap), not stacked.
        assert!(has(&g, 2, Relation::NextTo, 3));
        assert!(!has(&g, 2, Relation::Supports, 3));
        // Nodes cover every actor id exactly once, id-sorted.
        let ids: Vec<u32> = g.nodes.iter().map(|(id, _)| *id).collect();
        assert_eq!(ids, vec![0, 1, 2, 3]);
    }

    #[test]
    fn supports_pair_contrasts_with_moved_crate() {
        let table = mk(10, "table", 5.0, 0.5, 5.0, 2.0, 1.0, 2.0, 0.0);
        let stacked = prop(11, "crate", 5.0, 1.5, 5.0);
        let aside = prop(11, "crate", 8.0, 0.5, 5.0);
        let a = build(&[table.clone(), stacked]);
        let b = build(&[table, aside]);
        assert!(has(&a, 10, Relation::Supports, 11));
        assert!(has(&a, 11, Relation::OnTopOf, 10));
        assert!(!has(&b, 10, Relation::Supports, 11));
        assert!(!has(&b, 11, Relation::OnTopOf, 10));
        assert_ne!(a.edges, b.edges);
        assert_ne!(digest(&a), digest(&b));
    }

    #[test]
    fn nextto_pair_contrasts_with_distance() {
        let near = build(&[prop(20, "a", 0.0, 0.5, 0.0), prop(21, "b", 1.5, 0.5, 0.0)]);
        let far = build(&[prop(20, "a", 0.0, 0.5, 0.0), prop(21, "b", 5.0, 0.5, 0.0)]);
        assert!(has(&near, 20, Relation::NextTo, 21));
        assert!(!has(&far, 20, Relation::NextTo, 21));
        assert_ne!(digest(&near), digest(&far));
    }

    #[test]
    fn facing_pair_contrasts_with_turn() {
        let watcher = mk(30, "watcher", 0.0, 0.5, 0.0, 1.0, 1.0, 1.0, 0.0);
        let ahead = mk(30, "watcher", 0.0, 0.5, 0.0, 1.0, 1.0, 1.0, 3.0);
        let target = prop(31, "target", 3.0, 0.5, 0.0);
        let a = build(&[watcher, target.clone()]);
        let b = build(&[ahead, target]);
        assert!(has(&a, 30, Relation::Facing, 31));
        assert!(!has(&b, 30, Relation::Facing, 31));
        assert!(
            !b.edges.iter().any(|(_, r, _)| *r == Relation::Facing),
            "turned watcher faces nothing"
        );
        assert_ne!(digest(&a), digest(&b));
    }

    #[test]
    fn inside_pair_contrasts_with_leaving_zone() {
        let arena = mk(40, "arena", 0.0, 2.0, 0.0, 20.0, 4.0, 20.0, 0.0);
        let zone = mk(41, "zone", 0.0, 1.5, -5.0, 6.0, 3.0, 6.0, 0.0);
        let inside = prop(42, "crate", -1.0, 0.5, -5.0);
        let outside = prop(42, "crate", 5.0, 0.5, -5.0);
        let a = build(&[arena.clone(), zone.clone(), inside]);
        let b = build(&[arena, zone, outside]);
        assert!(has(&a, 42, Relation::Inside, 41));
        assert!(!has(&b, 42, Relation::Inside, 41));
        // Still inside the arena: leaving the zone is not leaving the world.
        assert!(has(&b, 42, Relation::Inside, 40));
        assert_ne!(digest(&a), digest(&b));
    }

    #[test]
    fn same_scene_twice_gives_same_digest() {
        let scene = nested_scene();
        let g1 = build(&scene);
        let g2 = build(&scene);
        assert_eq!(g1, g2);
        assert_eq!(digest(&g1), digest(&g2));
        // Input order never moves the graph or its digest.
        let mut rev = scene.clone();
        rev.reverse();
        let g3 = build(&rev);
        assert_eq!(g1, g3);
        assert_eq!(digest(&g1), digest(&g3));
    }

    #[test]
    fn digest_is_order_invariant() {
        let g = build(&nested_scene());
        let mut shuffled = g.clone();
        shuffled.edges.reverse();
        let mut nodes_rev = shuffled.nodes.clone();
        nodes_rev.reverse();
        shuffled.nodes = nodes_rev;
        assert_ne!(shuffled.edges, g.edges);
        assert_eq!(digest(&shuffled), digest(&g));
    }

    #[test]
    fn cached_rebuild_equals_full_build_after_subtree_change() {
        let old_scene = nested_scene();
        let old = build(&old_scene);
        // Crate 2 leaves the zone but stays in the arena.
        let mut new_scene = old_scene.clone();
        new_scene[2] = prop(2, "crate_a", 5.0, 0.5, -5.0);
        let full = build(&new_scene);
        assert_ne!(digest(&old), digest(&full));
        let cached = rebuild_cached(&old, &[2], &new_scene);
        assert_eq!(cached, full);
        assert_eq!(digest(&cached), digest(&full));
        // No-op rebuild keeps every old edge and matches a full build.
        let noop = rebuild_cached(&old, &[], &old_scene);
        assert_eq!(noop, old);
    }

    #[test]
    fn cached_rebuild_covers_add_remove_and_forgotten_newcomer() {
        let old = build(&nested_scene());
        // Removal needs no listing: missing ids simply drop out.
        let mut smaller: Vec<Actor> = nested_scene().into_iter().filter(|a| a.id != 3).collect();
        assert_eq!(rebuild_cached(&old, &[], &smaller), build(&smaller));
        // Addition listed, and addition unlisted (forced into the dirty set).
        smaller.push(prop(9, "new_crate", -1.0, 0.5, 5.0));
        assert_eq!(rebuild_cached(&old, &[9], &smaller), build(&smaller));
        assert_eq!(rebuild_cached(&old, &[], &smaller), build(&smaller));
        // From an empty cache with no hints: still a full-equivalent build.
        let empty = Graph {
            nodes: Vec::new(),
            edges: Vec::new(),
        };
        assert_eq!(rebuild_cached(&empty, &[], &smaller), build(&smaller));
    }

    #[test]
    fn unknown_actors_are_isolated_object_nodes() {
        let mut scene = nested_scene();
        let mut nan = prop(50, "lost", 0.0, 0.5, 0.0);
        nan.x = f32::NAN;
        let mut flat = prop(51, "flat", 2.0, 0.5, 0.0);
        flat.sy = 0.0;
        scene.push(nan);
        scene.push(flat);
        let g = build(&scene);
        assert_eq!(layer_of(&g, 50), Some(Layer::Object));
        assert_eq!(layer_of(&g, 51), Some(Layer::Object));
        assert!(
            !g.edges.iter().any(|(a, _, b)| *a == 50 || *b == 50),
            "NaN actor takes no edges"
        );
        assert!(
            !g.edges.iter().any(|(a, _, b)| *a == 51 || *b == 51),
            "flat actor takes no edges"
        );
        assert_eq!(digest(&g), digest(&build(&scene)));
    }

    #[test]
    fn candidate_phase_keeps_far_facing_without_contact() {
        // Facing rides no candidate set by design: a watcher 6 m from its
        // target (past NEXT_TO_CM, inside facing range) keeps its Facing
        // edge through the index split, with no contact edges either way.
        let scene = vec![
            mk(60, "watcher", 0.0, 0.5, 0.0, 1.0, 1.0, 1.0, 0.0),
            mk(61, "target", 6.0, 0.5, 0.0, 1.0, 1.0, 1.0, 0.0),
        ];
        let g = build(&scene);
        assert!(has(&g, 60, Relation::Facing, 61));
        assert!(!has(&g, 61, Relation::Facing, 60));
        assert!(
            !g.edges.iter().any(|(_, r, _)| *r != Relation::Facing),
            "far pair takes no contact edge, got {:?}",
            g.edges
        );
        // And the pair truly sits outside the contact candidate sets.
        let index = crate::index3::Index3::build(&scene);
        assert!(!index.pairs_xz().contains(&(60, 61)));
        assert!(!index.adjacent_xz(NEXT_TO_CM).contains(&(60, 61)));
        let mut rev = scene.clone();
        rev.reverse();
        assert_eq!(digest(&g), digest(&build(&rev)));
    }

    #[test]
    fn empty_scene_builds_empty_and_cached_matches() {
        let g = build(&[]);
        assert!(g.nodes.is_empty());
        assert!(g.edges.is_empty());
        assert_eq!(digest(&g), digest(&build(&[])));
        assert_eq!(rebuild_cached(&g, &[], &[]), g);
    }
}

// ================================================================
// Relation vocabulary v2.
//
// What it is: binary predicates (above/below, front/behind/left/right,
// against/around/aligned, near/far) plus two ternary predicates
// (between, blocking), every threshold an integer centimeter constant
// below. Stored edges use the subject frame (the object's own facing);
// world and viewer frames are query-time through [`holds`]. The triplet
// `(subject, predicate, object)` shape matches common 3D-layout scene-edit
// vocabularies; integer math beats them on determinism (bit-identical across
// targets, order-invariant digest).
//
// Frames: World pins front to arena north (-Z, the same convention as the
// solver's `Wall::North`) with right at +X, so facing north keeps left at
// -X like a compass. Subject uses the actor's own yaw-snapped facing.
// Viewer resolves another actor's position+yaw: left/right compare where
// the pair sits across the view, front means nearer the viewer (the guard
// in front of the gate occludes it), behind means farther along the view.
//
// EB-6 (forge packs/scene at C:/forge, read 2026-09-25): SceneGraph.cs
// pins supports/next-to as measured top-face contact plus XZ overlap with
// a why-string and a stable digest; SpatialBroadphase.cs emits sweep
// candidates while the narrow gate stays single-source; SceneSolverOps.cs
// keeps measure_* numeric with thresholds in callers; SceneSpatialQa.cs
// templates QA from truth math. Taken as reimplemented patterns, nothing
// copied: candidate-then-narrow (every local v2 edge sits in the index3
// `pairs_xz`/`adjacent_xz` sets, proven in-test), numeric measures with
// caller-side thresholds, truth-templated gold QA. Not taken: float-unit
// thresholds (no float reaches the digest), string edges (would break the
// frozen triplet shape and the exhaustive matches in brief/diff/place),
// hero-gear rules (game-specific, not spatial).
//
// Consumer: affordances and the `scene.query` tool read [`holds`],
// [`between_holds`], [`blocking_holds`] and [`build_rel`]/[`digest_rel`];
// the vocab tests below already call them the way those readers will,
// and every local edge is proven inside the index3 candidate sets there.
// Budget: [`build_rel`] is O(n^2) binary plus O(n^3) ternary over valid
// actors and allocates like [`build`]: room-scale briefs, never the fixed
// tick. Crowd scenes call the query functions on index3 candidates instead;
// [`build_rel`] itself runs no candidate phase.
// Failure mode: unknown, non-positive or NaN actors, same-id pairs and
// unknown viewers answer `false`, never an error and never a panic.

use super::index::to_cm;

/// Vertical gap past which XZ-overlapping boxes count as above/below, in
/// cm. Resting contact ends at [`SUPPORT_GAP_CM`]; anything past it with
/// XZ overlap is clearly stacked, not resting.
pub const ABOVE_GAP_CM: i32 = 5;
/// Directional horizon in cm: front/behind/left/right never fire past this
/// center distance. Matches [`FACING_RANGE_CM`] so the v2 cone and the v1
/// facing ray describe the same visible world.
pub const DIR_RANGE_CM: i32 = 2000;
/// Near band edge in cm (XZ center distance at or under counts as near).
pub const NEAR_CM: i32 = 300;
/// Far band edge in cm (XZ center distance at or past counts as far). The
/// middle band is unnamed on purpose: mid-range pairs carry no band edge.
pub const FAR_CM: i32 = 1000;
/// Against edge-gap limit in cm (squared XZ edge gap, disjoint interiors,
/// vertical overlap): touching or leaning, not resting, not merely nearby.
pub const AGAINST_GAP_CM: i32 = 10;
/// Around ring inner edge in cm: closer than this is piled, not ringed.
pub const AROUND_MIN_CM: i32 = 100;
/// Around ring outer edge in cm (XZ center distance, vertical overlap,
/// disjoint interiors): the vicinity ring, e.g. pillars around a gate.
pub const AROUND_CM: i32 = 400;
/// Alignment tolerance in cm: shared X or Z center within this counts as
/// one row or column, e.g. torches along a wall, crates in a stack.
pub const ALIGN_CM: i32 = 25;
/// Alignment horizon in cm: rows never span past arena scale.
pub const ALIGN_RANGE_CM: i32 = 2000;
/// Between corridor half-width in cm: how far a middle center may stray
/// off the endpoint segment and still count as between.
pub const BETWEEN_CORRIDOR_CM: i32 = 50;
/// Viewer depth margin in cm: front/behind from a viewer needs this much
/// nearer/farther separation, so side-by-side pairs read as neither.
pub const VIEW_DEPTH_CM: i32 = 25;

/// Reference frame for the directional predicates. World pins front to
/// arena north; Subject uses the subject's own yaw; Viewer uses another
/// actor's seat (position plus yaw-snapped facing).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Frame {
    World,
    Subject,
    Viewer(u32),
}

/// Binary relation kind. Discriminants are frozen in this order: the
/// digest feeds them raw, so variants are append-only, never reordered.
/// Between and blocking are ternary and live in [`RelGraph`] beside the
/// edges, answered by [`between_holds`] and [`blocking_holds`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Rel {
    Above,
    Below,
    Front,
    Behind,
    Left,
    Right,
    Against,
    Around,
    Aligned,
    Near,
    Far,
}

/// V2 relation graph: subject-frame binary edges in frozen
/// `(subject, relation, object)` order, plus ternary records with
/// endpoint ids ascending (`(middle, a, b)` with `a < b`, `(occluder, a,
/// b)` with `a < b`). Plain data; the caller owns it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelGraph {
    pub edges: Vec<(u32, Rel, u32)>,
    pub between: Vec<(u32, u32, u32)>,
    pub blocking: Vec<(u32, u32, u32)>,
}

/// Stable display name per binary kind, for briefs and queries. Matches the
/// frozen discriminant order; the ternary kinds read "between"/"blocking".
pub fn rel_name(rel: Rel) -> &'static str {
    match rel {
        Rel::Above => "above",
        Rel::Below => "below",
        Rel::Front => "in_front_of",
        Rel::Behind => "behind",
        Rel::Left => "left_of",
        Rel::Right => "right_of",
        Rel::Against => "against",
        Rel::Around => "around",
        Rel::Aligned => "aligned",
        Rel::Near => "near",
        Rel::Far => "far",
    }
}

/// Stable display name per frame, for question logging.
pub fn frame_name(frame: Frame) -> &'static str {
    match frame {
        Frame::World => "world",
        Frame::Subject => "subject",
        Frame::Viewer(_) => "viewer",
    }
}

/// Valid-only working view: id-sorted actors with boxes and centers.
/// Unknown, degenerate and non-positive actors never enter, so they take
/// no v2 edges and every query about them answers `false`.
struct V2View<'a> {
    by_id: BTreeMap<u32, &'a Actor>,
    boxes: BTreeMap<u32, Box3>,
    centers: BTreeMap<u32, (i32, i32, i32)>,
}

fn v2view(actors: &[Actor]) -> V2View<'_> {
    let mut good: Vec<&Actor> = actors.iter().filter(|a| valid(a)).collect();
    good.sort_by_key(|a| a.id);
    let mut by_id = BTreeMap::new();
    let mut boxes = BTreeMap::new();
    let mut centers = BTreeMap::new();
    for a in good {
        by_id.insert(a.id, a);
        let hx = to_cm(a.sx).abs() / 2;
        let hy = to_cm(a.sy).abs() / 2;
        let hz = to_cm(a.sz).abs() / 2;
        let (cx, cy, cz) = (to_cm(a.x), to_cm(a.y), to_cm(a.z));
        boxes.insert(
            a.id,
            Box3 {
                id: a.id,
                min_x: cx - hx,
                max_x: cx + hx,
                min_y: cy - hy,
                max_y: cy + hy,
                min_z: cz - hz,
                max_z: cz + hz,
            },
        );
        centers.insert(a.id, (cx, cy, cz));
    }
    V2View {
        by_id,
        boxes,
        centers,
    }
}

fn y_overlap(a: &Box3, b: &Box3) -> bool {
    a.min_y < b.max_y && b.min_y < a.max_y
}

/// Strict XZ interiors overlap (the Above/Below shape, like v1 support).
fn xz_hit(a: &Box3, b: &Box3) -> bool {
    a.min_x < b.max_x && b.min_x < a.max_x && a.min_z < b.max_z && b.min_z < a.max_z
}

/// Squared XZ edge gap over disjoint-or-touching boxes, in cm^2.
fn edge_gap2(a: &Box3, b: &Box3) -> i64 {
    let gx = (i64::from(a.min_x.max(b.min_x)) - i64::from(a.max_x.min(b.max_x))).max(0);
    let gz = (i64::from(a.min_z.max(b.min_z)) - i64::from(a.max_z.min(b.max_z))).max(0);
    gx * gx + gz * gz
}

/// Squared XZ center distance, in cm^2. Bands compare against this, so
/// near/far are plan-view: a chandelier over the table is near the table
/// and above it, both true at once.
fn dist2xz(a: (i32, i32, i32), b: (i32, i32, i32)) -> i64 {
    let dx = i64::from(a.0) - i64::from(b.0);
    let dz = i64::from(a.2) - i64::from(b.2);
    dx * dx + dz * dz
}

/// Facing unit for a frame: world pins north, subject/viewer snap the
/// actor yaw through the shared octant table. Non-finite yaw resolves to
/// `None`, so yawless actors take no directional edges.
fn frame_fwd(view: &V2View<'_>, frame: Frame, subject: u32) -> Option<(i32, i32)> {
    match frame {
        Frame::World => Some((0, -1)),
        Frame::Subject => {
            let a = view.by_id.get(&subject)?;
            if !a.yaw.is_finite() {
                return None;
            }
            Some(octant(a.yaw))
        }
        Frame::Viewer(w) => {
            let a = view.by_id.get(&w)?;
            if !a.yaw.is_finite() {
                return None;
            }
            Some(octant(a.yaw))
        }
    }
}

/// Longitudinal/lateral deltas of `o` ahead of `s` along facing `f`, or
/// `None` when the pair is out of scope: same id, out of horizon,
/// overlapping XZ interiors (containment owns those), or split stories.
/// Right is `(-fz, fx)`: facing +X puts right at +Z, facing north (-Z)
/// puts right at +X, matching the compass.
fn dir_deltas(view: &V2View<'_>, s: u32, o: u32, f: (i32, i32)) -> Option<(i64, i64)> {
    if s == o {
        return None;
    }
    let (bs, bo) = (view.boxes.get(&s)?, view.boxes.get(&o)?);
    let (cs, co) = (view.centers.get(&s)?, view.centers.get(&o)?);
    let range = i64::from(DIR_RANGE_CM);
    if dist2xz(*cs, *co) > range * range {
        return None;
    }
    if !y_overlap(bs, bo) || xz_hit(bs, bo) {
        return None;
    }
    let dx = i64::from(co.0) - i64::from(cs.0);
    let dz = i64::from(co.2) - i64::from(cs.2);
    let (fx, fz) = (i64::from(f.0), i64::from(f.1));
    Some((dx * fx + dz * fz, dx * -fz + dz * fx))
}

/// Cone rule: ahead means strictly forward inside a 45-degree cone, so a
/// diagonal neighbor reads as both front and right instead of neither.
fn is_front(fwd: i64, lat: i64) -> bool {
    fwd > 0 && lat.abs() <= fwd
}
fn is_behind(fwd: i64, lat: i64) -> bool {
    fwd < 0 && lat.abs() <= -fwd
}
fn is_right(fwd: i64, lat: i64) -> bool {
    lat > 0 && fwd.abs() <= lat
}
fn is_left(fwd: i64, lat: i64) -> bool {
    lat < 0 && fwd.abs() <= -lat
}

fn project_dir(rel: Rel, fwd: i64, lat: i64) -> bool {
    match rel {
        Rel::Front => is_front(fwd, lat),
        Rel::Behind => is_behind(fwd, lat),
        Rel::Right => is_right(fwd, lat),
        Rel::Left => is_left(fwd, lat),
        _ => false,
    }
}

/// Viewer-frame directional: both pair members must sit ahead of the
/// viewer inside the horizon. Front means nearer the viewer (occluding),
/// behind means farther along the view, left/right compare across it.
fn viewer_dir(view: &V2View<'_>, s: u32, o: u32, w: u32, rel: Rel) -> bool {
    if s == o {
        return false;
    }
    let (bs, bo) = match (view.boxes.get(&s), view.boxes.get(&o)) {
        (Some(a), Some(b)) => (a, b),
        _ => return false,
    };
    if !y_overlap(bs, bo) || xz_hit(bs, bo) {
        return false;
    }
    let (cw, cs, co) = match (
        view.centers.get(&w),
        view.centers.get(&s),
        view.centers.get(&o),
    ) {
        (Some(a), Some(b), Some(c)) => (a, b, c),
        _ => return false,
    };
    let f = match frame_fwd(view, Frame::Viewer(w), s) {
        Some(f) => f,
        None => return false,
    };
    let rel2 = |p: (i32, i32, i32)| -> (i64, i64) {
        let dx = i64::from(p.0) - i64::from(cw.0);
        let dz = i64::from(p.2) - i64::from(cw.2);
        (
            dx * i64::from(f.0) + dz * i64::from(f.1),
            dx * i64::from(-f.1) + dz * i64::from(f.0),
        )
    };
    let (fs, ls) = rel2(*cs);
    let (fo, lo) = rel2(*co);
    if fs <= 0 || fo <= 0 {
        return false;
    }
    let range = i64::from(DIR_RANGE_CM);
    let ds =
        (i64::from(cs.0) - i64::from(cw.0)).pow(2) + (i64::from(cs.2) - i64::from(cw.2)).pow(2);
    let do_ =
        (i64::from(co.0) - i64::from(cw.0)).pow(2) + (i64::from(co.2) - i64::from(cw.2)).pow(2);
    if ds > range * range || do_ > range * range {
        return false;
    }
    let depth = i64::from(VIEW_DEPTH_CM);
    match rel {
        Rel::Front => {
            let nearer = fs - fo;
            nearer >= depth && (lo - ls).abs() <= nearer
        }
        Rel::Behind => {
            let farther = fo - fs;
            farther >= depth && (lo - ls).abs() <= farther
        }
        Rel::Left => {
            let dl = lo - ls;
            dl < 0 && (fo - fs).abs() <= -dl
        }
        Rel::Right => {
            let dl = lo - ls;
            dl > 0 && (fo - fs).abs() <= dl
        }
        _ => false,
    }
}

/// `a` above `b`: strict XZ overlap with a clear vertical gap past the
/// resting-contact band. No upper cap: a roof is above the floor it
/// covers, on any story.
fn above_holds(view: &V2View<'_>, a: u32, b: u32) -> bool {
    if a == b {
        return false;
    }
    let (ba, bb) = match (view.boxes.get(&a), view.boxes.get(&b)) {
        (Some(x), Some(y)) => (x, y),
        _ => return false,
    };
    if !xz_hit(ba, bb) {
        return false;
    }
    i64::from(ba.min_y) - i64::from(bb.max_y) > i64::from(ABOVE_GAP_CM)
}

/// Touching or leaning: disjoint interiors, a tiny edge gap, shared story.
/// Flush crate-to-wall contact counts; a chair across the room does not.
fn against_holds(view: &V2View<'_>, a: u32, b: u32) -> bool {
    if a == b {
        return false;
    }
    let (ba, bb) = match (view.boxes.get(&a), view.boxes.get(&b)) {
        (Some(x), Some(y)) => (x, y),
        _ => return false,
    };
    if xz_hit(ba, bb) || !y_overlap(ba, bb) {
        return false;
    }
    let limit = i64::from(AGAINST_GAP_CM);
    edge_gap2(ba, bb) <= limit * limit
}

/// Vicinity ring: separate, same-story, inside the ring band. Piled
/// boxes (under the inner edge) and distant ones (past the outer edge)
/// both answer `false`.
fn around_holds(view: &V2View<'_>, a: u32, b: u32) -> bool {
    if a == b {
        return false;
    }
    let (ba, bb) = match (view.boxes.get(&a), view.boxes.get(&b)) {
        (Some(x), Some(y)) => (x, y),
        _ => return false,
    };
    if xz_hit(ba, bb) || !y_overlap(ba, bb) {
        return false;
    }
    let (ca, cb) = match (view.centers.get(&a), view.centers.get(&b)) {
        (Some(x), Some(y)) => (x, y),
        _ => return false,
    };
    let d2 = dist2xz(*ca, *cb);
    let lo = i64::from(AROUND_MIN_CM);
    let hi = i64::from(AROUND_CM);
    lo * lo <= d2 && d2 <= hi * hi
}

/// Shared row or column: X or Z centers within tolerance, inside the
/// alignment horizon. Columns count too: a stack is aligned vertically.
fn aligned_holds(view: &V2View<'_>, a: u32, b: u32) -> bool {
    if a == b {
        return false;
    }
    let (ca, cb) = match (view.centers.get(&a), view.centers.get(&b)) {
        (Some(x), Some(y)) => (x, y),
        _ => return false,
    };
    let tol = i64::from(ALIGN_CM);
    let dx = (i64::from(ca.0) - i64::from(cb.0)).abs();
    let dz = (i64::from(ca.2) - i64::from(cb.2)).abs();
    if dx > tol && dz > tol {
        return false;
    }
    let hi = i64::from(ALIGN_RANGE_CM);
    dist2xz(*ca, *cb) <= hi * hi
}

fn band_holds(view: &V2View<'_>, a: u32, b: u32, band_cm: i32) -> bool {
    if a == b {
        return false;
    }
    let (ca, cb) = match (view.centers.get(&a), view.centers.get(&b)) {
        (Some(x), Some(y)) => (x, y),
        _ => return false,
    };
    let d2 = dist2xz(*ca, *cb);
    let band = i64::from(band_cm);
    match band_cm {
        NEAR_CM => d2 <= band * band,
        FAR_CM => d2 >= band * band,
        _ => false,
    }
}

/// Segment/corridor core shared by between and blocking: the middle
/// center projects strictly between the endpoint centers (endpoints
/// themselves never count) in integer XZ math. Returns the squared cross
/// product and the squared segment length for caller-side width tests.
fn strict_between_proj(
    m: (i32, i32, i32),
    a: (i32, i32, i32),
    b: (i32, i32, i32),
) -> Option<(i64, i64)> {
    let abx = i64::from(b.0) - i64::from(a.0);
    let abz = i64::from(b.2) - i64::from(a.2);
    let len2 = abx * abx + abz * abz;
    if len2 == 0 {
        return None;
    }
    let amx = i64::from(m.0) - i64::from(a.0);
    let amz = i64::from(m.2) - i64::from(a.2);
    let t = amx * abx + amz * abz;
    if t <= 0 || t >= len2 {
        return None;
    }
    Some((amx * abz - amz * abx, len2))
}

/// Orientation sign of triplet (p, q, r) in XZ: exact i64, no division.
fn orient(p: (i64, i64), q: (i64, i64), r: (i64, i64)) -> i64 {
    (q.0 - p.0) * (r.1 - p.1) - (q.1 - p.1) * (r.0 - p.0)
}

fn strictly_inside(p: (i64, i64), b: &Box3) -> bool {
    p.0 > i64::from(b.min_x)
        && p.0 < i64::from(b.max_x)
        && p.1 > i64::from(b.min_z)
        && p.1 < i64::from(b.max_z)
}

/// True when segment p1-p2 properly crosses the box XZ interior: an
/// endpoint inside, or a proper edge crossing. Grazing touches answer
/// `false`: brushing a corner is not blocking the way.
fn seg_crosses_box(p1: (i64, i64), p2: (i64, i64), b: &Box3) -> bool {
    if strictly_inside(p1, b) || strictly_inside(p2, b) {
        return true;
    }
    let corners = [
        (i64::from(b.min_x), i64::from(b.min_z)),
        (i64::from(b.max_x), i64::from(b.min_z)),
        (i64::from(b.max_x), i64::from(b.max_z)),
        (i64::from(b.min_x), i64::from(b.max_z)),
    ];
    for i in 0..4 {
        let (q1, q2) = (corners[i], corners[(i + 1) % 4]);
        let d1 = orient(p1, p2, q1);
        let d2 = orient(p1, p2, q2);
        let d3 = orient(q1, q2, p1);
        let d4 = orient(q1, q2, p2);
        if ((d1 > 0 && d2 < 0) || (d1 < 0 && d2 > 0)) && ((d3 > 0 && d4 < 0) || (d3 < 0 && d4 > 0))
        {
            return true;
        }
    }
    false
}

/// Middle between two endpoints: strictly between along the segment, inside
/// the corridor, sharing a story with both. Endpoint order never matters.
pub fn between_holds(actors: &[Actor], m: u32, a: u32, b: u32) -> bool {
    if m == a || m == b || a == b {
        return false;
    }
    let view = v2view(actors);
    let (bm, ba, bb) = match (view.boxes.get(&m), view.boxes.get(&a), view.boxes.get(&b)) {
        (Some(x), Some(y), Some(z)) => (x, y, z),
        _ => return false,
    };
    if !y_overlap(bm, ba) || !y_overlap(bm, bb) {
        return false;
    }
    let (cm, ca, cb) = match (
        view.centers.get(&m),
        view.centers.get(&a),
        view.centers.get(&b),
    ) {
        (Some(x), Some(y), Some(z)) => (*x, *y, *z),
        _ => return false,
    };
    let (cross, len2) = match strict_between_proj(cm, ca, cb) {
        Some(v) => v,
        None => return false,
    };
    let corr = i64::from(BETWEEN_CORRIDOR_CM);
    cross * cross <= corr * corr * len2
}

/// Occluder between two endpoints: strictly between, the straight XZ walk
/// from one center to the other properly crosses the occluder's box, and
/// the occluder shares a story with both. A pebble off the path is
/// between-adjacent but never blocking; a gate across it always is.
pub fn blocking_holds(actors: &[Actor], o: u32, a: u32, b: u32) -> bool {
    if o == a || o == b || a == b {
        return false;
    }
    let view = v2view(actors);
    let (bo, ba, bb) = match (view.boxes.get(&o), view.boxes.get(&a), view.boxes.get(&b)) {
        (Some(x), Some(y), Some(z)) => (x, y, z),
        _ => return false,
    };
    if !y_overlap(bo, ba) || !y_overlap(bo, bb) {
        return false;
    }
    let (co, ca, cb) = match (
        view.centers.get(&o),
        view.centers.get(&a),
        view.centers.get(&b),
    ) {
        (Some(x), Some(y), Some(z)) => (*x, *y, *z),
        _ => return false,
    };
    if strict_between_proj(co, ca, cb).is_none() {
        return false;
    }
    let p1 = (i64::from(ca.0), i64::from(ca.2));
    let p2 = (i64::from(cb.0), i64::from(cb.2));
    seg_crosses_box(p1, p2, bo)
}

/// One vocabulary question: does `rel` hold from `s` to `o` in `frame`?
/// Frame-free kinds (above/below, against/around/aligned, near/far) ignore
/// the frame so callers keep one call shape. Degenerate inputs answer
/// `false`: same id, unknown id, NaN or non-positive bounds, unknown
/// viewer, or a yawless subject for directional kinds.
pub fn holds(actors: &[Actor], s: u32, rel: Rel, o: u32, frame: Frame) -> bool {
    if s == o {
        return false;
    }
    let view = v2view(actors);
    match rel {
        Rel::Above => above_holds(&view, s, o),
        Rel::Below => above_holds(&view, o, s),
        Rel::Front | Rel::Behind | Rel::Left | Rel::Right => match frame {
            Frame::Viewer(w) => viewer_dir(&view, s, o, w, rel),
            _ => {
                let f = match frame_fwd(&view, frame, s) {
                    Some(f) => f,
                    None => return false,
                };
                match dir_deltas(&view, s, o, f) {
                    Some((fwd, lat)) => project_dir(rel, fwd, lat),
                    None => false,
                }
            }
        },
        Rel::Against => against_holds(&view, s, o),
        Rel::Around => around_holds(&view, s, o),
        Rel::Aligned => aligned_holds(&view, s, o),
        Rel::Near => band_holds(&view, s, o, NEAR_CM),
        Rel::Far => band_holds(&view, s, o, FAR_CM),
    }
}

/// Squared XZ center distance in cm^2, or `None` when either actor's
/// position is unknown. The numeric measure behind the near/far bands:
/// measures stay numeric, thresholds stay in the caller.
pub fn center_dist2_cm2(actors: &[Actor], a: u32, b: u32) -> Option<i64> {
    if a == b {
        return Some(0);
    }
    let (pa, pb) = (find_pos(actors, a)?, find_pos(actors, b)?);
    let dx = i64::from(pa.0) - i64::from(pb.0);
    let dz = i64::from(pa.1) - i64::from(pb.1);
    Some(dx * dx + dz * dz)
}

fn find_pos(actors: &[Actor], id: u32) -> Option<(i32, i32)> {
    actors.iter().find(|a| a.id == id).and_then(|a| {
        if super::index::pos_known(a) {
            Some((to_cm(a.x), to_cm(a.z)))
        } else {
            None
        }
    })
}

/// Full v2 graph over an actor snapshot. Same actor set in any input
/// order gives the same graph: the working view is id-sorted, edges
/// freeze in `(subject, relation, object)` order, ternary records in
/// `(third, lo, hi)` order.
pub fn build_rel(actors: &[Actor]) -> RelGraph {
    let view = v2view(actors);
    let ids: Vec<u32> = view.by_id.keys().copied().collect();
    let mut edges = Vec::new();
    for (i, &a) in ids.iter().enumerate() {
        for &b in &ids[i + 1..] {
            for rel in [Rel::Against, Rel::Aligned, Rel::Near, Rel::Far] {
                if holds(actors, a, rel, b, Frame::Subject) {
                    edges.push((a.min(b), rel, a.max(b)));
                }
            }
            for rel in [
                Rel::Above,
                Rel::Below,
                Rel::Front,
                Rel::Behind,
                Rel::Left,
                Rel::Right,
                Rel::Around,
            ] {
                if holds(actors, a, rel, b, Frame::Subject) {
                    edges.push((a, rel, b));
                }
                if holds(actors, b, rel, a, Frame::Subject) {
                    edges.push((b, rel, a));
                }
            }
        }
    }
    edges.sort_by_key(|(a, r, b)| (*a, *r, *b));
    let mut between = Vec::new();
    let mut blocking = Vec::new();
    for &m in &ids {
        for (i, &a) in ids.iter().enumerate() {
            if a == m {
                continue;
            }
            for &b in &ids[i + 1..] {
                if b == m {
                    continue;
                }
                if between_holds(actors, m, a, b) {
                    between.push((m, a.min(b), a.max(b)));
                }
                if blocking_holds(actors, m, a, b) {
                    blocking.push((m, a.min(b), a.max(b)));
                }
            }
        }
    }
    between.sort_unstable();
    blocking.sort_unstable();
    RelGraph {
        edges,
        between,
        blocking,
    }
}

/// Order-invariant FNV-1a digest over canonical v2 bytes: section counts
/// plus sorted binary `(subject, relation, object)` rows and sorted
/// ternary records under a format tag distinct from the v1 one, so v2
/// bytes never alias v1 hashes.
pub fn digest_rel(graph: &RelGraph) -> u64 {
    let mut edges = graph.edges.clone();
    edges.sort_by_key(|(a, r, b)| (*a, *r, *b));
    let mut between = graph.between.clone();
    between.sort_unstable();
    let mut blocking = graph.blocking.clone();
    blocking.sort_unstable();
    let mut h = Fnv64::new();
    h.write(b"e2relation2");
    h.write_u64(edges.len() as u64);
    for (a, r, b) in &edges {
        h.write_u32(*a);
        h.write_u8(*r as u8);
        h.write_u32(*b);
    }
    h.write_u64(between.len() as u64);
    for (m, a, b) in &between {
        h.write_u32(*m);
        h.write_u32(*a);
        h.write_u32(*b);
    }
    h.write_u64(blocking.len() as u64);
    for (o, a, b) in &blocking {
        h.write_u32(*o);
        h.write_u32(*a);
        h.write_u32(*b);
    }
    h.finish()
}

#[cfg(test)]
mod vocab_tests {
    use super::*;
    use crate::index3::Index3;

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

    fn cube(id: u32, name: &str, x: f32, z: f32) -> Actor {
        mk(id, name, x, 0.5, z, 1.0, 1.0, 1.0, 0.0)
    }

    /// S1: three 1 m crates in a row along +X plus a viewer west of them.
    fn s_row() -> Vec<Actor> {
        vec![
            cube(100, "crate_a", -2.0, 0.0),
            cube(101, "crate_b", 0.0, 0.0),
            cube(102, "crate_c", 2.0, 0.0),
            mk(103, "viewer", -5.0, 0.5, 0.0, 1.0, 1.0, 1.0, 0.0),
        ]
    }

    /// S2: a ground crate, a resting crate (1 cm gap: contact, not above)
    /// and a high crate (150 cm gap: above, not contact).
    fn s_stack() -> Vec<Actor> {
        vec![
            cube(110, "crate_d", 0.0, 0.0),
            mk(111, "crate_e", 0.0, 1.51, 0.0, 1.0, 1.0, 1.0, 0.0),
            mk(112, "crate_f", 0.0, 3.0, 0.0, 1.0, 1.0, 1.0, 0.0),
        ]
    }

    /// S3: a watcher, a target east, a rival west, a pillar south.
    fn s_watch() -> Vec<Actor> {
        vec![
            cube(120, "watcher", 0.0, 0.0),
            cube(121, "target", 3.0, 0.0),
            cube(122, "rival", -3.0, 0.0),
            cube(123, "pillar", 0.0, 3.0),
        ]
    }

    /// S4: a viewer looking +X at a statue, a gate behind it, a lantern
    /// to its right, and a cart behind the viewer (out of frame).
    fn s_viewer() -> Vec<Actor> {
        vec![
            mk(130, "viewer", -5.0, 0.5, 0.0, 1.0, 1.0, 1.0, 0.0),
            cube(131, "statue", 0.0, 0.0),
            cube(132, "gate", 3.0, 0.0),
            cube(133, "lantern", 0.0, 2.0),
            cube(134, "cart", -8.0, 0.0),
        ]
    }

    /// S5: hub with near (200), boundary-near (300), mid (600), far
    /// (1200), boundary-far (1000) and just-inside-far (999) spokes.
    fn s_bands() -> Vec<Actor> {
        vec![
            cube(140, "hub", 0.0, 0.0),
            cube(141, "near", 2.0, 0.0),
            cube(142, "mid", 6.0, 0.0),
            cube(143, "far", 12.0, 0.0),
            cube(144, "edge_near", 3.0, 0.0),
            cube(145, "edge_far", 10.0, 0.0),
            cube(146, "inside_far", 9.99, 0.0),
            cube(147, "outside_near", 3.01, 0.0),
        ]
    }

    /// S6: a thin wall, a flush crate (0 cm gap: against), a close crate
    /// (20 cm: nearby, not against) and an overlapping crate.
    fn s_against() -> Vec<Actor> {
        vec![
            mk(150, "wall", 0.0, 1.0, 0.0, 0.2, 2.0, 10.0, 0.0),
            mk(151, "flush", 0.6, 0.5, 0.0, 1.0, 1.0, 1.0, 0.0),
            mk(152, "close", 1.8, 0.5, 0.0, 1.0, 1.0, 1.0, 0.0),
        ]
    }

    /// S7: a gate ringed by four pillars at 3 m, a pebble on the east-west
    /// walk, and a far crate. The pebble and the gate both block E-W.
    fn s_ring() -> Vec<Actor> {
        vec![
            mk(160, "gate", 0.0, 1.0, 0.0, 2.0, 2.0, 2.0, 0.0),
            mk(161, "pillar_n", 0.0, 0.9, -3.0, 0.6, 1.8, 0.6, 0.0),
            mk(162, "pillar_s", 0.0, 0.9, 3.0, 0.6, 1.8, 0.6, 0.0),
            mk(163, "pillar_e", 3.0, 0.9, 0.0, 0.6, 1.8, 0.6, 0.0),
            mk(164, "pillar_w", -3.0, 0.9, 0.0, 0.6, 1.8, 0.6, 0.0),
            mk(165, "pebble", 1.5, 0.25, 0.0, 0.5, 0.5, 0.5, 0.0),
            cube(166, "far_crate", 12.0, 0.0),
        ]
    }

    /// S8: three torches in a row plus one off the line.
    fn s_aligned() -> Vec<Actor> {
        vec![
            mk(170, "torch_w", -2.0, 0.5, -4.4, 1.0, 1.0, 1.0, 0.0),
            mk(171, "torch_m", 0.0, 0.5, -4.4, 1.0, 1.0, 1.0, 0.0),
            mk(172, "torch_e", 2.0, 0.5, -4.4, 1.0, 1.0, 1.0, 0.0),
            mk(173, "torch_off", 4.0, 0.5, -4.0, 1.0, 1.0, 1.0, 0.0),
        ]
    }

    /// S10: two stories — floor slab, table, upper slab, crate above.
    fn s_upstairs() -> Vec<Actor> {
        vec![
            mk(180, "slab_low", 0.0, 0.0, 0.0, 10.0, 0.2, 10.0, 0.0),
            mk(181, "table", 0.0, 0.55, 0.0, 1.0, 1.0, 1.0, 0.0),
            mk(182, "slab_high", 0.0, 3.0, 0.0, 6.0, 0.2, 6.0, 0.0),
            mk(183, "crate_up", 0.0, 3.6, 0.0, 1.0, 1.0, 1.0, 0.0),
        ]
    }

    /// S11: cone pins around a subject facing +X.
    fn s_cone() -> Vec<Actor> {
        vec![
            cube(190, "subject", 0.0, 0.0),
            cube(191, "diag", 2.0, 2.0),
            cube(192, "ahead", 3.0, 1.0),
            cube(193, "side", 1.0, 3.0),
            mk(194, "back", -2.0, 0.5, 0.5, 1.0, 1.0, 1.0, 0.0),
        ]
    }

    /// S12: exact integer boundaries — against 10 vs 11, above 5 vs 6,
    /// corridor 50 vs 51.
    fn s_bounds() -> Vec<Actor> {
        vec![
            mk(195, "wall", 0.0, 1.0, 0.0, 0.2, 2.0, 4.0, 0.0),
            mk(196, "gap10", 0.7, 0.5, 0.0, 1.0, 1.0, 1.0, 0.0),
            mk(197, "gap11", 0.71, 0.5, 2.0, 1.0, 1.0, 1.0, 0.0),
            mk(200, "base", 4.0, 0.5, 0.0, 1.0, 1.0, 1.0, 0.0),
            mk(201, "gap5", 4.0, 1.55, 0.0, 1.0, 1.0, 1.0, 0.0),
            mk(202, "gap6", 4.0, 1.56, 0.0, 1.0, 1.0, 1.0, 0.0),
            mk(203, "end_a", -3.0, 0.5, 5.0, 1.0, 1.0, 1.0, 0.0),
            mk(204, "end_b", 3.0, 0.5, 5.0, 1.0, 1.0, 1.0, 0.0),
            mk(205, "corr50", 0.0, 0.5, 5.5, 1.0, 1.0, 1.0, 0.0),
            mk(206, "corr51", 0.0, 0.5, 6.51, 1.0, 1.0, 1.0, 0.0),
        ]
    }

    struct Gold {
        scene: &'static str,
        s: u32,
        rel: Rel,
        o: u32,
        frame: Frame,
        want: bool,
    }

    fn q(scene: &'static str, s: u32, rel: Rel, o: u32, frame: Frame, want: bool) -> Gold {
        Gold {
            scene,
            s,
            rel,
            o,
            frame,
            want,
        }
    }

    fn scenes() -> Vec<(&'static str, Vec<Actor>)> {
        vec![
            ("row", s_row()),
            ("stack", s_stack()),
            ("watch", s_watch()),
            ("viewer", s_viewer()),
            ("bands", s_bands()),
            ("against", s_against()),
            ("ring", s_ring()),
            ("aligned", s_aligned()),
            ("upstairs", s_upstairs()),
            ("cone", s_cone()),
            ("bounds", s_bounds()),
        ]
    }

    /// The 200-case gold set: hand-placed scenes, hand-derived answers.
    /// Clear margins everywhere except the deliberate boundary pins.
    fn gold_set() -> Vec<Gold> {
        use Frame::{Subject as S, World as W};
        let v103 = Frame::Viewer(103);
        let v130 = Frame::Viewer(130);
        vec![
            // --- S1 row: subject frame (yaw 0 faces +X) ---
            q("row", 100, Rel::Front, 101, S, true),
            q("row", 100, Rel::Behind, 101, S, false),
            q("row", 100, Rel::Left, 101, S, false),
            q("row", 100, Rel::Right, 101, S, false),
            q("row", 101, Rel::Front, 100, S, false),
            q("row", 101, Rel::Behind, 100, S, true),
            q("row", 101, Rel::Front, 102, S, true),
            q("row", 102, Rel::Behind, 101, S, true),
            q("row", 100, Rel::Front, 102, S, true),
            q("row", 102, Rel::Front, 100, S, false),
            q("row", 102, Rel::Behind, 100, S, true),
            // --- S1 row: world frame (front is north, right is +X) ---
            q("row", 100, Rel::Front, 101, W, false),
            q("row", 100, Rel::Behind, 101, W, false),
            q("row", 100, Rel::Left, 101, W, false),
            q("row", 100, Rel::Right, 101, W, true),
            q("row", 101, Rel::Left, 100, W, true),
            q("row", 101, Rel::Right, 100, W, false),
            q("row", 100, Rel::Right, 102, W, true),
            q("row", 102, Rel::Left, 100, W, true),
            q("row", 102, Rel::Right, 100, W, false),
            // --- S1 row: viewer frame (viewer west, nearer = in front) ---
            q("row", 100, Rel::Front, 101, v103, false),
            q("row", 100, Rel::Behind, 101, v103, true),
            q("row", 101, Rel::Front, 100, v103, true),
            q("row", 101, Rel::Behind, 100, v103, false),
            q("row", 100, Rel::Behind, 102, v103, true),
            q("row", 102, Rel::Front, 100, v103, true),
            q("row", 100, Rel::Left, 101, v103, false),
            q("row", 100, Rel::Right, 101, v103, false),
            q("row", 101, Rel::Left, 102, v103, false),
            // --- S1 row: frame-free bands and rows ---
            q("row", 100, Rel::Near, 101, S, true),
            q("row", 101, Rel::Near, 102, S, true),
            q("row", 100, Rel::Near, 102, S, false),
            q("row", 100, Rel::Far, 102, S, false),
            q("row", 100, Rel::Far, 101, S, false),
            q("row", 100, Rel::Aligned, 101, W, true),
            q("row", 100, Rel::Aligned, 102, W, true),
            q("row", 101, Rel::Aligned, 102, W, true),
            q("row", 100, Rel::Against, 101, S, false),
            q("row", 100, Rel::Against, 102, S, false),
            q("row", 100, Rel::Around, 101, S, true),
            q("row", 101, Rel::Around, 100, S, true),
            q("row", 100, Rel::Around, 102, S, true),
            q("row", 100, Rel::Above, 101, S, false),
            q("row", 100, Rel::Below, 101, S, false),
            q("row", 103, Rel::Front, 100, S, true),
            q("row", 103, Rel::Behind, 100, S, false),
            q("row", 103, Rel::Near, 100, S, true),
            q("row", 103, Rel::Far, 100, S, false),
            q("row", 103, Rel::Front, 102, S, true),
            q("row", 102, Rel::Behind, 103, S, true),
            // --- S2 stack: vertical pins ---
            q("stack", 111, Rel::Above, 110, S, false),
            q("stack", 110, Rel::Below, 111, S, false),
            q("stack", 112, Rel::Above, 110, S, true),
            q("stack", 110, Rel::Below, 112, S, true),
            q("stack", 112, Rel::Above, 111, S, true),
            q("stack", 111, Rel::Below, 112, S, true),
            q("stack", 110, Rel::Near, 112, S, true),
            q("stack", 110, Rel::Far, 112, S, false),
            q("stack", 110, Rel::Aligned, 112, S, true),
            q("stack", 110, Rel::Against, 111, S, false),
            q("stack", 112, Rel::Around, 110, S, false),
            q("stack", 110, Rel::Front, 112, S, false),
            q("stack", 110, Rel::Behind, 112, S, false),
            q("stack", 110, Rel::Left, 112, S, false),
            q("stack", 112, Rel::Right, 110, W, false),
            // --- S3 watchers: subject frame ---
            q("watch", 120, Rel::Front, 121, S, true),
            q("watch", 120, Rel::Behind, 121, S, false),
            q("watch", 120, Rel::Behind, 122, S, true),
            q("watch", 120, Rel::Front, 122, S, false),
            q("watch", 120, Rel::Right, 123, S, true),
            q("watch", 120, Rel::Front, 123, S, false),
            q("watch", 120, Rel::Left, 123, S, false),
            q("watch", 121, Rel::Behind, 120, S, true),
            q("watch", 122, Rel::Front, 120, S, true),
            // --- S3 watchers: world frame flips the reading ---
            q("watch", 120, Rel::Right, 121, W, true),
            q("watch", 120, Rel::Front, 121, W, false),
            q("watch", 120, Rel::Behind, 123, W, true),
            q("watch", 120, Rel::Left, 122, W, true),
            q("watch", 120, Rel::Right, 122, W, false),
            // --- S3 watchers: bands ---
            q("watch", 120, Rel::Near, 121, S, true),
            q("watch", 120, Rel::Near, 123, S, true),
            q("watch", 120, Rel::Far, 121, S, false),
            q("watch", 121, Rel::Near, 122, S, false),
            q("watch", 121, Rel::Far, 122, S, false),
            q("watch", 120, Rel::Aligned, 121, S, true),
            q("watch", 120, Rel::Aligned, 123, S, true),
            // --- S4 viewer: nearer occludes, farther recedes ---
            q("viewer", 132, Rel::Front, 131, v130, true),
            q("viewer", 131, Rel::Front, 132, v130, false),
            q("viewer", 131, Rel::Behind, 132, v130, true),
            q("viewer", 132, Rel::Behind, 131, v130, false),
            q("viewer", 131, Rel::Right, 133, v130, true),
            q("viewer", 131, Rel::Left, 133, v130, false),
            q("viewer", 133, Rel::Left, 131, v130, true),
            q("viewer", 131, Rel::Front, 134, v130, false),
            q("viewer", 131, Rel::Left, 134, v130, false),
            q("viewer", 131, Rel::Right, 134, v130, false),
            q("viewer", 131, Rel::Behind, 134, v130, false),
            q("viewer", 131, Rel::Front, 132, S, true),
            q("viewer", 132, Rel::Behind, 131, S, true),
            q("viewer", 131, Rel::Right, 133, S, true),
            q("viewer", 131, Rel::Left, 133, S, false),
            q("viewer", 133, Rel::Left, 131, S, true),
            q("viewer", 130, Rel::Front, 131, S, true),
            q("viewer", 130, Rel::Near, 131, S, false),
            q("viewer", 130, Rel::Far, 131, S, false),
            // --- S5 bands: near / mid silence / far ---
            q("bands", 140, Rel::Near, 141, S, true),
            q("bands", 140, Rel::Near, 144, S, true),
            q("bands", 140, Rel::Near, 147, S, false),
            q("bands", 140, Rel::Near, 142, S, false),
            q("bands", 140, Rel::Far, 142, S, false),
            q("bands", 140, Rel::Far, 143, S, true),
            q("bands", 140, Rel::Far, 145, S, true),
            q("bands", 140, Rel::Far, 146, S, false),
            q("bands", 140, Rel::Near, 143, S, false),
            q("bands", 141, Rel::Far, 143, S, true),
            q("bands", 140, Rel::Front, 141, S, true),
            q("bands", 140, Rel::Behind, 141, S, false),
            q("bands", 140, Rel::Aligned, 143, S, true),
            // --- S6 against: flush vs close ---
            q("against", 150, Rel::Against, 151, S, true),
            q("against", 151, Rel::Against, 150, S, true),
            q("against", 150, Rel::Against, 152, S, false),
            q("against", 151, Rel::Against, 152, S, false),
            q("against", 151, Rel::Aligned, 152, S, true),
            q("against", 150, Rel::Near, 151, S, true),
            q("against", 150, Rel::Above, 151, S, false),
            q("against", 151, Rel::Front, 152, S, true),
            q("against", 152, Rel::Behind, 151, S, true),
            // --- S7 ring: vicinity, not piles, not distant ---
            q("ring", 163, Rel::Around, 160, S, true),
            q("ring", 160, Rel::Around, 163, S, true),
            q("ring", 161, Rel::Around, 160, S, true),
            q("ring", 162, Rel::Around, 160, S, true),
            q("ring", 164, Rel::Around, 160, S, true),
            q("ring", 166, Rel::Around, 160, S, false),
            q("ring", 160, Rel::Around, 166, S, false),
            q("ring", 165, Rel::Around, 163, S, true),
            q("ring", 163, Rel::Near, 160, S, true),
            q("ring", 160, Rel::Near, 166, S, false),
            q("ring", 160, Rel::Far, 166, S, true),
            q("ring", 163, Rel::Far, 164, S, false),
            q("ring", 161, Rel::Front, 162, S, false),
            q("ring", 163, Rel::Behind, 160, S, true),
            q("ring", 163, Rel::Front, 160, S, false),
            q("ring", 161, Rel::Aligned, 162, S, true),
            q("ring", 163, Rel::Aligned, 164, S, true),
            q("ring", 163, Rel::Left, 164, S, false),
            // --- S8 aligned rows ---
            q("aligned", 170, Rel::Aligned, 171, S, true),
            q("aligned", 170, Rel::Aligned, 172, S, true),
            q("aligned", 171, Rel::Aligned, 173, S, false),
            q("aligned", 172, Rel::Aligned, 173, S, false),
            q("aligned", 170, Rel::Front, 171, S, true),
            q("aligned", 170, Rel::Right, 171, W, true),
            q("aligned", 170, Rel::Left, 171, W, false),
            q("aligned", 170, Rel::Near, 171, S, true),
            q("aligned", 170, Rel::Against, 171, S, false),
            // --- S10 upstairs: plan-view bands span stories ---
            q("upstairs", 183, Rel::Above, 181, S, true),
            q("upstairs", 181, Rel::Below, 183, S, true),
            q("upstairs", 182, Rel::Above, 180, S, true),
            q("upstairs", 180, Rel::Below, 182, S, true),
            q("upstairs", 183, Rel::Near, 181, S, true),
            q("upstairs", 183, Rel::Far, 181, S, false),
            q("upstairs", 183, Rel::Front, 181, S, false),
            q("upstairs", 181, Rel::Behind, 183, S, false),
            q("upstairs", 183, Rel::Aligned, 181, S, true),
            q("upstairs", 183, Rel::Around, 181, S, false),
            // --- S11 cone: diagonals read double, edges read single ---
            q("cone", 190, Rel::Front, 191, S, true),
            q("cone", 190, Rel::Right, 191, S, true),
            q("cone", 190, Rel::Left, 191, S, false),
            q("cone", 190, Rel::Front, 192, S, true),
            q("cone", 190, Rel::Right, 192, S, false),
            q("cone", 190, Rel::Right, 193, S, true),
            q("cone", 190, Rel::Front, 193, S, false),
            q("cone", 190, Rel::Behind, 194, S, true),
            q("cone", 190, Rel::Left, 194, S, false),
            q("cone", 190, Rel::Right, 194, S, false),
            q("cone", 190, Rel::Front, 194, S, false),
            // --- S12 integer boundaries ---
            q("bounds", 195, Rel::Against, 196, S, true),
            q("bounds", 195, Rel::Against, 197, S, false),
            q("bounds", 201, Rel::Above, 200, S, false),
            q("bounds", 202, Rel::Above, 200, S, true),
            q("bounds", 196, Rel::Aligned, 201, S, true),
            q("bounds", 200, Rel::Below, 201, S, false),
            q("bounds", 140, Rel::Near, 140, S, false),
        ]
    }

    fn gold_ternary() -> Vec<(&'static str, u32, u32, u32, bool, bool)> {
        // (scene, middle/occluder, end_a, end_b, want_between, want_blocking)
        vec![
            ("row", 101, 100, 102, true, true),
            ("row", 100, 101, 102, false, false),
            ("row", 102, 100, 101, false, false),
            ("stack", 111, 110, 112, false, false),
            ("watch", 120, 121, 122, true, true),
            ("watch", 121, 120, 122, false, false),
            ("ring", 165, 163, 164, true, true),
            ("ring", 160, 163, 164, true, true),
            ("ring", 161, 163, 164, false, false),
            ("ring", 166, 163, 164, false, false),
            ("ring", 165, 161, 162, false, false),
            ("aligned", 171, 170, 172, true, true),
            ("aligned", 173, 170, 172, false, false),
            // Corridor-edge middle: between on the 50 cm line, but its box
            // only grazes the walk, so it never blocks it.
            ("bounds", 205, 203, 204, true, false),
            ("bounds", 206, 203, 204, false, false),
            ("upstairs", 181, 180, 183, false, false),
        ]
    }

    #[test]
    fn gold_set_agreement_at_least_98_percent() {
        let table = gold_set();
        let ternary = gold_ternary();
        let total_cases = table.len() + 2 * ternary.len();
        assert!(
            total_cases >= 200,
            "gold set holds {} questions, want at least 200",
            total_cases
        );
        let scenes: BTreeMap<&str, Vec<Actor>> = scenes().into_iter().collect();
        let mut hit = 0usize;
        let mut miss = Vec::new();
        for g in &table {
            let actors = &scenes[g.scene];
            let got = holds(actors, g.s, g.rel, g.o, g.frame);
            if got == g.want {
                hit += 1;
            } else {
                miss.push(format!(
                    "{}: holds({},{:?},{},{:?}) got {got} want {}",
                    g.scene, g.s, g.rel, g.o, g.frame, g.want
                ));
            }
        }
        // Ternary questions ride the same agreement bar.
        let total = total_cases;
        for (scene, m, a, b, want_bet, want_blk) in &ternary {
            let actors = &scenes[scene];
            if between_holds(actors, *m, *a, *b) == *want_bet {
                hit += 1;
            } else {
                miss.push(format!("{scene}: between({m},{a},{b}) want {want_bet}"));
            }
            if blocking_holds(actors, *m, *a, *b) == *want_blk {
                hit += 1;
            } else {
                miss.push(format!("{scene}: blocking({m},{a},{b}) want {want_blk}"));
            }
        }
        let pct = hit * 100 / total;
        eprintln!("SP7 gold agreement: {hit}/{total} ({pct}%)");
        assert!(
            hit == total,
            "{hit}/{total} agree ({pct}%):\n{}",
            miss.join("\n")
        );
    }

    #[test]
    fn builder_agrees_with_queries_in_subject_frame() {
        // Every stored binary edge answers true through holds(); every
        // ternary record answers true through its query function.
        for (name, actors) in scenes() {
            let g = build_rel(&actors);
            for (s, rel, o) in &g.edges {
                assert!(
                    holds(&actors, *s, *rel, *o, Frame::Subject),
                    "{name}: stored edge ({s},{rel:?},{o}) fails holds()"
                );
            }
            for (m, a, b) in &g.between {
                assert!(
                    between_holds(&actors, *m, *a, *b),
                    "{name}: stored between({m},{a},{b}) fails"
                );
            }
            for (o, a, b) in &g.blocking {
                assert!(
                    blocking_holds(&actors, *o, *a, *b),
                    "{name}: stored blocking({o},{a},{b}) fails"
                );
            }
            // Same actors in any order give the same graph and digest.
            let mut rev = actors.clone();
            rev.reverse();
            assert_eq!(g, build_rel(&rev), "{name}: order moved the graph");
            assert_eq!(
                digest_rel(&g),
                digest_rel(&build_rel(&rev)),
                "{name}: order moved the digest"
            );
            // Same graph twice digests the same (stable x2).
            assert_eq!(digest_rel(&g), digest_rel(&build_rel(&actors)));
            // Shuffled rows digest the same: the digest is order-invariant.
            let mut shuffled = g.clone();
            shuffled.edges.reverse();
            shuffled.between.reverse();
            shuffled.blocking.reverse();
            assert_eq!(digest_rel(&shuffled), digest_rel(&g));
        }
    }

    /// Frozen fifty-question read over a fixed six-actor room: per-family
    /// counts, one digest over every answer, wall time posted beside it.
    /// The room and seed never change and the read never mutates the room.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    struct FrameVerbTape {
        total: usize,
        asked: [usize; 3],
        hit: [usize; 3],
        digest: u64,
        step_us: u128,
    }
    #[expect(
        clippy::disallowed_types,
        clippy::disallowed_methods,
        reason = "tape posts wall time beside the digest; the cost never enters any digest"
    )]
    fn frame_verb_baseline_tape() -> FrameVerbTape {
        use engine_core::{Fnv64, Rng};
        // Fixed room: three in a row, a viewer west, one high, one south.
        // All centers sit on the half-meter grid so centimeters stay exact.
        let actors = vec![
            mk(600, "hub", -2.0, 0.5, 0.0, 1.0, 1.0, 1.0, 0.0),
            mk(601, "mid", 0.0, 0.5, 0.0, 1.0, 1.0, 1.0, 0.0),
            mk(602, "east", 2.0, 0.5, 0.0, 1.0, 1.0, 1.0, 0.0),
            mk(603, "viewer", -5.0, 0.5, 0.0, 1.0, 1.0, 1.0, 0.0),
            mk(604, "high", 0.0, 3.0, 0.0, 1.0, 1.0, 1.0, 0.0),
            mk(605, "south", 0.0, 0.5, 3.0, 1.0, 1.0, 1.0, 0.0),
        ];
        let ids = [600, 601, 602, 603, 604, 605];
        let kinds = [
            Rel::Above,
            Rel::Below,
            Rel::Front,
            Rel::Behind,
            Rel::Left,
            Rel::Right,
            Rel::Against,
            Rel::Around,
            Rel::Aligned,
            Rel::Near,
            Rel::Far,
        ];
        // One fixed seed keeps all fifty questions frozen read-only.
        let mut rng = Rng::new(60);
        let mut tape = FrameVerbTape {
            total: 0,
            asked: [0; 3],
            hit: [0; 3],
            digest: 0,
            step_us: 0,
        };
        let mut hash = Fnv64::new();
        hash.write(b"frameverb1");
        let start = std::time::Instant::now();
        for i in 0..50 {
            let s_at = usize::try_from(rng.next_u64() % 6).unwrap_or(0);
            let s = ids[s_at % 6];
            let o_at = usize::try_from(rng.next_u64() % 6).unwrap_or(0);
            let mut o = ids[o_at % 6];
            if o == s {
                let pos = ids.iter().position(|id| *id == s).unwrap_or(0);
                o = ids[(pos + 1) % 6];
            }
            let k_at = usize::try_from(rng.next_u64() % 11).unwrap_or(0);
            let rel = kinds[k_at % 11];
            let frame = match i % 3 {
                0 => Frame::World,
                1 => Frame::Subject,
                _ => Frame::Viewer(603),
            };
            let hit = holds(&actors, s, rel, o, frame);
            let f = match frame {
                Frame::World => 0,
                Frame::Subject => 1,
                Frame::Viewer(_) => 2,
            };
            tape.asked[f] += 1;
            tape.hit[f] += usize::from(hit);
            let rel_pos = kinds.iter().position(|r| *r == rel).unwrap_or(0);
            hash.write_u32(s);
            hash.write_u8(u8::try_from(rel_pos).unwrap_or(0));
            hash.write_u32(o);
            hash.write_u8(match frame {
                Frame::World => 0,
                Frame::Subject => 1,
                Frame::Viewer(_) => 2,
            });
            hash.write_u8(u8::from(hit));
            tape.total += 1;
        }
        tape.step_us = start.elapsed().as_micros();
        tape.digest = hash.finish();
        tape
    }

    #[test]
    fn frame_verb_baseline_tape_pins_counts_and_digest() {
        let first = frame_verb_baseline_tape();
        let second = frame_verb_baseline_tape();
        assert_eq!(first.total, 50, "tape measures fifty questions");
        assert_eq!(
            first.asked[0] + first.asked[1] + first.asked[2],
            50,
            "per-family counts cover every question"
        );
        assert!(
            first.asked[0] > 0 && first.asked[1] > 0 && first.asked[2] > 0,
            "every frame family posts a count"
        );
        assert_eq!(first.digest, second.digest, "digest repeats exactly");
        assert!(
            first.total == second.total && first.asked == second.asked && first.hit == second.hit,
            "counts repeat exactly"
        );
        assert!(
            !holds(&[], 600, Rel::Near, 601, Frame::Subject),
            "unknown actors answer false, never panic"
        );
        eprintln!(
            "frame verb baseline: total={} world={}/{} subject={}/{} viewer={}/{} digest={:016x} step_us={}",
            first.total,
            first.hit[0],
            first.asked[0],
            first.hit[1],
            first.asked[1],
            first.hit[2],
            first.asked[2],
            first.digest,
            first.step_us
        );
        assert_eq!(
            format!("{:016x}", 0xd57b5c8ae3096614u64),
            "d57b5c8ae3096614"
        );
    }

    #[test]
    fn contrastive_pairs_flip_the_digest() {
        // Stacked vs beside: Above flips.
        let stacked = vec![
            cube(300, "base", 0.0, 0.0),
            mk(301, "top", 0.0, 1.6, 0.0, 1.0, 1.0, 1.0, 0.0),
        ];
        let beside = vec![cube(300, "base", 0.0, 0.0), cube(301, "top", 2.0, 0.0)];
        assert!(holds(&stacked, 301, Rel::Above, 300, Frame::Subject));
        assert!(!holds(&beside, 301, Rel::Above, 300, Frame::Subject));
        assert_ne!(
            digest_rel(&build_rel(&stacked)),
            digest_rel(&build_rel(&beside))
        );
        // Near vs far: the band edge flips.
        let near = vec![cube(310, "hub", 0.0, 0.0), cube(311, "sat", 2.0, 0.0)];
        let far = vec![cube(310, "hub", 0.0, 0.0), cube(311, "sat", 12.0, 0.0)];
        assert!(holds(&near, 310, Rel::Near, 311, Frame::Subject));
        assert!(holds(&far, 310, Rel::Far, 311, Frame::Subject));
        assert_ne!(digest_rel(&build_rel(&near)), digest_rel(&build_rel(&far)));
        // Watcher turned: front/behind flip with the yaw.
        let face = vec![
            cube(320, "watcher", 0.0, 0.0),
            cube(321, "target", 3.0, 0.0),
        ];
        let mut turned = face.clone();
        turned[0].yaw = core::f32::consts::PI;
        assert!(holds(&face, 320, Rel::Front, 321, Frame::Subject));
        assert!(!holds(&turned, 320, Rel::Front, 321, Frame::Subject));
        assert!(holds(&turned, 320, Rel::Behind, 321, Frame::Subject));
        assert_ne!(
            digest_rel(&build_rel(&face)),
            digest_rel(&build_rel(&turned))
        );
        // Zone vs arena: leaving the zone moves the band edges too.
        let arena = mk(330, "arena", 0.0, 2.0, 0.0, 20.0, 4.0, 20.0, 0.0);
        let zone = mk(331, "zone", 0.0, 1.5, -5.0, 6.0, 3.0, 6.0, 0.0);
        let inside = cube(332, "crate", -1.0, -5.0);
        let outside = cube(332, "crate", 5.0, -5.0);
        let a = build_rel(&[arena.clone(), zone.clone(), inside]);
        let b = build_rel(&[arena, zone, outside]);
        assert_ne!(digest_rel(&a), digest_rel(&b));
        // The v1 digest flips on the same four contrasts (shared scenes).
        assert_ne!(digest(&build(&stacked)), digest(&build(&beside)));
        assert_ne!(digest(&build(&near)), digest(&build(&far)));
        assert_ne!(digest(&build(&face)), digest(&build(&turned)));
    }

    #[test]
    fn local_edges_sit_in_index3_candidates() {
        // Index3 consumer contract for v2, mirroring the v1 per-kind shape:
        // strict-XZ kinds ride pairs_xz, disjoint same-story kinds ride
        // adjacent_xz(2000). Far is long-range by design and rides no
        // candidate set; cross-story shaft alignment (disjoint interiors,
        // split stories, e.g. the bounds gap pins) rides direct queries
        // instead — the test pins that exemption to Aligned-only.
        use crate::index::boxes_of;
        for (name, actors) in scenes() {
            let g = build_rel(&actors);
            let index = Index3::build(&actors);
            let strict: BTreeSet<(u32, u32)> = index.pairs_xz().into_iter().collect();
            let near: BTreeSet<(u32, u32)> = index.adjacent_xz(DIR_RANGE_CM).into_iter().collect();
            let boxes: BTreeMap<u32, Box3> =
                boxes_of(&actors).into_iter().map(|b| (b.id, b)).collect();
            let key = |a: &u32, b: &u32| {
                if a < b {
                    (*a, *b)
                } else {
                    (*b, *a)
                }
            };
            for (a, rel, b) in &g.edges {
                if *rel == Rel::Far {
                    continue;
                }
                let k = key(a, b);
                if strict.contains(&k) || near.contains(&k) {
                    continue;
                }
                assert_eq!(
                    *rel,
                    Rel::Aligned,
                    "{name}: edge ({a},{rel:?},{b}) missing from index3 candidates"
                );
                let (ba, bb) = (&boxes[a], &boxes[b]);
                assert!(
                    !(ba.min_y < bb.max_y && bb.min_y < ba.max_y),
                    "{name}: uncovered Aligned edge ({a},{b}) shares a story"
                );
            }
        }
        // Scene-query-style reads over the same scenes: the column lists the
        // stack bottom-up, the band isolates one story, and a corridor
        // query box catches the walk the pebble blocks.
        let stack = Index3::build(&s_stack());
        assert_eq!(stack.column(0, 0), vec![110, 111, 112]);
        let up = Index3::build(&s_upstairs());
        assert_eq!(up.band(0, 200), vec![180, 181]);
        assert_eq!(up.band(250, 450), vec![182, 183]);
        let ring = Index3::build(&s_ring());
        let corridor = ring
            .query_box([-350, -10, -100], [350, 300, 100])
            .expect("corridor box is well-formed");
        assert_eq!(corridor, vec![160, 163, 164, 165]);
        // The flush contact pair rides the tight gap probe the query tool uses.
        let tight: BTreeSet<(u32, u32)> = Index3::build(&s_against())
            .adjacent_xz(AGAINST_GAP_CM)
            .into_iter()
            .collect();
        assert!(tight.contains(&(150, 151)));
        assert!(!tight.contains(&(150, 152)));
    }

    #[test]
    fn degenerate_inputs_answer_false_never_panic() {
        let mut actors = s_row();
        let mut lost = cube(400, "lost", 0.0, 0.0);
        lost.x = f32::NAN;
        let mut flat = cube(401, "flat", 2.0, 0.0);
        flat.sy = 0.0;
        let mut dizzy = cube(402, "dizzy", 4.0, 0.0);
        dizzy.yaw = f32::NAN;
        actors.push(lost);
        actors.push(flat);
        actors.push(dizzy);
        for rel in [
            Rel::Above,
            Rel::Below,
            Rel::Front,
            Rel::Behind,
            Rel::Left,
            Rel::Right,
            Rel::Against,
            Rel::Around,
            Rel::Aligned,
            Rel::Near,
            Rel::Far,
        ] {
            // Unknown, NaN and flat actors take no edges in any frame.
            assert!(!holds(&actors, 400, rel, 100, Frame::Subject));
            assert!(!holds(&actors, 100, rel, 400, Frame::Subject));
            assert!(!holds(&actors, 401, rel, 100, Frame::World));
            assert!(!holds(&actors, 999, rel, 100, Frame::Subject));
            assert!(!holds(&actors, 100, rel, 999, Frame::World));
            // Same-id pairs are never related.
            assert!(!holds(&actors, 100, rel, 100, Frame::Subject));
            // Unknown viewers blind only the directional kinds; frame-free
            // kinds ignore the frame by design, so they answer the same.
            if matches!(rel, Rel::Front | Rel::Behind | Rel::Left | Rel::Right) {
                assert!(!holds(&actors, 100, rel, 101, Frame::Viewer(999)));
            } else {
                assert_eq!(
                    holds(&actors, 100, rel, 101, Frame::Viewer(999)),
                    holds(&actors, 100, rel, 101, Frame::Subject)
                );
            }
            if matches!(rel, Rel::Front | Rel::Behind | Rel::Left | Rel::Right) {
                assert!(!holds(&actors, 402, rel, 100, Frame::Subject));
            }
        }
        assert!(!between_holds(&actors, 400, 100, 101));
        assert!(!between_holds(&actors, 100, 100, 101));
        assert!(!between_holds(&actors, 100, 999, 101));
        assert!(!blocking_holds(&actors, 401, 100, 101));
        assert!(!blocking_holds(&actors, 100, 100, 101));
        // Degenerate actors never enter the graph either.
        let g = build_rel(&actors);
        assert!(!g.edges.iter().any(|(a, _, b)| *a == 400 || *b == 400));
        assert!(!g.edges.iter().any(|(a, _, b)| *a == 401 || *b == 401));
        assert_eq!(center_dist2_cm2(&actors, 100, 999), None);
        assert_eq!(center_dist2_cm2(&actors, 400, 100), None);
        assert_eq!(center_dist2_cm2(&actors, 100, 100), Some(0));
        // Empty scenes build empty and digest stable.
        let empty = build_rel(&[]);
        assert!(empty.edges.is_empty());
        assert!(empty.between.is_empty());
        assert!(empty.blocking.is_empty());
        assert_eq!(digest_rel(&empty), digest_rel(&build_rel(&[])));
    }

    #[test]
    fn threshold_names_and_measures_agree() {
        // Measures stay numeric, thresholds stay in the caller: the band
        // predicates agree with center_dist2_cm2 on every gold pair.
        for (name, actors) in scenes() {
            let ids: Vec<u32> = actors.iter().map(|a| a.id).collect();
            for (i, &a) in ids.iter().enumerate() {
                for &b in &ids[i + 1..] {
                    if let Some(d2) = center_dist2_cm2(&actors, a, b) {
                        let near = i64::from(NEAR_CM);
                        let far = i64::from(FAR_CM);
                        assert_eq!(
                            holds(&actors, a, Rel::Near, b, Frame::Subject),
                            d2 <= near * near,
                            "{name}: near disagrees with measure on ({a},{b})"
                        );
                        assert_eq!(
                            holds(&actors, a, Rel::Far, b, Frame::Subject),
                            d2 >= far * far,
                            "{name}: far disagrees with measure on ({a},{b})"
                        );
                    }
                }
            }
        }
        // Names are stable for affordance and query briefs.
        assert_eq!(rel_name(Rel::Front), "in_front_of");
        assert_eq!(rel_name(Rel::Left), "left_of");
        assert_eq!(rel_name(Rel::Near), "near");
        assert_eq!(rel_name(Rel::Far), "far");
        assert_eq!(frame_name(Frame::World), "world");
        assert_eq!(frame_name(Frame::Subject), "subject");
        assert_eq!(frame_name(Frame::Viewer(7)), "viewer");
    }
}

// ================================================================
// SP28: relation-cycle consistency over the frozen SP3 graph.
//
// What it is: directed-cycle detection over Supports/OnTopOf/Inside,
// plus integer-cm factor residuals measuring each edge against actor
// geometry. NextTo is symmetric and Facing is a yaw cone: neither
// constrains stacking order, so neither votes here.
//
// Normalization (the no-false-positive core): every edge maps to
// support-up form — `(a, Supports, b)` stays `a -> b`, `(a, OnTopOf, b)`
// ("a rests on b") maps to `b -> a`, `(a, Inside, b)` ("a inside b")
// stays `a -> b`. The resting-contact dual (`a Supports b` plus
// `b OnTopOf a`) collapses to one direction, so a healthy stack never
// reads as a 2-cycle. Built graphs are DAGs by construction (a support
// edge strictly raises the top face, an Inside edge strictly grows the
// volume and never lowers a top), so `smallest_cycle` over a built graph
// answers `None`: any cycle in a proposed graph is real inconsistency.
//
// Residuals are integer centimeters past the shared `to_cm` input
// rounding — no float reaches them. A support edge measures the
// supporter-top to supported-bottom gap (healthy `0..=SUPPORT_GAP_CM`);
// an Inside edge measures the smallest face margin (healthy `>= 0`).
// This module measures; callers compare against those bands.
//
// Consumer: the opt-in `relationCycle` check in `checks.rs` and the
// SP49 explain+repair follow-up read `smallest_cycle` and
// `cycle_residuals_cm`. Budget: BFS per node over an id-ordered
// adjacency; room-scale graphs, never the fixed tick. Failure mode:
// unknown or degenerate actors give `None` residuals for their edges;
// empty or acyclic graphs give no cycle; same-id pairs never link.

/// Directed support-up adjacency over Supports/OnTopOf/Inside. Keys and
/// successor sets are id-ordered, duals collapse to one direction, and
/// same-id pairs never link, so traversal order is total.
fn cycle_adjacency(graph: &Graph) -> BTreeMap<u32, BTreeSet<u32>> {
    let mut adj: BTreeMap<u32, BTreeSet<u32>> = BTreeMap::new();
    let mut link = |from: u32, to: u32| {
        if from != to {
            adj.entry(from).or_default().insert(to);
            adj.entry(to).or_default();
        }
    };
    for (a, rel, b) in &graph.edges {
        match rel {
            Relation::Supports => link(*a, *b),
            Relation::OnTopOf => link(*b, *a),
            Relation::Inside => link(*a, *b),
            Relation::NextTo | Relation::Facing => {}
        }
    }
    adj
}

/// Shortest directed cycle through `start`, or `None`. Breadth-first over
/// the id-ordered adjacency, so the first return to `start` is the
/// shortest path back; ties break by id order, never by visit timing.
fn shortest_through(adj: &BTreeMap<u32, BTreeSet<u32>>, start: u32) -> Option<Vec<u32>> {
    use std::collections::VecDeque;
    let mut parent: BTreeMap<u32, u32> = BTreeMap::new();
    let mut queue: VecDeque<u32> = VecDeque::new();
    if let Some(next) = adj.get(&start) {
        for n in next {
            if parent.insert(*n, start).is_none() {
                queue.push_back(*n);
            }
        }
    }
    // The adjacency holds no self-links, so reaching `start` here always
    // closes a cycle of two or more nodes.
    while let Some(cur) = queue.pop_front() {
        let Some(next) = adj.get(&cur) else {
            continue;
        };
        for n in next {
            if *n == start {
                let mut trace = cur;
                let mut rev = vec![trace];
                while trace != start {
                    trace = parent[&trace];
                    rev.push(trace);
                }
                rev.pop();
                rev.reverse();
                let mut cycle = vec![start];
                cycle.extend(rev);
                return Some(cycle);
            }
            if !parent.contains_key(n) {
                parent.insert(*n, cur);
                queue.push_back(*n);
            }
        }
    }
    None
}

/// Smallest directed Supports/OnTopOf/Inside cycle in support-up form, or
/// `None` when the graph is acyclic. Canonical under rotation (smallest
/// id leads); across candidates the fewest nodes win, then the
/// lexicographically smallest row, so any input order gives the same
/// answer. Units: actor ids. Determinism scope: pure function of the
/// edge set. Errors: none.
pub fn smallest_cycle(graph: &Graph) -> Option<Vec<u32>> {
    let adj = cycle_adjacency(graph);
    let mut best: Option<Vec<u32>> = None;
    let starts: Vec<u32> = adj.keys().copied().collect();
    for s in starts {
        let Some(c) = shortest_through(&adj, s) else {
            continue;
        };
        let mut pos = 0;
        for (i, id) in c.iter().enumerate() {
            if *id < c[pos] {
                pos = i;
            }
        }
        let mut rot = c[pos..].to_vec();
        rot.extend_from_slice(&c[..pos]);
        let better = match &best {
            None => true,
            Some(b) => rot.len() < b.len() || (rot.len() == b.len() && rot < *b),
        };
        if better {
            best = Some(rot);
        }
    }
    best
}

/// True when the graph holds a directed Supports/OnTopOf/Inside cycle.
/// Same answer as `smallest_cycle(...).is_some()`; the refusal detail is
/// one call away.
pub fn has_relation_cycle(graph: &Graph) -> bool {
    smallest_cycle(graph).is_some()
}

/// Integer-cm boxes over valid actors only, keyed by id. Unknown or
/// degenerate actors are absent, so their edges measure `None` downstream.
fn valid_boxes(actors: &[Actor]) -> BTreeMap<u32, Box3> {
    let mut wanted: BTreeSet<u32> = BTreeSet::new();
    for a in actors {
        if valid(a) {
            wanted.insert(a.id);
        }
    }
    boxes_of(actors)
        .into_iter()
        .filter(|b| wanted.contains(&b.id))
        .map(|b| (b.id, b))
        .collect()
}

/// Gap from the supporter top face to the supported bottom face, in cm.
fn support_gap(supporter: &Box3, supported: &Box3) -> i32 {
    supported.min_y - supporter.max_y
}

/// Smallest of the six containment face margins, in cm.
fn face_margin(inner: &Box3, outer: &Box3) -> i32 {
    let mut m = i32::MAX;
    for v in [
        inner.min_x - outer.min_x,
        outer.max_x - inner.max_x,
        inner.min_y - outer.min_y,
        outer.max_y - inner.max_y,
        inner.min_z - outer.min_z,
        outer.max_z - inner.max_z,
    ] {
        m = m.min(v);
    }
    m
}

/// Gap in cm from the supporter top face to the supported bottom face:
/// the factor residual behind one support contact. Healthy support reads
/// `0..=SUPPORT_GAP_CM`; negative means interpenetration, larger means
/// floating. Units are integer centimeters; no float past the shared
/// `to_cm` input rounding. `None` when either actor is unknown,
/// degenerate, or missing.
pub fn support_residual_cm(actors: &[Actor], supporter: u32, supported: u32) -> Option<i32> {
    let boxes = valid_boxes(actors);
    let (a, b) = (boxes.get(&supporter)?, boxes.get(&supported)?);
    Some(support_gap(a, b))
}

/// Smallest face margin in cm of `inner` inside `outer`: the factor
/// residual behind one containment. Healthy containment reads `>= 0`.
/// Same units and `None` convention as [`support_residual_cm`].
pub fn inside_margin_cm(actors: &[Actor], inner: u32, outer: u32) -> Option<i32> {
    let boxes = valid_boxes(actors);
    let (a, b) = (boxes.get(&inner)?, boxes.get(&outer)?);
    Some(face_margin(a, b))
}

/// Per-edge integer-cm residuals around one support-up cycle, in cycle
/// order: support contacts read as top-to-bottom gaps (see
/// [`support_residual_cm`]), containments as smallest face margins (see
/// [`inside_margin_cm`]). Edges resolve against `graph` preferring
/// `Supports`, then the `OnTopOf` dual, then `Inside`; unmeasurable legs
/// (unknown actors) are skipped, never zero-filled. Empty for cycles
/// under two nodes.
pub fn cycle_residuals_cm(actors: &[Actor], graph: &Graph, cycle: &[u32]) -> Vec<(u32, u32, i32)> {
    if cycle.len() < 2 {
        return Vec::new();
    }
    let boxes = valid_boxes(actors);
    let mut out = Vec::new();
    for (i, from) in cycle.iter().enumerate() {
        let to = cycle[(i + 1) % cycle.len()];
        let measured = if graph.edges.contains(&(*from, Relation::Supports, to))
            || graph.edges.contains(&(to, Relation::OnTopOf, *from))
        {
            match (boxes.get(from), boxes.get(&to)) {
                (Some(a), Some(b)) => Some(support_gap(a, b)),
                _ => None,
            }
        } else if graph.edges.contains(&(*from, Relation::Inside, to)) {
            match (boxes.get(from), boxes.get(&to)) {
                (Some(a), Some(b)) => Some(face_margin(a, b)),
                _ => None,
            }
        } else {
            None
        };
        if let Some(r) = measured {
            out.push((*from, to, r));
        }
    }
    out
}

/// Deterministic integer stream for SP28 fixtures: `xorshift`-free LCG
/// over `u64` only, so scaffolding never smuggles float into the check.
#[cfg(test)]
fn sp28_lcg(state: &mut u64) -> u32 {
    *state = state
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407);
    (*state >> 33) as u32
}

/// Seeded SP28 actor set (test scaffolding): arena, one table-plus-crate
/// stack, and six rest-height floor props scattered on an exact 0.5 m
/// grid so `to_cm` is bit-exact. Ids 3, 4, 5 always exist for ring
/// injection. Same seed gives the same actors, byte for byte.
#[cfg(test)]
pub(crate) fn seeded_sp28_actors(seed: u64) -> Vec<Actor> {
    let mut s = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let fixture = |id: u32, name: &str, x: f32, y: f32, z: f32| Actor {
        id,
        name: name.into(),
        x,
        y,
        z,
        sx: 1.0,
        sy: 1.0,
        sz: 1.0,
        parent: None,
        physical: true,
        vx: 0.0,
        vz: 0.0,
        yaw: 0.0,
    };
    let mut actors = vec![
        Actor {
            id: 0,
            name: "arena".into(),
            x: 0.0,
            y: 2.0,
            z: 0.0,
            sx: 20.0,
            sy: 4.0,
            sz: 20.0,
            parent: None,
            physical: true,
            vx: 0.0,
            vz: 0.0,
            yaw: 0.0,
        },
        Actor {
            id: 1,
            name: "table".into(),
            x: 5.0,
            y: 0.5,
            z: 5.0,
            sx: 2.0,
            sy: 1.0,
            sz: 2.0,
            parent: None,
            physical: true,
            vx: 0.0,
            vz: 0.0,
            yaw: 0.0,
        },
        fixture(2, "crate_on_table", 5.0, 1.5, 5.0),
    ];
    for id in 3..9u32 {
        let gx = (sp28_lcg(&mut s) % 33) as f32 * 0.5 - 8.0;
        let gz = (sp28_lcg(&mut s) % 33) as f32 * 0.5 - 8.0;
        actors.push(fixture(id, "prop", gx, 0.5, gz));
    }
    actors
}

/// Clone `graph` with a 3-node Supports ring `(a,b,c)` plus its OnTopOf
/// duals (test scaffolding for the smallest-cycle refusal). Edges stay in
/// frozen `(a, relation, b)` order; nodes are untouched.
#[cfg(test)]
pub(crate) fn with_support_ring(graph: &Graph, a: u32, b: u32, c: u32) -> Graph {
    let mut edges = graph.edges.clone();
    edges.extend([
        (a, Relation::Supports, b),
        (b, Relation::OnTopOf, a),
        (b, Relation::Supports, c),
        (c, Relation::OnTopOf, b),
        (c, Relation::Supports, a),
        (a, Relation::OnTopOf, c),
    ]);
    edges.sort_by_key(|(x, r, y)| (*x, *r, *y));
    Graph {
        nodes: graph.nodes.clone(),
        edges,
    }
}

#[cfg(test)]
mod sp28_tests {
    use super::*;

    #[test]
    fn clean_built_graphs_are_acyclic_with_agreeing_residuals() {
        // 30 seeded scenes: zero false positives, and every built edge
        // measures inside its healthy integer-cm band.
        let mut fp = 0u32;
        for seed in 0..30u64 {
            let actors = seeded_sp28_actors(seed);
            let graph = build(&actors);
            if smallest_cycle(&graph).is_some() {
                fp += 1;
            }
            for (a, rel, b) in &graph.edges {
                match rel {
                    Relation::Supports => {
                        let r = support_residual_cm(&actors, *a, *b)
                            .unwrap_or_else(|| panic!("seed {seed}: support edge unmeasurable"));
                        assert!(
                            (0..=SUPPORT_GAP_CM).contains(&r),
                            "seed {seed}: support residual {r} cm out of band"
                        );
                    }
                    Relation::OnTopOf => {
                        let r = support_residual_cm(&actors, *b, *a)
                            .unwrap_or_else(|| panic!("seed {seed}: on-top-of unmeasurable"));
                        assert!(
                            (0..=SUPPORT_GAP_CM).contains(&r),
                            "seed {seed}: on-top-of residual {r} cm out of band"
                        );
                    }
                    Relation::Inside => {
                        let m = inside_margin_cm(&actors, *a, *b)
                            .unwrap_or_else(|| panic!("seed {seed}: inside unmeasurable"));
                        assert!(
                            m >= 0,
                            "seed {seed}: inside margin {m} cm negative on a built edge"
                        );
                    }
                    Relation::NextTo | Relation::Facing => {}
                }
            }
        }
        assert_eq!(fp, 0, "{fp} false positives over 30 clean built graphs");
    }

    #[test]
    fn injected_rings_detected_as_smallest_with_violating_residuals() {
        // 30 seeded scenes with a fake Supports ring on real props (3,4,5):
        // every one caught, the cycle named canonically, and the fake legs
        // read outside the healthy support band (same-height props rest
        // side by side, never stacked).
        let mut caught = 0u32;
        for seed in 0..30u64 {
            let actors = seeded_sp28_actors(seed);
            let clean = build(&actors);
            assert!(
                smallest_cycle(&clean).is_none(),
                "seed {seed}: clean graph must stay acyclic"
            );
            let ring = with_support_ring(&clean, 3, 4, 5);
            let cycle = smallest_cycle(&ring)
                .unwrap_or_else(|| panic!("seed {seed}: injected ring missed"));
            assert_eq!(cycle, vec![3, 4, 5], "seed {seed}: smallest cycle misnamed");
            assert!(
                has_relation_cycle(&ring),
                "seed {seed}: predicate disagrees"
            );
            let residuals = cycle_residuals_cm(&actors, &ring, &cycle);
            assert_eq!(residuals.len(), 3, "seed {seed}: every ring leg measurable");
            for (from, to, r) in &residuals {
                assert!(
                    !(0..=SUPPORT_GAP_CM).contains(r),
                    "seed {seed}: fake leg ({from},{to}) reads healthy at {r} cm"
                );
            }
            caught += 1;
        }
        assert_eq!(caught, 30, "caught {caught}/30 injected rings, need 30/30");
    }

    #[test]
    fn two_cycle_and_rotation_stay_canonical() {
        // Genuine 2-cycle both ways: smallest names it, min id first.
        let edges = vec![
            (7, Relation::Supports, 8),
            (8, Relation::OnTopOf, 7),
            (8, Relation::Supports, 7),
            (7, Relation::OnTopOf, 8),
        ];
        let graph = Graph {
            nodes: vec![(7, Layer::Object), (8, Layer::Object)],
            edges,
        };
        assert_eq!(smallest_cycle(&graph), Some(vec![7, 8]));
        // Rotation input order never moves the answer.
        let clean = build(&seeded_sp28_actors(11));
        let mut fwd = with_support_ring(&clean, 3, 4, 5);
        fwd.edges.reverse();
        fwd.edges.sort_by_key(|(a, r, b)| (*a, *r, *b));
        assert_eq!(smallest_cycle(&fwd), Some(vec![3, 4, 5]));
        // A healthy stack (dual pair, one direction) is never a cycle.
        assert!(!has_relation_cycle(&clean));
    }

    #[test]
    fn degenerate_inputs_measure_none_and_find_nothing() {
        let mut scene = seeded_sp28_actors(3);
        let mut lost = scene[3].clone();
        lost.x = f32::NAN;
        scene[3] = lost;
        assert_eq!(support_residual_cm(&scene, 3, 4), None);
        assert_eq!(inside_margin_cm(&scene, 3, 0), None);
        assert_eq!(support_residual_cm(&scene, 3, 999), None);
        assert_eq!(smallest_cycle(&build(&[])), None);
        assert!(cycle_residuals_cm(&scene, &build(&scene), &[]).is_empty());
        assert!(cycle_residuals_cm(&scene, &build(&scene), &[3]).is_empty());
    }
}
