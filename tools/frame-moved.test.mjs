// Run: node --test tools/frame-moved.test.mjs
// frame-moved on generated fixtures: identical frames print MOVED: none, a
// one-box change prints a small percent, size mismatch is handled, and a
// corrupt PNG exits 2. CLI runs happen in a temp repo root (tool + board
// copied in), so no repo file is touched; the baseline path rides
// FRAME_MOVED_BASELINE.
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { deflateSync } from "node:zlib";
import { copyFileSync, mkdirSync, mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { compareCaptures, decodePng, frozenSequenceInfo, lookStage, resolveCapture } from "./frame-moved.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));
const TOOL = join(HERE, "frame-moved.mjs");

// --- minimal PNG encoder for fixtures (8-bit RGBA, filter 0) ----------------
const CRC_TABLE = (() => {
  const t = new Uint32Array(256);
  for (let n = 0; n < 256; n += 1) {
    let c = n;
    for (let k = 0; k < 8; k += 1) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    t[n] = c;
  }
  return t;
})();

function crc32(buf) {
  let c = 0xffffffff;
  for (let i = 0; i < buf.length; i += 1) c = CRC_TABLE[(c ^ buf[i]) & 0xff] ^ (c >>> 8);
  return (c ^ 0xffffffff) >>> 0;
}

function chunk(type, data) {
  const head = Buffer.alloc(8);
  head.writeUInt32BE(data.length, 0);
  head.write(type, 4, "ascii");
  const tail = Buffer.alloc(4);
  tail.writeUInt32BE(crc32(Buffer.concat([Buffer.from(type, "ascii"), data])), 0);
  return Buffer.concat([head, data, tail]);
}

function encodePng(w, h, rgba) {
  const parts = [];
  for (let y = 0; y < h; y += 1) {
    parts.push(Buffer.from([0]));
    parts.push(Buffer.from(rgba.subarray(y * w * 4, (y + 1) * w * 4)));
  }
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(w, 0);
  ihdr.writeUInt32BE(h, 4);
  ihdr[8] = 8;
  ihdr[9] = 6;
  return Buffer.concat([
    Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]),
    chunk("IHDR", ihdr),
    chunk("IDAT", deflateSync(Buffer.concat(parts))),
    chunk("IEND", Buffer.alloc(0)),
  ]);
}

function solid(w, h, r, g, b) {
  const px = Buffer.alloc(w * h * 4);
  for (let i = 0; i < px.length; i += 4) {
    px[i] = r;
    px[i + 1] = g;
    px[i + 2] = b;
    px[i + 3] = 0xff;
  }
  return px;
}

const BOARD = (vs1, vs4) =>
  ["# SPRINT", "## Board", `- [${vs1}] VS1 game: arena kit`, `- [${vs4}] VS4 game: fighters`, ""].join("\n");

function sandbox({ board = BOARD(" ", " "), frames = {}, teamLook = null } = {}) {
  const root = mkdtempSync(join(tmpdir(), "frame-moved-"));
  mkdirSync(join(root, "tools"), { recursive: true });
  mkdirSync(join(root, "docs"), { recursive: true });
  copyFileSync(TOOL, join(root, "tools", "frame-moved.mjs"));
  writeFileSync(join(root, "docs", "SPRINT.md"), board);
  for (const [name, buf] of Object.entries(frames)) {
    mkdirSync(dirname(join(root, name)), { recursive: true });
    writeFileSync(join(root, name), buf);
  }
  if (teamLook !== null) {
    mkdirSync(join(root, "team"), { recursive: true });
    writeFileSync(join(root, "team", "look.md"), teamLook);
  }
  const env = { ...process.env, FRAME_MOVED_BASELINE: join(root, "baseline.json") };
  const run = (args) =>
    spawnSync(process.execPath, [join(root, "tools", "frame-moved.mjs"), ...args], {
      encoding: "utf8",
      cwd: root,
      env,
    });
  return { root, run };
}

const A = encodePng(64, 64, solid(64, 64, 200, 100, 50));
const A_BOX = (() => {
  const px = solid(64, 64, 200, 100, 50);
  for (let y = 4; y < 12; y += 1) {
    for (let x = 4; x < 12; x += 1) {
      const o = (y * 64 + x) * 4;
      px[o] = 20;
      px[o + 1] = 30;
      px[o + 2] = 40;
    }
  }
  return encodePng(64, 64, px);
})();
const SMALL = encodePng(32, 32, solid(32, 32, 200, 100, 50));

test("builtin decode round-trips dimensions and pixels", () => {
  const d = decodePng(A);
  assert.equal(d.width, 64);
  assert.equal(d.height, 64);
  assert.equal(d.data[0], 200);
  assert.equal(d.data[1], 100);
  assert.equal(d.data[2], 50);
});

test("pure compare: identical is 0, one box is small, resized crops", () => {
  const a = { width: 64, height: 64, data: decodePng(A).data };
  const same = compareCaptures(a, a, null);
  assert.equal(same.pct, 0);
  const box = compareCaptures(a, { width: 64, height: 64, data: decodePng(A_BOX).data }, null);
  assert.ok(box.pct > 0 && box.pct < 10, `box pct ${box.pct}`);
  assert.equal(box.changed, 64, "8x8 box changes 64 pixels");
  const small = compareCaptures(a, { width: 32, height: 32, data: decodePng(SMALL).data }, null);
  assert.equal(small.resized, true);
  assert.equal(small.total, 1024);
  assert.equal(small.pct, 0, "same paint at a smaller size compares clean");
});

test("lookStage: cubes until VS1/VS4 tick", () => {
  assert.equal(lookStage(BOARD(" ", " ")), "cubes");
  assert.equal(lookStage(BOARD("x", " ")), "kit");
  assert.equal(lookStage(BOARD(" ", "x")), "fighters");
  assert.equal(lookStage(BOARD("x", "X")), "fighters");
});

test("resolveCapture: a dir means its newest frame_*.png", () => {
  const { root } = sandbox({ frames: { "cap/frame_000002.png": A, "cap/frame_000010.png": A_BOX } });
  assert.equal(resolveCapture("cap", root), join(root, "cap", "frame_000010.png"));
});

test("identical frames print MOVED: none", () => {
  const { run } = sandbox({ frames: { "a.png": A, "b.png": A } });
  const r = run(["--new", "a.png", "--old", "b.png"]);
  assert.equal(r.status, 0);
  assert.match(r.stdout, /^FRAME: [0-9a-f]{8} 64x64$/m);
  assert.match(r.stdout, /^MOVED: none$/m);
  assert.match(r.stdout, /^LOOK_STAGE: cubes$/m);
  assert.match(r.stdout, /^ENGINE: (pixelmatch|builtin)$/m);
});

test("a one-box change prints a small percent", () => {
  const { run } = sandbox({ frames: { "a.png": A, "b.png": A_BOX } });
  const r = run(["--new", "b.png", "--old", "a.png"]);
  assert.equal(r.status, 0);
  const m = r.stdout.match(/^MOVED: pixel ([\d.]+)%$/m);
  assert.ok(m, `expected a MOVED percent, got:\n${r.stdout}`);
  const pct = Number(m[1]);
  assert.ok(pct > 0 && pct < 10, `box pct ${pct}`);
});

test("size mismatch compares at the smaller size and says so", () => {
  const { run } = sandbox({ frames: { "a.png": A, "s.png": SMALL } });
  const r = run(["--new", "a.png", "--old", "s.png"]);
  assert.equal(r.status, 0);
  assert.match(r.stdout, /size mismatch 64x64 vs 32x32, compared at 32x32/);
  assert.match(r.stdout, /^MOVED: none$/m);
});

test("a corrupt PNG exits 2", () => {
  const { run } = sandbox({ frames: { "a.png": A, "bad.png": Buffer.from("not a png at all") } });
  const r = run(["--new", "bad.png", "--old", "a.png"]);
  assert.equal(r.status, 2);
});

test("a missing file exits 2, as does no input", () => {
  const { run } = sandbox({ frames: { "a.png": A } });
  assert.equal(run(["--new", "gone.png", "--old", "a.png"]).status, 2);
  assert.equal(run([]).status, 2);
});

test("--baseline writes sha256/size/source/date and becomes the default old", () => {
  const { root, run } = sandbox({ frames: { "a.png": A } });
  const w = run(["--baseline", "a.png"]);
  assert.equal(w.status, 0);
  assert.match(w.stdout, /^FRAME: [0-9a-f]{8} 64x64$/m);
  const record = JSON.parse(readFileSync(join(root, "baseline.json"), "utf8"));
  for (const key of ["sha256", "size", "source", "date"]) assert.ok(key in record, `baseline has ${key}`);
  assert.equal(record.sha256.length, 64);
  const r = run(["--new", "a.png"]);
  assert.equal(r.status, 0);
  assert.match(r.stdout, /^MOVED: none$/m);
});

test("--json parses with moved, stage and engine", () => {
  const { run } = sandbox({ frames: { "a.png": A, "b.png": A_BOX } });
  const r = run(["--new", "b.png", "--old", "a.png", "--json"]);
  assert.equal(r.status, 0);
  const obj = JSON.parse(r.stdout);
  assert.equal(obj.moved, "pixel");
  assert.ok(obj.movedPct > 0 && obj.movedPct < 10);
  assert.equal(obj.stage, "cubes");
  assert.ok(["pixelmatch", "builtin"].includes(obj.engine));
});

test("ticked VS1/VS4 rows move LOOK_STAGE to kit/fighters", () => {
  const kit = sandbox({ board: BOARD("x", " "), frames: { "a.png": A } });
  assert.match(kit.run(["--new", "a.png", "--old", "a.png"]).stdout, /^LOOK_STAGE: kit$/m);
  const fighters = sandbox({ board: BOARD("x", "x"), frames: { "a.png": A } });
  assert.match(fighters.run(["--new", "a.png", "--old", "a.png"]).stdout, /^LOOK_STAGE: fighters$/m);
});

test("--since-last-playtest compares newest target capture with team/look.md FRAME", () => {
  const look = ["# look", "FRAME: old/frame_000001.png", ""].join("\n");
  const sb = sandbox({
    frames: { "old/frame_000001.png": A, "target/run/frame_000000.png": A, "target/run/frame_000001.png": A },
    teamLook: look,
  });
  const r = sb.run(["--since-last-playtest"]);
  assert.equal(r.status, 0);
  assert.match(r.stdout, /^MOVED: none$/m);
  const missing = sandbox({ frames: { "a.png": A } });
  assert.equal(missing.run(["--since-last-playtest"]).status, 2);
});

test("frozen manifest flags FROZEN with the shared png_fnv", () => {
  const rows = [0, 1, 2].map((tick) => ({
    tick,
    file: "frame_" + String(tick).padStart(6, "0") + ".png",
    width: 64,
    height: 64,
    checkpoint: "0000000000000000",
    png_fnv: "1345756a019eb64e",
    sha256: "aaa",
    bytes: 1,
  }));
  const manifest = JSON.stringify({ frames: rows });
  const sb = sandbox({
    frames: {
      "cap/frame_000000.png": A,
      "cap/frame_000001.png": A,
      "cap/frame_000002.png": A,
      "cap/manifest.json": Buffer.from(manifest),
      "old.png": A_BOX,
    },
  });
  const r = sb.run(["--new", "cap", "--old", "old.png"]);
  assert.equal(r.status, 0);
  assert.match(r.stdout, /^FROZEN: 3 frames share png_fnv 1345756a019eb64e/m);
  assert.match(r.stdout, /manifest\.json/);
  const j = JSON.parse(sb.run(["--new", "cap", "--old", "old.png", "--json"]).stdout);
  assert.equal(j.frozen.count, 3);
  assert.equal(j.frozen.hash, "1345756a019eb64e");
  assert.equal(j.frozen.kind, "png_fnv");
  const info = frozenSequenceInfo(sb.root + "/cap");
  assert.equal(info && info.count, 3);
  const mixedRows = [
    { tick: 0, file: "frame_000000.png", png_fnv: "aaaaaaaaaaaaaaaa", sha256: "aaa", width: 64, height: 64, checkpoint: "0", bytes: 1 },
    { tick: 1, file: "frame_000001.png", png_fnv: "bbbbbbbbbbbbbbbb", sha256: "bbb", width: 64, height: 64, checkpoint: "1", bytes: 1 },
  ];
  const mixedSb = sandbox({ frames: { "m/frame_000000.png": A, "m/frame_000001.png": A_BOX, "m/manifest.json": Buffer.from(JSON.stringify({ frames: mixedRows })), "old.png": A } });
  const mr = mixedSb.run(["--new", "m", "--old", "old.png"]);
  assert.equal(mr.status, 0);
  assert.doesNotMatch(mr.stdout, /FROZEN:/);
  assert.equal(frozenSequenceInfo(mixedSb.root + "/m"), null);
});
