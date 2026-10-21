// Our cargo joins the shared compile-slot pool (tools/jobserver.ps1) when it is up, and runs below the desktop's priority (OPT37, OPT40).
// A cargo with CARGO_MAKEFLAGS=-j --jobserver-auth=<name> takes slots from one named semaphore instead of a 12-job pool of its own.
// Windows hands the priority class down, so that cargo and every rustc under it inherit below-normal.
// Node built-ins only. The caller's own CARGO_MAKEFLAGS wins; E2040_NO_POOL=1 turns the pool off.
// E2040_POOL names the pool this session talks to (the tests hold their own); unset, the machine-wide pool.
import { spawnSync } from "node:child_process";
import os from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

export const POOL = "engine2040-jobserver";
const PS1 = join(dirname(fileURLToPath(import.meta.url)), "jobserver.ps1");
const seen = new Map();

// The pool this session talks to. E2040_POOL names another one (the tests hold
// their own); unset, every tool uses the machine-wide pool.
export const poolName = () => process.env.E2040_POOL || POOL;

// { up, free } for a named pool, cached per name for this process. { fresh: true } asks the pool again.
export function poolStatus(name = poolName(), { fresh = false } = {}) {
  if (!fresh && seen.has(name)) return seen.get(name);
  let status = { up: false, free: 0 };
  if (process.platform === "win32") {
    const r = spawnSync("pwsh", ["-NoProfile", "-File", PS1, "status", "-Name", name], { encoding: "utf8", timeout: 15000 });
    const m = r.status === 0 ? /up, (\d+) slots free/.exec(r.stdout ?? "") : null;
    if (m) status = { up: true, free: Number(m[1]) };
  }
  seen.set(name, status);
  return status;
}

// A spike (a measurement build) waits for a free compile slot (OPT41): with the pool
// up and no slot free it must not start, so a spike never compiles next to live work.
// --force starts it anyway. A pool that is down, or one with a free slot, changes nothing.
// The reason always names the state, so a tool can print one line and stop.
export function slotGate({ force = false, name = poolName() } = {}) {
  if (force) return { ok: true, up: false, free: null, reason: "forced" };
  if (process.env.E2040_NO_POOL === "1") return { ok: true, up: false, free: null, reason: "the pool is off" };
  const st = poolStatus(name, { fresh: true });
  if (!st.up) return { ok: true, up: false, free: 0, reason: "no pool to wait for" };
  if (st.free > 0) return { ok: true, up: true, free: st.free, reason: `${st.free} free slots` };
  return { ok: false, up: true, free: 0, reason: `the compile-slot pool ${name} has 0 free slots` };
}

// The env a cargo needs to join the pool; {} when the caller set CARGO_MAKEFLAGS, E2040_NO_POOL=1, the pool is down, or not Windows.
export function poolEnv(name = poolName()) {
  if (process.env.CARGO_MAKEFLAGS || process.env.E2040_NO_POOL === "1") return {};
  return poolStatus(name).up ? { CARGO_MAKEFLAGS: `-j --jobserver-auth=${name}` } : {};
}

// Lowers this process; children (cargo, rustc) inherit the class on Windows. Safe to call twice.
export function lowerPriority() {
  try { os.setPriority(os.constants.priority.PRIORITY_BELOW_NORMAL); } catch {}
}
