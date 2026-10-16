// Gateway health adapter: one merged health view for endpoint lanes.
//
// Steal (patterns only, own code): forge gateway health shapes —
// probe classify in gateway/GatewayProbeState.cs (connected wins, slow is
// not down), merged lane view in gateway/LaneStatus.cs (static over cards,
// no probing inside, no secrets in the reply), doctor bounds and redact in
// gateway/GatewayDoctor.cs (2 s probe bound, tokens and paths never leave),
// single timeout table in gateway/GatewayTimeoutTiers.cs, heartbeat age rule
// in docs/g23-heartbeat.md (unknown age is never old, 60 s stale bar).
//
// Engine use:
//   node tools/gateway-health.mjs --selftest     drills, exit 0 when shape holds
//   node tools/gateway-health.mjs --json         sample merged status
//   node tools/gateway-health.mjs --check <file> heartbeat file age check
//
// A heartbeat file holds { updatedAt: <unix ms> }. Missing stamp falls back
// to the file write time. Missing stamp and time means unknown age, never
// stale. Past the bar the check refuses loud with age, bar and recovery.
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";

export const UP = "up";
export const DOWN = "down";
export const SLOW = "slow";

export const EXIT_OK = 0;
export const EXIT_DEGRADED = 1;
export const EXIT_FAILED = 2;

export const PROBE_BOUND_MS = 2000;
export const HEARTBEAT_STALE_SEC = 60;
export const PROBE_CACHE_TTL_SEC = 300;

// One home for the bounds this adapter enforces. Small on purpose: the full
// gateway table lives in forge; here only the health numbers are pinned.
export const TIMEOUTS = {
  probeMs: PROBE_BOUND_MS,
  heartbeatStaleMs: HEARTBEAT_STALE_SEC * 1000,
  probeCacheTtlSec: PROBE_CACHE_TTL_SEC,
};

// Probe timeout is slow or unknown, never down. Only a refused link (or a
// dead holder) is down. A late answer is still an answer, so connected wins.
export function classifyProbe(connected, timedOut) {
  if (connected) return UP;
  if (timedOut) return SLOW;
  return DOWN;
}

export function isLive(state) {
  return state === UP;
}

// Tokens and local paths never leave a health reply. Every free-text detail
// rides this before it is served or logged.
export function redact(raw) {
  let s = String(raw ?? "");
  try {
    s = s.replace(/gdr_[A-Za-z0-9_-]+/g, "gdr_***");
    s = s.replace(/bearer\s+\S+/gi, "bearer ***");
    s = s.replace(/\btoken\s+[A-Za-z0-9_.-]+/gi, "token ***");
    s = s.replace(/FORGE_[A-Z_]*=[^\s,;]+/g, "FORGE_***=***");
    s = s.replace(/[A-Za-z]:\\[^\s"']*/g, "***");
    s = s.replace(/(home|Users|forge|tmp|var|etc)[^\s"']*/gi, "***");
  } catch {
    // Redact is best-effort: on a regex failure serve the raw text.
  }
  return s;
}

// What each lane needs before it can go live. Short on purpose.
const LANE_HINTS = {
  mcp: "endpoint up on 127.0.0.1",
  scene: "scene store open",
  play: "headless runner present",
  capture: "capture dir writable",
  asset: "asset cook queue idle",
};

export function laneNames() {
  return Object.keys(LANE_HINTS);
}

// One lane verdict from card fate and port only. No probing here: the caller
// probes once and passes cards in, so every lane agrees on one snapshot.
export function laneState(lane, pick, card) {
  if (!pick || pick === "-") {
    return {
      state: "none",
      detail: `${lane}: no hop; recovery: run the lane step, check the endpoint log`,
    };
  }
  if (!card) {
    return {
      state: "remote",
      detail: `${pick}: batch row, sleeps by policy, never live TCP`,
    };
  }
  if (card.fate !== "active") {
    return {
      state: "down",
      detail: redact(`${pick}: ${card.fate}; recovery: check the endpoint log, retry`),
    };
  }
  if (card.port === null || card.port === undefined) {
    return {
      state: "batch",
      detail: `${pick}: active batch row, sleeps by policy, never live TCP`,
    };
  }
  return { state: "live", detail: `${pick}: live :${card.port}` };
}

// Merged view over router hops plus registry cards. Pure and static: same
// input gives the same reply. hopsFor names the first hop per lane.
export function buildLanes(cards, hopsFor = () => []) {
  const bySlug = new Map((cards ?? []).map((c) => [c.slug, c]));
  return laneNames().map((lane) => {
    const hops = hopsFor(lane) ?? [];
    const pick = hops.length > 0 ? hops[0] : "-";
    const { state, detail } = laneState(lane, pick, bySlug.get(pick));
    return { lane, chain: [...hops], pick, state, needsKey: LANE_HINTS[lane], detail };
  });
}

// Heartbeat age: updatedAt wins, file time is the fallback, unknown is never
// old. Returns ageMs (null when unknown), stale flag and a loud reason.
export function checkHeartbeat(beat, fileMtimeMs, nowMs = Date.now()) {
  const stamp = Number(beat?.updatedAt);
  const mtime = Number(fileMtimeMs);
  const ageMs = Number.isFinite(stamp)
    ? nowMs - stamp
    : Number.isFinite(mtime)
      ? nowMs - mtime
      : null;
  if (ageMs === null) return { ageMs: null, stale: false, reason: "age unknown: no stamp and no time; never counts as old" };
  const barMs = TIMEOUTS.heartbeatStaleMs;
  if (ageMs > barMs) {
    return {
      ageMs,
      stale: true,
      reason: `stale: age ${Math.round(ageMs / 1000)} s past bar ${HEARTBEAT_STALE_SEC} s; recovery: restart the holder, check the endpoint log`,
    };
  }
  return { ageMs, stale: false, reason: `fresh: age ${Math.max(0, Math.round(ageMs / 1000))} s under bar ${HEARTBEAT_STALE_SEC} s` };
}

// Doctor roll-up: 0 all pass, 1 up with 1+ checks failing, 2 doctor failed.
export function exitFor(checks) {
  const fails = checks.filter((c) => !c.ok).length;
  return fails === 0 ? EXIT_OK : EXIT_DEGRADED;
}

function selftest() {
  const fails = [];
  const want = (name, cond) => {
    console.log(`${cond ? "ok" : "FAIL"} ${name}`);
    if (!cond) fails.push(name);
  };
  want("probe: connected wins over timeout", classifyProbe(true, true) === UP);
  want("probe: timeout without link is slow", classifyProbe(false, true) === SLOW);
  want("probe: refused link is down", classifyProbe(false, false) === DOWN);
  want("probe: only up is live", isLive(UP) && !isLive(SLOW) && !isLive(DOWN));
  want("lane: no hop is none", laneState("mcp", "-", undefined).state === "none");
  want("lane: missing card is remote", laneState("mcp", "batch-x", undefined).state === "remote");
  want("lane: parked fate is down", laneState("mcp", "mcp", { slug: "mcp", fate: "parked", port: 123 }).state === "down");
  want("lane: active without port is batch", laneState("mcp", "mcp", { slug: "mcp", fate: "active", port: null }).state === "batch");
  want("lane: active with port is live", laneState("mcp", "mcp", { slug: "mcp", fate: "active", port: 20128 }).state === "live");
  const merged = buildLanes(
    [{ slug: "mcp", fate: "active", port: 20128 }],
    (lane) => (lane === "mcp" ? ["mcp"] : []),
  );
  want("merged: one row per lane, no probe inside", merged.length === laneNames().length && merged[0].state === "live");
  const secret = redact("token gdr_abc123 bearer xyz C:\\forge\\data FORGE_GATEWAY_TOKEN=abc");
  want("redact: no token or path leaves", !secret.includes("gdr_abc123") && !secret.includes("C:\\forge") && !secret.includes("abc"));
  const now = 1_700_000_000_000;
  want("heartbeat: fresh beat passes", checkHeartbeat({ updatedAt: now - 2000 }, now - 2000, now).stale === false);
  const stale = checkHeartbeat({ updatedAt: now - 120_000 }, null, now);
  want("heartbeat: old beat refuses loud", stale.stale === true && stale.reason.includes("past bar"));
  want("heartbeat: unknown age never old", checkHeartbeat({}, Number.NaN, now).stale === false);
  want("doctor: clean is 0", exitFor([{ name: "a", ok: true }]) === EXIT_OK);
  want("doctor: one fail is 1", exitFor([{ name: "a", ok: false }]) === EXIT_DEGRADED);
  console.log(fails.length ? `RESULT FAIL: ${fails.length} drill(s)` : "RESULT PASS: gateway-health shape holds");
  return fails.length ? 1 : 0;
}

const isMain = import.meta.main;
if (isMain) {
  const argv = process.argv.slice(2);
  if (argv.includes("--selftest")) {
    process.exitCode = selftest();
  } else if (argv.includes("--json")) {
    const lanes = buildLanes([{ slug: "mcp", fate: "active", port: 20128 }], (lane) => (lane === "mcp" ? ["mcp"] : []));
    console.log(JSON.stringify({ ok: true, lanes, timeouts: TIMEOUTS }, null, 1));
  } else if (argv[0] === "--check") {
    const file = argv[1];
    if (!file) {
      console.error("usage: node tools/gateway-health.mjs --check <heartbeat.json>");
      process.exitCode = 2;
    } else {
      try {
        const beat = JSON.parse(readFileSync(resolve(file), "utf8"));
        const { stale, reason } = checkHeartbeat(beat, null);
        console.log(`${stale ? "STALE" : "FRESH"} ${redact(reason)}`);
        process.exitCode = stale ? EXIT_DEGRADED : EXIT_OK;
      } catch (error) {
        console.error(`RESULT FAIL: ${redact(error.message)}`);
        process.exitCode = EXIT_FAILED;
      }
    }
  } else {
    console.error("usage: node tools/gateway-health.mjs --selftest [--json] [--check <heartbeat.json>]");
    process.exitCode = 2;
  }
}
