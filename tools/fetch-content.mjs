#!/usr/bin/env node
// fetch-content.mjs - download listed content files with md5 + license sidecars.
// Use: node tools/fetch-content.mjs --help
//      node tools/fetch-content.mjs --list <file> [--out <dir>] [--dry-run]
// List file: one row per asset: `<source> <id> <url> [md5] [license] [author]`.
// Blank lines and `#` comments are skipped. source is polyhaven, ambientcg or
// url. A `-` url resolves to the source API root (polyhaven
// `api.polyhaven.com/files/{id}`, ambientCG `ambientCG.com/api/v2/full_json`);
// tests pass explicit local http urls so no real host is touched.
// Each asset lands in `<out>/<source>/<id>/` with a `<id>.license.json`
// sidecar {url, author, license, retrieved}. One request per second
// (--rate-ms, default 1000). Files whose md5 matches are skipped. A wrong
// md5 deletes the file and exits 1. --dry-run prints total bytes and every
// license and writes nothing. Sends a User-Agent, never scrapes pages.
import { createHash } from "node:crypto";
import { existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { basename, dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
const REPO = resolve(HERE, "..");
export const DEFAULT_OUT = join(REPO, "content", "_incoming");
export const RATE_MS = 1000;
export const UA = "engine2040-fetch-content/1.0 (+https://github.com/engine2040)";

export const HELP = `Usage: node tools/fetch-content.mjs --list <file> [--out <dir>] [--dry-run] [--rate-ms N]

Reads a list file of \`<source> <id> <url> [md5] [license] [author]\` lines and
downloads into \`<out>/<source>/<id>/\`, each with a \`<id>.license.json\`
sidecar (url, author, license, retrieved date). Sources: polyhaven
(api.polyhaven.com/files/{id}, User-Agent set, never scrape pages), ambientcg
(ambientCG.com/api/v2/full_json), plain urls with md5. One request per second
(--rate-ms, default 1000), skip files whose md5 matches. A wrong md5 deletes
the file and exits 1. --dry-run prints total bytes and every license.
`;

export function md5Of(bytes) {
  return createHash("md5").update(bytes).digest("hex");
}

export function resolveUrl(source, id, url) {
  if (url && url !== "-") return url;
  if (source === "polyhaven") return `https://api.polyhaven.com/files/${id}`;
  if (source === "ambientcg") return `https://ambientcg.com/api/v2/full_json?id=${id}`;
  throw new Error(`url source needs an explicit url, got id=${id}`);
}

// Parse the list file. Returns [{source, id, url, md5, license, author, line}].
export function parseList(text) {
  const rows = [];
  const lines = String(text ?? "").split(/\r?\n/);
  for (let i = 0; i < lines.length; i++) {
    const raw = lines[i].trim();
    if (!raw || raw.startsWith("#")) continue;
    const parts = raw.split(/\s+/);
    if (parts.length < 3) throw new Error(`list line ${i + 1}: want "<source> <id> <url> [md5] [license] [author]"`);
    const [source, id, url, md5 = "", license = "", author = ""] = parts;
    if (!["polyhaven", "ambientcg", "url"].includes(source)) {
      throw new Error(`list line ${i + 1}: bad source ${JSON.stringify(source)}`);
    }
    if (!id || !url) throw new Error(`list line ${i + 1}: id and url are required`);
    if (md5 && !/^[0-9a-f]{32}$/i.test(md5)) throw new Error(`list line ${i + 1}: bad md5 ${JSON.stringify(md5)}`);
    rows.push({ source, id, url, md5: md5.toLowerCase(), license, author, line: i + 1 });
  }
  return rows;
}

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

async function fetchBytes(url) {
  const res = await fetch(url, { headers: { "User-Agent": UA } });
  if (!res.ok) throw new Error(`GET ${url} -> HTTP ${res.status}`);
  return Buffer.from(await res.arrayBuffer());
}

export function sidecarFor(entry, url) {
  return {
    url,
    author: entry.author || "unknown",
    license: entry.license || "unknown",
    retrieved: new Date().toISOString().slice(0, 10),
  };
}

export async function run({ listFile, outDir = DEFAULT_OUT, dryRun = false, rateMs = RATE_MS }) {
  let text = null;
  try {
    text = readFileSync(listFile, "utf8");
  } catch {
    process.stderr.write(`fetch-content: error: list file absent: ${listFile}\n`);
    process.exit(2);
  }
  const rows = parseList(text);
  let totalBytes = 0;
  let downloaded = 0;
  let skipped = 0;
  let bad = 0;
  const licenses = [];
  let fetched = false;
  for (let i = 0; i < rows.length; i++) {
    const entry = rows[i];
    const url = resolveUrl(entry.source, entry.id, entry.url);
    const dir = join(outDir, entry.source, entry.id);
    const name = basename(new URL(url).pathname) || entry.id;
    const dest = join(dir, name);
    const sidecar = join(dir, `${entry.id}.license.json`);
    licenses.push(entry.license || "unknown");
    if (!dryRun && entry.md5 && existsSync(dest)) {
      const cur = md5Of(readFileSync(dest));
      if (cur === entry.md5) {
        skipped++;
        process.stdout.write(`SKIP ${entry.source} ${entry.id} md5-match ${dest}\n`);
        continue;
      }
    }
    if (fetched && rateMs > 0) await sleep(rateMs);
    const bytes = await fetchBytes(url);
    fetched = true;
    totalBytes += bytes.length;
    if (dryRun) continue;
    mkdirSync(dir, { recursive: true });
    if (entry.md5) {
      const got = md5Of(bytes);
      if (got !== entry.md5) {
        if (existsSync(dest)) rmSync(dest, { force: true });
        bad++;
        process.stderr.write(`FAIL ${entry.source} ${entry.id} md5 mismatch want=${entry.md5} got=${got}\n`);
        continue;
      }
    }
    writeFileSync(dest, bytes);
    writeFileSync(sidecar, JSON.stringify(sidecarFor(entry, url), null, 2) + "\n");
    downloaded++;
    process.stdout.write(`OK ${entry.source} ${entry.id} ${bytes.length}B ${dest}\n`);
  }
  if (dryRun) {
    process.stdout.write(`DRY total_bytes=${totalBytes} files=${rows.length}\n`);
    for (const lic of licenses) process.stdout.write(`LICENSE ${lic || "unknown"}\n`);
    return { ok: true, totalBytes, downloaded: 0, skipped: 0, bad: 0 };
  }
  if (bad > 0) {
    process.stderr.write(`fetch-content: error: ${bad} file(s) failed md5 and were deleted\n`);
    return { ok: false, totalBytes, downloaded, skipped, bad };
  }
  process.stdout.write(`RESULT fetch-content ok downloaded=${downloaded} skipped=${skipped} bytes=${totalBytes}\n`);
  return { ok: true, totalBytes, downloaded, skipped, bad: 0 };
}

function opt(args, name) {
  const i = args.findIndex((a) => a === name || a.startsWith(`${name}=`));
  if (i < 0) return "";
  if (args[i].includes("=")) return args[i].slice(name.length + 1);
  return args[i + 1] ?? "";
}

const isMain = import.meta.main;
if (isMain) {
  const args = process.argv.slice(2);
  if (args.includes("--help") || args.includes("-h")) {
    process.stdout.write(HELP);
  } else if (args.includes("--list") || args.some((a) => a.startsWith("--list="))) {
    const listFile = opt(args, "--list");
    if (!listFile) {
      process.stderr.write("fetch-content: error: --list <file> is required\n");
      process.exit(2);
    }
    const outDir = opt(args, "--out") || DEFAULT_OUT;
    const rateMs = Number(opt(args, "--rate-ms") || RATE_MS);
    const r = await run({ listFile, outDir, dryRun: args.includes("--dry-run"), rateMs });
    if (!r.ok) process.exitCode = 1;
  } else {
    process.stderr.write("fetch-content: error: need --list <file> or --help\n");
    process.exit(2);
  }
}
