// Checks content/arena/sources: every texture-set directory has a PROVENANCE
// row, a CC0 licence word, colour + normal + roughness image files, and the
// byte sizes in the row match the files on disk. Pure reads, no network.
import { existsSync, readdirSync, readFileSync, statSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
export const SOURCES_DIR = join(ROOT, "content", "arena", "sources");
export const SIZE_BUDGET_BYTES = 60 * 1024 * 1024;
const IMAGE_RE = /\.(jpe?g|png|exr)$/i;

/** Parse the markdown table rows of PROVENANCE.md into {id, cells} records. */
export function provenanceRows(text) {
  const rows = [];
  for (const line of text.split(/\r?\n/)) {
    if (!line.startsWith("|")) continue;
    const cells = line.split("|").slice(1, -1).map((c) => c.trim());
    if (cells.length < 7 || /^-+$/.test(cells[0]) || cells[0] === "asset id") continue;
    rows.push({ id: cells[0], cells });
  }
  return rows;
}

/** Return a list of problem strings; empty means the sources tree is clean. */
export function checkSources(dir = SOURCES_DIR) {
  const problems = [];
  const provPath = join(dir, "PROVENANCE.md");
  if (!existsSync(provPath)) return [`missing ${provPath}`];
  const rows = provenanceRows(readFileSync(provPath, "utf8"));
  const byId = new Map(rows.map((r) => [r.id, r]));
  const dirs = readdirSync(dir, { withFileTypes: true }).filter((e) => e.isDirectory()).map((e) => e.name);
  let total = 0;
  for (const id of dirs) {
    const row = byId.get(id);
    if (!row) problems.push(`${id}: no PROVENANCE row`);
    else if (!/\bCC0\b/.test(row.cells[2])) problems.push(`${id}: PROVENANCE licence cell has no CC0`);
    const files = readdirSync(join(dir, id));
    const images = files.filter((f) => IMAGE_RE.test(f));
    for (const kind of ["Color", "Normal", "Roughness"]) {
      if (!images.some((f) => f.toLowerCase().includes(kind.toLowerCase()))) problems.push(`${id}: no ${kind} image`);
    }
    const lic = join(dir, id, "LICENCE.txt");
    if (!existsSync(lic) || !/\bCC0\b/.test(readFileSync(lic, "utf8"))) problems.push(`${id}: LICENCE.txt missing or no CC0`);
    for (const f of files) {
      if (!IMAGE_RE.test(f) && f !== "LICENCE.txt") problems.push(`${id}: unexpected file ${f}`);
      const size = statSync(join(dir, id, f)).size;
      total += size;
      if (row && !new RegExp(`${f.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")} ${size}\\b`).test(row.cells[4])) {
        problems.push(`${id}: PROVENANCE row lacks "${f} ${size}"`);
      }
    }
  }
  for (const id of byId.keys()) if (!dirs.includes(id)) problems.push(`${id}: PROVENANCE row but no directory`);
  if (total > SIZE_BUDGET_BYTES) problems.push(`sources total ${total} bytes is over ${SIZE_BUDGET_BYTES}`);
  return problems;
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) {
  const problems = checkSources();
  for (const p of problems) console.error(p);
  console.log(problems.length ? `${problems.length} problem(s)` : "arena sources ok");
  process.exit(problems.length ? 1 : 0);
}
