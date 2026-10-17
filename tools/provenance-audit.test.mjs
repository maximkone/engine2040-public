// Run: node --test tools/provenance-audit.test.mjs
// CT46: the dry-run mirrors the Rust release gate and names every refusal.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import test from "node:test";
import { REPO, syncBlocks } from "./content-lib.mjs";
import { ALLOWED_SPDX, auditBlocks, auditBody, auditRecords, dryRun, refusal, unlistedTrees } from "./provenance-audit.mjs";

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

test("the real matrix lists every shipped arena and fighter asset once", () => {
  const rows = auditRecords();
  assert.ok(rows.length >= 50, `${rows.length} rows`);
  assert.equal(new Set(rows.map((r) => r.rec.name)).size, rows.length);
  assert.ok(rows.every((r) => r.rec.spdx === "CC0-1.0" && r.rec.url.includes("://")));
  assert.ok(rows.some((r) => r.rec.name === "content/arena/hdri/arena_night.hdr" && r.rec.url === "https://polyhaven.com/a/moonlit_golf" && r.disk === "match"));
  assert.ok(rows.some((r) => r.rec.name === "content/arena/textures/wood_normal_2k.jpg" && r.rec.author.startsWith("Charlotte Baglioni")));
  assert.ok(rows.some((r) => r.rec.name === "content/fighters/incoming/modular_rpg_characters.7z" && r.disk === "match"));
});

test("unlisted trees name an owner, and the generated block is current", () => {
  const trees = unlistedTrees();
  assert.ok(trees.some((t) => t.tree === "content/fighters/incoming/modular/"));
  assert.ok(trees.every((t) => t.owner && t.why));
  assert.match(auditBody(), /Release-refusal dry-run: \d+ refusals\./);
  assert.deepEqual(syncBlocks(auditBlocks()), []);
});
