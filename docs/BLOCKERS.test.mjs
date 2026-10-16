// Run: node --test docs/BLOCKERS.test.mjs
// Speed fix f03: BLOCKERS holds only open blockers; resolved rows live in git history.
// Docs-side zero-caller grep gate: a stale "unwired, zero callers" row must not
// linger once its production caller lands (TV2/rebake_for_set proved the skew).
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const BLOCKERS_PATH = join(ROOT, "docs", "BLOCKERS.md");
const NAV_DIRTY_PATH = join(ROOT, "crates", "destruction", "src", "nav_dirty.rs");

function blockers() {
  return readFileSync(BLOCKERS_PATH, "utf8").replace(/\r\n/g, "\n");
}

test("header says only open blockers live here", () => {
  const head = blockers().split("\n").slice(0, 6).join("\n");
  assert.match(head, /Only open blockers live here/, "header states the open-only rule");
  assert.match(head, /git log -p -- docs\/BLOCKERS\.md/, "header points at git history");
  assert.match(head, /archived weekly/, "header states weekly archive (remove it and this goes red)");
  assert.match(head, /Read only the date section/, "header says read date section only (remove it and this goes red)");
});

test("no resolved marker lingers (restore one and this goes red)", () => {
  const text = blockers();
  assert.doesNotMatch(text, /blocker resolved, drop this open row on next pass/, "px7 markers row is dropped");
  assert.doesNotMatch(text, /Resolved 2026-10-06: 529-px-casts/, "PX30 ticked row is dropped");
  assert.doesNotMatch(text, /owner word closed it/, "render-dirt closed row is dropped");
  assert.doesNotMatch(text, /RESOLVED wave 45/, "GE2 ledger row is dropped");
  assert.doesNotMatch(text, /^## Resolved /m, "resolved sections are archived weekly (restore one and this goes red)");
  assert.doesNotMatch(text, /staged_fight\.rs.*resolved 2026-10-03/, "header staged_fight resolved entry is dropped");
  assert.doesNotMatch(text, /Auto-fix drafts unsafe/, "DOC1/FX3 closed row is archived (restore it and this goes red)");
  assert.doesNotMatch(text, /Closed 2026-10-05/, "closed rows drop on weekly archive (restore it and this goes red)");
});

test("TV2 stays pruned while its production cooker caller exists", () => {
  const text = blockers();
  assert.doesNotMatch(text, /TV2 cooker entry unwired/, "TV2 row is gone once wired");
  assert.doesNotMatch(text, /tv2-cooker-unwired/, "TV2 fingerprint is gone once wired");
  const dirty = readFileSync(NAV_DIRTY_PATH, "utf8").replace(/\r\n/g, "\n");
  const production = dirty.split("#[cfg(test)]")[0];
  assert.ok(
    production.includes("engine_nav::links::rebake_for_set"),
    "WD9 cooker calls rebake_for_set in production code (revert the caller and this goes red)",
  );
});

test("open unwired rows keep their fingerprints (no over-prune)", () => {
  // 2026-10-13 f03: MP8/AM13/MP6/AM7/RA14 rows archived to git history when wired;
  // current open set is FIX5/PX36/FIX3C/RT19/shadow (one live-caller gate per fingerprint).
  const text = blockers();
  assert.match(text, /fix5-unwired/, "FIX5 row stays");
  assert.match(text, /px36-unwired/, "PX36 row stays");
  assert.match(text, /fix3c-unwired-island/, "FIX3C row stays");
  assert.match(text, /rt19-unwired/, "RT19 row stays");
  assert.match(text, /shadow-slice-over-max-unwired/, "shadow sun row stays");
});

test("one Open section so readers read only their date section (restore the duplicate and this goes red)", () => {
  const text = blockers();
  const count = text.split("\n## Open").length - 1;
  assert.equal(count, 1, "single ## Open section (duplicate header is rot)");
  assert.match(text, /px36-unwired/, "PX36 row stays in the Open section");
});

test("PX36 has one open unwired row (restore the duplicate and this goes red)", () => {
  const text = blockers();
  const rows = text.split("\n").filter((l) => l.includes("px36-unwired")).length;
  assert.equal(rows, 1, "single px36-unwired row (x3 row only; x2/LIFTED pruned f03)");
  assert.match(text, /px36-unwired x3/, "kept row is the x3 open blocker");
  assert.doesNotMatch(text, /caller block LIFTED/, "LIFTED status update is history, not an open row");
  assert.doesNotMatch(text, /remainder update: gate now PASS/, "PASS status update is history, not an open row");
});

test("FIX3C has one open island row (restore the duplicate and this goes red)", () => {
  const text = blockers();
  const rows = text.split("\n").filter((l) => l.includes("fix3c-unwired-island")).length;
  assert.equal(rows, 1, "single fix3c-unwired-island row (newest row only; older duplicate pruned f03)");
});

test("resolved GP18.b stays archived (restore the bullet and this goes red)", () => {
  const text = blockers();
  assert.doesNotMatch(text, /gp18b-slot-timeout/, "GP18.b proof-green row is archived");
  assert.doesNotMatch(text, /GP18\.b bench proof starved/, "GP18.b bullet is archived");
  const archive = readFileSync(join(ROOT, "docs", "archive", "BLOCKERS-archive.md"), "utf8").replace(/\r\n/g, "\n");
  assert.match(archive, /gp18b-slot-timeout/, "archive keeps the GP18.b fingerprint");
  assert.match(archive, /landed as 9e7c72fb/, "archive keeps the landing receipt");
});
