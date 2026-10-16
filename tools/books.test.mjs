// Tests for tools/books.mjs: chunks keep headings, BM25 ranks the right chunk first,
// Russian tokens match, the cache rule holds, CLI arguments parse, and a missing folder fails in one line.
// Run: node --test tools/books.test.mjs
import { test } from "node:test";
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import {
  BOOK_IDS,
  INDEX_VERSION,
  buildIndex,
  chunkText,
  findHeadings,
  headingOf,
  needsRebuild,
  parseArgs,
  searchIndex,
  tokenize,
} from "./books.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));

test("chunks keep each heading at the start of its section", () => {
  const text = [
    "Chapter 1 Introduction", "", "Para one about frames.", "",
    "1.1 Timing", "", "Para two about the fixed time step.", "Second line of para two.", "",
    "1.2 Culling", "", "Para four about frustum culling.",
  ].join("\n");
  const chunks = chunkText(text);
  assert.deepEqual(chunks.map((c) => c.heading), ["Chapter 1 Introduction", "1.1 Timing", "1.2 Culling"]);
  assert.match(chunks[1].text, /^1\.1 Timing\n\nPara two/);
  assert.ok(!chunks[0].text.includes("Para two"));
});

test("chunks stay near 1,200 characters and keep all the text", () => {
  const paras = Array.from({ length: 60 }, (_, i) => `Paragraph ${i} ${"word ".repeat(20)}`.trimEnd());
  const chunks = chunkText(paras.join("\n\n"));
  assert.ok(chunks.length > 1);
  for (const c of chunks) assert.ok(c.text.length <= 1250, `chunk too long: ${c.text.length}`);
  const joined = chunks.map((c) => c.text).join("\n");
  for (let i = 0; i < 60; i++) assert.ok(joined.includes(`Paragraph ${i} `), `missing paragraph ${i}`);
});

test("heading detection covers numbered, chapter, markdown and Russian lines, not TOC rows", () => {
  assert.equal(headingOf("1.5.10 Other Commercial Engines"), "1.5.10 Other Commercial Engines");
  assert.equal(headingOf("6.1. Подсистема запуска и остановки"), "6.1. Подсистема запуска и остановки");
  assert.equal(headingOf("### Building for Release"), "Building for Release");
  assert.equal(headingOf("Глава 3. Рендеринг"), "Глава 3. Рендеринг");
  assert.equal(headingOf("Chapter 10 Lifetimes"), "Chapter 10 Lifetimes");
  assert.equal(headingOf("11.11 ActionStateMachines 621"), null);
  assert.equal(headingOf("We measured 2.5 times more"), null);
  assert.equal(headingOf("| table | row |"), null);
  assert.equal(headingOf("Chapter 2, we'll explore the tools"), null);
  // A long intro summary that starts "Глава 3." is not a chapter heading.
  assert.equal(headingOf("Глава 3. Упорядочение памяти. После изучения различных атомарных операций и"), null);
  assert.equal(headingOf("Глава 3. Упорядочение памяти"), "Глава 3. Упорядочение памяти");
  assert.deepEqual(findHeadings("intro\n\n2.1 Frames\ntext"), [{ line: 3, text: "2.1 Frames" }]);
  // A shell comment inside a code fence is not a heading.
  assert.deepEqual(findHeadings("```sh\n# Concurrently, in a\n```\n\n## Real Heading"), [{ line: 5, text: "Real Heading" }]);
});

test("BM25 ranks the chunk with the query words first", () => {
  const index = buildIndex([
    { id: "a", text: "Frame pacing with a fixed time step keeps the simulation steady." },
    { id: "b", text: "Render culling skips objects outside the camera frustum." },
    { id: "c", text: "Pacing a sprint board and planning the next release." },
  ], { dir: "mem" });
  const hits = searchIndex(index, "frame pacing fixed time step", { k: 3 });
  assert.equal(hits[0].book, "a");
  assert.equal(hits.length, 2);
  assert.ok(hits[0].score > hits[1].score);
  assert.deepEqual(searchIndex(index, "frustum", { books: ["a"] }), []);
});

test("Russian text tokenizes and matches, with ё folded to е", () => {
  assert.deepEqual(tokenize("Фиксированный шаг, ёлка 2040!"), ["фиксированный", "шаг", "елка", "2040"]);
  const index = buildIndex([
    { id: "ru", text: "Фиксированный шаг времени делает симуляцию повторяемой." },
    { id: "en", text: "A fixed time step keeps the simulation repeatable." },
  ], { dir: "mem" });
  assert.equal(index.books.find((b) => b.id === "ru").lang, "ru");
  assert.equal(index.books.find((b) => b.id === "en").lang, "en");
  const hits = searchIndex(index, "ФИКСИРОВАННЫЙ шаг времени", { k: 3 });
  assert.equal(hits.length, 1);
  assert.equal(hits[0].book, "ru");
});

test("the cache rebuilds when the folder changed, a book changed, or a book is newer than the cache", () => {
  const files = [{ id: "geaen", mtimeMs: 1000, size: 5 }];
  const cached = { version: INDEX_VERSION, dir: "d", files };
  assert.equal(needsRebuild(null, files, "d", 2000), true);
  assert.equal(needsRebuild(cached, files, "d", 2000), false);
  assert.equal(needsRebuild(cached, files, "other", 2000), true);
  assert.equal(needsRebuild(cached, [{ id: "geaen", mtimeMs: 1500, size: 5 }], "d", 2000), true);
  assert.equal(needsRebuild(cached, files, "d", 500), true);
});

test("CLI arguments parse and bad input is refused", () => {
  assert.deepEqual(
    parseArgs(["search", "frame pacing fixed time step", "--book", "geaen,trpl", "--k", "3"]),
    { command: "search", query: "frame pacing fixed time step", books: ["geaen", "trpl"], k: 3 },
  );
  assert.deepEqual(parseArgs(["search", "render", "--book=gems1"]), { command: "search", query: "render", books: ["gems1"], k: 5 });
  assert.deepEqual(parseArgs(["search", "frame", "pacing"]), { command: "search", query: "frame pacing", books: null, k: 5 });
  assert.deepEqual(parseArgs(["list"]), { command: "list" });
  assert.deepEqual(parseArgs(["toc", "geaen"]), { command: "toc", id: "geaen" });
  assert.equal(parseArgs([]).command, "help");
  assert.throws(() => parseArgs(["search"]), /needs a query/);
  assert.throws(() => parseArgs(["search", "x", "--book", "nope"]), /unknown book id/);
  assert.throws(() => parseArgs(["search", "x", "--k", "0"]), /--k/);
  assert.throws(() => parseArgs(["search", "x", "--k", "51"]), /--k/);
  assert.throws(() => parseArgs(["toc"]), /needs a book id/);
  assert.throws(() => parseArgs(["bogus"]), /unknown command/);
  assert.equal(BOOK_IDS.length, 17);
  assert.ok(!BOOK_IDS.some((id) => id.endsWith("-m")));
});

test("a missing books folder prints one line and exits 1", () => {
  const missing = join(tmpdir(), `no-books-${process.pid}-${Date.now()}`);
  const r = spawnSync(process.execPath, [join(HERE, "books.mjs"), "list"], {
    env: { ...process.env, BOOKS_DIR: missing },
    encoding: "utf8",
  });
  assert.equal(r.status, 1);
  const lines = r.stderr.trim().split("\n");
  assert.equal(lines.length, 1);
  assert.match(lines[0], /BOOKS_DIR/);
});
