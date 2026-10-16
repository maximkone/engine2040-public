// Run: node --test tools/weekly-clip.test.mjs
// weekly-clip on fixture capture dirs: stitch skip without ffmpeg, NOOP on an
// unchanged picture, one order row across two --deliver runs, printed path.
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { deflateSync } from "node:zlib";
import {
  copyFileSync,
  existsSync,
  mkdirSync,
  mkdtempSync as mkdtempRaw,
  rmSync,
  readFileSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { HERE } from "./web-search.mjs";

// Remove every temp folder this file makes when it exits (test temp folders piled up in %TEMP%).
const madeDirs = [];
const mkdtempSync = (prefix, options) => { const dir = mkdtempRaw(prefix, options); madeDirs.push(dir); return dir; };
process.on("exit", () => { for (const dir of madeDirs) try { rmSync(dir, { recursive: true, force: true }); } catch { /* still in use: leave it */ } });
const TOOL = join(HERE, "weekly-clip.mjs");
const SHOWCASE = join(HERE, "showcase.mjs");
const WEEK = "2026W40";
const PREV = "2026W39";

// Minimal valid PNGs: 8x8 solid RGB with one pixel keyed by seed, so equal
// seeds give byte-identical frames and different seeds move the picture.
function crc32(buf) {
  let table = crc32.table;
  if (!table) {
    table = crc32.table = new Int32Array(256);
    for (let n = 0; n < 256; n += 1) {
      let c = n;
      for (let k = 0; k < 8; k += 1) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
      table[n] = c;
    }
  }
  let c = -1;
  for (const b of buf) c = table[(c ^ b) & 0xff] ^ (c >>> 8);
  return (c ^ -1) >>> 0;
}

function chunk(type, data) {
  const len = Buffer.alloc(4);
  len.writeUInt32BE(data.length);
  const body = Buffer.concat([Buffer.from(type, "ascii"), data]);
  const crc = Buffer.alloc(4);
  crc.writeUInt32BE(crc32(body));
  return Buffer.concat([len, body, crc]);
}

function pngBytes(seed) {
  const w = 8;
  const h = 8;
  const raw = Buffer.alloc(h * (1 + w * 3));
  for (let y = 0; y < h; y += 1) {
    raw[y * (1 + w * 3)] = 0;
    for (let x = 0; x < w; x += 1) {
      const o = y * (1 + w * 3) + 1 + x * 3;
      raw[o] = (seed * 37 + x * 11) & 0xff;
      raw[o + 1] = (seed * 91 + y * 13) & 0xff;
      raw[o + 2] = (seed * 17 + x + y) & 0xff;
    }
  }
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(w, 0);
  ihdr.writeUInt32BE(h, 4);
  ihdr[8] = 8;
  ihdr[9] = 2;
  const magic = Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]);
  return Buffer.concat([magic, chunk("IHDR", ihdr), chunk("IDAT", deflateSync(raw)), chunk("IEND", Buffer.alloc(0))]);
}

function sha256(buf) {
  return createHash("sha256").update(buf).digest("hex");
}

// A fixture capture dir: 12 tiny PNGs plus a valid hashed manifest.
function fixtureClip(dir, seedBase) {
  mkdirSync(dir, { recursive: true });
  const frames = [];
  for (let i = 0; i < 12; i += 1) {
    const name = `frame${String(i).padStart(4, "0")}.png`;
    const bytes = pngBytes(seedBase + i);
    writeFileSync(join(dir, name), bytes);
    frames.push({ file: name, sha256: sha256(bytes), bytes: bytes.length });
  }
  writeFileSync(join(dir, "manifest.json"), JSON.stringify({
    version: 1, clip: dir.split(/[/\\]/).pop(), seed: 2040, tape: "live",
    rows: ["SHOW1"], sha: "abcdef1234567890", fps: 30, frames,
  }));
  return join(dir, "frame0011.png");
}

// Fake ffmpeg: answers -version, otherwise writes bytes to the last arg.
function fakeFfmpegDir() {
  const dir = mkdtempSync(join(tmpdir(), "ffmpeg-fake-"));
  writeFileSync(join(dir, "ffmpeg-fake.mjs"), `const a = process.argv.slice(2);\nif (a.includes("-version")) { console.log("ffmpeg version test-fake"); process.exit(0); }\nconst { writeFileSync } = await import("node:fs");\nwriteFileSync(a[a.length - 1], Buffer.alloc(64, 7));\n`);
  const cmd = process.platform === "win32" ? "ffmpeg.cmd" : "ffmpeg";
  const body = process.platform === "win32"
    ? `@echo off\n"${process.execPath}" "${join(dir, "ffmpeg-fake.mjs")}" %*\n`
    : `#!/bin/sh\nexec "${process.execPath}" "${join(dir, "ffmpeg-fake.mjs")}" "$@"\n`;
  writeFileSync(join(dir, cmd), body, { mode: 0o755 });
  return dir;
}

// Fake empire: appends one order row to WEEKLY_CLIP_ORDERS_FILE, prints the id.
function fakeEmpire() {
  const dir = mkdtempSync(join(tmpdir(), "empire-fake-"));
  const path = join(dir, "empire.mjs");
  writeFileSync(path, `import { appendFileSync, existsSync, readFileSync, writeFileSync } from "node:fs";\nconst file = process.env.WEEKLY_CLIP_ORDERS_FILE;\nconst what = process.argv[4] ?? "";\nconst old = existsSync(file) ? readFileSync(file, "utf8") : "";\nconst head = "order_id,from_repo,product,brief,status,delivered_path,adopted,date,adopted_commit";\nconst rows = old.split(/\\r?\\n/).filter((l) => l.trim());\nlet max = 0;\nfor (const l of rows.slice(1)) { const m = /^O-(\\d+),/.exec(l); if (m) max = Math.max(max, Number(m[1])); }\nconst id = "O-" + String(max + 1).padStart(3, "0");\nconst row = [id, "marketing-studio", "order:fake", what + " | by engine2040-pilot", "open", "", "no", "2026-10-04", ""].join(",");\nif (!old.trim()) writeFileSync(file, head + "\\n" + row + "\\n");\nelse appendFileSync(file, row + "\\n");\nconsole.log("ordered " + id + " from engine2040 for marketing-studio: " + file);\n`);
  return path;
}

function sandbox() {
  const root = mkdtempSync(join(tmpdir(), "weekly-clip-"));
  mkdirSync(join(root, "tools"), { recursive: true });
  copyFileSync(TOOL, join(root, "tools", "weekly-clip.mjs"));
  if (existsSync(SHOWCASE)) copyFileSync(SHOWCASE, join(root, "tools", "showcase.mjs"));
  // showcase.mjs imports these two; the stitch step runs it from the sandbox.
  for (const dep of ["hash-util.mjs", "argv.mjs"]) copyFileSync(join(HERE, dep), join(root, "tools", dep));
  mkdirSync(join(root, "target", "clips"), { recursive: true });
  return root;
}

function runTool(root, args, env = {}) {
  const r = spawnSync(process.execPath, [join(root, "tools", "weekly-clip.mjs"), ...args], {
    encoding: "utf8",
    cwd: root,
    env: { ...process.env, ...env },
  });
  return { code: r.status, out: r.stdout ?? "", err: r.stderr ?? "" };
}

test("missing arena.exe exits 3 with the exact build command (never runs cargo)", () => {
  const root = sandbox();
  const { code, out, err } = runTool(root, ["--week", WEEK, "--clip", join(root, "nope")]);
  assert.equal(code, 2, "empty fixture dir is a usage error");
  assert.match(err + out, /no PNGs/);
  const root2 = sandbox();
  const r2 = runTool(root2, ["--week", WEEK]);
  assert.equal(r2.code, 3, `real path without arena.exe exits 3, got ${r2.code}: ${r2.err}${r2.out}`);
  assert.match(r2.err, /cargo build -p engine-game --bin arena/);
});

test("stitch is skipped when ffmpeg is absent and says so", () => {
  const root = sandbox();
  const clip = join(root, "target", "clips", WEEK);
  fixtureClip(clip, 5);
  const missing = join(root, "definitely-no-ffmpeg");
  const { code, out } = runTool(root, ["--week", WEEK, "--clip", clip], { WEEKLY_CLIP_FFMPEG: missing });
  assert.equal(code, 0, out);
  assert.match(out, /STITCH skipped: ffmpeg not found/);
  const clipJson = JSON.parse(readFileSync(join(clip, "clip.json"), "utf8"));
  assert.equal(clipJson.stitch, "skipped");
  assert.equal(clipJson.mp4_sha256, null);
  assert.equal(clipJson.seed, 2040);
  assert.equal(clipJson.tape, "live");
  assert.deepEqual(clipJson.rows, ["SHOW1"]);
});

test("unchanged picture is a NOOP and delivers nothing", () => {
  const root = sandbox();
  const prev = join(root, "target", "clips", PREV);
  fixtureClip(prev, 7);
  const clip = join(root, "target", "clips", WEEK);
  fixtureClip(clip, 7);
  const missing = join(root, "definitely-no-ffmpeg");
  const orders = join(root, "orders.csv");
  const { code, out } = runTool(root, ["--week", WEEK, "--clip", clip, "--deliver"], {
    WEEKLY_CLIP_FFMPEG: missing,
    WEEKLY_CLIP_ORDERS_FILE: orders,
  });
  assert.equal(code, 0, out);
  assert.match(out, new RegExp(`NOOP picture unchanged since ${PREV}`));
  assert.equal(existsSync(orders), false, "NOOP writes no order row");
});

test("the order row is written once, a second run adds no duplicate, the printed path is delivered", () => {
  const root = sandbox();
  const clip = join(root, "target", "clips", WEEK);
  fixtureClip(clip, 21);
  const studio = join(root, "studio");
  const orders = join(root, "orders.csv");
  const pathSep = process.platform === "win32" ? ";" : ":";
  const env = {
    PATH: `${fakeFfmpegDir()}${pathSep}${process.env.PATH}`,
    WEEKLY_CLIP_EMPIRE: fakeEmpire(),
    WEEKLY_CLIP_STUDIO_DIR: studio,
    WEEKLY_CLIP_ORDERS_FILE: orders,
  };
  const first = runTool(root, ["--week", WEEK, "--clip", clip, "--deliver"], env);
  assert.equal(first.code, 0, first.out + first.err);
  const delivered = /DELIVERED (\S+)/.exec(first.out)?.[1];
  assert.equal(delivered, `from-engine2040/${WEEK}`, `printed path is the studio-relative one: ${first.out}`);
  for (const f of [`${WEEK}.mp4`, `${WEEK}.gif`, "clip.json"]) {
    assert.equal(existsSync(join(studio, "from-engine2040", WEEK, f)), true, `${f} landed in the studio folder`);
  }
  const rows1 = readFileSync(orders, "utf8").split("\n").filter((l) => l.trim());
  assert.equal(rows1.length, 2, `header + one row, got:\n${rows1.join("\n")}`);
  assert.match(rows1[1], new RegExp(`^O-001,marketing-studio,.*delivered,from-engine2040/${WEEK},`));

  const second = runTool(root, ["--week", WEEK, "--clip", clip, "--deliver"], env);
  assert.equal(second.code, 0, second.out + second.err);
  const rows2 = readFileSync(orders, "utf8").split("\n").filter((l) => l.trim());
  assert.equal(rows2.length, 2, `second run adds no duplicate row, got:\n${rows2.join("\n")}`);
});

test("a moved picture writes clip.json and waits for --deliver", () => {
  const root = sandbox();
  const prev = join(root, "target", "clips", PREV);
  fixtureClip(prev, 7);
  const clip = join(root, "target", "clips", WEEK);
  fixtureClip(clip, 99);
  const missing = join(root, "definitely-no-ffmpeg");
  const { code, out } = runTool(root, ["--week", WEEK, "--clip", clip], { WEEKLY_CLIP_FFMPEG: missing });
  assert.equal(code, 0, out);
  assert.match(out, /MOVED: moved/);
  assert.match(out, /READY .*\(not delivered: pass --deliver\)/);
  const clipJson = JSON.parse(readFileSync(join(clip, "clip.json"), "utf8"));
  assert.equal(clipJson.look_stage, "moved");
  assert.equal(clipJson.frames, 12);
  assert.equal(clipJson.duration_s, 0.4);
});
