// Run: node content/arena/lod.test.mjs (also: node --test content/arena/lod.test.mjs)
// Arena cook records carry LOD + hash. report.json holds one cook record per
// asset (lod_tris + 16-hex digest). manifest.json holds one provenance row per
// file (64-hex sha256 + bytes). Missing files skip, never throw.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const ARENA_DIR = dirname(fileURLToPath(import.meta.url));
const LEVELS = 4;

export function isHex16(s) {
  return typeof s === "string" && /^[0-9a-f]{16}$/.test(s);
}

export function isHex64(s) {
  return typeof s === "string" && /^[0-9a-f]{64}$/.test(s);
}

// Read JSON or return null when the path is missing. Bad JSON still throws.
export function loadJsonIfExists(path) {
  try {
    return JSON.parse(readFileSync(path, "utf8"));
  } catch (e) {
    if (e && (e.code === "ENOENT" || e.code === "ENOTDIR")) return null;
    throw e;
  }
}

// List problems in one report.json cook record. Empty means the record is good.
export function findCookProblems(rec) {
  const bad = [];
  if (!rec || typeof rec.asset !== "string" || rec.asset === "") bad.push("asset name");
  if (!Number.isInteger(rec.meshes) || rec.meshes < 1) bad.push("meshes >= 1");
  if (!Number.isInteger(rec.lod0_tris) || rec.lod0_tris < 1) bad.push("lod0_tris >= 1");
  if (!Array.isArray(rec.lod_tris)) bad.push("lod_tris array");
  else {
    if (rec.lod_tris.length !== rec.meshes * LEVELS) bad.push("lod_tris len meshes*4");
    for (const n of rec.lod_tris) {
      if (!Number.isInteger(n) || n < 1) { bad.push("lod_tris all >= 1"); break; }
    }
    // Each mesh group of 4 must shrink, never grow or go empty.
    for (let m = 0; m < rec.meshes; m++) {
      const g = rec.lod_tris.slice(m * LEVELS, m * LEVELS + LEVELS);
      for (let i = 1; i < g.length; i++) {
        if (g[i] > g[i - 1]) { bad.push(`mesh ${m} lod grows`); break; }
      }
    }
    // lod0_tris is the sum of the LOD0 slot of each mesh.
    let sum = 0;
    for (let m = 0; m < rec.meshes; m++) sum += rec.lod_tris[m * LEVELS] ?? 0;
    if (rec.lod_tris.length === rec.meshes * LEVELS && sum !== rec.lod0_tris) {
      bad.push("lod0_tris sum mismatch");
    }
  }
  if (!isHex16(rec.digest)) bad.push("digest 16-hex");
  if (rec.hulls_contain !== true) bad.push("hulls_contain true");
  return bad;
}

// List problems in one manifest.json provenance row. Empty means good.
export function findProvenanceProblems(name, entry) {
  const bad = [];
  if (typeof name !== "string" || name === "") bad.push("name");
  if (!entry || !isHex64(entry.sha256)) bad.push("sha256 64-hex");
  if (!entry || !Number.isInteger(entry.bytes) || entry.bytes < 1) bad.push("bytes >= 1");
  return bad;
}

test("cook records carry LOD tris plus a digest", (t) => {
  const report = loadJsonIfExists(join(ARENA_DIR, "report.json"));
  if (!report) { t.skip("no report.json on disk"); return; }
  assert.ok(Array.isArray(report.assets) && report.assets.length > 0, "assets non-empty");
  for (const rec of report.assets) {
    assert.deepEqual(findCookProblems(rec), [], `bad cook record ${rec.asset}`);
  }
});

test("cook LOD0 total matches the asset rows", (t) => {
  const report = loadJsonIfExists(join(ARENA_DIR, "report.json"));
  if (!report) { t.skip("no report.json on disk"); return; }
  const sum = report.assets.reduce((n, r) => n + r.lod0_tris, 0);
  assert.equal(sum, report.total_lod0_tris, "total_lod0_tris is the asset sum");
});

test("bundle entries carry every mesh LOD the cook records name", (t) => {
  const report = loadJsonIfExists(join(ARENA_DIR, "report.json"));
  if (!report) { t.skip("no report.json on disk"); return; }
  const names = new Set((report.bundle_entries ?? []).map((e) => e.name));
  for (const rec of report.assets) {
    for (let m = 0; m < rec.meshes; m++) {
      for (let l = 0; l < LEVELS; l++) {
        assert.ok(names.has(`arena/${rec.asset}/m${String(m).padStart(2, "0")}_lod${l}`), `missing arena/${rec.asset}/m${m}_lod${l}`);
      }
    }
  }
});

test("provenance rows carry sha256 plus bytes", (t) => {
  const manifest = loadJsonIfExists(join(ARENA_DIR, "manifest.json"));
  if (!manifest) { t.skip("no manifest.json on disk"); return; }
  const files = manifest.files ?? {};
  assert.ok(Object.keys(files).length > 0, "files non-empty");
  for (const [name, entry] of Object.entries(files)) {
    assert.deepEqual(findProvenanceProblems(name, entry), [], `bad provenance row ${name}`);
  }
});

test("provenance covers every cooked gltf the report names", (t) => {
  const report = loadJsonIfExists(join(ARENA_DIR, "report.json"));
  const manifest = loadJsonIfExists(join(ARENA_DIR, "manifest.json"));
  if (!report || !manifest) { t.skip("no report.json or manifest.json on disk"); return; }
  for (const rec of report.assets) {
    const row = manifest.files[`src/${rec.asset}.gltf`];
    assert.ok(row, `manifest misses src/${rec.asset}.gltf`);
    assert.deepEqual(findProvenanceProblems(`src/${rec.asset}.gltf`, row), [], "row carries sha + bytes");
  }
});

test("validators refuse empty LOD and bad hashes", () => {
  const good = { asset: "crate", meshes: 1, lod0_tris: 8, lod_tris: [8, 4, 2, 1], digest: "0123456789abcdef", hulls_contain: true };
  assert.deepEqual(findCookProblems(good), []);
  const empty = { ...good, lod_tris: [8, 4, 0, 1] };
  assert.ok(findCookProblems(empty).length > 0, "empty LOD refuses");
  const grows = { ...good, lod_tris: [8, 9, 2, 1] };
  assert.ok(findCookProblems(grows).length > 0, "growing LOD refuses");
  const badDigest = { ...good, digest: "XYZ" };
  assert.ok(findCookProblems(badDigest).length > 0, "bad digest refuses");
  assert.deepEqual(findProvenanceProblems("src/crate.gltf", { sha256: "0".repeat(64), bytes: 7 }), []);
  assert.ok(findProvenanceProblems("src/crate.gltf", { sha256: "short", bytes: 7 }).length > 0, "short sha refuses");
  assert.ok(findProvenanceProblems("src/crate.gltf", { sha256: "0".repeat(64), bytes: 0 }).length > 0, "zero bytes refuses");
  assert.equal(loadJsonIfExists(join(ARENA_DIR, "no-such-file-xyz.json")), null, "missing path loads null");
});
