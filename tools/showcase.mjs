// SHOW1-stitch (tool half): stitch a captured PNG sequence to MP4 + GIF.
//
// Input:  target/showcase/<date>/<clip>/  (frame*.png + manifest.json)
// Set input with --set: target/showcase/<date>/ (recursive PNG census, 0 fails)
// Output: <clip>.mp4 + <clip>.gif written BESIDE the PNGs (same dir).
// Sources are never deleted; only the two outputs are created.
//
// Encoder policy (RESEARCH-VERDICT/SHOW1): external ffmpeg binary only,
// never a crate dependency, so the GM3 clean-machine run stays clean.
// Missing ffmpeg fails loudly with an install hint; no silent fallback.
//
// Manifest contract (written by SHOW1-cap, crates/playtest/src/showcase.rs):
//   {
//     "version": 1, "clip": "duel", "seed": 12345,
//     "tape": "tapes/duel.json", "rows": ["WIN1", "CR1"],
//     "sha": "<build SHA hex>", "fps": 30,
//     "frames": [{ "file": "frame0000.png", "sha256": "…", "bytes": 1234 }]
//   }
// Required: seed (uint), tape (string), rows (non-empty board-row IDs),
// sha (hex, 7..128 chars). Accepted aliases: buildSha/build_sha for sha,
// files for frames, frameCount/count/n for a bare frame count. Frames must
// be hashed objects ({file, sha256 64-hex, bytes}); string names or
// hash-less objects fail unless --allow-unhashed is passed. Row IDs must
// look like board rows (e.g. WIN1, GX10, SHOW1) and the PNGs must be valid,
// uniform size and match the manifest. A tampered manifest or PNG fails loudly.
//
// Usage from the repo/lane root:
//   node tools/showcase.mjs --clip target/showcase/2026-09-25/duel
//   node tools/showcase.mjs --clip <dir> [--fps 30] [--width 640]
//     [--out <dir>] [--manifest <path>] [--check-only] [--json] [--allow-unhashed]
//   node tools/showcase.mjs <clipDir>
//   node tools/showcase.mjs --help
import { spawnSync } from "node:child_process";
import { sha256 } from "./hash-util.mjs";
import { fail, usageFail } from "./argv.mjs";
import {
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  readdirSync,
  rmSync,
  statSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { basename, dirname, isAbsolute, join, resolve } from "node:path";

const ROW_RE = /^[A-Z]{1,4}[0-9]{1,3}[a-z]?$/;
const HEX_RE = /^[0-9a-fA-F]{7,128}$/;
// Bare output basename for manifest.clip: no directories, no parent refs, no
// absolute paths, so a hostile manifest cannot steer outputs outside the clip dir.
const CLIP_RE = /^[A-Za-z0-9][A-Za-z0-9._-]*$/;
// Per-file content hash: exactly 64 hex chars (SHA-256).
const SHA256_RE = /^[0-9a-fA-F]{64}$/;
// First 8 bytes of every PNG file.
const PNG_MAGIC = Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]);

const HELP = `showcase.mjs — stitch a SHOW1 PNG sequence to MP4 + GIF (external ffmpeg only)

Usage:
  node tools/showcase.mjs --clip target/showcase/<date>/<clip> [options]
  node tools/showcase.mjs <clipDir> [options]
  node tools/showcase.mjs --set target/showcase/<date> [--json]

Options:
  --clip <dir>      clip dir with PNGs + manifest.json (or first positional arg)
  --set <dir>       showcase set dir (a date dir holding clip subdirs): count
                    PNGs recursively and fail closed on 0 (check-only, writes
                    nothing; cannot be combined with --clip)
  --manifest <path> manifest path (default: <clip>/manifest.json)
  --fps <n>         override manifest fps (default: manifest fps or 30)
  --width <px>      GIF scale width, height follows aspect (default 640)
  --out <dir>       output dir (default: the clip dir, as <clip>.mp4/.gif)
  --check-only      validate manifest + PNGs, run no ffmpeg, write nothing
  --json            print the result summary as JSON
  --allow-unhashed  accept a manifest with no frames/files/count coverage
                    (frameless) or whose frames lack per-file sha256
                    (string names, hash-less objects); without it such
                    manifests fail loudly so byte tamper cannot pass silently
  -h, --help        print this help and exit 0

Exit codes: 0 ok · 1 validation/ffmpeg failure (incl. missing ffmpeg) · 2 usage.
Never deletes sources; overwrites only its own <clip>.mp4/.gif outputs.
manifest.clip is a bare basename (no path separators or parent refs); unsafe names fail.
`;





function parseArgs(argv) {
  const opts = {
    clip: null,
    set: null,
    manifest: null,
    fps: null,
    width: 640,
    out: null,
    checkOnly: false,
    json: false,
    allowUnhashed: false,
  };
  const positionals = [];
  for (let i = 0; i < argv.length; i += 1) {
    const a = argv[i];
    const next = () => {
      const v = argv[i + 1];
      if (v === undefined) usageFail("showcase", `flag ${a} needs a value`, HELP);
      i += 1;
      return v;
    };
    if (a === "--clip") opts.clip = next();
    else if (a === "--set") opts.set = next();
    else if (a === "--manifest") opts.manifest = next();
    else if (a === "--fps") opts.fps = next();
    else if (a === "--width") opts.width = next();
    else if (a === "--out") opts.out = next();
    else if (a === "--check-only") opts.checkOnly = true;
    else if (a === "--json") opts.json = true;
    else if (a === "--allow-unhashed") opts.allowUnhashed = true;
    else if (a === "--help" || a === "-h") {
      process.stdout.write(HELP);
      process.exit(0);
    } else if (a.startsWith("-")) usageFail("showcase", `unknown flag ${a}`, HELP);
    else positionals.push(a);
  }
  if (!opts.clip && positionals.length) opts.clip = positionals[0];
  if (positionals.length > 1) usageFail("showcase", "at most one positional <clipDir>", HELP);
  return opts;
}

// Thin wrapper so tests could stub one seam; ffmpeg stays an external binary.
function runTool(cmd, args, timeoutMs = 600_000) {
  const r = spawnSync(cmd, args, { encoding: "utf8", timeout: timeoutMs });
  return {
    ok: !r.error && r.status === 0,
    status: r.status,
    error: r.error ? String(r.error) : "",
    out: `${r.stdout ?? ""}${r.stderr ?? ""}`.trim(),
  };
}

function ffmpegHint() {
  return [
    "ffmpeg not found on PATH and no MP4/GIF can be cut without it.",
    "Install it, then re-run this command:",
    "  Windows: winget install Gyan.FFmpeg  (or: choco install ffmpeg)",
    "  macOS:   brew install ffmpeg",
    "  Linux:   sudo apt install ffmpeg  (or your distro equivalent)",
    "See https://ffmpeg.org/download.html — engine crates never bundle an encoder.",
  ].join("\n");
}

function checkFfmpeg() {
  const r = runTool("ffmpeg", ["-version"], 30_000);
  if (!r.ok) fail("showcase", `${ffmpegHint()}\n(probe: ${r.error || r.out || `exit ${r.status}`})`);
  return r.out.split("\n")[0];
}

function readManifest(manifestPath) {
  if (!existsSync(manifestPath)) {
    fail("showcase", `manifest not found: ${manifestPath} (SHOW1-cap writes manifest.json next to the PNGs)`);
  }
  let raw;
  try {
    raw = readFileSync(manifestPath, "utf8");
  } catch (e) {
    fail("showcase", `cannot read manifest ${manifestPath}: ${e.message}`);
  }
  try {
    return JSON.parse(raw);
  } catch (e) {
    fail("showcase", `manifest is not valid JSON: ${manifestPath}: ${e.message}`);
  }
}

// Required fields plus tolerated aliases; per-file entries verified later.
function validateManifestFields(m) {
  const problems = [];
  if (m === null || typeof m !== "object" || Array.isArray(m)) {
    return { problems: ["manifest root must be a JSON object"], sha: undefined, fps: undefined };
  }
  if (!Number.isInteger(m.seed) || m.seed < 0) problems.push("seed must be an unsigned integer");
  if (typeof m.tape !== "string" || !m.tape.trim()) problems.push("tape must be a non-empty string");
  if (!Array.isArray(m.rows) || m.rows.length === 0) {
    problems.push("rows must be a non-empty array of board row IDs (e.g. [\"WIN1\",\"CR1\"])");
  } else {
    for (const r of m.rows) {
      if (typeof r !== "string" || !ROW_RE.test(r.trim())) {
        problems.push(`rows has a bad board row ID: ${JSON.stringify(r)} (want e.g. WIN1, GX10, SHOW1)`);
        break;
      }
    }
  }
  const sha = m.sha ?? m.buildSha ?? m.build_sha ?? m.buildSHA;
  if (typeof sha !== "string" || !HEX_RE.test(sha.trim())) {
    problems.push("sha (build SHA hex, 7..128 chars) is missing or malformed — refusing a tampered manifest");
  }
  const fps = m.fps ?? m.frameRate ?? m.framerate;
  if (fps !== undefined && (!Number.isFinite(Number(fps)) || Number(fps) < 1 || Number(fps) > 120)) {
    problems.push(`fps must be 1..120, got ${JSON.stringify(fps)}`);
  }
  return { problems, sha: typeof sha === "string" ? sha.trim() : sha, fps };
}

// Renderer key filed by capture legs (presenter-twin, headless-synth,
// staged-cpu, game-camera-feed, crates-render): surfaced for the J2 "which
// renderer drew it" step, never gated, so old manifests without it still pass.
function manifestRenderer(m) {
  if (Array.isArray(m)) {
    const keys = m.map((e) => e?.renderer).filter((r) => typeof r === "string" && r.trim());
    if (keys.length === 0) return null;
    const first = keys[0].trim();
    return keys.every((r) => r.trim() === first) ? first : keys.map((r) => r.trim()).join(",");
  }
  if (m && typeof m === "object" && typeof m.renderer === "string" && m.renderer.trim()) {
    return m.renderer.trim();
  }
  return null;
}

function listPngs(clipDir) {
  let entries;
  try {
    entries = readdirSync(clipDir);
  } catch (e) {
    fail("showcase", `cannot read clip dir ${clipDir}: ${e.message}`);
  }
  // Sorted order is the concat demuxer order; keep it deterministic.
  const pngs = entries.filter((n) => n.toLowerCase().endsWith(".png")).sort();
  if (pngs.length === 0) fail("showcase", `no PNG frames in ${clipDir} (want frame*.png from SHOW1-cap)`);
  return pngs;
}

function pngSize(pngPath) {
  let buf;
  try {
    buf = readFileSync(pngPath);
  } catch (e) {
    fail("showcase", `cannot read frame ${pngPath}: ${e.message}`);
  }
  if (buf.length === 0 || !buf.subarray(0, 8).equals(PNG_MAGIC)) {
    fail("showcase", `not a PNG file (bad magic): ${pngPath} — refusing a corrupt capture`);
  }
  // IHDR width/height are big-endian u32 at bytes 16/20; short files fail.
  if (buf.length < 33 || buf.toString("ascii", 12, 16) !== "IHDR") {
    fail("showcase", `PNG header truncated (no IHDR): ${pngPath} — refusing a corrupt capture`);
  }
  const width = buf.readUInt32BE(16);
  const height = buf.readUInt32BE(20);
  if (!width || !height) fail("showcase", `PNG has zero width/height: ${pngPath}`);
  return { bytes: buf.length, width, height, data: buf };
}

// Manifest frames/files entries, when present, must match disk exactly:
// missing, extra or byte-divergent frames fail loudly (tamper evidence).
// A manifest with no frames/files/count coverage is frameless and is refused
// unless --allow-unhashed is given, so tampered PNGs fail loudly on both paths.
// String entries and hash-less objects carry no per-file digest, so byte
// tamper is invisible: they fail unless --allow-unhashed opts in. Legit
// writers (SHOW1-cap, game capture) always emit hashed objects, so the
// default refusal does not break real SHOW clips (surveyed wave22 duel,
// siege10s and the tool fixture: all hashed objects, zero string entries).
function crossCheckFrames(manifest, pngNames, pngByName, allowUnhashed = false) {
  const entry = manifest.frames ?? manifest.files;
  const countAlias = manifest.frameCount ?? manifest.count ?? manifest.n;
  const problems = [];
  if (entry === undefined) {
    if (countAlias !== undefined) {
      if (Number(countAlias) !== pngNames.length) {
        problems.push(`manifest frame count ${countAlias} != ${pngNames.length} PNGs on disk`);
      }
      return problems;
    }
    if (allowUnhashed) return problems;
    problems.push(
      "manifest has no frames/files/count coverage — refusing an unhashed capture (re-run with --allow-unhashed to accept frameless manifests explicitly)",
    );
    return problems;
  }
  if (Array.isArray(entry)) {
    const listed = entry.map((e) => (typeof e === "string" ? e : e?.file)).filter(Boolean);
    const onDisk = new Set(pngNames);
    const inManifest = new Set(listed);
    for (const f of listed) {
      if (!onDisk.has(f)) problems.push(`manifest lists missing frame: ${f}`);
    }
    for (const f of pngNames) {
      if (!inManifest.has(f)) problems.push(`frame on disk but absent from manifest: ${f}`);
    }
    for (const e of entry) {
      if (typeof e === "string") {
        if (!allowUnhashed) {
          problems.push(`frame entry without sha256: ${JSON.stringify(e)} — refusing an unhashed manifest (re-run with --allow-unhashed to accept explicitly unhashed captures)`);
        }
        continue;
      }
      if (typeof e !== "object" || e === null || typeof e.file !== "string") {
        problems.push(`frames entry must be a filename string or {file, sha256} object, got ${JSON.stringify(e)}`);
        continue;
      }
      const hit = pngByName.get(e.file);
      if (!hit) continue;
      if (typeof e.sha256 !== "string" || !SHA256_RE.test(e.sha256.trim())) {
        if (!allowUnhashed) {
          problems.push(`frame entry without sha256: ${JSON.stringify(e.file)} — refusing an unhashed manifest (re-run with --allow-unhashed to accept explicitly unhashed captures)`);
        }
        if (e.bytes !== undefined && Number(e.bytes) !== hit.bytes) {
          problems.push(`frame bytes differ from manifest size: ${e.file} (disk ${hit.bytes}, manifest ${e.bytes})`);
        }
        continue;
      }
      if (e.sha256.toLowerCase() !== sha256(hit.data)) {
        problems.push(`frame bytes differ from manifest sha256: ${e.file} — refusing a tampered capture`);
      }
      if (e.bytes !== undefined && Number(e.bytes) !== hit.bytes) {
        problems.push(`frame bytes differ from manifest size: ${e.file} (disk ${hit.bytes}, manifest ${e.bytes})`);
      }
    }
  } else if (Number.isInteger(Number(entry))) {
    if (Number(entry) !== pngNames.length) {
      problems.push(`manifest frame count ${entry} != ${pngNames.length} PNGs on disk`);
    }
  } else {
    problems.push("frames must be a file-entry array or an integer count");
  }
  if (countAlias !== undefined && Number(countAlias) !== pngNames.length) {
    problems.push(`manifest frame count ${countAlias} != ${pngNames.length} PNGs on disk`);
  }
  return problems;
}

// Recursive PNG census for a showcase set dir (target/showcase/<date>):
// counts *.png (any case) under every subdir. Synchronous; showcase sets
// hold hundreds of frames, never millions.
function countSetPngs(dir) {
  let count = 0;
  let entries;
  try {
    entries = readdirSync(dir, { withFileTypes: true });
  } catch (e) {
    fail("showcase", `cannot read showcase set dir ${dir}: ${e.message}`);
  }
  for (const e of entries) {
    const full = join(dir, e.name);
    if (e.isDirectory()) count += countSetPngs(full);
    else if (e.isFile() && e.name.toLowerCase().endsWith(".png")) count += 1;
  }
  return count;
}

// Set check: fail closed when a showcase set ships 0 PNGs anywhere under it.
// Names the set dir and basename so the empty drop never passes silently.
function checkShowcaseSet(setDir, asJson) {
  if (!existsSync(setDir) || !statSync(setDir).isDirectory()) {
    fail("showcase", `showcase set not found: ${setDir}`);
  }
  const pngs = countSetPngs(setDir);
  if (pngs === 0) {
    fail("showcase", `showcase set has 0 PNGs: ${setDir} (set ${basename(setDir)} ships no frame*.png from SHOW1-cap)`);
  }
  if (asJson) process.stdout.write(`${JSON.stringify({ ok: true, set: setDir, pngs })}\n`);
  else process.stdout.write(`showcase set OK: ${pngs} PNGs in ${setDir}\n`);
}

function writeConcatList(pngPaths) {
  const dir = mkdtempSync(join(tmpdir(), "e2040-showcase-"));
  const listPath = join(dir, "list.txt");
  // Concat demuxer: one "file '<path>'" line per frame; quote-escape for safety.
  const text = `${pngPaths.map((p) => `file '${p.replace(/'/g, "'\\''")}'`).join("\n")}\n`;
  writeFileSync(listPath, text, "utf8");
  return { dir, listPath };
}

function main() {
  const opts = parseArgs(process.argv.slice(2));
  if (opts.set && opts.clip) usageFail("showcase", "use one of --clip <clipDir> or --set <setDir>, not both", HELP);
  if (opts.set) {
    checkShowcaseSet(resolve(opts.set), opts.json);
    return;
  }
  if (!opts.clip) usageFail("showcase", "give a clip dir: --clip target/showcase/<date>/<clip>", HELP);
  const clipDir = resolve(opts.clip);
  if (!existsSync(clipDir) || !statSync(clipDir).isDirectory()) {
    fail("showcase", `clip dir not found: ${clipDir}`);
  }
  const manifestPath = opts.manifest ? resolve(opts.manifest) : join(clipDir, "manifest.json");
  const manifest = readManifest(manifestPath);
  const { problems: fieldProblems, sha, fps: manifestFps } = validateManifestFields(manifest);

  const fpsRaw = opts.fps ?? manifestFps ?? 30;
  const fps = Number(fpsRaw);
  if (!Number.isFinite(fps) || fps < 1 || fps > 120) fail("showcase", `--fps must be 1..120, got ${JSON.stringify(fpsRaw)}`);
  const width = Number(opts.width);
  if (!Number.isInteger(width) || width < 64 || width > 8192) fail("showcase", `--width must be 64..8192, got ${opts.width}`);

  const pngNames = listPngs(clipDir);
  const pngByName = new Map();
  let frameW = 0;
  let frameH = 0;
  for (const n of pngNames) {
    const info = pngSize(join(clipDir, n));
    if (!frameW) {
      frameW = info.width;
      frameH = info.height;
    } else if (info.width !== frameW || info.height !== frameH) {
      fail("showcase", `frame size drift: ${n} is ${info.width}x${info.height} but first frame is ${frameW}x${frameH}`);
    }
    pngByName.set(n, info);
  }
  const frameProblems = crossCheckFrames(manifest, pngNames, pngByName, opts.allowUnhashed);
  const allProblems = [...fieldProblems, ...frameProblems];
  if (allProblems.length) {
    fail("showcase", `manifest mismatch — refusing to stitch:\n  - ${allProblems.join("\n  - ")}`);
  }

  const clipName = manifest.clip && typeof manifest.clip === "string" && manifest.clip.trim()
    ? manifest.clip.trim()
    : basename(clipDir);
  if (!CLIP_RE.test(clipName)) {
    fail("showcase", `refusing unsafe manifest.clip ${JSON.stringify(manifest.clip ?? clipName)} — clip must be a bare basename (letters, digits, ., _, -; no ../ or separators)`);
  }
  const outDir = opts.out ? resolve(opts.out) : clipDir;
  if (!opts.checkOnly) {
    mkdirSync(outDir, { recursive: true });
  }
  const mp4Path = join(outDir, `${clipName}.mp4`);
  const gifPath = join(outDir, `${clipName}.gif`);

  const renderer = manifestRenderer(manifest);
  const summary = {
    clip: clipName,
    clipDir: isAbsolute(opts.clip) ? clipDir : opts.clip,
    manifest: manifestPath,
    seed: manifest.seed,
    tape: manifest.tape,
    rows: manifest.rows,
    sha,
    renderer,
    fps,
    frames: pngNames.length,
    frameSize: `${frameW}x${frameH}`,
    allowUnhashed: opts.allowUnhashed,
    outputs: opts.checkOnly ? [] : [mp4Path, gifPath],
  };

  if (opts.checkOnly) {
    const text = `showcase OK: ${pngNames.length} frames ${frameW}x${frameH} @${fps}fps, manifest ${manifestPath} valid (seed ${manifest.seed}, rows ${manifest.rows.join(",")}, renderer ${renderer ?? "unknown"})`;
    if (opts.json) process.stdout.write(`${JSON.stringify({ ok: true, ...summary })}\n`);
    else process.stdout.write(`${text}\n`);
    return;
  }

  const ffmpegLine = checkFfmpeg();
  const pngPaths = pngNames.map((n) => join(clipDir, n));
  const { dir: tmpListDir, listPath } = writeConcatList(pngPaths);
  try {
    const mp4 = runTool("ffmpeg", [
      "-y", "-f", "concat", "-safe", "0", "-r", String(fps), "-i", listPath,
      "-c:v", "libx264", "-pix_fmt", "yuv420p", "-crf", "18",
      "-movflags", "+faststart", mp4Path,
    ]);
    if (!mp4.ok) {
      fail("showcase", `ffmpeg MP4 stitch failed (exit ${mp4.status}):\n${mp4.out.slice(-4000)}\ninputs kept; nothing deleted`);
    }
    const gif = runTool("ffmpeg", [
      "-y", "-f", "concat", "-safe", "0", "-r", String(fps), "-i", listPath,
      "-vf", `fps=${fps},scale=${width}:-1:flags=lanczos`, gifPath,
    ]);
    if (!gif.ok) {
      fail("showcase", `ffmpeg GIF stitch failed (exit ${gif.status}):\n${gif.out.slice(-4000)}\nMP4 kept at ${mp4Path}; inputs kept; nothing deleted`);
    }
  } finally {
    // Only the temp concat list goes away; PNGs + manifest are never touched.
    rmSync(tmpListDir, { recursive: true, force: true });
  }
  summary.mp4Bytes = statSync(mp4Path).size;
  summary.gifBytes = statSync(gifPath).size;
  summary.ffmpeg = ffmpegLine;
  if (opts.json) process.stdout.write(`${JSON.stringify({ ok: true, ...summary })}\n`);
  else {
    process.stdout.write(
      `showcase OK: ${pngNames.length} frames ${frameW}x${frameH} @${fps}fps → ${mp4Path} (${summary.mp4Bytes} B) + ${gifPath} (${summary.gifBytes} B})\n`,
    );
  }
}

main();
