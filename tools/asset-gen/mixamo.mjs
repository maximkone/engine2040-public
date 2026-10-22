#!/usr/bin/env node
// Mixamo animation packs, fetched into a local folder outside git. Any agent or
// script can call it; nothing it writes goes into the repo.
//
//   node tools/asset-gen/mixamo.mjs --login                       sign in once, by hand, in a visible browser
//   node tools/asset-gen/mixamo.mjs --list <query>                headless: matching packs and their animation counts
//   node tools/asset-gen/mixamo.mjs --pack "<Pack Name>" [--fps 60] [--out <dir>] [--dry-run]
//   node tools/asset-gen/mixamo.mjs --help
//
// The tool never types credentials and never reads, prints or saves a password.
// The owner signs in once with --login; the browser profile keeps the session
// after that. Tokens and cookies stay inside the page: they are never printed,
// logged or written to a file.
//
// Output: <out>/<pack_slug>/ holds the unzipped FBX files and PROVENANCE.json.
// Resumable: a pack whose folder already holds one FBX per animation plus one
// (the character) is skipped before any export.
//
// Browser: the installed Edge (first) or Chrome, driven over CDP with built-in
// WebSocket and fetch (no Playwright). Unlike the loopback-only skill
// real-browser-automation, this tool must reach mixamo.com and the Adobe sign-in
// hosts, so its request guard is an allowlist of those hosts instead of loopback.
import { spawn, execFile } from "node:child_process";
import { createWriteStream, existsSync, mkdirSync, readFileSync, readdirSync, renameSync, rmSync, statSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { Readable } from "node:stream";
import { pipeline } from "node:stream/promises";
import { fileURLToPath } from "node:url";

export const DEFAULT_PROFILE = "C:\\forge-media\\mixamo-lane\\profile2";
export const DEFAULT_OUT = "C:\\forge-media\\mixamo";
export const SITE_URL = "https://www.mixamo.com/#/";
export const FPS_CHOICES = [24, 30, 60];
export const FORMAT = "fbx7_2019";
export const SOURCE = "Adobe Mixamo";
export const LICENCE_NOTE = "Mixamo: royalty-free for use in games; do not redistribute the raw files as a library";
export const API_KEY = "mixamo2";
export const LOGIN_POLL_MS = 3000;
export const LOGIN_TIMEOUT_MS = 10 * 60 * 1000;
export const EXPORT_POLL_MS = 3000;
export const EXPORT_TIMEOUT_MS = 10 * 60 * 1000;
export const RETRIES = 3;
export const RETRY_PAUSE_MS = 20 * 1000;
// Host suffixes the headless browser may reach. Everything else fails at the request guard.
// The last entry is the CloudFront host Mixamo serves its own app bundle from (seen 2026-10-22);
// without it the signed-out page renders no text at all.
export const ALLOWED_HOST_SUFFIXES = ["mixamo.com", "adobe.com", "adobelogin.com", "adobe.io", "adobeccstatic.com", "typekit.net", "d2z4fokrw4vfz7.cloudfront.net"];

export const EXIT = { ok: 0, error: 1, usage: 2, notSignedIn: 4 };

const USAGE = `usage:
  node tools/asset-gen/mixamo.mjs --login
  node tools/asset-gen/mixamo.mjs --list <query>
  node tools/asset-gen/mixamo.mjs --pack "<Pack Name>" [--fps 60] [--out <dir>] [--character <id>] [--dry-run] [--trace]
  node tools/asset-gen/mixamo.mjs --help

  --login      open a VISIBLE Edge or Chrome on the Mixamo profile; you sign in by hand, the tool waits (up to 10 min)
  --list       headless: packs whose name matches <query>, with their animation counts
  --pack       headless: export the pack as FBX (${FORMAT}, T-pose with skin, keyframe reduction none) and unzip it
  --fps        ${FPS_CHOICES.join(", ")} (default 60)
  --out        output root (default ${DEFAULT_OUT}); the pack lands in <out>/<pack_slug>/
  --character  Mixamo character id to export on (default: the account's first character)
  --dry-run    print the --pack plan; no browser, no network
  --trace      log Mixamo API calls to stderr (method, path, status; never headers or bodies)
env: MIXAMO_PROFILE (default ${DEFAULT_PROFILE}), MIXAMO_BROWSER (path to msedge.exe or chrome.exe)
exit: 0 ok, 1 error, 2 usage, 4 not signed in (run --login first)`;

export class UsageError extends Error {}

// ------------------------------------------------------------------ pure parts

/** Parse argv into one command plus its options. Throws UsageError on a bad argument. */
export function parseArgs(argv) {
  const o = { command: null, query: null, pack: null, fps: 60, out: DEFAULT_OUT, character: null, dryRun: false, trace: false, help: false };
  const value = (i, flag) => {
    if (i + 1 >= argv.length || argv[i + 1].startsWith("--")) throw new UsageError(`${flag} needs a value`);
    return argv[i + 1];
  };
  const setCommand = (name) => {
    if (o.command && o.command !== name) throw new UsageError(`pick one command: --${o.command} or --${name}`);
    o.command = name;
  };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    switch (a) {
      case "--help":
      case "-h": o.help = true; break;
      case "--login": setCommand("login"); break;
      case "--list": setCommand("list"); o.query = value(i, a); i++; break;
      case "--pack": setCommand("pack"); o.pack = value(i, a); i++; break;
      case "--fps": {
        const v = Number(value(i, a)); i++;
        if (!FPS_CHOICES.includes(v)) throw new UsageError(`--fps must be one of ${FPS_CHOICES.join(", ")}`);
        o.fps = v; break;
      }
      case "--out": o.out = value(i, a); i++; break;
      case "--character": o.character = value(i, a); i++; break;
      case "--dry-run": o.dryRun = true; break;
      case "--trace": o.trace = true; break;
      default: throw new UsageError(`unknown argument: ${a}`);
    }
  }
  if (!o.help && !o.command) throw new UsageError("pick one command: --login, --list <query> or --pack \"<Pack Name>\"");
  if (o.command === "list" && !o.query.trim()) throw new UsageError("--list needs a non-empty query");
  if (o.command === "pack" && !o.pack.trim()) throw new UsageError("--pack needs a pack name");
  return o;
}

/** "Sword and Shield Pack" -> "sword_and_shield_pack". */
export function packSlug(name) {
  const s = String(name).toLowerCase().replace(/[^a-z0-9]+/g, "_").replace(/^_+|_+$/g, "");
  if (!s) throw new UsageError(`pack name gives an empty folder name: ${name}`);
  return s;
}

/** Complete when the folder holds one FBX per animation plus one (the character). Unknown count is never complete. */
export function isComplete(fbxCount, animations) {
  return Number.isInteger(animations) && animations > 0 && fbxCount >= animations + 1;
}

export function countFbx(dir) {
  if (!existsSync(dir)) return 0;
  return readdirSync(dir).filter((f) => f.toLowerCase().endsWith(".fbx")).length;
}

/** Folder state from the disk: the FBX count and whether the pack is complete. */
export function folderStatus(dir, animations) {
  const fbx = countFbx(dir);
  return { fbx, complete: isComplete(fbx, animations) };
}

/** Animation count recorded by an earlier run, or null. */
export function recordedAnimations(dir) {
  const p = join(dir, "PROVENANCE.json");
  if (!existsSync(p)) return null;
  try {
    const n = JSON.parse(readFileSync(p, "utf8")).animations;
    return Number.isInteger(n) ? n : null;
  } catch {
    return null;
  }
}

export function provenanceRecord({ pack, slug, animations, fps, character, fetchedAt }) {
  return {
    pack,
    pack_slug: slug,
    animations,
    fps,
    format: FORMAT,
    pose: "t-pose",
    skin: true,
    keyframe_reduction: "none",
    character: character || null,
    source: SOURCE,
    licence: LICENCE_NOTE,
    fetched_at: fetchedAt,
    tool: "tools/asset-gen/mixamo.mjs",
  };
}

/** The export request body the page API takes for a whole pack. */
export function exportBody({ pack, character, fps }) {
  return {
    character_id: character,
    type: "MotionPack",
    product_name: pack,
    preferences: { format: FORMAT, mesh: "t-pose", skin: "true", fps: String(fps), reducekf: "0" },
    gms_hash: [],
  };
}

/** Pick the pack whose name matches exactly (case and spacing ignored). Throws with near names when none match. */
export function findPack(packs, name) {
  const norm = (s) => String(s).toLowerCase().replace(/\s+/g, " ").trim();
  const hit = packs.find((p) => norm(p.name) === norm(name));
  if (hit) return hit;
  const near = packs.filter((p) => norm(p.name).includes(norm(name)) || norm(name).includes(norm(p.name))).slice(0, 5).map((p) => `"${p.name}"`);
  throw new Error(`no pack named "${name}"${near.length ? `; close names: ${near.join(", ")}` : ""}`);
}

/**
 * Tolerant reader for a products search reply. Field names follow what the page
 * uses (id, name, type); the animation count is taken from whichever field the
 * reply carries, and is null when it carries none.
 */
export function parseProducts(body) {
  const rows = Array.isArray(body) ? body : Array.isArray(body?.results) ? body.results : [];
  return rows
    .filter((r) => r && typeof r.id === "string" && typeof r.name === "string")
    .map((r) => ({
      id: r.id,
      name: r.name,
      type: r.type || null,
      animations: countOf(r),
    }));
}

function countOf(r) {
  for (const k of ["motion_count", "motions_count", "num_motions", "animation_count"]) {
    if (Number.isInteger(r[k])) return r[k];
  }
  if (Array.isArray(r.motions)) return r.motions.length;
  return null;
}

export function productsPath(query, limit = 24) {
  return `/api/v1/products?page=1&limit=${limit}&type=${encodeURIComponent("Motion,MotionPack")}&query=${encodeURIComponent(query)}`;
}

/** True when the URL's host is one of the allowed suffixes, or the URL stays inside the browser (data, blob, about). */
export function hostAllowed(url) {
  let u;
  try { u = new URL(String(url)); } catch { return false; }
  if (u.protocol === "data:" || u.protocol === "blob:" || u.protocol === "about:") return true;
  if (!["http:", "https:", "ws:", "wss:"].includes(u.protocol)) return false;
  const h = u.hostname.toLowerCase();
  return ALLOWED_HOST_SUFFIXES.some((s) => h === s || h.endsWith("." + s));
}

/**
 * Run fn; on failure wait `pauseMs` and try again, at most `retries` times.
 * Pure apart from the injected sleep, so tests run with no delay.
 */
export async function withRetry(fn, { retries = RETRIES, pauseMs = RETRY_PAUSE_MS, sleepFn = sleep, onRetry = () => {} } = {}) {
  let last;
  for (let attempt = 0; attempt <= retries; attempt++) {
    try {
      return await fn();
    } catch (e) {
      last = e;
      if (attempt === retries) break;
      onRetry(e, attempt + 1);
      await sleepFn(pauseMs);
    }
  }
  throw last;
}

/** Lines for --dry-run. No network, no browser: it only reads the folder. */
export function planLines({ pack, slug, out, fps, character, folder }) {
  const dir = join(out, slug);
  const state = folder.complete
    ? `complete (${folder.fbx} FBX): the export would be skipped`
    : folder.fbx > 0
      ? `partial (${folder.fbx} FBX): the export runs again into the same folder`
      : "absent: the pack is fetched in full";
  return [
    "mixamo --pack dry run (no browser, no network)",
    `pack:       ${pack}`,
    `slug:       ${slug}`,
    `folder:     ${dir}`,
    `state:      ${state}`,
    `format:     ${FORMAT}, T-pose with skin, ${fps} fps, keyframe reduction none`,
    `character:  ${character || "(the account's first character, found on the page)"}`,
    `profile:    ${profilePath()}`,
    "steps:",
    "  1 find the pack on the page's own API (headless, signed-in profile)",
    "  2 skip when the folder is already complete",
    "  3 request the FBX export of the whole pack",
    "  4 poll the export until it is ready",
    "  5 download the zip (one at a time; 3 retries, 20 s apart)",
    "  6 unzip into the folder, count the FBX, write PROVENANCE.json",
  ];
}

export function profilePath() {
  return process.env.MIXAMO_PROFILE || DEFAULT_PROFILE;
}

export function provenanceJson(record) {
  return JSON.stringify(record, null, 2) + "\n";
}

function sleep(ms) {
  return new Promise((r) => setTimeout(r, ms));
}

function withTimeout(promise, ms, what) {
  let t;
  const timer = new Promise((_, rej) => { t = setTimeout(() => rej(new Error(`timeout after ${ms} ms: ${what}`)), ms); });
  return Promise.race([promise, timer]).finally(() => clearTimeout(t));
}

// ------------------------------------------------------------------ browser (CDP)

function findBrowserExe() {
  if (process.env.MIXAMO_BROWSER) {
    if (!existsSync(process.env.MIXAMO_BROWSER)) throw new Error(`MIXAMO_BROWSER not found: ${process.env.MIXAMO_BROWSER}`);
    return process.env.MIXAMO_BROWSER;
  }
  const e = process.env;
  const roots = [e.ProgramFiles, e["ProgramFiles(x86)"], e.LOCALAPPDATA].filter(Boolean);
  const candidates = [
    ...roots.map((r) => join(r, "Microsoft", "Edge", "Application", "msedge.exe")),
    ...roots.map((r) => join(r, "Google", "Chrome", "Application", "chrome.exe")),
  ];
  const hit = candidates.find((p) => existsSync(p));
  if (!hit) throw new Error("no Edge or Chrome found; set MIXAMO_BROWSER to an executable");
  return hit;
}

class Cdp {
  constructor(ws) {
    this.ws = ws;
    this.nextId = 1;
    this.pending = new Map();
    this.handlers = [];
    ws.addEventListener("message", (ev) => {
      const m = JSON.parse(typeof ev.data === "string" ? ev.data : Buffer.from(ev.data).toString("utf8"));
      if (m.id !== undefined) {
        const p = this.pending.get(m.id);
        if (!p) return;
        this.pending.delete(m.id);
        if (m.error) p.reject(new Error(`${p.method}: ${m.error.message}`));
        else p.resolve(m.result || {});
      } else {
        for (const h of this.handlers) h(m);
      }
    });
    ws.addEventListener("close", () => {
      for (const p of this.pending.values()) p.reject(new Error("browser connection closed"));
      this.pending.clear();
    });
  }

  static open(url) {
    return new Promise((resolveOpen, reject) => {
      const ws = new WebSocket(url);
      ws.addEventListener("open", () => resolveOpen(new Cdp(ws)), { once: true });
      ws.addEventListener("error", () => reject(new Error("CDP WebSocket connect failed")), { once: true });
    });
  }

  send(method, params = {}, sessionId, timeout = 30000) {
    const id = this.nextId++;
    const msg = { id, method, params };
    if (sessionId) msg.sessionId = sessionId;
    const p = new Promise((resolveSend, reject) => {
      this.pending.set(id, { resolve: resolveSend, reject, method });
      this.ws.send(JSON.stringify(msg));
    });
    return withTimeout(p, timeout, method).finally(() => this.pending.delete(id));
  }
}

/**
 * Start the installed browser on the Mixamo profile (persistent, never deleted).
 * guard: true limits page requests to ALLOWED_HOST_SUFFIXES. trace: log API calls.
 */
export async function launchBrowser({ headless, guard, trace = false, profile = profilePath(), width = 1280, height = 900 }) {
  const exe = findBrowserExe();
  mkdirSync(profile, { recursive: true });
  const portFile = join(profile, "DevToolsActivePort");
  rmSync(portFile, { force: true }); // a file left by an earlier run points at a dead port
  const flags = [
    "--remote-debugging-port=0",
    `--user-data-dir=${profile}`,
    "--no-first-run",
    "--no-default-browser-check",
    `--window-size=${width},${height}`,
  ];
  if (headless) flags.push("--headless=new");
  flags.push("about:blank");

  const child = spawn(exe, flags, { stdio: "ignore", windowsHide: true });
  let exitCode = null;
  child.on("exit", (code) => { exitCode = code; });

  const end = Date.now() + 30000;
  let port = null, wsPath = null;
  while (Date.now() < end) {
    if (existsSync(portFile)) {
      try {
        const [p, w] = readFileSync(portFile, "utf8").split(/\r?\n/).filter(Boolean);
        if (p && w) { port = p; wsPath = w; break; }
      } catch { /* the browser is still writing it */ }
    }
    if (exitCode !== null && exitCode !== 0) throw new Error(`browser exited (${exitCode}) before it opened the debug port`);
    await sleep(100);
  }
  if (!port) throw new Error(`no DevToolsActivePort in the profile after 30 s; is another browser open on ${profile}?`);

  const conn = await Cdp.open(`ws://127.0.0.1:${port}${wsPath}`);
  const { targetId } = await conn.send("Target.createTarget", { url: "about:blank" });
  const { sessionId } = await conn.send("Target.attachToTarget", { targetId, flatten: true });
  const blocked = [];
  if (guard) {
    await conn.send("Fetch.enable", { patterns: [{ urlPattern: "*" }] }, sessionId);
  }
  if (trace) {
    await conn.send("Network.enable", {}, sessionId);
  }
  conn.handlers.push((m) => {
    if (m.sessionId !== sessionId) return;
    if (m.method === "Fetch.requestPaused") {
      const { requestId, request } = m.params;
      if (hostAllowed(request.url)) {
        conn.send("Fetch.continueRequest", { requestId }, sessionId).catch(() => {});
      } else {
        if (blocked.length < 200) blocked.push(new URL(request.url).host);
        conn.send("Fetch.failRequest", { requestId, errorReason: "BlockedByClient" }, sessionId).catch(() => {});
      }
    }
    if (trace && m.method === "Network.requestWillBeSent" && /\/api\//.test(m.params.request.url)) {
      const u = new URL(m.params.request.url);
      console.error(`> ${m.params.request.method} ${u.pathname}`);
    }
    if (trace && m.method === "Network.responseReceived" && /\/api\//.test(m.params.response.url)) {
      const u = new URL(m.params.response.url);
      console.error(`< ${m.params.response.status} ${u.pathname}`);
    }
  });
  await conn.send("Page.enable", {}, sessionId);

  return {
    blocked,
    /** Run a page script that returns a JSON-able value (promises awaited). */
    async evaluate(fn, ...args) {
      const expression = `(${fn.toString()})(${args.map((a) => JSON.stringify(a)).join(", ")})`;
      const r = await conn.send("Runtime.evaluate", { expression, returnByValue: true, awaitPromise: true }, sessionId, 60000);
      if (r.exceptionDetails) throw new Error(`page script failed: ${r.exceptionDetails.exception?.description?.split("\n")[0] || "error"}`);
      return r.result?.value;
    },
    async navigate(url) {
      await conn.send("Page.navigate", { url }, sessionId, 30000);
      await this.waitUntil(() => document.readyState === "complete", 60000);
    },
    async waitUntil(fn, timeout) {
      const end = Date.now() + timeout;
      while (Date.now() < end) {
        if (await this.evaluate(fn)) return;
        await sleep(300);
      }
      throw new Error(`page did not reach the expected state within ${timeout} ms`);
    },
    /**
     * Browser.close, then wait for the CDP socket to drop (the browser has exited and
     * flushed the profile). Never kills. Edge leaves DevToolsActivePort behind, so the
     * socket is the signal, not the file.
     */
    async close(timeout = 60000) {
      const dropped = new Promise((r) => conn.ws.addEventListener("close", r, { once: true }));
      await conn.send("Browser.close", {}, undefined, 10000).catch(() => {});
      const gone = await Promise.race([dropped.then(() => true), sleep(timeout).then(() => false)]);
      if (!gone) console.error("browser did not exit within 60 s of Browser.close; leaving it alone");
      try { conn.ws.close(); } catch { /* already closed */ }
    },
  };
}

// Runs inside the page. Returns only booleans and counts, never page data.
function pageState() {
  const els = Array.from(document.querySelectorAll("a, button"));
  const login = els.some((e) => /^\s*(log\s*in|sign\s*in)\s*$/i.test(e.textContent || "") && e.getClientRects().length > 0);
  const upload = Array.from(document.querySelectorAll("a, button, span, div")).some((e) => /^\s*upload character\s*$/i.test(e.textContent || "") && e.getClientRects().length > 0);
  const textLen = ((document.body && document.body.innerText) || "").length;
  return { ready: document.readyState === "complete", login, upload, textLen };
}

// Runs inside the page with the page's own session. Returns { status, body }; the token never leaves the page.
async function pageApi(path, method, body) {
  const headers = { Accept: "application/json", "X-Api-Key": "mixamo2" };
  if (body) headers["Content-Type"] = "application/json";
  const m = /(?:^|;\s*)access_token=([^;]+)/.exec(document.cookie || "");
  if (m) headers.Authorization = "Bearer " + decodeURIComponent(m[1]);
  const res = await fetch(path, { method, headers, credentials: "include", body: body ? JSON.stringify(body) : undefined });
  let json = null;
  try { json = await res.json(); } catch { /* not JSON */ }
  return { status: res.status, body: json };
}

/** 'in' when the page shows the signed-in state, 'out' when the Log in link is there, else 'unknown'. */
async function signedInState(b) {
  const st = await b.evaluate(pageState);
  if (!st.ready) return "unknown";
  if (st.login) return "out";
  if (st.textLen < 200) return "unknown";
  const probe = await b.evaluate(pageApi, productsPath("", 1), "GET", null).catch(() => ({ status: 0 }));
  return st.upload && probe.status === 200 ? "in" : "unknown";
}

async function waitSignedIn(b, timeout) {
  const end = Date.now() + timeout;
  while (Date.now() < end) {
    const s = await signedInState(b);
    if (s === "in") return true;
    if (s === "out") return false;
    await sleep(1000);
  }
  return false;
}

// ------------------------------------------------------------------ commands

/** Download the zip to `part`, resuming a partial file when the server honours the Range header. */
async function download(url, part) {
  const have = existsSync(part) ? statSync(part).size : 0;
  const res = await fetch(url, { headers: have ? { Range: `bytes=${have}-` } : {} });
  if (!res.ok) throw new Error(`download HTTP ${res.status}`);
  const append = have > 0 && res.status === 206;
  await pipeline(Readable.fromWeb(res.body), createWriteStream(part, { flags: append ? "a" : "w" }));
}

function unzip(zip, dest) {
  return new Promise((resolveUnzip, reject) => {
    execFile("tar", ["-xf", zip, "-C", dest], { windowsHide: true }, (err) => {
      if (err) reject(new Error(`unzip failed: ${err.message.split("\n")[0]}`));
      else resolveUnzip();
    });
  });
}

async function runLogin(log) {
  const b = await launchBrowser({ headless: false, guard: false });
  try {
    await b.navigate(SITE_URL);
    const end = Date.now() + LOGIN_TIMEOUT_MS;
    while (Date.now() < end) {
      if ((await signedInState(b)) === "in") {
        log("signed in, profile saved");
        return EXIT.ok;
      }
      await sleep(LOGIN_POLL_MS);
    }
    console.error("not signed in after 10 minutes; run --login again");
    return EXIT.error;
  } finally {
    await b.close();
  }
}

/** Headless browser on the profile, checked for the signed-in state. The caller closes b when ok. */
async function openSignedIn(o) {
  const b = await launchBrowser({ headless: true, guard: true, trace: o.trace });
  try {
    await b.navigate(SITE_URL);
    if (await waitSignedIn(b, 25000)) return { b, ok: true };
  } catch (e) {
    await b.close();
    throw e;
  }
  await b.close();
  return { b: null, ok: false };
}

async function runList(o, log) {
  const { b, ok } = await openSignedIn(o);
  if (!ok) {
    console.error("not signed in to Mixamo in this profile: run --login first");
    return EXIT.notSignedIn;
  }
  try {
    const r = await withRetry(async () => {
      const res = await b.evaluate(pageApi, productsPath(o.query), "GET", null);
      if (res.status !== 200) throw new Error(`product search failed: HTTP ${res.status}`);
      return res;
    }, { onRetry: (e, n) => console.error(`retry ${n}/${RETRIES}: ${e.message}`) });
    const packs = parseProducts(r.body).filter((p) => !p.type || /MotionPack/.test(p.type));
    if (packs.length === 0) {
      log(`no packs match "${o.query}"`);
      return EXIT.ok;
    }
    for (const p of packs) {
      const count = p.animations === null ? "?" : String(p.animations);
      log(`${count.padStart(4)}  ${p.name}`);
    }
    return EXIT.ok;
  } finally {
    await b.close();
  }
}

async function runPack(o, log) {
  const slug = packSlug(o.pack);
  const dir = join(o.out, slug);
  if (o.dryRun) {
    const folder = folderStatus(dir, recordedAnimations(dir));
    for (const line of planLines({ pack: o.pack, slug, out: o.out, fps: o.fps, character: o.character, folder })) log(line);
    return EXIT.ok;
  }

  const { b, ok } = await openSignedIn(o);
  if (!ok) {
    console.error("not signed in to Mixamo in this profile: run --login first");
    return EXIT.notSignedIn;
  }
  try {
    const api = (path, method, body) => withRetry(async () => {
      const res = await b.evaluate(pageApi, path, method, body);
      if (res.status >= 500 || res.status === 429) throw new Error(`${method} ${path.split("?")[0]} HTTP ${res.status}`);
      return res;
    }, { onRetry: (e, n) => console.error(`retry ${n}/${RETRIES}: ${e.message}`) });

    const search = await api(productsPath(o.pack, 24), "GET", null);
    if (search.status !== 200) throw new Error(`product search failed: HTTP ${search.status}`);
    const pack = findPack(parseProducts(search.body).filter((p) => !p.type || /MotionPack/.test(p.type)), o.pack);
    const animations = pack.animations ?? recordedAnimations(dir);

    const before = folderStatus(dir, animations);
    if (before.complete) {
      log(`already complete: ${dir} (${before.fbx} FBX)`);
      return EXIT.ok;
    }

    let character = o.character;
    if (!character) {
      const chars = await api("/api/v1/products?page=1&limit=1&type=Character", "GET", null);
      character = parseProducts(chars.body)[0]?.id || null;
      if (!character) throw new Error("no character found on the account; pass --character <id>");
    }

    const exp = await api("/api/v1/animations/export", "POST", exportBody({ pack: pack.name, character, fps: o.fps }));
    if (exp.status !== 200 && exp.status !== 201 && exp.status !== 202) throw new Error(`export request failed: HTTP ${exp.status}`);

    const zipUrl = await waitForExport(b, character);
    mkdirSync(dir, { recursive: true });
    const part = join(dir, "_pack.zip.part");
    const zip = join(dir, "_pack.zip");
    await withRetry(() => download(zipUrl, part), { onRetry: (e, n) => console.error(`retry ${n}/${RETRIES}: ${e.message}`) });
    renameSync(part, zip);
    await unzip(zip, dir);
    rmSync(zip, { force: true });

    const fbx = countFbx(dir);
    const count = animations ?? Math.max(fbx - 1, 0);
    writeFileSync(join(dir, "PROVENANCE.json"), provenanceJson(provenanceRecord({
      pack: pack.name, slug, animations: count, fps: o.fps, character, fetchedAt: new Date().toISOString(),
    })));
    log(`fetched "${pack.name}": ${fbx} FBX in ${dir}`);
    return EXIT.ok;
  } finally {
    await b.close();
  }
}

/** Poll the character's monitor until the export is ready; returns the zip URL. */
async function waitForExport(b, character) {
  const path = `/api/v1/characters/${encodeURIComponent(character)}/monitor`;
  const end = Date.now() + EXPORT_TIMEOUT_MS;
  while (Date.now() < end) {
    const res = await b.evaluate(pageApi, path, "GET", null);
    const body = res.body || {};
    if (body.status === "failed") throw new Error("Mixamo reported the export failed");
    const url = body.job_result || body.url;
    if (res.status === 200 && body.status === "completed" && typeof url === "string") return url;
    await sleep(EXPORT_POLL_MS);
  }
  throw new Error("export not ready after 10 minutes");
}

// ------------------------------------------------------------------ entry

export async function main(argv = process.argv.slice(2), log = console.log) {
  let o;
  try {
    o = parseArgs(argv);
  } catch (e) {
    if (argv.includes("--help") || argv.includes("-h")) { log(USAGE); return EXIT.ok; }
    console.error(`mixamo: ${e.message}\n${USAGE}`);
    return EXIT.usage;
  }
  if (o.help) { log(USAGE); return EXIT.ok; }
  try {
    if (o.command === "login") return await runLogin(log);
    if (o.command === "list") return await runList(o, log);
    if (o.command === "pack") return await runPack(o, log);
    return EXIT.usage;
  } catch (e) {
    console.error(`mixamo: ${e.message}`);
    return EXIT.error;
  }
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main().then((code) => { process.exitCode = code; });
}
