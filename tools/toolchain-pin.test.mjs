// Run: node --test tools/toolchain-pin.test.mjs
// toolchain-pin: the 1.97.1 pin agrees across its three homes.
// Fields (read by real name, never guessed):
// - rust-toolchain.toml: [toolchain] channel
// - Cargo.toml: [workspace.package] rust-version (root [package] delegates via rust-version.workspace)
// - .github/workflows/ci.yml: install step `run: rustup toolchain install` plus a
//   `rust-toolchain.toml` name reference (CI defers to the file; no hardcoded pin).
// Owner: tools. Units: version strings compared; CI text scanned for deferral.
// Errors: non-string input throws TypeError, never silent; missing files fail the repo test.
import assert from "node:assert/strict";
import test from "node:test";
import { readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = resolve(join(dirname(fileURLToPath(import.meta.url)), ".."));

// [toolchain] channel = "1.97.1" in rust-toolchain.toml.
export function parseToolchainChannel(text) {
  if (typeof text !== "string") throw new TypeError("parseToolchainChannel needs a string");
  const m = /channel\s*=\s*"([^"]+)"/.exec(text);
  return m ? m[1].trim() : "";
}

// [workspace.package] rust-version = "1.97.1" in the root Cargo.toml.
// Section-scoped so per-crate `rust-version.workspace = true` lines never match.
export function parseWorkspaceRustVersion(text) {
  if (typeof text !== "string") throw new TypeError("parseWorkspaceRustVersion needs a string");
  const lines = text.split("\n");
  let section = "";
  for (const raw of lines) {
    const header = /^\s*\[([^\]]+)\]\s*(?:#.*)?$/.exec(raw);
    if (header) {
      section = header[1].trim();
      continue;
    }
    if (section === "workspace.package") {
      const m = /^\s*rust-version\s*=\s*"([^"]+)"\s*(?:#.*)?$/.exec(raw);
      if (m) return m[1].trim();
    }
  }
  return "";
}

// Root [package] must delegate with `rust-version.workspace = true`.
export function hasWorkspaceDelegation(text) {
  if (typeof text !== "string") throw new TypeError("hasWorkspaceDelegation needs a string");
  return /rust-version\.workspace\s*=\s*true/.test(text);
}

// CI defers when it names the file and installs via rustup (which reads the file).
export function ciDefersToToolchainFile(text) {
  if (typeof text !== "string") throw new TypeError("ciDefersToToolchainFile needs a string");
  return text.includes("rust-toolchain.toml") && text.includes("rustup toolchain install");
}

// A hardcoded Rust pin inside ci.yml would drift from the file. Node tool
// versions (node-version: 24) and cargo-deny@0.18.7 are not Rust pins and
// never match: only rust-specific pin shapes count.
export function ciHardcodedRustPins(text) {
  if (typeof text !== "string") throw new TypeError("ciHardcodedRustPins needs a string");
  const found = [];
  for (const raw of text.split("\n")) {
    const line = raw.trim();
    if (/toolchain\s*:\s*["']?1\.\d+/.test(line)) found.push(line);
    else if (/channel\s*=\s*"[^"]+"/.test(line)) found.push(line);
    else if (/rust-version\s*[:=]\s*["']?1\.\d+/.test(line)) found.push(line);
    else if (/dtolnay\/rust-toolchain@/.test(line)) found.push(line);
  }
  return found;
}

test("parseToolchainChannel reads [toolchain] channel", () => {
  assert.equal(parseToolchainChannel('[toolchain]\nchannel = "1.97.1"\n'), "1.97.1");
  assert.equal(parseToolchainChannel('[toolchain]\r\nchannel = "1.97.1"\r\n'), "1.97.1");
  assert.equal(parseToolchainChannel(""), "");
  assert.throws(() => parseToolchainChannel(null), TypeError);
});

test("parseWorkspaceRustVersion reads [workspace.package] only", () => {
  const manifest = [
    "[package]",
    'name = "engine2040"',
    "rust-version.workspace = true",
    "",
    "[workspace.package]",
    'rust-version = "1.97.1"',
    "",
    "[workspace.dependencies]",
    "serde = \"1\"",
  ].join("\n");
  assert.equal(parseWorkspaceRustVersion(manifest), "1.97.1");
  assert.equal(parseWorkspaceRustVersion('[package]\nname = "x"\n'), "");
  assert.equal(parseWorkspaceRustVersion('[workspace.package]\nversion = "0.1.0"\n'), "");
  assert.throws(() => parseWorkspaceRustVersion(undefined), TypeError);
});

test("hasWorkspaceDelegation spots the root delegate line", () => {
  assert.equal(hasWorkspaceDelegation("rust-version.workspace = true"), true);
  assert.equal(hasWorkspaceDelegation('rust-version = "1.97.1"'), false);
  assert.throws(() => hasWorkspaceDelegation(42), TypeError);
});

test("ci helpers: deferral needs both markers, hardcoded finds rust pins only", () => {
  const defer = "name: install repository toolchain (rust-toolchain.toml)\nrun: rustup toolchain install && rustup show active-toolchain";
  assert.equal(ciDefersToToolchainFile(defer), true);
  assert.equal(ciDefersToToolchainFile("run: rustup toolchain install"), false);
  assert.equal(ciDefersToToolchainFile('name: toolchain (rust-toolchain.toml)\nrun: echo hi'), false);
  assert.deepEqual(ciHardcodedRustPins(defer), []);
  assert.deepEqual(ciHardcodedRustPins("node-version: 24\n"), []);
  assert.deepEqual(ciHardcodedRustPins("tool: cargo-deny@0.18.7\n"), []);
  assert.equal(ciHardcodedRustPins('uses: dtolnay/rust-toolchain@1.97.1\n').length, 1);
  assert.equal(ciHardcodedRustPins('channel = "1.97.1"\n').length, 1);
  assert.throws(() => ciDefersToToolchainFile(null), TypeError);
  assert.throws(() => ciHardcodedRustPins(null), TypeError);
});

test("repo: toolchain pin agrees across rust-toolchain.toml, Cargo.toml and ci.yml", () => {
  const channel = parseToolchainChannel(readFileSync(join(ROOT, "rust-toolchain.toml"), "utf8"));
  const manifest = readFileSync(join(ROOT, "Cargo.toml"), "utf8");
  const workspaceRust = parseWorkspaceRustVersion(manifest);
  const ci = readFileSync(join(ROOT, ".github", "workflows", "ci.yml"), "utf8");
  assert.match(channel, /^\d+\.\d+\.\d+$/, `rust-toolchain.toml [toolchain] channel is a version, got ${channel}`);
  assert.match(workspaceRust, /^\d+\.\d+\.\d+$/, `Cargo.toml [workspace.package] rust-version is a version, got ${workspaceRust}`);
  assert.equal(workspaceRust, channel, `pin agrees: rust-toolchain.toml channel ${channel} vs Cargo.toml rust-version ${workspaceRust}`);
  assert.ok(hasWorkspaceDelegation(manifest), "root [package] delegates via rust-version.workspace = true");
  assert.ok(ciDefersToToolchainFile(ci), "ci.yml installs via `rustup toolchain install` and names rust-toolchain.toml");
  assert.deepEqual(ciHardcodedRustPins(ci), [], "ci.yml carries no hardcoded Rust pin");
});
