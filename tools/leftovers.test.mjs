import { test } from "node:test";
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import {
  findLeftovers,
  groupOf,
  namesPath,
  parseRows,
  parseStatus,
  rowsTouching,
  savePatches,
} from "./leftovers.mjs";

test("porcelain rows parse, renames keep the new path", () => {
  const raw = " M crates/game/src/lib.rs\0?? content/generated/props/\0R  new.rs\0old.rs\0";
  assert.deepEqual(parseStatus(raw), [
    { status: "M", path: "crates/game/src/lib.rs" },
    { status: "??", path: "content/generated/props/" },
    { status: "R", path: "new.rs" },
  ]);
});

test("a path groups by crate, tool folder or first two folders", () => {
  assert.equal(groupOf("crates/combat/src/fight.rs"), "crates/combat");
  assert.equal(groupOf("content/generated/concepts/a.jpg"), "content/generated");
  assert.equal(groupOf("content/generated/props/"), "content/generated");
  assert.equal(groupOf("tools/leftovers.mjs"), "tools");
  assert.equal(groupOf("AGENTS.md"), "(root)");
});

test("only paths idle past the bar are left over, oldest group first", () => {
  const hour = 3_600_000;
  const now = 100 * hour;
  const written = {
    "crates/game/src/lib.rs": now - 0.5 * hour,
    "content/generated/a.jpg": now - 12 * hour,
    "tools/x.mjs": now - 4 * hour,
    "gone.rs": null,
  };
  const rows = Object.keys(written).map((path) => ({ status: "M", path }));
  const groups = findLeftovers(rows, { hours: 3, now, stat: (p) => written[p] });
  assert.deepEqual(groups.map((g) => g.group), ["content/generated", "tools"]);
  assert.equal(groups[0].idleHours, 12);
});

test("a board row runs to the next item and keeps its Owns and Accept lines", () => {
  const rows = parseRows([
    {
      file: "SPRINT-1.md",
      body: [
        "# shard",
        "- [ ] ABC1 render: a thing. Owns `crates/render/src/a.rs`.",
        "  Accept: one test.",
        "  Progress: none.",
        "- [x] ABC2 done row. Owns `crates/game/src/lib.rs`.",
        "  needs: none.",
        "| table |",
      ].join("\n"),
    },
  ]);
  assert.deepEqual(rows.map((r) => r.id), ["ABC1", "ABC2"]);
  assert.match(rows[0].text, /Accept: one test/);
  assert.match(rows[0].text, /crates\/render\/src\/a\.rs/);
  assert.doesNotMatch(rows[0].text, /lib\.rs/);
});

test("a path matches at a boundary and picks the row that names it", () => {
  const rows = [
    { id: "AAA1", text: "Owns `crates/render/src/a.rs.bak`." },
    { id: "BBB2", text: "Owns `crates/render/src/a.rs`." },
    { id: "CCC3", text: "Owns `crates/render/src/`." },
  ];
  assert.equal(namesPath(rows[0].text, "crates/render/src/a.rs"), false);
  assert.equal(namesPath(rows[1].text, "crates/render/src/a.rs"), true);
  assert.deepEqual(rowsTouching(rows, ["crates/render/src/a.rs"]), ["BBB2"]);
  assert.deepEqual(rowsTouching(rows, ["crates/render/src/a.rs.bak"]), ["AAA1"]);
});

test("a saved patch carries its row and applies on a clean tree", () => {
  const dir = mkdtempSync(join(tmpdir(), "leftovers-save-"));
  try {
    const git = (...args) => execFileSync("git", args, { cwd: dir, encoding: "utf8" });
    git("init", "-q");
    git("config", "core.autocrlf", "false");
    writeFileSync(join(dir, "a.txt"), "one\n");
    writeFileSync(join(dir, "keep.txt"), "same\n");
    git("add", ".");
    git("-c", "user.email=a@b", "-c", "user.name=a", "commit", "-qm", "base");

    writeFileSync(join(dir, "a.txt"), "one\ntwo\n");
    writeFileSync(join(dir, "b.txt"), "new\n");
    const group = {
      group: "crates/demo",
      idleHours: 4,
      paths: [{ status: "M", path: "a.txt" }, { status: "??", path: "b.txt" }],
    };
    const written = savePatches([group], { root: dir, rowsOf: () => ["DEMO1"] });
    assert.equal(written.length, 1);
    const patch = readFileSync(written[0].file, "utf8");
    assert.match(patch.split("\n")[0], /crates\/demo.*row DEMO1/);
    assert.match(patch, /new file mode/);

    git("checkout", "--", ".");
    rmSync(join(dir, "b.txt"));
    execFileSync("git", ["apply", "--check", written[0].file], { cwd: dir });
    execFileSync("git", ["apply", written[0].file], { cwd: dir });
    assert.equal(readFileSync(join(dir, "a.txt"), "utf8"), "one\ntwo\n");
    assert.equal(readFileSync(join(dir, "b.txt"), "utf8"), "new\n");
    assert.equal(readFileSync(join(dir, "keep.txt"), "utf8"), "same\n");
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});
