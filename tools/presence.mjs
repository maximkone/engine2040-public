// Presence awareness: no sprint lock is ever needed.
//
// Every session announces itself with a heartbeat file under
// target/presence/<session>.json. Every reader lists who is active.
// No call blocks or waits: heartbeat overwrites one file, list reads
// directory entries. Stale files (no refresh for STALE_MS) read as STALE
// but never refuse a new writer.
//
//   node tools/presence.mjs heartbeat --session <id> [--wave <w>] [--files a,b] [--pid N] [--dir <d>]
//   node tools/presence.mjs list [--dir <d>]
//   node tools/presence.mjs --selftest
//
// Dir defaults to target/presence under the current working directory.
// Files are JSON: { session, pid, wave, files, heartbeat }.
import { existsSync, mkdirSync, readdirSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

// Heartbeats older than this read as STALE (shown, never blocking).
// Shorter than the old sprint lock (tools/sprint-lock.mjs STALE_MS = 1 h)
// so a closed session fades fast while a live one refreshes each wave.
export const STALE_MS = 5 * 60 * 1000;

const option = (args, key, fallback) => {
  const i = args.indexOf(key);
  return i === -1 ? fallback : (args[i + 1] ?? fallback);
};

const safeSession = (s) => /^[a-zA-Z0-9][a-zA-Z0-9-_]*$/.test(s ?? "");

export function heartbeat({ dir, session, pid, wave, files }) {
  if (!safeSession(session)) throw new Error(`bad --session "${session ?? ""}" (use [a-zA-Z0-9-_])`);
  mkdirSync(dir, { recursive: true });
  const record = {
    session,
    pid: Number.isFinite(Number(pid)) ? Number(pid) : process.pid,
    wave: wave ?? "",
    files: Array.isArray(files) ? files : String(files ?? "").split(",").map((s) => s.trim()).filter(Boolean),
    heartbeat: new Date().toISOString(),
  };
  writeFileSync(join(dir, `${session}.json`), `${JSON.stringify(record, null, 2)}\n`);
  return record;
}

export function list(dir) {
  let names = [];
  try {
    names = readdirSync(dir).filter((f) => f.endsWith(".json"));
  } catch {
    return [];
  }
  const now = Date.now();
  const rows = [];
  for (const name of names) {
    try {
      const raw = readFileSync(join(dir, name), "utf8");
      const rec = JSON.parse(raw);
      if (!rec || typeof rec.session !== "string") continue;
      const at = Date.parse(rec.heartbeat ?? "");
      const ageMs = Number.isFinite(at) ? now - at : NaN;
      rows.push({ ...rec, ageMs, stale: !Number.isFinite(ageMs) || ageMs > STALE_MS });
    } catch {
      // One unreadable heartbeat never breaks the list.
    }
  }
  rows.sort((a, b) => String(a.session).localeCompare(String(b.session)));
  return rows;
}

function printList(rows) {
  if (rows.length === 0) {
    console.log("(no presence: nobody announced yet)");
    return;
  }
  for (const r of rows) {
    const age = Number.isFinite(r.ageMs) ? `${Math.round(r.ageMs / 1000)}s ago` : "unknown age";
    const files = (r.files ?? []).join(",") || "-";
    console.log(`${r.session} pid=${r.pid} wave=${r.wave || "-"} files=${files} ${age}${r.stale ? " STALE" : ""}`);
  }
}

function selftest() {
  // Round-trip in an isolated temp dir: never touches the real
  // target/presence, never creates target/sprint.lock.
  const dir = join(resolve(tmpdir()), `presence-selftest-${Date.now()}-${process.pid}`);
  mkdirSync(dir, { recursive: true });
  const session = `selftest-${process.pid}`;
  heartbeat({ dir, session, pid: process.pid, wave: "selftest", files: ["tools/presence.mjs"] });
  const rows = list(dir);
  const found = rows.find((r) => r.session === session);
  if (!found) {
    console.error("SELFTEST FAIL: heartbeat not listed");
    process.exit(1);
  }
  if (found.stale) {
    console.error("SELFTEST FAIL: fresh heartbeat reads STALE");
    process.exit(1);
  }
  if (existsSync(resolve("target/sprint.lock"))) {
    console.error("SELFTEST FAIL: target/sprint.lock exists (presence must never create a lock)");
    process.exit(1);
  }
  console.log(`heartbeat ${session} -> ${join(dir, `${session}.json`)}`);
  console.log(`list shows ${found.session} pid=${found.pid}`);
  console.log("SELFTEST PASS");
}

function main() {
  const args = process.argv.slice(2);
  if (args.includes("--selftest")) {
    selftest();
    return;
  }
  const [command, ...rest] = args;
  const dir = resolve(option(rest, "--dir", "target/presence"));
  if (command === "heartbeat") {
    const session = option(rest, "--session", "");
    const wave = option(rest, "--wave", "");
    const files = option(rest, "--files", "");
    const pid = option(rest, "--pid", String(process.pid));
    try {
      const rec = heartbeat({ dir, session, pid, wave, files });
      console.log(`HEARTBEAT ${rec.session} -> ${join(dir, `${rec.session}.json`)}`);
    } catch (err) {
      console.error(err?.message ?? String(err));
      process.exit(2);
    }
  } else if (command === "list") {
    printList(list(dir));
  } else {
    console.error("Usage: node tools/presence.mjs heartbeat --session <id> [--wave <w>] [--files a,b] [--pid N] [--dir <d>]");
    console.error("       node tools/presence.mjs list [--dir <d>]");
    console.error("       node tools/presence.mjs --selftest");
    process.exit(2);
  }
}

if (import.meta.main) {
  main();
}
