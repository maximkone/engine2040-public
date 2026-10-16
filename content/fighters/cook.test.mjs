// Run: node content/fighters/cook.test.mjs (also: node --test content/fighters/cook.test.mjs)
// Cook gate: every fighter carries material + provenance.
// Material = parts[] with texture + tile, atlas PNGs on disk, body GLB on disk.
// Provenance = sources[] + incoming_files[] with URL + SPDX + author + SHA.
// Missing manifest skips cleanly; missing gitignored bundles skip, never fail.
import assert from "node:assert/strict";
import { existsSync, readFileSync, statSync } from "node:fs";
import { createHash } from "node:crypto";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";

const ROOT = dirname(fileURLToPath(import.meta.url));
const MANIFEST_PATH = join(ROOT, "manifest.json");

const EXPECTED_VARIANTS = ["hero", "opp_male_light", "opp_female_medium", "opp_female_light"];
const EXPECTED_JOINTS = 45;
const EXPECTED_CLIPS = 14;
const EXPECTED_PRIMS = 4;
const ALLOWED_SPDX = [
  "MIT",
  "Apache-2.0",
  "BSD-2-Clause",
  "BSD-3-Clause",
  "Zlib",
  "ISC",
  "CC0-1.0",
  "Unlicense",
  "0BSD",
  "MIT-0",
  "Unicode-3.0",
  "Unicode-DFS-2016",
];

const isHex64 = (s) => typeof s === "string" && /^[0-9a-f]{64}$/.test(s);
const isHex16 = (s) => typeof s === "string" && /^[0-9a-f]{16}$/.test(s);
const sha256File = (p) => createHash("sha256").update(readFileSync(p)).digest("hex");

function loadManifest() {
  try {
    if (!existsSync(MANIFEST_PATH)) return null;
    return JSON.parse(readFileSync(MANIFEST_PATH, "utf8"));
  } catch {
    return null;
  }
}

const manifest = loadManifest();
const noManifest = manifest === null ? "manifest.json missing, skipping" : false;

test("manifest lists the 4 fighters", { skip: noManifest }, () => {
  assert.equal(Array.isArray(manifest.variants), true, "variants is an array");
  assert.equal(manifest.variants.length, 4, "4 variants");
  assert.deepEqual(
    manifest.variants.map((v) => v.name),
    EXPECTED_VARIANTS,
    "hero plus 3 opponents in order",
  );
});

test("each fighter carries material", { skip: noManifest }, () => {
  assert.match(String(manifest.materials), /4 quadrant materials/, "top-level materials note names 4");
  for (const v of manifest.variants) {
    assert.equal(v.joints, EXPECTED_JOINTS, `${v.name} joints`);
    assert.equal(v.clips, EXPECTED_CLIPS, `${v.name} clips`);
    assert.equal(v.prims, EXPECTED_PRIMS, `${v.name} prims`);
    assert.ok(v.tris_lod0 > 0, `${v.name} has triangles`);
    assert.ok(Array.isArray(v.parts) && v.parts.length >= 8, `${v.name} has 8+ parts`);
    for (const p of v.parts) {
      assert.ok(typeof p.role === "string" && p.role.length > 0, `${v.name} part role`);
      assert.ok(typeof p.object === "string" && p.object.length > 0, `${v.name} part object`);
      assert.ok(typeof p.texture === "string" && p.texture.endsWith(".png"), `${v.name} part texture ${p.texture}`);
      assert.ok(Array.isArray(p.tile) && p.tile.length === 3, `${v.name} part tile`);
      const [tx, ty, tw] = p.tile;
      assert.ok(Number.isInteger(tx) && tx >= 0, `${v.name} tile x`);
      assert.ok(Number.isInteger(ty) && ty >= 0, `${v.name} tile y`);
      assert.ok(tw === 512 || tw === 1024, `${v.name} tile size 512 or 1024`);
    }
    // Atlas PNGs are the cooked material payloads: one composite + 4 quadrants.
    for (const f of [`${v.name}_atlas.png`, `${v.name}_atlas_q0.png`, `${v.name}_atlas_q1.png`, `${v.name}_atlas_q2.png`, `${v.name}_atlas_q3.png`]) {
      const p = join(ROOT, "atlas", f);
      assert.equal(existsSync(p), true, `atlas present: ${f}`);
      assert.ok(statSync(p).size > 0, `atlas non-empty: ${f}`);
    }
    // Body GLB is the cooked mesh payload: present, non-empty, matches the pin.
    const bodyPath = join(ROOT, "glb", `${v.name}_body.glb`);
    assert.equal(existsSync(bodyPath), true, `body present: ${v.name}_body.glb`);
    assert.ok(statSync(bodyPath).size > 0, `body non-empty: ${v.name}`);
    assert.equal(sha256File(bodyPath), v.body_glb_sha256, `${v.name} body bytes match the pin`);
  }
});

test("each fighter carries provenance", { skip: noManifest }, () => {
  assert.ok(Array.isArray(manifest.sources) && manifest.sources.length >= 2, "2+ sources");
  for (const s of manifest.sources) {
    assert.ok(typeof s.url === "string" && s.url.includes("://"), `source URL: ${s.file}`);
    assert.ok(!/\s/.test(s.url), `source URL has no spaces: ${s.file}`);
    assert.ok(ALLOWED_SPDX.includes(s.spdx), `source SPDX allowed: ${s.spdx}`);
    assert.ok(typeof s.author === "string" && s.author.trim().length > 0, `source author: ${s.file}`);
    assert.ok(isHex64(s.sha256), `source SHA-256: ${s.file}`);
    assert.ok(s.bytes > 0, `source bytes: ${s.file}`);
  }
  assert.ok(Array.isArray(manifest.incoming_files) && manifest.incoming_files.length >= 2, "incoming_files rows");
  for (const row of manifest.incoming_files) {
    assert.ok(typeof row.path === "string" && row.path.length > 0, "incoming path");
    assert.ok(ALLOWED_SPDX.includes(row.spdx), `incoming SPDX allowed: ${row.path}`);
    assert.ok(typeof row.author === "string" && row.author.trim().length > 0, `incoming author: ${row.path}`);
    if ("sha256" in row) assert.ok(isHex64(row.sha256), `incoming SHA-256: ${row.path}`);
    if ("bytes" in row) assert.ok(row.bytes > 0, `incoming bytes: ${row.path}`);
  }
  for (const v of manifest.variants) {
    assert.ok(isHex64(v.bundle_sha256), `${v.name} bundle SHA-256`);
    assert.ok(isHex64(v.body_glb_sha256), `${v.name} body SHA-256`);
  }
  assert.ok(Array.isArray(manifest.refused) && manifest.refused.length > 0, "refused rows stay refusal-only");
});

test("clips and sidecars are complete", { skip: noManifest }, () => {
  assert.equal(manifest.clips.length, 56, "14 clips x 4 variants");
  const shorts = new Set(manifest.clips.map((c) => c.short));
  assert.equal(shorts.size, 14, "14 canonical shorts");
  for (const c of manifest.clips) {
    assert.ok(EXPECTED_VARIANTS.includes(c.variant), `clip variant: ${c.short}`);
    assert.ok(isHex16(c.digest), `clip digest: ${c.short}`);
  }
  for (const short of shorts) {
    const p = join(ROOT, "sidecars", `${short}.json`);
    assert.equal(existsSync(p), true, `sidecar present: ${short}.json`);
    const side = JSON.parse(readFileSync(p, "utf8"));
    assert.equal(typeof side.loop, "boolean", `${short} loop`);
    assert.equal(typeof side.root_motion, "boolean", `${short} root_motion`);
  }
});

test("bundles match the size pin when cooked", { skip: noManifest }, (t) => {
  // Bundles are gitignored cook outputs: absent on a fresh checkout skips, never fails.
  let seen = 0;
  for (const v of manifest.variants) {
    const p = join(ROOT, "bundle", `${v.name}.e2040`);
    if (!existsSync(p)) continue;
    seen += 1;
    assert.ok(statSync(p).size > 0, `bundle non-empty: ${v.name}`);
    if (typeof v.bundle_bytes === "number") {
      assert.equal(statSync(p).size, v.bundle_bytes, `${v.name} bundle bytes match the pin`);
    }
  }
  if (seen === 0) t.skip("no bundles on disk, nothing to check");
});
