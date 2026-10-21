//! One frame of geometry: ground diamonds and house/unit sprite quads.
//!
//! Everything here is CPU data in framebuffer pixels, built from the sim and the
//! camera only; the window uploads it and draws. Houses are cells of a packed
//! building atlas, anchored with their manifest pivot so the sprite's front edge
//! sits on the footprint; units are atlas cells cut from the packed soldier
//! atlas or, when that file is missing, from engine-render's procedural
//! stand-in, turned to the eighth facing they walk or fight toward. Depth is the
//! fixed view's `x + z` key: ground first, then [`Part`] order.
//!
//! Two builders share one frame: [`build_with`] draws every house from one
//! cottage atlas, and [`build_with_arts`] asks the [`HouseArts`] registry which
//! atlas a house kind draws from, so a kind with its own building folder draws
//! its own cells and a kind without one falls back to the cottage. Every house
//! quad names its atlas slot in [`Scene::house_atlases`], so the window can
//! upload one sheet per slot and bind the right one per run of houses.

use engine_assets::bc::srgb_byte_to_linear;
use engine_render::kam2_iso::IsoCamera;
use engine_render::kam2_sprite_atlas::{CELL_H, CELL_W, ROWS, ROW_INFANTRY};
use engine_render::kam2_sprites::{facing_from_direction, Facing, SpriteAtlas};
use kam2_sim::{House, Sim, Terrain, Unit};

use crate::house::{HouseArts, HouseManifest, HouseState};
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
    /// One building cell per house, on the footprint centre.
    pub houses: Vec<SpriteQuad>,
    /// The atlas slot each house cell is cut from, parallel to `houses`.
    pub house_atlases: Vec<usize>,
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
/// frame. Every house draws from that one atlas, in slot zero.
#[must_use]
pub fn build_with(
    sim: &Sim,
    camera: &IsoCamera,
    units: &Units,
    houses: &HouseManifest,
    tick: u32,
) -> Scene {
    build_frame(sim, camera, units, tick, |house| {
        Some((0usize, houses, house.kind.footprint()))
    })
}

/// Build the frame with one atlas per house kind: `arts` says which manifest
/// and which atlas slot a house draws from. A kind with its own building
/// folder draws its own cells and its own footprint; a kind without one falls
/// back to the cottage and keeps the footprint the sim reserves.
#[must_use]
pub fn build_with_arts(
    sim: &Sim,
    camera: &IsoCamera,
    units: &Units,
    arts: &HouseArts,
    tick: u32,
) -> Scene {
    build_frame(sim, camera, units, tick, |house| {
        let art = arts.resolve(house.kind)?;
        let footprint = if art.own {
            let [w, h] = art.manifest.footprint();
            (i32::try_from(w).unwrap_or(0), i32::try_from(h).unwrap_or(0))
        } else {
            house.kind.footprint()
        };
        Some((art.slot, art.manifest, footprint))
    })
}

/// The shared frame builder: `resolve` names the atlas slot, the manifest and
/// the drawn footprint of one house.
fn build_frame<'a, F>(sim: &Sim, camera: &IsoCamera, units: &Units, tick: u32, resolve: F) -> Scene
where
    F: Fn(&House) -> Option<(usize, &'a HouseManifest, (i32, i32))>,
{
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
    let mut house_atlases = Vec::with_capacity(sim.houses().len());
    for house in sim.houses() {
        let Some((slot, art, footprint)) = resolve(house) else {
            continue;
        };
        let Some(quad) = house_quad(camera, house, art, footprint, tick) else {
            continue;
        };
        entries.push((
            house_depth(house.x, house.y, footprint),
            Part::House(drawn_houses.len()),
        ));
        drawn_houses.push(quad);
        house_atlases.push(slot);
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
        house_atlases,
        units: drawn_units,
        order: entries.into_iter().map(|(_, part)| part).collect(),
    }
}

/// Depth key of a house: its footprint base row at its west edge, which is the
/// centre of its bottom-left tile. A unit on the next row south sorts after it.
fn house_depth(x: i32, y: i32, footprint: (i32, i32)) -> f32 {
    let (_, h) = footprint;
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
/// centre, with the manifest pivot and cell size. `footprint` is the drawn
/// footprint: the manifest's own when the kind has its own art, the sim's when
/// the kind falls back to the cottage.
fn house_quad(
    camera: &IsoCamera,
    house: &House,
    art: &HouseManifest,
    footprint: (i32, i32),
    tick: u32,
) -> Option<SpriteQuad> {
    let row = art.row(HouseState::for_tick(tick, house.built_at))?;
    let [uv_min, uv_size] = art.cell_uv(row)?;
    let (w, h) = footprint;
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
    use kam2_sim::{Command, HouseKind, UnitKind};

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
            assert_eq!(quad.size, art.size(), "one manifest cell");
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
            quad.top_left[1] + art.size()[1] > corners[2][1],
            "the cell reaches the baked shadow row past the front edge"
        );
    }

    #[test]
    fn the_house_state_follows_the_first_working_tick() {
        let camera = IsoCamera::new(1280, 720);
        let sim = start();
        let farm = &sim.houses()[1];
        assert_eq!(farm.kind, HouseKind::Farm);
        assert_eq!(farm.built_at, 30, "a farm takes 30 ticks");
        let art = cottage();
        let atlas_h = f64::from(art.atlas_size()[1]);
        let padding = f64::from(art.padding());
        let pitch = f64::from(art.pitch()[1]);
        let half_built = (pitch + padding) / atlas_h;
        let finished = padding / atlas_h;
        for (tick, row_y) in [
            (0u32, half_built),
            (29, half_built),
            (30, finished),
            (120, finished),
        ] {
            let scene = frame(&sim, &camera, &Units::StandIn, tick);
            let quad = scene.houses[1];
            assert!(
                (f64::from(quad.uv_min[1]) - row_y).abs() < f64::from(EPS),
                "tick {tick} draws row {}",
                f64::from(quad.uv_min[1]) * atlas_h
            );
        }
        assert_eq!(
            art.row(HouseState::Damaged),
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
                house_depth(house.x, house.y, house.kind.footprint())
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

    /// The registry of the committed content: the cottage plus one slot per
    /// house kind, every manifest read from the files in the repo.
    fn registry() -> HouseArts {
        let mut arts = HouseArts::new();
        arts.set_cottage(cottage());
        for (kind, text) in crate::house::BUILDING_MANIFESTS {
            let manifest = HouseManifest::parse(text).expect("the committed manifest parses");
            arts.set_kind(kind, manifest);
        }
        arts
    }

    /// A road row at `y = 6` with one house of every kind above it.
    fn every_kind() -> Sim {
        let mut sim = Sim::new(MAP_W, MAP_H);
        for x in 0..MAP_W {
            sim.apply(Command::PlaceRoad { x, y: 6 })
                .expect("the road row fits");
        }
        for (kind, x, y) in [
            (HouseKind::Warehouse, 0, 3),
            (HouseKind::Farm, 4, 4),
            (HouseKind::Mill, 7, 4),
            (HouseKind::Bakery, 10, 4),
            (HouseKind::Woodcutter, 13, 4),
        ] {
            sim.apply(Command::PlaceHouse { kind, x, y })
                .expect("the house fits beside the road");
        }
        sim
    }

    #[test]
    fn every_kind_draws_its_own_atlas_slot() {
        let camera = IsoCamera::new(1280, 720);
        let sim = every_kind();
        let arts = registry();
        let scene = build_with_arts(&sim, &camera, &Units::StandIn, &arts, 0);
        assert_eq!(scene.houses.len(), 5);
        assert_eq!(scene.house_atlases.len(), scene.houses.len());
        for (index, house) in sim.houses().iter().enumerate() {
            assert_eq!(
                scene.house_atlases[index],
                usize::from(house.kind.index()) + 1,
                "{} draws its own atlas",
                house.kind.name()
            );
        }
    }

    #[test]
    fn a_kind_without_its_own_art_falls_back_to_the_cottage() {
        let camera = IsoCamera::new(1280, 720);
        let mut sim = Sim::new(MAP_W, MAP_H);
        for x in 0..MAP_W {
            sim.apply(Command::PlaceRoad { x, y: 6 })
                .expect("the road row fits");
        }
        sim.apply(Command::PlaceHouse {
            kind: HouseKind::Warehouse,
            x: 0,
            y: 3,
        })
        .expect("the warehouse fits beside the road");
        let mut arts = HouseArts::new();
        arts.set_cottage(cottage());
        let scene = build_with_arts(&sim, &camera, &Units::StandIn, &arts, 0);
        assert_eq!(scene.house_atlases, vec![0], "the cottage answers");
        // The fallback keeps the footprint the sim reserves: three tiles wide.
        let art = cottage();
        let house = &sim.houses()[0];
        let corners = footprint_screen(&camera, house.x, house.y, 3, 3);
        let anchor = [
            (corners[0][0] + corners[2][0]) * 0.5,
            (corners[0][1] + corners[2][1]) * 0.5,
        ];
        let quad = scene.houses[0];
        let pivot = [
            quad.top_left[0] + art.pivot()[0] as f32,
            quad.top_left[1] + art.pivot()[1] as f32,
        ];
        assert!(
            (pivot[0] - anchor[0]).abs() < EPS && (pivot[1] - anchor[1]).abs() < EPS,
            "the fallback pivot {pivot:?} sits on the 3x3 anchor {anchor:?}"
        );
    }

    #[test]
    fn the_cottage_builder_names_slot_zero_for_every_house() {
        let camera = IsoCamera::new(1280, 720);
        let sim = every_kind();
        let scene = frame(&sim, &camera, &Units::StandIn, 0);
        assert_eq!(scene.house_atlases.len(), scene.houses.len());
        assert!(
            scene.house_atlases.iter().all(|slot| *slot == 0),
            "the plain builder draws every house from the cottage atlas"
        );
    }
}
