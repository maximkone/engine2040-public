import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { copyFileSync, existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, utimesSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { copyVerdict, refreshCopy } from "./mcp-install.mjs";

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
const EXE = process.platform === "win32" ? "e2040-mcp.exe" : "e2040-mcp";
const OLD = new Date(2026, 8, 26, 12, 0); // local 2026-09-26 12:00
const NEW = new Date(2026, 9, 20, 12, 0); // local 2026-10-20 12:00
// target/release/ build and .opencode/bin/ copy with chosen bytes and mtimes.
const layOut = (dir, { copyBytes, copyAt, releaseAt }) => {
  const release = join(dir, "target", "release", EXE);
  const copy = join(dir, ".opencode", "bin", EXE);
  mkdirSync(dirname(release), { recursive: true });
  mkdirSync(dirname(copy), { recursive: true });
  writeFileSync(release, "build-bytes");
  writeFileSync(copy, copyBytes);
  utimesSync(release, releaseAt, releaseAt);
  utimesSync(copy, copyAt, copyAt);
};
// A copy function the running server locks once (EBUSY), then copies normally.
const lockedOnce = () => {
  const calls = [];
  const copy = (from, to) => {
    calls.push(to);
    if (calls.length === 1) throw Object.assign(new Error("EBUSY: resource busy or locked"), { code: "EBUSY" });
    copyFileSync(from, to);
  };
  return { copy, calls };
};

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

test("--check fails with a stale-copy line when the copy is older than the release build", () => {
  const dir = sandbox();
  try {
    layOut(dir, { copyBytes: "build-bytes", copyAt: OLD, releaseAt: NEW });
    const r = run(["--check"], dir);
    assert.equal(r.status, 1);
    assert.match(r.stdout, /RESULT FAIL/);
    assert.match(
      r.stdout,
      /\[NEEDS YOU\] copy is stale: \.opencode\/bin\/e2040-mcp(\.exe)? \(2026-09-26 12:00\) is older than target\/release\/e2040-mcp(\.exe)? \(2026-10-20 12:00\)/,
    );
  } finally {
    drop(dir);
  }
});

test("--check reads a copy at least as new as the release build as current", () => {
  const dir = sandbox();
  try {
    layOut(dir, { copyBytes: "build-bytes", copyAt: NEW, releaseAt: OLD });
    const r = run(["--check"], dir);
    // The fake bytes fail the --help probe, so the overall result stays FAIL; this pins the copy line.
    assert.match(r.stdout, /\[OK\] copy current: \.opencode\/bin\/e2040-mcp(\.exe)? matches target\/release\/e2040-mcp(\.exe)?/);
    assert.doesNotMatch(r.stdout, /copy is stale|copy missing/);
  } finally {
    drop(dir);
  }
});

test("copyVerdict passes a copy that matches the build and flags a size mismatch", () => {
  const dir = mkdtempSync(join(tmpdir(), "e2040-mcp-verdict-"));
  try {
    const release = join(dir, "release.exe");
    const copy = join(dir, "bin.exe");
    writeFileSync(release, "build-bytes");
    writeFileSync(copy, "build-bytes");
    utimesSync(release, OLD, OLD);
    utimesSync(copy, NEW, NEW);
    assert.equal(copyVerdict(release, copy).ok, true);
    writeFileSync(copy, "other");
    utimesSync(copy, NEW, NEW);
    const verdict = copyVerdict(release, copy);
    assert.equal(verdict.ok, false);
    assert.match(verdict.text, /copy is stale: .*differs in size from .*release\.exe/);
  } finally {
    drop(dir);
  }
});

test("a locked copy is renamed aside by its mtime date, then replaced; nothing is deleted", () => {
  const dir = mkdtempSync(join(tmpdir(), "e2040-mcp-lock-"));
  try {
    const src = join(dir, "release.exe");
    const dest = join(dir, "bin.exe");
    writeFileSync(src, "new-build");
    writeFileSync(dest, "old-build");
    utimesSync(dest, OLD, OLD);
    const { copy, calls } = lockedOnce();
    const { aside } = refreshCopy(src, dest, { copy });
    assert.equal(calls.length, 2, "the first copy hits the lock, the retry copies");
    assert.equal(aside, `${dest}.stale-2026-09-26`);
    assert.equal(readFileSync(aside, "utf8"), "old-build");
    assert.equal(readFileSync(dest, "utf8"), "new-build");
  } finally {
    drop(dir);
  }
});

test("a second lock on the same day keeps the earlier stale copy and takes a new name", () => {
  const dir = mkdtempSync(join(tmpdir(), "e2040-mcp-lock-"));
  try {
    const src = join(dir, "release.exe");
    const dest = join(dir, "bin.exe");
    writeFileSync(src, "new-build");
    writeFileSync(dest, "old-build");
    utimesSync(dest, OLD, OLD);
    writeFileSync(`${dest}.stale-2026-09-26`, "earlier-stale");
    const { copy } = lockedOnce();
    const { aside } = refreshCopy(src, dest, { copy });
    assert.equal(aside, `${dest}.stale-2026-09-26-2`);
    assert.equal(readFileSync(`${dest}.stale-2026-09-26`, "utf8"), "earlier-stale");
    assert.equal(readFileSync(aside, "utf8"), "old-build");
  } finally {
    drop(dir);
  }
});

test("a copy failure that is not a lock is rethrown and renames nothing", () => {
  const dir = mkdtempSync(join(tmpdir(), "e2040-mcp-lock-"));
  try {
    const src = join(dir, "release.exe");
    const dest = join(dir, "bin.exe");
    writeFileSync(src, "new-build");
    writeFileSync(dest, "old-build");
    utimesSync(dest, OLD, OLD);
    const copy = () => {
      throw Object.assign(new Error("ENOSPC: no space left on device"), { code: "ENOSPC" });
    };
    assert.throws(() => refreshCopy(src, dest, { copy }), /ENOSPC/);
    assert.equal(readFileSync(dest, "utf8"), "old-build");
    assert.equal(existsSync(`${dest}.stale-2026-09-26`), false);
  } finally {
    drop(dir);
  }
});

test("the disabled e2040-remote entry stays a --project-dir placeholder until the binary advertises --remote", () => {
  const dir = sandbox();
  try {
    const fake = join(dir, "fake-mcp.bin");
    writeFileSync(fake, "fake-binary-bytes");
    run(["--bin", fake], dir);
    const cfgPath = join(dir, "opencode.jsonc");
    // What an earlier install left behind: the remote entry pointing at --remote.
    const withRemote = readFileSync(cfgPath, "utf8").replace(
      /("e2040-remote"[\s\S]*?"command": )\[[^\]]*\]/,
      '$1[".opencode/bin/e2040-mcp.exe", "--remote", "http://127.0.0.1:18789"]',
    );
    assert.match(withRemote, /"--remote"/);
    writeFileSync(cfgPath, withRemote);
    run(["--bin", fake], dir);
    const cfg = readFileSync(cfgPath, "utf8");
    assert.doesNotMatch(cfg, /"--remote"/);
    assert.doesNotMatch(cfg, /127\.0\.0\.1:18789/);
    assert.match(cfg, /"e2040-remote": \{[\s\S]*?"command": \["[^"]*", "--project-dir", "\.opencode\/e2040-scratch"\]/);
    assert.match(cfg, /"enabled": false/);
  } finally {
    drop(dir);
  }
});
