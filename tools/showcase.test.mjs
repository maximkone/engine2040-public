// Run: node --test tools/showcase.test.mjs
// showcase on fixture capture dirs: check-only passes a hashed SHOW1 clip and
// writes nothing, a tampered PNG fails the sha256 cross-check, a frameless
// manifest fails by default but passes with --allow-unhashed, bad row IDs and
// unsafe manifest.clip are refused, a faked-ffmpeg stitch writes mp4+gif
// beside the PNGs, a 12-frame duel clip validates, a showcase set with 0
// PNGs fails closed naming the set, and a bare --check-only checks the newest
// date set (exit 1 when no set exists).
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { deflateSync } from "node:zlib";
import {
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";

const HERE = dirname(fileURLToPath(import.meta.url));
const TOOL = join(HERE, "showcase.mjs");

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

// A fixture capture dir: N tiny PNGs plus a valid hashed manifest.
function fixtureClip(dir, seeds, manifestExtra = {}) {
  mkdirSync(dir, { recursive: true });
  const frames = [];
  seeds.forEach((seed, i) => {
    const name = `frame${String(i).padStart(4, "0")}.png`;
    const bytes = pngBytes(seed);
    writeFileSync(join(dir, name), bytes);
    frames.push({ file: name, sha256: sha256(bytes), bytes: bytes.length });
  });
  writeFileSync(join(dir, "manifest.json"), JSON.stringify({
    version: 1, seed: 2040, tape: "tapes/duel.json",
    rows: ["SHOW1"], sha: "abcdef1234567890", fps: 30, frames,
    ...manifestExtra,
  }));
  return frames;
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

function runTool(args, env = {}, cwd = undefined) {
  const r = spawnSync(process.execPath, [TOOL, ...args], {
    encoding: "utf8",
    env: { ...process.env, ...env },
    cwd,
  });
  return { code: r.status, out: r.stdout ?? "", err: r.stderr ?? "" };
}

test("check-only validates a hashed SHOW1 clip and writes nothing", () => {
  const clip = join(mkdtempSync(join(tmpdir(), "showcase-")), "duel");
  fixtureClip(clip, [1, 2, 3, 4]);
  const { code, out, err } = runTool(["--clip", clip, "--check-only"]);
  assert.equal(code, 0, `exit 0 on a valid clip, got ${code}: ${err}${out}`);
  assert.match(out, /showcase OK: 4 frames 8x8 @30fps/);
  assert.match(out, /rows SHOW1/);
  assert.equal(existsSync(join(clip, "duel.mp4")), false, "check-only writes no mp4");
  assert.equal(existsSync(join(clip, "duel.gif")), false, "check-only writes no gif");
});

test("a tampered PNG byte fails the sha256 cross-check and writes nothing", () => {
  const clip = join(mkdtempSync(join(tmpdir(), "showcase-")), "duel");
  fixtureClip(clip, [11, 12, 13, 14]);
  const victim = join(clip, "frame0001.png");
  const buf = readFileSync(victim);
  buf[buf.length - 1] ^= 0xff;
  writeFileSync(victim, buf);
  const { code, out, err } = runTool(["--clip", clip, "--check-only"]);
  assert.equal(code, 1, `tamper is exit 1, got ${code}: ${err}${out}`);
  assert.match(err, /frame bytes differ from manifest sha256: frame0001\.png/);
  assert.equal(existsSync(join(clip, "duel.mp4")), false, "tamper writes no mp4");
  assert.equal(existsSync(join(clip, "duel.gif")), false, "tamper writes no gif");
});

test("a frameless manifest fails by default and passes with --allow-unhashed", () => {
  const clip = join(mkdtempSync(join(tmpdir(), "showcase-")), "duel");
  mkdirSync(clip, { recursive: true });
  const bytes = pngBytes(31);
  writeFileSync(join(clip, "frame0000.png"), bytes);
  writeFileSync(join(clip, "manifest.json"), JSON.stringify({
    version: 1, seed: 2040, tape: "tapes/duel.json",
    rows: ["SHOW1"], sha: "abcdef1234567890", fps: 30,
  }));
  const denied = runTool(["--clip", clip, "--check-only"]);
  assert.equal(denied.code, 1, `frameless default is exit 1, got ${denied.code}: ${denied.err}${denied.out}`);
  assert.match(denied.err, /no frames\/files\/count coverage/);
  const allowed = runTool(["--clip", clip, "--check-only", "--allow-unhashed"]);
  assert.equal(allowed.code, 0, `frameless --allow-unhashed is exit 0, got ${allowed.code}: ${allowed.err}${allowed.out}`);
  assert.match(allowed.out, /showcase OK: 1 frames/);
});

test("a bad board row ID and an unsafe manifest.clip are refused", () => {
  const badRow = join(mkdtempSync(join(tmpdir(), "showcase-")), "duel");
  fixtureClip(badRow, [41, 42], { rows: ["bogus!!"] });
  const r1 = runTool(["--clip", badRow, "--check-only"]);
  assert.equal(r1.code, 1, `bad row is exit 1, got ${r1.code}: ${r1.err}${r1.out}`);
  assert.match(r1.err, /bad board row ID/);

  const escape = join(mkdtempSync(join(tmpdir(), "showcase-")), "duel");
  fixtureClip(escape, [51, 52], { clip: "../escape" });
  const r2 = runTool(["--clip", escape, "--check-only"]);
  assert.equal(r2.code, 1, `unsafe clip is exit 1, got ${r2.code}: ${r2.err}${r2.out}`);
  assert.match(r2.err, /unsafe manifest\.clip/);
});

test("a missing manifest and an empty clip dir fail loudly", () => {
  const clip = join(mkdtempSync(join(tmpdir(), "showcase-")), "duel");
  mkdirSync(clip, { recursive: true });
  const r1 = runTool(["--clip", clip, "--check-only"]);
  assert.equal(r1.code, 1, `missing manifest is exit 1, got ${r1.code}: ${r1.err}${r1.out}`);
  assert.match(r1.err, /manifest not found/);

  const empty = join(mkdtempSync(join(tmpdir(), "showcase-")), "empty");
  mkdirSync(empty, { recursive: true });
  writeFileSync(join(empty, "manifest.json"), JSON.stringify({
    version: 1, seed: 2040, tape: "tapes/duel.json",
    rows: ["SHOW1"], sha: "abcdef1234567890", fps: 30, frames: [],
  }));
  const r2 = runTool(["--clip", empty, "--check-only"]);
  assert.equal(r2.code, 1, `no PNGs is exit 1, got ${r2.code}: ${r2.err}${r2.out}`);
  assert.match(r2.err, /no PNG frames/);
});

test("a faked-ffmpeg stitch writes mp4+gif beside the PNGs and keeps sources", () => {
  const clip = join(mkdtempSync(join(tmpdir(), "showcase-")), "duel");
  fixtureClip(clip, [61, 62, 63, 64]);
  const pathSep = process.platform === "win32" ? ";" : ":";
  const env = { PATH: `${fakeFfmpegDir()}${pathSep}${process.env.PATH}` };
  const { code, out, err } = runTool(["--clip", clip], env);
  assert.equal(code, 0, `stitch is exit 0, got ${code}: ${err}${out}`);
  assert.match(out, /duel\.mp4.*duel\.gif/);
  for (const f of ["duel.mp4", "duel.gif"]) {
    assert.equal(existsSync(join(clip, f)), true, `${f} written beside the PNGs`);
  }
  assert.equal(existsSync(join(clip, "frame0000.png")), true, "sources are never deleted");
  assert.equal(existsSync(join(clip, "manifest.json")), true, "manifest is never deleted");
});

test("a 12-frame hashed duel clip validates", () => {
  // The 12-frame hashed shape of one arena duel clip: showcase must accept it.
  const clip = join(mkdtempSync(join(tmpdir(), "showcase-")), "duel");
  fixtureClip(clip, [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12]);
  const { code, out, err } = runTool(["--clip", clip, "--check-only", "--json"]);
  assert.equal(code, 0, `consumer-shape clip validates, got ${code}: ${err}${out}`);
  const summary = JSON.parse(out);
  assert.equal(summary.ok, true);
  assert.equal(summary.frames, 12);
  assert.deepEqual(summary.rows, ["SHOW1"]);
});

test("a bare --check-only checks the newest date set and fails closed with none", () => {
  // The gate leg: no --clip and no --set means "the newest capture set".
  const root = mkdtempSync(join(tmpdir(), "showcase-gate-"));
  const bare = runTool(["--check-only"], {}, root);
  assert.equal(bare.code, 1, "no showcase root is exit 1, got " + bare.code + ": " + bare.err + bare.out);
  assert.match(bare.err, /no showcase sets/);

  fixtureClip(join(root, "target", "showcase", "2026-10-01", "duel"), [1, 2]);
  fixtureClip(join(root, "target", "showcase", "2026-10-23", "orbit"), [3, 4, 5]);
  const { code, out, err } = runTool(["--check-only"], {}, root);
  assert.equal(code, 0, "newest set is exit 0, got " + code + ": " + err + out);
  assert.match(out, /showcase set OK: 3 PNGs/);
  assert.match(out, /2026-10-23/);
});

test("a showcase set with 0 PNGs fails closed naming the set", () => {
  const set = join(mkdtempSync(join(tmpdir(), "showcase-set-")), "2026-10-08");
  mkdirSync(join(set, "dataset"), { recursive: true });
  writeFileSync(join(set, "dataset", "shard-0000.bin"), Buffer.alloc(16));
  const dunk = runTool(["--set", set, "--check-only"]);
  assert.equal(dunk.code, 1, "empty set is exit 1, got " + dunk.code + ": " + dunk.err + dunk.out);
  assert.match(dunk.err, /showcase set has 0 PNGs/);
  assert.match(dunk.err, /2026-10-08/);
  const full = join(mkdtempSync(join(tmpdir(), "showcase-set-")), "2026-10-06");
  fixtureClip(join(full, "orbit"), [1, 2]);
  const ok = runTool(["--set", full, "--check-only"]);
  assert.equal(ok.code, 0, "set with PNGs is exit 0, got " + ok.code + ": " + ok.err + ok.out);
  assert.match(ok.out, /showcase set OK: 2 PNGs/);
});
