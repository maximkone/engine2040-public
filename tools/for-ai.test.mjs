// Run: node --test tools/for-ai.test.mjs
// DOC4: docs/book/for-ai.md and the root llms.txt are what an AI reads first, so they must not lie.
//  - each is under 200 lines;
//  - every path they name exists, every `node tools/x.mjs` names a real tool, every `cargo -p` names a real package and `--bin` a real binary;
//  - every door verb they name is a verb the endpoint source registers;
//  - the golden hash equals the one in docs/HASH-LAW.md; the page is linked from README.md, SUMMARY.md and llms.txt.
// Files the public mirror does not carry (the private rules files) are skipped there, not failed.
import assert from "node:assert/strict";
import { existsSync, readFileSync, readdirSync } from "node:fs";
import { dirname, join, posix } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { buildModel, readCrate } from "./door-reference.mjs";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const read = (p) => readFileSync(join(ROOT, p), "utf8").replace(/\r\n/g, "\n");
const FILES = ["docs/book/for-ai.md", "llms.txt"];
const MAX_LINES = 200;
// The private tree has the exporter; the public mirror does not, and it also lacks the private files.
const ON_MIRROR = !existsSync(join(ROOT, "tools", "public-export.mjs"));
const PRIVATE = ["AGENTS.md", "CLAUDE.md", "sprint/", "team/", "archive/", "docs/archive/", "docs/EMPIRE-INBOX.md", "docs/ROYALTY.md"];
// Build outputs: named on purpose, absent from a fresh checkout.
const isOutput = (p) => /^(?:dist|target|out)\//.test(p);
const isPrivate = (p) => PRIVATE.some((d) => (d.endsWith("/") ? p.startsWith(d) : p === d));

// Backticked tokens and link targets that look like repository paths.
export function namedPaths(text) {
  const out = new Set();
  const looksLikePath = (t) => /^[\w.-]+(?:\/[\w.<>-]+)*\/?$/.test(t) && (t.includes("/") || /\.(?:md|mjs|toml|json|yml|txt)$/.test(t)) && !/^(?:https?:|\.\.?\/?$)/.test(t) && !t.includes("<") && !/^\d/.test(t);
  for (const m of text.matchAll(/`([^`\n]+)`/g)) if (looksLikePath(m[1].trim().replace(/[.,;:]+$/, ""))) out.add(m[1].trim().replace(/[.,;:]+$/, ""));
  return [...out];
}
export function linkTargets(text) {
  return [...text.matchAll(/\]\(([^)\s]+)\)/g)].map((m) => m[1]).filter((t) => !/^[a-z]+:/i.test(t) && !t.startsWith("#")).map((t) => t.split("#")[0]);
}
export function nodeCommands(text) {
  return [...text.matchAll(/\bnode (?:--test )?(tools\/[\w.-]+\.mjs)\b/g)].map((m) => m[1]);
}

const packageNames = () => {
  const names = new Set(["engine2040"]);
  for (const d of readdirSync(join(ROOT, "crates"))) {
    const toml = join(ROOT, "crates", d, "Cargo.toml");
    if (existsSync(toml)) { const m = /^name\s*=\s*"([^"]+)"/m.exec(readFileSync(toml, "utf8")); if (m) names.add(m[1]); }
  }
  return names;
};
const binNames = () => {
  const names = new Set();
  for (const d of readdirSync(join(ROOT, "crates"))) {
    const toml = join(ROOT, "crates", d, "Cargo.toml");
    if (existsSync(toml)) for (const m of readFileSync(toml, "utf8").matchAll(/\[\[bin\]\]\s*\nname\s*=\s*"([^"]+)"/g)) names.add(m[1]);
  }
  return names;
};

test("each file stays under 200 lines", () => {
  for (const f of FILES) assert.ok(read(f).split("\n").length < MAX_LINES, `${f} has ${read(f).split("\n").length} lines`);
});

test("every path the page and llms.txt name exists", () => {
  for (const f of FILES) {
    const text = read(f);
    const base = posix.dirname(f);
    const missing = [];
    for (const p of namedPaths(text)) {
      if ((isPrivate(p) && ON_MIRROR) || isOutput(p)) continue;
      if (!existsSync(join(ROOT, p)) && !existsSync(join(ROOT, "docs", "book", p)) && !existsSync(join(ROOT, "crates", "endpoint", "src", p))) missing.push(p);
    }
    for (const t of linkTargets(text)) {
      if (isPrivate(t) && ON_MIRROR) continue;
      if (!existsSync(join(ROOT, posix.normalize(posix.join(base, t))))) missing.push(`link ${t}`);
    }
    assert.deepEqual(missing, [], `${f} names paths that do not exist`);
    const count = namedPaths(text).length + linkTargets(text).length;
    assert.ok(count >= 8, `${f}: the path reader found the paths (${count})`);
  }
});

test("every node tool and cargo package the page names exists", () => {
  const pkgs = packageNames();
  const bins = binNames();
  for (const f of FILES) {
    const text = read(f);
    const tools = nodeCommands(text);
    assert.ok(tools.length >= 5, `${f}: found node commands`);
    for (const t of tools) assert.ok(existsSync(join(ROOT, t)), `${f}: ${t} does not exist`);
    for (const m of text.matchAll(/\bcargo [^`\n]*/g)) {
      for (const p of m[0].matchAll(/ -p (\S+)/g)) if (!p[1].startsWith("<")) assert.ok(pkgs.has(p[1]), `${f}: ${p[1]} is not a workspace package`);
      for (const b of m[0].matchAll(/--bin (\S+)/g)) assert.ok(bins.has(b[1]), `${f}: ${b[1]} is not a binary`);
    }
  }
  assert.ok(/cargo run --locked -- --verify/.test(read("docs/book/for-ai.md")), "the golden command is on the page");
});

test("every door verb named exists in the endpoint source", () => {
  const model = buildModel(readCrate(ROOT));
  const verbs = new Set([...model.core.map((v) => v.name), ...model.packs.flatMap((p) => p.verbs.map((v) => v.name))]);
  const packs = new Set(model.packs.map((p) => p.name));
  const text = read("docs/book/for-ai.md");
  const table = text.slice(text.indexOf("| Verb |"), text.indexOf("Arguments, ranges, refusals"));
  const named = [...table.matchAll(/^\| `([a-z_]+\.[a-z_.]+)` \|/gm)].map((m) => m[1]);
  assert.ok(named.length >= 10, "the verb table was found");
  for (const v of named) assert.ok(verbs.has(v), `${v} is not a registered verb`);
  for (const m of table.matchAll(/\(`([a-z_]+)`(?:, `[a-z_]+`)*, \.\.\.\)/g)) assert.ok(packs.has(m[1]), `${m[1]} is not a pack`);
  const llm = read("llms.txt");
  for (const m of llm.matchAll(/Start with ((?:`[a-z_.]+`(?:, )?)+)/g)) for (const v of m[1].match(/[a-z_.]+/g)) assert.ok(verbs.has(v), `llms.txt names ${v}, which is not a verb`);
});

test("the golden hash is the one in HASH-LAW.md and the page is linked from the entry points", () => {
  const golden = /GOLDEN_600=([0-9a-f]{16})/.exec(read("docs/HASH-LAW.md"))[1];
  for (const f of FILES) assert.ok(read(f).includes(golden), `${f} names the golden ${golden}`);
  assert.match(read("README.md"), /docs\/book\/for-ai\.md/);
  assert.match(read("docs/book/SUMMARY.md"), /\(for-ai\.md\)/);
  assert.match(read("llms.txt"), /\(docs\/book\/for-ai\.md\)/);
});

test("the path reader finds a made-up path and ignores prose", () => {
  assert.deepEqual(namedPaths("see `docs/nope/none.md` and `cargo test` and `d57b5c8ae3096614` and `tools/x.mjs`"), ["docs/nope/none.md", "tools/x.mjs"]);
  assert.deepEqual(nodeCommands("run `node tools/proof.mjs --skip-build` and `node --test tools/for-ai.test.mjs`"), ["tools/proof.mjs", "tools/for-ai.test.mjs"]);
  assert.deepEqual(linkTargets("[a](docs/a.md#x) [b](https://x.y/z) [c](#top)"), ["docs/a.md"]);
});
