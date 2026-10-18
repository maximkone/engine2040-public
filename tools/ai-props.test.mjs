// Run: node --test tools/ai-props.test.mjs
// CT40: the three AI props under content/props/ai/ hold the prop budget, carry
// provenance that names generator, model and prompt digest, and pin their own bytes.
// The engine side (intake, cook x2, licence refusal) is crates/assets/tests/it/ai_props.rs.
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import test from "node:test";
import { REPO } from "./content-lib.mjs";
import { ALLOWED_SPDX } from "./provenance-audit.mjs";

const DIR = join(REPO, "content", "props", "ai");
const PROPS = ["ai_amphora", "ai_barrel", "ai_crate"];
/** Prop LOD0 triangle bar (mesh_cook.rs PROP_MAX_TRIS) and the map size bar of the row. */
const PROP_MAX_TRIS = 5000;
const MAP_MAX_PX = 2048;
const sha256 = (bytes) => createHash("sha256").update(bytes).digest("hex");
const allowed = (spdx) => ALLOWED_SPDX.some((s) => s.toLowerCase() === spdx.toLowerCase());

/** The JSON chunk of a GLB: 12-byte header, then the first chunk is JSON. */
function glbJson(bytes) {
  assert.equal(bytes.toString("latin1", 0, 4), "glTF");
  assert.equal(bytes.toString("latin1", 16, 20), "JSON");
  return JSON.parse(bytes.toString("utf8", 20, 20 + bytes.readUInt32LE(12)));
}

/** Triangles of every TRIANGLES primitive (mode 4 is the default). */
function triangles(json) {
  return json.meshes.flatMap((m) => m.primitives).reduce((sum, p) => {
    assert.ok(p.mode === undefined || p.mode === 4, "triangle lists only");
    return sum + json.accessors[p.indices].count / 3;
  }, 0);
}

const load = (name) => {
  const glb = readFileSync(join(DIR, `${name}.glb`));
  return { glb, json: glbJson(glb), prov: JSON.parse(readFileSync(join(DIR, `${name}.provenance.json`), "utf8")) };
};

test("each AI prop is a triangle-list GLB under 5k tris with no map to size", () => {
  for (const name of PROPS) {
    const { json, prov } = load(name);
    const tris = triangles(json);
    assert.ok(tris > 0 && tris <= PROP_MAX_TRIS, `${name}: ${tris} tris`);
    assert.equal(tris, prov.game_tris, `${name}: provenance counts the real triangles`);
    assert.equal(json.images?.length ?? 0, 0, `${name}: untextured, so every map is under ${MAP_MAX_PX}`);
    assert.equal(prov.maps, 0);
  }
});

test("provenance names generator, model and prompt digest, and pins the file bytes", () => {
  const digests = new Set();
  for (const name of PROPS) {
    const { glb, prov } = load(name);
    assert.ok(prov.generator && allowed(prov.generator_license), `${name}: generator and its licence`);
    assert.ok(prov.model.id && allowed(prov.model.license), `${name}: model and its licence`);
    for (const c of prov.components) assert.ok(c.id && allowed(c.license), `${name}: component ${c.id}`);
    assert.equal(prov.source_digest, sha256(prov.prompt), `${name}: the prompt digest is the digest of the prompt`);
    assert.equal(prov.artifact_sha256, sha256(glb), `${name}: the sidecar pins the GLB`);
    assert.equal(prov.artifact_bytes, glb.length);
    assert.ok(prov.verifiers.length === 3 && prov.verifiers.every((v) => v.passed), `${name}: licence, budget and cook x2 verifiers`);
    digests.add(prov.artifact_sha256);
  }
  assert.equal(digests.size, PROPS.length, "no two props share bytes");
});
