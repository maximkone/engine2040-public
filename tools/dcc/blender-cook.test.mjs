// Run: node --test tools/dcc/blender-cook.test.mjs
// GEO1a: the pinned Blender check. The two tests that run Blender skip on a machine without
// the pinned install; the other two never start it.
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const script = join(here, "blender-cook.mjs");
const pin = JSON.parse(readFileSync(join(here, "blender.pin.json"), "utf8"));
const OLD_EXE = "C:/Program Files/Blender Foundation/Blender 5.0/blender.exe";

function check(...args) {
  const r = spawnSync("node", [script, "--check", ...args], { encoding: "utf8" });
  return { code: r.status, out: `${r.stdout}${r.stderr}` };
}

test("the pinned Blender passes and prints its version and cold start", { skip: !existsSync(pin.exe) && "pinned Blender not installed" }, () => {
  const r = check();
  assert.equal(r.code, 0, r.out);
  const m = /^ok (5\.2[^\n]*) (\d+\.\d\d)\n$/.exec(r.out);
  assert.ok(m, `unexpected output: ${r.out}`);
  console.log(`blender-cook: ${m[1]} cold start ${m[2]} s`);
  assert.ok(Number(m[2]) < 60, "inside the 60 s run timeout");
});

test("another Blender is refused as a version mismatch", { skip: !existsSync(OLD_EXE) && "Blender 5.0 not installed" }, () => {
  const r = check("--exe", OLD_EXE);
  assert.equal(r.code, 2, r.out);
  assert.ok(r.out.includes("blender.version_mismatch"), r.out);
});

test("a missing exe exits 3 blender.missing", () => {
  const r = check("--exe", join(tmpdir(), "no-such-dir", "blender.exe"));
  assert.equal(r.code, 3, r.out);
  assert.ok(r.out.includes("blender.missing"), r.out);
});

test("a pin file with a wrong sha exits 2 before Blender starts", () => {
  const dir = mkdtempSync(join(tmpdir(), "blender-pin-"));
  try {
    // Any existing file stands in for the exe: the sha is compared first, so nothing runs.
    const bad = join(dir, "bad.pin.json");
    writeFileSync(bad, JSON.stringify({ ...pin, exe: process.execPath, exe_sha256: "0".repeat(64) }));
    const r = check("--pin", bad);
    assert.equal(r.code, 2, r.out);
    assert.ok(r.out.includes("blender.version_mismatch"), r.out);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

// GEO1b: the recipe runner. The three tests that cook run the pinned Blender
// and skip on a machine without it; the two refusal tests never start it.
import { createHash } from "node:crypto";
import { createReadStream } from "node:fs";

function cook(...args) {
  const r = spawnSync("node", [script, ...args], { encoding: "utf8" });
  return { code: r.status, out: `${r.stdout}${r.stderr}` };
}

function sha256FileSync(path) {
  return new Promise((resolve, reject) => {
    const hash = createHash("sha256");
    createReadStream(path).on("data", (c) => hash.update(c)).on("error", reject).on("end", () => resolve(hash.digest("hex")));
  });
}

const RECIPE = join(here, "recipes", "crate.json");

test("two cooks of the crate recipe give the same glb sha256", { skip: !existsSync(pin.exe) && "pinned Blender not installed" }, () => {
  const dir = mkdtempSync(join(tmpdir(), "recipe-cook-"));
  try {
    const a = join(dir, "a.glb");
    const b = join(dir, "b.glb");
    const ra = cook("--recipe", RECIPE, "--out", a);
    assert.equal(ra.code, 0, ra.out);
    const rb = cook("--recipe", RECIPE, "--out", b);
    assert.equal(rb.code, 0, rb.out);
    const sideA = JSON.parse(readFileSync(`${a}.json`, "utf8"));
    const sideB = JSON.parse(readFileSync(`${b}.json`, "utf8"));
    assert.equal(sideA.output, sideB.output, "two cooks, one glb digest");
    assert.match(sideA.output, /^[0-9a-f]{64}$/);
    assert.match(sideA.blender_version, /^[0-9a-f]{64}$/);
    assert.match(sideA.recipe, /^[0-9a-f]{64}$/);
    assert.match(sideA.script, /^[0-9a-f]{64}$/);
    assert.equal(sideA.recipe, sideB.recipe);
    assert.equal(sideA.script, sideB.script);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("the cooked glb is glTF 2 and the sidecar names its bytes", { skip: !existsSync(pin.exe) && "pinned Blender not installed" }, async () => {
  const dir = mkdtempSync(join(tmpdir(), "recipe-cook-"));
  try {
    const glb = join(dir, "crate.glb");
    const r = cook("--recipe", RECIPE, "--out", glb);
    assert.equal(r.code, 0, r.out);
    const head = readFileSync(glb).subarray(0, 12);
    assert.equal(head.subarray(0, 4).toString("latin1"), "glTF", "the glb magic");
    assert.equal(head.readUInt32LE(4), 2, "glTF version 2");
    const side = JSON.parse(readFileSync(`${glb}.json`, "utf8"));
    assert.equal(side.output, await sha256FileSync(glb), "the sidecar's output digest is the file's");
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("an unknown op exits 4 recipe.unknown_op before Blender starts", () => {
  const dir = mkdtempSync(join(tmpdir(), "recipe-cook-"));
  try {
    const bad = join(dir, "bad.json");
    writeFileSync(bad, JSON.stringify({ ops: [{ op: "sky" }] }));
    const r = cook("--recipe", bad, "--out", join(dir, "x.glb"));
    assert.equal(r.code, 4, r.out);
    assert.ok(r.out.includes("recipe.unknown_op"), r.out);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("a python key is refused the same way", () => {
  const dir = mkdtempSync(join(tmpdir(), "recipe-cook-"));
  try {
    const bad = join(dir, "python.json");
    writeFileSync(bad, JSON.stringify({ python: "bpy", ops: [{ op: "cube" }] }));
    const r = cook("--recipe", bad, "--out", join(dir, "x.glb"));
    assert.equal(r.code, 4, r.out);
    assert.ok(r.out.includes("recipe.unknown_op"), r.out);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});
