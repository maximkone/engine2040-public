//! TV2 contract proof: every TV1 plan names exactly one NV2 link-registry
//! entry and rebakes exactly that link. Five fixtures (hurdle, vault, mantle,
//! climb, drop), each with its per-edge link record. Test-only: the chooser
//! stays in `traversal`, the registry in `engine_nav::links`; this file only
//! ties them, so there is no second registry.

use crate::traversal::{
    apply_warp, choose, plan, plan_hash, warp_error_mm, Probe, TraversalPlan, TraverseAction,
    MM_PER_M, WARP_TOL_MM,
};
use engine_nav::links::{rebake_for_set, rebake_links, LinkId, LinkKind, TraversalLink};
use engine_nav::tiles::{BreakSet, TileId};

/// One edge: the obstacle probe, where the root must land, and the link
/// record the cooker bakes for it.
struct Edge {
    probe: Probe,
    expect: TraverseAction,
    link: TraversalLink,
}

fn probe(height: f32, depth: f32, gap: f32, speed: f32, hold: bool) -> Probe {
    Probe {
        height_m: height,
        depth_m: depth,
        gap_m: gap,
        speed_m_s: speed,
        has_hold: hold,
    }
}

fn record(id: u32, kind: LinkKind, edge_m: [f32; 2]) -> TraversalLink {
    TraversalLink {
        id: LinkId(id),
        kind,
        edge_m,
        tile: TileId(id * 10),
    }
}

/// The five fixtures; every edge anchor is distinct, every tile too.
fn edges() -> Vec<Edge> {
    vec![
        Edge {
            probe: probe(0.5, 0.3, 0.4, 3.0, false),
            expect: TraverseAction::Hurdle,
            link: record(1, LinkKind::Hurdle, [4.0, 1.0]),
        },
        Edge {
            probe: probe(1.0, 0.6, 0.5, 2.0, false),
            expect: TraverseAction::Vault,
            link: record(2, LinkKind::Vault, [8.5, -2.0]),
        },
        Edge {
            probe: probe(1.5, 2.0, 0.2, 1.0, false),
            expect: TraverseAction::Mantle,
            link: record(3, LinkKind::Mantle, [-3.25, 6.0]),
        },
        Edge {
            probe: probe(2.0, 0.4, 0.3, 1.0, true),
            expect: TraverseAction::Climb,
            link: record(4, LinkKind::Climb, [12.0, 12.5]),
        },
        Edge {
            probe: probe(-1.0, 0.5, 0.4, 1.5, false),
            expect: TraverseAction::Drop,
            link: record(5, LinkKind::Drop, [0.5, -7.75]),
        },
    ]
}

/// The registry link kind a TV1 action must ride; `None` needs no link.
fn kind_of(action: TraverseAction) -> Option<LinkKind> {
    match action {
        TraverseAction::None => None,
        TraverseAction::Hurdle => Some(LinkKind::Hurdle),
        TraverseAction::Vault => Some(LinkKind::Vault),
        TraverseAction::Mantle => Some(LinkKind::Mantle),
        TraverseAction::Climb => Some(LinkKind::Climb),
        TraverseAction::Drop => Some(LinkKind::Drop),
    }
}

/// Registry entries whose anchor is this plan's edge, in millimeters.
fn entries_for<'a>(links: &'a [TraversalLink], plan: &TraversalPlan) -> Vec<&'a TraversalLink> {
    links
        .iter()
        .filter(|l| {
            [
                (l.edge_m[0] * MM_PER_M).round() as i32,
                (l.edge_m[1] * MM_PER_M).round() as i32,
            ] == plan.edge_mm
        })
        .collect()
}

#[test]
fn every_plan_names_exactly_its_registry_entry() {
    let edges = edges();
    let links: Vec<TraversalLink> = edges.iter().map(|e| e.link).collect();
    for e in &edges {
        let p = plan(&e.probe, e.link.edge_m).expect("fixture plans");
        assert_eq!(p.action, e.expect, "TV1 picks the expected action");
        assert_eq!(choose(&e.probe), Ok(e.expect));
        let found = entries_for(&links, &p);
        assert_eq!(
            found.len(),
            1,
            "one registry entry per edge: {:?}",
            p.action
        );
        assert_eq!(
            Some(found[0].kind),
            kind_of(p.action),
            "kind rides the action"
        );
        assert_eq!(found[0].id, e.link.id);
    }
}

#[test]
fn each_edge_rebakes_exactly_its_own_link() {
    let edges = edges();
    let links: Vec<TraversalLink> = edges.iter().map(|e| e.link).collect();
    for e in &edges {
        assert_eq!(
            rebake_links(&links, &[e.link.tile]),
            Ok(vec![e.link.id]),
            "tile {:?} rebakes only link {:?}",
            e.link.tile,
            e.link.id
        );
    }
    // A break batch over two edges rebakes those two, ascending by id.
    let set = BreakSet::new(7, &[edges[3].link.tile, edges[1].link.tile], 100).expect("set");
    assert_eq!(
        rebake_for_set(&links, &set),
        Ok(vec![edges[1].link.id, edges[3].link.id])
    );
}

#[test]
fn a_wrong_kind_or_missing_entry_fails_the_contract() {
    let edges = edges();
    let mut links: Vec<TraversalLink> = edges.iter().map(|e| e.link).collect();
    let vault = plan(&edges[1].probe, edges[1].link.edge_m).expect("plans");
    // Registry says Hurdle where TV1 vaults: the kind check must see it.
    links[1].kind = LinkKind::Hurdle;
    let found = entries_for(&links, &vault);
    assert_ne!(Some(found[0].kind), kind_of(vault.action));
    // Entry gone: nothing names the plan's edge.
    links.remove(1);
    assert!(entries_for(&links, &vault).is_empty());
}

#[test]
fn warped_roots_land_within_1cm_and_bytes_repeat_x2() {
    let run = || {
        edges()
            .iter()
            .map(|e| {
                let p = plan(&e.probe, e.link.edge_m).expect("fixture plans");
                let raw = [[0.0, 0.0], [1.0, 0.25], [2.0, 0.5]];
                let warped = apply_warp(&p, &raw).expect("fixture warps");
                let end = *warped.last().expect("warped nonempty");
                let err = warp_error_mm(end, e.link.edge_m);
                assert!(err <= WARP_TOL_MM, "miss {err} mm past 1 cm");
                (plan_hash(&p), warped)
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(run(), run());
}
