// Run: node --test skills/vram-budget-checks/SKILL.test.mjs
// Guards the cites in this skill: a cited repo path must exist in the tree, or
// sit on a line that names its cut. A cut module is deleted on purpose (the
// render dead-module wave rule: delete, name the cut, keep the recover command),
// so a reader is never sent to a file that is gone.
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
  "crates/render/src/uploads.rs",
  "crates/render/src/stream.rs",
];

// Live owners the fixed cites point at. A rename here has to move the cite.
const LIVE_OWNERS = [
  "crates/render/src/present/cache.rs",
  "crates/assets/src/stream.rs",
  "crates/assets/src/registry.rs",
  "crates/perf/src/budget.rs",
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

test("each cut note names the wave, the date and the recover command", () => {
  const skill = text("SKILL.md");
  for (const path of CUT_PATHS) {
    assert.ok(cutNoteLines(skill, path).length > 0, `SKILL.md cites ${path} without a cut note`);
  }
  assert.match(skill, /cut with the render dead-module wave/);
  assert.match(skill, /2026-10-17/);
  assert.match(skill, /git log --follow -- crates\/render\/src\/uploads\.rs/);
  assert.match(skill, /git log --follow -- crates\/render\/src\/stream\.rs/);
});

test("the replacement cites point at live owners", () => {
  const skill = text("SKILL.md");
  for (const rel of LIVE_OWNERS) {
    assert.ok(skill.includes(rel), `SKILL.md no longer cites ${rel}`);
    assert.ok(existsSync(join(ROOT, rel)), `${rel} is gone: move this skill's cite`);
  }
});
