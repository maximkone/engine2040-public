//! A2 material-bind round trip: bind, save, reopen byte-identical.
//!
//! Owner: the recorded A2 evidence run; uses the same shell, browser and
//! save codec path as the A2 session runner. Units: bytes for files.
//! Determinism: fixed node ids give identical save bytes run to run.
//! Errors: shell, browser or save failures fail the test with its cause.
//!
//! Attribution: byte-exact save and reopen check idea adapted from
//! vaibhav-1509/ijewel-vjson-studio (MIT,
//! https://github.com/vaibhav-1509/ijewel-vjson-studio); fresh write here,
//! no donor bytes copied (idea only).

use engine_editor::browser::{AssetKind, BundleEntry, BundleView};
use engine_editor::commands::Op;
use engine_editor::save::{load_from_bytes, save_to_bytes};
use engine_editor::shell::Shell;
use engine_scene::ids::ObjectId;
use engine_scene::transform::Transform;

/// Fixed test node id for `slot`: same shape as the A2 session slots so the
/// fixture follows the session path without reading licensed content bytes.
fn fixture_id(slot: u32) -> ObjectId {
    let text = format!("a2e70002-0000-4000-8000-{slot:012x}");
    ObjectId::parse(&text).expect("fixture slot id is a UUID")
}

/// Minimal bundle for the fixture: one material plus one mesh decoy.
fn fixture_bundle() -> BundleView {
    let mut bundle = BundleView::new();
    bundle
        .push(BundleEntry::new(
            AssetKind::Material,
            "hero-mat",
            &["fighter"],
        ))
        .expect("hero-mat entry validates");
    bundle
        .push(BundleEntry::new(AssetKind::Mesh, "crate", &["prop"]))
        .expect("crate entry validates");
    bundle
}

/// Shell with one node placed at the origin.
fn shell_with_node(slot: u32) -> (Shell, ObjectId) {
    let mut shell = Shell::new();
    let node = fixture_id(slot);
    shell
        .insert_node(node, ObjectId::NIL, Transform::identity())
        .expect("insert fixture node");
    (shell, node)
}

/// Bind persists through save and reopen byte-identical (the A2 guarantee).
#[test]
fn material_bind_save_reopen_is_byte_identical() {
    let bundle = fixture_bundle();
    let (mut shell, node) = shell_with_node(1);
    shell
        .bind_material(node, "hero-mat", &bundle)
        .expect("hero-mat binds");
    assert!(
        shell
            .material_of(node)
            .is_some_and(|name| name.as_str() == "hero-mat"),
        "binding is visible before save"
    );

    let revision = shell.revision();
    let saved = save_to_bytes(shell.document());
    assert!(!saved.is_empty(), "save writes bytes");
    let needle = b"hero-mat";
    assert!(
        saved.windows(needle.len()).any(|w| w == needle),
        "scene bytes carry the bound material"
    );

    // Same document saves identically twice.
    let saved_again = save_to_bytes(shell.document());
    assert_eq!(saved, saved_again, "repeated save is byte-identical");

    let reopened = load_from_bytes(&saved).expect("save reopens");
    assert_eq!(reopened.revision, revision, "revision survives reopen");
    assert_eq!(
        reopened.hierarchy.len(),
        shell.document().hierarchy.len(),
        "node count survives reopen"
    );
    assert!(
        reopened
            .materials
            .get(&node.to_string())
            .is_some_and(|name| name.as_str() == "hero-mat"),
        "reopened document keeps the binding"
    );
    let resaved = save_to_bytes(&reopened);
    assert_eq!(saved, resaved, "reopen then save is byte-identical");
}

/// Unknown assets, non-material assets and unknown nodes refuse cleanly.
#[test]
fn material_bind_refusals_write_nothing() {
    let bundle = fixture_bundle();
    let (mut shell, node) = shell_with_node(2);
    let before = shell.revision();

    assert!(
        shell.bind_material(node, "no-such-mat", &bundle).is_err(),
        "unknown material refuses"
    );
    assert!(
        shell.bind_material(node, "crate", &bundle).is_err(),
        "plain mesh refuses as a material"
    );
    assert!(
        shell
            .bind_material(fixture_id(99), "hero-mat", &bundle)
            .is_err(),
        "missing node refuses"
    );
    assert_eq!(shell.revision(), before, "refusals move no revision");
    assert!(
        shell.material_of(node).is_none(),
        "refusals stage no binding"
    );

    // The refused shell still round-trips byte-identical with no binding.
    let saved = save_to_bytes(shell.document());
    let reopened = load_from_bytes(&saved).expect("refused save reopens");
    assert_eq!(save_to_bytes(&reopened), saved, "refused bytes round-trip");
}

/// Unbinding restores the unbound bytes (the bind journals, it is not baked).
#[test]
fn material_unbind_restores_unbound_bytes() {
    let bundle = fixture_bundle();
    let (mut shell, node) = shell_with_node(3);
    let plain = save_to_bytes(shell.document());
    shell
        .bind_material(node, "hero-mat", &bundle)
        .expect("hero-mat binds");
    let bound = save_to_bytes(shell.document());
    assert_ne!(plain, bound, "the bind lands in the save bytes");

    shell
        .apply_ops(vec![Op::SetMaterial {
            id: node,
            material: None,
        }])
        .expect("unbind applies");
    assert!(shell.material_of(node).is_none(), "unbind clears binding");
    let reopened = load_from_bytes(&save_to_bytes(shell.document())).expect("unbound save reopens");
    assert!(
        reopened.materials.get(&node.to_string()).is_none(),
        "reopen keeps the unbind"
    );
}
