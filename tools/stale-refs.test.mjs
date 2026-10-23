// Live stale-doc-ref gate (read-only, rewritten 2026-10-23).
// History: the 2026-10-09 pin listed 6 lines that named two research files,
// docs/research/HARD-SYSTEMS-CARDS.md and docs/research/DISCOVERED.md, and
// asserted the gap count stayed at 6. A mate, tools/stale-targets.test.mjs,
// pinned both files as missing and was deleted when this gate replaced them.
// Both files are gone (record: docs/archive/2026-10-08/research/
// speed-2026-10-06-E13-docs.md), so no live doc may name them again.
// docs/archive/** is frozen history and docs/plan/2026-10-18-docs-audit.md is
// the dated audit record of the old pin; both are excluded on purpose.
// Run: node --test tools/stale-refs.test.mjs
import assert from "node:assert/strict";
import { existsSync, readdirSync, readFileSync } from "node:fs";
import { dirname, join, relative, sep } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";

const REPO = join(dirname(fileURLToPath(import.meta.url)), "..");
const DOCS = join(REPO, "docs");
const DEAD_NAMES = ["HARD-SYSTEMS-CARDS.md", "DISCOVERED.md"];
const FROZEN = "docs/archive";
const DATED = "docs/plan/2026-10-18-docs-audit.md";
const RECORD = "docs/archive/2026-10-08/research/speed-2026-10-06-E13-docs.md";

function relPath(abs) {
  return relative(REPO, abs).split(sep).join("/");
}

export function liveDocFiles() {
  const out = [];
  const walk = (dir) => {
    for (const entry of readdirSync(dir, { withFileTypes: true })) {
      const abs = join(dir, entry.name);
      const rel = relPath(abs);
      if (entry.isDirectory()) {
        if (rel === FROZEN) continue;
        walk(abs);
      } else if (entry.name.endsWith(".md") && rel !== DATED) {
        out.push({ rel, abs });
      }
    }
  };
  walk(DOCS);
  return out;
}

export function deadNameHits(rel, text) {
  const hits = [];
  const lines = String(text).replace(/\r\n/g, "\n").split("\n");
  for (let i = 0; i < lines.length; i += 1) {
    for (const name of DEAD_NAMES) {
      if (lines[i].includes(name)) hits.push({ file: rel, line: i + 1, name });
    }
  }
  return hits;
}

export function scanLiveDocs() {
  const files = liveDocFiles();
  const hits = files.flatMap((file) => deadNameHits(file.rel, readFileSync(file.abs, "utf8")));
  return { files: files.length, hits };
}

test("dead-name matcher reports both names with a line number", () => {
  const text = "live line\nsee docs/research/DISCOVERED.md and HARD-SYSTEMS-CARDS.md:22\n";
  assert.deepEqual(deadNameHits("docs/x.md", text), [
    { file: "docs/x.md", line: 2, name: "HARD-SYSTEMS-CARDS.md" },
    { file: "docs/x.md", line: 2, name: "DISCOVERED.md" },
  ]);
});

test("no live doc names an archived research file", () => {
  const scan = scanLiveDocs();
  console.log(`LIVE-SCAN docs: ${scan.files} markdown files (${FROZEN}/** and ${DATED} excluded)`);
  for (const hit of scan.hits) console.log(`STALE ${hit.file}:${hit.line} -> ${hit.name}`);
  console.log(`LIVE-SCAN dead names: ${scan.hits.length} hit(s) in live docs`);
  assert.ok(scan.files > 100, `scan saw only ${scan.files} markdown files; the walk is broken`);
  assert.deepEqual(scan.hits, []);
});

test("the deletion record cited by steals.md exists and lists both moves", () => {
  const abs = join(REPO, RECORD);
  assert.ok(existsSync(abs), `${RECORD} is missing; fix the citation in docs/plan/steals.md`);
  const text = readFileSync(abs, "utf8");
  for (const name of DEAD_NAMES) {
    assert.ok(text.includes(name), `${RECORD} names no ${name}; fix the citation in docs/plan/steals.md`);
  }
});

test("the skipped dated audit still names both files, so the scan is not vacuous", () => {
  const hits = deadNameHits(DATED, readFileSync(join(REPO, DATED), "utf8"));
  assert.deepEqual([...new Set(hits.map((hit) => hit.name))].sort(), [...DEAD_NAMES].sort());
});
