// node --test tools/asset-gen/character.test.mjs
import { test } from "node:test";
import assert from "node:assert/strict";
import { join } from "node:path";
import {
  argValue, chooseLane, gateVerdict, mpfbPlan, pngSize, glbImages, budgetCheck,
  receiptLine, buildReceipt, PAGE, FILL_MIN, FILL_MAX,
} from "./character.mjs";

test("argValue reads a flag's value, or the fallback", () => {
  const argv = ["node", "x", "--name", "probe", "--height"];
  assert.equal(argValue(argv, "name"), "probe");
  assert.equal(argValue(argv, "height", "1.9"), "1.9");
  assert.equal(argValue(argv, "bulk", "1.0"), "1.0");
});

test("chooseLane: --from wins, --concept means the image lane, nothing means none", () => {
  assert.equal(chooseLane(["--from", "mpfb"]), "mpfb");
  assert.equal(chooseLane(["--from", "concept", "--concept", "a.jpg"]), "concept");
  assert.equal(chooseLane(["--concept", "a.jpg"]), "concept");
  assert.equal(chooseLane(["--name", "x"]), null);
  assert.throws(() => chooseLane(["--from", "sf3d"]), /--from takes concept or mpfb/);
});

test("gateVerdict: the band edges pass, outside them says why", () => {
  assert.equal(FILL_MIN, 0.40);
  assert.equal(FILL_MAX, 0.70);
  assert.equal(gateVerdict(0.40), "pass");
  assert.equal(gateVerdict(0.70), "pass");
  assert.equal(gateVerdict(0.4892), "pass");
  assert.equal(gateVerdict(0.39), "fail_too_sparse");
  assert.equal(gateVerdict(0.7001), "fail_too_solid");
});

test("mpfbPlan: the five steps in the proven order, with the proven arguments", () => {
  const out = join("C:", "out");
  const steps = mpfbPlan({ root: "R", blender: "B.exe", out, faction: "wardens", height: 1.9, bulk: 1, tris: 2000 });
  assert.deepEqual(steps.map((s) => s.label), [
    "body (MPFB, headless)", "proportion pass", "style cook", "30 m gate", "22-bone rig",
  ]);
  const [body, prop, cook, gate, rig] = steps;
  assert.ok(body.args.includes("--python"));
  assert.ok(body.args.some((a) => a.endsWith("mpfb_body.py")));
  assert.deepEqual(body.args.slice(-4), ["--height", "1.9", "--bulk", "1"]);
  assert.ok(prop.args.join(" ").includes("--faction wardens"));
  assert.ok(prop.args.some((a) => a.endsWith("proportion_pass.py")));
  assert.ok(cook.args.join(" ").includes("--faction wardens --tris 2000"));
  assert.ok(cook.args.some((a) => a.endsWith("style_cook.py")));
  assert.ok(gate.args.some((a) => a.endsWith(join("cook", "lod0.glb"))));
  assert.ok(gate.args.some((a) => a.endsWith("mesh_silhouette.py")));
  assert.ok(rig.args.some((a) => a.endsWith("rig22.py")));
  assert.ok(rig.writes.endsWith(join("rig", "rigged.glb")));
  assert.ok(gate.writes.endsWith(join("gate", "silhouette.json")));
  for (const step of steps) assert.equal(step.command, "B.exe");
});

test("pngSize reads the IHDR of a PNG and refuses anything else", () => {
  const png = Buffer.alloc(24);
  Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]).copy(png, 0);
  png.writeUInt32BE(512, 16);
  png.writeUInt32BE(256, 20);
  assert.deepEqual(pngSize(png), { width: 512, height: 256 });
  assert.equal(pngSize(Buffer.from("not a png at all, just text here!")), null);
  assert.equal(pngSize(null), null);
});

// A minimal GLB: a JSON chunk naming one image in a bufferView, then the BIN chunk.
function fakeGlb(imageBytes) {
  const json = Buffer.from(JSON.stringify({
    asset: { version: "2.0" },
    images: [{ bufferView: 0, mimeType: "image/png" }],
    bufferViews: [{ buffer: 0, byteOffset: 0, byteLength: imageBytes.length }],
    buffers: [{ byteLength: imageBytes.length }],
  }));
  const jsonPad = Buffer.concat([json, Buffer.alloc((4 - (json.length % 4)) % 4, 0x20)]);
  const binPad = Buffer.concat([imageBytes, Buffer.alloc((4 - (imageBytes.length % 4)) % 4)]);
  const header = Buffer.alloc(12);
  header.writeUInt32LE(0x46546c67, 0);
  header.writeUInt32LE(2, 4);
  const total = 12 + 8 + jsonPad.length + 8 + binPad.length;
  header.writeUInt32LE(total, 8);
  const jsonHead = Buffer.alloc(8);
  jsonHead.writeUInt32LE(jsonPad.length, 0);
  jsonHead.writeUInt32LE(0x4e4f534a, 4);
  const binHead = Buffer.alloc(8);
  binHead.writeUInt32LE(binPad.length, 0);
  binHead.writeUInt32LE(0x004e4942, 4);
  return Buffer.concat([header, jsonHead, jsonPad, binHead, binPad]);
}

test("glbImages finds the embedded atlas bytes", () => {
  const png = Buffer.alloc(24);
  Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]).copy(png, 0);
  png.writeUInt32BE(512, 16);
  png.writeUInt32BE(512, 20);
  const images = glbImages(fakeGlb(png));
  assert.equal(images.length, 1);
  assert.equal(images[0].mimeType, "image/png");
  assert.deepEqual(pngSize(images[0].bytes), { width: 512, height: 512 });
  assert.throws(() => glbImages(Buffer.from("nope nope nope nope nope")), /not a GLB/);
});

const small = [{ width: 128, height: 128, bytes: 5000 }];

test("budgetCheck passes a cook inside the tri budget, the 256 px cap and the byte budgets", () => {
  const r = budgetCheck({ lod0Triangles: 1984, trisBudget: 2000, laneBytes: 175116, atlases: small });
  assert.equal(r.verdict, "pass");
  assert.deepEqual(r.findings, []);
  assert.equal(r.numbers.maxEdge, 128);
  assert.equal(r.numbers.decodedBytes, 128 * 128 * 4);
});

test("budgetCheck: an atlas over the cap passes, counted at the edge the rig cook shrinks it to", () => {
  const over = budgetCheck({ lod0Triangles: 1, trisBudget: 2000, laneBytes: 1, atlases: [{ width: 512, height: 512, bytes: 1 }] });
  assert.equal(over.verdict, "pass");
  assert.equal(over.numbers.maxEdge, 512);
  assert.equal(over.numbers.cookedEdge, 256);
  assert.equal(over.numbers.decodedBytes, 256 * 256 * 4);
  const wide = budgetCheck({ lod0Triangles: 1, trisBudget: 2000, laneBytes: 1, atlases: [{ width: 1024, height: 512, bytes: 1 }] });
  assert.equal(wide.numbers.decodedBytes, 256 * 128 * 4);
});

test("budgetCheck names the tri, lane and memory budgets when they are broken", () => {
  const r = budgetCheck({
    lod0Triangles: 2500, trisBudget: 2000, laneBytes: PAGE.firstBundleBytes,
    atlases: [{ width: 256, height: 256, bytes: 1 }, { width: 256, height: 256, bytes: 1 }],
  });
  assert.equal(r.verdict, "fail");
  const text = r.findings.join(" | ");
  assert.match(text, /lod0 2500 tris over the 2000 tri budget/);
  assert.match(text, /first-bundle budget 2040000/);
  const mem = budgetCheck({ lod0Triangles: 1, trisBudget: 2000, laneBytes: 1,
    atlases: Array.from({ length: 80 }, () => ({ width: 256, height: 256, bytes: 1 })) });
  assert.match(mem.findings.join(" "), /memory budget 19988480/);
});

test("receiptLine prints the one line with tris, atlas, fill, gate and budget", () => {
  const budget = budgetCheck({ lod0Triangles: 2500, trisBudget: 2000, laneBytes: 1, atlases: [{ width: 512, height: 512, bytes: 1 }] });
  const receipt = buildReceipt({
    name: "probe_recruit", faction: "ashkin", tris: 2000,
    seconds: { total: 312.4 },
    cookJson: { lods: [{ lod: 0, triangles: 1984 }, { lod: 1, triangles: 1083 }] },
    lodBytes: { 0: 175116, 1: 131152 },
    silhouette: { bbox_fill: 0.4892, fill_30m: 0.4952, height_30m_px: 88 },
    rig: null, budget, verdict: "budget_fail", stopped: null,
  });
  const line = receiptLine(receipt);
  assert.match(line, /^RECEIPT probe_recruit ashkin \| tris lod0 1984 lod1 1083 \| atlas 512 px \(page 256\)/);
  assert.match(line, /fill 0\.4892 \(30 m 0\.4952\) \| gate pass/);
  assert.match(line, /budget fail: lod0 2500 tris over the 2000 tri budget \| 312\.4 s$/);
  assert.equal(receipt.lods[1].bytes, 131152);
  assert.match(receipt.next_step, /not wired into the game/);
});
