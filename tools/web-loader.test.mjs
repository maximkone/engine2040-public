// crates/web/loader.js drills (QW3, QW12): fake fetch, cache and compiler, so the numbers and the failure reasons are exact.
//   node --test tools/web-loader.test.mjs
import test from "node:test";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { gzipSync } from "node:zlib";
import { CACHE_NAME, LoadError, AssetError, browserEnv, fetchAsset, fetchAssetPrefix, pageFetcher, loadEngine, loadLine, watchFirstFrame } from "../crates/web/loader.js";

const WASM = Buffer.from("fake wasm bytes".repeat(100));
const SHA = createHash("sha256").update(WASM).digest("hex");
const BASE = "http://127.0.0.1:9/";
const KEY = `${BASE}engine_bg.wasm?sha256=${SHA}`;
const tick = () => new Promise((r) => setTimeout(r, 5));

// QW4: a two-tier E2BUNDLE, so the loader has a coarse boundary to stop at.
function miniBundle() {
  const coarse = Buffer.from("coarse bytes");
  const later = Buffer.from("hdri bytes");
  const rows = [
    { path: "cooked/a.bin", priority: 0, data: coarse },
    { path: "hdri/studio.hdr", priority: 1, data: later },
  ];
  const indexLen = rows.reduce((n, r) => n + 2 + r.path.length + 25, 0);
  const head = Buffer.alloc(32 + indexLen);
  head.write("E2BUNDLE", 0, "latin1");
  head.writeUInt32LE(1, 8);
  head.writeUInt32LE(rows.length, 12);
  head.writeBigUInt64LE(BigInt(indexLen), 16);
  head.writeBigUInt64LE(0n, 24);
  let at = 32;
  let offset = 32 + indexLen;
  for (const r of rows) {
    head.writeUInt16LE(r.path.length, at); at += 2;
    head.write(r.path, at, "latin1"); at += r.path.length;
    head[at] = r.priority; at += 1;
    head.writeBigUInt64LE(BigInt(offset), at); at += 8;
    head.writeBigUInt64LE(BigInt(r.data.length), at); at += 8;
    head.writeBigUInt64LE(0n, at); at += 8;
    offset += r.data.length;
  }
  return { bytes: Buffer.concat([head, coarse, later]), first: 32 + indexLen + coarse.length };
}

// A page's world: a server (what each path answers), a Cache API that survives between loads, a clock the fakes advance.
function world({ type = "application/wasm", status = 200, buildJson = { files: { "engine_bg.wasm": { sha256: SHA } } }, putThrows = null, persist = true } = {}) {
  const w = { clock: 0, fetched: [], store: new Map(), persistCalls: 0 };
  const cache = {
    match: async (k) => w.store.get(k)?.clone(),
    put: async (k, res) => { if (putThrows) throw putThrows; w.store.set(k, res); },
  };
  w.env = {
    fetch: async (url) => {
      const path = new URL(url).pathname.slice(1);
      w.fetched.push(path);
      if (path === "build.json") return buildJson ? new Response(JSON.stringify(buildJson)) : new Response("nope", { status: 404 });
      w.clock += 300; // the download
      return new Response(WASM, { status, headers: { "content-type": type } });
    },
    caches: { open: async (name) => { assert.equal(name, CACHE_NAME); return cache; } },
    storage: { persist: async () => { w.persistCalls++; return persist; } },
    wasm: { compileStreaming: async (res) => { await res.arrayBuffer(); await tick(); w.clock += 120; return { module: true }; } },
    now: () => w.clock,
    sha256: async (bytes) => createHash("sha256").update(Buffer.from(bytes)).digest("hex"),
  };
  return w;
}

test("first load: bytes fetched, download and compile split, copy stored under its hash, persistence asked", async () => {
  const w = world();
  const { module, stats, settled } = await loadEngine(BASE, w.env);
  assert.deepEqual(module, { module: true });
  await settled;
  assert.deepEqual(stats, { bundle: "network", bytesFetched: WASM.length, downloadMs: 300, compileMs: 120, cache: "stored", persisted: true });
  assert.deepEqual(w.fetched, ["build.json", "engine_bg.wasm"]);
  assert.ok(w.store.has(KEY), "keyed by the SHA-256 from build.json");
  assert.equal(w.persistCalls, 1);
});

test("second load in the same profile reads the cache: bundle=cache, bytes fetched 0, no wasm request", async () => {
  const w = world();
  await (await loadEngine(BASE, w.env)).settled;
  w.fetched.length = 0;
  const { stats } = await loadEngine(BASE, w.env);
  assert.equal(stats.bundle, "cache");
  assert.equal(stats.bytesFetched, 0);
  assert.equal(stats.cache, "hit");
  assert.deepEqual(w.fetched, ["build.json"], "only build.json (revalidated) goes to the network");
});

test("a new build misses the old copy: another hash is another key", async () => {
  const w = world();
  await (await loadEngine(BASE, w.env)).settled;
  w.fetched.length = 0;
  const other = "1".repeat(64);
  const w2 = world({ buildJson: { files: { "engine_bg.wasm": { sha256: other } } } });
  w2.store = w.store;
  const { stats, settled } = await loadEngine(BASE, w2.env);
  assert.equal(stats.bundle, "network");
  assert.equal((await settled).cache, "hash-mismatch", "bytes that do not hash to build.json are never stored");
});

test("QW3: a server that types the wasm text/plain gives a LoadError that names the type and the fix", async () => {
  const w = world({ type: "text/plain" });
  await assert.rejects(loadEngine(BASE, w.env), (e) => {
    assert.ok(e instanceof LoadError);
    assert.match(e.message, /served as "text\/plain", not application\/wasm/);
    assert.match(e.message, /must type \.wasm files as application\/wasm/);
    return true;
  });
  await assert.rejects(loadEngine(BASE, world({ type: "" }).env), /no content-type/);
  assert.equal((await loadEngine(BASE, world({ type: "application/wasm; charset=binary" }).env)).stats.bundle, "network", "parameters after the type are fine");
});

test("a missing file is a LoadError with the status", async () => {
  await assert.rejects(loadEngine(BASE, world({ status: 404 }).env), /could not be fetched: HTTP 404/);
});

test("QW12: QuotaExceededError on the cache write is reported and the page still gets its module", async () => {
  const w = world({ putThrows: Object.assign(new Error("full"), { name: "QuotaExceededError" }) });
  const { module, stats, settled } = await loadEngine(BASE, w.env);
  assert.deepEqual(module, { module: true });
  await settled;
  assert.equal(stats.cache, "quota");
  assert.equal(stats.persisted, null);
  assert.equal(w.persistCalls, 0, "no persistence request for a copy that was not stored");
});

test("no Cache API (plain http) or no build.json: the page streams without a cache and says so", async () => {
  const none = world();
  none.env.caches = null;
  const a = await loadEngine(BASE, none.env);
  assert.equal((await a.settled).cache, "off");
  assert.equal(a.stats.bundle, "network");
  const noJson = world({ buildJson: null });
  const b = await loadEngine(BASE, noJson.env);
  assert.equal((await b.settled).cache, "off");
  assert.equal(noJson.store.size, 0);
});

test("watchFirstFrame resolves at once when the body already says running", async () => {
  assert.equal(await watchFirstFrame({ dataset: { state: "running" } }, () => 42), 42);
  assert.equal(await watchFirstFrame({ dataset: { state: "fatal" } }, () => 7), 7);
});

test("loadLine prints the three numbers and what is not known yet is left out", () => {
  const stats = { bundle: "network", bytesFetched: 3399353, downloadMs: 812, compileMs: 95, cache: "stored", persisted: true };
  assert.equal(loadLine(stats, 40.4, 4650.2), "load: bundle=network fetched=3399353 download=812ms compile=95ms init=40ms first-frame=4650ms cache=stored persist=granted");
  assert.equal(loadLine({ ...stats, persisted: null }), "load: bundle=network fetched=3399353 download=812ms compile=95ms cache=stored");
});

test("the file is plain ASCII", () => {
  const src = readFileSync(join(dirname(fileURLToPath(import.meta.url)), "..", "crates", "web", "loader.js"), "utf8");
  assert.ok(/^[\x09\x0a\x0d\x20-\x7e]*$/.test(src));
});

// WEB6: the runtime fetches a gameplay asset at runtime, through the page's own network.
test("WEB6: the loader requests the gameplay asset and returns its bytes", async () => {
  const body = Buffer.from("packed bundle bytes");
  const asked = [];
  const env = {
    fetch: async (url) => { asked.push(String(url)); return new Response(body, { status: 200 }); },
    now: () => 0,
  };
  const { bytes, stats } = await fetchAsset("http://127.0.0.1:9/assets/arena.e2bundle", env);
  assert.deepEqual(asked, ["http://127.0.0.1:9/assets/arena.e2bundle"], "the asset is requested by URL");
  assert.deepEqual([...bytes], [...body], "the bytes come back whole");
  assert.equal(stats.bytesFetched, body.length);
});

// QW4: the page draws a fetch bar, so the loader reports bytes as they arrive and one final done.
test("QW4: fetchAsset reports streamed progress and a final done", async () => {
  const body = new TextEncoder().encode("hello world");
  const parts = [body.subarray(0, 4), body.subarray(4, 8), body.subarray(8)];
  let next = 0;
  const stream = new ReadableStream({
    async pull(controller) {
      if (next >= parts.length) { controller.close(); return; }
      controller.enqueue(parts[next++]);
      await new Promise((r) => setTimeout(r, 1));
    },
  });
  const events = [];
  const env = {
    fetch: async () => new Response(stream, { status: 200, headers: { "content-length": String(body.length) } }),
    now: () => 0,
  };
  const { bytes } = await fetchAsset("http://127.0.0.1:9/assets/arena.e2bundle", env, (p) => events.push(p));
  assert.deepEqual([...bytes], [...body], "the bytes come back whole");
  assert.ok(events.some((e) => !e.done && e.received > 0 && e.received < body.length), "progress arrives before the end");
  assert.deepEqual(events.at(-1), { received: body.length, total: body.length, done: true });
  assert.ok(events.every((e) => e.total === body.length), "the content-length is the total");
});

// QW4: the first frame starts at the tier-0 boundary, not at the end of the HDRI.
test("QW4: fetchAssetPrefix resolves at the tier-0 boundary while the rest streams", async () => {
  const { bytes: full, first } = miniBundle();
  let release;
  const gate = new Promise((r) => { release = r; });
  let phase = 0;
  const stream = new ReadableStream({
    async pull(controller) {
      if (phase === 0) {
        controller.enqueue(full.subarray(0, first));
        phase = 1;
        await gate;
      } else if (phase === 1) {
        controller.enqueue(full.subarray(first));
        phase = 2;
      } else controller.close();
    },
  });
  const events = [];
  const env = {
    fetch: async () => new Response(stream, { status: 200, headers: { "content-length": String(full.length) } }),
    now: () => 0,
  };
  const { bytes } = await fetchAssetPrefix("http://127.0.0.1:9/assets/arena.e2bundle", env, (p) => events.push(p));
  assert.deepEqual([...bytes], [...full.subarray(0, first)], "only the coarse tier is in the prefix");
  assert.ok(!events.at(-1).done, "the bar is still moving when the first frame starts");
  release();
  for (let i = 0; i < 100 && !events.at(-1).done; i++) await tick();
  assert.deepEqual(events.at(-1), { received: full.length, total: full.length, done: true });
});

// QW10: the two cooked bundles ship gzip. The page must get the same bytes back, the bar
// must follow the wire, and the first frame must still start at the tier-0 boundary: the
// stream decompresses while it arrives, so nothing waits for the whole file.
test("QW10: a gzip bundle decodes to the cooked bytes and the bar follows the wire", async () => {
  const body = Buffer.from("a cooked bundle, gzipped on disk\n".repeat(40));
  const gz = gzipSync(body, { level: 9 });
  const events = [];
  const env = {
    fetch: async () => new Response(gz, { status: 200, headers: { "content-length": String(gz.length) } }),
    now: () => 0,
  };
  const { bytes, stats } = await fetchAsset("http://127.0.0.1:9/assets/arena.e2bundle", env, (p) => events.push(p));
  assert.deepEqual([...bytes], [...body], "the engine reads the cooked bytes, never the gzip");
  assert.equal(stats.bytesFetched, body.length, "bytesFetched is what the engine got");
  assert.deepEqual(events.at(-1), { received: gz.length, total: gz.length, done: true }, "the bar counts the compressed bytes the network sent");
});

test("QW10: a gzip bundle still resolves the tier-0 prefix before the rest arrives", async () => {
  const { bytes: full, first } = miniBundle();
  const gz = gzipSync(full, { level: 9 });
  const headEnd = Math.floor(gz.length * 0.9); // the last tenth waits behind the gate
  let release;
  const gate = new Promise((r) => { release = r; });
  let phase = 0;
  const stream = new ReadableStream({
    async pull(controller) {
      if (phase === 0) {
        controller.enqueue(gz.subarray(0, headEnd));
        phase = 1;
        await gate;
      } else if (phase === 1) {
        controller.enqueue(gz.subarray(headEnd));
        phase = 2;
      } else controller.close();
    },
  });
  const events = [];
  const env = {
    fetch: async () => new Response(stream, { status: 200, headers: { "content-length": String(gz.length) } }),
    now: () => 0,
  };
  const { bytes } = await fetchAssetPrefix("http://127.0.0.1:9/assets/arena.e2bundle", env, (p) => events.push(p));
  assert.deepEqual([...bytes], [...full.subarray(0, first)], "only the coarse tier is in the prefix");
  assert.ok(!events.at(-1).done, "the bar is still moving when the first frame starts");
  release();
  for (let i = 0; i < 100 && !events.at(-1).done; i++) await tick();
  assert.deepEqual(events.at(-1), { received: gz.length, total: gz.length, done: true });
});

test("QW10: bytes that start like gzip but are not are a typed refusal, never a blank arena", async () => {
  const broken = Uint8Array.from([0x1f, 0x8b, 0x08, 0x00, 0, 0, 0, 0, 0, 0x03, 0xff, 0xff, 0xff, 0xff]);
  const env = { fetch: async () => new Response(broken, { status: 200 }), now: () => 0 };
  await assert.rejects(fetchAsset("http://127.0.0.1:9/assets/arena.e2bundle", env), (e) => {
    assert.ok(e instanceof AssetError, "the failure is typed");
    assert.match(e.message, /assets\/arena\.e2bundle could not be read/);
    return true;
  });
});

test("WEB6: a missing asset is a typed refusal that names it", async () => {
  const env = { fetch: async () => new Response("not found", { status: 404 }), now: () => 0 };
  await assert.rejects(fetchAsset("http://127.0.0.1:9/assets/arena.e2bundle", env), (e) => {
    assert.ok(e instanceof AssetError, "the failure is typed");
    assert.match(e.message, /assets\/arena\.e2bundle could not be fetched: HTTP 404/);
    assert.equal(e.status, 404, "the status travels with the refusal");
    return true;
  });
});

test("WEB6: an empty body and a failed request are typed refusals, not empty assets", async () => {
  const empty = { fetch: async () => new Response(new Uint8Array(0), { status: 200 }), now: () => 0 };
  await assert.rejects(fetchAsset("http://127.0.0.1:9/assets/arena.e2bundle", empty), /assets\/arena\.e2bundle is empty/);
  const offline = { fetch: async () => { throw new Error("offline"); }, now: () => 0 };
  await assert.rejects(fetchAsset("http://127.0.0.1:9/assets/arena.e2bundle", offline), /could not be fetched: offline/);
});

// QW4: the page's progress bar is fed by the loader, not by a fake timer.
test("QW4: the page draws the arena bundle fetch bar from loader progress", () => {
  const root = join(dirname(fileURLToPath(import.meta.url)), "..");
  const html = readFileSync(join(root, "crates", "web", "index.html"), "utf8");
  assert.match(html, /<progress id="fetch-bar"/, "the page has the bar");
  assert.match(html, /pageFetcher\(import\.meta\.url, showFetch\)/, "the rig fetch reports its progress");
  assert.match(html, /pageFetcherPrefix\(import\.meta\.url, showFetch\)/, "the arena fetch reports the streamed progress");
  assert.match(html, /window\.engine2040FetchAssetPrefix = /, "the prefix fetcher is installed for the engine");
  assert.match(html, /delete document\.body\.dataset\.fetch/, "the bar hides when the fetch ends or refuses");
});

test("WEB6: the fetcher the engine asks for is the one the page installs", () => {
  const root = join(dirname(fileURLToPath(import.meta.url)), "..");
  const rust = readFileSync(join(root, "crates", "web", "src", "content.rs"), "utf8");
  assert.match(rust, /pub const PAGE_FETCHER: &str = "engine2040FetchAsset"/, "the engine asks for the page's fetcher by this name");
  assert.match(rust, /pub const PAGE_PREFIX_FETCHER: &str = "engine2040FetchAssetPrefix"/, "QW4 names the prefix fetcher too");
  const html = readFileSync(join(root, "crates", "web", "index.html"), "utf8");
  assert.match(html, /window\.engine2040FetchAsset = /, "the page installs it under that name");
  assert.match(html, /window\.engine2040FetchAssetPrefix = /, "the page installs the prefix fetcher");
});

test("WEB6: the fetcher the page installs answers with the asset's bytes", async () => {
  const { serve } = await import("./web-browse.mjs");
  const { mkdtempSync, mkdirSync, rmSync, writeFileSync } = await import("node:fs");
  const { tmpdir } = await import("node:os");
  const dir = mkdtempSync(join(tmpdir(), "web-fetcher-"));
  const body = Buffer.from("e2bundle bytes", "utf8");
  mkdirSync(join(dir, "assets"));
  writeFileSync(join(dir, "assets", "arena.e2bundle"), body);
  try {
    const server = await serve(dir);
    try {
      // The page's own fetcher, the one it installs for the engine: a relative asset
      // name against the page's URL, answered with the asset's bytes.
      const fetcher = pageFetcher(`http://127.0.0.1:${server.port}/index.html`);
      const bytes = await fetcher("assets/arena.e2bundle");
      assert.ok(bytes instanceof Uint8Array, "the engine is handed a Uint8Array");
      assert.deepEqual([...bytes], [...body]);
      assert.equal(server.log.at(-1).path, "/assets/arena.e2bundle", "the server saw the asset request");
      await assert.rejects(fetcher("assets/missing.e2bundle"), /assets\/missing\.e2bundle could not be fetched: HTTP 404/);
    } finally {
      await server.close();
    }
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("WEB6 end to end: the real server serves the asset and the loader brings its bytes back", async () => {
  const { serve } = await import("./web-browse.mjs");
  const { mkdtempSync, mkdirSync, rmSync, writeFileSync } = await import("node:fs");
  const { tmpdir } = await import("node:os");
  const dir = mkdtempSync(join(tmpdir(), "web-asset-"));
  const body = Buffer.from("e2bundle bytes", "utf8");
  mkdirSync(join(dir, "assets"));
  writeFileSync(join(dir, "assets", "arena.e2bundle"), body);
  try {
    const server = await serve(dir);
    try {
      const { bytes } = await fetchAsset(`http://127.0.0.1:${server.port}/assets/arena.e2bundle`, { ...browserEnv(), caches: null });
      assert.deepEqual([...bytes], [...body]);
      assert.equal(server.log.at(-1).path, "/assets/arena.e2bundle", "the server saw the asset request");
      await assert.rejects(fetchAsset(`http://127.0.0.1:${server.port}/assets/missing.e2bundle`, { ...browserEnv(), caches: null }), /assets\/missing\.e2bundle could not be fetched: HTTP 404/);
    } finally {
      await server.close();
    }
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

// The real fetch and the real WebAssembly.compileStreaming against the harness's own static server.
test("end to end: a real server and a real streaming compile; text/plain on .wasm is the fatal reason", async () => {
  const { serve } = await import("./web-browse.mjs");
  const { mkdtempSync, rmSync, writeFileSync } = await import("node:fs");
  const { tmpdir } = await import("node:os");
  const dir = mkdtempSync(join(tmpdir(), "web-loader-"));
  writeFileSync(join(dir, "engine_bg.wasm"), Buffer.from([0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00])); // an empty module
  try {
    const good = await serve(dir);
    try {
      const { module, stats } = await loadEngine(`http://127.0.0.1:${good.port}/index.html`, { ...browserEnv(), caches: null });
      assert.ok(module instanceof WebAssembly.Module);
      assert.equal(stats.bytesFetched, 8);
      assert.equal(stats.cache, "off");
    } finally {
      await good.close();
    }
    const bad = await serve(dir, { wasmType: "text/plain" });
    try {
      await assert.rejects(loadEngine(`http://127.0.0.1:${bad.port}/index.html`, { ...browserEnv(), caches: null }), (e) => e instanceof LoadError && /served as "text\/plain", not application\/wasm/.test(e.message));
    } finally {
      await bad.close();
    }
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("QW14: watchMemory reads the pages now and on every tick, and sees a grow (the buffer is read fresh)", async () => {
  const { watchMemory, memoryLine } = await import("../crates/web/loader.js");
  const memory = new WebAssembly.Memory({ initial: 4, maximum: 100 });
  const log = [];
  let tick = null, cleared = false, clock = 0;
  const w = watchMemory(memory, log, { everyMs: 60000, now: () => (clock += 1000), every: (fn, ms) => { assert.equal(ms, 60000); tick = fn; return 7; }, stopEvery: (t) => { cleared = t === 7; } });
  assert.equal(w.sample(), 4);
  memory.grow(16);
  tick();
  assert.deepEqual(log, [{ ms: 1000, pages: 4 }, { ms: 2000, pages: 20 }]);
  assert.equal(memoryLine(log), "memory: pages=20 peak=20 bytes=1310720 samples=2");
  assert.equal(memoryLine([]), "memory: no reading yet");
  w.stop();
  assert.ok(cleared);
});
