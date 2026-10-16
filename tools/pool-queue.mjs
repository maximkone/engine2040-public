// Work-steal deque shape shaped by crossbeam-rs/crossbeam-deque (Apache-2.0).
// Donor: crossbeam-rs/crossbeam-deque, licence Apache-2.0. Idea only: owner
// pushes/pops the bottom (LIFO), thieves steal the top (FIFO). All code below
// is written fresh for engine2040; no donor code is copied.
// SPDX-License-Identifier: MIT

// Single-owner, single-threaded model of the Chase-Lev / crossbeam deque
// shape: the owner works the bottom in LIFO order (hot task first) and a
// thief takes the top in FIFO order (oldest task first, least contention).
// Backed by an array plus a head index so steal is O(1) amortised.
export class WorkDeque {
  constructor() {
    this.buf = [];
    this.head = 0;
  }

  get size() {
    return this.buf.length - this.head;
  }

  isEmpty() {
    return this.size === 0;
  }

  // Owner-only: push to the bottom (newest end).
  push(task) {
    this.buf.push(task);
  }

  // Owner-only: pop the bottom (LIFO: newest first).
  pop() {
    if (this.isEmpty()) return undefined;
    const task = this.buf.pop();
    if (this.buf.length === this.head) {
      this.buf.length = 0;
      this.head = 0;
    }
    return task;
  }

  // Thief: steal the top (FIFO: oldest first).
  steal() {
    if (this.isEmpty()) return undefined;
    const task = this.buf[this.head];
    this.head += 1;
    if (this.head === this.buf.length) {
      this.buf.length = 0;
      this.head = 0;
    } else if (this.head > 64 && this.head * 2 > this.buf.length) {
      this.buf.splice(0, this.head);
      this.head = 0;
    }
    return task;
  }

  // Thief: steal up to n oldest tasks in FIFO order.
  stealBatch(n) {
    const count = Math.max(0, Math.min(Math.trunc(n) || 0, this.size));
    const out = this.buf.slice(this.head, this.head + count);
    this.head += count;
    if (this.head === this.buf.length) {
      this.buf.length = 0;
      this.head = 0;
    } else if (this.head > 64 && this.head * 2 > this.buf.length) {
      this.buf.splice(0, this.head);
      this.head = 0;
    }
    return out;
  }

  // Oldest-to-newest view for tests and debugging. A copy, never live refs.
  snapshot() {
    return this.buf.slice(this.head);
  }
}

// Fixed pool of per-lane deques. Lane i is owned by worker i; any worker may
// steal from any other lane. Synchronous and deterministic.
export class PoolQueue {
  constructor(workerCount) {
    const n = Math.trunc(workerCount) || 0;
    if (n < 1) throw new RangeError("PoolQueue needs at least 1 worker");
    this.lanes = Array.from({ length: n }, () => new WorkDeque());
  }

  get workers() {
    return this.lanes.length;
  }

  get total() {
    let sum = 0;
    for (const lane of this.lanes) sum += lane.size;
    return sum;
  }

  sizes() {
    return this.lanes.map((lane) => lane.size);
  }

  push(worker, task) {
    this.lanes[worker | 0].push(task);
  }

  pop(worker) {
    return this.lanes[worker | 0].pop();
  }

  // Move up to n oldest tasks from victim lane to thief lane bottom.
  // Returns the number of tasks moved.
  steal(victim, thief, n = 1) {
    const taken = this.lanes[victim | 0].stealBatch(n);
    for (const task of taken) this.lanes[thief | 0].push(task);
    return taken.length;
  }

  // Thief scans other lanes round-robin from a seed offset and steals one
  // batch from the first non-empty victim. Returns { victim, moved }.
  tryStealAny(thief, n = 1, offset = 0) {
    const who = thief | 0;
    for (let i = 0; i < this.lanes.length; i += 1) {
      const victim = (((who + 1 + i + (offset | 0)) % this.lanes.length) + this.lanes.length) % this.lanes.length;
      if (victim === who) continue;
      if (!this.lanes[victim].isEmpty()) {
        return { victim, moved: this.steal(victim, who, n) };
      }
    }
    return { victim: -1, moved: 0 };
  }
}

// Deterministic 32-bit LCG (Numerical Recipes constants) for the sim below.
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

// Lane-gate queue model: taskCount gate tasks with service ticks drawn from a
// seeded RNG. Before: one serial gate drains a global FIFO, so wait stacks up.
// After: workers lanes share the same arrivals and steal toward the
// least-loaded lane, so wait spreads across lanes. Returns p95 wait ticks for
// both shapes plus the wall-clock sim cost in ms.
export function simulateLaneGate({ taskCount = 512, workers = 4, seed = 2040 } = {}) {
  const rand = lcg(seed);
  const service = Array.from({ length: Math.max(1, Math.trunc(taskCount)) }, () => 1 + Math.floor(rand() * 9));
  const started = performance.now();

  // Before: single serial gate, FIFO. Wait of task i is the backlog ahead.
  let backlog = 0;
  const beforeWaits = service.map((svc) => {
    const wait = backlog;
    backlog += svc;
    return wait;
  });

  // After: least-loaded lane wins each arrival (what steady stealing holds).
  const loads = new Array(Math.max(1, Math.trunc(workers))).fill(0);
  const afterWaits = service.map((svc) => {
    let best = 0;
    for (let i = 1; i < loads.length; i += 1) {
      if (loads[i] < loads[best]) best = i;
    }
    const wait = loads[best];
    loads[best] += svc;
    return wait;
  });

  const ms = performance.now() - started;
  return {
    taskCount: service.length,
    workers: loads.length,
    seed: seed >>> 0,
    beforeP95: p95(beforeWaits),
    afterP95: p95(afterWaits),
    simMs: ms,
  };
}
