// Seal keys for perf and evidence seals, made by the AI (no owner step).
// Keeps E2040_PERF_SEAL_KEY and E2040_EVIDENCE_KEY (64 hex = 32 random bytes each)
// in <home>/.empire/state/engine2040-seal-keys.json, outside every repo.
// Keys are never printed and never committed.
//   node tools/seal-keys.mjs --ensure          make the file if missing, print only OK
//   node tools/seal-keys.mjs -- <cmd> [args]   run <cmd> with both keys in its env
import { spawnSync } from "node:child_process";
import { randomBytes } from "node:crypto";
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { homedir } from "node:os";
import { dirname, join } from "node:path";

export const KEY_NAMES = ["E2040_PERF_SEAL_KEY", "E2040_EVIDENCE_KEY"];
const HEX64 = /^[0-9a-f]{64}$/;

export function keyFile(home = homedir()) {
  return join(home, ".empire", "state", "engine2040-seal-keys.json");
}

/** Read the key file, making it (and any missing or malformed key) first. */
/** A corrupt or non-object key file warns loudly, then missing keys regenerate. */
export function ensureKeys(file = keyFile()) {
  let keys = {};
  if (existsSync(file)) {
    try { keys = JSON.parse(readFileSync(file, "utf8")); } catch (err) {
      console.warn(`WARN seal-keys: ${file} unreadable (${err?.message ?? err}), regenerating missing keys`);
      keys = {};
    }
    if (keys === null || typeof keys !== "object" || Array.isArray(keys)) {
      console.warn(`WARN seal-keys: ${file} holds non-object JSON, regenerating missing keys`);
      keys = {};
    }
  }
  let changed = false;
  for (const name of KEY_NAMES) {
    if (typeof keys[name] !== "string" || !HEX64.test(keys[name])) {
      keys[name] = randomBytes(32).toString("hex");
      changed = true;
    }
  }
  if (changed || !existsSync(file)) {
    mkdirSync(dirname(file), { recursive: true });
    writeFileSync(file, JSON.stringify(keys, null, 2) + "\n");
  }
  return Object.fromEntries(KEY_NAMES.map((name) => [name, keys[name]]));
}

/** Loud exit for a spawnSync result: spawn errors and signals report, never a bare code. */
export function describeSpawn(run) {
  if (run?.error) return { code: 1, problem: `spawn failed: ${run.error?.message ?? run.error}` };
  if (run?.signal) return { code: 1, problem: `killed by signal ${run.signal}` };
  if (run?.status === null || run?.status === undefined) return { code: 1, problem: "no exit status" };
  return { code: run.status, problem: null };
}
/** Env entries to hand a child; a key already set in the environment wins. */
export function sealKeyEnv(file = keyFile(), env = process.env) {
  const keys = ensureKeys(file);
  return Object.fromEntries(KEY_NAMES.map((name) => [name, env[name] || keys[name]]));
}

function main(argv) {
  if (argv[0] === "--ensure") {
    ensureKeys();
    console.log(`OK: ${KEY_NAMES.join(" + ")} ready (values not printed)`);
    return 0;
  }
  const dash = argv.indexOf("--");
  if (dash === -1 || dash === argv.length - 1) {
    console.error("usage: node tools/seal-keys.mjs --ensure | -- <cmd> [args]");
    return 2;
  }
  const [cmd, ...args] = argv.slice(dash + 1);
  const run = spawnSync(cmd, args, { stdio: "inherit", env: { ...process.env, ...sealKeyEnv() }, shell: /\.(cmd|bat)$/i.test(cmd) });
  const { code, problem } = describeSpawn(run);
  if (problem) console.error(`seal-keys: ${cmd}: ${problem}`);
  return code;
}

if (process.argv[1]?.endsWith("seal-keys.mjs")) {
  process.exit(main(process.argv.slice(2)));
}
