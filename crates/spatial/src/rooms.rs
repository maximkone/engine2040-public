//! SPR9: rooms as flood-filled regions of the wall boxes, with their exits.
//!
//! Consumer: `scene.query` kinds `in_room`, `room_of`, `exits_of` and the
//! brief's room grouping (lane A wires them; see
//! `docs/plan/spatial-entry-points.md`). Walls are tall, thin, physical boxes.
//! The plan view is rasterised on a cm grid; free cells are flood-filled.
//! Units: meters in, integer cm out. Determinism: scans run z ascending then x
//! ascending, rooms are numbered in scan order (`room_1..n`), every list is
//! sorted by integer keys, so input order never moves a byte.
//!
//! Doors: a run of free cells squeezed between two walls on one axis is a
//! gap. Under `capsule_cm` the capsule cannot pass: solid. Up to
//! [`DOOR_MAX_CM`]: a doorway, which separates rooms and is reported as an
//! exit (width = the gap). Wider: open floor. A doorway that touches only one
//! room (a pillar near a wall) is floor again. Free space that reaches the
//! edge of the padded grid is outside, id 0.

use super::index::{pos_known, size_known, to_cm};
use super::Actor;
use std::collections::{BTreeMap, BTreeSet, VecDeque};

fn ix<T: TryInto<usize>>(v: T) -> usize {
    v.try_into().unwrap_or(0)
}

/// A wall is at least this tall, in cm.
pub const WALL_MIN_H_CM: i32 = 150;
/// A wall's thin side is at most this, in cm.
pub const WALL_MAX_T_CM: i32 = 50;
/// Widest gap that still reads as a doorway, in cm.
pub const DOOR_MAX_CM: i32 = 200;
/// Default capsule width (door must be wider), in cm.
pub const CAPSULE_CM: i32 = 60;
/// Smallest raster cell, in cm; grows by doubling to stay under [`MAX_CELLS`].
const CELL_CM: i32 = 20;
/// Raster budget in cells.
const MAX_CELLS: i64 = 250_000;
/// Rooms smaller than this are noise, in cm^2.
const MIN_ROOM_CM2: i64 = 10_000;

/// One room.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Room {
    /// 1-based, in scan order.
    pub id: u32,
    pub area_cm2: i64,
    /// Centroid of its cells, `[x, z]` cm.
    pub center_cm: [i32; 2],
    /// Actor ids whose centers sit inside it (walls excluded), ascending.
    pub members: Vec<u32>,
}

/// A doorway between two rooms, or between a room and outside (`b == 0`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Exit {
    pub a: u32,
    pub b: u32,
    /// Narrowest gap along the doorway, cm.
    pub width_cm: i32,
    /// Center of the doorway cells, `[x, z]` cm.
    pub center_cm: [i32; 2],
}

/// Rooms, exits and the raster that answers point queries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rooms {
    pub rooms: Vec<Room>,
    pub exits: Vec<Exit>,
    cell: i32,
    origin: [i32; 2],
    w: i32,
    h: i32,
    label: Vec<u32>,
}

impl Rooms {
    /// Room holding the point, or `None` outside every room (walls included).
    pub fn room_of(&self, x_cm: i32, z_cm: i32) -> Option<u32> {
        let (cx, cz) = (
            (x_cm - self.origin[0]).div_euclid(self.cell),
            (z_cm - self.origin[1]).div_euclid(self.cell),
        );
        if cx < 0 || cz < 0 || cx >= self.w || cz >= self.h {
            return None;
        }
        Some(self.label[ix(cz * self.w + cx)]).filter(|r| *r > 0)
    }

    /// Exits touching room `id`, in exit order.
    pub fn exits_of(&self, id: u32) -> Vec<&Exit> {
        self.exits
            .iter()
            .filter(|e| e.a == id || e.b == id)
            .collect()
    }
}

struct Wall {
    lo: [i32; 2],
    hi: [i32; 2],
}

fn walls_of(actors: &[Actor]) -> Vec<Wall> {
    actors
        .iter()
        .filter(|a| a.physical && pos_known(a) && size_known(a))
        .filter(|a| a.sx > 0.0 && a.sy > 0.0 && a.sz > 0.0)
        .filter(|a| {
            let (sx, sy, sz) = (to_cm(a.sx).abs(), to_cm(a.sy).abs(), to_cm(a.sz).abs());
            sy >= WALL_MIN_H_CM && sx.min(sz) <= WALL_MAX_T_CM
        })
        .map(|a| {
            let (hx, hz) = (to_cm(a.sx).abs() / 2, to_cm(a.sz).abs() / 2);
            let (cx, cz) = (to_cm(a.x), to_cm(a.z));
            Wall {
                lo: [cx - hx, cz - hz],
                hi: [cx + hx, cz + hz],
            }
        })
        .collect()
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Cell {
    Solid,
    Free,
    Door(i32),
}

/// Plan-view raster geometry.
#[derive(Clone, Copy)]
struct Raster {
    cell: i32,
    origin: [i32; 2],
    w: i32,
    h: i32,
}

impl Raster {
    fn at(&self, x: i32, z: i32) -> usize {
        ix(z * self.w + x)
    }
    fn neighbours(&self, x: i32, z: i32) -> Vec<(i32, i32)> {
        [(x + 1, z), (x - 1, z), (x, z + 1), (x, z - 1)]
            .into_iter()
            .filter(|(nx, nz)| *nx >= 0 && *nz >= 0 && *nx < self.w && *nz < self.h)
            .collect()
    }
    fn on_border(&self, x: i32, z: i32) -> bool {
        x == 0 || z == 0 || x == self.w - 1 || z == self.h - 1
    }
}

/// Raster the walls, then turn narrow gaps into solid (under the capsule) or
/// doorway cells (up to [`DOOR_MAX_CM`]).
fn rasterize(walls: &[Wall], capsule_cm: i32) -> (Raster, Vec<Cell>) {
    let lo = [0, 1].map(|k| walls.iter().map(|w| w.lo[k]).min().unwrap_or(0));
    let hi = [0, 1].map(|k| walls.iter().map(|w| w.hi[k]).max().unwrap_or(0));
    let mut cell = CELL_CM;
    while i64::from((hi[0] - lo[0]) / cell + 3) * i64::from((hi[1] - lo[1]) / cell + 3) > MAX_CELLS
    {
        cell *= 2;
    }
    let origin = [
        lo[0].div_euclid(cell) * cell - cell,
        lo[1].div_euclid(cell) * cell - cell,
    ];
    let r = Raster {
        cell,
        origin,
        w: (hi[0] - origin[0]) / cell + 2,
        h: (hi[1] - origin[1]) / cell + 2,
    };
    let mut grid = vec![Cell::Free; ix(r.w * r.h)];
    for wall in walls {
        let span = |k: usize| {
            (
                (wall.lo[k] - origin[k]).div_euclid(cell),
                (wall.hi[k] - origin[k] - 1).div_euclid(cell),
            )
        };
        let ((x0, x1), (z0, z1)) = (span(0), span(1));
        for z in z0..=z1 {
            for x in x0..=x1 {
                grid[r.at(x, z)] = Cell::Solid;
            }
        }
    }
    let solid: Vec<bool> = grid.iter().map(|c| *c == Cell::Solid).collect();
    let mut run_w = vec![i32::MAX; grid.len()];
    for z in 0..r.h {
        scan_runs(
            r.w,
            |x| solid[r.at(x, z)],
            |x, len| run_w[r.at(x, z)] = run_w[r.at(x, z)].min(len * cell),
        );
    }
    for x in 0..r.w {
        scan_runs(
            r.h,
            |z| solid[r.at(x, z)],
            |z, len| run_w[r.at(x, z)] = run_w[r.at(x, z)].min(len * cell),
        );
    }
    for (i, g) in grid.iter_mut().enumerate() {
        if *g == Cell::Free && run_w[i] <= DOOR_MAX_CM {
            *g = if run_w[i] <= capsule_cm {
                Cell::Solid
            } else {
                Cell::Door(run_w[i])
            };
        }
    }
    (r, grid)
}

/// Flood-fill the free cells. Returns the component id per cell (0 = none) and
/// each component's cell count, or -1 when it touches the border (outside).
fn components(r: &Raster, grid: &[Cell]) -> (Vec<u32>, Vec<i64>) {
    let mut comp = vec![0u32; grid.len()];
    let mut sizes: Vec<i64> = vec![0];
    for z in 0..r.h {
        for x in 0..r.w {
            if grid[r.at(x, z)] != Cell::Free || comp[r.at(x, z)] != 0 {
                continue;
            }
            let id = u32::try_from(sizes.len()).unwrap_or(u32::MAX);
            let (mut n, mut border) = (0i64, false);
            let mut queue = VecDeque::from([(x, z)]);
            comp[r.at(x, z)] = id;
            while let Some((cx, cz)) = queue.pop_front() {
                n += 1;
                border |= r.on_border(cx, cz);
                for (nx, nz) in r.neighbours(cx, cz) {
                    if grid[r.at(nx, nz)] == Cell::Free && comp[r.at(nx, nz)] == 0 {
                        comp[r.at(nx, nz)] = id;
                        queue.push_back((nx, nz));
                    }
                }
            }
            sizes.push(if border { -1 } else { n });
        }
    }
    (comp, sizes)
}

/// One connected doorway: the free components it touches, its narrowest gap
/// and its center cell.
struct Door {
    touch: BTreeSet<u32>,
    width: i32,
    center: [i32; 2],
}

fn doorways(r: &Raster, grid: &[Cell], comp: &[u32]) -> (Vec<u32>, Vec<Door>) {
    let mut door_of = vec![u32::MAX; grid.len()];
    let mut doors: Vec<Door> = Vec::new();
    for z in 0..r.h {
        for x in 0..r.w {
            if !matches!(grid[r.at(x, z)], Cell::Door(_)) || door_of[r.at(x, z)] != u32::MAX {
                continue;
            }
            let did = u32::try_from(doors.len()).unwrap_or(u32::MAX);
            let (mut touch, mut width, mut n, mut sum) =
                (BTreeSet::new(), i32::MAX, 0i64, [0i64; 2]);
            let mut queue = VecDeque::from([(x, z)]);
            door_of[r.at(x, z)] = did;
            while let Some((cx, cz)) = queue.pop_front() {
                if let Cell::Door(run) = grid[r.at(cx, cz)] {
                    width = width.min(run);
                }
                n += 1;
                sum = [sum[0] + i64::from(cx), sum[1] + i64::from(cz)];
                for (nx, nz) in r.neighbours(cx, cz) {
                    match grid[r.at(nx, nz)] {
                        Cell::Door(_) if door_of[r.at(nx, nz)] == u32::MAX => {
                            door_of[r.at(nx, nz)] = did;
                            queue.push_back((nx, nz));
                        }
                        Cell::Free => {
                            touch.insert(comp[r.at(nx, nz)]);
                        }
                        _ => {}
                    }
                }
            }
            let mean = |s: i64| i32::try_from(s / n).unwrap_or(0);
            doors.push(Door {
                touch,
                width,
                center: [mean(sum[0]), mean(sum[1])],
            });
        }
    }
    (door_of, doors)
}

/// Find the rooms of a scene. `capsule_cm` is the width a gap must exceed.
pub fn find_rooms(actors: &[Actor], capsule_cm: i32) -> Rooms {
    let walls = walls_of(actors);
    if walls.is_empty() {
        return Rooms {
            rooms: Vec::new(),
            exits: Vec::new(),
            cell: CELL_CM,
            origin: [0, 0],
            w: 0,
            h: 0,
            label: Vec::new(),
        };
    }
    let (r, grid) = rasterize(&walls, capsule_cm);
    let (comp, sizes) = components(&r, &grid);
    let (door_of, doors) = doorways(&r, &grid, &comp);
    let outside = |c: u32| sizes[ix(c)] < 0;
    // Rooms: interior components big enough, numbered in scan order.
    let cell2 = i64::from(r.cell) * i64::from(r.cell);
    let mut room_no: BTreeMap<u32, u32> = BTreeMap::new();
    for c in 1..sizes.len() {
        let c = u32::try_from(c).unwrap_or(0);
        if !outside(c) && sizes[ix(c)] * cell2 >= MIN_ROOM_CM2 {
            let id = u32::try_from(room_no.len()).unwrap_or(0) + 1;
            room_no.insert(c, id);
        }
    }
    // A doorway touching two sides is an exit; touching one room only it is floor.
    let (mut merge, mut exits) = (BTreeMap::new(), Vec::new());
    for (i, d) in doors.iter().enumerate() {
        let ends: BTreeSet<u32> = d
            .touch
            .iter()
            .map(|c| if outside(*c) { 0 } else { *c })
            .collect();
        if ends.len() >= 2 {
            let list: Vec<u32> = ends.into_iter().collect();
            let room = |c: u32| {
                if c == 0 {
                    Some(0)
                } else {
                    room_no.get(&c).copied()
                }
            };
            if let (Some(ra), Some(rb)) = (room(list[0]), room(list[1])) {
                exits.push(Exit {
                    a: ra.min(rb),
                    b: ra.max(rb),
                    width_cm: d.width,
                    center_cm: [
                        r.origin[0] + d.center[0] * r.cell + r.cell / 2,
                        r.origin[1] + d.center[1] * r.cell + r.cell / 2,
                    ],
                });
            }
        } else if let Some(only) = ends.into_iter().next().filter(|c| *c != 0) {
            merge.insert(u32::try_from(i).unwrap_or(0), only);
        }
    }
    exits.sort_by_key(|e| (e.a, e.b, e.center_cm));
    let label: Vec<u32> = (0..grid.len())
        .map(|i| {
            let c = if comp[i] != 0 {
                comp[i]
            } else {
                merge.get(&door_of[i]).copied().unwrap_or(0)
            };
            room_no.get(&c).copied().unwrap_or(0)
        })
        .collect();
    let mut found = Rooms {
        rooms: Vec::new(),
        exits,
        cell: r.cell,
        origin: r.origin,
        w: r.w,
        h: r.h,
        label,
    };
    found.rooms = room_summaries(&found, room_no.len());
    fill_members(&mut found, actors, &walls);
    found
}

/// Area and centroid of each room from its labelled cells.
fn room_summaries(found: &Rooms, count: usize) -> Vec<Room> {
    let mut cells = vec![(0i64, 0i64, 0i64); count];
    for (i, l) in found.label.iter().enumerate().filter(|(_, l)| **l > 0) {
        let at = i32::try_from(i).unwrap_or(0);
        let c = &mut cells[ix(*l - 1)];
        *c = (
            c.0 + 1,
            c.1 + i64::from(at % found.w),
            c.2 + i64::from(at / found.w),
        );
    }
    let centroid = |sum: i64, n: i64, k: usize| {
        found.origin[k] + i32::try_from(sum / n.max(1)).unwrap_or(0) * found.cell + found.cell / 2
    };
    cells
        .into_iter()
        .enumerate()
        .map(|(i, (n, sx, sz))| Room {
            id: u32::try_from(i).unwrap_or(0) + 1,
            area_cm2: n * i64::from(found.cell) * i64::from(found.cell),
            center_cm: [centroid(sx, n, 0), centroid(sz, n, 1)],
            members: Vec::new(),
        })
        .collect()
}

/// Put every non-wall actor in the room its center sits in.
fn fill_members(found: &mut Rooms, actors: &[Actor], walls: &[Wall]) {
    let is_wall = |a: &Actor| {
        let (hx, hz) = (to_cm(a.sx).abs() / 2, to_cm(a.sz).abs() / 2);
        let (lo, hi) = (
            [to_cm(a.x) - hx, to_cm(a.z) - hz],
            [to_cm(a.x) + hx, to_cm(a.z) + hz],
        );
        walls.iter().any(|w| w.lo == lo && w.hi == hi)
    };
    for a in actors.iter().filter(|a| pos_known(a) && !is_wall(a)) {
        if let Some(r) = found.room_of(to_cm(a.x), to_cm(a.z)) {
            found.rooms[ix(r - 1)].members.push(a.id);
        }
    }
    for r in &mut found.rooms {
        r.members.sort_unstable();
    }
}

/// Calls `emit(index, run_len_cells)` for every free cell that sits in a run
/// of free cells bounded by solids on both ends along one line.
fn scan_runs(n: i32, solid: impl Fn(i32) -> bool, mut emit: impl FnMut(i32, i32)) {
    let mut start: Option<i32> = None;
    let mut seen_solid = false;
    for i in 0..=n {
        let s = i == n || solid(i);
        match (s, start) {
            (false, None) => start = Some(i),
            (true, Some(from)) => {
                if seen_solid && i < n {
                    for j in from..i {
                        emit(j, i - from);
                    }
                }
                start = None;
            }
            _ => {}
        }
        if s && i < n {
            seen_solid = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_core::Rng;

    fn wall(id: u32, lo: [i32; 2], hi: [i32; 2]) -> Actor {
        let m = |v: i32| v as f32 / 100.0;
        Actor {
            id,
            name: format!("wall_{id}"),
            x: m(lo[0] + hi[0]) / 2.0,
            y: 1.5,
            z: m(lo[1] + hi[1]) / 2.0,
            sx: m(hi[0] - lo[0]),
            sy: 3.0,
            sz: m(hi[1] - lo[1]),
            parent: None,
            physical: true,
            vx: 0.0,
            vz: 0.0,
            yaw: 0.0,
        }
    }

    fn prop(id: u32, x: i32, z: i32) -> Actor {
        let mut a = wall(id, [x - 20, z - 20], [x + 20, z + 20]);
        a.sy = 0.4;
        a
    }

    /// A row of `widths` rooms 400 cm deep. `doors[i]` is the gap in the wall
    /// after room i (0 = solid); the last wall and the outside have no door.
    fn row(widths: &[i32], doors: &[i32]) -> Vec<Actor> {
        let total: i32 = widths.iter().sum::<i32>() + 20 * (widths.len() as i32 + 1);
        let mut walls = vec![
            ([0, 0], [total, 20]),
            ([0, 420], [total, 440]),
            ([0, 20], [20, 420]),
        ];
        let mut x = 20;
        for (i, wd) in widths.iter().enumerate() {
            x += wd;
            let gap = if i + 1 < widths.len() { doors[i] } else { 0 };
            if gap == 0 {
                walls.push(([x, 20], [x + 20, 420]));
            } else {
                let side = (400 - gap) / 2;
                walls.push(([x, 20], [x + 20, 20 + side]));
                walls.push(([x, 420 - side], [x + 20, 420]));
            }
            x += 20;
        }
        walls
            .into_iter()
            .enumerate()
            .map(|(i, (lo, hi))| wall(i as u32 + 1, lo, hi))
            .collect()
    }

    #[test]
    fn a_two_room_house_has_two_rooms_and_one_exit() {
        let house = row(&[400, 400], &[120]);
        let r = find_rooms(&house, CAPSULE_CM);
        assert_eq!(r.rooms.len(), 2, "{:?}", r.rooms);
        assert_eq!(r.exits.len(), 1, "{:?}", r.exits);
        assert_eq!(
            (r.exits[0].a, r.exits[0].b, r.exits[0].width_cm),
            (1, 2, 120)
        );
        assert_eq!(r.room_of(220, 220), Some(1));
        assert_eq!(r.room_of(660, 220), Some(2));
        assert_eq!(r.room_of(-500, -500), None);
        assert_eq!(r.exits_of(1).len(), 1);
        // A gap narrower than the capsule is a wall: two sealed rooms, no exit.
        let sealed = row(&[400, 400], &[40]);
        let s = find_rooms(&sealed, CAPSULE_CM);
        assert_eq!((s.rooms.len(), s.exits.len()), (2, 0));
        // A wide opening is open floor: one room.
        let open = row(&[400, 400], &[300]);
        assert_eq!(find_rooms(&open, CAPSULE_CM).rooms.len(), 1);
    }

    #[test]
    fn members_are_listed_per_room_and_a_pillar_does_not_split_a_room() {
        let mut house = row(&[400, 400], &[120]);
        house.push(prop(100, 200, 200));
        house.push(prop(101, 640, 200));
        let mut pillar = wall(102, [100, 100], [140, 140]);
        pillar.sy = 3.0;
        house.push(pillar);
        let r = find_rooms(&house, CAPSULE_CM);
        assert_eq!(r.rooms.len(), 2);
        assert_eq!(r.exits.len(), 1);
        assert_eq!(r.rooms[0].members, vec![100]);
        assert_eq!(r.rooms[1].members, vec![101]);
    }

    /// Hand labels by construction: k rooms in a row, every second wall has a
    /// door, pillars scattered, the answer is k rooms and the door count.
    #[test]
    fn fifty_seeded_scenes_match_their_hand_labels() {
        let mut rng = Rng::new(0x2007);
        for case in 0..50 {
            let k = 1 + (rng.next_u32() % 4) as usize;
            let widths: Vec<i32> = (0..k)
                .map(|_| 300 + 20 * (rng.next_u32() % 10) as i32)
                .collect();
            let doors: Vec<i32> = (0..k)
                .map(|_| [0, 80, 120, 160][(rng.next_u32() % 4) as usize])
                .collect();
            let mut scene = row(&widths, &doors);
            let want_exits = doors[..k - 1].iter().filter(|g| **g > 0).count();
            let sealed = doors[..k - 1].iter().filter(|g| **g == 0).count();
            let mut x = 20;
            for (i, wd) in widths.iter().enumerate() {
                let (cx, cz) = (x + wd / 2, 100 + 20 * (rng.next_u32() % 10) as i32);
                scene.push(prop(200 + i as u32, cx - cx % 20, cz));
                x += wd + 20;
            }
            let rooms = find_rooms(&scene, CAPSULE_CM);
            let want_rooms = if k == 0 { 0 } else { k };
            assert_eq!(
                rooms.rooms.len(),
                want_rooms,
                "case {case} widths {widths:?} doors {doors:?}"
            );
            assert_eq!(rooms.exits.len(), want_exits, "case {case} sealed {sealed}");
            for r in &rooms.rooms {
                assert_eq!(r.members.len(), 1, "case {case}: one prop per room");
            }
            let mut shuffled = scene.clone();
            shuffled.reverse();
            assert_eq!(
                find_rooms(&shuffled, CAPSULE_CM),
                rooms,
                "case {case} order"
            );
        }
    }
}
