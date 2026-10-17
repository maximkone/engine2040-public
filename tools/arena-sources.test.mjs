// Run: node --test tools/arena-sources.test.mjs
// content/arena/sources holds raw CC0 texture sets (VR2 landing 1). Each set
// needs a PROVENANCE row with CC0, colour + normal + roughness files, and
// byte sizes that match the disk. A broken fixture tree must be caught too.
import assert from "node:assert/strict";
import { mkdirSync, mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { checkSources, provenanceRows, SOURCES_DIR } from "./arena-sources.mjs";

test("the real arena sources tree is clean", () => {
  assert.deepEqual(checkSources(SOURCES_DIR), []);
});

test("every real set is listed in PROVENANCE with at least five rows", () => {
  const rows = provenanceRows(readFileSync(join(SOURCES_DIR, "PROVENANCE.md"), "utf8"));
  assert.ok(rows.length >= 5, `rows: ${rows.length}`);
});

test("a set missing its normal map, row and CC0 is reported", () => {
  const dir = mkdtempSync(join(tmpdir(), "arena-src-"));
  mkdirSync(join(dir, "Bad001"));
  writeFileSync(join(dir, "Bad001", "Bad001_Color.jpg"), "x");
  writeFileSync(join(dir, "Bad001", "Bad001_Roughness.jpg"), "x");
  writeFileSync(join(dir, "PROVENANCE.md"), "| asset id | a | b | c | d | e | f |\n|---|---|---|---|---|---|---|\n");
  const problems = checkSources(dir).join("\n");
  assert.match(problems, /Bad001: no PROVENANCE row/);
  assert.match(problems, /Bad001: no Normal image/);
  assert.match(problems, /LICENCE.txt missing/);
});

test("a row without a CC0 word is reported", () => {
  const dir = mkdtempSync(join(tmpdir(), "arena-src-"));
  mkdirSync(join(dir, "Odd001"));
  for (const k of ["Color", "NormalGL", "Roughness"]) writeFileSync(join(dir, "Odd001", `Odd001_${k}.jpg`), "x");
  writeFileSync(join(dir, "Odd001", "LICENCE.txt"), "CC0");
  writeFileSync(
    join(dir, "PROVENANCE.md"),
    "| Odd001 | url | all rights reserved | 2026-10-17 | Odd001_Color.jpg 1; Odd001_NormalGL.jpg 1; Odd001_Roughness.jpg 1; LICENCE.txt 3 | GL | stone |\n",
  );
  assert.deepEqual(checkSources(dir), ["Odd001: PROVENANCE licence cell has no CC0"]);
});
