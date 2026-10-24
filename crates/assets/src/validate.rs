//! Content rules over a cooked mesh. A rule names one defect, its severity and
//! an optional fix. `validate` reports hits in `RULES` order; `fix` applies one
//! repair and re-checks that rule. Std-only, no IO, no randomness.
use super::cooked::CookedMesh;
use super::import_repair::recompute_tangents;

/// Triangles with an area at or below this (square meters) are degenerate.
const DEGENERATE_AREA_M2: f64 = 1.0e-12;
/// A normal may differ from unit length by this much.
const NORMAL_UNIT_TOL: f64 = 1.0e-3;
/// A zero-length normal has no direction to keep, so it becomes +Y.
const NORMAL_FALLBACK: [f32; 3] = [0.0, 1.0, 0.0];

/// How bad one hit is. An error means the cook must refuse the mesh; a warning
/// means the mesh is usable but the source should be fixed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    /// The cook must refuse the mesh.
    Error,
    /// The mesh is usable; the source should be fixed.
    Warning,
}

/// One content rule: a defect check and the repair for it, if one exists.
#[derive(Clone, Copy)]
pub struct Rule {
    /// Dotted id; the key that `fix` takes.
    pub id: &'static str,
    /// Severity of every hit this rule reports.
    pub severity: Severity,
    /// True when the mesh has the defect.
    pub check: fn(&CookedMesh) -> bool,
    /// Repair for the defect; `None` when only the source can change.
    pub fix: Option<fn(&mut CookedMesh)>,
}

/// One defect that `validate` found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hit {
    /// The rule that fired.
    pub id: &'static str,
    /// The rule's severity.
    pub severity: Severity,
}

/// Why `fix` did not clear a rule.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FixError {
    /// No rule has this id.
    Unknown(String),
    /// The rule has no repair; the source has to change.
    NoFix(&'static str),
    /// The repair ran and the rule still fires. The mesh was changed; drop it.
    StillHit(&'static str),
}

/// Every content rule, in the order `validate` reports them.
pub const RULES: &[Rule] = &[
    Rule {
        id: "tangents.missing",
        severity: Severity::Warning,
        check: tangents_missing,
        fix: Some(recompute_missing_tangents),
    },
    Rule {
        id: "tri.degenerate",
        severity: Severity::Warning,
        check: tri_degenerate,
        fix: Some(drop_degenerate_tris),
    },
    Rule {
        id: "normal.not_unit",
        severity: Severity::Warning,
        check: normal_not_unit,
        fix: Some(renormalize_normals),
    },
    Rule {
        id: "float.nonfinite",
        severity: Severity::Error,
        check: float_nonfinite,
        fix: None,
    },
];

/// Every rule that fires on `mesh`, in `RULES` order. Same mesh, same hits.
pub fn validate(mesh: &CookedMesh) -> Vec<Hit> {
    RULES
        .iter()
        .filter(|rule| (rule.check)(mesh))
        .map(|rule| Hit {
            id: rule.id,
            severity: rule.severity,
        })
        .collect()
}

/// Apply rule `id`'s repair, then re-check that rule. `Ok(())` means the rule
/// no longer fires. Other rules are not re-checked by this call.
pub fn fix(mesh: &mut CookedMesh, id: &str) -> Result<(), FixError> {
    let rule = RULES
        .iter()
        .find(|rule| rule.id == id)
        .ok_or_else(|| FixError::Unknown(id.to_owned()))?;
    let Some(repair) = rule.fix else {
        return Err(FixError::NoFix(rule.id));
    };
    repair(mesh);
    if (rule.check)(mesh) {
        return Err(FixError::StillHit(rule.id));
    }
    Ok(())
}

fn all_finite<const N: usize>(values: &[f32; N]) -> bool {
    values.iter().all(|v| v.is_finite())
}

/// The position at `index`, or `None` when the index is out of range.
fn position(mesh: &CookedMesh, index: u32) -> Option<&[f32; 3]> {
    usize::try_from(index)
        .ok()
        .and_then(|i| mesh.positions.get(i))
}

fn length(v: &[f32; 3]) -> f64 {
    let (x, y, z) = (f64::from(v[0]), f64::from(v[1]), f64::from(v[2]));
    (x * x + y * y + z * z).sqrt()
}

/// The tangent array does not hold one entry per vertex.
fn tangents_missing(mesh: &CookedMesh) -> bool {
    mesh.tangents.len() != mesh.positions.len()
}

/// Rebuild the tangents from positions and UVs with the import lane's solve.
fn recompute_missing_tangents(mesh: &mut CookedMesh) {
    mesh.tangents = recompute_tangents(mesh);
}

/// Zero-area test for one triangle. Out-of-range indices and non-finite
/// corners answer false: range is `validate_mesh`'s job and NaN is
/// `float.nonfinite`'s, so one defect never reports twice.
fn triangle_is_degenerate(mesh: &CookedMesh, corners: [u32; 3]) -> bool {
    let mut p = [[0.0f64; 3]; 3];
    for (slot, index) in p.iter_mut().zip(corners) {
        let Some(v) = position(mesh, index) else {
            return false;
        };
        if !all_finite(v) {
            return false;
        }
        *slot = [f64::from(v[0]), f64::from(v[1]), f64::from(v[2])];
    }
    let e1 = [p[1][0] - p[0][0], p[1][1] - p[0][1], p[1][2] - p[0][2]];
    let e2 = [p[2][0] - p[0][0], p[2][1] - p[0][1], p[2][2] - p[0][2]];
    let cross = [
        e1[1] * e2[2] - e1[2] * e2[1],
        e1[2] * e2[0] - e1[0] * e2[2],
        e1[0] * e2[1] - e1[1] * e2[0],
    ];
    let cross_len = (cross[0] * cross[0] + cross[1] * cross[1] + cross[2] * cross[2]).sqrt();
    cross_len * 0.5 <= DEGENERATE_AREA_M2
}

fn tri_degenerate(mesh: &CookedMesh) -> bool {
    mesh.indices
        .chunks_exact(3)
        .any(|t| triangle_is_degenerate(mesh, [t[0], t[1], t[2]]))
}

/// Drop every zero-area triangle. A trailing partial triangle stays as it is.
fn drop_degenerate_tris(mesh: &mut CookedMesh) {
    let mut kept = Vec::with_capacity(mesh.indices.len());
    for t in mesh.indices.chunks_exact(3) {
        if !triangle_is_degenerate(mesh, [t[0], t[1], t[2]]) {
            kept.extend_from_slice(t);
        }
    }
    kept.extend_from_slice(mesh.indices.chunks_exact(3).remainder());
    mesh.indices = kept;
}

/// A finite normal whose length is off unit length by more than the tolerance.
fn normal_not_unit(mesh: &CookedMesh) -> bool {
    mesh.normals
        .iter()
        .any(|n| all_finite(n) && (length(n) - 1.0).abs() > NORMAL_UNIT_TOL)
}

/// Renormalize each off-unit finite normal. A zero normal takes `NORMAL_FALLBACK`.
fn renormalize_normals(mesh: &mut CookedMesh) {
    for n in &mut mesh.normals {
        if !all_finite(n) || (length(n) - 1.0).abs() <= NORMAL_UNIT_TOL {
            continue;
        }
        *n = unit_dir(n).unwrap_or(NORMAL_FALLBACK);
    }
}

/// Unit direction of `v` in f32. The largest component is scaled to 1 first,
/// so the squares cannot overflow. `None` for a zero or non-finite vector.
fn unit_dir(v: &[f32; 3]) -> Option<[f32; 3]> {
    let big = v.iter().fold(0.0f32, |acc, x| acc.max(x.abs()));
    if big == 0.0 || !big.is_finite() {
        return None;
    }
    let s = [v[0] / big, v[1] / big, v[2] / big];
    let len = (s[0] * s[0] + s[1] * s[1] + s[2] * s[2]).sqrt();
    Some([s[0] / len, s[1] / len, s[2] / len])
}

/// Any NaN or infinity in positions, normals, tangents or uv0.
fn float_nonfinite(mesh: &CookedMesh) -> bool {
    mesh.positions.iter().any(|v| !all_finite(v))
        || mesh.normals.iter().any(|v| !all_finite(v))
        || mesh.tangents.iter().any(|v| !all_finite(v))
        || mesh.uv0.iter().any(|v| !all_finite(v))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two triangles on the XY plane: a quad every rule accepts.
    fn clean() -> CookedMesh {
        CookedMesh {
            positions: vec![
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [1.0, 1.0, 0.0],
                [0.0, 1.0, 0.0],
            ],
            normals: vec![[0.0, 0.0, 1.0]; 4],
            uv0: vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
            tangents: vec![[1.0, 0.0, 0.0, 1.0]; 4],
            indices: vec![0, 1, 2, 0, 2, 3],
        }
    }

    /// A fifth vertex at (0.5, 0, 0) and a triangle over vertices 0, 1 and 4.
    /// All three sit on the x axis, so the triangle has no area.
    fn add_collinear_tri(mesh: &mut CookedMesh) {
        mesh.positions.push([0.5, 0.0, 0.0]);
        mesh.normals.push([0.0, 0.0, 1.0]);
        mesh.uv0.push([0.5, 0.0]);
        mesh.tangents.push([1.0, 0.0, 0.0, 1.0]);
        mesh.indices.extend([0, 1, 4]);
    }

    /// One defect on top of the clean quad, named by its rule id.
    fn seeded(id: &str) -> CookedMesh {
        let mut mesh = clean();
        match id {
            "tangents.missing" => mesh.tangents.clear(),
            "tri.degenerate" => add_collinear_tri(&mut mesh),
            "normal.not_unit" => mesh.normals[1] = [0.0, 0.0, 2.0],
            "float.nonfinite" => mesh.positions[2][0] = f32::NAN,
            other => panic!("no seed for rule {other}"),
        }
        mesh
    }

    /// Every defect at once. Vertex 2 is NaN, so its two triangles skip the
    /// zero-area check and only `float.nonfinite` reports them.
    fn all_defects() -> CookedMesh {
        let mut mesh = clean();
        add_collinear_tri(&mut mesh);
        mesh.tangents.clear();
        mesh.normals[1] = [0.0, 0.0, 2.0];
        mesh.positions[2][0] = f32::NAN;
        mesh
    }

    fn ids(mesh: &CookedMesh) -> Vec<&'static str> {
        validate(mesh).into_iter().map(|hit| hit.id).collect()
    }

    #[test]
    fn validate_clean_mesh_reports_none() {
        assert!(validate(&clean()).is_empty());
    }

    #[test]
    fn validate_seeded_defect_reports_exactly_its_rule_id() {
        for rule in RULES {
            let hits = validate(&seeded(rule.id));
            assert_eq!(
                hits,
                vec![Hit {
                    id: rule.id,
                    severity: rule.severity
                }],
                "seed for {}",
                rule.id
            );
        }
    }

    #[test]
    fn validate_all_defects_reports_in_rule_order() {
        assert_eq!(
            ids(&all_defects()),
            [
                "tangents.missing",
                "tri.degenerate",
                "normal.not_unit",
                "float.nonfinite"
            ]
        );
    }

    #[test]
    fn validate_two_runs_give_identical_hits() {
        let mesh = all_defects();
        assert_eq!(validate(&mesh), validate(&mesh));
    }

    #[test]
    fn validate_non_finite_normal_is_float_only() {
        let mut mesh = clean();
        mesh.normals[0] = [f32::INFINITY, 0.0, 0.0];
        assert_eq!(ids(&mesh), ["float.nonfinite"]);
    }

    #[test]
    fn fix_clears_its_seeded_hit() {
        for rule in RULES.iter().filter(|rule| rule.fix.is_some()) {
            let mut mesh = seeded(rule.id);
            assert_eq!(fix(&mut mesh, rule.id), Ok(()), "fix for {}", rule.id);
            assert!(
                ids(&mesh).is_empty(),
                "fix for {} left {:?}",
                rule.id,
                ids(&mesh)
            );
        }
    }

    #[test]
    fn fix_clears_its_hit_and_no_other() {
        for rule in RULES.iter().filter(|rule| rule.fix.is_some()) {
            let expected: Vec<Hit> = validate(&all_defects())
                .into_iter()
                .filter(|hit| hit.id != rule.id)
                .collect();
            let mut mesh = all_defects();
            assert_eq!(fix(&mut mesh, rule.id), Ok(()), "fix for {}", rule.id);
            assert_eq!(validate(&mesh), expected, "fix for {}", rule.id);
        }
    }

    #[test]
    fn fix_refuses_unknown_id_and_rule_without_repair() {
        let mut mesh = seeded("float.nonfinite");
        assert_eq!(
            fix(&mut mesh, "float.nonfinite"),
            Err(FixError::NoFix("float.nonfinite"))
        );
        assert_eq!(
            fix(&mut mesh, "nope"),
            Err(FixError::Unknown("nope".to_owned()))
        );
    }

    #[test]
    fn fix_reports_still_hit_when_repair_cannot_cover_the_mesh() {
        // No normals: the tangent solve has nothing to orthogonalize against.
        let mut mesh = clean();
        mesh.normals.clear();
        mesh.tangents.clear();
        assert_eq!(
            fix(&mut mesh, "tangents.missing"),
            Err(FixError::StillHit("tangents.missing"))
        );
    }
}
