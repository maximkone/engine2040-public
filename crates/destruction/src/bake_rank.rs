//! bake_rank: detail where it breaks, a bake probe for chunk lattices.
//!
//! Consumer: the bake path, ahead of `bond_grade::regrade`; nothing here
//! runs in the tick. The idea (fine mesh where the front passes, coarse in
//! the wake; phase-field wake coarsening, arXiv 2609.21201) is used as a
//! plan only, no solver or code taken: a box lattice keeps its cell counts
//! and its contact graph, but each axis spaces its cuts by the likelihood
//! of impacts, so cells shrink near likely hits and grow elsewhere. Units:
//! meters. Determinism: `+ - * /` on floats in index order, no `Rng`; the
//! same box, counts and impacts give the same bytes on every PC. Errors:
//! [`RankError`]. Budget: `64` density bins per axis, bake only.

use crate::bake::CONTACT_MARGIN;
use crate::bonded::{Bond, BondedDesc, BreakError, Chunk3, MAX_BONDED_CHUNKS};
use crate::shards3d::in_contact;

/// Density bins per axis; the cut positions are exact within one bin.
const BINS: usize = 64;
/// Anchor tolerance for the bottom layer; meters.
const ANCHOR_EPS_M: f32 = 1e-3;
/// Largest accepted likelihood gain.
const MAX_GAIN: f32 = 1e3;

/// A place impacts are likely: a point and its relative weight.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Likely {
    /// Impact point; meters.
    pub at: [f32; 3],
    /// Relative weight, finite and non-negative.
    pub weight: f32,
}

/// The lattice to cut: box, cells per axis and how hard impacts pull.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RankSpec {
    /// Box minimum corner; meters.
    pub min: [f32; 3],
    /// Box maximum corner; meters.
    pub max: [f32; 3],
    /// Cells per axis, each at least one; the product is the chunk count.
    pub cells: [u32; 3],
    /// Distance at which an impact's pull halves; meters, positive.
    pub reach_m: f32,
    /// Density of a likely spot over the plain one; `0.0..=1000.0`.
    pub gain: f32,
}

/// Why a lattice was refused; input never panics.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RankError {
    /// Box or reach is not finite, positive and ordered, or gain is out of range.
    BadSpec,
    /// A cell count is zero or the product exceeds the collection cap.
    BadCells,
    /// An impact point or weight is not finite, or the weight is negative.
    BadImpact,
    /// A cell came out thinner than the contact margin, which would change
    /// the bond graph; lower the gain or the cell count.
    Thin,
    /// The lattice descriptor was refused.
    Bonded(BreakError),
}

impl core::fmt::Display for RankError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            RankError::BadSpec => write!(f, "lattice box, reach or gain is out of range"),
            RankError::BadCells => write!(f, "lattice cell counts are out of range"),
            RankError::BadImpact => write!(f, "likely impact is not finite"),
            RankError::Thin => write!(f, "a ranked cell is thinner than the contact margin"),
            RankError::Bonded(e) => write!(f, "lattice descriptor refused: {e}"),
        }
    }
}

impl std::error::Error for RankError {}

impl From<BreakError> for RankError {
    fn from(e: BreakError) -> Self {
        RankError::Bonded(e)
    }
}

/// Bin count as an exact `f32`.
fn bins_f32(n: usize) -> f32 {
    f32::from(u16::try_from(n).unwrap_or(u16::MAX))
}

/// Cut positions along one axis: `cells + 1` edges from `min` to `max`,
/// spaced by the inverse of the cumulative impact density.
fn axis_edges(spec: &RankSpec, likely: &[Likely], axis: usize) -> Vec<f32> {
    let (lo, hi) = (spec.min[axis], spec.max[axis]);
    let span = hi - lo;
    let mut density = [0.0f32; BINS];
    for (i, slot) in density.iter_mut().enumerate() {
        let centre = lo + span * (bins_f32(i) + 0.5) / bins_f32(BINS);
        let mut pull = 0.0f32;
        for l in likely {
            let u = (centre - l.at[axis]) / spec.reach_m;
            pull += l.weight / (1.0 + u * u);
        }
        *slot = 1.0 + spec.gain * pull;
    }
    let mut cum = [0.0f32; BINS + 1];
    for i in 0..BINS {
        cum[i + 1] = cum[i] + density[i];
    }
    let cells = usize::try_from(spec.cells[axis]).unwrap_or(1);
    let mut edges = vec![lo];
    let mut bin = 0usize;
    for j in 1..cells {
        let target = cum[BINS] * bins_f32(j) / bins_f32(cells);
        while bin + 1 < BINS && cum[bin + 1] <= target {
            bin += 1;
        }
        let within = (target - cum[bin]) / density[bin];
        edges.push(lo + span * (bins_f32(bin) + within) / bins_f32(BINS));
    }
    edges.push(hi);
    edges
}

/// Cuts the lattice: chunks in z, y, x index order, one per cell. With no
/// likely impacts every cell is the same size (the uniform lattice).
pub fn ranked_grid(spec: &RankSpec, likely: &[Likely]) -> Result<Vec<Chunk3>, RankError> {
    let ordered = (0..3)
        .all(|a| spec.min[a].is_finite() && spec.max[a].is_finite() && spec.min[a] < spec.max[a]);
    let gain_ok = spec.gain.is_finite() && (0.0..=MAX_GAIN).contains(&spec.gain);
    if !ordered || !gain_ok || !spec.reach_m.is_finite() || spec.reach_m <= 0.0 {
        return Err(RankError::BadSpec);
    }
    let total: u64 = spec.cells.iter().map(|&c| u64::from(c)).product();
    if spec.cells.contains(&0) || total > u64::try_from(MAX_BONDED_CHUNKS).unwrap_or(u64::MAX) {
        return Err(RankError::BadCells);
    }
    let bad = |l: &Likely| {
        !(l.weight.is_finite() && l.weight >= 0.0 && l.at.iter().all(|v| v.is_finite()))
    };
    if likely.iter().any(bad) {
        return Err(RankError::BadImpact);
    }
    let edges: Vec<Vec<f32>> = (0..3).map(|a| axis_edges(spec, likely, a)).collect();
    if edges
        .iter()
        .any(|e| e.windows(2).any(|w| w[1] - w[0] <= CONTACT_MARGIN))
    {
        return Err(RankError::Thin);
    }
    let mut chunks = Vec::with_capacity(usize::try_from(total).unwrap_or(0));
    for z in edges[2].windows(2) {
        for y in edges[1].windows(2) {
            for x in edges[0].windows(2) {
                let (hx, hy, hz) = (
                    (x[1] - x[0]) * 0.5,
                    (y[1] - y[0]) * 0.5,
                    (z[1] - z[0]) * 0.5,
                );
                chunks.push(Chunk3 {
                    x: x[0] + hx,
                    y: y[0] + hy,
                    z: z[0] + hz,
                    hx,
                    hy,
                    hz,
                    mass: hx * hy * hz * 8.0,
                });
            }
        }
    }
    Ok(chunks)
}

/// Bonded descriptor of a lattice: contact bonds at `breaking_strain`, the
/// bottom layer anchored, validated before it returns.
pub fn lattice_desc(
    chunks: Vec<Chunk3>,
    breaking_strain: f32,
    break_threshold: f32,
) -> Result<BondedDesc, RankError> {
    let mut bonds = Vec::new();
    for a in 0..chunks.len() {
        for b in (a + 1)..chunks.len() {
            if in_contact(&chunks[a], &chunks[b]) {
                bonds.push(Bond {
                    a: u32::try_from(a).unwrap_or(u32::MAX),
                    b: u32::try_from(b).unwrap_or(u32::MAX),
                    breaking_strain,
                });
            }
        }
    }
    let bottom = chunks
        .iter()
        .map(|c| c.y - c.hy)
        .fold(f32::INFINITY, f32::min);
    let anchors = (0..chunks.len())
        .filter(|&i| (chunks[i].y - chunks[i].hy) - bottom <= ANCHOR_EPS_M)
        .map(|i| u32::try_from(i).unwrap_or(u32::MAX))
        .collect();
    let desc = BondedDesc {
        chunks,
        bonds,
        anchors,
        break_threshold,
    };
    desc.validate()?;
    Ok(desc)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bake::BREAKING_THRESHOLD;
    use crate::crack::spread_graph;
    use crate::spread::spread_cracks;

    fn spec() -> RankSpec {
        RankSpec {
            min: [-2.0, 0.0, -0.4],
            max: [2.0, 3.0, 0.4],
            cells: [8, 6, 2],
            reach_m: 0.5,
            gain: 20.0,
        }
    }

    /// Impacts land low on the left of the wall.
    fn likely() -> [Likely; 2] {
        [
            Likely {
                at: [-1.2, 1.0, 0.0],
                weight: 1.0,
            },
            Likely {
                at: [-0.8, 1.6, 0.0],
                weight: 0.5,
            },
        ]
    }

    fn volume(c: &Chunk3) -> f32 {
        c.hx * c.hy * c.hz * 8.0
    }

    #[test]
    fn ranking_keeps_the_count_the_volume_and_the_bond_graph() {
        let s = spec();
        let uniform = ranked_grid(&s, &[]).expect("uniform");
        let ranked = ranked_grid(&s, &likely()).expect("ranked");
        assert_eq!(uniform.len(), 96);
        assert_eq!(ranked.len(), uniform.len());
        let total = |c: &[Chunk3]| c.iter().map(volume).sum::<f32>();
        assert!((total(&ranked) - total(&uniform)).abs() < 1e-3);
        let du = lattice_desc(uniform, 1.0, BREAKING_THRESHOLD).expect("uniform desc");
        let dr = lattice_desc(ranked, 1.0, BREAKING_THRESHOLD).expect("ranked desc");
        assert_eq!(du.bonds.len(), dr.bonds.len(), "same bond count as uniform");
        assert_eq!(du.anchors, dr.anchors);
    }

    #[test]
    fn cells_are_fine_where_impacts_land_and_coarse_in_the_wake() {
        let s = spec();
        let uniform = ranked_grid(&s, &[]).expect("uniform");
        let ranked = ranked_grid(&s, &likely()).expect("ranked");
        let near = |c: &Chunk3| (c.x + 1.2).abs() < 0.4 && (c.y - 1.0).abs() < 0.4;
        let wake = |c: &Chunk3| c.x > 1.0 && c.y > 2.0;
        let mean = |cs: &[Chunk3], keep: &dyn Fn(&Chunk3) -> bool| {
            let v: Vec<f32> = cs.iter().filter(|c| keep(c)).map(volume).collect();
            v.iter().sum::<f32>() / v.len().max(1) as f32
        };
        assert!(
            mean(&ranked, &near) < mean(&uniform, &near) * 0.6,
            "fine at the impact"
        );
        assert!(
            mean(&ranked, &wake) > mean(&uniform, &wake) * 1.3,
            "coarse in the wake"
        );
    }

    #[test]
    fn bakes_repeat_and_eight_insertion_orders_spread_equal() {
        let s = spec();
        let build = || {
            let chunks = ranked_grid(&s, &likely()).expect("ranked");
            lattice_desc(chunks, 1.0, BREAKING_THRESHOLD).expect("desc")
        };
        let desc = build();
        let bits = |d: &BondedDesc| {
            d.chunks
                .iter()
                .flat_map(|c| [c.x, c.y, c.z, c.hx, c.hy, c.hz].map(f32::to_bits))
                .collect::<Vec<_>>()
        };
        assert_eq!(bits(&desc), bits(&build()));
        let spread_of = |d: &BondedDesc| {
            let graph = spread_graph(d).expect("graph");
            let events = spread_cracks(&graph, graph.bond_count() / 3, 3 << 16, 4).expect("spread");
            (graph.bonds().to_vec(), events)
        };
        let want = spread_of(&desc);
        for k in 1..=8usize {
            let mut shuffled = desc.clone();
            let n = shuffled.bonds.len();
            shuffled.bonds.rotate_left(k * 13 % n);
            if k % 2 == 0 {
                shuffled.bonds.reverse();
            }
            let got = spread_of(&shuffled);
            assert_eq!(got.0, want.0, "order {k}: graph");
            assert_eq!(got.1.len(), want.1.len(), "order {k}: events");
        }
    }

    #[test]
    fn bad_lattices_refuse() {
        let s = spec();
        assert_eq!(
            ranked_grid(
                &RankSpec {
                    cells: [0, 1, 1],
                    ..s
                },
                &[]
            )
            .unwrap_err(),
            RankError::BadCells
        );
        assert_eq!(
            ranked_grid(
                &RankSpec {
                    cells: [16, 16, 16],
                    ..s
                },
                &[]
            )
            .unwrap_err(),
            RankError::BadCells
        );
        assert_eq!(
            ranked_grid(&RankSpec { reach_m: 0.0, ..s }, &[]).unwrap_err(),
            RankError::BadSpec
        );
        assert_eq!(
            ranked_grid(
                &RankSpec {
                    gain: f32::NAN,
                    ..s
                },
                &[]
            )
            .unwrap_err(),
            RankError::BadSpec
        );
        let nan = [Likely {
            at: [f32::NAN, 0.0, 0.0],
            weight: 1.0,
        }];
        assert_eq!(ranked_grid(&s, &nan).unwrap_err(), RankError::BadImpact);
        let hot = RankSpec {
            gain: 1000.0,
            reach_m: 0.05,
            cells: [40, 12, 2],
            ..s
        };
        let pile = [Likely {
            at: [0.0, 1.5, 0.0],
            weight: 1000.0,
        }];
        assert_eq!(ranked_grid(&hot, &pile).unwrap_err(), RankError::Thin);
    }
}
