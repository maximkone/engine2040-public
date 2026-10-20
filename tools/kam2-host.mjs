// KAM9: make the kam2-web build reachable from another PC with no setup there.
// A fixed loopback port serves target/kam2web and a free Cloudflare quick tunnel
// (the mechanism kam-web's docs/DEPLOY.md names) gives a public https address.
//   node tools/kam2-host.mjs            host until Ctrl+C (prints one URL line)
//   node tools/kam2-host.mjs --check    prove local + public + 60 frames, then stop
// Options: --port 8136 --frames 60 --out target/kam2host.png --chrome <path>
//          --no-build --rebuild
// Steps: reuse target/kam2web (or build it the way tools/kam2-web.mjs does),
// serve it on 127.0.0.1:<port>, start `cloudflared tunnel --url ... --no-autoupdate`,
// read the public URL from cloudflared, fetch it from this PC, then load
// `<public>/index.html?frames=N` in headless Chrome and save the frozen frame.
// Exit 0 when the proof holds, 1 when a proof failed, 2 when it cannot run.
// No account, key or secret: a quick tunnel is anonymous. Only the static page is
// exposed; the loopback server is not opened to the LAN.
import { spawn, spawnSync } from "node:child_process";
import { appendFileSync, cpSync, existsSync, mkdirSync, readFileSync, rmSync, statSync, writeFileSync } from "node:fs";
import { createServer } from "node:http";
import { tmpdir } from "node:os";
import { dirname, extname, join, normalize, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";
import { BASE_FLAGS, analyzePixels, decodePng, isDrawn, launchChrome } from "./web-browse.mjs";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const OUT_DIR = join(root, "target", "kam2web");
const DEFAULT_PNG = join(root, "target", "kam2host.png");
const TUNNEL_LOG = join(root, "target", "kam2host-tunnel.log");
const CHROME_LOG = join(root, "target", "kam2host-chrome.log");
const BUILT_WASM = join(root, "target", "wasm32-unknown-unknown", "web", "kam2_web.wasm");
const DEFAULT_PORT = 8136;
const REQUIRED = [
  "index.html",
  "app.js",
  "kam2web.js",
  "kam2web_bg.wasm",
  "soldier/manifest.json",
  "soldier/unit.soldier.atlas.png",
  "house/manifest.json",
  "house/house.cottage.atlas.png",
];

const MIME = {
  ".html": "text/html; charset=utf-8",
  ".js": "text/javascript; charset=utf-8",
  ".wasm": "application/wasm",
  ".json": "application/json",
  ".png": "image/png",
  ".txt": "text/plain; charset=utf-8",
  ".css": "text/css; charset=utf-8",
};

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const log = (line) => console.error(`[kam2-host] ${line}`);
const tail = (text, n = 25) => String(text).trimEnd().split(/\r?\n/).slice(-n).join("\n");

function run(cmd, args) {
  const r = spawnSync(cmd, args, { cwd: root, encoding: "utf8", maxBuffer: 1 << 28 });
  if (r.error) throw new Error(`${cmd} could not start: ${r.error.message}`);
  if (r.status !== 0) throw new Error(`${cmd} ${args.join(" ")} failed (exit ${r.status})\n${tail(`${r.stdout ?? ""}${r.stderr ?? ""}`)}`);
  return r.stdout ?? "";
}

function copy(from, to) {
  const source = join(root, from);
  if (!existsSync(source)) throw new Error(`${from} is missing`);
  cpSync(source, join(OUT_DIR, to));
}

// The same build tools/kam2-web.mjs makes: cargo wasm, wasm-bindgen, page, atlases.
function build() {
  log("cargo build --profile web --target wasm32-unknown-unknown -p kam2-web");
  run("cargo", ["build", "--profile", "web", "--target", "wasm32-unknown-unknown", "-p", "kam2-web"]);
  if (!existsSync(BUILT_WASM)) throw new Error(`${BUILT_WASM} was not written`);
  rmSync(OUT_DIR, { recursive: true, force: true });
  mkdirSync(join(OUT_DIR, "soldier"), { recursive: true });
  mkdirSync(join(OUT_DIR, "house"), { recursive: true });
  log("wasm-bindgen --target web");
  run("wasm-bindgen", [BUILT_WASM, "--target", "web", "--no-typescript", "--out-dir", OUT_DIR, "--out-name", "kam2web"]);
  copy("crates/kam2-web/page/index.html", "index.html");
  copy("crates/kam2-web/page/app.js", "app.js");
  copy("content/kam2/soldier/manifest.json", "soldier/manifest.json");
  copy("content/kam2/soldier/unit.soldier.atlas.png", "soldier/unit.soldier.atlas.png");
  copy("content/kam2/house/manifest.json", "house/manifest.json");
  copy("content/kam2/house/house.cottage.atlas.png", "house/house.cottage.atlas.png");
}

const isBuilt = () => REQUIRED.every((file) => existsSync(join(OUT_DIR, file)));

function ensureBuild(options) {
  if (options.rebuild) {
    build();
    return "built";
  }
  if (isBuilt()) {
    log(`reusing ${OUT_DIR}`);
    return "reused";
  }
  if (!options.build) throw new Error(`${OUT_DIR} holds no complete build and --no-build was given`);
  build();
  return "built";
}

function resolveRequest(dir, urlPath) {
  let p;
  try { p = decodeURIComponent(urlPath.split("?")[0]); } catch { return null; }
  if (p.includes("\0") || p.split(/[\\/]+/).includes("..")) return null;
  const base = resolve(dir);
  const rel = normalize(p === "/" ? "/index.html" : p).replace(/^[/\\]+/, "");
  const full = resolve(base, rel);
  return full === base || full.startsWith(base + sep) ? full : null;
}

async function serveFolder(dir, port) {
  const server = createServer((req, res) => {
    const file = resolveRequest(dir, req.url ?? "/");
    const ok = file && existsSync(file) && statSync(file).isFile();
    if (!ok) {
      res.writeHead(404, { "Content-Type": "text/plain; charset=utf-8" });
      res.end("not found");
      return;
    }
    res.writeHead(200, { "Content-Type": MIME[extname(file).toLowerCase()] ?? "application/octet-stream", "Cache-Control": "no-store" });
    res.end(readFileSync(file));
  });
  await new Promise((res, rej) => {
    server.once("error", rej);
    server.listen(port, "127.0.0.1", res);
  });
  return {
    port,
    close: () => new Promise((r) => {
      server.closeAllConnections?.();
      server.close(() => r());
    }),
  };
}

function findCloudflared() {
  const candidates = [
    process.env.KAM9_CLOUDFLARED,
    join(tmpdir(), "opencode", "cloudflared.exe"),
    join(process.env.LOCALAPPDATA ?? "", "cloudflared", "cloudflared.exe"),
    join(root, "tools", "cloudflared.exe"),
  ].filter(Boolean);
  const found = candidates.find((path) => existsSync(path));
  if (found) return found;
  const probe = spawnSync("where.exe", ["cloudflared"], { encoding: "utf8" });
  if (probe.status === 0) {
    const first = String(probe.stdout).trim().split(/\r?\n/)[0].trim();
    if (first && existsSync(first)) return first;
  }
  return null;
}

// The free quick tunnel: no login, no token. It prints its public address in a banner.
async function startTunnel(bin, port) {
  const args = ["tunnel", "--url", `http://127.0.0.1:${port}`, "--no-autoupdate"];
  writeFileSync(TUNNEL_LOG, `# ${bin} ${args.join(" ")}\n`);
  const child = spawn(bin, args, { cwd: root, stdio: ["ignore", "pipe", "pipe"], windowsHide: true });
  let text = "";
  const collect = (chunk) => {
    const part = chunk.toString();
    text += part;
    appendFileSync(TUNNEL_LOG, part);
  };
  child.stdout.on("data", collect);
  child.stderr.on("data", collect);

  const url = await new Promise((res, rej) => {
    const timer = setTimeout(() => rej(new Error(`cloudflared printed no public URL within 60 s (${bin} ${args.join(" ")}); see ${TUNNEL_LOG}`)), 60000);
    const scan = () => {
      const match = /https:\/\/[a-z0-9-]+\.trycloudflare\.com/.exec(text);
      if (match) {
        clearTimeout(timer);
        res(match[0]);
      }
    };
    child.stdout.on("data", scan);
    child.stderr.on("data", scan);
    child.once("error", (error) => {
      clearTimeout(timer);
      rej(new Error(`cloudflared could not start (${bin} ${args.join(" ")}): ${error.message}`));
    });
    child.once("exit", (code) => {
      clearTimeout(timer);
      rej(new Error(`cloudflared exited with ${code} before it printed a URL (${bin} ${args.join(" ")}); see ${TUNNEL_LOG}`));
    });
  });

  return {
    url: url.replace(/\/+$/, ""),
    async stop() {
      if (child.exitCode !== null) return;
      child.kill();
      for (let i = 0; i < 50 && child.exitCode === null; i += 1) await sleep(200);
      if (child.exitCode === null) {
        try { child.kill("SIGKILL"); } catch { /* already gone */ }
        for (let i = 0; i < 25 && child.exitCode === null; i += 1) await sleep(200);
      }
    },
  };
}

// Fetch until the address answers with the page (a fresh tunnel needs a few seconds at the edge).
async function fetchPage(url, { timeoutMs, want = null } = {}) {
  const started = Date.now();
  let last = "no attempt";
  while (Date.now() - started < timeoutMs) {
    try {
      const response = await fetch(url, { headers: { "cache-control": "no-store" }, signal: AbortSignal.timeout(20000) });
      const body = want ? await response.text() : "";
      last = `HTTP ${response.status}`;
      if (response.ok && (!want || body.includes(want))) return { status: response.status, body };
    } catch (error) {
      last = error.message;
    }
    await sleep(2000);
  }
  throw new Error(`${url} did not answer as expected within ${timeoutMs} ms (last: ${last})`);
}

async function evaluate(cdp, expression) {
  const result = await cdp.send("Runtime.evaluate", { expression, returnByValue: true, awaitPromise: true });
  if (result.exceptionDetails) {
    throw new Error(`evaluate: ${result.exceptionDetails.text} ${result.exceptionDetails.exception?.description ?? ""}`.trim());
  }
  return result.result.value;
}

async function waitFor(cdp, expression, ok, timeoutMs, label) {
  const started = Date.now();
  let last = null;
  while (Date.now() - started < timeoutMs) {
    await sleep(200);
    try {
      last = await evaluate(cdp, expression);
    } catch {
      continue;
    }
    if (ok(last)) return last;
  }
  throw new Error(`${label} within ${timeoutMs} ms (last ${JSON.stringify(last)})`);
}

function findChrome() {
  if (process.env.KAM9_CHROME) return process.env.KAM9_CHROME;
  const candidates = [
    "C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe",
    "C:\\Program Files (x86)\\Google\\Chrome\\Application\\chrome.exe",
    "C:\\Program Files (x86)\\Microsoft\\Edge\\Application\\msedge.exe",
    "C:\\Program Files\\Microsoft\\Edge\\Application\\msedge.exe",
    "/usr/bin/google-chrome",
    "/usr/bin/chromium",
  ];
  return candidates.find((path) => existsSync(path)) ?? "google-chrome";
}

// Load the public page for exactly `frames` animation frames and keep its frozen picture.
async function drive(publicPageUrl, frames, pngPath, chrome) {
  const browser = await launchChrome(chrome, BASE_FLAGS, CHROME_LOG);
  const { cdp } = browser;
  const exceptions = [];
  try {
    cdp.on("Runtime.exceptionThrown", (p) => exceptions.push(String(p.exceptionDetails?.exception?.description ?? p.exceptionDetails?.text ?? "exception").slice(0, 400)));
    await cdp.send("Page.enable");
    await cdp.send("Runtime.enable");
    await cdp.send("Log.enable");
    await cdp.send("Emulation.setDeviceMetricsOverride", { width: 1280, height: 720, deviceScaleFactor: 1, mobile: false });
    await cdp.send("Page.navigate", { url: `${publicPageUrl}?frames=${frames}` });
    const state = await waitFor(
      cdp,
      "[document.body.dataset.state ?? null, document.getElementById('report')?.textContent ?? '']",
      (v) => v && (v[0] === "frozen" || v[0] === "fatal"),
      150000,
      "the public page never reached its frame budget",
    );
    if (state[0] === "fatal") throw new Error(`the public page failed: ${state[1]}`);
    const ran = await evaluate(cdp, "window.kam2Frames");
    if (ran !== frames) throw new Error(`the public page ran ${ran} frames, asked ${frames}`);
    const report = JSON.parse(state[1]);
    if (report.ok !== true) throw new Error(`the report is not ok: ${state[1]}`);
    if (!/^[0-9a-f]{16}$/.test(report.hash)) throw new Error(`the report hash is not 16 hex digits: ${report.hash}`);
    if (!(report.tick > 0)) throw new Error(`the wasm module played no tick in ${frames} frames`);
    const shot = Buffer.from((await cdp.send("Page.captureScreenshot", { format: "png" })).data, "base64");
    writeFileSync(pngPath, shot);
    const stats = analyzePixels(decodePng(shot));
    if (!isDrawn(stats)) throw new Error(`the screenshot is blank (${JSON.stringify(stats)})`);
    if (exceptions.length) throw new Error(`uncaught page exceptions: ${exceptions.join(" | ")}`);
    return { frames: ran, report, bytes: statSync(pngPath).size };
  } finally {
    await browser.close();
  }
}

function parseArgs(argv) {
  const options = { check: false, port: DEFAULT_PORT, frames: 60, out: DEFAULT_PNG, chrome: null, build: true, rebuild: false };
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    if (arg === "--check") options.check = true;
    else if (arg === "--port") options.port = Number(argv[++i]);
    else if (arg === "--frames") options.frames = Number(argv[++i]);
    else if (arg === "--out") options.out = resolve(argv[++i] ?? DEFAULT_PNG);
    else if (arg === "--chrome") options.chrome = argv[++i];
    else if (arg === "--no-build") options.build = false;
    else if (arg === "--rebuild") options.rebuild = true;
    else if (arg === "--help") {
      console.log("usage: node tools/kam2-host.mjs [--check] [--port 8136] [--frames 60] [--out target/kam2host.png] [--chrome <path>] [--no-build] [--rebuild]");
      process.exit(0);
    } else throw new Error(`unknown argument ${arg}`);
  }
  if (!Number.isInteger(options.port) || options.port < 1024 || options.port > 65535) throw new Error("--port needs a whole number from 1024 to 65535");
  if (!Number.isInteger(options.frames) || options.frames < 1) throw new Error("--frames needs a whole number of 1 or more");
  return options;
}

const options = (() => {
  try {
    return parseArgs(process.argv.slice(2));
  } catch (error) {
    console.error(`kam2-host: ${error.message}`);
    process.exit(2);
  }
})();

async function check() {
  const mode = ensureBuild(options);
  log(`build ${mode}`);
  const server = await serveFolder(OUT_DIR, options.port);
  const localUrl = `http://127.0.0.1:${options.port}/index.html`;
  let tunnel = null;
  try {
    const local = await fetchPage(localUrl, { timeoutMs: 15000, want: "<canvas id=\"stage\"" });
    log(`local index.html HTTP ${local.status}`);

    const bin = findCloudflared();
    if (!bin) {
      throw new Error(`cloudflared is missing: "cloudflared tunnel --url http://127.0.0.1:${options.port} --no-autoupdate" cannot start. Put cloudflared.exe on PATH or set KAM9_CLOUDFLARED=<path> (free, no account).`);
    }
    log(`starting quick tunnel with ${bin}`);
    tunnel = await startTunnel(bin, options.port);
    const publicUrl = `${tunnel.url}/index.html`;
    const publicPage = await fetchPage(publicUrl, { timeoutMs: 120000, want: "<canvas id=\"stage\"" });
    log(`public index.html HTTP ${publicPage.status} at ${publicUrl}`);

    const result = await drive(publicUrl, options.frames, options.out, options.chrome ?? findChrome());
    console.log(`KAM9 OK local=${localUrl} public=${publicUrl} frames=${result.frames} png=${options.out} bytes=${result.bytes}`);
    return 0;
  } finally {
    if (tunnel) await tunnel.stop();
    await server.close();
  }
}

async function host() {
  const mode = ensureBuild(options);
  log(`build ${mode}`);
  const server = await serveFolder(OUT_DIR, options.port);
  const localUrl = `http://127.0.0.1:${options.port}/index.html`;
  let tunnel = null;
  let stopping = false;
  const stop = async (why) => {
    if (stopping) return;
    stopping = true;
    log(`stopping (${why})`);
    if (tunnel) await tunnel.stop();
    await server.close();
    log("stopped: server and tunnel are gone");
    process.exit(0);
  };
  process.on("SIGINT", () => { void stop("SIGINT"); });
  process.on("SIGTERM", () => { void stop("SIGTERM"); });

  try {
    const bin = findCloudflared();
    if (!bin) throw new Error(`cloudflared is missing: "cloudflared tunnel --url http://127.0.0.1:${options.port} --no-autoupdate" cannot start. Put cloudflared.exe on PATH or set KAM9_CLOUDFLARED=<path> (free, no account).`);
    log(`starting quick tunnel with ${bin}`);
    tunnel = await startTunnel(bin, options.port);
    const publicUrl = `${tunnel.url}/index.html`;
    await fetchPage(publicUrl, { timeoutMs: 120000, want: "<canvas id=\"stage\"" });
    console.log(`KAM9 HOST local=${localUrl} public=${publicUrl}`);
    log("Ctrl+C stops the server and the tunnel this process started.");
  } catch (error) {
    console.log(`KAM9 WALL local=${localUrl} public=none reason=${JSON.stringify(error.message)}`);
    log("the local server stays up; Ctrl+C stops it.");
  }
}

try {
  process.exitCode = options.check ? await check() : await host();
} catch (error) {
  console.error(`KAM9 FAIL ${error.message}`);
  process.exitCode = 1;
}
