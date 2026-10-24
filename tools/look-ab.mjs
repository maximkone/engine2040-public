// look-ab: one command proves a look pass changes the picture — the same seed
// and tick captured with the pass off and the pass on, then the pixel delta.
//
// Usage:
//   node tools/look-ab.mjs <pass> [--seed N] [--ticks N] [--fighters N] [--every N]
//                               [--out <dir>] [--exe <arena.exe>] [--json]
//   node tools/look-ab.mjs --off <off.png> --on <on.png>        (compare an existing pair)
//   node tools/look-ab.mjs --list                               (the passes it can switch)
//
// Every capture goes through `tools/arena-run.mjs`, which holds the one arena
// window lock; this tool never starts arena.exe itself. The pair runs under
// one lock, so nothing else draws between them. Both runs get the same
// `--seed`, `--ticks`, `--fighters` and capture cadence; only the pass switch
// differs, which is what makes the delta the pass's own.
//
// Output: one `LOOK-AB <pass>` line, then the off and on frame, the mean
// per-pixel max-channel delta (0..255), the max, and the share of pixels that
// changed. A pair with a zero delta is REFUSED (exit 1): a pass that draws no
// pixel is not evidence, never "LIVE". Exit 2 on a usage error, a missing
// capture dir or an undecodable PNG; exit 3 when a capture run fails.
//
// Pixels: the builtin decoder in `tools/frame-moved.mjs` (node's zlib), so the
// tool runs with no vendor install. Donors of that decoder: pngjs (MIT).
import { spawnSync } from "node:child_process";
import { existsSync, mkdirSync, readdirSync } from "node:fs";
import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { basename, dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { decodePng } from "./frame-moved.mjs";

const TOOLS = dirname(fileURLToPath(import.meta.url));
const ROOT = dirname(TOOLS);
const ARENA_RUN = join(TOOLS, "arena-run.mjs");

// Each pass names the switch that turns it OFF beside the shipped default that
// leaves it ON. A flag is one arena argument; an env is one variable and the
// two values the game parses (`crates/game/src/windowed.rs` reads them, and
// each parser takes an exact token). Nothing here guesses a switch: a pass the
// arena cannot turn off is not in this table.
export const PASSES = Object.freeze({
  decals: { off: { flag: "--no-break-look" }, what: "debris and decals of the broken crate" },
  ragdoll: { off: { flag: "--no-ragdoll" }, what: "the floored-fighter ragdoll body" },
  msaa: { env: "E2040_MSAA", on: "1", off: "0", what: "4x MSAA targets" },
  "contact-shadow": { env: "E2040_CONTACT_SHADOW", on: "1", off: "0", what: "contact shadows" },
  "auto-exposure": { env: "E2040_AUTO_EXPOSURE", on: "1", off: "0", what: "histogram auto exposure" },
  night: { env: "E2040_NIGHT", on: "1", off: "0", what: "torchlit night preset" },
});

// Arena arguments both runs share. The capture line is the one the runbook
// names, so a pair here is the same kind of frame every look row already takes.
export function captureArgs(pass, options = {}) {
  const { seed = 2040, ticks = 600, fighters = 4, every = 60, fps = 30, dir } = options;
  return [
    "--scene", "arena",
    "--fighters", String(fighters),
    "--seed", String(seed),
    "--ticks", String(ticks),
    "--exit",
    "--capture-dir", dir,
    "--capture-every", String(every),
    "--fps", String(fps),
  ];
}

// The one argument or variable that differs between the two runs.
export function switchFor(pass, side) {
  const spec = PASSES[pass];
  if (!spec) throw new UsageError(`unknown pass "${pass}"; known: ${Object.keys(PASSES).join(", ")}`);
  if (spec.env) return { env: { [spec.env]: side === "off" ? spec.off : spec.on } };
  return { extra: side === "off" ? [spec.off.flag] : [] };
}

// Compare two decoded RGBA frames: per-pixel max channel delta over RGB, its
// mean, its max and the share of pixels where any channel moved.
export function compareFrames(off, on) {
  if (off.width !== on.width || off.height !== on.height) {
    throw new UsageError(`frame sizes differ: ${off.width}x${off.height} vs ${on.width}x${on.height}`);
  }
  const pixels = off.width * off.height;
  let sum = 0;
  let max = 0;
  let changed = 0;
  for (let i = 0; i < pixels; i += 1) {
    const o = i * 4;
    const dr = Math.abs(off.data[o] - on.data[o]);
    const dg = Math.abs(off.data[o + 1] - on.data[o + 1]);
    const db = Math.abs(off.data[o + 2] - on.data[o + 2]);
    const delta = Math.max(dr, dg, db);
    sum += delta;
    if (delta > max) max = delta;
    if (delta > 0) changed += 1;
  }
  return {
    width: off.width,
    height: off.height,
    pixels,
    mean: pixels === 0 ? 0 : sum / pixels,
    max,
    changed,
    share: pixels === 0 ? 0 : changed / pixels,
  };
}

// A pair proves the pass only when at least one pixel moved. The wording is the
// ledger line other tools grep, so it stays stable.
export function verdict(pass, delta) {
  if (delta.changed === 0) {
    return { live: false, line: `REFUSED ${pass}: the two frames are identical (0 pixels differ, max 0) — a pass that draws no pixel is not evidence` };
  }
  return { live: true, line: `PASS ${pass}: ${delta.changed} of ${delta.pixels} pixels differ (${(delta.share * 100).toFixed(2)}%), mean ${delta.mean.toFixed(3)}, max ${delta.max}` };
}

export class UsageError extends Error {}

export function sha8(bytes) {
  return createHash("sha256").update(bytes).digest("hex").slice(0, 8);
}

// Newest frame_*.png in a capture dir, by name: the settled frame of the run.
export function newestFrame(dir) {
  if (!existsSync(dir)) throw new UsageError(`no capture dir at ${dir}`);
  const frames = readdirSync(dir).filter((f) => /^frame_\d+\.png$/.test(f)).sort();
  if (frames.length === 0) throw new UsageError(`no frame_*.png in ${dir}`);
  return join(dir, frames.at(-1));
}

function parseArgs(argv) {
  const options = { seed: 2040, ticks: 600, fighters: 4, every: 60, fps: 30, json: false, out: join(ROOT, "target", "look-ab") };
  let pass = null;
  let off = null;
  let on = null;
  let list = false;
  for (let i = 0; i < argv.length; i += 1) {
    const a = argv[i];
    const next = () => {
      const v = argv[i + 1];
      if (v === undefined) throw new UsageError(`${a} needs a value`);
      i += 1;
      return v;
    };
    if (a === "--off") off = next();
    else if (a === "--on") on = next();
    else if (a === "--out") options.out = next();
    else if (a === "--exe") options.exe = next();
    else if (a === "--json") options.json = true;
    else if (a === "--list") list = true;
    else if (a === "--seed") options.seed = Number(next());
    else if (a === "--ticks") options.ticks = Number(next());
    else if (a === "--fighters") options.fighters = Number(next());
    else if (a === "--every") options.every = Number(next());
    else if (a === "--fps") options.fps = Number(next());
    else if (a.startsWith("-")) throw new UsageError(`unknown option ${a}`);
    else if (pass === null) pass = a;
    else throw new UsageError(`one pass at a time; got "${pass}" and "${a}"`);
  }
  return { pass, off, on, list, options };
}

// Run one capture through the one-window lock. Returns the arena's exit code.
function capture(side, pass, options) {
  const dir = join(options.out, side);
  mkdirSync(dir, { recursive: true });
  const sw = switchFor(pass, side);
  const args = captureArgs(pass, { ...options, dir });
  const runArgs = ["--wait-s", "1800", ...(options.exe ? ["--exe", options.exe] : []), "--", ...args, ...(sw.extra ?? [])];
  const env = { ...process.env, ...(sw.env ?? {}) };
  const run = spawnSync(process.execPath, [ARENA_RUN, ...runArgs], { env, stdio: "inherit" });
  if (run.status !== 0) throw new UsageError(`the ${side} capture run failed (exit ${run.status}); nothing to compare`);
  return newestFrame(dir);
}

function main(argv) {
  const { pass, off, on, list, options } = parseArgs(argv);
  if (list) {
    for (const [name, spec] of Object.entries(PASSES)) {
      const how = spec.env ? `${spec.env}=${spec.off} (off) / ${spec.on} (on)` : spec.off.flag;
      process.stdout.write(`${name.padEnd(16)} ${how.padEnd(38)} ${spec.what}\n`);
    }
    return 0;
  }
  if (!pass && !(off && on)) throw new UsageError("give a pass name, or --off <png> --on <png>");
  const offPath = off ?? capture("off", pass, options);
  const onPath = on ?? capture("on", pass, options);
  const offBytes = readFileSync(offPath);
  const onBytes = readFileSync(onPath);
  const delta = compareFrames(decodePng(offBytes), decodePng(onBytes));
  const said = verdict(pass ?? basename(dirname(offPath)), delta);
  if (options.json) {
    process.stdout.write(`${JSON.stringify({ pass, off: offPath, on: onPath, offSha8: sha8(offBytes), onSha8: sha8(onBytes), ...delta, live: said.live }, null, 2)}\n`);
  } else {
    process.stdout.write(`LOOK-AB ${pass ?? basename(dirname(offPath))}\n`);
    process.stdout.write(`  off ${basename(offPath)} ${sha8(offBytes)}\n`);
    process.stdout.write(`  on  ${basename(onPath)} ${sha8(onBytes)}\n`);
    process.stdout.write(`  ${said.line}\n`);
  }
  return said.live ? 0 : 1;
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) {
  try {
    process.exitCode = main(process.argv.slice(2));
  } catch (err) {
    process.stderr.write(`look-ab: ${err.message}\n`);
    process.exitCode = 2;
  }
}
