// fetch-content.test.mjs - GS21 accept run against a local http fixture.
// No real hosts: two files served on 127.0.0.1, md5 verified, sidecars
// written, second run skips, wrong md5 deletes + fails, dry-run prints.
import { deepStrictEqual, equal, match, ok } from "node:assert";
import test from "node:test";
import { createServer } from "node:http";
import { mkdtempSync, readFileSync, writeFileSync, existsSync } from "node:fs";
import { join } from "node:path";
import { tmpdir } from "node:os";
import { createHash } from "node:crypto";
import { spawn, spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";
import { md5Of, parseList, run, sidecarFor } from "./fetch-content.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));
const TOOL = resolve(HERE, "fetch-content.mjs");
const md5 = (s) => createHash("md5").update(s).digest("hex");

function shut(srv) {
  try {
    if (srv.closeAllConnections) srv.closeAllConnections();
  } catch {}
  srv.close();
}

function serve(files) {
  let hits = 0;
  const srv = createServer((req, res) => {
    hits++;
    const body = files[req.url];
    if (body === undefined) {
      res.writeHead(404);
      res.end("no");
      return;
    }
    res.writeHead(200, { "Content-Type": "application/octet-stream" });
    res.end(body);
  });
  return new Promise((done) => {
    srv.listen(0, "127.0.0.1", () => done({ srv, port: srv.address().port, hits: () => hits }));
  });
}

function runTool(args) {
  return new Promise((resolve, reject) => {
    const child = spawn("node", args, {});
    let stdout = "";
    let stderr = "";
    child.stdout.on("data", (d) => {
      stdout += d;
    });
    child.stderr.on("data", (d) => {
      stderr += d;
    });
    child.on("error", reject);
    child.on("close", (status) => resolve({ status, stdout, stderr }));
  });
}

test("parseList skips comments and checks md5 shape", () => {
  const rows = parseList("# c\n\nurl a http://x/a d41d8cd98f00b204e9800998ecf8427e CC0-1.0 bob\n");
  equal(rows.length, 1);
  equal(rows[0].id, "a");
  try {
    parseList("url b http://x/b notmd5\n");
    ok(false, "bad md5 must throw");
  } catch (e) {
    match(String(e), /bad md5/);
  }
});

test("a CC-BY row keeps its attribution text and is refused without one", () => {
  const text = 'url arch http://x/arch.glb d41d8cd98f00b204e9800998ecf8427e CC-BY-3.0 Icawfype "Roman Arch" by Icawfype, CC-BY 3.0, via Poly Pizza\n';
  const [row] = parseList(text);
  equal(row.license, "CC-BY-3.0");
  equal(row.attribution, '"Roman Arch" by Icawfype, CC-BY 3.0, via Poly Pizza');
  deepStrictEqual(sidecarFor(row, row.url), {
    url: "http://x/arch.glb",
    author: "Icawfype",
    license: "CC-BY-3.0",
    retrieved: new Date().toISOString().slice(0, 10),
    attribution: '"Roman Arch" by Icawfype, CC-BY 3.0, via Poly Pizza',
  });
  try {
    parseList("url arch http://x/arch.glb d41d8cd98f00b204e9800998ecf8427e CC-BY-3.0 Icawfype\n");
    ok(false, "a CC-BY row without attribution must throw");
  } catch (e) {
    match(String(e), /needs attribution text/);
  }
  const cc0 = parseList("url crate http://x/c.glb d41d8cd98f00b204e9800998ecf8427e CC0-1.0 Quaternius\n");
  equal(cc0[0].attribution, "", "CC0 rows need no attribution");
});

test("downloads two files, writes sidecars, second run skips", async () => {
  const a = "alpha-bytes-1";
  const b = "beta-bytes-22";
  const { srv, port, hits } = await serve({ "/a.bin": a, "/b.bin": b });
  try {
    const dir = mkdtempSync(join(tmpdir(), "gs21-"));
    const list = join(dir, "list.txt");
    writeFileSync(
      list,
      `url filea http://127.0.0.1:${port}/a.bin ${md5(a)} CC0-1.0 alice\n` +
        `url fileb http://127.0.0.1:${port}/b.bin ${md5(b)} CC-BY-4.0 bob "B" by Bob, CC-BY 4.0, http://example.test/b\n`,
    );
    const out = join(dir, "out");
    const r1 = await run({ listFile: list, outDir: out, rateMs: 0 });
    deepStrictEqual([r1.ok, r1.downloaded, r1.skipped], [true, 2, 0]);
    for (const [id, body, lic] of [["filea", a, "CC0-1.0"], ["fileb", b, "CC-BY-4.0"]]) {
      const dest = join(out, "url", id, id === "filea" ? "a.bin" : "b.bin");
      ok(existsSync(dest), `downloaded ${dest}`);
      equal(md5Of(readFileSync(dest)), md5(body));
      const side = JSON.parse(readFileSync(join(out, "url", id, `${id}.license.json`), "utf8"));
      equal(side.license, lic);
      equal(side.attribution, lic.startsWith("CC-BY") ? '"B" by Bob, CC-BY 4.0, http://example.test/b' : undefined);
      ok(side.url.includes("127.0.0.1"), "sidecar keeps url");
      ok(/^\d{4}-\d{2}-\d{2}$/.test(side.retrieved), "sidecar has date");
    }
    const r2 = await run({ listFile: list, outDir: out, rateMs: 0 });
    deepStrictEqual([r2.ok, r2.downloaded, r2.skipped], [true, 0, 2]);
    equal(hits(), 2, "second run makes no requests when md5 matches");
  } finally {
    shut(srv);
  }
});

test("wrong md5 deletes the file and fails", async () => {
  const { srv, port } = await serve({ "/a.bin": "real-bytes" });
  try {
    const dir = mkdtempSync(join(tmpdir(), "gs21-bad-"));
    const list = join(dir, "list.txt");
    writeFileSync(list, `url filea http://127.0.0.1:${port}/a.bin ${md5("other-bytes")} CC0-1.0 alice\n`);
    const out = join(dir, "out");
    const codeBefore = process.exitCode ?? 0;
    const r = await run({ listFile: list, outDir: out, rateMs: 0 });
    equal(r.ok, false);
    equal(r.bad, 1);
    ok(!existsSync(join(out, "url", "filea", "a.bin")), "bad file deleted");
    equal(process.exitCode ?? 0, codeBefore, "library must not set process exit code");
  } finally {
    shut(srv);
  }
});

test("cli wrong md5 exits 1 and deletes the file", async () => {
  const { srv, port } = await serve({ "/a.bin": "real-bytes" });
  try {
    const dir = mkdtempSync(join(tmpdir(), "gs21-cli-bad-"));
    const list = join(dir, "list.txt");
    writeFileSync(list, `url filea http://127.0.0.1:${port}/a.bin ${md5("other-bytes")} CC0-1.0 alice\n`);
    const out = join(dir, "out");
    const r = await runTool([TOOL, "--list", list, "--out", out, "--rate-ms", "0"]);
    equal(r.status, 1, r.stderr);
    match(r.stderr, /md5/);
    ok(!existsSync(join(out, "url", "filea", "a.bin")), "bad file deleted");
  } finally {
    shut(srv);
  }
});

test("--dry-run prints total bytes and every license, writes nothing", async () => {
  const a = "alpha-bytes-1";
  const b = "beta-bytes-22";
  const { srv, port } = await serve({ "/a.bin": a, "/b.bin": b });
  try {
    const dir = mkdtempSync(join(tmpdir(), "gs21-dry-"));
    const list = join(dir, "list.txt");
    writeFileSync(
      list,
      `url filea http://127.0.0.1:${port}/a.bin ${md5(a)} CC0-1.0 alice\n` +
        `url fileb http://127.0.0.1:${port}/b.bin ${md5(b)} CC-BY-4.0 bob "B" by Bob, CC-BY 4.0, http://example.test/b\n`,
    );
    const out = join(dir, "out");
    const r = await runTool([TOOL, "--list", list, "--out", out, "--dry-run", "--rate-ms", "0"]);
    equal(r.status, 0, r.stderr);
    match(r.stdout, new RegExp(`total_bytes=${a.length + b.length}`));
    match(r.stdout, /CC0-1\.0/);
    match(r.stdout, /CC-BY-4\.0/);
    match(r.stdout, /ATTRIBUTION url fileb "B" by Bob, CC-BY 4\.0, http:\/\/example\.test\/b/);
    ok(!existsSync(join(out, "url", "filea", "a.bin")), "dry-run writes nothing");
  } finally {
    shut(srv);
  }
});

test("a bare list path works like --list, even after --out", async () => {
  const a = "alpha-bytes-1";
  const { srv, port } = await serve({ "/a.bin": a });
  try {
    const dir = mkdtempSync(join(tmpdir(), "gs22-bare-"));
    const list = join(dir, "list.txt");
    writeFileSync(list, `url filea http://127.0.0.1:${port}/a.bin ${md5(a)} CC0-1.0 alice\n`);
    const out = join(dir, "out");
    const r1 = await runTool([TOOL, "--dry-run", list, "--rate-ms", "0"]);
    equal(r1.status, 0, r1.stderr);
    match(r1.stdout, new RegExp(`total_bytes=${a.length}`));
    const r2 = await runTool([TOOL, "--out", out, "--dry-run", list, "--rate-ms", "0"]);
    equal(r2.status, 0, r2.stderr);
    match(r2.stdout, new RegExp(`total_bytes=${a.length}`));
    const r3 = await runTool([TOOL, "--dry-run"]);
    equal(r3.status, 2, "no list path still exits 2");
  } finally {
    shut(srv);
  }
});

test("--help names list, sidecar and dry-run", () => {
  const r = spawnSync("node", [TOOL, "--help"], { encoding: "utf8" });
  equal(r.status, 0);
  match(r.stdout, /--list/);
  match(r.stdout, /license\.json/);
  match(r.stdout, /--dry-run/);
});
