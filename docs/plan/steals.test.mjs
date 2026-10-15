// steals.md rot guard: the superseded ON40 UNVERIFIED line stays pruned.
// The 2026-10-07 ON40 line (MIT, verified live) resolves the old UNVERIFIED
// license, so the 2026-10-04 UNVERIFIED duplicate must not come back.
// Fails on revert: both assertions fail while lines 23-24 are present.
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const STEALS = join(dirname(fileURLToPath(import.meta.url)), "steals.md");
const text = readFileSync(STEALS, "utf8").replace(/\r\n/g, "\n");
const lines = text.split("\n");
const open = lines.filter((l) => /^\d{4}-\d\d-\d\d \| .* \| open\s*$/.test(l));

test("no UNVERIFIED open steal line remains", () => {
  const bad = open.filter((l) => /\| UNVERIFIED \|/.test(l));
  assert.equal(bad.length, 0, `UNVERIFIED open lines must stay pruned: ${bad.join(" / ")}`);
});

test("at most one open ON40 ggrs input_queue line", () => {
  const dupes = open.filter((l) => /ON40/.test(l) && /input_queue/.test(l));
  assert.ok(dupes.length <= 1, `duplicate ON40 input_queue open lines: ${dupes.length}`);
});

test("the superseding ON40 MIT line and the P6 puffin line are intact", () => {
  assert.ok(open.some((l) => /e97e3d2416/.test(l)), "superseding ON40 MIT line (e97e3d2416) missing");
  assert.ok(open.some((l) => /P6 \(PF2\)/.test(l) && /puffin/.test(l)), "P6 puffin line missing");
});
