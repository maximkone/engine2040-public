// EV15: counts per research-ladder rung, read from docs/research/LADDER.md.
//   node tools/ladder-counts.mjs          prints "RUNG count" lines (stable order) plus the total
// A ladder line is `- <RUNG> [QUALIFIER] | P<n> | ...`; the rung is its first word.
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

export const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
export const LADDER = join(ROOT, "docs", "research", "LADDER.md");
export const RUNGS = ["CARD", "SPIKE", "ROW", "LANDED", "BAR", "REJECTED", "DUPE", "CANDIDATE"];

// Map of rung -> line count. Known rungs come first (zero when absent), unknown rung words after, sorted.
export function countRungs(text) {
  const counts = new Map(RUNGS.map((r) => [r, 0]));
  for (const line of text.split(/\r?\n/)) {
    const m = /^\s*- ([A-Z]+)\b[^|]*\|\s*P[\d?]+\s*\|/.exec(line);
    if (m) counts.set(m[1], (counts.get(m[1]) ?? 0) + 1);
  }
  const known = RUNGS.map((r) => [r, counts.get(r)]);
  const extra = [...counts].filter(([r]) => !RUNGS.includes(r)).sort(([a], [b]) => a.localeCompare(b));
  return new Map([...known, ...extra]);
}

export function render(counts) {
  const rows = [...counts].map(([rung, n]) => `${rung.padEnd(10)} ${n}`);
  const total = [...counts.values()].reduce((a, b) => a + b, 0);
  return [...rows, `${"TOTAL".padEnd(10)} ${total}`].join("\n");
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) {
  console.log(render(countRungs(readFileSync(LADDER, "utf8"))));
}
