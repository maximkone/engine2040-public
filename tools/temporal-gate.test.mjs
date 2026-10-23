// Run: node --test tools/temporal-gate.test.mjs
// Pins the ENG20 ghosting metric: a still sequence scores exactly 0, a plain
// pan scores 0 (real motion is not stale content), a sequence with a repeated
// old frame scores 1.000 and names that frame and lag, the CLI splits those
// cases, and the result is deterministic.
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdirSync, mkdtempSync, renameSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";
import { crc32, deflateSync } from "node:zlib";
import {
  DEFAULT_BAR,
  LAG_MAX,
  MOVE_MIN,
  PAIR_BAR,
  frozenTaa,
  ghostScore,
  lumaFrame,
  meanAbsDiff,
  measureDir,
  measurePair,
  pairScore,
  panFrame,
  panSequence,
  selfcheck,
} from "./temporal-gate.mjs";

const TOOL = fileURLToPath(new URL("./temporal-gate.mjs", import.meta.url));
const W = 64;
const H = 48;
const SIG = Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]);

function chunk(type, data) {
  const head = Buffer.alloc(8);
  head.writeUInt32BE(data.length, 0);
  head.write(type, 4, "ascii");
  const crc = Buffer.alloc(4);
  crc.writeUInt32BE(crc32(Buffer.concat([head.subarray(4), data])) >>> 0, 0);
  return Buffer.concat([head, data, crc]);
}

/** An 8-bit RGB PNG of luma frames (grayscale triples, filter 0 rows). */
function encodeRgbPng(width, height, luma) {
  const raw = Buffer.alloc(height * (width * 3 + 1));
  for (let y = 0; y < height; y += 1) {
    for (let x = 0; x < width; x += 1) {
      const value = luma[y * width + x];
      const to = y * (width * 3 + 1) + 1 + x * 3;
      raw[to] = value;
      raw[to + 1] = value;
      raw[to + 2] = value;
    }
  }
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(width, 0);
  ihdr.writeUInt32BE(height, 4);
  ihdr[8] = 8;
  ihdr[9] = 2;
  return Buffer.concat([SIG, chunk("IHDR", ihdr), chunk("IDAT", deflateSync(raw)), chunk("IEND", Buffer.alloc(0))]);
}

function writeSequence(dir, width, height, frames) {
  mkdirSync(dir, { recursive: true });
  frames.forEach((frame, i) => {
    writeFileSync(join(dir, `frame_${String(i).padStart(6, "0")}.png`), encodeRgbPng(width, height, frame));
  });
}

function tempDir(t) {
  const dir = mkdtempSync(join(tmpdir(), "temporal-gate-"));
  t.after(() => rmSync(dir, { recursive: true, force: true }));
  return dir;
}

function run(args) {
  return spawnSync(process.execPath, [TOOL, ...args], { encoding: "utf8" });
}

/** The same pan with `alpha` of the previous frame blended in: a faint trail. */
function lagBlend(frames, alpha) {
  const out = [frames[0].slice()];
  for (let t = 1; t < frames.length; t += 1) {
    const frame = new Uint8Array(frames[t].length);
    for (let i = 0; i < frame.length; i += 1) {
      frame[i] = Math.round((1 - alpha) * frames[t][i] + alpha * frames[t - 1][i]);
    }
    out.push(frame);
  }
  return out;
}

test("a still sequence scores exactly 0", () => {
  const frames = Array.from({ length: 6 }, () => panFrame(W, H, 8));
  const r = ghostScore(frames, W, H);
  assert.equal(r.score, 0);
  assert.equal(r.frame, -1);
  assert.equal(r.lag, 1);
  assert.deepEqual(
    r.measured.map((m) => m.score),
    [0, 0, 0, 0, 0],
  );
  // A still frame is under MOVE_MIN, so its ratio is noise and is not scored.
  assert.ok(r.measured.every((m) => m.step <= MOVE_MIN));
});

test("a plain pan scores 0: real motion never looks like stale content", () => {
  const r = ghostScore(panSequence(W, H, 6), W, H);
  assert.equal(r.score, 0, `pan scored ${r.score}`);
  // Each frame's nearest match is its predecessor, so lag 1 on every frame.
  assert.ok(r.measured.every((m) => m.lag === 1));
});

test("a frame that repeats one two back scores 1.000 and names it", () => {
  const frames = panSequence(W, H, 6, 4, 3);
  const r = ghostScore(frames, W, H);
  // Frame 3 is a copy of frame 1: zero distance to t-2, a 4 px pan to t-1.
  assert.equal(r.score, 1, `stale score ${r.score}`);
  assert.equal(r.frame, 3);
  assert.equal(r.lag, 2);
  assert.equal(r.best, 0);
  assert.ok(r.step > 0);
  assert.ok(r.score > DEFAULT_BAR, `${r.score} must exceed the bar ${DEFAULT_BAR}`);
});

test("MOVE_MIN keeps a tiny step out of the ratio", () => {
  // Same stale copy, but a 1 px pan over a 512 px frame moves the mean luma
  // by under MOVE_MIN, so the metric refuses to divide and scores that frame
  // 0 rather than amplify noise into a ghost. The frame after it returns to
  // normal motion and now looks two back, which the bar absorbs.
  const frames = panSequence(512, 8, 6, 1, 3);
  const r = ghostScore(frames, 512, 8);
  assert.equal(r.measured[2].score, 0);
  assert.equal(r.measured[2].best, 0);
  assert.ok(r.measured[2].step <= MOVE_MIN, `step ${r.measured[2].step}`);
  assert.ok(r.score <= DEFAULT_BAR, `score ${r.score}`);
  assert.ok(r.measured.every((m) => m.step <= MOVE_MIN || m.score <= 1 / 3 + 1e-9));
});

test("ghostScore refuses a lag window under 2 and mismatched frames", () => {
  const frames = panSequence(W, H, 3);
  assert.throws(() => ghostScore(frames, W, H, { lagMax: 1 }), /lagMax/);
  assert.throws(() => ghostScore([], W, H), /no frames/);
  assert.throws(() => ghostScore([frames[0], frames[0].subarray(1)], W, H), /frame 1/);
});

test("meanAbsDiff is the mean of absolute differences and refuses a size mismatch", () => {
  const a = Uint8Array.from([0, 10, 20]);
  const b = Uint8Array.from([10, 10, 0]);
  assert.equal(meanAbsDiff(a, b), 10);
  assert.throws(() => meanAbsDiff(a, b.subarray(1)), /size mismatch/);
});

test("lumaFrame is BT.601 and refuses a short buffer", () => {
  const rgba = Buffer.from([255, 255, 255, 255, 0, 0, 0, 255]);
  assert.deepEqual(Array.from(lumaFrame(rgba, 2, 1)), [255, 0]);
  assert.throws(() => lumaFrame(rgba, 3, 1), /expected 12/);
});

test("the metric is deterministic across runs", () => {
  const frames = panSequence(W, H, 6, 4, 3);
  assert.deepEqual(ghostScore(frames, W, H), ghostScore(frames, W, H));
});

test("selfcheck passes at the default bar", () => {
  const r = selfcheck();
  assert.equal(r.ok, true, JSON.stringify(r));
  assert.equal(r.still, 0);
  assert.equal(r.pan, 0);
  assert.equal(r.lag, 2);
  assert.ok(r.stale > r.bar, `stale ${r.stale}`);
  assert.ok(LAG_MAX >= 2);
  assert.equal(r.pair_off, 0);
  assert.ok(r.pair > r.pair_bar, `pair ${r.pair} bar ${r.pair_bar}`);
  assert.equal(r.pair_bar, PAIR_BAR);
});

test("measureDir decodes PNGs and the CLI exits 0 on a still sequence", (t) => {
  const dir = tempDir(t);
  const frames = Array.from({ length: 6 }, () => panFrame(W, H, 8));
  writeSequence(dir, W, H, frames);
  const seen = measureDir(dir);
  assert.equal(seen.score, ghostScore(frames, W, H).score);
  assert.equal(seen.width, W);
  assert.equal(seen.height, H);
  assert.equal(seen.frame_count, 6);
  assert.equal(seen.worst, null);
  const r = run([dir]);
  assert.equal(r.status, 0, r.stderr);
  assert.match(r.stdout, /ghosting 0\.000 .*frames 6/);
});

test("the CLI exits 1 on a sequence with a repeated old frame", (t) => {
  const dir = tempDir(t);
  writeSequence(dir, W, H, panSequence(W, H, 6, 4, 3));
  const r = run([dir]);
  assert.equal(r.status, 1, r.stdout + r.stderr);
  assert.match(r.stdout, /ghosting 1\.000 .*frames 6 worst frame_000003\.png lag 2/);
  const json = run(["--json", dir]);
  assert.equal(json.status, 1, json.stderr);
  const parsed = JSON.parse(json.stdout);
  assert.equal(parsed.worst, "frame_000003.png");
  assert.equal(parsed.score, 1);
  assert.equal(parsed.lag, 2);
});

test("the CLI raises the bar and the same stale sequence passes under it", (t) => {
  const dir = tempDir(t);
  writeSequence(dir, W, H, panSequence(W, H, 6, 4, 3));
  const r = run(["--bar", "1.0", dir]);
  assert.equal(r.status, 0, r.stderr);
  assert.match(r.stdout, /bar 1\.000/);
});

test("a missing directory or a bad bar is a usage error, exit 2", (t) => {
  const dir = tempDir(t);
  const missing = run([join(dir, "missing")]);
  assert.equal(missing.status, 2);
  assert.match(missing.stderr, /no frame_\*\.png/);
  const bad = run(["--bar", "nope", dir]);
  assert.equal(bad.status, 2);
  assert.match(bad.stderr, /bad bar/);
  const noPair = run(["--pair", dir]);
  assert.equal(noPair.status, 2);
  assert.match(noPair.stderr, /usage:/);
});

test("pairScore: the off reference against itself separates nothing", () => {
  const off = panSequence(W, H, 8);
  const r = pairScore(off, off, W, H);
  assert.equal(r.score, 0);
  assert.equal(r.frame, -1);
  assert.equal(r.ghost_share, 0);
  assert.ok(r.moved_share > 0, `moved ${r.moved_share}`);
});

test("pairScore names the frozen-history ghost and separates it from the clean run", () => {
  const off = panSequence(W, H, 8);
  const on = frozenTaa(off, 0.9);
  const r = pairScore(off, on, W, H);
  assert.ok(r.score > 0.5, `separation ${r.score}`);
  assert.ok(r.score > PAIR_BAR, `${r.score} over ${PAIR_BAR}`);
  assert.ok(r.ghost_share > 0.9, `ghost share ${r.ghost_share}`);
  assert.equal(r.lag, 1);
  assert.ok(r.frame > 0);
  const worst = r.measured.find((m) => m.index === r.frame);
  assert.equal(worst.score, r.score);
});

test("a 10% old-frame trail is caught by the pair while the single score reads 0", () => {
  const off = panSequence(W, H, 8);
  const on = lagBlend(off, 0.1);
  assert.equal(ghostScore(on, W, H).score, 0, "the frame-level score misses the trail");
  const r = pairScore(off, on, W, H);
  assert.ok(Math.abs(r.score - 0.1) < 0.02, `alpha ${r.score}`);
  assert.ok(r.score > PAIR_BAR, `${r.score} over ${PAIR_BAR}`);
  assert.equal(r.ghost_share, 0, "a 10% trail is not mostly old content");
});

test("measurePair aligns by name and the CLI scores the pair apart", (t) => {
  const offDir = tempDir(t);
  const onDir = tempDir(t);
  const off = panSequence(W, H, 8);
  writeSequence(offDir, W, H, off);
  writeSequence(onDir, W, H, frozenTaa(off, 0.9));
  const r = measurePair(offDir, onDir);
  assert.equal(r.mode, "pair");
  assert.equal(r.frame_count, 8);
  assert.equal(r.off, offDir);
  assert.equal(r.on, onDir);
  assert.ok(r.score > PAIR_BAR, `separation ${r.score}`);
  assert.match(r.worst, /^frame_\d+\.png$/);
  const cli = run(["--pair", offDir, onDir]);
  assert.equal(cli.status, 1, cli.stdout + cli.stderr);
  assert.match(
    cli.stdout,
    /pair separation 0\.\d{3} .*frames 8 .*moved 0\.\d{3} ghost [01]\.\d{3}/,
  );
  const json = run(["--json", "--pair", offDir, onDir]);
  assert.equal(json.status, 1, json.stderr);
  const parsed = JSON.parse(json.stdout);
  assert.equal(parsed.mode, "pair");
  assert.equal(parsed.separation, parsed.score);
  assert.equal(parsed.bar, PAIR_BAR);
  assert.ok(parsed.separation > PAIR_BAR);
  const pass = run(["--pair", "--bar", "1.0", offDir, onDir]);
  assert.equal(pass.status, 0, pass.stderr);
});

test("a pair must align by frame count, size and name", (t) => {
  const off = panSequence(W, H, 4);
  assert.throws(() => pairScore(off, off.slice(1), W, H), /frame count/);
  assert.throws(() => pairScore([off[0]], [off[0]], W, H), /at least 2/);
  assert.throws(
    () => pairScore(off, off.map((frame, i) => (i === 1 ? frame.subarray(1) : frame)), W, H),
    /on frame 1/,
  );
  const offDir = tempDir(t);
  const onDir = tempDir(t);
  writeSequence(offDir, W, H, off);
  writeSequence(onDir, W, H, off);
  renameSync(join(onDir, "frame_000001.png"), join(onDir, "frame_000009.png"));
  assert.throws(() => measurePair(offDir, onDir), /misaligned/);
});

test("measurePair refuses captures of different sizes", (t) => {
  const offDir = tempDir(t);
  const onDir = tempDir(t);
  writeSequence(offDir, W, H, panSequence(W, H, 4));
  writeSequence(onDir, W + 1, H, panSequence(W + 1, H, 4));
  assert.throws(() => measurePair(offDir, onDir), /pair size/);
});
