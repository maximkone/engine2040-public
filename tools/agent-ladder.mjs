#!/usr/bin/env node
// agent-ladder: run the DOOR7 six-rung ladder twice and prove the two tapes
// agree. The ladder itself (crates/endpoint/src/e2e_ladder.rs) measures the
// model lane through real door verbs; this tool is the repeat check that
// makes the measurement a proof instead of a mood.
//
// It never builds: a missing or stale binary prints the exact cargo line and
// exits BLOCKED, the same way tools/bins.mjs does.
//
// Usage from the repo root:
//   node tools/agent-ladder.mjs              run twice, print both verdicts
//   node tools/agent-ladder.mjs --once       run once (a slow seat's shortcut)
//   node tools/agent-ladder.mjs --bin <exe>  an already built ladder binary
//   node tools/agent-ladder.mjs --help
//
// Exit codes: 0 every rung passed and the tapes agree - 1 a rung failed or
// the two tapes differ - 2 usage, or the ladder binary is missing.
import { existsSync } from "node:fs";
import { execFileSync } from "node:child_process";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
export const MAGIC = "e2040-e2e-ladder-ev1";
// One tape row carries exactly these eight fields.
export const FIELDS = ["id", "name", "gate", "calls", "tokens", "transcript", "refused", "check"];
const DEFAULT_BIN = join("target", "debug", "e2040-ladder.exe");
export const BUILD_LINE =
  "cargo build -p engine-endpoint --features mcp --bin e2040-ladder";

/// The tape block of one ladder run: the lines that compare, with no header
/// table and no wall clock. Both the operator table and `--tape` carry the
/// block; text without it (a crash, a usage error) reads as empty.
export function extractTape(text) {
  const lines = text.split(/\r?\n/);
  const start = lines.findIndex((line) => line.startsWith(`${MAGIC}\t`));
  if (start < 0) return "";
  const rows = [];
  for (let i = start; i < lines.length; i += 1) {
    const line = lines[i];
    if (line === "" || line.startsWith("#")) break;
    rows.push(line);
  }
  return `${rows.join("\n")}\n`;
}

/// Parse a tape block: its version plus one record per rung row. A row that
/// is not eight tab-separated fields is reported, never guessed at.
export function parseTape(tape) {
  const lines = tape.split(/\r?\n/).filter((line) => line !== "");
  if (lines.length === 0) return { version: null, rows: [], broken: ["empty tape"] };
  const head = lines[0].split("\t");
  const broken = [];
  if (head[0] !== MAGIC) broken.push(`magic is ${head[0] ?? "(none)"}, not ${MAGIC}`);
  if (head.length < 2) broken.push("the header row lost its version");
  const rows = [];
  for (const line of lines.slice(1)) {
    const fields = line.split("\t");
    if (fields.length !== FIELDS.length) {
      broken.push(`row \`${line}\` carries ${fields.length} fields, not ${FIELDS.length}`);
      continue;
    }
    rows.push(Object.fromEntries(FIELDS.map((name, index) => [name, fields[index]])));
  }
  return { version: head[1] ?? null, rows, broken };
}

/// Field-by-field differences between two tapes, so a red check names the rung
/// and the column that moved instead of dumping two blocks at the reader.
export function compareTapes(left, right) {
  const misses = [];
  const seen = new Map();
  for (const row of left.rows) seen.set(row.id, row);
  for (const row of right.rows) {
    const before = seen.get(row.id);
    if (before === undefined) {
      misses.push(`${row.id}: the second run added a rung`);
      continue;
    }
    for (const field of FIELDS.slice(2)) {
      if (before[field] !== row[field]) {
        misses.push(`${row.id}: ${field} ${before[field]} -> ${row[field]}`);
      }
    }
    seen.delete(row.id);
  }
  for (const row of seen.values()) misses.push(`${row.id}: the second run lost the rung`);
  return misses;
}

/// The rung lines a reader needs: pass or fail, and for a failure the verb
/// that refused or the gate that missed.
export function verdictRows(rows) {
  return rows.map((row) => {
    if (row.gate === "pass") {
      return `pass ${row.id} ${row.name} (${row.calls} calls, ${row.tokens} est tokens)`;
    }
    const why = row.refused !== "-" ? `refused ${row.refused}` : `missed ${row.check}`;
    return `FAIL ${row.id} ${row.name}: ${why}`;
  });
}

/// Run one ladder binary and return its text plus its own exit code. A
/// nonzero exit is a rung failure, not a tool failure: the tape still prints.
function runLadder(exe, extraArgs = []) {
  try {
    const text = execFileSync(exe, extraArgs, { encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] });
    return { text, code: 0 };
  } catch (error) {
    const code = typeof error.status === "number" ? error.status : 2;
    return { text: `${error.stdout ?? ""}`, code };
  }
}

const main = () => {
  const args = process.argv.slice(2);
  if (args.includes("--help") || args.includes("-h")) {
    console.log(
      "usage: node tools/agent-ladder.mjs [--once] [--bin <exe>]\n" +
        `  run the six-rung ladder twice and compare its tapes; a missing binary\n` +
        `  prints the rebuild line: ${BUILD_LINE}`,
    );
    return 0;
  }
  const flagIndex = args.indexOf("--bin");
  const exe = flagIndex >= 0 ? args[flagIndex + 1] : join(ROOT, DEFAULT_BIN);
  if (flagIndex >= 0 && (exe === undefined || exe === "")) {
    console.error("agent-ladder: --bin needs a path");
    return 2;
  }
  for (const arg of args) {
    if (arg !== "--once" && arg !== "--bin" && arg !== exe) {
      console.error(`agent-ladder: unknown flag \`${arg}\``);
      return 2;
    }
  }
  if (!existsSync(exe)) {
    console.error(`agent-ladder: no ladder binary at ${exe}`);
    console.error(`agent-ladder: build it first: ${BUILD_LINE}`);
    return 2;
  }
  const first = runLadder(exe);
  console.log(first.text.trimEnd());
  const left = parseTape(extractTape(first.text));
  if (left.broken.length > 0) {
    console.error(`agent-ladder: the first tape is unreadable: ${left.broken.join("; ")}`);
    return 2;
  }
  for (const line of verdictRows(left.rows)) console.log(line);
  if (args.includes("--once")) {
    console.log("agent-ladder: --once skipped the repeat check (the tape still must agree)");
    return first.code === 0 ? 0 : 1;
  }
  const second = runLadder(exe);
  const right = parseTape(extractTape(second.text));
  if (right.broken.length > 0) {
    console.error(`agent-ladder: the second tape is unreadable: ${right.broken.join("; ")}`);
    return 2;
  }
  const misses = compareTapes(left, right);
  const failed = left.rows.filter((row) => row.gate !== "pass").length;
  console.log(
    `agent-ladder: v${left.version} ${left.rows.length - failed}/${left.rows.length} rungs pass` +
      `; the repeat check ${misses.length === 0 ? "agrees" : `moved ${misses.length} field(s)`}`,
  );
  for (const miss of misses) console.error(`agent-ladder: ${miss}`);
  if (failed > 0 || misses.length > 0) return 1;
  return first.code === 0 && second.code === 0 ? 0 : 1;
};

if (process.argv[1] !== undefined && fileURLToPath(import.meta.url) === process.argv[1]) {
  process.exitCode = main();
}
