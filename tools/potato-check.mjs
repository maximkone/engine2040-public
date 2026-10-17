#!/usr/bin/env node
// potato-check: the potato gate G9 run (row GS16). Runs arena 8x600 with
// E2040_ADAPTER=integrated E2040_LIMITS=downlevel, samples working set with
// tasklist every 250 ms, prints the potato line plus one line per bar, and
// writes target/reports/potato-<date>.md. Bars in tools/potato-bar.json.
// The same samples stamp the run's load (`potato load: QUIET` or `LOADED n
// cargo + m rustc running`), because a number read while a build runs is a
// filed number, never a claim of speed (quiet.mjs rule). The bar lines and
// their PASS/FAIL do not change with the load; the `--floor` verdict does:
// on a LOADED box it is withheld (exit 3) instead of printing a red the owner
// would read as a product verdict (26 builds made one run read p99 557 ms
// against a 14 ms steady state, 2026-10-17).
// Owner 2026-10-11. Units: ms, MB. Pure parsers give the same line x2.
// Adapter evidence, in order: the GS6 `adapter: <name> <backend> <kind>
// vendor=0x<id>` line, else arena's live `arena per-pass: adapter="<name>"`
// cell (kind then comes from the name). No evidence reads NO_IGPU, exit 2.
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

// Adapter name quoted on arena's `arena per-pass:` line, e.g.
// `adapter="Intel(R) UHD Graphics"`. Null when that line or cell is absent.
// This is the line the live arena actually prints at --exit.
export function parsePerPassAdapter(output) {
  const line = String(output ?? "").split(/\r?\n/).find((l) => l.startsWith("arena per-pass:"));
  if (!line) return null;
  const name = /adapter="([^"]*)"/.exec(line)?.[1]?.trim();
  return name ? name : null;
}

// Adapter kind word in a GS6 `describe()` line: the token before `vendor=`
// (`adapter: RTX-3050 dx12 discrete vendor=0x10de`). Null when that shape is
// absent, so a plain name line never guesses a kind from this path.
export function parseAdapterTypeWord(output) {
  const line = String(output ?? "").split(/\r?\n/).find((l) => /^adapter:\s*\S/i.test(l));
  if (!line) return null;
  const before = line.replace(/\s+vendor=.*$/i, "").trim();
  const word = before.split(/\s+/).pop()?.toLowerCase() ?? "";
  return ["discrete", "integrated", "cpu", "other"].includes(word) ? word : null;
}

// Adapter kind from its name alone, for the live `per-pass` line that carries
// no kind word. Software first, then discrete, then integrated: the gate may
// only read integrated when the name really says so.
export function classifyAdapterKind(name) {
  const key = String(name ?? "").toLowerCase().replace(/[^a-z0-9]+/g, " ").trim();
  if (!key) return "unknown";
  if (/basic render driver|llvmpipe|\bwarp\b|software/.test(key)) return "cpu";
  if (/geforce|\brtx\b|\bgtx\b|quadro|tesla|\brx \d|\barc\b|radeon pro/.test(key)) return "discrete";
  if (/\buhd\b|\biris\b|hd graphics|vega|radeon tm graphics|radeon graphics/.test(key)) return "integrated";
  return "other";
}

// The one adapter name the potato line reports: the GS6 `describe()` line
// first (it carries the kind), else arena's live `per-pass` name. Spaces to
// underscores so the potato line stays space-separated. Null when neither
// line exists.
export function arenaAdapterName(output) {
  const text = String(output ?? "");
  const described = parseAdapterName(text);
  if (described !== null) return described;
  const live = parsePerPassAdapter(text);
  return live === null ? null : live.replace(/\s+/g, "_");
}

// Integrated adapter name, or null when the run shows no integrated adapter.
// Fails closed: no adapter evidence at all reads null, and a discrete or
// software adapter never reads integrated. An explicit kind word wins; with
// only a live name the name is classified.
export function findIntegratedAdapter(output) {
  const text = String(output ?? "");
  const name = arenaAdapterName(text);
  if (name === null) return null;
  const kind = parseAdapterTypeWord(text) ?? classifyAdapterKind(name);
  return kind === "integrated" ? name : null;
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

// Cargo and rustc processes in one `tasklist` sample. A gate number read
// while a build runs is a filed number, never a claim of speed (the
// quiet.mjs rule), so the run stamps its own load. Pure over the text;
// an empty or missing sample reads QUIET because nothing is running.
export function parseBuildLoad(csvText) {
  let cargo = 0;
  let rustc = 0;
  for (const line of String(csvText ?? "").split(/\r?\n/)) {
    if (/^"cargo\.exe"/i.test(line)) cargo += 1;
    else if (/^"rustc\.exe"/i.test(line)) rustc += 1;
  }
  return { cargo, rustc, loaded: cargo + rustc > 0 };
}

// Load stamp text: QUIET when no build runs, else LOADED with the counts
// and the wording that says the numbers are filed. Pure.
export function formatLoadText(load) {
  const cargo = Number(load?.cargo ?? 0);
  const rustc = Number(load?.rustc ?? 0);
  if (!(cargo + rustc > 0)) return "QUIET";
  return `LOADED ${cargo} cargo + ${rustc} rustc running (filed numbers, not a speed claim)`;
}

// One line for the run's load stamp, beside the potato line. Pure.
export function formatLoadLine(load) {
  return `potato load: ${formatLoadText(load)}`;
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

// Exit for the gate: 2 when no integrated adapter, 3 when `--floor` runs on a
// box a build stamped LOADED (the numbers are filed, so the gate withholds the
// verdict instead of printing a false FAIL - a run at 26 cargo/rustc read p99
// 557 ms against the 14 ms steady state on 2026-10-17), 1 when `--floor` runs
// and any bar is not PASS, else 0. Without `--floor` a FAIL still exits 0, and
// every bar line still prints its PASS/FAIL whatever the load: nothing is
// hidden, only the verdict is withheld.
export function decideExit({ hasIntegrated, verdicts, floor, loaded }) {
  if (!hasIntegrated) return 2;
  if (floor && loaded) return 3;
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
// The load line says whether the numbers came from a quiet box or a loaded
// one, so a filed number never reads as a quiet one; `withheld` records that
// `--floor` refused a verdict on that loaded run (exit 3).
export function formatReport({ at, cmd, env, adapter, line, verdicts, hasIntegrated, missing, timedOut, load, withheld }) {
  const bars = (verdicts ?? []).map(formatBarLine).join("\n");
  return [
    `# Potato check ${at ?? "UNKNOWN"}`,
    "",
    `cmd: ${cmd ?? "UNKNOWN"}`,
    `env: ${env ?? "UNKNOWN"}`,
    `load: ${load ? formatLoadText(load) : "UNKNOWN"}`,
    `floor: ${withheld ? "WITHHELD (box was loaded; numbers are filed, no verdict)" : "judged"}`,
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
// `loaded` is the run's own load stamp (see `parseBuildLoad`): a verdict on a
// box that was building is withheld by `decideExit`.
export function summarize({ output, tasklistText, bar, floor, loaded }) {
  const text = String(output ?? "");
  const adapter = arenaAdapterName(text) ?? "UNKNOWN";
  const hasIntegrated = findIntegratedAdapter(text) !== null;
  const { p50_ms, p99_ms } = parseFrameTimes(text);
  const peak_ws_mb = parseTasklistPeakMb(String(tasklistText ?? ""));
  const measured = { p50_ms, p99_ms, peak_ws_mb };
  const line = formatPotatoLine({ adapter, p50_ms, p99_ms, peak_ws_mb });
  const verdicts = checkBars(measured, bar);
  const wasLoaded = loaded === true;
  return {
    adapter,
    hasIntegrated,
    measured,
    line,
    verdicts,
    loaded: wasLoaded,
    exit: decideExit({ hasIntegrated, verdicts, floor: !!floor, loaded: wasLoaded }),
  };
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
  if (!existsSync(ARENA_EXE)) return { output: "", tasklistText: "", missing: true, timedOut: false, load: { cargo: 0, rustc: 0, loaded: false } };
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
  // Worst load seen over the run: one build during the sample makes the
  // frame numbers filed, not quiet.
  let load = { cargo: 0, rustc: 0, loaded: false };
  for (const text of snapshots) {
    const sample = parseBuildLoad(text);
    if (sample.cargo + sample.rustc > load.cargo + load.rustc) load = sample;
  }
  return { output, tasklistText: snapshots.join("\n"), missing: false, timedOut, tasklistFailures, load };
}

// Tool entry point with injectable seams: a test drives the real print,
// exit and report path with canned arena text, a fake tasklist and a temp
// report dir, so the Accept's "prints NO_IGPU and exits 2" / "--floor exits
// 1" are proven on the tool, not only on the pure helpers. Returns the exit
// code: 2 when no integrated adapter shows or the bar file is unreadable,
// 3 when `--floor` runs on a box a build stamped LOADED (no verdict on filed
// numbers), 1 when `--floor` runs and a bar is not PASS, else 0. Defaults are
// the live implementations, so production behaviour is unchanged.
// Caller: `main` (same file) passes `process.argv` and the live defaults.
export async function run(argv = [], deps = {}) {
  const log = deps.log ?? ((line) => console.log(line));
  const logError = deps.errorLog ?? ((line) => console.error(line));
  const runLiveFn = deps.runLive ?? runLive;
  const loadBarFn = deps.loadBar ?? (() => loadBar());
  const reportDir = deps.reportDir ?? REPORT_DIR;
  const now = deps.now ?? (() => new Date());
  if (argv.includes("--help") || argv.includes("-h")) {
    log("usage: node tools/potato-check.mjs [--floor]");
    return 0;
  }
  const floor = argv.includes("--floor");
  let bar;
  try {
    bar = loadBarFn();
  } catch (err) {
    logError(`potato-check: ${err?.message ?? err}`);
    return 2;
  }
  const live = await runLiveFn();
  const s = summarize({
    output: live.output,
    tasklistText: live.tasklistText,
    bar,
    floor,
    loaded: live.load?.loaded === true,
  });
  log(s.line);
  for (const v of s.verdicts) log(formatBarLine(v));
  log(formatLoadLine(live.load));
  if (s.exit === 3) log("potato floor: WITHHELD - a build ran during this run, so the bars are filed numbers, not a verdict (rerun on a quiet box)");
  if (!s.hasIntegrated) log("NO_IGPU");
  if (live.missing) logError(`potato-check: missing ${ARENA_EXE}`);
  if (live.timedOut) logError("potato-check: arena run timed out");
  try {
    mkdirSync(reportDir, { recursive: true });
    const at = now().toISOString();
    const file = join(reportDir, `potato-${at.slice(0, 10)}.md`);
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
        load: live.load,
        withheld: s.exit === 3,
      }),
    );
    log(`report: ${file}`);
  } catch (err) {
    logError(`potato-check: report failed: ${err?.message ?? err}`);
  }
  return s.exit;
}

async function main() {
  process.exitCode = await run(process.argv.slice(2));
}

if (import.meta.main) await main();
