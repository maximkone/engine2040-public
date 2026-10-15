#!/usr/bin/env node
// API2 ratchet: per-crate missing-docs never drops; empty /// undoc, removal fails.
import { existsSync, readdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url"; import { lex, testLines } from "./rust-quality.mjs";
const HERE = dirname(fileURLToPath(import.meta.url)); const DEFAULT_ROOT = resolve(HERE, ".."); const BASELINE_NAME = "docs-coverage.baseline.json"; const EPS = 1e-9;
const PUB_ITEM = /^\s*pub\s+(?:(?:const|async|unsafe|extern\s+"[^"]*")\s+)*(?:fn|struct|enum|trait|type|const|static|union|mod)\b/;
const DOC_EMPTY = /^\s*#\[\s*doc\s*=\s*"\s*"\s*\]\s*$/; const DOC_ATTR = /^\s*#\[\s*doc\b/; const ATTR_LINE = /^\s*#\[.*\]?\s*$/;
export function isDocumented(raw, l) {
  let b = l - 1;
  while (b >= 0 && ATTR_LINE.test(raw[b] ?? "")) { const a = raw[b] ?? ""; if (DOC_EMPTY.test(a)) { b--; continue; } if (DOC_ATTR.test(a)) return true; b--; }
  if (b < 0) return false;
  const t = (raw[b] ?? "").trim();
  if (t.startsWith("///")) {
    for (let k = b; k >= 0 && (raw[k] ?? "").trim().startsWith("///"); k--) if ((raw[k] ?? "").trim().slice(3).trim()) return true;
    return false;
  }
  if (t.startsWith("/**")) {
    const inner = t.slice(3); const shut = inner.lastIndexOf("*/"); const body = shut >= 0 ? inner.slice(0, shut) : inner;
    if (body.replace(/^\*+/, "").trim()) return true;
    if (shut >= 0) return false;
    for (let k = b + 1; k < raw.length; k++) {
      const u = (raw[k] ?? "").trim(); const e = u.indexOf("*/"); const s = e >= 0 ? u.slice(0, e) : u;
      if (s.replace(/^\*+/, "").trim()) return true;
      if (e >= 0) return false;
    }
    return false;
  }
  if (/^\*(?!\/)/.test(t) || t.startsWith("*/")) {
    let k = b;
    const tail = (s) => /^\*(?!\/)/.test(s) || s.startsWith("*/");
    while (k >= 0 && tail((raw[k] ?? "").trim())) {
      const u = (raw[k] ?? "").trim(); const e = u.indexOf("*/"); const s = e >= 0 ? u.slice(0, e) : u;
      if (s.replace(/^\*+/, "").trim()) return true;
      k--;
    }
    if ((raw[k] ?? "").trim().startsWith("/**")) {
      const h = (raw[k] ?? "").trim().slice(3); const q = h.lastIndexOf("*/"); const f = q >= 0 ? h.slice(0, q) : h;
      if (f.replace(/^\*+/, "").trim()) return true;
    }
    return false;
  }
  return false;
}
export function analyzeText(text) {
  const norm = text.replace(/\r\n/g, "\n"); const { code } = lex(norm);
  const lines = code.split("\n"); const raw = norm.split("\n"); const masked = testLines(code);
  let total = 0; let documented = 0; const missing = [];
  for (let l = 0; l < lines.length; l++) {
    if (masked[l] || !PUB_ITEM.test(lines[l])) continue;
    total++;
    if (isDocumented(raw, l)) documented++;
    else missing.push({ line: l + 1, text: (raw[l] ?? "").trim().slice(0, 120) });
  }
  return { total, documented, undocumented: total - documented, missing };
}
export function coverageOf(total, documented) { return total === 0 ? 100 : (100 * documented) / total; }
function walk(dir) {
  if (!existsSync(dir)) return [];
  return readdirSync(dir, { withFileTypes: true }).flatMap((e) => {
    if (e.name === "target" || e.name.startsWith(".")) return [];
    const p = join(dir, e.name); return e.isDirectory() ? walk(p) : e.name.endsWith(".rs") ? [p] : [];
  });
}
function packages(root) {
  const pkgs = []; const crates = join(root, "crates");
  for (const dir of existsSync(crates) ? readdirSync(crates).sort() : []) {
    const m = join(crates, dir, "Cargo.toml"); if (!existsSync(m)) continue;
    pkgs.push({ name: /^\s*name\s*=\s*"([^"]+)"/m.exec(readFileSync(m, "utf8"))?.[1] ?? `engine-${dir}`, dir: join(crates, dir) });
  }
  if (existsSync(join(root, "src"))) pkgs.push({ name: "engine2040", dir: root });
  return pkgs;
}
export function measure(root) {
  const out = new Map();
  for (const pkg of packages(root)) {
    const c = { name: pkg.name, total: 0, documented: 0, undocumented: 0, missing: [], files: 0 };
    const files = walk(join(pkg.dir, "src")); c.files = files.length;
    for (const file of files) {
      const a = analyzeText(readFileSync(file, "utf8"));
      c.total += a.total; c.documented += a.documented; c.undocumented += a.undocumented;
      const rel = relative(root, file).split("\\").join("/");
      for (const m of a.missing) c.missing.push(`${rel}:${m.line}: ${m.text}`);
    }
    c.coverage = coverageOf(c.total, c.documented); out.set(pkg.name, c);
  }
  return out;
}
function snapshot(measured) {
  const out = {};
  for (const [n, c] of [...measured].sort((a, b) => a[0].localeCompare(b[0]))) out[n] = { total: c.total, documented: c.documented, undocumented: c.undocumented, coverage: c.coverage };
  return out;
}
export function compare(baseline, measured) {
  const regressions = []; const improvements = [];
  for (const [name, c] of measured) {
    const base = baseline.crates?.[name];
    if (!base) { regressions.push(`${name}: new crate with no recorded baseline (run --update-baseline to record ${c.total} items at ${c.coverage.toFixed(1)}%)`); continue; }
    if (c.total < (base.total ?? 0)) regressions.push(`${name}: public items removed ${base.total} -> ${c.total} (undocumented ${base.undocumented} -> ${c.undocumented}, coverage ${c.coverage.toFixed(1)}% vs baseline ${Number(base.coverage).toFixed(1)}%); re-record with --update-baseline only after a deliberate removal`);
    else if (c.undocumented > (base.undocumented ?? 0)) regressions.push(`${name}: undocumented public items ${base.undocumented} -> ${c.undocumented} (coverage ${c.coverage.toFixed(1)}% vs baseline ${Number(base.coverage).toFixed(1)}%)`);
    else if (c.coverage < Number(base.coverage) - EPS) regressions.push(`${name}: coverage ${c.coverage.toFixed(1)}% < baseline ${Number(base.coverage).toFixed(1)}%`);
    else if (c.undocumented < (base.undocumented ?? 0) || c.coverage > Number(base.coverage) + EPS) improvements.push({ name, was: base.coverage, now: Math.round(c.coverage * 10) / 10 });
  }
  return { regressions, improvements };
}
function table(measured, baseline) {
  const rows = [["crate", "total", "doc", "undoc", "cov%", "base%", "status"]];
  for (const [name, c] of [...measured].sort((a, b) => a[1].coverage - b[1].coverage)) {
    const base = baseline.crates?.[name];
    const bad = !base || c.total < (base.total ?? 0) || c.undocumented > (base.undocumented ?? 0) || c.coverage < Number(base.coverage) - EPS;
    rows.push([name.replace(/^engine-/, ""), String(c.total), String(c.documented), String(c.undocumented), c.coverage.toFixed(1), base === undefined ? "new" : Number(base.coverage).toFixed(1), bad ? "FAIL" : "ok"]);
  }
  const w = rows[0].map((_, i) => Math.max(...rows.map((r) => r[i].length)));
  return rows.map((r) => r.map((v, i) => (i === 0 ? v.padEnd(w[i]) : v.padStart(w[i]))).join("  ")).join("\n");
}
function writeBaseline(path, measured) {
  writeFileSync(path, `${JSON.stringify({ version: 1, note: "Ratchet for tools/docs-coverage.mjs: per-crate doc coverage never drops; --update-baseline re-records after a deliberate docs change.", crates: snapshot(measured) }, null, 1)}\n`);
}
function main(argv) {
  const o = { crates: [], json: false, update: false, root: DEFAULT_ROOT, limit: 5 };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (a === "--crate") o.crates.push(argv[++i] ?? "");
    else if (a === "--json") o.json = true;
    else if (a === "--update-baseline") o.update = true;
    else if (a === "--root") o.root = resolve(argv[++i] ?? ".");
    else if (a === "--limit") o.limit = Number(argv[++i]);
    else throw new Error(`unknown argument: ${a}`);
  }
  if (!Number.isFinite(o.limit) || o.limit < 0) throw new Error("--limit needs a non-negative number");
  const bp = join(o.root, "tools", BASELINE_NAME); const measured = measure(o.root);
  const want = (n) => !o.crates.length || o.crates.some((c) => c === n || `engine-${c}` === n);
  const shown = new Map([...measured].filter(([n]) => want(n)));
  if (o.crates.length && !shown.size) throw new Error(`no crate matches ${o.crates.join(", ")}`);
  if (o.update) { writeBaseline(bp, measured); console.log(`WROTE ${bp} (${measured.size} crates)`); return 0; }
  if (!existsSync(bp)) { writeBaseline(bp, measured); console.log(`[INFO] no baseline found; recorded current coverage to ${bp}`); }
  const baseline = JSON.parse(readFileSync(bp, "utf8")); const { regressions, improvements } = compare(baseline, shown);
  if (o.json) {
    const out = {};
    for (const [n, c] of shown) out[n] = { total: c.total, documented: c.documented, undocumented: c.undocumented, coverage: Math.round(c.coverage * 10) / 10, files: c.files };
    console.log(JSON.stringify({ crates: out, regressions, improvements: improvements.length }, null, 1));
    return regressions.length ? 1 : 0;
  }
  console.log("Docs coverage: documented public items per crate (src only, #[cfg(test)] masked)");
  console.log(table(shown, baseline));
  for (const [n, c] of shown) {
    if (!regressions.some((r) => r.startsWith(`${n}:`))) continue;
    for (const m of c.missing.slice(0, o.limit)) console.log(`[FAIL] undocumented ${m}`);
    if (c.missing.length > o.limit) console.log(`[FAIL] ... and ${c.missing.length - o.limit} more undocumented item(s) in ${n} (run with --limit N)`);
  }
  for (const r of regressions) console.log(`[FAIL] ratchet ${r}`);
  if (improvements.length) console.log(`[INFO] ${improvements.length} crate(s) above baseline: the orchestrator re-records with --update-baseline after a deliberate docs change`);
  console.log(regressions.length ? `RESULT FAIL: ${regressions.length} coverage regression(s)` : `RESULT PASS: 0 coverage regressions over ${shown.size} crate(s)`);
  return regressions.length ? 1 : 0;
}
if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try { process.exitCode = main(process.argv.slice(2)); }
  catch (e) { console.error(`RESULT FAIL: ${e.message}`); process.exitCode = 2; }
}
