//! AD20 adapter: a baked [`NavMesh`] to the two spatial views, so actors get
//! their real nav polygon instead of one shared polygon 0.
//!
//! Consumer: the endpoint door (`scene.query reachable`, `scene.spatial` bake
//! checks) reads [`reach_view`] and [`nav_view`]; `engine-spatial` stays
//! nav-free, this crate already depends on it. Data owner: the caller passes
//! `(actor id, world position in meters)` rows; the mesh owns the polygons.
//! Units: meters in; the nav view uses the spatial 100 cm tile. Determinism:
//! polygon ids are dense and ordered, outputs are `BTreeMap`s, so the same
//! mesh and rows give the same views. Failure mode: an actor on no walkable
//! cell is left out of `at`, so `reachable` answers `OffMesh`, never a guess.

use crate::mesh::NavMesh;
use engine_spatial::sight::ReachView;
use engine_spatial::{NavView, Tile};

/// Polygon per actor plus polygon adjacency, for `sight::reachable`.
pub fn reach_view(mesh: &NavMesh, actors: &[(u32, [f32; 3])]) -> ReachView {
    let mut view = ReachView::default();
    for poly in mesh.polygons() {
        let neighbors = poly.neighbors().iter().map(|n| n.0).collect();
        view.adj.insert(poly.id().0, neighbors);
    }
    for (id, position) in actors {
        if let Ok(poly) = mesh.polygon_at(*position) {
            view.at.insert(*id, poly.0);
        }
    }
    view
}

/// Tile bake view for the spatial nav checks: every 100 cm tile holding a
/// walkable cell center is walkable; `spawn_id` roots the reach search.
pub fn nav_view(mesh: &NavMesh, spawn_id: u32) -> NavView {
    let mut view = NavView {
        spawn_id,
        ..NavView::default()
    };
    for poly in mesh.polygons() {
        for cell in poly.cells() {
            let [x, z] = mesh.cell_center_m(*cell);
            let tile = (
                ((x * 100.0).round() as i32).div_euclid(100),
                ((z * 100.0).round() as i32).div_euclid(100),
            );
            view.tiles.insert(tile, Tile { walk: true });
        }
    }
    view
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh::BakeConfig;
    use engine_spatial::sight::{reachable, Reach};
    use engine_spatial::{run_spatial, Actor};
    use std::collections::BTreeSet;

    type Tri = [[f32; 3]; 3];
    type Rows = Vec<(u32, [f32; 3])>;

    /// Upward-facing floor quad `[x0, x1] x [z0, z1]` as two triangles.
    fn quad(x0: f32, z0: f32, x1: f32, z1: f32) -> [Tri; 2] {
        [
            [[x0, 0.0, z0], [x0, 0.0, z1], [x1, 0.0, z0]],
            [[x1, 0.0, z0], [x0, 0.0, z1], [x1, 0.0, z1]],
        ]
    }

    fn actor(id: u32, pos: [f32; 3]) -> Actor {
        Actor {
            id,
            name: format!("a{id}"),
            x: pos[0],
            y: 0.5,
            z: pos[2],
            sx: 1.0,
            sy: 1.0,
            sz: 1.0,
            parent: None,
            physical: true,
            vx: 0.0,
            vz: 0.0,
            yaw: 0.0,
        }
    }

    #[test]
    fn reach_door_two_rooms_blocked_across_the_wall_until_a_door_joins_them() {
        let west = quad(0.0, 0.0, 4.0, 4.0);
        let east = quad(6.0, 0.0, 10.0, 4.0);
        let doorway = quad(4.0, 1.0, 6.0, 3.0);
        let rows = [
            (1, [2.0, 0.0, 2.0]),
            (2, [8.0, 0.0, 2.0]),
            (3, [-9.0, 0.0, -9.0]),
        ];
        let cfg = BakeConfig::default();

        let walled: Vec<Tri> = west.iter().chain(&east).copied().collect();
        let mesh = NavMesh::bake(&walled, &cfg).expect("bakes");
        let view = reach_view(&mesh, &rows);
        assert_ne!(view.at[&1], view.at[&2], "each room has its own polygon");
        assert_eq!(reachable(&view, 1, 2), Reach::Blocked);
        assert_eq!(
            reachable(&view, 1, 3),
            Reach::OffMesh,
            "off the mesh is not reachable"
        );

        let joined: Vec<Tri> = walled.iter().chain(&doorway).copied().collect();
        let mesh = NavMesh::bake(&joined, &cfg).expect("bakes");
        let view = reach_view(&mesh, &rows);
        assert_eq!(view.at[&1], view.at[&2], "a door merges the polygons");
        assert_eq!(reachable(&view, 1, 2), Reach::Reachable);
    }

    /// 3 x 3 lattice of 4 m rooms, 6 m pitch; random rooms and doors.
    /// Returns floor triangles, one actor per room and the connected set.
    fn lattice(seed: u64) -> (Vec<Tri>, Rows, BTreeSet<u32>) {
        let mut state = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
        let mut roll = move |n: u64| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state % n
        };
        let present: Vec<bool> = (0..9).map(|_| roll(10) < 7).collect();
        let at = |i: usize| ((i % 3) as f32 * 6.0, (i / 3) as f32 * 6.0);
        let mut tris = Vec::new();
        let mut rows = Vec::new();
        for i in (0..9).filter(|i| present[*i]) {
            let (x, z) = at(i);
            tris.extend(quad(x, z, x + 4.0, z + 4.0));
            rows.push((10 + i as u32, [x + 2.0, 0.0, z + 2.0]));
        }
        let mut links: Vec<(usize, usize)> = Vec::new();
        for i in 0..9 {
            for (j, east) in [(i + 1, true), (i + 3, false)] {
                let edge = if east { i % 3 < 2 } else { j < 9 };
                if edge && present[i] && present[j] && roll(2) == 0 {
                    let (x, z) = at(i);
                    tris.extend(if east {
                        quad(x + 4.0, z + 1.0, x + 6.0, z + 3.0)
                    } else {
                        quad(x + 1.0, z + 4.0, x + 3.0, z + 6.0)
                    });
                    links.push((i, j));
                }
            }
        }
        let first = (0..9).find(|i| present[*i]);
        let mut seen: BTreeSet<usize> = first.into_iter().collect();
        let mut grew = true;
        while grew {
            grew = false;
            for (a, b) in &links {
                if seen.contains(a) != seen.contains(b) {
                    seen.extend([*a, *b]);
                    grew = true;
                }
            }
        }
        (
            tris,
            rows,
            seen.into_iter().map(|i| 10 + i as u32).collect(),
        )
    }

    #[test]
    fn reach_door_mesh_and_tiles_agree_on_20_seeded_layouts() {
        let mut checked = 0;
        for seed in 1..=20u64 {
            let (tris, rows, connected) = lattice(seed);
            if rows.len() < 2 {
                continue;
            }
            let mesh = NavMesh::bake(&tris, &BakeConfig::default()).expect("bakes");
            let spawn = rows[0].0;
            let reach = reach_view(&mesh, &rows);
            let by_mesh: BTreeSet<u32> = rows
                .iter()
                .filter(|(id, _)| reachable(&reach, spawn, *id) == Reach::Reachable)
                .map(|(id, _)| *id)
                .collect();
            assert_eq!(by_mesh, connected, "seed {seed}: polygons vs lattice truth");

            let actors: Vec<Actor> = rows.iter().map(|(id, p)| actor(*id, *p)).collect();
            let report = run_spatial(&actors, &["navConnect"], Some(&nav_view(&mesh, spawn)))
                .expect("legal");
            let by_tiles: BTreeSet<u32> = rows
                .iter()
                .map(|(id, _)| *id)
                .filter(|id| {
                    !report
                        .findings
                        .iter()
                        .any(|f| f.check == "navConnect" && f.a == *id)
                })
                .collect();
            assert_eq!(by_tiles, by_mesh, "seed {seed}: tiles vs polygons");
            checked += 1;
        }
        assert!(checked >= 15, "only {checked} of 20 layouts had two rooms");
    }
}
