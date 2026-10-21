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
import { OWN_MATH_FILES, deriveFixedApi, genreFindings, platformMathHits } from "./sim-source-scan.mjs";
import { runWithVerdictCache, scanIslandsFrom } from "./wiring-scan.mjs";

// engine-core is the bottom layer (ARCHITECTURE.md): any crate may use it.
// Without this, packets re-derived its hash, rng and alloc probe instead.
const UNIVERSAL_EDGES = new Set(["engine-core"]);

export const allowedEdges = new Map([
  // KAM3 first playable window: the shell draws the match through engine-render's
  // window leg (fixed 2.5D projection, sprite atlas, HUD pass), composes engine-ui
  // HUD data, reads the shell clock and converts palette bytes with engine-assets.
  // kam2-sim stays std-only and IO-free: it takes no engine crate at all.
  ["kam2-game", ["kam2-sim", "engine-render", "engine-ui", "engine-assets", "engine-clock"]],
  ["kam2-sim", []],
  // KAM8 browser tab: the wasm shell owns the KAM6 match and its fixed 10 Hz tick
  // (kam2-game's AI seat and 2.5D scene), and projects the flat frame through the
  // same fixed camera the window uses. No clock, IO or GPU edge: the page hands in
  // its own time deltas and draws the flat frame with Canvas2D.
  ["kam2-web", ["kam2-sim", "kam2-game", "engine-render"]],
  ["engine2040", ["engine-core", "engine-ecs", "engine-render", "engine-physics", "engine-nav", "engine-audio", "engine-script", "engine-net", "engine-assets", "engine-destruction", "engine-ai", "engine-ponytail", "engine-proof"]],
  ["engine-ai", ["engine-ecs", "engine-physics", "engine-input", "engine-nn"]],
  ["engine-app", ["engine-core", "engine-ecs", "engine-scene", "engine-arena", "engine-clock"]],
  ["engine-audio", ["engine-observe"]],
  ["engine-save", ["engine-observe", "engine-ecs"]], // ARC5: dev-only edge; the spine payload test consumes real `Spine::snapshot`/`restore`, the crate itself stays ecs-free
  ["engine-anim", ["engine-nn", "engine-combat", "engine-ecs", "engine-nav"]],
  ["engine-playtest", ["engine-ai", "engine-arena", "engine-combat", "engine-material", "engine-render", "engine-provider", "engine-spatial"]], // NS31h2: the material gallery reads compiled records (std-only; outside the headless default graph)
  ["engine-arena", ["engine-input", "engine-physics", "engine-combat", "engine-ai", "engine-anim", "engine-destruction", "engine-ecs", "engine-provider", "engine-nav"]], // AL1 dev-only: mock provider drives the stamped-reply test; provider takes core only, no cycle. NAV-01: the nav_layers caller (docs/ARCHITECTURE.md, commit 23fe8a67) declares engine-nav for the sim-lane caller; the edge was sanctioned in the architecture doc and never written down here.
  ["engine-assets", ["engine-scene", "engine-anim", "engine-clock"]],
  // A9 chaos facade: counts only; proof owns fault injection, observe owns heal records.
  ["engine-chaos", ["engine-observe", "engine-proof"]],
  ["engine-combat", ["engine-input"]],
  ["engine-game", ["engine-arena", "engine-input", "engine-save", "engine-ui", "engine-audio", "engine-render", "engine-combat", "engine-anim", "engine-net", "engine-ai", "engine-core", "engine-destruction", "engine-world", "engine-spatial", "engine-assets", "engine-physfx", "engine-clock", "engine-perf"]],
  ["engine-destruction", ["engine-core", "engine-nav"]],
  // The material record composes destruction's MaterialRow (its sim half);
  // render, assets and the viewer read the record above it.
  ["engine-material", ["engine-destruction"]],
  ["engine-editor", ["engine-ecs", "engine-scene", "engine-render", "engine-spatial", "engine-assets", "engine-combat", "engine-ai", "engine-app", "engine-arena", "engine-material"]], // NS31a2: the document holds the material table and edits it with undo; engine-material takes destruction + core only, so no cycle. ARC6: the ui play-in loop steps one arena spine match for the inspector (arena never imports editor).
  // The duel teacher and strength gate run arena bouts through the public controller seam.
  ["engine-distill", ["engine-nn", "engine-ai", "engine-arena", "engine-input"]],
  // The squad pack reads legion formations from engine-ai (feature `mcp` only).
  // The net verb pack runs loopback bouts through engine-net (feature `mcp` only).
  // The physics pack (door verb) reads the std-only balance core: no rapier feature (PX40).
  // AD20: scene.query reachable reads polygons from a baked engine-nav mesh (feature `mcp`).
  // AD8: the door hosts the real look judge from engine-playtest and answers
  // `look.judge` with it (feature `mcp` only).
  // UI5: play.step paints HP bars, names, echo, tick and the you arrow from
  // engine-ui snapshots (feature `mcp` only); ui never imports endpoint.
  ["engine-endpoint", ["engine-geo", "engine-editor", "engine-scene", "engine-arena", "engine-spatial", "engine-provider", "engine-observe", "engine-game", "engine-anim", "engine-destruction", "engine-combat", "engine-material", "engine-assets", "engine-rules", "engine-ai", "engine-net", "engine-physics", "engine-nav", "engine-input", "engine-ui", "engine-playtest", "engine-proof"]],
  ["engine-input", ["engine-core"]],
  ["engine-net", ["engine-arena", "engine-input", "engine-observe", "engine-ecs", "engine-save"]], // ARC5: rejoin resim keeps spine checkpoints in `Spine::snapshot` bytes and rides the save rollback/persist envelopes
  ["engine-nav", ["engine-spatial"]], // SP40 KdTree crowd feed reads spatial KdTree
  // step_islands runs its lanes on the ecs JobPool (persistent workers, index-ordered merge).
  ["engine-physics", ["engine-anim", "engine-ecs"]],
  // RZ1 physical layer: float Rapier world, presentation only. It takes the ragdoll joint builder from
  // physics and the skeleton/pose types from anim. No sim crate (arena, combat, proof) may list it;
  // engine-game lists it as an optional dependency of its `app` leg (RZ4 break look).
  // engine-game takes engine-perf on the same pp leg: the window's input-latency meter
  // (PQ18) reads the perf crate's sampler, never the tick.
  ["engine-physfx", ["engine-physics", "engine-anim"]],
  ["engine-render", ["engine-ui", "engine-observe", "engine-assets", "engine-clock", "engine-material"]], // UIX13: the hud pass draws engine-ui frames (std-only); v1-only tangents (ARCHITECTURE C3); default stays std-only; the clock seam for frame timers (a browser has no std clock); engine-material only on the window leg (NS31c2: pipeline from the compiled WGSL)
  ["engine-ui", ["engine-input"]],
  // WEB4 browser shell: the golden tape and corpus run in the page through engine-proof; the shell hands the
  // page's clock to engine-clock. WEB5: on wasm32 it also drives the render crate's presenter for the arena frame
  // (a target-gated dependency, so the native build of this crate stays a library of pure modules). WEB7 maps browser input
  // and a WebAudio gate onto engine-input and the std-only engine-audio. WEB15: on wasm32 it holds the session the
  // native window holds (`engine_game::App`, the fixed tick and the fight draw helpers), so the page plays the match
  // it draws; that edge is target-gated too. WEB17: it reads the cooked arena content out of a fetched
  // `.e2bundle` through engine-assets' bundle reader (std-only default features; the offline cook owns
  // glTF and image decoding). Nothing depends on it.
  ["engine-web", ["engine-proof", "engine-clock", "engine-render", "engine-input", "engine-audio", "engine-net", "engine-game", "engine-assets"]],
  ["engine-viewer", ["engine-game", "engine-arena", "engine-app", "engine-material"]],
  ["engine-geo", ["engine-assets"]],
  ["engine-perf", ["engine-core", "engine-ecs", "engine-physics", "engine-render", "engine-arena", "engine-ai", "engine-destruction", "engine-net"]],
  // The ragdoll tape (feature `ragdoll`) builds the skeleton and pose types of the fixture from engine-anim.
  // The pillars receipt loads the shipped net through engine-ai and digests its fighter inputs (engine-input).
  ["engine-proof", ["engine-core", "engine-ecs", "engine-physics", "engine-scene", "engine-arena", "engine-observe", "engine-assets", "engine-editor", "engine-render", "engine-net", "engine-combat", "engine-anim", "engine-ai", "engine-input"]],
  ["engine-core", []],
  ["engine-ecs", []],
  ["engine-nn", []],
  // OPT36 workspace-hack is scoped out: externals-only union, no path edges.
  // It must stay out of the headless default graph (see the visit guard below).
  ["engine-workspace-hack", []],
  // NS32a: the rules adapter implements the arena's controller trait (feature `arena`, off by default).
  ["engine-rules", ["engine-arena"]],
  ["engine-observe", []],
  // WEB2 time seam for shell and tool crates: std-only, no engine edges. Simulation crates never list it
  // (a clock reading is a diagnostic and must not reach a hash); see the crate's lib.rs.
  ["engine-clock", []],
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
  "assets/gltf_conformance": "CT10 Accept names it as --test gltf_conformance; it drives the selected glTF models through import, place, save, reopen and play",
  "assets/vfetch_vectors": "its import leg does not compile at HEAD; merge it into tests/it once it builds",
  "endpoint/door_parity": "AD8 Accept names it as --test door_parity; it spawns e2040-mcp and runs the 20-task mirror proof",
  "endpoint/look_judge_hook": "registers a process-wide one-shot hook",
  "endpoint/look_judge_real": "the look judge hook is process-wide and one-shot",
  "game/crash_hook": "installs the crash hook and re-runs its own executable",
  "perf/profiler": "flips the process-wide ENABLED switch",
  "perf/w1_pf9a_paired": "flips the process-wide ENABLED switch",
  "editor/material_ops": "NS31a2 Accept names it as --test material_ops; it drives the document's material table through the public command API",
  "render/material_swatch": "NS31d3 Accept names it as --test material_swatch; it opens its own device and reads a swatch frame back, state the shared tests/it harness does not carry",
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

// Module reachability, one pass per file (E51, 2026-10-20). The old code
// tested one regex per (file, name) pair, so every parent file was rescanned
// once per child file: 747 files cost 2.1 s of regex work. These two helpers
// read a file once and answer every lookup from that one read.
// `mod` declarations: `mod x;`, `pub mod x;`, `pub(crate) mod x;`.
export function parseDeclaredModules(text) {
  const names = new Set();
  const re = /^\s*(?:pub(?:\([^)]*\))?\s+)?mod\s+([A-Za-z_][A-Za-z0-9_]*)\s*;/gm;
  let m;
  while ((m = re.exec(text)) !== null) names.add(m[1]);
  return names;
}

// `include!("path/name.rs")` targets: the base name without the .rs suffix.
export function parseIncludedFiles(text) {
  const names = new Set();
  const re = /include!\s*\(\s*"([^"]+)"\s*\)/g;
  let m;
  while ((m = re.exec(text)) !== null) {
    const path = m[1].split(/[\\/]/).pop() ?? "";
    names.add(path.replace(/\.rs$/, ""));
  }
  return names;
}

// Every Rust source file under a root, skipping `tests` directories: the same
// file set the wiring scan walks (E51: one shared list, one read per file).
export function sourceFilesUnder(root) {
  const out = [];
  const stack = [root];
  while (stack.length) {
    const dir = stack.pop();
    let entries = [];
    try { entries = readdirSync(dir, { withFileTypes: true }); }
    catch { continue; }
    for (const entry of entries) {
      const full = join(dir, entry.name);
      if (entry.isDirectory()) {
        if (entry.name === "tests") continue;
        stack.push(full);
      } else if (entry.isFile() && entry.name.endsWith(".rs")) {
        out.push(full);
      }
    }
  }
  out.sort();
  return out;
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
const HEADLESS_CACHE = "workspace-check.headless-tree.json";
function headlessPackages(repoDir, fp) {
  const path = join(repoDir, "target", HEADLESS_CACHE);
  if (!process.env.WORKSPACE_CHECK_NO_CACHE) {
    try {
      const cached = JSON.parse(readFileSync(path, "utf8"));
      if (cached?.fp === fp && Array.isArray(cached.lines)) return cached.lines;
    } catch { /* missing or corrupt: run cargo below */ }
  }
  const out = execFileSync("cargo", ["tree", "--locked", "-p", "engine2040", "-e", "normal,build", "--prefix", "none", "--format", "{p}"], { encoding: "utf8", maxBuffer: 32 * 1024 * 1024 });
  const lines = [...new Set(out.split("\n").map((l) => l.replace(/ \(\*\)$/, "").trim()).filter(Boolean))];
  try {
    mkdirSync(join(repoDir, "target"), { recursive: true });
    writeFileSync(path, JSON.stringify({ fp, lines }));
  } catch { /* cache is best-effort */ }
  return lines;
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
function wiringFingerprint(repoDir, sources) {
  const fps = [];
  try {
    const scannerPath = join(repoDir, "tools", "wiring-scan.mjs");
    const st = statSync(scannerPath);
    fps.push(`tools/wiring-scan.mjs:${st.mtimeMs}:${st.size}:${wiringContentHash(scannerPath)}`);
  } catch { fps.push("tools/wiring-scan.mjs:missing"); }
  // E51: the bytes come from the shared read, so the fingerprint costs no
  // extra disk pass over crates/. Without a shared map the tree is walked.
  if (sources) {
    for (const [file, entry] of [...sources.entries()].sort()) {
      fps.push(`${relative(repoDir, file)}:${entry.mtimeMs}:${entry.size}:${createHash("sha256").update(entry.text).digest("hex")}`);
    }
    return fps.join("\n");
  }
  for (const file of sourceFilesUnder(join(repoDir, "crates"))) {
    try {
      const st = statSync(file);
      fps.push(`${relative(repoDir, file)}:${st.mtimeMs}:${st.size}:${wiringContentHash(file)}`);
    } catch { fps.push(`${relative(repoDir, file)}:missing`); }
  }
  return fps.join("\n");
}
function loadWiringCache(repoDir, fp) {
  if (!fp) return null;
  if (process.env.WORKSPACE_CHECK_NO_CACHE) return null;
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
  if (hasHelpFlag(process.argv.slice(2))) {
    process.stdout.write(WORKSPACE_CHECK_HELP);
    return;
  }
  const repoDir = join(dirname(fileURLToPath(import.meta.url)), "..");
  runWithVerdictCache({
    repoDir,
    name: "workspace-check",
    // WORKSPACE_CHECK_NO_CACHE=1 runs the full audit every time (no replayed
    // verdict): the switch every other cache in this tool already honors.
    enabled: !process.env.WORKSPACE_CHECK_NO_CACHE,
    run: () => {
      try {
        audit(repoDir);
      } catch (error) {
        console.error(`FAIL workspace audit: ${error.message}`);
        process.exitCode = 1;
      }
    },
  });
}

function audit(repoDir) {
  const cached = loadCargoMetadata(repoDir);
  const metadata = cached.metadata
    ?? JSON.parse(execFileSync("cargo", ["metadata", "--locked", "--format-version", "1"], { encoding: "utf8", maxBuffer: 32 * 1024 * 1024 }));
  if (!cached.metadata) saveCargoMetadata(repoDir, cached.fp, metadata);
  const members = new Set(metadata.workspace_members);
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

  // Headless default = what a bare `cargo build` of the root package compiles (runtime and build deps, no dev
  // edges). `cargo metadata` cannot answer this: it unifies features across every workspace member and platform,
  // so a wasm32-only dep (engine-web -> engine-render/window) would pull winit, wgpu and gltf into the root graph.
  // `cargo tree -p engine2040` resolves the root alone, on the host target; path crates print "(<dir>)", externals do not.
  const headless = headlessPackages(repoDir, cached.fp);
  const visited = new Set();
  for (const line of headless) {
    const name = line.split(" ")[0];
    if (/ \(.+\)$/.test(line)) visited.add(name);
    else errors.push(`headless default pulls external package ${name}`);
  }
  // OPT36 scope: the workspace-hack union stays out of the headless default
  // graph (externals-only; enabling engine features would pull optional
  // externals into this resolve, which must stay std-only).
  const hackPkg = workspacePackages.find((pkg) => pkg.name === "engine-workspace-hack");
  if (hackPkg && visited.has(hackPkg.name)) errors.push("engine-workspace-hack is scoped out of the headless default graph");

  // A file no `mod` declares is never compiled, so its tests never run either.
  // Speed: one walk per crate plus a content cache shared by the module,
  // determinism and genre scans below (was: three walks and ~1500 reads).
  // E51 (2026-10-20): the whole crates/ tree is read once up front and the
  // wiring scan below reuses those bytes, so the workspace is scanned once
  // instead of twice (once here, once again inside the wiring child).
  let sourceFiles = 0;
  const contentCache = new Map();
  const cratesDir = join(dirname(root.manifest_path), "crates");
  const sharedSources = new Map();
  for (const file of sourceFilesUnder(cratesDir)) {
    try {
      const st = statSync(file);
      sharedSources.set(file, { text: readFileSync(file, "utf8"), size: st.size, mtimeMs: st.mtimeMs });
    } catch { /* unreadable file: the checks below report it */ }
  }
  const cachedRead = (file) => {
    let text = contentCache.get(file);
    if (text === undefined) {
      const shared = sharedSources.get(file);
      text = shared ? shared.text : readFileSync(file, "utf8");
      contentCache.set(file, text);
    }
    return text;
  };
  // One parse per file, then a set lookup (E51: was one regex test per
  // (file, name) pair, which rescanned every parent file once per child).
  const declaredCache = new Map();
  const declaredNames = (file) => {
    let names = declaredCache.get(file);
    if (names === undefined) {
      names = existsSync(file) ? parseDeclaredModules(cachedRead(file)) : new Set();
      declaredCache.set(file, names);
    }
    return names;
  };
  const includedCache = new Map();
  const includedNames = (file) => {
    let names = includedCache.get(file);
    if (names === undefined) {
      names = existsSync(file) ? parseIncludedFiles(cachedRead(file)) : new Set();
      includedCache.set(file, names);
    }
    return names;
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
      if (!parents.some((parent) => declaredNames(parent).has(name))) {
        // `#[path]`-nested modules (e.g. playtest trace_export.rs declared in
        // eval.rs) have no conventional parent file; accept any same-crate
        // file declaring `mod <name>;`. A truly undeclared file still fails.
        const declaredElsewhere = crateFiles.some((other) => other !== file && declaredNames(other).has(name));
        // `include!()`-inlined files (e.g. editor commands_coalesce.rs folded
        // into commands.rs so the journal types stay in one module) are
        // compiled as part of their includer, which owns them; accept the
        // includer the same way. A file with neither `mod` nor `include!`
        // still fails.
        const includedElsewhere = crateFiles.some((other) => {
          if (other === file || !existsSync(other)) return false;
          return includedNames(other).has(name);
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
  const ownMath = new Set(OWN_MATH_FILES); // Fixed-point type with its own integer sin/cos.
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

  // GE2 genre-assumption ledger, re-audited by GE11 (2026-10-18, 85 files):
  // engine crates below arena/game (the viewer sits above them, like game) may
  // name Fighter|Gladiator (types, fns, and FIGHTER|GLADIATOR caps and ids) only
  // in these listed files. Verdicts: contract site (the arena/net/input ABI
  // carries the word), harness (Rome harnesses and bench shapes that fixtures
  // must not import), budget-shape (a cap or size tuned for the arena), or move
  // (queued under GE4/GE5/GE6/GE7). A NEW file with such a name fails the gate;
  // moves shrink this list, never grow it, and a listed file that no longer
  // names a genre word is reported stale: prune it. Test-only uses live in the
  // ledger notes in docs/plan/runtime.md (ledger); the gate walks src/
  // (production code). Only identifier tokens count: genre words in comments,
  // doc lines and string literals do not (tools/sim-source-scan.mjs). New engine
  // code that must carry a snapshot ABI type such as FighterSnap names it
  // through a neutral alias (net `BodySnap`). `Arena` is out of scope by
  // design (engine-arena imports are arena-the-place wiring).
  const genreLedger = new Set([
    // ai, combat, ui, net: move (GE4/GE5/GE6/GE7) or contract sites.
    "ai/src/apply.rs", "ai/src/depth.rs", "ai/src/gladiator.rs",
    // The plan trace asks the ai crate's own gladiator percept for its plan
    // (contract site, the same site ai/src/gladiator.rs already carries).
    "ai/src/plan_trace.rs",
    "anim/src/pack.rs",
    "app/src/app.rs", "app/src/tests.rs",
    "combat/src/exchange.rs", "combat/src/fight.rs", "combat/src/lib.rs",
    "combat/src/sweep.rs", "combat/src/tables.rs",
    "ecs/src/bridge.rs",
    "editor/src/templates.rs",
    "endpoint/src/assistant.rs", "endpoint/src/assistant/fork.rs", "endpoint/src/play.rs",
    "endpoint/src/build_game.rs",
    "input/src/bindings.rs", "input/src/fighter.rs", "input/src/lib.rs",
    // The layer tape wraps the input FighterTape and names FighterError (contract site, like input/src/fighter.rs).
    "input/src/layers.rs",
    "net/src/dedicated.rs", "net/src/delta.rs", "net/src/quic.rs",
    "net/src/replay.rs", "net/src/scale.rs", "net/src/session.rs",
    // The snapshot codec copies FullSnap/FighterSnap bytes (contract site).
    "net/src/snapshot.rs",
    "perf/src/workloads.rs",
    "playtest/src/agent.rs", "playtest/src/causal.rs", "playtest/src/eval.rs",
    "playtest/src/keypoint.rs", "playtest/src/reachability.rs", "playtest/src/spatial_shapes.rs",
    // Keypoint submodules split out of keypoint.rs (listed above): the spec parser and the spine track name the arena
    // FighterTape/FighterInput contract types, the same sites keypoint.rs already carries.
    "playtest/src/keypoint/spec_parse.rs", "playtest/src/keypoint/spine.rs",
    // The injected verification unit names the arena FighterInput contract type, the same site keypoint.rs already carries.
    "playtest/src/keypoint/inject.rs",
    "playtest/src/tournament.rs", "playtest/src/winrates.rs",
    "proof/src/fuzz.rs",
    // provider: the 8-observation window (budget-shape); render: the baked-GI sample budget (presentation).
    "provider/src/announce.rs", "render/src/baked_gi.rs",
    "ui/src/lib.rs", "ui/src/menus.rs",
    // WEB7: the browser input records wrap engine-input FighterId/FighterInput (contract site, like input/src/fighter.rs).
    "web/src/input.rs",
    // GE11 caps and ids (FIGHTER|GLADIATOR constants), budget-shape unless noted.
    // anim: rig joint count, crossfade tuning and the 64-body pose and IK caps.
    "anim/src/crossfade.rs", "anim/src/fbik.rs", "anim/src/hit_react.rs", "anim/src/injured.rs",
    "anim/src/lod.rs", "anim/src/lod_w1_test.rs",
    "anim/src/state_machine.rs", "anim/src/state_machine/playback.rs", "anim/src/state_machine/tests.rs",
    // editor: Rome pack manifest path (harness).
    "editor/src/a2_session.rs", "editor/src/a3_session.rs",
    // endpoint: Rome world loader body height (harness, like play.rs).
    "endpoint/src/play/world_load.rs",
    // net: bout and duel harness sizes; the websocket and webtransport tests carry the 8-slot shape.
    "net/src/played.rs", "net/src/webduel.rs",
    "net/src/websocket/tests.rs", "net/src/webtransport/tests.rs",
    // observe: supervisor catalog ids that name the stuck-fighter brain case (contract site).
    "observe/src/event_queue.rs", "observe/src/fps.rs", "observe/src/heal.rs", "observe/src/lib.rs",
    "observe/src/supervisor.rs",
    "perf/src/fight_cost.rs",
    "physics/src/islands.rs", "physics/src/layers.rs", "physics/src/lod.rs",
    // GE12-era splits: the cloth module moved from one file into a directory, and the
    // collide track names the physics layer group's FIGHTER const (the same site
    // physics/src/layers.rs already carries).
    "physics/src/cloth/collide.rs", "physics/src/cloth/tests.rs",
    "playtest/src/brief_eval.rs",
    // proof: Rome harnesses (fixtures must not import them).
    "proof/src/a9.rs", "proof/src/arena_matrix.rs", "proof/src/chaos.rs", "proof/src/tape_fuzz.rs",
    // render: 8-fighter presentation shapes (a view of the arena, never fed to a hash).
    "render/src/accel.rs", "render/src/procedural_arena.rs", "render/src/twin_fighters.rs",
    // rules: MAX_FIGHTERS is the world's actor cap, equal to the net server player cap (contract site).
    "rules/src/arena.rs", "rules/src/fixtures.rs", "rules/src/host.rs", "rules/src/lib.rs",
    "rules/src/limits.rs", "rules/src/rules.rs", "rules/src/sim.rs", "rules/src/tape.rs",
  ]);
  const genreScan = [];
  for (const pkg of workspacePackages.filter((p) => !["engine-arena", "engine-game", "engine-viewer"].includes(p.name))) {
    for (const file of crateFilesByPkg.get(pkg.name) ?? []) {
      genreScan.push({ rel: relative(crates, file).split("\\").join("/"), src: cachedRead(file) });
    }
  }
  const genre = genreFindings(genreScan, genreLedger);
  if (genre.unlisted.length) {
    errors.push(`unlisted genre assumption (GE2 ledger; the ledger never grows, use a neutral name or alias such as net BodySnap): ${genre.unlisted.join(", ")}`);
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
    const wireFp = wiringFingerprint(repoDir, sharedSources);
    let wireOut = loadWiringCache(repoDir, wireFp);
    if (!wireOut) {
      // E51: the wiring scan runs in process on the bytes this audit already
      // read. The child process re-read the whole tree and paid a node
      // startup for the same answer.
      const islands = scanIslandsFrom(
        [...sharedSources.entries()].map(([file, entry]) => ({ file, text: entry.text })),
        cratesDir,
      );
      wireOut = `islands: ${islands.length}`;
      saveWiringCache(repoDir, wireFp, wireOut);
    }
    console.log(`WARN wiring (report-only): ${wireOut}`);
  } catch (error) { console.log(`WARN wiring (report-only): check failed: ${String(error.message).split("\n")[0]}`); }
  try { const flakyOut = execFileSync(process.execPath, [join(repoDir, "tools", "flaky-quarantine.mjs"), "--self-test"], { encoding: "utf8" }).split("\n").find((l) => l.startsWith("PASS"))?.trim(); console.log(`WARN flaky (report-only): ${flakyOut ?? "no PASS line"}`); } catch (error) { console.log(`WARN flaky (report-only): check failed: ${String(error.message).split("\n")[0]}`); }
  if (errors.length) throw new Error(errors.join("\n"));
  console.log(`PASS workspace: ${workspacePackages.length} packages share version/edition/license/rust-version`);
  console.log("PASS dependency edges and std-only engine-core");
  console.log(`PASS default headless graph: ${visited.size} workspace packages, no external dependencies`);
  console.log(`PASS modules: all ${sourceFiles} source files are compiled`);
  console.log(`PASS board: ${tasks.size} tasks in docs/SPRINT.md, ids unique, every needs resolves`);
  console.log("PASS lfs guard: no newly tracked non-LFS blob over 5 MB");
  console.log(`PASS test layout: ${layoutCrates} crates keep one integration-test binary (tests/it)`);
  console.log(`PASS maps: ${mapCrates} crates, ${mapPaths} paths match the tree`);
  console.log(genre.stale.length
    ? `WARN genre ledger (report-only): stale entries name no genre word any more, prune them: ${genre.stale.join(", ")}`
    : `PASS genre ledger: ${genreLedger.size} files, none unlisted, none stale`);
  console.log(mathFindings.length
    ? `WARN determinism (report-only until board task D1): platform float math in simulation code: ${mathFindings.join(", ")}`
    : "PASS determinism: no platform float math in simulation code");
} // audit

if (import.meta.main) {
  main();
}
