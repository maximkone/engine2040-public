// Web-target doctor: what an AI needs to build, run and prove the browser (wasm) target.
//   node tools/web-doctor.mjs
// One line per piece: `OK <name> <version>`, or `MISSING <name>` followed by `  fix: <command>`.
// The last line is `web target: READY` (exit 0) or `web target: NOT READY n missing` (exit 1).
// Probes are read-only: PATH lookups, version queries, `rustup target list`, `gh auth status`.
// Browser binaries are never launched (chrome.exe --version hands off to a running browser);
// a browser's version comes from its file properties. Nothing is installed here: fix lines
// are for the operator. Tests inject a fake environment (tools/web-doctor.test.mjs).
import { spawnSync } from "node:child_process";
import { existsSync, readFileSync, statSync } from "node:fs";
import { delimiter, dirname, join, resolve, win32 } from "node:path";
import { fileURLToPath } from "node:url";

export const MIN_NODE_MAJOR = 24; // .github/workflows/wasm-probe.yml pins node-version 24
export const MIN_BROWSER_MAJOR = 113; // WebGPU is on by default from Chrome and Edge 113
export const RUST_TARGETS = ["wasm32-unknown-unknown", "wasm32-wasip1"];
const BROWSER_NAMES = ["chrome", "google-chrome", "chromium", "msedge"];

const majorOf = (version) => Number(String(version).split(".")[0]);
const firstMatch = (text, re) => text.match(re)?.[1] ?? null;
const fileName = (path) => path.split(/[\\/]/).pop();

// Real environment. Each probe has a fake twin in the test; nothing else touches the machine.
export function realWhich(name, env = process.env) {
  const exts = env.PATHEXT ? env.PATHEXT.split(";").filter(Boolean) : [""];
  for (const dir of (env.PATH ?? "").split(delimiter).filter(Boolean)) {
    for (const ext of exts) {
      const path = join(dir, name + ext);
      if (existsSync(path) && statSync(path).isFile()) return path;
    }
  }
  return null;
}

export function realRun(cmd, args) {
  const r = spawnSync(cmd, args, { encoding: "utf8", timeout: 30_000, windowsHide: true, maxBuffer: 8 * 1024 * 1024 });
  return { ok: !r.error && r.status === 0, out: `${r.stdout ?? ""}${r.stderr ?? ""}`.trim() };
}

// The file version of a Windows binary, read without running it.
export function realFileVersion(path) {
  const quoted = path.replace(/'/g, "''");
  const r = realRun("powershell", ["-NoProfile", "-NonInteractive", "-Command", `(Get-Item -LiteralPath '${quoted}').VersionInfo.ProductVersion`]);
  return r.ok && r.out ? r.out.split(/\r?\n/)[0].trim() : null;
}

// Chrome and Edge install paths that are not on PATH (Windows).
export function browserInstallPaths(env = process.env) {
  const programFiles = env.ProgramFiles ?? "C:\\Program Files";
  const programFilesX86 = env["ProgramFiles(x86)"] ?? "C:\\Program Files (x86)";
  const paths = [
    win32.join(programFiles, "Google", "Chrome", "Application", "chrome.exe"),
    win32.join(programFilesX86, "Google", "Chrome", "Application", "chrome.exe"),
    win32.join(programFiles, "Microsoft", "Edge", "Application", "msedge.exe"),
    win32.join(programFilesX86, "Microsoft", "Edge", "Application", "msedge.exe"),
  ];
  if (env.LOCALAPPDATA) paths.push(win32.join(env.LOCALAPPDATA, "Google", "Chrome", "Application", "chrome.exe"));
  return paths;
}

export function realEnv(root = resolve(dirname(fileURLToPath(import.meta.url)), "..")) {
  return {
    which: (name) => realWhich(name),
    run: realRun,
    fileVersion: realFileVersion,
    exists: (path) => existsSync(path),
    readFile: (path) => {
      try {
        return readFileSync(path, "utf8");
      } catch {
        return null;
      }
    },
    nodeVersion: process.versions.node,
    browserPaths: browserInstallPaths(),
    lockPath: join(root, "Cargo.lock"),
  };
}

// Pure: every piece is decided by `env`. Returns the report lines and the missing count.
export function checkWebTarget(env) {
  const lines = [];
  let missing = 0;
  const ok = (name, version) => lines.push(`OK ${name} ${version}`);
  const miss = (name, fix, detail = "") => {
    missing += 1;
    lines.push(`MISSING ${name}${detail ? ` ${detail}` : ""}`, `  fix: ${fix}`);
  };
  // A tool is OK when it is on PATH and its version parses; otherwise MISSING with the given fix.
  const tool = (name, args, re, fix) => {
    if (!env.which(name)) return miss(name, fix);
    const r = env.run(name, args);
    const version = r.ok ? firstMatch(r.out, re) : null;
    return version ? ok(name, version) : miss(name, fix, "(version unreadable)");
  };

  if (majorOf(env.nodeVersion) >= MIN_NODE_MAJOR) ok("node", env.nodeVersion);
  else miss("node", "winget upgrade --id OpenJS.NodeJS.LTS -e", `${env.nodeVersion} (need ${MIN_NODE_MAJOR}+)`);

  // Rust targets come from rustup; each OK line carries the rustc version.
  const hasRustup = Boolean(env.which("rustup"));
  if (!hasRustup) miss("rustup", "winget install --id Rustlang.Rustup -e");
  const list = hasRustup ? env.run("rustup", ["target", "list", "--installed"]) : { ok: false, out: "" };
  const installed = new Set(list.out.split(/\r?\n/).map((s) => s.trim()));
  const rustc = hasRustup ? firstMatch(env.run("rustc", ["--version"]).out, /rustc (\d+\.\d+\.\d+)/) ?? "unknown" : "unknown";
  for (const target of RUST_TARGETS) {
    if (installed.has(target)) ok(`rust-target ${target}`, rustc);
    else {
      const why = !hasRustup ? "(rustup not found)" : list.ok ? "" : "(rustup target list failed)";
      miss(`rust-target ${target}`, `rustup target add ${target}`, why);
    }
  }

  // wasm-bindgen CLI must match the wasm-bindgen crate version pinned in Cargo.lock.
  const want = firstMatch(env.readFile(env.lockPath) ?? "", /name = "wasm-bindgen"\r?\nversion = "([^"]+)"/);
  const wantNote = want ? ` (Cargo.lock needs ${want})` : "";
  const install = want ? `cargo install wasm-bindgen-cli --version ${want} --locked` : "cargo install wasm-bindgen-cli --locked";
  if (!env.which("wasm-bindgen")) miss("wasm-bindgen", install, wantNote.trim());
  else {
    const r = env.run("wasm-bindgen", ["--version"]);
    const have = r.ok ? firstMatch(r.out, /wasm-bindgen (\d+\.\d+\.\d+)/) : null;
    if (have && (!want || have === want)) ok("wasm-bindgen", have);
    else miss("wasm-bindgen", want ? `${install} --force` : install, `${have ?? "(version unreadable)"}${wantNote}`);
  }

  tool("wasm-opt", ["--version"], /version (\d+)/, "winget install --id WebAssembly.Binaryen -e");
  tool("wasmtime", ["--version"], /(\d+\.\d+\.\d+)/, "winget install --id BytecodeAlliance.Wasmtime -e");

  // WebGPU needs Chrome or Edge 113+. The first candidate that qualifies is OK.
  const candidates = [];
  for (const name of BROWSER_NAMES) {
    const path = env.which(name);
    if (path) candidates.push({ name, path });
  }
  for (const path of env.browserPaths) {
    if (env.exists(path)) candidates.push({ name: fileName(path).replace(/\.exe$/i, ""), path });
  }
  let firstSeen = null;
  let browserOk = false;
  for (const c of candidates) {
    const version = env.fileVersion(c.path);
    if (version && majorOf(version) >= MIN_BROWSER_MAJOR) {
      ok(c.name, version);
      browserOk = true;
      break;
    }
    firstSeen ??= { ...c, version };
  }
  if (!browserOk) {
    if (!firstSeen) miss("chrome", "winget install --id Google.Chrome -e", `(Chrome or Edge ${MIN_BROWSER_MAJOR}+ needed for WebGPU)`);
    else {
      const id = firstSeen.name === "msedge" ? "Microsoft.Edge" : "Google.Chrome";
      const detail = firstSeen.version ? `${firstSeen.version} (WebGPU needs ${MIN_BROWSER_MAJOR}+)` : "(version unreadable)";
      miss(firstSeen.name, `winget upgrade --id ${id} -e`, detail);
    }
  }

  // gh runs the GitHub proofs (tools/gh-check.mjs), so it must be installed and logged in.
  if (!env.which("gh")) miss("gh", "winget install --id GitHub.cli -e");
  else {
    const gh = env.run("gh", ["--version"]);
    const version = gh.ok ? firstMatch(gh.out, /gh version (\d+\.\d+\.\d+)/) : null;
    if (!version) miss("gh", "winget upgrade --id GitHub.cli -e", "(version unreadable)");
    else if (!env.run("gh", ["auth", "status"]).ok) miss("gh", "gh auth login", "(not logged in to GitHub)");
    else ok("gh", version);
  }

  return { lines, missing };
}

// Prints the report through `write` and returns the exit code.
export function runWebDoctor(env, write = console.log) {
  const { lines, missing } = checkWebTarget(env);
  for (const line of lines) write(line);
  write(missing ? `web target: NOT READY ${missing} missing` : "web target: READY");
  return missing ? 1 : 0;
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  process.exitCode = runWebDoctor(realEnv());
}
