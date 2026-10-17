// beat-lint: ladder beat chart gate (GP40, Rogers ch4).
//
// Job: read content/design/ladder.rows and fail a badly paced ladder, so a
// clumped rung cannot ship. Rules: each row's minutes and new-element total
// equal the compiled chart the game reads (`RUNG_MINUTES` and
// `RUNG_NEW_ELEMENTS` of crates/game/src/ladder.rs, the arrays `Ladder::beat()`
// returns), at most 3 new elements per rung, the enemy tier never falls, the
// boss flag sits only on the last rung, and rewards plus ranks match
// crates/game/src/ladder.rs (a second door to the same numbers, GP27).
//
// The first-bout launch cap is NOT a rule here. ladder.rs owns it
// (`FIRST_BOUT_START_SECS` 45 s, `FIRST_BOUT_START_LIMIT_SECS` 60 s) and its
// Rust test proves it, so this lint never reads a bout-length column as a
// launch time. The old rule did, and it forced the rows file to disagree with
// the compiled table to pass.
//
// Ladder numbers (ladder.rs:15-27): rungs 0..=7 (MAX_RUNG 7, 8 rows),
// glory per win 100 plus 25 per rung held, loop bonus 500 on the top-rung
// win, rank floors 0/300/900/1800/3600. A fresh climb wins rung r for
// 100 + 25*r glory; ranks per rung are 0,0,1,1,1,2,2,3.
//
// Rows file format (content/design/ladder.rows):
//   first non-blank non-# line: `table ladder version 1`
//   rows: `rung_<0..7> tier=<1..3> new_moves=<int> new_rules=<int>
//     new_arena=<int> reward=<int> penalty=<int> minutes=<int>
//     rank=<0..4> boss=<0|1>`; whole numbers only, `#` comments.
//
// Prints one line per rung (id, tier, new total, minutes, reward, rank).
// Exit 1 when a rule fails: minutes or new total off the compiled chart,
// clumped rung (over 3 new), falling tier, boss off the last rung, reward or
// rank drift from ladder.rs. Exit 2 on bad CLI, missing file or unparsable
// rows (with line number).
//
// Usage:
//   node tools/beat-lint.mjs [--ladder PATH] [--help]
// Default is the repo file; paths resolve against the repo root.
//
// Owner: game design seat. Units: glory counts, whole minutes. Deterministic:
// same file gives same lines (no clock, no HashMap order).
import { readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
const ROOT = dirname(HERE);

// Mirror of crates/game/src/ladder.rs:15-27. A drift here fails the lint
// on purpose: rows must equal the compiled table, never fork it.
export const MAX_RUNG = 7;
export const GLORY_PER_WIN = 100;
export const GLORY_RUNG_BONUS = 25;
export const GLORY_PER_LOOP = 500;
export const GLORY_RANKS = [0, 300, 900, 1800, 3600];
export const MAX_NEW_PER_RUNG = 3;

// Mirror of crates/game/src/ladder.rs:41,43. The game reads these arrays
// through `Ladder::beat()` and `rung_beat()`, so a rows file that forks them
// is a second beat chart, not a second door. Drift fails the lint on purpose.
export const RUNG_NEW_ELEMENTS = [2, 1, 2, 1, 2, 1, 2, 1];
export const RUNG_MINUTES = [2, 2, 3, 3, 4, 4, 5, 6];

export const DEFAULT_LADDER = join(ROOT, "content", "design", "ladder.rows");

const INT_RE = /^\d+$/;

function fail(msg) {
  throw new Error(msg);
}

/** Glory rank: highest index whose floor the glory reaches. Pure. */
export function rankOf(glory) {
  let rank = 0;
  for (let i = 0; i < GLORY_RANKS.length; i += 1) {
    if (glory >= GLORY_RANKS[i]) rank = i;
  }
  return rank;
}

/** Expected reward for rung r: 100 + 25*r (ladder.rs apply). Pure. */
export function rewardForRung(rung) {
  return GLORY_PER_WIN + GLORY_RUNG_BONUS * rung;
}

/** Parse ladder rows text. Throws with line number on any bad row. */
export function parseLadder(text, label = "ladder") {
  const lines = text.split(/\r?\n/);
  let headerLine = 0;
  let version = 0;
  const rows = [];
  const seen = new Set();
  for (let i = 0; i < lines.length; i += 1) {
    const n = i + 1;
    let line = lines[i];
    const hash = line.indexOf("#");
    if (hash >= 0) line = line.slice(0, hash);
    line = line.trim();
    if (line === "") continue;
    if (headerLine === 0) {
      const m = line.match(/^table\s+ladder\s+version\s+(\d+)$/);
      if (!m) fail(label + " line " + n + ": first row must be `table ladder version 1`");
      version = Number(m[1]);
      if (!Number.isInteger(version) || version <= 0) fail(label + " line " + n + ": bad version");
      headerLine = n;
      continue;
    }
    const parts = line.split(/\s+/);
    const id = parts[0];
    if (!id || id.includes("=")) fail(label + " line " + n + ": misses rung id");
    if (seen.has(id)) fail(label + " line " + n + ": duplicate row " + id);
    seen.add(id);
    const want = ["tier", "new_moves", "new_rules", "new_arena", "reward", "penalty", "minutes", "rank", "boss"];
    const got = {};
    for (const p of parts.slice(1)) {
      const eq = p.indexOf("=");
      if (eq < 0) fail(label + " line " + n + ": bad cell `" + p + "`");
      const k = p.slice(0, eq);
      const v = p.slice(eq + 1);
      if (!want.includes(k)) fail(label + " line " + n + ": unknown key " + k);
      if (got[k] !== undefined) fail(label + " line " + n + ": duplicate key " + k);
      if (!INT_RE.test(v)) fail(label + " line " + n + ": " + k + " must be a whole number, got `" + v + "`");
      got[k] = Number(v);
    }
    for (const k of want) {
      if (got[k] === undefined) fail(label + " line " + n + ": missing key " + k);
    }
    rows.push({ id, ...got, line: n });
  }
  if (headerLine === 0) fail(label + ": misses `table ladder version 1` header");
  if (rows.length === 0) fail(label + ": no rung rows");
  return { version, rows };
}

/** Full check. Returns rows plus failures (exit 1 when non-empty). */
export function checkAll({ ladderText }) {
  const failures = [];
  const set = parseLadder(ladderText);
  const rows = set.rows;

  // Exactly rungs 0..=MAX_RUNG in order.
  if (rows.length !== MAX_RUNG + 1) {
    failures.push("rung count " + rows.length + ", want " + (MAX_RUNG + 1));
  }
  for (let i = 0; i < rows.length; i += 1) {
    if (rows[i].id !== "rung_" + i) {
      failures.push("row " + (i + 1) + " id `" + rows[i].id + "`, want `rung_" + i + "`");
    }
  }

  let cumulative = 0;
  let prevTier = 0;
  for (let i = 0; i < rows.length; i += 1) {
    const r = rows[i];
    const tag = r.id + " (line " + r.line + ")";
    const fresh = cumulative + r.reward + (i === MAX_RUNG ? GLORY_PER_LOOP : 0);

    if (r.tier < 1 || r.tier > 3) failures.push(tag + ": tier " + r.tier + " outside 1..3");
    if (r.tier < prevTier) failures.push(tag + ": tier falls " + prevTier + " -> " + r.tier);
    prevTier = r.tier;

    const added = r.new_moves + r.new_rules + r.new_arena;
    if (added > MAX_NEW_PER_RUNG) {
      failures.push(tag + ": " + added + " new elements, at most " + MAX_NEW_PER_RUNG);
    }
    if (i < RUNG_NEW_ELEMENTS.length && added !== RUNG_NEW_ELEMENTS[i]) {
      failures.push(tag + ": " + added + " new elements, ladder.rs teaches " + RUNG_NEW_ELEMENTS[i]);
    }

    if (r.reward !== rewardForRung(i)) {
      failures.push(tag + ": reward " + r.reward + ", ladder.rs pays " + rewardForRung(i));
    }
    if (r.penalty < 0 || r.penalty > r.reward) {
      failures.push(tag + ": penalty " + r.penalty + " outside 0..reward");
    }
    if (r.rank !== rankOf(fresh)) {
      failures.push(tag + ": rank " + r.rank + ", cumulative " + fresh + " reaches " + rankOf(fresh));
    }
    if (r.rank < 0 || r.rank > GLORY_RANKS.length - 1) {
      failures.push(tag + ": rank " + r.rank + " outside 0..4");
    }
    if (r.minutes < 1 || r.minutes > 10) {
      failures.push(tag + ": minutes " + r.minutes + " outside 1..10");
    }
    if (i < RUNG_MINUTES.length && r.minutes !== RUNG_MINUTES[i]) {
      failures.push(tag + ": minutes " + r.minutes + ", ladder.rs expects " + RUNG_MINUTES[i]);
    }
    if (r.boss !== 0 && r.boss !== 1) failures.push(tag + ": boss must be 0|1");
    if (i === MAX_RUNG && r.boss !== 1) failures.push(tag + ": last rung must carry boss=1");
    if (i !== MAX_RUNG && r.boss === 1) failures.push(tag + ": boss=1 before the last rung");

    cumulative += r.reward;
  }

  return { version: set.version, rows, cumulative, failures };
}

export function formatLines(r) {
  return r.rows.map((row) => {
    const added = row.new_moves + row.new_rules + row.new_arena;
    return row.id + " tier " + row.tier + " new " + added + " minutes " + row.minutes + " reward " + row.reward + " rank " + row.rank;
  });
}

export function parseArgs(argv) {
  const out = { ladder: DEFAULT_LADDER, help: false };
  for (let i = 0; i < argv.length; i += 1) {
    const a = argv[i];
    if (a === "--ladder") {
      const v = argv[i + 1];
      if (v === undefined || v.startsWith("--")) return { bad: true };
      out.ladder = resolve(v);
      i += 1;
    } else if (a === "--help" || a === "-h") {
      out.help = true;
    } else {
      return { bad: true };
    }
  }
  return out;
}

export function helpText() {
  return [
    "Usage: node tools/beat-lint.mjs [--ladder PATH] [--help]",
    "  Check the ladder beat chart: compiled minutes and new totals, tier climb, boss last.",
    "  Prints one line per rung. Exit 1 when a rule fails.",
    "Default: content/design/ladder.rows.",
  ].join("\n");
}

export function main(argv = []) {
  const opts = parseArgs(argv);
  if (opts.bad) {
    process.stderr.write(helpText() + "\n");
    return 2;
  }
  if (opts.help) {
    process.stdout.write(helpText() + "\n");
    return 0;
  }
  let ladderText;
  try {
    ladderText = readFileSync(opts.ladder, "utf8");
  } catch (err) {
    process.stderr.write(String(opts.ladder) + ": cannot read ladder (" + (err && err.message ? err.message : err) + ")\n");
    return 2;
  }
  let r;
  try {
    r = checkAll({ ladderText });
  } catch (err) {
    process.stderr.write(String(err && err.message ? err.message : err) + "\n");
    return 2;
  }
  process.stdout.write(formatLines(r).join("\n") + "\n");
  if (r.failures.length > 0) {
    for (const f of r.failures) process.stderr.write("rule: " + f + "\n");
    return 1;
  }
  return 0;
}

if (import.meta.main) {
  process.exit(main(process.argv.slice(2)));
}
