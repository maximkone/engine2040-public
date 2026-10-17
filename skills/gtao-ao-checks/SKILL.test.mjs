// Run: node --test skills/gtao-ao-checks/SKILL.test.mjs
// Guards the cites in this skill: every repo path it names must exist, and every
// `path:line` cite must land on the line that holds the thing the check talks
// about. arena.rs moved the flag since this skill was written (the `--look-probes`
// const sat at :92 and is at :98 today), so a reader sent to the old line read
// DEFAULT_TICKS instead. Move the cite in both files, never leave it dead.
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
  { file: "crates/render/src/shaders/blit.wgsl", line: 49, need: /ao_tex/ },
  { file: "crates/game/src/bin/arena.rs", line: 98, need: /--look-probes/ },
];

// Symbols the checks quote, and the crate file that owns them.
const GTAO_RS = "crates/render/src/gtao.rs";
const SYMBOLS = [
  "GtaoAo",
  "GtaoConfig",
  "GTAO_HALF_DIV",
  "GTAO_HALF_PASS",
  "GTAO_UPSAMPLE_PASS",
  "GTAO_HALF_TRANSIENT_BUDGET_BYTES",
  "GTAO_SLICES",
  "GTAO_TAPS",
  "GTAO_RADIUS_MAX_PX",
  "GTAO_FALLOFF_MAX_M",
  "GTAO_BUDGET_MS",
  "GTAO_GPU_PARITY_TOL",
  "radius_px",
  "falloff_m",
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

test("the quoted GTAO symbols still live in gtao.rs", () => {
  const body = readFileSync(join(ROOT, GTAO_RS), "utf8");
  const gone = SYMBOLS.filter((s) => !body.includes(s));
  assert.deepEqual(gone, [], `symbol gone from ${GTAO_RS}: move the cite in this skill: ${gone.join(" | ")}`);
});
