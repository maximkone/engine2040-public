//! weights: automatic skin weights from bone segments (std only, no feature).
//!
//! Owner: `engine-geo`; consumer: a skin cook that has a mesh and a rig's bone segments but no
//! painted weights (GEO6b). Units: meters, in the mesh's own space; a bone index is a `u16`
//! into the caller's `bones` slice. Determinism: a pure function of the mesh and the bones, all
//! math in `f64`, ties broken by the lower bone index, so a cook run twice gives the same
//! bytes. Errors: [`GeoError`] for a bad mesh, no bones, too many bones, or a non-finite bone.
//!
//! Method: the simple exact base before any heat or geodesic method. Each vertex keeps its
//! four nearest bones by closest-point distance and weighs each by `1 / (d + e)^2`, where `e`
//! is 1 percent of the mesh bounding-box diagonal, so a vertex on a bone gets a finite weight.

use crate::mesh::{narrow, segment_distance_sq, GeoError, GeoMesh};

/// Bone slots one vertex carries.
pub const SLOTS: usize = 4;

/// One bone as the segment from its head joint to its tail joint.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BoneSegment {
    /// Joint the bone starts at.
    pub head: [f32; 3],
    /// Joint the bone ends at.
    pub tail: [f32; 3],
}

/// The four nearest bones of one vertex with normalized weights, nearest first.
fn vertex_weights(p: [f64; 3], bones: &[[[f64; 3]; 2]], eps: f64) -> [(u16, f32); SLOTS] {
    let mut near: Vec<(f64, usize)> = bones
        .iter()
        .enumerate()
        .map(|(i, seg)| (segment_distance_sq(p, seg[0], seg[1]), i))
        .collect();
    near.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
    near.truncate(SLOTS);
    let raw: Vec<f64> = near
        .iter()
        .map(|&(d_sq, _)| {
            let d = (d_sq.sqrt() + eps).max(1e-12);
            1.0 / (d * d)
        })
        .collect();
    let total: f64 = raw.iter().sum();
    let mut out = [(0_u16, 0.0_f32); SLOTS];
    for (slot, (&(_, bone), w)) in out.iter_mut().zip(near.iter().zip(&raw)) {
        *slot = (u16::try_from(bone).unwrap_or(u16::MAX), narrow(w / total));
    }
    out
}

/// Skin weights for every vertex of `mesh` from `bones`: the four nearest bones by
/// closest-point distance, weight `1 / (d + 0.01 x bounding-box diagonal)^2`, normalized to
/// sum 1, ties by the lower bone index, unused slots `(0, 0.0)`.
///
/// # Errors
/// [`GeoError::Malformed`] for no bones or more than 65 536; [`GeoError::NonFinite`] for a
/// bone with a NaN or infinite joint; the mesh's own [`GeoMesh::validate`] errors.
pub fn bind_weights(
    mesh: &GeoMesh,
    bones: &[BoneSegment],
) -> Result<Vec<[(u16, f32); SLOTS]>, GeoError> {
    mesh.validate()?;
    if bones.is_empty() {
        return Err(GeoError::Malformed("no bones"));
    }
    if bones.len() > usize::from(u16::MAX) + 1 {
        return Err(GeoError::Malformed("more than 65536 bones"));
    }
    if !bones
        .iter()
        .flat_map(|b| b.head.into_iter().chain(b.tail))
        .all(f32::is_finite)
    {
        return Err(GeoError::NonFinite);
    }
    let segments: Vec<[[f64; 3]; 2]> = bones
        .iter()
        .map(|b| [b.head.map(f64::from), b.tail.map(f64::from)])
        .collect();
    let eps = 0.01 * mesh.diagonal();
    Ok(mesh
        .positions
        .iter()
        .map(|p| vertex_weights(p.map(f64::from), &segments, eps))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::primitives::cylinder;
    use engine_core::Fnv64;

    /// Three bones stacked along Y over the cylinder's height (-1.5..1.5), joints at -0.5 and 0.5.
    fn spine() -> Vec<BoneSegment> {
        [(-1.5, -0.5), (-0.5, 0.5), (0.5, 1.5)]
            .map(|(a, b)| BoneSegment {
                head: [0.0, a, 0.0],
                tail: [0.0, b, 0.0],
            })
            .to_vec()
    }

    /// `cylinder(0.3, 3.0, 24)` has rings only at the two caps. The test adds the rim at the
    /// bone ends and middles, and two axis vertices: one on bone 1's middle, one on the joint
    /// of bones 1 and 2. Returns the mesh and those two vertex indices.
    fn tube() -> (GeoMesh, usize, usize) {
        let mut mesh = cylinder(0.3, 3.0, 24);
        let rim: Vec<[f32; 3]> = mesh.positions[..24].to_vec();
        for y in [-1.0, -0.5, 0.0, 0.5, 1.0] {
            mesh.positions.extend(rim.iter().map(|p| [p[0], y, p[2]]));
        }
        let middle = mesh.positions.len();
        mesh.positions.push([0.0, 0.0, 0.0]);
        mesh.positions.push([0.0, 0.5, 0.0]);
        (mesh, middle, middle + 1)
    }

    fn digest(weights: &[[(u16, f32); SLOTS]]) -> u64 {
        let mut h = Fnv64::new();
        for slot in weights.iter().flatten() {
            h.write_u32(u32::from(slot.0));
            h.write_f32(slot.1);
        }
        h.finish()
    }

    #[test]
    fn every_vertex_sums_to_one_with_at_most_four_influences() {
        let (mesh, _, _) = tube();
        let w = bind_weights(&mesh, &spine()).expect("binds");
        assert_eq!(w.len(), mesh.positions.len());
        for (v, slots) in w.iter().enumerate() {
            let sum: f32 = slots.iter().map(|s| s.1).sum();
            assert!((sum - 1.0).abs() < 1e-6, "vertex {v} sums to {sum}");
            let live = slots.iter().filter(|s| s.1 > 0.0).count();
            assert!(
                (1..=SLOTS).contains(&live),
                "vertex {v} has {live} influences"
            );
            assert!(slots.iter().all(|s| usize::from(s.0) < 3));
        }
    }

    #[test]
    fn a_vertex_on_a_bone_follows_it_and_a_joint_splits_evenly() {
        let (mesh, middle, joint) = tube();
        let w = bind_weights(&mesh, &spine()).expect("binds");
        // On bone 1's middle: bone 1 first with at least 0.9 of the weight.
        assert_eq!(w[middle][0].0, 1);
        assert!(w[middle][0].1 >= 0.9, "middle weight {}", w[middle][0].1);
        // On the joint of bones 1 and 2: both within 0.4 and 0.6 (bone 1 wins the tie).
        assert_eq!([w[joint][0].0, w[joint][1].0], [1, 2]);
        for slot in &w[joint][..2] {
            assert!((0.4..=0.6).contains(&slot.1), "joint weight {}", slot.1);
        }
        // A rim vertex at the same height leans on bone 1 too, but the 1/d^2 falloff over
        // the 0.3 m radius keeps it well under the on-axis value.
        let rim = mesh
            .positions
            .iter()
            .position(|p| p[1] == 0.0 && p[0] > 0.29)
            .expect("a rim vertex at height 0");
        assert_eq!(w[rim][0].0, 1);
        assert!(w[rim][0].1 > 0.5 && w[rim][0].1 < w[middle][0].1);
    }

    #[test]
    fn four_nearest_bones_only_and_unused_slots_are_zero() {
        let mesh = GeoMesh::from_positions(vec![[0.0, 0.0, 0.0]], vec![0, 0, 0]);
        // Bone i sits i + 1 meters away on X: nearest are 0, 1, 2, 3; bone 4 is dropped.
        let bones: Vec<BoneSegment> = (1..=5_u8)
            .map(|i| BoneSegment {
                head: [f32::from(i), 0.0, 0.0],
                tail: [f32::from(i), 1.0, 0.0],
            })
            .collect();
        let w = bind_weights(&mesh, &bones).expect("binds");
        assert_eq!(w[0].map(|s| s.0), [0, 1, 2, 3]);
        assert!(w[0][0].1 > w[0][1].1 && w[0][1].1 > w[0][2].1 && w[0][2].1 > w[0][3].1);
        // One bone: slot 0 takes everything, the rest are the unused `(0, 0.0)`.
        let one = bind_weights(&mesh, &bones[..1]).expect("binds");
        assert_eq!(one[0], [(0, 1.0), (0, 0.0), (0, 0.0), (0, 0.0)]);
    }

    #[test]
    fn equal_distance_ties_go_to_the_lower_bone() {
        let mesh = GeoMesh::from_positions(vec![[0.0; 3]], vec![0, 0, 0]);
        let bone = |x: f32| BoneSegment {
            head: [x, 0.0, 0.0],
            tail: [x, 1.0, 0.0],
        };
        let w = bind_weights(&mesh, &[bone(1.0), bone(-1.0)]).expect("binds");
        assert_eq!([w[0][0].0, w[0][1].0], [0, 1]);
        assert_eq!(w[0][0].1, w[0][1].1);
    }

    #[test]
    fn the_result_digest_is_equal_twice() {
        let (mesh, _, _) = tube();
        let a = bind_weights(&mesh, &spine()).expect("binds");
        let b = bind_weights(&mesh, &spine()).expect("binds");
        assert_eq!(digest(&a), digest(&b));
        // A moved bone changes it.
        let mut moved = spine();
        moved[1].tail[0] = 0.2;
        let c = bind_weights(&mesh, &moved).expect("binds");
        assert_ne!(digest(&a), digest(&c));
    }

    #[test]
    fn bad_input_is_a_typed_error() {
        let (mesh, _, _) = tube();
        assert_eq!(
            bind_weights(&mesh, &[]),
            Err(GeoError::Malformed("no bones"))
        );
        let mut nan = spine();
        nan[2].head[1] = f32::NAN;
        assert_eq!(bind_weights(&mesh, &nan), Err(GeoError::NonFinite));
        let mut bad = mesh.clone();
        bad.indices.push(0);
        assert!(bind_weights(&bad, &spine()).is_err());
    }
}
