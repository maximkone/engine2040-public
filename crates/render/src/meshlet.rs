//! meshlet: virtual-geometry cluster-cut sketch, ideas only.
//! Hierarchical triangle groups swapped per view beside the oracle;
//! draw path stays out. Owner render lane. Units: counts, shares.
//! Determinism: stable order, integer sums, no clock. Errors typed.
//! Caller is the later submit pass, deferred here. Bounce note only:
//! endless bounces, no bake; cache holds surfaces, hit shades hits;
//! tighter at sixty, looser at thirty. No code here; baked in `gi`,
//! gated tables in `gi_dyn`.
#![forbid(unsafe_code)]
use std::fmt;
/// Default gate: the sketch never runs unless the caller opts in.
pub const MESHLET_IDEAS_ENABLED_DEFAULT: bool = false;
/// Cut bar: thirty percent of source triangles must cull on the cube.
pub const MESHLET_CUT_BAR: f32 = 0.30;
/// Sketch failures: disabled flag vs bad caller buffer.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MeshletIdeasError {
    /// Caller did not opt in; pass `enabled = true` for the sketch.
    Disabled,
    /// Caller buffer shorter than the cluster count.
    OutTooSmall { need: usize, have: usize },
    /// The view cannot be projected: bad lens, near plane or pixel bar.
    BadView,
}
impl fmt::Display for MeshletIdeasError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Disabled => write!(f, "meshlet sketch is disabled"),
            Self::OutTooSmall { need, have } => {
                write!(f, "meshlet out fits {have} but {need} must fit")
            }
            Self::BadView => write!(f, "cluster LOD view cannot be projected"),
        }
    }
}
impl std::error::Error for MeshletIdeasError {}
/// One triangle-group cluster with fallback and thin-out rules.
/// Crack-free rule: neighbors swap together so no seam opens. Fallback
/// rule: coarse mesh ships when full detail is unsuitable. Thin-out
/// caution: leaves and grass fade by area, never to zero.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Cluster {
    /// Triangles in this group.
    pub tris: u32,
    /// True when the whole group faces away and may skip.
    pub back: bool,
}
/// Decide one view: mark back groups culled, sum culled and drawn.
/// Requires `enabled`, else refuses loudly. Stable order, no heap use.
pub fn cluster_cut(
    clusters: &[Cluster],
    enabled: bool,
    out: &mut [bool],
) -> Result<(u32, u32), MeshletIdeasError> {
    if !enabled {
        return Err(MeshletIdeasError::Disabled);
    }
    if out.len() < clusters.len() {
        return Err(MeshletIdeasError::OutTooSmall {
            need: clusters.len(),
            have: out.len(),
        });
    }
    let mut culled: u32 = 0;
    let mut total: u32 = 0;
    for (index, cluster) in clusters.iter().enumerate() {
        total = total.saturating_add(cluster.tris);
        if cluster.back {
            culled = culled.saturating_add(cluster.tris);
            out[index] = true;
        } else {
            out[index] = false;
        }
    }
    let drawn = total.saturating_sub(culled);
    Ok((culled, drawn))
}

// ---------------------------------------------------------------------------
// Cluster LOD select: one view walks the assets hierarchy
// (`engine_assets::cluster_lod`) and returns the clusters to draw. A group
// draws when its projected error is under the view's bar; otherwise the walk
// descends into its children, and a leaf draws whatever its error is, because
// level 0 is the finest geometry the cook has. The projected error is
// meshoptimizer's `clodBounds` rule: `error / max(dist(center) - radius, znear)
// * projection * 0.5`, times the frame height for pixels.
// ---------------------------------------------------------------------------

/// One camera view the cluster LOD select answers for.
#[cfg(feature = "v1")]
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct ClodView {
    /// Camera position in meters, mesh space.
    pub camera_position: [f32; 3],
    /// `projection[1][1]`, i.e. `cot(fov_y / 2)`.
    pub projection: f32,
    /// Positive near-plane distance in meters.
    pub znear_m: f32,
    /// Frame height in pixels.
    pub screen_h: u32,
    /// A group draws at or under this projected error, in pixels.
    pub max_error_px: f32,
}

#[cfg(feature = "v1")]
impl ClodView {
    /// A camera `distance_m` from `center` on the `-Z` side, looking at it.
    #[must_use]
    pub fn at_distance(
        center: [f32; 3],
        distance_m: f32,
        fov_y_rad: f32,
        screen_h: u32,
        max_error_px: f32,
    ) -> Self {
        Self {
            camera_position: [center[0], center[1], center[2] - distance_m],
            projection: 1.0 / (fov_y_rad * 0.5).tan(),
            znear_m: 0.1,
            screen_h,
            max_error_px,
        }
    }

    /// True when the view can be projected: a finite, positive lens, near plane
    /// and bar. A broken view is a caller mistake, never a silent empty cut.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        let lens = self.projection.is_finite() && self.projection > 0.0;
        let near = self.znear_m.is_finite() && self.znear_m > 0.0;
        let bar = self.max_error_px.is_finite() && self.max_error_px >= 0.0;
        lens && near && bar && self.camera_position.iter().all(|v| v.is_finite())
    }
}

/// What one view draws from the hierarchy.
#[cfg(feature = "v1")]
#[derive(Clone, Debug, PartialEq)]
pub struct ClodDraw {
    /// Node ids to draw, ascending, one entry per cluster.
    pub nodes: Vec<u32>,
    /// Triangles those nodes draw.
    pub drawn_tris: u32,
    /// Triangles level 0 would draw: every source triangle once.
    pub lod0_tris: u32,
    /// Worst projected error the cut accepted, in pixels.
    pub worst_error_px: f32,
}

#[cfg(feature = "v1")]
impl ClodDraw {
    /// Drawn triangles as a share of the level 0 count, `0..=1`.
    #[must_use]
    #[expect(
        clippy::cast_possible_truncation,
        reason = "the share of two triangle counts is 0..=1, exact in f32"
    )]
    pub fn share_of_lod0(&self) -> f32 {
        if self.lod0_tris == 0 {
            return 0.0;
        }
        (f64::from(self.drawn_tris) / f64::from(self.lod0_tris)) as f32
    }
}

/// Pick the clusters one view draws from `tree`.
///
/// Walks each root down: a node draws while its projected error is at or under
/// `view.max_error_px`, else its children are walked instead; a leaf always
/// draws. Because every node's triangles are exactly its children's, the
/// result covers every source triangle exactly once at any bar.
/// Errors: [`MeshletIdeasError::Disabled`] without the opt-in,
/// [`MeshletIdeasError::BadView`] for a view that cannot be projected.
#[cfg(feature = "v1")]
pub fn clod_select(
    tree: &engine_assets::cluster_lod::ClodTree,
    view: ClodView,
    enabled: bool,
) -> Result<ClodDraw, MeshletIdeasError> {
    if !enabled {
        return Err(MeshletIdeasError::Disabled);
    }
    if !view.is_valid() {
        return Err(MeshletIdeasError::BadView);
    }
    let mut nodes = Vec::new();
    let mut drawn_tris = 0u32;
    let mut worst_error_px = 0.0f32;
    let mut stack = tree.roots.clone();
    while let Some(id) = stack.pop() {
        let Some(node) = tree.nodes.get(usize::try_from(id).unwrap_or(usize::MAX)) else {
            continue;
        };
        let px = node.bounds.projected_error_px(
            view.camera_position,
            view.projection,
            view.znear_m,
            view.screen_h,
        );
        if px <= view.max_error_px || node.children.is_empty() {
            nodes.push(id);
            drawn_tris = drawn_tris.saturating_add(node.triangle_count());
            worst_error_px = worst_error_px.max(px);
        } else {
            stack.extend_from_slice(&node.children);
        }
    }
    nodes.sort_unstable();
    Ok(ClodDraw {
        nodes,
        drawn_tris,
        lod0_tris: tree.triangle_count,
        worst_error_px,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_refuses_loudly() {
        let clusters = [Cluster {
            tris: 4,
            back: true,
        }];
        let mut out = [false; 1];
        assert_eq!(
            cluster_cut(&clusters, false, &mut out),
            Err(MeshletIdeasError::Disabled)
        );
    }

    #[test]
    fn out_too_small_names_need_have() {
        let clusters = [
            Cluster {
                tris: 4,
                back: true,
            },
            Cluster {
                tris: 6,
                back: false,
            },
        ];
        let mut out = [false; 1];
        assert_eq!(
            cluster_cut(&clusters, true, &mut out),
            Err(MeshletIdeasError::OutTooSmall { need: 2, have: 1 })
        );
    }

    #[test]
    fn back_groups_cull_and_sums_match() {
        let clusters = [
            Cluster {
                tris: 4,
                back: true,
            },
            Cluster {
                tris: 6,
                back: false,
            },
            Cluster {
                tris: 10,
                back: true,
            },
        ];
        let mut out = [false; 3];
        assert_eq!(cluster_cut(&clusters, true, &mut out), Ok((14, 6)));
        assert_eq!(out, [true, false, true]);
    }

    #[test]
    fn empty_replays_zero_zero() {
        let mut out = [true; 2];
        assert_eq!(cluster_cut(&[], true, &mut out), Ok((0, 0)));
    }

    #[test]
    fn same_input_replays_identical() {
        let clusters = [
            Cluster {
                tris: 7,
                back: true,
            },
            Cluster {
                tris: 3,
                back: false,
            },
        ];
        let (mut a, mut b) = ([false; 2], [false; 2]);
        let (ra, rb) = (
            cluster_cut(&clusters, true, &mut a),
            cluster_cut(&clusters, true, &mut b),
        );
        assert_eq!((ra, a), (rb, b));
    }
}

/// Cluster LOD select tests over the assets hierarchy. The fixture is a closed
/// ring of `2 * n * n` triangles — `n = 100` is the row's 20k receipt body — so
/// no simplified group can shrink a border the way an open sheet can.
#[cfg(all(test, feature = "v1"))]
mod clod_tests {
    use super::{clod_select, ClodView, MeshletIdeasError};
    use engine_assets::cluster_lod::ClodTree;
    use engine_assets::{build_clod_tree, ClodConfig, CookedMesh};

    /// A closed torus body: `2 * n * n` triangles, 1.2 m across the ring and
    /// 0.4 m through it, so its triangles are about a crowd body's.
    fn body_ring(n: usize) -> CookedMesh {
        let major = 0.6f32;
        let minor = 0.2f32;
        let mut positions = Vec::with_capacity(n * n);
        let mut normals = Vec::with_capacity(n * n);
        let mut uv0 = Vec::with_capacity(n * n);
        let mut tangents = Vec::with_capacity(n * n);
        for i in 0..n {
            let u = std::f32::consts::TAU * i as f32 / n as f32;
            for j in 0..n {
                let v = std::f32::consts::TAU * j as f32 / n as f32;
                let normal = [v.cos() * u.cos(), v.sin(), v.cos() * u.sin()];
                let hub = [major * u.cos(), 0.0, major * u.sin()];
                positions.push([
                    hub[0] + minor * normal[0],
                    hub[1] + minor * normal[1],
                    hub[2] + minor * normal[2],
                ]);
                normals.push(normal);
                uv0.push([i as f32 / n as f32, j as f32 / n as f32]);
                tangents.push([1.0, 0.0, 0.0, 1.0]);
            }
        }
        let mut indices = Vec::with_capacity(n * n * 6);
        for i in 0..n {
            let next_row = (i + 1) % n;
            for j in 0..n {
                let next_col = (j + 1) % n;
                let a = (i * n + j) as u32;
                let b = (next_row * n + j) as u32;
                let c = (next_row * n + next_col) as u32;
                let d = (i * n + next_col) as u32;
                indices.extend_from_slice(&[a, b, c, a, c, d]);
            }
        }
        CookedMesh {
            positions,
            normals,
            uv0,
            tangents,
            indices,
        }
    }

    fn tree_of(n: usize) -> ClodTree {
        build_clod_tree(&body_ring(n), ClodConfig::default())
            .expect("the ring body cooks a cluster LOD hierarchy")
    }

    /// The row's receipt: a 20k-triangle body viewed from 30 m draws under half
    /// its level 0 triangles while every drawn group stays under 1.5 px.
    #[test]
    fn clod_select_at_thirty_metres_draws_under_half_the_lod0_triangles() {
        let tree = tree_of(100);
        let center = [0.0, 0.0, 0.0];
        let view = ClodView::at_distance(center, 30.0, 35.0f32.to_radians(), 720, 1.5);
        let draw = clod_select(&tree, view, true).expect("the view selects a cut");
        let share = draw.share_of_lod0();
        let levels = tree.level_count();
        for level in 0..levels {
            let mut count = 0usize;
            let mut worst_m = 0.0f32;
            for node in &tree.nodes {
                if usize::from(node.level) == level {
                    count += 1;
                    worst_m = worst_m.max(node.bounds.error_m);
                }
            }
            println!(
                "level {level}: {count} nodes, worst error {worst_m:.4} m, {:.2} px at 30 m",
                worst_m / 30.0 * view.projection * 0.5 * 720.0
            );
        }
        assert_eq!(tree.triangle_count, 20_000, "fixture is the 20k-tri body");
        assert!(draw.drawn_tris > 0, "30 m still draws something");
        assert!(
            levels >= 4,
            "20k tris must build about 5 levels; got {levels} levels, share {share:.3}"
        );
        assert!(
            draw.worst_error_px < 1.5,
            "accepted error {} px over the 1.5 px bar; {levels} levels, share {share:.3}",
            draw.worst_error_px
        );
        assert!(
            share <= 0.45,
            "cut draws {} of {} LOD0 tris ({:.1}%), over 45%; {levels} levels, {} clusters, worst {} px",
            draw.drawn_tris,
            draw.lod0_tris,
            share * 100.0,
            draw.nodes.len(),
            draw.worst_error_px
        );
        println!(
            "clod receipt: {levels} levels, {} clusters, {} of {} tris ({:.1}%), worst {:.3} px at 30 m",
            draw.nodes.len(),
            draw.drawn_tris,
            draw.lod0_tris,
            share * 100.0,
            draw.worst_error_px
        );
    }

    #[test]
    fn clod_select_covers_every_triangle_exactly_once() {
        let tree = tree_of(24);
        let center = [0.0, 0.0, 0.0];
        let expected: Vec<u32> = (0..tree.triangle_count).collect();
        for bar_px in [0.0f32, 0.5, 4.0, 1_000.0] {
            let view = ClodView::at_distance(center, 12.0, 35.0f32.to_radians(), 720, bar_px);
            let draw = clod_select(&tree, view, true).expect("the view selects a cut");
            let mut covered: Vec<u32> = draw
                .nodes
                .iter()
                .flat_map(|id| {
                    let at = usize::try_from(*id).expect("node id fits");
                    tree.nodes[at].triangles.clone()
                })
                .collect();
            covered.sort_unstable();
            assert_eq!(
                covered, expected,
                "the {bar_px} px cut must cover every source triangle once"
            );
        }
    }

    #[test]
    fn clod_select_replays_and_refuses() {
        let tree = tree_of(16);
        let view = ClodView::at_distance([0.0, 0.0, 0.0], 8.0, 35.0f32.to_radians(), 720, 1.0);
        let first = clod_select(&tree, view, true).expect("the view selects a cut");
        let second = clod_select(&tree, view, true).expect("the view selects a cut");
        assert_eq!(first, second, "the same view must pick the same clusters");
        assert_eq!(
            clod_select(&tree, view, false),
            Err(MeshletIdeasError::Disabled)
        );
        let broken = ClodView {
            projection: 0.0,
            ..view
        };
        assert_eq!(
            clod_select(&tree, broken, true),
            Err(MeshletIdeasError::BadView)
        );
    }
}
