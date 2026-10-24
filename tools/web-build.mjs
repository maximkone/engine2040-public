// Build the browser shell into dist/web (WEB4).
//   node tools/web-build.mjs [--out dist/web] [--commit <hash>] [--no-opt] [--opt-ab]
//   node tools/web-build.mjs --simd | --scalar
//   node tools/web-build.mjs --size-sweep [z,s,3] [--commit <hash>]
//   node tools/web-build.mjs --check-public <dir> [--allow-dir docs/] [--allow-file llms.txt]
// cargo builds crates/web for wasm32-unknown-unknown with the size-tuned `web` profile (root
// Cargo.toml), the wasm-bindgen CLI at the version Cargo.lock locks turns the module into
// engine.js + engine_bg.wasm, wasm-opt -Oz shrinks it when it is installed, and the page
// (crates/web/index.html) is written beside them with build.json: sizes, gzip and brotli
// sizes, the first-bundle total, SHA-256,
// tool versions, the commit baked into the module. Exit 0 = dist/web is complete and under
// the size limit; 1 = a step failed (the message says which and how to fix it).
// QW1: --size-sweep builds the web profile once per opt-level (CARGO_PROFILE_WEB_OPT_LEVEL; the same LTO and one codegen unit),
// prints raw, gzip and brotli bytes per level and names the smallest; it writes no dist/web. The winner goes into
// [profile.web]; the browser job of the next run proves the golden receipt on it.
// QW2: --opt-ab runs wasm-opt at -Oz, -Os and -O3 on the same module, prints the three sizes and ships the smallest;
// build.json keeps the table (wasmOptAb) and twiggy's top 20 items.
// WEB10: the site is public (the mirror is public), so a bundle that holds a file nobody listed does not ship: build() refuses it
// before it touches dist/web, and --check-public <dir> (the Pages workflow runs it on the assembled site) lists the strays and exits 1.
// BG10: two arms of the same build. The shipped one (OPT61) compiles the module with wasm SIMD128 and lets wasm-opt use it;
// --scalar builds the arm that came before it, so the two can be built from one tree and their replay digests compared.
// The receipt (build.json and the printed line) names the flag each arm was built with, so a dist says how it was made.
// Every step goes through `deps` (run a command, read and write files), so the test drives
// the whole build with a fake toolchain.
import { createHash } from "node:crypto";
import { spawnSync } from "node:child_process";
import { cpSync, existsSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, statSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { gzipSync } from "node:zlib";
import { ARENA_BUNDLE, brotliSize, firstLoadBytes, firstTierBytes } from "./web-budget.mjs";

export const TARGET = "wasm32-unknown-unknown";
export const PROFILE = "web";
export const PACKAGE = "engine-web";
export const OUT_NAME = "engine";
export const ENGINE_JS = "engine.js";
export const WASM_FILE = "engine_bg.wasm";
export const LOADER = "loader.js";
export const INDEX_FILE = "index.html";
export const BUILD_FILE = "build.json";
// PIN: the number of hex digits of a content hash in a file name and in a build id. 12 hex = 48 bits, far more than the
// handful of files a build ships; the full SHA-256 stays in the pin for the byte check.
export const HASH_HEX = 12;
export const PIN_SCHEMA = 2;
export const SIZE_LIMIT_BYTES = 8 * 1024 * 1024; // dist/web, before game content
// Rust 1.97 emits bulk-memory, nontrapping float-to-int and sign-extension instructions (and, since 1.82, multivalue and
// reference types); wasm-opt rejects a module that uses a feature it was not told about. All of them plus SIMD ship in
// every browser that has WebGPU, so enabling them lets the optimizer use them and nothing newer. SIMD is pre-approved
// by docs/HASH-LAW.md, which records wasm32-wasip1-simd128 MATCH.
export const WASM_OPT_FEATURES = ["--enable-bulk-memory", "--enable-nontrapping-float-to-int", "--enable-sign-ext", "--enable-multivalue", "--enable-reference-types", "--enable-mutable-globals", "--enable-simd"];
// The target-scoped flag that makes cargo build the module with wasm SIMD. It goes in the
// per-target env var, never in RUSTFLAGS: host build scripts and proc macros must not see a
// wasm feature they cannot use.
export const SIMD_RUSTFLAGS = "-C target-feature=+simd128";
// BG10: the two arms of the build. The SIMD arm (the shipped one, OPT61) turns the feature on
// for cargo and lets wasm-opt use it; the scalar arm is the build that came before it, so the
// two can be built from one tree and their replay digests compared. A scalar build simply does
// not set the flag.
export const wasmRustflagsEnv = (simd = true) => (simd ? { CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUSTFLAGS: SIMD_RUSTFLAGS } : {});
export const wasmOptFeatures = (simd = true) => (simd ? WASM_OPT_FEATURES : WASM_OPT_FEATURES.filter((f) => f !== "--enable-simd"));
export const WASM_OPT_FLAGS = ["-Oz", ...WASM_OPT_FEATURES];
export const WASM_OPT_AB = ["-Oz", "-Os", "-O3"]; // the levels --opt-ab compares; the first wins a tie
export const SWEEP_LEVELS = ["z", "s", "3"]; // the opt-levels --size-sweep builds; the first wins a tie
export const TWIGGY_ITEMS = 20;
// Fixed names: the entry page, the manifest and the isolation worker (a service worker's registration name cannot be
// hashed without breaking that registration). Everything the page runs carries its content hash.
// WEB14: coi-serviceworker.js is the cross-origin-isolation worker the page registers; without it the page is single-threaded.
export const COI_WORKER = "coi-serviceworker.js";
export const DIST_FILES = [INDEX_FILE, COI_WORKER, BUILD_FILE];
// The hashed names the build writes: one build is the only set a page can name.
export const DIST_PATTERNS = [
  new RegExp(`^loader\\.[0-9a-f]{${HASH_HEX}}\\.js$`),
  new RegExp(`^engine\\.[0-9a-f]{${HASH_HEX}}\\.js$`),
  new RegExp(`^engine_bg\\.[0-9a-f]{${HASH_HEX}}\\.wasm$`),
  new RegExp(`^assets/arena\\.[0-9a-f]{${HASH_HEX}}\\.e2bundle$`),
  new RegExp(`^assets/fighters\\.[0-9a-f]{${HASH_HEX}}\\.e2bundle$`),
];
// WEB17: the cooked arena kit the played page fetches and stages (`?fight`); cooked offline by
// `cargo run -p engine-assets --features import --bin arena_bundle`.
export const KIT_BUNDLE = ARENA_BUNDLE;
// WEB6: the cooked fighter rig, the file the page fetches for `?fight`. The
// entries keep their `cooked/` prefix, so they match `rig_cook::cooked_names`.
export const RIG_BUNDLE = "assets/fighters.e2bundle";
// What a published bundle may hold: the fixed files, the hashed names above and wasm-bindgen's snippets/ folder.
// The Pages workflow adds the handbook (docs/) and llms.txt for the assembled site.
export const PUBLIC_FILES = [...DIST_FILES];
export const PUBLIC_PATTERNS = [...DIST_PATTERNS];
export const PUBLIC_DIRS = [];

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

// The hashed file name of a logical build file: `engine_bg.wasm` -> `engine_bg.<12 hex>.wasm`.
export function hashedName(name, sha256hex) {
  const at = name.lastIndexOf(".");
  if (at <= 0) throw new BuildError(`${name} has no extension to put the content hash before`);
  return `${name.slice(0, at)}.${sha256hex.slice(0, HASH_HEX)}${name.slice(at)}`;
}

// The build id: the pin's logical names, hashed names and full hashes, sorted, hashed once. Two builds of the same tree
// get the same id and the same names; any changed input changes the id (and the changed file's name).
export function pinId(files) {
  const canonical = Object.keys(files).sort().map((logical) => `${logical} ${files[logical].name} ${files[logical].sha256}`).join("\n");
  return createHash("sha256").update(canonical).digest("hex").slice(0, HASH_HEX);
}

export function renderHtml(template, { commit, build, pin } = {}) {
  for (const hole of ["{{COMMIT}}", "{{BUILD}}", "{{PIN}}"]) {
    if (!template.includes(hole)) throw new BuildError(`crates/web/index.html lost its ${hole} placeholder`);
  }
  // A `<` inside the JSON would close the script element early; names hold none, but escape it anyway.
  const json = JSON.stringify(pin).replace(/</g, "\\u003c");
  return template.replaceAll("{{COMMIT}}", String(commit)).replaceAll("{{BUILD}}", String(build)).replaceAll("{{PIN}}", json);
}

// WEB14: the text of the cross-origin-isolation worker the page registers. A missing file is refused here,
// with the same message whether the source is gone or the copy failed, so a page never ships without it.
export function readCoiWorker(deps, rootDir) {
  const path = join(rootDir, "crates", "web", COI_WORKER);
  try {
    return deps.readText(path);
  } catch {
    throw new BuildError(`crates/web/${COI_WORKER} is missing (WEB14): the page would never be cross-origin isolated`);
  }
}

const sha256 = (buf) => createHash("sha256").update(buf).digest("hex");
const gzipBytes = (buf) => gzipSync(buf, { level: 9 }).length;

const tail = (text, n = 25) => String(text).trimEnd().split(/\r?\n/).slice(-n).join("\n");

// The files of `files` (relative, forward slashes) that are not on the public list: not a listed file, not a hashed name
// pattern and not under a listed folder. `allow` adds files, patterns and folders for one check.
export function nonPublicFiles(files, { files: more = [], dirs = [], patterns = [] } = {}) {
  const names = new Set([...PUBLIC_FILES, ...more]);
  const folders = [...PUBLIC_DIRS, ...dirs];
  const shapes = [...PUBLIC_PATTERNS, ...patterns];
  return files.filter((f) => !names.has(f) && !folders.some((d) => f.startsWith(d)) && !shapes.some((re) => re.test(f)));
}

// Throws a BuildError naming the files of `dir` that are not public.
export function checkPublic(dir, allow = {}) {
  const stray = nonPublicFiles(listFiles(dir), allow);
  if (stray.length) throw new BuildError(`${dir} holds ${stray.length} file(s) outside the public list: ${stray.slice(0, 10).join(", ")}${stray.length > 10 ? ", ..." : ""}`);
}

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
  if (r.status !== 0) throw new BuildError(`${label} failed (exit ${r.status}): ${cmd} ${args.join(" ")}\n${tail(r.out, 120)}`);
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
// `simd` picks the arm: the SIMD one (the shipped build) or the scalar one it is compared against.
// Returns { rawBytes (before wasm-opt), wasmOpt (its version or null), flags (what ran or null), ab (the table or null) }.
function compile({ rootDir, hash, optimize, ab = false, env = {}, simd = true }, work, deps) {
  const { run, say } = deps;
  const step = stepOf(run);
  const level = env.CARGO_PROFILE_WEB_OPT_LEVEL;
  const features = wasmOptFeatures(simd);
  say(`[web-build] cargo build --profile ${PROFILE} --target ${TARGET} (commit ${hash}${level ? `, opt-level ${level}` : ""}, ${simd ? "simd128" : "scalar"})`);
  step("cargo build", "cargo", cargoArgs(), { env: { ENGINE_WEB_COMMIT: hash, ...env, ...wasmRustflagsEnv(simd) }, cwd: rootDir });
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
    say(`[web-build] wasm-opt ${["-Oz", ...features].join(" ")}`);
    step("wasm-opt", "wasm-opt", [wasmPath, "-o", wasmPath, "-Oz", ...features]);
    return { rawBytes, wasmOpt, flags: ["-Oz", ...features], ab: null };
  }
  const table = WASM_OPT_AB.map((flag) => {
    const file = `${wasmPath}.${flag}`;
    step("wasm-opt", "wasm-opt", [wasmPath, "-o", file, flag, ...features]);
    const bytes = deps.readBytes(file);
    say(`[wasm-opt-ab] ${flag} raw=${bytes.length} gzip=${gzipBytes(bytes)}`);
    return { flag, bytes: bytes.length, gzipBytes: gzipBytes(bytes), file };
  });
  const best = table.reduce((a, b) => (b.bytes < a.bytes ? b : a));
  cpSync(best.file, wasmPath);
  for (const t of table) rmSync(t.file, { force: true });
  say(`[wasm-opt-ab] smallest raw: ${best.flag}`);
  return { rawBytes, wasmOpt, flags: [best.flag, ...features], ab: table.map(({ file, ...row }) => row) };
}

// QW1: one full web build per opt-level; the printed table is the receipt. Nothing is written to dist/web.
export function sizeSweep({ rootDir = root, commit = null, levels = SWEEP_LEVELS, optimize = true, simd = true } = {}, deps = realDeps()) {
  const { hash } = toolInfo({ rootDir, commit }, deps);
  const rows = levels.map((level) => {
    const work = deps.tempDir();
    try {
      const c = compile({ rootDir, hash, optimize, env: { CARGO_PROFILE_WEB_OPT_LEVEL: level }, simd }, work, deps);
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
// `simd` picks the arm: the SIMD one (the shipped build) or the scalar one it is compared against.
export function build({ rootDir = root, outDir = join(root, "dist", "web"), commit = null, optimize = true, ab = false, simd = true } = {}, deps = realDeps()) {
  const { run, say } = deps;
  const { cliVersion, hash, rustc } = toolInfo({ rootDir, commit }, deps);
  const work = deps.tempDir();
  try {
    const c = compile({ rootDir, hash, optimize, ab, simd }, work, deps);

    // The largest items of the shipped module (QW2): twiggy prints a header, TWIGGY_ITEMS rows and a remainder line.
    let twiggy = [];
    const tw = run("twiggy", ["top", "-n", String(TWIGGY_ITEMS), join(work, `${OUT_NAME}_bg.wasm`)]);
    if (tw.status === 0) {
      twiggy = tw.out.trim().split(/\r?\n/).slice(0, TWIGGY_ITEMS + 3);
      for (const l of twiggy) say(`[twiggy] ${l}`);
    }

    const template = deps.readText(join(rootDir, "crates", "web", INDEX_FILE));
    const worker = readCoiWorker(deps, rootDir);
    const loaderText = deps.readText(join(rootDir, "crates", "web", LOADER));

    // WEB17 and WEB6: both content bundles are checked before outDir is touched,
    // so a missing one leaves the last build standing instead of half writing a
    // new one. Each is cooked offline; this step ships what the tree holds.
    const kitSource = join(rootDir, "content", "arena", "kit.e2bundle");
    if (!existsSync(kitSource)) {
      throw new BuildError(`missing ${kitSource}: cook it with cargo run -p engine-assets --features import --bin arena_bundle`);
    }
    const rigSource = join(rootDir, "content", "fighters", "rig.e2bundle");
    if (!existsSync(rigSource)) {
      throw new BuildError(
        `missing ${rigSource}: cook it with cargo run -p engine-assets --features import --bin rig_cook -- --root content/fighters --out content/fighters, ` +
          `then pack it with cargo run -p engine-assets --bin e2bundle -- pack content/fighters content/fighters/rig.e2bundle --include cooked/`,
      );
    }

    // PIN: wasm-bindgen may write engine.js, engine_bg.wasm and type declarations. Any other file would be one the pin
    // cannot name (engine.js imports a JS snippet by a fixed relative path), so the build refuses it by name instead of
    // shipping a file no page can verify. No build has written one yet.
    const workFiles = listFiles(work).filter((rel) => !rel.endsWith(".d.ts"));
    const unexpected = workFiles.filter((rel) => rel !== ENGINE_JS && rel !== WASM_FILE);
    if (unexpected.length) {
      throw new BuildError(`wasm-bindgen wrote ${unexpected.join(", ")}; the build pin names ${ENGINE_JS} and ${WASM_FILE} only (a file the pin does not name cannot be verified)`);
    }
    const kitRaw = deps.readBytes(kitSource);
    const rigRaw = deps.readBytes(rigSource);
    // The bytes each pinned logical name ships as. The page template is pinned by its own bytes: the rendered page embeds
    // the pin, so it cannot hash itself; the template hash still catches a page cached across a deploy. The isolation
    // worker keeps its fixed name (a service worker is registered by name) and its bytes are pinned.
    const shipped = new Map([
      [ENGINE_JS, deps.readBytes(join(work, ENGINE_JS))],
      [WASM_FILE, deps.readBytes(join(work, WASM_FILE))],
      [LOADER, Buffer.from(loaderText, "utf8")],
      [KIT_BUNDLE, gzipSync(kitRaw, { level: 9 })], // WEB17: the cooked arena kit, whole, gzip
      [RIG_BUNDLE, gzipSync(rigRaw, { level: 9 })], // WEB6: the cooked rig, its entries under `cooked/`, gzip
      [INDEX_FILE, Buffer.from(template, "utf8")],
      [COI_WORKER, Buffer.from(worker, "utf8")],
    ]);
    const pinFiles = {};
    for (const logical of [...shipped.keys()].sort()) {
      const sum = sha256(shipped.get(logical));
      const fixed = logical === INDEX_FILE || logical === COI_WORKER;
      pinFiles[logical] = { name: fixed ? logical : hashedName(logical, sum), sha256: sum };
    }
    const build = pinId(pinFiles);
    const pin = { schema: PIN_SCHEMA, build, files: pinFiles };
    const page = renderHtml(template, { commit: hash, build, pin });

    // WEB10: every name the build writes is on the public list before outDir is touched.
    const planned = [INDEX_FILE, COI_WORKER, BUILD_FILE, ...Object.values(pinFiles).map((f) => f.name)];
    const stray = nonPublicFiles(planned);
    if (stray.length) throw new BuildError(`the build holds ${stray.length} file(s) outside the public list: ${stray.join(", ")}`);

    // Everything is ready: only now touch outDir.
    rmSync(outDir, { recursive: true, force: true });
    mkdirSync(outDir, { recursive: true });
    for (const [logical, bytes] of shipped) {
      if (logical === INDEX_FILE || logical === COI_WORKER) continue; // written under their fixed names below
      const to = join(outDir, pinFiles[logical].name);
      mkdirSync(dirname(to), { recursive: true });
      writeFileSync(to, bytes);
    }
    writeFileSync(join(outDir, INDEX_FILE), page);
    writeFileSync(join(outDir, COI_WORKER), worker); // WEB14: the page registers it; 200 on the site is one more deploy check

    // QW10: both cooked bundles ship gzip-compressed. dist/web has a hard 8 MiB build limit
    // and a static host will not compress a binary type it does not know, so the compression
    // happens at build time and crates/web/loader.js undoes it while the body streams. The
    // hashed names carry those bytes' hashes, so a page and a dist always agree.
    const kitName = pinFiles[KIT_BUNDLE].name;
    const rigName = pinFiles[RIG_BUNDLE].name;
    const wasmName = pinFiles[WASM_FILE].name;
    const rawBundle = new Map([[kitName, kitRaw], [rigName, rigRaw]]);
    const files = {};
    let totalBytes = 0;
    let firstBundleBytes = 0;
    for (const rel of listFiles(outDir)) {
      if (rel === BUILD_FILE) continue;
      const bytes = deps.readBytes(join(outDir, rel));
      const raw = rawBundle.get(rel) ?? bytes;
      const first = rel === kitName ? firstTierBytes(raw) : raw;
      files[rel] = {
        bytes: bytes.length,
        ...(raw === bytes ? {} : { rawBytes: raw.length }),
        firstBytes: first.length,
        gzipBytes: gzipBytes(bytes),
        brotliBytes: brotliSize(bytes),
        sha256: sha256(bytes),
      };
      totalBytes += bytes.length;
      // QW4: the page's first load, in the same words as web-budget.mjs: the shipped form of
      // every file except build.json, and of the arena bundle only to its tier-0 prefix.
      firstBundleBytes += firstLoadBytes(rel, bytes, kitName);
    }
    // The page names these bytes; the receipt above must hash to the same sums or the pin would lie. The page itself is
    // pinned by its template bytes: the rendered page embeds the pin, so it cannot be hashed into it.
    for (const [logical, entry] of Object.entries(pinFiles)) {
      if (logical === INDEX_FILE) continue;
      const receipt = files[entry.name];
      if (!receipt || receipt.sha256 !== entry.sha256) throw new BuildError(`the written ${entry.name} does not hash to the pin's ${entry.sha256}`);
    }
    const info = {
      schema: PIN_SCHEMA,
      build,
      commit: hash,
      target: TARGET,
      profile: PROFILE,
      pin: pinFiles,
      files,
      firstBundleBytes,
      totalBytes,
      limitBytes: SIZE_LIMIT_BYTES,
      underLimit: totalBytes < SIZE_LIMIT_BYTES,
      rawWasmBytes: c.rawBytes,
      simd,
      wasmRustflags: simd ? SIMD_RUSTFLAGS : "",
      tools: { rustc, wasmBindgen: cliVersion, wasmOpt: c.wasmOpt },
      wasmOptFlags: c.flags,
      wasmOptAb: c.ab,
      twiggy,
    };
    // build.json lists the other files; it cannot hold its own hash. The page embeds `pin`, so the loader can compare
    // the two before it imports anything.
    writeFileSync(join(outDir, BUILD_FILE), `${JSON.stringify(info, null, 2)}\n`);
    // The numbers print before the gate, so an over-limit build still says how far over it is.
    say(
      `[web-build] dist/web ${totalBytes} bytes (limit ${SIZE_LIMIT_BYTES}); pin ${build}; wasm ${wasmName} ${files[wasmName].bytes} bytes, ${files[wasmName].gzipBytes} gzip; ` +
        `arena bundle ${kitName} ${files[kitName].bytes} gzip (${files[kitName].rawBytes} raw, tier 0 at ${files[kitName].firstBytes}); ` +
        `rig ${rigName} ${files[rigName].bytes} gzip (${files[rigName].rawBytes} raw); first_bundle ${firstBundleBytes} bytes; ` +
        `wasm rustflags ${simd ? SIMD_RUSTFLAGS : "none (scalar)"}`,
    );
    if (!info.underLimit) {
      rmSync(outDir, { recursive: true, force: true }); // no half-shipped build over the limit
      throw new BuildError(`dist/web is ${totalBytes} bytes, over the ${SIZE_LIMIT_BYTES} byte limit`);
    }
    return info;
  } finally {
    rmSync(work, { recursive: true, force: true });
  }
}

export function parseArgs(argv) {
  const o = { out: null, commit: null, optimize: true, ab: false, sweep: null, checkPublic: null, allowDirs: [], allowFiles: [], simd: true, error: null };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (a === "--out") o.out = argv[++i] ?? null;
    else if (a === "--commit") o.commit = argv[++i] ?? null;
    else if (a === "--no-opt") o.optimize = false;
    else if (a === "--opt-ab") o.ab = true;
    else if (a === "--simd") o.simd = true;
    else if (a === "--scalar") o.simd = false;
    else if (a === "--check-public") o.checkPublic = argv[++i] ?? null;
    else if (a === "--allow-dir") o.allowDirs.push(argv[++i] ?? "");
    else if (a === "--allow-file") o.allowFiles.push(argv[++i] ?? "");
    else if (a === "--size-sweep") o.sweep = argv[i + 1] && !argv[i + 1].startsWith("--") ? argv[++i].split(",") : SWEEP_LEVELS;
    else o.error = `unknown argument ${a}`;
  }
  if ((argv.includes("--out") && !o.out) || (argv.includes("--commit") && !o.commit)) o.error = "--out and --commit need a value";
  if ((argv.includes("--check-public") && !o.checkPublic) || o.allowDirs.includes("") || o.allowFiles.includes("")) o.error = "--check-public, --allow-dir and --allow-file need a value";
  if (argv.includes("--simd") && argv.includes("--scalar")) o.error = "--simd and --scalar are two different builds; pick one";
  if (o.sweep && !o.sweep.every((l) => /^[0-3sz]$/.test(l))) o.error = "--size-sweep takes opt-levels from 0 1 2 3 s z, comma separated";
  return o;
}

if (import.meta.main) {
  const a = parseArgs(process.argv.slice(2));
  if (a.error) {
    console.error(`web-build: ${a.error}\nusage: node tools/web-build.mjs [--out dist/web] [--commit <hash>] [--no-opt] [--opt-ab] [--simd | --scalar] | --size-sweep [z,s,3] | --check-public <dir> [--allow-dir d/] [--allow-file f]`);
    process.exit(2);
  }
  try {
    if (a.checkPublic) {
      checkPublic(resolve(a.checkPublic), { files: a.allowFiles, dirs: a.allowDirs });
      console.log(`web-build: ${a.checkPublic} holds only public files`);
    } else if (a.sweep) sizeSweep({ commit: a.commit, levels: a.sweep, optimize: a.optimize, simd: a.simd });
    else build({ outDir: a.out ? resolve(a.out) : join(root, "dist", "web"), commit: a.commit, optimize: a.optimize, ab: a.ab, simd: a.simd });
  } catch (e) {
    if (!(e instanceof BuildError)) throw e;
    console.error(`web-build: ${e.message}`);
    process.exit(1);
  }
}
