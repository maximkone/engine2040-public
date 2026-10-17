// web-headers drills (QW13): a fake server answers HEAD requests; the facts, the verdict and the retry are checked.
//   node --test tools/web-headers.test.mjs
import test from "node:test";
import assert from "node:assert/strict";
import { FILES, WASM_FILE, parseArgs, probeAll, probeFile, summaryLine, verdict } from "./web-headers.mjs";

// A site: path -> [status, headers]. Records the Accept-Encoding it was asked with.
function site(table) {
  const asked = [];
  const fetchFn = async (url, init) => {
    asked.push({ path: new URL(url).pathname, method: init.method, encoding: init.headers["accept-encoding"] });
    const [status, headers] = table[new URL(url).pathname.split("/").pop()] ?? [404, { "content-type": "text/html" }];
    return new Response(null, { status, headers });
  };
  return { fetchFn, asked };
}

const PAGES = Object.fromEntries(FILES.map((f) => [f, [200, { "content-type": f.endsWith(".wasm") ? "application/wasm" : "text/plain", "content-encoding": "gzip", "content-length": "1893357", "cache-control": "max-age=600" }]]));

test("probeFile asks like a browser (HEAD, br and gzip) and reads the three headers", async () => {
  const s = site(PAGES);
  const row = await probeFile("https://h.example/p/", WASM_FILE, s.fetchFn);
  assert.deepEqual(row, { file: WASM_FILE, status: 200, contentType: "application/wasm", contentEncoding: "gzip", contentLength: 1893357, cacheControl: "max-age=600" });
  assert.deepEqual(s.asked, [{ path: "/p/engine_bg.wasm", method: "HEAD", encoding: "br, gzip" }]);
  const bare = site({ "x.bin": [200, {}] });
  assert.deepEqual(await probeFile("https://h.example/", "x.bin", bare.fetchFn), { file: "x.bin", status: 200, contentType: null, contentEncoding: "identity", contentLength: null, cacheControl: null });
});

test("the verdict needs a 200 everywhere and application/wasm on the module; brotli is served only if some file came back br", async () => {
  const rows = await probeAll("https://h.example/", { fetchFn: site(PAGES).fetchFn });
  assert.deepEqual(verdict(rows), { ok: true, problems: [], brotliServed: false });
  assert.equal(summaryLine(rows), "web-headers: wasm type=application/wasm encoding=gzip bytes=1893357 brotli=not-served");
  const br = rows.map((r) => (r.file === "index.html" ? { ...r, contentEncoding: "br" } : r));
  assert.equal(verdict(br).brotliServed, true);
  assert.match(summaryLine(br), /brotli=served$/);
  const wrong = rows.map((r) => (r.file === WASM_FILE ? { ...r, contentType: "text/plain" } : r));
  assert.match(verdict(wrong).problems[0], /typed "text\/plain", not application\/wasm/);
  assert.match(verdict(rows.map((r) => (r.file === "build.json" ? { ...r, status: 404 } : r))).problems[0], /build\.json: HTTP 404/);
  assert.match(verdict(rows.filter((r) => r.file !== WASM_FILE)).problems[0], /was not probed/);
});

test("a deploy that reaches the edge late is retried; one that never does is reported", async () => {
  let calls = 0;
  const late = async (url, init) => {
    const path = new URL(url).pathname.slice(1);
    if (path === WASM_FILE && ++calls < 3) return new Response(null, { status: 404 });
    return new Response(null, { status: 200, headers: { "content-type": path === WASM_FILE ? "application/wasm" : "text/plain" } });
  };
  const rows = await probeAll("https://h.example/", { fetchFn: late, retries: 4, waitMs: 1 });
  assert.equal(verdict(rows).ok, true);
  assert.equal(calls, 3);
  const never = await probeAll("https://h.example/", { fetchFn: site({}).fetchFn, retries: 2, waitMs: 1 });
  assert.equal(verdict(never).ok, false);
});

test("parseArgs", () => {
  assert.deepEqual(parseArgs(["--url", "https://h/p/", "--retries", "6", "--wait", "10", "--out", "o.json"]), { url: "https://h/p/", out: "o.json", retries: 6, wait: 10, error: null });
  assert.match(parseArgs([]).error, /--url/);
  assert.match(parseArgs(["--url", "https://h"]).error, /ending in \//);
  assert.match(parseArgs(["--url", "https://h/", "--retries", "0"]).error, /--retries/);
  assert.match(parseArgs(["--url", "https://h/", "--x"]).error, /unknown argument/);
});
