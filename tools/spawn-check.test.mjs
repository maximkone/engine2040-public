// Run: node --test tools/spawn-check.test.mjs
// Audit: unchecked child_process spawns / ignored exit codes in wave-2/3 NEW tools files.
// Every spawn|execFile|execSync call site is classified checked/unchecked with file:line.
// Current pin: 1 site checked, unchecked 0.
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
const HERE = fileURLToPath(new URL(".", import.meta.url));
const ROOT = resolve(HERE, "..");
const NEW_FILES = [
  "tools/board-slice-cases.test.mjs",
  "tools/cast-safe.mjs",
  "tools/cast-safe.test.mjs",
  "tools/compare-split.test.mjs",
  "tools/duration-fmt.mjs",
  "tools/duration-fmt.test.mjs",
  "tools/flaky-quarantine.mjs",
  "tools/frame-triage.mjs",
  "tools/frame-triage.test.mjs",
  "tools/ibl-presets.test.mjs",
  "tools/knobs-check.test.mjs",
  "tools/q1-check.mjs",
  "tools/q1-check.test.mjs",
  "tools/sky-preset.test.mjs",
  "tools/sky-presets.test.mjs",
  "tools/sort-once.mjs",
  "tools/sort-once.test.mjs",
];
const EXPECTED_CHECKED = [
  "tools/board-slice-cases.test.mjs:15",
];
function collectSites() {
  const sites = [];
  for (const rel of NEW_FILES) {
    const text = readFileSync(join(ROOT, rel), "utf8");
    const lines = text.split("\n");
    lines.forEach((line, i) => {
      if (/^\s*import\b/.test(line)) return;
      if (!/\b(spawnSync|spawn|execFileSync|execFile|execSync)\s*\(/.test(line)) return;
      const at = `${rel}:${i + 1}`;
      const window = lines.slice(i, i + 12).join("\n");
      const checked = /\.(status|error|code)\b/.test(window) || /\bcatch\b/.test(window);
      sites.push({ at, checked, line: line.trim() });
    });
  }
  return sites;
}
test("every spawn/exec call site in NEW wave files checks exit code (unchecked 0)", () => {
  const sites = collectSites();
  const unchecked = sites.filter((s) => !s.checked).map((s) => s.at);
  for (const s of sites) console.log(`  ${s.checked ? "checked" : "UNCHECKED"} ${s.at} :: ${s.line}`);
  console.log(`spawn-check: ${sites.length} sites, unchecked ${unchecked.length}`);
  assert.equal(sites.length, EXPECTED_CHECKED.length, `want ${EXPECTED_CHECKED.length} sites, got ${sites.length}: ${sites.map((s) => s.at).join(", ")}`);
  assert.deepEqual(unchecked, [], `unchecked spawns ${unchecked.length}: ${unchecked.join(", ")} (want unchecked 0)`);
});
test("pins the exact checked sites", () => {
  const sites = collectSites();
  const got = sites.map((s) => s.at).sort();
  assert.deepEqual(got, [...EXPECTED_CHECKED].sort(), `sites moved; re-audit file:line list (got ${got.join(", ")})`);
  for (const s of sites) assert.equal(s.checked, true, `site ${s.at} lost its exit-code check`);
});
