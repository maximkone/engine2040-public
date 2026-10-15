//! crowd_seats: spectator seat layout for the 96-seat stands ring.
//!
//! Owner: render lane; consumer `crate::draw` re-export plus
//! `crates/render/src/present/gpu_present/submit.rs` present mesh.
//! Units: meters for positions, counts for seats. Determinism: integer
//! indices give identical positions; `+ - * /` and `sqrt` only.
//! Errors: none; indices wrap, never fail. Budget: O(1) per seat.

/// Crowd sectors seated from the stands seat rows.
/// Caller: `crates/render/src/present/gpu_present/submit.rs` draws one present mesh.
/// Units: meters for positions, counts for seats. Determinism: integer indices give identical positions.
pub const CROWD_SECTORS: usize = 4;
/// Seat rows per sector, mirrors the three stepped rows in the stands file.
pub const CROWD_ROWS_PER_SECTOR: usize = 3;
/// Seats per row in one sector, eight across the fight view.
pub const CROWD_SEATS_PER_ROW: usize = 8;
/// Palette slots cycled by seat, sampled by texture index.
pub const CROWD_PALETTE_LEN: usize = 4;
/// Inner wall face distance in meters, matches the present ring wall.
pub const CROWD_WALL_M: f32 = 21.0;
/// Pit wall top in meters.
pub const CROWD_WALL_H_M: f32 = 3.0;
/// Tier rise in meters, one step up per row.
pub const CROWD_TIER_RISE_M: f32 = 0.9;
/// Tier run in meters, one step back per row.
pub const CROWD_TIER_RUN_M: f32 = 1.4;
/// Spectator box width in meters.
pub const CROWD_BODY_W_M: f32 = 0.45;
/// Spectator torso box height in meters; the head rides one more cube above.
pub const CROWD_BODY_H_M: f32 = 1.1;
/// Spectator box depth in meters.
pub const CROWD_BODY_D_M: f32 = 0.35;

/// Total spectators seated, four sectors of three rows of eight.
/// Caller: `crates/render/src/present/gpu_present/submit.rs` sizes one present mesh.
pub fn crowd_total() -> usize {
    CROWD_SECTORS * CROWD_ROWS_PER_SECTOR * CROWD_SEATS_PER_ROW
}

/// Feet position for one seat in meters, feet on its tread top.
/// Wraps indices, so any input gives a valid seat, never fails.
/// Caller: `crates/render/src/present/gpu_present/submit.rs` bakes one present mesh.
pub fn crowd_seat_pos(sector: usize, row: usize, seat: usize) -> [f32; 3] {
    let sec = sector % CROWD_SECTORS;
    let r_wrap = row % CROWD_ROWS_PER_SECTOR;
    let s_wrap = seat % CROWD_SEATS_PER_ROW;
    let r_u16 = u16::try_from(r_wrap).unwrap_or(0);
    let s_u16 = u16::try_from(s_wrap).unwrap_or(0);
    let r_f = f32::from(r_u16);
    let s_f = f32::from(s_u16);
    let depth = CROWD_WALL_M + CROWD_TIER_RUN_M * (r_f + 1.0) + CROWD_TIER_RUN_M * 0.5;
    let height = CROWD_WALL_H_M + CROWD_TIER_RISE_M * (r_f + 1.0);
    let lateral = (s_f - 3.5) * 1.0;
    if sec == 0 {
        [2.0 + lateral, height, -depth]
    } else if sec == 1 {
        [2.0 + lateral, height, depth]
    } else if sec == 2 {
        [-depth, height, lateral]
    } else {
        [depth, height, lateral]
    }
}

/// Palette slot for one seat, zero to three, cycled by sector plus row plus seat.
/// Variety comes from the texture index, never a per-seat material.
/// Caller: `crates/render/src/present/gpu_present/submit.rs` writes palette uvs.
pub fn crowd_palette_index(sector: usize, row: usize, seat: usize) -> usize {
    (sector + row * 2 + seat) % CROWD_PALETTE_LEN
}

/// Axis bounds for one sector in meters, min then max, covering its three rows.
/// Sector-level proof uses this box, never per-seat tests per frame.
/// Caller: `crates/render/src/present/gpu_present/submit.rs` validates seats sit inside it.
pub fn crowd_sector_bounds(sector: usize) -> [[f32; 3]; 2] {
    let a = crowd_seat_pos(sector, 0, 0);
    let b = crowd_seat_pos(sector, 0, CROWD_SEATS_PER_ROW - 1);
    let c = crowd_seat_pos(sector, CROWD_ROWS_PER_SECTOR - 1, 0);
    let d = crowd_seat_pos(sector, CROWD_ROWS_PER_SECTOR - 1, CROWD_SEATS_PER_ROW - 1);
    let min_x = a[0].min(b[0]).min(c[0]).min(d[0]) - CROWD_BODY_W_M * 0.5;
    let max_x = a[0].max(b[0]).max(c[0]).max(d[0]) + CROWD_BODY_W_M * 0.5;
    let min_y = a[1].min(b[1]).min(c[1]).min(d[1]);
    let max_y = a[1].max(b[1]).max(c[1]).max(d[1]) + CROWD_BODY_H_M;
    let min_z = a[2].min(b[2]).min(c[2]).min(d[2]) - CROWD_BODY_D_M * 0.5;
    let max_z = a[2].max(b[2]).max(c[2]).max(d[2]) + CROWD_BODY_D_M * 0.5;
    [[min_x, min_y, min_z], [max_x, max_y, max_z]]
}

/// True when one sector's bounds survive the six packed planes.
/// One sphere test per sector, never per-seat culls: the staged seat-sector
/// fixture plus the submit bin's cold-path bounds check run it.
pub fn crowd_sector_visible(sector: usize, planes: &[[f32; 4]; 6]) -> bool {
    let bounds = crowd_sector_bounds(sector);
    let center = [
        (bounds[0][0] + bounds[1][0]) * 0.5,
        (bounds[0][1] + bounds[1][1]) * 0.5,
        (bounds[0][2] + bounds[1][2]) * 0.5,
    ];
    let dx = bounds[1][0] - bounds[0][0];
    let dy = bounds[1][1] - bounds[0][1];
    let dz = bounds[1][2] - bounds[0][2];
    let radius = (dx * dx + dy * dy + dz * dz).sqrt() * 0.5;
    super::sphere_visible_cpu(center, radius, planes)
}

/// Spectator head edge in meters: one head cube rides each body cube.
pub const CROWD_HEAD_M: f32 = 0.25;

/// Palette slot colors as sRGB bytes: tunic variety sampled by texture index.
/// Caller: `crates/render/src/present/gpu_present/submit.rs` bakes the 4x1 palette.
pub fn crowd_palette_rgba(index: usize) -> [u8; 4] {
    const PALETTE: [[u8; 4]; 4] = [
        [154, 74, 56, 255],
        [62, 92, 140, 255],
        [176, 150, 104, 255],
        [58, 58, 64, 255],
    ];
    PALETTE[index % CROWD_PALETTE_LEN]
}

#[cfg(test)]
mod tests {
    use super::super::Camera;
    use super::super::{frustum_planes, planes_to_packed, validate_camera};
    use super::*;

    fn test_camera() -> Camera {
        Camera {
            pos_m: [0.0, 0.0, 0.0],
            forward: [0.0, 0.0, -1.0],
            up: [0.0, 1.0, 0.0],
            fov_y_rad: std::f32::consts::FRAC_PI_2,
            aspect: 16.0 / 9.0,
            near_m: 0.1,
            far_m: 100.0,
        }
    }

    #[test]
    fn crowd_total_is_four_sectors_of_three_rows_of_eight() {
        assert_eq!(CROWD_SECTORS, 4);
        assert_eq!(CROWD_ROWS_PER_SECTOR, 3);
        assert_eq!(CROWD_SEATS_PER_ROW, 8);
        assert_eq!(crowd_total(), 96);
    }

    #[test]
    fn crowd_seats_sit_on_treads_and_differ() {
        let north_low = crowd_seat_pos(0, 0, 0);
        let north_high = crowd_seat_pos(0, 2, 7);
        assert!(
            (north_low[1] - 3.9).abs() < 1e-4,
            "row 0 tread top {north_low:?}"
        );
        assert!(
            (north_high[1] - 5.7).abs() < 1e-4,
            "row 2 tread top {north_high:?}"
        );
        assert_ne!(north_low, north_high, "seats differ");
        assert_ne!(
            crowd_seat_pos(0, 0, 0),
            crowd_seat_pos(1, 0, 0),
            "sectors differ"
        );
        assert_eq!(
            crowd_seat_pos(0, 0, 0),
            crowd_seat_pos(0, 0, 0),
            "same indices give same bytes x2"
        );
    }

    #[test]
    fn crowd_palette_cycles_four_by_texture_index() {
        let mut seen = [false; 4];
        for seat in 0..8 {
            let index = crowd_palette_index(0, 0, seat);
            assert!(index < CROWD_PALETTE_LEN, "index in range");
            seen[index] = true;
        }
        assert!(seen.iter().all(|v| *v), "all four palette slots appear");
        assert_ne!(
            crowd_palette_index(0, 0, 0),
            crowd_palette_index(0, 0, 1),
            "neighbours differ"
        );
    }

    #[test]
    fn crowd_sectors_cull_by_sector_not_per_seat() {
        let camera = test_camera();
        let basis = validate_camera(&camera).expect("test camera valid");
        let packed = planes_to_packed(&frustum_planes(&camera, &basis));
        assert!(
            crowd_sector_visible(0, &packed) || crowd_sector_visible(1, &packed),
            "some sector stays visible ahead"
        );
        let bounds = crowd_sector_bounds(0);
        assert!(bounds[0][0] < bounds[1][0], "bounds order x");
        assert!(bounds[0][1] < bounds[1][1], "bounds order y");
        assert!(bounds[0][2] < bounds[1][2], "bounds order z");
        assert_eq!(
            crowd_sector_bounds(0),
            crowd_sector_bounds(0),
            "bounds x2 equal"
        );
    }

    #[test]
    fn crowd_stands_has_seat_rows() {
        let text = include_str!("../../../content/arena/src/stands.gltf");
        assert!(
            text.contains("seat_rows"),
            "stands file names its seat rows"
        );
        assert!(
            text.contains("crowd_sectors"),
            "stands file names its sectors"
        );
    }

    #[test]
    fn crowd_palette_holds_four_opaque_slots() {
        let slots = [
            crowd_palette_rgba(0),
            crowd_palette_rgba(1),
            crowd_palette_rgba(2),
            crowd_palette_rgba(3),
        ];
        assert!(slots.iter().all(|s| s[3] == 255), "all slots opaque");
        assert_ne!(slots[0], slots[1], "neighbours differ");
        assert_ne!(slots[1], slots[2], "neighbours differ");
        assert_ne!(slots[2], slots[3], "neighbours differ");
        assert_eq!(crowd_palette_rgba(4), slots[0], "index wraps");
        assert!(CROWD_HEAD_M > 0.0, "heads have size");
    }
}
