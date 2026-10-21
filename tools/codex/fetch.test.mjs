// fetch.test.mjs - CDX1 accept run against a local http fixture.
// No real hosts: a fixture server on 127.0.0.1 serves the bytes, the sidecar
// is written, the size cap stops a source, the rate limit is honoured, a dry
// run writes nothing, and the shipped table prints one line per source.
import { deepStrictEqual, equal, match, ok } from "node:assert";
import test from "node:test";
import { createServer } from "node:http";
import { existsSync, mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { createHash } from "node:crypto";
import { spawn } from "node:child_process";
import { parseSources, run } from "./fetch.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));
const TOOL = resolve(HERE, "fetch.mjs");
const SHIPPED = resolve(HERE, "sources.json");

function shut(srv) {
  try {
    if (srv.closeAllConnections) srv.closeAllConnections();
  } catch {}
  srv.close();
}

// A fixture server that answers /item/<id> with a fixed body and records the
// time of every hit, so the test can measure the rate limit.
function serve(body) {
  const hits = [];
  const srv = createServer((req, res) => {
    hits.push(Date.now());
    const buf = Buffer.from(body);
    res.writeHead(200, { "Content-Type": "text/plain", "Content-Length": buf.length });
    res.end(buf);
  });
  return new Promise((ready) => {
    srv.listen(0, "127.0.0.1", () => ready({ srv, hits, port: srv.address().port }));
  });
}

function tmp() {
  return mkdtempSync(join(tmpdir(), "codex-"));
}

// A source table that points at the fixture server.
function table(port, over) {
  const src = {
    id: "fixture",
    label: "fixture",
    format: "text",
    licence: "CC0 fixture",
    rateLimitMs: 0,
    sizeCapBytes: 1048576,
    unitBytes: 64,
    url: `http://127.0.0.1:${port}/item/{id}`,
    items: [
      { id: "one", name: "one.txt" },
      { id: "two", name: "two.txt" },
    ],
    ...(over ?? {}),
  };
  return { version: 1, askAboveBytes: 524288000, sources: [src] };
}

// The tool runs as a child process. The wait must be async: the fixture
// server lives in this process, so a blocking call would starve it.
function cli(args) {
  return new Promise((done) => {
    const child = spawn("node", [TOOL, ...args], { stdio: ["ignore", "pipe", "pipe"] });
    let stdout = "";
    let stderr = "";
    child.stdout.on("data", (chunk) => (stdout += chunk));
    child.stderr.on("data", (chunk) => (stderr += chunk));
    child.on("close", (status) => done({ status, stdout, stderr }));
  });
}

test("the shipped table holds the twelve sources with a rate, a cap and a licence", () => {
  const parsed = parseSources(readFileSync(SHIPPED, "utf8"));
  equal(parsed.sources.length, 12);
  equal(parsed.askAboveBytes, 500 * 1024 * 1024);
  const ids = parsed.sources.map((s) => s.id);
  equal(new Set(ids).size, 12);
  for (const s of parsed.sources) {
    ok(s.url.startsWith("https://"), `${s.id} url is https`);
    ok(s.rateLimitMs >= 0, `${s.id} has a rate limit`);
    ok(s.sizeCapBytes > 0, `${s.id} has a size cap`);
    ok(s.unitBytes > 0, `${s.id} states a unit size`);
    ok(s.licence.length > 0, `${s.id} states a licence`);
    ok(s.units.length > 0, `${s.id} has items`);
    for (const u of s.units) ok(u.url.startsWith("https://"), `${s.id} unit url is https`);
  }
});

test("a dry run over the shipped table prints twelve lines and writes nothing", async () => {
  const out = join(tmp(), "codex");
  const r = await cli(["--dry-run", "--out", out]);
  equal(r.status, 0, r.stderr);
  const lines = r.stdout.trim().split("\n");
  equal(lines.length, 12);
  for (const line of lines) match(line, /^PLAN [a-z0-9-]+ units=\d+ bytes=\d+ cap=\d+ rate_ms=\d+ licence=.+ url=https:\/\//);
  ok(!existsSync(out), "the dry run wrote nothing");
});

test("a fetch writes the file and its sidecar", async () => {
  const { srv, port } = await serve("rome codex bytes");
  try {
    const dir = tmp();
    const file = join(dir, "table.json");
    writeFileSync(file, JSON.stringify(table(port)));
    const out = join(dir, "codex");
    const r = await cli(["--sources", file, "--out", out]);
    equal(r.status, 0, r.stderr);
    equal((r.stdout.match(/^OK /gm) ?? []).length, 2);
    const body = readFileSync(join(out, "fixture", "one.txt"), "utf8");
    equal(body, "rome codex bytes");
    const side = JSON.parse(readFileSync(join(out, "fixture", "one.txt.src.json"), "utf8"));
    equal(side.source, "fixture");
    equal(side.url, `http://127.0.0.1:${port}/item/one`);
    equal(side.licence, "CC0 fixture");
    equal(side.sha256, createHash("sha256").update("rome codex bytes").digest("hex"));
    match(side.fetchedAt, /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d{3}Z$/);
  } finally {
    shut(srv);
  }
});

test("the size cap stops the source and leaves no half file", async () => {
  const { srv, port } = await serve("x".repeat(4096));
  try {
    const dir = tmp();
    const file = join(dir, "table.json");
    writeFileSync(file, JSON.stringify(table(port, { sizeCapBytes: 512 })));
    const out = join(dir, "codex");
    const r = await cli(["--sources", file, "--out", out]);
    equal(r.status, 1);
    match(r.stdout, /FAIL fixture one\.txt size cap 512 bytes exceeded/);
    ok(!existsSync(join(out, "fixture", "one.txt")), "the partial file is gone");
    ok(!existsSync(join(out, "fixture", "one.txt.src.json")), "no sidecar for a capped file");
  } finally {
    shut(srv);
  }
});

test("the rate limit is honoured between two units of one source", async () => {
  const { srv, hits, port } = await serve("rate limited bytes");
  try {
    const dir = tmp();
    const file = join(dir, "table.json");
    writeFileSync(file, JSON.stringify(table(port, { rateLimitMs: 300 })));
    const out = join(dir, "codex");
    const r = await cli(["--sources", file, "--out", out]);
    equal(r.status, 0, r.stderr);
    equal(hits.length, 2);
    ok(hits[1] - hits[0] >= 280, `the gap was ${hits[1] - hits[0]} ms`);
  } finally {
    shut(srv);
  }
});

test("a plan above 500 MB needs --yes", async () => {
  const { srv, port } = await serve("small body");
  try {
    const dir = tmp();
    const file = join(dir, "table.json");
    writeFileSync(file, JSON.stringify(table(port, { unitBytes: 600 * 1024 * 1024 })));
    const out = join(dir, "codex");
    const refused = await cli(["--sources", file, "--out", out]);
    equal(refused.status, 2);
    match(refused.stderr, /above the 524288000 byte ask: rerun with --yes/);
    ok(!existsSync(out), "the refused run wrote nothing");
    const allowed = await cli(["--sources", file, "--out", out, "--yes"]);
    equal(allowed.status, 0, allowed.stderr);
    match(allowed.stdout, /^BYTES planned_total=1258291200 sources=1$/m);
    ok(existsSync(join(out, "fixture", "one.txt")), "the allowed run wrote the file");
  } finally {
    shut(srv);
  }
});

test("a bad table is refused and the help names the flags", async () => {
  const dir = tmp();
  const file = join(dir, "table.json");
  writeFileSync(file, JSON.stringify({ sources: [{ id: "x", url: "ftp://x", rateLimitMs: 0, sizeCapBytes: 1, unitBytes: 1, licence: "c" }] }));
  const r = await cli(["--sources", file]);
  equal(r.status, 2);
  match(r.stderr, /url must be http or https/);
  const help = await cli(["--help"]);
  equal(help.status, 0);
  match(help.stdout, /--dry-run/);
  match(help.stdout, /--yes/);
});

test("run returns the counts of a table with no sources to fetch", async () => {
  const parsed = parseSources(readFileSync(SHIPPED, "utf8"));
  const lines = [];
  const r = await run({ sources: parsed.sources, dryRun: true, log: (line) => lines.push(line) });
  ok(r.ok);
  equal(r.dryRun, true);
  equal(lines.length, 12);
  deepStrictEqual(
    parsed.sources.map((s) => s.id),
    [
      "perseus",
      "gutenberg",
      "wikipedia",
      "wikidata",
      "pleiades",
      "itinere",
      "met",
      "artic",
      "commons",
      "polyhaven",
      "smiths",
      "latin-audio",
    ],
  );
});
