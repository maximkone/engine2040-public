//! The 160 x 120 minimap: the whole field, its markers and the window the
//! camera looks at.
//!
//! The field is 16 x 12 tiles and the camera's view is wider than the field
//! at every zoom the shell offers, so the rectangle on the minimap is not a
//! cut-off map: it is the window the camera looks at, half the field in each
//! direction, centred on the camera's world centre. The page draws it where
//! the camera stands, so a pan moves the rectangle by exactly what the camera
//! moved, and the test in this module proves the two numbers agree.
//!
//! The map is centred on the world origin like the field itself: the minimap
//! pixel of a tile is `(tile + 0.5) * PX_PER_TILE` on both axes, and the world
//! point under a minimap pixel is `pixel / PX_PER_TILE - map / 2`. The page
//! reads those numbers from the one JSON line, so it never keeps a tile size
//! of its own.

use engine_render::kam2_iso::IsoCamera;
use kam2_sim::Sim;

/// Width of the minimap, in pixels.
pub const MINIMAP_W: u16 = 160;
/// Height of the minimap, in pixels.
pub const MINIMAP_H: u16 = 120;
/// Tiles the field is wide.
pub const MAP_W: u16 = 16;
/// Tiles the field is deep.
pub const MAP_H: u16 = 12;
/// Pixels one tile takes on the minimap.
pub const PX_PER_TILE: f32 = 10.0;
/// Tiles of the window the camera looks at, in width: half the field.
pub const LOOK_W: f32 = 8.0;
/// Tiles of the window the camera looks at, in depth: half the field.
pub const LOOK_H: f32 = 6.0;
/// The team colours of the renderer, in sRGB bytes: side 0 and side 1.
pub const SIDE_COLOURS: [[u8; 3]; 2] = [[0x2B, 0x6F, 0xD6], [0xC6, 0x37, 0x2E]];

/// One thing on the minimap: a tile and the side that owns it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Marker {
    /// Its tile.
    pub x: i32,
    /// Its row.
    pub z: i32,
    /// The side that owns it.
    pub side: u8,
}

/// The houses on the field, one marker each.
#[must_use]
pub fn house_markers(sim: &Sim) -> Vec<Marker> {
    sim.houses()
        .iter()
        .map(|house| Marker {
            x: house.x,
            z: house.y,
            side: house.side,
        })
        .collect()
}

/// The units on the field, one marker each.
#[must_use]
pub fn unit_markers(sim: &Sim) -> Vec<Marker> {
    sim.units()
        .iter()
        .map(|unit| Marker {
            x: unit.x,
            z: unit.y,
            side: unit.side,
        })
        .collect()
}

/// The window the camera looks at, in minimap pixels: `[x, y, w, h]`.
///
/// The window is centred on the camera's world centre, so it moves with the
/// camera and never with anything else.
#[must_use]
pub fn look_window(camera: &IsoCamera) -> [f32; 4] {
    let centre = camera.world_center();
    let x = (centre[0] + f32::from(MAP_W) / 2.0 - LOOK_W / 2.0) * PX_PER_TILE;
    let y = (centre[1] + f32::from(MAP_H) / 2.0 - LOOK_H / 2.0) * PX_PER_TILE;
    [x, y, LOOK_W * PX_PER_TILE, LOOK_H * PX_PER_TILE]
}

/// The window clipped to the field, in minimap pixels: `[x, y, w, h]`.
///
/// The canvas clips too, but the page draws the rectangle itself and wants
/// numbers it can trust.
#[must_use]
pub fn field_rect(window: [f32; 4]) -> [f32; 4] {
    let width = f32::from(MINIMAP_W);
    let height = f32::from(MINIMAP_H);
    let x0 = window[0].clamp(0.0, width);
    let y0 = window[1].clamp(0.0, height);
    let x1 = (window[0] + window[2]).clamp(0.0, width);
    let y1 = (window[1] + window[3]).clamp(0.0, height);
    [x0, y0, (x1 - x0).max(0.0), (y1 - y0).max(0.0)]
}

/// The tile the camera looks at: the field's centre tile plus the pan.
#[must_use]
pub fn centre_tile(pan: [f32; 2]) -> [f32; 2] {
    [
        pan[0] + f32::from(MAP_W) / 2.0,
        pan[1] + f32::from(MAP_H) / 2.0,
    ]
}

/// The minimap as one JSON line: the field, its markers and the window.
///
/// The page draws the markers and the rectangle from this one line, so it
/// never has to know a tile size or a team colour of its own.
#[must_use]
pub fn minimap_json(sim: &Sim, camera: &IsoCamera) -> String {
    let houses = markers_json(&house_markers(sim));
    let units = markers_json(&unit_markers(sim));
    let window = look_window(camera);
    let rect = field_rect(window);
    let centre = centre_tile(camera.world_center());
    format!(
        "{{\"w\":{MINIMAP_W},\"h\":{MINIMAP_H},\"tile\":{PX_PER_TILE},\"mapW\":{MAP_W},\"mapH\":{MAP_H},\"sideColours\":[[{},{},{}],[{},{},{}]],\"houses\":[{houses}],\"units\":[{units}],\"look\":[{},{},{},{}],\"rect\":[{},{},{},{}],\"centre\":[{},{}]}}",
        SIDE_COLOURS[0][0],
        SIDE_COLOURS[0][1],
        SIDE_COLOURS[0][2],
        SIDE_COLOURS[1][0],
        SIDE_COLOURS[1][1],
        SIDE_COLOURS[1][2],
        px(window[0]),
        px(window[1]),
        px(window[2]),
        px(window[3]),
        px(rect[0]),
        px(rect[1]),
        px(rect[2]),
        px(rect[3]),
        px(centre[0]),
        px(centre[1]),
    )
}

/// The markers as one JSON list.
fn markers_json(markers: &[Marker]) -> String {
    markers
        .iter()
        .map(|marker| {
            format!(
                "{{\"x\":{},\"z\":{},\"side\":{}}}",
                marker.x, marker.z, marker.side
            )
        })
        .collect::<Vec<_>>()
        .join(",")
}

/// One number of the minimap, with two decimals so the page reads the same
/// number the test does.
fn px(value: f32) -> String {
    format!("{value:.2}")
}

#[cfg(test)]
mod tests {
    use super::{
        centre_tile, field_rect, house_markers, look_window, minimap_json, unit_markers, LOOK_H,
        LOOK_W, MAP_H, MAP_W, MINIMAP_H, MINIMAP_W, PX_PER_TILE,
    };
    use crate::shell::Shell;

    fn shell() -> Shell {
        Shell::new().expect("the start village places")
    }

    #[test]
    fn the_minimap_is_the_whole_field() {
        assert_eq!(MINIMAP_W, 160);
        assert_eq!(MINIMAP_H, 120);
        assert_eq!(MAP_W, 16);
        assert_eq!(MAP_H, 12);
        assert_eq!(PX_PER_TILE, 10.0);
        assert_eq!(f32::from(MINIMAP_W), f32::from(MAP_W) * PX_PER_TILE);
        assert_eq!(f32::from(MINIMAP_H), f32::from(MAP_H) * PX_PER_TILE);
    }

    #[test]
    fn the_window_sits_where_the_camera_looks() {
        let shell = shell();
        let window = look_window(shell.camera());
        let centre = [window[0] + window[2] / 2.0, window[1] + window[3] / 2.0];
        let tile = centre_tile(shell.pan_offset());
        assert!(
            (centre[0] / PX_PER_TILE - tile[0]).abs() < 0.001,
            "the window is centred on the camera: {centre:?} against {tile:?}"
        );
        assert!(
            (centre[1] / PX_PER_TILE - tile[1]).abs() < 0.001,
            "the window is centred on the camera: {centre:?} against {tile:?}"
        );
        assert_eq!(window[2], LOOK_W * PX_PER_TILE);
        assert_eq!(window[3], LOOK_H * PX_PER_TILE);
    }

    #[test]
    fn a_pan_moves_the_window_by_what_the_camera_moved() {
        let mut shell = shell();
        let before = look_window(shell.camera());
        shell.pan(1.0, 0.0);
        let after = look_window(shell.camera());
        assert!(
            (after[0] - before[0] - PX_PER_TILE).abs() < 0.001,
            "one world unit is one tile on the minimap: {before:?} to {after:?}"
        );
        assert!(
            (after[1] - before[1]).abs() < 0.001,
            "a pan east leaves the row alone: {before:?} to {after:?}"
        );
        shell.pan(0.0, 1.0);
        let lower = look_window(shell.camera());
        assert!(
            (lower[1] - after[1] - PX_PER_TILE).abs() < 0.001,
            "one world unit south is one tile down: {after:?} to {lower:?}"
        );
    }

    #[test]
    fn the_window_stays_inside_the_field_it_is_clipped_to() {
        let mut shell = shell();
        for _ in 0..8 {
            shell.pan(2.0, 2.0);
            let window = look_window(shell.camera());
            let rect = field_rect(window);
            assert!(rect[0] >= 0.0 && rect[1] >= 0.0, "{rect:?}");
            assert!(
                rect[0] + rect[2] <= f32::from(MINIMAP_W) + 0.001
                    && rect[1] + rect[3] <= f32::from(MINIMAP_H) + 0.001,
                "{rect:?}"
            );
        }
    }

    #[test]
    fn the_markers_name_every_house_and_unit() {
        let shell = shell();
        let houses = house_markers(shell.game().sim());
        let units = unit_markers(shell.game().sim());
        assert_eq!(houses.len(), shell.houses() as usize);
        assert_eq!(units.len(), shell.units() as usize);
        assert!(houses.iter().all(|marker| marker.side < 2));
        assert!(units.iter().all(|marker| marker.side < 2));
    }

    #[test]
    fn the_json_carries_the_field_the_markers_and_the_window() {
        let shell = shell();
        let json = minimap_json(shell.game().sim(), shell.camera());
        assert!(json.contains("\"w\":160"), "{json}");
        assert!(json.contains("\"h\":120"), "{json}");
        assert!(json.contains("\"mapW\":16"), "{json}");
        assert!(json.contains("\"mapH\":12"), "{json}");
        assert!(
            json.contains("\"sideColours\":[[43,111,214],[198,55,46]]"),
            "{json}"
        );
        assert_eq!(json.matches("\"side\":").count(), 7, "{json}");
        assert!(
            json.contains("\"rect\":[40.00,30.00,80.00,60.00]"),
            "{json}"
        );
        assert!(json.contains("\"centre\":[8.00,6.00]"), "{json}");
    }
}
