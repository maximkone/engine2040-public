//! Procedural stand-in sprite atlas for the KAM1b proof.
//!
//! Four rows of eight cells: infantry, tank hull, tank turret and a target
//! marker. Bytes are sRGB; the team mask keeps coverage in alpha and is
//! sampled by the proof's tint pass. Art is drawn here, in code, so the
//! proof needs no content files and every cell is reproducible.
//!
//! The cells are only a stand-in for the Blender factory of the style lock:
//! same sizes, same facing order, same mask rule, no final pixels.

use crate::kam2_sprites::{Facing, SpriteAtlas};

/// Cell width in pixels: a 46 px hull plus one pixel of air per side.
pub const CELL_W: u32 = 48;
/// Cell height in pixels: room for the tallest stand-in and its shadow.
pub const CELL_H: u32 = 40;
/// Rows in the atlas.
pub const ROWS: u32 = 4;
/// Infantry row.
pub const ROW_INFANTRY: u32 = 0;
/// Tank hull row.
pub const ROW_HULL: u32 = 1;
/// Tank turret row, drawn as its own layer.
pub const ROW_TURRET: u32 = 2;
/// Target marker row, drawn on the ground.
pub const ROW_MARKER: u32 = 3;

const STEEL: [u8; 4] = [0x9A, 0xA3, 0xAB, 255];
const STEEL_DARK: [u8; 4] = [0x6E, 0x76, 0x7E, 255];
const SLATE: [u8; 4] = [0x3E, 0x4A, 0x57, 255];
const WOOD: [u8; 4] = [0x8B, 0x5A, 0x2B, 255];
const WOOD_DARK: [u8; 4] = [0x6E, 0x48, 0x22, 255];
const SAND: [u8; 4] = [0xD9, 0xBE, 0x86, 255];
const PLASTER: [u8; 4] = [0xD8, 0xCD, 0xB4, 255];
const GOLD: [u8; 4] = [0xD9, 0xA4, 0x41, 255];
/// Baked contact shadow: 35% black, as the style lock asks.
const SHADOW: [u8; 4] = [0x1B, 0x1F, 0x24, 90];
const MASK_SET: [u8; 4] = [255, 255, 255, 255];

/// One generated atlas: base colour bytes, mask bytes and the cell grid.
#[derive(Clone, Debug)]
pub struct SpriteArt {
    width: u32,
    height: u32,
    base: Vec<u8>,
    mask: Vec<u8>,
    layout: SpriteAtlas,
}

impl SpriteArt {
    /// Generate every cell of the stand-in atlas.
    pub fn build() -> Self {
        let layout = SpriteAtlas {
            cell_w: CELL_W,
            cell_h: CELL_H,
            rows: ROWS,
        };
        let width = layout.width();
        let height = layout.height();
        let stride = usize::try_from(width * 4).expect("atlas width fits usize");
        let mut art = Self {
            width,
            height,
            base: vec![0u8; stride * usize::try_from(height).expect("atlas height fits usize")],
            mask: vec![0u8; stride * usize::try_from(height).expect("atlas height fits usize")],
            layout,
        };
        for facing in Facing::ALL {
            let column = u32::from(facing.index());
            art.draw_cell(column, ROW_INFANTRY, &|cell| draw_infantry(cell, facing));
            art.draw_cell(column, ROW_HULL, &|cell| draw_hull(cell, facing));
            art.draw_cell(column, ROW_TURRET, &|cell| draw_turret(cell, facing));
            art.draw_cell(column, ROW_MARKER, &draw_marker);
        }
        art
    }

    /// Atlas width in pixels.
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Atlas height in pixels.
    pub fn height(&self) -> u32 {
        self.height
    }

    /// The cell grid this art was drawn for.
    pub fn layout(&self) -> SpriteAtlas {
        self.layout
    }

    /// Raw RGBA bytes, top row first.
    pub fn base_bytes(&self) -> &[u8] {
        &self.base
    }

    /// Raw RGBA mask bytes, top row first; alpha carries team coverage.
    pub fn mask_bytes(&self) -> &[u8] {
        &self.mask
    }

    /// One base texel as RGBA; out-of-range reads are transparent.
    pub fn base_rgba(&self, at: [u32; 2]) -> [u8; 4] {
        if at[0] >= self.width || at[1] >= self.height {
            return [0; 4];
        }
        let index = ((at[1] * self.width + at[0]) * 4) as usize;
        [
            self.base[index],
            self.base[index + 1],
            self.base[index + 2],
            self.base[index + 3],
        ]
    }

    /// Mask alpha at one texel: 0 outside a team band, 255 inside it.
    pub fn mask_alpha(&self, at: [u32; 2]) -> u8 {
        if at[0] >= self.width || at[1] >= self.height {
            return 0;
        }
        self.mask[((at[1] * self.width + at[0]) * 4 + 3) as usize]
    }

    fn draw_cell(&mut self, column: u32, row: u32, paint: &dyn Fn(&mut Cell<'_>)) {
        let mut cell = Cell {
            base: &mut self.base,
            mask: &mut self.mask,
            width: self.width,
            ox: column * CELL_W,
            oy: row * CELL_H,
        };
        paint(&mut cell);
    }
}

/// Absolute atlas pixel of the team band centre on an infantry chest.
pub fn infantry_team_pixel(facing: Facing) -> [u32; 2] {
    [
        u32::from(facing.index()) * CELL_W + infantry_chest(facing)[0],
        ROW_INFANTRY * CELL_H + infantry_chest(facing)[1],
    ]
}

/// Absolute atlas pixel of the team stripe on a tank hull.
pub fn hull_team_pixel(facing: Facing) -> [u32; 2] {
    [
        u32::from(facing.index()) * CELL_W + 12,
        ROW_HULL * CELL_H + 18,
    ]
}

/// Absolute atlas pixel of the team band on a turret.
pub fn turret_team_pixel(facing: Facing) -> [u32; 2] {
    [
        u32::from(facing.index()) * CELL_W + 21,
        ROW_TURRET * CELL_H + 16,
    ]
}

/// Absolute atlas pixel at the end of a turret barrel.
pub fn turret_barrel_tip(facing: Facing) -> [u32; 2] {
    let tip = barrel_tip_local(facing);
    [
        u32::from(facing.index()) * CELL_W + tip[0],
        ROW_TURRET * CELL_H + tip[1],
    ]
}

/// Absolute atlas pixel of the target marker centre.
pub fn marker_centre_pixel() -> [u32; 2] {
    [24, ROW_MARKER * CELL_H + 34]
}

fn infantry_chest(facing: Facing) -> [u32; 2] {
    [(24 + infantry_lean(facing)) as u32, 18]
}

fn infantry_lean(facing: Facing) -> i32 {
    [0, -1, -2, -1, 0, 1, 2, 1][usize::from(facing.index())]
}

fn barrel_tip_local(facing: Facing) -> [u32; 2] {
    let [dx, dy] = screen_direction(facing);
    let centre = [24.0_f32, 19.0_f32];
    let x = (centre[0] + dx * 17.0).round();
    let y = (centre[1] + dy * 17.0).round();
    [x as u32, y as u32]
}

/// Screen-space direction of a ground facing: x right, y down.
///
/// This is the KAM1a projection of a unit ground vector with y = 0, so a
/// drawn barrel and the chosen facing cell agree on screen.
fn screen_direction(facing: Facing) -> [f32; 2] {
    let [dx, dz] = facing.direction();
    let x = dx - dz;
    let y = 0.5 * (dx + dz);
    let length = (x * x + y * y).sqrt();
    [x / length, y / length]
}

fn draw_infantry(cell: &mut Cell<'_>, facing: Facing) {
    let lean = infantry_lean(facing);
    let cx = 24 + lean;
    let bottom = 36;
    let stride = [0, 1, 2, 1, 0, 1, 2, 1][usize::from(facing.index())];
    cell.ellipse(24, 37, 9, 2, SHADOW);
    cell.rect(cx - 6, bottom - 9 - stride, 4, 9 + stride, WOOD);
    cell.rect(cx + 2, bottom - 9, 4, 9, WOOD);
    cell.rect(cx - 6, bottom - 24, 12, 15, WOOD_DARK);
    cell.team(cx - 6, bottom - 20, 12, 4);
    let pack = matches!(facing, Facing::NW | Facing::N | Facing::NE);
    if pack {
        cell.rect(cx - 4, bottom - 23, 8, 7, PLASTER);
    }
    cell.disc(cx, bottom - 29, 4, SAND);
    cell.rect(cx - 5, bottom - 34, 10, 4, STEEL);
    cell.team(cx - 5, bottom - 30, 10, 2);
    cell.rect(cx + 5, bottom - 23, 2, 8, STEEL_DARK);
}

fn draw_hull(cell: &mut Cell<'_>, facing: Facing) {
    cell.ellipse(24, 37, 20, 3, SHADOW);
    cell.rect(5, 4, 38, 4, STEEL_DARK);
    cell.rect(5, 8, 38, 22, STEEL);
    cell.team(9, 16, 14, 4);
    cell.rect(1, 28, 46, 4, SLATE);
    cell.rect(1, 32, 46, 4, SLATE);
    for pad in 0..4 {
        let x = 5 + pad * 11;
        cell.rect(x, 29, 5, 2, STEEL_DARK);
        cell.rect(x, 33, 5, 2, STEEL_DARK);
    }
    if matches!(facing, Facing::N | Facing::NW | Facing::NE) {
        cell.rect(9, 10, 30, 2, STEEL_DARK);
    }
    if matches!(facing, Facing::S | Facing::SW | Facing::SE) {
        cell.rect(44, 12, 2, 14, STEEL_DARK);
    }
}

fn draw_turret(cell: &mut Cell<'_>, facing: Facing) {
    let centre = [24.0_f32, 19.0];
    let [dx, dy] = screen_direction(facing);
    let start = [centre[0] + dx * 4.0, centre[1] + dy * 4.0];
    let tip = [centre[0] + dx * 17.0, centre[1] + dy * 17.0];
    cell.line(start, tip, 4, SLATE);
    cell.disc(24, 19, 9, SLATE);
    cell.disc(24, 19, 8, STEEL);
    cell.team(20, 15, 8, 3);
    cell.rect(20, 15, 8, 1, STEEL_DARK);
}

fn draw_marker(cell: &mut Cell<'_>) {
    cell.rect(17, 33, 6, 2, GOLD);
    cell.rect(25, 33, 6, 2, GOLD);
    cell.rect(23, 27, 2, 6, GOLD);
    cell.rect(23, 35, 2, 4, GOLD);
    cell.disc(24, 34, 1, GOLD);
}

struct Cell<'a> {
    base: &'a mut [u8],
    mask: &'a mut [u8],
    width: u32,
    ox: u32,
    oy: u32,
}

impl Cell<'_> {
    fn put(&mut self, x: i32, y: i32, rgba: [u8; 4]) {
        if !(0..CELL_W as i32).contains(&x) || !(0..CELL_H as i32).contains(&y) {
            return;
        }
        let index = (((self.oy + y as u32) * self.width + self.ox + x as u32) * 4) as usize;
        self.base[index..index + 4].copy_from_slice(&rgba);
    }

    fn put_mask(&mut self, x: i32, y: i32) {
        if !(0..CELL_W as i32).contains(&x) || !(0..CELL_H as i32).contains(&y) {
            return;
        }
        let index = (((self.oy + y as u32) * self.width + self.ox + x as u32) * 4) as usize;
        self.mask[index..index + 4].copy_from_slice(&MASK_SET);
    }

    fn rect(&mut self, x: i32, y: i32, w: i32, h: i32, rgba: [u8; 4]) {
        for py in y..y + h {
            for px in x..x + w {
                self.put(px, py, rgba);
            }
        }
    }

    fn team(&mut self, x: i32, y: i32, w: i32, h: i32) {
        self.rect(x, y, w, h, PLASTER);
        for py in y..y + h {
            for px in x..x + w {
                self.put_mask(px, py);
            }
        }
    }

    fn disc(&mut self, cx: i32, cy: i32, radius: i32, rgba: [u8; 4]) {
        for dy in -radius..=radius {
            for dx in -radius..=radius {
                if dx * dx + dy * dy <= radius * radius {
                    self.put(cx + dx, cy + dy, rgba);
                }
            }
        }
    }

    fn ellipse(&mut self, cx: i32, cy: i32, rx: i32, ry: i32, rgba: [u8; 4]) {
        for dy in -ry..=ry {
            for dx in -rx..=rx {
                let nx = dx as f32 / rx as f32;
                let ny = dy as f32 / ry as f32;
                if nx * nx + ny * ny <= 1.0 {
                    self.put(cx + dx, cy + dy, rgba);
                }
            }
        }
    }

    fn line(&mut self, from: [f32; 2], to: [f32; 2], thickness: i32, rgba: [u8; 4]) {
        let steps = ((to[0] - from[0]).hypot(to[1] - from[1]) * 2.0).ceil() as i32;
        let steps = steps.max(1);
        let half = thickness / 2;
        for step in 0..=steps {
            let t = step as f32 / steps as f32;
            let x = (from[0] + (to[0] - from[0]) * t).round() as i32;
            let y = (from[1] + (to[1] - from[1]) * t).round() as i32;
            for dy in -half..=half {
                for dx in -half..=half {
                    self.put(x + dx, y + dy, rgba);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kam2_sprites::FACING_COUNT;

    #[test]
    fn atlas_matches_the_cell_layout() {
        let art = SpriteArt::build();
        assert_eq!(art.width(), CELL_W * FACING_COUNT as u32);
        assert_eq!(art.height(), CELL_H * ROWS);
        assert_eq!(art.layout().rows, ROWS);
        assert_eq!(
            art.base_bytes().len(),
            (art.width() * art.height() * 4) as usize
        );
        assert_eq!(art.mask_bytes().len(), art.base_bytes().len());
    }

    #[test]
    fn every_facing_has_art() {
        let art = SpriteArt::build();
        for row in [ROW_INFANTRY, ROW_HULL, ROW_TURRET] {
            for facing in Facing::ALL {
                let origin = [u32::from(facing.index()) * CELL_W, row * CELL_H];
                let mut opaque = 0;
                for y in 0..CELL_H {
                    for x in 0..CELL_W {
                        if art.base_rgba([origin[0] + x, origin[1] + y])[3] > 0 {
                            opaque += 1;
                        }
                    }
                }
                assert!(opaque > 250, "row {row} {facing:?} has {opaque} pixels");
            }
        }
        let s = [0, ROW_INFANTRY * CELL_H];
        let n = [4 * CELL_W, ROW_INFANTRY * CELL_H];
        let mut differs = false;
        for y in 0..CELL_H {
            for x in 0..24 {
                if art.base_rgba([s[0] + x, s[1] + y]) != art.base_rgba([n[0] + x, n[1] + y]) {
                    differs = true;
                }
            }
        }
        assert!(differs, "S and N infantry cells are identical");
    }

    #[test]
    fn mask_covers_only_the_team_bands() {
        let art = SpriteArt::build();
        for facing in Facing::ALL {
            assert_eq!(art.mask_alpha(infantry_team_pixel(facing)), 255);
            assert_eq!(art.mask_alpha(hull_team_pixel(facing)), 255);
            assert_eq!(art.mask_alpha(turret_team_pixel(facing)), 255);
            for at in [
                infantry_team_pixel(facing),
                hull_team_pixel(facing),
                turret_team_pixel(facing),
            ] {
                assert_eq!(art.base_rgba(at)[3], 255);
                assert_eq!(
                    [
                        art.base_rgba(at)[0],
                        art.base_rgba(at)[1],
                        art.base_rgba(at)[2]
                    ],
                    [PLASTER[0], PLASTER[1], PLASTER[2]]
                );
            }
        }
        let marker = marker_centre_pixel();
        assert_eq!(art.mask_alpha(marker), 0);
    }

    #[test]
    fn turret_barrels_end_in_slate_for_every_facing() {
        let art = SpriteArt::build();
        for facing in Facing::ALL {
            let tip = turret_barrel_tip(facing);
            let texel = art.base_rgba(tip);
            assert_eq!(
                [texel[0], texel[1], texel[2]],
                [SLATE[0], SLATE[1], SLATE[2]],
                "{facing:?} tip at {tip:?}"
            );
            assert_eq!(art.mask_alpha(tip), 0, "{facing:?} tip is not a team pixel");
        }
    }

    #[test]
    fn marker_is_gold_on_the_ground() {
        let art = SpriteArt::build();
        let texel = art.base_rgba(marker_centre_pixel());
        assert_eq!([texel[0], texel[1], texel[2]], [GOLD[0], GOLD[1], GOLD[2]]);
    }
}
