// Audit the workspace before landing: Cargo metadata, the resolved dependency
// graph, module reachability, sim determinism, genre ledger, baseline gate,
// board needs, and new LFS blobs.
import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { existsSync, mkdirSync, readdirSync, readFileSync, statSync, writeFileSync } from "node:fs";
import { basename, dirname, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { SIM_CRATES } from "./rust-quality.mjs";
import { archivedDoneRows, needsOf } from "./board-slice.mjs";
import { loadBoardMemo, parseBoardMemo } from "./board-memo.mjs";
import { breaches as researchBreaches, failLine as researchFailLine } from "./research-cap.mjs";
import { list as presenceList } from "./presence.mjs";
import { deriveFixedApi, genreHits, platformMathHits } from "./sim-source-scan.mjs";

// engine-core is the bottom layer (ARCHITECTURE.md): any crate may use it.
// Without this, packets re-derived its hash, rng and alloc probe instead.
const UNIVERSAL_EDGES = new Set(["engine-core"]);

const allowedEdges = new Map([
  ["engine2040", ["engine-core", "engine-ecs", "engine-render", "engine-physics", "engine-nav", "engine-audio", "engine-script", "engine-net", "engine-assets", "engine-destruction", "engine-ai", "engine-ponytail", "engine-proof"]],
  ["engine-ai", ["engine-ecs", "engine-physics", "engine-input", "engine-nn"]],
  ["engine-app", ["engine-core", "engine-ecs", "engine-scene", "engine-arena"]],
  ["engine-audio", ["engine-observe"]],
  ["engine-save", ["engine-observe"]],
  ["engine-anim", ["engine-nn", "engine-combat"]],
  ["engine-playtest", ["engine-arena", "engine-render", "engine-provider", "engine-spatial"]],
  ["engine-arena", ["engine-input", "engine-physics", "engine-combat", "engine-ai", "engine-anim", "engine-destruction", "engine-ecs", "engine-provider"]], // AL1 dev-only: mock provider drives the stamped-reply test; provider takes core only, no cycle
  ["engine-assets", ["engine-scene", "engine-anim"]],
  // A9 chaos facade: counts only; proof owns fault injection, observe owns heal records.
  ["engine-chaos", ["engine-observe", "engine-proof"]],
  ["engine-combat", ["engine-input"]],
  ["engine-game", ["engine-arena", "engine-input", "engine-save", "engine-ui", "engine-audio", "engine-render", "engine-combat", "engine-anim", "engine-net", "engine-ai", "engine-core", "engine-destruction", "engine-world", "engine-spatial", "engine-assets"]],
  ["engine-destruction", ["engine-core", "engine-nav"]],
  // The material record composes destruction's MaterialRow (its sim half);
  // render, assets and the viewer read the record above it.
  ["engine-material", ["engine-destruction"]],
  ["engine-editor", ["engine-scene", "engine-render", "engine-spatial", "engine-combat", "engine-ai", "engine-app"]],
  // The duel teacher and strength gate run arena bouts through the public controller seam.
  ["engine-distill", ["engine-nn", "engine-ai", "engine-arena", "engine-input"]],
  // The squad pack reads legion formations from engine-ai (feature `mcp` only).
  // The net verb pack runs loopback bouts through engine-net (feature `mcp` only).
  ["engine-endpoint", ["engine-geo", "engine-editor", "engine-scene", "engine-arena", "engine-spatial", "engine-provider", "engine-observe", "engine-game", "engine-anim", "engine-destruction", "engine-combat", "engine-material", "engine-assets", "engine-rules", "engine-ai", "engine-net"]],
  ["engine-input", ["engine-core"]],
  ["engine-net", ["engine-arena", "engine-input", "engine-observe"]],
  ["engine-nav", ["engine-spatial"]], // SP40 KdTree crowd feed reads spatial KdTree
  // step_islands runs its lanes on the ecs JobPool (persistent workers, index-ordered merge).
  ["engine-physics", ["engine-anim", "engine-ecs"]],
  ["engine-render", ["engine-observe", "engine-assets"]], // v1-only tangents (ARCHITECTURE C3); default stays std-only
  ["engine-ui", ["engine-input"]],
  ["engine-viewer", ["engine-game", "engine-arena", "engine-app", "engine-material"]],
  ["engine-geo", ["engine-assets"]],
  ["engine-perf", ["engine-core", "engine-ecs", "engine-physics", "engine-render", "engine-arena", "engine-ai", "engine-destruction", "engine-net"]],
  ["engine-proof", ["engine-core", "engine-ecs", "engine-physics", "engine-scene", "engine-arena", "engine-observe", "engine-assets", "engine-editor", "engine-render", "engine-net", "engine-combat"]],
  ["engine-core", []],
  ["engine-ecs", []],
  ["engine-nn", []],
  // OPT36 workspace-hack is scoped out: externals-only union, no path edges.
  // It must stay out of the headless default graph (see the visit guard below).
  ["engine-workspace-hack", []],
  // NS32a: the rules adapter implements the arena's controller trait (feature `arena`, off by default).
  ["engine-rules", ["engine-arena"]],
  ["engine-observe", []],
  ["engine-provider", []],
  ["engine-scene", []],
  ["engine-spatial", []],
  ["engine-world", []],
]);

// B3 LFS size guard (2026-10-02): after the history slim, a NEWLY tracked
// non-LFS blob over 5 MB fails. Pure helper (exported for tests); main()
// feeds it the staged-but-not-in-HEAD set, so the pre-slim regular blobs
// already in history never trip it.
export const LFS_SIZE_LIMIT = 5 * 1024 * 1024;
export function largeBlobViolations(entries, limit = LFS_SIZE_LIMIT) {
  return entries
    .filter((e) => !e.lfs && e.bytes > limit)
    .map((e) => `large non-LFS blob: ${e.path} is ${(e.bytes / 1048576).toFixed(2)} MB (> 5 MB) — track it via LFS (.gitattributes)`);
}

// Test layout guard (2026-10-11): a crate with tests/it/main.rs keeps its integration tests as
// modules of that ONE binary. Every top-level tests/*.rs file is one more executable to compile
// and link (the merge cut about 100 of them). A file that needs its own process is listed here
// with the reason; a file that sets #[global_allocator] needs no entry. Keys are <crate dir>/<stem>.
export const OWN_TEST_BINARY = {
  "assets/vfetch_vectors": "its import leg does not compile at HEAD; merge it into tests/it once it builds",
  "endpoint/look_judge_hook": "registers a process-wide one-shot hook",
  "game/crash_hook": "installs the crash hook and re-runs its own executable",
  "perf/profiler": "flips the process-wide ENABLED switch",
  "perf/w1_pf9a_paired": "flips the process-wide ENABLED switch",
};
export function testLayoutViolations(crates, own = OWN_TEST_BINARY) {
  const found = [];
  for (const { name, files } of crates) {
    for (const { file, text } of files) {
      const key = `${name}/${file.replace(/\.rs$/, "")}`;
      if (own[key] || /#\[global_allocator\]/.test(text)) continue;
      found.push(`test layout: crates/${name}/tests/${file} is a new test executable — make it a module of crates/${name}/tests/it/ (one \`mod\` line in main.rs), or list ${key} in OWN_TEST_BINARY with a reason`);
    }
  }
  return found;
}

// Structural map check (2026-10-16): the generated maps must match the tree.
// Fails when (a) a path listed in `.opencode/modmap.md` is missing on disk
// under `crates/`, or (b) the crate list in the maps stops matching the
// Cargo.toml workspace members (`crates/*`). Parses the two maps'
// headings/paths only; no generator run, no cargo. Pure (exported for tests);
// main() feeds it disk reads plus one existsSync pass, so it stays fast.
export function parseModmapHeadings(text) {
  return [...text.matchAll(/^##\s+(\S+)/gm)].map((m) => m[1]);
}
export function parseModmapPaths(text) {
  return [...text.matchAll(/^-\s+`([^`]+)`/gm)]
    .map((m) => m[1].split("·")[0].trim())
    .filter(Boolean);
}
export function structuralMapViolations({ headings, paths, workspaceCrates, repomapHasCrates, missing }) {
  const found = [];
  for (const p of missing ?? []) {
    found.push(`structural map check: .opencode/modmap.md lists \`${p}\` but crates/${p} is missing on disk (stale map, regenerate)`);
  }
  const headSet = new Set(headings ?? []);
  const workSet = new Set(workspaceCrates ?? []);
  const onlyHead = [...headSet].filter((c) => !workSet.has(c)).sort();
  const onlyWork = [...workSet].filter((c) => !headSet.has(c)).sort();
  if (onlyHead.length || onlyWork.length) {
    found.push(`structural map check: crate list mismatch: modmap headings [${[...headSet].sort().join(", ")}] vs workspace crates [${[...workSet].sort().join(", ")}] (only in map: ${onlyHead.join(", ") || "none"}; only on disk: ${onlyWork.join(", ") || "none"})`);
  }
  if (!repomapHasCrates) {
    found.push("structural map check: .opencode/repomap.md tree misses crates/ entry (stale map, regenerate)");
  }
  // Unused `paths` documents the parsed path list for callers; missing above carries the failure.
  void paths;
  return found;
}

// Presence summary (no-locks): one non-blocking line naming live sessions.
// Missing dir or corrupt JSON prints zero, never fails the audit.
function printPresenceSummary(repoDir) {
  try {
    const rows = presenceList(join(repoDir, "target", "presence"));
    const live = rows.filter((r) => !r.stale).length;
    console.log(`presence: ${live} live`);
  } catch {
    console.log("presence: 0 live");
  }
}

// Single-grammar board check (no second parser): task rows and needs come
// from board-slice.mjs (parseBoard/needsOf). The duplicate-id error scans
// raw rows with the same row pattern; first-wins rows match parseBoard.
export function parseBoardTasks(board, archived = null) {
  const { tasks: sliced } = parseBoardMemo(board, archived);
  const tasks = new Map();
  for (const [id, t] of sliced) {
    tasks.set(id, { id, done: t.done, text: t.lines.join(" "), lines: [...t.lines] });
  }
  const errors = [];
  const seen = new Set();
  for (const line of board.split(/\r?\n/)) {
    const head = /^- \[( |x)\] ([A-Z][A-Z0-9]*\d[a-z]?)\b(.*)$/.exec(line);
    if (head) {
      if (seen.has(head[2])) errors.push(`board: duplicate task ${head[2]}`);
      else seen.add(head[2]);
    }
  }
  for (const [id, t] of sliced) {
    const task = tasks.get(id);
    for (const dep of needsOf(t)) {
      if (!tasks.has(dep)) errors.push(`board: ${task.id} needs unknown task ${dep}`);
      else if (task.done && !tasks.get(dep).done) errors.push(`board: ${task.id} is ticked but needs open ${dep}`);
    }
  }
  return { tasks, errors };
}

// Speed: `cargo metadata` (3-10 s) dominates every run, yet its output is
// fixed by the manifests plus the lockfile. Cache the JSON in target/
// (gitignored) keyed by mtime+size of every manifest, the lockfile and the
// cargo config. A manifest edit misses the cache and re-runs cargo, so the
// audit never goes stale. Bypass with WORKSPACE_CHECK_NO_CACHE=1.
const CARGO_CACHE = "workspace-check.cargo-cache.json";
// E10 cut 1: `cargo --version` costs ~4 s per spawn on this box, yet its
// output only keys the metadata cache. Cache the version string in target/
// (gitignored) and reuse it without spawning. A toolchain bump without a
// manifest change keeps the old key; rust-toolchain.toml below still busts
// the metadata cache on toolchain edits, and manifests + lockfile cover the rest.
const CARGO_VERSION_CACHE = "workspace-check.cargo-version.txt";
function cachedCargoVersion(repoDir) {
  const cachePath = join(repoDir, "target", CARGO_VERSION_CACHE);
  try {
    const cached = readFileSync(cachePath, "utf8").trim();
    if (cached) return cached;
  } catch { /* missing or unreadable: spawn once below */ }
  let version = "";
  try { version = execFileSync("cargo", ["--version"], { encoding: "utf8" }).trim(); }
  catch { console.warn("workspace-check: `cargo --version` failed, fingerprint misses the version key"); return version; }
  try {
    mkdirSync(join(repoDir, "target"), { recursive: true });
    writeFileSync(cachePath, version);
  } catch { /* version cache is best-effort */ }
  return version;
}
function cargoFingerprints(repoDir) {
  const fps = [];
  const candidates = [join(repoDir, "Cargo.toml"), join(repoDir, "Cargo.lock"), join(repoDir, "rust-toolchain.toml"), join(repoDir, ".cargo", "config.toml")];
  try {
    for (const dir of readdirSync(join(repoDir, "crates"))) {
      candidates.push(join(repoDir, "crates", dir, "Cargo.toml"));
    }
  } catch { /* no crates dir: fingerprint misses, cargo re-runs */ }
  for (const p of candidates) {
    try {
      const st = statSync(p);
      fps.push(`${relative(repoDir, p)}:${st.mtimeMs}:${st.size}`);
    } catch { fps.push(`${relative(repoDir, p)}:missing`); }
  }
  let cargoVersion = "";
  try { cargoVersion = cachedCargoVersion(repoDir); }
  catch { /* version unknown: fingerprint still keys on files */ }
  return `${cargoVersion}\n${fps.join("\n")}`;
}
function loadCargoMetadata(repoDir) {
  const fp = cargoFingerprints(repoDir);
  if (process.env.WORKSPACE_CHECK_NO_CACHE) return { metadata: null, fp };
  try {
    const raw = readFileSync(join(repoDir, "target", CARGO_CACHE), "utf8");
    const cached = JSON.parse(raw);
    if (cached?.fp === fp && cached?.metadata) return { metadata: cached.metadata, fp, hit: true };
  } catch { /* missing or corrupt: run cargo below */ }
  return { metadata: null, fp };
}
function saveCargoMetadata(repoDir, fp, metadata) {
  try {
    mkdirSync(join(repoDir, "target"), { recursive: true });
    writeFileSync(join(repoDir, "target", CARGO_CACHE), JSON.stringify({ fp, metadata }));
  } catch { /* cache is best-effort: the audit already has its metadata */ }
}

// E10 cut 2: the LFS guard's `git diff --cached` costs ~5.6 s even with
// nothing staged, and the full guard is four git spawns. The staged set is a
// pure function of .git/index plus HEAD, both readable without spawning.
// Cache the fresh (staged-but-not-in-HEAD) list in target/ keyed by those
// fingerprints; an exact key match reuses without any git spawn. An
// index-only match within 60 s also reuses (covers packed-refs imprecision).
const LFS_CACHE = "workspace-check.lfs-cache.json";
const LFS_CACHE_TTL_MS = 60_000;
function indexFingerprint(repoDir) {
  try {
    const st = statSync(join(repoDir, ".git", "index"));
    return `${st.mtimeMs}:${st.size}`;
  } catch { return null; }
}
function headFingerprint(repoDir) {
  try {
    const head = readFileSync(join(repoDir, ".git", "HEAD"), "utf8").trim();
    const ref = /^ref:\s*(.+)$/.exec(head)?.[1];
    if (!ref) return `detached:${head}`;
    try {
      const refPath = join(repoDir, ".git", ...ref.split("/"));
      const st = statSync(refPath);
      const hash = readFileSync(refPath, "utf8").trim();
      return `${ref}:${hash}:${st.mtimeMs}:${st.size}`;
    } catch {
      try {
        const st = statSync(join(repoDir, ".git", "packed-refs"));
        return `${ref}:packed:${st.mtimeMs}:${st.size}`;
      } catch { return `${ref}:unknown`; }
    }
  } catch { return "unknown"; }
}
function loadLfsCache(repoDir, indexFp, headFp) {
  if (!indexFp) return null;
  try {
    const cached = JSON.parse(readFileSync(join(repoDir, "target", LFS_CACHE), "utf8"));
    if (!cached || !Array.isArray(cached.fresh)) return null;
    if (cached.indexFp === indexFp && cached.headFp === headFp) return cached.fresh;
    if (cached.indexFp === indexFp && Number.isFinite(cached.at) && Date.now() - cached.at < LFS_CACHE_TTL_MS) return cached.fresh;
  } catch { /* missing or corrupt: run git below */ }
  return null;
}
function saveLfsCache(repoDir, indexFp, headFp, fresh) {
  if (!indexFp) return;
  try {
    mkdirSync(join(repoDir, "target"), { recursive: true });
    writeFileSync(join(repoDir, "target", LFS_CACHE), JSON.stringify({ indexFp, headFp, fresh, at: Date.now() }));
  } catch { /* LFS cache is best-effort */ }
}

// E11 cut 3: the report-only wiring scan cost ~55 s per spawn (a full
// O(defs x files) regex pass over crates/), yet its first line is fixed by
// the scanned .rs bytes plus the scanner itself. Cache that line in target/
// (gitignored) keyed by content hash plus mtime+size of every scanned file;
// a scanner or file edit misses the cache and re-runs the scan, so the audit
// never goes stale (mtime+size alone collides on same-tick same-size edits).
// Narrow: the scan itself uses one identifier-token index per file plus one
// name -> users map (one lookup per def, ~1 s), not one RegExp per def over
// every other file's full text.
const WIRING_CACHE = "workspace-check.wiring-cache.json";
function wiringContentHash(path) {
  try {
    return createHash("sha256").update(readFileSync(path)).digest("hex");
  } catch { return "unreadable"; }
}
function wiringFingerprint(repoDir) {
  const fps = [];
  try {
    try {
      const scannerPath = join(repoDir, "tools", "wiring-scan.mjs");
      const st = statSync(scannerPath);
      fps.push(`tools/wiring-scan.mjs:${st.mtimeMs}:${st.size}:${wiringContentHash(scannerPath)}`);
    } catch { fps.push("tools/wiring-scan.mjs:missing"); }
    const root = join(repoDir, "crates");
    const stack = [root];
    const files = [];
    while (stack.length) {
      const dir = stack.pop();
      let entries = [];
      try { entries = readdirSync(dir, { withFileTypes: true }); }
      catch { continue; }
      for (const e of entries) {
        const full = join(dir, e.name);
        if (e.isDirectory()) {
          if (e.name === "tests") continue;
          stack.push(full);
        } else if (e.isFile() && e.name.endsWith(".rs")) {
          if (full.split(/[\\/]/).includes("tests")) continue;
          files.push(full);
        }
      }
    }
    files.sort();
    for (const f of files) {
      try {
        const st = statSync(f);
        fps.push(`${relative(repoDir, f)}:${st.mtimeMs}:${st.size}:${wiringContentHash(f)}`);
      } catch { fps.push(`${relative(repoDir, f)}:missing`); }
    }
  } catch { /* fingerprint is best-effort: a miss below re-runs the scan */ }
  return fps.join("\n");
}
function loadWiringCache(repoDir, fp) {
  if (!fp) return null;
  try {
    const cached = JSON.parse(readFileSync(join(repoDir, "target", WIRING_CACHE), "utf8"));
    if (cached?.fp === fp && typeof cached.line === "string" && cached.line) return cached.line;
  } catch { /* missing or corrupt: full scan below */ }
  return null;
}
function saveWiringCache(repoDir, fp, line) {
  if (!fp || !line) return;
  try {
    mkdirSync(join(repoDir, "target"), { recursive: true });
    writeFileSync(join(repoDir, "target", WIRING_CACHE), JSON.stringify({ fp, line, at: Date.now() }));
  } catch { /* wiring cache is best-effort */ }
}

export const WORKSPACE_CHECK_HELP = `workspace-check.mjs — audit the workspace before landing

Usage:
  node tools/workspace-check.mjs [--help]

Checks: Cargo metadata, dependency graph, module reachability, sim
determinism, genre ledger, baseline gate, board needs, new LFS blobs,
structural maps.

Lane triage: this audits the whole worktree, not your lane. A FAIL on a
file outside your owned paths / lane diff is pre-existing dirt, not yours:
leave the file alone and note pre-existing outside owned paths plus path.

Options:
  -h, --help     print this help and exit 0

Exit codes: 0 pass - 1 audit FAIL.
`;

export function hasHelpFlag(argv = process.argv.slice(2)) {
  return argv.includes("--help") || argv.includes("-h");
}

function main() {
try {
  if (hasHelpFlag(process.argv.slice(2))) {
    process.stdout.write(WORKSPACE_CHECK_HELP);
    return;
  }
  const repoDir = join(dirname(fileURLToPath(import.meta.url)), "..");
  const cached = loadCargoMetadata(repoDir);
  const metadata = cached.metadata
    ?? JSON.parse(execFileSync("cargo", ["metadata", "--locked", "--format-version", "1"], { encoding: "utf8", maxBuffer: 32 * 1024 * 1024 }));
  if (!cached.metadata) saveCargoMetadata(repoDir, cached.fp, metadata);
  const members = new Set(metadata.workspace_members);
  const packages = new Map(metadata.packages.map((pkg) => [pkg.id, pkg]));
  const nodes = new Map(metadata.resolve.nodes.map((node) => [node.id, node]));
  const errors = [];
  const workspacePackages = metadata.packages.filter((pkg) => members.has(pkg.id));
  const root = workspacePackages.find((pkg) => pkg.name === "engine2040");
  if (!root) throw new Error("engine2040 application missing from workspace");

  if (root.license !== "Apache-2.0" || !root.rust_version) errors.push("engine2040: project license must be Apache-2.0 and rust-version must be declared (Maxim 2026-10-16)");
  for (const pkg of workspacePackages) {
    for (const field of ["version", "edition", "license", "rust_version"]) {
      if (pkg[field] !== root[field]) errors.push(`${pkg.name}: ${field} ${pkg[field]} differs from workspace ${root[field]}`);
    }
    const allowed = new Set(allowedEdges.get(pkg.name) ?? []);
    for (const dependency of pkg.dependencies.filter((dep) => dep.path)) {
      if (pkg.name !== "engine-core" && UNIVERSAL_EDGES.has(dependency.name)) continue;
      if (!allowed.has(dependency.name)) errors.push(`${pkg.name} -> ${dependency.name}: undocumented local dependency edge`);
    }
    if (pkg.name === "engine-core" && pkg.dependencies.length !== 0) {
      errors.push("engine-core must remain std-only, including optional/build/dev dependencies");
    }
  }

  const visited = new Set();
  function visit(id) {
    if (visited.has(id)) return;
    visited.add(id);
    const pkg = packages.get(id);
    const node = nodes.get(id);
    if (!pkg || !node) throw new Error(`unresolved package ${id}`);
    if (pkg.source !== null) errors.push(`headless default pulls external package ${pkg.name}`);
    // Headless default is the runtime graph, not the test graph: skip
    // dev-only edges (e.g. arena dev-dep -> provider for the AL1
    // stamped-reply test). cargo metadata lists dev edges in node.deps with
    // dep_kinds[].kind === "dev"; a dep with any non-dev kind stays visited
    // (fail closed when dep_kinds is missing).
    for (const dependency of node.deps) {
      const kinds = dependency.dep_kinds;
      if (
        Array.isArray(kinds) &&
        kinds.length > 0 &&
        kinds.every((k) => k && k.kind === "dev")
      )
        continue;
      visit(dependency.pkg);
    }
  }
  visit(root.id);
  // OPT36 scope: the workspace-hack union stays out of the headless default
  // graph (externals-only; enabling engine features would pull optional
  // externals into this resolve, which must stay std-only).
  const hackPkg = workspacePackages.find((pkg) => pkg.name === "engine-workspace-hack");
  if (hackPkg && visited.has(hackPkg.id)) errors.push("engine-workspace-hack is scoped out of the headless default graph");

  // A file no `mod` declares is never compiled, so its tests never run either.
  // Speed: one walk per crate plus a content cache shared by the module,
  // determinism and genre scans below (was: three walks and ~1500 reads).
  let sourceFiles = 0;
  const contentCache = new Map();
  const cachedRead = (file) => {
    let text = contentCache.get(file);
    if (text === undefined) {
      text = readFileSync(file, "utf8");
      contentCache.set(file, text);
    }
    return text;
  };
  const declaresReCache = new Map();
  const declares = (file, name) => {
    if (!existsSync(file)) return false;
    let re = declaresReCache.get(name);
    if (!re) {
      re = new RegExp(`^\\s*(?:pub(?:\\([^)]*\\))?\\s+)?mod\\s+${name}\\s*;`, "m");
      declaresReCache.set(name, re);
    }
    return re.test(cachedRead(file));
  };
  const includeReCache = new Map();
  const includeDeclares = (file, name) => {
    let re = includeReCache.get(name);
    if (!re) {
      re = new RegExp(`include!\\s*\\(\\s*"[^"]*${name}(\\.rs)?"\\s*\\)`);
      includeReCache.set(name, re);
    }
    return re.test(cachedRead(file));
  };
  const walk = (dir) => readdirSync(dir, { withFileTypes: true }).flatMap((entry) =>
    entry.isDirectory() ? walk(join(dir, entry.name)) : entry.name.endsWith(".rs") ? [join(dir, entry.name)] : []);
  const crateFilesByPkg = new Map();
  for (const pkg of workspacePackages) {
    const src = join(dirname(pkg.manifest_path), "src");
    crateFilesByPkg.set(pkg.name, existsSync(src) ? walk(src) : []);
  }
  for (const pkg of workspacePackages) {
    const src = join(dirname(pkg.manifest_path), "src");
    const roots = new Set(pkg.targets.map((target) => target.src_path));
    const crateFiles = crateFilesByPkg.get(pkg.name) ?? [];
    for (const file of crateFiles) {
      sourceFiles += 1;
      const segments = relative(src, file).split(/[\\/]/);
      // src/bin/* files are auto-discovered binary roots.
      if (roots.has(file) || (segments[0] === "bin" && segments.length > 1)) continue;
      const isModRs = basename(file) === "mod.rs";
      const name = basename(isModRs ? dirname(file) : file, ".rs");
      const parentDir = isModRs ? dirname(dirname(file)) : dirname(file);
      const parents = parentDir === src
        ? [join(src, "lib.rs"), join(src, "main.rs")]
        : [`${parentDir}.rs`, join(parentDir, "mod.rs")];
      if (!parents.some((parent) => declares(parent, name))) {
        // `#[path]`-nested modules (e.g. playtest trace_export.rs declared in
        // eval.rs) have no conventional parent file; accept any same-crate
        // file declaring `mod <name>;`. A truly undeclared file still fails.
        const declaredElsewhere = crateFiles.some((other) => other !== file && declares(other, name));
        // `include!()`-inlined files (e.g. editor commands_coalesce.rs folded
        // into commands.rs so the journal types stay in one module) are
        // compiled as part of their includer, which owns them; accept the
        // includer the same way. A file with neither `mod` nor `include!`
        // still fails.
        const includedElsewhere = crateFiles.some((other) => {
          if (other === file || !existsSync(other)) return false;
          return includeDeclares(other, name);
        });
        if (!declaredElsewhere && !includedElsewhere) {
          errors.push(`${pkg.name}: ${relative(src, file)} is not declared by any module, so it is never compiled`);
        }
      }
    }
  }

  // Rollback multiplayer needs bit-identical simulation on every player's PC.
  // Platform float transcendentals (sin, exp, powf...) may round differently
  // across CPUs and C runtimes, so simulation code must use deterministic math.
  // Flipped true when board task D1 landed (vehicle.rs deterministic, 0 hits).
  // The scan (tools/sim-source-scan.mjs) skips comments and strings, and skips a
  // call only when its receiver is proven `engine_core::Fixed` (integer-table
  // sin/cos/tan); an unproven receiver counts, so `f32::sin` still fails. The
  // type-aware twin for the core methods is tools/sim-clippy.toml (CI clippy).
  const DETERMINISM_STRICT = true;
  const simCrates = new Set(SIM_CRATES);
  const ownMath = new Set(["core/src/fixed.rs"]); // Fixed-point type with its own integer sin/cos.
  const crates = join(dirname(root.manifest_path), "crates");
  const fixedPath = join(crates, "core", "src", "fixed.rs");
  // Without the Fixed source nothing is proven Fixed, so every call counts.
  const fixedApi = deriveFixedApi(existsSync(fixedPath) ? cachedRead(fixedPath) : "");
  const mathFindings = [];
  for (const pkg of workspacePackages.filter((p) => simCrates.has(p.name))) {
    for (const file of crateFilesByPkg.get(pkg.name) ?? []) {
      const rel = relative(crates, file).split("\\").join("/");
      if (ownMath.has(rel)) continue;
      // Tests sit after the first #[cfg(test)] by convention and may use any math.
      const hits = platformMathHits(cachedRead(file), fixedApi);
      if (hits.length) mathFindings.push(`${rel} (${hits.length}: ${hits.map((h) => `L${h.line} ${h.receiver}.${h.method}`).join(", ")})`);
    }
  }
  if (mathFindings.length && DETERMINISM_STRICT) {
    errors.push(`platform float math in simulation code (a Fixed call passes only when its receiver is provably Fixed in the file: use a typed binding like \`let a: Fixed = ..\`): ${mathFindings.join(", ")}`);
  }

  // GE2 genre-assumption ledger (verified 2026-09-25, 50 files): engine
  // crates below arena/game (the viewer sits above them, like game) may name Fighter|Gladiator only in these listed
  // files (contract sites, queued moves, harness/budget shapes). A NEW file
  // with such a name fails the gate; moves (GE4/GE5/GE6/GE7) shrink this
  // list, never grow it. Test-only uses live in the ledger notes in
  // docs/plan/runtime.md (ledger); the gate walks src/ (production code). Only identifier
  // tokens count: genre words in comments, doc lines and string literals do not
  // (tools/sim-source-scan.mjs). New engine code that must carry a snapshot ABI type
  // such as FighterSnap names it through a neutral alias (net `BodySnap`).
  const genreLedger = new Set([
    "ai/src/apply.rs",
    "ai/src/curves.rs", "ai/src/depth.rs", "ai/src/gladiator.rs", "ai/src/lib.rs",
    "anim/src/exert.rs", "anim/src/pack.rs",
    "app/src/app.rs", "app/src/tests.rs",
    "audio/src/crowd.rs",
    "combat/src/exchange.rs", "combat/src/fight.rs", "combat/src/gore.rs",
    "combat/src/lib.rs", "combat/src/sweep.rs", "combat/src/tables.rs",
    "ecs/src/bridge.rs",
    "editor/src/instant.rs", "editor/src/templates.rs",
    "endpoint/src/assistant.rs", "endpoint/src/play.rs",
    "endpoint/src/build_level.rs", "endpoint/src/build_character.rs",
    "endpoint/src/build_encounter.rs", "endpoint/src/build_game.rs",
    "input/src/bindings.rs", "input/src/fighter.rs", "input/src/lib.rs",
    "net/src/dedicated.rs", "net/src/delta.rs", "net/src/heal.rs",
    "net/src/lagcomp.rs", "net/src/lib.rs", "net/src/lobby.rs",
    "net/src/replay.rs",     "net/src/scale.rs", "net/src/session.rs",
    // The snapshot codec copies FullSnap/FighterSnap bytes (contract site).
    "net/src/snapshot.rs",
    "net/src/quic.rs",
    "observe/src/lib.rs",
    "perf/src/workloads.rs",
    "physics/src/grapple.rs",
    "playtest/src/agent.rs", "playtest/src/causal.rs", "playtest/src/eval.rs",
    "playtest/src/keypoint.rs", "playtest/src/lib.rs",
    "playtest/src/reachability.rs", "playtest/src/spatial_shapes.rs", "playtest/src/tournament.rs",
    "playtest/src/trigger.rs", "playtest/src/winrates.rs",
    "proof/src/fuzz.rs",
    "provider/src/announce.rs",
    "render/src/baked_gi.rs",
    "ui/src/lib.rs", "ui/src/menus.rs", "ui/src/text.rs",
  ]);
  const genreFindings = [];
  for (const pkg of workspacePackages.filter((p) => !["engine-arena", "engine-game", "engine-viewer"].includes(p.name))) {
    for (const file of crateFilesByPkg.get(pkg.name) ?? []) {
      const rel = relative(crates, file).split("\\").join("/");
      if (genreLedger.has(rel)) continue;
      const names = genreHits(cachedRead(file));
      if (names.length) genreFindings.push(`${rel} (${names.join(", ")})`);
    }
  }
  if (genreFindings.length) {
    errors.push(`unlisted genre assumption (GE2 ledger; the ledger never grows, use a neutral name or alias such as net BodySnap): ${genreFindings.join(", ")}`);
  }

  // PF11 append-only baseline rule (ADV1 finding 2): the exact-match gate
  // in baselines.rs recomputes every recorded row field-for-field, so a
  // widened row cannot hide from it. This presence check stops silent
  // deletion of that gate; the Rust test itself does the refusing.
  const baselineSrc = cachedRead(
    join(dirname(root.manifest_path), "crates", "perf", "src", "baselines.rs"));
  for (const marker of ["mod append_only_tests", "fn check_exact", "honest_rows_match_live_runs_exactly"]) {
    if (!baselineSrc.includes(marker)) {
      errors.push(`PF11 append-only gate missing from baselines.rs: ${marker}`);
    }
  }

  // The lane board drives parallel dispatch: task IDs must be unique, every
  // `needs:` must name a board task, and a ticked task cannot need an open one.
  // Split-safe: the index plus any docs/SPRINT-*.md shards (ARCHIVE excluded);
  // without shards this is the index bytes unchanged.
  // Single board read: the same text feeds board-needs and the research cap below.
  const boardText = loadBoardMemo(repoDir).text;
  const board = boardText.split("## Receipts")[0];
  const parsed = parseBoardTasks(board, archivedDoneRows(repoDir));
  const tasks = parsed.tasks;
  errors.push(...parsed.errors);

  // B3 LFS size guard: staged paths added to the index that are not in HEAD
  // (newly tracked files). LFS status comes from .gitattributes via check-attr.
  // Speed: four batched git calls (ls-tree, ls-files, batch-check,
  // check-attr) instead of 3 spawns per staged file.
  // E10 cut 2: the first `git diff --cached` alone costs ~5.6 s on a clean
  // tree. Skip all four git spawns on a cache hit (index+HEAD fingerprints
  // match, or index matches within 60 s); clean trees then cost two stats.
  try {
    const indexFp = indexFingerprint(repoDir);
    const headFp = headFingerprint(repoDir);
    const cachedFresh = loadLfsCache(repoDir, indexFp, headFp);
    if (cachedFresh) {
      errors.push(...largeBlobViolations(cachedFresh));
    } else {
    const staged = execFileSync("git", ["-C", repoDir, "diff", "--cached", "--name-only", "--diff-filter=A", "-z"], { encoding: "utf8" }).split("\0").filter(Boolean);
    const fresh = [];
    if (staged.length) {
      let batched = false;
      try {
        const inHead = new Set(
          execFileSync("git", ["-C", repoDir, "ls-tree", "HEAD", "--name-only", "-z", "--", ...staged], { encoding: "utf8" }).split("\0").filter(Boolean),
        );
        const freshPaths = staged.filter((p) => !inHead.has(p));
        const shaByPath = new Map();
        if (freshPaths.length) {
          const ls = execFileSync("git", ["-C", repoDir, "ls-files", "-s", "-z", "--", ...freshPaths], { encoding: "utf8" }).split("\0").filter(Boolean);
          for (const rec of ls) {
            const m = /^[0-9]+\s+([0-9a-f]{40})\s+[0-9]\t([\s\S]*)$/.exec(rec);
            if (m) shaByPath.set(m[2], m[1]);
          }
          const bytesBySha = new Map();
          const shas = [...shaByPath.values()];
          if (shas.length) {
            const out = execFileSync("git", ["-C", repoDir, "cat-file", "--batch-check"], { input: shas.join("\n"), encoding: "utf8" });
            for (const line of out.split("\n")) {
              const m = /^([0-9a-f]{40})\s+\S+\s+(\d+)\s*$/.exec(line.trim());
              if (m) bytesBySha.set(m[1], Number(m[2]));
            }
          }
          const lfsByPath = new Map();
          const attrOut = execFileSync("git", ["-C", repoDir, "check-attr", "filter", "--", ...freshPaths], { encoding: "utf8" });
          for (const line of attrOut.split("\n")) {
            const m = /^([\s\S]+): filter: (\S+)\s*$/.exec(line);
            if (m) lfsByPath.set(m[1], m[2] === "lfs");
          }
          for (const p of freshPaths) {
            const sha = shaByPath.get(p);
            if (!sha) continue;
            fresh.push({ path: p, bytes: bytesBySha.get(sha) ?? 0, lfs: lfsByPath.get(p) ?? false });
          }
        }
        batched = true;
      } catch { /* batch path failed: fall back to one spawn per file below */ }
      if (!batched) {
        for (const p of staged) {
          try { execFileSync("git", ["-C", repoDir, "cat-file", "-e", `HEAD:${p}`]); continue; }
          catch { /* not in HEAD: newly tracked */ }
          let bytes = 0;
          try { bytes = Number(execFileSync("git", ["-C", repoDir, "cat-file", "-s", `:${p}`], { encoding: "utf8" }).trim()); }
          catch { continue; }
          let lfs = false;
          try { lfs = /filter: lfs/.test(execFileSync("git", ["-C", repoDir, "check-attr", "filter", "--", p], { encoding: "utf8" })); }
          catch { /* unreadable attr: treat as non-LFS */ }
          fresh.push({ path: p, bytes, lfs });
        }
      }
    }
    errors.push(...largeBlobViolations(fresh));
    saveLfsCache(repoDir, indexFp, headFp, fresh);
    }
  } catch (error) {
    errors.push(`LFS size guard could not run: ${error.message}`);
  }

  // Research cap (tools/research-cap.mjs): an open board row with more
  // than CAP cards fails. A missing cards dir means no breach, never a crash.
  let researchFound = [];
  try {
    const cardsDir = join(dirname(root.manifest_path), "docs", "research", "cards");
    let cardNames = [];
    try { cardNames = readdirSync(cardsDir).filter((n) => n.endsWith(".md")); } catch { cardNames = []; }
    const cardTexts = new Map();
    for (const name of cardNames) {
      try { cardTexts.set(name, cachedRead(join(cardsDir, name))); } catch { cardTexts.set(name, ""); }
    }
    researchFound = researchBreaches({ cardNames, board: boardText, cardTexts });
    for (const b of researchFound) errors.push(researchFailLine(b));
  } catch (error) {
    errors.push(`research cap could not run: ${error.message}`);
  }

  // Test layout guard: crates that merged their tests into tests/it must not grow new test executables.
  let layoutCrates = 0;
  try {
    const cratesDir = join(dirname(root.manifest_path), "crates");
    const layout = [];
    for (const name of readdirSync(cratesDir)) {
      const testsDir = join(cratesDir, name, "tests");
      if (!existsSync(join(testsDir, "it", "main.rs"))) continue;
      const files = readdirSync(testsDir).filter((n) => n.endsWith(".rs")).map((file) => ({ file, text: cachedRead(join(testsDir, file)) }));
      layout.push({ name, files });
    }
    layoutCrates = layout.length;
    errors.push(...testLayoutViolations(layout));
  } catch (error) {
    errors.push(`test layout guard could not run: ${error.message}`);
  }

  // Structural map check: generated maps must match the tree (no generator, no cargo).
  let mapCrates = 0;
  let mapPaths = 0;
  try {
    const modmapText = readFileSync(join(repoDir, ".opencode", "modmap.md"), "utf8");
    const repomapText = readFileSync(join(repoDir, ".opencode", "repomap.md"), "utf8");
    const headings = parseModmapHeadings(modmapText);
    const mapPathsList = parseModmapPaths(modmapText);
    mapCrates = headings.length;
    mapPaths = mapPathsList.length;
    const cratesDir = join(repoDir, "crates");
    const workspaceCrates = readdirSync(cratesDir, { withFileTypes: true })
      .filter((e) => e.isDirectory() && existsSync(join(cratesDir, e.name, "Cargo.toml")))
      .map((e) => e.name);
    const missing = mapPathsList.filter((p) => !existsSync(join(cratesDir, p)));
    const repomapHasCrates = repomapText.includes("`crates/`");
    errors.push(...structuralMapViolations({ headings, paths: mapPathsList, workspaceCrates, repomapHasCrates, missing }));
  } catch (error) {
    errors.push(`structural map check could not run: ${error.message}`);
  }

  try { const archOut = execFileSync(process.execPath, [join(repoDir, "tools", "arch-fitness.mjs")], { encoding: "utf8" }).trim(); if (archOut === "PASS dependency cycle") console.log(archOut); else errors.push(archOut || "dependency cycle found"); } catch (error) { errors.push(`dependency cycle check failed: ${error.message}`); }
  try {
    const wireFp = wiringFingerprint(repoDir);
    let wireOut = loadWiringCache(repoDir, wireFp);
    if (!wireOut) {
      wireOut = execFileSync(process.execPath, [join(repoDir, "tools", "wiring-scan.mjs"), join(repoDir, "crates")], { encoding: "utf8" }).split("\n")[0].trim();
      saveWiringCache(repoDir, wireFp, wireOut);
    }
    console.log(`WARN wiring (report-only): ${wireOut}`);
  } catch (error) { console.log(`WARN wiring (report-only): check failed: ${String(error.message).split("\n")[0]}`); }
  try { const flakyOut = execFileSync(process.execPath, [join(repoDir, "tools", "flaky-quarantine.mjs"), "--self-test"], { encoding: "utf8" }).split("\n").find((l) => l.startsWith("PASS"))?.trim(); console.log(`WARN flaky (report-only): ${flakyOut ?? "no PASS line"}`); } catch (error) { console.log(`WARN flaky (report-only): check failed: ${String(error.message).split("\n")[0]}`); }
  printPresenceSummary(repoDir);
  if (errors.length) throw new Error(errors.join("\n"));
  console.log(`PASS workspace: ${workspacePackages.length} packages share version/edition/license/rust-version`);
  console.log("PASS dependency edges and std-only engine-core");
  console.log(`PASS default headless graph: ${visited.size} workspace packages, no external dependencies`);
  console.log(`PASS modules: all ${sourceFiles} source files are compiled`);
  console.log(`PASS board: ${tasks.size} tasks in docs/SPRINT.md, ids unique, every needs resolves`);
  console.log("PASS lfs guard: no newly tracked non-LFS blob over 5 MB");
  console.log(`PASS research cap: no open row over the card cap (${researchFound.length} breaches)`);
  console.log(`PASS test layout: ${layoutCrates} crates keep one integration-test binary (tests/it)`);
  console.log(`PASS maps: ${mapCrates} crates, ${mapPaths} paths match the tree`);
  console.log(mathFindings.length
    ? `WARN determinism (report-only until board task D1): platform float math in simulation code: ${mathFindings.join(", ")}`
    : "PASS determinism: no platform float math in simulation code");
} catch (error) {
  console.error(`FAIL workspace audit: ${error.message}`);
  process.exitCode = 1;
}
} // main

if (import.meta.main) {
  main();
}
