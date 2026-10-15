// Run: node --test tools/mp-evidence.test.mjs
import assert from "node:assert/strict";
import test from "node:test";
import { CORE_AREAS, rowArea, parseRowNumber, isNewer, extractDate, extractEvidence, newestPerArea, formatLine, collectEvidence } from "./mp-evidence.mjs";
import { loadBoardText, parseBoard } from "./board-slice.mjs";
test("core areas are MP and ON", () => {
  assert.deepEqual([...CORE_AREAS], ["MP", "ON"]);
});
test("rowArea maps MP and ON ids", () => {
  assert.equal(rowArea("MP1"), "MP");
  assert.equal(rowArea("MP16b"), "MP");
  assert.equal(rowArea("MP22"), "MP");
  assert.equal(rowArea("ON1"), "ON");
  assert.equal(rowArea("ON36"), "ON");
  assert.equal(rowArea("GX1"), null);
});
test("newest means highest number then suffix", () => {
  assert.equal(isNewer("MP22", "MP21"), true);
  assert.equal(isNewer("MP21", "MP22"), false);
  assert.equal(isNewer("MP16b", "MP16a"), true);
  assert.equal(isNewer("MP16a", "MP16"), true);
  assert.equal(isNewer("ON36", "ON31"), true);
  const best = newestPerArea(new Map(), ["MP6", "MP7", "MP22", "ON9", "ON31", "ON36"]);
  assert.equal(best.get("MP"), "MP22");
  assert.equal(best.get("ON"), "ON36");
});
test("extractDate finds first date else none", () => {
  assert.equal(extractDate("(done 2026-10-09: c879e3d)"), "2026-10-09");
  assert.equal(extractDate("(RES-STEAL 680, 2026-10-08.)"), "2026-10-08");
  assert.equal(extractDate("no date here"), "none");
});
test("extractEvidence prefers slashed md then rs", () => {
  const mp7 = "(PLN31 multiplayer.md, reviewed 2026-09-25.) (done 2026-10-09: c879e3d target/queue/archive-2026-10-07/builder-net-r3-review.md.)";
  assert.equal(extractEvidence(mp7), "target/queue/archive-2026-10-07/builder-net-r3-review.md");
  assert.equal(extractEvidence("Owns crates/net/src/rollback.rs only"), "crates/net/src/rollback.rs");
  assert.equal(extractEvidence("nothing here"), "none");
});
test("live board gives one line per area with row date path", () => {
  const rows = collectEvidence();
  assert.equal(rows.length, 2);
  assert.deepEqual(rows.map((r) => r.area), ["MP", "ON"]);
  for (const r of rows) {
    assert.ok(r.row.startsWith(r.area), "row starts with area");
    assert.ok(r.date === "none" || /^\d{4}-\d{2}-\d{2}$/.test(r.date), "date or none");
    assert.ok(typeof r.evidence === "string" && r.evidence.length > 0, "path present");
    const line = formatLine(r);
    assert.ok(line.includes(r.area) && line.includes(r.row) && line.includes(r.date) && line.includes(r.evidence), "line holds area row date path");
  }
});
test("live newest is the max id per area", () => {
  const text = loadBoardText();
  const parsed = parseBoard(text);
  const rows = collectEvidence();
  for (const r of rows) {
    for (const id of parsed.order) {
      if (rowArea(id) !== r.area) continue;
      assert.equal(isNewer(id, r.row), false, `${id} should not beat ${r.row}`);
    }
  }
});
