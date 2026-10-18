//! TV3 siege fixtures: the C5 siege map's ladder, rampart vault, rubble scramble
//! and breach drop ride the TV1 chooser and warp, and each edge names its own NV2
//! link record, rebaked exactly like TV2. Test-only: the chooser stays in
//! `traversal`, the registry in `engine_nav::links`; the siege set is data here,
//! so there is no second chooser and no second registry.

use crate::traversal::{
    apply_warp, choose, plan, plan_hash, warp_error_mm, Probe, TraversalPlan, TraverseAction,
    TraverseError, MAX_GAP_MM, MM_PER_M, WARP_TOL_MM,
};
use engine_nav::links::{rebake_for_set, rebake_links, LinkId, LinkKind, TraversalLink};
use engine_nav::tiles::{BreakSet, TileId};

/// One siege edge: where it is on the map, the probe the actor sees there, the
/// move the chooser must commit and the link record the cooker bakes for it.
struct Siege {
    name: &'static str,
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

/// The siege set. The low rubble scramble and the high heap share one breach
/// tile: a wall that falls scatters both, so one break rebakes both links.
fn siege() -> Vec<Siege> {
    let mut rubble_far = record(6, LinkKind::Mantle, [-9.0, 33.5]);
    rubble_far.tile = TileId(40);
    vec![
        Siege {
            name: "ladder to the wall walk",
            probe: probe(2.4, 0.3, 0.2, 0.5, true),
            expect: TraverseAction::Climb,
            link: record(1, LinkKind::Climb, [14.0, 30.0]),
        },
        Siege {
            name: "rampart vault",
            probe: probe(1.1, 0.6, 0.5, 2.5, false),
            expect: TraverseAction::Vault,
            link: record(2, LinkKind::Vault, [6.5, 28.25]),
        },
        Siege {
            name: "thick rampart stood on",
            probe: probe(1.1, 1.8, 0.5, 2.5, false),
            expect: TraverseAction::Mantle,
            link: record(3, LinkKind::Mantle, [-2.0, 28.25]),
        },
        Siege {
            name: "low rubble scramble",
            probe: probe(0.45, 0.5, 0.4, 3.0, false),
            expect: TraverseAction::Hurdle,
            link: record(4, LinkKind::Hurdle, [-7.5, 31.0]),
        },
        Siege {
            name: "deep rubble heap",
            probe: probe(0.7, 1.4, 0.4, 3.0, false),
            expect: TraverseAction::Mantle,
            link: record(5, LinkKind::Mantle, [-8.25, 32.25]),
        },
        Siege {
            name: "high rubble heap",
            probe: probe(1.4, 1.0, 0.3, 1.5, false),
            expect: TraverseAction::Mantle,
            link: rubble_far,
        },
        Siege {
            name: "breach drop off the wall walk",
            probe: probe(-1.8, 0.5, 0.4, 1.5, false),
            expect: TraverseAction::Drop,
            link: record(7, LinkKind::Drop, [12.0, 27.5]),
        },
    ]
}

fn kind_of(action: TraverseAction) -> LinkKind {
    match action {
        TraverseAction::Hurdle => LinkKind::Hurdle,
        TraverseAction::Vault => LinkKind::Vault,
        TraverseAction::Mantle => LinkKind::Mantle,
        TraverseAction::Climb => LinkKind::Climb,
        TraverseAction::Drop => LinkKind::Drop,
        TraverseAction::None => panic!("a siege edge always carries a move"),
    }
}

fn plan_of(edge: &Siege) -> TraversalPlan {
    plan(&edge.probe, edge.link.edge_m).unwrap_or_else(|err| panic!("{}: {err}", edge.name))
}

#[test]
fn siege_fixtures_pick_the_expected_actions() {
    for edge in siege() {
        let p = plan_of(&edge);
        assert_eq!(p.action, edge.expect, "{}", edge.name);
        assert_eq!(choose(&edge.probe), Ok(edge.expect), "{}", edge.name);
        assert_eq!(
            kind_of(p.action),
            edge.link.kind,
            "{}: link kind",
            edge.name
        );
    }
    // The set covers every move the siege map needs, each more than once or on purpose.
    let actions: Vec<TraverseAction> = siege().iter().map(|e| e.expect).collect();
    for want in [
        TraverseAction::Climb,
        TraverseAction::Vault,
        TraverseAction::Mantle,
        TraverseAction::Hurdle,
        TraverseAction::Drop,
    ] {
        assert!(actions.contains(&want), "{want:?} is in the siege set");
    }
}

#[test]
fn a_broken_ladder_and_a_wide_breach_are_refused_never_faked() {
    let set = siege();
    let mut ladder = set[0].probe;
    ladder.has_hold = false;
    assert_eq!(choose(&ladder), Err(TraverseError::NoHold));
    let mut breach = set[6].probe;
    breach.gap_m = (MAX_GAP_MM as f32 + 100.0) / MM_PER_M;
    assert!(matches!(
        choose(&breach),
        Err(TraverseError::GapTooWide { .. })
    ));
}

#[test]
fn every_siege_edge_names_one_link_and_a_breach_rebakes_exactly_the_scattered_ones() {
    let set = siege();
    let links: Vec<TraversalLink> = set.iter().map(|e| e.link).collect();
    for (edge, link) in set.iter().zip(&links) {
        let p = plan_of(edge);
        let named: Vec<&TraversalLink> = links
            .iter()
            .filter(|l| {
                [
                    (l.edge_m[0] * MM_PER_M).round() as i32,
                    (l.edge_m[1] * MM_PER_M).round() as i32,
                ] == p.edge_mm
            })
            .collect();
        assert_eq!(named.len(), 1, "{}: one entry per edge", edge.name);
        assert_eq!(named[0].id, link.id);
    }
    // One tile each, except the two far rubble heaps that share the breach tile.
    let far = TileId(40);
    let rebaked = |tiles: &[TileId]| {
        let set = BreakSet::new(7, tiles, 100).expect("set");
        rebake_for_set(&links, &set)
    };
    assert_eq!(rebaked(&[far]), Ok(vec![LinkId(4), LinkId(6)]));
    assert_eq!(rebaked(&[TileId(10)]), Ok(vec![LinkId(1)]));
    assert_eq!(
        rebaked(&[TileId(70), far, TileId(20)]),
        Ok(vec![LinkId(2), LinkId(4), LinkId(6), LinkId(7)])
    );
    assert_eq!(rebake_links(&links, &[TileId(999)]), Ok(vec![]));
}

#[test]
fn warped_roots_land_within_1cm_and_bytes_repeat_x2() {
    let run = || {
        siege()
            .iter()
            .map(|edge| {
                let p = plan_of(edge);
                // A 64-point raw path ending short of the edge, the way the clip's root motion does.
                let raw: Vec<[f32; 2]> = (0..64)
                    .map(|k| [k as f32 * 0.07, 28.0 + k as f32 * 0.05])
                    .collect();
                let warped = apply_warp(&p, &raw).expect("siege path warps");
                let end = *warped.last().expect("warped nonempty");
                let err = warp_error_mm(end, edge.link.edge_m);
                assert!(
                    err <= WARP_TOL_MM,
                    "{}: root misses the edge by {err} mm",
                    edge.name
                );
                let bits: Vec<[u32; 2]> = warped
                    .iter()
                    .map(|p| [p[0].to_bits(), p[1].to_bits()])
                    .collect();
                (plan_hash(&p), bits)
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(run(), run());
}
