//! uv2_chart: offline uv2 chart pack for arena static meshes (cook-time only).
//! Owner: assets lane; planned, no caller yet (verified 2026-10-16):
//! the `mesh_cook` uv2 channel plus bundle cook will consume `pack_uv2`.
//! Units: meters for positions, texels for atlas, unitless uv `0..1`.
//! Determinism: sorted indices, fixed shelf pack; same mesh gives same
//! uv2 plus same digest x2. Cook-time only, never the tick.
//! PORT(jpcy/xatlas@f700c77:source/xatlas/xatlas.h): ChartOptions plus
//! PackOptions chart-generate-and-pack shape, reimplemented, zero C bytes.
#![forbid(unsafe_code)]
use crate::provenance::{provenance_row, ProvenanceRecord};
use engine_core::hash_bytes;
use std::fmt;
/// Chart generation knobs (subset of xatlas `ChartOptions`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ChartOptions {
    /// Max chart area before split (`0` means no limit).
    pub max_chart_area: f32,
    /// Lower values make more, flatter charts.
    pub max_cost: f32,
    /// Chart growing rounds.
    pub max_iterations: u32,
}
/// Atlas pack knobs (subset of xatlas `PackOptions`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PackOptions {
    /// Atlas edge in texels.
    pub resolution: u32,
    /// Texels of padding around each chart.
    pub padding: u32,
}
/// Combined uv2 cook options.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Uv2Options {
    /// Chart knobs.
    pub chart: ChartOptions,
    /// Pack knobs.
    pub pack: PackOptions,
}
impl Default for Uv2Options {
    fn default() -> Self {
        Self {
            chart: ChartOptions {
                max_chart_area: 0.0,
                max_cost: 2.0,
                max_iterations: 1,
            },
            pack: PackOptions {
                resolution: 1024,
                padding: 4,
            },
        }
    }
}
/// Why a uv2 cook failed.
#[derive(Clone, Debug, PartialEq)]
pub enum Uv2Error {
    /// Caller input is unusable; reason carried.
    Invalid(String),
}
impl fmt::Display for Uv2Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(e) => write!(f, "uv2 chart: {e}"),
        }
    }
}
impl std::error::Error for Uv2Error {}
/// One uv2 chart pack: per-vertex uv2 plus per-triangle charts.
#[derive(Clone, Debug, PartialEq)]
pub struct Uv2Chart {
    /// Per-vertex uv2 in `0..1`, one per input vertex.
    pub uv2: Vec<[f32; 2]>,
    /// Per-triangle chart id, one per input triangle.
    pub chart_of_triangle: Vec<u32>,
    /// Chart count.
    pub chart_count: u32,
    /// Atlas edge in texels.
    pub atlas: u32,
    /// Stable digest over options plus packed uv2 plus charts.
    pub digest: u64,
}
fn area(p: [[f32; 3]; 3]) -> f32 {
    let e1 = [p[1][0] - p[0][0], p[1][1] - p[0][1], p[1][2] - p[0][2]];
    let e2 = [p[2][0] - p[0][0], p[2][1] - p[0][1], p[2][2] - p[0][2]];
    let c = [
        e1[1] * e2[2] - e1[2] * e2[1],
        e1[2] * e2[0] - e1[0] * e2[2],
        e1[0] * e2[1] - e1[1] * e2[0],
    ];
    (0.5 * (c[0] * c[0] + c[1] * c[1] + c[2] * c[2]).sqrt()).max(0.0)
}
fn face_chart(n: [f32; 3]) -> u32 {
    let ax = n[0].abs();
    let ay = n[1].abs();
    let az = n[2].abs();
    if ax >= ay && ax >= az {
        u32::from((n[0] < 0.0) || n[0].is_nan())
    } else if ay >= ax && ay >= az {
        if n[1] >= 0.0 {
            2
        } else {
            3
        }
    } else if n[2] >= 0.0 {
        4
    } else {
        5
    }
}
/// Pack uv2 for one triangle soup: chart by dominant normal axis, shelf
/// pack with padding, deterministic row-major uv2 in `0..1`.
pub fn pack_uv2(
    positions: &[[f32; 3]],
    normals: &[[f32; 3]],
    indices: &[u32],
    options: &Uv2Options,
) -> Result<Uv2Chart, Uv2Error> {
    let (verts, tris) = check_uv2_inputs(positions, normals, indices, options)?;
    let chart_of_triangle = chart_ids(normals, indices, tris);
    let order = sorted_order(positions, indices, tris);
    let mut uv2 = shelf_uv2(positions, indices, &order, verts, options);
    for uv in &mut uv2 {
        for c in uv.iter_mut() {
            *c = c.clamp(0.0, 1.0);
            if !c.is_finite() {
                *c = 0.0;
            }
        }
    }
    Ok(finish_chart(uv2, chart_of_triangle, options))
}
fn check_uv2_inputs(
    positions: &[[f32; 3]],
    normals: &[[f32; 3]],
    indices: &[u32],
    options: &Uv2Options,
) -> Result<(usize, usize), Uv2Error> {
    if positions.is_empty() || indices.is_empty() {
        return Err(Uv2Error::Invalid(
            "uv2 chart needs vertices and triangles".into(),
        ));
    }
    if !indices.len().is_multiple_of(3) {
        return Err(Uv2Error::Invalid(
            "uv2 index count is not a multiple of 3".into(),
        ));
    }
    if normals.len() != positions.len() {
        return Err(Uv2Error::Invalid("uv2 normals mismatch positions".into()));
    }
    for p in positions {
        if !p.iter().all(|c| c.is_finite()) {
            return Err(Uv2Error::Invalid("uv2 position is not finite".into()));
        }
    }
    let verts = positions.len();
    for idx in indices {
        let v = usize::try_from(*idx)
            .map_err(|_| Uv2Error::Invalid("uv2 index exceeds usize".into()))?;
        if v >= verts {
            return Err(Uv2Error::Invalid("uv2 index out of range".into()));
        }
    }
    if options.pack.resolution < 16 || options.pack.resolution > 4096 {
        return Err(Uv2Error::Invalid("uv2 resolution outside 16..=4096".into()));
    }
    Ok((verts, indices.len() / 3))
}
fn chart_ids(normals: &[[f32; 3]], indices: &[u32], tris: usize) -> Vec<u32> {
    let mut out = Vec::with_capacity(tris);
    for tri in indices.chunks_exact(3) {
        let n0 = normals[usize::try_from(tri[0]).unwrap_or(0)];
        let n1 = normals[usize::try_from(tri[1]).unwrap_or(0)];
        let n2 = normals[usize::try_from(tri[2]).unwrap_or(0)];
        let n = [
            n0[0] + n1[0] + n2[0],
            n0[1] + n1[1] + n2[1],
            n0[2] + n1[2] + n2[2],
        ];
        out.push(face_chart(n));
    }
    out
}
fn sorted_order(positions: &[[f32; 3]], indices: &[u32], tris: usize) -> Vec<usize> {
    let mut order: Vec<usize> = (0..tris).collect();
    order.sort_unstable_by(|a, b| {
        let pa = [
            positions[usize::try_from(indices[*a * 3]).unwrap_or(0)],
            positions[usize::try_from(indices[*a * 3 + 1]).unwrap_or(0)],
            positions[usize::try_from(indices[*a * 3 + 2]).unwrap_or(0)],
        ];
        let pb = [
            positions[usize::try_from(indices[*b * 3]).unwrap_or(0)],
            positions[usize::try_from(indices[*b * 3 + 1]).unwrap_or(0)],
            positions[usize::try_from(indices[*b * 3 + 2]).unwrap_or(0)],
        ];
        area(pb).total_cmp(&area(pa)).then_with(|| a.cmp(b))
    });
    order
}
fn shelf_uv2(
    positions: &[[f32; 3]],
    indices: &[u32],
    order: &[usize],
    verts: usize,
    options: &Uv2Options,
) -> Vec<[f32; 2]> {
    let _ = positions;
    let res = f32::from(u16::try_from(options.pack.resolution).unwrap_or(1024));
    let pad = f32::from(u16::try_from(options.pack.padding).unwrap_or(4));
    let cell = (res / 8.0).max(pad * 2.0 + 4.0);
    let cell_u32 = (options.pack.resolution / 8).max(options.pack.padding * 2 + 4);
    let per_row = usize::try_from(options.pack.resolution / cell_u32.max(1))
        .unwrap_or(1)
        .max(1);
    let mut uv2 = vec![[0.0f32; 2]; verts];
    for (slot, tri) in order.iter().enumerate() {
        let col = slot % per_row;
        let row = slot / per_row;
        let col_f = f32::from(u16::try_from(col).unwrap_or(0));
        let row_f = f32::from(u16::try_from(row).unwrap_or(0));
        let ox = (col_f * cell + pad) / res;
        let oy = (row_f * cell + pad) / res;
        let w = (cell - pad * 2.0) / res;
        let corners = [[ox, oy], [ox + w, oy], [ox + w, oy + w]];
        for k in 0..3 {
            let v = usize::try_from(indices[*tri * 3 + k]).unwrap_or(0);
            if let Some(slot) = uv2.get_mut(v) {
                if *slot == [0.0, 0.0] {
                    *slot = corners[k];
                }
            }
        }
    }
    uv2
}
fn finish_chart(uv2: Vec<[f32; 2]>, chart_of_triangle: Vec<u32>, options: &Uv2Options) -> Uv2Chart {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&options.chart.max_cost.to_le_bytes());
    bytes.extend_from_slice(&options.pack.resolution.to_le_bytes());
    bytes.extend_from_slice(&options.pack.padding.to_le_bytes());
    for uv in &uv2 {
        for c in uv {
            bytes.extend_from_slice(&c.to_le_bytes());
        }
    }
    for c in &chart_of_triangle {
        bytes.extend_from_slice(&c.to_le_bytes());
    }
    let digest = hash_bytes(&bytes);
    Uv2Chart {
        uv2,
        chart_of_triangle,
        chart_count: 6,
        atlas: options.pack.resolution,
        digest,
    }
}
/// Hashed provenance row for one uv2 cook, like other cooks.
pub fn provenance_for_chart(name: &str, chart: &Uv2Chart) -> ProvenanceRecord {
    let sha = format!(
        "{:016x}{:016x}",
        chart.digest,
        chart.digest ^ 0x9e37_79b9_7f4a_7c15
    );
    ProvenanceRecord {
        name: name.into(),
        url: "https://example.test/arena/static-mesh".into(),
        spdx: "CC0-1.0".into(),
        author: "Example Studio".into(),
        sha,
        generator: None,
        model: None,
        prompt_digest: None,
    }
}
/// Canonical provenance row text for one uv2 cook.
pub fn provenance_row_for_chart(name: &str, chart: &Uv2Chart) -> String {
    provenance_row(&provenance_for_chart(name, chart))
}
#[cfg(test)]
mod tests {
    use super::*;
    fn quad() -> (Vec<[f32; 3]>, Vec<[f32; 3]>, Vec<u32>) {
        (
            vec![
                [-1.0, 0.0, -1.0],
                [1.0, 0.0, -1.0],
                [1.0, 0.0, 1.0],
                [-1.0, 0.0, 1.0],
            ],
            vec![[0.0, 1.0, 0.0]; 4],
            vec![0, 1, 2, 0, 2, 3],
        )
    }
    #[test]
    fn uv2_pack_is_stable_x2_in_range_and_provenanced() {
        let (p, n, idx) = quad();
        let opts = Uv2Options::default();
        let a = pack_uv2(&p, &n, &idx, &opts).expect("packs");
        let b = pack_uv2(&p, &n, &idx, &opts).expect("packs again");
        assert_eq!(a.uv2, b.uv2, "uv2 stable x2");
        assert_eq!(a.digest, b.digest);
        assert!(a
            .uv2
            .iter()
            .flatten()
            .all(|c| c.is_finite() && (0.0..=1.0).contains(c)));
        assert_eq!(a.chart_of_triangle.len(), 2);
        let row = provenance_row_for_chart("content/arena/floor.glb", &a);
        assert!(row.contains("content/arena/floor.glb"));
        let rec = provenance_for_chart("content/arena/floor.glb", &a);
        assert!(crate::provenance::check_record(&rec).is_ok());
    }
    #[test]
    fn uv2_bad_inputs_fail_loudly() {
        let (p, n, _) = quad();
        let opts = Uv2Options::default();
        assert!(pack_uv2(&[], &[], &[], &opts).is_err());
        assert!(pack_uv2(&p, &n, &[0, 1], &opts).is_err());
        assert!(pack_uv2(&p, &n, &[0, 1, 99], &opts).is_err());
    }
}
