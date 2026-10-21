// Run: node --test tools/compile-cache-spike.test.mjs
// Pins the OPT9 cold-lane before/after math (idea shaped by mozilla/sccache,
// Apache-2.0; code written fresh, no donor bytes).
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import test from "node:test";
import { CELLS, cellEnv, delta, hitRate, laneArgs, parseArgs, statsTotals, verdict } from "./compile-cache-spike.mjs";

const TOOL = fileURLToPath(new URL("./compile-cache-spike.mjs", import.meta.url));

// A real 0.18.0 `sccache --show-stats --stats-format json` body, trimmed.
const STATS = {
  stats: {
    compile_requests: 7,
    requests_not_cacheable: 3,
    cache_hits: { counts: { Rust: 4 }, adv_counts: { rust: 4 } },
    cache_misses: { counts: { Rust: 6 }, adv_counts: { rust: 6 } },
  },
};

test("statsTotals sums counts once, never counts and adv_counts together", () => {
  assert.deepEqual(statsTotals(STATS), { hits: 4, misses: 6, requests: 7 });
  assert.deepEqual(statsTotals({ stats: { compile_requests: 0 } }), { hits: 0, misses: 0, requests: 0 });
});

test("delta reads one cell's traffic out of two snapshots", () => {
  const before = statsTotals(STATS);
  const after = statsTotals({
    stats: {
      compile_requests: 19,
      cache_hits: { counts: { Rust: 16 }, adv_counts: { rust: 16 } },
      cache_misses: { counts: { Rust: 9 } },
    },
  });
  assert.deepEqual(delta(before, after), { hits: 12, misses: 3, requests: 12 });
});

test("hitRate is hits over cacheable requests, unknown when nothing was asked", () => {
  assert.equal(hitRate({ hits: 12, misses: 3, requests: 15 }), 0.8);
  assert.equal(hitRate({ hits: 0, misses: 0, requests: 0 }), null);
});

test("verdict keeps the cache only at 30% faster, 80% hits and stable warm runs", () => {
  const keep = verdict({ coldSeconds: 400, primeSeconds: 400, warmSeconds: 100, warm2Seconds: 105, rate: 0.95 });
  assert.equal(keep.keep, true, keep.reasons.join("; "));
  assert.equal(keep.ratio, 0.25);
  const slow = verdict({ coldSeconds: 400, primeSeconds: 400, warmSeconds: 300, warm2Seconds: 300, rate: 0.95 });
  assert.equal(slow.keep, false);
  assert.match(slow.reasons.join("; "), /30%/);
  const cool = verdict({ coldSeconds: 400, primeSeconds: 400, warmSeconds: 100, warm2Seconds: 100, rate: 0.5 });
  assert.equal(cool.keep, false);
  assert.match(cool.reasons.join("; "), /hit rate/);
  const shaky = verdict({ coldSeconds: 400, primeSeconds: 400, warmSeconds: 100, warm2Seconds: 140, rate: 0.95 });
  assert.equal(shaky.keep, false);
  assert.match(shaky.reasons.join("; "), /10%/);
  const unmeasured = verdict({ coldSeconds: 0, primeSeconds: 0, warmSeconds: 0, warm2Seconds: 0, rate: null });
  assert.equal(unmeasured.keep, false);
  assert.equal(unmeasured.ratio, null);
});

test("the CI-shape rebuild is judged by the same 30% bar", () => {
  const fast = verdict({ coldSeconds: 400, primeSeconds: 400, rebuildSeconds: 100, warmSeconds: 100, warm2Seconds: 100, rate: 0.95 });
  assert.equal(fast.keep, true, fast.reasons.join("; "));
  assert.equal(fast.rebuildRatio, 0.25);
  const slow = verdict({ coldSeconds: 400, primeSeconds: 400, rebuildSeconds: 300, warmSeconds: 100, warm2Seconds: 100, rate: 0.95 });
  assert.equal(slow.keep, false);
  assert.match(slow.reasons.join("; "), /CI-shape rebuild/);
});

test("laneArgs is the lane's compile step with --no-run and no --release", () => {
  assert.deepEqual(laneArgs({ crates: ["engine-physics"], jobs: 4 }), [
    "test", "-j", "4", "--locked", "-p", "engine-physics", "--all-targets", "--no-run",
  ]);
  assert.deepEqual(laneArgs({ crates: ["engine-core", "engine-ecs"], jobs: 2 }).slice(-6), [
    "-p", "engine-core", "-p", "engine-ecs", "--all-targets", "--no-run",
  ]);
});

test("each cell owns a fresh target dir; only the cache cells wrap rustc", () => {
  const dirs = { run: "run", cache: "run/sccache", base: "C:/repo", port: 4399 };
  assert.deepEqual(CELLS, ["cold", "prime", "rebuild", "warm", "warm2"]);
  const cold = cellEnv("cold", dirs);
  assert.equal(cold.RUSTC_WRAPPER, undefined, "the baseline runs rustc directly");
  assert.equal(cold.SCCACHE_SERVER_PORT, undefined, "the baseline talks to no cache server");
  assert.match(cold.CARGO_TARGET_DIR, /cold$/);
  const targets = new Set();
  for (const cell of CELLS) {
    const env = cellEnv(cell, dirs);
    targets.add(env.CARGO_TARGET_DIR);
    if (cell === "cold") continue;
    assert.equal(env.RUSTC_WRAPPER, "sccache", `${cell} is opt-in per run`);
    assert.equal(env.CARGO_INCREMENTAL, "0", `${cell} cannot cache incremental artifacts`);
    assert.equal(env.SCCACHE_DIR, dirs.cache, `${cell} shares one cache`);
    assert.equal(env.SCCACHE_SERVER_PORT, "4399", `${cell} uses the spike's own server, never the box's`);
    assert.equal(env.SCCACHE_BASEDIRS, dirs.base);
  }
  // Every cell but the rebuild one owns a dir of its own; the rebuild cell is
  // the CI shape: the same path as prime, so the cache key still matches.
  assert.equal(targets.size, CELLS.length - 1);
  assert.equal(cellEnv("rebuild", dirs).CARGO_TARGET_DIR, cellEnv("prime", dirs).CARGO_TARGET_DIR);
});

test("parseArgs takes repeatable --lane and refuses unknown flags", () => {
  assert.deepEqual(parseArgs([]), { crates: ["engine-physics"], jobs: 4, json: false, dryRun: false, force: false, error: null });
  const a = parseArgs(["--lane", "engine-ecs", "--lane", "engine-input", "--jobs", "2", "--dry-run"]);
  assert.deepEqual(a.crates, ["engine-ecs", "engine-input"]);
  assert.equal(a.jobs, 2);
  assert.equal(a.dryRun, true);
  assert.equal(parseArgs(["--force"]).force, true);
  assert.match(parseArgs(["--nope"]).error, /unknown flag/);
  assert.match(parseArgs(["--jobs", "0"]).error, /--jobs/);
  assert.match(parseArgs(["--lane"]).error, /--lane/);
});

test("--dry-run prints the four cell lines and builds nothing", () => {
  const out = execFileSync(process.execPath, [TOOL, "--dry-run", "--lane", "engine-ecs"], { encoding: "utf8", timeout: 30000 });
  for (const cell of CELLS) assert.match(out, new RegExp(`\\[${cell}\\] cargo test`));
  assert.match(out, /RUSTC_WRAPPER=sccache/, "warm cells name the opt-in");
  assert.match(out, /no wrapper for the cold cell|baseline: no RUSTC_WRAPPER/i);
});

test("source carries the fresh-credit comment plus the licence line", () => {
  const src = readFileSync(TOOL, "utf8");
  assert.match(src, /mozilla\/sccache/);
  assert.match(src, /SPDX-License-Identifier: Apache-2\.0/);
  assert.match(src, /written fresh/);
});
