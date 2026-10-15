//! bake3d: offline 3D pre-fracture bake. Seeded bisection, deterministic on (seed, pieces).

use engine_core::Rng;

/// Shipped chunk data: 3D box + mass. Plain data, no behaviour.
#[derive(Clone, Copy, Debug)]
pub struct Chunk3 {
    /// Center x.
    pub x: f32,
    /// Center y.
    pub y: f32,
    /// Center z.
    pub z: f32,
    /// Half extent x.
    pub hx: f32,
    /// Half extent y.
    pub hy: f32,
    /// Half extent z.
    pub hz: f32,
    /// Mass = hx * hy * hz * 8.0.
    pub mass: f32,
}

/// Bake `pieces` chunks from box (x, y, z, hx, hy, hz). Deterministic on (seed, pieces).
/// Offline only: call at bake time, ship the returned data.
/// Arity mirrors the 2D bake signature, extended with the z axis.
#[expect(
    clippy::too_many_arguments,
    reason = "wide signature kept until its inputs move into a params struct"
)]
pub fn fracture3(
    x: f32,
    y: f32,
    z: f32,
    hx: f32,
    hy: f32,
    hz: f32,
    pieces: usize,
    seed: u64,
) -> Vec<Chunk3> {
    let mut rng = Rng::new(seed);
    let mut boxes = vec![(x, y, z, hx, hy, hz)];
    // exactly pieces-1 splits: each split adds one chunk, so any count lands.
    for _ in 0..pieces.max(1).saturating_sub(1) {
        // split the largest-volume box along its longest axis at a seeded ratio
        let mut best = 0;
        for i in 1..boxes.len() {
            let (_, _, _, ax, ay, az) = boxes[best];
            let (_, _, _, bx, by, bz) = boxes[i];
            if bx * by * bz > ax * ay * az {
                best = i;
            }
        }
        let (cx, cy, cz, ex, ey, ez) = boxes.remove(best);
        let t = 0.35 + 0.3 * ((rng.next_u64() % 1000) as f32 / 1000.0);
        // tie-break is x, then y, then z so the axis choice is deterministic.
        if ex >= ey && ex >= ez {
            let cut = cx - ex + 2.0 * ex * t; // cut position on x axis
            let (l0, l1) = (cx - ex, cut);
            let (r0, r1) = (cut, cx + ex);
            boxes.push(((l0 + l1) / 2.0, cy, cz, (l1 - l0) / 2.0, ey, ez));
            boxes.push(((r0 + r1) / 2.0, cy, cz, (r1 - r0) / 2.0, ey, ez));
        } else if ey >= ez {
            let cut = cy - ey + 2.0 * ey * t; // cut position on y axis
            let (l0, l1) = (cy - ey, cut);
            let (r0, r1) = (cut, cy + ey);
            boxes.push((cx, (l0 + l1) / 2.0, cz, ex, (l1 - l0) / 2.0, ez));
            boxes.push((cx, (r0 + r1) / 2.0, cz, ex, (r1 - r0) / 2.0, ez));
        } else {
            let cut = cz - ez + 2.0 * ez * t; // cut position on z axis
            let (l0, l1) = (cz - ez, cut);
            let (r0, r1) = (cut, cz + ez);
            boxes.push((cx, cy, (l0 + l1) / 2.0, ex, ey, (l1 - l0) / 2.0));
            boxes.push((cx, cy, (r0 + r1) / 2.0, ex, ey, (r1 - r0) / 2.0));
        }
    }
    boxes
        .into_iter()
        .map(|(x, y, z, hx, hy, hz)| Chunk3 {
            x,
            y,
            z,
            hx,
            hy,
            hz,
            mass: hx * hy * hz * 8.0,
        })
        .collect()
}

/// Canonical bake bytes: LE to_bits of (x, y, z, hx, hy, hz, mass) per chunk, in order.
pub fn bake_bytes3(chunks: &[Chunk3]) -> Vec<u8> {
    let mut out = Vec::with_capacity(chunks.len() * 28);
    for c in chunks {
        for w in [
            c.x.to_bits(),
            c.y.to_bits(),
            c.z.to_bits(),
            c.hx.to_bits(),
            c.hy.to_bits(),
            c.hz.to_bits(),
            c.mass.to_bits(),
        ] {
            out.extend_from_slice(&w.to_le_bytes());
        }
    }
    out
}

/// Bake hash: engine_core hash over [`bake_bytes3`]. Same seed means same hash.
pub fn bake_hash3(chunks: &[Chunk3]) -> u64 {
    engine_core::hash_bytes(&bake_bytes3(chunks))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx_eq(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    #[test]
    fn same_seed_bit_identical() {
        let a = fracture3(0.0, 0.0, 0.0, 10.0, 5.0, 4.0, 8, 42);
        let b = fracture3(0.0, 0.0, 0.0, 10.0, 5.0, 4.0, 8, 42);
        assert_eq!(a.len(), 8);
        for (x, y) in a.iter().zip(b.iter()) {
            assert_eq!(
                (
                    x.x.to_bits(),
                    x.y.to_bits(),
                    x.z.to_bits(),
                    x.hx.to_bits(),
                    x.hy.to_bits(),
                    x.hz.to_bits(),
                    x.mass.to_bits()
                ),
                (
                    y.x.to_bits(),
                    y.y.to_bits(),
                    y.z.to_bits(),
                    y.hx.to_bits(),
                    y.hy.to_bits(),
                    y.hz.to_bits(),
                    y.mass.to_bits()
                )
            );
        }
    }

    #[test]
    fn chunk_count_matches_pieces() {
        for pieces in [1usize, 2, 3, 4, 7, 8, 12, 16] {
            assert_eq!(
                fracture3(0.0, 0.0, 0.0, 10.0, 5.0, 4.0, pieces, 7).len(),
                pieces
            );
        }
    }

    #[test]
    fn zero_and_one_piece_bake_whole_box() {
        for pieces in [0usize, 1] {
            let c = fracture3(1.0, 2.0, 3.0, 4.0, 3.0, 2.0, pieces, 99);
            assert_eq!(c.len(), 1);
            let ch = c[0];
            assert!(approx_eq(ch.x, 1.0) && approx_eq(ch.y, 2.0) && approx_eq(ch.z, 3.0));
            assert!(approx_eq(ch.hx, 4.0) && approx_eq(ch.hy, 3.0) && approx_eq(ch.hz, 2.0));
            assert!(approx_eq(ch.mass, 192.0));
        }
    }

    #[test]
    fn mass_conserved_and_chunks_inside() {
        let (x, y, z, hx, hy, hz) = (3.0f32, -2.0, 1.0, 8.0, 6.0, 4.0);
        let total = hx * hy * hz * 8.0;
        for pieces in [1usize, 2, 5, 9, 16] {
            let c = fracture3(x, y, z, hx, hy, hz, pieces, 1234);
            assert_eq!(c.len(), pieces);
            let mut sum = 0.0;
            for ch in &c {
                assert!(ch.hx > 0.0 && ch.hy > 0.0 && ch.hz > 0.0 && ch.mass > 0.0);
                assert!(approx_eq(ch.mass, ch.hx * ch.hy * ch.hz * 8.0));
                assert!(ch.x - ch.hx >= x - hx - 1e-4 && ch.x + ch.hx <= x + hx + 1e-4);
                assert!(ch.y - ch.hy >= y - hy - 1e-4 && ch.y + ch.hy <= y + hy + 1e-4);
                assert!(ch.z - ch.hz >= z - hz - 1e-4 && ch.z + ch.hz <= z + hz + 1e-4);
                sum += ch.mass;
            }
            // F32 bisection rounding scales with volume, so the sum bound is looser.
            assert!((sum - total).abs() < 1e-2, "sum {sum} total {total}");
        }
    }

    #[test]
    fn bake_sha_stable_x2() {
        let a = fracture3(0.0, 0.0, 0.0, 10.0, 5.0, 4.0, 8, 42);
        let b = fracture3(0.0, 0.0, 0.0, 10.0, 5.0, 4.0, 8, 42);
        assert_eq!(bake_bytes3(&a), bake_bytes3(&b));
        assert_eq!(bake_hash3(&a), bake_hash3(&b));
        assert_ne!(
            bake_hash3(&a),
            bake_hash3(&fracture3(0.0, 0.0, 0.0, 10.0, 5.0, 4.0, 8, 100))
        );
    }
}
