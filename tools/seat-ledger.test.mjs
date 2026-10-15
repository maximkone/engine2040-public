// seat-ledger: header + RESULT parsing, window filter, per-role counts. Inline fixtures only.
import { test } from "node:test";
import assert from "node:assert/strict";
import { frontMatter, inWindow, parseHeaderRole, parseOutcome, parseRecord, summarize, windowRecords } from "./seat-ledger.mjs";

const NOW = 1_700_000_000_000;
const H = 3600_000;
const rec = (head, end) => `${head}\nbody line\n${end}\n`;
const headed = (role, end) => rec(`# Twin fight (${role}, wave 2, RL26)`, end);
const fm = (role, end) => rec(`---\nrole: ${role}\ntitle: t\n---`, end);

test("front matter matches the queue-cap style and tolerates CRLF", () => {
  const p = frontMatter("---\r\nrole: builder\r\ntitle: t\r\n---\r\nGoal: x\r\n");
  assert.equal(p.fm.role, "builder");
  assert.equal(frontMatter("no block"), null);
});

test("the # header names the role; front matter is the fallback", () => {
  assert.equal(parseHeaderRole("# Twin fight (builder, wave 2, RL26)"), "builder");
  assert.equal(parseHeaderRole("# Walk (Judge, wave 1, J1)"), "judge");
  assert.equal(parseHeaderRole("---\nrole: smith\n---\n"), null);
  assert.deepEqual(parseRecord(headed("builder", "RESULT: DONE - x | proof: y")), { role: "builder", outcome: "DONE" });
  assert.deepEqual(parseRecord(fm("smith", "RESULT: NOOP - x | proof: y")), { role: "smith", outcome: "NOOP" });
  assert.equal(parseRecord("no header and no outcome"), null);
});

test("the last RESULT or VERDICT line wins; PASS maps to DONE", () => {
  assert.equal(parseOutcome("RESULT: DONE - a\nRESULT: NOOP - b"), "NOOP");
  assert.equal(parseOutcome("VERDICT: PASS"), "DONE");
  assert.equal(parseOutcome("VERDICT: FAIL"), "PARTIAL");
  assert.equal(parseOutcome("VERDICT: BLOCKED"), "BLOCKED");
  assert.equal(parseOutcome("template `RESULT: DONE|PARTIAL` with no real line"), null);
});

test("summarize counts runs plus changed share per role", () => {
  const rows = summarize([
    { role: "builder", outcome: "DONE" },
    { role: "builder", outcome: "NOOP" },
    { role: "judge", outcome: "DONE" },
  ], ["builder", "judge"]);
  const b = rows.find((r) => r.role === "builder");
  const j = rows.find((r) => r.role === "judge");
  assert.equal(b.runs, 2);
  assert.equal(b.changedPct, 50);
  assert.equal(j.DONE, 1);
});

test("seats with no runs still print a zero line", () => {
  const rows = summarize([], ["builder", "judge"]);
  assert.deepEqual(rows.map((r) => r.role), ["builder", "judge"]);
  assert.ok(rows.every((r) => r.runs === 0 && r.changedPct === 0));
});

test("window by mtime keeps fresh records only", () => {
  assert.equal(inWindow(NOW - H, NOW, 24), true);
  assert.equal(inWindow(NOW - 25 * H, NOW, 24), false);
  const items = [
    { mtimeMs: NOW - H, text: headed("builder", "RESULT: DONE - a | proof: p") },
    { mtimeMs: NOW - 30 * H, text: headed("builder", "RESULT: DONE - old | proof: p") },
    { mtimeMs: NOW - H, text: fm("judge", "VERDICT: PASS") },
  ];
  const rows = summarize(windowRecords(items, NOW, 24), ["builder", "judge"]);
  assert.equal(rows.find((r) => r.role === "builder").runs, 1);
  assert.equal(rows.find((r) => r.role === "judge").DONE, 1);
});
