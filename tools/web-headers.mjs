// What the server really sends for the page's files (QW13): content-type, content-encoding and content-length of each file,
// asked the way a browser asks (Accept-Encoding: br, gzip).
//   node tools/web-headers.mjs --url https://maximkone.github.io/engine2040-public/ [--out out/pages-headers.json] [--retries 6] [--wait 10]
// Prints one line per file and one summary line
//   web-headers: wasm type=application/wasm encoding=gzip bytes=1893357 brotli=not-served
// and writes the same facts as JSON. Exit 0 = every file answered 200 and engine_bg.wasm is typed application/wasm (the page
// refuses a module typed any other way); 1 = it is not (after the retries: a fresh deploy takes a moment to reach the edge);
// 2 = could not run. `brotli=served` appears only when some file really came back content-encoding: br; the brotli lines of
// tools/web-budget.json are a promise about what a visitor downloads only when it does.
import { mkdirSync, writeFileSync } from "node:fs";
import { dirname, resolve } from "node:path";

export const FILES = ["index.html", "loader.js", "coi-serviceworker.js", "engine.js", "engine_bg.wasm", "build.json"];
export const WASM_FILE = "engine_bg.wasm";
export const MANIFEST = "build.json";

/** The wasm is the fixed name in an old dist, or its content-hashed name in a pinned one. */
export const isWasm = (file) => file === WASM_FILE || new RegExp(`^engine_bg\\.[0-9a-f]{12}\\.wasm$`).test(file);

/** The files a pinned build.json names (the fixed files keep their names and are in the pin too), or the fixed list for an old dist. */
export function filesFrom(manifest) {
  const pin = manifest?.pin;
  if (!pin || typeof pin !== "object" || Array.isArray(pin) || !Object.keys(pin).length) return FILES;
  const names = new Set([MANIFEST]);
  for (const entry of Object.values(pin)) if (typeof entry?.name === "string" && entry.name) names.add(entry.name);
  return [...names];
}

/** GET build.json and read its pin. Throws when the manifest itself is missing or not JSON. */
export async function fetchManifest(base, fetchFn = fetch) {
  const res = await fetchFn(new URL(MANIFEST, base), { cache: "no-cache" });
  if (!res.ok) throw new Error(`${MANIFEST} could not be fetched: HTTP ${res.status}`);
  const manifest = await res.json();
  if (!manifest?.pin) throw new Error(`${MANIFEST} names no build pin`);
  return manifest;
}

/** Ask for `file` under `base` with a HEAD request that offers br and gzip; returns the facts of the answer. */
export async function probeFile(base, file, fetchFn = fetch) {
  const res = await fetchFn(new URL(file, base), { method: "HEAD", headers: { "accept-encoding": "br, gzip" } });
  const h = (name) => res.headers.get(name);
  return { file, status: res.status, contentType: h("content-type"), contentEncoding: h("content-encoding") ?? "identity", contentLength: h("content-length") === null ? null : Number(h("content-length")), cacheControl: h("cache-control") };
}

/** What the probed rows prove: every file 200, the wasm typed application/wasm, and whether brotli is served at all. */
export function verdict(rows) {
  const problems = [];
  for (const r of rows) if (r.status !== 200) problems.push(`${r.file}: HTTP ${r.status}`);
  const wasm = rows.find((r) => isWasm(r.file));
  if (!wasm) problems.push(`${WASM_FILE} was not probed`);
  else if (!/^application\/wasm\s*(;|$)/i.test(wasm.contentType ?? "")) problems.push(`${wasm.file} is typed "${wasm.contentType}", not application/wasm`);
  return { ok: problems.length === 0, problems, brotliServed: rows.some((r) => r.contentEncoding === "br") };
}

/** The summary line for the job log. */
export function summaryLine(rows) {
  const wasm = rows.find((r) => isWasm(r.file));
  const v = verdict(rows);
  return `web-headers: wasm type=${wasm?.contentType ?? "?"} encoding=${wasm?.contentEncoding ?? "?"} bytes=${wasm?.contentLength ?? "?"} brotli=${v.brotliServed ? "served" : "not-served"}`;
}

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

/** Probe every file, retrying while the verdict is not ok (a deploy reaches the edge a little late). */
export async function probeAll(base, { files = FILES, fetchFn = fetch, retries = 1, waitMs = 0 } = {}) {
  let rows = [];
  for (let i = 0; i < retries; i++) {
    if (i) await sleep(waitMs);
    rows = [];
    for (const f of files) rows.push(await probeFile(base, f, fetchFn));
    if (verdict(rows).ok) break;
  }
  return rows;
}

/**
 * Probe a whole site: read build.json's pin first, then HEAD each file it names. A manifest that is not there yet (a
 * deploy still reaching the edge) is retried with the rest; after the last try the fixed names are probed so the
 * failure still names the files it asked for.
 */
export async function probeSite(base, { fetchFn = fetch, retries = 1, waitMs = 0 } = {}) {
  let rows = [];
  let manifestError = null;
  for (let i = 0; i < retries; i++) {
    if (i) await sleep(waitMs);
    manifestError = null;
    let files = FILES;
    try {
      files = filesFrom(await fetchManifest(base, fetchFn));
    } catch (e) {
      manifestError = String(e?.message ?? e);
    }
    rows = [];
    for (const f of files) {
      try {
        rows.push(await probeFile(base, f, fetchFn));
      } catch (e) {
        rows.push({ file: f, status: 0, error: String(e?.message ?? e) });
      }
    }
    if (verdict(rows).ok) break;
    // The last try keeps why the pin could not be read, so the failure names it.
    if (manifestError && i === retries - 1) rows.unshift({ file: MANIFEST, status: 0, error: manifestError });
  }
  return rows;
}

/** Command line to options. */
export function parseArgs(argv) {
  const o = { url: null, out: null, retries: 1, wait: 10, error: null };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (a === "--url") o.url = argv[++i] ?? null;
    else if (a === "--out") o.out = argv[++i] ?? null;
    else if (a === "--retries") o.retries = Number(argv[++i]);
    else if (a === "--wait") o.wait = Number(argv[++i]);
    else o.error = `unknown argument ${a}`;
  }
  if (!o.url || !/^https?:\/\/[^/\s]+\//.test(o.url)) o.error ??= "--url needs the site address ending in / (https://host/path/)";
  if (!(o.retries >= 1) || !(o.wait >= 0)) o.error ??= "--retries needs 1 or more, --wait seconds";
  return o;
}

if (import.meta.main) {
  const a = parseArgs(process.argv.slice(2));
  if (a.error) {
    console.error(`web-headers: ${a.error}`);
    process.exit(2);
  }
  let rows;
  try {
    rows = await probeSite(a.url, { retries: a.retries, waitMs: a.wait * 1000 });
  } catch (e) {
    console.error(`web-headers: ${e.message ?? e}`);
    process.exit(2);
  }
  for (const r of rows) console.log(`${a.url}${r.file} -> ${r.status} type=${r.contentType} encoding=${r.contentEncoding} bytes=${r.contentLength} cache-control=${r.cacheControl}`);
  const v = verdict(rows);
  console.log(summaryLine(rows));
  if (a.out) {
    mkdirSync(dirname(resolve(a.out)), { recursive: true });
    writeFileSync(a.out, `${JSON.stringify({ url: a.url, rows, ...v }, null, 2)}\n`);
  }
  if (process.env.GITHUB_STEP_SUMMARY) {
    const md = `### ${a.url}\n\`\`\`\n${rows.map((r) => `${r.file}  ${r.status}  ${r.contentType}  ${r.contentEncoding}  ${r.contentLength}`).join("\n")}\n${summaryLine(rows)}\n\`\`\`\n`;
    (await import("node:fs")).appendFileSync(process.env.GITHUB_STEP_SUMMARY, md);
  }
  for (const p of v.problems) console.error(`web-headers: ${p}`);
  process.exit(v.ok ? 0 : 1);
}
