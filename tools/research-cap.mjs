// research-cap: research aims at work (port of C:/forge/scripts/research-cap.mjs, CAP = 3).
// Same rule, engine paths only: drift check (tools/workspace-check.mjs) FAILs an
// open board row with more than CAP cards in docs/research/cards, and `--next`
// hands each reader an open row that still has room, fewest cards first,
// skipping a row claimed in the last 3 h.
// Steal file docs/plan/steals.md counts toward the same cap.
//   node tools/research-cap.mjs           prints each open row over the cap (exit 1)
//   node tools/research-cap.mjs --next    the rows a reader takes next (exit 0)
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
const ROOT = dirname(HERE);
export const CAP = 3;
export const CARDS_DIR = join(ROOT, "docs", "research", "cards");
export const BOARD = join(ROOT, "docs", "SPRINT.md");
export const CLAIMS = join(ROOT, "target", "queue", "claims.txt");

// The board's Rows table: `| ID | Home | Done-when | Status | Touched |` -> Map(id -> { done, status, home, doneWhen }).
export const boardRows = (md) => {
  const rows = new Map();
  for (const line of String(md ?? "").split("\n")) {
    const c = line.trim().replace(/^\||\|$/g, "").split("|").map((x) => x.trim());
    if (c.length >= 5 && c[0] !== "ID" && /^[A-Z]\d+(?:-[A-Z0-9]+)*$/.test(c[0])) rows.set(c[0], { done: /^done\b/i.test(c[3]), status: c[3], home: c[1], doneWhen: c[2] });
  }
  return rows;
};

// A card belongs to the longest row id that prefixes its file name (`C48-SPATIALFIX-accept-oracles-paper6.md`).
export const cardRow = (name, ids) => {
  let best = null;
  for (const id of ids) {
    if (name.startsWith(`${id}-`) || name === `${id}.md`) {
      if (best === null || id.length > best.length) best = id;
    }
  }
  return best ?? null;
};

// Precompiled per-id matchers, longest-first. Stable sort keeps ties in board order.
// Same source as before, no escaping change, so matches are identical.
export const rowPatterns = (ids) => {
  const list = [...ids].sort((a, b) => b.length - a.length);
  return list.map((id) => [id, new RegExp(`(^|[^A-Za-z0-9-])${id}([^A-Za-z0-9-]|$)`)]);
};

// A date-named card (`2026-09-30-<topic>.md`) carries no row in its name. It links through one explicit
// header line in its body instead (`- Row: C48-SPATIALFIX`). Only that line counts: a row id mentioned
// anywhere else in the prose never links a card, so passing mentions cannot inflate a row's count.
export const rowFromText = (text, ids, patterns = null) => {
  const list = patterns ?? rowPatterns(ids);
  for (const line of String(text ?? "").split("\n")) {
    const m = /^\s*(?:[-*]\s*)?(?:board\s+)?rows?\s*:\s*(.+?)\s*$/i.exec(line);
    if (!m) continue;
    const hit = list.find(([, re]) => re.test(m[1]));
    if (hit) return hit[0];
  }
  return null;
};

// The name wins over the header line, so renaming a card to `<ROW>-<topic>.md` keeps its row.
export const cardRowFor = (name, text, ids, patterns = null) => {
  const list = [...ids];
  return cardRow(name, list) ?? rowFromText(text, list, patterns);
};

const countByRow = (cardNames, rows, cardTexts = new Map()) => {
  const get = (n) => (typeof cardTexts.get === "function" ? cardTexts.get(n) : cardTexts[n]);
  const ids = [...rows.keys()];
  const patterns = rowPatterns(ids);
  const count = new Map();
  for (const name of cardNames) {
    const id = cardRowFor(name, get(name), ids, patterns);
    if (id) count.set(id, (count.get(id) ?? 0) + 1);
  }
  return count;
};

// Date-named cards with no `Row:` line link to nothing; name them so the loop can label them.
export const orphans = (cardNames, rows, cardTexts = new Map()) => {
  const get = (n) => (typeof cardTexts.get === "function" ? cardTexts.get(n) : cardTexts[n]);
  const ids = [...rows.keys()];
  const patterns = rowPatterns(ids);
  return cardNames.filter((name) => /^\d{4}-\d{2}-\d{2}-/.test(name) && !cardRowFor(name, get(name), ids, patterns));
};

// Open rows with more than `cap` cards, biggest first.
export const breaches = ({ cardNames, board, rows = null, cap = CAP, cardTexts = new Map() }) => {
  const table = rows ?? boardRows(board);
  return [...countByRow(cardNames, table, cardTexts)]
    .filter(([id, n]) => !table.get(id).done && n > cap)
    .map(([row, cards]) => ({ row, cards, doneWhen: table.get(row).doneWhen })).sort((a, b) => b.cards - a.cards);
};

// drift-check line for one breach.
export const failLine = (b, cap = CAP) =>
  `research: row ${b.row} has ${b.cards} cards (cap ${cap}) and is not done: a builder experiment on the row (its done-when: ${String(b.doneWhen).slice(0, 80)}), or one dossier card that merges the takes (the rest move to cards/archive/); no new card for it`;

// The rows a reader takes next: open, with room under the cap, partial before not-ready before ready, fewest cards
// first, and not claimed in the last 3 h (a claim line names the row id and a UTC time).
export const nextRows = ({ cardNames, board, claims = "", now = Date.now(), cap = CAP, n = 3, cardTexts = new Map() }) => {
  const rows = boardRows(board);
  const count = countByRow(cardNames, rows, cardTexts);
  const patterns = rowPatterns(rows.keys());
  const claimed = new Set(String(claims).split("\n").filter((l) => now - (Date.parse((l.match(/\d{4}-\d\d-\d\dT[\d:.]+Z/) ?? [])[0]) || 0) < 3 * 3_600_000).flatMap((l) => patterns.filter(([, re]) => re.test(l)).map(([id]) => id)));
  const weight = (s) => (/^partial/i.test(s) ? 0 : /^not-ready/i.test(s) ? 1 : /^ready/i.test(s) ? 2 : 3);
  return [...rows].filter(([id, r]) => !r.done && !claimed.has(id) && (count.get(id) ?? 0) < cap && weight(r.status) < 3)
    .map(([id, r], i) => ({ row: id, cards: count.get(id) ?? 0, status: r.status, home: r.home, doneWhen: r.doneWhen, w: weight(r.status), i }))
    .sort((a, b) => a.w - b.w || a.cards - b.cards || a.i - b.i).slice(0, n);
};

const isMain = process.argv[1] && import.meta.url.endsWith(process.argv[1].replace(/\\/g, "/").split("/").pop());
if (isMain) {
  const { existsSync, readdirSync, readFileSync } = await import("node:fs");
  const cardNames = existsSync(CARDS_DIR) ? readdirSync(CARDS_DIR).filter((n) => n.endsWith(".md")) : [];
  const cardTexts = new Map(cardNames.map((n) => {
    try { return [n, readFileSync(join(CARDS_DIR, n), "utf8")]; } catch { return [n, ""]; }
  }));
  const board = existsSync(BOARD) ? readFileSync(BOARD, "utf8") : "";
  if (process.argv.includes("--next")) {
    const i = process.argv.indexOf("--n");
    const found = nextRows({ cardNames, board, claims: existsSync(CLAIMS) ? readFileSync(CLAIMS, "utf8") : "", n: i > 0 ? Number(process.argv[i + 1]) || 3 : 3, cardTexts });
    if (!found.length) console.log(`research: every open row has ${CAP} cards or a fresh claim (RESULT: NOOP)`);
    for (const r of found) console.log(`${r.row}  ${r.cards} card(s)  [${r.status.slice(0, 40)}]\n    home: ${r.home}\n    done-when: ${String(r.doneWhen).slice(0, 140)}`);
  } else {
    const rows = boardRows(board);
    const found = breaches({ cardNames, rows, cardTexts });
    for (const b of found) console.log(failLine(b));
    if (!found.length) console.log(`research: no open row has more than ${CAP} cards`);
    const dated = orphans(cardNames, rows, cardTexts);
    if (dated.length) console.log(`research: ${dated.length} date-named card(s) link to no row (add a \`- Row: <ID>\` line): ${dated.slice(0, 5).join(", ")}${dated.length > 5 ? ", ..." : ""}`);
    process.exitCode = found.length ? 1 : 0;
  }
}
