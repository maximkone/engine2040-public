//! Bond-grade calibration corpus (DX14): seeded impacts measure the strain
//! at which each graded bond really snaps in a `BondedCollection`, and the
//! per-anchor-distance table is pinned against `fixtures/grade_calib.txt`.
//! Bake side only: nothing here is in the tick. To refresh the fixture after
//! a deliberate grade change, run with `GRADE_CALIB_BLESS=1`.

use engine_destruction::bond_grade::{regrade, GradeParams};
use engine_destruction::bonded::{BondedCollection, BondedDesc, BreakEvent};
use engine_destruction::structure::{building, BuildingSpec};

const FIXTURE: &str = "tests/fixtures/grade_calib.txt";
/// Hop buckets of the table; the last one holds every farther bond.
const HOPS: usize = 4;
/// Bisection steps per bond; the bracket is 1e7 impulse wide.
const STEPS: u32 = 48;

/// Two stories over 2 x 2 bays: small enough to bisect every bond.
fn wall(seed: u64) -> BondedDesc {
    let spec = BuildingSpec {
        bays_x: 2,
        bays_z: 2,
        stories: 2,
        knockouts: 0,
        ..BuildingSpec::three_story(seed)
    };
    building(&spec).expect("frame builds").desc
}

/// Anchor distance in bond hops of every chunk (`u32::MAX` if cut off).
fn hops(desc: &BondedDesc) -> Vec<u32> {
    let mut hops = vec![u32::MAX; desc.chunks.len()];
    let mut frontier: Vec<usize> = desc.anchors.iter().map(|&a| a as usize).collect();
    frontier.iter().for_each(|&a| hops[a] = 0);
    let mut depth = 0;
    while !frontier.is_empty() {
        depth += 1;
        let mut next = Vec::new();
        for bond in &desc.bonds {
            for (from, to) in [
                (bond.a as usize, bond.b as usize),
                (bond.b as usize, bond.a as usize),
            ] {
                if hops[from] == depth - 1 && hops[to] == u32::MAX {
                    hops[to] = depth;
                    next.push(to);
                }
            }
        }
        frontier = next;
    }
    hops
}

/// Measured limit of one bond in strain units: the smallest impulse on its
/// first chunk that snaps it, divided by the pair's mean mass.
fn measured_limit(desc: &BondedDesc, bond: usize) -> f32 {
    let (a, b) = (desc.bonds[bond].a as usize, desc.bonds[bond].b as usize);
    let snaps = |impulse: f32| {
        let mut field = BondedCollection::new(desc).expect("collection");
        field.apply_impact(a, impulse, 1).expect("impact");
        field
            .drain_events()
            .iter()
            .any(|e| matches!(e, BreakEvent::BondBroken { bond: id, .. } if *id as usize == bond))
    };
    let (mut lo, mut hi) = (0.0f32, 1e7f32);
    for _ in 0..STEPS {
        let mid = (lo + hi) * 0.5;
        if snaps(mid) {
            hi = mid;
        } else {
            lo = mid;
        }
    }
    hi / ((desc.chunks[a].mass + desc.chunks[b].mass) * 0.5)
}

/// Per hop bucket: bond count, summed graded strain, summed measured limit.
fn bucket(desc: &BondedDesc, graded: &BondedDesc, measured: &[f32]) -> [(u32, f32, f32); HOPS] {
    let hop = hops(desc);
    let mut rows = [(0u32, 0.0f32, 0.0f32); HOPS];
    for (i, bond) in graded.bonds.iter().enumerate() {
        let near = hop[bond.a as usize].min(hop[bond.b as usize]) as usize;
        let row = &mut rows[near.min(HOPS - 1)];
        row.0 += 1;
        row.1 += bond.breaking_strain;
        row.2 += measured[i];
    }
    rows
}

fn render(rows: &[(u32, f32, f32); HOPS]) -> String {
    let mut out = String::from(
        "# hop bonds graded_strain measured_limit (GradeParams::SIEGE, wall seeds 1 2 3)\n",
    );
    for (hop, (n, graded, measured)) in rows.iter().enumerate() {
        out.push_str(&format!("{hop} {n} {graded:.4} {measured:.4}\n"));
    }
    out
}

#[test]
fn graded_bonds_snap_at_their_strain_and_the_table_is_pinned() {
    let mut total = [(0u32, 0.0f32, 0.0f32); HOPS];
    // Graded strain without the anchor term, per bucket: the lift of the term.
    let mut flat_sum = [0.0f32; HOPS];
    for seed in 1..=3u64 {
        let desc = wall(seed);
        let graded = regrade(
            &desc,
            &GradeParams {
                seed,
                ..GradeParams::SIEGE
            },
        )
        .expect("grades");
        // Every bond holds until its graded strain fails: no snap below it.
        let measured: Vec<f32> = (0..graded.bonds.len())
            .map(|i| measured_limit(&graded, i))
            .collect();
        for (i, (bond, limit)) in graded.bonds.iter().zip(&measured).enumerate() {
            assert!(
                *limit >= bond.breaking_strain * 0.999,
                "seed {seed} bond {i}: {limit} under {}",
                bond.breaking_strain
            );
        }
        // Anchors survive even a hit far past every measured limit.
        let mut field = BondedCollection::new(&graded).expect("collection");
        for chunk in 0..graded.chunks.len() {
            field.apply_impact(chunk, 1e7, 1).expect("impact");
        }
        assert!(graded
            .anchors
            .iter()
            .all(|&a| field.is_anchored(a as usize)));
        let flat = GradeParams {
            anchor_gain: 0.0,
            ..GradeParams::SIEGE
        };
        let flat = regrade(&desc, &GradeParams { seed, ..flat }).expect("grades");
        for (sum, row) in flat_sum.iter_mut().zip(bucket(&desc, &flat, &measured)) {
            *sum += row.1;
        }
        for (sum, row) in total.iter_mut().zip(bucket(&desc, &graded, &measured)) {
            *sum = (sum.0 + row.0, sum.1 + row.1, sum.2 + row.2);
        }
    }
    for row in &mut total {
        let n = row.0.max(1) as f32;
        (row.1, row.2) = (row.1 / n, row.2 / n);
    }
    let rendered = render(&total);
    assert_eq!(rendered, render(&total), "render repeats");
    if std::env::var_os("GRADE_CALIB_BLESS").is_some() {
        std::fs::write(FIXTURE, &rendered).expect("fixture written");
    }
    let want = std::fs::read_to_string(FIXTURE)
        .unwrap_or_default()
        .replace("\r\n", "\n");
    assert_eq!(
        rendered, want,
        "grade table moved; bless with GRADE_CALIB_BLESS=1 if deliberate"
    );
    // The anchor term lifts a foot bond sevenfold and fades with every hop
    // (the buckets also differ in area and mass, so only the lift is pinned).
    let lift: Vec<f32> = (0..HOPS)
        .map(|h| total[h].1 * total[h].0 as f32 / flat_sum[h])
        .collect();
    assert!((lift[0] - 7.0).abs() < 0.07, "foot lift {lift:?}");
    assert!(
        lift.windows(2).all(|w| w[0] > w[1]) && lift[HOPS - 1] >= 1.0,
        "lift fades: {lift:?}"
    );
}
