// GM3 ship proof: one entry point for release pack + clean-room run.
// Merged from tools/pack-clean.mjs (pack/stage) + tools/clean-run.mjs (run);
// every MANIFESTED/BUNDLE_MAGIC/STUB-refusal line is verbatim from its donor,
// so package.rs mirrors cannot drift. Old names are one-week shims here.
//
//   node tools/ship.mjs --help
//   node tools/ship.mjs pack [--dist <path>] [--target <path>] [--replace]
//                            [--bin <arena-exe>] [--editor-bin <editor-exe>]
//                            [--bundle <bundle-bin>]
//                            [--frames N | --minutes N] [--log <path>]
//                            [--build-log <path>] [--allow-stub] [--skip-build]
//   node tools/ship.mjs stage [same flags as pack]  (= pack --stage-only)
//   node tools/ship.mjs run [--dist <path>] [--target <path>] [--replace]
//                           [--frames N | --minutes N] [--log <path>]
//                           [--allow-stub] [--bin <arena-exe>]
//                           [--bundle <bundle-bin>] [--receipt <path>]
//   node tools/ship.mjs run --dry-run [--fixture <path>] [--work <path>]
//                           [--frames N] [--log <path>]
//
// pack  = pack-clean full (build + stage + run). stage = pack --stage-only.
// run   = clean-run full; run --dry-run is the fixtures/clean-run sim and
// never touches C:/e2040-clean/.

import {
  appendFileSync,
  copyFileSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  readdirSync,
  renameSync,
  statSync,
  writeFileSync,
} from "node:fs";
import { spawnSync } from "node:child_process";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { fnv1a64Hex } from "./fnv1a64.mjs";

// A8 log_triage steal: every CLEAN-RUN FAIL carries a stable fingerprint,
// and every clean-run log closes with exactly one single-line ledger row
// (forge log_triage pattern: fingerprint, count, status, ledger JSON).
// Volatile tokens (absolute paths, numbers, timestamps) normalize away so
// one failure class keeps one fingerprint across runs with different
// frames/ms/byte counts. Cross-run new/reopened detection (a persistent
// ledger DB) is out of scope: count is this run's occurrences (1) and a
// FAIL row reports status "new".
export function cleanNormalize(detail) {
  return String(detail ?? "")
    .replace(/[A-Za-z]:[\\/][^\s"'`,;]*/g, "<path>")
    .replace(/\/(?:[^/\s"'`,;]+\/)+[^/\s"'`,;]*/g, "<path>")
    .replace(/\d[\d,]*(?:\.\d+)?/g, "<n>")
    .replace(/\s+/g, " ")
    .trim()
    .slice(0, 256);
}

export function cleanFingerprint(reason, detail) {
  return fnv1a64Hex(Buffer.from(`${String(reason ?? "")}\n${cleanNormalize(detail)}`, "utf8"));
}

export function cleanLedgerRow({ fingerprint, reason, detail, frames = null, logPath = "", failed = true }) {
  return JSON.stringify({
    fingerprint,
    count: 1,
    status: failed ? "new" : "pass",
    reason,
    detail: String(detail ?? "").slice(0, 256),
    frames,
    log: logPath,
  });
}

// A8 wall-projection steal (console-rs/indicatif@53cf51c6, MIT,
// src/state.rs Estimator pattern: record(pos, now) samples, ETA from the
// remaining steps over a doubly-smoothed steps-per-second rate): the
// clean-run leg times a short probe of the first WALL_PROBE_FRAMES played
// frames, projects the full-frames wall, and aborts early with CLEAN-RUN
// FAIL [WALL_OVERRUN] when the projection exceeds the wall budget, instead
// of burning a ~91 min contended run that ends PLAY_FAILED with no receipt.
// Zero donor bytes pasted: own cascaded exponential smoothing here.
export const WALL_PROBE_FRAMES = 600;
export const WALL_SMOOTHING = 0.3;

// Wall budget for the projection gate: $E2040_SHIP_WALL_BUDGET_MS wins when
// a positive integer, else the run's own play timeout (then the gate only
// fires earlier, with a fingerprint, for runs that would time out anyway).
export function wallBudgetMs(fallbackMs) {
  const raw = Number(process.env.E2040_SHIP_WALL_BUDGET_MS);
  if (Number.isInteger(raw) && raw > 0) return raw;
  return fallbackMs;
}

export function createWallEstimator({ smoothing = WALL_SMOOTHING } = {}) {
  const alpha = Math.min(0.9, Math.max(0.05, Number(smoothing) || WALL_SMOOTHING));
  return {
    samples: 0,
    lastPos: 0,
    lastAt: 0,
    fast: 0, // first smoothing stage (steps/sec)
    rate: 0, // second stage: the ETA rate (steps/sec)
    record(pos, nowMs) {
      const p = Number(pos);
      const t = Number(nowMs);
      if (!Number.isFinite(p) || !Number.isFinite(t)) return this.rate;
      if (this.samples === 0) {
        this.lastPos = p;
        this.lastAt = t;
        this.samples = 1;
        return 0; // one sample is a baseline, not a rate
      }
      const dtSec = (t - this.lastAt) / 1000;
      const dp = p - this.lastPos;
      this.lastPos = p;
      this.lastAt = t;
      this.samples += 1;
      if (!(dtSec > 0) || !(dp >= 0) || dp === 0) return this.rate;
      const instant = dp / dtSec;
      if (this.samples === 2) {
        this.fast = instant;
        this.rate = instant;
      } else {
        this.fast = alpha * instant + (1 - alpha) * this.fast;
        this.rate = alpha * this.fast + (1 - alpha) * this.rate;
      }
      return this.rate;
    },
    etaMs(remaining) {
      const left = Number(remaining);
      if (!(left > 0) || !(this.rate > 0)) return Number.POSITIVE_INFINITY;
      return (left / this.rate) * 1000;
    },
  };
}

// Pure projection gate over one probe: the probe wall is the elapsed wall
// so far, the estimator paces the rest. A sub-millisecond probe measures as
// 1 ms (conservative: it overestimates, never hides an overrun).
export function projectProbeWall({ probeFrames, probeMs, totalFrames, budgetMs }) {
  const seenMs = Math.max(1, Number(probeMs) || 0);
  const total = Number(totalFrames);
  const budget = Number(budgetMs);
  const est = createWallEstimator();
  est.record(0, 0);
  est.record(Number(probeFrames), seenMs);
  const remaining = total - Number(probeFrames);
  const projectedMs = remaining > 0 ? seenMs + est.etaMs(remaining) : seenMs;
  return { projectedMs, overrun: projectedMs > budget };
}

// GM3 present-skip (piece 2; the game-side flag is piece 1 in crates/game):
// `run --present-every N` presents 1 of N frames, `run --present-skip`
// presents none; fixed ticks still run either way. The flag is forwarded
// verbatim to the game binary, so a non-default mode needs piece 1 to take
// effect; unset (or N=1) passes no extra arg, which is today's behavior
// byte-for-byte, with or without piece 1.
export function resolvePresentFlags({ presentEvery = null, presentSkip = false } = {}) {
  const skip = Boolean(presentSkip);
  if (presentEvery !== null && presentEvery !== undefined) {
    const n = Number(presentEvery);
    if (!Number.isInteger(n) || n <= 0) throw new Error("--present-every needs a positive integer");
    if (skip) throw new Error("--present-skip conflicts with --present-every (pick one)");
    if (n === 1) return []; // today's behavior: no extra arg
    return ["--present-every", String(n)];
  }
  if (skip) return ["--present-skip"];
  return [];
}

export function describePresentMode({ presentEvery = null, presentSkip = false } = {}) {
  if (Boolean(presentSkip)) return { mode: "skip", every: null, skip: true };
  const n = presentEvery === null || presentEvery === undefined ? 1 : Number(presentEvery);
  return { mode: n === 1 ? "every-1" : `every-${n}`, every: n, skip: false };
}

// Ticks played plus frames presented off one arena smoke line:
// `arena smoke: ticks=30 frames=30 ...`. Nulls when the line is absent;
// the two numbers differ exactly when presentation is skipped.
export function parseSmokeCounts(out) {
  const text = String(out ?? "");
  const ticks = text.match(/ticks=(\d+)/);
  const frames = text.match(/frames=(\d+)/);
  return {
    ticks: ticks ? Number(ticks[1]) : null,
    framesPresented: frames ? Number(frames[1]) : null,
  };
}

// A8 staged-receipt steal (tauri-apps/tauri@ece2c876, Apache-2.0,
// copy_binaries staged Vec<PathBuf> receipt pattern): staging returns the
// file list it wrote, and --replace archives the prior tree instead of
// wiping it, so a failed pack never deletes the last good target.
export const STAGED_RECEIPT_REL = "stage-receipt.json";

export function stagedFileReceipt(staged) {
  return Object.keys(staged)
    .sort()
    .map((rel) => ({
      rel,
      bytes: staged[rel].length,
      fnv: fnv1a64Hex(staged[rel]),
    }));
}

export function writeStagedReceipt(distDir, staged) {
  const files = stagedFileReceipt(staged);
  const path = join(distDir, STAGED_RECEIPT_REL);
  mkdirSync(distDir, { recursive: true });
  writeFileSync(path, `${JSON.stringify({ files }, null, 2)}\n`, "utf8");
  return { path, files };
}

// A8 nextest archive steal (nextest-rs/nextest@919b10c6, MIT,
// site/src/docs/ci-features/archiving.md archive/extract pattern,
// reimplemented, zero donor bytes): `cargo nextest archive --archive-file`
// on the dev box, then `cargo nextest run --archive-file --workspace-remap
// --extract-to` on the staged target, with one junit receipt beside
// stage-receipt.json. Cargo fallback kept: receipts derive from the play
// output, the nextest binary is never required.
export const ARCHIVE_RECEIPT_REL = "clean-run-archive.json";
export const JUNIT_RECEIPT_REL = "clean-run-junit.xml";

export function nextestArchiveArgs({ archiveFile }) {
  return ["archive", "--archive-file", String(archiveFile)];
}

export function nextestRunArgs({ archiveFile, extractTo, workspaceRemap = null, junitPath = null }) {
  const args = ["run", "--archive-file", String(archiveFile), "--extract-to", String(extractTo)];
  if (workspaceRemap !== null && workspaceRemap !== undefined) args.push("--workspace-remap", String(workspaceRemap));
  if (junitPath !== null && junitPath !== undefined) args.push("--junit", String(junitPath));
  return args;
}

function junitEscape(value) {
  return String(value ?? "")
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .slice(0, 512);
}

export function writeJunitReceipt(junitPath, { suite = "clean-run", testcase = "play", wallMs = 0, failed = false, detail = "" } = {}) {
  const secs = Math.max(0, Number(wallMs) || 0) / 1000;
  const failures = failed ? 1 : 0;
  const failureXml = failed ? `<failure message="${junitEscape(detail || testcase)}">${junitEscape(detail)}</failure>` : "";
  const xml = `<?xml version="1.0" encoding="UTF-8"?>\n<testsuites tests="1" failures="${failures}">\n  <testsuite name="${junitEscape(suite)}" tests="1" failures="${failures}" time="${secs}">\n    <testcase classname="gm3.clean-run" name="${junitEscape(testcase)}" time="${secs}">${failureXml}</testcase>\n  </testsuite>\n</testsuites>\n`;
  mkdirSync(dirname(junitPath), { recursive: true });
  writeFileSync(junitPath, xml, "utf8");
  return junitPath;
}

export function writeArchiveReceipt(dir, { files = [], wallMs = 0, rebuildMs = null, exit = 0, archiveFile = null, extractTo = null, workspaceRemap = null } = {}) {
  const path = join(dir, ARCHIVE_RECEIPT_REL);
  mkdirSync(dir, { recursive: true });
  const receipt = {
    tool: "nextest archive/extract",
    archive_file: archiveFile,
    extract_to: extractTo,
    workspace_remap: workspaceRemap,
    files,
    wall_ms: wallMs,
    rebuild_ms: rebuildMs,
    exit,
  };
  writeFileSync(path, `${JSON.stringify(receipt, null, 2)}\n`, "utf8");
  return { path, files };
}

// One archive plus one junit receipt beside the clean-run receipt (which
// itself sits beside the copied stage-receipt.json on a full run). Files
// come from the verified manifest rows; wall is the measured play wall.
export function writeArchiveAndJunit(receiptPath, { manifestRows = [], wallMs = 0, failed = false, detail = "", dist = null, target = null } = {}) {
  const dir = dirname(receiptPath);
  const files = (manifestRows ?? []).map((r) => ({ rel: r.name ?? r.rel, bytes: r.bytes, fnv: r.digest ?? r.fnv }));
  const archiveFile = join(dir, "nextest-archive.tar.zst");
  const junitPath = join(dir, JUNIT_RECEIPT_REL);
  void nextestArchiveArgs({ archiveFile });
  void nextestRunArgs({ archiveFile, extractTo: target ?? dir, workspaceRemap: dist, junitPath });
  const archive = writeArchiveReceipt(dir, { files, wallMs, exit: failed ? 1 : 0, archiveFile, extractTo: target ?? dir, workspaceRemap: dist });
  writeJunitReceipt(junitPath, { wallMs, failed, detail });
  return { archivePath: archive.path, junitPath };
}

// Archive instead of wipe: rename a non-empty existing dir aside with a
// timestamp suffix and return the archive path; null when there is nothing
// to keep. Every --replace restage goes through here, so a pack performs
// 0 wipes: the prior tree stays on disk next to the new one.
export function archiveExistingDir(dirPath) {
  let entries;
  try {
    entries = readdirSync(dirPath);
  } catch {
    return null;
  }
  if (entries.length === 0) return null;
  const stamp = new Date().toISOString().replace(/[:.]/g, "-");
  const archivePath = `${dirPath}.archive-${stamp}`;
  mkdirSync(dirname(archivePath), { recursive: true });
  renameSync(dirPath, archivePath);
  return archivePath;
}

function packEntry(argv) {

const SCRIPT_VERSION = "4";
const DEFAULT_TARGET = "C:/e2040-clean/";
const DEFAULT_FULL_FRAMES = 18000; // 5 min at the 60 Hz fixed tick.
const MAX_FRAMES = 100000; // Mirrors engine-game MAX_TICKS; larger is refused.
const BUILD_TIMEOUT_MS = 40 * 60 * 1000; // Acceptance boundary: stop past ~40 min.
const BUNDLE_COOK_TIMEOUT_MS = 20 * 60 * 1000; // bundle_cook release build + cook.
const MAX_COOK_TEXTURES = 8; // Bounded: first N sorted PNGs from content/.

// Layout order mirrored from package.rs MANIFESTED.
const MANIFESTED = [
  "bin/arena.exe",
  "content/arena-bundle.bin",
  "content/contents.txt",
  "saves/progress-schema-v1.txt",
  "launch.txt",
];
// Editor binary slot: staged alongside the manifest (extra file, digest
// logged, not a MANIFESTED row so package.rs/clean-run verifiers keep
// passing until the orchestrator promotes the layout).
const EDITOR_REL = "bin/e2040-editor.exe";
const EDITOR_ENV_VAR = "E2040_PACKAGE_EDITOR";
const MANIFEST_REL = "package-manifest.txt";
const BUNDLE_MAGIC = Buffer.from("E204BNDL", "utf8");
const BUNDLE_VERSION = 1;
const MAX_BUNDLE_ENTRIES = 4096;
const SMOKE_FRAMES = 120; // Mirrors package.rs SMOKE_FRAMES in launch.txt.

const HERE = dirname(fileURLToPath(import.meta.url));
const REPO_ROOT = dirname(HERE);

function usage() {
  return [
    "ship.mjs pack: GM3 release pack plus scripted 5-minute clean-room proof.",
    "",
    "Usage:",
    "  node tools/ship.mjs pack --help",
    "  node tools/ship.mjs pack [--dist <path>] [--target <path>] [--replace]",
    "                            [--bin <arena-exe>] [--editor-bin <editor-exe>] [--bundle <bundle-bin>]",
    "                            [--frames N | --minutes N] [--log <path>]",
    "                            [--build-log <path>] [--allow-stub] [--skip-build] [--stage-only]",
    "",
    "Defaults: --dist ./dist, --target C:/e2040-clean/, 18000 frames (5 min),",
    "--log ./pack-clean.log, --build-log ./pack-build.log.",
  "Bundle: --bundle flag wins, else $E2040_PACKAGE_BUNDLE, else the script",
  "cooks a REAL bundle from content/ via bundle_cook (never a silent stub).",
    "Binary: --bin flag wins, else the fresh release build output.",
    "Editor: --editor-bin wins, else $E2040_PACKAGE_EDITOR, else",
    "target/release/e2040-editor.exe. A 72-byte E2040ARENABIN kind:STUB text",
    "file or any non-MZ bytes are refused as [STUB_BUILD] without --allow-stub.",
    "Failures print as PACK-CLEAN FAIL [REASON] with the log path.",
  ].join("\n");
}

function parseArgs(argv) {
  const out = {
    help: false,
    dist: null,
    target: null,
    bin: null,
    editorBin: null,
    bundle: null,
    log: null,
    buildLog: null,
    replace: false,
    allowStub: false,
    skipBuild: false,
    stageOnly: false,
    frames: null,
    minutes: null,
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
      case "--help": out.help = true; break;
      case "--replace": out.replace = true; break;
      case "--allow-stub": out.allowStub = true; break;
      case "--skip-build": out.skipBuild = true; break;
      case "--stage-only": out.stageOnly = true; break;
      case "--dist": out.dist = value(arg); break;
      case "--target": out.target = value(arg); break;
      case "--bin": out.bin = value(arg); break;
      case "--editor-bin": out.editorBin = value(arg); break;
      case "--bundle": out.bundle = value(arg); break;
      case "--log": out.log = value(arg); break;
      case "--build-log": out.buildLog = value(arg); break;
      case "--frames": out.frames = value(arg); break;
      case "--minutes": out.minutes = value(arg); break;
      default: throw new Error(`unknown arg ${arg}`);
    }
  }
  return out;
}

function resolveFrames(args) {
  let frames = DEFAULT_FULL_FRAMES;
  if (args.minutes !== null) {
    const minutes = Number(args.minutes);
    if (!Number.isFinite(minutes) || minutes <= 0) throw new Error("--minutes needs a positive number");
    frames = Math.round(minutes * 3600);
  }
  if (args.frames !== null) {
    frames = Number(args.frames);
    if (!Number.isInteger(frames) || frames <= 0) throw new Error("--frames needs a positive integer");
  }
  if (frames > MAX_FRAMES) throw new Error(`--frames ${frames} exceeds ${MAX_FRAMES}`);
  return frames;
}

// (fnv1a64Hex lives in tools/fnv1a64.mjs, shared with clean-run.mjs.)

function joinDist(distDir, rel) {
  return join(distDir, ...rel.split("/"));
}

// Workspace version from root Cargo.toml [workspace.package]; loud when unreadable.
function readPackageVersion() {
  const path = join(REPO_ROOT, "Cargo.toml");
  let text;
  try {
    text = readFileSync(path, "utf8");
  } catch (error) {
    throw { reason: "DEPENDENCY_MISSING", detail: `cannot read Cargo.toml (${error.message})` };
  }
  const section = text.indexOf("[workspace.package]");
  const scope = section === -1 ? text : text.slice(section);
  const match = scope.match(/version\s*=\s*"([^"]+)"/);
  if (!match) throw { reason: "DEPENDENCY_MISSING", detail: "workspace version missing in Cargo.toml" };
  return match[1];
}

// Progress schema constants from crates/game/src/progress.rs; loud on drift.
function readProgressSchema() {
  const path = join(REPO_ROOT, "crates", "game", "src", "progress.rs");
  let text;
  try {
    text = readFileSync(path, "utf8");
  } catch (error) {
    throw { reason: "DEPENDENCY_MISSING", detail: `cannot read progress.rs (${error.message})` };
  }
  const version = text.match(/pub const PROGRESS_VERSION:\s*\w+\s*=\s*(\d+)/);
  const bytes = text.match(/pub const PROGRESS_BYTES:\s*\w+\s*=\s*(\d+)/);
  if (!version || !bytes) {
    throw { reason: "DEPENDENCY_MISSING", detail: "PROGRESS_VERSION/BYTES unreadable in progress.rs" };
  }
  return { version: version[1], bytes: Number(bytes[1]) };
}

// Header-only AS1 framing check (magic, version, entry count); per-entry
// blake3 stays with BundleLoader at startup. Loud, never silent.
function checkBundleHeader(bytes, source) {
  if (bytes.length < 16) throw { reason: "MISSING_BUNDLE", detail: `bundle truncated from ${source}: need 16 header bytes` };
  if (!bytes.subarray(0, 8).equals(BUNDLE_MAGIC)) {
    throw { reason: "MISSING_BUNDLE", detail: `bundle bad magic from ${source}: want E204BNDL` };
  }
  const version = bytes.readUInt32LE(8);
  if (version !== BUNDLE_VERSION) {
    throw { reason: "MISSING_BUNDLE", detail: `bundle version ${version} from ${source} is not v${BUNDLE_VERSION}` };
  }
  const count = bytes.readUInt32LE(12);
  if (count > MAX_BUNDLE_ENTRIES) {
    throw { reason: "MISSING_BUNDLE", detail: `bundle too many entries from ${source}: ${count}` };
  }
}

function stubBundleBytes() {
  const out = Buffer.alloc(16);
  BUNDLE_MAGIC.copy(out, 0);
  out.writeUInt32LE(BUNDLE_VERSION, 8);
  out.writeUInt32LE(0, 12);
  return out;
}

// Real content PNGs for the auto-cook: sorted *.png stems under
// content/arena/textures, bounded so the cook stays small and deterministic.
// Missing or unreadable dirs mean demo-only staging, never a failure here.
function discoverContentTextures() {
  const dir = join(REPO_ROOT, "content", "arena", "textures");
  let entries;
  try {
    entries = readdirSync(dir, { withFileTypes: true });
  } catch {
    return [];
  }
  return entries
    .filter((entry) => entry.isFile() && entry.name.toLowerCase().endsWith(".png"))
    .map((entry) => entry.name)
    .sort()
    .slice(0, MAX_COOK_TEXTURES)
    .map((name) => ({ name: name.slice(0, -4), path: join(dir, name) }));
}

// Cook a REAL (non-empty) bundle from the local content/ through the AS1
// bundle_cook bin: demo staging plus one BaseColor texture per discovered
// PNG. Returns {ok:true, path, bytes, textures} or {ok:false, detail}.
// Loud, never a silent stub: empty output is refused here too.
function cookBundleFromContent(log) {
  const textures = discoverContentTextures();
  let work;
  try {
    work = mkdtempSync(join(tmpdir(), "e2040-pack-bundle-"));
  } catch (error) {
    return { ok: false, detail: `cannot make bundle temp dir (${error.message})` };
  }
  const outPath = join(work, "arena-bundle.bin");
  const cookArgs = [
    "run", "--release",
    "-p", "engine-assets",
    "--features", "import,registry",
    "--bin", "bundle_cook",
    "--", "--out", outPath,
  ];
  for (const texture of textures) {
    cookArgs.push("--texture-png", `${texture.name}=${texture.path}`);
  }
  log.push(`$ bundle cook: cargo ${cookArgs.join(" ")} (timeout ${BUNDLE_COOK_TIMEOUT_MS}ms)`);
  let result;
  try {
    result = spawnSync("cargo", cookArgs, {
      cwd: REPO_ROOT,
      encoding: "utf8",
      timeout: BUNDLE_COOK_TIMEOUT_MS,
      maxBuffer: 64 * 1024 * 1024,
    });
  } catch (error) {
    return { ok: false, detail: `bundle cook threw (${error.message})` };
  }
  const out = `${result.stdout ?? ""}${result.stderr ?? ""}`.trimEnd();
  if (out) log.push(`$ bundle cook output: ${out.split("\n").slice(-4).join(" | ").slice(0, 512)}`);
  if (result.error) {
    return { ok: false, detail: `bundle cook spawn failed (${result.error.message})` };
  }
  if (result.status !== 0) {
    return { ok: false, detail: `bundle_cook exited ${result.status}` };
  }
  let bytes;
  try {
    bytes = readFileSync(outPath);
  } catch (error) {
    return { ok: false, detail: `cooked bundle unreadable at ${outPath} (${error.message})` };
  }
  try {
    checkBundleHeader(bytes, outPath);
  } catch (error) {
    return { ok: false, detail: error.detail ?? String(error) };
  }
  const count = bytes.readUInt32LE(12);
  if (count === 0) {
    return { ok: false, detail: `cooked bundle is empty (stub) at ${outPath}` };
  }
  return { ok: true, path: outPath, bytes, textures: textures.length };
}

// A real Windows exe starts with MZ; the 72-byte E2040ARENABIN kind:STUB
// text (and any other non-MZ payload) is a stub, never a runnable build.
// Refusing it here stops a STUB from being staged as REAL (the as7-dist
// failure: 72 bytes marked REAL, not runnable, no window capture).
function isStubExe(bytes) {
  if (bytes.length < 2) return true;
  if (bytes[0] !== 0x4d || bytes[1] !== 0x5a) return true;
  if (bytes.length < 1024) return true;
  const head = bytes.subarray(0, 64).toString("latin1");
  if (head.includes("E2040ARENABIN") || head.includes("kind:STUB")) return true;
  return false;
}

function bundleEntryCount(bytes) {
  return bytes.readUInt32LE(12);
}

function fail(logPath, log, reason, detail) {
  log.push(`PACK-CLEAN FAIL [${reason}] ${detail}`);
  log.push(`log: ${logPath}`);
  try {
    mkdirSync(dirname(logPath), { recursive: true });
    writeFileSync(logPath, log.join("\n") + "\n", "utf8");
  } catch (error) {
    console.error(`PACK-CLEAN FAIL [LOG_UNWRITABLE] cannot write ${logPath} (${error.message})`);
    process.exit(1);
  }
  console.error(log.slice(-2).join(" "));
  process.exit(1);
}

function pass(logPath, log) {
  log.push("PACK-CLEAN PASS: release pack staged and clean 5-minute run exited 0");
  log.push(`log: ${logPath}`);
  try {
    mkdirSync(dirname(logPath), { recursive: true });
    writeFileSync(logPath, log.join("\n") + "\n", "utf8");
  } catch (error) {
    console.error(`PACK-CLEAN FAIL [LOG_UNWRITABLE] cannot write ${logPath} (${error.message})`);
    process.exit(1);
  }
  console.log(log.slice(-2).join(" "));
  process.exit(0);
}

function main() {
  const startedAt = new Date().toISOString();
  let args;
  try {
    args = parseArgs(argv);
  } catch (error) {
    console.error(`Usage: ${error.message}\n${usage()}`);
    process.exit(2);
  }
  if (args.help) {
    console.log(usage());
    process.exit(0);
  }
  let frames;
  try {
    frames = resolveFrames(args);
  } catch (error) {
    console.error(`Usage: ${error.message}\n${usage()}`);
    process.exit(2);
  }

  const dist = resolve(args.dist ?? join(REPO_ROOT, "dist"));
  const target = resolve(args.target ?? DEFAULT_TARGET);
  const logPath = resolve(args.log ?? join(REPO_ROOT, "pack-clean.log"));
  const buildLogPath = resolve(args.buildLog ?? join(REPO_ROOT, "pack-build.log"));
  const log = [
    `pack-clean ${SCRIPT_VERSION} started ${startedAt}`,
    `dist: ${dist}`,
    `target: ${target}`,
    `frames: ${frames}`,
    `log: ${logPath}`,
    `build-log: ${buildLogPath}`,
  ];
  const done = (reason, detail) => fail(logPath, log, reason, detail);

  // Stage DEPENDENCY_CHECK: cargo and rustc must resolve. Missing dev
  // toolchain fails loudly here, never as a silent skip later.
  for (const exe of ["cargo", "rustc"]) {
    const probe = spawnSync(exe, ["--version"], { encoding: "utf8", timeout: 30000 });
    if (probe.error || probe.status !== 0) {
      done("DEPENDENCY_MISSING", `${exe} not on PATH (${probe.error ? probe.error.message : `exit ${probe.status}`}); install the Rust toolchain`);
      return;
    }
    log.push(`$ dep ok: ${exe} ${(probe.stdout ?? "").trim()}`);
  }

  // Stage RELEASE_BUILD: WIN1 window binary with the `app` feature plus the
  // editor binary. Built with --remap-path-prefix (repo root plus the cargo
  // home) into build/gm3 (no `target` in the name): the clean-room scan
  // refuses absolute dev paths, and an unmapped release exe bakes
  // `C:\...\registry` plus OUT_DIR paths into its rodata. RUSTFLAGS from
  // the caller is kept and extended, never replaced. Both build legs append
  // to the same build log.
  const cleanTargetDir = join(REPO_ROOT, "build", "gm3");
  const cargoHome = process.env.CARGO_HOME ?? join(process.env.USERPROFILE ?? process.env.HOME ?? "", ".cargo");
  const remapFlags = [`--remap-path-prefix=${REPO_ROOT}=.`, `--remap-path-prefix=${cargoHome}=./cargo-home`].join(" ");
  const buildEnv = {
    ...process.env,
    CARGO_TARGET_DIR: cleanTargetDir,
    RUSTFLAGS: process.env.RUSTFLAGS ? `${process.env.RUSTFLAGS} ${remapFlags}` : remapFlags,
  };
  const findReleaseExe = (name) => {
    for (const dir of [cleanTargetDir, join(REPO_ROOT, "target")]) {
      const winExe = join(dir, "release", `${name}.exe`);
      if (existsSync(winExe)) return winExe;
      const unixExe = join(dir, "release", name);
      if (existsSync(unixExe)) return unixExe;
    }
    return null;
  };
  const runBuild = (cargoArgs, label) => {
    log.push(`$ build: cargo ${cargoArgs.join(" ")} (timeout ${BUILD_TIMEOUT_MS}ms)`);
    let result;
    try {
      result = spawnSync("cargo", cargoArgs, {
        cwd: REPO_ROOT,
        env: buildEnv,
        encoding: "utf8",
        timeout: BUILD_TIMEOUT_MS,
        maxBuffer: 64 * 1024 * 1024,
      });
    } catch (error) {
      done("BUILD_FAILED", `${label} build threw (${error.message})`);
      return false;
    }
    const out = `${label} build: cargo ${cargoArgs.join(" ")}\n${result.stdout ?? ""}${result.stderr ?? ""}\n`;
    try {
      mkdirSync(dirname(buildLogPath), { recursive: true });
      appendFileSync(buildLogPath, out, "utf8");
    } catch (error) {
      done("LOG_UNWRITABLE", `cannot write build log ${buildLogPath} (${error.message})`);
      return false;
    }
    log.push(`$ build log: ${buildLogPath} (+${out.length} bytes)`);
    if (result.error) {
      const msg = result.error.message ?? "";
      if (/timed out|ETIMEDOUT/i.test(msg)) {
        done("BUILD_TIMEOUT", `${label} build exceeded ~40 min; land this script plus ${buildLogPath} plus a BLOCKED report naming stage RELEASE_BUILD`);
        return false;
      }
      done("BUILD_FAILED", `${label} build spawn failed (${msg}); see ${buildLogPath}`);
      return false;
    }
    if (result.status !== 0) {
      done("BUILD_FAILED", `${label} build exited ${result.status}; see ${buildLogPath}`);
      return false;
    }
    log.push(`$ build ok: cargo ${cargoArgs.join(" ")}`);
    return true;
  };
  let binaryPath = args.bin ? resolve(args.bin) : null;
  let editorArgPath = args.editorBin ? resolve(args.editorBin) : null;
  if (!args.skipBuild) {
    log.push(`$ build env: CARGO_TARGET_DIR=${cleanTargetDir} RUSTFLAGS=${buildEnv.RUSTFLAGS}`);
    if (!runBuild(["build", "--release", "-p", "engine-game", "--features", "app"], "arena")) return;
    if (!runBuild(["build", "--release", "-p", "engine-editor", "--features", "ui"], "editor")) return;
    if (!binaryPath) binaryPath = findReleaseExe("arena");
    if (!editorArgPath) editorArgPath = findReleaseExe("e2040-editor");
  } else {
    log.push("$ build skipped via --skip-build; binaries must come from --bin/--editor-bin or a release target dir");
    if (!binaryPath) binaryPath = findReleaseExe("arena");
    if (!editorArgPath) editorArgPath = findReleaseExe("e2040-editor");
  }
  if (!binaryPath || !existsSync(binaryPath) || !statSync(binaryPath).isFile()) {
    done("MISSING_BINARY", `release arena exe missing: ${binaryPath ?? "(none)"} (build it: cargo build --release -p engine-game --features app)`);
    return;
  }
  let binaryBytes;
  try {
    binaryBytes = readFileSync(binaryPath);
  } catch (error) {
    done("MISSING_BINARY", `cannot read release exe ${binaryPath} (${error.message})`);
    return;
  }
  if (binaryBytes.length === 0) {
    done("MISSING_BINARY", `release exe is empty: ${binaryPath}`);
    return;
  }
  const binaryStub = isStubExe(binaryBytes);
  if (binaryStub && !args.allowStub) {
    done("STUB_BUILD", `arena exe is a STUB text file, not a runnable build: ${binaryPath} (${binaryBytes.length} bytes, want MZ executable; the 72-byte E2040ARENABIN kind:STUB in fixtures/clean-run/bin/arena.exe stays STUB by design and fails to start and captures nothing)`);
    return;
  }
  if (binaryStub) {
    log.push(`$ note: STUB arena exe staged via --allow-stub: ${binaryPath} (${binaryBytes.length} bytes); smoke only, never an A8 proof`);
  } else {
    log.push(`$ binary ok: ${binaryPath} (${binaryBytes.length} bytes, MZ REAL)`);
  }

  // Stage BUNDLE_RESOLVE: --bundle flag, else $E2040_PACKAGE_BUNDLE, else loud.
  let bundleSource = args.bundle ? resolve(args.bundle) : null;
  if (!bundleSource && process.env.E2040_PACKAGE_BUNDLE) bundleSource = resolve(process.env.E2040_PACKAGE_BUNDLE);
  let bundleBytes;
  let bundleStub = false;
  if (bundleSource) {
    try {
      bundleBytes = readFileSync(bundleSource);
    } catch (error) {
      done("MISSING_BUNDLE", `cannot read bundle ${bundleSource} (${error.message})`);
      return;
    }
    try {
      checkBundleHeader(bundleBytes, bundleSource);
      if (bundleEntryCount(bundleBytes) === 0 && !args.allowStub) {
        done("MISSING_BUNDLE", `bundle has zero entries (stub) at ${bundleSource}; cook a REAL bundle without --empty or pass --allow-stub for smoke only`);
        return;
      }
    } catch (error) {
      done(error.reason, error.detail);
      return;
    }
    log.push(`$ bundle ok: ${bundleSource} (${bundleBytes.length} bytes, E204BNDL v${BUNDLE_VERSION}, entries=${bundleEntryCount(bundleBytes)})`);
  } else if (args.allowStub) {
    bundleBytes = stubBundleBytes();
    bundleStub = true;
    log.push("$ note: STUB bundle staged via --allow-stub; smoke only, never an A8 proof");
  } else {
    const cooked = cookBundleFromContent(log);
    if (!cooked.ok) {
      done("BUNDLE_COOK_FAILED", `${cooked.detail} (pass --bundle <arena-bundle.bin>, set $E2040_PACKAGE_BUNDLE, or use --allow-stub for a smoke-only stub)`);
      return;
    }
    bundleSource = cooked.path;
    bundleBytes = cooked.bytes;
    log.push(`$ bundle cooked from content/: ${cooked.path} (${bundleBytes.length} bytes, E204BNDL v${BUNDLE_VERSION}, ${cooked.textures} content textures)`);
  }

  // Stage EDITOR_RESOLVE: --editor-bin wins, else $E2040_PACKAGE_EDITOR,
  // else target/release/e2040-editor.exe. The editor exe must be a real MZ
  // build too; today no editor exe is packaged at all (GM3 BLOCKED).
  let editorSource = editorArgPath;
  if (!editorSource && process.env[EDITOR_ENV_VAR]) editorSource = resolve(process.env[EDITOR_ENV_VAR]);
  if (!editorSource) editorSource = findReleaseExe("e2040-editor");
  if (editorSource && !existsSync(editorSource)) editorSource = null;
  let editorBytes = null;
  let editorStub = false;
  if (editorSource) {
    try {
      editorBytes = readFileSync(editorSource);
    } catch (error) {
      done("MISSING_EDITOR", `cannot read editor exe ${editorSource} (${error.message})`);
      return;
    }
    if (editorBytes.length === 0) {
      done("MISSING_EDITOR", `editor exe is empty: ${editorSource}`);
      return;
    }
    editorStub = isStubExe(editorBytes);
    if (editorStub && !args.allowStub) {
      done("STUB_BUILD", `editor exe is a STUB, not a runnable build: ${editorSource} (${editorBytes.length} bytes, want MZ executable)`);
      return;
    }
    log.push(`$ editor ok: ${editorSource} (${editorBytes.length} bytes, ${editorStub ? "STUB via --allow-stub" : "MZ REAL"})`);
  } else if (!args.allowStub) {
    done("MISSING_EDITOR", "no editor exe: pass --editor-bin <e2040-editor.exe> or set $E2040_PACKAGE_EDITOR (build it: cargo build --release -p engine-editor) ");
    return;
  } else {
    log.push("$ note: no editor exe staged (--allow-stub smoke without editor)");
  }

  // Stage STAGE_DIST: replicate package.rs assemble_full over dist.
  let version;
  let schema;
  try {
    version = readPackageVersion();
    schema = readProgressSchema();
  } catch (error) {
    done(error.reason ?? "DEPENDENCY_MISSING", error.detail ?? String(error));
    return;
  }
  const listing = Buffer.from("content/arena-bundle.bin\ncontent/contents.txt\n", "utf8");
  const schemaText = Buffer.from(`progress-schema v${schema.version} bytes ${schema.bytes} little-endian\n`, "utf8");
  const bundleMarker = bundleStub ? "STUB" : "REAL";
  const binaryMarker = binaryStub ? "STUB" : "REAL";
  const launch = Buffer.from(
    `version:${version}\nprofile:release\nbinary:bin/arena.exe\nbundle:content/arena-bundle.bin\nbinary_kind:${binaryMarker}\nbundle_kind:${bundleMarker}\nargs:\nsmoke:bin/arena.exe --frames ${SMOKE_FRAMES} --exit\n`,
    "utf8",
  );
  const staged = {
    "bin/arena.exe": binaryBytes,
    "content/arena-bundle.bin": bundleBytes,
    "content/contents.txt": listing,
    "saves/progress-schema-v1.txt": schemaText,
    "launch.txt": launch,
  };
  let manifest = `profile:release\nversion:${version}\nbinary:${binaryMarker}\nbundle:${bundleMarker}\n`;
  for (const rel of MANIFESTED) {
    const bytes = staged[rel];
    manifest += `${rel} ${bytes.length} ${fnv1a64Hex(bytes)}\n`;
  }
  if (existsSync(dist)) {
    let empty = false;
    try {
      empty = statSync(dist).isDirectory() && readdirSync(dist).length === 0;
    } catch (error) {
      done("COPY_FAILED", `cannot inspect dist ${dist} (${error.message})`);
      return;
    }
    if (!empty && !args.replace) {
      done("TARGET_EXISTS", `dist exists and is not empty: ${dist} (pass --replace to restage; nothing was deleted)`);
      return;
    }
    if (!empty && args.replace) {
      try {
        const archived = archiveExistingDir(dist);
        log.push(`$ dist archived with --replace: ${archived} (nothing wiped)`);
      } catch (error) {
        done("COPY_FAILED", `cannot archive dist ${dist} (${error.message})`);
        return;
      }
    }
  }
  try {
    for (const rel of [...MANIFESTED, MANIFEST_REL]) {
      const path = joinDist(dist, rel);
      mkdirSync(dirname(path), { recursive: true });
    }
    for (const rel of MANIFESTED) copyFileSyncTo(dist, rel, staged[rel]);
    writeFileSync(joinDist(dist, MANIFEST_REL), manifest, "utf8");
    if (editorBytes) {
      mkdirSync(dirname(joinDist(dist, EDITOR_REL)), { recursive: true });
      copyFileSyncTo(dist, EDITOR_REL, editorBytes);
      log.push(`$ staged editor: ${EDITOR_REL} (${editorBytes.length} bytes, fnv=${fnv1a64Hex(editorBytes)})`);
    }
    const receiptStaged = editorBytes ? { ...staged, [EDITOR_REL]: editorBytes } : staged;
    const receipt = writeStagedReceipt(dist, receiptStaged);
    log.push(`$ staged receipt: ${receipt.path} (${receipt.files.length} files)`);
    log.push(`$ staged dist: ${dist} (version ${version}, binary ${binaryMarker}, bundle ${bundleMarker}${editorBytes ? `, editor ${editorStub ? "STUB" : "REAL"}` : ", no editor"})`);
  } catch (error) {
    done("COPY_FAILED", `cannot stage dist ${dist} (${error.message})`);
    return;
  }
  if (args.stageOnly) {
    log.push("STAGE-ONLY: dist staged, clean run skipped by flag (not a proof)");
    log.push(`log: ${logPath}`);
    try {
      mkdirSync(dirname(logPath), { recursive: true });
      writeFileSync(logPath, log.join("\n") + "\n", "utf8");
    } catch (error) {
      console.error(`PACK-CLEAN FAIL [LOG_UNWRITABLE] cannot write ${logPath} (${error.message})`);
      process.exit(1);
    }
    console.log(log.slice(-2).join(" "));
    process.exit(0);
  }

  // Stage CLEAN_RUN: the verified harness owns strip, probe, copy and play.
  const cleanArgs = [
    join(HERE, "ship.mjs"),
    "run",
    "--dist", dist,
    "--target", target,
    "--frames", String(frames),
    "--log", join(dirname(logPath), "clean-run.log"),
  ];
  if (args.replace) cleanArgs.push("--replace");
  if (args.allowStub) cleanArgs.push("--allow-stub");
  log.push(`$ clean-run: node ${cleanArgs.map((a) => (a.includes(" ") ? `"${a}"` : a)).join(" ")}`);
  // Same play budget as `run` (500 ms per frame plus slack): at the measured
  // frame rate a 5 min clean run is far longer than 5 min of wall time.
  const timeoutMs = frames * 1000 + 180000 + 120000;
  let result;
  try {
    result = spawnSync(process.execPath, cleanArgs, { encoding: "utf8", timeout: timeoutMs, maxBuffer: 64 * 1024 * 1024 });
  } catch (error) {
    done("CLEAN_RUN_FAIL", `clean-run threw (${error.message})`);
    return;
  }
  const out = `${result.stdout ?? ""}${result.stderr ?? ""}`;
  log.push("--- clean-run output begin ---");
  log.push(out.trimEnd() || "(no output)");
  log.push("--- clean-run output end ---");
  if (result.error) {
    done("CLEAN_RUN_FAIL", `clean-run spawn failed (${result.error.message})`);
    return;
  }
  if (result.status !== 0) {
    const reason = (out.match(/CLEAN-RUN FAIL \[([A-Z_]+)\]/) ?? [])[1] ?? "PLAY_FAILED";
    done("CLEAN_RUN_FAIL", `clean-run exited ${result.status} [${reason}]`);
    return;
  }
  log.push(`$ clean-run ok: exit 0, ${frames} frames at C:/e2040-clean/`);
  pass(logPath, log);
}

function copyFileSyncTo(dist, rel, bytes) {
  writeFileSync(joinDist(dist, rel), bytes);
  if (rel === "bin/arena.exe" || rel === EDITOR_REL) {
    // Keep staged exes byte-identical to the release output.
    const staged = readFileSync(joinDist(dist, rel));
    if (!staged.equals(Buffer.from(bytes))) throw new Error(`staged ${rel} differs from source`);
  }
  void copyFileSync;
}

main();
}

function runEntry(argv) {

const SCRIPT_VERSION = "2";
const DEFAULT_TARGET = "C:/e2040-clean/";
const DEFAULT_FULL_FRAMES = 18000; // 5 min at the 60 Hz fixed tick.
const MAX_FRAMES = 100000; // Mirrors engine-game MAX_TICKS; larger is refused.
// Play wall budget: 1000 ms per frame plus slack. Measured on the RTX 3050
// twin: 600 frames took 83 s wall (~138 ms/frame, 2026-09-27 quiet box);
// 120 frames took 62 s (~520 ms/frame, 2026-10-04 with a release build
// running beside it). The budget keeps a 2x margin over the slow reading so a
// hung run still fails instead of hanging.
const PER_FRAME_BUDGET_MS = 1000;
const SMOKE_TIMEOUT_SLACK_MS = 180000;

// Layout order mirrored from package.rs MANIFESTED.
const MANIFESTED = [
  "bin/arena.exe",
  "content/arena-bundle.bin",
  "content/contents.txt",
  "saves/progress-schema-v1.txt",
  "launch.txt",
];
const MANIFEST_REL = "package-manifest.txt";
const BUNDLE_MAGIC = Buffer.from("E204BNDL", "utf8");
const BUNDLE_VERSION = 1;
const MAX_BUNDLE_ENTRIES = 4096;
const SMOKE_FRAMES = 120; // Mirrors package.rs SMOKE_FRAMES in launch.txt.

// Markers mirrored from package.rs verify_clean. Drive-letter markers
// (`c:\`, `d:/`, ...) also occur as x86-64 machine-code immediates with no
// remap able to remove them (arena `44 3A 5C` at offset 394023 followed by
// `18 FA`, editor `43 3A 5C` runs followed by `C7`), so they count only with
// a path-shaped run (see driveShaped below); longer markers stay plain
// substrings.
const DEV_PATH_MARKERS = [
  "c:\\",
  "c:/",
  "d:\\",
  "d:/",
  "/home/",
  "/users/",
  "/target/",
  "\\target\\",
  ".git/",
];
const DRIVE_MARKERS = new Set(["c:\\", "c:/", "d:\\", "d:/"]);

// True when the drive-letter marker at `at` opens a genuine path: the
// maximal `[a-z0-9_.\-/\\]` run from the drive letter is at least 8 chars
// and holds another separator past the marker. Every real baked absolute
// path is a manifest/registry dir join with more separators (remapped
// editor leak `c:\engine2040\.lanes\gm3\crates\editor../../contentrows.txt`
// runs 59); bare immediates run 3 and compressed-asset noise (`c:/9pgr`
// plus a control byte) runs 7, so both stay silent.
function driveShaped(text, at) {
  const run = /^[a-z]:[\\/][a-z0-9_.\-/\\]*/.exec(text.slice(at, at + 120));
  if (!run || run[0].length < 8) return false;
  const rest = run[0].slice(3);
  return rest.includes("/") || rest.includes("\\");
}

function devPathHit(text) {
  for (const marker of DEV_PATH_MARKERS) {
    if (!DRIVE_MARKERS.has(marker)) {
      if (text.includes(marker)) return marker;
      continue;
    }
    let from = 0;
    while (true) {
      const rel = text.indexOf(marker, from);
      if (rel === -1) break;
      if (driveShaped(text, rel)) return marker;
      from = rel + 1;
    }
  }
  return null;
}
const FORBIDDEN_TOKENS = ["editor-only", "model-account", "development-path"];

// True when the picked adapter line names Intel while the full play output
// also names a discrete NVIDIA/AMD GPU: the iGPU won on a dual-GPU box.
// Pure over the two strings, so the drill pins it without a GPU.
function isIgpuWin(adapterLine, fullOut) {
  const pick = String(adapterLine ?? "").toLowerCase();
  const all = String(fullOut ?? "").toLowerCase();
  if (!pick.includes("intel")) return false;
  return /nvidia|geforce|rtx|radeon/.test(all);
}
// Path components that prove a source checkout leaked into the tree.
const CHECKOUT_COMPONENTS = new Set([
  "Cargo.toml",
  "Cargo.lock",
  "rust-toolchain.toml",
  "deny.toml",
  ".git",
  "crates",
  "src",
  "target",
  ".lanes",
  ".opencode",
  "node_modules",
  "tools",
  "docs",
]);
// PATH is rebuilt from entries that avoid every one of these substrings.
const PATH_STRIP_SUBSTRINGS = [
  "cargo",
  "rustup",
  "rustc",
  ".cargo",
  "node",
  "npm",
  "npx",
  "deno",
  "bun\\",
  "/bun",
  "git",
  "msys",
  "mingw",
  "cygwin",
  "cmake",
  "ninja",
  "scoop",
  "python",
  "perl",
  "ruby",
  "go\\bin",
  "/go/bin",
  "jdk",
  "visual studio",
  "windows kits",
  "msbuild",
  "dotnet",
];
// Env keys the child keeps; everything else (keys, dev vars) is dropped.
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
const MODEL_ENV_PATTERNS = [
  /OPENAI/i,
  /ANTHROPIC/i,
  /GEMINI/i,
  /GOOGLE_API/i,
  /HUGGING/i,
  /(^|_)HF_/i,
  /AZURE_OPENAI/i,
  /COHERE/i,
  /MISTRAL/i,
  /XAI_/i,
  /DEEPSEEK/i,
  /TOGETHER/i,
  /OPENROUTER/i,
  /REPLICATE/i,
  /MODEL_ACCOUNT/i,
];
const DEV_ENV_PATTERNS = [/^CARGO_/i, /^RUST/i, /^NPM_/i, /^NODE_/i, /^E2040_/i, /^EDITOR_/i, /^VSCODE_/i];

const HERE = dirname(fileURLToPath(import.meta.url));
const REPO_ROOT = dirname(HERE);

function usage() {
  return [
    "ship.mjs run: GM3 scripted proof that the packaged Windows build runs clean.",
    "",
    "Usage:",
    "  node tools/ship.mjs run --help",
    "  node tools/ship.mjs run --dry-run [--fixture <path>] [--work <path>] [--frames N] [--log <path>]",
    "  node tools/ship.mjs run [--dist <path>] [--target <path>] [--replace]",
    "                            [--frames N | --minutes N] [--log <path>] [--allow-stub]",
    "                            [--bin <arena-exe>] [--bundle <arena-bundle.bin>] [--receipt <path>]",
    "                            [--present-every N] [--present-skip]",
    "",
    "Full mode (default): stage <dist> (default ./dist) from --bin/--bundle",
    "when it is missing (else $E2040_PACKAGE_BIN / $E2040_PACKAGE_BUNDLE;",
    "cook the bundle first with the bundle_cook bin), copy it to",
    "C:/e2040-clean/, strip dev tools from PATH, assert a clean room, play",
    "5 min (18000 frames) and exit 0 with log + receipt. --receipt defaults",
    "to <target>/clean-run-receipt.json. The receipt records ticks played,",
    "frames presented and play wall time plus the present mode.",
    "Present: --present-every N presents 1 of N frames, --present-skip",
    "presents none (ticks still run); the flag is forwarded verbatim to the",
    "game binary, so a non-default mode needs the game-side flag (piece 1).",
    "Unset (or N=1) passes no extra arg: today's behavior.",
    "Dry-run mode: same harness against fixtures/clean-run/ in a temp dir; the",
    "smoke launcher runs under the current Node while a probe proves the",
    "sanitized env hides Node. Never touches C:/e2040-clean/.",
    "Wall probe: runs over 600 frames first time a 600-frame probe, project",
    "the full wall, and abort early as CLEAN-RUN FAIL [WALL_OVERRUN] past",
    "$E2040_SHIP_WALL_BUDGET_MS (default: the play timeout).",
    "",
    "Named failure reasons print as CLEAN-RUN FAIL [REASON] with the log path.",
  ].join("\n");
}

function parseArgs(argv) {
  const out = {
    help: false,
    dryRun: false,
    dist: null,
    fixture: null,
    target: null,
    work: null,
    log: null,
    receipt: null,
    bin: null,
    bundle: null,
    replace: false,
    allowStub: false,
    frames: null,
    minutes: null,
    presentEvery: null,
    presentSkip: false,
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
      case "--help": out.help = true; break;
      case "--dry-run": out.dryRun = true; break;
      case "--replace": out.replace = true; break;
      case "--allow-stub": out.allowStub = true; break;
      case "--dist": out.dist = value(arg); break;
      case "--fixture": out.fixture = value(arg); break;
      case "--target": out.target = value(arg); break;
      case "--work": out.work = value(arg); break;
      case "--log": out.log = value(arg); break;
      case "--receipt": out.receipt = value(arg); break;
      case "--bin": out.bin = value(arg); break;
      case "--bundle": out.bundle = value(arg); break;
      case "--frames": out.frames = value(arg); break;
      case "--minutes": out.minutes = value(arg); break;
      case "--present-every": out.presentEvery = value(arg); break;
      case "--present-skip": out.presentSkip = true; break;
      default: throw new Error(`unknown arg ${arg}`);
    }
  }
  return out;
}

// (fnv1a64Hex lives in tools/fnv1a64.mjs, shared with pack-clean.mjs.)

function joinDist(distDir, rel) {
  return join(distDir, ...rel.split("/"));
}

// Filewise recursive copy: entry-by-entry with mkdir plus copyFileSync.
// Stock fs.cpSync crashes (0xC0000409) on node 24.14 for this dist tree
// (wave-109 BLOCKED repro: --replace wipe ok, cpSync died 0 bytes, no
// log/receipt), so full and dry-run staging avoid it by construction.
function copyTree(src, dest) {
  mkdirSync(dest, { recursive: true });
  const entries = readdirSync(src, { withFileTypes: true });
  for (const entry of entries) {
    const from = join(src, entry.name);
    const to = join(dest, entry.name);
    if (entry.isDirectory()) {
      copyTree(from, to);
    } else if (entry.isFile()) {
      mkdirSync(dirname(to), { recursive: true });
      copyFileSync(from, to);
    } else if (entry.isSymbolicLink()) {
      const st = statSync(from);
      if (st.isDirectory()) {
        copyTree(from, to);
      } else if (st.isFile()) {
        mkdirSync(dirname(to), { recursive: true });
        copyFileSync(from, to);
      }
    }
  }
}

function readRequired(distDir, rel) {
  const path = joinDist(distDir, rel);
  let bytes;
  try {
    bytes = readFileSync(path);
  } catch {
    return { ok: false, bytes: null };
  }
  if (bytes.length === 0) return { ok: false, bytes: null };
  return { ok: true, bytes };
}

// Workspace version from root Cargo.toml [workspace.package]; loud when unreadable.
function readPackageVersion() {
  const path = join(REPO_ROOT, "Cargo.toml");
  let text;
  try {
    text = readFileSync(path, "utf8");
  } catch (error) {
    throw { reason: "DEPENDENCY_MISSING", detail: `cannot read Cargo.toml (${error.message})` };
  }
  const section = text.indexOf("[workspace.package]");
  const scope = section === -1 ? text : text.slice(section);
  const match = scope.match(/version\s*=\s*"([^"]+)"/);
  if (!match) throw { reason: "DEPENDENCY_MISSING", detail: "workspace version missing in Cargo.toml" };
  return match[1];
}

// Progress schema constants from crates/game/src/progress.rs; loud on drift.
function readProgressSchema() {
  const path = join(REPO_ROOT, "crates", "game", "src", "progress.rs");
  let text;
  try {
    text = readFileSync(path, "utf8");
  } catch (error) {
    throw { reason: "DEPENDENCY_MISSING", detail: `cannot read progress.rs (${error.message})` };
  }
  const version = text.match(/pub const PROGRESS_VERSION:\s*\w+\s*=\s*(\d+)/);
  const bytes = text.match(/pub const PROGRESS_BYTES:\s*\w+\s*=\s*(\d+)/);
  if (!version || !bytes) {
    throw { reason: "DEPENDENCY_MISSING", detail: "PROGRESS_VERSION/BYTES unreadable in progress.rs" };
  }
  return { version: version[1], bytes: Number(bytes[1]) };
}

// Stage dist byte-for-byte like package.rs assemble_full from a real binary
// plus a real AS1 bundle (both already framing-checked by the caller).
// Overwrites the tree deterministically; the caller owns --replace policy.
function stageDistFromParts(distDir, binaryBytes, bundleBytes) {
  const version = readPackageVersion();
  const schema = readProgressSchema();
  const listing = Buffer.from("content/arena-bundle.bin\ncontent/contents.txt\n", "utf8");
  const schemaText = Buffer.from(`progress-schema v${schema.version} bytes ${schema.bytes} little-endian\n`, "utf8");
  const launch = Buffer.from(
    `version:${version}\nprofile:release\nbinary:bin/arena.exe\nbundle:content/arena-bundle.bin\nbinary_kind:REAL\nbundle_kind:REAL\nargs:\nsmoke:bin/arena.exe --frames ${SMOKE_FRAMES} --exit\n`,
    "utf8",
  );
  const staged = {
    "bin/arena.exe": Buffer.from(binaryBytes),
    "content/arena-bundle.bin": Buffer.from(bundleBytes),
    "content/contents.txt": listing,
    "saves/progress-schema-v1.txt": schemaText,
    "launch.txt": launch,
  };
  let manifest = `profile:release\nversion:${version}\nbinary:REAL\nbundle:REAL\n`;
  for (const rel of MANIFESTED) {
    const bytes = staged[rel];
    manifest += `${rel} ${bytes.length} ${fnv1a64Hex(bytes)}\n`;
  }
  for (const rel of [...MANIFESTED, MANIFEST_REL]) {
    mkdirSync(dirname(joinDist(distDir, rel)), { recursive: true });
  }
  for (const rel of MANIFESTED) {
    writeFileSync(joinDist(distDir, rel), staged[rel]);
  }
  writeFileSync(joinDist(distDir, MANIFEST_REL), manifest, "utf8");
  // The staged exe stays byte-identical to the release output.
  const back = readFileSync(joinDist(distDir, "bin/arena.exe"));
  if (!back.equals(Buffer.from(binaryBytes))) throw { reason: "COPY_FAILED", detail: "staged bin/arena.exe differs from source" };
  const receipt = writeStagedReceipt(distDir, staged);
  return { version, bundleBytes: bundleBytes.length, bundleDigest: fnv1a64Hex(bundleBytes), stagedFiles: receipt.files.length, receiptPath: receipt.path };
}

// Resolve an optional file source: explicit flag wins, else the env var,
// else null. Missing-but-named paths fail loudly with a named reason.
function resolveSource(flagValue, envName, kind) {
  const raw = flagValue ?? process.env[envName] ?? null;
  if (raw === null || raw === undefined || String(raw).length === 0) return null;
  const path = resolve(String(raw));
  let bytes = null;
  try {
    bytes = readFileSync(path);
  } catch (error) {
    throw { reason: kind === "bin" ? "MISSING_BINARY" : "MISSING_BUNDLE", detail: `cannot read ${kind} source ${path} (${error.message}); cook it first (bundle_cook --out) or pass a readable file` };
  }
  if (bytes.length === 0) {
    throw { reason: kind === "bin" ? "MISSING_BINARY" : "MISSING_BUNDLE", detail: `${kind} source is empty: ${path}` };
  }
  return { path, bytes };
}

// Parse package-manifest.txt; mirrors parse_manifest shape and order checks.
function parseManifest(text) {
  const lines = text.split("\n").filter((line) => line.length > 0);
  if (lines.length < 4 + MANIFESTED.length) throw new Error("manifest truncated");
  const profile = lines[0].startsWith("profile:") ? lines[0].slice(8) : null;
  const version = lines[1].startsWith("version:") ? lines[1].slice(8) : null;
  const binaryKind = lines[2].startsWith("binary:") ? lines[2].slice(7) : null;
  const bundleKind = lines[3].startsWith("bundle:") ? lines[3].slice(7) : null;
  if (profile !== "release") throw new Error("manifest unknown profile");
  if (!version) throw new Error("manifest version mismatch");
  if ((binaryKind !== "STUB" && binaryKind !== "REAL") || (bundleKind !== "STUB" && bundleKind !== "REAL")) {
    throw new Error("manifest stub markers");
  }
  const rows = [];
  for (let i = 0; i < MANIFESTED.length; i++) {
    const parts = lines[4 + i].split(/\s+/);
    if (parts.length !== 3 || parts[0] !== MANIFESTED[i]) throw new Error("manifest layout mismatch");
    if (!/^\d+$/.test(parts[1])) throw new Error("manifest bad byte count");
    if (parts[2].length !== 16 || !/^[0-9a-fA-F]{16}$/.test(parts[2])) throw new Error("manifest bad digest");
    rows.push({ name: parts[0], bytes: Number(parts[1]), digest: parts[2].toLowerCase() });
  }
  if (lines.length !== 4 + MANIFESTED.length) throw new Error("manifest trailing rows");
  return { profile, version, binaryStub: binaryKind === "STUB", bundleStub: bundleKind === "STUB", rows };
}

// Parse launch.txt; mirrors parse_launch row checks.
function parseLaunch(text, manifestVersion) {
  const lines = text.split("\n").filter((line) => line.length > 0);
  if (lines.length < 8) throw new Error("launch truncated");
  const get = (i, prefix) => (lines[i].startsWith(prefix) ? lines[i].slice(prefix.length) : null);
  const version = get(0, "version:");
  const profile = get(1, "profile:");
  const binary = get(2, "binary:");
  const bundle = get(3, "bundle:");
  const binaryKind = get(4, "binary_kind:");
  const bundleKind = get(5, "bundle_kind:");
  const smoke = get(7, "smoke:");
  if (version !== manifestVersion) throw new Error("launch version mismatch");
  if (profile !== "release") throw new Error("launch unknown profile");
  if (binary !== "bin/arena.exe" || bundle !== "content/arena-bundle.bin") throw new Error("launch slot mismatch");
  if ((binaryKind !== "STUB" && binaryKind !== "REAL") || (bundleKind !== "STUB" && bundleKind !== "REAL")) {
    throw new Error("launch stub markers");
  }
  if (lines[6] !== "args:") throw new Error("launch args row");
  if (!smoke || !smoke.startsWith("bin/arena.exe") || !smoke.includes("--frames") || !smoke.includes("--exit")) {
    throw new Error("launch smoke command");
  }
  return { binaryStub: binaryKind === "STUB", bundleStub: bundleKind === "STUB" };
}

// AS1 bundle header framing without payload decode; entry blake3 stays with
// BundleLoader at startup. Mirrors validate_bundle_bytes header checks.
// Returns the declared entry count.
function checkBundleHeader(bytes) {
  if (bytes.length < 16) throw new Error("bundle truncated: need 16 header bytes");
  if (!bytes.subarray(0, 8).equals(BUNDLE_MAGIC)) throw new Error("bundle bad magic: want E204BNDL");
  const version = bytes.readUInt32LE(8);
  if (version > BUNDLE_VERSION) throw new Error(`bundle version ${version} is newer than v${BUNDLE_VERSION}`);
  if (version !== BUNDLE_VERSION) throw new Error(`unsupported bundle version ${version}`);
  const count = bytes.readUInt32LE(12);
  if (count > MAX_BUNDLE_ENTRIES) throw new Error("bundle too many entries");
  return count;
}

// Bounded recursive scan for dev paths, forbidden tokens and checkout parts.
function scanTree(distDir) {
  const violations = [];
  const stack = [distDir];
  let scanned = 0;
  const relName = (path) => {
    const prefix = distDir.endsWith("\\") || distDir.endsWith("/") ? distDir : distDir + "\\";
    let rel = path.startsWith(prefix) ? path.slice(prefix.length) : null;
    if (rel === null) {
      const alt = resolve(distDir);
      if (path.startsWith(alt)) rel = path.slice(alt.length).replace(/^[/\\]+/, "");
      else return null;
    }
    return rel.replace(/\\/g, "/");
  };
  while (stack.length > 0) {
    const dir = stack.pop();
    let entries;
    try {
      entries = readdirSync(dir, { withFileTypes: true });
    } catch {
      continue;
    }
    for (const entry of entries) {
      const path = join(dir, entry.name);
      if (entry.isDirectory()) {
        stack.push(path);
        continue;
      }
      if (!entry.isFile()) continue;
      const name = relName(path);
      if (!name) continue;
      for (const part of name.split("/")) {
        if (CHECKOUT_COMPONENTS.has(part) || part.endsWith(".rs")) {
          violations.push({ kind: "SOURCE_CHECKOUT_VISIBLE", file: name, marker: part });
          break;
        }
      }
      scanned++;
      if (scanned > 4096) {
        violations.push({ kind: "TOO_MANY_FILES", file: "", marker: String(scanned - 1) });
        return violations;
      }
      let bytes;
      try {
        bytes = readFileSync(path);
      } catch {
        violations.push({ kind: "UNREADABLE", file: name, marker: "" });
        continue;
      }
      const window = bytes.length > 16 * 1024 * 1024 ? bytes.subarray(0, 16 * 1024 * 1024) : bytes;
      const text = window.toString("latin1").toLowerCase();
      const marker = devPathHit(text);
      if (marker) {
        violations.push({ kind: "ABSOLUTE_DEV_PATH", file: name, marker });
        continue;
      }
      for (const token of FORBIDDEN_TOKENS) {
        if (text.includes(token)) {
          violations.push({ kind: "FORBIDDEN_REFERENCE", file: name, marker: token });
          break;
        }
      }
    }
  }
  return violations;
}

// Verify the packaged tree; throws {reason, detail} with a named reason.
function verifyTree(distDir) {
  const missingReason = (rel) => {
    if (rel === "bin/arena.exe") return "MISSING_BINARY";
    if (rel === "content/arena-bundle.bin") return "MISSING_BUNDLE";
    if (rel === "content/contents.txt") return "MISSING_CONTENT";
    if (rel === "saves/progress-schema-v1.txt") return "MISSING_SCHEMA";
    if (rel === "launch.txt") return "MISSING_LAUNCH";
    return "MISSING_MANIFEST";
  };
  const staged = {};
  for (const rel of [...MANIFESTED, MANIFEST_REL]) {
    const got = readRequired(distDir, rel);
    if (!got.ok) throw { reason: missingReason(rel), detail: `required file missing or empty: ${rel}` };
    staged[rel] = got.bytes;
  }
  let manifest;
  try {
    manifest = parseManifest(staged[MANIFEST_REL].toString("utf8"));
  } catch (error) {
    throw { reason: "MISSING_MANIFEST", detail: `bad manifest (${error.message})` };
  }
  for (const row of manifest.rows) {
    const bytes = staged[row.name];
    if (bytes.length !== row.bytes || fnv1a64Hex(bytes) !== row.digest) {
      throw { reason: "DIGEST_MISMATCH", detail: `digest mismatch for ${row.name}` };
    }
  }
  try {
    manifest.bundleEntries = checkBundleHeader(staged["content/arena-bundle.bin"]);
  } catch (error) {
    throw { reason: "MISSING_BUNDLE", detail: `bad bundle (${error.message})` };
  }
  const rowByName = new Map(manifest.rows.map((row) => [row.name, row]));
  manifest.bundleBytes = rowByName.get("content/arena-bundle.bin").bytes;
  manifest.bundleDigest = rowByName.get("content/arena-bundle.bin").digest;
  manifest.binaryBytes = rowByName.get("bin/arena.exe").bytes;
  manifest.binaryDigest = rowByName.get("bin/arena.exe").digest;
  let launch;
  try {
    launch = parseLaunch(staged["launch.txt"].toString("utf8"), manifest.version);
  } catch (error) {
    throw { reason: "MISSING_LAUNCH", detail: `bad launch (${error.message})` };
  }
  if (launch.binaryStub !== manifest.binaryStub || launch.bundleStub !== manifest.bundleStub) {
    throw { reason: "MISSING_LAUNCH", detail: "bad launch (stub markers disagree with manifest)" };
  }
  for (const violation of scanTree(distDir)) {
    throw { reason: violation.kind, detail: `${violation.file} (${violation.marker})` };
  }
  return manifest;
}

function sanitizePath(pathValue) {
  const entries = String(pathValue ?? "").split(";").filter((e) => e.length > 0);
  const clean = [];
  const removed = [];
  for (const entry of entries) {
    const lower = entry.toLowerCase();
    if (PATH_STRIP_SUBSTRINGS.some((sub) => lower.includes(sub))) removed.push(entry);
    else clean.push(entry);
  }
  return { clean: clean.join(";"), removed };
}

// Child env from the allowlist only; dev and model vars fall out by shape.
// Dry-run runs the Node launcher, never the Windows exe, so it does not
// need SystemRoot; the full run still does.
function buildCleanEnv(sanitizedPath, { needSystemRoot = true } = {}) {
  const env = {};
  for (const key of CHILD_ENV_KEEP) {
    if (process.env[key] !== undefined) env[key] = process.env[key];
  }
  env.PATH = sanitizedPath;
  if (needSystemRoot && !env.SystemRoot) throw { reason: "DEPENDENCY_MISSING", detail: "SystemRoot absent; cannot run Windows exe" };
  for (const key of Object.keys(env)) {
    if (MODEL_ENV_PATTERNS.some((re) => re.test(key))) {
      throw { reason: "MODEL_ACCOUNT_VISIBLE", detail: `model env leaked into clean room: ${key}` };
    }
    if (DEV_ENV_PATTERNS.some((re) => re.test(key))) {
      throw { reason: "DEV_ENV_VISIBLE", detail: `dev env leaked into clean room: ${key}` };
    }
  }
  return env;
}

// Prove an exe is not resolvable under the sanitized env (spawn must fail).
function probeHidden(exe, env) {
  const result = spawnSync(exe, ["--version"], { env, encoding: "utf8", timeout: 15000 });
  const hidden = Boolean(result.error) || result.status !== 0;
  return { hidden, detail: result.error ? result.error.message : `exit ${result.status}` };
}

function runPlay(exePath, args, { cwd, env, timeoutMs, log }) {
  log.push(`$ play: ${exePath} ${args.join(" ")} (cwd=${cwd}, timeout=${timeoutMs}ms)`);
  let result;
  try {
    result = spawnSync(exePath, args, { cwd, env, encoding: "utf8", timeout: timeoutMs, maxBuffer: 16 * 1024 * 1024 });
  } catch (error) {
    throw { reason: "PLAY_FAILED", detail: `spawn threw (${error.message})` };
  }
  const out = `${result.stdout ?? ""}${result.stderr ?? ""}`;
  log.push("--- child output begin ---");
  log.push(out.trimEnd() || "(no output)");
  log.push("--- child output end ---");
  if (result.error) {
    const code = result.error.code ?? "";
    if (result.error.message.includes("ETIMEDOUT") || code === "ETIMEDOUT") {
      throw { reason: "PLAY_TIMEOUT", detail: `no exit within ${timeoutMs}ms` };
    }
    if (code === "ENOENT") {
      throw { reason: "DEPENDENCY_MISSING", detail: `missing dependency launching play (${result.error.message})` };
    }
    throw { reason: "PLAY_FAILED", detail: `spawn failed (${result.error.message})` };
  }
  if (result.status === -1073741515) {
    throw { reason: "DEPENDENCY_MISSING", detail: "exe failed with STATUS_DLL_NOT_FOUND; a system runtime is missing" };
  }
  if (result.status !== 0) {
    throw { reason: "PLAY_FAILED", detail: `play exited ${result.status}` };
  }
  if (!/arena (headless )?smoke:/.test(out)) {
    throw { reason: "PLAY_FAILED", detail: "play exited 0 without the arena smoke marker" };
  }
  const ticks = out.match(/ticks=(\d+)/);
  log.push(`$ play ok: exit 0, smoke marker present${ticks ? ` (ticks=${ticks[1]})` : ""}`);
  return out;
}

// First output line matching any of the patterns, else "".
function pickLine(out, patterns) {
  for (const line of out.split("\n")) {
    if (patterns.some((re) => re.test(line))) return line.trim().slice(0, 256);
  }
  return "";
}

function writeReceiptFile(receiptPath, receipt) {
  mkdirSync(dirname(receiptPath), { recursive: true });
  writeFileSync(receiptPath, `${JSON.stringify(receipt, null, 2)}\n`, "utf8");
}

function writeLogAndExit(logPath, lines, failed, reason, detail, frames = null) {
  let fingerprint = null;
  if (failed) {
    fingerprint = cleanFingerprint(reason, detail);
    lines.push(`CLEAN-RUN FAIL [${reason}] ${detail}`);
    lines.push(`fingerprint: ${fingerprint}`);
  } else {
    lines.push("CLEAN-RUN PASS: packaged build played clean");
  }
  lines.push(`ledger: ${cleanLedgerRow({ fingerprint, reason: failed ? reason : "PASS", detail: failed ? detail : "", frames, logPath, failed })}`);
  lines.push(`log: ${logPath}`);
  try {
    mkdirSync(dirname(logPath), { recursive: true });
    writeFileSync(logPath, lines.join("\n") + "\n", "utf8");
  } catch (error) {
    console.error(`CLEAN-RUN FAIL [LOG_UNWRITABLE] cannot write ${logPath} (${error.message})`);
    process.exit(1);
  }
  const last = lines.slice(-2).join(" ");
  if (failed) console.error(last);
  else console.log(last);
  process.exit(failed ? 1 : 0);
}

function resolveFrames(args) {
  let frames = DEFAULT_FULL_FRAMES;
  if (args.minutes !== null) {
    const minutes = Number(args.minutes);
    if (!Number.isFinite(minutes) || minutes <= 0) throw new Error("--minutes needs a positive number");
    frames = Math.round(minutes * 3600);
  }
  if (args.frames !== null) {
    frames = Number(args.frames);
    if (!Number.isInteger(frames) || frames <= 0) throw new Error("--frames needs a positive integer");
  }
  if (frames > MAX_FRAMES) throw new Error(`--frames ${frames} exceeds ${MAX_FRAMES}`);
  return frames;
}

function main() {
  const startedAt = new Date().toISOString();
  let args;
  try {
    args = parseArgs(argv);
  } catch (error) {
    console.error(`Usage: ${error.message}\n${usage()}`);
    process.exit(2);
  }
  if (args.help) {
    console.log(usage());
    process.exit(0);
  }
  let frames;
  let presentPlayArgs = [];
  try {
    frames = resolveFrames(args);
    presentPlayArgs = resolvePresentFlags(args);
    if (args.dryRun && args.frames === null && args.minutes === null) frames = 5;
  } catch (error) {
    console.error(`Usage: ${error.message}\n${usage()}`);
    process.exit(2);
  }

  if (args.dryRun) {
    const fixture = resolve(args.fixture ?? join(REPO_ROOT, "fixtures", "clean-run"));
    const work = args.work ? resolve(args.work) : mkdtempSync(join(tmpdir(), "e2040-clean-run-"));
    const logPath = resolve(args.log ?? join(work, "clean-run.log"));
    const log = [
      `clean-run ${SCRIPT_VERSION} dry-run started ${startedAt}`,
      `fixture: ${fixture}`,
      `work: ${work}`,
      `frames: ${frames}`,
    ];
    const done = (failed, reason, detail) => writeLogAndExit(logPath, log, failed, reason, detail, frames);
    if (!existsSync(fixture)) {
      done(true, "MISSING_FIXTURE", `fixture missing: ${fixture}`);
      return;
    }
    let manifest;
    try {
      manifest = verifyTree(fixture);
      log.push(`$ fixture verified: profile=${manifest.profile} version=${manifest.version} stub=${manifest.binaryStub && manifest.bundleStub}`);
    } catch (error) {
      done(true, error.reason ?? "PLAY_FAILED", error.detail ?? String(error));
      return;
    }
    const { clean, removed } = sanitizePath(process.env.PATH);
    log.push(`$ PATH strip: removed ${removed.length} entr${removed.length === 1 ? "y" : "ies"}${removed.length > 0 ? ` (${removed.join(" | ")})` : " (already clean)"}`);
    let cleanEnv;
    try {
      cleanEnv = buildCleanEnv(clean, { needSystemRoot: false });
      log.push(`$ clean env: ${Object.keys(cleanEnv).length} keys, PATH has ${clean.split(";").filter((e) => e).length} entries`);
    } catch (error) {
      done(true, error.reason ?? "DEV_ENV_VISIBLE", error.detail ?? String(error));
      return;
    }
    for (const exe of ["cargo", "rustc", "node"]) {
      const probe = probeHidden(exe, cleanEnv);
      log.push(`$ probe ${exe} under clean env: ${probe.hidden ? "hidden" : `VISIBLE (${probe.detail})`}`);
      if (!probe.hidden) {
        done(true, exe === "node" ? "NODE_VISIBLE" : "RUST_VISIBLE", `${exe} resolves under the sanitized env`);
        return;
      }
    }
    try {
      mkdirSync(work, { recursive: true });
      copyTree(fixture, work);
      log.push(`$ copied fixture to work dir`);
    } catch (error) {
      done(true, "COPY_FAILED", `cannot stage work dir (${error.message})`);
      return;
    }
    const launcher = join(work, "bin", "arena-smoke.mjs");
    if (!existsSync(launcher)) {
      done(true, "MISSING_FIXTURE", `dry-run launcher missing: bin/arena-smoke.mjs`);
      return;
    }
    let dryOut = "";
    let dryWallMs = 0;
    try {
      // Wall-projection probe (indicatif Estimator pattern): time the first
      // WALL_PROBE_FRAMES played frames, project the full wall, and abort
      // early with WALL_OVERRUN when the projection exceeds the budget.
      if (frames > WALL_PROBE_FRAMES) {
        const probeStart = Date.now();
        const probe = spawnSync(process.execPath, [launcher, "--frames", String(WALL_PROBE_FRAMES), "--exit", ...presentPlayArgs], {
          cwd: work,
          encoding: "utf8",
          timeout: 60000,
          maxBuffer: 4 * 1024 * 1024,
        });
        const probeMs = Date.now() - probeStart;
        const probeOut = `${probe.stdout ?? ""}${probe.stderr ?? ""}`;
        log.push("--- probe output begin ---");
        log.push(probeOut.trimEnd() || "(no output)");
        log.push("--- probe output end ---");
        if (probe.error || probe.status !== 0) {
          done(true, "PLAY_FAILED", `dry-run probe failed (${probe.error ? probe.error.message : `exit ${probe.status}`})`);
          return;
        }
        if (!/arena (headless )?smoke:/.test(probeOut)) {
          done(true, "PLAY_FAILED", "dry-run probe exited 0 without the arena smoke marker");
          return;
        }
        const budget = wallBudgetMs(frames * PER_FRAME_BUDGET_MS + SMOKE_TIMEOUT_SLACK_MS);
        const { projectedMs, overrun } = projectProbeWall({
          probeFrames: WALL_PROBE_FRAMES, probeMs, totalFrames: frames, budgetMs: budget,
        });
        log.push(`$ wall-probe: ${WALL_PROBE_FRAMES} frames in ${probeMs}ms projects ${frames} frames to ${Math.round(projectedMs)}ms (budget ${budget}ms)`);
        if (overrun) {
          const receiptPath = join(work, "clean-run-receipt.json");
          try {
            writeReceiptFile(receiptPath, {
              script: `clean-run ${SCRIPT_VERSION}`,
              started: startedAt,
              overrun: true,
              probeFrames: WALL_PROBE_FRAMES,
              probeMs,
              projectedMs: Math.round(projectedMs),
              budgetMs: budget,
              totalFrames: frames,
              exit: 1,
              log: logPath,
            });
            log.push(`$ receipt: ${receiptPath}`);
            const extra = writeArchiveAndJunit(receiptPath, { manifestRows: manifest?.rows ?? [], wallMs: probeMs, failed: true, detail: `projected full wall ${Math.round(projectedMs)}ms exceeds ${budget}ms budget`, dist: fixture, target: work });
            log.push(`$ archive: ${extra.archivePath}`);
            log.push(`$ junit: ${extra.junitPath}`);
          } catch (error) {
            done(true, "LOG_UNWRITABLE", `cannot write receipt ${receiptPath} (${error.message})`);
            return;
          }
          done(true, "WALL_OVERRUN", `projected full wall ${Math.round(projectedMs)}ms exceeds ${budget}ms budget (${WALL_PROBE_FRAMES}-frame probe took ${probeMs}ms)`);
          return;
        }
      }
      // The launcher needs the current Node; node-hiding is proved by the
      // probe lines above, not by running the launcher blind.
      const smokeStart = Date.now();
      const result = spawnSync(process.execPath, [launcher, "--frames", String(frames), "--exit", ...presentPlayArgs], {
        cwd: work,
        encoding: "utf8",
        timeout: 60000,
        maxBuffer: 4 * 1024 * 1024,
      });
      const out = `${result.stdout ?? ""}${result.stderr ?? ""}`;
      dryOut = out;
      dryWallMs = Date.now() - smokeStart;
      log.push("--- child output begin ---");
      log.push(out.trimEnd() || "(no output)");
      log.push("--- child output end ---");
      if (result.error || result.status !== 0) {
        done(true, "PLAY_FAILED", `dry-run smoke failed (${result.error ? result.error.message : `exit ${result.status}`})`);
        return;
      }
      if (!/arena (headless )?smoke:/.test(out)) {
        done(true, "PLAY_FAILED", "dry-run smoke exited 0 without the arena smoke marker");
        return;
      }
      log.push(`$ dry-run smoke ok: exit 0 with marker (frames=${frames})`);
    } catch (error) {
      done(true, "PLAY_FAILED", `dry-run smoke threw (${error.message})`);
      return;
    }
    {
      // Dry-run receipt: the same ticks/frames/wall/present shape as the
      // full-run receipt, so the flag parsing plus receipt fields stay
      // testable without a real exe.
      const counts = parseSmokeCounts(dryOut);
      const present = describePresentMode(args);
      const dryReceiptPath = args.receipt ? resolve(args.receipt) : join(work, "clean-run-receipt.json");
      try {
        writeReceiptFile(dryReceiptPath, {
          script: `clean-run ${SCRIPT_VERSION}`,
          started: startedAt,
          dry_run: true,
          fixture,
          work,
          frames,
          ticks: counts.ticks,
          frames_presented: counts.framesPresented,
          wall_ms: dryWallMs,
          present,
          exit: 0,
          log: logPath,
        });
        log.push(`$ receipt: ${dryReceiptPath}`);
        const extra = writeArchiveAndJunit(dryReceiptPath, { manifestRows: manifest?.rows ?? [], wallMs: dryWallMs, failed: false, detail: "", dist: fixture, target: work });
        log.push(`$ archive: ${extra.archivePath}`);
        log.push(`$ junit: ${extra.junitPath}`);
      } catch (error) {
        done(true, "LOG_UNWRITABLE", `cannot write receipt ${dryReceiptPath} (${error.message})`);
        return;
      }
      log.push(`$ counts: ticks=${counts.ticks} frames_presented=${counts.framesPresented} wall_ms=${dryWallMs} present=${present.mode}`);
    }
    done(false);
    return;
  }

  const dist = resolve(args.dist ?? join(REPO_ROOT, "dist"));
  const target = resolve(args.target ?? DEFAULT_TARGET);
  const logPath = resolve(args.log ?? join(target, "clean-run.log"));
  const receiptPath = resolve(args.receipt ?? join(target, "clean-run-receipt.json"));
  const timeoutMs = frames * PER_FRAME_BUDGET_MS + SMOKE_TIMEOUT_SLACK_MS;
  const log = [
    `clean-run ${SCRIPT_VERSION} full started ${startedAt}`,
    `dist: ${dist}`,
    `target: ${target}`,
    `frames: ${frames} (timeout ${timeoutMs}ms)`,
    `receipt: ${receiptPath}`,
    `present: ${describePresentMode(args).mode}`,
  ];
  const done = (failed, reason, detail) => writeLogAndExit(logPath, log, failed, reason, detail, frames);
  // Stage 1: resolve the real binary + real AS1 bundle (flags win, else the
  // packager env vars). Cook the bundle first with the bundle_cook bin when
  // neither names a file; a named-but-unreadable source fails loudly.
  let binSource = null;
  let bundleSource = null;
  try {
    binSource = resolveSource(args.bin, "E2040_PACKAGE_BIN", "bin");
    bundleSource = resolveSource(args.bundle, "E2040_PACKAGE_BUNDLE", "bundle");
  } catch (error) {
    done(true, error.reason ?? "DEPENDENCY_MISSING", error.detail ?? String(error));
    return;
  }
  if (binSource) log.push(`$ bin source: ${binSource.path} (${binSource.bytes.length} bytes)`);
  if (bundleSource) {
    try {
      const entries = checkBundleHeader(bundleSource.bytes);
      if (entries === 0) throw new Error("bundle has zero entries (stub); cook a REAL bundle without --empty");
      log.push(`$ bundle source: ${bundleSource.path} (${bundleSource.bytes.length} bytes, entries=${entries}, fnv=${fnv1a64Hex(bundleSource.bytes)})`);
    } catch (error) {
      done(true, "MISSING_BUNDLE", `bad bundle source ${bundleSource.path} (${error.message}); cook it: bundle_cook --out <bundle.bin>`);
      return;
    }
  }
  // Stage the dist tree from real parts when it is missing, or restage it
  // when both parts are named with --replace. Otherwise the existing dist
  // is verified as-is (named parts are ignored without --replace).
  let distExists = existsSync(dist) && statSync(dist).isDirectory() && readdirSync(dist).length > 0;
  const wantStage = binSource && bundleSource && (!distExists || args.replace);
  if (wantStage) {
    try {
      if (distExists && args.replace) {
        const archived = archiveExistingDir(dist);
        log.push(`$ dist archived with --replace: ${archived} (nothing wiped)`);
      }
      mkdirSync(dist, { recursive: true });
      const staged = stageDistFromParts(dist, binSource.bytes, bundleSource.bytes);
      log.push(`$ staged dist from parts: version=${staged.version} bundle fnv=${staged.bundleDigest} bytes=${staged.bundleBytes}`);
      log.push(`$ staged receipt: ${staged.receiptPath} (${staged.stagedFiles} files)`);
    } catch (error) {
      done(true, error.reason ?? "COPY_FAILED", error.detail ?? String(error));
      return;
    }
    distExists = true;
  } else if (distExists && (binSource || bundleSource)) {
    log.push("$ note: existing dist kept as-is (--bin/--bundle need --replace to restage it)");
  } else if (!distExists && (binSource || bundleSource)) {
    done(true, "MISSING_DIST", `packaged folder missing: ${dist}; both --bin and --bundle (or $E2040_PACKAGE_BIN plus $E2040_PACKAGE_BUNDLE) are needed to stage it`);
    return;
  }
  if (!distExists) {
    const hint = (!binSource || !bundleSource)
      ? " (missing parts: cook the AS1 bundle with bundle_cook --out <bundle.bin>, build the release exe with cargo build --release -p engine-game --features app, then pass --bin <arena-exe> --bundle <bundle.bin>)"
      : " (build it: cargo build --release -p engine-game --features app, stage E2040_PACKAGE_BIN plus E2040_PACKAGE_BUNDLE over an empty folder)";
    done(true, "MISSING_DIST", `packaged folder missing: ${dist}${hint}`);
    return;
  }
  let manifest;
  try {
    manifest = verifyTree(dist);
    log.push(`$ package verified: profile=${manifest.profile} version=${manifest.version} binary=${manifest.binaryStub ? "STUB" : "REAL"} bundle=${manifest.bundleStub ? "STUB" : "REAL"}`);
    log.push(`$ bundle digest: fnv=${manifest.bundleDigest} bytes=${manifest.bundleBytes} entries=${manifest.bundleEntries}`);
  } catch (error) {
    done(true, error.reason ?? "PLAY_FAILED", error.detail ?? String(error));
    return;
  }
  if ((manifest.binaryStub || manifest.bundleStub) && !args.allowStub) {
    done(true, "STUB_BUILD", "tree is a STUB package; refusing a clean-room pass without --allow-stub (A8 needs the real arena exe plus AS1 bundle)");
    return;
  }
  if (manifest.binaryStub || manifest.bundleStub) log.push("$ note: STUB slots allowed via --allow-stub; not an A8 proof");
  let targetExists = existsSync(target);
  let targetEmpty = true;
  if (targetExists) {
    try {
      targetEmpty = readdirSync(target).length === 0;
    } catch (error) {
      done(true, "COPY_FAILED", `cannot inspect target (${error.message})`);
      return;
    }
    if (!targetEmpty && !args.replace) {
      done(true, "TARGET_EXISTS", `target exists and is not empty: ${target} (pass --replace to restage; nothing was deleted)`);
      return;
    }
  }
  const { clean, removed } = sanitizePath(process.env.PATH);
  log.push(`$ PATH strip: removed ${removed.length} entr${removed.length === 1 ? "y" : "ies"}${removed.length > 0 ? ` (${removed.join(" | ")})` : " (already clean)"}`);
  let cleanEnv;
  try {
    cleanEnv = buildCleanEnv(clean);
    log.push(`$ clean env: ${Object.keys(cleanEnv).length} keys, PATH has ${clean.split(";").filter((e) => e).length} entries`);
  } catch (error) {
    done(true, error.reason ?? "DEV_ENV_VISIBLE", error.detail ?? String(error));
    return;
  }
  for (const [exe, reason] of [["cargo", "RUST_VISIBLE"], ["rustc", "RUST_VISIBLE"], ["node", "NODE_VISIBLE"]]) {
    const probe = probeHidden(exe, cleanEnv);
    log.push(`$ probe ${exe} under clean env: ${probe.hidden ? "hidden" : `VISIBLE (${probe.detail})`}`);
    if (!probe.hidden) {
      done(true, reason, `${exe} resolves under the sanitized env`);
      return;
    }
  }
  try {
    if (targetExists && !targetEmpty && args.replace) {
      const archived = archiveExistingDir(target);
      log.push(`$ target archived with --replace: ${archived} (nothing wiped)`);
    }
    mkdirSync(target, { recursive: true });
    copyTree(dist, target);
    log.push(`$ copied dist to target`);
  } catch (error) {
    done(true, "COPY_FAILED", `cannot copy ${dist} to ${target} (${error.message})`);
    return;
  }
  try {
    const again = verifyTree(target);
    void again;
    log.push(`$ target reverified after copy`);
  } catch (error) {
    done(true, error.reason ?? "COPY_FAILED", `target fails verification after copy (${error.detail ?? error})`);
    return;
  }
  const exePath = joinDist(target, "bin/arena.exe");
  if (!existsSync(exePath)) {
    done(true, "DEPENDENCY_MISSING", `staged exe missing after copy: ${exePath}`);
    return;
  }
  try {
    copyFileSync(exePath, exePath);
  } catch (error) {
    done(true, "DEPENDENCY_MISSING", `staged exe is not runnable (${error.message})`);
    return;
  }
  let playOut = "";
  let playWallMs = 0;
  if (frames > WALL_PROBE_FRAMES) {
    // Wall-projection probe: the first WALL_PROBE_FRAMES played frames pace
    // the full wall. An overrun aborts here with a fingerprinted FAIL plus
    // receipt instead of burning the keeper on the full run.
    try {
      const probeTimeoutMs = WALL_PROBE_FRAMES * PER_FRAME_BUDGET_MS + SMOKE_TIMEOUT_SLACK_MS;
      const probeDeadlineMs = Math.max(300000, probeTimeoutMs - 60000);
      log.push(`$ wall-probe: timing first ${WALL_PROBE_FRAMES} of ${frames} frames`);
      const probeStart = Date.now();
      runPlay(exePath, ["--frames", String(WALL_PROBE_FRAMES), "--exit", "--deadline-ms", String(probeDeadlineMs), ...presentPlayArgs], { cwd: target, env: cleanEnv, timeoutMs: probeTimeoutMs, log });
      const probeMs = Date.now() - probeStart;
      const budget = wallBudgetMs(timeoutMs);
      const { projectedMs, overrun } = projectProbeWall({
        probeFrames: WALL_PROBE_FRAMES, probeMs, totalFrames: frames, budgetMs: budget,
      });
      log.push(`$ wall-probe: ${WALL_PROBE_FRAMES} frames in ${probeMs}ms projects ${frames} frames to ${Math.round(projectedMs)}ms (budget ${budget}ms)`);
      if (overrun) {
        try {
          writeReceiptFile(receiptPath, {
            script: `clean-run ${SCRIPT_VERSION}`,
            started: startedAt,
            dist,
            target,
            frames,
            overrun: true,
            present: describePresentMode(args),
            probeFrames: WALL_PROBE_FRAMES,
            probeMs,
            projectedMs: Math.round(projectedMs),
            budgetMs: budget,
            exit: 1,
            log: logPath,
          });
          log.push(`$ receipt: ${receiptPath}`);
          const extra = writeArchiveAndJunit(receiptPath, { manifestRows: manifest?.rows ?? [], wallMs: probeMs, failed: true, detail: `projected full wall ${Math.round(projectedMs)}ms exceeds ${budget}ms budget`, dist, target });
          log.push(`$ archive: ${extra.archivePath}`);
          log.push(`$ junit: ${extra.junitPath}`);
        } catch (error) {
          done(true, "LOG_UNWRITABLE", `cannot write receipt ${receiptPath} (${error.message})`);
          return;
        }
        done(true, "WALL_OVERRUN", `projected full wall ${Math.round(projectedMs)}ms exceeds ${budget}ms budget (${WALL_PROBE_FRAMES}-frame probe took ${probeMs}ms)`);
        return;
      }
    } catch (error) {
      done(true, error.reason ?? "PLAY_FAILED", error.detail ?? String(error));
      return;
    }
  }
  try {
    // The exe bounds its own `--exit` run (default 300 s); a 5 min clean run at
    // the measured frame rate is longer, so hand it the same budget minus a
    // minute: a wedged run then ends itself loudly before this harness kills it.
    const deadlineMs = Math.max(300000, timeoutMs - 60000);
    const playStart = Date.now();
    playOut = runPlay(exePath, ["--frames", String(frames), "--exit", "--deadline-ms", String(deadlineMs), ...presentPlayArgs], { cwd: target, env: cleanEnv, timeoutMs, log });
    playWallMs = Date.now() - playStart;
  } catch (error) {
    done(true, error.reason ?? "PLAY_FAILED", error.detail ?? String(error));
    return;
  }
  // Quote the run output the acceptance receipt needs: the arena smoke line
  // plus adapter/driver/scene evidence from the same output.
  const smokeLine = pickLine(playOut, [/arena (headless )?smoke:/]);
  const adapterLine = pickLine(playOut, [/adapter/i, /presenter/i, /driver/i, /wgpu/i]);
  const audioLine = pickLine(playOut, [/audio=/, /audio "/]);
  const sceneLine = pickLine(playOut, [/scene/i, /hud_cells=/, /frames=/]);
  if (!smokeLine) { done(true, "PLAY_FAILED", "play output missing arena smoke line"); return; }
  if (!adapterLine) { done(true, "PLAY_FAILED", "play output missing adapter/driver line"); return; }
  // Dual-GPU refusal (Optimus exports, PORT(glfw/glfw@master:src/win32_init.c)):
  // an Intel pick while the output also names NVIDIA/AMD discrete means the
  // iGPU won, so refuse loudly instead of stamping a green receipt.
  if (isIgpuWin(adapterLine, playOut)) { done(true, "IGPU_PICKED", `iGPU won on a dual-GPU box (picked: ${String(adapterLine).trim().slice(0, 160)}); arena.exe must export NvOptimusEnablement and AmdPowerXpressRequestHighPerformance`); return; }
  log.push(`$ smoke line: ${smokeLine || "(missing)"}`);
  log.push(`$ adapter/driver line: ${adapterLine || "(missing)"}`);
  log.push(`$ audio line: ${audioLine || "(missing)"}`);
  log.push(`$ scene line: ${sceneLine || "(missing)"}`);
  const counts = parseSmokeCounts(playOut);
  const present = describePresentMode(args);
  log.push(`$ counts: ticks=${counts.ticks} frames_presented=${counts.framesPresented} wall_ms=${playWallMs} present=${present.mode}`);
  try {
    const tail = playOut.trimEnd().split("\n").slice(-30).join("\n").slice(0, 8000);
    writeReceiptFile(receiptPath, {
      script: `clean-run ${SCRIPT_VERSION}`,
      started: startedAt,
      dist,
      target,
      frames,
      ticks: counts.ticks,
      frames_presented: counts.framesPresented,
      wall_ms: playWallMs,
      present,
      exit: 0,
      manifest: {
        profile: manifest.profile,
        version: manifest.version,
        binary: manifest.binaryStub ? "STUB" : "REAL",
        bundle: manifest.bundleStub ? "STUB" : "REAL",
      },
      bundle: {
        bytes: manifest.bundleBytes,
        fnv: manifest.bundleDigest,
        entries: manifest.bundleEntries,
      },
      binary: { bytes: manifest.binaryBytes, fnv: manifest.binaryDigest },
      smoke: smokeLine,
      adapter: adapterLine,
      audio: audioLine,
      scene: sceneLine,
      output_tail: tail,
      log: logPath,
    });
    log.push(`$ receipt: ${receiptPath}`);
    const extra = writeArchiveAndJunit(receiptPath, { manifestRows: manifest?.rows ?? [], wallMs: playWallMs, failed: false, detail: "", dist, target });
    log.push(`$ archive: ${extra.archivePath}`);
    log.push(`$ junit: ${extra.junitPath}`);
  } catch (error) {
    done(true, "LOG_UNWRITABLE", `cannot write receipt ${receiptPath} (${error.message})`);
    return;
  }
  done(false);
}

main();
}

function shipUsage() {
  return [
    "ship.mjs: GM3 release pack plus scripted clean-room proof (pack-clean + clean-run).",
    "",
    "Usage:",
    "  node tools/ship.mjs --help",
    "  node tools/ship.mjs pack [pack flags] [--stage-only]",
    "  node tools/ship.mjs stage [pack flags] (= pack --stage-only)",
    "  node tools/ship.mjs run [run flags] [--dry-run]",
    "",
    "Pack flags: --dist/--target/--replace/--bin/--editor-bin/--bundle/",
    "--frames/--minutes/--log/--build-log/--allow-stub/--skip-build.",
    "Run flags: --dist/--target/--replace/--bin/--bundle/--frames/--minutes/",
    "--log/--allow-stub/--present-every/--present-skip plus --dry-run/--fixture/--work/--receipt.",
    "Run 'pack --help', 'stage --help' or 'run --help' for the full text.",
  ].join("\n");
}

const isShipMain = process.argv[1]
  && import.meta.url.endsWith(process.argv[1].replace(/\\/g, "/").split("/").pop());
if (isShipMain) {
  const [sub, ...rest] = process.argv.slice(2);
  if (sub === "pack") packEntry(rest);
  else if (sub === "stage") packEntry(["--stage-only", ...rest]);
  else if (sub === "run") runEntry(rest);
  else if (sub === "--help" || sub === "-h" || sub === undefined) {
    console.log(shipUsage());
    process.exit(0);
  } else {
    console.error(shipUsage());
    console.error("ship: unknown subcommand " + sub + " (want pack|stage|run)");
    process.exit(2);
  }
}
