// ENG19: the ship check must run the shipped build, not only build it. These
// tests hold the checker to the receipt it accepts: the golden hash and a
// positive tick pass, a wrong hash or a zero tick is refused by name, a
// receipt that did not verify itself is refused, and a real broken process
// turns the tool red. They also pin the arena boot line and the workflow call.
import assert from "node:assert/strict";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import {
  adapterLabel,
  adapterName,
  checkReceipt,
  checkSmoke,
  childEnv,
  GOLDEN_HASH,
  main,
  parseSmoke,
  runProof,
} from "./ship-run.mjs";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");

// The text the root proof binary prints for the golden run (`--verify`).
const GOOD = `engine2040 headless proof:
  ticks=600 final_hash=${GOLDEN_HASH}
  seed=2040 inputs=0 state_hash=0123456789abcdef backend=null mode=run verified=true
`;
const BAD_HASH = GOOD.replace(GOLDEN_HASH, "0000000000000000");
const ZERO_TICK = GOOD.replace("ticks=600", "ticks=0");

// The arena's receipts, window and headless, with and without an adapter.
const HEADLESS_SMOKE = `arena headless smoke: ticks=30 frames=30 hud_cells=5 hud_quads=3 mixed_frames=0 audio="audio muted: no device opened; mixer voices render offline" presenter="presenter skipped: no wgpu adapter on this machine" display="no display"`;
const WINDOW_SMOKE = `arena smoke: ticks=30 frames=30 alpha=1.000 hud_cells=5 hud_quads=3 fight_draws=9 audio="audio muted: no device opened; mixer voices render offline" presenter="presenter surface: Microsoft Basic Render Driver (10.0.1)"`;
const REAL_GPU_SMOKE = `arena headless smoke: ticks=30 frames=30 hud_cells=5 hud_quads=3 mixed_frames=0 audio="audio muted" presenter="presenter twin: NVIDIA GeForce RTX 3050 (537.13)" display="none"`;

// A temp folder that goes away with the test.
function tempDir(t) {
  const dir = mkdtempSync(join(tmpdir(), "ship-run-"));
  t.after(() => rmSync(dir, { recursive: true, force: true }));
  return dir;
}

// A spawn that always returns the same text, so main() runs without a build.
const fixedSpawn = (stdout, status = 0) => () => ({ status, stdout, stderr: "" });

// Two runs in one main() call: the proof answers --verify, the arena plays.
const twoRuns = (proofOut, arenaOut) => (command, args) => ({
  status: 0,
  stdout: args.includes("--verify") ? proofOut : arenaOut,
  stderr: "",
});

function collect() {
  const lines = [];
  const errors = [];
  return {
    lines,
    errors,
    out: (line) => lines.push(line),
    err: (line) => errors.push(line),
  };
}

test("the golden receipt passes and the tool quotes tick and hash", (t) => {
  const dir = tempDir(t);
  const proof = join(dir, "engine2040.exe");
  writeFileSync(proof, "not a real build");
  const io = collect();
  const code = main(["--proof", proof], { ...io, spawn: fixedSpawn(GOOD) });
  assert.equal(code, 0);
  assert.ok(io.lines.includes(`SHIP-RUN tick=600 hash=${GOLDEN_HASH}`), io.lines.join("\n"));
  assert.ok(io.lines.some((line) => line.startsWith("SHIP-RUN PASS")), io.lines.join("\n"));
  assert.equal(io.errors.length, 0);
});

test("a hash that is not the golden is refused by name", () => {
  const check = checkReceipt(BAD_HASH);
  assert.equal(check.ok, false);
  assert.equal(check.reason, "HASH_NOT_GOLDEN");
  assert.match(check.detail, /final_hash=0000000000000000/);
  assert.match(check.detail, new RegExp(GOLDEN_HASH));
});

test("a zero tick is refused by name", () => {
  const check = checkReceipt(ZERO_TICK);
  assert.equal(check.ok, false);
  assert.equal(check.reason, "ZERO_TICK");
});

test("a missing receipt is refused by name", () => {
  const check = checkReceipt("build ok: nothing ran\n");
  assert.equal(check.ok, false);
  assert.equal(check.reason, "MISSING_RECEIPT");
});

test("a receipt with no run line is refused by name", () => {
  const check = checkReceipt("engine2040 headless proof:\n  seed=2040 inputs=0\n");
  assert.equal(check.ok, false);
  assert.equal(check.reason, "NO_RUN_LINE");
});

test("a receipt that did not verify itself is refused by name", () => {
  const unverified = GOOD.replace("verified=true", "verified=false");
  const check = checkReceipt(unverified);
  assert.equal(check.ok, false);
  assert.equal(check.reason, "NOT_VERIFIED");
  // The same text passes when the caller does not ask for verification.
  assert.equal(checkReceipt(unverified, { requireVerified: false }).ok, true);
});

test("a broken receipt makes the CLI red and prints the named refusal", (t) => {
  const dir = tempDir(t);
  const proof = join(dir, "engine2040.exe");
  writeFileSync(proof, "not a real build");
  const io = collect();
  const code = main(["--proof", proof], { ...io, spawn: fixedSpawn(BAD_HASH) });
  assert.equal(code, 1);
  assert.ok(
    io.lines.includes(
      `SHIP-RUN FAIL [HASH_NOT_GOLDEN] final_hash=0000000000000000; want the golden ${GOLDEN_HASH}`,
    ),
    io.lines.join("\n"),
  );
  // The end of the run output is quoted, so the red names what was read.
  assert.ok(io.lines.includes(`  ticks=600 final_hash=0000000000000000`), io.lines.join("\n"));
  assert.match(io.errors.join("\n"), /SHIP-RUN FAIL/);
});

test("a real child process with a broken receipt is refused by name", (t) => {
  const dir = tempDir(t);
  const script = join(dir, "broken-proof.mjs");
  writeFileSync(script, `process.stdout.write(${JSON.stringify(BAD_HASH)});\n`, "utf8");
  const result = runProof(process.execPath, { args: [script] });
  assert.equal(result.status, 0);
  assert.equal(result.ok, false);
  assert.equal(result.reason, "HASH_NOT_GOLDEN");
});

test("a real child process with the golden receipt passes", (t) => {
  const dir = tempDir(t);
  const script = join(dir, "good-proof.mjs");
  writeFileSync(script, `process.stdout.write(${JSON.stringify(GOOD)});\n`, "utf8");
  const result = runProof(process.execPath, { args: [script] });
  assert.equal(result.status, 0);
  assert.equal(result.ok, true);
  assert.equal(result.ticks, 600);
  assert.equal(result.hash, GOLDEN_HASH);
});

test("the headless game line is read with its adapter", () => {
  const smoke = parseSmoke(HEADLESS_SMOKE);
  assert.equal(smoke.kind, "headless");
  assert.equal(smoke.ticks, 30);
  assert.equal(smoke.frames, 30);
  assert.equal(smoke.adapter, "none");
  assert.equal(checkSmoke(HEADLESS_SMOKE).ok, true);
});

test("a software presenter is named warp and a real adapter keeps its name", () => {
  const window = parseSmoke(WINDOW_SMOKE);
  assert.equal(window.kind, "window");
  assert.equal(adapterName(window.presenter), "Microsoft Basic Render Driver");
  assert.equal(adapterLabel(adapterName(window.presenter)), "warp");
  const real = parseSmoke(REAL_GPU_SMOKE);
  assert.equal(adapterLabel(real.adapter), "NVIDIA GeForce RTX 3050");
});

test("the arena leg quotes the game receipt and names the frame skip", (t) => {
  const dir = tempDir(t);
  const proof = join(dir, "engine2040.exe");
  const arena = join(dir, "arena.exe");
  writeFileSync(proof, "not a real build");
  writeFileSync(arena, "not a real build");
  const io = collect();
  const code = main(
    ["--proof", proof, "--arena", arena],
    { ...io, spawn: twoRuns(GOOD, HEADLESS_SMOKE) },
  );
  assert.equal(code, 0, io.lines.join("\n"));
  const game = io.lines.find((line) => line.startsWith("SHIP-GAME "));
  assert.match(game, /^SHIP-GAME ticks=30 frames=30 kind=headless /);
  assert.match(game, /presenter="presenter skipped: no wgpu adapter on this machine"/);
  assert.ok(
    io.lines.includes("SHIP-FRAME skipped adapter=none reason=no-hardware-adapter"),
    io.lines.join("\n"),
  );
});

test("a game run with no smoke line makes the CLI red", (t) => {
  const dir = tempDir(t);
  const proof = join(dir, "engine2040.exe");
  const arena = join(dir, "arena.exe");
  writeFileSync(proof, "not a real build");
  writeFileSync(arena, "not a real build");
  const io = collect();
  const code = main(
    ["--proof", proof, "--arena", arena],
    { ...io, spawn: twoRuns(GOOD, "arena: no display\n") },
  );
  assert.equal(code, 1);
  assert.ok(
    io.lines.some((line) => line.startsWith("SHIP-GAME FAIL [GAME_NO_RECEIPT]")),
    io.lines.join("\n"),
  );
});

test("the child environment keeps the system paths and drops the dev tools", () => {
  const env = childEnv({
    SystemRoot: "C:\\Windows",
    TEMP: "C:\\Temp",
    PATH: "C:\\cargo\\bin;C:\\Windows\\System32",
    CARGO_HOME: "C:\\cargo",
    OPENAI_API_KEY: "secret",
  });
  assert.equal(env.CARGO_HOME, undefined);
  assert.equal(env.OPENAI_API_KEY, undefined);
  assert.equal(env.TEMP, "C:\\Temp");
  assert.equal(env.PATH, "C:\\Windows\\System32;C:\\Windows");
});

test("the workflow runs the tool and the node tests", () => {
  const workflow = readFileSync(join(ROOT, ".github", "workflows", "ship-check.yml"), "utf8")
    .replace(/\r\n/g, "\n");
  // The job builds the proof binary into the clean folder, runs it there and
  // fails when the tool refuses the receipt.
  assert.match(workflow, /node tools\/ship-run\.mjs --proof target\/ship-check\/ship-a\/engine2040\.exe/);
  assert.match(workflow, /--arena target\/ship-check\/ship-a\/arena\.exe/);
  assert.match(workflow, /if \(\$LASTEXITCODE -ne 0\)/);
  // The negative proof runs in the job too.
  assert.match(workflow, /node --test tools\/ship-check\.test\.mjs tools\/ship-run\.test\.mjs/);
});
