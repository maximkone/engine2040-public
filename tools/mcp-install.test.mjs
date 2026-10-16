import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const TOOL = fileURLToPath(new URL("./mcp-install.mjs", import.meta.url));
const cleanEnv = { ...process.env, E2040_MCP_BIN: "" };
const run = (args, cwd) => spawnSync(process.execPath, [TOOL, ...args], {
  encoding: "utf8", cwd, env: cleanEnv, timeout: 60_000,
});
const sandbox = () => {
  const dir = mkdtempSync(join(tmpdir(), "e2040-mcp-install-"));
  writeFileSync(join(dir, "opencode.jsonc"), "{}\n");
  return dir;
};
const drop = (dir) => rmSync(dir, { recursive: true, force: true });

test("--help exits 0 with the usage line and changes nothing", () => {
  const dir = sandbox();
  try {
    const r = run(["--help"], dir);
    assert.equal(r.status, 0);
    assert.match(r.stdout, /Usage: node tools\/mcp-install\.mjs/);
    assert.equal(readFileSync(join(dir, "opencode.jsonc"), "utf8"), "{}\n");
    assert.equal(existsSync(join(dir, ".opencode")), false);
  } finally {
    drop(dir);
  }
});

test("an unknown flag exits 2 and names the flag", () => {
  const dir = sandbox();
  try {
    const r = run(["--bogus"], dir);
    assert.equal(r.status, 2);
    assert.match(r.stderr, /unknown flag\(s\): --bogus/);
  } finally {
    drop(dir);
  }
});

test("an empty --bin value exits 2", () => {
  const dir = sandbox();
  try {
    const r = run(["--bin", ""], dir);
    assert.equal(r.status, 2);
    assert.match(r.stderr, /--bin needs a non-empty value/);
  } finally {
    drop(dir);
  }
});

test("--check with no binary FAILs and writes nothing", () => {
  const dir = sandbox();
  try {
    const r = run(["--check"], dir);
    assert.equal(r.status, 1);
    assert.match(r.stdout, /RESULT FAIL/);
    assert.match(r.stdout, /no e2040-mcp binary found/);
    assert.equal(readFileSync(join(dir, "opencode.jsonc"), "utf8"), "{}\n");
    assert.equal(existsSync(join(dir, ".opencode")), false);
  } finally {
    drop(dir);
  }
});

test("install with a --bin writes the copy plus both entries, reruns byte-identical", () => {
  const dir = sandbox();
  try {
    const fake = join(dir, "fake-mcp.bin");
    writeFileSync(fake, "fake-binary-bytes");
    const first = run(["--bin", fake], dir);
    assert.equal(first.status, 1, "fake probe hides --project-dir so the run still FAILs");
    assert.match(first.stdout, /hides --project-dir/);
    const dest = join(dir, ".opencode", "bin", process.platform === "win32" ? "e2040-mcp.exe" : "e2040-mcp");
    assert.equal(existsSync(dest), true);
    assert.equal(readFileSync(dest, "utf8"), "fake-binary-bytes");
    const cfg = readFileSync(join(dir, "opencode.jsonc"), "utf8");
    assert.match(cfg, /"e2040": \{/);
    assert.match(cfg, /"e2040-remote": \{/);
    assert.match(cfg, /"enabled": true/);
    assert.match(cfg, /"enabled": false/);
    const second = run(["--bin", fake], dir);
    assert.equal(second.status, 1, "probe still fails, but the merge stays put");
    assert.match(second.stdout, /already current/);
    assert.equal(readFileSync(join(dir, "opencode.jsonc"), "utf8"), cfg);
  } finally {
    drop(dir);
  }
});

test("--check after an install reports the stale probe but leaves the config alone", () => {
  const dir = sandbox();
  try {
    const fake = join(dir, "fake-mcp.bin");
    writeFileSync(fake, "fake-binary-bytes");
    run(["--bin", fake], dir);
    const before = readFileSync(join(dir, "opencode.jsonc"), "utf8");
    const r = run(["--bin", fake, "--check"], dir);
    assert.equal(r.status, 1);
    assert.match(r.stdout, /RESULT FAIL/);
    assert.equal(readFileSync(join(dir, "opencode.jsonc"), "utf8"), before);
  } finally {
    drop(dir);
  }
});
