// eval-bar tests: the card table reads back, and a line drops red by the
// scorecard's own rule (a lower rate or a lost line), never by a new threshold.
import assert from "node:assert/strict";
import test from "node:test";
import { compare, main, parseCard, ratePermille } from "./eval-bar.mjs";

const CARD = `# Agent scorecard 1970-01-01

| suite | tier | passed | total | pass rate | tokens per task | vs last card |
|---|---|---|---|---|---|---|
| BreakBench | replay | 0 | 0 | unbuilt | unknown | - |
| EV1 | free | 25 | 25 | 100.0% | unknown | - |
| EV1 | strong | 24 | 25 | 96.0% | unknown | - |

RED lines: 0
`;

const bar = (rows) => ({ rows });

test("a card table reads back every row", () => {
  assert.deepEqual(parseCard(CARD), [
    { suite: "BreakBench", tier: "replay", passed: 0, total: 0 },
    { suite: "EV1", tier: "free", passed: 25, total: 25 },
    { suite: "EV1", tier: "strong", passed: 24, total: 25 },
  ]);
  assert.throws(() => parseCard("no table"), /no scorecard table rows/);
});

test("an unbuilt line has no rate, a full one is 100%", () => {
  assert.equal(ratePermille({ passed: 0, total: 0 }), null);
  assert.equal(ratePermille({ passed: 25, total: 25 }), 1000);
  assert.equal(ratePermille({ passed: 24, total: 25 }), 960);
});

test("the same numbers hold and a drop is red", () => {
  const card = parseCard(CARD);
  const holds = compare(bar([
    { suite: "EV1", tier: "free", passed: 25, total: 25 },
    { suite: "EV1", tier: "strong", passed: 24, total: 25 },
    { suite: "BreakBench", tier: "replay", passed: 0, total: 0 },
  ]), card);
  assert.deepEqual(holds.reds, []);
  const drops = compare(bar([{ suite: "EV1", tier: "strong", passed: 25, total: 25 }]), card);
  assert.deepEqual(drops.reds, [{ suite: "EV1", tier: "strong", was: 1000, now: 960 }]);
});

test("a lost line is red and a new line is not", () => {
  const card = parseCard(CARD).filter((line) => line.suite !== "BreakBench");
  const lost = compare(bar([{ suite: "spatial_eval", tier: "replay", passed: 100, total: 100 }]), card);
  assert.deepEqual(lost.reds, [{ suite: "spatial_eval", tier: "replay", was: 1000, now: null }]);
  const fresh = compare(bar([{ suite: "EV1", tier: "free", passed: 20, total: 25 }]), card);
  assert.deepEqual(fresh.reds, []);
});

test("no card prints the way out and exits 2", () => {
  const said = [];
  assert.equal(main(["--card", "definitely-missing-card.md"], (line) => said.push(line)), 2);
  assert.match(said.join("\n"), /no scorecard card found/);
});
