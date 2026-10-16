#!/usr/bin/env node
// Fail closed on a collected packet's patch before it reaches the judge.
// Usage: node tools/packet-gate.mjs --patch target/lanes/<id>.patch
//   --owned crates/combat/src/ --role writer [--root .lanes/<id>]
//   (`--patch -` reads the diff from stdin)
//        node tools/packet-gate.mjs --result <reply-or-record.md|->
//   (the reply's last RESULT line: a DONE needs a command or file proof and no FAIL; exit 1 otherwise)
// Roles: writer (owned code and tests), code (no test files), tests (test files only).
// Fake-pass guard (GS4): a diff that edits or removes existing assertion
// lines while code outside tests also changes fails with
// `FAKE-PASS-RISK: n assertion lines edited`, unless the packet row carries a
// `TESTS:` mark (`--tests` on the CLI, `testsMark: true` in auditPatch).
// Assertion lines are `assert*!(...)` (Rust), `assert(...)/assert.xxx(...)`
// (JS/TS), or an `expected` literal line under `tests/` or `#[cfg(test)]`.
// Pure additions (a new test) never count: only removed lines do.
// This checks the patch, not a helper's claims. Acceptance still needs a fresh
// independent rerun; a clean patch is not evidence that its tests passed.
// New Rust lines meet the top-tier bar in docs/RUST-QUALITY.md (layer 2):
// legacy debt is ratcheted by tools/rust-quality.mjs, new debt never lands
// silently: ratchet-raising lines fail unless the same patch moves
// tools/rust-quality.baseline.json (--accept/--tighten), and auditLand plus
// the pre-commit hook (tools/hooks/pre-commit) hold every commit, including
// auto-backup commits, to the same rule.
// --root <dir> is the tree holding the post-patch files (default: the lane
// .lanes/<id> named by the patch path, else "."); it separates test code.
// New (untracked) files under --owned are read whole and scanned as added. An
// owned path with no change in the patch and no file on disk is a FAIL.
import { execFileSync } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { isMainModule } from './steal-next.mjs';
import { proofProblem } from "./argv.mjs";
import { ownVerdict, resultOf } from "./land.mjs";
import { ALLOC, BIG_FILE_LINES, boardIds, CAST, functions, HASH_CONST, HOT_CRATES, HOT_FN, lex, LONG_FN_LINES, NONDET, SIM_CRATES, testLines } from "./rust-quality.mjs";

const roles = new Set(["writer", "code", "tests"]);

// Ratchet baseline: debt findings fail unless the same patch moves it.
const BASELINE_PATH = "tools/rust-quality.baseline.json";

// Crate of a repo path (`crates/<dir>/...` -> `engine-<dir>`; root package otherwise).
function crateOf(path) {
  const m = /^crates\/([^/]+)\//.exec(path);
  return m ? `engine-${m[1]}` : "engine2040";
}

// Added lines of one file section with their post-patch line numbers.
function addedLines(section) {
  const out = [];
  let line = 0;
  let inHunk = false;
  for (const raw of section.split(/\r?\n/)) {
    const hunk = /^@@ -\d+(?:,\d+)? \+(\d+)(?:,\d+)? @@/.exec(raw);
    if (hunk) { line = Number(hunk[1]); inHunk = true; continue; }
    if (!inHunk || raw.startsWith("\\")) continue;
    if (raw.startsWith("+") && !raw.startsWith("+++")) { out.push({ line, text: raw.slice(1) }); line++; }
    else if (raw.startsWith(" ") || raw === "") line++;
  }
  return out;
}

// Internal (no cross-file importers): used by auditPatch below.
function rustFindings(path, section, { root = ".", ids = new Set() } = {}) {
  const errors = [];
  const warnings = [];
  const added = addedLines(section);
  if (!added.length) return { errors, warnings };
  const crate = crateOf(path);
  const inCrate = path.replace(/^crates\/[^/]+\//, "");
  const testFile = /(^|\/)(?:tests|benches|examples)\//.test(inCrate) || /(?:^|\/)(?:tests|[^/]+_tests)\.rs$/.test(inCrate);
  const binFile = /^src\/(?:main\.rs|bin\/)/.test(inCrate) || /^examples\//.test(inCrate);
  const sim = SIM_CRATES.includes(crate);
  const hot = HOT_CRATES.includes(crate);
  // The post-patch file (when present) tells test code and function spans apart.
  const full = existsSync(join(root, path)) ? readFileSync(join(root, path), "utf8").replace(/\r\n/g, "\n") : null;
  const code = full === null ? null : lex(full).code;
  const mask = code === null ? null : testLines(code);
  const fns = code === null ? [] : functions(code);
  let heuristicTest = false;
  const addedSet = new Set(added.map((a) => a.line));
  const fail = (a, rule, why) => errors.push(`${path}:${a.line}: ${rule}: ${why}: ${a.text.trim().slice(0, 90)}`);
  let casts = 0;
  for (const a of added) {
    const { code: c, comments } = lex(a.text);
    const note = comments[0] ?? "";
    if (mask === null && /#\[cfg\(test\)\]|^\s*mod tests\b/.test(c)) heuristicTest = true;
    const isTest = testFile || (mask ? mask[a.line - 1] === 1 : heuristicTest);
    if (/\bdbg!\s*\(/.test(c)) fail(a, "rust-dbg", "remove debug output");
    if (/#!?\[allow\(/.test(c)) fail(a, "rust-allow", "use #[expect(lint, reason = \"...\")] so a stale exception fails the build");
    if (/#!?\[expect\(/.test(c)) {
      // rustfmt splits long attributes, so read up to the closing `)]`.
      const after = full !== null ? full.split("\n").slice(a.line - 1, a.line + 8) : added.filter((b) => b.line >= a.line && b.line < a.line + 9).map((b) => b.text);
      if (!/\breason\s*=/.test(after.join(" ").split(/\)\]/)[0])) fail(a, "rust-expect-reason", "every #[expect] states its reason");
    }
    if (/\bimpl\s+GlobalAlloc\s+for\b/.test(c) && crate !== "engine-core") fail(a, "rust-alloc-probe-copy", "use engine_core::alloc_probe (dev-dependency feature test-util)");
    if (crate !== "engine-core" && HASH_CONST.test(c.replace(/0x[0-9A-Fa-f_]+/g, (h) => h.replace(/_/g, "")))) fail(a, "rust-hash-copy", "import engine_core::{Fnv64, hash_bytes, splitmix64} instead of re-deriving them");
    if (note && !/\b(?:TODO|FIXME)\b/.test(note) && (note.match(/\b[A-Z][A-Z0-9]*\d+[a-z]?\b/g) ?? []).some((t) => ids.has(t))) fail(a, "rust-board-id", "no board IDs or history in code comments (CODELAW 12): the board and git carry it");
    if (isTest) continue;
    if (/\.unwrap\(\)/.test(c)) fail(a, "rust-unwrap", "use `?` with a typed error, or expect(\"<proven invariant>\")");
    if (/\.expect\(/.test(c)) fail(a, "rust-expect-site", "new .expect() raises the panic_sites ratchet: land it with a --accept/--tighten baseline update");
    if (/\b(?:panic|unreachable)!\s*\(/.test(c)) fail(a, "rust-panic-site", "new panic!/unreachable! raises the panic_sites ratchet: land it with a --accept/--tighten baseline update");
    if (!binFile && /\b(?:println|eprintln|print|eprint)!\s*\(/.test(c)) fail(a, "rust-print", "library code reports through typed errors or engine-observe, not stdout");
    if (!binFile && /\bResult\s*</.test(c) && /,\s*(?:String|&\s*'static\s+str)\s*>/.test(c)) fail(a, "rust-string-error", "return a typed error enum, not a string (binaries may report context at the boundary)");
    if (sim && NONDET.test(c) && !/\bdet-ok:/.test(note)) fail(a, "rust-nondet", "simulation state uses BTreeMap/sorted Vec and engine_core::Tick (waive only with `// det-ok: <why>` for data that never reaches the hash)");
    if (hot && ALLOC.test(c) && !/\balloc-ok:/.test(note)) {
      const fn = fns.find((f) => f.start < a.line - 1 && a.line - 1 < f.end && HOT_FN.test(f.name));
      if (fn) fail(a, "rust-hot-alloc", `no allocation in per-tick/per-frame fn ${fn.name}: reuse a buffer the system owns (waive only with \`// alloc-ok: <why>\` on a cold branch)`);
    }
    casts += (c.match(CAST) ?? []).length;
  }
  if (casts) warnings.push(`${path}: ${casts} new numeric \`as\` cast(s): prefer From/TryFrom or a named conversion`);
  if (mask !== null) {
    for (const f of fns) {
      const len = f.end - f.start + 1;
      if (mask[f.start] || len <= LONG_FN_LINES) continue;
      let touched = 0;
      for (let l = f.start + 1; l <= f.end + 1; l++) if (addedSet.has(l)) touched++;
      if (touched === len) errors.push(`${path}:${f.start + 1}: rust-long-fn: new fn ${f.name} is ${len} lines (limit ${LONG_FN_LINES}): split it into named steps`);
      else if (touched) warnings.push(`${path}:${f.start + 1}: edited fn ${f.name} is ${len} lines (limit ${LONG_FN_LINES}): split it while it is open`);
    }
    for (const a of added) {
      if (mask[a.line - 1] === 1) continue;
      if (!/^\s*pub\s+(?:fn|struct|enum|const|static|trait|type|mod)\b/.test(a.text)) continue;
      const lines = full.split("\n");
      let documented = false;
      for (let l = a.line - 1; l >= Math.max(1, a.line - 4); l--) {
        const prev = (lines[l - 1] ?? "").trim();
        if (!prev) continue;
        if (prev.startsWith("///")) { documented = true; break; }
        if (prev.startsWith("#[")) continue;
        break;
      }
      if (!documented) warnings.push(`${path}:${a.line}: rust-undoc: new public item without a doc comment: add /// docs (units, errors, determinism)`);
    }    const prod = full.split("\n").filter((l, i) => !mask[i] && l.trim()).length;
    if (prod > BIG_FILE_LINES) {
      const isNew = /^--- \/dev\/null$/m.test(section);
      const msg = `${path}: ${prod} production lines (limit ${BIG_FILE_LINES})`;
      if (isNew) errors.push(`${msg}: split the new file into modules`);
      else if (added.length > 40) warnings.push(`${msg}: deepen in a new module instead of growing this file`);
    }
  }
  return { errors, warnings };
}
const placeholder = /\b(?:TODO(?!\s*(?:\(|\[)?[A-Z]+\d+\b)|FIXME(?!\s*(?:\(|\[)?[A-Z]+\d+\b)|your[-_ ]code[-_ ]here|rest\s+unchanged|implement\s+me)\b|\b(?:todo|unimplemented)!\s*\(/i;

function pathInPatch(value) {
  if (!value || value === "/dev/null") return null;
  // Git's quoted diff paths need decoding, not space-based splitting.
  let name = value;
  if (name.startsWith('"')) {
    try { name = JSON.parse(name); } catch { return ""; }
  }
  if (!/^[ab]\//.test(name)) return "";
  name = name.slice(2).replace(/\\/g, "/");
  if (!name || name.startsWith("/") || name.split("/").some((part) => part === ".." || part === "." || !part)) return "";
  return name;
}

function scope(value) {
  const name = value.replace(/\\/g, "/");
  const parts = (name.endsWith("/") ? name.slice(0, -1) : name).split("/");
  if (!name || name.startsWith("/") || /^[a-z]:/i.test(name) || parts.some((part) => part === ".." || part === "." || !part)) {
    throw new Error(`invalid owned path: ${value}`);
  }
  return name;
}

function testPath(name) {
  return /(^|\/)(?:tests?|[^/]+\.tests)(\/|$)/i.test(name)
    || /(?:^|\/)(?:test_[^/]+|[^/]+(?:\.test\.[^/]+|_test\.rs|_tests\.rs)|tests?\.rs)$/i.test(name);
}

// GS4 fake-pass guard: one removed line is an edited assertion when it calls
// an assert helper or names an expected value with a literal.
function isAssertionLine(text) {
  if (/\bassert\w*\s*!/.test(text)) return true;
  if (/\bassert[\w.]*\s*\(/.test(text)) return true;
  if (/\bexpected\b/i.test(text) && /(\d|"|'|`)/.test(text)) return true;
  return false;
}

// A hunk line of a mixed Rust file (impl plus #[cfg(test)] in one file) is a
// test line when it is the module marker or an assertion itself.
function isMixedTestLine(text) {
  return /#\[cfg\(test\)\]|^\s*mod tests\b|^\s*#\[test\]/.test(text) || isAssertionLine(text);
}

// Count removed assertion lines in test scope, and whether production code
// outside tests also changed. Test scope is a test file (tests/, *.test.*,
// *_test.rs) or, for Rust, a section holding #[cfg(test)] / mod tests.
// Untracked whole-file sections are all-added, so they never count.
export function fakePassRisk(sections) {
  let edited = 0;
  let implChanged = false;
  for (const section of sections.filter((s) => s.startsWith("diff --git "))) {
    const old = /^--- (.+)$/m.exec(section);
    const next = /^\+\+\+ (.+)$/m.exec(section);
    if (!old || !next) continue;
    const before = pathInPatch(old[1]);
    const after = pathInPatch(next[1]);
    if (before === "" || after === "") continue;
    const name = after ?? before ?? "";
    const isTest = testPath(before ?? "") || testPath(after ?? "");
    const mixed = !isTest && /\.rs$/i.test(name) && /#\[cfg\(test\)\]|^\s*mod tests\b/m.test(section);
    for (const line of section.split(/\r?\n/)) {
      if (line.startsWith("@@") || line.startsWith("diff --git") || line.startsWith("---") || line.startsWith("+++") || line.startsWith("\\")) continue;
      if (line.startsWith("-") && !line.startsWith("---")) {
        const text = line.slice(1);
        if (isTest && isAssertionLine(text)) edited++;
        else if (mixed && isAssertionLine(text)) edited++;
        else if (!isTest && !mixed) implChanged ||= line.trim() !== "";
        else if (mixed && !isMixedTestLine(text) && text.trim() !== "") implChanged = true;
      } else if (line.startsWith("+") && !line.startsWith("+++")) {
        const text = line.slice(1);
        if (isTest) continue;
        if (mixed) { if (!isMixedTestLine(text) && text.trim() !== "") implChanged = true; }
        else if (text.trim() !== "") implChanged = true;
      } else if (!isTest && !mixed && (line.startsWith(" ") || line === "")) {
        continue;
      }
    }
  }
  return { edited, implChanged };
}

// A new (untracked) file is invisible to `git diff`: read it whole and report
// every line as added. Binary files cannot be scanned as text, so they fail.
export function untrackedSection(root, path) {
  const bytes = readFileSync(join(root, path));
  if (bytes.includes(0)) return `diff --git a/${path} b/${path}\nBinary files /dev/null and b/${path} differ\n`;
  const lines = bytes.toString("utf8").replace(/\r\n/g, "\n").split("\n");
  if (lines.at(-1) === "") lines.pop();
  return [`diff --git a/${path} b/${path}`, "--- /dev/null", `+++ b/${path}`, `@@ -0,0 +1,${lines.length} @@`, ...lines.map((l) => `+${l}`), ""].join("\n");
}

// Untracked files git reports under the owned paths (no git or no repo: none).
export function untrackedPaths(owned, cwd = ".") {
  if (!owned.length) return [];
  try {
    const out = execFileSync("git", ["ls-files", "--others", "--exclude-standard", "-z", "--", ...owned], { cwd, encoding: "utf8", stdio: ["ignore", "pipe", "ignore"] });
    return out.split("\0").filter(Boolean).map((p) => p.replace(/\\/g, "/"));
  } catch {
    return [];
  }
}

export function auditPatch(patch, { owned, role = "writer", root = ".", ids = null, untracked = [], testsMark = false }) {
  if (!roles.has(role)) throw new Error(`invalid role: ${role}`);
  if (!Array.isArray(owned) || owned.length === 0) throw new Error("at least one --owned path is required");
  const paths = owned.map(scope);
  const errors = [];
  const warnings = [];
  const board = ids ?? boardIds(".");
  const changed = new Set();
  const patchSections = String(patch).split(/(?=^diff --git )/m).filter(Boolean);
  const listed = new Set(patchSections.map((s) => /^diff --git a\/(\S+)/.exec(s)?.[1]).filter(Boolean));
  const fresh = untracked.map((name) => pathInPatch(`a/${name}`)).filter((name) => name && !listed.has(name)).map((name) => untrackedSection(root, name));
  const sections = [...patchSections, ...fresh];
  if (!sections.length || patchSections.some((s) => !s.startsWith("diff --git "))) errors.push("no usable git patch");
  for (const section of sections.filter((s) => s.startsWith("diff --git "))) {
    const old = /^--- (.+)$/m.exec(section);
    const next = /^\+\+\+ (.+)$/m.exec(section);
    if (!old || !next) {
      errors.push("unreadable or binary diff (requires a separately verified asset packet)");
      continue;
    }
    const before = pathInPatch(old[1]);
    const after = pathInPatch(next[1]);
    if (before === "" || after === "" || (!before && !after)) {
      errors.push("invalid diff path");
      continue;
    }
    for (const name of [before, after].filter(Boolean)) {
      changed.add(name);
      if (!paths.some((p) => p.endsWith("/") ? name.startsWith(p) : name === p)) errors.push(`${name}: outside owned paths`);
      if (role === "code" && testPath(name)) errors.push(`${name}: code-only packet changed tests`);
      if (role === "tests" && !testPath(name)) errors.push(`${name}: tests-only packet changed implementation`);
    }
    if (!after && before && testPath(before)) errors.push(`${before}: deleted a test file`);
    if (before && after && before !== after) errors.push(`${before} -> ${after}: rename requires separate review`);
    if (after?.endsWith(".rs")) {
      const rust = rustFindings(after, section, { root, ids: board });
      errors.push(...rust.errors);
      warnings.push(...rust.warnings);
    }    const codeFile = /\.(?:rs|cs|mjs|js|ts|tsx|py|jsonc?)$/i.test(after ?? "") && !testPath(after);
    if (!codeFile) continue;
    let inHunk = false;
    for (const line of section.split(/\r?\n/)) {
      if (line.startsWith("@@")) inHunk = true;
      else if (inHunk && line.startsWith("+") && !line.startsWith("+++")) {
        if (!/^\s*const placeholder = \//.test(line.slice(1)) && placeholder.test(line.slice(1))) errors.push(`${after}: added placeholder: ${line.slice(1).trim().slice(0, 90)}`);
      }
    }
  }
  // An owned path the packet never touched, and that is not on disk, claims nothing.
  for (const p of paths) {
    const hit = [...changed].some((name) => (p.endsWith("/") ? name.startsWith(p) : name === p));
    if (!hit && !existsSync(join(root, p))) errors.push(`${p}: owned path has no change and no file on disk`);
  }
  // GS4 fake-pass guard: edited assertions plus production changes fail
  // unless the row carries a TESTS: mark. Added tests alone always pass.
  const fake = fakePassRisk(sections);
  if (!testsMark && fake.edited > 0 && fake.implChanged) {
    errors.push(`FAKE-PASS-RISK: ${fake.edited} assertion lines edited while production code also changed (row needs a TESTS: mark to waive)`);
  }
  // Ratchet debt rides a baseline landing openly, never silently: findings
  // that raise a rust-quality count fail unless this patch also moves
  // tools/rust-quality.baseline.json (--accept/--tighten). The pre-commit
  // hook (tools/hooks/pre-commit) enforces the same rule on every commit,
  // including auto-backup commits that never pass through a packet review.
  const baselineTouched = changed.has(BASELINE_PATH);
  if (baselineTouched) {
    const waived = errors.filter((e) => /rust-(expect-site|panic-site|long-fn)\b/.test(e));
    for (const w of waived) warnings.push(`${w} (baseline moves in this landing)`);
    for (let i = errors.length - 1; i >= 0; i--) {
      if (/rust-(expect-site|panic-site|long-fn)\b/.test(errors[i])) errors.splice(i, 1);
    }
  } else {
    for (const w of warnings) {
      if (/new numeric `as` cast|edited fn .*limit \d+|deepen in a new module/.test(w)) {
        errors.push(`${w} (ratchet-baseline: land with --accept/--tighten so the baseline moves openly)`);
      }
    }
  }
  return { ok: errors.length === 0, changed: [...changed], errors, warnings };
}

// Commit-time twin of the patch rule above: a commit whose tree raises the
// ratchet is refused unless the same landing touches the baseline
// (--accept/--tighten). The pre-commit hook feeds this; tests pin it.
export function auditLand({ regressions = [], baselineTouched = false } = {}) {
  const debt = (regressions ?? []).filter((r) => typeof r === "string" && r.length);
  if (!debt.length) return { ok: true, errors: [] };
  if (baselineTouched) return { ok: true, errors: [], noted: debt };
  return {
    ok: false,
    errors: debt.map((d) => `${d} (ratchet-baseline: refused: land with --accept/--tighten so the baseline moves openly)`),
  };
}

// The reply, not the patch (2026-10-12 flow audit: 14 DONE proofs cited a book chapter, 12 judge DONE lines carried a FAIL).
// A DONE must come with a proof that names a command or file result (argv.mjs proofProblem) and with no FAIL: no
// VERDICT: FAIL/BLOCKED line and no FAIL in its text. Other statuses claim no result and pass. land.mjs applies the same
// checks before it commits. `text` is a done/ record or a bare reply.
export function auditResult(text) {
  const res = resultOf(text);
  const errors = [];
  if (!res.status) errors.push("no RESULT line");
  if (res.status === "DONE") {
    const said = ownVerdict(text);
    if (said.verdict && said.verdict !== "PASS") errors.push(`DONE after a ${said.verdict} verdict line: ${said.line.slice(0, 100)}`);
    if (/\bFAIL\b/.test(res.what)) errors.push(`the RESULT text says FAIL after DONE: ${res.what.slice(0, 100)}`);
    const bad = proofProblem(res.proof);
    if (bad) errors.push(`the RESULT proof ${bad}`);
  }
  return { ok: errors.length === 0, errors };
}

function cli(argv) {
  if (argv[0] === "--result") {
    if (!argv[1]) throw new Error("--result needs a reply file (or - for stdin)");
    const audit = auditResult(readFileSync(argv[1] === "-" ? 0 : argv[1], "utf8"));
    for (const error of audit.errors) console.error(`FAIL ${error}`);
    console.log(`RESULT ${audit.ok ? "PASS" : "FAIL"}: ${audit.errors.length} finding(s) in the reply's RESULT line`);
    if (!audit.ok) process.exitCode = 1;
    return;
  }
  const opts = { owned: [], role: "writer", patch: "", root: "", testsMark: false };
  for (let i = 0; i < argv.length; i++) {
    const arg = argv[i];
    if (arg === "--owned") opts.owned.push(argv[++i] ?? "");
    else if (arg === "--role") opts.role = argv[++i] ?? "";
    else if (arg === "--patch") opts.patch = argv[++i] ?? "";
    else if (arg === "--root") opts.root = argv[++i] ?? "";
    else if (arg === "--tests") opts.testsMark = true;
    else if (arg === "--help" || arg === "-h") { console.log("Usage: node tools/packet-gate.mjs --patch <file|-> --owned <path> [--role writer] [--root <dir>] [--tests]"); return; }
    else throw new Error(`unknown argument: ${arg}`);
  }
  if (!opts.patch) throw new Error("--patch is required");
  // A collected lane patch target/lanes/<id>.patch reads its files from .lanes/<id>.
  const lane = /(?:^|[\\/])target[\\/]lanes[\\/]([^\\/]+)\.patch$/.exec(opts.patch)?.[1];
  opts.root ||= lane && existsSync(join(".lanes", lane)) ? join(".lanes", lane) : ".";
  // `--patch -` reads the diff from stdin (a writer gating its own lane diff).
  const untracked = untrackedPaths(opts.owned, opts.root);
  const result = auditPatch(readFileSync(opts.patch === "-" ? 0 : opts.patch, "utf8"), { ...opts, untracked });
  for (const error of result.errors) console.error(`FAIL ${error}`);
  for (const warning of result.warnings) console.log(`WARN ${warning}`);
  console.log(`RESULT ${result.ok ? "PASS" : "FAIL"}: ${result.changed.length} changed path(s), ${result.errors.length} finding(s), ${result.warnings.length} warning(s) (root ${opts.root})`);
  if (!result.ok) process.exitCode = 1;
}

if (isMainModule(import.meta.url)) {
  try { cli(process.argv.slice(2)); }
  catch (error) { console.error(`RESULT FAIL: ${error.message}`); process.exitCode = 2; }
}
