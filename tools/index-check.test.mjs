// Run: node --test tools/index-check.test.mjs
// INDEX truth test: every docs/INDEX.md path exists, plus a line-count pin.
// Parses backticked paths from docs/INDEX.md, resolves each against docs/ (or
// the repo root for root-owned names), and asserts each target exists
// (file or dir). Reports n/n plus the INDEX line count; a drift of more than
// 15 lines from the 60-line state fails as a gap.
import assert from "node:assert/strict";
import { existsSync, readFileSync, readdirSync, statSync } from "node:fs";
import { dirname, join, normalize, resolve } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
const HERE = dirname(fileURLToPath(import.meta.url));
const ROOT = dirname(HERE);
const DOCS = join(ROOT, "docs");
const INDEX_PATH = join(DOCS, "INDEX.md");
const EXPECTED_LINES = 60;
const LINE_GAP = 15;
const ROOT_OWNED = new Set(["AGENTS.md", "CONTRIBUTING.md", "deny.toml"]);
// Steal codex AGENTS.md cap (openai/codex codex-rs/core/src/agents_md.rs:
// AGENTS_MD_MAX_BYTES = DEFAULT_PROJECT_DOC_MAX_BYTES = 32 KiB). Port the cap,
// not the loader: the repo root AGENTS.md must stay under it so every agent
// pays a bounded context tax. Chain files (kernel, roles, queue) are budgeted
// separately by their owners.
export const AGENTS_MD_MAX_BYTES = 32 * 1024;
export function agentsCapOk(size, max = AGENTS_MD_MAX_BYTES) {
  return Number.isInteger(size) && size >= 0 && size <= max;
}
export function isPathToken(token) {
  if (typeof token !== "string" || token.length === 0) return false;
  if (/\s/.test(token)) return false;
  if (token.includes("--")) return false;
  return /(?:\.md|\.toml|\.mjs)\/?$/.test(token) || token.endsWith("/");
}
export function extractIndexPaths(markdown) {
  const out = [];
  const seen = new Set();
  const re = /`([^`]+)`/g;
  let m;
  while ((m = re.exec(markdown)) !== null) {
    const token = m[1].trim().replace(/[.,;:]+$/, "");
    if (!isPathToken(token)) continue;
    if (seen.has(token)) continue;
    seen.add(token);
    out.push(token);
  }
  return out;
}
export function resolveIndexEntry(token) {
  if (token.startsWith("../")) return normalize(join(DOCS, token));
  if (token.startsWith("docs/")) return join(ROOT, token);
  if (token.includes("/")) return join(DOCS, token);
  if (ROOT_OWNED.has(token)) return join(ROOT, token);
  const underDocs = join(DOCS, token);
  if (existsSync(underDocs)) return underDocs;
  for (const sub of ["systems", "plan", "research", "library"]) {
    const cand = join(DOCS, sub, token);
    if (existsSync(cand)) return cand;
  }
  return join(ROOT, token);
}
export function checkEntry(abs) {
  if (abs.includes("*")) {
    const star = abs.indexOf("*");
    const dirPart = abs.slice(0, star);
    const slash = Math.max(dirPart.lastIndexOf("/"), dirPart.lastIndexOf("\\"));
    const dir = dirPart.slice(0, slash);
    const frag = dirPart.slice(slash + 1);
    const after = abs.slice(star + 1);
    let names;
    try {
      names = readdirSync(dir);
    } catch {
      return { ok: false, abs };
    }
    const hit = names.some((n) => n.startsWith(frag) && n.endsWith(after));
    return { ok: hit, abs };
  }
  return { ok: existsSync(abs), abs };
}
export function countLines(text) {
  const norm = text.replace(/\r\n/g, "\n");
  const parts = norm.split("\n");
  if (parts.length > 0 && parts[parts.length - 1] === "") parts.pop();
  return parts.length;
}
test("extract keeps doc paths and drops commands", () => {
  const md = "see `ROADMAP.md` and `research/COMPARE.md` plus `library/` not `git log --all` nor `node tools/board-slice.mjs --ready`";
  assert.deepEqual(extractIndexPaths(md), ["ROADMAP.md", "research/COMPARE.md", "library/"]);
});
test("extract dedupes and strips trailing punctuation", () => {
  const md = "`PLAN-REVIEW.md`, then `PLAN-REVIEW.md` and `plan/`";
  assert.deepEqual(extractIndexPaths(md), ["PLAN-REVIEW.md", "plan/"]);
});
test("resolve maps bare, nested, parent and root-owned names", () => {
  assert.equal(resolveIndexEntry("ROADMAP.md"), join(DOCS, "ROADMAP.md"));
  assert.equal(resolveIndexEntry("research/COMPARE.md"), join(DOCS, "research", "COMPARE.md"));
  assert.equal(resolveIndexEntry("../skills/SKILLS.md"), join(ROOT, "skills", "SKILLS.md"));
  assert.equal(resolveIndexEntry("AGENTS.md"), join(ROOT, "AGENTS.md"));
  assert.equal(resolveIndexEntry("deny.toml"), join(ROOT, "deny.toml"));
  assert.equal(resolve(resolveIndexEntry("plan/")), resolve(join(DOCS, "plan/")));
  assert.equal(resolveIndexEntry("ai-door.md"), join(DOCS, "systems", "ai-door.md"));
});
test("checkEntry reports a missing path as not ok (failure case)", () => {
  const abs = join(DOCS, "no-such-index-target-xyz.md");
  assert.equal(checkEntry(abs).ok, false);
});
test("INDEX truth: every listed path exists", () => {
  const text = readFileSync(INDEX_PATH, "utf8");
  const entries = extractIndexPaths(text);
  assert.ok(entries.length > 10, "INDEX should list many paths, got " + entries.length);
  const missing = [];
  for (const token of entries) {
    const abs = resolveIndexEntry(token);
    if (!checkEntry(abs).ok) missing.push(token + " -> " + abs);
  }
  const lineCount = countLines(text);
  const ok = entries.length - missing.length;
  console.log("INDEX truth: " + ok + "/" + entries.length + " paths exist, " + lineCount + " lines");
  assert.deepEqual(missing, [], "missing INDEX targets:\n" + missing.join("\n"));
});
test("INDEX line count pins the 60-line state (gap over 15)", () => {
  const text = readFileSync(INDEX_PATH, "utf8");
  const lineCount = countLines(text);
  console.log("INDEX lines: " + lineCount + " (expected ~" + EXPECTED_LINES + " +- " + LINE_GAP + ")");
  assert.ok(Math.abs(lineCount - EXPECTED_LINES) <= LINE_GAP, "gap: INDEX.md has " + lineCount + " lines, expected ~" + EXPECTED_LINES + " (+-" + LINE_GAP + ")");
});
test("agentsCapOk bounds the context tax (synthetic)", () => {
  assert.equal(agentsCapOk(0), true);
  assert.equal(agentsCapOk(6341), true);
  assert.equal(agentsCapOk(AGENTS_MD_MAX_BYTES), true);
  assert.equal(agentsCapOk(AGENTS_MD_MAX_BYTES + 1), false);
  assert.equal(agentsCapOk(-1), false);
});
test("AGENTS.md stays under the 32 KiB cap", () => {
  const abs = join(ROOT, "AGENTS.md");
  const size = statSync(abs).size;
  console.log("AGENTS.md bytes: " + size + " (cap " + AGENTS_MD_MAX_BYTES + ")");
  assert.ok(agentsCapOk(size), "gap: AGENTS.md is " + size + " bytes, cap is " + AGENTS_MD_MAX_BYTES);
});
