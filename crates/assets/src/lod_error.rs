//! lod_error: the LOD chain with a measured geometric error per level.
//!
//! [`crate::mesh_cook::build_lod_chain`] already produces LOD0 plus three
//! quadric simplifications, but a renderer that picks a level by projected
//! pixel error needs the error itself in metres. This module measures it: for
//! every level it takes the largest distance from a vertex of one mesh to the
//! nearest triangle of the other, in both directions (a two-way Hausdorff
//! bound), and reports it against LOD0. LOD0 therefore has error 0 and is the
//! source mesh unchanged, so arming LOD selection never changes level 0.
//!
//! Units: metres for positions and errors, triangles for counts. Determinism:
//! the simplifier is already byte-stable, and the measurement is a fixed
//! iteration order over vertices and triangles with `+ - * /` only, so the
//! same input gives the same errors twice. No allocation beyond the chain.
//! Caller: the arena kit bake stores one [`CookedLod`] per level beside each
//! actor mesh and hands `error_m` to the pixel-error LOD pick.

use crate::cooked::CookedMesh;
use crate::mesh_cook::{build_lod_chain, tri_count, MeshCookError};

/// One level of a measured LOD chain: the simplified mesh and how far it
/// strays from LOD0.
#[derive(Clone, Debug, PartialEq)]
pub struct CookedLod {
    /// Level index: 0 is the source mesh, 1..3 the simplifications.
    pub level: usize,
    /// Triangles in this level.
    pub triangles: u32,
    /// Two-way maximum vertex-to-surface deviation from LOD0 in metres; 0 for
    /// level 0.
    pub error_m: f32,
    /// True when the edge cap removed sub-pixel triangles from this level; a
    /// level that never met its triangle budget may be small because of it.
    pub edge_limited: bool,
    /// The level's mesh; level 0 is the source clone, byte for byte.
    pub mesh: CookedMesh,
}

/// Squared distance from `p` to segment `ab`.
fn point_segment_d2(p: [f32; 3], a: [f32; 3], b: [f32; 3]) -> f32 {
    let ab = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let ap = [p[0] - a[0], p[1] - a[1], p[2] - a[2]];
    let len2 = ab[0] * ab[0] + ab[1] * ab[1] + ab[2] * ab[2];
    if len2 <= f32::EPSILON {
        return ap[0] * ap[0] + ap[1] * ap[1] + ap[2] * ap[2];
    }
    let t = ((ap[0] * ab[0] + ap[1] * ab[1] + ap[2] * ab[2]) / len2).clamp(0.0, 1.0);
    let q = [a[0] + ab[0] * t, a[1] + ab[1] * t, a[2] + ab[2] * t];
    let d = [p[0] - q[0], p[1] - q[1], p[2] - q[2]];
    d[0] * d[0] + d[1] * d[1] + d[2] * d[2]
}

/// Squared distance from `p` to triangle `abc`; the closest point lies on a
/// face or an edge, so the minimum over the three edges plus the face plane
/// projection is the exact distance.
fn point_triangle_d2(p: [f32; 3], a: [f32; 3], b: [f32; 3], c: [f32; 3]) -> f32 {
    let mut best = point_segment_d2(p, a, b)
        .min(point_segment_d2(p, b, c))
        .min(point_segment_d2(p, c, a));
    // Face interior: project onto the triangle plane, keep it when it lands
    // inside all three edges (same-side test with the edge normals).
    let ab = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let ac = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
    let n = [
        ab[1] * ac[2] - ab[2] * ac[1],
        ab[2] * ac[0] - ab[0] * ac[2],
        ab[0] * ac[1] - ab[1] * ac[0],
    ];
    let n2 = n[0] * n[0] + n[1] * n[1] + n[2] * n[2];
    if n2 > f32::EPSILON {
        let ap = [p[0] - a[0], p[1] - a[1], p[2] - a[2]];
        let t = (ap[0] * n[0] + ap[1] * n[1] + ap[2] * n[2]) / n2;
        let q = [p[0] - n[0] * t, p[1] - n[1] * t, p[2] - n[2] * t];
        let inside = |u: [f32; 3], v: [f32; 3]| {
            let du = [v[0] - u[0], v[1] - u[1], v[2] - u[2]];
            let uq = [q[0] - u[0], q[1] - u[1], q[2] - u[2]];
            du[0] * uq[1] - du[1] * uq[0] >= -f32::EPSILON
                && du[1] * uq[2] - du[2] * uq[1] >= -f32::EPSILON
                && du[2] * uq[0] - du[0] * uq[2] >= -f32::EPSILON
        };
        if inside(a, b) && inside(b, c) && inside(c, a) {
            let d = [p[0] - q[0], p[1] - q[1], p[2] - q[2]];
            best = best.min(d[0] * d[0] + d[1] * d[1] + d[2] * d[2]);
        }
    }
    best
}

/// One position by triangle index; `None` when the index is outside the
/// mesh (the cook refuses those, so this is a belt-and-braces read).
fn vertex(mesh: &CookedMesh, index: u32) -> Option<[f32; 3]> {
    let at = usize::try_from(index).ok()?;
    mesh.positions.get(at).copied()
}

/// Largest distance from any vertex of `from` to the surface of `to`, metres.
/// `0` when either mesh has no triangles or no vertices.
#[must_use]
pub fn one_way_deviation_m(from: &CookedMesh, to: &CookedMesh) -> f32 {
    if from.positions.is_empty() || to.indices.len() < 3 {
        return 0.0;
    }
    let mut worst = 0.0f32;
    for point in &from.positions {
        let mut nearest = f32::MAX;
        for tri in to.indices.chunks_exact(3) {
            let (Some(a), Some(b), Some(c)) =
                (vertex(to, tri[0]), vertex(to, tri[1]), vertex(to, tri[2]))
            else {
                continue;
            };
            let d2 = point_triangle_d2(*point, a, b, c);
            if d2 < nearest {
                nearest = d2;
            }
        }
        if nearest.is_finite() {
            worst = worst.max(nearest.sqrt());
        }
    }
    worst
}

/// Two-way maximum vertex-to-surface deviation between two meshes, metres.
/// Symmetric by construction, so an error never depends on which mesh is
/// named first.
#[must_use]
pub fn max_deviation_m(a: &CookedMesh, b: &CookedMesh) -> f32 {
    one_way_deviation_m(a, b).max(one_way_deviation_m(b, a))
}

// ---------------------------------------------------------------------------
// Edge-length cap: a triangle whose longest edge is under a pixel at the
// level's switch distance can never be resolved, so it is dropped instead of
// reaching the vertex stage. This is meshoptimizer's `simplify_error_edge_limit`
// idea (clusterlod.h `:46`, applied as `min(error, sqrt(max_edge_sq) * limit)`),
// here as a post-pass over the kept triangles.
// ---------------------------------------------------------------------------

/// Shortest projected edge a kept triangle may have, in pixels. Under this the
/// triangle covers less than one pixel of screen at the level's switch
/// distance, where meshoptimizer's own cluster-LOD demo caps out too.
pub const MIN_EDGE_PX: f32 = 0.6;

/// Where one LOD level starts being used: its switch distance, the frame
/// height and the lens. The cap projects an edge as
/// `edge_m * screen_h / (switch_m * tan_half_fov_y)`, the same projection the
/// render pick uses for a radius.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LodEdgePolicy {
    /// Metric distance at which a node takes this level.
    pub switch_m: f32,
    /// Frame height in pixels.
    pub screen_h: u32,
    /// `tan(fov_y_rad / 2)` of the vertical field of view.
    pub tan_half_fov_y: f32,
    /// Keep a triangle only while its longest edge projects to at least this
    /// many pixels.
    pub min_edge_px: f32,
}

impl LodEdgePolicy {
    /// The policy for one level; `min_edge_px` is [`MIN_EDGE_PX`].
    #[must_use]
    pub fn new(switch_m: f32, screen_h: u32, tan_half_fov_y: f32) -> Self {
        Self {
            switch_m,
            screen_h,
            tan_half_fov_y,
            min_edge_px: MIN_EDGE_PX,
        }
    }

    /// Pixels one metre covers at this level's switch distance.
    #[must_use]
    pub fn px_per_m(&self) -> f32 {
        let screen_h_px = f32::from(u16::try_from(self.screen_h).unwrap_or(u16::MAX));
        screen_h_px / (self.switch_m * self.tan_half_fov_y)
    }

    /// True when a triangle with this longest edge is too small to keep.
    #[must_use]
    pub fn sub_pixel(&self, longest_edge_m: f32) -> bool {
        longest_edge_m * self.px_per_m() < self.min_edge_px
    }
}

/// One capped mesh and how many triangles the cap removed.
#[derive(Clone, Debug, PartialEq)]
pub struct EdgeCapped {
    /// The mesh with sub-pixel triangles removed and unused vertices dropped.
    pub mesh: CookedMesh,
    /// Triangles the cap removed.
    pub dropped: u32,
}

/// Longest edge squared of one triangle.
fn longest_edge_sq(a: [f32; 3], b: [f32; 3], c: [f32; 3]) -> f32 {
    let d2 = |p: [f32; 3], q: [f32; 3]| {
        let d = [p[0] - q[0], p[1] - q[1], p[2] - q[2]];
        d[0] * d[0] + d[1] * d[1] + d[2] * d[2]
    };
    // `mesh` is unused by the arithmetic; keep the signature explicit.
    d2(a, b).max(d2(b, c)).max(d2(c, a))
}

/// Drop every triangle whose longest edge projects under the policy's
/// minimum, then compact the vertices down to first-use order. One pass is
/// enough: dropping a triangle never lengthens another triangle's edges, so
/// the result is stable.
///
/// Errors: none; a mesh with no triangles comes back unchanged.
#[must_use]
pub fn edge_limit(mesh: &CookedMesh, policy: &LodEdgePolicy) -> EdgeCapped {
    let mut kept: Vec<[u32; 3]> = Vec::with_capacity(mesh.indices.len() / 3);
    let mut dropped = 0u32;
    for tri in mesh.indices.chunks_exact(3) {
        let (Some(a), Some(b), Some(c)) = (
            vertex(mesh, tri[0]),
            vertex(mesh, tri[1]),
            vertex(mesh, tri[2]),
        ) else {
            dropped += 1;
            continue;
        };
        let longest = longest_edge_sq(a, b, c).sqrt();
        if policy.sub_pixel(longest) {
            dropped += 1;
        } else {
            kept.push([tri[0], tri[1], tri[2]]);
        }
    }
    if dropped == 0 {
        return EdgeCapped {
            mesh: mesh.clone(),
            dropped,
        };
    }
    // Compact: keep only the vertices a kept triangle names, in first-use
    // order, so the upload never carries an orphan lane.
    let mut remap = vec![u32::MAX; mesh.positions.len()];
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut uv0 = Vec::new();
    let mut tangents = Vec::new();
    let mut indices = Vec::with_capacity(kept.len() * 3);
    let push = |old: u32,
                remap: &mut Vec<u32>,
                positions: &mut Vec<[f32; 3]>,
                normals: &mut Vec<[f32; 3]>,
                uv0: &mut Vec<[f32; 2]>,
                tangents: &mut Vec<[f32; 4]>|
     -> u32 {
        let at = usize::try_from(old).unwrap_or(usize::MAX);
        if let Some(existing) = remap.get(at).copied().filter(|m| *m != u32::MAX) {
            return existing;
        }
        let new = u32::try_from(positions.len()).unwrap_or(u32::MAX);
        if let Some(p) = mesh.positions.get(at) {
            positions.push(*p);
        }
        if let Some(n) = mesh.normals.get(at) {
            normals.push(*n);
        }
        if let Some(u) = mesh.uv0.get(at) {
            uv0.push(*u);
        }
        if let Some(t) = mesh.tangents.get(at) {
            tangents.push(*t);
        }
        if let Some(slot) = remap.get_mut(at) {
            *slot = new;
        }
        new
    };
    for tri in &kept {
        for old in tri {
            let new = push(
                *old,
                &mut remap,
                &mut positions,
                &mut normals,
                &mut uv0,
                &mut tangents,
            );
            indices.push(new);
        }
    }
    EdgeCapped {
        mesh: CookedMesh {
            positions,
            normals,
            uv0,
            tangents,
            indices,
        },
        dropped,
    }
}

/// The measured LOD chain for `source`: `levels` clamped to
/// `1..=MAX_LOD_LEVELS`, level 0 the untouched source with `error_m == 0`,
/// each later level the simplifier's output measured against level 0.
///
/// The reported error is a running maximum: `level k` reports the worst
/// deviation of levels `0..=k`. A two-way Hausdorff measurement is not
/// monotone in the triangle count — a coarser level can move a surface back
/// toward the source — but a picker that walks the chain must never take a
/// coarser level believing it is more accurate, so the chain reports the
/// worst error seen so far. No mesh is changed by that clamp.
///
/// Errors: what [`build_lod_chain`] refuses — an empty, non-finite,
/// out-of-range or zero-area mesh.
pub fn measure_lod_chain(
    source: &CookedMesh,
    levels: usize,
) -> Result<Vec<CookedLod>, MeshCookError> {
    measure_lod_chain_capped(source, levels, None)
}

/// [`measure_lod_chain`] with the edge cap: after each simplify the level is
/// run through [`edge_limit`] with that level's [`LodEdgePolicy`], and
/// `edge_limited` is set when the cap removed a triangle. Level 0 is never
/// capped: it is the source, byte for byte, whatever the policy says.
///
/// `policies` is indexed by level, so `policies[0]` is ignored; a missing
/// entry (fewer policies than levels) leaves that level uncapped.
pub fn measure_lod_chain_with_edges(
    source: &CookedMesh,
    levels: usize,
    policies: &[LodEdgePolicy],
) -> Result<Vec<CookedLod>, MeshCookError> {
    measure_lod_chain_capped(source, levels, Some(policies))
}

fn measure_lod_chain_capped(
    source: &CookedMesh,
    levels: usize,
    policies: Option<&[LodEdgePolicy]>,
) -> Result<Vec<CookedLod>, MeshCookError> {
    let chain = build_lod_chain(source, levels)?;
    let mut out: Vec<CookedLod> = Vec::with_capacity(chain.len());
    let mut worst = 0.0f32;
    for (level, mesh) in chain.into_iter().enumerate() {
        let (mesh, edge_limited) = match (level, policies) {
            (0, _) => (mesh, false),
            (_, Some(policies)) => match policies.get(level) {
                Some(policy) => {
                    let capped = edge_limit(&mesh, policy);
                    let limited = capped.dropped > 0;
                    (capped.mesh, limited)
                }
                None => (mesh, false),
            },
            (_, None) => (mesh, false),
        };
        if level > 0 {
            worst = worst.max(max_deviation_m(source, &mesh));
        }
        out.push(CookedLod {
            level,
            triangles: tri_count(&mesh),
            error_m: worst,
            edge_limited,
            mesh,
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A closed UV sphere of `rings x segments` quads, so quadric
    /// simplification has real surface to remove. Radius 0.45 m, the size of
    /// a fighter's torso, which is what the pixel-error pick projects.
    fn sphere(rings: usize, segments: usize) -> CookedMesh {
        let radius = 0.45f32;
        let mut positions = Vec::new();
        let mut normals = Vec::new();
        let mut uv0 = Vec::new();
        for ring in 0..=rings {
            let v = ring as f32 / rings as f32;
            let theta = v * std::f32::consts::PI;
            for segment in 0..=segments {
                let u = segment as f32 / segments as f32;
                let phi = u * std::f32::consts::TAU;
                let n = [
                    theta.sin() * phi.cos(),
                    theta.cos(),
                    theta.sin() * phi.sin(),
                ];
                positions.push([n[0] * radius, n[1] * radius, n[2] * radius]);
                normals.push(n);
                uv0.push([u, v]);
            }
        }
        let stride = segments + 1;
        let mut indices = Vec::new();
        for ring in 0..rings {
            for segment in 0..segments {
                let a = u32::try_from(ring * stride + segment).expect("index fits");
                let b = a + 1;
                let c = a + u32::try_from(stride).expect("stride fits");
                let d = c + 1;
                indices.extend_from_slice(&[a, c, b, b, c, d]);
            }
        }
        // The cook validates every attribute lane against the positions, so
        // the fixture carries a tangent per vertex like an imported mesh does.
        let tangents = vec![[1.0, 0.0, 0.0, 1.0]; positions.len()];
        CookedMesh {
            positions,
            normals,
            uv0,
            tangents,
            indices,
        }
    }

    /// Pixels one metre of error covers at `distance_m`, at 1080p through the
    /// 60 degree duel lens: the same projection `arena_lod` uses for a radius.
    fn error_px(error_m: f32, distance_m: f32) -> f32 {
        error_m * 1080.0 / (distance_m * (60.0f32.to_radians() * 0.5).tan())
    }

    /// A railing seen from the far seats: a 19 x 10 wall of 0.3 m quads (380
    /// triangles) with 60 small detail quads (120 triangles, 1 cm edges) laid
    /// 1 mm above one cell. The detail is sub-pixel at 60 m and fully covered
    /// by the cell below it, so dropping it must not move the surface.
    fn railing() -> CookedMesh {
        let mut positions = Vec::new();
        let mut normals = Vec::new();
        let mut uv0 = Vec::new();
        let mut indices = Vec::new();
        let quad = |positions: &mut Vec<[f32; 3]>,
                    normals: &mut Vec<[f32; 3]>,
                    uv0: &mut Vec<[f32; 2]>,
                    indices: &mut Vec<u32>,
                    origin: [f32; 3],
                    size: f32| {
            let base = u32::try_from(positions.len()).expect("index fits");
            let p = [
                [origin[0], origin[1], origin[2]],
                [origin[0] + size, origin[1], origin[2]],
                [origin[0], origin[1] + size, origin[2]],
                [origin[0] + size, origin[1] + size, origin[2]],
            ];
            for (i, point) in p.iter().enumerate() {
                positions.push(*point);
                normals.push([0.0, 0.0, 1.0]);
                uv0.push([f32::from(u16::try_from(i % 2).expect("fits")), 0.0]);
            }
            indices.extend_from_slice(&[base, base + 1, base + 2, base + 1, base + 3, base + 2]);
        };
        for row in 0..10u32 {
            for column in 0..19u32 {
                quad(
                    &mut positions,
                    &mut normals,
                    &mut uv0,
                    &mut indices,
                    [
                        f32::from(u16::try_from(column).expect("fits")) * 0.3,
                        f32::from(u16::try_from(row).expect("fits")) * 0.3,
                        0.0,
                    ],
                    0.3,
                );
            }
        }
        for detail in 0..60u32 {
            quad(
                &mut positions,
                &mut normals,
                &mut uv0,
                &mut indices,
                [
                    f32::from(u16::try_from(detail % 10).expect("fits")) * 0.01,
                    f32::from(u16::try_from(detail / 10).expect("fits")) * 0.01,
                    0.001,
                ],
                0.01,
            );
        }
        CookedMesh {
            positions,
            normals,
            uv0,
            tangents: Vec::new(),
            indices,
        }
    }

    #[test]
    fn edge_limit_drops_sub_pixel_triangles_and_keeps_the_surface_error() {
        let railing = railing();
        assert_eq!(tri_count(&railing), 500, "the fixture is the railing");
        // 60 m through the duel lens: a 0.3 m edge is 9.3 px (kept), a 1 cm
        // edge is 0.31 px (dropped under MIN_EDGE_PX = 0.6).
        let policy = LodEdgePolicy::new(60.0, 1080, (60.0f32.to_radians() * 0.5).tan());
        assert!(!policy.sub_pixel(0.3), "the wall must survive");
        assert!(policy.sub_pixel(0.01), "the detail must be sub-pixel");
        let capped = edge_limit(&railing, &policy);
        assert_eq!(capped.dropped, 120, "the 120 detail triangles go");
        assert_eq!(tri_count(&capped.mesh), 380, "500 -> 380, as recorded");
        assert_eq!(
            capped.mesh.positions.len(),
            380 * 4 / 2,
            "no orphan lane survives"
        );
        // The detail sat 1 mm over the cell that covers it, so nothing moved.
        let stray = max_deviation_m(&railing, &capped.mesh);
        assert!(
            stray <= 0.9,
            "the surface strayed {stray} m, over the 0.9 m cap"
        );
        assert!(
            stray <= 0.002,
            "the railing cover keeps the stray at the 1 mm offset"
        );
    }

    #[test]
    fn edge_limit_leaves_a_prop_at_five_metres_untouched() {
        // The same mesh at 5 m: a 1 cm edge is 3.7 px, so nothing is dropped.
        let railing = railing();
        let policy = LodEdgePolicy::new(5.0, 1080, (60.0f32.to_radians() * 0.5).tan());
        let capped = edge_limit(&railing, &policy);
        assert_eq!(capped.dropped, 0, "a close prop keeps every triangle");
        assert_eq!(capped.mesh, railing, "byte for byte untouched");
    }

    #[test]
    fn edge_limit_level_zero_is_never_capped() {
        let source = sphere(12, 24);
        let policies = [
            LodEdgePolicy::new(10.0, 1080, 0.577),
            LodEdgePolicy::new(20.0, 1080, 0.577),
            LodEdgePolicy::new(40.0, 1080, 0.577),
            LodEdgePolicy::new(60.0, 1080, 0.577),
        ];
        let chain = measure_lod_chain_with_edges(&source, 4, &policies).expect("chain");
        assert_eq!(
            chain[0].mesh, source,
            "LOD0 stays the source, byte for byte"
        );
        assert!(!chain[0].edge_limited, "LOD0 is never edge-limited");
        assert_eq!(chain.len(), 4);
        // A plain chain is equal to a capped one whose policy removes nothing.
        let plain = measure_lod_chain(&source, 4).expect("plain");
        assert!(plain.iter().all(|level| !level.edge_limited));
    }

    #[test]
    fn lod_error_chain_has_four_levels_of_falling_triangles_and_rising_error() {
        let source = sphere(12, 24);
        let chain = measure_lod_chain(&source, 4).expect("chain");
        assert_eq!(chain.len(), 4, "four levels, one per arena band");
        assert_eq!(chain[0].mesh, source, "LOD0 is the source, byte for byte");
        assert_eq!(chain[0].error_m, 0.0);
        assert_eq!(chain[0].triangles, tri_count(&source));
        for pair in chain.windows(2) {
            assert!(
                pair[1].triangles < pair[0].triangles,
                "LOD{} has {} triangles, not fewer than LOD{}'s {}",
                pair[1].level,
                pair[1].triangles,
                pair[0].level,
                pair[0].triangles
            );
            // Non-decreasing, not strictly: the running-maximum clamp means a
            // coarser level may measure no worse than the level before it,
            // but it must never measure better.
            assert!(
                pair[1].error_m >= pair[0].error_m,
                "LOD{} error {} m fell below LOD{}'s {} m",
                pair[1].level,
                pair[1].error_m,
                pair[0].level,
                pair[0].error_m
            );
        }
        assert!(
            chain[1].error_m > 0.0,
            "the first simplify must move the surface"
        );
        // A 0.9 m sphere simplified to an eighth of its triangles strays by
        // centimetres, never metres.
        assert!(chain[3].error_m < 0.25, "got {} m", chain[3].error_m);
    }

    #[test]
    fn lod_error_cook_gives_the_same_bytes_and_errors_twice() {
        let source = sphere(9, 16);
        let first = measure_lod_chain(&source, 4).expect("first");
        let second = measure_lod_chain(&source, 4).expect("second");
        assert_eq!(first, second, "same input, same bytes and same errors x2");
    }

    #[test]
    fn lod_error_deviation_is_symmetric() {
        let source = sphere(8, 12);
        let chain = measure_lod_chain(&source, 3).expect("chain");
        let forward = max_deviation_m(&source, &chain[2].mesh);
        let back = max_deviation_m(&chain[2].mesh, &source);
        assert_eq!(forward, back, "the bound does not depend on argument order");
        assert_eq!(max_deviation_m(&source, &source), 0.0);
    }

    #[test]
    fn lod_error_lod3_at_60m_is_measured_and_not_sub_pixel() {
        // Measured, not assumed: on a 0.9 m fighter-scale sphere the
        // one-eighth LOD3 strays ~0.12 m, which is ~3.8 px at 60 m through
        // the 60 degree lens, so the shipped ratio chain does NOT meet the
        // 1 px rule at the distance the band-projected table first asks for
        // LOD3. The pick's thresholds must therefore come from these
        // measured errors (RL61's "coarsest LOD under 1 px"), which is a
        // step-2 decision, not something this cook can pretend.
        let source = sphere(12, 24);
        let chain = measure_lod_chain(&source, 4).expect("chain");
        let error_m = chain[3].error_m;
        assert!(
            (0.10..0.15).contains(&error_m),
            "LOD3 error measured {error_m} m; the pin expects ~0.12 m"
        );
        let px_60 = error_px(error_m, 60.0);
        assert!(
            (3.0..5.0).contains(&px_60),
            "LOD3 error is {px_60:.3} px at 60 m; the pin expects ~3.8 px"
        );
        // The distance where LOD3's error finally covers under one pixel.
        let sub_pixel_m = error_m * 1080.0 / (60.0f32.to_radians() * 0.5).tan();
        assert!(
            (200.0..260.0).contains(&sub_pixel_m),
            "LOD3 is sub-pixel past {sub_pixel_m:.0} m"
        );
        // LOD1 is the mild level: a fraction of LOD3's error, so the chain
        // stays useful while the thresholds are re-derived from measurement.
        let px_lod1 = error_px(chain[1].error_m, 60.0);
        assert!(
            px_lod1 < px_60 / 2.0,
            "LOD1 error {:.4} m is {px_lod1:.3} px at 60 m, not clearly milder than LOD3's {px_60:.3} px",
            chain[1].error_m
        );
    }
}
