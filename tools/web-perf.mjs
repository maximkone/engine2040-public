// QW10: the web frame's draw calls and CPU milliseconds, off versus on, from one page run.
//   node tools/web-perf.mjs [--capture out/web-perf/arena-capture.json]
// The browse step loads `index.html?bundle=ab&freeze` with --page-query: the page alternates a
// block that rebuilds the stand-in draw list every frame (off) with a block that replays the list
// recorded once (on), captures one frozen frame in each of the first two blocks, and prints the
// numbers under `perf` in its receipt. This tool reads that receipt, prints one line
//   web-perf: mode=ab block=20 off_frames=60 on_frames=60 draws=14/14 off_cpu_p50=0.412
//             on_cpu_p50=0.398 delta_cpu=0.014ms list_off=7.2us list_on=0.9us pixels=EQUAL
// and fails when the arms drew different pictures or different draw counts, or when the CPU did
// not fall: the row's own gate says the replay is kept only if the printed milliseconds fall.
// Exit: 0 measured and the CPU fell, 1 a failed check, 2 could not run.
import { appendFileSync, existsSync, readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
// Fewest frames an arm must hold before its median means anything (the page's own block count).
export const FEWEST_FRAMES = 20;

export class PerfError extends Error {}

// The receipt object of an arena capture file (what web-browse.mjs writes as arena-capture.json).
export function receiptOfCapture(text) {
  let j;
  try { j = JSON.parse(text); } catch (e) { throw new PerfError(`the capture file is not JSON: ${e.message}`); }
  if (!j || typeof j !== "object" || Array.isArray(j)) throw new PerfError("the capture file is not a JSON object");
  if (typeof j.receiptText !== "string" || !j.receiptText) throw new PerfError("the capture file has no receiptText (the page printed no receipt)");
  let r;
  try { r = JSON.parse(j.receiptText); } catch (e) { throw new PerfError(`the page's receipt is not JSON: ${e.message}`); }
  if (!r || typeof r !== "object" || Array.isArray(r)) throw new PerfError("the page's receipt is not a JSON object");
  return r;
}

// The `perf` object of a receipt, with the numbers this gate reads checked for presence.
export function perfOf(receipt) {
  const p = receipt?.perf;
  if (!p || typeof p !== "object") throw new PerfError("the receipt carries no perf object (the page did not run ?bundle=ab)");
  const num = (v, what) => {
    if (typeof v !== "number" || !Number.isFinite(v) || v < 0) throw new PerfError(`${what} is not a number (${JSON.stringify(v)})`);
    return v;
  };
  const signed = (v, what) => {
    if (typeof v !== "number" || !Number.isFinite(v)) throw new PerfError(`${what} is not a number (${JSON.stringify(v)})`);
    return v;
  };
  const pixels = p.pixels;
  if (!pixels || typeof pixels !== "object") throw new PerfError("the receipt has no pixels object (the page captured no frame per arm)");
  const armJson = (name) => {
    const a = p[name];
    if (!a || typeof a !== "object") throw new PerfError(`the receipt has no ${name} arm`);
    return {
      frames: num(a.frames, `${name}.frames`),
      cpu_p50_ms: num(a.cpuP50Ms, `${name}.cpuP50Ms`),
      cpu_p95_ms: num(a.cpuP95Ms, `${name}.cpuP95Ms`),
      list_p50_us: num(a.listP50Us, `${name}.listP50Us`),
      draws: num(a.draws, `${name}.draws`),
    };
  };
  const off = armJson("off");
  const on = armJson("on");
  return {
    mode: String(p.mode),
    block: num(p.block, "block"),
    off,
    on,
    delta_cpu_ms: signed(p.deltaCpuMs, "deltaCpuMs"),
    delta_list_us: signed(p.deltaListUs, "deltaListUs"),
    draws_equal: p.drawsEqual === true,
    pixels: { off: String(pixels.off), on: String(pixels.on), equal: pixels.equal === true },
  };
}

const ms = (v) => v.toFixed(3);
const us = (v) => v.toFixed(1);

// The printed line.
export const formatLine = (p) => `web-perf: mode=${p.mode} block=${p.block} off_frames=${p.off.frames} on_frames=${p.on.frames} draws=${p.off.draws}/${p.on.draws} off_cpu_p50=${ms(p.off.cpu_p50_ms)} on_cpu_p50=${ms(p.on.cpu_p50_ms)} delta_cpu=${ms(p.delta_cpu_ms)}ms list_off=${us(p.off.list_p50_us)}us list_on=${us(p.on.list_p50_us)}us pixels=${p.pixels.equal ? "EQUAL" : "DIFFERENT"}`;

// Every check the row asks for: one run, both arms, the same picture, the same draw calls, and a
// CPU median that actually fell.
export function verdict(p) {
  const failures = [];
  if (p.mode !== "ab") failures.push(`the receipt's perf mode is ${p.mode}; the page must run ?bundle=ab`);
  for (const [name, arm] of [["off", p.off], ["on", p.on]]) {
    if (arm.frames < FEWEST_FRAMES) failures.push(`the ${name} arm measured ${arm.frames} frames, fewer than ${FEWEST_FRAMES}`);
  }
  if (!p.draws_equal || p.off.draws !== p.on.draws) {
    failures.push(`the arms issued different draw counts (off ${p.off.draws}, on ${p.on.draws})`);
  }
  if (!p.pixels.equal) {
    failures.push(`the two captures differ (off ${p.pixels.off}, on ${p.pixels.on}): the replay does not draw the same picture`);
  }
  if (!(p.on.cpu_p50_ms < p.off.cpu_p50_ms)) {
    failures.push(`no win: the on arm's cpu p50 ${ms(p.on.cpu_p50_ms)} ms is not below the off arm's ${ms(p.off.cpu_p50_ms)} ms`);
  }
  return { ok: failures.length === 0, failures };
}

export function parseArgs(argv) {
  const o = { capture: join(root, "out", "web-perf", "arena-capture.json"), error: null };
  for (let i = 0; i < argv.length; i++) {
    if (argv[i] === "--capture") o.capture = argv[++i];
    else o.error = `unknown argument ${argv[i]}`;
  }
  if (argv.includes("--capture") && !o.capture) o.error ??= "--capture needs a value";
  return o;
}

export function main(argv, say = console.log, fail = console.error) {
  const a = parseArgs(argv);
  if (a.error) {
    fail(`web-perf: ${a.error}\nusage: node tools/web-perf.mjs [--capture out/web-perf/arena-capture.json]`);
    return 2;
  }
  try {
    const path = resolve(a.capture);
    if (!existsSync(path)) throw new PerfError(`${path} does not exist (the browse step writes it with --page-query bundle=ab)`);
    const perf = perfOf(receiptOfCapture(readFileSync(path, "utf8")));
    say(formatLine(perf));
    const v = verdict(perf);
    for (const f of v.failures) fail(`web-perf: FAIL ${f}`);
    if (process.env.GITHUB_STEP_SUMMARY) {
      const row = (name, arm) => `| ${name} | ${arm.frames} | ${ms(arm.cpu_p50_ms)} | ${ms(arm.cpu_p95_ms)} | ${us(arm.list_p50_us)} | ${arm.draws} |`;
      const rows = [row("off (rebuild)", perf.off), row("on (replay)", perf.on)].join("\n");
      const summary = `### web-perf ${v.ok ? "ok" : "FAILED"}\n| arm | frames | cpu p50 ms | cpu p95 ms | list p50 us | draws |\n|---|---|---|---|---|---|\n${rows}\n\npixels ${perf.pixels.equal ? "equal" : "DIFFERENT"} (off ${perf.pixels.off}, on ${perf.pixels.on})\n`;
      appendFileSync(process.env.GITHUB_STEP_SUMMARY, summary);
    }
    return v.ok ? 0 : 1;
  } catch (e) {
    if (!(e instanceof PerfError)) throw e;
    fail(`web-perf: ${e.message}`);
    return 2;
  }
}

if (import.meta.main) process.exit(main(process.argv.slice(2)));
