import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { chmodSync, mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { delimiter, join } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { ACCEPTED_SPDX, COMBAT_TARGET, DIGEST_RE, GLB_BYTES_MAX, GLB_BYTES_MIN, GLB_SHA_RE, KEYS_ALLOWED, REFUSED_MARKS, checkClip, checkManifest } from "./cook-char.mjs";

const hex16 = (n) => n.toString(16).padStart(16, "0");

const goodClip = (i) => ({
  short: `clip${i}`,
  source_action: `act${i}`,
  glb_bytes: 60000,
  cook_digest: hex16(0x1000 + i),
  glb_sha16: hex16(0x2000 + i),
  keys: 432,
  loop: true,
  root_motion: false,
});

const goodManifest = () => {
  const canon = Array.from({ length: 14 }, (_, i) => goodClip(i));
  const batchA = Array.from({ length: 16 }, (_, i) => goodClip(100 + i));
  return {
    locomotion: {
      record_100style: { spdx: "CC-BY-4.0", fetched: false },
      cmu: { fetched: false },
      synthetic_bake: { db_digest: `0x${hex16(7)}`, clips: 8, frames: 360, entries: 360 },
      first_16: { fetched: false },
    },
    combat: { target: COMBAT_TARGET, canonical_14: canon, batch_a_16: batchA, landed: 30, remaining: { open_count: 50 } },
    refused: ["geno footage", "lafan1 set", "mixamo pack", "qal rig", "unreal asset"],
  };
};

test("gates stay pinned: target, licences, keys, byte envelope", () => {
  assert.equal(COMBAT_TARGET, 80);
  assert.ok(ACCEPTED_SPDX.has("CC0-1.0") && ACCEPTED_SPDX.has("CC-BY-4.0"));
  for (const m of ["geno", "lafan1", "mixamo", "qal", "unreal"]) assert.ok(REFUSED_MARKS.includes(m));
  for (const k of [432, 475, 819, 862]) assert.ok(KEYS_ALLOWED.has(k));
  assert.ok(GLB_BYTES_MIN < 60000 && 60000 < GLB_BYTES_MAX);
  assert.match(hex16(1), DIGEST_RE);
  assert.doesNotMatch("short", DIGEST_RE);
  assert.match(hex16(2), GLB_SHA_RE);
});

test("checkClip accepts a clean clip and records its digest", () => {
  const seen = new Set();
  assert.deepEqual(checkClip(goodClip(1), seen), []);
  assert.ok(seen.has(hex16(0x1001)));
});

test("checkClip flags duplicate digest, bad bytes, bad keys, bad flags", () => {
  const seen = new Set([hex16(0x1001)]);
  const dup = { ...goodClip(1) };
  assert.match(checkClip(dup, seen).join("\n"), /duplicate cook_digest/);
  const badBytes = { ...goodClip(2), cook_digest: hex16(0x9999), glb_bytes: 10 };
  assert.match(checkClip(badBytes, new Set()).join("\n"), /glb_bytes/);
  const badKeys = { ...goodClip(3), cook_digest: hex16(0x999a), keys: 12 };
  assert.match(checkClip(badKeys, new Set()).join("\n"), /keys 12/);
  const badLoop = { ...goodClip(4), cook_digest: hex16(0x999b), loop: "yes" };
  assert.match(checkClip(badLoop, new Set()).join("\n"), /loop is not a boolean/);
});

test("checkManifest accepts the 30-row Tier A shape", () => {
  assert.deepEqual(checkManifest(goodManifest()), []);
});

test("checkManifest flags drift: target, counts, landed, refused", () => {
  const m = goodManifest();
  m.combat.target = 99;
  m.combat.canonical_14.pop();
  m.combat.landed = 1;
  m.refused = ["nothing refused"];
  const errs = checkManifest(m).join("\n");
  assert.match(errs, /combat\.target 99/);
  assert.match(errs, /canonical_14 holds 13 rows/);
  assert.match(errs, /combat\.landed/);
  assert.match(errs, /refused list must name geno/);
});

test("checkManifest refuses fetched bytes and bad bake digest", () => {
  const m = goodManifest();
  m.locomotion.record_100style.fetched = true;
  m.locomotion.synthetic_bake.db_digest = "bad";
  const errs = checkManifest(m).join("\n");
  assert.match(errs, /must not claim fetched bytes/);
  assert.match(errs, /db_digest malformed/);
});

test("checkClip and checkManifest reject empty shapes", () => {
  assert.match(checkClip({}, new Set()).join("\n"), /missing short/);
  assert.deepEqual(checkManifest(null), ["manifest is not an object"]);
  assert.match(checkManifest({}).join("\n"), /locomotion section missing/);
});

test("stub_blender_receives_exact_argv_and_reports_peak", () => {
  const dir = mkdtempSync(join(tmpdir(), "cook-stub-"));
  const script = join(dir, "cook_char.py"), input = join(dir, "in.fbx"), output = join(dir, "out.glb");
  const argvFile = join(dir, "argv.json"), stubJs = join(dir, "blender-stub.js");
  writeFileSync(script, "# stub\n");
  writeFileSync(input, "in");
  const js = `import { writeFileSync } from "node:fs";\nwriteFileSync(process.env.COOK_STUB_OUT, JSON.stringify(process.argv.slice(2)), "utf8");\n`;
  writeFileSync(stubJs, js);
  const sh = join(dir, "blender");
  writeFileSync(sh, `#!/usr/bin/env node\n${js}`);
  try { chmodSync(sh, 0o755); } catch {}
  writeFileSync(join(dir, "blender.cmd"), `@echo off\r\nnode "${stubJs}" %*\r\n`);
  const tool = fileURLToPath(new URL("./cook-char.mjs", import.meta.url));
  const r = spawnSync(process.execPath, [tool, "--cook", input, output, "--script", script], { encoding: "utf8", env: { ...process.env, PATH: dir + delimiter + process.env.PATH, COOK_STUB_OUT: argvFile } });
  assert.equal(r.status, 0, `stub cook exits 0, got ${r.status} stderr=${r.stderr} stdout=${r.stdout}`);
  assert.match(String(r.stdout), /cook: peak_ws_mb=/);
  assert.deepEqual(JSON.parse(readFileSync(argvFile, "utf8")), ["-b", "-P", script, "--", input, output]);
});

test("no_blender_exits_2_with_hint", () => {
  const dir = mkdtempSync(join(tmpdir(), "cook-empty-"));
  const input = join(dir, "in.fbx"), output = join(dir, "out.glb"), script = join(dir, "cook_char.py");
  writeFileSync(input, "in");
  writeFileSync(script, "# stub\n");
  const tool = fileURLToPath(new URL("./cook-char.mjs", import.meta.url));
  const r = spawnSync(process.execPath, [tool, "--cook", input, output, "--script", script], { encoding: "utf8", env: { ...process.env, PATH: dir } });
  assert.equal(r.status, 2, `missing blender exits 2, got ${r.status} stderr=${r.stderr} stdout=${r.stdout}`);
  assert.match(String(r.stderr), /blender/i);
  assert.match(String(r.stderr), /install/i);
});
