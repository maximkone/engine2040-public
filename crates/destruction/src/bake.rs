//! bake: offline pre-fracture bake. Seeded bisection, deterministic on (seed, pieces).

use engine_core::{Fnv64, Rng};

/// Shipped chunk data: 2D box + mass. Plain data, no behaviour.
#[derive(Clone, Copy, Debug)]
pub struct Chunk {
    /// Center x.
    pub x: f32,
    /// Center y.
    pub y: f32,
    /// Half extent x.
    pub hx: f32,
    /// Half extent y.
    pub hy: f32,
    /// Mass = hx * hy * 4.0.
    pub mass: f32,
}

/// Bake `pieces` chunks from box (x, y, hx, hy). Deterministic on (seed, pieces).
/// Offline only: call at bake time, ship the returned data.
pub fn fracture(x: f32, y: f32, hx: f32, hy: f32, pieces: usize, seed: u64) -> Vec<Chunk> {
    let mut rng = Rng::new(seed);
    let mut rects = vec![(x, y, hx, hy)];
    // exactly pieces-1 splits: each split adds one chunk, so any count lands.
    for _ in 0..pieces.max(1).saturating_sub(1) {
        // split the largest-area rect along its longer axis at a seeded ratio
        let mut best = 0;
        for i in 1..rects.len() {
            let (_, _, ax, ay) = rects[best];
            let (_, _, bx, by) = rects[i];
            if bx * by > ax * ay {
                best = i;
            }
        }
        let (cx, cy, hw, hh) = rects.remove(best);
        let t = 0.35 + 0.3 * ((rng.next_u64() % 1000) as f32 / 1000.0);
        if hw >= hh {
            let cut = cx - hw + 2.0 * hw * t; // cut position on x axis
            let (l0, l1) = (cx - hw, cut);
            let (r0, r1) = (cut, cx + hw);
            rects.push(((l0 + l1) / 2.0, cy, (l1 - l0) / 2.0, hh));
            rects.push(((r0 + r1) / 2.0, cy, (r1 - r0) / 2.0, hh));
        } else {
            let cut = cy - hh + 2.0 * hh * t; // cut position on y axis
            let (l0, l1) = (cy - hh, cut);
            let (r0, r1) = (cut, cy + hh);
            rects.push((cx, (l0 + l1) / 2.0, hw, (l1 - l0) / 2.0));
            rects.push((cx, (r0 + r1) / 2.0, hw, (r1 - r0) / 2.0));
        }
    }
    rects
        .into_iter()
        .map(|(x, y, hx, hy)| Chunk {
            x,
            y,
            hx,
            hy,
            mass: hx * hy * 4.0,
        })
        .collect()
}

/// Canonical bake bytes: LE to_bits of (x, y, hx, hy, mass) per chunk, in order.
pub fn bake_bytes(chunks: &[Chunk]) -> Vec<u8> {
    let mut out = Vec::with_capacity(chunks.len() * 20);
    for c in chunks {
        for w in [
            c.x.to_bits(),
            c.y.to_bits(),
            c.hx.to_bits(),
            c.hy.to_bits(),
            c.mass.to_bits(),
        ] {
            out.extend_from_slice(&w.to_le_bytes());
        }
    }
    out
}

/// Bake hash: engine_core hash over [`bake_bytes`]. Same seed means same hash.
pub fn bake_hash(chunks: &[Chunk]) -> u64 {
    engine_core::hash_bytes(&bake_bytes(chunks))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx_eq(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    #[test]
    fn same_seed_bit_identical() {
        let a = fracture(0.0, 0.0, 10.0, 5.0, 8, 42);
        let b = fracture(0.0, 0.0, 10.0, 5.0, 8, 42);
        assert_eq!(a.len(), 8);
        for (x, y) in a.iter().zip(b.iter()) {
            assert_eq!(
                (
                    x.x.to_bits(),
                    x.y.to_bits(),
                    x.hx.to_bits(),
                    x.hy.to_bits(),
                    x.mass.to_bits()
                ),
                (
                    y.x.to_bits(),
                    y.y.to_bits(),
                    y.hx.to_bits(),
                    y.hy.to_bits(),
                    y.mass.to_bits()
                )
            );
        }
    }

    #[test]
    fn chunk_count_matches_pieces() {
        for pieces in [1usize, 2, 3, 4, 7, 8, 12, 16] {
            assert_eq!(fracture(0.0, 0.0, 10.0, 5.0, pieces, 7).len(), pieces);
        }
    }

    #[test]
    fn zero_and_one_piece_bake_whole_box() {
        for pieces in [0usize, 1] {
            let c = fracture(1.0, 2.0, 4.0, 3.0, pieces, 99);
            assert_eq!(c.len(), 1);
            let ch = c[0];
            assert!(approx_eq(ch.x, 1.0) && approx_eq(ch.y, 2.0));
            assert!(approx_eq(ch.hx, 4.0) && approx_eq(ch.hy, 3.0));
            assert!(approx_eq(ch.mass, 48.0));
        }
    }

    #[test]
    fn mass_conserved_and_chunks_inside() {
        let (x, y, hx, hy) = (3.0f32, -2.0, 8.0, 6.0);
        let total = hx * hy * 4.0;
        for pieces in [1usize, 2, 5, 9, 16] {
            let c = fracture(x, y, hx, hy, pieces, 1234);
            assert_eq!(c.len(), pieces);
            let mut sum = 0.0;
            for ch in &c {
                assert!(ch.hx > 0.0 && ch.hy > 0.0 && ch.mass > 0.0);
                assert!(approx_eq(ch.mass, ch.hx * ch.hy * 4.0));
                assert!(ch.x - ch.hx >= x - hx - 1e-4 && ch.x + ch.hx <= x + hx + 1e-4);
                assert!(ch.y - ch.hy >= y - hy - 1e-4 && ch.y + ch.hy <= y + hy + 1e-4);
                sum += ch.mass;
            }
            assert!(approx_eq(sum, total), "sum {sum} total {total}");
        }
    }

    #[test]
    fn bake_sha_stable_x2() {
        let a = fracture(0.0, 0.0, 10.0, 5.0, 8, 42);
        let b = fracture(0.0, 0.0, 10.0, 5.0, 8, 42);
        assert_eq!(bake_bytes(&a), bake_bytes(&b));
        assert_eq!(bake_hash(&a), bake_hash(&b));
        assert_ne!(
            bake_hash(&a),
            bake_hash(&fracture(0.0, 0.0, 10.0, 5.0, 8, 100))
        );
    }
}

/// Most shards one bake emits; bounds memory up front, pieces clamp to this.
pub const SHARD_CAP: usize = 12;
/// Impulse per unit mass above which a hit breaks; shard mass units.
pub const BREAKING_THRESHOLD: f32 = 3.0;
/// Contact slack when two chunk boxes count as touching; meters.
pub const CONTACT_MARGIN: f32 = 0.04;
const RES: usize = 64;

/// 7 fracture patterns as data (seeded, trig-free, deterministic).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Voronoi,
    Radial,
    SlabX,
    SlabY,
    Grid,
    Shell,
    Splinter,
}

/// Bake inputs: center plus half extents in meters, piece count, seed, mode.
/// Determinism: same inputs give the same sites; invalid pieces clamp later.
#[derive(Clone, Copy, Debug)]
pub struct Params {
    pub x: f32,
    pub y: f32,
    pub hx: f32,
    pub hy: f32,
    pub pieces: usize,
    pub seed: u64,
    pub mode: Mode,
}

/// Baked shards plus sites and contact edges; shipped to the runtime.
/// Units: meters for boxes, mass units for total; deterministic on inputs.
#[derive(Clone, Debug)]
pub struct Bake {
    pub shards: Vec<Chunk>,
    pub sites: Vec<[f32; 2]>,
    pub edges: Vec<(u16, u16)>,
    pub mode: Mode,
    pub seed: u64,
    pub total_mass: f32,
}

/// Impulse that breaks `mass`; shard mass units, deterministic.
pub fn breaking_impulse(mass: f32) -> f32 {
    mass * BREAKING_THRESHOLD
}
/// True when `impulse` breaks `mass`; pure compare, never panics.
pub fn should_break(impulse: f32, mass: f32) -> bool {
    impulse > breaking_impulse(mass)
}

fn gen_sites(mode: Mode, x: f32, y: f32, hx: f32, hy: f32, n: usize, seed: u64) -> Vec<[f32; 2]> {
    let tag = (mode as u64).wrapping_mul(0x9e3779b97f4a7c15);
    let mut rng = Rng::new(seed ^ tag ^ 0x243f6a8885a308d3u64);
    let mut fr = || (rng.next_u64() % 10_000) as f32 / 10_000.0;
    let c = |v: f32| v - 0.5;
    let mut o = Vec::with_capacity(n);
    match mode {
        Mode::Voronoi => {
            for _ in 0..n {
                o.push([x - hx + 2.0 * hx * fr(), y - hy + 2.0 * hy * fr()]);
            }
        }
        Mode::Radial => {
            o.push([x, y]);
            for k in 1..n {
                let per = (n + 2) / 4;
                let t = ((k - 1) / 4) as f32 / per.max(1) as f32 * 2.0 - 1.0;
                let (ox, oy) = match (k - 1) % 4 {
                    0 => (t, 1.0),
                    1 => (t, -1.0),
                    2 => (1.0, t),
                    _ => (-1.0, t),
                };
                o.push([
                    x + ox * hx * 0.7 + c(fr()) * hx * 0.2,
                    y + oy * hy * 0.7 + c(fr()) * hy * 0.2,
                ]);
            }
        }
        Mode::SlabX => {
            for i in 0..n {
                let t = (i as f32 + 0.5 + c(fr()) * 0.6) / n as f32;
                o.push([x - hx + 2.0 * hx * t, y + c(fr()) * hy * 0.3]);
            }
        }
        Mode::SlabY => {
            for i in 0..n {
                let t = (i as f32 + 0.5 + c(fr()) * 0.6) / n as f32;
                o.push([x + c(fr()) * hx * 0.3, y - hy + 2.0 * hy * t]);
            }
        }
        Mode::Grid => {
            let mut g = 1;
            while g * g < n {
                g += 1;
            }
            for i in 0..n {
                o.push([
                    x - hx + 2.0 * hx * ((i % g) as f32 + 0.5 + c(fr()) * 0.5) / g as f32,
                    y - hy + 2.0 * hy * ((i / g) as f32 + 0.5 + c(fr()) * 0.5) / g as f32,
                ]);
            }
        }
        Mode::Shell => {
            let lv = [0.0f32, 0.5, 0.85];
            for i in 0..n {
                let l = lv[i % 3];
                let f = c(fr()) * 2.0;
                let (ox, oy) = match (i / 3) % 4 {
                    0 => (f, l),
                    1 => (f, -l),
                    2 => (l, f),
                    _ => (-l, f),
                };
                o.push([
                    x + ox * hx + c(fr()) * hx * 0.1,
                    y + oy * hy + c(fr()) * hy * 0.1,
                ]);
            }
        }
        Mode::Splinter => {
            let m = hx.min(hy);
            for i in 0..n {
                let t = (i as f32 + 0.5) / n as f32;
                o.push([
                    x - hx + 2.0 * hx * t,
                    y - hy + 2.0 * hy * t + c(fr()) * m * 0.4,
                ]);
            }
        }
    }
    o
}

/// Adjacent label pairs of the `RES x RES` raster (right and down neighbors),
/// each as `(min, max)`, sorted and deduplicated so the edge list is canonical.
fn label_edges(label: &[u16]) -> Vec<(u16, u16)> {
    let mut edges: Vec<(u16, u16)> = Vec::new();
    for iy in 0..RES {
        for ix in 0..RES {
            let a = label[iy * RES + ix];
            if ix + 1 < RES {
                let b = label[iy * RES + ix + 1];
                if a != b {
                    edges.push((a.min(b), a.max(b)));
                }
            }
            if iy + 1 < RES {
                let b = label[(iy + 1) * RES + ix];
                if a != b {
                    edges.push((a.min(b), a.max(b)));
                }
            }
        }
    }
    edges.sort_unstable();
    edges.dedup();
    edges
}

/// Bakes `p` into shards; pieces clamp to `1..=SHARD_CAP`.
/// Units: meters in, mass units out. Determinism: same `p` gives same bake.
pub fn bake(p: &Params) -> Bake {
    let n = p.pieces.clamp(1, SHARD_CAP);
    let sites = gen_sites(p.mode, p.x, p.y, p.hx, p.hy, n, p.seed);
    let mut label = vec![0u16; RES * RES];
    let mut cnt = vec![0u32; n];
    let mut bb = vec![(RES, 0usize, RES, 0usize); n];
    for iy in 0..RES {
        let py = p.y - p.hy + 2.0 * p.hy * (iy as f32 + 0.5) / RES as f32;
        for ix in 0..RES {
            let px = p.x - p.hx + 2.0 * p.hx * (ix as f32 + 0.5) / RES as f32;
            let (mut bi, mut bd) = (0usize, f32::MAX);
            for (i, s) in sites.iter().enumerate() {
                let (dx, dy) = (px - s[0], py - s[1]);
                let d = dx * dx + dy * dy;
                if d < bd {
                    bd = d;
                    bi = i;
                }
            }
            label[iy * RES + ix] = bi as u16;
            cnt[bi] += 1;
            let b = &mut bb[bi];
            b.0 = b.0.min(ix);
            b.1 = b.1.max(ix);
            b.2 = b.2.min(iy);
            b.3 = b.3.max(iy);
        }
    }
    let edges = label_edges(&label);
    let total = p.hx * p.hy * 4.0;
    let tc = (RES * RES) as f32;
    let mut remap = vec![u16::MAX; n];
    let (mut shards, mut kept) = (Vec::new(), Vec::new());
    for (i, &k) in cnt.iter().enumerate() {
        if k == 0 {
            continue;
        }
        let (x0, x1, y0, y1) = bb[i];
        let ax0 = p.x - p.hx + 2.0 * p.hx * x0 as f32 / RES as f32;
        let ax1 = p.x - p.hx + 2.0 * p.hx * (x1 + 1) as f32 / RES as f32;
        let ay0 = p.y - p.hy + 2.0 * p.hy * y0 as f32 / RES as f32;
        let ay1 = p.y - p.hy + 2.0 * p.hy * (y1 + 1) as f32 / RES as f32;
        remap[i] = shards.len() as u16;
        shards.push(Chunk {
            x: (ax0 + ax1) / 2.0,
            y: (ay0 + ay1) / 2.0,
            hx: (ax1 - ax0) / 2.0,
            hy: (ay1 - ay0) / 2.0,
            mass: total * k as f32 / tc,
        });
        kept.push(sites[i]);
    }
    if shards.is_empty() {
        shards.push(Chunk {
            x: p.x,
            y: p.y,
            hx: p.hx,
            hy: p.hy,
            mass: total,
        });
        kept.push([p.x, p.y]);
        remap[0] = 0;
    }
    let sum: f32 = shards.iter().map(|s| s.mass).sum();
    if let Some(l) = shards.last_mut() {
        l.mass += total - sum;
    }
    let mut e2: Vec<(u16, u16)> = edges
        .into_iter()
        .filter_map(|(a, b)| {
            let (na, nb) = (remap[a as usize], remap[b as usize]);
            if na == u16::MAX || nb == u16::MAX || na == nb {
                None
            } else {
                Some((na.min(nb), na.max(nb)))
            }
        })
        .collect();
    e2.sort_unstable();
    e2.dedup();
    Bake {
        shards,
        sites: kept,
        edges: e2,
        mode: p.mode,
        seed: p.seed,
        total_mass: total,
    }
}

/// Digest over a mode bake (distinct from [`bake_hash`] over raw chunks).
pub fn bake_digest(b: &Bake) -> u64 {
    let mut h = Fnv64::new();
    h.write_u64(b.seed);
    h.write_u32(b.mode as u32);
    h.write_len32(b.shards.len());
    for s in &b.shards {
        for w in [
            s.x.to_bits(),
            s.y.to_bits(),
            s.hx.to_bits(),
            s.hy.to_bits(),
            s.mass.to_bits(),
        ] {
            h.write_u32(w);
        }
    }
    for s in &b.sites {
        h.write_u32(s[0].to_bits());
        h.write_u32(s[1].to_bits());
    }
    h.write_len32(b.edges.len());
    for (a, c) in &b.edges {
        h.write_u32(u32::from(*a));
        h.write_u32(u32::from(*c));
    }
    h.finish()
}

/// Neighbor shard ids of `id` in ascending order; empty when out of range.
/// Allocates: analysis only, never the tick.
pub fn neighbors(b: &Bake, id: usize) -> Vec<usize> {
    let mut o = Vec::new();
    for (a, c) in &b.edges {
        if *a as usize == id {
            o.push(*c as usize);
        } else if *c as usize == id {
            o.push(*a as usize);
        }
    }
    o.sort_unstable();
    o
}

#[cfg(test)]
mod mode_tests {
    use super::*;
    const MODES: [Mode; 7] = [
        Mode::Voronoi,
        Mode::Radial,
        Mode::SlabX,
        Mode::SlabY,
        Mode::Grid,
        Mode::Shell,
        Mode::Splinter,
    ];
    fn par(mode: Mode, pieces: usize, seed: u64) -> Params {
        Params {
            x: 3.0,
            y: -2.0,
            hx: 8.0,
            hy: 6.0,
            pieces,
            seed,
            mode,
        }
    }
    #[test]
    fn hash_x2() {
        for m in MODES {
            let (a, b) = (bake(&par(m, 8, 7)), bake(&par(m, 8, 7)));
            assert_eq!(bake_digest(&a), bake_digest(&b));
            assert_eq!(a.edges, b.edges);
        }
    }
    #[test]
    fn bake_digest_is_fnv1a_over_the_canonical_word_stream() {
        let b = bake(&par(Mode::Voronoi, 8, 0x0123_4567_89ab_cdef));
        // Oracle: seed low then high u32, mode, shard count and boxes, sites,
        // edge count and endpoints, as plain LE bytes.
        let mut words: Vec<u32> = vec![
            b.seed as u32,
            (b.seed >> 32) as u32,
            b.mode as u32,
            b.shards.len() as u32,
        ];
        for s in &b.shards {
            words.extend([
                s.x.to_bits(),
                s.y.to_bits(),
                s.hx.to_bits(),
                s.hy.to_bits(),
                s.mass.to_bits(),
            ]);
        }
        for s in &b.sites {
            words.extend([s[0].to_bits(), s[1].to_bits()]);
        }
        words.push(b.edges.len() as u32);
        for (a, c) in &b.edges {
            words.extend([u32::from(*a), u32::from(*c)]);
        }
        let bytes: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
        assert!(!b.edges.is_empty(), "the oracle must cover edges");
        assert_eq!(bake_digest(&b), engine_core::hash_bytes(&bytes));
    }
    #[test]
    fn mass_conserved_inside() {
        for m in MODES {
            for n in [1usize, 5, 12] {
                let b = bake(&par(m, n, 99));
                let tot = 8.0 * 6.0 * 4.0;
                assert!(
                    (b.shards.iter().map(|s| s.mass).sum::<f32>() - tot).abs() < 1e-2,
                    "{m:?}"
                );
                for s in &b.shards {
                    assert!(s.hx > 0.0 && s.hy > 0.0 && s.mass > 0.0);
                    assert!(s.x - s.hx >= 3.0 - 8.0 - 1e-3 && s.x + s.hx <= 3.0 + 8.0 + 1e-3);
                }
            }
        }
    }
    #[test]
    fn adjacency_symmetric() {
        let b = bake(&par(Mode::Voronoi, 9, 5));
        for (a, c) in &b.edges {
            assert!(a < c);
            assert!((*a as usize) < b.shards.len() && (*c as usize) < b.shards.len());
            assert!(neighbors(&b, *a as usize).contains(&(*c as usize)));
            assert!(neighbors(&b, *c as usize).contains(&(*a as usize)));
        }
    }
    #[test]
    fn cap_and_thresholds() {
        assert_eq!(bake(&par(Mode::Voronoi, 64, 1)).shards.len(), SHARD_CAP);
        assert!(
            should_break(3.01, 1.0)
                && !should_break(2.99, 1.0)
                && (breaking_impulse(2.0) - 6.0).abs() < 1e-6
        );
    }
}
