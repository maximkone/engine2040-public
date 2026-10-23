// ENG19: the ship check must run the shipped build, not only build it. These
// tests hold the checker to the receipt it accepts: the golden hash and a
// positive tick pass, a wrong hash or a zero tick is refused by name, a
// receipt that did not verify itself is refused, and a real broken process
// turns the tool red. They also pin the arena boot line and the workflow call.
import assert from "node:assert/strict";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { deflateSync } from "node:zlib";
import {
  adapterLabel,
  adapterName,
  allowSoftware,
  checkReceipt,
  checkSmoke,
  childEnv,
  frameLine,
  frameVerdict,
  GOLDEN_HASH,
  lookBar,
  main,
  noAdapter,
  parseReference,
  parseReferenceShot,
  parseSmoke,
  runArena,
  runFrameLeg,
  runProof,
  withSoftware,
} from "./ship-run.mjs";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");

// The text the root proof binary prints for the golden run (`--verify`).
const GOOD = `engine2040 headless proof:
  ticks=600 final_hash=${GOLDEN_HASH}
  seed=2040 inputs=0 state_hash=0123456789abcdef backend=null mode=run verified=true
`;
const BAD_HASH = GOOD.replace(GOLDEN_HASH, "0000000000000000");
const ZERO_TICK = GOOD.replace("ticks=600", "ticks=0");

// The arena's receipts, window and headless, with and without an adapter. The
// headless one carries the final checkpoint hash and the forced route.
const HEADLESS_SMOKE = `arena headless smoke: ticks=600 frames=0 hash=9f162a2fe205f0ff hud_cells=5 hud_quads=3 mixed_frames=0 headless=forced audio="audio muted: no device opened; mixer voices render offline" presenter="presenter skipped: no wgpu adapter on this machine" display="none requested"`;
const WINDOW_SMOKE = `arena smoke: ticks=30 frames=30 alpha=1.000 hud_cells=5 hud_quads=3 fight_draws=9 audio="audio muted: no device opened; mixer voices render offline" presenter="presenter surface: Microsoft Basic Render Driver (10.0.1)"`;
const REAL_GPU_SMOKE = `arena headless smoke: ticks=600 frames=600 hash=9f162a2fe205f0ff hud_cells=5 hud_quads=3 mixed_frames=0 headless=forced audio="audio muted" presenter="presenter twin: NVIDIA GeForce RTX 3050 (537.13)" display="none requested"`;
// The same run on a software rasterizer: frames are drawn, the presenter is
// named, and no hardware is involved.
const WARP_SMOKE = HEADLESS_SMOKE.replace("frames=0", "frames=600").replace(
  'presenter="presenter skipped: no wgpu adapter on this machine"',
  'presenter="presenter twin: Microsoft Basic Render Driver (10.0.1)"',
);
const SHORT_SMOKE = HEADLESS_SMOKE.replace("ticks=600", "ticks=30");
const NO_HASH_SMOKE = HEADLESS_SMOKE.replace(" hash=9f162a2fe205f0ff", "");
const MOVED_SMOKE = HEADLESS_SMOKE.replace("hash=9f162a2fe205f0ff", "hash=0000000000000000");

// What a hosted runner with no GPU wrote before the `--headless` route: the
// window opened, the presenter found no adapter and the binary exited before
// any smoke line. The headless route must never need an adapter, so this text
// beside no receipt is a red, not a skip.
const NO_ADAPTER_BOOT = `arena backend=auto fallback=dx12 reason=compiled-in default; no pilot report yet
arena vram budget_mb=7523 usage_mb=0 headroom_mb=7523 dedicated_mb=0
arena notice: audio muted: no device opened; mixer voices render offline
arena: presenter error (no wgpu adapter available)
`;

// A temp folder that goes away with the test.
function tempDir(t) {
  const dir = mkdtempSync(join(tmpdir(), "ship-run-"));
  t.after(() => rmSync(dir, { recursive: true, force: true }));
  return dir;
}

// A minimal 8-bit RGBA PNG (color type 6, filter 0) so the frame leg reads
// real pixels instead of a manifest. Solid colors keep the metrics easy to
// read: an identical pair scores 10000/0, and a checker with its inverse is
// under both bars.
function crc32(bytes) {
  let crc = 0xffffffff;
  for (const byte of bytes) {
    crc ^= byte;
    for (let bit = 0; bit < 8; bit += 1) crc = crc & 1 ? 0xedb88320 ^ (crc >>> 1) : crc >>> 1;
  }
  return (crc ^ 0xffffffff) >>> 0;
}

function pngChunk(type, data) {
  const head = Buffer.alloc(8);
  head.writeUInt32BE(data.length, 0);
  head.write(type, 4, "ascii");
  const crc = Buffer.alloc(4);
  crc.writeUInt32BE(crc32(Buffer.concat([Buffer.from(type, "ascii"), data])), 0);
  return Buffer.concat([head, data, crc]);
}

function rgbaPng(width, height, pixel) {
  const stride = width * 4;
  const raw = Buffer.alloc((stride + 1) * height);
  for (let y = 0; y < height; y += 1) {
    raw[y * (stride + 1)] = 0;
    for (let x = 0; x < width; x += 1) {
      const [red, green, blue] = pixel(x, y);
      const at = y * (stride + 1) + 1 + x * 4;
      raw[at] = red;
      raw[at + 1] = green;
      raw[at + 2] = blue;
      raw[at + 3] = 255;
    }
  }
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(width, 0);
  ihdr.writeUInt32BE(height, 4);
  ihdr[8] = 8; // bit depth
  ihdr[9] = 6; // RGBA
  return Buffer.concat([
    Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]),
    pngChunk("IHDR", ihdr),
    pngChunk("IDAT", deflateSync(raw)),
    pngChunk("IEND", Buffer.alloc(0)),
  ]);
}

function solidPng(width, height, color = [128, 128, 128]) {
  return rgbaPng(width, height, () => color);
}

function checkerPng(width, height, colors, invert = false) {
  return rgbaPng(width, height, (x, y) =>
    ((x >> 3) + (y >> 3) + (invert ? 1 : 0)) & 1 ? colors[1] : colors[0],
  );
}

// The reference run's output for a stub binary: one shot line plus the
// summary the tool parses. `bytes` is the filed size, not read back.
function referenceOutput({
  shots = 10,
  size = "64x48",
  adapter = "NVIDIA GeForce RTX 3050 6GB Laptop GPU",
  shot = "arena600-t000",
  bytes = 2048,
} = {}) {
  return [
    `arena reference: tier=native shot=${shot} tick=0 size=${size} png="native/${shot}.png" png_fnv=0123456789abcdef bytes=${bytes}`,
    `arena reference: tier=native shots=${shots} camera=arena600 seed=2040 size=${size} adapter="${adapter}" driver="32.0.16.1062 " fragment="native/fragment.json"`,
    "",
  ].join("\n");
}

// A reference PNG, a capture folder, and (unless asked otherwise) the shot
// PNG the stub capture run would have written.
function frameFixture(
  t,
  {
    width = 64,
    height = 48,
    reference = [128, 128, 128],
    capture = [128, 128, 128],
    referenceImage = null,
    captureImage = null,
    writeCapture = true,
  } = {},
) {
  const dir = tempDir(t);
  const referencePng = join(dir, "reference.png");
  const captureDir = join(dir, "capture");
  mkdirSync(join(captureDir, "native"), { recursive: true });
  writeFileSync(referencePng, referenceImage ?? solidPng(width, height, reference));
  if (writeCapture) {
    writeFileSync(
      join(captureDir, "native", "arena600-t000.png"),
      captureImage ?? solidPng(width, height, capture),
    );
  }
  return { dir, referencePng, captureDir };
}

// A spawn that always returns the same text, so main() runs without a build.
const fixedSpawn = (stdout, status = 0) => () => ({ status, stdout, stderr: "" });

// Two runs in one main() call: the proof answers --verify, the arena plays.
const twoRuns = (proofOut, arenaOut) => (command, args) => ({
  status: 0,
  stdout: args.includes("--verify") ? proofOut : arenaOut,
  stderr: "",
});

// The same two runs when the arena exits nonzero (a presenter that cannot
// paint) while the proof stays green.
const twoRunsStatus = (proofOut, arenaOut, arenaStatus) => (command, args) => ({
  status: args.includes("--verify") ? 0 : arenaStatus,
  stdout: args.includes("--verify") ? proofOut : arenaOut,
  stderr: "",
});

// Two game runs plus a frame-leg counter: the proof answers --verify, the
// game plays twice, and the reference capture is counted instead of run.
function twoRunsCounting(proofOut, arenaOut) {
  let frames = 0;
  const spawn = (command, args) => {
    if (args.includes("--verify")) return { status: 0, stdout: proofOut, stderr: "" };
    if (args.includes("--reference-set")) {
      frames += 1;
      return { status: 0, stdout: "", stderr: "" };
    }
    return { status: 0, stdout: arenaOut, stderr: "" };
  };
  return { spawn, frameRuns: () => frames };
}

function collect() {
  const lines = [];
  const errors = [];
  return {
    lines,
    errors,
    out: (line) => lines.push(line),
    err: (line) => errors.push(line),
  };
}

test("the golden receipt passes and the tool quotes tick and hash", (t) => {
  const dir = tempDir(t);
  const proof = join(dir, "engine2040.exe");
  writeFileSync(proof, "not a real build");
  const io = collect();
  const code = main(["--proof", proof], { ...io, spawn: fixedSpawn(GOOD) });
  assert.equal(code, 0);
  assert.ok(io.lines.includes(`SHIP-RUN tick=600 hash=${GOLDEN_HASH}`), io.lines.join("\n"));
  assert.ok(io.lines.some((line) => line.startsWith("SHIP-RUN PASS")), io.lines.join("\n"));
  assert.equal(io.errors.length, 0);
});

test("a hash that is not the golden is refused by name", () => {
  const check = checkReceipt(BAD_HASH);
  assert.equal(check.ok, false);
  assert.equal(check.reason, "HASH_NOT_GOLDEN");
  assert.match(check.detail, /final_hash=0000000000000000/);
  assert.match(check.detail, new RegExp(GOLDEN_HASH));
});

test("a zero tick is refused by name", () => {
  const check = checkReceipt(ZERO_TICK);
  assert.equal(check.ok, false);
  assert.equal(check.reason, "ZERO_TICK");
});

test("a missing receipt is refused by name", () => {
  const check = checkReceipt("build ok: nothing ran\n");
  assert.equal(check.ok, false);
  assert.equal(check.reason, "MISSING_RECEIPT");
});

test("a receipt with no run line is refused by name", () => {
  const check = checkReceipt("engine2040 headless proof:\n  seed=2040 inputs=0\n");
  assert.equal(check.ok, false);
  assert.equal(check.reason, "NO_RUN_LINE");
});

test("a receipt that did not verify itself is refused by name", () => {
  const unverified = GOOD.replace("verified=true", "verified=false");
  const check = checkReceipt(unverified);
  assert.equal(check.ok, false);
  assert.equal(check.reason, "NOT_VERIFIED");
  // The same text passes when the caller does not ask for verification.
  assert.equal(checkReceipt(unverified, { requireVerified: false }).ok, true);
});

test("a broken receipt makes the CLI red and prints the named refusal", (t) => {
  const dir = tempDir(t);
  const proof = join(dir, "engine2040.exe");
  writeFileSync(proof, "not a real build");
  const io = collect();
  const code = main(["--proof", proof], { ...io, spawn: fixedSpawn(BAD_HASH) });
  assert.equal(code, 1);
  assert.ok(
    io.lines.includes(
      `SHIP-RUN FAIL [HASH_NOT_GOLDEN] final_hash=0000000000000000; want the golden ${GOLDEN_HASH}`,
    ),
    io.lines.join("\n"),
  );
  // The end of the run output is quoted, so the red names what was read.
  assert.ok(io.lines.includes(`  ticks=600 final_hash=0000000000000000`), io.lines.join("\n"));
  assert.match(io.errors.join("\n"), /SHIP-RUN FAIL/);
});

test("a real child process with a broken receipt is refused by name", (t) => {
  const dir = tempDir(t);
  const script = join(dir, "broken-proof.mjs");
  writeFileSync(script, `process.stdout.write(${JSON.stringify(BAD_HASH)});\n`, "utf8");
  const result = runProof(process.execPath, { args: [script] });
  assert.equal(result.status, 0);
  assert.equal(result.ok, false);
  assert.equal(result.reason, "HASH_NOT_GOLDEN");
});

test("a real child process with the golden receipt passes", (t) => {
  const dir = tempDir(t);
  const script = join(dir, "good-proof.mjs");
  writeFileSync(script, `process.stdout.write(${JSON.stringify(GOOD)});\n`, "utf8");
  const result = runProof(process.execPath, { args: [script] });
  assert.equal(result.status, 0);
  assert.equal(result.ok, true);
  assert.equal(result.ticks, 600);
  assert.equal(result.hash, GOLDEN_HASH);
});

test("the headless game line is read with its adapter and final hash", () => {
  const smoke = parseSmoke(HEADLESS_SMOKE);
  assert.equal(smoke.kind, "headless");
  assert.equal(smoke.ticks, 600);
  assert.equal(smoke.frames, 0);
  assert.equal(smoke.hash, "9f162a2fe205f0ff");
  assert.equal(smoke.headless, "forced");
  assert.equal(smoke.adapter, "none");
  assert.equal(checkSmoke(HEADLESS_SMOKE).ok, true);
});

test("the game receipt must carry the requested ticks and a final hash", () => {
  assert.equal(checkSmoke(SHORT_SMOKE, { expectTicks: 600 }).reason, "GAME_SHORT_RUN");
  assert.equal(checkSmoke(NO_HASH_SMOKE).reason, "GAME_NO_HASH");
  assert.equal(checkSmoke(HEADLESS_SMOKE, { expectTicks: 600 }).ok, true);
  // A receipt that never ticked can never look like a hash.
  const zero = HEADLESS_SMOKE.replace("ticks=600", "ticks=0").replace(
    "hash=9f162a2fe205f0ff",
    "hash=-",
  );
  assert.equal(checkSmoke(zero).reason, "GAME_ZERO_TICK");
});

test("a software presenter is named warp and a real adapter keeps its name", () => {
  const window = parseSmoke(WINDOW_SMOKE);
  assert.equal(window.kind, "window");
  assert.equal(adapterName(window.presenter), "Microsoft Basic Render Driver");
  assert.equal(adapterLabel(adapterName(window.presenter)), "warp");
  const real = parseSmoke(REAL_GPU_SMOKE);
  assert.equal(adapterLabel(real.adapter), "NVIDIA GeForce RTX 3050");
});

test("the reference capture line is read with its adapter and shot", () => {
  const text = referenceOutput();
  const reference = parseReference(text);
  assert.equal(reference.tier, "native");
  assert.equal(reference.shots, 10);
  assert.equal(reference.width, 64);
  assert.equal(reference.height, 48);
  assert.equal(reference.adapter, "NVIDIA GeForce RTX 3050 6GB Laptop GPU");
  assert.equal(reference.driver, "32.0.16.1062");
  const shot = parseReferenceShot(text);
  assert.equal(shot.bytes, 2048);
  assert.equal(shot.png, "native/arena600-t000.png");
  assert.equal(parseReferenceShot(text, "arena600-t060"), null);
});

test("the frame bar is the look gate's own SSIM threshold or FLIP floor", () => {
  const bar = lookBar();
  assert.deepEqual(bar, { ssimMin: 9800, flipMax: 1200 });
  assert.equal(frameVerdict(9800, 9999, bar), "pass");
  assert.equal(frameVerdict(0, 1200, bar), "pass");
  assert.equal(frameVerdict(9799, 1201, bar), "fail");
  // A missing floor is loud, never a silent default.
  assert.throws(() => lookBar(join(ROOT, "fixtures")), /product-bar\.json/);
});

test("an identical shipped shot scores at the bar and passes", (t) => {
  const fx = frameFixture(t);
  const result = runFrameLeg("C:\\ship\\arena.exe", {
    captureDir: fx.captureDir,
    referencePng: fx.referencePng,
    size: "64x48",
    spawn: () => ({ status: 0, stdout: referenceOutput({ size: "64x48" }), stderr: "" }),
  });
  assert.equal(result.ok, true, result.detail);
  assert.equal(result.score, 10000);
  assert.equal(result.flip, 0);
  assert.equal(result.verdict, "pass");
  assert.equal(result.software, false);
  assert.equal(
    frameLine(result),
    'SHIP-FRAME adapter="NVIDIA GeForce RTX 3050 6GB Laptop GPU" resolution=64x48 score=10000 flip=0 verdict=pass',
  );
});

test("a shipped shot under both bars fails", (t) => {
  const colors = [
    [200, 180, 120],
    [40, 30, 20],
  ];
  const fx = frameFixture(t, {
    referenceImage: checkerPng(64, 48, colors, false),
    captureImage: checkerPng(64, 48, colors, true),
  });
  const result = runFrameLeg("C:\\ship\\arena.exe", {
    captureDir: fx.captureDir,
    referencePng: fx.referencePng,
    size: "64x48",
    spawn: () => ({ status: 0, stdout: referenceOutput({ size: "64x48" }), stderr: "" }),
  });
  assert.equal(result.ok, true, result.detail);
  assert.ok(result.score < 9800, `ssim ${result.score}`);
  assert.ok(result.flip > 1200, `flip ${result.flip}`);
  assert.equal(result.verdict, "fail");
  assert.match(frameLine(result), /verdict=fail$/);
});

test("a short reference run is refused by name", (t) => {
  const fx = frameFixture(t);
  const result = runFrameLeg("C:\\ship\\arena.exe", {
    captureDir: fx.captureDir,
    referencePng: fx.referencePng,
    size: "64x48",
    spawn: () => ({ status: 0, stdout: referenceOutput({ shots: 3, size: "64x48" }), stderr: "" }),
  });
  assert.equal(result.ok, false);
  assert.equal(result.reason, "FRAME_SHORT_RUN");
  assert.match(result.detail, /filed 3 shots/);
  assert.match(result.detail, /files 10/);
});

test("a capture run that writes no PNG is refused by name", (t) => {
  const fx = frameFixture(t, { writeCapture: false });
  const result = runFrameLeg("C:\\ship\\arena.exe", {
    captureDir: fx.captureDir,
    referencePng: fx.referencePng,
    size: "64x48",
    spawn: () => ({ status: 0, stdout: referenceOutput({ size: "64x48" }), stderr: "" }),
  });
  assert.equal(result.ok, false);
  assert.equal(result.reason, "FRAME_NO_CAPTURE");
  assert.match(result.detail, /arena600-t000\.png/);
});

test("a capture that names a software adapter is labelled warp, never pass", (t) => {
  const fx = frameFixture(t);
  const result = runFrameLeg("C:\\ship\\arena.exe", {
    captureDir: fx.captureDir,
    referencePng: fx.referencePng,
    size: "64x48",
    spawn: () => ({
      status: 0,
      stdout: referenceOutput({ size: "64x48", adapter: "Microsoft Basic Render Driver" }),
      stderr: "",
    }),
  });
  assert.equal(result.ok, true, result.detail);
  assert.equal(result.software, true);
  assert.equal(
    frameLine(result),
    "SHIP-FRAME adapter=warp reason=software-adapter resolution=64x48 score=10000 flip=0 verdict=software",
  );
});

test("the capture child gets the software flag only when the leg is software", (t) => {
  const fx = frameFixture(t);
  const seen = [];
  const spawn = (command, args, options) => {
    seen.push(options.env);
    return { status: 0, stdout: referenceOutput({ size: "64x48" }), stderr: "" };
  };
  const env = { ...childEnv(), E2040_ALLOW_SOFTWARE: "1" };
  const base = { captureDir: fx.captureDir, referencePng: fx.referencePng, size: "64x48", env, spawn };
  runFrameLeg("C:\\ship\\arena.exe", { ...base, software: false });
  runFrameLeg("C:\\ship\\arena.exe", { ...base, software: true });
  assert.equal(allowSoftware(env), true);
  assert.equal(seen[0].E2040_ALLOW_SOFTWARE, undefined);
  assert.equal(seen[1].E2040_ALLOW_SOFTWARE, "1");
  assert.equal(withSoftware({}, false).E2040_ALLOW_SOFTWARE, undefined);
});

test("the arena leg scores a real adapter and prints the SHIP-FRAME receipt", (t) => {
  const dir = tempDir(t);
  const proof = join(dir, "engine2040.exe");
  const arena = join(dir, "arena.exe");
  writeFileSync(proof, "not a real build");
  writeFileSync(arena, "not a real build");
  const fx = frameFixture(t, { width: 1280, height: 720 });
  const io = collect();
  let frameRuns = 0;
  const spawn = (command, args) => {
    if (args.includes("--verify")) return { status: 0, stdout: GOOD, stderr: "" };
    if (args.includes("--reference-set")) {
      frameRuns += 1;
      return { status: 0, stdout: referenceOutput({ size: "1280x720" }), stderr: "" };
    }
    return { status: 0, stdout: REAL_GPU_SMOKE, stderr: "" };
  };
  const code = main(["--proof", proof, "--arena", arena], {
    ...io,
    spawn,
    frame: { captureDir: fx.captureDir, referencePng: fx.referencePng },
  });
  assert.equal(code, 0, io.lines.join("\n"));
  assert.equal(frameRuns, 1);
  assert.ok(
    io.lines.includes(
      'SHIP-FRAME adapter="NVIDIA GeForce RTX 3050 6GB Laptop GPU" resolution=1280x720 score=10000 flip=0 verdict=pass',
    ),
    io.lines.join("\n"),
  );
  assert.equal(io.lines.some((line) => line.includes("frame-score-not-wired")), false);
  assert.equal(io.lines.some((line) => line.startsWith("SHIP-FRAME skipped")), false);
});

test("a none adapter keeps the skip line and never runs the frame leg", (t) => {
  const dir = tempDir(t);
  const proof = join(dir, "engine2040.exe");
  const arena = join(dir, "arena.exe");
  writeFileSync(proof, "not a real build");
  writeFileSync(arena, "not a real build");
  const io = collect();
  const runs = twoRunsCounting(GOOD, HEADLESS_SMOKE);
  const code = main(["--proof", proof, "--arena", arena], { ...io, spawn: runs.spawn });
  assert.equal(code, 0, io.lines.join("\n"));
  assert.equal(runs.frameRuns(), 0);
  assert.ok(io.lines.includes("SHIP-FRAME skipped adapter=none reason=no-hardware-adapter"));
});

test("a warp adapter without the opt-in keeps the skip line", (t) => {
  const dir = tempDir(t);
  const proof = join(dir, "engine2040.exe");
  const arena = join(dir, "arena.exe");
  writeFileSync(proof, "not a real build");
  writeFileSync(arena, "not a real build");
  const io = collect();
  const runs = twoRunsCounting(GOOD, WARP_SMOKE);
  const code = main(["--proof", proof, "--arena", arena], { ...io, spawn: runs.spawn });
  assert.equal(code, 0, io.lines.join("\n"));
  assert.equal(runs.frameRuns(), 0);
  assert.ok(io.lines.includes("SHIP-FRAME skipped adapter=warp reason=no-hardware-adapter"));
});

test("the software opt-in scores under WARP and never reads as a pass", (t) => {
  const dir = tempDir(t);
  const proof = join(dir, "engine2040.exe");
  const arena = join(dir, "arena.exe");
  writeFileSync(proof, "not a real build");
  writeFileSync(arena, "not a real build");
  const fx = frameFixture(t);
  const io = collect();
  let frameEnv = null;
  const spawn = (command, args, options) => {
    if (args.includes("--verify")) return { status: 0, stdout: GOOD, stderr: "" };
    if (args.includes("--reference-set")) {
      frameEnv = options.env;
      return {
        status: 0,
        stdout: referenceOutput({ size: "64x48", adapter: "Microsoft Basic Render Driver" }),
        stderr: "",
      };
    }
    return { status: 0, stdout: WARP_SMOKE, stderr: "" };
  };
  const code = main(["--proof", proof, "--arena", arena], {
    ...io,
    spawn,
    env: { ...childEnv(), E2040_ALLOW_SOFTWARE: "1" },
    frame: { captureDir: fx.captureDir, referencePng: fx.referencePng, size: "64x48" },
  });
  assert.equal(code, 0, io.lines.join("\n"));
  assert.equal(frameEnv.E2040_ALLOW_SOFTWARE, "1");
  const line = io.lines.find((row) => row.startsWith("SHIP-FRAME "));
  assert.equal(
    line,
    "SHIP-FRAME adapter=warp reason=software-adapter resolution=64x48 score=10000 flip=0 verdict=software",
  );
  assert.equal(line.includes("verdict=pass"), false);
  assert.equal(io.lines.join("\n").includes("NVIDIA"), false);
  assert.equal(io.lines.some((row) => row.startsWith("SHIP-FRAME skipped")), false);
});

test("the arena leg quotes the game receipt and names the frame skip", (t) => {
  const dir = tempDir(t);
  const proof = join(dir, "engine2040.exe");
  const arena = join(dir, "arena.exe");
  writeFileSync(proof, "not a real build");
  writeFileSync(arena, "not a real build");
  const io = collect();
  const code = main(
    ["--proof", proof, "--arena", arena],
    { ...io, spawn: twoRuns(GOOD, HEADLESS_SMOKE) },
  );
  assert.equal(code, 0, io.lines.join("\n"));
  const game = io.lines.find((line) => line.startsWith("SHIP-GAME tick="));
  assert.equal(
    game,
    "SHIP-GAME tick=600 hash=9f162a2fe205f0ff frames=0 seconds=10.000 adapter=none headless=forced runs=2",
  );
  assert.ok(
    io.lines.includes('SHIP-GAME presenter="presenter skipped: no wgpu adapter on this machine"'),
    io.lines.join("\n"),
  );
  assert.ok(
    io.lines.includes("SHIP-FRAME skipped adapter=none reason=no-hardware-adapter"),
    io.lines.join("\n"),
  );
});

test("two game runs with different final hashes make the CLI red", (t) => {
  const dir = tempDir(t);
  const proof = join(dir, "engine2040.exe");
  const arena = join(dir, "arena.exe");
  writeFileSync(proof, "not a real build");
  writeFileSync(arena, "not a real build");
  let runs = 0;
  const spawn = (command, args) => {
    if (args.includes("--verify")) return { status: 0, stdout: GOOD, stderr: "" };
    runs += 1;
    return { status: 0, stdout: runs === 1 ? HEADLESS_SMOKE : MOVED_SMOKE, stderr: "" };
  };
  const io = collect();
  const code = main(["--proof", proof, "--arena", arena], { ...io, spawn });
  assert.equal(code, 1);
  assert.ok(
    io.lines.some((line) => line.startsWith("SHIP-GAME FAIL [GAME_HASH_MOVED]")),
    io.lines.join("\n"),
  );
  assert.match(io.errors.join("\n"), /SHIP-RUN FAIL/);
});

test("a game run with no smoke line makes the CLI red", (t) => {
  const dir = tempDir(t);
  const proof = join(dir, "engine2040.exe");
  const arena = join(dir, "arena.exe");
  writeFileSync(proof, "not a real build");
  writeFileSync(arena, "not a real build");
  const io = collect();
  const code = main(
    ["--proof", proof, "--arena", arena],
    { ...io, spawn: twoRuns(GOOD, "arena: no display\n") },
  );
  assert.equal(code, 1);
  assert.ok(
    io.lines.some((line) => line.startsWith("SHIP-GAME FAIL [GAME_NO_RECEIPT]")),
    io.lines.join("\n"),
  );
});

// The mirrored runner has no wgpu adapter, but the shipped check passes
// `--headless`, so the game plays with no presenter instead of asking for one.
// A boot that still dies on "no wgpu adapter" is the old window route: it
// reaches no receipt and stays red, with the adapter named in the detail.
test("a no-adapter boot without the headless receipt is red, not skipped", (t) => {
  const dir = tempDir(t);
  const proof = join(dir, "engine2040.exe");
  const arena = join(dir, "arena.exe");
  writeFileSync(proof, "not a real build");
  writeFileSync(arena, "not a real build");
  const io = collect();
  const code = main(
    ["--proof", proof, "--arena", arena],
    { ...io, spawn: twoRunsStatus(GOOD, NO_ADAPTER_BOOT, 1) },
  );
  assert.equal(code, 1);
  assert.ok(io.lines.includes(`SHIP-RUN tick=600 hash=${GOLDEN_HASH}`), io.lines.join("\n"));
  assert.ok(
    io.lines.some((line) =>
      line.startsWith("SHIP-GAME FAIL [GAME_NO_RECEIPT] the arena binary exited 1; the headless route still asked for a wgpu adapter"),
    ),
    io.lines.join("\n"),
  );
  assert.match(io.errors.join("\n"), /SHIP-RUN FAIL/);
});

test("any other arena failure stays red", (t) => {
  const dir = tempDir(t);
  const proof = join(dir, "engine2040.exe");
  const arena = join(dir, "arena.exe");
  writeFileSync(proof, "not a real build");
  writeFileSync(arena, "not a real build");
  const io = collect();
  const code = main(
    ["--proof", proof, "--arena", arena],
    { ...io, spawn: twoRunsStatus(GOOD, "arena: presenter error (surface lost)\n", 1) },
  );
  assert.equal(code, 1);
  assert.ok(
    io.lines.some((line) => line.startsWith("SHIP-GAME FAIL [GAME_NO_RECEIPT]")),
    io.lines.join("\n"),
  );
  assert.equal(noAdapter("arena: presenter error (surface lost)"), false);
  assert.equal(noAdapter(NO_ADAPTER_BOOT), true);
});

test("runArena names a no-adapter boot by reason", () => {
  const result = runArena("C:\\ship\\arena.exe", {
    ticks: 600,
    spawn: () => ({ status: 1, stdout: NO_ADAPTER_BOOT, stderr: "" }),
  });
  assert.equal(result.status, 1);
  assert.equal(result.ok, false);
  assert.equal(result.reason, "GAME_NO_RECEIPT");
  assert.match(result.detail, /the headless route still asked for a wgpu adapter/);
  assert.equal(checkSmoke(NO_ADAPTER_BOOT).reason, "GAME_NO_RECEIPT");
});

test("the child environment keeps the system paths and drops the dev tools", () => {
  const env = childEnv({
    SystemRoot: "C:\\Windows",
    TEMP: "C:\\Temp",
    PATH: "C:\\cargo\\bin;C:\\Windows\\System32",
    CARGO_HOME: "C:\\cargo",
    OPENAI_API_KEY: "secret",
  });
  assert.equal(env.CARGO_HOME, undefined);
  assert.equal(env.OPENAI_API_KEY, undefined);
  assert.equal(env.TEMP, "C:\\Temp");
  assert.equal(env.PATH, "C:\\Windows\\System32;C:\\Windows");
});

test("the workflow runs the tool and the node tests", () => {
  const workflow = readFileSync(join(ROOT, ".github", "workflows", "ship-check.yml"), "utf8")
    .replace(/\r\n/g, "\n");
  // The job builds the proof binary into the clean folder, runs it there and
  // fails when the tool refuses the receipt.
  assert.match(workflow, /node tools\/ship-run\.mjs --proof target\/ship-check\/ship-a\/engine2040\.exe/);
  assert.match(workflow, /--arena target\/ship-check\/ship-a\/arena\.exe/);
  assert.match(workflow, /if \(\$LASTEXITCODE -ne 0\)/);
  // The negative proof runs in the job too.
  assert.match(workflow, /node --test tools\/ship-check\.test\.mjs tools\/ship-run\.test\.mjs/);
});
