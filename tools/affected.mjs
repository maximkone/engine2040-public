// Affected-package gate (OPT7): map a diff to changed workspace packages plus
// their reverse dependencies, so a lane acceptance gate runs that closure
// only, and to the matching tools/*.test.mjs files, so the tool gate runs
// that subset only. The full proof (`node tools/proof.mjs`) still runs once
// per wave.
// Idea port of lightsofapollo/cargo-affect@3136b757736e2164c8d4bb22bef0d0cb00a60e4a
// (MIT, https://github.com/lightsofapollo/cargo-affect) -- reverse-dep closure
// idea only, reimplemented here, zero donor bytes.
// PORT(lightsofapollo/cargo-affect@3136b757: reverse-dep closure over the resolve graph).
// Measured 2026-10-17: crates/combat/src/fight.rs -> closure 20 of 36 (55.6%),
// dependents only; crates/core/src/lib.rs -> whole workspace, closure 36 of 36
// (100.0%). Recorder only: no behavior change, no gate flag change.
// `node --test tools/affected.test.mjs` -> 24 pass / 0 fail.
//
//   node tools/affected.mjs [--base <ref>] [--files <paths...>] [--json] [--mark-green]
//
// With --files the mapping is computed for those repo-relative paths (used
// for seeded proof). Otherwise the changed files are `git diff --name-only
// <base>` (default HEAD, worktree included) plus untracked files from
// `git status --porcelain`.
//
// Green-skip: main() always compares the current diff against the last green
// record and prints SKIP when identical (same files, same closure). Run with
// `--mark-green` only after the lane gate actually went green; that writes
// the record readers compare against.
//
// Edges come from `cargo metadata` (declared path dependencies, including
// optional ones, so the closure never under-covers a feature lane) unioned
// with the default resolved graph. `cargo metadata` is a read-only query:
// it takes no `-j` flag, spawns one process and writes nothing to target/.
// The emitted lane test command always carries explicit `-j 2` and never
// `--release` (owner cargo discipline).
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { existsSync, readdirSync, readFileSync, statSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, relative, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = process.cwd();

const usage = () => {
  console.error("Usage: node tools/affected.mjs [--base <ref>] [--files <paths...>] [--json] [--mark-green]");
  console.error("  Maps the diff to workspace packages + reverse deps (cargo metadata).");
  console.error("  Lane gate runs the closure; the full proof stays once per wave.");
  console.error("  --mark-green records this diff as green (run only after a green gate).");
  process.exit(2);
};

// Files that invalidate the build graph itself: any lane could be affected.
// tools/ scripts are dev gates, never shipped dependencies: a tools/ edit
// gates only its own tool test (affectedToolTests), never the crate closure.
const WORKSPACE_WIDE = ["Cargo.toml", "Cargo.lock", ".cargo/config.toml", "deny.toml"];
// Non-crate dirs owned by a crate per the SPRINT lanes table.
const DIR_OWNERS = new Map([
  ["tests/", "engine-proof"],
  ["fixtures/", "engine-proof"],
  ["content/", "engine-assets"],
]);

const toPosix = (p) => p.split(sep).join("/");
const isWorkspaceWide = (f) => WORKSPACE_WIDE.some((w) => f === w || f.startsWith(w));

function parseArgs(argv) {
  const out = { base: "HEAD", files: null, json: false, markGreen: false, tiers: false, snapshotWrite: null, snapshotDiff: null };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (a === "--base") out.base = argv[++i] ?? usage();
    else if (a.startsWith("--base=")) out.base = a.slice("--base=".length);
    else if (a === "--files") {
      out.files = [];
      while (i + 1 < argv.length && !argv[i + 1].startsWith("--")) out.files.push(argv[++i]);
    } else if (a === "--json") out.json = true;
    else if (a === "--mark-green") out.markGreen = true;
    else if (a === "--tiers") out.tiers = true;
    else if (a === "--snapshot-write") out.snapshotWrite = argv[++i] ?? usage();
    else if (a.startsWith("--snapshot-write=")) out.snapshotWrite = a.slice("--snapshot-write=".length);
    else if (a === "--snapshot-diff") out.snapshotDiff = argv[++i] ?? usage();
    else if (a.startsWith("--snapshot-diff=")) out.snapshotDiff = a.slice("--snapshot-diff=".length);
    else usage();
  }
  return out;
}
// Exported for tests (pure for valid argv; invalid argv exits via usage, as the CLI does).
export { parseArgs };

const GIT_BUFFER_BYTES = 16 * 1024 * 1024;
function git(args) {
  const r = spawnSync("git", args, { encoding: "utf8", cwd: ROOT, maxBuffer: GIT_BUFFER_BYTES });
  if (r.error || r.status !== 0) throw new Error(`git ${args.join(" ")} failed: ${r.error?.message ?? r.stderr ?? r.status}`);
  return (r.stdout ?? "").split("\n").map((l) => l.trim()).filter(Boolean);
}

function changedFromGit(base) {
  // `git diff <base>` compares base to the worktree for tracked files;
  // untracked files never appear in a diff, so add them from status.
  const tracked = new Set(git(["diff", "--name-only", base]));
  for (const line of git(["status", "--porcelain"])) {
    const m = line.match(/^\?\?\s+(.*)$/);
    if (m) tracked.add(m[1].replace(/^"|"$/g, ""));
  }
  return [...tracked].map((f) => toPosix(f)).sort();
}

function loadMetadata() {
  const started = Date.now();
  const fingerprint = manifestFingerprint(ROOT);
  if (fingerprint !== null) {
    const cached = readCachedMetadata(ROOT, fingerprint);
    if (cached !== null) return { metadata: cached, metadataMs: Date.now() - started, metadataCached: true };
  }
  const r = spawnSync("cargo", ["metadata", "--locked", "--format-version", "1"], {
    encoding: "utf8",
    cwd: ROOT,
    maxBuffer: 64 * 1024 * 1024,
    timeout: 300_000,
  });
  const ms = Date.now() - started;
  if (r.error || r.status !== 0) {
    throw new Error(`cargo metadata failed after ${ms} ms: ${r.error?.message ?? r.stderr ?? "exit " + r.status}`);
  }
  const metadata = JSON.parse(r.stdout);
  if (fingerprint !== null) writeCachedMetadata(ROOT, fingerprint, r.stdout);
  return { metadata, metadataMs: ms, metadataCached: false };
}

// Metadata cache (OPT7 speed fix): `cargo metadata` (~0.5-2.5 s) is the
// slow leg of every crate-file gate run, but its output only changes when a
// manifest does. The fingerprint covers every manifest that shapes the graph
// (root Cargo.toml/Cargo.lock plus each member Cargo.toml); a hit skips the
// subprocess. The cache lives in the OS temp dir (never in the repo, so no
// lane can mistake it for a source), keyed by workspace root, and every
// read/write is best-effort: any failure falls back to `cargo metadata`.
// Pure except filesystem reads; exported for tests.
export function manifestFingerprint(root) {
  try {
    const manifests = [join(root, "Cargo.toml"), join(root, "Cargo.lock"), ...findManifests(join(root, "crates"))];
    const h = createHash("sha256");
    for (const m of manifests) {
      const st = statSync(m);
      h.update(`${toPosix(relative(root, m))}:${st.size}:${st.mtimeMs}\n`);
    }
    return h.digest("hex");
  } catch {
    return null; // unreadable tree: caller runs cargo metadata directly
  }
}

function findManifests(dir) {
  // Member manifests live one level under crates/, but walk deeper so a
  // future nested member still invalidates the cache (readdirSync walk).
  let out = [];
  let entries;
  try {
    entries = readdirSync(dir, { withFileTypes: true });
  } catch {
    return out;
  }
  for (const e of entries) {
    const full = join(dir, e.name);
    if (e.isDirectory()) out = out.concat(findManifests(full));
    else if (e.isFile() && e.name === "Cargo.toml") out.push(full);
  }
  return out.sort();
}

export function metadataCachePath(root) {
  const key = createHash("sha256").update(toPosix(resolve(root))).digest("hex").slice(0, 16);
  return join(tmpdir(), `engine2040-affected-${key}.json`);
}

export function readCachedMetadata(root, fingerprint) {
  try {
    const raw = JSON.parse(readFileSync(metadataCachePath(root), "utf8"));
    if (raw?.fingerprint === fingerprint && raw?.metadata) return raw.metadata;
    return null;
  } catch {
    return null;
  }
}

export function writeCachedMetadata(root, fingerprint, metadataJson) {
  try {
    writeFileSync(metadataCachePath(root), JSON.stringify({ fingerprint, metadata: JSON.parse(metadataJson) }));
  } catch { /* best-effort: the next run just re-runs cargo metadata */ }
}

export function computeAffected(changedFiles, metadata, workspaceRoot) {
  const members = new Set(metadata.workspace_members);
  const byId = new Map(metadata.packages.map((p) => [p.id, p]));
  const byName = new Map();
  const dirs = []; // [dirPrefix, packageName], longest prefix wins
  for (const pkg of metadata.packages) {
    if (!members.has(pkg.id)) continue;
    byName.set(pkg.name, pkg);
    const rel = toPosix(relative(workspaceRoot, dirname(resolve(pkg.manifest_path))));
    if (rel === "" || rel === ".") continue; // root package: matched explicitly, never by prefix
    dirs.push([`${rel}/`, pkg.name]);
  }
  dirs.sort((a, b) => b[0].length - a[0].length);

  // Reverse edges: dependent -> dependency, from declared path deps
  // (covers optional/feature deps) unioned with the default resolve graph.
  // Forward edges mirror them: package -> its direct path/build inputs, so
  // the required tier (what must build to test the affected closure) is the
  // affected set closed over forward deps. Idea from tekian/cargo-delta
  // (MIT) crates/delta-lib/src/cargo.rs impact tiers, reimplemented here.
  const reverse = new Map(); // depName -> Set(dependentName)
  const forward = new Map(); // packageName -> Set(depName)
  const addEdge = (dependent, dep) => {
    if (!byName.has(dependent) || !byName.has(dep) || dependent === dep) return;
    if (!reverse.has(dep)) reverse.set(dep, new Set());
    reverse.get(dep).add(dependent);
    if (!forward.has(dependent)) forward.set(dependent, new Set());
    forward.get(dependent).add(dep);
  };
  for (const pkg of metadata.packages) {
    if (!members.has(pkg.id)) continue;
    for (const dep of pkg.dependencies) {
      if (typeof dep.path === "string" && dep.path !== "" && byName.has(dep.name)) addEdge(pkg.name, dep.name);
    }
  }
  for (const node of metadata.resolve?.nodes ?? []) {
    const pkg = byId.get(node.pkg ?? node.id);
    if (!pkg || !members.has(pkg.id)) continue;
    for (const dep of node.deps ?? []) {
      const target = byId.get(dep.pkg);
      if (target && members.has(target.id)) addEdge(pkg.name, target.name);
    }
  }

  let workspaceWide = false;
  const changed = new Set();
  const unmapped = [];
  for (const file of changedFiles) {
    if (isWorkspaceWide(file)) {
      workspaceWide = true;
      continue;
    }
    // tools/ scripts are dev gates with their own tool-test closure
    // (affectedToolTests): they never join the crate closure nor unmapped.
    if (file === "tools" || file.startsWith("tools/")) continue;
    let owner = null;
    for (const [prefix, name] of dirs) {
      if (file === prefix.slice(0, -1) || file.startsWith(prefix)) {
        owner = name;
        break;
      }
    }
    if (!owner) {
      for (const [prefix, name] of DIR_OWNERS) {
        if (file.startsWith(prefix)) {
          owner = name;
          break;
        }
      }
    }
    if (owner) changed.add(owner);
    else unmapped.push(file);
  }

  const all = [...byName.keys()].sort();
  // engine-core is the workspace foundation (ARCHITECTURE.md bottom layer;
  // workspace-check UNIVERSAL_EDGES lets any crate use it without a declared
  // edge, so undeclared future uses are not auditable): a core edit resolves
  // all, the only sound closure for the bottom layer.
  // Required tier: the affected seeds plus their direct (depth-1) build
  // inputs, so the lane gate names what must be present to build the
  // affected tests. Depth-1, not transitive: the transitive closure over
  // this workspace explodes to all 35 packages (everything funnels through
  // engine-core plus the game/editor hubs), which would make the tier
  // useless as a selection. Local closure over the forward map above
  // (tested via `required`).
  const closeForward = (seeds) => {
    const out = new Set(seeds);
    for (const name of seeds) {
      for (const dep of forward.get(name) ?? []) out.add(dep);
    }
    return [...out].sort();
  };
  if (workspaceWide || changed.has("engine-core")) {
    return {
      changedPackages: [...changed].sort(),
      affected: all,
      required: all,
      total: all.length,
      workspaceWide,
      coreFoundation: changed.has("engine-core"),
      unmapped,
    };
  }
  // Closure over reverse deps: a change rebuilds/retests everything that
  // (transitively) depends on it.
  const affected = new Set(changed);
  const queue = [...changed];
  while (queue.length > 0) {
    const name = queue.pop();
    for (const dependent of reverse.get(name) ?? []) {
      if (!affected.has(dependent)) {
        affected.add(dependent);
        queue.push(dependent);
      }
    }
  }
  return { changedPackages: [...changed].sort(), affected: [...affected].sort(), required: closeForward(affected), total: all.length, workspaceWide: false, coreFoundation: false, unmapped };
}

export function laneCommand(affected) {
  return `cargo test -j 2 --locked ${affected.map((p) => `-p ${p}`).join(" ")} --all-targets`;
}

// The same lane gate on a GitHub runner (owner 2026-10-17: heavy cargo goes to GitHub, the PC has one build lock).
export function laneCommandOnGithub(affected) {
  return `node tools/gh-check.mjs ${laneCommand(affected)}`;
}

// Tool-test closure (OPT7 speed fix): a tools/ edit gates only its own
// stem test, never all tool tests and never the crate closure. A
// workspace-wide file still gates every tool test; crates-only or
// docs-only diffs gate none. Anything else under tools/ (nested dirs,
// vendor/, archive/, non-.mjs configs and baselines) can feed many gates,
// so it keeps the run-all-tool-tests fallback. Pure for tests; main()
// passes the readdir list so this never touches the filesystem.
export function affectedToolTests(changedFiles, allToolTests) {
  const all = [...allToolTests].sort();
  if (changedFiles.some(isWorkspaceWide)) return all;
  const have = new Set(all);
  const out = new Set();
  let fallback = false;
  for (const file of changedFiles) {
    // Unmappable tools/ change: nested path, non-.mjs file, or the dir
    // itself. A top-level .mjs without a matching test stays below: a new
    // helper with no test gates nothing.
    if (file === "tools" || (file.startsWith("tools/") && !/^tools\/[^/]+\.mjs$/.test(file))) {
      fallback = true;
      continue;
    }
    const m = /^tools\/([^/]+)\.mjs$/.exec(file);
    if (!m) continue;
    const base = m[1];
    if (base.endsWith(".test")) {
      const candidate = `tools/${base}.mjs`;
      if (have.has(candidate)) out.add(candidate);
    } else {
      const candidate = `tools/${base}.test.mjs`;
      if (have.has(candidate)) out.add(candidate);
    }
  }
  if (fallback) return all;
  return [...out].sort();
}

export function toolCommand(affectedTools) {
  return affectedTools.length > 0
    ? `node --test ${affectedTools.join(" ")}`
    : "(no tool changes: tool gate skips tool tests)";
}

// Green-skip (turborepo-hash idea port, idea only, zero donor bytes): when
// the exact same changed-file set plus affected closure already went green,
// the lane gate prints SKIP instead of re-running the same tests. Records
// live in the OS temp dir (never the repo), keyed by workspace root; every
// read/write is best-effort. The record is written only by an explicit
// `--mark-green` run after a real green gate; readers compare only.
// Pure except filesystem; exported for tests.
export function greenRecordPath(root) {
  const key = createHash("sha256").update(toPosix(resolve(root))).digest("hex").slice(0, 16);
  return join(tmpdir(), `engine2040-affected-green-${key}.json`);
}
export function hashChangedFiles(files) {
  return createHash("sha256").update([...files].sort().join("\n")).digest("hex").slice(0, 16);
}
export function readLastGreen(root) {
  try {
    const raw = JSON.parse(readFileSync(greenRecordPath(root), "utf8"));
    if (typeof raw?.filesHash === "string" && typeof raw?.affectedHash === "string" && raw?.verdict === "green") return raw;
    return null;
  } catch {
    return null;
  }
}
export function writeLastGreen(root, { filesHash, affectedHash }) {
  try {
    writeFileSync(greenRecordPath(root), JSON.stringify({ filesHash, affectedHash, verdict: "green" }));
    return true;
  } catch {
    return false; // best-effort: the next run just re-runs the gate
  }
}
export function shouldSkip(changedFiles, affected, last) {
  if (!last || last.verdict !== "green") return { skip: false };
  const filesHash = hashChangedFiles(changedFiles);
  const affectedHash = hashChangedFiles(affected);
  if (last.filesHash === filesHash && last.affectedHash === affectedHash) return { skip: true, filesHash };
  return { skip: false };
}

// Snapshot tiers (tekian/cargo-delta idea port, MIT, idea only, zero donor
// bytes): a JSON snapshot of the tier decision (modified/affected/required)
// that the landing gate diffs run-over-run. cargo-delta
// crates/delta-lib/src/snapshot.rs persists workspace state; here the record
// is the tier triple plus the changed-file list, written explicitly with
// --snapshot-write and compared with --snapshot-diff. Pure; exported.
export function buildSnapshot({ base, changedFiles, tiers, totalWorkspacePackages }) {
  return {
    v: 1,
    tool: "affected",
    base,
    changedFiles: [...changedFiles].sort(),
    tiers: {
      modified: [...tiers.modified].sort(),
      affected: [...tiers.affected].sort(),
      required: [...tiers.required].sort(),
    },
    totalWorkspacePackages,
  };
}
export function diffSnapshots(oldSnap, newSnap) {
  const oldFiles = new Set(oldSnap?.changedFiles ?? []);
  const newFiles = new Set(newSnap?.changedFiles ?? []);
  const addedFiles = [...newFiles].filter((f) => !oldFiles.has(f)).sort();
  const removedFiles = [...oldFiles].filter((f) => !newFiles.has(f)).sort();
  const oldAff = new Set(oldSnap?.tiers?.affected ?? []);
  const newAff = new Set(newSnap?.tiers?.affected ?? []);
  return {
    addedFiles,
    removedFiles,
    addedAffected: [...newAff].filter((a) => !oldAff.has(a)).sort(),
    removedAffected: [...oldAff].filter((a) => !newAff.has(a)).sort(),
    same: addedFiles.length === 0 && removedFiles.length === 0,
  };
}

// Speed fast path: tools/ edits never join the crate closure and unmapped
// files (docs/, team/, etc.) map to no crate, so cargo metadata (~1 s) is
// skipped when no file can own a crate. Total comes from the crates/ dir
// count plus the root engine2040 package. Same output shape, metadataMs 0.
// Local (not exported): no new public item, no extra caller needed.
const CRATE_PREFIXES = ["crates/", "tests/", "fixtures/", "content/"];
function needsCrateMetadata(changedFiles) {
  return changedFiles.some((f) => isWorkspaceWide(f) || CRATE_PREFIXES.some((p) => f.startsWith(p)));
}
function countWorkspacePackages() {
  try {
    let crates = 0;
    for (const name of readdirSync("crates")) {
      try {
        if (existsSync(`crates/${name}/Cargo.toml`)) crates++;
      } catch { /* ignore one entry */ }
    }
    return crates + 1; // +1: the root engine2040 package beside crates/*
  } catch {
    return null; // crates/ unreadable: fall back to cargo metadata
  }
}

function main() {
  const { base, files, json, markGreen, tiers, snapshotWrite, snapshotDiff } = parseArgs(process.argv.slice(2));
  const changedFiles = files ? files.map(toPosix).sort() : changedFromGit(base);
  let changedPackages = [];
  let affected = [];
  let required = [];
  let total = 0;
  let workspaceWide;
  let coreFoundation;
  let unmapped;
  let metadataMs;
  let metadataSkipped = false;
  let metadataCached = false;
  if (!needsCrateMetadata(changedFiles)) {
    const counted = countWorkspacePackages();
    if (counted !== null) {
      changedPackages = [];
      affected = [];
      required = [];
      total = counted;
      workspaceWide = false;
      coreFoundation = false;
      unmapped = changedFiles.filter((f) => !(f === "tools" || f.startsWith("tools/")));
      metadataMs = 0;
      metadataSkipped = true;
    }
  }
  if (!metadataSkipped) {
    const loaded = loadMetadata();
    const root = toPosix(loaded.metadata.workspace_root);
    metadataMs = loaded.metadataMs;
    metadataCached = loaded.metadataCached;
    ({ changedPackages, affected, required, total, workspaceWide, coreFoundation, unmapped } = computeAffected(changedFiles, loaded.metadata, root));
  }
  const fraction = total === 0 ? 0 : affected.length / total;
  // Same glob the proof gate runs (tools/*.test.mjs), so the tool gate
  // names a subset of exactly what the full gate would run.
  let allToolTests = [];
  try {
    allToolTests = readdirSync("tools").filter((f) => f.endsWith(".test.mjs")).map((f) => `tools/${f}`).sort();
  } catch (err) {
    console.error(`affected: failed to list tools/*.test.mjs: ${err?.message ?? err}`);
    allToolTests = [];
  }
  const affectedTools = affectedToolTests(changedFiles, allToolTests);
  const tiersOut = { modified: changedPackages, affected, required };
  const result = {
    changedFiles,
    changedPackages,
    affected,
    tiers: tiersOut,
    totalWorkspacePackages: total,
    workspaceWide,
    coreFoundation: Boolean(coreFoundation),
    unmapped,
    metadataMs,
    metadataCached,
    projectedLaneFraction: Number(fraction.toFixed(3)),
    laneGate: affected.length > 0 ? laneCommand(affected) : "(no crate changes: lane gate skips crate tests)",
    affectedTools,
    totalToolTests: allToolTests.length,
    toolGate: toolCommand(affectedTools),
    waveGate: "node tools/proof.mjs (full proof stays once per wave)",
  };
  // Green-skip verdict (best-effort read; a missing/corrupt record is RUN).
  const green = shouldSkip(changedFiles, affected, readLastGreen(ROOT));
  result.greenSkip = green.skip;
  if (markGreen) {
    const wrote = writeLastGreen(ROOT, {
      filesHash: hashChangedFiles(changedFiles),
      affectedHash: hashChangedFiles(affected),
    });
    result.markedGreen = wrote;
  }
  const snapshot = buildSnapshot({ base, changedFiles, tiers: tiersOut, totalWorkspacePackages: total });
  if (snapshotWrite) {
    try {
      writeFileSync(snapshotWrite, `${JSON.stringify(snapshot, null, 2)}\n`);
      result.snapshotWrote = snapshotWrite;
    } catch (err) {
      console.error(`affected: snapshot write failed: ${err?.message ?? err}`);
      process.exit(1);
    }
  }
  if (snapshotDiff) {
    let oldSnap = null;
    try {
      oldSnap = JSON.parse(readFileSync(snapshotDiff, "utf8"));
    } catch (err) {
      console.error(`affected: snapshot read failed: ${err?.message ?? err}`);
      process.exit(1);
    }
    result.snapshotDiff = diffSnapshots(oldSnap, snapshot);
  }
  if (tiers && !json) {
    console.log(`tiers: modified(${tiersOut.modified.length}) affected(${tiersOut.affected.length}) required(${tiersOut.required.length})`);
    console.log(`  modified: ${tiersOut.modified.join(" ") || "(none)"}`);
    console.log(`  affected: ${tiersOut.affected.join(" ") || "(none)"}`);
    console.log(`  required: ${tiersOut.required.join(" ") || "(none)"}`);
  }
  if (json) {
    console.log(JSON.stringify(result, null, 2));
    return;
  }
  console.log(metadataSkipped
    ? "AFFECTED engine2040 (cargo metadata skipped: no crate files)"
    : metadataCached
      ? `AFFECTED engine2040 (cargo metadata cached, read in ${metadataMs} ms)`
      : `AFFECTED engine2040 (cargo metadata in ${metadataMs} ms)`);
  console.log(`changed files (${changedFiles.length}):`);
  for (const f of changedFiles) console.log(`  ${f}`);
  if (workspaceWide) console.log("workspace-wide file touched: closure is the whole workspace");
  if (coreFoundation) console.log("engine-core is the foundation layer: closure is the whole workspace");
  console.log(`changed packages (${changedPackages.length}): ${changedPackages.join(" ") || "(none)"}`);
  console.log(`affected closure (${affected.length} of ${total}, projected lane fraction ${(fraction * 100).toFixed(1)}%):`);
  console.log(`  ${affected.join(" ") || "(none)"}`);
  if (unmapped.length > 0) console.log(`unmapped files (no crate gate): ${unmapped.join(" ")}`);
  console.log(`lane gate (closure only, dev, -j 2, no --release):\n  ${result.laneGate}`);
  if (affected.length > 0) console.log(`  on GitHub, no local build lock (AGENTS.md Fast paths):\n  ${laneCommandOnGithub(affected)}`);
  console.log(`required tier (affected plus build inputs, ${tiersOut.required.length} of ${total}):\n  ${tiersOut.required.join(" ") || "(none)"}`);
  console.log(green.skip
    ? `green-skip: SKIP (identical to last green run, files hash ${green.filesHash})`
    : "green-skip: RUN (no matching green record)");
  if (markGreen) console.log(`green-skip: marked this diff green (${result.markedGreen ? "record written" : "write failed, best-effort"})`);
  console.log(`tool gate (${affectedTools.length} of ${allToolTests.length} tool tests):\n  ${result.toolGate}`);
  console.log(`wave gate (full proof, once per wave):\n  ${result.waveGate}`);
  console.log(`metadata wall time: ${metadataMs} ms${metadataSkipped ? " (skipped)" : metadataCached ? " (cache hit)" : ""}`);
}

if (import.meta.main) {
  try {
    main();
  } catch (error) {
    console.error(`affected: ${error.message}`);
    process.exit(1);
  }
}
