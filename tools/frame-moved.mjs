// Compare two arena captures and print whether the look moved.
//
// Usage:
//   node tools/frame-moved.mjs --new <png|dir> [--old <png|dir>]
//   node tools/frame-moved.mjs --baseline <png>
//   node tools/frame-moved.mjs --since-last-playtest
//   node tools/frame-moved.mjs --new <png|dir> --json
//
// A dir means its newest frame_*.png (sorted by name). The default --old is
// the cube baseline (docs/evidence/arena-cube-frame.json, else the canonical
// owner-arena frame). --since-last-playtest compares the newest capture under
// target/ with the capture named by the last FRAME: line in team/look.md.
//
// Output: FRAME: <sha8> <w>x<h>, MOVED: pixel <n>% | none, LOOK_STAGE:
// cubes|kit|fighters, ENGINE: pixelmatch|builtin. Exit 0 always on a compared
// pair; exit 2 on a usage error, a missing file or an undecodable PNG.
//
// Donors: mapbox/pixelmatch (ISC) plus pngjs (MIT), installed locally under
// tools/vendor (gitignored). When they are missing the tool still works: it
// decodes 8-bit RGB/RGBA non-interlaced PNGs with node's own zlib and does a
// plain per-pixel RGB compare, noting ENGINE: builtin.
import { createRequire } from "node:module";
import {
  existsSync,
  mkdirSync,
  readdirSync,
  readFileSync,
  statSync,
  writeFileSync,
} from "node:fs";
import { inflateSync } from "node:zlib";
import { createHash } from "node:crypto";
import { basename, dirname, join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const ROOT = dirname(dirname(fileURLToPath(import.meta.url)));
const BASELINE_REL = join("docs", "evidence", "arena-cube-frame.json");
const CANONICAL_BASE_PNG = join("target", "pilot-gx13", "owner-arena", "frame_000015.png");
const NONE_PCT = 0.5;
const PIXELMATCH_THRESHOLD = 0.1;

const baselinePath = () => process.env.FRAME_MOVED_BASELINE ?? join(ROOT, BASELINE_REL);

export function fail(message) {
  process.stderr.write(`frame-moved: ${message}\n`);
  process.exitCode = 2;
  throw new Error(message);
}

function usage() {
  return [
    "usage: node tools/frame-moved.mjs --new <png|dir> [--old <png|dir>] [--json]",
    "       node tools/frame-moved.mjs --baseline <png> [--json]",
    "       node tools/frame-moved.mjs --since-last-playtest [--json]",
  ].join("\n");
}

// --- vendor engine (pixelmatch + pngjs) -------------------------------------
// Local-only donors under tools/vendor (gitignored, never global). Null when
// missing or unloadable; the caller falls back to the builtin engine.
async function vendorEngine() {
  try {
    const vdir = join(ROOT, "tools", "vendor", "node_modules");
    if (!existsSync(join(vdir, "pixelmatch")) || !existsSync(join(vdir, "pngjs"))) return null;
    const req = createRequire(fileURLToPath(import.meta.url));
    const pngjs = req(join(vdir, "pngjs"));
    const pm = req(join(vdir, "pixelmatch"));
    const pixelmatch = pm?.default ?? pm;
    if (typeof pixelmatch !== "function" || !pngjs?.PNG?.sync?.read) throw new Error("vendor shape");
    return { PNG: pngjs.PNG, pixelmatch, name: "pixelmatch" };
  } catch {
    return null;
  }
}

// --- builtin PNG decode (8-bit RGB/RGBA, non-interlaced, via node zlib) -----
const PNG_SIG = [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a];

export function decodePng(buf) {
  if (buf.length < 8 || !PNG_SIG.every((b, i) => buf[i] === b)) throw new Error("not a PNG");
  let off = 8;
  let width = 0;
  let height = 0;
  let depth = 0;
  let color = 0;
  let seenIhdr = false;
  const idat = [];
  while (off + 8 <= buf.length) {
    const len = buf.readUInt32BE(off);
    const type = buf.toString("ascii", off + 4, off + 8);
    const dataAt = off + 8;
    if (dataAt + len + 4 > buf.length) throw new Error("truncated chunk");
    if (type === "IHDR") {
      if (len !== 13) throw new Error("bad IHDR");
      width = buf.readUInt32BE(dataAt);
      height = buf.readUInt32BE(dataAt + 4);
      depth = buf[dataAt + 8];
      color = buf[dataAt + 9];
      if (buf[dataAt + 10] !== 0 || buf[dataAt + 11] !== 0 || buf[dataAt + 12] !== 0) {
        throw new Error("unsupported PNG (compression/filter/interlace)");
      }
      seenIhdr = true;
    } else if (type === "IDAT") {
      idat.push(buf.subarray(dataAt, dataAt + len));
    } else if (type === "IEND") {
      break;
    }
    off = dataAt + len + 4;
  }
  if (!seenIhdr || width === 0 || height === 0) throw new Error("missing IHDR");
  if (depth !== 8 || ![0, 2, 6].includes(color)) throw new Error("only 8-bit gray/RGB/RGBA PNGs");
  const channels = color === 6 ? 4 : color === 2 ? 3 : 1;
  const stride = width * channels;
  let raw;
  try {
    raw = inflateSync(Buffer.concat(idat));
  } catch {
    throw new Error("bad IDAT");
  }
  if (raw.length !== height * (stride + 1)) throw new Error("bad scanlines");
  const data = Buffer.alloc(width * height * 4);
  let prev = Buffer.alloc(stride);
  for (let y = 0; y < height; y += 1) {
    const filter = raw[y * (stride + 1)];
    const row = raw.subarray(y * (stride + 1) + 1, (y + 1) * (stride + 1));
    if (filter > 4) throw new Error("bad filter");
    const cur = Buffer.alloc(stride);
    for (let x = 0; x < stride; x += 1) {
      const a = x >= channels ? cur[x - channels] : 0;
      const b = prev[x];
      const c = x >= channels ? prev[x - channels] : 0;
      let pred = 0;
      if (filter === 1) pred = a;
      else if (filter === 2) pred = b;
      else if (filter === 3) pred = (a + b) >> 1;
      else if (filter === 4) {
        const p = a + b - c;
        const pa = Math.abs(p - a);
        const pb = Math.abs(p - b);
        const pc = Math.abs(p - c);
        pred = pa <= pb && pa <= pc ? a : pb <= pc ? b : c;
      }
      cur[x] = (row[x] + pred) & 0xff;
    }
    for (let x = 0; x < width; x += 1) {
      const o = (y * width + x) * 4;
      if (channels === 4) {
        data[o] = cur[x * 4];
        data[o + 1] = cur[x * 4 + 1];
        data[o + 2] = cur[x * 4 + 2];
        data[o + 3] = cur[x * 4 + 3];
      } else if (channels === 3) {
        data[o] = cur[x * 3];
        data[o + 1] = cur[x * 3 + 1];
        data[o + 2] = cur[x * 3 + 2];
        data[o + 3] = 0xff;
      } else {
        data[o] = cur[x];
        data[o + 1] = cur[x];
        data[o + 2] = cur[x];
        data[o + 3] = 0xff;
      }
    }
    prev = cur;
  }
  return { width, height, data };
}

export function loadCapture(path) {
  let bytes;
  try {
    bytes = readFileSync(path);
  } catch {
    fail(`missing file: ${path}`);
  }
  let decoded;
  try {
    decoded = decodePng(bytes);
  } catch (err) {
    fail(`undecodable PNG: ${path} (${err.message})`);
  }
  return {
    path,
    bytes: bytes.length,
    sha256: createHash("sha256").update(bytes).digest("hex"),
    ...decoded,
  };
}

// Compare at the smaller size (top-left overlap); changed pixels over all
// compared pixels, pixelmatch threshold 0.1, else a plain per-pixel RGB
// compare under the builtin engine.
export function compareCaptures(a, b, vendor) {
  const w = Math.min(a.width, b.width);
  const h = Math.min(a.height, b.height);
  const resized = a.width !== b.width || a.height !== b.height;
  // Hotspot: same-size identical frames skip both crop copies and the per-pixel walk.
  if (!resized && Buffer.isBuffer(a.data) && Buffer.isBuffer(b.data) && a.data.equals(b.data)) return { changed: 0, total: w * h, pct: 0, w, h, resized };
  const crop = (img) => {
    const out = Buffer.alloc(w * h * 4);
    for (let y = 0; y < h; y += 1) {
      img.data.copy(out, y * w * 4, (y * img.width) * 4, (y * img.width + w) * 4);
    }
    return out;
  };
  let changed;
  const da = crop(a);
  const db = crop(b);
  // Hotspot: identical frames skip the per-pixel walk with one native memcmp;
  // result fields are unchanged.
  if (da.equals(db)) return { changed: 0, total: w * h, pct: 0, w, h, resized };
  if (vendor) {
    const diff = Buffer.alloc(w * h * 4);
    changed = vendor.pixelmatch(da, db, diff, w, h, { threshold: PIXELMATCH_THRESHOLD });
  } else {
    changed = 0;
    for (let i = 0; i < da.length; i += 4) {
      if (da[i] !== db[i] || da[i + 1] !== db[i + 1] || da[i + 2] !== db[i + 2]) changed += 1;
    }
  }
  return { changed, total: w * h, pct: (100 * changed) / (w * h), w, h, resized };
}

// --- capture resolution ----------------------------------------------------
export function resolveCapture(input, cwd) {
  const abs = resolve(cwd, input);
  let st;
  try {
    st = statSync(abs);
  } catch {
    fail(`missing file: ${input}`);
  }
  if (st.isDirectory()) {
    const frames = readdirSync(abs).filter((f) => /^frame_\d+\.png$/.test(f)).sort();
    if (frames.length === 0) fail(`no frame_*.png in ${input}`);
    return join(abs, frames[frames.length - 1]);
  }
  return abs;
}

function newestCaptureUnder(dir) {
  let best = null;
  const walk = (d) => {
    for (const e of readdirSync(d, { withFileTypes: true })) {
      const p = join(d, e.name);
      if (e.isDirectory()) walk(p);
      else if (/^frame_\d+\.png$/.test(e.name)) {
        const m = statSync(p).mtimeMs;
        if (!best || m > best.mtimeMs || (m === best.mtimeMs && p > best.path)) best = { path: p, mtimeMs: m };
      }
    }
  };
  try {
    walk(dir);
  } catch {
    return null;
  }
  return best?.path ?? null;
}

// --- frozen sequence: every tick wrote the same pixels -----------------------
// A capture dir holds frame_*.png plus manifest.json (frames[].png_fnv /
// sha256). When all rows share one digest the camera never moved: the pair
// compare below would still say MOVED: none, so callers need this flag.
// Returns { count, kind, hash, source } when frozen, else null. Null also
// means no manifest/frames to judge (single frame, unreadable manifest).
export function frozenSequenceInfo(dirAbs) {
  let entries;
  try {
    entries = readdirSync(dirAbs, { withFileTypes: true });
  } catch {
    return null;
  }
  const manifestEntry = entries.find((e) => !e.isDirectory() && e.name === "manifest.json");
  if (manifestEntry) {
    const manifestPath = join(dirAbs, "manifest.json");
    try {
      const parsed = JSON.parse(readFileSync(manifestPath, "utf8"));
      const rows = Array.isArray(parsed?.frames) ? parsed.frames : null;
      if (rows && rows.length >= 2) {
        const fnvs = rows.map((r) => r?.png_fnv).filter((v) => typeof v === "string" && v.length > 0);
        if (fnvs.length === rows.length && new Set(fnvs).size === 1) {
          return { count: rows.length, kind: "png_fnv", hash: fnvs[0], source: manifestPath };
        }
        const shas = rows.map((r) => r?.sha256).filter((v) => typeof v === "string" && v.length > 0);
        if (shas.length === rows.length && new Set(shas).size === 1) {
          return { count: rows.length, kind: "sha256", hash: shas[0], source: manifestPath };
        }
      }
      if (rows && rows.length >= 2) return null;
    } catch {
      return null;
    }
  }
  const frames = entries
    .filter((e) => !e.isDirectory() && /^frame_\d+\.png$/.test(e.name))
    .map((e) => e.name)
    .sort();
  if (frames.length < 2) return null;
  try {
    const hashes = frames.map((f) => createHash("sha256").update(readFileSync(join(dirAbs, f))).digest("hex"));
    if (new Set(hashes).size === 1) {
      return { count: frames.length, kind: "sha256", hash: hashes[0], source: dirAbs };
    }
  } catch {
    return null;
  }
  return null;
}

// --- look stage: cubes until the board says the kit (VS1) or fighters (VS4) -
export function lookStage(boardText) {
  const ticked = new Set();
  for (const line of boardText.split(/\r?\n/)) {
    const m = line.match(/^-\s*\[(x|X)\]\s*(VS1|VS4)\b/);
    if (m) ticked.add(m[2]);
  }
  if (ticked.has("VS4")) return "fighters";
  if (ticked.has("VS1")) return "kit";
  return "cubes";
}

function readStage() {
  try {
    return lookStage(readFileSync(join(ROOT, "docs", "SPRINT.md"), "utf8"));
  } catch {
    return "cubes";
  }
}

function defaultOld() {
  try {
    const base = JSON.parse(readFileSync(baselinePath(), "utf8"));
    if (typeof base.source === "string" && base.source.length > 0) {
      const p = resolve(ROOT, base.source);
      if (existsSync(p)) return p;
    }
  } catch {
    // Fall through to the canonical cube frame.
  }
  const canon = join(ROOT, CANONICAL_BASE_PNG);
  if (existsSync(canon)) return canon;
  fail("no baseline yet: run node tools/frame-moved.mjs --baseline <png> first");
  return canon;
}

function lastPlaytestFrame() {
  const look = join(ROOT, "team", "look.md");
  if (!existsSync(look)) fail("no team/look.md yet: run a playtest first");
  const lines = readFileSync(look, "utf8").split(/\r?\n/).filter((l) => /^FRAME:\s*\S/.test(l));
  if (lines.length === 0) fail("team/look.md has no FRAME: line yet");
  const rest = lines[lines.length - 1].replace(/^FRAME:\s*/, "");
  const token = rest.split(/\s+/).find((t) => t.toLowerCase().endsWith(".png"))?.replace(/["']/g, "") ?? rest.split(/\s+/)[0];
  const abs = resolve(ROOT, token);
  if (!existsSync(abs)) fail(`playtest frame missing: ${token}`);
  return abs;
}

// --- CLI -------------------------------------------------------------------
function parseArgs(argv) {
  const args = { newInput: null, oldInput: null, baselinePng: null, sinceLast: false, json: false };
  for (let i = 0; i < argv.length; i += 1) {
    const a = argv[i];
    if (a === "--new") args.newInput = argv[(i += 1)];
    else if (a === "--old") args.oldInput = argv[(i += 1)];
    else if (a === "--baseline") args.baselinePng = argv[(i += 1)];
    else if (a === "--since-last-playtest") args.sinceLast = true;
    else if (a === "--json") args.json = true;
    else if (a === "--help" || a === "-h") {
      process.stdout.write(`${usage()}\n`);
      process.exit(0);
    } else {
      fail(`unknown argument: ${a}\n${usage()}`);
    }
  }
  if (!args.newInput && !args.baselinePng && !args.sinceLast) fail(`nothing to do\n${usage()}`);
  if (!args.newInput && !args.baselinePng && args.sinceLast) args.newInput = null;
  if ((!args.newInput || args.newInput.startsWith("-")) && !args.baselinePng && !args.sinceLast) {
    fail(`--new needs a <png|dir>\n${usage()}`);
  }
  return args;
}

async function main(argv, cwd) {
  const args = parseArgs(argv);
  const vendor = await vendorEngine();
  const engine = vendor ? "pixelmatch" : "builtin";

  if (args.baselinePng) {
    const pngPath = resolveCapture(args.baselinePng, cwd);
    const cap = loadCapture(pngPath);
    const rel = pngPath.startsWith(ROOT) ? pngPath.slice(ROOT.length + 1).replace(/\\/g, "/") : pngPath;
    const record = {
      sha256: cap.sha256,
      size: cap.bytes,
      source: rel,
      date: new Date().toISOString(),
    };
    try {
      mkdirSync(dirname(baselinePath()), { recursive: true });
      writeFileSync(baselinePath(), `${JSON.stringify(record, null, 2)}\n`);
    } catch {
      fail(`cannot write baseline: ${baselinePath()}`);
    }
    const frame = `FRAME: ${cap.sha256.slice(0, 8)} ${cap.width}x${cap.height}`;
    if (args.json) {
      process.stdout.write(`${JSON.stringify({ baseline: record, frame, engine })}\n`);
    } else {
      process.stdout.write(`${frame}\nBASELINE: wrote ${baselinePath()}\nENGINE: ${engine}\n`);
    }
    return;
  }

  let newPath;
  let oldPath;
  if (args.sinceLast) {
    const target = join(ROOT, "target");
    newPath = newestCaptureUnder(target);
    if (!newPath) fail("no captures under target/ yet");
    oldPath = lastPlaytestFrame();
  } else {
    newPath = resolveCapture(args.newInput, cwd);
    oldPath = args.oldInput ? resolveCapture(args.oldInput, cwd) : defaultOld();
  }

  const fresh = loadCapture(newPath);
  const old = loadCapture(oldPath);
  const cmp = compareCaptures(fresh, old, vendor);
  const moved = cmp.pct < NONE_PCT || fresh.sha256 === old.sha256 ? "none" : "pixel";
  const stage = readStage();
  const frame = `FRAME: ${fresh.sha256.slice(0, 8)} ${fresh.width}x${fresh.height}`;
  const frozen = frozenSequenceInfo(dirname(newPath));
  const frozenLabel = frozen ? "FROZEN: " + frozen.count + " frames share " + frozen.kind + " " + frozen.hash + " in " + frozen.source : null;

  if (args.json) {
    process.stdout.write(
      `${JSON.stringify({
        frame: { path: newPath, sha256: fresh.sha256, width: fresh.width, height: fresh.height },
        old: { path: oldPath, sha256: old.sha256, width: old.width, height: old.height },
        comparedAt: { width: cmp.w, height: cmp.h, resized: cmp.resized },
        movedPct: Math.round(cmp.pct * 100) / 100,
        moved,
        frozen,
        stage,
        engine,
      })}\n`,
    );
    return;
  }
  const lines = [frame];
  if (frozenLabel) {
    lines.push(frozenLabel);
  }
  if (cmp.resized) {
    lines.push(`NOTE: size mismatch ${fresh.width}x${fresh.height} vs ${old.width}x${old.height}, compared at ${cmp.w}x${cmp.h}`);
  }
  lines.push(moved === "none" ? "MOVED: none" : `MOVED: pixel ${cmp.pct.toFixed(2)}%`);
  lines.push(`LOOK_STAGE: ${stage}`);
  lines.push(`ENGINE: ${engine}`);
  process.stdout.write(`${lines.join("\n")}\n`);
}

const isMain = import.meta.main;
if (isMain) {
  try {
    await main(process.argv.slice(2), process.cwd());
  } catch (err) {
    if (process.exitCode !== 2) {
      process.stderr.write(`frame-moved: ${err.message}\n`);
      process.exitCode = 2;
    }
  }
}
