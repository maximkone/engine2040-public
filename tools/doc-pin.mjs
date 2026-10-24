#!/usr/bin/env node
// Pin a doc to the source contract it copies. The doc carries the first 8
// bytes of sha256 over the source file's CRLF-normalised text; a source edit
// without the doc's pin moving fails, so the hand-copied table cannot drift
// away from the file that owns it. `tools/docs-links.mjs` checks that a link
// resolves; this checks the text behind it.
//   node tools/doc-pin.mjs           check every pin, exit 1 on a mismatch
//   node tools/doc-pin.mjs --write   write each source's current hash into its doc
// Donor: rust-analyzer xtask/src/tidy.rs (`check_lsp_extensions_docs`,
// `stable_hash`; MIT OR Apache-2.0).
import { createHash } from "node:crypto";
import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

// The first 8 bytes of sha256, hex: 16 characters, short enough to read in a
// marker and long enough that a one-byte edit moves it.
export const PIN_HEX_CHARS = 16;

// A CRLF checkout and an LF checkout of the same file pin the same, so the pin
// survives git's line-ending normalisation.
export function normalizeText(text) {
  return String(text ?? "").replace(/\r\n/g, "\n");
}

export function pinHash(text) {
  return createHash("sha256").update(normalizeText(text), "utf8").digest("hex").slice(0, PIN_HEX_CHARS);
}

// The registry: one row per page that copies a source file's contract by hand.
// A page is pinned only when it restates a contract the source owns; a page
// generated from the source (docs/book/door-reference.md) needs no row.
export const DOC_PINS = [
  {
    docPath: "docs/book/for-ai.md",
    sourcePath: "crates/endpoint/src/resources.rs",
    what: "the door's verbs to start with: the page lists them by hand, the resource catalog owns the surface",
  },
];

function escapeRe(text) {
  return String(text).replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

export function marker(sourcePath, hash) {
  return `<!-- doc-pin: ${sourcePath} sha256-8:${hash} -->`;
}

function markerRe(sourcePath) {
  return new RegExp(`<!--\\s*doc-pin:\\s*${escapeRe(sourcePath)}\\s+sha256-8:([0-9a-f]{${PIN_HEX_CHARS}})\\s*-->`);
}

// The marker a doc carries for one source, or null when it carries none.
export function findPin(docText, sourcePath) {
  const text = String(docText ?? "");
  const m = markerRe(sourcePath).exec(text);
  if (!m) return null;
  return { sourcePath, hash: m[1], line: text.slice(0, m.index).split(/\r?\n/).length };
}

// The doc's text with the source's marker moved to `hash`. An existing marker
// is replaced in place (the page keeps its line endings); a page with no
// marker gets one at its end.
export function writePin(docText, sourcePath, hash) {
  const text = String(docText ?? "");
  const re = markerRe(sourcePath);
  if (re.test(text)) return text.replace(re, marker(sourcePath, hash));
  const eol = text.includes("\r\n") ? "\r\n" : "\n";
  const head = text.endsWith("\n") ? text : `${text}${eol}`;
  return `${head}${eol}${marker(sourcePath, hash)}${eol}`;
}

export function docPinFix(entry) {
  return `read ${entry.sourcePath} and update ${entry.docPath} to match, then node tools/doc-pin.mjs --write`;
}

// One finding for a stale or missing pin, null when the doc matches its source.
export function checkPin(entry, docText, sourceText) {
  const pin = findPin(docText, entry.sourcePath);
  if (!pin) return `${entry.docPath}: no doc-pin marker for ${entry.sourcePath} fix: ${docPinFix(entry)}`;
  const actual = pinHash(sourceText);
  if (pin.hash === actual) return null;
  return `${entry.docPath}:${pin.line}: doc pin ${pin.hash} != ${actual} for ${entry.sourcePath} (the source changed and the page that copies it did not) fix: ${docPinFix(entry)}`;
}

// Every stale pin under `root`, in registry order. A row whose doc or source is
// missing is a finding too: a pin that names nothing checks nothing.
export function checkPins(root = ".", registry = DOC_PINS) {
  const out = [];
  for (const entry of registry) {
    const docAbs = join(root, entry.docPath);
    const sourceAbs = join(root, entry.sourcePath);
    if (!existsSync(docAbs)) {
      out.push(`${entry.docPath}: missing, and tools/doc-pin.mjs pins it fix: restore the page, or drop its DOC_PINS row`);
      continue;
    }
    if (!existsSync(sourceAbs)) {
      out.push(`${entry.sourcePath}: missing, the source tools/doc-pin.mjs pins ${entry.docPath} to fix: restore the source, or drop its DOC_PINS row`);
      continue;
    }
    const finding = checkPin(entry, readFileSync(docAbs, "utf8"), readFileSync(sourceAbs, "utf8"));
    if (finding) out.push(finding);
  }
  return out;
}

// Bless: write each source's current hash into its doc. Returns the doc paths
// it wrote, so the CLI can say what moved.
export function writePins(root = ".", registry = DOC_PINS) {
  const written = [];
  for (const entry of registry) {
    const docAbs = join(root, entry.docPath);
    const sourceAbs = join(root, entry.sourcePath);
    if (!existsSync(docAbs) || !existsSync(sourceAbs)) continue; // checkPins names the gap
    const before = readFileSync(docAbs, "utf8");
    const after = writePin(before, entry.sourcePath, pinHash(readFileSync(sourceAbs, "utf8")));
    if (after !== before) writeFileSync(docAbs, after);
    written.push(entry.docPath);
  }
  return written;
}

export const DOC_PIN_HELP = `doc-pin.mjs - pin a doc to the source contract it copies

Usage:
  node tools/doc-pin.mjs           check every DOC_PINS row, exit 1 on a stale pin
  node tools/doc-pin.mjs --write   write each source's current hash into its doc
  node tools/doc-pin.mjs --root <dir>  check a tree other than this repo (tests use it)
  node tools/doc-pin.mjs --help    print this help and exit 0

The marker in the doc is \`<!-- doc-pin: <sourcePath> sha256-8:<16 hex> -->\`,
the first 8 bytes of sha256 over the source's CRLF-normalised text. A source
edit without the doc edit fails here and in tools/workspace-check.mjs.

Exit codes: 0 pass - 1 stale pin - 2 usage.
`;

export function hasHelpFlag(argv = process.argv.slice(2)) {
  return argv.includes("--help") || argv.includes("-h");
}

function main(argv) {
  if (hasHelpFlag(argv)) {
    process.stdout.write(DOC_PIN_HELP);
    return 0;
  }
  const opts = { write: false, root: "" };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (a === "--write") opts.write = true;
    else if (a === "--root") opts.root = argv[++i] ?? "";
    else {
      console.error(`unknown argument ${a}; only --write, --root and --help are taken`);
      return 2;
    }
  }
  if (opts.root === "" && argv.includes("--root")) {
    console.error("--root needs a directory");
    return 2;
  }
  const root = opts.root || join(dirname(fileURLToPath(import.meta.url)), "..");
  if (opts.write) {
    const written = writePins(root);
    console.log(`WROTE ${written.length} doc pin(s): ${written.join(", ")}`);
    return 0;
  }
  const findings = checkPins(root);
  for (const f of findings) console.log(`[FAIL] doc pin ${f}`);
  console.log(findings.length
    ? `RESULT FAIL: ${findings.length} doc pin(s) stale`
    : `RESULT PASS: ${DOC_PINS.length} doc pin(s) match their source`);
  return findings.length ? 1 : 0;
}

if (import.meta.main) {
  process.exitCode = main(process.argv.slice(2));
}
