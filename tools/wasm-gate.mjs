#!/usr/bin/env node
// WEB2 wasm gate. `cargo check --target wasm32-unknown-unknown` proves the web-path crates type-check for the
// browser, not that they run there: on that target std::fs returns errors, Instant::now and SystemTime::now
// panic, threads and raw sockets fail, and a tab cannot block on the GPU. This scans the non-test source of
// the web-path crates for those calls, prints the totals per crate, and fails when a file gets more of them
// than tools/wasm-gate.baseline.json allows. The baseline only goes down (--tighten); there is no way to raise it.
//   node tools/wasm-gate.mjs            totals per crate + ratchet against the baseline (exit 1 on a regression)
//   node tools/wasm-gate.mjs --list     also every hit as file:line
//   node tools/wasm-gate.mjs --tighten  lower baseline counts that improved
//   node tools/wasm-gate.mjs --init     write the first baseline (refuses when one exists)
//
// What is scanned: crates/<name>/src/**/*.rs of WEB_CRATES, with strings and comments blanked first. Not scanned:
// tests/, benches/, examples/ (they are outside src), src/bin and src/main.rs unless listed in WEB_BINS,
// files named tests.rs or *_tests.rs or inside a tests/ directory, and any item whose #[cfg(...)] is false
// for wasm32-unknown-unknown. That cfg rule is simple on purpose: the predicate is evaluated with
// target_arch = "wasm32", target_os = "unknown", target_family = "wasm", test, unix and windows known, and
// everything else (features, debug_assertions) unknown; an item is skipped only when the result is false, so
// `cfg(test)`, `cfg(not(target_arch = "wasm32"))`, `cfg(windows)` and `cfg(all(unix, feature = "x"))` skip it
// and `cfg(feature = "x")` does not. A `#[cfg(..)] mod x;` that is false skips the whole file or directory
// of x, and a leading `#![cfg(..)]` that is false skips the file. The item ends at its closing brace, or at
// the first `;` outside brackets.
import { existsSync, readdirSync, readFileSync, writeFileSync } from "node:fs";
import { basename, dirname, join, relative } from "node:path";
import { fileURLToPath } from "node:url";
import { lex, testLines, testOnlyFile } from "./rust-quality.mjs";

const REPO = join(dirname(fileURLToPath(import.meta.url)), "..");
export const BASELINE = "tools/wasm-gate.baseline.json";

// Directory names under crates/ that ship to the browser.
export const WEB_CRATES = [
  "core", "ecs", "geo", "spatial", "rules", "input", "combat", "anim", "ai", "nn", "physics", "physfx", "destruction",
  "nav", "arena", "scene", "material", "world", "save", "observe", "ui", "audio", "assets", "net", "game", "render",
  "proof", "app", "clock",
];
// Repo-relative bin files or directories whose code runs in the browser. None yet: the browser shell is the
// crates/web library; list a bin here when its code moves to the web path.
export const WEB_BINS = [];

export const HAZARDS = [
  { id: "fs", what: "std::fs: a browser has no file system, every call returns Err", res: [/\bstd::fs\b/g, /\buse\s+std::\{[^}]*\bfs\b/g] },
  { id: "clock", what: "Instant::now / SystemTime::now: panic on wasm32-unknown-unknown, read engine-clock instead", res: [/\b(?:Instant|SystemTime)::now\b/g] },
  {
    id: "thread",
    what: "thread::spawn / Builder::spawn: no threads in a tab, spawn returns Err",
    res: [/\bthread::(?:spawn|scope)\b/g, /\bthread::Builder::new\b/g, /(?<!thread::)\bBuilder::new\(\)(?:\s*\.\s*\w+\((?:[^()]|\([^()]*\))*\))*\s*\.spawn\(/g],
  },
  { id: "socket", what: "UdpSocket / TcpStream / TcpListener: no raw sockets in a browser", res: [/\b(?:UdpSocket|TcpStream|TcpListener)\b/g] },
  { id: "exe", what: "current_exe / current_dir: a tab has no executable path or working directory", res: [/\bcurrent_(?:exe|dir)\s*\(/g] },
  { id: "block", what: "pollster::block_on / device.poll(wait): the main thread cannot block on the GPU", res: [/\bpollster::block_on\b/g, /\.poll\(\s*[\w:]*[Ww]ait\w*/g] },
];
const WHAT = Object.fromEntries(HAZARDS.map((h) => [h.id, h.what]));

// ---- cfg predicates for wasm32-unknown-unknown ----
const TARGET = { target_arch: "wasm32", target_os: "unknown", target_family: "wasm", target_pointer_width: "32", target_vendor: "unknown", target_env: "", target_endian: "little" };
const FLAGS = { test: false, unix: false, windows: false };
const combine = (name, args) => {
  if (name === "not") return args[0] === null ? null : !args[0];
  if (name === "all") return args.includes(false) ? false : args.includes(null) ? null : true;
  if (name === "any") return args.includes(true) ? true : args.includes(null) ? null : false;
  return null;
};
// true / false for the browser target, null when the predicate depends on something unknown (features...).
export function evalCfg(text) {
  let i = 0;
  const skip = () => { while (i < text.length && /\s/.test(text[i])) i++; };
  const word = () => {
    skip();
    const m = /^[A-Za-z_][A-Za-z0-9_]*/.exec(text.slice(i));
    if (!m) throw new Error(`bad cfg predicate: ${text}`);
    i += m[0].length;
    return m[0];
  };
  const pred = () => {
    const name = word();
    skip();
    if (text[i] === "(") {
      i++;
      const args = [];
      for (skip(); text[i] !== ")"; skip()) { args.push(pred()); skip(); if (text[i] === ",") i++; }
      i++;
      return combine(name, args);
    }
    if (text[i] === "=") {
      i++; skip();
      const m = /^"([^"]*)"/.exec(text.slice(i));
      if (!m) throw new Error(`bad cfg predicate: ${text}`);
      i += m[0].length;
      return name in TARGET ? TARGET[name] === m[1] : null;
    }
    return name in FLAGS ? FLAGS[name] : null;
  };
  try { return pred(); } catch { return null; }
}

// ---- one file ----
function lineStarts(code) {
  const starts = [0];
  for (let i = 0; i < code.length; i++) if (code[i] === "\n") starts.push(i + 1);
  return starts;
}
function lineOf(starts, offset) {
  let lo = 0;
  let hi = starts.length - 1;
  while (lo < hi) {
    const mid = (lo + hi + 1) >> 1;
    if (starts[mid] <= offset) lo = mid; else hi = mid - 1;
  }
  return lo;
}
// Offset of the bracket closing the one at `open` (same kind), or -1.
function matchClose(code, open, up, down) {
  let depth = 0;
  for (let i = open; i < code.length; i++) {
    if (code[i] === up) depth++;
    else if (code[i] === down) { depth--; if (depth === 0) return i; }
  }
  return -1;
}

// Hazard hits of one source text, outside test code and outside items that are off for the browser.
// `mods` are the names of `#[cfg(false)] mod x;` declarations: the caller skips those files.
// `skipped` is true when the whole file is off (test-only, or a false leading #![cfg]).
export function scanText(text) {
  const src = text.replace(/\r\n/g, "\n");
  if (testOnlyFile(src)) return { hits: [], mods: [], skipped: true };
  const { code } = lex(src);
  const starts = lineStarts(code);
  const mask = Uint8Array.from(testLines(code));
  const mods = [];
  const attr = /#\s*(!?)\s*\[\s*cfg\s*\(/g;
  for (let m = attr.exec(code); m; m = attr.exec(code)) {
    const open = m.index + m[0].length - 1;
    const close = matchClose(code, open, "(", ")");
    if (close < 0 || evalCfg(src.slice(open + 1, close)) !== false) continue;
    if (m[1] === "!") {
      if (/^(?:\s*#!\[[^\]]*\])*\s*$/.test(code.slice(0, m.index))) return { hits: [], mods, skipped: true };
      continue;
    }
    let depth = 0;
    let end = code.length - 1;
    for (let k = m.index; k < code.length; k++) {
      const ch = code[k];
      if (ch === "[" || ch === "(") depth++;
      else if (ch === "]" || ch === ")") depth--;
      else if (depth === 0 && ch === ";") { end = k; break; }
      else if (depth === 0 && ch === "{") { const c = matchClose(code, k, "{", "}"); end = c < 0 ? code.length - 1 : c; break; }
    }
    const decl = /\bmod\s+(\w+)\s*;$/.exec(code.slice(m.index, end + 1));
    if (decl) mods.push(decl[1]);
    for (let l = lineOf(starts, m.index); l <= lineOf(starts, end); l++) mask[l] = 1;
    attr.lastIndex = end + 1;
  }
  const raw = src.split("\n");
  const hits = [];
  for (const h of HAZARDS) {
    for (const re of h.res) {
      for (const x of code.matchAll(re)) {
        const line = lineOf(starts, x.index);
        if (!mask[line]) hits.push({ line: line + 1, id: h.id, snippet: raw[line].trim().slice(0, 100) });
      }
    }
  }
  hits.sort((a, b) => a.line - b.line || a.id.localeCompare(b.id));
  return { hits, mods, skipped: false };
}

// ---- the tree ----
const posix = (p) => p.split("\\").join("/");
function walk(dir) {
  return readdirSync(dir, { withFileTypes: true }).flatMap((e) => {
    const p = join(dir, e.name);
    return e.isDirectory() ? walk(p) : e.name.endsWith(".rs") ? [p] : [];
  });
}
const isTestPath = (inSrc) => /(?:^|\/)tests\//.test(inSrc) || /(?:^|\/)(?:tests|[^/]+_tests)\.rs$/.test(inSrc);
const isBinPath = (inSrc) => inSrc === "main.rs" || inSrc.startsWith("bin/");

// repo-relative file -> { counts: { id: n }, hits: [{ line, id, snippet }] } for every scanned file with a hit.
export function scanTree(root, { crates = WEB_CRATES, bins = WEB_BINS } = {}) {
  const files = new Map();
  for (const name of crates) {
    const srcDir = join(root, "crates", name, "src");
    if (!existsSync(srcDir)) continue;
    const scans = walk(srcDir).sort().map((file) => ({ file, rel: posix(relative(root, file)), inSrc: posix(relative(srcDir, file)), scan: scanText(readFileSync(file, "utf8")) }));
    // A false `mod x;` declaration takes its file x.rs, x/mod.rs and the whole directory x/ out of the scan.
    const offFiles = new Set();
    const offDirs = [];
    for (const { file, scan } of scans) {
      const stem = basename(file, ".rs");
      const home = ["lib", "main", "mod"].includes(stem) ? dirname(file) : join(dirname(file), stem);
      for (const mod of scan.mods) {
        offFiles.add(posix(join(home, `${mod}.rs`)));
        offDirs.push(`${posix(join(home, mod))}/`);
      }
    }
    for (const { file, rel, inSrc, scan } of scans) {
      const path = posix(file);
      if (scan.skipped || !scan.hits.length || isTestPath(inSrc)) continue;
      if (offFiles.has(path) || offDirs.some((d) => path.startsWith(d))) continue;
      if (isBinPath(inSrc) && !bins.some((b) => rel === b || rel.startsWith(`${b}/`))) continue;
      const counts = {};
      for (const h of scan.hits) counts[h.id] = (counts[h.id] ?? 0) + 1;
      files.set(rel, { counts, hits: scan.hits });
    }
  }
  return files;
}

// Hits per crate dir and hazard.
export function totals(files) {
  const out = new Map();
  for (const [rel, { counts }] of files) {
    const dir = rel.split("/")[1];
    const row = out.get(dir) ?? Object.fromEntries(HAZARDS.map((h) => [h.id, 0]));
    for (const [id, n] of Object.entries(counts)) row[id] += n;
    out.set(dir, row);
  }
  return out;
}
export function table(files) {
  const rows = totals(files);
  const ids = HAZARDS.map((h) => h.id);
  const sum = Object.fromEntries(ids.map((id) => [id, 0]));
  const line = (name, row) => `${name.padEnd(12)}${ids.map((id) => String(row[id]).padStart(8)).join("")}${String(ids.reduce((s, id) => s + row[id], 0)).padStart(8)}`;
  const body = [...rows].sort(([a], [b]) => a.localeCompare(b)).map(([dir, row]) => { for (const id of ids) sum[id] += row[id]; return line(dir, row); });
  return [`${"crate".padEnd(12)}${ids.map((id) => id.padStart(8)).join("")}${"total".padStart(8)}`, ...body, line("all", sum)].join("\n");
}

// ---- the ratchet ----
export function snapshot(files) {
  const out = {};
  for (const rel of [...files.keys()].sort()) out[rel] = Object.fromEntries(Object.entries(files.get(rel).counts).sort(([a], [b]) => a.localeCompare(b)));
  return out;
}
// Regressions: a hazard count above the baseline, or a file the baseline does not list. Improvements: counts below it.
export function compare(baseline, files) {
  const base = baseline.files ?? {};
  const regressions = [];
  const improvements = [];
  for (const [file, { counts, hits }] of files) {
    for (const [id, now] of Object.entries(counts)) {
      const was = base[file]?.[id] ?? 0;
      if (now > was) regressions.push({ file, id, was, now, fresh: !base[file], hits: hits.filter((h) => h.id === id) });
    }
  }
  for (const [file, ids] of Object.entries(base)) {
    for (const [id, was] of Object.entries(ids)) {
      const now = files.get(file)?.counts[id] ?? 0;
      if (now < was) improvements.push({ file, id, was, now });
    }
  }
  return { regressions, improvements };
}
export const regressionText = (r) => `${r.fresh ? "new file with hazards" : "count rose"} ${r.file}: ${r.id} ${r.was} -> ${r.now} (${WHAT[r.id]})`;
// The baseline with every improved count lowered (zero removes the entry); it never adds or raises.
export function tightened(baseline, files) {
  const out = {};
  for (const [file, ids] of Object.entries(baseline.files ?? {})) {
    for (const [id, was] of Object.entries(ids)) {
      const now = Math.min(was, files.get(file)?.counts[id] ?? 0);
      if (now > 0) (out[file] ??= {})[id] = now;
    }
  }
  return out;
}

const NOTE = "Ratchet for tools/wasm-gate.mjs: per file and hazard, the count of calls that break in a browser. Counts only go down (--tighten); a file that is not listed must have none.";
const writeBaseline = (path, filesObj) => writeFileSync(path, `${JSON.stringify({ version: 1, note: NOTE, files: filesObj }, null, 1)}\n`);

export const HELP = `wasm-gate.mjs - calls in the web-path crates that type-check for wasm32-unknown-unknown but break in a browser

  node tools/wasm-gate.mjs            totals per crate + ratchet against ${BASELINE} (exit 1 on a regression)
  node tools/wasm-gate.mjs --list     also every hit as file:line
  node tools/wasm-gate.mjs --tighten  lower baseline counts that improved
  node tools/wasm-gate.mjs --init     write the first baseline (refuses when one exists)

Hazards:
${HAZARDS.map((h) => `  ${h.id.padEnd(7)}${h.what}`).join("\n")}

Exit codes: 0 ok - 1 ratchet regression - 2 usage.
`;

export function main(argv, { root = REPO, out = console.log } = {}) {
  if (argv.includes("--help") || argv.includes("-h")) { out(HELP.trimEnd()); return 0; }
  const opts = { list: false, tighten: false, init: false };
  for (const a of argv) {
    if (a === "--list") opts.list = true;
    else if (a === "--tighten") opts.tighten = true;
    else if (a === "--init") opts.init = true;
    else throw new Error(`unknown argument: ${a}`);
  }
  const files = scanTree(root);
  const path = join(root, BASELINE);
  out(table(files));
  if (opts.list) {
    for (const [rel, { hits }] of files) for (const h of hits) out(`${rel}:${h.line}: [${h.id}] ${h.snippet}`);
  }
  if (opts.init) {
    if (existsSync(path)) throw new Error(`${BASELINE} exists; use --tighten`);
    writeBaseline(path, snapshot(files));
    out(`RESULT wrote ${BASELINE} for ${files.size} file(s)`);
    return 0;
  }
  if (!existsSync(path)) throw new Error(`${BASELINE} is missing; run --init once`);
  const baseline = JSON.parse(readFileSync(path, "utf8"));
  const { regressions, improvements } = compare(baseline, files);
  for (const r of regressions) {
    out(`[FAIL] ratchet ${regressionText(r)}`);
    for (const h of r.hits) out(`         ${r.file}:${h.line}: ${h.snippet}`);
  }
  for (const i of improvements) out(`[IMPROVED] ${i.file}: ${i.id} ${i.was} -> ${i.now}`);
  if (opts.tighten) {
    if (improvements.length) writeBaseline(path, tightened(baseline, files));
    out(`RESULT tightened ${improvements.length} count(s)`);
    return regressions.length ? 1 : 0;
  }
  if (improvements.length) out(`[INFO] ${improvements.length} count(s) fell below the baseline: run --tighten with the landing`);
  out(regressions.length ? `RESULT FAIL: ${regressions.length} ratchet regression(s)` : `RESULT PASS: 0 ratchet regressions over ${files.size} file(s) with hazards`);
  return regressions.length ? 1 : 0;
}

if (import.meta.main) {
  try { process.exitCode = main(process.argv.slice(2)); }
  catch (error) { console.error(`RESULT FAIL: ${error.message}`); process.exitCode = 2; }
}
