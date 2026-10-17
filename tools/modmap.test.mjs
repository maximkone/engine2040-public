// Run: node --test tools/modmap.test.mjs
import assert from "node:assert/strict";
import test from "node:test";
import { fileOrder, leadingDoc, packageName, render } from "./modmap.mjs";
import { parseModmapHeadings, parseModmapPaths } from "./workspace-check.mjs";

test("leadingDoc takes the first non-empty //! line and skips attributes", () => {
  assert.equal(leadingDoc("//! core: fixed tick.\n//! more\nfn a() {}"), "core: fixed tick.");
  assert.equal(leadingDoc("#![forbid(unsafe_code)]\n//!\n//! after blank.\n"), "after blank.");
  assert.equal(leadingDoc("use std::fs;\n//! too late"), "(no //! line)");
});

test("packageName reads the package name", () => {
  assert.equal(packageName('[package]\nname = "engine-core"\nversion = "0.1.0"\n'), "engine-core");
  assert.equal(packageName("[workspace]\n"), "");
});

test("lib.rs sorts first, then path order", () => {
  const files = ["src/z.rs", "src/depth/steer.rs", "src/lib.rs", "src/depth.rs"].sort(fileOrder);
  assert.deepEqual(files, ["src/lib.rs", "src/depth.rs", "src/depth/steer.rs", "src/z.rs"]);
});

test("render output parses with the workspace-check map readers", () => {
  const text = render([{ dir: "core", pkg: "engine-core", files: [{ path: "src/lib.rs", lines: 10, doc: "core." }, { path: "src/a.rs", lines: 5, doc: "(no //! line)" }] }]);
  assert.deepEqual(parseModmapHeadings(text), ["core"]);
  assert.deepEqual(parseModmapPaths(text), ["core/src/lib.rs", "core/src/a.rs"]);
  assert.match(text, /## core \(engine-core, 2 files, 15 lines\)/);
  assert.match(text, /1 crates, 2 Rust files/);
});
