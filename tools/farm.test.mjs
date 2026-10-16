// Run: node --test tools/farm.test.mjs
import assert from "node:assert/strict";
import test from "node:test";
import { errorLines, failedSteps, farmVerdict } from "./farm.mjs";

test("errorLines keeps defect lines per job and drops the noise", () => {
  const log = [
    "test (render)\ttest render\t2026-10-16T06:45:19.6Z \u001b[1m\u001b[91merror\u001b[0m: could not compile `engine-render`",
    "test (render)\ttest render\t2026-10-16T06:45:20.1Z    Compiling engine-core v0.1.0",
    "quick\tfmt\t2026-10-16T06:45:21.0Z Diff in /home/runner/work/x/src/lib.rs:1:",
    "bad line without tabs",
  ].join("\n");
  const by = errorLines(log);
  assert.deepEqual([...by.keys()], ["test (render)", "quick"]);
  assert.equal(by.get("test (render)").length, 1);
  assert.match(by.get("test (render)")[0], /^error: could not compile/);
  assert.match(by.get("quick")[0], /^Diff in /);
});

test("errorLines keeps each FAIL family from a real farm run", () => {
  const log = [
    "quick\trust quality ratchet\t2026-10-16T13:40:25.4505027Z [FAIL] ratchet engine-render: dup_fns 9 -> 10 (function bodies duplicated elsewhere in the workspace)",
    "quick\trust quality ratchet\t2026-10-16T13:40:25.4505752Z [FAIL] ratchet engine-render: hot_alloc 127 -> 143 (allocations inside step/tick/update/render/mix functions of hot crates)",
    "quick\trust quality ratchet\t2026-10-16T13:40:25.4506917Z RESULT FAIL: 2 ratchet regression(s)",
    "release-proof\treal seed and recorded-input replay proofs\t2026-10-16T13:42:44.4999742Z [FAIL] rust quality ratchet (tools/rust-quality.mjs) \u2014 [FAIL] ratchet engine-render: dup_fns 9 -> 10 | RESULT FAIL: 2 ratchet regression(s)",
    "release-proof\treal seed and recorded-input replay proofs\t2026-10-16T13:42:44.5373842Z RESULT FAIL",
    "quick\tworkspace and dependency boundaries\t2026-10-16T13:40:20.0Z FAIL workspace audit: engine-render: shadow.rs is not declared by any module, so it is never compiled",
    "quick\tfmt\t2026-10-16T13:40:18.0Z node tools/workspace-check.mjs -> FAIL workspace audit: engine-assets: schema_compile.rs is not declared by any module",
  ].join("\n");
  const by = errorLines(log);
  assert.deepEqual([...by.keys()].sort(), ["quick", "release-proof"]);
  assert.ok(by.get("quick").some((l) => l.includes("[FAIL] ratchet engine-render: dup_fns 9 -> 10")), "dup_fns kept");
  assert.ok(by.get("quick").some((l) => l.includes("[FAIL] ratchet engine-render: hot_alloc 127 -> 143")), "hot_alloc kept");
  assert.ok(by.get("quick").some((l) => l.startsWith("RESULT FAIL: 2 ratchet")), "RESULT FAIL with count kept");
  assert.ok(by.get("release-proof").some((l) => l.includes("[FAIL] ratchet")), "release-proof [FAIL] kept");
  assert.ok(by.get("release-proof").some((l) => l === "RESULT FAIL"), "bare RESULT FAIL kept");
  assert.ok(by.get("quick").some((l) => l.includes("FAIL workspace audit: engine-render")), "FAIL workspace audit at start kept");
  assert.ok(by.get("quick").some((l) => l.includes("FAIL workspace audit: engine-assets")), "FAIL workspace audit mid-line kept");
});

test("errorLines drops benign N failed counter noise", () => {
  const log = [
    "release-proof\treal seed and recorded-input replay proofs\t2026-10-16T13:42:39.9823159Z [INFO] gpu-live truth gate (report-only): test result: ok. 18 passed; 0 failed; 0 ignored; 0 measured; 119 filtered out; finished in 0.00s",
    "quick\tgate tool tests\t2026-10-16T13:40:30.0Z 0 failed",
    "quick\tgate tool tests\t2026-10-16T13:40:31.0Z 2 failed",
  ].join("\n");
  const by = errorLines(log);
  assert.equal(by.size, 0, "benign counters dropped: " + JSON.stringify([...by.values()]));
  // A real FAILED verdict with counts still names a defect.
  const real = errorLines("quick\tgate tool tests\t2026-10-16T13:40:32.0Z test result: FAILED. 3 passed; 2 failed; 0 ignored");
  assert.equal(real.get("quick")?.length, 1, "FAILED verdict kept even with counts");
});

test("failedSteps lists every failing step, not just the first", () => {
  const job = { name: "quick", steps: [
    { name: "fmt", conclusion: "success" },
    { name: "workspace and dependency boundaries", conclusion: "failure" },
    { name: "rust quality ratchet (docs/RUST-QUALITY.md)", conclusion: "failure" },
    { name: "gate tool tests", conclusion: "failure" },
    { name: "dependency policy", conclusion: "success" },
  ] };
  assert.deepEqual(failedSteps(job), [
    "workspace and dependency boundaries",
    "rust quality ratchet (docs/RUST-QUALITY.md)",
    "gate tool tests",
  ]);
  assert.deepEqual(failedSteps({ name: "ok", steps: [{ name: "fmt", conclusion: "success" }] }), []);
  assert.deepEqual(failedSteps({ name: "empty" }), []);
});

test("farmVerdict reports all-skipped as SKIPPED, not GREEN", () => {
  const names = ["quick", "test", "clippy", "release-proof", "window-compile", "arm-golden"];
  const allSkipped = names.map((name) => ({ name, conclusion: "skipped" }));
  assert.equal(farmVerdict(allSkipped), "SKIPPED");
});

test("farmVerdict keeps mixed-run behavior: success+skipped stays GREEN, failure stays RED", () => {
  assert.equal(farmVerdict([
    { name: "quick", conclusion: "success" },
    { name: "clippy", conclusion: "skipped" },
  ]), "GREEN");
  assert.equal(farmVerdict([
    { name: "quick", conclusion: "success" },
    { name: "clippy", conclusion: "success" },
  ]), "GREEN");
  assert.equal(farmVerdict([
    { name: "quick", conclusion: "success" },
    { name: "clippy", conclusion: "failure" },
    { name: "test", conclusion: "skipped" },
  ]), "RED");
});
