#!/usr/bin/env node
// credits.mjs - the Rome credits page (CDX8).
//
// Job: write `content/rome/CREDITS.md`: every source the shipped Rome files
// use, with the url it came from and the licence it states.
//
// The credit of a source comes from three places, in this order:
//   1. the sidecar of a fetched file: `content/local/codex/<source>/<file>.src.json`
//      and the boards cache `content/local/codex/boards/<id>.src.json`. A sidecar
//      holds the url, the fetch time, the sha256 and the licence the site states.
//   2. the `sources` block a shipped Rome file carries: the same fields, copied
//      out of the sidecar when the file was written.
//   3. the source table `tools/codex/sources.json`: the label and the stated
//      licence of a source whose file the local cache does not hold.
//
// Which source a Rome file uses is read out of the file itself: the source id,
// the url of a unit, the `sources` block, a ref that names a work of the codex,
// or the Latin corpus the camp and line tables cite. A ref that names nothing
// in the codex is reported and never guessed.
//
// Usage:
//   node tools/codex/credits.mjs [--rome DIR] [--cache DIR] [--sources FILE]
//                                 [--out FILE] [--help]
//
// Rules:
//   - every source a Rome file names gets a section, sidecar or not.
//   - the page names the source, the url and the stated licence of every file.
//   - a ref that names no source of the codex is reported, not credited.
//   - the same input twice gives the same bytes.
//
// Exit: 0 ok, 1 no Rome file to read or the write failed, 2 bad CLI.
import { mkdirSync, readFileSync, readdirSync, renameSync, writeFileSync } from "node:fs";
import { dirname, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { parseSources } from "./fetch.mjs";
import { WORKS } from "./index.mjs";
import { CORPUS_FILES } from "../codex-check.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));
const REPO = resolve(HERE, "..", "..");
export const DEFAULT_ROME = join(REPO, "content", "rome");
export const DEFAULT_CACHE = join(REPO, "content", "local", "codex");
export const DEFAULT_SOURCES = join(HERE, "sources.json");
export const DEFAULT_OUT = join(REPO, "content", "rome", "CREDITS.md");

// The source table of the codex. The table states the licence of every source.
export { parseSources as parseRegistry } from "./fetch.mjs";

// The Latin corpus the Rome camp and line tables cite: the Pseudo-Hyginus and
// the Vegetius of cltk/lat_text_latin_library. The licence is the one the
// corpus states, the same string tools/codex-check.mjs names.
export const CORPUS_ID = "lat_text_latin_library";
export const CORPUS_LABEL = "cltk/lat_text_latin_library, the Latin texts of the codex";
export const CORPUS_LICENCE = "Public Domain Mark 1.0";
// The codes of the corpus passages, the two tools/codex-check.mjs reads.
const CORPUS_CODES = { PHG: "hyginus", VEG: "vegetius" };
// Where the corpus lives. tools/codex-check.mjs fetches the same files from the
// same place, but it keeps its base url to itself.
const CORPUS_RAW = "https://raw.githubusercontent.com/cltk/lat_text_latin_library";
// A ref that names the plan is a citation of our own plan, not of a source.
const PLAN_WORDS = ["rome.md", "docs/plan"];
// The shortest phrase that may name an item of a source. A short one would
// match by accident ("places" in a sentence about places).
const ITEM_MIN = 8;

export const HELP = `Usage: node tools/codex/credits.mjs [--rome DIR] [--cache DIR] [--sources FILE] [--out FILE] [--help]

Writes content/rome/CREDITS.md: every source the shipped Rome files use, with
the url it came from and the licence it states. The credit of a source comes
from the sidecar of the fetched file, else from the sources block the Rome file
carries, else from the source table tools/codex/sources.json.

  --rome DIR      the Rome files to read (content/rome)
  --cache DIR     the codex cache with the sidecars (content/local/codex)
  --sources FILE  the source table (tools/codex/sources.json)
  --out FILE      the page to write (content/rome/CREDITS.md)
  --help          print this help

Exit 0 ok, 1 no Rome file to read, 2 bad CLI.
`;

const escapeRe = (s) => String(s).replace(/[.*+?^${}()|[\]\\]/g, "\\$&");

/** Does the text hold the phrase? The phrase must stand on its own, so "csv"
 *  never matches inside a longer word. */
export function hasPhrase(text, phrase, minLength = 4) {
  const body = String(phrase ?? "");
  if (body.length < minLength) return false;
  return new RegExp(`(?<![\\w])${escapeRe(body)}(?![\\w])`, "i").test(String(text ?? ""));
}

/** The shipped Rome files: the tables and the json, the drafts left out. */
export function romeFileList(dir) {
  const out = [];
  const stack = [dir];
  while (stack.length) {
    const current = stack.pop();
    let entries = [];
    try {
      entries = readdirSync(current, { withFileTypes: true });
    } catch {
      continue;
    }
    for (const entry of entries) {
      const full = join(current, entry.name);
      if (entry.isDirectory()) {
        stack.push(full);
        continue;
      }
      if (!entry.isFile()) continue;
      if (!entry.name.endsWith(".toml") && !entry.name.endsWith(".json")) continue;
      if (entry.name.endsWith(".draft.toml")) continue;
      out.push({
        path: relative(dir, full).split("\\").join("/"),
        text: readFileSync(full, "utf8"),
      });
    }
  }
  out.sort((a, b) => (a.path < b.path ? -1 : a.path > b.path ? 1 : 0));
  return out;
}

/** Every string of a json value. */
export function jsonStrings(value, out = []) {
  if (typeof value === "string") {
    out.push(value);
    return out;
  }
  if (Array.isArray(value)) {
    for (const item of value) jsonStrings(item, out);
    return out;
  }
  if (value && typeof value === "object") {
    for (const item of Object.values(value)) jsonStrings(item, out);
  }
  return out;
}

/** The refs of a json value: the `ref` fields it holds. */
export function jsonRefs(value, out = []) {
  if (Array.isArray(value)) {
    for (const item of value) jsonRefs(item, out);
    return out;
  }
  if (value && typeof value === "object") {
    for (const [key, item] of Object.entries(value)) {
      if (key === "ref" && typeof item === "string") out.push(item);
      else jsonRefs(item, out);
    }
  }
  return out;
}

/** The source blocks of a json file: the records of the sources it used. */
export function sourceBlocks(value, out = []) {
  if (Array.isArray(value)) {
    for (const item of value) sourceBlocks(item, out);
    return out;
  }
  if (value && typeof value === "object") {
    for (const [key, item] of Object.entries(value)) {
      if (key !== "sources" || !Array.isArray(item)) {
        sourceBlocks(item, out);
        continue;
      }
      for (const entry of item) {
        if (!entry || typeof entry !== "object" || typeof entry.id !== "string") continue;
        out.push({
          id: entry.id,
          file: typeof entry.file === "string" ? entry.file : "",
          url: typeof entry.url === "string" ? entry.url : "",
          licence: typeof entry.licence === "string" ? entry.licence : "",
          sha256: typeof entry.sha256 === "string" ? entry.sha256 : "",
        });
      }
    }
  }
  return out;
}

/** The refs of a Rome table: every `ref` line it holds. */
export function tomlRefs(text) {
  const out = [];
  for (const raw of String(text ?? "").split(/\r?\n/)) {
    const line = raw.trim();
    if (!line || line.startsWith("#")) continue;
    const pair = /^ref\s*=\s*"([^"]*)"/.exec(line);
    if (pair) out.push(pair[1]);
  }
  return out;
}

/** The provenance sections of a Rome table: the donor files it was cut from.
 *  A section `[provenance.<id>]` holds the id, the file, the url, the licence
 *  and the sha256 of the donor, the same fields a sidecar holds. */
export function tomlProvenance(text) {
  const out = [];
  const sections = new Map();
  let current = "";
  for (const raw of String(text ?? "").split(/\r?\n/)) {
    const line = raw.trim();
    if (!line || line.startsWith("#")) continue;
    const header = /^\[([^\[\]]+)\]$/.exec(line);
    if (header) {
      current = header[1].trim();
      if (!current.startsWith("provenance.")) {
        current = "";
        continue;
      }
      if (!sections.has(current)) sections.set(current, new Map());
      continue;
    }
    if (!current) continue;
    const pair = /^([A-Za-z_][A-Za-z0-9_]*)\s*=\s*(.*)$/.exec(line);
    if (!pair) continue;
    const value = pair[2].trim();
    sections.get(current).set(pair[1], value.startsWith('"') && value.endsWith('"') ? value.slice(1, -1) : value);
  }
  for (const [section, values] of sections) {
    const id = values.get("id") ?? section.slice("provenance.".length);
    if (!id) continue;
    out.push({
      id,
      file: values.get("file") ?? "",
      url: values.get("url") ?? "",
      licence: values.get("licence") ?? "",
      sha256: values.get("sha256") ?? "",
    });
  }
  return out;
}

/** The source records of a Rome file: the `sources` blocks of a json and the
 *  `provenance` sections of a table. */
export function fileRecords(file) {
  if (!file.path.endsWith(".json")) return tomlProvenance(file.text);
  let data = null;
  try {
    data = JSON.parse(file.text);
  } catch {
    return [];
  }
  return sourceBlocks(data);
}

/** The works of the codex: the work key and the files that hold it. */
export function workIndex(works = WORKS) {
  const map = new Map();
  for (const [key, entry] of Object.entries(works ?? {})) {
    const source = key.split("/")[0];
    const work = String(entry?.work ?? "").toLowerCase();
    if (!work || !source) continue;
    if (!map.has(work)) map.set(work, []);
    map.get(work).push({ source, file: key.slice(source.length + 1) });
  }
  return map;
}

/** The corpus files: the Latin texts the camp and line tables cite. */
export function corpusFiles() {
  return CORPUS_FILES.map((file) => ({
    key: `${CORPUS_ID}/${file.name}`,
    name: file.name,
    url: `${CORPUS_RAW}/master/${file.path}`,
  }));
}

/** The corpus file a passage code names: "PHG 49" is the Pseudo-Hyginus. */
export function corpusFileFor(code) {
  const word = CORPUS_CODES[String(code ?? "").toUpperCase()] ?? "";
  const found = corpusFiles().find((file) => corpusWord(file.name) === word);
  return found?.name ?? "";
}

/** The word a corpus file is named after: "hyginus6.txt" is "hyginus". */
export function corpusWord(name) {
  return String(name ?? "")
    .replace(/\.[^.]+$/, "")
    .replace(/\d+$/, "")
    .toLowerCase();
}

/** The source a ref names when it names one outright: "DARE:fortifications.geojson,
 *  the feature the atlas names ...". The id may be the one of the source table
 *  or the one of a donor file a Rome table carries. */
export function sourceIdNamed(id, registry, records) {
  const want = String(id ?? "").toLowerCase();
  const source = registry.sources.find((item) => item.id.toLowerCase() === want);
  if (source) return source.id;
  for (const [key, files] of records) {
    if (key.toLowerCase() === want) return files.values().next().value?.id ?? key;
  }
  return "";
}

/** The file a ref names after the source: the first word of the rest, when it
 *  carries an extension. */
export function fileNameOf(text) {
  const token = String(text ?? "").trim().split(/[\s,]/)[0] ?? "";
  return token.includes(".") ? token : "";
}

/** The source a ref names: a work of the codex, an item of a source, a donor
 *  file or a passage of the corpus. Nothing when the ref names nothing we hold. */
export function resolveRef(ref, registry, records = new Map(), works = WORKS) {
  const text = String(ref ?? "");
  const named = /^([A-Za-z][A-Za-z0-9_.-]*):\s*(.*)$/.exec(text.trim());
  if (named) {
    const source = sourceIdNamed(named[1], registry, records);
    if (!source) return {};
    const file = fileNameOf(named[2]);
    return { source, files: file ? [file] : [] };
  }
  const code = /^(PHG|VEG)\s+\d+/.exec(text.trim());
  if (code) {
    const name = corpusFileFor(code[1]);
    return name ? { corpus: name } : {};
  }
  const index = workIndex(works);
  let best = null;
  for (const [work, files] of index) {
    if (!hasPhrase(text, work, 3)) continue;
    if (best && work.length <= best.work.length) continue;
    best = { work, files };
  }
  if (best) return { source: best.files[0].source, files: best.files.map((file) => file.file) };
  for (const file of corpusFiles()) {
    if (hasPhrase(text, corpusWord(file.name), 4)) return { corpus: file.name };
  }
  for (const source of registry.sources) {
    for (const unit of source.units) {
      if (hasPhrase(text, unit.id, ITEM_MIN)) return { source: source.id, files: [unit.name] };
    }
  }
  return {};
}

/** The sources a Rome file uses: the ids, the files of each id, the corpus
 *  files and the refs that named nothing. */
export function fileCitations(file, registry, records = new Map(), works = WORKS) {
  const ids = new Map();
  const corpus = new Set();
  const unresolved = [];
  const add = (id, name = "") => {
    if (!id) return;
    if (!ids.has(id)) ids.set(id, new Set());
    if (name) ids.get(id).add(name);
  };

  const refs = [];
  const strings = [];
  if (file.path.endsWith(".json")) {
    let data = null;
    try {
      data = JSON.parse(file.text);
    } catch {
      data = null;
    }
    if (data) {
      jsonStrings(data, strings);
      refs.push(...jsonRefs(data));
    }
  } else {
    refs.push(...tomlRefs(file.text));
  }

  // The source record of the file: the `sources` block of a json, the
  // `provenance` section of a table.
  for (const record of fileRecords(file)) {
    add(record.id, record.file);
  }

  // The source id and the url of a unit: the file names the source outright.
  for (const source of registry.sources) {
    if (strings.includes(source.id)) add(source.id);
    for (const unit of source.units) {
      if (strings.includes(unit.url)) add(source.id, unit.name);
    }
  }

  // The api of a source: the file holds the url the source answers on. Two
  // sources may share the api (Commons has two), so the one whose item the
  // file names wins.
  const candidates = registry.sources.filter((source) =>
    strings.some((value) => value.startsWith("http") && value.length >= 12 && source.url.startsWith(value)),
  );
  const named = candidates.filter((source) => source.units.some((unit) => hasPhrase(file.text, unit.id, ITEM_MIN)));
  for (const source of named) add(source.id);

  // The refs: a work of the codex, an item of a source, a donor file, a
  // corpus passage.
  for (const ref of refs) {
    const hit = resolveRef(ref, registry, records, works);
    if (hit.corpus) {
      corpus.add(hit.corpus);
      continue;
    }
    if (hit.source) {
      add(hit.source);
      for (const name of hit.files ?? []) add(hit.source, name);
      continue;
    }
    if (PLAN_WORDS.some((word) => ref.includes(word))) continue;
    unresolved.push({ path: file.path, ref });
  }
  return { ids, corpus, unresolved };
}

/** The sidecars of the cache: the url, the fetch time, the sha256 and the
 *  licence of every fetched file. */
export function readSidecars(cacheDir) {
  const out = [];
  const stack = [cacheDir];
  while (stack.length) {
    const dir = stack.pop();
    let entries = [];
    try {
      entries = readdirSync(dir, { withFileTypes: true });
    } catch {
      continue;
    }
    for (const entry of entries) {
      const full = join(dir, entry.name);
      if (entry.isDirectory()) {
        stack.push(full);
        continue;
      }
      if (!entry.name.endsWith(".src.json")) continue;
      let raw = null;
      try {
        raw = JSON.parse(readFileSync(full, "utf8"));
      } catch {
        continue;
      }
      const key = relative(cacheDir, full).split("\\").join("/").replace(/\.src\.json$/, "");
      out.push({
        source: String(raw.source ?? "") || key.split("/")[0],
        key,
        url: String(raw.url ?? ""),
        licence: String(raw.licence ?? ""),
        sha256: String(raw.sha256 ?? ""),
        fetchedAt: String(raw.fetchedAt ?? ""),
      });
    }
  }
  out.sort((a, b) => (a.key < b.key ? -1 : a.key > b.key ? 1 : 0));
  return out;
}

/** Is the sidecar of a file the Rome content used? A fetched file counts when
 *  a Rome file names it; an answer of a request counts when its source is
 *  used, because the boards cache holds one answer per request. */
export function sidecarUsed(sidecar, names) {
  const folder = sidecar.key.split("/")[0];
  const name = sidecar.key.slice(folder.length + 1);
  if (folder !== sidecar.source) return true;
  return names.has(name);
}

/** The credits of every source the Rome files use. */
export function buildCredits({ files = [], registry, sidecars = [], works = WORKS } = {}) {
  const used = new Map();
  const corpusUsed = new Map();
  const unresolved = [];
  // The source records of every Rome file: the donor files and the codex
  // files the tables carry the record of.
  const records = new Map();
  for (const file of files) {
    for (const record of fileRecords(file)) {
      if (!records.has(record.id)) records.set(record.id, new Map());
      records.get(record.id).set(record.file, record);
    }
  }
  for (const file of files) {
    const cites = fileCitations(file, registry, records, works);
    for (const [id, names] of cites.ids) {
      if (!used.has(id)) used.set(id, { id, names: new Set(), paths: new Set() });
      const entry = used.get(id);
      for (const name of names) entry.names.add(name);
      entry.paths.add(file.path);
    }
    for (const name of cites.corpus) {
      if (!corpusUsed.has(name)) corpusUsed.set(name, new Set());
      corpusUsed.get(name).add(file.path);
    }
    unresolved.push(...cites.unresolved);
  }

  const credits = [];
  for (const id of [...used.keys()].sort()) {
    const entry = used.get(id);
    const source = registry.sources.find((item) => item.id === id);
    const rows = new Map();
    for (const sidecar of sidecars) {
      if (sidecar.source !== id) continue;
      if (!sidecarUsed(sidecar, entry.names)) continue;
      const file = sidecar.key.split("/").pop() ?? sidecar.key;
      rows.set(`${id}/${file}`, { ...sidecar, key: `${id}/${file}`, from: "sidecar" });
    }
    for (const name of entry.names) {
      const key = `${id}/${name}`;
      if (rows.has(key)) continue;
      const record = records.get(id)?.get(name);
      const unit = source?.units.find((item) => item.name === name);
      if (record) {
        rows.set(key, {
          key,
          url: record.url,
          licence: record.licence,
          sha256: record.sha256,
          fetchedAt: "",
          from: "Rome file",
        });
        continue;
      }
      // The file a ref names may be the same file the record names, with a
      // shorter path. The record already carries it.
      const covered = [...(records.get(id)?.values() ?? [])].some(
        (item) => item.file === name || item.file.endsWith(`/${name}`),
      );
      if (covered) continue;
      rows.set(key, {
        key,
        url: unit?.url ?? source?.url ?? "",
        licence: source?.licence ?? "",
        sha256: "",
        fetchedAt: "",
        from: "source table",
      });
    }
    if (!rows.size) {
      rows.set(id, {
        key: id,
        url: source?.url ?? "",
        licence: source?.licence ?? "",
        sha256: "",
        fetchedAt: "",
        from: "source table",
      });
    }
    credits.push({
      id,
      label: source?.label ?? "",
      licence: source?.licence ?? records.get(id)?.values().next().value?.licence ?? "",
      usedBy: [...entry.paths].sort(),
      rows: [...rows.values()].sort((a, b) => (a.key < b.key ? -1 : a.key > b.key ? 1 : 0)),
    });
  }

  if (corpusUsed.size) {
    const rows = corpusFiles()
      .filter((file) => corpusUsed.has(file.name))
      .map((file) => ({
        key: file.key,
        url: file.url,
        licence: CORPUS_LICENCE,
        sha256: "",
        fetchedAt: "",
        from: "corpus",
      }));
    credits.push({
      id: CORPUS_ID,
      label: CORPUS_LABEL,
      licence: CORPUS_LICENCE,
      usedBy: [...new Set([...corpusUsed.values()].flatMap((set) => [...set]))].sort(),
      rows,
    });
  }

  return { credits, unresolved };
}

/** The credits page. */
export function renderCredits(credits, { unresolved = [] } = {}) {
  const lines = [];
  lines.push("# Rome credits");
  lines.push("");
  lines.push("Every source the shipped Rome content uses, with the url it came from and the");
  lines.push("licence it states. `node tools/codex/credits.mjs` writes this file; do not edit it");
  lines.push("by hand.");
  lines.push("");
  lines.push("The raw fetches stay local: `content/local/` is gitignored. A file that says");
  lines.push("`source table` is a source the local cache does not hold: its url and its licence");
  lines.push("come from `tools/codex/sources.json`.");
  lines.push("");
  lines.push("Our own plan (`docs/plan/rome.md`) is not a source and is not credited here.");
  lines.push("");
  lines.push("A donor file a Rome table carries in its `provenance` section is credited from");
  lines.push("that section: its url, its licence and its sha256.");
  lines.push("");
  for (const entry of credits) {
    const title = entry.label ? `${entry.label} (\`${entry.id}\`)` : `\`${entry.id}\``;
    lines.push(`## ${title}`);
    lines.push("");
    lines.push(`- licence: ${entry.licence}`);
    lines.push(`- used by: ${entry.usedBy.map((path) => `\`${path}\``).join(", ")}`);
    lines.push("");
    for (const row of entry.rows) {
      const bits = [`- \`${row.key}\``, `url <${row.url}>`, `licence ${row.licence}`];
      if (row.sha256) bits.push(`sha256 \`${row.sha256}\``);
      if (row.fetchedAt) bits.push(`fetched ${row.fetchedAt}`);
      bits.push(`from the ${row.from}`);
      lines.push(bits.join(" - "));
    }
    lines.push("");
  }
  if (unresolved.length) {
    lines.push("## Refs that name no source");
    lines.push("");
    lines.push("A ref the codex does not hold. It is reported, never guessed and never credited.");
    lines.push("A ref that cites our own plan is not listed: the plan is not a source.");
    lines.push("");
    for (const item of unresolved) lines.push(`- \`${item.path}\`: ${item.ref}`);
    lines.push("");
  }
  return lines.join("\n");
}

/** Write the page: the whole file lands or none does. */
export function writePage(path, text) {
  mkdirSync(dirname(path), { recursive: true });
  const temp = `${path}.tmp`;
  writeFileSync(temp, text, "utf8");
  try {
    renameSync(temp, path);
  } catch (error) {
    try {
      writeFileSync(path, text, "utf8");
    } catch {
      throw error;
    }
  }
}

/** Write the credits page and report what it holds. */
export function run({
  romeDir = DEFAULT_ROME,
  cacheDir = DEFAULT_CACHE,
  sourcesFile = DEFAULT_SOURCES,
  outFile = DEFAULT_OUT,
  log = (line) => process.stdout.write(`${line}\n`),
} = {}) {
  let registry = null;
  try {
    registry = parseSources(readFileSync(sourcesFile, "utf8"));
  } catch (error) {
    throw new Error(`${sourcesFile}: ${error.message}`);
  }
  const files = romeFileList(romeDir);
  if (!files.length) {
    log(`credits: no Rome file in ${romeDir}`);
    return { ok: false, code: 1, credits: [], unresolved: [], sources: 0, files: 0 };
  }
  const sidecars = readSidecars(cacheDir);
  const { credits, unresolved } = buildCredits({ files, registry, sidecars });
  for (const entry of credits) {
    log(
      `CREDITS source=${entry.id} label="${entry.label || entry.id}" files=${entry.rows.length} ` +
        `licence=${entry.licence} used_by=${entry.usedBy.length}`,
    );
  }
  for (const item of unresolved) log(`CREDITS unresolved ${item.path}: ${item.ref}`);
  const text = renderCredits(credits, { unresolved });
  writePage(outFile, text);
  const rows = credits.reduce((n, entry) => n + entry.rows.length, 0);
  log(`CREDITS wrote ${credits.length} source(s) and ${rows} file(s) to ${outFile}`);
  return {
    ok: true,
    code: 0,
    credits,
    unresolved,
    sources: credits.length,
    files: rows,
    out: outFile,
  };
}

function opt(args, name) {
  const at = args.findIndex((arg) => arg === name || arg.startsWith(`${name}=`));
  if (at < 0) return "";
  if (args[at].includes("=")) return args[at].slice(name.length + 1);
  return args[at + 1] ?? "";
}

if (import.meta.main) {
  const args = process.argv.slice(2);
  if (args.includes("--help") || args.includes("-h")) {
    process.stdout.write(HELP);
  } else {
    try {
      const result = run({
        romeDir: opt(args, "--rome") || DEFAULT_ROME,
        cacheDir: opt(args, "--cache") || DEFAULT_CACHE,
        sourcesFile: opt(args, "--sources") || DEFAULT_SOURCES,
        outFile: opt(args, "--out") || DEFAULT_OUT,
      });
      if (!result.ok) process.exitCode = result.code;
    } catch (error) {
      process.stderr.write(`credits: error: ${error.message}\n`);
      process.exit(2);
    }
  }
}
