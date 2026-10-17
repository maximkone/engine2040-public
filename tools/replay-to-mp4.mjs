// replay-to-mp4.mjs - GS25 replay to video (tape -> PNGs -> mp4).
// Runs the arena capture binary to numbered PNGs, then ffmpeg at 60 fps.
// Uses h264_nvenc when ffmpeg lists it, else libx264 veryfast crf 18.
// No ffmpeg exits 2 with the install hint. Pure argv builders and the frame
// plan stay exported so the test can pin them with stub runners.
// Exit: 0 wrote the clip, 2 bad flags / missing tape / no ffmpeg, else the
// status of the capture or encode step that failed.
import { existsSync, mkdtempSync, readdirSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { execSync, spawnSync } from "node:child_process";

const REPO_ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");

export const FFMPEG_INSTALL_HINT =
  "replay-to-mp4: E_FFMPEG_MISSING ffmpeg not found; install it (https://ffmpeg.org/download.html) and retry";
export const FRAMERATE = 60;
export const FRAME_PATTERN = "frame_%06d.png";
// --pipe holds every frame in memory; past this cap the file mode is the way.
export const PIPE_MAX_BYTES = 512 * 1024 * 1024;
export const USAGE =
  "usage: node tools/replay-to-mp4.mjs <tape> --out clip.mp4 [--max-ticks N] [--pipe] [--keep-frames]\n" +
  "  <tape>        input tape file driving the human slot\n" +
  "  --out <path>  output mp4 file (required; overwritten if it exists)\n" +
  "  --max-ticks N cap captured ticks (maps to arena --frames N); default is the tape length\n" +
  "  --pipe        stream PNG bytes into ffmpeg image2pipe stdin (small clips only)\n" +
  "  --keep-frames keep the numbered PNG directory after a good encode\n" +
  "  -h, --help    print this help and exit 0\n" +
  "capture: cargo run -p engine-game --features app --bin arena -- --tape <tape>\n" +
  "  --capture-dir <tmp> --capture-every 1 --sha <git sha> --fps 60 --frames N --exit\n" +
  "encode: ffmpeg -y -framerate 60 -i frame_%06d.png (-c:v h264_nvenc |\n" +
  "  -c:v libx264 -preset veryfast -crf 18) -pix_fmt yuv420p <out>\n" +
  "  h264_nvenc is used when listed and retried as libx264 if it fails to start";

export class UsageError extends Error {}

const TAPE_MAGIC = "E2040CAP";
const TAPE_HEADER_BYTES = 14;
const TAPE_INPUT_BYTES = 10;

/**
 * Ticks a capture tape spans (last input tick + 1), from its header and
 * records (crates/game/src/capture.rs: magic, u16 version, u32 count, then
 * 10-byte inputs that start with a u32 tick). Throws UsageError otherwise.
 */
export function readTapeTicks(bytes) {
  if (bytes.length < TAPE_HEADER_BYTES || bytes.subarray(0, 8).toString("latin1") !== TAPE_MAGIC) {
    throw new UsageError("not a capture tape (bad magic); pass --max-ticks N to skip reading it");
  }
  const version = bytes.readUInt16LE(8);
  if (version !== 1) {
    throw new UsageError("unsupported tape version " + version + "; pass --max-ticks N to skip reading it");
  }
  const count = bytes.readUInt32LE(10);
  if (bytes.length !== TAPE_HEADER_BYTES + count * TAPE_INPUT_BYTES) {
    throw new UsageError("tape length does not match its input count");
  }
  let last = -1;
  for (let i = 0; i < count; i += 1) {
    last = Math.max(last, bytes.readUInt32LE(TAPE_HEADER_BYTES + i * TAPE_INPUT_BYTES));
  }
  if (last < 0) throw new UsageError("tape has no inputs; pass --max-ticks N");
  return last + 1;
}

/**
 * Frame list from a directory listing: the numbered PNGs the capture wrote.
 * ffmpeg stops at the first missing number without a word, so a gap or an
 * empty list is an error here, never a short clip.
 */
export function planFrames(names) {
  const nums = [];
  for (const n of names) {
    const m = /^frame_(\d{6})\.png$/.exec(n);
    if (m) nums.push(Number(m[1]));
  }
  nums.sort((a, b) => a - b);
  if (nums.length === 0) throw new UsageError("capture wrote no frames");
  for (let i = 1; i < nums.length; i += 1) {
    if (nums[i] !== nums[i - 1] + 1) {
      throw new UsageError("frame gap after frame_" + String(nums[i - 1]).padStart(6, "0") + ".png");
    }
  }
  return { count: nums.length, first: nums[0], last: nums[nums.length - 1] };
}

/** Parse CLI args. Throws UsageError on bad input. */
export function parseArgs(raw) {
  let tape = null;
  let out = null;
  let maxTicks = null;
  let pipe = false;
  let keepFrames = false;
  for (let i = 0; i < raw.length; i += 1) {
    const a = raw[i];
    if (a === "-h" || a === "--help") return { help: true };
    if (a === "--out") {
      i += 1;
      const v = raw[i];
      if (v === undefined || v.startsWith("--") || v === "") {
        throw new UsageError("--out needs a value");
      }
      if (out !== null) throw new UsageError("duplicate --out");
      out = v;
    } else if (a === "--max-ticks") {
      i += 1;
      const v = raw[i];
      const n = typeof v === "string" ? Number(v) : NaN;
      if (!Number.isInteger(n) || n < 1 || n > 1000000) {
        throw new UsageError("--max-ticks needs an integer 1..1000000");
      }
      if (maxTicks !== null) throw new UsageError("duplicate --max-ticks");
      maxTicks = n;
    } else if (a === "--pipe") {
      if (pipe) throw new UsageError("duplicate --pipe");
      pipe = true;
    } else if (a === "--keep-frames") {
      if (keepFrames) throw new UsageError("duplicate --keep-frames");
      keepFrames = true;
    } else if (a.startsWith("--")) {
      throw new UsageError("unknown flag: " + a);
    } else if (tape === null) {
      tape = a;
    } else {
      throw new UsageError("too many positional args: " + a);
    }
  }
  if (tape === null) throw new UsageError("missing <tape>");
  if (out === null) throw new UsageError("missing --out <path>");
  return { tape, out, maxTicks, pipe, keepFrames };
}

/** Pick the encoder from `ffmpeg -hide_banner -encoders` text. */
export function pickEncoder(encodersText) {
  if (/(^|\s)h264_nvenc(\s|$)/m.test(encodersText)) return "h264_nvenc";
  return "libx264";
}

/**
 * Exact capture argv. The `arena` bin only builds with the `app` feature, and
 * it keeps running after the last frame unless `--frames N --exit` is given,
 * so both are always passed. `frames` is the presented-frame count to capture.
 */
export function buildCaptureArgv({ tape, dir, frames, sha, fps = FRAMERATE }) {
  if (!Number.isInteger(frames) || frames < 1) {
    throw new UsageError("capture needs a frame count of 1 or more");
  }
  const args = [
    "run", "-p", "engine-game", "--features", "app", "--bin", "arena", "--",
    "--tape", tape,
    "--capture-dir", dir,
    "--capture-every", "1",
    "--sha", sha,
    "--fps", String(fps),
    "--frames", String(frames),
    "--exit",
  ];
  return { cmd: "cargo", args };
}

/** Exact ffmpeg argv for one encoder case. `-y` so a rerun never waits on an overwrite prompt. */
/* PORT(jannik4/bevy_capture@0.6.0:features mp4_ffmpeg_cli_pipe/mp4_ffmpeg_cli: pipe vs file encode choice; pattern only) */
export function buildFfmpegArgv({ pattern, out, encoder, pipe = false, startNumber = 0 }) {
  const head = pipe
    ? ["-f", "image2pipe", "-framerate", String(FRAMERATE), "-i", "-"]
    : [
        "-framerate", String(FRAMERATE),
        ...(startNumber > 0 ? ["-start_number", String(startNumber)] : []),
        "-i", pattern,
      ];
  const codec = encoder === "h264_nvenc"
    ? ["-c:v", "h264_nvenc"]
    : ["-c:v", "libx264", "-preset", "veryfast", "-crf", "18"];
  return ["ffmpeg", ["-y", ...head, ...codec, "-pix_fmt", "yuv420p", out]];
}

/**
 * Encoder attempts in order, pure: nvenc first when listed, then libx264,
 * because a listed nvenc can still fail to start on a box with no usable
 * GPU or for a frame size under its minimum.
 */
export function encoderAttempts(encodersText) {
  return pickEncoder(encodersText) === "h264_nvenc" ? ["h264_nvenc", "libx264"] : ["libx264"];
}

/** All frame bytes in order for ffmpeg's stdin; refuses past PIPE_MAX_BYTES. */
export function readFramesForPipe(dir, names, readFile, cap = PIPE_MAX_BYTES) {
  const frames = names.filter((n) => /^frame_\d{6}\.png$/.test(n)).sort();
  const parts = [];
  let total = 0;
  for (const n of frames) {
    const bytes = Buffer.from(readFile(join(dir, n)));
    total += bytes.length;
    if (total > cap) {
      throw new UsageError("--pipe holds every frame in memory and this clip is past " + Math.round(cap / 1048576) + " MiB; drop --pipe");
    }
    parts.push(bytes);
  }
  return Buffer.concat(parts);
}

/** Build SHA hex for the manifest: E2040_SHA wins, else git HEAD. */
export function resolveSha(deps) {
  const env = (deps.env && deps.env.E2040_SHA ? String(deps.env.E2040_SHA) : "").trim();
  if (/^[0-9a-fA-F]{7,128}$/.test(env)) return env.toLowerCase();
  try {
    const sha = String(deps.execSync("git rev-parse HEAD").toString()).trim();
    if (/^[0-9a-fA-F]{7,128}$/.test(sha)) return sha.toLowerCase();
  } catch {
    // Fall through to the loud error below.
  }
  throw new UsageError("need a build SHA: set E2040_SHA to 7-128 hex chars");
}

const realDeps = {
  env: process.env,
  execSync: (cmd) => execSync(cmd, { stdio: ["ignore", "pipe", "pipe"] }),
  spawnSync: (cmd, args, opts) => (opts && opts.input
    ? spawnSync(cmd, args, { input: opts.input, stdio: ["pipe", "inherit", "inherit"] })
    : spawnSync(cmd, args, { stdio: "inherit", cwd: opts && opts.cwd })),
  mkdtemp: (prefix) => mkdtempSync(join(tmpdir(), prefix)),
  rm: (p) => rmSync(p, { recursive: true, force: true }),
  existsSync: (p) => existsSync(p),
  readdirSync: (d) => readdirSync(d),
  readFileSync: (p) => readFileSync(p),
  stdout: (s) => process.stdout.write(s),
  stderr: (s) => process.stderr.write(s),
};

/** Run the tool. Returns the exit code. Runners are injectable for tests. */
export function runMain(rawArgs, deps = realDeps) {
  let parsed;
  try {
    parsed = parseArgs(rawArgs);
  } catch (err) {
    deps.stderr(USAGE + "\n" + (err.message || err) + "\n");
    return 2;
  }
  if (parsed.help) {
    deps.stdout(USAGE + "\n");
    return 0;
  }
  const tapeExists = deps.existsSync ? deps.existsSync(parsed.tape) : true;
  if (!tapeExists) {
    deps.stderr("replay-to-mp4: tape not found: " + parsed.tape + "\n");
    return 2;
  }
  let encoders;
  try {
    encoders = deps.execSync("ffmpeg -hide_banner -encoders").toString();
  } catch {
    deps.stderr(FFMPEG_INSTALL_HINT + "\n");
    return 2;
  }
  let sha;
  try {
    sha = resolveSha(deps);
  } catch (err) {
    deps.stderr(USAGE + "\n" + (err.message || err) + "\n");
    return 2;
  }
  let frames = parsed.maxTicks;
  if (frames === null) {
    try {
      frames = readTapeTicks(Buffer.from((deps.readFileSync ?? readFileSync)(parsed.tape)));
    } catch (err) {
      deps.stderr("replay-to-mp4: " + (err.message || err) + "\n");
      return 2;
    }
  }
  const dir = deps.mkdtemp("replay-mp4-");
  const pattern = join(dir, FRAME_PATTERN);
  // cargo runs from the repo root; the tape path stays valid from any cwd.
  const capture = buildCaptureArgv({ tape: resolve(parsed.tape), dir, frames, sha });
  const captureRun = deps.spawnSync(capture.cmd, capture.args, { cwd: REPO_ROOT });
  if (captureRun.status !== 0) {
    const why = captureRun.error ? " (" + (captureRun.error.code || captureRun.error.message) + ")" : "";
    deps.stderr("replay-to-mp4: capture failed (status " + captureRun.status + ")" + why + "\n");
    return captureRun.status || 1;
  }
  let plan;
  try {
    plan = planFrames((deps.readdirSync ?? readdirSync)(dir));
  } catch (err) {
    deps.stderr("replay-to-mp4: " + (err.message || err) + " in " + dir + "\n");
    return 1;
  }
  let input;
  if (parsed.pipe) {
    try {
      input = readFramesForPipe(dir, (deps.readdirSync ?? readdirSync)(dir), deps.readFileSync ?? readFileSync, deps.pipeMaxBytes ?? PIPE_MAX_BYTES);
    } catch (err) {
      deps.stderr("replay-to-mp4: pipe read failed (" + (err.message || err) + ")\n");
      return 1;
    }
  }
  const attempts = encoderAttempts(encoders);
  for (let i = 0; i < attempts.length; i += 1) {
    const encoder = attempts[i];
    const [, ffmpegArgs] = buildFfmpegArgv({ pattern, out: parsed.out, encoder, pipe: parsed.pipe, startNumber: plan.first });
    const run = parsed.pipe ? deps.spawnSync("ffmpeg", ffmpegArgs, { input }) : deps.spawnSync("ffmpeg", ffmpegArgs);
    if (run.status === 0) {
      deps.stdout("replay-to-mp4: wrote " + parsed.out + " (" + encoder + (parsed.pipe ? " pipe" : "") + ", " + plan.count + " frames)\n");
      if (parsed.keepFrames) deps.stdout("replay-to-mp4: frames kept in " + dir + "\n");
      else if (deps.rm) deps.rm(dir);
      return 0;
    }
    if (i + 1 < attempts.length) {
      deps.stderr("replay-to-mp4: " + encoder + " failed (status " + run.status + "); retrying with " + attempts[i + 1] + "\n");
    } else {
      deps.stderr("replay-to-mp4: ffmpeg failed (status " + run.status + "); frames left in " + dir + "\n");
      return run.status || 1;
    }
  }
  return 1;
}

if (import.meta.main) {
  process.exitCode = runMain(process.argv.slice(2));
}
