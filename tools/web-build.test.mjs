// web-build drills: a fake toolchain (cargo, wasm-bindgen, wasm-opt) writes the files the real
// ones would, so the test checks what build() does with them: the dist/web file list, the
// sizes and hashes in build.json, the commit in the page, the flags each tool gets, and what
// happens when a step fails. No compiler runs.
//   node --test tools/web-build.test.mjs
import test from "node:test";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import {
  BuildError, DIST_FILES, SIZE_LIMIT_BYTES, SWEEP_LEVELS, TARGET, WASM_OPT_FEATURES, WASM_OPT_FLAGS, bindgenArgs, build, builtWasmPath, cargoArgs,
  listFiles, lockedVersion, parseArgs, realDeps, renderHtml, sizeSweep,
} from "./web-build.mjs";

const REPO = join(dirname(fileURLToPath(import.meta.url)), "..");
const TEMPLATE = readFileSync(join(REPO, "crates", "web", "index.html"), "utf8");
const LOADER = readFileSync(join(REPO, "crates", "web", "loader.js"), "utf8");
const LOCK = 'name = "wasm-bindgen"\nversion = "0.2.128"\nsource = "registry+x"\n\nname = "wgpu"\nversion = "30.0.1"\n';
const sha = (b) => createHash("sha256").update(b).digest("hex");

// A scratch repo root with the two inputs build() reads, and a scratch output directory.
function scratch(lock = LOCK) {
  const dir = mkdtempSync(join(tmpdir(), "web-build-test-"));
  mkdirSync(join(dir, "crates", "web"), { recursive: true });
  writeFileSync(join(dir, "Cargo.lock"), lock);
  writeFileSync(join(dir, "crates", "web", "index.html"), TEMPLATE);
  writeFileSync(join(dir, "crates", "web", "loader.js"), LOADER);
  return { dir, out: join(dir, "dist", "web"), done: () => rmSync(dir, { recursive: true, force: true }) };
}

// The fake machine. `wasmBytes` is what cargo "compiles"; `has` says which tools exist.
function toolchain(rootDir, { wasmBytes = 1000, bindgen = "0.2.128", has = ["wasm-bindgen", "wasm-opt"], cargoFails = false, optFactor = 0.5, twiggy = false, levelBytes = {}, flagFactor = {} } = {}) {
  const calls = [];
  const wasm = (n) => Buffer.alloc(n, 7);
  const run = (cmd, args, o = {}) => {
    calls.push({ cmd, args, env: o.env ?? {}, cwd: o.cwd });
    const key = `${cmd} ${args[0]}`;
    if (cmd === "rustc") return { status: 0, out: "rustc 1.97.1 (fake)\n" };
    if (cmd === "wasm-bindgen" && args[0] === "--version") return has.includes("wasm-bindgen") ? { status: 0, out: `wasm-bindgen ${bindgen}\n` } : { status: 127, out: "" };
    if (cmd === "wasm-opt" && args[0] === "--version") return has.includes("wasm-opt") ? { status: 0, out: "wasm-opt version 119\n" } : { status: 127, out: "" };
    if (cmd === "twiggy") return twiggy ? { status: 0, out: " Shallow Bytes | Item\n 400 | data[0]\n" } : { status: 127, out: "" };
    if (key === "cargo build") {
      if (cargoFails) return { status: 101, out: "error[E0599]: no variant named Wgsl\nerror: could not compile `engine-web`\n" };
      mkdirSync(dirname(builtWasmPath(rootDir)), { recursive: true });
      writeFileSync(builtWasmPath(rootDir), wasm(levelBytes[o.env?.CARGO_PROFILE_WEB_OPT_LEVEL] ?? wasmBytes));
      return { status: 0, out: "" };
    }
    if (cmd === "wasm-bindgen") {
      const outDir = args[args.indexOf("--out-dir") + 1];
      const name = args[args.indexOf("--out-name") + 1];
      const input = readFileSync(args[args.length - 1]);
      writeFileSync(join(outDir, `${name}.js`), `// glue\nexport default function init() {}\n`);
      writeFileSync(join(outDir, `${name}_bg.wasm`), input);
      return { status: 0, out: "" };
    }
    if (cmd === "wasm-opt") {
      const flag = args.find((a) => /^-O[a-z0-9]$/.test(a));
      writeFileSync(args[args.indexOf("-o") + 1], wasm(Math.floor(readFileSync(args[0]).length * (flagFactor[flag] ?? optFactor))));
      return { status: 0, out: "" };
    }
    return { status: 127, out: `unexpected ${cmd} ${args.join(" ")}` };
  };
  return { run, calls };
}

const quiet = (run) => ({ ...realDeps(), run, say: () => {} });

test("lockedVersion reads one locked version and refuses none or several", () => {
  assert.equal(lockedVersion(LOCK, "wasm-bindgen"), "0.2.128");
  assert.equal(lockedVersion(LOCK.replace(/\n/g, "\r\n"), "wgpu"), "30.0.1");
  assert.throws(() => lockedVersion(LOCK, "serde"), BuildError);
  const two = `${LOCK}\nname = "wasm-bindgen"\nversion = "0.2.129"\n`;
  assert.throws(() => lockedVersion(two, "wasm-bindgen"), /2 versions of wasm-bindgen/);
});

test("the cargo line builds only the web crate, locked, with the web profile for the browser target", () => {
  const a = cargoArgs();
  assert.deepEqual(a, ["build", "--locked", "--profile", "web", "--target", TARGET, "-p", "engine-web"]);
  assert.ok(!a.includes("--workspace") && !a.includes("--release"));
  assert.equal(TARGET, "wasm32-unknown-unknown");
});

test("wasm-bindgen is asked for the web target with fixed output names", () => {
  assert.deepEqual(bindgenArgs("in.wasm", "out"), ["--target", "web", "--no-typescript", "--out-dir", "out", "--out-name", "engine", "in.wasm"]);
});

test("build writes exactly the dist/web files, with real sizes and hashes in build.json", () => {
  const s = scratch();
  try {
    const t = toolchain(s.dir, { wasmBytes: 4000 });
    const info = build({ rootDir: s.dir, outDir: s.out, commit: "abc123def456" }, quiet(t.run));
    assert.deepEqual(listFiles(s.out), [...DIST_FILES].sort());
    const onDisk = JSON.parse(readFileSync(join(s.out, "build.json"), "utf8"));
    assert.deepEqual(onDisk, info);
    for (const name of ["index.html", "loader.js", "engine.js", "engine_bg.wasm"]) {
      const bytes = readFileSync(join(s.out, name));
      assert.equal(info.files[name].bytes, bytes.length, name);
      assert.equal(info.files[name].sha256, sha(bytes), name);
      assert.ok(info.files[name].gzipBytes > 0, name);
    }
    assert.ok(!("build.json" in info.files));
    assert.equal(info.totalBytes, Object.values(info.files).reduce((n, f) => n + f.bytes, 0));
    assert.equal(info.rawWasmBytes, 4000);
    assert.equal(info.files["engine_bg.wasm"].bytes, 2000, "wasm-opt output is what ships");
    assert.equal(info.underLimit, true);
    assert.equal(info.limitBytes, SIZE_LIMIT_BYTES);
    assert.equal(info.commit, "abc123def456");
    assert.deepEqual(info.tools, { rustc: "rustc 1.97.1 (fake)", wasmBindgen: "0.2.128", wasmOpt: "wasm-opt version 119" });
  } finally {
    s.done();
  }
});

test("the commit reaches the module (cargo env) and the page", () => {
  const s = scratch();
  try {
    const t = toolchain(s.dir);
    build({ rootDir: s.dir, outDir: s.out, commit: "feedface0001" }, quiet(t.run));
    const cargo = t.calls.find((c) => c.cmd === "cargo");
    assert.equal(cargo.env.ENGINE_WEB_COMMIT, "feedface0001");
    assert.deepEqual(cargo.args, cargoArgs());
    const html = readFileSync(join(s.out, "index.html"), "utf8");
    assert.ok(html.includes('data-commit="feedface0001"'));
    assert.ok(!html.includes("{{"), "a placeholder survived");
  } finally {
    s.done();
  }
});

test("wasm-opt gets the size flag and the instruction-set flags Rust 1.97 needs", () => {
  const s = scratch();
  try {
    const t = toolchain(s.dir);
    const info = build({ rootDir: s.dir, outDir: s.out, commit: "c" }, quiet(t.run));
    const opt = t.calls.find((c) => c.cmd === "wasm-opt" && c.args[0] !== "--version");
    for (const f of ["-Oz", "--enable-bulk-memory", "--enable-nontrapping-float-to-int", "--enable-sign-ext", "--enable-multivalue", "--enable-reference-types"]) assert.ok(opt.args.includes(f), f);
    assert.ok(!opt.args.includes("--all-features"), "only features every WebGPU browser has");
    assert.deepEqual(info.wasmOptFlags, WASM_OPT_FLAGS);
  } finally {
    s.done();
  }
});

test("without wasm-opt the build still ships and says so in build.json", () => {
  const s = scratch();
  try {
    const t = toolchain(s.dir, { has: ["wasm-bindgen"], wasmBytes: 3000 });
    const info = build({ rootDir: s.dir, outDir: s.out, commit: "c" }, quiet(t.run));
    assert.equal(info.tools.wasmOpt, null);
    assert.equal(info.wasmOptFlags, null);
    assert.equal(info.files["engine_bg.wasm"].bytes, 3000);
    assert.ok(!t.calls.some((c) => c.cmd === "wasm-opt" && c.args[0] !== "--version"));
    const skipped = toolchain(s.dir, { wasmBytes: 3000 });
    const info2 = build({ rootDir: s.dir, outDir: s.out, commit: "c", optimize: false }, quiet(skipped.run));
    assert.equal(info2.tools.wasmOpt, null);
  } finally {
    s.done();
  }
});

test("twiggy's top lines are kept when it is installed", () => {
  const s = scratch();
  try {
    const t = toolchain(s.dir, { twiggy: true });
    const info = build({ rootDir: s.dir, outDir: s.out, commit: "c" }, quiet(t.run));
    assert.equal(info.twiggy.length, 2);
    assert.deepEqual(t.calls.find((c) => c.cmd === "twiggy").args.slice(0, 3), ["top", "-n", "20"], "the receipt names the top 20 items");
    assert.match(info.twiggy[1], /data\[0\]/);
  } finally {
    s.done();
  }
});

test("QW2: --opt-ab runs -Oz, -Os and -O3 on the same module, ships the smallest and keeps the table", () => {
  const s = scratch();
  try {
    const t = toolchain(s.dir, { wasmBytes: 1000, flagFactor: { "-Oz": 0.7, "-Os": 0.5, "-O3": 0.9 } });
    const lines = [];
    const info = build({ rootDir: s.dir, outDir: s.out, commit: "c", ab: true }, { ...realDeps(), run: t.run, say: (l) => lines.push(l) });
    const opts = t.calls.filter((c) => c.cmd === "wasm-opt" && c.args[0] !== "--version");
    assert.equal(opts.length, 3);
    for (const o of opts) for (const f of WASM_OPT_FEATURES) assert.ok(o.args.includes(f), f);
    assert.deepEqual(info.wasmOptAb.map((r) => [r.flag, r.bytes]), [["-Oz", 700], ["-Os", 500], ["-O3", 900]]);
    assert.ok(info.wasmOptAb.every((r) => r.gzipBytes > 0));
    assert.equal(info.files["engine_bg.wasm"].bytes, 500, "the smallest raw module ships");
    assert.deepEqual(info.wasmOptFlags, ["-Os", ...WASM_OPT_FEATURES]);
    assert.ok(lines.some((l) => /^\[wasm-opt-ab\] -Os raw=500 gzip=\d+$/.test(l)));
    assert.deepEqual(listFiles(s.out), [...DIST_FILES].sort(), "no candidate file leaks into dist/web");
    assert.equal(build({ rootDir: s.dir, outDir: s.out, commit: "c" }, quiet(toolchain(s.dir).run)).wasmOptAb, null, "off by default");
  } finally {
    s.done();
  }
});

test("QW1: --size-sweep builds each opt-level with the profile env, prints a table and names the smallest raw wasm", () => {
  const s = scratch();
  try {
    const t = toolchain(s.dir, { levelBytes: { z: 1000, s: 800, 3: 2000 }, optFactor: 0.5 });
    const lines = [];
    const out = sizeSweep({ rootDir: s.dir, commit: "c" }, { ...realDeps(), run: t.run, say: (l) => lines.push(l) });
    const cargos = t.calls.filter((c) => c.cmd === "cargo");
    assert.deepEqual(cargos.map((c) => c.env.CARGO_PROFILE_WEB_OPT_LEVEL), SWEEP_LEVELS);
    for (const c of cargos) assert.deepEqual(c.args, cargoArgs(), "the same profile and package at every level");
    assert.deepEqual(out.rows.map((r) => [r.level, r.bytes, r.preOptBytes]), [["z", 500, 1000], ["s", 400, 800], ["3", 1000, 2000]]);
    assert.ok(out.rows.every((r) => r.gzipBytes > 0 && r.brotliBytes > 0));
    assert.equal(out.best, "s");
    assert.equal(lines.filter((l) => /^\[size-sweep\] opt-level=\S raw=\d+ pre-opt=\d+ gzip=\d+ brotli=\d+$/.test(l)).length, 3);
    assert.match(lines.at(-1), /smallest raw wasm: opt-level s \(400 bytes\)/);
    assert.ok(!existsSync(s.out), "the sweep writes no dist/web");
    const tie = sizeSweep({ rootDir: s.dir, commit: "c", levels: ["z", "s"] }, quiet(toolchain(s.dir).run));
    assert.equal(tie.best, "z", "a tie keeps the first level");
  } finally {
    s.done();
  }
});

test("a wasm-bindgen CLI of another version stops the build before cargo runs and names the fix", () => {
  const s = scratch();
  try {
    const t = toolchain(s.dir, { bindgen: "0.2.129" });
    assert.throws(() => build({ rootDir: s.dir, outDir: s.out, commit: "c" }, quiet(t.run)), (e) => {
      assert.ok(e instanceof BuildError);
      assert.match(e.message, /0\.2\.129 but Cargo\.lock locks 0\.2\.128/);
      assert.match(e.message, /cargo install wasm-bindgen-cli --version 0\.2\.128 --locked/);
      return true;
    });
    assert.ok(!t.calls.some((c) => c.cmd === "cargo"), "cargo must not run with a mismatched CLI");
    assert.ok(!existsSync(s.out));
  } finally {
    s.done();
  }
});

test("a missing wasm-bindgen CLI is reported with the install line", () => {
  const s = scratch();
  try {
    const t = toolchain(s.dir, { has: [] });
    assert.throws(() => build({ rootDir: s.dir, outDir: s.out, commit: "c" }, quiet(t.run)), /not installed.*cargo install wasm-bindgen-cli --version 0\.2\.128/);
  } finally {
    s.done();
  }
});

test("a failed cargo build throws with the compiler's words and leaves an older dist/web alone", () => {
  const s = scratch();
  try {
    mkdirSync(s.out, { recursive: true });
    writeFileSync(join(s.out, "old.txt"), "keep");
    const t = toolchain(s.dir, { cargoFails: true });
    assert.throws(() => build({ rootDir: s.dir, outDir: s.out, commit: "c" }, quiet(t.run)), /cargo build failed \(exit 101\)[\s\S]*no variant named Wgsl/);
    assert.equal(readFileSync(join(s.out, "old.txt"), "utf8"), "keep");
  } finally {
    s.done();
  }
});

test("a build over the size limit fails and ships nothing", () => {
  const s = scratch();
  try {
    const t = toolchain(s.dir, { has: ["wasm-bindgen"], wasmBytes: SIZE_LIMIT_BYTES + 1 });
    assert.throws(() => build({ rootDir: s.dir, outDir: s.out, commit: "c" }, quiet(t.run)), /over the 8388608 byte limit/);
    assert.ok(!existsSync(s.out));
  } finally {
    s.done();
  }
});

test("a wasm-bindgen run that writes the wrong files is caught", () => {
  const s = scratch();
  try {
    const t = toolchain(s.dir);
    const run = (cmd, args, o) => (cmd === "wasm-bindgen" && args[0] === "--target" ? { status: 0, out: "" } : t.run(cmd, args, o));
    assert.throws(() => build({ rootDir: s.dir, outDir: s.out, commit: "c" }, quiet(run)), /expected engine\.js and engine_bg\.wasm/);
  } finally {
    s.done();
  }
});

test("the page contract: canvas, hud, report, fatal message with the browser versions, module import", () => {
  const html = renderHtml(TEMPLATE, "abc");
  assert.match(html, /<canvas id="canvas"/);
  assert.match(html, /id="hud"/);
  assert.match(html, /<pre id="golden"/);
  assert.match(html, /body\[data-golden="mismatch"\] #golden/);
  assert.match(html, /<pre id="report"/);
  assert.match(html, /id="fatal"[^>]*role="alert"/);
  assert.match(html, /id="fatal-reason"/);
  assert.ok(html.includes('<pre id="input">input: kb=0 mouse=0 pad=0</pre>'));
  assert.ok(html.includes('<pre id="audio">audio: state=suspended</pre>'));
  assert.match(html, /<script type="module">/);
  assert.match(html, /from "\.\/engine\.js"/);
  // QW3/QW12: the module is fetched by loader.js, not by init's own fetch; its failure reaches the fatal reason.
  assert.match(html, /from "\.\/loader\.js"/);
  assert.match(html, /await loadEngine\(import\.meta\.url\)/);
  assert.match(html, /init\(\{ module_or_path: loaded\.module \}\)/);
  assert.ok(!/await init\(\);/.test(html), "init must not fetch the wasm itself");
  assert.match(html, /<pre id="load">/);
  assert.match(html, /"The engine did not start: " \+ e/);
  assert.match(html, /run\("canvas"\)/);
  for (const v of ["Chrome 113", "Edge 113", "Safari 26", "Firefox 141"]) assert.ok(html.includes(v), `fatal message lost ${v}`);
  assert.match(html, /body\[data-state="fatal"\] #fatal \{ display: block/);
  assert.match(html, /<body data-state="starting"/);
  // Nothing is fetched from another host: the page works from a file server alone.
  assert.ok(!/(?:src|href)="https?:/.test(html));
  assert.throws(() => renderHtml("<html></html>", "abc"), /lost its \{\{COMMIT\}\} placeholder/);
});

test("the real inputs build() reads exist: lock, template, crate manifest, web profile", () => {
  const lock = readFileSync(join(REPO, "Cargo.lock"), "utf8");
  assert.match(lockedVersion(lock, "wasm-bindgen"), /^0\.2\.\d+$/);
  const manifest = readFileSync(join(REPO, "crates", "web", "Cargo.toml"), "utf8");
  assert.match(manifest, /name = "engine-web"/);
  assert.match(manifest, /crate-type = \["cdylib", "rlib"\]/);
  const root = readFileSync(join(REPO, "Cargo.toml"), "utf8").replace(/\r\n/g, "\n");
  const web = root.split("[profile.web]")[1]?.split(/\n\[/)[0] ?? "";
  for (const line of ['inherits = "release"', 'opt-level = "z"', "lto = true", "codegen-units = 1", 'panic = "abort"']) assert.ok(web.includes(line), `[profile.web] lost ${line}`);
  assert.ok(!/overflow-checks\s*=\s*false/.test(web), "wasm must keep overflow checks");
  // Native builds are unchanged: the release profile has no panic override and keeps opt-level 3.
  const release = root.split("[profile.release]")[1].split(/\n\[/)[0];
  assert.match(release, /opt-level = 3/);
  assert.ok(!/panic/.test(release));
});

test("parseArgs", () => {
  assert.deepEqual(parseArgs([]), { out: null, commit: null, optimize: true, ab: false, sweep: null, error: null });
  assert.deepEqual(parseArgs(["--out", "x", "--commit", "abc", "--no-opt", "--opt-ab"]), { out: "x", commit: "abc", optimize: false, ab: true, sweep: null, error: null });
  assert.deepEqual(parseArgs(["--size-sweep"]).sweep, ["z", "s", "3"]);
  assert.deepEqual(parseArgs(["--size-sweep", "s,2", "--commit", "a"]).sweep, ["s", "2"]);
  assert.match(parseArgs(["--size-sweep", "fast"]).error, /opt-levels/);
  assert.match(parseArgs(["--wat"]).error, /unknown argument/);
  assert.match(parseArgs(["--out"]).error, /need a value/);
});

test("the workflow file keeps its step names plain YAML and runs the two tools", () => {
  const yml = readFileSync(join(REPO, ".github", "workflows", "web-build.yml"), "utf8");
  // A ": " inside an unquoted scalar makes GitHub reject the whole file ("workflow file issue").
  for (const m of yml.matchAll(/^\s*- name: (.+)$/gm)) {
    const name = m[1].trim();
    assert.ok(/^["']/.test(name) || !name.includes(": "), `step name needs quotes: ${name}`);
  }
  assert.match(yml, /branches: \["web-build-\*"\]/);
  assert.match(yml, /node tools\/web-build\.mjs/);
  assert.match(yml, /node tools\/web-browse\.mjs/);
  assert.match(yml, /wasm32-unknown-unknown/);
  assert.match(yml, /application\/wasm/);
});
