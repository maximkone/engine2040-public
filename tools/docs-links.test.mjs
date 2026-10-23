// Run: node --test tools/docs-links.test.mjs
import assert from "node:assert/strict";
import { mkdirSync, mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { extractLinks, findBroken } from "./docs-links.mjs";

test("extractLinks keeps relative targets and drops schemes, anchors and placeholders", () => {
  const text = [
    "[ok](a/b.md) and [web](https://x.y/z)",
    "[anchor](#now) [rust](crate::Mixer) [api](api/engine_core/index.html)",
    "[frag](docs/README.md#rules) [space](a%20b.md)",
    "Module['addRunDependency']('datafile_...')",
  ].join("\n");
  assert.deepEqual(extractLinks(text), [
    { target: "a/b.md", line: 1 },
    { target: "docs/README.md#rules", line: 3 },
    { target: "a%20b.md", line: 3 },
  ]);
});

test("findBroken resolves from the file's own folder and reports the line", () => {
  const root = mkdtempSync(join(tmpdir(), "docs-links-"));
  mkdirSync(join(root, "docs"), { recursive: true });
  writeFileSync(join(root, "docs", "a.md"), "[ok](../README.md)\n[bad](./gone.md)\n");
  writeFileSync(join(root, "README.md"), "# x\n");
  const broken = findBroken(root, [{ file: "docs/a.md", text: "[ok](../README.md)\n[bad](./gone.md)\n" }]);
  assert.deepEqual(broken.map((b) => [b.file, b.line, b.target]), [["docs/a.md", 2, "./gone.md"]]);
});

test("findBroken accepts a folder target and a fragment-only file", () => {
  const root = mkdtempSync(join(tmpdir(), "docs-links-"));
  mkdirSync(join(root, "docs", "sub"), { recursive: true });
  writeFileSync(join(root, "docs", "sub", "x.md"), "");
  const docs = [{ file: "docs/a.md", text: "[dir](sub/)\n[file](sub/x.md#top)\n" }];
  assert.deepEqual(findBroken(root, docs), []);
});
