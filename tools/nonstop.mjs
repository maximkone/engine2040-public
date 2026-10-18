// nonstop: drive a weak OpenCode model through the ready board rows, one fresh session per row.
//
// Why: a model session ends the moment the model writes a final message. This loop does not:
// it picks the next ready row, runs one `opencode run` on it, checks the tree, and goes on
// until nothing is left. The model never decides whether to continue.
//
// Usage from the repo root:
//   node tools/nonstop.mjs [--max N] [--match <regex>] [--ids A,B] [--model P/M] [--minutes M]
//                          [--retry] [--dry] [--help]
//
// Files: target/nonstop/ledger.jsonl (one line per attempt), target/nonstop/<ID>.log (model output).
// Stops when: no candidate row is left, --max is reached, `node tools/workspace-check.mjs` fails
// twice in a row, the model call fails three times in a row, or the file target/nonstop/STOP exists.
// A row already in the ledger is not tried again unless --retry.
//
// Exit codes: 0 finished or stopped by a guard - 2 usage.
import { spawnSync } from "node:child_process";
import { appendFileSync, existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
const ROOT = dirname(HERE);
const OUT = join(ROOT, "target", "nonstop");
const LEDGER = join(OUT, "ledger.jsonl");
const STOP = join(OUT, "STOP");
const DEFAULT_MODEL = "zen-proxy-deepseek/deepseek-v4.1-flash-free";
const DEFAULT_MINUTES = 40;
const ID_RE = /^[A-Z][A-Z0-9-]*\d[a-z]?\d*$/;

const HELP = `nonstop.mjs: run ready board rows through opencode, one fresh session per row

Usage:
  node tools/nonstop.mjs [--max N] [--match <regex>] [--ids A,B] [--model P/M] [--minutes M] [--retry] [--dry]

  --max N        stop after N rows (default: no limit)
  --match RE     only rows whose id matches the regex (example: '^(NV|WEB)')
  --ids A,B      only these ids, in this order
  --model P/M    opencode model (default ${DEFAULT_MODEL})
  --minutes M    kill one row's session after M minutes (default ${DEFAULT_MINUTES})
  --retry        try rows again that the ledger already holds
  --dry          print the next prompt and exit; runs nothing, writes nothing

Stop early: create the file target/nonstop/STOP.
`;

/**
 * The opencode binary to spawn.
 *
 * OpenCode itself sets `OPENCODE=1` for every child process, so that value is
 * not a path. Only `OPENCODE_BIN`, or an `OPENCODE` that looks like a path, names
 * the binary; anything else falls back to `opencode` on PATH.
 */
export function opencodeBin(env = process.env) {
  if (env.OPENCODE_BIN) return env.OPENCODE_BIN;
  const raw = env.OPENCODE;
  if (typeof raw === "string" && (raw.includes("/") || raw.includes("\\") || /\.exe$/i.test(raw))) {
    return raw;
  }
  return "opencode";
}

/** Ids of the open ready rows, in board-slice order, from the text of \`board-slice --ready\`. */
export function parseReadyList(text) {
  const ids = [];
  for (const line of text.split(/\r?\n/)) {
    const m = /^(\S+)\s+open\s/.exec(line);
    if (m && ID_RE.test(m[1]) && !ids.includes(m[1])) ids.push(m[1]);
  }
  return ids;
}

/** The first candidate id: ready, not in `done`, passing `--ids` / `--match`. */
export function pickNext(ready, done, opts = {}) {
  const re = opts.match ? new RegExp(opts.match) : null;
  const pool = opts.ids && opts.ids.length ? opts.ids.filter((id) => ready.includes(id)) : ready;
  return pool.find((id) => !done.has(id) && (!re || re.test(id))) ?? null;
}

/** The one prompt a session gets: the row and the fixed rules, nothing else to decide. */
export function buildPrompt(id, slice, today) {
  return [
    `You are finishing ONE board row of engine2040, then you stop. Row: ${id}. Today is ${today}.`,
    "Read first: AGENTS.md (\"Work from plans\"), docs/plan/2026-10-18-handoff.md (section \"Hard rules\"),",
    "and docs/HASH-LAW.md if the row touches an integer simulation crate (arena combat core ecs net input physics).",
    "",
    "The row and its context, exactly as on the board:",
    slice,
    "",
    "Do this, in order:",
    "1. Do the row's work. The smallest change that meets its Accept line. Reuse existing code. Do not invent files the row does not name.",
    "2. Prove it with the test the row names. Heavy cargo runs only through `node tools/gh-check.mjs cargo ...`. Frozen hashes must not move.",
    "3. `cargo fmt` on your files, then `node tools/rust-quality.mjs --owned`, then `node tools/workspace-check.mjs`. No FAIL from your files.",
    "4. Commit only your own paths: `git commit -F <msgfile> -- <paths>` (run `git add -N <file>` for new files first; `mod x;` and x.rs go in one commit).",
    "   Never `git add -A`, rebase, amend or force-push. Then `git pull --no-rebase --no-edit origin master` and `git push origin master`.",
    `5. Tick the row only if a named test ran green: change \`- [ ] ${id}\` to \`- [x] ${id}\` and append \` Landed <sha7>. <test names and gh run id>.\``,
    `   to the END of the row's last line. If only part is done, append \` Progress ${today}: <what exists, sha7, what is left>.\` and leave it open.`,
    "   Commit the shard file by path and push.",
    "",
    "If the row needs a human, a reference GPU, a download, a paid service or an owner decision: do NOT guess.",
    `Append \` Progress ${today}: operator/owner: <why>.\` to the row, commit, push, and stop.`,
    "Do not start any other row. Do not write plans or summaries. Never ask a question: nobody answers.",
    `Your last message is exactly one line: \`DONE ${id} <sha7>\` or \`PARTIAL ${id} <what is left>\` or \`SKIP ${id} <reason>\`.`,
  ].join("\n");
}

function parseArgs(argv) {
  const o = { max: Infinity, minutes: DEFAULT_MINUTES, model: DEFAULT_MODEL, match: "", ids: [], retry: false, dry: false, help: false };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (a === "--help" || a === "-h") o.help = true;
    else if (a === "--retry") o.retry = true;
    else if (a === "--dry") o.dry = true;
    else if (a === "--max") o.max = Number(argv[++i]);
    else if (a === "--minutes") o.minutes = Number(argv[++i]);
    else if (a === "--model") o.model = argv[++i];
    else if (a === "--match") o.match = argv[++i];
    else if (a === "--ids") o.ids = (argv[++i] ?? "").split(",").filter(Boolean);
    else return { error: `unknown argument ${a}` };
  }
  if (!(o.max > 0) || !(o.minutes > 0) || !o.model) return { error: "bad --max, --minutes or --model" };
  return o;
}

function node(args, opts = {}) {
  return spawnSync(process.execPath, args, { cwd: ROOT, encoding: "utf8", maxBuffer: 64 << 20, ...opts });
}

function readyIds() {
  const r = node([join(HERE, "board-slice.mjs"), "--ready"]);
  return parseReadyList(r.stdout ?? "");
}

function rowSlice(id) {
  return (node([join(HERE, "board-slice.mjs"), id]).stdout ?? "").trim();
}

function rowLine(slice) {
  const at = slice.indexOf("## Row");
  return at < 0 ? "" : (slice.slice(at).split(/\r?\n/)[2] ?? "");
}

function loadLedger() {
  if (!existsSync(LEDGER)) return [];
  return readFileSync(LEDGER, "utf8").split(/\r?\n/).filter(Boolean).map((l) => JSON.parse(l));
}

function today() {
  return new Date().toLocaleDateString("sv");
}

function main() {
  const o = parseArgs(process.argv.slice(2));
  if (o.error) {
    process.stderr.write(`${o.error}\n${HELP}`);
    return 2;
  }
  if (o.help) {
    process.stdout.write(HELP);
    return 0;
  }
  mkdirSync(OUT, { recursive: true });
  const attempts = loadLedger();
  const done = new Set(o.retry ? [] : attempts.filter((a) => a.outcome !== "model-error").map((a) => a.id));
  let tried = 0;
  let reds = 0;
  let modelErrors = 0;
  while (tried < o.max) {
    if (existsSync(STOP)) { console.log("nonstop: STOP file found, stopping"); break; }
    if (!o.dry) spawnSync("git", ["pull", "--no-rebase", "--no-edit", "origin", "master"], { cwd: ROOT, encoding: "utf8" });
    const id = pickNext(readyIds(), done, o);
    if (!id) { console.log("nonstop: no candidate row left"); break; }
    const before = rowSlice(id);
    const prompt = buildPrompt(id, before, today());
    if (o.dry) { process.stdout.write(`${prompt}\n`); return 0; }
    console.log(`nonstop: [${tried + 1}] ${id}`);
    const t0 = Date.now();
    const run = spawnSync(opencodeBin(), ["run", "--auto", "-m", o.model, "--title", `nonstop ${id}`, prompt], {
      cwd: ROOT, encoding: "utf8", maxBuffer: 256 << 20, timeout: o.minutes * 60_000,
    });
    const log = `${run.stdout ?? ""}\n${run.stderr ?? ""}\n${run.error ? `spawn ${opencodeBin()}: ${run.error.message}` : ""}`;
    writeFileSync(join(OUT, `${id}.log`), log);
    const after = rowSlice(id);
    const ticked = rowLine(after).startsWith("- [x]");
    let outcome = ticked ? "ticked" : after !== before ? "progress" : "unchanged";
    if (run.error || run.status !== 0) outcome = "model-error";
    modelErrors = outcome === "model-error" ? modelErrors + 1 : 0;
    const check = node([join(HERE, "workspace-check.mjs")]);
    const green = check.status === 0;
    reds = green ? 0 : reds + 1;
    const last = log.trim().split(/\r?\n/).filter(Boolean).pop() ?? "";
    const entry = { id, outcome, treeGreen: green, minutes: Math.round((Date.now() - t0) / 6000) / 10, last: last.slice(0, 200), at: new Date().toISOString() };
    appendFileSync(LEDGER, `${JSON.stringify(entry)}\n`);
    done.add(id);
    tried += 1;
    console.log(`nonstop:     ${outcome}, tree ${green ? "green" : "RED"}, ${entry.minutes} min, ${entry.last}`);
    if (reds >= 2) { console.log("nonstop: tree red twice in a row, stopping. Fix it, then run again."); break; }
    if (modelErrors >= 3) { console.log("nonstop: model call failed three times in a row, stopping. Check the proxy on :13579."); break; }
  }
  const all = loadLedger();
  const count = (k) => all.filter((a) => a.outcome === k).length;
  console.log(`nonstop: ledger ${all.length} rows: ${count("ticked")} ticked, ${count("progress")} progress, ${count("unchanged")} unchanged, ${count("model-error")} model errors`);
  return 0;
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) process.exit(main());
