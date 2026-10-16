// Run: node --test tools/knobs-check.test.mjs
// Arena knobs reader coverage: fixtures/games/arena_knobs.json lists 6 knobs
// and this test proves each knob has a reader in crates/game or tools.
// Reader = one file under those roots whose text contains the bare knob
// name (the task rg definition), excluding the fixture itself and this
// test file (both name every knob). Knobs with zero hits must be pinned in
// EXPECT_NOREADER with their name; a stale pin (now has a reader) fails so
// the gap list cannot go stale. Bare agents overcounts (English word), so
// a future tighten to quoted-key reads is noted where it is used.
import assert from "node:assert/strict";
import { readdirSync, readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
const HERE = dirname(fileURLToPath(import.meta.url));
const ROOT = dirname(HERE);
const FIXTURE = join(ROOT, "fixtures", "games", "arena_knobs.json");
const SELF = join(HERE, "knobs-check.test.mjs");
const SCAN_ROOTS = [join(ROOT, "crates", "game"), join(HERE)];
const SCAN_EXTS = new Set([".rs", ".mjs", ".cjs", ".json", ".toml"]);
const SKIP_DIRS = new Set(["vendor", "archive", "node_modules", "target", ".git"]);
// The 6 knobs in fixtures/games/arena_knobs.json (rg arena_knobs|knobs).
const EXPECTED_KNOBS = ["agents", "enemies", "half_extent_m", "kill_target", "camera_height_m", "camera_lead_m"];
// Verified 2026-10-09 by rg each knob in crates/game + tools (bare name,
// count >= 1): all 6 hit; zero gaps below.
const EXPECT_NOREADER = [];
function listFiles(root) {
  const out = [];
  const stack = [root];
  while (stack.length > 0) {
    const dir = stack.pop();
    let entries;
    try {
      entries = readdirSync(dir, { withFileTypes: true });
    } catch {
      continue;
    }
    for (const e of entries) {
      const full = join(dir, e.name);
      if (e.isDirectory()) {
        if (!SKIP_DIRS.has(e.name)) stack.push(full);
      } else if (e.isFile()) {
        out.push(full);
      }
    }
  }
  return out;
}
function countReaders(knob) {
  const fixtureAbs = resolve(FIXTURE);
  const selfAbs = resolve(SELF);
  const hits = [];
  for (const root of SCAN_ROOTS) {
    for (const file of listFiles(root)) {
      const abs = resolve(file);
      if (abs === fixtureAbs || abs === selfAbs) continue;
      const lower = file.toLowerCase();
      const dot = lower.lastIndexOf(".");
      const ext = dot >= 0 ? lower.slice(dot) : "";
      if (!SCAN_EXTS.has(ext)) continue;
      let text;
      try {
        text = readFileSync(file, "utf8");
      } catch {
        continue;
      }
      if (text.includes(knob)) {
        hits.push(file);
      }
      if (hits.length >= 10) break;
    }
    if (hits.length >= 10) break;
  }
  return { count: hits.length, files: hits };
}
function loadKnobs() {
  const raw = readFileSync(FIXTURE, "utf8");
  const data = JSON.parse(raw);
  assert.ok(Array.isArray(data.knobs), "fixture knobs is an array");
  return data.knobs.map((k) => k.knob);
}
test("arena knobs fixture lists the 6 known knobs", () => {
  const names = loadKnobs();
  assert.equal(names.length, 6, "fixture holds 6 knobs, got " + names.length);
  assert.deepEqual([...names].sort(), [...EXPECTED_KNOBS].sort(), "knob names match expected 6");
});
test("every knob has a reader or is a pinned EXPECT-NOREADER gap", () => {
  const names = loadKnobs();
  const withReaders = [];
  const gaps = [];
  for (const knob of names) {
    const found = countReaders(knob);
    if (found.count >= 1) {
      withReaders.push(knob);
    } else {
      gaps.push(knob);
      assert.ok(EXPECT_NOREADER.includes(knob), "knob " + knob + " has no reader but is not pinned in EXPECT-NOREADER");
    }
    assert.ok(found.count >= 1 || EXPECT_NOREADER.includes(knob), "knob " + knob + ": expected >=1 reader, got 0");
  }
  for (const pinned of EXPECT_NOREADER) {
    assert.ok(names.includes(pinned), "stale pin " + pinned + " is not one of the 6 fixture knobs");
    const found = countReaders(pinned);
    assert.equal(found.count, 0, "stale pin " + pinned + " now has readers: " + found.files.slice(0, 3).join(", "));
  }
  assert.deepEqual([...gaps].sort(), [...EXPECT_NOREADER].sort(), "gap list matches true zero-reader knobs");
  const quoted = withReaders.map((k) => JSON.stringify(k)).join(", ");
  console.log("knobs with readers " + withReaders.length + "/6: " + (quoted || "(none)"));
  assert.ok(withReaders.includes("half_extent_m"), "half_extent_m keeps its reader (rg count >= 1)");
});
