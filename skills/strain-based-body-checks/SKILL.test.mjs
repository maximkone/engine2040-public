// Run: node --test skills/strain-based-body-checks/SKILL.test.mjs
// Guards the cites in this skill: a cited repo path must exist in the tree, or
// sit on a line that names its cut. A cut module is deleted on purpose (physics
// dead-module wave rule: delete, name the cut, keep the recover command), so a
// reader is never sent to a file that is gone.
import assert from "node:assert/strict";
import { existsSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
const ROOT = join(HERE, "..", "..");

// Files this skill ships.
const FILES = ["SKILL.md", join("tests", "pairs.md")];

// Cited paths the tree no longer holds. Each one must carry its cut note.
const CUT_PATHS = [
  "crates/physics/src/cloth.rs",
  "crates/physics/tests/cloth_tape.rs",
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

test("the cut note names the wave, the date and the recover command", () => {
  const skill = text("SKILL.md");
  for (const path of CUT_PATHS) {
    assert.ok(cutNoteLines(skill, path).length > 0, `SKILL.md cuts ${path} without a note`);
  }
  assert.match(skill, /cut with the physics dead-module wave on 2026-10-16/);
  assert.match(skill, /git log --follow -- crates\/physics\/src\/cloth\.rs/);
});

test("the cut list matches the tree: every declared cut path is really gone", () => {
  for (const path of CUT_PATHS) {
    assert.strictEqual(existsSync(join(ROOT, path)), false, `${path} is back: drop it from CUT_PATHS`);
  }
});
