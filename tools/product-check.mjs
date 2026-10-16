#!/usr/bin/env node
// product-check: the loop's own eyes on the built game. Builds arena.exe when
// it is older than the code (one cargo slot), runs it twice (a look run with
// captures, a timing run), reads its receipt lines, scores them against
// tools/product-bar.json and, for every floor a run breaks, files one bug
// packet in target/queue/ready/ (bug: rows run right after loop: fixes and go
// in past the queue cap). The keeper starts it on its beat with --if-new; it
// returns at once while the crates tree is the one it last checked.
// Owner 2026-10-11, after the fighters strobed red at every start and no seat
// said so: "why i got to babysit this".
//   node tools/product-check.mjs [--if-new] [--no-build] [--no-file] [--json]
// Writes target/reports/product/latest.json, latest.md and history.jsonl;
// captures go to target/reports/product/runs/<sha>-<utc>/ (newest 6 kept).
// Exit 0 green or skipped, 1 a floor broke, 2 the check itself could not run.
import { createHash } from "node:crypto";
import { appendFileSync, existsSync, mkdirSync, readdirSync, readFileSync, rmSync, statSync, writeFileSync } from "node:fs";
import { execFileSync, spawnSync } from "node:child_process";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { acquireSlot, pidAlive, readKnobs, knob, releaseSlot } from "../.opencode/seat-guard.mjs";
import { binStatus, BINS } from "./bins.mjs";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const OUT = join(ROOT, "target", "reports", "product");
const READY = join(ROOT, "target", "queue", "ready");
const RUNNING = join(ROOT, "target", "queue", "running");
const ARENA = join(ROOT, "target", "release", "arena.exe");
const RUN_MS = 6 * 60_000;
const SLOT_WAIT_MS = 20 * 60_000;
const BUILD_MS = 45 * 60_000;
const KEEP_RUNS = 6;
const REFILE_MS = 6 * 3_600_000;

const num = (s) => (s === undefined || s === "-" ? null : Number(s));
const field = (line, key) => num(line?.match(new RegExp(`\\b${key}=([-\\d.]+)`))?.[1]);
const lineOf = (text, head) => String(text).split(/\r?\n/).find((l) => l.startsWith(head)) ?? null;

// The `arena view:` line: every drawn body as numbers, no image (crates/game/src/view_truth.rs).
export const parseView = (line) => {
  if (!line) return null;
  const span = line.match(/\bsim_ticks=([-\d.]+)\.\.([-\d.]+)/);
  const out = { sim_first: span ? Number(span[1]) : null, sim_last: span ? Number(span[2]) : null, pop_acts: line.match(/\bpop_acts=(\S+)/)?.[1] ?? null };
  for (const k of ["frames", "bodies", "offscreen_share", "nonfinite", "error_parts", "sunk_cm_max", "idle_float_cm_max", "pops", "pop_cm_max", "eye_in_body", "eye_below_ground", "closest_cm", "cover_max"]) out[k] = field(line, k);
  return out;
};

// The receipt lines arena.exe prints at --exit (d6293754 framing, 2026-10-11 tinted_share and view).
export const parseReceipt = (text) => {
  const split = lineOf(text, "arena frame-split:");
  const framing = lineOf(text, "arena framing:");
  const pass = lineOf(text, "arena per-pass:");
  const interval = split?.match(/interval_ms p50=([\d.]+) p99=([\d.]+)/);
  const passes = {};
  for (const m of String(pass ?? "").matchAll(/\b(sky|layers|particles|decals|ibl|fog|gtao|shadow\w*|taa|bloom|ao)=([\d.]+)ms/g)) passes[m[1]] = Number(m[2]);
  return {
    split: !!split,
    interval_p50_ms: interval ? Number(interval[1]) : null,
    interval_p99_ms: interval ? Number(interval[2]) : null,
    gpu_p50_ms: num(split?.match(/gpu_ms p50=([\d.]+)/)?.[1]),
    cpu_p50_ms: num(split?.match(/cpu_ms p50=([\d.]+)/)?.[1]),
    framing: !!framing,
    frames: field(framing, "frames"),
    empty: field(framing, "empty"),
    fewest_in_view: field(framing, "fewest_in_view"),
    steepest_down_deg: field(framing, "steepest_down_deg"),
    tinted_share: field(framing, "tinted_share"),
    validation_errors: field(pass, "validation_errors"),
    view: parseView(lineOf(text, "arena view:")),
    passes,
    panics: (String(text).match(/panicked at/g) ?? []).length,
  };
};

// Captured frames: a near-empty PNG is a blank frame; two byte-equal frames
// in a row are a frozen picture (8 fighting bodies never hold still 60 frames).
export const scoreCaptures = (files) => {
  let blank = 0, frozen = 0, prev = null;
  for (const f of files) {
    const bytes = readFileSync(f);
    if (bytes.length < 60_000) blank += 1;
    const sum = createHash("sha256").update(bytes).digest("hex");
    if (sum === prev) frozen += 1;
    prev = sum;
  }
  return { captures: files.length, blank, frozen };
};

// Reference fps: the median p50 interval of the last unloaded timing runs.
export const referenceInterval = (history) => {
  const xs = history.filter((h) => h.timing && !h.timing.loaded && h.timing.interval_p50_ms > 0).slice(-5).map((h) => h.timing.interval_p50_ms).sort((a, b) => a - b);
  return xs.length ? xs[Math.floor(xs.length / 2)] : null;
};

// Pass floors (GS3): `tools/pass-bar.json` lists each look pass as
// `required` or `pending`. A `required` pass with no ms or 0 ms over the run
// is red; `pending` only prints (never red).
export const passMs = (passes, name) => {
  if (!passes) return null;
  if (name === "shadow") {
    for (const [k, v] of Object.entries(passes)) {
      if (k === "shadow" || k.startsWith("shadow")) return v;
    }
    return null;
  }
  return passes[name] ?? null;
};

export const passFloors = (passes, passBar) => {
  const red = {};
  for (const [name, state] of Object.entries(passBar?.passes ?? {})) {
    if (state !== "required") continue;
    const ms = passMs(passes, name);
    if (ms === null || !(ms > 0)) red[`pass-${name}`] = `the ${name} pass reported ${ms === null ? "no ms" : "0 ms"} over the run (tools/pass-bar.json marks it required)`;
  }
  return red;
};

// Floors broken by one check: { metric: why } (empty = green). `bar.floors` from product-bar.json.
export const redFloors = ({ look, timing, caps, ref }, bar, passBar = null) => {
  const f = bar.floors ?? {};
  const red = {};
  if (look.timedOut || timing?.timedOut) red.hang = `arena.exe did not exit within ${RUN_MS / 60_000} min of a --frames 300 --exit run`;
  if (look.panics || timing?.panics) red.panic = `arena.exe panicked (${look.panics + (timing?.panics ?? 0)} panic lines)`;
  else if (look.exit !== 0 && look.exit !== null && !look.framingFailed) red.exit = `the look run exited ${look.exit}`;
  if (!look.timedOut && !look.framing) red.receipt = "the look run printed no `arena framing:` line (the fighter look did not draw)";
  if (look.empty > (f.framing_empty ?? 0)) red.framing = `${look.empty} of ${look.frames} frames showed no fighter`;
  if (look.tinted_share !== null && look.tinted_share > (f.tinted_share_max ?? 0.25)) red.strobe = `${Math.round(look.tinted_share * 100)}% of drawn bodies wore the red hit tint (floor ${Math.round((f.tinted_share_max ?? 0.25) * 100)}%)`;
  const v = look.view;
  if (!look.timedOut && look.framing && !v) red.view_receipt = "the look run printed no `arena view:` line (the view truth did not fold)";
  if (v) {
    if (v.nonfinite > (f.view_nonfinite ?? 0)) red.nan = `${v.nonfinite} posed vertices or joints were NaN or infinite`;
    if (v.error_parts > (f.view_error_parts ?? 0)) red.error_material = `${v.error_parts} body-part draws used the skin error material`;
    if (v.eye_in_body > (f.view_eye_in_body ?? 0)) red.camera_in_body = `the camera sat inside a fighter on ${v.eye_in_body} of ${v.frames} frames`;
    if (v.eye_below_ground > (f.view_eye_below_ground ?? 0)) red.camera_under_ground = `the camera was under the sand on ${v.eye_below_ground} of ${v.frames} frames`;
    if (v.sunk_cm_max > (f.view_sunk_cm_max ?? 20)) red.sunk = `a body sank ${v.sunk_cm_max} cm into the sand (floor ${f.view_sunk_cm_max ?? 20} cm)`;
    const first = f.view_first_tick_max ?? 30, span = f.view_sim_span_min ?? 60;
    if (v.sim_first !== null && (v.sim_first > first || v.sim_last - v.sim_first < span)) red.unseen_fight = `the window showed sim ticks ${v.sim_first}..${v.sim_last}: the fight must start on screen (first tick at most ${first}) and play at least ${span} ticks`;
  }
  if ((look.validation_errors ?? 0) > (f.validation_errors ?? 0)) red.validation = `${look.validation_errors} wgpu validation errors`;
  if (caps.blank > (f.blank_frames ?? 0)) red.blank = `${caps.blank} of ${caps.captures} captures are blank (PNG under 60 KB)`;
  if (caps.frozen > (f.frozen_pairs ?? 0)) red.freeze = `${caps.frozen} pairs of consecutive captures (60 frames apart) are byte-identical: the picture froze`;
  if (timing && !timing.loaded && timing.interval_p99_ms > (f.stall_p99_ms_max ?? 1500)) red.stall = `frame interval p99 ${timing.interval_p99_ms} ms (floor ${f.stall_p99_ms_max ?? 1500} ms)`;
  if (timing && !timing.loaded && ref && timing.interval_p50_ms > ref / (1 - (f.fps_drop_max ?? 0.3))) {
    red.slower = `frame interval p50 ${timing.interval_p50_ms} ms vs ${ref} ms reference (${Math.round(1000 / timing.interval_p50_ms)} vs ${Math.round(1000 / ref)} fps)`;
  }
  for (const [k, v] of Object.entries(passFloors(look.passes, passBar ?? bar.passBar ?? bar))) red[k] = v;
  return red;
};

// Where each symptom points first; the packet names them as suspects, not as the answer.
const SUSPECTS = {
  hang: ["crates/game/src/bin/arena.rs", "crates/render/src/present/mod.rs"],
  panic: ["crates/game/src/bin/arena.rs"],
  exit: ["crates/game/src/bin/arena.rs"],
  receipt: ["crates/game/src/fighter_look.rs", "crates/game/src/bin/arena.rs"],
  framing: ["crates/game/src/fighter_look.rs", "crates/game/src/director.rs"],
  strobe: ["crates/game/src/fighter_look.rs"],
  validation: ["crates/render/src/present/mod.rs"],
  blank: ["crates/render/src/present/mod.rs", "crates/game/src/bin/arena.rs"],
  freeze: ["crates/game/src/bin/arena.rs", "crates/game/src/fighter_look.rs"],
  stall: ["crates/game/src/bin/arena.rs", "crates/render/src/present/mod.rs"],
  slower: ["crates/game/src/bin/arena.rs", "crates/render/src/present/mod.rs"],
  view_receipt: ["crates/game/src/view_truth.rs", "crates/game/src/fighter_look.rs"],
  nan: ["crates/game/src/fighter_look.rs", "crates/anim/src/pose.rs"],
  error_material: ["crates/game/src/fighter_look.rs", "crates/render/src/v1_skin.rs"],
  camera_in_body: ["crates/game/src/fighter_look.rs"],
  camera_under_ground: ["crates/game/src/fighter_look.rs"],
  sunk: ["crates/game/src/fighter_look.rs"],
  unseen_fight: ["crates/game/src/bin/arena.rs"],
};

export const packetText = ({ metric, why, sha, runDir, history }) => {
  const paths = metric.startsWith("pass-")
    ? ["crates/render/src/present/mod.rs", "tools/pass-bar.json"]
    : (SUSPECTS[metric] ?? ["crates/game/src/bin/arena.rs"]);
  const since = history.filter((h) => !h.red?.[metric] && h.sha).at(-1)?.sha;
  return `---
role: builder
seat: coder
chain: start
title: Product check ${metric}: ${why.slice(0, 90)}
row: bug:product-${metric}
paths: ${paths.join(", ")}
test: cargo test -p engine-game --features app --lib; cargo check -p engine-game --features app --bin arena
max_diff: 200
source: node tools/product-check.mjs (keeper, tree at ${sha}); runs ${runDir}; last green ${since ?? "none recorded"}; target/reports/product/latest.md
---
Goal: The built arena broke a product floor at ${sha}: ${why}. Find the cause and fix it so the next product check is green for \`${metric}\`.
State: the keeper's product check ran \`target/release/arena.exe --scene arena --fighters 8 --frames 300 --exit --require-framing --capture-every 60\` and a timing run. Its numbers and captures: target/reports/product/latest.md and ${runDir}. ${since ? `It was green at ${since}: \`git log --oneline ${since}..HEAD -- crates\` lists the suspects.` : "No green check is recorded yet."}
Scope: the files in \`paths:\` first (they are where this symptom usually starts, not a verdict). Read the receipt lines in latest.md and the run's \`view.jsonl\` (one JSON line per frame: camera, then each body's act, clip, tint, bounds and screen rect; numbers first, open the capture PNGs with Read only if the numbers do not explain it), then write one failing unit test that shows the cause (a pure function over positions, cues or timings, no window), then the fix. At most 200 changed lines.
Proof: the new test fails before and passes after; the \`test:\` lines green with --features app (a run with 0 tests proves nothing); paste the before and after numbers from latest.md.
Stop: the cause is in a crate outside \`paths:\` or needs a rules or hash change: RESULT: BLOCKED - <file and symbol>, one line. Never weaken tools/product-bar.json.
`;
};

const run = (args) => {
  const r = spawnSync(ARENA, args, { cwd: ROOT, encoding: "utf8", timeout: RUN_MS, maxBuffer: 64 * 1024 * 1024, windowsHide: false });
  const text = `${r.stdout ?? ""}\n${r.stderr ?? ""}`;
  const timedOut = r.error?.code === "ETIMEDOUT" || r.signal === "SIGTERM";
  return { ...parseReceipt(text), exit: r.status, timedOut, framingFailed: /showed no fighter/.test(text), text };
};

const buildsRunning = () => {
  try { return execFileSync("tasklist", ["/FO", "CSV", "/NH"], { encoding: "utf8" }).split("\n").filter((l) => /^"(cargo|rustc)\.exe"/i.test(l)).length; } catch { return 0; }
};

const readJson = (p, dflt) => { try { return JSON.parse(readFileSync(p, "utf8")); } catch { return dflt; } };
const git = (...a) => execFileSync("git", a, { cwd: ROOT, encoding: "utf8" }).trim();
const utc = () => new Date().toISOString().replace(/[-:]/g, "").slice(0, 13) + "Z";

const build = async () => {
  const arena = BINS.find((b) => b.exe === "arena");
  const [st] = binStatus(ROOT, [arena]);
  if (st.state === "FRESH") return { ok: true, why: "fresh" };
  const dir = join(ROOT, "target", "cargo-slots");
  const id = `product-${process.pid}`;
  const max = Math.max(1, knob(readKnobs(ROOT), "cargo_slots", 3));
  const start = Date.now();
  for (;;) {
    if (acquireSlot({ dir, max, id, info: { seat: "product-check", cmd: arena.build, pid: process.pid } }).ok) break;
    if (Date.now() - start > SLOT_WAIT_MS) return { ok: false, why: "no cargo slot in 20 min" };
    await new Promise((r) => setTimeout(r, 15_000));
  }
  try {
    const [cmd, ...args] = arena.build.split(" ");
    const r = spawnSync(cmd, args, { cwd: ROOT, encoding: "utf8", timeout: BUILD_MS, maxBuffer: 64 * 1024 * 1024, env: { ...process.env, CARGO_BUILD_JOBS: "10", RUSTC_WRAPPER: "" }, windowsHide: true });
    const tail = `${r.stdout ?? ""}\n${r.stderr ?? ""}`.trim().split(/\r?\n/).slice(-20).join("\n");
    return r.status === 0 ? { ok: true, why: `built (${st.state.toLowerCase()})` } : { ok: false, why: `build failed (the working tree, maybe a coder mid-edit):\n${tail}` };
  } finally {
    releaseSlot({ dir, id });
  }
};

const pruneRuns = (runs) => {
  if (!existsSync(runs)) return;
  const dirs = readdirSync(runs).filter((d) => statSync(join(runs, d)).isDirectory()).sort();
  for (const d of dirs.slice(0, Math.max(0, dirs.length - KEEP_RUNS))) rmSync(join(runs, d), { recursive: true, force: true });
};

const summary = (c) => {
  const fps = (ms) => (ms ? `${(1000 / ms).toFixed(1)} fps (p50 ${ms} ms)` : "-");
  const reds = Object.entries(c.red);
  return [
    `# Product check ${c.sha} (${c.at})`,
    "",
    `Verdict: ${reds.length ? `RED (${reds.map(([k]) => k).join(", ")})` : "GREEN"}; build: ${c.build}`,
    `Look run: exit ${c.look.exit}, framing empty ${c.look.empty}/${c.look.frames}, fewest in view ${c.look.fewest_in_view}, steepest down ${c.look.steepest_down_deg} deg, tinted share ${c.look.tinted_share}, validation errors ${c.look.validation_errors}, captures ${c.caps.captures} (blank ${c.caps.blank}, frozen pairs ${c.caps.frozen}) in ${c.runDir}`,
    `Timing run: ${fps(c.timing?.interval_p50_ms)}, p99 ${c.timing?.interval_p99_ms} ms, gpu p50 ${c.timing?.gpu_p50_ms} ms, cpu p50 ${c.timing?.cpu_p50_ms} ms, ${c.timing?.loaded ? `LOADED (${c.timing.loaded} cargo/rustc running: fps not gated)` : "quiet"}; reference ${c.ref ? fps(c.ref) : "none yet"}`,
    `View truth (no image; per frame in ${c.runDir}/view.jsonl): ${c.look.view ? Object.entries(c.look.view).map(([k, x]) => `${k} ${x}`).join(", ") : "none"}`,
    `Live GPU passes (ms > 0): ${Object.entries(c.look.passes ?? {}).filter(([, v]) => v > 0).map(([k]) => k).join(", ") || "none"}`,
    `Pass floors (tools/pass-bar.json): ${Object.entries(c.passBar?.passes ?? c.bar.passBar?.passes ?? {}).map(([k, v]) => `${k} ${v}`).join(", ") || "none (no pass-bar)"}`,
    `Finish targets (tools/product-bar.json): ${Object.entries(c.bar.targets ?? {}).map(([k, v]) => `${k} ${v}`).join("; ")}`,
    "",
    ...reds.map(([k, v]) => `- RED ${k}: ${v}${c.filed.includes(k) ? " (bug packet filed)" : ""}`),
    "",
  ].join("\n");
};

const main = async () => {
  const argv = process.argv.slice(2);
  const has = (f) => argv.includes(f);
  mkdirSync(OUT, { recursive: true });
  const latestFile = join(OUT, "latest.json");
  const latest = readJson(latestFile, null);
  const tree = git("rev-parse", "HEAD:crates");
  const sha = git("rev-parse", "--short", "HEAD");
  if (existsSync(join(ROOT, "target", "sprint.halt")) && has("--if-new")) return console.log("product-check: SKIP sprint.halt");
  if (has("--if-new") && latest?.tree === tree) return console.log(`product-check: SAME tree as ${latest.sha} (${latest.verdict})`);
  const lockFile = join(OUT, "lock.json");
  const lock = readJson(lockFile, null);
  if (lock && pidAlive(lock.pid) && Date.now() - lock.at < 90 * 60_000) return console.log(`product-check: BUSY pid ${lock.pid}`);
  writeFileSync(lockFile, JSON.stringify({ pid: process.pid, at: Date.now(), sha }));
  try {
    const bar = readJson(join(ROOT, "tools", "product-bar.json"), { floors: {}, targets: {} });
    const passBar = readJson(join(ROOT, "tools", "pass-bar.json"), { passes: {} });
    const built = has("--no-build") ? { ok: existsSync(ARENA), why: "not built (--no-build)" } : await build();
    if (!built.ok || !existsSync(ARENA)) {
      console.log(`product-check: NOT RUN at ${sha}: ${built.why}`);
      process.exitCode = 2;
      return;
    }
    const runs = join(OUT, "runs");
    const runDir = join(runs, `${sha}-${utc()}`);
    mkdirSync(runDir, { recursive: true });
    const look = run(["--scene", "arena", "--fighters", "8", "--frames", "300", "--exit", "--require-framing", "--capture-dir", runDir, "--capture-every", "60", "--sha", sha, "--view-brief", join(runDir, "view.jsonl")]);
    writeFileSync(join(runDir, "look.txt"), look.text);
    const loaded = buildsRunning();
    const timing = { ...run(["--scene", "arena", "--fighters", "8", "--frames", "300", "--exit"]), loaded };
    writeFileSync(join(runDir, "timing.txt"), timing.text);
    const pngs = existsSync(runDir) ? readdirSync(runDir).filter((n) => /^frame_\d+\.png$/.test(n)).sort().map((n) => join(runDir, n)) : [];
    const caps = scoreCaptures(pngs);
    const historyFile = join(OUT, "history.jsonl");
    const history = existsSync(historyFile) ? readFileSync(historyFile, "utf8").split("\n").filter(Boolean).map((l) => { try { return JSON.parse(l); } catch { return null; } }).filter(Boolean) : [];
    const ref = referenceInterval(history);
    const red = redFloors({ look, timing, caps, ref }, bar, passBar);
    const filed = [];
    const lastFiled = latest?.filedAt ?? {};
    const filedAt = { ...lastFiled };
    if (!has("--no-file")) {
      mkdirSync(READY, { recursive: true });
      for (const [metric, why] of Object.entries(red)) {
        const name = `bug-product-${metric}.md`;
        if ([READY, RUNNING].some((d) => existsSync(join(d, name)))) continue;
        if (Date.now() - (lastFiled[metric] ?? 0) < REFILE_MS) continue;
        writeFileSync(join(READY, name), packetText({ metric, why, sha, runDir: runDir.replace(ROOT + "\\", "").replace(/\\/g, "/"), history }));
        filed.push(metric);
        filedAt[metric] = Date.now();
      }
    }
    const strip = ({ text, ...rest }) => rest;
    const check = { sha, tree, at: new Date().toISOString(), build: built.why.split("\n")[0], verdict: Object.keys(red).length ? "RED" : "GREEN", red, filed, filedAt, ref, caps, look: strip(look), timing: strip(timing), runDir: runDir.replace(/\\/g, "/"), bar, passBar };
    writeFileSync(latestFile, `${JSON.stringify(check, null, 2)}\n`);
    appendFileSync(historyFile, `${JSON.stringify({ sha, tree, at: check.at, verdict: check.verdict, red, timing: { interval_p50_ms: timing.interval_p50_ms, interval_p99_ms: timing.interval_p99_ms, loaded }, look: { empty: look.empty, tinted_share: look.tinted_share, view: look.view } })}\n`);
    writeFileSync(join(OUT, "latest.md"), summary(check));
    pruneRuns(runs);
    if (has("--json")) process.stdout.write(`${JSON.stringify(check, null, 2)}\n`);
    const fps = timing.interval_p50_ms ? (1000 / timing.interval_p50_ms).toFixed(1) : "-";
    console.log(`product-check: ${check.verdict} ${sha} fps=${fps}${loaded ? "(loaded)" : ""} empty=${look.empty} tinted=${look.tinted_share} pops=${look.view?.pops ?? "-"} sunk=${look.view?.sunk_cm_max ?? "-"} ticks=${look.view ? `${look.view.sim_first}..${look.view.sim_last}` : "-"} blank=${caps.blank} frozen=${caps.frozen} val=${look.validation_errors}${Object.keys(red).length ? ` red=${Object.keys(red).join(",")}` : ""}${filed.length ? ` filed=${filed.map((m) => `bug-product-${m}`).join(",")}` : ""}`);
    process.exitCode = Object.keys(red).length ? 1 : 0;
  } finally {
    rmSync(lockFile, { force: true });
  }
};

if (import.meta.main) await main();
