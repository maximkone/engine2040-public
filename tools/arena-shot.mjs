// arena-shot: the daily arena shot gate (VS9). One command runs the arena
// capture at the pinned camera path, then refuses placeholder frames.
//
// Run:  node tools/arena-shot.mjs [--dir target/vs9] [--no-build] [--json]
//       node tools/arena-shot.mjs --check <dir>    gate an existing capture
//       node tools/arena-shot.mjs --self-check     rules on fixtures, no GPU
//
// The gate refuses, by name:
//   cube-sha      a frame whose sha256 is the repo's pinned cube frame
//                 (docs/evidence/arena-cube-frame.json)
//   noise-64x48   a 64x48 frame, the headless synth placeholder
//   empty-box     a drawn fighter with no world box or no on-screen box, or a
//                 captured frame whose fighters all miss the picture
//   magenta       the error-mesh colour over 0.5% of a frame
//   frames        not exactly 3 captured frames
//   manifest      the manifest, a frame file or the view brief is missing or
//                 the PNG bytes disagree with the manifest
//   run           arena.exe failed, timed out or printed no capture receipt
//
// Beside the frames it writes mask PNGs (each drawn body's clipped screen box
// from --view-brief, gray 51*(slot+1)), masks.json (mask rows) and shot.json
// (the daily receipt). The masks are box silhouettes from the body bounds, not
// a per-pixel alpha cutout; the brief names them that way.
//
// Exit codes: 0 pass, 1 refused, 2 usage or tool failure.

import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { existsSync, mkdirSync, readdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join, relative } from "node:path";
import { crc32, deflateSync, inflateSync } from "node:zlib";
import { fileURLToPath } from "node:url";
import { binStatus } from "./bins.mjs";
import { decodePng } from "./fighter-count.mjs";

const ROOT = dirname(dirname(fileURLToPath(import.meta.url)));
const DEFAULT_DIR = join("target", "vs9");
const CUBE_EVIDENCE = join("docs", "evidence", "arena-cube-frame.json");
/** The exact capture the row names: 2 fighters on the fixed arena600 orbit. */
export const CAPTURE_ARGS = Object.freeze(["--scene", "arena", "--fighters", "2", "--camera-path", "arena600", "--frames", "120", "--exit"]);
export const CAPTURE_EVERY = 40;
export const FRAME_COUNT = 3;
export const PINNED_SEED = 2040;
export const NOISE_SIZE = Object.freeze([64, 48]);
export const MAGENTA_MAX_SHARE = 0.005;
export const PNG_SIG = Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]);
const RUN_MS = 5 * 60 * 1000;
const BUILD_MS = 15 * 60 * 1000;

const sha256 = (buf) => createHash("sha256").update(buf).digest("hex");
const rel = (path) => relative(ROOT, path).split("\\").join("/");

/** True for the error-mesh magenta a missing asset draws. Inputs 0-255. */
export function isMagenta(r, g, b) {
  return r >= 160 && b >= 160 && g <= 80 && Math.abs(r - b) <= 80;
}

/** Share of pixels that read magenta, 0..1, over decoded RGBA bytes. */
export function magentaShare(data) {
  let hit = 0;
  for (let i = 0; i < data.length; i += 4) if (isMagenta(data[i], data[i + 1], data[i + 2])) hit += 1;
  return hit / (data.length / 4);
}

/** The pinned cube frame digest, read from its one evidence row. */
export function readCubeEvidence(root = ROOT) {
  const path = join(root, CUBE_EVIDENCE);
  const row = JSON.parse(readFileSync(path, "utf8"));
  if (typeof row.sha256 !== "string" || !/^[0-9a-f]{64}$/.test(row.sha256)) throw new Error(`${rel(path)}: no sha256`);
  return { sha256: row.sha256, path };
}

/** Placeholder patterns a frame digest and shape carry. Pure. */
export function placeholderPatterns(frame, cubeSha) {
  const out = [];
  if (cubeSha && frame.sha256 === cubeSha) out.push("cube-sha");
  if (frame.width === NOISE_SIZE[0] && frame.height === NOISE_SIZE[1]) out.push("noise-64x48");
  return out;
}

/** Why a drawn body's box is empty, or null when it holds geometry. Pure. */
export function bodyBoxGap(body) {
  const finite = (v) => typeof v === "number" && Number.isFinite(v);
  const hasBox =
    Array.isArray(body?.min) &&
    Array.isArray(body?.max) &&
    body.min.length === 3 &&
    body.max.length === 3 &&
    body.min.every(finite) &&
    body.max.every(finite) &&
    body.min.every((v, i) => v <= body.max[i]);
  if (!hasBox) return `slot ${body?.slot}: no world box`;
  if (body.nonfinite) return `slot ${body.slot}: ${body.nonfinite} nonfinite parts`;
  if (body.error_parts) return `slot ${body.slot}: ${body.error_parts} error parts`;
  return null;
}

/** A body's clipped screen rect as pixels `[x0,y0,x1,y1]`, or null. Pure. */
export function screenRectPx(screen, width, height) {
  if (!Array.isArray(screen) || screen.length !== 4 || screen.some((v) => typeof v !== "number" || !Number.isFinite(v))) return null;
  const clamp = (v, hi) => Math.min(hi, Math.max(0, v));
  const x0 = clamp(Math.round((screen[0] + 1) * 0.5 * width), width);
  const x1 = clamp(Math.round((screen[2] + 1) * 0.5 * width), width);
  // The brief's y is up in NDC, a PNG's y grows down.
  const y0 = clamp(Math.round((1 - screen[3]) * 0.5 * height), height);
  const y1 = clamp(Math.round((1 - screen[1]) * 0.5 * height), height);
  if (x1 - x0 < 1 || y1 - y0 < 1) return null;
  return [x0, y0, x1, y1];
}

/** One frame's silhouette box mask plus the boxes it took. Pure. */
export function maskFor(bodies, width, height) {
  const gray = new Uint8Array(width * height);
  const boxes = [];
  for (const body of bodies) {
    const rect = screenRectPx(body.screen, width, height);
    if (!rect) continue;
    const value = Math.min(255, 51 * ((body.slot | 0) + 1));
    const [x0, y0, x1, y1] = rect;
    for (let y = y0; y < y1; y += 1) {
      for (let x = x0; x < x1; x += 1) {
        if (gray[y * width + x] < value) gray[y * width + x] = value;
      }
    }
    boxes.push({ slot: body.slot, rect, cover: Number(body.cover) || 0 });
  }
  return { gray, boxes };
}

function pngChunk(type, data) {
  const head = Buffer.alloc(8);
  head.writeUInt32BE(data.length, 0);
  head.write(type, 4, "ascii");
  const crc = Buffer.alloc(4);
  crc.writeUInt32BE(crc32(Buffer.concat([head.subarray(4), data])) >>> 0, 0);
  return Buffer.concat([head, data, crc]);
}

/** An 8-bit grayscale PNG with filter 0 on every row. */
export function encodeGrayPng(width, height, gray) {
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(width, 0);
  ihdr.writeUInt32BE(height, 4);
  ihdr[8] = 8;
  ihdr[9] = 0;
  const raw = Buffer.alloc(height * (width + 1));
  for (let y = 0; y < height; y += 1) raw.set(gray.subarray(y * width, (y + 1) * width), y * (width + 1) + 1);
  return Buffer.concat([PNG_SIG, pngChunk("IHDR", ihdr), pngChunk("IDAT", deflateSync(raw)), pngChunk("IEND", Buffer.alloc(0))]);
}

/** Read back an 8-bit grayscale PNG (the mask format this tool writes). */
export function decodeGrayPng(buf) {
  if (!buf.subarray(0, 8).equals(PNG_SIG)) throw new Error("not a PNG");
  let width = 0;
  let height = 0;
  let color = 255;
  const idat = [];
  for (let pos = 8; pos < buf.length; ) {
    const len = buf.readUInt32BE(pos);
    const type = buf.toString("ascii", pos + 4, pos + 8);
    const data = buf.subarray(pos + 8, pos + 8 + len);
    if (type === "IHDR") {
      width = data.readUInt32BE(0);
      height = data.readUInt32BE(4);
      color = data[9];
    } else if (type === "IDAT") idat.push(data);
    else if (type === "IEND") break;
    pos += 12 + len;
  }
  if (color !== 0) throw new Error("not grayscale");
  const raw = inflateSync(Buffer.concat(idat));
  if (raw.length !== height * (width + 1)) throw new Error("bad mask scanlines");
  const gray = new Uint8Array(width * height);
  for (let y = 0; y < height; y += 1) {
    if (raw[y * (width + 1)] !== 0) throw new Error("mask row filter");
    gray.set(raw.subarray(y * (width + 1) + 1, (y + 1) * (width + 1)), y * width);
  }
  return { width, height, gray };
}

/** One JSON line per presented frame, keyed by frame index. */
export function parseView(text) {
  const byFrame = new Map();
  for (const line of text.split("\n")) {
    if (!line.trim()) continue;
    try {
      const row = JSON.parse(line);
      if (Number.isInteger(row.frame)) byFrame.set(row.frame, row);
    } catch {
      // A torn tail line is skipped; the gate names the frames it cannot read.
    }
  }
  return byFrame;
}

/** Every gap one capture frame carries. Pure over the decoded frame and brief. */
export function frameGaps(frame, cubeSha) {
  const gaps = [];
  for (const pattern of placeholderPatterns(frame, cubeSha)) gaps.push(`${pattern}(${frame.file})`);
  if (frame.magentaShare > MAGENTA_MAX_SHARE) gaps.push(`magenta(${frame.file} ${(frame.magentaShare * 100).toFixed(3)}%)`);
  if (!frame.brief) {
    gaps.push(`manifest(no view row for presented frame ${frame.presented})`);
    return gaps;
  }
  const bodies = Array.isArray(frame.brief.bodies) ? frame.brief.bodies : [];
  if (bodies.length === 0) gaps.push(`empty-box(${frame.file}: no drawn body)`);
  for (const body of bodies) {
    const gap = bodyBoxGap(body);
    if (gap) gaps.push(`empty-box(${frame.file}: ${gap})`);
  }
  const boxes = bodies.filter((body) => screenRectPx(body.screen, frame.width, frame.height));
  if (boxes.length === 0) gaps.push(`empty-box(${frame.file}: no fighter on screen)`);
  return gaps;
}

/** Write the masks, masks.json and shot.json; return the mask rows. */
function writeMasks(dir, frames, cubeSha) {
  const rows = [];
  for (const frame of frames) {
    if (!frame.decoded) continue;
    const { gray, boxes } = maskFor(frame.brief?.bodies ?? [], frame.width, frame.height);
    const name = frame.file.replace(/\.png$/, "_mask.png");
    const bytes = encodeGrayPng(frame.width, frame.height, gray);
    const path = join(dir, name);
    writeFileSync(path, bytes);
    rows.push({
      file: name,
      frame: frame.presented,
      width: frame.width,
      height: frame.height,
      bytes: bytes.length,
      sha256: sha256(bytes),
      boxes,
    });
  }
  writeFileSync(join(dir, "masks.json"), `${JSON.stringify({ schema: 1, masks: rows, cube_sha: cubeSha ?? null }, null, 0)}\n`);
  return rows;
}

/**
 * Gate one sealed capture directory: manifest, three PNGs, view brief.
 * `--check` runs it on an existing dir; the default run seals one first.
 */
export function checkCapture(dir, cubeSha) {
  const gaps = [];
  const manifestPath = join(dir, "manifest.json");
  if (!existsSync(manifestPath)) return { gaps: [`manifest(${rel(manifestPath)} missing)`], frames: [], masks: [], magentaMax: 0 };
  const manifest = JSON.parse(readFileSync(manifestPath, "utf8"));
  const rows = Array.isArray(manifest.frames) ? manifest.frames : [];
  if (manifest.seed !== PINNED_SEED) gaps.push(`frames(seed ${manifest.seed} != ${PINNED_SEED})`);
  if (manifest.capture_every !== CAPTURE_EVERY) gaps.push(`frames(capture_every ${manifest.capture_every} != ${CAPTURE_EVERY})`);
  if (rows.length !== FRAME_COUNT) gaps.push(`frames(${rows.length} captured != ${FRAME_COUNT})`);
  const viewPath = join(dir, "view.jsonl");
  const view = existsSync(viewPath) ? parseView(readFileSync(viewPath, "utf8")) : new Map();
  if (!existsSync(viewPath)) gaps.push(`manifest(${rel(viewPath)} missing)`);
  if (!existsSync(join(dir, "per_pass_ms.json"))) gaps.push(`manifest(${rel(join(dir, "per_pass_ms.json"))} missing)`);
  const onDisk = readdirSync(dir).filter((name) => /^frame_\d{6}\.png$/.test(name)).sort();
  const named = new Set(rows.map((row) => row.file));
  for (const name of onDisk) if (!named.has(name)) gaps.push(`frames(${name} is not in the manifest)`);
  const frames = [];
  rows.forEach((row, index) => {
    const path = join(dir, row.file);
    if (!existsSync(path)) {
      gaps.push(`manifest(${row.file} missing)`);
      frames.push({ file: row.file, presented: index * CAPTURE_EVERY, decoded: false });
      return;
    }
    const buf = readFileSync(path);
    const digest = sha256(buf);
    const frame = {
      file: row.file,
      tick: row.tick,
      presented: index * CAPTURE_EVERY,
      width: row.width,
      height: row.height,
      sha256: digest,
      decoded: false,
    };
    if (digest !== row.sha256) gaps.push(`manifest(${row.file} sha256 ${digest.slice(0, 12)} != manifest ${String(row.sha256).slice(0, 12)})`);
    try {
      const png = decodePng(buf, row.file);
      frame.width = png.width;
      frame.height = png.height;
      frame.magentaShare = magentaShare(png.data);
      frame.decoded = true;
    } catch (err) {
      gaps.push(`manifest(${row.file}: ${err.message})`);
    }
    frame.brief = view.get(frame.presented);
    gaps.push(...frameGaps(frame, cubeSha));
    frames.push(frame);
  });
  const masks = frames.some((frame) => frame.decoded) ? writeMasks(dir, frames, cubeSha) : [];
  const magentaMax = frames.reduce((max, frame) => Math.max(max, frame.magentaShare ?? 0), 0);
  return { gaps, frames, masks, manifest, magentaMax };
}

function buildArena() {
  const row = binStatus(ROOT).find((bin) => bin.exe === "arena");
  if (row?.state === "FRESH") return { built: false, why: `fresh (built ${row.built})` };
  const why = row ? `${row.state}${row.commits ? ` (${row.commits} code commits since ${row.built})` : ""}` : "missing";
  const r = spawnSync("cargo", ["build", "--release", "-p", "engine-game", "--features", "app", "--bin", "arena"], {
    cwd: ROOT,
    encoding: "utf8",
    timeout: BUILD_MS,
    maxBuffer: 64 * 1024 * 1024,
    env: { ...process.env, CARGO_BUILD_JOBS: "10", RUSTC_WRAPPER: "" },
    windowsHide: true,
  });
  if (r.error || r.status !== 0) {
    const tail = `${r.stdout ?? ""}${r.stderr ?? ""}`.trim().split("\n").slice(-6).join("\n");
    throw new Error(`arena build failed (${why}): ${tail || r.error?.message || `exit ${r.status}`}`);
  }
  return { built: true, why };
}

function gitShortSha() {
  const r = spawnSync("git", ["rev-parse", "--short", "HEAD"], { cwd: ROOT, encoding: "utf8" });
  const sha = (r.stdout ?? "").trim();
  if (r.status !== 0 || !/^[0-9a-f]{7,40}$/.test(sha)) throw new Error("git rev-parse --short HEAD gave no build sha");
  return sha;
}

function runCapture(dir, sha) {
  mkdirSync(dir, { recursive: true });
  const exe = join(ROOT, "target", "release", "arena.exe");
  if (!existsSync(exe)) throw new Error(`${rel(exe)} missing after the build`);
  const args = [
    ...CAPTURE_ARGS,
    "--capture-dir", dir,
    "--capture-every", String(CAPTURE_EVERY),
    "--sha", sha,
    "--rows", "VS9",
    "--view-brief", join(dir, "view.jsonl"),
  ];
  const r = spawnSync(exe, args, { cwd: ROOT, encoding: "utf8", timeout: RUN_MS, maxBuffer: 64 * 1024 * 1024, windowsHide: true });
  const out = `${r.stdout ?? ""}\n${r.stderr ?? ""}`;
  if (r.error?.code === "ETIMEDOUT" || r.signal) return { ok: false, why: `arena.exe did not exit within ${RUN_MS / 1000} s`, out };
  if (r.status !== 0) return { ok: false, why: `arena.exe exit ${r.status}`, out };
  if (!/arena capture: frames=\d+/.test(out)) return { ok: false, why: "arena.exe printed no capture receipt", out };
  return { ok: true, args, out };
}

export function receipt({ verdict, sha, dir, result }) {
  const frames = result.frames.length;
  const magenta = frames ? (result.magentaMax * 100).toFixed(3) : "0.000";
  if (verdict === "PASS") return `arena-shot: PASS build=${sha} frames=${frames} masks=${result.masks.length} magenta_max=${magenta}% dir=${rel(dir)}`;
  const shown = result.gaps.slice(0, 6).join(" ");
  const more = result.gaps.length > 6 ? ` +${result.gaps.length - 6} more` : "";
  return `arena-shot: FAIL build=${sha} frames=${frames} masks=${result.masks.length} magenta_max=${magenta}% gaps: ${shown}${more}`;
}

/** Rules on fixtures under `<dir>/selfcheck`; no GPU and no arena.exe. */
export function selfCheck(dir) {
  const checks = [];
  const check = (name, ok, why = "fixture rule failed") => checks.push({ name, ok: Boolean(ok), why: ok ? null : why });
  const cube = readCubeEvidence();
  check("cube evidence digest", /^[0-9a-f]{64}$/.test(cube.sha256), "digest missing");
  check("cube frame refused", placeholderPatterns({ sha256: cube.sha256, width: 1280, height: 720 }, cube.sha256).join() === "cube-sha");
  check("64x48 frame refused", placeholderPatterns({ sha256: "00", width: 64, height: 48 }, cube.sha256).join() === "noise-64x48");
  check("real frame accepted", placeholderPatterns({ sha256: "11", width: 1280, height: 720 }, cube.sha256).length === 0);
  check("box with geometry holds", bodyBoxGap({ slot: 0, min: [0, 0, 0], max: [1, 1, 1], nonfinite: 0, error_parts: 0 }) === null);
  check("inverted box refused", bodyBoxGap({ slot: 0, min: [1, 0, 0], max: [0, 1, 1] }) !== null);
  check("nonfinite box refused", bodyBoxGap({ slot: 1, min: [0, 0, 0], max: [1, Number.NaN, 1] }) !== null);
  check("rect maps NDC to pixels", screenRectPx([-0.5, -0.5, 0.5, 0.5], 100, 100).join() === "25,25,75,75");
  check("rect under one pixel refused", screenRectPx([0.9, 0.9, 0.905, 0.905], 100, 100) === null);
  const rgba = Buffer.alloc(100 * 100 * 4, 0);
  for (let i = 0; i < 60; i += 1) {
    rgba[i * 4] = 220;
    rgba[i * 4 + 1] = 20;
    rgba[i * 4 + 2] = 220;
    rgba[i * 4 + 3] = 255;
  }
  const share = magentaShare(rgba);
  check("magenta share counted", Math.abs(share - 0.006) < 1e-9 && share > MAGENTA_MAX_SHARE, `share ${share}`);
  check("sand is not magenta", !isMagenta(194, 160, 104));
  const gray = new Uint8Array([0, 51, 102, 255, 0, 0]);
  const back = decodeGrayPng(encodeGrayPng(3, 2, gray));
  check("gray png round trips", back.width === 3 && back.height === 2 && back.gray.every((v, i) => v === gray[i]));
  const masked = maskFor(
    [
      { slot: 0, screen: [-1, 0, 0, 1], cover: 0.25 },
      { slot: 1, screen: [0, -1, 1, 0], cover: 0.25 },
      { slot: 1, screen: null, cover: 0 },
    ],
    100,
    100,
  );
  check("mask marks each slot and skips null", masked.gray[10 * 100 + 10] === 51 && masked.gray[90 * 100 + 90] === 102 && masked.boxes.length === 2);

  // Sealed fixture captures the command path must refuse, one rule each.
  const refused = sealFixture(join(dir, "selfcheck"), 200, 100, { bodies: "offscreen" });
  check("offscreen fighters refused", refused.gaps.some((gap) => gap.startsWith("empty-box(")), refused.gaps.join(" ") || "no gap");
  const noise = sealFixture(join(dir, "selfcheck-noise"), NOISE_SIZE[0], NOISE_SIZE[1], { bodies: "onscreen" });
  check("64x48 capture refused", noise.gaps.some((gap) => gap.startsWith("noise-64x48(")), noise.gaps.join(" ") || "no gap");
  const magenta = sealFixture(join(dir, "selfcheck-magenta"), 200, 100, { bodies: "onscreen", magenta: true });
  check("magenta capture refused", magenta.gaps.some((gap) => gap.startsWith("magenta(")), magenta.gaps.join(" ") || "no gap");
  const good = sealFixture(join(dir, "selfcheck-good"), 200, 100, { bodies: "onscreen" });
  check("plain sand capture accepted", good.gaps.length === 0, good.gaps.join(" ") || "no gap");
  return { ok: checks.every((row) => row.ok), checks };
}

/**
 * Seal one three-frame fixture capture: solid sand frames, a manifest and a
 * view brief. `bodies` picks on-screen boxes or the off-screen pair; `magenta`
 * paints 0.6% of the frame the error-mesh colour.
 */
function sealFixture(fixtureDir, width, height, { bodies, magenta = false }) {
  const solid = Buffer.alloc(width * height * 4);
  for (let i = 0; i < width * height; i += 1) {
    solid[i * 4] = 194;
    solid[i * 4 + 1] = 160;
    solid[i * 4 + 2] = 104;
    solid[i * 4 + 3] = 255;
  }
  if (magenta) {
    const count = Math.ceil(width * height * 0.006);
    for (let i = 0; i < count; i += 1) {
      solid[i * 4] = 220;
      solid[i * 4 + 1] = 20;
      solid[i * 4 + 2] = 220;
    }
  }
  mkdirSync(fixtureDir, { recursive: true });
  const bytes = encodeRgbPng(width, height, solid);
  const rows = [];
  const view = [];
  for (let i = 0; i < FRAME_COUNT; i += 1) {
    const file = `frame_${String(i).padStart(6, "0")}.png`;
    writeFileSync(join(fixtureDir, file), bytes);
    rows.push({ tick: i * CAPTURE_EVERY, file, width, height, sha256: sha256(bytes) });
    const screen = bodies === "onscreen" ? [-0.5, -0.5, 0.5, 0.5] : null;
    view.push({
      frame: i * CAPTURE_EVERY,
      bodies: [
        { slot: 0, min: [0, 0, 0], max: [1, 1, 1], screen, cover: screen ? 0.25 : 0, nonfinite: 0, error_parts: 0 },
        { slot: 1, min: [0, 0, 0], max: [1, 1, 1], screen: null, cover: 0, nonfinite: 0, error_parts: 0 },
      ],
    });
  }
  writeFileSync(
    join(fixtureDir, "manifest.json"),
    `${JSON.stringify({ schema: 1, seed: PINNED_SEED, capture_every: CAPTURE_EVERY, width, height, frames: rows })}\n`,
  );
  writeFileSync(join(fixtureDir, "per_pass_ms.json"), "{}\n");
  writeFileSync(join(fixtureDir, "view.jsonl"), `${view.map((row) => JSON.stringify(row)).join("\n")}\n`);
  return checkCapture(fixtureDir, readCubeEvidence().sha256);
}

/** A real RGB PNG of the given RGBA pixels (fixtures, so masks stay gray). */
function encodeRgbPng(width, height, rgba) {
  const raw = Buffer.alloc(height * (width * 3 + 1));
  for (let y = 0; y < height; y += 1) {
    for (let x = 0; x < width; x += 1) {
      const from = (y * width + x) * 4;
      const to = y * (width * 3 + 1) + 1 + x * 3;
      raw[to] = rgba[from];
      raw[to + 1] = rgba[from + 1];
      raw[to + 2] = rgba[from + 2];
    }
  }
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(width, 0);
  ihdr.writeUInt32BE(height, 4);
  ihdr[8] = 8;
  ihdr[9] = 2;
  return Buffer.concat([PNG_SIG, pngChunk("IHDR", ihdr), pngChunk("IDAT", deflateSync(raw)), pngChunk("IEND", Buffer.alloc(0))]);
}

export function usage() {
  return [
    "usage: node tools/arena-shot.mjs [--dir target/vs9] [--no-build] [--json]",
    "       node tools/arena-shot.mjs --check <dir> [--json]",
    "       node tools/arena-shot.mjs --self-check",
    "  Runs the arena capture (--scene arena --fighters 2 --camera-path arena600",
    "  --frames 120 --exit), then refuses cube-sha, 64x48 noise, empty fighter",
    "  boxes and magenta over 0.5%. Writes 3 PNGs, manifests and masks.",
  ].join("\n");
}

export function main(argv = []) {
  const opts = { dir: DEFAULT_DIR, build: true, json: false, selfCheck: false, check: null, help: false };
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    if (arg === "--dir") {
      const value = argv[i + 1];
      if (!value || value.startsWith("--")) return { code: 2, out: usage() };
      opts.dir = value;
      i += 1;
    } else if (arg === "--check") {
      const value = argv[i + 1];
      if (!value || value.startsWith("--")) return { code: 2, out: "arena-shot: --check needs a capture directory" };
      opts.check = value;
      i += 1;
    } else if (arg === "--no-build") opts.build = false;
    else if (arg === "--json") opts.json = true;
    else if (arg === "--self-check") opts.selfCheck = true;
    else if (arg === "--help" || arg === "-h") return { code: 0, out: usage() };
    else return { code: 2, out: `arena-shot: unknown flag ${arg}\n${usage()}` };
  }
  const dir = join(ROOT, opts.dir);
  try {
    if (opts.selfCheck) {
      const result = selfCheck(dir);
      const passed = result.checks.filter((row) => row.ok).length;
      const failed = result.checks.filter((row) => !row.ok);
      const out = opts.json
        ? JSON.stringify(result)
        : `arena-shot self-check: ${result.ok ? "PASS" : "FAIL"} ${passed}/${result.checks.length} checks${failed.map((row) => `\n  FAIL ${row.name}: ${row.why}`).join("")}`;
      return { code: result.ok ? 0 : 1, out };
    }
    let build = { built: false, why: "not asked (--check)" };
    if (!opts.check) {
      if (!opts.build) {
        const row = binStatus(ROOT).find((bin) => bin.exe === "arena");
        if (row?.state !== "FRESH") return { code: 1, out: `arena-shot: FAIL build(${row?.state ?? "missing"} arena.exe; drop --no-build)` };
        build = { built: false, why: `fresh (built ${row.built})` };
      } else {
        build = buildArena();
      }
    }
    const sha = gitShortSha();
    const target = opts.check ? join(ROOT, opts.check) : dir;
    if (!opts.check) {
      const run = runCapture(target, sha);
      if (!run.ok) return { code: 1, out: `arena-shot: FAIL run(${run.why})\n${run.out.trim().split("\n").slice(-4).join("\n")}` };
    }
    const cube = readCubeEvidence();
    const result = checkCapture(target, cube.sha256);
    const verdict = result.gaps.length === 0 ? "PASS" : "FAIL";
    const buildSha = opts.check ? (result.manifest?.sha ?? sha) : sha;
    const line = receipt({ verdict, sha: buildSha, dir: target, result });
    if (!opts.check) {
      const shot = {
        schema: 1,
        at: new Date().toISOString(),
        build: sha,
        dir: rel(target),
        capture_args: CAPTURE_ARGS,
        capture_every: CAPTURE_EVERY,
        build_state: build.why,
        verdict,
        gaps: result.gaps,
        magenta_max: result.magentaMax ?? 0,
        frames: result.frames.map((frame) => ({ file: frame.file, tick: frame.tick, sha256: frame.sha256, width: frame.width, height: frame.height, magenta: frame.magentaShare ?? null })),
        masks: result.masks.map((mask) => mask.file),
      };
      writeFileSync(join(target, "shot.json"), `${JSON.stringify(shot, null, 0)}\n`);
    }
    if (opts.json) return { code: verdict === "PASS" ? 0 : 1, out: JSON.stringify({ verdict, build: sha, dir: rel(target), gaps: result.gaps, magenta_max: result.magentaMax ?? 0, masks: result.masks.map((mask) => mask.file) }) };
    return { code: verdict === "PASS" ? 0 : 1, out: line };
  } catch (err) {
    return { code: 2, out: `arena-shot: ${err && err.message ? err.message : err}` };
  }
}

if (import.meta.main) {
  const result = main(process.argv.slice(2));
  if (result.out) process.stdout.write(`${result.out}\n`);
  process.exit(result.code);
}
