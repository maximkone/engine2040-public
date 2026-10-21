//! The flat frame the page reads: one `Float32Array` of ground and billboards.
//!
//! Layout, all values `f32`:
//! - header: `[ground_count, billboard_count, unit_count, house_count]`;
//! - `ground_count` diamonds, [`GROUND_STRIDE`] values each: four corners
//!   `x0 y0 x1 y1 x2 y2 x3 y3` in framebuffer pixels, then linear RGB;
//! - `billboard_count` sprites, [`BILLBOARD_STRIDE`] values each, farthest
//!   first: kind ([`KIND_UNIT`] or [`KIND_HOUSE`]), team colour in sRGB bytes,
//!   top-left pixel, cell size and atlas UV origin plus size.
//!
//! Colours stay linear, exactly as the scene pass holds them for the sRGB
//! surface; the page converts them once per frame. The depth order is the order
//! of the billboard rows: the caller never sorts.

use kam2_game::scene::{Part, Scene};

/// Values in the frame header.
pub const HEADER: usize = 4;
/// Values per ground diamond: four corners and a linear RGB colour.
pub const GROUND_STRIDE: usize = 11;
/// Values per billboard: kind, team, top-left, size and UV rectangle.
pub const BILLBOARD_STRIDE: usize = 12;
/// Billboard kind of a unit cell (the soldier atlas).
pub const KIND_UNIT: f32 = 1.0;
/// Billboard kind of a house cell (the cottage atlas).
pub const KIND_HOUSE: f32 = 2.0;

/// Flatten one scene into the page's frame.
#[must_use]
pub fn flatten(scene: &Scene) -> Vec<f32> {
    let mut billboards = Vec::with_capacity(scene.order.len() * BILLBOARD_STRIDE);
    let mut units = 0;
    let mut houses = 0;
    for part in &scene.order {
        let (kind, quad) = match *part {
            Part::Unit(index) => match scene.units.get(index) {
                Some(quad) => (KIND_UNIT, quad),
                None => continue,
            },
            Part::House(index) => match scene.houses.get(index) {
                Some(quad) => (KIND_HOUSE, quad),
                None => continue,
            },
        };
        if kind == KIND_UNIT {
            units += 1;
        } else {
            houses += 1;
        }
        billboards.extend([kind]);
        billboards.extend(quad.team.map(f32::from));
        billboards.extend([
            quad.top_left[0],
            quad.top_left[1],
            quad.size[0],
            quad.size[1],
            quad.uv_min[0],
            quad.uv_min[1],
            quad.uv_size[0],
            quad.uv_size[1],
        ]);
    }
    let mut out =
        Vec::with_capacity(HEADER + scene.ground.len() * GROUND_STRIDE + billboards.len());
    out.extend([
        scene.ground.len() as f32,
        (units + houses) as f32,
        units as f32,
        houses as f32,
    ]);
    for quad in &scene.ground {
        for corner in quad.corners {
            out.extend(corner);
        }
        out.extend(quad.color);
    }
    out.extend(billboards);
    out
}

#[cfg(test)]
mod tests {
    use super::{BILLBOARD_STRIDE, GROUND_STRIDE, HEADER, KIND_HOUSE, KIND_UNIT};
    use crate::shell::Shell;
    use kam2_game::house::{HouseManifest, HouseState};
    use kam2_game::soldier::SoldierManifest;

    /// The committed cottage manifest; it is the authority for the cell size.
    const HOUSE_MANIFEST_JSON: &str = include_str!("../../../content/kam2/house/manifest.json");
    /// The committed soldier manifest; it is the authority for the cell size.
    const SOLDIER_MANIFEST_JSON: &str =
        include_str!("../../../content/kam2/soldier/manifest.json");

    fn house_manifest() -> HouseManifest {
        HouseManifest::parse(HOUSE_MANIFEST_JSON).expect("the committed house manifest parses")
    }

    fn soldier_manifest() -> SoldierManifest {
        SoldierManifest::parse(SOLDIER_MANIFEST_JSON).expect("the committed soldier manifest parses")
    }

    fn frame() -> (Shell, Vec<f32>) {
        let shell = Shell::new().expect("the start village places");
        let flat = shell.frame();
        (shell, flat)
    }

    #[test]
    fn the_header_counts_every_section() {
        let (shell, flat) = frame();
        let scene = shell.scene();
        assert_eq!(flat[0] as usize, scene.ground.len());
        assert_eq!(flat[1] as usize, scene.houses.len() + scene.units.len());
        assert_eq!(flat[2] as usize, scene.units.len());
        assert_eq!(flat[3] as usize, scene.houses.len());
        assert_eq!(
            flat.len(),
            HEADER + (flat[0] as usize) * GROUND_STRIDE + (flat[1] as usize) * BILLBOARD_STRIDE
        );
    }

    #[test]
    fn every_ground_diamond_fits_the_viewport_and_holds_a_linear_colour() {
        let (_, flat) = frame();
        let count = flat[0] as usize;
        assert_eq!(count, 16 * 12, "all field tiles are drawn, house plots too");
        for i in 0..count {
            let o = HEADER + i * GROUND_STRIDE;
            for corner in 0..4 {
                let x = flat[o + corner * 2];
                let y = flat[o + corner * 2 + 1];
                assert!((0.0..=1280.0).contains(&x), "corner x {x}");
                assert!((0.0..=720.0).contains(&y), "corner y {y}");
            }
            for channel in &flat[o + 8..o + 11] {
                assert!((0.0..=1.0).contains(channel), "linear channel {channel}");
            }
        }
    }

    #[test]
    fn every_billboard_carries_its_cell_and_a_full_uv() {
        let (shell, flat) = frame();
        assert_eq!(shell.units(), 4);
        assert_eq!(shell.houses(), 3);
        let soldier = soldier_manifest().size();
        let cottage = house_manifest().size();        let billboards = flat[1] as usize;
        let base = HEADER + (flat[0] as usize) * GROUND_STRIDE;
        for i in 0..billboards {
            let o = base + i * BILLBOARD_STRIDE;
            let kind = flat[o];
            let width = flat[o + 6];
            let height = flat[o + 7];
            if kind == KIND_UNIT {
                assert_eq!([width, height], soldier, "soldier cell");
            } else {
                assert_eq!(kind, KIND_HOUSE);
                assert_eq!([width, height], cottage, "cottage cell");
            }
            for value in &flat[o + 8..o + 12] {
                assert!((0.0..=1.0).contains(value), "uv {value}");
            }
            for channel in &flat[o + 1..o + 4] {
                assert!((0.0..=255.0).contains(channel), "team byte {channel}");
            }
        }
    }

    #[test]
    fn the_order_holds_every_unit_and_house_once() {
        let (shell, flat) = frame();
        let billboards = flat[1] as usize;
        let base = HEADER + (flat[0] as usize) * GROUND_STRIDE;
        let mut units = 0;
        let mut houses = 0;
        for i in 0..billboards {
            match flat[base + i * BILLBOARD_STRIDE] {
                KIND_UNIT => units += 1,
                KIND_HOUSE => houses += 1,
                other => panic!("unknown billboard kind {other}"),
            }
        }
        assert_eq!(units, shell.units() as usize);
        assert_eq!(houses, shell.houses() as usize);
    }

    #[test]
    fn a_house_still_building_carries_the_half_built_cell() {
        let shell = Shell::new().expect("the start village places");
        let scene = shell.scene();
        assert_eq!(shell.ticks(), 0);
        let manifest = house_manifest();
        let row = |state: HouseState| {
            let row = manifest.row(state).expect("the manifest names the state");
            let [origin, _] = manifest.cell_uv(row).expect("the state row is inside the atlas");
            origin[1]
        };
        let finished = row(HouseState::Finished);
        let half = row(HouseState::HalfBuilt);
        let rows: Vec<f32> = scene.houses.iter().map(|quad| quad.uv_min[1]).collect();
        assert!(
            rows.iter().any(|v| (v - half).abs() < 1e-6),
            "the farm and the woodcutter still build at tick zero: {rows:?}"
        );
        assert!(
            rows.iter().any(|v| (v - finished).abs() < 1e-6),
            "the warehouse is finished at tick zero: {rows:?}"
        );
    }
}
