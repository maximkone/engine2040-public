// Size and speed budgets for the browser build (WEB11).
//   node tools/web-budget.mjs [--dir dist/web] [--receipt out/web/receipt.json] [--budget tools/web-budget.json] [--require-receipt]
// Reads dist/web (what web-build.mjs wrote) and the receipt web-browse.mjs writes (wasm memory at the ready line,
// time to the first frame), prints one line
//   web-budget: wasm=<bytes> gzip=<bytes> brotli=<bytes> first_bundle=<bytes> mem=<bytes> ttff=<ms>
// and fails when a number is over its budget in tools/web-budget.json. Units: bytes, ms.
//   wasm          the pinned wasm (engine_bg.<hash>.wasm) as shipped
//   gzip, brotli  that file gzip level 9 / brotli quality 11 (Unity advises brotli; the host may serve either)
//   first_bundle  the bytes the page loads first, each file in the form the host sends it (QW10: the two
//                 cooked bundles ship gzip), with the arena bundle counted only to its tier-0 prefix
//                 (QW4: the page starts there)
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
import { brotliCompressSync, constants, gunzipSync, gzipSync } from "node:zlib";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
export const BUDGET_KEYS = ["wasm", "gzip", "brotli", "first_bundle", "mem", "ttff_ms"];
export const WASM_FILE = "engine_bg.wasm";
export const BUILD_FILE = "build.json";
// QW4: the one file the page opens at its tier-0 boundary; every other file is
// loaded whole before the first frame.
export const ARENA_BUNDLE = "assets/arena.e2bundle";

export class BudgetError extends Error {}

export const brotliSize = (buf) => brotliCompressSync(buf, { params: { [constants.BROTLI_PARAM_QUALITY]: 11, [constants.BROTLI_PARAM_SIZE_HINT]: buf.length } }).length;
export const gzipSize = (buf) => gzipSync(buf, { level: 9 }).length;

// QW10: the two cooked bundles ship gzip-compressed (dist/web has a hard 8 MiB build
// limit and a static host will not compress a binary it does not know), so every
// measurement that looks inside a bundle unzips it first. The magic decides, exactly as it
// does in crates/web/loader.js.
export const isGzip = (buf) => buf.length >= 2 && buf[0] === 0x1f && buf[1] === 0x8b;
export const unzipped = (buf) => (isGzip(buf) ? gunzipSync(buf) : buf);

// PIN: the names a dist's build.json pins for the wasm and the arena bundle. A dist without a pin (an older build or a
// fixture) keeps the fixed names, so the gate still measures it.
export function distNames(dir) {
  try {
    const info = JSON.parse(readFileSync(join(dir, BUILD_FILE), "utf8"));
    return {
      wasm: info?.pin?.[WASM_FILE]?.name ?? WASM_FILE,
      arena: info?.pin?.[ARENA_BUNDLE]?.name ?? ARENA_BUNDLE,
    };
  } catch {
    return { wasm: WASM_FILE, arena: ARENA_BUNDLE };
  }
}

// WEB11 + QW4 + QW10: the bytes the page must receive before its first frame, each file in
// the form the host sends it. A file that already ships compressed counts as it stands, a
// plain one as its brotli size (the host may compress it), and the arena bundle only to its
// tier-0 prefix, which costs the gzip of those raw bytes while the stream decompresses.
// `arenaName` is the pinned file name; the fixed name is the default for a dist without a pin.
export function firstLoadBytes(rel, shipped, arenaName = ARENA_BUNDLE) {
  if (rel === arenaName) {
    const prefix = firstTierBytes(shipped);
    return isGzip(shipped) ? gzipSize(prefix) : brotliSize(prefix);
  }
  return isGzip(shipped) ? shipped.length : brotliSize(shipped);
}

// QW4: the bytes of an E2BUNDLE up to the end of its tier-0 blobs. That is the
// prefix the page opens before the first frame; the later tiers stream behind
// it. A file that is not a readable bundle is measured whole, never guessed.
export function firstTierBytes(buf) {
  buf = unzipped(buf); // QW10: a shipped bundle is gzip
  if (buf.length < 32 || buf.toString("latin1", 0, 8) !== "E2BUNDLE") return buf;
  try {
    const count = buf.readUInt32LE(12);
    let at = 32;
    let first = 0;
    for (let i = 0; i < count; i++) {
      const pathLen = buf.readUInt16LE(at);
      at += 2 + pathLen;
      const priority = buf[at];
      at += 1;
      const offset = Number(buf.readBigUInt64LE(at));
      at += 8;
      const len = Number(buf.readBigUInt64LE(at));
      at += 8;
      at += 8; // the blob's FNV-1a; the Rust reader verifies it on open
      if (priority === 0) first = Math.max(first, offset + len);
    }
    return first > 0 && first <= buf.length ? buf.subarray(0, first) : buf;
  } catch {
    return buf;
  }
}

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
  const names = distNames(dir);
  const wasmPath = join(dir, names.wasm);
  if (!existsSync(wasmPath)) throw new BudgetError(`${wasmPath} does not exist (run node tools/web-build.mjs first)`);
  const wasm = readFileSync(wasmPath);
  let firstBundle = 0;
  for (const rel of listFiles(dir)) {
    if (rel === BUILD_FILE) continue;
    firstBundle += firstLoadBytes(rel, rel === names.wasm ? wasm : readFileSync(join(dir, rel)), names.arena);
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
