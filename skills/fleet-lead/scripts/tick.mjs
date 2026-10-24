// node skills/fleet-lead/scripts/tick.mjs <ID> <x|p> "<text>"
// Tick a board row (x) or append a dated progress note (p). Finds "- [ ] <ID> " in docs/SPRINT-*.md and
// appends the text to the row's last line, keeping the file's line endings. Commit the shard by path after.
import { readdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const docs = join(dirname(fileURLToPath(import.meta.url)), "..", "..", "..", "docs");
const [id, mode, text] = process.argv.slice(2);
if (!id || !["x", "p"].includes(mode) || !text) {
  console.log('usage: tick.mjs <ID> <x|p> "<text>"');
  process.exit(2);
}
const d = new Date();
const today = `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
for (const f of readdirSync(docs).filter((n) => /^SPRINT-.*\.md$/.test(n))) {
  const raw = readFileSync(join(docs, f), "utf8");
  const eol = raw.includes("\r\n") ? "\r\n" : "\n";
  const lines = raw.split(/\r?\n/);
  const at = lines.findIndex((l) => l.startsWith(`- [ ] ${id} `));
  if (at < 0) continue;
  let end = at;
  while (end + 1 < lines.length && lines[end + 1].startsWith("  ") && lines[end + 1].trim()) end++;
  if (mode === "x") {
    lines[at] = lines[at].replace("- [ ] ", "- [x] ");
    lines[end] = `${lines[end]} ${text}`;
  } else {
    lines[end] = `${lines[end]} Progress ${today}: ${text}`;
  }
  writeFileSync(join(docs, f), lines.join(eol));
  console.log(`${id} ${mode === "x" ? "ticked" : "progress"} in ${f}:${at + 1}`);
  process.exit(0);
}
console.log(`${id}: open row not found`);
process.exit(1);
