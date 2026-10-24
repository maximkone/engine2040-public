import { test } from "node:test";
import assert from "node:assert/strict";
import { mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { parseFixture, readReading, readingFrom, SOURCE } from "./aa-flicker.mjs";

// The line the fixture printed on the RTX 3050 (LF5d, eb2e290ba).
const FIXTURE = `running 1 test
sand edge aa default: taa (jitter+resolve) 10.60 levels (edge step 201.68), msaa 4x 0.00 levels (edge step 201.68)
test sand_edge_aa_default ... ok
`;

test("the fixture line parses into the two temporal readings and both edge steps", () => {
  const p = parseFixture(FIXTURE);
  assert.equal(p.taa_levels, 10.6);
  assert.equal(p.msaa_levels, 0);
  assert.equal(p.taa_edge_step, 201.68);
  assert.equal(p.msaa_edge_step, 201.68);
  assert.match(p.line, /^sand edge aa default:/);
});

test("a skipped fixture is a named reason, never a number", () => {
  const p = parseFixture("SKIPPED sand_edge_aa_default: adapter cannot build the 4x pair\n");
  assert.match(p.skipped, /^sand_edge_aa_default: adapter cannot build the 4x pair/);
  assert.equal(p.taa_levels, undefined);
});

test("no fixture line and a short line are missing readings, not zeros", () => {
  assert.match(parseFixture("running 1 test\ntest sand_edge_aa_default ... ok\n").missing, /printed no/);
  assert.match(parseFixture("sand edge aa default: taa 10.60 levels\n").missing, /fewer than two/);
});

test("the reading names the calmer shipped default and keeps its provenance", () => {
  const r = readingFrom({ text: FIXTURE, sha: "abc1234", at: "2026-10-24T03:00:00.000Z", host: "pc" });
  assert.equal(r.taa_levels, 10.6);
  assert.equal(r.msaa_levels, 0);
  assert.equal(r.edge_step, 201.68);
  assert.equal(r.calmer, "4x msaa", "4x msaa is calmer than jitter plus resolve here");
  assert.equal(r.sha, "abc1234");
  assert.equal(r.at, "2026-10-24T03:00:00.000Z");
  assert.equal(r.host, "pc");
  assert.equal(r.source, SOURCE);
  const taaWins = readingFrom({ text: "sand edge aa default: taa 1.00 levels (edge step 200), msaa 4x 2.00 levels (edge step 200)", sha: "abc1234" });
  assert.equal(taaWins.calmer, "taa (jitter+resolve)");
  const tie = readingFrom({ text: "sand edge aa default: taa 0.00 levels (edge step 200), msaa 4x 0.00 levels (edge step 200)", sha: "abc1234" });
  assert.equal(tie.calmer, "tie", "an exact tie names both instead of picking a default");
});

test("a skipped run keeps the skip in the reading file, and an absent file reads null", () => {
  const r = readingFrom({ text: "SKIPPED sand_edge_aa_default: no wgpu adapter on this machine", sha: "abc1234" });
  assert.match(r.skipped, /no wgpu adapter/);
  assert.equal(r.taa_levels, undefined);
  const dir = mkdtempSync(join(tmpdir(), "aa-flicker-"));
  const file = join(dir, "aa-flicker.json");
  assert.equal(readReading(file), null, "no file is no reading");
  writeFileSync(file, `${JSON.stringify(r)}\n`);
  assert.match(readReading(file).skipped, /no wgpu adapter/);
});

test("a run that printed no fixture line is missing, never written as a reading", () => {
  const r = readingFrom({ text: "running 0 tests\ntest result: ok. 0 passed\n", sha: "abc1234" });
  assert.match(r.missing, /printed no/);
  assert.equal(r.skipped, undefined, "a missing fixture is not a skip with a reason");
  assert.equal(r.taa_levels, undefined);
});
