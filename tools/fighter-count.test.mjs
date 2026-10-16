// Run: node --test tools/fighter-count.test.mjs
// fighter-count pins a repeatable number for J1: fighter = 0 with a nonzero
// box count on the staged showcase set. Pure pixel rules plus one golden
// sample from the real frames; synthetic PNGs below are built by the local
// encoder so exact counts are asserted without fixtures.
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { deflateSync } from "node:zlib";
import {
  BOX_RULE,
  FIGHTER_RULE,
  PNG_SIGNATURE,
  RULE_VERSION,
  classifyPixel,
  countDir,
  countPngBuffer,
  decodePng,
  findNewestShowcaseDir,
  helpText,
  isBoxPixel,
  isFighterPixel,
  main,
  parseArgs,
} from "./fighter-count.mjs";

const TOOL = fileURLToPath(new URL("./fighter-count.mjs", import.meta.url));
const GOLDEN = fileURLToPath(
  new URL("../target/showcase/2026-10-12/staged-fight/frame_000000.png", import.meta.url),
);

const RED = [200, 40, 40, 255];
const DARK_RED = [128, 32, 32, 255];
const BLUE = [60, 150, 220, 255];
const DOME = [128, 112, 96, 255];

const CRC_TABLE = (() => {
  const t = new Uint32Array(256);
  for (let n = 0; n < 256; n += 1) {
    let c = n;
    for (let k = 0; k < 8; k += 1) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    t[n] = c >>> 0;
  }
  return t;
})();

function crc32(buf) {
  let c = 0xffffffff;
  for (const byte of buf) c = CRC_TABLE[(c ^ byte) & 255] ^ (c >>> 8);
  return (c ^ 0xffffffff) >>> 0;
}

function chunk(type, data) {
  const len = Buffer.alloc(4);
  len.writeUInt32BE(data.length);
  const body = Buffer.concat([Buffer.from(type, "ascii"), data]);
  const crc = Buffer.alloc(4);
  crc.writeUInt32BE(crc32(body));
  return Buffer.concat([len, body, crc]);
}

/** Build a small PNG. at(x, y) returns [r, g, b, a]. */
function encodePng(width, height, at, { colorType = 6, interlace = 0 } = {}) {
  const ch = colorType === 6 ? 4 : 3;
  const stride = width * ch;
  const raw = Buffer.alloc(height * (stride + 1));
  for (let y = 0; y < height; y += 1) {
    raw[y * (stride + 1)] = 0;
    for (let x = 0; x < width; x += 1) {
      const px = at(x, y);
      const o = y * (stride + 1) + 1 + x * ch;
      raw[o] = px[0];
      raw[o + 1] = px[1];
      raw[o + 2] = px[2];
      if (ch === 4) raw[o + 3] = px[3] === undefined ? 255 : px[3];
    }
  }
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(width, 0);
  ihdr.writeUInt32BE(height, 4);
  ihdr[8] = 8;
  ihdr[9] = colorType;
  ihdr[12] = interlace;
  return Buffer.concat([
    PNG_SIGNATURE,
    chunk("IHDR", ihdr),
    chunk("IDAT", deflateSync(raw)),
    chunk("IEND", Buffer.alloc(0)),
  ]);
}

test("rules carry a pinned version and documented text", () => {
  assert.equal(RULE_VERSION, 1);
  assert.match(BOX_RULE, /R-G/);
  assert.match(FIGHTER_RULE, /B-R/);
});

test("box rule pins dressing-box red and skips dome browns", () => {
  assert.equal(isBoxPixel(200, 40, 40), true);
  assert.equal(isBoxPixel(128, 32, 32), true);
  assert.equal(isBoxPixel(128, 112, 96), false);
  assert.equal(isBoxPixel(60, 150, 220), false);
});

test("fighter rule pins kit blue and skips reds and browns", () => {
  assert.equal(isFighterPixel(60, 150, 220), true);
  assert.equal(isFighterPixel(200, 40, 40), false);
  assert.equal(isFighterPixel(128, 112, 96), false);
  assert.equal(isFighterPixel(208, 160, 144), false);
});

test("classifyPixel routes box, fighter, and other", () => {
  assert.equal(classifyPixel(200, 40, 40), "box");
  assert.equal(classifyPixel(60, 150, 220), "fighter");
  assert.equal(classifyPixel(128, 112, 96), "other");
});

test("box and fighter rules are disjoint over a coarse grid", () => {
  for (let r = 0; r < 256; r += 51) {
    for (let g = 0; g < 256; g += 51) {
      for (let b = 0; b < 256; b += 51) {
        assert.equal(isBoxPixel(r, g, b) && isFighterPixel(r, g, b), false);
      }
    }
  }
});

test("countPngBuffer reports exact synthetic counts", () => {
  const png = encodePng(4, 4, (x, y) => {
    if (y < 2 && x < 2) return RED;
    if (y >= 2 && x >= 2) return BLUE;
    return DOME;
  });
  const c = countPngBuffer(png, "syn.png");
  assert.equal(c.box, 4);
  assert.equal(c.fighter, 4);
  assert.equal(c.total, 16);
});

test("countPngBuffer decodes RGB (color type 2) frames", () => {
  const png = encodePng(2, 1, (x) => (x === 0 ? RED : DOME), { colorType: 2 });
  const c = countPngBuffer(png, "rgb.png");
  assert.equal(c.box, 1);
  assert.equal(c.fighter, 0);
});

test("countPngBuffer is deterministic on the same bytes", () => {
  const png = encodePng(3, 3, (x, y) => ((x + y) % 2 === 0 ? RED : BLUE));
  assert.deepEqual(countPngBuffer(png, "a.png"), { ...countPngBuffer(png, "a.png") });
});

test("decode rejects bad signature, truncation, layout", () => {
  const good = encodePng(2, 2, () => DOME);
  assert.throws(() => decodePng(Buffer.from("hello"), "bad.png"), /signature/);
  assert.throws(() => decodePng(good.subarray(0, 20), "cut.png"), /truncated|IHDR|data/);
  const gray = Buffer.from(good);
  gray[25] = 0;
  assert.throws(() => decodePng(gray, "gray.png"), /color type/);
  const laced = Buffer.from(good);
  laced[28] = 1;
  assert.throws(() => decodePng(laced, "laced.png"), /interlace/);
});

test("countDir sums frames and rejects an empty dir", () => {
  const dir = mkdtempSync(join(tmpdir(), "fc-"));
  writeFileSync(join(dir, "a.png"), encodePng(2, 2, () => RED));
  writeFileSync(join(dir, "b.png"), encodePng(2, 2, () => BLUE));
  const r = countDir(dir);
  assert.equal(r.totals.frames, 2);
  assert.equal(r.totals.box, 4);
  assert.equal(r.totals.fighter, 4);
  assert.equal(r.totals.pixels, 8);
  assert.equal(r.frames[0].file, "a.png");
  const empty = mkdtempSync(join(tmpdir(), "fc-empty-"));
  assert.throws(() => countDir(empty), /no PNG frames/);
  assert.throws(() => countDir(join(empty, "missing")), /cannot read dir/);
});

test("parseArgs accepts flags and rejects misuse", () => {
  assert.deepEqual(parseArgs([]), { dir: null, json: false, help: false });
  assert.deepEqual(parseArgs(["--dir", "x", "--json"]), { dir: "x", json: true, help: false });
  assert.equal(parseArgs(["--nope"]).bad, true);
  assert.equal(parseArgs(["--dir"]).bad, true);
  assert.equal(parseArgs(["extra"]).bad, true);
});

test("golden sample pins frame_000000 box and fighter counts", () => {
  const c = countPngBuffer(readFileSync(GOLDEN), "frame_000000.png");
  assert.equal(c.width, 128);
  assert.equal(c.height, 96);
  assert.equal(c.box, 35);
  assert.equal(c.fighter, 0);
});

test("newest showcase set pins fighter 0 with nonzero boxes", () => {
  const dir = findNewestShowcaseDir();
  const r = countDir(dir);
  assert.ok(r.totals.frames >= 4);
  assert.equal(r.totals.fighter, 0);
  assert.ok(r.totals.box > 0);
});

test("CLI prints fighter and box counts for a frame dir", () => {
  const dir = mkdtempSync(join(tmpdir(), "fc-cli-"));
  writeFileSync(join(dir, "a.png"), encodePng(2, 2, () => RED));
  writeFileSync(join(dir, "b.png"), encodePng(2, 2, () => BLUE));
  const run = spawnSync(process.execPath, [TOOL, "--dir", dir], { encoding: "utf8" });
  assert.equal(run.status, 0);
  assert.match(run.stdout, /frames=2/);
  assert.match(run.stdout, /box=4/);
  assert.match(run.stdout, /fighter=4/);
  const js = spawnSync(process.execPath, [TOOL, "--dir", dir, "--json"], { encoding: "utf8" });
  assert.equal(js.status, 0);
  const parsed = JSON.parse(js.stdout);
  assert.equal(parsed.totals.box, 4);
  assert.equal(parsed.totals.fighter, 4);
  const help = spawnSync(process.execPath, [TOOL, "--help"], { encoding: "utf8" });
  assert.equal(help.status, 0);
  assert.equal(help.stdout.trim(), helpText().trim());
  const bad = spawnSync(process.execPath, [TOOL, "--nope"], { encoding: "utf8" });
  assert.equal(bad.status, 2);
  const gone = spawnSync(process.execPath, [TOOL, "--dir", join(dir, "missing")], { encoding: "utf8" });
  assert.equal(gone.status, 1);
});

test("main returns 0 on the newest showcase set", () => {
  assert.equal(main([]), 0);
});
