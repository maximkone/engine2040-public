// Quiet window runner: hold nonstop between rows, wait for the box to go
// quiet, then run one timing command. Nonstop checks target/nonstop/STOP only
// between rows (tools/nonstop.mjs:188), so this waits for its PID instead of
// killing an agent mid-row. STOP and the lock live under target/ (gitignored).
//
// Usage:
//   node tools/quiet-run.mjs [--dry] [--times N] [--max-wait MIN] [--cpu PCT]
//                            [--samples N] [--gap SEC] -- <cmd> [args...]
// Exit: command exit; 2 refused/busy probe; 3 no quiet window; 4 STOP/lock busy.
import { spawnSync } from "node:child_process";
import { cpus } from "node:os";
import {
  existsSync,
  mkdirSync,
  readFileSync,
  renameSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const STOP_TTL_MS = 3 * 60 * 60_000;

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

export function pidAlive(pid) {
  try {
    process.kill(Number(pid), 0);
    return true;
  } catch (error) {
    return error?.code === "EPERM";
  }
}

function readJson(path) {
  try {
    return JSON.parse(readFileSync(path, "utf8"));
  } catch {
    return null;
  }
}

// live: another quiet-run owns STOP; stale: a crashed quiet-run left it;
// foreign: a hand-made STOP, which is never deleted.
export function parseStop(text, alive = pidAlive, now = Date.now()) {
  let parsed = null;
  try {
    parsed = JSON.parse(text);
  } catch {
    return { state: "foreign" };
  }
  if (!parsed || !Number.isInteger(parsed.pid)) return { state: "foreign" };
  if (!alive(parsed.pid)) return { state: "stale", pid: parsed.pid, why: "pid gone" };
  if (now - Number(parsed.at ?? 0) > STOP_TTL_MS) {
    return { state: "stale", pid: parsed.pid, why: "TTL" };
  }
  return { state: "live", pid: parsed.pid };
}

export function quietVerdict({ cpu = [], nonstop = [], cargo = 0, rustc = 0, arena = 0, cpuMax = 10 }) {
  const reasons = [];
  if (nonstop.length) reasons.push(`nonstop pid ${nonstop.join(",")}`);
  if (cargo) reasons.push(`${cargo} cargo`);
  if (rustc) reasons.push(`${rustc} rustc`);
  if (arena) reasons.push(`${arena} arena`);
  const max = cpu.length ? Math.max(...cpu) : 100;
  if (!(max < cpuMax)) reasons.push(`cpu max ${max.toFixed(1)}% >= ${cpuMax}%`);
  return reasons.length
    ? `LOADED ${reasons.join(", ")}`
    : `QUIET cpu max ${max.toFixed(1)}% < ${cpuMax}%, no nonstop/cargo/rustc/arena`;
}

export function parseNonstopPids(text) {
  return String(text ?? "")
    .split(/\r?\n/)
    .map((line) => Number(line.trim()))
    .filter((pid) => Number.isInteger(pid) && pid > 0);
}

// PowerShell ConvertTo-Json gives one object for one row and an array for many.
export function parseProcs(text) {
  const raw = typeof text === "string" ? JSON.parse(text) : text;
  const rows = Array.isArray(raw) ? raw : raw ? [raw] : [];
  const out = { nonstop: [], cargo: 0, rustc: 0, arena: 0 };
  for (const row of rows) {
    const name = String(row?.Name ?? row?.ProcessName ?? "").toLowerCase();
    const cmd = String(row?.CommandLine ?? "");
    if (name === "node.exe" || name === "node") {
      if (/nonstop\.mjs/i.test(cmd)) {
        const pid = Number(row?.ProcessId ?? row?.Id);
        if (Number.isInteger(pid) && pid > 0) out.nonstop.push(pid);
      }
    } else if (name === "cargo.exe" || name === "cargo") {
      out.cargo += 1;
    } else if (name === "rustc.exe" || name === "rustc") {
      out.rustc += 1;
    } else if (name === "arena.exe" || name === "arena") {
      out.arena += 1;
    }
  }
  return out;
}

const PROCESS_QUERY =
  "$ErrorActionPreference='SilentlyContinue'; " +
  "Get-CimInstance Win32_Process -Filter \"Name='node.exe' OR Name='cargo.exe' OR Name='rustc.exe' OR Name='arena.exe'\" " +
  "| Select-Object ProcessId,Name,CommandLine | ConvertTo-Json -Compress";

// null means the probe failed: callers must refuse, never treat it as quiet.
export function scanProcesses(runner = spawnSync) {
  if (process.platform !== "win32") return null;
  const result = runner(
    "powershell",
    ["-NoProfile", "-NonInteractive", "-Command", PROCESS_QUERY],
    { encoding: "utf8", timeout: 20_000, windowsHide: true, maxBuffer: 4 * 1024 * 1024 },
  );
  if (result.error || result.status !== 0) return null;
  const text = String(result.stdout ?? "").trim();
  if (!text) return { nonstop: [], cargo: 0, rustc: 0, arena: 0 };
  try {
    return parseProcs(text);
  } catch {
    return null;
  }
}

export function cpuTimes() {
  const rows = cpus().map((cpu) => cpu.times);
  return {
    total: rows.reduce((sum, t) => sum + t.user + t.nice + t.sys + t.idle + t.irq, 0),
    idle: rows.reduce((sum, t) => sum + t.idle, 0),
  };
}

export async function sampleCpu({ n = 3, gapMs = 10_000, read = cpuTimes, wait = sleep } = {}) {
  const out = [];
  let prev = read();
  for (let i = 0; i < n; i++) {
    await wait(gapMs);
    const cur = read();
    const total = cur.total - prev.total;
    const idle = cur.idle - prev.idle;
    const pct = total > 0 ? Math.max(0, Math.min(100, 100 * (1 - idle / total))) : 100;
    out.push(pct);
    prev = cur;
  }
  return out;
}

export function parseArgs(argv) {
  const cut = argv.indexOf("--");
  if (cut < 0 || cut === argv.length - 1) return null;
  const opts = {
    dry: false,
    times: 1,
    maxWaitMin: 45,
    cpuMax: 10,
    samples: 3,
    gap: 10,
    cmd: argv[cut + 1],
    args: argv.slice(cut + 2),
  };
  const head = argv.slice(0, cut);
  for (let i = 0; i < head.length; i++) {
    const flag = head[i];
    const next = () => {
      const value = Number(head[++i]);
      if (!Number.isFinite(value)) throw new Error(`${flag} needs a number`);
      return value;
    };
    try {
      if (flag === "--dry") opts.dry = true;
      else if (flag === "--times") opts.times = Math.trunc(next());
      else if (flag === "--max-wait") opts.maxWaitMin = next();
      else if (flag === "--cpu") opts.cpuMax = next();
      else if (flag === "--samples") opts.samples = Math.trunc(next());
      else if (flag === "--gap") opts.gap = next();
      else return null;
    } catch {
      return null;
    }
  }
  if (opts.times < 1 || opts.maxWaitMin < 0 || opts.cpuMax <= 0 || opts.cpuMax > 100) return null;
  if (opts.samples < 1 || opts.gap < 1) return null;
  return opts;
}

function writeAtomic(file, body, deps) {
  const tmp = `${file}.${process.pid}.${Math.random().toString(16).slice(2)}.tmp`;
  writeFileSync(tmp, JSON.stringify(body));
  renameSync(tmp, file);
  deps?.written?.();
}

function claimStop(out, deps) {
  const file = join(out, "STOP");
  if (existsSync(file)) {
    const stop = parseStop(readFileSync(file, "utf8"), deps.alive ?? pidAlive, deps.now?.() ?? Date.now());
    if (stop.state === "live") return { ok: false, why: `STOP held by pid ${stop.pid}` };
    if (stop.state === "foreign") {
      return { ok: false, why: `STOP exists and is not quiet-run's; remove ${file} if stale` };
    }
    rmSync(file, { force: true });
  }
  writeAtomic(file, { pid: process.pid, at: Date.now(), by: "quiet-run" }, deps);
  return { ok: true };
}

function claimLock(out, deps) {
  const file = join(out, "quiet.lock");
  if (existsSync(file)) {
    const lock = readJson(file);
    if (lock && Number.isInteger(lock.pid) && (deps.alive ?? pidAlive)(lock.pid)) {
      return { ok: false, why: `quiet.lock held by pid ${lock.pid}` };
    }
    rmSync(file, { force: true });
  }
  writeAtomic(file, { pid: process.pid, at: Date.now(), by: "quiet-run" });
  return { ok: true };
}

export async function runQuiet(argv, deps = {}) {
  const opts = parseArgs(argv);
  if (!opts) {
    console.error("quiet-run: bad arguments; see the header for usage");
    return 2;
  }
  const root = deps.root ?? ROOT;
  const out = join(root, "target", "nonstop");
  mkdirSync(out, { recursive: true });
  const stopFile = join(out, "STOP");
  const lockFile = join(out, "quiet.lock");

  const lock = claimLock(out, deps);
  if (!lock.ok) {
    console.log(`BUSY ${lock.why}`);
    return 4;
  }

  let wroteStop = false;
  const cleanup = () => {
    const stop = readJson(stopFile);
    if (wroteStop && stop?.pid === process.pid) rmSync(stopFile, { force: true });
    const held = readJson(lockFile);
    if (held?.pid === process.pid) rmSync(lockFile, { force: true });
  };

  try {
    const stop = claimStop(out, deps);
    if (!stop.ok) {
      console.log(`BUSY ${stop.why}`);
      return 4;
    }
    wroteStop = true;

    const scan = deps.scan ?? scanProcesses;
    const readCpu = deps.cpuTimes ?? cpuTimes;
    const wait = deps.wait ?? sleep;
    const spawn = deps.spawn ?? spawnSync;
    const deadline = Date.now() + opts.maxWaitMin * 60_000;

    for (;;) {
      const procs = scan();
      if (procs === null) {
        console.log("REFUSED process probe failed");
        return 2;
      }
      if (procs.nonstop.length) {
        console.log(`waiting nonstop pid ${procs.nonstop.join(",")}`);
      } else {
        const cpu = await sampleCpu({
          n: opts.samples,
          gapMs: opts.gap * 1000,
          read: readCpu,
          wait,
        });
        const verdict = quietVerdict({ ...procs, cpu, cpuMax: opts.cpuMax });
        console.log(verdict);
        if (verdict.startsWith("QUIET")) break;
      }
      if (Date.now() >= deadline) {
        console.log(`TIMEOUT after ${opts.maxWaitMin} min; command not run`);
        return 3;
      }
      await wait(opts.gap * 1000);
    }

    if (opts.dry) {
      console.log(`DRY would run: ${opts.cmd}`);
      return 0;
    }

    console.log(`run: ${opts.cmd}`);
    let code = 0;
    for (let i = 0; i < opts.times; i++) {
      const result = spawn(opts.cmd, opts.args, { cwd: root, stdio: "inherit" });
      code = result.error ? 2 : (result.status ?? 2);
      if (code !== 0) break;
    }
    return code;
  } finally {
    cleanup();
  }
}

if (import.meta.main) {
  process.on("SIGINT", () => process.exit(130));
  process.on("SIGTERM", () => process.exit(143));
  runQuiet(process.argv.slice(2)).then((code) => {
    process.exitCode = code;
  });
}
