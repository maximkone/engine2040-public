//! MCP manifest vectors: ListTools and CallTool shape plus path gate, mock-only.
//!
//! Consumer: the MCP door author sizing the tool manifest. Data owner is the
//! endpoint registry (engine_endpoint tools); this file owns only mock
//! vectors. Units: tasks in, pass count out. Determinism: pure mock math,
//! no clock or IO. Errors are mock codes asserted by name.
//!
//! Attribution: manifest shape idea (ListTools list plus CallTool dispatch
//! with a path gate) follows tugcantopaloglu/godot-mcp (MIT, verified
//! at https://github.com/tugcantopaloglu/godot-mcp, src/index.ts); every line
//! below is fresh Rust against our own registry, no donor bytes copied.
//! PORT(tugcantopaloglu/godot-mcp:src/index.ts): manifest shape idea only.

use engine_endpoint::tools::{self, ToolDef};

// ---------------------------------------------------------------------------
// Mock manifest shape: ListTools entry plus CallTool dispatch, mock-only.
// ---------------------------------------------------------------------------

/// Longest tool path in bytes; the gate refuses longer with path_too_long.
const PATH_MAX_BYTES: usize = 512;

/// One ListTools row: name plus one-line description plus readonly flag.
/// Mirrors the donor manifest row (name, description and schema) in fresh Rust.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ManifestEntry {
    /// Tool name, exactly one registry name.
    name: &'static str,
    /// One-line description; the mock mints it from the name.
    description: String,
    /// Readonly flag copied from the registry.
    readonly: bool,
}

/// Mock door error: machine code plus detail naming the legal values.
#[derive(Debug, Clone, PartialEq, Eq)]
struct MockError {
    /// Machine code (unknown_tool, empty_path and friends).
    code: &'static str,
    /// Detail with the legal values plus one recovery.
    message: String,
}

impl MockError {
    /// New mock error with a static code.
    fn new(code: &'static str, message: String) -> Self {
        Self { code, message }
    }
}

impl core::fmt::Display for MockError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for MockError {}

/// ListTools mock: every registry tool with a minted description.
/// Pure: reads the frozen table only, writes nothing.
fn mock_list_tools() -> Vec<ManifestEntry> {
    tools::TOOLS
        .iter()
        .map(|def: &ToolDef| ManifestEntry {
            name: def.name,
            description: format!("{} door tool", def.name),
            readonly: def.readonly,
        })
        .collect()
}

/// Path gate for CallTool args: relative project paths only.
/// Rejects empty, over-long, null-byte, absolute and dot-dot escaping paths
/// with a named code; anything else passes. Pure string compares.
fn validate_tool_path(path: &str) -> Result<(), MockError> {
    if path.is_empty() {
        return Err(MockError::new(
            "empty_path",
            "path is empty; submit 1 to 512 bytes of relative path, then re-run".to_string(),
        ));
    }
    if path.len() > PATH_MAX_BYTES {
        return Err(MockError::new(
            "path_too_long",
            format!(
                "path holds {} bytes; at most 512; shorten it or split the call",
                path.len()
            ),
        ));
    }
    if path.contains(char::from(0_u8)) {
        return Err(MockError::new(
            "bad_path",
            "path holds a null byte; submit plain relative text, then re-run".to_string(),
        ));
    }
    // Byte compares below: 0x2F is slash, 0x5C backslash, 0x3A colon.
    let bytes = path.as_bytes();
    if bytes[0] == 0x2F || bytes[0] == 0x5C {
        return Err(MockError::new(
            "absolute_path",
            format!("path {path} is absolute; submit a relative project path, then re-run"),
        ));
    }
    if path.len() >= 2 && bytes[1] == 0x3A {
        return Err(MockError::new(
            "absolute_path",
            format!("path {path} is absolute; submit a relative project path, then re-run"),
        ));
    }
    let escaped =
        path.split("/").any(|part| part == "..") || path.split("\\").any(|part| part == "..");
    if escaped {
        return Err(MockError::new(
            "escapes_root",
            format!("path {path} escapes the project root with dot-dot; stay inside, then re-run"),
        ));
    }
    Ok(())
}

/// CallTool mock: gate the path, then resolve the tool by exact name.
/// Ok carries a sealed receipt string; Err names the refusal code.
/// Pure: reads the registry only, writes nothing.
fn mock_call_tool(name: &str, path: &str) -> Result<String, MockError> {
    validate_tool_path(path)?;
    let def = tools::lookup(name).ok_or_else(|| {
        MockError::new(
            "unknown_tool",
            format!("unknown tool {name}; list tools first, then call one listed name"),
        )
    })?;
    Ok(format!("ok:{}:{}", def.name, path))
}

// ---------------------------------------------------------------------------
// 20 mirrored tasks: (tool, path, expect_ok). Mock-only, fixed order.
// ---------------------------------------------------------------------------

/// Twenty mirrored door tasks: good ListTools and CallTool pairs pass, unknown
/// tools and bad paths refuse. Same input always seals the same verdict.
const MIRRORED_TASKS: [(&str, &str, bool); 20] = [
    ("scene.read", "scenes/arena.json", true),
    ("tx.preview", "batches/preview.json", true),
    ("tx.apply", "batches/apply.json", true),
    ("tx.undo", "journal/undo.json", true),
    ("scene.brief", "briefs/keep.txt", true),
    ("scene.spatial", "scenes/arena.json", true),
    ("scene.query", "queries/north.json", true),
    ("sim.plan_goals", "plans/goals.txt", true),
    ("log.digest", "logs/digest.json", true),
    ("log.query", "logs/query.json", true),
    ("api.search", "index/query.txt", true),
    ("api.describe", "index/entry.txt", true),
    ("code.callers", "src/tools.rs", true),
    ("asset.refs", "assets/fighter.glb", true),
    ("rpc.discover", "door/manifest.json", true),
    ("shell.exec", "scenes/arena.json", false),
    ("scene.read", "", false),
    ("scene.read", "/abs/arena.json", false),
    ("tx.preview", "../outside.json", false),
    ("tx.apply", "C:\\abs\\apply.json", false),
];

/// Run the 20 mirrored tasks through the mock door.
/// Returns (passed, total): a task passes when the mock verdict matches
/// the mirrored expectation. Deterministic over the fixed table.
fn run_mirrored_tasks() -> (usize, usize) {
    let mut passed = 0usize;
    for (name, path, expect_ok) in MIRRORED_TASKS {
        let ok = mock_call_tool(name, path).is_ok();
        if ok == expect_ok {
            passed += 1;
        }
    }
    (passed, MIRRORED_TASKS.len())
}

#[test]
fn manifest_list_shape_covers_registry() {
    let listed = mock_list_tools();
    assert_eq!(
        listed.len(),
        tools::TOOLS.len(),
        "ListTools mirrors the frozen registry one to one"
    );
    assert!(
        listed.len() >= MIRRORED_TASKS.len(),
        "manifest covers the 20 mirrored tasks: {} rows",
        listed.len()
    );
    let mut names: Vec<&str> = listed.iter().map(|row| row.name).collect();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), listed.len(), "manifest names stay unique");
    for row in &listed {
        assert!(!row.name.is_empty(), "every row names its tool");
        assert!(
            row.name
                .chars()
                .all(|c| c.is_ascii_lowercase() || c == '.' || c == '_' || c.is_ascii_digit()),
            "tool {} keeps the machine-code shape",
            row.name
        );
        assert!(
            !row.description.is_empty(),
            "tool {} carries a description",
            row.name
        );
        let def = tools::lookup(row.name)
            .unwrap_or_else(|| panic!("manifest row {} resolves in the registry", row.name));
        assert_eq!(
            def.readonly, row.readonly,
            "tool {} keeps its readonly flag",
            row.name
        );
    }
    for pinned in ["scene.read", "tx.preview", "tx.apply", "tx.undo"] {
        assert!(
            listed.iter().any(|row| row.name == pinned),
            "manifest lists pinned tool {pinned}"
        );
    }
    // Slim-8 stays read-only inside the manifest, so a weak model discovers
    // the door without ever touching a writer.
    for name in tools::SLIM8 {
        let row = listed
            .iter()
            .find(|row| row.name == *name)
            .unwrap_or_else(|| panic!("slim-8 tool {name} is listed"));
        assert!(row.readonly, "slim-8 tool {name} stays read-only");
    }
}

#[test]
fn call_tool_path_gate_vectors() {
    for good in [
        "scenes/arena.json",
        "batches/preview.json",
        "a/b/c.txt",
        "arena.json",
    ] {
        assert!(
            validate_tool_path(good).is_ok(),
            "relative path {good} passes the gate"
        );
    }
    let cases: [(&str, &str); 5] = [
        ("", "empty_path"),
        ("/abs/arena.json", "absolute_path"),
        ("C:\\abs\\apply.json", "absolute_path"),
        ("../outside.json", "escapes_root"),
        ("a/../../b.json", "escapes_root"),
    ];
    for (path, code) in cases {
        let refused = validate_tool_path(path).expect_err("bad path refuses");
        assert_eq!(refused.code, code, "path {path} refuses as {code}");
        assert!(
            !refused.message.is_empty(),
            "refusal for {path} names a recovery"
        );
    }
    let long = "p".repeat(PATH_MAX_BYTES + 1);
    let over = validate_tool_path(&long).expect_err("over-long path refuses");
    assert_eq!(over.code, "path_too_long");
    let nulled = validate_tool_path("a\0b.json").expect_err("null byte refuses");
    assert_eq!(nulled.code, "bad_path");
    // Unknown tools fail closed even under a good path.
    let unknown = mock_call_tool("shell.exec", "scenes/arena.json").expect_err("unknown refuses");
    assert_eq!(unknown.code, "unknown_tool");
    // Same inputs twice seal the same verdict: the mock replays exactly.
    let first = mock_call_tool("scene.read", "scenes/arena.json");
    let second = mock_call_tool("scene.read", "scenes/arena.json");
    assert_eq!(first, second, "replay without a model drifts");
}

#[test]
fn mirrored_20_tasks_pass_rate_is_full() {
    let (passed, total) = run_mirrored_tasks();
    let percent = passed * 100 / total;
    println!("mirrored-task pass rate: {passed}/{total} ({percent}%)");
    for (index, (name, path, expect_ok)) in MIRRORED_TASKS.iter().enumerate() {
        let ok = mock_call_tool(name, path).is_ok();
        assert_eq!(ok, *expect_ok, "task {index} {name} {path} verdict drifts");
    }
    assert_eq!(total, 20, "harness pins 20 mirrored tasks");
    assert_eq!(
        passed, total,
        "all 20 mirrored tasks agree: {passed}/{total}"
    );
    assert_eq!(percent, 100, "pass rate pins at 100%");
}
