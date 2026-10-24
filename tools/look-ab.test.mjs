// Run: node --test tools/look-ab.test.mjs
// Fixture PNG pairs only: no arena, no GPU, no window. The tool's own compare,
// refusal and printing are what these cases pin.
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdtempSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { deflateSync } from "node:zlib";
import { PASSES, UsageError, buildSha, captureArgs, clearCapture, compareFrames, newestFrame, sha8, switchFor, verdict } from "./look-ab.mjs";
import { decodePng } from "./frame-moved.mjs";

const TOOL = join(dirname(fileURLToPath(import.meta.url)), "look-ab.mjs");

// ---- a minimal RGBA PNG writer, so the fixtures live in code, not in git ----
const CRC = Array.from({ length: 256 }, (_, n) => {
  let c = n;
  for (let k = 0; k < 8; k += 1) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
  return c >>> 0;
});
const crc32 = (buf) => {
  let c = 0xffffffff;
  for (const b of buf) c = CRC[(c ^ b) & 0xff] ^ (c >>> 8);
  return (c ^ 0xffffffff) >>> 0;
};
const chunk = (type, data) => {
  const len = Buffer.alloc(4);
  len.writeUInt32BE(data.length);
  const body = Buffer.concat([Buffer.from(type, "ascii"), data]);
  const crc = Buffer.alloc(4);
  crc.writeUInt32BE(crc32(body));
  return Buffer.concat([len, body, crc]);
};
const SIG = Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]);
function encodePng(width, height, rgba) {
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(width, 0);
  ihdr.writeUInt32BE(height, 4);
  ihdr[8] = 8;
  ihdr[9] = 6; // RGBA
  const stride = width * 4;
  const raw = Buffer.alloc(height * (stride + 1));
  for (let y = 0; y < height; y += 1) {
    raw[y * (stride + 1)] = 0; // filter none
    rgba.copy(raw, y * (stride + 1) + 1, y * stride, (y + 1) * stride);
  }
  return Buffer.concat([SIG, chunk("IHDR", ihdr), chunk("IDAT", deflateSync(raw)), chunk("IEND", Buffer.alloc(0))]);
}

// A 32x24 arena-ish frame: a gradient with a bright patch, so a region rewrite
// moves real channels and not a flat field.
const W = 32;
const H = 24;
function base(seed = 0) {
  const px = Buffer.alloc(W * H * 4);
  for (let y = 0; y < H; y += 1) {
    for (let x = 0; x < W; x += 1) {
      const o = (y * W + x) * 4;
      px[o] = (x * 7 + seed) & 0xff;
      px[o + 1] = (y * 9 + seed) & 0xff;
      px[o + 2] = (x * y + seed) & 0xff;
      px[o + 3] = 0xff;
    }
  }
  return px;
}
// Rewrite an 8x8 block at (4,4) with a constant: 64 of 768 pixels, 8.33%.
function withPatch(px) {
  const out = Buffer.from(px);
  for (let y = 4; y < 12; y += 1) {
    for (let x = 4; x < 12; x += 1) {
      const o = (y * W + x) * 4;
      out[o] = 250;
      out[o + 1] = 10;
      out[o + 2] = 10;
    }
  }
  return out;
}

function tmp() {
  return mkdtempSync(join(tmpdir(), "look-ab-"));
}

test("identical frames are refused: a zero delta is not evidence", () => {
  const px = base();
  const delta = compareFrames(decodePng(encodePng(W, H, px)), decodePng(encodePng(W, H, px)));
  assert.equal(delta.changed, 0);
  assert.equal(delta.max, 0);
  assert.equal(delta.mean, 0);
  assert.equal(delta.share, 0);
  const said = verdict("night", delta);
  assert.equal(said.live, false);
  assert.match(said.line, /^REFUSED night:/);
  assert.match(said.line, /identical \(0 pixels differ/);
});

test("one changed region prints its share, its mean and its max", () => {
  const off = decodePng(encodePng(W, H, base()));
  const onPx = withPatch(base());
  const on = decodePng(encodePng(W, H, onPx));
  const delta = compareFrames(off, on);
  assert.equal(delta.pixels, W * H);
  assert.equal(delta.changed, 64, "the 8x8 patch is the only movement");
  assert.equal(Math.round(delta.share * 10000) / 100, 8.33, "64 of 768 pixels is 8.33%");
  assert.equal(delta.max, 250 - 4 * 7, "the red channel at x=4 is the largest step");
  assert.ok(delta.mean > 0 && delta.mean < delta.max, "mean sits between 0 and the max");
  const said = verdict("decals", delta);
  assert.equal(said.live, true);
  assert.match(said.line, /^PASS decals: 64 of 768 pixels differ \(8\.33%\), mean [\d.]+, max \d+$/);
});

test("the command refuses an identical pair and passes a moved one", () => {
  const dir = tmp();
  try {
    const same = join(dir, "same.png");
    writeFileSync(same, encodePng(W, H, base()));
    const identical = spawnSync(process.execPath, [TOOL, "--off", same, "--on", same], { encoding: "utf8" });
    assert.equal(identical.status, 1, "a zero delta exits 1");
    assert.match(identical.stdout, /REFUSED/, identical.stdout);

    const onPath = join(dir, "on.png");
    writeFileSync(onPath, encodePng(W, H, withPatch(base())));
    const moved = spawnSync(process.execPath, [TOOL, "--off", same, "--on", onPath], { encoding: "utf8" });
    assert.equal(moved.status, 0, moved.stderr);
    assert.match(moved.stdout, /LOOK-AB/, moved.stdout);
    assert.match(moved.stdout, /8\.33%/, moved.stdout);
    const json = spawnSync(process.execPath, [TOOL, "--off", same, "--on", onPath, "--json"], { encoding: "utf8" });
    const parsed = JSON.parse(json.stdout);
    assert.equal(parsed.live, true);
    assert.equal(parsed.changed, 64);
    assert.match(parsed.offSha8, /^[0-9a-f]{8}$/);
    assert.notEqual(parsed.offSha8, parsed.onSha8, "the two frames hash apart");
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("a pair of different sizes is a usage error, not a delta", () => {
  const off = decodePng(encodePng(W, H, base()));
  const on = decodePng(encodePng(W + 1, H, Buffer.alloc((W + 1) * H * 4)));
  assert.throws(() => compareFrames(off, on), UsageError);
});

test("the pass table names the arena's own switch for each side", () => {
  assert.deepEqual(switchFor("decals", "off").extra, ["--no-break-look"]);
  assert.deepEqual(switchFor("decals", "on").extra, []);
  assert.deepEqual(switchFor("ragdoll", "off").extra, ["--no-ragdoll"]);
  assert.deepEqual(switchFor("msaa", "off").env, { E2040_MSAA: "0" });
  assert.deepEqual(switchFor("msaa", "on").env, { E2040_MSAA: "1" });
  assert.deepEqual(switchFor("night", "on").env, { E2040_NIGHT: "1" });
  for (const [name, spec] of Object.entries(PASSES)) {
    assert.ok(spec.env ? spec.env.startsWith("E2040_") : spec.off.flag.startsWith("--"), `${name} has a real switch`);
  }
  assert.throws(() => switchFor("bloom", "off"), /unknown pass "bloom"; known:/);
});

test("both runs carry the same seed, ticks and capture cadence", () => {
  const offArgs = captureArgs("decals", { seed: 7, ticks: 300, dir: "off" });
  const onArgs = captureArgs("decals", { seed: 7, ticks: 300, dir: "on" });
  const strip = (args) => args.filter((a, i, all) => all[i - 1] !== "--capture-dir");
  assert.deepEqual(strip(offArgs), strip(onArgs), "only the capture dir names a side");
  const at = (args, flag) => args[args.indexOf(flag) + 1];
  assert.equal(at(offArgs, "--seed"), "7");
  assert.equal(at(offArgs, "--ticks"), "300");
  assert.equal(at(offArgs, "--frames"), "300", "--exit ends on frames, so the pair names one");
  assert.ok(offArgs.includes("--fixed-dt"), "one tick per frame: both sides land on the same tick");
  assert.equal(at(offArgs, "--fixed-dt"), "17", "--fixed-dt takes the pacer period in whole ms");
  assert.equal(at(offArgs, "--capture-every"), "60");
  assert.equal(at(offArgs, "--scene"), "arena");
});

test("the build sha rides both runs and the deadline is opt-in", () => {
  const sha = "abcdef1";
  const offArgs = captureArgs("decals", { dir: "off", sha });
  const onArgs = captureArgs("decals", { dir: "on", sha });
  const at = (args, flag) => args[args.indexOf(flag) + 1];
  assert.equal(at(offArgs, "--sha"), sha, "the arena refuses a capture dir without it");
  assert.equal(at(onArgs, "--sha"), sha, "the same label on both sides");
  assert.equal(offArgs.includes("--deadline-ms"), false, "the arena default bound stands");
  const slow = captureArgs("decals", { dir: "off", sha, deadlineMs: 900000 });
  assert.equal(at(slow, "--deadline-ms"), "900000");
  assert.match(buildSha({}), /^[0-9a-f]{7,128}$/i, "the default label is a real sha");
});

test("newestFrame picks the highest frame number in a capture dir", () => {
  const dir = tmp();
  try {
    for (const name of ["frame_000001.png", "frame_000120.png", "frame_000015.png", "manifest.json"]) {
      writeFileSync(join(dir, name), "");
    }
    assert.equal(newestFrame(dir), join(dir, "frame_000120.png"));
    assert.throws(() => newestFrame(join(dir, "nope")), UsageError);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("clearCapture empties a stale pair before its run and keeps other files", () => {
  const dir = tmp();
  try {
    writeFileSync(join(dir, "frame_000927.png"), "stale");
    writeFileSync(join(dir, "manifest.json"), "{}");
    writeFileSync(join(dir, "notes.txt"), "keep me");
    assert.equal(clearCapture(dir), 2, "the old frame and manifest go");
    assert.deepEqual(readdirSync(dir), ["notes.txt"], "an unrelated file stays");
    assert.equal(clearCapture(join(dir, "nope")), 0, "a dir that does not exist is fine");
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("--list prints one line per switchable pass", () => {
  const run = spawnSync(process.execPath, [TOOL, "--list"], { encoding: "utf8" });
  assert.equal(run.status, 0, run.stderr);
  for (const name of Object.keys(PASSES)) assert.match(run.stdout, new RegExp(`^${name} `, "m"), name);
});
