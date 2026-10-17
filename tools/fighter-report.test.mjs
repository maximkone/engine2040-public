// Run: node --test tools/fighter-report.test.mjs
// CT41 LOD table, CT32 clip dedupe manifest, CT33 sidecar conformance. Fixture
// trees prove each gate goes red; the real tree must be clean and current.
import assert from "node:assert/strict";
import { mkdirSync, mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { readJson, syncBlocks } from "./content-lib.mjs";
import {
  checkDedupe, checkLod, checkSidecars, clipRows, dedupeBody, fighterBlocks, fighterProblems, lodBody, lodTotals,
} from "./fighter-report.mjs";

const lod = (level, tris, bytes, hull = true) => ({ level, tris, bytes, hull_contains: hull });
const hero = () => ({
  cook_x2_identical: true, gate: "check_chain:ok", class: "Hero", class_max_tris: 40000,
  meshes: [
    { index: 0, base_tris: 800, lods: [lod(0, 800, 800), lod(1, 400, 400), lod(2, 200, 200), lod(3, 100, 100)] },
    { index: 1, base_tris: 200, lods: [lod(0, 200, 200), lod(1, 100, 100), lod(2, 50, 50), lod(3, 25, 25)] },
  ],
});

test("a clean bake has no LOD problems and the table carries its totals", () => {
  assert.deepEqual(checkLod(hero()), []);
  assert.deepEqual(lodTotals(hero()).map((t) => t.tris), [1000, 500, 250, 125]);
  const body = lodBody(hero());
  assert.match(body, /\| LOD1 \| 500 \| 500 \| PENDING \|/);
  assert.match(body, /all-LOD0 64 x 1000 = 0\.1 MiB/);
});

test("CT41: each broken bake rule is named", () => {
  const h = hero();
  h.cook_x2_identical = false;
  h.gate = "check_chain:fail";
  h.meshes[0].lods[2] = lod(2, 400, 200, false);
  h.meshes[1].base_tris = 41000;
  const out = checkLod(h).join("\n");
  assert.match(out, /cook x2 is not identical/);
  assert.match(out, /gate is check_chain:fail/);
  assert.match(out, /mesh 0: tris do not strictly decrease \(800>400>400>100\)/);
  assert.match(out, /mesh 0 LOD2: hull does not contain/);
  assert.match(out, /mesh 1: 41000 tris over class max 40000/);
  assert.match(out, /mesh 1: LOD0 200 tris differs from base 41000/);
  assert.match(out, /LOD2: ratio/);
});

/** Two clips, two variants each, with GLBs and sidecars. */
function fixture() {
  const dir = mkdtempSync(join(tmpdir(), "fighter-report-"));
  for (const d of ["tools/ct7proof/src", "glb/clips", "sidecars", "lod"]) mkdirSync(join(dir, d), { recursive: true });
  writeFileSync(join(dir, "tools/ct7proof/src/main.rs"), 'const CLIPS = [\n    ("human_male_idle", "idle"),\n    ("human_male_cast_main_hand_melee", "attack_1h_1"),\n];');
  const clip = (variant, short, digest, dur, extra = {}) => ({ variant, short, duration_s: dur, keys: 10, digest, loop: "false", root_motion: "false", ...extra });
  const clips = [
    clip("hero", "idle", "aaaaaaaaaaaaaaaa", "0.792", { loop: "true" }), clip("opp", "idle", "aaaaaaaaaaaaaaaa", "0.792", { loop: "true" }),
    clip("hero", "attack_1h_1", "bbbbbbbbbbbbbbbb", "0.417"), clip("opp", "attack_1h_1", "bbbbbbbbbbbbbbbb", "0.417"),
  ];
  writeFileSync(join(dir, "manifest.json"), JSON.stringify({ clips }));
  writeFileSync(join(dir, "glb/clips/human_male_idle.glb"), "glb-idle");
  writeFileSync(join(dir, "glb/clips/human_male_cast_main_hand_melee.glb"), "glb-attack");
  const feet = { left: "foot_l", right: "foot_r" };
  const side = (name, extra) => writeFileSync(join(dir, "sidecars", `${name}.json`), JSON.stringify({ clip: name, loop: false, root_motion: false, ground_y: 0.02, feet, ...extra }, null, 2));
  side("idle", { loop: true, tags: ["idle"] });
  side("attack_1h_1", { tags: ["attack"], hit_windows: [{ socket: "SOCKET_wep_R", start_s: 0.08, end_s: 0.22 }] });
  return dir;
}

test("a clean fixture passes dedupe and sidecar gates; the table names 2 unique digests over 4 slots", () => {
  const dir = fixture();
  assert.deepEqual(checkDedupe(dir), []);
  assert.deepEqual(checkSidecars(dir), []);
  assert.equal(clipRows(dir).length, 2);
  assert.match(dedupeBody(dir), /4 clip slots \(2 variants x 2 clips\) hold 2 unique cook digests/);
});

test("CT32: a clip whose variants disagree, a missing GLB and flag drift are named", () => {
  const dir = fixture();
  const m = readJson(join(dir, "manifest.json"));
  m.clips[1].digest = "cccccccccccccccc";
  m.clips[2].root_motion = "true";
  writeFileSync(join(dir, "manifest.json"), JSON.stringify(m));
  writeFileSync(join(dir, "tools/ct7proof/src/main.rs"), '("human_male_idle", "idle"),\n("human_male_cast_main_hand_melee", "attack_1h_1"),\n("human_male_run", "run"),');
  const out = checkDedupe(dir).join("\n");
  assert.match(out, /idle: 2 cook digests across 2 slots, want 1/);
  assert.match(out, /attack_1h_1: sidecar root_motion false differs from manifest true/);
  assert.match(out, /run: glb\/clips\/human_male_run\.glb missing/);
  assert.match(out, /run: no hero pin in manifest\.json/);
});

test("CT33: windows past the clip, bad sockets, missing feet, no attack window and bad edges are named", () => {
  const dir = fixture();
  const put = (name, doc) => writeFileSync(join(dir, "sidecars", `${name}.json`), JSON.stringify(doc));
  put("attack_1h_1", { clip: "attack_1h_1", tags: ["attack"], ground_y: 0.02, feet: {}, hit_windows: [{ socket: "wep_R", start_s: 0.1, end_s: 0.9 }], traversal: [{ edge: "fly", time_s: 0 }] });
  put("idle", { clip: "walk", tags: ["attack"], feet: { left: "a", right: "b" } });
  const out = checkSidecars(dir).join("\n");
  assert.match(out, /attack_1h_1\.json: LM2 reads contacts/);
  assert.match(out, /attack_1h_1\.json: socket wep_R is not SOCKET_<name>/);
  assert.match(out, /attack_1h_1\.json: window 0\.1\.\.0\.9 is outside the 0\.417 s clip/);
  assert.match(out, /attack_1h_1\.json: traversal edge fly is off the vocabulary/);
  assert.match(out, /idle\.json: clip is walk/);
  assert.match(out, /idle\.json: ground_y is required/);
  assert.match(out, /idle\.json: attack clip has no hit window for CB4/);
});

test("a sidecar set that does not match the manifest clips is named", () => {
  const dir = fixture();
  writeFileSync(join(dir, "sidecars", "extra.json"), "{}");
  assert.match(checkSidecars(dir).join("\n"), /sidecars 3 files do not match the 2 manifest clips/);
});

test("real tree: 14 clips, one digest each, clean sidecars, LOD bake and blocks current", () => {
  assert.equal(clipRows().length, 14);
  assert.deepEqual(fighterProblems(), []);
  assert.deepEqual(syncBlocks(fighterBlocks()), []);
});
