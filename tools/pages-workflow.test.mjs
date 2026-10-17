// Run: node --test tools/pages-workflow.test.mjs
// DOC1: .github/workflows/pages.yml builds the game, the handbook (mdBook) and rustdoc into one Pages site.
// There is no YAML library in the repo, so this file carries a small block-YAML reader (maps, lists, scalars, quoted strings,
// flow lists, | and > blocks) that is strict where GitHub is strict: tabs, a repeated key and a plain value that holds ": " are errors.
// Then it checks the workflow's shape and that every run block is valid bash.
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { DOC_CRATES, libName } from "./book-links.mjs";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");

// ---- a small block-YAML reader ----
export function parseYaml(text) {
  const lines = text.replace(/\r\n/g, "\n").split("\n");
  let at = 0;
  const fail = (msg, i = at) => { throw new Error(`yaml line ${i + 1}: ${msg}`); };
  const indentOf = (l) => { const m = /^( *)/.exec(l); if (/^ *\t/.test(l)) fail("a tab in the indentation"); return m[1].length; };
  const blank = (l) => /^\s*(#.*)?$/.test(l);
  const next = () => { while (at < lines.length && blank(lines[at])) at++; return at < lines.length ? at : -1; };

  const stripComment = (s) => {
    let q = null;
    for (let i = 0; i < s.length; i++) {
      const c = s[i];
      if (q) { if (c === "\\" && q === '"') i++; else if (c === q) q = null; } else if (c === '"' || c === "'") { if (i === 0 || /[\s\[,{]/.test(s[i - 1])) q = c; } else if (c === "#" && (i === 0 || /\s/.test(s[i - 1]))) return s.slice(0, i).trimEnd();
    }
    return s.trimEnd();
  };
  const quoted = (s, i) => {
    if (s[0] === '"') { try { return JSON.parse(s); } catch { fail(`bad double-quoted string ${s}`, i); } }
    return s.slice(1, -1).replace(/''/g, "'");
  };
  const scalar = (raw, i) => {
    const s = stripComment(raw).trim();
    if (s === "") return null;
    if (s[0] === '"' || s[0] === "'") { if (s[s.length - 1] !== s[0] || s.length < 2) fail(`unterminated quoted string ${s}`, i); return quoted(s, i); }
    if (s[0] === "[") {
      if (s[s.length - 1] !== "]") fail("unterminated flow list", i);
      const body = s.slice(1, -1).trim();
      return body === "" ? [] : body.split(",").map((p) => scalar(p, i));
    }
    if (s === "{}") return {};
    if (/: |:$/.test(s)) fail(`a plain value cannot hold ": " (quote it): ${s}`, i);
    return s;
  };
  const block = (parentIndent, i) => {
    // | or > scalar: the following lines more indented than the key.
    const out = [];
    let ind = null;
    while (at < lines.length) {
      const l = lines[at];
      if (l.trim() === "") { out.push(""); at++; continue; }
      const n = indentOf(l);
      if (n <= parentIndent) break;
      ind ??= n;
      if (n < ind) fail("a block line is less indented than the first", at);
      out.push(l.slice(ind));
      at++;
    }
    while (out.length && out[out.length - 1] === "") out.pop();
    return `${out.join("\n")}\n`;
  };
  const mapAt = (indent) => {
    const obj = {};
    for (;;) {
      const i = next();
      if (i < 0 || indentOf(lines[i]) !== indent || /^\s*-(\s|$)/.test(lines[i])) break;
      const m = /^ *([^\s:#][^:]*?)\s*:(?:\s+(.*))?$/.exec(lines[i]);
      if (!m) fail(`not a "key: value" line: ${lines[i].trim()}`, i);
      const key = m[1].replace(/^["']|["']$/g, "");
      if (Object.hasOwn(obj, key)) fail(`the key ${key} appears twice`, i);
      at = i + 1;
      const rest = m[2] === undefined ? "" : stripComment(m[2]).trim();
      if (/^[|>][+-]?$/.test(rest)) obj[key] = block(indent, i);
      else if (rest === "") {
        const j = next();
        if (j >= 0 && (indentOf(lines[j]) > indent || (indentOf(lines[j]) === indent && /^\s*-(\s|$)/.test(lines[j])))) obj[key] = node(indentOf(lines[j]));
        else obj[key] = null;
      } else obj[key] = scalar(rest, i);
    }
    return obj;
  };
  const seqAt = (indent) => {
    const arr = [];
    for (;;) {
      const i = next();
      if (i < 0 || indentOf(lines[i]) !== indent || !/^ *-(\s|$)/.test(lines[i])) break;
      const rest = lines[i].slice(indent + 1);
      const body = rest.trim();
      if (body === "") { at = i + 1; const j = next(); arr.push(j >= 0 && indentOf(lines[j]) > indent ? node(indentOf(lines[j])) : null); continue; }
      const offset = indent + 1 + (rest.length - rest.trimStart().length);
      if (/^[^\s"'\[{][^:]*?:(\s|$)/.test(body) && !body.startsWith("- ")) { lines[i] = `${" ".repeat(offset)}${rest.trimStart()}`; arr.push(mapAt(offset)); } else { at = i + 1; arr.push(scalar(body, i)); }
    }
    return arr;
  };
  const node = (indent) => {
    const i = next();
    if (i < 0) return null;
    return /^ *-(\s|$)/.test(lines[i]) ? seqAt(indent) : mapAt(indent);
  };
  const first = next();
  if (first < 0) return null;
  const root = node(indentOf(lines[first]));
  if (next() >= 0) fail("content after the document (bad indentation)");
  return root;
}

// ---- the reader itself ----
test("the yaml reader reads maps, lists, blocks and flow lists, and is strict like GitHub", () => {
  const doc = parseYaml('name: x\non:\n  push:\n    branches: [main, "web-build-*"]\nenv:\n  A: "1"\nsteps:\n  - uses: a/b@v1\n    with:\n      k: v # c\n  - name: run it\n    run: |\n      echo "a: b"\n      exit 0\n  - plain\n');
  assert.equal(doc.name, "x");
  assert.deepEqual(doc.on.push.branches, ["main", "web-build-*"]);
  assert.equal(doc.env.A, "1");
  assert.deepEqual(doc.steps[0], { uses: "a/b@v1", with: { k: "v" } });
  assert.equal(doc.steps[1].run, 'echo "a: b"\nexit 0\n');
  assert.equal(doc.steps[2], "plain");
  assert.throws(() => parseYaml("a: 1\na: 2\n"), /appears twice/);
  assert.throws(() => parseYaml("run: echo a: b\n"), /cannot hold/);
  assert.throws(() => parseYaml("a:\n\t- x\n"), /tab/);
  assert.throws(() => parseYaml("a: 1\n  b: 2\n"), /bad indentation|not a/);
});

// ---- the workflow ----
const text = readFileSync(join(ROOT, ".github", "workflows", "pages.yml"), "utf8");
const wf = parseYaml(text);
const steps = (job) => wf.jobs[job].steps;
const stepNamed = (job, re) => steps(job).find((s) => re.test(s.name ?? s.uses ?? ""));
const indexOfStep = (job, re) => steps(job).findIndex((s) => re.test(s.name ?? s.uses ?? ""));

test("pages.yml parses and keeps its triggers, permissions and two jobs", () => {
  assert.equal(wf.name, "pages");
  assert.ok("workflow_dispatch" in wf.on);
  assert.deepEqual(wf.on.push.branches, ["main"]);
  assert.equal(wf.permissions.pages, "write");
  assert.deepEqual(Object.keys(wf.jobs), ["build", "deploy"]);
  assert.match(wf.jobs.build.if, /maximkone\/engine2040-public/);
});

test("the deploy job runs from the default branch only and checks the handbook and rustdoc answer", () => {
  const d = wf.jobs.deploy;
  assert.equal(d.needs, "build");
  assert.match(d.if, /github\.ref == 'refs\/heads\/main'/);
  assert.equal(d.environment.name, "github-pages");
  const check = d.steps.find((s) => /handbook and rustdoc answer 200/.test(s.name ?? ""));
  assert.ok(check, "a post-deploy step curls the handbook");
  for (const path of ["docs/", "docs/api/engine_game/index.html"]) assert.ok(check.run.includes(path), `the post-deploy check asks for ${path}`);
});

test("mdBook is one pinned release binary with its sha256 checked", () => {
  const s = stepNamed("build", /mdBook/);
  assert.match(s.env.MDBOOK_TAG, /^v\d+\.\d+\.\d+$/);
  assert.match(s.env.MDBOOK_SHA256, /^[0-9a-f]{64}$/);
  assert.match(s.run, /releases\/download\/\$MDBOOK_TAG\/mdbook-\$MDBOOK_TAG-x86_64-unknown-linux-gnu\.tar\.gz/);
  assert.match(s.run, /sha256sum -c/);
});

test("the site is the game at the root, the handbook under docs/ and rustdoc under docs/api/, in that order", () => {
  const order = ["mdBook", "handbook sources", "site - game at the root", "rustdoc for the crates", "site - links, page and crate counts", "configure-pages", "upload-pages-artifact"].map((re) => indexOfStep("build", new RegExp(re)));
  assert.ok(order.every((n) => n >= 0), `a step is missing: ${order}`);
  assert.deepEqual([...order].sort((a, b) => a - b), order, "the steps run in the stated order");
  const assemble = stepNamed("build", /site - game at the root/).run;
  assert.match(assemble, /cp -r dist\/web\/\. site\//);
  assert.match(assemble, /mdbook build docs\/book --dest-dir "\$GITHUB_WORKSPACE\/site\/docs"/);
  assert.match(assemble, /book-links\.mjs --landing site\/index\.html/);
  assert.match(stepNamed("build", /rustdoc for the crates/).run, /cp -r target\/doc\/\. site\/docs\/api\//);
  const up = stepNamed("build", /upload-pages-artifact/);
  assert.equal(up.with.path, "site");
});

test("a broken link fails the build: sources before mdbook, the assembled site before the upload", () => {
  const src = stepNamed("build", /handbook sources/).run;
  assert.match(src, /node tools\/book-links\.mjs\n/);
  assert.match(src, /node tools\/door-reference\.mjs --check/);
  assert.match(src, /set -euo pipefail/);
  const site = stepNamed("build", /links, page and crate counts/).run;
  assert.match(site, /node tools\/book-links\.mjs --site site\n/);
  assert.match(site, /book-links\.mjs --site site --markdown/);
  assert.match(site, /GITHUB_STEP_SUMMARY/);
  assert.match(site, /Rustdoc warnings: \$RUSTDOC_WARNINGS/);
});

test("cargo doc covers exactly the crates api.md lists and DOC_CRATES names", () => {
  const s = stepNamed("build", /rustdoc for the crates/);
  const cmd = s.run.split("\n").find((l) => l.includes("cargo doc"));
  const crates = [...cmd.matchAll(/ -p (\S+)/g)].map((m) => m[1]);
  assert.deepEqual(crates, DOC_CRATES);
  assert.match(cmd, /--no-deps/);
  assert.match(cmd, /--locked/);
  assert.match(cmd, /--lib/);
  // -D warnings only when rustdoc is clean today; otherwise the run prints the count and says where.
  const strict = /-D warnings/.test(String(s.env?.RUSTDOCFLAGS ?? ""));
  if (!strict) assert.match(s.run, /RUSTDOC_WARNINGS/);
  const api = readFileSync(join(ROOT, "docs", "book", "api.md"), "utf8");
  const listed = [...api.matchAll(/^- \[([a-z-]+)\]\(api\/([a-z_]+)\/index\.html\): (.*)$/gm)];
  assert.deepEqual(listed.map((m) => m[1]), DOC_CRATES, "api.md lists the same crates in the same order");
  for (const m of listed) {
    assert.equal(m[2], libName(m[1]), `${m[1]} links to its rustdoc folder`);
    const lib = readFileSync(join(ROOT, "crates", m[1].replace(/^engine-/, ""), "src", "lib.rs"), "utf8");
    // The first sentence of the crate docs: `//!` lines joined until one ends with a period.
    const docLines = lib.split("\n").filter((l) => l.startsWith("//!")).map((l) => l.slice(3).trim());
    const first = [];
    for (const l of docLines.slice(docLines.findIndex((x) => x !== ""))) { if (l === "") break; first.push(l); if (l.endsWith(".")) break; }
    assert.equal(m[3].trim(), first.join(" "), `${m[1]}: api.md quotes the first sentence of the crate's own docs`);
  }
});

test("llms.txt reaches the site root with every repository path turned into a link to the mirror", () => {
  const s = stepNamed("build", /site - game at the root/).run;
  const sed = /sed -E '([^']+)'/.exec(s);
  assert.ok(sed, "a sed rewrite of llms.txt");
  const [verb, pattern, replacement, flags] = sed[1].split("#");
  assert.equal(verb, "s");
  assert.equal(flags, "g");
  const out = readFileSync(join(ROOT, "llms.txt"), "utf8").replace(new RegExp(pattern, "g"), (all, p) => replacement.replace("\\1", p));
  const links = [...out.matchAll(/\]\(([^)]+)\)/g)].map((x) => x[1]);
  assert.ok(links.length > 15);
  for (const l of links) assert.match(l, /^https:\/\/github\.com\/maximkone\/engine2040-public\/blob\/main\/[\w./-]+$/, "no relative link survives");
});

test("every run block is valid bash", { skip: spawnSync("bash", ["--version"]).error ? "no bash on this machine" : false }, () => {
  for (const job of Object.values(wf.jobs)) {
    for (const s of job.steps) {
      if (!s.run) continue;
      const script = s.run.replace(/\$\{\{[^}]*\}\}/g, "X");
      const r = spawnSync("bash", ["-n"], { input: script, encoding: "utf8" });
      assert.equal(r.status, 0, `${s.name}: ${r.stderr}`);
    }
  }
});
