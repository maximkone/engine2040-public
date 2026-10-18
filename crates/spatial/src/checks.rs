//! Table-driven checks. Frozen order; thresholds in integer cm.

use super::index::{
    build_rects, density_grid, overlap_pct, pos_known, size_known, sweep_pairs, to_cm, CmRect,
};
use super::{navcheck, Actor, Finding, Report};
use engine_core::Fnv64;
use std::collections::{BTreeMap, BTreeSet};

/// Every check name this crate can run.
pub const SPATIAL_CHECKS: [&str; 18] = [
    "overlap",
    "unknown-position",
    "floating",
    "bounds",
    "sunken",
    "scale",
    "orphan",
    "giantObject",
    "support",
    "dupeSpam",
    "oob99",
    "sweep",
    "overlapDensity",
    "grammar",
    "navConnect",
    "flowPath",
    "accessFront",
    "doorClear",
];

/// Phase-2 checks need a bake view (`run_spatial` + `Some(NavView)`).
pub const NAV_CHECKS: [&str; 4] = ["navConnect", "flowPath", "accessFront", "doorClear"];

/// Opt-in SP28 checks: legal in `run_spatial` but deliberately absent
/// from the frozen `SPATIAL_CHECKS` catalog, whose length pins the
/// repair seeded-count test and the endpoint legal-list goldens. The
/// whole-catalog `run()` therefore never asks for them, so existing
/// goldens stay byte-identical whether or not a cycle exists.
pub const OPT_IN_CHECKS: [&str; 3] = ["relationCycle", "seatedJoint", "supportChain"];

const WORLD_BOUND_CM: i32 = 10_000; // +-100 m, red
const OOB99_CM: i32 = 9_900; // +-99 m, yellow
const FLOAT_EPS_CM: i32 = 1; // min_y above this = floating
const SUPPORT_GAP_CM: i32 = 5; // supporter top within this below min_y
const DUPE_MAX: usize = 10; // same-name headcount limit
const SWEEP_HORIZON_TICKS: i32 = 60; // 1 s at fixed 60 Hz
const DENSITY_CELL_CM: i32 = 100; // 1 m crowd cells
const DENSITY_MAX: usize = 8; // headcount per cell
const NAME_MAX_LEN: usize = 64; // grammar: [a-z0-9_], no leading digit

/// Severity word for a check name (`red`, `yellow` or the fallback).
pub fn severity_of(check: &str) -> &'static str {
    match check {
        "overlap" | "bounds" | "scale" | "dupeSpam" => "red",
        _ => "yellow",
    }
}

fn check_index(check: &str) -> usize {
    SPATIAL_CHECKS
        .iter()
        .position(|c| *c == check)
        .unwrap_or(usize::MAX)
}

/// FNV-1a 64 over frozen-order findings (check name, both ids, percent
/// bits): the report hash, identical for the same findings on every platform.
pub(crate) fn hash_findings(findings: &[Finding]) -> u64 {
    let mut h = Fnv64::new();
    for f in findings {
        h.write_str(f.check);
        h.write_u32(f.a);
        h.write_u32(f.b);
        h.write_f32(f.pct);
    }
    h.finish()
}

/// Run the named checks over `actors`; unknown names come back as the error.
pub fn run_checks(actors: &[Actor], checks: &[&str]) -> Result<Report, Vec<&'static str>> {
    run_spatial(actors, checks, None)
}

/// Full run with optional nav bake view. Nav checks requested without a view
/// refuse loud (`needs-nav-view`); the offline `run()` never asks for them.
pub fn run_spatial(
    actors: &[Actor],
    checks: &[&str],
    nav: Option<&navcheck::NavView>,
) -> Result<Report, Vec<&'static str>> {
    let wanted: BTreeSet<&str> = checks.iter().copied().collect();
    for w in &wanted {
        if !SPATIAL_CHECKS.contains(w) && !OPT_IN_CHECKS.contains(w) {
            return Err(SPATIAL_CHECKS.to_vec());
        }
    }
    if nav.is_none() && wanted.iter().any(|w| NAV_CHECKS.contains(w)) {
        return Err(vec!["needs-nav-view"]);
    }
    let ids: BTreeMap<u32, &Actor> = actors.iter().map(|a| (a.id, a)).collect();
    let mut findings: Vec<Finding> = Vec::new();
    let mut push = |check: &'static str, a: u32, b: u32, pct: f32| {
        findings.push(Finding {
            check,
            a,
            b,
            pct,
            severity: severity_of(check),
        });
    };

    // unknown-position first: NaN actors skip every other check.
    let mut bad: BTreeSet<u32> = BTreeSet::new();
    if wanted.contains("unknown-position") {
        for a in actors {
            if !pos_known(a) {
                bad.insert(a.id);
                push("unknown-position", a.id, u32::MAX, 0.0);
            }
        }
    } else {
        for a in actors {
            if !pos_known(a) {
                bad.insert(a.id);
            }
        }
    }
    let good: Vec<Actor> = actors
        .iter()
        .filter(|a| !bad.contains(&a.id))
        .cloned()
        .collect();

    // overlap via the index3 sweep broadphase (sweep-then-exact): candidates
    // come from `sweep_xz` over `live_boxes` in one pass, exact intersection and percent from
    // the rect map. Rows the index drops (zero/negative size) ride a small
    // exact fallback, so the union is identical to the legacy pair set.
    if wanted.contains("overlap") {
        let rects = build_rects(&good);
        let area_of: BTreeMap<u32, i64> = rects.iter().map(|r| (r.id, r.area)).collect();
        let rect_of: BTreeMap<u32, &CmRect> = rects.iter().map(|r| (r.id, r)).collect();
        for (lo, hi) in super::index3::sweep_xz(&super::index3::live_boxes(&good)) {
            let (a, b) = (rect_of[&lo], rect_of[&hi]);
            let ix = i64::from(a.max_x.min(b.max_x)) - i64::from(a.min_x.max(b.min_x));
            let iz = i64::from(a.max_z.min(b.max_z)) - i64::from(a.min_z.max(b.min_z));
            if ix <= 0 || iz <= 0 {
                continue;
            }
            let pct = overlap_pct(ix * iz, area_of[&lo], area_of[&hi]);
            push("overlap", lo, hi, pct);
        }
        // Exact fallback for index-degenerate rects only: live-live pairs
        // are fully covered above, so test each degenerate rect against the
        // higher-id rects directly (degenerate counts stay tiny; findings
        // sort into frozen order below like every other check).
        let live_of: BTreeMap<u32, bool> = good
            .iter()
            .map(|a| (a.id, a.sx > 0.0 && a.sy > 0.0 && a.sz > 0.0))
            .collect();
        let mut degen: Vec<&CmRect> = rects
            .iter()
            .filter(|r| !live_of.get(&r.id).copied().unwrap_or(false))
            .collect();
        degen.sort_by_key(|r| r.id);
        for d in &degen {
            for o in rects.iter().filter(|o| o.id != d.id) {
                // Degenerate-degenerate pairs emit once from the lower id;
                // the live side of a degenerate-live pair never emits, so
                // every other combination emits here.
                if o.id < d.id && !live_of.get(&o.id).copied().unwrap_or(false) {
                    continue;
                }
                let ix = i64::from(d.max_x.min(o.max_x)) - i64::from(d.min_x.max(o.min_x));
                if ix <= 0 {
                    continue;
                }
                let iz = i64::from(d.max_z.min(o.max_z)) - i64::from(d.min_z.max(o.min_z));
                if iz <= 0 {
                    continue;
                }
                let (lo, hi) = if d.id < o.id {
                    (d.id, o.id)
                } else {
                    (o.id, d.id)
                };
                let pct = overlap_pct(ix * iz, area_of[&lo], area_of[&hi]);
                push("overlap", lo, hi, pct);
            }
        }
    }

    let giants = if wanted.contains("giantObject") {
        giant_ids(&good)
    } else {
        BTreeSet::new()
    };

    // Per-actor fused pass: one loop, many checks.
    for a in &good {
        let cx = to_cm(a.x);
        let cy = to_cm(a.y);
        let cz = to_cm(a.z);
        if wanted.contains("bounds")
            && (cx.abs() > WORLD_BOUND_CM || cy.abs() > WORLD_BOUND_CM || cz.abs() > WORLD_BOUND_CM)
        {
            push("bounds", a.id, u32::MAX, 0.0);
        }
        if wanted.contains("oob99")
            && (cx.abs() > OOB99_CM || cy.abs() > OOB99_CM || cz.abs() > OOB99_CM)
        {
            push("oob99", a.id, u32::MAX, 0.0);
        }
        if wanted.contains("scale") {
            let bad_size = !size_known(a)
                || a.sx <= 0.0
                || a.sy <= 0.0
                || a.sz <= 0.0
                || to_cm(a.sx).abs() > WORLD_BOUND_CM
                || to_cm(a.sy).abs() > WORLD_BOUND_CM
                || to_cm(a.sz).abs() > WORLD_BOUND_CM;
            if bad_size {
                push("scale", a.id, u32::MAX, 0.0);
            }
        }
        if wanted.contains("orphan") {
            if let Some(p) = a.parent {
                if !ids.contains_key(&p) {
                    push("orphan", a.id, u32::MAX, 0.0);
                }
            }
        }
        // Vertical checks need sane size.
        if size_known(a) && a.sy > 0.0 {
            let min_y = to_cm(a.y) - to_cm(a.sy).abs() / 2;
            let max_y = to_cm(a.y) + to_cm(a.sy).abs() / 2;
            if wanted.contains("floating") && min_y > FLOAT_EPS_CM {
                push("floating", a.id, u32::MAX, 0.0);
            }
            if wanted.contains("sunken") && max_y < -FLOAT_EPS_CM {
                push("sunken", a.id, u32::MAX, 0.0);
            }
            if wanted.contains("support") && a.physical && min_y > FLOAT_EPS_CM {
                let mut held = false;
                for b in &good {
                    if b.id == a.id || !pos_known(b) || !size_known(b) || b.sy <= 0.0 {
                        continue;
                    }
                    let b_top = to_cm(b.y) + to_cm(b.sy).abs() / 2;
                    let gap = min_y - b_top;
                    if (0..=SUPPORT_GAP_CM).contains(&gap) {
                        // XZ overlap?
                        let ax0 = cx - to_cm(a.sx).abs() / 2;
                        let ax1 = cx + to_cm(a.sx).abs() / 2;
                        let az0 = cz - to_cm(a.sz).abs() / 2;
                        let az1 = cz + to_cm(a.sz).abs() / 2;
                        let bx = to_cm(b.x);
                        let bz = to_cm(b.z);
                        let bx0 = bx - to_cm(b.sx).abs() / 2;
                        let bx1 = bx + to_cm(b.sx).abs() / 2;
                        let bz0 = bz - to_cm(b.sz).abs() / 2;
                        let bz1 = bz + to_cm(b.sz).abs() / 2;
                        if ax0 < bx1 && bx0 < ax1 && az0 < bz1 && bz0 < az1 {
                            held = true;
                            break;
                        }
                    }
                }
                if !held {
                    push("support", a.id, u32::MAX, 0.0);
                }
            }
        }
        if giants.contains(&a.id) {
            push("giantObject", a.id, u32::MAX, 0.0);
        }
    }

    // dupeSpam: name headcount over limit, sorted for determinism.
    if wanted.contains("dupeSpam") {
        let mut groups: BTreeMap<&str, Vec<u32>> = BTreeMap::new();
        for a in &good {
            groups.entry(a.name.as_str()).or_default().push(a.id);
        }
        for ids_in in groups.values() {
            if ids_in.len() > DUPE_MAX {
                let mut sorted = ids_in.clone();
                sorted.sort_unstable();
                let first = sorted[0];
                for id in sorted {
                    push("dupeSpam", id, first, 0.0);
                }
            }
        }
    }

    // sweep: swept rects collide where static rects do not (movers only).
    if wanted.contains("sweep") {
        let rects = build_rects(&good);
        let area_of: BTreeMap<u32, i64> = rects.iter().map(|r| (r.id, r.area)).collect();
        for (lo, hi, inter) in sweep_pairs(&good, SWEEP_HORIZON_TICKS) {
            let pct = overlap_pct(
                inter,
                area_of.get(&lo).copied().unwrap_or(1),
                area_of.get(&hi).copied().unwrap_or(1),
            );
            push("sweep", lo, hi, pct);
        }
    }

    // overlapDensity: crowd-cell headcount over limit, one finding per cell.
    if wanted.contains("overlapDensity") {
        for ids in density_grid(&good, DENSITY_CELL_CM).values() {
            if ids.len() > DENSITY_MAX {
                push("overlapDensity", ids[0], u32::MAX, ids.len() as f32);
            }
        }
    }

    // grammar: baked naming rule — [a-z0-9_], 1..=64 chars, no leading digit.
    if wanted.contains("grammar") {
        for a in &good {
            let ok = !a.name.is_empty()
                && a.name.len() <= NAME_MAX_LEN
                && a.name
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
                && !a.name.as_bytes()[0].is_ascii_digit();
            if !ok {
                push("grammar", a.id, u32::MAX, 0.0);
            }
        }
    }

    // relationCycle (SP28, opt-in): smallest directed Supports/OnTopOf/Inside
    // cycle over the freshly built graph. Built graphs are DAGs by
    // construction (support strictly raises the top face, Inside strictly
    // grows the volume), so a consistent scene stays silent here: any
    // finding names a real inconsistency. One finding per report, naming
    // the cycle's first edge in canonical rotation; the full cycle is one
    // `smallest_cycle` call away for SP49 repair.
    if wanted.contains("relationCycle") {
        let graph = super::relation::build(actors);
        if let Some(finding) = relation_cycle_finding(&graph) {
            findings.push(finding);
        }
    }

    // SPX2 seatedJoint / supportChain (opt-in): seats, embeds and floating bases.
    if wanted.contains("seatedJoint") || wanted.contains("supportChain") {
        findings.extend(
            super::support::support_findings(&good)
                .into_iter()
                .filter(|f| wanted.contains(f.check)),
        );
    }

    // Phase-2 nav checks (bake view only).
    if let Some(nv) = nav {
        findings.extend(navcheck::run_nav(actors, nv, &wanted));
    }

    findings.sort_by_key(|f| (check_index(f.check), f.a, f.b));
    let hash = hash_findings(&findings);
    Ok(Report { findings, hash })
}

/// Ground plate: a flat slab (height at most 50 cm, both sides at most 50
/// m). Plates are the floor of the layout, never a giant and never counted
/// when measuring the layout they carry.
const PLATE_H_CM: i32 = 50;
const PLATE_SIDE_CM: i32 = 5_000;

/// Solid boxes of the layout as `(id, min, max, largest side)` in cm: known,
/// positive-size, physical actors that are not ground plates, id ascending.
fn solid_extents(actors: &[Actor]) -> Vec<(u32, [i32; 3], [i32; 3], i32)> {
    let mut out = Vec::new();
    for a in actors {
        if !(a.physical && pos_known(a) && size_known(a) && a.sx > 0.0 && a.sy > 0.0 && a.sz > 0.0)
        {
            continue;
        }
        let size = [to_cm(a.sx).abs(), to_cm(a.sy).abs(), to_cm(a.sz).abs()];
        if size[1] <= PLATE_H_CM && size[0] <= PLATE_SIDE_CM && size[2] <= PLATE_SIDE_CM {
            continue;
        }
        let c = [to_cm(a.x), to_cm(a.y), to_cm(a.z)];
        let lo = [c[0] - size[0] / 2, c[1] - size[1] / 2, c[2] - size[2] / 2];
        let hi = [c[0] + size[0] / 2, c[1] + size[1] / 2, c[2] + size[2] / 2];
        out.push((a.id, lo, hi, size[0].max(size[1]).max(size[2])));
    }
    out.sort_by_key(|e| e.0);
    out
}

/// Corner pair of a box in cm, or the union of several.
type Bounds = Option<([i32; 3], [i32; 3])>;

fn union(a: Bounds, lo: [i32; 3], hi: [i32; 3]) -> Bounds {
    Some(match a {
        None => (lo, hi),
        Some((l, h)) => (
            [l[0].min(lo[0]), l[1].min(lo[1]), l[2].min(lo[2])],
            [h[0].max(hi[0]), h[1].max(hi[1]), h[2].max(hi[2])],
        ),
    })
}

/// Largest side of a bounds box.
fn span_of(b: Bounds) -> Option<i32> {
    b.map(|(l, h)| (h[0] - l[0]).max(h[1] - l[1]).max(h[2] - l[2]))
}

/// Per solid, the bounds of every other solid (prefix and suffix unions, so a
/// whole layout costs one pass).
fn rest_bounds(solids: &[(u32, [i32; 3], [i32; 3], i32)]) -> Vec<Bounds> {
    let mut before: Vec<Bounds> = Vec::with_capacity(solids.len() + 1);
    before.push(None);
    for s in solids {
        let last = before.last().copied().flatten();
        before.push(union(last, s.1, s.2));
    }
    let mut after: Bounds = None;
    let mut out: Vec<Bounds> = vec![None; solids.len()];
    for i in (0..solids.len()).rev() {
        let mut rest = before[i];
        if let Some((lo, hi)) = after {
            rest = union(rest, lo, hi);
        }
        out[i] = rest;
        after = union(after, solids[i].1, solids[i].2);
    }
    out
}

/// SPF5 giantObject: a solid is giant when its largest side exceeds the span
/// of every other solid, so a wall that is the edge of the layout is never
/// giant while a 50 m crate beside a 4 m room is. A shell that encloses the
/// rest of the layout in plan snugly (the rest spans at least a quarter of it) is the
/// edge too, so an arena box around its props stays quiet. Ground plates and
/// non-physical boxes neither count nor flag; fewer than two solids stay silent.
fn giant_ids(actors: &[Actor]) -> BTreeSet<u32> {
    let solids = solid_extents(actors);
    if solids.len() < 2 {
        return BTreeSet::new();
    }
    let rests = rest_bounds(&solids);
    solids
        .iter()
        .zip(rests)
        .filter(|(s, rest)| {
            let Some(span) = span_of(*rest) else {
                return false;
            };
            let encloses = rest.is_some_and(|(lo, hi)| {
                [0, 2]
                    .into_iter()
                    .all(|k| s.1[k] <= lo[k] && hi[k] <= s.2[k])
            });
            s.3 > span && !(encloses && span * 4 >= s.3)
        })
        .map(|(s, _)| s.0)
        .collect()
}

/// Span in cm of the layout without actor `id` (largest side of the box
/// around every other solid), or `None` when no other solid exists. Repair
/// reads it to size a giant back down to the layout it lives in.
pub(crate) fn layout_span_without(actors: &[Actor], id: u32) -> Option<i32> {
    let mut bounds: Bounds = None;
    for s in solid_extents(actors).iter().filter(|s| s.0 != id) {
        bounds = union(bounds, s.1, s.2);
    }
    span_of(bounds)
}

/// Opt-in SP28 refusal over an explicit relation graph: the smallest
/// directed Supports/OnTopOf/Inside cycle as one finding, or `None` when
/// the graph is acyclic. The finding names the cycle's first edge in
/// canonical rotation (`a` is the cycle minimum); `pct` carries no float
/// math (literal zero) and per-edge integer-cm evidence comes from
/// [`super::relation::cycle_residuals_cm`]. Consumer: the `relationCycle`
/// arm above (built graph) and SP49 repair (proposed graphs). Failure
/// mode: acyclic graphs answer `None`, never an error.
pub fn relation_cycle_finding(graph: &super::relation::Graph) -> Option<Finding> {
    let cycle = super::relation::smallest_cycle(graph)?;
    if cycle.len() < 2 {
        return None;
    }
    Some(Finding {
        check: "relationCycle",
        a: cycle[0],
        b: cycle[1],
        pct: 0.0,
        severity: severity_of("relationCycle"),
    })
}

#[cfg(test)]
mod tests {
    use super::super::index::{build_rects, overlap_pairs, overlap_pct};
    use super::super::Actor;
    use super::*;
    use std::collections::BTreeMap;

    fn resting(id: u32, name: &str, x: f32, z: f32) -> Actor {
        Actor {
            id,
            name: name.into(),
            x,
            y: 0.5,
            z,
            sx: 1.0,
            sy: 1.0,
            sz: 1.0,
            parent: None,
            physical: true,
            vx: 0.0,
            vz: 0.0,
            yaw: 0.0,
        }
    }

    /// Legacy oracle: overlap findings straight from the XZ rect sweep.
    fn legacy_overlap(actors: &[Actor]) -> Vec<(u32, u32, f32)> {
        let known: Vec<Actor> = actors.iter().filter(|a| pos_known(a)).cloned().collect();
        let rects = build_rects(&known);
        let area_of: BTreeMap<u32, i64> = rects.iter().map(|r| (r.id, r.area)).collect();
        let mut out: Vec<(u32, u32, f32)> = overlap_pairs(&rects)
            .into_iter()
            .map(|(lo, hi, inter)| (lo, hi, overlap_pct(inter, area_of[&lo], area_of[&hi])))
            .collect();
        out.sort_by_key(|a: &(u32, u32, f32)| (a.0, a.1));
        out
    }

    fn reported_overlap(actors: &[Actor]) -> Vec<(u32, u32, f32)> {
        let report = run_spatial(actors, &["overlap"], None).expect("overlap is legal");
        let mut out: Vec<(u32, u32, f32)> =
            report.findings.iter().map(|f| (f.a, f.b, f.pct)).collect();
        out.sort_by_key(|a: &(u32, u32, f32)| (a.0, a.1));
        out
    }

    #[test]
    fn overlap_reroute_matches_legacy_on_flat_scene() {
        let scene = vec![
            resting(0, "a", 0.0, 0.0),
            resting(1, "b", 0.5, 0.0),
            resting(2, "c", 5.0, 5.0),
            resting(3, "d", 5.4, 5.0),
            resting(4, "e", 40.0, 40.0),
        ];
        assert_eq!(reported_overlap(&scene), legacy_overlap(&scene));
        assert_eq!(reported_overlap(&scene).len(), 2);
    }

    #[test]
    fn overlap_reroute_matches_legacy_with_degenerate_rows() {
        // Flat, negative-size and NaN rows: the index drops them, the exact
        // fallback must keep the legacy pair set bit-identical.
        let mut flat = resting(10, "flat", 0.2, 0.0);
        flat.sy = 0.0;
        let mut neg = resting(11, "neg", 0.3, 0.0);
        neg.sx = -1.0;
        let mut lost = resting(12, "lost", 0.4, 0.0);
        lost.x = f32::NAN;
        let scene = vec![resting(0, "a", 0.0, 0.0), flat, neg, lost];
        assert_eq!(reported_overlap(&scene), legacy_overlap(&scene));
        // The flat and negative rows still pair in XZ like the legacy sweep.
        let pairs: Vec<(u32, u32)> = reported_overlap(&scene)
            .iter()
            .map(|(a, b, _)| (*a, *b))
            .collect();
        assert!(pairs.contains(&(0, 10)));
        assert!(pairs.contains(&(0, 11)));
        // NaN positions never pair anywhere.
        assert!(!reported_overlap(&scene)
            .iter()
            .any(|(a, b, _)| *a == 12 || *b == 12));
    }

    #[test]
    fn overlap_reroute_is_input_order_invariant() {
        let scene = vec![
            resting(0, "a", 0.0, 0.0),
            resting(1, "b", 0.5, 0.0),
            resting(2, "c", 5.0, 5.0),
            resting(3, "d", 5.4, 5.0),
        ];
        let mut rev = scene.clone();
        rev.reverse();
        assert_eq!(reported_overlap(&scene), reported_overlap(&rev));
        let r1 = run_spatial(&scene, &["overlap"], None).expect("legal");
        let r2 = run_spatial(&rev, &["overlap"], None).expect("legal");
        assert_eq!(r1.hash, r2.hash);
    }
}

#[cfg(test)]
mod sp28_tests {
    use super::super::relation::{build, seeded_sp28_actors, smallest_cycle, with_support_ring};
    use super::*;

    #[test]
    fn opt_in_silent_on_clean_loud_on_explicit_ring() {
        // 30 seeded scenes: the opt-in arm stays silent on built graphs
        // (0% FP by construction), while the explicit-graph refusal names
        // all 30 injected rings (>= 90% caught).
        let mut fp = 0u32;
        let mut caught = 0u32;
        for seed in 0..30u64 {
            let actors = seeded_sp28_actors(seed);
            let quiet = run_spatial(&actors, &["relationCycle"], None).expect("opt-in is legal");
            if quiet.count("relationCycle") != 0 {
                fp += 1;
            }
            let ring = with_support_ring(&build(&actors), 3, 4, 5);
            assert_eq!(
                smallest_cycle(&ring),
                Some(vec![3, 4, 5]),
                "seed {seed}: ring missed"
            );
            let finding = relation_cycle_finding(&ring)
                .unwrap_or_else(|| panic!("seed {seed}: refusal missing"));
            assert_eq!(finding.check, "relationCycle");
            assert_eq!((finding.a, finding.b), (3, 4));
            assert_eq!(finding.severity, "yellow");
            caught += 1;
        }
        assert_eq!(fp, 0, "{fp} false positives over 30 clean scenes");
        assert_eq!(caught, 30, "caught {caught}/30 injected rings, need 30/30");
    }

    #[test]
    fn catalog_stays_frozen_and_golden_untouched() {
        // The frozen catalog keeps its length (repair seeded-count and
        // endpoint legal-list goldens pin it); the whole-catalog run on a
        // consistent scene carries no cycle finding.
        assert_eq!(SPATIAL_CHECKS.len(), 18);
        assert_eq!(
            OPT_IN_CHECKS,
            ["relationCycle", "seatedJoint", "supportChain"]
        );
        let actors = seeded_sp28_actors(7);
        let full = super::super::run(&actors);
        assert_eq!(full.count("relationCycle"), 0);
        let e = run_spatial(&actors, &["nope"], None).unwrap_err();
        assert!(e.contains(&"overlap"));
    }

    #[expect(
        clippy::disallowed_types,
        clippy::disallowed_methods,
        reason = "brief p95 evidence measures wall time in-test only; no clock reaches the tick or any hash"
    )]
    #[test]
    fn receipt_brief_p95_and_bytes_x2() {
        // Acceptance numbers: brief p95 over the 60 SP28 scenes plus a
        // byte-identical re-render (bytes x2).
        use super::super::brief::{brief, brief_text};
        use std::time::Instant;
        let mut micros: Vec<u128> = Vec::new();
        let mut bytes = 0usize;
        for seed in 0..60u64 {
            let actors = seeded_sp28_actors(seed);
            let graph = build(&actors);
            let report = super::super::run(&actors);
            let t0 = Instant::now();
            let kept = brief(&actors, &graph, &report, 4096);
            let text = brief_text(&kept);
            micros.push(t0.elapsed().as_micros());
            let again = brief_text(&brief(&actors, &graph, &report, 4096));
            assert_eq!(text, again, "seed {seed}: brief bytes drifted");
            bytes = text.len();
        }
        micros.sort_unstable();
        let p95 = micros[(micros.len() * 95 / 100).min(micros.len() - 1)];
        println!("SP28 receipt: brief_p95_us={p95} bytes={bytes} scenes=60");
        assert!(p95 <= 5_000, "brief p95 {p95} us over 5 ms on SP28 scenes");
    }

    /// Seeded 60-graph corpus outcome over the frozen graph: `(caught, fp)`.
    /// Clean built graphs stay silent through the opt-in arm (0% FP);
    /// injected `Supports` rings are caught with the smallest cycle named
    /// and integer-cm residuals only. Test scaffolding, never the tick.
    fn cycle_corpus_counts() -> (u32, u32) {
        let mut caught = 0u32;
        let mut fp = 0u32;
        for seed in 0..60u64 {
            let actors = seeded_sp28_actors(seed);
            let clean = build(&actors);
            let quiet = run_spatial(&actors, &["relationCycle"], None).expect("opt-in is legal");
            if quiet.count("relationCycle") != 0 || smallest_cycle(&clean).is_some() {
                fp += 1;
            }
            let ring = with_support_ring(&clean, 3, 4, 5);
            let cycle = smallest_cycle(&ring)
                .unwrap_or_else(|| panic!("seed {seed}: injected ring missed"));
            assert_eq!(cycle, vec![3, 4, 5], "seed {seed}: smallest cycle misnamed");
            let finding = relation_cycle_finding(&ring)
                .unwrap_or_else(|| panic!("seed {seed}: refusal missing"));
            assert_eq!(finding.check, "relationCycle");
            assert_eq!((finding.a, finding.b), (3, 4));
            assert_eq!(finding.pct, 0.0);
            let residuals = super::super::relation::cycle_residuals_cm(&actors, &ring, &cycle);
            assert_eq!(residuals.len(), 3, "seed {seed}: every ring leg measurable");
            for (_, _, r) in &residuals {
                assert!(
                    !(0..=super::super::relation::SUPPORT_GAP_CM).contains(r),
                    "seed {seed}: fake leg reads healthy at {r} cm"
                );
            }
            caught += 1;
        }
        (caught, fp)
    }

    #[test]
    fn cycle_corpus_catches_seeded_rings_without_false_positives() {
        // 60 seeded graphs: >= 90% inconsistent caught, 0% FP, integer-cm
        // residuals only, smallest cycle named on every ring.
        let (caught, fp) = cycle_corpus_counts();
        assert_eq!(fp, 0, "{fp} false positives over 60 clean scenes");
        assert!(
            caught >= 54,
            "caught {caught}/60 injected rings, need >= 54"
        );
        assert_eq!(caught, 60, "caught {caught}/60 injected rings, need 60/60");
    }
}

/// SPF5: giantObject against the layout it lives in.
#[cfg(test)]
mod giant_tests {
    use super::*;

    fn solid(id: u32, x: f32, y: f32, z: f32, sx: f32, sy: f32, sz: f32) -> Actor {
        Actor {
            id,
            name: format!("solid_{id}"),
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
            yaw: 0.0,
        }
    }

    /// Four walls of a square room with the given inner half-side in meters.
    fn walls(first: u32, half: f32, long: f32) -> Vec<Actor> {
        vec![
            solid(first, 0.0, 1.5, -half, long, 3.0, 0.3),
            solid(first + 1, 0.0, 1.5, half, long, 3.0, 0.3),
            solid(first + 2, -half, 1.5, 0.0, 0.3, 3.0, long),
            solid(first + 3, half, 1.5, 0.0, 0.3, 3.0, long),
        ]
    }

    fn giants_of(actors: &[Actor]) -> Vec<u32> {
        let report = run_checks(actors, &["giantObject"]).expect("legal");
        report.findings.iter().map(|f| f.a).collect()
    }

    #[test]
    fn giant_layout_reads_the_layout_not_a_fixed_line() {
        // Four 20.6 m boundary walls around two props: the walls are the edge.
        let mut room = walls(1, 10.0, 20.6);
        room.push(solid(10, -2.0, 0.5, 0.0, 1.0, 1.0, 1.0));
        room.push(solid(11, 3.0, 0.5, 2.0, 1.0, 1.0, 1.0));
        assert_eq!(giants_of(&room), Vec::<u32>::new());
        // A 50 m crate dropped into a 4 m room is the one giant.
        let mut small = walls(1, 2.0, 4.0);
        small.push(solid(20, 30.0, 25.0, 0.0, 50.0, 50.0, 50.0));
        assert_eq!(giants_of(&small), vec![20]);
        // A 60 m wall inside a 100 m arena is no giant (old line said red).
        let mut arena = walls(1, 50.0, 100.0);
        arena.push(solid(30, 0.0, 1.5, 0.0, 60.0, 3.0, 0.3));
        assert_eq!(giants_of(&arena), Vec::<u32>::new());
        // Ground plates and non-physical boxes neither flag nor count.
        let mut plated = walls(1, 2.0, 4.0);
        plated.push(solid(40, 0.0, 0.1, 0.0, 40.0, 0.2, 40.0));
        let mut zone = solid(41, 30.0, 25.0, 0.0, 60.0, 60.0, 60.0);
        zone.physical = false;
        plated.push(zone);
        assert_eq!(giants_of(&plated), Vec::<u32>::new());
        // One solid alone has no layout to be bigger than.
        assert_eq!(
            giants_of(&[solid(1, 0.0, 30.0, 0.0, 60.0, 60.0, 60.0)]),
            Vec::<u32>::new()
        );
    }

    /// Brute force: union every other solid per candidate, no prefix tricks.
    fn brute_giants(actors: &[Actor]) -> Vec<u32> {
        let solids: Vec<&Actor> = actors
            .iter()
            .filter(|a| a.physical && a.sx > 0.0 && a.sy > 0.0 && a.sz > 0.0)
            .filter(|a| !(to_cm(a.sy) <= 50 && to_cm(a.sx) <= 5000 && to_cm(a.sz) <= 5000))
            .collect();
        let lo = |a: &Actor, k: usize| match k {
            0 => to_cm(a.x) - to_cm(a.sx) / 2,
            1 => to_cm(a.y) - to_cm(a.sy) / 2,
            _ => to_cm(a.z) - to_cm(a.sz) / 2,
        };
        let hi = |a: &Actor, k: usize| match k {
            0 => to_cm(a.x) + to_cm(a.sx) / 2,
            1 => to_cm(a.y) + to_cm(a.sy) / 2,
            _ => to_cm(a.z) + to_cm(a.sz) / 2,
        };
        let mut out = Vec::new();
        for c in &solids {
            let rest: Vec<&&Actor> = solids.iter().filter(|o| o.id != c.id).collect();
            if solids.len() < 2 {
                continue;
            }
            let span = (0..3)
                .map(|k| {
                    rest.iter().map(|o| hi(o, k)).max().unwrap_or(0)
                        - rest.iter().map(|o| lo(o, k)).min().unwrap_or(0)
                })
                .max()
                .unwrap_or(0);
            let side = to_cm(c.sx).max(to_cm(c.sy)).max(to_cm(c.sz));
            let encloses = [0, 2].into_iter().all(|k| {
                lo(c, k) <= rest.iter().map(|o| lo(o, k)).min().unwrap_or(0)
                    && rest.iter().map(|o| hi(o, k)).max().unwrap_or(0) <= hi(c, k)
            });
            if side > span && !(encloses && span * 4 >= side) {
                out.push(c.id);
            }
        }
        out.sort_unstable();
        out
    }

    #[test]
    fn giant_layout_matches_brute_force_on_seeded_scenes() {
        use engine_core::Rng;
        let mut rng = Rng::new(0x61a7);
        let mut fired = 0;
        for scene in 0..200u32 {
            let n = 2 + (rng.next_u32() % 12);
            let mut actors: Vec<Actor> = (0..n)
                .map(|i| {
                    let pick =
                        |r: &mut Rng, lo: u32, span: u32| (lo + r.next_u32() % span) as f32 / 10.0;
                    let big = rng.next_u32().is_multiple_of(6);
                    let (sx, sy, sz) = if big {
                        (
                            pick(&mut rng, 100, 900),
                            pick(&mut rng, 10, 300),
                            pick(&mut rng, 10, 900),
                        )
                    } else {
                        (
                            pick(&mut rng, 5, 40),
                            pick(&mut rng, 2, 40),
                            pick(&mut rng, 5, 40),
                        )
                    };
                    let mut a = solid(
                        i + 1,
                        pick(&mut rng, 0, 400) - 20.0,
                        sy / 2.0,
                        pick(&mut rng, 0, 400) - 20.0,
                        sx,
                        sy,
                        sz,
                    );
                    a.physical = !rng.next_u32().is_multiple_of(8);
                    a
                })
                .collect();
            let want = brute_giants(&actors);
            fired += usize::from(!want.is_empty());
            assert_eq!(giants_of(&actors), want, "scene {scene}");
            // Input order never moves the answer; the replay is byte-equal.
            actors.reverse();
            assert_eq!(giants_of(&actors), want, "scene {scene} reversed");
            assert_eq!(giants_of(&actors), giants_of(&actors), "scene {scene} x2");
        }
        assert!(
            fired > 20,
            "the seeded scenes must exercise the check: {fired}"
        );
    }
}
