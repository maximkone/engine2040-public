//! stump: the cap mesh that closes a severed limb on the remaining body (RZ3).
//!
//! A cut leaves an open hole where a limb was. `stump_cap` builds the closed
//! stub that fills it: a short cylinder with a top disc, a bottom disc and a
//! wall, sharing its two rings so every edge is used by exactly two triangles
//! (`StumpCap::is_closed`). `place_mesh` moves a cap or a limb body to a
//! world pose (meters, unit quaternion) with `+ - * /` and `sqrt` only, so a
//! presentation frame that bakes a body's mesh into world space stays
//! deterministic on one machine.
//!
//! Consumer: `crates/game/src/limb_look.rs` bakes one cap per cut and one
//! cylinder per falling limb into resident mesh keys. Data owner: this module
//! owns the cap and the placement; the game owns when a cap is drawn. Units:
//! meters, unit quaternions `[x, y, z, w]`, Y up. Determinism: presentation
//! only; nothing here is hashed and no value reaches the fixed tick. Errors:
//! `StumpError` for a radius, depth or segment count outside the table.
//! Budget: cold build of `segments * 4` triangles; no allocation per frame
//! except the mesh the caller asks for.

use std::collections::BTreeMap;

/// Narrowest cap radius, meters (a finger stub).
pub const MIN_RADIUS_M: f32 = 0.005;
/// Widest cap radius, meters (a thigh stub).
pub const MAX_RADIUS_M: f32 = 0.5;
/// Shortest cap depth along its axis, meters.
pub const MIN_DEPTH_M: f32 = 0.005;
/// Longest cap depth along its axis, meters.
pub const MAX_DEPTH_M: f32 = 0.6;
/// Fewest wall segments.
pub const MIN_SEGMENTS: u32 = 3;
/// Most wall segments.
pub const MAX_SEGMENTS: u32 = 64;

/// Why a cap could not be built. On every error nothing was built.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum StumpError {
    /// Radius outside `MIN_RADIUS_M..=MAX_RADIUS_M`, or not a number.
    Radius,
    /// Depth outside `MIN_DEPTH_M..=MAX_DEPTH_M`, or not a number.
    Depth,
    /// Segment count outside `MIN_SEGMENTS..=MAX_SEGMENTS`.
    Segments,
    /// A position or a rotation handed to `place_mesh` was not finite.
    Pose,
}

impl core::fmt::Display for StumpError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Radius => write!(
                f,
                "stump cap radius is outside {MIN_RADIUS_M}..={MAX_RADIUS_M} m"
            ),
            Self::Depth => write!(
                f,
                "stump cap depth is outside {MIN_DEPTH_M}..={MAX_DEPTH_M} m"
            ),
            Self::Segments => write!(
                f,
                "stump cap segments are outside {MIN_SEGMENTS}..={MAX_SEGMENTS}"
            ),
            Self::Pose => write!(f, "stump cap pose is not finite"),
        }
    }
}

impl std::error::Error for StumpError {}

/// One closed cap or limb cylinder: world-space positions, unit normals,
/// `0..=1` uvs and triangle indices, ready for a mesh upload. Plain data.
#[derive(Clone, PartialEq, Debug)]
pub struct StumpCap {
    /// One position per vertex, meters.
    pub positions: Vec<[f32; 3]>,
    /// One unit normal per vertex.
    pub normals: Vec<[f32; 3]>,
    /// One uv per vertex, `0..=1`.
    pub uvs: Vec<[f32; 2]>,
    /// Triangle indices, three per triangle.
    pub indices: Vec<u32>,
}

impl StumpCap {
    /// True when every undirected edge is used by exactly two triangles: the
    /// cap closes the hole instead of leaving an open rim.
    #[must_use]
    pub fn is_closed(&self) -> bool {
        let mut edges: BTreeMap<(u32, u32), u32> = BTreeMap::new();
        for triangle in self.indices.chunks_exact(3) {
            for pair in 0..3 {
                let a = triangle[pair];
                let b = triangle[(pair + 1) % 3];
                if a == b {
                    return false;
                }
                *edges.entry((a.min(b), a.max(b))).or_insert(0) += 1;
            }
        }
        !edges.is_empty() && edges.values().all(|count| *count == 2)
    }

    /// Distance of the farthest vertex from the axis `origin`..`origin + axis`
    /// in meters: how far the cap reaches around its own axis. `None` for a
    /// non-finite mesh.
    #[must_use]
    pub fn reach_m(&self, origin: [f32; 3], axis: [f32; 3]) -> Option<f32> {
        let axis_len = (axis[0] * axis[0] + axis[1] * axis[1] + axis[2] * axis[2]).sqrt();
        if !axis_len.is_finite() || axis_len <= 0.0 {
            return None;
        }
        let unit = [axis[0] / axis_len, axis[1] / axis_len, axis[2] / axis_len];
        let mut farthest: f32 = 0.0;
        for p in &self.positions {
            if !p.iter().all(|v| v.is_finite()) {
                return None;
            }
            let d = [p[0] - origin[0], p[1] - origin[1], p[2] - origin[2]];
            let along = d[0] * unit[0] + d[1] * unit[1] + d[2] * unit[2];
            let off = [
                d[0] - along * unit[0],
                d[1] - along * unit[1],
                d[2] - along * unit[2],
            ];
            farthest = farthest.max((off[0] * off[0] + off[1] * off[1] + off[2] * off[2]).sqrt());
        }
        Some(farthest)
    }
}

/// Build a closed cap along `+Y`: a top disc at `+depth/2`, a bottom disc at
/// `-depth/2` and the wall between them, centered on the origin.
///
/// Errors: [`StumpError::Radius`], [`StumpError::Depth`] or
/// [`StumpError::Segments`] for a value outside its table.
pub fn stump_cap(radius_m: f32, depth_m: f32, segments: u32) -> Result<StumpCap, StumpError> {
    if !radius_m.is_finite() || !(MIN_RADIUS_M..=MAX_RADIUS_M).contains(&radius_m) {
        return Err(StumpError::Radius);
    }
    if !depth_m.is_finite() || !(MIN_DEPTH_M..=MAX_DEPTH_M).contains(&depth_m) {
        return Err(StumpError::Depth);
    }
    if !(MIN_SEGMENTS..=MAX_SEGMENTS).contains(&segments) {
        return Err(StumpError::Segments);
    }
    let n = usize::try_from(segments).unwrap_or_default();
    let half = depth_m * 0.5;
    // The ring normal sits halfway between the disc normal and the wall normal,
    // a unit 45-degree bevel: one vertex per ring keeps the mesh closed.
    let bevel = std::f32::consts::FRAC_1_SQRT_2;
    let step = std::f32::consts::TAU / f32::from(u16::try_from(n).unwrap_or(1));
    let mut positions = Vec::with_capacity(n * 2 + 2);
    let mut normals = Vec::with_capacity(n * 2 + 2);
    let mut uvs = Vec::with_capacity(n * 2 + 2);
    // 0: top center, 1..=n: top ring, n+1: bottom center, n+2..=2n+1: bottom ring.
    positions.push([0.0, half, 0.0]);
    normals.push([0.0, 1.0, 0.0]);
    uvs.push([0.5, 0.5]);
    for i in 0..n {
        let angle = step * f32::from(u16::try_from(i).unwrap_or(0));
        let (s, c) = angle.sin_cos();
        positions.push([radius_m * c, half, radius_m * s]);
        normals.push([bevel * c, bevel, bevel * s]);
        uvs.push([0.5 + 0.5 * c, 0.5 + 0.5 * s]);
    }
    positions.push([0.0, -half, 0.0]);
    normals.push([0.0, -1.0, 0.0]);
    uvs.push([0.5, 0.5]);
    for i in 0..n {
        let angle = step * f32::from(u16::try_from(i).unwrap_or(0));
        let (s, c) = angle.sin_cos();
        positions.push([radius_m * c, -half, radius_m * s]);
        normals.push([bevel * c, -bevel, bevel * s]);
        uvs.push([0.5 + 0.5 * c, 0.5 - 0.5 * s]);
    }
    let ring = u32::try_from(n).unwrap_or(u32::MAX);
    let top = |i: usize| 1 + u32::try_from(i % n).unwrap_or(0);
    let bottom = |i: usize| ring + 2 + u32::try_from(i % n).unwrap_or(0);
    let mut indices = Vec::with_capacity(n * 12);
    for i in 0..n {
        indices.extend_from_slice(&[0, top(i + 1), top(i)]);
        indices.extend_from_slice(&[ring + 1, bottom(i), bottom(i + 1)]);
        indices.extend_from_slice(&[top(i), bottom(i), bottom(i + 1)]);
        indices.extend_from_slice(&[top(i), bottom(i + 1), top(i + 1)]);
    }
    let cap = StumpCap {
        positions,
        normals,
        uvs,
        indices,
    };
    debug_assert!(cap.is_closed(), "the cap builder makes a closed mesh");
    Ok(cap)
}

/// Move `base` to world pose `pos` plus unit `rot` `[x, y, z, w]`, in place in
/// a new mesh: `p * R + pos` for positions and `n * R` for normals. Only
/// `+ - * /` and `sqrt`; no platform trig.
///
/// Errors: [`StumpError::Pose`] for a non-finite position or a rotation whose
/// length is not finite and nonzero.
pub fn place_mesh(base: &StumpCap, pos: [f32; 3], rot: [f32; 4]) -> Result<StumpCap, StumpError> {
    if !pos.iter().all(|v| v.is_finite()) {
        return Err(StumpError::Pose);
    }
    let len = (rot[0] * rot[0] + rot[1] * rot[1] + rot[2] * rot[2] + rot[3] * rot[3]).sqrt();
    if !len.is_finite() || len <= 0.0 {
        return Err(StumpError::Pose);
    }
    let q = [rot[0] / len, rot[1] / len, rot[2] / len, rot[3] / len];
    let rotate = |v: [f32; 3]| -> [f32; 3] {
        // t = 2 * (q.xyz x v); v' = v + q.w * t + q.xyz x t.
        let t = [
            2.0 * (q[1] * v[2] - q[2] * v[1]),
            2.0 * (q[2] * v[0] - q[0] * v[2]),
            2.0 * (q[0] * v[1] - q[1] * v[0]),
        ];
        [
            v[0] + q[3] * t[0] + q[1] * t[2] - q[2] * t[1],
            v[1] + q[3] * t[1] + q[2] * t[0] - q[0] * t[2],
            v[2] + q[3] * t[2] + q[0] * t[1] - q[1] * t[0],
        ]
    };
    let positions = base
        .positions
        .iter()
        .map(|p| {
            let r = rotate(*p);
            [r[0] + pos[0], r[1] + pos[1], r[2] + pos[2]]
        })
        .collect();
    let normals = base.normals.iter().map(|n| rotate(*n)).collect();
    Ok(StumpCap {
        positions,
        normals,
        uvs: base.uvs.clone(),
        indices: base.indices.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stump_cap_is_closed_with_the_asked_radius_and_depth() {
        for (radius, depth, segments) in
            [(0.05f32, 0.08f32, 8u32), (0.11, 0.2, 12), (0.005, 0.005, 3)]
        {
            let cap = stump_cap(radius, depth, segments).expect("in the table");
            assert!(cap.is_closed(), "{segments} segments stays closed");
            let triangles = cap.indices.len() / 3;
            assert_eq!(
                triangles,
                usize::try_from(segments).unwrap() * 4,
                "two discs plus two wall triangles per segment"
            );
            // Every vertex carries a finite unit normal.
            for normal in &cap.normals {
                let len =
                    (normal[0] * normal[0] + normal[1] * normal[1] + normal[2] * normal[2]).sqrt();
                assert!((len - 1.0).abs() < 1e-4, "normal length {len}");
            }
            // The cap reaches its own radius around the axis and no more.
            let reach = cap.reach_m([0.0; 3], [0.0, 1.0, 0.0]).expect("finite");
            assert!((reach - radius).abs() < 1e-5, "reach {reach} vs {radius}");
            let top = cap.positions.iter().map(|p| p[1]).fold(f32::MIN, f32::max);
            let bottom = cap.positions.iter().map(|p| p[1]).fold(f32::MAX, f32::min);
            assert!((top - (depth * 0.5)).abs() < 1e-6, "top {top}");
            assert!((bottom + (depth * 0.5)).abs() < 1e-6, "bottom {bottom}");
        }
    }

    #[test]
    fn stump_cap_refuses_bad_numbers_loudly() {
        assert_eq!(stump_cap(0.0, 0.1, 8), Err(StumpError::Radius));
        assert_eq!(stump_cap(f32::NAN, 0.1, 8), Err(StumpError::Radius));
        assert_eq!(stump_cap(1.0, 0.1, 8), Err(StumpError::Radius));
        assert_eq!(stump_cap(0.05, 0.0, 8), Err(StumpError::Depth));
        assert_eq!(stump_cap(0.05, f32::INFINITY, 8), Err(StumpError::Depth));
        assert_eq!(stump_cap(0.05, 1.0, 8), Err(StumpError::Depth));
        assert_eq!(stump_cap(0.05, 0.1, 2), Err(StumpError::Segments));
        assert_eq!(stump_cap(0.05, 0.1, 65), Err(StumpError::Segments));
        assert!(StumpError::Radius.to_string().contains("radius"));
        assert!(StumpError::Segments.to_string().contains("segments"));
    }

    #[test]
    fn placed_mesh_moves_positions_and_turns_normals_by_the_pose() {
        let cap = stump_cap(0.06, 0.1, 6).expect("in the table");
        // A quarter turn about +X takes the cap's +Y axis onto +Z: the top
        // disc center (0, depth/2, 0) lands at pos + (0, 0, depth/2).
        let (s, c) = (0.5f32.sqrt(), 0.5f32.sqrt());
        let pos = [1.0f32, 2.0, 3.0];
        let placed = place_mesh(&cap, pos, [s, 0.0, 0.0, c]).expect("finite pose");
        assert!(placed.is_closed(), "placement keeps the topology");
        assert_eq!(placed.indices, cap.indices, "indices are untouched");
        let want = [pos[0], pos[1], pos[2] + 0.05];
        assert!(
            placed
                .positions
                .iter()
                .any(|p| (0..3).all(|k| (p[k] - want[k]).abs() < 1e-5)),
            "the axis point turned with the pose"
        );
        for p in &placed.positions {
            assert!(p.iter().all(|v| v.is_finite()));
        }
        // A bad pose refuses and builds nothing.
        assert_eq!(
            place_mesh(&cap, [f32::NAN, 0.0, 0.0], [0.0, 0.0, 0.0, 1.0]),
            Err(StumpError::Pose)
        );
        assert_eq!(place_mesh(&cap, [0.0; 3], [0.0; 4]), Err(StumpError::Pose));
    }

    #[test]
    fn place_mesh_repeats_bit_for_bit() {
        let cap = stump_cap(0.08, 0.12, 10).expect("in the table");
        let a = place_mesh(&cap, [0.3, 1.2, -0.4], [0.1, -0.2, 0.3, 0.928]).expect("pose");
        let b = place_mesh(&cap, [0.3, 1.2, -0.4], [0.1, -0.2, 0.3, 0.928]).expect("pose");
        assert_eq!(a, b);
    }
}
