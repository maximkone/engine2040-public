// Run: node --test tools/batch-a.test.mjs
// CT42: the 16 batch-a combat clips are vendored under content/fighters/incoming/batch-a/ with one
// sidecar each, and the intake note pins every GLB by sha16 (the per-file provenance rows).
// The retarget cook, dedupe and receipt match are crates/assets/tests/it/retarget_batch_a.rs.
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import test from "node:test";
import { REPO } from "./content-lib.mjs";

const BATCH = join(REPO, "content", "fighters", "incoming", "batch-a");
const intake = readFileSync(join(BATCH, "BATCH-A-INTAKE.md"), "utf8");
/** `batch-a/glb/<name>.glb (sha16 <hex>)` rows of the intake note, the r3 table and the r5 table. */
const pinned = new Map([...intake.matchAll(/batch-a\/glb\/(\S+)\.glb \(sha16 ([0-9a-f]{16})\)/g), ...intake.matchAll(/^\| \w+ \| (\S+) \| [\d-]+ \| \d+ \| ([0-9a-f]{16}) \|$/gm)].map((m) => [m[1], m[2]]));

test("each of the 16 GLBs is pinned by sha16 in the intake note and has a sidecar named after its clip", () => {
  const glbs = readdirSync(join(BATCH, "glb")).filter((f) => f.endsWith(".glb")).map((f) => f.slice(0, -4));
  assert.equal(glbs.length, 16);
  for (const name of glbs) {
    const sha = createHash("sha256").update(readFileSync(join(BATCH, "glb", `${name}.glb`))).digest("hex");
    assert.equal(pinned.get(name), sha.slice(0, 16), `${name}: the intake note pins this GLB`);
  }
  const sidecars = readdirSync(join(BATCH, "sidecars")).filter((f) => f.endsWith(".json"));
  assert.equal(sidecars.length, 16);
  for (const f of sidecars) {
    const tags = JSON.parse(readFileSync(join(BATCH, "sidecars", f), "utf8"));
    assert.equal(`${tags.clip}.json`, f, "the clip field is the file stem");
    assert.deepEqual(tags.feet, { left: "foot_l", right: "foot_r" });
  }
});
