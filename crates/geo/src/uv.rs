//! UV unwrap and chart packing with xatlas (MIT; vendored source, see `vendor/README.md`).
//!
//! `unwrap` cuts the mesh into charts, flattens each and packs them into one atlas. Vertices
//! on a cut are split, so the output has more vertices than the input (`source_vertex` says
//! which input vertex each came from). The new UVs replace `uvs` and lie in `0..=1`.
//! `overlapping_chart_pairs` is the independent check that no two charts share atlas area.

use std::collections::{BTreeMap, BTreeSet};

use crate::mesh::{frac, ix, narrow, scaled_count, GeoError, GeoMesh};

/// Chart and pack settings (a subset of xatlas `ChartOptions` and `PackOptions`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UnwrapOptions {
    /// Target atlas edge in texels: the packer scales charts so about `resolution` squared texels
    /// are needed; the packed atlas lands near it (`Unwrap::atlas_size` has the real size).
    pub resolution: u32,
    /// Texels of padding around each chart.
    pub padding: u32,
    /// Largest chart area before it is split (0 = no limit).
    pub max_chart_area: f32,
    /// Lower values make more, flatter charts.
    pub max_cost: f32,
    /// Chart growing rounds (more = better charts, slower).
    pub max_iterations: u32,
    /// Use the input UVs as a hint for where to cut.
    pub use_input_uvs: bool,
    /// Try harder to pack (slower, tighter).
    pub brute_force: bool,
}

impl Default for UnwrapOptions {
    fn default() -> Self {
        Self {
            resolution: 1024,
            padding: 4,
            max_chart_area: 0.0,
            max_cost: 2.0,
            max_iterations: 1,
            use_input_uvs: false,
            brute_force: false,
        }
    }
}

/// An unwrapped mesh.
#[derive(Clone, Debug, PartialEq)]
pub struct Unwrap {
    /// New vertices (positions and normals copied from the source, new UVs) and indices.
    pub mesh: GeoMesh,
    /// For each output vertex, the input vertex it came from.
    pub source_vertex: Vec<u32>,
    /// For each output triangle, its chart.
    pub chart_of_triangle: Vec<u32>,
    /// Number of charts.
    pub chart_count: u32,
    /// Atlas size in texels.
    pub atlas_size: [u32; 2],
    /// Texels per world unit chosen by the packer.
    pub texels_per_unit: f32,
}

impl Unwrap {
    /// Output vertices over input vertices (1.0 = no splits).
    #[must_use]
    pub fn vertex_growth(&self, input_vertices: usize) -> f64 {
        frac(self.mesh.positions.len(), input_vertices)
    }
}

#[expect(
    unsafe_code,
    reason = "FFI to the vendored xatlas shim; every call is one small wrapper"
)]
mod ffi {
    use crate::mesh::{ix, GeoError};

    /// xatlas chart and pack settings, field for field the shim's `GeoXatlasParams`.
    #[repr(C)]
    #[derive(Default)]
    pub struct Params {
        pub max_chart_area: f32,
        pub max_boundary_length: f32,
        pub normal_deviation_weight: f32,
        pub roundness_weight: f32,
        pub straightness_weight: f32,
        pub normal_seam_weight: f32,
        pub texture_seam_weight: f32,
        pub max_cost: f32,
        pub max_iterations: u32,
        pub use_input_uvs: u32,
        pub fix_winding: u32,
        pub max_chart_size: u32,
        pub padding: u32,
        pub texels_per_unit: f32,
        pub resolution: u32,
        pub bilinear: u32,
        pub block_align: u32,
        pub brute_force: u32,
        pub rotate_charts_to_axis: u32,
        pub rotate_charts: u32,
    }

    /// Atlas facts the shim reports, field for field `GeoXatlasInfo`.
    #[repr(C)]
    #[derive(Default)]
    pub struct Info {
        pub width: u32,
        pub height: u32,
        pub chart_count: u32,
        pub vertex_count: u32,
        pub index_count: u32,
        pub texels_per_unit: f32,
    }

    extern "C" {
        fn geo_xatlas_generate(
            positions: *const f32,
            normals: *const f32,
            uvs: *const f32,
            vertex_count: u32,
            indices: *const u32,
            index_count: u32,
            params: *const Params,
            error_code: *mut i32,
            info: *mut Info,
        ) -> *mut core::ffi::c_void;
        fn geo_xatlas_copy(
            handle: *const core::ffi::c_void,
            xref: *mut u32,
            uv: *mut f32,
            chart: *mut i32,
            indices: *mut u32,
        );
        fn geo_xatlas_destroy(handle: *mut core::ffi::c_void);
    }

    /// Result copied out of the C++ atlas.
    pub struct Output {
        pub info: Info,
        pub xref: Vec<u32>,
        pub uv: Vec<f32>,
        pub chart: Vec<i32>,
        pub indices: Vec<u32>,
    }

    /// The preconditions the C side relies on. Returns the vertex count.
    fn check_inputs(
        positions: &[f32],
        normals: &[f32],
        uvs: &[f32],
        indices: &[u32],
    ) -> Result<usize, GeoError> {
        if !positions.len().is_multiple_of(3) {
            return Err(GeoError::Malformed("positions length not a multiple of 3"));
        }
        let n = positions.len() / 3;
        if !normals.is_empty() && normals.len() != 3 * n {
            return Err(GeoError::Malformed("normals length"));
        }
        if !uvs.is_empty() && uvs.len() != 2 * n {
            return Err(GeoError::Malformed("uvs length"));
        }
        if !indices.len().is_multiple_of(3) {
            return Err(GeoError::Malformed("index count not a multiple of 3"));
        }
        if indices.iter().any(|&i| ix(i) >= n) {
            return Err(GeoError::Malformed("index out of range"));
        }
        Ok(n)
    }

    /// Run xatlas. The checks run before the FFI call, so no caller can reach the C side with a
    /// bad mesh. `normals` and `uvs` are empty or `3 * n` and `2 * n` floats; `positions` is
    /// `3 * n`; `indices` are whole triangles, all below `n`. `Err` is `Malformed` for bad input
    /// and `Donor` when xatlas itself fails.
    pub fn generate(
        positions: &[f32],
        normals: &[f32],
        uvs: &[f32],
        indices: &[u32],
        params: &Params,
    ) -> Result<Output, GeoError> {
        let vertices = check_inputs(positions, normals, uvs, indices)?;
        let n = u32::try_from(vertices).map_err(|_| GeoError::Malformed("too many vertices"))?;
        let index_count =
            u32::try_from(indices.len()).map_err(|_| GeoError::Malformed("too many indices"))?;
        let mut code = 0_i32;
        let mut info = Info::default();
        let ptr_or_null = |s: &[f32]| {
            if s.is_empty() {
                core::ptr::null()
            } else {
                s.as_ptr()
            }
        };
        // SAFETY: every pointer is valid for the lengths the shim reads (`3n`, `3n` or null,
        // `2n` or null floats and `index_count` u32); `params`, `code` and `info` are live
        // locals; the shim copies the input, so nothing outlives this call.
        let handle = unsafe {
            geo_xatlas_generate(
                positions.as_ptr(),
                ptr_or_null(normals),
                ptr_or_null(uvs),
                n,
                indices.as_ptr(),
                index_count,
                params,
                &mut code,
                &mut info,
            )
        };
        if handle.is_null() {
            return Err(GeoError::Donor(super::error_text(code).to_string()));
        }
        let mut out = Output {
            xref: vec![0; ix(info.vertex_count)],
            uv: vec![0.0; 2 * ix(info.vertex_count)],
            chart: vec![0; ix(info.vertex_count)],
            indices: vec![0; ix(info.index_count)],
            info,
        };
        // SAFETY: the buffers have exactly the vertex and index counts the shim reported for
        // this handle, and the handle is destroyed once, after the copy.
        unsafe {
            geo_xatlas_copy(
                handle,
                out.xref.as_mut_ptr(),
                out.uv.as_mut_ptr(),
                out.chart.as_mut_ptr(),
                out.indices.as_mut_ptr(),
            );
            geo_xatlas_destroy(handle);
        }
        Ok(out)
    }
}

fn error_text(code: i32) -> &'static str {
    match code {
        1 => "unspecified error",
        2 => "index out of range",
        3 => "invalid face vertex count",
        4 => "index count not a multiple of 3",
        _ => "atlas did not return one mesh",
    }
}

/// Unwrap and pack `mesh`. Existing UVs are ignored unless `use_input_uvs` is set.
pub fn unwrap(mesh: &GeoMesh, options: &UnwrapOptions) -> Result<Unwrap, GeoError> {
    mesh.validate()?;
    if mesh.indices.is_empty() {
        return Err(GeoError::Empty);
    }
    let flat = |v: &[[f32; 3]]| v.iter().flatten().copied().collect::<Vec<f32>>();
    let positions = flat(&mesh.positions);
    let normals = flat(&mesh.normals);
    let uvs: Vec<f32> = if options.use_input_uvs {
        mesh.uvs.iter().flatten().copied().collect()
    } else {
        Vec::new()
    };
    let params = ffi::Params {
        max_chart_area: options.max_chart_area,
        max_boundary_length: 0.0,
        normal_deviation_weight: 2.0,
        roundness_weight: 0.01,
        straightness_weight: 6.0,
        normal_seam_weight: 4.0,
        texture_seam_weight: 0.5,
        max_cost: options.max_cost,
        max_iterations: options.max_iterations.max(1),
        use_input_uvs: u32::from(options.use_input_uvs && !uvs.is_empty()),
        fix_winding: 0,
        max_chart_size: 0,
        padding: options.padding,
        texels_per_unit: 0.0,
        resolution: options.resolution.max(16),
        bilinear: 1,
        block_align: 0,
        brute_force: u32::from(options.brute_force),
        rotate_charts_to_axis: 1,
        rotate_charts: 1,
    };
    let out = ffi::generate(&positions, &normals, &uvs, &mesh.indices, &params)?;
    let (w, h) = (
        f64::from(out.info.width.max(1)),
        f64::from(out.info.height.max(1)),
    );
    let mut result = GeoMesh::default();
    for (i, &x) in out.xref.iter().enumerate() {
        result.positions.push(mesh.positions[ix(x)]);
        if !mesh.normals.is_empty() {
            result.normals.push(mesh.normals[ix(x)]);
        }
        result.uvs.push([
            narrow(f64::from(out.uv[2 * i]) / w),
            narrow(f64::from(out.uv[2 * i + 1]) / h),
        ]);
    }
    result.indices = out.indices;
    let chart_of_triangle = result
        .indices
        .chunks_exact(3)
        .map(|t| u32::try_from(out.chart[ix(t[0])]).unwrap_or(u32::MAX))
        .collect();
    Ok(Unwrap {
        mesh: result,
        source_vertex: out.xref,
        chart_of_triangle,
        chart_count: out.info.chart_count,
        atlas_size: [out.info.width, out.info.height],
        texels_per_unit: out.info.texels_per_unit,
    })
}

/// Do two UV triangles share area (not just an edge or a corner)?
fn triangles_overlap(a: [[f64; 2]; 3], b: [[f64; 2]; 3]) -> bool {
    const EPS: f64 = 1e-9;
    for tri in [a, b] {
        for k in 0..3 {
            let (p, q) = (tri[k], tri[(k + 1) % 3]);
            let axis = [-(q[1] - p[1]), q[0] - p[0]];
            let project = |t: [[f64; 2]; 3]| {
                let d: Vec<f64> = t.iter().map(|v| v[0] * axis[0] + v[1] * axis[1]).collect();
                (
                    d.iter().copied().fold(f64::INFINITY, f64::min),
                    d.iter().copied().fold(f64::NEG_INFINITY, f64::max),
                )
            };
            let (a_lo, a_hi) = project(a);
            let (b_lo, b_hi) = project(b);
            let len = (axis[0] * axis[0] + axis[1] * axis[1]).sqrt();
            if len > 0.0 && (a_hi.min(b_hi) - a_lo.max(b_lo)) / len <= EPS {
                return false;
            }
        }
    }
    true
}

/// Number of chart pairs whose UV triangles share area, found by an exact separating-axis test
/// over a uniform grid. 0 means every chart has its own space in the atlas.
#[must_use]
pub fn overlapping_chart_pairs(
    uvs: &[[f32; 2]],
    indices: &[u32],
    chart_of_triangle: &[u32],
) -> usize {
    const GRID: usize = 128;
    let tri = |t: usize| -> [[f64; 2]; 3] {
        [0, 1, 2].map(|k| uvs[ix(indices[3 * t + k])].map(f64::from))
    };
    let mut cells: BTreeMap<(usize, usize), Vec<usize>> = BTreeMap::new();
    let cell = |v: f64| scaled_count(GRID, narrow(v.clamp(0.0, 1.0))).min(GRID - 1);
    for t in 0..indices.len() / 3 {
        let p = tri(t);
        let (x0, x1) = (
            cell(p.iter().map(|v| v[0]).fold(f64::INFINITY, f64::min)),
            cell(p.iter().map(|v| v[0]).fold(f64::NEG_INFINITY, f64::max)),
        );
        let (y0, y1) = (
            cell(p.iter().map(|v| v[1]).fold(f64::INFINITY, f64::min)),
            cell(p.iter().map(|v| v[1]).fold(f64::NEG_INFINITY, f64::max)),
        );
        for x in x0..=x1 {
            for y in y0..=y1 {
                cells.entry((x, y)).or_default().push(t);
            }
        }
    }
    let mut pairs: BTreeSet<(u32, u32)> = BTreeSet::new();
    let mut tested: BTreeSet<(usize, usize)> = BTreeSet::new();
    for list in cells.values() {
        for (i, &a) in list.iter().enumerate() {
            for &b in &list[i + 1..] {
                let (ca, cb) = (chart_of_triangle[a], chart_of_triangle[b]);
                if ca == cb || pairs.contains(&(ca.min(cb), ca.max(cb))) {
                    continue;
                }
                if !tested.insert((a.min(b), a.max(b))) {
                    continue;
                }
                if triangles_overlap(tri(a), tri(b)) {
                    pairs.insert((ca.min(cb), ca.max(cb)));
                }
            }
        }
    }
    pairs.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::primitives::{cuboid, cuboid_faces, torus, uv_sphere};

    fn check(mesh: &GeoMesh) -> Unwrap {
        let u = unwrap(mesh, &UnwrapOptions::default()).unwrap();
        u.mesh.validate().unwrap();
        assert_eq!(u.mesh.triangle_count(), mesh.triangle_count());
        assert_eq!(u.chart_of_triangle.len(), mesh.triangle_count());
        assert!(
            u.mesh
                .uvs
                .iter()
                .flatten()
                .all(|&v| (0.0..=1.0).contains(&v)),
            "uv range"
        );
        assert_eq!(
            overlapping_chart_pairs(&u.mesh.uvs, &u.mesh.indices, &u.chart_of_triangle),
            0,
            "overlapping charts"
        );
        // The surface is unchanged: welded output is the welded input.
        assert!((u.mesh.volume() - mesh.volume()).abs() < 1e-6 * mesh.volume().abs().max(1.0));
        assert_eq!(u.mesh.open_edge_count(), mesh.open_edge_count());
        u
    }

    #[test]
    fn solids_unwrap_without_overlap_and_repeat() {
        for mesh in [
            cuboid([1.0, 2.0, 3.0]),
            cuboid_faces([1.0; 3]),
            uv_sphere(1.0, 12, 16),
            torus(1.0, 0.3, 20, 10),
        ] {
            let a = check(&mesh);
            let b = unwrap(&mesh, &UnwrapOptions::default()).unwrap();
            assert_eq!(a, b);
            assert_eq!(a.mesh.digest(), b.mesh.digest());
            assert!(a.chart_count >= 1);
        }
    }

    #[test]
    fn a_box_gets_separate_charts_so_it_splits_vertices() {
        let u = check(&cuboid([1.0; 3]));
        assert!(u.chart_count >= 2);
        assert!(
            u.vertex_growth(8) > 1.0,
            "a cube cannot flatten without cuts"
        );
    }

    #[test]
    fn overlap_checker_sees_overlap_and_ignores_touching() {
        let uvs = [
            [0.0, 0.0],
            [0.5, 0.0],
            [0.0, 0.5],
            [0.1, 0.1],
            [0.6, 0.1],
            [0.1, 0.6],
        ];
        let idx = [0, 1, 2, 3, 4, 5];
        assert_eq!(overlapping_chart_pairs(&uvs, &idx, &[0, 1]), 1);
        let touching = [
            [0.0, 0.0],
            [0.5, 0.0],
            [0.0, 0.5],
            [0.5, 0.0],
            [1.0, 0.0],
            [0.5, 0.5],
        ];
        assert_eq!(overlapping_chart_pairs(&touching, &idx, &[0, 1]), 0);
        // Same chart: never counted.
        assert_eq!(overlapping_chart_pairs(&uvs, &idx, &[0, 0]), 0);
    }

    #[test]
    fn bad_input_is_refused() {
        assert_eq!(
            unwrap(&GeoMesh::default(), &UnwrapOptions::default()),
            Err(GeoError::Empty)
        );
        let mut bad = cuboid([1.0; 3]);
        bad.indices[0] = 40;
        assert!(matches!(
            unwrap(&bad, &UnwrapOptions::default()),
            Err(GeoError::Malformed(_))
        ));
    }

    #[test]
    fn ffi_wrapper_refuses_bad_arrays_before_the_c_call() {
        // The wrapper checks its own preconditions: these calls never reach xatlas.
        let m = cuboid([1.0; 3]);
        let positions: Vec<f32> = m.positions.iter().flatten().copied().collect();
        let normals: Vec<f32> = (0..m.positions.len())
            .flat_map(|_| [0.0_f32, 0.0, 1.0])
            .collect();
        let params = ffi::Params::default();
        let refuse = |pos: &[f32], nrm: &[f32], uv: &[f32], idx: &[u32]| {
            ffi::generate(pos, nrm, uv, idx, &params).err()
        };
        // Two floats short: the C side would read past the end of the normals.
        assert_eq!(
            refuse(&positions, &normals[..normals.len() - 2], &[], &m.indices),
            Some(GeoError::Malformed("normals length"))
        );
        assert_eq!(
            refuse(&positions, &normals, &[0.0; 3], &m.indices),
            Some(GeoError::Malformed("uvs length"))
        );
        assert_eq!(
            refuse(&positions[..positions.len() - 1], &[], &[], &m.indices),
            Some(GeoError::Malformed("positions length not a multiple of 3"))
        );
        assert_eq!(
            refuse(&positions, &normals, &[], &m.indices[..35]),
            Some(GeoError::Malformed("index count not a multiple of 3"))
        );
        let mut far = m.indices.clone();
        far[5] = 40;
        assert_eq!(
            refuse(&positions, &normals, &[], &far),
            Some(GeoError::Malformed("index out of range"))
        );
        // A good mesh still unwraps through the public call.
        check(&m);
    }
}
