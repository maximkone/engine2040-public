// unwired-code-gate.mjs - print every public module of a crate that no other
// crate calls.
//
// Why this tool and not wiring-scan.mjs (the sibling in tools/): wiring-scan
// finds pub ITEMS (fn/struct/enum/trait/const) whose name appears in no other
// file. That cannot see a module the whole crate uses on itself: `crates/ai`
// calls `crate::utility::pick_gated` from inside `crates/ai`, so every item
// looks "used" and the island list stays empty while the module is
// unreachable from the product. AD5 (the diff gate in wiring-scan) only looks
// at NEW items in a diff, so the shelf that already exists never gets checked.
//
// This tool answers a different question: for every `pub mod` a crate exports,
// does any OTHER crate reach it? A module with no outside caller is dead
// public API: the tree carries it, the tests pass, nothing ships it.
//
// Two report classes:
//   no-caller          nobody outside the crate calls it, in any build.
//   behind-features:x  the only callers sit behind a feature that the default
//                      build does not turn on (or the module itself does), so
//                      the default build never reaches it either.
// A module whose own `#[cfg(feature = ...)]` is off by default is still
// reported: the tree ships it and nothing default-built calls it.
//
// The build that counts is the DEFAULT one: `cargo` turns on the `default`
// feature and nothing else, and no crate in this workspace declares a
// `default` key, so every `#[cfg(feature = "x")]` module is off by default.
//
// Donor shape: TimonPost/cargo-unused-features (MIT) - the feature side of
// the same question. Read live 2026-10-20, LICENSE MIT
// (https://github.com/TimonPost/cargo-unused-features/blob/main/LICENSE).
// We port the idea (walk the feature graph, report what nothing reaches) and
// vendor no bytes. No new dependency: node builtins only.
//
// usage:
//   node tools/unwired-code-gate.mjs [crates-root] [--json] [--any-features]
//                                       [--crate=DIR] [--reason=KIND] [--fail]
//                                       [--self-test] [--help]
//   default        report every dead public module, exit 0
//   --json         one JSON line instead of text lines
//   --any-features count a caller behind a feature flag as a caller
//   --crate=DIR    only this crate directory
//   --reason=KIND  only no-caller or behind-features
//   --fail         exit 1 when at least one module is reported
//   --self-test    prove the scan finds the known dead modules and does not
//                  flag a module that has a caller; exit 1 on any failure

import { existsSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, relative, resolve } from "node:path";

// ---------------------------------------------------------------- text helpers

// Strip block comments and line comments. String literals are tracked so a
// "//" inside a string does not eat the rest of the line.
export function stripComments(src) {
  let out = "";
  let i = 0;
  let inStr = false;
  let inChar = false;
  let inLine = false;
  let inBlock = false;
  let inRaw = false;
  let rawHash = 0;
  while (i < src.length) {
    const c = src[i];
    const n = src[i + 1];
    if (inLine) {
      if (c === "\n") { inLine = false; out += c; } else { out += " "; }
      i += 1;
      continue;
    }
    if (inBlock) {
      if (c === "*" && n === "/") { inBlock = false; out += "  "; i += 2; continue; }
      out += c === "\n" ? "\n" : " ";
      i += 1;
      continue;
    }
    if (inRaw) {
      if (c === '"' && src.slice(i + 1, i + 1 + rawHash).split('"').length - 1 === 0) inRaw = false;
      out += c;
      i += 1;
      continue;
    }
    if (inStr) {
      out += c;
      if (c === "\\") { out += n ?? ""; i += 2; continue; }
      if (c === '"') inStr = false;
      i += 1;
      continue;
    }
    if (inChar) {
      out += c;
      if (c === "\\") { out += n ?? ""; i += 2; continue; }
      if (c === "'") inChar = false;
      i += 1;
      continue;
    }
    if (c === "/" && n === "/") { inLine = true; out += "  "; i += 2; continue; }
    if (c === "/" && n === "*") { inBlock = true; out += "  "; i += 2; continue; }
    if (c === "r" && (n === '"' || n === "#")) {
      let j = i + 1;
      let hashes = 0;
      while (src[j] === "#") { hashes += 1; j += 1; }
      if (src[j] === '"') { inRaw = true; rawHash = hashes; out += src.slice(i, j + 1); i = j + 1; continue; }
    }
    if (c === '"') { inStr = true; out += c; i += 1; continue; }
    if (c === "'" && n !== undefined) { inChar = true; out += c; i += 1; continue; }
    out += c;
    i += 1;
  }
  return out;
}

// Production text only: everything before the first `#[cfg(test)]` ITEM.
// The cut skips a `#[cfg(test)]` that guards a `static` (the shared counting
// allocator every crate puts at the top of lib.rs), because cutting there
// would throw away the whole public module list of the crate.
export function prodText(src) {
  const lines = src.split("\n");
  let cut = -1;
  for (let i = 0; i < lines.length; i += 1) {
    if (!lines[i].trim().startsWith("#[cfg(test)]")) continue;
    let j = i + 1;
    while (j < lines.length && /^\s*#\[/.test(lines[j])) j += 1;
    const next = (lines[j] ?? "").trim();
    if (/^(pub\s+)?(mod|fn)\b/.test(next) || next.startsWith("#[test]")) { cut = i; break; }
  }
  return cut < 0 ? src : lines.slice(0, cut).join("\n");
}

// ---------------------------------------------------------------- cargo files

export function parsePackageName(cargoText) {
  const m = /^\s*name\s*=\s*"([^"]+)"/m.exec(cargoText);
  return m ? m[1] : null;
}

// The `[features]` table: name -> feature names it turns on. `dep:` and
// `pkg:` entries are not features and are dropped.
export function parseFeatures(cargoText) {
  const at = cargoText.indexOf("[features]");
  if (at < 0) return { features: {}, defaultOn: [] };
  const rest = cargoText.slice(at + "[features]".length);
  const end = rest.search(/\n\[/);
  const body = end < 0 ? rest : rest.slice(0, end);
  const features = {};
  const lines = body.split("\n");
  let current = null;
  let buf = "";
  for (const raw of lines) {
    const line = raw.trim();
    if (line === "" || line.startsWith("#")) continue;
    const head = /^([A-Za-z0-9_.-]+)\s*=\s*(.*)$/.exec(line);
    if (head && !line.startsWith("[")) {
      if (current) { features[current] = buf; buf = ""; }
      current = head[1];
      buf = head[2];
    } else if (current) {
      buf += " " + line;
    }
  }
  if (current) features[current] = buf;
  const clean = {};
  for (const [k, v] of Object.entries(features)) {
    const names = [];
    const inner = v.match(/\[(.*)\]/);
    if (inner) {
      for (const part of inner[1].split(",")) {
        const t = part.trim().replace(/^"|"$/g, "");
        if (t === "" || t.startsWith("dep:") || t.startsWith("pkg:")) continue;
        names.push(t);
      }
    }
    clean[k] = names;
  }
  const defaultOn = [];
  const seen = new Set();
  const add = (name) => {
    if (seen.has(name)) return;
    seen.add(name);
    defaultOn.push(name);
    for (const dep of clean[name] ?? []) add(dep);
  };
  for (const name of clean.default ?? []) add(name);
  return { features: clean, defaultOn };
}

// ---------------------------------------------------------------- cfg parsing

// A cfg attribute becomes: feature groups (AND across groups, OR inside one),
// a `not` list and a `test` flag.
export function parseCfg(attr) {
  const groups = [];
  const notFeatures = [];
  let test = false;
  const text = attr.replace(/\s+/g, " ");
  const takeTerms = (inner) => {
    const terms = [];
    let depth = 0;
    let buf = "";
    for (const ch of inner) {
      if (ch === "(") { depth += 1; buf += ch; continue; }
      if (ch === ")") { depth -= 1; buf += ch; continue; }
      if (ch === "," && depth === 0) { terms.push(buf); buf = ""; continue; }
      buf += ch;
    }
    if (buf.trim() !== "") terms.push(buf);
    return terms;
  };
  const walk = (expr) => {
    const e = expr.trim();
    if (e === "") return;
    if (e.startsWith("all(")) {
      for (const t of takeTerms(e.slice(4, e.lastIndexOf(")")))) walk(t);
      return;
    }
    if (e.startsWith("any(")) {
      const names = [];
      for (const t of takeTerms(e.slice(4, e.lastIndexOf(")")))) {
        const f = /feature\s*=\s*"([^"]+)"/.exec(t);
        if (f) names.push(f[1]);
        else if (/\btest\b/.test(t)) test = true;
      }
      if (names.length > 0) groups.push(names);
      return;
    }
    if (e.startsWith("not(")) {
      const inner = e.slice(4, e.lastIndexOf(")"));
      const f = /feature\s*=\s*"([^"]+)"/.exec(inner);
      if (f) notFeatures.push(f[1]);
      return;
    }
    const f = /feature\s*=\s*"([^"]+)"/.exec(e);
    if (f) { groups.push([f[1]]); return; }
    if (/\btest\b/.test(e)) test = true;
  };
  walk(text);
  return { groups, notFeatures, test };
}

// The cfg is satisfied by the default build when every group has one member
// the default build turns on, and no `not` member is on by default.
export function cfgSatisfiedByDefault(cfg, defaultOn) {
  if (cfg.test) return false;
  const on = new Set(defaultOn);
  for (const g of cfg.groups) {
    if (!g.some((f) => on.has(f))) return false;
  }
  for (const f of cfg.notFeatures) {
    if (on.has(f)) return false;
  }
  return true;
}

// The file is never compiled in the default build (its features are off).
export function cfgNeverOn(cfg, defaultOn) {
  const on = new Set(defaultOn);
  if (cfg.test) return true;
  for (const g of cfg.groups) {
    if (!g.some((f) => on.has(f))) return true;
  }
  return false;
}

// ---------------------------------------------------------------- the scan

function readText(file) {
  try { return readFileSync(file, "utf8"); } catch { return ""; }
}

// The module tree of one crate: every .rs file under src/ with the feature
// set it needs. `mod` lines are matched with the attribute lines above them,
// so a `#[cfg(feature = "x")] pub mod y;` is known to be behind x.
export function crateFiles(crateDir, crateInfo) {
  const files = new Map();
  const root = join(crateDir, "src", "lib.rs");
  const seen = new Set();
  const walk = (file, cfg) => {
    if (seen.has(file)) return;
    seen.add(file);
    let raw;
    try { raw = readFileSync(file, "utf8"); } catch { return; }
    const text = stripComments(raw);
    files.set(file, { text, prod: prodText(text), cfg });
    const lines = text.split("\n");
    for (let i = 0; i < lines.length; i += 1) {
      const m = /^[ \t]*(?:pub(?:\([^)]*\))?[ \t]+)?mod[ \t]+([A-Za-z_][A-Za-z0-9_]*)[ \t]*;/.exec(lines[i]);
      if (!m) continue;
      const name = m[1];
      let attr = "";
      for (let j = i - 1; j >= 0; j -= 1) {
        const l = lines[j].trim();
        if (l.startsWith("#[")) { attr = l + " " + attr; continue; }
        break;
      }
      const childCfg = attr.includes("cfg(") ? parseCfg(attr) : { groups: [], notFeatures: [], test: false };
      const dir = dirname(file);
      const flat = join(dir, name + ".rs");
      const nested = join(dir, name, "mod.rs");
      if (existsSync(flat)) walk(flat, childCfg);
      else if (existsSync(nested)) walk(nested, childCfg);
    }
  };
  walk(root, { groups: [], notFeatures: [], test: false });
  return files;
}

// The `pub use` lines of the crate root: module -> the names it re-exports.
export function rootReExports(libText) {
  const map = new Map();
  const re = /pub[ \t]+use[ \t]+([^;]+);/g;
  let m;
  while ((m = re.exec(libText)) !== null) {
    const path = m[1].replace(/\s+/g, " ").trim();
    const segs = path.split("::").map((s) => s.trim());
    let idx = 0;
    while (idx < segs.length && (segs[idx] === "crate" || segs[idx] === "self" || segs[idx] === "super")) idx += 1;
    if (idx >= segs.length) continue;
    const head = segs[idx];
    if (!/^[A-Za-z_][A-Za-z0-9_]*$/.test(head)) continue;
    const tail = segs.slice(idx + 1);
    if (tail.length === 0) continue;
    const last = tail[tail.length - 1];
    const names = [];
    if (last.startsWith("{")) {
      const inner = last.replace(/^\{|\}$/g, "");
      for (const part of inner.split(",")) {
        const t = part.trim().split(/\s+as\s+/)[0].trim();
        if (/^[A-Za-z_][A-Za-z0-9_]*$/.test(t)) names.push(t);
      }
    } else if (/^[A-Za-z_][A-Za-z0-9_]*$/.test(last)) {
      names.push(last);
    }
    if (names.length === 0) continue;
    const cur = map.get(head) ?? [];
    for (const n of names) if (!cur.includes(n)) cur.push(n);
    map.set(head, cur);
  }
  return map;
}

// The public modules a crate exports.
export function publicModules(crateDir) {
  const root = join(crateDir, "src", "lib.rs");
  const prod = prodText(stripComments(readText(root)));
  const out = [];
  const lines = prod.split("\n");
  for (let i = 0; i < lines.length; i += 1) {
    const m = /^[ \t]*pub(?:\([^)]*\))?[ \t]+mod[ \t]+([A-Za-z_][A-Za-z0-9_]*)[ \t]*;/.exec(lines[i]);
    if (!m) continue;
    let attr = "";
    for (let j = i - 1; j >= 0; j -= 1) {
      const l = lines[j].trim();
      if (l.startsWith("#[")) { attr = l + " " + attr; continue; }
      break;
    }
    const cfg = attr.includes("cfg(") ? parseCfg(attr) : { groups: [], notFeatures: [], test: false };
    const name = m[1];
    const dir = dirname(root);
    const flat = join(dir, name + ".rs");
    const nested = join(dir, name, "mod.rs");
    const file = existsSync(flat) ? flat : existsSync(nested) ? nested : null;
    out.push({ name, file, cfg });
  }
  return out;
}

function identOf(pkg) {
  return pkg.replace(/-/g, "_");
}

// Is `text` a caller of `module` of the crate with the given idents?
// The shapes: `crate_ident::module`, `crate_ident::module::item`,
// `crate_ident::re_exported_item` and the group import
// `crate_ident::{module, other}`.
export function mentions(text, idents, module, reExported) {
  const words = new Set([module, ...reExported]);
  for (const ident of idents) {
    for (const word of words) {
      const re = new RegExp("(?<![A-Za-z0-9_])" + ident + "::" + word + "(?![A-Za-z0-9_])");
      if (re.test(text)) return true;
    }
    const group = new RegExp("(?<![A-Za-z0-9_])" + ident + "::\\{([^}]*)\\}");
    const gm = group.exec(text);
    if (gm) {
      for (const part of gm[1].split(",")) {
        const t = part.trim().split(/\s+as\s+/)[0].trim();
        if (words.has(t)) return true;
      }
    }
  }
  return false;
}

// The whole scan. Returns the report rows and the counts.
export function scan(cratesRoot, options = {}) {
  const anyFeatures = options.anyFeatures === true;
  const dirs = [];
  for (const entry of readdirSync(cratesRoot, { withFileTypes: true })) {
    if (!entry.isDirectory()) continue;
    const dir = join(cratesRoot, entry.name);
    if (!existsSync(join(dir, "Cargo.toml"))) continue;
    dirs.push({ dir, name: entry.name });
  }
  dirs.sort((a, b) => (a.name < b.name ? -1 : a.name > b.name ? 1 : 0));

  const crates = [];
  for (const d of dirs) {
    const cargo = readText(join(d.dir, "Cargo.toml"));
    const pkg = parsePackageName(cargo);
    if (!pkg) continue;
    const { defaultOn } = parseFeatures(cargo);
    const info = { dir: d.dir, name: d.name, pkg, idents: [identOf(pkg)], defaultOn };
    if (d.name !== pkg) info.idents.push(d.name.replace(/-/g, "_"));
    info.files = crateFiles(d.dir, info);
    info.modules = publicModules(d.dir);
    info.reexports = rootReExports(prodText(stripComments(readText(join(d.dir, "src", "lib.rs")))));
    crates.push(info);
  }
  const byName = new Map(crates.map((c) => [c.name, c]));

  const rows = [];
  let publicCount = 0;
  for (const c of crates) {
    for (const mod of c.modules) {
      publicCount += 1;
      const reExported = c.reexports.get(mod.name) ?? [];
      const callers = [];
      for (const other of crates) {
        if (other.dir === c.dir) continue;
        for (const [file, info] of other.files) {
          if (info.cfg.test) continue; // test-only code is not a consumer
          // The caller writes the OWNER's path: `engine_ai::behavior::tick`.
          if (!mentions(info.prod, c.idents, mod.name, reExported)) continue;
          callers.push({ crate: other.name, file, cfg: info.cfg });
        }
      }
      const live = anyFeatures
        ? callers
        : callers.filter((x) => cfgSatisfiedByDefault(x.cfg, byName.get(x.crate).defaultOn));
      if (live.length > 0) continue;
      const moduleBehind = !cfgSatisfiedByDefault(mod.cfg, c.defaultOn);
      const behind = new Set();
      if (moduleBehind) for (const g of mod.cfg.groups) for (const f of g) behind.add(f);
      for (const x of callers) for (const g of x.cfg.groups) for (const f of g) behind.add(f);
      rows.push({
        crate: c.name,
        pkg: c.pkg,
        module: mod.name,
        file: mod.file ? relative(cratesRoot, mod.file).split("\\").join("/") : null,
        // no-caller: nobody outside the crate calls it, in any build.
        // behind-features:x: the only callers (or the module itself) sit behind
        // a feature that the default build does not turn on.
        reason: callers.length === 0
          ? (moduleBehind ? "behind-features:" + [...behind].sort().join(",") : "no-caller")
          : "behind-features:" + [...behind].sort().join(","),
        callers: callers.map((x) => x.crate + ":" + relative(cratesRoot, x.file).split("\\").join("/")).sort(),
      });
    }
  }
  rows.sort((a, b) => (a.crate + a.module < b.crate + b.module ? -1 : a.crate + b.module > b.crate + b.module ? 1 : 0));
  return { rows, publicCount, crateCount: crates.length };
}

// ---------------------------------------------------------------- self-test

// The known dead modules on today's tree. `nav::crowd` is the board row name
// (NV10 owns `crates/nav/src/crowd.rs`); the file that exists is orca.rs.
const EXPECT_DEAD = [
  { crate: "ai", module: "behavior", alias: "ai::behavior" },
  { crate: "ai", module: "utility", alias: "ai::utility" },
  { crate: "ai", module: "goap", alias: "ai::goap" },
  { crate: "ai", module: "blackboard", alias: "ai::blackboard" },
  { crate: "ai", module: "goalstack", alias: "ai::goalstack" },
  { crate: "ai", module: "director", alias: "ai::director" },
  { crate: "ai", module: "opponent", alias: "ai::opponent" },
  { crate: "nav", module: "orca", alias: "nav::crowd" },
  { crate: "anim", module: "lod", alias: "anim::lod" },
  { crate: "observe", module: "rerun_sink", alias: "observe::rerun_sink" },
  { crate: "render", module: "heal", alias: "RenderHeal" },
  { crate: "assets", module: "import_repair", alias: "assets::import_repair" },
];

// Modules with a real caller today: the scan must not flag them at all.
const EXPECT_LIVE = [
  { crate: "ai", module: "melee", why: "crates/arena/src/engage.rs imports engine_ai::MeleeView" },
  { crate: "ai", module: "tier", why: "crates/arena/src/fork.rs uses engine_ai::tier" },
];

// A tiny tree: `demo::used` is called by another crate, `demo::unused` is not.
function writeFixture(root) {
  const mk = (p, s) => { mkdirSync(dirname(p), { recursive: true }); writeFileSync(p, s); };
  mk(join(root, "crates", "demo", "Cargo.toml"), '[package]\nname = "demo"\nversion = "0.1.0"\nedition = "2021"\n');
  mk(join(root, "crates", "demo", "src", "lib.rs"), "//! demo crate.\npub mod used;\npub mod unused;\n");
  mk(join(root, "crates", "demo", "src", "used.rs"), "//! used.\npub fn thing() -> u32 { 1 }\n");
  mk(join(root, "crates", "demo", "src", "unused.rs"), "//! unused.\npub fn thing() -> u32 { 2 }\n");
  mk(join(root, "crates", "other", "Cargo.toml"), '[package]\nname = "other"\nversion = "0.1.0"\nedition = "2021"\n');
  mk(join(root, "crates", "other", "src", "lib.rs"), "//! other crate.\npub fn go() -> u32 { demo::used::thing() }\n");
  return root;
}

export function selfTest(cratesRoot) {
  const lines = [];
  let failed = 0;
  const scan1 = scan(cratesRoot);
  const key = (r) => r.crate + "::" + r.module;
  const found = new Map(scan1.rows.map((r) => [key(r), r]));

  for (const want of EXPECT_DEAD) {
    const hit = found.get(want.crate + "::" + want.module);
    if (hit) {
      lines.push("find " + want.alias + " " + hit.reason);
    } else {
      failed += 1;
      lines.push("MISS " + want.alias + " not reported");
    }
  }
  for (const live of EXPECT_LIVE) {
    if (found.has(live.crate + "::" + live.module)) {
      failed += 1;
      lines.push("FALSE-POSITIVE " + live.crate + "::" + live.module + " has a caller: " + live.why);
    } else {
      lines.push("not-flagged " + live.crate + "::" + live.module + " ok");
    }
  }

  const fixRoot = mkdtempSync(join(tmpdir(), "unwired-gate-"));
  try {
    writeFixture(fixRoot);
    const fix = scan(join(fixRoot, "crates"));
    const fixKeys = fix.rows.map(key);
    if (!fixKeys.includes("demo::unused")) { failed += 1; lines.push("FIXTURE-MISS demo::unused not reported"); }
    else lines.push("find demo::unused no-caller");
    if (fixKeys.includes("demo::used")) { failed += 1; lines.push("FIXTURE-FALSE-POSITIVE demo::used flagged"); }
    else lines.push("not-flagged demo::used ok");
    if (fix.publicCount !== 2) { failed += 1; lines.push("FIXTURE-MISS public modules seen: " + fix.publicCount + " want 2"); }
    else lines.push("fixture-public-modules 2 ok");
  } finally {
    rmSync(fixRoot, { recursive: true, force: true });
  }

  const twice = scan(cratesRoot);
  if (JSON.stringify(twice.rows) !== JSON.stringify(scan1.rows)) {
    failed += 1;
    lines.push("UNSTABLE two scans gave different rows");
  } else {
    lines.push("stable two scans equal");
  }

  for (const l of lines) process.stdout.write(l + "\n");
  if (failed === 0) {
    process.stdout.write("PASS unwired-code-gate: " + EXPECT_DEAD.length + " known dead modules found, " + EXPECT_LIVE.length + " live modules not flagged, fixture ok\n");
    return 0;
  }
  process.stdout.write("FAIL unwired-code-gate: " + failed + " check(s) failed\n");
  return 1;
}

// ---------------------------------------------------------------- main

export function helpText() {
  return [
    "unwired-code-gate.mjs - print every public module no other crate calls.",
    "usage: node tools/unwired-code-gate.mjs [crates-root] [--json] [--any-features] [--crate=DIR] [--reason=KIND] [--fail] [--self-test] [--help]",
    "  crates-root     folder of crates to scan (default: crates).",
    "  --json          one JSON line instead of text lines.",
    "  --any-features  count a caller behind a feature flag as a caller.",
    "  --crate=DIR     only this crate directory.",
    "  --reason=KIND   only no-caller or behind-features.",
    "  --fail          exit 1 when at least one module is reported.",
    "  --self-test     prove the known dead modules are found and a live one is not.",
    "  --help          print this and exit 0.",
    "the default build counts: a crate with no `default` feature key enables nothing,",
    "so a caller behind `#[cfg(feature = ...)]` is reported as behind-features.",
    "",
  ].join("\n");
}

if (import.meta.main) {
  const raw = process.argv.slice(2);
  let wantJson = false;
  let wantAny = false;
  let wantFail = false;
  let wantHelp = false;
  let wantSelfTest = false;
  let crateFilter = "";
  let reasonFilter = "";
  let arg = "crates";
  let picked = false;
  for (const a of raw) {
    if (a === "--json") wantJson = true;
    else if (a === "--any-features") wantAny = true;
    else if (a === "--fail") wantFail = true;
    else if (a === "--self-test") wantSelfTest = true;
    else if (a === "--help" || a === "-h") wantHelp = true;
    else if (a.startsWith("--crate=")) crateFilter = a.slice(8);
    else if (a.startsWith("--reason=")) reasonFilter = a.slice(9);
    else if (a.startsWith("--")) continue;
    else if (!picked) { arg = a; picked = true; }
  }
  if (wantHelp) {
    process.stdout.write(helpText());
    process.exitCode = 0;
  } else {
    const root = resolve(arg);
    if (!existsSync(root)) {
      process.stdout.write("FAIL unwired-code-gate: no crates root at " + root + "\n");
      process.exitCode = 1;
    } else if (wantSelfTest) {
      process.exitCode = selfTest(root);
    } else {
      const result = scan(root, { anyFeatures: wantAny });
      let rows = result.rows;
      if (crateFilter) rows = rows.filter((r) => r.crate === crateFilter);
      if (reasonFilter) rows = rows.filter((r) => r.reason.startsWith(reasonFilter));
      if (wantJson) {
        process.stdout.write(JSON.stringify({ ...result, rows }) + "\n");
      } else {
        for (const r of rows) {
          process.stdout.write("unwired-module " + r.crate + " " + r.module + " " + (r.file ?? "(no file)") + " " + r.reason + "\n");
        }
        const counts = {};
        for (const r of rows) counts[r.crate] = (counts[r.crate] || 0) + 1;
        const top = Object.entries(counts).sort((a, b) => (b[1] - a[1] || (a[0] < b[0] ? -1 : 1))).slice(0, 10);
        for (const [name, n] of top) process.stdout.write("top " + n + " " + name + "\n");
        process.stdout.write("SUMMARY unwired modules: " + rows.length + " of " + result.publicCount + " public modules in " + result.crateCount + " crates have no caller outside their own crate\n");
      }
      if (wantFail && rows.length > 0) process.exitCode = 1;
    }
  }
}
