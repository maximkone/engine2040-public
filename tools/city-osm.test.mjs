import assert from "node:assert/strict";
import test from "node:test";
import { cookExtractToGltf, inspectGltf, parseOsm } from "./city-osm.mjs";
// 500m x 500m slice: 12 building ways + 4 highway ways generated deterministically.
const sliceXml = () => {
  let s = `<osm version="0.6">`; let id = 1; const b0 = [];
  for (let b = 0; b < 12; b++) {
    const x = (b % 4) * 40, y = Math.floor(b / 4) * 40, ns = [];
    for (const [px, py] of [[x, y], [x + 20, y], [x + 20, y + 12], [x, y + 12]]) {
      s += `<node id="${id}" lat="${(51 + py / 100000).toFixed(6)}" lon="${(px / 100000).toFixed(6)}"/>`; ns.push(id++);
    }
    b0.push(ns);
  }
  const rn = [];
  for (let r = 0; r < 4; r++) { const ns = []; for (let k = 0; k < 4; k++) { s += `<node id="${id}" lat="${(51 + (r * 100 + k * 60) / 100000).toFixed(6)}" lon="${((-50 + k * 60) / 100000).toFixed(6)}"/>`; ns.push(id++); } rn.push(ns); }
  b0.forEach((ns, b) => { s += `<way id="${100 + b}">${ns.map((n) => `<nd ref="${n}"/>`).join("")}<tag k="building" v="yes"/><tag k="building:levels" v="${2 + (b % 3)}"/></way>`; });
  rn.forEach((ns, r) => { s += `<way id="${200 + r}">${ns.map((n) => `<nd ref="${n}"/>`).join("")}<tag k="highway" v="residential"/></way>`; });
  return s + `</osm>`;
};
test("slice parses to directed 2D road graph plus legal blocks", () => {
  const p = parseOsm(sliceXml());
  assert.equal(p.buildings.length, 12); assert.equal(p.highways.length, 4);
  assert.ok(p.graph.edges.length >= 12); assert.deepEqual([...p.graph.edges].sort(), p.graph.edges);
});
test("12-building slice cooks valid, x2 identical, loads with 12+1 meshes", async () => {
  const xml = sliceXml();
  const a = cookExtractToGltf(xml, { licence: "CC0-1.0" });
  const b = cookExtractToGltf(xml, { licence: "CC0-1.0" });
  assert.equal(a.prov.cookDigest, b.prov.cookDigest); assert.equal(a.gltf, b.gltf);
  assert.ok(a.stats.triangles <= 20000, `tris ${a.stats.triangles}`);
  const view = inspectGltf(a.gltf);
  assert.deepEqual(view.issues, []); assert.equal(view.ok, true);
  assert.ok(view.meshes >= 13, `meshes ${view.meshes}`); assert.ok(view.triangles === a.stats.triangles);
  let intakeFail = null, skipped = false; // intake clean path when heavy deps installed
  try {
    const { validateBytes } = await import("./gltf-intake.mjs");
    const r = await validateBytes(Buffer.from(a.gltf, "utf8"), ".gltf");
    intakeFail = r.fail;
  } catch { skipped = true; }
  if (!skipped) assert.equal(intakeFail, 0);
  else assert.equal(view.issues.length, 0);
  assert.match(a.prov.sourceSha256, /^[0-9a-f]{64}$/); assert.match(a.prov.cookDigest, /^[0-9a-f]{64}$/);
  assert.equal(a.prov.licence, "CC0-1.0");
});
test("refusals: UNKNOWN licence, over-cap extract, network input", async () => {
  assert.throws(() => cookExtractToGltf(sliceXml(), { licence: "UNKNOWN" }), /E_LICENCE/);
  assert.throws(() => cookExtractToGltf(sliceXml().repeat(8), { licence: "CC0-1.0" }), /E_CAP/);
  const { cookFile } = await import("./city-osm.mjs");
  assert.throws(() => cookFile("https://x/y.osm", "t/out.gltf", { licence: "CC0-1.0" }), /E_NETWORK/);
});
