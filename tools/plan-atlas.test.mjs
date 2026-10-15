// Run: node --test tools/plan-atlas.test.mjs
// plan-atlas queries over a sandbox repo: VISION map, prefix table, crates,
// generated ATLAS text, and the --write/--row/--crate/--plan/--find CLI.
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { copyFileSync, existsSync, mkdirSync, mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";
import {
  buildAtlas,
  buildIndex,
  cratesIn,
  parsePrefixTable,
  parseVisionPlansMap,
  prefixOf,
  section7Ids,
} from "./plan-atlas.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));
const TOOL = join(HERE, "plan-atlas.mjs");
const SLICE = join(HERE, "board-slice.mjs");
const GUARD = join(HERE, "steal-next.mjs");
const ARGV = join(HERE, "argv.mjs");

const VISION_SNIP = [
  "## Plans: which plan serves which part of the vision",
  "| Plan | Serves (Scorecard rows) | Board prefixes and notes |",
  "|---|---|---|",
  "| `docs/plan/ATLAS.md` | none: the generated map of every plan, its rows, crates and health | `node tools/plan-atlas.mjs --write`; never hand-edited |",
  "| `docs/plan/render-look.md` | Look on the RTX 3050 | RL, RP; read 2nd |",
  "",
].join("\n");

const README_SNIP = [
  "| Row | File | Prefixes |",
  "|---|---|---|",
  "| PLN3 | `render-look.md` | RL, RP |",
  "",
].join("\n");

const BOARD_SNIP = [
  "# SPRINT",
  "### Look (owner lane, `docs/plan/render-look.md`)",
  "- [x] RL0 base: done. needs: none.",
  "- [ ] RL1 render: ready row, needs: RL0.",
  "- [ ] RL2 render: waits, needs: RL1.",
  "",
].join("\n");

const PLAN_SNIP = [
  "# Render look - plan to the VISION bar",
  "## 1. Where it stands",
  "## 3. Target design",
  "The pass reads `crates/render/src/gtao.rs` and `crates/game/src/look.rs`.",
  "## 7. Board tasks",
  "- [ ] RL1 render: ready row. needs: RL0.",
  "- [ ] RL9 render: later row. needs: RL1.",
  "",
].join("\n");

function sandbox() {
  const root = mkdtempSync(join(tmpdir(), "plan-atlas-"));
  mkdirSync(join(root, "tools"), { recursive: true });
  mkdirSync(join(root, "docs", "plan"), { recursive: true });
  copyFileSync(TOOL, join(root, "tools", "plan-atlas.mjs"));
  copyFileSync(SLICE, join(root, "tools", "board-slice.mjs"));
  copyFileSync(GUARD, join(root, "tools", "steal-next.mjs"));
  copyFileSync(ARGV, join(root, "tools", "argv.mjs"));
  writeFileSync(join(root, "docs", "SPRINT.md"), BOARD_SNIP);
  writeFileSync(join(root, "docs", "VISION.md"), VISION_SNIP);
  writeFileSync(join(root, "docs", "plan", "README.md"), README_SNIP);
  writeFileSync(join(root, "docs", "plan", "render-look.md"), PLAN_SNIP);
  return root;
}

function run(root, args) {
  const r = spawnSync(process.execPath, [join(root, "tools", "plan-atlas.mjs"), ...args], { encoding: "utf8", cwd: root });
  return { code: r.status, out: r.stdout, err: r.stderr };
}

test("VISION Plans map names the generated ATLAS row with its --write command", () => {
  const rows = parseVisionPlansMap(VISION_SNIP);
  const atlas = rows.find((r) => r.file === "docs/plan/ATLAS.md");
  assert.ok(atlas, "ATLAS.md row present");
  assert.match(atlas.serves, /generated map of every plan, its rows, crates and health/);
  assert.match(atlas.notes, /node tools\/plan-atlas\.mjs --write/);
  assert.match(atlas.notes, /never hand-edited/);
});

test("prefix table maps RL to render-look; prefixOf splits RL25", () => {
  const { prefixToFiles } = parsePrefixTable(README_SNIP);
  assert.deepEqual(prefixToFiles.get("RL"), ["render-look.md"]);
  assert.deepEqual(prefixToFiles.get("RP"), ["render-look.md"]);
  assert.equal(prefixOf("RL25"), "RL");
  assert.equal(prefixOf("EA1a"), "EA");
});

test("crates and section-7 ids come from the plan text", () => {
  assert.deepEqual(cratesIn(PLAN_SNIP), ["game", "render"]);
  assert.deepEqual(section7Ids(PLAN_SNIP), ["RL1", "RL9"]);
});

test("buildAtlas lists rows, crates and health, marked never hand-edited", () => {
  const index = buildIndex(sandbox());
  const text = buildAtlas(index, "2026-10-08");
  assert.match(text, /Never edit by hand: fix the plan, then regenerate/);
  assert.match(text, /\| \[`render-look`\]\(render-look\.md\) \| 2 \/ 1 \/ 1 \| game, render \|/);
  assert.match(text, /1 section-7 row\(s\) off the board \(RL9\)/);
  assert.match(text, /\| RL \| `render-look` \|/);
  assert.match(text, /\| `render` \| `render-look` \(2\) \|/);
});

test("CLI --write regenerates ATLAS.md in the sandbox", () => {
  const root = sandbox();
  const r = run(root, ["--write"]);
  assert.equal(r.code, 0);
  assert.match(r.out, /wrote .*ATLAS\.md \(2 plans,/);
  const out = readFileSync(join(root, "docs", "plan", "ATLAS.md"), "utf8");
  assert.match(out, /render-look/);
  assert.match(out, /Never edit by hand/);
  assert.ok(existsSync(join(root, "tools", "plan-atlas.mjs")), "sandbox keeps the tool");
});

test("CLI --row/--crate/--plan/--find answer from the sandbox board and plans", () => {
  const root = sandbox();
  const row = run(root, ["--row", "RL1"]);
  assert.equal(row.code, 0);
  assert.match(row.out, /RL1: plan render-look, status ready/);
  const crate = run(root, ["--crate", "render"]);
  assert.equal(crate.code, 0);
  assert.match(crate.out, /render-look \(2 open\): game, render/);
  const plan = run(root, ["--plan", "render-look"]);
  assert.equal(plan.code, 0);
  assert.match(plan.out, /rows: 2 open \/ 1 ready \/ 1 done/);
  assert.match(plan.out, /off the board: RL9/);
  const find = run(root, ["--find", "gtao"]);
  assert.equal(find.code, 0);
  assert.match(find.out, /render-look: 3\. Target design:/);
});

test("CLI --help exits 0; unknown row exits 1; bare call is a usage error", () => {
  const root = sandbox();
  assert.equal(run(root, ["--help"]).code, 0);
  assert.match(run(root, ["--help"]).out, /--write/);
  assert.equal(run(root, ["--row", "ZZ9"]).code, 1);
  assert.equal(run(root, []).code, 2);
});

test("promise index: knapping/tri-tier/volumetrics indexed or explicitly absent", () => {
  const root = sandbox();
  for (const term of ["knapping", "tri-tier", "volumetrics"]) {
    const r = run(root, ["--find", term]);
    assert.equal(r.code, 1, term + " explicitly absent in sandbox");
    assert.ok(r.out.includes("0 plan"), term + " absent line");
  }
});

test("a needs: on a row moved to docs/archive is ticked: --row reports ready, not open", () => {
  const root = sandbox();
  mkdirSync(join(root, "docs", "archive"), { recursive: true });
  const board = readFileSync(join(root, "docs", "SPRINT.md"), "utf8");
  writeFileSync(join(root, "docs", "SPRINT.md"), `${board}\n- [ ] RL40 render: archived need probe. Tier A. Size S. Owns \`crates/render/src/x.rs\`. needs: RL39.\n`);
  writeFileSync(join(root, "docs", "archive", "SPRINT-done-2026-10-10.md"), "# SPRINT done\n\n- [x] RL39 render: moved to the archive. needs: none.\n");
  const r = run(root, ["--row", "RL40"]);
  assert.equal(r.code, 0);
  assert.match(r.out, /RL40: plan render-look, status ready/);
});
