// Run: node --test tools/tools-index.test.mjs
// Unit tests for the inventory parser and the exception rule, plus one live
// case: the real tools/ tree must pass the gate once tools/README.md names it.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { audit, listLiveTools, listTestFiles, mateOf, parseReadme } from "./tools-index.mjs";

const TOOLS = dirname(fileURLToPath(import.meta.url));
const README = join(TOOLS, "README.md");

const FIXTURE = [
  "# tools/",
  "Prose names `*.test.mjs`, the placeholder `tools/<name>.mjs` and `docs/book/ref-tools.md`.",
  "## Live tools",
  "- `tools/alpha.mjs` (+ `tools/alpha.test.mjs`): first.",
  "- `tools/beta.mjs`: second.",
  "## Test-only mates",
  "- `tools/gamma.test.mjs`",
].join("\n");

test("parseReadme keeps tool names and ignores prose and placeholders", () => {
  const listed = parseReadme(FIXTURE);
  assert.deepEqual(listed.live, ["alpha.mjs", "beta.mjs"]);
  assert.deepEqual(listed.tests, ["alpha.test.mjs", "gamma.test.mjs"]);
});

test("parseReadme names the mate of a live tool", () => {
  assert.equal(mateOf("alpha.mjs"), "alpha.test.mjs");
});

test("a live tool with no README line is a gap", () => {
  const result = audit({
    live: ["alpha.mjs", "beta.mjs", "delta.mjs"],
    tests: ["alpha.test.mjs", "beta.test.mjs"],
    readme: FIXTURE,
    exceptions: {},
  });
  assert.deepEqual(result.missingFromReadme, ["delta.mjs"]);
  assert.deepEqual(result.unexplainedNoTestMate, ["delta.mjs"]);
  assert.equal(result.ok, false);
});

test("a README entry whose file is gone is a gap", () => {
  const readme = `${FIXTURE}\n- \`tools/gone.mjs\`: retired.\n`;
  const result = audit({ live: ["alpha.mjs", "beta.mjs"], tests: ["alpha.test.mjs", "beta.test.mjs"], readme, exceptions: {} });
  assert.deepEqual(result.staleReadme, ["gone.mjs"]);
  assert.equal(result.ok, false);
});

test("an exception explains a missing mate, and only that tool", () => {
  const result = audit({
    live: ["alpha.mjs", "beta.mjs"],
    tests: ["alpha.test.mjs"],
    readme: FIXTURE,
    exceptions: { "beta.mjs": "needs a GPU; no headless proof" },
  });
  assert.deepEqual(result.noTestMate, ["beta.mjs"]);
  assert.deepEqual(result.explainedNoTestMate, ["beta.mjs"]);
  assert.deepEqual(result.unexplainedNoTestMate, []);
  assert.deepEqual(result.staleExceptions, []);
  assert.equal(result.ok, true);
});

test("an exception for a tool that has a mate is a stale gap", () => {
  const result = audit({
    live: ["alpha.mjs"],
    tests: ["alpha.test.mjs"],
    readme: FIXTURE,
    exceptions: { "alpha.mjs": "a mate was written later" },
  });
  assert.deepEqual(result.staleExceptions, ["alpha.mjs"]);
  assert.equal(result.ok, false);
});

test("an exception for a tool that is gone is a stale gap", () => {
  const result = audit({
    live: ["alpha.mjs"],
    tests: ["alpha.test.mjs"],
    readme: FIXTURE,
    exceptions: { "retired.mjs": "was a manual tool" },
  });
  assert.deepEqual(result.staleExceptions, ["retired.mjs"]);
  assert.equal(result.ok, false);
});

test("the live tree passes the inventory rule", () => {
  const result = audit({
    live: listLiveTools(TOOLS),
    tests: listTestFiles(TOOLS),
    readme: readFileSync(README, "utf8"),
  });
  console.log(
    `live tree: ${result.live.length} live, ${result.readmeLive.length} named, ` +
      `${result.noTestMate.length} without a mate (${result.explainedNoTestMate.length} explained)`,
  );
  assert.ok(result.live.length > 100, `expected a populated tools/ tree, got ${result.live.length}`);
  assert.ok(result.live.includes("tools-index.mjs"), "the gate names itself");
  assert.deepEqual(result.missingFromReadme, [], `no README line for:\n${result.missingFromReadme.join("\n")}`);
  assert.deepEqual(result.staleReadme, [], `README names a gone tool:\n${result.staleReadme.join("\n")}`);
  assert.deepEqual(result.unexplainedNoTestMate, [], `no mate and no exception:\n${result.unexplainedNoTestMate.join("\n")}`);
  assert.deepEqual(result.staleExceptions, [], `stale exception:\n${result.staleExceptions.join("\n")}`);
  assert.equal(result.ok, true);
});
