//! One frame of geometry: ground diamonds and house/unit sprite quads.
//!
//! Everything here is CPU data in framebuffer pixels, built from the sim and the
//! camera only; the window uploads it and draws. Houses are cells of the packed
//! cottage atlas, anchored with their manifest pivot so the sprite's front edge
//! sits on the footprint; units are atlas cells cut from the packed soldier
//! atlas or, when that file is missing, from engine-render's procedural
//! stand-in, turned to the eighth facing they walk or fight toward. Depth is the
//! fixed view's `x + z` key: ground first, then [`Part`] order.

use engine_assets::bc::srgb_byte_to_linear;
use engine_render::kam2_iso::IsoCamera;
use engine_render::kam2_sprite_atlas::{CELL_H, CELL_W, ROWS, ROW_INFANTRY};
use engine_render::kam2_sprites::{facing_from_direction, Facing, SpriteAtlas};
use kam2_sim::{House, HouseKind, Sim, Terrain, Unit};

use crate::house::{HouseManifest, HouseState};
use crate::soldier::{frame_index, SoldierManifest, UnitClip};
use crate::view::{footprint_center_world, tile_center_world, tile_diamond, MAP_H, MAP_W};

/// Style-lock palette, sRGB.
const GRASS_LIGHT: [u8; 3] = [0x8F, 0xA8, 0x4E];
const GRASS_DARK: [u8; 3] = [0x5E, 0x74, 0x33];
const DIRT_LIGHT: [u8; 3] = [0xC9, 0xA2, 0x6B];
const DIRT_DARK: [u8; 3] = [0x8A, 0x6A, 0x43];
const ROCK: [u8; 3] = [0x8A, 0x81, 0x72];
const WATER: [u8; 3] = [0x1F, 0x4E, 0x6E];
const ROAD: [u8; 3] = [0xD9, 0xBE, 0x86];
const TEAM_BLUE: [u8; 3] = [0x2B, 0x6F, 0xD6];
const TEAM_RED: [u8; 3] = [0xC6, 0x37, 0x2E];
/// Houses carry no team mask; a white tint leaves every baked pixel as it is.
const NO_TINT: [u8; 3] = [0xFF, 0xFF, 0xFF];

/// One flat quad for the shape pass. Corners are framebuffer pixels in draw
/// order (top, right, bottom, left for a diamond); the colour is linear-space
/// RGB, ready for the sRGB surface.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShapeQuad {
    /// Four corners in framebuffer pixels, clockwise from the top.
    pub corners: [[f32; 2]; 4],
    /// Linear RGB, one value per channel in `0..=1`.
    pub color: [f32; 3],
}

/// One atlas cell for the sprite pass: a screen-space quad and the team colour.
/// The atlas and mask textures hold sRGB bytes; the shader tints and converts.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpriteQuad {
    /// Top-left framebuffer pixel of the cell.
    pub top_left: [f32; 2],
    /// Cell size in pixels.
    pub size: [f32; 2],
    /// Atlas UV origin of the cell.
    pub uv_min: [f32; 2],
    /// Atlas UV size of the cell.
    pub uv_size: [f32; 2],
    /// Team colour in sRGB bytes; the mask alpha picks the band.
    pub team: [u8; 3],
}

/// One depth-sorted billboard of the frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part {
    /// Index into [`Scene::houses`].
    House(usize),
    /// Index into [`Scene::units`].
    Unit(usize),
}

/// One frame ready to upload: the ground, the house cells, the unit cells and
/// the depth order of the billboards.
#[derive(Clone, Debug, Default)]
pub struct Scene {
    /// Ground diamonds, in row order, including the tiles under a house.
    pub ground: Vec<ShapeQuad>,
    /// One cottage cell per house, on the footprint centre.
    pub houses: Vec<SpriteQuad>,
    /// One sprite cell per unit.
    pub units: Vec<SpriteQuad>,
    /// Billboard order, farthest first.
    pub order: Vec<Part>,
}

/// Which atlas the unit billboards are cut from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Units {
    /// The packed soldier atlas and its manifest, read from
    /// `content/kam2/soldier/` at runtime.
    Soldier(SoldierManifest),
    /// engine-render's procedural cells, drawn only when the atlas is missing.
    StandIn,
}

impl Units {
    /// One billboard: the cell for `facing` and `clip` at sim `tick`, with the
    /// manifest pivot on `ground` and the side's team colour.
    fn quad(
        &self,
        ground: [f32; 2],
        facing: Facing,
        clip: UnitClip,
        tick: u32,
        team: [u8; 3],
    ) -> Option<SpriteQuad> {
        match self {
            Self::Soldier(manifest) => {
                let anim = manifest.clip(clip)?;
                let column = manifest.facing_column(facing)?;
                let row = anim.row.checked_add(frame_index(tick, anim))?;
                let [uv_min, uv_size] = manifest.cell_uv(row, column)?;
                Some(SpriteQuad {
                    top_left: manifest.top_left(ground),
                    size: manifest.size(),
                    uv_min,
                    uv_size,
                    team,
                })
            }
            Self::StandIn => {
                let layout = SpriteAtlas {
                    cell_w: CELL_W,
                    cell_h: CELL_H,
                    rows: ROWS,
                };
                let [uv_min, uv_size] = layout.cell_uv(ROW_INFANTRY, facing)?;
                Some(SpriteQuad {
                    top_left: [ground[0] - 24.0, ground[1] - 38.0],
                    size: [CELL_W as f32, CELL_H as f32],
                    uv_min,
                    uv_size,
                    team,
                })
            }
        }
    }
}

/// Build the frame for `sim` through `camera`: `units` picks the unit atlas,
/// `houses` the cottage atlas, `tick` picks the build state and the animation
/// frame.
#[must_use]
pub fn build_with(
    sim: &Sim,
    camera: &IsoCamera,
    units: &Units,
    houses: &HouseManifest,
    tick: u32,
) -> Scene {
    let mut ground = Vec::with_capacity(usize::try_from(MAP_W * MAP_H).unwrap_or(0));
    for y in 0..MAP_H {
        for x in 0..MAP_W {
            ground.push(ShapeQuad {
                corners: tile_diamond(camera, x, y),
                color: linear(tile_color(sim, x, y)),
            });
        }
    }
    let mut entries = Vec::with_capacity(sim.houses().len() + sim.units().len());
    let mut drawn_houses = Vec::with_capacity(sim.houses().len());
    for house in sim.houses() {
        if let Some(quad) = house_quad(camera, house, houses, tick) {
            entries.push((
                house_depth(house.x, house.y, house.kind),
                Part::House(drawn_houses.len()),
            ));
            drawn_houses.push(quad);
        }
    }
    let mut drawn_units = Vec::with_capacity(sim.units().len());
    for unit in sim.units() {
        if let Some(quad) = unit_quad(camera, sim, unit, units, tick) {
            entries.push((
                unit.x as f32 + unit.y as f32 + 1.0,
                Part::Unit(drawn_units.len()),
            ));
            drawn_units.push(quad);
        }
    }
    entries.sort_by(|a, b| a.0.total_cmp(&b.0));
    Scene {
        ground,
        houses: drawn_houses,
        units: drawn_units,
        order: entries.into_iter().map(|(_, part)| part).collect(),
    }
}

/// Depth key of a house: its footprint base row at its west edge, which is the
/// centre of its bottom-left tile. A unit on the next row south sorts after it.
fn house_depth(x: i32, y: i32, kind: HouseKind) -> f32 {
    let (_, h) = kind.footprint();
    (x + y + h) as f32
}

/// Ground colour of one tile: the checkerboard terrain ramp, the road on top.
fn tile_color(sim: &Sim, x: i32, y: i32) -> [u8; 3] {
    if sim.map().is_road(x, y) {
        return ROAD;
    }
    let light = (x + y) % 2 == 0;
    match sim.map().terrain(x, y) {
        Some(Terrain::Dirt) => {
            if light {
                DIRT_LIGHT
            } else {
                DIRT_DARK
            }
        }
        Some(Terrain::Rock) => ROCK,
        Some(Terrain::Water) => WATER,
        Some(Terrain::Grass) | None => {
            if light {
                GRASS_LIGHT
            } else {
                GRASS_DARK
            }
        }
    }
}

/// One house billboard: the state cell the sim tick picks, on the footprint
/// centre, with the manifest pivot and cell size.
fn house_quad(
    camera: &IsoCamera,
    house: &House,
    art: &HouseManifest,
    tick: u32,
) -> Option<SpriteQuad> {
    let row = art.row(HouseState::for_tick(tick, house.built_at))?;
    let [uv_min, uv_size] = art.cell_uv(row)?;
    let (w, h) = house.kind.footprint();
    let [wx, wz] = footprint_center_world(house.x, house.y, w, h);
    Some(SpriteQuad {
        top_left: art.top_left(camera.project([wx, 0.0, wz])),
        size: art.size(),
        uv_min,
        uv_size,
        team: NO_TINT,
    })
}

/// One unit billboard: the clip it plays, the facing it walks or fights toward,
/// and the side's team colour. The atlas decides cell size and pivot.
fn unit_quad(
    camera: &IsoCamera,
    sim: &Sim,
    unit: &Unit,
    units: &Units,
    tick: u32,
) -> Option<SpriteQuad> {
    let facing = unit_facing(sim, unit);
    let clip = if unit.path.is_empty() {
        UnitClip::Idle
    } else {
        UnitClip::Walk
    };
    let [wx, wz] = tile_center_world(unit.x, unit.y);
    let ground = camera.project([wx, 0.0, wz]);
    let team = if unit.side == 0 { TEAM_BLUE } else { TEAM_RED };
    units.quad(ground, facing, clip, tick, team)
}

/// The eighth facing of a unit: its next path step while walking, otherwise its
/// target, otherwise toward the camera.
fn unit_facing(sim: &Sim, unit: &Unit) -> Facing {
    let direction = if let Some(&(next_x, next_y)) = unit.path.front() {
        Some([(next_x - unit.x) as f32, (next_y - unit.y) as f32])
    } else {
        unit.target
            .and_then(|id| sim.unit(id))
            .map(|target| [(target.x - unit.x) as f32, (target.y - unit.y) as f32])
    };
    direction
        .and_then(facing_from_direction)
        .unwrap_or(Facing::S)
}

/// sRGB palette bytes to the linear values the sRGB surface expects.
fn linear(rgb: [u8; 3]) -> [f32; 3] {
    [
        srgb_byte_to_linear(rgb[0]),
        srgb_byte_to_linear(rgb[1]),
        srgb_byte_to_linear(rgb[2]),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::view::footprint_screen;
    use kam2_sim::{Command, UnitKind};

    const EPS: f32 = 0.01;

    fn start() -> Sim {
        crate::script::match_start().expect("the fixed start village places")
    }

    fn cottage() -> HouseManifest {
        HouseManifest::parse(crate::house::MANIFEST_JSON)
            .expect("the committed cottage manifest parses")
    }

    fn frame(sim: &Sim, camera: &IsoCamera, units: &Units, tick: u32) -> Scene {
        build_with(sim, camera, units, &cottage(), tick)
    }

    #[test]
    fn the_start_village_draws_ground_houses_and_units() {
        let camera = IsoCamera::new(1280, 720);
        let sim = start();
        let scene = frame(&sim, &camera, &Units::StandIn, 0);
        assert_eq!(
            scene.ground.len(),
            usize::try_from(MAP_W * MAP_H).unwrap_or(0),
            "every tile gets one diamond, including under the houses"
        );
        assert_eq!(scene.houses.len(), 3);
        assert_eq!(scene.units.len(), 4);
        assert_eq!(scene.order.len(), 7);
        assert_eq!(scene.order.first(), Some(&Part::House(0)));
        assert_eq!(scene.order.last(), Some(&Part::Unit(3)));
        for quad in &scene.ground {
            for corner in quad.corners {
                assert!((0.0..=1280.0).contains(&corner[0]), "{corner:?}");
                assert!((0.0..=720.0).contains(&corner[1]), "{corner:?}");
            }
            for channel in quad.color {
                assert!((0.0..=1.0).contains(&channel), "{channel}");
            }
        }
    }

    #[test]
    fn the_cottage_stands_on_its_footprint_with_the_manifest_pivot() {
        let camera = IsoCamera::new(1280, 720);
        let sim = start();
        let scene = frame(&sim, &camera, &Units::StandIn, 30);
        assert_eq!(scene.houses.len(), sim.houses().len());
        let art = cottage();
        for (index, house) in sim.houses().iter().enumerate() {
            let (w, h) = house.kind.footprint();
            let quad = scene.houses[index];
            assert_eq!(quad.size, [128.0, 128.0], "one 128 px cell");
            // The pivot is the footprint centre: top and bottom corners meet there.
            let corners = footprint_screen(&camera, house.x, house.y, w, h);
            let anchor = [
                (corners[0][0] + corners[2][0]) * 0.5,
                (corners[0][1] + corners[2][1]) * 0.5,
            ];
            let pivot = [
                quad.top_left[0] + art.pivot()[0] as f32,
                quad.top_left[1] + art.pivot()[1] as f32,
            ];
            assert!(
                (pivot[0] - anchor[0]).abs() < EPS && (pivot[1] - anchor[1]).abs() < EPS,
                "house {index} pivot {pivot:?} on anchor {anchor:?}"
            );
        }
        // The cottage's own 2x2 footprint puts its last row on the front tile edge.
        let farm = &sim.houses()[1];
        let corners = footprint_screen(&camera, farm.x, farm.y, 2, 2);
        let quad = scene.houses[1];
        assert!(
            (quad.top_left[1] + 128.0 - (corners[2][1] + 1.0)).abs() < EPS,
            "the cell reaches the baked shadow row just past the front edge"
        );
    }

    #[test]
    fn the_house_state_follows_the_first_working_tick() {
        let camera = IsoCamera::new(1280, 720);
        let sim = start();
        let farm = &sim.houses()[1];
        assert_eq!(farm.kind, HouseKind::Farm);
        assert_eq!(farm.built_at, 30, "a farm takes 30 ticks");
        let half_built = (1.0 * 130.0 + 1.0) / 390.0;
        let finished = 1.0 / 390.0;
        for (tick, row_y) in [
            (0u32, half_built),
            (29, half_built),
            (30, finished),
            (120, finished),
        ] {
            let scene = frame(&sim, &camera, &Units::StandIn, tick);
            let quad = scene.houses[1];
            assert!(
                (quad.uv_min[1] - row_y).abs() < EPS,
                "tick {tick} draws row {}",
                quad.uv_min[1] * 390.0
            );
        }
        assert_eq!(
            cottage().row(HouseState::Damaged),
            Some(2),
            "the damaged row loads, no public damage field selects it yet"
        );
    }

    #[test]
    fn units_turn_to_their_eighth_facing() {
        let camera = IsoCamera::new(1280, 720);
        let mut sim = Sim::new(MAP_W, MAP_H);
        sim.apply(Command::SpawnUnit {
            kind: UnitKind::Swordsman,
            side: 0,
            x: 2,
            y: 2,
        })
        .expect("open ground");
        let still = frame(&sim, &camera, &Units::StandIn, 0);
        assert_eq!(still.units.len(), 1);
        assert!(
            (still.units[0].uv_min[0] - 0.0).abs() < EPS,
            "idle faces south"
        );

        let id = sim.units()[0].id;
        sim.apply(Command::MoveUnit { id, x: 3, y: 2 })
            .expect("one step east");
        let walking = frame(&sim, &camera, &Units::StandIn, 0);
        assert!(
            (walking.units[0].uv_min[0] - 0.75).abs() < EPS,
            "east is facing 6 of 8: {}",
            walking.units[0].uv_min[0]
        );
        assert!((walking.units[0].uv_size[0] - 0.125).abs() < EPS);
        assert_eq!(walking.units[0].team, TEAM_BLUE);
    }

    fn soldier() -> Units {
        Units::Soldier(
            SoldierManifest::parse(crate::soldier::MANIFEST_JSON)
                .expect("the committed soldier manifest parses"),
        )
    }

    #[test]
    fn the_real_atlas_walks_one_row_per_tick_and_turns_with_the_facing() {
        let camera = IsoCamera::new(1280, 720);
        let mut sim = Sim::new(MAP_W, MAP_H);
        sim.apply(Command::SpawnUnit {
            kind: UnitKind::Swordsman,
            side: 0,
            x: 2,
            y: 2,
        })
        .expect("open ground");
        let id = sim.units()[0].id;
        sim.apply(Command::MoveUnit { id, x: 3, y: 2 })
            .expect("one step east");
        let units = soldier();
        for (tick, row) in [(0u32, 1u32), (1, 2), (7, 8), (8, 1), (15, 8)] {
            let scene = frame(&sim, &camera, &units, tick);
            assert_eq!(scene.units.len(), 1);
            let quad = scene.units[0];
            assert!(
                (quad.uv_min[0] - (6.0 * 66.0 + 1.0) / 528.0).abs() < EPS,
                "east is column 6 at tick {tick}: {}",
                quad.uv_min[0]
            );
            assert!(
                (quad.uv_min[1] - (row as f32 * 66.0 + 1.0) / 594.0).abs() < EPS,
                "tick {tick} is walk row {row}: {}",
                quad.uv_min[1]
            );
            assert!((quad.uv_size[0] - 64.0 / 528.0).abs() < EPS);
            assert!((quad.uv_size[1] - 64.0 / 594.0).abs() < EPS);
        }
    }

    #[test]
    fn the_real_atlas_stands_on_the_tile_with_its_manifest_pivot() {
        let camera = IsoCamera::new(1280, 720);
        let sim = start();
        let scene = frame(&sim, &camera, &soldier(), 3);
        assert_eq!(scene.units.len(), 4);
        for (index, unit) in sim.units().iter().enumerate() {
            let ground = crate::view::tile_screen(&camera, unit.x, unit.y);
            let quad = scene.units[index];
            assert_eq!(quad.size, [64.0, 64.0]);
            assert!((quad.top_left[0] + 32.0 - ground[0]).abs() < EPS);
            assert!(
                (quad.top_left[1] + 56.0 - ground[1]).abs() < EPS,
                "the pivot sits on the tile center"
            );
            assert!(
                (quad.uv_min[1] - 1.0 / 594.0).abs() < EPS,
                "a standing unit holds the pose row, not a walk frame"
            );
        }
    }

    #[test]
    fn the_depth_order_runs_far_to_near() {
        let camera = IsoCamera::new(1280, 720);
        let sim = start();
        let scene = frame(&sim, &camera, &Units::StandIn, 0);
        let key = |part: &Part| match *part {
            Part::House(index) => {
                // The drawn house list keeps the sim's house order.
                let house = &sim.houses()[index];
                house_depth(house.x, house.y, house.kind)
            }
            Part::Unit(index) => {
                let unit = &sim.units()[index];
                unit.x as f32 + unit.y as f32 + 1.0
            }
        };
        for pair in scene.order.windows(2) {
            assert!(key(&pair[0]) <= key(&pair[1]), "near part drawn first");
        }
    }

    #[test]
    fn a_unit_in_front_of_a_house_draws_over_it() {
        let camera = IsoCamera::new(1280, 720);
        let mut sim = Sim::new(MAP_W, MAP_H);
        for x in 0..MAP_W {
            sim.apply(Command::PlaceRoad { x, y: 6 })
                .expect("the road row fits");
        }
        sim.apply(Command::PlaceHouse {
            kind: HouseKind::Farm,
            x: 4,
            y: 4,
        })
        .expect("the farm touches the road");
        for (side, x, y) in [(0u8, 4i32, 3i32), (0, 4, 6)] {
            sim.apply(Command::SpawnUnit {
                kind: UnitKind::Swordsman,
                side,
                x,
                y,
            })
            .expect("open ground beside the farm");
        }
        let scene = frame(&sim, &camera, &Units::StandIn, 0);
        assert_eq!(scene.houses.len(), 1);
        assert_eq!(
            scene.order,
            vec![Part::Unit(0), Part::House(0), Part::Unit(1)],
            "the unit south of the house draws last"
        );
    }

    #[test]
    fn the_side_colours_follow_the_style_lock() {
        let camera = IsoCamera::new(1280, 720);
        let sim = start();
        let scene = frame(&sim, &camera, &Units::StandIn, 0);
        assert_eq!(scene.units.len(), 4);
        let teams: Vec<[u8; 3]> = scene.units.iter().map(|unit| unit.team).collect();
        assert_eq!(teams.iter().filter(|team| **team == TEAM_BLUE).count(), 2);
        assert_eq!(teams.iter().filter(|team| **team == TEAM_RED).count(), 2);
        let grass = scene
            .ground
            .iter()
            .any(|quad| quad.color == linear(GRASS_LIGHT));
        assert!(grass, "the light grass tile is in the palette");
    }
}
