#!/usr/bin/env node
// potato-check: the potato gate G9 run (row GS16). Runs arena 8x600 with
// E2040_ADAPTER=integrated E2040_LIMITS=downlevel, samples working set with
// tasklist every 250 ms, prints the potato line plus one line per bar, and
// writes target/reports/potato-<date>.md. Bars in tools/potato-bar.json.
// Owner 2026-10-11. Units: ms, MB. Pure parsers give the same line x2.
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { execFileSync, spawn } from "node:child_process";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
export const BAR_PATH = join(ROOT, "tools", "potato-bar.json");

// First `adapter:` line in arena output, spaces to underscores so the
// potato line stays space-separated. Null when no adapter line exists.
export function parseAdapterName(output) {
  const line = String(output ?? "").split(/\r?\n/).find((l) => /^adapter:\s*\S/i.test(l));
  if (!line) return null;
  const rest = line.replace(/^adapter:\s*/i, "").trim();
  const noVendor = rest.split(/\s+vendor=/i)[0].trim();
  if (!noVendor) return null;
  return noVendor.replace(/\s+/g, "_");
}

// Integrated adapter name, or null when the output names no integrated
// adapter. Fails closed: no adapter line means no proof of integrated.
export function findIntegratedAdapter(output) {
  const text = String(output ?? "");
  const hasIntegrated = /integrated/i.test(text);
  if (!hasIntegrated) return null;
  return parseAdapterName(text);
}

// Frame p50/p99 from the last `arena frame-split:` line. Prefers
// `interval_ms` (wall interval) and falls back to `cpu_ms` (CPU work).
// Missing or unparsable numbers read null (UNKNOWN downstream).
export function parseFrameTimes(output) {
  const lines = String(output ?? "").split(/\r?\n/).filter((l) => l.startsWith("arena frame-split:"));
  if (lines.length === 0) return { p50_ms: null, p99_ms: null };
  const last = lines[lines.length - 1];
  const num = (s) => {
    const n = Number(s);
    return Number.isFinite(n) && n >= 0 ? n : null;
  };
  const interval = /interval_ms\s+p50=([\d.]+)\s+p99=([\d.]+)/.exec(last);
  if (interval) {
    const p50 = num(interval[1]);
    const p99 = num(interval[2]);
    if (p50 !== null && p99 !== null) return { p50_ms: p50, p99_ms: p99 };
  }
  const cpu = /cpu_ms\s+p50=([\d.]+)\s+p99=([\d.]+)/.exec(last);
  if (cpu) {
    const p50 = num(cpu[1]);
    const p99 = num(cpu[2]);
    if (p50 !== null && p99 !== null) return { p50_ms: p50, p99_ms: p99 };
  }
  return { p50_ms: null, p99_ms: null };
}

// Peak working set in MB from `tasklist /FO CSV /NH` text. Keeps only
// lines naming `procName`, reads every `"123,456 K"` cell, returns the
// max KB over 1024 rounded to whole MB. Null when nothing parses.
export function parseTasklistPeakMb(csvText, procName = "arena.exe") {
  const want = String(procName ?? "").toLowerCase();
  let maxKb = -1;
  for (const line of String(csvText ?? "").split(/\r?\n/)) {
    if (!line.toLowerCase().includes(want)) continue;
    for (const m of line.matchAll(/"([\d,]+)\s*K"/g)) {
      const kb = Number(m[1].replace(/,/g, ""));
      if (Number.isFinite(kb) && kb >= 0 && kb > maxKb) maxKb = kb;
    }
  }
  if (maxKb < 0) return null;
  return Math.round(maxKb / 1024);
}

// One potato line. Numbers print with 3 decimals (ms) and whole MB;
// null prints UNKNOWN so an unmeasured number never reads as zero.
export function formatPotatoLine({ adapter, p50_ms, p99_ms, peak_ws_mb }) {
  const name = adapter ?? "UNKNOWN";
  const p50 = p50_ms === null || p50_ms === undefined ? "UNKNOWN" : Number(p50_ms).toFixed(3);
  const p99 = p99_ms === null || p99_ms === undefined ? "UNKNOWN" : Number(p99_ms).toFixed(3);
  const ws = peak_ws_mb === null || peak_ws_mb === undefined ? "UNKNOWN" : String(Math.round(Number(peak_ws_mb)));
  return `potato: adapter=${name} p50_ms=${p50} p99_ms=${p99} peak_ws_mb=${ws}`;
}

// One verdict per bar: PASS when measured and inside the bar, FAIL when
// measured and over, UNKNOWN when unmeasured. Never throws.
export function checkBars(measured, bar) {
  const row = (key, actual, limit) => {
    if (actual === null || actual === undefined || !Number.isFinite(Number(actual))) {
      return { key, actual: null, bar: limit, status: "UNKNOWN" };
    }
    const a = Number(actual);
    return { key, actual: a, bar: limit, status: a <= limit ? "PASS" : "FAIL" };
  };
  return [
    row("p50_ms", measured.p50_ms, bar.p50_ms),
    row("p99_ms", measured.p99_ms, bar.p99_ms),
    row("peak_ws_mb", measured.peak_ws_mb, bar.working_set_mb),
  ];
}

// Exit for the gate: 2 when no integrated adapter, 1 when --floor runs
// and any bar is not PASS, else 0. Without --floor a FAIL still exits 0.
export function decideExit({ hasIntegrated, verdicts, floor }) {
  if (!hasIntegrated) return 2;
  if (floor && verdicts.some((v) => v.status !== "PASS")) return 1;
  return 0;
}

// Proposal bars from tools/potato-bar.json. Throws on a missing file or
// a non-numeric bar so the gate fails closed instead of passing by default.
export function loadBar(barPath = BAR_PATH) {
  const raw = JSON.parse(readFileSync(barPath, "utf8"));
  const bar = { p50_ms: Number(raw.p50_ms), p99_ms: Number(raw.p99_ms), working_set_mb: Number(raw.working_set_mb) };
  for (const [k, v] of Object.entries(bar)) {
    if (!Number.isFinite(v) || v < 0) throw new Error(`potato bar ${k} is not a number in ${barPath}`);
  }
  return bar;
}

export const ARENA_EXE = join(ROOT, "target", "release", "arena.exe");
export const REPORT_DIR = join(ROOT, "target", "reports");
export const RUN_ARGS = Object.freeze(["--scene", "arena", "--fighters", "8", "--frames", "600", "--exit"]);
export const RUN_ENV = Object.freeze({ E2040_ADAPTER: "integrated", E2040_LIMITS: "downlevel" });

// One bar verdict as one line; unmeasured reads UNKNOWN. Pure.
export function formatBarLine(v) {
  const key = v?.key ?? "UNKNOWN";
  const bar = v?.bar ?? "UNKNOWN";
  const status = v?.status ?? "UNKNOWN";
  const actual = v?.actual;
  const text =
    actual === null || actual === undefined || !Number.isFinite(Number(actual))
      ? "UNKNOWN"
      : key.endsWith("_ms")
        ? Number(actual).toFixed(3)
        : String(Math.round(Number(actual)));
  return `${key} ${text} <= ${bar} ${status}`;
}

// Markdown report for `target/reports/potato-<date>.md`. Pure over inputs.
export function formatReport({ at, cmd, env, adapter, line, verdicts, hasIntegrated, missing, timedOut }) {
  const bars = (verdicts ?? []).map(formatBarLine).join("\n");
  return [
    `# Potato check ${at ?? "UNKNOWN"}`,
    "",
    `cmd: ${cmd ?? "UNKNOWN"}`,
    `env: ${env ?? "UNKNOWN"}`,
    `adapter: ${adapter ?? "UNKNOWN"}${hasIntegrated ? "" : " (NO_IGPU)"}`,
    missing ? "arena: MISSING (no run; numbers are UNKNOWN)" : "arena: ran",
    timedOut ? "arena: TIMED OUT (numbers may be partial)" : "arena: exited",
    "",
    line ?? "potato: adapter=UNKNOWN p50_ms=UNKNOWN p99_ms=UNKNOWN peak_ws_mb=UNKNOWN",
    bars,
    "",
  ].join("\n");
}

// Pure summary over canned or live inputs; same inputs give same summary.
export function summarize({ output, tasklistText, bar, floor }) {
  const text = String(output ?? "");
  const adapter = parseAdapterName(text) ?? "UNKNOWN";
  const hasIntegrated = findIntegratedAdapter(text) !== null;
  const { p50_ms, p99_ms } = parseFrameTimes(text);
  const peak_ws_mb = parseTasklistPeakMb(String(tasklistText ?? ""));
  const measured = { p50_ms, p99_ms, peak_ws_mb };
  const line = formatPotatoLine({ adapter, p50_ms, p99_ms, peak_ws_mb });
  const verdicts = checkBars(measured, bar);
  return { adapter, hasIntegrated, measured, line, verdicts, exit: decideExit({ hasIntegrated, verdicts, floor: !!floor }) };
}

// One tasklist sample. A failed sample is loud (error names the cause) so
// a dead sampler reads as a warning downstream, never as a silent zero.
export function sampleTasklist(execFn = execFileSync) {
  try {
    return { text: execFn("tasklist", ["/FO", "CSV", "/NH"], { encoding: "utf8" }), error: null };
  } catch (err) {
    return { text: "", error: err?.message ?? String(err) };
  }
}

// Live run with working-set sampling every 250 ms; missing reads UNKNOWN.
async function runLive() {
  if (!existsSync(ARENA_EXE)) return { output: "", tasklistText: "", missing: true, timedOut: false };
  const child = spawn(ARENA_EXE, [...RUN_ARGS], {
    cwd: ROOT,
    env: { ...process.env, ...RUN_ENV },
    windowsHide: false,
  });
  let output = "";
  child.stdout?.on("data", (d) => {
    output += String(d);
  });
  child.stderr?.on("data", (d) => {
    output += String(d);
  });
  const snapshots = [];
  let tasklistFailures = 0;
  let tasklistError = "";
  const poll = () => {
    const s = sampleTasklist();
    if (s.error) {
      tasklistFailures += 1;
      tasklistError = s.error;
    } else {
      snapshots.push(s.text);
    }
  };
  poll();
  const timer = setInterval(poll, 250);
  const timedOut = await new Promise((resolve) => {
    const killer = setTimeout(() => {
      try {
        child.kill();
      } catch (err) {
        console.error(`potato-check: kill failed: ${err?.message ?? err}`);
      }
      resolve(true);
    }, 10 * 60_000);
    child.on("close", () => {
      clearTimeout(killer);
      resolve(false);
    });
    child.on("error", () => {
      clearTimeout(killer);
      resolve(false);
    });
  });
  clearInterval(timer);
  poll();
  if (tasklistFailures > 0) {
    console.error(`potato-check: tasklist failed ${tasklistFailures}x, peak may read UNKNOWN (last: ${tasklistError})`);
  }
  return { output, tasklistText: snapshots.join("\n"), missing: false, timedOut, tasklistFailures };
}

async function main() {
  const argv = process.argv.slice(2);
  if (argv.includes("--help") || argv.includes("-h")) {
    console.log("usage: node tools/potato-check.mjs [--floor]");
    return;
  }
  const floor = argv.includes("--floor");
  let bar;
  try {
    bar = loadBar();
  } catch (err) {
    console.error(`potato-check: ${err?.message ?? err}`);
    process.exitCode = 2;
    return;
  }
  const live = await runLive();
  const s = summarize({ output: live.output, tasklistText: live.tasklistText, bar, floor });
  console.log(s.line);
  for (const v of s.verdicts) console.log(formatBarLine(v));
  if (!s.hasIntegrated) console.log("NO_IGPU");
  if (live.missing) console.error(`potato-check: missing ${ARENA_EXE}`);
  if (live.timedOut) console.error("potato-check: arena run timed out");
  try {
    mkdirSync(REPORT_DIR, { recursive: true });
    const at = new Date().toISOString();
    const file = join(REPORT_DIR, `potato-${at.slice(0, 10)}.md`);
    writeFileSync(
      file,
      formatReport({
        at,
        cmd: `${ARENA_EXE} ${RUN_ARGS.join(" ")}`,
        env: "E2040_ADAPTER=integrated E2040_LIMITS=downlevel",
        adapter: s.adapter,
        line: s.line,
        verdicts: s.verdicts,
        hasIntegrated: s.hasIntegrated,
        missing: live.missing,
        timedOut: live.timedOut,
      }),
    );
    console.log(`report: ${file}`);
  } catch (err) {
    console.error(`potato-check: report failed: ${err?.message ?? err}`);
  }
  process.exitCode = s.exit;
}

if (import.meta.main) await main();
