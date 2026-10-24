#!/usr/bin/env node
// aa-flicker: the AA default's temporal number, measured once on a GPU machine
// and read by product-check. The fixture is the render crate's own sand-edge
// test (`crates/render/src/present/tests_motion.rs` `sand_edge_aa_default`):
// a two-frame pair for the shipped TAA (Halton jitter plus the resolve) and for
// 4x MSAA, with the mean luma change over the edge band in levels.
// This tool runs that test on this PC, parses its printed line and writes
// target/reports/aa-flicker.json. It never invents a number: a skipped test
// (no adapter, or an adapter that cannot build the 4x pair) writes the reason.
//   node tools/aa-flicker.mjs [--json]
// Exit 0 wrote a reading or a named skip, 2 could not run (no cargo, no parse).
// The number is look evidence only: it never enters the tape, a hash or the tick.
import { execFileSync, spawnSync } from "node:child_process";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";import { hostname } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const OUT = join(ROOT, "target", "reports", "aa-flicker.json");
// The GPU test runs on this PC by design (a runner has no adapter); the GPU
// feature gates the presenter's offscreen twin. One test binary rebuild is the
// cost of a fresh number.
const TEST_MS = 20 * 60_000;
export const TEST_LINE = "cargo test -p engine-render --lib --features window sand_edge_aa_default -- --nocapture --test-threads=1";
export const SOURCE = "crates/render sand_edge_aa_default";

// The fixture's printed line: `sand edge aa default: taa (jitter+resolve) 10.60
// levels (edge step 201.68), msaa 4x 0.00 levels (edge step 201.68)`.
export const parseFixture = (text) => {
  const s = String(text ?? "");
  const skipped = s.match(/^SKIPPED (\S+): (.+)$/m);
  if (skipped) return { skipped: `${skipped[1]}: ${skipped[2].trim()}` };
  const line = s.match(/^sand edge aa default: .+$/m)?.[0] ?? null;
  if (!line) return { missing: "the fixture printed no `sand edge aa default:` line" };
  const nums = [...line.matchAll(/(-?\d+(?:\.\d+)?) levels(?: \(edge step (-?\d+(?:\.\d+)?)\))?/g)];
  if (nums.length < 2) return { missing: `the fixture line carries fewer than two readings: ${line}` };
  const taa = Number(nums[0][1]);
  const taaStep = nums[0][2] === undefined ? null : Number(nums[0][2]);
  const msaa = Number(nums[1][1]);
  const msaaStep = nums[1][2] === undefined ? null : Number(nums[1][2]);
  if (![taa, msaa].every(Number.isFinite)) return { missing: `a fixture reading is not a number: ${line}` };
  return { taa_levels: taa, msaa_levels: msaa, taa_edge_step: taaStep, msaa_edge_step: msaaStep, line };
};

// The file product-check reads. `calmer` names the shipped default the fixture
// proves; a tie names both rather than picking one. A fixture that printed
// nothing is `missing`: no reading, and the caller must not write one.
export const readingFrom = ({ text, sha, at = new Date().toISOString(), host = hostname() }) => {
  const parsed = parseFixture(text);
  if (parsed.skipped) return { sha, at, host, source: SOURCE, skipped: parsed.skipped };
  if (parsed.missing) return { sha, at, host, source: SOURCE, missing: parsed.missing };
  const calmer = parsed.msaa_levels < parsed.taa_levels ? "4x msaa" : parsed.msaa_levels > parsed.taa_levels ? "taa (jitter+resolve)" : "tie";
  return {
    sha, at, host, source: SOURCE,
    taa_levels: parsed.taa_levels,
    msaa_levels: parsed.msaa_levels,
    edge_step: parsed.taa_edge_step ?? parsed.msaa_edge_step ?? null,
    calmer,
  };
};

const git = (...a) => execFileSync("git", a, { cwd: ROOT, encoding: "utf8" }).trim();

const main = () => {
  const argv = process.argv.slice(2);
  const sha = git("rev-parse", "--short", "HEAD");
  const [cmd, ...args] = TEST_LINE.split(" ");
  const r = spawnSync(cmd, args, { cwd: ROOT, encoding: "utf8", timeout: TEST_MS, maxBuffer: 64 * 1024 * 1024, windowsHide: true });
  if (r.error?.code === "ENOENT") {
    console.log("aa-flicker: NOT RUN: cargo is not on PATH");
    process.exitCode = 2;
    return;
  }
  const text = `${r.stdout ?? ""}\n${r.stderr ?? ""}`;
  const reading = readingFrom({ text, sha });
  if (reading.missing) {
    // A fixture that never ran is not a reading: keep any earlier one and show
    // the tail so the failure names itself.
    const tail = text.trim().split(/\r?\n/).slice(-15).join("\n");
    console.log(`aa-flicker: NOT RUN: ${reading.missing}\n${tail}`);
    process.exitCode = 2;
    return;
  }
  mkdirSync(dirname(OUT), { recursive: true });
  writeFileSync(OUT, `${JSON.stringify(reading, null, 2)}\n`);
  if (reading.skipped) {
    // A skip is a named reason, not a pass and not a number.
    console.log(`aa-flicker: none (${reading.skipped})`);
  }
  if (argv.includes("--json")) process.stdout.write(`${JSON.stringify(reading, null, 2)}\n`);
  else if (!reading.skipped) {
    console.log(`aa-flicker: taa ${reading.taa_levels} levels, 4x msaa ${reading.msaa_levels} levels (edge step ${reading.edge_step}), calmer ${reading.calmer} at ${sha}`);
  }
};

// The reading file product-check reads, or null when it was never written.
export const readReading = (file = OUT) => {
  try { return JSON.parse(readFileSync(file, "utf8")); } catch { return null; }
};

if (import.meta.main) main();
