//! The modeling ops of the `geo` pack, backed by the cook-time `engine-geo` crate. Compiled only
//! with engine-endpoint's `geo` feature (off by default), so the default build carries neither
//! the donor libraries nor their C++ compile step.
//!
//! Each op converts the recipe mesh to an `engine_geo::GeoMesh`, runs the tool and converts
//! back; a tool refusal becomes a typed `bad_recipe` error that names the op index and the fix.

use engine_geo::{
    boolean as geo_boolean, collision as geo_collision, lod as geo_lod, ops as geo_ops, primitives,
    uv as geo_uv, GeoError, GeoMesh,
};

use super::{
    bad, check_extent, BoolKind, CollisionFacts, CollisionOut, GeoCutter, GeoSolid, LodOut, Mesh,
    UvOut, MAX_ARRAY_COPIES, MAX_COLLISION_PARTS, MAX_MODEL_TRIS, MAX_WELD_M,
};
use crate::mcp::McpToolError;

fn count(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

fn to_geo(mesh: &Mesh) -> GeoMesh {
    GeoMesh {
        positions: mesh.verts.clone(),
        normals: Vec::new(),
        uvs: mesh.uvs.clone(),
        indices: mesh.tris.iter().flatten().copied().collect(),
    }
}

fn from_geo(geo: GeoMesh) -> Mesh {
    Mesh {
        tris: geo
            .indices
            .chunks_exact(3)
            .map(|t| [t[0], t[1], t[2]])
            .collect(),
        verts: geo.positions,
        uvs: geo.uvs,
    }
}

fn refused(index: usize, error: GeoError) -> McpToolError {
    let fix = match error {
        GeoError::NotClosed => "; a boolean needs a closed source, use a cube not a plane",
        GeoError::Empty => "; the result has no faces, move the cutter or pick another kind",
        _ => "",
    };
    bad(format!(
        "ops[{index}] refused by the geo kernel: {error}{fix}"
    ))
}

fn too_big(index: usize, tris: usize) -> Result<(), McpToolError> {
    if tris > MAX_MODEL_TRIS {
        Err(McpToolError::new(
            "too_big",
            format!(
                "ops[{index}] works on {tris} triangles; at most {MAX_MODEL_TRIS}; use a smaller source or cutter"
            ),
        ))
    } else {
        Ok(())
    }
}

fn segments_ok(index: usize, label: &str, value: u32, lo: u32) -> Result<usize, McpToolError> {
    if (lo..=64).contains(&value) {
        Ok(usize::try_from(value).unwrap_or(64))
    } else {
        Err(bad(format!(
            "ops[{index}].cutter.{label} {value} is outside {lo}..=64; send a count in range"
        )))
    }
}

fn offset_ok(index: usize, label: &str, offset: &[f32; 3]) -> Result<(), McpToolError> {
    for (axis, value) in offset.iter().enumerate() {
        if !value.is_finite() || value.abs() > super::MAX_SOURCE_SIZE_M {
            return Err(bad(format!(
                "ops[{index}].{label}[{axis}] is {value}; send meters within 1000"
            )));
        }
    }
    Ok(())
}

fn cutter_mesh(index: usize, cutter: &GeoCutter) -> Result<GeoMesh, McpToolError> {
    offset_ok(index, "cutter.offset", &cutter.offset)?;
    let solid = match &cutter.solid {
        GeoSolid::Cube { size } => {
            for (label, value) in ["x", "y", "z"].into_iter().zip(size) {
                check_extent(label, *value)?;
            }
            primitives::cuboid(*size)
        }
        GeoSolid::Sphere {
            radius,
            rings,
            segments,
        } => {
            check_extent("radius", *radius)?;
            primitives::uv_sphere(
                *radius,
                segments_ok(index, "rings", *rings, 2)?,
                segments_ok(index, "segments", *segments, 3)?,
            )
        }
        GeoSolid::Cylinder {
            radius,
            height,
            segments,
        } => {
            check_extent("radius", *radius)?;
            check_extent("height", *height)?;
            primitives::cylinder(
                *radius,
                *height,
                segments_ok(index, "segments", *segments, 3)?,
            )
        }
    };
    Ok(solid.placed([1.0; 3], 0, cutter.offset))
}

/// `boolean`: cut the mesh with the cutter; the result is welded and has no UVs.
pub(super) fn boolean(
    mesh: &mut Mesh,
    cutter: &GeoCutter,
    kind: BoolKind,
    index: usize,
) -> Result<(), McpToolError> {
    too_big(index, mesh.tris.len())?;
    let tool = cutter_mesh(index, cutter)?;
    let op = match kind {
        BoolKind::Union => geo_boolean::BoolOp::Union,
        BoolKind::Difference => geo_boolean::BoolOp::Difference,
        BoolKind::Intersection => geo_boolean::BoolOp::Intersection,
    };
    let result = geo_boolean::boolean(&to_geo(mesh), &tool, op).map_err(|e| refused(index, e))?;
    too_big(index, result.triangle_count())?;
    *mesh = from_geo(result);
    Ok(())
}

/// `unwrap`: new UVs; vertices split along chart cuts.
pub(super) fn unwrap(mesh: &mut Mesh, index: usize) -> Result<UvOut, McpToolError> {
    too_big(index, mesh.tris.len())?;
    let source = to_geo(mesh);
    let done = geo_uv::unwrap(&source, &geo_uv::UnwrapOptions::default())
        .map_err(|e| refused(index, e))?;
    let overlaps = geo_uv::overlapping_chart_pairs(
        &done.mesh.uvs,
        &done.mesh.indices,
        &done.chart_of_triangle,
    );
    let out = UvOut {
        charts: done.chart_count,
        atlas: done.atlas_size,
        overlapping_chart_pairs: count(overlaps),
    };
    too_big(index, done.mesh.triangle_count())?;
    *mesh = from_geo(done.mesh);
    Ok(out)
}

/// `lod`: keep a fraction of the triangles with UV seams and borders locked.
pub(super) fn lod(mesh: &mut Mesh, keep: f32, index: usize) -> Result<LodOut, McpToolError> {
    if !keep.is_finite() || !(0.05..=1.0).contains(&keep) {
        return Err(bad(format!(
            "ops[{index}].keep {keep} is outside [0.05, 1]; send the fraction of triangles to keep"
        )));
    }
    too_big(index, mesh.tris.len())?;
    let options = geo_lod::LodOptions {
        target_ratio: keep,
        ..geo_lod::LodOptions::default()
    };
    let result =
        geo_lod::simplify(&to_geo(mesh), &[], 0, 0.0, &options).map_err(|e| refused(index, e))?;
    let out = LodOut {
        triangles_before: count(result.triangles_before),
        triangles_after: count(result.triangles_after),
        locked_vertices: count(result.locked_vertices),
    };
    *mesh = from_geo(result.mesh);
    Ok(out)
}

/// `collision`: closed convex parts; the render mesh stays as it is.
pub(super) fn collision(
    mesh: &Mesh,
    max_hulls: u32,
    index: usize,
) -> Result<CollisionFacts, McpToolError> {
    if !(1..=MAX_COLLISION_PARTS).contains(&max_hulls) {
        return Err(bad(format!(
            "ops[{index}].max_hulls {max_hulls} is outside 1..={MAX_COLLISION_PARTS}; send a part count in range"
        )));
    }
    too_big(index, mesh.tris.len())?;
    let options = geo_collision::HullOptions {
        max_hulls,
        ..geo_collision::HullOptions::default()
    };
    let parts = geo_collision::decompose(&to_geo(mesh), &options).map_err(|e| refused(index, e))?;
    let digest_bits = geo_collision::digest(&parts);
    Ok(CollisionFacts {
        out: CollisionOut {
            parts: count(parts.len()),
            part_vertices: parts.iter().map(|p| count(p.positions.len())).collect(),
            digest: format!("{digest_bits:016x}"),
        },
        digest_bits,
    })
}

/// `mirror`: add the reflection through the origin plane of `axis`.
pub(super) fn mirror(mesh: &mut Mesh, axis: &str, index: usize) -> Result<(), McpToolError> {
    let axis = match axis {
        "x" => geo_ops::Axis::X,
        "y" => geo_ops::Axis::Y,
        "z" => geo_ops::Axis::Z,
        other => {
            return Err(bad(format!(
                "ops[{index}].axis `{other}` is not an axis; send one of x, y, z"
            )))
        }
    };
    too_big(index, mesh.tris.len())?;
    let result = geo_ops::mirror(&to_geo(mesh), axis);
    too_big(index, result.triangle_count())?;
    *mesh = from_geo(result);
    Ok(())
}

/// `array`: `count` copies, copy `k` moved by `k` times `offset`.
pub(super) fn array(
    mesh: &mut Mesh,
    count: u32,
    offset: [f32; 3],
    index: usize,
) -> Result<(), McpToolError> {
    if !(1..=MAX_ARRAY_COPIES).contains(&count) {
        return Err(bad(format!(
            "ops[{index}].count {count} is outside 1..={MAX_ARRAY_COPIES}; send a copy count in range"
        )));
    }
    offset_ok(index, "offset", &offset)?;
    let tris = mesh
        .tris
        .len()
        .saturating_mul(usize::try_from(count).unwrap_or(usize::MAX));
    if tris > MAX_MODEL_TRIS {
        return Err(McpToolError::new(
            "too_big",
            format!(
                "ops[{index}] would make {tris} triangles; at most {MAX_MODEL_TRIS}; send a smaller count"
            ),
        ));
    }
    *mesh = from_geo(geo_ops::array(&to_geo(mesh), count, offset));
    Ok(())
}

/// `weld`: merge vertices closer than `epsilon`; collapsed triangles go.
pub(super) fn weld(mesh: &mut Mesh, epsilon: f32, index: usize) -> Result<(), McpToolError> {
    if !epsilon.is_finite() || epsilon <= 0.0 || epsilon > MAX_WELD_M {
        return Err(bad(format!(
            "ops[{index}].epsilon {epsilon} is outside (0, {MAX_WELD_M}]; send a weld distance in meters"
        )));
    }
    too_big(index, mesh.tris.len())?;
    *mesh = from_geo(geo_ops::weld_by_distance(&to_geo(mesh), epsilon));
    Ok(())
}

/// `flip`: reverse every winding.
pub(super) fn flip(mesh: &mut Mesh, index: usize) -> Result<(), McpToolError> {
    too_big(index, mesh.tris.len())?;
    *mesh = from_geo(geo_ops::flip(&to_geo(mesh)));
    Ok(())
}
