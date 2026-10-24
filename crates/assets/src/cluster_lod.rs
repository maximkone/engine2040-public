//! cluster_lod: one cluster-LOD hierarchy over a cooked mesh.
//!
//! Owner: `engine-assets`. Consumer: the far crowd and the debris field pick
//! their triangles from one tree instead of a per-mesh 4-level chain
//! (`crates/render/src/arena_lod.rs`): a cut at one projected-error threshold
//! covers every triangle exactly once, so 100+ seated bodies and broken chunks
//! stop carrying LOD0 triangles at distance. Status: hierarchy build only; the
//! render select lands next.
//!
//! Level 0 nodes are the meshlet cook's clusters ([`super::meshlet::build_meshlets`],
//! 64 v / 124 t). Each higher level takes up to [`ClodConfig::group_size`]
//! neighbouring nodes, simplifies their merged triangles toward
//! [`ClodConfig::simplify_ratio`] of the triangle count with
//! [`super::collapse::simplify`], and records the measured deviation as the
//! group's error. A group whose simplification stays over
//! [`ClodConfig::simplify_threshold`] is stuck and ends its branch, exactly like
//! meshoptimizer's `clodBuild` terminal group.
//!
//! Bounds and error follow meshoptimizer's `demo/clusterlod.h` (MIT, Zeux):
//! the group bound is the merge of its children's spheres, and the error is the
//! running maximum, `max(previous_error * 1.0, current_error) + current_error * 0`,
//! so it never decreases up the tree. The projected-error rule is the donor's
//! `clodBounds` comment: `error / max(dist(center) - radius, znear) * proj * 0.5`,
//! multiplied by the frame height for pixels.
//!
//! Units: meters in, meters and pixels out. Determinism: index-order loops,
//! sorted triangle keys and node ids, `total_cmp` float order, no hash-map
//! iteration; the same mesh and config emit the same bytes twice. Errors: typed
//! [`MeshCookError`], never a panic.

use super::collapse::simplify;
use super::cooked::CookedMesh;
use super::lod_error::max_deviation_m;
use super::mesh_cook::{count_u32, tri_count, MeshCookError};
use super::meshlet::{build_meshlets, count_f32, meshlet_bounds, Meshlet, MeshletError};

/// Triangle target of one group as a fraction of its merged triangle count;
/// meshoptimizer's `clodDefaultConfig` value.
pub const DEFAULT_SIMPLIFY_RATIO: f32 = 0.5;
/// A group whose simplified mesh keeps more than this fraction of its merged
/// triangles is stuck; the same default as the donor's `simplify_threshold`.
pub const DEFAULT_SIMPLIFY_THRESHOLD: f32 = 0.85;
/// Nodes merged into one group per level; the donor groups 16 clusters, a
/// crowd body is far smaller, so four keeps the hierarchy shallow.
pub const DEFAULT_GROUP_SIZE: usize = 4;
/// Most levels a cook builds; a guard, not a target.
pub const MAX_LEVELS: usize = 8;
/// Most nodes one hierarchy holds; guards the cook.
pub const MAX_NODES: usize = 200_000;

/// How one hierarchy is built.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClodConfig {
    /// Triangle target of a group as a fraction of its merged triangles.
    pub simplify_ratio: f32,
    /// A group above this kept fraction of triangles is stuck and ends its branch.
    pub simplify_threshold: f32,
    /// Most nodes merged into one group.
    pub group_size: usize,
    /// Most levels built above level 0.
    pub max_levels: usize,
}

impl Default for ClodConfig {
    fn default() -> Self {
        Self {
            simplify_ratio: DEFAULT_SIMPLIFY_RATIO,
            simplify_threshold: DEFAULT_SIMPLIFY_THRESHOLD,
            group_size: DEFAULT_GROUP_SIZE,
            max_levels: MAX_LEVELS,
        }
    }
}

impl ClodConfig {
    /// The same config with out-of-range fields pulled into their ranges, so a
    /// caller's bad ratio can never make the build loop or divide by zero.
    #[must_use]
    pub fn sanitized(self) -> Self {
        let ratio = if self.simplify_ratio.is_finite() {
            self.simplify_ratio
        } else {
            DEFAULT_SIMPLIFY_RATIO
        };
        let threshold = if self.simplify_threshold.is_finite() {
            self.simplify_threshold
        } else {
            DEFAULT_SIMPLIFY_THRESHOLD
        };
        Self {
            simplify_ratio: ratio.clamp(0.01, 1.0),
            simplify_threshold: threshold.clamp(0.01, 1.0),
            group_size: self.group_size.clamp(2, 64),
            max_levels: self.max_levels.clamp(1, MAX_LEVELS),
        }
    }
}

/// Sphere bounds plus the running simplification error, in mesh space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClodBounds {
    /// Sphere center in meters.
    pub center: [f32; 3],
    /// Sphere radius in meters.
    pub radius_m: f32,
    /// Worst geometry error anywhere below this node, in meters.
    pub error_m: f32,
}

impl ClodBounds {
    /// Projected error as a fraction of the screen height (0..1), the donor's
    /// `clodBounds` rule: rotationally invariant, ignoring perspective
    /// distortion. `projection` is `projection[1][1]` (`cot(fovy / 2)`) and
    /// `znear` is the positive near-plane distance.
    #[must_use]
    pub fn projected_error(&self, camera_position: [f32; 3], projection: f32, znear_m: f32) -> f32 {
        let d = [
            self.center[0] - camera_position[0],
            self.center[1] - camera_position[1],
            self.center[2] - camera_position[2],
        ];
        let distance = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
        let depth = (distance - self.radius_m).max(znear_m).max(1e-4);
        self.error_m / depth * (projection * 0.5)
    }

    /// [`ClodBounds::projected_error`] in pixels of a frame `screen_h` tall.
    #[must_use]
    pub fn projected_error_px(
        &self,
        camera_position: [f32; 3],
        projection: f32,
        znear_m: f32,
        screen_h: u32,
    ) -> f32 {
        self.projected_error(camera_position, projection, znear_m)
            * count_f32(usize::try_from(screen_h).unwrap_or(usize::MAX))
    }

    /// True when the sphere reaches the camera; such a node is never culled.
    #[must_use]
    pub fn contains_point(&self, point: [f32; 3]) -> bool {
        let d = [
            point[0] - self.center[0],
            point[1] - self.center[1],
            point[2] - self.center[2],
        ];
        (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt() <= self.radius_m + 1e-4
    }
}

/// One node of the hierarchy: a cluster of renderable triangles plus what it
/// replaces.
#[derive(Clone, Debug, PartialEq)]
pub struct ClodNode {
    /// 0 for an original cluster, one more for every merge.
    pub level: u16,
    /// The group that replaced this node, or `None` for a root.
    pub parent: Option<u32>,
    /// Nodes this one merged, empty for a level 0 cluster.
    pub children: Vec<u32>,
    /// Sphere bounds with the running error.
    pub bounds: ClodBounds,
    /// Original triangle ids this node covers; children partition them.
    pub triangles: Vec<u32>,
    /// Renderable positions in meters, local to this node.
    pub positions: Vec<[f32; 3]>,
    /// Renderable triangle indices into [`ClodNode::positions`].
    pub indices: Vec<u32>,
}

impl ClodNode {
    /// Renderable triangle count of this node.
    #[must_use]
    pub fn triangle_count(&self) -> u32 {
        count_u32(self.indices.len() / 3)
    }
}

/// One cluster-LOD forest: roots are the branches that stopped merging.
#[derive(Clone, Debug, PartialEq)]
pub struct ClodTree {
    /// Nodes in build order: level 0 first, then each level's groups.
    pub nodes: Vec<ClodNode>,
    /// Root node ids, ascending.
    pub roots: Vec<u32>,
    /// Triangles in the source mesh; a cut always covers exactly this many.
    pub triangle_count: u32,
}

impl ClodTree {
    /// Node ids whose triangles draw at `error_m`: a node stays while its own
    /// error is at or under the threshold and its parent's is over it. Errors
    /// never decrease up the tree, so the result covers every source triangle
    /// exactly once at any threshold.
    #[must_use]
    pub fn cut(&self, error_m: f32) -> Vec<u32> {
        let mut cut = Vec::new();
        for (id, node) in self.nodes.iter().enumerate() {
            let under = node.bounds.error_m <= error_m;
            let parent_over = match node.parent {
                Some(parent) => {
                    let at = usize::try_from(parent).unwrap_or(usize::MAX);
                    self.nodes
                        .get(at)
                        .is_none_or(|p| p.bounds.error_m > error_m)
                }
                None => true,
            };
            if under && parent_over {
                cut.push(count_u32(id));
            }
        }
        cut
    }

    /// Source triangles covered by [`ClodTree::cut`], in cut order.
    #[must_use]
    pub fn cut_triangles(&self, error_m: f32) -> Vec<u32> {
        let mut triangles = Vec::new();
        for id in self.cut(error_m) {
            if let Some(node) = self.nodes.get(usize::try_from(id).unwrap_or(usize::MAX)) {
                triangles.extend_from_slice(&node.triangles);
            }
        }
        triangles
    }

    /// Triangles a cut draws at `error_m`, the number the LOD select reports.
    #[must_use]
    pub fn cut_drawn_triangles(&self, error_m: f32) -> u32 {
        let mut total = 0u32;
        for id in self.cut(error_m) {
            if let Some(node) = self.nodes.get(usize::try_from(id).unwrap_or(usize::MAX)) {
                total = total.saturating_add(node.triangle_count());
            }
        }
        total
    }

    /// Deepest level built, or 0 for an empty tree.
    #[must_use]
    pub fn level_count(&self) -> usize {
        self.nodes
            .iter()
            .map(|node| usize::from(node.level) + 1)
            .max()
            .unwrap_or(0)
    }

    /// The hierarchy as canonical little-endian bytes: the cook's receipt is
    /// byte-equal for equal input, and a later bundle ships this payload.
    #[must_use]
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(b"E2CLOD01");
        out.extend_from_slice(&self.triangle_count.to_le_bytes());
        out.extend_from_slice(&count_u32(self.roots.len()).to_le_bytes());
        for root in &self.roots {
            out.extend_from_slice(&root.to_le_bytes());
        }
        out.extend_from_slice(&count_u32(self.nodes.len()).to_le_bytes());
        for node in &self.nodes {
            out.extend_from_slice(&node.level.to_le_bytes());
            match node.parent {
                Some(parent) => {
                    out.push(1);
                    out.extend_from_slice(&parent.to_le_bytes());
                }
                None => out.push(0),
            }
            out.extend_from_slice(&count_u32(node.children.len()).to_le_bytes());
            for child in &node.children {
                out.extend_from_slice(&child.to_le_bytes());
            }
            for value in node.bounds.center {
                out.extend_from_slice(&value.to_bits().to_le_bytes());
            }
            out.extend_from_slice(&node.bounds.radius_m.to_bits().to_le_bytes());
            out.extend_from_slice(&node.bounds.error_m.to_bits().to_le_bytes());
            out.extend_from_slice(&count_u32(node.triangles.len()).to_le_bytes());
            for triangle in &node.triangles {
                out.extend_from_slice(&triangle.to_le_bytes());
            }
            out.extend_from_slice(&count_u32(node.positions.len()).to_le_bytes());
            for position in &node.positions {
                for value in position {
                    out.extend_from_slice(&value.to_bits().to_le_bytes());
                }
            }
            out.extend_from_slice(&count_u32(node.indices.len()).to_le_bytes());
            for index in &node.indices {
                out.extend_from_slice(&index.to_le_bytes());
            }
        }
        out
    }
}

/// Build the hierarchy over one cooked mesh.
///
/// Level 0 is [`build_meshlets`] over the source triangles. Each higher level
/// sorts the live nodes by bounds center, chunks them by `group_size`, merges
/// each chunk's triangles into one submesh, simplifies it, and keeps it only
/// when its triangle count fell to `simplify_threshold` of the merged count.
/// A stuck chunk keeps its children as roots and leaves the frontier.
/// Errors: [`MeshCookError`] from a refused mesh or simplification.
pub fn build_clod_tree(mesh: &CookedMesh, config: ClodConfig) -> Result<ClodTree, MeshCookError> {
    if mesh.positions.is_empty() || mesh.indices.len() < 3 {
        return Err(MeshCookError::Empty);
    }
    if mesh.indices.len() % 3 != 0 {
        return Err(MeshCookError::Degenerate {
            detail: format!(
                "index count {} is not a multiple of three",
                mesh.indices.len()
            ),
        });
    }
    let config = config.sanitized();
    let meshlets = build_meshlets(&mesh.positions, &mesh.indices).map_err(map_meshlet)?;
    let mut lookup = TriangleLookup::new(&mesh.indices)?;
    let mut nodes = Vec::new();
    let mut frontier = Vec::new();
    for meshlet in &meshlets {
        let triangles = lookup.take(meshlet)?;
        let node = leaf_node(mesh, meshlet, triangles);
        frontier.push(count_u32(nodes.len()));
        nodes.push(node);
        if nodes.len() > MAX_NODES {
            return Err(MeshCookError::TooLarge { count: nodes.len() });
        }
    }
    let mut level: u16 = 0;
    while frontier.len() > 1 && usize::from(level) < config.max_levels {
        level = level.saturating_add(1);
        let mut next = merge_level(mesh, &mut nodes, &frontier, &config, level)?;
        if next.len() == frontier.len() {
            // No group on this level got under the threshold; the forest is final.
            break;
        }
        next.sort_unstable();
        frontier = next;
    }
    let roots = (0..nodes.len())
        .filter(|id| nodes[*id].parent.is_none())
        .map(|id| count_u32(id))
        .collect();
    let triangle_count = count_u32(mesh.indices.len() / 3);
    Ok(ClodTree {
        nodes,
        roots,
        triangle_count,
    })
}

/// One level of merges: append the new group nodes and return the ids that may
/// merge again. A chunk that is stuck contributes nothing, so its children stay
/// roots of their own branch.
fn merge_level(
    mesh: &CookedMesh,
    nodes: &mut Vec<ClodNode>,
    frontier: &[u32],
    config: &ClodConfig,
    level: u16,
) -> Result<Vec<u32>, MeshCookError> {
    let order = spatial_order(nodes, frontier);
    let mut next = Vec::new();
    for chunk in order.chunks(config.group_size) {
        if chunk.len() < 2 {
            // A lone leftover has nothing to merge with; it is a root already.
            continue;
        }
        let triangles = chunk_triangles(nodes, chunk);
        let merged = submesh(mesh, &triangles)?;
        let merged_tris = tri_count(&merged);
        let target = ratio_target(merged_tris, config.simplify_ratio);
        let simplified = simplify(&merged, target)?;
        if f64::from(tri_count(&simplified))
            > f64::from(merged_tris) * f64::from(config.simplify_threshold)
        {
            continue;
        }
        let current_error = max_deviation_m(&merged, &simplified);
        let previous_error = chunk
            .iter()
            .filter_map(|id| nodes.get(usize::try_from(*id).unwrap_or(usize::MAX)))
            .fold(0.0f32, |worst, node| worst.max(node.bounds.error_m));
        let bounds = ClodBounds {
            error_m: previous_error.max(current_error),
            ..merge_bounds(nodes, chunk)
        };
        let id = count_u32(nodes.len());
        for child in chunk {
            if let Some(node) = nodes.get_mut(usize::try_from(*child).unwrap_or(usize::MAX)) {
                node.parent = Some(id);
            }
        }
        nodes.push(ClodNode {
            level,
            parent: None,
            children: chunk.to_vec(),
            bounds,
            triangles,
            positions: simplified.positions,
            indices: simplified.indices,
        });
        if nodes.len() > MAX_NODES {
            return Err(MeshCookError::TooLarge { count: nodes.len() });
        }
        next.push(id);
    }
    Ok(next)
}

/// Frontier node ids in spatial order: bounds center by `total_cmp`, then id,
/// so neighbouring clusters merge and reruns agree.
fn spatial_order(nodes: &[ClodNode], frontier: &[u32]) -> Vec<u32> {
    let mut order = frontier.to_vec();
    order.sort_by(|a, b| {
        let (Some(na), Some(nb)) = (
            nodes.get(usize::try_from(*a).unwrap_or(usize::MAX)),
            nodes.get(usize::try_from(*b).unwrap_or(usize::MAX)),
        ) else {
            return a.cmp(b);
        };
        for lane in 0..3 {
            let c = na.bounds.center[lane].total_cmp(&nb.bounds.center[lane]);
            if c != std::cmp::Ordering::Equal {
                return c;
            }
        }
        a.cmp(b)
    });
    order
}

/// Every original triangle id a chunk covers, ascending.
fn chunk_triangles(nodes: &[ClodNode], chunk: &[u32]) -> Vec<u32> {
    let mut triangles = Vec::new();
    for id in chunk {
        if let Some(node) = nodes.get(usize::try_from(*id).unwrap_or(usize::MAX)) {
            triangles.extend_from_slice(&node.triangles);
        }
    }
    triangles.sort_unstable();
    triangles
}

/// Conservative sphere of a chunk: the mean of the child centers, grown so
/// every child sphere is inside. Error is left at 0 for the caller to set.
fn merge_bounds(nodes: &[ClodNode], chunk: &[u32]) -> ClodBounds {
    let mut center = [0.0f32; 3];
    let mut count = 0u32;
    for id in chunk {
        if let Some(node) = nodes.get(usize::try_from(*id).unwrap_or(usize::MAX)) {
            for (lane, slot) in center.iter_mut().enumerate() {
                *slot += node.bounds.center[lane];
            }
            count = count.saturating_add(1);
        }
    }
    let divisor = count_f32(usize::try_from(count).unwrap_or(usize::MAX)).max(1.0);
    for slot in &mut center {
        *slot /= divisor;
    }
    let mut radius = 0.0f32;
    for id in chunk {
        if let Some(node) = nodes.get(usize::try_from(*id).unwrap_or(usize::MAX)) {
            let d = [
                node.bounds.center[0] - center[0],
                node.bounds.center[1] - center[1],
                node.bounds.center[2] - center[2],
            ];
            let reach = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt() + node.bounds.radius_m;
            radius = radius.max(reach);
        }
    }
    ClodBounds {
        center,
        radius_m: radius,
        error_m: 0.0,
    }
}

/// Level 0 node: the meshlet's own local geometry, error 0.
fn leaf_node(mesh: &CookedMesh, meshlet: &Meshlet, triangles: Vec<u32>) -> ClodNode {
    let mut positions = Vec::with_capacity(meshlet.vertices.len());
    for global in &meshlet.vertices {
        let at = usize::try_from(*global).unwrap_or(usize::MAX);
        positions.push(mesh.positions.get(at).copied().unwrap_or([0.0; 3]));
    }
    let mut indices = Vec::with_capacity(meshlet.triangles.len() * 3);
    for triangle in &meshlet.triangles {
        indices.extend_from_slice(triangle);
    }
    let bounds = meshlet_bounds(&mesh.positions, meshlet);
    ClodNode {
        level: 0,
        parent: None,
        children: Vec::new(),
        bounds: ClodBounds {
            center: bounds.center,
            radius_m: bounds.radius_m,
            error_m: 0.0,
        },
        triangles,
        positions,
        indices,
    }
}

/// The subset of `mesh` made of `triangles`, compacted to the vertices those
/// triangles use, so a group's measured deviation never sees a far vertex.
fn submesh(mesh: &CookedMesh, triangles: &[u32]) -> Result<CookedMesh, MeshCookError> {
    let mut corners = Vec::with_capacity(triangles.len() * 3);
    for triangle in triangles {
        let at = usize::try_from(*triangle)
            .unwrap_or(usize::MAX)
            .checked_mul(3)
            .ok_or(MeshCookError::TooLarge {
                count: triangles.len(),
            })?;
        let tri = mesh
            .indices
            .get(at..at.saturating_add(3))
            .ok_or(MeshCookError::IndexOutOfRange(count_u32(at / 3)))?;
        corners.extend_from_slice(tri);
    }
    let mut ids = corners.clone();
    ids.sort_unstable();
    ids.dedup();
    let mut positions = Vec::with_capacity(ids.len());
    let mut normals = Vec::with_capacity(ids.len());
    let mut uv0 = Vec::with_capacity(ids.len());
    let mut tangents = Vec::with_capacity(ids.len());
    for id in &ids {
        let at = usize::try_from(*id)
            .ok()
            .filter(|at| *at < mesh.positions.len())
            .ok_or(MeshCookError::IndexOutOfRange(*id))?;
        positions.push(mesh.positions[at]);
        normals.push(mesh.normals.get(at).copied().unwrap_or([0.0, 0.0, 1.0]));
        uv0.push(mesh.uv0.get(at).copied().unwrap_or([0.0, 0.0]));
        tangents.push(
            mesh.tangents
                .get(at)
                .copied()
                .unwrap_or([1.0, 0.0, 0.0, 1.0]),
        );
    }
    let mut indices = Vec::with_capacity(corners.len());
    for corner in &corners {
        let local = ids
            .binary_search(corner)
            .map_err(|_| MeshCookError::IndexOutOfRange(*corner))?;
        indices.push(count_u32(local));
    }
    Ok(CookedMesh {
        positions,
        normals,
        uv0,
        tangents,
        indices,
    })
}

/// Original triangle ids keyed by their sorted corner ids, so a meshlet's local
/// triangles can be named as source triangles without a hash map.
struct TriangleLookup {
    rows: Vec<([u32; 3], u32)>,
    used: Vec<bool>,
}

impl TriangleLookup {
    fn new(indices: &[u32]) -> Result<Self, MeshCookError> {
        let mut rows = Vec::with_capacity(indices.len() / 3);
        for (id, tri) in indices.chunks_exact(3).enumerate() {
            let mut key = [tri[0], tri[1], tri[2]];
            key.sort_unstable();
            rows.push((key, count_u32(id)));
        }
        rows.sort_unstable();
        Ok(Self {
            used: vec![false; rows.len()],
            rows,
        })
    }

    /// Source triangle ids one meshlet covers, in meshlet order. Duplicate
    /// triangle keys hand out the lowest free id, so the mapping stays total.
    fn take(&mut self, meshlet: &Meshlet) -> Result<Vec<u32>, MeshCookError> {
        let mut taken = Vec::with_capacity(meshlet.triangles.len());
        for triangle in &meshlet.triangles {
            let mut key = [0u32; 3];
            for (lane, corner) in triangle.iter().enumerate() {
                key[lane] = meshlet
                    .vertices
                    .get(usize::try_from(*corner).unwrap_or(usize::MAX))
                    .copied()
                    .ok_or(MeshCookError::IndexOutOfRange(*corner))?;
            }
            key.sort_unstable();
            let start = self.rows.partition_point(|row| row.0 < key);
            let mut id = None;
            for (at, row) in self.rows.iter().enumerate().skip(start) {
                if row.0 != key {
                    break;
                }
                if !self.used[at] {
                    self.used[at] = true;
                    id = Some(row.1);
                    break;
                }
            }
            let id = id.ok_or(MeshCookError::Degenerate {
                detail: format!("meshlet triangle {key:?} names no free source triangle"),
            })?;
            taken.push(id);
        }
        Ok(taken)
    }
}

/// `MeshletError` as the cook's error, so one error type reaches the caller.
fn map_meshlet(error: MeshletError) -> MeshCookError {
    match error {
        MeshletError::EmptyMesh => MeshCookError::Empty,
        MeshletError::UnalignedIndices { len } => MeshCookError::Degenerate {
            detail: format!("meshlet index count {len} is not a multiple of three"),
        },
        MeshletError::IndexOutOfRange(index) => MeshCookError::IndexOutOfRange(index),
        MeshletError::NonFinite(field) => MeshCookError::NonFinite(field),
        MeshletError::TooLarge { count, .. } => MeshCookError::TooLarge { count },
        MeshletError::ErrorOverBound { level, error } => MeshCookError::Degenerate {
            detail: format!("meshlet level {level} error {error} over bound"),
        },
        MeshletError::NonKtx2 { image } => MeshCookError::Degenerate {
            detail: format!("meshlet input image {image} is not KTX2"),
        },
    }
}

/// Triangle target for one group, at least one; a rounded fraction of the
/// merged count.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "a triangle count is a positive whole number far below u32::MAX"
)]
fn ratio_target(merged_tris: u32, ratio: f32) -> u32 {
    let scaled = f64::from(merged_tris) * f64::from(ratio);
    scaled.round().clamp(1.0, 4.0e9) as u32
}

#[cfg(test)]
#[path = "cluster_lod_tests.rs"]
mod tests;
