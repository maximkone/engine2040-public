// Run: node --test tools/jobserver.test.mjs. The pool checks need Windows (pwsh and a named semaphore); the live cap test also needs JOBSERVER_LIVE=1.
import { test } from "node:test";
import assert from "node:assert/strict";
import { spawn, spawnSync } from "node:child_process";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import os from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { lowerPriority, poolEnv, poolStatus } from "./jobserver.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));
const PS1 = join(HERE, "jobserver.ps1");
const MODULE = pathToFileURL(join(HERE, "jobserver.mjs")).href;
const WIN = process.platform === "win32";
const sleep = (ms) => new Promise((done) => setTimeout(done, ms));

// poolEnv reads the caller's env, so each pool test runs with CARGO_MAKEFLAGS and E2040_NO_POOL cleared, then restored.
async function clean(fn) {
  const keep = { CARGO_MAKEFLAGS: process.env.CARGO_MAKEFLAGS, E2040_NO_POOL: process.env.E2040_NO_POOL };
  delete process.env.CARGO_MAKEFLAGS;
  delete process.env.E2040_NO_POOL;
  try { return await fn(); } finally {
    for (const [k, v] of Object.entries(keep)) {
      if (v === undefined) delete process.env[k]; else process.env[k] = v;
    }
  }
}

function killTree(child) {
  if (!child?.pid) return;
  if (WIN) spawnSync("taskkill", ["/PID", String(child.pid), "/T", "/F"], { stdio: "ignore" });
  try { child.kill(); } catch {}
}

function startHolder(name) {
  return spawn("pwsh", ["-NoProfile", "-File", PS1, "start", "-Tokens", "1", "-Name", name], { stdio: "ignore" });
}

// Polls the pool without the cache until it is up or the budget is spent.
async function waitUp(name, ms) {
  const end = Date.now() + ms;
  for (;;) {
    const st = poolStatus(name, { fresh: true });
    if (st.up || Date.now() >= end) return st;
    await sleep(300);
  }
}

test("poolEnv is empty for a pool nobody holds", () => clean(() => {
  const name = `e2040-js-none-${process.pid}-${Math.random().toString(36).slice(2)}`;
  assert.deepEqual(poolEnv(name), {});
}));

test("a 1-slot holder under a test name: status counts 1 free and poolEnv names it", { skip: !WIN && "the pool is Windows only" }, () => clean(async () => {
  const name = `e2040-js-test-${process.pid}`;
  const holder = startHolder(name);
  try {
    const st = await waitUp(name, 15000);
    assert.equal(st.up, true, "holder did not come up within 15 s");
    assert.equal(st.free, 1);
    assert.deepEqual(poolEnv(name), { CARGO_MAKEFLAGS: `-j --jobserver-auth=${name}` });
  } finally {
    killTree(holder);
  }
}));

test("lowerPriority: a child that calls it spawns a grandchild below normal (10)", () => {
  const code = `import { spawnSync } from "node:child_process";
import { lowerPriority } from ${JSON.stringify(MODULE)};
lowerPriority();
const r = spawnSync(process.execPath, ["-e", "process.stdout.write(String(require('os').getPriority()))"], { encoding: "utf8" });
process.stdout.write(r.stdout);`;
  const out = spawnSync(process.execPath, ["--input-type=module", "-e", code], { encoding: "utf8" });
  assert.equal(out.status, 0, out.stderr);
  assert.equal(out.stdout.trim(), "10");
});

const LIVE_TOML = `[package]
name = "js_probe"
version = "0.1.0"
edition = "2021"

[dependencies]
regex = "1"
serde_json = "1"
itertools = "0.13"
rand = "0.8"
smallvec = "1"
bitflags = "2"
memchr = "2"
`;

// rustc processes whose command line names this target dir: [all, parentless]. Parentless is one per job when rustup proxies rustc.
function rustcCount(dir) {
  const ps = `$m = @(Get-CimInstance Win32_Process -Filter "name='rustc.exe'" | Where-Object { $_.CommandLine -and $_.CommandLine.Contains($env:E2040_JS_DIR) }); $ids = @($m | ForEach-Object { $_.ProcessId }); "$(@($m).Count) $(@($m | Where-Object { $ids -notcontains $_.ParentProcessId }).Count)"`;
  const r = spawnSync("pwsh", ["-NoProfile", "-Command", ps], { encoding: "utf8", timeout: 20000, env: { ...process.env, E2040_JS_DIR: dir } });
  const m = /^(\d+) (\d+)/.exec((r.stdout ?? "").trim());
  if (!m) throw new Error(`rustc count failed: ${r.stderr || r.error?.message}`);
  return { all: Number(m[1]), parentless: Number(m[2]) };
}

const liveSkip = !WIN ? "the live cap test is Windows only" : process.env.JOBSERVER_LIVE !== "1" ? "set JOBSERVER_LIVE=1 to run the live cap test" : false;

test("live: a 7-dependency check under a 1-slot pool never runs more than 2 rustc at once", { skip: liveSkip }, () => clean(async () => {
  const name = `e2040-js-test-live-${process.pid}`;
  const dir = mkdtempSync(join(os.tmpdir(), "e2040-js-live-"));
  const target = join(dir, "t");
  const holder = startHolder(name);
  let cargo = null;
  try {
    assert.equal((await waitUp(name, 15000)).free, 1, "1-slot holder did not come up");
    mkdirSync(join(dir, "src"));
    writeFileSync(join(dir, "Cargo.toml"), LIVE_TOML);
    writeFileSync(join(dir, "src", "lib.rs"), "pub fn probe() {}\n");
    const done = { ended: false, code: null };
    cargo = spawn("cargo", ["check", "--offline", "--target-dir", target], {
      cwd: dir, stdio: "ignore", env: { ...process.env, CARGO_MAKEFLAGS: `-j --jobserver-auth=${name}` },
    });
    cargo.on("exit", (code) => { done.ended = true; done.code = code; });
    const t0 = Date.now();
    let max = 0, maxParentless = 0, samples = 0;
    while (!done.ended && Date.now() - t0 < 60000) {
      const c = rustcCount(target);
      samples++;
      max = Math.max(max, c.all);
      maxParentless = Math.max(maxParentless, c.parentless);
      await sleep(300);
    }
    const wall = (Date.now() - t0) / 1000;
    console.log(`live cap: max rustc ${max} (parentless ${maxParentless}), ${samples} samples, cargo ${done.ended ? `exit ${done.code}` : "still running at 60 s"}, wall ${wall.toFixed(1)} s`);
    assert.ok(max <= 2, `saw ${max} rustc at once under a 1-slot pool`);
    assert.ok(max >= 1, "no rustc was seen, so the cap proved nothing");
    assert.ok(!done.ended || done.code === 0, `cargo check failed (exit ${done.code}), so the cap proved nothing`);
  } finally {
    killTree(cargo);
    killTree(holder);
    try { rmSync(dir, { recursive: true, force: true }); } catch {}
  }
}));
