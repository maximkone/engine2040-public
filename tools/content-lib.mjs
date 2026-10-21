// Shared helpers for the content report tools (arena-report, fighter-report,
// provenance-audit, hit-list): file digests, image sizes, markdown tables and
// generated blocks. A generated block is the text between
// `<!-- gen:NAME -->` and `<!-- /gen:NAME -->` in a markdown file; the tool
// owns that text, everything outside the markers stays hand-written.
// Pure reads except `writeBlocks`. Plain Node, no network.
import { createHash } from "node:crypto";
import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

export const REPO = join(dirname(fileURLToPath(import.meta.url)), "..");

/** SHA-256 hex and byte count of one file, or null when it is absent. `text` hashes LF-normalised bytes, so a CRLF checkout agrees with the index. */
export function fileDigest(path, { text = false } = {}) {
  if (!existsSync(path)) return null;
  const raw = readFileSync(path);
  const bytes = text ? Buffer.from(raw.toString("utf8").replace(/\r\n/g, "\n")) : raw;
  return { sha256: createHash("sha256").update(bytes).digest("hex"), bytes: bytes.length };
}

/** A pinned file git stores as text (no LFS, line endings normalised): pin its LF bytes, or Windows and Linux disagree. */
export const isTextPin = (path) => /\.(json|md|txt|toml|ron|csv)$/i.test(path);

/** Parse a JSON file from disk. */
export function readJson(path) {
  return JSON.parse(readFileSync(path, "utf8"));
}

/** Width and height of a PNG (IHDR) or baseline/progressive JPEG (SOF), else null. */
export function imageSize(path) {
  const b = readFileSync(path);
  if (b.length > 24 && b.toString("latin1", 1, 4) === "PNG") return { w: b.readUInt32BE(16), h: b.readUInt32BE(20) };
  if (b[0] !== 0xff || b[1] !== 0xd8) return null;
  for (let i = 2; i + 9 < b.length; ) {
    if (b[i] !== 0xff) return null;
    const marker = b[i + 1];
    if (marker >= 0xc0 && marker <= 0xcf && ![0xc4, 0xc8, 0xcc].includes(marker)) {
      return { h: b.readUInt16BE(i + 5), w: b.readUInt16BE(i + 7) };
    }
    i += 2 + b.readUInt16BE(i + 2);
  }
  return null;
}

/** Width and height of a Radiance .hdr from its `-Y h +X w` line, else null. */
export function hdrSize(path) {
  const head = readFileSync(path).subarray(0, 512).toString("latin1");
  const m = /^-Y (\d+) \+X (\d+)$/m.exec(head);
  return m ? { w: Number(m[2]), h: Number(m[1]) } : null;
}

/** Upper-bound VRAM of an RGBA8 texture with a full mip chain (w*h*4*4/3, floored). */
export function rgba8MipBytes(w, h) {
  return Math.floor((w * h * 4 * 4) / 3);
}

/** Render a GitHub markdown table. Cells are stringified as-is. */
export function mdTable(headers, rows) {
  const line = (cells) => `| ${cells.join(" | ")} |`;
  return [line(headers), line(headers.map(() => "---")), ...rows.map((r) => line(r.map(String)))].join("\n");
}

/** MiB with one decimal, for report prose. */
export function mib(bytes) {
  return (bytes / 1048576).toFixed(1);
}

const open = (name) => `<!-- gen:${name} -->`;
const close = (name) => `<!-- /gen:${name} -->`;
const lf = (text) => text.replace(/\r\n/g, "\n");

/** Wrap a body in the generated-block markers. */
export function genBlock(name, body) {
  return `${open(name)}\n${body}\n${close(name)}`;
}

/** The text inside block `name` (LF), or null when the markers are absent. */
export function readBlock(text, name) {
  const t = lf(text);
  const a = t.indexOf(open(name));
  const b = t.indexOf(close(name));
  if (a < 0 || b < a) return null;
  return t.slice(a + open(name).length + 1, b - 1);
}

/** Replace block `name` in `text`, keeping the file's line endings. Throws when the markers are absent. */
export function spliceBlock(text, name, body) {
  const crlf = text.includes("\r\n");
  const t = lf(text);
  const a = t.indexOf(open(name));
  const b = t.indexOf(close(name));
  if (a < 0 || b < a) throw new Error(`block ${name} has no markers`);
  const next = t.slice(0, a) + genBlock(name, body) + t.slice(b + close(name).length);
  return crlf ? next.replace(/\n/g, "\r\n") : next;
}

/**
 * Check or write generated blocks. Each spec is {file, name, body, seed}:
 * `file` is repo-relative, `body` the generated text, `seed` the hand-written
 * page used when the file does not exist yet. Returns the stale block names.
 */
export function syncBlocks(specs, { write = false, root = REPO } = {}) {
  const stale = [];
  for (const { file, name, body, seed } of specs) {
    const path = join(root, file);
    const text = existsSync(path) ? readFileSync(path, "utf8") : seed;
    if (text !== undefined && readBlock(text, name) === body) continue;
    stale.push(`${file}#${name}`);
    if (write && text !== undefined) writeFileSync(path, spliceBlock(text, name, body));
  }
  return stale;
}
