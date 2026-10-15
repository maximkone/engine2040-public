// duration-fmt: shared duration pretty-print (raw-ms variants to 0).
//
// Attribution: idea port of chronotope/humantime (Apache-2.0, license
// verified) -- single pure format_duration largest-unit chain only,
// reimplemented here as a fresh JS port, zero donor bytes. See
// docs/DEPENDENCIES.md donor policy (idea/fresh-code only).
//
// Job: one pure formatter shared by tools so `1500` and `1.5s` print the
// same text everywhere. Pure: same input gives the same string; no wall
// clock, no IO in the pure functions.
//
// Usage:
//   node tools/duration-fmt.mjs [ms-or-token ...]
//   node tools/duration-fmt.mjs --self-test
//   node tools/duration-fmt.mjs --help
//
// Bare run (no args) prints PASS plus one formatted sample and exits 0.
// Inputs: a finite number means raw ms (`0`, `1500`); a string may carry
// an optional unit suffix (`ms|s|m|h|d`, case-insensitive, `0` == `0ms`).
// Output: single largest unit (`0ms`, `999ms`, `1s`, `1.5s`, `1m`, ...).

const MS_PER_S = 1000;
const MS_PER_M = 60 * MS_PER_S;
const MS_PER_H = 60 * MS_PER_M;
const MS_PER_D = 24 * MS_PER_H;

const TOKEN_RE = /^(-?\d+(?:\.\d+)?)\s*(ms|s|m|h|d)?$/i;

/** Unit multiplier in ms for a suffix (`""` and `ms` are raw ms). */
export function unitToMs(unit) {
  switch (`${unit ?? ""}`.toLowerCase()) {
    case "":
    case "ms":
      return 1;
    case "s":
      return MS_PER_S;
    case "m":
      return MS_PER_M;
    case "h":
      return MS_PER_H;
    case "d":
      return MS_PER_D;
    default:
      throw new RangeError(`unknown duration unit ${JSON.stringify(unit)}`);
  }
}

/**
 * Parse a raw-ms number or token string to integer ms (>= 0).
 * Accepts `0`, `"0"`, `"0ms"` (all to 0), `"1500"`, `"1.5s"`, `"2m"`.
 * Throws RangeError/TypeError on bad input; never NaN, never negative.
 */
export function parseDurationMs(input) {
  if (typeof input === "number") {
    if (!Number.isFinite(input)) throw new RangeError(`duration must be finite, got ${input}`);
    if (input < 0) throw new RangeError(`duration must be >= 0, got ${input}`);
    return Math.round(input);
  }
  const text = `${input ?? ""}`.trim();
  const m = TOKEN_RE.exec(text);
  if (!m) throw new RangeError(`bad duration ${JSON.stringify(input)} (want <number>[ms|s|m|h|d])`);
  const value = Number(m[1]);
  if (!Number.isFinite(value)) throw new RangeError(`bad duration ${JSON.stringify(input)}`);
  if (value < 0) throw new RangeError(`duration must be >= 0, got ${JSON.stringify(input)}`);
  return Math.round(value * unitToMs(m[2] ?? ""));
}

/** Format a quotient with up to 2 decimals, trimmed (`1.50` -> `1.5`). */
function trimAmount(value) {
  const rounded = Math.round(value * 100) / 100;
  if (Number.isInteger(rounded)) return `${rounded}`;
  return `${rounded}`.replace(/(\.\d*?)0+$/, "$1").replace(/\.$/, "");
}

/**
 * Pretty-print non-negative finite ms as one largest unit.
 * Pure: same ms gives the same string. `0` -> `"0ms"`.
 */
export function formatDuration(ms) {
  if (!Number.isFinite(ms)) throw new RangeError(`duration must be finite, got ${ms}`);
  if (ms < 0) throw new RangeError(`duration must be >= 0, got ${ms}`);
  const rounded = Math.round(ms);
  if (rounded === 0) return "0ms";
  if (rounded >= MS_PER_D) return `${trimAmount(rounded / MS_PER_D)}d`;
  if (rounded >= MS_PER_H) return `${trimAmount(rounded / MS_PER_H)}h`;
  if (rounded >= MS_PER_M) return `${trimAmount(rounded / MS_PER_M)}m`;
  if (rounded >= MS_PER_S) return `${trimAmount(rounded / MS_PER_S)}s`;
  return `${rounded}ms`;
}

/** Format any accepted input (raw-ms number or token string). */
export function formatDurationInput(input) {
  return formatDuration(parseDurationMs(input));
}

const PINNED = [
  [0, 0, "0ms"],
  ["0", 0, "0ms"],
  ["0ms", 0, "0ms"],
  ["0s", 0, "0ms"],
  [1, 1, "1ms"],
  [999, 999, "999ms"],
  [1000, 1000, "1s"],
  [1500, 1500, "1.5s"],
  ["1500", 1500, "1.5s"],
  ["1.5s", 1500, "1.5s"],
  [60000, 60000, "1m"],
  ["1m", 60000, "1m"],
  [90000, 90000, "1.5m"],
  [3600000, 3600000, "1h"],
  ["1h", 3600000, "1h"],
  [86400000, 86400000, "1d"],
  ["1d", 86400000, "1d"],
  ["2h", 7200000, "2h"],
];

const NEGATIVE_CASES = ["-1", "-1ms", "abc", "1x", "", null, undefined, NaN, -5, Infinity];

/** Run pinned samples; returns { pass, fail, lines }. Pure except lines. */
export function selfTest() {
  const lines = [];
  let pass = 0;
  let fail = 0;
  for (const [input, wantMs, wantText] of PINNED) {
    let gotMs = null;
    let gotText = null;
    let err = null;
    try {
      gotMs = parseDurationMs(input);
      gotText = formatDuration(gotMs);
    } catch (e) {
      err = e;
    }
    const ok = err === null && gotMs === wantMs && gotText === wantText;
    if (ok) {
      pass += 1;
      lines.push(`ok ${JSON.stringify(input)} -> ${gotMs}ms -> ${gotText}`);
    } else {
      fail += 1;
      lines.push(
        `FAIL ${JSON.stringify(input)} -> ${err ? `error ${err.message}` : `${gotMs}ms -> ${gotText}`} (want ${wantMs}ms -> ${wantText})`,
      );
    }
  }
  for (const bad of NEGATIVE_CASES) {
    let threw = false;
    try {
      parseDurationMs(bad);
    } catch {
      threw = true;
    }
    if (threw) {
      pass += 1;
      lines.push(`ok reject ${JSON.stringify(bad)}`);
    } else {
      fail += 1;
      lines.push(`FAIL accept ${JSON.stringify(bad)} (want throw)`);
    }
  }
  return { pass, fail, lines };
}

function printHelp() {
  console.log("Usage: node tools/duration-fmt.mjs [ms-or-token ...] [--self-test] [--help]");
  console.log("  number = raw ms; token = <number>[ms|s|m|h|d] (e.g. 1500, 1.5s, 2m).");
  console.log("  no args: PASS plus one formatted sample (exit 0).");
}

function main(argv) {
  const rest = [];
  let wantTest = false;
  for (const a of argv) {
    if (a === "--self-test") wantTest = true;
    else if (a === "--help" || a === "-h") {
      printHelp();
      process.exit(0);
    } else if (a.startsWith("-")) {
      console.error(`duration-fmt: error: unknown flag ${a}`);
      process.exit(2);
    } else rest.push(a);
  }
  if (wantTest) {
    const r = selfTest();
    for (const l of r.lines) console.log(l);
    console.log(r.fail === 0 ? `PASS duration-fmt self-test ${r.pass}/${r.pass + r.fail}` : `FAIL duration-fmt self-test ${r.pass} pass ${r.fail} fail`);
    process.exit(r.fail === 0 ? 0 : 1);
  }
  if (rest.length > 0) {
    try {
      for (const token of rest) console.log(formatDurationInput(token));
    } catch (e) {
      console.error(`duration-fmt: error: ${e.message}`);
      process.exit(2);
    }
    return;
  }
  const sampleMs = 90000;
  const sample = formatDuration(sampleMs);
  const r = selfTest();
  console.log(`PASS duration-fmt sample: ${sampleMs} -> ${sample} (self-test ${r.pass}/${r.pass + r.fail})`);
  process.exit(r.fail === 0 ? 0 : 1);
}

if (process.argv[1] && process.argv[1].replace(/\\/g, "/").endsWith("tools/duration-fmt.mjs")) {
  main(process.argv.slice(2));
}
