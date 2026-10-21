// fix: apply the toolchain's mechanical fixes to the paths one agent owns (AF1).
//
//   node tools/fix.mjs -p <crate> --only <path>...
//   node tools/fix.mjs -p <crate> --only <path>... --dry-run
//
// -p takes the crate dir name (crates/core) or its package name (engine-core);
// cargo gets the package name. --only takes the owned paths and may repeat.
// --dry-run prints the plan and changes nothing.
//
// Steps:
//   1. snapshot every tracked and dirty file of the crate (git ls-files
//      --cached --others), the owned paths and the workspace Cargo.lock and
//      Cargo.toml, byte for byte;
//   2. cargo fix --allow-dirty --lib -p <package>;
//   3. cargo clippy --fix --allow-dirty --lib -p <package>;
//   4. rustfmt --edition 2021 on the --only files;
//   5. restore byte for byte every file outside --only that a fixer touched, so
//      another session's work in the same tree stays untouched;
//   6. print `fix: <n> owned files changed, <m> foreign files restored`.
//
// The tree root is the nearest ancestor of the cwd that holds a crates dir.
// Exit 0 when every step ran, 1 when a fixer failed, 2 on a usage error.
import { spawnSync } from "node:child_process";
import { existsSync, readFileSync, readdirSync, rmSync, statSync, writeFileSync } from "node:fs";
import { dirname, join, relative, resolve } from "node:path";

const USAGE = "usage: node tools/fix.mjs -p <crate> --only <path>...";
const MAX = 64 * 1024 * 1024;

// Windows paths compare case-insensitively, so the snapshot keys must too.
const key = (p) => (process.platform === "win32" ? p.toLowerCase() : p);

const fail = (message, code) => {
  console.error(`fix: error: ${message}`);
  if (code === 2) console.error(USAGE);
  process.exit(code);
};

const parseArgs = (argv) => {
  let crate = "";
  let dryRun = false;
  const only = [];
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (a === "-p" || a === "--package") crate = argv[++i] ?? "";
    else if (a === "--only") {
      while (i + 1 < argv.length && !argv[i + 1].startsWith("-")) only.push(argv[++i]);
    } else if (a === "--dry-run") dryRun = true;
    else if (a === "--help" || a === "-h") {
      console.log(USAGE);
      process.exit(0);
    } else fail(`unknown argument ${a}`, 2);
  }
  if (!crate) fail("-p <crate> is required", 2);
  if (only.length === 0) fail("--only <path>... is required", 2);
  return { crate, only, dryRun };
};

// The tree root: the nearest ancestor of the cwd that holds a crates dir.
const findRoot = (start) => {
  let dir = resolve(start);
  for (;;) {
    if (existsSync(join(dir, "crates"))) return dir;
    const up = dirname(dir);
    if (up === dir) return resolve(start);
    dir = up;
  }
};

// The package name out of the [package] table of a Cargo.toml.
const packageName = (toml) => {
  let inPackage = false;
  for (const line of toml.split(/\r?\n/)) {
    const head = /^\[([^\]]+)\]/.exec(line.trim());
    if (head) {
      inPackage = head[1].trim() === "package";
      continue;
    }
    if (inPackage) {
      const name = /^name\s*=\s*"([^"]+)"/.exec(line.trim());
      if (name) return name[1];
    }
  }
  return "";
};

const findCrate = (root, wanted) => {
  const crates = join(root, "crates");
  if (!existsSync(crates)) return null;
  for (const dir of readdirSync(crates).sort()) {
    const manifest = join(crates, dir, "Cargo.toml");
    if (!existsSync(manifest)) continue;
    const name = packageName(readFileSync(manifest, "utf8"));
    if (dir === wanted || name === wanted) return { dir: join(crates, dir), name: name || dir };
  }
  return null;
};

// Every file of the crate dir, used when the tree has no VCS of its own.
const walk = (dir, out = []) => {
  let entries = [];
  try {
    entries = readdirSync(dir, { withFileTypes: true });
  } catch {
    return out;
  }
  for (const entry of entries) {
    if (entry.name === "target" || entry.name.startsWith(".")) continue;
    const path = join(dir, entry.name);
    if (entry.isDirectory()) walk(path, out);
    else if (entry.isFile()) out.push(path);
  }
  return out;
};

// Every tracked and dirty file of the crate: git knows the tracked ones and the
// dirty new ones. Without a VCS we walk the crate dir instead.
const crateFiles = (root, crateDir) => {
  const top = spawnSync("git", ["-C", root, "rev-parse", "--show-toplevel"], { encoding: "utf8" });
  if (top.status === 0 && resolve(top.stdout.trim()) === resolve(root)) {
    const rel = relative(root, crateDir).split("\\").join("/");
    const list = spawnSync(
      "git",
      ["-C", root, "ls-files", "-z", "--cached", "--others", "--exclude-standard", "--", rel],
      { encoding: "utf8", maxBuffer: MAX },
    );
    if (list.status === 0) {
      return list.stdout.split("\0").filter(Boolean).map((p) => join(root, ...p.split("/")));
    }
  }
  return walk(crateDir);
};

const snapshot = (files) => {
  const map = new Map();
  for (const abs of files) {
    if (!existsSync(abs) || !statSync(abs).isFile()) continue;
    map.set(key(abs), { abs, buf: readFileSync(abs) });
  }
  return map;
};

const runStep = (name, cmd, args, cwd) => {
  console.log(`fix: running ${name}`);
  const r = spawnSync(cmd, args, { cwd, encoding: "utf8", shell: false, maxBuffer: MAX });
  const out = `${r.stdout ?? ""}${r.stderr ?? ""}`.trim();
  if (out) console.log(out);
  if (r.error) {
    console.error(`fix: ${name} did not run: ${r.error.message}`);
    return false;
  }
  if (r.status !== 0) {
    console.error(`fix: ${name} failed with status ${r.status}`);
    return false;
  }
  return true;
};

const main = () => {
  const { crate, only, dryRun } = parseArgs(process.argv.slice(2));
  const root = findRoot(process.cwd());
  const found = findCrate(root, crate);
  if (!found) fail(`unknown crate ${crate}`, 2);

  const ownedFiles = [];
  for (const p of only) {
    const abs = resolve(root, p);
    if (!existsSync(abs) || !statSync(abs).isFile()) fail(`owned path not found: ${p}`, 2);
    ownedFiles.push(abs);
  }
  const owned = new Set(ownedFiles.map(key));

  // Snapshot the crate's tracked and dirty files, the owned paths and the
  // workspace files cargo may rewrite: the lock and the manifest are not the
  // agent's files.
  const files = [...crateFiles(root, found.dir), ...ownedFiles];
  for (const name of ["Cargo.lock", "Cargo.toml"]) {
    const ws = join(root, name);
    if (existsSync(ws)) files.push(ws);
  }
  const snap = snapshot(files);

  console.log(
    `fix: crate ${relative(root, found.dir)} (${found.name}), ${ownedFiles.length} owned file(s), ${snap.size} file(s) snapshotted`,
  );
  for (const abs of ownedFiles) console.log(`fix: owned ${relative(root, abs)}`);

  const steps = [
    { name: "cargo fix", cmd: "cargo", args: ["fix", "--allow-dirty", "--lib", "-p", found.name] },
    {
      name: "cargo clippy --fix",
      cmd: "cargo",
      args: ["clippy", "--fix", "--allow-dirty", "--lib", "-p", found.name],
    },
    { name: "rustfmt", cmd: "rustfmt", args: ["--edition", "2021", ...ownedFiles] },
  ];

  if (dryRun) {
    for (const step of steps) console.log(`fix: would run ${step.cmd} ${step.args.join(" ")}`);
    console.log("fix: dry run, nothing changed");
    return;
  }

  let failed = "";
  for (const step of steps) {
    if (!runStep(step.name, step.cmd, step.args, root)) {
      failed = step.name;
      break;
    }
  }

  // Restore every file outside --only that a fixer touched, byte for byte.
  let ownedChanged = 0;
  let foreignRestored = 0;
  for (const [k, s] of snap) {
    const now = existsSync(s.abs) ? readFileSync(s.abs) : null;
    const same = now !== null && now.equals(s.buf);
    if (owned.has(k)) {
      if (!same) {
        ownedChanged++;
        if (now === null) writeFileSync(s.abs, s.buf);
      }
      continue;
    }
    if (same) continue;
    writeFileSync(s.abs, s.buf);
    foreignRestored++;
  }

  // The fixers keep their pre-fix copies in .rustfix; that scratch must not
  // stay in a tree that other sessions share.
  for (const dir of [join(found.dir, ".rustfix"), join(root, ".rustfix")]) {
    if (existsSync(dir)) {
      rmSync(dir, { recursive: true, force: true });
      console.log(`fix: removed ${relative(root, dir)} (fixer scratch)`);
    }
  }

  console.log(`fix: ${ownedChanged} owned files changed, ${foreignRestored} foreign files restored`);
  if (failed) {
    console.error(`fix: failed at ${failed}`);
    process.exit(1);
  }
};

if (import.meta.main) main();
