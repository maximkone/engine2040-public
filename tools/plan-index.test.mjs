// Run: node --test tools/plan-index.test.mjs
// docs/plan/README.md is the one plan index (owner 2026-10-10): every .md file
// in docs/plan has exactly one line `file - tag - proof - scope`, and a plan that
// owns a row of FOCUS.md "## Now" is tagged NOW.
import assert from "node:assert/strict";
import { readdirSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";
import { focusNowIds } from "./board-slice.mjs";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const PLAN_DIR = join(ROOT, "docs", "plan");
const LINE = /^- `([^`]+\.md)` — (NOW|NEXT|LATER) — ([^—]+?) — (\S.*)$/;
const PROOFS = ["Look", "Bodies", "AI door", "Multiplayer", "all four"];

function indexLines() {
  const text = readFileSync(join(PLAN_DIR, "README.md"), "utf8").replace(/\r\n/g, "\n");
  const start = text.indexOf("## Plan index");
  assert.ok(start >= 0, "README.md has no '## Plan index' section");
  const end = text.indexOf("\n## ", start + 4);
  const section = text.slice(start, end < 0 ? text.length : end);
  return section.split("\n").filter((l) => l.startsWith("- `")).map((l) => {
    const m = LINE.exec(l);
    assert.ok(m, `index line does not read 'file — NOW|NEXT|LATER — proof — scope': ${l.slice(0, 100)}`);
    return { file: m[1], tag: m[2], proof: m[3].trim(), scope: m[4].trim() };
  });
}

test("every docs/plan/*.md file has exactly one index line, and no line names a missing file", () => {
  const files = readdirSync(PLAN_DIR).filter((n) => n.endsWith(".md")).sort();
  const lines = indexLines();
  const names = lines.map((l) => l.file);
  assert.deepEqual([...new Set(names)].sort(), files, "index and directory must list the same files");
  assert.equal(names.length, new Set(names).size, "a file is listed twice");
});

test("tags and proofs agree: LATER feeds none, NOW and NEXT feed a Core proof", () => {
  for (const l of indexLines()) {
    if (l.tag === "LATER") {
      assert.equal(l.proof, "none", `${l.file} is LATER, so its proof is none`);
    } else {
      const parts = l.proof.split(",").map((p) => p.trim());
      assert.ok(parts.every((p) => PROOFS.includes(p)), `${l.file}: proof '${l.proof}' must be from ${PROOFS.join(", ")}`);
    }
  }
});

test("a plan that owns a FOCUS '## Now' row is tagged NOW", () => {
  const ids = focusNowIds(readFileSync(join(ROOT, "docs", "FOCUS.md"), "utf8"));
  assert.ok(ids.length > 0, "FOCUS.md has no Now rows");
  const tags = new Map(indexLines().map((l) => [l.file, l.tag]));
  for (const [file, tag] of tags) {
    if (file === "ATLAS.md") continue; // generated copy of every row
    const text = readFileSync(join(PLAN_DIR, file), "utf8");
    const owned = ids.filter((id) => new RegExp(`^- \\[.\\] ${id}\\b`, "m").test(text));
    if (owned.length) assert.equal(tag, "NOW", `${file} owns Now row(s) ${owned.join(" ")} so it must be tagged NOW`);
  }
});
