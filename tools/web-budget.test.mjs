// web-budget drills: the gate prints the line, passes inside the budgets and fails on a fixture over a budget.
//   node --test tools/web-budget.test.mjs
import test from "node:test";
import assert from "node:assert/strict";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { randomBytes } from "node:crypto";
import { BUDGET_KEYS, BudgetError, brotliSize, check, formatLine, gzipSize, main, measureDist, parseArgs, readBudgets, readReceipt } from "./web-budget.mjs";

const BUDGETS = { wasm: 5000, gzip: 4000, brotli: 3900, first_bundle: 6000, mem: 2_000_000, ttff_ms: 8000 };

function fixture({ wasmBytes = 3000, receipt = { wasmMemoryBytes: 1_179_648, ttffMs: 900 }, budgets = BUDGETS } = {}) {
  const dir = mkdtempSync(join(tmpdir(), "web-budget-"));
  const dist = join(dir, "dist");
  mkdirSync(join(dist, "snippets"), { recursive: true });
  writeFileSync(join(dist, "engine_bg.wasm"), Buffer.concat([Buffer.from([0, 0x61, 0x73, 0x6d]), randomBytes(wasmBytes - 4)]));
  writeFileSync(join(dist, "engine.js"), "export default function init() {}\n".repeat(50));
  writeFileSync(join(dist, "index.html"), "<!doctype html><title>x</title>");
  writeFileSync(join(dist, "snippets", "a.js"), "export const a = 1;\n");
  writeFileSync(join(dist, "build.json"), JSON.stringify({ twiggy: ["Shallow Bytes | Item", "123 | code[0]"] }));
  if (receipt) writeFileSync(join(dir, "receipt.json"), JSON.stringify(receipt));
  writeFileSync(join(dir, "budget.json"), JSON.stringify({ schema: 1, budgets }));
  return { dir, dist, receipt: join(dir, "receipt.json"), budget: join(dir, "budget.json") };
}

const run = (argv) => {
  const out = [], err = [];
  const code = main(argv, (l) => out.push(l), (l) => err.push(l));
  return { code, out, err };
};

test("measureDist counts wasm, gzip, brotli and the first bundle without build.json", () => {
  const f = fixture();
  try {
    const m = measureDist(f.dist);
    const wasm = readFileSync(join(f.dist, "engine_bg.wasm"));
    assert.equal(m.wasm, 3000);
    assert.equal(m.gzip, gzipSize(wasm));
    assert.equal(m.brotli, brotliSize(wasm));
    const parts = ["engine_bg.wasm", "engine.js", "index.html", "snippets/a.js"].map((p) => brotliSize(readFileSync(join(f.dist, p))));
    assert.equal(m.first_bundle, parts.reduce((a, b) => a + b, 0));
  } finally { rmSync(f.dir, { recursive: true, force: true }); }
});

test("measureDist says what to run when the wasm is missing", () => {
  const dir = mkdtempSync(join(tmpdir(), "web-budget-"));
  try { assert.throws(() => measureDist(dir), /web-build\.mjs/); } finally { rmSync(dir, { recursive: true, force: true }); }
});

test("readReceipt takes memory and ttff, null for anything missing or not a number", () => {
  assert.deepEqual(readReceipt('{"wasmMemoryBytes":1179648,"ttffMs":401}'), { mem: 1179648, ttff_ms: 401 });
  assert.deepEqual(readReceipt('{"wasmMemoryBytes":null,"ttffMs":"fast"}'), { mem: null, ttff_ms: null });
  assert.deepEqual(readReceipt("{}"), { mem: null, ttff_ms: null });
  assert.throws(() => readReceipt("nope"), BudgetError);
  assert.throws(() => readReceipt("[1]"), /not a JSON object|JSON/);
});

test("readBudgets wants every key as a positive number", () => {
  assert.deepEqual(readBudgets(JSON.stringify({ budgets: BUDGETS })), BUDGETS);
  for (const k of BUDGET_KEYS) {
    const b = { ...BUDGETS };
    delete b[k];
    assert.throws(() => readBudgets(JSON.stringify({ budgets: b })), new RegExp(k));
    assert.throws(() => readBudgets(JSON.stringify({ budgets: { ...BUDGETS, [k]: 0 } })), new RegExp(k));
  }
  assert.throws(() => readBudgets("{}"), /budgets/);
});

test("the checked-in tools/web-budget.json is valid", () => {
  const b = readBudgets(readFileSync(new URL("./web-budget.json", import.meta.url), "utf8"));
  assert.ok(b.wasm >= 215789 && b.ttff_ms <= 8000);
});

test("check passes at the budget and names every number over it", () => {
  assert.equal(check({ ...BUDGETS }, BUDGETS).ok, true);
  const v = check({ ...BUDGETS, wasm: 5001, mem: 3_000_000 }, BUDGETS);
  assert.equal(v.ok, false);
  assert.equal(v.failures.length, 2);
  assert.match(v.failures[0], /^wasm 5001 is over its budget 5000 by 1 /);
  assert.match(v.failures[1], /^mem /);
});

test("check ignores a missing receipt number unless the receipt is required", () => {
  const m = { wasm: 1, gzip: 1, brotli: 1, first_bundle: 1, mem: null, ttff_ms: null };
  assert.equal(check(m, BUDGETS).ok, true);
  const v = check(m, BUDGETS, { requireReceipt: true });
  assert.equal(v.ok, false);
  assert.equal(v.failures.length, 2);
});

test("formatLine prints the six numbers in the documented order", () => {
  assert.equal(formatLine({ wasm: 1, gzip: 2, brotli: 3, first_bundle: 4, mem: 5, ttff_ms: 6 }), "web-budget: wasm=1 gzip=2 brotli=3 first_bundle=4 mem=5 ttff=6");
  assert.match(formatLine({ wasm: 1, gzip: 2, brotli: 3, first_bundle: 4, mem: null, ttff_ms: null }), /mem=na ttff=na$/);
});

test("main passes inside the budgets and prints the line and the twiggy lines", () => {
  const f = fixture();
  try {
    const r = run(["--dir", f.dist, "--receipt", f.receipt, "--budget", f.budget, "--require-receipt"]);
    assert.equal(r.code, 0, r.err.join("\n"));
    assert.match(r.out[0], /^web-budget: wasm=3000 gzip=\d+ brotli=\d+ first_bundle=\d+ mem=1179648 ttff=900$/);
    assert.deepEqual(r.out.slice(1), ["web-budget: twiggy Shallow Bytes | Item", "web-budget: twiggy 123 | code[0]"]);
    assert.deepEqual(r.err, []);
  } finally { rmSync(f.dir, { recursive: true, force: true }); }
});

test("main fails a wasm over its budget", () => {
  const f = fixture({ wasmBytes: 6000 });
  try {
    const r = run(["--dir", f.dist, "--receipt", f.receipt, "--budget", f.budget]);
    assert.equal(r.code, 1);
    assert.ok(r.err.some((l) => /^web-budget: FAIL wasm 6000 is over its budget 5000/.test(l)), r.err.join("\n"));
  } finally { rmSync(f.dir, { recursive: true, force: true }); }
});

test("main fails memory and ttff over budget from the receipt", () => {
  const f = fixture({ receipt: { wasmMemoryBytes: 3_000_000, ttffMs: 9000 } });
  try {
    const r = run(["--dir", f.dist, "--receipt", f.receipt, "--budget", f.budget]);
    assert.equal(r.code, 1);
    assert.equal(r.err.length, 2);
    assert.match(r.err.join("\n"), /mem 3000000/);
    assert.match(r.err.join("\n"), /ttff_ms 9000/);
  } finally { rmSync(f.dir, { recursive: true, force: true }); }
});

test("main without a receipt prints na and passes; with --require-receipt a missing file is exit 2", () => {
  const f = fixture({ receipt: null });
  try {
    const r = run(["--dir", f.dist, "--budget", f.budget]);
    assert.equal(r.code, 0);
    assert.match(r.out[0], /mem=na ttff=na$/);
    const need = run(["--dir", f.dist, "--receipt", f.receipt, "--budget", f.budget, "--require-receipt"]);
    assert.equal(need.code, 2);
    assert.match(need.err.join("\n"), /receipt/);
  } finally { rmSync(f.dir, { recursive: true, force: true }); }
});

test("main refuses a receipt that lacks the numbers when they are required", () => {
  const f = fixture({ receipt: { schema: 1 } });
  try {
    const r = run(["--dir", f.dist, "--receipt", f.receipt, "--budget", f.budget, "--require-receipt"]);
    assert.equal(r.code, 1);
    assert.equal(r.err.length, 2);
  } finally { rmSync(f.dir, { recursive: true, force: true }); }
});

test("parseArgs and main refuse bad arguments with exit 2", () => {
  assert.equal(parseArgs(["--bogus"]).error, "unknown argument --bogus");
  assert.match(parseArgs(["--require-receipt"]).error, /--receipt/);
  assert.match(parseArgs(["--receipt"]).error, /needs a value/);
  assert.equal(run(["--bogus"]).code, 2);
  assert.equal(run(["--dir", join(tmpdir(), "no-such-dir-web-budget")]).code, 2);
});

test("the browse receipt carries what the gate reads", async () => {
  const { receiptOf } = await import("./web-browse.mjs");
  const r = receiptOf({ wasmMemoryBytes: 1179648, commit: "abc" }, 401, "swiftshader-angle");
  assert.deepEqual(readReceipt(JSON.stringify(r)), { mem: 1179648, ttff_ms: 401 });
  assert.deepEqual(readReceipt(JSON.stringify(receiptOf(null, null, "x"))), { mem: null, ttff_ms: null });
});

test("the line carries tier, validation and pipelines once the receipt has them (WEB8, WEB13)", () => {
  const f = fixture({ receipt: { wasmMemoryBytes: 1179648, ttffMs: 900, tier: "web", validation: 0, pipelines: 24, latePipelines: 0 } });
  try {
    const r = run(["--dir", f.dist, "--receipt", f.receipt, "--budget", f.budget, "--require-receipt"]);
    assert.equal(r.code, 0, r.err.join("\n"));
    assert.match(r.out[0], / mem=1179648 ttff=900 tier=web validation=0 pipelines=24$/);
  } finally { rmSync(f.dir, { recursive: true, force: true }); }
});

test("a validation error or a late pipeline on the page fails the gate", () => {
  const f = fixture({ receipt: { wasmMemoryBytes: 1179648, ttffMs: 900, tier: "web", validation: 2, pipelines: 24, latePipelines: 3 } });
  try {
    const r = run(["--dir", f.dist, "--receipt", f.receipt, "--budget", f.budget]);
    assert.equal(r.code, 1);
    assert.match(r.err.join("\n"), /validation 2 errors/);
    assert.match(r.err.join("\n"), /late_compile: 3 pipeline/);
  } finally { rmSync(f.dir, { recursive: true, force: true }); }
});

test("readReceipt keeps the new fields only when the receipt has them", () => {
  assert.deepEqual(readReceipt(JSON.stringify({ wasmMemoryBytes: 5, ttffMs: 6, tier: "potato", validation: 0, pipelines: 9, latePipelines: 1 })), { mem: 5, ttff_ms: 6, tier: "potato", validation: 0, pipelines: 9, late_pipelines: 1 });
  assert.deepEqual(readReceipt(JSON.stringify({ wasmMemoryBytes: 5, ttffMs: 6, tier: null, validation: null })), { mem: 5, ttff_ms: 6 });
});
