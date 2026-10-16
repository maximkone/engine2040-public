// Project doctor: finds what breaks development and repairs the safe parts.
//   node tools/doctor.mjs                 report only, never mutates
//   node tools/doctor.mjs --fix           report what --fix WOULD do, change nothing
//   node tools/doctor.mjs --fix --consent <csv>
//       apply only the consented scopes: toolchain, deny, lock, cache
// Exit 0 when healthy or healed; 1 when something still needs a person.
//
// Safety law (board DOC1): report-only by default; a live (fresh) lease is
// never released; lane evidence (target/lanes/*.patch) is never deleted;
// installs and cache eviction need explicit --consent; unknown probes fail
// honestly instead of reporting OK. Drills live in tools/doctor.test.mjs and
// run against fixture trees (E2040_DOCTOR_ROOT), with simulated disk space
// (E2040_DOCTOR_FREE_GB) so low-disk paths are drillable without filling a disk.
import { spawnSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, readdirSync, rmSync, statfsSync, statSync, writeFileSync } from "node:fs";
import { join } from "node:path";

const argv = process.argv.slice(2);
const fix = argv.includes("--fix");
const consentArg = argv.find((a) => a.startsWith("--consent="));
const consent = new Set(
  (consentArg ? consentArg.slice("--consent=".length) : "").split(",").map((s) => s.trim()).filter(Boolean),
);
// A fixture root lets drills run the doctor against a fake tree.
const ROOT = process.env.E2040_DOCTOR_ROOT ?? process.cwd();
const at = (p) => join(ROOT, p);
if (argv.includes("--help") || argv.includes("-h")) {
  console.log("Usage: node tools/doctor.mjs [--fix] [--consent=toolchain,deny,lock,cache]");
  process.exit(0);
}
const UNKNOWN = new Set(argv.filter((a) => a !== "--fix" && !a.startsWith("--consent=")));
if (UNKNOWN.size) {
  console.error("Usage: node tools/doctor.mjs [--fix] [--consent=toolchain,deny,lock,cache]");
  process.exit(2);
}
const KNOWN_SCOPES = new Set(["toolchain", "deny", "lock", "cache"]);
for (const scope of consent) {
  if (!KNOWN_SCOPES.has(scope)) {
    console.error(`unknown --consent scope "${scope}"; known: toolchain,deny,lock,cache`);
    process.exit(2);
  }
}
if (consent.size && !fix) {
  console.error("--consent needs --fix; without --fix the doctor only reports");
  process.exit(2);
}

let needsYou = 0;
const say = (tag, text) => console.log(`[${tag}] ${text}`);
const run = (cmd, args, timeout = 600_000, cwd = ROOT) => {
  const r = spawnSync(cmd, args, { encoding: "utf8", timeout, cwd, maxBuffer: 32 * 1024 * 1024 });
  return { ok: !r.error && r.status === 0, out: `${r.stdout ?? ""}${r.stderr ?? ""}`.trim() };
};
const problem = (text, remedy) => {
  needsYou += 1;
  say("NEEDS YOU", `${text} — ${remedy}`);
};
// A repair that lacks consent is reported as a WOULD line and counts as
// needing the operator, so bare --fix never mutates and never exits 0 dirty.
const gated = (scope, text, remedy, apply) => {
  if (!fix) {
    problem(text, remedy);
    return;
  }
  if (!consent.has(scope)) {
    needsYou += 1;
    say("WOULD", `${text} — rerun with --consent=${scope} to apply: ${remedy}`);
    return;
  }
  if (apply()) return;
  problem(text, remedy);
};

// 1. Rust toolchain from rust-toolchain.toml, with rustfmt and clippy.
// Probes are read-only (version queries). Installs need consent.
let channel = "unknown";
try {
  channel = readFileSync(at("rust-toolchain.toml"), "utf8").match(/channel\s*=\s*"([^"]+)"/)?.[1] ?? "unknown";
} catch {
  problem("rust-toolchain.toml is unreadable", "restore it before installing anything");
}
const toolchainOk = () => run("cargo", ["fmt", "--version"]).ok && run("cargo", ["clippy", "--version"]).ok;
if (toolchainOk()) {
  say("OK", `Rust toolchain ${channel} with rustfmt and clippy`);
} else {
  gated("toolchain", `Rust toolchain ${channel} or its components are missing`, "install it", () => {
    if (run("rustup", ["toolchain", "install"]).ok && toolchainOk()) {
      say("FIXED", `installed Rust toolchain ${channel} from rust-toolchain.toml`);
      return true;
    }
    return false;
  });
}

// 2. Node for the gate scripts (probe only; installing Node is always manual).
const nodeMajor = Number(process.versions.node.split(".")[0]);
if (nodeMajor >= 20) say("OK", `Node ${process.versions.node}`);
else problem(`Node ${process.versions.node} is too old`, "install Node 20 or newer");

// 3. cargo-deny, the license gate inside tools/proof.mjs. Install needs consent.
if (run("cargo", ["deny", "--version"]).ok) {
  say("OK", "cargo-deny installed");
} else {
  gated("deny", "cargo-deny is missing, so the license gate is skipped", "install it", () => {
    if (run("cargo", ["install", "cargo-deny@0.18.7", "--locked"], 1_800_000).ok) {
      say("FIXED", "installed cargo-deny 0.18.7");
      return true;
    }
    return false;
  });
}

// 4. Sprint lock: a live lease is NEVER released, with or without consent.
// A stale lock (no refresh for 3 h, i.e. a crashed session) is released only
// with --consent=lock, and the previous holder is named in the receipt.
const lockOut = run(process.execPath, [at("tools/sprint-lock.mjs"), "status"]).out;
if (lockOut.startsWith("HELD") || lockOut.startsWith("FREE")) {
  say("OK", `sprint lock ${lockOut}`);
} else if (lockOut.startsWith("STALE")) {
  gated("lock", `stale sprint lock: ${lockOut}`, "release it", () => {
    run(process.execPath, [at("tools/sprint-lock.mjs"), "release"]);
    say("FIXED", `released stale sprint lock (was: ${lockOut})`);
    return true;
  });
} else {
  problem(`sprint lock probe returned unrecognized output: ${lockOut || "(empty)"}`, "inspect target/sprint.lock by hand");
}

// 5. Lockfile and workspace rules (read-only probes).
// Speed: cargo metadata (up to ~10 s cold) is a pure function of the
// manifests plus the lockfile, so a verified-OK result is memoized in
// target/ (gitignored) keyed by those fingerprints. Any manifest, lockfile
// or toolchain edit misses and re-runs cargo; failures are never cached,
// so the report is identical with or without the cache. Fixture drills
// (E2040_DOCTOR_ROOT) miss and behave exactly as before.
const LOCK_CACHE = "doctor.lock-cache.json";
const lockFingerprint = () => {
  const fps = [];
  const candidates = ["Cargo.toml", "Cargo.lock", "rust-toolchain.toml", join(".cargo", "config.toml")];
  try {
    for (const dir of readdirSync(at("crates"))) candidates.push(join("crates", dir, "Cargo.toml"));
  } catch { /* no crates dir: fingerprint misses, cargo re-runs */ }
  for (const p of candidates) {
    try {
      const st = statSync(at(p));
      fps.push(p + ":" + st.mtimeMs + ":" + st.size);
    } catch { fps.push(p + ":missing"); }
  }
  return channel + "\n" + fps.join("\n");
};
const lockCacheHit = () => {
  try {
    const cached = JSON.parse(readFileSync(at(join("target", LOCK_CACHE)), "utf8"));
    if (cached && cached.fp === lockFingerprint() && cached.ok === true) return true;
  } catch { /* missing or corrupt: run cargo below */ }
  return false;
};
let metadata;
if (lockCacheHit()) {
  metadata = { ok: true, out: "" };
} else {
  metadata = run("cargo", ["metadata", "--locked", "--format-version", "1", "--no-deps"]);
  if (metadata.ok) {
    try {
      mkdirSync(at("target"), { recursive: true });
      writeFileSync(at(join("target", LOCK_CACHE)), JSON.stringify({ fp: lockFingerprint(), ok: true }));
    } catch (e) { console.warn(`warning: lock cache write skipped (best-effort): ${e?.message ?? e}`); }
  }
}
if (metadata.ok) say("OK", "Cargo.lock matches the manifests");
else problem("Cargo.lock does not match the manifests", "an orchestrator should run `cargo update -w`, review the diff and run the gate");
const audit = run(process.execPath, [at("tools/workspace-check.mjs")]);
if (audit.ok) say("OK", "workspace audit (metadata, edges, modules, board)");
else problem(`workspace audit failed: ${audit.out.split("\n").find((l) => l.includes("FAIL")) ?? audit.out}`, "fix the named file, then rerun");

// 6. Disk: shared build pools (target/pool-*) are caches; lane evidence
// (target/lanes/*.patch) is never touched. Eviction needs explicit consent,
// a low disk, AND zero registered lane worktrees (target/lanes-worktrees.json
// empty), so a live wave can never lose its build cache mid-flight.
const pools = [1, 2, 3, 4].map((k) => join("target", `pool-${k}`)).filter((d) => {
  try {
    return statSync(at(d)).isDirectory();
  } catch {
    return false;
  }
});
let lanesRegistered = -1;
try {
  lanesRegistered = Object.keys(JSON.parse(readFileSync(at(join("target", "lanes-worktrees.json")), "utf8")).worktrees ?? {}).length;
} catch {
  lanesRegistered = -1; // unknown: fail honestly below, never evict
}
const freeGb = (() => {
  const fake = Number(process.env.E2040_DOCTOR_FREE_GB);
  if (Number.isFinite(fake)) return fake; // drills only
  try {
    const s = statfsSync(at("."));
    return (s.bavail * s.bsize) / 1e9;
  } catch {
    return NaN;
  }
})();
if (Number.isNaN(freeGb)) {
  problem("disk free space is unknown on this machine", "free disk space by hand; no caches were touched");
} else if (freeGb >= 10) {
  say("OK", `disk: ${freeGb.toFixed(0)} GB free, ${pools.length} build pool(s), lane evidence untouched`);
} else if (lanesRegistered !== 0) {
  problem(
    `only ${freeGb.toFixed(1)} GB free with ${lanesRegistered < 0 ? "unknown" : lanesRegistered} lane(s) registered`,
    "free disk space by hand; pools are kept while any lane may be building",
  );
} else {
  gated("cache", `only ${freeGb.toFixed(1)} GB free, no lanes registered`, "evict own build pools", () => {
    for (const dir of pools) rmSync(at(dir), { recursive: true, force: true });
    say("FIXED", `disk was low (${freeGb.toFixed(1)} GB); evicted ${pools.length} own build pool(s), lane evidence kept`);
    return true;
  });
}

// 7. Git state worth knowing before a sprint (read-only).
const status = run("git", ["status", "--porcelain"]);
const dirty = status.out ? status.out.split("\n").length : 0;
say(dirty ? "INFO" : "OK", dirty ? `${dirty} uncommitted paths (a sprint commits green work first)` : "working tree clean");

console.log(needsYou ? `RESULT ${needsYou} item(s) need you` : "RESULT healthy");
process.exitCode = needsYou ? 1 : 0;
