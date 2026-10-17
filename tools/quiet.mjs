#!/usr/bin/env node
// quiet: run a timing command while no loop cargo build can start. Takes
// every cargo slot (the same target/cargo-slots files the keeper uses) as
// each one frees, for at most 8 min, runs the command, frees the slots.
// Coders wait for a slot meanwhile (the keeper waits up to 15 min).
//   node tools/quiet.mjs [--times N] [--out <path with {n}>] [--require-quiet] -- <exe> [args]
// Status goes to stderr: `QUIET` when every slot was held, else
// `LOADED k of N slots busy (...)`; the command still runs and the perf
// seat states the load next to its numbers. Exit = the last run's exit.
// 2026-10-17: the same verdict line, the UTC time and the command line also
// go to the `-load` sidecar (out with {n} -> `load`) before the first run, so
// a reader who keeps only target/reports/ can tell a ranking-only LOADED
// window from a claimable QUIET one. With `--require-quiet` a LOADED verdict
// exits 2 after the sidecar and the command never runs.
// 2026-10-10: perf BLOCKED whenever a build ran, and with 3 coders some
// build always runs, so frame cost was never measured.
import { readFileSync, writeFileSync, mkdirSync } from "node:fs";
import { spawnSync, execFileSync } from "node:child_process";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { acquireSlot, releaseSlot, listSlots, busyLine, knob } from "../.opencode/seat-guard.mjs";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const DIR = join(ROOT, "target", "cargo-slots");
const GRAB_MS = 8 * 60_000;
const RUN_MS = 10 * 60_000;

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const builds = () => {
  try {
    return execFileSync("tasklist", ["/FO", "CSV", "/NH"], { encoding: "utf8" }).split("\n").filter((l) => /^"(cargo|rustc)\.exe"/i.test(l)).length;
  } catch {
    return 0;
  }
};

export const parseArgs = (argv) => {
  const sep = argv.indexOf("--");
  if (sep < 0 || sep === argv.length - 1) return null;
  const opts = argv.slice(0, sep);
  const at = (f) => { const i = opts.indexOf(f); return i < 0 ? null : opts[i + 1]; };
  const times = Math.max(1, Math.min(10, Number(at("--times") ?? 1) || 1));
  return { times, out: at("--out"), cmd: argv[sep + 1], args: argv.slice(sep + 2) };
};

// `--require-quiet` is read here, never in parseArgs: its four-key shape is
// pinned by tools/quiet.test.mjs and tools/bins-quiet.test.mjs. Only the part
// before `--` is ours; the same flag after it belongs to the command.
export const wantQuiet = (argv) => {
  const sep = argv.indexOf("--");
  return (sep < 0 ? argv : argv.slice(0, sep)).includes("--require-quiet");
};

// Built once and used twice: the stderr line as before, and the sidecar stamp.
export const verdict = ({ held, max, running, others }) => (held === max && running === 0
  ? `QUIET (${max} of ${max} cargo slots held, no cargo or rustc running)`
  : `LOADED ${others.length} of ${max} slots busy, ${running} cargo/rustc running (${busyLine(others)})`);

// Real calls pass an `--out` carrying {n}, so the sidecar lands beside the
// CSVs; an out without {n} would keep its own path (pinned by the test).
export const sidecarPath = (out) => out.replace("{n}", "load");
export const sidecarText = (line, cmd, args, at) => `${line}\n${at}\n${[cmd, ...args].join(" ")}\n`;
export const refused = (line, require) => require === true && !line.startsWith("QUIET");

const main = async () => {
  const argv = process.argv.slice(2);
  const a = parseArgs(argv);
  if (!a) {
    console.error("usage: node tools/quiet.mjs [--times N] [--out <path with {n}>] [--require-quiet] -- <exe> [args]");
    process.exitCode = 2;
    return;
  }
  let knobs = {};
  try { knobs = JSON.parse(readFileSync(join(ROOT, ".opencode", "knobs.json"), "utf8")); } catch { /* default */ }
  const max = Math.max(1, Number(knob(knobs, "cargo_slots", 3)) || 3);
  const id = `quiet-${process.pid}`;
  const held = new Set();
  const free = () => releaseSlot({ dir: DIR, id });
  process.on("SIGINT", () => { free(); process.exit(130); });
  try {
    const start = Date.now();
    while (held.size < max && Date.now() - start < GRAB_MS) {
      const got = acquireSlot({ dir: DIR, max, id, info: { seat: "perf-quiet", cmd: a.cmd, pid: process.pid } });
      if (got.ok) { held.add(got.slot); continue; }
      await sleep(10_000);
    }
    // Builds started before the grab finish on their own; give them 3 min.
    for (let w = 0; held.size === max && builds() > 0 && w < 18; w++) await sleep(10_000);
    const others = listSlots(DIR).filter((s) => s.id !== id);
    const left = builds();
    const line = verdict({ held: held.size, max, running: left, others });
    console.error(line);
    // The sidecar lands before the first run: a report that keeps only the CSVs
    // still says which window produced them.
    if (a.out) {
      const side = join(ROOT, sidecarPath(a.out));
      mkdirSync(dirname(side), { recursive: true });
      writeFileSync(side, sidecarText(line, a.cmd, a.args, new Date().toISOString().replace(/\.\d+Z$/, "Z")));
      console.error(`load -> ${sidecarPath(a.out)}`);
    }
    if (refused(line, wantQuiet(argv))) {
      console.error("--require-quiet: refusing to run the command in a LOADED window (exit 2)");
      process.exitCode = 2;
      return;
    }
    let code = 0;
    for (let n = 1; n <= a.times; n++) {
      const r = spawnSync(a.cmd, a.args, { cwd: ROOT, encoding: "utf8", timeout: RUN_MS, maxBuffer: 64 * 1024 * 1024, stdio: a.out ? ["ignore", "pipe", "inherit"] : "inherit" });
      if (r.error) { console.error(`run ${n}: ${r.error.message}`); code = 1; break; }
      if (a.out) {
        const file = join(ROOT, a.out.replace("{n}", String(n)));
        mkdirSync(dirname(file), { recursive: true });
        writeFileSync(file, r.stdout ?? "");
        console.error(`run ${n}: exit ${r.status} -> ${a.out.replace("{n}", String(n))}`);
      }
      code = r.status ?? 1;
    }
    process.exitCode = code;
  } finally {
    free();
  }
};

if (import.meta.main) await main();
