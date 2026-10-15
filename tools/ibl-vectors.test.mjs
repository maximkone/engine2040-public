// tools/ibl-vectors.test.mjs - Idea-only from google/filament (Apache-2.0) libs/image/include/image/CubemapSH.h + CubemapSH.cpp; SH basis reimplemented, no code copied. Companion to crates/render/testdata/ibl_vectors.json; mirrors crates/render/src/ibl.rs IBL_SH_COEFFS=9.
//   node --test tools/ibl-vectors.test.mjs
import { describe, it } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const data = JSON.parse(readFileSync(join(root, "crates", "render", "testdata", "ibl_vectors.json"), "utf8"));
function shBasis(dir, c) {
  const x = dir[0];
  const y = dir[1];
  const z = dir[2];
  return [c.c0, c.c1 * y, c.c1 * z, c.c1 * x, c.c2 * x * y, c.c2 * y * z, c.c3 * (3 * z * z - 1), c.c2 * x * z, c.c4 * (x * x - y * y)];
}
function reconstruct(sh, dir, c) {
  const b = shBasis(dir, c);
  let s = 0;
  for (let i = 0; i < 9; i++) s += sh[i] * b[i];
  return s;
}
function close(a, b, eps) {
  return Math.abs(a - b) <= eps;
}
describe("ibl vectors", () => {
  it("SH reconstruct matches hand-computed ambient within epsilon", () => {
    assert.equal(data.sh.length, 9);
    assert.equal(data.sh.length, data.sh_coeffs);
    for (const v of data.vectors) {
      const got = reconstruct(data.sh, v.dir, data.basis_constants);
      assert.ok(close(got, v.expected_ambient, data.epsilon), v.id + " got " + got + " want " + v.expected_ambient);
    }
  });
  it("vector count quoted matches rows", () => {
    assert.equal(data.vectors.length, data.vector_count);
    assert.equal(data.vectors.length, 3);
  });
  it("dirs are unit and coeffs finite", () => {
    for (const v of data.vectors) {
      const l = Math.sqrt(v.dir[0] * v.dir[0] + v.dir[1] * v.dir[1] + v.dir[2] * v.dir[2]);
      assert.ok(close(l, 1, 1e-12), v.id);
      for (const x of data.sh) assert.ok(Number.isFinite(x), v.id);
    }
    assert.equal(data.sh_order, 2);
  });
});
