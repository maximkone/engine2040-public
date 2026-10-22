// mp6-check: checks the MP6 receipts that `net-bout mp6 --out DIR` writes.
// Read-only: reads six JSON files from DIR (default target/mp6), never runs
// cargo, never writes. Exit 0 when every check holds, 1 when one fails, 2
// when a receipt is missing or is not JSON.
// Usage: node tools/mp6-check.mjs [DIR]
import { existsSync, readFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

export const RECEIPTS = Object.freeze([
  "played-bout.json",
  "checkpoints.json",
  "drop-rejoin.json",
  "distributions.json",
  "soak-hash.json",
  "fault-matrix.json",
]);
export const PROFILES = Object.freeze(["lan", "wan"]);
// Shape of the 16-bot x600 soak proof (net-bout mp6 runs exactly this).
export const SOAK_BOTS = 16;
export const SOAK_TICKS = 600;
// MP26: the clock slew cap, in permille of the tick rate (5 percent).
export const SLEW_CAP_PERMILLE = 50;
// MP26: the input delay the pacing measurement runs at (the default, 2).
export const PACING_DELAY_TICKS = 2;
// MP26: the rollback bar over 1000 ticks at the 2-frame delay.
export const PACING_ROLLBACK_BAR = 3;

// Read every receipt in `dir`. Missing files are listed, not thrown; bad JSON throws.
export function loadReceipts(dir) {
  const receipts = {};
  const missing = [];
  for (const name of RECEIPTS) {
    const path = join(dir, name);
    if (!existsSync(path)) {
      missing.push(name);
      continue;
    }
    receipts[name] = JSON.parse(readFileSync(path, "utf8"));
  }
  return { receipts, missing };
}

// Check one set of parsed receipts. Returns { passes, problems }: a pass is a
// labelled check that held, a problem is a labelled check that did not.
export function checkMp6(r) {
  const passes = [];
  const problems = [];
  const warnings = [];
  const check = (label, ok, detail = "") => {
    (ok ? passes : problems).push(detail ? `${label}: ${detail}` : label);
  };

  const bout = r["played-bout.json"];
  const cps = r["checkpoints.json"];
  const drop = r["drop-rejoin.json"];
  const dist = r["distributions.json"];
  const soak = r["soak-hash.json"];
  const matrix = r["fault-matrix.json"];

  for (const p of PROFILES) {
    const b = bout?.[p];
    check(`${p} bout converged`, b?.converged === true, `converged=${b?.converged}`);
    check(`${p} tape replays to tip`, b?.tape_replays_to_tip === true);
    check(`${p} bout hash x2`, b?.hash_x2_identical === true);
    check(`${p} blows before drop and after rejoin`, b?.fight_events?.before_drop > 0 && b?.fight_events?.after_rejoin > 0, `before=${b?.fight_events?.before_drop} after=${b?.fight_events?.after_rejoin}`);

    const c = cps?.[p];
    const seats = Array.isArray(c?.seats) ? c.seats : [];
    check(`${p} checkpoints: two seats`, seats.length === 2, `seats=${seats.length}`);
    check(`${p} checkpoints: full checkpoint equality`, seats.length > 0 && seats.every((s) => s.checkpoints_equal_full_length === true && s.equals_server_tip === true && s.first_divergence === null));
    check(`${p} checkpoints cover the bout`, c?.ticks === b?.ticks, `checkpoints=${c?.ticks} bout=${b?.ticks}`);

    const d = drop?.[p];
    check(`${p} drop: seat rejoined and was granted`, d?.granted === true && typeof d?.rejoin_ticks === "number");
    check(`${p} drop: converged mid-fight`, d?.converged_mid_fight === true && d?.fight_live_at_both_ends === true);
  }

  const profiles = Array.isArray(dist?.profiles) ? dist.profiles : [];
  check("distributions posted for lan and wan", PROFILES.every((p) => profiles.some((row) => row.profile === p && row.links?.length > 0)));
  // MP24: the adaptive input delay holds every live resimulation inside the
  // 8-tick window, so an overrun fails the receipt with the measured
  // numbers, never a warning. Overruns stay counted per seat.
  const deepest = Math.max(0, ...profiles.flatMap((row) => (row.rollback ?? []).map((s) => s.rollback_max_ticks ?? 0)));
  const over = profiles.reduce((sum, row) => sum + (row.rollback ?? []).reduce((acc, s) => acc + (s.deeper_than_8_tick_window ?? 0), 0), 0);
  check("resim window covers measured rollbacks", over === 0 && deepest <= dist?.resim_window_ticks, `window=${dist?.resim_window_ticks} deepest=${deepest} over=${over}`);

  // MP26: the clock slew the 30-frame advantage window asked for rides every
  // seat row, bounded by 5 percent, with no frame skipped. A receipt that
  // posts no slew data fails here.
  const seats = profiles.flatMap((row) =>
    (row.rollback ?? []).map((seat) => ({ profile: row.profile, ...seat })),
  );
  const posted = seats.length > 0 && seats.every(
    (seat) => typeof seat.max_slew_permille === "number"
      && typeof seat.slewed_frames === "number"
      && typeof seat.skipped_frames === "number",
  );
  check("every seat posts its clock slew", posted, `seats=${seats.length}`);
  const fastest = Math.max(0, ...seats.map((seat) => seat.max_slew_permille ?? 0));
  check("clock slew stays inside 5 percent", posted && fastest <= SLEW_CAP_PERMILLE, `fastest=${fastest} cap=${SLEW_CAP_PERMILLE}`);
  const skipped = seats.reduce((sum, seat) => sum + (seat.skipped_frames ?? 0), 0);
  check("no frame skipped", posted && skipped === 0, `skipped=${skipped}`);
  check("advantage window posted at 30 frames", dist?.advantage_window_frames === 30, `frames=${dist?.advantage_window_frames}`);

  // MP26: every WAN profile holds its live resimulations inside the window.
  const wanRows = profiles.filter((row) => row.profile === "wan");
  const wanOver = wanRows.reduce((sum, row) => sum + (row.rollback ?? []).reduce((acc, s) => acc + (s.deeper_than_8_tick_window ?? 0), 0), 0);
  check("wan profiles hold every resim inside the window", wanRows.length > 0 && wanOver === 0, `over=${wanOver}`);

  // MP26: the pacing measurement the bar names: under 3 rollbacks per 1000
  // ticks at the 2-frame delay.
  const pacing = dist?.pacing;
  check(
    "pacing measurement posted at 2-frame delay",
    pacing?.input_delay_ticks === PACING_DELAY_TICKS && typeof pacing?.rollbacks_per_1000_ticks === "number",
    `delay=${pacing?.input_delay_ticks} per_1000=${pacing?.rollbacks_per_1000_ticks}`,
  );
  check(
    "under 3 rollbacks per 1000 ticks at 2-frame delay",
    typeof pacing?.rollbacks_per_1000_ticks === "number" && pacing.rollbacks_per_1000_ticks < PACING_ROLLBACK_BAR,
    `per_1000=${pacing?.rollbacks_per_1000_ticks} bar=${PACING_ROLLBACK_BAR}`,
  );

  check("soak: 16 bots x600 ticks", soak?.bots === SOAK_BOTS && soak?.ticks === SOAK_TICKS, `bots=${soak?.bots} ticks=${soak?.ticks}`);
  check("soak: hashes x2 identical", soak?.hash_x2_identical === true && soak?.hash_run1 === soak?.hash_run2);

  const faults = Array.isArray(matrix?.faults) ? matrix.faults : [];
  check("fault matrix: one checkpoint across faults", matrix?.all_faults_hold_one_checkpoint === true && faults.length > 0);
  check("fault matrix: every fault repeats", faults.length > 0 && faults.every((f) => f.x2_identical === true));

  return { passes, problems, warnings };
}

function main(argv) {
  const dir = resolve(argv[0] ?? "target/mp6");
  let loaded;
  try {
    loaded = loadReceipts(dir);
  } catch (err) {
    console.error(`mp6-check: ${err.message}`);
    return 2;
  }
  if (loaded.missing.length > 0) {
    console.error(`mp6-check: missing in ${dir}: ${loaded.missing.join(", ")}`);
    return 2;
  }
  const { passes, problems, warnings } = checkMp6(loaded.receipts);
  for (const line of passes) console.log(`PASS ${line}`);
  for (const line of problems) console.log(`FAIL ${line}`);
  for (const line of warnings) console.log(`WARN ${line} (owner decision, see packet)`);
  const ok = problems.length === 0;
  console.log(`MP6 CHECK ${ok ? "PASS" : "FAIL"} (${passes.length} pass, ${problems.length} fail) ${dir}`);
  return ok ? 0 : 1;
}

if (import.meta.main) {
  process.exitCode = main(process.argv.slice(2));
}
