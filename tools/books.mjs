#!/usr/bin/env node
// books: search the engine and Rust books on demand, so no agent loads a whole book.
// Each full_text.txt is cut into chunks of about 1,200 characters on paragraph
// boundaries; a heading always opens a chunk, so each chunk knows its section.
// BM25 ranks the chunks. Every hit names its book id and heading, for citations.
//   node tools/books.mjs search "<query>" [--book <id>[,<id>]] [--k 5]
//   node tools/books.mjs list                  id, language, size, title
//   node tools/books.mjs toc <id>              detected headings (max 200 lines)
// The index caches at target/books/index.json. It rebuilds when a full_text.txt is
// newer than the cache, or the folder or the book list changed.
// BOOKS_DIR overrides the work folder (default C:/empire/skillworks/work).
import { existsSync, mkdirSync, readFileSync, statSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
export const INDEX_PATH = join(ROOT, "target", "books", "index.json");
export const DEFAULT_DIR = "C:/empire/skillworks/work";
// Bump when chunking, headings or tokens change: the cache checks file times, not code.
export const INDEX_VERSION = 1;
export const CHUNK_CHARS = 1200;
const SNIPPET_CHARS = 700;
const TOC_MAX = 200;
const K1 = 1.2;
const B = 0.75;

// Book id -> title. Titles come from the engine review brief. The -m duplicates are left out.
export const BOOK_TITLES = {
  geaen: "Gregory, Game Engine Architecture, 2nd ed",
  gearus: "Gregory, Game Engine Architecture, 3rd ed",
  found1: "Lengyel, Foundations of Game Engine Development, vol 1: Mathematics",
  found2: "Lengyel, Foundations of Game Engine Development, vol 2: Rendering",
  gems1: "Game Engine Gems 1",
  gems2: "Game Engine Gems 2",
  gems3: "Game Engine Gems 3",
  trpl: "The Rust Programming Language, 3rd ed",
  progrust: "Programming Rust, 3rd ed",
  idiomatic: "Idiomatic Rust",
  bos: "Rust Atomics and Locks",
  transition: "Refactoring to Rust",
  rustwasm: "Game Development with Rust and WebAssembly",
  handsrust: "Wolverson, Hands-on Rust (game dev with bracket-lib, ECS)",
  testing: "Modern Game Testing",
  gogame: "Deep Learning and the Game of Go",
  dushkin: "Agentic and multi-agent design patterns",
};
export const BOOK_IDS = Object.keys(BOOK_TITLES);

export const USAGE = [
  'usage: node tools/books.mjs search "<query>" [--book <id>[,<id>]] [--k 5]',
  "       node tools/books.mjs list",
  "       node tools/books.mjs toc <id>",
  `ids: ${BOOK_IDS.join(" ")}`,
].join("\n");

// Heading lines: chapters, numbered sections (6.1, 6.1.1, gearus "6.1."), markdown #.
// Chapter titles are short; a long "Глава 3. ... и" line is an intro summary, not a heading.
const HEADING_RES = [
  // "Chapter 2," or "Chapter 2 we" is body text: a chapter line ends or starts a capitalised title.
  { re: /^(?:[Cc]hapter|Глава|глава)\s+\p{N}+[.:]?(?:\s+\p{Lu}|\s*$)/u, max: 60 },
  { re: /^(?:Appendix|Приложение)\s+[\p{Lu}\p{N}]/u, max: 60 },
  { re: /^\d+(?:\.\d+)+\.?\s+\S/u, max: 120 },
  { re: /^#{1,4}\s+[\p{L}\p{N}]/u, max: 120 },
];
export const normalizeText = (s) => String(s).replace(/^\uFEFF/, "").replace(/\r\n?/g, "\n");

// The heading text for one line, or null.
export const headingOf = (line) => {
  const t = String(line).trim().replace(/\s{2,}/g, " ");
  if (!t || t.length > 120 || t[0] === "|") return null;
  if (!HEADING_RES.some(({ re, max }) => t.length <= max && re.test(t))) return null;
  // TOC rows end with a page number ("12.1 Physics 648", "Chapter 9 Renderer 193").
  if (/\s\d{1,4}$/.test(t)) return null;
  return t.replace(/^#+\s*/, "");
};

// Every line with its heading text, or null. Lines inside ``` or ~~~ fences are never headings.
const scanLines = (text) => {
  let fence = false;
  return normalizeText(text).split("\n").map((raw, i) => {
    if (/^\s*(?:```|~~~)/.test(raw)) {
      fence = !fence;
      return { raw, line: i + 1, heading: null };
    }
    return { raw, line: i + 1, heading: fence ? null : headingOf(raw) };
  });
};

export const findHeadings = (text) =>
  scanLines(text).filter((l) => l.heading !== null).map((l) => ({ line: l.line, text: l.heading }));

// Paragraphs (blank-line separated) and heading lines, in order. Each unit has its start line.
const unitsOf = (text) => {
  const out = [];
  let buf = [];
  let start = 0;
  const flush = () => {
    if (buf.length) out.push({ text: buf.join("\n"), line: start + 1, heading: null });
    buf = [];
  };
  for (const { raw, line, heading } of scanLines(text)) {
    if (heading !== null) {
      flush();
      out.push({ text: raw.trim(), line, heading });
      continue;
    }
    if (!raw.trim()) {
      flush();
      continue;
    }
    if (!buf.length) start = line - 1;
    buf.push(raw);
  }
  flush();
  return out;
};

// Cut a long block into pieces of at most max characters: on line breaks first, then on spaces.
export const splitBlock = (text, max) => {
  const pieces = [];
  let cur = "";
  const put = (s) => {
    if (!s) return;
    if (!cur) cur = s;
    else if (cur.length + 1 + s.length <= max) cur += `\n${s}`;
    else {
      pieces.push(cur);
      cur = s;
    }
  };
  for (const line of text.split("\n")) {
    let rest = line;
    while (rest.length > max) {
      let cut = rest.lastIndexOf(" ", max);
      if (cut <= 0) cut = max;
      put(rest.slice(0, cut).trimEnd());
      rest = rest.slice(cut).trimStart();
    }
    put(rest);
  }
  if (cur) pieces.push(cur);
  return pieces;
};

// Chunks of about max characters. Each chunk's heading is the nearest heading above it.
// A chunk that holds only a heading is dropped: the next heading or text covers it.
export const chunkText = (text, max = CHUNK_CHARS) => {
  const chunks = [];
  let cur = null;
  let heading = null;
  const close = () => {
    if (cur && cur.body) chunks.push({ text: cur.text, heading: cur.heading, line: cur.line });
    cur = null;
  };
  for (const u of unitsOf(text)) {
    if (u.heading !== null) {
      close();
      heading = u.heading;
      cur = { text: u.text, heading, line: u.line, body: false };
      continue;
    }
    splitBlock(u.text, max).forEach((piece, i) => {
      if (cur && cur.body && cur.text.length + 2 + piece.length > max) close();
      if (!cur) cur = { text: piece, heading, line: u.line, body: true };
      else {
        cur.text += (i === 0 ? "\n\n" : "\n") + piece;
        cur.body = true;
      }
    });
  }
  close();
  return chunks;
};

// Lowercase word tokens: letters, marks and digits (Cyrillic included). ё folds to е.
export const tokenize = (text) =>
  String(text).normalize("NFC").toLowerCase().replace(/ё/g, "е").match(/[\p{L}\p{M}\p{N}]+/gu) ?? [];

export const detectLang = (text) => {
  const sample = text.slice(0, 200000);
  const letters = sample.match(/\p{L}/gu)?.length ?? 0;
  const cyrillic = sample.match(/\p{Script=Cyrillic}/gu)?.length ?? 0;
  return letters && cyrillic / letters > 0.3 ? "ru" : "en";
};

// books: [{ id, text }]. Pure: no file access, so tests can run on a small corpus.
export const buildIndex = (books, { dir = "", files = [] } = {}) => {
  const chunks = [];
  const bookRows = [];
  for (const { id, text } of books) {
    const body = normalizeText(text);
    bookRows.push({ id, title: BOOK_TITLES[id] ?? id, lang: detectLang(body), headings: findHeadings(body) });
    for (const c of chunkText(body)) chunks.push({ book: id, heading: c.heading, line: c.line, text: c.text, len: 0 });
  }
  const postings = new Map();
  let total = 0;
  chunks.forEach((c, d) => {
    const toks = tokenize(c.text);
    const tf = new Map();
    for (const t of toks) tf.set(t, (tf.get(t) ?? 0) + 1);
    c.len = toks.length;
    total += toks.length;
    for (const [t, n] of tf) {
      const p = postings.get(t);
      if (p) p.push(d, n);
      else postings.set(t, [d, n]);
    }
  });
  return {
    version: INDEX_VERSION,
    dir,
    files,
    books: bookRows,
    chunks,
    avgLen: chunks.length ? total / chunks.length : 0,
    postings,
  };
};

// files: [{ id, mtimeMs, size }] for the books on disk now. cacheMtimeMs: the cache file's mtime, or null.
export const needsRebuild = (cached, files, dir, cacheMtimeMs = null) => {
  if (!cached || cached.version !== INDEX_VERSION || cached.dir !== dir) return true;
  if (cached.files.length !== files.length) return true;
  const have = new Map(cached.files.map((f) => [f.id, f]));
  for (const f of files) {
    const old = have.get(f.id);
    if (!old || old.mtimeMs !== f.mtimeMs || old.size !== f.size) return true;
  }
  return cacheMtimeMs !== null && files.some((f) => f.mtimeMs > cacheMtimeMs);
};

// BM25 over the chunks. IDF uses the whole corpus; the book filter only drops hits.
export const searchIndex = (index, query, { books = null, k = 5 } = {}) => {
  const terms = [...new Set(tokenize(query))];
  const n = index.chunks.length;
  if (!terms.length || !n) return [];
  const avg = index.avgLen || 1;
  const allowed = books ? new Set(books) : null;
  const scores = new Map();
  for (const term of terms) {
    const p = index.postings.get(term);
    if (!p) continue;
    const df = p.length / 2;
    const idf = Math.log(1 + (n - df + 0.5) / (df + 0.5));
    for (let i = 0; i < p.length; i += 2) {
      const d = p[i];
      const c = index.chunks[d];
      if (allowed && !allowed.has(c.book)) continue;
      const tf = p[i + 1];
      const w = (idf * tf * (K1 + 1)) / (tf + K1 * (1 - B + (B * c.len) / avg));
      scores.set(d, (scores.get(d) ?? 0) + w);
    }
  }
  return [...scores]
    .sort((a, b) => b[1] - a[1] || a[0] - b[0])
    .slice(0, k)
    .map(([d, score]) => {
      const c = index.chunks[d];
      return { book: c.book, heading: c.heading, line: c.line, score, text: c.text };
    });
};

export const clip = (text, max = SNIPPET_CHARS) => (text.length > max ? `${text.slice(0, max).trimEnd()} ...` : text);

export const formatHit = (h) => `[${h.book} | ${h.heading ?? "(no heading)"}] ${h.score.toFixed(2)}\n${clip(h.text)}`;

const checkBook = (id) => {
  if (!BOOK_IDS.includes(id)) throw new Error(`unknown book id: ${id} (ids: ${BOOK_IDS.join(" ")})`);
  return id;
};

// argv after the script name. Throws Error with a plain message on bad input.
export const parseArgs = (argv) => {
  const [command, ...rest] = argv;
  if (!command || command === "--help" || command === "-h") return { command: "help" };
  if (command === "list") {
    if (rest.length) throw new Error("list takes no arguments");
    return { command };
  }
  if (command === "toc") {
    if (!rest[0]) throw new Error("toc needs a book id: toc <id>");
    return { command, id: checkBook(rest[0]) };
  }
  if (command !== "search") throw new Error(`unknown command: ${command}`);
  let books = null;
  let k = 5;
  const words = [];
  for (let i = 0; i < rest.length; i++) {
    const a = rest[i];
    if (a === "--book" || a.startsWith("--book=")) {
      const v = a.startsWith("--book=") ? a.slice("--book=".length) : rest[++i];
      if (v === undefined) throw new Error("--book needs a value: --book <id>[,<id>]");
      books = v.split(",").map((s) => s.trim()).filter(Boolean).map(checkBook);
      if (!books.length) throw new Error("--book needs at least one id");
    } else if (a === "--k" || a.startsWith("--k=")) {
      const v = a.startsWith("--k=") ? a.slice("--k=".length) : rest[++i];
      k = Number(v);
      if (!Number.isInteger(k) || k < 1 || k > 50) throw new Error("--k must be a whole number from 1 to 50");
    } else if (a.startsWith("--")) {
      throw new Error(`unknown flag: ${a}`);
    } else {
      words.push(a);
    }
  }
  const query = words.join(" ").trim();
  if (!query) throw new Error('search needs a query: search "<query>"');
  return { command: "search", query, books, k };
};

// The book files on disk: [{ id, path, mtimeMs, size }] in BOOK_IDS order.
export const bookFiles = (dir) =>
  BOOK_IDS.flatMap((id) => {
    const path = join(dir, `b1-${id}`, "full_text.txt");
    if (!existsSync(path)) return [];
    const st = statSync(path);
    return [{ id, path, mtimeMs: st.mtimeMs, size: st.size }];
  });

export const saveIndex = (index, path) => {
  mkdirSync(dirname(path), { recursive: true });
  writeFileSync(path, JSON.stringify({ ...index, postings: [...index.postings] }));
};

export const readIndex = (path) => {
  const raw = JSON.parse(readFileSync(path, "utf8"));
  return { ...raw, postings: new Map(raw.postings) };
};

// The cached index when it is current, else a fresh build (written to the cache).
export const loadIndex = (dir, files, { cachePath = INDEX_PATH } = {}) => {
  let cached = null;
  let cacheMtime = null;
  if (existsSync(cachePath)) {
    try {
      cached = readIndex(cachePath);
      cacheMtime = statSync(cachePath).mtimeMs;
    } catch {
      cached = null;
    }
  }
  const manifest = files.map(({ id, mtimeMs, size }) => ({ id, mtimeMs, size }));
  if (!needsRebuild(cached, manifest, dir, cacheMtime)) return cached;
  const t0 = performance.now();
  const books = files.map((f) => ({ id: f.id, text: readFileSync(f.path, "utf8") }));
  const index = buildIndex(books, { dir, files: manifest });
  saveIndex(index, cachePath);
  const secs = ((performance.now() - t0) / 1000).toFixed(1);
  const mb = (statSync(cachePath).size / 1e6).toFixed(1);
  console.error(`books: index built in ${secs} s, ${index.chunks.length} chunks, cache ${mb} MB`);
  return index;
};

const main = () => {
  let args;
  try {
    args = parseArgs(process.argv.slice(2));
  } catch (e) {
    console.error(`books: ${e.message}\n${USAGE}`);
    process.exitCode = 2;
    return;
  }
  if (args.command === "help") {
    console.log(USAGE);
    return;
  }
  const dir = process.env.BOOKS_DIR || DEFAULT_DIR;
  if (!existsSync(dir)) {
    console.error(`books: folder not found: ${dir} (set BOOKS_DIR to the folder holding b1-<id>/full_text.txt)`);
    process.exitCode = 1;
    return;
  }
  const files = bookFiles(dir);
  if (!files.length) {
    console.error(`books: no b1-<id>/full_text.txt under ${dir}`);
    process.exitCode = 1;
    return;
  }
  const index = loadIndex(dir, files);
  if (args.command === "list") {
    for (const id of BOOK_IDS) {
      const f = files.find((x) => x.id === id);
      const b = index.books.find((x) => x.id === id);
      if (!f || !b) {
        console.log(`${id.padEnd(10)} missing`);
        continue;
      }
      console.log(`${id.padEnd(10)} ${b.lang}  ${(f.size / 1e6).toFixed(2).padStart(6)} MB  ${b.title}`);
    }
    return;
  }
  if (args.command === "toc") {
    const b = index.books.find((x) => x.id === args.id);
    if (!b) {
      console.error(`books: no text for ${args.id} under ${dir}`);
      process.exitCode = 1;
      return;
    }
    if (!b.headings.length) console.log("no headings detected");
    for (const h of b.headings.slice(0, TOC_MAX)) console.log(`${String(h.line).padStart(7)}  ${h.text}`);
    if (b.headings.length > TOC_MAX) console.log(`... ${b.headings.length - TOC_MAX} more (${b.headings.length} headings in all)`);
    return;
  }
  const onDisk = new Set(index.books.map((b) => b.id));
  const missing = (args.books ?? []).filter((id) => !onDisk.has(id));
  if (missing.length) {
    console.error(`books: no full_text.txt for ${missing.join(", ")} under ${dir}`);
    process.exitCode = 1;
    return;
  }
  const hits = searchIndex(index, args.query, { books: args.books, k: args.k });
  if (!hits.length) {
    console.log(`no hits for "${args.query}"`);
    return;
  }
  console.log(hits.map(formatHit).join("\n\n"));
};

if (import.meta.main) main();
