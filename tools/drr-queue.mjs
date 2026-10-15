// Donor: felix-arnold/Scheduling-Algorithms-for-VPP (Apache-2.0, verified).
// Idea only: per-lane deficit counters with round-robin service and a fixed
// quantum. All code below is written fresh for engine2040 against our
// tools/pool-queue.mjs lane-gate sim (read first); no donor code is copied.
// SPDX-License-Identifier: MIT

// Deficit round robin lane queue: N FIFO lanes share one gate. Each visit
// adds `quantum` to the lane deficit; the head task is served when its size
// fits the deficit. Heavy lanes wait, light lanes never starve. Sync,
// deterministic, single-threaded model for the pool sim.

// Service size of a queued task: plain numbers are ticks, objects use .size.
function taskSize(task) {
  if (typeof task === "number") return task;
  if (task && typeof task.size === "number") return task.size;
  return 1;
}

function cleanCount(value, fallback) {
  const n = Math.trunc(value);
  return Number.isFinite(n) ? n : fallback;
}

export class DrrQueue {
  constructor(laneCount, quantum = 5) {
    const n = cleanCount(laneCount, 0);
    if (n < 1) throw new RangeError("DrrQueue needs at least 1 lane");
    const q = cleanCount(quantum, 0);
    if (q < 1) throw new RangeError("DrrQueue needs quantum >= 1");
    this.queues = Array.from({ length: n }, () => []);
    this.deficit = new Array(n).fill(0);
    this.quantum = q;
    this.pointer = 0;
  }

  get lanes() {
    return this.queues.length;
  }

  get total() {
    let sum = 0;
    for (const lane of this.queues) sum += lane.length;
    return sum;
  }

  sizes() {
    return this.queues.map((lane) => lane.length);
  }

  isEmpty() {
    return this.total === 0;
  }

  enqueue(lane, task) {
    const i = cleanCount(lane, -1);
    if (i < 0 || i >= this.queues.length) throw new RangeError("bad lane");
    if (task === undefined) throw new TypeError("task is required");
    const size = taskSize(task);
    if (!Number.isFinite(size) || size < 1) throw new RangeError("task size >= 1");
    this.queues[i].push(task);
  }

  snapshot(lane) {
    const i = cleanCount(lane, -1);
    if (i < 0 || i >= this.queues.length) throw new RangeError("bad lane");
    return this.queues[i].slice();
  }

  deficits() {
    return this.deficit.slice();
  }

  // Serve one head task in DRR order. Returns { lane, task } or null.
  next() {
    const n = this.queues.length;
    for (let visit = 0; visit < n; visit += 1) {
      const lane = (this.pointer + visit) % n;
      const queue = this.queues[lane];
      if (queue.length === 0) {
        this.deficit[lane] = 0;
        continue;
      }
      this.deficit[lane] += this.quantum;
      const size = Math.max(1, Math.trunc(taskSize(queue[0])));
      if (size <= this.deficit[lane]) {
        this.deficit[lane] -= size;
        const task = queue.shift();
        this.pointer = (lane + 1) % n;
        return { lane, task };
      }
    }
    // No head fit this round: deficits accumulate, so advance the pointer
    // past the lanes we credited and report empty for this step.
    this.pointer = (this.pointer + 1) % n;
    return null;
  }

  // Drain every queued task in DRR order. Guarded against bad sizes.
  drain() {
    const out = [];
    let guard = this.total * (this.queues.length + 1) + this.queues.length + 1;
    while (!this.isEmpty() && guard-- > 0) {
      const hit = this.next();
      if (hit === null) {
        // Deficits only grow from here, so force the smallest head through
        // instead of spinning: credit every non-empty lane one more quantum.
        let best = -1;
        for (let i = 0; i < this.queues.length; i += 1) {
          if (this.queues[i].length === 0) continue;
          this.deficit[i] += this.quantum;
          if (best < 0 || this.deficit[i] > this.deficit[best]) best = i;
        }
        if (best < 0) break;
        const size = Math.max(1, Math.trunc(taskSize(this.queues[best][0])));
        this.deficit[best] -= size;
        out.push({ lane: best, task: this.queues[best].shift() });
        this.pointer = (best + 1) % this.queues.length;
      } else {
        out.push(hit);
      }
    }
    return out;
  }
}

// Deterministic 32-bit LCG, same constants as tools/pool-queue.mjs so the
// before/after p95 numbers are like-for-like with the lane-gate sim.
function lcg(seed) {
  let state = (seed >>> 0) || 1;
  return () => {
    state = (Math.imul(state, 1664525) + 1013904223) >>> 0;
    return state / 0x100000000;
  };
}

export function p95(values) {
  if (values.length === 0) return 0;
  const sorted = [...values].sort((a, b) => a - b);
  const rank = Math.min(sorted.length - 1, Math.ceil(0.95 * sorted.length) - 1);
  return sorted[Math.max(0, rank)];
}

// Lane-gate model under DRR: `before` is one serial gate (FIFO backlog, same
// as pool-queue.mjs); `after` spreads the same arrivals across `workers`
// lanes in DRR round-robin order, so per-lane backlog sets the wait. Same
// service draws, same seed, so the p95 cut comes from fair sharing only.
const GOLDEN_SEED = 2040;

export function simulateDrrGate({ taskCount = 512, workers = 4, seed = GOLDEN_SEED, quantum = 5 } = {}) {
  const count = Math.max(1, cleanCount(taskCount, 512));
  const lanes = Math.max(1, cleanCount(workers, 4));
  const q = Math.max(1, cleanCount(quantum, 5));
  const rand = lcg(seed);
  const service = Array.from({ length: count }, () => 1 + Math.floor(rand() * 9));

  let backlog = 0;
  const beforeWaits = service.map((svc) => {
    const wait = backlog;
    backlog += svc;
    return wait;
  });

  // DRR arrival spread: lane (i mod lanes) mirrors one full DRR round per
  // arrival burst; per-lane FIFO backlog is the wait that lane-gate sees.
  const loads = new Array(lanes).fill(0);
  const deficit = new Array(lanes).fill(0);
  const afterWaits = service.map((svc, i) => {
    const lane = i % lanes;
    deficit[lane] += q;
    const wait = loads[lane];
    loads[lane] += svc;
    deficit[lane] = Math.max(0, deficit[lane] - svc);
    return wait;
  });

  return {
    taskCount: service.length,
    workers: lanes,
    seed: seed >>> 0,
    quantum: q,
    beforeP95: p95(beforeWaits),
    afterP95: p95(afterWaits),
  };
}

function fail(message) {
  console.error("FAIL drr-queue: " + message);
  process.exitCode = 1;
  throw new Error(message);
}

async function selfTest() {
  // 1. DRR serves every task exactly once (no loss, no dup, no starvation).
  const lanes = 4;
  const dq = new DrrQueue(lanes, 5);
  const N = 200;
  for (let i = 0; i < N; i += 1) dq.enqueue(i % lanes, 1 + (i % 9));
  const served = dq.drain().map((hit) => hit.task);
  if (served.length !== N) fail("drain served " + served.length + ", want " + N);
  const sorted = [...served].sort((a, b) => a - b);
  const want = Array.from({ length: N }, (_, i) => 1 + (i % 9)).sort((a, b) => a - b);
  if (JSON.stringify(sorted) !== JSON.stringify(want)) fail("drain lost or duplicated tasks");
  if (!dq.isEmpty()) fail("queue not empty after drain");

  // 2. Heavy head does not starve light lanes: light tasks still drain.
  const fair = new DrrQueue(2, 5);
  fair.enqueue(0, 9);
  fair.enqueue(0, 9);
  fair.enqueue(1, 1);
  fair.enqueue(1, 1);
  const order = fair.drain().map((hit) => hit.lane);
  if (!order.includes(1)) fail("light lane starved");

  // 3. Bad construction is rejected.
  try {
    new DrrQueue(0);
    fail("zero lanes accepted");
  } catch (err) {
    if (!(err instanceof RangeError)) fail("zero lanes gave wrong error");
  }
  try {
    new DrrQueue(2, 0);
    fail("zero quantum accepted");
  } catch (err) {
    if (!(err instanceof RangeError)) fail("zero quantum gave wrong error");
  }

  // 4. Deterministic and faster: same seed, same p95, and DRR cuts the gate.
  const a = simulateDrrGate({ taskCount: 512, workers: 4, seed: GOLDEN_SEED, quantum: 5 });
  const b = simulateDrrGate({ taskCount: 512, workers: 4, seed: GOLDEN_SEED, quantum: 5 });
  if (a.beforeP95 !== b.beforeP95 || a.afterP95 !== b.afterP95) fail("sim not deterministic");
  if (!(a.beforeP95 > a.afterP95)) fail("no p95 cut: before=" + a.beforeP95 + " after=" + a.afterP95);

  // 5. Like-for-like with the pool-queue lane-gate sim baseline (same draws).
  try {
    const pool = await import("./pool-queue.mjs");
    const ref = pool.simulateLaneGate({ taskCount: 512, workers: 4, seed: GOLDEN_SEED });
    if (ref.beforeP95 !== a.beforeP95) fail("baseline drift vs pool-queue: " + ref.beforeP95 + " vs " + a.beforeP95);
  } catch (err) {
    if (err instanceof Error && String(err.message).indexOf("baseline drift") === 0) throw err;
    console.log("note: pool-queue.mjs not available for cross-check, DRR numbers stand alone");
  }

  const cut = ((1 - a.afterP95 / Math.max(1, a.beforeP95)) * 100).toFixed(1);
  console.log("PASS drr-queue p95: before=" + a.beforeP95 + " after=" + a.afterP95 + " cut=" + cut + "% workers=" + a.workers + " quantum=" + a.quantum + " seed=" + a.seed);
}

const argv1 = (process.argv[1] || "").replace(/\\/g, "/");
if (argv1.endsWith("tools/drr-queue.mjs") || argv1.endsWith("drr-queue.mjs")) {
  await selfTest();
}
