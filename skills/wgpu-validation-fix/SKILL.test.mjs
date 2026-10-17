// Run: node --test skills/wgpu-validation-fix/SKILL.test.mjs
// Guards the cites this skill hands a reader: every repo path it names exists, every
// bare `file.rs:line` cite lands on the line that holds the thing the case talks
// about, and the cite that drifted stays out. The stride case pointed at
// baked_gi.rs:2150, which is an assert_eq! in gi_probe_bind_validates_and_scores;
// the padded copy it means sits in gi_lightmap_upload_roundtrips at 2095. Move the
// cite in both files, never leave it dead.
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
  { file: "crates/render/src/baked_gi.rs", line: 2095, need: /div_ceil\(256\)/ },
  { file: "crates/render/src/gpu.rs", line: 242, need: /CurrentSurfaceTexture::Lost/ },
  { file: "crates/render/src/present/surface.rs", line: 99, need: /pub fn recovery/ },
];

// Bare names the cases use, and the crate file that owns each. A new bare name
// must be added here, so a moved cite cannot hide behind a same-named file.
const BARE = {
  "baked_gi.rs": "crates/render/src/baked_gi.rs",
  "gpu.rs": "crates/render/src/gpu.rs",
  "surface.rs": "crates/render/src/present/surface.rs",
};

// Symbols the skill quotes, and the crate file that owns them.
const SYMBOLS = {
  "crates/render/src/baked_gi.rs": ["GiSubmitter", "ensure_buffers", "gi_lightmap_upload_roundtrips"],
  "crates/render/src/gpu.rs": ["GpuState", "CurrentSurfaceTexture"],
  "crates/render/src/ibl.rs": ["prefilter_cube_desc"],
  "crates/render/src/present/surface.rs": ["PresentError", "RecoveryAction"],
};

// Cites that went dead: the line they named no longer holds the thing.
const DEAD_CITES = ["baked_gi.rs:2150"];

function text(rel) {
  return readFileSync(join(HERE, rel), "utf8").replace(/\r\n/g, "\n");
}

function lines(rel) {
  return readFileSync(join(ROOT, rel), "utf8").split(/\r?\n/);
}

test("every repo path this skill cites exists in the tree", () => {
  const loose = [];
  for (const rel of FILES) {
    for (const m of text(rel).matchAll(/crates\/[A-Za-z0-9_./-]+/g)) {
      const cite = m[0].replace(/[.,;:)]+$/, "");
      if (!existsSync(join(ROOT, cite))) loose.push(`${rel}: ${cite}`);
    }
  }
  assert.deepEqual(loose, [], `cited path is gone: move the cite in this skill: ${loose.join(" | ")}`);
});

test("every bare file.rs:line cite lands on the live line, in both files", () => {
  const live = new Set(LINE_CITES.map((c) => `${c.file}:${c.line}`));
  const stray = [];
  for (const rel of FILES) {
    for (const m of text(rel).matchAll(/([A-Za-z0-9_-]+\.rs):(\d+)/g)) {
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
