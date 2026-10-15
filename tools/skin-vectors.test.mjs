// tools/skin-vectors.test.mjs - IDEA-ONLY node math mirror of bevyengine/bevy (MIT) crates/bevy_pbr/src/render/skinning.wesl; no code copied. Companion to crates/render/testdata/skin_vectors.json; mirrors crates/render/src/v1_skin.rs weight and normal rules.
import { describe, it } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const data = JSON.parse(readFileSync(join(root, "crates", "render", "testdata", "skin_vectors.json"), "utf8"));
function blendPosition(p, rowJ, rowW, mats) {
  const out = [0, 0, 0];
  for (let k = 0; k < 4; k++) {
    const m = mats[rowJ[k]];
    const w = rowW[k];
    out[0] += w * (p[0] * m[0][0] + p[1] * m[1][0] + p[2] * m[2][0] + m[3][0]);
    out[1] += w * (p[0] * m[0][1] + p[1] * m[1][1] + p[2] * m[2][1] + m[3][1]);
    out[2] += w * (p[0] * m[0][2] + p[1] * m[1][2] + p[2] * m[2][2] + m[3][2]);
  }
  return out;
}
function blendNormal(n, rowJ, rowW, mats) {
  const out = [0, 0, 0];
  for (let k = 0; k < 4; k++) {
    const m = mats[rowJ[k]];
    const w = rowW[k];
    out[0] += w * (n[0] * m[0][0] + n[1] * m[1][0] + n[2] * m[2][0]);
    out[1] += w * (n[0] * m[0][1] + n[1] * m[1][1] + n[2] * m[2][1]);
    out[2] += w * (n[0] * m[0][2] + n[1] * m[1][2] + n[2] * m[2][2]);
  }
  const l2 = out[0] * out[0] + out[1] * out[1] + out[2] * out[2];
  if (l2 < 1e-12) return [0, 1, 0];
  const l = Math.sqrt(l2);
  return [out[0] / l, out[1] / l, out[2] / l];
}
function close(a, b, eps) {
  return Math.abs(a - b) <= eps;
}
describe("skin vectors", () => {
  it("weights sum to 1 within eps and stay non-negative", () => {
    for (const v of data.vectors) {
      const sum = v.weights.reduce((a, b) => a + b, 0);
      assert.ok(close(sum, 1, data.weight_sum_eps), v.id + " sum=" + sum);
      for (const w of v.weights) assert.ok(w >= 0, v.id);
      for (const j of v.joints) assert.ok(j < data.joint_count, v.id);
    }
  });
  it("blended position matches hand-computed value", () => {
    for (const v of data.vectors) {
      const got = blendPosition(v.pos, v.joints, v.weights, data.matrices);
      for (let i = 0; i < 3; i++) assert.ok(close(got[i], v.expected_pos[i], 1e-6), v.id + " axis " + i + " got " + got + " want " + v.expected_pos);
    }
  });
  it("vector count quoted matches rows", () => {
    assert.equal(data.vectors.length, data.vector_count);
    assert.equal(data.vectors.length, 3);
  });
  it("J1 boxes-to-0 padding and unit normals hold", () => {
    for (const v of data.vectors) {
      for (let k = 0; k < 4; k++) {
        if (v.weights[k] === 0) assert.equal(v.joints[k], 0, v.id + " slot " + k);
      }
      const n = blendNormal(v.normal, v.joints, v.weights, data.matrices);
      const len = Math.sqrt(n[0] * n[0] + n[1] * n[1] + n[2] * n[2]);
      assert.ok(close(len, 1, data.normal_len_eps), v.id);
      for (let i = 0; i < 3; i++) assert.ok(close(n[i], v.expected_normal[i], 1e-6), v.id);
    }
    assert.equal(data.palette.length, 8);
  });
});
