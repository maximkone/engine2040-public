// land: the plan a review PASS turns into a by-path commit, and every reason it refuses.
import { test } from "node:test";
import assert from "node:assert/strict";
import { execFileSync, spawnSync } from "node:child_process";
import { copyFileSync, existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { acceptCommands, changedPaths, claudeClaimed, failFiles, landedLine, latestReview, planClose, planLanding, ratchetArgs, ratchetVerdict, resultOf, subjectOf, verdictOf, workspaceVerdict } from "./land.mjs";

const record = (extra = "", fm = {}) => `---\n${Object.entries({ role: "builder", title: "EG12 torch shadows in the forward shader", row: "EG12", paths: "crates/render/src/shadow.rs, crates/render/src/forward.wgsl", ...fm }).map(([k, v]) => `${k}: ${v}`).join("\n")}\n---\nGoal: g\n\n## Result (completed)\n\nRESULT: DONE - torches cast shadows | proof: cargo test -p engine-render --lib torch 3 passed${extra}\n`;
const pass = "Checks rerun.\nRUST: 4/5 DEPTH: 3/5\nVERDICT: PASS - torches shadow, tests rerun\n";
const base = { packet: "0-eg12-torch", reviewName: "0-eg12-torch-review.md", review: pass, changed: ["crates/render/src/shadow.rs", "crates/render/src/other.rs"], now: Date.parse("2026-10-11T08:00:00Z") };

test("a DONE run with a PASS review lands only its own changed paths", () => {
  const p = planLanding({ ...base, record: record() });
  assert.equal(p.ok, true);
  assert.deepEqual(p.paths, ["crates/render/src/shadow.rs"]);
  assert.match(p.message, /^EG12: torch shadows in the forward shader\n/);
  assert.match(p.message, /RESULT: DONE - torches cast shadows/);
  assert.match(p.message, /Review 0-eg12-torch-review: VERDICT: PASS .*\(DEPTH 3\/5, RUST 4\/5\)/);
});

test("refusals: not DONE, no review, FAIL, low depth, no paths, a claude claim", () => {
  assert.match(planLanding({ ...base, record: record().replace("RESULT: DONE", "RESULT: BLOCKED") }).why, /ended BLOCKED, not DONE, PARTIAL or NOOP/);
  assert.match(planLanding({ ...base, record: record(), review: null }).why, /no review record/);
  assert.match(planLanding({ ...base, record: record(), review: "VERDICT: PASS earlier\nVERDICT: FAIL - no caller\n" }).why, /says FAIL/);
  assert.match(planLanding({ ...base, record: record(), review: "DEPTH: 2/5 RUST: 4/5\nVERDICT: PASS\n" }).why, /DEPTH 2\/5/);
  assert.match(planLanding({ ...base, record: record("", { paths: "" }) }).why, /no paths: line/);
  const template = "End with exactly these lines:\nRUST: n/5 DEPTH: n/5\nVERDICT: PASS|FAIL|BLOCKED - <one line>\n\n## Result (completed)\n\nRan out of time.\n";
  assert.match(planLanding({ ...base, record: record(), review: template }).why, /says no VERDICT/, "the template's own verdict line is no PASS");
  assert.match(planLanding({ ...base, record: record(), review: "VERDICT: PASS - fine\n" }).why, /both must be given/, "a PASS without scores does not land");
  assert.match(planLanding({ ...base, record: record().replace("3 passed", "0 passed") }).why, /ran 0 tests/, "a filter that matched nothing proves nothing");
  assert.equal(planLanding({ ...base, record: record().replace("3 passed", "10 passed") }).ok, true);
  const claims = "claude-opus | 2026-10-11T07:30Z | EG11 crates/render/src/shadow.rs\n";
  const r = planLanding({ ...base, record: record(), claims });
  assert.equal(r.code, 1);
  assert.match(r.why, /claude-\* claim holds crates\/render\/src\/shadow.rs/);
  assert.equal(planLanding({ ...base, record: record(), claims: "claude-opus | 2026-10-10T07:30Z | EG11 crates/render/src/shadow.rs\n" }).ok, true, "a claim older than 3 h has lapsed");
});

// 2026-10-12: two NOOP packets (their edits verified and uncommitted) and every PARTIAL with a PASS bounced forever.
test("a PARTIAL or a NOOP lands on a review PASS when its own paths have changes; the commit names the kind", () => {
  const partial = planLanding({ ...base, record: record().replace("RESULT: DONE - torches cast shadows", "RESULT: PARTIAL - shadow pass only") });
  assert.equal(partial.ok, true);
  assert.deepEqual(partial.paths, ["crates/render/src/shadow.rs"]);
  assert.equal(partial.status, "PARTIAL");
  assert.match(partial.message, /\nResult PARTIAL: shadow pass only \| proof: cargo test -p engine-render --lib torch 3 passed\n/);
  const noop = planLanding({ ...base, record: record().replace("RESULT: DONE - torches cast shadows", "RESULT: NOOP - verified present edit, changed nothing") });
  assert.equal(noop.ok, true);
  assert.deepEqual(noop.paths, ["crates/render/src/shadow.rs"]);
  assert.match(noop.message, /\nResult NOOP: verified present edit, changed nothing \| proof: /);
  // the rest of the PASS rule still holds for them
  const nope = (res, over = {}) => planLanding({ ...base, record: record().replace("RESULT: DONE - torches cast shadows", res), ...over });
  assert.match(nope("RESULT: PARTIAL - half", { review: "RUST: 4/5 DEPTH: 3/5\nVERDICT: FAIL - no caller\n" }).why, /says FAIL/);
  assert.match(nope("RESULT: NOOP - x", { review: null }).why, /no review record/);
  assert.match(nope("RESULT: PARTIAL - the FAIL case").why, /RESULT text says FAIL after PARTIAL/);
  assert.match(nope("RESULT: NOOP - x", { claims: "claude-opus | 2026-10-11T07:30Z | EG11 crates/render/src/shadow.rs\n" }).why, /claude-\* claim holds/);
});

test("a NOOP with no changes under its paths is nothing to land (exit 0); BLOCKED refuses", () => {
  const noop = record().replace("RESULT: DONE - torches cast shadows", "RESULT: NOOP - already in the tree");
  const none = planLanding({ ...base, record: noop, changed: ["crates/ai/src/mind.rs"] });
  assert.equal(none.ok, false);
  assert.equal(none.code, 0);
  assert.match(none.why, /nothing to land: no changes under crates\/render\/src\/shadow\.rs/);
  const blocked = planLanding({ ...base, record: record().replace("RESULT: DONE - torches cast shadows", "RESULT: BLOCKED - no GPU") });
  assert.equal(blocked.ok, false);
  assert.equal(blocked.code, 1);
  assert.match(blocked.why, /ended BLOCKED/);
});

test("a PASS LIGHT review lands with no scores; a plain PASS with no scores still refuses", () => {
  const light = planLanding({ ...base, record: record(), review: "VERDICT: PASS LIGHT - 12 changed lines, proof rerun\n" });
  assert.equal(light.ok, true);
  assert.match(light.message, /\nReview 0-eg12-torch-review: VERDICT: PASS LIGHT - 12 changed lines, proof rerun\n/, "no scores on the line");
  assert.equal(planLanding({ ...base, record: record(), review: "**VERDICT: PASS LIGHT** - small diff\n" }).ok, true);
  assert.equal(verdictOf("VERDICT: PASS LIGHT - ok\n").light, true);
  assert.equal(verdictOf("RUST: 4/5 DEPTH: 3/5\nVERDICT: PASS - ok\n").light, false);
  assert.match(planLanding({ ...base, record: record(), review: "VERDICT: PASS - fine\n" }).why, /both must be given/);
  // a light verdict is still a verdict: the last line wins, FAIL/BLOCKED refuse, the template is none
  assert.match(planLanding({ ...base, record: record(), review: "VERDICT: PASS LIGHT - ok\nVERDICT: FAIL - no caller\n" }).why, /says FAIL/);
  assert.match(planLanding({ ...base, record: record(), review: "VERDICT: PASS|FAIL|BLOCKED - <one line>\n" }).why, /says no VERDICT/);
  assert.match(planLanding({ ...base, record: record(), review: "VERDICT: PASS LIGHT - ok\nVERDICT: PASS - later, no scores\n" }).why, /both must be given/, "the last verdict line decides");
});

test("no-ops: nothing changed under the paths, or landed already", () => {
  const none = planLanding({ ...base, record: record(), changed: ["crates/ai/src/mind.rs"] });
  assert.equal(none.ok, false);
  assert.equal(none.code, 0);
  const again = planLanding({ ...base, record: record("\nLanded: abc1234 2026-10-11T08:00:00Z") });
  assert.equal(again.code, 0);
  assert.match(again.why, /already landed in abc1234/);
});

test("a read or research seat never lands", () => {
  assert.match(planLanding({ ...base, record: record("", { role: "judge" }) }).why, /role judge lands nothing/);
});

test("the newest review wins; verdict and scores are the last ones", () => {
  assert.equal(latestReview(["x-review.md", "x-review-2.md", "x-reviewer.md", "y-review-3.md"], "x"), "x-review-2.md");
  assert.equal(latestReview(["x.md"], "x"), null);
  assert.deepEqual(verdictOf("VERDICT: FAIL\nDEPTH: 2/5\nDEPTH: 4/5 RUST: 3/5\n**VERDICT: PASS** - ok").verdict, "PASS");
  assert.equal(verdictOf("DEPTH: 2/5\nDEPTH: 4/5").depth, 4);
});

test("subjects: board rows lead, rot cuts say cut, the rest say loop", () => {
  assert.equal(subjectOf({ row: "EG12", title: "EG12: torch shadows" }), "EG12: torch shadows");
  assert.equal(subjectOf({ row: "MP6, MP17", title: "MP17 relay fix" }), "MP6 MP17: relay fix");
  assert.equal(subjectOf({ row: "rot:unwired-testonly", title: "Cut test-only helper" }), "cut: Cut test-only helper");
  assert.equal(subjectOf({ row: "none", title: "dup fns" }), "loop: dup fns");
});

test("a standing seat's run record lands by its OWNED and ROW lines", () => {
  const run = "# FOCUS Now coder #5 (@builder, standing)\n\n## Result (completed)\n\nROW: EG13 | OWNED: crates/render/src/present/surface.rs, crates/render/src/forward.rs | CRATE: engine-render\nRESULT: DONE - 4x MSAA | proof: cargo test -p engine-render --lib msaa 2 passed\n";
  const p = planLanding({ ...base, packet: "builder-focus-r5", reviewName: "builder-focus-r5-review.md", record: run, changed: ["crates/render/src/forward.rs"] });
  assert.equal(p.ok, true);
  assert.deepEqual(p.paths, ["crates/render/src/forward.rs"]);
  assert.match(p.message, /^EG13: FOCUS Now coder #5\n/);
  assert.match(planLanding({ ...base, record: "no heading\nRESULT: DONE" }).why, /no front matter and no standing-run heading/);
});

test("porcelain parsing keeps both sides of a rename and new files", () => {
  assert.deepEqual(changedPaths(" M crates/a.rs\0?? crates/b.rs\0R  crates/c.rs\0crates/old.rs\0"), ["crates/a.rs", "crates/b.rs", "crates/c.rs", "crates/old.rs"]);
  assert.deepEqual([...claudeClaimed("claude-x | 2026-10-11T07:00Z | EG1 crates\\a.rs crates/b.rs\nbuilder-r1 | 2026-10-11T07:00Z | EG2 crates/c.rs\n", Date.parse("2026-10-11T08:00Z"))], ["crates/a.rs", "crates/b.rs"]);
});

// ---- the run's own words: a template line, a quoted prompt line or a FAIL is not a PASS ----
test("the reply template is no RESULT, and lines the prompt quotes are not the run's own", () => {
  const prompt = "End with `RESULT: DONE|PARTIAL|BLOCKED|NOOP - <what> | proof: <where>` (a judge ends `VERDICT: PASS|FAIL|BLOCKED`).";
  const ranOut = `---\nrole: builder\ntitle: t\nrow: EG12\npaths: crates/render/src/shadow.rs\n---\n${prompt}\n\n## Result (completed)\n\nRan out of time.\n`;
  assert.deepEqual(resultOf(ranOut), { status: null, line: "", what: "", proof: "" });
  assert.match(planLanding({ ...base, record: ranOut }).why, /ended with no RESULT line/, "the template line read as DONE before");
  // A repair prompt quotes the review it repairs and the earlier DONE; neither is this run's.
  const quoted = "Goal: g\nThe review said:\nVERDICT: FAIL - oversize\nRESULT: DONE - old try | proof: cargo test -p x 1 passed\n";
  assert.equal(planLanding({ ...base, record: record().replace("Goal: g\n", quoted) }).ok, true, "a quoted FAIL above ## Result is history");
  assert.match(planLanding({ ...base, record: record().replace("Goal: g\n", quoted).replace(/RESULT: DONE - torches[^\n]*/, "Ran out of time.") }).why, /ended with no RESULT line/, "a quoted DONE is not the run's DONE");
  const r = resultOf(record());
  assert.equal(r.status, "DONE");
  assert.equal(r.what, "torches cast shadows");
  assert.equal(r.proof, "cargo test -p engine-render --lib torch 3 passed");
});

test("the keeper's closing line `DONE - <what> | proof: <cmd>` is the run's RESULT; BLOCKED, no proof and FAIL text still refuse (3 orphan packets parked for it, 2026-10-15)", () => {
  const keeper = (line) => record().replace(/\n## Result \(completed\)[\s\S]*$/, `\nRESULT: DONE|PARTIAL|BLOCKED|NOOP - <what> | proof: <line>\n\n## Keeper result (2026-10-15T06:41:56.269Z)\n${line}\n`);
  const ok = keeper("DONE - verified 3 orphan assets edits green, ready for land by path | proof: cargo test -p engine-assets => test result: ok. 146 passed; 0 failed");
  const r = resultOf(ok);
  assert.equal(r.status, "DONE");
  assert.equal(r.what, "verified 3 orphan assets edits green, ready for land by path");
  assert.equal(r.proof, "cargo test -p engine-assets => test result: ok. 146 passed; 0 failed");
  assert.equal(planLanding({ ...base, record: ok }).ok, true, "a PASS review and a real proof land it");
  assert.equal(resultOf(keeper("PARTIAL: half of it | proof: cargo test -p x 2 passed")).status, "PARTIAL");
  assert.match(planLanding({ ...base, record: keeper("BLOCKED - build slots full | proof: none") }).why, /ended BLOCKED/);
  assert.match(planLanding({ ...base, record: keeper("DONE - looks fine") }).why, /RESULT proof is empty/);
  assert.match(planLanding({ ...base, record: keeper("DONE - 2 FAIL left | proof: cargo test -p x 1 passed") }).why, /says FAIL/);
  assert.equal(resultOf(keeper("Ran out of time.")).status, null, "prose that starts with no status word is no result");
  assert.equal(resultOf(record().replace("RESULT: DONE - torches", "RESULT: PARTIAL - torches") + "\n## Keeper result (t)\nDONE - later | proof: cargo test -p x 1 passed\n").status, "PARTIAL", "the run's own RESULT line wins over the keeper's");
});

test("a fixed FAIL in the RESULT text (FAIL -> PASS, 0 FAIL) lands; a FAIL that is left still refuses", () => {
  const withWhat = (what) => record().replace("torches cast shadows", what);
  for (const ok of ["pose fold FAIL -> PASS after the fix", "fold FAIL→PASS", "workspace-check 0 FAIL", "FAIL: 0 left"]) assert.equal(planLanding({ ...base, record: withWhat(ok) }).ok, true, ok);
  for (const bad of ["pose fold FAIL", "FAIL -> PASS but one test FAIL", "2 FAIL left"]) assert.match(planLanding({ ...base, record: withWhat(bad) }).why, /says FAIL/, bad);
});

test("a DONE whose last VERDICT line says FAIL, or whose RESULT text says FAIL, is not a PASS", () => {
  const verdictFail = planLanding({ ...base, record: record("\nVERDICT: FAIL - 1018 changed lines over max_diff 200") });
  assert.equal(verdictFail.ok, false);
  assert.match(verdictFail.why, /last VERDICT line says FAIL/);
  const before = record().replace("RESULT: DONE", "VERDICT: FAIL - no caller\nRESULT: DONE");
  assert.match(planLanding({ ...base, record: before }).why, /last VERDICT line says FAIL/, "the judge order: verdict, then DONE");
  assert.match(planLanding({ ...base, record: record("\nVERDICT: BLOCKED - no GPU") }).why, /says BLOCKED/);
  assert.equal(planLanding({ ...base, record: record("\nVERDICT: FAIL - first\nVERDICT: PASS - fixed") }).ok, true, "the last VERDICT line wins");
  assert.equal(planLanding({ ...base, record: record("\nthe verdict: fail case is covered by a test") }).ok, true, "prose is no verdict line");
  const textFail = planLanding({ ...base, record: record().replace("torches cast shadows", "cut-shadow-sun-depth FAIL") });
  assert.equal(textFail.ok, false);
  assert.match(textFail.why, /RESULT text says FAIL after DONE/);
  assert.equal(planLanding({ ...base, record: record().replace("3 passed", "3 passed, 0 FAIL") }).ok, true, "the proof may quote a check count");
});

test("a DONE whose proof is a book cite, empty or a placeholder is not landed", () => {
  const cite = planLanding({ ...base, record: record().replace(/proof: .*/, "proof: geaen | 15.1 Components") });
  assert.equal(cite.ok, false);
  assert.match(cite.why, /RESULT proof .*no command or file result/);
  // miner seats add a `| paths:` cell after the proof; its file name is not the proof
  const miner = record().replace(/proof: .*/, "proof: geaen | 11.3 Poses | paths: target/queue/ready/0-lf17-book-death-pose.md");
  assert.equal(resultOf(miner).proof, "geaen | 11.3 Poses");
  assert.match(planLanding({ ...base, record: miner }).why, /RESULT proof names no command or file result/);
  assert.match(planLanding({ ...base, record: record().replace(/ \| proof: .*/, "") }).why, /RESULT proof is empty/);
  assert.match(planLanding({ ...base, record: record().replace(/proof: .*/, "proof: <where>") }).why, /RESULT proof/);
  assert.equal(planLanding({ ...base, record: record().replace(/proof: .*/, "proof: node --test tools/land.test.mjs -> 9 pass") }).ok, true);
});

// ---- a landing is recorded; a closed row is ticked only on its Accept command ----
const ROW = [
  "- [ ] EG12 render: torch shadows in the forward shader",
  "  Diff: good. Tier A. Size M. Accept: `cargo test -p engine-render --lib torch`",
  "  and a capture. F2P: the torch test fails before. P2P: `cargo test -p engine-render --lib`.",
  "  needs: none.",
];
const BOARD = (rows = ROW) => ["# SPRINT", "### Finish line", "- [x] A1 core: done.", ...rows, "- [ ] EG13 render: next row, needs: EG12.", ""].join("\n");
const PROOF = "cargo test -p engine-render --lib torch 3 passed";

test("acceptCommands reads the backticked commands of the Accept clause only", () => {
  assert.deepEqual(acceptCommands(ROW.join(" ")), ["cargo test -p engine-render --lib torch"], "the P2P command is not the Accept command");
  assert.deepEqual(acceptCommands("- [ ] X1 a: Accept: `! rg -n gone crates/a.rs` and `crates/a/src/b.rs` exist, `node tools/c.mjs --x`. F2P: `cargo test -p a`."), ["! rg -n gone crates/a.rs", "node tools/c.mjs --x"], "paths are no commands");
  assert.deepEqual(acceptCommands("- [ ] X1 a: Accept: the table matches. F2P: `cargo test -p a`."), []);
  assert.deepEqual(acceptCommands("- [ ] X1 a: no accept clause `cargo test -p a`"), []);
});

test("planClose: no closes: is nothing to do; a bad id, an unknown, a ticked or a command-less row is told why", () => {
  const at = (closes, proof = PROOF, text = BOARD()) => planClose({ closes, proof, sha: "abc1234", boards: [{ file: "docs/SPRINT-18-gaps.md", text }] });
  assert.equal(at(""), null);
  assert.equal(at(undefined), null);
  assert.match(at("EG12, EG13").why, /not one row id/);
  assert.match(at("eg12").why, /not one row id/);
  assert.match(at("ZZ9").why, /not a row on the board/);
  assert.match(at("A1").why, /already ticked/);
  assert.match(at("EG13").why, /no Accept command/);
});

test("planClose ticks the row on its Accept command: first line gets Landed <sha7>, nothing else moves", () => {
  const board = BOARD();
  const ok = planClose({ closes: "EG12", proof: PROOF, sha: "abc1234", boards: [{ file: "docs/SPRINT-18-gaps.md", text: board }] });
  assert.equal(ok.ok, true);
  assert.equal(ok.file, "docs/SPRINT-18-gaps.md");
  const was = board.split("\n");
  const now = ok.text.split("\n");
  assert.equal(now.length, was.length);
  assert.equal(now[3], "- [x] EG12 render: torch shadows in the forward shader Landed abc1234.");
  assert.deepEqual(now.filter((l, i) => l !== was[i]), [now[3]], "one line changed");
  // equals or contains, whitespace aside
  assert.equal(planClose({ closes: "EG12", proof: "cargo  test -p engine-render --lib torch", sha: "abc1234", boards: [{ file: "f", text: board }] }).ok, true, "equals");
  const no = planClose({ closes: "EG12", proof: "cargo test -p engine-render --lib 9 passed", sha: "abc1234", boards: [{ file: "f", text: board }] });
  assert.equal(no.ok, false);
  assert.match(no.why, /does not hold EG12's Accept command: cargo test -p engine-render --lib torch/);
  assert.equal(planClose({ closes: "EG12", proof: "", sha: "abc1234", boards: [{ file: "f", text: board }] }).ok, false);
  // every Accept command must be in the proof
  const two = BOARD(["- [ ] EG12 render: two. Accept: `cargo test -p a --lib x` and `node tools/c.mjs`. F2P: n.", "  needs: none."]);
  assert.equal(planClose({ closes: "EG12", proof: "cargo test -p a --lib x ok", sha: "abc1234", boards: [{ file: "f", text: two }] }).ok, false);
  assert.equal(planClose({ closes: "EG12", proof: "cargo test -p a --lib x ok; node tools/c.mjs ok", sha: "abc1234", boards: [{ file: "f", text: two }] }).ok, true);
  // CRLF files keep their line ends; the first board file that holds the id decides
  const crlf = planClose({ closes: "EG12", proof: PROOF, sha: "abc1234", boards: [{ file: "f", text: board.replace(/\n/g, "\r\n") }] });
  assert.equal(crlf.text.split("\r\n")[3], "- [x] EG12 render: torch shadows in the forward shader Landed abc1234.");
  assert.equal(/[^\r]\n/.test(crlf.text), false, "no bare LF appeared");
  const twice = planClose({ closes: "EG12", proof: PROOF, sha: "abc1234", boards: [{ file: "a", text: ok.text }, { file: "b", text: board }] });
  assert.match(twice.why, /already ticked/, "never untick, never tick a later duplicate");
});

test("planClose: a row with no Accept command ticks on the packet's own test: command, and only when the proof holds it (14 of 25 close packets, 2026-10-15)", () => {
  const at = (proof, test) => planClose({ closes: "EG13", proof, sha: "abc1234", test, boards: [{ file: "docs/SPRINT-18-gaps.md", text: BOARD() }] });
  assert.match(at(PROOF, "").why, /no Accept command/, "no test: no tick, as before");
  assert.match(at(PROOF, undefined).why, /no Accept command/);
  const ok = at("cargo test -p engine-render --lib msaa exit 0 x2", "cargo test -p engine-render --lib msaa");
  assert.equal(ok.ok, true);
  assert.deepEqual(ok.accept, ["cargo test -p engine-render --lib msaa"]);
  assert.match(ok.text, /^- \[x\] EG13 render: next row, needs: EG12\. Landed abc1234\.$/m);
  assert.match(at("cargo test -p engine-render --lib 9 passed", "cargo test -p engine-render --lib msaa").why, /does not hold EG13's Accept command: cargo test -p engine-render --lib msaa/);
  assert.equal(planClose({ closes: "EG12", proof: PROOF, sha: "abc1234", test: "cargo test -p other", boards: [{ file: "f", text: BOARD() }] }).ok, true, "a row's own Accept command still wins over the packet's test");
});

test("landedLine is five tab cells with no tab or newline inside a cell", () => {
  const line = landedLine({ at: "2026-10-12T09:00:00Z", row: "EG12", packet: "0-eg12-torch", sha: "abc1234", proof: "cargo test\t-p x\n3 passed" });
  assert.equal(line, "2026-10-12T09:00:00Z\tEG12\t0-eg12-torch\tabc1234\tcargo test -p x 3 passed");
  assert.equal(line.split("\t").length, 5);
  assert.equal(landedLine({ at: "t", row: "", packet: "p", sha: "s", proof: "" }).split("\t")[1], "-");
});

// ---- the workspace audit: a FAIL refuses only on the packet's own files (2026-10-12: one stray file refused every landing) ----
const OWN = ["crates/render/src/shadow.rs"];
const genre = (...files) => `FAIL workspace audit: unlisted genre assumption (GE2 ledger; the ledger never grows, use a neutral name or alias such as net BodySnap): ${files.join(", ")}\n`;
const audit = (stderr, own = OWN, status = 1) => workspaceVerdict({ status, stdout: "presence: 0 live\n", stderr }, own);

test("failFiles reads every line shape workspace-check prints, and nothing else", () => {
  assert.deepEqual(failFiles("unlisted genre assumption (GE2 ledger; the ledger never grows): playtest/src/replay.rs (FighterTape), ai/src/eqs.rs (FighterId, FighterInput)"), ["playtest/src/replay.rs", "ai/src/eqs.rs"]);
  assert.deepEqual(failFiles("platform float math in simulation code (a Fixed call passes only when its receiver is provably Fixed in the file: use a typed binding like `let a: Fixed = ..`): core/src/a.rs (2: L3 f32.sin, L9 f32.cos), ai/src/b.rs (1: L4 f32.exp)"), ["core/src/a.rs", "ai/src/b.rs"]);
  assert.deepEqual(failFiles("engine-playtest: sub\\replay.rs is not declared by any module, so it is never compiled"), ["src/sub\\replay.rs"]);
  assert.deepEqual(failFiles("large non-LFS blob: content/arena/big.glb is 6.00 MB (> 5 MB) — track it via LFS"), ["content/arena/big.glb"]);
  assert.deepEqual(failFiles("test layout: crates/ai/tests/extra.rs is a new test executable — make it a module of crates/ai/tests/it/ (one `mod` line in main.rs), or list ai/extra in OWN_TEST_BINARY with a reason"), ["crates/ai/tests/extra.rs"]);
  assert.deepEqual(failFiles("PF11 append-only gate missing from baselines.rs: mod append_only_tests"), ["baselines.rs"]);
  for (const none of ["engine-net -> engine-endpoint: undocumented local dependency edge", "board: EG12 needs unknown task ZZ9", "FAIL cycle: a -> b -> a", "research: row EG1 has 9 cards (cap 4) and is not done", "dependency cycle check failed: Command failed: node tools/arch-fitness.mjs", "Command failed: cargo metadata --locked at crates/x/Cargo.toml", ""]) {
    assert.deepEqual(failFiles(none), [], none);
  }
});

test("workspaceVerdict: a FAIL on another writer's file warns, a FAIL on the packet's own file or no file refuses", () => {
  assert.deepEqual(workspaceVerdict({ status: 0, stdout: "PASS workspace\n", stderr: "" }, OWN), { refusal: null, warns: [] });
  // the 2026-10-12 stall: replay.rs (another writer's, unlanded) refused every packet
  assert.deepEqual(audit(genre("playtest/src/replay.rs (FighterTape)")), { refusal: null, warns: ["playtest/src/replay.rs"] });
  assert.deepEqual(audit(`(node:1) ExperimentalWarning: noise\n${genre("playtest/src/replay.rs (FighterTape)")}`).warns, ["playtest/src/replay.rs"], "noise before the FAIL marker is not a line");
  // the packet's own file, by its crates/-relative name, by a bare name, or by a src-relative undeclared module
  assert.match(audit(genre("render/src/shadow.rs (FighterTape)")).refusal, /^workspace-check FAIL on this packet's own render\/src\/shadow\.rs \(/);
  assert.match(audit(genre("playtest/src/replay.rs (X), render/src/shadow.rs (Y)")).refusal, /own render\/src\/shadow\.rs/, "one packet file among others refuses");
  assert.match(audit("FAIL workspace audit: engine-render: shadow.rs is not declared by any module, so it is never compiled\n").refusal, /own src\/shadow\.rs/);
  assert.match(audit("FAIL workspace audit: PF11 append-only gate missing from baselines.rs: fn check_exact\n", ["crates/perf/src/baselines.rs"]).refusal, /own baselines\.rs/);
  assert.match(audit("FAIL workspace audit: test layout: crates/ai/tests/extra.rs is a new test executable — x\n", ["crates/ai/tests/extra.rs"]).refusal, /own crates\/ai\/tests\/extra\.rs/);
  // a path tail must be a whole segment: shadow.rs is not hadow.rs
  assert.equal(audit(genre("render/src/hadow.rs (X)")).refusal, null);
  // no file, one unnamed line among named ones, an empty stderr, a killed run: refuse as before
  assert.match(audit("FAIL workspace audit: engine-net -> engine-endpoint: undocumented local dependency edge\n").refusal, /^workspace-check FAIL \(.*undocumented local dependency edge\)$/);
  assert.match(audit(genre("playtest/src/replay.rs (X)").trimEnd() + "\nboard: EG12 needs unknown task ZZ9\n").refusal, /needs unknown task/);
  assert.match(audit("").refusal, /see node tools\/workspace-check\.mjs/);
  assert.match(audit("", OWN, null).refusal, /^workspace-check FAIL/);
  assert.match(audit("Error: something else\n").refusal, /^workspace-check FAIL/, "no FAIL marker, no files");
  assert.match(workspaceVerdict({ error: new Error("spawn ETIMEDOUT") }, OWN).refusal, /^workspace-check could not run \(spawn ETIMEDOUT/);
  assert.match(workspaceVerdict(undefined, OWN).refusal, /^workspace-check FAIL/, "no audit result is no pass");
});

// The tool run end to end in a throw-away git repo (master, no remote: the push fails and stays local).
const TOOLS = dirname(fileURLToPath(import.meta.url));
const sh = (cwd, ...args) => execFileSync("git", args, { cwd, encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] });
// auditOut: a stand-in tools/workspace-check.mjs that prints this to stderr and exits `auditCode` (null = the harness has none).
const landRepo = ({ closes = "EG12", result = `RESULT: DONE - torches cast shadows | proof: ${PROOF}`, board = BOARD(), auditOut = null, auditCode = 1, change = true, review = "RUST: 4/5 DEPTH: 3/5\nVERDICT: PASS - ok\n" } = {}) => {
  const root = mkdtempSync(join(tmpdir(), "land-e2e-"));
  mkdirSync(join(root, "tools"));
  mkdirSync(join(root, ".opencode"));
  for (const f of ["land.mjs", "queue-cap.mjs", "argv.mjs", "board-slice.mjs"]) copyFileSync(join(TOOLS, f), join(root, "tools", f));
  if (auditOut !== null) writeFileSync(join(root, "tools", "workspace-check.mjs"), `process.stderr.write(${JSON.stringify(auditOut)});\nprocess.exitCode = ${auditCode};\n`);
  copyFileSync(join(TOOLS, "..", ".opencode", "seat-guard.mjs"), join(root, ".opencode", "seat-guard.mjs"));
  mkdirSync(join(root, "crates", "render", "src"), { recursive: true });
  mkdirSync(join(root, "docs"));
  mkdirSync(join(root, "target", "queue", "done"), { recursive: true });
  writeFileSync(join(root, ".gitignore"), "target/\n");
  writeFileSync(join(root, "crates", "render", "src", "shadow.rs"), "fn a() {}\n");
  writeFileSync(join(root, "docs", "SPRINT-18-gaps.md"), board);
  sh(root, "init", "-q", "-b", "master");
  sh(root, "config", "user.name", "t");
  sh(root, "config", "user.email", "t@example.com");
  sh(root, "add", "-A");
  sh(root, "commit", "-q", "-m", "init");
  const fm = ["role: builder", "title: EG12 torch shadows", "row: EG12", ...(closes ? [`closes: ${closes}`] : []), "paths: crates/render/src/shadow.rs"].join("\n");
  writeFileSync(join(root, "target", "queue", "done", "0-eg12-torch.md"), `---\n${fm}\n---\nGoal: g\n\n## Result (completed)\n\n${result}\n`);
  if (review) writeFileSync(join(root, "target", "queue", "done", "0-eg12-torch-review.md"), review);
  if (change) writeFileSync(join(root, "crates", "render", "src", "shadow.rs"), "fn a() { /* torch */ }\n");
  return root;
};
const land = (root, ...extra) => spawnSync(process.execPath, [join(root, "tools", "land.mjs"), "0-eg12-torch", ...extra], { encoding: "utf8", cwd: root });
const subjects = (root) => sh(root, "log", "--format=%s").trim().split(/\r?\n/);

test("land: a PASS records itself, ticks the closed row on its Accept command and commits the board edit by path", () => {
  const root = landRepo();
  try {
    const r = land(root);
    assert.match(r.stdout, /^LANDED 0-eg12-torch: \w+ crates\/render\/src\/shadow\.rs \| board: ticked EG12 in docs\/SPRINT-18-gaps\.md/, r.stdout + r.stderr);
    const code = sh(root, "log", "--skip=1", "-1", "--format=%h").trim();
    assert.equal(subjects(root).length, 3, "init, the code commit, the board commit");
    assert.match(subjects(root)[0], /^EG12: /);
    assert.deepEqual(sh(root, "show", "--name-only", "--format=", "HEAD").trim().split(/\r?\n/), ["docs/SPRINT-18-gaps.md"]);
    assert.deepEqual(sh(root, "show", "--name-only", "--format=", "HEAD~1").trim().split(/\r?\n/), ["crates/render/src/shadow.rs"]);
    const board = readFileSync(join(root, "docs", "SPRINT-18-gaps.md"), "utf8");
    assert.match(board, new RegExp(`^- \\[x\\] EG12 render: torch shadows in the forward shader Landed ${code.slice(0, 7)}\\.$`, "m"));
    assert.match(board, /^- \[ \] EG13 /m, "no other row moves");
    assert.equal(sh(root, "status", "--porcelain", "--", "docs").trim(), "", "the board edit is committed");
    const tsv = readFileSync(join(root, "target", "queue", "landed.tsv"), "utf8").trim().split(/\r?\n/);
    assert.equal(tsv[0], "utc\trow\tpacket\tsha\tproof");
    const cells = tsv[1].split("\t");
    assert.match(cells[0], /^\d{4}-\d\d-\d\dT\d\d:\d\d:\d\dZ$/);
    assert.deepEqual(cells.slice(1), ["EG12", "0-eg12-torch", code, PROOF]);
    assert.match(land(root).stdout, /^NOOP /, "a second run is a no-op and adds no landed line");
    assert.equal(readFileSync(join(root, "target", "queue", "landed.tsv"), "utf8").trim().split(/\r?\n/).length, 2);
  } finally { rmSync(root, { recursive: true, force: true }); }
});

test("land: no closes:, or a proof without the Accept command, records the landing and ticks nothing", () => {
  for (const [opt, why] of [[{ closes: "" }, null], [{ result: "RESULT: DONE - torches cast shadows | proof: cargo test -p engine-render --lib 9 passed" }, /does not hold EG12's Accept command/]]) {
    const root = landRepo(opt);
    try {
      const r = land(root);
      assert.match(r.stdout, /^LANDED /, r.stdout + r.stderr);
      if (why) assert.match(r.stdout, why);
      assert.equal(subjects(root).length, 2, "init and the code commit only");
      assert.match(readFileSync(join(root, "docs", "SPRINT-18-gaps.md"), "utf8"), /^- \[ \] EG12 /m, "never ticked");
      assert.equal(readFileSync(join(root, "target", "queue", "landed.tsv"), "utf8").trim().split(/\r?\n/).length, 2, "header and the landing");
    } finally { rmSync(root, { recursive: true, force: true }); }
  }
});

test("land: a close-only packet (NOOP or DONE, code already in the tree, no review) ticks its row on its Accept command and commits only the board edit (36 of 36 close packets ended NOOP with the row open, 2026-10-15)", () => {
  const root = landRepo({ result: `RESULT: NOOP - already in the tree | proof: ${PROOF}`, change: false, review: "" });
  try {
    const r = land(root);
    assert.match(r.stdout, /^NOOP 0-eg12-torch: nothing to land: no changes under crates\/render\/src\/shadow\.rs .*\| board: ticked EG12 in docs\/SPRINT-18-gaps\.md/, r.stdout + r.stderr);
    assert.equal(subjects(root).length, 2, "init and the board tick only");
    assert.match(subjects(root)[0], /^EG12: board tick, code already on master at \w+/);
    assert.deepEqual(sh(root, "show", "--name-only", "--format=", "HEAD").trim().split(/\r?\n/), ["docs/SPRINT-18-gaps.md"]);
    assert.match(readFileSync(join(root, "docs", "SPRINT-18-gaps.md"), "utf8"), /^- \[x\] EG12 .* Landed \w{7}\.$/m);
    assert.match(readFileSync(join(root, "docs", "SPRINT-18-gaps.md"), "utf8"), /^- \[ \] EG13 /m, "no other row moves");
    assert.match(land(root).stdout, /^NOOP .*\| board: not ticked: EG12 is already ticked/, "a second run ticks nothing more");
    assert.equal(subjects(root).length, 2);
    assert.equal(existsSync(join(root, "target", "queue", "landed.tsv")), false, "a tick is no landing: no ledger line");
  } finally { rmSync(root, { recursive: true, force: true }); }
});

test("land: close-only never ticks without the Accept command in the proof, on a PARTIAL, on a FAIL review, or with no closes:", () => {
  const cases = [
    [{ result: "RESULT: NOOP - already in the tree | proof: cargo test -p engine-render --lib 9 passed" }, /not ticked: the proof does not hold EG12/],
    [{ result: `RESULT: PARTIAL - half | proof: ${PROOF}` }, /^NOOP .*nothing to land|REFUSED .*no review/],
    [{ review: "RUST: 4/5 DEPTH: 3/5\nVERDICT: FAIL - the test is fake\n" }, /REFUSED|says FAIL/],
    [{ closes: "" }, /^NOOP .*nothing to land(?!.*board:)|REFUSED .*no review/],
  ];
  for (const [opt, want] of cases) {
    const root = landRepo({ result: `RESULT: NOOP - already in the tree | proof: ${PROOF}`, change: false, review: "", ...opt });
    try {
      const r = land(root);
      assert.match(r.stdout, want, `${JSON.stringify(opt).slice(0, 60)} -> ${r.stdout}${r.stderr}`);
      assert.match(readFileSync(join(root, "docs", "SPRINT-18-gaps.md"), "utf8"), /^- \[ \] EG12 /m, "the row stays open");
      assert.equal(subjects(root).length, 1, "no commit");
    } finally { rmSync(root, { recursive: true, force: true }); }
  }
});

test("land: a PARTIAL lands its paths with the kind in the commit and leaves its row open, even when the proof holds the Accept command", () => {
  const root = landRepo({ result: `RESULT: PARTIAL - shadow pass only | proof: ${PROOF}` });
  try {
    const r = land(root);
    assert.match(r.stdout, /^LANDED 0-eg12-torch: \w+ crates\/render\/src\/shadow\.rs \| pushed: /, r.stdout + r.stderr);
    assert.doesNotMatch(r.stdout, /board:/);
    assert.equal(subjects(root).length, 2, "init and the code commit only");
    assert.match(sh(root, "log", "-1", "--format=%B"), /\nResult PARTIAL: shadow pass only \| proof: cargo test/);
    assert.match(readFileSync(join(root, "docs", "SPRINT-18-gaps.md"), "utf8"), /^- \[ \] EG12 /m, "never ticked");
    assert.match(land(root).stdout, /^NOOP /, "a second run is a no-op");
  } finally { rmSync(root, { recursive: true, force: true }); }
});

test("land: a board file with other uncommitted edits is ticked in the tree but not committed", () => {
  const root = landRepo();
  try {
    writeFileSync(join(root, "docs", "SPRINT-18-gaps.md"), `${BOARD()}- [ ] ZZ1 note: another session's edit.\n`);
    const r = land(root);
    assert.match(r.stdout, /board: ticked EG12 .*not committed/, r.stdout + r.stderr);
    assert.equal(subjects(root).length, 2);
    const board = readFileSync(join(root, "docs", "SPRINT-18-gaps.md"), "utf8");
    assert.match(board, /^- \[x\] EG12 .* Landed \w{7}\.$/m);
    assert.match(board, /^- \[ \] ZZ1 note/m, "the other edit stays");
    assert.equal(existsSync(join(root, "target", "queue", "landed.tsv")), true);
  } finally { rmSync(root, { recursive: true, force: true }); }
});

test("land: a book-cite proof or a FAIL in the RESULT text commits nothing", () => {
  for (const result of ["RESULT: DONE - torches cast shadows | proof: geaen | 15.1 Components", "RESULT: DONE - torches FAIL to cast shadows | proof: cargo test -p engine-render --lib torch 3 passed"]) {
    const root = landRepo({ result });
    try {
      const r = land(root);
      assert.match(r.stdout, /^REFUSED 0-eg12-torch: /, r.stdout + r.stderr);
      assert.equal(r.status, 1);
      assert.equal(subjects(root).length, 1);
      assert.equal(existsSync(join(root, "target", "queue", "landed.tsv")), false);
    } finally { rmSync(root, { recursive: true, force: true }); }
  }
});

test("land: a workspace-check FAIL on another writer's file warns and lands; on the packet's file, or naming no file, it refuses", () => {
  const outside = landRepo({ closes: "", auditOut: genre("playtest/src/replay.rs (FighterTape)") });
  try {
    const r = land(outside);
    assert.equal(r.status, 0, r.stdout + r.stderr);
    assert.match(r.stdout, /^WARN 0-eg12-torch: workspace-check FAIL outside this packet: playtest\/src\/replay\.rs; landing anyway\nLANDED 0-eg12-torch: \w+ crates\/render\/src\/shadow\.rs \| pushed: .* \| warn: workspace-check FAIL outside this packet\n$/, r.stdout + r.stderr);
    assert.equal(subjects(outside).length, 2, "init and the code commit");
  } finally { rmSync(outside, { recursive: true, force: true }); }
  for (const [auditOut, auditCode, why] of [
    [genre("playtest/src/replay.rs (X), render/src/shadow.rs (FighterTape)"), 1, /workspace-check FAIL on this packet's own render\/src\/shadow\.rs/],
    ["FAIL workspace audit: engine-net -> engine-endpoint: undocumented local dependency edge\n", 1, /workspace-check FAIL \(.*undocumented local dependency edge\)/],
    ["", 1, /workspace-check FAIL \(see node tools\/workspace-check\.mjs\)/],
  ]) {
    const root = landRepo({ closes: "", auditOut, auditCode });
    try {
      const r = land(root);
      assert.match(r.stdout, new RegExp(`^REFUSED 0-eg12-torch: ${why.source}`), r.stdout + r.stderr);
      assert.equal(r.status, 1);
      assert.equal(subjects(root).length, 1, "nothing committed");
      assert.equal(existsSync(join(root, "target", "queue", "landed.tsv")), false);
    } finally { rmSync(root, { recursive: true, force: true }); }
  }
  const green = landRepo({ closes: "", auditOut: "", auditCode: 0 });
  try {
    const r = land(green);
    assert.match(r.stdout, /^LANDED 0-eg12-torch: \w+ crates\/render\/src\/shadow\.rs \| pushed: [^\n]*\n$/, "a green audit lands with no WARN: " + r.stdout + r.stderr);
    assert.doesNotMatch(r.stdout, /WARN|warn:/);
  } finally { rmSync(green, { recursive: true, force: true }); }
});

test("land: a staged git-rm deletion lands by path instead of refusing on the pathspec", () => {
  const root = landRepo({ closes: "", auditOut: null });
  try {
    sh(root, "rm", "-f", "--", "crates/render/src/shadow.rs");
    assert.equal(existsSync(join(root, "crates", "render", "src", "shadow.rs")), false, "worktree gone after git rm");
    const r = land(root);
    assert.match(r.stdout, /^LANDED 0-eg12-torch: \w+ crates\/render\/src\/shadow\.rs/, `staged deletion lands by path: ${r.stdout}${r.stderr}`);
    assert.equal(r.status, 0);
    assert.equal(subjects(root).length, 2, "init and the deletion commit");
    assert.deepEqual(sh(root, "show", "--name-only", "--format=", "HEAD").trim().split(/\r?\n/), ["crates/render/src/shadow.rs"]);
    assert.match(land(root).stdout, /^NOOP /, "a second run is a no-op");
  } finally { rmSync(root, { recursive: true, force: true }); }
});

// ---- the rust ratchet scoped to the landing packet's owned paths: foreign debt warns, owned refuses ----
test("ratchetVerdict: clean lands, foreign debt warns, owned debt refuses", () => {
  assert.deepEqual(ratchetVerdict({ status: 0, stdout: "RESULT PASS: score 90 (A), 0 ratchet regressions", stderr: "" }, OWN), { refusal: null, warns: [] });
  const foreign = ratchetVerdict({ status: 0, stdout: "[WARN] foreign ratchet engine-ai: casts 5 -> 6\nRESULT PASS: score 90 (A)", stderr: "" }, OWN);
  assert.equal(foreign.refusal, null);
  assert.deepEqual(foreign.warns, ["engine-ai: casts 5 -> 6"]);
  const owned = ratchetVerdict({ status: 1, stdout: "[FAIL] ratchet engine-render: unwrap 0 -> 1\nRESULT FAIL: 1 ratchet regression(s)", stderr: "" }, OWN);
  assert.match(owned.refusal, /ratchet FAIL on owned crates\/render\/src\/shadow\.rs/);
  assert.deepEqual(ratchetArgs(OWN), ["--owned", "crates/render/src/shadow.rs"]);
  assert.deepEqual(ratchetArgs(["crates/a.rs", "crates/b.rs"]), ["--owned", "crates/a.rs", "--owned", "crates/b.rs"]);
});

test("land: ratchet --owned lands owned-clean and foreign debt with a warning, refuses owned debt", () => {
  // The stand-in refuses without the owned pass-through, so a landing proves land passed --owned <paths>.
  const stub = (body) => `const args = process.argv.slice(2);\nif (!args.includes("--owned") || !args.join(" ").includes("crates/render/src/shadow.rs")) { console.log("RESULT FAIL: unknown argument"); process.exitCode = 2; } else { ${body} }\n`;
  const cases = [
    { name: "clean", body: `console.log("RESULT PASS: score 90 (A), 0 ratchet regressions");`, code: 0, landed: true, warn: false },
    { name: "foreign", body: `console.log("[WARN] foreign ratchet engine-ai: casts 5 -> 6");\nconsole.log("RESULT PASS: score 90 (A)");`, code: 0, landed: true, warn: true },
    { name: "owned", body: `console.log("[FAIL] ratchet engine-render: unwrap 0 -> 1");\nconsole.log("RESULT FAIL: 1 ratchet regression(s)"); process.exitCode = 1;`, code: 1, landed: false, warn: false },
  ];
  for (const c of cases) {
    const root = landRepo({ closes: "", auditOut: null });
    try {
      writeFileSync(join(root, "tools", "rust-quality.mjs"), stub(c.body));
      const r = land(root);
      if (c.landed) {
        assert.equal(r.status, 0, `${c.name}: ${r.stdout}${r.stderr}`);
        assert.match(r.stdout, /LANDED 0-eg12-torch: \w+ crates\/render\/src\/shadow\.rs/, `${c.name}: ${r.stdout}${r.stderr}`);
        if (c.warn) assert.match(r.stdout, /ratchet foreign/, `${c.name} warns: ${r.stdout}${r.stderr}`);
        else assert.doesNotMatch(r.stdout, /ratchet foreign/, `${c.name} stays quiet: ${r.stdout}${r.stderr}`);
        assert.equal(subjects(root).length, 2, `${c.name}: init and the code commit`);
      } else {
        assert.equal(r.status, 1, `${c.name}: ${r.stdout}${r.stderr}`);
        assert.match(r.stdout, /^REFUSED 0-eg12-torch: ratchet FAIL on owned crates\/render\/src\/shadow\.rs/, `${c.name}: ${r.stdout}${r.stderr}`);
        assert.equal(subjects(root).length, 1, `${c.name}: nothing committed`);
        assert.equal(existsSync(join(root, "target", "queue", "landed.tsv")), false, `${c.name}: no ledger line`);
      }
    } finally { rmSync(root, { recursive: true, force: true }); }
  }
});
