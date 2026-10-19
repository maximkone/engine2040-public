// Run: node --test tools/nonstop.test.mjs
// nonstop's row picking and prompt, plus --help and --dry (which run no model).
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";
import { buildPrompt, classifyRun, opencodeBin, parseReadyList, pickNext } from "./nonstop.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));
const READY = [
  "NS31a2   open Rome first (2026-10-02 res material: the editor document holds the table",
  "E-4     open inbox row with a hyphen",
  "CQ-NET-API5 open hyphenated twice",
  "753      open parse artifact that is not an id",
  "SG3      done not an open row",
  "NS31a2   open duplicate line",
].join("\n");

test("parseReadyList keeps open ids once, hyphens included, and drops non-ids", () => {
  assert.deepEqual(parseReadyList(READY), ["NS31a2", "E-4", "CQ-NET-API5"]);
});

test("pickNext skips done rows and honors --ids and --match", () => {
  const ready = ["NS31a2", "E-4", "CQ-NET-API5"];
  assert.equal(pickNext(ready, new Set()), "NS31a2");
  assert.equal(pickNext(ready, new Set(["NS31a2"])), "E-4");
  assert.equal(pickNext(ready, new Set(), { match: "^CQ" }), "CQ-NET-API5");
  assert.equal(pickNext(ready, new Set(), { ids: ["CQ-NET-API5", "E-4"] }), "CQ-NET-API5");
  assert.equal(pickNext(ready, new Set(), { ids: ["GONE1"] }), null);
  assert.equal(pickNext(ready, new Set(ready)), null);
});

test("buildPrompt names the row, the date, the one-line ending and the hard rules", () => {
  const p = buildPrompt("QR2", "## Row\n- [ ] QR2 body", "2026-10-19");
  for (const needle of ["Row: QR2", "2026-10-19", "- [x] QR2", "DONE QR2", "git add -A", "gh-check.mjs", "Never ask a question"]) {
    assert.ok(p.includes(needle), needle);
  }
});

test("opencodeBin ignores OpenCode's own OPENCODE=1 marker and takes a real path", () => {
  assert.equal(opencodeBin({ OPENCODE: "1" }), "opencode");
  assert.equal(opencodeBin({}), "opencode");
  assert.equal(opencodeBin({ OPENCODE: "C:\\tools\\opencode.exe" }), "C:\\tools\\opencode.exe");
  assert.equal(opencodeBin({ OPENCODE: "/usr/local/bin/opencode" }), "/usr/local/bin/opencode");
  assert.equal(opencodeBin({ OPENCODE: "1", OPENCODE_BIN: "C:\\opencode\\opencode.exe" }), "C:\\opencode\\opencode.exe");
  assert.equal(opencodeBin({ OPENCODE_BIN: "custom-opencode", OPENCODE: "C:\\x.exe" }), "custom-opencode");
});

test("classifyRun reads a clock kill as a timeout, not a model error", () => {
  assert.equal(classifyRun({ ticked: true }), "ticked");
  assert.equal(classifyRun({ timedOut: true }), "timeout");
  assert.equal(classifyRun({ failed: true }), "model-error");
  assert.equal(classifyRun({ timedOut: true, failed: true }), "model-error", "a real failure wins");
  assert.equal(classifyRun({ changed: true }), "progress");
  assert.equal(classifyRun({ timedOut: true, changed: true }), "progress", "an edit before the kill counts");
  assert.equal(classifyRun({}), "unchanged");
  assert.equal(classifyRun({ ticked: true, timedOut: true, failed: true }), "ticked", "a ticked row is real");
});

test("--help exits 0 and --dry prints a prompt without running a model", () => {  const help = spawnSync(process.execPath, [join(HERE, "nonstop.mjs"), "--help"], { encoding: "utf8" });
  assert.equal(help.status, 0);
  assert.match(help.stdout, /one fresh session per row/);
  const dry = spawnSync(process.execPath, [join(HERE, "nonstop.mjs"), "--dry"], { encoding: "utf8" });
  assert.equal(dry.status, 0, dry.stderr);
  assert.match(dry.stdout, /You are finishing ONE board row/);
  const bad = spawnSync(process.execPath, [join(HERE, "nonstop.mjs"), "--nope"], { encoding: "utf8" });
  assert.equal(bad.status, 2);
});
