// Run: node --test tools/frame-check.test.mjs (also: node tools/frame-check.test.mjs)
// frame-0 fighters-present: frame-0 must show real fighters, not boxes-only placeholders.
// Boxes-only means staged_fight_frames.rs builds each body as one scaled cube
// (fight_body_mesh scales v1::cube() by FIGHT_BODY_SIZE_M, 24 verts / 36 indices).
// Real fighters mean CT7 bodies (2+ meshes over 5,000 tris per VS4 accept) drawn
// at fighter_views positions. Today the file still pins the cube, so the two
// refusal tests below FAIL. They turn green when VS4 lands and the cube pin is
// gone. Build-only: reads sources, never cargo, never GPU. Missing paths skip.
import assert from "node:assert/strict";
import { existsSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
const ROOT = join(HERE, "..");
const STAGED_REL = "crates/game/src/staged_fight_frames.rs";
const STAGED = join(ROOT, "crates", "game", "src", "staged_fight_frames.rs");
const LIB_REL = "crates/game/src/lib.rs";
const LIB = join(ROOT, "crates", "game", "src", "lib.rs");
const STAGED_CAPTURE =
  "capture step run_staged_fight_frames() (windowed::run_headless_smoke_* + arena binary frame-0 PNG/manifest)";
const LIB_CAPTURE =
  "capture step App::fighter_views draw leg (windowed::run_headless_smoke_* + arena binary)";

function skipMissing(t, rel, step) {
  const msg = `skipped: missing ${rel} (producer: ${step}); no source to check`;
  t.diagnostic(msg);
  t.skip(msg);
}

function readSource(path) {
  try {
    if (!existsSync(path)) return null;
    return readFileSync(path, "utf8");
  } catch {
    return null;
  }
}

test("frame-0 shows fighters, not boxes-only placeholders", (t) => {
  const src = readSource(STAGED);
  if (src === null) {
    skipMissing(t, STAGED_REL, STAGED_CAPTURE);
    return;
  }
  const hasSize = src.includes("FIGHT_BODY_SIZE_M");
  const hasMesh = src.includes("fight_body_mesh");
  const hasCube = src.includes("v1::cube()");
  const boxesOnly = hasSize && hasMesh && hasCube;
  assert.equal(
    boxesOnly,
    false,
    `boxes-only placeholders: fight_body_mesh scales v1::cube() by FIGHT_BODY_SIZE_M ` +
      `(frame-0 shows boxes, not CT7 fighters) at ${STAGED_REL}:147 ` +
      `(hasSize=${hasSize} hasMesh=${hasMesh} hasCube=${hasCube})`,
  );
});

test("frame-0 bodies are CT7 scale, not one cube (24 verts / 36 indices)", (t) => {
  const src = readSource(STAGED);
  if (src === null) {
    skipMissing(t, STAGED_REL, STAGED_CAPTURE);
    return;
  }
  const pins24 = src.includes("mesh.positions.len(), 24");
  const pins36 = src.includes("mesh.indices.len(), 36");
  const pinsCube = pins24 && pins36;
  assert.equal(
    pinsCube,
    false,
    `boxes-only placeholders: body mesh pinned to 24 verts / 36 indices (one cube) ` +
      `at ${STAGED_REL}:812-814; VS4 accept needs 2+ meshes over 5,000 tris ` +
      `(pins24=${pins24} pins36=${pins36})`,
  );
});

test("frame-0 dots carry 4 fighters (infrastructure present)", (t) => {
  const src = readSource(STAGED);
  if (src === null) {
    skipMissing(t, STAGED_REL, STAGED_CAPTURE);
    return;
  }
  assert.ok(src.includes("STAGED_FIGHT_FIGHTERS"), "staged fight names its fighter count");
  assert.ok(src.includes("staged_ct7_body_shas"), "staged fight pins CT7 body SHAs");
  const lib = readSource(LIB);
  if (lib === null) {
    skipMissing(t, LIB_REL, LIB_CAPTURE);
    return;
  }
  assert.ok(lib.includes("fighter_views"), "App::fighter_views exists for the draw leg");
  t.diagnostic("dots + SHAs + fighter_views present; only the body meshes are still boxes");
});
