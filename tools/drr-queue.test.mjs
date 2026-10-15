// Run: node --test tools/drr-queue.test.mjs
// drr-queue is the deficit-round-robin lane queue: N FIFO lanes share one
// gate, each visit adds quantum to the deficit and serves the head task
// when it fits. Pure plus deterministic, so no repo file is touched.
import assert from "node:assert/strict";
import test from "node:test";
import { DrrQueue, p95, simulateDrrGate } from "./drr-queue.mjs";

test("drain serves every task exactly once", () => {
  const dq = new DrrQueue(4, 5);
  const N = 200;
  for (let i = 0; i < N; i += 1) dq.enqueue(i % 4, 1 + (i % 9));
  const served = dq.drain().map((hit) => hit.task);
  assert.equal(served.length, N);
  assert.ok(dq.isEmpty());
  const sorted = [...served].sort((a, b) => a - b);
  const want = Array.from({ length: N }, (_, i) => 1 + (i % 9)).sort((a, b) => a - b);
  assert.deepEqual(sorted, want);
});

test("heavy head does not starve light lanes", () => {
  const dq = new DrrQueue(2, 5);
  dq.enqueue(0, 9);
  dq.enqueue(0, 9);
  dq.enqueue(1, 1);
  dq.enqueue(1, 1);
  const order = dq.drain().map((hit) => hit.lane);
  assert.ok(order.includes(1));
  assert.ok(order.includes(0));
});

test("bad construction and bad enqueue are rejected", () => {
  assert.throws(() => new DrrQueue(0), RangeError);
  assert.throws(() => new DrrQueue(2, 0), RangeError);
  const dq = new DrrQueue(2, 5);
  assert.throws(() => dq.enqueue(9, 1), RangeError);
  assert.throws(() => dq.enqueue(0, undefined), TypeError);
  assert.throws(() => dq.enqueue(0, 0), RangeError);
});

test("p95 picks the 95th percentile rank and 0 when empty", () => {
  assert.equal(p95([]), 0);
  const vals = Array.from({ length: 100 }, (_, i) => i + 1);
  assert.equal(p95(vals), 95);
});

test("sim is deterministic and fair sharing cuts the serial p95", () => {
  const a = simulateDrrGate({ taskCount: 512, workers: 4, seed: 2040, quantum: 5 });
  const b = simulateDrrGate({ taskCount: 512, workers: 4, seed: 2040, quantum: 5 });
  assert.equal(a.beforeP95, b.beforeP95);
  assert.equal(a.afterP95, b.afterP95);
  assert.equal(a.taskCount, 512);
  assert.equal(a.workers, 4);
  assert.equal(a.seed, 2040);
  assert.ok(a.beforeP95 > a.afterP95);
});
