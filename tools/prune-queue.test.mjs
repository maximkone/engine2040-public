// Run: node --test tools/prune-queue.test.mjs
// prune-queue keeps done/ to newest 50 and parks stale ready packets.
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, readdirSync, rmSync, writeFileSync, utimesSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { pickDoneOverflow, pickStaleReady } from "./prune-queue.mjs";

const MIN = 60000;
const NOW = 1_760_000_000_000;

const mk = (n, ageMin, extra = {}) =>
  Array.from({ length: n }, (_, i) => ({ name: `f-${String(i).padStart(3, "0")}.md`, mtime: NOW - ageMin * MIN - i * 1000, ...extra }));

test("prune_overflow keeps newest 50", () => {
  const files = mk(60, 0);
  const over = pickDoneOverflow(files, 50);
  assert.equal(over.length, 10);
  const names = new Set(over.map((f) => f.name));
  for (let i = 50; i < 60; i++) assert.ok(names.has(`f-${String(i).padStart(3, "0")}.md`), `oldest ${i} overflows`);
  assert.ok(!names.has("f-000.md"), "newest stays");
  assert.equal(pickDoneOverflow(mk(20, 0), 50).length, 0);
});

test("prune_stale picks only over-age non-running", () => {
  const old = { name: "old.md", mtime: NOW - 13 * 60 * MIN };
  const edge = { name: "edge.md", mtime: NOW - 720 * MIN };
  const young = { name: "young.md", mtime: NOW - 60 * MIN };
  const twin = { name: "twin.md", mtime: NOW - 13 * 60 * MIN };
  const got = pickStaleReady([old, edge, young, twin], 720, ["twin.md"], NOW).map((f) => f.name);
  assert.deepEqual(got, ["old.md"]);
});

test("prune_dryrun moves nothing", () => {
  const dir = join(tmpdir(), `prune-drill-${Date.now()}-${Math.floor(Math.random() * 1e6)}`);
  const done = join(dir, "target", "queue", "done");
  const ready = join(dir, "target", "queue", "ready");
  const running = join(dir, "target", "queue", "running");
  mkdirSync(done, { recursive: true });
  mkdirSync(ready, { recursive: true });
  mkdirSync(running, { recursive: true });
  try {
    writeFileSync(join(done, "a.md"), "a\n");
    writeFileSync(join(done, "b.md"), "b\n");
    writeFileSync(join(ready, "old.md"), "---\nrole: builder\n---\nold\n");
    utimesSync(join(ready, "old.md"), new Date(), new Date(Date.now() - 13 * 60 * MIN));
    writeFileSync(join(ready, "fresh.md"), "fresh\n");
    const out = execFileSync(process.execPath, [join(process.cwd(), "tools", "prune-queue.mjs")], {
      encoding: "utf8",
      cwd: dir,
    });
    assert.match(out, /done 2 keep 50 archive 0/);
    assert.match(out, /ready 2 stale 1/);
    assert.match(out, /moved nothing/);
    assert.ok(existsSync(join(done, "a.md")) && existsSync(join(done, "b.md")), "done untouched");
    assert.ok(existsSync(join(ready, "old.md")) && existsSync(join(ready, "fresh.md")), "ready untouched");
    assert.equal(readdirSync(done).length, 2);
    assert.equal(readdirSync(ready).length, 2);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});
