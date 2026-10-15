// Run: node --test tools/web-search.test.mjs
// web-search (AW3b shell-safe wrapper, 149 lines, 0 tests before): --help
// prints usage; --dry-run keeps the AW3 probe query as one argv element,
// URL-encodes >= and creates no junk file; positional words join with
// --limit respected; bad limits, unknown flags and missing queries fail
// usage (exit 2); --self-check passes offline; the source keeps the
// fetch-only seam; and the named consumers (researcher wrapper,
// drift-check) route through this tool.
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { existsSync, readdirSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import test from "node:test";
import { HERE } from "./web-search.mjs";
const TOOL = join(HERE, "web-search.mjs");
const ROOT = dirname(HERE);
const RESEARCHER = join(ROOT, ".opencode", "agents", "researcher.md");
const DRIFT = join(HERE, "drift-check.mjs");

function runTool(args) {
  const r = spawnSync(process.execPath, [TOOL, ...args], { encoding: "utf8" });
  return { code: r.status, out: r.stdout ?? "", err: r.stderr ?? "" };
}

function junkFiles() {
  return readdirSync(ROOT).filter((n) => n.startsWith("=") || /^chk-.*\.out$/.test(n));
}

test("--help prints the shell-safe usage and exits 0", () => {
  const { code, out } = runTool(["--help"]);
  assert.equal(code, 0, `help is exit 0, got ${code}`);
  assert.match(out, /shell-safe web query wrapper/, "help names the wrapper");
  assert.match(out, /--query/, "help documents --query");
  assert.match(out, /--dry-run/, "help documents --dry-run");
});

test("--dry-run keeps the AW3 probe as one element, encodes >=, writes nothing", () => {
  const probe = "topic:game created:>=2026-06-27";
  const before = new Set(junkFiles());
  const { code, out, err } = runTool(["--query", probe, "--dry-run"]);
  assert.equal(code, 0, `dry-run is exit 0, got ${code}: ${err}${out}`);
  const req = JSON.parse(out);
  assert.equal(req.query, probe, "probe stays one argv element");
  assert.equal(req.method, "GET");
  assert.ok(!req.url.includes(">="), "no raw >= reaches a shell");
  assert.ok(req.url.includes(encodeURIComponent(">=")), "query is URL-encoded");
  assert.ok(req.url.includes("api.github.com/search/repositories"), "request targets the search API");
  const created = junkFiles().filter((n) => !before.has(n));
  assert.deepEqual(created, [], `dry run creates no file (new junk: ${created.join(", ") || "none"})`);
});

test("positional words join into one query and --limit is respected", () => {
  const { code, out, err } = runTool(["hello", "world", "--limit", "5", "--dry-run"]);
  assert.equal(code, 0, `positional dry-run is exit 0, got ${code}: ${err}${out}`);
  const req = JSON.parse(out);
  assert.equal(req.query, "hello world", "positionals join with a space");
  assert.match(req.url, /per_page=5/, "--limit reaches the request URL");
});

test("bad limits, unknown flags and missing queries fail usage with exit 2", () => {
  for (const bad of ["0", "101", "abc"]) {
    const r = runTool(["--query", "x", "--limit", bad, "--dry-run"]);
    assert.equal(r.code, 2, `--limit ${bad} is exit 2, got ${r.code}: ${r.err}${r.out}`);
    assert.match(r.err, /--limit must be 1\.\.100/, "limit gate named");
  }
  const unknown = runTool(["--bogus"]);
  assert.equal(unknown.code, 2, `unknown flag is exit 2, got ${unknown.code}`);
  assert.match(unknown.err, /unknown flag/, "unknown flag named");
  const missing = runTool(["--dry-run"]);
  assert.equal(missing.code, 2, `missing query is exit 2, got ${missing.code}`);
  assert.match(missing.err, /need --query/, "missing query named");
});

test("--self-check passes offline with its shell-safety checks", () => {
  const { code, out, err } = runTool(["--self-check"]);
  assert.equal(code, 0, `self-check is exit 0, got ${code}: ${err}${out}`);
  assert.match(out, /shell stays disabled/, "shell-seam check reported");
  assert.match(out, /no exec\/execSync/, "exec-seam check reported");
  assert.match(out, /stays one argv element/, "one-element check reported");
  assert.match(out, /URL-encoded/, "encoding check reported");
  assert.match(out, /creates no file/, "no-junk check reported");
  assert.match(out, /RESULT PASS/, "self-check ends RESULT PASS");
});

test("the tool source keeps the fetch-only seam with no shell string", () => {
  assert.equal(existsSync(TOOL), true, "tool tools/web-search.mjs exists");
  const src = readFileSync(TOOL, "utf8");
  assert.ok(!/shell\s*:\s*true/.test(src), "no shell:true seam in the wrapper");
  assert.ok(!/\bexecSync\s*\(/.test(src) && !/\bexec\s*\(/.test(src), "no exec/execSync seam");
  assert.match(src, /fetch\(req\.url/, "request goes through the built-in fetch seam");
  assert.match(src, /encodeURIComponent\(query\)/, "query is encoded into the URL");
});

test("the named consumers route through this tool", () => {
  assert.equal(existsSync(RESEARCHER), true, "consumer .opencode/agents/researcher.md exists");
  const wrapper = readFileSync(RESEARCHER, "utf8");
  assert.match(wrapper, /web-search/, "researcher wrapper routes web-search");
  assert.equal(existsSync(DRIFT), true, "consumer tools/drift-check.mjs exists");
  const drift = readFileSync(DRIFT, "utf8");
  assert.match(drift, /tools\/web-search\.mjs/, "drift-check names tools/web-search.mjs");
  // Same AW3 probe shape the wrapper guards: the tool must accept it dry.
  const { code, out, err } = runTool(["--query", "topic:game created:>=2026-06-27", "--limit", "10", "--dry-run"]);
  assert.equal(code, 0, `consumer-shape probe validates, got ${code}: ${err}${out}`);
  const req = JSON.parse(out);
  assert.equal(req.query, "topic:game created:>=2026-06-27");
  assert.match(req.url, /per_page=10/);
});
