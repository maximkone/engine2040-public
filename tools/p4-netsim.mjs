#!/usr/bin/env node
// p4-netsim: deterministic 64-client online model for gate P4.
//
// Seed 2040. Units: ticks at 60 Hz, sim milliseconds, bytes.
// Model: the server keeps one state word per client slot and steps once per
// tick. Each client sends one input per tick, 6 ticks ahead (100 ms input
// delay). The server sends a full-state snapshot every 2 ticks to each online
// client. Per packet, each direction: 2% loss, one-way delay 55 + U[0,40] ms
// (mean RTT 150 ms; the 40 ms spread reorders packets).
// Reconnect: client 5 is offline for 3 s (180 ticks), then sends Join every
// 100 ms until a snapshot lands. The server reclaims its slot by token.
// Desync: a client applies a state whose rebuilt digest differs from the
// server digest for that tick (packet digest or server history).
// NOT covered: real sockets and the internet (gate C4), client prediction and
// rollback, the delta wire codec, real per-client bandwidth caps.
//
// Usage:
//   node tools/p4-netsim.mjs [--ticks N] [--reconnect on|off] [--corrupt C:T] [--selftest]
// P4_TICKS=N sets the tick count (default 600; one-hour soak = 216000).
// Exit 0 when the run passes (selftest: when the corrupt run is reported as a
// desync). Exit 1 otherwise, exit 2 on bad arguments.

const SEED = 2040n;
const CLIENTS = 64;
const TICK_HZ = 60;
const ONEWAY_FLOOR_MS = 55;
const JITTER_MS = 40;
const LOSS_PCT = 2;
const INPUT_DELAY_TICKS = 6;
const SNAP_EVERY = 2;
const JOIN_RETRY_TICKS = 6;
// Freshest applied snapshot must be within 18 ticks (300 ms) of the server.
const LAG_BAR_TICKS = 18;
const CONVERGE_BAR_MS = 5000;
const RECONNECT_CLIENT = 5;
const RECONNECT_START_TICK = 100;
const RECONNECT_TICKS = 180; // 3 s at 60 Hz
const MIN_RECONNECT_RUN_TICKS = 400;
const DEFAULT_TICKS = 600;
const MAX_TICKS = 1000000;
const LAG_SAMPLE_FROM_TICK = 10;
const SNAP_BYTES = 16 + 8 * CLIENTS; // header plus 64 words, informational
const M64 = (1n << 64n) - 1n;
const GOLDEN = 0x9e3779b97f4a7c15n;
const DIGEST_K = 0x100000001b3n;
const STATE_K = 0x5851f42d4c957f2dn;
const STATE_C = 0x14057b7ef767814fn;
const DOWN = 1n;
const UP = 2n;

// SplitMix64 finalizer, same constants as engine_core::mix64.
function mix64(x) {
  let z = x & M64;
  z = ((z ^ (z >> 30n)) * 0xbf58476d1ce4e5b9n) & M64;
  z = ((z ^ (z >> 27n)) * 0x94d049bb133111ebn) & M64;
  return z ^ (z >> 31n);
}

class Rng {
  constructor(seed) {
    this.s = mix64(seed);
  }
  next() {
    this.s = (this.s + GOLDEN) & M64;
    return mix64(this.s);
  }
  below(n) {
    return Number(this.next() % BigInt(n));
  }
}

function lane(kind, client) {
  return new Rng((SEED << 40n) ^ (kind << 32n) ^ BigInt(client));
}

function inputOf(client, tick) {
  return mix64((SEED << 44n) ^ (BigInt(client) << 24n) ^ BigInt(tick));
}

// Fold of all words; a single-word change always changes the digest.
function digestOf(tick, state) {
  let h = mix64(SEED ^ BigInt(tick));
  for (let i = 0; i < CLIENTS; i++) {
    h = ((h ^ state[i]) * DIGEST_K) & M64;
  }
  return mix64(h);
}

function tMs(tick) {
  return Math.floor((tick * 1000) / TICK_HZ);
}

function before(x, y) {
  return x.time < y.time || (x.time === y.time && x.seq < y.seq);
}

class EventQueue {
  constructor() {
    this.a = [];
  }
  get size() {
    return this.a.length;
  }
  peek() {
    return this.a[0];
  }
  push(ev) {
    const a = this.a;
    a.push(ev);
    let i = a.length - 1;
    while (i > 0) {
      const p = (i - 1) >> 1;
      if (!before(a[i], a[p])) break;
      [a[i], a[p]] = [a[p], a[i]];
      i = p;
    }
  }
  pop() {
    const a = this.a;
    const top = a[0];
    const last = a.pop();
    if (a.length > 0) {
      a[0] = last;
      let i = 0;
      for (;;) {
        const l = 2 * i + 1;
        const r = l + 1;
        let m = i;
        if (l < a.length && before(a[l], a[m])) m = l;
        if (r < a.length && before(a[r], a[m])) m = r;
        if (m === i) break;
        [a[i], a[m]] = [a[m], a[i]];
        i = m;
      }
    }
    return top;
  }
}

function runSim({ ticks, reconnect, corrupt }) {
  const started = performance.now();
  const queue = new EventQueue();
  let seq = 0;
  const stats = {
    downSent: 0,
    downLost: 0,
    upSent: 0,
    upLost: 0,
    reordered: 0,
    applied: 0,
    stale: 0,
    offlineDrop: 0,
    inputsArrived: 0,
    lateInputs: 0,
    missingInputs: 0,
    reclaimed: 0,
    newSlots: 0,
    corrupted: 0,
    maxLag: 0,
    desync: [],
  };
  const downRng = [];
  const upRng = [];
  for (let c = 0; c < CLIENTS; c++) {
    downRng.push(lane(DOWN, c));
    upRng.push(lane(UP, c));
  }
  const oneway = (rng) => ONEWAY_FLOOR_MS + rng.below(JITTER_MS + 1);
  const tokenOf = (c) => 0x5100 + c;
  const corruptState = corrupt ? { client: corrupt.client, tick: corrupt.tick, done: false } : null;

  const server = {
    state: Array.from({ length: CLIENTS }, (_, c) => mix64(SEED + BigInt(c))),
    lastIn: new Array(CLIENTS).fill(0n),
    pending: new Map(),
    nextTick: 0,
    online: new Array(CLIENTS).fill(true),
    slotOf: new Map(Array.from({ length: CLIENTS }, (_, c) => [tokenOf(c), c])),
    hist: [],
  };
  const clients = Array.from({ length: CLIENTS }, () => ({
    mode: "joined",
    appliedTick: -1,
    appliedDigest: 0n,
    applied: null,
    lastArrival: -1,
    joinStartMs: -1,
    convergeMs: -1,
  }));

  function sendSnap(c, tick, digest, frozen, now) {
    stats.downSent += 1;
    const rng = downRng[c];
    if (rng.below(100) < LOSS_PCT) {
      stats.downLost += 1;
      return;
    }
    let state = frozen;
    if (corruptState && !corruptState.done && corruptState.client === c && tick >= corruptState.tick) {
      state = frozen.slice();
      state[0] ^= 1n;
      corruptState.done = true;
      stats.corrupted += 1;
    }
    queue.push({ kind: "snap", time: now + oneway(rng), seq: seq++, client: c, tick, digest, state });
  }

  function sendInput(c, tick, now) {
    stats.upSent += 1;
    const rng = upRng[c];
    if (rng.below(100) < LOSS_PCT) {
      stats.upLost += 1;
      return;
    }
    queue.push({ kind: "input", time: now + oneway(rng), seq: seq++, client: c, tick, bits: inputOf(c, tick) });
  }

  function sendJoin(c, now) {
    stats.upSent += 1;
    const rng = upRng[c];
    if (rng.below(100) < LOSS_PCT) {
      stats.upLost += 1;
      return;
    }
    queue.push({ kind: "join", time: now + oneway(rng), seq: seq++, client: c });
  }

  function onInput(p) {
    if (p.tick < server.nextTick) {
      stats.lateInputs += 1;
      return;
    }
    server.pending.set(p.tick * CLIENTS + p.client, p.bits);
    stats.inputsArrived += 1;
  }

  function onJoin(p) {
    const c = p.client;
    if (server.online[c]) return;
    const slot = server.slotOf.get(tokenOf(c));
    if (slot === undefined) {
      stats.newSlots += 1;
      server.slotOf.set(tokenOf(c), c);
    } else {
      stats.reclaimed += 1;
    }
    server.online[c] = true;
  }

  function onSnap(p, now) {
    const cl = clients[p.client];
    if (cl.mode === "offline") {
      stats.offlineDrop += 1;
      return;
    }
    if (p.tick < cl.lastArrival) stats.reordered += 1;
    if (p.tick > cl.lastArrival) cl.lastArrival = p.tick;
    if (p.tick <= cl.appliedTick) {
      stats.stale += 1;
      return;
    }
    const rebuilt = digestOf(p.tick, p.state);
    if (rebuilt !== p.digest || rebuilt !== server.hist[p.tick]) {
      stats.desync.push({
        client: p.client,
        tick: p.tick,
        atMs: now,
        reason: rebuilt !== p.digest ? "packet digest mismatch" : "server history mismatch",
      });
      return;
    }
    cl.appliedTick = p.tick;
    cl.appliedDigest = rebuilt;
    cl.applied = p.state;
    stats.applied += 1;
    if (cl.mode === "joining") {
      cl.mode = "joined";
      cl.convergeMs = now - cl.joinStartMs;
    }
  }

  function deliverUntil(limit) {
    while (queue.size > 0 && queue.peek().time <= limit) {
      const ev = queue.pop();
      if (ev.kind === "snap") onSnap(ev, ev.time);
      else if (ev.kind === "input") onInput(ev);
      else onJoin(ev);
    }
  }

  function clientsTick(tick, now) {
    for (let c = 0; c < CLIENTS; c++) {
      const cl = clients[c];
      if (reconnect && c === RECONNECT_CLIENT) {
        if (tick === RECONNECT_START_TICK) {
          cl.mode = "offline";
          server.online[c] = false;
        }
        if (tick === RECONNECT_START_TICK + RECONNECT_TICKS) {
          cl.mode = "joining";
          cl.joinStartMs = now;
        }
      }
      if (cl.mode === "joined") {
        sendInput(c, tick + INPUT_DELAY_TICKS, now);
      } else if (
        cl.mode === "joining" &&
        (tick - (RECONNECT_START_TICK + RECONNECT_TICKS)) % JOIN_RETRY_TICKS === 0
      ) {
        sendJoin(c, now);
      }
    }
  }

  function serverTick(tick, now) {
    for (let c = 0; c < CLIENTS; c++) {
      const key = tick * CLIENTS + c;
      const bits = server.pending.get(key);
      if (bits !== undefined) {
        server.lastIn[c] = bits;
        server.pending.delete(key);
      } else if (tick >= INPUT_DELAY_TICKS && server.online[c]) {
        stats.missingInputs += 1;
      }
      server.state[c] = ((server.state[c] ^ server.lastIn[c]) * STATE_K + STATE_C) & M64;
    }
    server.nextTick = tick + 1;
    const digest = digestOf(tick, server.state);
    server.hist.push(digest);
    if (tick % SNAP_EVERY === 0) {
      const frozen = server.state.slice();
      for (let c = 0; c < CLIENTS; c++) {
        if (server.online[c]) sendSnap(c, tick, digest, frozen, now);
      }
    }
  }

  function sampleLag(tick) {
    if (tick < LAG_SAMPLE_FROM_TICK) return;
    for (const cl of clients) {
      if (cl.mode === "joined" && cl.appliedTick >= 0) {
        stats.maxLag = Math.max(stats.maxLag, tick - cl.appliedTick);
      }
    }
  }

  for (let tick = 0; tick < ticks; tick++) {
    const now = tMs(tick);
    deliverUntil(now);
    clientsTick(tick, now);
    serverTick(tick, now);
    sampleLag(tick);
  }
  deliverUntil(Infinity);

  const last = ticks - 1;
  const finalDigest = server.hist[last];
  let convergedFreshest = 0;
  for (const cl of clients) {
    const lag = last - cl.appliedTick;
    if (cl.appliedTick >= 0 && lag <= LAG_BAR_TICKS && cl.appliedDigest === server.hist[cl.appliedTick]) {
      convergedFreshest += 1;
    }
  }

  // Reliable settle: every client gets the final server state with no loss,
  // then the same digest check as any snapshot.
  const settleState = server.state.slice();
  const settleMs = tMs(last);
  for (let c = 0; c < CLIENTS; c++) {
    onSnap({ client: c, tick: last, digest: finalDigest, state: settleState }, settleMs);
  }
  let exactAfterSettle = 0;
  for (const cl of clients) {
    if (cl.appliedTick === last && digestOf(last, cl.applied) === finalDigest) exactAfterSettle += 1;
  }

  const rc = clients[RECONNECT_CLIENT];
  const reconnectOk =
    !reconnect ||
    (rc.convergeMs >= 0 && rc.convergeMs <= CONVERGE_BAR_MS && stats.reclaimed === 1 && stats.newSlots === 0);

  const reasons = [];
  if (stats.desync.length > 0) {
    const first = stats.desync[0];
    reasons.push(
      `desync events=${stats.desync.length} (first: client ${first.client} tick ${first.tick}, ${first.reason})`,
    );
  }
  if (convergedFreshest < CLIENTS) reasons.push(`freshest converged ${convergedFreshest}/${CLIENTS}`);
  if (exactAfterSettle < CLIENTS) reasons.push(`exact after settle ${exactAfterSettle}/${CLIENTS}`);
  if (stats.maxLag > LAG_BAR_TICKS) reasons.push(`max lag ${stats.maxLag} ticks over bar ${LAG_BAR_TICKS}`);
  if (!reconnectOk) {
    reasons.push(
      `reconnect client ${RECONNECT_CLIENT}: converge ${rc.convergeMs} ms (bar ${CONVERGE_BAR_MS}), reclaimed ${stats.reclaimed}, new_slots ${stats.newSlots}`,
    );
  }

  const seconds = ticks / TICK_HZ;
  const downKbpsPerClient = (stats.downSent * SNAP_BYTES * 8) / seconds / CLIENTS / 1000;

  return {
    ticks,
    reconnect,
    corrupt,
    stats,
    convergedFreshest,
    exactAfterSettle,
    reconnectOk,
    convergeMs: rc.convergeMs,
    downKbpsPerClient,
    wallMs: performance.now() - started,
    reasons,
    pass: reasons.length === 0,
  };
}

function printReport(r) {
  const out = [];
  out.push(
    `p4-netsim seed=2040 clients=${CLIENTS} ticks=${r.ticks} hz=${TICK_HZ} rtt_mean_ms=150 oneway_ms=${ONEWAY_FLOOR_MS}+U[0,${JITTER_MS}] loss_pct=${LOSS_PCT} input_delay_ticks=${INPUT_DELAY_TICKS} snap_every=${SNAP_EVERY} reconnect=${r.reconnect ? "on" : "off"}${r.corrupt ? ` corrupt=${r.corrupt.client}:${r.corrupt.tick}` : ""}`,
  );
  const s = r.stats;
  out.push(
    `p4-netsim wire: down sent=${s.downSent} lost=${s.downLost} up sent=${s.upSent} lost=${s.upLost} reordered=${s.reordered} applied=${s.applied} stale=${s.stale} offline_drop=${s.offlineDrop} late_inputs=${s.lateInputs} missing_inputs=${s.missingInputs}`,
  );
  out.push(`p4-netsim desync events=${s.desync.length} corrupted=${s.corrupted}`);
  out.push(
    `p4-netsim end: converged_freshest=${r.convergedFreshest}/${CLIENTS} (bar ${LAG_BAR_TICKS} ticks) max_lag=${s.maxLag} exact_after_settle=${r.exactAfterSettle}/${CLIENTS}`,
  );
  if (r.reconnect) {
    out.push(
      `p4-netsim reconnect: client=${RECONNECT_CLIENT} offline_ms=${(RECONNECT_TICKS * 1000) / TICK_HZ} converge_ms=${r.convergeMs.toFixed(1)} (bar ${CONVERGE_BAR_MS}) reclaimed=${s.reclaimed} new_slots=${s.newSlots} ok=${r.reconnectOk}`,
    );
  }
  out.push(
    `p4-netsim bandwidth (informational, full-state snapshots, not the wire codec): down_kbps_per_client=${r.downKbpsPerClient.toFixed(1)}`,
  );
  out.push(`p4-netsim wall_ms=${r.wallMs.toFixed(0)}`);
  for (const d of s.desync.slice(0, 5)) {
    out.push(`p4-netsim desync: client ${d.client} tick ${d.tick} at ${d.atMs} ms: ${d.reason}`);
  }
  out.push(r.pass ? "p4-netsim PASS" : `p4-netsim FAIL: ${r.reasons.join("; ")}`);
  console.log(out.join("\n"));
}

function parseArgs(argv) {
  const out = { ticks: null, reconnect: true, corrupt: null, selftest: false };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (a === "--ticks") {
      out.ticks = Number(argv[++i]);
    } else if (a === "--reconnect") {
      const v = argv[++i];
      if (v !== "on" && v !== "off") throw new Error("--reconnect takes on or off");
      out.reconnect = v === "on";
    } else if (a === "--corrupt") {
      const m = /^(\d+):(\d+)$/.exec(argv[++i] ?? "");
      if (!m) throw new Error("--corrupt takes C:T, e.g. 17:300");
      out.corrupt = { client: Number(m[1]), tick: Number(m[2]) };
    } else if (a === "--selftest") {
      out.selftest = true;
    } else {
      throw new Error(`unknown argument ${a}`);
    }
  }
  return out;
}

function resolveTicks(arg) {
  let ticks = arg;
  if (ticks === null) {
    const env = process.env.P4_TICKS;
    ticks = env === undefined || env === "" ? DEFAULT_TICKS : Number(env);
  }
  if (!Number.isInteger(ticks) || ticks < 1 || ticks > MAX_TICKS) {
    throw new Error(`ticks must be an integer 1..${MAX_TICKS}, got ${ticks}`);
  }
  return ticks;
}

function main() {
  const args = parseArgs(process.argv.slice(2));
  const ticks = resolveTicks(args.ticks);
  if (args.reconnect && ticks < MIN_RECONNECT_RUN_TICKS) {
    throw new Error(`reconnect needs at least ${MIN_RECONNECT_RUN_TICKS} ticks; use --reconnect off`);
  }
  if (args.corrupt && (args.corrupt.client >= CLIENTS || args.corrupt.tick >= ticks)) {
    throw new Error("--corrupt client must be below 64 and tick below the run length");
  }
  if (args.selftest) {
    const normal = runSim({ ticks, reconnect: args.reconnect, corrupt: null });
    printReport(normal);
    const corruptTick = Math.min(300, ticks - 2) & ~1;
    const bad = runSim({ ticks, reconnect: args.reconnect, corrupt: { client: 17, tick: corruptTick } });
    printReport(bad);
    const caught = bad.stats.desync.some((d) => d.client === 17);
    const ok = normal.pass && !bad.pass && caught;
    console.log(
      ok
        ? "p4-netsim selftest PASS: the clean run passes and the corrupted packet is reported as a desync"
        : "p4-netsim selftest FAIL: the check did not behave as expected",
    );
    process.exitCode = ok ? 0 : 1;
    return;
  }
  const r = runSim({ ticks, reconnect: args.reconnect, corrupt: args.corrupt });
  printReport(r);
  process.exitCode = r.pass ? 0 : 1;
}

try {
  main();
} catch (err) {
  console.error(`p4-netsim error: ${err.message}`);
  process.exitCode = 2;
}
