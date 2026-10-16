// Run: node --test tools/pool-queue.test.mjs
// Work-steal deque shape (idea shaped by crossbeam-rs/crossbeam-deque,
// Apache-2.0; code written fresh): owner end is LIFO, thief end is FIFO,
// batch steal keeps FIFO order, the pool moves tasks without loss or dup,
// and the lane-gate sim shows the p95 win of stealing over one serial gate.
import assert from "node:assert/strict";
import test from "node:test";
import { PoolQueue, WorkDeque, p95, simulateLaneGate } from "./pool-queue.mjs";

test("owner push/pop is LIFO (newest first)", () => {
  const q = new WorkDeque();
  q.push("a");
  q.push("b");
  q.push("c");
  assert.equal(q.pop(), "c");
  assert.equal(q.pop(), "b");
  assert.equal(q.pop(), "a");
  assert.equal(q.pop(), undefined);
  assert.ok(q.isEmpty());
});

test("thief steal takes the oldest task (FIFO) while owner keeps LIFO", () => {
  const q = new WorkDeque();
  for (const t of [1, 2, 3, 4]) q.push(t);
  assert.equal(q.steal(), 1);
  assert.equal(q.steal(), 2);
  assert.equal(q.pop(), 4);
  assert.equal(q.pop(), 3);
  assert.ok(q.isEmpty());
});

test("steal on an empty deque returns undefined and stealBatch clamps", () => {
  const q = new WorkDeque();
  assert.equal(q.steal(), undefined);
  assert.deepEqual(q.stealBatch(8), []);
  q.push("x");
  assert.deepEqual(q.stealBatch(8), ["x"]);
  assert.ok(q.isEmpty());
});

test("stealBatch returns oldest-first order and leaves the rest", () => {
  const q = new WorkDeque();
  for (let i = 1; i <= 6; i += 1) q.push(i);
  assert.deepEqual(q.stealBatch(2), [1, 2]);
  assert.deepEqual(q.snapshot(), [3, 4, 5, 6]);
  assert.equal(q.size, 4);
  assert.deepEqual(q.stealBatch(0), []);
  assert.deepEqual(q.stealBatch(-3), []);
});

test("pool steal moves oldest tasks victim -> thief without loss", () => {
  const pool = new PoolQueue(3);
  for (const t of ["a", "b", "c", "d"]) pool.push(0, t);
  assert.equal(pool.steal(0, 1, 2), 2);
  assert.deepEqual(pool.lanes[1].snapshot(), ["a", "b"]);
  assert.deepEqual(pool.lanes[0].snapshot(), ["c", "d"]);
  assert.equal(pool.total, 4);
  // Thief pops stolen work LIFO from its own bottom: b first, then a.
  assert.equal(pool.pop(1), "b");
  assert.equal(pool.pop(1), "a");
});

test("tryStealAny finds a non-empty victim and reports it", () => {
  const pool = new PoolQueue(4);
  pool.push(2, "job");
  const res = pool.tryStealAny(0, 1);
  assert.equal(res.victim, 2);
  assert.equal(res.moved, 1);
  assert.equal(pool.pop(0), "job");
  const idle = pool.tryStealAny(1, 1);
  assert.deepEqual(idle, { victim: -1, moved: 0 });
});

test("mixed push/pop/steal never loses or duplicates tasks", () => {
  const pool = new PoolQueue(4);
  const N = 200;
  for (let i = 0; i < N; i += 1) pool.push(i % 4, i);
  const seen = [];
  let guard = N * 4;
  while (pool.total > 0 && guard-- > 0) {
    for (let w = 0; w < 4; w += 1) {
      let task = pool.pop(w);
      if (task === undefined) {
        pool.tryStealAny(w, 2, task === undefined ? w : 0);
        task = pool.pop(w);
      }
      if (task !== undefined) seen.push(task);
    }
  }
  assert.equal(pool.total, 0);
  assert.equal(seen.length, N);
  assert.deepEqual([...seen].sort((a, b) => a - b), Array.from({ length: N }, (_, i) => i));
});

test("PoolQueue rejects zero workers", () => {
  assert.throws(() => new PoolQueue(0), RangeError);
});

test("p95 picks the 95th percentile rank and 0 when empty", () => {
  assert.equal(p95([]), 0);
  const vals = Array.from({ length: 100 }, (_, i) => i + 1);
  assert.equal(p95(vals), 95);
});

test("lane-gate sim is deterministic and stealing beats one serial gate", () => {
  const a = simulateLaneGate({ taskCount: 512, workers: 4, seed: 2040 });
  const b = simulateLaneGate({ taskCount: 512, workers: 4, seed: 2040 });
  assert.equal(a.beforeP95, b.beforeP95);
  assert.equal(a.afterP95, b.afterP95);
  assert.equal(a.taskCount, b.taskCount);
  assert.equal(a.workers, b.workers);
  assert.equal(a.seed, b.seed);
  assert.ok(a.beforeP95 > a.afterP95);
  assert.ok(a.simMs >= 0);
  console.log(`lane-gate p95: before=${a.beforeP95} after=${a.afterP95} simMs=${a.simMs.toFixed(2)}`);
});
