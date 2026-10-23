//! world_load: a scene document revision becomes the match world.
//!
//! Consumer: `play.run` with `scene_rev`. Producer of data: the authoring
//! [`Document`] (the hierarchy the door edits through `tx.apply` and
//! `tx.undo`). Output: one [`MatchWorld`] of floor boxes the spine step
//! plays around. Units: meters, radians; one box per node.
//!
//! Adapter: [`blocks`] maps rows from the one scene adapter
//! ([`engine_editor::spatial_view`]): a node with a `collider` component blocks
//! by that shape times its world scale, a sensor never blocks, a node that
//! carries other components but no collider is decoration and is skipped, and
//! a bare node (pose only) blocks by its world scale (a unit-scale node is a
//! 1 m cube, a 2 m wall is scale `[2, 2, 0.4]`), which is the same box
//! `scene.spatial` reports as the labelled default. Every shape lands as one
//! floor box (the match world holds oriented boxes only). Nodes wholly above
//! the fighter height or wholly below the floor are not obstacles.
//!
//! Determinism: rows come in id-text order and the world sorts its boxes
//! canonically, so insertion order never moves a hash. The axis comes from the
//! rotation quaternion algebraically, no `sin` or `cos`.
//! Errors: typed [`McpToolError`] codes naming the legal values.

use engine_arena::world::{MatchWorld, Obstacle, MAX_OBSTACLES, MAX_OBSTACLE_HALF_M};
use engine_arena::FIGHTER_HEIGHT_M;
use engine_editor::commands::Document;
use engine_editor::spatial_view;

use crate::mcp::McpToolError;

/// One solid block read from the scene, in match units.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Block {
    /// Center on the floor plane `[x, z]`, meters.
    pub center: [f32; 2],
    /// Full size along the box axes `[x, z]`, meters.
    pub size: [f32; 2],
    /// Box axis `[cos yaw, sin yaw]` (any length, normalized by the world).
    pub axis: [f32; 2],
}

/// Read the document's solid blocks. The only function that knows how the
/// scene says "this node blocks fighters" (see the module doc).
pub fn blocks(doc: &Document) -> Vec<Block> {
    let mut out = Vec::new();
    for row in spatial_view::rows_of(doc).rows {
        // Sensor and decoration rows never block; the height filter below is
        // this consumer's own (a fighter walks under a high box).
        let Some((footprint, axis)) = spatial_view::block_of(&row) else {
            continue;
        };
        let py = row.world.position[1];
        let fy = row.size[1];
        // Vertical overlap with a standing fighter: [0, FIGHTER_HEIGHT_M].
        let (bottom, top) = (py - fy * 0.5, py + fy * 0.5);
        if top <= 0.0 || bottom >= FIGHTER_HEIGHT_M {
            continue;
        }
        out.push(Block {
            center: [row.world.position[0], row.world.position[2]],
            size: footprint,
            axis,
        });
    }
    out
}

/// Build the match world of a document. Refuses a scene the match cannot
/// hold (too many blocks, a block larger than the arena can use, a flat or
/// non-finite block) with a code that names the legal limit.
pub fn match_world(doc: &Document) -> Result<MatchWorld, McpToolError> {
    let blocks = blocks(doc);
    if blocks.len() > MAX_OBSTACLES {
        return Err(McpToolError::new(
            "scene_too_big",
            format!(
                "the scene holds {} solid nodes; a match plays at most {MAX_OBSTACLES} \
                 (remove nodes or move them above {FIGHTER_HEIGHT_M} m)",
                blocks.len()
            ),
        ));
    }
    let obstacles = blocks
        .iter()
        .map(|block| Obstacle {
            center: block.center,
            half: [block.size[0] * 0.5, block.size[1] * 0.5],
            axis: block.axis,
        })
        .collect();
    MatchWorld::try_new(obstacles).map_err(|error| {
        McpToolError::new(
            "bad_scene",
            format!(
                "the scene cannot be played: {error}; every solid node needs a finite pose \
                 and a size in (0, {}] meters on x and z",
                MAX_OBSTACLE_HALF_M * 2.0
            ),
        )
    })
}

/// The world of `scene_rev`, which must be the document's current revision.
/// Revisions never repeat (undo publishes a new one), so a stale number means
/// the scene moved since the caller read it; the refusal names the current
/// revision so the caller can read again and retry.
pub fn world_for_revision(doc: &Document, scene_rev: u64) -> Result<MatchWorld, McpToolError> {
    if scene_rev != doc.revision {
        return Err(McpToolError::new(
            "stale_scene_rev",
            format!(
                "scene_rev {scene_rev} is not the current revision {}; read the scene \
                 (scene.read) and submit its revision",
                doc.revision
            ),
        ));
    }
    match_world(doc)
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_scene::component::{ColliderShape, Component};
    use engine_scene::ids::ObjectId;
    use engine_scene::transform::Transform;

    fn doc_with(nodes: &[([f32; 3], [f32; 3])]) -> Document {
        let mut doc = Document::new();
        for (pos, scale) in nodes {
            doc.hierarchy
                .insert(
                    ObjectId::new(),
                    ObjectId::NIL,
                    Transform::new(*pos, [0.0, 0.0, 0.0, 1.0], *scale),
                )
                .expect("inserts");
        }
        doc
    }

    #[test]
    fn empty_document_is_the_empty_world() {
        let world = match_world(&Document::new()).expect("empty scene plays");
        assert!(world.is_empty());
    }

    #[test]
    fn a_node_is_a_block_sized_by_its_scale() {
        let doc = doc_with(&[([3.0, 1.0, -2.0], [2.0, 2.0, 0.4])]);
        let world = match_world(&doc).expect("loads");
        let [wall] = world.obstacles() else {
            panic!("one obstacle expected");
        };
        assert_eq!(wall.center, [3.0, -2.0]);
        assert_eq!(wall.half, [1.0, 0.2]);
        assert_eq!(wall.axis, [1.0, 0.0]);
    }

    fn set(doc: &mut Document, id: ObjectId, component: Component) {
        doc.components
            .entry(id.to_string())
            .or_default()
            .insert(component.kind(), component);
    }

    #[test]
    fn a_collider_sets_the_shape_and_decoration_nodes_are_skipped() {
        use engine_scene::component::{AssetName, Collider, Mesh};
        let mut doc = Document::new();
        let wall = ObjectId::new();
        let prop = ObjectId::new();
        let sensor = ObjectId::new();
        for (id, x) in [(wall, 3.0), (prop, -3.0), (sensor, 6.0)] {
            doc.hierarchy
                .insert(
                    id,
                    ObjectId::NIL,
                    Transform::new([x, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0]),
                )
                .expect("inserts");
        }
        set(
            &mut doc,
            wall,
            Component::Collider(Collider {
                shape: ColliderShape::Box,
                half: [1.0, 1.0, 0.2],
                sensor: false,
            }),
        );
        // A mesh-only node is decoration: it no longer blocks.
        set(
            &mut doc,
            prop,
            Component::Mesh(Mesh {
                asset: AssetName::try_from("crate").expect("name"),
                cast_shadows: true,
            }),
        );
        set(
            &mut doc,
            sensor,
            Component::Collider(Collider {
                shape: ColliderShape::Sphere,
                half: [1.0, 1.0, 1.0],
                sensor: true,
            }),
        );
        let world = match_world(&doc).expect("loads");
        let [only] = world.obstacles() else {
            panic!("only the wall blocks, got {:?}", world.obstacles());
        };
        assert_eq!(only.center, [3.0, 0.0]);
        assert_eq!(only.half, [1.0, 0.2]);
    }

    #[test]
    fn a_collider_shape_scales_with_the_node() {
        use engine_scene::component::Collider;
        let mut doc = Document::new();
        let id = ObjectId::new();
        doc.hierarchy
            .insert(
                id,
                ObjectId::NIL,
                Transform::new([0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0], [2.0, 1.0, 2.0]),
            )
            .expect("inserts");
        set(
            &mut doc,
            id,
            Component::Collider(Collider {
                shape: ColliderShape::Capsule,
                half: [0.5, 0.5, 0.0],
                sensor: false,
            }),
        );
        let world = match_world(&doc).expect("loads");
        let [capsule] = world.obstacles() else {
            panic!("one obstacle expected");
        };
        // Radius 0.5 times the larger horizontal scale 2 gives a 2 m footprint.
        assert_eq!(capsule.half, [1.0, 1.0]);
    }

    #[test]
    fn nodes_above_the_fighters_or_under_the_floor_do_not_block() {
        let doc = doc_with(&[
            ([0.0, 5.0, 0.0], [1.0, 1.0, 1.0]),
            ([1.0, -3.0, 1.0], [1.0, 1.0, 1.0]),
        ]);
        assert!(match_world(&doc).expect("loads").is_empty());
    }

    /// One document, two readers: the door's `scene.spatial` snapshot and the
    /// play path's blocks describe the same boxes, so a door answer about an
    /// overlap is an answer about the match a player gets. The wall keeps its
    /// collider extents (4.0 x 0.2 footprint), a bare node its world scale, and
    /// the sensor, decoration and high node block neither way.
    #[test]
    fn door_and_play_measure_one_document_the_same_way() {
        use crate::mcp::McpServer;
        use crate::scene_tools::{snapshot, SceneSpatialArgs, SceneSpatialTool};
        use engine_editor::capability::Capability;
        use engine_editor::commands::Journal;
        use engine_scene::component::{AssetName, Collider, Mesh};
        use rmcp::handler::server::router::tool::SyncTool;

        let mut doc = Document::new();
        let wall = ObjectId::new();
        let crate_ = ObjectId::new();
        let sphere = ObjectId::new();
        let sensor = ObjectId::new();
        let decor = ObjectId::new();
        let high = ObjectId::new();
        for (id, pos, scale) in [
            (wall, [0.0, 1.0, 0.0], [1.0, 1.0, 1.0]),
            (crate_, [1.0, 0.5, 0.5], [1.0, 1.0, 1.0]),
            (sphere, [-5.0, 1.0, 1.0], [2.0, 2.0, 2.0]),
            (sensor, [0.0, 1.0, 3.0], [1.0, 1.0, 1.0]),
            (decor, [0.0, 1.0, -3.0], [1.0, 1.0, 1.0]),
            (high, [8.0, 5.0, 8.0], [1.0, 1.0, 1.0]),
        ] {
            doc.hierarchy
                .insert(
                    id,
                    ObjectId::NIL,
                    Transform::new(pos, [0.0, 0.0, 0.0, 1.0], scale),
                )
                .expect("inserts");
        }
        set(
            &mut doc,
            wall,
            Component::Collider(Collider {
                shape: ColliderShape::Box,
                half: [2.0, 1.5, 0.1],
                sensor: false,
            }),
        );
        set(
            &mut doc,
            sphere,
            Component::Collider(Collider {
                shape: ColliderShape::Sphere,
                half: [0.5, 0.5, 0.5],
                sensor: false,
            }),
        );
        set(
            &mut doc,
            sensor,
            Component::Collider(Collider {
                shape: ColliderShape::Box,
                half: [1.0, 1.0, 1.0],
                sensor: true,
            }),
        );
        set(
            &mut doc,
            decor,
            Component::Mesh(Mesh {
                asset: AssetName::try_from("lamp").expect("name"),
                cast_shadows: true,
            }),
        );

        let snap = snapshot(&doc);
        let play = blocks(&doc);
        // The wall reads by its collider, not as a 1 m cube.
        let wall_block = play
            .iter()
            .find(|block| block.center == [0.0, 0.0])
            .expect("the wall blocks");
        assert_eq!(wall_block.size, [4.0, 0.2], "collider extents, not default");
        assert_eq!(play.len(), 3, "wall, bare crate and sphere: {play:?}");
        assert_eq!(snap.actors.len(), 6, "the door sees every node");
        for (actor, id) in snap.actors.iter().zip(&snap.ids) {
            let found: Vec<&Block> = play
                .iter()
                .filter(|block| block.center == [actor.x, actor.z])
                .collect();
            match found.as_slice() {
                [one] => {
                    assert!(
                        [wall, crate_, sphere].contains(id),
                        "{id}: a non-solid row blocks"
                    );
                    assert_eq!(one.size, [actor.sx, actor.sz], "{id}: boxes disagree");
                    assert_eq!(one.axis, [1.0, 0.0], "{id}: unrotated axis");
                }
                [] => assert!(
                    ![wall, crate_, sphere].contains(id),
                    "{id}: a solid row does not block"
                ),
                many => panic!("{id}: one block per solid row, got {}", many.len()),
            }
        }

        let server = McpServer::with_document(doc, Journal::new(), 7, Capability::unrestricted(7));
        let door = SceneSpatialTool::invoke(&server, SceneSpatialArgs::default())
            .expect("scene.spatial reads");
        // Same verdict: the crate inside the wall is the one overlap either way.
        assert_eq!(door.verdict, "red", "the crate sits inside the wall");
        assert_eq!(door.size_source.collider, 3, "wall, sphere, sensor");
        assert_eq!(door.size_source.default, 3, "crate, lamp, high node");
        assert_eq!(door.default_sized.len(), 3, "{:?}", door.default_sized);
        for line in &door.default_sized {
            assert!(line.contains("default size"), "{line}");
        }
        assert!(
            door.default_sized
                .iter()
                .any(|line| line.starts_with("lamp_1 ")),
            "the bare nodes are named: {:?}",
            door.default_sized
        );
    }

    #[test]
    fn rotation_gives_the_box_axis_without_trig() {
        // 90 degrees about +y: q = (0, sin 45, 0, cos 45).
        let half_sqrt2 = 0.5f32.sqrt();
        let mut doc = Document::new();
        doc.hierarchy
            .insert(
                ObjectId::new(),
                ObjectId::NIL,
                Transform::new(
                    [0.0, 1.0, 0.0],
                    [0.0, half_sqrt2, 0.0, half_sqrt2],
                    [2.0, 2.0, 0.4],
                ),
            )
            .expect("inserts");
        let world = match_world(&doc).expect("loads");
        let axis = world.obstacles()[0].axis;
        assert!(
            axis[0].abs() < 1.0e-5 && (axis[1] - 1.0).abs() < 1.0e-5,
            "{axis:?}"
        );
    }

    #[test]
    fn scene_order_does_not_move_the_world() {
        let a = ([2.0, 1.0, 0.0], [1.0, 1.0, 1.0]);
        let b = ([-4.0, 1.0, 3.0], [2.0, 2.0, 0.5]);
        let ab = match_world(&doc_with(&[a, b])).expect("loads");
        let ba = match_world(&doc_with(&[b, a])).expect("loads");
        assert_eq!(ab.digest(), ba.digest());
    }

    #[test]
    fn stale_revision_refuses_with_the_current_one() {
        let doc = Document::new();
        assert!(world_for_revision(&doc, 0).is_ok());
        let err = world_for_revision(&doc, 4).expect_err("not current");
        assert_eq!(err.code, "stale_scene_rev");
        assert!(err.message.contains("current revision 0"));
    }

    #[test]
    fn too_many_solid_nodes_refuse_loudly() {
        let nodes: Vec<_> = (0..=MAX_OBSTACLES)
            .map(|i| ([i as f32, 1.0, 0.0], [0.5, 0.5, 0.5]))
            .collect();
        let err = match_world(&doc_with(&nodes)).expect_err("too many");
        assert_eq!(err.code, "scene_too_big");
    }

    /// A degenerate scale lane is the labelled default box, not a refusal: the
    /// door reports that same box, so play must play it.
    #[test]
    fn a_degenerate_scale_lane_plays_as_the_default_box() {
        let doc = doc_with(&[([0.0, 1.0, 0.0], [0.0, 1.0, 1.0])]);
        let world = match_world(&doc).expect("a zero lane is the default box");
        let [block] = world.obstacles() else {
            panic!("one block expected");
        };
        assert_eq!(block.half, [0.5, 0.5]);
    }

    #[test]
    fn a_node_too_wide_for_the_arena_refuses_as_a_bad_scene() {
        let doc = doc_with(&[([0.0, 1.0, 0.0], [MAX_OBSTACLE_HALF_M * 4.0, 1.0, 1.0])]);
        let err = match_world(&doc).expect_err("past the arena limit");
        assert_eq!(err.code, "bad_scene");
    }
}
