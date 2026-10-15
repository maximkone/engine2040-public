#!/usr/bin/env node
// rerun-timeline.mjs - Rerun-pattern per-tick timeline scrub, dependency-free.
// Pattern only from rerun-io/rerun@2c87e2e crates/top/re_sdk/src/recording_stream.rs
// (set_time_sequence timeline plus .rrd save; MIT/Apache-2.0, no SDK, no dep).
// Consumer: headless paired-rerun harness diffing two JSON tapes [{tick,hash}].
// Units: tick u64, hash u64 decimal string. No wall clock, no tick change (D5).
// Usage: node tools/rerun-timeline.mjs <a.json> <b.json>
//        node tools/rerun-timeline.mjs --self-check
import { readFileSync, writeFileSync } from "node:fs";
import { basename, dirname, join } from "node:path";
const MASK = (1n << 64n) - 1n;
const GAMMA = 0x9e3779b97f4a7c15n;
const C1 = 0xbf58476d1ce4e5b9n;
const C2 = 0x94d049bb133111ebn;
const FRAMES = 120;
const DIVERGE_TICK = 57;
function mix64(z) {
  let x = z & MASK;
  x = (((x ^ (x >> 30n)) * C1) & MASK);
  x = (((x ^ (x >> 27n)) * C2) & MASK);
  return x ^ (x >> 31n);
}
function splitmix64(z) { return mix64((z + GAMMA) & MASK); }
function record(seed, frames, divergeAt) {
  let state = BigInt(seed);
  const out = [];
  for (let tick = 0; tick < frames; tick++) {
    let hash = splitmix64(state);
    state = (state + GAMMA) & MASK;
    if (tick === divergeAt) hash = (hash ^ 1n) & MASK;
    out.push({ tick, hash: hash.toString() });
  }
  return out;
}
function normHash(value) { return BigInt(typeof value === "string" ? value : String(value)).toString(); }
function loadTape(path) {
  const raw = JSON.parse(readFileSync(path, "utf8"));
  if (!Array.isArray(raw)) throw new Error(`not a tape: ${path} must hold a JSON array`);
  return raw.map((row, index) => {
    if (row === null || typeof row !== "object" || !Number.isInteger(row.tick)) throw new Error(`bad row ${index} in ${path}: need {tick,hash}`);
    return { tick: row.tick, hash: normHash(row.hash) };
  });
}
function firstDivergent(a, b) {
  const n = Math.min(a.length, b.length);
  for (let i = 0; i < n; i++) if (a[i].tick !== b[i].tick || a[i].hash !== b[i].hash) return a[i].tick;
  if (a.length !== b.length) return (a.length > b.length ? a[n] : b[n]).tick;
  return null;
}
function diffRows(a, b) {
  const n = Math.min(a.length, b.length);
  let rows = Math.abs(a.length - b.length);
  for (let i = 0; i < n; i++) if (a[i].tick !== b[i].tick || a[i].hash !== b[i].hash) rows++;
  return rows;
}
function timeline(a, b) {
  const n = Math.max(a.length, b.length);
  const rows = [];
  for (let i = 0; i < n; i++) {
    const left = a[i] ?? null;
    const right = b[i] ?? null;
    const sequence = left !== null ? left.tick : right.tick;
    const same = left !== null && right !== null && left.tick === right.tick && left.hash === right.hash;
    rows.push({ timeline: "tick", sequence, a_hash: left?.hash ?? null, b_hash: right?.hash ?? null, divergent: !same });
  }
  return rows;
}
function report(a, b) {
  const first = firstDivergent(a, b);
  const rows = diffRows(a, b);
  return { first, rows, timeline: timeline(a, b) };
}
function selfCheck() {
  const a = record(2040, FRAMES, null);
  const b = record(2040, FRAMES, null);
  const c = record(2040, FRAMES, DIVERGE_TICK);
  const identical = report(a, b);
  const desync = report(a, c);
  console.log(`first_divergent=${desync.first === null ? "none" : desync.first}`);
  console.log(`replay_diff_rows=${identical.rows}`);
  const ok = desync.first === DIVERGE_TICK && identical.rows === 0 && identical.first === null;
  if (!ok) { console.error(`self-check FAIL: want first_divergent=57 replay_diff_rows=0`); process.exitCode = 1; }
}
function main(argv) {
  if (argv.includes("--self-check")) { selfCheck(); return; }
  if (argv.length !== 2) { console.error("usage: node tools/rerun-timeline.mjs <a.json> <b.json>"); process.exitCode = 2; return; }
  const a = loadTape(argv[0]);
  const b = loadTape(argv[1]);
  const result = report(a, b);
  const artifact = { tool: "rerun-timeline", timeline: "tick", first_divergent: result.first, replay_diff_rows: result.rows, rows: result.timeline };
  const outPath = join(dirname(argv[0]), `${basename(argv[0]).replace(/\.json$/i, "")}.scrub.json`);
  writeFileSync(outPath, `${JSON.stringify(artifact, null, 2)}\n`, "utf8");
  console.log(`first_divergent=${result.first === null ? "none" : result.first}`);
  console.log(`replay_diff_rows=${result.rows}`);
  console.log(`artifact=${outPath}`);
}
export { record, firstDivergent, diffRows, timeline, report, loadTape };
if (import.meta.main) main(process.argv.slice(2));
