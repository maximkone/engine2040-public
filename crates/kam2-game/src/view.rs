//! The 16x12 play field in the fixed 2.5D projection: tile to pixel and back.
//!
//! The camera stays in [`engine_render::kam2_iso`]; this module only names where
//! the sim's tiles sit in that projection, so the window, the click mapping and
//! the tests share one answer. World x runs east, world z south; screen y grows
//! down. The map is centered on the world origin, so the whole field fits a
//! 1280x720 window at 100% zoom.

use engine_render::kam2_iso::IsoCamera;

/// Field width in tiles; the sim's first match is 16 wide.
pub const MAP_W: i32 = 16;
/// Field height in tiles; the sim's first match is 12 tall.
pub const MAP_H: i32 = 12;
/// Screen pixels between two tile centers along +x at 100% zoom.
pub const STEP_X_PX: f32 = 32.0;
/// Screen pixels between two tile centers along +y at 100% zoom.
pub const STEP_Y_PX: f32 = 16.0;

/// World center of tile `(x, y)`, in the camera's ground units.
pub fn tile_center_world(x: i32, y: i32) -> [f32; 2] {
    [
        x as f32 + 0.5 - MAP_W as f32 * 0.5,
        y as f32 + 0.5 - MAP_H as f32 * 0.5,
    ]
}

/// Screen pixel of a tile center for this camera.
pub fn tile_screen(camera: &IsoCamera, x: i32, y: i32) -> [f32; 2] {
    let [wx, wz] = tile_center_world(x, y);
    camera.project([wx, 0.0, wz])
}

/// World centre of a `w` by `h` tile footprint with its top-left at `(x, y)`.
/// It is the ground anchor of a building sprite.
pub fn footprint_center_world(x: i32, y: i32, w: i32, h: i32) -> [f32; 2] {
    [
        x as f32 + w as f32 * 0.5 - MAP_W as f32 * 0.5,
        y as f32 + h as f32 * 0.5 - MAP_H as f32 * 0.5,
    ]
}

/// Tile under a screen pixel, or `None` when the pixel is off the field.
///
/// The inverse is the camera's own ground unprojection plus a floor on the map
/// coordinates, so a click rounds to the diamond the player sees.
pub fn screen_to_tile(camera: &IsoCamera, pixel: [f32; 2]) -> Option<(i32, i32)> {
    let [wx, wz] = camera.unproject_ground(pixel);
    if !wx.is_finite() || !wz.is_finite() {
        return None;
    }
    let fx = wx + MAP_W as f32 * 0.5;
    let fy = wz + MAP_H as f32 * 0.5;
    if fx < 0.0 || fy < 0.0 || fx >= MAP_W as f32 || fy >= MAP_H as f32 {
        return None;
    }
    Some((fx.floor() as i32, fy.floor() as i32))
}

/// Screen corners of tile `(x, y)` in draw order: top, right, bottom, left.
pub fn tile_diamond(camera: &IsoCamera, x: i32, y: i32) -> [[f32; 2]; 4] {
    footprint_screen(camera, x, y, 1, 1)
}

/// Screen corners of a `w` by `h` tile footprint with its top-left at `(x, y)`,
/// in draw order: top, right, bottom, left.
pub fn footprint_screen(camera: &IsoCamera, x: i32, y: i32, w: i32, h: i32) -> [[f32; 2]; 4] {
    [
        tile_corner_screen(camera, x, y),
        tile_corner_screen(camera, x + w, y),
        tile_corner_screen(camera, x + w, y + h),
        tile_corner_screen(camera, x, y + h),
    ]
}

/// Screen pixel of one tile-grid corner, not a tile center.
fn tile_corner_screen(camera: &IsoCamera, tx: i32, ty: i32) -> [f32; 2] {
    let wx = tx as f32 - MAP_W as f32 * 0.5;
    let wz = ty as f32 - MAP_H as f32 * 0.5;
    camera.project([wx, 0.0, wz])
}

/// The same four corners lifted `px` pixels up the screen, walls and roofs.
pub fn lift(corners: [[f32; 2]; 4], px: f32) -> [[f32; 2]; 4] {
    let mut out = corners;
    for corner in &mut out {
        corner[1] -= px;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const EPS: f32 = 0.01;

    fn camera() -> IsoCamera {
        IsoCamera::new(1280, 720)
    }

    #[test]
    fn every_tile_center_round_trips_through_the_screen() {
        let camera = camera();
        for ty in 0..MAP_H {
            for tx in 0..MAP_W {
                let pixel = tile_screen(&camera, tx, ty);
                assert_eq!(
                    screen_to_tile(&camera, pixel),
                    Some((tx, ty)),
                    "tile ({tx},{ty}) at {pixel:?}"
                );
            }
        }
    }

    #[test]
    fn a_click_inside_the_diamond_lands_on_its_tile() {
        let camera = camera();
        let center = tile_screen(&camera, 7, 8);
        for offset in [[0.0, 0.0], [15.0, 7.0], [-15.0, -7.0], [20.0, -5.0]] {
            let click = [center[0] + offset[0], center[1] + offset[1]];
            assert_eq!(screen_to_tile(&camera, click), Some((7, 8)), "{offset:?}");
        }
        let east = tile_screen(&camera, 8, 8);
        assert_eq!(screen_to_tile(&camera, east), Some((8, 8)));
        let south = tile_screen(&camera, 7, 9);
        assert_eq!(screen_to_tile(&camera, south), Some((7, 9)));
    }

    #[test]
    fn clicks_off_the_field_are_refused() {
        let camera = camera();
        let corner = tile_screen(&camera, 0, 0);
        assert_eq!(
            screen_to_tile(&camera, [corner[0] - 200.0, corner[1]]),
            None
        );
        assert_eq!(screen_to_tile(&camera, [f32::NAN, 10.0]), None);
        assert_eq!(screen_to_tile(&camera, [10.0, f32::INFINITY]), None);
    }

    #[test]
    fn neighbors_step_thirty_two_by_sixteen_pixels() {
        let camera = camera();
        let origin = tile_screen(&camera, 7, 7);
        let east = tile_screen(&camera, 8, 7);
        let south = tile_screen(&camera, 7, 8);
        assert!((east[0] - origin[0] - STEP_X_PX).abs() < EPS);
        assert!((east[1] - origin[1] - STEP_Y_PX).abs() < EPS);
        assert!((south[0] - origin[0] + STEP_X_PX).abs() < EPS);
        assert!((south[1] - origin[1] - STEP_Y_PX).abs() < EPS);
    }

    #[test]
    fn a_footprint_spans_its_tiles_in_pixels() {
        let camera = camera();
        let corners = footprint_screen(&camera, 0, 3, 3, 3);
        let xs: Vec<f32> = corners.iter().map(|c| c[0]).collect();
        let ys: Vec<f32> = corners.iter().map(|c| c[1]).collect();
        let min_x = xs.iter().copied().fold(f32::INFINITY, f32::min);
        let max_x = xs.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        let min_y = ys.iter().copied().fold(f32::INFINITY, f32::min);
        let max_y = ys.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        assert!(
            (max_x - min_x - 3.0 * 64.0).abs() < EPS,
            "span {min_x} {max_x}"
        );
        assert!(
            (max_y - min_y - 3.0 * 32.0).abs() < EPS,
            "span {min_y} {max_y}"
        );
        assert!(corners[0][1] < corners[3][1]);
        assert!(corners[3][0] < corners[1][0]);
        let lifted = lift(corners, 28.0);
        for (low, high) in corners.iter().zip(lifted) {
            assert!((low[1] - high[1] - 28.0).abs() < EPS);
            assert!((low[0] - high[0]).abs() < EPS);
        }
    }

    #[test]
    fn a_pan_moves_the_field_and_keeps_the_round_trip() {
        let mut camera = camera();
        let before = tile_screen(&camera, 3, 4);
        camera.pan_to([1.0, 2.0]);
        let after = tile_screen(&camera, 3, 4);
        assert!((after[0] - before[0]).abs() > 1.0, "{before:?} {after:?}");
        assert!((after[1] - before[1]).abs() > 1.0, "{before:?} {after:?}");
        assert_eq!(screen_to_tile(&camera, after), Some((3, 4)));
    }
}
