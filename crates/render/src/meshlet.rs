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
}
impl fmt::Display for MeshletIdeasError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Disabled => write!(f, "meshlet sketch is disabled"),
            Self::OutTooSmall { need, have } => {
                write!(f, "meshlet out fits {have} but {need} must fit")
            }
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
