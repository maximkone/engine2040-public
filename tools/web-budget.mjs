// Size and speed budgets for the browser build (WEB11).
//   node tools/web-budget.mjs [--dir dist/web] [--receipt out/web/receipt.json] [--budget tools/web-budget.json] [--require-receipt]
// Reads dist/web (what web-build.mjs wrote) and the receipt web-browse.mjs writes (wasm memory at the ready line,
// time to the first frame), prints one line
//   web-budget: wasm=<bytes> gzip=<bytes> brotli=<bytes> first_bundle=<bytes> mem=<bytes> ttff=<ms>
// and fails when a number is over its budget in tools/web-budget.json. Units: bytes, ms.
//   wasm          engine_bg.wasm as shipped
//   gzip, brotli  that file gzip level 9 / brotli quality 11 (Unity advises brotli; the host may serve either)
//   first_bundle  brotli size of everything the page loads first: every file in dist/web except build.json
//   mem           wasmMemoryBytes from the ready line
//   ttff          ms from navigation to the first frame on the canvas (one 250 ms poll of resolution)
// Without a receipt mem and ttff print `na` and are not gated, unless --require-receipt (then that fails).
// WEB8/WEB13: when the receipt carries them the line ends with `tier=<name> validation=<n> pipelines=<n>`; a validation error or a
// pipeline built after the loader (late_compile) fails the gate. An older receipt without them prints the six numbers only.
// Exit: 0 inside every budget, 1 a number over budget (or a required number missing), 2 could not run.
// Twiggy's top lines, when build.json holds them, are printed after the gate line for the record.
import { appendFileSync, existsSync, readFileSync, readdirSync } from "node:fs";
import { dirname, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { brotliCompressSync, constants, gzipSync } from "node:zlib";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
export const BUDGET_KEYS = ["wasm", "gzip", "brotli", "first_bundle", "mem", "ttff_ms"];
export const WASM_FILE = "engine_bg.wasm";

export class BudgetError extends Error {}

export const brotliSize = (buf) => brotliCompressSync(buf, { params: { [constants.BROTLI_PARAM_QUALITY]: 11, [constants.BROTLI_PARAM_SIZE_HINT]: buf.length } }).length;
export const gzipSize = (buf) => gzipSync(buf, { level: 9 }).length;

// Every file under `dir`, relative with forward slashes.
function listFiles(dir, base = dir) {
  const out = [];
  for (const e of readdirSync(dir, { withFileTypes: true })) {
    const p = join(dir, e.name);
    if (e.isDirectory()) out.push(...listFiles(p, base));
    else out.push(relative(base, p).split("\\").join("/"));
  }
  return out.sort();
}

// The four size numbers of a dist/web directory.
export function measureDist(dir) {
  const wasmPath = join(dir, WASM_FILE);
  if (!existsSync(wasmPath)) throw new BudgetError(`${wasmPath} does not exist (run node tools/web-build.mjs first)`);
  const wasm = readFileSync(wasmPath);
  let firstBundle = 0;
  for (const rel of listFiles(dir)) {
    if (rel === "build.json") continue;
    firstBundle += rel === WASM_FILE ? brotliSize(wasm) : brotliSize(readFileSync(join(dir, rel)));
  }
  return { wasm: wasm.length, gzip: gzipSize(wasm), brotli: brotliSize(wasm), first_bundle: firstBundle };
}

// mem and ttff from a browse receipt (text of receipt.json). A field the receipt lacks is null.
export function readReceipt(text) {
  let r;
  try { r = JSON.parse(text); } catch (e) { throw new BudgetError(`the receipt is not JSON: ${e.message}`); }
  if (!r || typeof r !== "object" || Array.isArray(r)) throw new BudgetError("the receipt is not a JSON object");
  const num = (v) => (typeof v === "number" && Number.isFinite(v) && v >= 0 ? v : null);
  const out = { mem: num(r.wasmMemoryBytes), ttff_ms: num(r.ttffMs) };
  if (typeof r.tier === "string" && r.tier) out.tier = r.tier;
  for (const [from, to] of [["validation", "validation"], ["pipelines", "pipelines"], ["latePipelines", "late_pipelines"]]) {
    const n = num(r[from]);
    if (n !== null) out[to] = n;
  }
  return out;
}

// The budgets object of web-budget.json text; every key must be a positive number.
export function readBudgets(text) {
  let j;
  try { j = JSON.parse(text); } catch (e) { throw new BudgetError(`the budget file is not JSON: ${e.message}`); }
  const b = j?.budgets;
  if (!b || typeof b !== "object") throw new BudgetError('the budget file has no "budgets" object');
  for (const k of BUDGET_KEYS) {
    if (!(typeof b[k] === "number" && b[k] > 0)) throw new BudgetError(`budget "${k}" must be a positive number`);
  }
  return Object.fromEntries(BUDGET_KEYS.map((k) => [k, b[k]]));
}

// The printed line.
export const formatLine = (m) => {
  const base = `web-budget: wasm=${m.wasm} gzip=${m.gzip} brotli=${m.brotli} first_bundle=${m.first_bundle} mem=${m.mem ?? "na"} ttff=${m.ttff_ms ?? "na"}`;
  if (m.tier == null && m.validation == null && m.pipelines == null) return base;
  return `${base} tier=${m.tier ?? "na"} validation=${m.validation ?? "na"} pipelines=${m.pipelines ?? "na"}`;
};

// Compare measured numbers to budgets. A number that is null is missing: a failure only when `requireReceipt`.
export function check(measured, budgets, { requireReceipt = false } = {}) {
  const failures = [];
  for (const k of BUDGET_KEYS) {
    const v = measured[k];
    if (v === null || v === undefined) {
      if (requireReceipt) failures.push(`${k} is missing from the receipt (the browse step did not record it)`);
      continue;
    }
    if (v > budgets[k]) failures.push(`${k} ${v} is over its budget ${budgets[k]} by ${v - budgets[k]} (${(((v - budgets[k]) / budgets[k]) * 100).toFixed(1)}%)`);
  }
  if (measured.validation > 0) failures.push(`validation ${measured.validation} errors on the page (the budget is 0)`);
  if (measured.late_pipelines > 0) failures.push(`late_compile: ${measured.late_pipelines} pipeline(s) built after the loader (the budget is 0)`);
  return { ok: failures.length === 0, failures };
}

export function parseArgs(argv) {
  const o = { dir: "dist/web", receipt: null, budget: join(root, "tools", "web-budget.json"), requireReceipt: false, error: null };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (a === "--dir") o.dir = argv[++i];
    else if (a === "--receipt") o.receipt = argv[++i];
    else if (a === "--budget") o.budget = argv[++i];
    else if (a === "--require-receipt") o.requireReceipt = true;
    else o.error = `unknown argument ${a}`;
  }
  for (const k of ["dir", "budget"]) if (!o[k]) o.error ??= `--${k} needs a value`;
  if (argv.includes("--receipt") && !o.receipt) o.error ??= "--receipt needs a value";
  if (o.requireReceipt && !o.receipt) o.error ??= "--require-receipt needs --receipt <file>";
  return o;
}

export function main(argv, say = console.log, fail = console.error) {
  const a = parseArgs(argv);
  if (a.error) { fail(`web-budget: ${a.error}\nusage: node tools/web-budget.mjs [--dir dist/web] [--receipt out/web/receipt.json] [--budget tools/web-budget.json] [--require-receipt]`); return 2; }
  try {
    const budgets = readBudgets(readFileSync(resolve(a.budget), "utf8"));
    const measured = { ...measureDist(resolve(a.dir)), mem: null, ttff_ms: null };
    if (a.receipt) {
      const path = resolve(a.receipt);
      if (!existsSync(path)) {
        if (a.requireReceipt) throw new BudgetError(`${path} does not exist (the browse step writes it)`);
      } else Object.assign(measured, readReceipt(readFileSync(path, "utf8")));
    }
    const line = formatLine(measured);
    say(line);
    try {
      const info = JSON.parse(readFileSync(join(resolve(a.dir), "build.json"), "utf8"));
      for (const l of info.twiggy ?? []) say(`web-budget: twiggy ${l}`);
    } catch { /* no build.json or no twiggy lines */ }
    const verdict = check(measured, budgets, { requireReceipt: a.requireReceipt });
    for (const f of verdict.failures) fail(`web-budget: FAIL ${f}`);
    if (process.env.GITHUB_STEP_SUMMARY) {
      const rows = BUDGET_KEYS.map((k) => `| ${k} | ${measured[k] ?? "na"} | ${budgets[k]} |`).join("\n");
      appendFileSync(process.env.GITHUB_STEP_SUMMARY, `### web-budget ${verdict.ok ? "ok" : "FAILED"}\n| number | measured | budget |\n|---|---|---|\n${rows}\n`);
    }
    return verdict.ok ? 0 : 1;
  } catch (e) {
    if (!(e instanceof BudgetError)) throw e;
    fail(`web-budget: ${e.message}`);
    return 2;
  }
}

if (import.meta.main) process.exit(main(process.argv.slice(2)));
