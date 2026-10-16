// cover-gap.test.mjs: pins the 5 most-changed untested files so deletion or emptying fails loudly.
// Each pin asserts the file exists, stays over a sane min size, and names its owner module.
// On failure, add a real behavior test for the named file, then keep this pin green.
import assert from "node:assert/strict";
import { existsSync, readFileSync, statSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");

const PINS = [
  {
    file: "crates/render/src/present.rs",
    changes: 29,
    minBytes: 10000,
    owner: "Owner: render lane",
    row: "cover-gap present.rs needs a real engine-render test",
  },
  {
    file: "crates/game/src/bin/arena.rs",
    changes: 25,
    minBytes: 20000,
    owner: "engine_game::App",
    row: "cover-gap arena.rs needs a real engine-game window-binary test",
  },
  {
    file: "crates/perf/src/baselines.rs",
    changes: 18,
    minBytes: 10000,
    owner: "Recorded baselines",
    row: "cover-gap baselines.rs needs a real engine-perf test",
  },
  {
    file: "crates/editor/src/commands.rs",
    changes: 17,
    minBytes: 50000,
    owner: "Command service",
    row: "cover-gap commands.rs needs a real engine-editor test",
  },
  {
    file: "crates/game/src/camera.rs",
    changes: 15,
    minBytes: 50000,
    owner: "Owner: `engine-game`",
    row: "cover-gap camera.rs needs a real engine-game camera test",
  },
];

for (const pin of PINS) {
  test(`pin ${pin.file} (${pin.changes} changes, 0 hits)`, () => {
    const full = join(ROOT, pin.file);
    assert.ok(existsSync(full), `${pin.file} missing; ${pin.row}`);
    const size = statSync(full).size;
    assert.ok(
      size > pin.minBytes,
      `${pin.file} only ${size} bytes (min ${pin.minBytes}); deleted or emptied; ${pin.row}`,
    );
    const src = readFileSync(full, "utf8");
    assert.ok(
      src.includes(pin.owner),
      `${pin.file} no longer names its owner module (${pin.owner}); ${pin.row}`,
    );
  });
}
