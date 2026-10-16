// Autonomous sprint waves for the OpenCode setup. Each wave is one fresh
// orchestrator session (`opencode run`; v2 no longer runs slash commands
// headless, so the message points at .opencode/commands/sprint.md). The board
// decides what happens next. The loop holds the sprint lock for the whole run
// and tries one replan wave when nothing is ready. It stops if replan adds no
// ready work, two waves make no board progress, two sessions fail in a row,
// target/sprint.halt exists, or --waves is reached.
//   node tools/sprint-loop.mjs [--waves N] [--width K] [--agent NAME]
//     [--model PROVIDER/MODEL] [--paid] [--mix] [--safe] [--dry-run]
//     [--prune-queue] [--keep N]
// --prune-queue moves target/queue/done overflow to target/queue/archive-YYYY-MM-DD,
// keeping the newest --keep files (default 50) so ready/claim scans stay fast.
// --mix alternates the paid lane every even wave (10+10 across time; one model
// per wave session, no per-packet mix exists in OpenCode).
// OPENCODE_BIN overrides the opencode executable.
import { spawnSync } from "node:child_process";
import { appendFileSync, existsSync, mkdirSync, readFileSync, readdirSync, renameSync, statSync } from "node:fs";
import { join } from "node:path";
import { archivedDoneRows, loadBoardText, parseBoard, rowReady } from "./board-slice.mjs";

const args = process.argv.slice(2);
const option = (key, fallback) => {
  const index = args.indexOf(key);
  return index === -1 ? fallback : args[index + 1];
};
const waves = Number(option("--waves", "50"));
const width = Number(option("--width", "20"));
const agent = option("--agent", "orchestrator");
const paid = args.includes("--paid");
const mix = args.includes("--mix");
const baseModel = option("--model", "");
const safe = args.includes("--safe");
const dryRun = args.includes("--dry-run");
const pruneOnly = args.includes("--prune-queue");
const keep = Number(option("--keep", "50"));
if (args.includes("--help") || args.includes("-h")) {
  console.log("Usage: node tools/sprint-loop.mjs");
  process.exit(0);
}
if (!Number.isInteger(waves) || waves < 1 || !Number.isInteger(width) || width < 1 || width > 30) {
  console.error("Usage: node tools/sprint-loop.mjs [--waves N] [--width 1..30] [--agent NAME] [--model PROVIDER/MODEL] [--paid] [--mix] [--safe] [--dry-run] [--prune-queue] [--keep N]");
  process.exit(2);
}
if (!Number.isInteger(keep) || keep < 1) {
  console.error("Usage: node tools/sprint-loop.mjs [--prune-queue] [--keep N]");
  process.exit(2);
}
const opencode = process.env.OPENCODE_BIN ?? "opencode";
const windows = process.platform === "win32";

// Same row grammar and readiness as SPRINT.md states, from the one shared
// parser (board-slice.mjs): no second copy of the row split lives here.
// Split-safe: reads docs/SPRINT.md plus docs/SPRINT-*.md shards when they
// exist; single-file trees return the index bytes unchanged. Done rows moved
// to docs/archive/SPRINT-done-*.md count as ticked (needs: and the count only).
function board() {
  const text = loadBoardText(undefined, { receipts: false });
  const { tasks, order } = parseBoard(text, archivedDoneRows());
  const ready = order.map((id) => tasks.get(id)).filter((t) => rowReady(tasks, t)).map((t) => t.id);
  const done = [...tasks.values()].filter((t) => t.done).length;
  return { done, total: tasks.size, ready };
}

// Queue hygiene: target/queue/done grows one file per packet, so ready and
// claim scans slow down. Keep the newest `keep` done files, move the rest to
// target/queue/archive-YYYY-MM-DD. Oldest by mtime (then name) moves first.
function pruneDoneQueue(keepN, reportOnly) {
  const doneDir = join("target", "queue", "done");
  let names;
  try {
    names = readdirSync(doneDir).filter((f) => {
      try { return statSync(join(doneDir, f)).isFile(); } catch { return false; }
    });
  } catch { return { before: 0, after: 0, archived: 0, dir: "" }; }
  const before = names.length;
  if (before <= keepN) return { before, after: before, archived: 0, dir: "" };
  const ranked = names.map((f) => {
    let mtime = 0;
    try { mtime = statSync(join(doneDir, f)).mtimeMs; } catch { mtime = 0; }
    return { f, mtime };
  }).sort((a, b) => b.mtime - a.mtime || (a.f < b.f ? -1 : 1));
  const overflow = ranked.slice(keepN);
  const day = new Date().toISOString().slice(0, 10);
  const outDir = join("target", "queue", `archive-${day}`);
  let archived = 0, failed = 0;
  if (!reportOnly) { try { mkdirSync(outDir, { recursive: true }); } catch { failed += 1; } }
  for (const { f } of overflow) {
    if (reportOnly) { archived += 1; continue; }
    try { renameSync(join(doneDir, f), join(outDir, f)); archived += 1; }
    catch { failed += 1; }
  }
  return { before, after: before - archived, archived, failed, dir: outDir };
}

const node = (script, ...rest) => spawnSync(process.execPath, [script, ...rest], { encoding: "utf8" });
// Timing shape donors: sharkdp/hyperfine (Apache-2.0,
// https://github.com/sharkdp/hyperfine) per-leg mean/min/max table +
// slowest-first summary; casey/just (CC0-1.0,
// https://github.com/casey/just) --timestamp per-recipe prefix. Shape only,
// no donor code copied. Licences verified in LICENSE-APACHE / LICENSE-MIT.
const loopStart = Date.now();
const pad2 = (n) => String(n).padStart(2, "0");
const stamp = (t = Date.now()) => {
  const s = Math.floor((t - loopStart) / 1000);
  return `${pad2(Math.floor(s / 60))}:${pad2(s % 60)}`;
};
const legs = [];
const timeLeg = (name, fn) => {
  const t0 = Date.now();
  const out = fn();
  const ms = Date.now() - t0;
  legs.push({ name, ms });
  console.log(`[${stamp()} ${name}] ${ms}ms`);
  return out;
};
const timingSummary = () => {
  const ranked = [...legs].sort((a, b) => b.ms - a.ms);
  const line = ranked.map((l) => `${l.name}=${l.ms}ms`).join(", ") || "none";
  console.log(`[${stamp()} timing] slowest-first: ${line}`);
};
const log = (line) => {
  mkdirSync("target", { recursive: true });
  appendFileSync("target/sprint-loop.log", `${new Date().toISOString()} ${line}\n`);
  console.log(line);
};

if (pruneOnly) {
  const r = pruneDoneQueue(keep, dryRun);
  console.log(`QUEUE-PRUNE done=${r.before}->${dryRun ? r.before : r.after} archived=${r.archived} keep=${keep} failed=${r.failed ?? 0}${r.dir ? ` to=${r.dir}` : ""}`);
  if (!dryRun && (r.failed ?? 0) > 0) process.exit(1);
  process.exit(0);
}

if (dryRun) {
  const state = timeLeg("board", () => board());
  console.log(`DRY RUN: ${state.done}/${state.total} ticked, ${state.ready.length} ready: ${state.ready.join(" ")}`);
  timingSummary();
  console.log(`PASS dry-run ${state.done}/${state.total} ticked`);
  process.exit(0);
}

const lock = node("tools/sprint-lock.mjs", "acquire", "sprint-loop");
if (lock.status !== 0) {
  console.error(lock.stderr || lock.stdout);
  process.exit(1);
}
const release = () => node("tools/sprint-lock.mjs", "release");
process.on("SIGINT", () => {
  release();
  log("STOP interrupted");
  process.exit(130);
});

let stalls = 0;
let failures = 0;
for (let wave = 1; wave <= waves; wave += 1) {
  if (existsSync("target/sprint.halt")) {
    log("STOP operator halt (delete target/sprint.halt to resume)");
    break;
  }
  const before = board();
  const replan = before.ready.length === 0;
  node("tools/sprint-lock.mjs", "refresh", "sprint-loop");
  const pruned = pruneDoneQueue(keep, false);
  if (pruned.archived > 0) log(`QUEUE-PRUNE done=${pruned.before}->${pruned.after} archived=${pruned.archived} keep=${keep} to=${pruned.dir}`);
  const clean = node("tools/clean.mjs", "--auto");
  log(`CLEAN ${clean.out.split("\n").pop() ?? "clean done"}`);
  // Lead brief: farm state + dirty-tree order + claims, so the wave leads
  // itself without a human message. Cheap (no cargo, ~10ms); never fails a wave.
  let brief = "";
  try {
    const b = node("tools/lead-brief.mjs");
    brief = (b.stdout ?? "").trim().slice(0, 1500);
  } catch { brief = ""; }
  log(`WAVE ${wave}${replan ? " REPLAN" : ""}: ${before.ready.length} ready, ${before.done}/${before.total} ticked`);
  const leadCtx = brief ? `\nLive context (read first, no human will repeat it):\n${brief}` : "";
  const message = replan
    ? `Run exactly one replan wave now. Read .opencode/commands/sprint.md and follow it with the arguments: wave ${width}${safe ? " safe" : ""}. No board row is ready; check plans and coverage, add or unblock real rows, then write target/sprint-handoff.md. The sprint lock is held by sprint-loop for you.${leadCtx}`
    : `Run exactly one sprint wave now. Read .opencode/commands/sprint.md and follow it with the arguments: wave ${width}${safe ? " safe" : ""}. The sprint lock is held by sprint-loop for you. Work autonomously, never ask questions, and write target/sprint-handoff.md before you stop.${leadCtx}`;
  const wavePaid = paid || (mix && wave % 2 === 0);
  const model = baseModel || (wavePaid ? "opencode-go/muse-spark-1.3-contributor" : "");
  log(`LANE ${model ? "paid" : "free"}`);
  const runArgs = ["run", "--agent", agent];
  if (model) runArgs.push("--model", model);
  runArgs.push(windows ? `"${message}"` : message);
  // cmd.exe needs the message quoted as one argument; it holds no cmd metacharacters.
  const result = spawnSync(opencode, runArgs, { stdio: "inherit", shell: windows });
  if (result.error || result.status !== 0) {
    failures += 1;
    log(`wave ${wave} session failed: ${result.error?.message ?? `exit ${result.status}`}`);
    if (failures >= 2) {
      log("STOP two failed sessions in a row");
      break;
    }
    continue;
  }
  failures = 0;
  const after = board();
  const ticked = after.done - before.done;
  log(`wave ${wave} ticked ${ticked}, ${after.done}/${after.total} done, ${after.ready.length} ready`);
  if (replan && after.ready.length === 0) {
    log("STOP replan found no ready work; read target/sprint-handoff.md and docs/BLOCKERS.md");
    break;
  }
  stalls = ticked > 0 || after.total > before.total || after.ready.length > before.ready.length ? 0 : stalls + 1;
  if (stalls >= 2) {
    log("STOP two waves without board progress; read target/sprint-handoff.md and docs/BLOCKERS.md");
    break;
  }
}
release();
