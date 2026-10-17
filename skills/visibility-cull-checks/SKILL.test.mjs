// Run: node --test skills/visibility-cull-checks/SKILL.test.mjs
// Guards the cites in this skill: a cited repo path must exist in the tree, or
// sit on a line that names its cut. A scenario in tests/pairs.md names the file
// a bad diff lands in, so a path that moved (or never existed) sends a reader
// to nothing; fixing it means pointing at the live owner.
import assert from "node:assert/strict";
import { existsSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
const ROOT = join(HERE, "..", "..");

// Files this skill ships.
const FILES = ["SKILL.md", join("tests", "pairs.md")];

// Live owners the fixed cites point at. A rename or a module move has to move
// the cite, so this test goes red instead of the scenario quietly rotting.
const LIVE_OWNERS = [
  "crates/render/src/draw.rs",
  "crates/render/src/core.rs",
  "crates/arena/src/zones.rs",
];

const CITE = /(?:crates|tools|docs|content|fixtures|tests)\/[A-Za-z0-9_./-]+\.[A-Za-z0-9]+/g;

function text(rel) {
  return readFileSync(join(HERE, rel), "utf8").replace(/\r\n/g, "\n");
}

function cites(body) {
  return [...new Set([...body.matchAll(CITE)].map((m) => m[0].replace(/[.,;:)]+$/, "")))];
}

/** Lines that name `path` and say `cut`, so the reader learns the file is gone. */
function cutNoteLines(body, path) {
  return body.split("\n").filter((l) => l.includes(path) && /\bcut\b/i.test(l));
}

test("a cited path that is gone carries its cut note in the same file", () => {
  const loose = [];
  for (const rel of FILES) {
    const body = text(rel);
    for (const cite of cites(body)) {
      if (existsSync(join(ROOT, cite))) continue;
      if (cutNoteLines(body, cite).length > 0) continue;
      loose.push(`${rel}: ${cite}`);
    }
  }
  assert.deepEqual(loose, [], `cited path is gone with no cut note beside it: ${loose.join(" | ")}`);
});

test("the scenario cites point at live owners", () => {
  const body = text(join("tests", "pairs.md"));
  for (const rel of LIVE_OWNERS) {
    assert.ok(body.includes(rel), `tests/pairs.md no longer cites ${rel}`);
    assert.ok(existsSync(join(ROOT, rel)), `${rel} is gone: move this skill's cite`);
  }
});

test("no scenario sends a reader to a path the tree never held", () => {
  const body = text(join("tests", "pairs.md"));
  for (const dead of [
    "crates/spatial/src/cull.rs",
    "crates/render/src/light_cull.rs",
    "crates/game/src/zones.rs",
  ]) {
    assert.equal(existsSync(join(ROOT, dead)), false, `${dead} is back: drop this pin`);
  }
});
