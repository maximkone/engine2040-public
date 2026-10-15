import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import {
  DOWN, EXIT_DEGRADED, EXIT_FAILED, EXIT_OK, HEARTBEAT_STALE_SEC,
  PROBE_BOUND_MS, PROBE_CACHE_TTL_SEC, SLOW, TIMEOUTS, UP,
  buildLanes, checkHeartbeat, classifyProbe, exitFor, isLive,
  laneNames, laneState, redact,
} from "./gateway-health.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));
const REPO_ROOT = resolve(HERE, "..");
const GH = join(HERE, "gateway-health.mjs");
const NOW = 1_700_000_000_000;

const runGh = (args) => {
  try {
    const out = execFileSync(process.execPath, [GH, ...args], {
      encoding: "utf8", cwd: REPO_ROOT, timeout: 60_000,
    });
    return { status: 0, out };
  } catch (error) {
    return { status: error.status ?? EXIT_FAILED, out: `${error.stdout ?? ""}${error.stderr ?? ""}` };
  }
};

test("probe: connected wins, timeout is slow, refused is down", () => {
  assert.equal(classifyProbe(true, true), UP);
  assert.equal(classifyProbe(true, false), UP);
  assert.equal(classifyProbe(false, true), SLOW);
  assert.equal(classifyProbe(false, false), DOWN);
  assert.equal(isLive(UP), true);
  assert.equal(isLive(SLOW), false);
  assert.equal(isLive(DOWN), false);
});

test("redact: no token, bearer secret, env value or local path leaves", () => {
  assert.ok(!redact("token gdr_abc123 bearer xyz C:\\forge\\data FORGE_GATEWAY_TOKEN=abc").includes("gdr_abc123"), "gateway token redacted");
  assert.ok(!redact("token gdr_abc123 bearer xyz C:\\forge\\data FORGE_GATEWAY_TOKEN=abc").includes("xyz"), "bearer value redacted");
  assert.ok(!redact("token gdr_abc123 bearer xyz C:\\forge\\data FORGE_GATEWAY_TOKEN=abc").includes("C:\\forge"), "windows path redacted");
  assert.ok(!redact("token gdr_abc123 bearer xyz C:\\forge\\data FORGE_GATEWAY_TOKEN=abc").includes("FORGE_GATEWAY_TOKEN=abc"), "env value redacted");
  assert.equal(redact("mcp: live :20128"), "mcp: live :20128");
});

test("laneState: one verdict per card fate, never a probe", () => {
  assert.equal(laneState("mcp", "-", undefined).state, "none");
  assert.equal(laneState("mcp", "batch-x", undefined).state, "remote");
  const parked = laneState("mcp", "mcp", { slug: "mcp", fate: "parked", port: 123 });
  assert.equal(parked.state, "down");
  assert.match(parked.detail, /recovery/);
  assert.equal(laneState("mcp", "mcp", { slug: "mcp", fate: "active", port: null }).state, "batch");
  assert.equal(laneState("mcp", "mcp", { slug: "mcp", fate: "active" }).state, "batch");
  const live = laneState("mcp", "mcp", { slug: "mcp", fate: "active", port: 20128 });
  assert.equal(live.state, "live");
  assert.match(live.detail, /20128/);
});

test("buildLanes: one row per lane with its need, same input same reply", () => {
  const cards = [{ slug: "mcp", fate: "active", port: 20128 }];
  const hopsFor = (lane) => (lane === "mcp" ? ["mcp"] : []);
  const first = buildLanes(cards, hopsFor);
  assert.equal(first.length, laneNames().length);
  assert.equal(first[0].lane, "mcp");
  assert.equal(first[0].state, "live");
  for (const row of first) {
    assert.ok(row.needsKey.length > 0, `${row.lane} names what it needs`);
    assert.ok(Array.isArray(row.chain), `${row.lane} keeps its chain`);
  }
  assert.deepEqual(buildLanes(cards, hopsFor), first);
});

test("checkHeartbeat: stamp wins, file time is the fallback, unknown is never old", () => {
  const fresh = checkHeartbeat({ updatedAt: NOW - 2000 }, NOW - 2000, NOW);
  assert.equal(fresh.stale, false);
  const stale = checkHeartbeat({ updatedAt: NOW - 120_000 }, null, NOW);
  assert.equal(stale.stale, true);
  assert.match(stale.reason, new RegExp(`past bar ${HEARTBEAT_STALE_SEC} s`));
  assert.match(stale.reason, /recovery/);
  const viaMtime = checkHeartbeat({}, NOW - 120_000, NOW);
  assert.equal(viaMtime.stale, true, "file time is the fallback stamp");
  const unknown = checkHeartbeat({}, Number.NaN, NOW);
  assert.deepEqual(unknown, {
    ageMs: null, stale: false,
    reason: "age unknown: no stamp and no time; never counts as old",
  });
});

test("exitFor: clean is 0, any fail is 1; bounds stay pinned", () => {
  assert.equal(exitFor([{ name: "a", ok: true }]), EXIT_OK);
  assert.equal(exitFor([]), EXIT_OK);
  assert.equal(exitFor([{ name: "a", ok: false }]), EXIT_DEGRADED);
  assert.deepEqual(TIMEOUTS, { probeMs: PROBE_BOUND_MS, heartbeatStaleMs: HEARTBEAT_STALE_SEC * 1000, probeCacheTtlSec: PROBE_CACHE_TTL_SEC });
  assert.equal(PROBE_BOUND_MS, 2000);
  assert.equal(HEARTBEAT_STALE_SEC, 60);
  assert.equal(PROBE_CACHE_TTL_SEC, 300);
});

test("--selftest drills green with exit 0", () => {
  const r = runGh(["--selftest"]);
  assert.equal(r.status, 0);
  assert.match(r.out, /RESULT PASS: gateway-health shape holds/);
});

test("--check reads a heartbeat file: fresh passes, old refuses loud", () => {
  const work = mkdtempSync(join(tmpdir(), "e2040-gateway-health-test-"));
  const freshFile = join(work, "fresh.json");
  writeFileSync(freshFile, JSON.stringify({ updatedAt: Date.now() - 2000 }));
  const fresh = runGh(["--check", freshFile]);
  assert.equal(fresh.status, EXIT_OK);
  assert.match(fresh.out, /FRESH/);
  const staleFile = join(work, "stale.json");
  writeFileSync(staleFile, JSON.stringify({ updatedAt: Date.now() - 120_000 }));
  const stale = runGh(["--check", staleFile]);
  assert.equal(stale.status, EXIT_DEGRADED);
  assert.match(stale.out, /STALE/);
  assert.match(stale.out, /recovery/);
  const missing = runGh(["--check", join(work, "missing.json")]);
  assert.equal(missing.status, EXIT_FAILED);
});

test("--json prints one merged view with timeouts", () => {
  const r = runGh(["--json"]);
  assert.equal(r.status, 0);
  const body = JSON.parse(r.out);
  assert.equal(body.ok, true);
  assert.equal(body.lanes.length, laneNames().length);
  assert.equal(body.timeouts.probeMs, 2000);
});
