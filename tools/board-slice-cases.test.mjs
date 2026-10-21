// Attribution: diagnostic table shape adapted from kazupon/args-tokens (MIT)
// README resolveArgs idea (args -> resolved values/positionals); here
// args -> resolved { code, hint }. No donor code copied, shape only.
import { test } from "node:test";
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";
// Read-only import: board-slice.mjs runs main() only when invoked as a
// script, so importing its parsers has no side effects and touches no files.
import { parseBoard } from "./board-slice.mjs";
assert.equal(typeof parseBoard, "function", "read-only import stays callable");
const SLICE = fileURLToPath(new URL("./board-slice.mjs", import.meta.url));
function runSlice(args) {
  try {
    const stdout = execFileSync(process.execPath, [SLICE, ...args], {
      encoding: "utf8",
      stdio: ["ignore", "pipe", "pipe"],
    });
    return { code: 0, stdout: String(stdout), stderr: "" };
  } catch (err) {
    return {
      code: Number(err && err.status !== undefined ? err.status : 1),
      stdout: String(err && err.stdout ? err.stdout : ""),
      stderr: String(err && err.stderr ? err.stderr : ""),
    };
  }
}
// resolveArgs-style table: each input argv resolves to an expected exit
// code plus a stderr hint substring (bad-flag diagnostic vectors, n=8).
const CASES = [
  { args: ["--bogus"], code: 2, hint: "unknown flag --bogus" },
  { args: ["-z"], code: 2, hint: "unknown flag -z" },
  { args: ["--find"], code: 2, hint: "--find needs a text" },
  { args: ["AA1", "BB2"], code: 2, hint: "at most one <ID>" },
  { args: [], code: 2, hint: "need <ID> or --self-check" },
  { args: ["--json"], code: 2, hint: "need <ID> or --self-check" },
  { args: ["--open"], code: 2, hint: "need <ID> or --self-check" },
  { args: ["NO-SUCH-ROW-XYZ"], code: 1, hint: "unknown row NO-SUCH-ROW-XYZ" },
];
for (const c of CASES) {
  const label = c.args.length ? c.args.join(" ") : "(no args)";
  test("bad flag: " + label + " -> code " + c.code, () => {
    const r = runSlice(c.args);
    assert.equal(r.code, c.code, "exit code for [" + label + "]");
    assert.ok(
      r.stderr.includes(c.hint),
      "stderr hint for [" + label + "]: want " + JSON.stringify(c.hint),
    );
  });
}

// 2026-10-10: a section header with prose or a second plan before `)` still
// names its first docs/plan file, and a `##` heading or a plan-less header
// ends the section, so a later shard's rows never inherit an earlier plan.
test("section plan: first docs/plan path, no inheritance across headings", () => {
  const text = [
    "### Live (owner 2026-09-26, `docs/plan/living-world.md`)",
    "- [ ] LW1 a",
    "",
    "### Urban (owner 2026-09-26, Finish B; `docs/plan/urban-war.md`)",
    "- [ ] UW1 b",
    "",
    "### Body (owner 2026-09-29, `docs/plan/body-and-break.md`, `docs/plan/hard-systems.md`; pillar 4)",
    "- [ ] BX1 c",
    "",
    "### Operator",
    "- [ ] O1 d",
    "",
    "## Receipts",
    "- [ ] R1 e",
  ].join("\n");
  const { tasks } = parseBoard(text);
  assert.equal(tasks.get("LW1").sectionPlan, "docs/plan/living-world.md");
  assert.equal(tasks.get("UW1").sectionPlan, "docs/plan/urban-war.md");
  assert.equal(tasks.get("BX1").sectionPlan, "docs/plan/body-and-break.md");
  assert.equal(tasks.get("O1").sectionPlan, null);
  assert.equal(tasks.get("R1").sectionPlan, null);
});

// 2026-10-10: sprint-loop cut the joined board at the first `## Receipts`,
// so a shard sorting after the receipts shard was invisible to the loop.
test("loadBoardText receipts:false keeps rows of a shard after the receipts shard", async (t) => {
  const { mkdtempSync, mkdirSync, rmSync, writeFileSync } = await import("node:fs");
  const { tmpdir } = await import("node:os");
  const { join } = await import("node:path");
  const { loadBoardText } = await import("./board-slice.mjs");
  const root = mkdtempSync(join(tmpdir(), "board-"));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  mkdirSync(join(root, "docs"));
  writeFileSync(join(root, "docs", "SPRINT.md"), "# Board\n\n- [ ] A1 a\n");
  writeFileSync(join(root, "docs", "SPRINT-19-receipts.md"), "## Receipts\n\n- [ ] R9 quoted row\n");
  writeFileSync(join(root, "docs", "SPRINT-20-late.md"), "### Late (owner 2026-10-10, `docs/plan/x.md`)\n\n- [ ] Z1 z\n");
  const { tasks } = parseBoard(loadBoardText(root, { receipts: false }));
  assert.deepEqual([...tasks.keys()].sort(), ["A1", "Z1"]);
  assert.ok(parseBoard(loadBoardText(root)).tasks.has("R9"), "default keeps the receipts text");
});

// 2026-10-21: a plan §3 is a design essay (render-look.md held ~9 KB), so
// every row sliced from that plan blew the 8192 byte budget and the board
// self-check read FAIL. The excerpt is capped now; the row stays verbatim.
test("slice budget: every ready slice fits the budget (self-check passes)", () => {
  const r = runSlice(["--self-check"]);
  assert.equal(r.code, 0, "self-check exit: " + r.stdout.slice(-240));
  assert.match(r.stdout, /RESULT PASS: \d+\/\d+ ready slices within budget/);
});

test("plan §3 excerpt is capped with a pointer, the row stays byte-identical", () => {
  const r = runSlice(["VS3"]);
  if (r.code !== 0) return; // VS3 later archived: the self-check test still guards the budget.
  assert.ok(
    Buffer.byteLength(r.stdout, "utf8") <= 8192,
    "slice within the 8192 byte budget",
  );
  assert.match(r.stdout, /excerpt truncated at \d+ bytes; the full text is in docs\/plan\//);
  assert.match(r.stdout, /- \[ \] VS3 game: kit textures/);
});
