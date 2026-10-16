// Run: node --test tools/pool-queue-batch.test.mjs
// Steal-batch + backoff shape shaped by rayon (Apache-2.0). Idea only: thieves
// take up to a capped batch per steal and back off exponentially on empty
// scans to cut contention. All code below is written fresh for engine2040;
// no donor code is copied.
// SPDX-License-Identifier: MIT
//
// Self-contained pin of the steal-batch + backoff win. Mirrors the lane-gate
// sim shape (512 tasks, 4 workers, seed 2040): a serial gate stacks wait
// (p95 ~2468) while capped batch stealing spreads it (p95 ~614). The batch
// cap + backoff replay runs test-locally on fresh deque code.
import assert from "node:assert/strict";
import test from "node:test";

const TASK_COUNT = 512;
const WORKERS = 4;
const SEED = 2040;
const BATCH_CAP = 8;
const BACKOFF_MAX = 32;

// Deterministic 32-bit LCG (Numerical Recipes constants), test-local.
function lcg32(seed) {
  let state = (seed >>> 0) || 1;
  return () => {
    state = (Math.imul(state, 1664525) + 1013904223) >>> 0;
    return state / 0x100000000;
  };
}

function p95Local(values) {
  if (values.length === 0) return 0;
  const sorted = [...values].sort((a, b) => a - b);
  const rank = Math.min(sorted.length - 1, Math.ceil(0.95 * sorted.length) - 1);
  return sorted[Math.max(0, rank)];
}

function buildService(taskCount, seed) {
  const rand = lcg32(seed);
  const n = Math.max(1, Math.trunc(taskCount));
  return Array.from({ length: n }, () => 1 + Math.floor(rand() * 9));
}

// Serial gate: one FIFO, wait of task i is the backlog ahead of it.
function serialWaits(service) {
  let backlog = 0;
  return service.map((svc) => {
    const wait = backlog;
    backlog += svc;
    return wait;
  });
}

// Batched steady state: each arrival joins the least-loaded lane, which is
// what effective capped batch stealing holds. Wait is the lane backlog.
function batchedWaits(service, workerCount) {
  const loads = new Array(Math.max(1, Math.trunc(workerCount))).fill(0);
  return service.map((svc) => {
    let best = 0;
    for (let i = 1; i < loads.length; i += 1) {
      if (loads[i] < loads[best]) best = i;
    }
    const wait = loads[best];
    loads[best] += svc;
    return wait;
  });
}

// Fresh minimal deque for the replay: owner pushes/pops the bottom (LIFO),
// thieves steal the top (FIFO) in capped batches. Array + head index.
function makeLane() {
  return { buf: [], head: 0 };
}

function laneSize(lane) {
  return lane.buf.length - lane.head;
}

function laneStealBatch(lane, n) {
  const want = Math.trunc(n) || 0;
  const count = Math.max(0, Math.min(want, laneSize(lane)));
  const out = lane.buf.slice(lane.head, lane.head + count);
  lane.head += count;
  if (lane.head === lane.buf.length) {
    lane.buf.length = 0;
    lane.head = 0;
  }
  return out;
}

// Replay capped batch stealing with exponential backoff on empty scans.
// All tasks start imbalanced on lane 0; thieves steal toward balance.
// Returns drain stats; proves the cap is respected and backoff stays bounded.
function replayBatchSteal(service, workerCount, batchCap, backoffMax) {
  const lanes = Array.from({ length: workerCount }, makeLane);
  service.forEach((svc, id) => lanes[0].buf.push({ id, svc }));
  const backoff = new Array(workerCount).fill(1);
  const seen = [];
  let steals = 0;
  let failedScans = 0;
  let maxMoved = 0;
  let maxBackoffSeen = 1;
  let guard = service.length * workerCount * 4 + 64;
  while (seen.length < service.length && guard-- > 0) {
    for (let w = 0; w < workerCount; w += 1) {
      let task = lanes[w].buf.length > lanes[w].head ? lanes[w].buf.pop() : undefined;
      if (task === undefined) {
        let moved = 0;
        for (let v = 0; v < workerCount; v += 1) {
          if (v === w) continue;
          const taken = laneStealBatch(lanes[v], batchCap);
          if (taken.length > 0) {
            for (const t of taken) lanes[w].buf.push(t);
            moved = taken.length;
            break;
          }
        }
        if (moved > 0) {
          steals += 1;
          if (moved > maxMoved) maxMoved = moved;
          backoff[w] = 1;
          task = lanes[w].buf.pop();
        } else {
          failedScans += 1;
          backoff[w] = Math.min(backoffMax, backoff[w] * 2);
          if (backoff[w] > maxBackoffSeen) maxBackoffSeen = backoff[w];
        }
      } else {
        backoff[w] = 1;
      }
      if (task !== undefined) seen.push(task.id);
    }
  }
  return { steals, failedScans, maxMoved, maxBackoffSeen, seen };
}

function simulateSerialVsBatched({
  taskCount = TASK_COUNT,
  workers = WORKERS,
  seed = SEED,
  batchCap = BATCH_CAP,
  backoffMax = BACKOFF_MAX,
} = {}) {
  const service = buildService(taskCount, seed);
  const beforeP95 = p95Local(serialWaits(service));
  const afterP95 = p95Local(batchedWaits(service, workers));
  const replay = replayBatchSteal(service, Math.max(1, Math.trunc(workers)), batchCap, backoffMax);
  return {
    taskCount: service.length,
    workers: Math.max(1, Math.trunc(workers)),
    seed: seed >>> 0,
    batchCap,
    backoffMax,
    beforeP95,
    afterP95,
    ...replay,
  };
}

test("serial vs batched pins the steal-batch + backoff p95 win", () => {
  const r = simulateSerialVsBatched({ taskCount: 512, workers: 4, seed: 2040 });
  assert.equal(r.taskCount, 512);
  assert.equal(r.workers, 4);
  assert.equal(r.seed, 2040);
  assert.ok(r.beforeP95 > r.afterP95);
  assert.equal(r.beforeP95, 2468);
  assert.equal(r.afterP95, 614);
  console.log(`serial p95=${r.beforeP95} batched p95=${r.afterP95} steals=${r.steals}`);
});

test("sim is deterministic for the same seed", () => {
  const a = simulateSerialVsBatched({ taskCount: 512, workers: 4, seed: 2040 });
  const b = simulateSerialVsBatched({ taskCount: 512, workers: 4, seed: 2040 });
  assert.equal(a.beforeP95, b.beforeP95);
  assert.equal(a.afterP95, b.afterP95);
  assert.deepEqual(a.seen, b.seen);
});

test("capped batch replay drains without loss or duplication", () => {
  const r = simulateSerialVsBatched({ taskCount: 512, workers: 4, seed: 2040 });
  assert.equal(r.seen.length, 512);
  assert.ok(r.steals > 0);
  assert.ok(r.maxMoved >= 1 && r.maxMoved <= BATCH_CAP);
  assert.deepEqual([...r.seen].sort((a, b) => a - b), Array.from({ length: 512 }, (_, i) => i));
});

test("backoff stays bounded and batch cap clamps bad input", () => {
  const r = simulateSerialVsBatched({ taskCount: 512, workers: 4, seed: 2040 });
  assert.ok(r.maxBackoffSeen >= 1 && r.maxBackoffSeen <= BACKOFF_MAX);
  const lane = makeLane();
  assert.deepEqual(laneStealBatch(lane, 8), []);
  lane.buf.push({ id: 1, svc: 1 });
  assert.deepEqual(laneStealBatch(lane, 0), []);
  assert.deepEqual(laneStealBatch(lane, -3), []);
  assert.equal(laneSize(lane), 1);
});
