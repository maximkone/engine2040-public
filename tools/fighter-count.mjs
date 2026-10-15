// fighter-count: fighter-vs-box pixel counter for showcase frames.
//
// Job: give J1 a repeatable number on staged arena captures. The staged
// stand-in scene holds a dome plus red dressing boxes and no fighters,
// so the pinned result is fighter = 0 with a nonzero box count. Same PNG
// bytes always give the same counts: pure decode plus fixed v1 rules.
//
// v1 rules (change only with a version bump):
//   box:     R >= 120 and R - G >= 60 and R - B >= 50 (dressing-box red)
//   fighter: B >= 140 and B - R >= 50 and 70 <= G <= 210 (fighter-kit blue)
// Box wins ties. The two rules are disjoint by construction.
//
// Usage:
//   node tools/fighter-count.mjs [--dir <frames-dir>] [--json] [--help]
// Default dir is the newest frame set under target/showcase.
//
// Exit codes: 0 success, 1 read or decode failure, 2 bad CLI use.

import { readdirSync, readFileSync, statSync } from "node:fs";
import { inflateSync } from "node:zlib";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

export const RULE_VERSION = 1;
export const BOX_RULE = "R>=120,R-G>=60,R-B>=50";
export const FIGHTER_RULE = "B>=140,B-R>=50,70<=G<=210";
export const PNG_SIGNATURE = Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]);
const MAX_PIXELS = 64000000;

/** True for dressing-box red pixels. Inputs are 0-255 channels. */
export function isBoxPixel(r, g, b) {
  return r >= 120 && r - g >= 60 && r - b >= 50;
}

/** True for fighter-kit blue pixels. Inputs are 0-255 channels. */
export function isFighterPixel(r, g, b) {
  return b >= 140 && b - r >= 50 && g >= 70 && g <= 210;
}

/** Classify one pixel: "box" wins ties, else "fighter", else "other". */
export function classifyPixel(r, g, b) {
  if (isBoxPixel(r, g, b)) return "box";
  if (isFighterPixel(r, g, b)) return "fighter";
  return "other";
}

function readChunk(buf, pos, label) {
  if (pos + 12 > buf.length) throw new Error(label + ": truncated PNG");
  const len = buf.readUInt32BE(pos);
  const type = buf.toString("ascii", pos + 4, pos + 8);
  if (pos + 12 + len > buf.length) throw new Error(label + ": truncated " + type);
  return { len, type, data: buf.subarray(pos + 8, pos + 8 + len), next: pos + 12 + len };
}

function unfilter(raw, width, height, channels) {
  const stride = width * channels;
  const out = Buffer.alloc(height * stride);
  let prev = Buffer.alloc(stride);
  for (let y = 0; y < height; y += 1) {
    const rowStart = y * (stride + 1);
    const filter = raw[rowStart];
    if (filter > 4) throw new Error("unknown filter " + filter + " on row " + y);
    const row = raw.subarray(rowStart + 1, rowStart + 1 + stride);
    const base = y * stride;
    for (let x = 0; x < stride; x += 1) {
      const a = x >= channels ? out[base + x - channels] : 0;
      const b = prev[x];
      const c = x >= channels ? prev[x - channels] : 0;
      let v = row[x];
      if (filter === 1) v += a;
      else if (filter === 2) v += b;
      else if (filter === 3) v += (a + b) >> 1;
      else if (filter === 4) {
        const q = a + b - c;
        const pa = Math.abs(q - a);
        const pb = Math.abs(q - b);
        const pc = Math.abs(q - c);
        v += pa <= pb && pa <= pc ? a : pb <= pc ? b : c;
      }
      out[base + x] = v & 255;
    }
    prev = out.subarray(base, base + stride);
  }
  return out;
}

/**
 * Decode an 8-bit PNG (color types 2 RGB or 6 RGBA, no interlace).
 * Returns { width, height, data } with data as RGBA bytes.
 * Throws Error on bad signature, truncation, or unsupported layout.
 */
export function decodePng(buf, label = "png") {
  const bytes = Buffer.isBuffer(buf) ? buf : Buffer.from(buf);
  if (bytes.length < 8 || !bytes.subarray(0, 8).equals(PNG_SIGNATURE)) {
    throw new Error(label + ": not a PNG (bad signature)");
  }
  let width = 0;
  let height = 0;
  let channels = 0;
  let seenIhdr = false;
  const idats = [];
  let pos = 8;
  while (pos < bytes.length) {
    const { type, data, next } = readChunk(bytes, pos, label);
    if (type === "IHDR") {
      if (data.length !== 13) throw new Error(label + ": bad IHDR");
      width = data.readUInt32BE(0);
      height = data.readUInt32BE(4);
      const bitDepth = data[8];
      const colorType = data[9];
      const interlace = data[12];
      if (width === 0 || height === 0) throw new Error(label + ": bad size");
      if (width * height > MAX_PIXELS) throw new Error(label + ": image too large");
      if (bitDepth !== 8) throw new Error(label + ": unsupported bit depth " + bitDepth);
      if (colorType === 6) channels = 4;
      else if (colorType === 2) channels = 3;
      else throw new Error(label + ": unsupported color type " + colorType);
      if (interlace !== 0) throw new Error(label + ": interlaced PNG not supported");
      seenIhdr = true;
    } else if (type === "IDAT") {
      idats.push(data);
    } else if (type === "IEND") {
      pos = next;
      break;
    }
    pos = next;
  }
  if (!seenIhdr) throw new Error(label + ": missing IHDR");
  if (idats.length === 0) throw new Error(label + ": missing image data");
  let raw;
  try {
    raw = inflateSync(Buffer.concat(idats));
  } catch (err) {
    throw new Error(label + ": deflate failed", { cause: err });
  }
  const stride = width * channels;
  if (raw.length !== height * (stride + 1)) {
    throw new Error(label + ": bad decoded size");
  }
  const flat = unfilter(raw, width, height, channels);
  let data = flat;
  if (channels === 3) {
    data = Buffer.alloc(width * height * 4);
    for (let i = 0, o = 0; i < flat.length; i += 3, o += 4) {
      data[o] = flat[i];
      data[o + 1] = flat[i + 1];
      data[o + 2] = flat[i + 2];
      data[o + 3] = 255;
    }
  }
  return { width, height, data };
}

/** Count box and fighter pixels in one PNG buffer. Pure and repeatable. */
export function countPngBuffer(buf, label = "png") {
  const { width, height, data } = decodePng(buf, label);
  let box = 0;
  let fighter = 0;
  for (let i = 0; i < data.length; i += 4) {
    const kind = classifyPixel(data[i], data[i + 1], data[i + 2]);
    if (kind === "box") box += 1;
    else if (kind === "fighter") fighter += 1;
  }
  return { file: label, width, height, box, fighter, total: width * height };
}

/** True when dir holds at least one PNG frame. Never throws. */
export function hasPngFrames(dir) {
  try {
    return readdirSync(dir).some((n) => n.toLowerCase().endsWith(".png"));
  } catch {
    return false;
  }
}

/**
 * Count every PNG frame in dir (sorted by name).
 * Returns { dir, frames, totals } with totals { frames, box, fighter, pixels }.
 */
export function countDir(dir) {
  let stat;
  try {
    stat = statSync(dir);
  } catch (err) {
    throw new Error(dir + ": cannot read dir", { cause: err });
  }
  if (!stat.isDirectory()) throw new Error(dir + ": not a directory");
  const names = readdirSync(dir)
    .filter((n) => n.toLowerCase().endsWith(".png"))
    .sort();
  if (names.length === 0) throw new Error(dir + ": no PNG frames");
  const frames = names.map((n) => countPngBuffer(readFileSync(join(dir, n)), n));
  const totals = {
    frames: frames.length,
    box: frames.reduce((s, f) => s + f.box, 0),
    fighter: frames.reduce((s, f) => s + f.fighter, 0),
    pixels: frames.reduce((s, f) => s + f.total, 0),
  };
  return { dir, frames, totals };
}

/**
 * Newest frame set under <repoRoot>/target/showcase: the latest dated dir
 * that holds PNGs, preferring its orbit/ child, then its staged-fight/
 * child, when one of them holds the frames.
 */
export function findNewestShowcaseDir(repoRoot = fileURLToPath(new URL("..", import.meta.url))) {
  const root = join(repoRoot, "target", "showcase");
  let entries;
  try {
    entries = readdirSync(root).sort().reverse();
  } catch (err) {
    throw new Error(root + ": cannot list showcase", { cause: err });
  }
  for (const name of entries) {
    if (!/^\d{4}-\d{2}-\d{2}/.test(name)) continue;
    const full = join(root, name);
    let stat;
    try {
      stat = statSync(full);
    } catch {
      continue;
    }
    if (!stat.isDirectory()) continue;
    const orbit = join(full, "orbit");
    if (hasPngFrames(orbit)) return orbit;
    const staged = join(full, "staged-fight");
    if (hasPngFrames(staged)) return staged;
    if (hasPngFrames(full)) return full;
  }
  throw new Error(root + ": no frame set found");
}

/** Parse CLI args. Returns { dir, json, help } or { bad } on misuse. */
export function parseArgs(argv) {
  const out = { dir: null, json: false, help: false };
  for (let i = 0; i < argv.length; i += 1) {
    const a = argv[i];
    if (a === "--dir") {
      const v = argv[i + 1];
      if (v === undefined || v.startsWith("--")) return { bad: true };
      out.dir = v;
      i += 1;
    } else if (a === "--json") {
      out.json = true;
    } else if (a === "--help" || a === "-h") {
      out.help = true;
    } else {
      return { bad: true };
    }
  }
  return out;
}

export function helpText() {
  return [
    "Usage: node tools/fighter-count.mjs [--dir <frames-dir>] [--json] [--help]",
    "  Count fighter-kit blue vs dressing-box red pixels per showcase frame.",
    "  Default dir is the newest frame set under target/showcase.",
    "Options:",
    "  --dir D     count PNG frames in D instead of the newest showcase set",
    "  --json      print the result as JSON instead of text",
    "  --help, -h  print this help, exit 0",
  ].join("\n");
}

function formatHuman(result) {
  const lines = [];
  lines.push("dir: " + result.dir);
  lines.push("rules: v" + RULE_VERSION + " box(" + BOX_RULE + ") fighter(" + FIGHTER_RULE + ")");
  for (const f of result.frames) {
    lines.push(f.file + ": box=" + f.box + " fighter=" + f.fighter);
  }
  const t = result.totals;
  lines.push("total: frames=" + t.frames + " box=" + t.box + " fighter=" + t.fighter + " pixels=" + t.pixels);
  if (t.fighter === 0) lines.push("verdict: fighters=0 (repeatable, none staged)");
  return lines.join("\n");
}

/** CLI entry. Returns the process exit code. */
export function main(argv = []) {
  const opts = parseArgs(argv);
  if (opts.bad) {
    process.stderr.write(helpText() + "\n");
    return 2;
  }
  if (opts.help) {
    process.stdout.write(helpText() + "\n");
    return 0;
  }
  let dir = opts.dir;
  if (dir === null || dir === undefined) {
    try {
      dir = findNewestShowcaseDir();
    } catch (err) {
      process.stderr.write(String(err && err.message ? err.message : err) + "\n");
      return 1;
    }
  }
  let result;
  try {
    result = countDir(dir);
  } catch (err) {
    process.stderr.write(String(err && err.message ? err.message : err) + "\n");
    return 1;
  }
  if (opts.json) {
    process.stdout.write(
      JSON.stringify({ version: RULE_VERSION, boxRule: BOX_RULE, fighterRule: FIGHTER_RULE, ...result }) + "\n",
    );
  } else {
    process.stdout.write(formatHuman(result) + "\n");
  }
  return 0;
}

if (import.meta.main) {
  process.exit(main(process.argv.slice(2)));
}
