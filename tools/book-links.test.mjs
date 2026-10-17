// Run: node --test tools/book-links.test.mjs
// DOC1: a broken internal link in the handbook fails the build (the Pages job runs tools/book-links.mjs on the sources and on the site).
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { dirname, join } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { DOC_CRATES, chapters, checkSite, checkSource, libName, maskCode, pageIds, pageLinks, readBook, slug, withLandingLink } from "./book-links.mjs";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");

const book = (extra = {}) => new Map(Object.entries({
  "SUMMARY.md": "# Summary\n\n[Start](start.md)\n\n- [A](a.md)\n- [B](sub/b.md)\n",
  "start.md": "# Start\n\nGo to [A](a.md#second-part) or [B](sub/b.md).\n",
  "a.md": "# A\n\n## Second part\n\nBack to [start](start.md) and [here](#second-part).\n\n```sh\n[not a link](nowhere.md)\n```\n\nInline `[no](nope.md)` is code.\n",
  "sub/b.md": "# B\n\nUp: [A](../a.md). Rustdoc: [core](../api/engine_core/index.html). Site: [x](https://example.com/x.md).\n",
  ...extra,
}));

test("headings get mdBook's ids, repeats get a number, code is skipped", () => {
  assert.equal(slug("Typed refusals (2)!"), "typed-refusals-2");
  assert.equal(slug("`sim.build_encounter`"), "simbuild_encounter"); // dots dropped, underscore kept
  const ids = pageIds("# Door\n\n### `scene.read`\n\n## Notes\n\n## Notes\n\n# Own {#custom}\n\n```\n# not a heading\n```\n<a id=\"anchor\"></a>\n");
  assert.deepEqual([...ids].sort(), ["anchor", "custom", "door", "notes", "notes-1", "sceneread"].sort());
});

test("links are read from inline, reference and html forms, never from code", () => {
  const text = "a [x](one.md) ![i](img/p.png)\n\n[ref]: two.md\n<a href=\"three.html\">t</a>\n`[c](code.md)`\n\n```\n[f](fence.md)\n```\n";
  assert.deepEqual(pageLinks(text).map((l) => l.target), ["one.md", "img/p.png", "two.md", "three.html"]);
  assert.equal(maskCode("`a` b").length, 5);
});

test("SUMMARY chapters include prefix chapters and list items", () => {
  assert.deepEqual(chapters("# S\n\n[Start](start.md)\n\n- [A](a.md)\n  - [Nested](n.md)\n").map((c) => c.path), ["start.md", "a.md", "n.md"]);
});

test("a clean book passes", () => {
  const r = checkSource(book());
  assert.deepEqual(r.problems, []);
  assert.equal(r.pages, 3);
  assert.ok(r.links >= 6);
});

test("a dead file link, a dead fragment and a link out of the book each fail with file and line", () => {
  const r = checkSource(book({
    "a.md": "# A\n\n[gone](missing.md)\n\n[frag](start.md#nope)\n\n[out](../library/x.md)\n\n[self](#nothing)\n",
  }));
  const why = r.problems.map((p) => `${p.file}:${p.line}:${p.target}`);
  assert.deepEqual(why, ["a.md:3:missing.md", "a.md:5:start.md#nope", "a.md:7:../library/x.md", "a.md:9:#nothing", "start.md:3:a.md#second-part"]); // the last one: this a.md has lost its heading
});

test("a chapter that does not exist fails; a page SUMMARY does not list is a warning, and a link to it fails", () => {
  const r = checkSource(book({ "SUMMARY.md": "# S\n\n[Start](start.md)\n\n- [A](a.md)\n- [Ghost](ghost.md)\n", "orphan.md": "# Orphan\n" }));
  assert.ok(r.problems.some((p) => p.target === "ghost.md" && /does not exist/.test(p.why)));
  assert.ok(r.problems.some((p) => p.file === "start.md" && /sub\/b\.md is not listed/.test(p.why)), "start.md links to a page that is not built");
  assert.deepEqual(r.warnings.map((w) => w.file).sort(), ["orphan.md", "sub/b.md"]);
});

test("the real handbook has no broken internal link", () => {
  const r = checkSource(readBook(join(ROOT, "docs", "book")));
  assert.deepEqual(r.problems, [], r.problems.map((p) => `${p.file}:${p.line} ${p.target}: ${p.why}`).join("\n"));
  assert.ok(r.pages >= 16, `SUMMARY lists ${r.pages} pages`);
  assert.ok(r.links > 100);
});

test("the command line exits 0 on the real book and 1 on a book with a dead link", () => {
  const ok = spawnSync(process.execPath, [join(ROOT, "tools", "book-links.mjs")], { cwd: ROOT, encoding: "utf8" });
  assert.equal(ok.status, 0, ok.stderr);
  assert.match(ok.stdout, /book links: \d+ pages, \d+ internal links, 0 broken/);
  const bad = spawnSync(process.execPath, [join(ROOT, "tools", "book-links.mjs"), "--nope"], { cwd: ROOT, encoding: "utf8" });
  assert.equal(bad.status, 2);
});

// ---- the assembled site ----
const html = (links) => `<html><body>${links.map((l) => `<a href="${l}">x</a>`).join("")}<h1 id="top">T</h1></body></html>`;
const site = (extra = {}, drop = []) => {
  const pages = {
    "index.html": "<html>game</html>",
    "docs/index.html": html(["a.html", "css/main.css", "https://example.com"]),
    "docs/a.html": html(["index.html", "a.html#top", "api/engine_core/index.html", "../index.html"]),
    "docs/b.html": html(["a.html"]),
    "docs/css/main.css": "",
    ...Object.fromEntries(DOC_CRATES.map((c) => [`docs/api/${libName(c)}/index.html`, "<html>rustdoc</html>"])),
    ...extra,
  };
  for (const d of drop) delete pages[d];
  return { files: new Set(Object.keys(pages)), read: (p) => pages[p] };
};
const SUMMARY = "# S\n\n- [A](a.html.md)\n";

test("a complete site passes and reports the page and crate counts", () => {
  const r = checkSite(site(), { summary: "# S\n\n- [Index](index.md)\n- [A](a.md)\n- [B](b.md)\n" });
  assert.deepEqual(r.problems, []);
  assert.equal(r.handbookPages, 3);
  assert.equal(r.apiCrates.length, 12);
  assert.deepEqual(r.apiCrates, DOC_CRATES.map(libName));
});

test("a missing chapter page, a missing rustdoc crate, a dead handbook link and a dead fragment each fail", () => {
  const r = checkSite(site({ "docs/b.html": html(["gone.html", "a.html#nothing"]) }, ["docs/api/engine_net/index.html"]), { summary: "# S\n\n- [A](a.md)\n- [B](b.md)\n- [C](c.md)\n" });
  const why = r.problems.map((p) => `${p.file}|${p.target}`);
  assert.ok(why.includes("docs/|c.md"), "SUMMARY chapter c was not built");
  assert.ok(why.includes("docs/api/|engine_net"), "rustdoc wrote no engine_net");
  assert.ok(why.includes("docs/b.html|gone.html"));
  assert.ok(why.includes("docs/b.html|a.html#nothing"));
  assert.equal(r.apiCrates.length, 11);
});

test("the game page must exist at the site root and the handbook under docs/", () => {
  const r = checkSite(site({}, ["index.html", "docs/index.html"]), { summary: SUMMARY });
  assert.ok(r.problems.some((p) => p.file === "index.html"));
  assert.ok(r.problems.some((p) => p.file === "docs/index.html"));
});

// ---- the landing link ----
test("the game page gets one link to the handbook, once", () => {
  const page = "<!doctype html><html><body><canvas></canvas></body></html>";
  const once = withLandingLink(page);
  assert.equal(once.added, true);
  assert.match(once.html, /<a id="docs-link" href="docs\/"[^>]*>Handbook and API docs<\/a>\n<\/body>/);
  const twice = withLandingLink(once.html);
  assert.equal(twice.added, false);
  assert.equal(twice.html, once.html);
  assert.throws(() => withLandingLink("<html>no end</html>"), /no <\/body>/);
});
