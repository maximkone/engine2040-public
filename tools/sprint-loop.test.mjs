import assert from "node:assert/strict";
import test from "node:test";
import { spawnSync } from "node:child_process";
import { existsSync, mkdirSync, mkdtempSync, readdirSync, rmSync, statSync, utimesSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { archivedDoneRows, loadBoardText, parseBoard, rowReady } from "./board-slice.mjs";

const TOOL = fileURLToPath(new URL("./sprint-loop.mjs", import.meta.url));
const ROOT = fileURLToPath(new URL("..", import.meta.url));
const run = (args) => spawnSync(process.execPath, [TOOL, ...args], { encoding: "utf8", cwd: ROOT });

// Independent oracle from the one shared row parser: the loop must report
// exactly what board-slice.mjs sees, never a second copy of the grammar.
function boardOracle() {
  const text = loadBoardText(undefined, { receipts: false });
  const { tasks, order } = parseBoard(text, archivedDoneRows());
  const ready = order.map((id) => tasks.get(id)).filter((t) => rowReady(tasks, t)).map((t) => t.id);
  const done = [...tasks.values()].filter((t) => t.done).length;
  return { done, total: tasks.size, ready };
}

test("--dry-run reports the oracle board state and exits 0", () => {
  const { done, total, ready } = boardOracle();
  assert.ok(total > 0, "oracle sees board rows");
  const r = run(["--dry-run"]);
  assert.equal(r.status, 0);
  const line = `DRY RUN: ${done}/${total} ticked, ${ready.length} ready: ${ready.join(" ")}`;
  assert.ok(r.stdout.includes(line), `dry run names the live board state:\n${r.stdout.slice(-400)}`);
});

test("--dry-run lists every ready id the oracle sees", () => {
  const { ready } = boardOracle();
  const r = run(["--dry-run"]);
  assert.equal(r.status, 0);
  const tail = r.stdout.split("\n").find((l) => l.startsWith("DRY RUN:")) ?? "";
  for (const id of ready.slice(0, 25)) {
    assert.ok(tail.includes(id), `ready id ${id} is listed`);
  }
  assert.ok(!tail.includes("  "), "ready ids are single-space joined");
});

test("an out-of-range --width is a usage error, never a wave", () => {
  for (const width of ["0", "31", "abc"]) {
    const r = run(["--dry-run", "--width", width]);
    assert.equal(r.status, 2, `--width ${width} exits 2`);
    assert.match(r.stderr, /Usage: node tools\/sprint-loop\.mjs/);
    assert.ok(!r.stdout.includes("WAVE"), "no wave starts on bad args");
  }
});

test("a non-positive --waves is a usage error, never a wave", () => {
  for (const waves of ["0", "-3", "abc"]) {
    const r = run(["--waves", waves]);
    assert.equal(r.status, 2, `--waves ${waves} exits 2`);
    assert.match(r.stderr, /Usage: node tools\/sprint-loop\.mjs/);
    assert.ok(!r.stdout.includes("WAVE"), "no wave starts on bad args");
  }
});

// Queue hygiene: done overflow must move to a dated archive, keeping the
// newest --keep files so ready/claim scans stay fast. Fails without the fix:
// a reverted tool has no --prune-queue flag and never prints QUEUE-PRUNE.
const queueFixture = (total) => {
  const dir = mkdtempSync(join(tmpdir(), "sprint-queue-"));
  const done = join(dir, "target", "queue", "done");
  mkdirSync(done, { recursive: true });
  const base = Date.now() - total * 1000;
  for (let i = 0; i < total; i += 1) {
    const name = `packet-${String(i).padStart(3, "0")}.md`;
    writeFileSync(join(done, name), `# ${name}\n`);
    const t = new Date(base + i * 1000);
    utimesSync(join(done, name), t, t);
  }
  // Pin the prune ranking: fixture mtimes must strictly increase with the packet index.
  let prev = -1;
  for (let i = 0; i < total; i += 1) {
    const name = `packet-${String(i).padStart(3, "0")}.md`;
    const mtime = statSync(join(done, name)).mtimeMs;
    assert.ok(mtime > prev, `fixture mtime increases at ${name}`);
    prev = mtime;
  }
  return { dir, done };
};
const pruneIn = (dir, args) =>
  spawnSync(process.execPath, [TOOL, ...args], { encoding: "utf8", cwd: dir });

test("--prune-queue keeps the newest keep files and archives the rest", () => {
  const { dir, done } = queueFixture(55);
  try {
    const r = pruneIn(dir, ["--prune-queue", "--keep", "50"]);
    assert.equal(r.status, 0, `prune exits 0, got: ${r.stdout}${r.stderr}`);
    assert.match(r.stdout, /QUEUE-PRUNE done=55->50 archived=5 keep=50/);
    assert.match(r.stdout, /archive-\d{4}-\d{2}-\d{2}/);
    assert.equal(readdirSync(done).length, 50, "done is capped at 50");
    const day = new Date().toISOString().slice(0, 10);
    const archived = readdirSync(join(dir, "target", "queue", `archive-${day}`));
    assert.equal(archived.length, 5, "overflow moves to the dated archive");
    assert.ok(!existsSync(join(done, "packet-000.md")), "oldest file is archived");
    assert.ok(existsSync(join(done, "packet-054.md")), "newest file is kept");
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("--prune-queue --dry-run reports the overflow and moves nothing", () => {
  const { dir, done } = queueFixture(52);
  try {
    const r = pruneIn(dir, ["--prune-queue", "--keep", "50", "--dry-run"]);
    assert.equal(r.status, 0, `dry prune exits 0, got: ${r.stdout}${r.stderr}`);
    assert.match(r.stdout, /QUEUE-PRUNE done=52->52 archived=2 keep=50/);
    assert.equal(readdirSync(done).length, 52, "dry-run moves nothing");
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("a non-positive --keep is a usage error", () => {
  const r = run(["--prune-queue", "--keep", "0"]);
  assert.equal(r.status, 2, "--keep 0 exits 2");
  assert.match(r.stderr, /Usage: node tools\/sprint-loop\.mjs/);
});

test("QUEUE-PRUNE counts rename failures instead of hiding them", () => {
  const { dir } = queueFixture(55);
  try {
    const r = pruneIn(dir, ["--prune-queue", "--keep", "50"]);
    assert.equal(r.status, 0);
    assert.match(r.stdout, /failed=0/, "success reports zero failures");
    const dry = pruneIn(dir, ["--prune-queue", "--keep", "50", "--dry-run"]);
    assert.match(dry.stdout, /failed=0/, "dry-run reports zero failures");
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});
