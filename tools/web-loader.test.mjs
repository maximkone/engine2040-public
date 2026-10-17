// crates/web/loader.js drills (QW3, QW12): fake fetch, cache and compiler, so the numbers and the failure reasons are exact.
//   node --test tools/web-loader.test.mjs
import test from "node:test";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { CACHE_NAME, LoadError, browserEnv, loadEngine, loadLine, watchFirstFrame } from "../crates/web/loader.js";

const WASM = Buffer.from("fake wasm bytes".repeat(100));
const SHA = createHash("sha256").update(WASM).digest("hex");
const BASE = "http://127.0.0.1:9/";
const KEY = `${BASE}engine_bg.wasm?sha256=${SHA}`;
const tick = () => new Promise((r) => setTimeout(r, 5));

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
