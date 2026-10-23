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

// WEB6: gameplay content is fetched at runtime, not read from a file system (the browser has
// none). The page owns the network: the engine asks for an asset by URL through the page's own
// fetcher (window.engine2040FetchAsset, installed by index.html) and the bytes come back through
// the asset seam in engine-assets. A failure here is a typed AssetError that names the asset, so
// the page can say what is missing instead of drawing an empty arena.

/** A gameplay asset could not be fetched. The message names the asset. */
export class AssetError extends Error {
  constructor(message, status = null) {
    super(message);
    this.name = "AssetError";
    /** The HTTP status the server answered with, or null when it never answered. */
    this.status = status;
  }
}

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

// The path part of an asset URL, so an error names the asset and not a whole URL.
const assetName = (url) => {
  try {
    return new URL(url, "http://localhost/").pathname.replace(/^\/+/, "");
  } catch {
    return String(url);
  }
};

// The message of a thrown value, bounded to one line.
const reason = (err) => String(err?.message ?? err ?? "unknown").slice(0, 120);

// QW10: the build ships the two cooked bundles gzip-compressed. dist/web must stay under
// its 8 MiB limit, and a static host does not compress a binary type it does not know, so
// the gzip lives on disk and the page undoes it while the body streams. The magic decides:
// a dist that still holds a raw bundle loads exactly as before, so the two forms never
// disagree about what a bundle is.
const GZ_BYTES = [0x1f, 0x8b];

// Read the body until its first two bytes are known, then hand the whole body on from byte
// zero. `gzip` says which form arrived; nothing is buffered beyond two bytes, so the
// tier-0 prefix still reaches the page while the rest of the bundle is in flight.
async function sniffGzip(body) {
  const reader = body.getReader();
  let head = new Uint8Array(0);
  while (head.length < 2) {
    const { done, value } = await reader.read();
    if (done) break;
    if (value?.byteLength) head = concatBytes(head, value);
  }
  const gzip = head.length >= 2 && head[0] === GZ_BYTES[0] && head[1] === GZ_BYTES[1];
  const stream = new ReadableStream({
    start(controller) {
      if (head.byteLength) controller.enqueue(head);
    },
    async pull(controller) {
      const { done, value } = await reader.read();
      if (done) {
        controller.close();
        return;
      }
      if (value?.byteLength) controller.enqueue(value);
    },
    cancel(reason) {
      return reader.cancel(reason);
    },
  });
  return { gzip, stream };
}

// Two byte runs, joined. `head` stays small: the sniff never reads past two bytes.
function concatBytes(a, b) {
  const out = new Uint8Array(a.byteLength + b.byteLength);
  out.set(a, 0);
  out.set(b, a.byteLength);
  return out;
}

// The page's fetch bar follows the wire, so a compressed body reports the bytes the network
// sent (its content-length), not the larger bytes it decodes to.
function countWire(stream, total, onProgress) {
  if (!onProgress) return stream;
  let received = 0;
  return stream.pipeThrough(
    new TransformStream({
      transform(chunk, controller) {
        received += chunk.byteLength;
        onProgress({ received, total: total || received, done: false });
        controller.enqueue(chunk);
      },
      flush() {
        onProgress({ received, total: total || received, done: true });
      },
    }),
  );
}

// The first `n` bytes of the chunks read so far. The caller has checked that
// they are present; a short result means the body ended early.
function takeBytes(chunks, n) {
  const bytes = new Uint8Array(n);
  let at = 0;
  for (const chunk of chunks) {
    if (at >= n) break;
    const part = chunk.subarray(0, Math.min(chunk.byteLength, n - at));
    bytes.set(part, at);
    at += part.byteLength;
  }
  return bytes.subarray(0, at);
}

// The end of the tier-0 blobs in an E2BUNDLE header and index (QW4). The page
// resolves the kit fetch at this byte, so the first frame starts on the coarse
// level while the HDRI tier streams behind it. 0 means no tier-0 blob.
function firstTierEnd(head) {
  if (head.length < 32) return 0;
  let magic = "";
  for (let i = 0; i < 8; i++) magic += String.fromCharCode(head[i]);
  if (magic !== "E2BUNDLE") throw new Error("not an E2BUNDLE (bad magic)");
  const view = new DataView(head.buffer, head.byteOffset, head.byteLength);
  const count = view.getUint32(12, true);
  let at = 32;
  let first = 0;
  for (let i = 0; i < count; i++) {
    if (at + 2 > head.length) throw new Error("the bundle index is torn");
    const pathLen = view.getUint16(at, true);
    at += 2;
    if (at + pathLen + 25 > head.length) throw new Error("the bundle index is torn");
    at += pathLen;
    const priority = head[at];
    at += 1;
    const offset = Number(view.getBigUint64(at, true));
    at += 8;
    const len = Number(view.getBigUint64(at, true));
    at += 8;
    at += 8; // the blob's FNV-1a; the Rust reader verifies it on open
    if (priority === 0) first = Math.max(first, offset + len);
  }
  return first;
}

/**
 * Read a response body chunk by chunk so the page can show fetch progress
 * (QW4). Returns the whole body as a Uint8Array. `total` is 0 when the server
 * sent no content-length; the last callback then names the size it did read.
 * `onPrefix` gets the tier-0 prefix as soon as those bytes arrive, while the
 * reader keeps going for the rest of the bundle.
 */
async function readBody(stream, total, onProgress, onPrefix) {
  const reader = stream.getReader();
  const chunks = [];
  let received = 0;
  let indexEnd = 0;
  let first = 0;
  let sent = false;
  try {
    for (;;) {
      const { done, value } = await reader.read();
      if (done) break;
      if (!value || value.byteLength === 0) continue;
      chunks.push(value);
      received += value.byteLength;
      if (onPrefix && !sent) {
        if (!indexEnd && received >= 32) {
          const head = takeBytes(chunks, 32);
          const view = new DataView(head.buffer, head.byteOffset, head.byteLength);
          indexEnd = 32 + Number(view.getBigUint64(16, true));
        }
        if (indexEnd && received >= indexEnd) {
          first = firstTierEnd(takeBytes(chunks, indexEnd));
          if (first === 0) first = received; // no tier 0: the whole body is the prefix
        }
        if (first && received >= first) {
          onPrefix(takeBytes(chunks, first));
          sent = true;
        }
      }
      onProgress?.({ received, total: total || received, done: false });
    }
  } finally {
    reader.releaseLock?.();
  }
  if (onPrefix && !sent) onPrefix(takeBytes(chunks, received));
  onProgress?.({ received, total: total || received, done: true });
  return takeBytes(chunks, received);
}

/**
 * Fetch one gameplay asset at runtime (WEB6).
 *
 * Resolves { bytes, stats }: `bytes` is a Uint8Array of the whole body and `stats` is
 * { bytesFetched, downloadMs }. Rejects with an `AssetError` when the request itself fails, when
 * the server answers with a status other than 200, or when the body is empty. The caller never
 * gets an empty asset and never a silent failure. QW4: `onProgress` receives
 * { received, total, done } as the body streams in; `total` is 0 without a content-length.
 */
export async function fetchAsset(url, env = browserEnv(), onProgress = null, onPrefix = null) {
  const name = assetName(url);
  const t0 = env.now();
  let res;
  try {
    res = await env.fetch(url);
  } catch (e) {
    throw new AssetError(`${name} could not be fetched: ${reason(e)}`);
  }
  if (!res.ok) throw new AssetError(`${name} could not be fetched: HTTP ${res.status}`, res.status);
  let body;
  try {
    const total = Number(res.headers?.get?.("content-length")) || 0;
    if (res.body && typeof res.body.getReader === "function") {
      const { gzip, stream } = await sniffGzip(res.body);
      if (!gzip) {
        body = await readBody(stream, total, onProgress, onPrefix);
      } else {
        if (typeof DecompressionStream !== "function") {
          throw new AssetError(`${name} is gzip and this browser cannot decompress a stream (DecompressionStream is missing)`);
        }
        // The bar follows the compressed bytes; the prefix callback still sees the bundle.
        body = await readBody(countWire(stream, total, onProgress).pipeThrough(new DecompressionStream("gzip")), 0, null, onPrefix);
      }
    } else {
      body = new Uint8Array(await res.arrayBuffer());
    }
  } catch (e) {
    if (e instanceof AssetError) throw e;
    throw new AssetError(`${name} could not be read: ${reason(e)}`);
  }
  if (!body || body.byteLength === 0) throw new AssetError(`${name} is empty`);
  // A body with no stream (a test fake or an old browser) has no earlier
  // boundary to report, so the whole body is the prefix.
  if (onPrefix) onPrefix(new Uint8Array(body));
  return {
    bytes: new Uint8Array(body),
    stats: { bytesFetched: body.byteLength, downloadMs: Math.round(env.now() - t0) },
  };
}

/**
 * Fetch only the first tier of a bundle (QW4): the promise resolves as soon as
 * the header, the index and the tier-0 blobs have arrived, while the same
 * response keeps streaming for the progress bar. The bytes are a prefix the
 * Rust reader opens with `BundleSource::open_prefix`; the HDRI tier is not in
 * them, so the first frame does not wait for it.
 */
export async function fetchAssetPrefix(url, env = browserEnv(), onProgress = null) {
  const name = assetName(url);
  const t0 = env.now();
  let settle;
  let fail;
  let settled = false;
  const prefix = new Promise((resolve, reject) => {
    settle = resolve;
    fail = reject;
  });
  const full = fetchAsset(url, env, onProgress, (bytes) => {
    if (settled) return;
    settled = true;
    settle(bytes);
  });
  full.then(
    () => {
      if (!settled) {
        settled = true;
        fail(new AssetError(`${name} ended before its first tier`));
      }
    },
    (err) => {
      if (!settled) {
        settled = true;
        fail(err);
      } else {
        // The prefix is usable; the rest failed behind it. Hide the bar.
        onProgress?.({ received: 0, total: 0, done: true, error: true });
      }
    },
  );
  const bytes = await prefix;
  if (bytes.byteLength === 0) throw new AssetError(`${name} is empty`);
  return {
    bytes,
    stats: { bytesFetched: bytes.byteLength, downloadMs: Math.round(env.now() - t0) },
  };
}

/**
 * The fetcher the page hands the engine (WEB6): the engine asks for an asset by
 * URL and gets the asset's bytes as a Uint8Array, or a rejection it turns into a
 * typed refusal. `base` is the page's own URL, so a relative asset name lands in
 * the same folder as the page. QW4: `onProgress` draws the page's fetch bar.
 */
export const pageFetcher = (base, onProgress = null) => (url) =>
  fetchAsset(new URL(url, base), browserEnv(), onProgress).then((r) => r.bytes);

/**
 * The QW4 fetcher for the arena bundle: it answers with the tier-0 prefix as
 * soon as that arrives, not with the whole body. The page's progress bar still
 * follows the rest of the response in the background.
 */
export const pageFetcherPrefix = (base, onProgress = null) => (url) =>
  fetchAssetPrefix(new URL(url, base), browserEnv(), onProgress).then((r) => r.bytes);

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
