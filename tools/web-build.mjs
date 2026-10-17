// Build the browser shell into dist/web (WEB4).
//   node tools/web-build.mjs [--out dist/web] [--commit <hash>] [--no-opt]
// cargo builds crates/web for wasm32-unknown-unknown with the size-tuned `web` profile (root
// Cargo.toml), the wasm-bindgen CLI at the version Cargo.lock locks turns the module into
// engine.js + engine_bg.wasm, wasm-opt -Oz shrinks it when it is installed, and the page
// (crates/web/index.html) is written beside them with build.json: sizes, gzip sizes, SHA-256,
// tool versions, the commit baked into the module. Exit 0 = dist/web is complete and under
// the size limit; 1 = a step failed (the message says which and how to fix it).
// Every step goes through `deps` (run a command, read and write files), so the test drives
// the whole build with a fake toolchain.
import { createHash } from "node:crypto";
import { spawnSync } from "node:child_process";
import { cpSync, existsSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, statSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { gzipSync } from "node:zlib";

export const TARGET = "wasm32-unknown-unknown";
export const PROFILE = "web";
export const PACKAGE = "engine-web";
export const OUT_NAME = "engine";
export const SIZE_LIMIT_BYTES = 8 * 1024 * 1024; // dist/web, before game content
// Rust 1.97 emits bulk-memory, nontrapping float-to-int and sign-extension instructions (and, since 1.82, multivalue and
// reference types); wasm-opt rejects a module that uses a feature it was not told about. All six ship in every browser
// that has WebGPU, so enabling them lets the optimizer use them and nothing newer.
export const WASM_OPT_FLAGS = ["-Oz", "--enable-bulk-memory", "--enable-nontrapping-float-to-int", "--enable-sign-ext", "--enable-multivalue", "--enable-reference-types", "--enable-mutable-globals"];
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

// Build dist/web. Returns the build.json object. Throws BuildError when a step fails; `outDir` is then either untouched or gone, never half written.
export function build({ rootDir = root, outDir = join(root, "dist", "web"), commit = null, optimize = true } = {}, deps = realDeps()) {
  const { run, say } = deps;
  const step = (label, cmd, args, o) => {
    const r = run(cmd, args, o);
    if (r.status !== 0) throw new BuildError(`${label} failed (exit ${r.status}): ${cmd} ${args.join(" ")}\n${tail(r.out)}`);
    return r.out;
  };

  const lockText = deps.readText(join(rootDir, "Cargo.lock"));
  const bindgenVersion = lockedVersion(lockText, "wasm-bindgen");
  const cli = run("wasm-bindgen", ["--version"]);
  const cliVersion = cli.status === 0 ? cli.out.trim().replace(/^wasm-bindgen\s+/, "") : null;
  if (cliVersion === null) throw new BuildError(`the wasm-bindgen CLI is not installed; Cargo.lock locks ${bindgenVersion}: ${bindgenFix(bindgenVersion)}`);
  if (cliVersion !== bindgenVersion) throw new BuildError(`wasm-bindgen CLI is ${cliVersion} but Cargo.lock locks ${bindgenVersion}; they must match exactly: ${bindgenFix(bindgenVersion)}`);

  let hash = commit ?? process.env.ENGINE_WEB_COMMIT ?? null;
  if (!hash) {
    const g = run("git", ["rev-parse", "--short=12", "HEAD"], { cwd: rootDir });
    hash = g.status === 0 && g.out.trim() ? g.out.trim() : "unknown";
  }

  const rustc = run("rustc", ["--version"]).out.trim();
  say(`[web-build] cargo build --profile ${PROFILE} --target ${TARGET} (commit ${hash})`);
  step("cargo build", "cargo", cargoArgs(), { env: { ENGINE_WEB_COMMIT: hash }, cwd: rootDir });
  const built = builtWasmPath(rootDir);
  if (!existsSync(built)) throw new BuildError(`cargo reported success but ${built} does not exist`);

  const work = deps.tempDir();
  try {
    say("[web-build] wasm-bindgen --target web");
    step("wasm-bindgen", "wasm-bindgen", bindgenArgs(built, work));
    const wasmPath = join(work, `${OUT_NAME}_bg.wasm`);
    const jsPath = join(work, `${OUT_NAME}.js`);
    if (!existsSync(wasmPath) || !existsSync(jsPath)) throw new BuildError(`wasm-bindgen wrote ${listFiles(work).join(", ") || "nothing"}; expected ${OUT_NAME}.js and ${OUT_NAME}_bg.wasm`);
    const rawBytes = statSync(wasmPath).size;

    let wasmOpt = null;
    const probe = optimize ? run("wasm-opt", ["--version"]) : { status: 1, out: "" };
    if (optimize && probe.status === 0) {
      wasmOpt = probe.out.trim();
      say(`[web-build] wasm-opt ${WASM_OPT_FLAGS.join(" ")}`);
      step("wasm-opt", "wasm-opt", [wasmPath, "-o", wasmPath, ...WASM_OPT_FLAGS]);
    } else {
      say(`[web-build] wasm-opt ${optimize ? "not installed" : "skipped"}: the module ships as wasm-bindgen wrote it`);
    }

    let twiggy = [];
    const tw = run("twiggy", ["top", "-n", "8", wasmPath]);
    if (tw.status === 0) {
      twiggy = tw.out.trim().split(/\r?\n/).slice(0, 12);
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
      files[rel] = { bytes: bytes.length, gzipBytes: gzipSync(bytes, { level: 9 }).length, sha256: sha256(bytes) };
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
      rawWasmBytes: rawBytes,
      tools: { rustc, wasmBindgen: cliVersion, wasmOpt },
      wasmOptFlags: wasmOpt ? WASM_OPT_FLAGS : null,
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
  const o = { out: null, commit: null, optimize: true, error: null };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (a === "--out") o.out = argv[++i] ?? null;
    else if (a === "--commit") o.commit = argv[++i] ?? null;
    else if (a === "--no-opt") o.optimize = false;
    else o.error = `unknown argument ${a}`;
  }
  if ((argv.includes("--out") && !o.out) || (argv.includes("--commit") && !o.commit)) o.error = "--out and --commit need a value";
  return o;
}

if (import.meta.main) {
  const a = parseArgs(process.argv.slice(2));
  if (a.error) {
    console.error(`web-build: ${a.error}\nusage: node tools/web-build.mjs [--out dist/web] [--commit <hash>] [--no-opt]`);
    process.exit(2);
  }
  try {
    build({ outDir: a.out ? resolve(a.out) : join(root, "dist", "web"), commit: a.commit, optimize: a.optimize });
  } catch (e) {
    if (!(e instanceof BuildError)) throw e;
    console.error(`web-build: ${e.message}`);
    process.exit(1);
  }
}
