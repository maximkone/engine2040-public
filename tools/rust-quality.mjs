#!/usr/bin/env node
// Rust quality scorecard and ratchet for every workspace crate.
// Measures production code (test modules, tests/, benches/ and examples/ are
// excluded except for `tests` and `board_ids`), scores each crate 0-100 against
// the top-tier bar in docs/RUST-QUALITY.md, and fails when any crate's debt
// count rises above tools/rust-quality.baseline.json. The baseline only goes
// down (--tighten); raising it needs --accept with a reason, which is logged.
//   node tools/rust-quality.mjs                 scorecard + ratchet, exit 1 on regression
//   node tools/rust-quality.mjs --crate ecs     one crate (repeatable)
//   node tools/rust-quality.mjs --list dup_fns --crate anim [--limit 40]
//   node tools/rust-quality.mjs --json
//   node tools/rust-quality.mjs --init          write the baseline once (refuses when it exists)
//   node tools/rust-quality.mjs --tighten       lower improved baseline counts (orchestrator)
//   node tools/rust-quality.mjs --accept <crate> --reason "<why>"   (orchestrator)
// (--limit only takes effect with --list.)
import { createHash } from "node:crypto";
import { existsSync, mkdirSync, readdirSync, readFileSync, writeFileSync } from "node:fs";
import { join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { OWN_MATH_FILES, deriveFixedApi, platformMathHits } from "./sim-source-scan.mjs";

// Authoritative simulation crates: determinism rules apply (HASH-LAW.md).
export const SIM_CRATES = ["engine-core", "engine-ecs", "engine-physics", "engine-combat", "engine-ai", "engine-nav", "engine-destruction", "engine-net", "engine-scene", "engine-anim", "engine-spatial", "engine-rules", "engine-arena"];
// Crates with a per-tick, per-frame or real-time path where steady-state allocation is a defect.
export const HOT_CRATES = [...SIM_CRATES, "engine-arena", "engine-render", "engine-audio", "engine-input"];

const BASELINE = "tools/rust-quality.baseline.json";
// Canonical clippy config for simulation crates (type-aware determinism bans).
const SIM_CLIPPY = "tools/sim-clippy.toml";
// Simulation crates still holding HashMap/Instant in production code; each
// adopts SIM_CLIPPY once its `nondet` count reaches zero, then leaves this list.
const SIM_CLIPPY_PENDING = [];
export const LONG_FN_LINES = 100;
export const BIG_FILE_LINES = 1500;
const DUP_MIN_TOKENS = 30;

// Per-file result cache under target/: warm reruns reuse metrics/hits/clones
// keyed by sha256(file text + JSON(METRICS, SIM_CLIPPY, LONG_FN_LINES,
// BIG_FILE_LINES)) per relative path. Fresh JS, zero vendor bytes.
// Donor: eslint file-hash result cache (MIT)
// https://github.com/eslint/eslint/blob/main/lib/cli-engine/lint-result-cache.js
export const CACHE_DIR = "target/rust-quality-cache";
const CACHE_VERSION = 3;

function cachePath(root, rel) {
  return join(root, CACHE_DIR, `${encodeURIComponent(rel)}.json`);
}

function configString(ids, fixedApi) {
  return JSON.stringify({
    v: CACHE_VERSION,
    fixed: Object.values(fixedApi).map((set) => [...set].sort()),
    metrics: METRICS,
    simClippy: SIM_CLIPPY,
    longFn: LONG_FN_LINES,
    bigFile: BIG_FILE_LINES,
    ids: [...ids].sort(),
  });
}

function kindTag(kind) {
  return `${kind.crate}|${kind.lib ? 1 : 0}${kind.bin ? 1 : 0}${kind.sim ? 1 : 0}${kind.hot ? 1 : 0}${kind.core ? 1 : 0}${kind.testFile ? 1 : 0}${kind.ownMath ? 1 : 0}`;
}

function fileKey(text, tag, config) {
  const h = createHash("sha256");
  h.update(text, "utf8");
  h.update("|", "utf8");
  h.update(config, "utf8");
  h.update("|", "utf8");
  h.update(tag, "utf8");
  return h.digest("hex");
}

function cacheRead(cacheFile, key) {
  try {
    const entry = JSON.parse(readFileSync(cacheFile, "utf8"));
    if (!entry || entry.key !== key) return null;
    if (!entry.metrics || !entry.hits || !entry.clones) return null;
    return { metrics: entry.metrics, hits: entry.hits, clones: entry.clones };
  } catch {
    return null;
  }
}

function cacheWrite(cacheFile, key, a) {
  try {
    writeFileSync(cacheFile, JSON.stringify({ key, metrics: a.metrics, hits: a.hits, clones: a.clones }));
  } catch {
    // Cache write failures never fail the run; next rerun re-analyzes.
  }
}

// Metric table: `bad` counts are ratcheted per crate and must not rise. The
// score interpolates each metric's density per 1,000 code lines (or ratio)
// between `target` (full weight) and `zeroAt` (no weight). Weights sum to 100.
export const METRICS = {
  unwrap: { weight: 8, target: 0, zeroAt: 1, bad: true, what: "`.unwrap()` in production code" },
  panic_sites: { weight: 6, target: 1, zeroAt: 5, bad: true, what: "`expect`/`panic!`/`unreachable!`/`todo!` in production code" },
  casts: { weight: 8, target: 5, zeroAt: 40, bad: true, what: "numeric `as` casts (use From/TryFrom or a named conversion)" },
  allow_no_reason: { weight: 4, target: 0, zeroAt: 1, bad: true, what: "`#[allow]`/`#[expect]` without `reason =`" },
  dead_code_allows: { weight: 3, target: 0, zeroAt: 0.5, bad: true, what: "`allow(dead_code)` hiding unconsumed code" },
  print_in_lib: { weight: 4, target: 0, zeroAt: 1, bad: true, what: "print!/println!/eprintln!/dbg! in library code" },
  string_errors: { weight: 5, target: 0, zeroAt: 1, bad: true, what: "`Result<_, String>` or `&'static str` errors" },
  board_ids: { weight: 7, target: 0, zeroAt: 5, bad: true, what: "board IDs or history in code comments (CODELAW 12)" },
  hash_reimpl: { weight: 6, target: 0, zeroAt: 1, bad: true, what: "FNV/SplitMix constants re-implemented outside engine-core" },
  dup_fns: { weight: 8, target: 0, zeroAt: 2, bad: true, what: "function bodies duplicated elsewhere in the workspace" },
  passthrough: { weight: 3, target: 0.3, zeroAt: 3, bad: true, what: "free functions that only forward their arguments" },
  long_fns: { weight: 7, target: 0.1, zeroAt: 2, bad: true, what: `functions over ${LONG_FN_LINES} lines` },
  big_files: { weight: 4, target: 0, zeroAt: 3, bad: true, perCrate: true, what: `files over ${BIG_FILE_LINES} production lines` },
  hot_alloc: { weight: 8, target: 0, zeroAt: 3, bad: true, what: "allocations inside step/tick/update/render/mix functions of hot crates" },
  nondet: { weight: 7, target: 0, zeroAt: 2, bad: true, what: "HashMap/HashSet/Instant/SystemTime/thread_rng in simulation crates" },
  float_math: { weight: 0, target: 0, zeroAt: 1, bad: true, what: "platform float math (sin, cos, exp, powf...) in simulation crates, the type-blind twin of tools/sim-clippy.toml" },
  waivers: { weight: 0, target: 0, zeroAt: 1, bad: true, what: "`det-ok:`/`alloc-ok:` waivers (budgeted, reviewed)" },
  undoc_pub: { weight: 4, target: 5, zeroAt: 50, bad: true, ratio: true, what: "public items without a doc comment (percent)" },
  unsafe: { weight: 0, target: 0, zeroAt: 1, bad: true, what: "`unsafe` uses (each needs SAFETY and a test)" },
  tests: { weight: 8, target: 10, zeroAt: 0, bad: false, what: "`#[test]` functions per 1,000 production code lines (higher is better)" },
};
// Metrics a crate entering the baseline must already have at zero.
const STRICT_NEW = ["unwrap", "allow_no_reason", "dead_code_allows", "print_in_lib", "string_errors", "board_ids", "hash_reimpl", "dup_fns", "nondet", "float_math"];

// One sentence per rule id: the exact edit that clears the finding, so an
// agent fixes a red gate without reading the rule source. Guidance only: the
// text never changes a count, a score or the ratchet.
export const RULE_FIXES = {
  unwrap: "replace `.unwrap()` with `?` in a function that returns `Result`, or with `expect(\"why it cannot fail\")` when the value is proven",
  panic_sites: "return a typed error (`Err(MyError::Case)`) or handle the case with `match` instead of `panic!`, `expect`, `unreachable!` or `todo!`",
  casts: "replace the `as` cast with `TryFrom` (`u32::try_from(x).map_err(MyError::Narrow)?`), or keep it under `#[expect(clippy::cast_possible_truncation, reason = \"the range is proven\")]`",
  allow_no_reason: "add the reason to the attribute: `#[expect(lint_name, reason = \"why\")]`",
  dead_code_allows: "delete `#[allow(dead_code)]` and either call the item from real code or delete the item",
  print_in_lib: "delete the `println!`/`eprintln!`/`dbg!` from the library and return the value or a typed error instead",
  string_errors: "replace the `String`/`&'static str` error with a typed error enum of the crate and return `Err(MyError::Case)`",
  board_ids: "delete the row id from the comment, the history belongs in git and not in the code",
  hash_reimpl: "use the engine-core owner `engine_core::{Fnv64, hash_bytes, splitmix64}` instead of the local constant or loop",
  dup_fns: "move the duplicated body to its owner crate (engine-core for a primitive) and call it from both callers",
  passthrough: "delete the forwarding wrapper and call the target function directly",
  long_fns: "split the function into named steps, each one under 100 lines",
  big_files: "move a section of the file into its own module (`mod name;` in lib.rs) and shrink the file under 1500 lines",
  hot_alloc: "reuse a buffer that lives on the struct (or a scratch arena) instead of allocating inside the hot function",
  nondet: "use a `BTreeMap` or a sorted `Vec` instead of `HashMap`/`HashSet`, and `engine_core::Tick` instead of `Instant::now`/`SystemTime`",
  float_math: "use the integer math of `engine_core::fixed::Fixed` (or plain integers) instead of `sin`/`cos`/`exp`/`powf`",
  waivers: "delete the `det-ok:`/`alloc-ok:` waiver and fix the nondeterminism or the allocation it excuses",
  undoc_pub: "add a `///` doc comment above the public item that says what it does",
  unsafe: "delete the `unsafe` block, or keep it with a `// SAFETY:` comment that names the invariant and a test that proves it",
  tests: "add a `#[test]` that fails when the new behaviour breaks",
};

// The fix of a rule id, and the tail a reported line carries: " fix: <text>".
// Every rule id in METRICS has a fix; the fallback keeps a line complete.
export function fixFor(ruleId) {
  return RULE_FIXES[ruleId] ?? `apply the ${ruleId} rule of docs/RUST-QUALITY.md (its edit is not written down yet)`;
}
export function fixTail(ruleId) {
  return ` fix: ${fixFor(ruleId)}`;
}

export const HELP = `rust-quality.mjs - Rust quality scorecard and ratchet for every workspace crate

Usage:
  node tools/rust-quality.mjs                 scorecard + ratchet, exit 1 on regression
  node tools/rust-quality.mjs --crate <name>   one crate (repeatable)
  node tools/rust-quality.mjs --list <metric> --crate <name> [--limit 40]
  node tools/rust-quality.mjs --json
  node tools/rust-quality.mjs --init          write the baseline once (refuses when it exists)
  node tools/rust-quality.mjs --tighten       lower improved baseline counts (orchestrator)
  node tools/rust-quality.mjs --accept <crate> --reason "<why>"   (orchestrator)
  node tools/rust-quality.mjs --owned <path>  ratchet only regressions touching owned paths (land)

Options:
  --crate <name>   score one crate (repeatable, also matches engine-<name>)
  --owned <path>   scope the ratchet to owned repo paths (repeatable, comma ok; foreign debt warns, owned debt still fails)
  --list <metric>  list hits for one metric (needs --limit to cap output)
  --limit <n>      cap listed hits (only with --list)
  --json           machine-readable scorecard
  --no-cache       bypass per-file cache under target/rust-quality-cache
  -h, --help       print this help and exit 0

Exit codes: 0 ok - 1 ratchet regression - 2 usage.
`;

// ---------------------------------------------------------------------------
// Lexing: blank strings and comments (same length, newlines kept) so regexes
// and brace matching only see code; comment text is kept per line.
// ---------------------------------------------------------------------------
export function lex(text) {
  const code = [];
  const comments = [];
  let line = 0;
  let i = 0;
  const n = text.length;
  const note = (s) => { comments[line] = (comments[line] ?? "") + s; };
  const ident = (ch) => ch !== undefined && /[A-Za-z0-9_]/.test(ch);
  while (i < n) {
    const c = text[i];
    const d = text[i + 1];
    if (c === "\n") { code.push("\n"); line++; i++; continue; }
    if (c === "/" && d === "/") {
      while (i < n && text[i] !== "\n") { note(text[i]); code.push(" "); i++; }
      continue;
    }
    if (c === "/" && d === "*") {
      let depth = 0;
      while (i < n) {
        if (text[i] === "/" && text[i + 1] === "*") { depth++; note("/*"); code.push(" ", " "); i += 2; continue; }
        if (text[i] === "*" && text[i + 1] === "/") { depth--; note("*/"); code.push(" ", " "); i += 2; if (depth === 0) break; continue; }
        if (text[i] === "\n") { code.push("\n"); line++; i++; continue; }
        note(text[i]); code.push(" "); i++;
      }
      continue;
    }
    if ((c === "r" || (c === "b" && d === "r")) && !ident(text[i - 1])) {
      const m = /^b?r(#*)"/.exec(text.slice(i, i + 300));
      if (m) {
        const close = `"${m[1]}`;
        for (let k = 0; k < m[0].length; k++) code.push(text[i + k]);
        i += m[0].length;
        while (i < n && text.slice(i, i + close.length) !== close) {
          if (text[i] === "\n") { code.push("\n"); line++; } else code.push(" ");
          i++;
        }
        for (let k = 0; k < close.length && i < n; k++, i++) code.push(text[i]);
        continue;
      }
    }
    if (c === '"') {
      code.push('"'); i++;
      while (i < n && text[i] !== '"') {
        if (text[i] === "\\") { code.push(" "); i++; if (text[i] === "\n") { code.push("\n"); line++; i++; continue; } }
        if (i >= n) break;
        if (text[i] === "\n") { code.push("\n"); line++; } else code.push(" ");
        i++;
      }
      if (i < n) { code.push('"'); i++; }
      continue;
    }
    if (c === "'") {
      // Char literal ('x', '\n', '\u{..}', astral 'x') versus lifetime ('a).
      let end = -1;
      if (d === "\\") end = text.indexOf("'", i + 3);
      else if (d !== undefined && d !== "\n") {
        const width = d.codePointAt(0) > 0xffff ? 2 : 1;
        if (text[i + 1 + width] === "'") end = i + 1 + width;
      }
      if (end > i && end - i <= 12 && !text.slice(i + 1, end).includes("\n")) {
        code.push("'");
        for (let k = i + 1; k < end; k++) code.push(" ");
        code.push("'");
        i = end + 1;
        continue;
      }
    }
    code.push(c); i++;
  }
  return { code: code.join(""), comments };
}

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

// Offset of the brace closing the one at `open`, or the end of the text.
function matchBrace(code, open) {
  let depth = 0;
  for (let i = open; i < code.length; i++) {
    if (code[i] === "{") depth++;
    else if (code[i] === "}") { depth--; if (depth === 0) return i; }
  }
  return code.length - 1;
}

// Lines covered by #[cfg(test)] / #[test] items (0-based, inclusive).
export function testLines(code) {
  const starts = lineStarts(code);
  const mask = new Uint8Array(starts.length);
  const attr = /#\[(?:cfg\((?:all\()?\s*test\b|test\]|tokio::test\b|bench\])/g;
  for (let m = attr.exec(code); m; m = attr.exec(code)) {
    let i = m.index;
    // Skip this and any stacked attributes to the item itself.
    let semi = -1;
    let brace = -1;
    let depth = 0;
    for (let k = i; k < code.length; k++) {
      const ch = code[k];
      if (ch === "[" || ch === "(") depth++;
      else if (ch === "]" || ch === ")") depth--;
      else if (depth === 0 && ch === ";") { semi = k; break; }
      else if (depth === 0 && ch === "{") { brace = k; break; }
    }
    const end = brace >= 0 ? matchBrace(code, brace) : semi >= 0 ? semi : i;
    for (let l = lineOf(starts, i); l <= lineOf(starts, end); l++) mask[l] = 1;
    attr.lastIndex = end + 1;
  }
  return mask;
}

// Spans (offsets) of `impl Trait for Type { .. }` blocks.
function traitImplSpans(code) {
  const spans = [];
  const re = /\bimpl\b[^{;]*?\bfor\b[^{;]*\{/g;
  for (let m = re.exec(code); m; m = re.exec(code)) {
    const open = m.index + m[0].length - 1;
    spans.push([open, matchBrace(code, open)]);
  }
  return spans;
}

// Every `fn` with a body: name, lines, params, body text.
export function functions(code) {
  const starts = lineStarts(code);
  const impls = traitImplSpans(code);
  const out = [];
  const re = /\bfn\s+([A-Za-z_][A-Za-z0-9_]*)/g;
  for (let m = re.exec(code); m; m = re.exec(code)) {
    let i = m.index + m[0].length;
    while (/\s/.test(code[i] ?? "")) i++;
    if (code[i] === "<") {
      let depth = 0;
      for (; i < code.length; i++) {
        if (code[i] === "<") depth++;
        else if (code[i] === ">" && code[i - 1] !== "-") { depth--; if (depth === 0) { i++; break; } }
      }
    }
    while (i < code.length && code[i] !== "(") i++;
    const pOpen = i;
    let depth = 0;
    for (; i < code.length; i++) {
      if (code[i] === "(") depth++;
      else if (code[i] === ")") { depth--; if (depth === 0) break; }
    }
    const params = code.slice(pOpen + 1, i);
    // A `;` ends a bodiless declaration only outside brackets: `-> [f32; 3]`
    // and `where T: Into<[u8; 4]>` hold one before the body opens.
    let open = -1;
    let square = 0;
    for (let k = i + 1; k < code.length; k++) {
      const c = code[k];
      if (c === "[") square++;
      else if (c === "]") square--;
      else if (square === 0 && c === ";") break;
      else if (square === 0 && c === "{") { open = k; break; }
    }
    if (open < 0) continue;
    const close = matchBrace(code, open);
    out.push({
      name: m[1],
      start: lineOf(starts, m.index),
      end: lineOf(starts, close),
      params,
      body: code.slice(open + 1, close),
      hasSelf: /\bself\b/.test(params),
      traitImpl: impls.some(([a, b]) => m.index > a && m.index < b),
    });
  }
  return out;
}

const KEYWORDS = new Set("as async await break const continue crate dyn else enum extern false fn for if impl in let loop match mod move mut pub ref return self Self static struct super trait true type unsafe use where while u8 u16 u32 u64 u128 usize i8 i16 i32 i64 i128 isize f32 f64 bool char str".split(" "));

// Type-2 clone key: locals renamed, calls/fields/types/literals kept.
export function cloneKey(body) {
  const toks = body.match(/[A-Za-z_][A-Za-z0-9_]*|0x[0-9A-Fa-f_]+|\d[\d_]*(?:\.\d+)?(?:[eE][+-]?\d+)?|::|->|=>|==|!=|<=|>=|&&|\|\||\S/g) ?? [];
  const out = [];
  for (let k = 0; k < toks.length; k++) {
    const t = toks[k];
    if (/^[0-9]/.test(t)) { out.push(t.replace(/_/g, "").toLowerCase()); continue; }
    if (!/^[A-Za-z_]/.test(t) || KEYWORDS.has(t) || /^[A-Z]/.test(t)) { out.push(t); continue; }
    const prev = toks[k - 1];
    const next = toks[k + 1];
    const local = prev === "let" || prev === "mut" || prev === "for" || prev === "|";
    if (!local && (prev === "." || next === "(" || next === "!" || next === "::" || next === ":")) out.push(t);
    else out.push("$v");
  }
  return { key: out.join(" "), tokens: out.length };
}

function paramNames(params) {
  const names = [];
  let depth = 0;
  let cur = "";
  for (const ch of params) {
    if ("<([".includes(ch)) depth++;
    if (">)]".includes(ch)) depth--;
    if (ch === "," && depth === 0) { names.push(cur); cur = ""; } else cur += ch;
  }
  if (cur.trim()) names.push(cur);
  return names.map((p) => p.split(":")[0].replace(/\bmut\b/, "").trim()).filter(Boolean);
}

export function isPassthrough(fn) {
  if (fn.hasSelf || fn.traitImpl) return false;
  const body = fn.body.trim().replace(/;$/, "").replace(/^return\s+/, "").trim();
  const m = /^([A-Za-z_][\w]*(?:::[A-Za-z_][\w]*)*)(?:::<[^>]*>)?\s*\(([^()]*)\)$/.exec(body);
  if (!m || m[1].startsWith("Self") || m[1] === fn.name) return false;
  const args = m[2].split(",").map((s) => s.trim()).filter(Boolean);
  const names = paramNames(fn.params);
  // A zero-argument constructor alias (`fn builder() -> B { B::default() }`) is idiom, not a relay.
  return names.length > 0 && args.length === names.length && args.every((a, k) => a === names[k]);
}

export const HOT_FN = /(?:^|_)(?:step|tick|update|advance|integrate|simulate|submit|render|draw|mix|solve)(?:_|$)|(?:^|_)frame$/;
export const ALLOC = /\bVec::new\(\)|\bvec!\s*\[|\bVec::with_capacity\(|\.to_vec\(\)|\.collect(?:::<[^>]*>)?\(\)|\bString::new\(\)|\bString::from\(|\bformat!\s*\(|\.to_string\(\)|\.to_owned\(\)|\bBox::new\(|\b(?:Hash|BTree)(?:Map|Set)::new\(\)|\bVecDeque::new\(\)|\bRc::new\(|\bArc::new\(|\.to_bytes\(\)/;
export const NONDET = /\b(?:HashMap|HashSet|RandomState|DefaultHasher)\b|\bInstant::now\b|\bSystemTime\b|\bthread_rng\b|\brand::random\b/;
export const HASH_CONST = /0x0*(?:cbf29ce484222325|100000001b3|811c9dc5|1000193|bf58476d1ce4e5b9)(?![0-9a-f])/i;
export const CAST = /\bas\s+(?:u8|u16|u32|u64|u128|usize|i8|i16|i32|i64|i128|isize|f32|f64)\b/g;
const PUB_ITEM = /^\s*pub\s+(?:(?:const|async|unsafe|extern\s+"[^"]*")\s+)*(?:fn|struct|enum|trait|type|const|static|union)\b/;
const WAIVER = /\b(?:det-ok|alloc-ok):/;
// Cheap necessary condition for a board row id in a comment: an upper-case
// letter followed by digits. The full id pattern needs the same shape, so the
// hint never drops a real id (E51: it skips the id scan on most comments).
const BOARD_ID_HINT = /[A-Z][A-Z0-9]*\d/;

// Owner order inside a clone group: engine-core first, then path order. The
// comparison is plain, not localeCompare, so the owner does not depend on the
// machine's locale (E51: the first sort call paid ~25 ms for ICU data).
export function compareCloneMembers(a, b) {
  return (b.core - a.core) || (a.at < b.at ? -1 : a.at > b.at ? 1 : 0);
}

export function boardIds(root = ".") {
  const path = join(root, "docs", "SPRINT.md");
  if (!existsSync(path)) return new Set();
  const board = readFileSync(path, "utf8").split("## Receipts")[0];
  return new Set([...board.matchAll(/^- \[[ x]\] ([A-Z][A-Z0-9]*\d+[a-z]?)\b/gm)].map((m) => m[1]));
}

const count = (s, re) => (s.match(re) ?? []).length;

// Analyze one file. `kind`: { crate, lib, sim, hot, core, testFile, bin, fixedApi, ownMath }.
// `fixedApi` (deriveFixedApi of engine-core's fixed.rs) lets `Fixed` sin/cos pass the float scan.
export function analyzeFile(text, kind, ids = new Set()) {
  const { code, comments } = lex(text.replace(/\r\n/g, "\n"));
  const lines = code.split("\n");
  const raw = text.replace(/\r\n/g, "\n").split("\n");
  const test = kind.testFile ? new Uint8Array(lines.length).fill(1) : testLines(code);
  const m = Object.fromEntries(Object.keys(METRICS).map((k) => [k, 0]));
  m.loc = 0;
  m.lines = 0;
  m.pub_items = 0;
  const hits = Object.fromEntries(Object.keys(METRICS).map((k) => [k, []]));
  const hit = (metric, line, n = 1) => { m[metric] += n; hits[metric].push(line); };
  for (let l = 0; l < lines.length; l++) {
    const c = lines[l];
    const note = comments[l] ?? "";
    if (/#\[test\]|#\[(?:tokio::)?test\b/.test(c)) m.tests++;
    if (ids.size && note && BOARD_ID_HINT.test(note)) {
      const named = note.match(/\b[A-Z][A-Z0-9]*\d+[a-z]?\b/g) ?? [];
      if (named.some((t) => ids.has(t))) hit("board_ids", l);
    }
    if (WAIVER.test(note)) hit("waivers", l);
    if (test[l]) continue;
    if (c.trim() || note.trim()) m.lines++;
    if (!c.trim()) continue;
    m.loc++;
    const u = count(c, /\.unwrap\(\)/g);
    if (u) hit("unwrap", l, u);
    const p = count(c, /\.expect\(|\bpanic!\s*\(|\bunreachable!\s*\(|\btodo!\s*\(|\bunimplemented!\s*\(/g);
    if (p) hit("panic_sites", l, p);
    const k = count(c, CAST);
    if (k) hit("casts", l, k);
    if (/#!?\[(?:allow|expect)\(/.test(c)) {
      const attr = lines.slice(l, l + 8).join(" ").split(/\)\]/)[0];
      if (!/\breason\s*=/.test(attr)) hit("allow_no_reason", l);
      if (/\bdead_code\b/.test(attr)) hit("dead_code_allows", l);
    }
    if (kind.lib && !kind.bin && /\b(?:println|eprintln|print|eprint|dbg)!\s*\(/.test(c)) hit("print_in_lib", l);
    if (!kind.bin && /\bResult\s*</.test(c) && /,\s*(?:String|&\s*'static\s+str)\s*>/.test(c)) hit("string_errors", l);
    if (!kind.core && HASH_CONST.test(c.replace(/(0x[0-9A-Fa-f_]+)/g, (h) => h.replace(/_/g, "")))) hit("hash_reimpl", l);
    if (kind.sim && NONDET.test(c) && !/\bdet-ok:/.test(note)) hit("nondet", l);
    if (/\bunsafe\b/.test(c)) hit("unsafe", l);
    if (PUB_ITEM.test(c)) {
      m.pub_items++;
      let b = l - 1;
      // Walk up over attribute lines, including a multi-line `#[x(` ... `)]` (rustfmt wraps long ones).
      for (let n = 0; b >= 0 && n < 64; n++) {
        const t = raw[b] ?? "";
        if (/^\s*#\[/.test(t)) {
          b--;
          continue;
        }
        if (/^\s*\)+\]\s*$/.test(t)) {
          let s = b - 1;
          while (s >= 0 && b - s <= 16 && !/^\s*#\[/.test(raw[s] ?? "")) s--;
          if (s >= 0 && /^\s*#\[/.test(raw[s] ?? "")) {
            b = s - 1;
            continue;
          }
        }
        break;
      }
      if (!/^\s*(?:\/\/\/|\/\*\*|#\[doc)/.test(raw[b] ?? "")) hit("undoc_pub", l);
    }
  }
  if (kind.sim && !kind.testFile && !kind.ownMath) {
    for (const h of platformMathHits(raw.join("\n"), kind.fixedApi ?? deriveFixedApi(""))) {
      if (!test[h.line - 1]) hit("float_math", h.line - 1);
    }
  }
  if (m.lines > BIG_FILE_LINES) hit("big_files", 0);
  const fns = functions(code).filter((f) => !test[f.start]);
  const clones = [];
  for (const f of fns) {
    if (f.end - f.start + 1 > LONG_FN_LINES) hit("long_fns", f.start);
    if (isPassthrough(f)) hit("passthrough", f.start);
    if (kind.hot && HOT_FN.test(f.name)) {
      for (let l = f.start + 1; l < f.end; l++) {
        if (!test[l] && ALLOC.test(lines[l]) && !/\balloc-ok:/.test(comments[l] ?? "")) hit("hot_alloc", l);
      }
    }
    if (f.end - f.start >= 3) {
      const { key, tokens } = cloneKey(f.body);
      if (tokens >= DUP_MIN_TOKENS) clones.push({ key, line: f.start, name: f.name });
    }
  }
  return { metrics: m, hits, clones, raw };
}

function walk(dir) {
  if (!existsSync(dir)) return [];
  return readdirSync(dir, { withFileTypes: true }).flatMap((e) => {
    if (e.name === "target" || e.name.startsWith(".")) return [];
    const p = join(dir, e.name);
    return e.isDirectory() ? walk(p) : e.name.endsWith(".rs") ? [p] : [];
  });
}

export function packages(root = ".") {
  const cached = packageCache.get(root);
  if (cached) return cached;
  const pkgs = [];
  const crates = join(root, "crates");
  for (const dir of existsSync(crates) ? readdirSync(crates).sort() : []) {
    const manifest = join(crates, dir, "Cargo.toml");
    if (!existsSync(manifest)) continue;
    const name = /^\s*name\s*=\s*"([^"]+)"/m.exec(readFileSync(manifest, "utf8"))?.[1] ?? dir;
    pkgs.push({ name, dir: join(crates, dir), roots: [""] });
  }
  pkgs.push({ name: "engine2040", dir: root, roots: ["src", "tests", "examples", "benches"] });
  packageCache.set(root, pkgs);
  return pkgs;
}

// The package list is read once per root per process (E51: the crate-filtered
// run read every manifest twice - once to measure, once for the orphan check).
const packageCache = new Map();

// A file whose first item, after comments, is `#![cfg(test)]` compiles only
// into test builds: a test helper module counts as test code like tests.rs.
export function testOnlyFile(text) {
  return /^(?:\s*\/\/[^\n]*\n)*\s*#!\[cfg\(test\)\]/.test(text);
}

// Test code by path: tests/benches/examples roots, any `tests/` folder, `tests.rs`,
// and `*_test.rs` / `*_tests.rs` files.
export function testPath(inPkg) {
  return /^(?:tests|benches|examples)\//.test(inPkg) || /(?:^|\/)tests\//.test(inPkg) || /(?:^|\/)(?:tests|[^/]+_tests?)\.rs$/.test(inPkg);
}

// Module names a file declares as `#[cfg(test)] mod x;` (file modules, not inline bodies).
export function cfgTestMods(text) {
  const names = [];
  const re = /#\[cfg\(test\)\]\s*(?:#\[[^\]]*\]\s*)*(?:pub(?:\([^)]*\))?\s+)?mod\s+(\w+)\s*;/g;
  for (let m = re.exec(text); m; m = re.exec(text)) names.push(m[1]);
  return names;
}

// Package-relative paths (`dir/x.rs`, `dir/x/mod.rs` and the `dir/x/` folder prefix)
// of the modules `rel`'s text declares under `#[cfg(test)]`.
export function cfgTestModPaths(rel, text) {
  const slash = rel.lastIndexOf("/");
  const dir = slash < 0 ? "" : rel.slice(0, slash);
  const base = rel.slice(slash + 1);
  const own = /^(?:lib|main|mod)\.rs$/.test(base) ? dir : `${dir ? `${dir}/` : ""}${base.replace(/\.rs$/, "")}`;
  return cfgTestMods(text).map((n) => `${own ? `${own}/` : ""}${n}`);
}

// Measure the workspace. Returns per-crate metrics, hits and clone groups.
// `filter`, when given, skips packages it rejects before any walk/read.
// `useCache` reuses per-file metrics/hits/clones from CACHE_DIR; --no-cache passes false.
export function measure(root = ".", filter = null, useCache = true) {
  const ids = boardIds(root);
  const crates = new Map();
  const cloneIndex = new Map();
  const cacheRoot = useCache ? join(root, CACHE_DIR) : null;
  const fixedPath = join(root, "crates", "core", "src", "fixed.rs");
  const fixedApi = deriveFixedApi(existsSync(fixedPath) ? readFileSync(fixedPath, "utf8") : "");
  const config = useCache ? configString(ids, fixedApi) : "";
  if (cacheRoot) {
    try { mkdirSync(cacheRoot, { recursive: true }); } catch { /* cache is best-effort */ }
  }
  for (const pkg of packages(root)) {
    if (filter && !filter(pkg.name)) continue;
    const files = pkg.roots.flatMap((r) => walk(join(pkg.dir, r)));
    const lib = existsSync(join(pkg.dir, "src", "lib.rs"));
    const total = { name: pkg.name, metrics: null, hits: {}, files: files.length };
    const texts = new Map(files.map((f) => [f, readFileSync(f, "utf8")]));
    const testMods = new Set();
    for (const [f, t] of texts) {
      if (!t.includes("#[cfg(test)]")) continue;
      for (const p of cfgTestModPaths(relative(pkg.dir, f).split("\\").join("/"), t)) testMods.add(p);
    }
    for (const file of files) {
      const rel = relative(root, file).split("\\").join("/");
      const inPkg = relative(pkg.dir, file).split("\\").join("/");
      const text = texts.get(file);
      const declaredTest = [...testMods].some((p) => inPkg === `${p}.rs` || inPkg.startsWith(`${p}/`));
      const kind = {
        crate: pkg.name,
        lib,
        bin: /^src\/(?:main\.rs|bin\/)/.test(inPkg) || !lib,
        sim: SIM_CRATES.includes(pkg.name),
        hot: HOT_CRATES.includes(pkg.name),
        core: pkg.name === "engine-core",
        fixedApi,
        ownMath: OWN_MATH_FILES.includes(rel.slice("crates/".length)),
        testFile: testPath(inPkg) || declaredTest || testOnlyFile(text),
      };
      let cacheFile = null;
      let key = null;
      let cached = null;
      if (cacheRoot) {
        cacheFile = cachePath(root, rel);
        key = fileKey(text, kindTag(kind), config);
        cached = cacheRead(cacheFile, key);
      }
      if (cached) {
        if (!total.metrics) total.metrics = Object.fromEntries(Object.keys(cached.metrics).map((k) => [k, 0]));
        for (const [k, v] of Object.entries(cached.metrics)) total.metrics[k] += v;
        for (const [k, list] of Object.entries(cached.hits)) {
          for (const s of list) (total.hits[k] ??= []).push(s);
        }
        for (const c of cached.clones) {
          const at = `${rel}:${c.line + 1}`;
          if (!cloneIndex.has(c.key)) cloneIndex.set(c.key, []);
          cloneIndex.get(c.key).push({ crate: pkg.name, at, name: c.name, core: kind.core });
        }
        continue;
      }
      const a = analyzeFile(text, kind, ids);
      const formatted = {};
      for (const [k, list] of Object.entries(a.hits)) {
        if (!list.length) continue;
        formatted[k] = list.map((l) => `${rel}:${l + 1}: ${(a.raw[l] ?? "").trim().slice(0, 100)}${fixTail(k)}`);
      }
      if (cacheRoot) cacheWrite(cacheFile, key, { metrics: a.metrics, hits: formatted, clones: a.clones });
      if (!total.metrics) total.metrics = Object.fromEntries(Object.keys(a.metrics).map((k) => [k, 0]));
      for (const [k, v] of Object.entries(a.metrics)) total.metrics[k] += v;
      for (const [k, list] of Object.entries(formatted)) {
        for (const s of list) (total.hits[k] ??= []).push(s);
      }
      for (const c of a.clones) {
        const at = `${rel}:${c.line + 1}`;
        if (!cloneIndex.has(c.key)) cloneIndex.set(c.key, []);
        cloneIndex.get(c.key).push({ crate: pkg.name, at, name: c.name, core: kind.core });
      }
    }
    total.metrics ??= Object.fromEntries([...Object.keys(METRICS), "loc", "lines", "pub_items"].map((k) => [k, 0]));
    crates.set(pkg.name, total);
  }
  // Each clone group keeps one owner (engine-core first, then path order);
  // every other member counts against its crate.
  const groups = [...cloneIndex.values()].filter((g) => g.length > 1);
  for (const g of groups) {
    // Plain comparison, not localeCompare: the order must not depend on the
    // machine's locale, and the first sort call paid ~25 ms for ICU data
    // (E51). Same order on the current tree (28 groups, checked).
    g.sort(compareCloneMembers);
    for (const member of g.slice(1)) {
      const c = crates.get(member.crate);
      c.metrics.dup_fns++;
      (c.hits.dup_fns ??= []).push(`${member.at}: fn ${member.name} duplicates ${g[0].at} (fn ${g[0].name})${fixTail("dup_fns")}`);
    }
  }
  return { crates, groups };
}

export function score(m) {
  const kloc = Math.max(m.loc / 1000, 0.05);
  let total = 0;
  const parts = {};
  for (const [k, spec] of Object.entries(METRICS)) {
    if (!spec.weight) continue;
    let v;
    if (k === "tests") v = m.tests / kloc;
    else if (spec.ratio) v = m.pub_items ? (100 * m.undoc_pub) / m.pub_items : 0;
    else if (spec.perCrate) v = m[k];
    else v = m[k] / kloc;
    let frac;
    if (spec.bad) frac = v <= spec.target ? 1 : v >= spec.zeroAt ? 0 : 1 - (v - spec.target) / (spec.zeroAt - spec.target);
    else frac = v >= spec.target ? 1 : v / spec.target;
    parts[k] = Math.round(frac * spec.weight * 10) / 10;
    total += frac * spec.weight;
  }
  return { score: Math.round(total), parts };
}

export const grade = (s) => (s >= 90 ? "A" : s >= 80 ? "B" : s >= 70 ? "C" : s >= 60 ? "D" : "F");

// The fix tail of a new-crate regression: every rule that is not zero names
// its own edit, so the line alone tells the agent what to change.
function newCrateFix(dirty) {
  if (!dirty.length) return ` fix: raise the score to 90 with zero ${STRICT_NEW.join("/")}`;
  return ` fix: clear the nonzero rules first: ${dirty.map((k) => `${k}: ${fixFor(k)}`).join("; ")}`;
}

// Ratchet: which crate metrics rose above the baseline (or fell, for tests).
export function compare(baseline, crates) {
  const regressions = [];
  const improvements = [];
  for (const [name, c] of crates) {
    const base = baseline.crates?.[name];
    if (!base) {
      const s = score(c.metrics).score;
      const dirty = STRICT_NEW.filter((k) => c.metrics[k] > 0);
      if (s < 90 || dirty.length) regressions.push(`${name}: new crate must enter at grade A with zero ${STRICT_NEW.join("/")} (score ${s}${dirty.length ? `, nonzero: ${dirty.join(", ")}` : ""})${newCrateFix(dirty)}`);
      continue;
    }
    for (const [k, spec] of Object.entries(METRICS)) {
      const now = c.metrics[k];
      const was = base[k];
      if (was === undefined) continue;
      if (spec.bad ? now > was : now < was) regressions.push(`${name}: ${k} ${was} -> ${now} (${spec.what})${fixTail(k)}`);
      else if (spec.bad ? now < was : now > was) improvements.push({ name, k, was, now });
    }
  }
  return { regressions, improvements };
}

// Baseline rows whose crate is gone pin nothing and hide a rename: the names
// in the baseline that no workspace package carries.
export function onlyBase(baseline, names) {
  return Object.keys(baseline.crates ?? {}).filter((name) => !names.has(name));
}

// Land scope: foreign staged debt never refuses an owned-clean packet.
// A hit is "crates/.../file.rs:line: snippet"; owned holds repo paths of
// the landing packet. Exact or dir-prefix match counts; empty owned means
// no scoping. Hitless regressions (tests) and unknown shapes stay blockers.
export function normalizeRepoPath(p) {
  return String(p ?? "").replace(/\\/g, "/").replace(/^(?:\.\/)+/, "").replace(/\/+$/, "");
}
export function hitIsOwned(hit, owned) {
  const f = normalizeRepoPath(String(hit ?? "").split(":")[0]);
  const list = (Array.isArray(owned) ? owned : String(owned ?? "").split(",")).map(normalizeRepoPath).filter(Boolean);
  if (!list.length || !f) return true;
  return list.some((o) => f === o || f.startsWith(o.endsWith("/") ? o : `${o}/`));
}
export function scopeRatchetToOwned(regressions, crates, owned) {
  const list = (Array.isArray(owned) ? owned : String(owned ?? "").split(",")).map(normalizeRepoPath).filter(Boolean);
  if (!list.length) return { blockers: [...regressions], foreign: [] };
  const blockers = [];
  const foreign = [];
  for (const r of regressions) {
    const text = String(r ?? "");
    if (text.startsWith("sim config")) { blockers.push(r); continue; }
    const cut = text.indexOf(":");
    const name = cut < 0 ? text : text.slice(0, cut);
    const rest = cut < 0 ? "" : text.slice(cut + 1).trim();
    if (rest.startsWith("new crate")) {
      const all = crates?.get?.(name) ? Object.values(crates.get(name).hits ?? {}).flat() : [];
      if (!all.length) { blockers.push(r); continue; }
      (all.some((h) => hitIsOwned(h, list)) ? blockers : foreign).push(r);
      continue;
    }
    const metric = rest.split(/\s+/)[0];
    const hits = crates?.get?.(name)?.hits?.[metric] ?? [];
    if (!hits.length) { blockers.push(r); continue; }
    (hits.some((h) => hitIsOwned(h, list)) ? blockers : foreign).push(r);
  }
  return { blockers, foreign };
}

// Every simulation crate carries a byte-identical SIM_CLIPPY, except the
// pending ones, which must not carry a stale partial copy either. Every
// finding names the copy that clears it.
export function simClippyFindings(root = ".", crates = new Map()) {
  const out = [];
  const canonical = existsSync(join(root, SIM_CLIPPY)) ? readFileSync(join(root, SIM_CLIPPY), "utf8") : null;
  if (canonical === null) return [`${SIM_CLIPPY} missing fix: restore ${SIM_CLIPPY} from git, it is the canonical copy every simulation crate holds`];
  for (const pkg of packages(root).filter((p) => SIM_CRATES.includes(p.name))) {
    const conf = join(pkg.dir, "clippy.toml");
    const rel = relative(root, conf).split("\\").join("/");
    const pending = SIM_CLIPPY_PENDING.includes(pkg.name);
    if (pending) {
      if (crates.get(pkg.name)?.metrics.nondet === 0) out.push(`${pkg.name}: nondet is 0, adopt ${SIM_CLIPPY} and leave SIM_CLIPPY_PENDING fix: copy ${SIM_CLIPPY} to ${rel} and drop ${pkg.name} from SIM_CLIPPY_PENDING in tools/rust-quality.mjs`);
      continue;
    }
    if (!existsSync(conf)) out.push(`${pkg.name}: missing clippy.toml (copy ${SIM_CLIPPY}) fix: copy ${SIM_CLIPPY} to ${rel}`);
    else if (readFileSync(conf, "utf8") !== canonical) out.push(`${pkg.name}: clippy.toml differs from ${SIM_CLIPPY} fix: copy ${SIM_CLIPPY} over ${rel}`);
  }
  return out;
}

function snapshot(crates) {
  const out = {};
  for (const [name, c] of [...crates].sort((a, b) => a[0].localeCompare(b[0]))) {
    out[name] = Object.fromEntries(Object.keys(METRICS).map((k) => [k, c.metrics[k]]));
  }
  return out;
}

function table(crates) {
  const cols = ["unwrap", "panic_sites", "casts", "allow_no_reason", "print_in_lib", "string_errors", "board_ids", "hash_reimpl", "dup_fns", "long_fns", "big_files", "hot_alloc", "nondet", "float_math", "undoc_pub", "tests"];
  const short = { unwrap: "unwrap", panic_sites: "panic", casts: "casts", allow_no_reason: "allow", print_in_lib: "print", string_errors: "strErr", board_ids: "ids", hash_reimpl: "hash", dup_fns: "dup", long_fns: "longFn", big_files: "bigF", hot_alloc: "hotAl", nondet: "nondet", float_math: "float", undoc_pub: "undoc", tests: "tests" };
  const rows = [["crate", "kloc", "score", "gr", ...cols.map((c) => short[c])]];
  let loc = 0;
  let weighted = 0;
  const sum = {};
  for (const [name, c] of [...crates].sort((a, b) => score(a[1].metrics).score - score(b[1].metrics).score)) {
    const s = score(c.metrics).score;
    loc += c.metrics.loc;
    weighted += s * c.metrics.loc;
    for (const k of [...cols, "loc", "pub_items"]) sum[k] = (sum[k] ?? 0) + c.metrics[k];
    rows.push([name.replace(/^engine-/, ""), (c.metrics.loc / 1000).toFixed(1), String(s), grade(s), ...cols.map((k) => String(c.metrics[k]))]);
  }
  const ws = loc ? Math.round(weighted / loc) : 0;
  rows.push(["WORKSPACE", (loc / 1000).toFixed(1), String(ws), grade(ws), ...cols.map((k) => String(sum[k]))]);
  const width = rows[0].map((_, i) => Math.max(...rows.map((r) => r[i].length)));
  return { text: rows.map((r) => r.map((v, i) => (i === 0 ? v.padEnd(width[i]) : v.padStart(width[i]))).join("  ")).join("\n"), workspace: ws, sum };
}

function main(argv) {
  // Speed: --help exits before the workspace scan (measure reads every .rs file).
  if (argv.includes("--help") || argv.includes("-h")) {
    process.stdout.write(HELP);
    return 0;
  }
  const opts = { crates: [], owned: [], list: "", limit: 40, limitSet: false, json: false, tighten: false, accept: "", reason: "", init: false, noCache: false };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (a === "--crate") opts.crates.push(argv[++i] ?? "");
    else if (a === "--owned") opts.owned.push(...String(argv[++i] ?? "").split(",").map((s) => s.trim()).filter(Boolean));
    else if (a === "--list") opts.list = argv[++i] ?? "";
    else if (a === "--limit") { opts.limit = Number(argv[++i]); opts.limitSet = true; }
    else if (a === "--json") opts.json = true;
    else if (a === "--tighten") opts.tighten = true;
    else if (a === "--accept") opts.accept = argv[++i] ?? "";
    else if (a === "--reason") opts.reason = argv[++i] ?? "";
    else if (a === "--init") opts.init = true;
    else if (a === "--no-cache") opts.noCache = true;
    else throw new Error(`unknown argument: ${a}`);
  }
  if (opts.list && !METRICS[opts.list]) throw new Error(`unknown metric ${opts.list}; one of ${Object.keys(METRICS).join(", ")}`);
  if (opts.limitSet && !opts.list) throw new Error("--limit needs --list (it caps listed hits)");
  if (opts.accept && opts.reason.trim().length < 10) throw new Error("--accept needs --reason with a real explanation");
  const want = (name) => !opts.crates.length || opts.crates.some((c) => c === name || `engine-${c}` === name);
  const { crates, groups } = measure(".", opts.crates.length ? want : null, !opts.noCache);
  const shown = new Map([...crates].filter(([name]) => want(name)));
  if (opts.crates.length && !shown.size) throw new Error(`no crate matches ${opts.crates.join(", ")}`);

  if (opts.init) {
    if (existsSync(BASELINE)) throw new Error(`${BASELINE} exists; use --tighten or --accept`);
    writeFileSync(BASELINE, `${JSON.stringify({ version: 1, note: "Ratchet for tools/rust-quality.mjs: counts only go down (--tighten); --accept logs every raise.", crates: snapshot(crates), accepted: [] }, null, 1)}\n`);
    console.log(`WROTE ${BASELINE} (${crates.size} crates)`);
    return 0;
  }
  const baseline = existsSync(BASELINE) ? JSON.parse(readFileSync(BASELINE, "utf8")) : { crates: {} };
  if (opts.accept) {
    const name = [...crates.keys()].find((n) => n === opts.accept || n === `engine-${opts.accept}`);
    if (!name) throw new Error(`no crate ${opts.accept}`);
    const now = snapshot(new Map([[name, crates.get(name)]]))[name];
    (baseline.accepted ??= []).push({ date: new Date().toISOString().slice(0, 10), crate: name, from: baseline.crates[name] ?? null, to: now, reason: opts.reason.trim() });
    baseline.crates[name] = now;
    writeFileSync(BASELINE, `${JSON.stringify(baseline, null, 1)}\n`);
    console.log(`ACCEPTED ${name} into ${BASELINE}: ${opts.reason.trim()}`);
    return 0;
  }

  let { regressions, improvements } = compare(baseline, shown);
  const orphans = onlyBase(baseline, new Set(packages(".").map((p) => p.name)));
  if (!opts.crates.length) regressions.push(...simClippyFindings(".", crates).map((f) => `sim config ${f}`));
  // Land scope: foreign staged debt warns, owned debt still fails.
  let foreign = [];
  if (opts.owned.length) {
    const scoped = scopeRatchetToOwned(regressions, shown, opts.owned);
    regressions = scoped.blockers;
    foreign = scoped.foreign;
  }
  if (opts.tighten) {
    let n = 0;
    for (const { name, k, was, now } of improvements) {
      if (!METRICS[k].bad) continue;
      baseline.crates[name][k] = now;
      console.log(`TIGHTEN ${name} ${k} ${was} -> ${now}`);
      n++;
    }
    for (const [name, c] of shown) {
      if (baseline.crates[name]) continue;
      const s = score(c.metrics).score;
      if (s >= 90 && STRICT_NEW.every((k) => c.metrics[k] === 0)) {
        baseline.crates[name] = snapshot(new Map([[name, c]]))[name];
        console.log(`ADD ${name} (score ${s})`);
        n++;
      }
    }
    writeFileSync(BASELINE, `${JSON.stringify(baseline, null, 1)}\n`);
    console.log(`RESULT tightened ${n} entr${n === 1 ? "y" : "ies"}`);
    return regressions.length ? 1 : 0;
  }
  if (opts.list) {
    const all = [...shown.values()].flatMap((c) => c.hits[opts.list] ?? []);
    const lines = opts.owned.length ? all.filter((h) => hitIsOwned(h, opts.owned)) : all;
    for (const l of lines.slice(0, opts.limit)) console.log(l);
    for (const r of foreign) console.log(`[WARN] foreign ratchet ${r}`);
    console.log(`RESULT ${lines.length} ${opts.list} hit(s)${lines.length > opts.limit ? ` (first ${opts.limit} shown)` : ""}`);
    return (regressions.length || lines.length) ? 1 : 0;
  }
  if (opts.json) {
    const out = {};
    for (const [name, c] of shown) out[name] = { ...c.metrics, ...score(c.metrics), grade: grade(score(c.metrics).score) };
    console.log(JSON.stringify({ crates: out, cloneGroups: groups.length, regressions, foreign, onlyBase: orphans, improvements: improvements.length }, null, 1));
    return regressions.length ? 1 : 0;
  }
  const t = table(shown);
  console.log("Rust quality scorecard: production code per crate, score 0-100 against docs/RUST-QUALITY.md");
  console.log(t.text);
  console.log(`clone groups (duplicated fn bodies, workspace): ${groups.length}`);
  for (const r of regressions) console.log(`[FAIL] ratchet ${r}`);
  for (const r of foreign) console.log(`[WARN] foreign ratchet ${r}`);
  for (const name of orphans) console.log(`[WARN] baseline row for a crate that does not exist: ${name}`);
  // Only lower-is-better counts lock in; a rising test density stays informational.
  const lockable = improvements.filter(({ k }) => METRICS[k].bad).length;
  if (lockable) console.log(`[INFO] ${lockable} metric(s) improved below baseline: run --tighten with the landing`);
  console.log(regressions.length ? `RESULT FAIL: ${regressions.length} ratchet regression(s) fix: every [FAIL] line above names the edit that clears its rule` : `RESULT PASS: score ${t.workspace} (${grade(t.workspace)}), 0 ratchet regressions over ${shown.size} crate(s)`);
  return regressions.length ? 1 : 0;
}

if (import.meta.main) {
  try { process.exitCode = main(process.argv.slice(2)); }
  catch (error) { console.error(`RESULT FAIL: ${error.message}`); process.exitCode = 2; }
}
