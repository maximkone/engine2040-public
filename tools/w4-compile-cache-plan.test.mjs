// Idea-only port of mozilla/sccache (Apache-2.0, allow-listed in deny.toml).
// Warm-cache compile plan: content-hash cache keys (compiler + lang + sorted
// flags + input digest), local-then-remote tiers, LRU eviction under a size
// cap, and a warm build order so shared deps compile once. No donor code is
// copied: the key function below is a fresh sha256 recipe, the entries are
// fixture data. Never runs cargo.
//   node --test tools/w4-compile-cache-plan.test.mjs
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";

const PLAN = join(dirname(fileURLToPath(import.meta.url)), "w4-compile-cache-plan.json");

function loadPlan() {
  return JSON.parse(readFileSync(PLAN, "utf8"));
}

// Canonical cache key: sha256 over newline-joined fields, first 16 hex chars
// (64-bit fingerprint). Sorted flags keep flag order out of the key.
function keyOf(entry) {
  const flags = [...entry.flags].sort().join(" ");
  const canonical = [entry.compiler, entry.lang, flags, entry.input_sha256].join("\n") + "\n";
  return createHash("sha256").update(canonical, "utf8").digest("hex").slice(0, 16);
}

test("plan carries its credit, version, tiers and LRU discipline", () => {
  const doc = loadPlan();
  assert.match(doc._credit, /mozilla\/sccache/, "credit names the donor");
  assert.match(doc._credit, /Apache-2\.0/, "credit names the SPDX");
  assert.equal(doc.version, 1, "plan version is 1");
  assert.deepEqual(doc.tiers, ["local-disk", "remote"], "local first, remote second");
  assert.equal(doc.eviction, "lru", "LRU eviction");
  assert.ok(Number.isInteger(doc.size_cap_mb) && doc.size_cap_mb > 0, "positive size cap");
});

test("every entry key matches the canonical key function", () => {
  const doc = loadPlan();
  assert.equal(doc.entries.length, 6, "6 fixture entries");
  const keys = new Set();
  for (const e of doc.entries) {
    assert.match(e.key, /^[0-9a-f]{16}$/, `${e.crate} key is 16 hex chars`);
    assert.equal(e.key, keyOf(e), `${e.crate} key reproduces from its fields`);
    assert.ok(!keys.has(e.key), `duplicate key ${e.key}`);
    keys.add(e.key);
  }
});

test("key function is deterministic and sensitive to every field", () => {
  const doc = loadPlan();
  const [a] = doc.entries;
  assert.equal(keyOf(a), keyOf(structuredClone(a)), "same inputs give the same key");
  const reflag = structuredClone(a);
  reflag.flags = [...reflag.flags, "-Cdebug-assertions=on"];
  assert.notEqual(keyOf(reflag), keyOf(a), "a new flag changes the key");
  const recomp = structuredClone(a);
  recomp.compiler = "rustc 9.9.9";
  assert.notEqual(keyOf(recomp), keyOf(a), "a new compiler changes the key");
});

test("warm order builds shared deps once and total bytes fit the cap", () => {
  const doc = loadPlan();
  const crates = doc.entries.map((e) => e.crate);
  assert.deepEqual([...doc.warm_order].sort(), [...crates].sort(), "warm order covers every entry once");
  assert.equal(new Set(doc.warm_order).size, crates.length, "no crate warms twice");
  assert.equal(doc.warm_order[0], "engine-core", "shared dep warms first");
  const bytes = doc.entries.reduce((n, e) => n + e.bytes, 0);
  assert.ok(bytes <= doc.size_cap_mb * 1024 * 1024, `bytes ${bytes} fit the cap`);
});

test("LRU victim is the least recently used entry", () => {
  const doc = loadPlan();
  const victim = doc.entries.reduce((old, e) => (e.last_used < old.last_used ? e : old));
  assert.equal(doc.lru_victim, victim.crate, "plan names the true LRU victim");
});
