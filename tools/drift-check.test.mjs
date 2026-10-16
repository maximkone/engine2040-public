import assert from "node:assert/strict";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { readFileSync } from "node:fs";
import {
  DRIFT_EXCUSE_SOURCE,
  DRIFT_NEG_RE,
  DRIFT_ROW_RE,
  excuseLines,
  focusFamilies,
  isExcuseLine,
  isShellJunk,
  parseInboxOpen,
  parseKeeperState,
  medianMs,
  rowTimeoutGuidance,
  tuneStuckTimeout,
  STUCK_STATIC_THINKER_MS,
  STUCK_STATIC_WINDOW_MS,
  STUCK_TUNED_MIN_MS,
  shellJunk,
} from "./drift-check.mjs";

test("focus families are the FOCUS Now items: their Rows: IDs, and an item named last only warns", () => {
  const focus = "# F\n\n## Now (owner)\n\nWins over all below.\n\n1. **Look:** captures. Proof: x. Rows: FC1 EG8 EG11 Q1\n" +
    "2. **Bodies:** cuts. Rows: EG45 HS2\n3. **AI door:** door. Rows: HS29 AL1\n5. **Multiplayer (last, owner 2026-10-10):** duel. Rows: MP24 ON30\n" +
    "\n10-10: Q1 off GX13 needs.\n\n## Later\n\n1. **Old:** Rows: OPT1 DT5\n";
  const fams = focusFamilies(focus);
  assert.deepEqual(fams.map((f) => [f.name, f.ids, f.last]), [
    ["look", ["FC1", "EG8", "EG11", "Q1"], false],
    ["bodies", ["EG45", "HS2"], false],
    ["ai door", ["HS29", "AL1"], false],
    ["multiplayer", ["MP24", "ON30"], true],
  ]);
  assert.ok(fams[0].re.test("EG11 one real sun shadow map"));
  assert.ok(fams[0].re.test("Review EG8: a pass is live"));
  assert.ok(!fams[0].re.test("EG110 something") && !fams[0].re.test("OPT1 speed"), "whole IDs only, nothing from other sections");
  assert.deepEqual(focusFamilies("# F\n\n## Later\n1. **x:** Rows: A1\n"), []);
  // The real file parses: every Now item gives a family.
  const real = readFileSync(fileURLToPath(new URL("../docs/FOCUS.md", import.meta.url)), "utf8");
  assert.ok(focusFamilies(real).length >= 4, "docs/FOCUS.md Now items parse");
});

test("shell junk: seeded = and chk-*.out names fail, plain names pass", () => {
  assert.equal(isShellJunk("=lead-seed"), true);
  assert.equal(isShellJunk("chk-lead-seed.out"), true);
  assert.equal(isShellJunk("=query"), true);
  assert.equal(isShellJunk("chk-2026-10-05.out"), true);
  assert.equal(isShellJunk("sprint-handoff.md"), false);
  assert.equal(isShellJunk("chk-lead-seed"), false, "no .out suffix: not junk");
  assert.equal(isShellJunk("check.out"), false);
});

test("shellJunk filters a root listing down to the droppings", () => {
  assert.deepEqual(
    shellJunk(["=lead-seed", "chk-lead-seed.out", "docs", "tools", "chk-lead-seed"]),
    ["=lead-seed", "chk-lead-seed.out"],
  );
  assert.deepEqual(shellJunk(["docs", "tools", "package.json"]), []);
});

test("excuse: a GPU row called operator work fails", () => {
  assert.equal(isExcuseLine("GX3 blocked: GPU-gated, operator-side handoff"), true);
  assert.equal(isExcuseLine("Q1 needs operator-side capture"), true);
  assert.equal(isExcuseLine("RL5 has no GPU on this box"), true, "the no inside no GPU still fails");
  assert.equal(isExcuseLine("GM3 needs a GPU runner"), true);
});

test("excuse: denials and rows without an excuse pass", () => {
  assert.equal(isExcuseLine("GX3 is not operator-side: this machine is the reference GPU"), false);
  assert.equal(isExcuseLine("No row is operator-side here"), false);
  assert.equal(isExcuseLine("Q1 never operator-side, taking it on the laptop"), false);
  assert.equal(isExcuseLine("BX1 needs a new fixture"), false, "no row id, no excuse");
  assert.equal(isExcuseLine("GX3 needs a new fixture"), false, "row id without excuse phrase passes");
  assert.equal(isExcuseLine("GPU-gated queue with no row id"), false, "excuse without a GPU row id");
  assert.match("GX3".toString(), DRIFT_ROW_RE);
  assert.equal(DRIFT_NEG_RE.test("not"), true);
});

test("excuseLines keeps only the offending handoff lines", () => {
  const bad = excuseLines([
    "GX3 GPU-gated, operator-side for 20 waves",
    "GX3 is not operator-side: this machine renders it",
    "BX1 needs a new fixture",
  ].join("\n"));
  assert.deepEqual(bad, ["GX3 GPU-gated, operator-side for 20 waves"]);
  assert.deepEqual(excuseLines(""), []);
});

test("inbox: only open boxes count, ticked boxes ignored", () => {
  const inbox = [
    "- [ ] EB-2026-10-03 first open",
    "- [x] EB-2026-10-02 done, ignored",
    "- [ ] EB-2026-10-04 second open",
    "  - [ ] EB-2026-10-04 indented, not a top-level box",
  ].join("\n");
  assert.deepEqual(parseInboxOpen(inbox), ["2026-10-03", "2026-10-04"]);
  assert.deepEqual(parseInboxOpen(""), []);
});


test("keeperState: corrupt loop-keeper.json reports its parse error", () => {
  assert.deepEqual(parseKeeperState(""), { state: null, error: null });
  assert.deepEqual(parseKeeperState("   "), { state: null, error: null });
  assert.deepEqual(parseKeeperState("null"), { state: null, error: null });
  const bad = parseKeeperState("{not json");
  assert.equal(bad.state, null);
  assert.ok(typeof bad.error === "string" && bad.error.length > 0);
  const good = parseKeeperState(`{"decision":"go","at":"t"}`);
  assert.equal(good.error, null);
  assert.equal(good.state.decision, "go");
});

test("tuner: 3 done records tune the stuck window off static", () => {
  const hour = 3_600_000;
  const done = [2 * hour, 4 * hour, 6 * hour];
  const tuned = tuneStuckTimeout(done, STUCK_STATIC_WINDOW_MS);
  assert.notEqual(tuned, STUCK_STATIC_WINDOW_MS);
  assert.equal(tuned, 12 * hour);
  assert.ok(tuned < STUCK_STATIC_WINDOW_MS);
  assert.match(rowTimeoutGuidance("Q1", done), /tune 72h -> 12h/);
});

test("tuner: thin or bad samples keep the static window", () => {
  assert.equal(tuneStuckTimeout([], STUCK_STATIC_WINDOW_MS), STUCK_STATIC_WINDOW_MS);
  assert.equal(tuneStuckTimeout([5], STUCK_STATIC_WINDOW_MS), STUCK_STATIC_WINDOW_MS);
  assert.equal(medianMs([]), 0);
  assert.match(rowTimeoutGuidance("Q1", [1]), /keep 72h/);
});

test("drift swallows count failures instead of passing silently", () => {
  const src = readFileSync(fileURLToPath(new URL("./drift-check.mjs", import.meta.url)), "utf8");
  assert.ok(src.includes("git status failed"), "git status failure is a FAIL");
  assert.ok(src.includes("board log unreadable"), "board log failure is a FAIL");
  assert.ok(src.includes("cannot list root"), "root list failure is a FAIL");
  assert.ok(src.includes("focus: the loop sent") && src.includes('say("WARN", `focus:'), "thin focus sample is WARN, not PASS");
  assert.ok(!src.includes('if (titles.length < 10) say("PASS"'), "no PASS-on-thin-sample lie remains");
});

test("tuner null-db input: median filters junk before the estimate", () => {
  const hour = 3_600_000;
  // Junk samples never reach the median: NaN, Infinity, zero, negative, over 30 days.
  assert.equal(medianMs([NaN, Infinity, -hour, 0, 31 * 24 * hour]), 0);
  assert.equal(medianMs([2 * hour, NaN, 4 * hour, Infinity, 6 * hour]), 4 * hour);
  assert.equal(medianMs([6 * hour, 2 * hour, 4 * hour]), 4 * hour, "sorted before pick");
  assert.equal(medianMs([2 * hour, 4 * hour]), 3 * hour, "even count averages pair");
  assert.equal(medianMs([5 * hour]), 5 * hour);
});

test("tuner null-db input: clamp to floor, cap and bad bases", () => {
  const hour = 3_600_000;
  // Tiny medians clamp up to the 4 h floor.
  assert.equal(tuneStuckTimeout([1000, 2000, 3000], STUCK_STATIC_WINDOW_MS), STUCK_TUNED_MIN_MS);
  // Huge medians cap at twice the base.
  assert.equal(
    tuneStuckTimeout([100 * hour, 120 * hour, 140 * hour], STUCK_STATIC_WINDOW_MS),
    2 * STUCK_STATIC_WINDOW_MS,
  );
  // Junk mixed into a good sample is ignored, not counted.
  assert.equal(tuneStuckTimeout([2 * hour, 4 * hour, 6 * hour, NaN, -1], STUCK_STATIC_WINDOW_MS), 12 * hour);
  // Bad bases fall back to the static window.
  assert.equal(tuneStuckTimeout([2 * hour, 4 * hour, 6 * hour], NaN), 12 * hour);
  assert.equal(tuneStuckTimeout([2 * hour, 4 * hour, 6 * hour], -1), 12 * hour);
  // Two valid samples is still thin: keeps base.
  assert.equal(tuneStuckTimeout([2 * hour, 4 * hour, NaN], STUCK_STATIC_WINDOW_MS), STUCK_STATIC_WINDOW_MS);
});

test("tuner null-db input: guidance names thin vs tuned samples", () => {
  const hour = 3_600_000;
  assert.match(rowTimeoutGuidance("Q1", []), /Q1: keep 72h \(n=0, thin sample\)/);
  assert.match(rowTimeoutGuidance("Q1", [2 * hour, 4 * hour, 6 * hour]), /Q1: tune 72h -> 12h \(n=3, median 4h\)/);
  // Junk samples do not inflate n.
  assert.match(rowTimeoutGuidance("Q1", [2 * hour, 4 * hour, NaN]), /n=2, thin sample/);
  assert.match(rowTimeoutGuidance("Q1", [1000, 2000, 3000]), /tune 72h -> 4h/);
});

test("tuner constants: floor below thinker below static window", () => {
  assert.equal(STUCK_STATIC_WINDOW_MS, 72 * 3_600_000);
  assert.equal(STUCK_STATIC_THINKER_MS, 48 * 3_600_000);
  assert.equal(STUCK_TUNED_MIN_MS, 4 * 3_600_000);
  assert.ok(STUCK_TUNED_MIN_MS < STUCK_STATIC_THINKER_MS);
  assert.ok(STUCK_STATIC_THINKER_MS < STUCK_STATIC_WINDOW_MS);
});

test("keeperState null-db input: blank, scalar and broken JSON", () => {
  assert.deepEqual(parseKeeperState(null), { state: null, error: null });
  assert.deepEqual(parseKeeperState(undefined), { state: null, error: null });
  // Bare "null" parses to null state with no error, same as blank.
  assert.deepEqual(parseKeeperState("null"), { state: null, error: null });
  const num = parseKeeperState("42");
  assert.equal(num.error, null);
  assert.equal(num.state, 42);
  const arr = parseKeeperState("[1,2]");
  assert.deepEqual(arr.state, [1, 2]);
  const cut = parseKeeperState(`{"decision":"go"`);
  assert.equal(cut.state, null);
  assert.ok(typeof cut.error === "string" && cut.error.length > 0);
});

test("inbox null-db input: null, CRLF and non-dates ignored", () => {
  assert.deepEqual(parseInboxOpen(null), []);
  assert.deepEqual(parseInboxOpen(undefined), []);
  assert.deepEqual(parseInboxOpen("- [ ] EB-2026-10-03 open\r\n- [x] EB-2026-10-02 done\r\n"), ["2026-10-03"]);
  assert.deepEqual(parseInboxOpen("- [ ] EB-not-a-date\n- [ ] EB-2026-10-04 ok\n"), ["2026-10-04"]);
  assert.deepEqual(parseInboxOpen("- [X] EB-2026-10-05 upper tick ignored\n"), []);
});

test("shell junk edges: bare marker, case and missing prefix", () => {
  assert.equal(isShellJunk("="), true, "bare = is still a droppings name");
  assert.equal(isShellJunk(""), false);
  assert.equal(isShellJunk("chk-.out"), true);
  assert.equal(isShellJunk("CHK-lead.out"), false, "match is lowercase chk- only");
  assert.equal(isShellJunk(" chk-lead-seed.out"), false, "leading space is not junk");
  assert.deepEqual(shellJunk([]), []);
  const names = ["=a", "docs"];
  shellJunk(names);
  assert.deepEqual(names, ["=a", "docs"], "filter does not mutate the listing");
});

test("empire rows: every GPU family matches, other rows do not", () => {
  for (const id of ["GX3", "RL5", "RT2", "VG1", "VX4", "Q1", "PQ1", "GM3"]) {
    assert.equal(DRIFT_ROW_RE.test(id), true, `${id} is a GPU row`);
  }
  for (const id of ["BX1", "DT5", "OPT3", "SP35", "MP1", "EB-2026-10-03"]) {
    assert.equal(DRIFT_ROW_RE.test(id), false, `${id} is not a GPU row`);
  }
  assert.equal(DRIFT_ROW_RE.test("GX3 GX5"), true);
});

test("empire excuses: every operator phrase counts, denials do not", () => {
  const src = new RegExp(DRIFT_EXCUSE_SOURCE, "i");
  assert.ok(src.test("GPU-gated"), "hyphen form");
  assert.ok(src.test("GPU gated"), "space form");
  assert.ok(src.test("operator-only"), "operator-only form");
  assert.ok(src.test("needs a GPU"), "needs a GPU form");
  assert.ok(src.test("needs the GPU"), "needs the GPU form");
  assert.ok(src.test("no GPU"), "bare no GPU form");
  assert.equal(isExcuseLine("GX3 is operator-only, waiting on the laptop"), true);
  assert.equal(isExcuseLine("VG1 needs the GPU runner tonight"), true);
  assert.equal(isExcuseLine("RT2 needs a GPU runner tonight"), true);
  assert.equal(isExcuseLine("VX4 is GPU gated and operator only"), true);
  assert.equal(isExcuseLine("PQ1 has no GPU on this box"), true);
  assert.equal(isExcuseLine("GX3 is not GPU-gated, rendering here"), false, "negation before phrase passes");
  assert.equal(isExcuseLine("Q1 never needs a GPU, taking it locally"), false, "never before phrase passes");
  assert.equal(isExcuseLine("GX3 renders locally, no excuse here"), false);
});

test("empire excuses: negation window is the 40 chars before the phrase", () => {
  // "not" 41 chars before the phrase falls outside the window: still an excuse.
  const far = `GX3 ${"not" + "x".repeat(38)} GPU-gated today`;
  assert.ok(far.indexOf("GPU-gated") - far.indexOf("not") > 40);
  assert.equal(isExcuseLine(far), true, "negation outside 40 chars does not cancel");
  // Same "not" inside the window cancels the excuse.
  const near = "GX3 not ready but GPU-gated today";
  assert.equal(isExcuseLine(near), false);
  // Negation after the phrase does not cancel either.
  assert.equal(isExcuseLine("GX3 GPU-gated today, not tomorrow"), true);
});

test("empire excuses: excuseLines handles blanks and non-strings", () => {
  assert.deepEqual(excuseLines(null), []);
  assert.deepEqual(excuseLines(undefined), []);
  assert.deepEqual(excuseLines("GX3 GPU-gated, operator-side\n\nBX1 needs a fixture\n"), [
    "GX3 GPU-gated, operator-side",
  ]);
  const two = excuseLines("GX3 GPU-gated\nQ1 needs a GPU runner");
  assert.deepEqual(two, ["GX3 GPU-gated", "Q1 needs a GPU runner"]);
});

test("drift reads the board with the archive: moved done rows count as ticked", () => {
  // The stuck check reads the same board as the other readers; a needs: on a
  // moved row must resolve, so the parser gets docs/archive/SPRINT-done-*.md too.
  const src = readFileSync(fileURLToPath(new URL("./drift-check.mjs", import.meta.url)), "utf8");
  assert.ok(src.includes("archivedDoneRows"), "drift imports the shared archive reader");
  assert.ok(/parseBoard\(boardText,\s*archivedDoneRows\(ROOT\)\)/.test(src), "the stuck check passes the archive to parseBoard");
});
