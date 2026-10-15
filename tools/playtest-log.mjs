// playtest-log.mjs: check a playtest CSV log.
// Short English. Std only. CRLF safe.

import { readFileSync } from "node:fs";

const COLUMNS = [
  "tester",
  "in_audience",
  "build_sha",
  "minute",
  "confusion_point",
  "question",
  "yes_no",
  "what",
  "where",
  "why",
  "conclusion",
  "bug_id",
  "severity",
];

// Split one CSV line. Handles quoted fields with "" escape.
function splitLine(line) {
  const out = [];
  let cur = "";
  let inQ = false;
  for (let i = 0; i < line.length; i++) {
    const c = line[i];
    if (inQ) {
      if (c === '"') {
        if (line[i + 1] === '"') {
          cur += '"';
          i++;
        } else {
          inQ = false;
        }
      } else {
        cur += c;
      }
    } else {
      if (c === '"') {
        inQ = true;
      } else if (c === ",") {
        out.push(cur);
        cur = "";
      } else {
        cur += c;
      }
    }
  }
  out.push(cur);
  return out.map((s) => s.trim());
}

// Check CSV text. Returns { lines, pass, fail, total }.
// lines hold { row, ok, reason }. Row counts the header as row 1.
export function checkCsvText(text) {
  const raw = text.split(/\r?\n/);
  // Drop trailing empty lines.
  while (raw.length > 0 && raw[raw.length - 1].trim() === "") raw.pop();
  if (raw.length === 0) return { lines: [], pass: 0, fail: 0, total: 0 };
  const header = splitLine(raw[0]);
  const lines = [];
  let pass = 0;
  let fail = 0;
  // Header must hold the 13 columns.
  const headerOk =
    header.length === COLUMNS.length &&
    COLUMNS.every((c, i) => header[i] === c);
  for (let i = 1; i < raw.length; i++) {
    const row = i + 1;
    if (raw[i].trim() === "") continue;
    if (!headerOk) {
      fail++;
      lines.push({ row, ok: false, reason: "bad header" });
      continue;
    }
    const cols = splitLine(raw[i]);
    if (cols.length !== COLUMNS.length) {
      fail++;
      lines.push({ row, ok: false, reason: "bad column count" });
      continue;
    }
    const rec = Object.fromEntries(COLUMNS.map((c, k) => [c, cols[k]]));
    if (rec.where === "") {
      fail++;
      lines.push({ row, ok: false, reason: "missing where" });
      continue;
    }
    if (rec.conclusion === "") {
      fail++;
      lines.push({ row, ok: false, reason: "missing conclusion" });
      continue;
    }
    pass++;
    lines.push({ row, ok: true, reason: "ok" });
  }
  return { lines, pass, fail, total: pass + fail };
}

function printResult(res) {
  for (const l of res.lines) {
    console.log(`${l.ok ? "PASS" : "FAIL"} ${l.row} ${l.reason}`);
  }
  console.log(`checked ${res.total} rows: ${res.pass} pass, ${res.fail} fail`);
}

// CLI: node tools/playtest-log.mjs --check <csv>
const args = process.argv.slice(2);
if (args[0] === "--check" && args[1]) {
  const text = readFileSync(args[1], "utf8");
  const res = checkCsvText(text);
  printResult(res);
  process.exit(res.fail === 0 ? 0 : 1);
} else if (import.meta.url === `file://${process.argv[1]}`.replaceAll("\\", "/") || process.argv[1]?.endsWith("playtest-log.mjs")) {
  if (args[0] !== "--check") {
    console.log("use: node tools/playtest-log.mjs --check <csv>");
    process.exit(2);
  }
}
