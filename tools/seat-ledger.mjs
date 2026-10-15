// seat-ledger: DONE and NOOP per role over the last hours (owner 2026-10-10).
//   node tools/seat-ledger.mjs [--hours 24] [--json]
// Done records start `# <title> (<role>, wave N, ...)` and end with a RESULT:
// or VERDICT: line; this parses those two. Window by file mtime. Read only.
import { existsSync, readdirSync, readFileSync, statSync } from "node:fs";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { readKnobs, seatMix } from "../.opencode/seat-guard.mjs";

const rootOf = () => resolve(process.cwd());

export const frontMatter = (text) => {
  const m = String(text ?? "").replace(/\r\n/g, "\n").match(/^---\n([\s\S]*?)\n---\n?([\s\S]*)$/);
  if (!m) return null;
  const fm = Object.fromEntries(m[1].split("\n").map((l) => l.match(/^([\w-]+):\s*(.*)$/)).filter(Boolean).map((x) => [x[1], x[2].trim()]));
  return { fm, body: m[2] };
};

// `# Twin fight (builder, wave 2, RL26)`: first item in parens is the role.
export const parseHeaderRole = (text) => {
  const m = String(text ?? "").replace(/\r\n/g, "\n").match(/^#\s+.+\(\s*([A-Za-z][\w-]*)\s*,\s*wave\s+\d+/m);
  return m ? m[1].toLowerCase() : null;
};

// Last RESULT:/VERDICT: line wins. Judge PASS counts as DONE, FAIL as PARTIAL.
export const parseOutcome = (text) => {
  const hits = [...String(text ?? "").matchAll(/^(?:RESULT|VERDICT)\s*:\s*(DONE|PARTIAL|BLOCKED|NOOP|PASS|FAIL)\b/gim)];
  if (!hits.length) return null;
  const raw = hits[hits.length - 1][1].toUpperCase();
  if (raw === "PASS") return "DONE";
  if (raw === "FAIL") return "PARTIAL";
  return raw;
};

// Role from the `#` header, else front matter `role:`. Outcome from the end.
export const parseRecord = (text) => {
  const role = parseHeaderRole(text) ?? frontMatter(text)?.fm?.role?.toLowerCase() ?? null;
  const outcome = parseOutcome(text);
  if (!role || !outcome) return null;
  return { role, outcome };
};

export const inWindow = (mtimeMs, nowMs, hours) => Number(mtimeMs) >= Number(nowMs) - Number(hours) * 3600_000;

// Pure step for tests: inline { mtimeMs, text } items, never the real queue.
export const windowRecords = (items, nowMs, hours) => items
  .filter((it) => inWindow(it.mtimeMs, nowMs, hours))
  .map((it) => parseRecord(it.text)).filter(Boolean);

export const summarize = (records, seats = []) => {
  const map = new Map(seats.map((s) => [s, { runs: 0, DONE: 0, PARTIAL: 0, BLOCKED: 0, NOOP: 0 }]));
  for (const r of records) {
    if (!map.has(r.role)) map.set(r.role, { runs: 0, DONE: 0, PARTIAL: 0, BLOCKED: 0, NOOP: 0 });
    const c = map.get(r.role);
    c.runs += 1;
    c[r.outcome] += 1;
  }
  return [...map.entries()].sort(([a], [b]) => (a < b ? -1 : 1)).map(([role, c]) => ({
    role, ...c, changedPct: c.runs ? Math.round((100 * (c.DONE + c.PARTIAL)) / c.runs) : 0,
  }));
};

export const formatLine = (r) => `${r.role}: runs=${r.runs} DONE=${r.DONE} PARTIAL=${r.PARTIAL} BLOCKED=${r.BLOCKED} NOOP=${r.NOOP} changed=${r.changedPct}%`;

const usage = () => "usage: node tools/seat-ledger.mjs [--hours <n>] [--json]";

if (import.meta.main) {
  const args = process.argv.slice(2);
  let hours = 24;
  let json = false;
  for (let i = 0; i < args.length; i++) {
    const a = args[i];
    if (a === "--hours") { hours = Number(args[++i]); if (!(hours > 0)) { console.log(usage()); process.exit(2); } }
    else if (a === "--json") json = true;
    else if (a === "-h" || a === "--help") { console.log(`seat-ledger — DONE and NOOP per role\n\n${usage()}`); process.exit(0); }
    else { console.log(usage()); process.exit(2); }
  }
  const root = rootOf();
  const seats = seatMix(readKnobs(root)).seats.map((s) => String(s.seat).toLowerCase());
  const dir = join(root, "target", "queue", "done");
  const now = Date.now();
  let items = [];
  if (existsSync(dir)) {
    for (const n of readdirSync(dir).filter((x) => x.endsWith(".md"))) {
      try {
        const p = join(dir, n);
        items.push({ mtimeMs: statSync(p).mtimeMs, text: readFileSync(p, "utf8") });
      } catch { /* unreadable record counts as no run */ }
    }
  }
  const rows = summarize(windowRecords(items, now, hours), seats);
  if (json) console.log(JSON.stringify(rows, null, 2));
  else for (const r of rows) console.log(formatLine(r));
  process.exit(0);
}
