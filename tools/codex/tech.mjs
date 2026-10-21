#!/usr/bin/env node
// tech.mjs - the Rome codex tech table drafts (CDX4).
//
// Job: read the codex passage index (CDX2), pick the passages whose topics
// match a tech table and write one DRAFT toml per table into
// content/rome/tech/. Every draft line holds the number with its unit, the
// ref of the passage it came from and the source (url, licence), so a
// reader can check the passage. A person or the lead promotes a line into
// the real table; no crate reads a draft.
//
// Usage:
//   node tools/codex/tech.mjs [--index FILE] [--out DIR] [--dry-run] [--help]
//
// Rules:
//   - every file written ends with .draft.toml and carries `draft = true`.
//   - a number without a unit is not a measure and is dropped.
//   - a passage with no ref, no measure or no topic a table wants is
//     skipped.
//   - `place` and `year` are no tech tables: places go to the places table
//     and years to the timeline.
//
// Exit: 0 ok, 1 the index is missing or holds no passage, 2 bad CLI.
import { existsSync, mkdirSync, readFileSync, statSync, writeFileSync } from "node:fs";
import { dirname, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { findNumbers, TOPICS } from "./index.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));
const REPO = resolve(HERE, "..", "..");
export const DEFAULT_INDEX = join(REPO, "content", "local", "codex", "index.jsonl");
export const DEFAULT_OUT = join(REPO, "content", "rome", "tech");

// The tech tables and the topics that feed them. A passage may feed more
// than one table: the camp passage also holds the march of the army.
export const TABLES = {
  camp: ["camp"],
  march: ["march"],
  bridge: ["bridge"],
  road: ["road"],
  siege: ["siege"],
  craft: ["craft"],
  tool: ["tool"],
};

for (const [table, topics] of Object.entries(TABLES)) {
  for (const topic of topics) {
    if (!TOPICS.includes(topic)) throw new Error(`the table ${table} wants the unknown topic ${topic}`);
  }
}

// The things a tech measure can be about. The nearest one before the number
// names what the measure gives: the ditch of "the ditch is 5 ft wide".
const SUBJECTS = new Set([
  "abutment", "agger", "arrow", "ball", "ballista", "beam", "bolt", "bridge", "camp",
  "castra", "castrum", "catapult", "causeway", "city", "column", "crane", "ditch",
  "engine", "ford", "formation", "fort", "fossa", "fence", "frontage", "gate", "gladius",
  "guard", "helmet", "javelin", "legion", "line", "murus", "oppidum", "palisade",
  "pier", "pilum", "plank", "pons", "porta", "post", "rampart", "rank", "river", "road",
  "scutum", "shield", "siege", "sling", "spear", "step", "stone", "street", "tent",
  "tower", "town", "turret", "vallum", "valla", "via", "watch",
]);

// The word after the number says how the measure is taken.
const QUALITY = new Map([
  ["wide", "width"],
  ["broad", "width"],
  ["deep", "depth"],
  ["high", "height"],
  ["tall", "height"],
  ["long", "length"],
  ["short", "length"],
  ["thick", "thickness"],
]);

const STOP = /[.;:!?]/;

/** The sentences of a passage as one line: the fetched texts wrap their
 * lines, and a wrap is no sentence end. */
function flatten(text) {
  return String(text ?? "").replace(/\s+/g, " ").trim();
}

/** The words of a text, without the punctuation. */
function words(text) {
  return String(text ?? "").match(/[A-Za-z]+/g) ?? [];
}

/** The part of the text before and after the number, cut at the sentence
 * ends. */
function around(text, numberText) {
  const flat = flatten(text);
  const at = flat.indexOf(numberText);
  if (at < 0) return { before: flat, after: "" };
  let start = 0;
  for (let i = at - 1; i >= 0; i -= 1) {
    if (STOP.test(flat[i])) {
      start = i + 1;
      break;
    }
  }
  let end = flat.length;
  for (let i = at; i < flat.length; i += 1) {
    if (STOP.test(flat[i])) {
      end = i;
      break;
    }
  }
  return { before: flat.slice(start, at), after: flat.slice(at + numberText.length, end) };
}

/** The nearest thing a measure is about before the number, else after it. */
function subjectOf(before, after) {
  for (const list of [words(before).reverse(), words(after)]) {
    for (const word of list) {
      const lower = word.toLowerCase();
      if (SUBJECTS.has(lower)) return lower;
    }
  }
  return "";
}

/** The label of a measure: what it gives and how, read from the sentence the
 * number sits in. "5 ft wide" in a sentence about the ditch is "ditch
 * width". */
export function measureLabel(text, number, fallback) {
  const { before, after } = around(text, number.text);
  const subject = subjectOf(before, after) || fallback;
  const first = words(after)[0]?.toLowerCase() ?? "";
  const quality = QUALITY.get(first) ?? "";
  return {
    subject,
    quality,
    label: quality ? `${subject} ${quality}` : subject,
  };
}

/** A toml key from words: lower case, no dot, no space. */
export function slug(text) {
  return String(text ?? "")
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "_")
    .replace(/^_+|_+$/g, "");
}

/** A toml string: the value in quotes, the quotes and the slashes safe. */
export function tomlString(value) {
  const body = String(value ?? "")
    .replace(/\\/g, "\\\\")
    .replace(/"/g, '\\"')
    .replace(/\r/g, "\\r")
    .replace(/\n/g, "\\n")
    .replace(/[\u0000-\u0008\u000B\u000C\u000E-\u001F\u007F]/g, "");
  return `"${body}"`;
}

/** The index as records: one passage per line of json. */
export function readIndex(file) {
  const lines = readFileSync(file, "utf8").split(/\r?\n/);
  const records = [];
  const bad = [];
  lines.forEach((line, i) => {
    const trimmed = line.trim();
    if (!trimmed) return;
    try {
      const record = JSON.parse(trimmed);
      if (record && typeof record === "object" && !Array.isArray(record)) records.push(record);
      else bad.push(i + 1);
    } catch {
      bad.push(i + 1);
    }
  });
  return { records, bad };
}

/** The measures of a passage: the numbers with a unit. The index holds them
 * already; a record without the field is measured again from its text. A
 * number without a unit is no measure and is left out. */
export function measuresOf(record) {
  const raw = Array.isArray(record.numbers) ? record.numbers : findNumbers(record.text ?? "");
  const out = [];
  for (const number of raw) {
    const value = Number(number?.value);
    const unit = String(number?.unit ?? "").trim();
    if (!Number.isFinite(value) || !unit) continue;
    out.push({
      value,
      unit,
      text: String(number?.text ?? `${value} ${unit}`).trim(),
    });
  }
  return out;
}

/** The passages that feed a table: those with a ref, with a measure and with
 * a topic the table wants. */
export function pickPassages(records, tables = TABLES) {
  const picks = [];
  const skipped = [];
  for (const record of records) {
    const ref = String(record.ref ?? "").trim();
    if (!ref) {
      skipped.push({ record, why: "no ref" });
      continue;
    }
    const numbers = measuresOf(record);
    if (!numbers.length) {
      skipped.push({ record, why: "no measure" });
      continue;
    }
    const topics = Array.isArray(record.topics) ? record.topics : [];
    const tablesOf = Object.keys(tables).filter((table) =>
      tables[table].some((topic) => topics.includes(topic)));
    if (!tablesOf.length) {
      skipped.push({ record, why: "no topic a table wants" });
      continue;
    }
    for (const table of tablesOf) picks.push({ table, record, ref, numbers });
  }
  return { picks, skipped };
}

/** The draft entries of one table: one entry per passage, with its measures
 * and the keys they get. */
export function entriesFor(picks, table) {
  const entries = [];
  const refs = new Map();
  for (const pick of picks) {
    if (pick.table !== table) continue;
    const base = slug(pick.ref) || "passage";
    const seen = (refs.get(base) ?? 0) + 1;
    refs.set(base, seen);
    const key = seen === 1 ? base : `${base}_${seen}`;
    const measures = [];
    const keys = new Map();
    for (const number of pick.numbers) {
      const label = measureLabel(pick.record.text ?? "", number, table);
      const measureKey = slug([label.subject, label.quality, number.unit].filter(Boolean).join("_"));
      const used = (keys.get(measureKey) ?? 0) + 1;
      keys.set(measureKey, used);
      measures.push({
        key: used === 1 ? measureKey : `${measureKey}_${used}`,
        value: number.value,
        unit: number.unit,
        text: number.text,
        label: label.label,
      });
    }
    entries.push({ key, ref: pick.ref, record: pick.record, measures });
  }
  return entries;
}

/** The draft toml of one table: a header that says it is a draft and one
 * table per measure, each with the number, the unit, the ref and the
 * source. */
export function draftToml(table, entries, indexFile = "") {
  const lines = [];
  lines.push("# DRAFT: a codex tech table, not game data. No crate reads this file.");
  lines.push("# Generated by tools/codex/tech.mjs from the codex passage index.");
  lines.push("# A person or the lead checks the passage and promotes a line into");
  lines.push("# the real table. Nothing here ships.");
  lines.push("draft = true");
  lines.push(`table = ${tomlString(table)}`);
  if (indexFile) lines.push(`index = ${tomlString(indexFile)}`);
  lines.push("");
  for (const entry of entries) {
    const record = entry.record ?? {};
    for (const measure of entry.measures) {
      lines.push(`[${entry.key}.${measure.key}]`);
      lines.push(`value = ${measure.value}`);
      lines.push(`unit = ${tomlString(measure.unit)}`);
      lines.push(`label = ${tomlString(measure.label)}`);
      lines.push(`ref = ${tomlString(entry.ref)}`);
      lines.push(`work = ${tomlString(record.work ?? "")}`);
      lines.push(`book = ${Number(record.book) || 0}`);
      lines.push(`chapter = ${Number(record.chapter) || 0}`);
      lines.push(`section = ${Number(record.section) || 0}`);
      lines.push(`title = ${tomlString(record.title ?? "")}`);
      lines.push(`author = ${tomlString(record.author ?? "")}`);
      lines.push(`url = ${tomlString(record.url ?? "")}`);
      lines.push(`licence = ${tomlString(record.licence ?? "")}`);
      lines.push("");
    }
  }
  return lines.join("\n");
}

/** The index path as the repo names it, or the whole path when it lies
 * outside the repo. */
function indexPath(indexFile) {
  const rel = relative(REPO, indexFile);
  const path = rel && !rel.startsWith("..") ? rel : indexFile;
  return path.split("\\").join("/");
}

/** Pull the tech tables out of the codex index and write the drafts. */export function run({
  indexFile = DEFAULT_INDEX,
  outDir = DEFAULT_OUT,
  dryRun = false,
  log = (line) => process.stdout.write(`${line}\n`),
} = {}) {
  if (!existsSync(indexFile)) {
    log(`no codex index at ${indexFile}`);
    return { ok: false, code: 1, picks: 0, files: [], tables: [] };
  }
  const { records, bad } = readIndex(indexFile);
  if (!records.length) {
    log(`the codex index holds no passage: ${indexFile}`);
    return { ok: false, code: 1, picks: 0, files: [], tables: [] };
  }
  for (const line of bad) log(`the codex index line ${line} is no json: skipped`);
  const { picks, skipped } = pickPassages(records);
  for (const pick of picks) {
    for (const measure of pick.numbers) {
      const label = measureLabel(pick.record.text ?? "", measure, pick.table);
      log(`${pick.table}: ${pick.ref} ${measure.text} ${label.label}`);
    }
  }
  const files = [];
  for (const table of Object.keys(TABLES)) {
    const entries = entriesFor(picks, table);
    if (!entries.length) continue;
    const file = join(outDir, `${table}.draft.toml`);
    const text = draftToml(table, entries, indexPath(indexFile));
    if (dryRun) {
      log(`would write ${file} entries=${entries.length}`);
    } else {
      mkdirSync(outDir, { recursive: true });
      writeFileSync(file, text, "utf8");
      log(`wrote ${file} entries=${entries.length}`);
    }
    files.push(file);
  }
  if (!files.length) log("no passage feeds a tech table");
  const skippedWhy = new Map();
  for (const skip of skipped) skippedWhy.set(skip.why, (skippedWhy.get(skip.why) ?? 0) + 1);
  for (const [why, count] of skippedWhy) log(`skipped ${count} passage(s): ${why}`);
  return { ok: true, code: 0, picks: picks.length, files, tables: Object.keys(TABLES) };
}

export const HELP = `Usage: node tools/codex/tech.mjs [--index FILE] [--out DIR] [--dry-run] [--help]

Pulls the numbers with units and refs out of the codex passage index and
writes one DRAFT toml per tech table into content/rome/tech/ (camp, march,
bridge, road, siege, craft, tool). Every file it writes ends with
.draft.toml and carries draft = true: no crate reads it. A person or the
lead checks the passage and promotes a line into the real table.

Prints one line per measure of every passage it picks, for example
  camp: vegetius.3.8 5 ft ditch width

  --index FILE  the passage index (content/local/codex/index.jsonl)
  --out DIR     where the drafts go (content/rome/tech)
  --dry-run     print the plan, write nothing
  --help        print this help

Exit 0 ok, 1 the index is missing or holds no passage, 2 bad CLI.
`;

function parseArgs(args) {
  const out = {
    indexFile: DEFAULT_INDEX,
    outDir: DEFAULT_OUT,
    dryRun: false,
    help: false,
  };
  for (let i = 0; i < args.length; i += 1) {
    const arg = args[i];
    if (arg === "--help" || arg === "-h") {
      out.help = true;
      continue;
    }
    if (arg === "--dry-run") {
      out.dryRun = true;
      continue;
    }
    if (arg === "--index" || arg === "--out") {
      const value = args[i + 1];
      if (!value || value.startsWith("--")) throw new Error(`${arg} needs a value`);
      if (arg === "--index") out.indexFile = resolve(value);
      else out.outDir = resolve(value);
      i += 1;
      continue;
    }
    if (arg.startsWith("--index=")) {
      out.indexFile = resolve(arg.slice("--index=".length));
      continue;
    }
    if (arg.startsWith("--out=")) {
      out.outDir = resolve(arg.slice("--out=".length));
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
      if (existsSync(opts.outDir) && !statSync(opts.outDir).isDirectory()) {
        throw new Error(`${opts.outDir} is a file, not a folder`);
      }
      const result = run(opts);
      process.exitCode = result.code;
    }
  } catch (error) {
    process.stderr.write(`tech: error: ${error.message}\n`);
    process.exitCode = 2;
  }
}
