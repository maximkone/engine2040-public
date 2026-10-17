// Run: node --test skills/gaffer-netcode/SKILL.test.mjs
// Guards the cites this skill hands a reader: every repo path it names exists, and
// every `path:line` cite lands on the line that holds the symbol the case talks
// about. The P1 case named session.rs:201 for the `Server` authority type; that
// line is a fragment datagram copy (`Vec::with_capacity(IOQ3_HEADER_LEN ..)`), and
// `pub struct Server` sits at session.rs:326. Move the cite in both files, never
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

// Line cites the cases hand a reader, and the text the line must hold.
const LINE_CITES = [
  { file: "crates/net/src/session.rs", line: 326, need: /pub struct Server/ },
  { file: "crates/net/src/snapshot.rs", line: 38, need: /pub fn save_snapshot/ },
  { file: "crates/net/src/interp.rs", line: 212, need: /pub struct SnapshotInterpolator/ },
  { file: "crates/net/src/online.rs", line: 63, need: /pub fn resolve_path/ },
];

// Bare names a case uses, and the crate file that owns each. A new bare name must
// be added here, so a moved cite cannot hide behind a same-named file.
const BARE = {
  "session.rs": "crates/net/src/session.rs",
  "snapshot.rs": "crates/net/src/snapshot.rs",
  "interp.rs": "crates/net/src/interp.rs",
  "online.rs": "crates/net/src/online.rs",
};

// Symbols the skill quotes, and the crate file that owns them.
const SYMBOLS = {
  "crates/net/src/session.rs": ["Server", "StatePacket", "redundancy_window"],
  "crates/net/src/snapshot.rs": ["save_snapshot", "load_snapshot", "ConfirmedHistory"],
  "crates/net/src/interp.rs": ["SnapshotInterpolator"],
  "crates/net/src/online.rs": ["resolve_path"],
  "crates/net/src/delta.rs": ["Interpolator"],
};

// Cites that went dead: the line they named no longer holds the thing.
const DEAD_CITES = ["session.rs:201"];

const PATH_CITE = /(?:crates|tools|docs|content|fixtures|tests)\/[A-Za-z0-9_./-]+\.[A-Za-z0-9]+/g;
const LINE_CITE = /([A-Za-z0-9_-]+\.rs):(\d+)/g;

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
      const file = BARE[m[1]];
      if (!file) {
        stray.push(`${rel}: ${m[0]} (bare name not in the map)`);
        continue;
      }
      if (!live.has(`${file}:${m[2]}`)) stray.push(`${rel}: ${file}:${m[2]}`);
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

test("the dead cite stays out of both files", () => {
  const back = [];
  for (const rel of FILES) {
    for (const dead of DEAD_CITES) if (text(rel).includes(dead)) back.push(`${rel}: ${dead}`);
  }
  assert.deepEqual(back, [], `dead cite came back: ${back.join(" | ")}`);
});
