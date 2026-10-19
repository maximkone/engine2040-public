//! TV4 rubble-variance proof: the siege rubble scramble edges hold across a
//! seeded sweep of rubble height, depth, gap and approach speed, and the
//! breach rebake is exact twice. Test-only: the chooser stays in `traversal`,
//! the registry in `engine_nav::links`, and every lane drawn here sits inside
//! the chooser gates, so the expected move is the only legal pick in the band.
//!
//! A band is not a guess: the band-edge test walks one step past each band and
//! shows the chooser really turns there, so the sweep cannot pass by being
//! constant.

use crate::traversal::{
    apply_warp, choose, plan, plan_hash, warp_error_mm, Probe, TraverseAction, HURDLE_DEPTH_MM,
    HURDLE_MAX_MM, HURDLE_MIN_SPEED_MM_S, MANTLE_MAX_MM, MM_PER_M, STEP_MM, VAULT_MAX_MM,
    WARP_TOL_MM,
};
use engine_core::Rng;
use engine_nav::links::{rebake_for_set, LinkId, LinkKind, TraversalLink};
use engine_nav::tiles::{BreakSet, TileId};

/// One rubble edge: the TV3 probe's move, the breach link the cooker bakes for
/// it, and the integer-mm band each lane sweeps. Bands are inclusive.
struct Rubble {
    name: &'static str,
    expect: TraverseAction,
    link: TraversalLink,
    height_mm: (i32, i32),
    depth_mm: (i32, i32),
    gap_mm: (i32, i32),
    speed_mm_s: (i32, i32),
}

fn record(id: u32, kind: LinkKind, edge_m: [f32; 2]) -> TraversalLink {
    TraversalLink {
        id: LinkId(id),
        kind,
        edge_m,
        tile: TileId(id * 10),
    }
}

/// The TV3 rubble split: the low scramble, the deep heap and the high heap.
/// The two far heaps share the breach tile (`id * 10 = 40`), so one break
/// rebakes both; the deep heap sits on its own tile 50.
fn rubble() -> Vec<Rubble> {
    let mut high = record(6, LinkKind::Mantle, [-9.0, 33.5]);
    // The high heap sits on the same breach tile as the low scramble, so the
    // falling wall scatters both.
    high.tile = TileId(40);
    vec![
        Rubble {
            name: "low rubble scramble",
            expect: TraverseAction::Hurdle,
            link: record(4, LinkKind::Hurdle, [-7.5, 31.0]),
            // Hurdle gates: h <= 0.75 m, depth <= 0.8 m, speed >= 1.0 m/s.
            height_mm: (STEP_MM + 50, HURDLE_MAX_MM),
            depth_mm: (100, HURDLE_DEPTH_MM),
            gap_mm: (100, 500),
            speed_mm_s: (HURDLE_MIN_SPEED_MM_S, 4000),
        },
        Rubble {
            name: "deep rubble heap",
            expect: TraverseAction::Mantle,
            link: record(5, LinkKind::Mantle, [-8.25, 32.25]),
            // Past the hurdle depth band the low top is mantled onto, whatever
            // the speed: depth only, height stays under the hurdle top.
            height_mm: (STEP_MM + 50, HURDLE_MAX_MM),
            depth_mm: (HURDLE_DEPTH_MM + 100, 1400),
            gap_mm: (100, 500),
            speed_mm_s: (100, 4000),
        },
        Rubble {
            name: "high rubble heap",
            expect: TraverseAction::Mantle,
            link: high,
            // Above the vault top and at or below the mantle top: always a
            // mantle, so the sweep covers the whole chest-high band.
            height_mm: (VAULT_MAX_MM + 50, MANTLE_MAX_MM),
            depth_mm: (100, 1400),
            gap_mm: (100, 500),
            speed_mm_s: (100, 4000),
        },
    ]
}

/// Kind the registry record must carry for a chosen move.
fn kind_of(action: TraverseAction) -> LinkKind {
    match action {
        TraverseAction::Hurdle => LinkKind::Hurdle,
        TraverseAction::Vault => LinkKind::Vault,
        TraverseAction::Mantle => LinkKind::Mantle,
        TraverseAction::Climb => LinkKind::Climb,
        TraverseAction::Drop => LinkKind::Drop,
        TraverseAction::None => panic!("a rubble edge always carries a move"),
    }
}

/// One lane draw in `[lo, hi]` inclusive; bands are small positive mm.
fn draw(rng: &mut Rng, lo: i32, hi: i32) -> i32 {
    let span = u64::try_from(hi - lo + 1).expect("band span is positive");
    lo + i32::try_from(rng.next_range(0, span)).expect("draw fits i32")
}

/// Draw one probe inside the edge's bands: integer mm first, meters after, the
/// way the arena probe reaches the chooser.
fn sample(edge: &Rubble, rng: &mut Rng) -> Probe {
    let height = draw(rng, edge.height_mm.0, edge.height_mm.1);
    let depth = draw(rng, edge.depth_mm.0, edge.depth_mm.1);
    let gap = draw(rng, edge.gap_mm.0, edge.gap_mm.1);
    let speed = draw(rng, edge.speed_mm_s.0, edge.speed_mm_s.1);
    probe_mm(height, depth, gap, speed)
}

fn probe_mm(height: i32, depth: i32, gap: i32, speed: i32) -> Probe {
    Probe {
        height_m: height as f32 / MM_PER_M,
        depth_m: depth as f32 / MM_PER_M,
        gap_m: gap as f32 / MM_PER_M,
        speed_m_s: speed as f32 / MM_PER_M,
        has_hold: false,
    }
}

/// The root path a rubble scramble clip feeds the warp: 64 points ending short
/// of the edge.
fn breach_path() -> Vec<[f32; 2]> {
    (0..64)
        .map(|k| [k as f32 * 0.07, 28.0 + k as f32 * 0.05])
        .collect()
}

/// Seeded sweep: every draw picks the edge's expected move, names its link
/// kind, and warps its root inside 1 cm of the edge anchor.
#[test]
fn rubble_var_scramble_holds_across_seeded_height_and_noise_sweeps() {
    let edges = rubble();
    let links: Vec<TraversalLink> = edges.iter().map(|e| e.link).collect();
    let raw = breach_path();
    let mut heights: Vec<i32> = Vec::new();
    let mut worst_mm = 0.0f32;
    for seed in [1u64, 7, 2040, 9999] {
        for edge in &edges {
            let mut rng = Rng::new(seed).fork_named(edge.name);
            for _ in 0..64 {
                let p = sample(edge, &mut rng);
                heights.push((p.height_m * MM_PER_M).round() as i32);
                assert_eq!(
                    choose(&p),
                    Ok(edge.expect),
                    "{}: seed {seed} probe {p:?}",
                    edge.name
                );
                let planned =
                    plan(&p, edge.link.edge_m).unwrap_or_else(|err| panic!("{}: {err}", edge.name));
                assert_eq!(planned.action, edge.expect, "{}", edge.name);
                assert_eq!(kind_of(planned.action), edge.link.kind, "{}", edge.name);
                let named: Vec<&TraversalLink> = links
                    .iter()
                    .filter(|l| {
                        [
                            (l.edge_m[0] * MM_PER_M).round() as i32,
                            (l.edge_m[1] * MM_PER_M).round() as i32,
                        ] == planned.edge_mm
                    })
                    .collect();
                assert_eq!(named.len(), 1, "{}: one link per edge", edge.name);
                assert_eq!(named[0].id, edge.link.id, "{}", edge.name);
                let warped = apply_warp(&planned, &raw).expect("rubble path warps");
                let end = *warped.last().expect("warped nonempty");
                let err = warp_error_mm(end, edge.link.edge_m);
                assert!(
                    err <= WARP_TOL_MM,
                    "{}: root misses the edge by {err} mm",
                    edge.name
                );
                worst_mm = if err > worst_mm { err } else { worst_mm };
            }
        }
    }
    assert_eq!(heights.len(), 4 * 3 * 64, "the sweep covered every lane");
    assert!(
        heights.iter().any(|h| *h != heights[0]),
        "the sweep must really move the rubble height"
    );
    assert_eq!(worst_mm, 0.0, "integer warp lands exactly on the edge");
}

/// The bands are the chooser gates, not a guess: the exact band ends still pick
/// the expected move, and one step past a band really turns the pick.
#[test]
fn rubble_var_bands_sit_exactly_on_the_chooser_gates() {
    let edges = rubble();
    let low = &edges[0];
    for height in [low.height_mm.0, low.height_mm.1] {
        assert_eq!(choose(&probe_mm(height, 500, 300, 3000)), Ok(low.expect));
    }
    for depth in [low.depth_mm.0, low.depth_mm.1] {
        assert_eq!(choose(&probe_mm(500, depth, 300, 3000)), Ok(low.expect));
    }
    for speed in [low.speed_mm_s.0, low.speed_mm_s.1] {
        assert_eq!(choose(&probe_mm(500, 500, 300, speed)), Ok(low.expect));
    }
    // One step past each low-scramble gate: the pick really changes.
    assert_eq!(
        choose(&probe_mm(low.height_mm.1 + 1, 500, 300, 3000)),
        Ok(TraverseAction::Vault),
        "past the hurdle top a clearable block vaults"
    );
    assert_eq!(
        choose(&probe_mm(500, low.depth_mm.1 + 1, 300, 3000)),
        Ok(TraverseAction::Mantle),
        "past the hurdle depth a low block is mantled onto"
    );
    assert_eq!(
        choose(&probe_mm(500, 500, 300, low.speed_mm_s.0 - 1)),
        Ok(TraverseAction::Mantle),
        "under the hurdle speed a low block is mantled onto"
    );

    let deep = &edges[1];
    for height in [deep.height_mm.0, deep.height_mm.1] {
        for depth in [deep.depth_mm.0, deep.depth_mm.1] {
            assert_eq!(choose(&probe_mm(height, depth, 300, 3000)), Ok(deep.expect));
        }
    }
    assert_eq!(
        choose(&probe_mm(500, HURDLE_DEPTH_MM, 300, 3000)),
        Ok(TraverseAction::Hurdle),
        "inside the hurdle depth the same top hurdles"
    );

    let high = &edges[2];
    for height in [high.height_mm.0, high.height_mm.1] {
        assert_eq!(choose(&probe_mm(height, 600, 300, 1500)), Ok(high.expect));
    }
    assert_eq!(
        choose(&probe_mm(high.height_mm.1 + 1, 600, 300, 1500)),
        Err(crate::traversal::TraverseError::NoHold),
        "past the mantle top a bare heap needs a hold"
    );
    assert_eq!(
        MANTLE_MAX_MM, high.height_mm.1,
        "band top is the mantle gate"
    );
    assert_eq!(STEP_MM + 50, low.height_mm.0, "band floor is the step gate");
}

/// A breach rebake is exact twice, and the whole sweep replays byte-identical:
/// same link ids, same plan hashes, same warped bits.
#[test]
fn rubble_var_breach_rebake_is_exact_x2() {
    let links: Vec<TraversalLink> = rubble().iter().map(|e| e.link).collect();
    let breach = BreakSet::new(7, &[TileId(40)], 100).expect("breach set");
    let first = rebake_for_set(&links, &breach).expect("breach rebakes");
    assert_eq!(
        first,
        vec![LinkId(4), LinkId(6)],
        "the falling wall scatters both far heaps and nothing else"
    );
    assert_eq!(
        rebake_for_set(&links, &breach),
        Ok(first),
        "same registry, same breach, same ids twice"
    );
    assert_eq!(
        rebake_for_set(&links, &BreakSet::new(7, &[TileId(50)], 100).expect("set")),
        Ok(vec![LinkId(5)]),
        "another rock names only the deep heap"
    );
    assert_eq!(sweep_receipt(&links), sweep_receipt(&links), "sweep x2");
}

/// Plan hash plus warped bits per swept sample, in draw order: the receipt two
/// runs must agree on byte for byte.
fn sweep_receipt(links: &[TraversalLink]) -> Vec<(u64, Vec<[u32; 2]>)> {
    let edges = rubble();
    let raw = breach_path();
    let mut receipt = Vec::with_capacity(4 * 3 * 64);
    for seed in [1u64, 7, 2040, 9999] {
        for (edge, link) in edges.iter().zip(links.iter()) {
            let mut rng = Rng::new(seed).fork_named(edge.name);
            for _ in 0..64 {
                let p = sample(edge, &mut rng);
                let planned = plan(&p, link.edge_m).expect("rubble sample plans");
                let warped = apply_warp(&planned, &raw).expect("rubble path warps");
                let bits: Vec<[u32; 2]> = warped
                    .iter()
                    .map(|q| [q[0].to_bits(), q[1].to_bits()])
                    .collect();
                receipt.push((plan_hash(&planned), bits));
            }
        }
    }
    receipt
}
