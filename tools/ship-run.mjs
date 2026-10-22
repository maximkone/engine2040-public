#!/usr/bin/env node
// ENG19: the ship check must run the shipped build, not only build it. This
// starts the proof binary from the clean folder with a sanitized environment,
// reads the run receipt and refuses a receipt that is missing, carries no run
// line, reports a zero tick or a hash that is not the golden. It also starts
// the shipped arena binary so the job proves the game boots.
//
//   node tools/ship-run.mjs --proof <engine2040.exe> [--arena <arena.exe>]
//                            [--log <path>] [--frames N] [--golden <hex>]
//
// The proof receipt is the only gate: its `ticks=<n> final_hash=<16 hex>` line
// must carry the golden hash and a positive tick. The arena run prints its own
// smoke receipt; its sim hash is the arena's own, never the golden.
//
// A hosted runner may expose no wgpu adapter: the arena presenter then exits
// before the smoke line. That one case is a named skip that says
// `adapter=none`; every other arena failure stays red.
//
// The frame score against content/reference/ is not built here: the reference
// set is a Rust recipe and no PNG rides in git, so the tool names the adapter
// and prints frame=skipped instead of inventing a score.
//
// Exit codes: 0 pass - 1 the build did not prove itself - 2 usage.
import { spawnSync } from "node:child_process";
import { existsSync, mkdirSync, statSync, writeFileSync } from "node:fs";
import { dirname, resolve } from "node:path";

// The golden hash of the root proof fixture: seed 2040, 600 ticks, no inputs.
export const GOLDEN_HASH = "d57b5c8ae3096614";

// The arena boot proof plays half a second of game time. Its own 5 min default
// deadline is too wide for one CI step, so the run names a tighter bound.
export const DEFAULT_FRAMES = 30;
export const ARENA_DEADLINE_MS = 120_000;
export const ARENA_TIMEOUT_MS = 150_000;
export const PROOF_TIMEOUT_MS = 120_000;

// One named refusal. `detail` says what was read and what was wanted, so the
// red line is enough to fix the run without opening the log.
function refuse(reason, detail) {
  return { ok: false, reason, detail };
}

// Check the proof receipt text. The first line names the receipt; the second
// line carries the tick count and the final hash. A run that verified itself
// also says `verified=true` on the third line.
export function checkReceipt(
  text,
  { golden = GOLDEN_HASH, minTicks = 1, requireVerified = true } = {},
) {
  const source = String(text ?? "");
  if (!/^engine2040 headless proof:/m.test(source)) {
    return refuse("MISSING_RECEIPT", "no 'engine2040 headless proof:' line in the run output");
  }
  const run = source.match(/^\s*ticks=(\d+)\s+final_hash=([0-9a-fA-F]{16})\s*$/m);
  if (!run) {
    return refuse("NO_RUN_LINE", "the receipt has no 'ticks=<n> final_hash=<16 hex>' line");
  }
  const ticks = Number(run[1]);
  const hash = run[2].toLowerCase();
  if (!(ticks >= minTicks)) {
    return refuse("ZERO_TICK", `the receipt reports ticks=${ticks}; want at least ${minTicks}`);
  }
  if (hash !== golden) {
    return refuse("HASH_NOT_GOLDEN", `final_hash=${hash}; want the golden ${golden}`);
  }
  const seed = source.match(/^\s*seed=(\d+)\b[^\n]*\bverified=(true|false)\b/m);
  const verified = seed ? seed[2] === "true" : null;
  if (requireVerified && verified !== true) {
    return refuse("NOT_VERIFIED", "the receipt does not say verified=true (run the proof with --verify)");
  }
  return {
    ok: true,
    reason: "PASS",
    detail: `ticks=${ticks} hash=${hash}`,
    ticks,
    hash,
    seed: seed ? Number(seed[1]) : null,
    verified,
  };
}

// The arena's exit receipt: `arena smoke:` from a window run, or
// `arena headless smoke:` when there is no display. Both carry the tick count,
// the presented frame count and the presenter notice that names the adapter.
export function parseSmoke(text) {
  const line = String(text ?? "")
    .split(/\r?\n/)
    .find((row) => /^arena (headless )?smoke:/.test(row));
  if (!line) return null;
  const ticks = Number((line.match(/(?:^|\s)ticks=(\d+)/) ?? [])[1]);
  const frames = Number((line.match(/(?:^|\s)frames=(\d+)/) ?? [])[1]);
  if (!Number.isInteger(ticks) || !Number.isInteger(frames)) return null;
  const presenter = (line.match(/presenter="([^"]*)"/) ?? [])[1] ?? "";
  return {
    kind: /^arena headless smoke:/.test(line) ? "headless" : "window",
    ticks,
    frames,
    presenter,
    adapter: adapterName(presenter),
  };
}

// The adapter as the presenter notice prints it:
//   presenter twin: NVIDIA GeForce RTX 3050 (537.13)      headless
//   presenter surface: NVIDIA GeForce RTX 3050 (537.13)   window
//   presenter skipped: no wgpu adapter on this machine    no adapter
export function adapterName(presenter) {
  const text = String(presenter ?? "");
  const named = text.match(/presenter (?:twin|surface): (.+?) \(/);
  if (named) return named[1].trim();
  if (/presenter skipped/i.test(text)) return "none";
  return "unknown";
}

// A boot that never reaches the arena smoke line because the machine has no
// wgpu adapter. The presenter prints exactly this before it exits, and the
// row says to name the adapter when the runner cannot present.
export function noAdapter(text) {
  return /no wgpu adapter available/i.test(String(text ?? ""));
}

// The label the job prints for the frame leg. A software rasterizer is not a
// real GPU adapter, so it reads as `warp` and the frame score stays skipped.
export function adapterLabel(name) {
  const text = String(name ?? "").toLowerCase();
  if (!text || text === "none" || text === "unknown") return "none";
  if (/warp|microsoft basic render|software|llvmpipe/.test(text)) return "warp";
  return name;
}

// Check the arena boot receipt: a smoke line with a positive tick count.
export function checkSmoke(text) {
  const smoke = parseSmoke(text);
  if (!smoke) {
    return refuse("GAME_NO_RECEIPT", "no 'arena smoke:' or 'arena headless smoke:' line in the run output");
  }
  if (!(smoke.ticks > 0)) {
    return refuse("GAME_ZERO_TICK", `the game run reports ticks=${smoke.ticks}`);
  }
  return { ok: true, reason: "PASS", detail: `ticks=${smoke.ticks} frames=${smoke.frames}`, smoke };
}

// The child sees only what a deployed build needs: the system paths and the
// temp folders. Dev tools, cargo and model keys stay out, so a run that needs
// the checkout's environment cannot pass as a clean-folder run.
const CHILD_ENV_KEEP = [
  "SystemRoot",
  "SystemDrive",
  "windir",
  "OS",
  "PATHEXT",
  "COMSPEC",
  "TEMP",
  "TMP",
  "USERPROFILE",
  "LOCALAPPDATA",
  "HOMEDRIVE",
  "HOMEPATH",
  "NUMBER_OF_PROCESSORS",
  "PROCESSOR_ARCHITECTURE",
];

export function childEnv(source = process.env) {
  const env = {};
  for (const key of CHILD_ENV_KEEP) {
    if (source[key] !== undefined) env[key] = source[key];
  }
  const root = source.SystemRoot ?? "C:\\Windows";
  env.PATH = `${root}\\System32;${root}`;
  return env;
}

// One child process with its output as text. The spawn is injected so a test
// can pin the parsing without building or starting a binary.
export function runProcess(command, args, options = {}) {
  const {
    cwd,
    env = childEnv(),
    timeoutMs = PROOF_TIMEOUT_MS,
    spawn = spawnSync,
  } = options;
  const result = spawn(command, args, {
    cwd,
    env,
    encoding: "utf8",
    timeout: timeoutMs,
    maxBuffer: 32 * 1024 * 1024,
    windowsHide: true,
  });
  const out = `${result.stdout ?? ""}${result.stderr ?? ""}`;
  return { status: result.status, error: result.error ?? null, out };
}

// Run the proof binary from its own folder and check its receipt. `args`
// exists for the tests: the CLI always runs the binary with `--verify`.
export function runProof(exe, options = {}) {
  const path = resolve(exe);
  const {
    args = ["--verify"],
    spawn = spawnSync,
    env = childEnv(),
    timeoutMs = PROOF_TIMEOUT_MS,
    golden = GOLDEN_HASH,
    minTicks = 1,
  } = options;
  const run = runProcess(path, args, { cwd: dirname(path), env, timeoutMs, spawn });
  if (run.error) {
    return {
      ...refuse("PROOF_FAILED", `cannot start the proof binary: ${run.error.message}`),
      out: run.out,
      status: run.status,
    };
  }
  const check = checkReceipt(run.out, { golden, minTicks });
  if (run.status !== 0) {
    return {
      ...refuse(check.ok ? "PROOF_EXIT" : check.reason, `the proof binary exited ${run.status}; ${check.detail}`),
      out: run.out,
      status: run.status,
    };
  }
  return { ...check, out: run.out, status: run.status };
}

// Run the shipped arena binary headless for a few ticks and check its boot
// receipt. The clean folder is the child's working folder, so the run cannot
// rely on the checkout's current directory.
export function runArena(exe, options = {}) {
  const path = resolve(exe);
  const {
    frames = DEFAULT_FRAMES,
    spawn = spawnSync,
    env = childEnv(),
    deadlineMs = ARENA_DEADLINE_MS,
    timeoutMs = ARENA_TIMEOUT_MS,
  } = options;
  const args = [
    "--frames",
    String(frames),
    "--exit",
    "--mute",
    "--deadline-ms",
    String(deadlineMs),
  ];
  const run = runProcess(path, args, { cwd: dirname(path), env, timeoutMs, spawn });
  if (run.error) {
    return {
      ...refuse("GAME_FAILED", `cannot start the arena binary: ${run.error.message}`),
      out: run.out,
      status: run.status,
    };
  }
  const check = checkSmoke(run.out);
  if (!check.ok && noAdapter(run.out)) {
    return {
      ...refuse("GAME_NO_ADAPTER", `no wgpu adapter on this machine; the arena binary exited ${run.status}`),
      out: run.out,
      status: run.status,
    };
  }
  if (run.status !== 0) {
    return {
      ...refuse(check.ok ? "GAME_EXIT" : check.reason, `the arena binary exited ${run.status}; ${check.detail}`),
      out: run.out,
      status: run.status,
    };
  }
  return { ...check, out: run.out, status: run.status };
}

// The last non-empty lines of one run: a red names the end of the output, not
// the whole boot log.
export function tail(text, count = 20) {
  return String(text ?? "")
    .split(/\r?\n/)
    .filter((line) => line.trim() !== "")
    .slice(-count);
}

export const HELP = `ship-run.mjs — run the shipped build and check its receipt

Usage:
  node tools/ship-run.mjs --proof <engine2040.exe> [--arena <arena.exe>]
                           [--log <path>] [--frames N] [--golden <hex>]

The proof binary runs with --verify from its own folder, under an environment
that holds only the system paths and the temp folders. Its receipt must say
\`ticks=<n> final_hash=d57b5c8ae3096614\` with n > 0, and it must have verified
itself; the tool then prints \`SHIP-RUN tick=<n> hash=<h>\`.

--arena adds the game boot proof: the arena binary plays N frames (default 30)
headless and must print its smoke receipt with a positive tick count. Its sim
hash is the arena's own, so it is quoted, never compared with the golden. A
machine with no wgpu adapter reaches no smoke line: that one case prints
\`SHIP-GAME skipped adapter=none reason=no-wgpu-adapter\` and does not fail the
job. The frame score against content/reference/ is not available here (the
reference set is a Rust recipe, no PNG in git), so the tool prints the adapter
and \`frame=skipped\` instead of a score.

Named refusals print as \`SHIP-RUN FAIL [REASON] <detail>\`.
Exit codes: 0 pass - 1 the build did not prove itself - 2 usage.
`;

function parseArgs(argv) {
  const out = {
    help: false,
    proof: null,
    arena: null,
    log: null,
    frames: DEFAULT_FRAMES,
    golden: GOLDEN_HASH,
  };
  for (let i = 0; i < argv.length; i++) {
    const arg = argv[i];
    const value = (flag) => {
      const next = argv[i + 1];
      if (next === undefined || next.startsWith("--")) throw new Error(`missing value for ${flag}`);
      i++;
      return next;
    };
    switch (arg) {
      case "--help":
      case "-h":
        out.help = true;
        break;
      case "--proof":
        out.proof = value(arg);
        break;
      case "--arena":
        out.arena = value(arg);
        break;
      case "--log":
        out.log = value(arg);
        break;
      case "--frames": {
        const frames = Number(value(arg));
        if (!Number.isInteger(frames) || frames <= 0) throw new Error("--frames needs a positive integer");
        out.frames = frames;
        break;
      }
      case "--golden": {
        const golden = value(arg).toLowerCase();
        if (!/^[0-9a-f]{16}$/.test(golden)) throw new Error("--golden needs 16 hex digits");
        out.golden = golden;
        break;
      }
      default:
        throw new Error(`unknown arg ${arg}`);
    }
  }
  return out;
}

// The log is the artifact: the SHIP lines first, then the raw output of each
// run, so a reader can check the quoted line against the receipt.
function writeLog(path, lines, sections) {
  const body = [...lines];
  for (const [name, text] of sections) {
    body.push(`--- ${name} output begin ---`);
    body.push(String(text ?? "").trimEnd() || "(no output)");
    body.push(`--- ${name} output end ---`);
  }
  mkdirSync(dirname(resolve(path)), { recursive: true });
  writeFileSync(path, `${body.join("\n")}\n`, "utf8");
}

export function main(
  argv,
  { out = console.log, err = console.error, spawn = spawnSync, env = childEnv() } = {},
) {
  let args;
  try {
    args = parseArgs(argv);
  } catch (error) {
    err(`ship-run: ${error.message}\n${HELP}`);
    return 2;
  }
  if (args.help) {
    out(HELP.trimEnd());
    return 0;
  }
  if (!args.proof) {
    err(`ship-run: need --proof <engine2040.exe>\n${HELP}`);
    return 2;
  }
  const proofPath = resolve(args.proof);
  if (!existsSync(proofPath) || !statSync(proofPath).isFile()) {
    err(`SHIP-RUN FAIL [MISSING_PROOF] no such file: ${proofPath}`);
    return 1;
  }

  const lines = [];
  const say = (line) => {
    lines.push(line);
    out(line);
  };
  const sections = [];
  let failed = false;

  const proof = runProof(proofPath, { spawn, env, golden: args.golden });
  sections.push(["proof", proof.out]);
  if (proof.ok) {
    say(`SHIP-RUN tick=${proof.ticks} hash=${proof.hash}`);
  } else {
    failed = true;
    say(`SHIP-RUN FAIL [${proof.reason}] ${proof.detail}`);
    say(`SHIP-RUN FAIL: the proof output ends:`);
    for (const line of tail(proof.out)) say(line);
  }

  if (args.arena) {
    const arenaPath = resolve(args.arena);
    if (!existsSync(arenaPath) || !statSync(arenaPath).isFile()) {
      failed = true;
      say(`SHIP-RUN FAIL [MISSING_ARENA] no such file: ${arenaPath}`);
    } else {
      const arena = runArena(arenaPath, { spawn, env, frames: args.frames });
      sections.push(["arena", arena.out]);
      if (arena.ok) {
        const smoke = arena.smoke;
        const label = adapterLabel(smoke.adapter);
        say(
          `SHIP-GAME ticks=${smoke.ticks} frames=${smoke.frames} kind=${smoke.kind} presenter="${smoke.presenter}"`,
        );
        const reason = label === "none" || label === "warp" ? "no-hardware-adapter" : "reference-png-not-on-disk";
        say(`SHIP-FRAME skipped adapter=${label} reason=${reason}`);
      } else if (arena.reason === "GAME_NO_ADAPTER") {
        // The runner cannot present, so the game boot is named, not scored.
        // The proof receipt above still gates the job.
        say(`SHIP-GAME skipped adapter=none reason=no-wgpu-adapter (${arena.detail})`);
        say("SHIP-FRAME skipped adapter=none reason=no-wgpu-adapter");
      } else {
        failed = true;
        say(`SHIP-GAME FAIL [${arena.reason}] ${arena.detail}`);
        say("SHIP-GAME FAIL: the arena output ends:");
        for (const line of tail(arena.out)) say(line);
      }
    }
  }

  const verdict = failed
    ? "SHIP-RUN FAIL: the shipped build did not prove itself"
    : "SHIP-RUN PASS: the shipped build ran headless and its receipt is the golden";
  lines.push(verdict);

  if (args.log) {
    try {
      writeLog(args.log, lines, sections);
      out(`SHIP-RUN log: ${resolve(args.log)}`);
    } catch (error) {
      err(`SHIP-RUN FAIL [LOG_UNWRITABLE] cannot write ${args.log}: ${error.message}`);
      return 1;
    }
  }

  if (failed) {
    err(verdict);
    return 1;
  }
  out(verdict);
  return 0;
}

if (import.meta.main) {
  try {
    process.exitCode = main(process.argv.slice(2));
  } catch (error) {
    console.error(`ship-run: ${error.message}`);
    process.exitCode = 2;
  }
}
