#!/usr/bin/env node
// codex-check.mjs - the Rome codex check (HX6).
//
// Job: read the two public-domain sources behind the Rome tables - Vegetius
// `De re militari` I and Pseudo-Hyginus `De munitionibus castrorum` - from the
// corpus `cltk/lat_text_latin_library` (Public Domain Mark 1.0) and check every
// number of `content/rome/camp.toml` and `content/rome/line.toml` against the
// passage it names. Each codex row of a table holds the passage ref, the
// spelling the source writes and the hash of that passage's text: a
// re-numbered edition then fails the check instead of quietly moving a ditch
// from 5 ft to 4 ft.
//
// The numbers and their refs are written down in
// docs/research/hunt-2026-10-21/07-codex-vegetius-hyginus.md. One Roman foot
// is 296 mm, the value the tables carry in their `units` section.
//
// Usage:
//   node tools/codex-check.mjs             check the shipped tables
//   node tools/codex-check.mjs --fetch     fetch the corpus into the cache first
//   node tools/codex-check.mjs --help
//
// Rules:
//   - the corpus is cached in content/local/codex/lat_text_latin_library/ and is
//     never committed; a missing file names itself and the fetch command.
//   - every run prints one line per number and a summary line.
//   - a number no passage supports prints `unmatched`; a passage whose hash
//     moved prints `changed`. Either way the run exits 1.
//
// Exit: 0 ok, 1 a number or a passage drifted, 2 bad CLI, a missing corpus file
// or an unreadable table.
import { createHash } from "node:crypto";
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
const REPO = resolve(HERE, "..");

/** Where the corpus is cached. The folder is gitignored: the corpus is fetched,
 * never committed. */
export const DEFAULT_CORPUS = join(REPO, "content", "local", "codex", "lat_text_latin_library");
/** The tables the check reads. */
export const DEFAULT_CAMP = join(REPO, "content", "rome", "camp.toml");
export const DEFAULT_LINE = join(REPO, "content", "rome", "line.toml");

/** The two files of the corpus the Rome tables cite. */
export const CORPUS_FILES = [
  { name: "hyginus6.txt", path: "hyginus/hyginus6.txt" },
  { name: "vegetius1.txt", path: "vegetius1.txt" },
];
const RAW = "https://raw.githubusercontent.com/cltk/lat_text_latin_library";
const BRANCHES = ["master", "main"];
const UA = "engine2040-codex-check/1.0 (+https://github.com/maximkone/engine2040)";

export const HELP = `Usage: node tools/codex-check.mjs [--fetch] [--corpus DIR] [--camp FILE] [--line FILE] [--help]

Checks every number of content/rome/camp.toml and content/rome/line.toml against
the passage it names in the corpus cltk/lat_text_latin_library (Public Domain
Mark 1.0): Vegetius De re militari I and Pseudo-Hyginus De munitionibus
castrorum. Each codex row of a table holds the ref, the spelling the source
writes and the hash of that passage's text.

Prints one line per number, for example
  camp ditch.one_night.width_ft  PHG 49  quinque = 5 ft  1480 mm  table 5 ft  ok
and a summary line
  codex: numbers=25 unmatched=0

  --fetch      fetch the corpus into the cache when a file is missing
  --corpus DIR where the corpus files live
  --camp FILE  the camp table
  --line FILE  the line table
  --help       print this help

Exit 0 ok, 1 a number or a passage drifted, 2 bad CLI, a missing corpus file or
an unreadable table.
`;

const escapeRe = (s) => s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");

/** One table file: its sections, each a map of key to value. A value is an
 * integer or a string; the Rome tables hold nothing else. */
export function parseTable(text) {
  const sections = new Map();
  let current = null;
  for (const raw of String(text).split(/\r?\n/)) {
    const line = raw.trim();
    if (!line || line.startsWith("#")) continue;
    if (line.startsWith("[") && line.endsWith("]")) {
      current = line.slice(1, -1);
      if (!sections.has(current)) sections.set(current, new Map());
      continue;
    }
    const at = line.indexOf("=");
    if (at < 0 || !current) throw new Error(`not a key = value line: ${line}`);
    const key = line.slice(0, at).trim();
    const value = line.slice(at + 1).trim();
    if (value.startsWith('"') && value.endsWith('"') && value.length >= 2) {
      sections.get(current).set(key, value.slice(1, -1));
    } else if (/^\d+$/.test(value)) {
      sections.get(current).set(key, Number(value));
    } else {
      throw new Error(`not a whole number or a quoted string: ${line}`);
    }
  }
  return sections;
}

const ROMAN = { I: 1, V: 5, X: 10, L: 50, C: 100, D: 500, M: 1000 };

/** A roman numeral as a number. The corpus writes the old additive style, so
 * `IIII` is 4 and `XXIIII` is 24. */
export function romanToInt(text) {
  const s = String(text).toUpperCase();
  if (!s) return 0;
  let n = 0;
  for (let i = 0; i < s.length; i += 1) {
    const v = ROMAN[s[i]] ?? 0;
    const next = ROMAN[s[i + 1]] ?? 0;
    n += v < next ? -v : v;
  }
  return n;
}

/** The Pseudo-Hyginus paragraphs: a line that starts with `N.` opens one. The
 * corpus carries the numbering already, so the splitter is a line regex. */
export function splitHyginus(text) {
  const out = new Map();
  let current = null;
  for (const line of String(text).split(/\r?\n/)) {
    const m = /^(\d+)\.\s(.*)$/.exec(line);
    if (m) {
      current = Number(m[1]);
      out.set(current, m[2]);
      continue;
    }
    if (current !== null) out.set(current, `${out.get(current)}\n${line}`);
  }
  return out;
}

/** The Vegetius chapters: a line that starts with a roman numeral and a dot
 * opens one. The chapter is read by value, so the old additive spelling counts.
 * The last line of a chapter wins: the summary at the top of the file repeats
 * `I.` and the real chapter I follows it. */
export function splitVegetius(text) {
  const out = new Map();
  let current = null;
  for (const line of String(text).split(/\r?\n/)) {
    const m = /^([IVXLCDM]+)\.\s(.*)$/.exec(line);
    if (m) {
      current = romanToInt(m[1]);
      out.set(current, m[2]);
      continue;
    }
    if (current !== null) out.set(current, `${out.get(current)}\n${line}`);
  }
  return out;
}

/** The passage a ref names: `PHG 49` is Pseudo-Hyginus paragraph 49, `VEG 1.24`
 * is Vegetius book I chapter 24. */
export function passageFor(ref, corpus) {
  const m = /^(PHG|VEG)\s+(\d+)(?:\.(\d+))?$/.exec(String(ref).trim());
  if (!m) return null;
  const number = Number(m[1] === "PHG" ? m[2] : (m[3] ?? m[2]));
  const text = m[1] === "PHG" ? corpus.hyginus.get(number) : corpus.vegetius.get(number);
  return text === undefined ? null : { text, number };
}

/** The passage as one line: the corpus wraps its lines and the hash must not
 * move because the file wraps them differently. */
export function normalizePassage(text) {
  return String(text).replace(/\s+/g, " ").trim();
}

/** The hash a table stores beside a ref. */
export function passageHash(text) {
  const digest = createHash("sha256").update(normalizePassage(text), "utf8").digest("hex");
  return `sha256:${digest}`;
}

// The number words the sources write, with the inflections they use.
const NUMBER_WORDS = new Map(Object.entries({
  unus: 1, una: 1, unum: 1, duo: 2, duae: 2, duos: 2, tres: 3, tria: 3, tribus: 3, terni: 3,
  quattuor: 4, quatuor: 4, quinque: 5, sex: 6, septem: 7, octo: 8, nouem: 9, novem: 9,
  decem: 10, denum: 10, undecim: 11, duodecim: 12, tredecim: 13, quattuordecim: 14,
  viginti: 20, triginta: 30, quadraginta: 40, quinquaginta: 50, sexaginta: 60,
  septuaginta: 70, octoginta: 80, nonaginta: 90, centum: 100, ducenti: 200, trecenti: 300,
  quadringenti: 400, quingenti: 500, sescenti: 600, septingenti: 700, octingenti: 800,
  nongenti: 900, mille: 1000, milia: 1000, millia: 1000,
}));

/** The number a spelling carries: `quinque` is 5, `VIII` is 8, `Decem milia` is
 * 10000. Null when the spelling is no number. */
export function numberFromForm(form) {
  const words = String(form).trim().split(/\s+/).filter(Boolean);
  if (!words.length) return null;
  let total = 1;
  for (const word of words) {
    let value = null;
    if (/^[IVXLCDM]+$/.test(word)) value = romanToInt(word);
    else value = NUMBER_WORDS.get(word.toLowerCase()) ?? null;
    if (value === null) return null;
    total *= value;
  }
  return total;
}

/** Whether a passage carries a spelling, as a whole word or phrase. */
export function hasForm(text, form) {
  const words = String(form).trim().split(/\s+/).filter(Boolean);
  if (!words.length) return false;
  const pattern = words.map(escapeRe).join("\\s+");
  return new RegExp(`(?<![\\p{L}\\p{N}])${pattern}(?![\\p{L}\\p{N}])`, "u").test(String(text));
}

/** The codex rows of a table: the sections under `codex.`. */
export function codexRows(sections) {
  const rows = [];
  for (const [name, values] of sections) {
    if (!name.startsWith("codex.")) continue;
    rows.push({
      name,
      entry: values.get("entry") ?? "",
      key: values.get("key") ?? "",
      ref: values.get("ref") ?? "",
      form: values.get("form") ?? "",
      hash: values.get("hash") ?? "",
    });
  }
  return rows;
}

/** Fetch one corpus file into the cache. The branch name is tried in turn, so a
 * renamed default branch does not stop the check. */
async function fetchFile(file, corpusDir) {
  for (const branch of BRANCHES) {
    const url = `${RAW}/${branch}/${file.path}`;
    const res = await fetch(url, { headers: { "User-Agent": UA } });
    if (res.status === 404) continue;
    if (!res.ok) throw new Error(`${url}: http ${res.status}`);
    const body = Buffer.from(await res.arrayBuffer());
    mkdirSync(corpusDir, { recursive: true });
    writeFileSync(join(corpusDir, file.name), body);
    return { url, bytes: body.length };
  }
  throw new Error(`not found on ${BRANCHES.join(" or ")}: ${file.path}`);
}

/** Read the corpus, fetching what is missing when the run may fetch. */
export async function loadCorpus(corpusDir, { fetch = false, log = () => {} } = {}) {
  mkdirSync(corpusDir, { recursive: true });
  const missing = CORPUS_FILES.filter((file) => !existsSync(join(corpusDir, file.name)));
  if (missing.length && !fetch) {
    const names = missing.map((file) => file.name).join(", ");
    log(`codex: missing corpus file(s): ${names}`);
    log(`codex: fetch them with: node tools/codex-check.mjs --fetch`);
    return null;
  }
  for (const file of missing) {
    const got = await fetchFile(file, corpusDir);
    log(`codex: fetched ${file.name} (${got.bytes} bytes) from ${got.url}`);
  }
  const hyginus = splitHyginus(readFileSync(join(corpusDir, "hyginus6.txt"), "utf8"));
  const vegetius = splitVegetius(readFileSync(join(corpusDir, "vegetius1.txt"), "utf8"));
  return { hyginus, vegetius };
}

/** Check every codex row of one table file against the corpus. */
export function checkTable({ name, file, sections, corpus }) {
  const units = sections.get("units");
  const mmPerFt = units?.get("mm_per_ft");
  if (typeof mmPerFt !== "number" || mmPerFt <= 0) {
    throw new Error(`${file}: the units section has no mm_per_ft`);
  }
  const rows = [];
  for (const row of codexRows(sections)) {
    const entry = sections.get(row.entry);
    const value = entry?.get(row.key);
    if (typeof value !== "number") {
      throw new Error(`${file}: the codex row ${row.name} names no number at ${row.entry}.${row.key}`);
    }
    const passage = passageFor(row.ref, corpus);
    if (!passage) {
      throw new Error(`${file}: the codex row ${row.name} names no passage for ${row.ref}`);
    }
    const source = numberFromForm(row.form);
    if (source === null) {
      throw new Error(`${file}: the codex row ${row.name} writes no number as ${row.form}`);
    }
    const digest = passageHash(passage.text);
    const label = `${name} ${row.entry}.${row.key}`;
    if (digest !== row.hash) {
      rows.push({
        label,
        ref: row.ref,
        status: "changed",
        detail: `passage is ${digest}, the table holds ${row.hash}`,
      });
      continue;
    }
    const text = normalizePassage(passage.text);
    const mm = value * mmPerFt;
    if (!hasForm(text, row.form) || source !== value) {
      rows.push({
        label,
        ref: row.ref,
        status: "unmatched",
        detail: `the source writes ${row.form} = ${source} ft, the table says ${value} ft`,
      });
      continue;
    }
    rows.push({
      label,
      ref: row.ref,
      status: "ok",
      detail: `${row.form} = ${source} ft`,
      mm,
      value,
    });
  }
  return rows;
}

/** Check the shipped tables against the corpus. */
export async function run({
  corpusDir = DEFAULT_CORPUS,
  campFile = DEFAULT_CAMP,
  lineFile = DEFAULT_LINE,
  fetch = false,
  log = (line) => process.stdout.write(`${line}\n`),
} = {}) {
  const corpus = await loadCorpus(corpusDir, { fetch, log });
  if (!corpus) return { ok: false, code: 2, numbers: 0, unmatched: 0, changed: 0 };
  let rows = [];
  try {
    for (const [name, file] of [["camp", campFile], ["line", lineFile]]) {
      if (!existsSync(file)) throw new Error(`no table at ${file}`);
      const sections = parseTable(readFileSync(file, "utf8"));
      rows = rows.concat(checkTable({ name, file, sections, corpus }));
    }
  } catch (err) {
    log(`codex: error: ${err.message}`);
    return { ok: false, code: 2, numbers: 0, unmatched: 0, changed: 0 };
  }
  for (const row of rows) {
    if (row.status === "ok") {
      log(`${row.label}  ${row.ref}  ${row.detail}  ${row.mm} mm  table ${row.value} ft  ok`);
    } else {
      log(`${row.label}  ${row.ref}  ${row.status}  ${row.detail}`);
    }
  }
  const unmatched = rows.filter((row) => row.status === "unmatched").length;
  const changed = rows.filter((row) => row.status === "changed").length;
  const summary = `codex: numbers=${rows.length} unmatched=${unmatched}`;
  log(changed ? `${summary} changed=${changed}` : summary);
  return { ok: unmatched === 0 && changed === 0, code: unmatched || changed ? 1 : 0, numbers: rows.length, unmatched, changed };
}

function parseArgs(args) {
  const out = {
    corpusDir: DEFAULT_CORPUS,
    campFile: DEFAULT_CAMP,
    lineFile: DEFAULT_LINE,
    fetch: false,
    help: false,
  };
  for (let i = 0; i < args.length; i += 1) {
    const arg = args[i];
    if (arg === "--help" || arg === "-h") {
      out.help = true;
      continue;
    }
    if (arg === "--fetch") {
      out.fetch = true;
      continue;
    }
    if (arg === "--corpus" || arg === "--camp" || arg === "--line") {
      const value = args[i + 1];
      if (!value || value.startsWith("--")) throw new Error(`${arg} needs a value`);
      if (arg === "--corpus") out.corpusDir = resolve(value);
      else if (arg === "--camp") out.campFile = resolve(value);
      else out.lineFile = resolve(value);
      i += 1;
      continue;
    }
    throw new Error(`unknown argument ${arg}`);
  }
  return out;
}

if (import.meta.main) {
  try {
    const opts = parseArgs(process.argv.slice(2));
    if (opts.help) {
      process.stdout.write(HELP);
    } else {
      const result = await run(opts);
      process.exitCode = result.code;
    }
  } catch (err) {
    process.stderr.write(`codex: error: ${err.message}\n`);
    process.exit(2);
  }
}
