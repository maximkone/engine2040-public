// Run: node --test tools/replay-to-mp4.test.mjs
// GS25 replay to video: tape -> PNGs -> mp4. Pure argv builders stay
// pinned with stub runners so no cargo or ffmpeg runs in the test.
import assert from "node:assert/strict";
import { existsSync, mkdirSync, mkdtempSync, readdirSync, readFileSync, rmSync, statSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { execSync, spawnSync } from "node:child_process";
import { crc32, deflateSync } from "node:zlib";
import {
  FFMPEG_INSTALL_HINT,
  FRAMERATE,
  FRAME_PATTERN,
  PIPE_MAX_BYTES,
  USAGE,
  UsageError,
  buildCaptureArgv,
  buildFfmpegArgv,
  encoderAttempts,
  parseArgs,
  pickEncoder,
  planFrames,
  readFramesForPipe,
  readTapeTicks,
  resolveSha,
  runMain,
} from "./replay-to-mp4.mjs";

// Same bytes crates/game/src/capture.rs writes: magic, u16 version, u32 count,
// then 10-byte inputs (u32 tick, slot, mx, mz, i16 facing, buttons).
function tapeBytes(ticks) {
  const buf = Buffer.alloc(14 + ticks.length * 10);
  buf.write("E2040CAP", 0, "latin1");
  buf.writeUInt16LE(1, 8);
  buf.writeUInt32LE(ticks.length, 10);
  ticks.forEach((t, i) => buf.writeUInt32LE(t, 14 + i * 10));
  return buf;
}

const REPO_ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const FRAMES3 = ["frame_000000.png", "frame_000001.png", "frame_000002.png"];

function stubDeps({
  encoders = " Encoder h264_nvenc ",
  shaEnv = "abc1234",
  captureStatus = 0,
  ffmpegStatus = 0,
  tapeExists = true,
  tape = tapeBytes([0, 59, 119]),
  names = FRAMES3,
} = {}) {
  const out = [];
  const err = [];
  const spawns = [];
  const removed = [];
  const statuses = Array.isArray(ffmpegStatus) ? [...ffmpegStatus] : null;
  const deps = {
    env: { E2040_SHA: shaEnv },
    execSync: (cmd) => {
      if (cmd.includes("ffmpeg")) return Buffer.from(encoders);
      if (cmd.includes("git")) return Buffer.from("deadbeef1234567\n");
      throw new Error("unexpected exec: " + cmd);
    },
    spawnSync: (cmd, args, opts) => {
      spawns.push([cmd, args, opts]);
      if (cmd !== "ffmpeg") return { status: captureStatus };
      return { status: statuses ? statuses.shift() : ffmpegStatus };
    },
    mkdtemp: (prefix) => join("/tmp", prefix + "test"),
    rm: (p) => removed.push(p),
    existsSync: (p) => (typeof tapeExists === "function" ? tapeExists(p) : tapeExists),
    readdirSync: () => names,
    readFileSync: (p) => (/frame_\d{6}\.png$/.test(String(p)) ? Buffer.from("png") : tape),
    stdout: (s) => out.push(s),
    stderr: (s) => err.push(s),
  };
  return { deps, out, err, spawns, removed };
}

test("constants pin the 60fps numbered-frame contract", () => {
  assert.equal(FRAMERATE, 60);
  assert.equal(FRAME_PATTERN, "frame_%06d.png");
  assert.ok(USAGE.includes("<tape>") && USAGE.includes("--out clip.mp4"));
  assert.ok(FFMPEG_INSTALL_HINT.includes("ffmpeg not found"));
  assert.ok(new UsageError("x") instanceof Error);
});

test("parseArgs accepts tape, out and max-ticks", () => {
  assert.deepEqual(parseArgs(["tape.json", "--out", "clip.mp4"]), { tape: "tape.json", out: "clip.mp4", maxTicks: null, pipe: false, keepFrames: false });
  assert.deepEqual(parseArgs(["tape.json", "--out", "clip.mp4", "--max-ticks", "600"]), {
    tape: "tape.json",
    out: "clip.mp4",
    maxTicks: 600,
    pipe: false,
    keepFrames: false,
  });
  assert.deepEqual(parseArgs(["tape.json", "--out", "clip.mp4", "--pipe", "--keep-frames"]), {
    tape: "tape.json",
    out: "clip.mp4",
    maxTicks: null,
    pipe: true,
    keepFrames: true,
  });
});

test("parseArgs help and rejects bad input", () => {
  assert.deepEqual(parseArgs(["-h"]), { help: true });
  assert.deepEqual(parseArgs(["--help"]), { help: true });
  assert.throws(() => parseArgs([]), /missing <tape>/);
  assert.throws(() => parseArgs(["tape.json"]), /missing --out/);
  assert.throws(() => parseArgs(["a", "b", "--out", "c.mp4"]), /too many positional/);
  assert.throws(() => parseArgs(["tape.json", "--out", "a.mp4", "--out", "b.mp4"]), /duplicate --out/);
  assert.throws(() => parseArgs(["tape.json", "--out", "a.mp4", "--bogus"]), /unknown flag/);
  assert.throws(() => parseArgs(["tape.json", "--out", "a.mp4", "--max-ticks", "0"]), /--max-ticks needs/);
  assert.throws(() => parseArgs(["tape.json", "--out", "a.mp4", "--max-ticks", "x"]), /--max-ticks needs/);
  assert.throws(() => parseArgs(["tape.json", "--out", "a.mp4", "--pipe", "--pipe"]), /duplicate --pipe/);
  assert.throws(() => parseArgs(["tape.json", "--out", "a.mp4", "--keep-frames", "--keep-frames"]), /duplicate --keep-frames/);
});

test("pickEncoder prefers nvenc only when listed", () => {
  assert.equal(pickEncoder("Encoders:\n V..... h264_nvenc\n"), "h264_nvenc");
  assert.equal(pickEncoder("Encoders:\n V..... libx264\n"), "libx264");
  assert.equal(pickEncoder("h264_nvenc_extra"), "libx264");
});

test("buildCaptureArgv pins the exact arena argv", () => {
  const got = buildCaptureArgv({ tape: "t.json", dir: "/tmp/d", frames: 600, sha: "abc1234" });
  assert.equal(got.cmd, "cargo");
  assert.deepEqual(got.args, [
    "run", "-p", "engine-game", "--features", "app", "--bin", "arena", "--",
    "--tape", "t.json",
    "--capture-dir", "/tmp/d",
    "--capture-every", "1",
    "--sha", "abc1234",
    "--fps", "60",
    "--frames", "600",
    "--exit",
  ]);
  for (const bad of [null, undefined, 0, -3, 1.5, "600"]) {
    assert.throws(() => buildCaptureArgv({ tape: "t", dir: "d", frames: bad, sha: "abc1234" }), /frame count/);
  }
});

test("the capture argv matches what the arena bin really needs", () => {
  // `--bin arena` is refused without the app feature, and without --frames plus
  // --exit the window never closes: both come from the crate, not from memory.
  const toml = readFileSync(new URL("../crates/game/Cargo.toml", import.meta.url), "utf8");
  const block = toml.split("[[bin]]").find((b) => /^\s*name = "arena"\s*$/m.test(b.split(/\r?\n\r?\n/)[0])) ?? "";
  assert.match(block, /required-features = \["app"\]/);
  const { args } = buildCaptureArgv({ tape: "t", dir: "d", frames: 5, sha: "abc1234" });
  assert.equal(args[args.indexOf("--features") + 1], "app");
  assert.equal(args[args.indexOf("--bin") + 1], "arena");
  assert.ok(args.includes("--exit"));
  const rust = readFileSync(new URL("../crates/game/src/capture.rs", import.meta.url), "utf8");
  for (const flag of ["--capture-dir", "--capture-every", "--tape", "--sha", "--fps"]) {
    assert.ok(rust.includes(`"${flag}" =>`), `capture.rs parses ${flag}`);
    assert.ok(args.includes(flag));
  }
  assert.match(rust, /const TAPE_MAGIC: &\[u8; 8\] = b"E2040CAP";/);
  assert.match(rust, /const HEADER_BYTES: usize = 14;/);
  assert.match(rust, /const INPUT_BYTES: usize = 10;/);
});

test("buildFfmpegArgv pins both encoder cases", () => {
  const [nvCmd, nvArgs] = buildFfmpegArgv({ pattern: "frame_%06d.png", out: "clip.mp4", encoder: "h264_nvenc" });
  assert.equal(nvCmd, "ffmpeg");
  assert.deepEqual(nvArgs, ["-y", "-framerate", "60", "-i", "frame_%06d.png", "-c:v", "h264_nvenc", "-pix_fmt", "yuv420p", "clip.mp4"]);
  const [, xArgs] = buildFfmpegArgv({ pattern: "frame_%06d.png", out: "clip.mp4", encoder: "libx264" });
  assert.deepEqual(xArgs, [
    "-y", "-framerate", "60", "-i", "frame_%06d.png",
    "-c:v", "libx264", "-preset", "veryfast", "-crf", "18", "-pix_fmt", "yuv420p", "clip.mp4",
  ]);
  const [, shifted] = buildFfmpegArgv({ pattern: "f_%06d.png", out: "c.mp4", encoder: "libx264", startNumber: 4 });
  assert.deepEqual(shifted.slice(0, 7), ["-y", "-framerate", "60", "-start_number", "4", "-i", "f_%06d.png"]);
});

test("readTapeTicks reads the span of a real tape and refuses the rest", () => {
  assert.equal(readTapeTicks(tapeBytes([0, 10, 119])), 120);
  assert.equal(readTapeTicks(tapeBytes([7])), 8);
  assert.throws(() => readTapeTicks(Buffer.from('{"ticks":3}\n')), /bad magic/);
  assert.throws(() => readTapeTicks(Buffer.alloc(4)), /bad magic/);
  assert.throws(() => readTapeTicks(tapeBytes([])), /no inputs/);
  const v2 = tapeBytes([1]);
  v2.writeUInt16LE(2, 8);
  assert.throws(() => readTapeTicks(v2), /unsupported tape version 2/);
  assert.throws(() => readTapeTicks(tapeBytes([1, 2]).subarray(0, 30)), /does not match/);
});

test("planFrames needs a gapless numbered list and names the break", () => {
  assert.deepEqual(planFrames(["manifest.json", ...FRAMES3, "cutlog.json"]), { count: 3, first: 0, last: 2 });
  assert.deepEqual(planFrames(["frame_000002.png", "frame_000001.png"]), { count: 2, first: 1, last: 2 });
  assert.throws(() => planFrames([]), /no frames/);
  assert.throws(() => planFrames(["manifest.json", "frame_1.png"]), /no frames/);
  assert.throws(() => planFrames(["frame_000000.png", "frame_000002.png"]), /gap after frame_000000\.png/);
});

test("encoderAttempts tries nvenc first only when it is listed", () => {
  assert.deepEqual(encoderAttempts(" V....D h264_nvenc NVIDIA "), ["h264_nvenc", "libx264"]);
  assert.deepEqual(encoderAttempts(" V....D libx264 "), ["libx264"]);
  assert.deepEqual(encoderAttempts(""), ["libx264"]);
});

test("readFramesForPipe joins frames in order and honours the memory cap", () => {
  const seen = [];
  const read = (p) => { seen.push(p); return Buffer.from(p.endsWith("1.png") ? "BB" : "AA"); };
  const got = readFramesForPipe("d", ["frame_000001.png", "manifest.json", "frame_000000.png"], read);
  assert.equal(got.toString(), "AABB");
  assert.equal(seen.length, 2);
  assert.throws(() => readFramesForPipe("d", FRAMES3, () => Buffer.alloc(10), 25), /drop --pipe/);
  assert.ok(PIPE_MAX_BYTES >= 64 * 1024 * 1024);
});

test("resolveSha prefers E2040_SHA then git then throws", () => {
  assert.equal(resolveSha({ env: { E2040_SHA: "ABC1234" }, execSync: () => { throw new Error("no git"); } }), "abc1234");
  assert.equal(resolveSha({ env: {}, execSync: () => Buffer.from("DEADBEEF1234567\n") }), "deadbeef1234567");
  assert.throws(() => resolveSha({ env: {}, execSync: () => { throw new Error("no git"); } }), /need a build SHA/);
});

test("runMain help and usage errors exit without runners", () => {
  const h = stubDeps();
  assert.equal(runMain(["-h"], h.deps), 0);
  assert.ok(h.out.join("").includes("usage:"));
  const bad = stubDeps();
  assert.equal(runMain([], bad.deps), 2);
  assert.ok(bad.err.join("").includes("missing <tape>"));
});

test("runMain without ffmpeg exits 2 with the install hint", () => {
  const s = stubDeps();
  s.deps.execSync = (cmd) => {
    if (cmd.includes("ffmpeg")) throw new Error("not found");
    return Buffer.from("sha");
  };
  assert.equal(runMain(["t.json", "--out", "c.mp4"], s.deps), 2);
  assert.ok(s.err.join("").includes("ffmpeg not found"));
  assert.ok(s.err.join("").includes("E_FFMPEG_MISSING"));
  assert.equal(s.spawns.length, 0);
});

test("runMain capture failure propagates its status", () => {
  const s = stubDeps({ captureStatus: 3 });
  assert.equal(runMain(["t.json", "--out", "c.mp4"], s.deps), 3);
  assert.ok(s.err.join("").includes("capture failed"));
  assert.equal(s.spawns.length, 1);
});

test("runMain success runs capture then ffmpeg and names the encoder", () => {
  const s = stubDeps({ encoders: " h264_nvenc " });
  assert.equal(runMain(["t.json", "--out", "c.mp4"], s.deps), 0);
  assert.equal(s.spawns.length, 2);
  assert.equal(s.spawns[0][0], "cargo");
  assert.ok(s.spawns[0][1].includes(resolve("t.json")));
  assert.equal(s.spawns[0][2].cwd, REPO_ROOT);
  // The tape spans ticks 0..119, so the capture asks for 120 frames and then exits.
  const cap = s.spawns[0][1];
  assert.equal(cap[cap.indexOf("--frames") + 1], "120");
  assert.ok(cap.includes("--exit") && cap.includes("--features"));
  assert.equal(s.spawns[1][0], "ffmpeg");
  assert.deepEqual(s.spawns[1][1], ["-y", "-framerate", "60", "-i", join("/tmp", "replay-mp4-test", FRAME_PATTERN), "-c:v", "h264_nvenc", "-pix_fmt", "yuv420p", "c.mp4"]);
  assert.ok(s.out.join("").includes("wrote c.mp4 (h264_nvenc, 3 frames)"));
  assert.deepEqual(s.removed, [join("/tmp", "replay-mp4-test")]);
  const x = stubDeps({ encoders: "libx264 only" });
  assert.equal(runMain(["t.json", "--out", "c.mp4"], x.deps), 0);
  assert.ok(x.spawns[1][1].includes("libx264"));
  assert.ok(x.out.join("").includes("(libx264, 3 frames)"));
});

test("runMain --max-ticks wins over the tape and the tape is then not read", () => {
  const s = stubDeps({ tape: Buffer.from("not a tape") });
  assert.equal(runMain(["t.json", "--out", "c.mp4", "--max-ticks", "90"], s.deps), 0);
  const cap = s.spawns[0][1];
  assert.equal(cap[cap.indexOf("--frames") + 1], "90");
});

test("runMain refuses an unreadable tape with exit 2 before any capture", () => {
  const s = stubDeps({ tape: Buffer.from('{"ticks":3}') });
  assert.equal(runMain(["t.json", "--out", "c.mp4"], s.deps), 2);
  assert.ok(s.err.join("").includes("bad magic"));
  assert.equal(s.spawns.length, 0);
});

test("runMain --keep-frames keeps the directory and says where", () => {
  const s = stubDeps();
  assert.equal(runMain(["t.json", "--out", "c.mp4", "--keep-frames"], s.deps), 0);
  assert.deepEqual(s.removed, []);
  assert.ok(s.out.join("").includes("frames kept in " + join("/tmp", "replay-mp4-test")));
});

test("runMain retries as libx264 when a listed nvenc fails to start", () => {
  const s = stubDeps({ encoders: " h264_nvenc ", ffmpegStatus: [1, 0] });
  assert.equal(runMain(["t.json", "--out", "c.mp4"], s.deps), 0);
  assert.deepEqual(s.spawns.map((x) => x[0]), ["cargo", "ffmpeg", "ffmpeg"]);
  assert.ok(s.spawns[1][1].includes("h264_nvenc"));
  assert.ok(s.spawns[2][1].includes("libx264") && !s.spawns[2][1].includes("h264_nvenc"));
  assert.ok(s.err.join("").includes("h264_nvenc failed (status 1); retrying with libx264"));
  assert.ok(s.out.join("").includes("(libx264, 3 frames)"));
  assert.equal(s.removed.length, 1);
});

test("runMain keeps the frames and the last status when every encoder fails", () => {
  const s = stubDeps({ encoders: " h264_nvenc ", ffmpegStatus: [1, 7] });
  assert.equal(runMain(["t.json", "--out", "c.mp4"], s.deps), 7);
  assert.ok(s.err.join("").includes("ffmpeg failed (status 7)"));
  assert.deepEqual(s.removed, []);
  const x = stubDeps({ encoders: "libx264", ffmpegStatus: 4 });
  assert.equal(runMain(["t.json", "--out", "c.mp4"], x.deps), 4);
  assert.equal(x.spawns.filter((c) => c[0] === "ffmpeg").length, 1);
});

test("runMain fails loudly on an empty or gapped capture and never calls ffmpeg", () => {
  const empty = stubDeps({ names: ["manifest.json"] });
  assert.equal(runMain(["t.json", "--out", "c.mp4"], empty.deps), 1);
  assert.ok(empty.err.join("").includes("capture wrote no frames"));
  assert.deepEqual(empty.spawns.map((x) => x[0]), ["cargo"]);
  const gap = stubDeps({ names: ["frame_000000.png", "frame_000003.png"] });
  assert.equal(runMain(["t.json", "--out", "c.mp4"], gap.deps), 1);
  assert.ok(gap.err.join("").includes("frame gap after frame_000000.png"));
  assert.deepEqual(gap.removed, []);
});

test("runMain reports a cargo that cannot start", () => {
  const s = stubDeps();
  s.deps.spawnSync = (cmd, args, opts) => { s.spawns.push([cmd, args, opts]); return { status: null, error: Object.assign(new Error("spawn"), { code: "ENOENT" }) }; };
  assert.equal(runMain(["t.json", "--out", "c.mp4"], s.deps), 1);
  assert.ok(s.err.join("").includes("capture failed (status null) (ENOENT)"));
});

test("runMain missing tape exits 2 without runners", () => {
  const s = stubDeps({ tapeExists: false });
  assert.equal(runMain(["no-such-tape.json", "--out", "c.mp4"], s.deps), 2);
  assert.ok(s.err.join("").includes("tape not found"));
  assert.equal(s.spawns.length, 0);
});

test("runMain converts a real tape fixture to mp4", () => {
  const dir = mkdtempSync(join(tmpdir(), "replay-mp4-"));
  const tape = join(dir, "tape.json");
  const out = join(dir, "clip.mp4");
  writeFileSync(tape, tapeBytes([0, 1, 2]));
  const s = stubDeps({ tapeExists: (p) => existsSync(p) });
  s.deps.readFileSync = readFileSync;
  s.deps.readdirSync = readdirSync;
  s.deps.mkdtemp = (prefix) => {
    mkdirSync(join(dir, prefix + "frames"));
    return join(dir, prefix + "frames");
  };
  const origSpawn = s.deps.spawnSync;
  s.deps.spawnSync = (cmd, args, opts) => {
    const r = origSpawn(cmd, args, opts);
    if (cmd === "cargo") {
      const fdir = args[args.indexOf("--capture-dir") + 1];
      for (let i = 0; i < 3; i += 1) writeFileSync(join(fdir, "frame_" + String(i).padStart(6, "0") + ".png"), "png");
    }
    if (cmd === "ffmpeg") {
      const target = args[args.length - 1];
      writeFileSync(target, "fake-mp4-from-tape\n");
    }
    return r;
  };
  assert.equal(runMain([tape, "--out", out], s.deps), 0);
  assert.ok(existsSync(out));
  assert.ok(statSync(out).size > 0);
  rmSync(dir, { recursive: true, force: true });
});

test("buildFfmpegArgv pipe pins image2pipe for both encoders", () => {
  const [, nv] = buildFfmpegArgv({ out: "clip.mp4", encoder: "h264_nvenc", pipe: true });
  assert.deepEqual(nv, ["-y", "-f", "image2pipe", "-framerate", "60", "-i", "-", "-c:v", "h264_nvenc", "-pix_fmt", "yuv420p", "clip.mp4"]);
  const [, x] = buildFfmpegArgv({ out: "clip.mp4", encoder: "libx264", pipe: true });
  assert.deepEqual(x, ["-y", "-f", "image2pipe", "-framerate", "60", "-i", "-", "-c:v", "libx264", "-preset", "veryfast", "-crf", "18", "-pix_fmt", "yuv420p", "clip.mp4"]);
});

test("runMain pipe streams 600 PNG bytes with pinned argv", () => {
  const dir = mkdtempSync(join(tmpdir(), "replay-mp4-pipe-"));
  const tape = join(dir, "tape.json");
  const out = join(dir, "clip-pipe.mp4");
  writeFileSync(tape, tapeBytes([0, 599]));
  const frames = join(dir, "frames");
  mkdirSync(frames, { recursive: true });
  const bytes = Buffer.from("fake-png");
  const s = stubDeps({ encoders: " h264_nvenc ", tapeExists: (p) => existsSync(p) });
  s.deps.readFileSync = readFileSync;
  s.deps.readdirSync = readdirSync;
  s.deps.mkdtemp = () => frames;
  let stdinBytes = 0;
  s.deps.spawnSync = (cmd, args, opts) => {
    s.spawns.push([cmd, args, opts]);
    if (cmd === "cargo") {
      for (let i = 0; i < 600; i += 1) writeFileSync(join(frames, "frame_" + String(i).padStart(6, "0") + ".png"), bytes);
      return { status: 0 };
    }
    stdinBytes = opts.input.length;
    writeFileSync(out, "fake-mp4-pipe\n");
    return { status: 0 };
  };
  assert.equal(runMain([tape, "--out", out, "--max-ticks", "600", "--pipe"], s.deps), 0);
  assert.equal(s.spawns[1][0], "ffmpeg");
  assert.ok(s.spawns[1][1].includes("image2pipe") && s.spawns[1][1].includes("h264_nvenc"));
  assert.equal(readdirSync(frames).filter((n) => n.endsWith(".png")).length, 600);
  assert.equal(stdinBytes, 600 * bytes.length);
  rmSync(dir, { recursive: true, force: true });
});

test("runMain pipe with no frames fails without ffmpeg", () => {
  const s = stubDeps({ names: [] });
  assert.equal(runMain(["t.json", "--out", "c.mp4", "--pipe"], s.deps), 1);
  assert.ok(s.err.join("").includes("capture wrote no frames"));
  assert.equal(s.spawns.length, 1);
});

test("runMain pipe past the memory cap fails before ffmpeg starts", () => {
  const s = stubDeps();
  s.deps.readFileSync = (p) => (/frame_\d{6}\.png$/.test(String(p)) ? Buffer.alloc(100) : tapeBytes([0]));
  s.deps.pipeMaxBytes = 250;
  assert.equal(runMain(["t.json", "--out", "c.mp4", "--pipe"], s.deps), 1);
  assert.ok(s.err.join("").includes("drop --pipe"));
  assert.equal(s.spawns.length, 1);
});

// ---- Real ffmpeg legs: only the cargo capture is stubbed (it writes real PNGs).
// Skipped when ffmpeg or its libx264 encoder is absent, so the file stays green anywhere.
function probeFfmpeg() {
  const v = spawnSync("ffmpeg", ["-version"], { encoding: "utf8" });
  if (v.status !== 0) return { ok: false };
  const e = spawnSync("ffmpeg", ["-hide_banner", "-encoders"], { encoding: "utf8" });
  return { ok: /\blibx264\b/.test(e.stdout || ""), listing: e.stdout || "" };
}
const FF = probeFfmpeg();
const FF_SKIP = FF.ok ? false : "ffmpeg with libx264 not on PATH";

function pngBytes(w, h, rgb) {
  const chunk = (type, data) => {
    const len = Buffer.alloc(4);
    len.writeUInt32BE(data.length);
    const body = Buffer.concat([Buffer.from(type, "latin1"), data]);
    const crc = Buffer.alloc(4);
    crc.writeUInt32BE(crc32(body) >>> 0);
    return Buffer.concat([len, body, crc]);
  };
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(w, 0);
  ihdr.writeUInt32BE(h, 4);
  ihdr[8] = 8;
  ihdr[9] = 2;
  const row = Buffer.concat([Buffer.from([0]), Buffer.from(Array.from({ length: w }, () => rgb).flat())]);
  const raw = Buffer.concat(Array.from({ length: h }, () => row));
  return Buffer.concat([
    Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]),
    chunk("IHDR", ihdr),
    chunk("IDAT", deflateSync(raw)),
    chunk("IEND", Buffer.alloc(0)),
  ]);
}

// Real tape, real fs, real ffmpeg; `cargo` writes `count` PNGs of size w x h.
function realRun({ w, h, count, args = [], listNvenc = null }) {
  const dir = mkdtempSync(join(tmpdir(), "replay-mp4-real-"));
  const tape = join(dir, "run.tape");
  const out = join(dir, "clip.mp4");
  writeFileSync(tape, tapeBytes([0, count - 1]));
  const o = [];
  const e = [];
  const ffmpegErr = [];
  const deps = {
    env: { E2040_SHA: "abc1234" },
    execSync: (cmd) => (listNvenc !== null && cmd.startsWith("ffmpeg") ? Buffer.from(listNvenc) : execSync(cmd, { stdio: ["ignore", "pipe", "pipe"] })),
    spawnSync: (cmd, a, opts) => {
      if (cmd === "cargo") {
        const fdir = a[a.indexOf("--capture-dir") + 1];
        for (let i = 0; i < count; i += 1) {
          writeFileSync(join(fdir, "frame_" + String(i).padStart(6, "0") + ".png"), pngBytes(w, h, [(i * 20) % 256, 128, 255 - ((i * 20) % 256)]));
        }
        return { status: 0 };
      }
      const r = spawnSync(cmd, a, { input: opts && opts.input, encoding: "utf8" });
      ffmpegErr.push(r.stderr || "");
      return r;
    },
    mkdtemp: (prefix) => mkdtempSync(join(dir, prefix)),
    rm: (p) => rmSync(p, { recursive: true, force: true }),
    existsSync,
    readdirSync,
    readFileSync,
    stdout: (s) => o.push(s),
    stderr: (s) => e.push(s),
  };
  const code = runMain([tape, "--out", out, ...args], deps);
  return { code, out, o: o.join(""), e: e.join(""), dir, ffmpegErr: ffmpegErr.join("") };
}

function probeClip(file) {
  const r = spawnSync("ffprobe", ["-v", "error", "-count_frames", "-select_streams", "v:0", "-show_entries", "stream=codec_name,width,height,nb_read_frames", "-of", "json", file], { encoding: "utf8" });
  if (r.status !== 0) return null;
  return JSON.parse(r.stdout).streams[0];
}

function assertClip(res, w, h, count) {
  assert.equal(res.code, 0, res.e + res.ffmpegErr);
  assert.ok(existsSync(res.out) && statSync(res.out).size > 0);
  assert.equal(readFileSync(res.out).subarray(4, 8).toString("latin1"), "ftyp");
  const info = probeClip(res.out);
  if (info) {
    assert.equal(info.codec_name, "h264");
    assert.deepEqual([info.width, info.height, Number(info.nb_read_frames)], [w, h, count]);
  }
}

test("real ffmpeg: numbered PNGs become an h264 mp4 with the box's own encoder pick", { skip: FF_SKIP }, () => {
  const res = realRun({ w: 320, h: 180, count: 12 });
  try {
    assertClip(res, 320, 180, 12);
    assert.match(res.o, /wrote .*clip\.mp4 \((h264_nvenc|libx264), 12 frames\)/);
    assert.ok(!existsSync(join(res.dir, "replay-mp4-")), "frame dir is gone after a good encode");
  } finally {
    rmSync(res.dir, { recursive: true, force: true });
  }
});

test("real ffmpeg: a listed nvenc that cannot encode a tiny frame still ends in a valid clip", { skip: FF_SKIP }, () => {
  // 64x64 is under every NVENC minimum, so a box that lists the encoder must retry libx264.
  const res = realRun({ w: 64, h: 64, count: 10, listNvenc: " V....D h264_nvenc NVIDIA\n V....D libx264 x\n" });
  try {
    assertClip(res, 64, 64, 10);
    assert.match(res.o, /\((h264_nvenc|libx264), 10 frames\)/);
    // The attempt list starts at nvenc, so a libx264 result means nvenc failed first.
    if (res.o.includes("(libx264,")) assert.match(res.e, /h264_nvenc failed \(status \d+\); retrying with libx264/);
  } finally {
    rmSync(res.dir, { recursive: true, force: true });
  }
});

test("real ffmpeg: --pipe streams the PNG sequence through image2pipe", { skip: FF_SKIP }, () => {
  const res = realRun({ w: 128, h: 96, count: 8, args: ["--pipe", "--keep-frames"], listNvenc: " V....D libx264 x\n" });
  try {
    assertClip(res, 128, 96, 8);
    assert.match(res.o, /\(libx264 pipe, 8 frames\)/);
    assert.match(res.o, /frames kept in /);
  } finally {
    rmSync(res.dir, { recursive: true, force: true });
  }
});

test("the real CLI with no ffmpeg on PATH exits 2 with the typed install hint", () => {
  const dir = mkdtempSync(join(tmpdir(), "replay-mp4-nopath-"));
  try {
    const tape = join(dir, "run.tape");
    writeFileSync(tape, tapeBytes([0, 59]));
    const env = Object.fromEntries(Object.entries(process.env).filter(([k]) => k.toLowerCase() !== "path"));
    env.PATH = dir;
    const tool = fileURLToPath(new URL("./replay-to-mp4.mjs", import.meta.url));
    const r = spawnSync(process.execPath, [tool, tape, "--out", join(dir, "c.mp4")], { encoding: "utf8", env });
    assert.equal(r.status, 2, r.stderr);
    assert.match(r.stderr, /E_FFMPEG_MISSING ffmpeg not found; install it/);
    assert.equal(existsSync(join(dir, "c.mp4")), false);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});
