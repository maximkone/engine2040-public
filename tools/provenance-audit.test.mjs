// Run: node --test tools/provenance-audit.test.mjs
// CT46: the dry-run mirrors the Rust release gate and names every refusal.
// CT52 rehearsal, CT74 cooked maps, CT80 studio HDRI.
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import test from "node:test";
import { fileDigest, readJson, REPO, syncBlocks } from "./content-lib.mjs";
import { ALLOWED_SPDX, auditBlocks, auditBody, auditRecords, diskState, dryRun, refusal, STUDIO_HDRI, unlistedTrees } from "./provenance-audit.mjs";

const RUST = readFileSync(join(REPO, "crates", "assets", "src", "provenance.rs"), "utf8");
const good = () => ({ name: "content/arena/sand.glb", url: "https://example.org/sand", spdx: "CC0-1.0", author: "A. Author", sha: "ab".repeat(32) });
const row = (rec, disk = "match") => ({ rec, disk });

test("the SPDX allow-list and field bounds equal the Rust gate", () => {
  const list = /ALLOWED_SPDX: &\[&str\] = &\[([^\]]*)\]/.exec(RUST)[1];
  assert.deepEqual([...list.matchAll(/"([^"]+)"/g)].map((m) => m[1]), ALLOWED_SPDX);
  for (const [name, value] of [["MAX_NAME_CHARS", 64], ["MAX_FIELD_CHARS", 256], ["SHA_MIN_LEN", 16], ["SHA_MAX_LEN", 64]]) {
    assert.match(RUST, new RegExp(`pub const ${name}: usize = ${value};`));
  }
});

test("a hand record passes and each gap gets the Rust code", () => {
  assert.equal(refusal(good()), null);
  assert.equal(refusal({ ...good(), url: "" }), "missing_url");
  assert.equal(refusal({ ...good(), url: "no scheme" }), "missing_url");
  assert.equal(refusal({ ...good(), author: "  " }), "missing_author");
  assert.equal(refusal({ ...good(), sha: "abc" }), "bad_sha");
  assert.equal(refusal({ ...good(), sha: "AB".repeat(32) }), "bad_sha");
  assert.equal(refusal({ ...good(), spdx: "CC0 1.0" }), "bad_spdx");
  assert.equal(refusal({ ...good(), spdx: "GPL-3.0-only" }), "license_refused");
  assert.equal(refusal({ ...good(), spdx: "cc0-1.0" }), null, "SPDX compares case-insensitively like the gate");
  assert.equal(refusal({ ...good(), name: "Content/Sand.glb" }), "bad_name");
  assert.equal(refusal({ ...good(), name: "a".repeat(65) }), "bad_name");
});

test("the dry-run names every gap, not only the first, plus duplicates and SHA drift", () => {
  const rows = [
    row({ ...good(), name: "a/one.glb", url: "" }),
    row({ ...good(), name: "a/two.glb", spdx: "GPL-3.0-only" }),
    row({ ...good(), name: "a/three.glb" }, "DRIFT"),
    row({ ...good(), name: "a/three.glb" }),
    row({ ...good(), name: "a/ok.glb" }, "off-tree"),
  ];
  assert.deepEqual(dryRun(rows), [
    { asset: "a/one.glb", code: "missing_url" },
    { asset: "a/two.glb", code: "license_refused" },
    { asset: "a/three.glb", code: "sha_drift" },
    { asset: "a/three.glb", code: "duplicate" },
  ]);
});

test("the disk column reads the same with or without the local files", () => {
  const here = { sha256: "aa", bytes: 1 };
  assert.equal(diskState(here, "aa", false), "match");
  assert.equal(diskState(here, "aa", true), "off-tree", "a git-ignored or local-only file never claims match");
  assert.equal(diskState(null, "aa", true), "off-tree");
  assert.equal(diskState(null, "aa", false), "off-tree");
  assert.equal(diskState(here, "bb", true), "DRIFT", "drift is still named where the file is present");
  assert.equal(diskState(here, "bb", false), "DRIFT");
});

test("the real matrix lists every shipped arena and fighter asset once", () => {
  const rows = auditRecords();
  assert.ok(rows.length >= 50, `${rows.length} rows`);
  assert.equal(new Set(rows.map((r) => r.rec.name)).size, rows.length);
  assert.ok(rows.every((r) => r.rec.spdx === "CC0-1.0" && r.rec.url.includes("://")));
  const disk = (name) => rows.find((r) => r.rec.name === name)?.disk;
  assert.equal(disk("content/arena/hdri/arena_day.hdr"), "match");
  assert.equal(disk("content/arena/hdri/arena_night.hdr"), "off-tree", "local-only HDRI: same on a fresh checkout");
  assert.equal(disk("content/fighters/incoming/modular_rpg_characters.7z"), "off-tree", "git-ignored pack: same on a fresh checkout");
  assert.ok(rows.some((r) => r.rec.name === "content/arena/hdri/arena_night.hdr" && r.rec.url === "https://polyhaven.com/a/moonlit_golf"));
  assert.ok(rows.some((r) => r.rec.name === "content/arena/textures/wood_normal_2k.jpg" && r.rec.author.startsWith("Charlotte Baglioni")));
  assert.deepEqual([...new Set(dryRun(rows).map((g) => g.code))], ["bad_name"], "the only real refusals left are the over-long fighter names");
});

test("CT52: a seeded 50-record bundle names each seeded gap and nothing else, the same twice", () => {
  const bundle = auditRecords().slice(0, 50);
  assert.equal(bundle.length, 50);
  assert.deepEqual(dryRun(bundle), [], "the real first 50 are clean before seeding");
  const seed = { 7: { url: "" }, 19: { author: "   " }, 33: { sha: "" } };
  const seeded = bundle.map((r, i) => ({ ...r, rec: { ...r.rec, ...seed[i] } }));
  const want = [
    { asset: "content/arena/src/debris.gltf", code: "missing_url" },
    { asset: "content/arena/textures/sand_normal_2k.jpg", code: "missing_author" },
    { asset: "content/arena/textures/cloth_orm.png", code: "bad_sha" },
  ];
  assert.deepEqual(dryRun(seeded), want);
  assert.deepEqual(dryRun(seeded), want);
  assert.equal(auditBody(), auditBody());
});

test("CT74: every cooked KTX2 map carries URL, CC0, author and SHA of its ambientCG set", () => {
  const cooked = auditRecords().filter((r) => r.rec.name.startsWith("content/arena/cooked/"));
  const receipt = readJson(join(REPO, "content", "arena", "cooked", "cook.json")).maps.map((m) => `content/arena/cooked/${m.file}`);
  assert.deepEqual(cooked.map((r) => r.rec.name).sort(), receipt.sort(), "one record per cooked map in the receipt");
  assert.equal(cooked.length, 15);
  const sets = ["Bricks097", "Ground093C", "Metal046B", "PavingStones150", "Planks039"].map((id) => `https://ambientcg.com/a/${id}`);
  assert.deepEqual([...new Set(cooked.map((r) => r.rec.url))].sort(), sets);
  for (const r of cooked) {
    assert.deepEqual([r.rec.author, r.rec.spdx, r.disk, refusal(r.rec)], ["ambientCG", "CC0-1.0", "match", null], r.rec.name);
    assert.equal(r.rec.sha, fileDigest(join(REPO, r.rec.name)).sha256);
  }
  assert.ok(unlistedTrees().some((t) => t.tree === "content/arena/sources/" && t.owner === "CT74"));
});

test("CT80: the studio HDRI is vendored with a provenance row and both copies hold the sidecar's bytes", () => {
  const side = readJson(join(REPO, "content", "incoming", "studio_small_09_1k.hdr.provenance.json"));
  const rows = auditRecords().filter((r) => [STUDIO_HDRI, ...side.copies].includes(r.rec.name));
  assert.equal(rows.length, 2);
  for (const r of rows) {
    assert.deepEqual([r.rec.url, r.rec.author, r.rec.spdx, r.disk, refusal(r.rec)], ["https://polyhaven.com/a/studio_small_09", "Sergej Majboroda", "CC0-1.0", "match", null], r.rec.name);
  }
  const bytes = readFileSync(join(REPO, STUDIO_HDRI));
  assert.equal(bytes.length, side.bytes);
  assert.equal(createHash("md5").update(bytes).digest("hex"), side.md5, "the md5 Poly Haven publishes for the 1k file");
  assert.equal(fileDigest(join(REPO, side.copies[0])).sha256, side.sha256);
});

test("unlisted trees name an owner, and the generated block is current", () => {
  const trees = unlistedTrees();
  assert.ok(trees.some((t) => t.tree === "content/fighters/incoming/modular/"));
  assert.ok(trees.some((t) => t.tree === "content/incoming/" && t.owner === "CT80"));
  assert.ok(trees.every((t) => t.owner && t.why));
  assert.match(auditBody(), /Release-refusal dry-run: \d+ refusals\./);
  assert.deepEqual(syncBlocks(auditBlocks()), []);
});
