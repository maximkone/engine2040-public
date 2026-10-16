// Run: node --test tools/replay-to-mp4.test.mjs
// GS25 replay to video: tape -> PNGs -> mp4. Pure argv builders stay
// pinned with stub runners so no cargo or ffmpeg runs in the test.
import assert from "node:assert/strict";
import { existsSync, mkdirSync, mkdtempSync, readdirSync, readFileSync, rmSync, statSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import {
  FFMPEG_INSTALL_HINT,
  FRAMERATE,
  FRAME_PATTERN,
  USAGE,
  UsageError,
  buildCaptureArgv,
  buildFfmpegArgv,
  parseArgs,
  pickEncoder,
  resolveSha,
  runMain,
} from "./replay-to-mp4.mjs";

function stubDeps({ encoders = " Encoder h264_nvenc ", shaEnv = "abc1234", captureStatus = 0, ffmpegStatus = 0, tapeExists = true } = {}) {
  const out = [];
  const err = [];
  const spawns = [];
  const deps = {
    env: { E2040_SHA: shaEnv },
    execSync: (cmd) => {
      if (cmd.includes("ffmpeg")) return Buffer.from(encoders);
      if (cmd.includes("git")) return Buffer.from("deadbeef1234567\n");
      throw new Error("unexpected exec: " + cmd);
    },
    spawnSync: (cmd, args, opts) => {
      spawns.push([cmd, args, opts]);
      return { status: cmd === "ffmpeg" ? ffmpegStatus : captureStatus };
    },
    mkdtemp: (prefix) => join("/tmp", prefix + "test"),
    existsSync: (p) => (typeof tapeExists === "function" ? tapeExists(p) : tapeExists),
    readdirSync: (d) => readdirSync(d),
    readFileSync: (p) => readFileSync(p),
    stdout: (s) => out.push(s),
    stderr: (s) => err.push(s),
  };
  return { deps, out, err, spawns };
}

test("constants pin the 60fps numbered-frame contract", () => {
  assert.equal(FRAMERATE, 60);
  assert.equal(FRAME_PATTERN, "frame_%06d.png");
  assert.ok(USAGE.includes("<tape>") && USAGE.includes("--out clip.mp4"));
  assert.ok(FFMPEG_INSTALL_HINT.includes("ffmpeg not found"));
  assert.ok(new UsageError("x") instanceof Error);
});

test("parseArgs accepts tape, out and max-ticks", () => {
  assert.deepEqual(parseArgs(["tape.json", "--out", "clip.mp4"]), { tape: "tape.json", out: "clip.mp4", maxTicks: null, pipe: false });
  assert.deepEqual(parseArgs(["tape.json", "--out", "clip.mp4", "--max-ticks", "600"]), {
    tape: "tape.json",
    out: "clip.mp4",
    maxTicks: 600,
    pipe: false,
  });
  assert.deepEqual(parseArgs(["tape.json", "--out", "clip.mp4", "--pipe"]), {
    tape: "tape.json",
    out: "clip.mp4",
    maxTicks: null,
    pipe: true,
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
});

test("pickEncoder prefers nvenc only when listed", () => {
  assert.equal(pickEncoder("Encoders:\n V..... h264_nvenc\n"), "h264_nvenc");
  assert.equal(pickEncoder("Encoders:\n V..... libx264\n"), "libx264");
  assert.equal(pickEncoder("h264_nvenc_extra"), "libx264");
});

test("buildCaptureArgv pins the exact arena argv", () => {
  const got = buildCaptureArgv({ tape: "t.json", dir: "/tmp/d", maxTicks: null, sha: "abc1234" });
  assert.equal(got.cmd, "cargo");
  assert.deepEqual(got.args, [
    "run", "-p", "engine-game", "--bin", "arena", "--",
    "--tape", "t.json",
    "--capture-dir", "/tmp/d",
    "--capture-every", "1",
    "--sha", "abc1234",
    "--fps", "60",
  ]);
  const capped = buildCaptureArgv({ tape: "t.json", dir: "/tmp/d", maxTicks: 600, sha: "abc1234" });
  assert.ok(capped.args.includes("--frames") && capped.args.includes("600"));
});

test("buildFfmpegArgv pins both encoder cases", () => {
  const [nvCmd, nvArgs] = buildFfmpegArgv({ pattern: "frame_%06d.png", out: "clip.mp4", encoder: "h264_nvenc" });
  assert.equal(nvCmd, "ffmpeg");
  assert.deepEqual(nvArgs, ["-framerate", "60", "-i", "frame_%06d.png", "-c:v", "h264_nvenc", "-pix_fmt", "yuv420p", "clip.mp4"]);
  const [, xArgs] = buildFfmpegArgv({ pattern: "frame_%06d.png", out: "clip.mp4", encoder: "libx264" });
  assert.deepEqual(xArgs, [
    "-framerate", "60", "-i", "frame_%06d.png",
    "-c:v", "libx264", "-preset", "veryfast", "-crf", "18", "-pix_fmt", "yuv420p", "clip.mp4",
  ]);
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
  assert.ok(s.spawns[0][1].includes("t.json"));
  assert.equal(s.spawns[1][0], "ffmpeg");
  assert.ok(s.spawns[1][1].includes("h264_nvenc"));
  assert.ok(s.out.join("").includes("wrote c.mp4 (h264_nvenc)"));
  const x = stubDeps({ encoders: "libx264 only" });
  assert.equal(runMain(["t.json", "--out", "c.mp4"], x.deps), 0);
  assert.ok(x.spawns[1][1].includes("libx264"));
  assert.ok(x.out.join("").includes("(libx264)"));
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
  writeFileSync(tape, '{"ticks":3}\n');
  const s = stubDeps({ tapeExists: (p) => existsSync(p) });
  s.deps.mkdtemp = (prefix) => join(dir, prefix + "frames");
  const origSpawn = s.deps.spawnSync;
  s.deps.spawnSync = (cmd, args) => {
    const r = origSpawn(cmd, args);
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
  assert.deepEqual(nv, ["-f", "image2pipe", "-framerate", "60", "-i", "-", "-c:v", "h264_nvenc", "-pix_fmt", "yuv420p", "clip.mp4"]);
  const [, x] = buildFfmpegArgv({ out: "clip.mp4", encoder: "libx264", pipe: true });
  assert.deepEqual(x, ["-f", "image2pipe", "-framerate", "60", "-i", "-", "-c:v", "libx264", "-preset", "veryfast", "-crf", "18", "-pix_fmt", "yuv420p", "clip.mp4"]);
});

test("runMain pipe streams 600 PNG bytes with pinned argv", () => {
  const dir = mkdtempSync(join(tmpdir(), "replay-mp4-pipe-"));
  const tape = join(dir, "tape.json");
  const out = join(dir, "clip-pipe.mp4");
  writeFileSync(tape, '{"ticks":600}\n');
  const frames = join(dir, "frames");
  mkdirSync(frames, { recursive: true });
  const bytes = Buffer.from("fake-png");
  const s = stubDeps({ encoders: " h264_nvenc ", tapeExists: (p) => existsSync(p) });
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
  const s = stubDeps();
  s.deps.readdirSync = () => [];
  assert.equal(runMain(["t.json", "--out", "c.mp4", "--pipe"], s.deps), 1);
  assert.ok(s.err.join("").includes("pipe read failed"));
  assert.equal(s.spawns.length, 1);
});
