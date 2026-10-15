//! Typed component ops through the command service: eight kinds set, saved,
//! reopened byte-identical, undone, redone, sealed to disk and refused when
//! invalid. Consumer: door v2 (`tx.preview` / `tx.apply` component ops).

use engine_editor::commands::{
    apply, redo, undo, Batch, BatchError, CommandMeta, Document, Journal, Op,
};
use engine_editor::durable::{apply_persisted, rehydrate, reopen_sealed};
use engine_editor::save::{load_from_bytes, save_to_bytes};
use engine_editor::wal::{wal_apply, Wal};
use engine_scene::component::{
    Anatomy, AssetName, Collider, ColliderShape, Component, ComponentKind, Destructible, Light,
    LightKind, MaterialRef, Mesh, Rig, Spawner, ALL_KINDS,
};
use engine_scene::ids::ObjectId;
use engine_scene::transform::Transform;

fn name(text: &str) -> AssetName {
    AssetName::try_from(text).expect("test name is valid")
}

/// One valid sample of every kind, in tag order.
fn samples() -> [Component; 8] {
    [
        Component::Mesh(Mesh {
            asset: name("meshes/wall"),
            cast_shadows: true,
        }),
        Component::Collider(Collider {
            shape: ColliderShape::Box,
            half: [1.0, 2.5, 0.125],
            sensor: false,
        }),
        Component::Material(MaterialRef {
            asset: name("marble"),
            tint: [1.0, 0.5, 0.25],
        }),
        Component::Light(Light {
            kind: LightKind::Spot,
            color: [1.0, 0.9, 0.8],
            intensity: 12.5,
            range: 30.0,
        }),
        Component::Spawner(Spawner {
            archetype: name("fighter.grunt"),
            count: 3,
            period_ticks: 600,
        }),
        Component::Destructible(Destructible {
            material: name("stone"),
            health: 400,
            chunks: 64,
        }),
        Component::Anatomy(Anatomy {
            template: name("humanoid"),
            limbs: 4,
        }),
        Component::Rig(Rig {
            skeleton: name("hero_skel"),
            clips: name("hero_clips"),
        }),
    ]
}

/// A document with one node, applied through the command service.
fn doc_with_node() -> (Document, Journal, ObjectId) {
    let mut doc = Document::new();
    let mut journal = Journal::new();
    let id = ObjectId::new();
    apply(
        &mut doc,
        &mut journal,
        &Batch::new(
            vec![Op::Insert {
                id,
                parent: ObjectId::NIL,
                local: Transform::IDENTITY,
            }],
            0,
            1,
        ),
    )
    .expect("insert applies");
    (doc, journal, id)
}

fn set_all(id: ObjectId, revision: u64, key: u64) -> Batch {
    Batch::new(
        samples()
            .into_iter()
            .map(|component| Op::SetComponent { id, component })
            .collect(),
        revision,
        key,
    )
}

fn scratch(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("e2040-comp-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir creates");
    dir
}

#[test]
fn eight_components_save_reload_byte_identical_and_undo() {
    let (mut doc, mut journal, id) = doc_with_node();
    let before = doc.clone();
    let before_bytes = save_to_bytes(&before);
    let rev = doc.revision;
    apply(&mut doc, &mut journal, &set_all(id, rev, 2)).expect("eight components apply");
    let slot = doc
        .components
        .get(&id.to_string())
        .expect("node has components");
    assert_eq!(slot.len(), 8);
    for (kind, value) in ALL_KINDS.iter().zip(samples()) {
        assert_eq!(slot.get(kind), Some(&value));
    }

    // Save and reopen: equal document, byte-identical re-save.
    let bytes = save_to_bytes(&doc);
    assert_eq!(bytes, save_to_bytes(&doc), "same document, same bytes");
    let back = load_from_bytes(&bytes).expect("reopens");
    assert_eq!(back, doc);
    assert_eq!(save_to_bytes(&back), bytes, "re-save is byte-identical");

    // Undo restores the exact pre-batch bytes; redo restores the components.
    undo(&mut doc, &mut journal).expect("undo applies");
    assert!(doc.components.is_empty());
    assert_eq!(doc.hierarchy, before.hierarchy);
    let mut undone_bytes = save_to_bytes(&doc);
    // The revision keeps counting; compare everything else by patching it.
    undone_bytes[8..16].copy_from_slice(&before.revision.to_le_bytes());
    let mut base = before_bytes.clone();
    let len = base.len();
    base.truncate(len - 4);
    undone_bytes.truncate(undone_bytes.len() - 4);
    assert_eq!(undone_bytes, base, "undo restores the exact document bytes");
    redo(&mut doc, &mut journal).expect("redo applies");
    assert_eq!(doc.components, back.components);
}

#[test]
fn set_replaces_one_kind_and_undo_restores_the_old_value() {
    let (mut doc, mut journal, id) = doc_with_node();
    let rev = doc.revision;
    apply(&mut doc, &mut journal, &set_all(id, rev, 2)).expect("seeds");
    let louder = Component::Light(Light {
        kind: LightKind::Point,
        color: [0.0, 0.0, 1.0],
        intensity: 99.0,
        range: 5.0,
    });
    let rev = doc.revision;
    apply(
        &mut doc,
        &mut journal,
        &Batch::new(
            vec![Op::SetComponent {
                id,
                component: louder,
            }],
            rev,
            3,
        ),
    )
    .expect("replace applies");
    let slot = doc.components.get(&id.to_string()).expect("slot");
    assert_eq!(slot.get(&ComponentKind::Light), Some(&louder));
    assert_eq!(slot.len(), 8, "replace keeps one slot per kind");
    undo(&mut doc, &mut journal).expect("undo");
    assert_eq!(
        doc.components
            .get(&id.to_string())
            .and_then(|slot| slot.get(&ComponentKind::Light)),
        Some(&samples()[3])
    );
}

#[test]
fn remove_component_undoes_and_missing_kinds_refuse() {
    let (mut doc, mut journal, id) = doc_with_node();
    let rev = doc.revision;
    apply(&mut doc, &mut journal, &set_all(id, rev, 2)).expect("seeds");
    let rev = doc.revision;
    apply(
        &mut doc,
        &mut journal,
        &Batch::new(
            vec![Op::RemoveComponent {
                id,
                kind: ComponentKind::Rig,
            }],
            rev,
            3,
        ),
    )
    .expect("remove applies");
    assert!(doc
        .components
        .get(&id.to_string())
        .is_some_and(|slot| !slot.contains_key(&ComponentKind::Rig)));
    undo(&mut doc, &mut journal).expect("undo");
    assert_eq!(
        doc.components
            .get(&id.to_string())
            .and_then(|slot| slot.get(&ComponentKind::Rig)),
        Some(&samples()[7])
    );

    // A fresh node carries nothing: removing refuses and writes nothing.
    let other = ObjectId::new();
    let rev = doc.revision;
    apply(
        &mut doc,
        &mut journal,
        &Batch::new(
            vec![Op::Insert {
                id: other,
                parent: ObjectId::NIL,
                local: Transform::IDENTITY,
            }],
            rev,
            4,
        ),
    )
    .expect("insert");
    let rev = doc.revision;
    let snapshot = doc.clone();
    let refused = apply(
        &mut doc,
        &mut journal,
        &Batch::new(
            vec![Op::RemoveComponent {
                id: other,
                kind: ComponentKind::Mesh,
            }],
            rev,
            5,
        ),
    )
    .expect_err("missing kind refuses");
    assert_eq!(
        refused,
        BatchError::MissingComponent {
            id: other,
            kind: ComponentKind::Mesh
        }
    );
    assert!(refused.to_string().contains("mesh"));
    assert_eq!(doc, snapshot, "a refused batch writes nothing");
}

#[test]
fn bad_values_and_unknown_nodes_refuse_atomically() {
    let (mut doc, mut journal, id) = doc_with_node();
    let snapshot = doc.clone();
    let nan = Component::Light(Light {
        kind: LightKind::Point,
        color: [1.0, 1.0, 1.0],
        intensity: f32::NAN,
        range: 1.0,
    });
    // A good op first, then the bad one: nothing may land.
    let batch = Batch::new(
        vec![
            Op::SetComponent {
                id,
                component: samples()[0],
            },
            Op::SetComponent { id, component: nan },
        ],
        doc.revision,
        9,
    );
    assert_eq!(
        apply(&mut doc, &mut journal, &batch).expect_err("NaN refuses"),
        BatchError::BadComponent {
            id,
            kind: ComponentKind::Light
        }
    );
    assert_eq!(doc, snapshot);
    let ghost = ObjectId::new();
    let batch = Batch::new(
        vec![Op::SetComponent {
            id: ghost,
            component: samples()[0],
        }],
        doc.revision,
        10,
    );
    assert_eq!(
        apply(&mut doc, &mut journal, &batch).expect_err("unknown node refuses"),
        BatchError::UnknownNode { id: ghost }
    );
    assert_eq!(doc, snapshot);
}

#[test]
fn removing_a_node_takes_its_components_and_undo_brings_them_back() {
    let (mut doc, mut journal, id) = doc_with_node();
    let rev = doc.revision;
    apply(&mut doc, &mut journal, &set_all(id, rev, 2)).expect("seeds");
    let with = doc.components.clone();
    let rev = doc.revision;
    apply(
        &mut doc,
        &mut journal,
        &Batch::new(vec![Op::Remove { id }], rev, 3),
    )
    .expect("remove applies");
    assert!(doc.components.is_empty(), "components die with the node");
    // The saved bytes never name the missing node.
    assert!(load_from_bytes(&save_to_bytes(&doc))
        .expect("reopens")
        .components
        .is_empty());
    undo(&mut doc, &mut journal).expect("undo");
    assert_eq!(doc.components, with);
}

#[test]
fn components_survive_the_durable_journal_and_the_wal() {
    // Durable generations: apply_persisted seals the save plus the journal.
    let dir = scratch("durable");
    let (mut doc, mut journal, id) = doc_with_node();
    let rev = doc.revision;
    apply_persisted(
        &mut doc,
        &mut journal,
        &set_all(id, rev, 2),
        &CommandMeta::default(),
        &dir,
        rev + 1,
    )
    .expect("persists");
    let opened = reopen_sealed(&dir).expect("io").expect("a generation");
    let mut reopened_doc = Document::new();
    let mut reopened_journal = Journal::new();
    rehydrate(&mut reopened_doc, &mut reopened_journal, opened);
    assert_eq!(reopened_doc.components, doc.components);
    // The sealed journal carries the component ops: undo after reopen works.
    undo(&mut reopened_doc, &mut reopened_journal).expect("undo after reopen");
    assert!(reopened_doc.components.is_empty());
    let _ = std::fs::remove_dir_all(&dir);

    // WAL: the op frame decodes back to the same ops on reopen.
    let wal_dir = scratch("wal");
    let mut live = Document::new();
    let mut live_journal = Journal::new();
    let (mut wal, _report) = Wal::open(&wal_dir, &mut live, &mut live_journal).expect("wal opens");
    let id = ObjectId::new();
    let meta = CommandMeta::default();
    let insert = Batch::new(
        vec![Op::Insert {
            id,
            parent: ObjectId::NIL,
            local: Transform::IDENTITY,
        }],
        0,
        1,
    );
    wal_apply(&mut wal, &mut live, &mut live_journal, &insert, &meta, 1).expect("wal insert");
    let rev = live.revision;
    wal_apply(
        &mut wal,
        &mut live,
        &mut live_journal,
        &set_all(id, rev, 2),
        &meta,
        2,
    )
    .expect("wal apply");
    assert_eq!(
        live.components.get(&id.to_string()).map(|s| s.len()),
        Some(8)
    );
    drop(wal);
    // Reopen replays the logged frames to the same components.
    let mut again = Document::new();
    let mut again_journal = Journal::new();
    let (_wal, _report) = Wal::open(&wal_dir, &mut again, &mut again_journal).expect("wal reopens");
    assert_eq!(again.components, live.components);
    assert_eq!(again.revision, live.revision);
    let _ = std::fs::remove_dir_all(&wal_dir);
}
