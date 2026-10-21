//! parts: one pose for a part set - the union box, one shared translation, one triangle budget.
//!
//! Cook-time only (decision D4): std only, no feature, no donor library. The caller is the
//! part-set intake (GN6): a generator writes N part meshes, and every part has to land where
//! the generator drew it, so the pose comes from the union box of the whole set and never from
//! one part alone.
//!
//! Units: meters in, meters and triangle counts out. Determinism: a pure function of the meshes
//! and the budget, so the same set gives the same plan every run and the plan can be baked.
//! Errors: [`PartError`]; a refusal allocates nothing.
//! Budget: at most [`MAX_PARTS`] parts, one pass over the positions, no per-vertex work beyond
//! one add.
//!
//! The cook that turns the plan into destruction chunks lives in `engine-assets` (the intake
//! owns the `FRAC_` names, the provenance row and the cache); this module owns only the
//! geometry, so the plan can be proved without the asset crate.

use crate::mesh::GeoMesh;

/// Most parts one set may hold.
pub const MAX_PARTS: usize = 16;

/// Why a part set was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PartError {
    /// The set holds no part.
    Empty,
    /// The set holds more parts than [`MAX_PARTS`].
    TooManyParts {
        /// The refused part count.
        parts: usize,
    },
    /// One part has no triangles, so it has no place in the union box.
    EmptyPart {
        /// The refused part index.
        index: usize,
    },
    /// A position is not finite, so no box can be trusted.
    NonFinite,
    /// The budget cannot give every part one triangle.
    ZeroBudget,
}

impl std::fmt::Display for PartError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => write!(f, "geo: part set is empty"),
            Self::TooManyParts { parts } => {
                write!(
                    f,
                    "geo: part set holds {parts} parts, MAX_PARTS is {MAX_PARTS}"
                )
            }
            Self::EmptyPart { index } => write!(f, "geo: part {index} has no triangles"),
            Self::NonFinite => write!(f, "geo: a part position is not finite"),
            Self::ZeroBudget => write!(f, "geo: the triangle budget gives a part zero triangles"),
        }
    }
}

impl std::error::Error for PartError {}

/// The plan for one part set: where the whole set goes and what each part may keep.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PartPlan {
    /// Part count.
    pub parts: usize,
    /// Triangles each part may keep.
    pub budget_per_part: u32,
    /// Union box over every part, the minimum corner.
    pub union_min: [f32; 3],
    /// Union box over every part, the maximum corner.
    pub union_max: [f32; 3],
    /// The one translation every part gets; it centres the union box on the origin.
    pub translate: [f32; 3],
}

impl PartPlan {
    /// Triangles the whole set may keep.
    #[must_use]
    pub fn total_budget(&self) -> u32 {
        self.budget_per_part
            .saturating_mul(u32::try_from(self.parts).unwrap_or(u32::MAX))
    }
}

/// Union box over every part: the minimum and the maximum corner.
///
/// Errors: [`PartError::Empty`] for an empty set, [`PartError::TooManyParts`] over
/// [`MAX_PARTS`], [`PartError::EmptyPart`] for a part with no positions, and
/// [`PartError::NonFinite`] for a position that is not finite.
pub fn union_bounds(meshes: &[&GeoMesh]) -> Result<([f32; 3], [f32; 3]), PartError> {
    check_set(meshes)?;
    let mut min = [f32::INFINITY; 3];
    let mut max = [f32::NEG_INFINITY; 3];
    for mesh in meshes {
        for position in &mesh.positions {
            for lane in 0..3 {
                let value = position[lane];
                if value < min[lane] {
                    min[lane] = value;
                }
                if value > max[lane] {
                    max[lane] = value;
                }
            }
        }
    }
    Ok((min, max))
}

/// The triangle budget each part gets: `max_tris / parts`, rounded down.
///
/// The sum never exceeds `max_tris`, so a set cannot grow past the cap the caller set.
#[must_use]
pub fn split_budget(max_tris: u32, parts: usize) -> Vec<u32> {
    if parts == 0 {
        return Vec::new();
    }
    let count = u32::try_from(parts).unwrap_or(u32::MAX);
    vec![max_tris / count; parts]
}

/// The plan for one part set: one union box, one translation, one budget per part.
///
/// The translation centres the union box on the origin, so the parts keep their relative
/// offsets and the whole prop turns around its own middle.
///
/// Errors: every [`PartError`], plus [`PartError::ZeroBudget`] when the budget cannot give
/// every part one triangle.
pub fn plan(meshes: &[&GeoMesh], max_tris: u32) -> Result<PartPlan, PartError> {
    let (union_min, union_max) = union_bounds(meshes)?;
    let parts = meshes.len();
    let budget_per_part = max_tris / u32::try_from(parts).unwrap_or(u32::MAX);
    if budget_per_part == 0 {
        return Err(PartError::ZeroBudget);
    }
    let centre = [
        (union_min[0] + union_max[0]) * 0.5,
        (union_min[1] + union_max[1]) * 0.5,
        (union_min[2] + union_max[2]) * 0.5,
    ];
    Ok(PartPlan {
        parts,
        budget_per_part,
        union_min,
        union_max,
        translate: [-centre[0], -centre[1], -centre[2]],
    })
}

/// The same mesh moved by `translate`; normals and UVs ride along unchanged.
#[must_use]
pub fn translated(mesh: &GeoMesh, translate: [f32; 3]) -> GeoMesh {
    let mut out = mesh.clone();
    apply_translation(&mut out, translate);
    out
}

/// Move a mesh by `translate` in place. Normals and UVs do not change.
pub fn apply_translation(mesh: &mut GeoMesh, translate: [f32; 3]) {
    for position in &mut mesh.positions {
        for lane in 0..3 {
            position[lane] += translate[lane];
        }
    }
}

/// Refuse a set that cannot be planned, before any box is built.
fn check_set(meshes: &[&GeoMesh]) -> Result<(), PartError> {
    if meshes.is_empty() {
        return Err(PartError::Empty);
    }
    if meshes.len() > MAX_PARTS {
        return Err(PartError::TooManyParts {
            parts: meshes.len(),
        });
    }
    for (index, mesh) in meshes.iter().enumerate() {
        if mesh.positions.is_empty() || mesh.indices.is_empty() {
            return Err(PartError::EmptyPart { index });
        }
        for position in &mesh.positions {
            if !position[0].is_finite() || !position[1].is_finite() || !position[2].is_finite() {
                return Err(PartError::NonFinite);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    //! What the part-set plan promises: one pose, one budget each, the same plan twice.

    use super::*;
    use crate::primitives::cuboid;

    /// A unit cube moved along x.
    fn cube_at(x: f32) -> GeoMesh {
        let mut mesh = cuboid([1.0, 1.0, 1.0]);
        for position in &mut mesh.positions {
            position[0] += x;
        }
        mesh
    }

    /// Four cubes side by side, the GN6 fixture shape.
    fn four_parts() -> Vec<GeoMesh> {
        vec![cube_at(0.0), cube_at(2.0), cube_at(4.0), cube_at(6.0)]
    }

    #[test]
    fn four_parts_get_one_pose_and_one_budget_each() {
        let parts = four_parts();
        let refs: Vec<&GeoMesh> = parts.iter().collect();
        let plan = plan(&refs, 5_000).expect("four parts plan");
        assert_eq!(plan.parts, 4);
        // 5,000 triangles over four parts is 1,250 each, and the sum stays under the cap.
        assert_eq!(plan.budget_per_part, 1_250);
        assert_eq!(plan.total_budget(), 5_000);
        // The union box covers every part, from the first cube to the last.
        assert_eq!(plan.union_min, [-0.5, -0.5, -0.5]);
        assert_eq!(plan.union_max, [6.5, 0.5, 0.5]);
        // One pose for the whole set: the union box centre lands on the origin.
        assert_eq!(plan.translate, [-3.0, 0.0, 0.0]);
    }

    #[test]
    fn the_pose_centres_the_union_box_and_keeps_the_parts_together() {
        let parts = four_parts();
        let refs: Vec<&GeoMesh> = parts.iter().collect();
        let plan = plan(&refs, 5_000).expect("four parts plan");
        let moved: Vec<GeoMesh> = refs
            .iter()
            .map(|mesh| translated(mesh, plan.translate))
            .collect();
        // The union box now straddles the origin.
        let moved_refs: Vec<&GeoMesh> = moved.iter().collect();
        let (min, max) = union_bounds(&moved_refs).expect("union of moved parts");
        for lane in 0..3 {
            let centre = (min[lane] + max[lane]) * 0.5;
            assert!(centre.abs() < 1e-6, "lane {lane} centre {centre}");
        }
        // The parts still fit together: the gap between two neighbours is unchanged.
        let before = parts[1].positions[0][0] - parts[0].positions[0][0];
        let after = moved[1].positions[0][0] - moved[0].positions[0][0];
        assert!((before - after).abs() < 1e-6);
    }

    #[test]
    fn the_plan_is_the_same_twice_and_the_moved_meshes_match() {
        let parts = four_parts();
        let refs: Vec<&GeoMesh> = parts.iter().collect();
        let first = plan(&refs, 5_000).expect("plan");
        let second = plan(&refs, 5_000).expect("plan again");
        assert_eq!(first, second);
        let a: Vec<GeoMesh> = refs
            .iter()
            .map(|m| translated(m, first.translate))
            .collect();
        let b: Vec<GeoMesh> = refs
            .iter()
            .map(|m| translated(m, second.translate))
            .collect();
        for (left, right) in a.iter().zip(b.iter()) {
            assert_eq!(left.digest(), right.digest());
        }
    }

    #[test]
    fn translation_moves_positions_and_leaves_normals_and_uvs_alone() {
        let mut mesh = cuboid([1.0, 1.0, 1.0]);
        mesh.normals = vec![[0.0, 1.0, 0.0]; mesh.positions.len()];
        mesh.uvs = vec![[0.25, 0.75]; mesh.positions.len()];
        let normals = mesh.normals.clone();
        let uvs = mesh.uvs.clone();
        let before = mesh.positions.clone();
        apply_translation(&mut mesh, [1.0, 2.0, 3.0]);
        assert_eq!(mesh.normals, normals);
        assert_eq!(mesh.uvs, uvs);
        for (position, origin) in mesh.positions.iter().zip(before.iter()) {
            for lane in 0..3 {
                let moved = position[lane] - origin[lane];
                let want = [1.0_f32, 2.0, 3.0][lane];
                assert!((moved - want).abs() < 1e-6, "lane {lane} moved {moved}");
            }
        }
    }

    #[test]
    fn refusals_name_the_problem() {
        // An empty set has no box.
        assert_eq!(union_bounds(&[]), Err(PartError::Empty));
        // Too many parts is refused before any work.
        let many: Vec<GeoMesh> = (0..=MAX_PARTS).map(|_| cube_at(0.0)).collect();
        let many_refs: Vec<&GeoMesh> = many.iter().collect();
        assert_eq!(
            union_bounds(&many_refs),
            Err(PartError::TooManyParts {
                parts: MAX_PARTS + 1
            })
        );
        // A part with no triangles has no place in the box.
        let empty = GeoMesh::default();
        let good = cube_at(0.0);
        assert_eq!(
            union_bounds(&[&good, &empty]),
            Err(PartError::EmptyPart { index: 1 })
        );
        // A non-finite position is refused.
        let mut broken = cube_at(0.0);
        broken.positions[0][1] = f32::NAN;
        assert_eq!(union_bounds(&[&good, &broken]), Err(PartError::NonFinite));
        // A budget too small to give every part a triangle is refused.
        let one = [cube_at(0.0)];
        let one_refs: Vec<&GeoMesh> = one.iter().collect();
        assert_eq!(plan(&one_refs, 0), Err(PartError::ZeroBudget));
        // The error text names the cause.
        assert_eq!(
            PartError::ZeroBudget.to_string(),
            "geo: the triangle budget gives a part zero triangles"
        );
    }

    #[test]
    fn the_budget_split_never_grows_past_the_cap() {
        let split = split_budget(5_000, 3);
        assert_eq!(split, vec![1_666, 1_666, 1_666]);
        let sum: u32 = split.iter().sum();
        assert!(sum <= 5_000);
        assert!(split_budget(10, 0).is_empty());
    }
}
