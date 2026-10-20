//! FBX import behind the `fbx` feature: one [`GeoMesh`] per mesh node, in meters, +Y up.
//!
//! Donor: `ufbx` 0.11.5 (MIT OR Unlicense), its C loader compiled from source, so the feature is
//! off by default (`crates/geo/Cargo.toml:30-31`) and no tick crate links it.
//!
//! Invariants: the input is a byte slice, never a path (`load_external_files` is off, so a file
//! cannot pull another file off the disk); node order is `scene.nodes` order and vertex order is
//! the first corner that asks for an attribute trio, so the same bytes give the same meshes; a
//! mesh node whose faces triangulate to nothing is skipped; every refusal is a [`GeoError`] and
//! a ufbx error keeps the first 80 characters of its text in [`GeoError::Donor`]. `f32` is fine
//! here: this is cook-time, like the rest of `engine-geo`.

use std::collections::BTreeMap;

use ufbx::{CoordinateAxes, CoordinateAxis, LoadOpts, Mesh, Node, Vec3};

use crate::mesh::{narrow, to_u32, GeoError, GeoMesh};

/// Target space of the result: right-handed, +X right, +Y up, +Z toward the viewer, one unit is
/// one meter. ufbx rotates and scales the file's own axes and units into it while loading.
const TARGET_AXES: CoordinateAxes = CoordinateAxes {
    right: CoordinateAxis::PositiveX,
    up: CoordinateAxis::PositiveY,
    front: CoordinateAxis::PositiveZ,
};

/// The load options every import uses: the target space above, generated normals, no external
/// files and no animation (none of it reaches a mesh).
fn load_options() -> LoadOpts<'static> {
    LoadOpts {
        target_axes: TARGET_AXES,
        target_unit_meters: 1.0,
        generate_missing_normals: true,
        load_external_files: false,
        ignore_missing_external_files: true,
        ignore_animation: true,
        ..LoadOpts::default()
    }
}

/// Import every mesh node of an FBX file: triangulated by ufbx, in meters, +Y up, positions,
/// normals and UV0 per vertex.
///
/// `GeoError::Empty` when the file holds no mesh with a triangle. Any ufbx failure becomes
/// `GeoError::Donor` with the first 80 characters of its text; nothing here panics on bad bytes.
pub fn import_fbx(bytes: &[u8]) -> Result<Vec<GeoMesh>, GeoError> {
    let scene = ufbx::load_memory(bytes, load_options()).map_err(donor)?;
    let mut meshes = Vec::new();
    for node in &scene.nodes {
        let Some(mesh_ref) = node.mesh.as_ref() else {
            continue;
        };
        let mesh: &Mesh = mesh_ref;
        let converted = mesh_of(node, mesh)?;
        // A node that draws no triangle (an empty or point-only mesh) is not a solid.
        if !converted.indices.is_empty() {
            meshes.push(converted);
        }
    }
    if meshes.is_empty() {
        return Err(GeoError::Empty);
    }
    Ok(meshes)
}

/// One ufbx mesh node as a [`GeoMesh`], in the target space.
fn mesh_of(node: &Node, mesh: &Mesh) -> Result<GeoMesh, GeoError> {
    if !mesh.vertex_position.exists {
        return Err(GeoError::Malformed("fbx mesh has no vertex positions"));
    }
    let to_world = node.geometry_to_world;
    let to_world_normals = ufbx::matrix_for_normals(&to_world);
    let sources = Sources {
        mesh,
        to_world: &to_world,
        to_world_normals: &to_world_normals,
        has_normals: mesh.vertex_normal.exists,
        has_uvs: mesh.vertex_uv.exists,
    };

    let mut out = GeoMesh::default();
    let mut seen: BTreeMap<u32, u32> = BTreeMap::new();
    let mut corners: Vec<u32> = Vec::new();
    for face in &mesh.faces {
        // ufbx writes absolute corner indices, the same numbering the vertex attribute
        // `indices` arrays use.
        ufbx::triangulate_face_vec(&mut corners, mesh, *face);
        for &corner in &corners {
            let vertex = match seen.get(&corner) {
                Some(&vertex) => vertex,
                None => {
                    let vertex = push_vertex(&mut out, corner, &sources)?;
                    seen.insert(corner, vertex);
                    vertex
                }
            };
            out.indices.push(vertex);
        }
    }
    out.validate()?;
    Ok(out)
}

/// One mesh's attributes plus its node transform, needed to append a vertex.
struct Sources<'a> {
    mesh: &'a Mesh,
    to_world: &'a ufbx::Matrix,
    to_world_normals: &'a ufbx::Matrix,
    has_normals: bool,
    has_uvs: bool,
}

/// Append the vertex a corner index names and return its new index.
fn push_vertex(out: &mut GeoMesh, corner: u32, sources: &Sources<'_>) -> Result<u32, GeoError> {
    let mesh = sources.mesh;
    let position = corner_value(
        &mesh.vertex_position.indices,
        &mesh.vertex_position.values,
        corner,
    )?;
    let world = ufbx::transform_position(sources.to_world, position);
    let vertex = to_u32(out.positions.len());
    out.positions
        .push([narrow(world.x), narrow(world.y), narrow(world.z)]);
    if sources.has_normals {
        let normal = corner_value(
            &mesh.vertex_normal.indices,
            &mesh.vertex_normal.values,
            corner,
        )?;
        let world = ufbx::transform_direction(sources.to_world_normals, normal);
        out.normals.push(unit(world));
    }
    if sources.has_uvs {
        let uv = corner_value(&mesh.vertex_uv.indices, &mesh.vertex_uv.values, corner)?;
        out.uvs.push([narrow(uv.x), narrow(uv.y)]);
    }
    Ok(vertex)
}

/// The attribute value a corner tuple points at: `indices[corner]`, then `values[that]`.
///
/// One accessor for every attribute, so a truncated or mutated tuple is a typed refusal and
/// never a slice panic.
fn corner_value<T: Copy>(indices: &[u32], values: &[T], corner: u32) -> Result<T, GeoError> {
    let corner = usize::try_from(corner).map_err(|_| GeoError::Malformed("fbx corner"))?;
    let value = indices
        .get(corner)
        .copied()
        .ok_or(GeoError::Malformed("fbx attribute index"))?;
    let value = usize::try_from(value).map_err(|_| GeoError::Malformed("fbx attribute index"))?;
    values
        .get(value)
        .copied()
        .ok_or(GeoError::Malformed("fbx attribute value"))
}

/// A transformed normal as a unit vector; a zero normal stays zero.
fn unit(v: Vec3) -> [f32; 3] {
    let length = (v.x * v.x + v.y * v.y + v.z * v.z).sqrt();
    if length > 0.0 {
        [
            narrow(v.x / length),
            narrow(v.y / length),
            narrow(v.z / length),
        ]
    } else {
        [0.0, 0.0, 0.0]
    }
}

/// A ufbx failure as [`GeoError::Donor`], text cut to 80 characters on a char boundary.
fn donor(error: ufbx::Error) -> GeoError {
    GeoError::Donor(short(&format!("{error:?}")))
}

/// The first 80 characters of `text`.
fn short(text: &str) -> String {
    text.chars().take(80).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn donor_text_keeps_the_first_eighty_characters() {
        let long = "x".repeat(120);
        assert_eq!(short(&long).chars().count(), 80);
        assert_eq!(short("boom"), "boom");
        // Counted in characters, never bytes: no panic on a multibyte cut.
        let wide = "é".repeat(100);
        assert_eq!(short(&wide).chars().count(), 80);
        let error = short("geometry cache missing");
        assert_eq!(error, "geometry cache missing");
    }

    #[test]
    fn bytes_that_are_not_fbx_are_a_typed_error() {
        let error = import_fbx(b"not an fbx file").expect_err("garbage never imports");
        assert!(matches!(error, GeoError::Donor(_)), "{error:?}");
        assert!(
            matches!(import_fbx(&[]), Err(GeoError::Donor(_) | GeoError::Empty)),
            "empty input is a refusal, not a panic"
        );
    }
}
