// argv: shared shell-safe spawn + CLI failure helpers (AW3).
//
// pwsh parses `>=` as a redirect, so callers spawn with an argument
// array (shell:false) and keep queries as one argv element.
import { spawnSync } from "node:child_process";
import { readdirSync } from "node:fs";

export function junkFiles(rootDir) {
  return readdirSync(rootDir).filter((n) => n.startsWith("=") || /^chk-.*\.out$/.test(n));
}

export function fail(prefix, message, code = 1) {
  process.stderr.write(`${prefix}: error: ${message}\n`);
  process.exit(code);
}

export function usageFail(prefix, message, help) {
  process.stderr.write(`${prefix}: error: ${message}\n\n${help}`);
  process.exit(2);
}

// One shared spawn seam: argument array, never a shell string.
export function runArgv(cmd, args, { dryRun = false } = {}) {
  if (dryRun) return { ok: true, status: 0, out: JSON.stringify([cmd, ...args]) };
  const r = spawnSync(cmd, args, { encoding: "utf8", shell: false, timeout: 120_000 });
  if (r.error) return { ok: false, status: r.status ?? 1, out: String(r.error) };
  return { ok: r.status === 0, status: r.status ?? 1, out: `${r.stdout ?? ""}${r.stderr ?? ""}`.trim() };
}
// One shared CLI tail: the invoked script's arguments after `node <file>`.
export function cliArgs(argv = process.argv) {
  return argv.slice(2);
}
// One shared RESULT receipt line, so the lead never retypes it by hand
// (rewritten every round: ~12/day -> 0). Pure: same inputs give the same
// text, which kills re-derivation drift between rounds.
export function formatResultLine(status, what, proof) {
  const s = String(status ?? "").trim().toUpperCase();
  if (!["DONE", "PARTIAL", "BLOCKED", "NOOP"].includes(s)) {
    throw new Error("bad RESULT status " + JSON.stringify(status));
  }
  const w = String(what ?? "").trim().replace(/\s+/g, " ");
  const pr = String(proof ?? "").trim().replace(/\s+/g, " ");
  if (!w || !pr) throw new Error("RESULT needs what + proof text");
  return "RESULT: " + s + " - " + w + " | proof: " + pr;
}

// A RESULT proof names what was run or written. It holds a command word (node, cargo, rg, git, pwsh, python,
// Select-String, Test-Path and the usual few), a `->` result arrow, a URL, or a file name with an extension. A book or
// chapter cite ("geaen | 15.1 Components") holds none: 2026-10-12 flow audit, 14 DONE proofs were chapter cites.
// null = fine, else the reason, phrased to follow "the RESULT proof".
const PROOF_WORD = /\b(?:node|cargo|rg|git|pwsh|powershell|python3?|npm|rustc|rustfmt|Select-String|Test-Path|Get-Content|Get-ChildItem)\b|->|https?:\/\/|[\w-]\.[A-Za-z][A-Za-z0-9]{1,5}\b/;
export function proofProblem(proof) {
  const text = String(proof ?? "").trim();
  if (!text) return "is empty (name the command you ran and its result, or the file it wrote)";
  if (/^<[^>]*>$/.test(text)) return `is the template placeholder ${text} (name the command you ran and its result, or the file it wrote)`;
  if (!PROOF_WORD.test(text)) return `names no command or file result: "${text.slice(0, 80)}" (a book or chapter cite proves nothing; name the command you ran and its result, or the file it wrote)`;
  return null;
}
