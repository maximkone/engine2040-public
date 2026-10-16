// Donor: gotestyourself/gotestsum cmd/rerunfails.go, Apache-2.0 (licence verified), pattern only.
// https://github.com/gotestyourself/gotestsum
// Flaky-test quarantine gate: quarantine tests failing >=2 times, fail closed
// on panic/data-race markers. Pattern only, no donor bytes.
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";

export const QUARANTINE_THRESHOLD = 2;

export const FLAKY_HELP = [
  "Usage: node tools/flaky-quarantine.mjs [--counts <file>] [--fail <name>=<n> ...] [--threshold <n>] [--self-test]",
  "       node tools/flaky-quarantine.mjs --help | -h",
  "",
  "Quarantine gate (default with no args runs the self-test):",
  "  node tools/flaky-quarantine.mjs",
  "  node tools/flaky-quarantine.mjs --self-test",
  "From a failure-count JSON file ({ test: count } or { failures: { test: count } }):",
  "  node tools/flaky-quarantine.mjs --counts target/flaky-counts.json",
  "From CLI counts:",
  "  node tools/flaky-quarantine.mjs --fail foo=2 --fail bar=1",
].join("\n");

// Pure CLI parse (exported for tests): mirrors tools/proof.mjs style,
// throws the usage line on unknown flags, exits 2 in main.
export function parseQuarantineArgs(argv) {
  const out = { counts: null, fails: [], threshold: QUARANTINE_THRESHOLD, selfTest: false, help: false };
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    if (arg === "--help" || arg === "-h") {
      out.help = true;
    } else if (arg === "--self-test") {
      out.selfTest = true;
    } else if (arg === "--counts") {
      const next = argv[i + 1];
      if (!next || next.startsWith("--")) throw new Error(FLAKY_HELP.split("\n")[0]);
      out.counts = next;
      i += 1;
    } else if (arg === "--fail") {
      const next = argv[i + 1];
      if (!next || !/^[^=]+=\d+$/.test(next)) throw new Error(FLAKY_HELP.split("\n")[0]);
      const parts = next.split("=");
      out.fails.push({ name: parts[0], count: Number(parts[1]) });
      i += 1;
    } else if (arg === "--threshold") {
      const next = argv[i + 1];
      if (!next || !/^\d+$/.test(next) || Number(next) < 1) throw new Error(FLAKY_HELP.split("\n")[0]);
      out.threshold = Number(next);
      i += 1;
    } else {
      throw new Error(FLAKY_HELP.split("\n")[0]);
    }
  }
  return out;
}

// Fatal markers abort the gate (fail closed, never quarantine silently).
export function hasFatalMarker(text) {
  const s = String(text == null ? "" : text);
  return /panic:/i.test(s) || /data race/i.test(s) || /DATA RACE/.test(s);
}

// Pure normalize of a failure-count JSON value (exported for tests).
export function normalizeCounts(json) {
  if (json == null || typeof json !== "object") return [];
  const src = json.failures && typeof json.failures === "object" ? json.failures : json;
  const rows = [];
  for (const entry of Object.entries(src)) {
    const name = entry[0];
    const n = Number(entry[1]);
    if (typeof name !== "string" || name === "") continue;
    if (Number.isFinite(n)) rows.push({ name: name, count: Math.floor(n) });
  }
  const extra = typeof json.output === "string" ? json.output : "";
  return { rows: rows, extra: extra };
}

// Pure quarantine verdict (exported for tests): names failing >= threshold.
export function quarantineList(rows, threshold) {
  const limit = threshold == null ? QUARANTINE_THRESHOLD : threshold;
  return rows
    .filter((r) => r.count >= limit)
    .map((r) => r.name)
    .sort();
}

function loadCountsFile(path) {
  const raw = readFileSync(path, "utf8");
  if (hasFatalMarker(raw)) {
    return { fatal: true, rows: [], extra: raw };
  }
  let parsed;
  try {
    parsed = JSON.parse(raw);
  } catch {
    throw new Error("unparsable failure-count JSON: " + path);
  }
  const norm = normalizeCounts(parsed);
  const rows = Array.isArray(norm) ? norm : norm.rows;
  const extra = Array.isArray(norm) ? "" : norm.extra;
  if (hasFatalMarker(extra)) return { fatal: true, rows: rows, extra: extra };
  return { fatal: false, rows: rows, extra: extra };
}

function reportQuarantine(rows, threshold) {
  const list = quarantineList(rows, threshold);
  for (const name of list) {
    const hit = rows.find((r) => r.name === name);
    console.log("[QUARANTINE] " + name + " (" + (hit ? hit.count : threshold) + " fails >= " + threshold + ")");
  }
  console.log("PASS quarantine=" + list.length + " threshold=" + threshold + " checked=" + rows.length);
  console.log(list.length ? "RESULT PASS (quarantined " + list.length + ")" : "RESULT PASS (nothing to quarantine)");
  return list.length;
}

function runSelfTest() {
  const rows = [
    { name: "example_pass_once", count: 1 },
    { name: "example_flaky_a", count: 2 },
    { name: "example_flaky_b", count: 3 },
  ];
  const quarantined = quarantineList(rows, QUARANTINE_THRESHOLD);
  const okQuarantine = quarantined.length === 2 && quarantined[0] === "example_flaky_a" && quarantined[1] === "example_flaky_b";
  const okFatal = hasFatalMarker("panic: runtime error") && hasFatalMarker("WARNING: DATA RACE") && !hasFatalMarker("ok all green");
  const ok = okQuarantine && okFatal;
  for (const name of quarantined) console.log("[QUARANTINE] " + name);
  console.log(ok ? "PASS quarantine=" + quarantined.length + " self-test" : "FAIL self-test quarantine=" + quarantined.length);
  console.log(ok ? "RESULT PASS (self-test)" : "RESULT FAIL (self-test)");
  return ok;
}

function main() {
  const raw = process.argv.slice(2);
  if (raw.includes("--help") || raw.includes("-h")) {
    console.log(FLAKY_HELP);
    process.exit(0);
  }
  if (raw.length === 0) {
    const ok = runSelfTest();
    process.exit(ok ? 0 : 1);
  }
  let args;
  try {
    args = parseQuarantineArgs(raw);
  } catch (error) {
    console.error(error.message);
    process.exit(2);
  }
  if (args.help) {
    console.log(FLAKY_HELP);
    process.exit(0);
  }
  if (args.selfTest) {
    const ok = runSelfTest();
    process.exit(ok ? 0 : 1);
  }
  let rows = args.fails.slice();
  if (args.counts) {
    let loaded;
    try {
      loaded = loadCountsFile(args.counts);
    } catch (error) {
      console.error("FAIL " + error.message);
      process.exit(1);
    }
    if (loaded.fatal) {
      console.error("FAIL fatal marker (panic/data race): abort, quarantine nothing");
      process.exit(1);
    }
    rows = rows.concat(loaded.rows);
  }
  const blob = rows.map((r) => r.name + " " + r.count).join("\n");
  if (hasFatalMarker(blob)) {
    console.error("FAIL fatal marker (panic/data race): abort, quarantine nothing");
    process.exit(1);
  }
  if (!rows.length) {
    console.log("PASS quarantine=0 threshold=" + args.threshold + " checked=0");
    console.log("RESULT PASS (nothing to quarantine)");
    process.exit(0);
  }
  reportQuarantine(rows, args.threshold);
  process.exit(0);
}

if (import.meta.main) {
  main();
}

