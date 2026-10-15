// One orchestrator per working tree: two sprints at once race on files, Cargo
// locks and commits. The lock lives in target/ (never committed). A lock not
// refreshed for 60 minutes counts as stale, from a crashed or closed session
// (the orchestrator refreshes after every wave). `takeover` replaces a lock
// the owner knows is dead, e.g. left by a closed OpenCode app.
// Interactive sessions all call themselves "orchestrator", so `acquire` and
// `takeover` for that owner add a session token (orchestrator#3fa9) and print
// it: that session uses the exact token for refresh and release, and a second
// session (plain "orchestrator" or another token) is refused while the lock
// is fresh (owner 2026-09-25: two orchestrators blocked each other's judges).
//   node tools/sprint-lock.mjs acquire <owner>   fails while a fresh lock exists
//   node tools/sprint-lock.mjs refresh <owner>   keep a long sprint's lock fresh
//   node tools/sprint-lock.mjs release [owner]   at handoff (owner must match a fresh lock)
//   node tools/sprint-lock.mjs takeover <owner>  replace any lock, noisily
//   node tools/sprint-lock.mjs status
import { randomBytes } from "node:crypto";
import { existsSync, mkdirSync, readFileSync, rmSync, statSync, writeFileSync } from "node:fs";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";

const LOCK = "target/sprint.lock";
const STALE_MS = 60 * 60 * 1000;
// Pure owner resolution (exported for tests): throws on a flag-like owner,
// otherwise returns the lease owner (a fresh interactive orchestrator lease
// gets a session token). No filesystem access.
export function parseOwner(command, args) {
  const ownerFlag = args.indexOf("--owner");
  const asked =
    ownerFlag >= 0 ? args[ownerFlag + 1] : args.find((a, i) => !a.startsWith("-") && !args[i - 1]?.startsWith("--"));
  if (asked?.startsWith("-")) {
    throw new Error(`Bad owner "${asked}". Usage: node tools/sprint-lock.mjs acquire|takeover|refresh|release|status [owner]`);
  }
  if ((command === "acquire" || command === "takeover") && (asked ?? "orchestrator") === "orchestrator") {
    return `orchestrator#${randomBytes(2).toString("hex")}`;
  }
  return asked ?? "orchestrator";
}

function main() {
const [command, ...args] = process.argv.slice(2);
// The owner is the first plain word, or the word after --owner; other flags are
// ignored. On 2026-09-26 the loop called `acquire --owner orchestrator --id go2`,
// the lock read "--owner since ...", and the keeper stopped after every wave.
let owner;
try {
  owner = parseOwner(command, args);
} catch (error) {
  console.error(error.message);
  process.exit(2);
}

const ageMinutes = () => Math.round((Date.now() - statSync(LOCK).mtimeMs) / 60_000);
const holder = () => readFileSync(LOCK, "utf8").trim();
const fresh = () => existsSync(LOCK) && Date.now() - statSync(LOCK).mtimeMs < STALE_MS;
const write = () => {
  mkdirSync("target", { recursive: true });
  writeFileSync(LOCK, `${owner} since ${new Date().toISOString()}\n`);
};
// Atomic first-create for acquire: O_CREAT|O_EXCL so concurrent losers get
// EEXIST instead of silently winning last-writer. Stale takeover keeps the
// plain overwrite path below; refresh/takeover/release are untouched.
const writeExclusive = () => {
  mkdirSync("target", { recursive: true });
  writeFileSync(LOCK, `${owner} since ${new Date().toISOString()}\n`, { flag: "wx" });
};

if (command === "acquire") {
  if (fresh()) {
    console.error(`LOCKED by ${holder()} (${ageMinutes()} min ago). Another sprint is running; stop.`);
    process.exit(1);
  }
  // Atomic first: exactly one concurrent creator wins; losers get EEXIST and
  // re-check (a fresh file means they lost the race and must refuse, a stale
  // one keeps the serial takeover path). Serial missing takes the fast path
  // with stdout identical to before.
  try {
    writeExclusive();
  } catch (err) {
    if (err?.code !== "EEXIST") throw err;
    if (fresh()) {
      console.error(`LOCKED by ${holder()} (${ageMinutes()} min ago). Another sprint is running; stop.`);
      process.exit(1);
    }
    console.log(`[INFO] replacing stale lock: ${holder()} (${ageMinutes()} min old)`);
    write();
  }
  console.log(`ACQUIRED ${LOCK} for ${owner}`);
  if (owner.includes("#")) console.log(`TOKEN ${owner}: use it exactly for refresh and release`);
} else if (command === "takeover") {
  if (existsSync(LOCK)) console.log(`[WARN] taking over lock: ${holder()} (${ageMinutes()} min old)`);
  write();
  console.log(`ACQUIRED ${LOCK} for ${owner} (takeover)`);
  if (owner.includes("#")) console.log(`TOKEN ${owner}: use it exactly for refresh and release`);
} else if (command === "refresh") {
  if (fresh() && !holder().startsWith(`${owner} `)) {
    console.error(`LOCKED by ${holder()}; refresh refused. Use your session token exactly (the keeper's continue names it: "Here: lock orchestrator#xxxx").`);
    process.exit(1);
  }
  write();
  console.log(`REFRESHED ${LOCK} for ${owner}`);
} else if (command === "release") {
  if (fresh() && !holder().startsWith(`${owner} `)) {
    console.error(`LOCKED by ${holder()}; release refused for ${owner}.`);
    process.exit(1);
  }
  rmSync(LOCK, { force: true });
  console.log("RELEASED");
} else if (command === "status") {
  console.log(existsSync(LOCK) ? `${fresh() ? "HELD" : "STALE"}: ${holder()} (${ageMinutes()} min)` : "FREE");
} else {
  console.error("Usage: node tools/sprint-lock.mjs acquire|takeover|refresh|release|status [owner]");
  process.exit(2);
}
} // main

if (import.meta.main) {
  main();
}
