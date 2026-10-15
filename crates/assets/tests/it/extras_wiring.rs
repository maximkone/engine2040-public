#![cfg(feature = "import")]
use engine_assets::extras::{parse, AttachKind};
use engine_assets::mesh_cook::is_socket_name;
use engine_core::hash_bytes;
const DOC: &str = r#"{"asset":{"version":"2.0"},"nodes":[{"name":"SOCKET_hand_r","extras":{"skein":{"Hitbox":{"half":[0.09,0.09,0.22]}}}},{"name":"SOCKET_wep_R","extras":{"skein":{"Socket":{"joint":"hand_r"}}}},{"name":"Spawn_A","extras":{"skein":{"Spawn":{"team":"attackers"}}}},{"name":"Body"}]}"#;
#[test]
fn extras_land_in_order_and_refuse_unknown() {
    let a = parse(DOC.as_bytes()).expect("spike parses");
    assert_eq!(a.len(), 3, "3 extras nodes land 3 attaches");
    assert!(matches!(a[0].kind, AttachKind::Hitbox) && a[0].node == 0);
    assert!(matches!(a[1].kind, AttachKind::Socket) && a[1].node == 1);
    assert!(matches!(a[2].kind, AttachKind::Spawn) && a[2].node == 2);
    assert!(a[0].payload.contains("half") && a[1].payload.contains("hand_r"));
    let again = parse(DOC.as_bytes()).expect("reparse");
    assert_eq!(a, again, "parse x2 identical");
    let enc =
        |v: &Vec<engine_assets::extras::ExtrasAttach>| hash_bytes(format!("{v:?}").as_bytes());
    assert_eq!(enc(&a), enc(&again), "cook digest x2 byte-equal");
    assert!(is_socket_name("SOCKET_hand_r") && is_socket_name("SOCKET_wep_R"));
    assert!(!is_socket_name("Spawn_A") && !is_socket_name("Body"));
    let evil = DOC.replace("\"Spawn\"", "\"Boss\"");
    assert!(matches!(
        parse(evil.as_bytes()),
        Err(engine_assets::extras::ExtrasError::UnknownKey { .. })
    ));
}
