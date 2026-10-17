//! MCP manifest pins: ListTools shape plus path gate, registry-direct.
//!
//! Consumer: the MCP door author sizing the tool manifest. Data owner is the
//! endpoint registry (`engine_endpoint::tools`); this file owns only the
//! pins below. Every assertion reads the frozen registry; nothing here
//! simulates the door, so no mock can drift from the table it checks.

use engine_endpoint::tools;

#[test]
fn manifest_list_shape_covers_registry() {
    let listed = tools::TOOLS;
    assert!(!listed.is_empty(), "the frozen registry lists tools");
    let mut names: Vec<&str> = listed.iter().map(|def| def.name).collect();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), listed.len(), "manifest names stay unique");
    for def in listed {
        assert!(!def.name.is_empty(), "every row names its tool");
        assert!(
            def.name
                .chars()
                .all(|c| c.is_ascii_lowercase() || c == '.' || c == '_' || c.is_ascii_digit()),
            "tool {} keeps the machine-code shape",
            def.name
        );
    }
    for pinned in ["scene.read", "tx.preview", "tx.apply", "tx.undo"] {
        assert!(
            listed.iter().any(|def| def.name == pinned),
            "manifest lists pinned tool {pinned}"
        );
    }
    // Slim-8 stays read-only inside the manifest, so a weak model discovers
    // the door without ever touching a writer.
    for name in tools::SLIM8 {
        let def = tools::lookup(name).unwrap_or_else(|| panic!("slim-8 tool {name} is listed"));
        assert!(def.readonly, "slim-8 tool {name} stays read-only");
    }
    // Unknown tools fail closed even under a good path.
    assert!(
        tools::lookup("shell.exec").is_none(),
        "unknown tool shell.exec stays unlisted"
    );
}
