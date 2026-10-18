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
