// arena-net: the two-process loopback proof of `arena --host` / `arena --join`.
// Runs real arena processes on this PC with no window (`--headless`, scripted input), each
// scenario twice: plain duel, duel with the spear, the duel with a fault link on both sides
// (latency, jitter, loss, reorder) and, when a net_relay exe is given, the duel through the relay.
// Checks: both peers exit 0 and print the same final `arena sim:` hash; a scenario's two runs
// agree; the faulted and the relayed duel end on the plain duel's hash (the host waits for late
// inputs, so the network never changes the match); the spear duel ends elsewhere.
// Writes target/arena-net/receipt.json. Exit 0 all hold, 1 a check failed, 2 could not run.
// Usage: node tools/arena-net.mjs [--exe target/release/arena.exe] [--relay-exe target/release/net_relay.exe]
//        [--ticks 600] [--out target/arena-net]
import { spawn } from "node:child_process";
import { existsSync, mkdirSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { createInterface } from "node:readline";
import { fileURLToPath } from "node:url";

export const FAULT = "2,2,5,5";
const ROOT = join(fileURLToPath(new URL(".", import.meta.url)), "..");

// The guest line a host prints: `... types: arena.exe --scene arena --join ADDR CODE`.
export function joinLine(line) {
  const hit = /types: arena\.exe --scene arena --join (\S+) (E2040NET-\S+)\s*$/.exec(line);
  return hit && /this PC|the guest types/.test(line) ? { addr: hit[1], code: hit[2] } : null;
}

// `arena sim: ticks=N hash=H` -> { ticks, hash }.
export function simLine(line) {
  const hit = /^arena sim: ticks=(\d+) hash=([0-9a-f]{16})\s*$/.exec(line);
  return hit ? { ticks: Number(hit[1]), hash: hit[2] } : null;
}

// `arena net: receipt k=v k=v ...` -> { k: v }.
export function receiptLine(line) {
  const at = line.indexOf("arena net: receipt ");
  if (at < 0) return null;
  const out = {};
  for (const pair of line.slice(at + 19).trim().split(/\s+/)) {
    const eq = pair.indexOf("=");
    if (eq > 0) out[pair.slice(0, eq)] = pair.slice(eq + 1);
  }
  return out;
}

// `net-relay: serving on 127.0.0.1:PORT (token ...)` -> "127.0.0.1:PORT".
export function relayLine(line) {
  const hit = /^net-relay: serving on (\S+) /.exec(line);
  return hit ? hit[1] : null;
}

// Every failed check over the scenario results, as plain sentences (empty: all hold).
export function judge(results, ticks) {
  const fails = [];
  const byName = {};
  for (const run of results) {
    const tag = `${run.name} run ${run.round}`;
    for (const side of ["host", "guest"]) {
      const peer = run[side];
      if (peer.code !== 0) fails.push(`${tag}: ${side} exited ${peer.code}`);
      if (!peer.sim) fails.push(`${tag}: ${side} printed no arena sim line`);
      else if (peer.sim.ticks !== ticks) fails.push(`${tag}: ${side} ran ${peer.sim.ticks} ticks, want ${ticks}`);
      if (peer.receipt && peer.receipt.desync !== "none") fails.push(`${tag}: ${side} desync at ${peer.receipt.desync}`);
    }
    if (run.host.sim && run.guest.sim && run.host.sim.hash !== run.guest.sim.hash) {
      fails.push(`${tag}: host ${run.host.sim.hash} != guest ${run.guest.sim.hash}`);
    }
    (byName[run.name] ??= []).push(run.host.sim?.hash ?? null);
  }
  for (const [name, hashes] of Object.entries(byName)) {
    if (new Set(hashes).size !== 1) fails.push(`${name}: the runs disagree (${hashes.join(" ")})`);
  }
  const first = (name) => byName[name]?.[0] ?? null;
  for (const name of ["fault", "relay"]) {
    if (byName[name] && first(name) !== first("plain")) {
      fails.push(`${name}: ${first(name)} != plain ${first("plain")} (the network changed the match)`);
    }
  }
  if (byName.spear && first("spear") === first("plain")) fails.push("spear: same hash as the plain duel");
  return fails;
}

// Run one process to its exit, every stdout/stderr line to `onLine`; kill it at `timeoutMs`.
function runProcess(exe, args, onLine, timeoutMs) {
  return new Promise((done) => {
    const child = spawn(exe, args, { cwd: ROOT, stdio: ["ignore", "pipe", "pipe"], windowsHide: true });
    const lines = [];
    for (const stream of [child.stdout, child.stderr]) {
      createInterface({ input: stream }).on("line", (line) => {
        lines.push(line);
        onLine(line, child);
      });
    }
    const timer = setTimeout(() => child.kill(), timeoutMs);
    child.on("error", (err) => {
      clearTimeout(timer);
      done({ code: 2, lines: [...lines, String(err)], child });
    });
    child.on("close", (code) => {
      clearTimeout(timer);
      done({ code: code ?? 1, lines, child });
    });
    onLine(null, child);
  });
}

function peerResult(run) {
  return {
    code: run.code,
    sim: run.lines.map(simLine).find(Boolean) ?? null,
    receipt: run.lines.map(receiptLine).find(Boolean) ?? null,
    tail: run.lines.slice(-6),
  };
}

// One scenario run: a host, then a guest on the line the host printed.
async function scenario(exe, name, round, ticks, extra, relayAddr) {
  const timeoutMs = ticks * 17 + 90_000;
  const base = ["--scene", "arena", "--fighters", "2", "--ticks", String(ticks), "--headless"];
  const hostArgs = [...base, "--host", "--bind", "127.0.0.1:0", ...extra.host];
  if (relayAddr) hostArgs.push("--relay", relayAddr);
  let guestRun = null;
  const host = runProcess(exe, hostArgs, (line) => {
    const join = line && joinLine(line);
    if (join && !guestRun) {
      guestRun = runProcess(exe, ["--scene", "arena", "--headless", "--join", join.addr, join.code, ...extra.guest], () => {}, timeoutMs);
    }
  }, timeoutMs);
  const hosted = await host;
  const joined = guestRun ? await guestRun : { code: 2, lines: ["the host printed no join line"] };
  return { name, round, host: peerResult(hosted), guest: peerResult(joined) };
}

export function parseArgs(argv) {
  const opts = { exe: "target/release/arena.exe", relayExe: null, ticks: 600, out: "target/arena-net" };
  for (let i = 0; i < argv.length; i += 2) {
    const [flag, value] = [argv[i], argv[i + 1]];
    if (value === undefined) throw new Error(`${flag} needs a value`);
    if (flag === "--exe") opts.exe = value;
    else if (flag === "--relay-exe") opts.relayExe = value;
    else if (flag === "--ticks") opts.ticks = Number(value);
    else if (flag === "--out") opts.out = value;
    else throw new Error(`unknown flag ${flag}`);
  }
  if (!Number.isInteger(opts.ticks) || opts.ticks < 60) throw new Error("--ticks wants a whole number >= 60");
  return opts;
}

async function main(argv) {
  let opts;
  try {
    opts = parseArgs(argv);
  } catch (err) {
    console.error(`arena-net: ${err.message}`);
    return 2;
  }
  const exe = resolve(ROOT, opts.exe);
  if (!existsSync(exe)) {
    console.error(`arena-net: ${exe} is missing (cargo build --release -p engine-game --features app --bin arena)`);
    return 2;
  }
  const none = { host: [], guest: [] };
  const plans = [
    ["plain", none],
    ["spear", { host: ["--spear"], guest: [] }],
    ["fault", { host: ["--net-fault", FAULT], guest: ["--net-fault", FAULT] }],
  ];
  let relay = null;
  let relayAddr = null;
  if (opts.relayExe) {
    const relayExe = resolve(ROOT, opts.relayExe);
    relay = spawn(relayExe, ["--bind", "127.0.0.1:0"], { cwd: ROOT, stdio: ["ignore", "pipe", "pipe"], windowsHide: true });
    relayAddr = await new Promise((done) => {
      createInterface({ input: relay.stdout }).on("line", (line) => relayLine(line) && done(relayLine(line)));
      relay.on("error", () => done(null));
      setTimeout(() => done(null), 15_000);
    });
    if (!relayAddr) {
      console.error(`arena-net: ${relayExe} printed no serving line`);
      relay.kill();
      return 2;
    }
    plans.push(["relay", none]);
  }
  const results = [];
  for (const [name, extra] of plans) {
    for (const round of [1, 2]) {
      const run = await scenario(exe, name, round, opts.ticks, extra, name === "relay" ? relayAddr : null);
      results.push(run);
      const show = (peer) => `${peer.sim?.hash ?? "-"} exit=${peer.code}`;
      console.log(`arena-net: ${name} run ${round}: host ${show(run.host)} guest ${show(run.guest)}`);
      for (const side of ["host", "guest"]) {
        if (run[side].receipt) console.log(`  ${side} ${Object.entries(run[side].receipt).map(([k, v]) => `${k}=${v}`).join(" ")}`);
        if (run[side].code !== 0) console.log(`  ${side} tail:\n    ${run[side].tail.join("\n    ")}`);
      }
    }
  }
  relay?.kill();
  const fails = judge(results, opts.ticks);
  const out = resolve(ROOT, opts.out);
  mkdirSync(out, { recursive: true });
  const receipt = { date: new Date().toISOString(), exe: opts.exe, ticks: opts.ticks, fault: FAULT, results, fails };
  writeFileSync(join(out, "receipt.json"), `${JSON.stringify(receipt, null, 2)}\n`);
  console.log(fails.length ? `arena-net: FAIL\n  ${fails.join("\n  ")}` : `arena-net: PASS (${results.length} runs, receipt ${join(opts.out, "receipt.json")})`);
  return fails.length ? 1 : 0;
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main(process.argv.slice(2)).then((code) => process.exit(code));
}
