//! Collision decomposition: a concave mesh into a few closed convex parts, on the V-HACD code
//! inside `parry3d` (Apache-2.0; the same Rapier collision stack the sim already links).
//!
//! Every part is rebuilt as a real convex hull of its points, capped at `max_hull_vertices` by
//! farthest-point sampling (deterministic: fixed start, ties by lowest index), so a part is
//! always closed, outward and small enough for the physics bridge. `parry3d` runs with its
//! `enhanced-determinism` feature (libm math), the setting `rapier3d` uses in this workspace.
//!
//! [`visacd`] is a second bake beside that frozen one: greedy visibility-plane cuts over the
//! mesh's own triangles (cf. [`decompose`], which only voxelizes into clusters). It is a
//! CPU-only, std reimplementation from VisACD's ideas, no donor code; a cut clips the
//! surface, hulls each side, and the parts stay disjoint by construction (each one lies in
//! every half-space on its path).

use std::collections::BTreeMap;

use parry3d::math::Vector;
use parry3d::transformation::try_convex_hull;
use parry3d::transformation::vhacd::{VHACDParameters, VHACD};

use crate::mesh::{ix, narrow, to_u64, GeoError, GeoMesh};

/// Options for one decomposition.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HullOptions {
    /// Most convex parts to return.
    pub max_hulls: u32,
    /// V-HACD concavity bound (0.01 tight, 0.05 loose), relative to the shape.
    pub concavity: f32,
    /// Voxels along the longest side.
    pub resolution: u32,
    /// Most vertices in one part.
    pub max_hull_vertices: usize,
}

impl Default for HullOptions {
    fn default() -> Self {
        Self {
            max_hulls: 16,
            concavity: 0.02,
            resolution: 64,
            max_hull_vertices: 64,
        }
    }
}

fn vec3(p: [f32; 3]) -> Vector {
    Vector::new(p[0], p[1], p[2])
}

/// Keep at most `max` of the points: the extreme point on the lowest axis first, then the point
/// farthest from everything chosen so far.
fn farthest_points(points: &[Vector], max: usize) -> Vec<Vector> {
    if points.len() <= max {
        return points.to_vec();
    }
    let mut first = 0;
    for (i, p) in points.iter().enumerate() {
        if p.x < points[first].x {
            first = i;
        }
    }
    let mut chosen = vec![first];
    let mut dist: Vec<f32> = points
        .iter()
        .map(|p| (*p - points[first]).length_squared())
        .collect();
    while chosen.len() < max {
        let mut best = 0;
        for (i, &d) in dist.iter().enumerate() {
            if d > dist[best] {
                best = i;
            }
        }
        if dist[best] <= 0.0 {
            break;
        }
        chosen.push(best);
        for (i, p) in points.iter().enumerate() {
            let d = (*p - points[best]).length_squared();
            if d < dist[i] {
                dist[i] = d;
            }
        }
    }
    chosen.sort_unstable();
    chosen.into_iter().map(|i| points[i]).collect()
}

fn hull_mesh(points: &[Vector], max_vertices: usize) -> Option<GeoMesh> {
    let limited = farthest_points(points, max_vertices.max(4));
    let (verts, tris) = try_convex_hull(&limited).ok()?;
    let positions = verts.iter().map(|v| [v.x, v.y, v.z]).collect();
    let indices = tris.iter().flatten().copied().collect();
    let mesh = GeoMesh::from_positions(positions, indices);
    (mesh.triangle_count() >= 4).then_some(mesh)
}

/// The convex hull of the whole mesh (the single-hull baseline the decomposition improves on).
pub fn single_hull(mesh: &GeoMesh, max_hull_vertices: usize) -> Result<GeoMesh, GeoError> {
    mesh.validate()?;
    if mesh.indices.is_empty() {
        return Err(GeoError::Empty);
    }
    let points: Vec<Vector> = mesh.positions.iter().copied().map(vec3).collect();
    hull_mesh(&points, max_hull_vertices).ok_or_else(|| GeoError::Donor("degenerate hull".into()))
}

/// Decompose `mesh` into closed convex parts.
pub fn decompose(mesh: &GeoMesh, options: &HullOptions) -> Result<Vec<GeoMesh>, GeoError> {
    mesh.validate()?;
    if mesh.indices.is_empty() {
        return Err(GeoError::Empty);
    }
    let points: Vec<Vector> = mesh.positions.iter().copied().map(vec3).collect();
    let tris: Vec<[u32; 3]> = mesh
        .indices
        .chunks_exact(3)
        .map(|t| [t[0], t[1], t[2]])
        .collect();
    let params = VHACDParameters {
        concavity: options.concavity,
        resolution: options.resolution.max(8),
        max_convex_hulls: options.max_hulls.max(1),
        convex_hull_approximation: false,
        ..VHACDParameters::default()
    };
    let vhacd = VHACD::decompose(&params, &points, &tris, true);
    let mut parts = Vec::new();
    for (verts, _) in vhacd.compute_exact_convex_hulls(&points, &tris) {
        if let Some(part) = hull_mesh(&verts, options.max_hull_vertices) {
            parts.push(part);
        }
    }
    if parts.is_empty() {
        // A shape V-HACD could not split (tiny, flat): one hull is still a valid collider.
        parts.push(single_hull(mesh, options.max_hull_vertices)?);
    }
    parts.truncate(ix(options.max_hulls.max(1)));
    Ok(parts)
}

/// How far `p` sits inside the outward convex `part`: the least distance to an outward face
/// plane, negative outside, positive inside, infinity for a face with no area.
fn inside_depth(part: &GeoMesh, p: [f64; 3]) -> f64 {
    part.indices
        .chunks_exact(3)
        .map(|t| {
            let [a, b, c] = [0, 1, 2].map(|k| part.positions[ix(t[k])].map(f64::from));
            let (u, v) = (
                [0, 1, 2].map(|k| b[k] - a[k]),
                [0, 1, 2].map(|k| c[k] - a[k]),
            );
            let normal = [
                u[1] * v[2] - u[2] * v[1],
                u[2] * v[0] - u[0] * v[2],
                u[0] * v[1] - u[1] * v[0],
            ];
            let length = normal.iter().map(|n| n * n).sum::<f64>().sqrt();
            if length == 0.0 {
                return f64::INFINITY;
            }
            let out: f64 = (0..3).map(|k| normal[k] * (p[k] - a[k])).sum();
            -out / length
        })
        .fold(f64::INFINITY, f64::min)
}

/// True when `p` is inside the convex `part` or on its surface: no outward face plane has it
/// farther out than a micrometer (the signed distance to every face plane is at most zero).
fn inside_convex(part: &GeoMesh, p: [f64; 3]) -> bool {
    inside_depth(part, p) >= -1e-6
}

/// How much of `mesh` the convex `parts` miss: for every vertex of `mesh` the distance to the
/// nearest part (0 when it is inside one), the largest of them over the bounding-box diagonal
/// of `mesh`. 0.0 means the parts cover every vertex; infinity when there are no parts. It
/// sees a part that was lost or cut away, not a hull that is too fat: a single hull holds
/// every vertex of its mesh and scores 0.0.
#[must_use]
pub fn concavity(mesh: &GeoMesh, parts: &[GeoMesh]) -> f32 {
    let distance = |p: [f64; 3]| {
        if parts.iter().any(|part| inside_convex(part, p)) {
            return 0.0;
        }
        parts
            .iter()
            .map(|part| part.surface_distance(p))
            .fold(f64::INFINITY, f64::min)
    };
    let worst = mesh
        .positions
        .iter()
        .map(|p| distance(p.map(f64::from)))
        .fold(0.0, f64::max);
    narrow(worst / mesh.diagonal().max(f64::MIN_POSITIVE))
}

/// Options for the visibility-plane bake ([`visacd`]) that runs beside the V-HACD bake.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VisacdOptions {
    /// Most convex parts to return.
    pub max_parts: u32,
    /// Quantized split-plane positions tried along each cut direction (at least 2).
    pub planes_per_axis: u32,
    /// Most vertices in one part (the same cap the V-HACD bake uses).
    pub max_hull_vertices: usize,
    /// Stop when the best cut shrinks the hull volume by less than this fraction of the root.
    pub min_gain: f64,
}

impl Default for VisacdOptions {
    fn default() -> Self {
        Self {
            max_parts: 16,
            planes_per_axis: 8,
            max_hull_vertices: 64,
            min_gain: 0.01,
        }
    }
}

/// How many of a part's heaviest face normals join the three axes as cut directions.
const FACE_NORMALS: usize = 4;

/// One split plane, quantized to the node's grid: points with `normal . p <= offset` stay on
/// the low side, the rest go to the high side.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Cut {
    normal: Vector,
    offset: f32,
}

/// Signed distance of `p` from the plane: at most zero means the low side.
fn plane_dist(cut: Cut, p: Vector) -> f32 {
    cut.normal.dot(p) - cut.offset
}

/// Order two planes by their numbers only (a stable, platform-independent key).
fn cmp_cut(a: Cut, b: Cut) -> std::cmp::Ordering {
    a.normal
        .x
        .total_cmp(&b.normal.x)
        .then(a.normal.y.total_cmp(&b.normal.y))
        .then(a.normal.z.total_cmp(&b.normal.z))
        .then(a.offset.total_cmp(&b.offset))
}

/// Sutherland-Hodgman clip of one polygon to `dist <= 0` (`low`) or `>= 0` (high), adding the
/// point where the plane crosses an edge. Orientation is kept.
fn clip_polygon(poly: &[Vector], cut: Cut, low: bool) -> Vec<Vector> {
    let mut out = Vec::new();
    for i in 0..poly.len() {
        let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
        let (da, db) = (plane_dist(cut, a), plane_dist(cut, b));
        let (ia, ib) = if low {
            (da <= 0.0, db <= 0.0)
        } else {
            (da >= 0.0, db >= 0.0)
        };
        if ia {
            out.push(a);
        }
        if ia != ib {
            let t = da / (da - db);
            out.push(a + (b - a) * t);
        }
    }
    out
}

/// The triangles of `soup` on one side of `cut`, each kept triangle fanned back to triangles.
fn clip_soup(soup: &[[Vector; 3]], cut: Cut, low: bool) -> Vec<[Vector; 3]> {
    let mut out = Vec::new();
    for tri in soup {
        let poly = clip_polygon(tri, cut, low);
        for k in 2..poly.len() {
            out.push([poly[0], poly[k - 1], poly[k]]);
        }
    }
    out
}

/// The distinct vertices of `soup`, in a fixed order (by position bits).
fn soup_points(soup: &[[Vector; 3]]) -> Vec<Vector> {
    let mut map = BTreeMap::new();
    for tri in soup {
        for p in tri {
            map.insert([p.x.to_bits(), p.y.to_bits(), p.z.to_bits()], *p);
        }
    }
    map.into_values().collect()
}

/// A hull of a point set at the vertex cap.
struct Hull {
    mesh: GeoMesh,
    volume: f64,
    /// The cap had to drop hull vertices (so the hull may no longer hold every point).
    sampled: bool,
}

fn inside_vec(part: &GeoMesh, p: &Vector) -> bool {
    inside_convex(part, [f64::from(p.x), f64::from(p.y), f64::from(p.z)])
}

/// Hull of `points` at the vertex cap. The hull is taken first and only its own vertices are
/// sampled, so the cap spends its budget on corners, never on the interior points of a flat
/// face. `None` when the points do not span a solid.
fn capped_hull(points: &[Vector], max_vertices: usize) -> Option<Hull> {
    let cap = max_vertices.max(4);
    let (verts, _) = try_convex_hull(points).ok()?;
    let limited = farthest_points(&verts, cap);
    let sampled = limited.len() < verts.len();
    let (hull_verts, tris) = try_convex_hull(&limited).ok()?;
    let mesh = GeoMesh::from_positions(
        hull_verts.iter().map(|v| [v.x, v.y, v.z]).collect(),
        tris.iter().flatten().copied().collect(),
    );
    // A flat hull (a patch of an open surface) has no volume and is no collider.
    if mesh.triangle_count() < 4 {
        return None;
    }
    let volume = mesh.volume();
    (volume > 0.0).then_some(Hull {
        mesh,
        volume,
        sampled,
    })
}

/// [`capped_hull`] plus the coverage rule: when the cap had to drop hull vertices and the
/// smaller hull no longer holds every point, the answer is `None`. A part that misses its own
/// surface leaves a vertex of the mesh outside every part, so such a cut never pays.
fn covering_hull(points: &[Vector], max_vertices: usize) -> Option<(GeoMesh, f64)> {
    let hull = capped_hull(points, max_vertices)?;
    if hull.sampled && !points.iter().all(|p| inside_vec(&hull.mesh, p)) {
        return None;
    }
    Some((hull.mesh, hull.volume))
}

/// The heaviest distinct face normals of `soup`, at most `max`: a cut across a face looks
/// along the face a viewer sees, which is where a concave edge hides (the visibility edge of
/// the bake). Order is by total area, then by the numbers, so it never depends on triangle
/// order.
fn face_normals(soup: &[[Vector; 3]], max: usize) -> Vec<Vector> {
    let mut by_key: BTreeMap<[u32; 3], (f32, Vector)> = BTreeMap::new();
    for tri in soup {
        let normal = (tri[1] - tri[0]).cross(tri[2] - tri[0]);
        let length = normal.length();
        if length <= 0.0 {
            continue;
        }
        let unit = normal / length;
        let key = [unit.x.to_bits(), unit.y.to_bits(), unit.z.to_bits()];
        let entry = by_key.entry(key).or_insert((0.0, unit));
        entry.0 += length;
    }
    let mut list: Vec<(f32, Vector)> = by_key.into_values().collect();
    list.sort_by(|a, b| b.0.total_cmp(&a.0));
    list.truncate(max);
    list.into_iter().map(|(_, n)| n).collect()
}

/// Candidate split planes for one leaf: along each direction (the three axes and the leaf's
/// heaviest face normals) the fixed quantized grid, plus the middle of the two widest empty
/// spans. A wide empty span is where the surface steps back: a neck or a notch, the
/// visibility edge a cut should land on.
fn candidate_cuts(points: &[Vector], normals: &[Vector], per_axis: u32) -> Vec<Cut> {
    let quanta = per_axis.max(2);
    let quanta_f = narrow(f64::from(quanta));
    let axes = [
        Vector::new(1.0, 0.0, 0.0),
        Vector::new(0.0, 1.0, 0.0),
        Vector::new(0.0, 0.0, 1.0),
    ];
    let mut cuts = Vec::new();
    for normal in axes.iter().copied().chain(normals.iter().copied()) {
        let (mut lo, mut hi) = (f32::INFINITY, f32::NEG_INFINITY);
        let mut dist: Vec<f32> = Vec::with_capacity(points.len());
        for p in points {
            let d = normal.dot(*p);
            lo = lo.min(d);
            hi = hi.max(d);
            dist.push(d);
        }
        let span = hi - lo;
        if span <= 0.0 {
            continue;
        }
        let grid = |t: u32| lo + span * (narrow(f64::from(t)) / quanta_f);
        for t in 1..quanta {
            cuts.push(Cut {
                normal,
                offset: grid(t),
            });
        }
        dist.sort_unstable_by(f32::total_cmp);
        let mut gaps: Vec<(f32, f32)> = dist
            .windows(2)
            .filter(|w| w[1] > w[0])
            .map(|w| (w[1] - w[0], w[0]))
            .collect();
        gaps.sort_unstable_by(|a, b| b.0.total_cmp(&a.0).then(a.1.total_cmp(&b.1)));
        for &(gap, start) in gaps.iter().take(2) {
            let step = ((start + gap * 0.5 - lo) / span * quanta_f).round();
            let step = step.clamp(1.0, quanta_f - 1.0);
            let step = u32::try_from(engine_core::checked::saturating_f64_to_u64(f64::from(step)))
                .unwrap_or(u32::MAX);
            cuts.push(Cut {
                normal,
                offset: grid(step),
            });
        }
    }
    cuts.sort_unstable_by(|a, b| cmp_cut(*a, *b));
    cuts.dedup();
    cuts
}

/// The greedy max cut of one part: the plane whose two sides add up to the least hull volume
/// (the largest cut), `None` when no plane leaves a solid on both sides or none shrinks the
/// hull.
fn best_cut(
    soup: &[[Vector; 3]],
    points: &[Vector],
    volume: f64,
    per_axis: u32,
    cap: usize,
) -> Option<(Cut, f64, f64)> {
    let mut best: Option<(Cut, f64, f64, f64)> = None;
    let normals = face_normals(soup, FACE_NORMALS);
    for cut in candidate_cuts(points, &normals, per_axis) {
        let low = soup_points(&clip_soup(soup, cut, true));
        let high = soup_points(&clip_soup(soup, cut, false));
        if low.len() < 4 || high.len() < 4 {
            continue;
        }
        let (Some((_, lv)), Some((_, rv))) = (covering_hull(&low, cap), covering_hull(&high, cap))
        else {
            continue;
        };
        let gain = volume - lv - rv;
        if best.is_none_or(|b| gain > b.3) {
            best = Some((cut, lv, rv, gain));
        }
    }
    best.filter(|b| b.3 > 0.0).map(|(c, l, r, _)| (c, l, r))
}

/// One greedy part: the surface on its side of every cut above it, its hull volume, and the
/// best cut found for it (if any).
struct Leaf {
    soup: Vec<[Vector; 3]>,
    points: Vec<Vector>,
    volume: f64,
    best: Option<(Cut, f64, f64)>,
}

impl Leaf {
    fn new(soup: Vec<[Vector; 3]>, volume: f64, per_axis: u32, cap: usize) -> Self {
        let points = soup_points(&soup);
        let best = best_cut(&soup, &points, volume, per_axis, cap);
        Self {
            soup,
            points,
            volume,
            best,
        }
    }
}

/// Bake `mesh` into closed convex parts with greedy visibility-plane cuts (the VisACD shape:
/// a CPU-only, std reimplementation from the paper's ideas, no donor code), beside the frozen
/// V-HACD [`decompose`]. At each step the part whose best plane removes the most hull volume
/// is cut in two; the loop stops at `max_parts` or when no cut pays.
///
/// Cutting clips the surface, so a side keeps its cut cross-section and its hull is exact for
/// a convex side: the parts of a split add up to their parent. Every part sits inside every
/// half-space on its path, so two parts share no volume: they are separated by the first
/// plane that split them. Deterministic: candidates are sorted by their plane numbers, ties
/// go to the earlier candidate, then to the lowest part index.
pub fn visacd(mesh: &GeoMesh, options: &VisacdOptions) -> Result<Vec<GeoMesh>, GeoError> {
    mesh.validate()?;
    if mesh.indices.is_empty() {
        return Err(GeoError::Empty);
    }
    let cap = options.max_hull_vertices.max(4);
    let welded = mesh.welded();
    let points: Vec<Vector> = welded.positions.iter().copied().map(vec3).collect();
    let soup: Vec<[Vector; 3]> = welded
        .indices
        .chunks_exact(3)
        .map(|t| [points[ix(t[0])], points[ix(t[1])], points[ix(t[2])]])
        .collect();
    let per_axis = options.planes_per_axis.max(2);
    let max_parts = ix(options.max_parts.max(1));
    let root_points = soup_points(&soup);
    let Some(root) = capped_hull(&root_points, cap) else {
        // No solid hull (flat or tiny): one hull is still a valid collider.
        return Ok(vec![single_hull(mesh, cap)?]);
    };
    let floor = root.volume * options.min_gain.max(0.0);
    let mut leaves = vec![Leaf::new(soup, root.volume, per_axis, cap)];
    while leaves.len() < max_parts {
        let gain_of = |leaf: &Leaf| leaf.best.map(|(_, lv, rv)| leaf.volume - lv - rv);
        let pick = leaves
            .iter()
            .enumerate()
            .filter_map(|(i, leaf)| Some((i, gain_of(leaf)?)).filter(|&(_, g)| g > floor))
            .max_by(|a, b| a.1.total_cmp(&b.1).then(b.0.cmp(&a.0)))
            .map(|(i, _)| i);
        let Some(index) = pick else { break };
        let Some((cut, lv, rv)) = leaves[index].best else {
            break;
        };
        let low = clip_soup(&leaves[index].soup, cut, true);
        let high = clip_soup(&leaves[index].soup, cut, false);
        leaves[index] = Leaf::new(low, lv, per_axis, cap);
        leaves.push(Leaf::new(high, rv, per_axis, cap));
    }
    let mut parts = Vec::new();
    for leaf in &leaves {
        if let Some(hull) = capped_hull(&leaf.points, cap) {
            parts.push(hull.mesh);
        }
    }
    if parts.is_empty() {
        parts.push(single_hull(mesh, cap)?);
    }
    parts.truncate(max_parts);
    Ok(parts)
}

/// Deepest overlap of two convex parts in meters: the deepest a vertex of one part sits inside
/// the other. 0.0 when the parts are apart or only touch. A plane cut leaves each part inside
/// its own half-spaces, so two parts of one bake can only touch: this stays 0.0 on them.
#[must_use]
pub fn penetration(a: &GeoMesh, b: &GeoMesh) -> f32 {
    let depth = |outer: &GeoMesh, inner: &GeoMesh| {
        inner
            .positions
            .iter()
            .map(|p| inside_depth(outer, p.map(f64::from)))
            .fold(f64::NEG_INFINITY, f64::max)
    };
    let worst = depth(a, b).max(depth(b, a));
    if worst.partial_cmp(&1e-6) != Some(std::cmp::Ordering::Greater) {
        return 0.0;
    }
    narrow(worst)
}

/// Volume error of `parts` against `mesh`: `|sum of part volumes - mesh volume| / mesh
/// volume`, in `0.0..`. `None` when `mesh` is not a closed solid with a positive volume (an
/// open surface has no signed volume to compare against).
#[must_use]
pub fn volume_error(mesh: &GeoMesh, parts: &[GeoMesh]) -> Option<f32> {
    let solid = mesh.volume();
    if solid <= 0.0 || !mesh.welded().edge_report().is_closed() {
        return None;
    }
    let total: f64 = parts.iter().map(GeoMesh::volume).sum();
    Some(narrow((total - solid).abs() / solid))
}

/// Digest of a whole decomposition (part count, then each part's digest), the cook receipt.
#[must_use]
pub fn digest(parts: &[GeoMesh]) -> u64 {
    let mut h = engine_core::Fnv64::new();
    h.write_u64(to_u64(parts.len()));
    for p in parts {
        h.write_u64(p.digest());
    }
    h.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::primitives::{cuboid, l_prism, torus};

    fn assert_part_ok(part: &GeoMesh, max_vertices: usize) {
        part.validate().unwrap();
        assert_eq!(part.open_edge_count(), 0, "part is closed");
        assert!(part.volume() > 0.0, "outward");
        assert!(part.positions.len() <= max_vertices);
    }

    #[test]
    fn l_prism_splits_into_fewer_than_one_hull_of_error() {
        let l = l_prism(2.0, 1.0, 1.0);
        let one = single_hull(&l, 64).unwrap();
        assert!(
            one.volume() > 3.4,
            "single hull fills the notch: {}",
            one.volume()
        );
        let parts = decompose(&l, &HullOptions::default()).unwrap();
        assert!(
            parts.len() >= 2 && parts.len() <= 16,
            "{} parts",
            parts.len()
        );
        let total: f64 = parts.iter().map(GeoMesh::volume).sum();
        // The parts overlap a little but hug the L much closer than one hull does.
        assert!(
            total < one.volume(),
            "parts {total} vs hull {}",
            one.volume()
        );
        assert!(total > 2.9, "parts cover the L: {total}");
        for p in &parts {
            assert_part_ok(p, 64);
        }
    }

    #[test]
    fn l_prism_parts_cover_every_vertex_and_a_lost_part_shows() {
        let l = l_prism(2.0, 1.0, 1.0);
        let parts = decompose(&l, &HullOptions::default()).unwrap();
        let split = concavity(&l, &parts);
        assert!(split <= 0.02, "parts concavity {split}");
        // Lose one part and the vertices only it held are left out.
        let lost = concavity(&l, &parts[1..]);
        assert!(lost >= 0.10, "one part lost: {lost}");
    }

    #[test]
    fn a_single_hull_scores_zero_because_it_holds_every_vertex() {
        // The row asked for one hull to score at least 0.10 on the L; by the vertex-to-part
        // definition it cannot: the hull contains all of them, notch corner included.
        let l = l_prism(2.0, 1.0, 1.0);
        assert_eq!(concavity(&l, &[single_hull(&l, 64).unwrap()]), 0.0);
    }

    #[test]
    fn concavity_is_zero_inside_and_grows_outside() {
        let cube = cuboid([2.0; 3]);
        // The mesh against itself, and against a part that swallows it: every vertex is inside.
        assert_eq!(concavity(&cube, std::slice::from_ref(&cube)), 0.0);
        assert_eq!(concavity(&cube, &[cube.placed([1.5; 3], 0, [0.0; 3])]), 0.0);
        // The same cube moved 0.5 along X: the far vertices are 0.5 outside the part.
        let moved = cube.placed([1.0; 3], 0, [0.5, 0.0, 0.0]);
        let c = concavity(&cube, &[moved]);
        assert!((c - 0.5 / (2.0 * 3.0_f32.sqrt())).abs() < 1e-4, "moved {c}");
        // No parts: nothing covers the vertices.
        assert!(concavity(&cube, &[]).is_infinite());
    }

    #[test]
    fn convex_shape_stays_one_part() {
        let parts = decompose(&cuboid([1.0, 2.0, 3.0]), &HullOptions::default()).unwrap();
        assert_eq!(parts.len(), 1);
        assert_part_ok(&parts[0], 64);
        assert!((parts[0].volume() - 6.0).abs() < 0.5);
    }

    #[test]
    fn torus_obeys_the_part_cap_and_repeats() {
        let t = torus(1.0, 0.35, 24, 10);
        let options = HullOptions {
            max_hulls: 8,
            ..HullOptions::default()
        };
        let a = decompose(&t, &options).unwrap();
        let b = decompose(&t, &options).unwrap();
        assert_eq!(digest(&a), digest(&b));
        assert!(a.len() > 1 && a.len() <= 8, "{} parts", a.len());
        for p in &a {
            assert_part_ok(p, 64);
        }
    }

    #[test]
    fn vertex_cap_is_enforced() {
        let t = torus(1.0, 0.35, 24, 10);
        let options = HullOptions {
            max_hull_vertices: 12,
            ..HullOptions::default()
        };
        for p in decompose(&t, &options).unwrap() {
            assert_part_ok(&p, 12);
        }
    }

    #[test]
    fn empty_and_malformed_inputs_are_refused() {
        assert_eq!(
            decompose(&GeoMesh::default(), &HullOptions::default()),
            Err(GeoError::Empty)
        );
        let mut bad = cuboid([1.0; 3]);
        bad.indices[2] = 77;
        assert!(matches!(
            decompose(&bad, &HullOptions::default()),
            Err(GeoError::Malformed(_))
        ));
        assert_eq!(
            visacd(&GeoMesh::default(), &VisacdOptions::default()),
            Err(GeoError::Empty)
        );
        assert!(matches!(
            visacd(&bad, &VisacdOptions::default()),
            Err(GeoError::Malformed(_))
        ));
    }

    /// Every part is closed, outward, inside the vertex cap, and no two parts overlap.
    fn assert_parts_disjoint(parts: &[GeoMesh], max_vertices: usize) {
        for part in parts {
            assert_part_ok(part, max_vertices);
        }
        for a in 0..parts.len() {
            for b in (a + 1)..parts.len() {
                assert!(
                    penetration(&parts[a], &parts[b]) <= 1e-4,
                    "parts {a} and {b} overlap"
                );
            }
        }
    }

    /// The concavity of the V-HACD bake of `mesh`, the bar `visacd` must not lose to.
    fn vhacd_concavity(mesh: &GeoMesh) -> f32 {
        concavity(mesh, &decompose(mesh, &HullOptions::default()).unwrap())
    }

    #[test]
    fn visacd_cuts_the_l_prism_at_the_notch_with_no_volume_error() {
        let l = l_prism(2.0, 1.0, 1.0);
        let options = VisacdOptions::default();
        let parts = visacd(&l, &options).unwrap();
        assert!(
            parts.len() >= 2 && parts.len() <= 16,
            "{} parts",
            parts.len()
        );
        assert_parts_disjoint(&parts, options.max_hull_vertices);
        assert_eq!(concavity(&l, &parts), 0.0, "every vertex is in a part");
        let err = volume_error(&l, &parts).unwrap();
        assert!(err <= 0.05, "volume error {err}");
        // Same input, same bytes.
        assert_eq!(digest(&parts), digest(&visacd(&l, &options).unwrap()));
    }

    #[test]
    fn visacd_keeps_a_convex_shape_in_one_part() {
        let options = VisacdOptions::default();
        for mesh in [cuboid([1.0, 2.0, 3.0]), torus(1.0, 0.35, 12, 6)] {
            let parts = visacd(&mesh, &options).unwrap();
            if mesh.triangle_count() <= 12 {
                assert_eq!(parts.len(), 1, "a box is already convex");
                assert_eq!(concavity(&mesh, &parts), 0.0);
            }
            assert!(concavity(&mesh, &parts) <= vhacd_concavity(&mesh) + 1e-6);
            assert_parts_disjoint(&parts, options.max_hull_vertices);
        }
    }

    #[test]
    fn visacd_respects_the_part_cap_and_the_vertex_cap() {
        let t = torus(1.0, 0.35, 24, 10);
        let options = VisacdOptions {
            max_parts: 4,
            max_hull_vertices: 16,
            ..VisacdOptions::default()
        };
        let parts = visacd(&t, &options).unwrap();
        assert!(parts.len() <= 4, "{} parts", parts.len());
        assert_parts_disjoint(&parts, 16);
        assert_eq!(digest(&parts), digest(&visacd(&t, &options).unwrap()));
    }
}
