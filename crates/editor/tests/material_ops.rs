//! NS31a2: the document holds the material table and edits it with undo.
//!
//! Drives the two material ops through the public command API: `PutMaterial`
//! (add or replace a record) and `SetMaterialParam` (one value of one record),
//! plus the `RemoveLastMaterial` inverse of an appended put. Every test reads
//! the table's own digests (`look_digest`, `sim_digest`) so a restore is
//! asserted on bytes, not on a re-derived value.

use engine_editor::commands::{apply, redo, undo, Batch, BatchError, Document, Journal, Op};

/// Canonical text of the shipped `stone` record, renamed to `name`.
///
/// The first `"stone"` in the canonical text is the record's own name, so one
/// replacement gives a valid record for a fresh name with the stone look and
/// body (the tests never hand-write schema JSON).
fn stone_text(doc: &Document, name: &str) -> String {
    let record = doc.material_set.get("stone").expect("shipped stone record");
    record
        .to_text()
        .replacen("\"stone\"", &format!("\"{name}\""), 1)
}

/// Apply one single-op batch at the document's current revision.
fn apply_one(
    doc: &mut Document,
    journal: &mut Journal,
    key: u64,
    op: Op,
) -> Result<(), BatchError> {
    apply(doc, journal, &Batch::new(vec![op], doc.revision, key)).map(|_| ())
}

/// Set one parameter of one record.
fn set_param(
    doc: &mut Document,
    journal: &mut Journal,
    key: u64,
    material: &str,
    path: &str,
    value: &str,
) -> Result<(), BatchError> {
    apply_one(
        doc,
        journal,
        key,
        Op::SetMaterialParam {
            material: material.to_string(),
            path: path.to_string(),
            value: value.to_string(),
        },
    )
}

#[test]
fn put_then_undo_restores_both_digests_and_the_table() {
    let mut doc = Document::new();
    let mut journal = Journal::new();
    let before = doc.material_set.clone();
    let (look, sim) = (before.look_digest(), before.sim_digest());
    let records = before.len();

    let text = stone_text(&doc, "editor_stone");
    apply_one(&mut doc, &mut journal, 1, Op::PutMaterial { text }).expect("fresh name appends");

    // The append is real: one more record, both digests move, the name reads.
    assert_eq!(doc.material_set.len(), records + 1);
    assert!(doc.material_set.get("editor_stone").is_some());
    assert_ne!(doc.material_set.look_digest(), look);
    assert_ne!(doc.material_set.sim_digest(), sim);
    // A fresh record is appended, so the journal inverse drops the last one.
    assert_eq!(
        journal.past.last().expect("one entry").inverse,
        vec![Op::RemoveLastMaterial]
    );

    undo(&mut doc, &mut journal).expect("the put undoes");
    assert_eq!(doc.material_set, before, "the table comes back byte-exact");
    assert_eq!(doc.material_set.look_digest(), look, "look digest restored");
    assert_eq!(doc.material_set.sim_digest(), sim, "sim digest restored");
    assert!(doc.material_set.get("editor_stone").is_none());

    // Redo replays the same put; a second undo restores the same bytes.
    redo(&mut doc, &mut journal).expect("the put redoes");
    assert_eq!(doc.material_set.len(), records + 1);
    assert_ne!(doc.material_set.look_digest(), look);
    undo(&mut doc, &mut journal).expect("the put undoes again");
    assert_eq!(doc.material_set, before);
}

#[test]
fn put_of_a_known_name_replaces_and_undo_puts_the_previous_record_back() {
    let mut doc = Document::new();
    let mut journal = Journal::new();
    let marble = doc
        .material_set
        .get("marble")
        .expect("shipped marble")
        .to_text();
    let (look, sim) = (
        doc.material_set.look_digest(),
        doc.material_set.sim_digest(),
    );

    // A stone record under the name `marble` replaces the shipped record.
    let text = stone_text(&doc, "marble");
    let replacement = doc
        .material_set
        .get("stone")
        .expect("shipped stone")
        .to_text()
        .replacen("\"stone\"", "\"marble\"", 1);
    apply_one(&mut doc, &mut journal, 1, Op::PutMaterial { text }).expect("a known name replaces");
    assert_eq!(
        doc.material_set
            .get("marble")
            .expect("marble stays")
            .to_text(),
        replacement
    );
    assert_ne!(doc.material_set.look_digest(), look);
    assert_ne!(doc.material_set.sim_digest(), sim);
    // A replace inverts to a put of the previous record's text.
    assert_eq!(
        journal.past.last().expect("one entry").inverse,
        vec![Op::PutMaterial {
            text: marble.clone()
        }]
    );

    undo(&mut doc, &mut journal).expect("the replace undoes");
    assert_eq!(
        doc.material_set
            .get("marble")
            .expect("marble stays")
            .to_text(),
        marble
    );
    assert_eq!(doc.material_set.look_digest(), look);
    assert_eq!(doc.material_set.sim_digest(), sim);
}

#[test]
fn set_material_param_look_edit_undoes_to_the_old_digests() {
    let mut doc = Document::new();
    let mut journal = Journal::new();
    let before = doc.material_set.clone();
    let (look, sim) = (before.look_digest(), before.sim_digest());
    let marble = before.get("marble").expect("shipped marble").to_text();

    set_param(
        &mut doc,
        &mut journal,
        1,
        "marble",
        "look.specular_roughness",
        "0.08",
    )
    .expect("a legal look value applies");
    assert_ne!(
        doc.material_set
            .get("marble")
            .expect("marble stays")
            .to_text(),
        marble
    );
    assert_ne!(
        doc.material_set.look_digest(),
        look,
        "the look digest moved"
    );
    assert_eq!(
        doc.material_set.sim_digest(),
        sim,
        "a look edit never moves the sim digest"
    );

    undo(&mut doc, &mut journal).expect("the edit undoes");
    assert_eq!(doc.material_set, before, "the record comes back byte-exact");
    assert_eq!(doc.material_set.look_digest(), look);
    assert_eq!(doc.material_set.sim_digest(), sim);
}

#[test]
fn set_material_param_body_edit_moves_the_sim_digest_only() {
    let mut doc = Document::new();
    let mut journal = Journal::new();
    let before = doc.material_set.clone();
    let (look, sim) = (before.look_digest(), before.sim_digest());

    set_param(&mut doc, &mut journal, 1, "marble", "body.toughness", "9")
        .expect("a legal sim value applies");
    assert_ne!(
        doc.material_set.sim_digest(),
        sim,
        "the sim digest moved with toughness"
    );
    assert_eq!(
        doc.material_set.look_digest(),
        look,
        "a sim edit never moves the look digest"
    );

    undo(&mut doc, &mut journal).expect("the edit undoes");
    assert_eq!(doc.material_set, before);
    assert_eq!(doc.material_set.sim_digest(), sim);
    assert_eq!(doc.material_set.look_digest(), look);
}

#[test]
fn a_bad_value_is_refused_with_the_record_path_and_holds_the_revision() {
    let mut doc = Document::new();
    let mut journal = Journal::new();
    let before = doc.material_set.clone();

    // A non-JSON value names the record path it was aimed at.
    let rev = doc.revision;
    let err = set_param(
        &mut doc,
        &mut journal,
        1,
        "marble",
        "look.specular_roughness",
        "not-a-number",
    )
    .expect_err("a bad value is refused");
    match &err {
        BatchError::BadMaterialRecord { path, .. } => {
            assert_eq!(
                path, "look.specular_roughness",
                "the refusal names the path"
            );
        }
        other => panic!("expected a material record refusal, got {other:?}"),
    }
    assert!(
        err.to_string().contains("look.specular_roughness"),
        "the report carries the path: {err}"
    );
    assert_eq!(doc.revision, rev, "a refusal publishes no revision");
    assert_eq!(doc.material_set, before, "a refusal leaves the table alone");
    assert_eq!(journal.past_len(), 0, "a refusal journals nothing");

    // A value outside the schema range names the same path.
    let err = set_param(
        &mut doc,
        &mut journal,
        2,
        "marble",
        "look.specular_roughness",
        "9",
    )
    .expect_err("9 is outside the roughness range");
    match &err {
        BatchError::BadMaterialRecord { path, .. } => assert_eq!(path, "look.specular_roughness"),
        other => panic!("expected a material record refusal, got {other:?}"),
    }

    // An unknown record name names the material.
    let err = set_param(
        &mut doc,
        &mut journal,
        3,
        "no-such-stone",
        "look.specular_roughness",
        "0.1",
    )
    .expect_err("an unknown record is refused");
    match &err {
        BatchError::BadMaterialRecord { path, .. } => assert_eq!(path, "material"),
        other => panic!("expected a material record refusal, got {other:?}"),
    }

    // A bad put text is refused the same way, path `$` for the document.
    let err = apply_one(
        &mut doc,
        &mut journal,
        4,
        Op::PutMaterial {
            text: "not json".to_string(),
        },
    )
    .expect_err("bad record text is refused");
    match &err {
        BatchError::BadMaterialRecord { path, .. } => assert_eq!(path, "$"),
        other => panic!("expected a material record refusal, got {other:?}"),
    }
    assert_eq!(doc.revision, rev, "no refusal published a revision");
    assert_eq!(doc.material_set, before);
    assert_eq!(journal.past_len(), 0);
}

#[test]
fn remove_last_drops_the_newest_record_and_is_none_when_empty() {
    // The direct table behavior behind `Op::RemoveLastMaterial`'s inverse.
    let mut set = Document::new().material_set;
    let records = set.len();
    let last = set
        .records()
        .last()
        .expect("the shipped table holds records");
    let last_name = last.name.clone();
    let removed = set.remove_last().expect("a record is there to drop");
    assert_eq!(removed.name, last_name);
    assert_eq!(set.len(), records - 1);
    for _ in 1..records {
        assert!(set.remove_last().is_some(), "records keep draining");
    }
    assert!(set.is_empty(), "the table drains to empty");
    assert!(
        set.remove_last().is_none(),
        "an empty table has nothing to drop"
    );
}

#[test]
fn remove_last_material_undoes_an_appended_put_through_the_journal() {
    // The inverse op on its own: an appended put, undone by the drop, then
    // re-done from the recorded inverse text.
    let mut doc = Document::new();
    let mut journal = Journal::new();
    let name = "editor_drop";
    let text = stone_text(&doc, name);
    apply_one(
        &mut doc,
        &mut journal,
        1,
        Op::PutMaterial { text: text.clone() },
    )
    .expect("the record appends");
    assert!(doc.material_set.get(name).is_some());

    apply_one(&mut doc, &mut journal, 2, Op::RemoveLastMaterial).expect("the drop applies");
    assert!(doc.material_set.get(name).is_none(), "the record is gone");
    undo(&mut doc, &mut journal).expect("the drop undoes");
    assert_eq!(
        doc.material_set
            .get(name)
            .expect("the record is back")
            .to_text(),
        text
    );
    // Undoing the put as well leaves the shipped table untouched.
    undo(&mut doc, &mut journal).expect("the put undoes");
    assert_eq!(doc.material_set, Document::new().material_set);
}
