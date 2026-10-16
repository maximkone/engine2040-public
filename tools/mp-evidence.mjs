// mp-evidence: per Core multiplayer area newest row, date and evidence path.
// Read-only. Core list from docs/FOCUS.md: rows MP and ON.
// Board reads via ./board-slice.mjs only. Never runs cargo. No writes.
import { loadBoardText, parseBoard } from "./board-slice.mjs";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
export const CORE_AREAS = Object.freeze(["MP", "ON"]);
export function rowArea(id) {
  const s = String(id);
  if (/^MP\d/.test(s)) return "MP";
  if (/^ON\d/.test(s)) return "ON";
  return null;
}
export function parseRowNumber(id) {
  const m = /^(MP|ON)(\d+)([a-z]?)$/.exec(String(id));
  if (!m) return null;
  return { area: m[1], num: Number(m[2]), suffix: m[3] || "" };
}
export function isNewer(a, b) {
  const pa = parseRowNumber(a);
  const pb = parseRowNumber(b);
  if (!pa || !pb) return String(a) > String(b);
  if (pa.num !== pb.num) return pa.num > pb.num;
  return pa.suffix > pb.suffix;
}
export function extractDate(text) {
  const m = /(\d{4}-\d{2}-\d{2})/.exec(String(text));
  return m ? m[1] : "none";
}
export function extractEvidence(text) {
  const s = String(text);
  const mdSlash = s.match(/[A-Za-z0-9_.\-/$]+\/[A-Za-z0-9_.\-/$]*\.md\b/g);
  if (mdSlash && mdSlash.length) return mdSlash[0];
  const anyMd = s.match(/[A-Za-z0-9_.\-]+\.md\b/g);
  if (anyMd && anyMd.length) return anyMd[0];
  const rsSlash = s.match(/(?:crates|tools|docs|content|fixtures)\/[A-Za-z0-9_.\-/$]+\.rs\b/g);
  if (rsSlash && rsSlash.length) return rsSlash[0];
  const anyRs = s.match(/[A-Za-z0-9_.\-]+\.rs\b/g);
  if (anyRs && anyRs.length) return anyRs[0];
  return "none";
}
export function newestPerArea(tasks, order) {
  const best = new Map();
  for (const id of order) {
    const area = rowArea(id);
    if (!area) continue;
    if (!CORE_AREAS.includes(area)) continue;
    const cur = best.get(area);
    if (!cur || isNewer(id, cur)) best.set(area, id);
  }
  return best;
}
export function formatLine(entry) {
  return `${entry.area} | ${entry.row} | ${entry.date} | ${entry.evidence}`;
}
export function collectEvidence(root) {
  const text = loadBoardText(root);
  const parsed = parseBoard(text);
  const best = newestPerArea(parsed.tasks, parsed.order);
  const out = [];
  for (const area of CORE_AREAS) {
    const row = best.get(area);
    if (!row) {
      out.push({ area, row: "none", date: "none", evidence: "none" });
      continue;
    }
    const task = parsed.tasks.get(row);
    const full = task ? task.lines.join(" ") : row;
    out.push({ area, row, date: extractDate(full), evidence: extractEvidence(full) });
  }
  return out;
}
function main(argv) {
  if (argv.includes("--help") || argv.includes("-h")) {
    process.stdout.write("mp-evidence.mjs -- per Core multiplayer area newest row\nUsage:\n  node tools/mp-evidence.mjs [--help]\nOutput: one area | row | date | evidence line per area (MP, ON). Read-only.\n");
    process.exit(0);
  }
  if (argv.length > 0) {
    process.stderr.write("mp-evidence: unknown flag\n");
    process.exit(2);
  }
  const rows = collectEvidence();
  for (const r of rows) process.stdout.write(formatLine(r) + "\n");
}
if (import.meta.main) {
  main(process.argv.slice(2));
}
