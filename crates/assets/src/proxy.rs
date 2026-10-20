//! proxy: ray-tracing proxy meshes with a stored deviation bound.
//!
//! Consumer: the RT acceleration-structure build (`engine-render::accel`
//! builds one BLAS per proxy). Data owner: `engine-assets`. Units: meters and
//! triangles. Cook time only; the fixed tick never runs this module.
//!
//! Method (Solari proxy rule): the full raster mesh goes down the CT2 quadric
//! chain at a ladder of targets, smallest proxy first, and the first proxy
//! whose two-sided deviation from the full surface stays inside the asset's
//! budget ships with that measured bound stored beside it. The budget is
//! [`PROXY_BOUND_RATIO`] of the mesh's bounding-box diagonal, floored at
//! [`PROXY_MIN_BOUND_M`]. A mesh at or below [`PROXY_MIN_TRIS`] keeps its own
//! surface as the proxy with a zero bound.
//!
//! Determinism: the same mesh walks the same ladder and collapses the same
//! edges, so the same proxy bytes and the same bound come out twice (`+ - * /`
//! and `sqrt` only, index-order loops, no clock, no map iteration).

use crate::cooked::CookedMesh;
use crate::mesh_cook::{index_usize, simplify, tri_count, validate_cookable, MeshCookError};
use engine_core::vec3::{cross3, dist2};
use engine_core::Fnv64;
use std::fmt;

/// Most triangles one proxy may hold: the render BLAS cap
/// (`engine_render::accel::ACCEL_MAX_PROXY_TRIS`).
pub const PROXY_MAX_TRIS: u32 = 65_536;

/// Deviation budget as a fraction of the mesh's bounding-box diagonal. The
/// stored bound is the measured deviation, never this ratio.
pub const PROXY_BOUND_RATIO: f32 = 0.15;

/// Smallest deviation budget, in meters, so tiny assets still hold a bound.
pub const PROXY_MIN_BOUND_M: f32 = 0.02;

/// Smallest target the ladder tries, in triangles.
pub const PROXY_MIN_TRIS: u32 = 8;

/// Simplification ladder: full triangles divided by each divisor, smallest
/// proxy first. The cook stops at the first target that stays inside the
/// asset's budget.
pub const PROXY_DIVISORS: [u32; 6] = [64, 32, 16, 8, 4, 2];

/// Last rung of the ladder: three quarters of the full triangle count, tried
/// when even the half target drifts past the budget. It still removes a
/// quarter of the triangles, so a stubborn asset gets a real proxy.
pub const PROXY_LAST_FRACTION: (u32, u32) = (3, 4);

/// One simplified proxy for the ray-tracing structures, with the measured
/// deviation from the full raster surface and the budget it was cooked to.
#[derive(Clone, Debug, PartialEq)]
pub struct ProxyMesh {
    positions: Vec<[f32; 3]>,
    indices: Vec<u32>,
    deviation_m: f32,
    bound_m: f32,
    full_tris: u32,
    digest: u64,
}

impl ProxyMesh {
    /// Simplification plus measured bound plus stable digest.
    fn from_parts(
        full_tris: u32,
        positions: Vec<[f32; 3]>,
        indices: Vec<u32>,
        deviation_m: f32,
        bound_m: f32,
    ) -> Self {
        let digest = proxy_digest(&positions, &indices, deviation_m, bound_m, full_tris);
        Self {
            positions,
            indices,
            deviation_m,
            bound_m,
            full_tris,
            digest,
        }
    }

    /// Proxy positions in meters, for the BLAS build.
    pub fn positions(&self) -> &[[f32; 3]] {
        &self.positions
    }

    /// Proxy triangle indices into [`Self::positions`].
    pub fn indices(&self) -> &[u32] {
        &self.indices
    }

    /// Proxy triangle count.
    pub fn tri_count(&self) -> u32 {
        u32::try_from(self.indices.len() / 3).unwrap_or(u32::MAX)
    }

    /// Proxy vertex count.
    pub fn vert_count(&self) -> usize {
        self.positions.len()
    }

    /// Measured two-sided deviation from the full raster surface, in meters.
    pub fn deviation_m(&self) -> f32 {
        self.deviation_m
    }

    /// Deviation budget the cook demanded for this asset, in meters.
    pub fn bound_m(&self) -> f32 {
        self.bound_m
    }

    /// Whether the measured deviation stays inside the asset's budget.
    pub fn within_bound(&self) -> bool {
        self.deviation_m <= self.bound_m
    }

    /// Triangle count of the full raster mesh this proxy replaces.
    pub fn full_tris(&self) -> u32 {
        self.full_tris
    }

    /// Stable digest over the proxy bytes, the deviation, the budget and the
    /// full triangle count.
    pub fn digest(&self) -> u64 {
        self.digest
    }
}

/// Why a proxy cook failed; every refused input maps to one variant.
#[derive(Clone, Debug, PartialEq)]
pub enum ProxyError {
    /// The full mesh failed the CT2 cook checks.
    Cook(MeshCookError),
    /// A mesh too large to carry as its own surface would hold more triangles
    /// than [`PROXY_MAX_TRIS`].
    OverCap {
        /// Triangles in the refused proxy.
        tris: u32,
        /// Cap that refused it.
        max: u32,
    },
}

impl fmt::Display for ProxyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cook(error) => write!(f, "proxy cook: {error}"),
            Self::OverCap { tris, max } => write!(f, "proxy over cap: {tris} tris over {max}"),
        }
    }
}

impl std::error::Error for ProxyError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Cook(error) => Some(error),
            Self::OverCap { .. } => None,
        }
    }
}

impl From<MeshCookError> for ProxyError {
    fn from(error: MeshCookError) -> Self {
        Self::Cook(error)
    }
}

/// One light probe: the segment from `from` to `to`, in meters.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ray {
    /// Segment start, usually outside the surface.
    pub from: [f32; 3],
    /// Segment end, usually behind the surface.
    pub to: [f32; 3],
}

/// Result of [`light_leaks`]: rays cast and rays the proxy let through.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LeakReport {
    /// Rays tested.
    pub cast: usize,
    /// Rays the full surface blocks but the proxy lets through.
    pub leaks: usize,
}

impl LeakReport {
    /// True when the proxy blocks every ray the full surface blocks.
    pub fn clean(self) -> bool {
        self.leaks == 0
    }
}

/// Cook the proxy for one full raster mesh: walk [`PROXY_DIVISORS`] from the
/// smallest target up, and return the first proxy whose two-sided deviation
/// stays inside the asset budget with that measured bound stored. A mesh that
/// cannot be simplified inside the budget keeps its own surface as the proxy
/// (zero bound): a valid BLAS input with nothing leaked, just no win.
pub fn cook_proxy(full: &CookedMesh) -> Result<ProxyMesh, ProxyError> {
    validate_cookable(full)?;
    let full_tris = tri_count(full);
    let bound_m = proxy_budget_m(full);
    if full_tris > PROXY_MAX_TRIS {
        return Err(ProxyError::OverCap {
            tris: full_tris,
            max: PROXY_MAX_TRIS,
        });
    }
    if full_tris > PROXY_MIN_TRIS {
        for divisor in PROXY_DIVISORS {
            let target = (full_tris / divisor).max(PROXY_MIN_TRIS);
            if let Some(proxy) = try_target(full, full_tris, target, bound_m)? {
                return Ok(proxy);
            }
        }
        let (numerator, denominator) = PROXY_LAST_FRACTION;
        let last = full_tris / denominator * numerator;
        if let Some(proxy) = try_target(full, full_tris, last, bound_m)? {
            return Ok(proxy);
        }
    }
    Ok(ProxyMesh::from_parts(
        full_tris,
        full.positions.clone(),
        full.indices.clone(),
        0.0,
        bound_m,
    ))
}

/// One ladder rung: simplify toward `target` and measure. `None` means the
/// rung offered no smaller mesh or drifted past the budget; `Some` carries the
/// proxy inside the bound.
fn try_target(
    full: &CookedMesh,
    full_tris: u32,
    target: u32,
    bound_m: f32,
) -> Result<Option<ProxyMesh>, ProxyError> {
    if target >= full_tris {
        return Ok(None);
    }
    let proxy = simplify(full, target)?;
    let tris = tri_count(&proxy);
    if tris >= full_tris || tris > PROXY_MAX_TRIS {
        return Ok(None);
    }
    let deviation_m = surface_deviation_m(full, &proxy);
    if deviation_m <= bound_m {
        Ok(Some(ProxyMesh::from_parts(
            full_tris,
            proxy.positions,
            proxy.indices,
            deviation_m,
            bound_m,
        )))
    } else {
        Ok(None)
    }
}

/// Deviation budget for one full mesh: [`PROXY_BOUND_RATIO`] of its
/// bounding-box diagonal, never below [`PROXY_MIN_BOUND_M`].
fn proxy_budget_m(full: &CookedMesh) -> f32 {
    let scaled = PROXY_BOUND_RATIO * mesh_diagonal(full);
    if scaled.is_finite() {
        scaled.max(PROXY_MIN_BOUND_M)
    } else {
        PROXY_MIN_BOUND_M
    }
}

/// Bounding-box diagonal of one mesh, in meters.
fn mesh_diagonal(mesh: &CookedMesh) -> f32 {
    let mut low = [f32::INFINITY; 3];
    let mut high = [f32::NEG_INFINITY; 3];
    for point in &mesh.positions {
        for axis in 0..3 {
            low[axis] = low[axis].min(point[axis]);
            high[axis] = high[axis].max(point[axis]);
        }
    }
    let span = between(high, low);
    dot(span, span).sqrt()
}

/// Two-sided deviation: the farthest full vertex from the proxy surface and
/// the farthest proxy vertex from the full surface, whichever is larger.
fn surface_deviation_m(full: &CookedMesh, proxy: &CookedMesh) -> f32 {
    let out = worst_point_mesh_distance(&full.positions, &proxy.positions, &proxy.indices);
    let back = worst_point_mesh_distance(&proxy.positions, &full.positions, &full.indices);
    out.max(back)
}

/// Farthest distance from any `points` entry to the triangle surface; infinite
/// when the surface holds no triangle, so an empty proxy can never pass.
fn worst_point_mesh_distance(points: &[[f32; 3]], positions: &[[f32; 3]], indices: &[u32]) -> f32 {
    if indices.len() < 3 {
        return f32::INFINITY;
    }
    let mut worst = 0.0f32;
    for point in points {
        let mut best = f32::INFINITY;
        for tri in indices.chunks_exact(3) {
            let (Some(a), Some(b), Some(c)) = (
                positions.get(index_usize(tri[0])),
                positions.get(index_usize(tri[1])),
                positions.get(index_usize(tri[2])),
            ) else {
                continue;
            };
            let distance = closest_distance_sq(*point, *a, *b, *c);
            if distance < best {
                best = distance;
            }
        }
        if best.is_finite() && best > worst {
            worst = best;
        }
    }
    worst.sqrt()
}

/// Squared distance from `p` to triangle `abc`: the plane distance when the
/// projection lands inside the triangle, else the nearest edge.
fn closest_distance_sq(p: [f32; 3], a: [f32; 3], b: [f32; 3], c: [f32; 3]) -> f32 {
    let ab = between(b, a);
    let ac = between(c, a);
    let normal = cross3(ab, ac);
    let area = dot(normal, normal);
    if area <= 1e-20 {
        return dist2(p, a).min(dist2(p, b)).min(dist2(p, c));
    }
    let height = dot(between(p, a), normal);
    let foot = between(p, scale(normal, height / area));
    let offset = between(foot, a);
    if let Some([v, w]) = barycentric(offset, ab, ac) {
        if v >= 0.0 && w >= 0.0 && v + w <= 1.0 {
            return dist2(p, foot);
        }
    }
    dist2(p, a)
        .min(segment_distance_sq(p, a, b))
        .min(segment_distance_sq(p, b, c))
        .min(segment_distance_sq(p, c, a))
}

/// Barycentric weights `[v, w]` of `offset = p - a` in the basis `ab`, `ac`;
/// `None` when the triangle is degenerate. Weights are not clamped.
fn barycentric(offset: [f32; 3], ab: [f32; 3], ac: [f32; 3]) -> Option<[f32; 2]> {
    let d00 = dot(ab, ab);
    let d01 = dot(ab, ac);
    let d11 = dot(ac, ac);
    let denom = d00 * d11 - d01 * d01;
    if denom.abs() <= 1e-20 {
        return None;
    }
    let d20 = dot(offset, ab);
    let d21 = dot(offset, ac);
    Some([
        (d11 * d20 - d01 * d21) / denom,
        (d00 * d21 - d01 * d20) / denom,
    ])
}

/// Squared distance from `p` to the segment `ab`.
fn segment_distance_sq(p: [f32; 3], a: [f32; 3], b: [f32; 3]) -> f32 {
    let ab = between(b, a);
    let length = dot(ab, ab);
    if length <= 1e-20 {
        return dist2(p, a);
    }
    let t = (dot(between(p, a), ab) / length).clamp(0.0, 1.0);
    dist2(p, add_scaled(a, ab, t))
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// Difference `lhs - rhs` of two points, axis by axis.
fn between(lhs: [f32; 3], rhs: [f32; 3]) -> [f32; 3] {
    let mut out = [0.0f32; 3];
    for axis in 0..3 {
        out[axis] = lhs[axis] - rhs[axis];
    }
    out
}

fn scale(v: [f32; 3], k: f32) -> [f32; 3] {
    [v[0] * k, v[1] * k, v[2] * k]
}

fn add_scaled(a: [f32; 3], step: [f32; 3], amount: f32) -> [f32; 3] {
    let s = scale(step, amount);
    [a[0] + s[0], a[1] + s[1], a[2] + s[2]]
}

/// Count light paths the full surface blocks but the proxy lets through. A
/// leak is a ray whose first proxy crossing sits deeper than the full
/// surface's first crossing plus the stored bound: the proxy may drift by its
/// bound, never by a wall. A proxy that opens a hole leaks, so the tunnel
/// fixture fails.
pub fn light_leaks(full: &CookedMesh, proxy: &ProxyMesh, rays: &[Ray]) -> LeakReport {
    let mut leaks = 0usize;
    for ray in rays {
        let Some(first) = nearest_hit(&full.positions, &full.indices, *ray) else {
            continue; // open to light in the raster mesh too: not a proxy leak
        };
        let length = dot(between(ray.to, ray.from), between(ray.to, ray.from)).sqrt();
        if length <= 1e-9 {
            continue;
        }
        let allowed = first + proxy.bound_m() / length;
        let leaked = match nearest_hit(&proxy.positions, &proxy.indices, *ray) {
            None => true,
            Some(depth) => depth > allowed,
        };
        if leaked {
            leaks += 1;
        }
    }
    LeakReport {
        cast: rays.len(),
        leaks,
    }
}

/// Nearest crossing of the segment with the mesh, as a `0..=1` depth; a hole
/// lets the segment through.
fn nearest_hit(positions: &[[f32; 3]], indices: &[u32], ray: Ray) -> Option<f32> {
    let dir = between(ray.to, ray.from);
    let mut best: Option<f32> = None;
    for tri in indices.chunks_exact(3) {
        let (Some(a), Some(b), Some(c)) = (
            positions.get(index_usize(tri[0])),
            positions.get(index_usize(tri[1])),
            positions.get(index_usize(tri[2])),
        ) else {
            continue;
        };
        let Some(depth) = triangle_depth(*a, *b, *c, ray.from, dir) else {
            continue;
        };
        if best.is_none_or(|known| depth < known) {
            best = Some(depth);
        }
    }
    best
}

/// Depth of the segment's crossing with the triangle, or `None`: intersect the
/// ray with the triangle's plane, keep `0..=1` of the segment, then test the
/// barycentric weights. Two-sided; a hole lets the segment through.
fn triangle_depth(
    a: [f32; 3],
    b: [f32; 3],
    c: [f32; 3],
    origin: [f32; 3],
    dir: [f32; 3],
) -> Option<f32> {
    let e1 = between(b, a);
    let e2 = between(c, a);
    let normal = cross3(e1, e2);
    let denom = dot(normal, dir);
    if denom.abs() <= 1e-12 {
        return None;
    }
    let t = dot(normal, between(a, origin)) / denom;
    if !(0.0..=1.0).contains(&t) {
        return None;
    }
    let hit = add_scaled(origin, dir, t);
    let [v, w] = barycentric(between(hit, a), e1, e2)?;
    if v >= 0.0 && w >= 0.0 && v + w <= 1.0 {
        Some(t)
    } else {
        None
    }
}

/// Stable `u64` over one proxy: arrays by bits, the measured deviation, the
/// budget and the full triangle count. The same mesh gives the same digest
/// twice.
fn proxy_digest(
    positions: &[[f32; 3]],
    indices: &[u32],
    deviation_m: f32,
    bound_m: f32,
    full_tris: u32,
) -> u64 {
    let mut state = Fnv64::new();
    state.write_len(positions.len());
    for position in positions {
        for lane in position {
            state.write_f32(*lane);
        }
    }
    state.write_len(indices.len());
    for index in indices {
        state.write_u32(*index);
    }
    state.write_f32(deviation_m);
    state.write_f32(bound_m);
    state.write_u32(full_tris);
    state.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kit_import::KIT_FILES;
    use crate::mesh_cook::{cook_gltf_bytes, MeshClass};
    use std::fs;
    use std::path::Path;
    use std::sync::OnceLock;

    /// The museum kit: the eight arena models the prefab museum places,
    /// cooked through the CT2 prop lane; every LOD0 mesh is one museum asset.
    fn museum_assets() -> &'static [(String, CookedMesh)] {
        static MUSEUM: OnceLock<Vec<(String, CookedMesh)>> = OnceLock::new();
        MUSEUM.get_or_init(|| {
            let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../content/arena/src");
            let mut assets = Vec::new();
            for file in KIT_FILES {
                let bytes =
                    fs::read(root.join(file)).unwrap_or_else(|error| panic!("{file}: {error}"));
                let cooked = cook_gltf_bytes(&bytes, MeshClass::Prop)
                    .unwrap_or_else(|error| panic!("{file}: {error}"));
                for (index, mesh) in cooked.meshes.iter().enumerate() {
                    assets.push((format!("{file}#{index}"), mesh.lods[0].clone()));
                }
            }
            assets
        })
    }

    #[test]
    fn proxies_stay_inside_their_bound_on_every_museum_asset() {
        let assets = museum_assets();
        let mut files: Vec<&str> = assets
            .iter()
            .filter_map(|(name, _)| name.split('#').next())
            .collect();
        files.sort_unstable();
        files.dedup();
        assert_eq!(files.len(), KIT_FILES.len(), "every kit file is present");
        let mut simplified = 0usize;
        for (name, full) in assets {
            let proxy = cook_proxy(full).unwrap_or_else(|error| panic!("{name}: {error}"));
            assert!(
                proxy.deviation_m().is_finite(),
                "{name}: deviation is not finite"
            );
            assert!(
                proxy.within_bound(),
                "{name}: deviation {} m over bound {} m",
                proxy.deviation_m(),
                proxy.bound_m()
            );
            assert!(
                proxy.bound_m() <= proxy_budget_m(full),
                "{name}: bound {} m over the budget",
                proxy.bound_m()
            );
            assert_eq!(proxy.full_tris(), tri_count(full), "{name}: full count");
            assert!(
                proxy.vert_count() <= full.positions.len(),
                "{name}: proxy added vertices"
            );
            if proxy.tri_count() == tri_count(full) {
                assert_eq!(
                    proxy.deviation_m(),
                    0.0,
                    "{name}: an unsimplified proxy must carry the raster surface"
                );
            } else {
                assert!(
                    proxy.tri_count() < tri_count(full),
                    "{name}: proxy {} tris vs {} raster tris",
                    proxy.tri_count(),
                    tri_count(full)
                );
                simplified += 1;
            }
        }
        assert!(
            simplified >= assets.len() * 3 / 4,
            "only {simplified} of {} museum assets got a simplified proxy",
            assets.len()
        );
    }

    #[test]
    fn the_same_mesh_cooks_the_same_proxy_twice() {
        for (name, full) in museum_assets() {
            let first =
                cook_proxy(full).unwrap_or_else(|error| panic!("{name} first cook: {error}"));
            let second =
                cook_proxy(full).unwrap_or_else(|error| panic!("{name} second cook: {error}"));
            assert_eq!(first, second, "{name}: proxy x2 differs");
            assert_eq!(first.digest(), second.digest(), "{name}: digest x2 differs");
        }
    }

    #[test]
    fn a_tiny_mesh_keeps_its_own_surface_as_the_proxy() {
        let full = grid_quad(1);
        let proxy = cook_proxy(&full).expect("a two-triangle mesh cooks");
        assert_eq!(proxy.tri_count(), 2);
        assert_eq!(proxy.deviation_m(), 0.0);
        assert!(proxy.within_bound());
    }

    #[test]
    fn a_degenerate_mesh_is_refused_not_panicking() {
        let error = cook_proxy(&CookedMesh::default()).expect_err("empty mesh refused");
        assert!(matches!(error, ProxyError::Cook(MeshCookError::Empty)));
    }

    #[test]
    fn a_light_leak_through_a_proxy_fails_the_tunnel_fixture() {
        let tunnel = tunnel_mesh();
        let proxy = cook_proxy(&tunnel).expect("the tunnel cooks a proxy");
        assert!(
            proxy.tri_count() < tri_count(&tunnel),
            "the tunnel proxy must be simplified"
        );
        let rays = tunnel_light_rays();
        let clean = light_leaks(&tunnel, &proxy, &rays);
        assert_eq!(clean.cast, rays.len());
        assert!(clean.clean(), "the tunnel proxy leaks light: {clean:?}");

        let (holed, hole_ray) = poked_proxy(&proxy);
        let aimed = light_leaks(&tunnel, &proxy, &[hole_ray]);
        assert!(aimed.clean(), "the clean proxy already leaks: {aimed:?}");
        let report = light_leaks(&tunnel, &holed, &[hole_ray]);
        assert_eq!(report.cast, 1);
        assert!(
            !report.clean(),
            "a hole in the proxy did not fail the tunnel fixture"
        );
    }

    /// One axis-aligned grid patch: `nu` x `nv` cells from `origin` along
    /// `along` and `up`, with the given outward normal.
    fn add_patch(
        mesh: &mut CookedMesh,
        origin: [f32; 3],
        along: [f32; 3],
        up: [f32; 3],
        nu: u32,
        nv: u32,
        normal: [f32; 3],
    ) {
        let base = u32::try_from(mesh.positions.len()).expect("fixture fits u32");
        for j in 0..=nv {
            for i in 0..=nu {
                let u = i as f32 / nu as f32;
                let v = j as f32 / nv as f32;
                mesh.positions.push([
                    origin[0] + along[0] * u + up[0] * v,
                    origin[1] + along[1] * u + up[1] * v,
                    origin[2] + along[2] * u + up[2] * v,
                ]);
                mesh.normals.push(normal);
                mesh.uv0.push([u, v]);
                mesh.tangents.push([1.0, 0.0, 0.0, 1.0]);
            }
        }
        for j in 0..nv {
            for i in 0..nu {
                let a = base + j * (nu + 1) + i;
                let b = a + 1;
                let c = a + nu + 1;
                let d = c + 1;
                mesh.indices.extend_from_slice(&[a, b, d, a, d, c]);
            }
        }
    }

    /// Unit grid: two triangles on the `z = 0` plane.
    fn grid_quad(n: u32) -> CookedMesh {
        let mut mesh = CookedMesh::default();
        add_patch(
            &mut mesh,
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            n,
            n,
            [0.0, 0.0, 1.0],
        );
        mesh
    }

    /// A sealed 4 x 2 x 2 m tunnel shell: four walls plus two ends, so light
    /// outside must never reach the inside.
    fn tunnel_mesh() -> CookedMesh {
        let mut mesh = CookedMesh::default();
        // Floor, ceiling.
        add_patch(
            &mut mesh,
            [0.0, 0.0, -1.0],
            [4.0, 0.0, 0.0],
            [0.0, 0.0, 2.0],
            8,
            4,
            [0.0, -1.0, 0.0],
        );
        add_patch(
            &mut mesh,
            [0.0, 2.0, -1.0],
            [4.0, 0.0, 0.0],
            [0.0, 0.0, 2.0],
            8,
            4,
            [0.0, 1.0, 0.0],
        );
        // Side walls.
        add_patch(
            &mut mesh,
            [0.0, 0.0, -1.0],
            [4.0, 0.0, 0.0],
            [0.0, 2.0, 0.0],
            8,
            4,
            [0.0, 0.0, -1.0],
        );
        add_patch(
            &mut mesh,
            [0.0, 0.0, 1.0],
            [4.0, 0.0, 0.0],
            [0.0, 2.0, 0.0],
            8,
            4,
            [0.0, 0.0, 1.0],
        );
        // Ends.
        add_patch(
            &mut mesh,
            [0.0, 0.0, -1.0],
            [0.0, 0.0, 2.0],
            [0.0, 2.0, 0.0],
            8,
            4,
            [-1.0, 0.0, 0.0],
        );
        add_patch(
            &mut mesh,
            [4.0, 0.0, -1.0],
            [0.0, 0.0, 2.0],
            [0.0, 2.0, 0.0],
            8,
            4,
            [1.0, 0.0, 0.0],
        );
        weld(&mut mesh);
        mesh
    }

    /// Weld coincident seam vertices so the shell is closed: patches meet at
    /// shared edges, and only a closed surface can prove no light gets in.
    fn weld(mesh: &mut CookedMesh) {
        use std::collections::BTreeMap;
        const GRID: f32 = 100_000.0;
        let mut seen: BTreeMap<(i64, i64, i64), u32> = BTreeMap::new();
        let mut remap = Vec::with_capacity(mesh.positions.len());
        let mut positions = Vec::new();
        let mut normals = Vec::new();
        let mut uv0 = Vec::new();
        let mut tangents = Vec::new();
        for (index, point) in mesh.positions.iter().enumerate() {
            let key = (
                (point[0] * GRID).round() as i64,
                (point[1] * GRID).round() as i64,
                (point[2] * GRID).round() as i64,
            );
            let welded = *seen.entry(key).or_insert_with(|| {
                let at = u32::try_from(positions.len()).expect("fixture fits u32");
                positions.push(*point);
                normals.push(mesh.normals[index]);
                uv0.push(mesh.uv0[index]);
                tangents.push(mesh.tangents[index]);
                at
            });
            remap.push(welded);
        }
        for index in &mut mesh.indices {
            *index = remap[index_usize(*index)];
        }
        mesh.positions = positions;
        mesh.normals = normals;
        mesh.uv0 = uv0;
        mesh.tangents = tangents;
    }

    /// Probe rays from outside each tunnel face toward the inside; a light
    /// outside a wall must stay outside.
    fn tunnel_light_rays() -> Vec<Ray> {
        let mut rays = Vec::new();
        let across = [0.5f32, 1.0, 1.5];
        for x in [0.5f32, 1.5, 2.5, 3.5] {
            for y in across {
                for z in [-0.5f32, 0.0, 0.5] {
                    rays.push(Ray {
                        from: [x, y, -4.0],
                        to: [x, y, 0.0],
                    });
                    rays.push(Ray {
                        from: [x, y, 4.0],
                        to: [x, y, 0.0],
                    });
                    rays.push(Ray {
                        from: [x, -3.0, z],
                        to: [x, 1.0, z],
                    });
                    rays.push(Ray {
                        from: [x, 5.0, z],
                        to: [x, 1.0, z],
                    });
                    rays.push(Ray {
                        from: [-3.0, y, z],
                        to: [2.0, y, z],
                    });
                    rays.push(Ray {
                        from: [7.0, y, z],
                        to: [2.0, y, z],
                    });
                }
            }
        }
        rays
    }

    /// Copy the proxy with the triangle nearest the `+Z` wall centre removed,
    /// plus the ray that aimed through it.
    fn poked_proxy(proxy: &ProxyMesh) -> (ProxyMesh, Ray) {
        let aim = [2.0, 1.0, 1.0];
        let mut drop = 0usize;
        let mut best = f32::INFINITY;
        for (slot, tri) in proxy.indices.chunks_exact(3).enumerate() {
            let centroid = tri_centroid(proxy, tri);
            let distance = dist2(centroid, aim);
            if distance < best {
                best = distance;
                drop = slot;
            }
        }
        let tri = &proxy.indices[drop * 3..drop * 3 + 3];
        let a = proxy.positions[index_usize(tri[0])];
        let b = proxy.positions[index_usize(tri[1])];
        let c = proxy.positions[index_usize(tri[2])];
        let centroid = tri_centroid(proxy, tri);
        let mut normal = cross3(between(b, a), between(c, a));
        let length = dot(normal, normal).sqrt();
        if length > 1e-12 {
            normal = [normal[0] / length, normal[1] / length, normal[2] / length];
        }
        if normal[2] < 0.0 {
            normal = [-normal[0], -normal[1], -normal[2]];
        }
        let mut holed = proxy.clone();
        holed.indices.drain(drop * 3..drop * 3 + 3);
        let ray = Ray {
            from: add_scaled(centroid, normal, 2.0),
            to: add_scaled(centroid, normal, -0.5),
        };
        (holed, ray)
    }

    fn tri_centroid(proxy: &ProxyMesh, tri: &[u32]) -> [f32; 3] {
        let a = proxy.positions[index_usize(tri[0])];
        let b = proxy.positions[index_usize(tri[1])];
        let c = proxy.positions[index_usize(tri[2])];
        [
            (a[0] + b[0] + c[0]) / 3.0,
            (a[1] + b[1] + c[1]) / 3.0,
            (a[2] + b[2] + c[2]) / 3.0,
        ]
    }
}
