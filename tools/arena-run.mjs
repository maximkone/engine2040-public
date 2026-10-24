// arena-run: run the arena window, one at a time on this PC.
//
// Owner 2026-10-24 ("4 ARENAS LAUNCHED WTH"): agents each opened their own arena.exe capture run
// at once, which buried the desktop and made every frame-time number worthless. Every agent
// window run goes through here:
//
//   node tools/arena-run.mjs [--wait-s 1800] [--exe <saved arena.exe>] -- <arena.exe arguments>
//
// It takes `target/arena-window.lock` (an atomic create), waits while another run holds it,
// clears a lock whose owner process is gone or that is older than 15 minutes, runs
// `target/release/arena.exe` above normal priority with stdio passed through, and releases the
// lock when the window exits. Exit code: the window's own, or 2 when it could not start.
// `--exe <path>` runs a saved copy of the binary instead (a before/after pair taken under one lock).
import { spawn } from "node:child_process";
import { closeSync, existsSync, mkdirSync, openSync, readFileSync, rmSync, statSync, writeSync } from "node:fs";
import { constants as osConstants, setPriority } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
export const LOCK = join(ROOT, "target", "arena-window.lock");
export const STALE_MS = 15 * 60 * 1000;

const alive = (pid) => {
  try {
    process.kill(pid, 0);
    return true;
  } catch (err) {
    return err.code === "EPERM";
  }
};

// True when a lock file may be taken over: its owner is gone, or it is older than STALE_MS.
export const isStale = (lock = LOCK, now = Date.now()) => {
  if (!existsSync(lock)) return true;
  let pid = 0;
  try {
    pid = Number(JSON.parse(readFileSync(lock, "utf8")).pid) || 0;
  } catch {
    pid = 0;
  }
  const age = now - statSync(lock).mtimeMs;
  return age > STALE_MS || (pid > 0 && !alive(pid));
};

// Take the lock once: true when this process holds it now.
export const tryLock = (lock = LOCK, info = { pid: process.pid, at: new Date().toISOString() }) => {
  mkdirSync(dirname(lock), { recursive: true });
  try {
    const fd = openSync(lock, "wx");
    writeSync(fd, JSON.stringify(info));
    closeSync(fd);
    return true;
  } catch (err) {
    if (err.code !== "EEXIST") throw err;
    if (isStale(lock)) {
      rmSync(lock, { force: true });
      return tryLock(lock, info);
    }
    return false;
  }
};

export const release = (lock = LOCK) => rmSync(lock, { force: true });

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

export const main = async (argv) => {
  const dash = argv.indexOf("--");
  const own = dash >= 0 ? argv.slice(0, dash) : argv;
  const args = dash >= 0 ? argv.slice(dash + 1) : [];
  const waitAt = own.indexOf("--wait-s");
  const waitS = waitAt >= 0 ? Number(own[waitAt + 1]) || 1800 : 1800;
  const exeAt = own.indexOf("--exe");
  const exe = exeAt >= 0 && own[exeAt + 1] ? resolve(own[exeAt + 1]) : join(ROOT, "target", "release", "arena.exe");
  if (!existsSync(exe)) {
    console.log(`arena-run: ${exe} is missing; build it: cargo build --release -p engine-game --features app --bin arena`);
    return 2;
  }
  const deadline = Date.now() + waitS * 1000;
  let said = false;
  while (!tryLock()) {
    if (!said) console.log("arena-run: another arena window is running; waiting for it (one window at a time on this PC)");
    said = true;
    if (Date.now() > deadline) {
      console.log(`arena-run: still busy after ${waitS} s; nothing started`);
      return 2;
    }
    await sleep(5000);
  }
  try {
    const child = spawn(exe, args, { cwd: ROOT, stdio: "inherit" });
    try {
      setPriority(child.pid, osConstants.priority.PRIORITY_ABOVE_NORMAL);
    } catch {
      // A lower-privilege shell may refuse; the window still runs.
    }
    return await new Promise((resolve) => {
      child.on("exit", (code) => resolve(code ?? 1));
      child.on("error", (err) => {
        console.log(`arena-run: ${err.message}`);
        resolve(2);
      });
    });
  } finally {
    release();
  }
};

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) main(process.argv.slice(2)).then((c) => process.exit(c));
