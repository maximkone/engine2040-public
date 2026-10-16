// Run: node --test tools/dcc/blender/verify_op.test.mjs
// GEO1 verify after each op: the python script checks one recipe op.
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { dirname, join } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const dir = dirname(fileURLToPath(import.meta.url));
const script = join(dir, "verify_op.py");

function run(args) {
  try {
    const out = execFileSync("python", [script, ...args], { encoding: "utf8" });
    return { status: 0, out };
  } catch (e) {
    return { status: e.status ?? 1, out: (e.stdout ?? "") + (e.stderr ?? "") };
  }
}

test("help names the allow list and exit 5", () => {
  const r = run(["--help"]);
  assert.equal(r.status, 0);
  assert.ok(r.out.includes("--op"));
  assert.ok(r.out.includes("exit 5"));
});

test("list-ops holds 8 ops with the 4 new ones", () => {
  const r = run(["--list-ops"]);
  assert.equal(r.status, 0);
  const ops = r.out.trim().split("\n").map((s) => s.trim()).filter(Boolean);
  assert.equal(ops.length, 8);
  for (const want of ["boolean", "subdivide", "decimate", "uv_smart"]) {
    assert.ok(ops.includes(want), `missing ${want}`);
  }
});

test("matching counts pass", () => {
  const r = run(["--op", "boolean", "--objects", "2", "--tris", "100", "--exp-objects", "2", "--exp-tris", "100"]);
  assert.equal(r.status, 0);
  assert.ok(r.out.includes("recipe.verify_ok"));
});

test("wrong tri count exits 5 with recipe.verify_failed", () => {
  const r = run(["--op", "boolean", "--objects", "2", "--tris", "99", "--exp-objects", "2", "--exp-tris", "100"]);
  assert.equal(r.status, 5);
  assert.ok(r.out.includes("recipe.verify_failed"));
  assert.ok(r.out.includes("op=boolean"));
  assert.ok(r.out.includes("check=tri_count"));
});

test("wrong object count exits 5 with the check name", () => {
  const r = run(["--op", "subdivide", "--objects", "1", "--tris", "50", "--exp-objects", "2", "--exp-tris", "50"]);
  assert.equal(r.status, 5);
  assert.ok(r.out.includes("check=object_count"));
});

test("unknown op exits 5", () => {
  const r = run(["--op", "nope", "--objects", "1", "--tris", "1", "--exp-objects", "1", "--exp-tris", "1"]);
  assert.equal(r.status, 5);
  assert.ok(r.out.includes("check=unknown_op"));
});

test("nan vertex exits 5", () => {
  const r = run(["--op", "decimate", "--objects", "1", "--tris", "10", "--exp-objects", "1", "--exp-tris", "10", "--verts", "NaN,0,0;0,0,0"]);
  assert.equal(r.status, 5);
  assert.ok(r.out.includes("check=nan_vertices"));
});
