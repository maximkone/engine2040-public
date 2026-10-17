// Run: node --test skills/frame-loop-rules/SKILL.test.mjs
// Guards the cites in this skill: every repo path it names must exist, and every
// `path:line` cite must land on the line that holds the symbol the rule talks
// about. The window mixer lock moved since this skill was written (it sat at
// arena.rs:518, then arena_frame/mod.rs:107, and is at :114 today), so a reader
// sent to an old line read the imports or the end of main instead. Move the cite
// in both files, never leave it dead.
import assert from "node:assert/strict";
import { existsSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
const ROOT = join(HERE, "..", "..");

// Files this skill ships.
const FILES = ["SKILL.md", join("tests", "pairs.md")];

// Line cites the rules hand a reader, and the text the line must hold.
const LINE_CITES = [
  { file: "crates/game/src/bin/arena_frame/mod.rs", line: 114, need: /self\.mixer\.try_lock\(\)/ },
];

// Live paths the Repo facts hand a reader. A rename here has to move the cite.
const LIVE_PATHS = [
  "crates/core/src/time.rs",
  "crates/game/src/bin/arena.rs",
  "crates/game/src/crash.rs",
  "crates/game/tests/crash_hook.rs",
  "crates/game/src/bin/arena_frame/mod.rs",
];

const LINE_CITE = /(?:crates|tools|docs)\/[A-Za-z0-9_./-]+\.(?:rs|wgsl|mjs):\d+/g;
const PATH_CITE = /(?:crates|tools|docs|content|fixtures|tests)\/[A-Za-z0-9_./-]+\.[A-Za-z0-9]+/g;

function text(rel) {
  return readFileSync(join(HERE, rel), "utf8").replace(/\r\n/g, "\n");
}

function lines(rel) {
  return readFileSync(join(ROOT, rel), "utf8").split(/\r?\n/);
}

test("every repo path this skill cites exists in the tree", () => {
  const loose = [];
  for (const rel of FILES) {
    for (const m of text(rel).matchAll(PATH_CITE)) {
      const cite = m[0].replace(/[.,;:)]+$/, "");
      if (!existsSync(join(ROOT, cite))) loose.push(`${rel}: ${cite}`);
    }
  }
  assert.deepEqual(loose, [], `cited path is gone: move the cite in this skill: ${loose.join(" | ")}`);
});

test("every line cite lands on the live line, in both files", () => {
  const live = new Set(LINE_CITES.map((c) => `${c.file}:${c.line}`));
  const stray = [];
  for (const rel of FILES) {
    for (const m of text(rel).matchAll(LINE_CITE)) {
      if (!live.has(m[0])) stray.push(`${rel}: ${m[0]}`);
    }
  }
  assert.deepEqual(stray, [], `line cite drifted off the live line: ${stray.join(" | ")}`);
  for (const c of LINE_CITES) {
    const line = lines(c.file)[c.line - 1] ?? "";
    assert.match(line, c.need, `${c.file}:${c.line} no longer holds ${c.need}: ${line.trim()}`);
  }
});

test("the Repo facts cite live paths and the crash hook it names", () => {
  const skill = text("SKILL.md");
  for (const rel of LIVE_PATHS) {
    assert.ok(skill.includes(rel), `SKILL.md no longer cites ${rel}`);
    assert.ok(existsSync(join(ROOT, rel)), `${rel} is gone: move this skill's cite`);
  }
  assert.match(readFileSync(join(ROOT, "crates/game/src/crash.rs"), "utf8"), /pub fn install_run_hook\(/);
  assert.match(readFileSync(join(ROOT, "crates/game/src/bin/arena.rs"), "utf8"), /crash::install_run_hook\(/);
});
