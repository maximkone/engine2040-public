// web-doctor drills: a fake environment decides which web-target pieces are
// present, so no test reads the real PATH, runs a real binary or touches the
// real browser install. Each missing piece must print exactly one fix line.
//   node --test tools/web-doctor.test.mjs
import test from "node:test";
import assert from "node:assert/strict";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { checkWebTarget, realWhich, runWebDoctor } from "./web-doctor.mjs";

const BIN = "C:\\fake\\bin";
const CHROME = "C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe";
const LOCK = 'name = "wasm-bindgen"\nversion = "0.2.128"\n';

// A fake machine. `present` names resolve on PATH; `outputs` maps "<tool> <args>"
// to stdout (missing key = the command fails); `versions` maps a browser file
// name to its file version; `browserPaths` are install paths that exist.
const fakeEnv = ({ present = [], outputs = {}, versions = {}, lock = null, nodeVersion = "24.14.0", browserPaths = [] } = {}) => ({
  which: (name) => (present.includes(name) ? `${BIN}\\${name}.exe` : null),
  run: (cmd, args) => {
    const key = `${cmd.split(/[\\/]/).pop().replace(/\.exe$/i, "")} ${args.join(" ")}`;
    return key in outputs ? { ok: true, out: outputs[key] } : { ok: false, out: "" };
  },
  fileVersion: (path) => versions[path.split(/[\\/]/).pop()] ?? null,
  exists: (path) => browserPaths.includes(path),
  readFile: () => lock,
  nodeVersion,
  browserPaths,
  lockPath: "C:\\fake\\Cargo.lock",
});

const READY_OUTPUTS = {
  "rustup target list --installed": "x86_64-pc-windows-msvc\nwasm32-unknown-unknown\nwasm32-wasip1\n",
  "rustc --version": "rustc 1.97.1 (8bab26f4f 2026-07-14)",
  "wasm-bindgen --version": "wasm-bindgen 0.2.128",
  "wasm-opt --version": "wasm-opt version 119 (version_119)",
  "wasmtime --version": "wasmtime-cli 27.0.0",
  "gh --version": "gh version 2.88.1 (2026-03-12)",
  "gh auth status": "github.com\n  ✓ Logged in to github.com",
};
const READY = {
  present: ["rustup", "wasm-bindgen", "wasm-opt", "wasmtime", "chrome", "gh"],
  outputs: READY_OUTPUTS,
  versions: { "chrome.exe": "155.0.8059.80" },
  lock: LOCK,
};

test("nothing installed: every missing piece prints one fix line and the tool exits 1", () => {
  const out = [];
  const code = runWebDoctor(fakeEnv(), (l) => out.push(l));
  const fixes = out.filter((l) => l.startsWith("  fix: ")).map((l) => l.slice("  fix: ".length));
  assert.deepEqual(fixes, [
    "winget install --id Rustlang.Rustup -e",
    "rustup target add wasm32-unknown-unknown",
    "rustup target add wasm32-wasip1",
    "cargo install wasm-bindgen-cli --locked",
    "winget install --id WebAssembly.Binaryen -e",
    "winget install --id BytecodeAlliance.Wasmtime -e",
    "winget install --id Google.Chrome -e",
    "winget install --id GitHub.cli -e",
  ]);
  assert.equal(out.at(-1), "web target: NOT READY 8 missing");
  assert.equal(code, 1);
});

test("everything installed: every piece is OK, the target is READY and the tool exits 0", () => {
  const out = [];
  const code = runWebDoctor(fakeEnv(READY), (l) => out.push(l));
  assert.ok(!out.some((l) => l.startsWith("MISSING")), out.join("\n"));
  assert.ok(out.includes("OK wasm-bindgen 0.2.128"));
  assert.ok(out.includes("OK wasm-opt 119"));
  assert.ok(out.includes("OK chrome 155.0.8059.80"));
  assert.equal(out.at(-1), "web target: READY");
  assert.equal(code, 0);
});

test("only the wasm-bindgen version mismatched: that one piece is MISSING with the matching fix", () => {
  const env = fakeEnv({ ...READY, outputs: { ...READY_OUTPUTS, "wasm-bindgen --version": "wasm-bindgen 0.2.127" } });
  const { lines, missing } = checkWebTarget(env);
  assert.deepEqual(lines.filter((l) => l.startsWith("MISSING")), ["MISSING wasm-bindgen 0.2.127 (Cargo.lock needs 0.2.128)"]);
  assert.ok(lines.includes("  fix: cargo install wasm-bindgen-cli --version 0.2.128 --locked --force"));
  assert.equal(missing, 1);
  assert.equal(runWebDoctor(env, () => {}), 1);
});

test("one rust target missing: only that target gets a fix line", () => {
  const env = fakeEnv({ ...READY, outputs: { ...READY_OUTPUTS, "rustup target list --installed": "wasm32-unknown-unknown\n" } });
  const { lines, missing } = checkWebTarget(env);
  assert.deepEqual(lines.filter((l) => l.startsWith("  fix:")), ["  fix: rustup target add wasm32-wasip1"]);
  assert.equal(missing, 1);
});

test("Chrome older than 113 cannot do WebGPU: it is MISSING and gets an upgrade fix", () => {
  const env = fakeEnv({ ...READY, versions: { "chrome.exe": "100.0.4896.60" } });
  const { lines, missing } = checkWebTarget(env);
  assert.ok(lines.includes("MISSING chrome 100.0.4896.60 (WebGPU needs 113+)"));
  assert.ok(lines.includes("  fix: winget upgrade --id Google.Chrome -e"));
  assert.equal(missing, 1);
});

test("a browser in its usual install path counts when it is not on PATH", () => {
  const env = fakeEnv({ ...READY, present: READY.present.filter((n) => n !== "chrome"), browserPaths: [CHROME] });
  const { lines, missing } = checkWebTarget(env);
  assert.ok(lines.includes("OK chrome 155.0.8059.80"), lines.join("\n"));
  assert.equal(missing, 0);
});

test("gh on PATH but not logged in: MISSING with the login fix", () => {
  const outputs = { ...READY_OUTPUTS };
  delete outputs["gh auth status"];
  const { lines, missing } = checkWebTarget(fakeEnv({ ...READY, outputs }));
  assert.ok(lines.includes("MISSING gh (not logged in to GitHub)"));
  assert.ok(lines.includes("  fix: gh auth login"));
  assert.equal(missing, 1);
});

test("Node older than the pinned major is MISSING", () => {
  const { lines, missing } = checkWebTarget(fakeEnv({ ...READY, nodeVersion: "20.11.0" }));
  assert.ok(lines.includes("MISSING node 20.11.0 (need 24+)"));
  assert.ok(lines.includes("  fix: winget upgrade --id OpenJS.NodeJS.LTS -e"));
  assert.equal(missing, 1);
});

test("realWhich finds a tool through the PATH it is given, with PATHEXT, and nothing else", () => {
  const dir = mkdtempSync(join(tmpdir(), "web-doctor-which-"));
  try {
    writeFileSync(join(dir, "tool-x.ZZ"), "");
    const env = { PATH: dir, PATHEXT: ".ZZ" };
    assert.equal(realWhich("tool-x", env), join(dir, "tool-x.ZZ"));
    assert.equal(realWhich("absent-tool", env), null);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});
