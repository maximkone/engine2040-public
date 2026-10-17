#!/usr/bin/env node
// book-links: the handbook (docs/book, built by mdBook) must not ship a broken internal link.
//   node tools/book-links.mjs                       check the book sources: SUMMARY, every page, every relative link and #fragment
//   node tools/book-links.mjs --site <dir>          check the assembled Pages site: game at <dir>/, handbook at <dir>/docs/, rustdoc at <dir>/docs/api/
//   node tools/book-links.mjs --site <dir> --markdown   the same, with the counts as a markdown table for the job summary
//   node tools/book-links.mjs --landing <index.html>    add the handbook link to the game page (a no-op when it is there)
// Exit 0 = nothing broken, 1 = a problem (each is printed as file:line and the reason), 2 = bad arguments.
// mdBook builds a page whose link is dead without a word, so this is the link check (DOC1). It reads the same ids mdBook gives
// headings (lowercase, spaces to dashes, only letters, digits, - and _), so a `page.md#section` link is checked against the page.
// Links to files outside docs/book are problems: the published site has only the book, the game and rustdoc.
import { existsSync, readFileSync, readdirSync, statSync, writeFileSync } from "node:fs";
import { dirname, join, posix, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
export const BOOK_DIR = "docs/book";
// The crates a game author touches: pages.yml runs `cargo doc --no-deps` for exactly these and the book's api.md lists them.
export const DOC_CRATES = ["engine-core", "engine-arena", "engine-combat", "engine-physics", "engine-physfx", "engine-anim", "engine-render", "engine-game", "engine-net", "engine-assets", "engine-input", "engine-endpoint"];
export const libName = (crate) => crate.replaceAll("-", "_");
// A book page may link into rustdoc by this prefix; the files exist only after the site is assembled (--site checks them).
export const API_PREFIX = "api/";

// ---- markdown reading ----
// The text with fenced code blocks blanked (same length), so a link or heading inside an example does not count.
// With spans set, inline code spans are blanked too (a link needs real brackets, a heading id keeps its code text).
export function maskCode(text, spans = true) {
  const lines = text.replace(/\r\n/g, "\n").split("\n");
  let fence = null;
  const out = lines.map((line) => {
    const f = /^\s{0,3}(`{3,}|~{3,})/.exec(line);
    if (fence) { if (f && f[1][0] === fence[0] && f[1].length >= fence.length && /^\s*[`~]+\s*$/.test(line)) fence = null; return " ".repeat(line.length); }
    if (f) { fence = f[1]; return " ".repeat(line.length); }
    return spans ? line.replace(/(`+)([^`]|[^`][^]*?[^`])\1(?!`)/g, (m) => " ".repeat(m.length)) : line;
  });
  return out.join("\n");
}

// mdBook's heading id: letters, digits, - and _ kept (lowercased), whitespace to -, everything else dropped.
export const slug = (text) => [...text].filter((c) => /[\p{L}\p{N}_\-\s]/u.test(c)).map((c) => (/\s/.test(c) ? "-" : c.toLowerCase())).join("");

// Every anchor a page offers: heading ids (repeats get -1, -2, ...), {#custom} ids and html id attributes.
export function pageIds(text) {
  const ids = new Set();
  const seen = new Map();
  const masked = maskCode(text, false);
  for (const line of masked.split("\n")) {
    const h = /^\s{0,3}#{1,6}\s+(.*?)\s*#*\s*$/.exec(line);
    if (!h) continue;
    let body = h[1].replace(/`/g, "");
    const custom = /\{#([^}\s]+)[^}]*\}\s*$/.exec(body);
    if (custom) { ids.add(custom[1]); continue; } // an explicit id replaces the generated one
    body = body.replace(/!?\[([^\]]*)\]\([^)]*\)/g, "$1").replace(/<[^>]+>/g, "").replace(/[*~]/g, "");
    const base = slug(body);
    const n = seen.get(base) ?? 0;
    seen.set(base, n + 1);
    ids.add(n === 0 ? base : `${base}-${n}`);
  }
  for (const m of masked.matchAll(/\bid="([^"]+)"/g)) ids.add(m[1]);
  return ids;
}

// Link targets of one page with their 1-based lines: inline links and images, reference definitions, html href and src.
export function pageLinks(text) {
  const masked = maskCode(text);
  const out = [];
  const lineAt = (i) => masked.slice(0, i).split("\n").length;
  for (const m of masked.matchAll(/!?\[(?:[^\[\]]|\[[^\]]*\])*\]\(\s*(<[^>]+>|[^)\s]+)(?:\s+"[^"]*")?\s*\)/g)) out.push({ target: m[1].replace(/^<|>$/g, ""), line: lineAt(m.index) });
  for (const m of masked.matchAll(/^\s{0,3}\[[^\]]+\]:\s*(\S+)/gm)) out.push({ target: m[1].replace(/^<|>$/g, ""), line: lineAt(m.index) });
  for (const m of masked.matchAll(/\b(?:href|src)="([^"]+)"/g)) out.push({ target: m[1], line: lineAt(m.index) });
  return out.sort((a, b) => a.line - b.line);
}

const isExternal = (t) => /^[a-z][a-z0-9+.-]*:/i.test(t) || t.startsWith("//");

// The chapters of SUMMARY.md in order: [{ title, path }].
export function chapters(summary) {
  const out = [];
  for (const m of summary.replace(/\r\n/g, "\n").matchAll(/^\s*(?:[-*]\s+)?\[([^\]]*)\]\(([^)\s]+)\)\s*$/gm)) out.push({ title: m[1], path: m[2] });
  return out;
}

// ---- source check ----
// files: Map relative path (forward slashes, inside docs/book) -> text for .md, anything for other files. Returns { pages, links, problems }.
export function checkSource(files) {
  const problems = [];
  const warnings = [];
  const mdPaths = [...files.keys()].filter((p) => p.endsWith(".md")).sort();
  const summary = files.get("SUMMARY.md");
  if (summary === undefined) return { pages: 0, links: 0, warnings, problems: [{ file: "SUMMARY.md", line: 1, target: "", why: "the book has no SUMMARY.md" }] };
  const listed = chapters(summary);
  const listedPaths = new Set(listed.map((c) => c.path));
  for (const c of listed) if (!files.has(c.path)) problems.push({ file: "SUMMARY.md", line: lineOfText(summary, c.path), target: c.path, why: "the chapter file does not exist" });
  // A page SUMMARY does not list is never built; that is a warning here and a dead link when a built page points at it.
  for (const p of mdPaths) if (p !== "SUMMARY.md" && !listedPaths.has(p)) warnings.push({ file: p, line: 1, target: p, why: "the page is not listed in SUMMARY.md, so mdBook does not build it" });
  const idsOf = new Map();
  const ids = (p) => { if (!idsOf.has(p)) idsOf.set(p, pageIds(files.get(p))); return idsOf.get(p); };
  let links = 0;
  for (const p of mdPaths) {
    for (const { target, line } of pageLinks(files.get(p))) {
      if (isExternal(target)) continue;
      links++;
      const [pathPart, frag = ""] = splitFragment(target);
      if (pathPart === "") { if (frag && !ids(p).has(frag)) problems.push({ file: p, line, target, why: "no heading or id with that name on this page" }); continue; }
      const dest = posix.normalize(posix.join(posix.dirname(p), pathPart.split("?")[0]));
      if (dest.startsWith("../") || dest === "..") { problems.push({ file: p, line, target, why: "leaves docs/book: the published site has only the book, the game and rustdoc; name the path in code or use a full URL" }); continue; }
      if (dest.startsWith(API_PREFIX)) continue;
      if (!files.has(dest)) { problems.push({ file: p, line, target, why: "the file does not exist in docs/book" }); continue; }
      if (dest.endsWith(".md") && listedPaths.has(p) && !listedPaths.has(dest)) { problems.push({ file: p, line, target, why: `${dest} is not listed in SUMMARY.md, so mdBook does not build it and the link is dead on the site` }); continue; }
      if (frag && dest.endsWith(".md") && !ids(dest).has(frag)) problems.push({ file: p, line, target, why: `${dest} has no heading or id "${frag}"` });
    }
  }
  return { pages: listed.length, links, warnings, problems };
}

const splitFragment = (t) => { const i = t.indexOf("#"); return i < 0 ? [t, ""] : [t.slice(0, i), t.slice(i + 1)]; };
const lineOfText = (text, needle) => { const i = text.indexOf(needle); return i < 0 ? 1 : text.slice(0, i).split("\n").length; };

// ---- site check ----
// site: { files: Set of relative paths (forward slashes), read(path) -> text }. The handbook is <site>/docs, rustdoc <site>/docs/api.
export function checkSite(site, { crates = DOC_CRATES, summary } = {}) {
  const problems = [];
  const has = (p) => site.files.has(p);
  if (!has("index.html")) problems.push({ file: "index.html", line: 1, target: "", why: "the game page is missing at the site root" });
  if (!has("docs/index.html")) problems.push({ file: "docs/index.html", line: 1, target: "", why: "the handbook is missing under docs/" });
  const chapterList = summary ? chapters(summary) : [];
  const pagesFound = chapterList.filter((c) => has(`docs/${c.path.replace(/\.md$/, ".html")}`));
  for (const c of chapterList) if (!has(`docs/${c.path.replace(/\.md$/, ".html")}`)) problems.push({ file: "docs/", line: 1, target: c.path, why: "mdBook did not build this SUMMARY chapter" });
  const apiCrates = crates.filter((c) => has(`docs/api/${libName(c)}/index.html`));
  for (const c of crates) if (!has(`docs/api/${libName(c)}/index.html`)) problems.push({ file: "docs/api/", line: 1, target: libName(c), why: "rustdoc wrote no index.html for this crate" });
  const idsOf = new Map();
  const htmlIds = (p) => { if (!idsOf.has(p)) idsOf.set(p, new Set([...site.read(p).matchAll(/\b(?:id|name)="([^"]+)"/g)].map((m) => m[1]))); return idsOf.get(p); };
  let links = 0;
  const handbook = [...site.files].filter((p) => p.endsWith(".html") && p.startsWith("docs/") && !p.startsWith(`docs/${API_PREFIX}`) && !/(?:^|\/)(?:print|404|toc)\.html$/.test(p)).sort();
  for (const p of handbook) {
    const text = site.read(p);
    for (const m of text.matchAll(/\b(?:href|src)="([^"]*)"/g)) {
      const raw = m[1].replace(/&amp;/g, "&");
      if (raw === "" || isExternal(raw) || raw.startsWith("data:")) continue;
      links++;
      const line = text.slice(0, m.index).split("\n").length;
      const [pathPart, frag = ""] = splitFragment(raw);
      let dest = pathPart === "" ? p : pathPart.startsWith("/") ? pathPart.slice(1) : posix.normalize(posix.join(posix.dirname(p), pathPart.split("?")[0]));
      if (dest.endsWith("/")) dest += "index.html";
      if (!has(dest) && has(`${dest}/index.html`)) dest += "/index.html";
      if (!has(dest)) { problems.push({ file: p, line, target: raw, why: "no such file in the assembled site" }); continue; }
      if (frag && dest.endsWith(".html") && !htmlIds(dest).has(frag)) problems.push({ file: p, line, target: raw, why: `${dest} has no id "${frag}"` });
    }
  }
  return { handbookPages: pagesFound.length, chapters: chapterList.length, apiCrates: apiCrates.map(libName), links, problems };
}

// ---- landing link ----
export const LANDING_LINK = '<a id="docs-link" href="docs/" style="position:fixed;right:8px;top:8px;z-index:30;padding:2px 8px;background:rgba(0,0,0,0.65);color:#e8e8e8;border-radius:3px;font:12px ui-monospace,Menlo,Consolas,monospace;text-decoration:none">Handbook and API docs</a>';
// The game page with a link to the handbook before </body>; unchanged when it already links to docs/.
export function withLandingLink(html) {
  if (/href="docs\/"/.test(html)) return { html, added: false };
  if (!html.includes("</body>")) throw new Error("the game page has no </body> to put the handbook link before");
  return { html: html.replace("</body>", () => `${LANDING_LINK}\n</body>`), added: true };
}

// ---- files ----
export function readBook(dir) {
  const files = new Map();
  const walk = (d) => {
    for (const e of readdirSync(d, { withFileTypes: true })) {
      const p = join(d, e.name);
      if (e.isDirectory()) walk(p);
      else files.set(p.slice(dir.length + 1).split(sep).join("/"), e.name.endsWith(".md") ? readFileSync(p, "utf8") : "");
    }
  };
  walk(dir);
  return files;
}

export function readSite(dir) {
  const files = new Set();
  const walk = (d) => {
    for (const e of readdirSync(d, { withFileTypes: true })) {
      const p = join(d, e.name);
      if (e.isDirectory()) walk(p);
      else files.add(p.slice(dir.length + 1).split(sep).join("/"));
    }
  };
  walk(dir);
  return { files, read: (p) => readFileSync(join(dir, ...p.split("/")), "utf8") };
}

const show = (problems) => problems.map((p) => `${p.file}:${p.line}: ${p.target ? `${p.target}: ` : ""}${p.why}`).join("\n");

function main(argv) {
  const opt = { src: join(ROOT, BOOK_DIR), site: null, markdown: false, landing: null, crates: DOC_CRATES };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (a === "--src") opt.src = resolve(argv[++i] ?? "");
    else if (a === "--site") opt.site = resolve(argv[++i] ?? "");
    else if (a === "--landing") opt.landing = resolve(argv[++i] ?? "");
    else if (a === "--crates") opt.crates = (argv[++i] ?? "").split(",").filter(Boolean);
    else if (a === "--markdown") opt.markdown = true;
    else { console.error(`book-links: unknown argument ${a}\nusage: node tools/book-links.mjs [--src docs/book] [--site <dir> [--markdown] [--crates a,b]] [--landing <index.html>]`); return 2; }
  }
  if (opt.landing) {
    const { html, added } = withLandingLink(readFileSync(opt.landing, "utf8"));
    if (added) writeFileSync(opt.landing, html);
    console.log(added ? `landing link added to ${opt.landing}` : `landing link already in ${opt.landing}`);
    return 0;
  }
  if (!existsSync(opt.src) || !statSync(opt.src).isDirectory()) { console.error(`book-links: ${opt.src} is not a folder`); return 2; }
  if (opt.site) {
    const summary = readFileSync(join(opt.src, "SUMMARY.md"), "utf8");
    const r = checkSite(readSite(opt.site), { crates: opt.crates, summary });
    if (opt.markdown) {
      console.log("### Docs site\n\n| Part | URL path | Count |\n|---|---|---|\n| Game | `/` | 1 page |");
      console.log(`| Handbook (mdBook) | \`/docs/\` | ${r.handbookPages} of ${r.chapters} chapters built |`);
      console.log(`| Rustdoc | \`/docs/api/\` | ${r.apiCrates.length} of ${opt.crates.length} crates: ${r.apiCrates.map((c) => `\`${c}\``).join(", ")} |`);
      console.log(`| Links checked in the handbook | | ${r.links}, broken: ${r.problems.length} |`);
    } else {
      console.log(`handbook: ${r.handbookPages} of ${r.chapters} chapters built under docs/`);
      console.log(`rustdoc: ${r.apiCrates.length} of ${opt.crates.length} crates under docs/api/: ${r.apiCrates.join(", ")}`);
      console.log(`links: ${r.links} checked in the handbook, ${r.problems.length} broken`);
    }
    if (r.problems.length) { console.error(show(r.problems)); return 1; }
    return 0;
  }
  const r = checkSource(readBook(opt.src));
  console.log(`book links: ${r.pages} pages, ${r.links} internal links, ${r.problems.length} broken, ${r.warnings.length} warnings`);
  if (r.warnings.length) console.log(show(r.warnings.map((w) => ({ ...w, why: `warning: ${w.why}` }))));
  if (r.problems.length) { console.error(show(r.problems)); return 1; }
  return 0;
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) process.exitCode = main(process.argv.slice(2));
