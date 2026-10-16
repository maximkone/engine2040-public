//! world_load: a scene document revision becomes the match world.
//!
//! Consumer: `play.run` with `scene_rev`. Producer of data: the authoring
//! [`Document`] (the hierarchy the door edits through `tx.apply` and
//! `tx.undo`). Output: one [`MatchWorld`] of floor boxes the spine step
//! plays around. Units: meters, radians; one box per node.
//!
//! Adapter: [`blocks`] is the single place that reads the scene schema. A node
//! with a `collider` component blocks by that shape (box half extents, sphere
//! radius or capsule, times the node's world scale; sensors never block). A
//! node that carries other components but no collider is decoration and is
//! skipped. A bare node (pose only) keeps the unit cube convention the
//! spatial tools use: the box is centered at the node's world position and
//! sized by its world scale (a unit-scale node is a 1 m cube, a 2 m wall is
//! scale `[2, 2, 0.4]`). Every shape lands as one floor box (the match world
//! holds oriented boxes only). Nodes wholly above the fighter height or
//! wholly below the floor are not obstacles.
//!
//! Determinism: nodes visit in hierarchy propagation order; the world sorts
//! its boxes canonically, so insertion order never moves a hash. The axis
//! comes from the rotation quaternion algebraically, no `sin` or `cos`.
//! Errors: typed [`McpToolError`] codes naming the legal values.

use engine_arena::world::{MatchWorld, Obstacle, MAX_OBSTACLES, MAX_OBSTACLE_HALF_M};
use engine_arena::FIGHTER_HEIGHT_M;
use engine_editor::commands::Document;
use engine_scene::component::{ColliderShape, Component, ComponentKind};

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
    for (id, world) in doc.hierarchy.world_all() {
        let [px, py, pz] = world.position;
        let [sx, sy, sz] = world.scale;
        if !(px.is_finite() && py.is_finite() && pz.is_finite()) {
            continue;
        }
        let (sx, sy, sz) = (sx.abs(), sy.abs(), sz.abs());
        // Footprint and height: a collider component gives the shape (scaled
        // by the node's world scale); a node with other components but no
        // collider is decoration and never blocks; a bare node keeps the
        // unit-cube convention.
        let own = doc.components.get(&id.to_string());
        let (fx, fy, fz) = match own.and_then(|slot| slot.get(&ComponentKind::Collider)) {
            Some(Component::Collider(collider)) => {
                if collider.sensor {
                    continue;
                }
                let [hx, hy, hz] = collider.half;
                match collider.shape {
                    ColliderShape::Box => (2.0 * hx * sx, 2.0 * hy * sy, 2.0 * hz * sz),
                    ColliderShape::Sphere => {
                        let r = hx * sx.max(sy).max(sz);
                        (2.0 * r, 2.0 * r, 2.0 * r)
                    }
                    ColliderShape::Capsule => {
                        let r = hx * sx.max(sz);
                        (2.0 * r, 2.0 * (hy + hx) * sy, 2.0 * r)
                    }
                }
            }
            _ if own.is_some_and(|slot| !slot.is_empty()) => continue,
            _ => (sx, sy, sz),
        };
        // Vertical overlap with a standing fighter: [0, FIGHTER_HEIGHT_M].
        let (bottom, top) = (py - fy * 0.5, py + fy * 0.5);
        if top <= 0.0 || bottom >= FIGHTER_HEIGHT_M {
            continue;
        }
        let (sx, sz) = (fx, fz);
        let [qx, qy, qz, qw] = world.rotation;
        // Yaw axis from the quaternion without trigonometry:
        // cos ~ 1 - 2(y^2 + z^2), sin ~ 2(w y + x z) (scaled by the same
        // factor when the quaternion is not exactly unit length).
        let cos = 1.0 - 2.0 * (qy * qy + qz * qz);
        let sin = 2.0 * (qw * qy + qx * qz);
        let axis = if cos == 0.0 && sin == 0.0 {
            [1.0, 0.0]
        } else {
            [cos, sin]
        };
        out.push(Block {
            center: [px, pz],
            size: [sx, sz],
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

    #[test]
    fn a_flat_node_refuses_as_a_bad_scene() {
        let doc = doc_with(&[([0.0, 1.0, 0.0], [0.0, 1.0, 1.0])]);
        let err = match_world(&doc).expect_err("zero width");
        assert_eq!(err.code, "bad_scene");
    }
}
