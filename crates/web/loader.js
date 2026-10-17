// How the page gets the engine module (QW3, QW12). Plain ES module, no dependencies, so node tests drive it with fakes.
//
// QW3: the wasm is fetched here, not inside wasm-bindgen's init, so the page owns the answer to "is the server typing it as
// application/wasm?" (a wrong type throws a LoadError the page shows as the fatal reason, never a blank page) and the three
// numbers of the load are measured, not guessed. WebAssembly.compileStreaming compiles while the bytes arrive; a second
// reader on a clone of the response times the last byte, so
//   downloadMs = fetch start to the last byte,
//   compileMs  = the compile time left AFTER the last byte (0 when compile finished under the download).
//
// bytesFetched counts the decoded module bytes (what the host sends on the wire is smaller when it compresses; tools/web-headers.mjs
// records that).
//
// QW12: build.json (always revalidated) names the SHA-256 of engine_bg.wasm. The bytes are kept in the Cache API under
// engine_bg.wasm?sha256=<that hash>, so a repeat visit reads them locally (bundle=cache, bytesFetched=0) and a new build
// cannot be served stale bytes. A copy is stored only when its hash matches. QuotaExceededError (or any cache failure) is
// reported in stats.cache and the page loads anyway from the network bytes. navigator.storage.persist() is asked once
// the copy is stored so the browser does not evict it under pressure.

export const CACHE_NAME = "engine2040-wasm";
export const WASM_FILE = "engine_bg.wasm";

/** The wasm could not be fetched or is served wrongly; the message is the reason the page shows. */
export class LoadError extends Error {}

const hex = (buf) => [...new Uint8Array(buf)].map((b) => b.toString(16).padStart(2, "0")).join("");

/** The browser's own fetch, caches, storage, WebAssembly, clock and SHA-256; tests pass fakes instead. */
export const browserEnv = () => ({
  fetch: (...a) => fetch(...a),
  caches: globalThis.caches ?? null,
  storage: globalThis.navigator?.storage ?? null,
  wasm: WebAssembly,
  now: () => performance.now(),
  sha256: async (bytes) => hex(await crypto.subtle.digest("SHA-256", bytes)),
});

// The response is usable when it is a 200 typed application/wasm; otherwise the LoadError says what the server sent.
function checkResponse(res) {
  if (!res.ok) throw new LoadError(`${WASM_FILE} could not be fetched: HTTP ${res.status}`);
  const type = res.headers.get("content-type") ?? "";
  if (!/^application\/wasm\s*(;|$)/i.test(type)) {
    throw new LoadError(`${WASM_FILE} is served as "${type || "no content-type"}", not application/wasm, so the browser will not compile it while it downloads; the server must type .wasm files as application/wasm`);
  }
}

// The SHA-256 build.json lists for the wasm, or null (no build.json, or not the shape): then nothing is cached.
async function wantedHash(env, base) {
  try {
    const res = await env.fetch(new URL("build.json", base), { cache: "no-cache" });
    const hash = (await res.json()).files?.[WASM_FILE]?.sha256;
    return /^[0-9a-f]{64}$/.test(hash) ? hash : null;
  } catch {
    return null;
  }
}

// Store the verified bytes. Returns what stats.cache and stats.persisted should say.
async function remember(env, cache, key, bytes, want) {
  if ((await env.sha256(bytes)) !== want) return { cache: "hash-mismatch", persisted: null };
  try {
    await cache.put(key, new Response(bytes, { headers: { "content-type": "application/wasm" } }));
  } catch (e) {
    return { cache: e?.name === "QuotaExceededError" ? "quota" : `error:${e?.name ?? "unknown"}`, persisted: null };
  }
  let persisted = null;
  try { persisted = (await env.storage?.persist?.()) ?? null; } catch { /* a refusal is not a failure */ }
  return { cache: "stored", persisted };
}

/**
 * Fetch and compile the engine module. Resolves { module, stats, settled }: `module` goes to init({ module_or_path }),
 * `stats` is { bundle: "network"|"cache", bytesFetched, downloadMs, compileMs, cache, persisted } and `settled` resolves
 * (never rejects) once the cache copy is written and stats.cache is final. Rejects with a LoadError for a bad response.
 */
export async function loadEngine(base, env = browserEnv()) {
  const stats = { bundle: "network", bytesFetched: 0, downloadMs: 0, compileMs: 0, cache: "off", persisted: null };
  const url = new URL(WASM_FILE, base);
  const want = env.caches ? await wantedHash(env, base) : null;
  let cache = null, key = null, res = null;
  if (want) {
    try {
      cache = await env.caches.open(CACHE_NAME);
      key = `${url.href}?sha256=${want}`;
      res = await cache.match(key);
    } catch {
      cache = null;
    }
  }
  stats.cache = cache ? "miss" : want ? "unavailable" : "off";
  if (res) {
    stats.bundle = "cache";
    stats.cache = "hit";
  }
  const t0 = env.now();
  res ??= await env.fetch(url);
  checkResponse(res);
  const bodyBytes = res.clone().arrayBuffer().then((b) => { stats.downloadMs = env.now() - t0; return b; });
  bodyBytes.catch(() => {}); // a failed compile throws below; this copy of the failure is not a second report
  const module = await env.wasm.compileStreaming(res);
  const compiled = env.now();
  const bytes = await bodyBytes;
  stats.compileMs = Math.max(0, Math.round(compiled - t0 - stats.downloadMs));
  stats.downloadMs = Math.round(stats.downloadMs);
  if (stats.bundle === "network") stats.bytesFetched = bytes.byteLength;
  const settled = stats.bundle === "network" && cache
    ? remember(env, cache, key, bytes, want).then((r) => Object.assign(stats, r), () => { stats.cache = "error"; return stats; }).then(() => stats)
    : Promise.resolve(stats);
  return { module, stats, settled };
}

/** Resolves with env.now() when `body` (the page's <body>) first carries data-state running or fatal: the first frame, or the reason there is none. */
export function watchFirstFrame(body, now = () => performance.now()) {
  return new Promise((resolve) => {
    const seen = () => ["running", "fatal"].includes(body.dataset.state);
    if (seen()) return resolve(now());
    const mo = new MutationObserver(() => {
      if (seen()) { mo.disconnect(); resolve(now()); }
    });
    mo.observe(body, { attributes: true, attributeFilter: ["data-state"] });
  });
}

/** The one line the page prints in #load. Fields not known yet are left out. */
export function loadLine(stats, initMs, firstFrameMs) {
  const f = [`bundle=${stats.bundle}`, `fetched=${stats.bytesFetched}`, `download=${stats.downloadMs}ms`, `compile=${stats.compileMs}ms`];
  if (initMs != null) f.push(`init=${Math.round(initMs)}ms`);
  if (firstFrameMs != null) f.push(`first-frame=${Math.round(firstFrameMs)}ms`);
  f.push(`cache=${stats.cache}`);
  if (stats.persisted !== null) f.push(`persist=${stats.persisted ? "granted" : "denied"}`);
  return `load: ${f.join(" ")}`;
}

/**
 * QW14: sample the module's linear memory (64 KiB pages) now and then every `everyMs` into `log` as { ms, pages }.
 * `memory` is the WebAssembly.Memory init() returns; `memory.buffer` is read fresh each time because memory.grow
 * detaches the old buffer. Returns { sample, stop }; `sample()` takes one reading on demand (the first frame, a forced grow).
 */
export function watchMemory(memory, log, { everyMs = 60000, now = () => performance.now(), every = setInterval, stopEvery = clearInterval } = {}) {
  const sample = () => {
    const pages = memory.buffer.byteLength / 65536;
    log.push({ ms: Math.round(now()), pages });
    return pages;
  };
  const timer = every(sample, everyMs);
  return { sample, stop: () => stopEvery(timer) };
}

/** The line the page prints in #memory: the latest pages, the peak and how many readings there are. */
export function memoryLine(log) {
  if (!log.length) return "memory: no reading yet";
  const peak = Math.max(...log.map((r) => r.pages));
  return `memory: pages=${log[log.length - 1].pages} peak=${peak} bytes=${peak * 65536} samples=${log.length}`;
}
