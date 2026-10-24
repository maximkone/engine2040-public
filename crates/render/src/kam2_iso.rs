//! Fixed 2.5D projection for the ground field.
//!
//! One view direction forever: yaw 45 degrees, pitch atan(0.5). A square
//! world tile is `TILE_WORLD` wide and covers 64 x 32 screen pixels at 100%
//! zoom. Screen pixels start at the viewport's top-left and y grows down.
//!
//! The locked pitch foreshortens a square tile by `sin(pitch)`, not 0.5, so
//! the flat projection carries one constant vertical scale on top of the pitch
//! basis: the style lock fixes the pixel aspect at 2:1. That scale is fixed;
//! the camera never rotates and zoom only changes world units per pixel.

use std::f32::consts::{FRAC_PI_4, SQRT_2};
use std::fmt;

/// Ground grid width in tiles.
pub const GRID_TILES: u16 = 16;
/// Ground tiles drawn in one field.
pub const TILE_COUNT: u32 = 256;
/// Tile width in screen pixels at 100% zoom.
pub const TILE_PX_W: u32 = 64;
/// Tile height in screen pixels at 100% zoom.
pub const TILE_PX_H: u32 = 32;
/// Tile edge in world units; the grid is square.
pub const TILE_WORLD: f32 = 1.0;
/// The only zoom levels the style lock allows.
pub const ZOOM_PERCENTS: [u32; 3] = [100, 125, 150];
/// World units per screen pixel at 100% zoom.
///
/// The tile diagonal is `sqrt(2)` world units and 64 pixels wide:
/// `sqrt(2) / 64 = 1 / (32 * sqrt(2))`.
pub const WORLD_PER_PX_100: f32 = TILE_WORLD / (32.0 * SQRT_2);

/// Locked camera pitch in radians: `atan(0.5)`.
pub fn pitch_rad() -> f32 {
    0.5_f32.atan()
}

/// Locked camera yaw in radians: 45 degrees.
pub fn yaw_rad() -> f32 {
    FRAC_PI_4
}

/// Fixed screen-y scale that turns the pitch foreshortening into the locked
/// 2:1 diamond. `0.5 / sin(pitch)`, never changed by pan or zoom.
pub fn vertical_stretch() -> f32 {
    0.5 / pitch_rad().sin()
}

/// One fixed camera basis. World axes: x east, y up, z south.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct IsoBasis {
    /// Screen right in world space, unit length.
    pub right: [f32; 3],
    /// Screen up in world space, unit length.
    pub up: [f32; 3],
    /// Camera view direction in world space, unit length; y is negative.
    pub forward: [f32; 3],
}

/// The one view direction of the game: yaw 45 degrees, pitch `atan(0.5)`.
pub fn fixed_basis() -> IsoBasis {
    let sin_pitch = pitch_rad().sin();
    let cos_pitch = pitch_rad().cos();
    let axis = yaw_rad().sin();
    IsoBasis {
        right: [axis, 0.0, -axis],
        up: [-axis * sin_pitch, cos_pitch, -axis * sin_pitch],
        forward: [-axis * cos_pitch, -sin_pitch, -axis * cos_pitch],
    }
}

/// The requested zoom percent is not one of [`ZOOM_PERCENTS`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ZoomError {
    /// The refused percent.
    pub requested: u32,
}

impl fmt::Display for ZoomError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "zoom {} is not one of {:?}",
            self.requested, ZOOM_PERCENTS
        )
    }
}

impl std::error::Error for ZoomError {}

/// Orthographic ground camera with a fixed direction.
///
/// The camera looks at `world_center` on the ground plane and never rotates.
/// Only pan, resize and the three locked zoom levels can change what it shows.
#[derive(Clone, Copy, Debug)]
pub struct IsoCamera {
    screen_center: [f32; 2],
    world_center: [f32; 2],
    world_per_px: f32,
    zoom_percent: u32,
    basis: IsoBasis,
    stretch: f32,
}

impl IsoCamera {
    /// Camera centered on world origin for a viewport of `width` x `height`
    /// physical pixels at 100% zoom.
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            screen_center: [width as f32 * 0.5, height as f32 * 0.5],
            world_center: [0.0, 0.0],
            world_per_px: WORLD_PER_PX_100,
            zoom_percent: 100,
            basis: fixed_basis(),
            stretch: vertical_stretch(),
        }
    }

    /// Keep the world center and zoom, move to a new viewport size.
    pub fn resize(&mut self, width: u32, height: u32) {
        self.screen_center = [width as f32 * 0.5, height as f32 * 0.5];
    }

    /// Pan the view to a ground point; the view direction stays fixed.
    pub fn pan_to(&mut self, ground: [f32; 2]) {
        self.world_center = ground;
    }

    /// Ground point at the viewport center.
    pub fn world_center(&self) -> [f32; 2] {
        self.world_center
    }

    /// Viewport size in physical pixels.
    pub fn viewport(&self) -> [f32; 2] {
        [self.screen_center[0] * 2.0, self.screen_center[1] * 2.0]
    }

    /// World units per screen pixel at the current zoom.
    pub fn world_per_px(&self) -> f32 {
        self.world_per_px
    }

    /// Current zoom percent; always one of [`ZOOM_PERCENTS`].
    pub fn zoom_percent(&self) -> u32 {
        self.zoom_percent
    }

    /// Set one of the three locked zoom levels. Any other percent is refused
    /// and leaves the camera unchanged.
    pub fn set_zoom_percent(&mut self, percent: u32) -> Result<(), ZoomError> {
        let scale = match percent {
            100 => 1.0,
            125 => 1.25,
            150 => 1.5,
            requested => return Err(ZoomError { requested }),
        };
        self.world_per_px = WORLD_PER_PX_100 / scale;
        self.zoom_percent = percent;
        Ok(())
    }

    /// Fixed view basis.
    pub fn basis(&self) -> IsoBasis {
        self.basis
    }

    /// Fixed 2:1 screen-y scale.
    pub fn vertical_stretch(&self) -> f32 {
        self.stretch
    }

    /// Project a world point to a screen pixel (top-left origin, y down).
    pub fn project(&self, world: [f32; 3]) -> [f32; 2] {
        let rel = [
            world[0] - self.world_center[0],
            world[1],
            world[2] - self.world_center[1],
        ];
        let x = dot3(rel, self.basis.right) / self.world_per_px;
        let y = -dot3(rel, self.basis.up) * self.stretch / self.world_per_px;
        [self.screen_center[0] + x, self.screen_center[1] + y]
    }

    /// Invert [`IsoCamera::project`] for the ground plane (world y = 0).
    pub fn unproject_ground(&self, pixel: [f32; 2]) -> [f32; 2] {
        let x = (pixel[0] - self.screen_center[0]) * self.world_per_px;
        let y = -(pixel[1] - self.screen_center[1]) * self.world_per_px / self.stretch;
        let rel = [
            self.basis.right[0] * x + self.basis.up[0] * y,
            self.basis.up[1] * y,
            self.basis.right[2] * x + self.basis.up[2] * y,
        ];
        let sin_pitch = -self.basis.forward[1];
        let t = rel[1] / sin_pitch;
        [
            self.world_center[0] + rel[0] + self.basis.forward[0] * t,
            self.world_center[1] + rel[2] + self.basis.forward[2] * t,
        ]
    }

    /// World center of tile `(tx, ty)`; the field spans `-8..8` world units.
    pub fn tile_center(tx: u16, ty: u16) -> [f32; 3] {
        let half = f32::from(GRID_TILES) * 0.5;
        [
            (f32::from(tx) + 0.5 - half) * TILE_WORLD,
            0.0,
            (f32::from(ty) + 0.5 - half) * TILE_WORLD,
        ]
    }

    /// Four ground corners of tile `(tx, ty)` in screen order: top, right,
    /// bottom, left.
    pub fn tile_corners(tx: u16, ty: u16) -> [[f32; 3]; 4] {
        let center = Self::tile_center(tx, ty);
        let half = TILE_WORLD * 0.5;
        [
            [center[0] - half, 0.0, center[2] - half],
            [center[0] + half, 0.0, center[2] - half],
            [center[0] + half, 0.0, center[2] + half],
            [center[0] - half, 0.0, center[2] + half],
        ]
    }

    /// Screen pixels of the four tile corners, top, right, bottom, left.
    pub fn tile_screen_corners(&self, tx: u16, ty: u16) -> [[f32; 2]; 4] {
        let corners = Self::tile_corners(tx, ty);
        [
            self.project(corners[0]),
            self.project(corners[1]),
            self.project(corners[2]),
            self.project(corners[3]),
        ]
    }
}

fn dot3(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

#[cfg(test)]
mod tests {
    use super::*;

    const EPS: f32 = 1e-3;

    #[test]
    fn pitch_is_atan_half_and_yaw_is_45_degrees() {
        assert!((pitch_rad() - 0.5_f32.atan()).abs() < 1e-7);
        assert!((pitch_rad().tan() - 0.5).abs() < 1e-6);
        assert_eq!(yaw_rad(), FRAC_PI_4);
        assert_eq!(TILE_COUNT, u32::from(GRID_TILES) * u32::from(GRID_TILES));
    }

    #[test]
    fn basis_is_orthonormal_and_fixed() {
        let basis = fixed_basis();
        for axis in [basis.right, basis.up, basis.forward] {
            assert!((dot3(axis, axis) - 1.0).abs() < 1e-6);
        }
        assert!(dot3(basis.right, basis.up).abs() < 1e-6);
        assert!(dot3(basis.right, basis.forward).abs() < 1e-6);
        assert!(dot3(basis.up, basis.forward).abs() < 1e-6);
        assert!(basis.forward[1] < 0.0);
        assert!(basis.up[1] > 0.0);
    }

    #[test]
    fn tile_centers_round_trip_through_screen_pixels() {
        let camera = IsoCamera::new(1280, 720);
        for ty in 0..GRID_TILES {
            for tx in 0..GRID_TILES {
                let center = IsoCamera::tile_center(tx, ty);
                let pixel = camera.project(center);
                let back = camera.unproject_ground(pixel);
                assert!((back[0] - center[0]).abs() < EPS, "tx={tx} ty={ty}");
                assert!((back[1] - center[2]).abs() < EPS, "tx={tx} ty={ty}");
            }
        }
    }

    #[test]
    fn world_origin_sits_at_the_viewport_center() {
        let camera = IsoCamera::new(1280, 720);
        assert_eq!(camera.project([0.0, 0.0, 0.0]), [640.0, 360.0]);
    }

    #[test]
    fn diamond_is_64_by_32_pixels_at_zoom_1() {
        let camera = IsoCamera::new(1280, 720);
        let center = camera.project(IsoCamera::tile_center(7, 7));
        let corners = camera.tile_screen_corners(7, 7);
        let offsets = [[0.0, -16.0], [32.0, 0.0], [0.0, 16.0], [-32.0, 0.0]];
        for (corner, offset) in corners.iter().zip(offsets) {
            assert!((corner[0] - (center[0] + offset[0])).abs() < EPS);
            assert!((corner[1] - (center[1] + offset[1])).abs() < EPS);
        }
        let width = corners[1][0] - corners[3][0];
        let height = corners[2][1] - corners[0][1];
        assert_eq!((TILE_PX_W, TILE_PX_H), (64, 32));
        assert!((width - 64.0).abs() < EPS, "width {width}");
        assert!((height - 32.0).abs() < EPS, "height {height}");
        assert!((width / height - 2.0).abs() < EPS);
    }

    #[test]
    fn neighboring_tiles_step_32_by_16_pixels() {
        let camera = IsoCamera::new(1280, 720);
        let origin = camera.project(IsoCamera::tile_center(7, 7));
        let east = camera.project(IsoCamera::tile_center(8, 7));
        let south = camera.project(IsoCamera::tile_center(7, 8));
        assert!((east[0] - origin[0] - 32.0).abs() < EPS);
        assert!((east[1] - origin[1] - 16.0).abs() < EPS);
        assert!((south[0] - origin[0] + 32.0).abs() < EPS);
        assert!((south[1] - origin[1] - 16.0).abs() < EPS);
    }

    #[test]
    fn camera_never_rotates() {
        let reference = fixed_basis();
        let mut camera = IsoCamera::new(800, 600);
        assert_eq!(camera.basis(), reference);
        camera.pan_to([12.0, -9.0]);
        camera.resize(1920, 1080);
        camera.set_zoom_percent(150).expect("150 is a locked zoom");
        assert_eq!(camera.basis(), reference);
        assert_eq!(camera.zoom_percent(), 150);
        assert_eq!(
            camera.set_zoom_percent(133),
            Err(ZoomError { requested: 133 })
        );
        assert_eq!(camera.zoom_percent(), 150);
        assert_eq!(camera.basis(), reference);
        assert_eq!(camera.vertical_stretch(), vertical_stretch());
        let center = IsoCamera::tile_center(3, 11);
        camera.pan_to([center[0], center[2]]);
        assert!((camera.project(center)[0] - 960.0).abs() < EPS);
        assert!((camera.project(center)[1] - 540.0).abs() < EPS);
        assert_eq!(camera.basis(), reference);
    }

    #[test]
    fn zoom_steps_keep_the_diamond_ratio() {
        for percent in ZOOM_PERCENTS {
            let mut camera = IsoCamera::new(1280, 720);
            camera
                .set_zoom_percent(percent)
                .expect("listed zoom is allowed");
            let corners = camera.tile_screen_corners(7, 7);
            let width = corners[1][0] - corners[3][0];
            let height = corners[2][1] - corners[0][1];
            let scale = percent as f32 / 100.0;
            assert!((width - 64.0 * scale).abs() < EPS, "width at {percent}%");
            assert!((height - 32.0 * scale).abs() < EPS, "height at {percent}%");
            assert!((width / height - 2.0).abs() < EPS);
        }
    }
}
