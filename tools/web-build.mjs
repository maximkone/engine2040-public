// Build the browser shell into dist/web (WEB4).
//   node tools/web-build.mjs [--out dist/web] [--commit <hash>] [--no-opt] [--opt-ab]
//   node tools/web-build.mjs --size-sweep [z,s,3] [--commit <hash>]
// cargo builds crates/web for wasm32-unknown-unknown with the size-tuned `web` profile (root
// Cargo.toml), the wasm-bindgen CLI at the version Cargo.lock locks turns the module into
// engine.js + engine_bg.wasm, wasm-opt -Oz shrinks it when it is installed, and the page
// (crates/web/index.html) is written beside them with build.json: sizes, gzip sizes, SHA-256,
// tool versions, the commit baked into the module. Exit 0 = dist/web is complete and under
// the size limit; 1 = a step failed (the message says which and how to fix it).
// QW1: --size-sweep builds the web profile once per opt-level (CARGO_PROFILE_WEB_OPT_LEVEL; the same LTO and one codegen unit),
// prints raw, gzip and brotli bytes per level and names the smallest; it writes no dist/web. The winner goes into
// [profile.web]; the browser job of the next run proves the golden receipt on it.
// QW2: --opt-ab runs wasm-opt at -Oz, -Os and -O3 on the same module, prints the three sizes and ships the smallest;
// build.json keeps the table (wasmOptAb) and twiggy's top 20 items.
// Every step goes through `deps` (run a command, read and write files), so the test drives
// the whole build with a fake toolchain.
import { createHash } from "node:crypto";
import { spawnSync } from "node:child_process";
import { cpSync, existsSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, statSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { gzipSync } from "node:zlib";
import { brotliSize } from "./web-budget.mjs";

export const TARGET = "wasm32-unknown-unknown";
export const PROFILE = "web";
export const PACKAGE = "engine-web";
export const OUT_NAME = "engine";
export const SIZE_LIMIT_BYTES = 8 * 1024 * 1024; // dist/web, before game content
// Rust 1.97 emits bulk-memory, nontrapping float-to-int and sign-extension instructions (and, since 1.82, multivalue and
// reference types); wasm-opt rejects a module that uses a feature it was not told about. All six ship in every browser
// that has WebGPU, so enabling them lets the optimizer use them and nothing newer.
export const WASM_OPT_FEATURES = ["--enable-bulk-memory", "--enable-nontrapping-float-to-int", "--enable-sign-ext", "--enable-multivalue", "--enable-reference-types", "--enable-mutable-globals"];
export const WASM_OPT_FLAGS = ["-Oz", ...WASM_OPT_FEATURES];
export const WASM_OPT_AB = ["-Oz", "-Os", "-O3"]; // the levels --opt-ab compares; the first wins a tie
export const SWEEP_LEVELS = ["z", "s", "3"]; // the opt-levels --size-sweep builds; the first wins a tie
export const TWIGGY_ITEMS = 20;
// The files the page needs, in the order build.json lists them. Anything else wasm-bindgen writes (snippets/) is copied too.
export const DIST_FILES = ["index.html", "engine.js", "engine_bg.wasm", "build.json"];

export class BuildError extends Error {}

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");

// The version of package `name` in Cargo.lock text. Exactly one version must be locked: the
// wasm-bindgen CLI has to match the library byte for byte.
export function lockedVersion(lockText, name) {
  const versions = [];
  for (const block of lockText.split(/\r?\n\r?\n/)) {
    const m = block.match(/^name = "([^"]+)"\s*\r?\nversion = "([^"]+)"/m);
    if (m && m[1] === name) versions.push(m[2]);
  }
  if (versions.length !== 1) throw new BuildError(`Cargo.lock has ${versions.length} versions of ${name} (${versions.join(", ") || "none"}); the build needs exactly one`);
  return versions[0];
}

export const cargoArgs = () => ["build", "--locked", "--profile", PROFILE, "--target", TARGET, "-p", PACKAGE];
export const builtWasmPath = (rootDir) => join(rootDir, "target", TARGET, PROFILE, "engine_web.wasm");
export const bindgenArgs = (wasmIn, outDir) => ["--target", "web", "--no-typescript", "--out-dir", outDir, "--out-name", OUT_NAME, wasmIn];
export const bindgenFix = (version) => `cargo install wasm-bindgen-cli --version ${version} --locked`;

export function renderHtml(template, commit) {
  if (!template.includes("{{COMMIT}}")) throw new BuildError("crates/web/index.html lost its {{COMMIT}} placeholder");
  return template.replaceAll("{{COMMIT}}", commit);
}

const sha256 = (buf) => createHash("sha256").update(buf).digest("hex");
const gzipBytes = (buf) => gzipSync(buf, { level: 9 }).length;

const tail = (text, n = 25) => String(text).trimEnd().split(/\r?\n/).slice(-n).join("\n");

// Every file under `dir`, relative with forward slashes.
export function listFiles(dir, base = dir) {
  const out = [];
  for (const e of readdirSync(dir, { withFileTypes: true })) {
    const p = join(dir, e.name);
    if (e.isDirectory()) out.push(...listFiles(p, base));
    else out.push(relative(base, p).split("\\").join("/"));
  }
  return out.sort();
}

// The real machine. `run(cmd, args, {env, cwd})` -> { status, out }.
export const realDeps = () => ({
  run: (cmd, args, o = {}) => {
    const r = spawnSync(cmd, args, { encoding: "utf8", maxBuffer: 1 << 28, env: { ...process.env, ...(o.env ?? {}) }, cwd: o.cwd });
    return { status: r.error ? 127 : r.status, out: `${r.stdout ?? ""}${r.stderr ?? ""}` };
  },
  readText: (p) => readFileSync(p, "utf8"),
  readBytes: (p) => readFileSync(p),
  tempDir: () => mkdtempSync(join(tmpdir(), "web-build-")),
  say: (line) => console.log(line),
});

// Run a command; a non-zero exit throws a BuildError that quotes the end of its output.
const stepOf = (run) => (label, cmd, args, o) => {
  const r = run(cmd, args, o);
  if (r.status !== 0) throw new BuildError(`${label} failed (exit ${r.status}): ${cmd} ${args.join(" ")}\n${tail(r.out)}`);
  return r.out;
};

// The wasm-bindgen CLI version (it must equal the locked library), the commit baked into the module and rustc's line.
function toolInfo({ rootDir, commit }, deps) {
  const { run } = deps;
  const bindgenVersion = lockedVersion(deps.readText(join(rootDir, "Cargo.lock")), "wasm-bindgen");
  const cli = run("wasm-bindgen", ["--version"]);
  const cliVersion = cli.status === 0 ? cli.out.trim().replace(/^wasm-bindgen\s+/, "") : null;
  if (cliVersion === null) throw new BuildError(`the wasm-bindgen CLI is not installed; Cargo.lock locks ${bindgenVersion}: ${bindgenFix(bindgenVersion)}`);
  if (cliVersion !== bindgenVersion) throw new BuildError(`wasm-bindgen CLI is ${cliVersion} but Cargo.lock locks ${bindgenVersion}; they must match exactly: ${bindgenFix(bindgenVersion)}`);
  let hash = commit ?? process.env.ENGINE_WEB_COMMIT ?? null;
  if (!hash) {
    const g = run("git", ["rev-parse", "--short=12", "HEAD"], { cwd: rootDir });
    hash = g.status === 0 && g.out.trim() ? g.out.trim() : "unknown";
  }
  return { cliVersion, hash, rustc: run("rustc", ["--version"]).out.trim() };
}

// cargo, wasm-bindgen and (when installed) wasm-opt: leaves engine.js and engine_bg.wasm in `work`. `env` is extra cargo
// environment (the sweep's opt-level); `ab` runs wasm-opt once per WASM_OPT_AB level and keeps the smallest module.
// Returns { rawBytes (before wasm-opt), wasmOpt (its version or null), flags (what ran or null), ab (the table or null) }.
function compile({ rootDir, hash, optimize, ab = false, env = {} }, work, deps) {
  const { run, say } = deps;
  const step = stepOf(run);
  const level = env.CARGO_PROFILE_WEB_OPT_LEVEL;
  say(`[web-build] cargo build --profile ${PROFILE} --target ${TARGET} (commit ${hash}${level ? `, opt-level ${level}` : ""})`);
  step("cargo build", "cargo", cargoArgs(), { env: { ENGINE_WEB_COMMIT: hash, ...env }, cwd: rootDir });
  const built = builtWasmPath(rootDir);
  if (!existsSync(built)) throw new BuildError(`cargo reported success but ${built} does not exist`);
  say("[web-build] wasm-bindgen --target web");
  step("wasm-bindgen", "wasm-bindgen", bindgenArgs(built, work));
  const wasmPath = join(work, `${OUT_NAME}_bg.wasm`);
  if (!existsSync(wasmPath) || !existsSync(join(work, `${OUT_NAME}.js`))) throw new BuildError(`wasm-bindgen wrote ${listFiles(work).join(", ") || "nothing"}; expected ${OUT_NAME}.js and ${OUT_NAME}_bg.wasm`);
  const rawBytes = statSync(wasmPath).size;

  const probe = optimize ? run("wasm-opt", ["--version"]) : { status: 1, out: "" };
  if (!optimize || probe.status !== 0) {
    say(`[web-build] wasm-opt ${optimize ? "not installed" : "skipped"}: the module ships as wasm-bindgen wrote it`);
    return { rawBytes, wasmOpt: null, flags: null, ab: null };
  }
  const wasmOpt = probe.out.trim();
  if (!ab) {
    say(`[web-build] wasm-opt ${WASM_OPT_FLAGS.join(" ")}`);
    step("wasm-opt", "wasm-opt", [wasmPath, "-o", wasmPath, ...WASM_OPT_FLAGS]);
    return { rawBytes, wasmOpt, flags: WASM_OPT_FLAGS, ab: null };
  }
  const table = WASM_OPT_AB.map((flag) => {
    const file = `${wasmPath}.${flag}`;
    step("wasm-opt", "wasm-opt", [wasmPath, "-o", file, flag, ...WASM_OPT_FEATURES]);
    const bytes = deps.readBytes(file);
    say(`[wasm-opt-ab] ${flag} raw=${bytes.length} gzip=${gzipBytes(bytes)}`);
    return { flag, bytes: bytes.length, gzipBytes: gzipBytes(bytes), file };
  });
  const best = table.reduce((a, b) => (b.bytes < a.bytes ? b : a));
  cpSync(best.file, wasmPath);
  for (const t of table) rmSync(t.file, { force: true });
  say(`[wasm-opt-ab] smallest raw: ${best.flag}`);
  return { rawBytes, wasmOpt, flags: [best.flag, ...WASM_OPT_FEATURES], ab: table.map(({ file, ...row }) => row) };
}

// QW1: one full web build per opt-level; the printed table is the receipt. Nothing is written to dist/web.
export function sizeSweep({ rootDir = root, commit = null, levels = SWEEP_LEVELS, optimize = true } = {}, deps = realDeps()) {
  const { hash } = toolInfo({ rootDir, commit }, deps);
  const rows = levels.map((level) => {
    const work = deps.tempDir();
    try {
      const c = compile({ rootDir, hash, optimize, env: { CARGO_PROFILE_WEB_OPT_LEVEL: level } }, work, deps);
      const bytes = deps.readBytes(join(work, `${OUT_NAME}_bg.wasm`));
      const row = { level, preOptBytes: c.rawBytes, bytes: bytes.length, gzipBytes: gzipBytes(bytes), brotliBytes: brotliSize(bytes) };
      deps.say(`[size-sweep] opt-level=${level} raw=${row.bytes} pre-opt=${row.preOptBytes} gzip=${row.gzipBytes} brotli=${row.brotliBytes}`);
      return row;
    } finally {
      rmSync(work, { recursive: true, force: true });
    }
  });
  const best = rows.reduce((a, b) => (b.bytes < a.bytes ? b : a));
  deps.say(`[size-sweep] smallest raw wasm: opt-level ${best.level} (${best.bytes} bytes); set it in [profile.web] and let the browser job prove the golden receipt`);
  return { rows, best: best.level };
}

// Build dist/web. Returns the build.json object. Throws BuildError when a step fails; `outDir` is then either untouched or gone, never half written.
export function build({ rootDir = root, outDir = join(root, "dist", "web"), commit = null, optimize = true, ab = false } = {}, deps = realDeps()) {
  const { run, say } = deps;
  const { cliVersion, hash, rustc } = toolInfo({ rootDir, commit }, deps);
  const work = deps.tempDir();
  try {
    const c = compile({ rootDir, hash, optimize, ab }, work, deps);

    // The largest items of the shipped module (QW2): twiggy prints a header, TWIGGY_ITEMS rows and a remainder line.
    let twiggy = [];
    const tw = run("twiggy", ["top", "-n", String(TWIGGY_ITEMS), join(work, `${OUT_NAME}_bg.wasm`)]);
    if (tw.status === 0) {
      twiggy = tw.out.trim().split(/\r?\n/).slice(0, TWIGGY_ITEMS + 3);
      for (const l of twiggy) say(`[twiggy] ${l}`);
    }

    const page = renderHtml(deps.readText(join(rootDir, "crates", "web", "index.html")), hash);

    // Everything is ready: only now touch outDir.
    rmSync(outDir, { recursive: true, force: true });
    mkdirSync(outDir, { recursive: true });
    for (const rel of listFiles(work)) {
      if (rel.endsWith(".d.ts")) continue;
      const to = join(outDir, rel);
      mkdirSync(dirname(to), { recursive: true });
      cpSync(join(work, rel), to);
    }
    writeFileSync(join(outDir, "index.html"), page);

    const files = {};
    let totalBytes = 0;
    for (const rel of listFiles(outDir)) {
      const bytes = deps.readBytes(join(outDir, rel));
      files[rel] = { bytes: bytes.length, gzipBytes: gzipBytes(bytes), sha256: sha256(bytes) };
      totalBytes += bytes.length;
    }
    const info = {
      schema: 1,
      commit: hash,
      target: TARGET,
      profile: PROFILE,
      files,
      totalBytes,
      limitBytes: SIZE_LIMIT_BYTES,
      underLimit: totalBytes < SIZE_LIMIT_BYTES,
      rawWasmBytes: c.rawBytes,
      tools: { rustc, wasmBindgen: cliVersion, wasmOpt: c.wasmOpt },
      wasmOptFlags: c.flags,
      wasmOptAb: c.ab,
      twiggy,
    };
    // build.json lists the other files; it cannot hold its own hash.
    writeFileSync(join(outDir, "build.json"), `${JSON.stringify(info, null, 2)}\n`);
    if (!info.underLimit) {
      rmSync(outDir, { recursive: true, force: true }); // no half-shipped build over the limit
      throw new BuildError(`dist/web is ${totalBytes} bytes, over the ${SIZE_LIMIT_BYTES} byte limit`);
    }
    say(`[web-build] dist/web ${totalBytes} bytes (limit ${SIZE_LIMIT_BYTES}); wasm ${files["engine_bg.wasm"].bytes} bytes, ${files["engine_bg.wasm"].gzipBytes} gzip`);
    return info;
  } finally {
    rmSync(work, { recursive: true, force: true });
  }
}

export function parseArgs(argv) {
  const o = { out: null, commit: null, optimize: true, ab: false, sweep: null, error: null };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (a === "--out") o.out = argv[++i] ?? null;
    else if (a === "--commit") o.commit = argv[++i] ?? null;
    else if (a === "--no-opt") o.optimize = false;
    else if (a === "--opt-ab") o.ab = true;
    else if (a === "--size-sweep") o.sweep = argv[i + 1] && !argv[i + 1].startsWith("--") ? argv[++i].split(",") : SWEEP_LEVELS;
    else o.error = `unknown argument ${a}`;
  }
  if ((argv.includes("--out") && !o.out) || (argv.includes("--commit") && !o.commit)) o.error = "--out and --commit need a value";
  if (o.sweep && !o.sweep.every((l) => /^[0-3sz]$/.test(l))) o.error = "--size-sweep takes opt-levels from 0 1 2 3 s z, comma separated";
  return o;
}

if (import.meta.main) {
  const a = parseArgs(process.argv.slice(2));
  if (a.error) {
    console.error(`web-build: ${a.error}\nusage: node tools/web-build.mjs [--out dist/web] [--commit <hash>] [--no-opt] [--opt-ab] | --size-sweep [z,s,3]`);
    process.exit(2);
  }
  try {
    if (a.sweep) sizeSweep({ commit: a.commit, levels: a.sweep, optimize: a.optimize });
    else build({ outDir: a.out ? resolve(a.out) : join(root, "dist", "web"), commit: a.commit, optimize: a.optimize, ab: a.ab });
  } catch (e) {
    if (!(e instanceof BuildError)) throw e;
    console.error(`web-build: ${e.message}`);
    process.exit(1);
  }
}
