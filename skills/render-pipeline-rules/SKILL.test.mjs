// Run: node --test skills/render-pipeline-rules/SKILL.test.mjs
// Guards the cites in this skill: a cited repo path must exist in the tree, and
// the tangent-builder cite must name the live owner. engine-assets owns tangents
// (ARCHITECTURE C3): v1.rs builds them with import_repair::recompute_tangents,
// so a cite of a local `compute_tangents` sends a reader to a symbol that is gone.
// Two drifted: the LF1 forward feed added a doc block above the WGSL, so the bind
// probe's copy slid from sky.rs:814 down to the copy line; and the P2 stall span
// `submit.rs:1089-1103` went dead when the lane rewrites moved the readback, so
// that cite names its owner by symbol. Move a drifted cite and this pin together.
import assert from "node:assert/strict";
import { existsSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
const ROOT = join(HERE, "..", "..");

// Files this skill ships.
const FILES = ["SKILL.md", join("tests", "pairs.md")];

// Live owners the repo-fact cites point at. A rename here has to move the cite.
const LIVE_OWNERS = [
  "crates/render/src/present/gpu_present/submit.rs",
  "crates/render/src/sky.rs",
  "crates/render/src/fog.rs",
  "crates/render/src/v1.rs",
  "crates/render/src/twin_shadow.rs",
  "crates/render/src/v1_skin.rs",
  "crates/game/src/bin/arena.rs",
];

// The one owner of tangents (import lane), cited by the MeshData repo fact.
const TANGENT_OWNER = "crates/assets/src/import_repair.rs";
const TANGENT_FN = "recompute_tangents";

// A `path:line` cite a check hands a reader: the file it points at and the text
// that line must carry. A `path:line` cite in the skill files that this pin does
// not know is a stray. One drifted: the LF1 forward feed added a doc block above
// the WGSL, so the bind probe's copy slid from sky.rs:814 down to the copy line
// pinned here. Move the cite and this pin together, never leave either dead.
const LINE_CITES = [
  { cite: "crates/render/src/sky.rs:989", need: /dst\[i\] = src\[i\]/ },
];

// The P2 stall cite names its owner by symbol, never by line: submit.rs is
// rewritten lane by lane (EG11 sun depth, RL52 staged draw), so a number drifts.
// The old span `submit.rs:1089-1103` held a bind-group list at HEAD, not the
// wait, so it stays out of both files.
const STALL_FILE = "crates/render/src/present/gpu_present/submit.rs";
const STALL_FN = "map_readback_f32";
const STALL_CITE = `${STALL_FILE}::${STALL_FN}`;
const DEAD_SPANS = ["submit.rs:1089-1103"];

const CITE = /(?:crates|tools|docs|content|fixtures|tests)\/[A-Za-z0-9_./-]+\.[A-Za-z0-9]+/g;
const LINE_CITE = /(?:crates|tools|docs|content|fixtures|tests)\/[A-Za-z0-9_./-]+\.[A-Za-z0-9]+:\d+/g;

function text(rel) {
  return readFileSync(join(HERE, rel), "utf8").replace(/\r\n/g, "\n");
}

function lines(rel) {
  return readFileSync(join(ROOT, rel), "utf8").split(/\r?\n/);
}

function fileOf(cite) {
  return cite.slice(0, cite.lastIndexOf(":"));
}

function lineOf(cite) {
  return Number(cite.slice(cite.lastIndexOf(":") + 1));
}

function cites(body) {
  return [...new Set([...body.matchAll(CITE)].map((m) => m[0].replace(/[.,;:)]+$/, "")))];
}

test("every cited repo path exists in the tree", () => {
  const gone = [];
  for (const rel of FILES) {
    for (const cite of cites(text(rel))) {
      if (!existsSync(join(ROOT, cite))) gone.push(`${rel}: ${cite}`);
    }
  }
  assert.deepEqual(gone, [], `cited path is gone: ${gone.join(" | ")}`);
});

test("the repo-fact cites point at live owners", () => {
  const skill = text("SKILL.md");
  for (const rel of LIVE_OWNERS) {
    assert.ok(skill.includes(rel), `SKILL.md no longer cites ${rel}`);
    assert.ok(existsSync(join(ROOT, rel)), `${rel} is gone: move this skill's cite`);
  }
});

test("the tangent builder cite names the live owner, not the dead shorthand", () => {
  const skill = text("SKILL.md");
  const withoutOwner = skill.replaceAll(TANGENT_FN, "");
  assert.ok(
    !/compute_tangents/.test(withoutOwner),
    "SKILL.md still cites the dead `compute_tangents`: name the owner `recompute_tangents`",
  );
  assert.ok(skill.includes("`" + TANGENT_FN + "`"), `SKILL.md must cite the live owner \`${TANGENT_FN}\``);
  assert.ok(existsSync(join(ROOT, TANGENT_OWNER)), `${TANGENT_OWNER} is gone: move the tangent cite`);
  assert.match(
    readFileSync(join(ROOT, TANGENT_OWNER), "utf8"),
    new RegExp(`pub fn ${TANGENT_FN}\\b`),
    `${TANGENT_OWNER} no longer defines ${TANGENT_FN}`,
  );
});

test("every line cite lands on the live line, in both files", () => {
  const pinned = new Map();
  for (const c of LINE_CITES) pinned.set(fileOf(c.cite), c);
  const cited = new Set();
  const stray = [];
  for (const rel of FILES) {
    const body = text(rel);
    for (const m of body.matchAll(LINE_CITE)) {
      const file = fileOf(m[0]);
      cited.add(file);
      if (!pinned.has(file)) {
        stray.push(`${rel}: ${m[0]}`);
        continue;
      }
      const line = lines(file)[lineOf(m[0]) - 1] ?? "";
      assert.match(line, pinned.get(file).need, `${m[0]} no longer holds ${pinned.get(file).need}: ${line.trim()}`);
    }
  }
  assert.deepEqual(stray, [], `line cite the pin does not know, pin it or drop it: ${stray.join(" | ")}`);
  for (const file of pinned.keys()) assert.ok(cited.has(file), `${file} is no longer cited in this skill`);
});

test("the P2 stall cite names the live wait by symbol, not the dead span", () => {
  const body = readFileSync(join(ROOT, STALL_FILE), "utf8");
  assert.match(
    body,
    new RegExp(`pub\\(super\\) fn ${STALL_FN}\\b`),
    `${STALL_FILE} no longer defines ${STALL_FN}: the P2 cite is dead`,
  );
  assert.match(body, /wait_indefinitely/, `${STALL_FILE} no longer blocks on wait_indefinitely`);
  for (const rel of FILES) {
    assert.ok(text(rel).includes(STALL_CITE), `${rel} no longer cites ${STALL_CITE}`);
    for (const dead of DEAD_SPANS) assert.ok(!text(rel).includes(dead), `${rel} cites the dead span ${dead}`);
  }
});
