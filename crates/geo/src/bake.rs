//! bake: object-space normal map from low UV to high surface via parry rays.
use crate::mesh::{ix, narrow, GeoError, GeoMesh};
use parry3d::math::Vector;
use parry3d::query::{Ray, RayCast};
use parry3d::shape::TriMesh;
/// Object-space normal map, row-major texels.
#[derive(Clone, Debug, PartialEq)]
pub struct NormalMap {
    /// Edge length in texels.
    pub size: u32,
    /// Hit normal per texel as `(0.5+0.5*n)*255` rounded.
    pub rgb: Vec<[u8; 3]>,
    /// True when baked or dilated.
    pub covered: Vec<bool>,
}
fn unit(v: [f32; 3]) -> Option<[f32; 3]> {
    let l = (f64::from(v[0]) * f64::from(v[0])
        + f64::from(v[1]) * f64::from(v[1])
        + f64::from(v[2]) * f64::from(v[2]))
    .sqrt();
    if l < 1e-12 {
        return None;
    }
    let i = 1.0 / l;
    Some([
        narrow(f64::from(v[0]) * i),
        narrow(f64::from(v[1]) * i),
        narrow(f64::from(v[2]) * i),
    ])
}
fn byte_of(x: f32) -> u8 {
    let c = ((0.5f32 + 0.5f32 * x) * 255.0f32).round().clamp(0.0, 255.0);
    if !c.is_finite() {
        return 128;
    }
    let e = (c.to_bits() >> 23) & 0xff;
    if e < 127 {
        return 0;
    }
    u8::try_from(((1u32 << 23) | (c.to_bits() & 0x7fffff)) >> 150u32.wrapping_sub(e)).unwrap_or(255)
}
fn bary(p: [f64; 2], a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> Option<[f64; 3]> {
    let d = (b[1] - c[1]) * (a[0] - c[0]) + (c[0] - b[0]) * (a[1] - c[1]);
    if d.abs() < 1e-12 {
        return None;
    }
    let w0 = ((b[1] - c[1]) * (p[0] - c[0]) + (c[0] - b[0]) * (p[1] - c[1])) / d;
    let w1 = ((c[1] - a[1]) * (p[0] - c[0]) + (a[0] - c[0]) * (p[1] - c[1])) / d;
    let w2 = 1.0 - w0 - w1;
    if w0 < -1e-9 || w1 < -1e-9 || w2 < -1e-9 {
        return None;
    }
    Some([w0, w1, w2])
}
/// Pixel-center UV of texel `(x, y)` for a map of width `1 / inv`.
/// Caller: [`bake_normals`] (same file) walks every texel through here.
fn texel_uv(x: usize, y: usize, inv: f64) -> [f64; 2] {
    [
        (f64::from(u32::try_from(x).unwrap_or(u32::MAX)) + 0.5) * inv,
        (f64::from(u32::try_from(y).unwrap_or(u32::MAX)) + 0.5) * inv,
    ]
}

/// Shot direction for one texel of triangle `tri` (three vertex indices) at barycentric `w`:
/// blended vertex normals when the low mesh carries them, else the triangle face normal;
/// `None` when the triangle is degenerate and the texel must be skipped.
/// Caller: [`bake_normals`] (same file) casts one parry ray per texel.
fn texel_dir(low: &GeoMesh, tri: [usize; 3], w: [f64; 3]) -> Option<[f32; 3]> {
    let [ia, ib, ic] = tri;
    if low.normals.len() == low.positions.len() {
        let (na, nb, nc) = (low.normals[ia], low.normals[ib], low.normals[ic]);
        let v = [
            narrow(w[0] * f64::from(na[0]) + w[1] * f64::from(nb[0]) + w[2] * f64::from(nc[0])),
            narrow(w[0] * f64::from(na[1]) + w[1] * f64::from(nb[1]) + w[2] * f64::from(nc[1])),
            narrow(w[0] * f64::from(na[2]) + w[1] * f64::from(nb[2]) + w[2] * f64::from(nc[2])),
        ];
        unit(v)
    } else {
        let (pa, pb, pc) = (low.positions[ia], low.positions[ib], low.positions[ic]);
        let e1 = [pb[0] - pa[0], pb[1] - pa[1], pb[2] - pa[2]];
        let e2 = [pc[0] - pa[0], pc[1] - pa[1], pc[2] - pa[2]];
        unit([
            e1[1] * e2[2] - e1[2] * e2[1],
            e1[2] * e2[0] - e1[0] * e2[2],
            e1[0] * e2[1] - e1[1] * e2[0],
        ])
    }
}

/// Spread baked texels up to 4px into uncovered neighbors, nearest first.
/// Caller: [`bake_normals`] (same file) dilates the baked map in place.
fn dilate_cover(
    n: usize,
    total: usize,
    si: i32,
    sc: &[bool],
    sr: &[[u8; 3]],
    rgb: &mut [[u8; 3]],
    covered: &mut [bool],
) {
    for y in 0..n {
        for x in 0..n {
            if sc[y * n + x] {
                continue;
            }
            let (xi, yi) = (i32::try_from(x).unwrap_or(0), i32::try_from(y).unwrap_or(0));
            let (mut bd, mut bj) = (i32::MAX, usize::MAX);
            for dy in -4..=4 {
                for dx in -4..=4 {
                    let (nx, ny) = (xi + dx, yi + dy);
                    if nx < 0 || ny < 0 || nx >= si || ny >= si {
                        continue;
                    }
                    let j = usize::try_from(ny).unwrap_or(usize::MAX) * n
                        + usize::try_from(nx).unwrap_or(usize::MAX);
                    if j >= total || !sc[j] {
                        continue;
                    }
                    let d = dx * dx + dy * dy;
                    if d < bd || (d == bd && j < bj) {
                        bd = d;
                        bj = j;
                    }
                }
            }
            if bj != usize::MAX {
                rgb[y * n + x] = sr[bj];
                covered[y * n + x] = true;
            }
        }
    }
}
/// Largest bake edge accepted, texels. The cost is one barycentric test per
/// texel per low triangle, so callers budget the product themselves; this cap
/// only keeps one map under 1 Mi texels.
pub const MAX_BAKE_EDGE: u32 = 1024;

/// Bake `high` normals into `low` UV space, then dilate covered texels 4px.
pub fn bake_normals(
    low: &GeoMesh,
    high: &GeoMesh,
    size: u32,
    cage: f32,
) -> Result<NormalMap, GeoError> {
    low.validate()?;
    high.validate()?;
    if low.indices.is_empty() || high.indices.is_empty() {
        return Err(GeoError::Empty);
    }
    if low.uvs.len() != low.positions.len() {
        return Err(GeoError::Malformed("uvs missing"));
    }
    if size == 0 || size > MAX_BAKE_EDGE {
        return Err(GeoError::Malformed("size out of range"));
    }
    if !cage.is_finite() || cage <= 0.0 {
        return Err(GeoError::Malformed("cage out of range"));
    }
    let n = usize::try_from(size).unwrap_or(0);
    let total = n.checked_mul(n).unwrap_or(0);
    let verts = high
        .positions
        .iter()
        .map(|p| Vector::new(p[0], p[1], p[2]))
        .collect::<Vec<_>>();
    let idxs = high
        .indices
        .chunks_exact(3)
        .map(|t| [t[0], t[1], t[2]])
        .collect::<Vec<_>>();
    let mesh =
        TriMesh::new(verts, idxs).map_err(|_| GeoError::Donor("trimesh build".to_string()))?;
    let tris = low.indices.len() / 3;
    let mut rgb = vec![[0u8; 3]; total];
    let mut covered = vec![false; total];
    let inv = 1.0 / f64::from(size);
    for y in 0..n {
        for x in 0..n {
            let p = texel_uv(x, y, inv);
            for t in 0..tris {
                let (ia, ib, ic) = (
                    ix(low.indices[3 * t]),
                    ix(low.indices[3 * t + 1]),
                    ix(low.indices[3 * t + 2]),
                );
                let w = bary(
                    p,
                    [f64::from(low.uvs[ia][0]), f64::from(low.uvs[ia][1])],
                    [f64::from(low.uvs[ib][0]), f64::from(low.uvs[ib][1])],
                    [f64::from(low.uvs[ic][0]), f64::from(low.uvs[ic][1])],
                );
                let Some(w) = w else { continue };
                let (pa, pb, pc) = (low.positions[ia], low.positions[ib], low.positions[ic]);
                let pos = [
                    narrow(
                        w[0] * f64::from(pa[0]) + w[1] * f64::from(pb[0]) + w[2] * f64::from(pc[0]),
                    ),
                    narrow(
                        w[0] * f64::from(pa[1]) + w[1] * f64::from(pb[1]) + w[2] * f64::from(pc[1]),
                    ),
                    narrow(
                        w[0] * f64::from(pa[2]) + w[1] * f64::from(pb[2]) + w[2] * f64::from(pc[2]),
                    ),
                ];
                let Some(dir) = texel_dir(low, [ia, ib, ic], w) else {
                    break;
                };
                let o = Vector::new(
                    pos[0] + dir[0] * cage,
                    pos[1] + dir[1] * cage,
                    pos[2] + dir[2] * cage,
                );
                let r = Ray::new(o, Vector::new(-dir[0], -dir[1], -dir[2]));
                if let Some(h) = mesh.cast_local_ray_and_get_normal(&r, cage * 2.0, true) {
                    if let Some(v) = unit([h.normal.x, h.normal.y, h.normal.z]) {
                        rgb[y * n + x] = [byte_of(v[0]), byte_of(v[1]), byte_of(v[2])];
                        covered[y * n + x] = true;
                    }
                }
                break;
            }
        }
    }
    let (sc, sr) = (covered.clone(), rgb.clone());
    let si = i32::try_from(size).unwrap_or(0);
    dilate_cover(n, total, si, &sc, &sr, &mut rgb, &mut covered);
    Ok(NormalMap { size, rgb, covered })
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::primitives::uv_sphere;
    #[test]
    fn bake_rejects_bad_inputs() {
        let m = uv_sphere(1.0, 8, 12);
        assert_eq!(
            bake_normals(&GeoMesh::default(), &m, 32, 0.2),
            Err(GeoError::Empty)
        );
        assert_eq!(
            bake_normals(&m, &m, 0, 0.2),
            Err(GeoError::Malformed("size out of range"))
        );
        assert_eq!(
            bake_normals(&m, &m, 32, 0.0),
            Err(GeoError::Malformed("cage out of range"))
        );
    }
    #[test]
    fn texel_uv_names_pixel_centers() {
        assert_eq!(texel_uv(0, 0, 0.25), [0.125, 0.125]);
        assert_eq!(texel_uv(3, 1, 0.25), [0.875, 0.375]);
    }
    #[test]
    fn texel_dir_prefers_vertex_normals_then_face_normal() {
        let mesh = GeoMesh {
            positions: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            normals: vec![],
            uvs: vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]],
            indices: vec![0, 1, 2],
        };
        let w = [0.25, 0.25, 0.5];
        assert_eq!(
            texel_dir(&mesh, [0, 1, 2], w),
            Some([0.0, 0.0, 1.0]),
            "face normal of an XY triangle points +z"
        );
        let with_n = GeoMesh {
            normals: vec![[0.0, 0.0, 1.0]; 3],
            ..mesh.clone()
        };
        assert_eq!(
            texel_dir(&with_n, [0, 1, 2], w),
            Some([0.0, 0.0, 1.0]),
            "blended vertex normals point +z"
        );
        let flat = GeoMesh {
            positions: vec![[1.0, 2.0, 3.0]; 3],
            ..mesh.clone()
        };
        assert_eq!(
            texel_dir(&flat, [0, 1, 2], w),
            None,
            "a degenerate triangle has no direction"
        );
    }
    #[test]
    fn dilate_cover_fills_texels_within_four_px() {
        let n = 5usize;
        let total = n * n;
        let si = 5i32;
        let mut sc = vec![false; total];
        let mut sr = vec![[0u8; 3]; total];
        sc[2 * n + 2] = true;
        sr[2 * n + 2] = [255, 0, 0];
        let mut rgb = vec![[0u8; 3]; total];
        let mut covered = vec![false; total];
        rgb[2 * n + 2] = [255, 0, 0];
        covered[2 * n + 2] = true;
        dilate_cover(n, total, si, &sc, &sr, &mut rgb, &mut covered);
        assert!(
            covered.iter().all(|&c| c),
            "every 5x5 texel sits within 4px of the center"
        );
        assert!(rgb.iter().all(|&p| p == [255, 0, 0]));
        let n2 = 11usize;
        let total2 = n2 * n2;
        let mut sc2 = vec![false; total2];
        let mut sr2 = vec![[0u8; 3]; total2];
        sc2[5 * n2 + 5] = true;
        sr2[5 * n2 + 5] = [0, 255, 0];
        let mut rgb2 = vec![[0u8; 3]; total2];
        let mut covered2 = vec![false; total2];
        rgb2[5 * n2 + 5] = [0, 255, 0];
        covered2[5 * n2 + 5] = true;
        dilate_cover(n2, total2, 11, &sc2, &sr2, &mut rgb2, &mut covered2);
        assert!(
            !covered2[0],
            "a corner 5px out of reach must stay uncovered"
        );
        assert_eq!(rgb2[0], [0, 0, 0]);
    }

    /// Bake a 48 x 64 unit sphere into `low` at 256 px and check the sphere accept: at least
    /// 90 percent of the texels inside a UV triangle are covered, the covered texels' normals
    /// are within 3 degrees (mean) of the analytic sphere normal at the texel's surface
    /// point, the bytes repeat, and the bake takes at most 5 s.
    fn check_sphere_bake(low: &GeoMesh) {
        const SIZE: usize = 256;
        let high = uv_sphere(1.0, 48, 64);
        let start = std::time::Instant::now();
        let map = bake_normals(low, &high, 256, 0.2).unwrap();
        let seconds = start.elapsed().as_secs_f64();
        assert!(seconds <= 5.0, "bake took {seconds} s");
        let bytes = |m: &NormalMap| m.rgb.iter().flatten().copied().collect::<Vec<u8>>();
        let again = bake_normals(low, &high, 256, 0.2).unwrap();
        assert_eq!(
            engine_core::hash_bytes(&bytes(&map)),
            engine_core::hash_bytes(&bytes(&again)),
            "digest of rgb equal x2"
        );
        let inv = 1.0 / f64::from(map.size);
        let (mut inside, mut covered, mut angle_sum) = (0_u32, 0_u32, 0.0_f64);
        for y in 0..SIZE {
            for x in 0..SIZE {
                let p = texel_uv(x, y, inv);
                let found = low.indices.chunks_exact(3).find_map(|t| {
                    let uv = |k: usize| low.uvs[ix(t[k])].map(f64::from);
                    bary(p, uv(0), uv(1), uv(2)).map(|w| (t, w))
                });
                let Some((tri, w)) = found else { continue };
                inside += 1;
                if !map.covered[y * SIZE + x] {
                    continue;
                }
                covered += 1;
                let pos: [f64; 3] = std::array::from_fn(|k| {
                    (0..3)
                        .map(|j| w[j] * f64::from(low.positions[ix(tri[j])][k]))
                        .sum()
                });
                let len = pos.iter().map(|v| v * v).sum::<f64>().sqrt();
                let got = map.rgb[y * SIZE + x].map(|b| f64::from(b) / 255.0 * 2.0 - 1.0);
                let got_len = got.iter().map(|v| v * v).sum::<f64>().sqrt();
                let cos = (0..3).map(|k| pos[k] / len * got[k] / got_len).sum::<f64>();
                angle_sum += cos.clamp(-1.0, 1.0).acos().to_degrees();
            }
        }
        let coverage = f64::from(covered) / f64::from(inside);
        let mean = angle_sum / f64::from(covered);
        println!("bake: {inside} texels in UV triangles, coverage {coverage:.4}, mean angle {mean:.3} deg, {seconds:.2} s");
        assert!(inside > 10_000, "the low sphere fills the atlas: {inside}");
        assert!(coverage >= 0.9, "coverage {coverage}");
        assert!(mean <= 3.0, "mean angle {mean} degrees");
    }

    #[test]
    fn a_sphere_bakes_onto_its_own_lat_long_uvs() {
        check_sphere_bake(&uv_sphere(1.0, 8, 12));
    }

    #[cfg(feature = "uv")]
    #[test]
    fn a_sphere_bakes_onto_an_unwrapped_low_mesh() {
        use crate::uv::{unwrap, UnwrapOptions};
        let low = unwrap(&uv_sphere(1.0, 8, 12), &UnwrapOptions::default()).unwrap();
        check_sphere_bake(&low.mesh);
    }
}
