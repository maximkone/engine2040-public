#!/usr/bin/env node
// fetch.mjs - the Rome codex fetch tool (CDX1).
//
// Job: fetch the codex sources listed in `tools/codex/sources.json` (the
// twelve of docs/plan/rome-codex.md section 2) into
// `content/local/codex/<source>/`. Every file lands with a `<file>.src.json`
// sidecar: url, fetchedAt, sha256 and the licence the site states. The tool
// fetches the url the table names, one unit per item, at the source's own
// rate limit and under the source's size cap. It never scrapes a page.
//
// Raw fetches stay local: `content/local/` is gitignored, so nothing here is
// committed. The committed side is the small game data CDX3-CDX5 derive from
// it, each entry keeping its codex reference.
//
// Usage:
//   node tools/codex/fetch.mjs [<source>] [--sources FILE] [--out DIR]
//                               [--dry-run] [--yes] [--help]
//   node tools/codex/fetch.mjs --dry-run     # the plan over every source
//   node tools/codex/fetch.mjs gutenberg      # one source by id
//
// Rules:
//   - `--dry-run` prints one line per source and writes nothing.
//   - the run prints the planned bytes before the first request.
//   - a plan above 500 MB needs `--yes`; without it the tool refuses (exit 2).
//   - a source that passes its size cap fails: the partial file is deleted,
//     the source is reported, and the run exits 1.
//   - every request carries a User-Agent; one request per rate window.
//
// Exit: 0 ok, 1 a fetch failed, 2 bad CLI, a missing table, or a plan above
// the ask threshold without --yes.
import { createHash } from "node:crypto";
import { createWriteStream, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { once } from "node:events";

const HERE = dirname(fileURLToPath(import.meta.url));
const REPO = resolve(HERE, "..", "..");
export const DEFAULT_SOURCES = join(HERE, "sources.json");
export const DEFAULT_OUT = join(REPO, "content", "local", "codex");
export const ASK_ABOVE_BYTES = 500 * 1024 * 1024;
export const UA = "engine2040-codex-fetch/1.0 (+https://github.com/maximkone/engine2040)";

export const HELP = `Usage: node tools/codex/fetch.mjs [<source>] [--sources FILE] [--out DIR] [--dry-run] [--yes] [--help]

Fetches the codex sources of tools/codex/sources.json into <out>/<source>/,
each file with a <file>.src.json sidecar (url, fetchedAt, sha256, licence).
<source> picks one source id; without it every source runs. --dry-run prints
the plan and writes nothing. --yes allows a plan above 500 MB. Exit 0 ok,
1 a fetch failed, 2 bad CLI or a plan above the ask threshold.
`;

const SLUG = /^[a-z0-9][a-z0-9-]*$/;

// Read the source table. Every source needs an id, an http url, a rate limit,
// a size cap, the size of one unit and the licence it states.
export function parseSources(text) {
  let raw;
  try {
    raw = typeof text === "string" ? JSON.parse(text) : text;
  } catch (err) {
    throw new Error(`sources table is not JSON: ${err.message}`);
  }
  if (!raw || typeof raw !== "object" || !Array.isArray(raw.sources)) {
    throw new Error("sources table needs a sources array");
  }
  const askAboveBytes = raw.askAboveBytes ?? ASK_ABOVE_BYTES;
  if (!Number.isInteger(askAboveBytes) || askAboveBytes <= 0) {
    throw new Error("askAboveBytes must be a positive whole number");
  }
  const seen = new Set();
  const sources = raw.sources.map((s, i) => {
    const where = `source ${i + 1}`;
    if (!s || typeof s !== "object") throw new Error(`${where}: not an object`);
    const id = String(s.id ?? "");
    if (!SLUG.test(id)) throw new Error(`${where}: bad id ${JSON.stringify(s.id)}`);
    if (seen.has(id)) throw new Error(`${where}: duplicate id ${id}`);
    seen.add(id);
    const url = String(s.url ?? "");
    if (!/^https?:\/\//.test(url)) throw new Error(`${where} (${id}): url must be http or https`);
    for (const key of ["rateLimitMs", "sizeCapBytes", "unitBytes"]) {
      const value = s[key];
      if (!Number.isInteger(value) || value < 0) throw new Error(`${where} (${id}): ${key} must be a whole number`);
    }
    if (s.sizeCapBytes <= 0) throw new Error(`${where} (${id}): sizeCapBytes must be above zero`);
    if (s.unitBytes <= 0) throw new Error(`${where} (${id}): unitBytes must be above zero`);
    const licence = String(s.licence ?? "");
    if (!licence) throw new Error(`${where} (${id}): the licence must be stated`);
    const items = Array.isArray(s.items) ? s.items : [];
    if (items.length === 0) throw new Error(`${where} (${id}): needs at least one item`);
    const units = items.map((it, j) => {
      const itemId = String(it?.id ?? "");
      const name = String(it?.name ?? "");
      if (!itemId) throw new Error(`${where} (${id}): item ${j + 1} needs an id`);
      if (!name || name.includes("/") || name.includes("\\") || name.includes("..")) {
        throw new Error(`${where} (${id}): item ${j + 1} needs a plain file name`);
      }
      const unitUrl = url.replaceAll("{id}", encodeURIComponent(itemId));
      if (!/^https?:\/\//.test(unitUrl)) throw new Error(`${where} (${id}): the url does not resolve to http`);
      return { id: itemId, name, url: unitUrl };
    });
    return {
      id,
      label: String(s.label ?? id),
      format: String(s.format ?? ""),
      licence,
      note: String(s.note ?? ""),
      rateLimitMs: s.rateLimitMs,
      sizeCapBytes: s.sizeCapBytes,
      unitBytes: s.unitBytes,
      url,
      units,
    };
  });
  return { askAboveBytes, sources };
}

// The plan of one source: its units and the bytes the table states for them.
export function planSource(source) {
  return {
    source,
    units: source.units,
    plannedBytes: source.unitBytes * source.units.length,
  };
}

const sleep = (ms) => new Promise((done) => setTimeout(done, ms));

// Stream one unit to disk under a byte cap. The file is deleted when the cap
// or the network fails, so a half file never sits in the library.
async function download(unit, dest, capBytes) {
  const res = await fetch(unit.url, { headers: { "User-Agent": UA } });
  if (!res.ok) throw new Error(`GET ${unit.url} -> HTTP ${res.status}`);
  if (!res.body) throw new Error(`GET ${unit.url} -> no body`);
  const hash = createHash("sha256");
  const out = createWriteStream(dest);
  // The handle must be open before the first byte, and it must be closed
  // before the file is deleted: on Windows a file with an open handle
  // cannot be removed.
  const closed = new Promise((done) => {
    out.once("close", done);
    out.once("error", done);
  });
  await new Promise((ready, fail) => {
    out.once("open", ready);
    out.once("error", fail);
  });
  let bytes = 0;
  const reader = res.body.getReader();
  try {
    for (;;) {
      const { done, value } = await reader.read();
      if (done) break;
      bytes += value.length;
      if (bytes > capBytes) throw new Error(`size cap ${capBytes} bytes exceeded`);
      hash.update(value);
      if (!out.write(value)) await once(out, "drain");
    }
    await new Promise((done, fail) => out.end((err) => (err ? fail(err) : done())));
  } catch (err) {
    try {
      await reader.cancel();
    } catch {}
    out.destroy();
    await closed;
    rmSync(dest, { force: true });
    throw err;
  }
  return { bytes, sha256: hash.digest("hex") };
}

export async function run({
  sources,
  askAboveBytes = ASK_ABOVE_BYTES,
  outDir = DEFAULT_OUT,
  dryRun = false,
  yes = false,
  only = "",
  log = (line) => process.stdout.write(`${line}\n`),
  errLog = (line) => process.stderr.write(`${line}\n`),
}) {
  const picked = only ? sources.filter((s) => s.id === only) : sources;
  if (only && picked.length === 0) throw new Error(`no source with id ${only}`);
  const plan = picked.map(planSource);
  const plannedBytes = plan.reduce((n, p) => n + p.plannedBytes, 0);
  if (dryRun) {
    for (const p of plan) {
      log(
        `PLAN ${p.source.id} units=${p.units.length} bytes=${p.plannedBytes} ` +
          `cap=${p.source.sizeCapBytes} rate_ms=${p.source.rateLimitMs} ` +
          `licence=${p.source.licence} url=${p.source.url}`,
      );
    }
    return { ok: true, dryRun: true, plannedBytes, fetched: 0, failed: 0, bytes: 0 };
  }
  if (plannedBytes > askAboveBytes && !yes) {
    errLog(`codex: the plan is ${plannedBytes} bytes, above the ${askAboveBytes} byte ask: rerun with --yes`);
    return { ok: false, needsYes: true, plannedBytes, fetched: 0, failed: 0, bytes: 0 };
  }
  log(`BYTES planned_total=${plannedBytes} sources=${plan.length}`);
  let fetched = 0;
  let failed = 0;
  let bytes = 0;
  for (const p of plan) {
    const s = p.source;
    const dir = join(outDir, s.id);
    let sourceBytes = 0;
    let lastDoneAt = 0;
    for (const unit of s.units) {
      // The rate window starts when the last request of this source ended.
      const wait = lastDoneAt === 0 ? 0 : Math.max(0, lastDoneAt + s.rateLimitMs - Date.now());
      if (wait > 0) await sleep(wait);
      mkdirSync(dir, { recursive: true });
      const dest = join(dir, unit.name);
      try {
        const got = await download(unit, dest, s.sizeCapBytes - sourceBytes);
        lastDoneAt = Date.now();
        sourceBytes += got.bytes;
        bytes += got.bytes;
        fetched += 1;
        const sidecar = {
          source: s.id,
          url: unit.url,
          fetchedAt: new Date().toISOString(),
          sha256: got.sha256,
          licence: s.licence,
        };
        writeFileSync(`${dest}.src.json`, `${JSON.stringify(sidecar, null, 2)}\n`);
        log(`OK ${s.id} ${unit.name} ${got.bytes}B ${dest}`);
      } catch (err) {
        failed += 1;
        rmSync(dest, { force: true });
        const reason = String(err?.message ?? err);
        log(`FAIL ${s.id} ${unit.name} ${reason}`);
        if (reason.includes("size cap")) break;
      }
    }
  }
  log(`RESULT codex fetched=${fetched} failed=${failed} bytes=${bytes}`);
  return { ok: failed === 0, dryRun: false, plannedBytes, fetched, failed, bytes };
}

function opt(args, name) {
  const i = args.findIndex((a) => a === name || a.startsWith(`${name}=`));
  if (i < 0) return "";
  if (args[i].includes("=")) return args[i].slice(name.length + 1);
  return args[i + 1] ?? "";
}

// The source id is the one argument that is not a flag and not a flag value.
export function positional(args) {
  let found = "";
  for (let i = 0; i < args.length; i++) {
    const a = args[i];
    if (a === "--sources" || a === "--out") {
      i += 1;
      continue;
    }
    if (a.startsWith("--sources=") || a.startsWith("--out=")) continue;
    if (!a.startsWith("-") && !found) found = a;
  }
  return found;
}

const isMain = import.meta.main;
if (isMain) {
  const args = process.argv.slice(2);
  if (args.includes("--help") || args.includes("-h")) {
    process.stdout.write(HELP);
  } else {
    const sourcesFile = opt(args, "--sources") || DEFAULT_SOURCES;
    const outDir = opt(args, "--out") || DEFAULT_OUT;
    let table = null;
    try {
      table = parseSources(readFileSync(sourcesFile, "utf8"));
    } catch (err) {
      process.stderr.write(`codex: error: ${sourcesFile}: ${err.message}\n`);
      process.exit(2);
    }
    try {
      const r = await run({
        sources: table.sources,
        askAboveBytes: table.askAboveBytes,
        outDir,
        dryRun: args.includes("--dry-run"),
        yes: args.includes("--yes"),
        only: positional(args),
      });
      if (!r.ok) process.exitCode = r.needsYes ? 2 : 1;
    } catch (err) {
      process.stderr.write(`codex: error: ${err.message}\n`);
      process.exit(2);
    }
  }
}
