// node --test tools/asset-gen/guards.test.mjs
//
// The generators stay quiet unless told. --help and --dry-run never reach the
// network or the disk, and a run without --yes never starts. Every child runs
// with a preload that turns any fetch, http request or child process into exit
// 99, and with a dead proxy, so a leak fails loudly and costs nothing.
import { test } from "node:test";
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { existsSync, mkdtempSync, readdirSync, rmSync, statSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
const CONCEPTS_DIR = join(ROOT, "content", "generated", "concepts");
const CONCEPTS = "tools/asset-gen/concepts.mjs";
const BATCH = "tools/asset-gen/batch.mjs";
const TRIPO = "tools/asset-gen/tripo.mjs";
const SF3D = "tools/asset-gen/sf3d.mjs";
const TRIPO_IMAGE = "content/generated/concepts/concept_warden_fighter.jpg";

// Runs before the script. Any network or process start is refused with exit 99.
const TRAP = `
import cp from "node:child_process";
import http from "node:http";
import https from "node:https";
import { syncBuiltinESMExports } from "node:module";
const block = (what) => () => { process.stderr.write("GUARD blocked " + what + "\\n"); process.exit(99); };
globalThis.fetch = block("fetch");
for (const m of [http, https]) { m.request = block("http request"); m.get = block("http get"); }
for (const f of ["exec", "execFile", "execFileSync", "execSync", "spawn", "spawnSync", "fork"]) cp[f] = block("child " + f);
syncBuiltinESMExports();
`;
const TRAP_URL = `data:text/javascript,${encodeURIComponent(TRAP)}`;
const DEAD_PROXY = "http://127.0.0.1:9";

function run(script, args) {
  const env = { ...process.env, HTTPS_PROXY: DEAD_PROXY, HTTP_PROXY: DEAD_PROXY, https_proxy: DEAD_PROXY, http_proxy: DEAD_PROXY };
  delete env.TRIPO_API_KEY;
  return spawnSync(process.execPath, ["--import", TRAP_URL, join(ROOT, script), ...args], {
    cwd: ROOT, encoding: "utf8", timeout: 60_000, env,
  });
}

function assertClean(r, code) {
  assert.equal(r.status, code, `exit ${r.status}, want ${code}\n${r.stdout}\n${r.stderr}`);
  assert.doesNotMatch(r.stderr, /GUARD blocked/);
}

// Name, size and mtime of everything directly in a folder, so any write shows.
function snapshot(dir) {
  if (!existsSync(dir)) return "absent";
  return readdirSync(dir).sort().map((n) => {
    const s = statSync(join(dir, n));
    return `${n} ${s.size} ${s.mtimeMs}`;
  }).join("\n");
}

function withTmp(fn) {
  const dir = mkdtempSync(join(tmpdir(), "asset-gen-guard-"));
  try {
    return fn(dir);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
}

test("concepts --help prints usage, exit 0, nothing requested or written", () => {
  const before = snapshot(CONCEPTS_DIR);
  const r = run(CONCEPTS, ["--help"]);
  assertClean(r, 0);
  assert.match(r.stdout, /usage: node tools\/asset-gen\/concepts\.mjs/);
  assert.equal(snapshot(CONCEPTS_DIR), before);
});

test("concepts without --yes prints the plan, exit 2, nothing requested or written", () => {
  const before = snapshot(CONCEPTS_DIR);
  const r = run(CONCEPTS, []);
  assertClean(r, 2);
  assert.match(r.stdout, /prompt:/);
  assert.match(r.stderr, /add --yes to spend/);
  assert.equal(snapshot(CONCEPTS_DIR), before);
});

test("concepts --dry-run lists the jobs, exit 0, nothing requested or written", () => {
  const before = snapshot(CONCEPTS_DIR);
  const r = run(CONCEPTS, ["--dry-run", "--only", "ashkin_gunner"]);
  assertClean(r, 0);
  assert.match(r.stdout, /concept_ashkin_gunner\.jpg/);
  assert.equal(snapshot(CONCEPTS_DIR), before);
});

test("batch --help prints usage, exit 0", () => {
  withTmp((out) => {
    const r = run(BATCH, ["--help", "--out", out]);
    assertClean(r, 0);
    assert.match(r.stdout, /usage: node tools\/asset-gen\/batch\.mjs/);
    assert.deepEqual(readdirSync(out), []);
  });
});

test("batch without --yes prints the plan, exit 2, out folder untouched", () => {
  withTmp((out) => {
    const r = run(BATCH, ["--out", out]);
    assertClean(r, 2);
    assert.match(r.stdout, /concept\(s\) to run/);
    assert.match(r.stderr, /add --yes to run/);
    assert.deepEqual(readdirSync(out), []);
  });
});

test("batch --dry-run lists the concepts, exit 0, out folder untouched", () => {
  withTmp((out) => {
    const r = run(BATCH, ["--dry-run", "--only", "ashkin_gunner", "--out", out]);
    assertClean(r, 0);
    assert.match(r.stdout, /ashkin_gunner/);
    assert.deepEqual(readdirSync(out), []);
  });
});

test("sf3d --help prints usage, exit 0, no python started", () => {
  const r = run(SF3D, ["--help"]);
  assertClean(r, 0);
  assert.match(r.stdout, /usage: node tools\/asset-gen\/sf3d\.mjs/);
});

test("tripo --help prints usage, exit 0", () => {
  withTmp((out) => {
    const r = run(TRIPO, ["--help", "--out", out]);
    assertClean(r, 0);
    assert.match(r.stdout, /usage: node tools\/asset-gen\/tripo\.mjs/);
    assert.deepEqual(readdirSync(out), []);
  });
});

test("tripo refuses the free plan before anything else", () => {
  withTmp((out) => {
    const r = run(TRIPO, ["--text", "a wooden crate", "--out", out, "--plan", "free"]);
    assertClean(r, 1);
    assert.match(r.stderr, /--plan must be "paid"/);
    assert.deepEqual(readdirSync(out), []);
  });
});

test("tripo without --yes prints the plan, exit 2, no key read, nothing sent or written", () => {
  withTmp((out) => {
    const r = run(TRIPO, ["--text", "a wooden crate", "--out", out]);
    assertClean(r, 2);
    assert.match(r.stdout, /plan: one paid Tripo text-to-model task/);
    assert.match(r.stderr, /add --yes to spend/);
    assert.deepEqual(readdirSync(out), []);
  });
});

test("tripo --dry-run with a picture prints the plan, exit 0, nothing written", () => {
  withTmp((out) => {
    const r = run(TRIPO, ["--image", TRIPO_IMAGE, "--out", out, "--dry-run"]);
    assertClean(r, 0);
    assert.match(r.stdout, /model\.glb/);
    assert.deepEqual(readdirSync(out), []);
  });
});
