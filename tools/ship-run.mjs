#!/usr/bin/env node
// ENG19: the ship check must run the shipped build, not only build it. This
// starts the proof binary from the clean folder with a sanitized environment,
// reads the run receipt and refuses a receipt that is missing, carries no run
// line, reports a zero tick or a hash that is not the golden. It also plays
// the shipped arena binary with `--headless`, so the job proves the game runs
// on a machine with no window and no wgpu adapter, then repeats the run and
// refuses two different final hashes.
//
//   node tools/ship-run.mjs --proof <engine2040.exe> [--arena <arena.exe>]
//                            [--log <path>] [--ticks N] [--golden <hex>]
//
// The proof receipt is the only gate that carries the legacy golden: its
// `ticks=<n> final_hash=<16 hex>` line must carry the golden hash and a
// positive tick. The game's sim is the arena fixture and never the legacy
// golden (docs/HASH-LAW.md: the golden never runs arena), so its receipt is
// gated on the tick count, the final hash being present, and two fresh runs
// printing the same hash. A missing receipt, a short run, a missing hash or a
// hash that moved between runs fails the job.
//
// A hosted runner may expose no wgpu adapter: the headless route then presents
// no offscreen frame and names it in the receipt (`frames=0`, presenter
// skipped). The sim still plays, so that case is read as `adapter=none`, not
// skipped.
//
// When the game leg names a real adapter, the shipped binary also draws the
// native reference tier offscreen into a temp folder. One shot is scored
// against content/reference/native/arena600-t000.png with the existing
// iqa-gate scorer, and the tool prints one `SHIP-FRAME` receipt. The bar is
// the look gate's own: SSIM at the scorer's threshold or FLIP under
// `look_flip_max` in tools/product-bar.json. A software adapter is skipped
// unless E2040_ALLOW_SOFTWARE=1, and then it is labelled and never reads as a
// hardware pass.
//
// Exit codes: 0 pass - 1 the build did not prove itself - 2 usage.
import { spawnSync } from "node:child_process";
import {
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  statSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { DEFAULT_THRESHOLD, decodePng, scorePair } from "./iqa-gate.mjs";

// The golden hash of the root proof fixture: seed 2040, 600 ticks, no inputs.
export const GOLDEN_HASH = "d57b5c8ae3096614";

// The shipped game leg plays ten seconds of game time (600 fixed ticks at
// 60 Hz), the same tick set the arena600 reference captures are shot over.
// The game's own fixture hash is quoted, never compared with the legacy
// golden; two fresh runs must print the same hash.
export const ARENA_TICKS = 600;
export const ARENA_FRAMES_PER_SECOND = 60;
export const ARENA_DEADLINE_MS = 120_000;
export const ARENA_TIMEOUT_MS = 150_000;
export const PROOF_TIMEOUT_MS = 120_000;

// The frame leg scores the native reference tier the arena's
// `--reference-set` draws: 1280x720, ten shots on the seed-2040 arena600
// orbit. Only the first shot is scored; the rest are the tier's own shape.
export const FRAME_TIER = "native";
export const FRAME_SHOT = "arena600-t000";
export const FRAME_SHOTS = 10;
export const FRAME_RESOLUTION = "1280x720";
export const FRAME_REFERENCE_PNG = "content/reference/native/arena600-t000.png";
export const FRAME_TIMEOUT_MS = 180_000;

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");

// One named refusal. `detail` says what was read and what was wanted, so the
// red line is enough to fix the run without opening the log.
function refuse(reason, detail) {
  return { ok: false, reason, detail };
}

// Check the proof receipt text. The first line names the receipt; the second
// line carries the tick count and the final hash. A run that verified itself
// also says `verified=true` on the third line.
export function checkReceipt(
  text,
  { golden = GOLDEN_HASH, minTicks = 1, requireVerified = true } = {},
) {
  const source = String(text ?? "");
  if (!/^engine2040 headless proof:/m.test(source)) {
    return refuse("MISSING_RECEIPT", "no 'engine2040 headless proof:' line in the run output");
  }
  const run = source.match(/^\s*ticks=(\d+)\s+final_hash=([0-9a-fA-F]{16})\s*$/m);
  if (!run) {
    return refuse("NO_RUN_LINE", "the receipt has no 'ticks=<n> final_hash=<16 hex>' line");
  }
  const ticks = Number(run[1]);
  const hash = run[2].toLowerCase();
  if (!(ticks >= minTicks)) {
    return refuse("ZERO_TICK", `the receipt reports ticks=${ticks}; want at least ${minTicks}`);
  }
  if (hash !== golden) {
    return refuse("HASH_NOT_GOLDEN", `final_hash=${hash}; want the golden ${golden}`);
  }
  const seed = source.match(/^\s*seed=(\d+)\b[^\n]*\bverified=(true|false)\b/m);
  const verified = seed ? seed[2] === "true" : null;
  if (requireVerified && verified !== true) {
    return refuse("NOT_VERIFIED", "the receipt does not say verified=true (run the proof with --verify)");
  }
  return {
    ok: true,
    reason: "PASS",
    detail: `ticks=${ticks} hash=${hash}`,
    ticks,
    hash,
    seed: seed ? Number(seed[1]) : null,
    verified,
  };
}

// The arena's headless receipt: `arena headless smoke:` (or `arena smoke:`
// from a window run). Both carry the tick count, the presented frame count,
// the presenter notice that names the adapter, and the final checkpoint hash
// (`hash=-` when no tick ran). `headless=forced` names the `--headless` route.
export function parseSmoke(text) {
  const line = String(text ?? "")
    .split(/\r?\n/)
    .find((row) => /^arena (headless )?smoke:/.test(row));
  if (!line) return null;
  const ticks = Number((line.match(/(?:^|\s)ticks=(\d+)/) ?? [])[1]);
  const frames = Number((line.match(/(?:^|\s)frames=(\d+)/) ?? [])[1]);
  if (!Number.isInteger(ticks) || !Number.isInteger(frames)) return null;
  const presenter = (line.match(/presenter="([^"]*)"/) ?? [])[1] ?? "";
  const hash = (line.match(/(?:^|\s)hash=([0-9a-fA-F]{16})\b/) ?? [])[1];
  const headless = (line.match(/(?:^|\s)headless=([a-z-]+)/) ?? [])[1] ?? null;
  return {
    kind: /^arena headless smoke:/.test(line) ? "headless" : "window",
    ticks,
    frames,
    hash: hash ? hash.toLowerCase() : null,
    headless,
    presenter,
    adapter: adapterName(presenter),
  };
}

// The adapter as the presenter notice prints it:
//   presenter twin: NVIDIA GeForce RTX 3050 (537.13)      headless
//   presenter surface: NVIDIA GeForce RTX 3050 (537.13)   window
//   presenter skipped: no wgpu adapter on this machine    no adapter
export function adapterName(presenter) {
  const text = String(presenter ?? "");
  const named = text.match(/presenter (?:twin|surface): (.+?) \(/);
  if (named) return named[1].trim();
  if (/presenter skipped/i.test(text)) return "none";
  return "unknown";
}

// A boot that never reaches the arena smoke line because a presenter asked for
// a wgpu adapter. The `--headless` route must never need one; when this text
// shows up beside a missing receipt, the detail says so and the run stays red.
export function noAdapter(text) {
  return /no wgpu adapter available/i.test(String(text ?? ""));
}

// The label the job prints for the frame leg. A software rasterizer is not a
// real GPU adapter, so it reads as `warp` and the frame score stays skipped.
export function adapterLabel(name) {
  const text = String(name ?? "").toLowerCase();
  if (!text || text === "none" || text === "unknown") return "none";
  if (/warp|microsoft basic render|software|llvmpipe/.test(text)) return "warp";
  return name;
}

// Check the arena game receipt: a smoke line with a positive tick count, the
// requested tick count, and a 16-hex final hash. `expectTicks` pins the fixture
// the run asked for; `requireHash` is what the shipped check passes, so a
// receipt without a sim hash can never pass as a game run.
export function checkSmoke(text, { minTicks = 1, expectTicks = null, requireHash = true } = {}) {
  const smoke = parseSmoke(text);
  if (!smoke) {
    return refuse("GAME_NO_RECEIPT", "no 'arena smoke:' or 'arena headless smoke:' line in the run output");
  }
  if (!(smoke.ticks >= minTicks)) {
    return refuse("GAME_ZERO_TICK", `the game run reports ticks=${smoke.ticks}`);
  }
  if (expectTicks !== null && smoke.ticks !== expectTicks) {
    return refuse(
      "GAME_SHORT_RUN",
      `the game run reports ticks=${smoke.ticks}; the fixture asks for ${expectTicks}`,
    );
  }
  if (requireHash && !smoke.hash) {
    return refuse("GAME_NO_HASH", "the game receipt has no 'hash=<16 hex>' field");
  }
  return {
    ok: true,
    reason: "PASS",
    detail: `ticks=${smoke.ticks} frames=${smoke.frames} hash=${smoke.hash ?? "-"}`,
    smoke,
  };
}

// The child sees only what a deployed build needs: the system paths and the
// temp folders. Dev tools, cargo and model keys stay out, so a run that needs
// the checkout's environment cannot pass as a clean-folder run.
const CHILD_ENV_KEEP = [
  "SystemRoot",
  "SystemDrive",
  "windir",
  "OS",
  "PATHEXT",
  "COMSPEC",
  "TEMP",
  "TMP",
  "USERPROFILE",
  "LOCALAPPDATA",
  "HOMEDRIVE",
  "HOMEPATH",
  "NUMBER_OF_PROCESSORS",
  "PROCESSOR_ARCHITECTURE",
];

export function childEnv(source = process.env) {
  const env = {};
  for (const key of CHILD_ENV_KEEP) {
    if (source[key] !== undefined) env[key] = source[key];
  }
  const root = source.SystemRoot ?? "C:\\Windows";
  env.PATH = `${root}\\System32;${root}`;
  return env;
}

// One child process with its output as text. The spawn is injected so a test
// can pin the parsing without building or starting a binary.
export function runProcess(command, args, options = {}) {
  const {
    cwd,
    env = childEnv(),
    timeoutMs = PROOF_TIMEOUT_MS,
    spawn = spawnSync,
  } = options;
  const result = spawn(command, args, {
    cwd,
    env,
    encoding: "utf8",
    timeout: timeoutMs,
    maxBuffer: 32 * 1024 * 1024,
    windowsHide: true,
  });
  const out = `${result.stdout ?? ""}${result.stderr ?? ""}`;
  return { status: result.status, error: result.error ?? null, out };
}

// Run the proof binary from its own folder and check its receipt. `args`
// exists for the tests: the CLI always runs the binary with `--verify`.
export function runProof(exe, options = {}) {
  const path = resolve(exe);
  const {
    args = ["--verify"],
    spawn = spawnSync,
    env = childEnv(),
    timeoutMs = PROOF_TIMEOUT_MS,
    golden = GOLDEN_HASH,
    minTicks = 1,
  } = options;
  const run = runProcess(path, args, { cwd: dirname(path), env, timeoutMs, spawn });
  if (run.error) {
    return {
      ...refuse("PROOF_FAILED", `cannot start the proof binary: ${run.error.message}`),
      out: run.out,
      status: run.status,
    };
  }
  const check = checkReceipt(run.out, { golden, minTicks });
  if (run.status !== 0) {
    return {
      ...refuse(check.ok ? "PROOF_EXIT" : check.reason, `the proof binary exited ${run.status}; ${check.detail}`),
      out: run.out,
      status: run.status,
    };
  }
  return { ...check, out: run.out, status: run.status };
}

// One `--headless` run of the shipped arena binary. The clean folder is the
// child's working folder, so the run cannot rely on the checkout's current
// directory; the route never opens a window, so no display or GPU is needed.
function runArenaOnce(path, { ticks, spawn, env, deadlineMs, timeoutMs }) {
  const args = [
    "--headless",
    "--frames",
    String(ticks),
    "--exit",
    "--mute",
    "--deadline-ms",
    String(deadlineMs),
  ];
  const run = runProcess(path, args, { cwd: dirname(path), env, timeoutMs, spawn });
  if (run.error) {
    return {
      ...refuse("GAME_FAILED", `cannot start the arena binary: ${run.error.message}`),
      out: run.out,
      status: run.status,
    };
  }
  const check = checkSmoke(run.out, { expectTicks: ticks, requireHash: true });
  if (run.status !== 0) {
    const asked = noAdapter(run.out) ? "; the headless route still asked for a wgpu adapter" : "";
    return {
      ...refuse(
        check.ok ? "GAME_EXIT" : check.reason,
        `the arena binary exited ${run.status}${asked}; ${check.detail}`,
      ),
      out: run.out,
      status: run.status,
    };
  }
  return { ...check, out: run.out, status: run.status };
}

// Play the shipped game with no window and no GPU, twice from fresh state, and
// compare the final hashes. The second run is the game's own verify: the same
// fixed fixture must end on the same checkpoint hash, so a run that stopped
// early, skipped the sim or drifted turns the job red. The arena fixture never
// carries the legacy golden (docs/HASH-LAW.md: the golden never runs arena).
export function runArena(exe, options = {}) {
  const path = resolve(exe);
  const {
    ticks = ARENA_TICKS,
    runs = 2,
    spawn = spawnSync,
    env = childEnv(),
    deadlineMs = ARENA_DEADLINE_MS,
    timeoutMs = ARENA_TIMEOUT_MS,
  } = options;
  let first = null;
  for (let index = 0; index < runs; index++) {
    const one = runArenaOnce(path, { ticks, spawn, env, deadlineMs, timeoutMs });
    if (!one.ok) return one;
    if (first === null) {
      first = one;
    } else if (one.smoke.hash !== first.smoke.hash) {
      return {
        ...refuse(
          "GAME_HASH_MOVED",
          `run ${1} ended hash=${first.smoke.hash}; run ${index + 1} ended hash=${one.smoke.hash}`,
        ),
        out: `${first.out}\n${one.out}`,
        status: one.status,
      };
    }
  }
  return { ...first, runs };
}

// The software opt-in is E2040_ALLOW_SOFTWARE=1, the renderer's own switch.
// The child environment is sanitized, so the tool hands the flag over only
// when it means it; a hardware leg strips it.
export function allowSoftware(source = {}) {
  return String(source?.E2040_ALLOW_SOFTWARE ?? "") === "1";
}

export function withSoftware(env, allowed) {
  const out = { ...env };
  if (allowed) out.E2040_ALLOW_SOFTWARE = "1";
  else delete out.E2040_ALLOW_SOFTWARE;
  return out;
}

// The reference run's summary line:
//   arena reference: tier=native shots=10 camera=arena600 seed=2040
//     size=1280x720 adapter="NVIDIA ..." driver="..." fragment="..."
// One `arena reference: ... shot=arena600-t000 ... png="..." bytes=<n>`
// line per shot names the file the mode wrote. The scorer reads the PNG
// itself; these lines are the receipt, never the score.
export function parseReference(text) {
  const lines = String(text ?? "").split(/\r?\n/);
  const summary = lines.find((line) => /^arena reference: tier=\S+ shots=\d+/.test(line));
  if (!summary) return null;
  const shots = Number((summary.match(/(?:^|\s)shots=(\d+)/) ?? [])[1]);
  const size = summary.match(/(?:^|\s)size=(\d+)x(\d+)/);
  const adapter = (summary.match(/(?:^|\s)adapter="([^"]*)"/) ?? [])[1];
  if (!Number.isInteger(shots) || !size || adapter === undefined) return null;
  return {
    tier: (summary.match(/(?:^|\s)tier=(\S+)/) ?? [])[1] ?? "",
    shots,
    width: Number(size[1]),
    height: Number(size[2]),
    adapter: adapter.trim(),
    driver: ((summary.match(/(?:^|\s)driver="([^"]*)"/) ?? [])[1] ?? "").trim(),
    summary,
  };
}

// One shot line of the same run, for the shot the score reads.
export function parseReferenceShot(text, shot = FRAME_SHOT) {
  const escaped = String(shot).replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const line = String(text ?? "")
    .split(/\r?\n/)
    .find((row) => new RegExp(`^arena reference: tier=\\S+ shot=${escaped}(?:\\s|$)`).test(row));
  if (!line) return null;
  const bytes = (line.match(/(?:^|\s)bytes=(\d+)/) ?? [])[1];
  const png = (line.match(/(?:^|\s)png="([^"]*)"/) ?? [])[1];
  if (!bytes || png === undefined) return null;
  return { png, bytes: Number(bytes) };
}

// The look gate's own bar: iqa-gate's SSIM threshold and product-bar's FLIP
// floor. Neither number is invented here, and a missing floor is loud.
export function lookBar(root = ROOT) {
  const path = join(root, "tools", "product-bar.json");
  const bar = JSON.parse(readFileSync(path, "utf8"));
  const flipMax = bar?.floors?.look_flip_max;
  if (!Number.isFinite(flipMax)) throw new Error(`${path} has no floors.look_flip_max`);
  return { ssimMin: DEFAULT_THRESHOLD, flipMax };
}

export function frameVerdict(score, flip, bar = lookBar()) {
  return Number(score) >= bar.ssimMin || Number(flip) <= bar.flipMax ? "pass" : "fail";
}

function parseFrameSize(size) {
  const match = String(size ?? "").match(/^(\d+)x(\d+)$/);
  if (!match) throw new Error(`bad frame size ${size}; want <w>x<h>`);
  return { width: Number(match[1]), height: Number(match[2]) };
}

// The one SHIP-FRAME receipt. A hardware capture names the adapter that drew
// the pixels; a software capture says so and never carries `verdict=pass`.
export function frameLine(frame) {
  const resolution = frame.resolution ?? FRAME_RESOLUTION;
  if (frame.software) {
    return `SHIP-FRAME adapter=warp reason=software-adapter resolution=${resolution} score=${frame.score} flip=${frame.flip} verdict=software`;
  }
  return `SHIP-FRAME adapter="${frame.adapter}" resolution=${resolution} score=${frame.score} flip=${frame.flip} verdict=${frame.verdict}`;
}

// Score one committed reference shot against a fresh shipped capture. The
// capture draws the native tier offscreen into `captureDir`; the score reads
// that PNG and the reference PNG with the same decoder iqa-gate uses. A short
// run, a missing capture, a size mismatch or an unreadable pair is refused by
// name; a low score is the caller's `verdict=fail`.
export function runFrameLeg(exe, options = {}) {
  const path = resolve(exe);
  const {
    spawn = spawnSync,
    env = childEnv(),
    software = false,
    captureDir = mkdtempSync(join(tmpdir(), "ship-frame-")),
    referencePng = join(ROOT, FRAME_REFERENCE_PNG),
    size = FRAME_RESOLUTION,
    timeoutMs = FRAME_TIMEOUT_MS,
    bar = null,
  } = options;
  const base = { out: "", status: null, captureDir };
  let want;
  let gate;
  try {
    want = parseFrameSize(size);
    gate = bar ?? lookBar();
  } catch (error) {
    return { ...refuse("FRAME_BAR_UNREADABLE", error.message), ...base };
  }
  const args = ["--reference-set", "--reference-tier", FRAME_TIER, "--reference-dir", captureDir];
  const run = runProcess(path, args, {
    cwd: dirname(path),
    env: withSoftware(env, software),
    timeoutMs,
    spawn,
  });
  base.out = run.out;
  base.status = run.status;
  if (run.error) {
    return {
      ...refuse("FRAME_FAILED", `cannot start the reference capture: ${run.error.message}`),
      ...base,
    };
  }
  if (run.status !== 0) {
    const why = /no wgpu adapter/i.test(run.out)
      ? "the reference capture found no wgpu adapter"
      : `the reference capture exited ${run.status}`;
    return { ...refuse("FRAME_CAPTURE_EXIT", why), ...base };
  }
  const reference = parseReference(run.out);
  if (!reference) {
    return {
      ...refuse("FRAME_NO_RECEIPT", "no 'arena reference: tier=native shots=<n>' line in the capture output"),
      ...base,
    };
  }
  if (reference.tier !== FRAME_TIER) {
    return {
      ...refuse("FRAME_TIER", `the capture drew tier=${reference.tier}; the frame leg scores ${FRAME_TIER}`),
      ...base,
    };
  }
  if (reference.width !== want.width || reference.height !== want.height) {
    return {
      ...refuse(
        "FRAME_SIZE",
        `the capture drew ${reference.width}x${reference.height}; the frame leg wants ${size}`,
      ),
      ...base,
    };
  }
  if (reference.shots !== FRAME_SHOTS) {
    return {
      ...refuse(
        "FRAME_SHORT_RUN",
        `the capture filed ${reference.shots} shots; the ${FRAME_TIER} tier files ${FRAME_SHOTS}`,
      ),
      ...base,
    };
  }
  if (!reference.adapter) {
    return { ...refuse("FRAME_NO_ADAPTER", "the capture line names no adapter"), ...base };
  }
  const shot = parseReferenceShot(run.out);
  if (!shot) {
    return { ...refuse("FRAME_NO_SHOT", `no 'arena reference: ... shot=${FRAME_SHOT}' line`), ...base };
  }
  if (!(shot.bytes > 0)) {
    return { ...refuse("FRAME_EMPTY_SHOT", `shot ${FRAME_SHOT} filed ${shot.bytes} bytes`), ...base };
  }
  const capturePath = join(captureDir, FRAME_TIER, `${FRAME_SHOT}.png`);
  if (!existsSync(capturePath)) {
    return { ...refuse("FRAME_NO_CAPTURE", `the capture did not write ${capturePath}`), ...base };
  }
  if (!existsSync(referencePng)) {
    return { ...refuse("FRAME_NO_REFERENCE", `no committed reference shot at ${referencePng}`), ...base };
  }
  let expected;
  let actual;
  try {
    expected = decodePng(readFileSync(referencePng), referencePng);
    actual = decodePng(readFileSync(capturePath), capturePath);
  } catch (error) {
    return { ...refuse("FRAME_UNREADABLE", `cannot decode the pair: ${error.message}`), ...base };
  }
  if (expected.w !== actual.w || expected.h !== actual.h) {
    return {
      ...refuse(
        "FRAME_SIZE_MISMATCH",
        `reference is ${expected.w}x${expected.h}; capture is ${actual.w}x${actual.h}`,
      ),
      ...base,
    };
  }
  if (expected.w !== want.width || expected.h !== want.height) {
    return {
      ...refuse(
        "FRAME_SIZE",
        `the scored pair is ${expected.w}x${expected.h}; the frame leg wants ${size}`,
      ),
      ...base,
    };
  }
  let score;
  try {
    score = scorePair(expected.data, actual.data, expected.w, expected.h, gate.ssimMin);
  } catch (error) {
    return { ...refuse("FRAME_UNREADABLE", `cannot score the pair: ${error.message}`), ...base };
  }
  const verdict = frameVerdict(score.ssim_x10000, score.flip_x10000, gate);
  const adapterClass = adapterLabel(reference.adapter);
  return {
    ok: true,
    reason: "PASS",
    detail: `shots=${reference.shots} adapter="${reference.adapter}" score=${score.ssim_x10000} flip=${score.flip_x10000} verdict=${verdict}`,
    adapter: reference.adapter,
    adapterLabel: adapterClass,
    software: adapterClass === "warp" || adapterClass === "none",
    resolution: `${expected.w}x${expected.h}`,
    score: score.ssim_x10000,
    flip: score.flip_x10000,
    verdict,
    bar: gate,
    referencePng,
    capturePath,
    ...base,
  };
}

// The last non-empty lines of one run: a red names the end of the output, not
// the whole boot log.
export function tail(text, count = 20) {
  return String(text ?? "")
    .split(/\r?\n/)
    .filter((line) => line.trim() !== "")
    .slice(-count);
}

export const HELP = `ship-run.mjs — run the shipped build and check its receipt

Usage:
  node tools/ship-run.mjs --proof <engine2040.exe> [--arena <arena.exe>]
                           [--log <path>] [--ticks N] [--golden <hex>]

The proof binary runs with --verify from its own folder, under an environment
that holds only the system paths and the temp folders. Its receipt must say
\`ticks=<n> final_hash=d57b5c8ae3096614\` with n > 0, and it must have verified
itself; the tool then prints \`SHIP-RUN tick=<n> hash=<h>\`.

--arena adds the shipped game: the arena binary plays N fixed ticks (default
600, ten seconds at 60 Hz) with \`--headless\`, no window and no GPU adapter,
and must print \`arena headless smoke: ticks=N frames=<f> hash=<16 hex>
headless=forced\`. The run repeats from fresh state and both final hashes must
be equal, so the game's own fixture is verified without pretending it is the
legacy golden (docs/HASH-LAW.md: the golden never runs arena). A missing
receipt, a short run, a missing hash or a hash that moved turns the job red. On
a machine with no adapter the run presents no frame (\`frames=0\`) and the tool
names \`adapter=none\`.

With a real adapter the tool scores the packed frame: the binary draws the ten
native reference shots (1280x720) offscreen into a temp folder, one shot is
compared with content/reference/native/arena600-t000.png, and the tool prints
\`SHIP-FRAME adapter="<name>" resolution=1280x720 score=<ssim_x10000>
flip=<flip_x10000> verdict=pass|fail\`. The bar is the look gate's own: SSIM at
tools/iqa-gate.mjs's threshold or FLIP under product-bar.json's
\`look_flip_max\`. A software adapter is skipped; set E2040_ALLOW_SOFTWARE=1 to
score it as \`adapter=warp reason=software-adapter ... verdict=software\`, which
never reads as a hardware pass.

Named refusals print as \`SHIP-RUN FAIL [REASON] <detail>\`.
Exit codes: 0 pass - 1 the build did not prove itself - 2 usage.
`;

function parseArgs(argv) {
  const out = {
    help: false,
    proof: null,
    arena: null,
    log: null,
    ticks: ARENA_TICKS,
    golden: GOLDEN_HASH,
  };
  for (let i = 0; i < argv.length; i++) {
    const arg = argv[i];
    const value = (flag) => {
      const next = argv[i + 1];
      if (next === undefined || next.startsWith("--")) throw new Error(`missing value for ${flag}`);
      i++;
      return next;
    };
    switch (arg) {
      case "--help":
      case "-h":
        out.help = true;
        break;
      case "--proof":
        out.proof = value(arg);
        break;
      case "--arena":
        out.arena = value(arg);
        break;
      case "--log":
        out.log = value(arg);
        break;
      case "--ticks": {
        const ticks = Number(value(arg));
        if (!Number.isInteger(ticks) || ticks <= 0) throw new Error("--ticks needs a positive integer");
        out.ticks = ticks;
        break;
      }
      case "--golden": {
        const golden = value(arg).toLowerCase();
        if (!/^[0-9a-f]{16}$/.test(golden)) throw new Error("--golden needs 16 hex digits");
        out.golden = golden;
        break;
      }
      default:
        throw new Error(`unknown arg ${arg}`);
    }
  }
  return out;
}

// The log is the artifact: the SHIP lines first, then the raw output of each
// run, so a reader can check the quoted line against the receipt.
function writeLog(path, lines, sections) {
  const body = [...lines];
  for (const [name, text] of sections) {
    body.push(`--- ${name} output begin ---`);
    body.push(String(text ?? "").trimEnd() || "(no output)");
    body.push(`--- ${name} output end ---`);
  }
  mkdirSync(dirname(resolve(path)), { recursive: true });
  writeFileSync(path, `${body.join("\n")}\n`, "utf8");
}

export function main(
  argv,
  {
    out = console.log,
    err = console.error,
    spawn = spawnSync,
    env = withSoftware(childEnv(process.env), allowSoftware(process.env)),
    frame: frameOptions = {},
  } = {},
) {
  let args;
  try {
    args = parseArgs(argv);
  } catch (error) {
    err(`ship-run: ${error.message}\n${HELP}`);
    return 2;
  }
  if (args.help) {
    out(HELP.trimEnd());
    return 0;
  }
  if (!args.proof) {
    err(`ship-run: need --proof <engine2040.exe>\n${HELP}`);
    return 2;
  }
  const proofPath = resolve(args.proof);
  if (!existsSync(proofPath) || !statSync(proofPath).isFile()) {
    err(`SHIP-RUN FAIL [MISSING_PROOF] no such file: ${proofPath}`);
    return 1;
  }

  const lines = [];
  const say = (line) => {
    lines.push(line);
    out(line);
  };
  const sections = [];
  let failed = false;
  // The sanitized environment carries the renderer's software opt-in only
  // when the caller set it; the frame leg uses the same env as the game leg.
  const software = allowSoftware(env);
  const legEnv = withSoftware(env, software);

  const proof = runProof(proofPath, { spawn, env: legEnv, golden: args.golden });
  sections.push(["proof", proof.out]);
  if (proof.ok) {
    say(`SHIP-RUN tick=${proof.ticks} hash=${proof.hash}`);
  } else {
    failed = true;
    say(`SHIP-RUN FAIL [${proof.reason}] ${proof.detail}`);
    say(`SHIP-RUN FAIL: the proof output ends:`);
    for (const line of tail(proof.out)) say(line);
  }

  if (args.arena) {
    const arenaPath = resolve(args.arena);
    if (!existsSync(arenaPath) || !statSync(arenaPath).isFile()) {
      failed = true;
      say(`SHIP-RUN FAIL [MISSING_ARENA] no such file: ${arenaPath}`);
    } else {
      const arena = runArena(arenaPath, { spawn, env: legEnv, ticks: args.ticks });
      sections.push(["arena", arena.out]);
      if (arena.ok) {
        const smoke = arena.smoke;
        const label = adapterLabel(smoke.adapter);
        const seconds = (smoke.ticks / ARENA_FRAMES_PER_SECOND).toFixed(3);
        say(
          `SHIP-GAME tick=${smoke.ticks} hash=${smoke.hash} frames=${smoke.frames} seconds=${seconds} adapter=${label} headless=${smoke.headless ?? "unknown"} runs=${arena.runs}`,
        );
        say(`SHIP-GAME presenter="${smoke.presenter}"`);
        // A no-adapter run presents nothing, so the frame leg is named, not
        // scored. A real adapter draws the native reference tier and the
        // scorer reads one committed shot. A software adapter runs only when
        // E2040_ALLOW_SOFTWARE=1 asks for it, and then it is labelled.
        if (label === "none" || (label === "warp" && !software)) {
          say(`SHIP-FRAME skipped adapter=${label} reason=no-hardware-adapter`);
        } else {
          const frame = runFrameLeg(arenaPath, {
            ...frameOptions,
            spawn,
            env: legEnv,
            software: label === "warp",
          });
          sections.push(["frame", frame.out ?? ""]);
          if (frame.ok) {
            say(frameLine(frame));
            if (!frame.software && frame.verdict !== "pass") failed = true;
          } else {
            failed = true;
            say(`SHIP-FRAME FAIL [${frame.reason}] ${frame.detail}`);
            for (const line of tail(frame.out)) say(line);
          }
        }
      } else {
        failed = true;
        say(`SHIP-GAME FAIL [${arena.reason}] ${arena.detail}`);
        say("SHIP-GAME FAIL: the arena output ends:");
        for (const line of tail(arena.out)) say(line);
      }
    }
  }

  const verdict = failed
    ? "SHIP-RUN FAIL: the shipped build did not prove itself"
    : "SHIP-RUN PASS: the proof receipt is the golden and the shipped game played headless twice to one hash";
  lines.push(verdict);

  if (args.log) {
    try {
      writeLog(args.log, lines, sections);
      out(`SHIP-RUN log: ${resolve(args.log)}`);
    } catch (error) {
      err(`SHIP-RUN FAIL [LOG_UNWRITABLE] cannot write ${args.log}: ${error.message}`);
      return 1;
    }
  }

  if (failed) {
    err(verdict);
    return 1;
  }
  out(verdict);
  return 0;
}

if (import.meta.main) {
  try {
    process.exitCode = main(process.argv.slice(2));
  } catch (error) {
    console.error(`ship-run: ${error.message}`);
    process.exitCode = 2;
  }
}
