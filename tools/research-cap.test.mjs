import assert from "node:assert/strict";
import test from "node:test";
import { boardRows, breaches, CAP, cardRow, cardRowFor, failLine, nextRows, orphans, rowFromText } from "./research-cap.mjs";

const BOARD = [
  "| ID | Home | Done-when | Status | Touched |",
  "|----|------|-----------|--------|---------|",
  "| C9 | bodies | cloth lands | ready | 2026-10-05 |",
  "| C48-SPATIALFIX | aidoor | oracles paper | partial | 2026-10-05 |",
  "| C77 | look | sky ships | not-ready | 2026-10-05 |",
  "| D2 | look | sky done | done 2026-10-04 | 2026-10-04 |",
].join("\n");
const IDS = ["C9", "C48-SPATIALFIX", "C48", "C77", "D2"];

test("boardRows reads the Rows table and marks done rows", () => {
  const rows = boardRows(BOARD);
  assert.equal(rows.size, 4);
  assert.equal(rows.get("C9").done, false);
  assert.equal(rows.get("C9").home, "bodies");
  assert.equal(rows.get("C48-SPATIALFIX").status, "partial");
  assert.equal(rows.get("D2").done, true);
});

test("cardRow keeps the longest row id that prefixes the file name", () => {
  assert.equal(cardRow("C48-SPATIALFIX-accept-paper6.md", IDS), "C48-SPATIALFIX");
  assert.equal(cardRow("C9.md", IDS), "C9");
  assert.equal(cardRow("2026-09-30-topic.md", IDS), null);
});

test("rowFromText links only the Row: line, never a prose mention", () => {
  assert.equal(rowFromText("- Row: C48-SPATIALFIX", IDS), "C48-SPATIALFIX");
  assert.equal(rowFromText("Rows: C9, C48-SPATIALFIX", IDS), "C48-SPATIALFIX");
  assert.equal(rowFromText("this row C9 is great work", IDS), null);
  assert.equal(rowFromText("", IDS), null);
});

test("cardRowFor prefers the file name over the Row: line", () => {
  assert.equal(cardRowFor("C9-notes.md", "- Row: C48-SPATIALFIX", IDS), "C9");
  assert.equal(cardRowFor("2026-09-30-topic.md", "- Row: C9", IDS), "C9");
});

test("breaches flags open rows over the cap, biggest first; done rows never breach", () => {
  const cardNames = [
    "C9-a.md", "C9-b.md", "C9-c.md", "C9-d.md", "C9-e.md",
    "D2-a.md", "D2-b.md", "D2-c.md", "D2-d.md", "D2-e.md", "D2-f.md",
    "2026-09-30-c48.md",
  ];
  const cardTexts = new Map([["2026-09-30-c48.md", "- Row: C48-SPATIALFIX"],
    ["C48-SPATIALFIX-x.md", ""], ["C48-SPATIALFIX-y.md", ""], ["C48-SPATIALFIX-z.md", ""]]);
  const names = [...cardNames, "C48-SPATIALFIX-x.md", "C48-SPATIALFIX-y.md", "C48-SPATIALFIX-z.md"];
  const found = breaches({ cardNames: names, board: BOARD, cardTexts });
  assert.deepEqual(found.map((b) => [b.row, b.cards]), [["C9", 5], ["C48-SPATIALFIX", 4]]);
  assert.match(failLine(found[0]), /row C9 has 5 cards \(cap 3\)/);
  const closed = breaches({ cardNames: names, board: BOARD.replace("| ready |", "| done 2026-10-06 |"), cardTexts });
  assert.deepEqual(closed.map((b) => [b.row, b.cards]), [["C48-SPATIALFIX", 4]]);
});

test("a row at exactly the cap is clean", () => {
  const names = ["C77-a.md", "C77-b.md", "C77-c.md"];
  assert.deepEqual(breaches({ cardNames: names, board: BOARD, cap: CAP }), []);
  assert.equal(CAP, 3);
});

test("nextRows serves partial first, skips full, done and freshly claimed rows", () => {
  const base = { cardNames: ["C48-SPATIALFIX-a.md", "C48-SPATIALFIX-b.md"], board: BOARD, n: 3 };
  const open = nextRows({ ...base, now: Date.parse("2026-10-06T14:00:00Z") });
  assert.deepEqual(open.map((r) => r.row), ["C48-SPATIALFIX", "C77", "C9"]);
  const claimed = nextRows({
    ...base,
    claims: "C48-SPATIALFIX | builder-aidoor-r1 | 2026-10-06T13:30Z | docs/research/cards/C48-x.md",
    now: Date.parse("2026-10-06T14:00:00Z"),
  });
  assert.deepEqual(claimed.map((r) => r.row), ["C77", "C9"]);
  const stale = nextRows({
    ...base,
    claims: "C48-SPATIALFIX | builder-aidoor-r1 | 2026-10-06T09:00Z | docs/research/cards/C48-x.md",
    now: Date.parse("2026-10-06T14:00:00Z"),
  });
  assert.equal(stale[0].row, "C48-SPATIALFIX");
  const full = nextRows({ cardNames: ["C77-a.md", "C77-b.md", "C77-c.md"], board: BOARD, n: 5, now: Date.parse("2026-10-06T14:00:00Z") });
  assert.ok(!full.some((r) => r.row === "C77"), "a row at the cap has no room");
});

test("orphans names date cards with no Row: line", () => {
  const rows = boardRows(BOARD);
  const texts = new Map([["2026-09-30-other.md", "- Row: C9"]]);
  assert.deepEqual(
    orphans(["2026-09-30-topic.md", "2026-09-30-other.md", "C9-notes.md"], rows, texts),
    ["2026-09-30-topic.md"],
  );
});
