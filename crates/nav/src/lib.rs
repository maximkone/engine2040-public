//! nav: tiled dynamic tiles plus flow fields plus traversal links plus mesh bake.
//! Query homes: tiles (versioned tiles + flow steer), links (rebake for
//! destruction break sets), mesh (ludus proof bake), flow (steals proof).

pub mod bake;
mod cursor;
pub mod flow;
pub mod funnel;
pub mod links;
pub mod mesh;
pub mod orca;
pub mod reach;
pub mod rebake;
pub mod spline;
pub mod tiles;

#[cfg(test)]
mod wiring_tests {
    //! Crate-root wiring vectors: the roots a caller names (`bake`, `flow`,
    //! `links`, `mesh`, `rebake`, `tiles`) stay reachable and keep their
    //! headline contract. Each module owns its own behavior tests; this file
    //! owns only the root re-export contract, the way `engine-scene`'s
    //! `wiring_tests` and `engine-endpoint`'s `lib_std_wiring` do.
    //!
    //! Why here: `crates/nav/src/lib.rs` changed 16 times in 30 days yet held
    //! zero `#[test]`, so a dropped `pub mod` line compiled the crate but
    //! broke every caller at its `use` line. The layout guard allows only the
    //! pre-existing `crates/nav/tests/tiles.rs` binary, so the vectors live
    //! beside the root; `crate::x` is the same path an external
    //! `engine_nav::x` caller takes. Determinism: integer and slice compares
    //! over pure bakes, no clock, no IO. Errors are panics via asserts.

    use crate::{bake, flow, links, mesh, rebake, tiles};

    /// `bake` + `flow`: one offline bake answers a runtime downhill step, and
    /// the same grid bakes the same distance field and hash twice.
    #[test]
    fn root_bake_and_flow_reach_the_goal() {
        let blocked = vec![false; 5 * 5];
        let goal = bake::Goal { x: 4, y: 4 };
        let mut nav = bake::BakedNav::bake(&blocked, 5, 5, &bake::TileEdge(2));
        nav.bake_goal(&goal);
        assert!(nav.validate().is_empty(), "a good bake validates clean");
        assert_eq!(
            nav.direction(&goal, &bake::Cell { x: 0, y: 0 }),
            Some(bake::Cell { x: 0, y: 1 }),
            "runtime reads the cached field through the root"
        );

        let field = flow::bake_flow(&blocked, 5, 5, (4, 4));
        assert_eq!(field[0], 8, "manhattan distance on an open grid");
        assert_eq!(flow::follow(&field, 5, 5, (0, 0)), Some((0, 1)));
        assert_eq!(
            flow::flow_hash(&field),
            flow::flow_hash(&flow::bake_flow(&blocked, 5, 5, (4, 4))),
            "same grid hashes the same through the root"
        );
        let walls = flow::seeded_blocked(2040, 8, 8, 250);
        assert_eq!(walls, flow::seeded_blocked(2040, 8, 8, 250));
        assert!(walls.iter().any(|cell| *cell), "density 250 walls cells");
    }

    /// `mesh` + `tiles` + `rebake`: a baked floor survives its versioned
    /// bytes, and a mesh-tile rebake bumps exactly the tile it names.
    #[test]
    fn root_mesh_tiles_and_rebake_bump_one_tile() {
        let floor = [
            [[0.0, 0.0, 0.0], [0.0, 0.0, 1.0], [1.0, 0.0, 1.0]],
            [[0.0, 0.0, 0.0], [1.0, 0.0, 1.0], [1.0, 0.0, 0.0]],
        ];
        assert_eq!(mesh::MESH_VERSION, 1, "mesh byte version pinned");
        assert_eq!(mesh::MAX_TRIS, 65_536, "triangle bound pinned");
        let baked = mesh::NavMesh::bake(&floor, &mesh::BakeConfig::default()).expect("floor bakes");
        assert!(baked.polygon_count() > 0, "a walkable floor bakes polygons");
        let bytes = baked.to_bytes();
        let back = mesh::NavMesh::from_bytes(&bytes).expect("baked bytes decode");
        assert_eq!(back.polygon_count(), baked.polygon_count());
        assert!(
            mesh::NavMesh::from_bytes(&bytes[..bytes.len() - 1]).is_err(),
            "a truncated blob refuses"
        );

        assert_eq!(tiles::TICKS_PER_SECOND, engine_core::TICK_HZ as u32);
        assert_eq!(tiles::MAX_TILES, 65_536, "tile bound pinned");
        let mut registry = tiles::TileRegistry::new(16).expect("registry builds");
        assert_eq!(
            rebake::rebake_mesh_tile(&mut registry, &baked, tiles::TileId(0)),
            Ok(1),
            "the first rebake of tile 0 lands at version 1"
        );
        assert_eq!(registry.version(tiles::TileId(0)), Ok(1));
        assert_eq!(
            rebake::rebake_mesh_tile(&mut registry, &baked, tiles::TileId(9)),
            Err(tiles::TileError::BadTile { tile: 9 }),
            "a tile past the mesh refuses and bumps nothing"
        );
    }

    /// `links`: the bounds stay pinned and a dirty tile names exactly the
    /// links sitting on it.
    #[test]
    fn root_links_name_dirty_tiles() {
        assert_eq!(links::MAX_LINKS, 4096, "link bound pinned");
        assert_eq!(links::MM_PER_M, 1000.0, "meter unit pinned");
        assert_eq!(links::EDGE_RANGE_M, 1000.0, "edge range pinned");
        assert_eq!(
            links::rebake_links(&[], &[]),
            Ok(Vec::<links::LinkId>::new()),
            "no links means no ids"
        );
        let authored = [
            links::TraversalLink {
                id: links::LinkId(7),
                kind: links::LinkKind::Vault,
                edge_m: [1.0, 2.0],
                tile: tiles::TileId(1),
            },
            links::TraversalLink {
                id: links::LinkId(3),
                kind: links::LinkKind::Drop,
                edge_m: [3.0, 4.0],
                tile: tiles::TileId(2),
            },
        ];
        assert_eq!(
            links::rebake_links(&authored, &[tiles::TileId(1)]),
            Ok(vec![links::LinkId(7)]),
            "only the link on the dirty tile is named"
        );
        assert_eq!(
            links::rebake_links(&authored, &[tiles::TileId(0)]),
            Ok(Vec::<links::LinkId>::new()),
            "a clean tile set names nothing"
        );
    }
}
