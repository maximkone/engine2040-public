// Run: node --test tools/modmap.test.mjs
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";
import {
  PACK_BUDGET_TOKENS,
  buildPacket,
  crateDirOf,
  crateMapRowOf,
  diffSummary,
  fileOrder,
  leadingDoc,
  mapSectionOf,
  ownsCrate,
  packageName,
  planForCrate,
  planIndexLineOf,
  render,
  tokenCount,
} from "./modmap.mjs";
import { parseModmapHeadings, parseModmapPaths } from "./workspace-check.mjs";
import { parsePrefixTable } from "./plan-atlas.mjs";

const modmapPath = join(dirname(fileURLToPath(import.meta.url)), "modmap.mjs");

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

test("tokenCount is the house rule: bytes / 4, rounded", () => {
  assert.equal(tokenCount(144615), 36154); // the house rule: bytes / 4, rounded
  assert.equal(tokenCount(3422), 856);
  assert.equal(tokenCount(0), 0);
});

test("crateDirOf takes a crate dir or its crates/ path", () => {
  assert.equal(crateDirOf("render"), "render");
  assert.equal(crateDirOf("crates/render"), "render");
  assert.equal(crateDirOf("crates/render/"), "render");
  assert.equal(crateDirOf("not a crate"), null);
  assert.equal(crateDirOf(""), null);
});

test("mapSectionOf takes the crate's own section only", () => {
  const text = render([
    { dir: "core", pkg: "engine-core", files: [{ path: "src/lib.rs", lines: 10, doc: "core." }] },
    { dir: "render", pkg: "engine-render", files: [{ path: "src/lib.rs", lines: 20, doc: "render." }, { path: "src/a.rs", lines: 5, doc: "a." }] },
  ]);
  const slice = mapSectionOf(text, "render");
  assert.equal(slice.length, 3);
  assert.match(slice[0], /^## render \(engine-render, 2 files, 25 lines\)/);
  assert.match(slice[1], /^- `render\/src\/lib\.rs` · 20 · render\.$/);
  assert.deepEqual(mapSectionOf(text, "nope"), []);
});

test("crateMapRowOf finds the row of one crate", () => {
  const text = "| Package | Dir | Job |\n|---|---|\n| `engine-core` | `crates/core` | core. |\n| `engine-render` | `crates/render` | render. |\n";
  assert.match(crateMapRowOf(text, "render"), /^\| `engine-render` \| `crates\/render`/);
  assert.equal(crateMapRowOf(text, "nope"), "");
});

test("planIndexLineOf finds the index line of one plan", () => {
  const text = "# Plans\n\n- `render-look.md` — NOW — Look — moving fighter captures\n- `quality.md` — NOW — Look, Bodies — the engine stays fast\n";
  assert.equal(planIndexLineOf(text, "quality.md"), "- `quality.md` — NOW — Look, Bodies — the engine stays fast");
  assert.equal(planIndexLineOf(text, "nope.md"), "");
});

test("ownsCrate counts only paths inside the crate", () => {
  assert.ok(ownsCrate("crates/render", "render"));
  assert.ok(ownsCrate("crates/render/src/x.rs", "render"));
  assert.ok(!ownsCrate("crates/renderx/src/x.rs", "render"));
  assert.ok(!ownsCrate("crates/core/src/x.rs", "render"));
});

test("planForCrate picks the plan that serves most of the crate's rows", () => {
  const table = parsePrefixTable("| Row | File | Prefixes |\n|---|---|---|\n| PLN2 | `render-look.md` | RL, RP |\n| PLN10 | `quality.md` | PF, OPT |\n");
  const rows = [{ id: "RL12" }, { id: "RL13" }, { id: "RP4" }, { id: "PF1" }];
  assert.equal(planForCrate(rows, table), "render-look.md");
  assert.equal(planForCrate([{ id: "PF1" }], table), "quality.md");
  assert.equal(planForCrate([], table), "");
});

test("diffSummary counts the lines a stale map adds and drops", () => {
  const d = diffSummary("a\nb\n", "a\nc\n");
  assert.deepEqual(d, { added: 1, removed: 1 });
  assert.deepEqual(diffSummary("a\n", "a\n"), { added: 0, removed: 0 });
});

test("the render packet carries the four parts under the token budget", () => {
  const packet = buildPacket("render");
  assert.ok(packet.ok, packet.text);
  assert.ok(packet.tokens <= PACK_BUDGET_TOKENS, `packet is ${packet.tokens} tokens`);
  assert.match(packet.text, /^# packet: crates\/render/);
  assert.match(packet.text, /## Map slice \(\.opencode\/modmap\.md\)/);
  assert.match(packet.text, /## Crate row \(docs\/CRATE-MAP\.md\)/);
  assert.match(packet.text, /## Plan line \(docs\/plan\/README\.md\)/);
  assert.match(packet.text, /## Board rows \(\d+ of \d+ open rows own crates\/render/);
  assert.match(packet.text, /PASS crates\/render packet: \d+ tokens under the 6000 budget/);
  assert.ok(packet.rows.shown <= packet.rows.total);
});

test("--packet is an alias of --pack and both print the same packet", () => {
  const alias = spawnSync(process.execPath, [modmapPath, "--packet", "crates/render"], { encoding: "utf8" });
  const pack = spawnSync(process.execPath, [modmapPath, "--pack", "crates/render"], { encoding: "utf8" });
  assert.equal(alias.status, 0, alias.stderr);
  assert.equal(pack.status, 0, pack.stderr);
  assert.equal(alias.stdout, pack.stdout);
  assert.match(alias.stdout, /- tokens: \d+ \(bytes \/ 4, rounded\)/);
});

test("the packet stays under the budget for every crate with open rows", () => {
  for (const dir of ["core", "render", "combat", "game", "ui", "audio"]) {
    const packet = buildPacket(dir);
    if (!packet.ok) continue;
    assert.ok(packet.tokens <= PACK_BUDGET_TOKENS, `crates/${dir} packet is ${packet.tokens} tokens`);
  }
});

