#!/usr/bin/env node
// quiet: run a timing command while no loop cargo build can start. Takes
// every cargo slot (the same target/cargo-slots files the keeper uses) as
// each one frees, for at most 8 min, runs the command, frees the slots.
// Coders wait for a slot meanwhile (the keeper waits up to 15 min).
//   node tools/quiet.mjs [--times N] [--out <path with {n}>] -- <exe> [args]
// Status goes to stderr: `QUIET` when every slot was held, else
// `LOADED k of N slots busy (...)`; the command still runs and the perf
// seat states the load next to its numbers. Exit = the last run's exit.
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

const main = async () => {
  const a = parseArgs(process.argv.slice(2));
  if (!a) {
    console.error("usage: node tools/quiet.mjs [--times N] [--out <path with {n}>] -- <exe> [args]");
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
    console.error(held.size === max && left === 0
      ? `QUIET (${max} of ${max} cargo slots held, no cargo or rustc running)`
      : `LOADED ${others.length} of ${max} slots busy, ${left} cargo/rustc running (${busyLine(others)})`);
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
