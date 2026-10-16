// rerun-timeline.test.mjs - PF9 DEBUG-pattern leg spike (EXPERIMENT).
// Fixture: seeded 120-row tapes; pair A/B identical, pair C flips one hash
// bit at tick 57 (mirrors crates/observe/tests/it/w1_pf9_rerun.rs record()).
// Metric: first_divergent tick plus replay_diff_rows on the identical pair.
// Baseline: first_divergent=57, replay_diff_rows=0 (in-tree w1_pf9_rerun.rs:34).
import { describe, it } from "node:test";
import assert from "node:assert/strict";
import { mkdtempSync, writeFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { record, report, firstDivergent, diffRows, timeline, loadTape } from "./rerun-timeline.mjs";

const FRAMES = 120;
const DIVERGE_TICK = 57;

describe("rerun-timeline per-tick scrub", () => {
  it("seeded desync scrubs to tick 57 with one timeline row per tick", () => {
    const a = record(2040, FRAMES, null);
    const c = record(2040, FRAMES, DIVERGE_TICK);
    assert.equal(a.length, FRAMES);
    assert.equal(firstDivergent(a, c), DIVERGE_TICK);
    const desync = report(a, c);
    assert.equal(desync.first, DIVERGE_TICK);
    assert.equal(desync.timeline.length, FRAMES);
    for (let i = 0; i < FRAMES; i++) {
      assert.equal(desync.timeline[i].timeline, "tick");
      assert.equal(desync.timeline[i].sequence, i);
    }
    assert.equal(desync.timeline[DIVERGE_TICK].divergent, true);
    assert.equal(desync.timeline[0].divergent, false);
    console.log(`first_divergent=${desync.first}`);
  });

  it("identical pair replays with 0 diff rows via JSON tapes", () => {
    const a = record(2040, FRAMES, null);
    const b = record(2040, FRAMES, null);
    assert.equal(firstDivergent(a, b), null);
    assert.equal(diffRows(a, b), 0);
    const identical = report(a, b);
    assert.equal(identical.first, null);
    assert.equal(identical.rows, 0);
    assert.equal(timeline(a, b).every((row) => row.divergent === false), true);
    const dir = mkdtempSync(join(tmpdir(), "rerun-timeline-"));
    try {
      const aPath = join(dir, "a.json");
      const bPath = join(dir, "b.json");
      writeFileSync(aPath, JSON.stringify(a));
      writeFileSync(bPath, JSON.stringify(b));
      const ta = loadTape(aPath);
      const tb = loadTape(bPath);
      assert.equal(report(ta, tb).rows, 0);
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
    console.log(`replay_diff_rows=${identical.rows}`);
  });
});
