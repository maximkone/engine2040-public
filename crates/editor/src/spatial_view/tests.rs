//! Adapter proofs: real extents change an answer, play and the door agree, a
//! bare node is labelled, and the mapping is deterministic.

use engine_scene::component::{AssetName, Collider, ColliderShape, Component, Mesh};
use engine_scene::ids::ObjectId;
use engine_scene::transform::Transform;

use super::*;
use crate::commands::{apply, Batch, Journal, Op};

/// One inserted node: pose plus an optional component.
#[derive(Clone, Copy)]
struct Node {
    id: ObjectId,
    local: Transform,
    component: Option<Component>,
    key: u64,
}

fn at(x: f32, y: f32, z: f32, scale: [f32; 3]) -> Transform {
    Transform::new([x, y, z], [0.0, 0.0, 0.0, 1.0], scale)
}

fn collider(half: [f32; 3], shape: ColliderShape, sensor: bool) -> Component {
    Component::Collider(Collider {
        shape,
        half,
        sensor,
    })
}

/// A box collider `half` meters on each side, no sensor.
fn solid(half: [f32; 3]) -> Component {
    collider(half, ColliderShape::Box, false)
}

/// The wall from the audit: half extents `[2, 1.5, 0.1]`, so 4 x 3 x 0.2 m.
fn wall_box() -> Component {
    solid([2.0, 1.5, 0.1])
}

/// Build a document by applying one insert (plus component) per node.
fn doc_of(nodes: &[Node]) -> Document {
    let mut doc = Document::new();
    let mut journal = Journal::new();
    for node in nodes {
        let mut ops = vec![Op::Insert {
            id: node.id,
            parent: ObjectId::NIL,
            local: node.local,
        }];
        if let Some(component) = node.component {
            ops.push(Op::SetComponent {
                id: node.id,
                component,
            });
        }
        let batch = Batch {
            ops,
            base_revision: doc.revision,
            idempotency_key: node.key,
            provenance: None,
        };
        apply(&mut doc, &mut journal, &batch).expect("fixture applies");
    }
    doc
}

fn id(n: u64) -> ObjectId {
    ObjectId::parse(&format!("00000000-0000-0000-0000-{n:012}")).expect("fixed id parses")
}

fn node(id: ObjectId, local: Transform, component: Option<Component>, key: u64) -> Node {
    Node {
        id,
        local,
        component,
        key,
    }
}

fn overlap_pairs(doc: &Document) -> usize {
    let (actors, _) = actors(doc);
    engine_spatial::run_spatial(&actors, &["overlap"], None)
        .expect("overlap is legal")
        .count("overlap")
}

/// The fallback rows the old adapter posted: every node a 1 m cube on its
/// position, whatever the collider says. Kept here only as the before-picture
/// the fix must beat.
fn unit_cube_rows(doc: &Document) -> Vec<Actor> {
    let mut out = Vec::new();
    for (index, row) in rows_of(doc).rows.iter().enumerate() {
        out.push(Actor {
            id: u32::try_from(index).unwrap_or(u32::MAX),
            name: format!("prop_{index:04}"),
            x: row.world.position[0],
            y: row.world.position[1],
            z: row.world.position[2],
            sx: 1.0,
            sy: 1.0,
            sz: 1.0,
            parent: None,
            physical: true,
            vx: 0.0,
            vz: 0.0,
            yaw: yaw_of(row.world.rotation),
        });
    }
    out
}

/// The accept test of AD-07: a wall collider `[2, 1.5, 0.1]` with a crate
/// inside it overlaps. The old 1 m cube said it did not.
#[test]
fn wall_collider_with_a_crate_inside_reports_overlap() {
    let wall = id(1);
    let crate_ = id(2);
    let doc = doc_of(&[
        node(wall, at(0.0, 0.0, 0.0, [1.0; 3]), Some(wall_box()), 1),
        // 1.6 m east of the wall center, 0.5 m up: outside a 1 m cube, inside
        // the real 4 m wall (its east face sits at x = 2 m).
        node(crate_, at(1.6, 0.5, 0.0, [1.0; 3]), None, 2),
    ]);

    let rows = rows_of(&doc);
    let wall_row = rows
        .rows
        .iter()
        .find(|row| row.id == wall)
        .expect("wall row");
    assert_eq!(wall_row.size, [4.0, 3.0, 0.2]);
    assert_eq!(wall_row.basis, SizeBasis::Collider);
    assert_eq!(
        overlap_pairs(&doc),
        1,
        "real extents must report the overlap"
    );

    // Before: the same document through the 1 m default cube.
    let old = engine_spatial::run_spatial(&unit_cube_rows(&doc), &["overlap"], None)
        .expect("overlap is legal");
    assert_eq!(old.count("overlap"), 0, "the 1 m cube misses the wall");
}

/// The door's rows and the play path read one document the same way: same
/// boxes, and a verdict over boxes the play world holds.
#[test]
fn adapter_rows_match_the_play_blocks() {
    let wall = id(1);
    let crate_ = id(2);
    let decor = id(3);
    let sensor = id(4);
    let doc = doc_of(&[
        node(wall, at(0.0, 0.0, 0.0, [1.0; 3]), Some(wall_box()), 1),
        node(crate_, at(1.6, 0.5, 0.0, [1.0; 3]), None, 2),
        // Components but no collider: decoration, never a block.
        node(
            decor,
            at(-6.0, 0.5, 0.0, [2.0, 2.0, 2.0]),
            Some(Component::Mesh(Mesh {
                asset: AssetName::try_from("lamp.glb").expect("asset name parses"),
                cast_shadows: true,
            })),
            3,
        ),
        // A trigger volume reports overlaps without pushing: no block.
        node(
            sensor,
            at(6.0, 0.5, 0.0, [1.0; 3]),
            Some(collider([0.5, 0.5, 0.5], ColliderShape::Box, true)),
            4,
        ),
    ]);

    let rows = rows_of(&doc).rows;
    assert_eq!(
        rows.iter().filter_map(block_of).count(),
        2,
        "only the wall and the bare crate block"
    );
    let wall_row = rows.iter().find(|row| row.id == wall).expect("wall row");
    assert_eq!(block_of(wall_row), Some(([4.0, 0.2], [1.0, 0.0])));
    let crate_row = rows.iter().find(|row| row.id == crate_).expect("crate row");
    assert_eq!(block_of(crate_row), Some(([1.0, 1.0], [1.0, 0.0])));
    assert_eq!(
        rows.iter().find(|row| row.id == decor).map(|row| row.solid),
        Some(Solid::Decoration)
    );
    assert_eq!(
        rows.iter()
            .find(|row| row.id == sensor)
            .map(|row| row.solid),
        Some(Solid::Sensor)
    );

    // The verdict over the same boxes: the crate sits inside the wall's 4 m
    // span, which is the shape the play world now holds.
    assert_eq!(overlap_pairs(&doc), 1);
}

/// A bare node keeps the unit-cube convention through its own scale, and says
/// so: the count is real and the report line names it.
#[test]
fn bare_node_is_default_sized_and_named() {
    let bare = id(7);
    let doc = doc_of(&[node(bare, at(0.0, 0.0, 0.0, [2.0, 2.0, 0.4]), None, 1)]);
    let rows = rows_of(&doc);
    assert_eq!(rows.rows.len(), 1);
    let row = rows.rows[0];
    assert_eq!(row.basis, SizeBasis::Default);
    assert_eq!(row.size, [2.0, 2.0, 0.4]);
    assert_eq!(row.solid, Solid::Blocks, "a bare node still blocks");
    let counts = SizeCounts::of(&rows.rows);
    assert_eq!(counts.collider, 0);
    assert_eq!(counts.default, 1, "the fallback is counted, not hidden");
    let line = default_row("wall_1 [00000000-0000-0000-0000-000000000007]", row.size);
    assert!(line.contains("no collider"), "{line}");
    assert!(line.contains("default size"), "{line}");
    assert!(line.contains("2.00x2.00x0.40"), "{line}");
}

/// Same document, same bytes: the adapter is pure and order-stable.
#[test]
fn rows_are_deterministic_and_byte_equal() {
    let wall = node(id(2), at(0.0, 0.0, 0.0, [1.0; 3]), Some(wall_box()), 1);
    let crate_ = node(id(1), at(1.6, 0.5, 0.0, [1.0; 3]), None, 2);
    let forward = doc_of(&[wall, crate_]);
    // Same nodes, opposite insertion order: the rows still sort by id text.
    let backward = doc_of(&[crate_, wall]);
    let first = format!("{:?}", actors(&forward));
    let second = format!("{:?}", actors(&forward));
    assert_eq!(first, second, "two calls give byte-equal rows");
    assert_eq!(
        rows_of(&forward).rows,
        rows_of(&backward).rows,
        "insertion order never moves a row"
    );
    let ids: Vec<ObjectId> = rows_of(&forward).rows.iter().map(|row| row.id).collect();
    assert_eq!(ids, vec![id(1), id(2)], "id-text order");
}

/// Zero or non-finite scale never turns a box into a hole or NaN.
#[test]
fn degenerate_scale_falls_back_to_one() {
    let doc = Document::new();
    let node_id = id(9);
    let (size, basis, solidity) = box_of(&doc.components, node_id, [0.0, f32::NAN, -3.0]);
    assert_eq!(size, [1.0, 1.0, 3.0]);
    assert_eq!(basis, SizeBasis::Default, "no collider is no measurement");
    assert_eq!(solidity, Solid::Blocks, "a bare node still blocks");
    assert!(size.iter().all(|v| v.is_finite() && *v > 0.0));
}
