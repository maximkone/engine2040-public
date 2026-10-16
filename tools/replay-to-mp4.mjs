// replay-to-mp4.mjs - GS25 replay to video (tape -> PNGs -> mp4).
// Runs the arena capture binary to numbered PNGs, then ffmpeg at 60 fps.
// Uses h264_nvenc when ffmpeg lists it, else libx264 veryfast crf 18.
// No ffmpeg exits 2 with the install hint. Pure argv builders stay
// exported so the test can pin the exact argv with stub runners.
import { existsSync, mkdtempSync, readdirSync, readFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { execSync, spawnSync } from "node:child_process";

export const FFMPEG_INSTALL_HINT =
  "replay-to-mp4: ffmpeg not found; install it (https://ffmpeg.org/download.html) and retry";
export const FRAMERATE = 60;
export const FRAME_PATTERN = "frame_%06d.png";
export const USAGE =
  "usage: node tools/replay-to-mp4.mjs <tape> --out clip.mp4 [--max-ticks N] [--pipe]\n" +
  "  <tape>        input tape file driving the human slot\n" +
  "  --out <path>  output mp4 file (required)\n" +
  "  --max-ticks N cap captured ticks (maps to arena --frames N)\n" +
  "  --pipe        stream PNG bytes into ffmpeg image2pipe stdin\n" +
  "  -h, --help    print this help and exit 0\n" +
  "capture: cargo run -p engine-game --bin arena -- --tape <tape>\n" +
  "  --capture-dir <tmp> --capture-every 1 --sha <git sha> --fps 60\n" +
  "encode: ffmpeg -framerate 60 -i frame_%06d.png (-c:v h264_nvenc |\n" +
  "  -c:v libx264 -preset veryfast -crf 18) -pix_fmt yuv420p <out>";

export class UsageError extends Error {}

/** Parse CLI args. Throws UsageError on bad input. */
export function parseArgs(raw) {
  let tape = null;
  let out = null;
  let maxTicks = null;
  let pipe = false;
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
  return { tape, out, maxTicks, pipe };
}

/** Pick the encoder from `ffmpeg -hide_banner -encoders` text. */
export function pickEncoder(encodersText) {
  if (/(^|\s)h264_nvenc(\s|$)/m.test(encodersText)) return "h264_nvenc";
  return "libx264";
}

/** Exact capture argv. Real entry is the arena binary (arena_capture.rs is its helper). */
export function buildCaptureArgv({ tape, dir, maxTicks, sha, fps = FRAMERATE }) {
  const args = [
    "run", "-p", "engine-game", "--bin", "arena", "--",
    "--tape", tape,
    "--capture-dir", dir,
    "--capture-every", "1",
    "--sha", sha,
    "--fps", String(fps),
  ];
  if (maxTicks !== null && maxTicks !== undefined) {
    args.push("--frames", String(maxTicks));
  }
  return { cmd: "cargo", args };
}

/** Exact ffmpeg argv for one encoder case. */
/* PORT(jannik4/bevy_capture@0.6.0:features mp4_ffmpeg_cli_pipe/mp4_ffmpeg_cli: pipe vs file encode choice; pattern only) */
export function buildFfmpegArgv({ pattern, out, encoder, pipe = false }) {
  if (pipe) {
    const head = ["-f", "image2pipe", "-framerate", String(FRAMERATE), "-i", "-"];
    if (encoder === "h264_nvenc") {
      return ["ffmpeg", [...head, "-c:v", "h264_nvenc", "-pix_fmt", "yuv420p", out]];
    }
    return ["ffmpeg", [...head, "-c:v", "libx264", "-preset", "veryfast", "-crf", "18", "-pix_fmt", "yuv420p", out]];
  }
  const head = ["-framerate", String(FRAMERATE), "-i", pattern];
  if (encoder === "h264_nvenc") {
    return ["ffmpeg", [...head, "-c:v", "h264_nvenc", "-pix_fmt", "yuv420p", out]];
  }
  return ["ffmpeg", [...head, "-c:v", "libx264", "-preset", "veryfast", "-crf", "18", "-pix_fmt", "yuv420p", out]];
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
    : spawnSync(cmd, args, { stdio: "inherit" })),
  mkdtemp: (prefix) => mkdtempSync(join(tmpdir(), prefix)),
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
  const encoder = pickEncoder(encoders);
  const dir = deps.mkdtemp("replay-mp4-");
  const pattern = join(dir, FRAME_PATTERN);
  const capture = buildCaptureArgv({ tape: parsed.tape, dir, maxTicks: parsed.maxTicks, sha });
  const captureRun = deps.spawnSync(capture.cmd, capture.args);
  if (captureRun.status !== 0) {
    deps.stderr("replay-to-mp4: capture failed (status " + captureRun.status + ")\n");
    return captureRun.status || 1;
  }
  if (parsed.pipe) {
    const [, pipeArgs] = buildFfmpegArgv({ out: parsed.out, encoder, pipe: true });
    let names;
    try {
      const rd = deps.readdirSync ?? readdirSync;
      names = rd(dir).filter((n) => /^frame_\d+\.png$/.test(n)).sort();
    } catch (err) {
      deps.stderr("replay-to-mp4: pipe read failed (" + (err.message || err) + ")\n");
      return 1;
    }
    if (names.length === 0) {
      deps.stderr("replay-to-mp4: pipe read failed (no frames in " + dir + ")\n");
      return 1;
    }
    let input;
    try {
      const rf = deps.readFileSync ?? readFileSync;
      input = Buffer.concat(names.map((n) => Buffer.from(rf(join(dir, n)))));
    } catch (err) {
      deps.stderr("replay-to-mp4: pipe read failed (" + (err.message || err) + ")\n");
      return 1;
    }
    const pipeRun = deps.spawnSync("ffmpeg", pipeArgs, { input });
    if (pipeRun.status !== 0) {
      deps.stderr("replay-to-mp4: ffmpeg failed (status " + pipeRun.status + ")\n");
      return pipeRun.status || 1;
    }
    deps.stdout("replay-to-mp4: wrote " + parsed.out + " (" + encoder + " pipe)\n");
    return 0;
  }
  const [, ffmpegArgs] = buildFfmpegArgv({ pattern, out: parsed.out, encoder });
  const ffmpegRun = deps.spawnSync("ffmpeg", ffmpegArgs);
  if (ffmpegRun.status !== 0) {
    deps.stderr("replay-to-mp4: ffmpeg failed (status " + ffmpegRun.status + ")\n");
    return ffmpegRun.status || 1;
  }
  deps.stdout("replay-to-mp4: wrote " + parsed.out + " (" + encoder + ")\n");
  return 0;
}

if (import.meta.main) {
  process.exitCode = runMain(process.argv.slice(2));
}
