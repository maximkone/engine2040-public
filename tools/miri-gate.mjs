#!/usr/bin/env node
// CQ15 Miri UB gate. The workspace lints `unsafe_code = "deny"`, so every `unsafe` block, fn, impl or extern is a
// reasoned `#[expect(unsafe_code)]` site. Miri is the tool that checks those sites for undefined behaviour, so this
// gate keeps three lists from drifting apart:
//   1. the unsafe sites in the source (scanned, strings and comments blanked first),
//   2. tools/miri-sites.json: per file with sites, the exact count and either the `cargo miri test` line that runs it
//      or a reason Miri cannot (foreign code, Windows-only),
//   3. .github/workflows/miri.yml: every Miri line in the map must be a step there, verbatim.
// It fails on a new or removed site not in the map, a Miri line the workflow does not run, and any suppression:
// `cfg(miri)` / `cfg_attr(miri ..)` in Rust (a test that skips itself under Miri), `-Zmiri-` flags or MIRIFLAGS and
// `continue-on-error` in the workflow. It also prints the forbid-rollout list: crates with no unsafe site at all
// that can take `#![forbid(unsafe_code)]` today.
//   node tools/miri-gate.mjs          audit + report (exit 1 on a problem)
//   node tools/miri-gate.mjs --list   also every site as file:line
import { existsSync, readdirSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { lex } from "./rust-quality.mjs";

const REPO = join(dirname(fileURLToPath(import.meta.url)), "..");
export const MAP = "tools/miri-sites.json";
export const WORKFLOW = ".github/workflows/miri.yml";
const MIRI_PREFIX = "cargo miri test -p ";

const SITE = /\bunsafe\b\s*(?:\{|impl\b|fn\b|extern\b|trait\b)/g;
const CFG_MIRI = /\bcfg(?:_attr)?!?\s*\([^\n]*?\bmiri\b/g;
const FORBID = /#!\[forbid\(\s*unsafe_code\s*\)\]/;

const lineOf = (code, offset) => code.slice(0, offset).split("\n").length;

// Unsafe sites and cfg(miri) suppressions of one Rust source text: [{ line }] each.
export function scanText(text) {
  const { code } = lex(text);
  const find = (re) => [...code.matchAll(re)].map((m) => ({ line: lineOf(code, m.index) }));
  return { sites: find(SITE), suppressions: find(CFG_MIRI), forbid: FORBID.test(code) };
}

const walk = (dir, out = []) => {
  for (const e of existsSync(dir) ? readdirSync(dir, { withFileTypes: true }) : []) {
    if (e.name === "target" || e.name === "node_modules" || e.name.startsWith(".")) continue;
    const p = join(dir, e.name);
    if (e.isDirectory()) walk(p, out);
    else if (e.name.endsWith(".rs")) out.push(p);
  }
  return out;
};

// Every Rust file of every crate: { crate: [{ file, sites, suppressions, forbid }] }.
export function scanTree(root) {
  const crates = {};
  const base = join(root, "crates");
  for (const name of existsSync(base) ? readdirSync(base) : []) {
    if (!existsSync(join(base, name, "Cargo.toml"))) continue;
    crates[name] = walk(join(base, name)).map((f) => ({
      file: f.slice(root.length + 1).split("\\").join("/"),
      ...scanText(readFileSync(f, "utf8")),
    }));
  }
  return crates;
}

// Problems of the map and the workflow against the scan. Empty when the three lists agree.
export function problems(crates, map, workflowText) {
  const out = [];
  const workflow = workflowText.split("\n").filter((l) => !l.trim().startsWith("#")).join("\n");
  const files = Object.values(crates).flat();
  const entries = map.sites ?? {};
  for (const f of files) {
    const n = f.sites.length;
    const want = entries[f.file];
    if (n && !want) out.push(`${f.file}: ${n} unsafe site(s) not in ${MAP}; add the file with a miri line or a skip reason`);
    else if (want && n && want.count !== n) out.push(`${f.file}: ${n} unsafe site(s) but ${MAP} says ${want.count}; audit the change, then fix the count`);
    for (const s of f.suppressions) out.push(`${f.file}:${s.line}: cfg(miri) is a suppression, the gate allows none`);
  }
  const known = new Set(files.map((f) => f.file));
  for (const [file, want] of Object.entries(entries)) {
    if (!known.has(file) || !files.find((f) => f.file === file).sites.length) out.push(`${MAP}: ${file} has no unsafe site any more; drop the entry`);
    const kinds = ["miri", "skip"].filter((k) => typeof want[k] === "string" && want[k].trim());
    if (kinds.length !== 1) out.push(`${MAP}: ${file} needs exactly one of "miri" (a cargo miri test line) or "skip" (why Miri cannot run it)`);
    if (want.miri && !want.miri.startsWith(MIRI_PREFIX)) out.push(`${MAP}: ${file} miri line must start with "${MIRI_PREFIX}"`);
    if (want.miri && !workflow.includes(want.miri)) out.push(`${WORKFLOW} does not run \`${want.miri}\` (${file})`);
  }
  for (const m of workflow.matchAll(/-Zmiri-[\w-]+|MIRIFLAGS|continue-on-error/g)) out.push(`${WORKFLOW}: ${m[0]} is a suppression, the gate allows none`);
  return out;
}

// Crates with no unsafe site whose lib.rs / main.rs does not forbid it yet.
export function forbidRollout(crates) {
  const rollout = [];
  for (const [name, files] of Object.entries(crates)) {
    const roots = files.filter((f) => /\/src\/(?:lib|main)\.rs$/.test(f.file));
    if (roots.length && files.every((f) => !f.sites.length) && !roots.every((f) => f.forbid)) rollout.push(name);
  }
  return rollout;
}

export function main(argv, { root = REPO, out = console.log } = {}) {
  const crates = scanTree(root);
  const read = (rel) => (existsSync(join(root, rel)) ? readFileSync(join(root, rel), "utf8") : null);
  const mapText = read(MAP);
  const workflow = read(WORKFLOW);
  if (mapText === null || workflow === null) { out(`missing ${mapText === null ? MAP : WORKFLOW}`); return 1; }
  const found = problems(crates, JSON.parse(mapText), workflow);
  const sites = Object.values(crates).flat().filter((f) => f.sites.length);
  out(`unsafe sites: ${sites.reduce((n, f) => n + f.sites.length, 0)} in ${sites.length} files`);
  for (const f of sites) out(`  ${f.file}: ${f.sites.length}${argv.includes("--list") ? ` (lines ${f.sites.map((s) => s.line).join(", ")})` : ""}`);
  out(`forbid-rollout (no unsafe, no #![forbid(unsafe_code)] yet): ${forbidRollout(crates).join(" ") || "none"}`);
  for (const p of found) out(`FAIL ${p}`);
  out(found.length ? `miri-gate: ${found.length} problem(s)` : "miri-gate: ok");
  return found.length ? 1 : 0;
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) process.exit(main(process.argv.slice(2)));
