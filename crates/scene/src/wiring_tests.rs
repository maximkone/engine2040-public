//! Crate-root wiring vectors: identity survives the save roundtrip, the
//! content digest versions the bytes, and the two never mix.
//!
//! Consumer: an editor or command caller linking `engine-scene` through the
//! crate root (`crate::{codec, component, ids, prefab, provenance, schema,
//! transform}`) who writes saves and reimports them. Data owner stays each
//! module; this file owns only the root contract. Units: node counts, save
//! versions, digest hex chars. Determinism: integer compares and string
//! equality only, no clock, no IO, no RNG seed asserted (minted ids only
//! ever compare unequal). Errors are panics via asserts.
//!
//! Why this file exists: `crates/scene/src/lib.rs` changed 6 times in 30
//! days yet held zero `#[test]` and no covering module, so a dropped
//! `pub mod` would compile the crate but break every caller at their `use`
//! line. Each vector below names the root path, so removing or gating a
//! module fails here first. It lives beside the crate root (not in
//! `tests/it/`) because the layout guard would otherwise flag the
//! pre-existing top-level `tests/schema_gate_vectors.rs` binary.

use crate::{
    codec::{self, CodecError, Scene, CURRENT_VERSION, MIN_SUPPORTED_VERSION},
    component,
    ids::{self, content_digest, ObjectId},
    pin::{self, PinError, ScenePin, RUN_PIN_VERSION},
    prefab, provenance, schema,
    transform::{self, Hierarchy, Transform},
};

/// A minted identity survives the save roundtrip: encode stamps the
/// current version, decode returns the same id with its pose intact.
#[test]
fn identity_survives_save_roundtrip_through_root() {
    let id = ObjectId::new();
    let mut tree = Hierarchy::new();
    tree.insert(id, ObjectId::NIL, Transform::identity())
        .expect("fresh root inserts via root");
    assert!(tree.contains(id), "inserted id is present via root");

    let save = Scene::encode(&tree);
    assert_eq!(
        save.version, CURRENT_VERSION,
        "encode stamps the current version via root"
    );
    assert_eq!(save.nodes.len(), 1, "one node encodes via root");
    assert_eq!(save.nodes[0].id, id, "encoded node keeps its id via root");
    assert_eq!(
        save.nodes[0].parent,
        ObjectId::NIL,
        "root keeps the nil parent slot via root"
    );

    let back = save.decode().expect("roundtrip decodes via root");
    assert_eq!(back.len(), 1, "decoded tree holds one node via root");
    assert!(back.contains(id), "decoded tree keeps the id via root");
    assert_eq!(
        back.local(id),
        Some(Transform::identity()),
        "decoded pose matches via root"
    );
}

/// The digest versions the bytes while the id versions the object: adding a
/// node changes the digest, and digest text never parses as an identity.
#[test]
fn digest_versions_bytes_and_refuses_identity_slot_through_root() {
    let empty = Scene::encode(&Hierarchy::new());
    let empty_text = serde_json::to_string(&empty).expect("empty save serializes");
    let empty_digest = content_digest(empty_text.as_bytes());

    let mut tree = Hierarchy::new();
    tree.insert(ObjectId::new(), ObjectId::NIL, Transform::identity())
        .expect("fresh root inserts via root");
    let one = Scene::encode(&tree);
    let one_text = serde_json::to_string(&one).expect("one-node save serializes");
    let one_digest = content_digest(one_text.as_bytes());

    assert_eq!(
        content_digest(empty_text.as_bytes()),
        empty_digest,
        "same bytes digest identically via root"
    );
    assert_ne!(
        one_digest, empty_digest,
        "added node changes the digest via root"
    );
    for digest in [&empty_digest, &one_digest] {
        assert_eq!(digest.len(), 64, "digest is 64 hex chars via root");
        assert!(
            digest
                .chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
            "digest is lowercase hex via root"
        );
        assert_eq!(
            ObjectId::parse(digest),
            Err(ids::ParseIdError),
            "digest text never parses as an identity via root"
        );
    }
    assert_ne!(
        ObjectId::new(),
        ObjectId::new(),
        "minted ids stay unique via root"
    );
    assert_eq!(
        ObjectId::default(),
        ObjectId::NIL,
        "default id is the nil slot via root"
    );
}

/// The version gate refuses a save newer than the reader, naming the
/// versions involved, and the floor plus current pin through the root.
#[test]
fn version_gate_refuses_newer_save_through_root() {
    assert_eq!(
        CURRENT_VERSION,
        ids::SCHEMA_VERSION,
        "codec current tracks the id schema via root"
    );
    assert_eq!(
        MIN_SUPPORTED_VERSION, 1,
        "oldest migration floor pins via root"
    );
    let future = Scene {
        version: CURRENT_VERSION + 1,
        nodes: Vec::new(),
    };
    match future.decode() {
        Err(CodecError::UnsupportedVersion { got, min, current }) => {
            assert_eq!(got, CURRENT_VERSION + 1, "gate names the save via root");
            assert_eq!(min, MIN_SUPPORTED_VERSION, "gate names the floor via root");
            assert_eq!(current, CURRENT_VERSION, "gate names the reader via root");
        }
        other => panic!("newer save must refuse via root, got {other:?}"),
    }
    assert_eq!(
        codec::MIN_SUPPORTED_VERSION,
        MIN_SUPPORTED_VERSION,
        "floor import matches direct path via root"
    );
}

/// The remaining root modules resolve with their documented caps, so a
/// dropped or re-gated module fails at the `use` lines above first.
#[test]
fn root_caps_pin_documented_budgets_through_root() {
    assert_eq!(
        component::ALL_KINDS.len(),
        8,
        "component kind table size via root"
    );
    assert_eq!(prefab::PREFAB_VERSION, 1, "prefab version via root");
    assert_eq!(prefab::MAX_PREFAB_NODES, 10_000, "prefab node cap via root");
    assert_eq!(
        provenance::PROVENANCE_VERSION,
        1,
        "provenance version via root"
    );
    assert_eq!(provenance::MAX_PARENTS, 8, "provenance parent cap via root");
    assert_eq!(
        schema::MAX_COMPONENT_NAME_LEN,
        64,
        "component name cap via root"
    );
    assert_eq!(
        transform::MIN_INVERTIBLE_SCALE,
        1e-6,
        "invertible scale floor via root"
    );
}

/// Identity text roundtrips and hierarchy guards fail loudly through the
/// root: a minted id stringifies to hyphenated text and parses back, nil
/// keeps its slot, garbage refuses, and nil ids, duplicates or unknown
/// parents refuse with the named error instead of corrupting the tree.
#[test]
fn id_text_roundtrip_and_parent_guards_fail_loudly_through_root() {
    // Text roundtrip via root: hyphenated 36 chars, parses back, nil pins.
    let id = ObjectId::new();
    let text = id.to_string();
    assert_eq!(text.len(), 36, "hyphenated id text via root");
    assert_eq!(
        text.parse::<ObjectId>()
            .expect("minted text parses via root"),
        id,
        "parsed text keeps the id via root"
    );
    assert_eq!(
        ObjectId::parse(&text).expect("parse via root"),
        id,
        "parse path agrees via root"
    );
    assert_eq!(
        ObjectId::NIL
            .to_string()
            .parse::<ObjectId>()
            .expect("nil text parses via root"),
        ObjectId::NIL,
        "nil text keeps the nil slot via root"
    );
    assert!(ObjectId::parse("").is_err(), "empty text refuses via root");
    assert!(
        ObjectId::parse("not-an-id").is_err(),
        "garbage refuses via root"
    );

    // Parent guards via root: nil id, duplicate and unknown parent refuse.
    let mut tree = Hierarchy::new();
    assert_eq!(
        tree.insert(ObjectId::NIL, ObjectId::NIL, Transform::identity()),
        Err(transform::HierarchyError::NilId),
        "nil id refuses via root"
    );
    tree.insert(id, ObjectId::NIL, Transform::identity())
        .expect("fresh root inserts via root");
    assert_eq!(
        tree.insert(id, ObjectId::NIL, Transform::identity()),
        Err(transform::HierarchyError::Duplicate),
        "duplicate refuses via root"
    );
    let orphan = ObjectId::new();
    let missing = ObjectId::new();
    assert_eq!(
        tree.insert(orphan, missing, Transform::identity()),
        Err(transform::HierarchyError::UnknownParent),
        "unknown parent refuses via root"
    );
    assert!(
        !tree.contains(orphan),
        "refused insert leaves no node via root"
    );
    assert_eq!(tree.len(), 1, "only the good insert stays via root");
}

/// The pre-start pin gate stays reachable through the crate root with its
/// headline receipt intact, so a dropped `pub mod pin` fails at the `use`
/// above instead of at a caller's `use engine_scene::pin::ScenePin` line.
///
/// Consumer: the `play.step`/`play.run` gate pins the scene it plays via
/// `Scene::pin` through the root, then demands the pin with
/// `ScenePin::require` and checks the live revision plus digest with
/// `ScenePin::check` before simulating. Units: pin format version,
/// schema version, revision counts, 64-char lowercase digests.
/// Determinism: integer compares and string equality only, no clock,
/// no IO. Errors are the typed `PinError` the door quotes.
#[test]
fn root_pin_gate_stays_reachable_with_headline_receipt_through_root() {
    assert_eq!(RUN_PIN_VERSION, 1, "pin format version via root");
    assert_eq!(
        pin::RUN_PIN_VERSION,
        RUN_PIN_VERSION,
        "pin module path agrees via root"
    );
    assert_eq!(
        ScenePin::require(None),
        Err(PinError::MissingPin),
        "unpinned run refuses via root"
    );
    assert!(
        PinError::MissingPin.to_string().contains("scene pin"),
        "missing-pin names the recovery via root"
    );

    let mut tree = Hierarchy::new();
    let id = ObjectId::new();
    tree.insert(id, ObjectId::NIL, Transform::identity())
        .expect("fresh root inserts via root");
    let save = Scene::encode(&tree);

    assert_eq!(
        save.pin(0),
        Err(PinError::EmptyArena),
        "revision 0 never pins via root"
    );
    let pin = save.pin(7).expect("revision 7 pins via root");
    assert_eq!(pin.version, RUN_PIN_VERSION, "pin stamps format via root");
    assert_eq!(pin.schema, CURRENT_VERSION, "pin stamps schema via root");
    assert_eq!(pin.revision, 7, "pin stamps revision via root");
    assert_eq!(pin.digest, save.digest(), "pin digests the save via root");
    assert_eq!(pin.digest.len(), 64, "pin digest is 64 hex via root");
    assert!(
        pin.digest
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
        "pin digest is lowercase hex via root"
    );
    let held = ScenePin::require(Some(pin.clone())).expect("held pin passes via root");
    assert_eq!(held, pin, "held pin roundtrips via root");

    held.check(7, &held.digest)
        .expect("live revision plus digest passes via root");
    assert_eq!(
        held.check(8, &held.digest),
        Err(PinError::StalePin {
            expected: 7,
            got: 8
        }),
        "moved revision refuses via root"
    );
    let empty = Scene::encode(&Hierarchy::new());
    assert_ne!(
        empty.digest(),
        held.digest,
        "empty save digests differ via root"
    );
    assert_eq!(
        held.check(7, &empty.digest()),
        Err(PinError::DigestMismatch),
        "drifted bytes refuse via root"
    );
}
