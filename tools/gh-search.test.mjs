import assert from "node:assert/strict";
import test from "node:test";
import { spawnSync } from "node:child_process";
import { readdirSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const TOOL = fileURLToPath(new URL("./gh-search.mjs", import.meta.url));
const ROOT = dirname(dirname(TOOL));
const run = (args) => spawnSync(process.execPath, [TOOL, ...args], { encoding: "utf8", cwd: ROOT });
const junk = () => readdirSync(ROOT).filter((n) => n.startsWith("=") || /^chk-.*\.out$/.test(n));

// AW3: pwsh parses >= as a redirect, so the query must stay one argv
// element through the wrapper and never touch a shell.
const PROBE = "topic:game created:>=2026-06-27";

test("--help prints usage and exits 0", () => {
  const r = run(["--help"]);
  assert.equal(r.status, 0);
  assert.match(r.stdout, /gh-search\.mjs/);
  assert.match(r.stdout, /--dry-run/);
});

test("--dry-run prints the gh argv with the query as one element and creates no file", () => {
  const before = new Set(junk());
  const r = run(["--query", PROBE, "--dry-run"]);
  assert.equal(r.status, 0);
  const argv = JSON.parse(r.stdout.trim());
  assert.deepEqual(argv.slice(0, 2), ["gh", "search"]);
  assert.ok(argv.includes(PROBE), `probe query stays one element: ${JSON.stringify(argv)}`);
  assert.ok(!argv.some((a) => a.includes(">=") && a !== PROBE), "no raw >= fragment outside the query element");
  const created = junk().filter((n) => !before.has(n));
  assert.deepEqual(created, [], "dry run creates no junk file");
});

test("--dry-run with --type code selects the code field set", () => {
  const r = run(["--query", PROBE, "--type", "code", "--limit", "5", "--dry-run"]);
  assert.equal(r.status, 0);
  const argv = JSON.parse(r.stdout.trim());
  assert.ok(argv.includes("code"), `type element present: ${JSON.stringify(argv)}`);
  assert.ok(argv.includes("path,url,repository"), `code fields selected: ${JSON.stringify(argv)}`);
  assert.ok(argv.includes(PROBE), "query survives as one element");
});

test("unknown flag, bad type and bad limit are usage errors (exit 2), never a search", () => {
  for (const args of [
    ["--query", PROBE, "--bogus"],
    ["--query", PROBE, "--type", "bogus", "--dry-run"],
    ["--query", PROBE, "--limit", "0", "--dry-run"],
    ["--query", PROBE, "--limit", "abc", "--dry-run"],
  ]) {
    const r = run(args);
    assert.equal(r.status, 2, `${args.join(" ")} exits 2`);
    assert.match(r.stderr, /node tools\/gh-search\.mjs/);
  }
  const missing = run(["--type", "repos", "--dry-run"]);
  assert.equal(missing.status, 2);
  assert.match(missing.stderr, /need --query/);
});
