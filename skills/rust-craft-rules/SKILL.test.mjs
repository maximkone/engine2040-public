// Run: node --test skills/rust-craft-rules/SKILL.test.mjs
// Guards the cites in this skill: every repo path it names must exist, every
// `path:line` cite must land on the line that holds the thing the check talks
// about, and the cites that went dead must stay out. Two drifted: the window
// mixer's `try_lock` left crates/game/src/bin/arena.rs (the tick moved to
// arena_frame/mod.rs, where the lock-free feed queues and counts and only the
// audible mirror drops), and the old `STEP_HOOK` static is cut, so
// play_session.rs:113 was a serde helper. Move the cite in both files, never
// leave it dead.
import assert from "node:assert/strict";
import { existsSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
const ROOT = join(HERE, "..", "..");

// Files this skill ships.
const FILES = ["SKILL.md", join("tests", "pairs.md")];

// Line cites the checks hand a reader, and the text the line must hold.
const LINE_CITES = [
  { file: "crates/game/src/bin/arena_frame/mod.rs", line: 114, need: /try_lock/ },
  { file: "crates/endpoint/src/play/play_session.rs", line: 804, need: /with_step_capture_hook/ },
  { file: "crates/core/src/alloc_probe.rs", line: 31, need: /SAFETY/ },
];

// Symbols the checks quote, and the crate file that owns them.
const SYMBOLS = {
  "crates/endpoint/src/play/play_session.rs": ["StepCaptureHook", "with_step_capture_hook", "PlaySlot"],
  "crates/geo/src/uv.rs": ["check_inputs"],
  "crates/endpoint/src/discover.rs": ["WATCH_HUB"],
  "crates/endpoint/src/packs/anim.rs": ["DOORS"],
  "crates/endpoint/src/play/requery.rs": ["lock_poisoned"],
  "crates/ecs/src/pool.rs": ["resume_unwind"],
  "crates/audio/src/feed.rs": ["FeedStats"],
  "crates/game/src/windowed.rs": ["feed_tick_audio"],
};

// Cites that went dead: the path or line they named no longer holds the thing.
const DEAD_CITES = ["crates/game/src/bin/arena.rs:518", "play_session.rs:113"];

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

test("the quoted symbols still live in their crate file", () => {
  for (const [file, names] of Object.entries(SYMBOLS)) {
    const body = readFileSync(join(ROOT, file), "utf8");
    const gone = names.filter((n) => !body.includes(n));
    assert.deepEqual(gone, [], `symbol gone from ${file}: move the cite in this skill: ${gone.join(" | ")}`);
  }
});

test("the dead cites stay out of both files", () => {
  const back = [];
  for (const rel of FILES) {
    for (const dead of DEAD_CITES) if (text(rel).includes(dead)) back.push(`${rel}: ${dead}`);
  }
  assert.deepEqual(back, [], `dead cite came back: ${back.join(" | ")}`);
});
