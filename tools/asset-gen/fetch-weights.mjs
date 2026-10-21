// Resilient weights fetch for the one local 3D lane.
//
// Survives an internet restart: the file is resumed with an HTTP Range, every
// attempt is logged to a state file, a DNS failure pins the host in the
// Windows hosts file and retries, and re-running the script continues from
// whatever is already on disk. Three sources are tried in order.
//
//   node tools/asset-gen/fetch-weights.mjs
import { createWriteStream, existsSync, mkdirSync, readFileSync, renameSync, statSync, writeFileSync, appendFileSync, openSync, closeSync, unlinkSync } from "node:fs";
import { pipeline } from "node:stream/promises";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { execFileSync } from "node:child_process";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
const DEST = process.env.SF3D_WEIGHTS || "C:\\forge-media\\models\\sf3d-weights";
const STATE = join(DEST, "fetch-state.json");
const HOSTS = "C:\\Windows\\System32\\drivers\\etc\\hosts";
const TOKEN_FILE = join(process.env.USERPROFILE || "", ".cache", "huggingface", "token");

const TARGETS = [
  { name: "config.yaml", size: 2405 },
  { name: "model.safetensors", size: 4024289892 },
];
const SOURCES = [
  { id: "huggingface", base: "https://huggingface.co/stabilityai/stable-fast-3d/resolve/main", auth: true },
  { id: "hf-mirror", base: "https://hf-mirror.com/stabilityai/stable-fast-3d/resolve/main", auth: false },
  { id: "modelscope", base: "https://www.modelscope.cn/models/stabilityai/stable-fast-3d/resolve/master", auth: false },
];

mkdirSync(DEST, { recursive: true });
const token = existsSync(TOKEN_FILE) ? readFileSync(TOKEN_FILE, "utf8").trim() : null;
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

// One writer at a time: the scheduler, the watcher and a hand-run command can
// all reach this script, and two writers on one .part file would corrupt it.
const LOCK = join(DEST, "fetch.lock");
function alive(pid) {
  try { process.kill(pid, 0); return true; } catch (err) { return err.code === "EPERM"; }
}
function takeLock() {
  try {
    closeSync(openSync(LOCK, "wx"));
    writeFileSync(LOCK, String(process.pid));
    return true;
  } catch {
    let pid = 0;
    try { pid = Number(readFileSync(LOCK, "utf8").trim()); } catch { pid = 0; }
    if (pid && alive(pid)) return false;
    try { unlinkSync(LOCK); } catch { /* the stale lock goes anyway */ }
    closeSync(openSync(LOCK, "wx"));
    writeFileSync(LOCK, String(process.pid));
    return true;
  }
}
function releaseLock() {
  try {
    const pid = Number(readFileSync(LOCK, "utf8").trim());
    if (pid === process.pid) unlinkSync(LOCK);
  } catch { /* nothing to release */ }
}
function reason(err) {
  const code = err?.cause?.code || err?.code || "";
  const msg = err?.cause?.message || err?.message || "unknown";
  return code ? `${code} ${msg}` : msg;
}

function loadState() {
  if (!existsSync(STATE)) return {};
  try { return JSON.parse(readFileSync(STATE, "utf8")); } catch { return {}; }
}
function saveState(s) {
  writeFileSync(STATE, JSON.stringify(s, null, 2) + "\n", "utf8");
}
function log(line) {
  console.log(line);
  try { appendFileSync(join(DEST, "fetch.log"), `${new Date().toISOString()} ${line}\n`); } catch { /* log is best effort */ }
}

// Pin a host in the Windows hosts file after a DNS failure. We are admin on
// this box; the entry is marked so it can be removed later.
function pinHost(host, ip) {
  try {
    const cur = existsSync(HOSTS) ? readFileSync(HOSTS, "utf8") : "";
    if (cur.includes(`# empire-pin ${host}`)) {
      const out = cur.split(/\r?\n/).map((l) => (l.includes(`# empire-pin ${host}`) ? `# empire-pin ${host} ${ip}` : l)).join("\n");
      writeFileSync(HOSTS, out);
    } else {
      appendFileSync(HOSTS, `\n# empire-pin ${host} ${ip}\n`);
    }
    execFileSync("ipconfig", ["/flushdns"], { stdio: "ignore" });
    log(`pinned ${host} -> ${ip}`);
    return true;
  } catch (err) {
    log(`pin failed for ${host}: ${err.message}`);
    return false;
  }
}

async function resolveViaDoh(host) {
  for (const endpoint of [
    `https://dns.google/resolve?name=${host}&type=A`,
    `https://cloudflare-dns.com/dns-query?name=${host}&type=A`,
  ]) {
    try {
      const res = await fetch(endpoint, { headers: { accept: "application/dns-json" } });
      const j = await res.json();
      const ip = (j.Answer || []).find((a) => a.type === 1)?.data;
      if (ip) return ip;
    } catch { /* try the next resolver */ }
  }
  return null;
}

async function download(source, target) {
  const url = `${source.base}/${target.name}`;
  const part = join(DEST, `${target.name}.part`);
  const have = existsSync(part) ? statSync(part).size : 0;
  if (have === target.size) return true;
  const headers = {};
  if (have > 0) headers.Range = `bytes=${have}-`;
  if (source.auth && token) headers.Authorization = `Bearer ${token}`;

  const res = await fetch(url, { headers, redirect: "follow" });
  if (have > 0 && res.status !== 206) {
    // The server ignored the range: start the file over rather than corrupt it.
    if (res.status === 200) {
      const body = await res.arrayBuffer();
      writeFileSync(part, Buffer.from(body));
      return statSync(part).size === target.size;
    }
    throw new Error(`http ${res.status}`);
  }
  if (!res.ok) throw new Error(`http ${res.status}`);
  await pipeline(res.body, createWriteStream(part, { flags: have > 0 ? "a" : "w" }));
  const size = statSync(part).size;
  if (size !== target.size) throw new Error(`size ${size} of ${target.size}`);
  renameSync(part, join(DEST, target.name));
  return true;
}

async function main() {
  if (!takeLock()) {
    log("another fetcher holds the lock, leaving it alone");
    process.exit(0);
  }
  process.on("exit", releaseLock);
  const state = loadState();
  for (const target of TARGETS) {
    const finalPath = join(DEST, target.name);
    if (existsSync(finalPath) && statSync(finalPath).size === target.size) {
      log(`${target.name} already complete`);
      continue;
    }
    let attempt = 0;
    let lastError = "none";
    while (true) {
      attempt += 1;
      for (const source of SOURCES) {
        try {
          const ok = await download(source, target);
          if (ok) {
            log(`${target.name} done from ${source.id} in ${attempt} attempt(s)`);
            state[target.name] = { done: true, bytes: target.size, attempts: attempt, source: source.id };
            saveState(state);
            lastError = "none";
            break;
          }
        } catch (err) {
          lastError = reason(err);
          const dnsFail = /ENOTFOUND|EAI_AGAIN|getaddrinfo|NameResolution/i.test(lastError);
          if (dnsFail) {
            // Pin whichever host failed, not just the source host: the file
            // redirects to a CDN name, and that is the one that blinks.
            const hosts = new Set([new URL(`${source.base}/${target.name}`).hostname]);
            for (const m of lastError.matchAll(/([a-z0-9-]+\.[a-z0-9-]+\.[a-z]{2,})/g)) hosts.add(m[1]);
            for (const host of hosts) {
              const ip = await resolveViaDoh(host);
              if (ip) await pinHost(host, ip);
            }
          }
          log(`${target.name} via ${source.id} failed: ${lastError}`);
        }
      }
      if (state[target.name]?.done) break;
      const backoff = Math.min(60, 2 ** Math.min(attempt, 5));
      state[target.name] = { done: false, bytes: existsSync(join(DEST, `${target.name}.part`)) ? statSync(join(DEST, `${target.name}.part`)).size : 0, attempts: attempt, lastError };
      saveState(state);
      log(`retrying in ${backoff}s (attempt ${attempt}, last: ${lastError})`);
      await sleep(backoff * 1000);
    }
  }
  log("ALL WEIGHTS COMPLETE");
}

main().catch((err) => { log(`FATAL ${reason(err)}`); process.exit(1); });
