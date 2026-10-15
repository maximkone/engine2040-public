// Run: node --test tools/mp6-check.test.mjs
import assert from "node:assert/strict";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { checkMp6, loadReceipts, RECEIPTS } from "./mp6-check.mjs";

// A receipt set shaped like `net-bout mp6` output, all checks green.
function greenReceipts() {
  const profile = (name) => ({
    bout: {
      profile: name, ticks: 600, converged: true, tape_replays_to_tip: true,
      hash_x2_identical: true, fight_events: { before_drop: 12, during_drop: 3, after_rejoin: 20 },
    },
    cps: {
      profile: name, ticks: 600,
      seats: [0, 1].map((slot) => ({ slot, checkpoints_equal_full_length: true, equals_server_tip: true, first_divergence: null })),
    },
    drop: { profile: name, granted: true, rejoin_ticks: 14, converged_mid_fight: true, fight_live_at_both_ends: true },
    dist: {
      profile: name,
      links: [{ from: "server", to: "client0", datagrams_sent: 9 }],
      rollback: [0, 1].map((slot) => ({ slot, rollback_max_ticks: 5, deeper_than_8_tick_window: 0 })),
    },
  });
  const lan = profile("lan");
  const wan = profile("wan");
  return {
    "played-bout.json": { lan: lan.bout, wan: wan.bout },
    "checkpoints.json": { lan: lan.cps, wan: wan.cps },
    "drop-rejoin.json": { lan: lan.drop, wan: wan.drop },
    "distributions.json": { resim_window_ticks: 8, profiles: [lan.dist, wan.dist] },
    "soak-hash.json": { bots: 16, ticks: 600, hash_run1: "00ff", hash_run2: "00ff", hash_x2_identical: true },
    "fault-matrix.json": { all_faults_hold_one_checkpoint: true, faults: [{ fault: "loss", x2_identical: true }] },
  };
}

test("a green receipt set passes every check", () => {
  const { passes, problems } = checkMp6(greenReceipts());
  assert.deepEqual(problems, []);
  assert.ok(passes.length >= 20, `${passes.length} checks held`);
});

test("a lost checkpoint seat fails the equality check", () => {
  const r = greenReceipts();
  r["checkpoints.json"].wan.seats[1].equals_server_tip = false;
  const { problems } = checkMp6(r);
  assert.ok(problems.some((line) => line.startsWith("wan checkpoints: full checkpoint equality")), problems.join("\n"));
});

test("a rollback deeper than the resim window fails the receipt", () => {
  const r = greenReceipts();
  r["distributions.json"].profiles[0].rollback[0].deeper_than_8_tick_window = 2;
  r["distributions.json"].profiles[0].rollback[0].rollback_max_ticks = 11;
  const { problems } = checkMp6(r);
  assert.ok(problems.some((line) => line.startsWith("resim window covers measured rollbacks") && line.includes("window=8") && line.includes("deepest=11") && line.includes("over=2")), problems.join("\n"));
});

test("soak must be 16 bots over 600 ticks and repeat", () => {
  const r = greenReceipts();
  r["soak-hash.json"].bots = 8;
  r["soak-hash.json"].hash_run2 = "00fe";
  const names = checkMp6(r).problems.map((line) => line.split(":")[0]);
  assert.ok(names.includes("soak"), names.join(", "));
});

test("a fault that does not repeat fails the matrix", () => {
  const r = greenReceipts();
  r["fault-matrix.json"].faults[0].x2_identical = false;
  const { problems } = checkMp6(r);
  assert.ok(problems.some((line) => line.startsWith("fault matrix: every fault repeats")));
});

test("loadReceipts lists missing files and parses present ones", () => {
  const dir = mkdtempSync(join(tmpdir(), "mp6-check-"));
  try {
    writeFileSync(join(dir, RECEIPTS[0]), JSON.stringify({ lan: { converged: true } }));
    const { receipts, missing } = loadReceipts(dir);
    assert.equal(receipts[RECEIPTS[0]].lan.converged, true);
    assert.deepEqual(missing, RECEIPTS.slice(1));
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});
