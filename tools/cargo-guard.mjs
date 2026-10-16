// cargo-guard: BUILD-GUARD time/message caps beside tools/proof.mjs (OPT36).
// PORT cargo-limit env_vars.rs + process.rs shape, reimplemented, zero donor bytes.
// Reads CARGO_TIME_LIMIT (default 1s, 0 = no limit) + CARGO_MSG_LIMIT (default
// 0 = no limit), streams the child, starts the kill timer only after the first
// error line, kills the child tree at the cap, prints E_BUILD_TIMEOUT receipt.
import { spawn, spawnSync } from "node:child_process";
export const DEFAULT_TIME_LIMIT_S = 1;
export function parseLimits(env = {}) {
  const t = env.CARGO_TIME_LIMIT ?? "1";
  const m = env.CARGO_MSG_LIMIT ?? "0";
  let time = Number(String(t).trim() === "" ? "1" : t);
  if (!Number.isFinite(time) || time < 0) time = DEFAULT_TIME_LIMIT_S;
  let msg = Number(String(m).trim() === "" ? "0" : m);
  if (!Number.isFinite(msg) || msg < 0) msg = 0;
  return { timeLimitSec: time, msgLimit: Math.floor(msg) };
}
export function isErrorLine(line) {
  return /\berror(?::|\[)/i.test(String(line ?? ""));
}
export function trimTail(lines, limit) {
  const arr = [...(lines ?? [])];
  if (!(limit > 0)) return arr;
  return arr.slice(0, limit);
}
function killTree(pid) {
  if (process.platform === "win32") {
    spawnSync("taskkill", ["/PID", String(pid), "/T", "/F"], { stdio: "ignore" });
  } else {
    try { process.kill(pid, "SIGINT"); } catch {}
  }
}
function main() {
  const raw = process.argv.slice(2);
  const argv = raw[0] === "--" ? raw.slice(1) : raw;
  if (!argv.length) { console.error("Usage: node tools/cargo-guard.mjs [--] <cmd> [args...]"); process.exit(2); }
  const { timeLimitSec, msgLimit } = parseLimits(process.env);
  const child = spawn(argv[0], argv.slice(1), { stdio: ["ignore", "pipe", "pipe"] });
  let state = "Running";
  let firstAt = 0; let timer = null; let timedOut = false; let sent = 0;
  const killAtCap = () => {
    if (state === "Killing" || child.exitCode !== null) return;
    state = "Killing"; timedOut = true;
    const elapsed = ((Date.now() - firstAt) / 1000).toFixed(1);
    killTree(child.pid);
    setTimeout(() => { try { child.kill("SIGKILL"); } catch {} }, 2000);
    console.error(`E_BUILD_TIMEOUT: red build killed at CARGO_TIME_LIMIT=${timeLimitSec}s (elapsed ${elapsed}s after first error)`);
  };
  const onLine = (line, out) => {
    if (state === "Running" && isErrorLine(line)) {
      state = "KillTimerStarted"; firstAt = Date.now();
      if (timeLimitSec > 0) timer = setTimeout(killAtCap, timeLimitSec * 1000);
    }
    if (msgLimit > 0 && sent >= msgLimit) return;
    sent += 1; out.write(`${line}\n`);
  };
  const pump = (chunk, rest, out) => {
    const parts = String(rest.buf + chunk).split("\n"); rest.buf = parts.pop() ?? "";
    for (const p of parts) onLine(p.replace(/\r$/, ""), out);
  };
  const rOut = { buf: "" }; const rErr = { buf: "" };
  child.stdout.on("data", (c) => pump(c, rOut, process.stdout));
  child.stderr.on("data", (c) => pump(c, rErr, process.stderr));
  child.on("error", (err) => { console.error(`cargo-guard: spawn failed: ${err.message}`); process.exit(127); });
  child.on("close", (code, sig) => {
    clearTimeout(timer);
    if (timedOut) process.exit(124);
    process.exit(code ?? (sig ? 1 : 0));
  });
}
if (import.meta.main) main();
