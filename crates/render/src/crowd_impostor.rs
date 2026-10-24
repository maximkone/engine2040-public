//! crowd_impostor: octahedral far-row select over the staged crowd.
//!
//! Owner: render lane. Units: meters for distance, cells as counts.
//! Determinism: integer order plus one `sqrt` only in the caller; this
//! select uses `+ - * /` and comparisons, so the same bytes select the
//! same path twice. Errors are typed `CrowdImpostorError`. Budget:
//! O(1) per seat over caller-owned slices, no allocation. Caller:
//! `crowd_inst::CrowdBuilder::build` routes L2 seats here; the next
//! slice binds the quad through `queue.write_buffer`.
//! PORT(wojtekpil/Godot-Octahedral-Impostors@HEAD-verified-2026-10-15:
//! view direction mapped to atlas cell, one quad per far object;
//! reimplemented, zero bytes vendored).

use std::fmt;

/// Far-row failures; shape mirrors [`super::CrowdInstError`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CrowdImpostorError {
    /// One view direction is not finite; seat index carried.
    BadDirection(usize),
    /// Caller buffer too short; sizes carried.
    BadBuffer { want: usize, got: usize },
}

impl fmt::Display for CrowdImpostorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadDirection(i) => write!(f, "bad impostor direction {i}"),
            Self::BadBuffer { want, got } => {
                write!(f, "impostor buffer holds {got}, needs {want}")
            }
        }
    }
}

impl std::error::Error for CrowdImpostorError {}

/// Far-row band in meters: past L1 the body freezes, the quad takes over.
pub(crate) const IMPOSTOR_DIST_M: f32 = 45.0;
/// Atlas edge in cells: 4 by 4 octahedral cells.
pub const ATLAS_EDGE: u8 = 4;

/// True while one seat takes the quad path: past the L1 band or NaN.
/// Discrete LOD chains switch outward with distance [gearus | 11.1.1].
#[must_use]
pub(crate) fn select_impostor(dist_m: f32) -> bool {
    if !dist_m.is_finite() {
        return true;
    }
    dist_m > IMPOSTOR_DIST_M
}

/// Octahedral cell for one view direction: fold to the octahedron,
/// quantize to 4 by 4, pack row-major. O(1), no allocation.
#[must_use]
pub fn octa_cell_for(dir: [f32; 3]) -> u8 {
    let ax = dir[0].abs();
    let ay = dir[1].abs();
    let az = dir[2].abs();
    let sum = ax + ay + az;
    if !sum.is_finite() || sum <= 1e-9 {
        return 0;
    }
    let inv = 1.0 / sum;
    let mut ox = dir[0] * inv;
    let mut oy = dir[1] * inv;
    if dir[2] < 0.0 {
        let px = ox;
        let sx = if ox < 0.0 { -1.0 } else { 1.0 };
        let sy = if oy < 0.0 { -1.0 } else { 1.0 };
        ox = (1.0 - oy.abs()) * sx;
        oy = (1.0 - px.abs()) * sy;
    }
    let qx = if ox < -0.5 {
        0u8
    } else if ox < 0.0 {
        1u8
    } else if ox < 0.5 {
        2u8
    } else {
        3u8
    };
    let qy = if oy < -0.5 {
        0u8
    } else if oy < 0.0 {
        1u8
    } else if oy < 0.5 {
        2u8
    } else {
        3u8
    };
    qy.saturating_mul(ATLAS_EDGE).saturating_add(qx)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn far_select_routes_past_band_only() {
        assert!(!select_impostor(10.0));
        assert!(!select_impostor(45.0));
        assert!(select_impostor(45.01));
        assert!(select_impostor(200.0));
        assert!(select_impostor(f32::NAN));
    }

    #[test]
    fn octa_cells_stay_in_atlas_and_differ() {
        let a = octa_cell_for([0.0, 0.0, 1.0]);
        let b = octa_cell_for([0.0, 0.0, -1.0]);
        let c = octa_cell_for([1.0, 0.0, 0.0]);
        assert!(a < 16 && b < 16 && c < 16);
        assert!(a != b, "opposite views land on opposite cells");
    }
}
