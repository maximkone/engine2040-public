// Run: node --test tools/compile-cache.test.mjs
// Pins the warm-cache compile plan output (idea shaped by mozilla/sccache,
// Apache-2.0; code written fresh, no donor bytes).
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import test from "node:test";
import { PLAN_LINES, planText } from "./compile-cache.mjs";

const TOOL = fileURLToPath(new URL("./compile-cache.mjs", import.meta.url));

const EXPECTED = [
  "compile-cache plan (sccache idea, Apache-2.0):",
  "1. record cold check time on a clean tree",
  "2. enable the cache for one session only",
  "3. run the same check twice to warm, then read hit rate",
  "4. record warm check time with a full cache",
  "5. drop the cache and repeat once to confirm the gap",
  "done-when: cold 300s -> warm 70s, hit rate >80%, two warm runs agree within 10%",
].join("\n") + "\n";

test("plan text pins the warm-cache steps and done-when gate", () => {
  assert.deepEqual(PLAN_LINES.join("\n") + "\n", EXPECTED);
  assert.equal(planText(), EXPECTED);
});

test("tool prints the pinned plan on stdout with exit 0", () => {
  const out = execFileSync(process.execPath, [TOOL], { encoding: "utf8", timeout: 30000 });
  assert.equal(out, EXPECTED);
  console.log(out.trimEnd());
});

test("source carries the fresh-credit comment plus the licence line", () => {
  const src = readFileSync(TOOL, "utf8");
  assert.match(src, /mozilla\/sccache/);
  assert.match(src, /SPDX-License-Identifier: Apache-2\.0/);
  assert.match(src, /written fresh/);
});

